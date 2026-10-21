//! Outlines in `.xlsx` files: the levels of rows and columns, what is
//! folded, where summaries lie and whether the symbols show.

mod common;

use common::{assert_well_formed, assert_worksheet_order, enter, keep_sample, package, part};
use opensheet_engine::outline::{Axis, Group};
use opensheet_engine::{Locale, Rect, Workbook};
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

/// Two groups of rows under their totals and a grand total, and two
/// grouped columns.
fn outlined() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let cells = [
        ("A1", "Region"),
        ("B1", "Q1"),
        ("C1", "Q2"),
        ("D1", "Year"),
        ("A2", "North"),
        ("B2", "10"),
        ("C2", "12"),
        ("D2", "=SUM(B2:C2)"),
        ("A3", "North"),
        ("B3", "20"),
        ("C3", "8"),
        ("D3", "=SUM(B3:C3)"),
        ("A4", "North Total"),
        ("B4", "=SUBTOTAL(9,B2:B3)"),
        ("A5", "South"),
        ("B5", "5"),
        ("C5", "7"),
        ("D5", "=SUM(B5:C5)"),
        ("A6", "South Total"),
        ("B6", "=SUBTOTAL(9,B5:B5)"),
        ("A7", "Grand Total"),
        ("B7", "=SUBTOTAL(9,B2:B5)"),
    ];
    for (cell, text) in cells {
        enter(&mut wb, 0, cell, text);
    }
    wb.outline_group(0, Axis::Rows, 1, 5).unwrap();
    wb.outline_group(0, Axis::Rows, 1, 2).unwrap();
    wb.outline_group(0, Axis::Rows, 4, 4).unwrap();
    wb.outline_group(0, Axis::Cols, 1, 2).unwrap();
    wb.commit("setup");
    wb
}

#[test]
fn levels_and_folded_groups_round_trip() {
    let mut wb = outlined();
    wb.outline_set_collapsed(
        0,
        Axis::Rows,
        &Group {
            level: 2,
            first: 1,
            last: 2,
        },
        true,
    );
    wb.outline_detail(0, Rect::cell(20, 3), false).unwrap();
    wb.commit("fold");
    let bytes = to_bytes(&wb);
    keep_sample("outline.xlsx", &bytes);
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &xml);
    assert_worksheet_order("sheet1.xml", &xml);
    assert!(
        xml.contains("outlineLevelRow=\"2\" outlineLevelCol=\"1\""),
        "{xml}"
    );
    assert!(
        xml.contains("<row r=\"2\" hidden=\"1\" outlineLevel=\"2\">"),
        "{xml}"
    );
    assert!(
        xml.contains("<row r=\"4\" outlineLevel=\"1\" collapsed=\"1\">"),
        "{xml}"
    );
    assert!(xml.contains("<row r=\"5\" outlineLevel=\"2\">"), "{xml}");
    assert!(xml.contains(" hidden=\"1\" outlineLevel=\"1\"/>"), "{xml}");
    assert!(xml.contains("<col min=\"4\" max=\"4\""), "{xml}");
    // Excel's defaults are not spelled out.
    assert!(
        !xml.contains("outlinePr") && !xml.contains("showOutlineSymbols"),
        "{xml}"
    );

    let back = load(&bytes);
    assert_eq!(back.sheets[0].outline, wb.sheets[0].outline);
    assert_eq!(back.sheets[0].hidden_rows, wb.sheets[0].hidden_rows);
    assert_eq!(back.sheets[0].hidden_cols, wb.sheets[0].hidden_cols);
    // The folded group opens again as it did before saving.
    let mut back = back;
    back.outline_detail(0, Rect::cell(3, 0), true).unwrap();
    assert!(back.sheets[0].hidden_rows.is_empty());
}

#[test]
fn settings_round_trip() {
    let mut wb = outlined();
    wb.set_outline_settings(0, false, false, false);
    wb.set_tab_color(0, Some(opensheet_engine::Color::rgb(255, 0, 0)));
    wb.commit("settings");
    let bytes = to_bytes(&wb);
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1.xml", &xml);
    assert!(
        xml.contains("<outlinePr summaryBelow=\"0\" summaryRight=\"0\"/></sheetPr>"),
        "{xml}"
    );
    assert!(xml.contains("showOutlineSymbols=\"0\""), "{xml}");
    let back = load(&bytes);
    let outline = &back.sheets[0].outline;
    assert_eq!(
        (
            outline.summary_below,
            outline.summary_right,
            outline.show_symbols
        ),
        (false, false, false)
    );
    assert_eq!(outline.rows.levels, wb.sheets[0].outline.rows.levels);
}

#[test]
fn an_outline_written_by_excel_is_read() {
    let sheet = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
        "<sheetPr><outlinePr summaryBelow=\"0\"/></sheetPr>",
        "<sheetViews><sheetView showOutlineSymbols=\"0\" workbookViewId=\"0\"/></sheetViews>",
        "<sheetFormatPr defaultRowHeight=\"15\" outlineLevelRow=\"2\" outlineLevelCol=\"1\"/>",
        "<cols><col min=\"2\" max=\"3\" width=\"9\" hidden=\"1\" outlineLevel=\"1\"/>",
        "<col min=\"4\" max=\"4\" width=\"9\" collapsed=\"1\"/></cols>",
        "<sheetData><row r=\"1\" collapsed=\"1\"><c r=\"A1\"><v>1</v></c></row>",
        "<row r=\"2\" hidden=\"1\" outlineLevel=\"1\"><c r=\"A2\"><v>2</v></c></row>",
        "<row r=\"3\" hidden=\"1\" outlineLevel=\"2\"/><row r=\"4\" outlineLevel=\"9\"/></sheetData>",
        "</worksheet>"
    );
    let bytes = package(&[
        (
            "[Content_Types].xml",
            concat!(
                "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
                "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
                "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
                "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
                "<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
                "</Types>"
            ),
        ),
        (
            "_rels/.rels",
            concat!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>",
                "</Relationships>"
            ),
        ),
        (
            "xl/workbook.xml",
            concat!(
                "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
                "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
                "<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            concat!(
                "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>",
                "</Relationships>"
            ),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ]);
    let wb = load(&bytes);
    let sh = &wb.sheets[0];
    assert!(!sh.outline.summary_below && sh.outline.summary_right && !sh.outline.show_symbols);
    assert_eq!(
        sh.outline
            .rows
            .levels
            .iter()
            .map(|(r, l)| (*r, *l))
            .collect::<Vec<_>>(),
        [(1, 1), (2, 2), (3, 7)]
    );
    assert!(sh.outline.rows.collapsed.contains(&0));
    assert_eq!(
        sh.outline.cols.levels.keys().copied().collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(sh.outline.cols.collapsed.contains(&3));
    assert_eq!(sh.hidden_rows.iter().copied().collect::<Vec<_>>(), [1, 2]);
    // The group folded under row 1 opens from it.
    let group = sh.outline.rows.group_at(1, 1).unwrap();
    assert_eq!((group.first, group.last), (1, 3));
    assert_eq!(sh.outline_summary(Axis::Rows, &group), Some(0));
}
