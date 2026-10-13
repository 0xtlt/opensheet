//! Protection in `.xlsx` files: sheets, the workbook and the locked /
//! hidden flags of cells; our own round trip, protection as Excel writes
//! it (with passwords hashed by an independent implementation), and
//! damaged input.

mod common;

use common::{
    assert_well_formed, assert_workbook_order, assert_worksheet_order, at, enter, package, part,
    value,
};
use opensheet_engine::protect::{
    Action, Denied, PasswordError, PasswordHash, Permission, SheetProtection, WorkbookProtection,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::{Locale, Rect, Value, Workbook};
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

fn may_edit(wb: &Workbook, sheet: usize, cell: &str) -> Result<(), Denied> {
    let (row, col) = at(cell);
    wb.permits(&Action::EditCells {
        sheet,
        ranges: &[Rect::cell(row, col)],
    })
}

fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    wb.add_sheet(3);
    enter(&mut wb, 0, "A1", "locked");
    enter(&mut wb, 0, "B1", "open");
    enter(&mut wb, 0, "C1", "=A1&B1");
    enter(&mut wb, 0, "D1", "=LEN(A1)");
    let cell = |name: &str| Rect::parse(name).unwrap();
    wb.update_style(0, cell("B1"), &|s| s.locked = false);
    wb.update_style(0, cell("C1"), &|s| s.hidden = true);
    wb.update_style(0, cell("D1"), &|s| {
        s.locked = false;
        s.hidden = true;
    });
    // A whole column and a whole row open for input.
    wb.update_style(0, Rect::new(0, 5, MAX_ROWS - 1, 5), &|s| s.locked = false);
    wb.update_style(1, Rect::new(3, 0, 3, MAX_COLS - 1), &|s| s.locked = false);
    wb.sheets[0].merges.push(cell("A3:B4"));
    wb.commit("setup");

    // What the dialog proposes, with a password.
    assert!(wb.protect_sheet(0, SheetProtection::default(), "Excel-2025"));
    // A choice of permissions, no password.
    let some = SheetProtection::allowing(&[
        Permission::SelectUnlockedCells,
        Permission::FormatCells,
        Permission::InsertRows,
        Permission::Sort,
        Permission::AutoFilter,
        Permission::EditObjects,
    ]);
    assert!(wb.protect_sheet(1, some, ""));
    // The password hash of older files.
    let mut old = SheetProtection::allowing(&Permission::ALL);
    old.password = Some(PasswordHash::legacy("test"));
    wb.sheets[2].protection = Some(old);
    wb.protect_workbook(true, false, "classeur");
    wb
}

