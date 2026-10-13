//! Writing charts: one drawing part per sheet that has charts, one chart
//! part per chart.
//!
//! A chart part says what the model says, with what the model leaves
//! automatic spelled the way Excel spells it in its own charts (the
//! reader takes those spellings for automatic again). A chart read from a
//! file and not changed since is written as the text it was read from
//! instead, when that text fits the new package (see [`foreign`]).
//!
//! Excel validates these parts against the schema and offers to "repair"
//! (that is, drop) a chart at the first surprise, so the order of the
//! elements below is the order of the schema, whatever would read better.
//! It also refuses things the schema allows (a label position a chart
//! type does not have, a trendline on a stacked series, ...): those are
//! not written.

use super::{
    DASHES, DISPLAY_UNITS, EMU_PER_PT, EMU_PER_PX, FRAME_GREY, LABEL_POSITIONS, MARKERS,
    OUTLINE_WIDTH, Plot, SLICE_OUTLINE_WIDTH, TICK_LABELS, TICK_MARKS, TREND_WIDTH, Track, foreign,
    innermost_level, is_stroked, list_separator, name_cell, name_of, series_line,
};
use crate::xlsx::points_to_px;
use crate::xlsx::write::{
    MAX_COL_WIDTH_PX, MAX_ROW_HEIGHT_PT, NS_DOC_REL, NS_PKG_REL, push_number,
};
use crate::xlsx::color::push_drawing_color;
use crate::xlsx::xml::{XML_DECL, push_attr, push_cell_text, push_text};
use opensheet_engine::ast::{CellRef, RefExpr};
use opensheet_engine::chart::{
    Axis, AxisCrossing, BlanksAs, Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, Dash,
    DataLabels, DisplayUnit, ErrorAmount, ErrorDirection, Family, Fill, Grouping, LabelOffset,
    LegendPos, Line, ManualLayout, Marker, MarkerShape, Series, SeriesName, ShapeFormat,
    TextFormat, TrendKind, category_levels,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::workbook::Content;
use opensheet_engine::{Color, Value, Workbook};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::Arc;

const NS_CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const NS_DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_SHEET_DRAWING: &str =
    "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";

pub(crate) const CT_DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
pub(crate) const CT_CHART: &str =
    "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

/// Minor gridlines: lighter than the major ones.
pub(super) const MINOR_GREY: Color = Color(0xF2F2F2);
/// Titles, labels and legends: dark grey rather than black.
const TEXT_GREY: &str = "595959";
/// A line that is not drawn.
const NO_LINE: &str = "<a:ln><a:noFill/></a:ln>";
/// The points of a series that get a colour each where colours vary by
/// point; a series with more is left to Excel beyond that.
const VARIED_POINTS: u64 = 4096;

/// The parts carrying the charts of one sheet.
pub(crate) struct Drawing {
    /// Index of the sheet in the workbook.
    pub sheet: usize,
    /// `n` of `xl/drawings/drawing{n}.xml`.
    pub number: usize,
    pub xml: String,
    /// Relationships of the drawing part: `rId{k}` is its k-th chart.
    pub rels: String,
    /// (`n` of `xl/charts/chart{n}.xml`, content)
    pub charts: Vec<(usize, String)>,
    /// Where the anchor of each chart of the sheet is in `xml`, in the
    /// order of `Sheet::charts`: the pictures and shapes are woven in
    /// between them, in stacking order (`drawing::add_parts`).
    pub chart_anchors: Vec<std::ops::Range<usize>>,
}

/// The drawing of every sheet that has charts.
pub(crate) fn drawings(wb: &Workbook) -> Vec<Drawing> {
    let mut out: Vec<Drawing> = Vec::new();
    let mut chart_count = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if sheet.charts.is_empty() {
            continue;
        }
        // The geometry Excel will see: the sizes as they are written.
        let cols = Track {
            default: sheet.default_col_width,
            sizes: &sheet.col_widths,
            hidden: &sheet.hidden_cols,
            max: MAX_COL_WIDTH_PX,
            count: MAX_COLS,
        };
        let rows = Track {
            default: sheet.default_row_height,
            sizes: &sheet.row_heights,
            hidden: &sheet.hidden_rows,
            max: points_to_px(MAX_ROW_HEIGHT_PT),
            count: MAX_ROWS,
        };
        let writer = ChartWriter { wb, sheet: index };
        let mut xml = String::from(XML_DECL);
        w!(
            xml,
            "<xdr:wsDr xmlns:xdr=\"{NS_SHEET_DRAWING}\" xmlns:a=\"{NS_DRAWING}\">"
        );
        let mut rels = String::from(XML_DECL);
        w!(rels, "<Relationships xmlns=\"{NS_PKG_REL}\">");
        let mut charts = Vec::with_capacity(sheet.charts.len());
        let mut chart_anchors = Vec::with_capacity(sheet.charts.len());
        for (i, chart) in sheet.charts.iter().enumerate() {
            chart_count += 1;
            let start = xml.len();
            push_anchor(&mut xml, chart, i + 1, &cols, &rows);
            chart_anchors.push(start..xml.len());
            w!(
                rels,
                "<Relationship Id=\"rId{}\" Type=\"{NS_DOC_REL}/chart\" Target=\"../charts/chart{chart_count}.xml\"/>",
                i + 1
            );
            charts.push((chart_count, writer.chart_xml(chart, chart_count)));
        }
        xml.push_str("</xdr:wsDr>");
        rels.push_str("</Relationships>");
        out.push(Drawing {
            sheet: index,
            number: out.len() + 1,
            xml,
            rels,
            charts,
            chart_anchors,
        });
    }
    out
}

// ---- drawing -------------------------------------------------------------------

fn push_corner(x: &mut String, name: &str, (col, x_px): (u32, f64), (row, y_px): (u32, f64)) {
    let emu = |px: f64| (px * EMU_PER_PX).round() as i64;
    w!(
        x,
        "<xdr:{name}><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{name}>",
        emu(x_px),
        emu(y_px)
    );
}

/// The frame of the `number`-th chart of a sheet (counting from 1). Like
/// Excel's own, it is pinned to the cells under its two corners; `oneCell`
/// says it moves with the first one but keeps its size when cells are
/// resized, which is what an anchor cell plus a size in pixels means.
fn push_anchor(x: &mut String, chart: &Chart, number: usize, cols: &Track, rows: &Track) {
    let length = |px: f32, fallback: f32| {
        if px.is_finite() && px > 0.0 {
            px as f64
        } else {
            fallback as f64
        }
    };
    // An offset larger than its cell (the column was narrowed since) is
    // moved to the cell it really falls in.
    let left = cols.locate(chart.anchor.1, chart.offset.0 as f64);
    let top = rows.locate(chart.anchor.0, chart.offset.1 as f64);
    let right = cols.locate(left.0, left.1 + length(chart.size.0, DEFAULT_CHART_SIZE.0));
    let bottom = rows.locate(top.0, top.1 + length(chart.size.1, DEFAULT_CHART_SIZE.1));
    x.push_str("<xdr:twoCellAnchor editAs=\"oneCell\">");
    push_corner(x, "from", left, top);
    push_corner(x, "to", right, bottom);
    w!(
        x,
        "<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{}\" name=\"",
        number + 1
    );
    if chart.name.is_empty() {
        w!(x, "Chart {number}");
    } else {
        push_attr(x, &chart.name);
    }
    x.push('"');
    if chart.hidden {
        x.push_str(" hidden=\"1\"");
    }
    x.push_str("/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>");
    x.push_str("<xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm>");
    w!(
        x,
        "<a:graphic><a:graphicData uri=\"{NS_CHART}\"><c:chart xmlns:c=\"{NS_CHART}\" xmlns:r=\"{NS_DOC_REL}\" r:id=\"rId{number}\"/></a:graphicData></a:graphic>"
    );
    x.push_str("</xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>");
}

// ---- chart -----------------------------------------------------------------------

fn solid_fill(color: Color) -> String {
    let mut x = String::from("<a:solidFill>");
    push_drawing_color(&mut x, color, None);
    x.push_str("</a:solidFill>");
    x
}

/// The fill of the shapes of a series. Bubbles overlap: like Excel's own,
/// they let a quarter of what is behind them through.
fn shape_fill(color: Color, plot: Plot) -> String {
    if plot.family != Family::Bubble {
        return solid_fill(color);
    }
    let mut x = String::from("<a:solidFill>");
    push_drawing_color(&mut x, color, Some(75_000));
    x.push_str("</a:solidFill>");
    x
}

