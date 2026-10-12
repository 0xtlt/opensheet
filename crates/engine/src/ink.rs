//! Ink: strokes drawn by hand on a sheet with the pens of the Draw tab
//! (`Sheet::inks`, in drawing order: the last one is on top).
//!
//! A stroke floats over the cells like a picture: a cell holds its
//! top-left corner, it has an offset inside that cell and a size, in
//! pixels at 100 %. It moves with its cell and keeps its size. Its points
//! are kept as they were drawn, relative to the corner of its frame; a
//! stroke that was resized is drawn stretched from that natural size
//! (`Ink::extent`) to its frame.
//!
//! What the hand draws is cleaned here too, so that it can be tested
//! without a screen: [`simplify`] drops the points a stroke does not need,
//! [`curve`] rounds it for drawing, and [`recognise`] tells a rectangle,
//! an ellipse, a triangle, a line or an arrow from a stroke (Ink to
//! Shape).

use crate::ast::StructuralEdit;
use crate::drawing::{Corner, Frame, ZOrder, columns, moved_corner, rows};
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS};
use crate::style::Color;
use crate::validation::localized;
use crate::workbook::{Sheet, Workbook};
use std::sync::Arc;

/// What a stroke was drawn with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PenKind {
    #[default]
    Pen,
    /// Drawn a little lighter than a pen, like graphite.
    Pencil,
    /// Wide and translucent: what is under it shows through.
    Highlighter,
}

impl PenKind {
    pub const ALL: [PenKind; 3] = [PenKind::Pen, PenKind::Pencil, PenKind::Highlighter];

    /// How opaque the strokes of this tool are (0 to 1).
    pub fn opacity(self) -> f32 {
        match self {
            PenKind::Pen => 1.0,
            PenKind::Pencil => 0.8,
            PenKind::Highlighter => 0.5,
        }
    }

    /// The name used in files of this application's own making.
    pub fn file_name(self) -> &'static str {
        match self {
            PenKind::Pen => "pen",
            PenKind::Pencil => "pencil",
            PenKind::Highlighter => "highlighter",
        }
    }

    pub fn from_file_name(name: &str) -> Option<PenKind> {
        PenKind::ALL.into_iter().find(|kind| kind.file_name() == name)
    }
}

/// A point of a stroke: pixels at 100 % (x, y).
pub type Pt = (f32, f32);

/// One stroke.
#[derive(Clone, Debug, PartialEq)]
pub struct Ink {
    /// Unique among the strokes of the workbook (assigned by
    /// `Workbook::add_ink`).
    pub id: u32,
    /// "Ink 1": unique on the sheet.
    pub name: String,
    pub kind: PenKind,
    pub color: Color,
    /// Thickness of the line in pixels at 100 %.
    pub width: f32,
    /// The points, from the top-left corner of the frame at its natural
    /// size. Shared: the undo snapshots of a sheet do not copy them.
    pub points: Arc<[Pt]>,
    /// The size the stroke was drawn at. Never zero.
    pub extent: (f32, f32),
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell (x, y).
    pub offset: (f32, f32),
    /// Width and height of the frame now.
    pub size: (f32, f32),
    /// Cannot be changed while the sheet is protected without "Edit
    /// objects".
    pub locked: bool,
}

/// The smallest side a frame is given: a dot or a straight horizontal
/// line still has a box to be picked by.
pub const MIN_EXTENT: f32 = 1.0;

