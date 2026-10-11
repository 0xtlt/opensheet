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
