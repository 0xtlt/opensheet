//! `Range`: values, formulas, number formats, getting from a range to
//! another, and the editing commands. `RangeSort` and the result of
//! `removeDuplicates` are here too.

use std::sync::Arc;

use serde_json::Value as J;

use opensheet_engine::ast::Expr;
use opensheet_engine::features::Edit;
use opensheet_engine::find::Search;
use opensheet_engine::format::{code_from_locale, code_to_locale};
use opensheet_engine::hyperlink::Hyperlink;
use opensheet_engine::ops::{Direction, SortError, SortLevel, SortOn, SortSpec};
use opensheet_engine::paste_special::{PasteSpecial, PasteWhat};
use opensheet_engine::protect::Action;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::table::TableEdit;
use opensheet_engine::value::Value;
use opensheet_engine::workbook::{ClearMode, Content, Workbook};

use super::util::{address, parse_color, range_arg, table_fail, this_range};
use super::worksheet::find_options;
use super::{ApiClass, Call, Res, Ret, class, enum_value, table};
use crate::host::{Host, to_script};

// ---- reading -----------------------------------------------------------------

/// One thing read from every cell of the range a call is made on, as the
/// array of rows scripts get, or for the first cell only.
fn read(h: &mut Host, call: &Call, single: bool, english: bool, cell: fn(&Workbook, usize, u32, u32) -> J) -> Res<Ret> {
    let (sheet, mut rect) = this_range(h, call)?;
    if single {
        rect = Rect::cell(rect.r0, rect.c0);
    }
    h.sized(call, rect)?;
    h.calculate();
    let grid = |wb: &mut Workbook| {
        (rect.r0..=rect.r1)
            .map(|row| J::from((rect.c0..=rect.c1).map(|col| cell(wb, sheet, row, col)).collect::<Vec<J>>()))
            .collect::<Vec<J>>()
    };
    let mut rows = if english { h.english(grid) } else { grid(&mut h.wb) };
    Ok(Ret::Value(if single {
        rows.pop().and_then(|row| row.as_array().and_then(|r| r.first().cloned())).unwrap_or(J::Null)
    } else {
        J::from(rows)
    }))
}

fn value_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    to_script(&wb.sheets[sheet].value_at(row, col))
}

fn text_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    wb.display_text(sheet, row, col).text.into()
}

/// The formula of a cell in the language the workbook is set to at the
/// time of the call; what it holds for a cell without one.
fn formula_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    match wb.cell(sheet, row, col).map(|cell| &cell.content) {
        Some(Content::Formula(_)) => wb.formula_text(sheet, row, col).unwrap_or_default().into(),
        Some(Content::Value(value)) => to_script(value),
        _ => J::from(""),
    }
}

fn format_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    wb.style_at(sheet, row, col).num_fmt.as_ref().into()
}

fn local_format_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    code_to_locale(&wb.style_at(sheet, row, col).num_fmt, wb.locale).into()
}

fn type_of(wb: &Workbook, sheet: usize, row: u32, col: u32) -> J {
    match wb.sheets[sheet].value_at(row, col) {
        Value::Empty => "Empty",
        Value::Number(_) => "Double",
        Value::Text(_) => "String",
        Value::Bool(_) => "Boolean",
        Value::Error(_) => "Error",
        Value::Array(_) => "Unknown",
    }
    .into()
}

// ---- writing -----------------------------------------------------------------

/// How what a script writes in a cell is read.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Entry {
    /// As typed in an English Excel: numbers, dates, `=` formulas.
    English,
    /// As typed in the language of the workbook.
    Local,
    /// A number format code, English or in the language of the workbook.
    Format,
    LocalFormat,
}

/// What follows cells being written: formulas are out of date, and
/// tables take in what was entered against them, rename the columns
/// whose header changed and fill their calculated columns.
pub(super) fn entered(h: &mut Host, sheet: usize, rect: Rect) {
    h.changed();
    if !h.wb.sheets[sheet].tables.is_empty() && h.wb.tables_after_edit(sheet, rect) != TableEdit::default() {
        h.restructured();
    }
}

pub(super) fn write_cell(h: &mut Host, call: &Call, sheet: usize, row: u32, col: u32, value: &J, entry: Entry) -> Res<()> {
    match (entry, value) {
        // As in Excel, null leaves the cell as it is.
        (_, J::Null) => {}
        (Entry::Format | Entry::LocalFormat, J::String(code)) => {
            let code = if entry == Entry::LocalFormat { code_from_locale(code, h.wb.locale) } else { code.clone() };
            let code = if code.is_empty() { "General".to_string() } else { code };
            h.wb.update_style(sheet, Rect::cell(row, col), &|style| style.num_fmt = code.as_str().into());
        }
        (Entry::Format | Entry::LocalFormat, _) => return Err(call.invalid()),
        (_, J::Number(n)) => h.wb.set_value(sheet, row, col, Value::Number(n.as_f64().unwrap_or(0.0))),
        (_, J::Bool(b)) => h.wb.set_value(sheet, row, col, Value::Bool(*b)),
        (_, J::String(text)) if text.is_empty() => h.wb.set_value(sheet, row, col, Value::Empty),
        (_, J::String(text)) => {
            let done = match entry {
                Entry::Local => h.wb.set_cell_text(sheet, row, col, text),
                _ => h.english(|wb| wb.set_cell_text(sheet, row, col, text)),
            };
            done.map_err(|error| call.fail(format!("The formula {text} is not valid ({}).", error.message)))?;
        }
        _ => return Err(call.invalid()),
    }
    Ok(())
}

