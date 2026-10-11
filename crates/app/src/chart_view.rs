//! Drawing charts over the grid: columns, bars, lines, areas, pies and
//! scatter plots, with title, axes, gridlines and legend laid out the way
//! Excel's default chart style does.

use crate::grid::{Draw, GridLayout};
use crate::theme::{self, c};
use gpui::{
    BorderStyle, Bounds, Font, FontStyle, FontWeight, Hsla, PathBuilder, Pixels, Point, ShapedLine, SharedString,
    TextRun, WindowTextSystem, fill, px, quad, size,
};
use opensheet_engine::chart::{AxisScale, Chart, ChartData, ChartKind, ChartTitle, LegendPos, nice_axis, palette_color};
use opensheet_engine::{Locale, format};

const TEXT: u32 = 0x595959;
const GRIDLINE: u32 = 0xD9D9D9;
const AXIS: u32 = 0xBFBFBF;

pub struct ChartStyle<'a> {
    /// Font family for every label.
    pub font: SharedString,
    /// Shown when the chart has an automatic title and several series.
    pub placeholder_title: &'a str,
    /// Prefix of unnamed series ("Series" → "Series1").
    pub series_prefix: &'a str,
    pub locale: &'static Locale,
}

/// Pixel box (x0, y0, x1, y1) of a chart in grid-element coordinates.
pub fn chart_rect(layout: &GridLayout, chart: &Chart) -> (f32, f32, f32, f32) {
    let z = layout.zoom;
    let x0 = layout.col_x(chart.anchor.1) + chart.offset.0 * z;
    let y0 = layout.row_y(chart.anchor.0) + chart.offset.1 * z;
    (x0, y0, x0 + chart.size.0 * z, y0 + chart.size.1 * z)
}

/// The eight resize handles: corners and edge midpoints, clockwise from the
/// top-left corner.
pub fn handle_points(rect: (f32, f32, f32, f32)) -> [(f32, f32); 8] {
    let (x0, y0, x1, y1) = rect;
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    [
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ]
}

struct Painter<'a> {
    layout: &'a GridLayout,
    ts: &'a WindowTextSystem,
    items: Vec<Draw>,
    clip: Bounds<Pixels>,
    family: SharedString,
}

impl Painter<'_> {
    fn pt(&self, x: f32, y: f32) -> Point<Pixels> {
        self.layout.to_window(x, y)
    }

    fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds<Pixels> {
        Bounds::new(self.pt(x0, y0), size(px((x1 - x0).max(0.0)), px((y1 - y0).max(0.0))))
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) {
        let (x0, x1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (y0, y1) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        self.items.push(Draw::Quad(fill(self.bounds(x0, y0, x1, y1), color)));
    }

    fn frame(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) {
        self.items.push(Draw::Quad(quad(
            self.bounds(x0, y0, x1, y1),
            px(0.),
            gpui::transparent_black(),
            px(1.),
            color,
            BorderStyle::Solid,
        )));
    }

    fn polygon(&mut self, points: &[(f32, f32)], color: Hsla) {
        if points.len() < 3 {
            return;
        }
        let pts: Vec<Point<Pixels>> = points.iter().map(|(x, y)| self.pt(*x, *y)).collect();
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&pts, true);
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Hsla) {
        if points.len() < 2 {
            return;
        }
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(self.pt(points[0].0, points[0].1));
        for (x, y) in &points[1..] {
            builder.line_to(self.pt(*x, *y));
        }
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, color: Hsla) {
        let n = 20;
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.polygon(&pts, color);
    }

    fn shape(&self, text: &str, size: f32, color: Hsla, bold: bool) -> ShapedLine {
        let text: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: text.len(),
            font: Font {
                family: self.family.clone(),
                features: Default::default(),
                fallbacks: None,
                weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
                style: FontStyle::Normal,
            },
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.ts.shape_line(text, px(size), &[run], None)
    }

    /// Draw a shaped line with its top-left corner at (x, y).
    fn put(&mut self, line: ShapedLine, x: f32, y: f32, size: f32) {
        self.items.push(Draw::Text {
            line,
            origin: self.pt(x.round(), y.round()),
            line_height: px((size * 1.25).round()),
            clip: self.clip,
        });
    }

    /// Draw text, returning its width.
    fn label(&mut self, text: &str, x: f32, y: f32, size: f32, color: Hsla) -> f32 {
        let line = self.shape(text, size, color, false);
        let w = f32::from(line.width);
        self.put(line, x, y, size);
        w
    }
}

