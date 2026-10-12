//! A small, allocation-light pull parser for the XML found in OOXML
//! packages, plus the escaping helpers used by the writer.
//!
//! The parser is deliberately lenient: namespace prefixes are dropped from
//! element names (`x:row` reads as `row`), unknown entities are kept as
//! written, and truncated input simply ends the event stream. It never
//! panics on malformed input.

use std::borrow::Cow;

pub(crate) const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// A start tag (`<a b="c">`) or an empty element (`<a b="c"/>`).
#[derive(Clone, Copy)]
pub(crate) struct Tag<'a> {
    /// Element name without its namespace prefix.
    pub name: &'a str,
    attrs: &'a str,
    /// True for `<a/>`: no content and no matching `Close` event follows.
    pub empty: bool,
}

pub(crate) enum Event<'a> {
    Open(Tag<'a>),
    /// End tag; the name has no namespace prefix.
    Close(&'a str),
    /// Character data, entities resolved.
    Text(Cow<'a, str>),
    Eof,
}

/// Iterator over the raw `name="value"` pairs of a tag.
pub(crate) struct Attrs<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Attrs<'a> {
    /// (qualified name, raw value with entities still escaped)
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        let s = self.rest.trim_start();
        let eq = s.find('=')?;
        let key = s[..eq].trim_end();
        let after = s[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let body = &after[1..];
        let end = body.find(quote)?;
        self.rest = &body[end + 1..];
        Some((key, &body[..end]))
    }
}

fn local_name(name: &str) -> &str {
    match name.rfind(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

impl<'a> Tag<'a> {
    pub fn attrs(&self) -> Attrs<'a> {
        Attrs { rest: self.attrs }
    }

    /// Value of the attribute with exactly this (qualified) name.
    pub fn attr(&self, name: &str) -> Option<Cow<'a, str>> {
        self.attrs()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| unescape(v))
    }

    /// Value of the first attribute whose name, ignoring any namespace
    /// prefix, is `name` (`r:id` matches `id`).
    pub fn attr_local(&self, name: &str) -> Option<Cow<'a, str>> {
        self.attrs()
            .find(|(k, _)| local_name(k) == name)
            .map(|(_, v)| unescape(v))
    }

    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        self.attrs()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| parse_bool(v))
    }

    pub fn attr_f64(&self, name: &str) -> Option<f64> {
        self.attrs()
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v.trim().parse::<f64>().ok())
            .filter(|n| n.is_finite())
    }

    pub fn attr_u32(&self, name: &str) -> Option<u32> {
        self.attrs()
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v.trim().parse::<u32>().ok())
    }
}

/// `xsd:boolean` as written by spreadsheet applications.
pub(crate) fn parse_bool(v: &str) -> bool {
    matches!(v.trim(), "1" | "true" | "TRUE" | "True" | "on")
}

pub(crate) struct Reader<'a> {
    src: &'a str,
    pos: usize,
}

/// Index of the `>` closing the tag that starts at the beginning of `s`,
/// ignoring any `>` inside quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = 0u8;
    for (i, &b) in s.as_bytes().iter().enumerate().skip(1) {
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
        } else if b == b'"' || b == b'\'' {
            quote = b;
        } else if b == b'>' {
            return Some(i);
        }
    }
    None
}

impl<'a> Reader<'a> {
    pub fn new(src: &'a str) -> Reader<'a> {
        Reader {
            src: src.strip_prefix('\u{feff}').unwrap_or(src),
            pos: 0,
        }
    }

    fn finish(&mut self) -> Event<'a> {
        self.pos = self.src.len();
        Event::Eof
    }