/// Write one thing in every cell of the range, or an array of rows with
/// one item per cell.
fn write(h: &mut Host, call: &Call, grid: bool, entry: Entry) -> Res<Ret> {
    let (sheet, rect) = this_range(h, call)?;
    let formats = matches!(entry, Entry::Format | Entry::LocalFormat);
    // One format for whole rows or columns is kept on them, not cell by cell.
    if formats && !grid {
        let code = call.str(0)?;
        let code = if entry == Entry::LocalFormat { code_from_locale(code, h.wb.locale) } else { code.to_string() };
        let code = if code.is_empty() { "General".to_string() } else { code };
        h.permit_edit(call, &Edit::Format { sheet })?;
        h.wb.update_style(sheet, rect, &|style| style.num_fmt = code.as_str().into());
        h.changed();
        return Ok(Ret::Nothing);
    }
    h.sized(call, rect)?;
    let rows = if grid {
        let rows = call.array(0)?;
        let fits = rows.len() == rect.rows() as usize
            && rows.iter().all(|row| row.as_array().is_some_and(|r| r.len() == rect.cols() as usize));
        if !fits {
            return Err(call.fail("The number of rows or columns in the input array doesn't match the size or dimensions of the range."));
        }
        Some(rows)
    } else {
        None
    };
    if formats {
        h.permit_edit(call, &Edit::Format { sheet })?;
    } else {
        h.permit_edit(call, &Edit::Enter { sheet, ranges: &[rect] })?;
        h.spend(call, rect.rows() as u64 * rect.cols() as u64)?;
    }
    // One formula for several cells is entered as with Ctrl+Enter: its
    // relative references follow each cell.
    let mut filled: Option<Arc<Expr>> = None;
    for row in rect.r0..=rect.r1 {
        for col in rect.c0..=rect.c1 {
            if let Some(formula) = &filled {
                let moved = formula.shifted((row - rect.r0) as i64, (col - rect.c0) as i64);
                h.wb.set_formula(sheet, row, col, moved);
                continue;
            }
            let value = match rows {
                Some(rows) => &rows[(row - rect.r0) as usize][(col - rect.c0) as usize],
                None => call.arg(0),
            };
            write_cell(h, call, sheet, row, col, value, entry)?;
            if rows.is_none() {
                if let Some(Content::Formula(formula)) = h.wb.cell(sheet, row, col).map(|cell| &cell.content) {
                    filled = Some(formula.clone());
                }
            }
        }
    }
    if formats {
        h.changed();
    } else {
        entered(h, sheet, rect);
    }
    Ok(Ret::Nothing)
}

// ---- from a range to another -------------------------------------------------

/// A range of the same sheet from signed coordinates; fails off the sheet.
fn at(h: &Host, call: &Call, sheet: usize, r0: i64, c0: i64, r1: i64, c1: i64) -> Res<Ret> {
    let inside = |row: i64, col: i64| (0..MAX_ROWS as i64).contains(&row) && (0..MAX_COLS as i64).contains(&col);
    if !inside(r0, c0) || !inside(r1, c1) || r1 < r0 || c1 < c0 {
        return Err(call.invalid());
    }
    Ok(Ret::Object(h.range(sheet, Rect::new(r0 as u32, c0 as u32, r1 as u32, c1 as u32))))
}

fn direction(call: &Call, index: usize) -> Res<Direction> {
    Ok(match enum_value(call, index, "KeyboardDirection")?.ok_or_else(|| call.invalid())? {
        "Left" => Direction::Left,
        "Right" => Direction::Right,
        "Up" => Direction::Up,
        _ => Direction::Down,
    })
}

/// Where Ctrl+Arrow lands from the active cell of a range: the cell the
/// call names, the first of the range without one.
fn edge(h: &mut Host, call: &Call, sheet: usize, rect: Rect) -> Res<(Direction, (u32, u32))> {
    let towards = direction(call, 0)?;
    let from = if call.has(1) {
        let (on, cell) = range_arg(h, call, 1, sheet)?;
        if on != sheet {
            return Err(call.invalid());
        }
        (cell.r0, cell.c0)
    } else {
        (rect.r0, rect.c0)
    };
    h.calculate();
    Ok((towards, h.wb.data_edge(sheet, from.0, from.1, towards)))
}