fn fmt_value(v: f64, code: &str, locale: &Locale) -> String {
    if code.eq_ignore_ascii_case("General") {
        format::format_general(v, locale, 11)
    } else {
        format::format_number(v, code, locale).text
    }
}

fn series_name(data: &ChartData, i: usize, style: &ChartStyle) -> String {
    let name = &data.series[i].name;
    if name.is_empty() {
        format!("{}{}", style.series_prefix, i + 1)
    } else {
        name.clone()
    }
}

/// Build the drawing commands of one chart.
pub fn build_chart(
    chart: &Chart,
    data: &ChartData,
    layout: &GridLayout,
    ts: &WindowTextSystem,
    style: &ChartStyle,
    selected: bool,
) -> Vec<Draw> {
    let (x0, y0, x1, y1) = chart_rect(layout, chart);
    let z = layout.zoom;
    let mut p = Painter {
        layout,
        ts,
        items: Vec::new(),
        clip: Bounds::new(layout.to_window(x0, y0), size(px(x1 - x0), px(y1 - y0))),
        family: style.font.clone(),
    };
    p.rect(x0, y0, x1, y1, c(0xFFFFFF));
    p.frame(x0, y0, x1, y1, c(GRIDLINE));

    let text = c(TEXT);
    let label_size = (12.0 * z).max(6.0);
    let label_h = (label_size * 1.25).round();
    let pad = (10.0 * z).round();
    let (mut left, mut top, mut right, mut bottom) = (x0 + pad, y0 + pad, x1 - pad, y1 - pad);

    // Title.
    let title = match (&chart.title, &data.title) {
        (ChartTitle::None, _) => None,
        (_, Some(t)) => Some(t.clone()),
        _ => Some(style.placeholder_title.to_string()),
    };
    if let Some(title) = title {
        let size_px = (18.67 * z).max(8.0);
        let line = p.shape(&title, size_px, text, false);
        let w = f32::from(line.width);
        p.put(line, x0 + ((x1 - x0 - w) / 2.0).max(4.0), top - 2.0 * z, size_px);
        top += (size_px * 1.25).round() + 4.0 * z;
    }

    // Legend entries: categories for a pie, series otherwise.
    let entries: Vec<(String, Hsla)> = if chart.kind.is_round() {
        let n = data.series.first().map(|s| s.values.len()).unwrap_or(0);
        (0..n)
            .map(|i| {
                (
                    data.categories.get(i).cloned().unwrap_or_default(),
                    theme::engine_color(palette_color(i)),
                )
            })
            .collect()
    } else {
        (0..data.series.len())
            .map(|i| (series_name(data, i, style), theme::engine_color(data.series[i].color)))
            .collect()
    };
    let swatch = (7.0 * z).round().max(4.0);
    let is_line = matches!(chart.kind, ChartKind::Line | ChartKind::Scatter);
    if chart.legend != LegendPos::None && !entries.is_empty() {
        let widths: Vec<f32> = entries
            .iter()
            .map(|(name, _)| f32::from(p.shape(name, label_size, text, false).width))
            .collect();
        let entry_gap = 14.0 * z;
        let mark_w = if is_line { 18.0 * z } else { swatch };
        let draw_entry = |p: &mut Painter, i: usize, x: f32, y: f32| {
            let (name, color) = &entries[i];
            let cy = y + label_h / 2.0;
            if is_line {
                p.rect(x, cy - 1.0 * z, x + mark_w, cy + 1.0 * z, *color);
                if chart.kind == ChartKind::Scatter {
                    p.disc(x + mark_w / 2.0, cy, 3.0 * z, *color);
                }
            } else {
                p.rect(x, cy - swatch / 2.0, x + swatch, cy + swatch / 2.0, *color);
            }
            p.label(name, x + mark_w + 4.0 * z, y, label_size, text);
        };
        match chart.legend {
            LegendPos::Bottom | LegendPos::Top => {
                // Entries flow onto as many centred rows as they need.
                let available = (x1 - x0 - 2.0 * pad).max(40.0);
                let entry_w = |i: usize| widths[i] + mark_w + 4.0 * z;
                let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
                let mut row_w = 0.0f32;
                for i in 0..entries.len() {
                    let w = entry_w(i);
                    let needed = if rows.last().unwrap().is_empty() { w } else { row_w + entry_gap + w };
                    if needed > available && !rows.last().unwrap().is_empty() {
                        rows.push(vec![i]);
                        row_w = w;
                    } else {
                        rows.last_mut().unwrap().push(i);
                        row_w = needed;
                    }
                }
                // Never let the legend take more than a third of the chart.
                let max_rows = (((y1 - y0) / 3.0) / label_h).floor().max(1.0) as usize;
                rows.truncate(max_rows);
                let block_h = label_h * rows.len() as f32;
                let mut y = if chart.legend == LegendPos::Bottom {
                    bottom -= block_h + 6.0 * z;
                    bottom + 6.0 * z
                } else {
                    let y = top;
                    top += block_h + 6.0 * z;
                    y
                };
                for row in &rows {
                    let total: f32 =
                        row.iter().map(|i| entry_w(*i)).sum::<f32>() + entry_gap * (row.len() as f32 - 1.0);
                    let mut x = x0 + ((x1 - x0 - total) / 2.0).max(pad);
                    for i in row {
                        draw_entry(&mut p, *i, x, y);
                        x += entry_w(*i) + entry_gap;
                    }
                    y += label_h;
                }
            }
            LegendPos::Right | LegendPos::Left => {
                let col_w = widths.iter().cloned().fold(0.0f32, f32::max) + mark_w + 4.0 * z;
                let total_h = label_h * entries.len() as f32;
                let x = if chart.legend == LegendPos::Right {
                    right -= col_w + 8.0 * z;
                    right + 8.0 * z
                } else {
                    let x = left;
                    left += col_w + 8.0 * z;
                    x
                };
                let mut y = top + ((bottom - top - total_h) / 2.0).max(0.0);
                for i in 0..entries.len() {
                    draw_entry(&mut p, i, x, y);
                    y += label_h;
                }
            }
            LegendPos::None => {}
        }
    }

    if right - left > 20.0 && bottom - top > 20.0 {
        if chart.kind.is_round() {
            draw_pie(&mut p, chart, data, (left, top, right, bottom), z, style, label_size);
        } else if chart.kind.is_horizontal() {
            draw_bars(&mut p, chart, data, (left, top, right, bottom), z, style, label_size);
        } else if chart.kind == ChartKind::Scatter {
            draw_scatter(&mut p, chart, data, (left, top, right, bottom), z, style, label_size);
        } else {
            draw_columns(&mut p, chart, data, (left, top, right, bottom), z, style, label_size);
        }
    }

    if selected {
        p.frame(x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0, c(0x8A8A8A));
        // Handles sit on the frame and may poke out of the chart box.
        p.clip = Bounds::new(layout.to_window(x0 - 8.0, y0 - 8.0), size(px(x1 - x0 + 16.0), px(y1 - y0 + 16.0)));
        for (hx, hy) in handle_points((x0, y0, x1, y1)) {
            p.disc(hx, hy, 4.5, c(0x8A8A8A));
            p.disc(hx, hy, 3.5, c(0xFFFFFF));
        }
    }
    p.items
}

