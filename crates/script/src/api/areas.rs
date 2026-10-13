//! `RangeAreas`: several ranges of one worksheet held together, as a
//! selection made with ⌘ held, the blanks of a range or the cells of a
//! conditional format.

use serde_json::Value as J;

use opensheet_engine::find::Search;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::value::Value;
use opensheet_engine::workbook::{Content, Workbook};

use super::range::{clear_rect, format_of, formula_of, text_of, type_of, used_within, value_of};
use super::util::{address, range_from, resolve_areas};
use super::worksheet::find_options;
use super::{ApiClass, Call, Res, Ret, class, enum_value, object, state_of};
use crate::host::Host;

/// A RangeAreas object: the sheet, then four numbers per range.
pub(super) fn areas_object(h: &Host, sheet: usize, rects: &[Rect]) -> J {
    let mut state = vec![J::from(h.sheet_id(sheet))];
    for rect in rects {
        state.extend([rect.r0, rect.c0, rect.r1, rect.c1].map(J::from));
    }
    object("RangeAreas", state)
}

/// The sheet and the ranges of a RangeAreas object, from its state.
pub(super) fn areas_from(h: &Host, call: &Call, state: &[J]) -> Res<(usize, Vec<Rect>)> {
    let id = state.first().and_then(J::as_i64).ok_or_else(|| call.invalid())?;
    let sheet = h.sheet(call, id)?;
    let numbers: Vec<u32> = state[1..].iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect();
    if numbers.is_empty() || numbers.len() != state.len() - 1 || numbers.len() % 4 != 0 {
        return Err(call.invalid());
    }
    Ok((sheet, numbers.chunks(4).map(|n| Rect::new(n[0], n[1], n[2], n[3])).collect()))
}

pub(super) fn this_areas(h: &Host, call: &Call) -> Res<(usize, Vec<Rect>)> {
    areas_from(h, call, call.this)
}

/// An argument that is a Range, a RangeAreas or addresses separated by
/// commas (`A1:B2, D4`), read from `sheet`.
pub(super) fn areas_arg(h: &mut Host, call: &Call, index: usize, sheet: usize) -> Res<(usize, Vec<Rect>)> {
    let value = call.arg(index);
    if let J::String(text) = value {
        return resolve_areas(h, call, sheet, text);
    }
    if let Some(state) = state_of(value, "RangeAreas") {
        return areas_from(h, call, state);
    }
    match state_of(value, "Range") {
        Some(state) => range_from(h, call, state).map(|(sheet, rect)| (sheet, vec![rect])),
        None => Err(call.invalid()),
    }
}

/// Cells as the ranges a reader would name: runs along a row, joined
/// down the rows when they span the same columns (the ranges Go To
/// Special selects).
pub(super) fn rects_of_cells(mut cells: Vec<(u32, u32)>) -> Vec<Rect> {
    cells.sort_unstable();
    cells.dedup();
    let mut runs: Vec<Rect> = Vec::new();
    for (row, col) in cells {
        match runs.last_mut() {
            Some(run) if run.r0 == row && run.c1 + 1 == col => run.c1 = col,
            _ => runs.push(Rect::cell(row, col)),
        }
    }
    let mut out: Vec<Rect> = Vec::new();
    for run in runs {
        match out.iter_mut().find(|rect| rect.c0 == run.c0 && rect.c1 == run.c1 && rect.r1 + 1 == run.r0) {
            Some(rect) => rect.r1 = run.r0,
            None => out.push(run),
        }
    }
    out
}

/// `Sheet1!A1:B2,Sheet1!D4`, the ranges parted as lists are in a language.
fn addresses(h: &Host, sheet: usize, rects: &[Rect], separator: char) -> String {
    let each: Vec<String> = rects.iter().map(|rect| address(h, sheet, *rect)).collect();
    each.join(&separator.to_string())
}