/// The smallest rectangle around the cells of `rect` that are used.
fn used_within(h: &mut Host, sheet: usize, rect: Rect, values_only: bool) -> Option<Rect> {
    h.calculate();
    let sh = &h.wb.sheets[sheet];
    let mut out: Option<Rect> = None;
    let mut add = |found: Rect| {
        if let Some(part) = found.intersection(&rect) {
            out = Some(out.map_or(part, |o| o.union(&part)));
        }
    };
    for ((row, col), cell) in sh.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
        if *col >= rect.c0 && *col <= rect.c1 && !(values_only && matches!(cell.content, Content::Empty)) {
            add(Rect::cell(*row, *col));
        }
    }
    for spill in &sh.spills {
        add(Rect::new(spill.row, spill.col, spill.row + spill.array.rows as u32 - 1, spill.col + spill.array.cols as u32 - 1));
    }
    out
}

// ---- searching ---------------------------------------------------------------

fn sort_fail(call: &Call, error: SortError) -> super::Fail {
    match error {
        SortError::MergedCells => call.fail("To do this, all the merged cells need to be the same size."),
    }
}

/// The `SortField` objects of a sort as the levels of the Sort dialog.
pub(super) fn sort_levels(call: &Call, fields: &[J]) -> Res<Vec<SortLevel>> {
    fields
        .iter()
        .map(|field| {
            let key = field.get("key").and_then(J::as_u64).ok_or_else(|| call.invalid())?;
            let text = |name: &str| field.get(name).and_then(J::as_str);
            let color = || Ok(Some(parse_color(text("color").ok_or_else(|| call.invalid())?).ok_or_else(|| call.invalid())?));
            let on = match text("sortOn").unwrap_or("Value") {
                "Value" => SortOn::Values,
                "CellColor" => SortOn::CellColor(color()?),
                "FontColor" => SortOn::FontColor(color()?),
                "Icon" => return Err(call.unsupported("ExcelScript.SortOn.icon")),
                _ => return Err(call.invalid()),
            };
            if text("dataOption").is_some_and(|option| option != "Normal") {
                return Err(call.unsupported("ExcelScript.SortDataOption.textAsNumber"));
            }
            if field.get("subField").is_some_and(|sub| !sub.is_null()) {
                return Err(call.unsupported("SortField.subField"));
            }
            Ok(SortLevel {
                key: key as u32,
                on,
                descending: !field.get("ascending").and_then(J::as_bool).unwrap_or(true),
                custom_list: None,
            })
        })
        .collect()
}

