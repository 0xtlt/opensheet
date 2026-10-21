//! Equations: Office Math (`m:oMath`) in the text of a shape.
//!
//! Excel writes a shape that holds an equation twice, in a choice
//! (`mc:AlternateContent`): once with the equation, in an `a14:m` element
//! of its paragraph, for applications that know it; once with the
//! equation as plain text, for the others. Both are written here, and the
//! first is read.
//!
//! Kept: runs, fractions (with or without bar), scripts, radicals, n-ary
//! operators, delimiters, functions, accents, bars, limits and matrices.
//! Of what the engine has no structure for (boxes, grouping characters,
//! equation arrays, phantoms, scripts before their base), the content is
//! kept and the structure dropped. The look of single runs (bold, script
//! letters) is not kept.

use super::*;
use opensheet_engine::math::{
    Equation, Math, PLACEHOLDER, Row, limits_beside, parse, to_linear, tokens,
};

const NS_MATH: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
pub(super) const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
pub(super) const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

// ---- writing -------------------------------------------------------------------

struct Writer<'a> {
    /// The properties every run carries: size, slant, colour, font.
    run: &'a str,
}

impl Writer<'_> {
    fn text(&self, x: &mut String, text: &str, upright: bool) {
        x.push_str("<m:r>");
        if upright {
            x.push_str("<m:rPr><m:sty m:val=\"p\"/></m:rPr>");
        }
        x.push_str(self.run);
        x.push_str("<m:t>");
        push_text(x, text);
        x.push_str("</m:t></m:r>");
    }

    /// A row inside an element (`m:e`, `m:num`…). An empty place is an
    /// empty element.
    fn part(&self, x: &mut String, name: &str, row: &Row) {
        if row.is_empty()
            || matches!(row.as_slice(), [Math::Text(text)] if text.chars().eq([PLACEHOLDER]))
        {
            w!(x, "<m:{name}/>");
            return;
        }
        w!(x, "<m:{name}>");
        self.row(x, row, false);
        w!(x, "</m:{name}>");
    }

    fn optional(&self, x: &mut String, name: &str, row: &Option<Row>) {
        match row {
            Some(row) => self.part(x, name, row),
            None => {
                w!(x, "<m:{name}/>");
            }
        }
    }

    fn row(&self, x: &mut String, row: &Row, upright: bool) {
        // Characters side by side are one run.
        let mut run = String::new();
        for node in row {
            if let Math::Text(text) = node {
                run.push_str(text);
                continue;
            }
            if !run.is_empty() {
                self.text(x, &std::mem::take(&mut run), upright);
            }
            self.node(x, node);
        }
        if !run.is_empty() {
            self.text(x, &run, upright);
        }
    }

    fn node(&self, x: &mut String, node: &Math) {
        match node {
            Math::Text(text) => self.text(x, text, false),
            Math::Fraction { num, den, bar } => {
                x.push_str("<m:f>");
                if !*bar {
                    x.push_str("<m:fPr><m:type m:val=\"noBar\"/></m:fPr>");
                }
                self.part(x, "num", num);
                self.part(x, "den", den);
                x.push_str("</m:f>");
            }
            Math::Script { base, sub, sup } => {
                let name = match (sub, sup) {
                    (Some(_), Some(_)) => "sSubSup",
                    (Some(_), None) => "sSub",
                    _ => "sSup",
                };
                w!(x, "<m:{name}>");
                self.part(x, "e", base);
                if let Some(sub) = sub {
                    self.part(x, "sub", sub);
                }
                if let Some(sup) = sup {
                    self.part(x, "sup", sup);
                }
                w!(x, "</m:{name}>");
            }
            Math::Radical { degree, body } => {
                x.push_str("<m:rad>");
                if degree.is_none() {
                    x.push_str("<m:radPr><m:degHide m:val=\"1\"/></m:radPr>");
                }
                self.optional(x, "deg", degree);
                self.part(x, "e", body);
                x.push_str("</m:rad>");
            }
            Math::Nary {
                op,
                sub,
                sup,
                limits_beside,
                body,
            } => {
                x.push_str("<m:nary><m:naryPr>");
                // The integral sign is the operator of an element that
                // names none.
                if *op != '∫' {
                    w!(x, "<m:chr m:val=\"{op}\"/>");
                }
                w!(
                    x,
                    "<m:limLoc m:val=\"{}\"/>",
                    if *limits_beside { "subSup" } else { "undOvr" }
                );
                if sub.is_none() {
                    x.push_str("<m:subHide m:val=\"1\"/>");
                }
                if sup.is_none() {
                    x.push_str("<m:supHide m:val=\"1\"/>");
                }
                x.push_str("</m:naryPr>");
                self.optional(x, "sub", sub);
                self.optional(x, "sup", sup);
                self.part(x, "e", body);
                x.push_str("</m:nary>");
            }
            Math::Delimited { open, close, items } => {
                x.push_str("<m:d>");
                // Round brackets are those of an element that names none.
                if (*open, *close) != (Some('('), Some(')')) {
                    x.push_str("<m:dPr><m:begChr m:val=\"");
                    push_attr(x, &open.map(String::from).unwrap_or_default());
                    x.push_str("\"/><m:endChr m:val=\"");
                    push_attr(x, &close.map(String::from).unwrap_or_default());
                    x.push_str("\"/></m:dPr>");
                }
                for item in items {
                    self.part(x, "e", item);
                }
                x.push_str("</m:d>");
            }
            Math::Function { name, arg } => {
                x.push_str("<m:func><m:fName>");
                self.name(x, name);
                x.push_str("</m:fName>");
                self.part(x, "e", arg);
                x.push_str("</m:func>");
            }
            Math::Accent { mark, body } => {
                w!(x, "<m:acc><m:accPr><m:chr m:val=\"{mark}\"/></m:accPr>");
                self.part(x, "e", body);
                x.push_str("</m:acc>");
            }
            Math::Bar { over, body } => {
                w!(
                    x,
                    "<m:bar><m:barPr><m:pos m:val=\"{}\"/></m:barPr>",
                    if *over { "top" } else { "bot" }
                );
                self.part(x, "e", body);
                x.push_str("</m:bar>");
            }
            Math::Limit { base, limit, upper } => self.limit(x, base, limit, *upper, false),
            Math::Matrix { rows } => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
                w!(
                    x,
                    "<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val=\"{columns}\"/><m:mcJc m:val=\"center\"/></m:mcPr></m:mc></m:mcs></m:mPr>"
                );
                for row in rows {
                    x.push_str("<m:mr>");
                    for cell in row {
                        self.part(x, "e", cell);
                    }
                    x.push_str("</m:mr>");
                }
                x.push_str("</m:m>");
            }
        }
    }

    fn limit(&self, x: &mut String, base: &Row, limit: &Row, upper: bool, upright: bool) {
        let name = if upper { "limUpp" } else { "limLow" };
        w!(x, "<m:{name}><m:e>");
        self.row(x, base, upright);
        x.push_str("</m:e>");
        self.part(x, "lim", limit);
        w!(x, "</m:{name}>");
    }

    /// The name of a function: upright, with what is attached to it.
    fn name(&self, x: &mut String, name: &Row) {
        for node in name {
            match node {
                Math::Text(text) => self.text(x, text, true),
                Math::Script { base, sub, sup } => {
                    let element = match (sub, sup) {
                        (Some(_), Some(_)) => "sSubSup",
                        (Some(_), None) => "sSub",
                        _ => "sSup",
                    };
                    w!(x, "<m:{element}><m:e>");
                    self.row(x, base, true);
                    x.push_str("</m:e>");
                    if let Some(sub) = sub {
                        self.part(x, "sub", sub);
                    }
                    if let Some(sup) = sup {
                        self.part(x, "sup", sup);
                    }
                    w!(x, "</m:{element}>");
                }
                Math::Limit { base, limit, upper } => self.limit(x, base, limit, *upper, true),
                other => self.node(x, other),
            }
        }
    }
}

