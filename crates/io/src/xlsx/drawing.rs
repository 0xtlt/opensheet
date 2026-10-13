//! Pictures and shapes: the anchors of a drawing part that hold something
//! other than a chart (`xdr:pic`, `xdr:sp`, `xdr:cxnSp`), and the image
//! files of the pictures (`xl/media/image1.png`).
//!
//! A sheet has one drawing part for everything that floats over its cells.
//! The charts build it (`chart/write.rs`); [`add_parts`] weaves the
//! pictures and shapes of the sheet among them, so that the anchors of
//! the part are in the stacking order charts and drawings share
//! (`Sheet::object_stack`), which is how a file says it. An image used
//! several times, on one sheet or on several, is stored once.
//!
//! Kept: where a drawing is and how it follows the cells, rotation and
//! flips, name and description, the locks, the image of a picture as it
//! was inserted; of a shape, its preset geometry, fill, outline (width,
//! dash, ends) and text with one font, alignment and position.
//!
//! Groups are kept with their members (a chart in a group is left out
//! of it; the chart reader shows it as a chart of its own).
//!
//! Of a picture, what the Picture Format tab does to it: its crop
//! (`a:srcRect`) and the shape it is cropped to (its `a:prstGeom`),
//! transparency (`a:alphaModFix`), brightness and contrast (`a:lum`),
//! recolouring (`a:grayscl`, `a:biLevel`, `a:duotone`; "Washout" is the
//! `a:lum` Excel writes for it). Saturation and colour temperature have
//! no element of their own in a blip (Excel keeps them beside a second,
//! HD Photo copy of the image, which is not written): they are written
//! as the nearest effects there are, `a:hsl` (saturation) and `a:tint`
//! (towards orange or blue), and read back from them. Of pictures and
//! shapes, the effects that are drawn (`a:effectLst`: `a:outerShdw`,
//! `a:glow`, `a:softEdge`).
//!
//! Not kept, because the engine has nothing for it:
//! the other effects (reflection, inner shadows, 3-D), pictures that are linked rather than
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
    Corner, Dash, Drawing, DrawingKind, Effects, Frame, Glow, LineEnd, NEUTRAL_TEMPERATURE,
    ObjectRef, Outline, Picture, PictureAdjust, Placement, Recolor, Shadow, Shape, ShapePreset,
    TextAlign, TextAnchor,
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
    if drawing.hidden {
        x.push_str(" hidden=\"1\"");
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

/// The properties of a run (`a:rPr`), or of the end of a paragraph that
/// has none (`a:endParaRPr`).
fn push_run_properties(x: &mut String, element: &str, font: &Font) {
    w!(x, "<a:{element} sz=\"{}\"", font.size as u32 * 50);
    if font.bold {
        x.push_str(" b=\"1\"");
    }
    if font.italic {
        x.push_str(" i=\"1\"");
    }
    match font.underline {
        Underline::None => {}
        Underline::Single => x.push_str(" u=\"sng\""),
        Underline::Double => x.push_str(" u=\"dbl\""),
    }
    if font.strike {
        x.push_str(" strike=\"sngStrike\"");
    }
    x.push('>');
    if let Some(color) = font.color {
        push_solid_fill(x, color);
    }
    for script in ["latin", "cs"] {
        w!(x, "<a:{script} typeface=\"");
        push_attr(x, &font.name);
        x.push_str("\"/>");
    }
    w!(x, "</a:{element}>");
}

fn push_text_body(x: &mut String, shape: &Shape) {
    w!(
        x,
        "<xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"{}\"/><a:lstStyle/>",
        shape.anchor.ooxml_name()
    );
    for paragraph in shape.text.split('\n') {
        w!(x, "<a:p><a:pPr algn=\"{}\"/>", shape.align.ooxml_name());
        if paragraph.is_empty() {
            push_run_properties(x, "endParaRPr", &shape.font);
        } else {
            x.push_str("<a:r>");
            push_run_properties(x, "rPr", &shape.font);
            x.push_str("<a:t>");
            push_text(x, paragraph);
            x.push_str("</a:t></a:r>");
        }
        x.push_str("</a:p>");
    }
    x.push_str("</xdr:txBody>");
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

fn push_shape(x: &mut String, drawing: &Drawing, shape: &Shape, id: usize, frame: Frame) {
    if is_connector(shape) {
        x.push_str("<xdr:cxnSp macro=\"\"><xdr:nvCxnSpPr>");
        push_identity(x, drawing, id);
        push_non_visual(x, "cNvCxnSpPr", "", "cxnSpLocks", drawing);
        x.push_str("</xdr:nvCxnSpPr><xdr:spPr>");
        push_xfrm(x, drawing, frame);
        x.push_str("<a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom>");
        push_outline(x, shape.outline.as_ref());
        push_effects(x, &drawing.effects);
        x.push_str("</xdr:spPr></xdr:cxnSp>");
        return;
    }
    x.push_str("<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr>");
    push_identity(x, drawing, id);
    let text_box = if shape.text_box { " txBox=\"1\"" } else { "" };
    push_non_visual(x, "cNvSpPr", text_box, "spLocks", drawing);
    x.push_str("</xdr:nvSpPr><xdr:spPr>");
    push_xfrm(x, drawing, frame);
    w!(
        x,
        "<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>",
        shape.preset.ooxml_name()
    );
    match shape.fill {
        Some(color) => push_solid_fill(x, color),
        None => x.push_str("<a:noFill/>"),
    }
    push_outline(x, shape.outline.as_ref());
    push_effects(x, &drawing.effects);
    x.push_str("</xdr:spPr>");
    push_text_body(x, shape);
    x.push_str("</xdr:sp>");
}

/// A fraction as the thousandths of a percent files count in.
fn per_100k(fraction: f32) -> i64 {
    if fraction.is_finite() {
        (fraction as f64 * 100_000.0).round() as i64
    } else {
        0
    }
}

/// A colour with how opaque it is: one of the theme by its name, any
/// other by its value, with `<a:alpha>` in it when it lets light through.
fn push_color_alpha(x: &mut String, color: Color, transparency: f32) {
    let alpha = per_100k(1.0 - transparency.clamp(0.0, 1.0)) as u32;
    super::color::push_drawing_color(x, color, (alpha != 100_000).then_some(alpha));
}

/// The side of the shape a shadow is measured from (`algn`): the one it
/// falls away from, as Excel's presets say it.
fn shadow_alignment(shadow: &Shadow) -> &'static str {
    if shadow.distance <= 0.0 {
        return "ctr";
    }
    // Eight directions, clockwise from the right.
    const SIDES: [&str; 8] = ["l", "tl", "t", "tr", "r", "br", "b", "bl"];
    let turn = (shadow.angle as f64).rem_euclid(360.0);
    SIDES[((turn / 45.0).round() as usize) % 8]
}

/// `<a:effectLst>`: the glow, the shadow and the soft edges of a drawing,
/// in the order of the schema. Nothing for a drawing without effects.
fn push_effects(x: &mut String, effects: &Effects) {
    if effects.is_none() {
        return;
    }
    let length = |points: f32| (points.max(0.0) as f64 * EMU_PER_PT).round() as i64;
    x.push_str("<a:effectLst>");
    if let Some(glow) = &effects.glow {
        w!(x, "<a:glow rad=\"{}\">", length(glow.radius));
        push_color_alpha(x, glow.color, glow.transparency);
        x.push_str("</a:glow>");
    }
    if let Some(shadow) = &effects.shadow {
        let angle = ((shadow.angle as f64).rem_euclid(360.0) * ANGLE_UNITS).round() as i64;
        w!(
            x,
            "<a:outerShdw blurRad=\"{}\" dist=\"{}\" dir=\"{angle}\"",
            length(shadow.blur),
            length(shadow.distance)
        );
        let size = per_100k(shadow.size);
        if size != 100_000 {
            w!(x, " sx=\"{size}\" sy=\"{size}\"");
        }
        w!(x, " algn=\"{}\" rotWithShape=\"0\">", shadow_alignment(shadow));
        push_color_alpha(x, shadow.color, shadow.transparency);
        x.push_str("</a:outerShdw>");
    }
    if effects.soft_edge > 0.0 {
        w!(x, "<a:softEdge rad=\"{}\"/>", length(effects.soft_edge));
    }
    x.push_str("</a:effectLst>");
}

/// The hue `a:tint` pulls a picture towards for a colour temperature:
/// orange for a warm one, blue for a cold one (60 000ths of a degree).
const WARM_HUE: i64 = 30 * 60_000;
const COLD_HUE: i64 = 210 * 60_000;
/// The kelvins between the neutral temperature and either end.
const TEMPERATURE_SPAN: f32 = 5000.0;

/// The effects inside `<a:blip>` that say how a picture is adjusted.
fn push_blip_effects(x: &mut String, adjust: &PictureAdjust) {
    if adjust.transparency > 0.0 {
        w!(x, "<a:alphaModFix amt=\"{}\"/>", per_100k(1.0 - adjust.transparency.min(1.0)));
    }
    let (mut bright, mut contrast) = (adjust.brightness, adjust.contrast);
    match adjust.recolor {
        Recolor::None => {}
        Recolor::Grayscale => x.push_str("<a:grayscl/>"),
        Recolor::BlackAndWhite(threshold) => {
            w!(x, "<a:biLevel thresh=\"{}\"/>", per_100k(threshold.clamp(0.0, 1.0)));
        }
        Recolor::Duotone(dark, light) => {
            x.push_str("<a:duotone>");
            push_color_alpha(x, dark, 0.0);
            push_color_alpha(x, light, 0.0);
            x.push_str("</a:duotone>");
        }
        // Excel's own spelling of it: brighter, with less contrast.
        Recolor::Washout => {
            bright = WASHOUT.0;
            contrast = WASHOUT.1;
        }
    }
    if adjust.saturation != 1.0 {
        w!(
            x,
            "<a:hsl hue=\"0\" sat=\"{}\" lum=\"0\"/>",
            per_100k((adjust.saturation - 1.0).clamp(-1.0, 3.0))
        );
    }
    if adjust.temperature != NEUTRAL_TEMPERATURE {
        let warmth = (adjust.temperature - NEUTRAL_TEMPERATURE) / TEMPERATURE_SPAN;
        let hue = if warmth > 0.0 { WARM_HUE } else { COLD_HUE };
        w!(x, "<a:tint hue=\"{hue}\" amt=\"{}\"/>", per_100k(warmth.abs().min(1.0)));
    }
    if bright != 0.0 || contrast != 0.0 {
        x.push_str("<a:lum");
        if bright != 0.0 {
            w!(x, " bright=\"{}\"", per_100k(bright.clamp(-1.0, 1.0)));
        }
        if contrast != 0.0 {
            w!(x, " contrast=\"{}\"", per_100k(contrast.clamp(-1.0, 1.0)));
        }
        x.push_str("/>");
    }
}

/// The brightness and the contrast Excel writes for "Washout".
const WASHOUT: (f32, f32) = (0.7, -0.7);

/// `image` is the relationship of the drawing part to the image file.
fn push_picture(
    x: &mut String,
    drawing: &Drawing,
    picture: &Picture,
    id: usize,
    frame: Frame,
    image: usize,
) {
    x.push_str("<xdr:pic><xdr:nvPicPr>");
    push_identity(x, drawing, id);
    push_non_visual(x, "cNvPicPr", "", "picLocks", drawing);
    w!(
        x,
        "</xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r=\"{NS_DOC_REL}\" r:embed=\"rId{image}\""
    );
    let mut effects = String::new();
    push_blip_effects(&mut effects, &picture.adjust);
    if effects.is_empty() {
        x.push_str("/>");
    } else {
        w!(x, ">{effects}</a:blip>");
    }
    // Only the sides that are cropped are named, as Excel does.
    if picture.adjust.crop != [0.0; 4] {
        x.push_str("<a:srcRect");
        for (side, amount) in ["l", "t", "r", "b"].into_iter().zip(picture.adjust.crop) {
            if amount != 0.0 {
                w!(x, " {side}=\"{}\"", per_100k(amount));
            }
        }
        x.push_str("/>");
    }
    x.push_str("<a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr>");
    push_xfrm(x, drawing, frame);
    w!(
        x,
        "<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>",
        picture.adjust.shape.ooxml_name()
    );
    push_effects(x, &drawing.effects);
    x.push_str("</xdr:spPr></xdr:pic>");
}

/// What the objects of one drawing part share while they are written:
/// the ids they take in turn, and the image files the part relates to.
struct PartWriter<'a> {
    /// The id the next object gets (`id` of its `cNvPr`).
    next_id: usize,
    /// The relationship of the first image; the charts have those before.
    first_image: usize,
    /// The image files of the part, by name in `xl/media/`.
    images: Vec<String>,
    media: &'a mut Media,
    extras: &'a mut Extras,
}

