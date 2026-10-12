//! Where a picture or a shape is on screen and what its outline looks
//! like: flips and rotation, the outlines of the presets as polylines,
//! arrowheads, the handles and what dragging them does. Plain numbers
//! only, so that all of it is tested without a window.

use opensheet_engine::drawing::{Frame, LineEnd, PathCommand};

pub type Pt = (f32, f32);

/// The smallest side a handle can give a frame, in pixels at 100 %.
pub const MIN_SIDE: f64 = 2.0;

/// Turn a point about a centre, clockwise on screen (y runs down).
pub fn rotate(p: Pt, centre: Pt, degrees: f32) -> Pt {
    if degrees == 0.0 {
        return p;
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (dx, dy) = (p.0 - centre.0, p.1 - centre.1);
    (centre.0 + dx * cos - dy * sin, centre.1 + dx * sin + dy * cos)
}

/// An angle brought back between 0 (included) and 360 degrees.
pub fn normalise_angle(degrees: f32) -> f32 {
    let turned = degrees.rem_euclid(360.0);
    if turned >= 360.0 || !turned.is_finite() { 0.0 } else { turned }
}

/// A frame as it is drawn: its box before rotation, the rotation in
/// degrees clockwise about its centre, and the flips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub rotation: f32,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Placed {
    /// A frame that is neither turned nor flipped.
    pub fn plain(x0: f32, y0: f32, x1: f32, y1: f32) -> Placed {
        Placed {
            x0,
            y0,
            x1,
            y1,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
        }
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn centre(&self) -> Pt {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }

    /// Where a point of the frame, given as fractions of its width and
    /// height, is drawn: flipped, then turned.
    pub fn point(&self, fx: f32, fy: f32) -> Pt {
        let fx = if self.flip_h { 1.0 - fx } else { fx };
        let fy = if self.flip_v { 1.0 - fy } else { fy };
        rotate(
            (self.x0 + fx * self.width(), self.y0 + fy * self.height()),
            self.centre(),
            self.rotation,
        )
    }

    /// A point of the screen as the frame sees it before it is turned.
    pub fn unturned(&self, p: Pt) -> Pt {
        rotate(p, self.centre(), -self.rotation)
    }

    /// True for a point inside the turned box.
    pub fn contains(&self, p: Pt) -> bool {
        let (x, y) = self.unturned(p);
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    /// The corners of the turned box, clockwise from the one that was at
    /// the top left.
    pub fn corners(&self) -> [Pt; 4] {
        let c = self.centre();
        [(self.x0, self.y0), (self.x1, self.y0), (self.x1, self.y1), (self.x0, self.y1)]
            .map(|p| rotate(p, c, self.rotation))
    }

    /// The eight handles: corners and edge midpoints, clockwise from the
    /// top-left corner (the numbering of `objects.rs`).
    pub fn handles(&self) -> [Pt; 8] {
        let (mx, my) = self.centre();
        [
            (self.x0, self.y0),
            (mx, self.y0),
            (self.x1, self.y0),
            (self.x1, my),
            (self.x1, self.y1),
            (mx, self.y1),
            (self.x0, self.y1),
            (self.x0, my),
        ]
        .map(|p| rotate(p, (mx, my), self.rotation))
    }

    /// The handle that turns the frame: `gap` pixels above the middle of
    /// its top edge.
    pub fn rotation_handle(&self, gap: f32) -> Pt {
        let (mx, _) = self.centre();
        rotate((mx, self.y0 - gap), self.centre(), self.rotation)
    }

    /// The upright box (x0, y0, x1, y1) around the turned frame.
    pub fn extent(&self) -> (f32, f32, f32, f32) {
        let corners = self.corners();
        let xs = corners.map(|p| p.0);
        let ys = corners.map(|p| p.1);
        (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        )
    }
}

/// One stroke of an outline: its points, and whether it comes back to its
/// first one.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub points: Vec<Pt>,
    pub closed: bool,
}

/// The outline of a preset (`ShapePreset::path`) as it is drawn in a
/// frame: curves become short straight runs, fine enough for the size of
/// the frame.
pub fn flatten(commands: &[PathCommand], placed: &Placed) -> Vec<Polyline> {
    let steps = (((placed.width().abs() + placed.height().abs()) / 10.0).ceil() as usize).clamp(8, 40);
    let mut lines: Vec<Polyline> = Vec::new();
    // In fractions of the frame, like the commands.
    let mut current = (0.0f32, 0.0f32);
    let mut points: Vec<Pt> = Vec::new();
    let mut finish = |points: &mut Vec<Pt>, closed: bool| {
        if points.len() >= 2 {
            lines.push(Polyline {
                points: std::mem::take(points),
                closed,
            });
        } else {
            points.clear();
        }
    };
    for command in commands {
        match *command {
            PathCommand::MoveTo(x, y) => {
                finish(&mut points, false);
                current = (x, y);
                points.push(placed.point(x, y));
            }
            PathCommand::LineTo(x, y) => {
                current = (x, y);
                points.push(placed.point(x, y));
            }
            PathCommand::CurveTo(ax, ay, bx, by, x, y) => {
                let (sx, sy) = current;
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let u = 1.0 - t;
                    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    points.push(placed.point(
                        w0 * sx + w1 * ax + w2 * bx + w3 * x,
                        w0 * sy + w1 * ay + w2 * by + w3 * y,
                    ));
                }
                current = (x, y);
            }
            PathCommand::Close => finish(&mut points, true),
        }
    }
    finish(&mut points, false);
    lines
}