/// Smallest and largest plotted value (sums per category when stacked).
fn value_range(chart: &Chart, data: &ChartData) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    if chart.kind.is_stacked() {
        let n = data.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..n {
            let (mut pos, mut neg) = (0.0, 0.0);
            for s in &data.series {
                if let Some(Some(v)) = s.values.get(i) {
                    if *v >= 0.0 {
                        pos += v;
                    } else {
                        neg += v;
                    }
                }
            }
            lo = lo.min(neg);
            hi = hi.max(pos);
        }
    } else {
        for s in &data.series {
            for v in s.values.iter().flatten() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    // Bars and areas grow from zero, so zero is always on the axis.
    if !matches!(chart.kind, ChartKind::Line | ChartKind::Scatter) {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    (lo, hi)
}

/// How many category labels to skip so that they do not run into each other.
fn label_step(widths: &[f32], slot: f32, gap: f32) -> usize {
    let widest = widths.iter().cloned().fold(0.0f32, f32::max);
    if slot <= 0.0 {
        return 1;
    }
    (((widest + gap) / slot).ceil() as usize).max(1)
}

/// Column, stacked column, line and area charts: categories along the
/// bottom, values up the left side.
fn draw_columns(
    p: &mut Painter,
    chart: &Chart,
    data: &ChartData,
    area: (f32, f32, f32, f32),
    z: f32,
    style: &ChartStyle,
    label_size: f32,
) {
    let (left, top, right, bottom) = area;
    let text = c(TEXT);
    let label_h = (label_size * 1.25).round();
    let n = data.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }
    let (lo, hi) = value_range(chart, data);
    let scale = nice_axis(lo, hi);
    let ticks = scale.ticks();
    let tick_labels: Vec<String> = ticks
        .iter()
        .map(|v| fmt_value(*v, &data.value_format, style.locale))
        .collect();
    let tick_w = tick_labels
        .iter()
        .map(|t| f32::from(p.shape(t, label_size, text, false).width))
        .fold(0.0f32, f32::max);

    let px0 = left + tick_w + 8.0 * z;
    let px1 = right;
    let py0 = top + label_h / 2.0;
    let py1 = bottom - label_h - 6.0 * z;
    if px1 - px0 < 10.0 || py1 - py0 < 10.0 {
        return;
    }
    let y_of = |v: f64| py1 - scale.fraction(v) as f32 * (py1 - py0);

    // Horizontal gridlines and value labels.
    for (v, label) in ticks.iter().zip(&tick_labels) {
        let y = y_of(*v).round();
        p.rect(px0, y, px1, y + 1.0, c(GRIDLINE));
        let w = f32::from(p.shape(label, label_size, text, false).width);
        p.label(label, px0 - 6.0 * z - w, y - label_h / 2.0, label_size, text);
    }

    let slot = (px1 - px0) / n as f32;
    let zero_y = y_of(0.0f64.clamp(scale.min, scale.max)).round();
    let ns = data.series.len().max(1) as f32;

    match chart.kind {
        ChartKind::Column => {
            // Gap between clusters 219 % of a bar, bars 27 % apart.
            let bar = slot / (ns + 0.27 * (ns - 1.0) + 2.19);
            let cluster = bar * (ns + 0.27 * (ns - 1.0));
            for i in 0..n {
                let start = px0 + slot * i as f32 + (slot - cluster) / 2.0;
                for (si, s) in data.series.iter().enumerate() {
                    if let Some(Some(v)) = s.values.get(i) {
                        let x = start + si as f32 * bar * 1.27;
                        let y = y_of(*v);
                        p.rect(x, y, x + bar.max(1.0), zero_y, theme::engine_color(s.color));
                        if chart.data_labels {
                            let label = fmt_value(*v, &data.value_format, style.locale);
                            let w = f32::from(p.shape(&label, label_size, text, false).width);
                            let ly = if *v >= 0.0 { y - label_h - 1.0 } else { y + 1.0 };
                            p.label(&label, x + (bar - w) / 2.0, ly, label_size, text);
                        }
                    }
                }
            }
        }
        ChartKind::StackedColumn => {
            let bar = slot / 2.5;
            for i in 0..n {
                let x = px0 + slot * i as f32 + (slot - bar) / 2.0;
                let (mut pos, mut neg) = (0.0f64, 0.0f64);
                for s in &data.series {
                    if let Some(Some(v)) = s.values.get(i) {
                        let (from, to) = if *v >= 0.0 {
                            let from = pos;
                            pos += v;
                            (from, pos)
                        } else {
                            let from = neg;
                            neg += v;
                            (from, neg)
                        };
                        p.rect(x, y_of(to), x + bar, y_of(from), theme::engine_color(s.color));
                    }
                }
            }
        }
        ChartKind::Area => {
            for s in &data.series {
                let mut pts: Vec<(f32, f32)> = Vec::new();
                for i in 0..n {
                    let x = if n == 1 {
                        (px0 + px1) / 2.0
                    } else {
                        px0 + (px1 - px0) * i as f32 / (n as f32 - 1.0)
                    };
                    let v = s.values.get(i).copied().flatten().unwrap_or(0.0);
                    pts.push((x, y_of(v)));
                }
                if let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) {
                    let mut poly = vec![(first.0, zero_y)];
                    poly.extend(pts.iter().copied());
                    poly.push((last.0, zero_y));
                    p.polygon(&poly, theme::engine_color(s.color));
                }
            }
        }
        _ => {
            // Line: 2.25 pt strokes through the middle of each slot; a gap
            // in the data breaks the line.
            for s in &data.series {
                let color = theme::engine_color(s.color);
                let mut run: Vec<(f32, f32)> = Vec::new();
                for i in 0..n {
                    match s.values.get(i).copied().flatten() {
                        Some(v) => run.push((px0 + slot * (i as f32 + 0.5), y_of(v))),
                        None => {
                            if run.len() == 1 {
                                p.disc(run[0].0, run[0].1, 2.0 * z, color);
                            }
                            p.polyline(&run, (3.0 * z).max(1.5), color);
                            run.clear();
                        }
                    }
                }
                if run.len() == 1 {
                    p.disc(run[0].0, run[0].1, 2.0 * z, color);
                }
                p.polyline(&run, (3.0 * z).max(1.5), color);
                if chart.data_labels {
                    for i in 0..n {
                        if let Some(v) = s.values.get(i).copied().flatten() {
                            let label = fmt_value(v, &data.value_format, style.locale);
                            let w = f32::from(p.shape(&label, label_size, text, false).width);
                            p.label(
                                &label,
                                px0 + slot * (i as f32 + 0.5) - w / 2.0,
                                y_of(v) - label_h - 3.0 * z,
                                label_size,
                                text,
                            );
                        }
                    }
                }
            }
        }
    }

    // Category axis and labels.
    p.rect(px0, zero_y, px1, zero_y + 1.0, c(AXIS));
    let widths: Vec<f32> = data
        .categories
        .iter()
        .take(n)
        .map(|t| f32::from(p.shape(t, label_size, text, false).width))
        .collect();
    let step = label_step(&widths, slot, 6.0 * z);
    for i in (0..n).step_by(step) {
        if let (Some(label), Some(w)) = (data.categories.get(i), widths.get(i)) {
            let cx = if chart.kind == ChartKind::Area && n > 1 {
                px0 + (px1 - px0) * i as f32 / (n as f32 - 1.0)
            } else {
                px0 + slot * (i as f32 + 0.5)
            };
            let x = (cx - w / 2.0).clamp(px0 - tick_w, px1 - w);
            p.label(label, x, py1 + 5.0 * z, label_size, text);
        }
    }
}

