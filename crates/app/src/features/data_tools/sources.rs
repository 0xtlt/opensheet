//! Where external data comes from: fetching the bytes of an address and
//! reading them as the tables the Navigator offers.
//!
//! An address is a path, a `file://` URL or an `http(s)://` URL. Files
//! are read directly. The web is reached through the system's
//! `/usr/bin/curl`, run as a child process: it costs no dependency and
//! no build time, and it uses the certificates and the proxy settings of
//! macOS, which a Rust HTTP client would have to be taught. Fetching
//! blocks: callers run [`fetch`] off the window thread for anything
//! remote, and only when the user asked (an address typed, a refresh).

use opensheet_engine::external::{Datum, QueryItem, SourceFormat, SourceTable, html_tables, json_tables, sniff, xml_tables};
use opensheet_engine::{Locale, Value, Workbook};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The most a source may weigh.
const SIZE_LIMIT: u64 = 50 * 1024 * 1024;
/// Seconds a server has to answer in full.
const TIMEOUT: u32 = 30;

/// What an address gave.
pub(super) struct Fetched {
    pub bytes: Vec<u8>,
    /// The `Content-Type` a server announced.
    pub content_type: Option<String>,
}

/// Why an address gave nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FetchError {
    /// Not an address this program fetches.
    BadAddress,
    /// The file is not there or cannot be read.
    Missing,
    /// No network, no such host, or nobody listening.
    NoConnection,
    /// The server answered with this HTTP status.
    Status(u32),
    Timeout,
    TooLarge,
}

/// Whether an address is on the web rather than on this machine.
pub(super) fn is_remote(address: &str) -> bool {
    let lower = address.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// The file an address names, when it names one: a `file://` URL or a
/// plain path.
pub(super) fn local_path(address: &str) -> Option<PathBuf> {
    if let Some(rest) = address.strip_prefix("file://") {
        // After the (empty or "localhost") host comes the path, with
        // %XX for what a URL cannot hold.
        let path = &rest[rest.find('/')?..];
        let mut bytes = Vec::with_capacity(path.len());
        let raw = path.as_bytes();
        let mut i = 0;
        while i < raw.len() {
            let escaped = (raw[i] == b'%').then(|| raw.get(i + 1..i + 3)).flatten();
            match escaped.and_then(|hex| std::str::from_utf8(hex).ok()).and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                Some(byte) => {
                    bytes.push(byte);
                    i += 3;
                }
                None => {
                    bytes.push(raw[i]);
                    i += 1;
                }
            }
        }
        return Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()));
    }
    (!address.contains("://")).then(|| PathBuf::from(address))
}

/// The `file://` URL of a path.
pub(super) fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => url.push(byte as char),
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

/// An address as typed, made whole: a web address without its scheme
/// gets `http://`.
pub(super) fn normalize(typed: &str) -> String {
    let typed = typed.trim();
    if typed.contains("://") || typed.starts_with('/') || typed.is_empty() {
        typed.to_string()
    } else {
        format!("http://{typed}")
    }
}

/// The program that fetches web addresses.
fn curl() -> PathBuf {
    PathBuf::from("/usr/bin/curl")
}

