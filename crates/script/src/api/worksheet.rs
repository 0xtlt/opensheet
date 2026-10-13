//! `Worksheet`, its frozen panes and the filters (`AutoFilter`).

use serde_json::Value as J;

use opensheet_engine::features::Edit;
use opensheet_engine::filter::{
    ColorFilter, Condition, Criteria, CustomFilter, CustomOp, DynamicFilter, FilterError, FilterScope, Top10, ValueFilter,
};
use opensheet_engine::find::{FindOptions, LookIn};
use opensheet_engine::names::NameScope;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::table::TableOption;
use opensheet_engine::workbook::{Content, Sheet, SheetVisibility};

use super::util::{color_arg, color_text, parse_color, range_arg, resolve, resolve_areas, table_fail};
use super::workbook::{add_name, all_tables, name_in, names_in, sheet_guid, sheet_name_problem};
use super::{ApiClass, Call, Res, Ret, areas, chart, class, comment, enum_value, object, pivot, shape, state_of, table};
use crate::host::{Host, MAX_SHEETS, rect_from};

/// The sheet a Worksheet object (or one of its parts) stands for.
fn this_sheet(h: &Host, call: &Call) -> Res<usize> {
    h.sheet(call, call.this_int(0))
}

/// The smallest rectangle around the cells that hold something.
fn content_rect(sheet: &Sheet) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    let mut add = |rect: Rect| {
        out = Some(out.map_or(rect, |o| o.union(&rect)));
    };
    for ((row, col), cell) in &sheet.cells {
        if !matches!(cell.content, Content::Empty) {
            add(Rect::cell(*row, *col));
        }
    }
    for spill in &sheet.spills {
        add(Rect::new(
            spill.row,
            spill.col,
            spill.row + spill.array.rows as u32 - 1,
            spill.col + spill.array.cols as u32 - 1,
        ));
    }
    out
}

/// The used range of a sheet: with formats, or of contents only.
pub(super) fn used_rect(h: &mut Host, sheet: usize, values_only: bool) -> Option<Rect> {
    h.calculate();
    let sh = &h.wb.sheets[sheet];
    if values_only { content_rect(sh) } else { sh.used_rect() }
}

/// The `completeMatch` and `matchCase` of the criteria of a search.
pub(super) fn find_options(call: &Call, index: usize) -> Res<FindOptions> {
    let criteria = call.arg(index);
    if !criteria.is_null() && !criteria.is_object() {
        return Err(call.invalid());
    }
    let flag = |key: &str| criteria.get(key).and_then(J::as_bool).unwrap_or(false);
    Ok(FindOptions {
        match_case: flag("matchCase"),
        whole_cell: flag("completeMatch"),
        look_in: LookIn::Formulas,
        by_columns: false,
    })
}

// ---- filters -----------------------------------------------------------------

/// One comparison of a custom filter: `>=10`, `<>done`, `=a*`, `text`.
fn condition(text: &str) -> Condition {
    let (op, value) = [
        ("<>", CustomOp::NotEqual),
        (">=", CustomOp::GreaterOrEqual),
        ("<=", CustomOp::LessOrEqual),
        (">", CustomOp::Greater),
        ("<", CustomOp::Less),
        ("=", CustomOp::Equal),
    ]
    .into_iter()
    .find_map(|(prefix, op)| text.strip_prefix(prefix).map(|rest| (op, rest)))
    .unwrap_or((CustomOp::Equal, text));
    Condition {
        op,
        value: value.to_string(),
    }
}

