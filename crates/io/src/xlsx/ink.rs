//! Ink (the strokes of the Draw tab) in the drawing part of a sheet.
//!
//! **Written** as freeform shapes: each stroke is an `xdr:sp` with a
//! custom geometry (`a:custGeom`) of one open path of Bézier curves, no
//! fill and a round line, which every version of Excel shows as it was
//! drawn. Excel itself keeps ink as an `xdr14:contentPart` pointing to an
//! InkML part, with a picture as fallback; that is not written here: the
//! fallback picture would have to be rendered, and an ink part Excel finds
//! fault with would cost a repair prompt.
//!
//! So that this application gets its strokes back as ink and not as
//! shapes, each anchor sits in the fallback of an `mc:AlternateContent`
//! whose choice requires a namespace of OpenSheet and names the tool:
//! Excel does not know the namespace and takes the fallback; the reader of
//! pictures and shapes (`drawing.rs`) does not follow fallbacks and leaves
//! the strokes to this file.
//!
//! **Read**: those, and the ink of Excel: every trace of the InkML part an
//! `xdr14:contentPart` points to becomes a stroke with the colour, the
//! width and the tool of its brush, placed in the frame of the anchor.
//! Pressure and the other channels of a trace are ignored.

use super::chart::write::Drawing as DrawingPart;
use super::read::{Package, relationships};
use super::write::NS_PKG_REL;
use super::xml::{Event, Reader, Tag, XML_DECL, push_attr};
use opensheet_engine::drawing::{Corner, Frame};
use opensheet_engine::ink::{Ink, MIN_EXTENT, PenKind, Pt};
use opensheet_engine::{Color, MAX_COLS, MAX_ROWS, Sheet, Workbook};
use std::fmt::Write as _;
use std::io::{Read, Seek};

const NS_DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_SHEET_DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const NS_COMPATIBILITY: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// The namespace of the choice only this application takes.
pub(crate) const NS_INK: &str = "https://opensheet.org/2026/ink";

const EMU_PER_PX: f64 = 9525.0;
const ROOT_END: &str = "</xdr:wsDr>";
/// No stroke is farther from the corner of the sheet, or larger.
const MAX_PX: f64 = 600_000_000.0;
/// A file cannot make a stroke of more points than this.
const MAX_POINTS: usize = 200_000;

// ---- writing -------------------------------------------------------------------

fn emu(px: f64) -> i64 {
    if px.is_finite() { (px * EMU_PER_PX).round() as i64 } else { 0 }
}

fn push_corner(x: &mut String, name: &str, ((row, col), (dx, dy)): Corner) {
    w!(
        x,
        "<xdr:{name}><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{name}>",
        emu(dx as f64),
        emu(dy as f64)
    );
}

fn push_point(x: &mut String, (px, py): (f64, f64)) {
    w!(x, "<a:pt x=\"{}\" y=\"{}\"/>", emu(px), emu(py));
}

/// The path of a stroke: the curve the screen draws through its points
/// (a Catmull-Rom spline), said as the Bézier curves a file knows. Each
/// curve ends on a point of the stroke, which is how they are read back.
fn push_path(x: &mut String, ink: &Ink) {
    let at = |i: isize| {
        let (px, py) = ink.points[i.clamp(0, ink.points.len() as isize - 1) as usize];
        (px as f64, py as f64)
    };
    w!(x, "<a:path w=\"{}\" h=\"{}\" fill=\"none\"><a:moveTo>", emu(ink.extent.0 as f64), emu(ink.extent.1 as f64));
    push_point(x, at(0));
    x.push_str("</a:moveTo>");
    match ink.points.len() {
        // A dot: a line of no length, which a round cap makes a disc.
        1 => {
            x.push_str("<a:lnTo>");
            push_point(x, at(0));
            x.push_str("</a:lnTo>");
        }
        2 => {
            x.push_str("<a:lnTo>");
            push_point(x, at(1));
            x.push_str("</a:lnTo>");
        }
        count => {
            for i in 0..count as isize - 1 {
                let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
                x.push_str("<a:cubicBezTo>");
                push_point(x, (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0));
                push_point(x, (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0));
                push_point(x, p2);
                x.push_str("</a:cubicBezTo>");
            }
        }
    }
    x.push_str("</a:path>");
}

