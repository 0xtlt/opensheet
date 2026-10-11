//! Delimited text files (`.csv`, `.tsv`, `.txt`).
//!
//! Loading interprets every field the way typing it into a cell would (in
//! the workbook's locale), except that formulas are never created from text
//! files. Saving writes what the cells display.

use crate::{IoError, Result, decode_utf16, extension, write_file};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::value::Value;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Locale, Workbook};
use std::path::Path;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// The CSV delimiter Excel uses in a locale: `;` where the comma is the
/// decimal separator, `,` elsewhere.
pub fn default_delimiter(locale: &Locale) -> char {
    if locale.decimal_sep == ',' { ';' } else { ',' }
}

/// Windows-1252 differs from Latin-1 in 0x80..=0x9F.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

/// Decode the bytes of a text file: UTF-8 (with or without BOM), UTF-16
/// with BOM, or Windows-1252 when the bytes are not valid UTF-8.
pub fn decode_text(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => return decode_utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => return decode_utf16(rest, false),
        _ => {}
    }
    let bytes = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x80..=0x9F => CP1252_HIGH[(b - 0x80) as usize],
                _ => b as char,
            })
            .collect(),
    }
}

/// Guess the delimiter (tab, `;` or `,`) from the first lines of a file.
///
/// The winner is the candidate that occurs the same number of times on the
/// most lines, quoted sections aside. On a tie, tab beats `;` beats `,`:
/// decimal commas are common in semicolon-separated files, the reverse is
/// not. `locale` settles the remaining ambiguity of a single column of
/// decimal-comma numbers.
pub fn detect_delimiter(text: &str, locale: &Locale) -> char {
    const CANDIDATES: [char; 3] = ['\t', ';', ','];
    const SAMPLE_LINES: usize = 64;

    let mut lines: Vec<[usize; 3]> = Vec::new();
    let mut counts = [0usize; 3];
    let mut has_content = false;
    let mut in_quotes = false;
    // True while every comma seen sits between two digits ("1,5").
    let mut commas_look_decimal = true;
    let mut prev = '\n';
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            in_quotes = !in_quotes;
            has_content = true;
        } else if in_quotes {
            // Delimiters and line breaks inside quotes are data.
        } else if ch == '\n' || ch == '\r' {
            if has_content {
                lines.push(counts);
                has_content = false;
                if lines.len() >= SAMPLE_LINES {
                    break;
                }
            }
            counts = [0; 3];
        } else {
            has_content = true;
            if let Some(i) = CANDIDATES.iter().position(|d| *d == ch) {
                counts[i] += 1;
                let between_digits =
                    prev.is_ascii_digit() && chars.peek().is_some_and(|n| n.is_ascii_digit());
                if ch == ',' && !between_digits {
                    commas_look_decimal = false;
                }
            }
        }
        prev = ch;
    }
    if has_content {
        lines.push(counts);
    }

    // Number of lines sharing the most common non-zero count.
    let score = |i: usize| {
        lines
            .iter()
            .filter(|line| line[i] > 0)
            .map(|line| lines.iter().filter(|other| other[i] == line[i]).count())
            .max()
            .unwrap_or(0)
    };
    let mut best: Option<(usize, usize)> = None;
    for i in 0..CANDIDATES.len() {
        let s = score(i);
        if s > 0 && best.is_none_or(|(_, top)| s > top) {
            best = Some((i, s));
        }
    }
    let Some((winner, _)) = best else {
        return default_delimiter(locale);
    };
    let one_comma_at_most = lines.iter().all(|line| line[2] <= 1);
    if CANDIDATES[winner] == ','
        && locale.decimal_sep == ','
        && commas_look_decimal
        && one_comma_at_most
    {
        // A single column of numbers such as "1,5".
        return ';';
    }
    CANDIDATES[winner]
}

/// Split delimited text into fields (RFC 4180 quoting), calling `emit` with
/// (row, column, field) for every field in file order.
fn scan(text: &str, delimiter: char, mut emit: impl FnMut(usize, usize, &str)) {
    let mut field = String::new();
    let (mut row, mut col) = (0usize, 0usize);
    let mut in_quotes = false;
    // The current field started with a quote.
    let mut quoted = false;
    // The current record has at least one field to emit.
    let mut pending = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch != '"' {
                field.push(ch);
            } else if chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else {
                in_quotes = false;
            }
        } else if ch == '"' && field.is_empty() && !quoted {
            in_quotes = true;
            quoted = true;
            pending = true;
        } else if ch == delimiter {
            emit(row, col, &field);
            field.clear();
            quoted = false;
            col += 1;
            pending = true;
        } else if ch == '\n' || ch == '\r' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            emit(row, col, &field);
            field.clear();
            quoted = false;
            row += 1;
            col = 0;
            pending = false;
        } else {
            field.push(ch);
            pending = true;
        }
    }
    if pending {
        emit(row, col, &field);
    }
}

