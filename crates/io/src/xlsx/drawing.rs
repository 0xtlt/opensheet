//! Pictures and shapes: the anchors of a drawing part that hold something
//! other than a chart (`xdr:pic`, `xdr:sp`, `xdr:cxnSp`), and the image
//! files of the pictures (`xl/media/image1.png`).
//!
//! A sheet has one drawing part for everything that floats over its cells.
//! The charts build it (`chart/write.rs`); [`add_parts`] puts the pictures
//! and shapes of the sheet after them, so the charts of a sheet lie under
//! its other drawings, which keep their order among themselves. An image
//! used several times, on one sheet or on several, is stored once.
//!
//! Kept: where a drawing is and how it follows the cells, rotation and
//! flips, name and description, the locks, the image of a picture as it
//! was inserted; of a shape, its preset geometry, fill, outline (width,
//! dash, ends) and text with one font, alignment and position.
//!
//! Not kept, because the engine has nothing for it: groups (left out),
//! cropping and effects of pictures, pictures that are linked rather than
//! embedded or of a format the engine does not show (EMF, SVG, TIFF),
//! geometries outside the engine's gallery (read as rectangles, connectors
//! as straight lines), gradients and patterns (read as one colour),
//! transparency, text formatted run by run (the first run speaks for all),
//! margins and columns of text.
//!
//! One oddity of the format is honoured both ways: the anchor of a shape
//! turned on its side (45° to 135° either way) describes the box of the
//! shape as it stands, width and height exchanged, where `a:xfrm` keeps
//! the box before rotation.

