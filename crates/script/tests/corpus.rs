//! Scripts of the kind Excel's samples show, run as the application runs
//! them (one undo step each, undone and redone by `ok`) and checked on
//! the workbook they leave.

mod common;
use common::*;
use opensheet_engine::Rect;
use opensheet_engine::style::{BorderStyle, HAlign};
use opensheet_engine::value::Value;
use opensheet_engine::workbook::Workbook;
use opensheet_script::Context;

fn sales() -> Workbook {
    book(&[
        ("A1", "Region"),
        ("B1", "Rep"),
        ("C1", "Amount"),
        ("A2", "North"),
        ("B2", "Ann"),
        ("C2", "1200"),
        ("A3", "South"),
        ("B3", "Bob"),
        ("C3", "800"),
        ("A4", "East"),
        ("B4", "Cy"),
        ("C4", "2500"),
        ("A5", "North"),
        ("B5", "Di"),
        ("C5", "300"),
        ("A6", "West"),
        ("B6", "Ed"),
        ("C6", "950"),
    ])
}

#[test]
fn format_a_header_row_and_fit_the_columns() {
    let mut wb = sales();
    let out = ok(&mut wb, include_str!("corpus/format_header.ts"));
    let header = wb.style_at(0, 0, 2).clone();
    assert!(header.font.bold);
    assert_eq!(header.fill.map(|c| c.value()), Some(0x4472C4));
    assert_eq!(header.font.color.map(|c| c.value()), Some(0xFFFFFF));
    assert_eq!(header.h_align, HAlign::Center);
    assert_eq!(header.borders.bottom.style, BorderStyle::Medium);
    assert!(!wb.style_at(0, 1, 0).font.bold);
    assert_eq!(wb.sheets[0].frozen_rows, 1);
    assert_eq!(wb.sheets[0].col_widths.len(), 3);
    assert_eq!(out.logs[0].text, "Formatted 3 headers over 5 rows.");
    assert_eq!(out.logs[0].line, Some(23));
    assert_eq!(out.autofit.len(), 1);
    // An empty sheet is said so and left alone.
    let mut empty = book(&[]);
    let out = ok(&mut empty, include_str!("corpus/format_header.ts"));
    assert_eq!(out.logs[0].text, "The sheet is empty.");
    assert!(!empty.can_undo());
}

#[test]
fn total_a_column() {
    let mut wb = book(&[
        ("A1", "Item"),
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
    ]);
    let out = ok(&mut wb, include_str!("corpus/total_column.ts"));
    assert_eq!(out.returned.as_deref(), Some("54.5"));
    assert_eq!(wb.formula_text(0, 2, 3).as_deref(), Some("=B3*C3"));
    assert_eq!(wb.formula_text(0, 4, 1).as_deref(), Some("=SUM(B2:B4)"));
    assert_eq!(wb.formula_text(0, 4, 3).as_deref(), Some("=SUM(D2:D4)"));
    assert_eq!(
        (
            text(&wb, "A5"),
            text(&wb, "B5"),
            text(&wb, "D5"),
            text(&wb, "D3")
        ),
        ("Total".into(), "17".into(), "54.50".into(), "24.50".into())
    );
    assert!(wb.style_at(0, 4, 0).font.bold);
    assert_eq!(wb.style_at(0, 4, 3).borders.top.style, BorderStyle::Double);
}

#[test]
fn make_a_table_sort_it_and_filter_it() {
    let mut wb = sales();
    let out = ok(&mut wb, include_str!("corpus/table_sort_filter.ts"));
    let table = &wb.sheets[0].tables[0];
    assert_eq!(
        (table.name.as_str(), table.range, table.totals_row),
        ("Sales", Rect::new(0, 0, 6, 2), true)
    );
    assert_eq!(table.style.as_deref(), Some("TableStyleMedium9"));
    let reps: Vec<String> = (2..=6).map(|row| text(&wb, &format!("B{row}"))).collect();
    assert_eq!(reps, ["Cy", "Ann", "Ed", "Bob", "Di"]);
    // South and West are filtered out; the total counts what is shown.
    assert_eq!(
        wb.sheets[0]
            .filtered_rows
            .iter()
            .copied()
            .collect::<Vec<u32>>(),
        [3, 4]
    );
    assert_eq!(number(&wb, "C7"), 4000.0);
    assert_eq!(out.logs[0].text, "3 of 5 rows shown, total 4,000");
}

