//! 3-D columns, bars, lines and areas: the chart is built as boxes,
//! ribbons and prisms in a box of its own, seen through Excel's camera
//! (rotation, elevation, right-angle axes or perspective) and painted from
//! the back to the front, with walls, floor and gridlines behind.

use super::cartesian::{Shape, shapes};
use super::scale::{Fit, Scale};
use super::{
    AXIS, AxisId, ChartElement, GRIDLINE, Ink, Painter, Stroke, TEXT, TextAnchor, fmt_value, groups, label_text, line_height,
    shade,
};
use crate::chart::{Family, View3D};
use crate::style::Color;

/// Excel leaves one and a half times the depth of a shape free around it
/// in its row (`gapDepth` 150), half of it in front and half behind.
const GAP_DEPTH: f32 = 1.5;

/// Shading of the faces that do not look at the viewer, measured on
/// Excel's own drawings.
const SIDE_SHADE: f32 = -0.36;
const TOP_SHADE: f32 = -0.2;

/// The camera: turns a point of the chart's box (x to the right, y up, z
/// away from the viewer) into a point of the screen.
struct Camera {
    /// Right-angle axes: depth only shifts a point, by (`kx`, `ky`) per
    /// unit; otherwise the box is turned and seen in perspective.
    oblique: bool,
    kx: f32,
    ky: f32,
    sin_y: f32,
    cos_y: f32,
    sin_x: f32,
    cos_x: f32,
    /// Distance of the eye from the middle of the box.
    eye: f32,
    /// Size of the box.
    w: f32,
    h: f32,
    d: f32,
    /// Fit of the projected box into the plot.
    min: (f32, f32),
    scale: f32,
    origin: (f32, f32),
}

impl Camera {
    fn raw(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        if self.oblique {
            return (x + z * self.kx, y + z * self.ky);
        }
        let (x, y, z) = (x - self.w / 2.0, y - self.h / 2.0, z - self.d / 2.0);
        // Around the vertical axis, then tipped towards the viewer.
        let x1 = x * self.cos_y + z * self.sin_y;
        let z1 = -x * self.sin_y + z * self.cos_y;
        let y2 = y * self.cos_x + z1 * self.sin_x;
        let z2 = -y * self.sin_x + z1 * self.cos_x;
        let s = self.eye / (self.eye + z2).max(self.eye * 0.1);
        (x1 * s, y2 * s)
    }

    fn at(&self, x: f32, y: f32, z: f32) -> (f32, f32) {
        let (rx, ry) = self.raw(x, y, z);
        (
            self.origin.0 + (rx - self.min.0) * self.scale,
            self.origin.1 - (ry - self.min.1) * self.scale,
        )
    }

