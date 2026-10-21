//! Painting a printed page in a window: the print preview shows the very
//! operations the PDF is written from (`opensheet_print::Scene`), scaled
//! to the screen.

use crate::theme::ca;
use gpui::{
    App, Bounds, ContentMask, Corners, Font, FontStyle, FontWeight, Hsla, ImageFormat, PathBuilder,
    Pixels, Point, RenderImage, SharedString, TextRun, Window, fill, point, px, size,
};
use opensheet_print::{Image, Op, Paint, Path, Scene, Segment};
use std::sync::Arc;

/// A page ready to be painted: its images are decoded once, not at every
/// repaint.
pub(crate) struct Prepared {
    pub scene: Scene,
    /// The images of the page, by the address of the `Image` they show.
    images: Vec<(usize, Arc<RenderImage>)>,
}

/// Decode the images of a page for the screen.
pub(crate) fn prepare(scene: Scene, cx: &App) -> Prepared {
    let mut images: Vec<(usize, Arc<RenderImage>)> = Vec::new();
    for op in &scene.ops {
        let Op::Image { image, .. } = op else {
            continue;
        };
        let key = Arc::as_ptr(image) as usize;
        if images.iter().any(|(k, _)| *k == key) {
            continue;
        }
        if let Ok(decoded) = screen_image(image).to_image_data(cx.svg_renderer()) {
            images.push((key, decoded));
        }
    }
    Prepared { scene, images }
}

