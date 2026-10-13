//! Macro-enabled workbooks: the VBA project and what goes with it are
//! kept byte for byte, the code names follow the sheets, and a file saved
//! without macros has nothing left that points to them.

mod common;

use common::{enter, keep_sample, part, part_names};
use opensheet_engine::{Locale, Workbook};
use opensheet_io::vba::{ModuleKind, build_project, read_project};
use std::io::{Cursor, Read, Write};

const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MS_REL: &str = "http://schemas.microsoft.com/office/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn bytes_of(package: &[u8], name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(package)).expect("a zip archive");
    let mut file = zip.by_name(name).unwrap_or_else(|_| panic!("missing part {name}"));
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    bytes
}

fn load(bytes: &[u8]) -> Workbook {
    opensheet_io::read_xlsx(Cursor::new(bytes), Locale::en()).expect("a workbook")
}

fn save_xlsm(wb: &Workbook) -> Vec<u8> {
    opensheet_io::write_xlsm(wb, Cursor::new(Vec::new())).expect("a file").into_inner()
}

fn save_xlsx(wb: &Workbook) -> Vec<u8> {
    opensheet_io::write_xlsx(wb, Cursor::new(Vec::new())).expect("a file").into_inner()
}

/// Every relationship of the package leads to a part that is there, every
/// part has a content type, and no content type is given to a part that
/// is not there: what Excel checks before anything else.
fn assert_consistent(package: &[u8]) {
    let names = part_names(package);
    let types = part(package, "[Content_Types].xml");
    for name in &names {
        if name == "[Content_Types].xml" {
            continue;
        }
        let extension = name.rsplit('.').next().unwrap_or("");
        let typed = types.contains(&format!("PartName=\"/{name}\"")) || types.contains(&format!("Extension=\"{extension}\""));
        assert!(typed, "{name} has no content type");
        if !name.ends_with(".rels") {
            continue;
        }
        // "xl/_rels/workbook.xml.rels" speaks for "xl/workbook.xml".
        let (dir, file) = name.rsplit_once("/_rels/").map_or(("", name.trim_start_matches("_rels/")), |(dir, file)| (dir, file));
        let source = file.trim_end_matches(".rels");
        if !source.is_empty() {
            let owner = if dir.is_empty() { source.to_string() } else { format!("{dir}/{source}") };
            assert!(names.contains(&owner), "{name} belongs to no part");
        }
        let rels = part(package, name);
        for chunk in rels.split("<Relationship ").skip(1) {
            if chunk.contains("TargetMode=\"External\"") {
                continue;
            }
            let target = chunk.split("Target=\"").nth(1).and_then(|rest| rest.split('"').next()).expect("a target");
            let mut path: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).collect() };
            for segment in target.trim_start_matches('/').split('/') {
                match segment {
                    ".." => {
                        path.pop();
                    }
                    "." | "" => {}
                    other => path.push(other),
                }
            }
            let resolved = path.join("/");
            assert!(names.contains(&resolved), "{name}: {target} leads nowhere");
        }
    }
    for chunk in types.split("PartName=\"/").skip(1) {
        let name = chunk.split('"').next().unwrap();
        assert!(names.iter().any(|n| n == name), "a content type for {name}, which is not there");
    }
}