/// Split delimited text into records of fields (RFC 4180 quoting). An empty
/// line yields a record with one empty field.
pub fn parse_delimited(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    scan(text, delimiter, |row, _, field| {
        if rows.len() <= row {
            rows.resize_with(row + 1, Vec::new);
        }
        rows[row].push(field.to_string());
    });
    rows
}

fn set_field(wb: &mut Workbook, row: u32, col: u32, field: &str) {
    // Formulas are not run from text files, and a leading apostrophe is
    // data here, not the "enter as text" marker of interactive entry.
    let literal = field.starts_with(['=', '\'']);
    if !literal && wb.set_cell_text(0, row, col, field).is_ok() {
        // "+A1" and "-SUM(B:B)" would have been taken for formulas.
        let formula = wb
            .cell(0, row, col)
            .is_some_and(|c| matches!(c.content, Content::Formula(_)));
        if !formula {
            return;
        }
    }
    wb.set_value(0, row, col, Value::text(field));
}

/// Build a one-sheet workbook from delimited text.
pub fn workbook_from_text(text: &str, delimiter: char, locale: &'static Locale) -> Workbook {
    let mut wb = Workbook::new(locale);
    let mut unflushed = 0usize;
    scan(text, delimiter, |row, col, field| {
        if field.is_empty() || row >= MAX_ROWS as usize || col >= MAX_COLS as usize {
            return;
        }
        set_field(&mut wb, row as u32, col as u32, field);
        // Loading is not undoable; do not let the undo log grow with it.
        unflushed += 1;
        if unflushed >= 4096 {
            wb.clear_history();
            unflushed = 0;
        }
    });
    wb.recalc();
    wb.clear_history();
    wb.modified = false;
    wb
}

/// A valid sheet name made from the file name, as Excel does for text files.
fn sheet_name_for(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    let name: String = stem
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | '?' | '*' | '[' | ']' | ':') {
                '_'
            } else {
                c
            }
        })
        .take(31)
        .collect();
    let name = name.trim().trim_matches('\'');
    (!name.is_empty()).then(|| name.to_string())
}

/// Load a delimited text file as a one-sheet workbook. The delimiter is
/// detected (always a tab for `.tsv`); numbers, dates and booleans are
/// recognised as typed in `locale`.
pub fn load_csv(path: &Path, locale: &'static Locale) -> Result<Workbook> {
    let text = decode_text(&std::fs::read(path)?);
    let delimiter = match extension(path).as_str() {
        "tsv" | "tab" => '\t',
        _ => detect_delimiter(&text, locale),
    };
    let mut wb = workbook_from_text(&text, delimiter, locale);
    if let (Some(name), Some(sheet)) = (sheet_name_for(path), wb.sheets.first_mut()) {
        sheet.name = name;
    }
    Ok(wb)
}

fn push_field(out: &mut String, field: &str, delimiter: char) {
    if !field.contains([delimiter, '"', '\n', '\r']) {
        out.push_str(field);
        return;
    }
    out.push('"');
    for c in field.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
}

/// The displayed text of a sheet as delimited text: every row from the
/// first to the last one with content, every column from A to the last one
/// with content, lines ending in CR LF.
pub fn sheet_to_text(wb: &Workbook, sheet: usize, delimiter: char) -> Result<String> {
    let sh = wb.sheets.get(sheet).ok_or(IoError::NoSuchSheet(sheet))?;
    let mut extent: Option<(u32, u32)> = None;
    let mut grow = |r: u32, c: u32| {
        extent = Some(match extent {
            Some((max_r, max_c)) => (max_r.max(r), max_c.max(c)),
            None => (r, c),
        });
    };
    for ((r, c), cell) in &sh.cells {
        if !matches!(cell.content, Content::Empty) {
            grow(*r, *c);
        }
    }
    for sp in &sh.spills {
        if sp.array.rows > 0 && sp.array.cols > 0 {
            grow(
                (sp.row as usize + sp.array.rows - 1).min(MAX_ROWS as usize - 1) as u32,
                (sp.col as usize + sp.array.cols - 1).min(MAX_COLS as usize - 1) as u32,
            );
        }
    }
    let mut out = String::new();
    let Some((max_row, max_col)) = extent else {
        return Ok(out);
    };
    for r in 0..=max_row {
        for c in 0..=max_col {
            if c > 0 {
                out.push(delimiter);
            }
            push_field(&mut out, &wb.display_text(sheet, r, c).text, delimiter);
        }
        out.push_str("\r\n");
    }
    Ok(out)
}