/// True for a point inside a closed outline (a ray to the right crosses
/// it an odd number of times).
pub fn in_polygon(p: Pt, points: &[Pt]) -> bool {
    let mut inside = false;
    let mut previous = match points.last() {
        Some(last) => *last,
        None => return false,
    };
    for point in points {
        let (a, b) = (previous, *point);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            inside = !inside;
        }
        previous = b;
    }
    inside
}

fn distance_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (lx, ly) = (b.0 - a.0, b.1 - a.1);
    let length = lx * lx + ly * ly;
    let along = if length > 0.0 {
        (((p.0 - a.0) * lx + (p.1 - a.1) * ly) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - along * lx).hypot(p.1 - a.1 - along * ly)
}

/// How far a point is from a stroke.
pub fn distance_to(p: Pt, line: &Polyline) -> f32 {
    let mut best = f32::INFINITY;
    for pair in line.points.windows(2) {
        best = best.min(distance_to_segment(p, pair[0], pair[1]));
    }
    if let (true, Some(first), Some(last)) = (line.closed, line.points.first(), line.points.last()) {
        best = best.min(distance_to_segment(p, *last, *first));
    }
    best
}

/// What a line ends with: an outline to fill or two strokes to draw, and
/// how much of the line the head replaces.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrowHead {
    pub points: Vec<Pt>,
    /// Filled when true, else drawn as an open stroke.
    pub filled: bool,
    /// The line stops this far before its end, under the head.
    pub cut: f32,
}

/// The head at the end `tip` of a line coming from `from`; `size` is its
/// length and its width.
pub fn arrow_head(kind: LineEnd, tip: Pt, from: Pt, size: f32) -> Option<ArrowHead> {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let length = dx.hypot(dy);
    if length <= 0.0 || kind == LineEnd::None {
        return None;
    }
    // Along the line and across it.
    let (ux, uy) = (dx / length, dy / length);
    let (nx, ny) = (-uy, ux);
    let at = |along: f32, across: f32| (tip.0 + ux * along + nx * across, tip.1 + uy * along + ny * across);
    let half = size / 2.0;
    let head = match kind {
        LineEnd::None => return None,
        LineEnd::Triangle => ArrowHead {
            points: vec![at(0.0, 0.0), at(-size, half), at(-size, -half)],
            filled: true,
            cut: size * 0.8,
        },
        LineEnd::Stealth => ArrowHead {
            points: vec![at(0.0, 0.0), at(-size, half), at(-size * 0.6, 0.0), at(-size, -half)],
            filled: true,
            cut: size * 0.5,
        },
        LineEnd::Diamond => ArrowHead {
            points: vec![at(half, 0.0), at(0.0, half), at(-half, 0.0), at(0.0, -half)],
            filled: true,
            cut: 0.0,
        },
        LineEnd::Oval => ArrowHead {
            points: (0..16)
                .map(|i| {
                    let angle = i as f32 / 16.0 * std::f32::consts::TAU;
                    at(half * angle.cos(), half * angle.sin())
                })
                .collect(),
            filled: true,
            cut: 0.0,
        },
        LineEnd::Open => ArrowHead {
            points: vec![at(-size, half), at(0.0, 0.0), at(-size, -half)],
            filled: false,
            cut: 0.0,
        },
    };
    Some(head)
}

