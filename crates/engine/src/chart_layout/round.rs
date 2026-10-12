//! Pies, doughnuts and 3-D pies: one series (one ring per series for a
//! doughnut) shown as shares of a whole.

use super::{Ink, Painter, Seg, TEXT, TextAnchor, arc_points, arc_segs, explosion_of, label_text, shade};
use crate::chart::{DataLabels, Family, LabelPosition};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// One slice: its place in the series, its angles (radians, clockwise
/// from 3 o'clock on screen) and its share of the whole.
struct Slice {
    point: usize,
    start: f32,
    sweep: f32,
    share: f64,
}

/// The slices of a series. Like Excel, a pie shows the size of a value,
/// whatever its sign.
fn slices(values: &[Option<f64>], first_angle: f32) -> Vec<Slice> {
    let sizes: Vec<f64> = values.iter().map(|v| v.unwrap_or(0.0).abs()).collect();
    let total: f64 = sizes.iter().sum();
    let mut out = Vec::new();
    if total <= 0.0 {
        return out;
    }
    // Slices start at 12 o'clock (plus the angle asked for) and run clockwise.
    let mut angle = -FRAC_PI_2 + first_angle.to_radians();
    for (point, size) in sizes.iter().enumerate() {
        let share = size / total;
        let sweep = share as f32 * TAU;
        if sweep > 0.0 {
            out.push(Slice {
                point,
                start: angle,
                sweep,
                share,
            });
        }
        angle += sweep;
    }
    out
}

/// Where the label of a slice goes: (x, top, anchor), or nothing when it
/// has no room.
#[allow(clippy::too_many_arguments)]
fn place_label(
    p: &Painter,
    labels: &DataLabels,
    text: &str,
    slice: &Slice,
    centre: (f32, f32),
    radii: (f32, f32),
    inner: f32,
    doughnut: bool,
) -> Option<(f32, f32, TextAnchor)> {
    let (cx, cy) = centre;
    let (rx, ry) = radii;
    let w = p.width(text, p.label_size);
    let h = p.label_h;
    let mid = slice.start + slice.sweep / 2.0;
    let (sin, cos) = mid.sin_cos();
    // Inside: at this fraction of the radius.
    let inside = |f: f32| Some((cx + rx * f * cos, cy + ry * f * sin - h / 2.0, TextAnchor::Middle));
    let outside = || {
        // The box of the text just clear of the rim, on the side it is on.
        let (ex, ey) = (rx + 6.0 * p.z, ry + 6.0 * p.z);
        let x = cx + ex * cos + cos * w / 2.0;
        let y = cy + ey * sin + sin * h / 2.0;
        Some((x, y - h / 2.0, TextAnchor::Middle))
    };
    let ring_mid = (1.0 + inner) / 2.0;
    // Room across the slice where an inside label would sit.
    let fits = |f: f32| slice.sweep > 0.25 && (slice.sweep >= PI || 2.0 * rx * f * (slice.sweep / 2.0).sin() >= w * 0.8);
    match labels.position {
        LabelPosition::Center => inside(if doughnut { ring_mid } else { 0.5 }),
        LabelPosition::InsideEnd if !doughnut => inside(0.8),
        LabelPosition::OutsideEnd if !doughnut => outside(),
        _ if doughnut => fits(ring_mid).then(|| inside(ring_mid)).flatten(),
        _ if fits(0.65) => inside(0.65),
        _ => outside(),
    }
}

