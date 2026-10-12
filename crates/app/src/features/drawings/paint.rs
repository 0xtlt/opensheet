//! Painting pictures and shapes: where a drawing is in each pane of the
//! grid, and the paths, bitmaps and lines of text it is made of. The same
//! code draws the shapes of the gallery.

use super::geometry::{self, Placed, Polyline, Pt};
use super::picture;
use super::text::{self, TextLook};
use crate::grid::{Draw, FontResolver, GridLayout, Layer, pt_to_px};
use crate::theme::{c, engine_color};
use gpui::{
    Bounds, Font, FontStyle, FontWeight, Hsla, IntoElement, PathBuilder, Pixels, Point, WindowTextSystem, canvas, point,
    prelude::*, px, size,
};
use opensheet_engine::Sheet;
use opensheet_engine::drawing::{Drawing, DrawingKind, Frame, LineEnd, Outline, Shape, ShapeTool, TEXT_MARGIN};
use opensheet_engine::style::Underline;

/// Colour of the selection frame and of the handles (as `objects.rs`).
const HANDLE: u32 = 0x8A8A8A;

/// How far above its frame the rotation handle of a shape is.
pub const ROTATION_GAP: f32 = 22.0;

/// One of the parts of the grid that scroll on their own: four when rows
/// and columns are frozen, one when nothing is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    /// What it shows of the grid element (x0, y0, x1, y1).
    pub clip: (f32, f32, f32, f32),
    /// What is added to a position on the zoomed sheet to get its position
    /// in the grid element.
    pub dx: f64,
    pub dy: f64,
}

impl Pane {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.clip.0 && x < self.clip.2 && y >= self.clip.1 && y < self.clip.3
    }

    /// True when a box (x0, y0, x1, y1) shows in the pane.
    pub fn shows(&self, extent: (f32, f32, f32, f32)) -> bool {
        extent.2 >= self.clip.0 && extent.0 <= self.clip.2 && extent.3 >= self.clip.1 && extent.1 <= self.clip.3
    }
}

/// The panes of the grid, frozen ones first.
pub fn panes(layout: &GridLayout) -> Vec<Pane> {
    let (hw, hh) = (layout.header_w, layout.header_h);
    // The frozen part (if any) and the scrolling part along one axis:
    // where each starts and ends, and its offset.
    let along = |header: f32, frozen: f32, end: f32, scroll: f64| {
        let mut parts = Vec::new();
        if frozen > 0.0 {
            parts.push((header, header + frozen, header as f64));
        }
        parts.push((header + frozen, end.max(header + frozen), header as f64 - scroll));
        parts
    };
    let columns = along(hw, layout.frozen_w, layout.width(), layout.scroll_x);
    let rows = along(hh, layout.frozen_h, layout.height(), layout.scroll_y);
    let mut out = Vec::new();
    for (y0, y1, dy) in &rows {
        for (x0, x1, dx) in &columns {
            out.push(Pane {
                clip: (*x0, *y0, *x1, *y1),
                dx: *dx,
                dy: *dy,
            });
        }
    }
    out
}

/// The pane under a point of the grid element (none over the headers).
pub fn pane_at(layout: &GridLayout, x: f32, y: f32) -> Option<Pane> {
    panes(layout).into_iter().find(|pane| pane.contains(x, y))
}

/// A point of the sheet (pixels at 100 %) on the zoomed sheet the layout
/// works with. It goes through the cell holding the point, so that a frame
/// that ends on a cell edge is drawn ending on it at any zoom.
pub fn zoomed(layout: &GridLayout, sheet: &Sheet, x: f64, y: f64) -> (f64, f64) {
    let ((row, col), (dx, dy)) = sheet.corner_at(x, y);
    let z = layout.zoom as f64;
    (
        layout.cols.start(col) + (dx as f64 * z).min(layout.cols.size(col) as f64),
        layout.rows.start(row) + (dy as f64 * z).min(layout.rows.size(row) as f64),
    )
}