/// A number as an attribute value.
fn num(n: f64) -> String {
    let mut text = String::new();
    push_number(&mut text, n);
    text
}

/// `<a:ln>`: a line of that width in points, in that colour. `round` is
/// the way Excel draws the lines of series, with round ends and joins.
fn push_stroke(x: &mut String, width: f32, color: Color, dash: Dash, round: bool) {
    // The format stops at 1584 points.
    let emu = (width.clamp(0.0, 1584.0) as f64 * EMU_PER_PT).round() as i64;
    w!(x, "<a:ln w=\"{emu}\"");
    x.push_str(if round { " cap=\"rnd\">" } else { ">" });
    x.push_str(&solid_fill(color));
    if let Some(name) = name_of(&DASHES, dash).filter(|_| dash != Dash::Solid) {
        w!(x, "<a:prstDash val=\"{name}\"/>");
    }
    x.push_str(if round {
        "<a:round/></a:ln>"
    } else {
        "</a:ln>"
    });
}

/// A hairline of the given colour.
fn hairline(color: Color) -> String {
    let mut x = String::from("<c:spPr>");
    push_stroke(&mut x, OUTLINE_WIDTH, color, Dash::Solid, false);
    x.push_str("</c:spPr>");
    x
}

/// The line of an axis or of its gridlines, in `<c:spPr>`: thin and of
/// the colour `automatic` unless the model says otherwise.
fn rule_xml(line: &Line, automatic: Color) -> String {
    if line.shown == Some(false) {
        return format!("<c:spPr>{NO_LINE}</c:spPr>");
    }
    let mut x = String::from("<c:spPr>");
    push_stroke(
        &mut x,
        line.width.unwrap_or(OUTLINE_WIDTH),
        line.color.unwrap_or(automatic),
        line.dash,
        false,
    );
    x.push_str("</c:spPr>");
    x
}

/// Where an element of a chart is.
#[derive(Clone, Copy, PartialEq)]
enum Placed {
    /// A title: a corner.
    Corner,
    /// The legend: a corner and, when it was resized, a size.
    Frame,
    /// The plot area: the rectangle between the axes, or (`false`) the box
    /// around it and what its axes write.
    Plot { inner: bool },
}

/// `<c:layout>` of an element put somewhere by hand, as Excel writes it:
/// its edges measured from those of the chart.
fn layout_xml(layout: &ManualLayout, placed: Placed) -> String {
    let mut x = String::from("<c:layout><c:manualLayout>");
    if let Placed::Plot { inner } = placed {
        w!(
            x,
            "<c:layoutTarget val=\"{}\"/>",
            if inner { "inner" } else { "outer" }
        );
    }
    w!(
        x,
        "<c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"{}\"/><c:y val=\"{}\"/>",
        num(layout.x),
        num(layout.y)
    );
    if placed != Placed::Corner {
        for (element, size) in [("w", layout.w), ("h", layout.h)] {
            if let Some(size) = size.filter(|v| v.is_finite() && *v > 0.0) {
                w!(x, "<c:{element} val=\"{}\"/>", num(size));
            }
        }
    }
    x.push_str("</c:manualLayout></c:layout>");
    x
}

/// Whether a manual layout can be written: files hold plain numbers.
fn placeable(layout: &&ManualLayout) -> bool {
    layout.x.is_finite() && layout.y.is_finite()
}

/// Run properties (`a:defRPr`, `a:rPr`): what the format says, and
/// `rest` (attributes the element always has, such as `lang`).
/// `size` and `bold` are what stands for an automatic size and weight
/// where they are spelled out all the same.
fn run_properties(element: &str, text: &TextFormat, rest: &str, size: Option<u32>, bold: Option<bool>) -> String {
    let mut x = format!("<a:{element}{rest}");
    let size = text
        .size
        .filter(|s| s.is_finite())
        .map(|s| (s.clamp(1.0, 4000.0) * 100.0).round() as u32)
        .or(size);
    if let Some(size) = size {
        w!(x, " sz=\"{size}\"");
    }
    if let Some(bold) = text.bold.or(bold) {
        w!(x, " b=\"{}\"", bold as u8);
    }
    if let Some(italic) = text.italic {
        w!(x, " i=\"{}\"", italic as u8);
    }
    if let Some(underline) = text.underline {
        w!(x, " u=\"{}\"", if underline { "sng" } else { "none" });
    }
    let font = text.font.as_deref().filter(|f| !f.is_empty());
    if text.color.is_none() && font.is_none() {
        x.push_str("/>");
        return x;
    }
    x.push('>');
    if let Some(color) = text.color {
        x.push_str(&solid_fill(color));
    }
    if let Some(font) = font {
        x.push_str("<a:latin typeface=\"");
        push_attr(&mut x, font);
        x.push_str("\"/>");
    }
    w!(x, "</a:{element}>");
    x
}

/// `<c:txPr>` of the labels of an axis, of the legend or of data labels;
/// nothing where the text is left to Excel. A text with a font of its own
/// says its size as well, be it the 9 points such texts have anyway.
fn text_xml(text: &TextFormat) -> String {
    if text.is_auto() {
        return String::new();
    }
    format!(
        "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr>{}</a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>",
        run_properties("defRPr", text, "", Some(900), None)
    )
}

/// A title of a chart or of an axis.
struct TitleSpec<'a> {
    /// Without a text it is the title Excel makes up.
    text: Option<&'a str>,
    /// The size of its letters when it has none of its own, in hundredths
    /// of a point.
    size: u32,
    /// Running along a vertical axis, from bottom to top.
    upright: bool,
    overlay: bool,
    format: &'a TextFormat,
    layout: Option<&'a ManualLayout>,
}

/// `<c:title>` of a chart or of an axis.
fn push_title(x: &mut String, spec: TitleSpec) {
    let TitleSpec {
        text,
        size,
        upright,
        overlay,
        format,
        layout,
    } = spec;
    let properties = run_properties("defRPr", format, "", Some(size), Some(false));
    x.push_str("<c:title>");
    if let Some(text) = text {
        x.push_str("<c:tx><c:rich>");
        x.push_str(if upright {
            "<a:bodyPr rot=\"-5400000\" vert=\"horz\"/>"
        } else {
            "<a:bodyPr/>"
        });
        x.push_str("<a:lstStyle/>");
        // One paragraph per line.
        for paragraph in text.split('\n') {
            w!(x, "<a:p><a:pPr>{properties}</a:pPr>");
            if !paragraph.is_empty() {
                x.push_str("<a:r>");
                // Like Excel, a text with a font of its own says so on
                // its runs as well.
                if !format.is_auto() {
                    x.push_str(&run_properties(
                        "rPr",
                        format,
                        " lang=\"en-US\"",
                        None,
                        None,
                    ));
                }
                x.push_str("<a:t>");
                push_text(x, paragraph);
                x.push_str("</a:t></a:r>");
            }
            x.push_str("</a:p>");
        }
        x.push_str("</c:rich></c:tx>");
    }
    if let Some(layout) = layout.filter(placeable) {
        x.push_str(&layout_xml(layout, Placed::Corner));
    }
    w!(x, "<c:overlay val=\"{}\"/>", overlay as u8);
    if text.is_none() {
        // Excel writes the title it makes up small and bold unless told
        // otherwise: it is drawn like any other title.
        w!(
            x,
            "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr>{properties}</a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>"
        );
    }
    x.push_str("</c:title>");
}

/// `<c:numFmt>` of an axis or of data labels that do not show the format
/// of the cells.
fn push_number_format(x: &mut String, code: &str) {
    x.push_str("<c:numFmt formatCode=\"");
    push_attr(x, code);
    x.push_str("\" sourceLinked=\"0\"/>");
}

