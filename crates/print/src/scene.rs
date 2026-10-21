//! What a page draws, as a list of vector operations: the output of page
//! rendering, the input of the PDF writer, and what a print preview can
//! paint on screen. `Canvas` builds such a list; it is also what the
//! application draws charts and pictures with (see `ObjectPainter`).
//!
//! Coordinates are in points (1/72 inch) from the top-left corner of the
//! page, y growing downwards.

use crate::fonts::{Face, FontDb};
use crate::image::Image;
use opensheet_engine::style::Color;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> RectF {
        RectF { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn intersects(&self, o: &RectF) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    /// A cubic Bézier curve: two control points, then the end point.
    CurveTo(f64, f64, f64, f64, f64, f64),
    Close,
}

/// An outline made of lines and curves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn move_to(mut self, x: f64, y: f64) -> Path {
        self.segments.push(Segment::MoveTo(x, y));
        self
    }

    pub fn line_to(mut self, x: f64, y: f64) -> Path {
        self.segments.push(Segment::LineTo(x, y));
        self
    }

    pub fn curve_to(mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> Path {
        self.segments.push(Segment::CurveTo(x1, y1, x2, y2, x, y));
        self
    }

    pub fn close(mut self) -> Path {
        self.segments.push(Segment::Close);
        self
    }

    pub fn rect(r: RectF) -> Path {
        Path::new()
            .move_to(r.x, r.y)
            .line_to(r.right(), r.y)
            .line_to(r.right(), r.bottom())
            .line_to(r.x, r.bottom())
            .close()
    }

    pub fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
        Path::new().move_to(x0, y0).line_to(x1, y1)
    }

    /// A polyline through `points`.
    pub fn polyline(points: &[(f64, f64)]) -> Path {
        let mut path = Path::new();
        for (i, (x, y)) in points.iter().enumerate() {
            path = if i == 0 {
                path.move_to(*x, *y)
            } else {
                path.line_to(*x, *y)
            };
        }
        path
    }

    /// The ellipse inscribed in `r`.
    pub fn ellipse(r: RectF) -> Path {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        Path::new()
            .move_to(cx + r.w / 2.0, cy)
            .arc(cx, cy, r.w / 2.0, r.h / 2.0, 0.0, 360.0)
            .close()
    }

    /// Continue the path with an arc of the ellipse centred on (cx, cy):
    /// from `start` degrees over `sweep` degrees, measured clockwise on
    /// the page from the positive x axis. A line joins the current point
    /// to the start of the arc; begin with `move_to` there to avoid it.
    pub fn arc(mut self, cx: f64, cy: f64, rx: f64, ry: f64, start: f64, sweep: f64) -> Path {
        let pieces = (sweep.abs() / 90.0).ceil().max(1.0) as usize;
        let step = sweep.to_radians() / pieces as f64;
        // Control point distance for a cubic approximating an arc.
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut a = start.to_radians();
        let (x, y) = (cx + rx * a.cos(), cy + ry * a.sin());
        match self.segments.last() {
            None | Some(Segment::Close) => self.segments.push(Segment::MoveTo(x, y)),
            Some(
                Segment::MoveTo(px, py)
                | Segment::LineTo(px, py)
                | Segment::CurveTo(_, _, _, _, px, py),
            ) => {
                // Already there: no joining line.
                if (px - x).abs() > 1e-9 || (py - y).abs() > 1e-9 {
                    self.segments.push(Segment::LineTo(x, y));
                }
            }
        }
        for _ in 0..pieces {
            let b = a + step;
            self.segments.push(Segment::CurveTo(
                cx + rx * (a.cos() - k * a.sin()),
                cy + ry * (a.sin() + k * a.cos()),
                cx + rx * (b.cos() + k * b.sin()),
                cy + ry * (b.sin() - k * b.cos()),
                cx + rx * b.cos(),
                cy + ry * b.sin(),
            ));
            a = b;
        }
        self
    }
}

/// A colour and how opaque it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub color: Color,
    /// 0 (invisible) to 1 (opaque).
    pub opacity: f32,
}

impl Paint {
    pub const BLACK: Paint = Paint {
        color: Color::BLACK,
        opacity: 1.0,
    };

    pub fn translucent(color: Color, opacity: f32) -> Paint {
        Paint {
            color,
            opacity: opacity.clamp(0.0, 1.0),
        }
    }
}