/// Shorten a stroke by `cut` at its end (`at_end`) or at its start, where
/// an arrowhead takes over.
pub fn trim(points: &mut Vec<Pt>, cut: f32, at_end: bool) {
    if cut <= 0.0 || points.len() < 2 {
        return;
    }
    if !at_end {
        points.reverse();
    }
    let mut left = cut;
    while points.len() >= 2 {
        let (tip, before) = (points[points.len() - 1], points[points.len() - 2]);
        let length = (tip.0 - before.0).hypot(tip.1 - before.1);
        // The line never disappears under its head.
        if length > left || points.len() == 2 {
            let keep = ((length - left) / length.max(f32::EPSILON)).clamp(0.05, 1.0);
            let last = points.len() - 1;
            points[last] = (before.0 + (tip.0 - before.0) * keep, before.1 + (tip.1 - before.1) * keep);
            break;
        }
        left -= length;
        points.pop();
    }
    if !at_end {
        points.reverse();
    }
}

/// Which way a resize handle pulls, given the rotation of its frame: 0
/// left-right, 1 up-left / down-right, 2 up-down, 3 up-right / down-left.
pub fn handle_axis(handle: u8, rotation: f32) -> u8 {
    // Clockwise from the right-hand side.
    let base = match handle % 8 {
        3 => 0.0,
        4 => 45.0,
        5 => 90.0,
        6 => 135.0,
        7 => 180.0,
        0 => 225.0,
        1 => 270.0,
        _ => 315.0,
    };
    ((normalise_angle(base + rotation) / 45.0).round() as u8) % 4
}

/// The frame after one of its handles (numbered clockwise from the
/// top-left corner) was dragged by `delta` sheet pixels. The opposite side
/// stays where it is, also when the frame is turned; `from_centre` grows
/// both sides instead, and `keep_aspect` keeps the proportions when a
/// corner is dragged.
pub fn resize(frame: Frame, rotation: f32, handle: u8, delta: (f64, f64), keep_aspect: bool, from_centre: bool) -> Frame {
    let (sin, cos) = (rotation as f64).to_radians().sin_cos();
    // The movement as the frame sees it before it is turned.
    let (dx, dy) = (delta.0 * cos + delta.1 * sin, -delta.0 * sin + delta.1 * cos);
    // The sides the handle moves: -1 the left / top one, 1 the right /
    // bottom one.
    let (hx, hy): (f64, f64) = match handle % 8 {
        0 => (-1.0, -1.0),
        1 => (0.0, -1.0),
        2 => (1.0, -1.0),
        3 => (1.0, 0.0),
        4 => (1.0, 1.0),
        5 => (0.0, 1.0),
        6 => (-1.0, 1.0),
        _ => (-1.0, 0.0),
    };
    let both = if from_centre { 2.0 } else { 1.0 };
    let mut width = (frame.width + hx * dx * both).max(if hx != 0.0 { MIN_SIDE.min(frame.width) } else { 0.0 });
    let mut height = (frame.height + hy * dy * both).max(if hy != 0.0 { MIN_SIDE.min(frame.height) } else { 0.0 });
    if keep_aspect && hx != 0.0 && hy != 0.0 && frame.width > 0.0 && frame.height > 0.0 {
        // Follow the side that was pulled furthest.
        let scale = (width / frame.width).max(height / frame.height);
        width = frame.width * scale;
        height = frame.height * scale;
    }
    let (shift_x, shift_y) = if from_centre {
        (0.0, 0.0)
    } else {
        (hx * (width - frame.width) / 2.0, hy * (height - frame.height) / 2.0)
    };
    let cx = frame.x + frame.width / 2.0 + shift_x * cos - shift_y * sin;
    let cy = frame.y + frame.height / 2.0 + shift_x * sin + shift_y * cos;
    Frame::new(cx - width / 2.0, cy - height / 2.0, width, height)
}

/// The two ends of a line on the sheet: where its outline starts (the
/// top-left corner of its frame before flips) and where it ends.
pub fn line_ends(frame: Frame, flip_h: bool, flip_v: bool, rotation: f32) -> ((f64, f64), (f64, f64)) {
    let (x0, x1) = if flip_h { (frame.x + frame.width, frame.x) } else { (frame.x, frame.x + frame.width) };
    let (y0, y1) = if flip_v { (frame.y + frame.height, frame.y) } else { (frame.y, frame.y + frame.height) };
    let (cx, cy) = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
    let (sin, cos) = (rotation as f64).to_radians().sin_cos();
    let turn = |x: f64, y: f64| (cx + (x - cx) * cos - (y - cy) * sin, cy + (x - cx) * sin + (y - cy) * cos);
    (turn(x0, y0), turn(x1, y1))
}

