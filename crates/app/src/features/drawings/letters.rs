//! The text of a shape as outlines: every letter a closed path, taken
//! from its font, that can be moved point by point. This is how warped
//! text (Text Effects ▸ Transform) is drawn, and text whose runs differ
//! in size, which GPUI's one-size layout cannot show; gradients and
//! reflections are drawn on the same paths.
//!
//! The paths are made once for the screen and for print: the screen
//! shows them as a vector image (`svg.rs`), a page gets them as filled
//! paths, so that a warp prints as it shows.
//!
//! Outlines come from the TrueType fonts of the machine (the print
//! crate reads them). A font without such outlines (PostScript-flavoured
//! OpenType) gives none: the text is then drawn the plain way, unwarped.

use super::svg;
use crate::grid::{FontResolver, pt_to_px};
use opensheet_engine::drawing::{Shape, TEXT_MARGIN, TextAlign, TextAnchor};
use opensheet_engine::style::{Color, Underline};
use opensheet_engine::textwarp::{LinePlace, WarpAlign, WarpLine};
use opensheet_engine::wordart::{
    GradientKind, TextEffects, TextGradient, TextReflection, TextShadow,
};
use opensheet_print::{Canvas, Face, FontDb, LineJoin, OutlineStep, Paint, Path, RectF, Stroke};
use std::fmt::Write as _;
use std::sync::Arc;

type P = (f64, f64);

/// A closed outline as its corners.
pub type Contour = Vec<P>;

/// The paths of the characters of one run (or of the part of it on one
/// line), in one look.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub contours: Vec<Contour>,
    pub color: Color,
    pub effects: TextEffects,
    /// Width of a line to draw around the paths in their own colour, for
    /// a bold the font has no file for; 0 for none.
    pub thicken: f64,
}

/// The text of a shape as paths, measured from the top-left corner of the
/// shape as it lies level; lengths are pixels at the zoom it was made for.
#[derive(Clone, Debug, PartialEq)]
pub struct Letters {
    pub pieces: Vec<Piece>,
    /// The box of the text (left, top, right, bottom): what a gradient
    /// spans and what a reflection mirrors.
    pub bounds: (f64, f64, f64, f64),
    /// Pixels per point.
    pub per_point: f64,
}

/// One character on its line, before the line is placed.
struct Glyph {
    piece: usize,
    /// Where its origin is along the line.
    x: f64,
    advance: f64,
    size: f64,
    slanted: bool,
    steps: Vec<OutlineStep>,
}

/// An underline or a strikethrough: a bar along the line.
#[derive(Clone, Copy)]
struct Bar {
    piece: usize,
    from: f64,
    to: f64,
    /// Its middle, below the baseline.
    y: f64,
    thickness: f64,
}

#[derive(Default)]
struct Line {
    glyphs: Vec<Glyph>,
    bars: Vec<Bar>,
    width: f64,
    ascent: f64,
    descent: f64,
    /// The largest size of letters on the line.
    size: f64,
}

/// How much a slanted letter leans, for an italic the font has no file
/// for.
const SLANT: f64 = 0.2;

/// How long the straight pieces a path is cut into are at most, as a
/// fraction of the size of the letters: short enough for a warp to bend
/// a straight stroke smoothly.
const PIECE: f64 = 1.0 / 16.0;

impl Letters {
    /// The text of a shape as paths, in its frame `width` by `height`
    /// pixels at a zoom of `zoom`. `None` when the text is empty or one
    /// of its fonts has no outlines to give.
    pub fn of(
        shape: &Shape,
        width: f64,
        height: f64,
        zoom: f64,
        fonts: &FontResolver,
    ) -> Option<Letters> {
        let resolve = |name: &Arc<str>| fonts.resolve(name).to_string();
        Letters::with_fonts(shape, width, height, zoom, &resolve, FontDb::system())
    }

