//! External data other than cut text: the tables of a web page, of a
//! JSON or an XML document, or of another workbook (Data > Get Data).
//!
//! This file reads what a source holds into [`SourceTable`]s — the items
//! Excel's Navigator lists — and writes the chosen one into a sheet. The
//! query is remembered with the text imports of the sheet
//! (`Sheet::text_imports`, a [`TextImport`] whose `web` says which item of
//! the document it takes) so that it can be refreshed. Nothing here
//! fetches anything: the caller hands over the bytes it obtained.
//!
//! Text is typed the way a text import types it (numbers, dates, text, in
//! the locale of the workbook), except that nothing read from outside ever
//! becomes a formula.

use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::text_columns::{TextColumnsError, TextImport};
use crate::value::Value;
use crate::workbook::{ClearMode, Content, Workbook};

/// One cell of a table read from a source.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Empty,
    /// Text whose type is still to be detected.
    Auto(String),
    /// A value the source already types (a JSON number, a cell of a
    /// workbook).
    Exact(Value),
    /// A number with the format its source shows it in (a date of
    /// another workbook).
    Formatted(f64, String),
}

impl Datum {
    /// What a preview shows for the cell.
    pub fn preview(&self) -> String {
        match self {
            Datum::Empty => String::new(),
            Datum::Auto(text) => text.clone(),
            Datum::Exact(Value::Text(text)) => text.to_string(),
            Datum::Exact(Value::Number(n)) | Datum::Formatted(n, _) => format!("{n}"),
            Datum::Exact(Value::Bool(b)) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Datum::Exact(_) => String::new(),
        }
    }
}

/// Which item of a document a query takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryItem {
    /// The n-th table of a page, from 1.
    Index(u32),
    /// A table, sheet, name or path called so.
    Name(String),
}

/// What makes a [`TextImport`] a query on a document rather than on cut
/// text: the item taken, or the first when none is named.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct WebQuery {
    pub item: Option<QueryItem>,
}

/// The properties of a query (Connection Properties and External Data
/// Properties).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryProps {
    pub description: String,
    /// "Enable background refresh".
    pub background: bool,
    /// "Refresh data when opening the file".
    pub refresh_on_open: bool,
    /// "Refresh every n minutes"; 0 when off.
    pub refresh_minutes: u32,
    /// "Preserve cell formatting".
    pub preserve_formatting: bool,
    /// "Adjust column width".
    pub adjust_width: bool,
}

impl Default for QueryProps {
    /// What Excel gives a new query.
    fn default() -> QueryProps {
        QueryProps {
            description: String::new(),
            background: true,
            refresh_on_open: false,
            refresh_minutes: 0,
            preserve_formatting: true,
            adjust_width: true,
        }
    }
}

/// A table a source offers.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTable {
    /// The name the Navigator shows.
    pub name: String,
    /// How a query asks for this table again.
    pub item: Option<QueryItem>,
    pub rows: Vec<Vec<Datum>>,
}

impl SourceTable {
    pub fn columns(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }
}

/// What a document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFormat {
    Html,
    Json,
    Xml,
    Text,
    Workbook,
}

/// Tell what the bytes of a source are, from the type its server gave
/// (if any) and from how they begin.
pub fn sniff(bytes: &[u8], content_type: Option<&str>) -> SourceFormat {
    if bytes.starts_with(b"PK\x03\x04") {
        return SourceFormat::Workbook;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_lowercase();
    let head = head.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let markup = |head: &str| {
        if head.contains("<html")
            || head.contains("<!doctype html")
            || head.contains("<table")
            || head.contains("<body")
        {
            SourceFormat::Html
        } else {
            SourceFormat::Xml
        }
    };
    let kind = content_type.unwrap_or("").to_lowercase();
    if kind.contains("json") {
        SourceFormat::Json
    } else if kind.contains("html") {
        SourceFormat::Html
    } else if kind.contains("xml") {
        markup(head)
    } else if kind.contains("csv") {
        SourceFormat::Text
    } else if head.starts_with('{') || head.starts_with('[') {
        SourceFormat::Json
    } else if head.starts_with('<') {
        markup(head)
    } else {
        SourceFormat::Text
    }
}

// ---- markup ----------------------------------------------------------------

/// A piece of an HTML or XML document.
#[derive(Debug, PartialEq)]
enum Token {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    Close(String),
    Text(String),
}

/// The characters a few named entities stand for; pages use many more,
/// which are left as they are written.
const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
    ("nbsp", " "),
    ("euro", "€"),
    ("copy", "©"),
    ("reg", "®"),
    ("deg", "°"),
    ("ndash", "–"),
    ("mdash", "—"),
    ("hellip", "…"),
    ("laquo", "«"),
    ("raquo", "»"),
    ("eacute", "é"),
    ("egrave", "è"),
    ("ecirc", "ê"),
    ("agrave", "à"),
    ("acirc", "â"),
    ("ccedil", "ç"),
    ("ugrave", "ù"),
    ("ocirc", "ô"),
    ("icirc", "î"),
    ("auml", "ä"),
    ("ouml", "ö"),
    ("uuml", "ü"),
    ("szlig", "ß"),
    ("ntilde", "ñ"),
    ("aacute", "á"),
    ("iacute", "í"),
    ("oacute", "ó"),
    ("uacute", "ú"),
    ("pound", "£"),
    ("yen", "¥"),
    ("times", "×"),
    ("minus", "−"),
];

