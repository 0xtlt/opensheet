//! What the data tools keep in `.xlsx` files: the Advanced Filter of a
//! sheet, scenarios, data tables and the text files a workbook imports.

mod common;

use common::{assert_worksheet_order, enter, package, part, part_names};
use opensheet_engine::consolidate::{ConsolidateFunction, ConsolidateSource, Consolidation};
use opensheet_engine::{Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

/// The package with one part replaced.
fn with_part(bytes: &[u8], name: &str, text: &str) -> Vec<u8> {
    let names = part_names(bytes);
    let texts: Vec<String> = names.iter().map(|n| if n == name { text.to_string() } else { part(bytes, n) }).collect();
    let parts: Vec<(&str, &str)> = names.iter().map(String::as_str).zip(texts.iter().map(String::as_str)).collect();
    package(&parts)
}

/// A list in A1:B5 and a criteria range in D1:D2.
fn list() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let cells = [
        ("A1", "Name"),
        ("B1", "City"),
        ("A2", "Ann"),
        ("B2", "Paris"),
        ("A3", "Bob"),
        ("B3", "Lyon"),
        ("A4", "Cid"),
        ("B4", "Paris"),
        ("A5", "Dee"),
        ("B5", "Nice"),
        ("D1", "City"),
        ("D2", "Paris"),
    ];
    for (cell, text) in cells {
        enter(&mut wb, 0, cell, text);
    }
    wb.commit("setup");
    wb
}

#[test]
fn advanced_filter_in_place_round_trips() {
    let mut wb = list();
    wb.advanced_filter_in_place(0, rect("A1:B5"), Some((0, rect("D1:D2"))), false).unwrap();
    // A row hidden by hand under the list stays one.
    wb.snapshot();
    wb.sheets[0].hidden_rows.insert(7);
    wb.commit("filter");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<sheetPr filterMode=\"1\"></sheetPr>"), "{sheet}");
    assert!(sheet.contains("<row r=\"3\" hidden=\"1\""), "{sheet}");
    assert!(!sheet.contains("<autoFilter"));
    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(
        workbook.contains("<definedName name=\"_xlnm.Criteria\" localSheetId=\"0\">Sheet1!$D$1:$D$2</definedName>"),
        "{workbook}"
    );
    assert!(
        workbook.contains("<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">Sheet1!$A$1:$B$5</definedName>"),
        "{workbook}"
    );

    let mut back = load(&bytes);
    let filter = back.sheets[0].advanced_filter.clone().expect("the filter is read back");
    assert_eq!(filter.range, rect("A1:B5"));
    assert_eq!(filter.criteria, Some(rect("D1:D2")));
    assert_eq!(filter.hidden.iter().copied().collect::<Vec<_>>(), [2, 4]);
    assert_eq!(back.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2, 4]);
    assert_eq!(back.sheets[0].hidden_rows.iter().copied().collect::<Vec<_>>(), [7]);
    // Clear shows the rows of the list again, not the one hidden by hand.
    assert!(back.clear_advanced_filter(0));
    back.commit("Clear");
    assert!(back.sheets[0].filtered_rows.is_empty());
    assert!(back.sheets[0].is_row_hidden(7));
}

#[test]
fn filter_mode_of_an_autofilter_is_not_an_advanced_filter() {
    // Excel also writes filterMode for the criteria of an AutoFilter.
    let mut wb = list();
    wb.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
    wb.commit("filter");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml").replace("<dimension", "<sheetPr filterMode=\"1\"/><dimension");
    let bytes = with_part(&bytes, "xl/worksheets/sheet1.xml", &sheet);
    let back = load(&bytes);
    assert!(back.sheets[0].auto_filter.is_some());
    assert!(back.sheets[0].advanced_filter.is_none());
}

#[test]
fn consolidation_settings_round_trip() {
    let mut wb = list();
    wb.add_sheet(1);
    assert!(wb.rename_sheet(1, "North & South"));
    for (cell, text) in [("A1", "Q1"), ("A2", "4"), ("B1", "Q2"), ("B2", "5")] {
        enter(&mut wb, 1, cell, text);
    }
    let spec = Consolidation {
        function: ConsolidateFunction::Average,
        sources: vec![
            ConsolidateSource {
                sheet: "North & South".into(),
                range: rect("A1:B2"),
            },
            ConsolidateSource {
                sheet: "Sheet1".into(),
                range: rect("F1:G2"),
            },
        ],
        top_row: true,
        left_column: false,
    };
    wb.consolidate(0, rect("J1"), &spec).unwrap();
    // With the filter, to check where the element goes.
    wb.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
    wb.commit("Consolidate");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1", &sheet);
    assert!(
        sheet.contains(concat!(
            "<dataConsolidate function=\"average\" topLabels=\"1\"><dataRefs count=\"2\">",
            "<dataRef ref=\"A1:B2\" sheet=\"North &amp; South\"/><dataRef ref=\"F1:G2\" sheet=\"Sheet1\"/>",
            "</dataRefs></dataConsolidate>"
        )),
        "{sheet}"
    );
    let back = load(&bytes);
    assert_eq!(back.sheets[0].consolidation.as_ref(), Some(&spec));
    assert_eq!(back.sheets[1].consolidation, None);

    // As Excel writes it, with a link to another workbook among the references.
    let excel = sheet.replace(
        "<dataConsolidate function=\"average\" topLabels=\"1\">",
        "<dataConsolidate function=\"stdDevp\" leftLabels=\"1\" link=\"1\">",
    );
    let excel = excel.replace("<dataRef ref=\"F1:G2\" sheet=\"Sheet1\"/>", "<dataRef ref=\"A1:B9\" sheet=\"Other\" r:id=\"rId9\"/>");
    let back = load(&with_part(&bytes, "xl/worksheets/sheet1.xml", &excel));
    let read = back.sheets[0].consolidation.as_ref().expect("the settings");
    assert_eq!(read.function, ConsolidateFunction::StdDevP);
    assert!(read.left_column && !read.top_row);
    assert_eq!(read.sources, spec.sources[..1]);
}
