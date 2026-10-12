//! Helpers shared by the integration tests.
#![allow(dead_code)]

use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Locale, Value, Workbook};
use std::io::{Cursor, Read, Write};

/// 0-based (row, col) of an A1-style name.
pub fn at(name: &str) -> (u32, u32) {
    parse_cell_name(name).unwrap_or_else(|| panic!("bad cell name {name}"))
}

/// Type `text` into a cell, as a user would.
pub fn enter(wb: &mut Workbook, sheet: usize, name: &str, text: &str) {
    let (r, c) = at(name);
    wb.set_cell_text(sheet, r, c, text)
        .unwrap_or_else(|e| panic!("{name}: cannot enter {text:?}: {e:?}"));
}

pub fn value(wb: &Workbook, sheet: usize, name: &str) -> Value {
    let (r, c) = at(name);
    wb.value_at(sheet, r, c)
}

/// Formula of a cell in canonical English syntax, without the `=`.
pub fn formula(wb: &Workbook, sheet: usize, name: &str) -> Option<String> {
    let (r, c) = at(name);
    match &wb.cell(sheet, r, c)?.content {
        Content::Formula(e) => Some(e.to_text(Locale::en())),
        _ => None,
    }
}

/// Build a zip archive from (name, content) pairs.
pub fn package(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Keep a file the tests made, to open it in Excel: when the environment
/// variable `OPENSHEET_SAMPLES` names a directory, `bytes` are written
/// there as `name`. (`OPENSHEET_SAMPLES=/tmp/x cargo test -p opensheet_io`)
pub fn keep_sample(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_SAMPLES") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

/// The text of one part of a zip archive.
pub fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    let mut file = zip
        .by_name(name)
        .unwrap_or_else(|_| panic!("missing part {name}"));
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

pub fn part_names(bytes: &[u8]) -> Vec<String> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    zip.file_names().map(|n| n.unwrap().into_owned()).collect()
}

/// Panic unless `xml` is a well-formed document with a single root element
/// (checked with an independent parser).
pub fn assert_well_formed(name: &str, xml: &str) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut depth = 0usize;
    let mut roots = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                for attr in e.attributes() {
                    attr.unwrap_or_else(|err| panic!("{name}: bad attribute: {err}"));
                }
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            Ok(Event::Empty(e)) => {
                for attr in e.attributes() {
                    attr.unwrap_or_else(|err| panic!("{name}: bad attribute: {err}"));
                }
                if depth == 0 {
                    roots += 1;
                }
            }
            Ok(Event::End(_)) => {
                assert!(depth > 0, "{name}: unbalanced end tag");
                depth -= 1;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => panic!("{name} is not well-formed: {err}"),
        }
    }
    assert_eq!(depth, 0, "{name}: unclosed element");
    assert_eq!(roots, 1, "{name}: expected exactly one root element");
    assert!(xml.starts_with("<?xml"), "{name}: missing XML declaration");
}

/// The children of a worksheet, in the only order the schema
/// (`CT_Worksheet`) and Excel accept.
pub const WORKSHEET_CHILDREN: [&str; 39] = [
    "sheetPr",
    "dimension",
    "sheetViews",
    "sheetFormatPr",
    "cols",
    "sheetData",
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

/// The children of the workbook element, in the order of `CT_Workbook`.
pub const WORKBOOK_CHILDREN: [&str; 19] = [
    "fileVersion",
    "fileSharing",
    "workbookPr",
    "workbookProtection",
    "bookViews",
    "sheets",
    "functionGroups",
    "externalReferences",
    "definedNames",
    "calcPr",
    "oleSize",
    "customWorkbookViews",
    "pivotCaches",
    "smartTagPr",
    "smartTagTypes",
    "webPublishing",
    "fileRecoveryPr",
    "webPublishObjects",
    "extLst",
];

/// Names of the elements directly under the root of `xml`.
pub fn children(xml: &str) -> Vec<String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut depth = 0usize;
    let mut out = Vec::new();
    let name =
        |e: &quick_xml::events::BytesStart| String::from_utf8_lossy(e.name().as_ref()).into_owned();
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) => {
                if depth == 1 {
                    out.push(name(&e));
                }
                depth += 1;
            }
            Event::Empty(e) if depth == 1 => out.push(name(&e)),
            Event::End(_) => depth -= 1,
            Event::Eof => return out,
            _ => {}
        }
    }
}

/// Panic unless the children of the root of `xml` are all in `order`, in
/// that order (only `repeatable` ones more than once).
pub fn assert_child_order(part: &str, xml: &str, order: &[&str], repeatable: &[&str]) {
    let mut last = None;
    for child in children(xml) {
        let position = order
            .iter()
            .position(|known| *known == child)
            .unwrap_or_else(|| panic!("{part}: unexpected element <{child}>"));
        match last {
            Some(previous) if position < previous => {
                panic!("{part}: <{child}> comes after <{}>", order[previous])
            }
            Some(previous) if position == previous => {
                assert!(
                    repeatable.contains(&child.as_str()),
                    "{part}: two <{child}>"
                )
            }
            _ => {}
        }
        last = Some(position);
    }
}

/// Panic unless a worksheet part has its elements in schema order.
pub fn assert_worksheet_order(part: &str, xml: &str) {
    assert_child_order(part, xml, &WORKSHEET_CHILDREN, &["conditionalFormatting"]);
}

/// Panic unless the workbook part has its elements in schema order.
pub fn assert_workbook_order(xml: &str) {
    assert_child_order("xl/workbook.xml", xml, &WORKBOOK_CHILDREN, &[]);
}
