//! Reading charts: where they sit (the drawing part of a sheet) and what
//! they show (chart parts), from files written by any application.
//!
//! A chart part is first read as it is written (plots, series and axes,
//! the `Raw...` types), then turned into the engine's model. There,
//! whatever the file spells out the way Excel spells what it does by
//! itself is automatic again, so that a chart written by this crate comes
//! back as the chart it was. Chart types the engine does not have are
//! read as the closest one it has (see [`plot_of`]).

use super::{
    DASHES, DISPLAY_UNITS, EMU_PER_PT, EMU_PER_PX, FRAME_GREY, LABEL_POSITIONS, LEGACY_MARKER_SIZE,
    LEGACY_MARKERS, MARKERS, OUTLINE_WIDTH, Plot, SLICE_OUTLINE_WIDTH, TICK_LABELS, TICK_MARKS,
    TREND_WIDTH, Track, foreign, innermost_level, is_stroked, list_separator, named, scheme_slot,
};
use crate::xlsx::color::{Palette, adjust_luminance, blend, parse_rgb};
use crate::xlsx::formula::from_file_text;
use crate::xlsx::xml::{Event, Reader, Tag, decode_cell_text};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::chart::{
    Axes, Axis, AxisCrossing, BlanksAs, Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE,
    DEFAULT_HOLE_SIZE, Dash, DataLabels, DataTable, DisplayUnit, ErrorAmount, ErrorBars,
    ErrorDirection, Family, Fill, ForeignChart, Grouping, LegendPos, Line, Marker, MarkerShape,
    PointFormat, Series, SeriesName, ShapeFormat, TickMark, TrendKind, Trendline, View3D,
    palette_color,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::{Color, Locale};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Largest width or height given to a chart, in pixels (Excel stops a
/// little above 16 000).
const MAX_CHART_PX: f64 = 20_000.0;

/// Hand every child element of `parent` to `f`. `f` has to consume the
/// child: parse it up to its end tag, or [`skip`](Reader::skip) it.
fn each_child<'a>(
    r: &mut Reader<'a>,
    parent: &Tag<'a>,
    mut f: impl FnMut(&mut Reader<'a>, &Tag<'a>),
) {
    if parent.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) => f(r, &t),
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
}

/// The `val` attribute of a property element such as `<c:barDir val="col"/>`.
fn val<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<Cow<'a, str>> {
    let value = tag.attr("val");
    r.skip(tag);
    value
}

/// A boolean property; `<c:delete/>` without a value means "true".
fn flag<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> bool {
    let on = tag.attr_bool("val").unwrap_or(true);
    r.skip(tag);
    on
}

// ---- drawing -------------------------------------------------------------------

/// A corner pinned to a cell: the cell and the distance into it, in pixels.
#[derive(Clone, Copy, Default)]
struct Corner {
    col: u32,
    row: u32,
    x: f64,
    y: f64,
}

/// A frame of a drawing part that shows charts.
#[derive(Default)]
pub(crate) struct Frame {
    from: Option<Corner>,
    to: Option<Corner>,
    /// Position on the sheet, for frames not tied to cells.
    pos: Option<(f64, f64)>,
    /// Size, for frames with a single corner.
    ext: Option<(f64, f64)>,
    /// Relationship ids of the charts shown in the frame.
    pub charts: Vec<String>,
}

fn emu_to_px(text: &str) -> f64 {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|emu| emu.is_finite())
        .map_or(0.0, |emu| emu / EMU_PER_PX)
}

fn parse_corner<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Corner {
    let mut m = Corner::default();
    let index = |text: &str, count: u32| text.trim().parse::<u32>().unwrap_or(0).min(count - 1);
    each_child(r, tag, |r, t| match t.name {
        "col" => m.col = index(&r.text(t), MAX_COLS),
        "row" => m.row = index(&r.text(t), MAX_ROWS),
        "colOff" => m.x = emu_to_px(&r.text(t)),
        "rowOff" => m.y = emu_to_px(&r.text(t)),
        _ => r.skip(t),
    });
    m
}

fn parse_frame<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Frame {
    let mut frame = Frame::default();
    let pair = |t: &Tag, a: &str, b: &str| {
        let px = |name: &str| t.attr(name).map_or(0.0, |v| emu_to_px(&v));
        (px(a), px(b))
    };
    each_child(r, tag, |r, t| match t.name {
        "from" => frame.from = Some(parse_corner(r, t)),
        "to" => frame.to = Some(parse_corner(r, t)),
        "pos" => {
            frame.pos = Some(pair(t, "x", "y"));
            r.skip(t);
        }
        "ext" => {
            frame.ext = Some(pair(t, "cx", "cy"));
            r.skip(t);
        }
        // The content: a graphic frame referring to a chart part, possibly
        // inside a group. Pictures and shapes refer to no chart.
        _ => {
            if t.empty {
                return;
            }
            let mut depth = 0usize;
            loop {
                match r.next() {
                    Event::Open(inner) => {
                        if inner.name == "chart"
                            && let Some(id) = inner.attr_local("id")
                        {
                            frame.charts.push(id.into_owned());
                        }
                        if !inner.empty {
                            depth += 1;
                        }
                    }
                    Event::Close(_) if depth == 0 => break,
                    Event::Close(_) => depth -= 1,
                    Event::Eof => break,
                    Event::Text(_) => {}
                }
            }
        }
    });
    frame
}

/// The frames of a drawing part that show charts, in drawing order.
pub(crate) fn parse_drawing(xml: &str) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                // Frames holding newer kinds of content are wrapped in a
                // choice between the real thing and a picture of it.
                "wsDr" | "AlternateContent" | "Choice" => {}
                "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor" => {
                    let frame = parse_frame(&mut r, &t);
                    if !frame.charts.is_empty() {
                        out.push(frame);
                    }
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Where a frame puts a chart, in the engine's terms.
pub(crate) struct Placement {
    /// (row, col)
    pub anchor: (u32, u32),
    /// Pixels (x, y)
    pub offset: (f32, f32),
    /// Pixels (width, height)
    pub size: (f32, f32),
}

impl Frame {
    pub fn placement(&self, cols: &Track, rows: &Track) -> Placement {
        let from = self.from.unwrap_or_else(|| {
            // Not tied to a cell: the cell under the top-left corner.
            let (x, y) = self.pos.unwrap_or_default();
            let (col, x) = cols.locate(0, x);
            let (row, y) = rows.locate(0, y);
            Corner { col, row, x, y }
        });
        let (width, height) = match (self.from, self.to) {
            (Some(_), Some(to)) => (
                cols.span(from.col, to.col) + to.x - from.x,
                rows.span(from.row, to.row) + to.y - from.y,
            ),
            _ => self.ext.unwrap_or_default(),
        };
        let length = |px: f64, fallback: f32| {
            if px.is_finite() && px >= 1.0 {
                px.min(MAX_CHART_PX) as f32
            } else {
                fallback
            }
        };
        let inside = |px: f64| px.clamp(0.0, MAX_CHART_PX) as f32;
        Placement {
            anchor: (from.row, from.col),
            offset: (inside(from.x), inside(from.y)),
            size: (
                length(width, DEFAULT_CHART_SIZE.0),
                length(height, DEFAULT_CHART_SIZE.1),
            ),
        }
    }
}

// ---- values ----------------------------------------------------------------------

/// The number in the `val` attribute of a property element. Strict files
/// write some of them as percentages.
fn number<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<f64> {
    let value = tag
        .attr("val")
        .and_then(|v| v.trim().trim_end_matches('%').parse::<f64>().ok())
        .filter(|n| n.is_finite());
    r.skip(tag);
    value
}

/// A whole number, brought within the bounds the format gives it.
fn integer<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, min: i64, max: i64) -> Option<i64> {
    number(r, tag).map(|n| n.round().clamp(min as f64, max as f64) as i64)
}

