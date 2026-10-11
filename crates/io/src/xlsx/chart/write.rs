//! Writing charts: one drawing part per sheet that has charts, one chart
//! part per chart.
//!
//! Excel validates these parts against the schema and offers to "repair"
//! (that is, drop) a chart at the first surprise, so the order of the
//! elements below is the order of the schema, whatever would read better.

use super::{EMU_PER_PX, Track, innermost_level, name_cell, series_line};
use crate::xlsx::points_to_px;
use crate::xlsx::write::{
    MAX_COL_WIDTH_PX, MAX_ROW_HEIGHT_PT, NS_DOC_REL, NS_PKG_REL, push_number,
};
use crate::xlsx::xml::{XML_DECL, push_cell_text, push_text};
use opensheet_engine::ast::{CellRef, RefExpr};
use opensheet_engine::chart::{
    Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::workbook::{Content, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
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

/// Gridlines and axis lines: the light grey of the charts Excel makes.
const LINE_GREY: &str = "D9D9D9";
/// Titles, labels and legends: dark grey rather than black.
const TEXT_GREY: &str = "595959";

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
            default: DEFAULT_COL_WIDTH,
            sizes: &sheet.col_widths,
            hidden: &sheet.hidden_cols,
            max: MAX_COL_WIDTH_PX,
            count: MAX_COLS,
        };
        let rows = Track {
            default: DEFAULT_ROW_HEIGHT,
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
        for (i, chart) in sheet.charts.iter().enumerate() {
            chart_count += 1;
            push_anchor(&mut xml, chart, i + 1, &cols, &rows);
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
        });
    }
    out
}

// ---- drawing -------------------------------------------------------------------

fn push_marker(x: &mut String, name: &str, (col, x_px): (u32, f64), (row, y_px): (u32, f64)) {
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
    push_marker(x, "from", left, top);
    push_marker(x, "to", right, bottom);
    w!(
        x,
        "<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{}\" name=\"Chart {number}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>",
        number + 1
    );
    x.push_str("<xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm>");
    w!(
        x,
        "<a:graphic><a:graphicData uri=\"{NS_CHART}\"><c:chart xmlns:c=\"{NS_CHART}\" xmlns:r=\"{NS_DOC_REL}\" r:id=\"rId{number}\"/></a:graphicData></a:graphic>"
    );
    x.push_str("</xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>");
}

// ---- chart -----------------------------------------------------------------------

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

fn solid_fill(color: Color) -> String {
    format!(
        "<a:solidFill><a:srgbClr val=\"{:06X}\"/></a:solidFill>",
        color.0 & 0xFF_FFFF
    )
}

/// A hairline of the given colour.
fn line(rgb: &str) -> String {
    format!(
        "<c:spPr><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill></a:ln></c:spPr>"
    )
}

fn push_title(x: &mut String, text: &str) {
    x.push_str("<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/>");
    // One paragraph per line.
    for paragraph in text.split('\n') {
        x.push_str("<a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr>");
        if !paragraph.is_empty() {
            x.push_str("<a:r><a:t>");
            push_text(x, paragraph);
            x.push_str("</a:t></a:r>");
        }
        x.push_str("</a:p>");
    }
    x.push_str("</c:rich></c:tx><c:overlay val=\"0\"/></c:title>");
}

#[derive(Clone, Copy)]
enum AxisKind {
    /// The labels of a chart with categories.
    Category,
    /// The scale such a chart measures its values on. `between`: whether
    /// the points sit between the tick marks of the category axis (bars)
    /// or on them.
    Value { between: bool },
    /// Either scale of a scatter chart.
    Xy,
}

