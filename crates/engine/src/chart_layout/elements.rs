//! The parts of a chart a user can point at: which element every primitive
//! of a layout belongs to, what lies under a point, and where the handles
//! of a selected element go.
//!
//! The layout tags what it draws as it goes (`Painter::tag`); nothing here
//! knows how a chart is laid out.

use super::{Prim, Scale, Seg, TextAnchor, TextMeasure, line_height};

/// One of the four axes of a chart (see `chart::Axes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AxisId {
    Category,
    Value,
    SecondaryCategory,
    SecondaryValue,
}

impl AxisId {
    pub const ALL: [AxisId; 4] = [AxisId::Category, AxisId::Value, AxisId::SecondaryCategory, AxisId::SecondaryValue];

    pub fn is_secondary(self) -> bool {
        matches!(self, AxisId::SecondaryCategory | AxisId::SecondaryValue)
    }

    /// The axis of the categories (the X axis of a scatter chart).
    pub fn is_category(self) -> bool {
        matches!(self, AxisId::Category | AxisId::SecondaryCategory)
    }

    pub fn of(self, axes: &crate::chart::Axes) -> &crate::chart::Axis {
        match self {
            AxisId::Category => &axes.category,
            AxisId::Value => &axes.value,
            AxisId::SecondaryCategory => &axes.secondary_category,
            AxisId::SecondaryValue => &axes.secondary_value,
        }
    }

    pub fn of_mut(self, axes: &mut crate::chart::Axes) -> &mut crate::chart::Axis {
        match self {
            AxisId::Category => &mut axes.category,
            AxisId::Value => &mut axes.value,
            AxisId::SecondaryCategory => &mut axes.secondary_category,
            AxisId::SecondaryValue => &mut axes.secondary_value,
        }
    }
}

/// A part of a chart that can be selected, formatted and deleted. Series,
/// points, trendlines and error bars are numbered as in the model
/// (`Chart::series`, `Series::trendlines`, `Series::error_bars`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ChartElement {
    #[default]
    ChartArea,
    PlotArea,
    Title,
    Legend,
    /// An entry of the legend, numbered as `Chart::legend_hidden` does.
    LegendEntry(u32),
    Axis(AxisId),
    AxisTitle(AxisId),
    MajorGridlines(AxisId),
    MinorGridlines(AxisId),
    Series(u32),
    /// A single point: (series, point).
    Point(u32, u32),
    /// The data labels of a series.
    DataLabels(u32),
    /// (series, trendline).
    Trendline(u32, u32),
    /// (series, error bars).
    ErrorBars(u32, u32),
    DataTable,
}

impl ChartElement {
    /// The series an element belongs to.
    pub fn series(self) -> Option<usize> {
        match self {
            ChartElement::Series(s)
            | ChartElement::Point(s, _)
            | ChartElement::DataLabels(s)
            | ChartElement::Trendline(s, _)
            | ChartElement::ErrorBars(s, _) => Some(s as usize),
            _ => None,
        }
    }

    /// What a first click on this element selects: a point selects its
    /// series, a legend entry the legend.
    pub fn parent(self) -> ChartElement {
        match self {
            ChartElement::Point(s, _) => ChartElement::Series(s),
            ChartElement::LegendEntry(_) => ChartElement::Legend,
            other => other,
        }
    }
}

impl ChartElement {
    /// The format of the text of an element that is, or has, text: a
    /// title, the labels of an axis, the legend, data labels.
    pub fn text(self, chart: &crate::chart::Chart) -> Option<&crate::chart::TextFormat> {
        match self {
            ChartElement::Title => Some(&chart.title_text),
            ChartElement::Legend | ChartElement::LegendEntry(_) => Some(&chart.legend_text),
            ChartElement::Axis(axis) => Some(&axis.of(&chart.axes).text),
            ChartElement::AxisTitle(axis) => Some(&axis.of(&chart.axes).title_text),
            ChartElement::DataLabels(s) => {
                let own = chart.series.get(s as usize).and_then(|series| series.labels.as_ref());
                Some(own.map_or(&chart.label_options.text, |labels| &labels.text))
            }
            _ => None,
        }
    }