/// Replace the character references of markup text.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..]
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '#'))
            .map(|n| n + 1)
            .unwrap_or(rest.len());
        let name = &rest[1..end];
        let decoded = if let Some(number) = name.strip_prefix('#') {
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok(),
                None => number.parse().ok(),
            };
            code.and_then(char::from_u32).map(String::from)
        } else {
            ENTITIES
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, text)| text.to_string())
        };
        match decoded {
            Some(text) => {
                out.push_str(&text);
                rest = &rest[end..];
                rest = rest.strip_prefix(';').unwrap_or(rest);
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Cut a document into tags and text. Pages are forgiven what browsers
/// forgive: with `html`, names are lower case and what is inside
/// `<script>` and `<style>` is not text.
fn tokens(src: &str, html: bool) -> Vec<Token> {
    let mut out = Vec::new();
    let mut rest = src;
    while !rest.is_empty() {
        let Some(at) = rest.find('<') else {
            out.push(Token::Text(decode_entities(rest)));
            break;
        };
        if at > 0 {
            out.push(Token::Text(decode_entities(&rest[..at])));
        }
        rest = &rest[at..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
        } else if let Some(data) = rest.strip_prefix("<![CDATA[") {
            let end = data.find("]]>").unwrap_or(data.len());
            out.push(Token::Text(data[..end].to_string()));
            rest = data.get(end + 3..).unwrap_or("");
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').unwrap_or(close.len());
            let name = close[..end].trim();
            out.push(Token::Close(if html {
                name.to_lowercase()
            } else {
                name.to_string()
            }));
            rest = close.get(end + 1..).unwrap_or("");
        } else if rest[1..].starts_with(|ch: char| ch.is_alphabetic() || ch == '_') {
            let (token, after) = open_tag(&rest[1..], html);
            let raw = match &token {
                Token::Open {
                    name, empty: false, ..
                } if html && (name == "script" || name == "style") => Some(name.clone()),
                _ => None,
            };
            out.push(token);
            rest = after;
            if let Some(name) = raw {
                // Up to the closing tag, whatever is in between.
                let lower = rest.to_ascii_lowercase();
                rest = lower
                    .find(&format!("</{name}"))
                    .map_or("", |end| &rest[end..]);
            }
        } else {
            // A lone "<" is text.
            out.push(Token::Text("<".to_string()));
            rest = &rest[1..];
        }
    }
    out
}

/// Read a start tag from after its `<`; returns it and what follows.
fn open_tag(src: &str, html: bool) -> (Token, &str) {
    let name_end = src
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(src.len());
    let name = if html {
        src[..name_end].to_lowercase()
    } else {
        src[..name_end].to_string()
    };
    let mut attrs = Vec::new();
    let mut rest = &src[name_end..];
    let mut empty = false;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(after) = rest.strip_prefix('>') {
            rest = after;
            break;
        }
        if let Some(after) = rest.strip_prefix('/') {
            empty = after.trim_start().starts_with('>');
            rest = after;
            continue;
        }
        let key_end = rest
            .find(|ch: char| ch.is_whitespace() || ch == '=' || ch == '>' || ch == '/')
            .unwrap_or(rest.len());
        if key_end == 0 {
            // A stray character: step over it.
            rest = &rest[rest.chars().next().map_or(1, char::len_utf8)..];
            continue;
        }
        let key = if html {
            rest[..key_end].to_lowercase()
        } else {
            rest[..key_end].to_string()
        };
        rest = rest[key_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote) = after.chars().next().filter(|ch| *ch == '"' || *ch == '\'') {
                let body = &after[1..];
                let end = body.find(quote).unwrap_or(body.len());
                value = decode_entities(&body[..end]);
                rest = body.get(end + 1..).unwrap_or("");
            } else {
                let end = after
                    .find(|ch: char| ch.is_whitespace() || ch == '>')
                    .unwrap_or(after.len());
                value = decode_entities(&after[..end]);
                rest = &after[end..];
            }
        }
        attrs.push((key, value));
    }
    // Elements HTML never closes.
    let void = html
        && matches!(
            name.as_str(),
            "br" | "hr" | "img" | "input" | "meta" | "link" | "col" | "wbr" | "area" | "base"
        );
    (
        Token::Open {
            name,
            attrs,
            empty: empty || void,
        },
        rest,
    )
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Runs of white space become one space; none at the ends.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- HTML ------------------------------------------------------------------

/// A `<table>` being read.
#[derive(Default)]
struct HtmlTable {
    /// Its place among the tables of the page, from 0.
    order: usize,
    id: String,
    caption: String,
    in_caption: bool,
    grid: Vec<Vec<Option<String>>>,
    /// The cell being read: its row, column and text.
    cell: Option<(usize, usize, String)>,
}

/// The tables of a page, in the order their `<table>` tags open. A cell
/// that spans several rows or columns leaves the others empty, as a web
/// query of Excel does.
pub fn html_tables(html: &str) -> Vec<SourceTable> {
    struct Build {
        table: HtmlTable,
        row: usize,
        started: bool,
    }
    fn place(build: &mut Build, colspan: usize, rowspan: usize) {
        let row = build.row;
        let grid = &mut build.table.grid;
        let mut col = 0;
        while grid[row].get(col).is_some_and(Option::is_some) {
            col += 1;
        }
        for r in row..row + rowspan {
            while grid.len() <= r {
                grid.push(Vec::new());
            }
            while grid[r].len() < col + colspan {
                grid[r].push(None);
            }
            for c in col..col + colspan {
                grid[r][c] = Some(String::new());
            }
        }
        build.table.cell = Some((row, col, String::new()));
    }
    fn end_cell(build: &mut Build) {
        if let Some((row, col, text)) = build.table.cell.take() {
            build.table.grid[row][col] = Some(tidy(&text));
        }
    }
    fn new_row(build: &mut Build) {
        end_cell(build);
        if build.started {
            build.row += 1;
        }
        build.started = true;
        while build.table.grid.len() <= build.row {
            build.table.grid.push(Vec::new());
        }
    }

    let mut done: Vec<HtmlTable> = Vec::new();
    let mut stack: Vec<Build> = Vec::new();
    let mut count = 0;
    for token in tokens(html, true) {
        match token {
            Token::Open { name, attrs, .. } => match name.as_str() {
                "table" => {
                    stack.push(Build {
                        table: HtmlTable {
                            order: count,
                            id: attr(&attrs, "id").unwrap_or("").to_string(),
                            ..HtmlTable::default()
                        },
                        row: 0,
                        started: false,
                    });
                    count += 1;
                }
                "caption" => {
                    if let Some(build) = stack.last_mut() {
                        build.table.in_caption = true;
                    }
                }
                "tr" => {
                    if let Some(build) = stack.last_mut() {
                        new_row(build);
                    }
                }
                "td" | "th" => {
                    if let Some(build) = stack.last_mut() {
                        end_cell(build);
                        if !build.started {
                            new_row(build);
                        }
                        let span = |name: &str| {
                            attr(&attrs, name)
                                .and_then(|text| text.trim().parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, 1000)
                        };
                        place(build, span("colspan"), span("rowspan"));
                    }
                }
                "br" | "p" | "div" | "li" => {
                    if let Some((_, _, text)) =
                        stack.last_mut().and_then(|build| build.table.cell.as_mut())
                    {
                        text.push(' ');
                    }
                }
                _ => {}
            },
            Token::Close(name) => match name.as_str() {
                "table" => {
                    if let Some(mut build) = stack.pop() {
                        end_cell(&mut build);
                        done.push(build.table);
                    }
                }
                "caption" => {
                    if let Some(build) = stack.last_mut() {
                        build.table.in_caption = false;
                    }
                }
                "td" | "th" | "tr" => {
                    if let Some(build) = stack.last_mut() {
                        end_cell(build);
                    }
                }
                _ => {}
            },
            Token::Text(text) => {
                if let Some(build) = stack.last_mut() {
                    if build.table.in_caption {
                        build.table.caption.push_str(&text);
                    } else if let Some((_, _, cell)) = build.table.cell.as_mut() {
                        cell.push_str(&text);
                    }
                }
            }
        }
    }
    // A page cut short still gives the tables it began.
    while let Some(mut build) = stack.pop() {
        end_cell(&mut build);
        done.push(build.table);
    }
    done.sort_by_key(|table| table.order);
    done.into_iter()
        .filter(|table| table.grid.iter().any(|row| !row.is_empty()))
        .map(|table| {
            let caption = tidy(&table.caption);
            let name = if !caption.is_empty() {
                caption
            } else if !table.id.is_empty() {
                table.id.clone()
            } else {
                format!("Table {}", table.order)
            };
            let rows = table
                .grid
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| match cell {
                            Some(text) if !text.is_empty() => Datum::Auto(text),
                            _ => Datum::Empty,
                        })
                        .collect()
                })
                .collect();
            SourceTable {
                name,
                item: Some(QueryItem::Index(table.order as u32 + 1)),
                rows,
            }
        })
        .collect()
}