/// A reference of a chart part: a plain range on a sheet of the workbook.
/// Anything else (names, unions, other workbooks) has no place in the model.
fn parse_ref(text: &str) -> Option<RefExpr> {
    let mut expr = from_file_text(text)?;
    while let Expr::Paren(inner) = expr {
        expr = *inner;
    }
    match expr {
        Expr::Ref(r) => Some(r),
        _ => None,
    }
}

/// `<c:strRef>`: the reference, and the text last seen there (several cells
/// read as one text, as Excel shows them).
fn parse_str_ref<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> (Option<RefExpr>, Option<String>) {
    let mut reference = None;
    let mut cached: Option<String> = None;
    each_child(r, tag, |r, t| match t.name {
        "f" => reference = parse_ref(&r.text(t)),
        "strCache" => each_child(r, t, |r, t| match t.name {
            "pt" => each_child(r, t, |r, t| match t.name {
                "v" => {
                    let text = r.text(t);
                    let cached = cached.get_or_insert_with(String::new);
                    if !cached.is_empty() {
                        cached.push(' ');
                    }
                    cached.push_str(&decode_cell_text(&text));
                }
                _ => r.skip(t),
            }),
            _ => r.skip(t),
        }),
        _ => r.skip(t),
    });
    (reference, cached)
}

/// The range behind `<c:cat>`, `<c:val>`, `<c:xVal>`, `<c:yVal>` or
/// `<c:bubbleSize>`. Data typed into the chart itself (`numLit`, `strLit`)
/// has none.
fn parse_data_ref<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<RefExpr> {
    let mut reference = None;
    each_child(r, tag, |r, t| match t.name {
        "numRef" | "strRef" | "multiLvlStrRef" => each_child(r, t, |r, t| match t.name {
            "f" => reference = parse_ref(&r.text(t)),
            _ => r.skip(t),
        }),
        _ => r.skip(t),
    });
    reference
}

/// `<c:tx>` of a series: a cell, or text typed into the chart.
fn parse_series_name<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> SeriesName {
    let mut name = SeriesName::None;
    each_child(r, tag, |r, t| match t.name {
        "strRef" => {
            name = match parse_str_ref(r, t) {
                (Some(reference), _) => SeriesName::Cell(reference),
                // A name the model cannot follow keeps its last text.
                (None, Some(text)) => SeriesName::Text(text),
                (None, None) => SeriesName::None,
            }
        }
        "v" => name = SeriesName::Text(decode_cell_text(&r.text(t)).into_owned()),
        _ => r.skip(t),
    });
    name
}

// ---- text ------------------------------------------------------------------------

/// The text of a DrawingML text body: the runs of a paragraph joined, one
/// line per paragraph.
fn parse_rich<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> String {
    let mut text = String::new();
    let mut paragraphs = 0;
    each_child(r, tag, |r, t| match t.name {
        "p" => {
            if paragraphs > 0 {
                text.push('\n');
            }
            paragraphs += 1;
            each_child(r, t, |r, t| match t.name {
                "r" | "fld" => each_child(r, t, |r, t| match t.name {
                    "t" => text.push_str(&r.text(t)),
                    _ => r.skip(t),
                }),
                "br" => {
                    text.push('\n');
                    r.skip(t);
                }
                _ => r.skip(t),
            });
        }
        _ => r.skip(t),
    });
    text
}

struct Title {
    text: ChartTitle,
    /// Drawn over the plot.
    overlay: bool,
}

/// `<c:title>`: text typed into the chart, the text of the cell the title
/// is linked to, or (no text at all) the title Excel makes up.
fn parse_title<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Title {
    let mut title = Title {
        text: ChartTitle::Auto,
        overlay: false,
    };
    each_child(r, tag, |r, t| match t.name {
        "tx" => each_child(r, t, |r, t| match t.name {
            "rich" => title.text = ChartTitle::Text(parse_rich(r, t)),
            "strRef" => {
                if let (_, Some(text)) = parse_str_ref(r, t) {
                    title.text = ChartTitle::Text(text);
                }
            }
            _ => r.skip(t),
        }),
        "overlay" => title.overlay = flag(r, t),
        _ => r.skip(t),
    });
    title
}

struct Legend {
    pos: LegendPos,
    overlay: bool,
    /// Entries deleted from the legend.
    hidden: Vec<u32>,
}

fn parse_legend<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Legend {
    let mut legend = Legend {
        // A legend that does not say where it is sits on the right.
        pos: LegendPos::Right,
        overlay: false,
        hidden: Vec::new(),
    };
    each_child(r, tag, |r, t| match t.name {
        "legendPos" => {
            legend.pos = match val(r, t).as_deref() {
                Some("b") => LegendPos::Bottom,
                Some("t") => LegendPos::Top,
                Some("l") => LegendPos::Left,
                Some("tr") => LegendPos::TopRight,
                _ => LegendPos::Right,
            }
        }
        "legendEntry" => {
            let (mut index, mut deleted) = (None, false);
            each_child(r, t, |r, t| match t.name {
                "idx" => index = integer(r, t, 0, u32::MAX as i64),
                "delete" => deleted = flag(r, t),
                // An entry with a font of its own.
                _ => r.skip(t),
            });
            if let Some(index) = index
                && deleted
            {
                legend.hidden.push(index as u32);
            }
        }
        "overlay" => legend.overlay = flag(r, t),
        _ => r.skip(t),
    });
    // An entry is deleted once.
    let mut seen = BTreeSet::new();
    legend.hidden.retain(|index| seen.insert(*index));
    legend
}

/// `<c:dLbls>`: what the labels show, or `None` when they are deleted or
/// show nothing. Labels of single points (`dLbl`) are not looked at.
fn parse_labels<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<DataLabels> {
    let mut deleted = false;
    let mut labels = DataLabels {
        value: false,
        leader_lines: false,
        ..DataLabels::default()
    };
    each_child(r, tag, |r, t| match t.name {
        "delete" => deleted = flag(r, t),
        "numFmt" => {
            if t.attr_bool("sourceLinked") != Some(true) {
                labels.number_format = t.attr("formatCode").map(Cow::into_owned);
            }
            r.skip(t);
        }
        "dLblPos" => {
            labels.position = val(r, t)
                .and_then(|v| named(&LABEL_POSITIONS, &v))
                .unwrap_or_default()
        }
        "showLegendKey" => labels.legend_key = flag(r, t),
        "showVal" => labels.value = flag(r, t),
        "showCatName" => labels.category = flag(r, t),
        "showSerName" => labels.series_name = flag(r, t),
        "showPercent" => labels.percent = flag(r, t),
        "showBubbleSize" => labels.bubble_size = flag(r, t),
        "separator" => labels.separator = Some(r.text(t).into_owned()),
        "showLeaderLines" => labels.leader_lines = flag(r, t),
        _ => r.skip(t),
    });
    (!deleted && labels.any()).then_some(labels)
}

// ---- colours and lines -----------------------------------------------------------

/// `<a:solidFill>` (or any element holding a colour): an RGB, theme or
/// system colour, lightened or darkened by the adjustments nested in it.
/// Transparency is ignored.
pub(crate) fn parse_solid_fill<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<Color> {
    let mut color = None;
    each_child(r, tag, |r, t| {
        let mut rgb = match t.name {
            "srgbClr" => t.attr("val").and_then(|v| parse_rgb(&v)),
            "schemeClr" => t
                .attr("val")
                .and_then(|v| scheme_slot(&v))
                .map(|slot| palette.theme[slot]),
            "sysClr" => t.attr("lastClr").and_then(|v| parse_rgb(&v)).or_else(|| {
                match t.attr("val").as_deref() {
                    Some("windowText") => Some(0x000000),
                    Some("window") => Some(0xFFFFFF),
                    _ => None,
                }
            }),
            _ => None,
        };
        // Lightness is scaled, then shifted; both in thousandths of a
        // percent.
        let (mut scale, mut shift) = (1.0, 0.0);
        each_child(r, t, |r, adjustment| {
            let amount = adjustment
                .attr_f64("val")
                .map(|v| (v / 100_000.0).clamp(0.0, 1.0));
            if let (Some(base), Some(amount)) = (rgb, amount) {
                match adjustment.name {
                    "lumMod" => {
                        scale *= amount;
                        shift *= amount;
                    }
                    "lumOff" => shift += amount,
                    "tint" => rgb = Some(blend(base, 0xFFFFFF, amount)),
                    "shade" => rgb = Some(blend(base, 0x000000, amount)),
                    _ => {}
                }
            }
            r.skip(adjustment);
        });
        if (scale, shift) != (1.0, 0.0) {
            rgb = rgb.map(|base| adjust_luminance(base, scale, shift));
        }
        if color.is_none() {
            color = rgb.map(Color);
        }
    });
    color
}

