//! Vector pictures: what the engine needs to know of an SVG document
//! without drawing it — that it is one, the size it asks for, and how to
//! repaint it (Graphics Fill and Graphics Outline change the document
//! itself, as in Excel, so that the file holds what the sheet shows).
//!
//! The document is handled as text: its tags are found and their `fill`,
//! `stroke` and `style` attributes rewritten; nothing else is touched.

use crate::style::Color;

/// True when the bytes are an SVG document: text whose first element is
/// `<svg>`.
pub fn is_svg(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    root_tag(text).is_some()
}

/// The tags of a document: (start, end) of each start tag, `<` and `>`
/// included; comments, declarations and end tags are passed over.
fn start_tags(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].find('<').map(|i| at + i) {
        let rest = &text[open..];
        if rest.starts_with("<!--") {
            at = rest.find("-->").map_or(text.len(), |i| open + i + 3);
        } else if rest.starts_with("<![CDATA[") {
            at = rest.find("]]>").map_or(text.len(), |i| open + i + 3);
        } else if rest.starts_with("<?") || rest.starts_with("<!") || rest.starts_with("</") {
            at = rest.find('>').map_or(text.len(), |i| open + i + 1);
        } else {
            // To the `>` that is not inside a quoted value.
            let mut quote = None;
            let mut end = text.len();
            for (i, byte) in bytes.iter().enumerate().skip(open + 1) {
                match (quote, *byte) {
                    (None, b'"' | b'\'') => quote = Some(*byte),
                    (Some(q), b) if b == q => quote = None,
                    (None, b'>') => {
                        end = i + 1;
                        break;
                    }
                    _ => {}
                }
            }
            if end <= text.len() && text[open..end].ends_with('>') {
                out.push((open, end));
            }
            at = end;
        }
    }
    out
}

/// The name of the element a start tag opens, without its prefix.
fn tag_name(tag: &str) -> &str {
    let name = tag[1..].split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or("");
    name.rsplit(':').next().unwrap_or(name)
}

fn root_tag(text: &str) -> Option<(usize, usize)> {
    let first = *start_tags(text).first()?;
    (tag_name(&text[first.0..first.1]) == "svg").then_some(first)
}

/// The attributes of a start tag: (name, start and end of the value
/// without its quotes, start of the name).
fn attributes(tag: &str) -> Vec<(&str, usize, usize, usize)> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    // After the element name.
    let mut at = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
    while at < bytes.len() {
        while at < bytes.len() && (bytes[at].is_ascii_whitespace() || bytes[at] == b'/') {
            at += 1;
        }
        let name_start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() && !matches!(bytes[at], b'=' | b'>' | b'/') {
            at += 1;
        }
        let name = &tag[name_start..at];
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if name.is_empty() || at >= bytes.len() || bytes[at] != b'=' {
            if name.is_empty() {
                at += 1;
            }
            continue;
        }
        at += 1;
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let Some(quote) = bytes.get(at).copied().filter(|b| matches!(b, b'"' | b'\'')) else {
            continue;
        };
        let value_start = at + 1;
        let value_end = tag[value_start..].find(quote as char).map_or(tag.len(), |i| value_start + i);
        out.push((name, value_start, value_end, name_start));
        at = value_end + 1;
    }
    out
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    attributes(tag).into_iter().find(|(n, ..)| *n == name).map(|(_, start, end, _)| &tag[start..end])
}

/// A length of a document in pixels at 96 per inch. `None` for a
/// percentage, which is a share of something the document does not say.
fn length(text: &str) -> Option<f32> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(text.len());
    // "em" ends in a letter a number may hold: the unit starts there.
    let split = if text[..split].ends_with(['e', 'E']) { split - 1 } else { split };
    let value: f32 = text[..split].parse().ok()?;
    let factor = match text[split..].trim() {
        "" | "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "mm" => 96.0 / 25.4,
        "cm" => 96.0 / 2.54,
        "in" => 96.0,
        "em" => 16.0,
        "ex" => 8.0,
        _ => return None,
    };
    let pixels = value * factor;
    (pixels.is_finite() && pixels > 0.0).then_some(pixels)
}

/// The size an SVG document has when inserted, in pixels at 100 %: its
/// `width` and `height`, else those of its view box. `None` when the bytes
/// are not an SVG document.
pub fn svg_size(data: &[u8]) -> Option<(f32, f32)> {
    let text = std::str::from_utf8(data).ok()?;
    let (start, end) = root_tag(text)?;
    let tag = &text[start..end];
    let view_box = attribute(tag, "viewBox").and_then(|value| {
        let numbers: Vec<f32> = value.split([' ', ',']).filter(|part| !part.is_empty()).filter_map(|part| part.parse().ok()).collect();
        match numbers[..] {
            [_, _, width, height] if width > 0.0 && height > 0.0 => Some((width, height)),
            _ => None,
        }
    });
    let width = attribute(tag, "width").and_then(length);
    let height = attribute(tag, "height").and_then(length);
    Some(match (width, height, view_box) {
        (Some(width), Some(height), _) => (width, height),
        // One side given: the other follows the proportions of the box.
        (Some(width), None, Some((w, h))) => (width, width * h / w),
        (None, Some(height), Some((w, h))) => (height * w / h, height),
        (_, _, Some(size)) => size,
        // What a browser gives a document that says nothing.
        (width, height, None) => (width.unwrap_or(300.0), height.unwrap_or(150.0)),
    })
}