impl PartWriter<'_> {
    /// The relationship of the part to the image of a picture.
    fn image(&mut self, picture: &Picture) -> usize {
        let name = self.media.name(picture, self.extras);
        let place = self.images.iter().position(|known| *known == name);
        let place = place.unwrap_or_else(|| {
            self.images.push(name);
            self.images.len() - 1
        });
        self.first_image + place
    }

    /// A picture, a shape, or a group with its members, in `frame`.
    fn push_object(&mut self, x: &mut String, drawing: &Drawing, frame: Frame) {
        let id = self.next_id;
        self.next_id += 1;
        match &drawing.kind {
            DrawingKind::Shape(shape) => push_shape(x, drawing, shape, id, frame),
            DrawingKind::Picture(picture) => {
                let image = self.image(picture);
                push_picture(x, drawing, picture, id, frame, image);
            }
            DrawingKind::Group(group) => {
                x.push_str("<xdr:grpSp><xdr:nvGrpSpPr>");
                push_identity(x, drawing, id);
                push_non_visual(x, "cNvGrpSpPr", "", "grpSpLocks", drawing);
                x.push_str("</xdr:nvGrpSpPr><xdr:grpSpPr>");
                // The members are placed in the coordinates of the sheet:
                // the group's own box is also the box of its children.
                let mut xfrm = String::new();
                push_xfrm(&mut xfrm, drawing, frame);
                let children = format!(
                    "<a:chOff x=\"{}\" y=\"{}\"/><a:chExt cx=\"{}\" cy=\"{}\"/></a:xfrm>",
                    emu(frame.x),
                    emu(frame.y),
                    emu(frame.width),
                    emu(frame.height)
                );
                x.push_str(&xfrm.replace("</a:xfrm>", &children));
                x.push_str("</xdr:grpSpPr>");
                for member in &group.members {
                    self.push_object(x, member, group.member_frame(frame, member));
                }
                x.push_str("</xdr:grpSp>");
            }
        }
    }

    /// One anchor with its object.
    fn push_anchor(&mut self, x: &mut String, sheet: &Sheet, drawing: &Drawing) {
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
        self.push_object(x, drawing, sheet.drawing_frame(drawing));
        x.push_str("<xdr:clientData");
        if !drawing.locked {
            x.push_str(" fLocksWithSheet=\"0\"");
        }
        if !drawing.print {
            x.push_str(" fPrintsWithSheet=\"0\"");
        }
        x.push_str("/></xdr:twoCellAnchor>");
    }
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
        let known = self
            .0
            .iter()
            .find(|(data, _)| Arc::ptr_eq(data, &picture.data) || **data == *picture.data);
        if let Some((_, name)) = known {
            return name.clone();
        }
        let extension = picture.format.extension();
        let name = format!("image{}.{extension}", self.0.len() + 1);
        extras.add_default(extension, picture.format.mime());
        extras.add_part(format!("xl/media/{name}"), None, picture.data.to_vec());
        self.0.push((picture.data.clone(), name.clone()));
        name
    }
}