use super::chart::write::Drawing as DrawingPart;
use super::color::{Palette, adjust_luminance, parse_rgb, push_drawing_color, scheme_color};
use super::parts::{Extras, push_relationship};
use super::read::{Package, Rel, relationships};
use super::write::{NS_DOC_REL, NS_PKG_REL};
use super::xml::{Event, Reader, Tag, XML_DECL, push_attr, push_text};
use opensheet_engine::drawing::{
    Corner, Dash, Drawing, DrawingKind, Frame, LineEnd, Outline, Picture, Placement, Shape,
    ShapePreset, TextAlign, TextAnchor,
};
use opensheet_engine::style::{Font, Underline};
use opensheet_engine::{Color, MAX_COLS, MAX_ROWS, Sheet, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

const NS_DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_SHEET_DRAWING: &str =
    "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

const EMU_PER_PX: f64 = 9525.0;
const EMU_PER_PT: f64 = 12_700.0;
/// Angles are in 60 000ths of a degree.
const ANGLE_UNITS: f64 = 60_000.0;
/// The width of an outline that gives none (`w` of `<a:ln>`).
const DEFAULT_LINE_PT: f32 = 0.75;
/// No drawing is farther from the corner of the sheet, or larger: about
/// what a sheet of the largest rows can hold.
const MAX_PX: f64 = 600_000_000.0;

const ROOT_END: &str = "</xdr:wsDr>";
const RELS_END: &str = "</Relationships>";

mod smartart;
mod svg;
mod wordart;

// ---- writing -------------------------------------------------------------------

fn emu(px: f64) -> i64 {
    if px.is_finite() {
        (px * EMU_PER_PX).round() as i64
    } else {
        0
    }
}

/// True when a drawing turned by `degrees` stands on its side: its anchor
/// then holds the box with width and height exchanged.
fn on_its_side(degrees: f32) -> bool {
    degrees.is_finite() && (45.0..135.0).contains(&degrees.rem_euclid(180.0))
}

/// A frame with its width and height exchanged about its centre.
fn turned(frame: Frame) -> Frame {
    let (cx, cy) = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
    Frame::new(
        (cx - frame.height / 2.0).max(0.0),
        (cy - frame.width / 2.0).max(0.0),
        frame.height,
        frame.width,
    )
}

/// Put a drawing in a frame given in pixels from the corner of the sheet,
/// keeping the way it follows the cells.
fn set_frame(sheet: &Sheet, drawing: &mut Drawing, frame: Frame) {
    // Anchored to A1 the offsets are a place on the sheet; settling finds
    // the cells.
    drawing.anchor = (0, 0);
    drawing.offset = (frame.x as f32, frame.y as f32);
    drawing.size = (frame.width as f32, frame.height as f32);
    sheet.settle_drawing(drawing);
}

/// After a drawing was placed in the box its anchor describes: if it is
/// turned on its side, that was the box as it stands.
fn stand_up(sheet: &Sheet, drawing: &mut Drawing) {
    if on_its_side(drawing.rotation) {
        let standing = sheet.drawing_frame(drawing);
        set_frame(sheet, drawing, turned(standing));
    }
}

/// The two corners the anchor of a drawing names.
fn corners(sheet: &Sheet, drawing: &Drawing) -> (Corner, Corner) {
    if !on_its_side(drawing.rotation) {
        return sheet.drawing_corners(drawing);
    }
    let mut boxed = drawing.clone();
    boxed.placement = Placement::Fixed;
    set_frame(sheet, &mut boxed, turned(sheet.drawing_frame(drawing)));
    sheet.drawing_corners(&boxed)
}

fn push_corner(x: &mut String, name: &str, ((row, col), (dx, dy)): Corner) {
    w!(
        x,
        "<xdr:{name}><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{name}>",
        emu(dx as f64),
        emu(dy as f64)
    );
}

/// `<xdr:cNvPr>`: the id of the object in its part, its name and its
/// description.
fn push_identity(x: &mut String, drawing: &Drawing, id: usize) {
    w!(x, "<xdr:cNvPr id=\"{id}\" name=\"");
    push_attr(x, &drawing.name);
    x.push('"');
    if !drawing.alt_text.is_empty() {
        x.push_str(" descr=\"");
        push_attr(x, &drawing.alt_text);
        x.push('"');
    }
    x.push_str("/>");
}

/// `<a:xfrm>`: the box of the drawing before rotation, as an application
/// that does not follow anchors would place it.
fn push_xfrm(x: &mut String, drawing: &Drawing, frame: Frame) {
    x.push_str("<a:xfrm");
    let turn = (drawing.rotation as f64).rem_euclid(360.0);
    let angle = if turn.is_finite() {
        (turn * ANGLE_UNITS).round() as i64
    } else {
        0
    };
    if angle != 0 {
        w!(x, " rot=\"{angle}\"");
    }
    if drawing.flip_h {
        x.push_str(" flipH=\"1\"");
    }
    if drawing.flip_v {
        x.push_str(" flipV=\"1\"");
    }
    w!(
        x,
        "><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(frame.x),
        emu(frame.y),
        emu(frame.width),
        emu(frame.height)
    );
}

fn push_solid_fill(x: &mut String, color: Color) {
    x.push_str("<a:solidFill>");
    push_drawing_color(x, color, None);
    x.push_str("</a:solidFill>");
}

fn push_outline(x: &mut String, outline: Option<&Outline>) {
    let Some(outline) = outline else {
        x.push_str("<a:ln><a:noFill/></a:ln>");
        return;
    };
    let width = if outline.width.is_finite() {
        (outline.width.max(0.0) as f64 * EMU_PER_PT).round() as i64
    } else {
        0
    };
    w!(x, "<a:ln w=\"{width}\">");
    push_solid_fill(x, outline.color);
    if outline.dash != Dash::Solid {
        w!(x, "<a:prstDash val=\"{}\"/>", outline.dash.ooxml_name());
    }
    if outline.start != LineEnd::None {
        w!(x, "<a:headEnd type=\"{}\"/>", outline.start.ooxml_name());
    }
    if outline.end != LineEnd::None {
        w!(x, "<a:tailEnd type=\"{}\"/>", outline.end.ooxml_name());
    }
    x.push_str("</a:ln>");
}

/// What a connector is read as: a line with nothing but its outline. A
/// connector has no text in a file, so a line is written as one only when
/// that loses nothing.
fn connector_look(shape: &Shape) -> bool {
    shape.preset.is_line()
        && !shape.text_box
        && shape.fill.is_none()
        && shape.text.is_empty()
        && shape.font == Font::default()
        && shape.align == TextAlign::Center
        && shape.anchor == TextAnchor::Middle
}

/// Excel draws its arrows as connectors.
fn is_connector(shape: &Shape) -> bool {
    connector_look(shape)
        && shape
            .outline
            .is_some_and(|line| line.start != LineEnd::None || line.end != LineEnd::None)
}

/// The properties of an object that are not drawn (`xdr:cNvSpPr` and its
/// like): only the lock of its proportions is kept.
fn push_non_visual(
    x: &mut String,
    element: &str,
    attributes: &str,
    locks: &str,
    drawing: &Drawing,
) {
    if drawing.lock_aspect {
        w!(
            x,
            "<xdr:{element}{attributes}><a:{locks} noChangeAspect=\"1\"/></xdr:{element}>"
        );
    } else {
        w!(x, "<xdr:{element}{attributes}/>");
    }
}

/// `<a:prstGeom>`: the preset and, when the shape has one of its own, its
/// adjustment (in 100 000ths of the shorter side).
fn push_geometry(x: &mut String, shape: &Shape) {
    w!(x, "<a:prstGeom prst=\"{}\">", shape.preset.ooxml_name());
    match shape.adjust.filter(|adjust| adjust.is_finite()) {
        Some(adjust) => {
            let value = (adjust.clamp(0.0, 100.0) as f64 * 100_000.0).round() as i64;
            w!(x, "<a:avLst><a:gd name=\"adj\" fmla=\"val {value}\"/></a:avLst>");
        }
        None => x.push_str("<a:avLst/>"),
    }
    x.push_str("</a:prstGeom>");
}

/// The adjustment `<a:prstGeom>` gives its preset, if it gives one the
/// engine follows. Consumes the element.
fn parse_adjust<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, preset: Option<ShapePreset>) -> Option<f32> {
    let mut adjust = None;
    each_child(r, tag, |r, list| {
        each_child(r, list, |r, guide| {
            if guide.name == "gd" && guide.attr("name").as_deref() == Some("adj") {
                adjust = guide
                    .attr("fmla")
                    .and_then(|formula| formula.strip_prefix("val ")?.trim().parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .map(|value| (value / 100_000.0) as f32);
            }
            r.skip(guide);
        });
    });
    adjust.filter(|_| matches!(preset, Some(ShapePreset::Chevron | ShapePreset::Trapezoid)))
}

fn push_shape(x: &mut String, drawing: &Drawing, shape: &Shape, id: usize, frame: Frame) {
    if is_connector(shape) {
        x.push_str("<xdr:cxnSp macro=\"\"><xdr:nvCxnSpPr>");
        push_identity(x, drawing, id);
        push_non_visual(x, "cNvCxnSpPr", "", "cxnSpLocks", drawing);
        x.push_str("</xdr:nvCxnSpPr><xdr:spPr>");
        push_xfrm(x, drawing, frame);
        x.push_str("<a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom>");
        push_outline(x, shape.outline.as_ref());
        x.push_str("</xdr:spPr></xdr:cxnSp>");
        return;
    }
    x.push_str("<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr>");
    push_identity(x, drawing, id);
    let text_box = if shape.text_box { " txBox=\"1\"" } else { "" };
    push_non_visual(x, "cNvSpPr", text_box, "spLocks", drawing);
    x.push_str("</xdr:nvSpPr><xdr:spPr>");
    push_xfrm(x, drawing, frame);
    push_geometry(x, shape);
    match shape.fill {
        Some(color) => push_solid_fill(x, color),
        None => x.push_str("<a:noFill/>"),
    }
    push_outline(x, shape.outline.as_ref());
    x.push_str("</xdr:spPr>");
    wordart::push_text_body(x, shape);
    x.push_str("</xdr:sp>");
}

/// `image` is the relationship of the drawing part to the image file,
/// `vector` the one to the SVG document of a vector picture.
fn push_picture(x: &mut String, drawing: &Drawing, id: usize, frame: Frame, image: usize, vector: Option<usize>) {
    x.push_str("<xdr:pic><xdr:nvPicPr>");
    push_identity(x, drawing, id);
    push_non_visual(x, "cNvPicPr", "", "picLocks", drawing);
    w!(x, "</xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r=\"{NS_DOC_REL}\" r:embed=\"rId{image}\"");
    match vector {
        Some(vector) => {
            x.push('>');
            svg::push_extension(x, vector);
            x.push_str("</a:blip>");
        }
        None => x.push_str("/>"),
    }
    x.push_str("<a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr>");
    push_xfrm(x, drawing, frame);
    x.push_str("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>");
}

/// One anchor with its object. `id` is the object's own in the part,
/// `image` the relationship to the image of a picture.
fn push_anchor(
    x: &mut String,
    sheet: &Sheet,
    drawing: &Drawing,
    id: usize,
    image: Option<(usize, Option<usize>)>,
    graphic: Option<usize>,
) {
    // Excel anchors everything by two cells and says apart how the
    // object follows them.
    x.push_str("<xdr:twoCellAnchor");
    if drawing.placement != Placement::MoveAndSize {
        w!(x, " editAs=\"{}\"", drawing.placement.ooxml_name());
    }
    x.push('>');
    let (from, to) = corners(sheet, drawing);
    push_corner(x, "from", from);
    push_corner(x, "to", to);
    let frame = sheet.drawing_frame(drawing);
    match (&drawing.kind, image) {
        (DrawingKind::Shape(shape), _) => push_shape(x, drawing, shape, id, frame),
        (DrawingKind::Picture(_), Some((image, vector))) => push_picture(x, drawing, id, frame, image, vector),
        (DrawingKind::Picture(_), None) => {}
        (DrawingKind::SmartArt(_), _) => {
            if let Some(first) = graphic {
                smartart::push_frame(x, drawing, id, first);
            }
        }
    }
    x.push_str("<xdr:clientData");
    if !drawing.locked {
        x.push_str(" fLocksWithSheet=\"0\"");
    }
    if !drawing.print {
        x.push_str(" fPrintsWithSheet=\"0\"");
    }
    x.push_str("/></xdr:twoCellAnchor>");
}

/// Put `more` before the closing tag `end` of a part.
fn insert_before_end(part: &mut String, end: &str, more: &str) {
    if part.ends_with(end) {
        part.truncate(part.len() - end.len());
        part.push_str(more);
        part.push_str(end);
    }
}

/// The image files of the package being written: (bytes, name in
/// `xl/media/`).
#[derive(Default)]
struct Media(Vec<(Arc<[u8]>, String)>);

impl Media {
    /// The file of a picture, stored once however many pictures show it.
    fn name(&mut self, picture: &Picture, extras: &mut Extras) -> String {
        self.file(&picture.data, picture.format.extension(), picture.format.mime(), extras)
    }

    /// A media file of the package, stored once.
    fn file(&mut self, bytes: &Arc<[u8]>, extension: &'static str, mime: &'static str, extras: &mut Extras) -> String {
        let known = self
            .0
            .iter()
            .find(|(data, name)| name.ends_with(extension) && (Arc::ptr_eq(data, bytes) || **data == **bytes));
        if let Some((_, name)) = known {
            return name.clone();
        }
        let name = format!("image{}.{extension}", self.0.len() + 1);
        extras.add_default(extension, mime);
        extras.add_part(format!("xl/media/{name}"), None, bytes.to_vec());
        self.0.push((bytes.clone(), name.clone()));
        name
    }
}

/// Add the pictures and shapes of `wb` to the drawing parts of its sheets,
/// `parts` (those of the sheets with charts, to begin with), and their
/// image files to the package.
pub(crate) fn add_parts(wb: &Workbook, parts: &mut Vec<DrawingPart>, extras: &mut Extras) {
    let mut media = Media::default();
    // SmartArt graphics are numbered through the workbook.
    let mut graphics = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if sheet.drawings.is_empty() {
            continue;
        }
        let at = parts
            .iter()
            .position(|part| part.sheet == index)
            .unwrap_or_else(|| {
                parts.push(DrawingPart {
                    sheet: index,
                    number: 0,
                    xml: format!(
                        "{XML_DECL}<xdr:wsDr xmlns:xdr=\"{NS_SHEET_DRAWING}\" xmlns:a=\"{NS_DRAWING}\">{ROOT_END}"
                    ),
                    rels: format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\">{RELS_END}"),
                    charts: Vec::new(),
                });
                parts.len() - 1
            });
        // The charts come first: they have the first ids and the first
        // relationships of the part.
        let charts = sheet.charts.len();
        let mut anchors = String::new();
        // What the part relates to after its charts: (type, target).
        let mut links: Vec<(String, String)> = Vec::new();
        for (k, drawing) in sheet.drawings.iter().enumerate() {
            let mut related = |name: String| {
                let target = format!("../media/{name}");
                let place = links.iter().position(|(_, known)| *known == target);
                let place = place.unwrap_or_else(|| {
                    links.push((format!("{NS_DOC_REL}/image"), target));
                    links.len() - 1
                });
                charts + 1 + place
            };
            let image = drawing.as_picture().map(|picture| {
                let image = related(media.name(picture, extras));
                let vector = picture
                    .svg
                    .as_ref()
                    .map(|document| related(media.file(document, "svg", "image/svg+xml", extras)));
                (image, vector)
            });
            // A SmartArt graphic: its five parts, related one after the
            // other.
            let graphic = drawing.as_smartart().map(|art| {
                graphics += 1;
                let first = charts + 1 + links.len();
                let frame = sheet.drawing_frame(drawing);
                links.extend(smartart::add_parts(art, wb.theme(), (frame.width, frame.height), graphics, first, extras));
                first
            });
            push_anchor(&mut anchors, sheet, drawing, charts + 2 + k, image, graphic);
        }
        let mut rels = String::new();
        for (k, (kind, target)) in links.iter().enumerate() {
            push_relationship(&mut rels, charts + 1 + k, kind, target);
        }
        insert_before_end(&mut parts[at].xml, ROOT_END, &anchors);
        insert_before_end(&mut parts[at].rels, RELS_END, &rels);
    }
    // One part per sheet, numbered in the order of the sheets.
    parts.sort_by_key(|part| part.sheet);
    for (n, part) in parts.iter_mut().enumerate() {
        part.number = n + 1;
    }
}

