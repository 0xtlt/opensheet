//! The Range class: values, formulas, number formats, getting around,
//! and the editing commands.

mod common;
use common::*;
use opensheet_engine::Rect;
use opensheet_engine::locale::Locale;
use opensheet_engine::workbook::Workbook;
use opensheet_script::Context;

/// A small list with a header row.
fn stock() -> Workbook {
    book(&[
        ("A1", "Name"),
        ("B1", "Qty"),
        ("C1", "Price"),
        ("A2", "Pen"),
        ("B2", "10"),
        ("C2", "1.5"),
        ("A3", "Ink"),
        ("B3", "2"),
        ("C3", "12.25"),
        ("A4", "Pad"),
        ("B4", "5"),
        ("C4", "3"),
    ])
}

#[test]
fn values_formulas_and_number_formats() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/values.ts"));
    assert_eq!(number(&wb, "D3"), 24.5);
    assert_eq!(text(&wb, "C3"), "12.3");
}

#[test]
fn from_a_range_to_another() {
    let mut wb = stock();
    let context = Context {
        selection: Rect::new(1, 1, 2, 2),
        active_cell: (2, 2),
        ..Context::default()
    };
    let out = ok_with(&mut wb, include_str!("model/navigation.ts"), &context);
    assert_eq!(out.selected, Some((0, Rect::new(1, 0, 3, 0))));
    assert_eq!(out.activated, Some(0));
}

#[test]
fn english_and_the_language_of_the_workbook() {
    let mut wb = book_in(Locale::fr(), &[("A1", "1,5"), ("A2", "2,5")]);
    ok(
        &mut wb,
        r#"function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  sheet.getRange("B1").setFormula("=SUM(A1:A2)");
  eq(sheet.getRange("B1").getFormulaLocal(), "=SOMME(A1:A2)");
  eq(sheet.getRange("B1").getFormula(), "=SUM(A1:A2)");
  eq(sheet.getRange("B1").getValue(), 4);
  sheet.getRange("B2").setFormulaLocal("=MOYENNE(A1;A2)");
  eq(sheet.getRange("B2").getFormula(), "=AVERAGE(A1,A2)");
  eq(sheet.getRange("B1:B2").getFormulasLocal(), [["=SOMME(A1:A2)"], ["=MOYENNE(A1;A2)"]]);
  sheet.getRange("C1").setValue("1.25");
  eq(sheet.getRange("C1").getValue(), 1.25);
  sheet.getRange("C1").setNumberFormatLocal("0,000");
  eq(sheet.getRange("C1").getNumberFormat(), "0.000");
  eq(sheet.getRange("C1").getNumberFormatLocal(), "0,000");
  eq(sheet.getRange("C1").getText(), "1,250");
  sheet.getRange("C2").setValue("12/31/2024");
  eq(sheet.getRange("C2").getValue(), 45657);
  sheet.getRange("C3").setFormula("=IF(A1>1,TRUE,FALSE)");
  eq(sheet.getRange("C3").getValue(), true);
  eq(sheet.getRange("C3").getText(), "VRAI");
  eq(workbook.addNamedItem("Somme", "=SUM(Feuil1!$A$1:$A$2)").getValue(), 4);
  eq(workbook.addNamedItemFormulaLocal("Moyenne", "=MOYENNE(Feuil1!$A$1:$A$2)").getFormula(), "=AVERAGE(Feuil1!$A$1:$A$2)");
}"#,
    );
    assert_eq!(wb.locale.id, "fr");
    assert_eq!(text(&wb, "B2"), "2");
}

#[test]
fn cells_that_move_copy_and_clear() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/editing.ts"));
    assert_eq!(wb.sheets[0].merges, [Rect::new(0, 13, 1, 14), Rect::new(3, 13, 3, 14), Rect::new(4, 13, 4, 14)]);
    assert_eq!(wb.sheets.len(), 2);
}

#[test]
fn sort_search_fill_link_and_hide() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/data.ts"));
    assert!(wb.sheets[0].hidden_rows.contains(&2) && !wb.sheets[0].hidden_rows.contains(&3));
    assert!(wb.sheets[0].hidden_cols.contains(&1) && !wb.sheets[0].hidden_cols.contains(&2));
    assert_eq!(wb.sheets[0].hyperlinks.len(), 0);
}