/// Add the pictures and shapes of `wb` to the drawing parts of its sheets,
/// `parts` (those of the sheets with charts, to begin with), and their
/// image files to the package.
pub(crate) fn add_parts(wb: &Workbook, parts: &mut Vec<DrawingPart>, extras: &mut Extras) {
    let mut media = Media::default();
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
                    chart_anchors: Vec::new(),
                });
                parts.len() - 1
            });
        // The charts come first: they have the first ids and the first
        // relationships of the part.
        let charts = sheet.charts.len();
        let mut anchors: Vec<String> = Vec::new();
        let mut writer = PartWriter {
            next_id: charts + 2,
            first_image: charts + 1,
            images: Vec::new(),
            media: &mut media,
            extras,
        };
        for drawing in &sheet.drawings {
            let mut anchor = String::new();
            writer.push_anchor(&mut anchor, sheet, drawing);
            anchors.push(anchor);
        }
        let images = writer.images;
        let mut rels = String::new();
        for (k, name) in images.iter().enumerate() {
            push_relationship(
                &mut rels,
                charts + 1 + k,
                &format!("{NS_DOC_REL}/image"),
                &format!("../media/{name}"),
            );
        }
        // The anchors of the part are in stacking order, bottom-most
        // first, whatever they hold: those of the charts are taken out
        // and put back among the others.
        let part = &mut parts[at];
        let chart_anchors: Vec<String> = part
            .chart_anchors
            .iter()
            .map(|range| part.xml[range.clone()].to_string())
            .collect();
        if let Some(first) = part.chart_anchors.first() {
            part.xml.truncate(first.start);
            part.xml.push_str(ROOT_END);
        }
        part.chart_anchors.clear();
        let mut woven = String::new();
        for object in sheet.object_stack() {
            let anchor = match object {
                ObjectRef::Chart(id) => sheet
                    .charts
                    .iter()
                    .position(|chart| chart.id == id)
                    .and_then(|i| chart_anchors.get(i)),
                ObjectRef::Drawing(id) => sheet
                    .drawings
                    .iter()
                    .position(|drawing| drawing.id == id)
                    .and_then(|i| anchors.get(i)),
            };
            woven.push_str(anchor.map_or("", |anchor| anchor));
        }
        insert_before_end(&mut part.xml, ROOT_END, &woven);
        insert_before_end(&mut part.rels, RELS_END, &rels);
    }
    // Slicers and timelines are frames of the same parts, after the rest.
    for (index, sheet) in wb.sheets.iter().enumerate() {
        // Their ids come after those of the charts, the pictures, the
        // shapes (a group takes one, and one for each of its members) and
        // the strokes drawn by hand.
        let objects: usize = sheet.drawings.iter().map(|drawing| drawing.object_count()).sum();
        let anchors = super::slicer::anchors(sheet, sheet.charts.len() + objects + sheet.inks.len() + 2);
        if anchors.is_empty() {
            continue;
        }
        let at = parts.iter().position(|part| part.sheet == index).unwrap_or_else(|| {
            parts.push(DrawingPart {
                sheet: index,
                number: 0,
                xml: format!("{XML_DECL}<xdr:wsDr xmlns:xdr=\"{NS_SHEET_DRAWING}\" xmlns:a=\"{NS_DRAWING}\">{ROOT_END}"),
                rels: format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\">{RELS_END}"),
                charts: Vec::new(),
                chart_anchors: Vec::new(),
            });
            parts.len() - 1
        });
        insert_before_end(&mut parts[at].xml, ROOT_END, &anchors);
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
    parse_color_alpha(r, tag, palette).map(|(color, _)| color)
}