    /// The box of the projected corners: (left, bottom, width, height).
    fn extent(&self) -> (f32, f32, f32, f32) {
        let (mut lo, mut hi) = ((f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, f32::NEG_INFINITY));
        for corner in 0..8 {
            let (x, y) = self.raw(
                if corner & 1 == 0 { 0.0 } else { self.w },
                if corner & 2 == 0 { 0.0 } else { self.h },
                if corner & 4 == 0 { 0.0 } else { self.d },
            );
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        (lo.0, lo.1, (hi.0 - lo.0).max(1e-3), (hi.1 - lo.1).max(1e-3))
    }

    /// A camera for a box that fills the plot. `depth` is the depth of the
    /// box as a share of the length of its category axis (the horizontal
    /// one, the vertical one for bars).
    fn new(view: &View3D, plot: (f32, f32, f32, f32), depth: f32, horizontal: bool) -> Camera {
        let (px0, py0, px1, py1) = plot;
        let (pw, ph) = (px1 - px0, py1 - py0);
        let (sin_y, cos_y) = ((view.rot_y % 360) as f32).to_radians().sin_cos();
        let (sin_x, cos_x) = (view.rot_x.clamp(-90, 90) as f32).to_radians().sin_cos();
        let mut cam = Camera {
            oblique: view.right_angle_axes,
            kx: sin_y,
            ky: sin_x,
            sin_y,
            cos_y,
            sin_x,
            cos_x,
            eye: 1.0,
            w: pw,
            h: ph,
            d: 0.0,
            min: (0.0, 0.0),
            scale: 1.0,
            origin: (px0, py1),
        };
        let tall = view.height_percent.map(|p| p.clamp(5, 500) as f32 / 100.0);
        if cam.oblique {
            // The front of the box is drawn as it is; its depth takes what
            // it needs from the width and the height of the plot.
            let (kx, ky) = (cam.kx.abs(), cam.ky.abs());
            if horizontal {
                cam.h = ph / (1.0 + depth * ky);
                cam.d = cam.h * depth;
                cam.w = (pw - cam.d * kx).max(pw * 0.3);
            } else {
                cam.w = pw / (1.0 + depth * kx);
                cam.d = cam.w * depth;
                cam.h = match tall {
                    Some(t) => (cam.w * t).min(ph - cam.d * ky),
                    None => ph - cam.d * ky,
                }
                .max(ph * 0.3);
            }
        } else {
            // The base is one unit long; the free dimension is the one
            // that makes the picture of the box as wide and as high as the
            // plot, found by bisection (the picture grows with it).
            let fov = (view.perspective.clamp(1, 240) as f32).to_radians();
            let fit = |cam: &mut Camera, free: f32| {
                if horizontal {
                    (cam.w, cam.h) = (free, 1.0);
                } else {
                    (cam.w, cam.h) = (1.0, free);
                }
                cam.d = depth;
                // The eye stands where the box, turned, fills its field of view.
                let reach = (cam.w * cam.cos_y.abs() + cam.d * cam.sin_y.abs()) / 2.0;
                cam.eye = (reach / (fov / 2.0).tan()).max(reach * 1.2 + cam.d);
                let (_, _, bw, bh) = cam.extent();
                bw / bh
            };
            match (tall, horizontal) {
                (Some(t), false) => {
                    fit(&mut cam, t);
                }
                _ => {
                    let (mut lo, mut hi) = (0.02f32, 50.0f32);
                    for _ in 0..40 {
                        let mid = (lo * hi).sqrt();
                        let wider = fit(&mut cam, mid) > pw / ph;
                        // A taller box makes a narrower picture; a longer
                        // bar chart a wider one.
                        if wider != horizontal {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                }
            }
        }
        let (x, y, bw, bh) = cam.extent();
        cam.min = (x, y);
        cam.scale = (pw / bw).min(ph / bh);
        // Centred in the room that is left over.
        cam.origin = (px0 + (pw - bw * cam.scale) / 2.0, py1 - (ph - bh * cam.scale) / 2.0);
        cam
    }

    /// The viewer sees the right-hand faces of things (else the left).
    fn sees_right(&self) -> bool {
        self.sin_y >= 0.0
    }

    /// The viewer sees the top of things (else what is under them).
    fn sees_top(&self) -> bool {
        self.sin_x >= 0.0
    }
}

/// A box with its faces along the axes of the chart.
struct Solid {
    x: (f32, f32),
    y: (f32, f32),
    z: (f32, f32),
    color: Color,
}

fn quad(p: &mut Painter, cam: &Camera, corners: [(f32, f32, f32); 4], color: Color) {
    let pts: Vec<(f32, f32)> = corners.iter().map(|(x, y, z)| cam.at(*x, *y, *z)).collect();
    // The hairline of the same colour closes the seams between faces.
    p.polygon(&pts, Some(Ink::solid(color)), Some(Stroke::new(color, 0.5)));
}

fn paint_solid(p: &mut Painter, cam: &Camera, s: &Solid) {
    let ((x0, x1), (y0, y1), (z0, z1)) = (s.x, s.y, s.z);
    let side = if cam.sees_right() { x1 } else { x0 };
    quad(p, cam, [(side, y0, z0), (side, y0, z1), (side, y1, z1), (side, y1, z0)], shade(s.color, SIDE_SHADE));
    let lid = if cam.sees_top() { y1 } else { y0 };
    quad(p, cam, [(x0, lid, z0), (x1, lid, z0), (x1, lid, z1), (x0, lid, z1)], shade(s.color, TOP_SHADE));
    quad(p, cam, [(x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0)], s.color);
}

/// Twice the signed area of a polygon on screen: its sign says which way
/// round the points run, and so which side of a surface is seen.
fn winding(pts: &[(f32, f32)]) -> f32 {
    let mut sum = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        sum += a.0 * b.1 - b.0 * a.1;
    }
    sum
}

pub(super) fn draw_3d(p: &mut Painter, area: (f32, f32, f32, f32)) {
    let chart = p.chart;
    let data = p.data;
    let z = p.z;
    let (left, top, right, bottom) = area;
    let label_size = p.label_size;
    let label_h = p.label_h;
    let kind = chart.kind;
    let family = kind.family();
    let horizontal = kind.is_horizontal();

    let groups = groups(chart, data);
    let series: Vec<usize> = groups.iter().flat_map(|g| g.series.iter().copied()).collect();
    let n = data.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if n == 0 || series.is_empty() {
        return;
    }
    let shapes: Vec<Option<Shape>> = shapes(chart, data, &groups, n);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for shape in shapes.iter().flatten() {
        for v in shape.to.iter().chain(shape.from.iter()).flatten() {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    if family != Family::Line {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    // The length of the value axis is not known before the camera is set
    // up: a first scale gives the labels their room, the final one follows.
    let percent = kind.is_percent();
    let fit = |room: usize| Fit {
        percent,
        room,
        snug: false,
    };
    let scale = Scale::new(&chart.axes.value, lo, hi, fit(5));

    // Lines and plain areas stand one behind the other; everything else
    // shares a single row.
    let rows = if family == Family::Line || (family == Family::Area && !kind.is_stacked()) { series.len() } else { 1 };
    let on_ticks = family == Family::Area && n > 1;
    let slots = if on_ticks { n - 1 } else { n } as f32;

    // ---- labels and the room they take -------------------------------------
    let locale = p.options.locale;
    let value_axis = &chart.axes.value;
    let code = value_axis.number_format.clone().unwrap_or_else(|| {
        if kind.is_percent() { "0%".to_string() } else { data.value_format.clone() }
    });
    let labels_of = |p: &Painter, scale: &Scale| -> Vec<(f64, String, f32)> {
        if !value_axis.visible {
            return Vec::new();
        }
        scale
            .major_ticks()
            .iter()
            .map(|v| {
                let text = fmt_value(*v / value_axis.display_unit.divisor(), &code, locale);
                let w = p.width(&text, label_size);
                (scale.fraction(*v), text, w)
            })
            .collect()
    };
    let value_labels = labels_of(p, &scale);
    let cat_labels: Vec<(String, f32)> = if chart.axes.category.visible {
        (0..n)
            .map(|k| {
                let text = data.categories.get(k).cloned().unwrap_or_default();
                let w = p.width(&text, label_size);
                (text, w)
            })
            .collect()
    } else {
        Vec::new()
    };
    let row_labels: Vec<(String, f32)> = if rows > 1 {
        series
            .iter()
            .map(|&si| {
                let text = p.series_name(si);
                let w = p.width(&text, label_size);
                (text, w)
            })
            .collect()
    } else {
        Vec::new()
    };
    let widest = |labels: &[(String, f32)]| labels.iter().map(|l| l.1).fold(0.0, f32::max);
    let value_w = value_labels.iter().map(|l| l.2).fold(0.0, f32::max);
    let title_size = (13.33 * z).max(6.0);
    let title_h = line_height(title_size);
    let (side_title, foot_title) = if horizontal {
        (chart.axes.category.title.as_ref(), value_axis.title.as_ref())
    } else {
        (value_axis.title.as_ref(), chart.axes.category.title.as_ref())
    };
    let side_w = if horizontal { widest(&cat_labels).min((right - left) * 0.4) } else { value_w };
    let foot_h = if (horizontal && value_labels.is_empty()) || (!horizontal && cat_labels.is_empty()) { 0.0 } else { label_h + 6.0 * z };
    let px0 = left + if side_w > 0.0 { side_w + 8.0 * z } else { 0.0 } + if side_title.is_some() { title_h + 6.0 * z } else { 0.0 };
    let px1 = right - if row_labels.is_empty() { 4.0 * z } else { widest(&row_labels) + 8.0 * z };
    let py0 = top + label_h / 2.0;
    let py1 = bottom - foot_h - if foot_title.is_some() { title_h + 4.0 * z } else { 0.0 };
    if px1 - px0 < 20.0 || py1 - py0 < 20.0 {
        return;
    }
    let view = chart.view3d_or_default();
    // A shape is as deep as it is wide: a bar as its width, a line or an
    // area as a bar that would stand alone in its category.
    let across = match family {
        Family::Column | Family::Bar if !kind.is_stacked() => series.len() as f32,
        _ => 1.0,
    };
    let gap = match family {
        Family::Column | Family::Bar => chart.gap_width_or_default() as f32 / 100.0,
        _ => 1.5,
    };
    let row_share = (1.0 + GAP_DEPTH) / (across + gap) * view.depth_percent.clamp(20, 2000) as f32 / 100.0;
    let depth = rows as f32 * row_share / slots.max(1.0);
    let cam = Camera::new(&view, (px0, py0, px1, py1), depth, horizontal);
    let (w, h, d) = (cam.w, cam.h, cam.d);
    let row_d = d / rows as f32;
    // Now that the length of the value axis is known: no more gridlines
    // than its labels have room for.
    let (from, to) = if horizontal { (cam.at(0.0, 0.0, 0.0), cam.at(w, 0.0, 0.0)) } else { (cam.at(0.0, 0.0, 0.0), cam.at(0.0, h, 0.0)) };
    let length = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    let spacing = if horizontal { (value_w + 12.0 * z).max(34.0 * z) } else { label_h * 1.25 };
    let scale = Scale::new(value_axis, lo, hi, fit(Fit::along(length, spacing).room));
    p.scale(AxisId::Value, &scale);
    let base = scale.crossing(value_axis.crossing);
    let ticks = scale.major_ticks();
    let value_labels = labels_of(p, &scale);
    // A point of the chart: position along the categories (in slots),
    // fraction of the value axis, depth.
    let pos = |c: f32, f: f32, depth: f32| -> (f32, f32, f32) {
        if horizontal { (f * w, c / slots.max(1.0) * h, depth) } else { (c / slots.max(1.0) * w, f * h, depth) }
    };

    // ---- walls, floor, gridlines -------------------------------------------
    let grid = Stroke::new(GRIDLINE, 1.0);
    let edge = Stroke::new(AXIS, 1.0);
    let far_x = if cam.sees_right() { 0.0 } else { w };
    let far_y = if cam.sees_top() { 0.0 } else { h };
    let outline = |p: &mut Painter, corners: [(f32, f32, f32); 4]| {
        let pts: Vec<(f32, f32)> = corners.iter().map(|(x, y, z)| cam.at(*x, *y, *z)).collect();
        p.polygon(&pts, None, Some(edge));
    };
    // Like Excel, only the floor has an edge; the walls show by their gridlines.
    p.tag(ChartElement::PlotArea);
    p.area(ChartElement::PlotArea, (px0, py0, px1, py1));
    if horizontal {
        outline(p, [(far_x, 0.0, 0.0), (far_x, 0.0, d), (far_x, h, d), (far_x, h, 0.0)]);
    } else {
        outline(p, [(0.0, far_y, 0.0), (w, far_y, 0.0), (w, far_y, d), (0.0, far_y, d)]);
    }
    if value_axis.major_gridlines {
        let element = ChartElement::MajorGridlines(AxisId::Value);
        p.tag(element);
        for v in &ticks {
            let f = scale.clamped(*v) as f32;
            // Across the back wall, then forward along the wall beside it.
            let (a, b, c) = if horizontal {
                ((f * w, 0.0, d), (f * w, h, d), (f * w, far_y, 0.0))
            } else {
                ((0.0, f * h, d), (w, f * h, d), (far_x, f * h, 0.0))
            };
            let (a2, b2) = (cam.at(a.0, a.1, a.2), cam.at(b.0, b.1, b.2));
            p.line(a2.0, a2.1, b2.0, b2.1, grid);
            p.anchor(element, a2.0, a2.1);
            p.anchor(element, b2.0, b2.1);
            let near = if horizontal { (f * w, far_y, d) } else { (far_x, f * h, d) };
            let (n2, c2) = (cam.at(near.0, near.1, near.2), cam.at(c.0, c.1, c.2));
            p.line(n2.0, n2.1, c2.0, c2.1, grid);
        }
    }

    // ---- the series --------------------------------------------------------
    let fb = scale.clamped(base) as f32;
    let mut texts: Vec<(usize, String, f32, f32, TextAnchor)> = Vec::new();
    let labelled = |si: usize| chart.labels_for(&chart.series[si]);
    match family {
        Family::Column | Family::Bar => {
            let ns = series.len() as f32;
            let stacked = kind.is_stacked();
            let gap = chart.gap_width_or_default() as f32 / 100.0;
            let cluster_bars = if stacked { 1.0 } else { ns };
            let bar = 1.0 / (cluster_bars + gap);
            let thick = row_d / (1.0 + GAP_DEPTH);
            let (z0, z1) = ((row_d - thick) / 2.0, (row_d + thick) / 2.0);
            let mut solids: Vec<(Solid, usize, usize)> = Vec::new();
            for (j, &si) in series.iter().enumerate() {
                let Some(shape) = &shapes[si] else { continue };
                for k in 0..n {
                    let Some(v) = shape.to[k] else { continue };
                    let from = shape.from[k].unwrap_or(base);
                    let slot = if stacked { 0.0 } else { j as f32 };
                    let c0 = k as f32 + (1.0 - bar * cluster_bars) / 2.0 + slot * bar;
                    let (f0, f1) = (scale.clamped(from) as f32, scale.clamped(v) as f32);
                    let a = pos(c0, f0.min(f1), z0);
                    let b = pos(c0 + bar, f0.max(f1), z1);
                    solids.push((
                        Solid {
                            x: (a.0, b.0),
                            y: (a.1, b.1),
                            z: (z0, z1),
                            color: p.point_color(si, k),
                        },
                        si,
                        k,
                    ));
                }
            }
            // Farthest first: along the categories, then along the stack.
            let order = |s: &Solid| {
                let x = if cam.sees_right() { s.x.0 } else { -s.x.1 };
                let y = if cam.sees_top() { s.y.0 } else { -s.y.1 };
                if horizontal { (y, x) } else { (x, y) }
            };
            solids.sort_by(|a, b| order(&a.0).partial_cmp(&order(&b.0)).unwrap_or(std::cmp::Ordering::Equal));
            for (solid, si, k) in &solids {
                let element = ChartElement::Point(*si as u32, *k as u32);
                p.tag(element);
                paint_solid(p, &cam, solid);
                // Handles on the corners of the face that looks at the viewer.
                for (x, y) in [(solid.x.0, solid.y.0), (solid.x.1, solid.y.0), (solid.x.1, solid.y.1), (solid.x.0, solid.y.1)] {
                    let (x, y) = cam.at(x, y, solid.z.0);
                    p.anchor(element, x, y);
                }
                if let Some(labels) = labelled(*si) {
                    let text = label_text(p, *si, *k, None, labels);
                    if text.is_empty() {
                        continue;
                    }
                    if horizontal {
                        let (x, y) = cam.at(solid.x.1, (solid.y.0 + solid.y.1) / 2.0, z0);
                        texts.push((*si, text, x + 4.0 * z, y - label_h / 2.0, TextAnchor::Start));
                    } else {
                        let (x, y) = cam.at((solid.x.0 + solid.x.1) / 2.0, solid.y.1, z0);
                        texts.push((*si, text, x, y - label_h - 2.0 * z, TextAnchor::Middle));
                    }
                }
            }
        }
        _ => {
            let thick = row_d / (1.0 + GAP_DEPTH);
            // The first series stands at the front: paint the last one first.
            let order: Vec<(usize, usize)> = if rows > 1 {
                series.iter().copied().enumerate().rev().collect()
            } else {
                series.iter().copied().enumerate().collect()
            };
            for (row, si) in order {
                let Some(shape) = &shapes[si] else { continue };
                let row = if rows > 1 { row as f32 } else { 0.0 };
                let (z0, z1) = (row * row_d + (row_d - thick) / 2.0, row * row_d + (row_d + thick) / 2.0);
                let color = data.series[si].color;
                let centre = |k: usize| if on_ticks { k as f32 } else { k as f32 + 0.5 };
                p.tag(ChartElement::Series(si as u32));
                // Runs of points without a gap.
                let mut runs: Vec<Vec<(f32, f32, f32)>> = vec![Vec::new()];
                for k in 0..n {
                    match shape.to[k] {
                        Some(v) => {
                            let lower = scale.clamped(shape.from[k].unwrap_or(base)) as f32;
                            runs.last_mut().expect("a run").push((centre(k), scale.clamped(v) as f32, lower));
                        }
                        None => runs.push(Vec::new()),
                    }
                }
                for run in runs.iter().filter(|r| r.len() >= 2) {
                    for pt in run {
                        let (x, y, depth) = pos(pt.0, pt.1, z0);
                        let (x, y) = cam.at(x, y, depth);
                        p.anchor(ChartElement::Series(si as u32), x, y);
                    }
                    let mut pieces: Vec<usize> = (0..run.len() - 1).collect();
                    if !cam.sees_right() {
                        pieces.reverse();
                    }
                    for i in pieces {
                        let (a, b) = (run[i], run[i + 1]);
                        let (a0, b0) = (pos(a.0, a.1, z0), pos(b.0, b.1, z0));
                        let (a1, b1) = (pos(a.0, a.1, z1), pos(b.0, b.1, z1));
                        let pts: Vec<(f32, f32)> =
                            [a0, b0, b1, a1].iter().map(|(x, y, z)| cam.at(*x, *y, *z)).collect();
                        // Seen from above or from below?
                        let upper = winding(&pts) < 0.0;
                        if family == Family::Line {
                            let tone = if upper { color } else { shade(color, SIDE_SHADE) };
                            p.polygon(&pts, Some(Ink::solid(tone)), Some(Stroke::new(tone, 0.5)));
                        } else if upper {
                            let tone = shade(color, TOP_SHADE);
                            p.polygon(&pts, Some(Ink::solid(tone)), Some(Stroke::new(tone, 0.5)));
                        }
                    }
                    if family == Family::Area {
                        // The end of the prism that faces the viewer.
                        let end = if cam.sees_right() { run[run.len() - 1] } else { run[0] };
                        quad(
                            p,
                            &cam,
                            [pos(end.0, end.2, z0), pos(end.0, end.2, z1), pos(end.0, end.1, z1), pos(end.0, end.1, z0)],
                            shade(color, SIDE_SHADE),
                        );
                        let mut front: Vec<(f32, f32)> = run
                            .iter()
                            .map(|pt| {
                                let (x, y, z) = pos(pt.0, pt.1, z0);
                                cam.at(x, y, z)
                            })
                            .collect();
                        front.extend(run.iter().rev().map(|pt| {
                            let (x, y, z) = pos(pt.0, pt.2, z0);
                            cam.at(x, y, z)
                        }));
                        p.polygon(&front, Some(Ink::solid(color)), Some(Stroke::new(color, 0.5)));
                    }
                }
                if let Some(labels) = labelled(si) {
                    for k in 0..n {
                        let Some(v) = shape.to[k] else { continue };
                        let text = label_text(p, si, k, None, labels);
                        if text.is_empty() {
                            continue;
                        }
                        let (x, y, depth) = pos(centre(k), scale.clamped(v) as f32, z0);
                        let (x, y) = cam.at(x, y, depth);
                        texts.push((si, text, x, y - label_h - 2.0 * z, TextAnchor::Middle));
                    }
                }
            }
        }
    }

    // ---- axes --------------------------------------------------------------
    let text = TEXT;
    // The category axis runs along the front edge of the floor (or of the
    // side wall, for bars), at the value where it crosses the scale.
    let (a, b) = (pos(0.0, fb, 0.0), pos(slots, fb, 0.0));
    let (value_id, category_id) = (ChartElement::Axis(AxisId::Value), ChartElement::Axis(AxisId::Category));
    p.tag(category_id);
    if chart.axes.category.visible {
        let (a2, b2) = (cam.at(a.0, a.1, a.2), cam.at(b.0, b.1, b.2));
        p.line(a2.0, a2.1, b2.0, b2.1, Stroke::new(AXIS, 1.0));
    }
    let centre = |k: usize| if on_ticks { k as f32 } else { k as f32 + 0.5 };
    if horizontal {
        p.tag(value_id);
        for (f, label, _) in &value_labels {
            let (x, y) = cam.at(*f as f32 * w, 0.0, 0.0);
            p.text(label, x.round(), y + 5.0 * z, label_size, text, TextAnchor::Middle);
        }
        p.tag(category_id);
        let step = ((label_h / (cam.scale * h / slots.max(1.0)).max(1.0)).ceil() as usize).max(1);
        for (k, (label, lw)) in cat_labels.iter().enumerate() {
            if k % step != 0 {
                continue;
            }
            let (x, y) = cam.at(0.0, centre(k) / slots.max(1.0) * h, 0.0);
            p.text(label, x - 6.0 * z - lw.min(side_w), y - label_h / 2.0, label_size, text, TextAnchor::Start);
        }
    } else {
        p.tag(value_id);
        for (f, label, _) in &value_labels {
            let (x, y) = cam.at(0.0, *f as f32 * h, 0.0);
            p.text(label, x - 6.0 * z, y.round() - label_h / 2.0, label_size, text, TextAnchor::End);
        }
        p.tag(category_id);
        let slot_px = cam.scale * w / slots.max(1.0);
        let step = (((widest(&cat_labels) + 6.0 * z) / slot_px.max(1.0)).ceil() as usize).max(1);
        for (k, (label, lw)) in cat_labels.iter().enumerate() {
            if k % step != 0 {
                continue;
            }
            let (x, y) = cam.at(centre(k) / slots.max(1.0) * w, 0.0, 0.0);
            p.text(label, x - lw / 2.0, y + 5.0 * z, label_size, text, TextAnchor::Start);
        }
    }
    // Rows are named along the edge of the floor that runs into the depth.
    p.tag(ChartElement::PlotArea);
    for (row, (label, _)) in row_labels.iter().enumerate() {
        let (x, y) = cam.at(w, 0.0, (row as f32 + 0.5) * row_d);
        p.text(label, x + 6.0 * z, y - label_h / 2.0, label_size, text, TextAnchor::Start);
    }
    let (side_id, foot_id) = if horizontal { (AxisId::Category, AxisId::Value) } else { (AxisId::Value, AxisId::Category) };
    if let Some(title) = side_title.filter(|t| !t.is_empty()) {
        p.tag(ChartElement::AxisTitle(side_id));
        p.text_rotated(title, left + title_h / 2.0, (py0 + py1) / 2.0, title_size, text, -90.0);
    }
    if let Some(title) = foot_title.filter(|t| !t.is_empty()) {
        p.tag(ChartElement::AxisTitle(foot_id));
        p.text(title, (px0 + px1) / 2.0, py1 + foot_h + 2.0 * z, title_size, text, TextAnchor::Middle);
    }
    for (si, label, x, y, anchor) in texts {
        p.tag(ChartElement::DataLabels(si as u32));
        p.text(&label, x, y, label_size, text, anchor);
    }
}