/// `getSpecialCells`: the cells of a kind among those of `rects`, as Go
/// To Special finds them; never beyond the cells in use, except for the
/// visible cells.
pub(super) fn special(h: &mut Host, call: &Call, sheet: usize, rects: &[Rect]) -> Res<Ret> {
    let kind = enum_value(call, 0, "SpecialCellType")?.ok_or_else(|| call.invalid())?;
    let values = enum_value(call, 1, "SpecialCellValueType")?.unwrap_or("All");
    let wanted = |value: &Value| {
        let named = |part: &str| values == "All" || values.contains(part);
        match value {
            Value::Number(_) => named("Number"),
            Value::Text(_) => named("Text"),
            Value::Bool(_) => named("Logical"),
            Value::Error(_) => named("Errors"),
            Value::Empty | Value::Array(_) => false,
        }
    };
    h.calculate();
    let sh = &h.wb.sheets[sheet];
    let used: Vec<Rect> = sh.used_rect().map_or(Vec::new(), |used| rects.iter().filter_map(|rect| rect.intersection(&used)).collect());
    // The cells of the sheet that lie in the ranges and pass a test.
    let cells_where = |keep: &dyn Fn(u32, u32, &Content) -> bool| -> Vec<(u32, u32)> {
        let mut found = Vec::new();
        for area in &used {
            for ((row, col), cell) in sh.cells.range((area.r0, 0)..=(area.r1, u32::MAX)) {
                if *col >= area.c0 && *col <= area.c1 && keep(*row, *col, &cell.content) {
                    found.push((*row, *col));
                }
            }
        }
        found
    };
    // The parts of ranges of rules that lie in the ranges asked.
    let shared = |ranges: Vec<Rect>| -> Vec<Rect> {
        let mut out: Vec<Rect> = Vec::new();
        for part in ranges.iter().flat_map(|range| rects.iter().filter_map(|rect| rect.intersection(range))) {
            if !out.contains(&part) {
                out.push(part);
            }
        }
        out
    };
    let first = rects.first().map(|rect| (rect.r0, rect.c0)).unwrap_or((0, 0));
    let found: Vec<Rect> = match kind {
        "Constants" => rects_of_cells(cells_where(&|_, _, content| matches!(content, Content::Value(value) if wanted(value)))),
        "Formulas" => rects_of_cells(cells_where(&|row, col, content| matches!(content, Content::Formula(_)) && wanted(&sh.value_at(row, col)))),
        "Blanks" => {
            let mut blank = Vec::new();
            for area in &used {
                h.sized(call, *area)?;
                for row in area.r0..=area.r1 {
                    blank.extend((area.c0..=area.c1).filter(|col| sh.is_blank(row, *col)).map(|col| (row, col)));
                }
            }
            rects_of_cells(blank)
        }
        "Visible" => {
            let hidden_rows = sh.rows_not_shown();
            let mut out = Vec::new();
            for rect in rects {
                // Runs of rows that show, crossed with runs of columns that show.
                let runs = |from: u32, to: u32, hidden: &std::collections::BTreeSet<u32>| -> Vec<(u32, u32)> {
                    let mut runs = Vec::new();
                    let mut start = from;
                    for gone in hidden.range(from..=to) {
                        if *gone > start {
                            runs.push((start, *gone - 1));
                        }
                        start = *gone + 1;
                    }
                    if start <= to {
                        runs.push((start, to));
                    }
                    runs
                };
                let cols = runs(rect.c0, rect.c1, &sh.hidden_cols);
                for (r0, r1) in runs(rect.r0, rect.r1, &hidden_rows) {
                    out.extend(cols.iter().map(|(c0, c1)| Rect::new(r0, *c0, r1, *c1)));
                }
            }
            out
        }
        "ConditionalFormats" => shared(sh.cond_formats.iter().flat_map(|cf| cf.ranges.iter().copied()).collect()),
        "SameConditionalFormat" => shared(sh.cond_formats.iter().filter(|cf| cf.applies_to(first.0, first.1)).flat_map(|cf| cf.ranges.iter().copied()).collect()),
        "DataValidations" => shared(sh.validations.iter().flat_map(|rule| rule.ranges.iter().copied()).collect()),
        "SameDataValidation" => shared(sh.validations.iter().filter(|rule| rule.contains(first.0, first.1)).flat_map(|rule| rule.ranges.iter().copied()).collect()),
        _ => return Err(call.invalid()),
    };
    Ok(Ret::maybe((!found.is_empty()).then(|| areas_object(h, sheet, &found))))
}

/// `findAll`: every cell of the ranges (of the sheet when there is none)
/// holding a text.
pub(super) fn find_all(h: &mut Host, call: &Call, sheet: usize, within: &[Rect]) -> Res<Ret> {
    let needle = call.str(0)?;
    let options = find_options(call, 1)?;
    h.calculate();
    let search = Search { needle, options: &options, workbook: false, within };
    let cells: Vec<(u32, u32)> = h.wb.find_everywhere(&search, sheet).into_iter().map(|(_, row, col)| (row, col)).collect();
    Ok(Ret::maybe((!cells.is_empty()).then(|| areas_object(h, sheet, &rects_of_cells(cells)))))
}

/// The ranges moved by an offset, when they all stay on the sheet.
fn moved(rects: &[Rect], rows: i64, cols: i64) -> Option<Vec<Rect>> {
    rects
        .iter()
        .map(|rect| {
            let (r0, c0, r1, c1) = (rect.r0 as i64 + rows, rect.c0 as i64 + cols, rect.r1 as i64 + rows, rect.c1 as i64 + cols);
            (r0 >= 0 && c0 >= 0 && r1 < MAX_ROWS as i64 && c1 < MAX_COLS as i64).then(|| Rect::new(r0 as u32, c0 as u32, r1 as u32, c1 as u32))
        })
        .collect()
}

