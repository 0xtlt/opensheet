//! Where external data comes from: fetching the bytes of an address and
//! reading them as the tables the Navigator offers.
//!
//! A web address is an `http://` or `https://` URL. It is fetched in
//! process with [`ureq`], a small blocking client, so nothing is
//! shelled out. TLS is rustls with the Mozilla root set, which builds
//! the same way on macOS, Windows and Linux and is the stack GPUI
//! already links.
//! GPUI's own HTTP client is an async fork of reqwest and does not
//! turn on compression, so it is not reused here. The request asks for
//! gzip, deflate and brotli, and the body is decompressed before it is
//! returned. An `HTTP_PROXY`, `HTTPS_PROXY` or `ALL_PROXY` variable is
//! honoured. Fetching blocks: callers run [`fetch`] off the window
//! thread, and only after the user has asked (an address typed, a
//! refresh) or agreed to a schedule the workbook itself requested.
//!
//! Only `http` and `https` are requested, and a redirect is followed
//! only when it stays on those two schemes. A `file://` URL or a bare
//! path is not a web address, and [`fetch`] refuses it. Local files
//! are read by [`read_local`] when the user imports one — text, JSON,
//! XML or a workbook — or refreshes a text import. A web query that
//! arrived in a workbook cannot point this reader at a file on the
//! machine.

use opensheet_engine::external::{Datum, QueryItem, SourceFormat, SourceTable, html_tables, json_tables, sniff, xml_tables};
use opensheet_engine::{Locale, Value, Workbook};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The most a source may weigh, decompressed.
const SIZE_LIMIT: u64 = 50 * 1024 * 1024;
/// Seconds a server has to answer in full, redirects included.
const TIMEOUT: u64 = 30;
/// How long a connection may take to open.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Redirects followed before giving up. The next one is refused.
const MAX_REDIRECTS: u32 = 10;

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
    /// A `file://` URL or a path, which a web query is not allowed to open.
    LocalFile,
}

/// Whether an address is on the web rather than on this machine.
pub(super) fn is_remote(address: &str) -> bool {
    let lower = address.trim_start().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Whether an address names a file on this machine: a `file:` URL, in
/// any case, or a path that carries no scheme.
pub(super) fn names_a_file(address: &str) -> bool {
    let address = address.trim();
    local_path(address).is_some() || address.to_ascii_lowercase().starts_with("file:")
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

/// Read a file the user named: a `file://` URL or a plain path.
pub(super) fn read_local(address: &str) -> Result<Fetched, FetchError> {
    let Some(path) = local_path(address) else {
        return Err(FetchError::BadAddress);
    };
    let size = std::fs::metadata(&path).map_err(|_| FetchError::Missing)?.len();
    if size > SIZE_LIMIT {
        return Err(FetchError::TooLarge);
    }
    let bytes = std::fs::read(&path).map_err(|_| FetchError::Missing)?;
    Ok(Fetched { bytes, content_type: None })
}

/// Whether this text may be handed to the HTTP client: `http` or
/// `https`, and no space or control character that would change the
/// request line.
fn allowed_web_address(address: &str) -> bool {
    is_remote(address) && !address.chars().any(|ch| ch.is_control() || ch == ' ')
}

/// The client for one fetch. Redirects are followed by [`fetch`] itself,
/// so a `Location` that leaves http(s) is never requested. The default
/// config already reads a proxy from the environment.
fn web_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(Duration::from_secs(TIMEOUT)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

/// Get what a web address holds. Blocks until it is there, at most
/// [`TIMEOUT`] seconds. A file URL or a path is refused: those are not
/// fetched, they are read with [`read_local`] when the user asked for a file.
pub(super) fn fetch(address: &str) -> Result<Fetched, FetchError> {
    if names_a_file(address) {
        return Err(FetchError::LocalFile);
    }
    if !allowed_web_address(address) {
        return Err(FetchError::BadAddress);
    }
    let agent = web_agent();
    let deadline = Instant::now() + Duration::from_secs(TIMEOUT);
    let mut address = address.trim().to_string();
    for hop in 0..=MAX_REDIRECTS {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(FetchError::Timeout);
        }
        let response = agent
            .get(&address)
            .header("Accept-Encoding", "gzip, deflate, br")
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
            .map_err(transport_error)?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if hop == MAX_REDIRECTS {
                // The original request plus ten redirects have already run.
                return Err(FetchError::NoConnection);
            }
            let location = response.headers().get("location").and_then(|value| value.to_str().ok()).ok_or(FetchError::BadAddress)?;
            address = redirect_target(&address, location)?;
            continue;
        }
        if status >= 400 {
            return Err(FetchError::Status(u32::from(status)));
        }
        return read_response(response);
    }
    Err(FetchError::NoConnection)
}

/// The address a `Location` names, refused unless it is still http(s).
fn redirect_target(current: &str, location: &str) -> Result<String, FetchError> {
    let base = url::Url::parse(current).map_err(|_| FetchError::BadAddress)?;
    let next = base.join(location.trim()).map_err(|_| FetchError::BadAddress)?;
    let address = next.as_str();
    if !allowed_web_address(address) {
        return Err(FetchError::BadAddress);
    }
    Ok(address.to_string())
}

fn transport_error(error: ureq::Error) -> FetchError {
    match error {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        ureq::Error::StatusCode(code) => FetchError::Status(u32::from(code)),
        ureq::Error::BadUri(_) | ureq::Error::RedirectFailed | ureq::Error::RequireHttpsOnly(_) => FetchError::BadAddress,
        ureq::Error::BodyExceedsLimit(_) => FetchError::TooLarge,
        _ => FetchError::NoConnection,
    }
}

/// The body, decompressed, and the type the server announced.
fn read_response(mut response: ureq::http::Response<ureq::Body>) -> Result<Fetched, FetchError> {
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    // gzip and brotli are already inflated by the client, which then
    // drops this header. deflate is not, so it is still named here.
    let deflate = response
        .headers()
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim().eq_ignore_ascii_case("deflate")));
    let bytes = read_limited(response.body_mut().as_reader())?;
    let bytes = if deflate { inflate(&bytes)? } else { bytes };
    Ok(Fetched { bytes, content_type })
}