/// One stroke: the choice that names it as ink, and the freeform shape.
fn push_ink(x: &mut String, sheet: &Sheet, ink: &Ink, id: usize) {
    let frame = sheet.ink_frame(ink);
    let (from, to) = sheet.ink_corners(ink);
    w!(
        x,
        "<mc:AlternateContent xmlns:mc=\"{NS_COMPATIBILITY}\"><mc:Choice xmlns:osi=\"{NS_INK}\" Requires=\"osi\"><osi:ink kind=\"{}\"/></mc:Choice><mc:Fallback>",
        ink.kind.file_name()
    );
    x.push_str("<xdr:twoCellAnchor editAs=\"oneCell\">");
    push_corner(x, "from", from);
    push_corner(x, "to", to);
    w!(x, "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"{id}\" name=\"");
    push_attr(x, &ink.name);
    x.push_str("\"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr>");
    w!(
        x,
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(frame.x),
        emu(frame.y),
        emu(frame.width),
        emu(frame.height)
    );
    x.push_str("<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst>");
    push_path(x, ink);
    x.push_str("</a:pathLst></a:custGeom><a:noFill/>");
    let width = ink.width_in((frame.width as f32, frame.height as f32));
    w!(x, "<a:ln w=\"{}\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"{:06X}\"", emu(width as f64), ink.color.value() & 0xFF_FFFF);
    let opacity = ink.kind.opacity();
    if opacity < 1.0 {
        w!(x, "><a:alpha val=\"{}\"/></a:srgbClr>", (opacity * 100_000.0).round() as u32);
    } else {
        x.push_str("/>");
    }
    x.push_str("</a:solidFill><a:round/></a:ln></xdr:spPr></xdr:sp><xdr:clientData");
    if !ink.locked {
        x.push_str(" fLocksWithSheet=\"0\"");
    }
    x.push_str("/></xdr:twoCellAnchor></mc:Fallback></mc:AlternateContent>");
}

/// Add the strokes of `wb` to the drawing parts of its sheets, after the
/// charts, the pictures and the shapes (`parts` holds those already).
pub(crate) fn add_parts(wb: &Workbook, parts: &mut Vec<DrawingPart>) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if sheet.inks.is_empty() {
            continue;
        }
        let at = parts.iter().position(|part| part.sheet == index).unwrap_or_else(|| {
            parts.push(DrawingPart {
                sheet: index,
                number: 0,
                xml: format!("{XML_DECL}<xdr:wsDr xmlns:xdr=\"{NS_SHEET_DRAWING}\" xmlns:a=\"{NS_DRAWING}\">{ROOT_END}"),
                rels: format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\"></Relationships>"),
                charts: Vec::new(),
                chart_anchors: Vec::new(),
            });
            parts.len() - 1
        });
        // The ids of the part go on after those of the charts (from 2)
        // and of the pictures and shapes.
        // (a group takes one, and one for each of its members).
        let objects: usize = sheet.drawings.iter().map(|drawing| drawing.object_count()).sum();
        let first = sheet.charts.len() + objects + 2;
        let mut anchors = String::new();
        for (k, ink) in sheet.inks.iter().enumerate() {
            push_ink(&mut anchors, sheet, ink, first + k);
        }
        let xml = &mut parts[at].xml;
        if xml.ends_with(ROOT_END) {
            xml.truncate(xml.len() - ROOT_END.len());
            xml.push_str(&anchors);
            xml.push_str(ROOT_END);
        }
    }
    // One part per sheet, numbered in the order of the sheets.
    parts.sort_by_key(|part| part.sheet);
    for (n, part) in parts.iter_mut().enumerate() {
        part.number = n + 1;
    }
}

// ---- reading -------------------------------------------------------------------

fn each_child<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, mut child: impl FnMut(&mut Reader<'a>, &Tag<'a>)) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) => child(r, &t),
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
}

fn px(emu: f64) -> f64 {
    if emu.is_finite() { (emu / EMU_PER_PX).clamp(-MAX_PX, MAX_PX) } else { 0.0 }
}

fn parse_corner<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Corner {
    let (mut row, mut col, mut dx, mut dy) = (0, 0, 0.0, 0.0);
    let index = |text: &str, count: u32| text.trim().parse::<u32>().unwrap_or(0).min(count - 1);
    let offset = |text: &str| px(text.trim().parse::<f64>().unwrap_or(0.0)).max(0.0) as f32;
    each_child(r, tag, |r, t| {
        let text = r.text(t);
        match t.name {
            "col" => col = index(&text, MAX_COLS),
            "row" => row = index(&text, MAX_ROWS),
            "colOff" => dx = offset(&text),
            "rowOff" => dy = offset(&text),
            _ => {}
        }
    });
    ((row, col), (dx, dy))
}