/// The frame and the flips (horizontal, vertical) of a line drawn from
/// `start` to `end`, without rotation.
pub fn line_frame(start: (f64, f64), end: (f64, f64)) -> (Frame, bool, bool) {
    (
        Frame::new(start.0.min(end.0), start.1.min(end.1), (end.0 - start.0).abs(), (end.1 - start.1).abs()),
        end.0 < start.0,
        end.1 < start.1,
    )
}

/// Bring `to` onto the nearest direction from `from` that is a multiple of
/// `step` degrees (Shift while drawing a line), keeping its distance along
/// that direction.
pub fn constrain_direction(from: (f64, f64), to: (f64, f64), step: f64) -> (f64, f64) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if dx == 0.0 && dy == 0.0 {
        return to;
    }
    let angle = (dy.atan2(dx).to_degrees() / step).round() * step;
    let (sin, cos) = angle.to_radians().sin_cos();
    let length = dx * cos + dy * sin;
    (from.0 + length * cos, from.1 + length * sin)
}

/// The frame drawn by dragging from `start` to `end`; `square` gives it
/// equal sides (Shift), the longer of the two.
pub fn drawn_frame(start: (f64, f64), end: (f64, f64), square: bool) -> Frame {
    let (mut dx, mut dy) = (end.0 - start.0, end.1 - start.1);
    if square {
        let side = dx.abs().max(dy.abs());
        dx = if dx < 0.0 { -side } else { side };
        dy = if dy < 0.0 { -side } else { side };
    }
    Frame::new(start.0.min(start.0 + dx), start.1.min(start.1 + dy), dx.abs(), dy.abs())
}