#[test]
fn protection_round_trips() {
    let original = sample();
    let bytes = to_bytes(&original);
    common::keep_sample("protection.xlsx", &bytes);
    let mut wb = load(&bytes);
    for (i, sheet) in original.sheets.iter().enumerate() {
        assert_eq!(sheet.protection, wb.sheets[i].protection, "sheet {i}");
    }
    assert_eq!(original.protection, wb.protection);
    assert!(!wb.modified && !wb.can_undo());

    // Locked and hidden cells.
    let flags = |wb: &Workbook, sheet: usize, cell: &str| {
        let (row, col) = at(cell);
        let style = wb.style_at(sheet, row, col);
        (style.locked, style.hidden)
    };
    assert_eq!(flags(&wb, 0, "A1"), (true, false));
    assert_eq!(flags(&wb, 0, "B1"), (false, false));
    assert_eq!(flags(&wb, 0, "C1"), (true, true));
    assert_eq!(flags(&wb, 0, "D1"), (false, true));
    assert_eq!(flags(&wb, 0, "F900"), (false, false));
    assert_eq!(flags(&wb, 1, "Z4"), (false, false));
    assert_eq!(flags(&wb, 1, "Z5"), (true, false));

    // The protection protects.
    assert_eq!(may_edit(&wb, 0, "A1"), Err(Denied::Sheet));
    assert_eq!(may_edit(&wb, 0, "B1"), Ok(()));
    assert_eq!(may_edit(&wb, 0, "F7"), Ok(()));
    assert_eq!(
        may_edit(&wb, 3, "A1"),
        Ok(()),
        "the last sheet is not protected"
    );
    assert!(wb.is_formula_hidden(0, 0, 2) && !wb.is_formula_hidden(0, 0, 0));
    assert_eq!(wb.formula_bar_text(0, 0, 2), "");
    assert_eq!(
        wb.permits(&Action::FormatCells { sheet: 0 }),
        Err(Denied::Sheet)
    );
    assert_eq!(wb.permits(&Action::FormatCells { sheet: 1 }), Ok(()));
    assert_eq!(wb.permits(&Action::InsertRows { sheet: 1 }), Ok(()));
    assert_eq!(
        wb.permits(&Action::InsertColumns { sheet: 1 }),
        Err(Denied::Sheet)
    );
    assert!(!wb.can_select_cell(1, 0, 0) && wb.can_select_cell(1, 3, 0));
    assert_eq!(wb.permits(&Action::RenameSheet), Err(Denied::Workbook));

    // The passwords still open what they closed.
    assert!(wb.sheet_needs_password(0) && !wb.sheet_needs_password(1));
    assert_eq!(
        wb.unprotect_sheet(0, "excel-2025"),
        Err(PasswordError::Wrong)
    );
    assert_eq!(wb.unprotect_sheet(0, "Excel-2025"), Ok(()));
    assert_eq!(may_edit(&wb, 0, "A1"), Ok(()));
    assert_eq!(wb.unprotect_sheet(1, ""), Ok(()));
    assert_eq!(wb.unprotect_sheet(2, "Test"), Err(PasswordError::Wrong));
    assert_eq!(wb.unprotect_sheet(2, "test"), Ok(()));
    assert!(wb.workbook_needs_password());
    assert_eq!(wb.unprotect_workbook(""), Err(PasswordError::Wrong));
    assert_eq!(wb.unprotect_workbook("classeur"), Ok(()));
    assert_eq!(wb.permits(&Action::RenameSheet), Ok(()));

    // A second generation is the same file: hashes and salts are kept,
    // not made again.
    let again = to_bytes(&load(&bytes));
    for name in [
        "xl/workbook.xml",
        "xl/worksheets/sheet1.xml",
        "xl/styles.xml",
    ] {
        assert_eq!(part(&bytes, name), part(&again, name), "{name}");
    }
    // Unprotected, nothing of it is left in the file.
    let open = to_bytes(&wb);
    assert!(!part(&open, "xl/workbook.xml").contains("Protection"));
    for n in 1..=4 {
        let name = format!("xl/worksheets/sheet{n}.xml");
        assert!(!part(&open, &name).contains("sheetProtection"), "{name}");
    }
}

#[test]
fn protection_is_written_the_way_excel_writes_it() {
    let bytes = to_bytes(&sample());
    for n in 1..=4 {
        let name = format!("xl/worksheets/sheet{n}.xml");
        let sheet = part(&bytes, &name);
        assert_well_formed(&name, &sheet);
        assert_worksheet_order(&name, &sheet);
    }
    let workbook = part(&bytes, "xl/workbook.xml");
    assert_well_formed("workbook.xml", &workbook);
    assert_workbook_order(&workbook);

    // Sheet 1: a salted SHA-512 hash, as Excel 2013 and later make them,
    // between the cells and the merges.
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    let start = sheet
        .find("<sheetProtection")
        .expect("sheet 1 is protected");
    let element = &sheet[start..start + sheet[start..].find("/>").unwrap() + 2];
    assert!(sheet[..start].ends_with("</sheetData>"), "{sheet}");
    assert!(sheet[start + element.len()..].starts_with("<mergeCells"));
    assert!(
        element.starts_with("<sheetProtection algorithmName=\"SHA-512\" hashValue=\""),
        "{element}"
    );
    assert!(
        element.ends_with("\" spinCount=\"100000\" sheet=\"1\" objects=\"1\" scenarios=\"1\"/>"),
        "{element}"
    );
    let attr = |name: &str| {
        let from = element.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        &element[from..from + element[from..].find('"').unwrap()]
    };
    assert_eq!(attr("hashValue").len(), 88, "64 bytes in Base64");
    assert_eq!(attr("saltValue").len(), 24, "16 bytes in Base64");

    assert!(part(&bytes, "xl/worksheets/sheet2.xml").contains(concat!(
        "<sheetProtection sheet=\"1\" scenarios=\"1\" formatCells=\"0\" insertRows=\"0\" ",
        "selectLockedCells=\"1\" sort=\"0\" autoFilter=\"0\"/>"
    )));
    assert!(part(&bytes, "xl/worksheets/sheet3.xml").contains(concat!(
        "<sheetProtection password=\"CBEB\" sheet=\"1\" formatCells=\"0\" formatColumns=\"0\" ",
        "formatRows=\"0\" insertColumns=\"0\" insertRows=\"0\" insertHyperlinks=\"0\" ",
        "deleteColumns=\"0\" deleteRows=\"0\" sort=\"0\" autoFilter=\"0\" pivotTables=\"0\"/>"
    )));
    assert!(!part(&bytes, "xl/worksheets/sheet4.xml").contains("sheetProtection"));

    let start = workbook
        .find("<workbookProtection")
        .expect("the workbook is protected");
    assert!(workbook[..start].ends_with("<workbookPr/>"));
    let element = &workbook[start..start + workbook[start..].find("/>").unwrap() + 2];
    assert!(workbook[start + element.len()..].starts_with("<bookViews>"));
    assert!(
        element.starts_with(
            "<workbookProtection workbookAlgorithmName=\"SHA-512\" workbookHashValue=\""
        ),
        "{element}"
    );
    assert!(element.contains("\" workbookSaltValue=\""));
    assert!(element.ends_with("\" workbookSpinCount=\"100000\" lockStructure=\"1\"/>"));

    // Cell formats say only what is not the default (locked, shown).
    let styles = part(&bytes, "xl/styles.xml");
    for expected in [
        " applyProtection=\"1\"><protection locked=\"0\"/></xf>",
        " applyProtection=\"1\"><protection hidden=\"1\"/></xf>",
        " applyProtection=\"1\"><protection locked=\"0\" hidden=\"1\"/></xf>",
    ] {
        assert_eq!(styles.matches(expected).count(), 1, "{expected}\n{styles}");
    }
    assert!(
        styles.contains("<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>")
    );
}

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

