//! Chart parts written back as they were read.
//!
//! A chart nobody changed is best saved as the text it came from. That
//! text was written for another package, though: before it goes into
//! ours it is checked for everything that would make Excel take the new
//! file for a damaged one.

use crate::xlsx::formula::from_file_text;
use crate::xlsx::xml::unescape;
use opensheet_engine::ast::Expr;
use std::borrow::Cow;

/// The namespaces of references to other parts of a package (`r:id`,
/// `r:embed`, ...), in the transitional and the strict flavour of the
/// format.
const RELATIONSHIPS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
];

enum Token<'a> {
    /// A start tag or an empty element, with its qualified name and its
    /// attributes as written.
    Open {
        name: &'a str,
        attrs: &'a str,
        empty: bool,
    },
    Close(&'a str),
    /// Character data, as written.
    Text(&'a str),
    /// A comment, a processing instruction, a CDATA section.
    Other,
    End,
}

/// A tokenizer that, unlike the reader of the package, knows where in the
/// text it is and forgives nothing.
struct Scanner<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    /// `None` when the text is not XML.
    fn next(&mut self) -> Option<Token<'a>> {
        let rest = &self.src[self.pos..];
        if rest.is_empty() {
            return Some(Token::End);
        }
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            self.pos += end;
            return Some(Token::Text(&rest[..end]));
        }
        for (start, end) in [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")] {
            if let Some(body) = rest.strip_prefix(start) {
                self.pos += start.len() + body.find(end)? + end.len();
                return Some(Token::Other);
            }
        }
        if rest.starts_with("<!") {
            // A document type: nothing a chart part has.
            return None;
        }
        // The end of the tag is the first `>` outside a quoted value.
        let mut quote = None;
        let end = rest.bytes().position(|b| {
            match quote {
                Some(q) if b == q => quote = None,
                Some(_) => {}
                None if b == b'"' || b == b'\'' => quote = Some(b),
                None => return b == b'>',
            }
            false
        })?;
        self.pos += end + 1;
        if let Some(name) = rest[..end].strip_prefix("</") {
            return Some(Token::Close(name.trim()));
        }
        let inner = &rest[1..end];
        let (inner, empty) = match inner.strip_suffix('/') {
            Some(inner) => (inner, true),
            None => (inner, false),
        };
        let name_end = inner.find(char::is_whitespace).unwrap_or(inner.len());
        if name_end == 0 {
            return None;
        }
        Some(Token::Open {
            name: &inner[..name_end],
            attrs: &inner[name_end..],
            empty,
        })
    }
}

/// The `name="value"` pairs of a tag, values as written.
fn attributes(mut rest: &str) -> impl Iterator<Item = (&str, &str)> {
    std::iter::from_fn(move || {
        let s = rest.trim_start();
        let eq = s.find('=')?;
        let after = s[eq + 1..].trim_start();
        let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let end = after[1..].find(quote)?;
        rest = &after[end + 2..];
        Some((s[..eq].trim_end(), &after[1..=end]))
    })
}

/// Prefix and local part of a qualified name.
fn split(name: &str) -> (&str, &str) {
    name.split_once(':').unwrap_or(("", name))
}

/// Whether a formula of a chart part reads nothing but ranges of sheets
/// that exist. Names and other workbooks are refused as well: nothing
/// says they will be in the file being written.
fn reads_known_sheets(formula: &str, sheet_exists: &dyn Fn(&str) -> bool) -> bool {
    let Some(expr) = from_file_text(&unescape(formula)) else {
        return false;
    };
    let mut known = true;
    expr.walk(&mut |node| match node {
        Expr::Ref(r) => known &= r.sheet.as_deref().is_some_and(sheet_exists),
        // `#REF!` is how Excel itself spells a range that is gone.
        Expr::Paren(_) | Expr::Error(_) => {}
        _ => known = false,
    });
    known
}