/// The point of the sheet shown at a position of the grid element in a
/// pane: the reverse of `zoomed`.
pub fn sheet_point(layout: &GridLayout, sheet: &Sheet, pane: &Pane, x: f32, y: f32) -> (f64, f64) {
    let z = layout.zoom as f64;
    let (zx, zy) = ((x as f64 - pane.dx).max(0.0), (y as f64 - pane.dy).max(0.0));
    let (col, row) = (layout.cols.index_at(zx), layout.rows.index_at(zy));
    let inside = (
        ((zx - layout.cols.start(col)) / z).max(0.0) as f32,
        ((zy - layout.rows.start(row)) / z).max(0.0) as f32,
    );
    sheet.corner_point(((row, col), inside))
}

/// Where a frame of the sheet is drawn in a pane.
pub fn placed_frame(layout: &GridLayout, sheet: &Sheet, frame: Frame, turn: (f32, bool, bool), pane: &Pane) -> Placed {
    let (x0, y0) = zoomed(layout, sheet, frame.x, frame.y);
    let (x1, y1) = zoomed(layout, sheet, frame.x + frame.width, frame.y + frame.height);
    Placed {
        x0: (x0 + pane.dx) as f32,
        y0: (y0 + pane.dy) as f32,
        x1: (x1 + pane.dx) as f32,
        y1: (y1 + pane.dy) as f32,
        rotation: turn.0,
        flip_h: turn.1,
        flip_v: turn.2,
    }
}

/// Where a drawing is drawn in a pane.
pub fn placed(layout: &GridLayout, sheet: &Sheet, drawing: &Drawing, pane: &Pane) -> Placed {
    placed_frame(
        layout,
        sheet,
        sheet.drawing_frame(drawing),
        (drawing.rotation, drawing.flip_h, drawing.flip_v),
        pane,
    )
}

/// The box a drawing may paint in: its turned frame, with room for its
/// outline, the tail of a callout, arrowheads and the handles.
pub fn extent(placed: &Placed, zoom: f32) -> (f32, f32, f32, f32) {
    let (x0, y0, x1, y1) = placed.extent();
    let room = ROTATION_GAP + 10.0 + (placed.height().abs() * 0.13).max(12.0 * zoom);
    (x0 - room, y0 - room, x1 + room, y1 + room)
}

/// What painting needs besides the drawing.
pub struct Brush<'a> {
    /// Window position of the point (0, 0) the frames are measured from.
    pub origin: Point<Pixels>,
    pub zoom: f32,
    /// Device pixels per pixel.
    pub scale: f32,
    pub ts: &'a WindowTextSystem,
    /// Text is cut off there.
    pub clip: Bounds<Pixels>,
}

impl Brush<'_> {
    fn at(&self, p: Pt) -> Point<Pixels> {
        point(self.origin.x + px(p.0), self.origin.y + px(p.1))
    }

    fn fill(&self, points: &[Pt], color: Hsla, out: &mut Vec<Draw>) {
        if points.len() < 3 {
            return;
        }
        let points: Vec<Point<Pixels>> = points.iter().map(|p| self.at(*p)).collect();
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&points, true);
        if let Ok(path) = builder.build() {
            out.push(Draw::Path(path, color));
        }
    }

    /// `dashes`: lengths of dashes and gaps in pixels; empty for a solid
    /// line.
    fn stroke(&self, line: &Polyline, width: f32, dashes: &[f32], color: Hsla, out: &mut Vec<Draw>) {
        if line.points.len() < 2 {
            return;
        }
        let mut builder = PathBuilder::stroke(px(width));
        if !dashes.is_empty() {
            let dashes: Vec<Pixels> = dashes.iter().map(|d| px(*d)).collect();
            builder = builder.dash_array(&dashes);
        }
        builder.move_to(self.at(line.points[0]));
        for p in &line.points[1..] {
            builder.line_to(self.at(*p));
        }
        if line.closed {
            builder.close();
        }
        if let Ok(path) = builder.build() {
            out.push(Draw::Path(path, color));
        }
    }

    fn disc(&self, centre: Pt, radius: f32, color: Hsla, out: &mut Vec<Draw>) {
        let points: Vec<Pt> = (0..20)
            .map(|i| {
                let angle = i as f32 / 20.0 * std::f32::consts::TAU;
                (centre.0 + radius * angle.cos(), centre.1 + radius * angle.sin())
            })
            .collect();
        self.fill(&points, color, out);
    }
}