/// A macro-enabled workbook as Excel lays one out: two sheets, a project,
/// its three signatures.
fn macro_workbook(project: &[u8]) -> Vec<u8> {
    let types = format!(
        "<?xml version=\"1.0\"?><Types xmlns=\"{CT}\"><Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.ms-excel.sheet.macroEnabled.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/vbaProjectSignature.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignature\"/><Override PartName=\"/xl/vbaProjectSignatureAgile.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignatureAgile\"/><Override PartName=\"/xl/vbaProjectSignatureV3.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignatureV3\"/></Types>"
    );
    let root = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let workbook = format!(
        "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><fileVersion appName=\"xl\" codeName=\"{{37E998C4-C9E5-D4B9-71C8-EB1FF731991C}}\"/><workbookPr codeName=\"ThisWorkbook\" defaultThemeVersion=\"166925\"/><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Notes\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet2.xml\"/><Relationship Id=\"rId3\" Type=\"{MS_REL}/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>"
    );
    let project_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{MS_REL}/vbaProjectSignature\" Target=\"vbaProjectSignature.bin\"/><Relationship Id=\"rId2\" Type=\"{MS_REL}/vbaProjectSignatureAgile\" Target=\"vbaProjectSignatureAgile.bin\"/><Relationship Id=\"rId3\" Type=\"{MS_REL}/vbaProjectSignatureV3\" Target=\"vbaProjectSignatureV3.bin\"/></Relationships>"
    );
    let sheet1 = format!(
        "<worksheet xmlns=\"{MAIN}\"><sheetPr codeName=\"Sheet1\"><tabColor rgb=\"FFFF0000\"/></sheetPr><sheetData><row r=\"1\"><c r=\"A1\"><v>42</v></c></row></sheetData></worksheet>"
    );
    let sheet2 = format!("<worksheet xmlns=\"{MAIN}\"><sheetPr codeName=\"Feuil2\"/><sheetData/></worksheet>");
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet1.as_bytes()),
        ("xl/worksheets/sheet2.xml", sheet2.as_bytes()),
        ("xl/vbaProject.bin", project),
        ("xl/_rels/vbaProject.bin.rels", project_rels.as_bytes()),
        ("xl/vbaProjectSignature.bin", &[0xDE, 0xAD, 0x00, 0x01]),
        ("xl/vbaProjectSignatureAgile.bin", &[0xBE, 0xEF, 0xFF]),
        ("xl/vbaProjectSignatureV3.bin", &[0x03, 0x00, 0x0D, 0x0A, 0x1A]),
    ])
}

const MODULE: &str = "Option Explicit\n\nSub FormatReport()\n    Range(\"A1:C1\").Font.Bold = True\nEnd Sub\n\nSub ClearReport()\n    Range(\"A2:C100\").ClearContents\nEnd Sub\n";

fn project() -> Vec<u8> {
    build_project(&[
        ("ThisWorkbook", ModuleKind::Document, "Private Sub Workbook_Open()\n    MsgBox \"Hello\"\nEnd Sub\n"),
        ("Sheet1", ModuleKind::Document, "Sub Recalculate()\n    Me.Calculate\nEnd Sub\n"),
        ("Feuil2", ModuleKind::Document, ""),
        ("Module1", ModuleKind::Standard, MODULE),
    ])
}

#[test]
fn a_project_goes_through_byte_for_byte() {
    let project = project();
    let original = macro_workbook(&project);
    assert_consistent(&original);
    let wb = load(&original);
    let kept = wb.vba.project.as_ref().expect("the project is kept");
    assert_eq!(&*kept.bin, project.as_slice());
    let signatures: Vec<(&str, &str, &[u8])> = kept.signatures.iter().map(|s| (s.name.as_str(), s.kind.as_str(), &*s.data)).collect();
    assert_eq!(
        signatures,
        [
            ("vbaProjectSignature.bin", "vbaProjectSignature", &[0xDE, 0xAD, 0x00, 0x01][..]),
            ("vbaProjectSignatureAgile.bin", "vbaProjectSignatureAgile", &[0xBE, 0xEF, 0xFF][..]),
            ("vbaProjectSignatureV3.bin", "vbaProjectSignatureV3", &[0x03, 0x00, 0x0D, 0x0A, 0x1A][..]),
        ]
    );
    assert_eq!(wb.vba.workbook_code_name.as_deref(), Some("ThisWorkbook"));
    assert_eq!(wb.sheets[0].code_name.as_deref(), Some("Sheet1"));
    assert_eq!(wb.sheets[1].code_name.as_deref(), Some("Feuil2"));
    // Opening a file with macros is not a change to save.
    assert!(!wb.modified);

    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    assert_eq!(bytes_of(&saved, "xl/vbaProject.bin"), project);
    assert_eq!(bytes_of(&saved, "xl/vbaProjectSignature.bin"), [0xDE, 0xAD, 0x00, 0x01]);
    assert_eq!(bytes_of(&saved, "xl/vbaProjectSignatureAgile.bin"), [0xBE, 0xEF, 0xFF]);
    assert_eq!(bytes_of(&saved, "xl/vbaProjectSignatureV3.bin"), [0x03, 0x00, 0x0D, 0x0A, 0x1A]);
    let types = part(&saved, "[Content_Types].xml");
    assert!(types.contains("<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.ms-excel.sheet.macroEnabled.main+xml\"/>"));
    assert!(types.contains("<Override PartName=\"/xl/vbaProject.bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/>"));
    assert!(types.contains("<Override PartName=\"/xl/vbaProjectSignature.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignature\"/>"));
    assert!(types.contains("<Override PartName=\"/xl/vbaProjectSignatureAgile.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignatureAgile\"/>"));
    assert!(types.contains("<Override PartName=\"/xl/vbaProjectSignatureV3.bin\" ContentType=\"application/vnd.ms-office.vbaProjectSignatureV3\"/>"));
    let rels = part(&saved, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains(&format!("Type=\"{MS_REL}/vbaProject\" Target=\"vbaProject.bin\"")), "{rels}");
    let project_rels = part(&saved, "xl/_rels/vbaProject.bin.rels");
    for kind in ["vbaProjectSignature", "vbaProjectSignatureAgile", "vbaProjectSignatureV3"] {
        assert!(project_rels.contains(&format!("Type=\"{MS_REL}/{kind}\" Target=\"{kind}.bin\"")), "{project_rels}");
    }
    assert!(part(&saved, "xl/workbook.xml").contains("<workbookPr codeName=\"ThisWorkbook\"/>"));
    assert!(part(&saved, "xl/worksheets/sheet1.xml").contains("<sheetPr codeName=\"Sheet1\"><tabColor"));
    assert!(part(&saved, "xl/worksheets/sheet2.xml").contains("<sheetPr codeName=\"Feuil2\"></sheetPr>"));
    common::assert_well_formed("sheet1", &part(&saved, "xl/worksheets/sheet1.xml"));

    // And once more: what was written is read the same.
    let again = load(&saved);
    assert_eq!(again.vba, wb.vba);
    assert_eq!(save_xlsm(&again), saved);
    // The modules are still readable in what was saved.
    let read = read_project(&bytes_of(&saved, "xl/vbaProject.bin")).expect("a project");
    assert_eq!(read.modules.len(), 4);
    assert_eq!(read.modules[3].code(), MODULE);
}