// ---- reading -------------------------------------------------------------------

/// The children of an element, each handed to `child` (which consumes it).
fn each_child<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    mut child: impl FnMut(&mut Reader<'a>, &Tag<'a>),
) {
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
    if emu.is_finite() {
        (emu / EMU_PER_PX).clamp(0.0, MAX_PX)
    } else {
        0.0
    }
}

fn px_text(text: &str) -> f64 {
    px(text.trim().parse::<f64>().unwrap_or(0.0))
}

fn parse_corner<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Corner {
    let (mut row, mut col, mut dx, mut dy) = (0, 0, 0.0, 0.0);
    let index = |text: &str, count: u32| text.trim().parse::<u32>().unwrap_or(0).min(count - 1);
    each_child(r, tag, |r, t| match t.name {
        "col" => col = index(&r.text(t), MAX_COLS),
        "row" => row = index(&r.text(t), MAX_ROWS),
        "colOff" => dx = px_text(&r.text(t)),
        "rowOff" => dy = px_text(&r.text(t)),
        _ => r.skip(t),
    });
    ((row, col), (dx as f32, dy as f32))
}

/// Where the colours of a theme are in the palette, by the names shapes
/// call them.
fn scheme_slot(name: &str) -> Option<usize> {
    Some(match name {
        "dk1" | "tx1" => 0,
        "lt1" | "bg1" => 1,
        "dk2" | "tx2" => 2,
        "lt2" | "bg2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        "hlink" => 10,
        "folHlink" => 11,
        _ => return None,
    })
}