/// The width of an outline on screen.
pub fn stroke_width(outline: &Outline, zoom: f32) -> f32 {
    (pt_to_px(outline.width) * zoom).max(0.75)
}

/// The font and the colour of the text of a shape.
pub fn text_look(shape: &Shape, zoom: f32, fonts: &FontResolver) -> TextLook {
    TextLook {
        font: Font {
            family: fonts.resolve(&shape.font.name),
            features: Default::default(),
            fallbacks: None,
            weight: if shape.font.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
            style: if shape.font.italic { FontStyle::Italic } else { FontStyle::Normal },
        },
        size: (pt_to_px(shape.font.size_pt()) * zoom).max(1.0),
        color: engine_color(shape.font.color.unwrap_or(opensheet_engine::Color::BLACK)),
        underline: shape.font.underline != Underline::None,
        strike: shape.font.strike,
        align: shape.align,
        anchor: shape.anchor,
    }
}

/// The upright box (x0, y0, x1, y1) the text of a shape is laid out in:
/// the text rectangle of its preset less the margins. GPUI draws text
/// level only, so the text of a turned shape stays level around the place
/// its rectangle is turned to; when the shape lies on its side the box
/// does too, so that the text still wraps inside the shape.
pub fn text_box(shape: &Shape, placed: &Placed, natural: (f32, f32), zoom: f32) -> (f32, f32, f32, f32) {
    let [left, top, right, bottom] = shape.preset.text_rect(natural.0, natural.1);
    let mut width = ((right - left) * placed.width().abs() - 2.0 * TEXT_MARGIN.0 * zoom).max(1.0);
    let mut height = ((bottom - top) * placed.height().abs() - 2.0 * TEXT_MARGIN.1 * zoom).max(1.0);
    if (geometry::normalise_angle(placed.rotation + 45.0) % 180.0) >= 90.0 {
        std::mem::swap(&mut width, &mut height);
    }
    let (cx, cy) = placed.point((left + right) / 2.0, (top + bottom) / 2.0);
    (cx - width / 2.0, cy - height / 2.0, cx + width / 2.0, cy + height / 2.0)
}

/// The outline of a shape on screen.
pub fn outline_of(shape: &Shape, placed: &Placed, natural: (f32, f32)) -> Vec<Polyline> {
    geometry::flatten(&shape.preset.path(natural.0, natural.1), placed)
}

/// The length (and the width) of the arrowheads of an outline.
fn head_size(width: f32, zoom: f32) -> f32 {
    3.0 * width.max(pt_to_px(2.0) * zoom)
}

fn draw_shape(shape: &Shape, placed: &Placed, natural: (f32, f32), brush: &Brush, text: Option<&FontResolver>, out: &mut Vec<Draw>) {
    let lines = outline_of(shape, placed, natural);
    if let (Some(fill), false) = (shape.fill, shape.preset.is_line()) {
        for line in lines.iter().filter(|line| line.closed) {
            brush.fill(&line.points, engine_color(fill), out);
        }
    }
    if let Some(outline) = &shape.outline {
        let width = stroke_width(outline, brush.zoom);
        let color = engine_color(outline.color);
        let unit = width.max(1.0);
        let dashes: Vec<f32> = outline.dash.pattern().iter().map(|d| d * unit).collect();
        for mut line in lines {
            if !line.closed {
                // The ends of a line: heads pointing away from it, and the
                // line stopping under them.
                let size = head_size(width, brush.zoom);
                let count = line.points.len();
                let ends = [
                    (outline.start, line.points[0], line.points[1], false),
                    (outline.end, line.points[count - 1], line.points[count - 2], true),
                ];
                for (kind, tip, from, at_end) in ends {
                    let Some(head) = geometry::arrow_head(kind, tip, from, size) else {
                        continue;
                    };
                    geometry::trim(&mut line.points, head.cut, at_end);
                    if head.filled {
                        brush.fill(&head.points, color, out);
                    } else {
                        let strokes = Polyline {
                            points: head.points,
                            closed: false,
                        };
                        brush.stroke(&strokes, width, &[], color, out);
                    }
                }
            }
            brush.stroke(&line, width, &dashes, color, out);
        }
    }
    if let Some(fonts) = text.filter(|_| !shape.text.is_empty() && !shape.preset.is_line()) {
        let look = text_look(shape, brush.zoom, fonts);
        let (x0, y0, x1, y1) = text_box(shape, placed, natural, brush.zoom);
        let laid = text::lay_out(&shape.text, &look, x1 - x0, y1 - y0, None, brush.ts);
        let line_height = px(laid.line_height);
        for line in laid.lines {
            out.push(Draw::Text {
                origin: brush.at(((x0 + line.x).round(), y0 + line.y)),
                line: line.shaped,
                line_height,
                clip: brush.clip,
            });
        }
    }
}