#[test]
fn a_macro_free_file_has_nothing_of_the_project() {
    let wb = load(&macro_workbook(&project()));
    let saved = save_xlsx(&wb);
    assert_consistent(&saved);
    let names = part_names(&saved);
    assert!(names.iter().all(|name| !name.to_lowercase().contains("vba")), "{names:?}");
    let types = part(&saved, "[Content_Types].xml");
    assert!(types.contains("ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\""));
    assert!(!types.contains("macroEnabled") && !types.contains("vbaProject"));
    assert!(!part(&saved, "xl/_rels/workbook.xml.rels").contains("vbaProject"));
    // The code names stay, as in a macro-free file Excel writes.
    assert!(part(&saved, "xl/workbook.xml").contains("codeName=\"ThisWorkbook\""));
    // The workbook in memory still has its project: only the file lost it.
    assert!(wb.has_vba_project());
    let reopened = load(&saved);
    assert!(!reopened.has_vba_project());
    assert_eq!(reopened.sheets[0].code_name.as_deref(), Some("Sheet1"));
}

#[test]
fn code_names_follow_renames_moves_copies_and_deletions() {
    let mut wb = load(&macro_workbook(&project()));
    assert!(wb.rename_sheet(0, "Sales 2026"));
    wb.move_sheet(0, 1);
    let copy = wb.duplicate_sheet(1).expect("a copy");
    let added = wb.add_sheet(0);
    wb.commit("sheets");
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    let back = load(&saved);
    let names: Vec<(&str, Option<&str>)> = back.sheets.iter().map(|s| (s.name.as_str(), s.code_name.as_deref())).collect();
    assert_eq!(added, 0);
    assert_eq!(copy, 2);
    assert_eq!(
        names,
        [("Sheet4", None), ("Notes", Some("Feuil2")), ("Sales 2026", Some("Sheet1")), ("Sales 2026 (2)", None)]
    );
    // A sheet without a code name has no attribute at all.
    assert!(!part(&saved, "xl/worksheets/sheet1.xml").contains("codeName"));
    assert!(!part(&saved, "xl/worksheets/sheet4.xml").contains("codeName"));
    // No two sheets share one.
    let mut codes: Vec<&str> = back.sheets.iter().filter_map(|s| s.code_name.as_deref()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 2);

    // The sheet that goes takes its code name; the project stays whole.
    assert!(wb.delete_sheet(2));
    wb.commit("delete");
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    let back = load(&saved);
    assert!(back.sheets.iter().all(|s| s.code_name.as_deref() != Some("Sheet1")));
    assert_eq!(bytes_of(&saved, "xl/vbaProject.bin"), project());
    // Undo brings the sheet back with its code name.
    assert!(wb.undo());
    assert_eq!(wb.sheets[2].code_name.as_deref(), Some("Sheet1"));
}