// Hashes of the passwords "Excel-2025" and "classeur", computed with
// Python's hashlib as ECMA-376 describes: H0 = SHA-512(salt + password in
// UTF-16LE), then 100 000 times H = SHA-512(H + round as 4 bytes).
const SHEET_HASH: &str =
    "Q8twIrovgraBFXtOM+YLQ3DcuRcBzfPaysLQEDdVEs7rysxZQi4ivagvwOufryTCYiVkPCV1Zg0g4HEcLPxHew==";
const SHEET_SALT: &str = "EBESExQVFhcYGRobHB0eHw==";
const BOOK_HASH: &str =
    "nELMulKVWf5cL3lbWHvNgLdAnL97S6ftoaUXpsc1Y4Iz2CztI9ul2qrweZdVP26kWelgkvTbSH3WVFgMZBsgJA==";
const BOOK_SALT: &str = "ihEC/pkQM0fADV6nIWuQBA==";

/// A workbook as Excel saves it: protected structure, a sheet protected
/// with a password and a few permissions, one with the old kind of hash,
/// one with protection attributes but not protected, one with hashes of
/// other algorithms.
fn excel_file() -> Vec<u8> {
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {MAIN}><fileVersion appName="xl" lastEdited="7" lowestEdited="7" rupBuild="28129"/><workbookPr defaultThemeVersion="202300"/>
<workbookProtection workbookAlgorithmName="SHA-512" workbookHashValue="{BOOK_HASH}" workbookSaltValue="{BOOK_SALT}" workbookSpinCount="100000" lockStructure="1"/>
<bookViews><workbookView xWindow="0" yWindow="0" windowWidth="28800" windowHeight="17600"/></bookViews>
<sheets><sheet name="Budget" sheetId="1" r:id="rId1"/><sheet name="Ancien" sheetId="2" r:id="rId2"/><sheet name="Libre" sheetId="3" r:id="rId3"/><sheet name="Autres" sheetId="4" r:id="rId4"/></sheets>
<calcPr calcId="191029"/></workbook>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet {MAIN}><fonts count="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyProtection="1"><protection locked="0"/></xf>
<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyProtection="1"><protection hidden="1"/></xf>
<xf numFmtId="2" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyProtection="1"><protection locked="0" hidden="1"/></xf>
<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1" applyProtection="1"><alignment horizontal="center"/><protection locked="1" hidden="0"/></xf></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
    );
    let budget = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN}><dimension ref="A1:E1"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr baseColWidth="10" defaultRowHeight="15"/>
