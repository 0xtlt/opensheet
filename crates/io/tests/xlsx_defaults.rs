//! The defaults a workbook brings with it: its default font, and the
//! default column width and row height of its sheets. Column widths in a
//! file count in digits of that font, so the same number is not the same
//! width in two workbooks.

mod common;

use common::{assert_well_formed, assert_worksheet_order, enter, package, part, value};
use opensheet_engine::workbook::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use opensheet_engine::{Locale, StyleId, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// A styles part whose default font is `font`, the way Excel lays it out:
/// font 0 is the font of the Normal style and of the first cell format.
fn styles(font: &str, size: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet {MAIN}><fonts count="2"><font><sz val="{size}"/><color theme="1"/><name val="{font}"/><family val="2"/><scheme val="minor"/></font><font><b/><sz val="{size}"/><color theme="1"/><name val="{font}"/><family val="2"/><scheme val="minor"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
    )
}

/// A one-sheet workbook with the given styles part and the given
/// `sheetFormatPr`, `cols` and rows.
fn file(styles: &str, format: &str, cols: &str, rows: &str) -> Vec<u8> {
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN} xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac"><dimension ref="A1:C3"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews>{format}{cols}<sheetData>{rows}</sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    );
    package(&[
        (
            "xl/workbook.xml",
            &format!(
                r#"<workbook {MAIN}><sheets><sheet name="Feuil1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

const ROWS: &str = r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" s="1"><v>2</v></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>text</t></is></c></row>"#;

fn default_font(wb: &Workbook) -> (String, f32) {
    let font = &wb.styles.default_style().font;
    (font.name.to_string(), font.size_pt())
}

#[test]
fn a_workbook_in_calibri() {
    // What Excel for Windows wrote for fifteen years.
    let bytes = file(
        &styles("Calibri", "11"),
        r#"<sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/>"#,
        r#"<cols><col min="1" max="1" width="12.7109375" customWidth="1"/><col min="2" max="2" width="9.140625" style="1"/><col min="4" max="4" width="9.140625" customWidth="1"/></cols>"#,
        ROWS,
    );
    let mut wb = load(&bytes);
    assert_eq!(default_font(&wb), ("Calibri".into(), 11.0));
    let sheet = &wb.sheets[0];
    assert_eq!(
        (sheet.default_col_width, sheet.default_row_height),
        (64.0, 20.0)
    );
    assert_eq!(sheet.col_width(0), 89.0);
    assert_eq!(
        sheet.col_widths.keys().copied().collect::<Vec<_>>(),
        [0, 3],
        "a column of the default width has none, unless it was set"
    );
    assert_eq!(sheet.col_width(1), 64.0);
    assert!(wb.style_at(0, 5, 1).font.bold, "the style of column B");

    // Cells nobody formatted have the default style, and so has whatever
    // is typed next.
    assert_eq!(wb.cell(0, 0, 0).unwrap().style, StyleId(0));
    assert_eq!(wb.cell(0, 1, 0).unwrap().style, StyleId(0));
    assert_ne!(wb.cell(0, 0, 1).unwrap().style, StyleId(0));
    enter(&mut wb, 0, "C7", "typed");
    assert_eq!(wb.style_at(0, 6, 2).font.name.as_ref(), "Calibri");
    // A new sheet is like the others.
    let added = wb.add_sheet(1);
    wb.commit("sheet");
    assert_eq!(wb.sheets[added].default_col_width, 64.0);
    assert_eq!(wb.style_at(added, 0, 0).font.name.as_ref(), "Calibri");
    // The default style is style 0 for whoever asks for it again; the
    // engine's own default is now a style like any other.
    let calibri = wb.styles.default_style().clone();
    assert_eq!(wb.styles.intern(calibri), StyleId(0));
    assert_ne!(wb.styles.intern(Default::default()), StyleId(0));

    // Saved: the default font is font 0, the sizes are those of the font
    // and need not be said.
    let saved = to_bytes(&wb);
    common::keep_sample("defaults-calibri-11.xlsx", &saved);
    let styles = part(&saved, "xl/styles.xml");
    assert!(
        styles.contains("<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/><scheme val=\"minor\"/></font>"),
        "{styles}"
    );
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1.xml", &sheet);
    assert!(
        sheet.contains("<sheetFormatPr defaultRowHeight=\"15\"/>"),
        "{sheet}"
    );
    assert!(sheet.contains(concat!(
        "<cols><col min=\"1\" max=\"1\" width=\"12.71428571\" customWidth=\"1\"/>",
        "<col min=\"2\" max=\"2\" width=\"9.14285714\" style=\"1\"/>",
        "<col min=\"4\" max=\"4\" width=\"9.14285714\" customWidth=\"1\"/></cols>"
    )));
    let again = load(&saved);
    assert_eq!(default_font(&again), ("Calibri".into(), 11.0));
    assert_eq!(again.sheets[0].col_widths, wb.sheets[0].col_widths);
    assert_eq!(again.sheets[0].col_styles.len(), 1);
}

#[test]
fn heights_from_a_scaled_screen() {
    // The same workbook saved on a screen scaled to 125 %: the heights
    // Excel worked out by itself (the default, and row 2) are whole pixels
    // of that screen. Row 3 was set by hand.
    let bytes = file(
        &styles("Calibri", "11"),
        r#"<sheetFormatPr defaultRowHeight="14.4" x14ac:dyDescent="0.3"/>"#,
        "",
        r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="2" ht="14.4" x14ac:dyDescent="0.3"><c r="A2"><v>2</v></c></row><row r="3" ht="28.8" customHeight="1"><c r="A3"><v>3</v></c></row><row r="4" ht="18"><c r="A4"><v>4</v></c></row>"#,
    );
    let wb = load(&bytes);
    let sheet = &wb.sheets[0];
    assert_eq!(sheet.default_row_height, 20.0, "Calibri 11 at 100 %");
    assert_eq!(
        sheet
            .row_heights
            .iter()
            .map(|(r, h)| (*r, *h))
            .collect::<Vec<_>>(),
        [(2, 38.4), (3, 24.0)]
    );
    assert_eq!(sheet.row_height(1), 20.0);
    let saved = part(&to_bytes(&wb), "xl/worksheets/sheet1.xml");
    assert!(
        saved.contains("<sheetFormatPr defaultRowHeight=\"15\"/>"),
        "{saved}"
    );
}

#[test]
fn a_workbook_from_excel_for_mac() {
    // Aptos Narrow 12, ten characters to a column, rows of 16 points.
    let bytes = file(
        &styles("Aptos Narrow", "12"),
        r#"<sheetFormatPr baseColWidth="10" defaultRowHeight="16"/>"#,
        r#"<cols><col min="1" max="1" width="10.83203125" customWidth="1"/><col min="2" max="3" width="20" customWidth="1"/></cols>"#,
        ROWS,
    );
    let wb = load(&bytes);
    assert_eq!(default_font(&wb), ("Aptos Narrow".into(), 12.0));
    let sheet = &wb.sheets[0];
    // Digits of 8 pixels: 10 × 8 + 5, rounded up to a multiple of 8.
    assert_eq!(
        (sheet.default_col_width, sheet.default_row_height),
        (88.0, 21.0)
    );
    assert_eq!(sheet.col_width(0), 86.66);
    assert_eq!(sheet.col_width(1), 160.0);
    assert_eq!(sheet.col_width(3), 88.0);

    let saved = to_bytes(&wb);
    common::keep_sample("defaults-aptos-12.xlsx", &saved);
    let xml = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<sheetFormatPr defaultColWidth=\"11\" defaultRowHeight=\"15.75\"/>"),
        "{xml}"
    );
    assert!(xml.contains(concat!(
        "<cols><col min=\"1\" max=\"1\" width=\"10.8325\" customWidth=\"1\"/>",
        "<col min=\"2\" max=\"3\" width=\"20\" customWidth=\"1\"/></cols>"
    )));
    assert!(
        part(&saved, "xl/styles.xml")
            .contains("<font><sz val=\"12\"/><name val=\"Aptos Narrow\"/></font>")
    );
    let mut again = load(&saved);
    assert_eq!(default_font(&again), ("Aptos Narrow".into(), 12.0));
    assert_eq!(
        (
            again.sheets[0].default_col_width,
            again.sheets[0].default_row_height
        ),
        (88.0, 21.0)
    );
    assert_eq!(again.sheets[0].col_widths, sheet.col_widths);
    // A sheet added to this workbook has its sizes, not the engine's.
    let added = again.add_sheet(1);
    assert_eq!(
        (
            again.sheets[added].default_col_width,
            again.sheets[added].default_row_height
        ),
        (88.0, 21.0)
    );
    assert_eq!(again.sheets[added].col_width(3), 88.0);
    assert_eq!(again.sheets[added].row_height(3), 21.0);
}

#[test]
fn sizes_set_for_a_sheet() {
    // A default width and a default height chosen by the user, in a
    // workbook in Arial 10 (Excel before 2007; Google Sheets exports look
    // like this too).
    let bytes = file(
        &styles("Arial", "10"),
        r#"<sheetFormatPr customHeight="1" defaultColWidth="12.63" defaultRowHeight="15.75"/>"#,
        r#"<cols><col min="1" max="2" width="12.63"/><col min="3" max="3" width="30.5" customWidth="1"/><col min="5" max="5" width="9.140625"/></cols>"#,
        r#"<row r="1" ht="15.75"><c r="A1"><v>1</v></c></row><row r="2" ht="12.75"><c r="A2"><v>2</v></c></row>"#,
    );
    let wb = load(&bytes);
    assert_eq!(default_font(&wb), ("Arial".into(), 10.0));
    let sheet = &wb.sheets[0];
    assert_eq!(
        (sheet.default_col_width, sheet.default_row_height),
        (88.41, 21.0)
    );
    assert_eq!(
        sheet
            .col_widths
            .iter()
            .map(|(c, w)| (*c, *w))
            .collect::<Vec<_>>(),
        [(2, 213.5), (4, 64.0)]
    );
    assert_eq!(
        sheet
            .row_heights
            .iter()
            .map(|(r, h)| (*r, *h))
            .collect::<Vec<_>>(),
        [(1, 17.0)],
        "the natural height of the font is not this sheet's default"
    );

    let saved = to_bytes(&wb);
    common::keep_sample("defaults-arial-10.xlsx", &saved);
    let xml = part(&saved, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &xml);
    assert!(
        xml.contains("<sheetFormatPr defaultColWidth=\"12.63\" defaultRowHeight=\"15.75\" customHeight=\"1\"/>"),
        "{xml}"
    );
    let again = load(&saved);
    assert_eq!(
        (
            again.sheets[0].default_col_width,
            again.sheets[0].default_row_height
        ),
        (88.41, 21.0)
    );
    assert_eq!(again.sheets[0].col_widths, sheet.col_widths);
    assert_eq!(again.sheets[0].row_heights, sheet.row_heights);

    // Widths Excel wrote for a workbook in Arial 10 (its Solver add-in):
    // whole pixels once multiplied by the 7 pixels of an Arial digit.
    let solver = load(&file(
        &styles("Arial", "10"),
        r#"<sheetFormatPr defaultColWidth="8.85546875" defaultRowHeight="12.75"/>"#,
        r#"<cols><col min="1" max="1" width="76.42578125" style="1" customWidth="1"/><col min="2" max="16384" width="8.85546875" style="1"/></cols>"#,
        ROWS,
    ));
    assert_eq!(solver.sheets[0].col_width(0), 535.0);
    assert_eq!(solver.sheets[0].col_widths.len(), 1);
    assert_eq!(solver.sheets[0].col_styles.len(), 16384);

    // Without sizes of its own a sheet in Arial 10 has columns of 64
    // pixels and rows of 17 (12.75 points), which Excel does not write.
    let plain = load(&file(
        &styles("Arial", "10"),
        r#"<sheetFormatPr defaultRowHeight="12.75"/>"#,
        "",
        ROWS,
    ));
    assert_eq!(
        (
            plain.sheets[0].default_col_width,
            plain.sheets[0].default_row_height
        ),
        (64.0, 17.0)
    );
    let xml = part(&to_bytes(&plain), "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<sheetFormatPr defaultRowHeight=\"12.75\"/>"),
        "{xml}"
    );
}

#[test]
fn other_default_fonts() {
    // (font, size, sheetFormatPr, default column width, default row height)
    let cases = [
        // Japanese and Chinese versions of Excel: the "8.38" column.
        (
            "游ゴシック",
            "11",
            r#"<sheetFormatPr defaultRowHeight="18.75"/>"#,
            72.0,
            25.0,
        ),
        (
            "等线",
            "11",
            r#"<sheetFormatPr defaultRowHeight="14.25"/>"#,
            72.0,
            19.0,
        ),
        (
            "ＭＳ Ｐゴシック",
            "11",
            r#"<sheetFormatPr defaultRowHeight="13.5"/>"#,
            72.0,
            18.0,
        ),
        // LibreOffice.
        (
            "Liberation Sans",
            "10",
            r#"<sheetFormatPr defaultColWidth="11.53515625" defaultRowHeight="12.8" zeroHeight="false" outlineLevelRow="0" outlineLevelCol="0"/>"#,
            80.75,
            17.0,
        ),
        (
            "Verdana",
            "10",
            r#"<sheetFormatPr baseColWidth="10" defaultRowHeight="13"/>"#,
            88.0,
            17.0,
        ),
        (
            "Calibri",
            "12",
            r#"<sheetFormatPr baseColWidth="10" defaultRowHeight="15.75"/>"#,
            88.0,
            21.0,
        ),
        (
            "Times New Roman",
            "12",
            r#"<sheetFormatPr defaultRowHeight="15.75"/>"#,
            72.0,
            21.0,
        ),
        // The sheets of the Solver add-in that comes with Excel.
        (
            "Arial",
            "10",
            r#"<sheetFormatPr defaultColWidth="8.85546875" defaultRowHeight="12.75" x14ac:dyDescent="0.2"/>"#,
            62.0,
            17.0,
        ),
        // No `sheetFormatPr` at all: what the font gives.
        ("Calibri", "11", "", 64.0, 20.0),
        ("Calibri", "14", "", 88.0, 25.0),
        ("Arial", "10", "", 64.0, 17.0),
        (
            "A Font From Nowhere",
            "11",
            r#"<sheetFormatPr defaultRowHeight="14"/>"#,
            72.0,
            19.0,
        ),
    ];
    for (font, size, format, col, row) in cases {
        let wb = load(&file(&styles(font, size), format, "", ROWS));
        assert_eq!(default_font(&wb).0, font);
        let sheet = &wb.sheets[0];
        assert_eq!(
            (sheet.default_col_width, sheet.default_row_height),
            (col, row),
            "{font} {size}"
        );
        // And back.
        let again = load(&to_bytes(&wb));
        assert_eq!(default_font(&again), default_font(&wb), "{font} {size}");
        assert_eq!(
            (
                again.sheets[0].default_col_width,
                again.sheets[0].default_row_height
            ),
            (col, row),
            "{font} {size} after a round trip"
        );
        assert_eq!(value(&again, 0, "A2"), Value::text("text"));
    }
}

#[test]
fn the_font_of_cells_is_not_always_the_font_of_widths() {
    // Written by a program: the first cell format (what unformatted cells
    // look like) is Arial 10, the Normal style is Calibri 11. Widths count
    // in digits of the Normal font.
    let styles = format!(
        r#"<styleSheet {MAIN}><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><sz val="10"/><name val="Arial"/></font></fonts>
<cellStyleXfs count="2"><xf numFmtId="0" fontId="1"/><xf numFmtId="0" fontId="0"/></cellStyleXfs>
<cellXfs count="2"><xf numFmtId="0" fontId="1" xfId="1"/><xf numFmtId="0" fontId="0" xfId="1"/></cellXfs>
<cellStyles count="2"><cellStyle name="Other" xfId="0"/><cellStyle name="Normal" xfId="1" builtinId="0"/></cellStyles></styleSheet>"#
    );
    let wb = load(&file(
        &styles,
        "",
        r#"<cols><col min="1" max="1" width="10" customWidth="1"/></cols>"#,
        ROWS,
    ));
    assert_eq!(default_font(&wb), ("Arial".into(), 10.0));
    assert_eq!(wb.style_at(0, 0, 1).font.name.as_ref(), "Calibri");
    assert_eq!(wb.sheets[0].col_width(0), 70.0);
    assert_eq!(wb.sheets[0].default_row_height, 20.0);
}

#[test]
fn our_own_workbooks_are_unchanged() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "1");
    wb.set_col_width(0, 1, Some(100.0));
    wb.commit("setup");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<sheetFormatPr defaultRowHeight=\"15\"/><cols><col min=\"2\" max=\"2\" width=\"14.28571429\" customWidth=\"1\"/></cols>"), "{sheet}");
    assert!(part(&bytes, "xl/styles.xml").contains(
        "<fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Aptos Narrow\"/><scheme val=\"minor\"/></font></fonts>"
    ));
    let loaded = load(&bytes);
    assert_eq!(loaded.styles.default_style(), wb.styles.default_style());
    assert_eq!(
        (
            loaded.sheets[0].default_col_width,
            loaded.sheets[0].default_row_height
        ),
        (DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT)
    );
    assert_eq!(loaded.sheets[0].col_width(1), 100.0);

    // Sizes of a sheet changed here are written, and come back.
    wb.sheets[0].default_col_width = 100.0;
    wb.sheets[0].default_row_height = 30.0;
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains("<sheetFormatPr defaultColWidth=\"14.28571429\" defaultRowHeight=\"22.5\" customHeight=\"1\"/>"),
        "{sheet}"
    );
    let loaded = load(&bytes);
    assert_eq!(
        (
            loaded.sheets[0].default_col_width,
            loaded.sheets[0].default_row_height
        ),
        (100.0, 30.0)
    );
    assert_eq!(loaded.sheets[0].col_widths, wb.sheets[0].col_widths);
}