#[test]
fn templates_and_files_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let wb = load(&macro_workbook(&project()));
    let content_type = |name: &str| -> (String, bool) {
        let path = dir.path().join(name);
        opensheet_io::save(&wb, 0, &path).expect("saved");
        let bytes = std::fs::read(&path).unwrap();
        assert_consistent(&bytes);
        let types = part(&bytes, "[Content_Types].xml");
        let start = types.find("PartName=\"/xl/workbook.xml\" ContentType=\"").unwrap() + 41;
        let kind = types[start..].split('"').next().unwrap().to_string();
        // The file opens again, with or without its project.
        let back = opensheet_io::load(&path, Locale::en()).expect("loaded");
        (kind, back.has_vba_project())
    };
    assert_eq!(content_type("a.xlsm"), ("application/vnd.ms-excel.sheet.macroEnabled.main+xml".to_string(), true));
    assert_eq!(content_type("a.xltm"), ("application/vnd.ms-excel.template.macroEnabled.main+xml".to_string(), true));
    assert_eq!(
        content_type("a.xlsx"),
        ("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml".to_string(), false)
    );
    assert_eq!(
        content_type("a.xltx"),
        ("application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml".to_string(), false)
    );
}

#[test]
fn a_workbook_without_macros_saved_as_xlsm_is_a_plain_macro_enabled_file() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "1");
    wb.commit("x");
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    assert!(part(&saved, "[Content_Types].xml").contains("macroEnabled.main+xml"));
    assert!(part_names(&saved).iter().all(|name| !name.contains("vbaProject")));
    assert!(part(&saved, "xl/workbook.xml").contains("<workbookPr/>"));
    assert!(!part(&saved, "xl/worksheets/sheet1.xml").contains("sheetPr"));
}

/// What macro-enabled files may hold beside the project and OpenSheet
/// has no model for: ActiveX controls, a dialog sheet, a shape of the
/// legacy drawing that does not say what it is. They are not read, so
/// they are not written: the file saved has no trace of them, and nothing
/// pointing at them. (Form controls are kept: see `xlsx_controls.rs`.)
#[test]
fn what_is_not_read_is_dropped_without_leaving_loose_ends() {
    let types = format!(
        "<?xml version=\"1.0\"?><Types xmlns=\"{CT}\"><Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.ms-excel.sheet.macroEnabled.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/dialogsheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.dialogsheet+xml\"/><Override PartName=\"/xl/ctrlProps/ctrlProp1.xml\" ContentType=\"application/vnd.ms-excel.controlproperties+xml\"/><Override PartName=\"/xl/activeX/activeX1.xml\" ContentType=\"application/vnd.ms-office.activeX+xml\"/></Types>"
    );
    let root = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let workbook = format!(
        "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><workbookPr codeName=\"ThisWorkbook\"/><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Dialog1\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{DOC_REL}/dialogsheet\" Target=\"dialogsheets/sheet1.xml\"/><Relationship Id=\"rId3\" Type=\"{MS_REL}/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>"
    );
    let sheet = format!(
        "<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><sheetPr codeName=\"Sheet1\"/><sheetData><row r=\"1\"><c r=\"A1\"><v>7</v></c></row></sheetData><legacyDrawing r:id=\"rId1\"/><controls><control shapeId=\"1025\" r:id=\"rId2\" name=\"Button 1\"/><control shapeId=\"1026\" r:id=\"rId3\" name=\"CommandButton1\"/></controls></worksheet>"
    );
    let sheet_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/><Relationship Id=\"rId2\" Type=\"{DOC_REL}/ctrlProp\" Target=\"../ctrlProps/ctrlProp1.xml\"/><Relationship Id=\"rId3\" Type=\"{DOC_REL}/control\" Target=\"../activeX/activeX1.xml\"/></Relationships>"
    );
    let project = project();
    let original = zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/dialogsheets/sheet1.xml", b"<dialogsheet/>"),
        ("xl/drawings/vmlDrawing1.vml", b"<xml xmlns:v=\"urn:schemas-microsoft-com:vml\"><v:shape id=\"_x0000_s1025\" type=\"#_x0000_t201\"/></xml>"),
        ("xl/ctrlProps/ctrlProp1.xml", b"<formControlPr objectType=\"Button\"/>"),
        ("xl/activeX/activeX1.xml", b"<ax:ocx xmlns:ax=\"http://schemas.microsoft.com/office/2006/activeX\"/>"),
        ("xl/vbaProject.bin", &project),
    ]);
    assert_consistent(&original);
    let wb = load(&original);
    // The dialog sheet is not a sheet of cells: it is not loaded.
    assert_eq!(wb.sheets.len(), 1);
    assert!(wb.has_vba_project());
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    let names = part_names(&saved);
    for gone in ["ctrlProps", "activeX", "dialogsheets", "vmlDrawing"] {
        assert!(names.iter().all(|name| !name.contains(gone)), "{gone} in {names:?}");
    }
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("<controls") && !sheet.contains("legacyDrawing"), "{sheet}");
    assert_eq!(bytes_of(&saved, "xl/vbaProject.bin"), project);
    assert_eq!(common::value(&load(&saved), 0, "A1"), opensheet_engine::Value::Number(7.0));
}