/// Save one sheet as delimited text (UTF-8 with BOM) using `delimiter`.
pub fn save_delimited(wb: &Workbook, sheet: usize, path: &Path, delimiter: char) -> Result<()> {
    let text = sheet_to_text(wb, sheet, delimiter)?;
    let mut bytes = Vec::with_capacity(text.len() + UTF8_BOM.len());
    bytes.extend_from_slice(&UTF8_BOM);
    bytes.extend_from_slice(text.as_bytes());
    write_file(path, &bytes)
}

/// Save one sheet as "CSV UTF-8": displayed values, `;` as delimiter when
/// the workbook's locale uses a decimal comma and `,` otherwise.
pub fn save_csv(wb: &Workbook, sheet: usize, path: &Path) -> Result<()> {
    save_delimited(wb, sheet, path, default_delimiter(wb.locale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        let rows = parse_delimited(
            "a,\"b,1\",\"say \"\"hi\"\"\"\r\n\"multi\nline\",,x\n\nlast",
            ',',
        );
        assert_eq!(
            rows,
            vec![
                vec!["a", "b,1", "say \"hi\""],
                vec!["multi\nline", "", "x"],
                vec![""],
                vec!["last"],
            ]
        );
        // A quote inside an unquoted field is literal; text after a closing
        // quote is kept.
        assert_eq!(
            parse_delimited("5\" disk,\"a\"b\n", ','),
            vec![vec!["5\" disk", "ab"]]
        );
        assert_eq!(parse_delimited("a;b\r", ';'), vec![vec!["a", "b"]]);
        assert_eq!(parse_delimited("a,\n", ','), vec![vec!["a", ""]]);
        assert_eq!(parse_delimited("\"\"", ','), vec![vec![""]]);
        assert!(parse_delimited("", ',').is_empty());
        let mut out = String::new();
        for field in ["plain", "a,b", "q\"q", "l\nl", " pad "] {
            push_field(&mut out, field, ',');
            out.push('|');
        }
        assert_eq!(out, "plain|\"a,b\"|\"q\"\"q\"|\"l\nl\"| pad |");
    }

    #[test]
    fn delimiters() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n", en), ',');
        assert_eq!(detect_delimiter("a;b;c\n1;2;3\n", en), ';');
        assert_eq!(detect_delimiter("a\tb\n1,5\t2,5\n", fr), '\t');
        // Decimal commas in a semicolon-separated file.
        assert_eq!(
            detect_delimiter("nom;montant\nDupont;1,5\nDurand;2,25\n", fr),
            ';'
        );
        assert_eq!(detect_delimiter("1,5;2,5\n3,5;4,5\n", fr), ';');
        assert_eq!(detect_delimiter("1,5;2,5\n3,5;4,5\n", en), ';');
        // Delimiters inside quotes do not count.
        assert_eq!(
            detect_delimiter("name,note\n\"Smith; John\",\"a;b;c\"\n", en),
            ','
        );
        assert_eq!(detect_delimiter("x,\"multi\nline;;;\",z\n1,2,3\n", en), ',');
        // A stray header line does not win over the consistent body.
        assert_eq!(
            detect_delimiter("Report; generated\na,b,c\n1,2,3\n4,5,6\n", en),
            ','
        );
        // One column: nothing to detect.
        assert_eq!(detect_delimiter("alpha\nbeta\n", en), ',');
        assert_eq!(detect_delimiter("alpha\nbeta\n", fr), ';');
        assert_eq!(detect_delimiter("", fr), ';');
        // A single column of decimal-comma numbers is not two columns.
        assert_eq!(detect_delimiter("1,5\n2,5\n3,75\n", fr), ';');
        assert_eq!(detect_delimiter("1,5\n2,5\n3,75\n", en), ',');
        // English-style file opened in a French locale.
        assert_eq!(detect_delimiter("name,amount\nA,1.5\nB,2\n", fr), ',');
    }

    #[test]
    fn encodings() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFa,b"), "a,b");
        assert_eq!(decode_text("é€".as_bytes()), "é€");
        // Windows-1252: é = E9, € = 80, œ = 9C.
        assert_eq!(decode_text(b"caf\xE9;\x80;c\x9Cur"), "café;€;cœur");
        assert_eq!(decode_text(b"\xFF\xFEa\x00\xE9\x00"), "aé");
        assert_eq!(decode_text(b"\xFE\xFF\x00a\x00\xE9"), "aé");
        assert_eq!(decode_text(b""), "");
    }

    #[test]
    fn sheet_names() {
        assert_eq!(
            sheet_name_for(Path::new("/tmp/sales 2024.csv")).as_deref(),
            Some("sales 2024")
        );
        assert_eq!(
            sheet_name_for(Path::new("a[b]:c.csv")).as_deref(),
            Some("a_b__c")
        );
        let long = sheet_name_for(Path::new(&format!("{}.csv", "x".repeat(50)))).unwrap();
        assert_eq!(long.chars().count(), 31);
    }
}