    /// What is left to read. A second reader over it reads the same
    /// events as this one would.
    pub fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    // Not an `Iterator`: the stream ends with an explicit `Eof` event, which
    // keeps the `match` loops of the callers flat.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Event<'a> {
        loop {
            let rest = &self.src[self.pos..];
            if rest.is_empty() {
                return Event::Eof;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.pos += end;
                return Event::Text(unescape(&rest[..end]));
            }
            if let Some(body) = rest.strip_prefix("<!--") {
                match body.find("-->") {
                    Some(i) => self.pos += 4 + i + 3,
                    None => return self.finish(),
                }
                continue;
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                return match body.find("]]>") {
                    Some(i) => {
                        self.pos += 9 + i + 3;
                        Event::Text(Cow::Borrowed(&body[..i]))
                    }
                    None => self.finish(),
                };
            }
            if rest.starts_with("<?") {
                match rest.find("?>") {
                    Some(i) => self.pos += i + 2,
                    None => return self.finish(),
                }
                continue;
            }
            if rest.starts_with("<!") {
                // DOCTYPE, possibly with an internal subset in brackets.
                let mut depth = 0i32;
                let mut end = None;
                for (i, b) in rest.bytes().enumerate() {
                    match b {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => {
                            end = Some(i);
                            break;
                        }
                        _ => {}
                    }
                }
                match end {
                    Some(i) => self.pos += i + 1,
                    None => return self.finish(),
                }
                continue;
            }
            if let Some(body) = rest.strip_prefix("</") {
                return match body.find('>') {
                    Some(i) => {
                        self.pos += 2 + i + 1;
                        Event::Close(local_name(body[..i].trim()))
                    }
                    None => self.finish(),
                };
            }
            let Some(end) = tag_end(rest) else {
                return self.finish();
            };
            self.pos += end + 1;
            let inner = &rest[1..end];
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(i) => (i, true),
                None => (inner, false),
            };
            let name_end = inner
                .find(|c: char| c.is_whitespace())
                .unwrap_or(inner.len());
            return Event::Open(Tag {
                name: local_name(&inner[..name_end]),
                attrs: &inner[name_end..],
                empty,
            });
        }
    }

    /// Skip to the end of the element opened by `tag`.
    pub fn skip(&mut self, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        let mut depth = 0usize;
        loop {
            match self.next() {
                Event::Open(t) if !t.empty => depth += 1,
                Event::Close(_) => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                Event::Eof => return,
                _ => {}
            }
        }
    }

    /// The character data inside the element opened by `tag` (including
    /// that of nested elements), consuming its end tag.
    pub fn text(&mut self, tag: &Tag<'a>) -> Cow<'a, str> {
        if tag.empty {
            return Cow::Borrowed("");
        }
        let mut acc: Option<Cow<'a, str>> = None;
        let mut depth = 0usize;
        loop {
            match self.next() {
                Event::Text(t) => match &mut acc {
                    None => acc = Some(t),
                    Some(a) => a.to_mut().push_str(&t),
                },
                Event::Open(t) => {
                    if !t.empty {
                        depth += 1;
                    }
                }
                Event::Close(_) => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                Event::Eof => break,
            }
        }
        acc.unwrap_or(Cow::Borrowed(""))
    }
}

/// Resolve entity and character references and normalise line ends
/// (`\r\n` and lone `\r` read as `\n`, as XML requires).
pub(crate) fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains(['&', '\r']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(['&', '\r']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix('\r') {
            out.push('\n');
            rest = after.strip_prefix('\n').unwrap_or(after);
            continue;
        }
        let resolved = tail[1..].find(';').filter(|j| *j <= 10).and_then(|j| {
            let name = &tail[1..1 + j];
            let ch = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix('#')
                    .and_then(|n| match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse::<u32>().ok(),
                    })
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, j + 2))
        });
        match resolved {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Characters XML 1.0 cannot represent at all.
fn is_illegal(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')
}

/// Append `s` as element content.
pub(crate) fn push_text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if is_illegal(c) => {}
            c => out.push(c),
        }
    }
}

/// Append `s` as an attribute value (without the surrounding quotes).
pub(crate) fn push_attr(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            c if is_illegal(c) => {}
            c => out.push(c),
        }
    }
}

/// True when `s` starts with `xHHHH_`, i.e. a preceding `_` would read as
/// an OOXML character escape.
fn looks_like_escape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 6 && b[0] == b'x' && b[1..5].iter().all(|c| c.is_ascii_hexdigit()) && b[5] == b'_'
}