#[test]
fn copy_rows_to_another_sheet() {
    let mut wb = book(&[
        ("A1", "Task"),
        ("B1", "Status"),
        ("A2", "Write"),
        ("B2", "Open"),
        ("A3", "Review"),
        ("B3", "Done"),
        ("A4", "Ship"),
        ("B4", "Open"),
    ]);
    wb.rename_sheet(0, "Tasks");
    wb.update_style(0, Rect::new(0, 0, 0, 1), &|style| style.font.bold = true);
    wb.set_col_width(0, 0, Some(120.0));
    wb.commit("setup");
    wb.clear_history();
    for round in 0..2 {
        let out = ok(&mut wb, include_str!("corpus/copy_between_sheets.ts"));
        assert_eq!(out.returned.as_deref(), Some("2"));
        assert_eq!(wb.sheets.len(), 2, "round {round}");
        assert_eq!(wb.sheets[1].name, "Open items");
        let cell = |name: &str| value(&wb, 1, name);
        assert_eq!(
            (cell("A1"), cell("A2"), cell("A3"), cell("B3"), cell("A4")),
            (
                Value::text("Task"),
                Value::text("Write"),
                Value::text("Ship"),
                Value::text("Open"),
                Value::Empty
            )
        );
        assert!(wb.style_at(1, 0, 0).font.bold);
        assert_eq!(wb.sheets[1].col_widths.get(&0), Some(&120.0));
        assert_eq!(wb.sheets[1].tab_color.map(|c| c.value()), Some(0x70AD47));
        assert_eq!(
            (out.activated, out.selected),
            (Some(1), Some((1, Rect::cell(0, 0))))
        );
    }
    // Without its sheet the script says why it stops.
    let mut other = book(&[("A1", "x")]);
    let error = fails(&mut other, include_str!("corpus/copy_between_sheets.ts"));
    assert_eq!(
        (error.message.as_str(), error.line),
        ("There is no sheet named Tasks.", Some(8))
    );
}

#[test]
fn build_a_summary_sheet() {
    let mut wb = sales();
    let second = wb.add_sheet(1);
    for (name, text) in [
        ("A1", "Rep"),
        ("B1", "Amount"),
        ("A2", "Flo"),
        ("B2", "10.5"),
        ("A3", "Gus"),
        ("B3", "n/a"),
    ] {
        let (row, col) = opensheet_engine::refs::parse_cell_name(name).unwrap();
        wb.set_cell_text(second, row, col, text).unwrap();
    }
    wb.add_sheet(2);
    wb.rename_sheet(1, "Q2 data");
    wb.commit("setup");
    wb.clear_history();
    for _ in 0..2 {
        let out = ok(&mut wb, include_str!("corpus/summary_sheet.ts"));
        assert_eq!(wb.sheets[0].name, "Summary");
        assert_eq!(wb.sheets.len(), 4);
        let cell = |name: &str| text(&wb, name);
        assert_eq!(
            (cell("A2"), cell("B2"), cell("C2")),
            ("Sheet1".into(), "5".into(), "5,750.00".into())
        );
        assert_eq!(
            (cell("A3"), cell("B3"), cell("C3")),
            ("Q2 data".into(), "2".into(), "10.50".into())
        );
        assert_eq!(
            (cell("A4"), cell("B4"), cell("C4")),
            ("All".into(), "7".into(), "5,760.50".into())
        );
        assert_eq!(wb.sheets[0].hyperlinks.len(), 2);
        assert_eq!(wb.sheets[0].hyperlinks[1].location, "'Q2 data'!A1");
        assert_eq!(out.activated, Some(0));
        assert_eq!(
            out.returned.as_deref(),
            Some(
                r#"[{"name":"Sheet1","rows":5,"total":5750},{"name":"Q2 data","rows":2,"total":10.5}]"#
            )
        );
        assert_eq!(out.logs[0].text, "Summarized 2 sheets.");
    }
}