/// How a file paints a surface or a line.
#[derive(Clone, Copy, PartialEq, Default)]
enum Paint {
    /// It does not say: the application does what it does by itself.
    #[default]
    Automatic,
    /// `a:noFill`.
    Nothing,
    Color(Color),
}

impl Paint {
    fn color(self) -> Option<Color> {
        match self {
            Paint::Color(color) => Some(color),
            _ => None,
        }
    }
}

/// `<a:ln>`.
#[derive(Clone, Copy, Default)]
struct Stroke {
    paint: Paint,
    /// In points.
    width: Option<f32>,
    dash: Dash,
}

/// `<c:spPr>`.
#[derive(Clone, Copy, Default)]
struct Shape {
    fill: Paint,
    line: Option<Stroke>,
}

/// A fill element, read (`None`, and nothing read, for any other element).
/// The model has plain colours only: a gradient is its first colour, a
/// pattern its foreground, a picture is automatic.
fn parse_paint<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<Paint> {
    let mut color = None;
    match tag.name {
        "noFill" => {
            r.skip(tag);
            return Some(Paint::Nothing);
        }
        "solidFill" => color = parse_solid_fill(r, tag, palette),
        "gradFill" => each_child(r, tag, |r, t| match t.name {
            "gsLst" => each_child(r, t, |r, t| match t.name {
                "gs" if color.is_none() => color = parse_solid_fill(r, t, palette),
                _ => r.skip(t),
            }),
            _ => r.skip(t),
        }),
        "pattFill" => each_child(r, tag, |r, t| match t.name {
            "fgClr" => color = parse_solid_fill(r, t, palette),
            _ => r.skip(t),
        }),
        "blipFill" | "grpFill" => r.skip(tag),
        _ => return None,
    }
    Some(color.map_or(Paint::Automatic, Paint::Color))
}

fn parse_shape<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Shape {
    let mut shape = Shape::default();
    each_child(r, tag, |r, t| match t.name {
        "ln" => {
            let mut stroke = Stroke {
                width: t
                    .attr_f64("w")
                    .filter(|w| *w >= 0.0)
                    .map(|w| (w / EMU_PER_PT) as f32),
                ..Stroke::default()
            };
            each_child(r, t, |r, t| match t.name {
                "prstDash" => {
                    stroke.dash = match val(r, t).as_deref() {
                        // The three presets Excel does not offer.
                        Some("dot") => Dash::RoundDot,
                        Some("sysDashDot") => Dash::DashDot,
                        Some("sysDashDotDot") => Dash::LongDashDotDot,
                        other => other.and_then(|v| named(&DASHES, v)).unwrap_or_default(),
                    }
                }
                "custDash" => {
                    stroke.dash = Dash::Dash;
                    r.skip(t);
                }
                _ => match parse_paint(r, t, palette) {
                    Some(paint) => stroke.paint = paint,
                    None => r.skip(t),
                },
            });
            shape.line = Some(stroke);
        }
        _ => match parse_paint(r, t, palette) {
            Some(paint) => shape.fill = paint,
            None => r.skip(t),
        },
    });
    shape
}

/// Widths that differ by less are the same width: files hold them in
/// units of 1/12700 of a point.
const WIDTH_TOLERANCE: f32 = 0.001;

/// A stroke in the model's terms, for something whose automatic line is
/// drawn or not, in that colour and that width. What the file spells out
/// but would be automatic anyway is automatic.
fn line_of(stroke: Option<Stroke>, auto_shown: bool, auto_color: Color, auto_width: f32) -> Line {
    let Some(stroke) = stroke else {
        return Line::default();
    };
    let shown = match stroke.paint {
        Paint::Nothing => false,
        Paint::Color(_) => true,
        Paint::Automatic => auto_shown,
    };
    if !shown {
        // A line that is not drawn has neither colour nor width.
        return Line {
            shown: auto_shown.then_some(false),
            ..Line::default()
        };
    }
    let color = stroke.paint.color().filter(|c| *c != auto_color);
    let width = stroke
        .width
        .filter(|w| (w - auto_width).abs() > WIDTH_TOLERANCE);
    Line {
        // Where there is no line unless told otherwise, a colour or a
        // width of its own says there is one.
        shown: (!auto_shown && color.is_none() && width.is_none()).then_some(true),
        color,
        width,
        dash: stroke.dash,
    }
}

// ---- series ----------------------------------------------------------------------

/// `<c:marker>` of a series or of a point.
#[derive(Clone, Copy)]
struct RawMarker {
    /// `None`: left to the application.
    symbol: Option<MarkerShape>,
    size: Option<u8>,
    shape: Shape,
}

fn parse_marker<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawMarker {
    let mut marker = RawMarker {
        symbol: None,
        size: None,
        shape: Shape::default(),
    };
    each_child(r, tag, |r, t| match t.name {
        "symbol" => {
            marker.symbol = match val(r, t).as_deref() {
                // A picture is no shape the engine draws.
                Some("picture") => Some(MarkerShape::Circle),
                other => other.and_then(|v| named(&MARKERS, v)),
            }
        }
        "size" => marker.size = integer(r, t, 2, 72).map(|n| n as u8),
        "spPr" => marker.shape = parse_shape(r, t, palette),
        _ => r.skip(t),
    });
    marker
}

/// `<c:dPt>`.
struct RawPoint {
    index: u32,
    explosion: Option<u32>,
    shape: Shape,
    marker: Option<RawMarker>,
}

fn parse_point<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<RawPoint> {
    let mut index = None;
    let mut point = RawPoint {
        index: 0,
        explosion: None,
        shape: Shape::default(),
        marker: None,
    };
    each_child(r, tag, |r, t| match t.name {
        "idx" => index = integer(r, t, 0, u32::MAX as i64),
        "explosion" => point.explosion = integer(r, t, 0, u32::MAX as i64).map(|n| n as u32),
        "spPr" => point.shape = parse_shape(r, t, palette),
        "marker" => point.marker = Some(parse_marker(r, t, palette)),
        _ => r.skip(t),
    });
    point.index = index? as u32;
    Some(point)
}

/// `<c:trendline>`: the trendline, and its line as the file has it.
struct RawTrend {
    trend: Trendline,
    stroke: Option<Stroke>,
}

