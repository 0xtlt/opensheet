//! The Workbook, Application, NamedItem and Worksheet classes.

mod common;
use common::*;
use opensheet_script::Context;

#[test]
fn worksheets_of_a_workbook() {
    let mut wb = book(&[("A1", "1")]);
    let context = Context {
        workbook_name: "Budget.xlsx".into(),
        ..Context::default()
    };
    let out = ok_with(&mut wb, include_str!("model/worksheets.ts"), &context);
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Sheet1 (3)", "Sheet1", "Sheet1 (5)", "Data", "Sheet1 (2)", "Sheet1 (4)"]);
    assert_eq!(out.activated, Some(3));
    assert!(!wb.sheets[3].show_gridlines && !wb.sheets[3].show_headers);
    assert_eq!(wb.sheets[3].tab_color.map(|c| c.value()), Some(0x00FF00));
}

#[test]
fn names_and_calculation() {
    let mut wb = book(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "4"), ("B2", "5"), ("B3", "6")]);
    ok(&mut wb, include_str!("model/names.ts"));
    assert_eq!(wb.names.keys().collect::<Vec<_>>(), ["DATA", "LABEL", "TOTAL"]);
    assert_eq!(wb.sheets[0].names.keys().collect::<Vec<_>>(), ["LOCAL", "TWICE"]);
    assert_eq!(number(&wb, "E1"), 101.0);
}