/// `parse_color` with how opaque the colour is (1 when nothing says).
fn parse_color_alpha<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    palette: &Palette,
) -> Option<(Color, f32)> {
    let mut color = None;
    let mut alpha = 1.0;
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
                    "alpha" if color.is_none() => alpha = amount as f32,
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
    color.map(|color| (color, alpha))
}

/// `<a:effectLst>`: the effects the engine draws; the others are left.
fn parse_effects<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Effects {
    let mut effects = Effects::default();
    let points = |t: &Tag, name: &str| {
        let emu = t.attr_f64(name).filter(|v| v.is_finite()).unwrap_or(0.0);
        (emu / EMU_PER_PT).clamp(0.0, 10_000.0) as f32
    };
    each_child(r, tag, |r, t| match t.name {
        "outerShdw" => {
            let angle = t.attr_f64("dir").filter(|v| v.is_finite()).unwrap_or(0.0);
            let size = t.attr_f64("sx").filter(|v| v.is_finite() && *v > 0.0);
            let mut shadow = Shadow {
                blur: points(t, "blurRad"),
                distance: points(t, "dist"),
                angle: (angle / ANGLE_UNITS).rem_euclid(360.0) as f32,
                size: size.map_or(1.0, |v| (v / 100_000.0).clamp(0.01, 10.0) as f32),
                color: Color::BLACK,
                transparency: 0.0,
            };
            if let Some((color, alpha)) = parse_color_alpha(r, t, palette) {
                shadow.color = color;
                shadow.transparency = 1.0 - alpha;
            }
            effects.shadow = Some(shadow);
        }
        "glow" => {
            let radius = points(t, "rad");
            if let Some((color, alpha)) = parse_color_alpha(r, t, palette) {
                effects.glow = Some(Glow {
                    color,
                    transparency: 1.0 - alpha,
                    radius,
                });
            }
        }
        "softEdge" => {
            effects.soft_edge = points(t, "rad");
            r.skip(t);
        }
        _ => r.skip(t),
    });
    effects
}

