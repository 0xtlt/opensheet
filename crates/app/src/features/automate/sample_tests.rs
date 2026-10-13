//! Each sample of the gallery, run by the real engine on a workbook made
//! for it. A sample that the engine cannot run is a sample the user
//! cannot run either.

use super::samples::SAMPLES;
use opensheet_engine::style::HAlign;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Locale, Rect, Value, Workbook};
use opensheet_script::{Context, Outcome};

/// A list with a header row in A1:C4 of the first sheet.
fn workbook() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let rows = [["Region", "Units", "Revenue"], ["West", "140", "2800"], ["North", "120", "2400"], ["South", "85", "1700"]];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    wb.commit("setup");
    wb
}

/// Run a sample as the window does: a snapshot, the script, one undo step.
fn run(id: &str, wb: &mut Workbook, selection: Rect) -> Outcome {
    let sample = SAMPLES.iter().find(|sample| sample.id == id).expect("a sample");
    assert_eq!(opensheet_script::check(sample.body), Vec::new(), "{id}");
    let context = Context {
        active_sheet: 0,
        selection,
        active_cell: (selection.r0, selection.c0),
        ..Context::default()
    };
    wb.snapshot();
    let outcome = opensheet_script::run(sample.body, wb, &context);
    assert_eq!(outcome.error, None, "{id}");
    wb.commit("Run Script");
    outcome
}

fn logged(outcome: &Outcome) -> Vec<&str> {
    outcome.logs.iter().map(|log| log.text.as_str()).collect()
}

#[test]
fn the_header_row_is_formatted() {
    let mut wb = workbook();
    let outcome = run("header", &mut wb, Rect::cell(0, 0));
    for col in 0..3 {
        let style = wb.style_at(0, 0, col);
        assert!(style.font.bold, "column {col}");
        assert_eq!(style.fill.map(|color| color.value()), Some(0x1F4E78));
        assert_eq!(style.font.color.map(|color| color.value()), Some(0xFFFFFF));
        assert_eq!(style.h_align, HAlign::Center);
    }
    // The rows under it are left alone.
    assert!(!wb.style_at(0, 1, 0).font.bold && wb.style_at(0, 1, 0).fill.is_none());
    assert_eq!(logged(&outcome), ["Formatted Sheet1!A1:C1."]);
    // One undo step takes the formats back.
    assert!(wb.undo());
    assert!(!wb.style_at(0, 0, 0).font.bold);

    // An empty sheet: a word in the output, no error.
    let mut empty = Workbook::new(Locale::en());
    assert_eq!(logged(&run("header", &mut empty, Rect::cell(0, 0))), ["The sheet is empty."]);
}

#[test]
fn a_total_row_is_added_under_the_list() {
    let mut wb = workbook();
    let outcome = run("total", &mut wb, Rect::cell(2, 1));
    assert_eq!(wb.value_at(0, 4, 0), Value::Text("Total".into()));
    assert_eq!(wb.value_at(0, 4, 1), Value::Number(345.0));
    assert_eq!(wb.value_at(0, 4, 2), Value::Number(6900.0));
    for col in [1, 2] {
        let cell = wb.cell(0, 4, col).expect("a total");
        assert!(matches!(cell.content, Content::Formula(_)), "column {col}");
    }
    assert!(wb.style_at(0, 4, 0).font.bold);
    assert_eq!(logged(&outcome), ["Added 2 sums in Sheet1!A5:C5."]);
}