#[test]
fn clean_blank_rows_and_untidy_text() {
    let mut wb = book(&[
        ("A1", "Name"),
        ("B1", "Count"),
        ("A3", "  Ann   Lee "),
        ("B3", "'12"),
        ("A4", " "),
        ("A6", "Bob"),
        ("B6", "'3.5"),
        ("A7", "Total"),
        ("B7", "=SUM(B2:B6)"),
    ]);
    wb.update_style(0, Rect::new(8, 0, 8, 1), &|style| style.font.italic = true);
    wb.commit("setup");
    wb.clear_history();
    let out = ok(&mut wb, include_str!("corpus/clean_blank_rows.ts"));
    // Rows 2, 4, 5 and 8 hold nothing, and row 9 only a format.
    assert_eq!(out.returned.as_deref(), Some("5"));
    let cell = |name: &str| value(&wb, 0, name);
    assert_eq!(
        (cell("A1"), cell("A2"), cell("B2"), cell("A3"), cell("B3")),
        (
            Value::text("Name"),
            Value::text("Ann Lee"),
            Value::Number(12.0),
            Value::text("Bob"),
            Value::Number(3.5)
        )
    );
    assert_eq!(
        (cell("A4"), cell("A5")),
        (Value::text("Total"), Value::Empty)
    );
}