/// The sample to open in Excel: a workbook with data and a project whose
/// macros work on it.
#[test]
fn sample_with_macros() {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [
        ("A1", "Region"),
        ("B1", "Units"),
        ("C1", "Revenue"),
        ("A2", "North"),
        ("B2", "120"),
        ("C2", "2400"),
        ("A3", "South"),
        ("B3", "85"),
        ("C3", "1700"),
        ("A4", "West"),
        ("B4", "140"),
        ("C4", "2800"),
        ("A6", "Total"),
        ("B6", "=SUM(B2:B4)"),
        ("C6", "=SUM(C2:C4)"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    wb.add_sheet(1);
    wb.rename_sheet(0, "Report");
    wb.rename_sheet(1, "Notes");
    enter(&mut wb, 1, "A1", "Open the Macros dialog (View > Macros): FormatReport, ClearReport, Sheet1.Recalculate");
    wb.commit("sample");
    wb.vba.workbook_code_name = Some("ThisWorkbook".to_string());
    wb.sheets[0].code_name = Some("Sheet1".to_string());
    wb.sheets[1].code_name = Some("Sheet2".to_string());
    let project = build_project(&[
        ("ThisWorkbook", ModuleKind::Document, ""),
        ("Sheet1", ModuleKind::Document, "Sub Recalculate()\n    Me.Calculate\nEnd Sub\n"),
        ("Sheet2", ModuleKind::Document, ""),
        ("Module1", ModuleKind::Standard, MODULE),
    ]);
    wb.vba.project = Some(opensheet_engine::vba::VbaProject {
        bin: project.clone().into(),
        signatures: Vec::new(),
    });
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    keep_sample("macros.xlsm", &saved);
    let back = load(&saved);
    assert_eq!(back.vba, wb.vba);
    let read = read_project(&back.vba.project.as_ref().unwrap().bin).expect("a project");
    let macros: Vec<String> = read.macros().into_iter().map(|m| m.name).collect();
    assert_eq!(macros, ["ClearReport", "FormatReport", "Sheet1.Recalculate"]);
    assert_eq!(common::value(&back, 0, "C6"), opensheet_engine::Value::Number(6900.0));
}

/// A project Excel itself wrote (its Solver add-in), when Excel is on
/// this machine: it goes through OpenSheet unchanged.
#[test]
fn a_project_written_by_excel_goes_through() {
    let path = std::path::Path::new("/Applications/Microsoft Excel.app/Contents/Resources/en.lproj/Add-Ins/Solver.xlam");
    let Ok(original) = std::fs::read(path) else {
        return;
    };
    let project = bytes_of(&original, "xl/vbaProject.bin");
    let wb = load(&original);
    assert_eq!(wb.vba.project.as_ref().map(|p| &*p.bin), Some(project.as_slice()));
    assert_eq!(wb.vba.workbook_code_name.as_deref(), Some("ThisWorkbook"));
    assert!(wb.sheets.iter().all(|s| s.code_name.is_some()));
    let saved = save_xlsm(&wb);
    assert_consistent(&saved);
    assert_eq!(bytes_of(&saved, "xl/vbaProject.bin"), project);
    let read = read_project(&project).expect("Excel's project");
    assert!(!read.macros().is_empty() || !read.modules.is_empty());
}