/// A colour channel as light: shades and tints are mixed there, which is
/// why "accent 1, 15 % shade" is not 15 % of its numbers.
fn to_linear(channel: u32) -> f64 {
    let v = channel as f64 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(light: f64) -> u32 {
    let light = light.clamp(0.0, 1.0);
    let v = if light <= 0.003_130_8 {
        light * 12.92
    } else {
        1.055 * light.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u32
}

/// Keep `amount` of the light of a colour and fill the rest with black
/// (`toward` 0) or white (`toward` 1): `<a:shade>` and `<a:tint>`.
fn mix_light(rgb: u32, amount: f64, toward: f64) -> u32 {
    let channel = |shift: u32| {
        let light = to_linear((rgb >> shift) & 0xFF) * amount + toward * (1.0 - amount);
        from_linear(light) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// The colour an element holds (`<a:solidFill>`, a reference of a style,
/// a stop of a gradient): given outright or by its place in the theme,
/// lightened or darkened by the adjustments nested in it. Transparency is
/// ignored.
fn parse_color<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<Color> {
    let mut color = None;
    each_child(r, tag, |r, t| {
        // The place in the theme of a colour that is given by it.
        let slot = match t.name {
            "schemeClr" => t.attr("val").and_then(|v| scheme_slot(&v)),
            _ => None,
        };
        let mut rgb = match t.name {
            "srgbClr" => t.attr("val").and_then(|v| parse_rgb(&v)),
            "schemeClr" => slot.map(|slot| palette.theme[slot]),
            "sysClr" => t.attr("lastClr").and_then(|v| parse_rgb(&v)).or_else(|| {
                match t.attr("val").as_deref() {
                    Some("windowText") => Some(0x000000),
                    Some("window") => Some(0xFFFFFF),
                    _ => None,
                }
            }),
            // Light, in thousandths of a percent.
            "scrgbClr" => {
                let channel = |name: &str| t.attr_f64(name).map(|v| from_linear(v / 100_000.0));
                match (channel("r"), channel("g"), channel("b")) {
                    (Some(r), Some(g), Some(b)) => Some((r << 16) | (g << 8) | b),
                    _ => None,
                }
            }
            "prstClr" => match t.attr("val").as_deref() {
                Some("black") => Some(0x000000),
                Some("white") => Some(0xFFFFFF),
                _ => None,
            },
            _ => None,
        };
        // Lightness is scaled, then shifted.
        let (mut scale, mut shift) = (1.0, 0.0);
        // Nothing but that was done to the colour.
        let mut plain_lightness = true;
        each_child(r, t, |r, adjustment| {
            let amount = adjustment
                .attr_f64("val")
                .filter(|v| v.is_finite())
                .map(|v| (v / 100_000.0).clamp(0.0, 1.0));
            if let (Some(base), Some(amount)) = (rgb, amount) {
                match adjustment.name {
                    "lumMod" => {
                        scale *= amount;
                        shift *= amount;
                    }
                    "lumOff" => shift += amount,
                    "tint" => rgb = Some(mix_light(base, amount, 1.0)),
                    "shade" => rgb = Some(mix_light(base, amount, 0.0)),
                    _ => {}
                }
            }
            plain_lightness &= matches!(adjustment.name, "lumMod" | "lumOff" | "alpha");
            r.skip(adjustment);
        });
        // A colour of the theme, as it is or as a picker varies it, stays
        // a colour of the theme.
        let reference = slot
            .filter(|_| plain_lightness)
            .and_then(|slot| scheme_color(palette, slot, scale, shift));
        if (scale, shift) != (1.0, 0.0) {
            rgb = rgb.map(|base| adjust_luminance(base, scale, shift));
        }
        if color.is_none() {
            color = reference.or(rgb.map(Color));
        }
    });
    color
}

/// How a file paints a surface or a line.
#[derive(Clone, Copy, Default)]
enum Paint {
    /// It does not say: the style of the shape decides.
    #[default]
    Unset,
    Nothing,
    Color(Color),
}

/// Read a fill element into `paint` if it is one; else pass it over.
/// Gradients are read as their first colour, patterns as their
/// foreground.
fn parse_paint<'a>(r: &mut Reader<'a>, t: &Tag<'a>, palette: &Palette, paint: &mut Paint) {
    let color = |color: Option<Color>| color.map_or(Paint::Unset, Paint::Color);
    match t.name {
        "noFill" => {
            *paint = Paint::Nothing;
            r.skip(t);
        }
        "solidFill" => *paint = color(parse_color(r, t, palette)),
        "gradFill" => {
            let mut first = None;
            each_child(r, t, |r, list| {
                each_child(r, list, |r, stop| {
                    let found = parse_color(r, stop, palette);
                    first = first.or(found);
                });
            });
            *paint = color(first);
        }
        "pattFill" => {
            let mut foreground = None;
            each_child(r, t, |r, c| match c.name {
                "fgClr" => foreground = parse_color(r, c, palette),
                _ => r.skip(c),
            });
            *paint = color(foreground);
        }
        _ => r.skip(t),
    }
}

/// `<a:ln>` as a file has it.
#[derive(Clone, Copy, Default)]
struct RawLine {
    paint: Paint,
    width: Option<f32>,
    dash: Dash,
    start: LineEnd,
    end: LineEnd,
}

fn parse_line<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawLine {
    let mut line = RawLine {
        width: tag
            .attr_f64("w")
            .filter(|w| w.is_finite() && *w >= 0.0)
            .map(|w| (w / EMU_PER_PT).min(1584.0) as f32),
        ..RawLine::default()
    };
    let end = |t: &Tag| {
        t.attr("type")
            .and_then(|name| LineEnd::from_ooxml_name(&name))
            .unwrap_or_default()
    };
    each_child(r, tag, |r, t| match t.name {
        "prstDash" => {
            line.dash = t
                .attr("val")
                .and_then(|name| Dash::from_ooxml_name(&name))
                .unwrap_or_default();
            r.skip(t);
        }
        "headEnd" => {
            line.start = end(t);
            r.skip(t);
        }
        "tailEnd" => {
            line.end = end(t);
            r.skip(t);
        }
        _ => parse_paint(r, t, palette, &mut line.paint),
    });
    line
}

/// What the theme of a workbook gives its shapes.
struct Theme<'a> {
    palette: &'a Palette,
    /// The widths of its three line styles, in points.
    line_widths: [f32; 3],
}

/// The line styles of the Office themes up to 2022, for a workbook
/// without a theme part.
const LINE_WIDTHS: [f32; 3] = [0.5, 1.0, 1.5];

/// The widths of the line styles of a theme part (`a:lnStyleLst`).
fn parse_line_widths(xml: &str) -> [f32; 3] {
    let mut widths = LINE_WIDTHS;
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "theme" | "themeElements" | "fmtScheme" => {}
                "lnStyleLst" => {
                    let mut n = 0;
                    each_child(&mut r, &t, |r, line| {
                        let width = line.attr_f64("w").filter(|w| w.is_finite() && *w >= 0.0);
                        if let (Some(width), Some(slot)) = (width, widths.get_mut(n)) {
                            *slot = (width / EMU_PER_PT).min(1584.0) as f32;
                        }
                        n += 1;
                        r.skip(line);
                    });
                    break;
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    widths
}

/// `<xdr:style>`: what the theme gives a shape where it says nothing
/// itself. Only the colours are followed, and the width of the line.
#[derive(Clone, Copy, Default)]
struct RawStyle {
    line: Option<(Color, f32)>,
    fill: Option<Color>,
    text: Option<Color>,
}

fn parse_style<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme) -> RawStyle {
    let mut style = RawStyle::default();
    each_child(r, tag, |r, t| {
        // The first entry of the theme's lists is number 1; 0 is none.
        let idx = t.attr_u32("idx").unwrap_or(0);
        match t.name {
            "lnRef" => {
                let color = parse_color(r, t, theme.palette);
                let width = theme.line_widths[idx.clamp(1, 3) as usize - 1];
                style.line = color.filter(|_| idx > 0).map(|color| (color, width));
            }
            "fillRef" => style.fill = parse_color(r, t, theme.palette).filter(|_| idx > 0),
            // Black from the theme is the automatic colour of text.
            "fontRef" => style.text = parse_color(r, t, theme.palette).filter(|color| color.value() != 0),
            _ => r.skip(t),
        }
    });
    style
}

/// A text without the characters a file cannot hold again; line breaks
/// and tabs stay where a text has `lines`.
fn printable(text: &str, lines: bool) -> String {
    text.chars()
        .filter(|c| !c.is_control() || (lines && matches!(c, '\n' | '\t')))
        .collect()
}

/// What an anchor holds, as its part says it.
enum RawKind {
    /// A SmartArt graphic: the relationships to its data, layout, style
    /// and colours parts.
    Graphic([String; 4]),
    /// The relationships of the drawing part to the image and, for a
    /// vector picture, to its SVG document.
    Picture(String, Option<String>),
    Shape(Shape),
}

struct RawObject {
    name: String,
    alt_text: String,
    lock_aspect: bool,
    rotation: f32,
    flip_h: bool,
    flip_v: bool,
    kind: RawKind,
}

impl RawObject {
    fn new(kind: RawKind) -> RawObject {
        RawObject {
            name: String::new(),
            alt_text: String::new(),
            lock_aspect: false,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            kind,
        }
    }

    /// `<xdr:nvSpPr>` and its like: the name and the locks.
    fn parse_identity<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) -> bool {
        let mut text_box = false;
        each_child(r, tag, |r, t| match t.name {
            "cNvPr" => {
                let text = |name: &str| t.attr(name).unwrap_or_default();
                self.name = printable(&text("name"), false);
                self.alt_text = printable(&text("descr"), true);
                r.skip(t);
            }
            "cNvSpPr" | "cNvPicPr" | "cNvCxnSpPr" => {
                text_box = t.attr_bool("txBox").unwrap_or(false);
                each_child(r, t, |r, locks| {
                    if matches!(locks.name, "spLocks" | "picLocks" | "cxnSpLocks") {
                        self.lock_aspect = locks.attr_bool("noChangeAspect").unwrap_or(false);
                    }
                    r.skip(locks);
                });
            }
            _ => r.skip(t),
        });
        text_box
    }

    /// `<a:xfrm>`: only how the object is turned; where it is, the anchor
    /// says.
    fn parse_xfrm<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        let angle = tag.attr_f64("rot").filter(|angle| angle.is_finite());
        self.rotation = angle.map_or(0.0, |angle| (angle / ANGLE_UNITS).rem_euclid(360.0) as f32);
        self.flip_h = tag.attr_bool("flipH").unwrap_or(false);
        self.flip_v = tag.attr_bool("flipV").unwrap_or(false);
        r.skip(tag);
    }
}