/// The effects inside `<a:blip>`: how the picture is adjusted.
fn parse_blip_effects<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    palette: &Palette,
    adjust: &mut PictureAdjust,
) {
    let fraction = |t: &Tag, name: &str| {
        t.attr_f64(name)
            .filter(|v| v.is_finite())
            .map(|v| (v / 100_000.0) as f32)
    };
    each_child(r, tag, |r, t| match t.name {
        "alphaModFix" => {
            adjust.transparency = 1.0 - fraction(t, "amt").unwrap_or(1.0).clamp(0.0, 1.0);
            r.skip(t);
        }
        "grayscl" => {
            adjust.recolor = Recolor::Grayscale;
            r.skip(t);
        }
        "biLevel" => {
            adjust.recolor =
                Recolor::BlackAndWhite(fraction(t, "thresh").unwrap_or(0.5).clamp(0.0, 1.0));
            r.skip(t);
        }
        "duotone" => {
            let mut colors = Vec::new();
            // Each child is a colour: read as the one colour of a parent.
            each_child(r, t, |r, c| {
                let rgb = match c.name {
                    "srgbClr" => c.attr("val").and_then(|v| parse_rgb(&v)),
                    "schemeClr" => c
                        .attr("val")
                        .and_then(|v| scheme_slot(&v))
                        .map(|slot| palette.theme[slot]),
                    "prstClr" => match c.attr("val").as_deref() {
                        Some("black") => Some(0x000000),
                        Some("white") => Some(0xFFFFFF),
                        _ => None,
                    },
                    _ => None,
                };
                colors.extend(rgb.map(Color));
                r.skip(c);
            });
            if let [dark, light] = colors[..] {
                adjust.recolor = Recolor::Duotone(dark, light);
            }
        }
        "hsl" => {
            adjust.saturation = (1.0 + fraction(t, "sat").unwrap_or(0.0)).clamp(0.0, 4.0);
            r.skip(t);
        }
        "tint" => {
            let amount = fraction(t, "amt").unwrap_or(0.0).clamp(0.0, 1.0);
            let hue = t.attr_f64("hue").filter(|v| v.is_finite()).unwrap_or(0.0) as i64;
            // Towards the reds and yellows: warmer.
            let warm = !(90 * 60_000..270 * 60_000).contains(&hue.rem_euclid(360 * 60_000));
            let sign = if warm { 1.0 } else { -1.0 };
            adjust.temperature = NEUTRAL_TEMPERATURE + sign * amount * TEMPERATURE_SPAN;
            r.skip(t);
        }
        "lum" => {
            let bright = fraction(t, "bright").unwrap_or(0.0).clamp(-1.0, 1.0);
            let contrast = fraction(t, "contrast").unwrap_or(0.0).clamp(-1.0, 1.0);
            if (bright, contrast) == WASHOUT && adjust.recolor == Recolor::None {
                adjust.recolor = Recolor::Washout;
            } else {
                adjust.brightness = bright;
                adjust.contrast = contrast;
            }
            r.skip(t);
        }
        _ => r.skip(t),
    });
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

/// The properties of the first run of a text, which stand for the whole
/// text here.
struct RawRun {
    font: Font,
    /// The run gives its colour itself.
    colored: bool,
}

fn parse_run_properties<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawRun {
    let mut font = Font::default();
    if let Some(size) = tag.attr_f64("sz").filter(|size| size.is_finite()) {
        font.size = (size / 50.0).round().clamp(2.0, 818.0) as u16;
    }
    font.bold = tag.attr_bool("b").unwrap_or(false);
    font.italic = tag.attr_bool("i").unwrap_or(false);
    font.underline = match tag.attr("u").as_deref() {
        None | Some("none") => Underline::None,
        Some("dbl" | "wavyDbl") => Underline::Double,
        Some(_) => Underline::Single,
    };
    font.strike = matches!(
        tag.attr("strike").as_deref(),
        Some("sngStrike" | "dblStrike")
    );
    let mut paint = Paint::Unset;
    each_child(r, tag, |r, t| match t.name {
        "latin" => {
            // "+mn-lt" and the like name a font of the theme.
            if let Some(name) = t
                .attr("typeface")
                .filter(|name| !name.is_empty() && !name.starts_with('+'))
            {
                font.name = Arc::from(name.as_ref());
            }
            r.skip(t);
        }
        _ => parse_paint(r, t, palette, &mut paint),
    });
    if let Paint::Color(color) = paint {
        font.color = Some(color);
    }
    RawRun {
        font,
        colored: !matches!(paint, Paint::Unset),
    }
}

/// `<xdr:txBody>` as a file has it.
#[derive(Default)]
struct RawText {
    text: String,
    run: Option<RawRun>,
    align: Option<TextAlign>,
    anchor: Option<TextAnchor>,
}

fn parse_paragraph<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, out: &mut RawText) {
    each_child(r, tag, |r, t| match t.name {
        "pPr" => {
            if out.align.is_none() {
                out.align = Some(
                    t.attr("algn")
                        .and_then(|name| TextAlign::from_ooxml_name(&name))
                        .unwrap_or_default(),
                );
            }
            r.skip(t);
        }
        // A run, or a field shown as the text it had when saved.
        "r" | "fld" => each_child(r, t, |r, c| match c.name {
            "rPr" => {
                let run = parse_run_properties(r, c, palette);
                out.run.get_or_insert(run);
            }
            "t" => out.text.push_str(&r.text(c)),
            _ => r.skip(c),
        }),
        "br" => {
            out.text.push('\n');
            r.skip(t);
        }
        "endParaRPr" => {
            let run = parse_run_properties(r, t, palette);
            out.run.get_or_insert(run);
        }
        _ => r.skip(t),
    });
}

