//! What the application paints on paper itself: charts, pictures and
//! shapes, and the data bars and icons of conditional formats
//! (`opensheet_print::ObjectPainter`).
//!
//! The canvas handed to every method is in sheet pixels at 100 % with its
//! origin on the object, so the numbers are those of the screen.

use crate::features::drawings::geometry;
use gpui::RenderImage;
use opensheet_engine::chart::Chart;
use opensheet_engine::chart_layout::{self, Ink, LayoutOptions, Prim, Seg, TextAnchor, layout_chart};
use opensheet_engine::condfmt::{Adornment, Bar, Icon};
use opensheet_engine::drawing::{
    Drawing, DrawingKind, ImageFormat, PathCommand, Picture, Shape, TEXT_MARGIN, TextAlign,
    TextAnchor as ShapeAnchor,
};
use opensheet_engine::{Color, StyleId, Workbook};
use opensheet_print::{Canvas, FontDb, Image, LineCap, LineJoin, ObjectPainter, Paint, Path, RectF, Stroke, TextStyle};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Measures chart text with the fonts of the printer.
struct Measure<'a> {
    fonts: &'a FontDb,
    family: &'a str,
}

impl chart_layout::TextMeasure for Measure<'_> {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        self.width_styled(text, size, bold, false, None)
    }

    fn width_styled(&self, text: &str, size: f32, bold: bool, italic: bool, family: Option<&str>) -> f32 {
        let face = self.fonts.face(family.unwrap_or(self.family), bold, italic);
        self.fonts.text_width(&face, text, size as f64) as f32
    }
}

fn paint_of(ink: Ink) -> Paint {
    Paint::translucent(ink.color, ink.opacity)
}

fn stroke_of(stroke: &chart_layout::Stroke) -> Stroke {
    let width = stroke.width as f64;
    let dash: Vec<f64> = stroke.dash.pattern().iter().map(|d| *d as f64 * width).collect();
    Stroke::solid(paint_of(stroke.ink), width)
        .dashed(&dash)
        .join(LineJoin::Round)
}

fn path_of(segs: &[Seg]) -> Path {
    let mut path = Path::new();
    for seg in segs {
        path = match *seg {
            Seg::Move(x, y) => path.move_to(x as f64, y as f64),
            Seg::Line(x, y) => path.line_to(x as f64, y as f64),
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                path.curve_to(ax as f64, ay as f64, bx as f64, by as f64, x as f64, y as f64)
            }
            Seg::Close => path.close(),
        };
    }
    path
}

/// Paints charts, drawings and cell adornments for a print job.
pub(crate) struct PaperPainter {
    /// Decodes the pictures the print crate cannot read by itself (GIF,
    /// BMP, interlaced PNG) with the window system's decoders.
    decode: Box<dyn Fn(gpui::Image) -> Option<Arc<RenderImage>>>,
    /// Pictures already decoded, by the address of their file bytes.
    images: RefCell<HashMap<usize, Option<Arc<Image>>>>,
}

impl PaperPainter {
    pub(crate) fn new(cx: &gpui::App) -> PaperPainter {
        let svg = cx.svg_renderer();
        PaperPainter {
            decode: Box::new(move |image| image.to_image_data(svg.clone()).ok()),
            images: RefCell::new(HashMap::new()),
        }
    }

    /// A painter for where there is no window (tests): pictures the print
    /// crate cannot read by itself are left out.
    #[cfg(test)]
    pub(crate) fn headless() -> PaperPainter {
        PaperPainter {
            decode: Box::new(|_| None),
            images: RefCell::new(HashMap::new()),
        }
    }

    fn image(&self, picture: &Picture) -> Option<Arc<Image>> {
        let key = picture.data.as_ptr() as usize;
        self.images
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                Image::from_file_bytes(&picture.data)
                    .or_else(|| self.decode(picture))
                    .map(Arc::new)
            })
            .clone()
    }

    /// Decode a picture the print crate cannot read.
    fn decode(&self, picture: &Picture) -> Option<Image> {
        let format = match picture.format {
            ImageFormat::Png => gpui::ImageFormat::Png,
            ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
            ImageFormat::Gif => gpui::ImageFormat::Gif,
            ImageFormat::Bmp => gpui::ImageFormat::Bmp,
        };
        let decoded = (self.decode)(gpui::Image::from_bytes(format, picture.data.to_vec()))?;
        let size = decoded.size(0);
        // The frame comes back as blue, green, red, alpha.
        let mut pixels = decoded.as_bytes(0)?.to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Image::from_rgba(i32::from(size.width) as u32, i32::from(size.height) as u32, pixels)
    }
}

