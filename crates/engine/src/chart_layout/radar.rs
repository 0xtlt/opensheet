//! Radar charts: one spoke per category, the values measured from the
//! centre, each series a closed line (filled or not) around it.

use super::scale::Scale;
use super::{GRIDLINE, Ink, Painter, Stroke, TEXT, TextAnchor, fmt_value, label_text};
use crate::chart::ChartKind;
use std::f32::consts::{FRAC_PI_2, TAU};

pub(super) fn draw_radar(p: &mut Painter, area: (f32, f32, f32, f32)) {
    let chart = p.chart;
    let data = p.data;
    let z = p.z;
    let (left, top, right, bottom) = area;
    let label_size = p.label_size;
    let label_h = p.label_h;
    let count = data.series.len().min(chart.series.len());
    let n = data.series.iter().take(count).map(|s| s.values.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }
    let (mut lo, mut hi) = (0.0f64, f64::NEG_INFINITY);
    for v in data.series.iter().take(count).flat_map(|s| s.values.iter().flatten()) {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    if !hi.is_finite() {
        return;
    }
    let scale = Scale::new(&chart.axes.value, lo, hi, false);
    let show_categories = chart.axes.category.visible;
    let names: Vec<(String, f32)> = (0..n)
        .map(|k| {
            let text = if show_categories { data.categories.get(k).cloned().unwrap_or_default() } else { String::new() };
            let w = p.width(&text, label_size);
            (text, w)
        })
        .collect();
    let widest = names.iter().map(|(_, w)| *w).fold(0.0, f32::max);
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0;
    let r = ((bottom - top) / 2.0 - label_h - 6.0 * z)
        .min((right - left) / 2.0 - widest - 8.0 * z)
        .max(8.0);
    // The first category points up; the others follow clockwise.
    let angle = |k: usize| -FRAC_PI_2 + TAU * k as f32 / n as f32;
    let at = |k: usize, f: f64| {
        let a = angle(k);
        (cx + r * f as f32 * a.cos(), cy + r * f as f32 * a.sin())
    };

    // Rings and spokes.
    let grid = Stroke::new(GRIDLINE, 1.0);
    let ticks = scale.major_ticks();
    if chart.axes.value.major_gridlines {
        for v in &ticks {
            let f = scale.clamped(*v);
            if f <= 0.0 {
                continue;
            }
            let ring: Vec<(f32, f32)> = (0..n).map(|k| at(k, f)).collect();
            if n >= 3 {
                p.polygon(&ring, None, Some(grid));
            } else {
                p.ellipse(cx, cy, r * f as f32, r * f as f32, None, Some(grid));
            }
        }
    }
    if show_categories {
        for k in 0..n {
            let (x, y) = at(k, 1.0);
            p.line(cx, cy, x, y, grid);
        }
    }

    // Series.
    let mut texts: Vec<(String, f32, f32, TextAnchor)> = Vec::new();
    for si in 0..count {
        let look = p.look(si);
        let sd = &data.series[si];
        let points: Vec<Option<(f32, f32)>> =
            (0..n).map(|k| sd.values.get(k).copied().flatten().map(|v| at(k, scale.clamped(v)))).collect();
        let all: Vec<(f32, f32)> = points.iter().flatten().copied().collect();
        if look.kind == ChartKind::RadarFilled {
            p.polygon(&all, Some(Ink::solid(look.color)), None);
        } else if let Some(stroke) = look.line {
            if all.len() == n && n >= 3 {
                p.polygon(&all, None, Some(stroke));
            } else {
                // A missing value opens the line.
                let mut run: Vec<(f32, f32)> = Vec::new();
                for pt in &points {
                    match pt {
                        Some(pt) => run.push(*pt),
                        None => {
                            p.polyline(&run, stroke);
                            run.clear();
                        }
                    }
                }
                p.polyline(&run, stroke);
                // Back to the first point, when both ends are there.
                if let (Some(Some(last)), Some(Some(first))) = (points.last(), points.first())
                    && n >= 3
                {
                    p.polyline(&[*last, *first], stroke);
                }
            }
        }
        if let Some(marker) = &look.marker {
            for (x, y) in &all {
                p.marker(marker, *x, *y);
            }
        }
        if let Some(labels) = chart.labels_for(&chart.series[si]) {
            for (k, pt) in points.iter().enumerate() {
                let Some((x, y)) = pt else { continue };
                let text = label_text(p, si, k, None, labels);
                if !text.is_empty() {
                    texts.push((text, *x + 4.0 * z, *y - label_h, TextAnchor::Start));
                }
            }
        }
    }

    // Values up the first spoke, categories around the outside.
    if chart.axes.value.visible {
        let code = chart.axes.value.number_format.as_deref().unwrap_or(&data.value_format);
        let divisor = chart.axes.value.display_unit.divisor();
        for v in &ticks {
            let label = fmt_value(*v / divisor, code, p.options.locale);
            let y = (cy - r * scale.clamped(*v) as f32).round();
            p.text(&label, cx - 4.0 * z, y - label_h / 2.0, label_size, TEXT, TextAnchor::End);
        }
    }
    for (k, (name, _)) in names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }
        let a = angle(k);
        let (sin, cos) = a.sin_cos();
        let x = cx + (r + 6.0 * z) * cos;
        let y = cy + (r + 6.0 * z) * sin;
        let anchor = if cos > 0.2 {
            TextAnchor::Start
        } else if cos < -0.2 {
            TextAnchor::End
        } else {
            TextAnchor::Middle
        };
        // Above the top spoke, under the bottom one, level with the others.
        let top = y - label_h / 2.0 + sin * label_h / 2.0;
        p.text(name, x, top, label_size, TEXT, anchor);
    }
    for (text, x, y, anchor) in texts {
        p.text(&text, x, y, label_size, TEXT, anchor);
    }
}