<sheetData><row r="1"><c r="A1"><v>1</v></c><c r="B1" s="1"><v>2</v></c><c r="C1" s="2"><f>A1+B1</f><v>3</v></c><c r="D1" s="3"><f>C1*2</f><v>6</v></c><c r="E1" s="4"><v>5</v></c></row></sheetData>
<sheetProtection algorithmName="SHA-512" hashValue="{SHEET_HASH}" saltValue="{SHEET_SALT}" spinCount="100000" sheet="1" objects="1" scenarios="1" formatCells="0" formatColumns="0" insertRows="0" sort="0"/>
<mergeCells count="1"><mergeCell ref="A3:B4"/></mergeCells>
<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    );
    let old = format!(
        r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><sheetProtection password="CBEB" sheet="1" objects="1" scenarios="1" selectLockedCells="1" selectUnlockedCells="1"/></worksheet>"#
    );
    let free = format!(
        r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><sheetProtection password="83AF" objects="1" scenarios="1"/></worksheet>"#
    );
    // SHA-1 over 50 000 rounds ("vieux"), then a second, stray element.
    let others = format!(
        r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><sheetProtection algorithmName="SHA-1" hashValue="J8qQCA36j3ivvYGVcXcBmhg1hBA=" saltValue="{SHEET_SALT}" spinCount="50000" sheet="true" autoFilter="false" pivotTables="0"/></worksheet>"#
    );
    package(&[
        ("xl/workbook.xml", &workbook),
        ("xl/styles.xml", &styles),
        ("xl/worksheets/sheet1.xml", &budget),
        ("xl/worksheets/sheet2.xml", &old),
        ("xl/worksheets/sheet3.xml", &free),
        ("xl/worksheets/sheet4.xml", &others),
    ])
}

#[test]
fn protection_from_excel() {
    let mut wb = load(&excel_file());

    assert_eq!(
        wb.protection,
        WorkbookProtection {
            lock_structure: true,
            lock_windows: false,
            password: Some(PasswordHash::Salted {
                algorithm: "SHA-512".into(),
                hash: BOOK_HASH.into(),
                salt: BOOK_SALT.into(),
                spin_count: 100_000,
            }),
        }
    );
    assert_eq!(wb.permits(&Action::DeleteSheet), Err(Denied::Workbook));

    let budget = wb.sheets[0]
        .protection
        .clone()
        .expect("Budget is protected");
    let allowed: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|p| budget.allows(*p))
        .collect();
    assert_eq!(
        allowed,
        [
            Permission::SelectLockedCells,
            Permission::SelectUnlockedCells,
            Permission::FormatCells,
            Permission::FormatColumns,
            Permission::InsertRows,
            Permission::Sort,
        ]
    );
    assert_eq!(may_edit(&wb, 0, "A1"), Err(Denied::Sheet));
    assert_eq!(may_edit(&wb, 0, "B1"), Ok(()));
    assert_eq!(may_edit(&wb, 0, "C1"), Err(Denied::Sheet));
    assert_eq!(may_edit(&wb, 0, "D1"), Ok(()));
    assert_eq!(may_edit(&wb, 0, "E1"), Err(Denied::Sheet));
    assert_eq!(wb.formula_bar_text(0, 0, 0), "1");
    assert_eq!(wb.formula_bar_text(0, 0, 2), "", "a hidden formula");
    assert_eq!(wb.formula_bar_text(0, 0, 3), "");
    assert_eq!(value(&wb, 0, "D1"), Value::Number(6.0));
    assert_eq!(wb.sheets[0].merges.len(), 1);

    let old = wb.sheets[1]
        .protection
        .clone()
        .expect("Ancien is protected");
    assert_eq!(old.password, Some(PasswordHash::Legacy(0xCBEB)));
    assert!(!wb.can_select_cell(1, 0, 0));
    assert!(Permission::ALL.into_iter().all(|p| !old.allows(p)));
    assert_eq!(wb.sheets[2].protection, None, "objects and scenarios only");
    assert_eq!(may_edit(&wb, 2, "A1"), Ok(()));
    let others = wb.sheets[3]
        .protection
        .clone()
        .expect("Autres is protected");
    assert!(others.allows(Permission::AutoFilter) && others.allows(Permission::PivotTables));
    assert!(!others.allows(Permission::Sort) && others.allows(Permission::EditObjects));

    // Saved and read again, everything is as it was, hashes included.
    let bytes = to_bytes(&wb);
    common::keep_sample("protection-from-excel.xlsx", &bytes);
    let again = load(&bytes);
    assert_eq!(again.protection, wb.protection);
    for (a, b) in wb.sheets.iter().zip(&again.sheets) {
        assert_eq!(a.protection, b.protection, "{}", a.name);
    }
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains(&format!(
            "<sheetProtection algorithmName=\"SHA-512\" hashValue=\"{SHEET_HASH}\" saltValue=\"{SHEET_SALT}\" spinCount=\"100000\" sheet=\"1\" objects=\"1\" scenarios=\"1\" formatCells=\"0\" formatColumns=\"0\" insertRows=\"0\" sort=\"0\"/>"
        )),
        "{sheet}"
    );
    assert_worksheet_order("sheet1.xml", &sheet);

    // The passwords Excel was given are the ones that open.
    assert_eq!(
        wb.unprotect_sheet(0, "excel-2025"),
        Err(PasswordError::Wrong)
    );
    assert_eq!(wb.unprotect_sheet(0, ""), Err(PasswordError::Wrong));
    assert_eq!(wb.unprotect_sheet(0, "Excel-2025"), Ok(()));
    assert_eq!(may_edit(&wb, 0, "A1"), Ok(()));
    assert_eq!(wb.unprotect_sheet(1, "test"), Ok(()));
    assert_eq!(wb.unprotect_sheet(3, "neuf"), Err(PasswordError::Wrong));
    assert_eq!(wb.unprotect_sheet(3, "vieux"), Ok(()));
    assert_eq!(wb.unprotect_workbook("Classeur"), Err(PasswordError::Wrong));
    assert_eq!(wb.unprotect_workbook("classeur"), Ok(()));
    assert_eq!(wb.permits(&Action::DeleteSheet), Ok(()));
}

