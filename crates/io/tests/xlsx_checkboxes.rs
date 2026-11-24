//! Checkboxes in cells (Insert > Checkbox) in `.xlsx` files: what the
//! writer puts in the package, that they come back, and a file made the
//! way Excel makes it.

mod common;

use common::{assert_well_formed, keep_sample, package, part, part_names};
use opensheet_engine::{Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn write(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .expect("the workbook is written")
        .into_inner()
}

fn read(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the workbook is read")
}

#[test]
fn checkboxes_are_written_as_excel_writes_them_and_come_back() {
    let mut wb = Workbook::new(Locale::en());
    wb.insert_checkboxes(0, &[Rect::new(0, 0, 1, 0)]);
    wb.set_value(0, 0, 0, Value::Bool(true));
    wb.set_value(0, 0, 1, Value::text("plain"));
    let bytes = write(&wb);
    keep_sample("checkboxes.xlsx", &bytes);

    assert!(
        part_names(&bytes).contains(&"xl/featurePropertyBag/featurePropertyBag.xml".to_string())
    );
    let bag = part(&bytes, "xl/featurePropertyBag/featurePropertyBag.xml");
    assert_well_formed("featurePropertyBag.xml", &bag);
    assert!(bag.contains("<bag type=\"Checkbox\"/>"));
    let styles = part(&bytes, "xl/styles.xml");
    assert_well_formed("styles.xml", &styles);
    assert_eq!(styles.matches("<xfpb:xfComplement i=\"0\"/>").count(), 1);
    assert!(part(&bytes, "[Content_Types].xml").contains(
        "<Override PartName=\"/xl/featurePropertyBag/featurePropertyBag.xml\" ContentType=\"application/vnd.ms-excel.featurepropertybag+xml\"/>"
    ));
    assert!(part(&bytes, "xl/_rels/workbook.xml.rels").contains(
        "Type=\"http://schemas.microsoft.com/office/2022/11/relationships/FeaturePropertyBag\" Target=\"featurePropertyBag/featurePropertyBag.xml\""
    ));

    let back = read(&bytes);
    assert_eq!(back.checkbox_at(0, 0, 0), Some(true));
    assert_eq!(back.checkbox_at(0, 1, 0), Some(false));
    assert_eq!(back.checkbox_at(0, 0, 1), None);
    assert!(!back.style_at(0, 0, 1).checkbox);
}

#[test]
fn a_workbook_without_checkboxes_has_no_property_bags() {
    let mut wb = Workbook::new(Locale::en());
    wb.set_value(0, 0, 0, Value::Bool(true));
    let bytes = write(&wb);
    assert!(
        !part_names(&bytes)
            .iter()
            .any(|n| n.contains("featurePropertyBag"))
    );
    assert!(!part(&bytes, "xl/styles.xml").contains("xfComplement"));
}

/// The parts of a workbook with checkboxes in A1 (ticked) and A2, as
/// Excel and XlsxWriter write them.
#[test]
fn checkboxes_of_an_excel_file_are_read() {
    let decl = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
    let types = format!(
        "{decl}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/><Override PartName=\"/xl/featurePropertyBag/featurePropertyBag.xml\" ContentType=\"application/vnd.ms-excel.featurepropertybag+xml\"/></Types>"
    );
    let rels = format!(
        "{decl}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
    );
    let workbook = format!(
        "{decl}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "{decl}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId5\" Type=\"http://schemas.microsoft.com/office/2022/11/relationships/FeaturePropertyBag\" Target=\"featurePropertyBag/featurePropertyBag.xml\"/></Relationships>"
    );
    let bag = format!(
        "{decl}<FeaturePropertyBags xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2022/featurepropertybag\"><bag type=\"Checkbox\"/><bag type=\"XFControls\"><bagId k=\"CellControl\">0</bagId></bag><bag type=\"XFComplement\"><bagId k=\"XFControls\">1</bagId></bag><bag type=\"XFComplements\" extRef=\"XFComplementsMapperExtRef\"><a k=\"MappedFeaturePropertyBags\"><bagId>2</bagId></a></bag></FeaturePropertyBags>"
    );
    let styles = format!(
        "{decl}<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"2\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"><extLst><ext uri=\"{{C7286773-470A-42A8-94C5-96B5CB345126}}\" xmlns:xfpb=\"http://schemas.microsoft.com/office/spreadsheetml/2022/featurepropertybag\"><xfpb:xfComplement i=\"0\"/></ext></extLst></xf></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>"
    );
    let sheet = format!(
        "{decl}<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" s=\"1\" t=\"b\"><v>1</v></c><c r=\"B1\" t=\"b\"><v>1</v></c></row><row r=\"2\"><c r=\"A2\" s=\"1\" t=\"b\"><v>0</v></c></row></sheetData></worksheet>"
    );
    let bytes = package(&[
        ("[Content_Types].xml", &types),
        ("_rels/.rels", &rels),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &workbook_rels),
        ("xl/featurePropertyBag/featurePropertyBag.xml", &bag),
        ("xl/styles.xml", &styles),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]);
    let wb = read(&bytes);
    assert_eq!(wb.checkbox_at(0, 0, 0), Some(true));
    assert_eq!(wb.checkbox_at(0, 1, 0), Some(false));
    // TRUE without the format stays the word TRUE.
    assert_eq!(wb.checkbox_at(0, 0, 1), None);
    // Saved again, they are still checkboxes.
    let again = read(&write(&wb));
    assert_eq!(again.checkbox_at(0, 0, 0), Some(true));
}