/// Read at most [`SIZE_LIMIT`] bytes. One more means the source is too big.
fn read_limited(mut reader: impl Read) -> Result<Vec<u8>, FetchError> {
    let mut bytes = Vec::new();
    reader.by_ref().take(SIZE_LIMIT + 1).read_to_end(&mut bytes).map_err(read_error)?;
    if bytes.len() as u64 > SIZE_LIMIT { Err(FetchError::TooLarge) } else { Ok(bytes) }
}

/// `Content-Encoding: deflate` is a zlib stream, except when a server
/// sends the raw form. Try the zlib wrapper first.
fn inflate(bytes: &[u8]) -> Result<Vec<u8>, FetchError> {
    let zlib = read_limited(flate2::read::ZlibDecoder::new(bytes));
    if zlib.is_ok() { zlib } else { read_limited(flate2::read::DeflateDecoder::new(bytes)) }
}

fn read_error(error: std::io::Error) -> FetchError {
    let message = error.to_string().to_ascii_lowercase();
    if error.kind() == std::io::ErrorKind::TimedOut || message.contains("timeout") || message.contains("timed out") {
        FetchError::Timeout
    } else {
        FetchError::NoConnection
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
        /// A `Content-Encoding` to announce, when the body is already compressed.
        pub encoding: Option<&'static str>,
    }

    pub fn page(content_type: &'static str, body: &str) -> Answer {
        Answer {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
            location: None,
            encoding: None,
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
                    // The encodings the client offered, so a test can read them back.
                    let offered = request.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("accept-encoding").then(|| value.trim().to_string())
                    });
                    let answer = if path == "/encoding" {
                        Answer {
                            status: 200,
                            content_type: "text/plain",
                            body: offered.unwrap_or_default().into_bytes(),
                            location: None,
                            encoding: None,
                        }
                    } else {
                        answers.lock().unwrap().get(&path).cloned().unwrap_or(Answer {
                            status: 404,
                            content_type: "text/html",
                            body: b"<html><body>Not Found</body></html>".to_vec(),
                            location: None,
                            encoding: None,
                        })
                    };
                    let mut head = format!("HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n", answer.status, answer.content_type, answer.body.len());
                    if let Some(location) = &answer.location {
                        head.push_str(&format!("Location: {location}\r\n"));
                    }
                    if let Some(encoding) = answer.encoding {
                        head.push_str(&format!("Content-Encoding: {encoding}\r\n"));
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
                encoding: None,
            },
        );
        server.set(
            "/broken",
            Answer {
                status: 500,
                content_type: "text/plain",
                body: b"no".to_vec(),
                location: None,
                encoding: None,
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
        // https, and nothing is listening on this port.
        assert_eq!(fetch("https://127.0.0.1:1/nope").err(), Some(FetchError::NoConnection));
        // A redirection may not leave http(s). file:// is not requested.
        server.set(
            "/to-file",
            Answer {
                status: 302,
                content_type: "text/plain",
                body: Vec::new(),
                location: Some("file:///etc/passwd".into()),
                encoding: None,
            },
        );
        assert_eq!(fetch(&server.url("/to-file")).err(), Some(FetchError::BadAddress));
        server.set(
            "/to-ftp",
            Answer {
                status: 302,
                content_type: "text/plain",
                body: Vec::new(),
                location: Some("ftp://127.0.0.1/x".into()),
                encoding: None,
            },
        );
        assert_eq!(fetch(&server.url("/to-ftp")).err(), Some(FetchError::BadAddress));
        // Eleven hops is one past the limit.
        server.set(
            "/loop",
            Answer {
                status: 302,
                content_type: "text/plain",
                body: Vec::new(),
                location: Some("/loop".into()),
                encoding: None,
            },
        );
        assert_eq!(fetch(&server.url("/loop")).err(), Some(FetchError::NoConnection));
    }

    #[test]
    fn fetch_decompresses_gzip_deflate_and_brotli() {
        let plain = b"<table><tr><td>ok</td></tr></table>";
        let server = Server::start();
        server.set("/gz", compressed("gzip", &gzip(plain)));
        server.set("/df", compressed("deflate", &zlib_wrap(plain)));
        server.set("/raw", compressed("deflate", &deflate_raw(plain)));
        server.set("/br", compressed("br", &brotli_compress(plain)));
        assert_eq!(fetch(&server.url("/gz")).unwrap().bytes, plain);
        assert_eq!(fetch(&server.url("/df")).unwrap().bytes, plain);
        assert_eq!(fetch(&server.url("/raw")).unwrap().bytes, plain);
        assert_eq!(fetch(&server.url("/br")).unwrap().bytes, plain);
        let offered = fetch(&server.url("/encoding")).unwrap();
        let offered = String::from_utf8(offered.bytes).unwrap();
        assert!(offered.split(',').any(|part| part.trim() == "gzip"), "{offered}");
        assert!(offered.split(',').any(|part| part.trim() == "deflate"), "{offered}");
        assert!(offered.split(',').any(|part| part.trim() == "br"), "{offered}");
    }

    fn compressed(encoding: &'static str, body: &[u8]) -> Answer {
        Answer {
            status: 200,
            content_type: "text/html",
            body: body.to_vec(),
            location: None,
            encoding: Some(encoding),
        }
    }

    fn gzip(plain: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain).unwrap();
        encoder.finish().unwrap()
    }

    fn zlib_wrap(plain: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain).unwrap();
        encoder.finish().unwrap()
    }

    fn deflate_raw(plain: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain).unwrap();
        encoder.finish().unwrap()
    }

    fn brotli_compress(plain: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        brotli::BrotliCompress(&mut &plain[..], &mut output, &brotli::enc::BrotliEncoderParams::default()).unwrap();
        output
    }

    #[test]
    fn web_fetch_refuses_local_files() {
        let dir = std::env::temp_dir().join(format!("opensheet-fetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secret.txt");
        std::fs::write(&path, b"TOP-SECRET").unwrap();
        let url = file_url(&path);
        assert!(names_a_file(&url) && names_a_file(path.to_str().unwrap()) && names_a_file("FILE:///etc/passwd"));
        assert!(!names_a_file("https://example.test/a") && !names_a_file("ftp://127.0.0.1/x"));
        // The web fetch never opens the file, whatever way it is written.
        assert_eq!(fetch(&url).err(), Some(FetchError::LocalFile));
        assert_eq!(fetch(path.to_str().unwrap()).err(), Some(FetchError::LocalFile));
        assert_eq!(fetch("file://localhost/etc/passwd").err(), Some(FetchError::LocalFile));
        assert_eq!(fetch("FILE:///etc/passwd").err(), Some(FetchError::LocalFile));
        assert_eq!(fetch("/nowhere/at/all.csv").err(), Some(FetchError::LocalFile));
        // The file import still reads a path the caller named.
        assert_eq!(read_local(&url).unwrap().bytes, b"TOP-SECRET");
        assert_eq!(read_local("/nowhere/at/all.csv").err(), Some(FetchError::Missing));
        std::fs::remove_dir_all(&dir).ok();
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