/// A freeform shape read as a stroke.
#[derive(Default)]
struct Freeform {
    name: String,
    points: Vec<Pt>,
    extent: (f32, f32),
    /// Width of the line in pixels, as drawn in its frame.
    width: f32,
    color: u32,
}

/// What an anchor holds for this file.
enum Body {
    Freeform(Freeform),
    /// The relationship of the drawing part to an InkML part.
    Part(String),
}

struct Anchor {
    from: Option<Corner>,
    to: Option<Corner>,
    locked: bool,
    name: String,
    body: Option<Body>,
}

fn parse_colour(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten()
}

/// `<a:path>`: the points its commands end on.
fn parse_path<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, shape: &mut Freeform) {
    let side = |name: &str| px(tag.attr_f64(name).unwrap_or(0.0)).max(0.0) as f32;
    shape.extent = (side("w"), side("h"));
    each_child(r, tag, |r, command| {
        // The last point of a command is where the pen arrives.
        let mut end = None;
        each_child(r, command, |r, pt| {
            if pt.name == "pt" {
                let at = |name: &str| px(pt.attr_f64(name).unwrap_or(0.0)) as f32;
                end = Some((at("x"), at("y")));
            }
            r.skip(pt);
        });
        if let (Some(end), true) = (end, shape.points.len() < MAX_POINTS) {
            shape.points.push(end);
        }
    });
    // A dot is written as a line of no length.
    if shape.points.len() == 2 && shape.points[0] == shape.points[1] {
        shape.points.pop();
    }
}

fn parse_freeform<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Freeform {
    let mut shape = Freeform {
        width: 1.0,
        ..Freeform::default()
    };
    each_child(r, tag, |r, t| match t.name {
        "nvSpPr" => each_child(r, t, |r, c| {
            if c.name == "cNvPr" {
                shape.name = c.attr("name").unwrap_or_default().chars().filter(|ch| !ch.is_control()).collect();
            }
            r.skip(c);
        }),
        "spPr" => each_child(r, t, |r, c| match c.name {
            "custGeom" => each_child(r, c, |r, g| match g.name {
                "pathLst" => each_child(r, g, |r, path| {
                    if path.name == "path" && shape.points.is_empty() {
                        parse_path(r, path, &mut shape);
                    } else {
                        r.skip(path);
                    }
                }),
                _ => r.skip(g),
            }),
            "ln" => {
                if let Some(w) = c.attr_f64("w") {
                    shape.width = px(w).max(0.0) as f32;
                }
                each_child(r, c, |r, fill| match fill.name {
                    "solidFill" => each_child(r, fill, |r, colour| {
                        if colour.name == "srgbClr" {
                            shape.color = colour.attr("val").and_then(|v| parse_colour(&v)).unwrap_or(0);
                        }
                        r.skip(colour);
                    }),
                    _ => r.skip(fill),
                });
            }
            _ => r.skip(c),
        }),
        _ => r.skip(t),
    });
    shape
}

fn parse_anchor<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Anchor {
    let mut anchor = Anchor {
        from: None,
        to: None,
        locked: true,
        name: String::new(),
        body: None,
    };
    each_child(r, tag, |r, t| match t.name {
        "from" => anchor.from = Some(parse_corner(r, t)),
        "to" => anchor.to = Some(parse_corner(r, t)),
        "clientData" => {
            anchor.locked = t.attr_bool("fLocksWithSheet").unwrap_or(true);
            r.skip(t);
        }
        "sp" => anchor.body = Some(Body::Freeform(parse_freeform(r, t))),
        "contentPart" => {
            anchor.body = t.attr_local("id").filter(|id| !id.is_empty()).map(|id| Body::Part(id.into_owned()));
            each_child(r, t, |r, c| match c.name {
                "nvContentPartPr" => each_child(r, c, |r, n| {
                    if n.name == "cNvPr" {
                        anchor.name = n.attr("name").unwrap_or_default().chars().filter(|ch| !ch.is_control()).collect();
                    }
                    r.skip(n);
                }),
                _ => r.skip(c),
            });
        }
        _ => r.skip(t),
    });
    anchor
}