fn parse_trendline<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawTrend {
    let mut trend = Trendline::new(TrendKind::Linear);
    let mut stroke = None;
    let mut kind = None;
    let (mut order, mut period) = (2, 2);
    each_child(r, tag, |r, t| match t.name {
        "name" => trend.name = Some(r.text(t).into_owned()),
        "spPr" => stroke = parse_shape(r, t, palette).line,
        "trendlineType" => kind = val(r, t),
        "order" => order = integer(r, t, 2, 6).unwrap_or(2),
        "period" => period = integer(r, t, 2, u32::MAX as i64).unwrap_or(2),
        "forward" => trend.forward = number(r, t).unwrap_or(0.0).max(0.0),
        "backward" => trend.backward = number(r, t).unwrap_or(0.0).max(0.0),
        "intercept" => trend.intercept = number(r, t),
        "dispRSqr" => trend.show_r2 = flag(r, t),
        "dispEq" => trend.show_equation = flag(r, t),
        // The label of the equation, extensions.
        _ => r.skip(t),
    });
    trend.kind = match kind.as_deref() {
        Some("exp") => TrendKind::Exponential,
        Some("log") => TrendKind::Logarithmic,
        Some("poly") => TrendKind::Polynomial(order as u8),
        Some("power") => TrendKind::Power,
        Some("movingAvg") => TrendKind::MovingAverage(period as u32),
        _ => TrendKind::Linear,
    };
    // What a kind has no use for is not kept: Excel would refuse it back.
    match trend.kind {
        TrendKind::Linear | TrendKind::Polynomial(_) => {}
        TrendKind::Exponential => trend.intercept = trend.intercept.filter(|at| *at > 0.0),
        TrendKind::Logarithmic | TrendKind::Power => trend.intercept = None,
        TrendKind::MovingAverage(_) => {
            trend = Trendline {
                kind: trend.kind,
                name: trend.name,
                ..Trendline::new(trend.kind)
            }
        }
    }
    RawTrend { trend, stroke }
}

/// `<c:errBars>`. `None` for bars whose amounts are read from cells, which
/// the model does not have.
fn parse_error_bars<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<ErrorBars> {
    let mut bars = ErrorBars {
        horizontal: false,
        direction: ErrorDirection::Both,
        amount: ErrorAmount::Fixed(0.0),
        end_cap: true,
    };
    let mut kind = None;
    let mut amount = None;
    each_child(r, tag, |r, t| match t.name {
        "errDir" => bars.horizontal = val(r, t).as_deref() == Some("x"),
        "errBarType" => {
            bars.direction = match val(r, t).as_deref() {
                Some("plus") => ErrorDirection::Plus,
                Some("minus") => ErrorDirection::Minus,
                _ => ErrorDirection::Both,
            }
        }
        "errValType" => kind = val(r, t),
        "noEndCap" => bars.end_cap = !flag(r, t),
        "val" => amount = number(r, t).map(|n| n.max(0.0)),
        _ => r.skip(t),
    });
    // Amounts left out are those Excel proposes.
    bars.amount = match kind.as_deref() {
        Some("cust") => return None,
        Some("percentage") => ErrorAmount::Percent(amount.unwrap_or(5.0)),
        Some("stdDev") => ErrorAmount::StdDev(amount.unwrap_or(1.0)),
        Some("stdErr") => ErrorAmount::StdErr,
        _ => ErrorAmount::Fixed(amount.unwrap_or(1.0)),
    };
    Some(bars)
}

/// A series as its plot describes it.
struct RawSeries {
    /// Position among the series as they were created: what automatic
    /// formatting goes by.
    index: Option<u32>,
    /// Rank among all the series of the chart.
    order: Option<u32>,
    name: SeriesName,
    values: Option<RefExpr>,
    categories: Option<RefExpr>,
    sizes: Option<RefExpr>,
    shape: Shape,
    marker: Option<RawMarker>,
    smooth: Option<bool>,
    points: Vec<RawPoint>,
    explosion: u32,
    /// Labels of its own: `Some(None)` when they show nothing.
    labels: Option<Option<DataLabels>>,
    trendlines: Vec<RawTrend>,
    error_bars: Vec<ErrorBars>,
}

/// `<c:ser>` of any chart type.
fn parse_series<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawSeries {
    let mut s = RawSeries {
        index: None,
        order: None,
        name: SeriesName::None,
        values: None,
        categories: None,
        sizes: None,
        shape: Shape::default(),
        marker: None,
        smooth: None,
        points: Vec::new(),
        explosion: 0,
        labels: None,
        trendlines: Vec::new(),
        error_bars: Vec::new(),
    };
    each_child(r, tag, |r, t| match t.name {
        "idx" => s.index = integer(r, t, 0, u32::MAX as i64).map(|n| n as u32),
        "order" => s.order = integer(r, t, 0, u32::MAX as i64).map(|n| n as u32),
        "tx" => s.name = parse_series_name(r, t),
        "spPr" => s.shape = parse_shape(r, t, palette),
        "marker" => s.marker = Some(parse_marker(r, t, palette)),
        "smooth" => s.smooth = Some(flag(r, t)),
        "explosion" => {
            s.explosion = integer(r, t, 0, u32::MAX as i64).map_or(0, |n| n as u32);
        }
        "dPt" => s.points.extend(parse_point(r, t, palette)),
        "dLbls" => s.labels = Some(parse_labels(r, t)),
        "trendline" => s.trendlines.push(parse_trendline(r, t, palette)),
        "errBars" => s.error_bars.extend(parse_error_bars(r, t)),
        "cat" | "xVal" => s.categories = parse_data_ref(r, t),
        "val" | "yVal" => s.values = parse_data_ref(r, t),
        "bubbleSize" => s.sizes = parse_data_ref(r, t),
        // Pictures, shapes of 3-D bars, extensions.
        _ => r.skip(t),
    });
    s
}

// ---- plots -----------------------------------------------------------------------

/// A chart-type element and its series.
struct RawPlot {
    plot: Plot,
    /// The element is of a type the engine lacks and is read as its nearest
    /// relative: what its series look like means something else there.
    approximate: bool,
    series: Vec<RawSeries>,
    /// Labels of the series that have none of their own.
    labels: Option<DataLabels>,
    vary_colors: Option<bool>,
    gap_width: Option<u32>,
    overlap: Option<i32>,
    first_slice: Option<u32>,
    hole: Option<u32>,
    bubble_scale: Option<u32>,
    /// What the series that say nothing about their markers, their line or
    /// its curve get.
    auto_markers: bool,
    auto_lines: bool,
    auto_smooth: bool,
    /// The axes the series are measured on, by `c:axId`.
    axes: Vec<String>,
}

/// What a chart-type element draws: the family of its series, whether in
/// three dimensions, and whether this is only the nearest thing the engine
/// has. Stock and surface charts become lines; a pie with a second plot
/// is a pie.
fn plot_of(element: &str) -> Option<(Family, bool, bool)> {
    Some(match element {
        "barChart" => (Family::Column, false, false),
        "bar3DChart" => (Family::Column, true, false),
        "lineChart" => (Family::Line, false, false),
        "line3DChart" => (Family::Line, true, false),
        "stockChart" | "surfaceChart" | "surface3DChart" => (Family::Line, false, true),
        "areaChart" => (Family::Area, false, false),
        "area3DChart" => (Family::Area, true, false),
        "pieChart" => (Family::Pie, false, false),
        "pie3DChart" => (Family::Pie, true, false),
        "ofPieChart" => (Family::Pie, false, true),
        "doughnutChart" => (Family::Doughnut, false, false),
        "scatterChart" => (Family::Scatter, false, false),
        "bubbleChart" => (Family::Bubble, false, false),
        "radarChart" => (Family::Radar, false, false),
        _ => return None,
    })
}