/// `<c:dLbls>` of a series of that plot, or of all its series.
/// `separator` is what stands between the parts of a label unless the
/// labels say what does.
fn labels_xml(labels: &DataLabels, plot: Plot, separator: &str, moved: &[LabelOffset]) -> String {
    let labels = plot.labels(labels);
    let shown = label_parts(&labels, separator);
    let mut x = String::from("<c:dLbls>");
    // The labels dragged away from their points: each says once more
    // what it shows.
    let mut seen = BTreeSet::new();
    for offset in moved {
        if offset.dx.is_finite() && offset.dy.is_finite() && seen.insert(offset.index) {
            w!(
                x,
                "<c:dLbl><c:idx val=\"{}\"/><c:layout><c:manualLayout><c:x val=\"{}\"/><c:y val=\"{}\"/></c:manualLayout></c:layout>{shown}</c:dLbl>",
                offset.index,
                num(offset.dx),
                num(offset.dy)
            );
        }
    }
    x.push_str(&shown);
    if plot.is_round() {
        w!(
            x,
            "<c:showLeaderLines val=\"{}\"/>",
            labels.leader_lines as u8
        );
    }
    x.push_str("</c:dLbls>");
    x
}

/// What data labels show and how, for all the labels of a series or for
/// one of them: the elements `c:dLbls` and `c:dLbl` share. `labels` are
/// those a file can hold ([`Plot::labels`]).
fn label_parts(labels: &DataLabels, separator: &str) -> String {
    let mut x = String::new();
    if let Some(code) = labels.number_format.as_deref().filter(|c| !c.is_empty()) {
        push_number_format(&mut x, code);
    }
    x.push_str(&text_xml(&labels.text));
    // An automatic position has no name: it is the absence of one.
    if let Some(position) = name_of(&LABEL_POSITIONS, labels.position) {
        w!(x, "<c:dLblPos val=\"{position}\"/>");
    }
    w!(
        x,
        "<c:showLegendKey val=\"{}\"/><c:showVal val=\"{}\"/><c:showCatName val=\"{}\"/><c:showSerName val=\"{}\"/><c:showPercent val=\"{}\"/><c:showBubbleSize val=\"{}\"/>",
        labels.legend_key as u8,
        labels.value as u8,
        labels.category as u8,
        labels.series_name as u8,
        labels.percent as u8,
        labels.bubble_size as u8
    );
    // The automatic separator is spelled out wherever it shows.
    let parts = [
        labels.value,
        labels.category,
        labels.series_name,
        labels.percent,
        labels.bubble_size,
    ];
    let several = parts.into_iter().filter(|shown| *shown).count() > 1;
    if let Some(separator) = labels.separator.as_deref().or(several.then_some(separator)) {
        x.push_str("<c:separator>");
        push_text(&mut x, separator);
        x.push_str("</c:separator>");
    }
    x
}

/// `<c:marker>` of a series or of one of its points, with the colours of
/// its inside and of its outline.
fn marker_xml(marker: &Marker, inside: Color, around: Color) -> String {
    let Some(symbol) =
        name_of(&MARKERS, marker.shape).filter(|_| marker.shape != MarkerShape::None)
    else {
        return "<c:marker><c:symbol val=\"none\"/></c:marker>".to_string();
    };
    let mut x = format!(
        "<c:marker><c:symbol val=\"{symbol}\"/><c:size val=\"{}\"/><c:spPr>",
        marker.size.clamp(2, 72)
    );
    if is_stroked(marker.shape) {
        x.push_str("<a:noFill/>");
    } else {
        x.push_str(&solid_fill(inside));
    }
    push_stroke(&mut x, OUTLINE_WIDTH, around, Dash::Solid, false);
    x.push_str("</c:spPr></c:marker>");
    x
}

/// The line of a series drawn as a line, in `<a:ln>`.
fn line_xml(s: &Series, kind: ChartKind, color: Color) -> String {
    let automatic = kind.default_line_width();
    let mut x = String::new();
    if s.line_shown(kind) {
        let width = s.line.width.unwrap_or(automatic);
        push_stroke(&mut x, width, color, s.line.dash, true);
    } else {
        // What Excel writes for points without a line between them.
        x.push_str("<a:ln w=\"");
        w!(x, "{}", (automatic as f64 * EMU_PER_PT).round() as i64);
        x.push_str("\" cap=\"rnd\"><a:noFill/><a:round/></a:ln>");
    }
    x
}

/// Whether a line is drawn where there is none unless told otherwise
/// (around bars, areas and bubbles, around the plot area): when it says
/// so, or says what it looks like.
fn is_drawn(line: &Line) -> bool {
    line.shown
        .unwrap_or(line.color.is_some() || line.width.is_some())
}

/// The outline of the shapes of a series (bars, areas, slices, bubbles,
/// the filled area of a radar chart), in `<a:ln>`; nothing where there
/// is none. `ink` is the colour of the series.
fn outline_xml(s: &Series, plot: Plot, kind: ChartKind, ink: Color) -> String {
    let mut x = String::new();
    let line = &s.line;
    if plot.is_round() {
        // White lines between the slices unless told otherwise.
        if line.shown == Some(false) {
            x.push_str(NO_LINE);
        } else {
            let width = line.width.unwrap_or(SLICE_OUTLINE_WIDTH);
            let color = line.color.unwrap_or(Color::WHITE);
            push_stroke(&mut x, width, color, line.dash, false);
        }
    } else if plot.filled {
        if line.shown == Some(false) {
            x.push_str(NO_LINE);
        } else {
            let width = line.width.unwrap_or(kind.default_line_width());
            push_stroke(&mut x, width, line.color.unwrap_or(ink), line.dash, false);
        }
    } else if is_drawn(line) && plot.family != Family::Line {
        let width = line.width.unwrap_or(OUTLINE_WIDTH);
        push_stroke(&mut x, width, line.color.unwrap_or(ink), line.dash, false);
    }
    x
}

/// `<c:spPr>` of the chart area or of the plot area. Left to itself, the
/// chart area is white in a thin grey frame and the plot area nothing.
fn area_xml(format: &ShapeFormat, chart_area: bool) -> String {
    let mut x = String::from("<c:spPr>");
    match format.fill {
        Fill::Solid(color) => x.push_str(&solid_fill(color)),
        Fill::Auto if chart_area => x.push_str(&solid_fill(Color::WHITE)),
        Fill::Auto | Fill::None => x.push_str("<a:noFill/>"),
    }
    let border = &format.border;
    if chart_area && border.shown != Some(false) || is_drawn(border) {
        let width = border.width.unwrap_or(OUTLINE_WIDTH);
        let color = border.color.unwrap_or(FRAME_GREY);
        push_stroke(&mut x, width, color, border.dash, false);
    } else {
        x.push_str(NO_LINE);
    }
    // The shadow and the glow of the area, after its line as in a shape.
    crate::xlsx::drawing::push_effects(&mut x, &format.effects);
    x.push_str("</c:spPr>");
    x
}

/// The `c:axId` of the axes of a chart. They only have to differ inside a
/// chart; like other writers, keep them apart across the charts of a
/// package as well.
struct AxisIds {
    category: u64,
    value: u64,
    second_value: u64,
    second_category: u64,
    /// The depth of a 3-D chart.
    series: u64,
}

impl AxisIds {
    /// `number` is the chart's number in the package.
    fn new(number: usize) -> AxisIds {
        let base = 50_000_000 + (number as u64 % 1_000_000) * 10;
        AxisIds {
            category: base + 1,
            value: base + 2,
            second_value: base + 3,
            second_category: base + 4,
            series: base + 5,
        }
    }
}

/// One axis element.
struct AxisSpec<'a> {
    axis: &'a Axis,
    /// A scale of numbers (`c:valAx`) rather than a list of categories.
    scale: bool,
    id: u64,
    /// The axis it crosses, and where: what the model keeps as the
    /// crossing of that other axis.
    crosses: (u64, AxisCrossing),
    /// The side of the plot it runs along: `b`, `l`, `t` or `r`.
    side: char,
    /// Scales: the points sit between the tick marks of the category axis
    /// (bars, lines) rather than on them (areas, X/Y charts).
    between: bool,
    /// Whether the line of the axis itself is drawn when the model leaves
    /// it alone: like Excel, all but the value scale of a chart with
    /// categories.
    line: bool,
}

