//! Delimited text files: loading, saving and round trips in both locales.

mod common;

use common::{enter, value};
use opensheet_engine::datetime::date_to_serial;
use opensheet_engine::workbook::Content;
use opensheet_engine::{ErrorKind, Locale, Value, Workbook};
use opensheet_io::{IoError, load, load_csv, save, save_csv, save_delimited, sheet_to_text};
use std::path::Path;

const BOM: &str = "\u{feff}";

fn read(path: &Path) -> String {
    String::from_utf8(std::fs::read(path).unwrap()).unwrap()
}

/// Every cell showing something, with its value.
fn cells(wb: &Workbook) -> Vec<((u32, u32), Value)> {
    wb.sheets[0]
        .cells
        .iter()
        .filter(|(_, c)| !matches!(c.content, Content::Empty))
        .map(|(pos, c)| (*pos, c.value.clone()))
        .collect()
}

/// A sheet whose displayed text reads back as the same values. Cells are
/// entered as text in the workbook's locale, so their display and their
/// re-interpretation use the same conventions.
fn sample(locale: &'static Locale, entries: &[(&str, &str)]) -> Workbook {
    let mut wb = Workbook::new(locale);
    for (cell, text) in entries {
        enter(&mut wb, 0, cell, text);
    }
    // Text that needs quoting, in any locale.
    wb.set_value(0, 5, 0, Value::text("semi;colon, comma"));
    wb.set_value(0, 5, 1, Value::text("say \"hi\""));
    wb.set_value(0, 5, 2, Value::text("two\nlines"));
    wb.set_value(0, 5, 3, Value::text("=not a formula"));
    wb.set_value(0, 6, 0, Value::text("  padded  "));
    wb.set_value(0, 6, 3, Value::text("çà et là — €"));
    wb.commit("setup");
    wb
}