fn parse_text<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawText {
    let mut out = RawText::default();
    let mut paragraphs = 0;
    each_child(r, tag, |r, t| match t.name {
        "bodyPr" => {
            out.anchor = Some(
                t.attr("anchor")
                    .and_then(|name| TextAnchor::from_ooxml_name(&name))
                    .unwrap_or_default(),
            );
            r.skip(t);
        }
        "p" => {
            if paragraphs > 0 {
                out.text.push('\n');
            }
            paragraphs += 1;
            parse_paragraph(r, t, palette, &mut out);
        }
        _ => r.skip(t),
    });
    out
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
    /// The relationship of the drawing part to the image.
    Picture(String),
    Shape(Shape),
    /// The members of a group, with the box their places are given in
    /// (`chOff` and `chExt`, in pixels).
    Group {
        members: Vec<RawObject>,
        child_box: Option<(f64, f64, f64, f64)>,
    },
}

struct RawObject {
    name: String,
    alt_text: String,
    hidden: bool,
    lock_aspect: bool,
    rotation: f32,
    flip_h: bool,
    flip_v: bool,
    /// The box `a:xfrm` gives the object, in pixels: where a member of a
    /// group is among the others (an object that stands alone is where
    /// its anchor says).
    xfrm_box: Option<(f64, f64, f64, f64)>,
    effects: Effects,
    /// How a picture is adjusted.
    adjust: PictureAdjust,
    kind: RawKind,
}

impl RawObject {
    fn new(kind: RawKind) -> RawObject {
        RawObject {
            name: String::new(),
            alt_text: String::new(),
            hidden: false,
            lock_aspect: false,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            xfrm_box: None,
            effects: Effects::default(),
            adjust: PictureAdjust::default(),
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
                self.hidden = t.attr_bool("hidden").unwrap_or(false);
                r.skip(t);
            }
            "cNvSpPr" | "cNvPicPr" | "cNvCxnSpPr" | "cNvGrpSpPr" => {
                text_box = t.attr_bool("txBox").unwrap_or(false);
                each_child(r, t, |r, locks| {
                    if matches!(locks.name, "spLocks" | "picLocks" | "cxnSpLocks" | "grpSpLocks") {
                        self.lock_aspect = locks.attr_bool("noChangeAspect").unwrap_or(false);
                    }
                    r.skip(locks);
                });
            }
            _ => r.skip(t),
        });
        text_box
    }

    /// `<a:xfrm>`: how the object is turned and the box it is given
    /// (where an object that stands alone is, its anchor says). Returns
    /// the box of the children of a group (`chOff`, `chExt`).
    fn parse_xfrm<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<(f64, f64, f64, f64)> {
        let angle = tag.attr_f64("rot").filter(|angle| angle.is_finite());
        self.rotation = angle.map_or(0.0, |angle| (angle / ANGLE_UNITS).rem_euclid(360.0) as f32);
        self.flip_h = tag.attr_bool("flipH").unwrap_or(false);
        self.flip_v = tag.attr_bool("flipV").unwrap_or(false);
        let pair = |t: &Tag, a: &str, b: &str| {
            let length = |name: &str| t.attr_f64(name).map_or(0.0, px);
            (length(a), length(b))
        };
        let (mut off, mut ext, mut child_off, mut child_ext) = (None, None, None, None);
        each_child(r, tag, |r, t| {
            match t.name {
                "off" => off = Some(pair(t, "x", "y")),
                "ext" => ext = Some(pair(t, "cx", "cy")),
                "chOff" => child_off = Some(pair(t, "x", "y")),
                "chExt" => child_ext = Some(pair(t, "cx", "cy")),
                _ => {}
            }
            r.skip(t);
        });
        if let (Some((x, y)), Some((width, height))) = (off, ext) {
            self.xfrm_box = Some((x, y, width, height));
        }
        match (child_off, child_ext) {
            (Some((x, y)), Some((width, height))) => Some((x, y, width, height)),
            _ => None,
        }
    }
}