    /// The size, in points, Excel gives the text of this element.
    pub fn text_points(self) -> f32 {
        match self {
            ChartElement::Title => super::TITLE_POINTS,
            ChartElement::AxisTitle(_) => super::AXIS_TITLE_POINTS,
            _ => super::LABEL_POINTS,
        }
    }

    /// Change the text of an element; with the chart area, every text of
    /// the chart, as Excel does. The labels of a series that follows the
    /// chart's become its own first. Returns whether there was a text.
    pub fn format_text(self, chart: &mut crate::chart::Chart, f: impl Fn(&mut crate::chart::TextFormat)) -> bool {
        match self {
            ChartElement::ChartArea => chart.text_formats_mut().into_iter().for_each(f),
            ChartElement::Title => f(&mut chart.title_text),
            ChartElement::Legend | ChartElement::LegendEntry(_) => f(&mut chart.legend_text),
            ChartElement::Axis(axis) => f(&mut axis.of_mut(&mut chart.axes).text),
            ChartElement::AxisTitle(axis) => f(&mut axis.of_mut(&mut chart.axes).title_text),
            ChartElement::DataLabels(s) => {
                let shared = chart.label_options.clone();
                let Some(series) = chart.series.get_mut(s as usize) else {
                    return false;
                };
                f(&mut series.labels.get_or_insert(shared).text);
            }
            _ => return false,
        }
        true
    }
}

/// A place where a selected element shows a handle.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    element: ChartElement,
    x: f32,
    y: f32,
    /// The point is a target of its own (a point of a line without
    /// markers has no shape to click on).
    pick: bool,
}

/// What a selected element shows: thin frames and round handles.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outline {
    /// (x0, y0, x1, y1)
    pub frames: Vec<[f32; 4]>,
    pub handles: Vec<(f32, f32)>,
}

/// A chart laid out, with what is needed to point at its parts.
#[derive(Clone, Debug, Default)]
pub struct ChartLayout {
    /// The primitives, in painting order.
    pub prims: Vec<Prim>,
    /// The element each primitive belongs to.
    pub tags: Vec<ChartElement>,
    /// The box (x0, y0, x1, y1) each primitive fills.
    extents: Vec<[f32; 4]>,
    /// Areas given by the layout itself (the plot area).
    boxes: Vec<(ChartElement, [f32; 4])>,
    anchors: Vec<Anchor>,
    /// The scales of the axes that measure numbers, as they were drawn.
    scales: Vec<(AxisId, Scale)>,
    /// (primitive, series, point) of every data label.
    labels: Vec<(usize, u32, u32)>,
    size: (f32, f32),
}

/// Collects tags while a chart is laid out.
#[derive(Default)]
pub(crate) struct Tagger {
    /// (index of the first primitive, element): the element of every
    /// primitive from there to the next entry.
    spans: Vec<(usize, ChartElement)>,
    boxes: Vec<(ChartElement, [f32; 4])>,
    anchors: Vec<Anchor>,
    scales: Vec<(AxisId, Scale)>,
    /// (primitive, series, point) of every data label.
    labels: Vec<(usize, u32, u32)>,
}

impl Tagger {
    pub fn label(&mut self, at: usize, series: u32, point: u32) {
        self.labels.push((at, series, point));
    }

    pub fn tag(&mut self, at: usize, element: ChartElement) {
        match self.spans.last_mut() {
            Some((start, last)) if *start == at => *last = element,
            Some((_, last)) if *last == element => {}
            _ => self.spans.push((at, element)),
        }
    }

    pub fn area(&mut self, element: ChartElement, rect: (f32, f32, f32, f32)) {
        self.boxes.push((element, [rect.0.min(rect.2), rect.1.min(rect.3), rect.0.max(rect.2), rect.1.max(rect.3)]));
    }