impl ObjectPainter for PaperPainter {
    fn paint_chart(&self, wb: &Workbook, sheet: usize, chart: &Chart, size: (f64, f64), canvas: &mut Canvas) {
        let data = wb.chart_data(sheet, chart);
        let family = wb.styles.get(StyleId(0)).font.name.clone();
        let fonts = canvas.fonts();
        let measure = Measure { fonts, family: &family };
        // The canvas is in sheet pixels: the chart is laid out as on
        // screen at 100 %.
        let options = LayoutOptions {
            scale: 1.0,
            locale: wb.locale,
            labels: crate::chart_view::chart_labels(),
        };
        for prim in layout_chart(chart, &data, (size.0 as f32, size.1 as f32), &options, &measure) {
            match prim {
                Prim::Rect {
                    x,
                    y,
                    w,
                    h,
                    fill,
                    stroke,
                } => {
                    let rect = RectF::new(x as f64, y as f64, w as f64, h as f64);
                    if let Some(fill) = fill {
                        canvas.fill_rect(rect, paint_of(fill));
                    }
                    if let Some(stroke) = stroke {
                        canvas.stroke_rect(rect, stroke_of(&stroke));
                    }
                }
                Prim::Line { x0, y0, x1, y1, stroke } => {
                    canvas.line(x0 as f64, y0 as f64, x1 as f64, y1 as f64, stroke_of(&stroke));
                }
                Prim::Path { segs, fill, stroke } => {
                    if let Some(fill) = fill {
                        canvas.fill_path(path_of(&segs), paint_of(fill));
                    }
                    if let Some(stroke) = stroke {
                        canvas.stroke_path(path_of(&segs), stroke_of(&stroke));
                    }
                }
                Prim::Text(run) => {
                    let face = fonts.face(run.font.as_deref().unwrap_or(&family), run.bold, run.italic);
                    let size = run.size as f64;
                    let width = fonts.text_width(&face, &run.text, size);
                    let left = match run.anchor {
                        TextAnchor::Start => 0.0,
                        TextAnchor::Middle => -width / 2.0,
                        TextAnchor::End => -width,
                    };
                    // `y` is the middle of the line: the glyphs sit around it.
                    let baseline = (face.ascent() - face.descent()) * size / 2.0;
                    canvas.save();
                    canvas.translate(run.x as f64, run.y as f64);
                    if run.rotation != 0.0 {
                        // The layout turns clockwise, the canvas the other way.
                        canvas.rotate(-run.rotation as f64);
                    }
                    canvas.face_text(&face, &run.text, left, baseline, size, Paint::from(run.color));
                    if run.underline {
                        let under = baseline + size * 0.12;
                        canvas.line(left, under, left + width, under, Stroke::solid(Paint::from(run.color), (size / 14.0).max(0.5)));
                    }
                    canvas.restore();
                }
            }
        }
    }

    fn paint_drawing(&self, wb: &Workbook, _sheet: usize, drawing: &Drawing, size: (f64, f64), canvas: &mut Canvas) {
        let (w, h) = size;
        if w <= 0.0 && h <= 0.0 {
            return;
        }
        // Turned about the centre of the frame, after the flips.
        let turn = |canvas: &mut Canvas, flips: bool| {
            canvas.translate(w / 2.0, h / 2.0);
            if drawing.rotation != 0.0 {
                canvas.rotate(-drawing.rotation as f64);
            }
            if flips {
                canvas.scale(
                    if drawing.flip_h { -1.0 } else { 1.0 },
                    if drawing.flip_v { -1.0 } else { 1.0 },
                );
            }
            canvas.translate(-w / 2.0, -h / 2.0);
        };
        match &drawing.kind {
            DrawingKind::Picture(picture) => {
                // As on screen: the pixels the frame shows, adjusted,
                // cut to their outline, with what the picture wears
                // (`drawings::render`), in a box about the frame.
                use crate::features::drawings::picture as pixels;
                let natural = (w as f32, h as f32);
                let dress = crate::features::drawings::dress_for_page(drawing, picture);
                let shorter = natural.0.min(natural.1).max(1.0);
                let soft = (crate::grid::pt_to_px(drawing.effects.soft_edge.max(0.0)) / shorter).min(0.5);
                let made = pixels::printed(picture, soft, natural, (drawing.flip_h, drawing.flip_v), &dress)
                    .and_then(|(bitmap, share)| Some((Image::from_rgba(bitmap.width, bitmap.height, bitmap.data)?, share)));
                if let Some((image, share)) = made {
                    canvas.save();
                    turn(canvas, false);
                    let (bw, bh) = (w / share.0 as f64, h / share.1 as f64);
                    canvas.image(&Arc::new(image), RectF::new((w - bw) / 2.0, (h - bh) / 2.0, bw, bh));
                    canvas.restore();
                } else if let Some(image) = self.image(picture) {
                    // A format only the window system decodes: the
                    // image as it is, cropped.
                    canvas.save();
                    turn(canvas, true);
                    let whole = picture.adjust.uncropped(opensheet_engine::drawing::Frame::new(0.0, 0.0, w, h));
                    canvas.clip_rect(RectF::new(0.0, 0.0, w, h));
                    canvas.image(&image, RectF::new(whole.x, whole.y, whole.width, whole.height));
                    canvas.restore();
                }
            }
            // The members of a group, each where the group puts it.
            DrawingKind::Group(_) => {
                for leaf in drawing.leaves(opensheet_engine::drawing::Frame::new(0.0, 0.0, w, h)) {
                    let member = Drawing {
                        rotation: leaf.rotation,
                        flip_h: leaf.flip_h,
                        flip_v: leaf.flip_v,
                        ..leaf.drawing.clone()
                    };
                    canvas.save();
                    canvas.translate(leaf.frame.x, leaf.frame.y);
                    self.paint_drawing(wb, _sheet, &member, (leaf.frame.width, leaf.frame.height), canvas);
                    canvas.restore();
                }
            }
            DrawingKind::Shape(shape) => {
                canvas.save();
                turn(canvas, true);
                paint_shape(shape, w, h, canvas);
                canvas.restore();
                if !shape.text.is_empty() {
                    // Text is turned with its shape but never mirrored.
                    canvas.save();
                    turn(canvas, false);
                    paint_shape_text(wb, shape, w, h, canvas);
                    canvas.restore();
                }
            }
            // A SmartArt graphic: the shapes it is laid out as, then what
            // is written in them. (Fills that let what is under them show
            // are printed solid.)
            DrawingKind::SmartArt(art) => {
                let pieces = art.pieces(wb.theme(), w as f32, h as f32);
                for text in [false, true] {
                    for piece in &pieces {
                        let (pw, ph) = (piece.width as f64, piece.height as f64);
                        canvas.save();
                        canvas.translate(piece.x as f64 + pw / 2.0, piece.y as f64 + ph / 2.0);
                        if !text {
                            canvas.rotate(-piece.rotation as f64);
                            canvas.scale(if piece.flip_h { -1.0 } else { 1.0 }, 1.0);
                        }
                        canvas.translate(-pw / 2.0, -ph / 2.0);
                        if text {
                            if !piece.shape.text.is_empty() {
                                paint_shape_text(wb, &piece.shape, pw, ph, canvas);
                            }
                        } else {
                            paint_shape(&piece.shape, pw, ph, canvas);
                        }
                        canvas.restore();
                    }
                }
            }
        }
    }

