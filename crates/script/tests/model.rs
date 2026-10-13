//! The classes of the object model, one script per group of members,
//! and the proof that every member `api()` lists answers a call.

mod common;
use common::*;
use opensheet_engine::Rect;
use opensheet_engine::locale::Locale;
use opensheet_engine::style::BorderStyle;
use opensheet_engine::workbook::Workbook;
use opensheet_script::{Autofit, Context, api, enums, members_called};

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

fn worksheets() {
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

fn names() {
    let mut wb = book(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "4"), ("B2", "5"), ("B3", "6")]);
    ok(&mut wb, include_str!("model/names.ts"));
    assert_eq!(wb.names.keys().collect::<Vec<_>>(), ["DATA", "LABEL", "TOTAL"]);
    assert_eq!(wb.sheets[0].names.keys().collect::<Vec<_>>(), ["LOCAL", "TWICE"]);
    assert_eq!(number(&wb, "E1"), 101.0);
}

fn values() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/values.ts"));
    assert_eq!(number(&wb, "D3"), 24.5);
    assert_eq!(text(&wb, "C3"), "12.3");
}

fn navigation() {
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

fn editing() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/editing.ts"));
    assert_eq!(wb.sheets[0].merges, [Rect::new(0, 13, 1, 14), Rect::new(3, 13, 3, 14), Rect::new(4, 13, 4, 14)]);
    assert_eq!(wb.sheets.len(), 2);
}

fn data() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/data.ts"));
    assert!(wb.sheets[0].hidden_rows.contains(&2) && !wb.sheets[0].hidden_rows.contains(&3));
    assert!(wb.sheets[0].hidden_cols.contains(&1) && !wb.sheets[0].hidden_cols.contains(&2));
    assert_eq!(wb.sheets[0].hyperlinks.len(), 0);
}

fn format() {
    let mut wb = stock();
    let out = ok(&mut wb, include_str!("model/format.ts"));
    let header = wb.style_at(0, 0, 1).clone();
    assert!(header.font.bold && header.font.italic && !header.font.strike && header.wrap);
    assert_eq!((header.font.name.as_ref(), header.font.size), ("Arial", 28));
    assert_eq!(header.fill, None);
    assert_eq!(wb.style_at(0, 0, 2).fill.map(|c| c.value()), Some(0x4472C4));
    // The lines inside are the bottom of a cell and the top of the next.
    let middle = wb.style_at(0, 2, 1).borders;
    assert_eq!((middle.top.style, middle.bottom.style), (BorderStyle::Dotted, BorderStyle::Dotted));
    assert_eq!(middle.left.color.map(|c| c.value()), Some(0x808080));
    assert_eq!(wb.style_at(0, 1, 0).borders.top.style, BorderStyle::Hair);
    assert_eq!(wb.style_at(0, 1, 0).borders.left.style, BorderStyle::Thin);
    assert_eq!(wb.style_at(0, 3, 2).borders.right.style, BorderStyle::Medium);
    assert_eq!(wb.style_at(0, 3, 0).borders.bottom.style, BorderStyle::None);
    assert!(!wb.style_at(0, 2, 1).locked && wb.style_at(0, 2, 1).hidden);
    assert_eq!(
        out.autofit,
        [
            Autofit { sheet: 0, columns: true, first: 0, last: 4 },
            Autofit { sheet: 0, columns: false, first: 0, last: 3 },
        ]
    );
}

fn filter() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/filter.ts"));
    assert_eq!((wb.sheets[0].frozen_rows, wb.sheets[0].frozen_cols), (1, 1));
    assert!(wb.sheets[0].auto_filter.is_some() && wb.sheets[0].filtered_rows.is_empty());
}

fn tables() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/tables.ts"));
    assert_eq!(wb.sheets[0].tables.len(), 1);
    assert_eq!(wb.sheets[0].tables[0].columns.len(), 2);
}

#[test]
fn worksheets_of_a_workbook() {
    worksheets();
}

#[test]
fn names_and_calculation() {
    names();
}

#[test]
fn values_formulas_and_number_formats() {
    values();
}

#[test]
fn from_a_range_to_another() {
    navigation();
}

#[test]
fn cells_that_move_copy_and_clear() {
    editing();
}

#[test]
fn sort_search_fill_link_and_hide() {
    data();
}

#[test]
fn fonts_fills_borders_alignment_and_sizes() {
    format();
}

#[test]
fn frozen_panes_and_the_filter_of_a_sheet() {
    filter();
}

#[test]
fn tables_their_columns_and_their_filters() {
    tables();
}

#[test]
fn english_and_the_language_of_the_workbook() {
    let mut wb = book_in(Locale::fr(), &[("A1", "1,5"), ("A2", "2,5")]);
    ok(&mut wb, include_str!("model/locale.ts"));
    assert_eq!(wb.locale.id, "fr");
    assert_eq!(text(&wb, "B2"), "2");
}

/// Every member the table lists has answered a call of the scripts above:
/// the list a user is shown and what runs cannot differ.
#[test]
fn every_listed_member_runs() {
    for scenario in [worksheets, names, values, navigation, editing, data, format, filter, tables] {
        scenario();
    }
    let called = members_called();
    let mut missing = Vec::new();
    let mut count = 0;
    for class in api() {
        assert!(!class.description.is_empty(), "{} has no description", class.name);
        for member in class.members {
            count += 1;
            assert!(member.signature.starts_with(&format!("{}(", member.name)), "{}", member.signature);
            assert!(!member.description.is_empty(), "{}.{} has no description", class.name, member.name);
            if !called.contains(&(class.name, member.name)) {
                missing.push(format!("{}.{}", class.name, member.name));
            }
        }
    }
    assert!(missing.is_empty(), "no test calls {missing:?}");
    assert!(count > 250, "{count} members");
    // The first class is the one `main` receives; names are not repeated.
    assert_eq!(api()[0].name, "Workbook");
    let mut names: Vec<&str> = api().iter().map(|c| c.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), api().len());
    for class in api() {
        let mut members: Vec<&str> = class.members.iter().map(|m| m.name).collect();
        members.sort_unstable();
        members.dedup();
        assert_eq!(members.len(), class.members.len(), "{}", class.name);
    }
}

/// The enums are those a script reads, with Excel's values.
#[test]
fn enums_are_what_scripts_read() {
    let mut source = String::from("function main(workbook: ExcelScript.Workbook) {\n");
    for e in enums() {
        for (member, value) in e.members {
            source.push_str(&format!("  eq(ExcelScript.{}.{member}, {value:?});\n", e.name));
        }
    }
    source.push_str("}\n");
    let mut wb = stock();
    ok(&mut wb, &source);
    let of = |name: &str| enums().iter().find(|e| e.name == name).map(|e| e.members).expect(name);
    assert!(of("ClearApplyTo").contains(&("contents", "Contents")));
    assert!(of("BorderIndex").contains(&("edgeBottom", "EdgeBottom")));
    assert!(of("HorizontalAlignment").contains(&("centerAcrossSelection", "CenterAcrossSelection")));
}