fn dynamic(call: &Call, name: &str) -> Res<DynamicFilter> {
    const MONTHS: [&str; 12] =
        ["January", "Februray", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    if let Some(period) = name.strip_prefix("AllDatesInPeriod") {
        if let Some(month) = MONTHS.iter().position(|m| *m == period) {
            return Ok(DynamicFilter::Month(month as u8 + 1));
        }
        if let Some(quarter) = period.strip_prefix("Quarter").and_then(|q| q.parse::<u8>().ok()) {
            return Ok(DynamicFilter::Quarter(quarter));
        }
    }
    Ok(match name {
        "AboveAverage" => DynamicFilter::AboveAverage,
        "BelowAverage" => DynamicFilter::BelowAverage,
        "Today" => DynamicFilter::Today,
        "Yesterday" => DynamicFilter::Yesterday,
        "Tomorrow" => DynamicFilter::Tomorrow,
        "ThisWeek" => DynamicFilter::ThisWeek,
        "LastWeek" => DynamicFilter::LastWeek,
        "NextWeek" => DynamicFilter::NextWeek,
        "ThisMonth" => DynamicFilter::ThisMonth,
        "LastMonth" => DynamicFilter::LastMonth,
        "NextMonth" => DynamicFilter::NextMonth,
        "ThisQuarter" => DynamicFilter::ThisQuarter,
        "LastQuarter" => DynamicFilter::LastQuarter,
        "NextQuarter" => DynamicFilter::NextQuarter,
        "ThisYear" => DynamicFilter::ThisYear,
        "LastYear" => DynamicFilter::LastYear,
        "NextYear" => DynamicFilter::NextYear,
        "YearToDate" => DynamicFilter::YearToDate,
        _ => return Err(call.invalid()),
    })
}

/// The checked values of a values filter: texts as the cells show them,
/// the empty text for the blanks.
pub(super) fn values_filter(call: &Call, values: &[J]) -> Res<Criteria> {
    let mut filter = ValueFilter::default();
    for value in values {
        let text = match value {
            J::String(s) => s.clone(),
            J::Number(n) => n.to_string(),
            J::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            J::Object(_) => return Err(call.unsupported("A date filter (FilterDatetime)")),
            _ => return Err(call.invalid()),
        };
        if text.is_empty() {
            filter.blanks = true;
        } else {
            filter.values.push(text);
        }
    }
    Ok(Criteria::Values(filter))
}

pub(super) fn custom_filter(first: &str, second: Option<&str>, operator: Option<&str>) -> Criteria {
    Criteria::Custom(CustomFilter {
        first: condition(first),
        second: second.map(condition),
        and: operator != Some("Or"),
    })
}

pub(super) fn top_filter(call: &Call, count: f64, top: bool, percent: bool) -> Res<Criteria> {
    if !(count >= 1.0 && count <= 500.0) {
        return Err(call.invalid());
    }
    Ok(Criteria::Top10(Top10::new(top, percent, count)))
}

pub(super) fn dynamic_filter(call: &Call, index: usize) -> Res<Criteria> {
    let name = enum_value(call, index, "DynamicFilterCriteria")?.ok_or_else(|| call.invalid())?;
    Ok(Criteria::Dynamic(dynamic(call, name)?))
}

pub(super) fn color_filter(call: &Call, index: usize, font: bool) -> Res<Criteria> {
    Ok(Criteria::Color(ColorFilter {
        color: color_arg(call, index)?,
        font,
    }))
}

/// A `FilterCriteria` object of the API as the engine filters.
pub(super) fn criteria_from(call: &Call, criteria: &J) -> Res<Criteria> {
    let text = |key: &str| criteria.get(key).and_then(J::as_str);
    let count = |key: &str| {
        let value = criteria.get(key).ok_or_else(|| call.invalid())?;
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
            .ok_or_else(|| call.invalid())
    };
    let color = |font: bool| {
        let color = text("color").ok_or_else(|| call.invalid())?;
        Ok(Criteria::Color(ColorFilter {
            color: Some(parse_color(color).ok_or_else(|| call.invalid())?),
            font,
        }))
    };
    match text("filterOn").ok_or_else(|| call.invalid())? {
        "Values" => {
            let values = criteria.get("values").and_then(J::as_array).ok_or_else(|| call.invalid())?;
            values_filter(call, values)
        }
        "Custom" => Ok(custom_filter(
            text("criterion1").ok_or_else(|| call.invalid())?,
            text("criterion2"),
            text("operator"),
        )),
        "TopItems" => top_filter(call, count("criterion1")?, true, false),
        "BottomItems" => top_filter(call, count("criterion1")?, false, false),
        "TopPercent" => top_filter(call, count("criterion1")?, true, true),
        "BottomPercent" => top_filter(call, count("criterion1")?, false, true),
        "Dynamic" => Ok(Criteria::Dynamic(dynamic(call, text("dynamicCriteria").ok_or_else(|| call.invalid())?)?)),
        "CellColor" => color(false),
        "FontColor" => color(true),
        "Icon" => Err(call.unsupported("ExcelScript.FilterOn.icon")),
        _ => Err(call.invalid()),
    }
}

/// Set (or clear) the criterion of a column of a filter.
pub(super) fn set_criteria(h: &mut Host, call: &Call, sheet: usize, scope: FilterScope, column: i64, criteria: Option<Criteria>) -> Res<()> {
    h.permit(call, &Action::UseAutoFilter { sheet })?;
    h.calculate();
    let column = u32::try_from(column).map_err(|_| call.invalid())?;
    if !h.wb.set_filter_criteria(sheet, scope, column, criteria) {
        return Err(call.invalid());
    }
    h.restructured();
    Ok(())
}

pub(super) fn auto_filter_object(h: &Host, sheet: usize, table: Option<u32>) -> J {
    object("AutoFilter", vec![h.sheet_id(sheet).into(), table.unwrap_or(0).into()])
}

/// The sheet and the filter an AutoFilter object stands for.
fn this_filter(h: &Host, call: &Call) -> Res<(usize, FilterScope)> {
    match call.this_int(1) {
        0 => Ok((this_sheet(h, call)?, FilterScope::Sheet)),
        id => {
            let (sheet, _) = h.wb.table_by_id(id as u32).ok_or_else(|| call.gone())?;
            Ok((sheet, FilterScope::Table(id as u32)))
        }
    }
}

pub(super) const WORKSHEET: ApiClass = class!("Worksheet", "A sheet of the workbook: its cells, tables, names and what the window shows of it.", {
    getName "(): string" "The name on the tab." => |h, c| Ok(h.wb.sheets[this_sheet(h, c)?].name.clone().into());
    setName "(name: string): void" "Renames the worksheet; formulas follow." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let name = c.str(0)?;
        h.permit(c, &Action::RenameSheet)?;
        if let Some(problem) = sheet_name_problem(&h.wb, name, Some(sheet)) {
            return Err(c.fail(problem));
        }
        h.wb.rename_sheet(sheet, name);
        h.restructured();
        Ok(Ret::Nothing)
    };
    getId "(): string" "An identifier that stays the same when the worksheet is renamed or moved." => |h, c| {
        this_sheet(h, c)?;
        Ok(sheet_guid(c.this_int(0)).into())
    };
    getRange "(address?: string): Range" "The range at an address (A1, B2:D5, A:C, a defined name); the whole sheet without one." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let Some(text) = c.opt_str(0)? else {
            return Ok(Ret::Object(h.range(sheet, Rect::new(0, 0, MAX_ROWS - 1, MAX_COLS - 1))));
        };
        let (on, rect) = resolve(h, c, sheet, text)?;
        if on != sheet {
            return Err(c.invalid());
        }
        Ok(Ret::Object(h.range(sheet, rect)))
    };
    getRangeByIndexes "(startRow: number, startColumn: number, rowCount: number, columnCount: number): Range" "The range starting at a row and a column (0-based) with that many rows and columns." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let rect = rect_from(c.int(0)?, c.int(1)?, c.int(2)?, c.int(3)?).ok_or_else(|| c.invalid())?;
        Ok(Ret::Object(h.range(sheet, rect)))
    };
    getCell "(row: number, column: number): Range" "The cell at a row and a column (0-based)." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let rect = rect_from(c.int(0)?, c.int(1)?, 1, 1).ok_or_else(|| c.invalid())?;
        Ok(Ret::Object(h.range(sheet, rect)))
    };
    getRanges "(address?: string): RangeAreas" "Several ranges of the worksheet held together, from addresses separated by commas (A1:B2, D4) or a name; the whole worksheet without an address." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let Some(text) = c.opt_str(0)? else {
            return Ok(Ret::Object(areas::areas_object(h, sheet, &[Rect::new(0, 0, MAX_ROWS - 1, MAX_COLS - 1)])));
        };
        let (on, rects) = resolve_areas(h, c, sheet, text)?;
        if on != sheet {
            return Err(c.invalid());
        }
        Ok(Ret::Object(areas::areas_object(h, sheet, &rects)))
    };
    findAll "(text: string, criteria: ExcelScript.WorksheetSearchCriteria): RangeAreas | undefined" "Every cell of the worksheet holding a text, as Find All; undefined when none does." => |h, c| {
        let sheet = this_sheet(h, c)?;
        areas::find_all(h, c, sheet, &[])
    };
    getUsedRange "(valuesOnly?: boolean): Range | undefined" "The smallest range holding every cell with a value or a format (values only on request); undefined for an empty sheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let values_only = c.opt_bool(0)?.unwrap_or(false);
        Ok(Ret::maybe(used_rect(h, sheet, values_only).map(|rect| h.range(sheet, rect))))
    };
    activate "(): void" "Brings the worksheet in front. A hidden worksheet cannot be." => |h, c| {
        let sheet = this_sheet(h, c)?;
        if !h.wb.sheets[sheet].is_visible() {
            return Err(c.fail("A hidden worksheet cannot be activated."));
        }
        h.activate(sheet);
        Ok(Ret::Nothing)
    };
    delete "(): void" "Deletes the worksheet. The last one cannot be deleted." => |h, c| {
        let sheet = this_sheet(h, c)?;
        h.permit(c, &Action::DeleteSheet)?;
        if !h.wb.delete_sheet(sheet) {
            return Err(c.fail("A workbook must contain at least one visible worksheet."));
        }
        h.sheet_deleted(sheet);
        Ok(Ret::Nothing)
    };
    copy "(positionType?: ExcelScript.WorksheetPositionType, relativeTo?: Worksheet): Worksheet" "Copies the worksheet: at the beginning (the default), at the end, or before or after another worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        h.permit(c, &Action::CopySheet)?;
        let position = enum_value(c, 0, "WorksheetPositionType")?.unwrap_or("None");
        let relative = match state_of(c.arg(1), "Worksheet") {
            Some(state) => Some(state.first().and_then(J::as_i64).ok_or_else(|| c.invalid())?),
            None if c.has(1) => return Err(c.invalid()),
            None => None,
        };
        if h.wb.sheets.len() >= MAX_SHEETS {
            return Err(c.fail(format!("A script cannot make a workbook of more than {MAX_SHEETS} worksheets.")));
        }
        let made = h.wb.duplicate_sheet(sheet).ok_or_else(|| c.gone())?;
        h.sheet_added(made);
        let id = h.sheet_id(made);
        // Where the copy goes, counted with the copy taken out.
        let anchor = |h: &Host| -> Res<usize> {
            let index = h.sheet(c, relative.ok_or_else(|| c.invalid())?)?;
            Ok(if index > made { index - 1 } else { index })
        };
        let to = match position {
            "End" => h.wb.sheets.len() - 1,
            "Before" => anchor(h)?,
            "After" => anchor(h)? + 1,
            _ => 0,
        };
        if to != made {
            h.wb.move_sheet(made, to);
            h.sheet_moved(made, to);
        }
        Ok(Ret::Object(object("Worksheet", vec![id.into()])))
    };
    getPosition "(): number" "The place of the worksheet among the tabs, 0-based." => |h, c| Ok((this_sheet(h, c)? as u64).into());
    setPosition "(position: number): void" "Moves the worksheet to a place among the tabs, 0-based." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let to = usize::try_from(c.int(0)?).ok().filter(|to| *to < h.wb.sheets.len()).ok_or_else(|| c.invalid())?;
        h.permit(c, &Action::MoveSheet)?;
        if to != sheet {
            h.wb.move_sheet(sheet, to);
            h.sheet_moved(sheet, to);
        }
        Ok(Ret::Nothing)
    };
    getNext "(visibleOnly?: boolean): Worksheet | undefined" "The worksheet after this one (the next visible one on request); undefined for the last." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let next = if c.opt_bool(0)?.unwrap_or(false) {
            h.wb.next_visible_sheet(sheet, true)
        } else {
            (sheet + 1 < h.wb.sheets.len()).then_some(sheet + 1)
        };
        Ok(Ret::maybe(next.map(|i| h.worksheet(i))))
    };
    getPrevious "(visibleOnly?: boolean): Worksheet | undefined" "The worksheet before this one (the previous visible one on request); undefined for the first." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let previous = if c.opt_bool(0)?.unwrap_or(false) { h.wb.next_visible_sheet(sheet, false) } else { sheet.checked_sub(1) };
        Ok(Ret::maybe(previous.map(|i| h.worksheet(i))))
    };
    getVisibility "(): ExcelScript.SheetVisibility" "Visible, Hidden, or VeryHidden for a worksheet Unhide… does not list." => |h, c| {
        Ok(match h.wb.sheets[this_sheet(h, c)?].visibility {
            SheetVisibility::Visible => "Visible",
            SheetVisibility::Hidden => "Hidden",
            SheetVisibility::VeryHidden => "VeryHidden",
        }.into())
    };
    setVisibility "(visibility: ExcelScript.SheetVisibility): void" "Hides the worksheet (Hidden, or VeryHidden to keep it out of Unhide…) or shows it again. The last visible worksheet cannot be hidden." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let visibility = match enum_value(c, 0, "SheetVisibility")?.ok_or_else(|| c.invalid())? {
            "Hidden" => SheetVisibility::Hidden,
            "VeryHidden" => SheetVisibility::VeryHidden,
            _ => SheetVisibility::Visible,
        };
        if h.wb.sheets[sheet].visibility == visibility {
            return Ok(Ret::Nothing);
        }
        h.permit(c, if visibility == SheetVisibility::Visible { &Action::UnhideSheet } else { &Action::HideSheet })?;
        if !h.wb.set_sheet_visibility(sheet, visibility) {
            return Err(c.fail("A workbook must contain at least one visible worksheet."));
        }
        if visibility != SheetVisibility::Visible {
            h.sheet_hidden(sheet);
        }
        Ok(Ret::Nothing)
    };
    getTabColor "(): string" "The colour of the tab as #RRGGBB; empty text for none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        Ok(h.wb.sheets[sheet].tab_color.map(color_text).unwrap_or_default().into())
    };
    setTabColor "(color: string): void" "Colours the tab (#RRGGBB or a colour name); empty text removes the colour." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let color = color_arg(c, 0)?;
        h.permit(c, &Action::ChangeSheet { sheet })?;
        h.wb.set_tab_color(sheet, color);
        Ok(Ret::Nothing)
    };
    getShowGridlines "(): boolean" "Whether the gridlines show." => |h, c| Ok(h.wb.sheets[this_sheet(h, c)?].show_gridlines.into());
    setShowGridlines "(showGridlines: boolean): void" "Shows or hides the gridlines." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let show = c.bool(0)?;
        h.wb.snapshot();
        h.wb.sheets[sheet].show_gridlines = show;
        Ok(Ret::Nothing)
    };
    getShowHeadings "(): boolean" "Whether the row and column headings show." => |h, c| Ok(h.wb.sheets[this_sheet(h, c)?].show_headers.into());
    setShowHeadings "(showHeadings: boolean): void" "Shows or hides the row and column headings." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let show = c.bool(0)?;
        h.wb.snapshot();
        h.wb.sheets[sheet].show_headers = show;
        Ok(Ret::Nothing)
    };
    getProtection "(): WorksheetProtection" "The protection of the worksheet." => |h, c| {
        this_sheet(h, c)?;
        Ok(Ret::object("WorksheetProtection", vec![c.this_int(0).into()]))
    };
    getFreezePanes "(): WorksheetFreezePanes" "The frozen panes of the worksheet." => |h, c| {
        this_sheet(h, c)?;
        Ok(Ret::object("WorksheetFreezePanes", vec![c.this_int(0).into()]))
    };
    getAutoFilter "(): AutoFilter" "The filter of the worksheet (Data > Filter)." => |h, c| {
        let sheet = this_sheet(h, c)?;
        Ok(Ret::Object(auto_filter_object(h, sheet, None)))
    };
    getTables "(): Table[]" "The tables of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        Ok(Ret::Object(all_tables(h, Some(sheet))))
    };
    getTable "(key: string): Table | undefined" "The table of the worksheet with this name or ID; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        table::find(h, c, Some(sheet))
    };
    addTable "(address: Range | string, hasHeaders: boolean): Table" "Makes a table of a range of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let (on, rect) = range_arg(h, c, 0, sheet)?;
        table::add(h, c, on, rect)
    };
    getCharts "(): Chart[]" "The charts of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        Ok(Ret::Object(chart::charts_of(h, sheet)))
    };
    getChart "(name: string): Chart | undefined" "The chart of the worksheet with this name; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        chart::find(h, c, sheet)
    };
    addChart "(type: ExcelScript.ChartType, sourceData: Range, seriesBy?: ExcelScript.ChartSeriesBy): Chart" "Adds a chart of a range: labels in its first row and column are found as when a chart is inserted by hand." => |h, c| {
        let sheet = this_sheet(h, c)?;
        chart::add(h, c, sheet)
    };
    getNames "(): NamedItem[]" "The defined names whose scope is the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        Ok(Ret::Object(names_in(h, NameScope::Sheet(sheet))))
    };
    getNamedItem "(name: string): NamedItem | undefined" "The defined name of the worksheet spelled this way; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        name_in(h, c, NameScope::Sheet(sheet))
    };
    addNamedItem "(name: string, reference: Range | string, comment?: string): NamedItem" "Defines a name whose scope is the worksheet (formulas in English)." => |h, c| {
        let sheet = this_sheet(h, c)?;
        add_name(h, c, NameScope::Sheet(sheet), false)
    };
    addNamedItemFormulaLocal "(name: string, formula: string, comment?: string): NamedItem" "Defines a name of the worksheet from a formula in the language of the workbook." => |h, c| {
        let sheet = this_sheet(h, c)?;
        add_name(h, c, NameScope::Sheet(sheet), true)
    };
    addComment "(cellAddress: Range | string, content: ExcelScript.CommentRichContent | string, contentType?: ExcelScript.ContentType): Comment" "Starts a comment thread on a cell of the worksheet, as the user of the application." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::add(h, c, sheet)
    };
    getComments "(): Comment[]" "The comments of the worksheet." => |h, c| Ok(comment::all(h, Some(this_sheet(h, c)?), true));
    getComment "(commentId: string): Comment | undefined" "The comment of the worksheet with this identifier; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::by_id(h, c, Some(sheet))
    };
    getCommentByCell "(cellAddress: Range | string): Comment" "The comment of a cell of the worksheet. Fails when the cell has none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::by_cell(h, c, sheet)
    };
    getCommentByReplyId "(replyId: string): Comment" "The comment of the worksheet a reply belongs to. Fails when there is no such reply." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::by_reply(h, c, Some(sheet))
    };
    addNote "(cellAddress: Range | string, content: string): Note" "Puts a note on a cell of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::add_note(h, c, sheet)
    };
    getNotes "(): Note[]" "The notes of the worksheet." => |h, c| Ok(comment::all(h, Some(this_sheet(h, c)?), false));
    getNote "(key: number | string): Note | undefined" "The note of a cell of the worksheet (by address), or the note at a place among its notes (0-based); undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        comment::note_by_key(h, c, sheet)
    };
    addPivotTable "(name: string, source: Range | string | Table, destination: Range | string): PivotTable" "Inserts a PivotTable without fields on the worksheet: of a range (header row first) or of a table, with its top-left cell at the destination." => |h, c| {
        let sheet = this_sheet(h, c)?;
        pivot::add(h, c, sheet)
    };
    getPivotTables "(): PivotTable[]" "The PivotTables of the worksheet." => |h, c| Ok(pivot::all(h, Some(this_sheet(h, c)?)));
    getPivotTable "(name: string): PivotTable | undefined" "The PivotTable of the worksheet with this name; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        pivot::by_name(h, c, Some(sheet))
    };
    refreshAllPivotTables "(): void" "Refreshes the PivotTables of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        pivot::refresh_all(h, c, Some(sheet))
    };
    addGeometricShape "(geometricShapeType: ExcelScript.GeometricShapeType): Shape" "Draws a geometric shape at the top left of the worksheet, at the size a click of its tool gives; setLeft, setTop, setWidth and setHeight place it." => |h, c| {
        let sheet = this_sheet(h, c)?;
        shape::add_geometric(h, c, sheet)
    };
    addTextBox "(text?: string): Shape" "Draws a text box at the top left of the worksheet, with a text." => |h, c| {
        let sheet = this_sheet(h, c)?;
        shape::add_text_box(h, c, sheet)
    };
    addLine "(startLeft: number, startTop: number, endLeft: number, endTop: number, connectorType?: ExcelScript.ConnectorType): Shape" "Draws a straight line between two points, in points from the top left of the worksheet." => |h, c| {
        let sheet = this_sheet(h, c)?;
        shape::add_line(h, c, sheet)
    };
    addImage "(base64ImageString: string): Shape" "Inserts a picture (a PNG, JPEG, GIF or BMP file as base64 text) at the top left of the worksheet, at its natural size." => |h, c| {
        let sheet = this_sheet(h, c)?;
        shape::add_image(h, c, sheet)
    };
    getShapes "(): Shape[]" "The shapes and pictures of the worksheet, the bottom-most first." => |h, c| Ok(shape::all(h, this_sheet(h, c)?));
    getShape "(key: string): Shape | undefined" "The shape of the worksheet with this name or ID; undefined when there is none." => |h, c| {
        let sheet = this_sheet(h, c)?;
        shape::find(h, c, sheet)
    };
    replaceAll "(text: string, replacement: string, criteria: ExcelScript.ReplaceCriteria): number" "Replaces a text in the constants and formulas of the worksheet; returns how many cells changed." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let (needle, replacement) = (c.str(0)?, c.str(1)?);
        let options = find_options(c, 2)?;
        h.permit_edit(c, &Edit::Replace { sheet, needle, options: &options })?;
        let changed = h.wb.replace_all(sheet, needle, replacement, &options);
        h.changed();
        Ok((changed as u64).into())
    };
    calculate "(markAllDirty: boolean): void" "Calculates the formulas of the worksheet now." => |h, c| {
        let sheet = this_sheet(h, c)?;
        h.calculate_now();
        h.wb.calculate_sheet(sheet);
        Ok(Ret::Nothing)
    };
});