fn parse_picture<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<RawObject> {
    let mut object = RawObject::new(RawKind::Picture(String::new(), None));
    let mut embed = None;
    let mut vector = None;
    each_child(r, tag, |r, t| match t.name {
        "nvPicPr" => {
            object.parse_identity(r, t);
        }
        "blipFill" => each_child(r, t, |r, c| {
            if c.name == "blip" {
                embed = c.attr_local("embed").map(|id| id.into_owned());
                vector = svg::parse_extension(r, c);
            } else {
                r.skip(c);
            }
        }),
        "spPr" => each_child(r, t, |r, c| match c.name {
            "xfrm" => object.parse_xfrm(r, c),
            _ => r.skip(c),
        }),
        _ => r.skip(t),
    });
    // A picture that is only linked has no image in the file.
    object.kind = RawKind::Picture(embed.filter(|id| !id.is_empty())?, vector);
    Some(object)
}

fn parse_shape<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme) -> RawObject {
    let palette = theme.palette;
    let connector = tag.name == "cxnSp";
    let mut object = RawObject::new(RawKind::Picture(String::new(), None));
    let mut text_box = false;
    let mut preset = None;
    let mut fill = Paint::Unset;
    let mut line: Option<RawLine> = None;
    let mut style = RawStyle::default();
    let mut text = wordart::RawText::default();
    let mut adjust = None;
    each_child(r, tag, |r, t| match t.name {
        "nvSpPr" | "nvCxnSpPr" => text_box = object.parse_identity(r, t),
        "spPr" => each_child(r, t, |r, c| match c.name {
            "xfrm" => object.parse_xfrm(r, c),
            "prstGeom" => {
                preset = c
                    .attr("prst")
                    .and_then(|name| ShapePreset::from_ooxml_name(&name));
                adjust = parse_adjust(r, c, preset);
            }
            "ln" => line = Some(parse_line(r, c, palette)),
            _ => parse_paint(r, c, palette, &mut fill),
        }),
        "style" => style = parse_style(r, t, theme),
        "txBody" => text = wordart::parse_text(r, t, palette),
        _ => r.skip(t),
    });
    // A geometry the engine does not have: the nearest thing it has.
    let preset = preset.unwrap_or(if connector {
        ShapePreset::Line
    } else {
        ShapePreset::Rect
    });
    let fill = match fill {
        Paint::Color(color) => Some(color),
        Paint::Nothing => None,
        Paint::Unset => style.fill,
    };
    let line = line.unwrap_or_default();
    let stroke = match line.paint {
        Paint::Color(color) => Some(color),
        Paint::Nothing => None,
        Paint::Unset => style.line.map(|(color, _)| color),
    };
    let outline = stroke.map(|color| Outline {
        color,
        width: line
            .width
            .or(style.line.map(|(_, width)| width))
            .unwrap_or(DEFAULT_LINE_PT),
        dash: line.dash,
        start: line.start,
        end: line.end,
    });
    // Without text, a shape has the text settings of a new one.
    let mut shape = Shape {
        preset,
        text_box,
        fill: fill.filter(|_| !preset.is_line() || !connector),
        outline,
        text: String::new(),
        font: Font::default(),
        align: text.align.unwrap_or(if text.anchor.is_some() {
            TextAlign::Left
        } else {
            TextAlign::Center
        }),
        anchor: text.anchor.unwrap_or(TextAnchor::Middle),
        effects: Default::default(),
        runs: Vec::new(),
        wrap: true,
        adjust,
    };
    text.into_shape(&mut shape, style.text);
    object.kind = RawKind::Shape(shape);
    object
}

