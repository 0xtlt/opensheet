//! Reading charts: where they sit (the drawing part of a sheet) and what
//! they plot (chart parts), from files written by any application.
//!
//! Only what the engine models is extracted; everything else in these
//! parts is stepped over. Chart types the engine does not have are read as
//! the closest one it has (see [`family`]).

use super::{EMU_PER_PX, Track, innermost_level};
use crate::xlsx::color::{Palette, adjust_luminance, blend, parse_rgb};
use crate::xlsx::formula::from_file_text;
use crate::xlsx::xml::{Event, Reader, Tag, decode_cell_text};
use opensheet_engine::Color;
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::chart::{
    ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use std::borrow::Cow;

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
struct Marker {
    col: u32,
    row: u32,
    x: f64,
    y: f64,
}

/// A frame of a drawing part that shows charts.
#[derive(Default)]
pub(crate) struct Frame {
    from: Option<Marker>,
    to: Option<Marker>,
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

fn parse_marker<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Marker {
    let mut m = Marker::default();
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
        "from" => frame.from = Some(parse_marker(r, t)),
        "to" => frame.to = Some(parse_marker(r, t)),
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
            Marker { col, row, x, y }
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

// ---- chart -----------------------------------------------------------------------

/// What a chart part says, in the engine's terms.
pub(crate) struct ChartContent {
    pub kind: ChartKind,
    pub title: ChartTitle,
    pub legend: LegendPos,
    pub series: Vec<Series>,
    pub data_labels: bool,
}

/// The chart types of the format, reduced to those of the engine.
#[derive(Clone, Copy, PartialEq)]
enum Family {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
}

/// The family of a plot element. Types the engine lacks become their
/// nearest relative: 3-D variants lose their depth; stock, radar and
/// surface charts become lines; a pie with a second plot is a pie; bubbles
/// lose their sizes and become a scatter chart.
fn family(element: &str) -> Option<Family> {
    Some(match element {
        "barChart" | "bar3DChart" => Family::Bar,
        "lineChart" | "line3DChart" | "stockChart" | "radarChart" | "surfaceChart"
        | "surface3DChart" => Family::Line,
        "areaChart" | "area3DChart" => Family::Area,
        "pieChart" | "pie3DChart" | "ofPieChart" => Family::Pie,
        "doughnutChart" => Family::Doughnut,
        "scatterChart" | "bubbleChart" => Family::Scatter,
        _ => return None,
    })
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

/// The range behind `<c:cat>`, `<c:val>`, `<c:xVal>` or `<c:yVal>`. Data
/// typed into the chart itself (`numLit`, `strLit`) has none.
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

/// `<c:title>`: text typed into the chart, the text of the cell the title
/// is linked to, or (no text at all) the title Excel makes up.
fn parse_title<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> ChartTitle {
    let mut title = ChartTitle::Auto;
    each_child(r, tag, |r, t| match t.name {
        "tx" => each_child(r, t, |r, t| match t.name {
            "rich" => title = ChartTitle::Text(parse_rich(r, t)),
            "strRef" => {
                if let (_, Some(text)) = parse_str_ref(r, t) {
                    title = ChartTitle::Text(text);
                }
            }
            _ => r.skip(t),
        }),
        _ => r.skip(t),
    });
    title
}

fn parse_legend<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> LegendPos {
    // A legend that does not say where it is sits on the right.
    let mut pos = LegendPos::Right;
    each_child(r, tag, |r, t| match t.name {
        "legendPos" => {
            pos = match val(r, t).as_deref() {
                Some("b") => LegendPos::Bottom,
                Some("t") => LegendPos::Top,
                Some("l") => LegendPos::Left,
                // "r", and "tr" (the top right corner).
                _ => LegendPos::Right,
            }
        }
        _ => r.skip(t),
    });
    pos
}

/// `<c:dLbls>`: whether the points are labelled, with whatever content.
fn parse_labels<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> bool {
    let mut deleted = false;
    let mut shown = false;
    each_child(r, tag, |r, t| match t.name {
        "delete" => deleted = flag(r, t),
        "showVal" | "showCatName" | "showSerName" | "showPercent" => shown |= flag(r, t),
        // Including `dLbl`, the label of a single point.
        _ => r.skip(t),
    });
    shown && !deleted
}

/// Position of a theme colour in a [`Palette`].
fn scheme_slot(name: &str) -> Option<usize> {
    Some(match name {
        "dk1" | "tx1" => 0,
        "lt1" | "bg1" => 1,
        "dk2" | "tx2" => 2,
        "lt2" | "bg2" => 3,
        "accent1" => 4,
        "accent2" => 5,
        "accent3" => 6,
        "accent4" => 7,
        "accent5" => 8,
        "accent6" => 9,
        "hlink" => 10,
        "folHlink" => 11,
        _ => return None,
    })
}

/// `<a:solidFill>`: an RGB, theme or system colour, lightened or darkened
/// by the adjustments nested in it. Transparency is ignored.
fn parse_solid_fill<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<Color> {
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

/// `<c:spPr>`: (fill, outline) when they are plain colours.
fn parse_shape<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    palette: &Palette,
) -> (Option<Color>, Option<Color>) {
    let mut fill = None;
    let mut outline = None;
    each_child(r, tag, |r, t| match t.name {
        "solidFill" => fill = parse_solid_fill(r, t, palette),
        "ln" => each_child(r, t, |r, t| match t.name {
            "solidFill" => outline = parse_solid_fill(r, t, palette),
            _ => r.skip(t),
        }),
        _ => r.skip(t),
    });
    (fill, outline)
}

/// A series as its plot describes it.
struct PlottedSeries {
    /// Rank among all the series of the chart.
    order: Option<u32>,
    name: SeriesName,
    values: Option<RefExpr>,
    categories: Option<RefExpr>,
    color: Option<Color>,
    labels: bool,
}

fn parse_series<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    family: Family,
    palette: &Palette,
) -> PlottedSeries {
    let mut s = PlottedSeries {
        order: None,
        name: SeriesName::None,
        values: None,
        categories: None,
        color: None,
        labels: false,
    };
    let (mut fill, mut outline, mut marker) = (None, None, None);
    each_child(r, tag, |r, t| match t.name {
        "order" => s.order = val(r, t).and_then(|v| v.trim().parse().ok()),
        "tx" => s.name = parse_series_name(r, t),
        "spPr" => (fill, outline) = parse_shape(r, t, palette),
        "marker" => each_child(r, t, |r, t| match t.name {
            "spPr" => marker = parse_shape(r, t, palette).0,
            _ => r.skip(t),
        }),
        "dLbls" => s.labels = parse_labels(r, t),
        "cat" | "xVal" => s.categories = parse_data_ref(r, t),
        "val" | "yVal" => s.values = parse_data_ref(r, t),
        // Colours of single points, trend lines, error bars, bubble sizes,
        // extensions.
        _ => r.skip(t),
    });
    // The colour of a series is that of its ink: the stroke of a line (or,
    // when only the points are drawn, their fill), the fill of a shape.
    s.color = match family {
        Family::Line | Family::Scatter => outline.or(marker).or(fill),
        _ => fill.or(outline),
    };
    s
}

struct Plot {
    kind: ChartKind,
    series: Vec<PlottedSeries>,
    labels: bool,
}

fn parse_plot<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, family: Family, palette: &Palette) -> Plot {
    let mut horizontal = false;
    let mut stacked = false;
    let mut series = Vec::new();
    let mut labels = false;
    each_child(r, tag, |r, t| match t.name {
        "barDir" => horizontal = val(r, t).as_deref() == Some("bar"),
        // Stacked to 100 % is stacked: the engine has no percent scale.
        "grouping" => stacked = matches!(val(r, t).as_deref(), Some("stacked" | "percentStacked")),
        "ser" => series.push(parse_series(r, t, family, palette)),
        "dLbls" => labels |= parse_labels(r, t),
        _ => r.skip(t),
    });
    let kind = match family {
        Family::Bar => match (horizontal, stacked) {
            (false, false) => ChartKind::Column,
            (false, true) => ChartKind::StackedColumn,
            (true, false) => ChartKind::Bar,
            (true, true) => ChartKind::StackedBar,
        },
        Family::Line => ChartKind::Line,
        Family::Area => ChartKind::Area,
        Family::Pie => ChartKind::Pie,
        Family::Doughnut => ChartKind::Doughnut,
        Family::Scatter => ChartKind::Scatter,
    };
    Plot {
        kind,
        series,
        labels,
    }
}

/// Read a chart part. `None` when it holds no chart the engine can show.
pub(crate) fn parse_chart(xml: &str, palette: &Palette) -> Option<ChartContent> {
    let mut title: Option<ChartTitle> = None;
    let mut title_deleted = false;
    let mut legend = LegendPos::None;
    let mut plots: Vec<Plot> = Vec::new();

    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "chartSpace" => {}
            Event::Open(t) if t.name == "chart" => each_child(&mut r, &t, |r, t| match t.name {
                "title" => title = Some(parse_title(r, t)),
                "autoTitleDeleted" => title_deleted = flag(r, t),
                "plotArea" => each_child(r, t, |r, t| match family(t.name) {
                    Some(family) => plots.push(parse_plot(r, t, family, palette)),
                    // Layout, axes, data table, formatting.
                    None => r.skip(t),
                }),
                "legend" => legend = parse_legend(r, t),
                _ => r.skip(t),
            }),
            Event::Open(t) => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }

    // A chart mixing types (columns with a line over them) is drawn in the
    // type of its first plot, with the series of all of them.
    let kind = plots.first()?.kind;
    let data_labels = plots
        .iter()
        .any(|p| p.labels || p.series.iter().any(|s| s.labels));
    let mut plotted: Vec<PlottedSeries> = plots.into_iter().flat_map(|p| p.series).collect();
    for (i, s) in plotted.iter_mut().enumerate() {
        s.order.get_or_insert(i as u32);
    }
    plotted.sort_by_key(|s| s.order);
    // What Excel counts, including the series there is no range to read.
    let series_in_file = plotted.len();
    let series = plotted
        .into_iter()
        .filter_map(|s| {
            // A series is its values; without a range to read them from
            // there is nothing to plot.
            let values = s.values?;
            let categories = s.categories.map(|c| innermost_level(&c, &values));
            Some(Series {
                name: s.name,
                values,
                categories,
                color: s.color,
            })
        })
        .collect();
    Some(ChartContent {
        kind,
        // Without a title element, Excel still titles a chart of a single
        // series with the name of that series, unless the title was
        // deleted. A chart of several series has no title then.
        title: match title {
            Some(title) => title,
            None if title_deleted || series_in_file != 1 => ChartTitle::None,
            None => ChartTitle::Auto,
        },
        legend,
        series,
        data_labels,
    })
}