impl From<Color> for Paint {
    fn from(color: Color) -> Paint {
        Paint {
            color,
            opacity: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    /// Extends half the line width beyond each end.
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// How an outline is stroked.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub paint: Paint,
    pub width: f64,
    /// Lengths of dashes and gaps, alternating; empty = solid.
    pub dash: Vec<f64>,
    pub cap: LineCap,
    pub join: LineJoin,
}

impl Stroke {
    pub fn solid(paint: impl Into<Paint>, width: f64) -> Stroke {
        Stroke {
            paint: paint.into(),
            width,
            dash: Vec::new(),
            cap: LineCap::Butt,
            join: LineJoin::Miter,
        }
    }

    pub fn dashed(mut self, dash: &[f64]) -> Stroke {
        self.dash = dash.to_vec();
        self
    }

    pub fn cap(mut self, cap: LineCap) -> Stroke {
        self.cap = cap;
        self
    }

    pub fn join(mut self, join: LineJoin) -> Stroke {
        self.join = join;
        self
    }
}

/// A font as text asks for it: a family name and a style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle<'a> {
    pub family: &'a str,
    pub bold: bool,
    pub italic: bool,
    /// Font size, in the canvas's units.
    pub size: f64,
}

/// One drawing operation.
#[derive(Clone)]
pub enum Op {
    /// Remember the clip and the transformation...
    Save,
    /// ...and bring them back.
    Restore,
    /// Draw only inside this rectangle (and the clip already in effect).
    Clip(RectF),
    /// Apply a transformation `[a b c d e f]` to what follows:
    /// x' = a·x + c·y + e, y' = b·x + d·y + f.
    Transform([f64; 6]),
    Fill {
        path: Path,
        paint: Paint,
    },
    Stroke {
        path: Path,
        stroke: Stroke,
    },
    /// One line of text in one face, its baseline starting at (x, y).
    Text {
        face: Arc<Face>,
        size: f64,
        x: f64,
        y: f64,
        text: String,
        paint: Paint,
    },
    /// An image stretched over a rectangle.
    Image {
        image: Arc<Image>,
        rect: RectF,
    },
}

/// Everything drawn on one page.
#[derive(Clone)]
pub struct Scene {
    /// Paper width and height in points.
    pub size: (f64, f64),
    pub ops: Vec<Op>,
}

/// Records drawing operations.
pub struct Canvas<'a> {
    fonts: &'a FontDb,
    ops: Vec<Op>,
    depth: usize,
    black_and_white: bool,
}

impl<'a> Canvas<'a> {
    pub fn new(fonts: &'a FontDb) -> Canvas<'a> {
        Canvas {
            fonts,
            ops: Vec::new(),
            depth: 0,
            black_and_white: false,
        }
    }

    pub fn into_ops(mut self) -> Vec<Op> {
        self.restore_to(0);
        self.ops
    }

    pub fn fonts(&self) -> &'a FontDb {
        self.fonts
    }

    /// The sheet prints in black and white: a painter should draw
    /// without colour.
    pub fn black_and_white(&self) -> bool {
        self.black_and_white
    }

    pub(crate) fn set_black_and_white(&mut self, on: bool) {
        self.black_and_white = on;
    }

    pub fn save(&mut self) {
        self.depth += 1;
        self.ops.push(Op::Save);
    }