/// The document asking for another size: it is then drawn stretched over
/// `width` × `height` pixels, whatever its proportions (how a picture
/// fills its frame). `None` when the bytes are not an SVG document.
pub fn resized(data: &[u8], width: f32, height: f32) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(data).ok()?;
    let (start, end) = root_tag(text)?;
    let tag = &text[start..end];
    let natural = svg_size(data)?;
    let mut has_view_box = false;
    let mut root = tag.to_string();
    // Out with what sizes the document now.
    for (name, _, value_end, name_start) in attributes(tag).into_iter().rev() {
        match name {
            "width" | "height" | "preserveAspectRatio" => {
                root.replace_range(name_start..value_end + 1, "");
            }
            "viewBox" => has_view_box = true,
            _ => {}
        }
    }
    let close = if root.ends_with("/>") { root.len() - 2 } else { root.len() - 1 };
    let mut added = format!(" width=\"{width}\" height=\"{height}\" preserveAspectRatio=\"none\"");
    if !has_view_box {
        added.push_str(&format!(" viewBox=\"0 0 {} {}\"", natural.0, natural.1));
    }
    root.insert_str(close, &added);
    Some(format!("{}{root}{}", &text[..start], &text[end..]).into_bytes())
}

/// What Graphics Fill and Graphics Outline do to a vector picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repaint {
    /// Every filled part in one colour; `None` fills nothing.
    Fill(Option<Color>),
    /// Every part outlined in one colour, the line `width` pixels of the
    /// document wide; `None` takes the outlines away.
    Outline(Option<(Color, f32)>),
}

impl Repaint {
    /// The properties to set, with their values.
    fn properties(self) -> Vec<(&'static str, String)> {
        let hex = |color: Color| format!("#{:06X}", color.value() & 0xFF_FFFF);
        match self {
            Repaint::Fill(color) => vec![("fill", color.map_or("none".to_string(), hex))],
            Repaint::Outline(Some((color, width))) => vec![("stroke", hex(color)), ("stroke-width", format!("{width}"))],
            Repaint::Outline(None) => vec![("stroke", "none".to_string())],
        }
    }

    /// Parts that are explicitly not filled stay so when the fill
    /// changes (line drawings keep their inside empty); an outline is
    /// given to everything.
    fn keeps(self, property: &str, value: &str) -> bool {
        matches!(self, Repaint::Fill(Some(_))) && property == "fill" && value.trim() == "none"
    }
}

/// Rewrite the declarations of a style (`fill:#123;stroke:none`).
fn repaint_declarations(style: &str, repaint: Repaint) -> String {
    let properties = repaint.properties();
    let mut out = String::with_capacity(style.len());
    let mut rest = style;
    // A declaration ends at `;` or, in a style sheet, at `}`.
    loop {
        let end = rest.find([';', '}', '{']).unwrap_or(rest.len());
        let declaration = &rest[..end];
        let replaced = declaration.split_once(':').and_then(|(name, value)| {
            let (_, new) = properties.iter().find(|(property, _)| *property == name.trim())?;
            (!repaint.keeps(name.trim(), value)).then(|| format!("{name}:{new}"))
        });
        out.push_str(replaced.as_deref().unwrap_or(declaration));
        if end == rest.len() {
            break;
        }
        out.push_str(&rest[end..end + 1]);
        rest = &rest[end + 1..];
    }
    out
}

fn repaint_tag(tag: &str, repaint: Repaint, root: bool) -> String {
    let properties = repaint.properties();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut seen = Vec::new();
    for (name, start, end, _) in attributes(tag) {
        let value = &tag[start..end];
        if name == "style" {
            edits.push((start, end, repaint_declarations(value, repaint)));
        } else if let Some((property, new)) = properties.iter().find(|(property, _)| *property == name) {
            seen.push(*property);
            if !repaint.keeps(name, value) {
                edits.push((start, end, new.clone()));
            }
        }
    }
    let mut out = tag.to_string();
    for (start, end, new) in edits.into_iter().rev() {
        out.replace_range(start..end, &new);
    }
    if root {
        // What the parts that say nothing inherit.
        let close = if out.ends_with("/>") { out.len() - 2 } else { out.len() - 1 };
        let mut added = String::new();
        for (property, value) in properties.iter().filter(|(property, _)| !seen.contains(property)) {
            added.push_str(&format!(" {property}=\"{value}\""));
        }
        out.insert_str(close, &added);
    }
    out
}

