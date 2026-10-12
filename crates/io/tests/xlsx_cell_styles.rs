//! Cell styles in `.xlsx` files: our own round trip, a styles part in the
//! form Excel gives it (names in the language of the file, known by their
//! `builtinId`), and damaged input.

mod common;

use common::{assert_well_formed, at, enter, package, part};
use opensheet_engine::cell_style::{StyleGroup, StyleParts};
use opensheet_engine::{Color, Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn style_at<'a>(wb: &'a Workbook, name: &str) -> &'a str {
    let (row, col) = at(name);
    wb.cell_style_at(0, row, col)
}

fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [("A1", "fine"), ("A2", "0.25"), ("A3", "Report"), ("A4", "mine"), ("A5", "plain"), ("A6", "note")] {
        enter(&mut wb, 0, cell, text);
    }
    wb.apply_cell_style(0, &[rect("A1")], "Good").unwrap();
    wb.apply_cell_style(0, &[rect("A2")], "Percent").unwrap();
    wb.apply_cell_style(0, &[rect("A3")], "Heading 1").unwrap();
    // Good with a format of its own on top.
    wb.apply_cell_style(0, &[rect("B1")], "Good").unwrap();
    wb.update_style(0, rect("B1"), &|s| s.font.italic = true);
    // A style of the user's that brings a fill and a number format.
    let mut look = wb.styles.default_style().clone();
    look.fill = Some(Color(0x123456));
    look.num_fmt = "0.000".into();
    look.font.bold = true;
    let parts = StyleParts {
        number: true,
        fill: true,
        ..StyleParts::NONE
    };
    wb.add_cell_style("My <style>", look, parts).unwrap();
    wb.apply_cell_style(0, &[rect("A4")], "My <style>").unwrap();
    // A built-in style changed here.
    let mut note = wb.cell_style("Note").unwrap();
    note.style.fill = Some(Color(0xABCDEF));
    wb.modify_cell_style("Note", note.style, note.includes).unwrap();
    wb.apply_cell_style(0, &[rect("A6")], "Note").unwrap();
    wb.commit("styles");
    wb
}

#[test]
fn styles_survive_a_round_trip() {
    let wb = sample();
    let bytes = to_bytes(&wb);
    common::keep_sample("cell-styles.xlsx", &bytes);
    let styles = part(&bytes, "xl/styles.xml");
    assert_well_formed("styles.xml", &styles);
    // Normal, then the styles defined or in use, each with its format.
    let names = styles.split("<cellStyles").nth(1).unwrap().split("</cellStyles>").next().unwrap();
    assert_eq!(
        names,
        " count=\"6\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/>\
         <cellStyle name=\"My &lt;style&gt;\" xfId=\"1\"/>\
         <cellStyle name=\"Good\" xfId=\"2\" builtinId=\"26\"/>\
         <cellStyle name=\"Note\" xfId=\"3\" builtinId=\"10\" customBuiltin=\"1\"/>\
         <cellStyle name=\"Heading 1\" xfId=\"4\" builtinId=\"16\"/>\
         <cellStyle name=\"Percent\" xfId=\"5\" builtinId=\"5\"/>"
    );
    assert!(styles.contains("<cellStyleXfs count=\"6\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/>"), "{styles}");
    // The style of the user leaves out what it does not include.
    assert!(styles.contains("applyFont=\"0\" applyBorder=\"0\" applyAlignment=\"0\" applyProtection=\"0\"/>"), "{styles}");
    // The cells name their style.
    let cell_xfs = styles.split("<cellXfs").nth(1).unwrap().split("</cellXfs>").next().unwrap();
    for id in 0..6 {
        assert!(cell_xfs.contains(&format!("xfId=\"{id}\"")), "xfId {id} in {cell_xfs}");
    }

    let back = load(&bytes);
    for (cell, style) in [("A1", "Good"), ("A2", "Percent"), ("A3", "Heading 1"), ("A4", "My <style>"), ("A5", "Normal"), ("A6", "Note"), ("B1", "Good")] {
        assert_eq!(style_at(&back, cell), style, "{cell}");
    }
    assert!(back.style_at(0, 0, 1).font.italic);
    assert_eq!(back.style_at(0, 3, 0).fill, Some(Color(0x123456)));
    assert!(!back.style_at(0, 3, 0).font.bold);
    // Only what Excel's catalogue lacks is kept by the workbook.
    assert_eq!(back.cell_styles, wb.cell_styles);
    let mine = back.cell_style("My <style>").unwrap();
    assert_eq!((mine.group(), mine.includes.fill, mine.includes.font), (StyleGroup::Custom, true, false));
    assert_eq!(back.cell_style("Note").unwrap().style.fill, Some(Color(0xABCDEF)));
    assert_eq!(back.cell_style("Good"), wb.cell_style("Good"));
    // And once more, unchanged.
    assert_eq!(part(&to_bytes(&back), "xl/styles.xml"), styles);
}