#[test]
fn colour_cells_by_their_value() {
    let mut wb = book(&[
        ("A1", "Name"),
        ("B1", "Score"),
        ("A2", "Ann"),
        ("B2", "91"),
        ("A3", "Bob"),
        ("B3", "49.5"),
        ("A4", "Cy"),
        ("B4", "67"),
        ("A5", "Di"),
        ("B5", "absent"),
        ("A6", "Ed"),
        ("B6", "80"),
    ]);
    wb.update_style(0, Rect::new(0, 0, 0, 1), &|style| style.font.bold = true);
    wb.commit("setup");
    wb.clear_history();
    let out = ok(&mut wb, include_str!("corpus/color_by_value.ts"));
    let fill = |row: u32| wb.style_at(0, row, 1).fill.map(|c| c.value());
    assert_eq!(
        [fill(1), fill(2), fill(3), fill(4), fill(5)],
        [
            Some(0xC6E0B4),
            Some(0xF8CBAD),
            Some(0xFFE699),
            None,
            Some(0xC6E0B4)
        ]
    );
    assert!(wb.style_at(0, 2, 1).font.bold && !wb.style_at(0, 1, 1).font.bold);
    assert!(wb.style_at(0, 0, 2).font.bold);
    let grades: Vec<String> = (1..=6).map(|row| text(&wb, &format!("C{row}"))).collect();
    assert_eq!(grades, ["Grade", "Merit", "Fail", "Pass", "", "Merit"]);
    assert_eq!(
        out.returned.as_deref(),
        Some(r#"{"Merit":2,"Fail":1,"Pass":1}"#)
    );
    assert_eq!(out.logs[0].text, "{Merit: 2, Fail: 1, Pass: 1}");
}

#[test]
fn rename_and_reorder_sheets() {
    let mut wb = book(&[("A1", "Zebra")]);
    for (index, title) in [(1, "Mango"), (2, ""), (3, "Apple")] {
        wb.add_sheet(index);
        if !title.is_empty() {
            wb.set_cell_text(index, 0, 0, title).unwrap();
        }
    }
    wb.set_cell_text(1, 3, 0, "=Sheet1!A1").unwrap();
    wb.commit("setup");
    wb.clear_history();
    let context = Context {
        active_sheet: 3,
        ..Context::default()
    };
    let out = ok_with(&mut wb, include_str!("corpus/reorder_sheets.ts"), &context);
    assert_eq!(
        out.returned.as_deref(),
        Some(r#"["Apple","Mango","Sheet3","Zebra"]"#)
    );
    assert_eq!(
        wb.sheets
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Apple", "Mango", "Sheet3", "Zebra"]
    );
    assert_eq!(
        wb.sheets
            .iter()
            .map(|s| s.tab_color.is_some())
            .collect::<Vec<_>>(),
        [true, true, false, true]
    );
    // Formulas followed the sheet that was renamed.
    assert_eq!(wb.formula_text(1, 3, 0).as_deref(), Some("=Zebra!A1"));
    assert_eq!(wb.value_at(1, 3, 0), Value::text("Zebra"));
    assert_eq!(out.activated, Some(0));
}

fn orders() -> Workbook {
    let mut wb = book(&[
        ("A1", "Date"),
        ("B1", "Item"),
        ("C1", "Qty"),
        ("D1", "Price"),
        ("A2", "1/15/2024"),
        ("B2", "Pen"),
        ("C2", "10"),
        ("D2", "1.5"),
        ("A3", "2/29/2024"),
        ("B3", "Ink"),
        ("C3", "2"),
        ("D3", "12.25"),
    ]);
    let id = wb
        .add_table(0, Rect::new(0, 0, 2, 3), true)
        .expect("a table");
    wb.rename_table(0, id, "Orders").expect("a name");
    wb.commit("table");
    wb.clear_history();
    wb
}

#[test]
fn return_the_rows_of_a_table() {
    let mut wb = orders();
    let out = ok(&mut wb, include_str!("corpus/table_to_json.ts"));
    assert_eq!(
        out.returned.as_deref(),
        Some(
            r#"[{"Date":"1/15/2024","Item":"Pen","Qty":10,"Price":1.5},{"Date":"2/29/2024","Item":"Ink","Qty":2,"Price":12.25}]"#
        )
    );
    assert!(!wb.can_undo(), "reading changes nothing");
    let mut empty = book(&[("A1", "x")]);
    let error = fails(&mut empty, include_str!("corpus/table_to_json.ts"));
    assert_eq!(
        (error.message.as_str(), error.line),
        ("The workbook has no table.", Some(12))
    );
}

#[test]
fn fill_an_invoice_from_a_table() {
    let mut wb = orders();
    let out = ok(&mut wb, include_str!("corpus/invoice.ts"));
    assert_eq!(
        out.returned.as_deref(),
        Some(r#"{"number":"2024-017","total":47.4}"#)
    );
    assert_eq!(out.activated, Some(1));
    let cell = |name: &str| {
        wb.display_text(
            1,
            opensheet_engine::refs::parse_cell_name(name).unwrap().0,
            opensheet_engine::refs::parse_cell_name(name).unwrap().1,
        )
        .text
    };
    assert_eq!(
        (cell("A1"), cell("A4"), cell("B5"), cell("D4"), cell("D5")),
        (
            "Invoice 2024-017".into(),
            "2024-01-15".into(),
            "Ink".into(),
            "15.00".into(),
            "24.50".into()
        )
    );
    assert_eq!(
        (cell("C7"), cell("D7"), cell("D8"), cell("D9")),
        ("Net".into(), "39.50".into(), "7.90".into(), "47.40".into())
    );
    assert_eq!(wb.sheets[1].merges, [Rect::new(0, 0, 0, 3)]);
    assert!(wb.names.contains_key("TAXRATE"));
    assert_eq!(wb.style_at(1, 3, 1).borders.bottom.style, BorderStyle::Thin);
}