#[test]
fn nonsense_sizes_are_ignored() {
    let formats = [
        r#"<sheetFormatPr defaultColWidth="-5" defaultRowHeight="-1" baseColWidth="abc"/>"#,
        r#"<sheetFormatPr defaultColWidth="0" defaultRowHeight="0" baseColWidth="-2"/>"#,
        r#"<sheetFormatPr defaultColWidth="NaN" defaultRowHeight="inf" baseColWidth="1e99"/>"#,
        r#"<sheetFormatPr defaultColWidth="1e99" defaultRowHeight="1e99" customHeight="1"/>"#,
        r#"<sheetFormatPr baseColWidth="255" defaultRowHeight="100000"/>"#,
        r#"<sheetFormatPr customHeight="1"/>"#,
        r#"<sheetFormatPr"#,
    ];
    for format in formats {
        for font_size in ["11", "0", "-3", "1e9", "x"] {
            let wb = load(&file(
                &styles("Calibri", font_size),
                format,
                r#"<cols><col min="1" max="1" width="1e99" customWidth="1"/><col min="2" max="2" width="-1"/></cols>"#,
                ROWS,
            ));
            let sheet = &wb.sheets[0];
            assert!(
                (1.0..=1800.0).contains(&sheet.default_col_width),
                "{format}: {}",
                sheet.default_col_width
            );
            assert!(
                (1.0..=546.0).contains(&sheet.default_row_height),
                "{format}: {}",
                sheet.default_row_height
            );
            assert!(
                sheet
                    .col_widths
                    .values()
                    .all(|w| (0.0..=1800.0).contains(w))
            );
            // Whatever was read makes a file that reads the same.
            let again = load(&to_bytes(&wb));
            assert_eq!(
                again.sheets[0].default_row_height, sheet.default_row_height,
                "{format} / {font_size}"
            );
            assert!((again.sheets[0].default_col_width - sheet.default_col_width).abs() < 0.01);
        }
    }
}