pub(super) fn draw_round(p: &mut Painter, area: (f32, f32, f32, f32)) {
    let chart = p.chart;
    let data = p.data;
    let z = p.z;
    let (left, top, right, bottom) = area;
    let doughnut = chart.kind.family() == Family::Doughnut;
    let rings = if doughnut { data.series.len().min(chart.series.len()) } else { data.series.len().min(1) };
    if rings == 0 {
        return;
    }
    let first_angle = chart.first_slice_angle.min(360) as f32;
    let all: Vec<Vec<Slice>> = (0..rings).map(|si| slices(&data.series[si].values, first_angle)).collect();
    if all.iter().all(Vec::is_empty) {
        return;
    }
    // Labels written outside the pie need room around it.
    let outside = !doughnut
        && (0..rings).any(|si| {
            chart.labels_for(&chart.series[si]).is_some_and(|l| {
                l.position == LabelPosition::OutsideEnd
                    || (!matches!(l.position, LabelPosition::Center | LabelPosition::InsideEnd)
                        && all[si].iter().any(|s| s.sweep <= 0.25))
            })
        });
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0;
    let room = if outside { p.label_h + 8.0 * z } else { 4.0 * z };
    let widest = if outside { 30.0 * z } else { 0.0 };
    let available = (((right - left) / 2.0 - widest).min((bottom - top) / 2.0) - room).max(4.0);
    // Slices pulled out of the pie take their room from its radius.
    let burst = if doughnut {
        0.0
    } else {
        all[0].iter().map(|s| explosion_of(&chart.series[0], s.point)).max().unwrap_or(0) as f32 / 100.0
    };
    let r = available / (1.0 + burst);
    let hole = if doughnut { chart.hole_size.clamp(10, 90) as f32 / 100.0 } else { 0.0 };
    let ring_w = r * (1.0 - hole) / rings as f32;

    let mut texts: Vec<(String, f32, f32, TextAnchor)> = Vec::new();
    for (si, ring) in all.iter().enumerate() {
        let look = p.look(si);
        let (r_in, r_out) = if doughnut {
            (r * hole + ring_w * si as f32, r * hole + ring_w * (si as f32 + 1.0))
        } else {
            (0.0, r)
        };
        let whole = ring.len() == 1;
        for slice in ring {
            let color = p.point_color(si, slice.point);
            let mid = slice.start + slice.sweep / 2.0;
            let out = if doughnut { 0.0 } else { explosion_of(&chart.series[si], slice.point) as f32 / 100.0 * r };
            let (ox, oy) = (cx + out * mid.cos(), cy + out * mid.sin());
            let end = slice.start + slice.sweep;
            let fill = Some(Ink::solid(color));
            if whole && !doughnut {
                p.ellipse(ox, oy, r_out, r_out, fill, look.outline);
            } else {
                let mut segs = Vec::new();
                if doughnut {
                    segs.push(Seg::Move(ox + r_out * slice.start.cos(), oy + r_out * slice.start.sin()));
                    arc_segs(&mut segs, ox, oy, r_out, r_out, slice.start, end);
                    segs.push(Seg::Line(ox + r_in * end.cos(), oy + r_in * end.sin()));
                    arc_segs(&mut segs, ox, oy, r_in, r_in, end, slice.start);
                } else {
                    segs.push(Seg::Move(ox, oy));
                    segs.push(Seg::Line(ox + r_out * slice.start.cos(), oy + r_out * slice.start.sin()));
                    arc_segs(&mut segs, ox, oy, r_out, r_out, slice.start, end);
                }
                segs.push(Seg::Close);
                p.out.push(super::Prim::Path {
                    segs,
                    fill,
                    stroke: look.outline,
                });
            }
            if let Some(labels) = chart.labels_for(&chart.series[si]) {
                let text = label_text(p, si, slice.point, Some(slice.share), labels);
                if text.is_empty() {
                    continue;
                }
                let inner = if doughnut { r_in / r_out } else { 0.0 };
                if let Some((x, y, anchor)) =
                    place_label(p, labels, &text, slice, (ox, oy), (r_out, r_out), inner, doughnut)
                {
                    texts.push((text, x, y, anchor));
                }
            }
        }
    }
    let size = p.label_size;
    for (text, x, y, anchor) in texts {
        p.text(&text, x, y, size, TEXT, anchor);
    }
}