impl Ink {
    /// A stroke through points of the sheet (pixels at 100 % from the
    /// corner of A1), and the frame it takes there. `None` without points.
    /// The frame is that of the points: the thickness of the line hangs
    /// over it.
    pub fn through(kind: PenKind, color: Color, width: f32, sheet_points: &[(f64, f64)]) -> Option<(Ink, Frame)> {
        let first = *sheet_points.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
        for (x, y) in sheet_points {
            (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
        }
        let extent = (((x1 - x0) as f32).max(MIN_EXTENT), ((y1 - y0) as f32).max(MIN_EXTENT));
        let points: Vec<Pt> = sheet_points.iter().map(|(x, y)| ((x - x0) as f32, (y - y0) as f32)).collect();
        let ink = Ink {
            id: 0,
            name: String::new(),
            kind,
            color,
            width: width.max(0.25),
            points: points.into(),
            extent,
            anchor: (0, 0),
            offset: (x0 as f32, y0 as f32),
            size: extent,
            locked: true,
        };
        Some((ink, Frame::new(x0, y0, extent.0 as f64, extent.1 as f64)))
    }

    /// The points stretched to a frame of the given size.
    pub fn points_in(&self, size: (f32, f32)) -> Vec<Pt> {
        let (sx, sy) = (size.0 / self.extent.0.max(MIN_EXTENT), size.1 / self.extent.1.max(MIN_EXTENT));
        self.points.iter().map(|(x, y)| (x * sx, y * sy)).collect()
    }

    /// The thickness of the line in a frame of the given size: it grows
    /// with the stroke, as in Excel.
    pub fn width_in(&self, size: (f32, f32)) -> f32 {
        let (sx, sy) = (size.0 / self.extent.0.max(MIN_EXTENT), size.1 / self.extent.1.max(MIN_EXTENT));
        // A straight stroke has one side of no real size: the other says
        // how much it grew.
        let flat = (self.extent.0 <= MIN_EXTENT, self.extent.1 <= MIN_EXTENT);
        let scale = match flat {
            (true, false) => sy,
            (false, true) => sx,
            _ => (sx * sy).sqrt(),
        };
        (self.width * scale).max(0.25)
    }
}

// ---- geometry --------------------------------------------------------------

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let along = if length > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0) } else { 0.0 };
    distance(p, (a.0 + along * dx, a.1 + along * dy))
}

/// Distance from `p` to a line through `points` (infinite without points).
pub fn path_distance(p: (f64, f64), points: &[(f64, f64)]) -> f64 {
    match points {
        [] => f64::INFINITY,
        [only] => distance(p, *only),
        _ => points.windows(2).map(|w| segment_distance(p, w[0], w[1])).fold(f64::INFINITY, f64::min),
    }
}

/// True when `p` is inside the closed outline `polygon` (even-odd rule).
pub fn in_polygon(p: (f64, f64), polygon: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = match polygon.last() {
        Some(last) => *last,
        None => return false,
    };
    for current in polygon {
        if (current.1 > p.1) != (previous.1 > p.1) {
            let x = current.0 + (p.1 - current.1) / (previous.1 - current.1) * (previous.0 - current.0);
            if p.0 < x {
                inside = !inside;
            }
        }
        previous = *current;
    }
    inside
}

/// Length of the line through `points`.
fn path_length(points: &[(f64, f64)]) -> f64 {
    points.windows(2).map(|w| distance(w[0], w[1])).sum()
}

/// The points a stroke needs: those closer than `tolerance` to the line
/// through their neighbours are dropped (Ramer–Douglas–Peucker). The two
/// ends are kept.
pub fn simplify(points: &[(f64, f64)], tolerance: f64) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    // Spans still to look at, instead of recursion: a long stroke must not
    // run the stack out.
    let mut spans = vec![(0, points.len() - 1)];
    while let Some((first, last)) = spans.pop() {
        let mut farthest = (0.0, first);
        for i in first + 1..last {
            let d = segment_distance(points[i], points[first], points[last]);
            if d > farthest.0 {
                farthest = (d, i);
            }
        }
        if farthest.0 > tolerance {
            keep[farthest.1] = true;
            spans.push((first, farthest.1));
            spans.push((farthest.1, last));
        }
    }
    points.iter().zip(keep).filter_map(|(p, kept)| kept.then_some(*p)).collect()
}

/// What the mouse gave, cleaned for keeping: the jitter of the hand is
/// averaged out and the points the line does not need are dropped.
pub fn smooth(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    // Each inner point moves a little towards the middle of its
    // neighbours: enough to quiet the hand, not enough to dull a corner.
    let mut out = Vec::with_capacity(points.len());
    out.push(points[0]);
    for w in points.windows(3) {
        out.push(((w[0].0 + 6.0 * w[1].0 + w[2].0) / 8.0, (w[0].1 + 6.0 * w[1].1 + w[2].1) / 8.0));
    }
    out.push(points[points.len() - 1]);
    simplify(&out, 0.35)
}