/// Append cell text as element content, using the SpreadsheetML `_xHHHH_`
/// convention for characters XML cannot carry (control characters, and
/// carriage returns, which a parser would otherwise turn into line feeds).
pub(crate) fn push_cell_text(out: &mut String, s: &str) {
    for (i, c) in s.char_indices() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '_' if looks_like_escape(&s[i + 1..]) => out.push_str("_x005F_"),
            '\r' => out.push_str("_x000D_"),
            c if is_illegal(c) => {
                out.push_str(&format!("_x{:04X}_", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// Append user text (a title, a message) as an attribute value, without
/// the surrounding quotes. Such attributes follow the `_xHHHH_` convention
/// of cell text for what an attribute cannot carry as it is: line breaks,
/// tabs and control characters.
pub(crate) fn push_xstring_attr(out: &mut String, s: &str) {
    for (i, c) in s.char_indices() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '_' if looks_like_escape(&s[i + 1..]) => out.push_str("_x005F_"),
            c if is_illegal(c) || matches!(c, '\n' | '\r' | '\t') => {
                out.push_str(&format!("_x{:04X}_", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// Decode the `_xHHHH_` escapes of SpreadsheetML strings.
pub(crate) fn decode_cell_text(s: &str) -> Cow<'_, str> {
    if !s.contains("_x") {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("_x") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded = if looks_like_escape(&tail[1..]) {
            u32::from_str_radix(&tail[2..6], 16)
                .ok()
                .and_then(char::from_u32)
        } else {
            None
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[7..];
            }
            None => {
                out.push('_');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(src: &str) -> String {
        let mut r = Reader::new(src);
        let mut out = String::new();
        loop {
            match r.next() {
                Event::Open(t) => {
                    out.push_str(&format!("<{}", t.name));
                    for (k, v) in t.attrs() {
                        out.push_str(&format!(" {k}={}", unescape(v)));
                    }
                    out.push_str(if t.empty { "/>" } else { ">" });
                }
                Event::Close(n) => out.push_str(&format!("</{n}>")),
                Event::Text(t) => out.push_str(&format!("[{t}]")),
                Event::Eof => return out,
            }
        }
    }

    #[test]
    fn events() {
        let src = "\u{feff}<?xml version=\"1.0\"?>\n<!-- c --><x:a p='1' q = \"a&gt;b>c\"><b/>t &amp; &#65;&#x42;<![CDATA[<raw>]]></x:a>";
        assert_eq!(dump(src), "[\n]<a p=1 q=a>b>c><b/>[t & AB][<raw>]</a>");
    }

    #[test]
    fn truncated_input_does_not_panic() {
        for src in [
            "<a",
            "<a b=\"",
            "<!-- x",
            "<![CDATA[x",
            "</a",
            "<a>&#xZZ;&",
            "<?xml",
            "<!DOCTYPE a [",
            "é<é à='1'>",
        ] {
            dump(src);
        }
    }

    #[test]
    fn text_and_skip() {
        let mut r = Reader::new("<r><a><b>1</b>2</a><c>x<d>y</d>z</c><e/></r>");
        let Event::Open(_) = r.next() else { panic!() };
        let Event::Open(a) = r.next() else { panic!() };
        r.skip(&a);
        let Event::Open(c) = r.next() else { panic!() };
        assert_eq!(r.text(&c), "xyz");
        let Event::Open(e) = r.next() else { panic!() };
        assert_eq!(r.text(&e), "");
        assert!(matches!(r.next(), Event::Close("r")));
    }

    #[test]
    fn attributes() {
        let mut r =
            Reader::new("<sheet name=\"A &amp; B\" sheetId=\"3\" r:id=\"rId7\" hidden=\"true\"/>");
        let Event::Open(t) = r.next() else { panic!() };
        assert_eq!(t.attr("name").unwrap(), "A & B");
        assert_eq!(t.attr_u32("sheetId"), Some(3));
        assert_eq!(t.attr("id"), None);
        assert_eq!(t.attr_local("id").unwrap(), "rId7");
        assert_eq!(t.attr_bool("hidden"), Some(true));
        assert_eq!(t.attr_bool("missing"), None);
    }

    #[test]
    fn line_ends() {
        assert_eq!(unescape("a\r\nb\rc&#13;"), "a\nb\nc\r");
    }

    #[test]
    fn cell_text_escapes() {
        for s in [
            "plain",
            "a\rb\n\tc",
            "_x000D_ literal",
            "bell\u{7}",
            "<&>",
            "_x",
            "__x0041_",
        ] {
            let mut enc = String::new();
            push_cell_text(&mut enc, s);
            assert_eq!(decode_cell_text(&unescape(&enc)), s, "{enc}");
        }
        assert_eq!(decode_cell_text("a_x000D__x000A_b"), "a\r\nb");
    }

    #[test]
    fn text_attributes() {
        for s in [
            "plain",
            "two\nlines\r\nand\ta tab",
            "\"quoted\" <&>",
            "_x000A_ literal",
            "bell\u{7}",
        ] {
            let mut enc = String::new();
            push_xstring_attr(&mut enc, s);
            assert!(!enc.contains(['\n', '\r', '\t', '"', '<']), "{enc}");
            assert_eq!(decode_cell_text(&unescape(&enc)), s, "{enc}");
        }
        let mut enc = String::new();
        push_xstring_attr(&mut enc, "a\nb");
        assert_eq!(enc, "a_x000A_b");
        // Excel writes the escapes in lower case.
        assert_eq!(decode_cell_text("a_x000a_b"), "a\nb");
    }
}