// ---- the view of what shows -------------------------------------------------------

/// The sheet, the rows and the columns of a RangeView: those of its range
/// that show, or the one row a view of a row stands for.
fn view(h: &Host, call: &Call) -> Res<(usize, Vec<u32>, Vec<u32>)> {
    let (sheet, rect) = range_from(h, call, call.this)?;
    let rect = h.wb.clamp_to_used(sheet, rect);
    h.sized(call, rect)?;
    let sh = &h.wb.sheets[sheet];
    let hidden = sh.rows_not_shown();
    let mut rows: Vec<u32> = (rect.r0..=rect.r1).filter(|row| !hidden.contains(row)).collect();
    let cols: Vec<u32> = (rect.c0..=rect.c1).filter(|col| !sh.hidden_cols.contains(col)).collect();
    if let Ok(index) = usize::try_from(call.this_int(5)) {
        rows = vec![*rows.get(index).ok_or_else(|| call.gone())?];
    }
    Ok((sheet, rows, cols))
}

/// One thing read from every cell of a view, as an array of rows.
fn view_grid(h: &mut Host, call: &Call, english: bool, cell: fn(&Workbook, usize, u32, u32) -> J) -> Res<Ret> {
    let (sheet, rows, cols) = view(h, call)?;
    h.calculate();
    let grid = |wb: &mut Workbook| rows.iter().map(|row| J::from(cols.iter().map(|col| cell(wb, sheet, *row, *col)).collect::<Vec<J>>())).collect::<Vec<J>>();
    Ok(Ret::Value(J::from(if english { h.english(grid) } else { grid(&mut h.wb) })))
}

pub(super) const RANGE_VIEW: ApiClass = class!("RangeView", "The cells of a range that show: the rows a filter or Hide left out, and the hidden columns, are skipped.", {
    getRange "(): Range" "The range the view is of; for a view of one row, that row of it." => |h, c| {
        let (sheet, rect) = range_from(h, c, c.this)?;
        match usize::try_from(c.this_int(5)) {
            Ok(_) => {
                let (_, rows, _) = view(h, c)?;
                Ok(Ret::Object(h.range(sheet, Rect::new(rows[0], rect.c0, rows[0], rect.c1))))
            }
            Err(_) => Ok(Ret::Object(h.range(sheet, rect))),
        }
    };
    getRowCount "(): number" "How many rows show." => |h, c| Ok((view(h, c)?.1.len() as u64).into());
    getColumnCount "(): number" "How many columns show." => |h, c| Ok((view(h, c)?.2.len() as u64).into());
    getIndex "(): number" "The place of a view of one row among the rows that show; -1 for the view of the whole range." => |h, c| {
        view(h, c)?;
        Ok(c.this_int(5).into())
    };
    getRows "(): RangeView[]" "One view per row that shows." => |h, c| {
        let (_, rows, _) = view(h, c)?;
        let whole = c.this_int(5) < 0;
        let each = (0..rows.len()).map(|index| {
            let mut state = c.this.to_vec();
            if whole {
                state[5] = index.into();
            }
            object("RangeView", state)
        });
        Ok(Ret::Object(J::from(each.collect::<Vec<J>>())))
    };
    getValues "(): (string | number | boolean)[][]" "The values of the cells that show." => |h, c| view_grid(h, c, false, value_of);
    getText "(): string[][]" "The cells that show, as they are displayed." => |h, c| view_grid(h, c, false, text_of);
    getFormulas "(): string[][]" "The formulas of the cells that show, in English; values for the cells without one." => |h, c| view_grid(h, c, true, formula_of);
    getNumberFormat "(): string[][]" "The number format codes of the cells that show, in English." => |h, c| view_grid(h, c, false, format_of);
    getValueTypes "(): ExcelScript.RangeValueType[][]" "The kind of value of each cell that shows." => |h, c| view_grid(h, c, false, type_of);
    getCellAddresses "(): string[][]" "The address of each cell that shows (Sheet1!A2)." => |h, c| {
        let (sheet, rows, cols) = view(h, c)?;
        let grid: Vec<J> = rows.iter().map(|row| J::from(cols.iter().map(|col| J::from(address(h, sheet, Rect::cell(*row, *col)))).collect::<Vec<J>>())).collect();
        Ok(Ret::Value(J::from(grid)))
    };
});