/// A rounded line through `points` for drawing: `steps` points on each
/// span of a Catmull-Rom curve, which passes through every point.
pub fn curve(points: &[Pt], steps: usize) -> Vec<Pt> {
    if points.len() < 3 || steps < 2 {
        return points.to_vec();
    }
    let at = |i: isize| points[i.clamp(0, points.len() as isize - 1) as usize];
    let mut out = Vec::with_capacity((points.len() - 1) * steps + 1);
    for i in 0..points.len() as isize - 1 {
        let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
        for step in 0..steps {
            let t = step as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let blend = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            out.push((blend(p0.0, p1.0, p2.0, p3.0), blend(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out.push(points[points.len() - 1]);
    out
}

// ---- ink to shape ----------------------------------------------------------

/// What a stroke was meant to be (Draw ▸ Ink to Shape). Positions are
/// those of the stroke: points of the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Recognised {
    Line { from: (f64, f64), to: (f64, f64) },
    /// The arrowhead is at `to`.
    Arrow { from: (f64, f64), to: (f64, f64) },
    Rectangle(Frame),
    Ellipse(Frame),
    /// An isosceles triangle; `flip_v`: it points down.
    Triangle { frame: Frame, flip_v: bool },
}

fn bounds(points: &[(f64, f64)]) -> Frame {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (x, y) in points {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    Frame::new(x0, y0, x1 - x0, y1 - y0)
}

/// Area enclosed by the outline `points` (shoelace formula).
fn area(points: &[(f64, f64)]) -> f64 {
    let mut twice = 0.0;
    let mut previous = match points.last() {
        Some(last) => *last,
        None => return 0.0,
    };
    for current in points {
        twice += previous.0 * current.1 - current.0 * previous.1;
        previous = *current;
    }
    twice.abs() / 2.0
}

/// The shape a stroke looks like, if any. Rectangles, ellipses and
/// triangles are recognised upright, as Excel's own shapes are.
pub fn recognise(points: &[(f64, f64)]) -> Option<Recognised> {
    if points.len() < 2 {
        return None;
    }
    let frame = bounds(points);
    let diagonal = frame.width.hypot(frame.height);
    // Too small to be anything but a dot.
    if diagonal < 12.0 {
        return None;
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let length = path_length(points);
    let closed = distance(first, last) < (0.25 * diagonal).max(10.0) && length > 2.0 * diagonal;
    if !closed {
        return recognise_open(points, diagonal);
    }
    // A closed stroke: how much of its box it fills tells the three
    // shapes apart (a rectangle all of it, an ellipse π/4, a triangle
    // half), and its corners confirm.
    if frame.width < 8.0 || frame.height < 8.0 {
        return None;
    }
    let filled = area(points) / (frame.width * frame.height);
    let corners = corners_of(points, diagonal);
    if filled > 0.86 && corners.len() == 4 {
        return Some(Recognised::Rectangle(frame));
    }
    if (0.38..0.62).contains(&filled) && corners.len() == 3 {
        // Which way it points: one corner at one edge of the box, two at
        // the other.
        let middle = frame.y + frame.height / 2.0;
        let above = corners.iter().filter(|c| c.1 < middle).count();
        return Some(Recognised::Triangle {
            frame,
            flip_v: above == 2,
        });
    }
    if (0.66..0.90).contains(&filled) && roundness(points, &frame) < 0.14 {
        return Some(Recognised::Ellipse(frame));
    }
    None
}

/// The corners of a closed stroke: the points a rough outline keeps.
fn corners_of(points: &[(f64, f64)], diagonal: f64) -> Vec<(f64, f64)> {
    // Start the outline at the point farthest from the middle (a corner if
    // there is one), so that where the pen was put down does not count as
    // a corner.
    let frame = bounds(points);
    let centre = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
    let start = points
        .iter()
        .enumerate()
        .max_by(|a, b| distance(*a.1, centre).total_cmp(&distance(*b.1, centre)))
        .map_or(0, |(i, _)| i);
    let mut outline: Vec<(f64, f64)> = points[start..].iter().chain(&points[..start]).copied().collect();
    outline.push(outline[0]);
    let mut rough = simplify(&outline, 0.09 * diagonal);
    rough.pop();
    // Two corners almost on each other are one (where the stroke closes).
    let mut corners: Vec<(f64, f64)> = Vec::new();
    for corner in rough {
        if corners.iter().all(|c| distance(*c, corner) > 0.15 * diagonal) {
            corners.push(corner);
        }
    }
    corners
}

/// How far a closed stroke is from the ellipse of its box: the mean
/// difference between 1 and the distance of its points from the centre,
/// measured in radii.
fn roundness(points: &[(f64, f64)], frame: &Frame) -> f64 {
    let (rx, ry) = (frame.width / 2.0, frame.height / 2.0);
    let (cx, cy) = (frame.x + rx, frame.y + ry);
    let total: f64 = points
        .iter()
        .map(|(x, y)| (((x - cx) / rx).hypot((y - cy) / ry) - 1.0).abs())
        .sum();
    total / points.len() as f64
}

/// An open stroke is a line, or an arrow: a line whose pen came back from
/// its end to draw the head.
fn recognise_open(points: &[(f64, f64)], diagonal: f64) -> Option<Recognised> {
    let (first, last) = (points[0], points[points.len() - 1]);
    let chord = distance(first, last);
    let straight = points.iter().map(|p| segment_distance(*p, first, last)).fold(0.0, f64::max);
    if chord > 0.0 && straight < (0.07 * chord).max(3.0) && path_length(points) < 1.2 * chord {
        return Some(Recognised::Line { from: first, to: last });
    }
    // The shaft is the first long straight run; everything after its end
    // stays close to that end.
    let rough = simplify(points, 0.05 * diagonal);
    let tip = *rough.get(1)?;
    let shaft = distance(first, tip);
    if rough.len() < 3 || shaft < 0.6 * diagonal {
        return None;
    }
    let head = rough[2..].iter().map(|p| distance(*p, tip)).fold(0.0, f64::max);
    (head < 0.45 * shaft).then_some(Recognised::Arrow { from: first, to: tip })
}

// ---- on the sheet ----------------------------------------------------------

fn ink_name(locale: &Locale) -> &'static str {
    localized(locale, ["Ink", "Encre", "Entrada de lápiz", "Freihand", "Input penna"])
}

impl Sheet {
    /// Where a stroke is on the sheet now.
    pub fn ink_frame(&self, ink: &Ink) -> Frame {
        Frame {
            x: columns(self).position(ink.anchor.1, ink.offset.0),
            y: rows(self).position(ink.anchor.0, ink.offset.1),
            width: ink.size.0.max(0.0) as f64,
            height: ink.size.1.max(0.0) as f64,
        }
    }

    /// The two corners of a stroke as a file writes them.
    pub fn ink_corners(&self, ink: &Ink) -> (Corner, Corner) {
        let frame = self.ink_frame(ink);
        (self.corner_at(frame.x, frame.y), self.corner_at(frame.x + frame.width, frame.y + frame.height))
    }

    /// The points of a stroke on the sheet (pixels at 100 % from the
    /// corner of A1), as it is drawn now.
    pub fn ink_points(&self, ink: &Ink) -> Vec<(f64, f64)> {
        let frame = self.ink_frame(ink);
        ink.points_in((frame.width as f32, frame.height as f32))
            .into_iter()
            .map(|(x, y)| (frame.x + x as f64, frame.y + y as f64))
            .collect()
    }

    /// Put a stroke at a frame of the sheet: its corner goes to the cell
    /// under it.
    fn settle_ink(&self, ink: &mut Ink, frame: Frame) {
        let clean = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
        (ink.anchor, ink.offset) = self.corner_at(clean(frame.x), clean(frame.y));
        ink.size = ((clean(frame.width) as f32).max(MIN_EXTENT), (clean(frame.height) as f32).max(MIN_EXTENT));
    }

    /// The strokes within `reach` pixels of a point of the sheet, topmost
    /// first: what the eraser removes and what a click selects.
    pub fn inks_at(&self, x: f64, y: f64, reach: f64) -> Vec<u32> {
        self.inks
            .iter()
            .rev()
            .filter(|ink| {
                let frame = self.ink_frame(ink);
                let room = reach + ink.width_in((frame.width as f32, frame.height as f32)) as f64 / 2.0;
                if x < frame.x - room || y < frame.y - room || x > frame.x + frame.width + room || y > frame.y + frame.height + room {
                    return false;
                }
                path_distance((x, y), &self.ink_points(ink)) <= room
            })
            .map(|ink| ink.id)
            .collect()
    }

    /// The strokes a lasso encloses, bottom-most first: those with every
    /// point inside the outline `lasso` (points of the sheet).
    pub fn inks_inside(&self, lasso: &[(f64, f64)]) -> Vec<u32> {
        if lasso.len() < 3 {
            return Vec::new();
        }
        self.inks
            .iter()
            .filter(|ink| self.ink_points(ink).iter().all(|p| in_polygon(*p, lasso)))
            .map(|ink| ink.id)
            .collect()
    }

    fn ink_title(&self, base: &str) -> String {
        let mut number = self.inks.len() + 1;
        loop {
            let name = format!("{base} {number}");
            if !self.inks.iter().any(|ink| ink.name.eq_ignore_ascii_case(&name)) {
                return name;
            }
            number += 1;
        }
    }
}

impl Workbook {
    fn next_ink_id(&self) -> u32 {
        self.sheets.iter().flat_map(|s| s.inks.iter().map(|ink| ink.id)).max().unwrap_or(0) + 1
    }

    /// Add a stroke on top of the others, at `frame` on the sheet; returns
    /// its id. It gets a name when it has none. Recorded for undo, like
    /// every function below that changes a stroke.
    pub fn add_ink(&mut self, sheet: usize, mut ink: Ink, frame: Frame) -> Option<u32> {
        if sheet >= self.sheets.len() || ink.points.is_empty() {
            return None;
        }
        self.snapshot();
        ink.id = self.next_ink_id();
        let sh = &mut self.sheets[sheet];
        if ink.name.is_empty() {
            ink.name = sh.ink_title(ink_name(self.locale));
        }
        ink.extent = (ink.extent.0.max(MIN_EXTENT), ink.extent.1.max(MIN_EXTENT));
        sh.settle_ink(&mut ink, frame);
        let id = ink.id;
        sh.inks.push(ink);
        Some(id)
    }

    /// Draw a stroke through points of the sheet with a pen.
    pub fn draw_ink(&mut self, sheet: usize, kind: PenKind, color: Color, width: f32, points: &[(f64, f64)]) -> Option<u32> {
        let (ink, frame) = Ink::through(kind, color, width, points)?;
        self.add_ink(sheet, ink, frame)
    }

    pub fn ink(&self, sheet: usize, id: u32) -> Option<&Ink> {
        self.sheets.get(sheet)?.inks.iter().find(|ink| ink.id == id)
    }

    /// Where a stroke is on its sheet.
    pub fn ink_frame(&self, sheet: usize, id: u32) -> Option<Frame> {
        let sh = self.sheets.get(sheet)?;
        sh.inks.iter().find(|ink| ink.id == id).map(|ink| sh.ink_frame(ink))
    }

    pub fn remove_ink(&mut self, sheet: usize, id: u32) -> bool {
        if self.ink(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].inks.retain(|ink| ink.id != id);
        true
    }

    /// Move and / or resize a stroke to a frame on the sheet.
    pub fn set_ink_frame(&mut self, sheet: usize, id: u32, frame: Frame) -> bool {
        let Some(index) = self.sheets.get(sheet).and_then(|sh| sh.inks.iter().position(|ink| ink.id == id)) else {
            return false;
        };
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let mut ink = sh.inks.remove(index);
        sh.settle_ink(&mut ink, frame);
        sh.inks.insert(index, ink);
        true
    }

    /// Change the look of a stroke (colour, thickness, tool, name).
    pub fn update_ink(&mut self, sheet: usize, id: u32, f: impl FnOnce(&mut Ink)) -> bool {
        if self.ink(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        if let Some(ink) = self.sheets[sheet].inks.iter_mut().find(|ink| ink.id == id) {
            f(ink);
            ink.id = id;
        }
        true
    }

    /// Move a stroke in the stacking order of the strokes. Returns false
    /// when it is already there.
    pub fn reorder_ink(&mut self, sheet: usize, id: u32, order: ZOrder) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let Some(from) = sh.inks.iter().position(|ink| ink.id == id) else {
            return false;
        };
        let top = sh.inks.len() - 1;
        let to = match order {
            ZOrder::BringToFront => top,
            ZOrder::SendToBack => 0,
            ZOrder::BringForward => (from + 1).min(top),
            ZOrder::SendBackward => from.saturating_sub(1),
        };
        if to == from {
            return false;
        }
        self.snapshot();
        let inks = &mut self.sheets[sheet].inks;
        let ink = inks.remove(from);
        inks.insert(to, ink);
        true
    }
}

/// See `features::on_structural_edit`: a stroke follows the cell holding
/// its corner.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.inks.is_empty() || !sheet.name.eq_ignore_ascii_case(edited_sheet) {
        return;
    }
    let (row_axis, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    for ink in &mut sheet.inks {
        if row_axis {
            (ink.anchor.0, ink.offset.1) = moved_corner(ink.anchor.0, ink.offset.1, false, insert, at, count, MAX_ROWS);
        } else {
            (ink.anchor.1, ink.offset.0) = moved_corner(ink.anchor.1, ink.offset.0, false, insert, at, count, MAX_COLS);
        }
    }
}

/// See `features::on_sheet_duplicated`: the strokes of the copy get ids of
/// their own (they share their points).
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    let next = wb.next_ink_id();
    for (ink, id) in wb.sheets[copy].inks.iter_mut().zip(next..) {
        ink.id = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb() -> Workbook {
        Workbook::new(Locale::en())
    }

    fn zigzag() -> Vec<(f64, f64)> {
        vec![(100.0, 50.0), (130.0, 80.0), (160.0, 50.0), (190.0, 80.0)]
    }

    #[test]
    fn a_stroke_is_placed_where_it_was_drawn() {
        let mut wb = wb();
        let id = wb.draw_ink(0, PenKind::Pen, Color(0xFF0000), 2.0, &zigzag()).unwrap();
        wb.commit("Ink");
        let ink = wb.ink(0, id).unwrap();
        assert_eq!(ink.name, "Ink 1");
        assert_eq!(ink.extent, (90.0, 30.0));
        assert_eq!(&ink.points[..2], &[(0.0, 0.0), (30.0, 30.0)]);
        let frame = wb.ink_frame(0, id).unwrap();
        assert_eq!((frame.x, frame.y, frame.width, frame.height), (100.0, 50.0, 90.0, 30.0));
        // The points on the sheet are those that were drawn.
        assert_eq!(wb.sheets[0].ink_points(ink), zigzag());
        // Undo takes it away, redo brings it back.
        assert!(wb.undo());
        assert!(wb.sheets[0].inks.is_empty());
        assert!(wb.redo());
        assert_eq!(wb.sheets[0].inks.len(), 1);
    }

    #[test]
    fn a_stroke_follows_its_cell_and_keeps_its_size() {
        let mut wb = wb();
        let id = wb.draw_ink(0, PenKind::Pen, Color::BLACK, 2.0, &zigzag()).unwrap();
        wb.commit("Ink");
        let before = wb.ink_frame(0, id).unwrap();
        let (row_h, col_w) = (wb.sheets[0].default_row_height as f64, wb.sheets[0].default_col_width as f64);
        wb.insert_rows(0, 0, 2);
        wb.insert_cols(0, 0, 1);
        let after = wb.ink_frame(0, id).unwrap();
        assert_eq!((after.x, after.y), (before.x + col_w, before.y + 2.0 * row_h));
        assert_eq!((after.width, after.height), (before.width, before.height));
        // Widening a column before it pushes it; widening one under it
        // does not stretch it.
        wb.snapshot();
        wb.sheets[0].col_widths.insert(0, col_w as f32 + 40.0);
        wb.commit("Column Width");
        let pushed = wb.ink_frame(0, id).unwrap();
        assert_eq!((pushed.x, pushed.width), (after.x + 40.0, after.width));
    }

    #[test]
    fn resizing_stretches_the_points_and_the_line() {
        let mut wb = wb();
        let id = wb.draw_ink(0, PenKind::Pen, Color::BLACK, 2.0, &zigzag()).unwrap();
        assert!(wb.set_ink_frame(0, id, Frame::new(10.0, 20.0, 180.0, 60.0)));
        wb.commit("Size Object");
        let ink = wb.ink(0, id).unwrap();
        let points = wb.sheets[0].ink_points(ink);
        assert_eq!(points[1], (70.0, 80.0));
        assert_eq!(ink.width_in(ink.size), 4.0);
        // The natural points are untouched.
        assert_eq!(ink.points[1], (30.0, 30.0));
    }

    #[test]
    fn the_eraser_and_the_lasso_find_strokes() {
        let mut wb = wb();
        let a = wb.draw_ink(0, PenKind::Pen, Color::BLACK, 2.0, &zigzag()).unwrap();
        let b = wb
            .draw_ink(0, PenKind::Highlighter, Color(0xFFFF00), 12.0, &[(100.0, 200.0), (200.0, 200.0)])
            .unwrap();
        let sheet = &wb.sheets[0];
        // On the first stroke, between two of its points.
        assert_eq!(sheet.inks_at(115.0, 65.0, 3.0), vec![a]);
        // In its box but away from the line.
        assert!(sheet.inks_at(130.0, 52.0, 3.0).is_empty());
        // A wide line is touched from farther away.
        assert_eq!(sheet.inks_at(150.0, 208.0, 3.0), vec![b]);
        // A lasso around the first one only.
        let lasso = [(90.0, 40.0), (200.0, 40.0), (200.0, 90.0), (90.0, 90.0)];
        assert_eq!(sheet.inks_inside(&lasso), vec![a]);
        // Cutting through a stroke does not select it.
        let half = [(90.0, 40.0), (150.0, 40.0), (150.0, 90.0), (90.0, 90.0)];
        assert!(sheet.inks_inside(&half).is_empty());
        let all = [(0.0, 0.0), (300.0, 0.0), (300.0, 300.0), (0.0, 300.0)];
        assert_eq!(sheet.inks_inside(&all), vec![a, b]);
    }

    #[test]
    fn strokes_are_restacked_and_copied_with_their_sheet() {
        let mut wb = wb();
        let a = wb.draw_ink(0, PenKind::Pen, Color::BLACK, 2.0, &zigzag()).unwrap();
        let b = wb.draw_ink(0, PenKind::Pencil, Color::BLACK, 1.0, &zigzag()).unwrap();
        assert!(wb.reorder_ink(0, a, ZOrder::BringToFront));
        assert!(!wb.reorder_ink(0, a, ZOrder::BringForward));
        let order: Vec<u32> = wb.sheets[0].inks.iter().map(|ink| ink.id).collect();
        assert_eq!(order, vec![b, a]);
        wb.commit("Change Order");
        wb.duplicate_sheet(0);
        let ids: Vec<u32> = wb.sheets[1].inks.iter().map(|ink| ink.id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| *id != a && *id != b));
        // The copies share the points of the originals.
        assert!(Arc::ptr_eq(&wb.sheets[0].inks[0].points, &wb.sheets[1].inks[0].points));
    }

    #[test]
    fn smoothing_keeps_the_ends_and_the_shape() {
        // A straight line with a hand's jitter becomes a few points.
        let shaky: Vec<(f64, f64)> = (0..100).map(|i| (i as f64, if i % 2 == 0 { 0.2 } else { -0.2 })).collect();
        let clean = smooth(&shaky);
        assert!(clean.len() <= 4, "{} points left", clean.len());
        assert_eq!((clean[0], clean[clean.len() - 1]), (shaky[0], shaky[99]));
        // A corner survives.
        let corner: Vec<(f64, f64)> =
            (0..=50).map(|i| (i as f64, 0.0)).chain((1..=50).map(|i| (50.0, i as f64))).collect();
        let kept = smooth(&corner);
        assert!(kept.iter().any(|p| distance(*p, (50.0, 0.0)) < 1.5));
        // The curve drawn through points passes through each of them.
        let through = curve(&[(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)], 4);
        assert_eq!(through.len(), 9);
        assert_eq!((through[0], through[4], through[8]), ((0.0, 0.0), (10.0, 10.0), (20.0, 0.0)));
    }

    /// Points along the outline through `corners`, a few pixels apart and
    /// a little shaky, as a hand draws them.
    fn drawn(corners: &[(f64, f64)]) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for w in corners.windows(2) {
            let steps = (distance(w[0], w[1]) / 4.0).ceil() as usize;
            for i in 0..steps {
                let t = i as f64 / steps as f64;
                let wobble = ((out.len() * 7) % 5) as f64 * 0.5 - 1.0;
                out.push((w[0].0 + (w[1].0 - w[0].0) * t + wobble, w[0].1 + (w[1].1 - w[0].1) * t - wobble));
            }
        }
        out.push(corners[corners.len() - 1]);
        out
    }

    #[test]
    fn ink_to_shape_tells_the_shapes_apart() {
        let rectangle = drawn(&[(100.0, 100.0), (300.0, 104.0), (297.0, 220.0), (102.0, 216.0), (101.0, 103.0)]);
        match recognise(&rectangle) {
            Some(Recognised::Rectangle(frame)) => {
                assert!((frame.x - 100.0).abs() < 4.0 && (frame.width - 200.0).abs() < 8.0, "{frame:?}")
            }
            other => panic!("rectangle seen as {other:?}"),
        }
        let ellipse: Vec<(f64, f64)> = (0..=60)
            .map(|i| {
                let a = i as f64 / 60.0 * std::f64::consts::TAU;
                (200.0 + 90.0 * a.cos() + (i % 3) as f64, 150.0 + 50.0 * a.sin())
            })
            .collect();
        assert!(matches!(recognise(&ellipse), Some(Recognised::Ellipse(_))), "{:?}", recognise(&ellipse));
        let triangle = drawn(&[(200.0, 100.0), (290.0, 240.0), (110.0, 238.0), (198.0, 102.0)]);
        assert!(matches!(recognise(&triangle), Some(Recognised::Triangle { flip_v: false, .. })), "{:?}", recognise(&triangle));
        let down = drawn(&[(110.0, 100.0), (290.0, 102.0), (200.0, 240.0), (112.0, 101.0)]);
        assert!(matches!(recognise(&down), Some(Recognised::Triangle { flip_v: true, .. })), "{:?}", recognise(&down));
        let line = drawn(&[(50.0, 50.0), (250.0, 120.0)]);
        match recognise(&line) {
            Some(Recognised::Line { from, to }) => assert_eq!((from, to), (line[0], (250.0, 120.0))),
            other => panic!("line seen as {other:?}"),
        }
        // A shaft, then the pen comes back to draw the two sides of the head.
        let arrow = drawn(&[(50.0, 100.0), (250.0, 100.0), (225.0, 85.0), (250.0, 100.0), (225.0, 115.0)]);
        match recognise(&arrow) {
            Some(Recognised::Arrow { from, to }) => {
                assert!(distance(from, (50.0, 100.0)) < 3.0 && distance(to, (250.0, 100.0)) < 6.0, "{from:?} {to:?}")
            }
            other => panic!("arrow seen as {other:?}"),
        }
        // A scribble and a dot are left as ink.
        let scribble = drawn(&[(0.0, 0.0), (80.0, 60.0), (10.0, 70.0), (90.0, 5.0), (40.0, 90.0), (120.0, 40.0)]);
        assert_eq!(recognise(&scribble), None);
        assert_eq!(recognise(&[(5.0, 5.0), (6.0, 6.0)]), None);
    }
}