    fn paint_adornment(
        &self,
        _wb: &Workbook,
        _sheet: usize,
        _cell: (u32, u32),
        adornment: &Adornment,
        size: (f64, f64),
        canvas: &mut Canvas,
    ) {
        if let Some(bar) = &adornment.bar {
            paint_bar(bar, size, canvas);
        }
        if let Some(icon) = adornment.icon {
            // At the left of the cell, in the middle of its height.
            let side = ICON_SIZE.min(size.1 - 2.0).max(4.0);
            paint_icon(icon.icon(), 3.0, (size.1 - side) / 2.0, side, canvas);
        }
    }
}

// ----------------------------------------------------------------- shapes

/// Sheet pixels per point.
const PX_PER_PT: f64 = 96.0 / 72.0;

fn paint_shape(shape: &Shape, w: f64, h: f64, canvas: &mut Canvas) {
    let commands = shape.path(w as f32, h as f32);
    let mut path = Path::new();
    for command in &commands {
        path = match *command {
            PathCommand::MoveTo(x, y) => path.move_to(x as f64 * w, y as f64 * h),
            PathCommand::LineTo(x, y) => path.line_to(x as f64 * w, y as f64 * h),
            PathCommand::CurveTo(ax, ay, bx, by, x, y) => path.curve_to(
                ax as f64 * w,
                ay as f64 * h,
                bx as f64 * w,
                by as f64 * h,
                x as f64 * w,
                y as f64 * h,
            ),
            PathCommand::Close => path.close(),
        };
    }
    if let Some(fill) = shape.fill.filter(|_| !shape.preset.is_line()) {
        canvas.fill_path(path.clone(), fill);
    }
    let Some(outline) = &shape.outline else {
        return;
    };
    let width = outline.width as f64 * PX_PER_PT;
    let dash: Vec<f64> = outline.dash.pattern().iter().map(|d| *d as f64 * width).collect();
    let stroke = Stroke::solid(outline.color, width).dashed(&dash).join(LineJoin::Round);
    if !shape.preset.is_line() {
        canvas.stroke_path(path, stroke);
        return;
    }
    // A line: heads at its ends, as the sheet draws them (the same
    // outlines, from `drawings::geometry`), and the line stopping under
    // them.
    let mut points: Vec<(f32, f32)> = commands
        .iter()
        .filter_map(|command| match *command {
            PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) | PathCommand::CurveTo(_, _, _, _, x, y) => {
                Some((x * w as f32, y * h as f32))
            }
            PathCommand::Close => None,
        })
        .collect();
    if points.len() < 2 {
        return;
    }
    let size = geometry::head_size(width as f32, 1.0);
    let last = points.len() - 1;
    let ends = [
        (outline.start, points[0], points[1], false),
        (outline.end, points[last], points[last - 1], true),
    ];
    for (kind, tip, from, at_end) in ends {
        let Some(head) = geometry::arrow_head(kind, tip, from, size) else {
            continue;
        };
        geometry::trim(&mut points, head.cut, at_end);
        let outline_points: Vec<(f64, f64)> = head.points.iter().map(|p| (p.0 as f64, p.1 as f64)).collect();
        if head.filled {
            canvas.fill_path(Path::polyline(&outline_points).close(), outline.color);
        } else {
            canvas.stroke_path(
                Path::polyline(&outline_points),
                Stroke::solid(outline.color, width).cap(LineCap::Round).join(LineJoin::Round),
            );
        }
    }
    let line: Vec<(f64, f64)> = points.iter().map(|p| (p.0 as f64, p.1 as f64)).collect();
    canvas.stroke_path(Path::polyline(&line), stroke);
}