pub(super) const FREEZE_PANES: ApiClass = class!("WorksheetFreezePanes", "The rows and columns of a worksheet that stay in place when it scrolls.", {
    freezeRows "(count?: number): void" "Freezes the first rows (one without a count); frozen columns are released." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1);
        let count = u32::try_from(count).ok().filter(|n| *n < MAX_ROWS).ok_or_else(|| c.invalid())?;
        h.wb.set_frozen(sheet, count, 0);
        Ok(Ret::Nothing)
    };
    freezeColumns "(count?: number): void" "Freezes the first columns (one without a count); frozen rows are released." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1);
        let count = u32::try_from(count).ok().filter(|n| *n < MAX_COLS).ok_or_else(|| c.invalid())?;
        h.wb.set_frozen(sheet, 0, count);
        Ok(Ret::Nothing)
    };
    freezeAt "(frozenRange: Range | string): void" "Freezes the rows and the columns of a range that starts at the top left: what is above and left of its end stays in place." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let (on, rect) = range_arg(h, c, 0, sheet)?;
        if on != sheet {
            return Err(c.invalid());
        }
        let rows = if rect.is_whole_cols() { 0 } else { rect.r1 + 1 };
        let cols = if rect.is_whole_rows() { 0 } else { rect.c1 + 1 };
        h.wb.set_frozen(sheet, rows, cols);
        Ok(Ret::Nothing)
    };
    unfreeze "(): void" "Releases the frozen rows and columns." => |h, c| {
        let sheet = this_sheet(h, c)?;
        h.wb.set_frozen(sheet, 0, 0);
        Ok(Ret::Nothing)
    };
    getLocation "(): Range | undefined" "The frozen cells; undefined when nothing is frozen." => |h, c| {
        let sheet = this_sheet(h, c)?;
        let sh = &h.wb.sheets[sheet];
        let rect = match (sh.frozen_rows, sh.frozen_cols) {
            (0, 0) => None,
            (rows, 0) => Some(Rect::new(0, 0, rows - 1, MAX_COLS - 1)),
            (0, cols) => Some(Rect::new(0, 0, MAX_ROWS - 1, cols - 1)),
            (rows, cols) => Some(Rect::new(0, 0, rows - 1, cols - 1)),
        };
        Ok(Ret::maybe(rect.map(|rect| h.range(sheet, rect))))
    };
});

