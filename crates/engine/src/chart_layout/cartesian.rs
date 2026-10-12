//! Flat charts with two axes: columns, bars, lines, areas and their
//! combinations on a category axis, scatter and bubble charts on two
//! scales; with secondary axes, trendlines, error bars and data labels.

use super::scale::{Fit, Scale};
use super::{
    AXIS, AxisId, ChartElement, GRIDLINE, Group, Ink, MINOR_GRIDLINE, PT, Painter, Stroke, TEXT, TextAnchor, clip_polygon,
    clip_polyline, fmt_value, groups, label_text, line_height, smooth_points,
};
use crate::chart::{
    Axis, AxisCrossing, BlanksAs, Chart, ChartData, DataLabels, DisplayUnit, ErrorAmount, ErrorBars, ErrorDirection, Family, Fill,
    Grouping, LabelPosition, TickLabels, TickMark, TrendKind,
};
use crate::chart_trend::{self, EquationPart};
use crate::style::Color;

/// Where the points of a series sit on the value axis: `to` is the
/// plotted value (the running total when stacked), `from` where its bar or
/// band starts (`None` = at the category axis).
pub(super) struct Shape {
    pub from: Vec<Option<f64>>,
    pub to: Vec<Option<f64>>,
}

/// Where every series sits on its value axis: stacked series are piled
/// up, shares of 100 % are worked out, blanks are filled in as the chart
/// asks.
pub(super) fn shapes(chart: &Chart, data: &ChartData, groups: &[Group], n: usize) -> Vec<Option<Shape>> {
    let blank = |v: Option<f64>, family: Family| match v {
        Some(v) => Some(v),
        None if family == Family::Area => Some(0.0),
        None if chart.blanks == BlanksAs::Zero && family != Family::Column && family != Family::Bar => Some(0.0),
        None => None,
    };
    let mut shapes: Vec<Option<Shape>> = (0..chart.series.len()).map(|_| None).collect();
    for g in groups {
        let family = g.kind.family();
        let value = |si: usize, k: usize| blank(data.series[si].values.get(k).copied().flatten(), family);
        match g.kind.grouping() {
            Grouping::Standard => {
                for &si in &g.series {
                    shapes[si] = Some(Shape {
                        from: vec![None; n],
                        to: (0..n).map(|k| value(si, k)).collect(),
                    });
                }
            }
            grouping => {
                let totals: Vec<f64> =
                    (0..n).map(|k| g.series.iter().filter_map(|&si| value(si, k)).map(f64::abs).sum()).collect();
                let (mut up, mut down) = (vec![0.0f64; n], vec![0.0f64; n]);
                // Bars pile positive values upwards and negative ones
                // downwards; lines and areas simply add up.
                let two_way = matches!(family, Family::Column | Family::Bar);
                for &si in &g.series {
                    let mut shape = Shape {
                        from: vec![None; n],
                        to: vec![None; n],
                    };
                    for k in 0..n {
                        let Some(v) = value(si, k) else { continue };
                        let v = match grouping {
                            Grouping::Percent if totals[k] > 0.0 => v / totals[k],
                            Grouping::Percent => 0.0,
                            _ => v,
                        };
                        let sum = if v < 0.0 && two_way { &mut down[k] } else { &mut up[k] };
                        shape.from[k] = Some(*sum);
                        *sum += v;
                        shape.to[k] = Some(*sum);
                    }
                    shapes[si] = Some(shape);
                }
            }
        }
    }
    shapes
}

/// A trendline ready to draw, in the units of the axes: category numbers
/// (1 = the first) or X values, and values.
struct Trend {
    series: usize,
    /// Position among the trendlines of its series.
    index: usize,
    stroke: Stroke,
    points: Vec<(f64, f64)>,
    label: Vec<Vec<EquationPart>>,
}

/// The plot rectangle and how categories and values map into it.
struct Geo {
    px0: f32,
    py0: f32,
    px1: f32,
    py1: f32,
    /// Bars run sideways: the category axis is the vertical one.
    horizontal: bool,
    /// Category slots (categories plus the room of forecasts).
    slots: usize,
    /// Points sit on the tick marks (area charts) instead of between them.
    on_ticks: bool,
    reversed: bool,
}

impl Geo {
    fn span(&self) -> f64 {
        if self.on_ticks { self.slots.saturating_sub(1).max(1) as f64 } else { self.slots.max(1) as f64 }
    }

    /// Fraction along the category axis of a position counted in slots.
    fn cat(&self, c: f64) -> f64 {
        let u = c / self.span();
        if self.reversed { 1.0 - u } else { u }
    }

    /// Slot position of the centre of category `k`.
    fn centre(&self, k: usize) -> f64 {
        if self.on_ticks { k as f64 } else { k as f64 + 0.5 }
    }

    /// The pixel at fraction `u` of the category axis (or X scale) and
    /// fraction `f` of the value axis.
    fn at(&self, u: f64, f: f64) -> (f32, f32) {
        let (w, h) = (self.px1 - self.px0, self.py1 - self.py0);
        if self.horizontal {
            (self.px0 + f as f32 * w, self.py1 - u as f32 * h)
        } else {
            (self.px0 + u as f32 * w, self.py1 - f as f32 * h)
        }
    }

    /// Length of a slot in pixels.
    fn slot(&self) -> f32 {
        let len = if self.horizontal { self.py1 - self.py0 } else { self.px1 - self.px0 };
        len / self.span() as f32
    }

    fn rect(&self) -> (f32, f32, f32, f32) {
        (self.px0, self.py0, self.px1, self.py1)
    }

    fn inside(&self, x: f32, y: f32) -> bool {
        x >= self.px0 - 0.5 && x <= self.px1 + 0.5 && y >= self.py0 - 0.5 && y <= self.py1 + 0.5
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    /// Left of the plot, or under it.
    Low,
    /// Right of the plot, or above it.
    High,
    /// Next to an axis that crosses the plot.
    Inside,
}

/// An axis as it will be drawn: labels with their place along it, the side
/// of the plot they are written on, the title.
struct AxisPlan {
    /// The axis runs up the plot (its labels sit beside it).
    vertical: bool,
    side: Side,
    /// (fraction along the axis, text, width)
    labels: Vec<(f64, String, f32)>,
    /// Labels name categories: they are centred on slots and thinned out
    /// when they do not fit.
    categories: bool,
    title: Option<String>,
    unit: Option<String>,
    /// Fraction along the other axis where the line of this axis runs.
    line_at: f64,
    draw_line: bool,
    ticks: Vec<f64>,
    tick_mark: TickMark,
    minor_ticks: Vec<f64>,
    minor_mark: TickMark,
}

impl AxisPlan {
    fn label_w(&self) -> f32 {
        self.labels.iter().map(|l| l.2).fold(0.0, f32::max)
    }
}

/// Smallest and largest of a set of numbers.
#[derive(Clone, Copy)]
struct Range {
    lo: f64,
    hi: f64,
}

impl Range {
    fn empty() -> Range {
        Range {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        }
    }