/// A group with the pictures, shapes and groups in it; the charts and
/// the other things a group may hold are left out.
fn parse_group<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme, depth: usize) -> RawObject {
    let mut object = RawObject::new(RawKind::Picture(String::new()));
    let mut members = Vec::new();
    let mut child_box = None;
    each_child(r, tag, |r, t| match t.name {
        "nvGrpSpPr" => {
            object.parse_identity(r, t);
        }
        "grpSpPr" => each_child(r, t, |r, c| match c.name {
            "xfrm" => child_box = object.parse_xfrm(r, c),
            _ => r.skip(c),
        }),
        _ => {
            let mut member = None;
            parse_content(r, t, theme, depth + 1, &mut member);
            members.extend(member);
        }
    });
    object.kind = RawKind::Group { members, child_box };
    object
}

fn parse_picture<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme) -> Option<RawObject> {
    let palette = theme.palette;
    let mut object = RawObject::new(RawKind::Picture(String::new()));
    let mut embed = None;
    each_child(r, tag, |r, t| match t.name {
        "nvPicPr" => {
            object.parse_identity(r, t);
        }
        "blipFill" => each_child(r, t, |r, c| match c.name {
            "blip" => {
                embed = c.attr_local("embed").map(|id| id.into_owned());
                parse_blip_effects(r, c, palette, &mut object.adjust);
            }
            "srcRect" => {
                let side = |name: &str| {
                    let amount = c.attr_f64(name).filter(|v| v.is_finite()).unwrap_or(0.0);
                    (amount / 100_000.0).clamp(-10.0, 0.99) as f32
                };
                let crop = [side("l"), side("t"), side("r"), side("b")];
                // A crop that leaves nothing is no crop.
                if crop[0] + crop[2] < 0.995 && crop[1] + crop[3] < 0.995 {
                    object.adjust.crop = crop;
                }
                r.skip(c);
            }
            _ => r.skip(c),
        }),
        "spPr" => each_child(r, t, |r, c| match c.name {
            "xfrm" => {
                object.parse_xfrm(r, c);
            }
            "prstGeom" => {
                let preset = c.attr("prst").and_then(|name| ShapePreset::from_ooxml_name(&name));
                // A picture cut to a line would show nothing.
                object.adjust.shape = preset.filter(|preset| !preset.is_line()).unwrap_or(ShapePreset::Rect);
                r.skip(c);
            }
            "effectLst" => object.effects = parse_effects(r, c, palette),
            _ => r.skip(c),
        }),
        _ => r.skip(t),
    });
    // A picture that is only linked has no image in the file.
    object.kind = RawKind::Picture(embed.filter(|id| !id.is_empty())?);
    Some(object)
}