fn parse_plot<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    (family, is_3d, approximate): (Family, bool, bool),
    palette: &Palette,
) -> RawPlot {
    let mut p = RawPlot {
        plot: Plot {
            family,
            grouping: Grouping::Standard,
            is_3d,
            filled: false,
        },
        approximate,
        series: Vec::new(),
        labels: None,
        vary_colors: None,
        gap_width: None,
        overlap: None,
        first_slice: None,
        hole: None,
        bubble_scale: None,
        auto_markers: false,
        auto_lines: true,
        auto_smooth: false,
        axes: Vec::new(),
    };
    // `c:marker` of a line chart; `c:scatterStyle` or `c:radarStyle`.
    let mut markers = false;
    let mut style = None;
    each_child(r, tag, |r, t| match t.name {
        "barDir" => {
            if val(r, t).as_deref() == Some("bar") {
                p.plot.family = Family::Bar;
            }
        }
        "grouping" => {
            p.plot.grouping = match val(r, t).as_deref() {
                Some("stacked") => Grouping::Stacked,
                Some("percentStacked") => Grouping::Percent,
                // "clustered" and "standard".
                _ => Grouping::Standard,
            }
        }
        "scatterStyle" | "radarStyle" => style = val(r, t),
        "varyColors" => p.vary_colors = Some(flag(r, t)),
        "ser" => p.series.push(parse_series(r, t, palette)),
        "dLbls" => p.labels = parse_labels(r, t),
        "gapWidth" => p.gap_width = integer(r, t, 0, 500).map(|n| n as u32),
        "overlap" => p.overlap = integer(r, t, -100, 100).map(|n| n as i32),
        "firstSliceAng" => p.first_slice = integer(r, t, 0, 360).map(|n| n as u32),
        "holeSize" => p.hole = integer(r, t, 10, 90).map(|n| n as u32),
        "bubbleScale" => p.bubble_scale = integer(r, t, 0, 300).map(|n| n as u32),
        "marker" => markers = flag(r, t),
        "axId" => {
            // Three at most: two axes and the depth of a 3-D chart.
            let id = val(r, t).filter(|_| p.axes.len() < 3);
            p.axes.extend(id.map(|id| id.trim().to_string()));
        }
        // Drop lines, high-low lines, up-down bars, series lines, the
        // shape of 3-D bars, extensions.
        _ => r.skip(t),
    });
    let style = style.as_deref();
    match p.plot.family {
        Family::Line => {
            // Lines in 3-D are ribbons, never stacked and without markers.
            if is_3d || approximate {
                p.plot.grouping = Grouping::Standard;
            }
            p.auto_markers = markers && !is_3d;
        }
        Family::Scatter => {
            // Excel writes "lineMarker" for all but smooth lines and lets
            // the series say the rest.
            p.auto_markers = !matches!(style, Some("line" | "smooth" | "none"));
            p.auto_lines = !matches!(style, Some("marker" | "none"));
            p.auto_smooth = matches!(style, Some("smooth" | "smoothMarker"));
        }
        Family::Radar => {
            p.plot.filled = style == Some("filled");
            p.auto_markers = style == Some("marker");
        }
        Family::Column | Family::Bar | Family::Area => {}
        _ => p.plot.grouping = Grouping::Standard,
    }
    p
}

/// How a series is drawn, as its element and its plot say.
struct Looks {
    /// The colour of the series, spelled out or not.
    ink: Color,
    /// The line of a series drawn as a line (`shown` is then in
    /// `line_shown`), the outline of any other.
    line: Line,
    line_shown: bool,
    marker: Marker,
    smooth: bool,
}

impl RawPlot {
    /// The kind of a chart whose first plot is this one and has no series
    /// to tell: the plainest that is written this way.
    fn empty_kind(&self) -> ChartKind {
        let markers = match self.plot.family {
            Family::Radar => self.auto_markers,
            _ => self.auto_smooth,
        };
        self.plot.kind(markers, self.auto_smooth, self.auto_smooth)
    }

    /// Automatic width of the line of a series of this plot.
    fn line_width(&self) -> f32 {
        self.plot.kind(false, true, false).default_line_width()
    }

    /// `position` is the place of the series among those of the chart,
    /// which decides its automatic colour.
    fn looks(&self, s: &RawSeries, position: usize) -> Looks {
        let automatic = palette_color(position);
        let none = Marker::new(MarkerShape::None);
        if self.approximate || !self.plot.is_line_like() {
            // Shapes: the colour is that of their fill. Lines of a kind
            // the engine lacks have it on their stroke.
            let stroke = s.shape.line.filter(|_| self.approximate);
            let ink = s.shape.fill.color();
            let ink = ink
                .or(stroke.and_then(|l| l.paint.color()))
                .unwrap_or(automatic);
            let line = if self.approximate || self.plot.family == Family::Line {
                // Lines of a kind the engine lacks; ribbons in 3-D.
                Line::default()
            } else if self.plot.filled {
                line_of(s.shape.line, true, ink, self.line_width())
            } else if self.plot.is_round() {
                line_of(s.shape.line, true, Color::WHITE, SLICE_OUTLINE_WIDTH)
            } else {
                line_of(s.shape.line, false, ink, OUTLINE_WIDTH)
            };
            return Looks {
                ink,
                line,
                line_shown: true,
                marker: none,
                smooth: false,
            };
        }

        let stroke = s.shape.line.unwrap_or_default();
        let line_shown = match stroke.paint {
            Paint::Nothing => false,
            Paint::Color(_) => true,
            Paint::Automatic => self.auto_lines,
        };
        // Markers left to Excel are those of its older versions: a shape
        // per series, larger than the ones it draws now.
        let legacy = || LEGACY_MARKERS[s.index.unwrap_or(position as u32) as usize % 9];
        let (shape, size) = match &s.marker {
            Some(RawMarker {
                symbol: Some(shape),
                size,
                ..
            }) => (*shape, size.unwrap_or(Marker::DEFAULT_SIZE)),
            Some(RawMarker { size, .. }) if self.auto_markers => {
                (legacy(), size.unwrap_or(LEGACY_MARKER_SIZE))
            }
            None if self.auto_markers => (legacy(), LEGACY_MARKER_SIZE),
            _ => (MarkerShape::None, Marker::DEFAULT_SIZE),
        };
        let marker_shown = shape != MarkerShape::None;

        // The colours of what is drawn: the line, the inside of the
        // markers and their outline; what the file leaves to Excel has the
        // automatic colour. A marker without a fill or without an outline
        // is not in the model: it is all of the colour it has.
        let (fill, outline) = s.marker.map_or((Paint::Automatic, Paint::Automatic), |m| {
            (
                m.shape.fill,
                m.shape.line.map(|l| l.paint).unwrap_or_default(),
            )
        });
        let inside = match (fill, outline) {
            (Paint::Nothing, Paint::Color(_)) => outline,
            _ => fill,
        };
        let around = match (outline, fill) {
            (Paint::Nothing, Paint::Color(_)) => fill,
            _ => outline,
        };
        let drawn = |shown: bool, paint: Paint| shown.then(|| paint.color().unwrap_or(automatic));
        let line_color = drawn(line_shown, stroke.paint);
        // Markers that are strokes have no inside.
        let inside = drawn(marker_shown && !is_stroked(shape), inside);
        let around = drawn(marker_shown, around);
        // The colour of the series is that of its line; that of its markers
        // when they are of one colour and the line of another, or when
        // there is no line. Whatever has another colour says so itself.
        let ink = match (line_color, inside, around) {
            (Some(line), Some(inside), Some(around)) if inside == around && inside != line => {
                inside
            }
            (Some(line), ..) => line,
            (None, Some(inside), _) => inside,
            (None, None, Some(around)) => around,
            (None, None, None) => automatic,
        };
        let own = |part: Option<Color>| part.filter(|color| *color != ink);
        let line = if line_shown {
            Line {
                shown: None,
                color: own(line_color),
                width: stroke
                    .width
                    .filter(|w| (w - self.line_width()).abs() > WIDTH_TOLERANCE),
                dash: stroke.dash,
            }
        } else {
            Line::default()
        };
        Looks {
            ink,
            line,
            line_shown,
            marker: Marker {
                shape,
                size: if marker_shown {
                    size
                } else {
                    Marker::DEFAULT_SIZE
                },
                fill: own(inside),
                line: own(around),
            },
            smooth: self.plot.has_smoothing() && s.smooth.unwrap_or(self.auto_smooth),
        }
    }
}