fn push_axis(x: &mut String, spec: AxisSpec) {
    let a = spec.axis;
    let element = if spec.scale { "c:valAx" } else { "c:catAx" };
    w!(x, "<{element}><c:axId val=\"{}\"/><c:scaling>", spec.id);
    let log_base = a
        .log_base
        .filter(|b| spec.scale && (2.0..=1000.0).contains(b));
    if let Some(base) = log_base {
        w!(x, "<c:logBase val=\"{}\"/>", num(base));
    }
    w!(
        x,
        "<c:orientation val=\"{}\"/>",
        if a.reversed { "maxMin" } else { "minMax" }
    );
    if spec.scale {
        // A logarithmic scale has no zero, and no scale ends before it starts.
        let fits = |bound: &f64| bound.is_finite() && (log_base.is_none() || *bound > 0.0);
        let min = a.min.filter(fits);
        let max = a
            .max
            .filter(fits)
            .filter(|max| min.is_none_or(|min| min < *max));
        if let Some(max) = max {
            w!(x, "<c:max val=\"{}\"/>", num(max));
        }
        if let Some(min) = min {
            w!(x, "<c:min val=\"{}\"/>", num(min));
        }
    }
    w!(
        x,
        "</c:scaling><c:delete val=\"{}\"/><c:axPos val=\"{}\"/>",
        !a.visible as u8,
        spec.side
    );
    if a.major_gridlines {
        w!(
            x,
            "<c:majorGridlines>{}</c:majorGridlines>",
            rule_xml(&a.major_gridline, FRAME_GREY)
        );
    }
    if a.minor_gridlines {
        w!(
            x,
            "<c:minorGridlines>{}</c:minorGridlines>",
            rule_xml(&a.minor_gridline, MINOR_GREY)
        );
    }
    if let Some(title) = &a.title {
        push_title(
            x,
            TitleSpec {
                text: Some(title),
                size: 1000,
                upright: matches!(spec.side, 'l' | 'r'),
                overlay: false,
                format: &a.title_text,
                layout: a.title_layout.as_ref(),
            },
        );
    }
    match a.number_format.as_deref().filter(|code| !code.is_empty()) {
        Some(code) => push_number_format(x, code),
        None => x.push_str("<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/>"),
    }
    w!(
        x,
        "<c:majorTickMark val=\"{}\"/><c:minorTickMark val=\"{}\"/><c:tickLblPos val=\"{}\"/>",
        name_of(&TICK_MARKS, a.major_ticks).unwrap_or("none"),
        name_of(&TICK_MARKS, a.minor_ticks).unwrap_or("none"),
        name_of(&TICK_LABELS, a.labels).unwrap_or("nextTo")
    );
    // An axis without a line of its own gets one by being given a colour
    // or a width.
    let lined = a
        .line
        .shown
        .unwrap_or(spec.line || a.line.color.is_some() || a.line.width.is_some());
    if lined {
        x.push_str(&rule_xml(&a.line, FRAME_GREY));
    } else {
        w!(x, "<c:spPr>{NO_LINE}</c:spPr>");
    }
    x.push_str(&text_xml(&a.text));
    w!(x, "<c:crossAx val=\"{}\"/>", spec.crosses.0);
    match spec.crosses.1 {
        AxisCrossing::At(at) if at.is_finite() => {
            w!(x, "<c:crossesAt val=\"{}\"/>", num(at));
        }
        AxisCrossing::Min => x.push_str("<c:crosses val=\"min\"/>"),
        AxisCrossing::Max => x.push_str("<c:crosses val=\"max\"/>"),
        AxisCrossing::Auto | AxisCrossing::At(_) => x.push_str("<c:crosses val=\"autoZero\"/>"),
    }
    if !spec.scale {
        // `auto`: Excel makes it an axis of dates when the labels are dates.
        x.push_str("<c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/></c:catAx>");
        return;
    }
    w!(
        x,
        "<c:crossBetween val=\"{}\"/>",
        if spec.between { "between" } else { "midCat" }
    );
    for (element, unit) in [("majorUnit", a.major_unit), ("minorUnit", a.minor_unit)] {
        if let Some(unit) = unit.filter(|u| u.is_finite() && *u > 0.0) {
            w!(x, "<c:{element} val=\"{}\"/>", num(unit));
        }
    }
    let unit = match a.display_unit {
        DisplayUnit::None => None,
        DisplayUnit::Custom(by) => {
            (by.is_finite() && by > 0.0).then(|| format!("<c:custUnit val=\"{}\"/>", num(by)))
        }
        built_in => {
            name_of(&DISPLAY_UNITS, built_in).map(|name| format!("<c:builtInUnit val=\"{name}\"/>"))
        }
    };
    if let Some(unit) = unit {
        w!(x, "<c:dispUnits>{unit}");
        if a.display_unit_label {
            x.push_str("<c:dispUnitsLbl><c:layout/></c:dispUnitsLbl>");
        }
        x.push_str("</c:dispUnits>");
    }
    x.push_str("</c:valAx>");
}

/// The series of a chart that go into one chart-type element: those of
/// one plot, measured on the same pair of axes.
struct Group<'c> {
    plot: Plot,
    secondary: bool,
    /// (position in the chart, series, the kind it is drawn as)
    series: Vec<(usize, &'c Series, ChartKind)>,
}

/// The groups of a chart, in the order their first series come in.
fn groups(chart: &Chart) -> Vec<Group<'_>> {
    // Secondary axes are a second pair: with nothing on the first, the
    // series are simply on the axes of the chart.
    let split = chart.series.iter().any(|s| !chart.on_secondary(s));
    let mut out: Vec<Group> = Vec::new();
    for (index, s) in chart.series.iter().enumerate() {
        let kind = chart.series_kind(s);
        let plot = Plot::of(kind);
        let secondary = split && chart.on_secondary(s);
        match out
            .iter_mut()
            .find(|g| g.plot == plot && g.secondary == secondary)
        {
            Some(group) => group.series.push((index, s, kind)),
            None => out.push(Group {
                plot,
                secondary,
                series: vec![(index, s, kind)],
            }),
        }
    }
    if out.is_empty() {
        // A chart without series is still a chart of its type.
        out.push(Group {
            plot: Plot::of(chart.kind),
            secondary: false,
            series: Vec::new(),
        });
    }
    out
}

/// Whether the depth of a 3-D chart is an axis of its own: one row per
/// series, as lines and unstacked areas have.
fn has_series_axis(plot: Plot) -> bool {
    plot.is_3d
        && match plot.family {
            Family::Line => true,
            Family::Area => plot.grouping == Grouping::Standard,
            _ => false,
        }
}

/// The cells of a range that show something. Applications expect their
/// values next to every reference of a chart (and show them until they
/// have recalculated, or for good when they cannot reach the cells).
struct Source {
    sheet: usize,
    /// Number of cells in the range.
    count: u64,
    /// (position in the range counted row by row, row, column, value)
    cells: Vec<(u64, u32, u32, Value)>,
}

impl Source {
    /// Whether the range holds numbers and nothing else.
    fn is_numeric(&self) -> bool {
        !self.cells.is_empty()
            && self
                .cells
                .iter()
                .all(|(.., value)| matches!(value, Value::Number(_)))
    }
}

struct ChartWriter<'a> {
    wb: &'a Workbook,
    /// The sheet holding the chart: references without a sheet mean it.
    sheet: usize,
}