/// Get what an address holds. Blocks until it is there, at most
/// [`TIMEOUT`] seconds for the web.
pub(super) fn fetch(address: &str) -> Result<Fetched, FetchError> {
    if let Some(path) = local_path(address) {
        let size = std::fs::metadata(&path).map_err(|_| FetchError::Missing)?.len();
        if size > SIZE_LIMIT {
            return Err(FetchError::TooLarge);
        }
        let bytes = std::fs::read(&path).map_err(|_| FetchError::Missing)?;
        return Ok(Fetched { bytes, content_type: None });
    }
    if !is_remote(address) || address.chars().any(|ch| ch.is_control() || ch == ' ') {
        return Err(FetchError::BadAddress);
    }
    let mut child = Command::new(curl())
        .args(["--silent", "--show-error", "--location", "--max-redirs", "10", "--compressed"])
        .args(["--proto", "=http,https", "--proto-redir", "=http,https"])
        .args(["--connect-timeout", "10", "--max-time", &TIMEOUT.to_string(), "--max-filesize", &SIZE_LIMIT.to_string()])
        // The status and the type of the answer, apart from its body.
        .args(["--write-out", "%{stderr}\n%{http_code}\t%{content_type}", "--", address.trim()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| FetchError::NoConnection)?;
    // A server that does not say how much it sends is stopped at the
    // limit all the same.
    let mut bytes = Vec::new();
    let read = child.stdout.take().map(|out| out.take(SIZE_LIMIT + 1).read_to_end(&mut bytes));
    if bytes.len() as u64 > SIZE_LIMIT {
        child.kill().ok();
        child.wait().ok();
        return Err(FetchError::TooLarge);
    }
    let mut report = String::new();
    if let Some(mut err) = child.stderr.take() {
        err.read_to_string(&mut report).ok();
    }
    let status = child.wait().map_err(|_| FetchError::NoConnection)?;
    if !matches!(read, Some(Ok(_))) {
        return Err(FetchError::NoConnection);
    }
    match status.code() {
        Some(0) => {}
        Some(1 | 3) => return Err(FetchError::BadAddress),
        Some(28) => return Err(FetchError::Timeout),
        Some(63) => return Err(FetchError::TooLarge),
        _ => return Err(FetchError::NoConnection),
    }
    let (code, kind) = report.lines().last().and_then(|line| line.split_once('\t')).unwrap_or(("", ""));
    match code.trim().parse::<u32>() {
        Ok(code) if code >= 400 => Err(FetchError::Status(code)),
        _ => Ok(Fetched {
            bytes,
            content_type: Some(kind.trim().to_string()).filter(|kind| !kind.is_empty()),
        }),
    }
}

/// What a source holds: text to cut in the Text Import Wizard, or tables
/// to choose from.
pub(super) enum Document {
    Text(String),
    Tables(Vec<SourceTable>),
}

/// The name of the document at an address: the last piece of its path.
pub(super) fn document_name(address: &str) -> String {
    if let Some(stem) = local_path(address).as_deref().and_then(Path::file_stem) {
        return stem.to_string_lossy().into_owned();
    }
    let path = address.split(['?', '#']).next().unwrap_or(address).trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or(path);
    let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
    if stem.is_empty() { last.to_string() } else { stem.to_string() }
}

/// Read what was fetched. `None` when it holds no table: a page without
/// one, or a document that cannot be read.
pub(super) fn read_document(fetched: &Fetched, address: &str, locale: &'static Locale) -> Option<Document> {
    let text = || opensheet_io::decode_text(&fetched.bytes);
    let tables = match sniff(&fetched.bytes, fetched.content_type.as_deref()) {
        SourceFormat::Text => {
            let text = text();
            return (!text.trim().is_empty()).then_some(Document::Text(text));
        }
        SourceFormat::Html => html_tables(&text()),
        SourceFormat::Json => json_tables(&text(), &document_name(address))?,
        SourceFormat::Xml => xml_tables(&text())?,
        SourceFormat::Workbook => workbook_tables(&fetched.bytes, locale)?,
    };
    (!tables.is_empty()).then_some(Document::Tables(tables))
}

/// The cells of a range of another workbook, with the values they show.
fn range_rows(wb: &Workbook, sheet: usize, range: opensheet_engine::Rect) -> Vec<Vec<Datum>> {
    (range.r0..=range.r1)
        .map(|row| {
            (range.c0..=range.c1)
                .map(|col| match wb.value_at(sheet, row, col) {
                    Value::Empty => Datum::Empty,
                    Value::Number(n) => {
                        let style = wb.style_at(sheet, row, col);
                        if style.is_general() { Datum::Exact(Value::Number(n)) } else { Datum::Formatted(n, style.num_fmt.to_string()) }
                    }
                    Value::Text(text) if text.is_empty() => Datum::Empty,
                    other => Datum::Exact(other),
                })
                .collect()
        })
        .collect()
}

/// What a workbook offers: its tables, its sheets that hold something
/// and its names that are one range, as Excel's Navigator lists them.
fn workbook_tables(bytes: &[u8], locale: &'static Locale) -> Option<Vec<SourceTable>> {
    let wb = opensheet_io::read_xlsx(std::io::Cursor::new(bytes), locale).ok()?;
    let mut out = Vec::new();
    let mut offer = |name: &str, sheet: usize, range| {
        out.push(SourceTable {
            name: name.to_string(),
            item: Some(QueryItem::Name(name.to_string())),
            rows: range_rows(&wb, sheet, range),
        });
    };
    for (index, sheet) in wb.sheets.iter().enumerate() {
        for table in &sheet.tables {
            offer(&table.name, index, table.range);
        }
    }
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if let Some(used) = sheet.used_rect() {
            offer(&sheet.name, index, used);
        }
    }
    for (_, name) in wb.all_names() {
        if let Some([area]) = wb.reference_areas(0, 0, 0, &name.name).as_deref() {
            offer(&name.name, area.sheet, area.rect);
        }
    }
    Some(out)
}