/// The line of a trendline of a series of that colour. Excel's own
/// (round dots, a point and a half wide, in the colour of the series) is
/// the automatic one; a solid line that would otherwise read the same
/// says its width.
fn trend_line(stroke: Option<Stroke>, series: Color) -> Line {
    let line = line_of(stroke, true, series, TREND_WIDTH);
    let dotted = Line {
        dash: Dash::RoundDot,
        ..Line::default()
    };
    if line == dotted {
        Line::default()
    } else if stroke.is_some() && line == Line::default() {
        Line {
            width: Some(TREND_WIDTH),
            ..line
        }
    } else {
        line
    }
}

// ---- axes ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum AxisElement {
    /// `c:catAx`, `c:dateAx`.
    Category,
    /// `c:valAx`.
    Value,
    /// `c:serAx`: the depth of a 3-D chart.
    Series,
}

struct RawAxis {
    id: String,
    element: AxisElement,
    /// The options of the axis itself. Where the other axis crosses it is
    /// not among them: files say that on the other axis.
    axis: Axis,
    /// Where this axis crosses the other one.
    crosses: AxisCrossing,
    /// Runs along the bottom or the top of the plot.
    horizontal: Option<bool>,
}

fn parse_axis<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawAxis {
    let element = match tag.name {
        "valAx" => AxisElement::Value,
        "serAx" => AxisElement::Series,
        _ => AxisElement::Category,
    };
    let mut raw = RawAxis {
        id: String::new(),
        element,
        axis: Axis {
            // What Excel draws when a file does not say.
            major_ticks: TickMark::Outside,
            ..Axis::new()
        },
        crosses: AxisCrossing::Auto,
        horizontal: None,
    };
    let scale = element == AxisElement::Value;
    let a = &mut raw.axis;
    each_child(r, tag, |r, t| match t.name {
        "axId" => raw.id = val(r, t).map_or(String::new(), |id| id.trim().to_string()),
        "scaling" => each_child(r, t, |r, t| match t.name {
            "logBase" if scale => a.log_base = number(r, t).filter(|b| (2.0..=1000.0).contains(b)),
            "orientation" => a.reversed = val(r, t).as_deref() == Some("maxMin"),
            "max" if scale => a.max = number(r, t),
            "min" if scale => a.min = number(r, t),
            _ => r.skip(t),
        }),
        "delete" => a.visible = !flag(r, t),
        "axPos" => raw.horizontal = val(r, t).map(|side| matches!(&*side, "b" | "t")),
        "majorGridlines" => {
            a.major_gridlines = true;
            r.skip(t);
        }
        "minorGridlines" => {
            a.minor_gridlines = true;
            r.skip(t);
        }
        "title" => {
            a.title = match parse_title(r, t).text {
                ChartTitle::Text(text) => Some(text),
                _ => None,
            }
        }
        "numFmt" => {
            if t.attr_bool("sourceLinked") != Some(true) {
                a.number_format = t.attr("formatCode").map(Cow::into_owned);
            }
            r.skip(t);
        }
        "majorTickMark" => {
            a.major_ticks = val(r, t)
                .and_then(|v| named(&TICK_MARKS, &v))
                .unwrap_or(TickMark::Cross)
        }
        "minorTickMark" => {
            a.minor_ticks = val(r, t)
                .and_then(|v| named(&TICK_MARKS, &v))
                .unwrap_or(TickMark::Cross)
        }
        "tickLblPos" => {
            a.labels = val(r, t)
                .and_then(|v| named(&TICK_LABELS, &v))
                .unwrap_or_default()
        }
        "crosses" => {
            raw.crosses = match val(r, t).as_deref() {
                Some("min") => AxisCrossing::Min,
                Some("max") => AxisCrossing::Max,
                _ => AxisCrossing::Auto,
            }
        }
        "crossesAt" => raw.crosses = number(r, t).map_or(AxisCrossing::Auto, AxisCrossing::At),
        "majorUnit" if scale => a.major_unit = number(r, t).filter(|u| *u > 0.0),
        "minorUnit" if scale => a.minor_unit = number(r, t).filter(|u| *u > 0.0),
        "dispUnits" if scale => {
            let mut label = false;
            each_child(r, t, |r, t| match t.name {
                "builtInUnit" => {
                    a.display_unit = val(r, t)
                        .and_then(|v| named(&DISPLAY_UNITS, &v))
                        .unwrap_or_default()
                }
                "custUnit" => {
                    a.display_unit = number(r, t)
                        .filter(|u| *u > 0.0)
                        .map_or(DisplayUnit::None, DisplayUnit::Custom)
                }
                "dispUnitsLbl" => {
                    label = true;
                    r.skip(t);
                }
                _ => r.skip(t),
            });
            // Without a unit there is no label to show or hide.
            a.display_unit_label = label || a.display_unit == DisplayUnit::None;
        }
        // The formatting of line and labels, label skipping, extensions.
        _ => r.skip(t),
    });
    raw
}

/// The two axes a plot is drawn against.
#[derive(Clone, Copy, Default)]
struct AxisPair<'a> {
    category: Option<&'a RawAxis>,
    value: Option<&'a RawAxis>,
}

fn axes_of<'a>(axes: &'a [RawAxis], plot: &RawPlot) -> AxisPair<'a> {
    let own: Vec<&RawAxis> = plot
        .axes
        .iter()
        .filter_map(|id| axes.iter().find(|a| a.id == *id))
        .filter(|a| a.element != AxisElement::Series)
        .collect();
    if plot.plot.is_xy() {
        // Two scales: X is the horizontal one, or the first where the
        // file does not tell.
        return match own.as_slice() {
            [first, second, ..]
                if first.horizontal == Some(false) && second.horizontal == Some(true) =>
            {
                AxisPair {
                    category: Some(second),
                    value: Some(first),
                }
            }
            [first, rest @ ..] => AxisPair {
                category: Some(first),
                value: rest.first().copied(),
            },
            [] => AxisPair::default(),
        };
    }
    let first = |element: AxisElement| own.iter().find(|a| a.element == element).copied();
    AxisPair {
        category: first(AxisElement::Category),
        value: first(AxisElement::Value),
    }
}

/// An axis of the model: its own options, and where the other axis of its
/// pair crosses it, which the file says on that other axis.
fn axis_of(own: Option<&RawAxis>, other: Option<&RawAxis>, default: Axis) -> Axis {
    match own {
        Some(own) => Axis {
            crossing: other.map_or(AxisCrossing::Auto, |o| o.crosses),
            ..own.axis.clone()
        },
        None => default,
    }
}

// ---- chart -----------------------------------------------------------------------

/// `<c:view3D>`, as far as it says.
#[derive(Default)]
struct RawView {
    rot_x: Option<i32>,
    height: Option<u32>,
    rot_y: Option<u32>,
    depth: Option<u32>,
    right_angle: Option<bool>,
    perspective: Option<u32>,
}

fn parse_view<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawView {
    let mut view = RawView::default();
    each_child(r, tag, |r, t| match t.name {
        "rotX" => view.rot_x = integer(r, t, -90, 90).map(|n| n as i32),
        "hPercent" => view.height = integer(r, t, 5, 500).map(|n| n as u32),
        "rotY" => view.rot_y = integer(r, t, 0, 360).map(|n| n as u32),
        "depthPercent" => view.depth = integer(r, t, 20, 2000).map(|n| n as u32),
        "rAngAx" => view.right_angle = Some(flag(r, t)),
        "perspective" => view.perspective = integer(r, t, 0, 240).map(|n| n as u32),
        _ => r.skip(t),
    });
    view
}

fn parse_table<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> DataTable {
    let mut table = DataTable {
        horizontal_lines: false,
        vertical_lines: false,
        outline: false,
        legend_keys: false,
    };
    each_child(r, tag, |r, t| match t.name {
        "showHorzBorder" => table.horizontal_lines = flag(r, t),
        "showVertBorder" => table.vertical_lines = flag(r, t),
        "showOutline" => table.outline = flag(r, t),
        "showKeys" => table.legend_keys = flag(r, t),
        _ => r.skip(t),
    });
    table
}

