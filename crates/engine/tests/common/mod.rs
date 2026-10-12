//! Helpers shared by the integration tests.
#![allow(dead_code)]

use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::table::{Table, TableColumn};
use opensheet_engine::{ErrorKind, Locale, Rect, Value, Workbook};

/// 0-based (row, col) of an A1-style name.
pub fn at(name: &str) -> (u32, u32) {
    parse_cell_name(name).unwrap_or_else(|| panic!("bad cell name {name}"))
}

/// Type `text` into a cell, as a user would.
pub fn enter(wb: &mut Workbook, sheet: usize, name: &str, text: &str) {
    let (r, c) = at(name);
    wb.set_cell_text(sheet, r, c, text)
        .unwrap_or_else(|e| panic!("{name}: cannot enter {text:?}: {e:?}"));
}

pub fn value(wb: &Workbook, sheet: usize, name: &str) -> Value {
    let (r, c) = at(name);
    wb.value_at(sheet, r, c)
}

pub fn num(n: f64) -> Value {
    Value::Number(n)
}

pub fn err(kind: ErrorKind) -> Value {
    Value::Error(kind)
}

/// Formula of a cell as the formula bar shows it.
pub fn shown(wb: &Workbook, sheet: usize, name: &str) -> String {
    let (r, c) = at(name);
    wb.formula_text(sheet, r, c).unwrap_or_default()
}

/// Enter `formula` in a spare cell of `sheet` and return its value.
pub fn calc(wb: &mut Workbook, sheet: usize, cell: &str, formula: &str) -> Value {
    enter(wb, sheet, cell, formula);
    wb.commit("formula");
    value(wb, sheet, cell)
}

/// A workbook of `sheets` sheets ("Sheet1", ...) in English.
pub fn workbook(sheets: usize) -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for i in 1..sheets {
        wb.add_sheet(i);
    }
    wb.commit("sheets");
    wb
}

/// Declare the cells of `range` (already filled, headers included) a table.
/// Only what references need is filled in: tables hold more than that.
#[allow(clippy::needless_update)]
pub fn add_table(wb: &mut Workbook, sheet: usize, name: &str, range: &str, totals_row: bool) {
    let range = Rect::parse(range).unwrap();
    let columns = (range.c0..=range.c1)
        .map(|c| TableColumn {
            name: match wb.value_at(sheet, range.r0, c) {
                Value::Text(t) => t.to_string(),
                other => panic!("header of column {c}: {other:?}"),
            },
            ..Default::default()
        })
        .collect();
    let id = wb
        .sheets
        .iter()
        .flat_map(|s| s.tables.iter().map(|t| t.id))
        .max()
        .unwrap_or(0)
        + 1;
    wb.sheets[sheet].tables.push(Table {
        id,
        name: name.to_string(),
        range,
        header_row: true,
        totals_row,
        columns,
        ..Default::default()
    });
}