pub(super) const AUTO_FILTER: ApiClass = class!("AutoFilter", "The filter of a worksheet or of a table: its buttons and what each column is filtered on.", {
    apply "(range: Range | string, columnIndex?: number, criteria?: ExcelScript.FilterCriteria): void" "Puts the filter on a range (header row first) and, with a column (0-based in the range) and criteria, filters that column." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        match scope {
            FilterScope::Sheet => {
                let (on, mut rect) = range_arg(h, c, 0, sheet)?;
                if on != sheet {
                    return Err(c.invalid());
                }
                h.calculate();
                if rect.is_single() {
                    rect = h.wb.current_region(sheet, rect.r0, rect.c0);
                }
                let rect = h.wb.clamp_to_used(sheet, rect);
                if h.wb.auto_filter(sheet, scope).is_none_or(|f| f.range != rect) {
                    h.permit(c, &Action::ToggleAutoFilter { sheet })?;
                    h.wb.set_auto_filter(sheet, rect).map_err(|e| c.fail(match e {
                        FilterError::Table => "A filter of the worksheet cannot run into a table.",
                        FilterError::NothingToFilter => "This can't be applied to the selected range.",
                    }))?;
                    h.restructured();
                }
            }
            FilterScope::Table(id) => {
                if h.wb.auto_filter(sheet, scope).is_none() {
                    h.permit(c, &Action::ChangeTable { sheet })?;
                    h.wb.set_table_option(sheet, id, TableOption::FilterButton, true).map_err(|e| table_fail(c, e))?;
                    h.restructured();
                }
            }
        }
        if c.has(1) {
            let criteria = c.has(2).then(|| criteria_from(c, c.arg(2))).transpose()?;
            if let Some(criteria) = criteria {
                set_criteria(h, c, sheet, scope, c.int(1)?, Some(criteria))?;
            }
        }
        Ok(Ret::Nothing)
    };
    clearCriteria "(): void" "Shows every row again, keeping the filter buttons." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        h.permit(c, &Action::UseAutoFilter { sheet })?;
        h.wb.clear_filters(sheet, scope);
        h.restructured();
        Ok(Ret::Nothing)
    };
    clearColumnCriteria "(columnIndex: number): void" "Removes the criteria of one column (0-based in the filter's range)." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        set_criteria(h, c, sheet, scope, c.int(0)?, None)?;
        Ok(Ret::Nothing)
    };
    getEnabled "(): boolean" "Whether the filter buttons are there." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        Ok(h.wb.auto_filter(sheet, scope).is_some().into())
    };
    getIsDataFiltered "(): boolean" "Whether a column is filtered." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        Ok(h.wb.auto_filter(sheet, scope).is_some_and(|f| f.is_filtered()).into())
    };
    getRange "(): Range | undefined" "The range the filter is on; undefined without a filter." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        Ok(Ret::maybe(h.wb.auto_filter(sheet, scope).map(|f| h.range(sheet, f.range))))
    };
    reapply "(): void" "Filters again with the values the cells have now." => |h, c| {
        let (sheet, _) = this_filter(h, c)?;
        h.permit(c, &Action::UseAutoFilter { sheet })?;
        h.calculate();
        h.wb.reapply_filters(sheet);
        h.restructured();
        Ok(Ret::Nothing)
    };
    remove "(): void" "Removes the filter: its buttons go and every row shows." => |h, c| {
        let (sheet, scope) = this_filter(h, c)?;
        match scope {
            FilterScope::Sheet => {
                if h.wb.auto_filter(sheet, scope).is_some() {
                    h.permit(c, &Action::ToggleAutoFilter { sheet })?;
                    h.wb.remove_auto_filter(sheet);
                }
            }
            FilterScope::Table(id) => {
                h.permit(c, &Action::ChangeTable { sheet })?;
                h.wb.set_table_option(sheet, id, TableOption::FilterButton, false).map_err(|e| table_fail(c, e))?;
            }
        }
        h.restructured();
        Ok(Ret::Nothing)
    };
});