/// Break text into lines no wider than `max_width`, at spaces.
fn wrap(canvas: &Canvas, text: &str, style: &TextStyle, max_width: f64) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.trim_end_matches('\r').split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && canvas.text_width(&candidate, style) > max_width {
                out.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = candidate;
            }
        }
        out.push(line);
    }
    out
}

fn paint_shape_text(wb: &Workbook, shape: &Shape, w: f64, h: f64, canvas: &mut Canvas) {
    let [left, top, right, bottom] = shape.text_rect(w as f32, h as f32);
    let (x0, x1) = (
        left as f64 * w + TEXT_MARGIN.0 as f64,
        right as f64 * w - TEXT_MARGIN.0 as f64,
    );
    let (y0, y1) = (
        top as f64 * h + TEXT_MARGIN.1 as f64,
        bottom as f64 * h - TEXT_MARGIN.1 as f64,
    );
    let size = shape.font.size_pt() as f64 * PX_PER_PT;
    let default = wb.styles.get(StyleId(0)).font.name.clone();
    let family: &str = if shape.font.name.is_empty() {
        &default
    } else {
        &shape.font.name
    };
    let style = TextStyle {
        family,
        bold: shape.font.bold,
        italic: shape.font.italic,
        size,
    };
    let lines = wrap(canvas, &shape.text, &style, (x1 - x0).max(size));
    let line_height = size * 1.2;
    let total = line_height * lines.len() as f64;
    let face = canvas.face(&style);
    let mut y = match shape.anchor {
        ShapeAnchor::Top => y0,
        ShapeAnchor::Middle => (y0 + y1 - total) / 2.0,
        ShapeAnchor::Bottom => y1 - total,
    };
    let color = shape.font.color.unwrap_or(Color::BLACK);
    for line in lines {
        let width = canvas.text_width(&line, &style);
        let x = match shape.align {
            TextAlign::Left | TextAlign::Justify => x0,
            TextAlign::Center => (x0 + x1 - width) / 2.0,
            TextAlign::Right => x1 - width,
        };
        let baseline = y + (line_height - (face.ascent() + face.descent()) * size) / 2.0 + face.ascent() * size;
        canvas.text(&line, x, baseline, &style, color);
        y += line_height;
    }
}

// -------------------------------------------------- data bars and icons

/// Blank space around a data bar inside its cell, in pixels.
const BAR_INSET: f64 = 2.0;
/// Side of an icon of an icon set, in pixels.
const ICON_SIZE: f64 = 16.0;

fn mix(a: Color, b: Color, t: f64) -> Color {
    let channel = |shift: u32| {
        let (x, y) = (((a.value() >> shift) & 0xFF) as f64, ((b.value() >> shift) & 0xFF) as f64);
        ((x + (y - x) * t).round() as u32) << shift
    };
    Color(channel(16) | channel(8) | channel(0))
}

fn paint_bar(bar: &Bar, size: (f64, f64), canvas: &mut Canvas) {
    let (w, h) = size;
    let room = (w - 2.0 * BAR_INSET).max(0.0);
    let at = |fraction: f32| BAR_INSET + fraction.clamp(0.0, 1.0) as f64 * room;
    let (from, to) = (at(bar.start), at(bar.end));
    let (top, height) = (BAR_INSET, (h - 2.0 * BAR_INSET).max(0.0));
    let (x0, x1) = (from.min(to), from.max(to));
    if x1 > x0 && height > 0.0 {
        if bar.gradient {
            // The colour fades towards the tip of the bar, in strips.
            let strips = 24;
            for k in 0..strips {
                let (a, b) = (k as f64 / strips as f64, (k + 1) as f64 / strips as f64);
                let (sa, sb) = (from + (to - from) * a, from + (to - from) * b);
                let color = mix(bar.fill, Color::WHITE, (a + b) / 2.0 * 0.9);
                // A hair of overlap hides the seams between strips.
                canvas.fill_rect(RectF::new(sa.min(sb) - 0.05, top, (sb - sa).abs() + 0.1, height), color);
            }
        } else {
            canvas.fill_rect(RectF::new(x0, top, x1 - x0, height), bar.fill);
        }
        if let Some(border) = bar.border {
            canvas.stroke_rect(RectF::new(x0, top, x1 - x0, height), Stroke::solid(border, 0.75));
        }
    }
    if let Some(axis) = bar.axis {
        let x = at(axis);
        canvas.line(x, 0.0, x, h, Stroke::solid(bar.axis_color, 0.75).dashed(&[2.0, 1.5]));
    }
}

const GREEN: Color = Color(0x3DAE49);
const YELLOW: Color = Color(0xF2B800);
const RED: Color = Color(0xE0443A);
const GRAY: Color = Color(0x7F7F7F);
const DARK: Color = Color(0x404040);
const PINK: Color = Color(0xF4A7A3);
const GOLD: Color = Color(0xF7C331);