/// An anchor of a drawing part, with the picture or the shape it holds.
struct RawAnchor {
    placement: Placement,
    from: Option<Corner>,
    to: Option<Corner>,
    /// Place on the sheet, in pixels, of an anchor tied to no cell.
    pos: Option<(f64, f64)>,
    /// Size in pixels, where the anchor has no second corner.
    ext: Option<(f64, f64)>,
    locked: bool,
    print: bool,
    object: Option<RawObject>,
}

/// Wrappers deep enough to be on purpose are not followed.
const MAX_NESTING: usize = 4;

/// Read what an anchor holds: a picture or a shape, possibly inside a
/// choice between it and a fallback for older applications. Chart frames
/// are the charts' business, groups are left out.
fn parse_content<'a>(
    r: &mut Reader<'a>,
    t: &Tag<'a>,
    theme: &Theme,
    depth: usize,
    object: &mut Option<RawObject>,
) {
    match t.name {
        "pic" if object.is_none() => *object = parse_picture(r, t),
        "sp" | "cxnSp" if object.is_none() => *object = Some(parse_shape(r, t, theme)),
        // A frame holds a chart (which the charts read) or a graphic.
        "graphicFrame" if object.is_none() => {
            let mut frame = RawObject::new(RawKind::Picture(String::new(), None));
            if let Some(ids) = smartart::parse_frame(r, t, &mut frame) {
                frame.kind = RawKind::Graphic(ids);
                *object = Some(frame);
            }
        }
        "AlternateContent" | "Choice" if depth < MAX_NESTING => {
            each_child(r, t, |r, c| parse_content(r, c, theme, depth + 1, object))
        }
        _ => r.skip(t),
    }
}