/// An SVG document repainted. `None` when the bytes are not one.
pub fn repaint(data: &[u8], repaint: Repaint) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(data).ok()?;
    let (root, _) = root_tag(text)?;
    let tags = start_tags(text);
    let mut out = String::with_capacity(text.len() + 32);
    let mut at = 0;
    for (i, (start, end)) in tags.iter().enumerate() {
        out.push_str(&text[at..*start]);
        let tag = &text[*start..*end];
        out.push_str(&repaint_tag(tag, repaint, *start == root));
        at = *end;
        // The rules of a style sheet, up to the next tag.
        if tag_name(tag) == "style" && !tag.ends_with("/>") {
            let until = tags.get(i + 1).map_or(text.len(), |next| next.0);
            let close = text[at..until].find("</").map_or(until, |found| at + found);
            out.push_str(&repaint_declarations(&text[at..close], repaint));
            at = close;
        }
    }
    out.push_str(&text[at..]);
    Some(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICON: &str = r##"<?xml version="1.0"?><!-- an icon --><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48" width="2in"><style>.a{fill:#123456;stroke:red}</style><path class="a" d="M0 0h9"/><circle r="4" fill='blue' style="fill-opacity:.5;fill:green"/><rect fill="none" stroke-width="3" width="5" height="5"/></svg>"##;

    #[test]
    fn documents_are_recognised_and_measured() {
        assert!(is_svg(ICON.as_bytes()));
        assert!(!is_svg(b"<html><svg/></html>") && !is_svg(b"\x89PNG\r\n\x1a\n") && !is_svg(b"svg"));
        // A width alone: the height follows the view box.
        assert_eq!(svg_size(ICON.as_bytes()), Some((192.0, 96.0)));
        let sized = |attributes: &str| svg_size(format!("<svg {attributes}/>").as_bytes());
        assert_eq!(sized(r#"width="72pt" height="25.4mm""#), Some((96.0, 96.0)));
        assert_eq!(sized(r#"viewBox="0,0,24,12""#), Some((24.0, 12.0)));
        assert_eq!(sized(r#"width="100%" height="50%" viewBox="0 0 10 20""#), Some((10.0, 20.0)));
        assert_eq!(sized(r#"width="1.5em""#), Some((24.0, 150.0)));
        assert_eq!(sized(""), Some((300.0, 150.0)));
        assert_eq!(svg_size(b"<svgx/>"), None);
        // A `>` inside a value does not end its tag.
        assert_eq!(svg_size(br#"<svg:svg data-x="a>b" width='10' height="20">"#), Some((10.0, 20.0)));
    }

    #[test]
    fn documents_are_stretched_over_a_frame() {
        let text = |data: &[u8], w, h| String::from_utf8(resized(data, w, h).unwrap()).unwrap();
        let wide = text(ICON.as_bytes(), 300.0, 20.0);
        assert!(wide.contains(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48"  width="300" height="20" preserveAspectRatio="none">"#), "{wide}");
        assert_eq!(svg_size(wide.as_bytes()), Some((300.0, 20.0)));
        // A document without a view box gets the one its size implies.
        let bare = text(br#"<svg height="10" width="40"><rect/></svg>"#, 8.0, 8.0);
        assert_eq!(bare, r#"<svg   width="8" height="8" preserveAspectRatio="none" viewBox="0 0 40 10"><rect/></svg>"#);
        assert_eq!(resized(b"<p/>", 1.0, 1.0), None);
    }

    #[test]
    fn fills_and_outlines_are_repainted() {
        let text = |repaint_as| String::from_utf8(repaint(ICON.as_bytes(), repaint_as).unwrap()).unwrap();
        let red = text(Repaint::Fill(Some(Color(0xC00000))));
        // The root, the style sheet, attributes and inline styles; what is
        // not filled stays empty, and nothing else moves.
        assert!(red.contains(r##"width="2in" fill="#C00000">"##), "{red}");
        assert!(red.contains(".a{fill:#C00000;stroke:red}"));
        assert!(red.contains(r##"fill='#C00000' style="fill-opacity:.5;fill:#C00000""##));
        assert!(red.contains(r#"<rect fill="none" stroke-width="3""#));
        assert!(red.starts_with(r#"<?xml version="1.0"?><!-- an icon --><svg"#) && red.ends_with("</svg>"));
        // No fill empties everything.
        let empty = text(Repaint::Fill(None));
        assert_eq!(empty.matches("none").count(), 5, "{empty}");
        let outlined = text(Repaint::Outline(Some((Color(0x0070C0), 1.5))));
        assert!(outlined.contains(r##"width="2in" stroke="#0070C0" stroke-width="1.5">"##), "{outlined}");
        assert!(outlined.contains(".a{fill:#123456;stroke:#0070C0}") && outlined.contains(r#"stroke-width="1.5" width="5""#));
        let bare = text(Repaint::Outline(None));
        assert!(bare.contains(r#"width="2in" stroke="none">"#) && bare.contains("stroke:none}"));
        // Repainting twice is repainting once.
        let twice = String::from_utf8(repaint(red.as_bytes(), Repaint::Fill(Some(Color(0xC00000)))).unwrap()).unwrap();
        assert_eq!(twice, red);
        assert_eq!(repaint(b"not a picture", Repaint::Fill(None)), None);
        // A document closed in its own tag.
        let single = String::from_utf8(repaint(b"<svg width='1'/>", Repaint::Fill(None)).unwrap()).unwrap();
        assert_eq!(single, "<svg width='1' fill=\"none\"/>");
    }
}