pub(super) const RANGE: ApiClass = class!("Range", "A rectangle of cells of a worksheet. Rows and columns are counted from 0.", {
    getAddress "(): string" "The address with the name of the sheet: Sheet1!A1:B2." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(address(h, sheet, rect).into())
    };
    getAddressLocal "(): string" "The address in the language of the workbook." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(address(h, sheet, rect).into())
    };
    getWorksheet "(): Worksheet" "The worksheet the range is on." => |h, c| {
        let (sheet, _) = this_range(h, c)?;
        Ok(Ret::Object(h.worksheet(sheet)))
    };
    getRowCount "(): number" "How many rows the range has." => |h, c| Ok(this_range(h, c)?.1.rows().into());
    getColumnCount "(): number" "How many columns the range has." => |h, c| Ok(this_range(h, c)?.1.cols().into());
    getCellCount "(): number" "How many cells the range has." => |h, c| {
        let rect = this_range(h, c)?.1;
        Ok((rect.rows() as u64 * rect.cols() as u64).into())
    };
    getRowIndex "(): number" "The row of the first cell, 0-based." => |h, c| Ok(this_range(h, c)?.1.r0.into());
    getColumnIndex "(): number" "The column of the first cell, 0-based." => |h, c| Ok(this_range(h, c)?.1.c0.into());
    getIsEntireRow "(): boolean" "Whether the range is whole rows." => |h, c| Ok(this_range(h, c)?.1.is_whole_rows().into());
    getIsEntireColumn "(): boolean" "Whether the range is whole columns." => |h, c| Ok(this_range(h, c)?.1.is_whole_cols().into());

    getValue "(): string | number | boolean" "The value of the first cell: dates are serial numbers, an empty cell is the empty text, an error is its text." => |h, c| read(h, c, true, false, value_of);
    getValues "(): (string | number | boolean)[][]" "The values of the cells, row by row." => |h, c| read(h, c, false, false, value_of);
    setValue "(value: string | number | boolean): void" "Writes one value in every cell. Text is read as typed in English: =SUM(A1:A3) is a formula, 12/31/2024 a date." => |h, c| write(h, c, false, Entry::English);
    setValues "(values: (string | number | boolean)[][]): void" "Writes one value per cell; the array must have the size of the range. null leaves a cell as it is." => |h, c| write(h, c, true, Entry::English);
    getText "(): string" "What the first cell shows, with its number format." => |h, c| read(h, c, true, false, text_of);
    getTexts "(): string[][]" "What the cells show, row by row." => |h, c| read(h, c, false, false, text_of);
    getValueType "(): ExcelScript.RangeValueType" "The kind of value of the first cell: Empty, String, Double, Boolean or Error." => |h, c| read(h, c, true, false, type_of);
    getValueTypes "(): ExcelScript.RangeValueType[][]" "The kind of value of every cell." => |h, c| read(h, c, false, false, type_of);
    getFormula "(): string" "The formula of the first cell in English (=SUM(A1:A3)); its value for a cell without a formula." => |h, c| read(h, c, true, true, formula_of);
    getFormulas "(): string[][]" "The formulas of the cells in English, row by row." => |h, c| read(h, c, false, true, formula_of);
    setFormula "(formula: string): void" "Writes one formula, in English, in every cell, as Ctrl+Enter does: relative references follow each cell." => |h, c| write(h, c, false, Entry::English);
    setFormulas "(formulas: string[][]): void" "Writes one formula per cell, in English; the array must have the size of the range." => |h, c| write(h, c, true, Entry::English);
    getFormulaLocal "(): string" "The formula of the first cell in the language of the workbook." => |h, c| read(h, c, true, false, formula_of);
    getFormulasLocal "(): string[][]" "The formulas of the cells in the language of the workbook." => |h, c| read(h, c, false, false, formula_of);
    setFormulaLocal "(formulaLocal: string): void" "Writes one formula, in the language of the workbook, in every cell." => |h, c| write(h, c, false, Entry::Local);
    setFormulasLocal "(formulasLocal: string[][]): void" "Writes one formula per cell, in the language of the workbook." => |h, c| write(h, c, true, Entry::Local);
    getNumberFormat "(): string" "The number format code of the first cell, in English (#,##0.00)." => |h, c| read(h, c, true, false, format_of);
    getNumberFormats "(): string[][]" "The number format codes of the cells." => |h, c| read(h, c, false, false, format_of);
    setNumberFormat "(numberFormat: string): void" "Gives every cell one number format code, in English." => |h, c| write(h, c, false, Entry::Format);
    setNumberFormats "(numberFormats: string[][]): void" "Gives each cell its number format code; the array must have the size of the range." => |h, c| write(h, c, true, Entry::Format);
    getNumberFormatLocal "(): string" "The number format code of the first cell in the language of the workbook." => |h, c| read(h, c, true, false, local_format_of);
    getNumberFormatsLocal "(): string[][]" "The number format codes of the cells in the language of the workbook." => |h, c| read(h, c, false, false, local_format_of);
    setNumberFormatLocal "(numberFormatLocal: string): void" "Gives every cell one number format code, written in the language of the workbook." => |h, c| write(h, c, false, Entry::LocalFormat);
    setNumberFormatsLocal "(numberFormatsLocal: string[][]): void" "Gives each cell its number format code, written in the language of the workbook." => |h, c| write(h, c, true, Entry::LocalFormat);

    getFormat "(): RangeFormat" "The formatting of the range: font, fill, borders, alignment, sizes." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::object("RangeFormat", c.this.to_vec()))
    };
    getCell "(row: number, column: number): Range" "The cell at a row and a column counted from the first cell of the range; it may lie outside the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (row, col) = (rect.r0 as i64 + c.int(0)?, rect.c0 as i64 + c.int(1)?);
        at(h, c, sheet, row, col, row, col)
    };
    getRow "(row: number): Range" "One row of the range, counted from its first row." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let row = rect.r0 as i64 + c.int(0)?;
        at(h, c, sheet, row, rect.c0 as i64, row, rect.c1 as i64)
    };
    getColumn "(column: number): Range" "One column of the range, counted from its first column." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let col = rect.c0 as i64 + c.int(0)?;
        at(h, c, sheet, rect.r0 as i64, col, rect.r1 as i64, col)
    };
    getEntireRow "(): Range" "The whole rows of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::new(rect.r0, 0, rect.r1, MAX_COLS - 1))))
    };
    getEntireColumn "(): Range" "The whole columns of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::new(0, rect.c0, MAX_ROWS - 1, rect.c1))))
    };
    getOffsetRange "(rowOffset: number, columnOffset: number): Range" "The range of the same size that many rows down and columns right (up and left when negative)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (rows, cols) = (c.int(0)?, c.int(1)?);
        at(h, c, sheet, rect.r0 as i64 + rows, rect.c0 as i64 + cols, rect.r1 as i64 + rows, rect.c1 as i64 + cols)
    };
    getResizedRange "(deltaRows: number, deltaColumns: number): Range" "The range from the same first cell with that many rows and columns more (fewer when negative)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        at(h, c, sheet, rect.r0 as i64, rect.c0 as i64, rect.r1 as i64 + c.int(0)?, rect.c1 as i64 + c.int(1)?)
    };
    getAbsoluteResizedRange "(numRows: number, numColumns: number): Range" "The range from the same first cell with that many rows and columns." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        at(h, c, sheet, rect.r0 as i64, rect.c0 as i64, rect.r0 as i64 + c.int(0)? - 1, rect.c0 as i64 + c.int(1)? - 1)
    };
    getLastCell "(): Range" "The last cell of the range (bottom right)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::cell(rect.r1, rect.c1))))
    };
    getLastRow "(): Range" "The last row of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::new(rect.r1, rect.c0, rect.r1, rect.c1))))
    };
    getLastColumn "(): Range" "The last column of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::new(rect.r0, rect.c1, rect.r1, rect.c1))))
    };
    getRowsAbove "(count?: number): Range" "The rows right above the range (one without a count), in its columns." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1).max(1);
        at(h, c, sheet, rect.r0 as i64 - count, rect.c0 as i64, rect.r0 as i64 - 1, rect.c1 as i64)
    };
    getRowsBelow "(count?: number): Range" "The rows right below the range (one without a count), in its columns." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1).max(1);
        at(h, c, sheet, rect.r1 as i64 + 1, rect.c0 as i64, rect.r1 as i64 + count, rect.c1 as i64)
    };
    getColumnsBefore "(count?: number): Range" "The columns right before the range (one without a count), in its rows." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1).max(1);
        at(h, c, sheet, rect.r0 as i64, rect.c0 as i64 - count, rect.r1 as i64, rect.c0 as i64 - 1)
    };
    getColumnsAfter "(count?: number): Range" "The columns right after the range (one without a count), in its rows." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let count = c.opt_int(0)?.unwrap_or(1).max(1);
        at(h, c, sheet, rect.r0 as i64, rect.c1 as i64 + 1, rect.r1 as i64, rect.c1 as i64 + count)
    };
    getRangeEdge "(direction: ExcelScript.KeyboardDirection, activeCell?: Range | string): Range" "The cell Ctrl+Arrow reaches from the first cell of the range (or from activeCell): the edge of the block of data in that direction." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (_, (row, col)) = edge(h, c, sheet, rect)?;
        Ok(Ret::Object(h.range(sheet, Rect::cell(row, col))))
    };
    getExtendedRange "(direction: ExcelScript.KeyboardDirection, activeCell?: Range | string): Range" "The range extended to the cell Ctrl+Shift+Arrow reaches in that direction." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (towards, (row, col)) = edge(h, c, sheet, rect)?;
        let extended = match towards {
            Direction::Up | Direction::Down => Rect::new(rect.r0.min(row), rect.c0, rect.r1.max(row), rect.c1),
            Direction::Left | Direction::Right => Rect::new(rect.r0, rect.c0.min(col), rect.r1, rect.c1.max(col)),
        };
        Ok(Ret::Object(h.range(sheet, extended)))
    };
    getSurroundingRegion "(): Range" "The block of data around the first cell of the range (Ctrl+A): it ends at blank rows and columns." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.calculate();
        let region = h.wb.current_region(sheet, rect.r0, rect.c0);
        Ok(Ret::Object(h.range(sheet, region)))
    };
    getUsedRange "(valuesOnly?: boolean): Range | undefined" "The smallest range holding the used cells of this range (those with a value only, on request); undefined when there is none." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let values_only = c.opt_bool(0)?.unwrap_or(false);
        Ok(Ret::maybe(used_within(h, sheet, rect, values_only).map(|used| h.range(sheet, used))))
    };
    getIntersection "(anotherRange: Range | string): Range | undefined" "The cells this range shares with another; undefined when they share none." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (on, other) = range_arg(h, c, 0, sheet)?;
        Ok(Ret::maybe(rect.intersection(&other).filter(|_| on == sheet).map(|shared| h.range(sheet, shared))))
    };
    getBoundingRect "(anotherRange: Range | string): Range" "The smallest range holding this range and another of the same sheet." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (on, other) = range_arg(h, c, 0, sheet)?;
        if on != sheet {
            return Err(c.invalid());
        }
        Ok(Ret::Object(h.range(sheet, rect.union(&other))))
    };
    getTables "(fullyContained?: boolean): Table[]" "The tables that overlap the range (those wholly inside it, on request)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let inside = c.opt_bool(0)?.unwrap_or(false);
        let tables = h.wb.sheets[sheet]
            .tables
            .iter()
            .filter(|t| if inside { rect.intersection(&t.range) == Some(t.range) } else { rect.intersects(&t.range) })
            .map(|t| table::table_object(t.id))
            .collect::<Vec<J>>();
        Ok(Ret::Object(J::from(tables)))
    };

    clear "(applyTo?: ExcelScript.ClearApplyTo): void" "Clears the cells: everything (the default), contents, formats, or the hyperlinks." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let apply = enum_value(c, 0, "ClearApplyTo")?.unwrap_or("All");
        if matches!(apply, "Hyperlinks" | "RemoveHyperlinks") {
            h.permit(c, &Action::EditCells { sheet, ranges: &[rect] })?;
            h.wb.remove_hyperlinks(sheet, &[rect]);
            if apply == "RemoveHyperlinks" {
                h.wb.unformat_links(sheet, rect);
            }
            return Ok(Ret::Nothing);
        }
        let mode = match apply {
            "Contents" => ClearMode::Contents,
            "Formats" => ClearMode::Formats,
            _ => ClearMode::All,
        };
        h.permit_edit(c, &Edit::Clear { sheet, ranges: &[rect], mode })?;
        h.wb.clear(sheet, rect, mode);
        if mode == ClearMode::Formats {
            h.restructured();
        } else {
            entered(h, sheet, h.wb.clamp_to_used(sheet, rect));
            h.restructured();
        }
        Ok(Ret::Nothing)
    };
    delete "(shift: ExcelScript.DeleteShiftDirection): void" "Deletes the cells; those below move up, or those to the right move left. Whole rows and columns are deleted as such." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let up = enum_value(c, 0, "DeleteShiftDirection")?.ok_or_else(|| c.invalid())? == "Up";
        if up && rect.is_whole_rows() {
            h.permit_edit(c, &Edit::DeleteRows { sheet, at: rect.r0, count: rect.rows() })?;
            h.wb.delete_rows(sheet, rect.r0, rect.rows());
        } else if !up && rect.is_whole_cols() {
            h.permit_edit(c, &Edit::DeleteCols { sheet, at: rect.c0, count: rect.cols() })?;
            h.wb.delete_cols(sheet, rect.c0, rect.cols());
        } else {
            h.permit(c, &Action::ChangeSheet { sheet })?;
            h.wb.shift_cells(sheet, rect, up, false).map_err(|e| table_fail(c, e))?;
        }
        h.restructured();
        Ok(Ret::Nothing)
    };
    insert "(shift: ExcelScript.InsertShiftDirection): Range" "Inserts blank cells where the range is; what was there moves down or right. Whole rows and columns are inserted as such. Returns the new cells." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let down = enum_value(c, 0, "InsertShiftDirection")?.ok_or_else(|| c.invalid())? == "Down";
        if down && rect.is_whole_rows() {
            h.permit_edit(c, &Edit::InsertRows { sheet })?;
            h.wb.insert_rows(sheet, rect.r0, rect.rows());
        } else if !down && rect.is_whole_cols() {
            h.permit_edit(c, &Edit::InsertCols { sheet })?;
            h.wb.insert_cols(sheet, rect.c0, rect.cols());
        } else {
            h.permit(c, &Action::ChangeSheet { sheet })?;
            h.wb.shift_cells(sheet, rect, down, true).map_err(|e| table_fail(c, e))?;
        }
        h.restructured();
        Ok(Ret::Object(h.range(sheet, rect)))
    };
    copyFrom "(sourceRange: Range | string, copyType?: ExcelScript.RangeCopyType, skipBlanks?: boolean, transpose?: boolean): void" "Copies a range here, as Paste and Paste Special do: everything (the default), formulas, values, formats or links." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (from, source) = range_arg(h, c, 0, sheet)?;
        let kind = enum_value(c, 1, "RangeCopyType")?.unwrap_or("All");
        let spec = PasteSpecial {
            what: match kind {
                "Formulas" => PasteWhat::Formulas,
                "Values" => PasteWhat::Values,
                "Formats" => PasteWhat::Formats,
                _ => PasteWhat::All,
            },
            skip_blanks: c.opt_bool(2)?.unwrap_or(false),
            transpose: c.opt_bool(3)?.unwrap_or(false),
            ..PasteSpecial::default()
        };
        h.calculate();
        let clip = h.wb.copy(from, source);
        let area = if kind == "Link" { h.wb.paste_area(sheet, rect, &clip) } else { h.wb.paste_special_area(sheet, rect, &clip, &spec) };
        h.sized(c, area)?;
        h.spend(c, area.rows() as u64 * area.cols() as u64)?;
        h.permit_edit(c, &Edit::Paste { sheet, dest: area, clip: &clip, mode: spec.what.mode() })?;
        let written = if kind == "Link" { h.wb.paste_link(sheet, rect, &clip) } else { h.wb.paste_special(sheet, rect, &clip, &spec) };
        entered(h, sheet, written);
        h.restructured();
        Ok(Ret::Nothing)
    };
    moveTo "(destinationRange: Range | string): void" "Moves the cells to another place (cut and paste): formulas that read them follow." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (to, target) = range_arg(h, c, 0, sheet)?;
        let rect = h.wb.clamp_to_used(sheet, rect);
        h.permit_edit(c, &Edit::Move { src_sheet: sheet, src: rect, dest_sheet: to, dest_row: target.r0, dest_col: target.c0 })?;
        let landed = h.wb.move_range(sheet, rect, to, target.r0, target.c0);
        entered(h, to, landed);
        h.restructured();
        Ok(Ret::Nothing)
    };
    merge "(across?: boolean): void" "Merges the cells into one, or the cells of each row with across. Only the first value is kept." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.permit_edit(c, &Edit::Merge { sheet })?;
        if c.opt_bool(0)?.unwrap_or(false) {
            let rect = h.wb.clamp_to_used(sheet, rect);
            for row in rect.r0..=rect.r1 {
                h.wb.merge(sheet, Rect::new(row, rect.c0, row, rect.c1));
            }
        } else {
            h.wb.merge(sheet, rect);
        }
        h.restructured();
        Ok(Ret::Nothing)
    };
    unmerge "(): void" "Splits the merged cells of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.permit_edit(c, &Edit::Merge { sheet })?;
        h.wb.unmerge(sheet, rect);
        h.restructured();
        Ok(Ret::Nothing)
    };
    select "(): void" "Selects the range in the window, bringing its worksheet in front." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.activate(sheet);
        h.selection = rect;
        h.active_cell = (rect.r0, rect.c0);
        h.selected = true;
        Ok(Ret::Nothing)
    };
    getSort "(): RangeSort" "The sort of the range." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::object("RangeSort", c.this.to_vec()))
    };
    removeDuplicates "(columns: number[], includesHeader: boolean): RemoveDuplicatesResult" "Deletes the rows that repeat an earlier row in the given columns (0-based in the range)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let columns = c
            .array(0)?
            .iter()
            .map(|column| column.as_u64().map(|n| n as u32).filter(|n| *n < rect.cols()).ok_or_else(|| c.invalid()))
            .collect::<Res<Vec<u32>>>()?;
        if columns.is_empty() {
            return Err(c.invalid());
        }
        let header = c.bool(1)?;
        h.permit_edit(c, &Edit::RemoveDuplicates { sheet })?;
        h.calculate();
        let done = h.wb.remove_duplicates(sheet, rect, &columns, header).map_err(|e| sort_fail(c, e))?;
        h.restructured();
        Ok(Ret::object("RemoveDuplicatesResult", vec![(done.removed as u64).into(), (done.remaining as u64).into()]))
    };
    find "(text: string, criteria: ExcelScript.SearchCriteria): Range | undefined" "The first cell of the range holding a text (the last with searchDirection Backwards); undefined when none does." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let needle = c.str(0)?;
        let options = find_options(c, 1)?;
        let backwards = c.arg(1).get("searchDirection").and_then(J::as_str) == Some("Backwards");
        h.calculate();
        let search = Search { needle, options: &options, workbook: false, within: &[rect] };
        let found = h.wb.find_everywhere(&search, sheet);
        let hit = if backwards { found.last() } else { found.first() };
        Ok(Ret::maybe(hit.map(|(_, row, col)| h.range(sheet, Rect::cell(*row, *col)))))
    };
    replaceAll "(text: string, replacement: string, criteria: ExcelScript.ReplaceCriteria): number" "Replaces a text in the constants and formulas of the range; returns how many cells changed." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let (needle, replacement) = (c.str(0)?, c.str(1)?);
        let options = find_options(c, 2)?;
        h.permit_edit(c, &Edit::Replace { sheet, needle, options: &options })?;
        let search = Search { needle, options: &options, workbook: false, within: &[rect] };
        let done = h.wb.replace_everywhere(&search, sheet, replacement).map_err(|broken| {
            c.fail(format!("The replacement would leave a formula that is not valid: {}", broken.text))
        })?;
        entered(h, sheet, h.wb.clamp_to_used(sheet, rect));
        Ok((done.cells as u64).into())
    };
    autoFill "(destinationRange?: Range | string, autoFillType?: ExcelScript.AutoFillType): void" "Fills a larger range from this one as the fill handle does: series go on, formulas are copied." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        if !c.has(0) {
            return Err(c.unsupported("Range.autoFill without a destination range"));
        }
        let (on, dest) = range_arg(h, c, 0, sheet)?;
        match enum_value(c, 1, "AutoFillType")?.unwrap_or("FillDefault") {
            "FillDefault" => {}
            other => return Err(c.unsupported(format!("ExcelScript.AutoFillType.{}{}", other[..1].to_lowercase(), &other[1..]))),
        }
        let along_rows = dest.c0 == rect.c0 && dest.c1 == rect.c1 && dest.r0 <= rect.r0 && dest.r1 >= rect.r1;
        let along_cols = dest.r0 == rect.r0 && dest.r1 == rect.r1 && dest.c0 <= rect.c0 && dest.c1 >= rect.c1;
        let both_ends = (dest.r0 < rect.r0 && dest.r1 > rect.r1) || (dest.c0 < rect.c0 && dest.c1 > rect.c1);
        if on != sheet || !(along_rows || along_cols) || both_ends {
            return Err(c.fail("The destination must contain the range and extend it in one direction."));
        }
        h.sized(c, dest)?;
        h.spend(c, dest.rows() as u64 * dest.cols() as u64)?;
        h.permit_edit(c, &Edit::Fill { sheet, dest })?;
        h.calculate();
        h.wb.autofill(sheet, rect, dest);
        entered(h, sheet, dest);
        Ok(Ret::Nothing)
    };
    getHyperlink "(): ExcelScript.RangeHyperlink | undefined" "The hyperlink of the first cell ({address, documentReference, screenTip, textToDisplay}); undefined without one." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.calculate();
        let Some(link) = h.wb.hyperlink_at(sheet, rect.r0, rect.c0) else {
            return Ok(Ret::Nothing);
        };
        let mut out = serde_json::Map::new();
        if !link.address.is_empty() {
            out.insert("address".into(), link.address.clone().into());
        }
        if !link.location.is_empty() {
            out.insert("documentReference".into(), link.location.clone().into());
        }
        if !link.tooltip.is_empty() {
            out.insert("screenTip".into(), link.tooltip.clone().into());
        }
        out.insert("textToDisplay".into(), h.wb.display_text(sheet, rect.r0, rect.c0).text.into());
        Ok(Ret::Value(J::Object(out)))
    };
    setHyperlink "(hyperlink: ExcelScript.RangeHyperlink): void" "Puts a hyperlink on the range: {address} for a web page or a file, {documentReference} for a place in the workbook, with screenTip and textToDisplay." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let given = c.arg(0);
        let text = |key: &str| given.get(key).and_then(J::as_str).filter(|s| !s.is_empty());
        let rect = h.wb.clamp_to_used(sheet, rect);
        let mut link = match (text("address"), text("documentReference")) {
            (Some(target), place) => {
                let mut link = Hyperlink::external(rect, target);
                if let Some(place) = place {
                    link.location = place.to_string();
                }
                link
            }
            (None, Some(place)) => Hyperlink::place(rect, place),
            (None, None) => return Err(c.invalid()),
        };
        link.tooltip = text("screenTip").unwrap_or_default().to_string();
        h.permit(c, &Action::InsertHyperlink { sheet, ranges: &[rect] })?;
        // As when a link is inserted by hand: cells that show nothing
        // show the target, and the cells look like links.
        let shown = text("textToDisplay").map(str::to_string);
        let target = if link.address.is_empty() { link.location.clone() } else { link.target() };
        h.sized(c, rect)?;
        for row in rect.r0..=rect.r1 {
            for col in rect.c0..=rect.c1 {
                match &shown {
                    Some(text) => h.wb.set_value(sheet, row, col, Value::text(text)),
                    None if h.wb.sheets[sheet].is_blank(row, col) => h.wb.set_value(sheet, row, col, Value::text(&target)),
                    None => {}
                }
            }
        }
        h.wb.set_hyperlink(sheet, link);
        h.wb.format_as_link(sheet, rect, false);
        entered(h, sheet, rect);
        Ok(Ret::Nothing)
    };
    getHidden "(): boolean" "Whether every cell of the range is hidden, by its row or by its column." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let sh = &h.wb.sheets[sheet];
        let rows = hidden_rows(sh, rect);
        let cols = (rect.c0..=rect.c1).all(|col| sh.hidden_cols.contains(&col));
        Ok((rows || cols).into())
    };
    getRowHidden "(): boolean" "Whether every row of the range is hidden (by hand or by a filter)." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(hidden_rows(&h.wb.sheets[sheet], rect).into())
    };
    setRowHidden "(rowHidden: boolean): void" "Hides or shows the rows of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let hidden = c.bool(0)?;
        h.permit_edit(c, &Edit::SizeRows { sheet })?;
        h.wb.set_rows_hidden(sheet, rect.r0, rect.r1, hidden);
        h.restructured();
        Ok(Ret::Nothing)
    };
    getColumnHidden "(): boolean" "Whether every column of the range is hidden." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let sh = &h.wb.sheets[sheet];
        Ok((rect.c0..=rect.c1).all(|col| sh.hidden_cols.contains(&col)).into())
    };
    setColumnHidden "(columnHidden: boolean): void" "Hides or shows the columns of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let hidden = c.bool(0)?;
        h.permit_edit(c, &Edit::SizeCols { sheet })?;
        h.wb.set_cols_hidden(sheet, rect.c0, rect.c1, hidden);
        Ok(Ret::Nothing)
    };
    calculate "(): void" "Calculates the formulas now." => |h, c| {
        this_range(h, c)?;
        h.calculate_now();
        Ok(Ret::Nothing)
    };
});