fn parse_anchor<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme) -> RawAnchor {
    let mut anchor = RawAnchor {
        placement: match tag.name {
            "oneCellAnchor" => Placement::MoveOnly,
            "absoluteAnchor" => Placement::Fixed,
            _ => tag
                .attr("editAs")
                .and_then(|name| Placement::from_ooxml_name(&name))
                .unwrap_or_default(),
        },
        from: None,
        to: None,
        pos: None,
        ext: None,
        locked: true,
        print: true,
        object: None,
    };
    let pair = |t: &Tag, a: &str, b: &str| {
        let length = |name: &str| t.attr_f64(name).map_or(0.0, px);
        (length(a), length(b))
    };
    each_child(r, tag, |r, t| match t.name {
        "from" => anchor.from = Some(parse_corner(r, t)),
        "to" => anchor.to = Some(parse_corner(r, t)),
        "pos" => {
            anchor.pos = Some(pair(t, "x", "y"));
            r.skip(t);
        }
        "ext" => {
            anchor.ext = Some(pair(t, "cx", "cy"));
            r.skip(t);
        }
        "clientData" => {
            anchor.locked = t.attr_bool("fLocksWithSheet").unwrap_or(true);
            anchor.print = t.attr_bool("fPrintsWithSheet").unwrap_or(true);
            r.skip(t);
        }
        _ => parse_content(r, t, theme, 0, &mut anchor.object),
    });
    anchor
}