/// A chart part as it is written.
#[derive(Default)]
struct RawChart {
    title: Option<Title>,
    title_deleted: bool,
    view: Option<RawView>,
    plots: Vec<RawPlot>,
    axes: Vec<RawAxis>,
    table: Option<DataTable>,
    plot_area: Shape,
    legend: Option<Legend>,
    blanks: BlanksAs,
    chart_area: Shape,
}

fn parse_raw(xml: &str, palette: &Palette) -> RawChart {
    let mut raw = RawChart::default();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "chartSpace" => {}
            Event::Open(t) if t.name == "chart" => each_child(&mut r, &t, |r, t| match t.name {
                "title" => raw.title = Some(parse_title(r, t)),
                "autoTitleDeleted" => raw.title_deleted = flag(r, t),
                "view3D" => raw.view = Some(parse_view(r, t)),
                "plotArea" => each_child(r, t, |r, t| match t.name {
                    "catAx" | "dateAx" | "valAx" | "serAx" => raw.axes.push(parse_axis(r, t)),
                    "dTable" => raw.table = Some(parse_table(r, t)),
                    "spPr" => raw.plot_area = parse_shape(r, t, palette),
                    // A chart type, or the layout and extensions.
                    element => match plot_of(element) {
                        Some(base) => raw.plots.push(parse_plot(r, t, base, palette)),
                        None => r.skip(t),
                    },
                }),
                "legend" => raw.legend = Some(parse_legend(r, t)),
                "dispBlanksAs" => {
                    raw.blanks = match val(r, t).as_deref() {
                        Some("zero") => BlanksAs::Zero,
                        Some("span") => BlanksAs::Span,
                        _ => BlanksAs::Gap,
                    }
                }
                // Floor and walls, pivot formats, extensions.
                _ => r.skip(t),
            }),
            // The chart area; its text is not in the model.
            Event::Open(t) if t.name == "spPr" => raw.chart_area = parse_shape(&mut r, &t, palette),
            Event::Open(t) => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    raw
}

/// A series on its way into the model.
struct Drawn {
    /// Index of its plot.
    plot: usize,
    raw: RawSeries,
    looks: Looks,
    /// The kind of a chart of such series.
    kind: ChartKind,
}

impl Drawn {
    /// The kind of the series when it is not that of the chart: series of
    /// another plot than the chart's have a type of their own, if the
    /// chart can show it next to its own.
    fn own_kind(&self, chart: ChartKind) -> Option<ChartKind> {
        (Plot::of(self.kind) != Plot::of(chart) && chart.combines_with(self.kind))
            .then_some(self.kind)
    }

    /// What the series is drawn as in a chart of that kind.
    fn drawn_as(&self, chart: ChartKind) -> Plot {
        Plot::of(self.own_kind(chart).unwrap_or(chart))
    }
}