/// Horizontal bars: categories up the left side (first at the bottom, as in
/// Excel), values along the bottom.
fn draw_bars(
    p: &mut Painter,
    chart: &Chart,
    data: &ChartData,
    area: (f32, f32, f32, f32),
    z: f32,
    style: &ChartStyle,
    label_size: f32,
) {
    let (left, top, right, bottom) = area;
    let text = c(TEXT);
    let label_h = (label_size * 1.25).round();
    let n = data.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }
    let (lo, hi) = value_range(chart, data);
    let scale = nice_axis(lo, hi);
    let ticks = scale.ticks();
    let cat_widths: Vec<f32> = data
        .categories
        .iter()
        .take(n)
        .map(|t| f32::from(p.shape(t, label_size, text, false).width))
        .collect();
    let cat_w = cat_widths.iter().cloned().fold(0.0f32, f32::max).min((right - left) * 0.4);

    let px0 = left + cat_w + 8.0 * z;
    let px1 = right - 8.0 * z;
    let py0 = top;
    let py1 = bottom - label_h - 6.0 * z;
    if px1 - px0 < 10.0 || py1 - py0 < 10.0 {
        return;
    }
    let x_of = |v: f64| px0 + scale.fraction(v) as f32 * (px1 - px0);

    for v in &ticks {
        let x = x_of(*v).round();
        p.rect(x, py0, x + 1.0, py1, c(GRIDLINE));
        let label = fmt_value(*v, &data.value_format, style.locale);
        let w = f32::from(p.shape(&label, label_size, text, false).width);
        p.label(&label, x - w / 2.0, py1 + 5.0 * z, label_size, text);
    }

    let slot = (py1 - py0) / n as f32;
    let zero_x = x_of(0.0f64.clamp(scale.min, scale.max)).round();
    let ns = data.series.len().max(1) as f32;
    for i in 0..n {
        // Category 0 sits at the bottom.
        let slot_top = py1 - slot * (i as f32 + 1.0);
        if chart.kind == ChartKind::StackedBar {
            let bar = slot / 2.5;
            let y = slot_top + (slot - bar) / 2.0;
            let (mut pos, mut neg) = (0.0f64, 0.0f64);
            for s in &data.series {
                if let Some(Some(v)) = s.values.get(i) {
                    let (from, to) = if *v >= 0.0 {
                        let from = pos;
                        pos += v;
                        (from, pos)
                    } else {
                        let from = neg;
                        neg += v;
                        (from, neg)
                    };
                    p.rect(x_of(from), y, x_of(to), y + bar, theme::engine_color(s.color));
                }
            }
        } else {
            // Gap between clusters 182 % of a bar, as in Excel's bar charts.
            let bar = slot / (ns + 1.82);
            let cluster = bar * ns;
            let start = slot_top + (slot - cluster) / 2.0;
            for (si, s) in data.series.iter().enumerate() {
                if let Some(Some(v)) = s.values.get(i) {
                    // The first series is drawn lowest within the cluster.
                    let y = start + (ns - 1.0 - si as f32) * bar;
                    p.rect(zero_x, y, x_of(*v), y + bar.max(1.0), theme::engine_color(s.color));
                    if chart.data_labels {
                        let label = fmt_value(*v, &data.value_format, style.locale);
                        p.label(&label, x_of(*v) + 3.0 * z, y + (bar - label_h) / 2.0, label_size, text);
                    }
                }
            }
        }
        if let (Some(label), Some(w)) = (data.categories.get(i), cat_widths.get(i)) {
            let line = p.shape(label, label_size, text, false);
            let x = px0 - 6.0 * z - w.min(cat_w);
            p.put(line, x, slot_top + (slot - label_h) / 2.0, label_size);
        }
    }
    p.rect(zero_x, py0, zero_x + 1.0, py1, c(AXIS));
}

