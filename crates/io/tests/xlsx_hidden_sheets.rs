//! Hidden sheets in `.xlsx` files: our own round trip, a workbook as
//! Excel writes it, and files where nothing visible is left.

mod common;

use common::{assert_well_formed, enter, package, part, value};
use opensheet_engine::workbook::SheetVisibility::{Hidden, VeryHidden, Visible};
use opensheet_engine::{Locale, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Four sheets: a hidden one first, the report, a very hidden one and a
/// second visible one. The report reads both hidden sheets.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for index in 1..4 {
        wb.add_sheet(index);
    }
    for (index, name) in ["Rates", "Report", "Secrets", "Notes"].iter().enumerate() {
        wb.rename_sheet(index, name);
    }
    enter(&mut wb, 0, "A1", "0.2");
    enter(&mut wb, 2, "A1", "1000");
    enter(
        &mut wb,
        1,
        "A1",
        "Tax (reads the hidden sheets Rates and Secrets)",
    );
    enter(&mut wb, 1, "A2", "=Rates!A1*Secrets!A1");
    enter(
        &mut wb,
        3,
        "A1",
        "Rates is hidden (Unhide… lists it); Secrets is very hidden (it does not)",
    );
    assert!(wb.set_sheet_visibility(0, Hidden));
    assert!(wb.set_sheet_visibility(2, VeryHidden));
    wb.commit("hidden sheets");
    wb
}

#[test]
fn hidden_sheets_survive_a_round_trip() {
    let wb = sample();
    let bytes = to_bytes(&wb);
    common::keep_sample("hidden-sheets.xlsx", &bytes);
    let book = part(&bytes, "xl/workbook.xml");
    assert_well_formed("workbook.xml", &book);
    // As Excel writes them: the state between the id and the relationship,
    // the front sheet and the first tab named when sheets before are hidden.
    assert!(
        book.contains("<sheet name=\"Rates\" sheetId=\"1\" state=\"hidden\" r:id=\"rId1\"/>"),
        "{book}"
    );
    assert!(
        book.contains("<sheet name=\"Report\" sheetId=\"2\" r:id=\"rId2\"/>"),
        "{book}"
    );
    assert!(
        book.contains("<sheet name=\"Secrets\" sheetId=\"3\" state=\"veryHidden\" r:id=\"rId3\"/>"),
        "{book}"
    );
    assert!(
        book.contains("windowHeight=\"17600\" firstSheet=\"1\" activeTab=\"1\"/>"),
        "{book}"
    );
    // One sheet is selected, and it is the one in front.
    let selected = |n: usize| {
        part(&bytes, &format!("xl/worksheets/sheet{n}.xml")).contains("tabSelected=\"1\"")
    };
    assert_eq!(
        [selected(1), selected(2), selected(3), selected(4)],
        [false, true, false, false]
    );

    let back = load(&bytes);
    let states: Vec<_> = back.sheets.iter().map(|s| s.visibility).collect();
    assert_eq!(states, [Hidden, Visible, VeryHidden, Visible]);
    assert_eq!(value(&back, 1, "A2"), Value::Number(200.0));
    // And once more, unchanged.
    assert_eq!(part(&to_bytes(&back), "xl/workbook.xml"), book);
}

#[test]
fn a_workbook_without_hidden_sheets_names_no_tab() {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.commit("two");
    let book = part(&to_bytes(&wb), "xl/workbook.xml");
    assert!(
        !book.contains("state=") && !book.contains("activeTab") && !book.contains("firstSheet"),
        "{book}"
    );
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
const BOOK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#;
const SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#;

fn foreign(sheets: &str) -> Vec<u8> {
    let book = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView xWindow="0" yWindow="500" windowWidth="28800" windowHeight="16420" firstSheet="1" activeTab="1"/></bookViews><sheets>{sheets}</sheets></workbook>"#
    );
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("xl/workbook.xml", &book),
        ("xl/_rels/workbook.xml.rels", BOOK_RELS),
        ("xl/worksheets/sheet1.xml", SHEET),
        ("xl/worksheets/sheet2.xml", SHEET),
    ])
}

#[test]
fn states_written_by_excel_are_read() {
    let wb = load(&foreign(
        r#"<sheet name="Data" sheetId="1" state="veryHidden" r:id="rId1"/><sheet name="Front" sheetId="2" state="visible" r:id="rId2"/>"#,
    ));
    assert_eq!(
        (wb.sheets[0].visibility, wb.sheets[1].visibility),
        (VeryHidden, Visible)
    );
    assert_eq!(wb.nearest_visible_sheet(0), 1);
    // An unknown state is a visible sheet.
    let wb = load(&foreign(
        r#"<sheet name="A" sheetId="1" state="shy" r:id="rId1"/><sheet name="B" sheetId="2" r:id="rId2"/>"#,
    ));
    assert!(wb.sheets.iter().all(|s| s.is_visible()));
}

#[test]
fn a_file_with_nothing_visible_shows_its_first_sheet() {
    let wb = load(&foreign(
        r#"<sheet name="A" sheetId="1" state="hidden" r:id="rId1"/><sheet name="B" sheetId="2" state="veryHidden" r:id="rId2"/>"#,
    ));
    assert_eq!(
        (wb.sheets[0].visibility, wb.sheets[1].visibility),
        (Visible, VeryHidden)
    );
}