impl RawChart {
    /// The chart in the engine's terms. `None` when there is no plot the
    /// engine can show.
    fn into_chart(mut self, separator: &str) -> Option<Chart> {
        if self.plots.is_empty() {
            return None;
        }
        let mut plotted: Vec<(usize, RawSeries)> = Vec::new();
        for (at, plot) in self.plots.iter_mut().enumerate() {
            plotted.extend(plot.series.drain(..).map(|s| (at, s)));
        }
        for (position, (_, s)) in plotted.iter_mut().enumerate() {
            s.order.get_or_insert(position as u32);
        }
        plotted.sort_by_key(|(_, s)| s.order);
        // What Excel counts, including the series there is no range to read.
        let series_in_file = plotted.len();
        let plots = &self.plots;
        let drawn: Vec<Drawn> = plotted
            .into_iter()
            // A series is its values; without a range to read them from
            // there is nothing to plot.
            .filter(|(_, s)| s.values.is_some())
            .enumerate()
            .map(|(position, (at, raw))| {
                let looks = plots[at].looks(&raw, position);
                let kind = plots[at].plot.kind(
                    looks.marker.shape != MarkerShape::None,
                    looks.line_shown,
                    looks.smooth,
                );
                Drawn {
                    plot: at,
                    raw,
                    looks,
                    kind,
                }
            })
            .collect();

        // The chart is of the type of its first series; the plot of that
        // series speaks for the chart where the model has one setting and
        // the file one per plot.
        let (main, kind) = match drawn.first() {
            Some(first) => (first.plot, first.kind),
            None => (0, plots[0].empty_kind()),
        };
        let mut chart = Chart::new(kind);

        // Series plotted against other axes than the first plot's are on
        // the secondary ones, where the chart can have such.
        let ids = |plot: &RawPlot| plot.axes.iter().cloned().collect::<BTreeSet<String>>();
        let primary_ids = ids(&plots[0]);
        let can_split = !kind.is_3d() && !kind.is_round() && kind.family() != Family::Radar;
        let secondary: Vec<bool> = plots
            .iter()
            .map(|p| {
                can_split && !primary_ids.is_empty() && !p.axes.is_empty() && ids(p) != primary_ids
            })
            .collect();

        if !kind.is_round() {
            let defaults = Axes::for_kind(kind);
            let pair = axes_of(&self.axes, &plots[0]);
            chart.axes.category = axis_of(pair.category, pair.value, defaults.category);
            chart.axes.value = axis_of(pair.value, pair.category, defaults.value);
            if let Some(at) = secondary.iter().position(|s| *s) {
                let second = axes_of(&self.axes, &plots[at]);
                // A category axis shared with the primary series is no
                // second category axis.
                let category = second
                    .category
                    .filter(|a| pair.category.is_none_or(|p| p.id != a.id));
                // The secondary scale crosses at the far end unless told
                // otherwise: that is its automatic place. So it is for the
                // secondary category axis when it is shown; Excel's own,
                // which is not, says nothing of the kind.
                let far_end = |axis: &RawAxis, always: bool| match axis.crosses {
                    AxisCrossing::Max => AxisCrossing::Auto,
                    AxisCrossing::Auto if always || axis.axis.visible => AxisCrossing::Min,
                    at => at,
                };
                chart.axes.secondary_category = match category {
                    Some(own) => Axis {
                        crossing: second
                            .value
                            .map_or(AxisCrossing::Auto, |scale| far_end(scale, true)),
                        ..own.axis.clone()
                    },
                    None => defaults.secondary_category,
                };
                chart.axes.secondary_value = match second.value {
                    Some(own) => Axis {
                        crossing: category.map_or(AxisCrossing::Auto, |axis| far_end(axis, false)),
                        ..own.axis.clone()
                    },
                    None => defaults.secondary_value,
                };
            }
        }

        // Labels: a file has them per plot and per series, the model for
        // the chart, with exceptions. Labels all the series have are the
        // chart's; failing that, those of the main plot are, and a series
        // with other labels has labels of its own. The separator the
        // engine puts between the parts of a label by itself is no
        // separator of their own.
        let labels_of = |plot: Plot, labels: &DataLabels| {
            let mut labels = plot.labels(labels);
            labels.separator = labels.separator.filter(|s| s != separator);
            labels
        };
        let labelled: Vec<Option<DataLabels>> = drawn
            .iter()
            .map(|d| {
                let plot = &plots[d.plot];
                match &d.raw.labels {
                    Some(own) => own.as_ref(),
                    None => plot.labels.as_ref(),
                }
                .map(|l| labels_of(d.drawn_as(kind), l))
            })
            .collect();
        let chart_wide = match labelled.split_first() {
            Some((first, rest)) if rest.iter().all(|l| l == first) => first.clone(),
            _ => plots[main]
                .labels
                .as_ref()
                .map(|l| labels_of(plots[main].plot, l)),
        };
        chart.data_labels = chart_wide.is_some();
        chart.label_options = chart_wide.clone().unwrap_or_default();

        // Where every point has a colour of its own, the automatic colours
        // of the points are spelled out in files and are not formatting.
        chart.vary_colors = plots[main].vary_colors.filter(|v| *v != kind.is_round());
        let varied = chart.varies_colors() && (kind.is_round() || drawn.len() == 1);

        for (position, (d, own_labels)) in drawn.into_iter().zip(labelled).enumerate() {
            let plot = &plots[d.plot];
            let own_kind = d.own_kind(kind);
            let effective = own_kind.unwrap_or(kind);
            let drawn_as = Plot::of(effective);
            let Some(values) = d.raw.values else {
                continue;
            };
            let mut series = Series {
                name: d.raw.name,
                categories: d.raw.categories.map(|c| innermost_level(&c, &values)),
                // A colour that is the automatic one is no colour of its own.
                color: Some(d.looks.ink).filter(|ink| *ink != palette_color(position)),
                kind: own_kind,
                secondary: secondary[d.plot],
                ..Series::new(values)
            };
            if drawn_as.family == Family::Bubble {
                series.bubble_sizes = d.raw.sizes;
            }
            // The looks of a series drawn as something else altogether
            // mean nothing.
            if drawn_as == plot.plot {
                series.line = d.looks.line.clone();
                if drawn_as.is_line_like() {
                    let looks = &d.looks;
                    if looks.line_shown != effective.has_lines() {
                        series.line.shown = Some(looks.line_shown);
                    }
                    let automatic = if effective.has_markers() {
                        Marker::new(MarkerShape::Circle)
                    } else {
                        Marker::new(MarkerShape::None)
                    };
                    series.marker = (looks.marker != automatic).then(|| looks.marker.clone());
                    if looks.smooth != effective.is_smooth() {
                        series.smooth = Some(looks.smooth);
                    }
                }
            }
            for point in &d.raw.points {
                let color = if drawn_as.is_line_like() {
                    let marker = point.marker.map(|m| m.shape);
                    marker
                        .and_then(|m| m.fill.color().or(m.line.and_then(|l| l.paint.color())))
                        .or(point.shape.line.and_then(|l| l.paint.color()))
                } else {
                    point.shape.fill.color()
                };
                let color = color.filter(|c| !varied || *c != palette_color(point.index as usize));
                let explosion = point.explosion.filter(|_| drawn_as.is_round());
                if color.is_some() || explosion.is_some() {
                    series.points.push(PointFormat {
                        index: point.index,
                        color,
                        explosion,
                    });
                }
            }
            // In order, and one for each point: the first the file has.
            series.points.sort_by_key(|p| p.index);
            series.points.dedup_by_key(|p| p.index);
            if drawn_as.is_round() {
                series.explosion = d.raw.explosion;
            }
            if own_labels != chart_wide.as_ref().map(|l| drawn_as.labels(l)) {
                series.labels = Some(own_labels.unwrap_or_else(DataLabels::none));
            }
            if drawn_as.takes_trendlines() {
                series.trendlines = d
                    .raw
                    .trendlines
                    .into_iter()
                    .map(|t| Trendline {
                        line: trend_line(t.stroke, d.looks.ink),
                        ..t.trend
                    })
                    .collect();
            }
            if drawn_as.takes_error_bars() {
                // One set along each axis the series has a scale on.
                for bars in d.raw.error_bars {
                    let fits = drawn_as.is_xy() || !bars.horizontal;
                    if fits
                        && !series
                            .error_bars
                            .iter()
                            .any(|b| b.horizontal == bars.horizontal)
                    {
                        series.error_bars.push(bars);
                    }
                }
            }
            chart.series.push(series);
        }

        // Bars without a gap or an overlap of their own have Excel's: 150
        // and none.
        let bars = |p: &&RawPlot| matches!(p.plot.family, Family::Column | Family::Bar);
        if let Some(p) = plots.iter().find(bars) {
            let gap = p.gap_width.unwrap_or(150);
            chart.gap_width = (gap != kind.default_gap_width()).then_some(gap);
        }
        if let Some(p) = plots.iter().find(|p| bars(p) && !p.plot.is_3d) {
            let overlap = p.overlap.unwrap_or(0);
            let automatic = p.plot.kind(false, false, false).default_overlap();
            chart.overlap = (overlap != automatic).then_some(overlap);
        }
        if let Some(p) = plots.iter().find(|p| p.plot.is_round() && !p.plot.is_3d) {
            chart.first_slice_angle = p.first_slice.unwrap_or(0);
        }
        if let Some(p) = plots.iter().find(|p| p.plot.family == Family::Doughnut) {
            chart.hole_size = p.hole.unwrap_or(DEFAULT_HOLE_SIZE);
        }
        if let Some(p) = plots.iter().find(|p| p.plot.family == Family::Bubble) {
            chart.bubble_scale = p.bubble_scale.unwrap_or(100);
        }

        if kind.is_3d() {
            let automatic = View3D::for_kind(kind);
            let said = self.view.unwrap_or_default();
            let mut view = View3D {
                rot_x: said.rot_x.unwrap_or(automatic.rot_x),
                rot_y: said.rot_y.unwrap_or(automatic.rot_y),
                perspective: said.perspective.unwrap_or(automatic.perspective),
                right_angle_axes: said.right_angle.unwrap_or(automatic.right_angle_axes),
                depth_percent: said.depth.unwrap_or(automatic.depth_percent),
                height_percent: said.height,
            };
            if kind == ChartKind::Pie3D {
                // Turning a pie around its axis is moving its first slice:
                // the file has one angle for both.
                chart.first_slice_angle = view.rot_y % 360;
                view.rot_y = automatic.rot_y;
            }
            chart.view3d = (view != automatic).then_some(view);
        }

        // Without a title element, Excel still titles a chart of a single
        // series with the name of that series, unless the title was
        // deleted. A chart of several series has no title then.
        match self.title {
            Some(title) => {
                chart.title = title.text;
                chart.title_overlay = title.overlay;
            }
            None if self.title_deleted || series_in_file != 1 => chart.title = ChartTitle::None,
            None => chart.title = ChartTitle::Auto,
        }
        if let Some(legend) = self.legend {
            chart.legend = legend.pos;
            chart.legend_overlay = legend.overlay;
            chart.legend_hidden = legend.hidden;
        }
        // The chart area is white in a thin grey frame unless told
        // otherwise, the plot area nothing at all.
        chart.chart_area = ShapeFormat {
            fill: match self.chart_area.fill {
                Paint::Automatic | Paint::Color(Color::WHITE) => Fill::Auto,
                Paint::Nothing => Fill::None,
                Paint::Color(color) => Fill::Solid(color),
            },
            border: line_of(self.chart_area.line, true, FRAME_GREY, OUTLINE_WIDTH),
        };
        chart.plot_area = ShapeFormat {
            fill: match self.plot_area.fill {
                Paint::Automatic | Paint::Nothing => Fill::Auto,
                Paint::Color(color) => Fill::Solid(color),
            },
            border: line_of(self.plot_area.line, false, FRAME_GREY, OUTLINE_WIDTH),
        };
        chart.data_table = self.table.filter(|_| Plot::of(kind).takes_data_table());
        chart.blanks = self.blanks;
        Some(chart)
    }
}

/// Read a chart part. `None` when it holds no chart the engine can show.
/// The chart remembers the text it was read from (see [`ForeignChart`]);
/// where it sits is for the caller to say. The locale is the workbook's:
/// it decides what is automatic between the parts of a data label.
pub(crate) fn parse_chart(xml: &str, palette: &Palette, locale: &Locale) -> Option<Chart> {
    let mut chart = parse_raw(xml, palette).into_chart(&list_separator(locale))?;
    chart.foreign = Some(Arc::new(ForeignChart {
        // The text will be read without the theme of this workbook.
        xml: Arc::from(foreign::without_theme(xml, &palette.theme)),
        as_read: chart.content(),
    }));
    Some(chart)
}