#[test]
fn a_table_is_made_and_sorted() {
    let mut wb = workbook();
    run("table", &mut wb, Rect::cell(1, 1));
    let table = wb.sheets[0].tables.first().expect("a table");
    assert!(table.header_row && table.totals_row);
    assert_eq!(table.style.as_deref(), Some("TableStyleMedium2"));
    assert_eq!((table.range.r0, table.range.c0, table.range.c1), (0, 0, 2));
    // Sorted on the first column, A to Z.
    let regions: Vec<Value> = (1..4).map(|row| wb.value_at(0, row, 0)).collect();
    assert_eq!(regions, [Value::Text("North".into()), Value::Text("South".into()), Value::Text("West".into())]);
    assert_eq!(wb.value_at(0, 1, 1), Value::Number(120.0));

    // Filtered on its last column: some rows show, some do not.
    let filter = table.auto_filter.as_ref().expect("the filter of the table");
    assert_eq!(filter.columns.keys().copied().collect::<Vec<_>>(), [2]);
    let hidden = wb.sheets[0].filtered_rows.len();
    assert!(hidden > 0 && hidden < 3, "{hidden} rows filtered");

    // A sheet that has a table already gets no second one.
    let outcome = run("table", &mut wb, Rect::cell(1, 1));
    assert_eq!(wb.sheets[0].tables.len(), 1);
    assert_eq!(logged(&outcome), ["This sheet already has a table."]);
}

#[test]
fn hidden_rows_and_columns_are_shown_on_every_sheet() {
    let mut wb = workbook();
    let second = wb.add_sheet(1);
    wb.sheets[0].hidden_rows.extend([1, 2, 40]);
    wb.sheets[0].hidden_cols.insert(1);
    wb.sheets[second].hidden_cols.extend([0, 7]);
    wb.sheets[second].hidden_rows.insert(3);
    wb.commit("hide");
    let outcome = run("unhide", &mut wb, Rect::cell(0, 0));
    for sheet in &wb.sheets {
        assert!(sheet.hidden_rows.is_empty() && sheet.hidden_cols.is_empty(), "{}", sheet.name);
    }
    assert_eq!(logged(&outcome), ["Rows and columns shown on 2 sheet(s)."]);
}

#[test]
fn a_summary_sheet_lists_the_others() {
    let mut wb = workbook();
    let second = wb.add_sheet(1);
    wb.set_cell_text(second, 1, 1, "x").unwrap();
    wb.add_sheet(2);
    wb.commit("sheets");
    let outcome = run("summary", &mut wb, Rect::cell(0, 0));
    let summary = wb.sheet_index("Summary").expect("the summary sheet");
    assert_eq!(outcome.activated, Some(summary));
    let row = |r: u32| -> Vec<Value> { (0..4).map(|c| wb.value_at(summary, r, c)).collect() };
    assert_eq!(row(0), ["Sheet", "Used range", "Rows", "Columns"].map(|text| Value::Text(text.into())));
    assert_eq!(row(1), [Value::Text("Sheet1".into()), Value::Text("Sheet1!A1:C4".into()), Value::Number(4.0), Value::Number(3.0)]);
    assert_eq!(row(2)[2..], [Value::Number(1.0), Value::Number(1.0)]);
    // An empty sheet is listed with no range.
    assert_eq!(row(3)[2..], [Value::Number(0.0), Value::Number(0.0)]);
    assert!(wb.style_at(summary, 0, 0).font.bold);

    // Run again: the same sheet is written anew, no second one is added.
    let sheets = wb.sheets.len();
    run("summary", &mut wb, Rect::cell(0, 0));
    assert_eq!(wb.sheets.len(), sheets);
    assert_eq!(wb.value_at(summary, 4, 0), Value::Empty);
}

#[test]
fn cells_above_the_average_are_filled() {
    let mut wb = workbook();
    // Units: 140, 120, 85 — the average is 115.
    let outcome = run("average", &mut wb, Rect::new(1, 1, 3, 1));
    let filled: Vec<bool> = (1..4).map(|row| wb.style_at(0, row, 1).fill.map(|color| color.value()) == Some(0xC6EFCE)).collect();
    assert_eq!(filled, [true, true, false]);
    assert_eq!(logged(&outcome), ["Average: 115. Cells above it: 2."]);

    // A selection without numbers: a word in the output, nothing filled.
    let outcome = run("average", &mut wb, Rect::new(0, 0, 3, 0));
    assert_eq!(logged(&outcome), ["There is no number in the selection."]);
    assert!(wb.style_at(0, 1, 0).fill.is_none());
}