/// A web server for the tests, on this machine only: no test reaches the
/// internet.
#[cfg(test)]
pub(super) mod test_server {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// What the server answers for a path.
    #[derive(Clone)]
    pub struct Answer {
        pub status: u16,
        pub content_type: &'static str,
        pub body: Vec<u8>,
        /// Where a redirection (301, 302) points.
        pub location: Option<String>,
    }

    pub fn page(content_type: &'static str, body: &str) -> Answer {
        Answer {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
            location: None,
        }
    }

    #[derive(Clone)]
    pub struct Server {
        pub port: u16,
        answers: Arc<Mutex<HashMap<String, Answer>>>,
    }

    impl Server {
        /// Start answering on a free port of 127.0.0.1, until the test
        /// process ends.
        pub fn start() -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port on this machine");
            let server = Server {
                port: listener.local_addr().expect("an address").port(),
                answers: Arc::default(),
            };
            let answers = server.answers.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(n) if n > 0 => request.extend_from_slice(&chunk[..n]),
                            _ => break,
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let answer = answers.lock().unwrap().get(&path).cloned().unwrap_or(Answer {
                        status: 404,
                        content_type: "text/html",
                        body: b"<html><body>Not Found</body></html>".to_vec(),
                        location: None,
                    });
                    let mut head = format!("HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n", answer.status, answer.content_type, answer.body.len());
                    if let Some(location) = &answer.location {
                        head.push_str(&format!("Location: {location}\r\n"));
                    }
                    head.push_str("\r\n");
                    stream.write_all(head.as_bytes()).ok();
                    stream.write_all(&answer.body).ok();
                }
            });
            server
        }

        pub fn set(&self, path: &str, answer: Answer) {
            self.answers.lock().unwrap().insert(path.to_string(), answer);
        }

        pub fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{Answer, Server, page};
    use super::*;

    #[test]
    fn addresses() {
        assert!(is_remote("HTTPS://example.test/x") && is_remote(" http://127.0.0.1/"));
        assert!(!is_remote("file:///tmp/x") && !is_remote("/tmp/http://x"));
        let path = Path::new("/Users/ann/Mes données/été 100%.json");
        let url = file_url(path);
        assert_eq!(url, "file:///Users/ann/Mes%20donn%C3%A9es/%C3%A9t%C3%A9%20100%25.json");
        assert_eq!(local_path(&url).as_deref(), Some(path));
        assert_eq!(local_path("file://localhost/tmp/a.csv"), Some(PathBuf::from("/tmp/a.csv")));
        assert_eq!(local_path("/tmp/a b.csv"), Some(PathBuf::from("/tmp/a b.csv")));
        assert_eq!(local_path("http://127.0.0.1/a"), None);
        assert_eq!(normalize(" 127.0.0.1:8080/page "), "http://127.0.0.1:8080/page");
        assert_eq!(normalize("https://x.test/a"), "https://x.test/a");
        assert_eq!(normalize("/tmp/a.json"), "/tmp/a.json");
        assert_eq!(document_name("http://x.test/data/people.json?all=1"), "people");
        assert_eq!(document_name("http://x.test/"), "x");
    }

    #[test]
    fn fetching_from_a_server_on_this_machine() {
        let server = Server::start();
        server.set("/page", page("text/html; charset=utf-8", "<table><tr><td>é</td></tr></table>"));
        server.set(
            "/old",
            Answer {
                status: 302,
                content_type: "text/html",
                body: Vec::new(),
                location: Some("/page".into()),
            },
        );
        server.set(
            "/broken",
            Answer {
                status: 500,
                content_type: "text/plain",
                body: b"no".to_vec(),
                location: None,
            },
        );
        let got = fetch(&server.url("/page")).ok().expect("the page");
        assert_eq!(got.content_type.as_deref(), Some("text/html; charset=utf-8"));
        assert_eq!(String::from_utf8_lossy(&got.bytes), "<table><tr><td>é</td></tr></table>");
        // A redirection is followed.
        let moved = fetch(&server.url("/old")).ok().expect("the page it moved to");
        assert_eq!(moved.bytes, got.bytes);
        assert_eq!(fetch(&server.url("/nothing")).err(), Some(FetchError::Status(404)));
        assert_eq!(fetch(&server.url("/broken")).err(), Some(FetchError::Status(500)));
        // Nobody listens on a port that was just given back.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(fetch(&format!("http://127.0.0.1:{free}/")).err(), Some(FetchError::NoConnection));
        // Only the web and files; nothing that could be read as an option.
        assert_eq!(fetch("ftp://127.0.0.1/x").err(), Some(FetchError::BadAddress));
        assert_eq!(fetch("http://127.0.0.1/a b").err(), Some(FetchError::BadAddress));
        assert_eq!(fetch("/nowhere/at/all.csv").err(), Some(FetchError::Missing));
    }

    #[test]
    fn documents() {
        let fetched = |body: &str, kind: Option<&str>| Fetched {
            bytes: body.as_bytes().to_vec(),
            content_type: kind.map(str::to_string),
        };
        let names = |document: Option<Document>| match document {
            Some(Document::Tables(tables)) => tables.into_iter().map(|table| table.name).collect::<Vec<_>>(),
            Some(Document::Text(_)) => vec!["(text)".to_string()],
            None => Vec::new(),
        };
        let en = Locale::en();
        assert_eq!(names(read_document(&fetched("<html><table><tr><td>1</td></tr></table></html>", None), "http://h/p", en)), ["Table 0"]);
        assert!(names(read_document(&fetched("<html><p>nothing</p></html>", None), "http://h/p", en)).is_empty());
        assert_eq!(names(read_document(&fetched("{\"rows\":[{\"a\":1}]}", None), "file:///tmp/stock.json", en)), ["stock", "rows"]);
        assert_eq!(names(read_document(&fetched("a;b\n1;2\n", Some("text/csv")), "http://h/x.csv", en)), ["(text)"]);
        assert_eq!(names(read_document(&fetched("<list><item><a>1</a></item></list>", None), "/tmp/x.xml", en)), ["item"]);

        // A workbook: its tables first, then its sheets.
        let mut wb = Workbook::new(en);
        wb.sheets[0].name = "Stock".into();
        for (row, cells) in [["Item", "Day"], ["Nut", "2024-03-05"], ["Bolt", "2024-03-06"]].iter().enumerate() {
            for (col, text) in cells.iter().enumerate() {
                wb.set_cell_text(0, row as u32, col as u32, text).unwrap();
            }
        }
        wb.commit("setup");
        let bytes = opensheet_io::write_xlsx(&wb, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
        let Some(Document::Tables(tables)) = read_document(&Fetched { bytes, content_type: None }, "/tmp/stock.xlsx", en) else {
            panic!("a workbook");
        };
        let listed: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
        assert_eq!(listed, ["Stock"]);
        assert_eq!(tables[0].rows[1][0], Datum::Exact(Value::text("Nut")));
        // A date keeps the format that shows it as one.
        assert!(matches!(&tables[0].rows[1][1], Datum::Formatted(n, format) if *n > 45000.0 && format.contains('y')), "{:?}", tables[0].rows[1][1]);
    }
}