/// An arrow pointing up, turned by `degrees` clockwise, in a square of
/// side `s` whose corner is the origin.
fn arrow(canvas: &mut Canvas, s: f64, degrees: f64, color: Color) {
    canvas.save();
    canvas.translate(s / 2.0, s / 2.0);
    canvas.rotate(-degrees);
    let u = s / 16.0;
    let points = [
        (0.0, -7.0 * u),
        (6.0 * u, -0.5 * u),
        (2.2 * u, -0.5 * u),
        (2.2 * u, 7.0 * u),
        (-2.2 * u, 7.0 * u),
        (-2.2 * u, -0.5 * u),
        (-6.0 * u, -0.5 * u),
    ];
    canvas.fill_path(Path::polyline(&points).close(), color);
    canvas.restore();
}

fn disc(canvas: &mut Canvas, s: f64, inset: f64, color: Color) {
    canvas.fill_path(
        Path::ellipse(RectF::new(inset, inset, s - 2.0 * inset, s - 2.0 * inset)),
        color,
    );
}

fn ring(canvas: &mut Canvas, s: f64, color: Color) {
    canvas.stroke_path(
        Path::ellipse(RectF::new(1.5, 1.5, s - 3.0, s - 3.0)),
        Stroke::solid(color, s / 16.0),
    );
}

/// A mark drawn with strokes: a check, a cross or an exclamation mark.
fn mark(canvas: &mut Canvas, s: f64, kind: u8, color: Color, heavy: bool) {
    let u = s / 16.0;
    let stroke = || {
        Stroke::solid(color, if heavy { 2.6 * u } else { 2.0 * u })
            .cap(LineCap::Round)
            .join(LineJoin::Round)
    };
    match kind {
        0 => canvas.stroke_path(
            Path::polyline(&[(4.0 * u, 8.5 * u), (7.0 * u, 11.5 * u), (12.0 * u, 4.5 * u)]),
            stroke(),
        ),
        1 => {
            canvas.line(4.5 * u, 4.5 * u, 11.5 * u, 11.5 * u, stroke());
            canvas.line(11.5 * u, 4.5 * u, 4.5 * u, 11.5 * u, stroke());
        }
        _ => {
            canvas.line(8.0 * u, 3.5 * u, 8.0 * u, 9.0 * u, stroke());
            canvas.fill_path(Path::ellipse(RectF::new(6.7 * u, 10.8 * u, 2.6 * u, 2.6 * u)), color);
        }
    }
}