    /// `of` with the fonts given: `family` names the family a font of the
    /// workbook is drawn with on this machine.
    pub fn with_fonts(
        shape: &Shape,
        width: f64,
        height: f64,
        zoom: f64,
        family: &dyn Fn(&Arc<str>) -> String,
        db: &FontDb,
    ) -> Option<Letters> {
        if shape.text.trim().is_empty() {
            return None;
        }
        let per_point = pt_to_px(1.0) as f64 * zoom;
        let frame = text_frame(shape, width, height, zoom);
        let (room_x, room_y) = (frame.2 - frame.0, frame.3 - frame.1);

        // One piece per run; its characters are set on lines.
        let runs = shape.text_runs();
        let mut pieces: Vec<Piece> = Vec::with_capacity(runs.len());
        let mut faces: Vec<(Arc<Face>, f64)> = Vec::with_capacity(runs.len());
        for (_, font, effects) in &runs {
            let face = db.face(&family(&font.name), font.bold, font.italic);
            let size = font.size_pt() as f64 * per_point;
            let (bold, _) = face.synthetic_styles();
            pieces.push(Piece {
                contours: Vec::new(),
                color: font.color.unwrap_or(Color::BLACK),
                effects: *effects,
                thicken: if bold { size * 0.03 } else { 0.0 },
            });
            faces.push((face, size));
        }
        let mut lines: Vec<Line> = Vec::new();
        let mut start = 0;
        for paragraph in shape.text.split('\n') {
            let end = start + paragraph.len();
            let mut line = Line::default();
            for (i, (covered, font, _)) in runs.iter().enumerate() {
                let (from, to) = (covered.start.max(start), covered.end.min(end));
                let (face, size) = &faces[i];
                // An empty line is as high as the letters it would hold.
                if covered.start <= end && covered.end >= start {
                    line.ascent = line.ascent.max(face.ascent() * size);
                    line.descent = line.descent.max(face.descent() * size);
                    line.size = line.size.max(*size);
                }
                if from >= to {
                    continue;
                }
                let begun = line.width;
                // Characters the font lacks come from another one.
                for (drawn_by, text) in db.runs(face, &shape.text[from..to]) {
                    let (_, slanted) = drawn_by.synthetic_styles();
                    for ch in text.chars() {
                        let (steps, advance) = drawn_by.outline(ch)?;
                        line.glyphs.push(Glyph {
                            piece: i,
                            x: line.width,
                            advance: advance * size,
                            size: *size,
                            slanted,
                            steps,
                        });
                        line.width += advance * size;
                    }
                }
                let mut bar = |y: f64, thickness: f64| {
                    line.bars.push(Bar {
                        piece: i,
                        from: begun,
                        to: line.width,
                        y: y * size,
                        thickness: (thickness * size).max(per_point * 0.5),
                    });
                };
                if font.underline != Underline::None {
                    bar(face.underline_offset(), face.underline_thickness());
                }
                if font.strike {
                    bar(-face.strikeout_offset(), face.strikeout_thickness());
                }
            }
            lines.push(line);
            start = end + 1;
        }

        let align = match shape.align {
            TextAlign::Left | TextAlign::Justify => WarpAlign::Left,
            TextAlign::Center => WarpAlign::Centre,
            TextAlign::Right => WarpAlign::Right,
        };
        let mut bounds = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        if let Some(warp) = &shape.warp {
            // Warped: the guide curves fill the text rectangle, and the
            // lines are dealt out to them.
            let geometry = warp.geometry(room_x, room_y);
            let measured: Vec<WarpLine> = lines
                .iter()
                .map(|line| WarpLine {
                    width: line.width,
                    ascent: line.ascent,
                    descent: line.descent,
                })
                .collect();
            let mut places = geometry.place(&measured, align);
            // Between two curves Excel stretches the ink of a line, not the
            // box of its font: the letters reach both curves and both ends
            // (seen by opening the same file in Excel). Letters set on a
            // path keep the measures of their font.
            let stretched = places
                .iter()
                .all(|place| matches!(place, LinePlace::Between { .. }));
            let inked: Vec<Option<Ink>> = lines
                .iter()
                .map(|line| if stretched { ink(line) } else { None })
                .collect();
            let measured: Vec<WarpLine> = measured
                .iter()
                .zip(&inked)
                .map(|(measure, ink)| match ink {
                    Some(ink) => WarpLine {
                        width: ink.right - ink.left,
                        ascent: ink.above,
                        descent: ink.below,
                    },
                    None => *measure,
                })
                .collect();
            if stretched {
                places = geometry.place(&measured, align);
            }
            for (((line, measure), place), ink) in
                lines.iter().zip(&measured).zip(&places).zip(&inked)
            {
                let bends = matches!(place, LinePlace::Between { .. });
                let left = ink.as_ref().map_or(0.0, |ink| ink.left);
                let map = |x: f64, y: f64, centre: f64| {
                    let at = geometry.map(place, measure, x - left, y, centre - left);
                    (at.0 + frame.0, at.1 + frame.1)
                };
                trace(line, &mut pieces, bends, &map);
            }
            for point in pieces
                .iter()
                .flat_map(|piece| piece.contours.iter().flatten())
            {
                bounds = (
                    bounds.0.min(point.0),
                    bounds.1.min(point.1),
                    bounds.2.max(point.0),
                    bounds.3.max(point.1),
                );
            }
        } else {
            // Level: lines broken at the width of the rectangle, stacked
            // and held at the top, the middle or the bottom of it.
            let lines = if shape.wrap {
                wrapped(lines, room_x)
            } else {
                lines
            };
            let heights: Vec<f64> = lines
                .iter()
                .map(|line| (line.size * 1.2).round().max(1.0))
                .collect();
            let total: f64 = heights.iter().sum();
            let mut y = frame.1
                + match shape.anchor {
                    TextAnchor::Top => 0.0,
                    TextAnchor::Middle => ((room_y - total) / 2.0).round(),
                    TextAnchor::Bottom => room_y - total,
                };
            for (line, line_height) in lines.iter().zip(&heights) {
                let x = frame.0
                    + match align {
                        WarpAlign::Left => 0.0,
                        WarpAlign::Centre => (room_x - line.width) / 2.0,
                        WarpAlign::Right => room_x - line.width,
                    };
                // The baseline where GPUI puts it in a line of this height.
                let baseline = y + (line_height - line.ascent - line.descent) / 2.0 + line.ascent;
                trace(line, &mut pieces, false, &|px, py, _| {
                    (x + px, baseline + py)
                });
                bounds = (
                    bounds.0.min(x),
                    bounds.1.min(y),
                    bounds.2.max(x + line.width),
                    bounds.3.max(y + line_height),
                );
                y += line_height;
            }
        }
        if !bounds.0.is_finite() || !bounds.2.is_finite() {
            return None;
        }
        Some(Letters {
            pieces,
            bounds,
            per_point,
        })
    }

    /// How far around its bounds the text may reach: outlines, shadows,
    /// slanted letters.
    fn pad(&self) -> f64 {
        let reach = self
            .pieces
            .iter()
            .map(|piece| {
                let outline = piece.effects.outline.map_or(0.0, |line| line.width as f64);
                let shadow = piece
                    .effects
                    .shadow
                    .map_or(0.0, |shadow| (shadow.distance + 2.0 * shadow.blur) as f64);
                (outline + shadow) * self.per_point + piece.thicken
            })
            .fold(0.0, f64::max);
        (reach + 4.0).ceil()
    }