    pub fn anchor(&mut self, element: ChartElement, x: f32, y: f32, pick: bool) {
        self.anchors.push(Anchor { element, x, y, pick });
    }

    pub fn scale(&mut self, axis: AxisId, scale: &Scale) {
        self.scales.push((axis, scale.clone()));
    }

    pub fn finish(self, prims: Vec<Prim>, size: (f32, f32), measure: &dyn TextMeasure) -> ChartLayout {
        let mut tags = Vec::with_capacity(prims.len());
        let mut span = 0;
        for i in 0..prims.len() {
            while span + 1 < self.spans.len() && self.spans[span + 1].0 <= i {
                span += 1;
            }
            tags.push(match self.spans.get(span) {
                Some((start, element)) if *start <= i => *element,
                _ => ChartElement::ChartArea,
            });
        }
        let extents = prims.iter().map(|p| extent(p, measure)).collect();
        ChartLayout {
            prims,
            tags,
            extents,
            boxes: self.boxes,
            anchors: self.anchors,
            scales: self.scales,
            labels: self.labels,
            size,
        }
    }
}

/// The box a primitive fills.
fn extent(prim: &Prim, measure: &dyn TextMeasure) -> [f32; 4] {
    match prim {
        Prim::Rect { x, y, w, h, .. } => [*x, *y, x + w, y + h],
        Prim::Line { x0, y0, x1, y1, .. } => [x0.min(*x1), y0.min(*y1), x0.max(*x1), y0.max(*y1)],
        Prim::Path { segs, .. } => {
            let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for (x, y) in flatten(segs).into_iter().flatten() {
                b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            if b[0] > b[2] { [0.0; 4] } else { b }
        }
        Prim::Text(run) => {
            let w = measure.width_styled(&run.text, run.size, run.bold, run.italic, run.font.as_deref());
            let h = line_height(run.size);
            let left = match run.anchor {
                TextAnchor::Start => run.x,
                TextAnchor::Middle => run.x - w / 2.0,
                TextAnchor::End => run.x - w,
            };
            if run.rotation == 0.0 {
                [left, run.y - h / 2.0, left + w, run.y + h / 2.0]
            } else {
                // Turned around its anchor: the corners of the box, turned.
                let (sin, cos) = run.rotation.to_radians().sin_cos();
                let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
                for (dx, dy) in [(left - run.x, -h / 2.0), (left - run.x + w, -h / 2.0), (left - run.x + w, h / 2.0), (left - run.x, h / 2.0)] {
                    let (x, y) = (run.x + dx * cos - dy * sin, run.y + dx * sin + dy * cos);
                    b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                }
                b
            }
        }
    }
}

/// The outlines of a path as polylines, curves cut into short pieces.
fn flatten(segs: &[Seg]) -> Vec<Vec<(f32, f32)>> {
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut run: Vec<(f32, f32)> = Vec::new();
    for seg in segs {
        match *seg {
            Seg::Move(x, y) => {
                if run.len() > 1 {
                    out.push(std::mem::take(&mut run));
                }
                run = vec![(x, y)];
            }
            Seg::Line(x, y) => run.push((x, y)),
            Seg::Cubic(ax, ay, bx, by, x, y) => {
                let Some(&(sx, sy)) = run.last() else { continue };
                for k in 1..=8 {
                    let t = k as f32 / 8.0;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    run.push((a * sx + b * ax + c * bx + d * x, a * sy + b * ay + c * by + d * y));
                }
            }
            Seg::Close => {
                if let Some(first) = run.first().copied() {
                    run.push(first);
                }
            }
        }
    }
    if run.len() > 1 {
        out.push(run);
    }
    out
}

fn inside_polygon(points: &[(f32, f32)], x: f32, y: f32) -> bool {
    let mut inside = false;
    let mut j = points.len().wrapping_sub(1);
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from a point to a segment.
fn segment_distance(a: (f32, f32), b: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((x - a.0 - t * dx).powi(2) + (y - a.1 - t * dy).powi(2)).sqrt()
}

fn in_box(b: &[f32; 4], x: f32, y: f32, slack: f32) -> bool {
    x >= b[0] - slack && x <= b[2] + slack && y >= b[1] - slack && y <= b[3] + slack
}

/// How near a line the pointer must be to pick it.
const REACH: f32 = 4.0;

impl ChartLayout {
    /// Whether a point lies on a primitive.
    fn on_prim(&self, i: usize, x: f32, y: f32) -> bool {
        if !in_box(&self.extents[i], x, y, REACH) {
            return false;
        }
        match &self.prims[i] {
            Prim::Rect { .. } | Prim::Text(_) => in_box(&self.extents[i], x, y, 1.0),
            Prim::Line { x0, y0, x1, y1, stroke } => segment_distance((*x0, *y0), (*x1, *y1), x, y) <= REACH.max(stroke.width / 2.0),
            Prim::Path { segs, fill, stroke } => {
                let runs = flatten(segs);
                if fill.is_some() && runs.iter().any(|run| inside_polygon(run, x, y)) {
                    return true;
                }
                let reach = REACH.max(stroke.map_or(0.0, |s| s.width / 2.0));
                stroke.is_some() && runs.iter().any(|run| run.windows(2).any(|w| segment_distance(w[0], w[1], x, y) <= reach))
            }
        }
    }

    /// The box around everything drawn for an element.
    pub fn bounds(&self, element: ChartElement) -> Option<[f32; 4]> {
        if element == ChartElement::ChartArea {
            return Some([0.0, 0.0, self.size.0, self.size.1]);
        }
        if let Some((_, b)) = self.boxes.iter().find(|(e, _)| *e == element) {
            return Some(*b);
        }
        let mut out: Option<[f32; 4]> = None;
        for (i, tag) in self.tags.iter().enumerate() {
            let own = match element {
                ChartElement::Legend => matches!(tag, ChartElement::Legend | ChartElement::LegendEntry(_)),
                ChartElement::Series(s) => matches!(tag, ChartElement::Series(t) | ChartElement::Point(t, _) if *t == s),
                _ => *tag == element,
            };
            if own {
                let b = self.extents[i];
                out = Some(match out {
                    Some(o) => [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])],
                    None => b,
                });
            }
        }
        // Text is given some air, as Excel's frames do.
        let pad = match element {
            ChartElement::Title | ChartElement::AxisTitle(_) | ChartElement::Legend => super::FRAME_PAD,
            _ => 0.0,
        };
        out.map(|b| [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad])
    }

    /// The element under a point (pixels from the corner of the chart).
    /// A point of a series and an entry of the legend are reported as
    /// such: `ChartElement::parent` gives what a first click selects.
    pub fn hit(&self, x: f32, y: f32) -> ChartElement {
        if x < 0.0 || y < 0.0 || x > self.size.0 || y > self.size.1 {
            return ChartElement::ChartArea;
        }
        let boxed = |wanted: &dyn Fn(ChartElement) -> bool| -> Option<ChartElement> {
            let mut seen: Vec<ChartElement> = Vec::new();
            for tag in &self.tags {
                if wanted(*tag) && !seen.contains(tag) {
                    seen.push(*tag);
                    if self.bounds(*tag).is_some_and(|b| in_box(&b, x, y, 0.0)) {
                        return Some(*tag);
                    }
                }
            }
            None
        };
        // What is written over the plot comes first.
        if let Some(hit) = boxed(&|e| matches!(e, ChartElement::Title | ChartElement::AxisTitle(_) | ChartElement::LegendEntry(_))) {
            return hit;
        }
        if self.bounds(ChartElement::Legend).is_some_and(|b| in_box(&b, x, y, 0.0)) {
            return ChartElement::Legend;
        }
        // Then the marks, topmost first; a point without a shape of its
        // own is picked by its place.
        for anchor in self.anchors.iter().rev().filter(|a| a.pick) {
            if (anchor.x - x).abs() <= REACH + 1.0 && (anchor.y - y).abs() <= REACH + 1.0 {
                return anchor.element;
            }
        }
        for i in (0..self.prims.len()).rev() {
            let mark = matches!(
                self.tags[i],
                ChartElement::Series(_)
                    | ChartElement::Point(..)
                    | ChartElement::DataLabels(_)
                    | ChartElement::Trendline(..)
                    | ChartElement::ErrorBars(..)
            );
            if mark && self.on_prim(i, x, y) {
                return self.tags[i];
            }
        }
        if let Some(hit) = boxed(&|e| matches!(e, ChartElement::Axis(_) | ChartElement::DataTable)) {
            return hit;
        }
        for i in (0..self.prims.len()).rev() {
            if matches!(self.tags[i], ChartElement::MajorGridlines(_) | ChartElement::MinorGridlines(_)) && self.on_prim(i, x, y) {
                return self.tags[i];
            }
        }
        match self.bounds(ChartElement::PlotArea) {
            Some(b) if in_box(&b, x, y, 0.0) => ChartElement::PlotArea,
            _ => ChartElement::ChartArea,
        }
    }

    /// Every element the chart shows, in the order they were drawn; points
    /// and legend entries are left out.
    pub fn elements(&self) -> Vec<ChartElement> {
        let mut out = vec![ChartElement::ChartArea];
        let mut add = |element: ChartElement| {
            let element = element.parent();
            if !out.contains(&element) {
                out.push(element);
            }
        };
        for (element, _) in &self.boxes {
            add(*element);
        }
        for tag in &self.tags {
            add(*tag);
        }
        out
    }

    /// Width and height of the chart area the layout was made for.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// The data label under a point: (series, point).
    pub fn data_label_at(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        self.labels.iter().rev().find(|(i, ..)| in_box(&self.extents[*i], x, y, 2.0)).map(|(_, s, k)| (*s, *k))
    }

    /// The box of the data label of a point.
    pub fn data_label_box(&self, series: u32, point: u32) -> Option<[f32; 4]> {
        self.labels.iter().find(|(_, s, k)| *s == series && *k == point).map(|(i, ..)| self.extents[*i])
    }

    /// The scale an axis was drawn with: its bounds and units, automatic
    /// or not. `None` for an axis of categories.
    pub fn scale(&self, axis: AxisId) -> Option<&Scale> {
        self.scales.iter().find(|(id, _)| *id == axis).map(|(_, scale)| scale)
    }

    /// The frames and handles a selected element shows.
    pub fn outline(&self, element: ChartElement) -> Outline {
        let mut out = Outline::default();
        let own = |tag: ChartElement| match element {
            ChartElement::Series(s) => matches!(tag, ChartElement::Series(t) | ChartElement::Point(t, _) if t == s),
            _ => tag == element,
        };
        out.handles = self.anchors.iter().filter(|a| own(a.element)).map(|a| (a.x, a.y)).collect();
        if !out.handles.is_empty() {
            return out;
        }
        match element {
            ChartElement::ChartArea => {}
            // Every label has a frame of its own.
            ChartElement::DataLabels(_) => {
                for (i, tag) in self.tags.iter().enumerate() {
                    if *tag == element && matches!(self.prims[i], Prim::Text(_)) {
                        let b = self.extents[i];
                        out.frames.push([b[0] - 2.0, b[1], b[2] + 2.0, b[3]]);
                    }
                }
            }
            _ => out.frames.extend(self.bounds(element)),
        }
        for b in &out.frames {
            out.handles.extend([(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])]);
            // Areas that can be resized in Excel show handles on their edges too.
            if matches!(element, ChartElement::PlotArea | ChartElement::Legend) {
                let (mx, my) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
                out.handles.extend([(mx, b[1]), (b[2], my), (mx, b[3]), (b[0], my)]);
            }
        }
        out
    }
}