fn draw_scatter(
    p: &mut Painter,
    chart: &Chart,
    data: &ChartData,
    area: (f32, f32, f32, f32),
    z: f32,
    style: &ChartStyle,
    label_size: f32,
) {
    let (left, top, right, bottom) = area;
    let text = c(TEXT);
    let label_h = (label_size * 1.25).round();
    let (lo, hi) = value_range(chart, data);
    let y_scale = nice_axis(lo, hi);
    let xs: Vec<f64> = data.x_values.iter().flatten().copied().collect();
    if xs.is_empty() {
        return;
    }
    let x_lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_hi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let x_scale: AxisScale = nice_axis(x_lo, x_hi);

    let y_ticks = y_scale.ticks();
    let y_labels: Vec<String> = y_ticks
        .iter()
        .map(|v| fmt_value(*v, &data.value_format, style.locale))
        .collect();
    let tick_w = y_labels
        .iter()
        .map(|t| f32::from(p.shape(t, label_size, text, false).width))
        .fold(0.0f32, f32::max);
    let px0 = left + tick_w + 8.0 * z;
    let px1 = right - 8.0 * z;
    let py0 = top + label_h / 2.0;
    let py1 = bottom - label_h - 6.0 * z;
    if px1 - px0 < 10.0 || py1 - py0 < 10.0 {
        return;
    }
    let x_of = |v: f64| px0 + x_scale.fraction(v) as f32 * (px1 - px0);
    let y_of = |v: f64| py1 - y_scale.fraction(v) as f32 * (py1 - py0);

    for (v, label) in y_ticks.iter().zip(&y_labels) {
        let y = y_of(*v).round();
        p.rect(px0, y, px1, y + 1.0, c(GRIDLINE));
        let w = f32::from(p.shape(label, label_size, text, false).width);
        p.label(label, px0 - 6.0 * z - w, y - label_h / 2.0, label_size, text);
    }
    for v in x_scale.ticks() {
        let x = x_of(v).round();
        p.rect(x, py0, x + 1.0, py1, c(GRIDLINE));
        let label = fmt_value(v, &data.x_format, style.locale);
        let w = f32::from(p.shape(&label, label_size, text, false).width);
        p.label(&label, x - w / 2.0, py1 + 5.0 * z, label_size, text);
    }
    p.rect(px0, py1, px1, py1 + 1.0, c(AXIS));
    p.rect(px0, py0, px0 + 1.0, py1, c(AXIS));

    for s in &data.series {
        let color = theme::engine_color(s.color);
        for (i, v) in s.values.iter().enumerate() {
            if let (Some(y), Some(Some(x))) = (v, data.x_values.get(i)) {
                p.disc(x_of(*x), y_of(*y), (3.5 * z).max(2.0), color);
            }
        }
    }
}

