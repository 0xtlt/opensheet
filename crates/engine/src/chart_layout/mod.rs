//! Laying a chart out: a chart, the data it plots and a size go in, simple
//! vector primitives come out (rectangles, lines, paths, runs of text).
//! Nothing here knows about a window or a page: the application converts
//! the primitives to its display list, a PDF writer to drawing operators.
//! The only thing asked of the consumer is the width of a piece of text
//! (`TextMeasure`).
//!
//! Coordinates are pixels from the top-left corner of the chart, y down.
//! Sizes that Excel gives in points (fonts, line widths, markers) are
//! multiplied by `LayoutOptions::scale`: 1 draws the chart at 100 %, 96 dpi.
//!
//! The metrics (fonts, paddings, colours) are those of Excel's default
//! chart style.

mod cartesian;
mod radar;
mod round;
mod scale;
mod three_d;

pub use scale::{Fit, Scale};

use crate::chart::{
    Chart, ChartData, ChartKind, ChartTitle, Dash, Family, Fill, LegendPos, Line, MarkerShape, Series, TrendKind,
    palette_color,
};
use crate::format;
use crate::locale::Locale;
use crate::style::Color;

/// Titles, labels and legends: dark grey rather than black.
pub(crate) const TEXT: Color = Color(0x595959);
pub(crate) const GRIDLINE: Color = Color(0xD9D9D9);
pub(crate) const MINOR_GRIDLINE: Color = Color(0xF2F2F2);
pub(crate) const AXIS: Color = Color(0xBFBFBF);

/// Pixels per point at 100 %.
pub(crate) const PT: f32 = 96.0 / 72.0;

/// A colour and how opaque it is (1 = solid).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ink {
    pub color: Color,
    pub opacity: f32,
}

impl Ink {
    pub fn solid(color: Color) -> Ink {
        Ink { color, opacity: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub ink: Ink,
    /// Width in pixels.
    pub width: f32,
    /// `Dash::pattern` gives the lengths, in units of `width`.
    pub dash: Dash,
}

impl Stroke {
    pub fn new(color: Color, width: f32) -> Stroke {
        Stroke {
            ink: Ink::solid(color),
            width,
            dash: Dash::Solid,
        }
    }
}

/// One step of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// Cubic Bézier: first control point, second control point, end.
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAnchor {
    /// `x` is the left end of the text.
    Start,
    Middle,
    End,
}

/// One line of text in the chart's font.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// The anchor point: `x` as `anchor` says, `y` the middle of the line
    /// (a line is `line_height(size)` tall).
    pub x: f32,
    pub y: f32,
    pub anchor: TextAnchor,
    /// Font size in pixels.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: Color,
    /// Degrees, clockwise, around the anchor point (-90 reads bottom to
    /// top, as the title of a vertical axis does).
    pub rotation: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    /// An axis-aligned rectangle; `w` and `h` are never negative.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Option<Ink>,
        stroke: Option<Stroke>,
    },
    Line {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stroke: Stroke,
    },
    Path {
        segs: Vec<Seg>,
        fill: Option<Ink>,
        stroke: Option<Stroke>,
    },
    Text(TextRun),
}

/// Height of a line of text of the given font size, in pixels.
pub fn line_height(size: f32) -> f32 {
    (size * 1.25).round()
}

/// The one thing the layout needs from whoever draws the text.
pub trait TextMeasure {
    /// Width in pixels of `text` on one line, at `size` pixels.
    fn width(&self, text: &str, size: f32, bold: bool) -> f32;
}

/// The words a chart shows that come from the application's language
/// rather than from the workbook.
#[derive(Clone, Debug)]
pub struct Labels<'a> {
    /// Shown when the chart has an automatic title and several series.
    pub placeholder_title: &'a str,
    /// Prefix of unnamed series ("Series" → "Series1").
    pub series_prefix: &'a str,
    /// Names of automatic trendlines: linear, exponential, logarithmic,
    /// polynomial, power, moving average ("Linear" → "Linear (Sales)",
    /// "per. Mov. Avg." → "2 per. Mov. Avg. (Sales)").
    pub trend_names: [&'a str; 6],
    /// Display units: hundreds, thousands, millions, billions, trillions.
    pub unit_names: [&'a str; 5],
}

impl Default for Labels<'static> {
    fn default() -> Self {
        Labels {
            placeholder_title: "Chart Title",
            series_prefix: "Series",
            trend_names: ["Linear", "Expon.", "Log.", "Poly.", "Power", "per. Mov. Avg."],
            unit_names: ["Hundreds", "Thousands", "Millions", "Billions", "Trillions"],
        }
    }
}