// ---- JSON ------------------------------------------------------------------

/// A JSON value; the members of an object keep their order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<Json>),
    Record(Vec<(String, Json)>),
}

struct JsonReader<'a> {
    src: &'a [u8],
    at: usize,
    depth: usize,
}

impl JsonReader<'_> {
    fn space(&mut self) {
        while self
            .src
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.space();
        if self.src.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, word: &str, value: Json) -> Option<Json> {
        if self.src[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Some(value)
        } else {
            None
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.space();
        // Documents nested deeper than any real one are refused rather
        // than followed to the end of the stack.
        if self.depth > 200 {
            return None;
        }
        match *self.src.get(self.at)? {
            b'{' => {
                self.at += 1;
                self.depth += 1;
                let mut members = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.space();
                        let key = self.string()?;
                        if !self.eat(b':') {
                            return None;
                        }
                        members.push((key, self.value()?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::Record(members))
            }
            b'[' => {
                self.at += 1;
                self.depth += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                self.depth -= 1;
                Some(Json::List(items))
            }
            b'"' => self.string().map(Json::Text),
            b't' => self.word("true", Json::Bool(true)),
            b'f' => self.word("false", Json::Bool(false)),
            b'n' => self.word("null", Json::Null),
            _ => {
                let start = self.at;
                while self.src.get(self.at).is_some_and(|byte| {
                    matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                }) {
                    self.at += 1;
                }
                std::str::from_utf8(&self.src[start..self.at])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Json::Number)
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = std::str::from_utf8(self.src.get(self.at..self.at + 4)?).ok()?;
        self.at += 4;
        u32::from_str_radix(digits, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if self.src.get(self.at) != Some(&b'"') {
            return None;
        }
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let byte = *self.src.get(self.at)?;
            self.at += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = *self.src.get(self.at)?;
                    self.at += 1;
                    let ch = match escape {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let mut code = self.hex4()?;
                            // Two escapes for a character outside the
                            // basic plane.
                            if (0xD800..0xDC00).contains(&code)
                                && self.src[self.at..].starts_with(b"\\u")
                            {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000
                                    + ((code - 0xD800) << 10)
                                    + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(code).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                other => out.push(other),
            }
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Read a JSON document; `None` when it is not one.
pub fn parse_json(text: &str) -> Option<Json> {
    let mut reader = JsonReader {
        src: text.trim_start_matches('\u{feff}').as_bytes(),
        at: 0,
        depth: 0,
    };
    let value = reader.value()?;
    reader.space();
    (reader.at == reader.src.len()).then_some(value)
}

fn json_scalar(value: &Json) -> Datum {
    match value {
        Json::Null => Datum::Empty,
        Json::Bool(b) => Datum::Exact(Value::Bool(*b)),
        Json::Number(n) => Datum::Exact(Value::Number(*n)),
        Json::Text(text) if text.is_empty() => Datum::Empty,
        Json::Text(text) => Datum::Auto(text.clone()),
        Json::List(items) => {
            // A list of plain values fits a cell; a list of records is
            // a table of its own, which the Navigator offers.
            if items
                .iter()
                .all(|item| !matches!(item, Json::List(_) | Json::Record(_)))
            {
                let texts: Vec<String> = items
                    .iter()
                    .map(|item| json_scalar(item).preview())
                    .collect();
                Datum::Exact(Value::text(texts.join(", ")))
            } else {
                Datum::Exact(Value::text("[List]"))
            }
        }
        Json::Record(_) => Datum::Exact(Value::text("[Record]")),
    }
}

/// The members of a record as columns: those of a nested record are named
/// `outer.inner`, the way Excel names the columns it expands.
fn flatten(members: &[(String, Json)], prefix: &str, out: &mut Vec<(String, Datum)>) {
    for (key, value) in members {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Json::Record(inner) if !inner.is_empty() => flatten(inner, &name, out),
            other => out.push((name, json_scalar(other))),
        }
    }
}

/// Lay a JSON value out as rows: a list gives a row per item under the
/// names of the members, a record a row per member.
fn json_rows(value: &Json) -> Vec<Vec<Datum>> {
    let header = |names: &[String]| {
        names
            .iter()
            .map(|name| Datum::Exact(Value::text(name)))
            .collect::<Vec<_>>()
    };
    match value {
        Json::List(items) => {
            let mut names: Vec<String> = Vec::new();
            let mut records = Vec::new();
            for item in items {
                let mut fields = Vec::new();
                match item {
                    Json::Record(members) => flatten(members, "", &mut fields),
                    other => fields.push(("Value".to_string(), json_scalar(other))),
                }
                for (name, _) in &fields {
                    if !names.contains(name) {
                        names.push(name.clone());
                    }
                }
                records.push(fields);
            }
            let mut rows = vec![header(&names)];
            for fields in records {
                let mut row = vec![Datum::Empty; names.len()];
                for (name, datum) in fields {
                    if let Some(col) = names.iter().position(|known| *known == name) {
                        row[col] = datum;
                    }
                }
                rows.push(row);
            }
            rows
        }
        Json::Record(members) => {
            let mut fields = Vec::new();
            flatten(members, "", &mut fields);
            let mut rows = vec![header(&["Name".to_string(), "Value".to_string()])];
            rows.extend(
                fields
                    .into_iter()
                    .map(|(name, datum)| vec![Datum::Exact(Value::text(name)), datum]),
            );
            rows
        }
        scalar => vec![vec![json_scalar(scalar)]],
    }
}

/// The lists of records inside a document, with the path that leads to
/// each (`data.items`).
fn json_lists<'a>(value: &'a Json, path: &str, depth: usize, out: &mut Vec<(String, &'a Json)>) {
    if let (Json::Record(members), true) = (value, depth < 6) {
        for (key, inner) in members {
            let name = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            match inner {
                Json::List(items) if items.iter().any(|item| matches!(item, Json::Record(_))) => {
                    out.push((name, inner))
                }
                Json::Record(_) => json_lists(inner, &name, depth + 1, out),
                _ => {}
            }
        }
    }
}

/// The tables of a JSON document: the document itself (named `name`),
/// then every list of records found inside it.
pub fn json_tables(text: &str, name: &str) -> Option<Vec<SourceTable>> {
    let root = parse_json(text)?;
    let mut tables = vec![SourceTable {
        name: name.to_string(),
        item: None,
        rows: json_rows(&root),
    }];
    let mut lists = Vec::new();
    json_lists(&root, "", 0, &mut lists);
    for (path, list) in lists {
        tables.push(SourceTable {
            name: path.clone(),
            item: Some(QueryItem::Name(path)),
            rows: json_rows(list),
        });
    }
    Some(tables)
}

// ---- XML -------------------------------------------------------------------

#[derive(Default)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
    text: String,
}

/// The root element of an XML document.
fn parse_xml(text: &str) -> Option<Element> {
    let mut stack: Vec<Element> = vec![Element::default()];
    for token in tokens(text, false) {
        match token {
            Token::Open { name, attrs, empty } => {
                let element = Element {
                    name,
                    attrs,
                    ..Element::default()
                };
                if empty {
                    stack.last_mut()?.children.push(element);
                } else {
                    stack.push(element);
                }
            }
            Token::Close(_) => {
                if stack.len() > 1 {
                    let element = stack.pop()?;
                    stack.last_mut()?.children.push(element);
                }
            }
            Token::Text(text) => stack.last_mut()?.text.push_str(&text),
        }
    }
    // What was left open is closed where the document ends.
    while stack.len() > 1 {
        let element = stack.pop()?;
        stack.last_mut()?.children.push(element);
    }
    stack.pop()?.children.into_iter().next()
}

/// An element as columns: its attributes, then its children, those of a
/// child that has children of its own named `child.inner`.
fn xml_fields(element: &Element, prefix: &str, out: &mut Vec<(String, String)>) {
    let named = |name: &str| {
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        }
    };
    for (key, value) in &element.attrs {
        if !key.starts_with("xmlns") {
            out.push((named(&format!("Attribute:{key}")), value.clone()));
        }
    }
    if element.children.is_empty() {
        let text = tidy(&element.text);
        if !text.is_empty() || element.attrs.is_empty() {
            out.push((
                if prefix.is_empty() {
                    element.name.clone()
                } else {
                    prefix.to_string()
                },
                text,
            ));
        }
        return;
    }
    for child in &element.children {
        let name = named(&child.name);
        if child.children.is_empty() && child.attrs.is_empty() {
            let text = tidy(&child.text);
            // The same child several times: its values side by side.
            match out.iter_mut().find(|(known, _)| *known == name) {
                Some((_, value)) => {
                    value.push_str(", ");
                    value.push_str(&text);
                }
                None => out.push((name, text)),
            }
        } else {
            xml_fields(child, &name, out);
        }
    }
}

fn xml_rows(elements: &[&Element]) -> Vec<Vec<Datum>> {
    let mut names: Vec<String> = Vec::new();
    let mut records = Vec::new();
    for element in elements {
        let mut fields = Vec::new();
        xml_fields(element, "", &mut fields);
        for (name, _) in &fields {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        records.push(fields);
    }
    let mut rows = vec![
        names
            .iter()
            .map(|name| Datum::Exact(Value::text(name)))
            .collect::<Vec<_>>(),
    ];
    for fields in records {
        let mut row = vec![Datum::Empty; names.len()];
        for (name, text) in fields {
            if let (Some(col), false) = (
                names.iter().position(|known| *known == name),
                text.is_empty(),
            ) {
                row[col] = Datum::Auto(text);
            }
        }
        rows.push(row);
    }
    rows
}

/// The tables of an XML document: one for each kind of element directly
/// under the root, a row per element. A root with plain values only is a
/// table of one row.
pub fn xml_tables(text: &str) -> Option<Vec<SourceTable>> {
    let root = parse_xml(text)?;
    let records: Vec<&Element> = root
        .children
        .iter()
        .filter(|child| !child.children.is_empty() || !child.attrs.is_empty())
        .collect();
    if records.is_empty() {
        return Some(vec![SourceTable {
            name: root.name.clone(),
            item: None,
            rows: xml_rows(&[&root]),
        }]);
    }
    let mut kinds: Vec<&str> = Vec::new();
    for record in &records {
        if !kinds.contains(&record.name.as_str()) {
            kinds.push(&record.name);
        }
    }
    Some(
        kinds
            .into_iter()
            .map(|kind| {
                let elements: Vec<&Element> = records
                    .iter()
                    .copied()
                    .filter(|record| record.name == kind)
                    .collect();
                SourceTable {
                    name: kind.to_string(),
                    item: Some(QueryItem::Name(kind.to_string())),
                    rows: xml_rows(&elements),
                }
            })
            .collect(),
    )
}

/// The table a query asks for among those of its source: the item it
/// names, or the first.
pub fn pick<'a>(tables: &'a [SourceTable], query: &WebQuery) -> Option<&'a SourceTable> {
    match &query.item {
        None => tables.first(),
        Some(item) => tables
            .iter()
            .find(|table| table.item.as_ref() == Some(item)),
    }
}

// ---- writing a table into a sheet ------------------------------------------

impl Workbook {
    /// Where `table` would land from `dest`.
    pub fn source_table_extent(
        &self,
        dest: (u32, u32),
        table: &SourceTable,
    ) -> Result<Rect, TextColumnsError> {
        let (rows, cols) = (table.rows.len() as u32, table.columns() as u32);
        if rows == 0 || cols == 0 {
            return Err(TextColumnsError::NoData);
        }
        if dest.0 + rows > MAX_ROWS || dest.1 + cols > MAX_COLS {
            return Err(TextColumnsError::OffSheet);
        }
        Ok(Rect::new(
            dest.0,
            dest.1,
            dest.0 + rows - 1,
            dest.1 + cols - 1,
        ))
    }

    fn write_source_table(&mut self, sheet: usize, dest: (u32, u32), table: &SourceTable) {
        for (i, cells) in table.rows.iter().enumerate() {
            let row = dest.0 + i as u32;
            for (j, datum) in cells.iter().enumerate() {
                let col = dest.1 + j as u32;
                match datum {
                    Datum::Empty => self.set_content(sheet, row, col, Content::Empty),
                    Datum::Exact(value) => {
                        self.set_content(sheet, row, col, Content::Value(value.clone()))
                    }
                    Datum::Formatted(number, format) => {
                        if self.style_at(sheet, row, col).is_general() {
                            let format: std::sync::Arc<str> = std::sync::Arc::from(format.as_str());
                            self.update_style(sheet, Rect::cell(row, col), &move |s| {
                                s.num_fmt = format.clone()
                            });
                        }
                        self.set_content(sheet, row, col, Content::Value(Value::Number(*number)));
                    }
                    Datum::Auto(text) => {
                        // Typed as at the keyboard, but what comes from
                        // outside never computes.
                        let typed =
                            !text.starts_with(['=', '+', '-', '@']) || text.parse::<f64>().is_ok();
                        if !typed
                            || self.set_cell_text(sheet, row, col, text).is_err()
                            || self.is_formula(sheet, row, col)
                        {
                            self.set_content(sheet, row, col, Content::Value(Value::text(text)));
                        }
                    }
                }
            }
        }
    }

    fn is_formula(&self, sheet: usize, row: u32, col: u32) -> bool {
        self.cell(sheet, row, col)
            .is_some_and(|cell| matches!(cell.content, Content::Formula(_)))
    }

    /// Load `table` into `sheet` from `dest` and remember the query
    /// there. `import.range` and an empty `import.name` are filled in.
    /// Returns the rectangle written. The caller commits.
    pub fn import_source_table(
        &mut self,
        sheet: usize,
        dest: (u32, u32),
        table: &SourceTable,
        mut import: TextImport,
    ) -> Result<Rect, TextColumnsError> {
        if sheet >= self.sheets.len() {
            return Err(TextColumnsError::NoData);
        }
        let extent = self.source_table_extent(dest, table)?;
        self.write_source_table(sheet, dest, table);
        if import.name.is_empty() {
            import.name = self.text_import_name(&import.path);
        }
        import.range = extent;
        self.snapshot();
        self.sheets[sheet].text_imports.push(import);
        Ok(extent)
    }

    /// Refresh a query with the table its source holds now: the old data
    /// is cleared (with its formats when the query does not preserve
    /// them) and the new written from the same corner. The caller
    /// commits.
    pub fn refresh_source_table(
        &mut self,
        sheet: usize,
        index: usize,
        table: &SourceTable,
    ) -> Result<Rect, TextColumnsError> {
        let import = self
            .sheets
            .get(sheet)
            .and_then(|sh| sh.text_imports.get(index))
            .cloned()
            .ok_or(TextColumnsError::NoData)?;
        let dest = (import.range.r0, import.range.c0);
        let extent = self.source_table_extent(dest, table)?;
        self.clear_import(sheet, &import);
        self.write_source_table(sheet, dest, table);
        self.snapshot();
        self.sheets[sheet].text_imports[index].range = extent;
        Ok(extent)
    }

    /// Empty the cells of a query before they are filled again.
    pub(crate) fn clear_import(&mut self, sheet: usize, import: &TextImport) {
        self.clear(sheet, import.range, ClearMode::Contents);
        if !import.props.preserve_formatting {
            self.clear(sheet, import.range, ClearMode::Formats);
        }
    }

    /// Whether a query of `sheet` could be called `name`: a valid name
    /// that no other query (the one at `except` aside) and no defined
    /// name has.
    pub fn import_name_free(&self, name: &str, except: (usize, usize)) -> bool {
        let valid = name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
            && name
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.')
            && crate::refs::parse_cell_name(name).is_none();
        let key = name.to_uppercase();
        let taken = self.names.contains_key(&key)
            || self.sheets.iter().enumerate().any(|(sheet, on)| {
                on.names.contains_key(&key)
                    || on.text_imports.iter().enumerate().any(|(index, import)| {
                        (sheet, index) != except && import.name.to_uppercase() == key
                    })
            });
        valid && !taken
    }

    /// Change a query (its name, address, item or properties). The
    /// caller commits.
    pub fn update_import(
        &mut self,
        sheet: usize,
        index: usize,
        change: impl FnOnce(&mut TextImport),
    ) {
        if self
            .sheets
            .get(sheet)
            .is_some_and(|sh| index < sh.text_imports.len())
        {
            self.snapshot();
            change(&mut self.sheets[sheet].text_imports[index]);
        }
    }

    /// Forget a query; its data stays on the sheet as plain cells. The
    /// caller commits.
    pub fn remove_import(&mut self, sheet: usize, index: usize) {
        if self
            .sheets
            .get(sheet)
            .is_some_and(|sh| index < sh.text_imports.len())
        {
            self.snapshot();
            self.sheets[sheet].text_imports.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::text_columns::{Delimiters, Layout, Spec};

    fn texts(table: &SourceTable) -> Vec<Vec<String>> {
        table
            .rows
            .iter()
            .map(|row| row.iter().map(Datum::preview).collect())
            .collect()
    }

    fn query(path: &str, item: Option<QueryItem>) -> TextImport {
        TextImport {
            name: String::new(),
            path: path.to_string(),
            range: Rect::cell(0, 0),
            first_row: 1,
            spec: Spec::new(Layout::Delimited(Delimiters::default())),
            web: Some(WebQuery { item }),
            props: QueryProps::default(),
        }
    }

    const PAGE: &str = r#"<!DOCTYPE html><html><head><title>Sales</title>
        <style>td { color: red } table > tr {}</style>
        <script>if (a < b) { document.write("<table><tr><td>no</td></tr></table>"); }</script></head>
        <body><p>Intro</p>
        <table id="t1"><caption>Sales &amp; returns</caption>
          <thead><tr><th>Name</th><th>Qty</th><th>When</th></tr></thead>
          <tr><td>Ann<br>Lee</td><td> 1,5 </td><td>2024-03-05</td>
          <tr><td>B&eacute;a</td><td>4</td><td>=1+1</td></tr>
        </table>
        <TABLE><TR><TD colspan=2>Wide</TD><TD rowspan="2">Tall</TD></TR><TR><TD>a</TD><TD>b</TD></TR>
          <tr><td>outer<table><tr><td>inner</td></tr></table></td></tr></TABLE>
        </body></html>"#;

    #[test]
    fn tables_of_a_page() {
        let tables = html_tables(PAGE);
        let names: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
        assert_eq!(names, ["Sales & returns", "Table 1", "Table 2"]);
        assert_eq!(tables[0].item, Some(QueryItem::Index(1)));
        assert_eq!(
            texts(&tables[0]),
            [
                ["Name", "Qty", "When"],
                ["Ann Lee", "1,5", "2024-03-05"],
                ["Béa", "4", "=1+1"]
            ]
        );
        // Spans leave the cells they cover empty; a nested table is a
        // table of its own and not text of the cell around it.
        assert_eq!(
            texts(&tables[1]),
            [vec!["Wide", "", "Tall"], vec!["a", "b", ""], vec!["outer"]]
        );
        assert_eq!(texts(&tables[2]), [["inner"]]);
        assert!(html_tables("<p>No table here</p>").is_empty());
        // A page that stops in the middle of a table.
        assert_eq!(
            texts(&html_tables("<table><tr><td>x<td>y")[0]),
            [["x", "y"]]
        );
    }

    #[test]
    fn json_documents() {
        assert_eq!(
            parse_json(r#" {"a": [1, -2.5e1, "xé\n", true, null]} "#),
            Some(Json::Record(vec![(
                "a".to_string(),
                Json::List(vec![
                    Json::Number(1.0),
                    Json::Number(-25.0),
                    Json::Text("xé\n".to_string()),
                    Json::Bool(true),
                    Json::Null
                ]),
            )]))
        );
        assert_eq!(parse_json("{\"a\": }"), None);
        assert_eq!(parse_json("[1, 2] trailing"), None);
        assert_eq!(parse_json(r#""😀""#), Some(Json::Text("😀".to_string())));

        let text = r#"[{"name":"Ann","address":{"city":"Paris","geo":{"lat":48.8}},"tags":["a","b"]},
                       {"name":"Bob","age":41,"address":{"city":"Lyon"},"orders":[{"id":1}]}]"#;
        let tables = json_tables(text, "people").unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "people");
        assert_eq!(
            texts(&tables[0]),
            [
                vec![
                    "name",
                    "address.city",
                    "address.geo.lat",
                    "tags",
                    "age",
                    "orders"
                ],
                vec!["Ann", "Paris", "48.8", "a, b", "", ""],
                vec!["Bob", "Lyon", "", "", "41", "[List]"],
            ]
        );
        assert_eq!(tables[0].rows[1][2], Datum::Exact(Value::Number(48.8)));

        // A record: a row per member, and its lists of records as tables.
        let tables = json_tables(
            r#"{"total":2,"page":{"size":10},"data":{"items":[{"id":1},{"id":2}]}}"#,
            "doc",
        )
        .unwrap();
        assert_eq!(
            texts(&tables[0]),
            [
                ["Name", "Value"],
                ["total", "2"],
                ["page.size", "10"],
                ["data.items", "[List]"]
            ]
        );
        assert_eq!(tables[1].name, "data.items");
        assert_eq!(
            tables[1].item,
            Some(QueryItem::Name("data.items".to_string()))
        );
        assert_eq!(texts(&tables[1]), [["id"], ["1"], ["2"]]);
        assert_eq!(
            pick(
                &tables,
                &WebQuery {
                    item: Some(QueryItem::Name("data.items".to_string()))
                }
            )
            .map(|table| table.rows.len()),
            Some(3)
        );
        assert!(
            pick(
                &tables,
                &WebQuery {
                    item: Some(QueryItem::Index(4))
                }
            )
            .is_none()
        );
        assert!(json_tables("not json", "x").is_none());
    }

    #[test]
    fn xml_documents() {
        let text = r#"<?xml version="1.0"?><!-- stock --><catalog xmlns="urn:x">
            <book id="b1"><title>Dune</title><price>9.5</price><author><first>Frank</first><last>Herbert</last></author></book>
            <book id="b2"><title><![CDATA[War & Peace]]></title><tag>a</tag><tag>b</tag></book>
            <shop city="Paris"/>
            <updated>2024-01-02</updated></catalog>"#;
        let tables = xml_tables(text).unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].name, "book");
        assert_eq!(
            texts(&tables[0]),
            [
                vec![
                    "Attribute:id",
                    "title",
                    "price",
                    "author.first",
                    "author.last",
                    "tag"
                ],
                vec!["b1", "Dune", "9.5", "Frank", "Herbert", ""],
                vec!["b2", "War & Peace", "", "", "", "a, b"],
            ]
        );
        assert_eq!(texts(&tables[1]), [["Attribute:city"], ["Paris"]]);
        // Plain values under the root: one row.
        let flat =
            xml_tables("<rate><from>EUR</from><to>USD</to><value>1.08</value></rate>").unwrap();
        assert_eq!(flat[0].item, None);
        assert_eq!(
            texts(&flat[0]),
            [["from", "to", "value"], ["EUR", "USD", "1.08"]]
        );
        assert!(xml_tables("").is_none());
    }

    #[test]
    fn formats_are_told_apart() {
        assert_eq!(sniff(b"PK\x03\x04rest", None), SourceFormat::Workbook);
        assert_eq!(
            sniff(b"\n  <!DOCTYPE HTML><html>", None),
            SourceFormat::Html
        );
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><rows/>", None),
            SourceFormat::Xml
        );
        assert_eq!(
            sniff(
                b"<?xml version=\"1.0\"?><html><body/></html>",
                Some("application/xhtml+xml")
            ),
            SourceFormat::Html
        );
        assert_eq!(sniff(b" [1]", None), SourceFormat::Json);
        assert_eq!(sniff(b"a,b\n1,2", None), SourceFormat::Text);
        assert_eq!(
            sniff(b"a,b\n1,2", Some("application/json; charset=utf-8")),
            SourceFormat::Json
        );
        assert_eq!(sniff(b"[x],b", Some("text/csv")), SourceFormat::Text);
    }

    #[test]
    fn load_and_refresh_a_table() {
        let mut w = Workbook::new(Locale::fr());
        let tables = html_tables(PAGE);
        let extent = w
            .import_source_table(
                0,
                (1, 1),
                &tables[0],
                query(
                    "http://127.0.0.1:1/sales/report.html?year=2024",
                    Some(QueryItem::Index(1)),
                ),
            )
            .unwrap();
        w.commit("Import Data");
        assert_eq!(extent, Rect::parse("B2:D4").unwrap());
        assert_eq!(w.sheets[0].text_imports[0].name, "report");
        assert_eq!(w.value_at(0, 1, 1), Value::text("Name"));
        // Typed in the locale of the workbook: a decimal comma, a date.
        assert_eq!(w.value_at(0, 2, 2), Value::Number(1.5));
        assert!(matches!(w.value_at(0, 2, 3), Value::Number(n) if n > 45000.0));
        // What looks like a formula stays text.
        assert_eq!(w.value_at(0, 3, 3), Value::text("=1+1"));
        assert_eq!(w.text_import_at(0, 3, 3), Some(0));

        // Fewer rows and columns after a refresh: the rest is cleared.
        let smaller = SourceTable {
            name: String::new(),
            item: None,
            rows: vec![
                vec![Datum::Auto("Name".into())],
                vec![Datum::Auto("-12".into())],
            ],
        };
        let extent = w.refresh_source_table(0, 0, &smaller).unwrap();
        w.commit("Refresh");
        assert_eq!(extent, Rect::parse("B2:B3").unwrap());
        assert_eq!(w.value_at(0, 2, 1), Value::Number(-12.0));
        assert_eq!(w.value_at(0, 3, 3), Value::Empty);
        assert_eq!(w.sheets[0].text_imports[0].range, extent);
        w.undo();
        assert_eq!(w.value_at(0, 3, 3), Value::text("=1+1"));
        assert_eq!(
            w.sheets[0].text_imports[0].range,
            Rect::parse("B2:D4").unwrap()
        );

        let empty = SourceTable {
            name: String::new(),
            item: None,
            rows: Vec::new(),
        };
        assert_eq!(
            w.refresh_source_table(0, 0, &empty),
            Err(TextColumnsError::NoData)
        );
        w.remove_import(0, 0);
        w.commit("Delete");
        assert!(w.sheets[0].text_imports.is_empty());
        assert_eq!(w.value_at(0, 1, 1), Value::text("Name"));
    }
}