/// The items that draw a picture or a shape in a frame. `natural` is the
/// size of the frame at 100 % (the proportions of the outline depend on
/// it). The text of a shape is drawn when `text` gives the fonts: not
/// while it is being typed in place.
pub fn draw(drawing: &Drawing, placed: &Placed, natural: (f32, f32), brush: &Brush, text: Option<&FontResolver>, out: &mut Vec<Draw>) {
    match &drawing.kind {
        DrawingKind::Shape(shape) => draw_shape(shape, placed, natural, brush, text, out),
        DrawingKind::Picture(picture) => match picture::image_for(picture, placed, brush.scale) {
            Some((image, (x0, y0, x1, y1))) => out.push(Draw::Image {
                image,
                bounds: Bounds::new(brush.at((x0, y0)), size(px(x1 - x0), px(y1 - y0))),
            }),
            None => {
                // A picture that cannot be decoded: a crossed-out box.
                let corners = placed.corners();
                brush.fill(&corners, c(0xF3F3F3), out);
                let frame = Polyline {
                    points: corners.to_vec(),
                    closed: true,
                };
                brush.stroke(&frame, 1.0, &[], c(0xB0B0B0), out);
                for (a, b) in [(0, 2), (1, 3)] {
                    let cross = Polyline {
                        points: vec![corners[a], corners[b]],
                        closed: false,
                    };
                    brush.stroke(&cross, 1.0, &[], c(0xC42B1C), out);
                }
            }
        },
    }
}

/// What a handle of the selected drawing does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// One of the eight resize handles, clockwise from the top-left one.
    Resize(u8),
    /// An end of a line: its start (false) or its end.
    LineEnd(bool),
    Rotate,
}

/// The handles of a selected drawing and where they are: the two ends of
/// a line, or the eight handles of a frame and the one that turns it.
pub fn grips(drawing: &Drawing, placed: &Placed) -> Vec<(Grip, Pt)> {
    if drawing.as_shape().is_some_and(|shape| shape.preset.is_line()) {
        return vec![
            (Grip::LineEnd(false), placed.point(0.0, 0.0)),
            (Grip::LineEnd(true), placed.point(1.0, 1.0)),
        ];
    }
    let mut out: Vec<(Grip, Pt)> = placed
        .handles()
        .into_iter()
        .enumerate()
        .map(|(i, p)| (Grip::Resize(i as u8), p))
        .collect();
    out.push((Grip::Rotate, placed.rotation_handle(ROTATION_GAP)));
    out
}