/// Whether every row of a rectangle is hidden.
fn hidden_rows(sheet: &opensheet_engine::Sheet, rect: Rect) -> bool {
    let hidden = sheet.rows_not_shown();
    hidden.range(rect.r0..=rect.r1).count() as u32 == rect.rows()
}

pub(super) const RANGE_SORT: ApiClass = class!("RangeSort", "The sort of a range.", {
    apply "(fields: ExcelScript.SortField[], matchCase?: boolean, hasHeaders?: boolean, orientation?: ExcelScript.SortOrientation, method?: ExcelScript.SortMethod): void" "Sorts the rows of the range on one or several columns ({key, ascending, sortOn, color}; key is 0-based in the range), or its columns with orientation Columns." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let levels = sort_levels(c, c.array(0)?)?;
        if enum_value(c, 4, "SortMethod")?.is_some_and(|method| method != "PinYin") {
            return Err(c.unsupported("ExcelScript.SortMethod.strokeCount"));
        }
        let rect = h.wb.clamp_to_used(sheet, rect);
        let spec = SortSpec {
            levels,
            has_header: c.opt_bool(2)?.unwrap_or(false),
            case_sensitive: c.opt_bool(1)?.unwrap_or(false),
            left_to_right: enum_value(c, 3, "SortOrientation")? == Some("Columns"),
        };
        let span = if spec.left_to_right { rect.rows() } else { rect.cols() };
        if spec.levels.is_empty() || spec.levels.iter().any(|level| level.key >= span) {
            return Err(c.invalid());
        }
        h.permit_edit(c, &Edit::Sort { sheet, range: rect })?;
        h.calculate();
        h.wb.sort_range(sheet, rect, &spec).map_err(|e| sort_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
});

pub(super) const REMOVE_DUPLICATES_RESULT: ApiClass = class!("RemoveDuplicatesResult", "What removeDuplicates did.", {
    getRemoved "(): number" "How many rows were deleted." => |_, c| Ok(c.this_int(0).into());
    getUniqueRemaining "(): number" "How many distinct rows are left." => |_, c| Ok(c.this_int(1).into());
});