fn is_anchor(name: &str) -> bool {
    matches!(name, "twoCellAnchor" | "oneCellAnchor")
}

/// `<mc:AlternateContent>`: this application's own strokes (the choice
/// names the tool, the fallback holds the shape) and Excel's (the choice
/// holds the ink, the fallback a picture of it).
fn parse_alternate<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, out: &mut Vec<(Anchor, Option<PenKind>)>) {
    let mut own: Option<PenKind> = None;
    each_child(r, tag, |r, branch| match branch.name {
        "Choice" => each_child(r, branch, |r, t| {
            if t.name == "ink" {
                own = Some(t.attr("kind").and_then(|kind| PenKind::from_file_name(&kind)).unwrap_or_default());
                r.skip(t);
            } else if is_anchor(t.name) {
                let anchor = parse_anchor(r, t);
                if matches!(anchor.body, Some(Body::Part(_))) {
                    out.push((anchor, None));
                }
            } else {
                r.skip(t);
            }
        }),
        "Fallback" if own.is_some() => each_child(r, branch, |r, t| {
            if is_anchor(t.name) {
                let anchor = parse_anchor(r, t);
                if matches!(anchor.body, Some(Body::Freeform(_))) {
                    out.push((anchor, own));
                }
            } else {
                r.skip(t);
            }
        }),
        _ => r.skip(branch),
    });
}

fn parse_part(xml: &str) -> Vec<(Anchor, Option<PenKind>)> {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "wsDr" => {}
                "AlternateContent" => parse_alternate(&mut r, &t, &mut out),
                // Ink written without a fallback.
                name if is_anchor(name) => {
                    let anchor = parse_anchor(&mut r, &t);
                    if matches!(anchor.body, Some(Body::Part(_))) {
                        out.push((anchor, None));
                    }
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

// ---- InkML ---------------------------------------------------------------------

/// A brush of an InkML part.
#[derive(Clone, Debug, PartialEq)]
struct Brush {
    id: String,
    /// Width of the tip in pixels.
    width: f32,
    color: u32,
    highlighter: bool,
}

/// One trace of an InkML part: its points in pixels and its brush.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Trace {
    pub points: Vec<(f64, f64)>,
    pub width: f32,
    pub color: u32,
    pub kind: PenKind,
}

/// A length of an InkML part in pixels.
fn length_px(value: f64, units: &str) -> f64 {
    let per_unit = match units {
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "in" => 96.0,
        "pt" => 96.0 / 72.0,
        "emu" => 1.0 / EMU_PER_PX,
        // Himetric, the unit of ink without one: 1/1000 cm.
        _ => 96.0 / 2540.0,
    };
    value * per_unit
}

/// The numbers of a trace: points are separated by commas, values by
/// spaces or by the sign or the prefix of the next one. `!` gives values
/// as they are, `'` as differences from the previous point and `"` as
/// differences of differences; a prefix holds until another one.
fn parse_trace(text: &str, channels: usize) -> Vec<Vec<f64>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Explicit,
        First,
        Second,
    }
    let mut out: Vec<Vec<f64>> = Vec::new();
    let mut mode = vec![Mode::Explicit; channels];
    let mut last = vec![0.0f64; channels];
    let mut speed = vec![0.0f64; channels];
    for point in text.split(',') {
        let bytes = point.as_bytes();
        let mut values: Vec<f64> = Vec::with_capacity(channels);
        let mut i = 0;
        while i < bytes.len() && values.len() < channels {
            let channel = values.len();
            match bytes[i] {
                b' ' | b'\t' | b'\n' | b'\r' => i += 1,
                b'!' => {
                    mode[channel] = Mode::Explicit;
                    i += 1;
                }
                b'\'' => {
                    mode[channel] = Mode::First;
                    i += 1;
                }
                b'"' => {
                    mode[channel] = Mode::Second;
                    i += 1;
                }
                // A value that is not given, or a flag: the channel keeps
                // what it had.
                b'?' | b'*' | b'T' | b'F' => {
                    values.push(last[channel]);
                    i += 1;
                }
                _ => {
                    let start = i;
                    if matches!(bytes[i], b'-' | b'+') {
                        i += 1;
                    }
                    while i < bytes.len() && (bytes[i].is_ascii_digit() || matches!(bytes[i], b'.' | b'e' | b'E')) {
                        i += 1;
                    }
                    if i == start {
                        // Something this reader does not know: step over.
                        i += 1;
                        continue;
                    }
                    let Ok(number) = point[start..i].parse::<f64>() else {
                        continue;
                    };
                    let value = match mode[channel] {
                        Mode::Explicit => number,
                        Mode::First => {
                            speed[channel] = number;
                            last[channel] + number
                        }
                        Mode::Second => {
                            speed[channel] += number;
                            last[channel] + speed[channel]
                        }
                    };
                    last[channel] = value;
                    values.push(value);
                }
            }
        }
        if values.len() >= 2 && out.len() < MAX_POINTS {
            out.push(values);
        }
    }
    out
}