/// The frame and the handles of the selected drawing.
pub fn draw_selection(drawing: &Drawing, placed: &Placed, brush: &Brush, out: &mut Vec<Draw>) {
    let grips = grips(drawing, placed);
    if grips.len() > 2 {
        let frame = Polyline {
            points: placed.corners().to_vec(),
            closed: true,
        };
        brush.stroke(&frame, 1.0, &[], c(HANDLE), out);
    }
    for (grip, at) in grips {
        if grip == Grip::Rotate {
            // A ring with an opening and a head: an arrow turning round.
            brush.disc(at, 6.5, c(0xFFFFFF), out);
            let arc = Polyline {
                points: (0..=14)
                    .map(|i| {
                        let angle = (-60.0 + i as f32 * 20.0).to_radians();
                        (at.0 + 4.0 * angle.cos(), at.1 + 4.0 * angle.sin())
                    })
                    .collect(),
                closed: false,
            };
            let (tip, before) = (arc.points[0], arc.points[1]);
            if let Some(head) = geometry::arrow_head(LineEnd::Triangle, tip, before, 4.5) {
                brush.fill(&head.points, c(0x5F5F5F), out);
            }
            brush.stroke(&arc, 1.3, &[], c(0x5F5F5F), out);
        } else {
            brush.disc(at, 4.5, c(HANDLE), out);
            brush.disc(at, 3.5, c(0xFFFFFF), out);
        }
    }
}

/// A shape of the gallery drawn small, as Excel's gallery shows it: its
/// outline, dark on white.
pub fn thumbnail(tool: ShapeTool, side: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let ts = window.text_system().clone();
            let brush = Brush {
                origin: bounds.origin,
                zoom: 1.0,
                scale: window.scale_factor(),
                ts: &ts,
                clip: bounds,
            };
            let mut items = Vec::new();
            draw_thumbnail(tool, side, &brush, &mut items);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .size(px(side))
    .flex_none()
}

/// A level line drawn with an outline: what the Weight, Dashes and Arrows
/// menus show for each of their choices.
pub fn line_sample(width: f32, height: f32, outline: Outline) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let ts = window.text_system().clone();
            let brush = Brush {
                origin: bounds.origin,
                zoom: 1.0,
                scale: window.scale_factor(),
                ts: &ts,
                clip: bounds,
            };
            let mut shape = ShapeTool::Line.shape();
            shape.outline = Some(outline);
            let middle = (height / 2.0).floor() + 0.5;
            let placed = Placed::plain(5.0, middle, width - 5.0, middle);
            let mut items = Vec::new();
            draw_shape(&shape, &placed, (width - 10.0, 0.0), &brush, None, &mut items);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