fn draw_pie(
    p: &mut Painter,
    chart: &Chart,
    data: &ChartData,
    area: (f32, f32, f32, f32),
    z: f32,
    style: &ChartStyle,
    label_size: f32,
) {
    let (left, top, right, bottom) = area;
    let Some(series) = data.series.first() else {
        return;
    };
    let values: Vec<f64> = series.values.iter().map(|v| v.unwrap_or(0.0).max(0.0)).collect();
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return;
    }
    let cx = (left + right) / 2.0;
    let cy = (top + bottom) / 2.0;
    let r = ((right - left).min(bottom - top) / 2.0 - 4.0 * z).max(4.0);
    let text = c(0xFFFFFF);
    let label_h = (label_size * 1.25).round();

    // Slices start at 12 o'clock and run clockwise.
    let mut angle = -std::f32::consts::FRAC_PI_2;
    let mut edges: Vec<f32> = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let sweep = (*v / total) as f32 * std::f32::consts::TAU;
        if sweep <= 0.0 {
            continue;
        }
        let steps = ((sweep / 0.04).ceil() as usize).max(2);
        let mut pts = vec![(cx, cy)];
        for k in 0..=steps {
            let a = angle + sweep * k as f32 / steps as f32;
            pts.push((cx + r * a.cos(), cy + r * a.sin()));
        }
        p.polygon(&pts, theme::engine_color(palette_color(i)));
        edges.push(angle);
        if chart.data_labels && sweep > 0.25 {
            let mid = angle + sweep / 2.0;
            let label = fmt_value(*v, &data.value_format, style.locale);
            let w = f32::from(p.shape(&label, label_size, text, false).width);
            let lr = r * if chart.kind == ChartKind::Doughnut { 0.8 } else { 0.65 };
            p.label(
                &label,
                cx + lr * mid.cos() - w / 2.0,
                cy + lr * mid.sin() - label_h / 2.0,
                label_size,
                text,
            );
        }
        angle += sweep;
    }
    // White separators between slices.
    if edges.len() > 1 {
        for a in edges {
            p.polyline(&[(cx, cy), (cx + r * a.cos(), cy + r * a.sin())], (1.5 * z).max(1.0), c(0xFFFFFF));
        }
    }
    if chart.kind == ChartKind::Doughnut {
        p.disc(cx, cy, r * 0.6, c(0xFFFFFF));
    }
}