impl ChartWriter<'_> {
    fn source(&self, r: &RefExpr) -> Option<Source> {
        let sheet = match &r.sheet {
            Some(name) => self.wb.sheet_index(name)?,
            None => self.sheet,
        };
        let sh = self.wb.sheets.get(sheet)?;
        let rect = self.wb.clamp_to_used(sheet, r.rect());
        // Only the cells that exist are visited: a range may cover whole
        // columns of a sheet with a handful of cells.
        let mut filled: BTreeSet<(u32, u32)> = BTreeSet::new();
        let has_content = |content: &Content| !matches!(content, Content::Empty);
        if rect.rows() as usize <= sh.cells.len() {
            for row in rect.r0..=rect.r1 {
                filled.extend(
                    sh.cells
                        .range((row, rect.c0)..=(row, rect.c1))
                        .filter(|(_, cell)| has_content(&cell.content))
                        .map(|(at, _)| *at),
                );
            }
        } else {
            filled.extend(
                sh.cells
                    .range((rect.r0, rect.c0)..=(rect.r1, rect.c1))
                    .filter(|((_, col), cell)| {
                        (rect.c0..=rect.c1).contains(col) && has_content(&cell.content)
                    })
                    .map(|(at, _)| *at),
            );
        }
        for sp in &sh.spills {
            if sp.array.rows == 0 || sp.array.cols == 0 {
                continue;
            }
            let spilled = Rect::new(
                sp.row,
                sp.col,
                sp.row.saturating_add(sp.array.rows as u32 - 1),
                sp.col.saturating_add(sp.array.cols as u32 - 1),
            );
            if let Some(both) = rect.intersection(&spilled) {
                for row in both.r0..=both.r1 {
                    filled.extend((both.c0..=both.c1).map(|col| (row, col)));
                }
            }
        }
        let cells = filled
            .into_iter()
            .filter_map(|(row, col)| {
                let value = sh.value_at(row, col);
                let index = (row - rect.r0) as u64 * rect.cols() as u64 + (col - rect.c0) as u64;
                (!value.is_empty()).then_some((index, row, col, value))
            })
            .collect();
        Some(Source {
            sheet,
            count: rect.rows() as u64 * rect.cols() as u64,
            cells,
        })
    }

    /// A reference as chart parts spell it: with its sheet, and absolute.
    /// One to a sheet that is gone is written as the error it evaluates to:
    /// Excel takes a file naming an unknown sheet for a damaged one.
    fn formula(&self, r: &RefExpr) -> String {
        let sheet: Option<Arc<str>> = match &r.sheet {
            Some(name) => self.wb.sheet_index(name).map(|_| name.clone()),
            None => self
                .wb
                .sheets
                .get(self.sheet)
                .map(|sheet| Arc::from(sheet.name.as_str())),
        };
        let Some(sheet) = sheet else {
            return "#REF!".to_string();
        };
        let fixed = |corner: CellRef| CellRef {
            abs_row: true,
            abs_col: true,
            ..corner
        };
        RefExpr {
            sheet: Some(sheet),
            start: fixed(r.start),
            end: r.end.map(fixed),
            kind: r.kind,
        }
        .to_text()
    }

    /// `<c:f>` and, when the range can be read, `<{cache}>` with its size.
    /// The caller adds the points and closes the cache.
    fn open_ref<'s>(
        &self,
        x: &mut String,
        r: &RefExpr,
        source: Option<&'s Source>,
        cache: &str,
        format: Option<&str>,
    ) -> Option<&'s Source> {
        x.push_str("<c:f>");
        push_text(x, &self.formula(r));
        x.push_str("</c:f>");
        let source = source.filter(|s| s.count <= u32::MAX as u64)?;
        w!(x, "<{cache}>");
        if let Some(code) = format {
            x.push_str("<c:formatCode>");
            push_text(x, code);
            x.push_str("</c:formatCode>");
        }
        w!(x, "<c:ptCount val=\"{}\"/>", source.count);
        Some(source)
    }

    /// A reference to numbers. Cells without a number have no point.
    fn push_num_ref(&self, x: &mut String, r: &RefExpr, source: Option<&Source>) {
        let format = source.and_then(|s| {
            let (_, row, col, _) = s
                .cells
                .iter()
                .find(|(.., value)| matches!(value, Value::Number(_)))?;
            Some(self.wb.style_at(s.sheet, *row, *col).num_fmt.as_ref())
        });
        x.push_str("<c:numRef>");
        if let Some(source) = self.open_ref(
            x,
            r,
            source,
            "c:numCache",
            Some(format.unwrap_or("General")),
        ) {
            for (index, _, _, value) in &source.cells {
                if let Value::Number(n) = value
                    && n.is_finite()
                {
                    w!(x, "<c:pt idx=\"{index}\"><c:v>");
                    push_number(x, *n);
                    x.push_str("</c:v></c:pt>");
                }
            }
            x.push_str("</c:numCache>");
        }
        x.push_str("</c:numRef>");
    }

    /// A reference to labels: the cells as they are displayed.
    fn push_str_ref(&self, x: &mut String, r: &RefExpr, source: Option<&Source>) {
        x.push_str("<c:strRef>");
        if let Some(source) = self.open_ref(x, r, source, "c:strCache", None) {
            for (index, row, col, _) in &source.cells {
                let text = self.wb.display_text(source.sheet, *row, *col).text;
                if !text.is_empty() {
                    w!(x, "<c:pt idx=\"{index}\"><c:v>");
                    push_cell_text(x, &text);
                    x.push_str("</c:v></c:pt>");
                }
            }
            x.push_str("</c:strCache>");
        }
        x.push_str("</c:strRef>");
    }

    /// Labels on several levels (`c:multiLvlStrRef`) as Excel caches
    /// them: the level next to the plot first, and in each level the
    /// points that have a text.
    fn push_multi_level(&self, x: &mut String, r: &RefExpr, points: usize, levels: &[&[String]]) {
        x.push_str("<c:multiLvlStrRef><c:f>");
        push_text(x, &self.formula(r));
        w!(x, "</c:f><c:multiLvlStrCache><c:ptCount val=\"{points}\"/>");
        for level in levels {
            x.push_str("<c:lvl>");
            for (index, text) in level.iter().enumerate().filter(|(_, text)| !text.is_empty()) {
                w!(x, "<c:pt idx=\"{index}\"><c:v>");
                push_cell_text(x, text);
                x.push_str("</c:v></c:pt>");
            }
            x.push_str("</c:lvl>");
        }
        x.push_str("</c:multiLvlStrCache></c:multiLvlStrRef>");
    }

    /// The labels of a block of cells holding one level per column
    /// (`in_columns`) or per row, the one next to the data last.
    fn push_label_block(&self, x: &mut String, r: &RefExpr, in_columns: bool) {
        let sheet = match &r.sheet {
            Some(name) => self.wb.sheet_index(name),
            None => Some(self.sheet),
        };
        let Some(sheet) = sheet.filter(|sheet| *sheet < self.wb.sheets.len()) else {
            // Nothing to cache of a sheet that is gone.
            x.push_str("<c:multiLvlStrRef><c:f>");
            push_text(x, &self.formula(r));
            x.push_str("</c:f></c:multiLvlStrRef>");
            return;
        };
        let rect = self.wb.clamp_to_used(sheet, r.rect());
        let text = |row: u32, col: u32| self.wb.display_text(sheet, row, col).text;
        let levels: Vec<Vec<String>> = if in_columns {
            (rect.c0..=rect.c1).rev().map(|col| (rect.r0..=rect.r1).map(|row| text(row, col)).collect()).collect()
        } else {
            (rect.r0..=rect.r1).rev().map(|row| (rect.c0..=rect.c1).map(|col| text(row, col)).collect()).collect()
        };
        let points = if in_columns { rect.rows() } else { rect.cols() } as usize;
        let levels: Vec<&[String]> = levels.iter().map(Vec::as_slice).collect();
        self.push_multi_level(x, r, points, &levels);
    }

    /// Categories or X values: numbers when the cells hold nothing else,
    /// labels otherwise.
    fn push_axis_data(&self, x: &mut String, r: &RefExpr) {
        let source = self.source(r);
        if source.as_ref().is_some_and(Source::is_numeric) {
            self.push_num_ref(x, r, source.as_ref());
        } else {
            self.push_str_ref(x, r, source.as_ref());
        }
    }

    /// The sizes of the bubbles of a series. A series without sizes has
    /// bubbles of one size, which a file has to spell out.
    /// `values` are the cells of its values.
    fn push_bubble_sizes(&self, x: &mut String, s: &Series, values: Option<&Source>) {
        x.push_str("<c:bubbleSize>");
        match &s.bubble_sizes {
            Some(sizes) => {
                let sizes = series_line(sizes);
                self.push_num_ref(x, &sizes, self.source(&sizes).as_ref());
            }
            None => {
                let source = values.filter(|s| s.count <= u32::MAX as u64);
                w!(
                    x,
                    "<c:numLit><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>",
                    source.map_or(0, |s| s.count)
                );
                for (index, _, _, value) in source.iter().flat_map(|s| &s.cells) {
                    if matches!(value, Value::Number(n) if n.is_finite()) {
                        w!(x, "<c:pt idx=\"{index}\"><c:v>1</c:v></c:pt>");
                    }
                }
                x.push_str("</c:numLit>");
            }
        }
        x.push_str("</c:bubbleSize>");
    }

    /// `<c:dPt>`: the points of a series that do not look like the others,
    /// and all of them where every point has a colour of its own: Excel
    /// would pick those colours itself, and not always the engine's.
    /// `count` is the number of points to give a colour then, `ink` the
    /// colour of the series.
    fn push_points(
        &self,
        x: &mut String,
        s: &Series,
        (plot, kind): (Plot, ChartKind),
        count: u64,
        ink: Color,
    ) {
        // The points with a format of their own (the first for each
        // point), among those that get a colour anyway.
        let mut formatted: Vec<_> = s.points.iter().collect();
        formatted.sort_by_key(|p| p.index);
        formatted.dedup_by_key(|p| p.index);
        let mut points: Vec<(u32, Option<_>)> = (0..count as u32).map(|i| (i, None)).collect();
        for point in formatted {
            if (point.index as u64) < count {
                points[point.index as usize].1 = Some(point);
            } else {
                points.push((point.index, Some(point)));
            }
        }
        let marker = s.marker_for(kind);
        for (index, point) in points {
            let varied = ((index as u64) < count).then(|| self.wb.theme().chart_color(index as usize));
            let color = point.and_then(|p| p.color).or(varied);
            // Only slices can be pulled out.
            let explosion = point.and_then(|p| p.explosion).filter(|_| plot.is_round());
            if color.is_none() && explosion.is_none() {
                continue;
            }
            w!(x, "<c:dPt><c:idx val=\"{index}\"/>");
            if matches!(plot.family, Family::Column | Family::Bar | Family::Bubble) {
                x.push_str("<c:invertIfNegative val=\"0\"/>");
            }
            if plot.is_line_like()
                && marker.shape != MarkerShape::None
                && let Some(color) = color
            {
                x.push_str(&marker_xml(&marker, color, color));
            }
            x.push_str("<c:bubble3D val=\"0\"/>");
            if let Some(explosion) = explosion {
                w!(x, "<c:explosion val=\"{explosion}\"/>");
            }
            if let Some(color) = color {
                // The formatting of a point replaces that of its series as
                // a whole: what the point does not change is repeated.
                if plot.is_line_like() {
                    w!(x, "<c:spPr>{}</c:spPr>", line_xml(s, kind, color));
                } else {
                    w!(
                        x,
                        "<c:spPr>{}{}</c:spPr>",
                        shape_fill(color, plot),
                        outline_xml(s, plot, kind, ink)
                    );
                }
            }
            x.push_str("</c:dPt>");
        }
    }

    /// `ink` is the colour of the series, which is that of its trendlines
    /// unless they have one of their own.
    fn push_trendlines(&self, x: &mut String, s: &Series, plot: Plot, ink: Color) {
        if !plot.takes_trendlines() {
            return;
        }
        for trend in &s.trendlines {
            x.push_str("<c:trendline>");
            if let Some(name) = &trend.name {
                x.push_str("<c:name>");
                push_text(x, name);
                x.push_str("</c:name>");
            }
            x.push_str("<c:spPr>");
            let line = &trend.line;
            if *line == Line::default() {
                // Excel's own: round dots in the colour of the series.
                push_stroke(x, TREND_WIDTH, ink, Dash::RoundDot, true);
            } else if line.shown == Some(false) {
                x.push_str(NO_LINE);
            } else {
                let width = line.width.unwrap_or(TREND_WIDTH);
                push_stroke(x, width, line.color.unwrap_or(ink), line.dash, true);
            }
            x.push_str("</c:spPr>");
            let name = match trend.kind {
                TrendKind::Linear => "linear",
                TrendKind::Exponential => "exp",
                TrendKind::Logarithmic => "log",
                TrendKind::Polynomial(_) => "poly",
                TrendKind::Power => "power",
                TrendKind::MovingAverage(_) => "movingAvg",
            };
            w!(x, "<c:trendlineType val=\"{name}\"/>");
            // Excel refuses a file for an order, a period or an intercept
            // it would not let one type in.
            let mut fitted = true;
            let mut intercept = None;
            match trend.kind {
                TrendKind::Polynomial(order) => {
                    w!(x, "<c:order val=\"{}\"/>", order.clamp(2, 6));
                    intercept = trend.intercept;
                }
                TrendKind::MovingAverage(period) => {
                    w!(x, "<c:period val=\"{}\"/>", period.max(2));
                    fitted = false;
                }
                TrendKind::Linear => intercept = trend.intercept,
                TrendKind::Exponential => intercept = trend.intercept.filter(|at| *at > 0.0),
                TrendKind::Logarithmic | TrendKind::Power => {}
            }
            if fitted {
                for (element, periods) in [("forward", trend.forward), ("backward", trend.backward)]
                {
                    if periods.is_finite() && periods > 0.0 {
                        w!(x, "<c:{element} val=\"{}\"/>", num(periods));
                    }
                }
                if let Some(at) = intercept.filter(|at| at.is_finite()) {
                    w!(x, "<c:intercept val=\"{}\"/>", num(at));
                }
            }
            w!(
                x,
                "<c:dispRSqr val=\"{}\"/><c:dispEq val=\"{}\"/></c:trendline>",
                (fitted && trend.show_r2) as u8,
                (fitted && trend.show_equation) as u8
            );
        }
    }

    fn push_error_bars(&self, x: &mut String, s: &Series, plot: Plot) {
        if !plot.takes_error_bars() {
            return;
        }
        // One set along each axis the series is measured on.
        let mut written: Vec<bool> = Vec::new();
        for bars in &s.error_bars {
            if (bars.horizontal && !plot.is_xy()) || written.contains(&bars.horizontal) {
                continue;
            }
            written.push(bars.horizontal);
            x.push_str("<c:errBars>");
            if plot.is_xy() {
                w!(
                    x,
                    "<c:errDir val=\"{}\"/>",
                    if bars.horizontal { 'x' } else { 'y' }
                );
            } else if plot.family != Family::Bar {
                // Excel leaves the direction out for horizontal bars.
                x.push_str("<c:errDir val=\"y\"/>");
            }
            let direction = match bars.direction {
                ErrorDirection::Both => "both",
                ErrorDirection::Plus => "plus",
                ErrorDirection::Minus => "minus",
            };
            let (kind, amount) = match bars.amount {
                ErrorAmount::Fixed(by) => ("fixedVal", Some(by)),
                ErrorAmount::Percent(by) => ("percentage", Some(by)),
                ErrorAmount::StdDev(by) => ("stdDev", Some(by)),
                ErrorAmount::StdErr => ("stdErr", None),
            };
            w!(
                x,
                "<c:errBarType val=\"{direction}\"/><c:errValType val=\"{kind}\"/><c:noEndCap val=\"{}\"/>",
                !bars.end_cap as u8
            );
            if let Some(by) = amount {
                let by = if by.is_finite() { by.max(0.0) } else { 0.0 };
                w!(x, "<c:val val=\"{}\"/>", num(by));
            }
            x.push_str("</c:errBars>");
        }
    }

    /// `index` is the position of the series in the chart, `kind` the kind
    /// it is drawn as.
    fn push_series(
        &self,
        x: &mut String,
        chart: &Chart,
        plot: Plot,
        (index, s, kind): (usize, &Series, ChartKind),
    ) {
        w!(
            x,
            "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>"
        );
        match &s.name {
            SeriesName::None => {}
            SeriesName::Text(text) => {
                x.push_str("<c:tx><c:v>");
                push_cell_text(x, text);
                x.push_str("</c:v></c:tx>");
            }
            SeriesName::Cell(r) => {
                let r = name_cell(r);
                x.push_str("<c:tx>");
                self.push_str_ref(x, &r, self.source(&r).as_ref());
                x.push_str("</c:tx>");
            }
        }
        // The colour goes where the series has its ink: the stroke of a
        // line and its markers, the fill of anything else. What a series
        // looks like is always spelled out, whatever the type of the
        // chart would give it, and so is its automatic colour, which is
        // the engine's for the theme of the workbook.
        let ink = s.color.unwrap_or_else(|| self.wb.theme().chart_color(index));
        // Where every point has a colour of its own, each says which.
        let varied = chart.varies_colors() && (chart.kind.is_round() || chart.series.len() == 1);
        // Excel reads a series along one row or one column.
        let plotted = series_line(&s.values);
        // A PivotChart whose rows have several fields plots the items
        // among the rows of its ranges, and the caches hold those only.
        let pivot_points = self.wb.pivot_chart_points(chart.id);
        let values = self.source(&plotted).map(|mut source| {
            if let Some(points) = &pivot_points {
                source.cells = std::mem::take(&mut source.cells)
                    .into_iter()
                    .filter_map(|(index, row, col, value)| {
                        let at = points.keep.iter().position(|k| *k as u64 == index)?;
                        Some((at as u64, row, col, value))
                    })
                    .collect();
                source.count = points.keep.len() as u64;
            }
            source
        });
        let points = match &values {
            Some(values) if varied => values.count.min(VARIED_POINTS),
            _ => 0,
        };
        if plot.is_line_like() {
            w!(
                x,
                "<c:spPr>{}</c:spPr>",
                line_xml(s, kind, s.line.color.unwrap_or(ink))
            );
            let marker = s.marker_for(kind);
            let (inside, around) = (marker.fill.unwrap_or(ink), marker.line.unwrap_or(ink));
            x.push_str(&marker_xml(&marker, inside, around));
        } else {
            // A fill for the whole series would hide the colours Excel
            // gives the points beyond those that are spelled out.
            let fill = match s.color {
                None if varied => String::new(),
                _ => shape_fill(ink, plot),
            };
            let outline = outline_xml(s, plot, kind, ink);
            if !fill.is_empty() || !outline.is_empty() {
                w!(x, "<c:spPr>{fill}{outline}</c:spPr>");
            }
            if matches!(plot.family, Family::Column | Family::Bar | Family::Bubble) {
                x.push_str("<c:invertIfNegative val=\"0\"/>");
            }
            if plot.is_round() && s.explosion > 0 {
                w!(x, "<c:explosion val=\"{}\"/>", s.explosion);
            }
        }
        self.push_points(x, s, (plot, kind), points, ink);
        match &s.labels {
            Some(own) if own.any() => x.push_str(&labels_xml(
                own,
                plot,
                &list_separator(self.wb.locale),
                &s.label_offsets,
            )),
            // Labels that were dragged are said on their series, whose
            // labels the chart's then are.
            None if !s.label_offsets.is_empty()
                && chart.data_labels
                && chart.label_options.any() =>
            {
                x.push_str(&labels_xml(
                    &chart.label_options,
                    plot,
                    &list_separator(self.wb.locale),
                    &s.label_offsets,
                ))
            }
            // "None for this one" only has to be said where the others
            // have labels.
            Some(_) if chart.data_labels && chart.label_options.any() => {
                x.push_str("<c:dLbls><c:delete val=\"1\"/></c:dLbls>")
            }
            _ => {}
        }
        self.push_trendlines(x, s, plot, ink);
        self.push_error_bars(x, s, plot);
        let (along, across) = if plot.is_xy() {
            ("c:xVal", "c:yVal")
        } else {
            ("c:cat", "c:val")
        };
        if let Some(r) = &s.categories {
            w!(x, "<{along}>");
            if plot.is_xy() {
                self.push_axis_data(x, &innermost_level(r, &plotted));
            } else if let Some(points) = &pivot_points {
                let levels: Vec<&[String]> = std::iter::once(points.labels.as_slice()).chain(points.outer_levels.iter().map(Vec::as_slice)).collect();
                self.push_multi_level(x, r, points.keep.len(), &levels);
            } else if let Some(in_columns) = category_levels(r.rect(), plotted.rect()) {
                self.push_label_block(x, r, in_columns);
            } else {
                self.push_axis_data(x, r);
            }
            w!(x, "</{along}>");
        }
        w!(x, "<{across}>");
        self.push_num_ref(x, &plotted, values.as_ref());
        w!(x, "</{across}>");
        if plot.family == Family::Bubble {
            self.push_bubble_sizes(x, s, values.as_ref());
            x.push_str("<c:bubble3D val=\"0\"/>");
        }
        if plot.has_smoothing() {
            // Ribbons in 3-D are never curved.
            let smooth = !plot.is_3d && s.smooth_for(kind);
            w!(x, "<c:smooth val=\"{}\"/>", smooth as u8);
        }
        x.push_str("</c:ser>");
    }

    fn push_group(&self, x: &mut String, chart: &Chart, group: &Group, ids: &AxisIds) {
        let plot = group.plot;
        // What the element says of its series as a whole follows the first.
        let lead = group.series.first().map_or(chart.kind, |(.., kind)| *kind);
        let element = plot.element();
        w!(x, "<c:{element}>");
        let bars = matches!(plot.family, Family::Column | Family::Bar);
        let grouping = match plot.grouping {
            Grouping::Standard if bars => "clustered",
            Grouping::Standard => "standard",
            Grouping::Stacked => "stacked",
            Grouping::Percent => "percentStacked",
        };
        match plot.family {
            Family::Column | Family::Bar => {
                let direction = if plot.family == Family::Bar {
                    "bar"
                } else {
                    "col"
                };
                w!(
                    x,
                    "<c:barDir val=\"{direction}\"/><c:grouping val=\"{grouping}\"/>"
                );
            }
            Family::Line | Family::Area => {
                w!(x, "<c:grouping val=\"{grouping}\"/>");
            }
            Family::Scatter => {
                // Excel knows these two only; the series say the rest.
                let style = if lead.is_smooth() {
                    "smoothMarker"
                } else {
                    "lineMarker"
                };
                w!(x, "<c:scatterStyle val=\"{style}\"/>");
            }
            Family::Radar => {
                let style = if plot.filled {
                    "filled"
                } else if lead.has_markers() {
                    "marker"
                } else {
                    "standard"
                };
                w!(x, "<c:radarStyle val=\"{style}\"/>");
            }
            Family::Pie | Family::Doughnut | Family::Bubble => {}
        }
        w!(x, "<c:varyColors val=\"{}\"/>", chart.varies_colors() as u8);
        for series in &group.series {
            self.push_series(x, chart, plot, *series);
        }
        if chart.data_labels && chart.label_options.any() {
            let separator = list_separator(self.wb.locale);
            x.push_str(&labels_xml(&chart.label_options, plot, &separator, &[]));
        }
        let (category, value) = if group.secondary {
            (ids.second_category, ids.second_value)
        } else {
            (ids.category, ids.value)
        };
        let axes = format!("<c:axId val=\"{category}\"/><c:axId val=\"{value}\"/>");
        // A 3-D chart names its depth axis too, or says it has none.
        let depth = if has_series_axis(plot) { ids.series } else { 0 };
        match (plot.family, plot.is_3d) {
            (Family::Column | Family::Bar, false) => {
                w!(
                    x,
                    "<c:gapWidth val=\"{}\"/><c:overlap val=\"{}\"/>{axes}",
                    chart.gap_width_or_default(),
                    chart.overlap_for(lead)
                );
            }
            (Family::Column | Family::Bar, true) => {
                w!(
                    x,
                    "<c:gapWidth val=\"{}\"/><c:shape val=\"box\"/>{axes}<c:axId val=\"{depth}\"/>",
                    chart.gap_width_or_default()
                );
            }
            (Family::Line, false) => {
                w!(x, "<c:marker val=\"1\"/>{axes}");
            }
            (Family::Line | Family::Area, true) => {
                w!(x, "{axes}<c:axId val=\"{depth}\"/>");
            }
            (Family::Area | Family::Scatter | Family::Radar, _) => x.push_str(&axes),
            // Turning a pie in 3-D is a matter of the camera.
            (Family::Pie, true) => {}
            (Family::Pie, false) => {
                w!(
                    x,
                    "<c:firstSliceAng val=\"{}\"/>",
                    chart.first_slice_angle.min(360)
                );
            }
            (Family::Doughnut, _) => {
                w!(
                    x,
                    "<c:firstSliceAng val=\"{}\"/><c:holeSize val=\"{}\"/>",
                    chart.first_slice_angle.min(360),
                    chart.hole_size.clamp(10, 90)
                );
            }
            (Family::Bubble, _) => {
                w!(
                    x,
                    "<c:bubbleScale val=\"{}\"/><c:showNegBubbles val=\"0\"/>{axes}",
                    chart.bubble_scale.min(300)
                );
            }
        }
        w!(x, "</c:{element}>");
    }

    /// The axes of the plot area: the pair of the chart, the second pair
    /// when series are measured on it, the depth of a 3-D chart.
    fn push_axes(&self, x: &mut String, chart: &Chart, groups: &[Group], ids: &AxisIds) {
        let plot = Plot::of(chart.kind);
        if plot.is_round() {
            return;
        }
        let axes = &chart.axes;
        // Both axes of an X/Y chart are scales.
        let xy = plot.is_xy();
        // Horizontal bars have their categories up the left side.
        let (along, across, far) = if chart.kind.is_horizontal() {
            ('l', 'b', 't')
        } else {
            ('b', 'l', 'r')
        };
        let between = |secondary: bool| {
            groups
                .iter()
                .filter(|g| g.secondary == secondary)
                .any(|g| !g.plot.is_xy() && g.plot.family != Family::Area)
        };
        push_axis(
            x,
            AxisSpec {
                axis: &axes.category,
                scale: xy,
                id: ids.category,
                crosses: (ids.value, axes.value.crossing),
                side: along,
                between: between(false),
                line: true,
            },
        );
        push_axis(
            x,
            AxisSpec {
                axis: &axes.value,
                scale: true,
                id: ids.value,
                crosses: (ids.category, axes.category.crossing),
                side: across,
                between: between(false),
                line: xy,
            },
        );
        if groups.iter().any(|g| g.secondary) {
            // As Excel has them: the second scale at the far end of the
            // categories unless the model says where, and the second
            // category axis, when it is shown, at the far end of that
            // scale (at the near end, it would lie on the first one).
            let crossing = match axes.secondary_category.crossing {
                AxisCrossing::Auto => AxisCrossing::Max,
                at => at,
            };
            let category_crossing = match axes.secondary_value.crossing {
                AxisCrossing::Auto if axes.secondary_category.visible => AxisCrossing::Max,
                at => at,
            };
            push_axis(
                x,
                AxisSpec {
                    axis: &axes.secondary_value,
                    scale: true,
                    id: ids.second_value,
                    crosses: (ids.second_category, crossing),
                    side: far,
                    between: between(true),
                    line: xy,
                },
            );
            push_axis(
                x,
                AxisSpec {
                    axis: &axes.secondary_category,
                    scale: xy,
                    id: ids.second_category,
                    crosses: (ids.second_value, category_crossing),
                    side: along,
                    between: between(true),
                    line: true,
                },
            );
        }
        if has_series_axis(plot) {
            w!(
                x,
                "<c:serAx><c:axId val=\"{}\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/>{}<c:crossAx val=\"{}\"/><c:crosses val=\"autoZero\"/></c:serAx>",
                ids.series,
                hairline(FRAME_GREY),
                ids.value
            );
        }
    }

    /// The chart part of a chart; `number` is the chart's number in the
    /// package. A chart read from a file and left as it was is the text
    /// it was read from, when that text fits the package being written.
    fn chart_xml(&self, chart: &Chart, number: usize) -> String {
        let sheet_exists = |name: &str| self.wb.sheet_index(name).is_some();
        match chart
            .unchanged_source()
            .and_then(|xml| foreign::reusable(xml, &sheet_exists))
        {
            Some(xml) => xml.into_owned(),
            // A PivotChart also says which PivotTable it draws.
            None => super::super::pivot::bind_chart(self.wb, chart, self.generate(chart, number)),
        }
    }

    /// A chart part that says what the model says.
    fn generate(&self, chart: &Chart, number: usize) -> String {
        let kind = chart.kind;
        let ids = AxisIds::new(number);
        let groups = groups(chart);
        let mut x = String::from(XML_DECL);
        w!(
            x,
            "<c:chartSpace xmlns:c=\"{NS_CHART}\" xmlns:a=\"{NS_DRAWING}\" xmlns:r=\"{NS_DOC_REL}\">"
        );
        x.push_str("<c:date1904 val=\"0\"/><c:roundedCorners val=\"0\"/><c:chart>");
        // A title element without text is the title Excel makes up: the name
        // of the series when there is one series, a placeholder otherwise.
        match &chart.title {
            ChartTitle::Text(_) | ChartTitle::Auto => {
                push_title(
                    &mut x,
                    TitleSpec {
                        text: match &chart.title {
                            ChartTitle::Text(text) => Some(text),
                            _ => None,
                        },
                        size: 1400,
                        upright: false,
                        overlay: chart.title_overlay,
                        format: &chart.title_text,
                        layout: chart.title_layout.as_ref(),
                    },
                );
                x.push_str("<c:autoTitleDeleted val=\"0\"/>");
            }
            ChartTitle::None => x.push_str("<c:autoTitleDeleted val=\"1\"/>"),
        }
        if kind.is_3d() {
            let view = chart.view3d_or_default();
            // A pie has one angle for the camera and for its first slice.
            let turn = if kind == ChartKind::Pie3D {
                (view.rot_y % 360 + chart.first_slice_angle % 360) % 360
            } else {
                view.rot_y.min(360)
            };
            w!(
                x,
                "<c:view3D><c:rotX val=\"{}\"/>",
                view.rot_x.clamp(-90, 90)
            );
            if let Some(height) = view.height_percent {
                w!(x, "<c:hPercent val=\"{}\"/>", height.clamp(5, 500));
            }
            w!(
                x,
                "<c:rotY val=\"{turn}\"/><c:depthPercent val=\"{}\"/><c:rAngAx val=\"{}\"/><c:perspective val=\"{}\"/></c:view3D>",
                view.depth_percent.clamp(20, 2000),
                view.right_angle_axes as u8,
                view.perspective.min(240)
            );
            // Floor and walls without thickness or colour, as Excel's own.
            for wall in ["floor", "sideWall", "backWall"] {
                w!(
                    x,
                    "<c:{wall}><c:thickness val=\"0\"/><c:spPr><a:noFill/>{NO_LINE}</c:spPr></c:{wall}>"
                );
            }
        }
        x.push_str("<c:plotArea>");
        let sized = |m: &&ManualLayout| {
            let given = |v: Option<f64>| v.is_some_and(|v| v.is_finite() && v > 0.0);
            placeable(m) && given(m.w) && given(m.h)
        };
        match chart.plot_layout.as_ref().filter(sized) {
            Some(layout) => x.push_str(&layout_xml(
                layout,
                Placed::Plot {
                    inner: !chart.plot_layout_outer,
                },
            )),
            None => x.push_str("<c:layout/>"),
        }
        for group in &groups {
            self.push_group(&mut x, chart, group, &ids);
        }
        self.push_axes(&mut x, chart, &groups, &ids);
        if let Some(table) = chart
            .data_table
            .filter(|_| Plot::of(kind).takes_data_table())
        {
            w!(
                x,
                "<c:dTable><c:showHorzBorder val=\"{}\"/><c:showVertBorder val=\"{}\"/><c:showOutline val=\"{}\"/><c:showKeys val=\"{}\"/></c:dTable>",
                table.horizontal_lines as u8,
                table.vertical_lines as u8,
                table.outline as u8,
                table.legend_keys as u8
            );
        }
        x.push_str(&area_xml(&chart.plot_area, false));
        x.push_str("</c:plotArea>");

        let legend = match chart.legend {
            LegendPos::None => None,
            LegendPos::Bottom => Some("b"),
            LegendPos::Right => Some("r"),
            LegendPos::Top => Some("t"),
            LegendPos::Left => Some("l"),
            LegendPos::TopRight => Some("tr"),
        };
        if let Some(side) = legend {
            w!(x, "<c:legend><c:legendPos val=\"{side}\"/>");
            let mut hidden = BTreeSet::new();
            for entry in &chart.legend_hidden {
                if hidden.insert(*entry) {
                    w!(
                        x,
                        "<c:legendEntry><c:idx val=\"{entry}\"/><c:delete val=\"1\"/></c:legendEntry>"
                    );
                }
            }
            if let Some(layout) = chart.legend_layout.as_ref().filter(placeable) {
                x.push_str(&layout_xml(layout, Placed::Frame));
            }
            w!(
                x,
                "<c:overlay val=\"{}\"/>{}</c:legend>",
                chart.legend_overlay as u8,
                text_xml(&chart.legend_text)
            );
        }
        let blanks = match chart.blanks {
            BlanksAs::Gap => "gap",
            BlanksAs::Zero => "zero",
            BlanksAs::Span => "span",
        };
        w!(
            x,
            "<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"{blanks}\"/></c:chart>"
        );
        x.push_str(&area_xml(&chart.chart_area, true));
        w!(
            x,
            "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr><a:solidFill><a:srgbClr val=\"{TEXT_GREY}\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>"
        );
        x.push_str("</c:chartSpace>");
        x
    }
}