/// The paragraph of a shape that holds an equation: `<a14:m>` and the
/// properties its end has.
pub(super) fn push_paragraph(x: &mut String, equation: &Equation, font: &Font) {
    let mut run = format!("<a:rPr sz=\"{}\" i=\"1\">", font.size as u32 * 50);
    if let Some(color) = font.color {
        push_solid_fill(&mut run, color);
    }
    run.push_str("<a:latin typeface=\"Cambria Math\" panose=\"02040503050406030204\" pitchFamily=\"18\" charset=\"0\"/></a:rPr>");
    let writer = Writer { run: &run };
    w!(
        x,
        "<a14:m><m:oMathPara xmlns:m=\"{NS_MATH}\"><m:oMathParaPr><m:jc m:val=\"centerGroup\"/></m:oMathParaPr><m:oMath xmlns:m=\"{NS_MATH}\">"
    );
    if equation.linear {
        // Shown as it is typed: characters, no structure.
        writer.text(x, &equation.to_linear(), false);
    } else {
        writer.row(x, &equation.body, false);
    }
    w!(
        x,
        "</m:oMath></m:oMathPara></a14:m><a:endParaRPr sz=\"{}\"/>",
        font.size as u32 * 50
    );
}

// ---- reading -------------------------------------------------------------------