#[test]
fn a_workbook_without_styles_writes_normal_alone() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "x");
    wb.update_style(0, rect("A1"), &|s| s.font.bold = true);
    wb.commit("bold");
    let styles = part(&to_bytes(&wb), "xl/styles.xml");
    assert!(styles.contains("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>"));
    assert!(styles.contains("<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>"));
}

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Feuil1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

const SHEET: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1"><v>1</v></c><c r="B1" s="2"><v>2</v></c><c r="C1" s="3"><v>3</v></c><c r="D1" s="4"><v>4</v></c><c r="E1"><v>5</v></c></row></sheetData></worksheet>"#;

/// A styles part in the form a French Excel gives it: the built-in styles
/// under their French names, the formats of the styles with the parts
/// they leave out, a style of the user's, and the styles listed in another
/// order than their formats.
const FRENCH_STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="4"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font><font><sz val="11"/><color rgb="FF006100"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font><font><sz val="11"/><color rgb="FF9C0006"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font><font><b/><sz val="14"/><color rgb="FFFF0000"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="5"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFC6EFCE"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFC7CE"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FF00B0F0"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="0" fontId="2" fillId="3" borderId="0" applyNumberFormat="0" applyBorder="0" applyAlignment="0" applyProtection="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" applyNumberFormat="0" applyBorder="0" applyAlignment="0" applyProtection="0"/><xf numFmtId="0" fontId="3" fillId="4" borderId="0" applyNumberFormat="0" applyBorder="0" applyAlignment="0" applyProtection="0"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="2"/><xf numFmtId="0" fontId="2" fillId="3" borderId="0" xfId="1"/><xf numFmtId="0" fontId="3" fillId="4" borderId="0" xfId="3"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="9"/></cellXfs><cellStyles count="4"><cellStyle name="Insatisfaisant" xfId="1" builtinId="27"/><cellStyle name="Mon style" xfId="3"/><cellStyle name="Normal" xfId="0" builtinId="0"/><cellStyle name="Satisfaisant" xfId="2" builtinId="26"/></cellStyles><dxfs count="0"/><tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/></styleSheet>"#;

#[test]
fn styles_from_a_french_excel_keep_their_names() {
    let bytes = package(&[("xl/workbook.xml", WORKBOOK), ("xl/worksheets/sheet1.xml", SHEET), ("xl/styles.xml", FRENCH_STYLES)]);
    let wb = load(&bytes);
    // Built-in styles go by their English names, whatever the file calls
    // them; a format naming a style that is not there is Normal.
    for (cell, style) in [("A1", "Good"), ("B1", "Bad"), ("C1", "Mon style"), ("D1", "Normal"), ("E1", "Normal")] {
        assert_eq!(style_at(&wb, cell), style, "{cell}");
    }
    // Good and Bad are Excel's own: only the style of the user is kept.
    let kept: Vec<&str> = wb.cell_styles.iter().map(|style| style.name.as_str()).collect();
    assert_eq!(kept, ["Mon style"]);
    let mine = wb.cell_style("Mon style").unwrap();
    assert_eq!(mine.style.fill, Some(Color(0x00B0F0)));
    assert!(mine.style.font.bold);
    assert_eq!(mine.includes, StyleParts { font: true, fill: true, ..StyleParts::NONE });
    // Written again and read again: the same names on the same cells.
    let again = load(&to_bytes(&wb));
    for (cell, style) in [("A1", "Good"), ("B1", "Bad"), ("C1", "Mon style"), ("D1", "Normal")] {
        assert_eq!(style_at(&again, cell), style, "{cell}");
    }
    assert_eq!(again.cell_styles, wb.cell_styles);
}

#[test]
fn damaged_style_lists_are_survived() {
    // Cut anywhere: in the fonts, the formats of the styles, the cell
    // formats, the list of styles.
    for tenth in 1..10 {
        let cut = FRENCH_STYLES.len() * tenth / 10;
        let bytes = package(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/worksheets/sheet1.xml", SHEET),
            ("xl/styles.xml", &FRENCH_STYLES[..cut]),
        ]);
        let wb = load(&bytes);
        assert_eq!(common::value(&wb, 0, "A1"), opensheet_engine::Value::Number(1.0));
        assert_well_formed("styles.xml", &part(&to_bytes(&wb), "xl/styles.xml"));
    }
}