    pub fn restore(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
            self.ops.push(Op::Restore);
        }
    }

    /// Number of `save` calls not yet matched by `restore`.
    pub(crate) fn depth(&self) -> usize {
        self.depth
    }

    /// Undo every `save` made beyond `depth` (a painter that returns
    /// without restoring must not disturb the rest of the page).
    pub(crate) fn restore_to(&mut self, depth: usize) {
        while self.depth > depth {
            self.restore();
        }
    }

    /// Restrict drawing to a rectangle until the matching `restore`.
    pub fn clip_rect(&mut self, rect: RectF) {
        self.ops.push(Op::Clip(rect));
    }

    /// Apply a transformation until the matching `restore`.
    pub fn transform(&mut self, matrix: [f64; 6]) {
        self.ops.push(Op::Transform(matrix));
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.transform([1.0, 0.0, 0.0, 1.0, dx, dy]);
    }

    pub fn scale(&mut self, sx: f64, sy: f64) {
        self.transform([sx, 0.0, 0.0, sy, 0.0, 0.0]);
    }

    /// Rotate around the origin; positive angles turn counter-clockwise
    /// on the page.
    pub fn rotate(&mut self, degrees: f64) {
        let (sin, cos) = degrees.to_radians().sin_cos();
        self.transform([cos, -sin, sin, cos, 0.0, 0.0]);
    }

    pub fn fill_path(&mut self, path: Path, paint: impl Into<Paint>) {
        self.ops.push(Op::Fill {
            path,
            paint: paint.into(),
        });
    }

    pub fn stroke_path(&mut self, path: Path, stroke: Stroke) {
        if stroke.width > 0.0 {
            self.ops.push(Op::Stroke { path, stroke });
        }
    }

    pub fn fill_rect(&mut self, rect: RectF, paint: impl Into<Paint>) {
        if rect.w > 0.0 && rect.h > 0.0 {
            self.fill_path(Path::rect(rect), paint);
        }
    }

    pub fn stroke_rect(&mut self, rect: RectF, stroke: Stroke) {
        self.stroke_path(Path::rect(rect), stroke);
    }

    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, stroke: Stroke) {
        self.stroke_path(Path::line(x0, y0, x1, y1), stroke);
    }

    /// The face a text style prints with.
    pub fn face(&self, style: &TextStyle) -> Arc<Face> {
        self.fonts.face(style.family, style.bold, style.italic)
    }

    /// Width of one line of text, exactly as `text` draws it.
    pub fn text_width(&self, text: &str, style: &TextStyle) -> f64 {
        self.fonts.text_width(&self.face(style), text, style.size)
    }

    /// Draw one line of text with its baseline starting at (x, y);
    /// returns its width. Characters the font lacks are drawn with
    /// another installed font that has them.
    pub fn text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        style: &TextStyle,
        paint: impl Into<Paint>,
    ) -> f64 {
        let face = self.face(style);
        self.face_text(&face, text, x, y, style.size, paint.into())
    }

    /// Like `text`, with a face already chosen.
    pub fn face_text(
        &mut self,
        face: &Arc<Face>,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        paint: Paint,
    ) -> f64 {
        let mut advance = 0.0;
        for (run_face, run) in self.fonts.runs(face, text) {
            let width = run_face.text_width(&run, size);
            self.ops.push(Op::Text {
                face: run_face,
                size,
                x: x + advance,
                y,
                text: run,
                paint,
            });
            advance += width;
        }
        advance
    }

    /// Draw an image stretched over a rectangle.
    pub fn image(&mut self, image: &Arc<Image>, rect: RectF) {
        if rect.w > 0.0 && rect.h > 0.0 {
            self.ops.push(Op::Image {
                image: image.clone(),
                rect,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let r = Path::rect(RectF::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(r.segments.len(), 5);
        assert_eq!(r.segments[2], Segment::LineTo(4.0, 6.0));
        // A full ellipse is four curves and ends where it began.
        let e = Path::ellipse(RectF::new(0.0, 0.0, 20.0, 10.0));
        assert_eq!(e.segments.len(), 6);
        assert_eq!(e.segments[0], Segment::MoveTo(20.0, 5.0));
        match e.segments[4] {
            Segment::CurveTo(_, _, _, _, x, y) => {
                assert!((x - 20.0).abs() < 1e-9 && (y - 5.0).abs() < 1e-9)
            }
            _ => panic!("not a curve"),
        }
        // A quarter arc reaches the bottom of the circle (y grows down).
        let a = Path::new().arc(0.0, 0.0, 10.0, 10.0, 0.0, 90.0);
        match a.segments[1] {
            Segment::CurveTo(_, _, _, _, x, y) => {
                assert!(x.abs() < 1e-9 && (y - 10.0).abs() < 1e-9)
            }
            _ => panic!("not a curve"),
        }
        assert_eq!(
            Path::polyline(&[(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)])
                .segments
                .len(),
            3
        );
    }

    #[test]
    fn canvas_records_and_balances() {
        let fonts = FontDb::standard_only();
        let mut canvas = Canvas::new(&fonts);
        canvas.save();
        canvas.clip_rect(RectF::new(0.0, 0.0, 10.0, 10.0));
        canvas.save();
        canvas.rotate(90.0);
        let style = TextStyle {
            family: "Arial",
            bold: false,
            italic: false,
            size: 10.0,
        };
        let width = canvas.text("Hello", 5.0, 5.0, &style, Color::BLACK);
        assert!((width - 22.78).abs() < 1e-9);
        assert!((canvas.text_width("Hello", &style) - width).abs() < 1e-12);
        // Nothing is recorded for what would not show.
        canvas.fill_rect(RectF::new(0.0, 0.0, 0.0, 5.0), Color::BLACK);
        canvas.line(0.0, 0.0, 1.0, 1.0, Stroke::solid(Color::BLACK, 0.0));
        assert_eq!(canvas.depth(), 2);
        let ops = canvas.into_ops();
        // Unbalanced saves are closed.
        let saves = ops.iter().filter(|o| matches!(o, Op::Save)).count();
        let restores = ops.iter().filter(|o| matches!(o, Op::Restore)).count();
        assert_eq!((saves, restores), (2, 2));
        assert_eq!(
            ops.iter().filter(|o| matches!(o, Op::Text { .. })).count(),
            1
        );
        assert_eq!(ops.len(), 7);
    }
}