pub struct LayoutOptions<'a> {
    /// Pixels per pixel at 100 %: the zoom of the sheet on screen.
    pub scale: f32,
    pub locale: &'a Locale,
    pub labels: Labels<'a>,
}

/// Lay a chart out in a box of `size` pixels. The primitives come in
/// painting order.
pub fn layout_chart(
    chart: &Chart,
    data: &ChartData,
    size: (f32, f32),
    options: &LayoutOptions,
    measure: &dyn TextMeasure,
) -> Vec<Prim> {
    let z = options.scale;
    let label_size = (12.0 * z).max(6.0);
    let mut p = Painter {
        chart,
        data,
        options,
        measure,
        z,
        out: Vec::new(),
        label_size,
        label_h: line_height(label_size),
    };
    p.draw(size.0, size.1);
    p.out
}

/// How a series is drawn, automatic choices resolved.
pub(crate) struct Look {
    pub kind: ChartKind,
    /// The ink of the series.
    pub color: Color,
    /// The stroke joining the points of a line, scatter or radar series.
    pub line: Option<Stroke>,
    /// The outline of bars, areas and slices.
    pub outline: Option<Stroke>,
    pub marker: Option<MarkerLook>,
    pub smooth: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct MarkerLook {
    pub shape: MarkerShape,
    /// Half the size, in pixels.
    pub r: f32,
    pub fill: Color,
    pub line: Color,
}

/// The sample of a series shown in the legend.
pub(crate) enum Key {
    Swatch(Color),
    Line(Option<Stroke>, Option<MarkerLook>),
}

pub(crate) struct Painter<'a> {
    pub chart: &'a Chart,
    pub data: &'a ChartData,
    pub options: &'a LayoutOptions<'a>,
    pub measure: &'a dyn TextMeasure,
    pub z: f32,
    pub out: Vec<Prim>,
    /// Size of axis labels, legend entries and data labels (9 pt).
    pub label_size: f32,
    pub label_h: f32,
}