fn star(canvas: &mut Canvas, s: f64, fill: Option<Color>, half: bool) {
    let (cx, cy, outer, inner) = (s / 2.0, s / 2.0 + s / 32.0, s * 0.47, s * 0.19);
    let points: Vec<(f64, f64)> = (0..10)
        .map(|k| {
            let angle = (-90.0 + 36.0 * k as f64).to_radians();
            let r = if k % 2 == 0 { outer } else { inner };
            (cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect();
    let outline = Path::polyline(&points).close();
    match (fill, half) {
        (Some(color), false) => canvas.fill_path(outline.clone(), color),
        (Some(color), true) => {
            // The left half only.
            canvas.save();
            canvas.clip_rect(RectF::new(0.0, 0.0, s / 2.0, s));
            canvas.fill_path(outline.clone(), color);
            canvas.restore();
        }
        (None, _) => {}
    }
    canvas.stroke_path(outline, Stroke::solid(Color(0xC9A227), s / 20.0).join(LineJoin::Round));
}

/// Draw an icon of an icon set in a square of side `s` at (x, y). The
/// shapes are simple renderings of Excel's: same form, same colour.
fn paint_icon(icon: Icon, x: f64, y: f64, s: f64, canvas: &mut Canvas) {
    use Icon::*;
    canvas.save();
    canvas.translate(x, y);
    let u = s / 16.0;
    match icon {
        GreenUpArrow => arrow(canvas, s, 0.0, GREEN),
        YellowSideArrow => arrow(canvas, s, 90.0, YELLOW),
        RedDownArrow => arrow(canvas, s, 180.0, RED),
        GrayUpArrow => arrow(canvas, s, 0.0, GRAY),
        GraySideArrow => arrow(canvas, s, 90.0, GRAY),
        GrayDownArrow => arrow(canvas, s, 180.0, GRAY),
        YellowUpInclineArrow => arrow(canvas, s, 45.0, YELLOW),
        YellowDownInclineArrow => arrow(canvas, s, 135.0, YELLOW),
        GrayUpInclineArrow => arrow(canvas, s, 45.0, GRAY),
        GrayDownInclineArrow => arrow(canvas, s, 135.0, GRAY),
        GreenFlag | YellowFlag | RedFlag => {
            let color = match icon {
                GreenFlag => GREEN,
                YellowFlag => YELLOW,
                _ => RED,
            };
            canvas.line(4.0 * u, 2.0 * u, 4.0 * u, 14.5 * u, Stroke::solid(DARK, 1.2 * u));
            let cloth = [(4.6 * u, 2.5 * u), (13.0 * u, 5.5 * u), (4.6 * u, 8.5 * u)];
            canvas.fill_path(Path::polyline(&cloth).close(), color);
        }
        GreenCircle => disc(canvas, s, 1.5, GREEN),
        YellowCircle => disc(canvas, s, 1.5, YELLOW),
        RedCircle => disc(canvas, s, 1.5, RED),
        PinkCircle => disc(canvas, s, 1.5, PINK),
        GrayCircle => disc(canvas, s, 1.5, GRAY),
        BlackCircle => disc(canvas, s, 1.5, DARK),
        RedCircleWithBorder => {
            disc(canvas, s, 1.5, RED);
            ring(canvas, s, Color(0x9C2B24));
        }
        BlackCircleWithBorder => {
            disc(canvas, s, 1.5, DARK);
            ring(canvas, s, Color(0x9A9A9A));
        }
        WhiteCircle => {
            disc(canvas, s, 1.5, Color::WHITE);
            ring(canvas, s, DARK);
        }
        CircleWithOneWhiteQuarter | CircleWithTwoWhiteQuarters | CircleWithThreeWhiteQuarters => {
            let black_quarters = match icon {
                CircleWithOneWhiteQuarter => 3.0,
                CircleWithTwoWhiteQuarters => 2.0,
                _ => 1.0,
            };
            disc(canvas, s, 1.5, Color::WHITE);
            let (c, r) = (s / 2.0, s / 2.0 - 1.5);
            // The black part runs clockwise from the top.
            let pie = Path::new()
                .move_to(c, c)
                .line_to(c, c - r)
                .arc(c, c, r, r, -90.0, 90.0 * black_quarters)
                .close();
            canvas.fill_path(pie, DARK);
            ring(canvas, s, DARK);
        }
        GreenTrafficLight | YellowTrafficLight | RedTrafficLight => {
            let color = match icon {
                GreenTrafficLight => GREEN,
                YellowTrafficLight => YELLOW,
                _ => RED,
            };
            canvas.fill_rect(RectF::new(1.0 * u, 1.0 * u, 14.0 * u, 14.0 * u), DARK);
            disc(canvas, s, 3.0 * u, color);
        }
        YellowTriangle => {
            let points = [(8.0 * u, 2.0 * u), (15.0 * u, 14.0 * u), (1.0 * u, 14.0 * u)];
            canvas.fill_path(Path::polyline(&points).close(), YELLOW);
        }
        GreenUpTriangle => {
            let points = [(8.0 * u, 4.0 * u), (14.0 * u, 12.0 * u), (2.0 * u, 12.0 * u)];
            canvas.fill_path(Path::polyline(&points).close(), GREEN);
        }
        RedDownTriangle => {
            let points = [(2.0 * u, 4.0 * u), (14.0 * u, 4.0 * u), (8.0 * u, 12.0 * u)];
            canvas.fill_path(Path::polyline(&points).close(), RED);
        }
        YellowDash => canvas.fill_rect(RectF::new(2.0 * u, 6.5 * u, 12.0 * u, 3.0 * u), YELLOW),
        RedDiamond => {
            let points = [
                (8.0 * u, 1.5 * u),
                (14.5 * u, 8.0 * u),
                (8.0 * u, 14.5 * u),
                (1.5 * u, 8.0 * u),
            ];
            canvas.fill_path(Path::polyline(&points).close(), RED);
        }
        GreenCheckSymbol => {
            disc(canvas, s, 1.0, GREEN);
            mark(canvas, s, 0, Color::WHITE, false);
        }
        YellowExclamationSymbol => {
            disc(canvas, s, 1.0, YELLOW);
            mark(canvas, s, 2, Color::WHITE, false);
        }
        RedCrossSymbol => {
            disc(canvas, s, 1.0, RED);
            mark(canvas, s, 1, Color::WHITE, false);
        }
        GreenCheck => mark(canvas, s, 0, GREEN, true),
        YellowExclamation => mark(canvas, s, 2, YELLOW, true),
        RedCross => mark(canvas, s, 1, RED, true),
        ZeroBars | OneBar | TwoBars | ThreeBars | FourBars => {
            let filled = match icon {
                ZeroBars => 0,
                OneBar => 1,
                TwoBars => 2,
                ThreeBars => 3,
                _ => 4,
            };
            for k in 0..4 {
                let height = (4.0 + 3.0 * k as f64) * u;
                let color = if k < filled { Color(0x4472C4) } else { Color(0xC8C8C8) };
                canvas.fill_rect(
                    RectF::new((1.5 + 3.5 * k as f64) * u, 14.5 * u - height, 2.6 * u, height),
                    color,
                );
            }
        }
        GoldStar => star(canvas, s, Some(GOLD), false),
        HalfGoldStar => star(canvas, s, Some(GOLD), true),
        SilverStar => star(canvas, s, None, false),
        FourFilledBoxes | ThreeFilledBoxes | TwoFilledBoxes | OneFilledBox | ZeroFilledBoxes => {
            let filled = match icon {
                FourFilledBoxes => 4,
                ThreeFilledBoxes => 3,
                TwoFilledBoxes => 2,
                OneFilledBox => 1,
                _ => 0,
            };
            // Filled from the bottom-left box, as Excel's ratings are.
            for (k, (bx, by)) in [(0.0, 1.0), (1.0, 1.0), (0.0, 0.0), (1.0, 0.0)].into_iter().enumerate() {
                let rect = RectF::new((2.0 + 6.5 * bx) * u, (2.0 + 6.5 * by) * u, 5.5 * u, 5.5 * u);
                if k < filled {
                    canvas.fill_rect(rect, Color(0x4472C4));
                } else {
                    canvas.fill_rect(rect, Color(0xD9D9D9));
                }
            }
        }
    }
    canvas.restore();
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_print::Op;

    #[test]
    fn colours_mix() {
        assert_eq!(mix(Color(0x000000), Color(0xFFFFFF), 0.5), Color(0x808080));
        assert_eq!(mix(Color(0x638EC6), Color::WHITE, 0.0), Color(0x638EC6));
        assert_eq!(mix(Color(0x638EC6), Color::WHITE, 1.0), Color::WHITE);
    }

    #[test]
    fn data_bars_stay_inside_the_cell() {
        let fonts = FontDb::standard_only();
        let mut canvas = Canvas::new(&fonts);
        let bar = Bar {
            start: 0.5,
            end: 0.0,
            fill: Color(0xFF0000),
            gradient: false,
            border: Some(Color(0x800000)),
            negative: true,
            axis: Some(0.5),
            axis_color: Color(0x000000),
        };
        paint_bar(&bar, (104.0, 20.0), &mut canvas);
        let ops = canvas.into_ops();
        // The bar from the axis leftwards, its outline, the axis.
        assert_eq!(ops.len(), 3);
        let Op::Fill { path, .. } = &ops[0] else {
            panic!("the bar is filled first");
        };
        assert_eq!(path.segments[0], opensheet_print::Segment::MoveTo(2.0, 2.0));
        assert_eq!(path.segments[2], opensheet_print::Segment::LineTo(52.0, 18.0));
        // A gradient bar is painted in strips.
        let mut canvas = Canvas::new(&fonts);
        paint_bar(
            &Bar {
                gradient: true,
                border: None,
                axis: None,
                ..bar
            },
            (104.0, 20.0),
            &mut canvas,
        );
        assert_eq!(canvas.into_ops().len(), 24);
    }

    #[test]
    fn every_icon_draws_something() {
        use opensheet_engine::condfmt::IconSet;
        let fonts = FontDb::standard_only();
        for set in IconSet::ALL {
            for icon in set.icons() {
                let mut canvas = Canvas::new(&fonts);
                paint_icon(*icon, 3.0, 2.0, 16.0, &mut canvas);
                let ops = canvas.into_ops();
                assert!(
                    ops.iter().any(|op| matches!(op, Op::Fill { .. } | Op::Stroke { .. })),
                    "{icon:?} draws nothing"
                );
                // Whatever was saved is restored.
                let saves = ops.iter().filter(|op| matches!(op, Op::Save)).count();
                let restores = ops.iter().filter(|op| matches!(op, Op::Restore)).count();
                assert_eq!(saves, restores);
            }
        }
    }

    #[test]
    fn shapes_are_outlined_and_their_text_wrapped() {
        use opensheet_engine::drawing::ShapeTool;
        let fonts = FontDb::standard_only();
        let wb = Workbook::new(opensheet_engine::Locale::en());
        let mut shape = ShapeTool::ALL[0].shape();
        shape.text = "one two three four five six seven eight nine ten".into();
        let mut canvas = Canvas::new(&fonts);
        paint_shape(&shape, 120.0, 80.0, &mut canvas);
        let outline = canvas.into_ops();
        assert!(outline.iter().any(|op| matches!(op, Op::Fill { .. })));
        assert!(outline.iter().any(|op| matches!(op, Op::Stroke { .. })));
        let mut canvas = Canvas::new(&fonts);
        paint_shape_text(&wb, &shape, 120.0, 80.0, &mut canvas);
        let lines: Vec<String> = canvas
            .into_ops()
            .into_iter()
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text),
                _ => None,
            })
            .collect();
        assert!(lines.len() > 1, "the text is wrapped: {lines:?}");
        assert_eq!(lines.join(" "), shape.text);
    }
    /// A report with everything the application paints on paper: a chart,
    /// a shape with text, a picture, data bars and icons, and notes.
    /// Set `OPENSHEET_PRINT_OUT` to a directory to keep the PDF and the
    /// workbook (to look at them, and to open the workbook in the app).
    #[test]
    fn a_whole_report_prints() {
        use opensheet_engine::chart::ChartKind;
        use opensheet_engine::condfmt::{BarColor, CondFormat, DataBar, IconRule, IconSet, Rule};
        use opensheet_engine::drawing::{Frame, ShapeTool};
        use opensheet_engine::page::PrintComments;
        use opensheet_engine::{Locale, Rect};
        use opensheet_print::{PrintJob, PrintWhat, plan, print_pdf, render_page};

        let mut wb = Workbook::new(Locale::fr());
        let products = [
            "Clavier", "Souris", "Écran", "Webcam", "Casque", "Station", "Câble", "Tapis",
        ];
        let _ = wb.set_cell_text(0, 0, 0, "Produit");
        let _ = wb.set_cell_text(0, 0, 1, "Ventes");
        let _ = wb.set_cell_text(0, 0, 2, "Marge");
        let _ = wb.set_cell_text(0, 0, 3, "Évolution");
        for row in 1..=70u32 {
            let name = format!("{} {}", products[(row as usize - 1) % products.len()], row);
            let _ = wb.set_cell_text(0, row, 0, &name);
            let _ = wb.set_cell_text(0, row, 1, &(120 + (row * 37) % 400).to_string());
            let _ = wb.set_cell_text(0, row, 2, &(((row * 13) % 60) as i64 - 20).to_string());
            let _ = wb.set_cell_text(0, row, 3, &((row * 7) % 10).to_string());
        }
        wb.update_style(0, Rect::new(0, 0, 0, 3), &|s| {
            s.font.bold = true;
            s.fill = Some(Color(0xDDEBF7));
        });
        wb.add_cond_format(
            0,
            CondFormat::new(
                vec![Rect::new(1, 1, 70, 1)],
                Rule::DataBar(DataBar::preset(BarColor::Blue, true)),
            ),
        );
        wb.add_cond_format(
            0,
            CondFormat::new(
                vec![Rect::new(1, 2, 70, 2)],
                Rule::DataBar(DataBar::preset(BarColor::Green, false)),
            ),
        );
        wb.add_cond_format(
            0,
            CondFormat::new(
                vec![Rect::new(1, 3, 70, 3)],
                Rule::IconSet(IconRule::preset(IconSet::ThreeArrows)),
            ),
        );
        let mut chart = wb.chart_from_range(0, Rect::new(0, 0, 8, 1), ChartKind::Column);
        chart.anchor = (2, 5);
        wb.add_chart(0, chart);
        let shape = wb
            .add_shape(0, ShapeTool::ALL[1], Frame::new(400.0, 420.0, 220.0, 90.0))
            .unwrap();
        wb.update_drawing(0, shape, |d| {
            if let Some(shape) = d.as_shape_mut() {
                shape.text = "À valider avant la clôture du trimestre".into();
            }
        });
        let arrow = ShapeTool::ALL
            .iter()
            .copied()
            .find(|tool| tool.preset().is_line())
            .unwrap();
        wb.add_shape(0, arrow, Frame::new(400.0, 540.0, 200.0, 60.0));
        // A picture: 40 × 20 pixels, half red, half blue.
        let mut pixels = Vec::new();
        for _ in 0..20 {
            for x in 0..40 {
                pixels.extend_from_slice(if x < 20 { &[220, 40, 40] } else { &[40, 80, 200] });
            }
        }
        let picture = wb
            .add_picture(0, crate::features::page::scene::png(40, 20, 3, &pixels), (30, 5))
            .unwrap();
        wb.update_drawing(0, picture, |d| d.size = (160.0, 80.0));
        wb.set_note(0, 3, 1, "Ana", "Ana:\nChiffre à confirmer.");
        wb.set_note_visible(0, 3, 1, true);
        wb.update_page_setup(0, |s| {
            s.header_footer.odd_header = "&L&\"Arial,Bold\"&14Rapport des ventes&R&D".into();
            s.header_footer.odd_footer = "&CPage &P de &N".into();
            s.comments = PrintComments::AsDisplayed;
            s.gridlines = true;
        });
        wb.commit("report");

        let fonts = FontDb::standard_only();
        let painter = PaperPainter::headless();
        let mut job = PrintJob::new(PrintWhat::Sheet(0));
        job.fonts = Some(&fonts);
        job.painter = Some(&painter);
        job.file_name = "Rapport.xlsx".into();
        let p = plan(&wb, &job);
        // Two pages down; the chart reaches onto a second page across.
        assert!(p.pages.len() >= 2);
        let first = render_page(&wb, &job, &p, 0).unwrap();
        let texts: Vec<&str> = first
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        // The chart (its title and an axis label), the shape's text, the
        // header and the note are all on the first page.
        assert!(texts.contains(&"Ventes"), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("À valider")));
        assert!(texts.contains(&"Rapport des ventes"));
        assert!(texts.contains(&"Chiffre à confirmer."));
        assert!(first.ops.iter().any(|op| matches!(op, Op::Image { .. })));
        // Bars and icons: many more filled shapes than the cells alone.
        let fills = first.ops.iter().filter(|op| matches!(op, Op::Fill { .. })).count();
        assert!(fills > 300, "{fills} fills");
        let pdf = print_pdf(&wb, &job);
        assert!(pdf.starts_with(b"%PDF-1.7"));
        if let Some(out) = std::env::var_os("OPENSHEET_PRINT_OUT") {
            let out = std::path::PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            std::fs::write(out.join("report.pdf"), &pdf).unwrap();
            // The same with the fonts of this machine, as the app prints.
            job.fonts = None;
            std::fs::write(out.join("report-system-fonts.pdf"), print_pdf(&wb, &job)).unwrap();
            opensheet_io::save(&wb, 0, &out.join("report.xlsx")).unwrap();
        }
    }
}