#[test]
fn english_round_trip() {
    let en = Locale::en();
    let wb = sample(
        en,
        &[
            ("A1", "Item"),
            ("B1", "Price"),
            ("C1", "Qty"),
            ("D1", "Total"),
            ("A2", "Widget, large"),
            ("B2", "1234.5"),
            ("C2", "3"),
            ("D2", "=B2*C2"),
            ("A3", "Gadget"),
            ("B3", "-0.25"),
            ("C3", "TRUE"),
            ("D3", "=1/0"),
            ("A4", "3/12/2025"),
            ("B4", "50%"),
            // Row 5 is left empty, column E too.
            ("F2", "end"),
        ],
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Price list.csv");
    save_csv(&wb, 0, &path).unwrap();

    let text = read(&path);
    assert!(
        text.starts_with(BOM),
        "CSV UTF-8 starts with a byte order mark"
    );
    let lines: Vec<&str> = text.trim_start_matches(BOM).split("\r\n").collect();
    assert_eq!(lines[0], "Item,Price,Qty,Total,,");
    assert_eq!(lines[1], "\"Widget, large\",1234.5,3,3703.5,,end");
    assert!(lines[2].starts_with("Gadget,-0.25,TRUE,#DIV/0!,"));
    assert_eq!(lines[4], ",,,,,", "an empty row keeps its place");
    assert_eq!(
        lines[5],
        "\"semi;colon, comma\",\"say \"\"hi\"\"\",\"two\nlines\",=not a formula,,"
    );
    assert_eq!(lines[6], "  padded  ,,,çà et là — €,,");
    assert_eq!(lines.len(), 8, "seven rows, each ended by CR LF");
    assert_eq!(lines[7], "");

    let loaded = load_csv(&path, en).unwrap();
    assert!(!loaded.modified && !loaded.can_undo());
    assert_eq!(loaded.sheets.len(), 1);
    assert_eq!(
        loaded.sheets[0].name, "Price list",
        "the sheet is named after the file"
    );
    // Same cells, same values: formulas have become their results.
    assert_eq!(cells(&loaded), cells(&wb));
    assert_eq!(value(&loaded, 0, "B2"), Value::Number(1234.5));
    assert_eq!(value(&loaded, 0, "D2"), Value::Number(3703.5));
    assert_eq!(value(&loaded, 0, "C3"), Value::Bool(true));
    assert_eq!(value(&loaded, 0, "D3"), Value::Error(ErrorKind::Div0));
    assert_eq!(
        value(&loaded, 0, "A4"),
        Value::Number(date_to_serial(2025, 3, 12).unwrap())
    );
    assert_eq!(value(&loaded, 0, "B4"), Value::Number(0.5));
    assert_eq!(value(&loaded, 0, "C6"), Value::text("two\nlines"));
    assert_eq!(value(&loaded, 0, "D6"), Value::text("=not a formula"));
    assert!(
        loaded.sheets[0]
            .cells
            .values()
            .all(|c| !matches!(c.content, Content::Formula(_)))
    );
    // Saving what was loaded gives the same file.
    assert_eq!(
        sheet_to_text(&loaded, 0, ',').unwrap(),
        text.trim_start_matches(BOM)
    );
}

#[test]
fn french_round_trip() {
    let fr = Locale::fr();
    let wb = sample(
        fr,
        &[
            ("A1", "Article"),
            ("B1", "Prix"),
            ("A2", "Vis, écrou"),
            ("B2", "1234,5"),
            ("C2", "3"),
            ("D2", "=B2*C2"),
            ("A3", "Boulon"),
            ("B3", "-0,25"),
            ("C3", "VRAI"),
            ("D3", "=SOMME(B2:B3)"),
            ("A4", "12/03/2025"),
            ("B4", "0,125"),
            ("C4", "FAUX"),
        ],
    );
    assert_eq!(value(&wb, 0, "B2"), Value::Number(1234.5));
    assert_eq!(
        value(&wb, 0, "A4"),
        Value::Number(date_to_serial(2025, 3, 12).unwrap())
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tarifs.csv");
    save_csv(&wb, 0, &path).unwrap();

    let text = read(&path);
    let lines: Vec<&str> = text.trim_start_matches(BOM).split("\r\n").collect();
    assert_eq!(
        lines[0], "Article;Prix;;",
        "semicolons where the comma is the decimal separator"
    );
    assert_eq!(
        lines[1], "Vis, écrou;1234,5;3;3703,5",
        "a comma needs no quotes here"
    );
    assert!(lines[2].starts_with("Boulon;-0,25;VRAI;1234,25"));
    assert!(lines[3].ends_with(";0,125;FAUX;"));
    assert_eq!(
        lines[5],
        "\"semi;colon, comma\";\"say \"\"hi\"\"\";\"two\nlines\";=not a formula"
    );

    // The delimiter is detected, numbers are read with a decimal comma.
    let loaded = load_csv(&path, fr).unwrap();
    assert_eq!(loaded.locale.id, "fr");
    assert_eq!(cells(&loaded), cells(&wb));
    assert_eq!(value(&loaded, 0, "A2"), Value::text("Vis, écrou"));
    assert_eq!(value(&loaded, 0, "B2"), Value::Number(1234.5));
    assert_eq!(value(&loaded, 0, "D2"), Value::Number(3703.5));
    assert_eq!(value(&loaded, 0, "C3"), Value::Bool(true));
    assert_eq!(value(&loaded, 0, "C4"), Value::Bool(false));
    assert_eq!(
        value(&loaded, 0, "A4"),
        Value::Number(date_to_serial(2025, 3, 12).unwrap())
    );
    assert_eq!(
        sheet_to_text(&loaded, 0, ';').unwrap(),
        text.trim_start_matches(BOM)
    );
}

#[test]
fn loading_interprets_fields_like_typed_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.csv");
    std::fs::write(
        &path,
        "name,amount,when,flag\r\n\"Smith, John\",\"1,234.50\",3/12/2025,true\r\n'007,+A1,-SUM(B:B),=1+1\r\n\r\nlast,,,x,\r\n",
    )
    .unwrap();
    let wb = load_csv(&path, Locale::en()).unwrap();
    assert_eq!(value(&wb, 0, "A2"), Value::text("Smith, John"));
    assert_eq!(value(&wb, 0, "B2"), Value::Number(1234.5));
    assert_eq!(
        value(&wb, 0, "C2"),
        Value::Number(date_to_serial(2025, 3, 12).unwrap())
    );
    assert_eq!(
        wb.style_at(0, 1, 2).num_fmt.as_ref(),
        "m/d/yyyy",
        "dates get a date format"
    );
    assert_eq!(value(&wb, 0, "D2"), Value::Bool(true));
    // Nothing in a text file becomes a formula, and an apostrophe is data.
    assert_eq!(value(&wb, 0, "A3"), Value::text("'007"));
    assert_eq!(value(&wb, 0, "B3"), Value::text("+A1"));
    assert_eq!(value(&wb, 0, "C3"), Value::text("-SUM(B:B)"));
    assert_eq!(value(&wb, 0, "D3"), Value::text("=1+1"));
    assert!(
        wb.sheets[0]
            .cells
            .values()
            .all(|c| matches!(c.content, Content::Value(_)))
    );
    // The blank line keeps row 4 empty; trailing delimiters add nothing.
    assert_eq!(value(&wb, 0, "A4"), Value::Empty);
    assert_eq!(value(&wb, 0, "A5"), Value::text("last"));
    assert_eq!(value(&wb, 0, "D5"), Value::text("x"));
    assert_eq!(wb.sheets[0].cells.len(), 14);
    assert!(!wb.modified && !wb.can_undo());
}

#[test]
fn encodings_and_delimiters() {
    let dir = tempfile::tempdir().unwrap();
    let fr = Locale::fr();

    // Windows-1252, as saved by older versions of Excel.
    let legacy = dir.path().join("legacy.csv");
    std::fs::write(&legacy, b"Pr\xE9nom;Prix \x80;c\x9Cur\r\nZo\xEB;12,5;3\r\n").unwrap();
    let wb = load_csv(&legacy, fr).unwrap();
    assert_eq!(value(&wb, 0, "A1"), Value::text("Prénom"));
    assert_eq!(value(&wb, 0, "B1"), Value::text("Prix €"));
    assert_eq!(value(&wb, 0, "C1"), Value::text("cœur"));
    assert_eq!(value(&wb, 0, "A2"), Value::text("Zoë"));
    assert_eq!(value(&wb, 0, "B2"), Value::Number(12.5));

    // UTF-8 with a byte order mark; the mark is not part of the first field.
    let utf8 = dir.path().join("utf8.csv");
    std::fs::write(&utf8, "\u{feff}5;é\n").unwrap();
    let wb = load_csv(&utf8, fr).unwrap();
    assert_eq!(value(&wb, 0, "A1"), Value::Number(5.0));
    assert_eq!(value(&wb, 0, "B1"), Value::text("é"));

    // "Unicode text": UTF-16 little endian, tab-delimited.
    let utf16 = dir.path().join("unicode.txt");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend("a\tb\n1,5\t2\n".encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&utf16, bytes).unwrap();
    let wb = load(&utf16, fr).unwrap();
    assert_eq!(value(&wb, 0, "B1"), Value::text("b"));
    assert_eq!(value(&wb, 0, "A2"), Value::Number(1.5));

    // A .tsv file is tab-delimited whatever it contains.
    let tsv = dir.path().join("table.tsv");
    std::fs::write(&tsv, "a,b;c\td\n").unwrap();
    let wb = load(&tsv, Locale::en()).unwrap();
    assert_eq!(value(&wb, 0, "A1"), Value::text("a,b;c"));
    assert_eq!(value(&wb, 0, "B1"), Value::text("d"));

    // A comma-separated file opened in a French locale is still split.
    let english = dir.path().join("english.csv");
    std::fs::write(&english, "name,city\nAda,London\nAlan,Wilmslow\n").unwrap();
    let wb = load(&english, fr).unwrap();
    assert_eq!(value(&wb, 0, "B3"), Value::text("Wilmslow"));
}

#[test]
fn saving_by_extension_and_edge_cases() {
    let en = Locale::en();
    let dir = tempfile::tempdir().unwrap();
    let mut wb = Workbook::new(en);
    enter(&mut wb, 0, "B2", "tab\there");
    enter(&mut wb, 0, "C2", "1.5");
    wb.add_sheet(1);
    enter(&mut wb, 1, "A1", "second");
    // A dynamic array: its spilled values are part of what is displayed.
    enter(&mut wb, 0, "A4", "=C2*{1,2,3}");
    wb.commit("setup");

    // .tsv and .txt are tab-delimited; leading empty rows and columns stay.
    for name in ["out.tsv", "out.txt"] {
        let path = dir.path().join(name);
        save(&wb, 0, &path).unwrap();
        assert_eq!(
            read(&path),
            format!("{BOM}\t\t\r\n\t\"tab\there\"\t1.5\r\n\t\t\r\n1.5\t3\t4.5\r\n")
        );
        let back = load(&path, en).unwrap();
        assert_eq!(value(&back, 0, "B2"), Value::text("tab\there"));
        assert_eq!(value(&back, 0, "C4"), Value::Number(4.5));
    }
    // The sheet index selects what is written.
    let second = dir.path().join("second.csv");
    save(&wb, 1, &second).unwrap();
    assert_eq!(read(&second), format!("{BOM}second\r\n"));
    save_delimited(&wb, 1, &second, '|').unwrap();
    assert_eq!(read(&second), format!("{BOM}second\r\n"));
    assert!(matches!(
        save_csv(&wb, 7, &second),
        Err(IoError::NoSuchSheet(7))
    ));
    assert_eq!(
        read(&second),
        format!("{BOM}second\r\n"),
        "a failed save leaves the file alone"
    );

    // An empty sheet gives an empty file, which loads as an empty workbook.
    let empty = dir.path().join("empty.csv");
    save_csv(&Workbook::new(en), 0, &empty).unwrap();
    assert_eq!(read(&empty), BOM);
    let back = load_csv(&empty, en).unwrap();
    assert!(back.sheets[0].cells.is_empty());
    assert_eq!(back.sheets[0].name, "empty");

    assert!(matches!(
        load_csv(&dir.path().join("missing.csv"), en),
        Err(IoError::Io(_))
    ));
}