fn draw_thumbnail(tool: ShapeTool, side: f32, brush: &Brush, out: &mut Vec<Draw>) {
    let ink = opensheet_engine::Color(0x404040);
    let mut shape = tool.shape();
    shape.fill = None;
    shape.text.clear();
    if let Some(outline) = &mut shape.outline {
        outline.color = ink;
        outline.width = 0.9;
    }
    // The proportions of the shape as a click inserts it, fitted in the
    // square; a line runs along its diagonal.
    let (width, height) = tool.default_size();
    let pad = (side * 0.14).round();
    let room = side - 2.0 * pad;
    let (w, h) = if shape.preset.is_line() {
        (room, room)
    } else if width >= height {
        (room, (room * (height / width).max(0.62)).round())
    } else {
        ((room * (width / height).max(0.62)).round(), room)
    };
    let (x0, y0) = (((side - w) / 2.0).round() + 0.5, ((side - h) / 2.0).round() + 0.5);
    let placed = Placed::plain(x0, y0, x0 + w - 1.0, y0 + h - 1.0);
    // A callout keeps room for its tail, which hangs below its frame.
    let placed = if tool == ShapeTool::Callout {
        Placed {
            y1: placed.y1 - h * 0.12,
            ..placed
        }
    } else {
        placed
    };
    draw_shape(&shape, &placed, (width.max(1.0), height.max(1.0)), brush, None, out);
    if tool == ShapeTool::TextBox {
        // What tells a text box from a rectangle: lines of text in it.
        for (i, length) in [0.7, 0.5].into_iter().enumerate() {
            let y = placed.y0 + placed.height() * (0.36 + 0.3 * i as f32);
            let line = Polyline {
                points: vec![(placed.x0 + 3.0, y.round() + 0.5), (placed.x0 + 3.0 + (placed.width() - 6.0) * length, y.round() + 0.5)],
                closed: false,
            };
            brush.stroke(&line, 1.0, &[], engine_color(ink), out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::Drawing;

    #[test]
    fn a_line_has_two_grips_and_a_frame_nine() {
        let placed = Placed::plain(10.0, 10.0, 110.0, 60.0);
        let rectangle = Drawing::shape(ShapeTool::Rectangle.shape(), (100.0, 50.0));
        let found = grips(&rectangle, &placed);
        assert_eq!(found.len(), 9);
        assert_eq!(found[0], (Grip::Resize(0), (10.0, 10.0)));
        assert_eq!(found[4], (Grip::Resize(4), (110.0, 60.0)));
        assert_eq!(found[8], (Grip::Rotate, (60.0, 10.0 - ROTATION_GAP)));
        // The ends of a line follow its flips.
        let line = Drawing::shape(ShapeTool::Arrow.shape(), (100.0, 50.0));
        assert_eq!(
            grips(&line, &placed),
            vec![(Grip::LineEnd(false), (10.0, 10.0)), (Grip::LineEnd(true), (110.0, 60.0))]
        );
        let flipped = Placed { flip_v: true, ..placed };
        assert_eq!(grips(&line, &flipped)[0], (Grip::LineEnd(false), (10.0, 60.0)));
    }

    #[test]
    fn text_keeps_clear_of_the_outline() {
        let placed = Placed::plain(0.0, 0.0, 200.0, 100.0);
        let rectangle = ShapeTool::Rectangle.shape();
        let (x0, y0, x1, y1) = text_box(&rectangle, &placed, (200.0, 100.0), 1.0);
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(x0, TEXT_MARGIN.0) && near(y0, TEXT_MARGIN.1));
        assert!(near(x1, 200.0 - TEXT_MARGIN.0) && near(y1, 100.0 - TEXT_MARGIN.1));
        // In an ellipse the text rectangle is the square inside it.
        let ellipse = ShapeTool::Ellipse.shape();
        let (x0, y0, x1, y1) = text_box(&ellipse, &placed, (200.0, 100.0), 1.0);
        assert!(x0 > 29.0 + TEXT_MARGIN.0 && x1 < 171.0 - TEXT_MARGIN.0 && y0 > 14.0 && y1 < 86.0);
        // Turned a little, the box keeps its size and its centre; turned
        // a quarter it lies on its side like the shape.
        let tilted = Placed { rotation: 30.0, ..placed };
        let (tx0, ty0, tx1, ty1) = text_box(&rectangle, &tilted, (200.0, 100.0), 1.0);
        assert!((tx1 - tx0 - (200.0 - 2.0 * TEXT_MARGIN.0)).abs() < 0.01);
        assert!(((tx0 + tx1) / 2.0 - 100.0).abs() < 0.01 && ((ty0 + ty1) / 2.0 - 50.0).abs() < 0.01);
        for quarter in [90.0, 270.0, 100.0] {
            let turned = Placed { rotation: quarter, ..placed };
            let (tx0, ty0, tx1, ty1) = text_box(&rectangle, &turned, (200.0, 100.0), 1.0);
            assert!((tx1 - tx0 - (100.0 - 2.0 * TEXT_MARGIN.1)).abs() < 0.01, "{quarter}");
            assert!((ty1 - ty0 - (200.0 - 2.0 * TEXT_MARGIN.0)).abs() < 0.01, "{quarter}");
        }
        let upside_down = Placed { rotation: 180.0, ..placed };
        let (tx0, _, tx1, _) = text_box(&rectangle, &upside_down, (200.0, 100.0), 1.0);
        assert!((tx1 - tx0 - (200.0 - 2.0 * TEXT_MARGIN.0)).abs() < 0.01);
        // Margins grow with the zoom, and a box never has no room at all.
        let small = text_box(&rectangle, &Placed::plain(0.0, 0.0, 10.0, 4.0), (10.0, 4.0), 2.0);
        assert!(small.2 - small.0 >= 1.0 && small.3 - small.1 >= 1.0);
        assert_eq!(stroke_width(&Outline::solid(opensheet_engine::Color::BLACK, 3.0), 1.0), 4.0);
        assert_eq!(stroke_width(&Outline::solid(opensheet_engine::Color::BLACK, 0.25), 1.0), 0.75);
    }
}