/// One axis running along the given side of the plot (`b`ottom or `l`eft).
/// Scales carry gridlines; the lines of the axes themselves are drawn
/// except for the value scale of a chart with categories, as Excel does.
fn push_axis(x: &mut String, kind: AxisKind, (id, crossing): (u64, u64), side: char) {
    let element = match kind {
        AxisKind::Category => "c:catAx",
        AxisKind::Value { .. } | AxisKind::Xy => "c:valAx",
    };
    w!(
        x,
        "<{element}><c:axId val=\"{id}\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{side}\"/>"
    );
    if !matches!(kind, AxisKind::Category) {
        w!(
            x,
            "<c:majorGridlines>{}</c:majorGridlines>",
            line(LINE_GREY)
        );
    }
    x.push_str("<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/>");
    if matches!(kind, AxisKind::Value { .. }) {
        x.push_str("<c:spPr><a:ln><a:noFill/></a:ln></c:spPr>");
    } else {
        x.push_str(&line(LINE_GREY));
    }
    w!(
        x,
        "<c:crossAx val=\"{crossing}\"/><c:crosses val=\"autoZero\"/>"
    );
    match kind {
        AxisKind::Category => x.push_str(
            "<c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/>",
        ),
        AxisKind::Value { between: true } => x.push_str("<c:crossBetween val=\"between\"/>"),
        AxisKind::Value { between: false } | AxisKind::Xy => {
            x.push_str("<c:crossBetween val=\"midCat\"/>")
        }
    }
    w!(x, "</{element}>");
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

    fn push_series(&self, x: &mut String, kind: ChartKind, index: usize, s: &Series) {
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
        // line, the markers of a scatter chart, the fill of anything else.
        let fill = s.color.map(solid_fill);
        match kind {
            ChartKind::Line => {
                if let Some(fill) = &fill {
                    w!(
                        x,
                        "<c:spPr><a:ln w=\"28575\" cap=\"rnd\">{fill}<a:round/></a:ln></c:spPr>"
                    );
                }
                x.push_str("<c:marker><c:symbol val=\"none\"/></c:marker>");
            }
            ChartKind::Scatter => {
                // Points only: no line from one to the next.
                x.push_str(
                    "<c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:noFill/><a:round/></a:ln></c:spPr>",
                );
                x.push_str("<c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/>");
                if let Some(fill) = &fill {
                    w!(x, "<c:spPr>{fill}<a:ln w=\"9525\">{fill}</a:ln></c:spPr>");
                }
                x.push_str("</c:marker>");
            }
            _ => {
                if let Some(fill) = &fill {
                    w!(x, "<c:spPr>{fill}</c:spPr>");
                }
                if !matches!(kind, ChartKind::Area | ChartKind::Pie | ChartKind::Doughnut) {
                    x.push_str("<c:invertIfNegative val=\"0\"/>");
                }
            }
        }
        let (categories, values) = match kind {
            ChartKind::Scatter => ("c:xVal", "c:yVal"),
            _ => ("c:cat", "c:val"),
        };
        // Excel reads a series along one row or one column.
        let plotted = series_line(&s.values);
        if let Some(r) = &s.categories {
            w!(x, "<{categories}>");
            self.push_axis_data(x, &innermost_level(r, &plotted));
            w!(x, "</{categories}>");
        }
        w!(x, "<{values}>");
        self.push_num_ref(x, &plotted, self.source(&plotted).as_ref());
        w!(x, "</{values}>");
        if matches!(kind, ChartKind::Line | ChartKind::Scatter) {
            x.push_str("<c:smooth val=\"0\"/>");
        }
        x.push_str("</c:ser>");
    }

    /// `number` is the chart's number in the package.
    fn chart_xml(&self, chart: &Chart, number: usize) -> String {
        let kind = chart.kind;
        let mut x = String::from(XML_DECL);
        w!(
            x,
            "<c:chartSpace xmlns:c=\"{NS_CHART}\" xmlns:a=\"{NS_DRAWING}\" xmlns:r=\"{NS_DOC_REL}\">"
        );
        x.push_str("<c:date1904 val=\"0\"/><c:roundedCorners val=\"0\"/><c:chart>");
        // A title element without text is the title Excel makes up: the name
        // of the series when there is one series, a placeholder otherwise.
        match &chart.title {
            ChartTitle::Text(text) => {
                push_title(&mut x, text);
                x.push_str("<c:autoTitleDeleted val=\"0\"/>");
            }
            ChartTitle::Auto => x.push_str(
                "<c:title><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
            ),
            ChartTitle::None => x.push_str("<c:autoTitleDeleted val=\"1\"/>"),
        }
        x.push_str("<c:plotArea><c:layout/>");

        let column = |grouping: &str, direction: &str| {
            format!("<c:barDir val=\"{direction}\"/><c:grouping val=\"{grouping}\"/>")
        };
        let (element, head) = match kind {
            ChartKind::Column => ("c:barChart", column("clustered", "col")),
            ChartKind::StackedColumn => ("c:barChart", column("stacked", "col")),
            ChartKind::Bar => ("c:barChart", column("clustered", "bar")),
            ChartKind::StackedBar => ("c:barChart", column("stacked", "bar")),
            ChartKind::Line => ("c:lineChart", "<c:grouping val=\"standard\"/>".to_string()),
            ChartKind::Area => ("c:areaChart", "<c:grouping val=\"standard\"/>".to_string()),
            ChartKind::Pie => ("c:pieChart", String::new()),
            ChartKind::Doughnut => ("c:doughnutChart", String::new()),
            ChartKind::Scatter => (
                "c:scatterChart",
                "<c:scatterStyle val=\"lineMarker\"/>".to_string(),
            ),
        };
        // Slices of a pie each get a colour; elsewhere a series has one.
        w!(
            x,
            "<{element}>{head}<c:varyColors val=\"{}\"/>",
            kind.is_round() as u8
        );
        for (i, s) in chart.series.iter().enumerate() {
            self.push_series(&mut x, kind, i, s);
        }
        if chart.data_labels {
            x.push_str("<c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/>");
            if kind.is_round() {
                x.push_str("<c:showLeaderLines val=\"1\"/>");
            }
            x.push_str("</c:dLbls>");
        }
        // Axis ids only have to differ inside a chart; like other writers,
        // keep them apart across the charts of a package as well.
        let base = 50_000_000 + (number as u64 % 1_000_000) * 10;
        let (first, second) = (base + 1, base + 2);
        let axis_ids = format!("<c:axId val=\"{first}\"/><c:axId val=\"{second}\"/>");
        match kind {
            ChartKind::Column | ChartKind::Bar => {
                w!(
                    x,
                    "<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/>{axis_ids}"
                );
            }
            ChartKind::StackedColumn | ChartKind::StackedBar => {
                w!(
                    x,
                    "<c:gapWidth val=\"150\"/><c:overlap val=\"100\"/>{axis_ids}"
                );
            }
            ChartKind::Line => {
                w!(x, "<c:marker val=\"1\"/>{axis_ids}");
            }
            ChartKind::Area | ChartKind::Scatter => x.push_str(&axis_ids),
            ChartKind::Pie => x.push_str("<c:firstSliceAng val=\"0\"/>"),
            ChartKind::Doughnut => {
                x.push_str("<c:firstSliceAng val=\"0\"/><c:holeSize val=\"75\"/>")
            }
        }
        w!(x, "</{element}>");

        let category = AxisKind::Category;
        match kind {
            ChartKind::Pie | ChartKind::Doughnut => {}
            ChartKind::Scatter => {
                push_axis(&mut x, AxisKind::Xy, (first, second), 'b');
                push_axis(&mut x, AxisKind::Xy, (second, first), 'l');
            }
            ChartKind::Bar | ChartKind::StackedBar => {
                push_axis(&mut x, category, (first, second), 'l');
                push_axis(
                    &mut x,
                    AxisKind::Value { between: true },
                    (second, first),
                    'b',
                );
            }
            ChartKind::Area => {
                push_axis(&mut x, category, (first, second), 'b');
                push_axis(
                    &mut x,
                    AxisKind::Value { between: false },
                    (second, first),
                    'l',
                );
            }
            ChartKind::Column | ChartKind::StackedColumn | ChartKind::Line => {
                push_axis(&mut x, category, (first, second), 'b');
                push_axis(
                    &mut x,
                    AxisKind::Value { between: true },
                    (second, first),
                    'l',
                );
            }
        }
        x.push_str("</c:plotArea>");

        let legend = match chart.legend {
            LegendPos::None => None,
            LegendPos::Bottom => Some('b'),
            LegendPos::Right => Some('r'),
            LegendPos::Top => Some('t'),
            LegendPos::Left => Some('l'),
        };
        if let Some(side) = legend {
            w!(
                x,
                "<c:legend><c:legendPos val=\"{side}\"/><c:overlay val=\"0\"/></c:legend>"
            );
        }
        x.push_str("<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart>");
        w!(
            x,
            "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr><a:solidFill><a:srgbClr val=\"{TEXT_GREY}\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>"
        );
        x.push_str("</c:chartSpace>");
        x
    }
}