fn value<'a>(tag: &Tag<'a>) -> Option<String> {
    tag.attr_local("val").map(|value| value.into_owned())
}

/// A flag of the properties of a structure: present means on, unless it
/// says otherwise.
fn flag(tag: &Tag) -> bool {
    !matches!(value(tag).as_deref(), Some("0" | "false" | "off"))
}

fn first_char(text: Option<String>) -> Option<char> {
    text.and_then(|text| text.chars().next())
}

/// The row an element holds; an empty place is a placeholder.
fn part<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Row {
    let found = row(r, tag);
    if found.is_empty() {
        vec![Math::Text(PLACEHOLDER.to_string())]
    } else {
        found
    }
}

/// Letters side by side as one word: the name of a function.
fn as_name(row: Row) -> Row {
    let mut out: Row = Vec::new();
    for node in row {
        match node {
            Math::Text(text) => match out.last_mut() {
                Some(Math::Text(word)) => word.push_str(&text),
                _ => out.push(Math::Text(text)),
            },
            Math::Script { base, sub, sup } => out.push(Math::Script {
                base: as_name(base),
                sub,
                sup,
            }),
            Math::Limit { base, limit, upper } => out.push(Math::Limit {
                base: as_name(base),
                limit,
                upper,
            }),
            other => out.push(other),
        }
    }
    out
}