fn parse_shape<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, theme: &Theme) -> RawObject {
    let palette = theme.palette;
    let connector = tag.name == "cxnSp";
    let mut object = RawObject::new(RawKind::Picture(String::new()));
    let mut text_box = false;
    let mut preset = None;
    let mut fill = Paint::Unset;
    let mut line: Option<RawLine> = None;
    let mut style = RawStyle::default();
    let mut text = RawText::default();
    each_child(r, tag, |r, t| match t.name {
        "nvSpPr" | "nvCxnSpPr" => text_box = object.parse_identity(r, t),
        "spPr" => each_child(r, t, |r, c| match c.name {
            "xfrm" => {
                object.parse_xfrm(r, c);
            }
            "prstGeom" => {
                preset = c
                    .attr("prst")
                    .and_then(|name| ShapePreset::from_ooxml_name(&name));
                r.skip(c);
            }
            "ln" => line = Some(parse_line(r, c, palette)),
            "effectLst" => object.effects = parse_effects(r, c, palette),
            _ => parse_paint(r, c, palette, &mut fill),
        }),
        "style" => style = parse_style(r, t, theme),
        "txBody" => text = parse_text(r, t, palette),
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
    let mut font = Font::default();
    if let Some(run) = text.run {
        font = run.font;
        if !run.colored {
            font.color = style.text;
        }
    }
    object.kind = RawKind::Shape(Shape {
        preset,
        text_box,
        fill: fill.filter(|_| !preset.is_line() || !connector),
        outline,
        text: printable(&text.text, true),
        font,
        align: text.align.unwrap_or(if text.anchor.is_some() {
            TextAlign::Left
        } else {
            TextAlign::Center
        }),
        anchor: text.anchor.unwrap_or(TextAnchor::Middle),
    });
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
    /// Which anchor of the part this is, counting from 1 and counting
    /// those of the charts: the place in the stacking order.
    index: u32,
}

/// Wrappers deep enough to be on purpose are not followed.
const MAX_NESTING: usize = 4;
/// Nor are groups in groups beyond what a hand would make.
const MAX_GROUPS: usize = 16;

/// Read what an anchor holds: a picture, a shape or a group of them,
/// possibly inside a choice between it and a fallback for older
/// applications. Chart frames are the charts' business.
fn parse_content<'a>(
    r: &mut Reader<'a>,
    t: &Tag<'a>,
    theme: &Theme,
    depth: usize,
    object: &mut Option<RawObject>,
) {
    match t.name {
        "pic" if object.is_none() => *object = parse_picture(r, t, theme),
        "sp" | "cxnSp" if object.is_none() => *object = Some(parse_shape(r, t, theme)),
        "grpSp" if object.is_none() && depth < MAX_GROUPS => *object = Some(parse_group(r, t, theme, depth)),
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
        index: 0,
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
    let mut index = 0;
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "wsDr" | "AlternateContent" | "Choice" => {}
                "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor" => {
                    let mut anchor = parse_anchor(&mut r, &t, theme);
                    index += 1;
                    anchor.index = index;
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
            let name = object.name.clone();
            let Some(mut drawing) = self.build(pkg, &rels, object) else {
                continue;
            };
            drawing.placement = anchor.placement;
            drawing.locked = anchor.locked;
            drawing.print = anchor.print;
            drawing.z = anchor.index;
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
                    || sheet
                        .charts
                        .iter()
                        .any(|other| other.name.eq_ignore_ascii_case(name))
            };
            let base = match (&drawing.kind, name.trim()) {
                (DrawingKind::Picture(_), "") => "Picture",
                (DrawingKind::Shape(_), "") => "Shape",
                (DrawingKind::Group(_), "") => "Group",
                (_, name) => name,
            };
            let mut name = name.clone();
            let mut number = sheet.drawings.len() + 1;
            while taken(&name) {
                name = format!("{base} {number}");
                number += 1;
            }
            drawing.name = name;
            // The members of a group are measured in its size on the
            // sheet, whatever unit the file gave their places in.
            let frame = sheet.drawing_frame(&drawing);
            drawing.measure_members_in((frame.width as f32, frame.height as f32));
            sheet.drawings.push(drawing);
        }
    }

    /// The drawing an object of a part describes, with an id of its own
    /// (and its members', for a group) but no place yet. `None` for a
    /// picture whose image is missing and for a group left with nothing.
    fn build<R: Read + Seek>(
        &mut self,
        pkg: &mut Package<R>,
        rels: &[Rel],
        object: RawObject,
    ) -> Option<Drawing> {
        let mut drawing = match object.kind {
            RawKind::Shape(shape) => Drawing::shape(shape, (0.0, 0.0)),
            RawKind::Picture(embed) => {
                let target = rels
                    .iter()
                    .find(|rel| rel.id == embed && rel.kind == "image")
                    .map(|rel| rel.target.clone())?;
                let image = self.images.entry(target).or_insert_with_key(|target| {
                    pkg.read_bytes(target).ok().flatten().and_then(Picture::new)
                });
                // The bytes are shared; how they show is each picture's.
                let mut picture = image.clone()?;
                picture.adjust = object.adjust;
                Drawing::picture(picture)
            }
            RawKind::Group { members, child_box } => {
                // Where its members are is given in a box of its own;
                // without one, in the box of the group.
                let (left, top, width, height) = child_box.or(object.xfrm_box).unwrap_or_default();
                let mut held = Vec::new();
                for member in members {
                    let place = member.xfrm_box;
                    let name = member.name.clone();
                    let Some(mut drawing) = self.build(pkg, rels, member) else {
                        continue;
                    };
                    let (x, y, w, h) = place.unwrap_or((left, top, width, height));
                    drawing.offset = ((x - left) as f32, (y - top) as f32);
                    drawing.size = (w.clamp(0.0, MAX_PX) as f32, h.clamp(0.0, MAX_PX) as f32);
                    drawing.name = name;
                    held.push(drawing);
                }
                if held.is_empty() {
                    return None;
                }
                Drawing::group(held, (width.clamp(0.0, MAX_PX) as f32, height.clamp(0.0, MAX_PX) as f32))
            }
        };
        // After its members: an object that is left out takes no id.
        drawing.id = self.next_id;
        self.next_id += 1;
        drawing.rotation = object.rotation;
        drawing.flip_h = object.flip_h;
        drawing.flip_v = object.flip_v;
        drawing.lock_aspect = object.lock_aspect;
        drawing.alt_text = object.alt_text;
        drawing.hidden = object.hidden;
        drawing.effects = object.effects;
        Some(drawing)
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