/// The anchors of a drawing part that hold a picture or a shape, in
/// drawing order.
fn parse_part(xml: &str, theme: &Theme) -> Vec<RawAnchor> {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "wsDr" | "AlternateContent" | "Choice" => {}
                "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor" => {
                    let anchor = parse_anchor(&mut r, &t, theme);
                    if anchor.object.is_some() {
                        out.push(anchor);
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

/// Reads the pictures and shapes of the sheets of a package, one sheet
/// after the other: the drawings get ids of their own in the workbook, and
/// pictures showing the same image file share its bytes.
pub(crate) struct Loader {
    next_id: u32,
    /// The image files read so far, by part name; `None` for those that
    /// are missing or not images the engine shows.
    images: HashMap<String, Option<Picture>>,
    /// The SVG documents of vector pictures, by part name.
    vectors: HashMap<String, Option<Arc<[u8]>>>,
    /// The widths of the line styles of the workbook's theme, in points.
    line_widths: [f32; 3],
}

impl Loader {
    /// `workbook_rels` are the relationships of the workbook part, which
    /// lead to the theme.
    pub fn new<R: Read + Seek>(pkg: &mut Package<R>, workbook_rels: &[Rel]) -> Loader {
        let theme = workbook_rels
            .iter()
            .find(|rel| rel.kind == "theme")
            .and_then(|rel| pkg.read(&rel.target).ok().flatten());
        Loader {
            next_id: 1,
            images: HashMap::new(),
            vectors: HashMap::new(),
            line_widths: theme.map_or(LINE_WIDTHS, |xml| parse_line_widths(&xml)),
        }
    }

    /// Read the pictures and shapes of the drawing part `sheet_part`
    /// relates to as `drawing_rel` (the `r:id` of its `<drawing>`), onto
    /// `sheet`, whose column widths and row heights are those of the file
    /// already. A part that is missing or makes no sense costs the
    /// drawings it describes, never the sheet.
    pub fn read_sheet<R: Read + Seek>(
        &mut self,
        pkg: &mut Package<R>,
        sheet_part: &str,
        drawing_rel: Option<&str>,
        sheet: &mut Sheet,
        palette: &Palette,
    ) {
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
        let theme = Theme {
            palette,
            line_widths: self.line_widths,
        };
        let anchors = parse_part(&xml, &theme);
        if anchors.is_empty() {
            return;
        }
        let rels = relationships(pkg, &part);
        for anchor in anchors {
            let Some(object) = anchor.object else {
                continue;
            };
            let mut drawing = match object.kind {
                RawKind::Shape(shape) => Drawing::shape(shape, (0.0, 0.0)),
                RawKind::Graphic(ids) => match smartart::load(pkg, &rels, &ids, &theme) {
                    Some(art) => Drawing::smartart(art, (0.0, 0.0)),
                    None => continue,
                },
                RawKind::Picture(embed, vector) => {
                    let image_of = |id: &str| rels.iter().find(|rel| rel.id == id && rel.kind == "image").map(|rel| rel.target.clone());
                    let Some(target) = image_of(&embed) else {
                        continue;
                    };
                    let image = self.images.entry(target).or_insert_with_key(|target| {
                        pkg.read_bytes(target).ok().flatten().and_then(Picture::new)
                    });
                    let Some(mut picture) = image.clone() else {
                        continue;
                    };
                    // A vector picture: its document, shared like images.
                    if let Some(target) = vector.and_then(|id| image_of(&id)) {
                        let document = self.vectors.entry(target).or_insert_with_key(|target| {
                            let bytes = pkg.read_bytes(target).ok().flatten()?;
                            opensheet_engine::svg::is_svg(&bytes).then(|| Arc::from(bytes))
                        });
                        picture.svg = document.clone();
                    }
                    Drawing::picture(picture)
                }
            };
            drawing.placement = anchor.placement;
            drawing.rotation = object.rotation;
            drawing.flip_h = object.flip_h;
            drawing.flip_v = object.flip_v;
            drawing.lock_aspect = object.lock_aspect;
            drawing.locked = anchor.locked;
            drawing.print = anchor.print;
            drawing.alt_text = object.alt_text;
            let size = |(width, height): (f64, f64)| (width as f32, height as f32);
            match (anchor.from, anchor.to, anchor.pos, anchor.ext) {
                (Some(from), Some(to), _, _) => sheet.place_drawing(&mut drawing, from, to),
                (Some(from), None, _, Some(ext)) => {
                    (drawing.anchor, drawing.offset) = from;
                    drawing.size = size(ext);
                    sheet.settle_drawing(&mut drawing);
                }
                (None, _, Some((x, y)), Some(ext)) => {
                    drawing.placement = Placement::Fixed;
                    drawing.offset = (x as f32, y as f32);
                    drawing.size = size(ext);
                    sheet.settle_drawing(&mut drawing);
                }
                // An anchor that is nowhere.
                _ => continue,
            }
            stand_up(sheet, &mut drawing);
            // The shapes a graphic keeps from its file were laid out for
            // the frame it has.
            let frame = sheet.drawing_frame(&drawing);
            if let Some(foreign) = drawing.as_smartart_mut().and_then(|art| art.foreign.as_mut()) {
                Arc::make_mut(foreign).size = (frame.width as f32, frame.height as f32);
            }
            // An anchor can be made to say what no file written from here
            // says (a second corner before the first, offsets larger than
            // their cells, a place beyond the last cell): the drawing is
            // read as writing it would put it. One that stands on its
            // side is left alone unless that moves it, since turning it
            // twice more would only round its numbers again.
            let mut written = drawing.clone();
            let (from, to) = corners(sheet, &written);
            sheet.place_drawing(&mut written, from, to);
            stand_up(sheet, &mut written);
            let (here, there) = (sheet.drawing_frame(&drawing), sheet.drawing_frame(&written));
            let moved = [
                here.x - there.x,
                here.y - there.y,
                here.width - there.width,
                here.height - there.height,
            ];
            if !on_its_side(drawing.rotation) || moved.iter().any(|by| by.abs() > 0.01) {
                drawing = written;
            }
            // A name of its own on the sheet.
            let taken = |name: &str| {
                name.is_empty()
                    || sheet
                        .drawings
                        .iter()
                        .any(|other| other.name.eq_ignore_ascii_case(name))
            };
            let base = match (&drawing.kind, object.name.trim()) {
                (DrawingKind::Picture(_), "") => "Picture",
                (DrawingKind::Shape(_), "") => "Shape",
                (DrawingKind::SmartArt(_), "") => "Diagram",
                (_, name) => name,
            };
            let mut name = object.name.clone();
            let mut number = sheet.drawings.len() + 1;
            while taken(&name) {
                name = format!("{base} {number}");
                number += 1;
            }
            drawing.name = name;
            drawing.id = self.next_id;
            self.next_id += 1;
            sheet.drawings.push(drawing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_on_their_side_swap_their_box() {
        assert!(!on_its_side(0.0) && !on_its_side(44.9) && !on_its_side(135.0));
        assert!(on_its_side(45.0) && on_its_side(90.0) && on_its_side(134.9));
        assert!(on_its_side(270.0) && on_its_side(-90.0) && !on_its_side(180.0));
        assert!(!on_its_side(f32::NAN) && !on_its_side(f32::INFINITY));
        let frame = Frame::new(100.0, 50.0, 80.0, 20.0);
        let standing = turned(frame);
        assert_eq!(standing, Frame::new(130.0, 20.0, 20.0, 80.0));
        assert_eq!(turned(standing), frame);
    }

    #[test]
    fn colours_of_the_theme_are_shaded_as_light() {
        // The theme written into every package: the one of Excel today.
        let palette = Palette {
            theme: opensheet_engine::Theme::office().colors.part_order(),
            ..Palette::default()
        };
        let color = |xml: &str| {
            let mut r = Reader::new(xml);
            let Event::Open(tag) = r.next() else {
                panic!("no element in {xml}");
            };
            parse_color(&mut r, &tag, &palette).map(|color| color.value())
        };
        // The outline of Excel's new shapes and of its text boxes: the
        // colours the engine gives its own.
        assert_eq!(
            color(
                "<a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"15000\"/></a:schemeClr></a:lnRef>"
            ),
            Some(0x042433)
        );
        assert_eq!(
            color(
                "<a:solidFill><a:schemeClr val=\"lt1\"><a:shade val=\"50000\"/></a:schemeClr></a:solidFill>"
            ),
            Some(0xBCBCBC)
        );
        assert_eq!(
            color(
                "<a:solidFill><a:srgbClr val=\"000000\"><a:tint val=\"50000\"/><a:alpha val=\"30000\"/></a:srgbClr></a:solidFill>"
            ),
            Some(0xBCBCBC)
        );
        assert_eq!(
            color(
                "<a:solidFill><a:schemeClr val=\"accent1\"><a:lumMod val=\"20000\"/><a:lumOff val=\"80000\"/></a:schemeClr></a:solidFill>"
            ),
            // "80 % lighter" stays a colour of the theme, with the value
            // cells give that variant.
            Some(opensheet_engine::Theme::office().accent(1, 0.8).value())
        );
        assert_eq!(
            color(
                "<a:solidFill><a:schemeClr val=\"accent1\"><a:lumMod val=\"20000\"/><a:lumOff val=\"70000\"/></a:schemeClr></a:solidFill>"
            ),
            Some(adjust_luminance(palette.theme[4], 0.2, 0.7))
        );
        assert_eq!(
            color(
                "<a:fillRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"100000\" b=\"50000\"/></a:fillRef>"
            ),
            Some(0x00FFBC)
        );
        assert_eq!(
            color("<a:solidFill><a:sysClr val=\"windowText\" lastClr=\"112233\"/></a:solidFill>"),
            Some(0x112233)
        );
        assert_eq!(
            color("<a:solidFill><a:prstClr val=\"white\"/></a:solidFill>"),
            Some(0xFFFFFF)
        );
        assert_eq!(
            color("<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>"),
            None
        );
        assert_eq!(color("<a:solidFill/>"), None);
    }

    #[test]
    fn line_styles_of_a_theme() {
        assert_eq!(
            parse_line_widths(&crate::xlsx::theme::xml(&opensheet_engine::Theme::office())),
            [1.0, 1.5, 2.0]
        );
        assert_eq!(
            parse_line_widths(
                "<a:theme><a:themeElements><a:fmtScheme><a:fillStyleLst><a:ln w=\"1\"/></a:fillStyleLst><a:lnStyleLst><a:ln w=\"6350\"/><a:ln/><a:ln w=\"-4\"/><a:ln w=\"38100\"/></a:lnStyleLst>"
            ),
            [0.5, 1.0, 1.5]
        );
        assert_eq!(parse_line_widths("no theme"), LINE_WIDTHS);
    }

    #[test]
    fn only_plain_arrows_are_connectors() {
        let arrow = opensheet_engine::drawing::ShapeTool::Arrow.shape();
        assert!(is_connector(&arrow));
        let line = opensheet_engine::drawing::ShapeTool::Line.shape();
        assert!(connector_look(&line) && !is_connector(&line));
        // Text has no place in a connector.
        let mut labelled = arrow.clone();
        labelled.text = "to here".to_string();
        assert!(!is_connector(&labelled));
        let mut bold = arrow;
        bold.font.bold = true;
        assert!(!is_connector(&bold));
    }
}