/// An image of a page as a file GPUI can decode.
fn screen_image(image: &Image) -> gpui::Image {
    match &image.pixels {
        opensheet_print::Pixels::Jpeg(bytes) => {
            gpui::Image::from_bytes(ImageFormat::Jpeg, bytes.clone())
        }
        opensheet_print::Pixels::Rgb(data) => {
            gpui::Image::from_bytes(ImageFormat::Png, png(image.width, image.height, 3, data))
        }
        opensheet_print::Pixels::Rgba(data) => {
            gpui::Image::from_bytes(ImageFormat::Png, png(image.width, image.height, 4, data))
        }
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG file of raw pixels (`channels` bytes each, rows from the top),
/// written without compression: it only travels to the decoder next door.
pub(crate) fn png(width: u32, height: u32, channels: usize, data: &[u8]) -> Vec<u8> {
    let stride = width as usize * channels;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in data.chunks(stride.max(1)).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        zlib.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        zlib.push(blocks.peek().is_none() as u8);
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &raw {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&(b << 16 | a).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut tagged = kind.to_vec();
        tagged.extend_from_slice(body);
        out.extend_from_slice(&tagged);
        out.extend_from_slice(&crc32(&tagged).to_be_bytes());
    };
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    out
}

/// The transformation and the clip in effect while painting.
#[derive(Clone, Copy)]
struct State {
    /// `[a b c d e f]`: x' = a·x + c·y + e, y' = b·x + d·y + f, in points.
    matrix: [f64; 6],
    clip: Bounds<Pixels>,
}

/// `outer` applied after `inner`.
pub(crate) fn compose(outer: [f64; 6], inner: [f64; 6]) -> [f64; 6] {
    let [a, b, c, d, e, f] = outer;
    let [ia, ib, ic, id, ie, jf] = inner;
    [
        a * ia + c * ib,
        b * ia + d * ib,
        a * ic + c * id,
        b * ic + d * id,
        a * ie + c * jf + e,
        b * ie + d * jf + f,
    ]
}

fn color(paint: &Paint) -> Hsla {
    ca(paint.color.value(), paint.opacity)
}

struct Brush<'a> {
    window: &'a mut Window,
    origin: Point<Pixels>,
    /// Window pixels per point of paper.
    scale: f64,
    state: State,
    stack: Vec<State>,
}

impl Brush<'_> {
    /// A point of the page in the window.
    fn at(&self, x: f64, y: f64) -> Point<Pixels> {
        let [a, b, c, d, e, f] = self.state.matrix;
        point(
            self.origin.x + px(((a * x + c * y + e) * self.scale) as f32),
            self.origin.y + px(((b * x + d * y + f) * self.scale) as f32),
        )
    }

    /// True while nothing is turned or sheared: rectangles stay upright.
    fn upright(&self) -> bool {
        self.state.matrix[1].abs() < 1e-9 && self.state.matrix[2].abs() < 1e-9
    }

    /// Window pixels per unit of what is being drawn.
    fn stretch(&self) -> f64 {
        let [a, b, ..] = self.state.matrix;
        a.hypot(b) * self.scale
    }

    fn masked(&mut self, f: impl FnOnce(&mut Window)) {
        let bounds = self.state.clip;
        self.window
            .with_content_mask(Some(ContentMask { bounds }), f);
    }

    fn trace(&self, builder: &mut PathBuilder, path: &Path) {
        for segment in &path.segments {
            match *segment {
                Segment::MoveTo(x, y) => builder.move_to(self.at(x, y)),
                Segment::LineTo(x, y) => builder.line_to(self.at(x, y)),
                Segment::CurveTo(ax, ay, bx, by, x, y) => {
                    builder.cubic_bezier_to(self.at(x, y), self.at(ax, ay), self.at(bx, by))
                }
                Segment::Close => builder.close(),
            }
        }
    }

    /// The corners of a path that is an upright rectangle.
    fn as_rect(&self, path: &Path) -> Option<Bounds<Pixels>> {
        if !self.upright() {
            return None;
        }
        let corners: Vec<(f64, f64)> = match path.segments.as_slice() {
            [
                Segment::MoveTo(x0, y0),
                Segment::LineTo(x1, y1),
                Segment::LineTo(x2, y2),
                Segment::LineTo(x3, y3),
                Segment::Close,
            ] => {
                vec![(*x0, *y0), (*x1, *y1), (*x2, *y2), (*x3, *y3)]
            }
            _ => return None,
        };
        let square = (corners[0].1 == corners[1].1
            && corners[1].0 == corners[2].0
            && corners[2].1 == corners[3].1
            && corners[3].0 == corners[0].0)
            || (corners[0].0 == corners[1].0
                && corners[1].1 == corners[2].1
                && corners[2].0 == corners[3].0
                && corners[3].1 == corners[0].1);
        square.then(|| {
            Bounds::from_corners(
                self.at(corners[0].0, corners[0].1),
                self.at(corners[2].0, corners[2].1),
            )
        })
    }

    /// A straight line along the pixel grid, as a rectangle: thin lines
    /// stay sharp, and lines thinner than a pixel are drawn paler rather
    /// than not at all.
    fn hairline(&mut self, from: Point<Pixels>, to: Point<Pixels>, width: f32, ink: Hsla) {
        let device = 1.0 / self.window.scale_factor();
        let thick = width.max(device);
        let mut ink = ink;
        if width < device {
            ink.a *= (width / device).clamp(0.35, 1.0);
        }
        let bounds = if from.y == to.y {
            Bounds::new(
                point(from.x.min(to.x), from.y - px(thick / 2.0)),
                size((to.x - from.x).abs(), px(thick)),
            )
        } else {
            Bounds::new(
                point(from.x - px(thick / 2.0), from.y.min(to.y)),
                size(px(thick), (to.y - from.y).abs()),
            )
        };
        self.masked(|window| window.paint_quad(fill(bounds, ink)));
    }

    fn fill(&mut self, path: &Path, paint: &Paint) {
        let ink = color(paint);
        if let Some(bounds) = self.as_rect(path) {
            self.masked(|window| window.paint_quad(fill(bounds, ink)));
            return;
        }
        let mut builder = PathBuilder::fill();
        self.trace(&mut builder, path);
        if let Ok(built) = builder.build() {
            self.masked(|window| window.paint_path(built, ink));
        }
    }

    fn stroke(&mut self, path: &Path, stroke: &opensheet_print::Stroke) {
        let ink = color(&stroke.paint);
        let width = (stroke.width * self.stretch()) as f32;
        if stroke.dash.is_empty() && self.upright() {
            // Lines and rectangles along the pixel grid.
            let points: Vec<Point<Pixels>> = path
                .segments
                .iter()
                .filter_map(|segment| match *segment {
                    Segment::MoveTo(x, y) | Segment::LineTo(x, y) => Some(self.at(x, y)),
                    _ => None,
                })
                .collect();
            let straight = points.len() >= 2
                && points.len() + matches!(path.segments.last(), Some(Segment::Close)) as usize
                    == path.segments.len()
                && points
                    .windows(2)
                    .all(|pair| pair[0].x == pair[1].x || pair[0].y == pair[1].y);
            if straight {
                let closed = matches!(path.segments.last(), Some(Segment::Close));
                for pair in points.windows(2) {
                    self.hairline(pair[0], pair[1], width, ink);
                }
                if closed {
                    self.hairline(points[points.len() - 1], points[0], width, ink);
                }
                return;
            }
        }
        let mut builder = PathBuilder::stroke(px(width.max(0.5)));
        if !stroke.dash.is_empty() {
            let stretch = self.stretch();
            let dashes: Vec<Pixels> = stroke
                .dash
                .iter()
                .map(|d| px(((d * stretch) as f32).max(1.0)))
                .collect();
            builder = builder.dash_array(&dashes);
        }
        self.trace(&mut builder, path);
        if let Ok(built) = builder.build() {
            self.masked(|window| window.paint_path(built, ink));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        face: &opensheet_print::Face,
        font_size: f64,
        x: f64,
        y: f64,
        text: &str,
        paint: &Paint,
        cx: &mut App,
    ) {
        let ink = color(paint);
        let pixels = (font_size * self.stretch()) as f32;
        if pixels < 0.5 || text.is_empty() {
            return;
        }
        let font = Font {
            family: SharedString::from(face.family().to_string()),
            features: Default::default(),
            fallbacks: None,
            weight: if face.bold() {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if face.italic() {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
        };
        let shown: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: shown.len(),
            font: font.clone(),
            color: ink,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self
            .window
            .text_system()
            .shape_line(shown.clone(), px(pixels), &[run], None);
        let start = self.at(x, y);
        let [a, b, ..] = self.state.matrix;
        // Clockwise on the screen, where y grows downwards.
        let turn = b.atan2(a) as f32;
        if turn.abs() < 1e-3 {
            // The baseline of the line goes through the point.
            let line_height = line.ascent + line.descent;
            let origin = point(start.x, start.y - line.ascent);
            self.masked(|window| {
                let _ = line.paint(origin, line_height, window, cx);
            });
            return;
        }
        // GPUI cannot turn glyphs: turned text is painted as a shape (see
        // `chart_view::text_asset`, whose paths these are).
        let width = f32::from(line.width).ceil() + 2.0;
        let height = (pixels * 1.25).round();
        let baseline = f32::from(line.ascent);
        let path = crate::chart_view::text_asset_path(
            width,
            height,
            baseline,
            pixels,
            face.bold(),
            &font.family,
            &shown,
        );
        // The shape centres the text in its box: put the start of the
        // baseline on the point, then turn around it.
        let advance = f32::from(line.width);
        let bounds = Bounds::new(
            point(
                start.x - px((width - advance) / 2.0),
                start.y - px(baseline),
            ),
            size(px(width), px(height)),
        );
        self.masked(|window| {
            crate::chart_view::paint_turned(path, bounds, start, turn, ink, window, cx)
        });
    }
}

/// Paint a page so that it fills `bounds` (which has the proportions of
/// the paper).
pub(crate) fn paint(page: &Prepared, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let scene = &page.scene;
    if scene.size.0 <= 0.0 {
        return;
    }
    let mut brush = Brush {
        origin: bounds.origin,
        scale: f32::from(bounds.size.width) as f64 / scene.size.0,
        state: State {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            clip: bounds.intersect(&window.content_mask().bounds),
        },
        stack: Vec::new(),
        window,
    };
    for op in &scene.ops {
        match op {
            Op::Save => brush.stack.push(brush.state),
            Op::Restore => {
                if let Some(state) = brush.stack.pop() {
                    brush.state = state;
                }
            }
            Op::Clip(rect) => {
                // The box around the rectangle, should it be turned.
                let corners = [
                    brush.at(rect.x, rect.y),
                    brush.at(rect.right(), rect.y),
                    brush.at(rect.right(), rect.bottom()),
                    brush.at(rect.x, rect.bottom()),
                ];
                let min = |f: fn(&Point<Pixels>) -> Pixels| {
                    corners.iter().map(f).fold(px(f32::MAX), Pixels::min)
                };
                let max = |f: fn(&Point<Pixels>) -> Pixels| {
                    corners.iter().map(f).fold(px(f32::MIN), Pixels::max)
                };
                let around = Bounds::from_corners(
                    point(min(|p| p.x), min(|p| p.y)),
                    point(max(|p| p.x), max(|p| p.y)),
                );
                brush.state.clip = brush.state.clip.intersect(&around);
            }
            Op::Transform(matrix) => brush.state.matrix = compose(brush.state.matrix, *matrix),
            Op::Fill { path, paint } => brush.fill(path, paint),
            Op::Stroke { path, stroke } => brush.stroke(path, stroke),
            Op::Text {
                face,
                size,
                x,
                y,
                text,
                paint,
            } => brush.text(face, *size, *x, *y, text, paint, cx),
            Op::Image { image, rect } => {
                let key = Arc::as_ptr(image) as usize;
                let Some((_, decoded)) = page.images.iter().find(|(k, _)| *k == key) else {
                    continue;
                };
                let decoded = decoded.clone();
                let place = Bounds::from_corners(
                    brush.at(rect.x, rect.y),
                    brush.at(rect.right(), rect.bottom()),
                );
                brush.masked(|window| {
                    let _ = window.paint_image(place, Corners::default(), decoded, 0, false);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transformations_compose_inside_out() {
        let shift = [1.0, 0.0, 0.0, 1.0, 10.0, 20.0];
        let double = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        // Scale first, then shift: (3, 4) → (6, 8) → (16, 28).
        let m = compose(shift, double);
        assert_eq!(
            (
                m[0] * 3.0 + m[2] * 4.0 + m[4],
                m[1] * 3.0 + m[3] * 4.0 + m[5]
            ),
            (16.0, 28.0)
        );
        // The other way round: (3, 4) → (13, 24) → (26, 48).
        let m = compose(double, shift);
        assert_eq!(
            (
                m[0] * 3.0 + m[2] * 4.0 + m[4],
                m[1] * 3.0 + m[3] * 4.0 + m[5]
            ),
            (26.0, 48.0)
        );
    }

    #[test]
    fn raw_pixels_make_a_png() {
        // Two pixels, red then blue.
        let file = png(2, 1, 3, &[255, 0, 0, 0, 0, 255]);
        assert_eq!(
            &file[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        assert_eq!(&file[12..16], b"IHDR");
        assert_eq!(&file[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
        // The print crate reads it back to the same pixels.
        let image = Image::from_file_bytes(&file).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(
            image.pixels,
            opensheet_print::Pixels::Rgb(vec![255, 0, 0, 0, 0, 255])
        );
        let file = png(1, 2, 4, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let image = Image::from_file_bytes(&file).unwrap();
        assert_eq!(
            image.pixels,
            opensheet_print::Pixels::Rgba(vec![1, 2, 3, 4, 5, 6, 7, 8])
        );
        // The checksum of the standard test vector.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