/// A pie seen from above its rim: an ellipse on a short cylinder, the
/// slices at the front showing their sides.
pub(super) fn draw_pie_3d(p: &mut Painter, area: (f32, f32, f32, f32)) {
    let chart = p.chart;
    let data = p.data;
    let z = p.z;
    let (left, top, right, bottom) = area;
    let Some(series) = data.series.first() else {
        return;
    };
    if chart.series.is_empty() {
        return;
    }
    let view = chart.view3d_or_default();
    // In a 3-D pie the rotation of the view is the angle of the first slice.
    let ring = slices(&series.values, (view.rot_y % 360) as f32 + chart.first_slice_angle as f32);
    if ring.is_empty() {
        return;
    }
    let elevation = (view.rot_x.clamp(10, 80) as f32).to_radians();
    let squash = elevation.sin();
    // Thickness of the pie, as a share of its radius.
    let thick = 0.2 * view.depth_percent.clamp(20, 2000) as f32 / 100.0 * elevation.cos();
    let burst = ring.iter().map(|s| explosion_of(&chart.series[0], s.point)).max().unwrap_or(0) as f32 / 100.0;
    let labelled = chart.labels_for(&chart.series[0]);
    let room = if labelled.is_some_and(|l| l.position == LabelPosition::OutsideEnd) { p.label_h + 8.0 * z } else { 4.0 * z };
    let w = (right - left) / 2.0 - room;
    let h = bottom - top - 2.0 * room;
    let rx = (w / (1.0 + burst)).min(h / ((2.0 * squash + thick) * (1.0 + burst))).max(4.0);
    let ry = rx * squash;
    let band = rx * thick;
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0 - band / 2.0;
    let look = p.look(0);

    // Farthest slices first: those whose middle points up the screen.
    let mut order: Vec<&Slice> = ring.iter().collect();
    order.sort_by(|a, b| {
        let key = |s: &Slice| (s.start + s.sweep / 2.0).sin();
        key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut texts: Vec<(String, f32, f32, TextAnchor)> = Vec::new();
    for slice in order {
        let color = p.point_color(0, slice.point);
        let mid = slice.start + slice.sweep / 2.0;
        let out = explosion_of(&chart.series[0], slice.point) as f32 / 100.0;
        let (ox, oy) = (cx + out * rx * mid.cos(), cy + out * ry * mid.sin());
        let end = slice.start + slice.sweep;
        let rim = |a: f32| (ox + rx * a.cos(), oy + ry * a.sin());
        // The cut faces show where a slice stands apart from its neighbours.
        if out > 0.0 || burst > 0.0 {
            for a in [slice.start, end] {
                let (x, y) = rim(a);
                let face = [(ox, oy), (x, y), (x, y + band), (ox, oy + band)];
                p.polygon(&face, Some(Ink::solid(shade(color, -0.2))), None);
            }
        }
        // The rim shows on the half of the ellipse that faces the viewer.
        for turn in -1..=2 {
            let lo = slice.start.max(turn as f32 * TAU);
            let hi = end.min(turn as f32 * TAU + PI);
            if hi - lo <= 1e-4 {
                continue;
            }
            let mut wall = arc_points(ox, oy, rx, ry, lo, hi);
            let lower: Vec<(f32, f32)> = wall.iter().rev().map(|(x, y)| (*x, y + band)).collect();
            wall.extend(lower);
            p.polygon(&wall, Some(Ink::solid(shade(color, -0.3))), None);
        }
        let fill = Some(Ink::solid(color));
        if ring.len() == 1 {
            p.ellipse(ox, oy, rx, ry, fill, None);
        } else {
            let (sx, sy) = rim(slice.start);
            let mut segs = vec![Seg::Move(ox, oy), Seg::Line(sx, sy)];
            arc_segs(&mut segs, ox, oy, rx, ry, slice.start, end);
            segs.push(Seg::Close);
            p.out.push(super::Prim::Path {
                segs,
                fill,
                stroke: look.outline,
            });
        }
        if let Some(labels) = labelled {
            let text = label_text(p, 0, slice.point, Some(slice.share), labels);
            if !text.is_empty()
                && let Some((x, y, anchor)) = place_label(p, labels, &text, slice, (ox, oy), (rx, ry), 0.0, false)
            {
                // Labels under the pie clear its rim.
                let below = if (slice.start + slice.sweep / 2.0).sin() > 0.0 && y > oy + ry * 0.9 { band } else { 0.0 };
                texts.push((text, x, y + below, anchor));
            }
        }
    }
    let size = p.label_size;
    for (text, x, y, anchor) in texts {
        p.text(&text, x, y, size, TEXT, anchor);
    }
}