fn row<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Row {
    let mut out = Vec::new();
    each_child(r, tag, |r, t| match t.name {
        "r" => each_child(r, t, |r, c| match c.name {
            "t" => out.extend(tokens(&printable(&r.text(c), false))),
            _ => r.skip(c),
        }),
        "f" => {
            let (mut num, mut den, mut bar) = (Vec::new(), Vec::new(), true);
            each_child(r, t, |r, c| match c.name {
                "fPr" => each_child(r, c, |r, p| {
                    if p.name == "type" {
                        bar = value(p).as_deref() != Some("noBar");
                    }
                    r.skip(p);
                }),
                "num" => num = part(r, c),
                "den" => den = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Fraction { num, den, bar });
        }
        "sSub" | "sSup" | "sSubSup" | "sPre" => {
            let (mut base, mut sub, mut sup) = (Vec::new(), None, None);
            each_child(r, t, |r, c| match c.name {
                "e" => base = part(r, c),
                "sub" => sub = Some(part(r, c)),
                "sup" => sup = Some(part(r, c)),
                _ => r.skip(c),
            });
            out.push(Math::Script { base, sub, sup });
        }
        "rad" => {
            let (mut degree, mut body, mut hidden) = (None, Vec::new(), false);
            each_child(r, t, |r, c| match c.name {
                "radPr" => each_child(r, c, |r, p| {
                    if p.name == "degHide" {
                        hidden = flag(p);
                    }
                    r.skip(p);
                }),
                "deg" => degree = Some(part(r, c)),
                "e" => body = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Radical {
                degree: degree.filter(|_| !hidden),
                body,
            });
        }
        "nary" => {
            let (mut op, mut beside, mut sub, mut sup, mut body) =
                ('∫', None, None, None, Vec::new());
            let (mut no_sub, mut no_sup) = (false, false);
            each_child(r, t, |r, c| match c.name {
                "naryPr" => each_child(r, c, |r, p| {
                    match p.name {
                        "chr" => op = first_char(value(p)).unwrap_or(op),
                        "limLoc" => beside = Some(value(p).as_deref() == Some("subSup")),
                        "subHide" => no_sub = flag(p),
                        "supHide" => no_sup = flag(p),
                        _ => {}
                    }
                    r.skip(p);
                }),
                "sub" => sub = Some(part(r, c)),
                "sup" => sup = Some(part(r, c)),
                "e" => body = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Nary {
                op,
                sub: sub.filter(|_| !no_sub),
                sup: sup.filter(|_| !no_sup),
                limits_beside: beside.unwrap_or_else(|| limits_beside(op)),
                body,
            });
        }
        "d" => {
            let (mut open, mut close, mut items) = (Some('('), Some(')'), Vec::new());
            each_child(r, t, |r, c| match c.name {
                "dPr" => each_child(r, c, |r, p| {
                    match p.name {
                        "begChr" => open = first_char(value(p)),
                        "endChr" => close = first_char(value(p)),
                        _ => {}
                    }
                    r.skip(p);
                }),
                "e" => items.push(part(r, c)),
                _ => r.skip(c),
            });
            // Without delimiters it holds its content and nothing else.
            if open.is_none() && close.is_none() {
                out.extend(items.into_iter().flatten());
            } else {
                out.push(Math::Delimited { open, close, items });
            }
        }
        "func" => {
            let (mut name, mut arg) = (Vec::new(), Vec::new());
            each_child(r, t, |r, c| match c.name {
                "fName" => name = as_name(row(r, c)),
                "e" => arg = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Function { name, arg });
        }
        "acc" => {
            let (mut mark, mut body) = ('\u{302}', Vec::new());
            each_child(r, t, |r, c| match c.name {
                "accPr" => each_child(r, c, |r, p| {
                    if p.name == "chr" {
                        mark = first_char(value(p)).unwrap_or(mark);
                    }
                    r.skip(p);
                }),
                "e" => body = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Accent { mark, body });
        }
        "bar" => {
            let (mut over, mut body) = (false, Vec::new());
            each_child(r, t, |r, c| match c.name {
                "barPr" => each_child(r, c, |r, p| {
                    if p.name == "pos" {
                        over = value(p).as_deref() == Some("top");
                    }
                    r.skip(p);
                }),
                "e" => body = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Bar { over, body });
        }
        "limLow" | "limUpp" => {
            let (mut base, mut limit) = (Vec::new(), Vec::new());
            each_child(r, t, |r, c| match c.name {
                "e" => base = part(r, c),
                "lim" => limit = part(r, c),
                _ => r.skip(c),
            });
            out.push(Math::Limit {
                base,
                limit,
                upper: t.name == "limUpp",
            });
        }
        "m" => {
            let mut rows = Vec::new();
            each_child(r, t, |r, c| {
                if c.name != "mr" {
                    r.skip(c);
                    return;
                }
                let mut cells = Vec::new();
                each_child(r, c, |r, cell| match cell.name {
                    "e" => cells.push(part(r, cell)),
                    _ => r.skip(cell),
                });
                rows.push(cells);
            });
            out.push(Math::Matrix { rows });
        }
        // Structures the engine has not: what they hold is kept.
        "box" | "borderBox" | "groupChr" | "phant" | "eqArr" | "oMath" | "oMathPara" => {
            each_child(r, t, |r, c| match c.name {
                "e" | "oMath" => out.extend(row(r, c)),
                _ => r.skip(c),
            })
        }
        _ => r.skip(t),
    });
    out
}

/// The equation an `<a14:m>` element holds.
pub(super) fn parse_equation<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Equation {
    let body = row(r, tag);
    // Nothing but characters: an equation kept in the linear format, if
    // building it up gives it a structure.
    if !body.is_empty() && body.iter().all(|node| matches!(node, Math::Text(_))) {
        let built = parse(&to_linear(&body));
        if built != body {
            return Equation {
                body: built,
                linear: true,
            };
        }
    }
    Equation {
        body,
        linear: false,
    }
}