/// Whether the text says it is in an encoding other than the one it will
/// be written in.
fn declares_other_encoding(xml: &str) -> bool {
    let Some(declaration) = xml
        .trim_start_matches('\u{feff}')
        .strip_prefix("<?xml")
        .and_then(|rest| rest.split("?>").next())
    else {
        return false;
    };
    attributes(declaration)
        .any(|(key, value)| key == "encoding" && !value.eq_ignore_ascii_case("utf-8"))
}

/// The text of a chart part as it can go into the package being written,
/// or `None` when the chart has to be generated from the model instead
/// (the module documentation says when). The text is returned as it is,
/// but for `c:externalData` and `c:userShapes`, which are cut out.
pub(crate) fn reusable<'a>(
    xml: &'a str,
    sheet_exists: &dyn Fn(&str) -> bool,
) -> Option<Cow<'a, str>> {
    if declares_other_encoding(xml) {
        return None;
    }
    // A byte order mark is no text before the root element.
    let mut scanner = Scanner {
        src: xml,
        pos: xml.len() - xml.trim_start_matches('\u{feff}').len(),
    };
    let mut open: Vec<&str> = Vec::new();
    let mut root: Option<&str> = None;
    // Prefixes bound to a namespace of relationships, and the prefixes of
    // all the attributes that stay in the text.
    let mut relationship_prefixes: Vec<&str> = Vec::new();
    let mut attribute_prefixes: Vec<&str> = Vec::new();
    // Byte ranges to leave out, and the one being skipped: where it
    // starts and how many elements were open then.
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let mut cutting: Option<(usize, usize)> = None;
    let mut formula: Option<String> = None;
    loop {
        let start = scanner.pos;
        match scanner.next()? {
            Token::End => break,
            Token::Other => {}
            Token::Text(text) => match &mut formula {
                Some(formula) => formula.push_str(text),
                None if open.is_empty() && !text.trim().is_empty() => return None,
                None => {}
            },
            Token::Open { name, attrs, empty } => {
                let (prefix, local) = split(name);
                if open.is_empty() {
                    if root.is_some() || local != "chartSpace" {
                        return None;
                    }
                    root = Some(prefix);
                }
                let of_the_chart = open.len() == 1 && root == Some(prefix);
                let cut = cutting.is_none()
                    && of_the_chart
                    && matches!(local, "externalData" | "userShapes");
                if of_the_chart && local == "pivotSource" {
                    return None;
                }
                for (key, value) in attributes(attrs) {
                    match split(key) {
                        ("xmlns", bound) => {
                            if RELATIONSHIPS.contains(&&*unescape(value)) {
                                relationship_prefixes.push(bound);
                            }
                        }
                        (used, _) if cutting.is_none() && !cut && !used.is_empty() => {
                            attribute_prefixes.push(used)
                        }
                        _ => {}
                    }
                }
                if cut {
                    if empty {
                        cuts.push((start, scanner.pos));
                    } else {
                        cutting = Some((start, open.len()));
                    }
                } else if cutting.is_none() && matches!(local, "f" | "sqref") {
                    if empty {
                        return None;
                    }
                    formula = Some(String::new());
                }
                if !empty {
                    open.push(name);
                }
            }
            Token::Close(name) => {
                if open.pop() != Some(name) {
                    return None;
                }
                if let Some(formula) = formula.take()
                    && !reads_known_sheets(&formula, sheet_exists)
                {
                    return None;
                }
                if let Some((from, depth)) = cutting
                    && open.len() == depth
                {
                    cuts.push((from, scanner.pos));
                    cutting = None;
                }
            }
        }
    }
    if root.is_none() || !open.is_empty() {
        return None;
    }
    if attribute_prefixes
        .iter()
        .any(|prefix| relationship_prefixes.contains(prefix))
    {
        return None;
    }
    if cuts.is_empty() {
        return Some(Cow::Borrowed(xml));
    }
    let mut kept = String::with_capacity(xml.len());
    let mut at = 0;
    for (from, to) in cuts {
        kept.push_str(&xml[at..from]);
        at = to;
    }
    kept.push_str(&xml[at..]);
    Some(Cow::Owned(kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

    fn check(xml: &str) -> Option<String> {
        reusable(xml, &|sheet| sheet.eq_ignore_ascii_case("Data")).map(Cow::into_owned)
    }

    fn part(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace {NS}>{body}</c:chartSpace>"
        )
    }

    #[test]
    fn sound_parts_are_kept_as_they_are() {
        for body in [
            "",
            "<c:chart><c:plotArea><c:barChart><c:ser><c:val><c:numRef><c:f>Data!$A$1:$A$3</c:f></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart>",
            "<!-- a comment --><c:chart a='>' b=\"/>\"><c:f>(data!$A$1)</c:f><c:f>#REF!</c:f></c:chart>\n",
            "<c:clrMapOvr bg1=\"lt1\"/><c:chart/><c:printSettings><c:pageSetup/></c:printSettings>",
        ] {
            let xml = part(body);
            assert_eq!(check(&xml).as_deref(), Some(xml.as_str()), "{body}");
        }
        // A byte order mark, no declaration, another prefix.
        let bare = "\u{feff}<chartSpace xmlns=\"x\"><chart/></chartSpace>";
        assert_eq!(check(bare).as_deref(), Some(bare));
    }

    #[test]
    fn references_to_other_parts() {
        // Cut out where they add nothing to the picture.
        let xml = part(
            "<c:chart/><c:externalData r:id=\"rId1\"><c:autoUpdate val=\"0\"/></c:externalData><c:printSettings/><c:userShapes r:id=\"rId2\"/>",
        );
        assert_eq!(check(&xml), Some(part("<c:chart/><c:printSettings/>")));
        // Anywhere else, the chart is written from the model.
        for body in [
            "<c:chart><c:spPr><a:blipFill xmlns:a=\"a\"><a:blip r:embed=\"rId3\"/></a:blipFill></c:spPr></c:chart>",
            "<c:chart/><c:printSettings><c:legacyDrawingHF r:id=\"rId1\"/></c:printSettings>",
            "<c:chart><c:externalData r:id=\"rId1\"/></c:chart>",
            "<c:chart xmlns:rel=\"http://purl.oclc.org/ooxml/officeDocument/relationships\"><c:x rel:id=\"rId1\"/></c:chart>",
            "<c:pivotSource><c:name>[Book.xlsx]Data!Pivot</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart/>",
        ] {
            assert_eq!(check(&part(body)), None, "{body}");
        }
    }

    #[test]
    fn cells_that_cannot_be_read() {
        for formula in [
            "Gone!$A$1:$A$3",
            "[1]Data!$A$1",
            "SomeName",
            "Data!SomeName",
            "OFFSET(Data!$A$1,0,0,3,1)",
            "$A$1:$A$3",
            "",
        ] {
            let body = format!("<c:chart><c:f>{formula}</c:f></c:chart>");
            assert_eq!(check(&part(&body)), None, "{formula}");
        }
        let extension = "<c:chart><c15:fullRef xmlns:c15=\"x\"><c15:sqref>Gone!$A$1</c15:sqref></c15:fullRef></c:chart>";
        assert_eq!(check(&part(extension)), None);
        assert_eq!(check(&part("<c:chart><c:f/></c:chart>")), None);
    }

    #[test]
    fn text_that_is_no_chart_part() {
        let sound = part("<c:chart><c:f>Data!$A$1</c:f></c:chart>");
        assert!(check(&sound).is_some());
        // Cut short anywhere, it is not written back.
        for cut in 0..sound.len() - 1 {
            assert_eq!(check(&sound[..cut]), None, "{cut}");
        }
        for xml in [
            "",
            "not xml",
            "<worksheet/>",
            "<c:chartSpace><c:chart></c:chartSpace>",
            "<c:chartSpace></c:chart></c:chartSpace>",
            "<c:chartSpace/><c:chartSpace/>",
            "<c:chartSpace/>trailing",
            "<!DOCTYPE c:chartSpace><c:chartSpace/>",
            "<c:chartSpace><!-- unfinished</c:chartSpace>",
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?><c:chartSpace/>",
        ] {
            assert_eq!(check(xml), None, "{xml}");
        }
        assert!(check("<?xml version='1.0' encoding='utf-8'?><c:chartSpace/>").is_some());
    }
}