    /// Where the ink of the letters is, from its top to its bottom. A
    /// reflection mirrors the ink, not the box of the lines: opened in
    /// Excel, the mirror image of a word starts right under its letters.
    fn ink_span(&self) -> (f64, f64) {
        let ys = self
            .pieces
            .iter()
            .flat_map(|piece| piece.contours.iter().flatten())
            .map(|point| point.1);
        let (top, bottom) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(top, bottom), y| {
            (top.min(y), bottom.max(y))
        });
        if top.is_finite() && bottom > top {
            (top, bottom)
        } else {
            (self.bounds.1, self.bounds.3)
        }
    }

    /// The reflection of the text, if any piece has one: how far under
    /// the bounds it reaches.
    fn reflection_reach(&self) -> f64 {
        let height = self.bounds.3 - self.bounds.1;
        self.pieces
            .iter()
            .filter_map(|piece| piece.effects.reflection)
            .map(|reflection| {
                reflection.distance as f64 * self.per_point + reflection.reach as f64 * height
            })
            .fold(0.0, f64::max)
    }

    /// The vector image that draws the text, turned by `turn` degrees
    /// about `pivot`, and the box it is painted in (left, top, width,
    /// height, measured like the paths). `scale`: device pixels per pixel.
    pub fn svg(&self, turn: f64, pivot: P, scale: f64) -> (String, (f64, f64, f64, f64)) {
        let pad = self.pad();
        let (x0, y0, x1, y1) = (
            self.bounds.0 - pad,
            self.bounds.1 - pad,
            self.bounds.2 + pad,
            self.bounds.3 + pad + self.reflection_reach(),
        );
        let turned = turn.rem_euclid(360.0) > 0.05 && turn.rem_euclid(360.0) < 359.95;
        // The image holds the box, or the box turned about the pivot.
        let (left, top, right, bottom) = if turned {
            let (sin, cos) = turn.to_radians().sin_cos();
            let corners = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| {
                let (dx, dy) = (x - pivot.0, y - pivot.1);
                (pivot.0 + dx * cos - dy * sin, pivot.1 + dx * sin + dy * cos)
            });
            (
                corners
                    .iter()
                    .map(|c| c.0)
                    .fold(f64::INFINITY, f64::min)
                    .floor(),
                corners
                    .iter()
                    .map(|c| c.1)
                    .fold(f64::INFINITY, f64::min)
                    .floor(),
                corners
                    .iter()
                    .map(|c| c.0)
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil(),
                corners
                    .iter()
                    .map(|c| c.1)
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil(),
            )
        } else {
            (x0.floor(), y0.floor(), x1.ceil(), y1.ceil())
        };
        let (w, h) = ((right - left).max(1.0), (bottom - top).max(1.0));
        let fit = (svg::MAX_SIDE as f64 / (w.max(h) * scale).max(1.0)).min(1.0);
        let mut out = String::new();
        let _ = write!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {w} {h}">"#,
            (w * scale * fit).ceil(),
            (h * scale * fit).ceil(),
        );
        // What the paths refer to: a blur per kind of shadow, a paint per
        // gradient, the fading of each reflection.
        let mut shadows: Vec<TextShadow> = Vec::new();
        let mut gradients: Vec<TextGradient> = Vec::new();
        let mut reflections: Vec<TextReflection> = Vec::new();
        for piece in &self.pieces {
            if let Some(shadow) = piece
                .effects
                .shadow
                .filter(|shadow| !shadows.contains(shadow))
            {
                shadows.push(shadow);
            }
            if let Some(gradient) = piece
                .effects
                .gradient
                .filter(|gradient| !gradients.contains(gradient))
            {
                gradients.push(gradient);
            }
            if let Some(reflection) = piece
                .effects
                .reflection
                .filter(|reflection| !reflections.contains(reflection))
            {
                reflections.push(reflection);
            }
        }
        let ink = self.ink_span();
        out.push_str("<defs>");
        for (i, shadow) in shadows.iter().enumerate() {
            let spread = shadow.blur as f64 * self.per_point / 2.0;
            if spread > 0.0 {
                let _ = write!(
                    out,
                    r#"<filter id="s{i}" x="-20%" y="-60%" width="140%" height="220%"><feGaussianBlur stdDeviation="{spread:.2}"/></filter>"#
                );
            }
        }
        for (i, gradient) in gradients.iter().enumerate() {
            let (from, to) = gradient_line(gradient.kind, self.bounds);
            match gradient.kind {
                GradientKind::Linear(_) => {
                    let _ = write!(
                        out,
                        r#"<linearGradient id="g{i}" gradientUnits="userSpaceOnUse" x1="{:.2}" y1="{:.2}" x2="{:.2}" y2="{:.2}">"#,
                        from.0, from.1, to.0, to.1
                    );
                }
                GradientKind::FromCentre => {
                    let _ = write!(
                        out,
                        r#"<radialGradient id="g{i}" gradientUnits="userSpaceOnUse" cx="{:.2}" cy="{:.2}" r="{:.2}">"#,
                        from.0,
                        from.1,
                        (to.0 - from.0).hypot(to.1 - from.1)
                    );
                }
            }
            for stop in gradient.stops() {
                let _ = write!(
                    out,
                    r#"<stop offset="{:.4}" stop-color="{}"/>"#,
                    stop.at,
                    hex(stop.color)
                );
            }
            out.push_str(match gradient.kind {
                GradientKind::Linear(_) => "</linearGradient>",
                GradientKind::FromCentre => "</radialGradient>",
            });
        }
        for (i, reflection) in reflections.iter().enumerate() {
            // The mirror image starts under the text and fades out over
            // the part of its height it reaches.
            let from = ink.1 + reflection.distance as f64 * self.per_point;
            let to = from + (reflection.reach as f64 * (ink.1 - ink.0)).max(1.0);
            let _ = write!(
                out,
                r#"<linearGradient id="f{i}" gradientUnits="userSpaceOnUse" x1="0" y1="{from:.2}" x2="0" y2="{to:.2}"><stop offset="0" stop-color="white" stop-opacity="{:.3}"/><stop offset="1" stop-color="white" stop-opacity="0"/></linearGradient>"#,
                reflection.opacity.clamp(0.0, 1.0)
            );
            let _ = write!(
                out,
                r#"<mask id="m{i}" maskUnits="userSpaceOnUse" x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}"><rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="url(#f{i})"/></mask>"#,
                x0,
                from - 1.0,
                x1 - x0,
                to - from + 2.0,
                x0,
                from - 1.0,
                x1 - x0,
                to - from + 2.0
            );
            let spread = reflection.blur as f64 * self.per_point / 2.0;
            if spread > 0.0 {
                let _ = write!(
                    out,
                    r#"<filter id="r{i}"><feGaussianBlur stdDeviation="{spread:.2}"/></filter>"#
                );
            }
        }
        out.push_str("</defs>");
        let _ = write!(
            out,
            r#"<g transform="translate({:.2} {:.2})">"#,
            -left, -top
        );
        if turned {
            let _ = write!(
                out,
                r#"<g transform="rotate({turn:.2} {:.2} {:.2})">"#,
                pivot.0, pivot.1
            );
        }
        // Shadows lie under everything, reflections under the letters.
        for (i, shadow) in shadows.iter().enumerate() {
            let (dx, dy) = shadow.shift();
            let _ = write!(
                out,
                r#"<g transform="translate({:.2} {:.2})""#,
                dx as f64 * self.per_point,
                dy as f64 * self.per_point
            );
            if shadow.blur > 0.0 {
                let _ = write!(out, r#" filter="url(#s{i})""#);
            }
            out.push('>');
            for piece in self
                .pieces
                .iter()
                .filter(|piece| piece.effects.shadow.as_ref() == Some(shadow))
            {
                self.path(piece, Some(shadow), &gradients, &mut out);
            }
            out.push_str("</g>");
        }
        for (i, reflection) in reflections.iter().enumerate() {
            // Mirrored about the line half the gap under the text.
            let mirror = ink.1 + reflection.distance as f64 * self.per_point / 2.0;
            let _ = write!(out, r#"<g mask="url(#m{i})""#);
            if reflection.blur > 0.0 {
                let _ = write!(out, r#" filter="url(#r{i})""#);
            }
            let _ = write!(
                out,
                r#"><g transform="translate(0 {:.2}) scale(1 -1)">"#,
                2.0 * mirror
            );
            for piece in self
                .pieces
                .iter()
                .filter(|piece| piece.effects.reflection.as_ref() == Some(reflection))
            {
                self.path(piece, None, &gradients, &mut out);
            }
            out.push_str("</g></g>");
        }
        for piece in &self.pieces {
            self.path(piece, None, &gradients, &mut out);
        }
        if turned {
            out.push_str("</g>");
        }
        out.push_str("</g></svg>");
        (out, (left, top, w, h))
    }

    /// One piece as a path element: in its own look, or as the shadow it
    /// casts.
    fn path(
        &self,
        piece: &Piece,
        shadow: Option<&TextShadow>,
        gradients: &[TextGradient],
        out: &mut String,
    ) {
        if piece.contours.is_empty() {
            return;
        }
        out.push_str(r#"<path d=""#);
        for contour in &piece.contours {
            for (i, point) in contour.iter().enumerate() {
                let _ = write!(
                    out,
                    "{}{:.2} {:.2}",
                    if i == 0 { "M" } else { "L" },
                    point.0,
                    point.1
                );
            }
            out.push('Z');
        }
        out.push('"');
        let outline = piece
            .effects
            .outline
            .map(|line| (line.color, (line.width as f64 * self.per_point).max(0.5)));
        match shadow {
            Some(shadow) => {
                let (ink, opacity) = (hex(shadow.color), shadow.opacity.clamp(0.0, 1.0));
                if piece.effects.hollow {
                    out.push_str(r#" fill="none""#);
                } else {
                    let _ = write!(out, r#" fill="{ink}" fill-opacity="{opacity}""#);
                }
                if let Some((_, width)) = outline {
                    let _ = write!(
                        out,
                        r#" stroke="{ink}" stroke-opacity="{opacity}" stroke-width="{width:.2}" stroke-linejoin="round""#
                    );
                }
            }
            None => {
                let paint = match piece
                    .effects
                    .gradient
                    .and_then(|gradient| gradients.iter().position(|known| *known == gradient))
                {
                    Some(index) => format!("url(#g{index})"),
                    None => hex(piece.color),
                };
                if piece.effects.hollow {
                    out.push_str(r#" fill="none""#);
                } else {
                    let _ = write!(out, r#" fill="{paint}""#);
                }
                if let Some((color, width)) = outline {
                    let _ = write!(
                        out,
                        r#" stroke="{}" stroke-width="{width:.2}" stroke-linejoin="round""#,
                        hex(color)
                    );
                } else if piece.thicken > 0.0 && !piece.effects.hollow {
                    let _ = write!(
                        out,
                        r#" stroke="{paint}" stroke-width="{:.2}" stroke-linejoin="round""#,
                        piece.thicken
                    );
                }
            }
        }
        out.push_str("/>");
    }
}

/// The rectangle the text of a shape is set in (left, top, right,
/// bottom) in its frame `width` by `height` at a zoom: the text rectangle
/// of the shape less its margins. A warp fills it.
pub fn text_frame(shape: &Shape, width: f64, height: f64, zoom: f64) -> (f64, f64, f64, f64) {
    let [left, top, right, bottom] = shape.text_rect(width as f32, height as f32);
    let (margin_x, margin_y) = (TEXT_MARGIN.0 as f64 * zoom, TEXT_MARGIN.1 as f64 * zoom);
    (
        left as f64 * width + margin_x,
        top as f64 * height + margin_y,
        (right as f64 * width - margin_x).max(left as f64 * width + margin_x + 1.0),
        (bottom as f64 * height - margin_y).max(top as f64 * height + margin_y + 1.0),
    )
}

// ---- on a page ---------------------------------------------------------------

/// How many bands a gradient or the fading of a reflection is printed in.
const BANDS: usize = 32;

impl Letters {
    /// The contours of a piece as one path, each point seen through
    /// `through`.
    fn page_path(piece: &Piece, through: &dyn Fn(P) -> P) -> Path {
        let mut path = Path::new();
        for contour in &piece.contours {
            for (i, point) in contour.iter().enumerate() {
                let (x, y) = through(*point);
                path = if i == 0 {
                    path.move_to(x, y)
                } else {
                    path.line_to(x, y)
                };
            }
            path = path.close();
        }
        path
    }

    /// Fill a piece in its own paint: its colour, or its gradient cut
    /// into bands of one colour each across the way it runs (a page has
    /// paths of one colour only). A gradient from the centre has no such
    /// bands: it is printed in the colour of its middle.
    fn page_fill(&self, piece: &Piece, opacity: f32, canvas: &mut Canvas) {
        let plain = |color: Color, canvas: &mut Canvas| {
            canvas.fill_path(
                Letters::page_path(piece, &|p| p),
                Paint::translucent(color, opacity),
            );
        };
        match piece.effects.gradient {
            None => plain(piece.color, canvas),
            Some(gradient) => match gradient.kind {
                GradientKind::FromCentre => plain(gradient.color_at(0.5), canvas),
                GradientKind::Linear(angle) => {
                    let (from, to) = gradient_line(gradient.kind, self.bounds);
                    let length = (to.0 - from.0).hypot(to.1 - from.1).max(1e-6);
                    // Seen with the gradient running along x from `from`:
                    // each band is then an upright rectangle to clip to.
                    let (sin, cos) = (angle as f64).to_radians().sin_cos();
                    let level = |p: P| {
                        let (dx, dy) = (p.0 - from.0, p.1 - from.1);
                        (dx * cos + dy * sin, -dx * sin + dy * cos)
                    };
                    let path = Letters::page_path(piece, &level);
                    let reach = (self.bounds.2 - self.bounds.0)
                        .hypot(self.bounds.3 - self.bounds.1)
                        + self.pad();
                    canvas.save();
                    canvas.transform([cos, sin, -sin, cos, from.0, from.1]);
                    for band in 0..BANDS {
                        let (start, end) =
                            (band as f64 / BANDS as f64, (band + 1) as f64 / BANDS as f64);
                        // The first and the last band go on to the edge
                        // of the letters; neighbours overlap a hair, so
                        // that no gap shows between them.
                        let x0 = if band == 0 { -reach } else { start * length };
                        let x1 = if band + 1 == BANDS {
                            length + reach
                        } else {
                            end * length + 0.25
                        };
                        canvas.save();
                        canvas.clip_rect(RectF::new(x0, -reach, x1 - x0, 2.0 * reach));
                        canvas.fill_path(
                            path.clone(),
                            Paint::translucent(
                                gradient.color_at(((start + end) / 2.0) as f32),
                                opacity,
                            ),
                        );
                        canvas.restore();
                    }
                    canvas.restore();
                }
            },
        }
    }

    /// Draw the text on a page: shadows (not blurred), reflections,
    /// then the letters with their outlines.
    pub fn print(&self, canvas: &mut Canvas) {
        let outline = |piece: &Piece| {
            piece
                .effects
                .outline
                .map(|line| (line.color, (line.width as f64 * self.per_point).max(0.5)))
        };
        let round = |color: Color, opacity: f32, width: f64| {
            Stroke::solid(Paint::translucent(color, opacity), width).join(LineJoin::Round)
        };
        for piece in &self.pieces {
            let Some(shadow) = piece.effects.shadow else {
                continue;
            };
            let (dx, dy) = shadow.shift();
            let moved = |p: P| {
                (
                    p.0 + dx as f64 * self.per_point,
                    p.1 + dy as f64 * self.per_point,
                )
            };
            let opacity = shadow.opacity.clamp(0.0, 1.0);
            if !piece.effects.hollow {
                canvas.fill_path(
                    Letters::page_path(piece, &moved),
                    Paint::translucent(shadow.color, opacity),
                );
            }
            if let Some((_, width)) = outline(piece) {
                canvas.stroke_path(
                    Letters::page_path(piece, &moved),
                    round(shadow.color, opacity, width),
                );
            }
        }
        let (x0, _, x1, _) = self.bounds;
        let (ink_top, bottom) = self.ink_span();
        let height = bottom - ink_top;
        for piece in &self.pieces {
            let Some(reflection) = piece.effects.reflection else {
                continue;
            };
            // Mirrored about the line half the gap under the text, and
            // shown band by band, each fainter than the one above.
            let gap = reflection.distance as f64 * self.per_point;
            let reach = (reflection.reach as f64 * height).max(1.0);
            let pad = self.pad();
            let bands = BANDS / 2;
            for band in 0..bands {
                let (start, end) = (band as f64 / bands as f64, (band + 1) as f64 / bands as f64);
                let opacity =
                    reflection.opacity.clamp(0.0, 1.0) * (1.0 - ((start + end) / 2.0) as f32);
                canvas.save();
                canvas.clip_rect(RectF::new(
                    x0 - pad,
                    bottom + gap + start * reach,
                    x1 - x0 + 2.0 * pad,
                    (end - start) * reach,
                ));
                canvas.transform([1.0, 0.0, 0.0, -1.0, 0.0, 2.0 * bottom + gap]);
                if !piece.effects.hollow {
                    self.page_fill(piece, opacity, canvas);
                }
                if let Some((color, width)) = outline(piece) {
                    canvas.stroke_path(
                        Letters::page_path(piece, &|p| p),
                        round(color, opacity, width),
                    );
                }
                canvas.restore();
            }
        }
        for piece in &self.pieces {
            if !piece.effects.hollow {
                self.page_fill(piece, 1.0, canvas);
            }
            if let Some((color, width)) = outline(piece) {
                canvas.stroke_path(Letters::page_path(piece, &|p| p), round(color, 1.0, width));
            } else if piece.thicken > 0.0
                && !piece.effects.hollow
                && piece.effects.gradient.is_none()
            {
                canvas.stroke_path(
                    Letters::page_path(piece, &|p| p),
                    round(piece.color, 1.0, piece.thicken),
                );
            }
        }
    }
}

fn hex(color: Color) -> String {
    format!("#{:06X}", color.value() & 0xFF_FFFF)
}

/// Where a gradient starts and ends over a box: the two ends of the line
/// it runs along or, from the centre, the centre and a corner.
pub fn gradient_line(kind: GradientKind, bounds: (f64, f64, f64, f64)) -> (P, P) {
    let centre = ((bounds.0 + bounds.2) / 2.0, (bounds.1 + bounds.3) / 2.0);
    match kind {
        GradientKind::Linear(angle) => {
            let (sin, cos) = (angle as f64).to_radians().sin_cos();
            // As far along the direction as the corners of the box reach.
            let reach = ((bounds.2 - bounds.0) / 2.0 * cos).abs()
                + ((bounds.3 - bounds.1) / 2.0 * sin).abs();
            (
                (centre.0 - cos * reach, centre.1 - sin * reach),
                (centre.0 + cos * reach, centre.1 + sin * reach),
            )
        }
        GradientKind::FromCentre => (centre, (bounds.2, bounds.3)),
    }
}

/// Break lines that are wider than `room` where they have a blank.
fn wrapped(lines: Vec<Line>, room: f64) -> Vec<Line> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width <= room {
            out.push(line);
            continue;
        }
        let Line {
            glyphs,
            bars,
            ascent,
            descent,
            size,
            ..
        } = line;
        // Where each row starts: after the last blank that still fits.
        let mut starts = vec![0usize];
        let mut row_start = 0.0;
        let mut last_blank: Option<usize> = None;
        for (i, glyph) in glyphs.iter().enumerate() {
            let blank = glyph.steps.is_empty();
            let begun = *starts.last().unwrap_or(&0);
            if !blank && glyph.x + glyph.advance - row_start > room && i > begun {
                // Break after the last blank of the row, or right here
                // in a word longer than the row.
                let at = last_blank
                    .filter(|blank| *blank + 1 > begun)
                    .map_or(i, |blank| blank + 1);
                starts.push(at);
                row_start = glyphs[at].x;
                last_blank = None;
            }
            if blank {
                last_blank = Some(i);
            }
        }
        let mut glyphs = glyphs.into_iter().enumerate().peekable();
        for (row, start) in starts.iter().enumerate() {
            let end = starts.get(row + 1).copied().unwrap_or(usize::MAX);
            let mut part = Line {
                ascent,
                descent,
                size,
                ..Line::default()
            };
            let mut shift = None;
            while let Some((_, mut glyph)) = glyphs.next_if(|(i, _)| *i >= *start && *i < end) {
                let shift = *shift.get_or_insert(glyph.x);
                glyph.x -= shift;
                part.width = glyph.x + glyph.advance;
                part.glyphs.push(glyph);
            }
            // Blanks that end a row take no room.
            while part
                .glyphs
                .last()
                .is_some_and(|glyph| glyph.steps.is_empty())
                && part.glyphs.len() > 1
            {
                let dropped = part.glyphs.pop();
                part.width -= dropped.map_or(0.0, |glyph| glyph.advance);
            }
            let shift = shift.unwrap_or(0.0);
            for bar in &bars {
                let (from, to) = (
                    (bar.from - shift).max(0.0),
                    (bar.to - shift).min(part.width),
                );
                if from < to {
                    part.bars.push(Bar { from, to, ..*bar });
                }
            }
            out.push(part);
        }
    }
    out
}

/// Turn the glyphs and the bars of a line into paths, each point placed
/// by `map` (x along the line, y below the baseline, the middle of its
/// letter along the line). `bends`: straight strokes are cut into short
/// pieces, because `map` bends them.
/// Where the ink of a line is: from `left` to `right` along it, `above`
/// its baseline and `below` it (negative when no letter comes down to
/// the baseline).
struct Ink {
    left: f64,
    right: f64,
    above: f64,
    below: f64,
}

/// The ink of a line, from the outlines of its letters. `None` for a
/// line without ink (empty, or spaces only) or too thin to stretch.
fn ink(line: &Line) -> Option<Ink> {
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    let mut reach = |point: P| {
        bounds = (
            bounds.0.min(point.0),
            bounds.1.min(point.1),
            bounds.2.max(point.0),
            bounds.3.max(point.1),
        );
    };
    for glyph in &line.glyphs {
        // As in `trace`: from the font to the line, y downwards.
        let local = |x: f32, y: f32| {
            let (x, y) = (x as f64, y as f64);
            let x = if glyph.slanted { x + SLANT * y } else { x };
            (glyph.x + x * glyph.size, -y * glyph.size)
        };
        let mut pen = (0.0, 0.0);
        for outline in &glyph.steps {
            match *outline {
                OutlineStep::Move(x, y) | OutlineStep::Line(x, y) => {
                    pen = local(x, y);
                    reach(pen);
                }
                OutlineStep::Quad(cx, cy, x, y) => {
                    let (control, end) = (local(cx, cy), local(x, y));
                    // The curve goes furthest where its slope turns.
                    for (from, by, to) in [(pen.0, control.0, end.0), (pen.1, control.1, end.1)] {
                        let turn = from - 2.0 * by + to;
                        if turn.abs() > f64::EPSILON {
                            let t = (from - by) / turn;
                            if t > 0.0 && t < 1.0 {
                                let s = 1.0 - t;
                                let at = (
                                    s * s * pen.0 + 2.0 * s * t * control.0 + t * t * end.0,
                                    s * s * pen.1 + 2.0 * s * t * control.1 + t * t * end.1,
                                );
                                reach(at);
                            }
                        }
                    }
                    pen = end;
                    reach(pen);
                }
                OutlineStep::Close => {}
            }
        }
    }
    let size = line.size.max(1.0);
    (bounds.2 - bounds.0 > size * 0.05 && bounds.3 - bounds.1 > size * 0.05).then(|| Ink {
        left: bounds.0,
        right: bounds.2,
        above: -bounds.1,
        below: bounds.3,
    })
}

fn trace(line: &Line, pieces: &mut [Piece], bends: bool, map: &dyn Fn(f64, f64, f64) -> P) {
    for glyph in &line.glyphs {
        let centre = glyph.x + glyph.advance / 2.0;
        // From the font (its size as unit, y upwards) to the line.
        let local = |x: f32, y: f32| {
            let (x, y) = (x as f64, y as f64);
            let x = if glyph.slanted { x + SLANT * y } else { x };
            (glyph.x + x * glyph.size, -y * glyph.size)
        };
        let step = glyph.size * PIECE;
        let mut contour: Contour = Vec::new();
        let mut pen = (0.0, 0.0);
        let line_to = |contour: &mut Contour, pen: &mut P, to: P| {
            let count = if bends {
                ((to.0 - pen.0).hypot(to.1 - pen.1) / step).ceil().max(1.0) as usize
            } else {
                1
            };
            for i in 1..=count {
                let t = i as f64 / count as f64;
                contour.push(map(
                    pen.0 + (to.0 - pen.0) * t,
                    pen.1 + (to.1 - pen.1) * t,
                    centre,
                ));
            }
            *pen = to;
        };
        for outline in &glyph.steps {
            match *outline {
                OutlineStep::Move(x, y) => {
                    if contour.len() > 2 {
                        pieces[glyph.piece]
                            .contours
                            .push(std::mem::take(&mut contour));
                    }
                    contour.clear();
                    pen = local(x, y);
                    contour.push(map(pen.0, pen.1, centre));
                }
                OutlineStep::Line(x, y) => line_to(&mut contour, &mut pen, local(x, y)),
                OutlineStep::Quad(cx, cy, x, y) => {
                    let (control, end) = (local(cx, cy), local(x, y));
                    let length = (control.0 - pen.0).hypot(control.1 - pen.1)
                        + (end.0 - control.0).hypot(end.1 - control.1);
                    let count = (length / (step / 2.0)).ceil().clamp(2.0, 24.0) as usize;
                    for i in 1..=count {
                        let t = i as f64 / count as f64;
                        let s = 1.0 - t;
                        contour.push(map(
                            s * s * pen.0 + 2.0 * s * t * control.0 + t * t * end.0,
                            s * s * pen.1 + 2.0 * s * t * control.1 + t * t * end.1,
                            centre,
                        ));
                    }
                    pen = end;
                }
                OutlineStep::Close => {
                    if contour.len() > 2 {
                        pieces[glyph.piece]
                            .contours
                            .push(std::mem::take(&mut contour));
                    }
                    contour.clear();
                }
            }
        }
        if contour.len() > 2 {
            pieces[glyph.piece].contours.push(contour);
        }
    }
    for bar in &line.bars {
        // A bar bends with the line: it follows the letters above it.
        let count = if bends {
            ((bar.to - bar.from) / (line.size.max(1.0) * PIECE))
                .ceil()
                .max(1.0) as usize
        } else {
            1
        };
        let (top, bottom) = (bar.y - bar.thickness / 2.0, bar.y + bar.thickness / 2.0);
        let x = |i: usize| bar.from + (bar.to - bar.from) * i as f64 / count as f64;
        let mut contour: Contour = (0..=count).map(|i| map(x(i), top, x(i))).collect();
        contour.extend((0..=count).rev().map(|i| map(x(i), bottom, x(i))));
        pieces[bar.piece].contours.push(contour);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::ShapeTool;
    use opensheet_engine::textwarp::{TextWarp, WarpPreset};
    use opensheet_engine::wordart::WordArtStyle;

    /// The letters of a shape with the fonts of this machine, each family
    /// drawn by itself.
    fn letters(shape: &Shape, width: f64, height: f64) -> Option<Letters> {
        Letters::with_fonts(
            shape,
            width,
            height,
            1.0,
            &|name| name.to_string(),
            FontDb::system(),
        )
    }

    fn art(text: &str) -> Shape {
        let mut shape = WordArtStyle(1).shape(text);
        shape.font.set_name("Arial");
        shape.font.set_size_pt(36.0);
        shape
    }

    fn extent(letters: &Letters) -> (f64, f64, f64, f64) {
        let points = || {
            letters
                .pieces
                .iter()
                .flat_map(|piece| piece.contours.iter().flatten())
        };
        (
            points().map(|p| p.0).fold(f64::INFINITY, f64::min),
            points().map(|p| p.1).fold(f64::INFINITY, f64::min),
            points().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
            points().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
        )
    }

    #[test]
    fn level_text_is_set_in_its_rectangle() {
        let mut shape = art("Hop");
        shape.align = TextAlign::Left;
        let Some(level) = letters(&shape, 300.0, 100.0) else {
            // No outlines on this machine: nothing to check.
            return;
        };
        // One piece, with the outlines of H, o (two) and p (two).
        assert_eq!(level.pieces.len(), 1);
        assert_eq!(level.pieces[0].contours.len(), 5);
        let (x0, y0, x1, y1) = extent(&level);
        // It starts at the left margin and stays in the first line.
        assert!(
            x0 >= TEXT_MARGIN.0 as f64 && x0 < TEXT_MARGIN.0 as f64 + 12.0,
            "{x0}"
        );
        assert!(
            y0 > TEXT_MARGIN.1 as f64 && y1 < TEXT_MARGIN.1 as f64 + 48.0 * 1.2 + 1.0,
            "{y0} {y1}"
        );
        assert!(x1 - x0 > 50.0 && x1 - x0 < 110.0);
        // The box of the text is one line of 1.2 times the size.
        assert_eq!(level.bounds.3 - level.bounds.1, 58.0);
        // Right-aligned, it ends at the right margin.
        shape.align = TextAlign::Right;
        let right = extent(&letters(&shape, 300.0, 100.0).unwrap());
        assert!(
            (right.2 - (300.0 - TEXT_MARGIN.0 as f64)).abs() < 12.0,
            "{}",
            right.2
        );
        // Blank text, or a font without outlines, gives nothing.
        assert!(letters(&art("  "), 300.0, 100.0).is_none());
        assert!(
            Letters::with_fonts(
                &art("Hop"),
                300.0,
                100.0,
                1.0,
                &|name| name.to_string(),
                &FontDb::standard_only()
            )
            .is_none()
        );
    }

    #[test]
    fn warped_text_fills_its_shape() {
        let mut shape = art("Warped text");
        if letters(&shape, 400.0, 200.0).is_none() {
            return;
        }
        let level = extent(&letters(&shape, 400.0, 200.0).unwrap());
        // Plain: stretched over the whole text rectangle, whatever the
        // size of the letters.
        shape.warp = Some(TextWarp::from(WarpPreset::Plain));
        let plain = extent(&letters(&shape, 400.0, 200.0).unwrap());
        assert!(plain.0 < 20.0 && plain.2 > 380.0, "{plain:?}");
        assert!(plain.3 - plain.1 > 2.0 * (level.3 - level.1));
        // Triangle: letters in the middle are taller than those at the ends.
        shape.warp = Some(TextWarp::from(WarpPreset::Triangle));
        let triangle = letters(&shape, 400.0, 200.0).unwrap();
        let top_near = |x: f64| {
            triangle.pieces[0]
                .contours
                .iter()
                .flatten()
                .filter(|p| (p.0 - x).abs() < 30.0)
                .map(|p| p.1)
                .fold(f64::INFINITY, f64::min)
        };
        assert!(
            top_near(200.0) + 20.0 < top_near(30.0),
            "{} {}",
            top_near(200.0),
            top_near(30.0)
        );
        // Straight strokes are cut small enough to bend: no piece of a
        // path is longer than a fraction of the letters.
        let longest = triangle.pieces[0]
            .contours
            .iter()
            .flat_map(|contour| {
                contour
                    .windows(2)
                    .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
            })
            .fold(0.0, f64::max);
        assert!(longest < 30.0, "{longest}");
        // Arch: the letters stand on the upper half of an ellipse, their
        // feet on it and their heads outside.
        shape.warp = Some(TextWarp::from(WarpPreset::ArchUp));
        let arch = letters(&shape, 400.0, 400.0).unwrap();
        let (cx, cy) = (200.0, 200.0);
        let radius = (400.0 - 2.0 * TEXT_MARGIN.0 as f64) / 2.0;
        let nearest = arch.pieces[0]
            .contours
            .iter()
            .flatten()
            .map(|p| (p.0 - cx).hypot(p.1 - cy))
            .fold(f64::INFINITY, f64::min);
        assert!(
            nearest > radius - 16.0 && nearest < radius + 4.0,
            "{nearest} {radius}"
        );
        assert!(
            arch.pieces[0]
                .contours
                .iter()
                .flatten()
                .all(|p| p.1 < cy + 8.0)
        );
    }

    /// Every preset drawn in a frame, with its guide curves, as one
    /// image to look at: written where `OPENSHEET_SAMPLES` says.
    #[test]
    fn every_preset_is_drawn() {
        let dir = std::env::var_os("OPENSHEET_SAMPLES");
        let (cell_w, cell_h, columns) = (300.0, 170.0, 4);
        let mut sheet = String::new();
        for (i, preset) in WarpPreset::ALL.into_iter().enumerate() {
            let mut shape = art(preset.name());
            shape.align = TextAlign::Center;
            shape.effects.shadow = None;
            let warp = TextWarp::from(preset);
            shape.warp = Some(warp);
            let lines = match warp.geometry(100.0, 100.0).places() {
                1 => 1,
                places => places,
            };
            if lines > 1 {
                shape.text = (0..lines)
                    .map(|line| {
                        if line == 0 {
                            preset.name().to_string()
                        } else {
                            format!("line {}", line + 1)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            let (w, h) = (260.0, 130.0);
            let Some(shown) = letters(&shape, w, h) else {
                return;
            };
            // Nothing is lost and nothing flies away: every point is
            // finite and near the frame.
            let (x0, y0, x1, y1) = extent(&shown);
            assert!(x0.is_finite() && y1.is_finite(), "{preset:?}");
            assert!(
                x0 > -w && x1 < 2.0 * w && y0 > -2.0 * h && y1 < 3.0 * h,
                "{preset:?}: {x0} {y0} {x1} {y1}"
            );
            assert!(
                shown.pieces[0].contours.len()
                    >= preset
                        .name()
                        .chars()
                        .filter(|ch| !ch.is_whitespace())
                        .count(),
                "{preset:?}"
            );
            let (document, at) = shown.svg(0.0, (w / 2.0, h / 2.0), 1.0);
            let (ox, oy) = (
                20.0 + cell_w * (i % columns) as f64,
                20.0 + cell_h * (i / columns) as f64,
            );
            let _ = write!(
                sheet,
                r##"<rect x="{ox}" y="{oy}" width="{w}" height="{h}" fill="none" stroke="#BBBBBB"/>"##
            );
            let geometry = warp.geometry(
                w - 2.0 * TEXT_MARGIN.0 as f64,
                h - 2.0 * TEXT_MARGIN.1 as f64,
            );
            for curve in &geometry.curves {
                let points: Vec<String> = curve
                    .points
                    .iter()
                    .map(|p| {
                        format!(
                            "{:.1},{:.1}",
                            ox + TEXT_MARGIN.0 as f64 + p.0,
                            oy + TEXT_MARGIN.1 as f64 + p.1
                        )
                    })
                    .collect();
                let _ = write!(
                    sheet,
                    r##"<polyline points="{}" fill="none" stroke="#E0A0A0" stroke-width="0.6"/>"##,
                    points.join(" ")
                );
            }
            let inner = document.replacen(
                "<svg ",
                &format!(r#"<svg x="{:.1}" y="{:.1}" "#, ox + at.0, oy + at.1),
                1,
            );
            sheet.push_str(&inner);
            let _ = write!(
                sheet,
                r##"<text x="{ox}" y="{:.1}" font-family="Helvetica" font-size="10" fill="#666666">{}</text>"##,
                oy + h + 12.0,
                preset.ooxml_name()
            );
        }
        if let Some(dir) = dir {
            // Two images of five rows each: small enough to look at.
            let (width, height) = (40.0 + cell_w * columns as f64, 20.0 + cell_h * 5.0);
            for half in 0..2 {
                let document = format!(
                    r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 {} {width} {height}"><rect y="{0}" width="{width}" height="{height}" fill="white"/>{sheet}</svg>"##,
                    cell_h * 5.0 * half as f64
                );
                let name = format!("opensheet-warp-gallery-{}.svg", half + 1);
                std::fs::write(std::path::Path::new(&dir).join(name), document)
                    .expect("the sample is written");
            }
        }
    }

    #[test]
    fn runs_of_different_sizes_share_a_baseline() {
        let mut shape = ShapeTool::TextBox.shape();
        shape.font.set_name("Arial");
        shape.font.set_size_pt(12.0);
        shape.text = "HH".to_string();
        shape.format_text(1..2, &|font, _| font.set_size_pt(36.0));
        let Some(mixed) = letters(&shape, 300.0, 100.0) else {
            return;
        };
        assert_eq!(mixed.pieces.len(), 2);
        let box_of = |piece: &Piece| {
            let ys = || piece.contours.iter().flatten().map(|p| p.1);
            (
                ys().fold(f64::INFINITY, f64::min),
                ys().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        let (small, large) = (box_of(&mixed.pieces[0]), box_of(&mixed.pieces[1]));
        // Both stand on the same line; the large one is three times as tall.
        assert!((small.1 - large.1).abs() < 0.5, "{small:?} {large:?}");
        assert!(((large.1 - large.0) / (small.1 - small.0) - 3.0).abs() < 0.1);
    }

    #[test]
    fn long_lines_are_broken_at_blanks_when_the_shape_wraps() {
        let mut shape = ShapeTool::Rectangle.shape();
        shape.font.set_name("Arial");
        shape.font.set_size_pt(20.0);
        shape.text = "one two three four five six".to_string();
        shape.anchor = TextAnchor::Top;
        let Some(wide) = letters(&shape, 1000.0, 200.0) else {
            return;
        };
        let narrow = letters(&shape, 160.0, 200.0).unwrap();
        // The same letters, on more lines.
        assert_eq!(
            wide.pieces[0].contours.len(),
            narrow.pieces[0].contours.len()
        );
        assert!(narrow.bounds.3 - narrow.bounds.1 >= 2.0 * (wide.bounds.3 - wide.bounds.1));
        let (x0, _, x1, _) = extent(&narrow);
        assert!(x0 >= 0.0 && x1 <= 160.0, "{x0} {x1}");
        // Not wrapped, it stays on one line.
        shape.wrap = false;
        let one = letters(&shape, 160.0, 200.0).unwrap();
        assert_eq!(one.bounds.3 - one.bounds.1, wide.bounds.3 - wide.bounds.1);
    }

    #[test]
    fn the_image_draws_gradients_shadows_and_reflections() {
        let mut shape = art("Shine");
        shape.effects.gradient = Some(TextGradient::between(
            Color(0xFFC000),
            Color(0xC00000),
            GradientKind::Linear(90.0),
        ));
        shape.effects.reflection = Some(TextReflection::PRESETS[1]);
        let Some(shown) = letters(&shape, 300.0, 100.0) else {
            return;
        };
        let (document, (left, top, width, height)) = shown.svg(0.0, (150.0, 50.0), 2.0);
        // The gradient runs down the box of the text.
        assert!(
            document.contains(r##"<linearGradient id="g0" gradientUnits="userSpaceOnUse""##),
            "{document}"
        );
        assert!(document.contains(r##"<stop offset="0.0000" stop-color="#FFC000"/><stop offset="1.0000" stop-color="#C00000"/>"##));
        assert_eq!(document.matches(r#"fill="url(#g0)""#).count(), 2);
        // The shadow of the style lies under the reflection, which lies
        // under the letters.
        let (shadow, mirror) = (
            document.find(r#"filter="url(#s0)""#).unwrap(),
            document.find(r#"<g mask="url(#m0)""#).unwrap(),
        );
        assert!(shadow < mirror && document.contains("scale(1 -1)"));
        // The image has room for the reflection under the text.
        assert!(left < shown.bounds.0 && top < shown.bounds.1 && left + width > shown.bounds.2);
        assert!(top + height > shown.bounds.3 + 0.5 * (shown.bounds.3 - shown.bounds.1));
        // Turned, it holds the box whichever way it lies.
        let (turned, (_, _, w, h)) = shown.svg(90.0, (150.0, 50.0), 2.0);
        assert!(turned.contains(r#"<g transform="rotate(90.00 150.00 50.00)">"#));
        assert!(h > w);
        // From the centre: a radial paint.
        shape.effects.gradient = Some(TextGradient::between(
            Color(0xFFFFFF),
            Color(0x0070C0),
            GradientKind::FromCentre,
        ));
        let (document, _) = letters(&shape, 300.0, 100.0)
            .unwrap()
            .svg(0.0, (150.0, 50.0), 1.0);
        assert!(document.contains("<radialGradient id=\"g0\""));
        assert_eq!(
            gradient_line(GradientKind::Linear(0.0), (0.0, 0.0, 100.0, 40.0)),
            ((0.0, 20.0), (100.0, 20.0))
        );
        let down = gradient_line(GradientKind::Linear(90.0), (0.0, 0.0, 100.0, 40.0));
        assert!((down.0.1 - 0.0).abs() < 1e-9 && (down.1.1 - 40.0).abs() < 1e-9);
    }
}