/// Points of a circle, for shapes that are clipped or mixed with straight
/// edges; whole discs are drawn with Bézier arcs instead.
pub(crate) fn arc_points(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let steps = (((to - from).abs() / 0.04).ceil() as usize).max(2);
    (0..=steps)
        .map(|k| {
            let a = from + (to - from) * k as f32 / steps as f32;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

/// Bézier segments drawing the arc of an ellipse from angle `from` to
/// `to` (radians, clockwise on screen), starting from the current point.
pub(crate) fn arc_segs(segs: &mut Vec<Seg>, cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32) {
    let pieces = (((to - from).abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
    let step = (to - from) / pieces as f32;
    // Distance of the control points for an arc of `step` radians.
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for i in 0..pieces {
        let a0 = from + step * i as f32;
        let a1 = a0 + step;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        segs.push(Seg::Cubic(
            cx + rx * (c0 - k * s0),
            cy + ry * (s0 + k * c0),
            cx + rx * (c1 + k * s1),
            cy + ry * (s1 - k * c1),
            cx + rx * c1,
            cy + ry * s1,
        ));
    }
}

/// A colour moved towards white (`amount` > 0) or black (< 0).
pub(crate) fn shade(color: Color, amount: f32) -> Color {
    let ch = |v: u8| -> u32 {
        let v = v as f32;
        let out = if amount >= 0.0 { v + (255.0 - v) * amount } else { v * (1.0 + amount) };
        out.round().clamp(0.0, 255.0) as u32
    };
    Color((ch(color.r()) << 16) | (ch(color.g()) << 8) | ch(color.b()))
}

pub(crate) fn fmt_value(v: f64, code: &str, locale: &Locale) -> String {
    if code.eq_ignore_ascii_case("General") {
        format::format_general(v, locale, 11)
    } else {
        format::format_number(v, code, locale).text
    }
}

/// Keep the part of a polygon that lies inside a rectangle.
pub(crate) fn clip_polygon(points: &[(f32, f32)], rect: (f32, f32, f32, f32)) -> Vec<(f32, f32)> {
    let (x0, y0, x1, y1) = rect;
    // One pass per edge of the rectangle (Sutherland–Hodgman).
    let pass = |input: Vec<(f32, f32)>, inside: &dyn Fn((f32, f32)) -> bool, cut: &dyn Fn((f32, f32), (f32, f32)) -> (f32, f32)| {
        let mut out = Vec::with_capacity(input.len() + 4);
        for i in 0..input.len() {
            let a = input[i];
            let b = input[(i + 1) % input.len()];
            match (inside(a), inside(b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cut(a, b)),
                (false, true) => {
                    out.push(cut(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
        out
    };
    let at_x = |x: f32| move |a: (f32, f32), b: (f32, f32)| (x, a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0));
    let at_y = |y: f32| move |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * (y - a.1) / (b.1 - a.1), y);
    let mut pts = points.to_vec();
    pts = pass(pts, &|p| p.0 >= x0, &at_x(x0));
    pts = pass(pts, &|p| p.0 <= x1, &at_x(x1));
    pts = pass(pts, &|p| p.1 >= y0, &at_y(y0));
    pass(pts, &|p| p.1 <= y1, &at_y(y1))
}

/// The pieces of a polyline that lie inside a rectangle.
pub(crate) fn clip_polyline(points: &[(f32, f32)], rect: (f32, f32, f32, f32)) -> Vec<Vec<(f32, f32)>> {
    let (x0, y0, x1, y1) = rect;
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut run: Vec<(f32, f32)> = Vec::new();
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        // Liang–Barsky: the part t0..t1 of the segment that is inside.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut inside = true;
        for (p, q) in [(-dx, a.0 - x0), (dx, x1 - a.0), (-dy, a.1 - y0), (dy, y1 - a.1)] {
            if p == 0.0 {
                inside &= q >= 0.0;
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        if !inside || t0 > t1 {
            if run.len() > 1 {
                out.push(std::mem::take(&mut run));
            }
            run.clear();
            continue;
        }
        let start = (a.0 + dx * t0, a.1 + dy * t0);
        let end = (a.0 + dx * t1, a.1 + dy * t1);
        if run.last() != Some(&start) {
            if run.len() > 1 {
                out.push(std::mem::take(&mut run));
            }
            run.clear();
            run.push(start);
        }
        run.push(end);
        if t1 < 1.0 {
            out.push(std::mem::take(&mut run));
        }
    }
    if run.len() > 1 {
        out.push(run);
    }
    out
}

/// Points along the curve Excel draws through the points of a smoothed
/// line (a Catmull-Rom spline).
pub(crate) fn smooth_points(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    const STEPS: usize = 12;
    let mut out = vec![points[0]];
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        for k in 1..=STEPS {
            let t = k as f32 / STEPS as f32;
            let (t2, t3) = (t * t, t * t * t);
            let blend = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            out.push((blend(p0.0, p1.0, p2.0, p3.0), blend(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out
}

impl Painter<'_> {
    pub fn width(&self, text: &str, size: f32) -> f32 {
        self.measure.width(text, size, false)
    }

    /// Fill a rectangle given by two opposite corners.
    pub fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color) {
        self.rect_styled(x0, y0, x1, y1, Some(Ink::solid(color)), None);
    }

    pub fn rect_styled(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, fill: Option<Ink>, stroke: Option<Stroke>) {
        if fill.is_none() && stroke.is_none() {
            return;
        }
        self.out.push(Prim::Rect {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x1 - x0).abs(),
            h: (y1 - y0).abs(),
            fill,
            stroke,
        });
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, stroke: Stroke) {
        self.out.push(Prim::Line { x0, y0, x1, y1, stroke });
    }

    /// A one-pixel horizontal rule whose top edge is `y`.
    pub fn hline(&mut self, x0: f32, x1: f32, y: f32, color: Color) {
        self.line(x0, y + 0.5, x1, y + 0.5, Stroke::new(color, 1.0));
    }

    /// A one-pixel vertical rule whose left edge is `x`.
    pub fn vline(&mut self, x: f32, y0: f32, y1: f32, color: Color) {
        self.line(x + 0.5, y0, x + 0.5, y1, Stroke::new(color, 1.0));
    }

    pub fn polygon(&mut self, points: &[(f32, f32)], fill: Option<Ink>, stroke: Option<Stroke>) {
        if points.len() < 3 || (fill.is_none() && stroke.is_none()) {
            return;
        }
        let mut segs = Vec::with_capacity(points.len() + 1);
        segs.push(Seg::Move(points[0].0, points[0].1));
        segs.extend(points[1..].iter().map(|(x, y)| Seg::Line(*x, *y)));
        segs.push(Seg::Close);
        self.out.push(Prim::Path { segs, fill, stroke });
    }

    pub fn polyline(&mut self, points: &[(f32, f32)], stroke: Stroke) {
        if points.len() < 2 {
            return;
        }
        let mut segs = Vec::with_capacity(points.len());
        segs.push(Seg::Move(points[0].0, points[0].1));
        segs.extend(points[1..].iter().map(|(x, y)| Seg::Line(*x, *y)));
        self.out.push(Prim::Path {
            segs,
            fill: None,
            stroke: Some(stroke),
        });
    }

    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, fill: Option<Ink>, stroke: Option<Stroke>) {
        let mut segs = vec![Seg::Move(cx + rx, cy)];
        arc_segs(&mut segs, cx, cy, rx, ry, 0.0, std::f32::consts::TAU);
        segs.push(Seg::Close);
        self.out.push(Prim::Path { segs, fill, stroke });
    }

    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.ellipse(cx, cy, r, r, Some(Ink::solid(color)), None);
    }

    /// Text with its top-left corner (or top-centre, top-right, as
    /// `anchor` says) at (x, top).
    pub fn text(&mut self, text: &str, x: f32, top: f32, size: f32, color: Color, anchor: TextAnchor) {
        self.out.push(Prim::Text(TextRun {
            text: text.replace(['\n', '\r'], " "),
            x,
            y: top + line_height(size) / 2.0,
            anchor,
            size,
            bold: false,
            italic: false,
            color,
            rotation: 0.0,
        }));
    }

    /// Text centred on a point and turned around it.
    pub fn text_rotated(&mut self, text: &str, cx: f32, cy: f32, size: f32, color: Color, rotation: f32) {
        self.out.push(Prim::Text(TextRun {
            text: text.replace(['\n', '\r'], " "),
            x: cx,
            y: cy,
            anchor: TextAnchor::Middle,
            size,
            bold: false,
            italic: false,
            color,
            rotation,
        }));
    }

    /// A label of the standard size, left-aligned; returns its width.
    pub fn label(&mut self, text: &str, x: f32, top: f32, color: Color) -> f32 {
        let size = self.label_size;
        self.text(text, x, top, size, color, TextAnchor::Start);
        self.width(text, size)
    }

    pub fn marker(&mut self, m: &MarkerLook, cx: f32, cy: f32) {
        let r = m.r;
        let fill = Some(Ink::solid(m.fill));
        let thin = Stroke::new(m.line, (0.75 * PT * self.z).max(0.75));
        let outline = (m.line != m.fill).then_some(thin);
        let stroke = Stroke::new(m.line, (1.5 * self.z).max(1.0));
        match m.shape {
            MarkerShape::None => {}
            MarkerShape::Circle => self.ellipse(cx, cy, r, r, fill, outline),
            MarkerShape::Square => self.rect_styled(cx - r, cy - r, cx + r, cy + r, fill, outline),
            MarkerShape::Diamond => {
                let d = r * 1.2;
                self.polygon(&[(cx, cy - d), (cx + d, cy), (cx, cy + d), (cx - d, cy)], fill, outline);
            }
            MarkerShape::Triangle => {
                self.polygon(&[(cx, cy - r), (cx + r, cy + r), (cx - r, cy + r)], fill, outline);
            }
            MarkerShape::X => {
                self.line(cx - r, cy - r, cx + r, cy + r, stroke);
                self.line(cx - r, cy + r, cx + r, cy - r, stroke);
            }
            MarkerShape::Star => {
                self.line(cx - r, cy - r, cx + r, cy + r, stroke);
                self.line(cx - r, cy + r, cx + r, cy - r, stroke);
                self.line(cx, cy - r, cx, cy + r, stroke);
            }
            MarkerShape::Plus => {
                self.line(cx - r, cy, cx + r, cy, stroke);
                self.line(cx, cy - r, cx, cy + r, stroke);
            }
            MarkerShape::Dash => self.rect(cx - r, cy - r * 0.3, cx + r, cy + r * 0.3, m.fill),
            MarkerShape::Dot => self.rect(cx - r * 0.5, cy - r * 0.3, cx + r * 0.5, cy + r * 0.3, m.fill),
        }
    }

    pub fn series_name(&self, i: usize) -> String {
        match self.data.series.get(i) {
            Some(s) if !s.name.is_empty() => s.name.clone(),
            _ => format!("{}{}", self.options.labels.series_prefix, i + 1),
        }
    }

    /// The stroke a `Line` asks for, given what "automatic" means where
    /// it is used: whether there is a line at all, its colour, its width
    /// in points.
    pub fn stroke(&self, line: &Line, auto_shown: bool, auto_color: Color, auto_width: f32) -> Option<Stroke> {
        if !line.shown.unwrap_or(auto_shown || line.color.is_some() || line.width.is_some()) {
            return None;
        }
        let width = line.width.unwrap_or(auto_width).clamp(0.0, 100.0) * PT * self.z;
        Some(Stroke {
            ink: Ink::solid(line.color.unwrap_or(auto_color)),
            width: width.max(0.5),
            dash: line.dash,
        })
    }

    pub fn look(&self, index: usize) -> Look {
        let series = &self.chart.series[index];
        let kind = self.chart.series_kind(series);
        let color = self.data.series.get(index).map_or_else(|| palette_color(index), |s| s.color);
        let lined = matches!(kind.family(), Family::Line | Family::Scatter | Family::Radar);
        let line = if lined && series.line_shown(kind) {
            // Lines never get thinner than a pixel and a half on screen.
            self.stroke(&series.line, true, color, kind.default_line_width()).map(|s| Stroke {
                width: if series.line.width.is_none() { s.width.max(1.5) } else { s.width },
                ..s
            })
        } else {
            None
        };
        let outline = if lined {
            None
        } else {
            // Slices are kept apart by a white line.
            let round = kind.is_round();
            self.stroke(&series.line, round, Color::WHITE, if round { 1.5 } else { 0.75 })
        };
        let marker = series.marker_for(kind);
        let marker = (lined && marker.shape != MarkerShape::None).then(|| MarkerLook {
            shape: marker.shape,
            r: (marker.size.clamp(2, 72) as f32 * 0.7 * self.z).max(2.0),
            fill: marker.fill.unwrap_or(color),
            line: marker.line.unwrap_or(color),
        });
        Look {
            kind,
            color,
            line,
            outline,
            marker,
            smooth: lined && series.smooth_for(kind),
        }
    }

    /// Whether each point of the series gets a colour of the palette.
    pub fn varied(&self) -> bool {
        self.chart.varies_colors() && (self.chart.kind.is_round() || self.chart.series.len() == 1)
    }

    /// The colour of one point of a series.
    pub fn point_color(&self, series: usize, point: usize) -> Color {
        let s = &self.chart.series[series];
        if let Some(color) = s.point(point).and_then(|p| p.color) {
            return color;
        }
        if self.varied() {
            return palette_color(point);
        }
        self.data.series.get(series).map_or_else(|| palette_color(series), |s| s.color)
    }

    pub fn trend_name(&self, series: usize, kind: TrendKind, name: &Option<String>) -> String {
        if let Some(name) = name {
            return name.clone();
        }
        let names = &self.options.labels.trend_names;
        let of = self.series_name(series);
        match kind {
            TrendKind::Linear => format!("{} ({of})", names[0]),
            TrendKind::Exponential => format!("{} ({of})", names[1]),
            TrendKind::Logarithmic => format!("{} ({of})", names[2]),
            TrendKind::Polynomial(_) => format!("{} ({of})", names[3]),
            TrendKind::Power => format!("{} ({of})", names[4]),
            TrendKind::MovingAverage(period) => format!("{} {} ({of})", period.max(2), names[5]),
        }
    }

    /// The stroke of a trendline: the colour of its series, 1.5 pt, round
    /// dots, unless it says otherwise.
    pub fn trend_stroke(&self, series: usize, line: &Line) -> Stroke {
        let color = self.data.series.get(series).map_or_else(|| palette_color(series), |s| s.color);
        let auto = Line {
            dash: if *line == Line::default() { Dash::RoundDot } else { line.dash },
            ..line.clone()
        };
        self.stroke(&auto, true, color, 1.5).unwrap_or(Stroke::new(color, 2.0 * self.z))
    }

    /// The entries of the legend, those removed from it left out.
    fn legend_entries(&self) -> Vec<(String, Key)> {
        let chart = self.chart;
        let mut entries: Vec<(String, Key)> = Vec::new();
        if self.varied() {
            let n = self.data.series.first().map_or(0, |s| s.values.len());
            for i in 0..n {
                let name = self.data.categories.get(i).cloned().unwrap_or_default();
                entries.push((name, Key::Swatch(self.point_color(0, i))));
            }
        } else {
            for i in 0..self.data.series.len().min(chart.series.len()) {
                let look = self.look(i);
                let key = match look.kind.family() {
                    Family::Line | Family::Scatter | Family::Radar if look.kind != ChartKind::RadarFilled => {
                        Key::Line(look.line, look.marker)
                    }
                    _ => Key::Swatch(look.color),
                };
                entries.push((self.series_name(i), key));
            }
        }
        if self.trends_possible() {
            for (i, s) in chart.series.iter().enumerate().take(self.data.series.len()) {
                for t in &s.trendlines {
                    let stroke = self.trend_stroke(i, &t.line);
                    entries.push((self.trend_name(i, t.kind, &t.name), Key::Line(Some(stroke), None)));
                }
            }
        }
        let mut index = 0u32;
        entries.retain(|_| {
            index += 1;
            !chart.legend_hidden.contains(&(index - 1))
        });
        // Clustered bars are listed from the top one down, as Excel does:
        // the first series is the lowest bar of each cluster.
        if chart.kind.family() == Family::Bar
            && !chart.kind.is_stacked()
            && !chart.axes.category.reversed
            && !self.varied()
        {
            let series = chart.series.len().min(self.data.series.len()) as u32;
            let shown = (0..series).filter(|i| !chart.legend_hidden.contains(i)).count();
            entries[..shown].reverse();
        }
        entries
    }

    /// Trendlines and error bars exist on flat charts with two axes.
    pub fn trends_possible(&self) -> bool {
        let kind = self.chart.kind;
        !kind.is_3d() && !kind.is_round() && kind.family() != Family::Radar
    }

    fn draw_key(&mut self, key: &Key, x: f32, cy: f32, mark_w: f32) {
        let z = self.z;
        match key {
            Key::Swatch(color) => {
                let s = (7.0 * z).round().max(4.0);
                self.rect(x, cy - s / 2.0, x + mark_w.max(s), cy + s / 2.0, *color);
            }
            Key::Line(stroke, marker) => {
                if let Some(stroke) = stroke {
                    if stroke.dash == Dash::Solid {
                        let half = (stroke.width / 2.0).clamp(0.5, 2.0 * z);
                        self.rect(x, cy - half, x + mark_w, cy + half, stroke.ink.color);
                    } else {
                        self.line(x, cy, x + mark_w, cy, *stroke);
                    }
                }
                if let Some(marker) = marker {
                    let small = MarkerLook {
                        r: marker.r.min(3.0 * z).max(2.0),
                        ..*marker
                    };
                    self.marker(&small, x + mark_w / 2.0, cy);
                }
            }
        }
    }

    fn draw(&mut self, w: f32, h: f32) {
        let chart = self.chart;
        let z = self.z;
        let text = TEXT;
        // Chart area.
        let area_fill = match chart.chart_area.fill {
            Fill::Auto => Some(Ink::solid(Color::WHITE)),
            Fill::None => None,
            Fill::Solid(color) => Some(Ink::solid(color)),
        };
        let frame = self.stroke(&chart.chart_area.border, true, GRIDLINE, 0.75).map(|s| Stroke {
            // The automatic frame is a hairline whatever the zoom.
            width: if chart.chart_area.border.width.is_none() { 1.0 } else { s.width },
            ..s
        });
        self.rect_styled(0.0, 0.0, w, h, area_fill, frame);

        let pad = (10.0 * z).round();
        let (mut left, mut top, mut right, mut bottom) = (pad, pad, w - pad, h - pad);
        let label_size = self.label_size;
        let label_h = self.label_h;

        // Title.
        let title = match (&chart.title, &self.data.title) {
            (ChartTitle::None, _) => None,
            (_, Some(t)) => Some(t.clone()),
            _ => Some(self.options.labels.placeholder_title.to_string()),
        };
        let mut title_run = None;
        if let Some(title) = title {
            let size = (18.67 * z).max(8.0);
            let tw = self.width(&title, size);
            title_run = Some((title, ((w - tw) / 2.0).max(4.0), top - 2.0 * z, size));
            if !chart.title_overlay {
                top += line_height(size) + 4.0 * z;
            }
        }

        // Legend: measured now, so that the plot knows the room left, and
        // drawn after the plot when it lies over it.
        let entries = if chart.legend == LegendPos::None { Vec::new() } else { self.legend_entries() };
        let mut placed: Vec<(usize, f32, f32)> = Vec::new();
        // A swatch for shapes, a piece of line for lines; next to lines a
        // swatch is as wide as they are, as in Excel's legends.
        let swatch = (7.0 * z).round().max(4.0);
        let lined = entries.iter().any(|(_, key)| matches!(key, Key::Line(..)));
        let marks: Vec<f32> = entries.iter().map(|_| if lined { 18.0 * z } else { swatch }).collect();
        if !entries.is_empty() {
            let widths: Vec<f32> = entries.iter().map(|(name, _)| self.width(name, label_size)).collect();
            let entry_gap = 14.0 * z;
            let entry_w = |i: usize| widths[i] + marks[i] + 4.0 * z;
            let reserve = !chart.legend_overlay;
            match chart.legend {
                LegendPos::Bottom | LegendPos::Top => {
                    // Entries flow onto as many centred rows as they need.
                    let available = (w - 2.0 * pad).max(40.0);
                    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
                    let mut row_w = 0.0f32;
                    for i in 0..entries.len() {
                        let ew = entry_w(i);
                        let last = rows.last_mut().expect("at least one row");
                        let needed = if last.is_empty() { ew } else { row_w + entry_gap + ew };
                        if needed > available && !last.is_empty() {
                            rows.push(vec![i]);
                            row_w = ew;
                        } else {
                            last.push(i);
                            row_w = needed;
                        }
                    }
                    // Never let the legend take more than a third of the chart.
                    let max_rows = ((h / 3.0) / label_h).floor().max(1.0) as usize;
                    rows.truncate(max_rows);
                    let block_h = label_h * rows.len() as f32;
                    let mut y = if chart.legend == LegendPos::Bottom {
                        let y = bottom - block_h;
                        if reserve {
                            bottom -= block_h + 6.0 * z;
                        }
                        y
                    } else {
                        let y = top;
                        if reserve {
                            top += block_h + 6.0 * z;
                        }
                        y
                    };
                    for row in &rows {
                        let total: f32 =
                            row.iter().map(|i| entry_w(*i)).sum::<f32>() + entry_gap * (row.len() as f32 - 1.0);
                        let mut x = ((w - total) / 2.0).max(pad);
                        for i in row {
                            // An entry wider than the chart is left out.
                            if x + marks[*i] <= w - pad && y >= 0.0 {
                                placed.push((*i, x, y));
                            }
                            x += entry_w(*i) + entry_gap;
                        }
                        y += label_h;
                    }
                }
                LegendPos::Right | LegendPos::Left | LegendPos::TopRight => {
                    let col_w = (0..entries.len()).map(entry_w).fold(0.0f32, f32::max);
                    let total_h = label_h * entries.len() as f32;
                    let x = if chart.legend == LegendPos::Left {
                        let x = left;
                        if reserve {
                            left += col_w + 8.0 * z;
                        }
                        x
                    } else {
                        let x = right - col_w;
                        if reserve {
                            right -= col_w + 8.0 * z;
                        }
                        x
                    };
                    let mut y = if chart.legend == LegendPos::TopRight {
                        top
                    } else {
                        top + ((bottom - top - total_h) / 2.0).max(0.0)
                    };
                    for (i, mark) in marks.iter().enumerate() {
                        // Entries the chart has no room for are left out.
                        if x >= 0.0 && x + mark <= w && y + label_h <= h {
                            placed.push((i, x, y));
                        }
                        y += label_h;
                    }
                }
                LegendPos::None => {}
            }
        }

        if right - left > 20.0 && bottom - top > 20.0 {
            let area = (left, top, right, bottom);
            match (chart.kind.family(), chart.kind.is_3d()) {
                (Family::Pie, true) => round::draw_pie_3d(self, area),
                (Family::Pie | Family::Doughnut, _) => round::draw_round(self, area),
                (Family::Radar, _) => radar::draw_radar(self, area),
                (_, true) => three_d::draw_3d(self, area),
                _ => cartesian::draw_cartesian(self, area),
            }
        }

        if let Some((title, x, y, size)) = title_run {
            self.text(&title, x, y, size, text, TextAnchor::Start);
        }
        for (i, x, y) in placed {
            let (name, key) = &entries[i];
            self.draw_key(key, x, y + label_h / 2.0, marks[i]);
            self.label(name, x + marks[i] + 4.0 * z, y, text);
        }
    }
}

/// The series of a chart drawn by one family of kinds on one pair of axes.
pub(crate) struct Group {
    pub kind: ChartKind,
    pub secondary: bool,
    /// Indices into `Chart::series`.
    pub series: Vec<usize>,
}

/// Split the series into the groups that are drawn together: stacked with
/// each other, clustered next to each other.
pub(crate) fn groups(chart: &Chart, data: &ChartData) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (i, s) in chart.series.iter().enumerate().take(data.series.len()) {
        let kind = chart.series_kind(s);
        // Lines with and without markers are one group: they stack together.
        let same = |g: &Group| {
            g.kind.family() == kind.family() && g.kind.grouping() == kind.grouping() && g.secondary == chart.on_secondary(s)
        };
        match out.iter_mut().find(|g| same(g)) {
            Some(group) => group.series.push(i),
            None => out.push(Group {
                kind,
                secondary: chart.on_secondary(s),
                series: vec![i],
            }),
        }
    }
    out
}

/// The text of a data label: the parts the options ask for, in Excel's
/// order, joined by the separator.
pub(crate) fn label_text(p: &Painter, series: usize, point: usize, percent: Option<f64>, labels: &crate::chart::DataLabels) -> String {
    let sd = &p.data.series[series];
    let locale = p.options.locale;
    let xy = p.chart.kind.is_xy();
    let code = labels.number_format.as_deref().unwrap_or(&sd.format);
    let mut parts: Vec<String> = Vec::new();
    if labels.series_name {
        parts.push(p.series_name(series));
    }
    if labels.category {
        if xy {
            if let Some(Some(x)) = sd.x_values.get(point) {
                parts.push(fmt_value(*x, &p.data.x_format, locale));
            }
        } else if let Some(name) = p.data.categories.get(point) {
            parts.push(name.clone());
        }
    }
    if labels.value
        && let Some(Some(v)) = sd.values.get(point)
    {
        parts.push(fmt_value(*v, code, locale));
    }
    if labels.percent
        && let Some(share) = percent
    {
        parts.push(fmt_value(share, labels.number_format.as_deref().unwrap_or("0%"), locale));
    }
    if labels.bubble_size
        && let Some(Some(v)) = sd.sizes.get(point)
    {
        parts.push(fmt_value(*v, "General", locale));
    }
    let auto = format!("{} ", locale.arg_sep);
    parts.join(labels.separator.as_deref().unwrap_or(&auto))
}

/// The series' own formatting of a point, for callers that have the
/// series at hand.
pub(crate) fn explosion_of(series: &Series, point: usize) -> u32 {
    series.point(point).and_then(|p| p.explosion).unwrap_or(series.explosion).min(400)
}

#[cfg(test)]
mod tests;