/// The rotation of a frame whose rotation handle was dragged to `at`:
/// straight above the centre is 0. `snap` rounds to multiples of 15
/// degrees (Shift); otherwise to whole degrees.
pub fn drag_angle(centre: Pt, at: Pt, snap: bool) -> f32 {
    let angle = (at.1 - centre.1).atan2(at.0 - centre.0).to_degrees() + 90.0;
    let step = if snap { 15.0 } else { 1.0 };
    normalise_angle((angle / step).round() * step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::ShapePreset;

    fn close(a: Pt, b: Pt) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn frames_turn_clockwise_about_their_centre() {
        let mut placed = Placed::plain(100.0, 50.0, 300.0, 150.0);
        assert_eq!(placed.point(0.0, 0.0), (100.0, 50.0));
        assert_eq!(placed.point(1.0, 0.5), (300.0, 100.0));
        placed.rotation = 90.0;
        // The top-left corner goes to the top right of the upright box.
        assert!(close(placed.point(0.0, 0.0), (250.0, 0.0)));
        assert!(close(placed.point(1.0, 1.0), (150.0, 200.0)));
        let extent = placed.extent();
        assert!(close((extent.0, extent.1), (150.0, 0.0)) && close((extent.2, extent.3), (250.0, 200.0)));
        assert!(placed.contains((200.0, 10.0)) && !placed.contains((110.0, 60.0)));
        assert!(close(placed.unturned((250.0, 0.0)), (100.0, 50.0)));
        // Flips come first: the mirrored point is then turned.
        placed.flip_h = true;
        assert!(close(placed.point(0.0, 0.0), (250.0, 200.0)));
        // The rotation handle of an upright frame is above its top edge.
        let upright = Placed::plain(0.0, 100.0, 80.0, 140.0);
        assert_eq!(upright.rotation_handle(20.0), (40.0, 80.0));
        assert_eq!(upright.handles()[4], (80.0, 140.0));
        assert_eq!(normalise_angle(-90.0), 270.0);
        assert_eq!(normalise_angle(720.0), 0.0);
    }

    #[test]
    fn outlines_become_polylines() {
        let placed = Placed::plain(0.0, 0.0, 200.0, 100.0);
        let rect = flatten(&ShapePreset::Rect.path(200.0, 100.0), &placed);
        assert_eq!(rect.len(), 1);
        assert!(rect[0].closed);
        assert_eq!(rect[0].points, vec![(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (0.0, 100.0)]);
        // A line is one open stroke, and flipping gives the other diagonal.
        let line = flatten(&ShapePreset::Line.path(200.0, 100.0), &placed);
        assert_eq!((line[0].closed, line[0].points.clone()), (false, vec![(0.0, 0.0), (200.0, 100.0)]));
        let flipped = Placed { flip_v: true, ..placed };
        assert_eq!(
            flatten(&ShapePreset::Line.path(200.0, 100.0), &flipped)[0].points,
            vec![(0.0, 100.0), (200.0, 0.0)]
        );
        // An ellipse stays within its frame and touches its four sides.
        let ellipse = &flatten(&ShapePreset::Ellipse.path(200.0, 100.0), &placed)[0];
        assert!(ellipse.closed && ellipse.points.len() > 30);
        let xs: Vec<f32> = ellipse.points.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = ellipse.points.iter().map(|p| p.1).collect();
        assert!(xs.iter().all(|x| (-0.01..=200.01).contains(x)) && ys.iter().all(|y| (-0.01..=100.01).contains(y)));
        assert!(xs.iter().any(|x| *x > 199.9) && xs.iter().any(|x| *x < 0.1));
        assert!(in_polygon((100.0, 50.0), &ellipse.points));
        assert!(!in_polygon((5.0, 5.0), &ellipse.points));
        // Every preset gives something to draw.
        for preset in ShapePreset::ALL {
            let lines = flatten(&preset.path(96.0, 96.0), &Placed::plain(0.0, 0.0, 96.0, 96.0));
            assert!(!lines.is_empty(), "{preset:?}");
            assert_eq!(lines[0].closed, !preset.is_line(), "{preset:?}");
        }
    }

    #[test]
    fn hits_follow_the_outline() {
        let star = &flatten(&ShapePreset::Star5.path(100.0, 100.0), &Placed::plain(0.0, 0.0, 100.0, 100.0))[0];
        assert!(in_polygon((50.0, 50.0), &star.points));
        // Between two points of the star there is nothing.
        assert!(!in_polygon((15.0, 15.0), &star.points));
        let line = Polyline {
            points: vec![(0.0, 0.0), (100.0, 0.0)],
            closed: false,
        };
        assert_eq!(distance_to((50.0, 3.0), &line), 3.0);
        assert_eq!(distance_to((104.0, 3.0), &line), 5.0);
        let square = Polyline {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
            closed: true,
        };
        // The side that closes the outline counts.
        assert_eq!(distance_to((-2.0, 5.0), &square), 2.0);
        assert!(!in_polygon((5.0, 5.0), &[]));
    }

    #[test]
    fn arrowheads_point_along_the_line() {
        let head = arrow_head(LineEnd::Triangle, (100.0, 0.0), (0.0, 0.0), 10.0).unwrap();
        assert!(head.filled);
        assert_eq!(head.points, vec![(100.0, 0.0), (90.0, 5.0), (90.0, -5.0)]);
        assert_eq!(head.cut, 8.0);
        let open = arrow_head(LineEnd::Open, (0.0, 50.0), (0.0, 0.0), 10.0).unwrap();
        assert!(!open.filled);
        assert!(close(open.points[1], (0.0, 50.0)) && close(open.points[0], (-5.0, 40.0)));
        assert!(arrow_head(LineEnd::None, (1.0, 1.0), (0.0, 0.0), 10.0).is_none());
        assert!(arrow_head(LineEnd::Oval, (1.0, 1.0), (1.0, 1.0), 10.0).is_none());
        assert_eq!(arrow_head(LineEnd::Diamond, (0.0, 0.0), (-9.0, 0.0), 8.0).unwrap().points.len(), 4);
        // The line is shortened under a head, at either end.
        let mut points = vec![(0.0, 0.0), (100.0, 0.0)];
        trim(&mut points, 8.0, true);
        assert!(close(points[0], (0.0, 0.0)) && close(points[1], (92.0, 0.0)));
        trim(&mut points, 2.0, false);
        assert!(close(points[0], (2.0, 0.0)) && close(points[1], (92.0, 0.0)));
        // A line shorter than its head keeps a stub.
        let mut short = vec![(0.0, 0.0), (4.0, 0.0)];
        trim(&mut short, 8.0, true);
        assert!(short[1].0 > 0.0 && short[1].0 < 1.0);
    }

    #[test]
    fn handles_resize_from_the_opposite_side() {
        let frame = Frame::new(100.0, 100.0, 200.0, 100.0);
        // Bottom-right corner, 20 right and 10 down.
        assert_eq!(resize(frame, 0.0, 4, (20.0, 10.0), false, false), Frame::new(100.0, 100.0, 220.0, 110.0));
        // Top-left corner: the bottom-right one does not move.
        assert_eq!(resize(frame, 0.0, 0, (20.0, 10.0), false, false), Frame::new(120.0, 110.0, 180.0, 90.0));
        // A side handle ignores the movement across it.
        assert_eq!(resize(frame, 0.0, 3, (-50.0, 30.0), false, false), Frame::new(100.0, 100.0, 150.0, 100.0));
        assert_eq!(resize(frame, 0.0, 1, (7.0, -30.0), false, false), Frame::new(100.0, 70.0, 200.0, 130.0));
        // Proportions kept from a corner: the larger pull wins.
        assert_eq!(resize(frame, 0.0, 4, (100.0, 10.0), true, false), Frame::new(100.0, 100.0, 300.0, 150.0));
        // Not from a side.
        assert_eq!(resize(frame, 0.0, 3, (100.0, 0.0), true, false), Frame::new(100.0, 100.0, 300.0, 100.0));
        // From the centre both sides move.
        assert_eq!(resize(frame, 0.0, 3, (10.0, 0.0), false, true), Frame::new(90.0, 100.0, 220.0, 100.0));
        // A side cannot be pulled through the opposite one.
        assert_eq!(resize(frame, 0.0, 3, (-500.0, 0.0), false, false).width, MIN_SIDE);
        // Turned a quarter, the handle on the "right" side points down:
        // pulling it down widens the frame, and its left side (now on
        // top) stays where it was.
        let turned = resize(frame, 90.0, 3, (0.0, 40.0), false, false);
        assert!((turned.width - 240.0).abs() < 1e-6 && (turned.height - 100.0).abs() < 1e-6);
        let fixed = |f: Frame| {
            let placed = Placed {
                rotation: 90.0,
                ..Placed::plain(f.x as f32, f.y as f32, (f.x + f.width) as f32, (f.y + f.height) as f32)
            };
            placed.handles()[7]
        };
        assert!(close(fixed(frame), fixed(turned)));
        assert_eq!(handle_axis(3, 0.0), 0);
        assert_eq!(handle_axis(0, 0.0), 1);
        assert_eq!(handle_axis(1, 0.0), 2);
        assert_eq!(handle_axis(2, 0.0), 3);
        assert_eq!(handle_axis(3, 90.0), 2);
        assert_eq!(handle_axis(7, 45.0), 1);
    }

    #[test]
    fn lines_are_drawn_between_two_points() {
        let (frame, flip_h, flip_v) = line_frame((100.0, 80.0), (40.0, 120.0));
        assert_eq!((frame, flip_h, flip_v), (Frame::new(40.0, 80.0, 60.0, 40.0), true, false));
        assert_eq!(line_ends(frame, flip_h, flip_v, 0.0), ((100.0, 80.0), (40.0, 120.0)));
        // A turned line (from a file) has its ends turned with it.
        let (start, end) = line_ends(Frame::new(0.0, 0.0, 100.0, 0.0), false, false, 90.0);
        assert!((start.0 - 50.0).abs() < 1e-6 && (start.1 + 50.0).abs() < 1e-6);
        assert!((end.0 - 50.0).abs() < 1e-6 && (end.1 - 50.0).abs() < 1e-6);
        // Shift keeps a line on a multiple of 45 degrees.
        let to = constrain_direction((0.0, 0.0), (100.0, 8.0), 45.0);
        assert!((to.0 - 100.0).abs() < 1e-6 && to.1.abs() < 1e-6);
        let to = constrain_direction((0.0, 0.0), (60.0, 50.0), 45.0);
        assert!((to.0 - to.1).abs() < 1e-6 && (to.0 - 55.0).abs() < 1e-6);
        assert_eq!(constrain_direction((5.0, 5.0), (5.0, 5.0), 45.0), (5.0, 5.0));
        // Shapes: any corner can be the start, Shift makes a square.
        assert_eq!(drawn_frame((100.0, 100.0), (40.0, 130.0), false), Frame::new(40.0, 100.0, 60.0, 30.0));
        assert_eq!(drawn_frame((100.0, 100.0), (40.0, 130.0), true), Frame::new(40.0, 100.0, 60.0, 60.0));
        // The rotation handle: above is 0, to the right 90.
        assert_eq!(drag_angle((0.0, 0.0), (0.0, -50.0), false), 0.0);
        assert_eq!(drag_angle((0.0, 0.0), (50.0, 0.0), false), 90.0);
        assert_eq!(drag_angle((0.0, 0.0), (-50.0, 1.0), false), 269.0);
        assert_eq!(drag_angle((0.0, 0.0), (50.0, 10.0), true), 105.0);
    }
}