#[test]
fn damaged_protection_costs_nothing_else() {
    let sheet = |protection: &str| {
        format!(
            r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>{protection}<mergeCells><mergeCell ref="A3:B4"/></mergeCells></worksheet>"#
        )
    };
    let cases = [
        // (element, protected?, has a password?)
        (r#"<sheetProtection/>"#, false, false),
        (r#"<sheetProtection sheet="maybe"/>"#, false, false),
        (
            r#"<sheetProtection sheet="1" password="not hex" objects="?" sort="2"/>"#,
            true,
            false,
        ),
        (
            r#"<sheetProtection sheet="1" password="12345"/>"#,
            true,
            false,
        ),
        (
            r#"<sheetProtection sheet="1" algorithmName="" hashValue="" saltValue=""/>"#,
            true,
            false,
        ),
        (
            r#"<sheetProtection sheet="1" algorithmName="ROT13" hashValue="!!!" saltValue="???" spinCount="-1"/>"#,
            true,
            true,
        ),
        (
            r#"<sheetProtection sheet="1" algorithmName="SHA-512" hashValue="AAAA" spinCount="4000000000"/>"#,
            true,
            false,
        ),
        (
            r#"<sheetProtection sheet="1" algorithmName="SHA-512" hashValue="AAAA" saltValue="AAAA" spinCount="1000"/>"#,
            true,
            true,
        ),
        (
            r#"<sheetProtection sheet="1"><password>83AF</password></sheetProtection>"#,
            true,
            false,
        ),
        // The tag never ends: what follows is taken for more of it.
        (r#"<sheetProtection sheet="1" password="83AF""#, true, true),
    ];
    for (element, protected, password) in cases {
        let bytes = package(&[
            (
                "xl/workbook.xml",
                r#"<workbook><workbookProtection lockStructure="x" lockWindows="1" workbookPassword="zz" workbookAlgorithmName="SHA-512"/><sheets><sheet name="S" r:id="rId1"/></sheets></workbook>"#,
            ),
            ("xl/worksheets/sheet1.xml", &sheet(element)),
        ]);
        let mut wb = load(&bytes);
        assert_eq!(value(&wb, 0, "A1"), Value::Number(1.0), "{element}");
        assert_eq!(wb.is_sheet_protected(0), protected, "{element}");
        assert_eq!(wb.sheet_needs_password(0), password, "{element}");
        assert!(!wb.protection.lock_structure && wb.protection.lock_windows);
        assert_eq!(wb.protection.password, None);
        // A hash nobody can make opens to nobody, and does not panic.
        if password {
            assert_eq!(wb.unprotect_sheet(0, "anything"), Err(PasswordError::Wrong));
        }
        // What was read can be written and read again.
        let again = load(&to_bytes(&wb));
        assert_eq!(
            again.sheets[0].protection, wb.sheets[0].protection,
            "{element}"
        );
        assert_eq!(again.protection, wb.protection);
    }
}

/// Review > Allow Edit Ranges: `<protectedRanges>`, between
/// `<sheetProtection>` and what follows it, with the password hashed as
/// the sheet's is.
#[test]
fn ranges_that_may_be_edited_round_trip() {
    use opensheet_engine::protect::ProtectedRange;
    let rect = |name: &str| Rect::parse(name).unwrap();
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [("A1", "Open to all"), ("A3", "Password: pw"), ("C3", "x"), ("A5", "Locked")] {
        enter(&mut wb, 0, cell, text);
    }
    let salted = PasswordHash::salted("pw", b"0123456789abcdef", 100_000);
    let mut described = ProtectedRange::new("Legacy_1", vec![rect("E1")], Some(PasswordHash::legacy("pw")));
    described.security_descriptor = Some("O:WDG:WDD:(A;;CC;;;S-1-5-21-1)".into());
    wb.set_protected_ranges(
        0,
        vec![ProtectedRange::new("Range1", vec![rect("A1:B1")], None), ProtectedRange::new("Budget", vec![rect("A3:B3"), rect("C3")], Some(salted)), described],
    );
    wb.commit("Allow Edit Ranges");
    wb.protect_sheet(0, SheetProtection::default(), "");
    let bytes = to_bytes(&wb);
    common::keep_sample("protected-ranges.xlsx", &bytes);

    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1", &sheet);
    assert_worksheet_order("sheet1", &sheet);
    let start = sheet.find("<protectedRanges>").expect("the ranges");
    let end = sheet.find("</protectedRanges>").unwrap();
    assert!(sheet[..start].ends_with("<sheetProtection sheet=\"1\" objects=\"1\" scenarios=\"1\"/>"), "{sheet}");
    let ranges = &sheet[start..end];
    assert!(ranges.contains("<protectedRange sqref=\"A1:B1\" name=\"Range1\"/>"), "{ranges}");
    assert!(ranges.contains("<protectedRange algorithmName=\"SHA-512\" hashValue=\""), "{ranges}");
    assert!(ranges.contains("\" saltValue=\"MDEyMzQ1Njc4OWFiY2RlZg==\" spinCount=\"100000\" sqref=\"A3:B3 C3\" name=\"Budget\"/>"), "{ranges}");
    assert!(ranges.contains("<protectedRange password=\"CF75\" sqref=\"E1\" name=\"Legacy_1\" securityDescriptor=\"O:WDG:WDD:(A;;CC;;;S-1-5-21-1)\"/>"), "{ranges}");

    let mut back = load(&bytes);
    let read = &back.sheets[0].protected_ranges;
    let written = &wb.sheets[0].protected_ranges;
    assert_eq!(read.len(), 3);
    for (a, b) in read.iter().zip(written) {
        assert_eq!((&a.name, &a.ranges, &a.password, &a.security_descriptor), (&b.name, &b.ranges, &b.password, &b.security_descriptor));
    }
    assert_eq!(may_edit(&back, 0, "A1"), Ok(()));
    assert_eq!(may_edit(&back, 0, "A5"), Err(Denied::Sheet));
    // The password was not given in this session yet.
    assert_eq!(may_edit(&back, 0, "C3"), Err(Denied::Sheet));
    assert_eq!(back.range_awaiting_password(0, &[rect("C3")]), Some(1));
    assert_eq!(back.unlock_range(0, 1, "pw"), Ok(()));
    assert_eq!(may_edit(&back, 0, "C3"), Ok(()));
    assert_eq!(back.unlock_range(0, 2, "pw"), Ok(()));

    // Ranges Excel wrote on a sheet that is not protected are kept; one
    // without cells or without a title is left out.
    let plain = sheet.replace("<sheetProtection sheet=\"1\" objects=\"1\" scenarios=\"1\"/>", "").replace("sqref=\"E1\"", "sqref=\"\"");
    let names = common::part_names(&bytes);
    let texts: Vec<String> = names.iter().map(|n| if n == "xl/worksheets/sheet1.xml" { plain.clone() } else { part(&bytes, n) }).collect();
    let parts: Vec<(&str, &str)> = names.iter().map(String::as_str).zip(texts.iter().map(String::as_str)).collect();
    let back = load(&package(&parts));
    assert!(back.sheets[0].protection.is_none());
    assert_eq!(back.sheets[0].protected_ranges.len(), 2);
}