/// The traces of an InkML part, in pixels in the space of the ink.
pub(crate) fn parse_inkml(xml: &str) -> Vec<Trace> {
    // Channels in the order of the trace format; X and Y are the first two
    // when the part has no format.
    let mut channels: Vec<String> = Vec::new();
    // Units per centimetre of the X and Y channels.
    let mut resolution: (f64, f64) = (1000.0, 1000.0);
    let mut brushes: Vec<Brush> = Vec::new();
    let mut traces = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "channel" if channels.len() < 16 => {
                    channels.push(t.attr("name").unwrap_or_default().into_owned());
                    r.skip(&t);
                }
                "channelProperty" => {
                    if t.attr("name").as_deref() == Some("resolution") {
                        let value = t.attr_f64("value").filter(|v| v.is_finite() && *v > 0.0);
                        // "1/cm" is what Office writes; per inch is the
                        // other unit a digitiser gives.
                        let per_cm = match t.attr("units").as_deref() {
                            Some("1/in") => value.map(|v| v / 2.54),
                            _ => value,
                        };
                        match (t.attr("channel").as_deref(), per_cm) {
                            (Some("X"), Some(v)) => resolution.0 = v,
                            (Some("Y"), Some(v)) => resolution.1 = v,
                            _ => {}
                        }
                    }
                    r.skip(&t);
                }
                "brush" => {
                    let mut brush = Brush {
                        id: t.attr_local("id").unwrap_or_default().into_owned(),
                        width: 2.0,
                        color: 0,
                        highlighter: false,
                    };
                    each_child(&mut r, &t, |r, p| {
                        if p.name == "brushProperty" {
                            let value = p.attr("value").unwrap_or_default();
                            match p.attr("name").as_deref() {
                                Some("width") => {
                                    let units = p.attr("units").unwrap_or_default();
                                    if let Ok(v) = value.trim().parse::<f64>() {
                                        brush.width = (length_px(v, &units) as f32).clamp(0.25, 200.0);
                                    }
                                }
                                Some("color") => brush.color = parse_colour(&value).unwrap_or(0),
                                Some("rasterOp") => brush.highlighter |= value.trim() == "maskPen",
                                Some("transparency") => {
                                    brush.highlighter |= value.trim().parse::<f64>().is_ok_and(|v| v > 0.0)
                                }
                                _ => {}
                            }
                        }
                        r.skip(p);
                    });
                    brushes.push(brush);
                }
                "trace" => {
                    let brush_ref = t.attr("brushRef").unwrap_or_default().trim_start_matches('#').to_string();
                    let text = r.text(&t);
                    let count = channels.len().max(2);
                    let (ix, iy) = (
                        channels.iter().position(|c| c == "X").unwrap_or(0),
                        channels.iter().position(|c| c == "Y").unwrap_or(1),
                    );
                    let points: Vec<(f64, f64)> = parse_trace(&text, count)
                        .into_iter()
                        .filter_map(|values| Some((*values.get(ix)?, *values.get(iy)?)))
                        .map(|(x, y)| (x / resolution.0 * 96.0 / 2.54, y / resolution.1 * 96.0 / 2.54))
                        .collect();
                    if points.is_empty() {
                        continue;
                    }
                    let brush = brushes.iter().find(|b| b.id == brush_ref).or(brushes.first());
                    traces.push(Trace {
                        points,
                        width: brush.map_or(2.0, |b| b.width),
                        color: brush.map_or(0, |b| b.color),
                        kind: if brush.is_some_and(|b| b.highlighter) { PenKind::Highlighter } else { PenKind::Pen },
                    });
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    traces
}

// ---- onto the sheet ------------------------------------------------------------

/// Reads the ink of the sheets of a package, one sheet after the other:
/// the strokes get ids of their own in the workbook.
pub(crate) struct Loader {
    next_id: u32,
}

impl Loader {
    pub fn new() -> Loader {
        Loader { next_id: 1 }
    }

    fn push(&mut self, sheet: &mut Sheet, mut ink: Ink, frame: Frame, name: &str) {
        let taken = |name: &str| name.is_empty() || sheet.inks.iter().any(|other| other.name.eq_ignore_ascii_case(name));
        let base = if name.trim().is_empty() { "Ink" } else { name.trim() };
        let mut name = name.to_string();
        let mut number = sheet.inks.len() + 1;
        while taken(&name) {
            name = format!("{base} {number}");
            number += 1;
        }
        ink.name = name;
        ink.id = self.next_id;
        self.next_id += 1;
        (ink.anchor, ink.offset) = sheet.corner_at(frame.x.max(0.0), frame.y.max(0.0));
        ink.size = ((frame.width as f32).max(MIN_EXTENT), (frame.height as f32).max(MIN_EXTENT));
        sheet.inks.push(ink);
    }

    /// Read the ink of the drawing part `sheet_part` relates to as
    /// `drawing_rel` onto `sheet`, whose column widths and row heights are
    /// those of the file already. What makes no sense is left out.
    pub fn read_sheet<R: Read + Seek>(&mut self, pkg: &mut Package<R>, sheet_part: &str, drawing_rel: Option<&str>, sheet: &mut Sheet) {
        let Some(drawing_rel) = drawing_rel else {
            return;
        };
        let Some(part) = relationships(pkg, sheet_part)
            .into_iter()
            .find(|rel| rel.id == drawing_rel && rel.kind == "drawing")
            .map(|rel| rel.target)
        else {
            return;
        };
        let Ok(Some(xml)) = pkg.read(&part) else {
            return;
        };
        // Most drawing parts hold no ink: do not parse them twice.
        if !xml.contains("contentPart") && !xml.contains(NS_INK) {
            return;
        }
        let anchors = parse_part(&xml);
        if anchors.is_empty() {
            return;
        }
        let rels = relationships(pkg, &part);
        for (anchor, own) in anchors {
            let (Some(from), Some(to)) = (anchor.from, anchor.to) else {
                continue;
            };
            let (x0, y0) = sheet.corner_point(from);
            let (x1, y1) = sheet.corner_point(to);
            let frame = Frame::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0));
            match anchor.body {
                Some(Body::Freeform(shape)) => {
                    if shape.points.is_empty() {
                        continue;
                    }
                    let mut ink = Ink {
                        id: 0,
                        name: String::new(),
                        kind: own.unwrap_or_default(),
                        color: Color(shape.color),
                        width: 1.0,
                        points: shape.points.into(),
                        extent: (shape.extent.0.max(MIN_EXTENT), shape.extent.1.max(MIN_EXTENT)),
                        anchor: (0, 0),
                        offset: (0.0, 0.0),
                        size: (0.0, 0.0),
                        locked: anchor.locked,
                    };
                    // The file gives the line as it is drawn in the frame;
                    // the stroke keeps it at its natural size.
                    let size = ((frame.width as f32).max(MIN_EXTENT), (frame.height as f32).max(MIN_EXTENT));
                    let grown = ink.width_in(size);
                    ink.width = (shape.width / grown).clamp(0.25, 200.0);
                    self.push(sheet, ink, frame, &shape.name);
                }
                Some(Body::Part(id)) => {
                    let Some(target) = rels.iter().find(|rel| rel.id == id).map(|rel| rel.target.clone()) else {
                        continue;
                    };
                    let Ok(Some(inkml)) = pkg.read(&target) else {
                        continue;
                    };
                    self.place_traces(sheet, parse_inkml(&inkml), frame, anchor.locked, &anchor.name);
                }
                None => {}
            }
        }
    }

    /// Put the traces of one ink object in the frame of its anchor: the
    /// frame is the box of all of them, the thickness of their lines
    /// included.
    fn place_traces(&mut self, sheet: &mut Sheet, traces: Vec<Trace>, frame: Frame, locked: bool, name: &str) {
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for trace in &traces {
            let half = trace.width as f64 / 2.0;
            for (x, y) in &trace.points {
                let b = bounds.get_or_insert((x - half, y - half, x + half, y + half));
                *b = (b.0.min(x - half), b.1.min(y - half), b.2.max(x + half), b.3.max(y + half));
            }
        }
        let Some((bx0, by0, bx1, by1)) = bounds else {
            return;
        };
        let scale = |have: f64, want: f64| if have > 0.0 && want > 0.0 { want / have } else { 1.0 };
        let (sx, sy) = (scale(bx1 - bx0, frame.width), scale(by1 - by0, frame.height));
        for trace in traces {
            let points: Vec<(f64, f64)> = trace
                .points
                .iter()
                .map(|(x, y)| (frame.x + (x - bx0) * sx, frame.y + (y - by0) * sy))
                .collect();
            let width = trace.width * (sx * sy).sqrt() as f32;
            if let Some((mut ink, at)) = Ink::through(trace.kind, Color(trace.color), width, &points) {
                ink.locked = locked;
                self.push(sheet, ink, at, name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_are_read_in_their_three_notations() {
        // Values as they are.
        assert_eq!(parse_trace("10 20, 30 40", 2), vec![vec![10.0, 20.0], vec![30.0, 40.0]]);
        // Signs separate values; a third channel is carried along.
        assert_eq!(parse_trace("10-20 5,-3 4 5", 3), vec![vec![10.0, -20.0, 5.0], vec![-3.0, 4.0, 5.0]]);
        // Differences, then differences of differences, then explicit again.
        let points = parse_trace("100 200, '5 '10, 5 10, \"1 \"-2, 0 0, !7 !8", 2);
        assert_eq!(
            points,
            vec![
                vec![100.0, 200.0],
                vec![105.0, 210.0],
                vec![110.0, 220.0],
                vec![116.0, 228.0],
                vec![122.0, 236.0],
                vec![7.0, 8.0],
            ]
        );
        // A point without both coordinates is left out.
        assert_eq!(parse_trace("1 2, 3, 4 5", 2), vec![vec![1.0, 2.0], vec![4.0, 5.0]]);
    }

    #[test]
    fn an_ink_part_gives_its_traces_and_brushes() {
        let xml = r##"<?xml version="1.0" encoding="UTF-8"?>
<inkml:ink xmlns:inkml="http://www.w3.org/2003/InkML"><inkml:definitions>
<inkml:context xml:id="ctx0"><inkml:inkSource xml:id="inkSrc0"><inkml:traceFormat>
<inkml:channel name="X" type="integer" max="32767" units="cm"/><inkml:channel name="Y" type="integer" max="32767" units="cm"/>
<inkml:channel name="F" type="integer" max="32767" units="dev"/></inkml:traceFormat>
<inkml:channelProperties><inkml:channelProperty channel="X" name="resolution" value="1000" units="1/cm"/>
<inkml:channelProperty channel="Y" name="resolution" value="1000" units="1/cm"/></inkml:channelProperties></inkml:inkSource></inkml:context>
<inkml:brush xml:id="br0"><inkml:brushProperty name="width" value="0.05" units="cm"/><inkml:brushProperty name="color" value="#E71224"/></inkml:brush>
<inkml:brush xml:id="br1"><inkml:brushProperty name="width" value="0.5" units="cm"/><inkml:brushProperty name="color" value="#FFFC00"/>
<inkml:brushProperty name="rasterOp" value="maskPen"/></inkml:brush></inkml:definitions>
<inkml:trace contextRef="#ctx0" brushRef="#br0">0 0 16383,2540 1270 16383</inkml:trace>
<inkml:trace contextRef="#ctx0" brushRef="#br1">0 2540 100,2540 2540 100</inkml:trace></inkml:ink>"##;
        let traces = parse_inkml(xml);
        assert_eq!(traces.len(), 2);
        // 2.54 cm is 96 pixels.
        assert_eq!(traces[0].points, vec![(0.0, 0.0), (96.0, 48.0)]);
        assert_eq!((traces[0].color, traces[0].kind), (0xE71224, PenKind::Pen));
        assert!((traces[0].width - 1.89).abs() < 0.01);
        assert_eq!((traces[1].color, traces[1].kind), (0xFFFC00, PenKind::Highlighter));
        assert_eq!(traces[1].points[1], (96.0, 96.0));
    }
}