pub(super) const RANGE_AREAS: ApiClass = class!("RangeAreas", "Several ranges of one worksheet held together.", {
    getAddress "(): string" "The addresses of the ranges, separated by commas (Sheet1!A1:B2,Sheet1!D4)." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(addresses(h, sheet, &rects, ',').into())
    };
    getAddressLocal "(): string" "The addresses of the ranges, separated as lists are in the language of the workbook." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(addresses(h, sheet, &rects, h.wb.locale.arg_sep).into())
    };
    getAreaCount "(): number" "How many ranges there are." => |h, c| Ok((this_areas(h, c)?.1.len() as u64).into());
    getAreas "(): Range[]" "The ranges, one by one." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(Ret::Object(J::from(rects.into_iter().map(|rect| h.range(sheet, rect)).collect::<Vec<J>>())))
    };
    getCellCount "(): number" "How many cells the ranges hold together." => |h, c| {
        let cells: u64 = this_areas(h, c)?.1.iter().map(|rect| rect.rows() as u64 * rect.cols() as u64).sum();
        Ok(cells.into())
    };
    getWorksheet "(): Worksheet" "The worksheet of the ranges." => |h, c| {
        let (sheet, _) = this_areas(h, c)?;
        Ok(Ret::Object(h.worksheet(sheet)))
    };
    getIsEntireColumn "(): boolean" "Whether every range is made of whole columns." => |h, c| Ok(this_areas(h, c)?.1.iter().all(Rect::is_whole_cols).into());
    getIsEntireRow "(): boolean" "Whether every range is made of whole rows." => |h, c| Ok(this_areas(h, c)?.1.iter().all(Rect::is_whole_rows).into());
    getEntireColumn "(): RangeAreas" "The whole columns of the ranges." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        let whole: Vec<Rect> = rects.iter().map(|rect| Rect::new(0, rect.c0, MAX_ROWS - 1, rect.c1)).collect();
        Ok(Ret::Object(areas_object(h, sheet, &whole)))
    };
    getEntireRow "(): RangeAreas" "The whole rows of the ranges." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        let whole: Vec<Rect> = rects.iter().map(|rect| Rect::new(rect.r0, 0, rect.r1, MAX_COLS - 1)).collect();
        Ok(Ret::Object(areas_object(h, sheet, &whole)))
    };
    getOffsetRangeAreas "(rowOffset: number, columnOffset: number): RangeAreas" "The same ranges that many rows down and columns right (up and left when negative)." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        let moved = moved(&rects, c.int(0)?, c.int(1)?).ok_or_else(|| c.invalid())?;
        Ok(Ret::Object(areas_object(h, sheet, &moved)))
    };
    getUsedRangeAreas "(valuesOnly?: boolean): RangeAreas | undefined" "The used part of each range (of those with a value only, on request); undefined when none holds anything." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        let values_only = c.opt_bool(0)?.unwrap_or(false);
        let used: Vec<Rect> = rects.into_iter().filter_map(|rect| used_within(h, sheet, rect, values_only)).collect();
        Ok(Ret::maybe((!used.is_empty()).then(|| areas_object(h, sheet, &used))))
    };
    getIntersection "(anotherRange: Range | RangeAreas | string): RangeAreas | undefined" "The cells these ranges share with a range or with other ranges; undefined when they share none." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        let (on, others) = areas_arg(h, c, 0, sheet)?;
        let shared: Vec<Rect> = rects.iter().flat_map(|rect| others.iter().filter_map(|other| rect.intersection(other))).collect();
        Ok(Ret::maybe((on == sheet && !shared.is_empty()).then(|| areas_object(h, sheet, &shared))))
    };
    getSpecialCells "(cellType: ExcelScript.SpecialCellType, cellValueType?: ExcelScript.SpecialCellValueType): RangeAreas | undefined" "The cells of the ranges of a kind, as Go To Special finds them; undefined when there is none." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        special(h, c, sheet, &rects)
    };
    getFormat "(): RangeFormat" "The formatting of the ranges: what is set on it reaches every range." => |h, c| {
        let (_, rects) = this_areas(h, c)?;
        // The state of a Range, an empty place for the side of a border,
        // then the other ranges.
        let mut state: Vec<J> = c.this[..5].to_vec();
        state.push(J::Null);
        for rect in &rects[1..] {
            state.extend([rect.r0, rect.c0, rect.r1, rect.c1].map(J::from));
        }
        Ok(Ret::object("RangeFormat", state))
    };
    clear "(applyTo?: ExcelScript.ClearApplyTo): void" "Clears the cells of every range: everything (the default), contents, formats, or the hyperlinks." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        for rect in rects {
            clear_rect(h, c, sheet, rect)?;
        }
        Ok(Ret::Nothing)
    };
    calculate "(): void" "Calculates the formulas of the workbook now." => |h, c| {
        this_areas(h, c)?;
        h.calculate_now();
        Ok(Ret::Nothing)
    };
});