    fn add(&mut self, v: f64) {
        if v.is_finite() {
            self.lo = self.lo.min(v);
            self.hi = self.hi.max(v);
        }
    }

    fn is_empty(&self) -> bool {
        self.lo > self.hi
    }
}

/// How many category labels to skip so that they do not run into each other.
fn label_step(widest: f32, slot: f32, gap: f32) -> usize {
    if slot <= 0.0 {
        return 1;
    }
    (((widest + gap) / slot).ceil() as usize).max(1)
}

fn unit_name(p: &Painter, unit: DisplayUnit) -> Option<String> {
    let names = &p.options.labels.unit_names;
    Some(match unit {
        DisplayUnit::None => return None,
        DisplayUnit::Hundreds => names[0].to_string(),
        DisplayUnit::Thousands => names[1].to_string(),
        DisplayUnit::Millions => names[2].to_string(),
        DisplayUnit::Billions => names[3].to_string(),
        DisplayUnit::Trillions => names[4].to_string(),
        other => format!("x {}", fmt_value(other.divisor(), "General", p.options.locale)),
    })
}

/// The labels of a scale: every major tick, formatted, with its place.
fn scale_labels(p: &Painter, axis: &Axis, scale: &Scale, code: &str) -> Vec<(f64, String, f32)> {
    let divisor = axis.display_unit.divisor();
    let code = axis.number_format.as_deref().unwrap_or(code);
    scale
        .major_ticks()
        .into_iter()
        .map(|v| {
            let text = fmt_value(v / divisor, code, p.options.locale);
            let w = p.width(&text, p.label_size);
            (scale.fraction(v), text, w)
        })
        .collect()
}

/// Which side of the plot the labels of an axis go, given where its line
/// runs along the other axis.
fn label_side(axis: &Axis, line_at: f64) -> Option<Side> {
    if !axis.visible {
        return None;
    }
    match axis.labels {
        TickLabels::None => None,
        TickLabels::Low => Some(Side::Low),
        TickLabels::High => Some(Side::High),
        TickLabels::NextToAxis if line_at <= 0.001 => Some(Side::Low),
        TickLabels::NextToAxis if line_at >= 0.999 => Some(Side::High),
        TickLabels::NextToAxis => Some(Side::Inside),
    }
}

/// Standard deviation of the numbers of a series (of a sample, as STDEV).
fn std_dev(values: &[Option<f64>]) -> (f64, f64, usize) {
    let nums: Vec<f64> = values.iter().flatten().copied().collect();
    let n = nums.len();
    if n < 2 {
        return (nums.first().copied().unwrap_or(0.0), 0.0, n);
    }
    let mean = nums.iter().sum::<f64>() / n as f64;
    let var = nums.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    (mean, var.sqrt(), n)
}

/// The ends of an error bar on a point of value `v`.
fn error_ends(bars: &ErrorBars, values: &[Option<f64>], v: f64) -> (f64, f64) {
    let (centre, amount) = match bars.amount {
        ErrorAmount::Fixed(a) => (v, a.abs()),
        ErrorAmount::Percent(pc) => (v, v.abs() * pc.abs() / 100.0),
        // Around the mean of the series, the same for every point.
        ErrorAmount::StdDev(times) => {
            let (mean, sd, _) = std_dev(values);
            (mean, sd * times.abs())
        }
        ErrorAmount::StdErr => {
            let (_, sd, n) = std_dev(values);
            (v, if n > 0 { sd / (n as f64).sqrt() } else { 0.0 })
        }
    };
    match bars.direction {
        ErrorDirection::Both => (centre - amount, centre + amount),
        ErrorDirection::Plus => (centre, centre + amount),
        ErrorDirection::Minus => (centre - amount, centre),
    }
}

/// A data label waiting to be drawn over everything else.
struct PendingLabel {
    series: usize,
    text: String,
    x: f32,
    top: f32,
    anchor: TextAnchor,
    key: Option<Color>,
}

pub(super) fn draw_cartesian(p: &mut Painter, area: (f32, f32, f32, f32)) {
    let chart = p.chart;
    let data = p.data;
    let z = p.z;
    let locale = p.options.locale;
    let (left, top, right, bottom) = area;
    let label_size = p.label_size;
    let label_h = p.label_h;
    let xy = chart.kind.is_xy();
    let horizontal = chart.kind.is_horizontal();

    let mut groups = groups(chart, data);
    if groups.is_empty() {
        return;
    }
    // Excel paints areas first, then bars, then lines, whatever the order
    // of the series.
    groups.sort_by_key(|g| match g.kind.family() {
        Family::Area => 0,
        Family::Column | Family::Bar => 1,
        _ => 2,
    });
    let n = data.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if n == 0 {
        return;
    }

    let shapes = shapes(chart, data, &groups, n);

    // ---- trendlines --------------------------------------------------------
    let mut trends: Vec<Trend> = Vec::new();
    let mut extra_slots = 0usize;
    for g in &groups {
        if g.kind.is_stacked() {
            continue;
        }
        for &si in &g.series {
            let sd = &data.series[si];
            let pts: Vec<(f64, f64)> = (0..sd.values.len())
                .filter_map(|k| {
                    let y = sd.values[k]?;
                    let x = if xy { sd.x_values.get(k).copied().flatten()? } else { k as f64 + 1.0 };
                    Some((x, y))
                })
                .collect();
            if pts.len() < 2 {
                continue;
            }
            let x_lo = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let x_hi = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
            for (index, t) in chart.series[si].trendlines.iter().enumerate() {
                let stroke = p.trend_stroke(si, &t.line);
                let forward = t.forward.max(0.0);
                // A category axis has half a category before the first point.
                let backward = if xy { t.backward.max(0.0) } else { t.backward.clamp(0.0, 0.5) };
                let mut label = Vec::new();
                let points = match t.kind {
                    TrendKind::MovingAverage(period) => chart_trend::moving_average(&pts, period),
                    kind => {
                        let Some(fit) = chart_trend::fit(kind, &pts, t.intercept) else { continue };
                        if t.show_equation {
                            label.push(fit.equation(locale));
                        }
                        if t.show_r2 {
                            label.push(fit.r_squared_label(locale));
                        }
                        let (from, to) = (x_lo - backward, x_hi + forward);
                        let steps = if kind == TrendKind::Linear { 1 } else { 96 };
                        (0..=steps)
                            .map(|i| from + (to - from) * i as f64 / steps as f64)
                            .map(|x| (x, fit.value_at(x)))
                            .filter(|(_, y)| y.is_finite())
                            .collect()
                    }
                };
                if !xy {
                    extra_slots = extra_slots.max(forward.ceil() as usize);
                }
                if points.len() >= 2 {
                    trends.push(Trend {
                        series: si,
                        index,
                        stroke,
                        points,
                        label,
                    });
                }
            }
        }
    }
    let trends = if p.trends_possible() { trends } else { Vec::new() };

    // ---- scales ------------------------------------------------------------
    let has_secondary = groups.iter().any(|g| g.secondary);
    let mut ranges = [Range::empty(), Range::empty()];
    let mut percent = [true, true];
    for g in &groups {
        let range = &mut ranges[g.secondary as usize];
        percent[g.secondary as usize] &= g.kind.is_percent();
        for &si in &g.series {
            let Some(shape) = &shapes[si] else { continue };
            for k in 0..n {
                if let Some(v) = shape.to[k] {
                    range.add(v);
                }
                if let Some(v) = shape.from[k] {
                    range.add(v);
                }
            }
            for bars in chart.series[si].error_bars.iter().filter(|b| !b.horizontal) {
                for v in data.series[si].values.iter().flatten() {
                    let (lo, hi) = error_ends(bars, &data.series[si].values, *v);
                    range.add(lo);
                    range.add(hi);
                }
            }
        }
        // Bars and areas grow from zero, so zero is always on the axis.
        if matches!(g.kind.family(), Family::Column | Family::Bar | Family::Area) {
            range.add(0.0);
        }
    }
    for t in &trends {
        let range = &mut ranges[chart.on_secondary(&chart.series[t.series]) as usize];
        for (_, y) in &t.points {
            range.add(*y);
        }
    }
    let mut x_range = Range::empty();
    let bubble_chart = chart.kind.family() == Family::Bubble;
    if xy {
        for (si, sd) in data.series.iter().enumerate().take(chart.series.len()) {
            for (k, x) in sd.x_values.iter().enumerate() {
                if let (Some(x), Some(Some(_))) = (x, sd.values.get(k)) {
                    x_range.add(*x);
                    for bars in chart.series[si].error_bars.iter().filter(|b| b.horizontal) {
                        let (lo, hi) = error_ends(bars, &sd.x_values, *x);
                        x_range.add(lo);
                        x_range.add(hi);
                    }
                }
            }
        }
        for t in &trends {
            for (x, _) in &t.points {
                x_range.add(*x);
            }
        }
        if x_range.is_empty() {
            return;
        }
        // Bubbles reach beyond their centres: like Excel, both scales
        // get a quarter of the range of the data at each end (at full size).
        if bubble_chart {
            let pad = |r: &mut Range| {
                let d = (r.hi - r.lo) * 0.25 * chart.bubble_scale.min(300) as f64 / 100.0;
                r.lo -= d;
                r.hi += d;
            };
            pad(&mut x_range);
            pad(&mut ranges[0]);
        }
    }
    let value_code = |i: usize| -> String {
        if percent[i] && !xy {
            return "0%".to_string();
        }
        let first = groups.iter().filter(|g| g.secondary as usize == i).flat_map(|g| g.series.iter()).min();
        match (i, first) {
            (1, Some(&si)) => data.series[si].format.clone(),
            _ => data.value_format.clone(),
        }
    };
    // An axis shows as many gridlines as its labels have room for. Its
    // length is not known before its labels are, but nearly: what the
    // other axis and the titles take is.
    let title_size = (13.33 * z).max(6.0);
    let title_h = line_height(title_size);
    let titled = |axis: &Axis, gap: f32| if axis.title.as_ref().is_some_and(|t| !t.is_empty()) { title_h + gap } else { 0.0 };
    let labelled = |axis: &Axis| axis.visible && axis.labels != TickLabels::None;
    let cat_axis = &chart.axes.category;
    let up = if horizontal {
        0.0
    } else {
        let under = match chart.data_table {
            Some(_) if !xy => (label_h + 4.0 * z) * (chart.series.len() as f32 + 1.0) + 2.0 * z,
            _ if labelled(cat_axis) => label_h + 6.0 * z,
            _ => 0.0,
        };
        // The categories written once more above the plot.
        let far = &chart.axes.secondary_category;
        let over = if has_secondary && !xy && far.visible {
            (if far.labels == TickLabels::None { 0.0 } else { label_h + 6.0 * z }) + titled(far, 4.0 * z)
        } else {
            0.0
        };
        bottom - top - label_h / 2.0 - under - over - titled(cat_axis, 4.0 * z)
    };
    let across = if xy {
        right - left - 8.0 * z - (label_size * 1.8 + 8.0 * z) - titled(&chart.axes.value, 6.0 * z)
    } else {
        let names = if labelled(cat_axis) {
            let widest = data.categories.iter().take(n).map(|c| p.width(c, label_size)).fold(0.0, f32::max);
            widest.min((right - left) * 0.4) + 8.0 * z
        } else {
            0.0
        };
        right - left - 8.0 * z - names - titled(cat_axis, 6.0 * z)
    };
    let fitted = |p: &Painter, axis: &Axis, lo: f64, hi: f64, code: &str, upright: bool, percent: bool| -> Scale {
        let fit = if upright {
            Fit::along(up, label_h * 1.25)
        } else {
            // Wide enough for the longest label and some air; Excel keeps
            // the labels of a horizontal scale well apart even when short.
            let divisor = axis.display_unit.divisor();
            let code = axis.number_format.as_deref().unwrap_or(code);
            let widest = [lo, hi].iter().map(|v| p.width(&fmt_value(v / divisor, code, locale), label_size)).fold(0.0, f32::max);
            Fit::along(across, (widest + 12.0 * z).max(34.0 * z))
        };
        Scale::new(
            axis,
            lo,
            hi,
            Fit {
                percent,
                snug: bubble_chart,
                ..fit
            },
        )
    };
    let scale_of = |p: &Painter, i: usize, axis: &Axis| {
        let r = ranges[i];
        let (lo, hi) = if r.is_empty() { (0.0, 1.0) } else { (r.lo, r.hi) };
        fitted(p, axis, lo, hi, &value_code(i), !horizontal, percent[i] && !xy)
    };
    let scales = [scale_of(p, 0, &chart.axes.value), scale_of(p, 1, &chart.axes.secondary_value)];
    let x_scale = xy.then(|| fitted(p, &chart.axes.category, x_range.lo, x_range.hi, &data.x_format, false, false));
    p.scale(AxisId::Value, &scales[0]);
    if has_secondary {
        p.scale(AxisId::SecondaryValue, &scales[1]);
    }
    if let Some(xs) = &x_scale {
        p.scale(AxisId::Category, xs);
    }

    // ---- axes, before the plot rectangle is known --------------------------
    let slots = n + extra_slots;
    let on_ticks = !xy && slots > 1 && groups.iter().all(|g| g.kind.family() == Family::Area);
    let geo0 = Geo {
        px0: 0.0,
        py0: 0.0,
        px1: 1.0,
        py1: 1.0,
        horizontal,
        slots,
        on_ticks,
        reversed: !xy && chart.axes.category.reversed,
    };
    // Where the value axis stands along the category axis, and where the
    // category axis stands along the value axis.
    let value_at = match &x_scale {
        Some(xs) => xs.clamped(xs.crossing(chart.axes.category.crossing)),
        None => match chart.axes.category.crossing {
            AxisCrossing::Max => geo0.cat(geo0.span()),
            AxisCrossing::At(k) if k.is_finite() => geo0.cat((k - 1.0).clamp(0.0, geo0.span())),
            _ => geo0.cat(0.0),
        },
    };
    let base_value = scales[0].crossing(chart.axes.value.crossing);
    let category_at = scales[0].clamped(base_value);
    let base_secondary = scales[1].crossing(chart.axes.secondary_value.crossing);

    let scale_plan = |p: &Painter, axis: &Axis, scale: &Scale, code: &str, vertical: bool, line_at: f64, line: bool| {
        let side = label_side(axis, line_at);
        AxisPlan {
            vertical,
            side: side.unwrap_or(Side::Low),
            labels: if side.is_some() { scale_labels(p, axis, scale, code) } else { Vec::new() },
            categories: false,
            title: axis.title.clone().filter(|t| !t.is_empty()),
            unit: if axis.display_unit_label { unit_name(p, axis.display_unit) } else { None },
            line_at,
            draw_line: line && axis.visible,
            ticks: scale.major_ticks().iter().map(|v| scale.fraction(*v)).collect(),
            tick_mark: if axis.visible { axis.major_ticks } else { TickMark::None },
            minor_ticks: scale.minor_ticks().iter().map(|v| scale.fraction(*v)).collect(),
            minor_mark: if axis.visible { axis.minor_ticks } else { TickMark::None },
        }
    };
    let value_plan = scale_plan(p, &chart.axes.value, &scales[0], &value_code(0), !horizontal, value_at, xy);
    let secondary_plan = has_secondary.then(|| {
        // The secondary scale stands at the other end of the categories.
        let at = if value_at < 0.5 { 1.0 } else { 0.0 };
        scale_plan(p, &chart.axes.secondary_value, &scales[1], &value_code(1), !horizontal, at, false)
    });
    let mut category_plan = match &x_scale {
        Some(xs) => scale_plan(p, &chart.axes.category, xs, &data.x_format, false, category_at, true),
        None => {
            let axis = &chart.axes.category;
            let side = label_side(axis, category_at);
            let labels = if side.is_some() {
                (0..n)
                    .map(|k| {
                        let text = data.categories.get(k).cloned().unwrap_or_default();
                        let w = p.width(&text, label_size);
                        (geo0.cat(geo0.centre(k)), text, w)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let boundaries: Vec<f64> = if on_ticks {
                (0..slots).map(|k| geo0.cat(k as f64)).collect()
            } else {
                (0..=slots).map(|k| geo0.cat(k as f64)).collect()
            };
            AxisPlan {
                vertical: horizontal,
                side: side.unwrap_or(Side::Low),
                labels,
                categories: true,
                title: axis.title.clone().filter(|t| !t.is_empty()),
                unit: None,
                line_at: category_at,
                draw_line: axis.visible,
                ticks: boundaries,
                tick_mark: if axis.visible { axis.major_ticks } else { TickMark::None },
                minor_ticks: Vec::new(),
                minor_mark: TickMark::None,
            }
        }
    };

    // The categories once more, at the far end of the plot, when the
    // secondary category axis is shown.
    let far_plan = (has_secondary && !xy && chart.axes.secondary_category.visible).then(|| AxisPlan {
        side: Side::High,
        title: chart.axes.secondary_category.title.clone().filter(|t| !t.is_empty()),
        line_at: 1.0,
        draw_line: true,
        tick_mark: chart.axes.secondary_category.major_ticks,
        labels: if chart.axes.secondary_category.labels == TickLabels::None {
            Vec::new()
        } else {
            (0..n)
                .map(|k| {
                    let text = data.categories.get(k).cloned().unwrap_or_default();
                    let w = p.width(&text, label_size);
                    (geo0.cat(geo0.centre(k)), text, w)
                })
                .collect()
        },
        vertical: horizontal,
        categories: true,
        unit: None,
        ticks: category_plan.ticks.clone(),
        minor_ticks: Vec::new(),
        minor_mark: TickMark::None,
    });
    // A data table takes the place of the category labels, under an
    // upright plot.
    let table = chart.data_table.filter(|_| !xy && !horizontal);
    let table_rows: Vec<usize> = (0..chart.series.len().min(data.series.len())).collect();
    let row_h = label_h + 4.0 * z;
    let table_h = if table.is_some() { row_h * (table_rows.len() as f32 + 1.0) } else { 0.0 };
    let key_w = if table.is_some_and(|t| t.legend_keys) { (7.0 * z).round().max(4.0) + 4.0 * z } else { 0.0 };
    let names_w = table_rows.iter().map(|si| p.width(&p.series_name(*si), label_size)).fold(0.0, f32::max) + key_w + 8.0 * z;
    if table.is_some() {
        category_plan.labels.clear();
    }

    // ---- room around the plot ----------------------------------------------
    let (mut m_left, mut m_top, mut m_right, mut m_bottom) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let max_cat_w = (right - left) * 0.4;
    let block = |plan: &AxisPlan| -> f32 {
        if plan.labels.is_empty() || plan.side == Side::Inside {
            0.0
        } else if plan.vertical {
            let w = if plan.categories { plan.label_w().min(max_cat_w) } else { plan.label_w() };
            w + 8.0 * z
        } else {
            label_h + 6.0 * z
        }
    };
    let extras = |plan: &AxisPlan| -> f32 {
        let title = if plan.title.is_some() { title_h + if plan.vertical { 6.0 * z } else { 4.0 * z } } else { 0.0 };
        let unit = if plan.unit.is_some() { label_h + 4.0 * z } else { 0.0 };
        title + unit
    };
    for plan in [Some(&value_plan), secondary_plan.as_ref(), Some(&category_plan), far_plan.as_ref()].into_iter().flatten() {
        let need = block(plan) + extras(plan);
        match (plan.vertical, plan.side) {
            (true, Side::High) => m_right += need,
            (true, _) => m_left += need,
            (false, Side::High) => m_top += need,
            (false, _) => m_bottom += need,
        }
        // The first and last labels of a scale stick out of the plot.
        if !plan.categories && !plan.labels.is_empty() {
            if plan.vertical {
                m_top = m_top.max(label_h / 2.0);
            } else {
                m_right = m_right.max(8.0 * z);
            }
        }
    }
    if table.is_some() {
        m_bottom += table_h + 2.0 * z;
        m_left = m_left.max(names_w);
    }
    let geo = Geo {
        px0: left + m_left,
        py0: top + m_top,
        px1: right - m_right,
        py1: bottom - m_bottom,
        ..geo0
    };
    if geo.px1 - geo.px0 < 10.0 || geo.py1 - geo.py0 < 10.0 {
        return;
    }
    let (px0, py0, px1, py1) = geo.rect();

    // ---- plot area, gridlines ----------------------------------------------
    let plot_fill = match chart.plot_area.fill {
        Fill::Solid(color) => Some(Ink::solid(color)),
        _ => None,
    };
    let plot_border = p.stroke(&chart.plot_area.border, false, GRIDLINE, 0.75);
    p.tag(ChartElement::PlotArea);
    p.area(ChartElement::PlotArea, geo.rect());
    p.rect_styled(px0, py0, px1, py1, plot_fill, plot_border);

    // A line across the plot at fraction `f` of an axis that runs up
    // (`vertical`) or along the plot.
    let rule = |p: &mut Painter, element: ChartElement, vertical: bool, f: f64, color: Color| {
        p.tag(element);
        if vertical {
            let y = (py1 - f as f32 * (py1 - py0)).round();
            p.hline(px0, px1, y, color);
            p.anchor(element, px0, y);
            p.anchor(element, px1, y);
        } else {
            let x = (px0 + f as f32 * (px1 - px0)).round();
            p.vline(x, py0, py1, color);
            p.anchor(element, x, py0);
            p.anchor(element, x, py1);
        }
    };
    let mut gridded: Vec<(AxisId, &Axis, &AxisPlan)> =
        vec![(AxisId::Category, &chart.axes.category, &category_plan), (AxisId::Value, &chart.axes.value, &value_plan)];
    if let Some(plan) = &secondary_plan {
        gridded.push((AxisId::SecondaryValue, &chart.axes.secondary_value, plan));
    }
    for (id, axis, plan) in &gridded {
        if axis.minor_gridlines {
            for f in &plan.minor_ticks {
                rule(p, ChartElement::MinorGridlines(*id), plan.vertical, *f, MINOR_GRIDLINE);
            }
        }
    }
    for (id, axis, plan) in &gridded {
        if axis.major_gridlines {
            for f in &plan.ticks {
                rule(p, ChartElement::MajorGridlines(*id), plan.vertical, *f, GRIDLINE);
            }
        }
    }

    // Labels of a scale that runs up the plot are written before the
    // series, as the gridlines are.
    let text = TEXT;

    // ---- series ------------------------------------------------------------
    let mut pending: Vec<PendingLabel> = Vec::new();
    let scale_for = |si: usize| &scales[chart.on_secondary(&chart.series[si]) as usize];
    let base_for = |si: usize| if chart.on_secondary(&chart.series[si]) { base_secondary } else { base_value };
    let push_label = |pending: &mut Vec<PendingLabel>, p: &Painter, si: usize, k: usize, labels: &DataLabels, x: f32, top: f32, anchor: TextAnchor| {
        let label = label_text(p, si, k, None, labels);
        if !label.is_empty() {
            pending.push(PendingLabel {
                series: si,
                text: label,
                x,
                top,
                anchor,
                key: labels.legend_key.then(|| p.point_color(si, k)),
            });
        }
    };

    for g in &groups {
        let family = g.kind.family();
        match family {
            Family::Column | Family::Bar => {
                let ns = g.series.len() as f32;
                let stacked = g.kind.is_stacked();
                let gap = chart.gap_width.unwrap_or(g.kind.default_gap_width()).min(500) as f32 / 100.0;
                let overlap = if stacked { 1.0 } else { chart.overlap_for(g.kind) as f32 / 100.0 };
                let slot = geo.slot();
                // Width of a bar and of its cluster, in slots.
                let cluster_bars = ns - (ns - 1.0) * overlap;
                let bar = 1.0 / (cluster_bars + gap);
                let cluster = bar * cluster_bars;
                for (j, &si) in g.series.iter().enumerate() {
                    let Some(shape) = &shapes[si] else { continue };
                    let look = p.look(si);
                    let scale = scale_for(si);
                    let base = base_for(si);
                    let labels = chart.labels_for(&chart.series[si]).cloned();
                    for k in 0..n {
                        let Some(v) = shape.to[k] else { continue };
                        let from = shape.from[k].unwrap_or(base);
                        let c0 = k as f32 + (1.0 - cluster) / 2.0 + j as f32 * bar * (1.0 - overlap);
                        // A bar is never thinner than a pixel.
                        let c1 = c0 + bar.max(1.0 / slot.max(1.0));
                        let (f0, f1) = (scale.clamped(from), scale.clamped(v));
                        let (mut xa, mut ya) = geo.at(geo.cat(c0 as f64), f0);
                        let (xb, yb) = geo.at(geo.cat(c1 as f64), f1);
                        // The foot of the bar sits on a whole pixel, like
                        // the axis it stands on.
                        if !stacked {
                            if horizontal {
                                xa = xa.round();
                            } else {
                                ya = ya.round();
                            }
                        }
                        let color = p.point_color(si, k);
                        let point = ChartElement::Point(si as u32, k as u32);
                        p.tag(point);
                        p.rect_styled(xa, ya, xb, yb, Some(Ink::solid(color)), look.outline);
                        for (x, y) in [(xa, ya), (xb, ya), (xb, yb), (xa, yb)] {
                            p.anchor(point, x, y);
                        }
                        if let Some(labels) = &labels {
                            let raw = data.series[si].values.get(k).copied().flatten().unwrap_or(0.0);
                            // "Up" is the direction the bar grows on screen.
                            let grows = (raw >= 0.0) != scale.reversed;
                            let position = match labels.position {
                                LabelPosition::Center | LabelPosition::InsideEnd | LabelPosition::InsideBase => labels.position,
                                LabelPosition::OutsideEnd if !stacked => LabelPosition::OutsideEnd,
                                _ if stacked => LabelPosition::Center,
                                _ => LabelPosition::OutsideEnd,
                            };
                            if horizontal {
                                let mid = (ya + yb) / 2.0 - label_h / 2.0;
                                let (x, anchor) = match (position, grows) {
                                    (LabelPosition::OutsideEnd, true) => (xb + 3.0 * z, TextAnchor::Start),
                                    (LabelPosition::OutsideEnd, false) => (xb - 3.0 * z, TextAnchor::End),
                                    (LabelPosition::InsideEnd, true) => (xb - 3.0 * z, TextAnchor::End),
                                    (LabelPosition::InsideEnd, false) => (xb + 3.0 * z, TextAnchor::Start),
                                    (LabelPosition::InsideBase, true) => (xa + 3.0 * z, TextAnchor::Start),
                                    (LabelPosition::InsideBase, false) => (xa - 3.0 * z, TextAnchor::End),
                                    _ => ((xa + xb) / 2.0, TextAnchor::Middle),
                                };
                                push_label(&mut pending, p, si, k, labels, x, mid, anchor);
                            } else {
                                let cx = (xa + xb) / 2.0;
                                let top = match (position, grows) {
                                    (LabelPosition::OutsideEnd, true) => yb - label_h - 1.0,
                                    (LabelPosition::OutsideEnd, false) => yb + 1.0,
                                    (LabelPosition::InsideEnd, true) => yb + 2.0,
                                    (LabelPosition::InsideEnd, false) => yb - label_h - 2.0,
                                    (LabelPosition::InsideBase, true) => ya - label_h - 2.0,
                                    (LabelPosition::InsideBase, false) => ya + 2.0,
                                    _ => (ya + yb) / 2.0 - label_h / 2.0,
                                };
                                push_label(&mut pending, p, si, k, labels, cx, top, TextAnchor::Middle);
                            }
                        }
                    }
                }
            }
            Family::Area => {
                for &si in &g.series {
                    let Some(shape) = &shapes[si] else { continue };
                    let look = p.look(si);
                    let scale = scale_for(si);
                    let base = base_for(si);
                    let mut upper: Vec<(f32, f32)> = Vec::new();
                    let mut lower: Vec<(f32, f32)> = Vec::new();
                    for k in 0..n {
                        let Some(v) = shape.to[k] else { continue };
                        let u = geo.cat(geo.centre(k));
                        upper.push(geo.at(u, scale.fraction(v)));
                        let (x, y) = geo.at(u, scale.fraction(shape.from[k].unwrap_or(base)));
                        // Like the axis, the foot of a plain area sits on a pixel.
                        lower.push(if shape.from[k].is_none() { (x, y.round()) } else { (x, y) });
                    }
                    let mut poly = upper.clone();
                    poly.extend(lower.iter().rev());
                    let poly = clip_polygon(&poly, geo.rect());
                    p.tag(ChartElement::Series(si as u32));
                    p.polygon(&poly, Some(Ink::solid(look.color)), look.outline);
                    for (x, y) in upper.iter().filter(|(x, y)| geo.inside(*x, *y)) {
                        p.anchor(ChartElement::Series(si as u32), *x, *y);
                    }
                    if let Some(labels) = chart.labels_for(&chart.series[si]) {
                        // In the middle of the band, as Excel writes them.
                        let mut at = 0;
                        for k in 0..n {
                            if shape.to[k].is_none() {
                                continue;
                            }
                            let ((x, y1), (_, y0)) = (upper[at], lower[at]);
                            at += 1;
                            push_label(&mut pending, p, si, k, labels, x, (y0 + y1) / 2.0 - label_h / 2.0, TextAnchor::Middle);
                        }
                    }
                }
            }
            _ => {
                // Lines, scatter plots and bubbles: points joined or not.
                let bubble = family == Family::Bubble;
                let largest = data
                    .series
                    .iter()
                    .flat_map(|s| s.sizes.iter().flatten())
                    .cloned()
                    .fold(0.0f64, f64::max);
                // The largest bubble is a quarter of the plot across.
                let bubble_r = (px1 - px0).min(py1 - py0) * 0.125 * chart.bubble_scale.min(300) as f32 / 100.0;
                for &si in &g.series {
                    let Some(shape) = &shapes[si] else { continue };
                    let sd = &data.series[si];
                    let look = p.look(si);
                    let scale = scale_for(si);
                    let point = |k: usize| -> Option<(f32, f32)> {
                        let v = shape.to[k]?;
                        let u = match &x_scale {
                            Some(xs) => xs.fraction(sd.x_values.get(k).copied().flatten()?),
                            None => geo.cat(geo.centre(k)),
                        };
                        Some(geo.at(u, scale.fraction(v)))
                    };
                    let points: Vec<Option<(f32, f32)>> = (0..n).map(point).collect();
                    p.tag(ChartElement::Series(si as u32));
                    if let Some(stroke) = look.line {
                        // A gap in the data breaks the line, unless gaps
                        // are to be spanned.
                        let mut runs: Vec<Vec<(f32, f32)>> = vec![Vec::new()];
                        for pt in &points {
                            match pt {
                                Some(pt) => runs.last_mut().expect("a run").push(*pt),
                                None if chart.blanks == BlanksAs::Span => {}
                                None => runs.push(Vec::new()),
                            }
                        }
                        for run in runs {
                            if run.len() == 1 && look.marker.is_none() && geo.inside(run[0].0, run[0].1) {
                                // A point on its own still shows.
                                p.disc(run[0].0, run[0].1, 2.0 * z, stroke.ink.color);
                            }
                            let run = if look.smooth { smooth_points(&run) } else { run };
                            for piece in clip_polyline(&run, geo.rect()) {
                                p.polyline(&piece, stroke);
                            }
                        }
                    }
                    for (k, pt) in points.iter().enumerate() {
                        let Some((x, y)) = *pt else { continue };
                        if !geo.inside(x, y) {
                            continue;
                        }
                        let mut reach = 0.0f32;
                        let element = ChartElement::Point(si as u32, k as u32);
                        p.tag(element);
                        if bubble {
                            let size = sd.sizes.get(k).copied().flatten().unwrap_or(if sd.sizes.is_empty() { 1.0 } else { 0.0 });
                            let biggest = if largest > 0.0 { largest } else { 1.0 };
                            if size > 0.0 {
                                let r = bubble_r * (size / biggest).sqrt() as f32;
                                let ink = Ink {
                                    color: p.point_color(si, k),
                                    opacity: 0.75,
                                };
                                p.ellipse(x, y, r, r, Some(ink), look.outline);
                                p.anchor(element, x, y);
                                reach = r;
                            }
                        } else if let Some(marker) = &look.marker {
                            let own = chart.series[si].point(k).and_then(|pt| pt.color);
                            let m = match own {
                                Some(color) => super::MarkerLook {
                                    fill: color,
                                    line: color,
                                    ..*marker
                                },
                                None => *marker,
                            };
                            p.marker(&m, x, y);
                            reach = marker.r;
                        }
                        if !bubble {
                            p.pick(element, x, y);
                        }
                        if let Some(labels) = chart.labels_for(&chart.series[si]) {
                            let mid = y - label_h / 2.0;
                            let off = reach + 3.0 * z;
                            let (lx, top, anchor) = match labels.position {
                                LabelPosition::Above => (x, y - label_h - off, TextAnchor::Middle),
                                LabelPosition::Below => (x, y + off, TextAnchor::Middle),
                                LabelPosition::Left => (x - off, mid, TextAnchor::End),
                                LabelPosition::Center => (x, mid, TextAnchor::Middle),
                                _ => (x + off, mid, TextAnchor::Start),
                            };
                            push_label(&mut pending, p, si, k, labels, lx, top, anchor);
                        }
                    }
                }
            }
        }
    }

    // ---- error bars --------------------------------------------------------
    if p.trends_possible() {
        let stroke = Stroke::new(TEXT, (0.75 * PT * z).max(1.0));
        let cap = 3.0 * z;
        for g in &groups {
            if g.kind.is_stacked() {
                continue;
            }
            for (j, &si) in g.series.iter().enumerate() {
                let sd = &data.series[si];
                let scale = scale_for(si);
                // The middle of the point along the category axis.
                let centre = |k: usize| -> Option<f64> {
                    match &x_scale {
                        Some(xs) => Some(xs.fraction(sd.x_values.get(k).copied().flatten()?)),
                        None if matches!(g.kind.family(), Family::Column | Family::Bar) => {
                            let ns = g.series.len() as f64;
                            let gap = chart.gap_width.unwrap_or(g.kind.default_gap_width()).min(500) as f64 / 100.0;
                            let overlap = chart.overlap_for(g.kind) as f64 / 100.0;
                            let cluster_bars = ns - (ns - 1.0) * overlap;
                            let bar = 1.0 / (cluster_bars + gap);
                            let c0 = k as f64 + (1.0 - bar * cluster_bars) / 2.0 + j as f64 * bar * (1.0 - overlap);
                            Some(geo.cat(c0 + bar / 2.0))
                        }
                        None => Some(geo.cat(geo.centre(k))),
                    }
                };
                for (index, bars) in chart.series[si].error_bars.iter().enumerate() {
                    let element = ChartElement::ErrorBars(si as u32, index as u32);
                    p.tag(element);
                    for k in 0..sd.values.len() {
                        let (Some(v), Some(u)) = (sd.values[k], centre(k)) else { continue };
                        let f = scale.fraction(v);
                        let (a, b) = match (&x_scale, bars.horizontal) {
                            (Some(xs), true) => {
                                let Some(x) = sd.x_values.get(k).copied().flatten() else { continue };
                                let (lo, hi) = error_ends(bars, &sd.x_values, x);
                                (geo.at(xs.clamped(lo), f), geo.at(xs.clamped(hi), f))
                            }
                            (_, true) => continue,
                            _ => {
                                let (lo, hi) = error_ends(bars, &sd.values, v);
                                (geo.at(u, scale.clamped(lo)), geo.at(u, scale.clamped(hi)))
                            }
                        };
                        if !geo.inside((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0) {
                            continue;
                        }
                        p.line(a.0, a.1, b.0, b.1, stroke);
                        p.anchor(element, a.0, a.1);
                        p.anchor(element, b.0, b.1);
                        if bars.end_cap {
                            // Across the bar, at both ends.
                            let across_x = (a.1 - b.1).abs() > (a.0 - b.0).abs();
                            for end in [a, b] {
                                if across_x {
                                    p.line(end.0 - cap, end.1, end.0 + cap, end.1, stroke);
                                } else {
                                    p.line(end.0, end.1 - cap, end.0, end.1 + cap, stroke);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- trendlines --------------------------------------------------------
    let mut trend_labels: Vec<(f32, f32, &Trend)> = Vec::new();
    for t in &trends {
        let scale = scale_for(t.series);
        let pts: Vec<(f32, f32)> = t
            .points
            .iter()
            .map(|(x, y)| {
                let u = match &x_scale {
                    Some(xs) => xs.fraction(*x),
                    // Category number 1 is the first category.
                    None => geo.cat(if on_ticks { x - 1.0 } else { x - 0.5 }),
                };
                geo.at(u, scale.fraction(*y))
            })
            .collect();
        let mut last = None;
        let element = ChartElement::Trendline(t.series as u32, t.index as u32);
        p.tag(element);
        for piece in clip_polyline(&pts, geo.rect()) {
            if let Some((x, y)) = piece.first() {
                p.anchor(element, *x, *y);
            }
            last = piece.last().copied();
            p.polyline(&piece, t.stroke);
        }
        if let Some((x, y)) = last {
            p.anchor(element, x, y);
        }
        if let (Some((x, y)), false) = (last, t.label.is_empty()) {
            trend_labels.push((x, y, t));
        }
    }

    // ---- axes --------------------------------------------------------------
    let mut plans: Vec<(AxisId, &AxisPlan)> = vec![(AxisId::Value, &value_plan)];
    plans.extend(secondary_plan.as_ref().map(|plan| (AxisId::SecondaryValue, plan)));
    plans.push((AxisId::Category, &category_plan));
    plans.extend(far_plan.as_ref().map(|plan| (AxisId::SecondaryCategory, plan)));
    for (id, plan) in plans {
        let tick = 4.0 * z;
        p.tag(ChartElement::Axis(id));
        if plan.vertical {
            let x = (px0 + plan.line_at as f32 * (px1 - px0)).round();
            if plan.draw_line {
                p.vline(x, py0, py1, AXIS);
            }
            let y_of = |f: f64| py1 - f as f32 * (py1 - py0);
            let marks = |p: &mut Painter, mark: TickMark, at: &[f64], len: f32| {
                let (a, b) = match mark {
                    TickMark::None => return,
                    TickMark::Outside => (-len, 0.0),
                    TickMark::Inside => (0.0, len),
                    TickMark::Cross => (-len, len),
                };
                // "Outside" is away from the plot: to the left of a left axis.
                let (a, b) = if plan.line_at >= 0.999 { (-b, -a) } else { (a, b) };
                for f in at {
                    let y = y_of(*f).round();
                    p.hline(x + a, x + b + 1.0, y, AXIS);
                }
            };
            marks(p, plan.minor_mark, &plan.minor_ticks, tick / 2.0);
            marks(p, plan.tick_mark, &plan.ticks, tick);
            let label_w = if plan.categories { plan.label_w().min(max_cat_w) } else { plan.label_w() };
            // Thin out category labels that would run into each other.
            let step = if plan.categories { label_step(label_h, geo.slot(), 0.0) } else { 1 };
            for (i, (f, label, w)) in plan.labels.iter().enumerate() {
                if i % step != 0 {
                    continue;
                }
                let mid = if plan.categories { y_of(*f) } else { y_of(*f).round() };
                let top = mid - label_h / 2.0;
                match plan.side {
                    Side::Low if plan.categories => {
                        p.text(label, px0 - 6.0 * z - w.min(label_w), top, label_size, text, TextAnchor::Start)
                    }
                    Side::Low => p.text(label, px0 - 6.0 * z, top, label_size, text, TextAnchor::End),
                    Side::High => p.text(label, px1 + 6.0 * z, top, label_size, text, TextAnchor::Start),
                    Side::Inside => p.text(label, x - 6.0 * z, top, label_size, text, TextAnchor::End),
                }
            }
            let labels_w = if plan.labels.is_empty() || plan.side == Side::Inside { 0.0 } else { label_w + 8.0 * z };
            let high = plan.side == Side::High;
            let mut out = labels_w + 4.0 * z;
            if let Some(unit) = &plan.unit {
                let cx = if high { px1 + out + label_h / 2.0 } else { px0 - out - label_h / 2.0 };
                let w = p.width(unit, label_size);
                p.text_rotated(unit, cx, (py0 + w / 2.0 + 4.0 * z).min((py0 + py1) / 2.0), label_size, text, -90.0);
                out += label_h + 4.0 * z;
            }
            if let Some(title) = &plan.title {
                let cx = if high { px1 + out + title_h / 2.0 } else { px0 - out - title_h / 2.0 };
                p.tag(ChartElement::AxisTitle(id));
                p.text_rotated(title, cx, (py0 + py1) / 2.0, title_size, text, -90.0);
            }
        } else {
            let y = (py1 - plan.line_at as f32 * (py1 - py0)).round();
            if plan.draw_line {
                p.hline(px0, px1, y, AXIS);
            }
            let x_of = |f: f64| px0 + f as f32 * (px1 - px0);
            let marks = |p: &mut Painter, mark: TickMark, at: &[f64], len: f32| {
                let (a, b) = match mark {
                    TickMark::None => return,
                    TickMark::Outside => (0.0, len),
                    TickMark::Inside => (-len, 0.0),
                    TickMark::Cross => (-len, len),
                };
                let (a, b) = if plan.line_at >= 0.999 { (-b, -a) } else { (a, b) };
                for f in at {
                    let x = x_of(*f).round();
                    p.vline(x, y + a, y + b + 1.0, AXIS);
                }
            };
            marks(p, plan.minor_mark, &plan.minor_ticks, tick / 2.0);
            marks(p, plan.tick_mark, &plan.ticks, tick);
            let top = match plan.side {
                Side::Low => py1 + 5.0 * z,
                Side::High => py0 - 5.0 * z - label_h,
                Side::Inside => y + 5.0 * z,
            };
            if plan.categories {
                let step = label_step(plan.label_w(), geo.slot(), 6.0 * z);
                let tick_w = if value_plan.vertical && value_plan.side == Side::Low { value_plan.label_w() } else { 0.0 };
                for (i, (f, label, w)) in plan.labels.iter().enumerate() {
                    if i % step != 0 {
                        continue;
                    }
                    let x = (x_of(*f) - w / 2.0).clamp(px0 - tick_w, (px1 - w).max(px0 - tick_w));
                    p.text(label, x, top, label_size, text, TextAnchor::Start);
                }
            } else {
                for (f, label, _) in &plan.labels {
                    p.text(label, x_of(*f).round(), top, label_size, text, TextAnchor::Middle);
                }
            }
            let labels_h = if plan.categories && plan.side == Side::Low && table.is_some() {
                table_h
            } else if plan.labels.is_empty() || plan.side == Side::Inside {
                0.0
            } else {
                label_h + 6.0 * z
            };
            let high = plan.side == Side::High;
            let mut out = labels_h + 2.0 * z;
            if let Some(unit) = &plan.unit {
                let top = if high { py0 - out - label_h } else { py1 + out };
                p.text(unit, px1, top, label_size, text, TextAnchor::End);
                out += label_h + 4.0 * z;
            }
            if let Some(title) = &plan.title {
                let top = if high { py0 - out - title_h } else { py1 + out };
                p.tag(ChartElement::AxisTitle(id));
                p.text(title, (px0 + px1) / 2.0, top, title_size, text, TextAnchor::Middle);
            }
        }
    }

    // ---- data table --------------------------------------------------------
    if let Some(table) = table {
        p.tag(ChartElement::DataTable);
        let rule = Stroke::new(GRIDLINE, 1.0);
        let rows = table_rows.len();
        let x_of = |c: f64| px0 + geo.cat(c) as f32 * (px1 - px0);
        let (x_left, y_end) = (px0 - names_w, py1 + table_h);
        // The categories head the columns.
        for k in 0..n {
            if let Some(name) = data.categories.get(k) {
                p.text(name, x_of(k as f64 + 0.5), py1 + 2.0 * z, label_size, text, TextAnchor::Middle);
            }
        }
        for (row, &si) in table_rows.iter().enumerate() {
            let y = py1 + row_h * (row as f32 + 1.0);
            let mut x = x_left + 4.0 * z;
            if table.legend_keys {
                let s = key_w - 4.0 * z;
                p.rect(x, y + row_h / 2.0 - s / 2.0, x + s, y + row_h / 2.0 + s / 2.0, data.series[si].color);
                x += key_w;
            }
            p.text(&p.series_name(si), x, y + 2.0 * z, label_size, text, TextAnchor::Start);
            for k in 0..n {
                if let Some(Some(v)) = data.series[si].values.get(k) {
                    let cell = fmt_value(*v, &data.series[si].format, locale);
                    p.text(&cell, x_of(k as f64 + 0.5), y + 2.0 * z, label_size, text, TextAnchor::Middle);
                }
            }
        }
        if table.horizontal_lines {
            for row in 1..=rows {
                let y = (py1 + row_h * row as f32).round() + 0.5;
                p.line(if row == 1 && !table.outline { px0 } else { x_left }, y, px1, y, rule);
            }
        }
        if table.vertical_lines {
            for k in 0..=slots {
                let x = x_of(k as f64).round() + 0.5;
                p.line(x, py1, x, y_end, rule);
            }
        }
        if table.outline {
            // Around the columns, and around the names beside them.
            let (top, head, end) = (py1.round() + 0.5, (py1 + row_h).round() + 0.5, y_end.round() + 0.5);
            let (l, m, r) = (x_left.round() + 0.5, px0.round() + 0.5, px1.round() + 0.5);
            for (a, b) in [((m, top), (r, top)), ((r, top), (r, end)), ((l, end), (r, end)), ((l, head), (l, end)), ((l, head), (m, head)), ((m, top), (m, end))] {
                p.line(a.0, a.1, b.0, b.1, rule);
            }
        }
    }

    // ---- labels over everything --------------------------------------------
    for (x, y, t) in trend_labels {
        p.tag(ChartElement::Trendline(t.series as u32, t.index as u32));
        let sup = (label_size * 0.65).max(5.0);
        let line_w = |parts: &[EquationPart]| -> f32 {
            parts.iter().map(|part| p.width(&part.text, if part.raised { sup } else { label_size })).sum()
        };
        let w = t.label.iter().map(|l| line_w(l)).fold(0.0, f32::max);
        let h = label_h * t.label.len() as f32;
        // Near the end of the line, above it, inside the plot.
        let lx = (x - w - 4.0 * z).clamp(px0 + 2.0, (px1 - w - 2.0).max(px0 + 2.0));
        let mut ly = (y - h - 6.0 * z).clamp(py0, (py1 - h).max(py0));
        for parts in &t.label {
            let mut cx = lx;
            // Pieces on the same level are one run of text.
            let mut runs: Vec<(String, bool)> = Vec::new();
            for part in parts {
                match runs.last_mut() {
                    Some((text, raised)) if *raised == part.raised => text.push_str(&part.text),
                    _ => runs.push((part.text.clone(), part.raised)),
                }
            }
            for (run, raised) in runs {
                if raised {
                    p.text(&run, cx, ly - label_size * 0.2, sup, text, TextAnchor::Start);
                    cx += p.width(&run, sup);
                } else {
                    p.text(&run, cx, ly, label_size, text, TextAnchor::Start);
                    cx += p.width(&run, label_size);
                }
            }
            ly += label_h;
        }
    }
    for label in pending {
        p.tag(ChartElement::DataLabels(label.series as u32));
        let mut x = label.x;
        if let Some(color) = label.key {
            // The swatch goes before the text, the pair keeps its anchor.
            let s = (6.0 * z).round().max(3.0);
            let w = p.width(&label.text, label_size);
            let start = match label.anchor {
                TextAnchor::Start => x,
                TextAnchor::Middle => x - (w + s + 3.0 * z) / 2.0,
                TextAnchor::End => x - w - s - 3.0 * z,
            };
            let cy = label.top + label_h / 2.0;
            p.rect(start, cy - s / 2.0, start + s, cy + s / 2.0, color);
            x = match label.anchor {
                TextAnchor::Start => x + s + 3.0 * z,
                TextAnchor::Middle => x + (s + 3.0 * z) / 2.0,
                TextAnchor::End => x,
            };
        }
        p.text(&label.text, x, label.top, label_size, text, label.anchor);
    }
}
