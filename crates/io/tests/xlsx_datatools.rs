//! What the data tools keep in `.xlsx` files: the Advanced Filter of a
//! sheet, scenarios, data tables and the text files a workbook imports.

mod common;

use common::{assert_worksheet_order, enter, package, part, part_names, value};
use opensheet_engine::consolidate::{ConsolidateFunction, ConsolidateSource, Consolidation};
use opensheet_engine::whatif::{DataTable, Scenario, ScenarioCell};
use opensheet_engine::{Locale, Rect, Value, Workbook};
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

fn scenario(name: &str, cells: &[(u32, u32, &str)]) -> Scenario {
    Scenario {
        name: name.into(),
        cells: cells
            .iter()
            .map(|(row, col, value)| ScenarioCell {
                row: *row,
                col: *col,
                value: value.to_string(),
            })
            .collect(),
        ..Scenario::default()
    }
}

#[test]
fn scenarios_round_trip() {
    let mut wb = list();
    let mut low = scenario("Low & slow", &[(0, 5, "50"), (1, 5, "5%"), (2, 5, "some \"text\"")]);
    low.comment = "Created by Ann on 10/12/2026".into();
    low.user = "Ann".into();
    low.locked = true;
    let mut high = scenario("High", &[(0, 5, "1.5")]);
    high.hidden = true;
    wb.scenario_add(0, low.clone()).unwrap();
    wb.scenario_add(0, high.clone()).unwrap();
    wb.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
    wb.protect_sheet(0, Default::default(), "");
    wb.commit("scenarios");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1", &sheet);
    assert!(
        sheet.contains(concat!(
            "<scenarios current=\"0\" show=\"0\">",
            "<scenario name=\"Low &amp; slow\" locked=\"1\" count=\"3\" user=\"Ann\" comment=\"Created by Ann on 10/12/2026\">",
            "<inputCells r=\"F1\" val=\"50\"/><inputCells r=\"F2\" val=\"0.05\"/><inputCells r=\"F3\" val=\"some &quot;text&quot;\"/>",
            "</scenario>",
            "<scenario name=\"High\" hidden=\"1\" count=\"1\"><inputCells r=\"F1\" val=\"1.5\"/></scenario>",
            "</scenarios>"
        )),
        "{sheet}"
    );
    let back = load(&bytes);
    // A percentage comes back as the number it is.
    low.cells[1].value = "0.05".into();
    assert_eq!(back.sheets[0].scenarios, [low.clone(), high.clone()]);

    // Read in French, numbers are typed with a comma; written again, with a point.
    let french = read_xlsx(Cursor::new(&bytes), Locale::fr()).unwrap();
    assert_eq!(french.sheets[0].scenarios[1].cells[0].value, "1,5");
    assert_eq!(french.sheets[0].scenarios[0].cells[1].value, "0,05");
    assert!(part(&to_bytes(&french), "xl/worksheets/sheet1.xml").contains("<inputCells r=\"F1\" val=\"1.5\"/>"));

    // As Excel writes them, with what we do not keep; a scenario without
    // cells or with a name already taken is left out.
    let excel = sheet.replace(
        "<scenarios current=\"0\" show=\"0\">",
        "<scenarios current=\"1\" show=\"1\" sqref=\"A3\"><scenario name=\"Empty\" count=\"0\"/><scenario name=\"high\" count=\"1\"><inputCells r=\"$B$2\" val=\"7\" numFmtId=\"9\"/></scenario>",
    );
    let back = load(&with_part(&bytes, "xl/worksheets/sheet1.xml", &excel));
    let names: Vec<&str> = back.sheets[0].scenarios.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["high", "Low & slow"]);
    assert_eq!(back.sheets[0].scenarios[0].cells, [ScenarioCell { row: 1, col: 1, value: "7".into() }]);
}

#[test]
fn data_tables_round_trip() {
    let mut wb = Workbook::new(Locale::en());
    let cells = [
        ("B1", "10"),
        ("B2", "3"),
        ("B3", "=B1*B2"),
        ("D1", "=B3"),
        ("E1", "10"),
        ("F1", "20"),
        ("D2", "1"),
        ("D3", "2"),
        ("E7", "=B3&\"!\""),
        ("D8", "5"),
        ("A11", "=B3+1"),
        ("B10", "7"),
    ];
    for (cell, text) in cells {
        enter(&mut wb, 0, cell, text);
    }
    wb.commit("setup");
    wb.data_table_create(0, rect("D1:F3"), Some((0, 1)), Some((1, 1))).unwrap();
    wb.data_table_create(0, rect("D7:E8"), None, Some((1, 1))).unwrap();
    wb.data_table_create(0, rect("A10:B11"), Some((0, 1)), None).unwrap();
    wb.commit("Data Table");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    for expected in [
        "<c r=\"E2\"><f t=\"dataTable\" ref=\"E2:F3\" dt2D=\"1\" dtr=\"1\" r1=\"B1\" r2=\"B2\"/><v>10</v></c><c r=\"F2\"><v>20</v></c>",
        "<c r=\"E8\" t=\"str\"><f t=\"dataTable\" ref=\"E8\" r1=\"B2\"/><v>50!</v></c>",
        "<c r=\"B11\"><f t=\"dataTable\" ref=\"B11\" dtr=\"1\" r1=\"B1\"/><v>22</v></c>",
    ] {
        assert!(sheet.contains(expected), "{expected}\n{sheet}");
    }

    let mut back = load(&bytes);
    assert_eq!(
        back.sheets[0].data_tables,
        [
            DataTable {
                range: rect("E2:F3"),
                row_input: Some((0, 1)),
                col_input: Some((1, 1)),
            },
            DataTable {
                range: rect("E8"),
                row_input: None,
                col_input: Some((1, 1)),
            },
            DataTable {
                range: rect("B11"),
                row_input: Some((0, 1)),
                col_input: None,
            },
        ]
    );
    assert_eq!(value(&back, 0, "F3"), Value::Number(40.0));
    // Read back, the tables are live.
    enter(&mut back, 0, "F1", "30");
    back.commit("edit");
    assert_eq!(value(&back, 0, "F3"), Value::Number(60.0));
    assert_eq!(value(&back, 0, "B1"), Value::Number(10.0));

    // A table Excel marks as having lost an input cell stays constants.
    let broken = sheet.replace("ref=\"E8\" r1=\"B2\"", "ref=\"E8\" del1=\"1\" r1=\"B2\"");
    let back = load(&with_part(&bytes, "xl/worksheets/sheet1.xml", &broken));
    assert_eq!(back.sheets[0].data_tables.len(), 2);
    assert_eq!(value(&back, 0, "E8"), Value::text("50!"));
}
