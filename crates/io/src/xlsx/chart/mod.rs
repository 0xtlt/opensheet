//! Charts: the drawing part of a sheet (where each chart sits) and the chart
//! parts (what each one shows and how).
//!
//! # What is written and read
//!
//! Every field of the engine's model has its place in a chart part, and
//! what is written is read back as the same model:
//!
//! * the type of the chart and of each series: one chart-type element
//!   (`c:barChart`, `c:lineChart`, ...) per group of series that share a
//!   [`Plot`] and a pair of axes. Types that differ only by the look of
//!   their series (lines with or without markers, the five scatter types)
//!   share an element; which one a chart is, is read from the look of its
//!   first series;
//! * per series: names and ranges, colour, line or outline, marker,
//!   smoothing, single points (`c:dPt`), explosion, data labels,
//!   trendlines, error bars, bubble sizes;
//! * per chart: title, legend (with deleted entries), data labels, the two
//!   pairs of axes with all their options, gaps and overlap of bars, first
//!   slice and hole of pies and doughnuts, bubble scale, the 3-D camera,
//!   chart and plot area, data table, the treatment of blanks.
//!
//! An option left automatic in the model is written the way Excel spells
//! it in its own new charts, and that spelling is read back as automatic.
//! Automatic colours are spelled out too (the colours of the engine's
//! palette, per series or per point): Excel would take them from a theme,
//! and packages are written without one. What a chart has no use for (the
//! hole of a doughnut in a column chart, an overlap without bars, a
//! legend entry without a legend) is not written, and what Excel refuses
//! in a file (a label position its chart type does not have, trendlines
//! on stacked series, secondary axes in 3-D) is left out.
//!
//! # Charts of other applications
//!
//! The model is a subset of the format. A chart read from a file keeps the
//! text of its part ([`ForeignChart`]), and as long as nothing but its
//! place changes, [`foreign::reusable`] lets the writer put that text back
//! as it was, with everything the model has no place for. Its theme
//! colours alone are replaced by the colours they stand for, when the
//! theme of the workbook is not the one Excel assumes for a package
//! without a theme ([`foreign::without_theme`]). The text is not written
//! back when it
//!
//! * is not a well-formed chart part in UTF-8,
//! * names a sheet the workbook no longer has, or reads cells in a way the
//!   engine cannot check (names, other workbooks),
//! * belongs to a pivot table (`c:pivotSource`), or
//! * refers to other parts of its package (pictures used as fills, for
//!   instance), which are not carried over. Two such references are
//!   simply cut out, as they add nothing to the picture: `c:externalData`
//!   (the workbook embedded in a chart of a document) and `c:userShapes`
//!   (drawings on top of the chart).
//!
//! Then, and whenever the chart was changed, the part is generated from
//! the model, and what the model does not hold is lost:
//!
//! * fonts, sizes and colours of text; the formatting of titles, legend,
//!   axis lines, gridlines, data labels, error bars, walls and floor;
//!   manual layouts (positions of the plot area, of titles, of labels);
//! * gradients, patterns and pictures (a gradient or a pattern becomes its
//!   first colour), transparency, shadows and other effects, the absence
//!   of a fill on a series, hollow markers, the colours a theme gives
//!   what the file leaves automatic;
//! * data labels and markers of single points, labels read from cells,
//!   custom error amounts read from ranges, the label of a trendline;
//! * series whose values are not a range of this workbook (numbers typed
//!   into the chart, names, other workbooks), multi-level categories but
//!   for the innermost level, titles linked to a cell (their last text is
//!   kept), axis titles without a text;
//! * stock, surface and pie-of-pie charts (read as lines and as a pie),
//!   the shapes of 3-D bars, drop lines, high-low lines and up-down bars,
//!   series lines, the depth axis of 3-D charts, line, marker and
//!   smoothing of lines in 3-D;
//! * differences between groups of the same chart that the model keeps
//!   once (the gap of bars, "vary colours", the first slice), a third
//!   pair of axes, date axes (written as automatic category axes), chart
//!   style parts and extension lists.
//!
//! [`ForeignChart`]: opensheet_engine::chart::ForeignChart

pub(crate) mod foreign;
pub(crate) mod read;
pub(crate) mod write;

use opensheet_engine::ast::{CellRef, RefExpr, RefKind};
use opensheet_engine::chart::{
    ChartKind, Dash, DataLabels, DisplayUnit, Family, Grouping, LabelPosition, MarkerShape,
    TickLabels, TickMark,
};
use opensheet_engine::{Color, Locale};
use std::collections::{BTreeMap, BTreeSet};

/// Line widths are in English Metric Units too: 12 700 to the point.
const EMU_PER_PT: f64 = 12_700.0;

/// Automatic width, in points, of the outline of a shape and of the frame
/// of the chart area.
const OUTLINE_WIDTH: f32 = 0.75;
/// Automatic width of the white lines between the slices of a pie.
const SLICE_OUTLINE_WIDTH: f32 = 1.5;
/// Automatic width of a trendline.
const TREND_WIDTH: f32 = 1.5;
/// Size of the markers Excel picks by itself.
const LEGACY_MARKER_SIZE: u8 = 7;
/// Automatic frames, gridlines and axis lines: the light grey of the
/// charts Excel makes.
const FRAME_GREY: Color = Color(0xD9D9D9);

/// The markers Excel gives the series of a chart that leaves them to it,
/// by position of the series.
const LEGACY_MARKERS: [MarkerShape; 9] = [
    MarkerShape::Diamond,
    MarkerShape::Square,
    MarkerShape::Triangle,
    MarkerShape::X,
    MarkerShape::Star,
    MarkerShape::Circle,
    MarkerShape::Plus,
    MarkerShape::Dot,
    MarkerShape::Dash,
];

const DASHES: [(Dash, &str); 8] = [
    (Dash::Solid, "solid"),
    (Dash::RoundDot, "sysDot"),
    (Dash::SquareDot, "sysDash"),
    (Dash::Dash, "dash"),
    (Dash::DashDot, "dashDot"),
    (Dash::LongDash, "lgDash"),
    (Dash::LongDashDot, "lgDashDot"),
    (Dash::LongDashDotDot, "lgDashDotDot"),
];

const MARKERS: [(MarkerShape, &str); 10] = [
    (MarkerShape::None, "none"),
    (MarkerShape::Circle, "circle"),
    (MarkerShape::Square, "square"),
    (MarkerShape::Diamond, "diamond"),
    (MarkerShape::Triangle, "triangle"),
    (MarkerShape::X, "x"),
    (MarkerShape::Star, "star"),
    (MarkerShape::Dash, "dash"),
    (MarkerShape::Dot, "dot"),
    (MarkerShape::Plus, "plus"),
];

const LABEL_POSITIONS: [(LabelPosition, &str); 9] = [
    (LabelPosition::Center, "ctr"),
    (LabelPosition::InsideEnd, "inEnd"),
    (LabelPosition::InsideBase, "inBase"),
    (LabelPosition::OutsideEnd, "outEnd"),
    (LabelPosition::BestFit, "bestFit"),
    (LabelPosition::Above, "t"),
    (LabelPosition::Below, "b"),
    (LabelPosition::Left, "l"),
    (LabelPosition::Right, "r"),
];

const TICK_MARKS: [(TickMark, &str); 4] = [
    (TickMark::None, "none"),
    (TickMark::Inside, "in"),
    (TickMark::Outside, "out"),
    (TickMark::Cross, "cross"),
];

const TICK_LABELS: [(TickLabels, &str); 4] = [
    (TickLabels::NextToAxis, "nextTo"),
    (TickLabels::High, "high"),
    (TickLabels::Low, "low"),
    (TickLabels::None, "none"),
];

const DISPLAY_UNITS: [(DisplayUnit, &str); 9] = [
    (DisplayUnit::Hundreds, "hundreds"),
    (DisplayUnit::Thousands, "thousands"),
    (DisplayUnit::TenThousands, "tenThousands"),
    (DisplayUnit::HundredThousands, "hundredThousands"),
    (DisplayUnit::Millions, "millions"),
    (DisplayUnit::TenMillions, "tenMillions"),
    (DisplayUnit::HundredMillions, "hundredMillions"),
    (DisplayUnit::Billions, "billions"),
    (DisplayUnit::Trillions, "trillions"),
];

/// Markers that are strokes without an inside. Excel paints the fill of
/// such a marker as a square behind it: they are written without one, and
/// their colour is that of their strokes.
fn is_stroked(shape: MarkerShape) -> bool {
    matches!(
        shape,
        MarkerShape::X | MarkerShape::Star | MarkerShape::Plus
    )
}

/// What stands between the parts of a data label when the model leaves it
/// automatic: the list separator of the locale, and a space. Excel left to
/// itself would put the parts on lines of their own.
fn list_separator(locale: &Locale) -> String {
    format!("{} ", locale.arg_sep)
}

/// The name a file gives a value of the model.
fn name_of<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> Option<&'static str> {
    table
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, name)| *name)
}

/// The value of the model a file names.
fn named<T: Copy>(table: &[(T, &'static str)], name: &str) -> Option<T> {
    table.iter().find(|(_, n)| *n == name).map(|(v, _)| *v)
}

/// Position of a theme colour in a [`Palette`](super::color::Palette).
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

/// What a chart-type element of a file draws: the element itself, the
/// direction of its bars, its grouping. Kinds that share a plot differ by
/// the look of their series only, and their series can sit in one element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plot {
    pub family: Family,
    pub grouping: Grouping,
    pub is_3d: bool,
    /// Radar charts: the series are filled.
    pub filled: bool,
}

impl Plot {
    pub fn of(kind: ChartKind) -> Plot {
        Plot {
            family: kind.family(),
            grouping: kind.grouping(),
            is_3d: kind.is_3d(),
            filled: kind == ChartKind::RadarFilled,
        }
    }

    /// Name of the element, without its prefix.
    pub fn element(self) -> &'static str {
        match (self.family, self.is_3d) {
            (Family::Column | Family::Bar, false) => "barChart",
            (Family::Column | Family::Bar, true) => "bar3DChart",
            (Family::Line, false) => "lineChart",
            (Family::Line, true) => "line3DChart",
            (Family::Area, false) => "areaChart",
            (Family::Area, true) => "area3DChart",
            (Family::Pie, false) => "pieChart",
            (Family::Pie, true) => "pie3DChart",
            (Family::Doughnut, _) => "doughnutChart",
            (Family::Scatter, _) => "scatterChart",
            (Family::Bubble, _) => "bubbleChart",
            (Family::Radar, _) => "radarChart",
        }
    }

    /// The kind of a series of this plot that has markers, a line between
    /// its points and a curved line, or not.
    pub fn kind(self, markers: bool, line: bool, smooth: bool) -> ChartKind {
        use ChartKind::*;
        use Grouping::*;
        match (self.family, self.is_3d) {
            (Family::Column, false) => match self.grouping {
                Standard => Column,
                Stacked => StackedColumn,
                Percent => PercentStackedColumn,
            },
            (Family::Column, true) => match self.grouping {
                Standard => Column3D,
                Stacked => StackedColumn3D,
                Percent => PercentStackedColumn3D,
            },
            (Family::Bar, false) => match self.grouping {
                Standard => Bar,
                Stacked => StackedBar,
                Percent => PercentStackedBar,
            },
            (Family::Bar, true) => match self.grouping {
                Standard => Bar3D,
                Stacked => StackedBar3D,
                Percent => PercentStackedBar3D,
            },
            (Family::Line, true) => Line3D,
            (Family::Line, false) => match (self.grouping, markers) {
                (Standard, false) => Line,
                (Stacked, false) => StackedLine,
                (Percent, false) => PercentStackedLine,
                (Standard, true) => LineMarkers,
                (Stacked, true) => StackedLineMarkers,
                (Percent, true) => PercentStackedLineMarkers,
            },
            (Family::Area, false) => match self.grouping {
                Standard => Area,
                Stacked => StackedArea,
                Percent => PercentStackedArea,
            },
            (Family::Area, true) => match self.grouping {
                Standard => Area3D,
                Stacked => StackedArea3D,
                Percent => PercentStackedArea3D,
            },
            (Family::Pie, false) => Pie,
            (Family::Pie, true) => Pie3D,
            (Family::Doughnut, _) => Doughnut,
            (Family::Scatter, _) => match (line, smooth, markers) {
                (false, ..) => Scatter,
                (true, true, true) => ScatterSmoothMarkers,
                (true, true, false) => ScatterSmooth,
                (true, false, true) => ScatterLinesMarkers,
                (true, false, false) => ScatterLines,
            },
            (Family::Bubble, _) => Bubble,
            (Family::Radar, _) if self.filled => RadarFilled,
            (Family::Radar, _) if markers => RadarMarkers,
            (Family::Radar, _) => Radar,
        }
    }

    /// Series drawn as a line through markers: their colour is that of a
    /// stroke, and they have a `c:marker`.
    pub fn is_line_like(self) -> bool {
        match self.family {
            Family::Line => !self.is_3d,
            Family::Scatter => true,
            Family::Radar => !self.filled,
            _ => false,
        }
    }

    pub fn is_round(self) -> bool {
        matches!(self.family, Family::Pie | Family::Doughnut)
    }

    pub fn is_xy(self) -> bool {
        matches!(self.family, Family::Scatter | Family::Bubble)
    }

    /// Whether the series say if their line is a curve (`c:smooth`).
    pub fn has_smoothing(self) -> bool {
        matches!(self.family, Family::Line | Family::Scatter)
    }

    /// Where Excel accepts the data labels of such a series; anywhere else
    /// it takes the whole file for damaged.
    pub fn label_positions(self) -> &'static [LabelPosition] {
        use LabelPosition::*;
        match self.family {
            Family::Pie => &[BestFit, OutsideEnd, InsideEnd, Center],
            _ if self.is_3d => &[],
            Family::Column | Family::Bar if self.grouping == Grouping::Standard => {
                &[OutsideEnd, InsideEnd, InsideBase, Center]
            }
            Family::Column | Family::Bar => &[InsideEnd, InsideBase, Center],
            Family::Line | Family::Scatter | Family::Bubble => &[Above, Below, Left, Right, Center],
            Family::Area | Family::Doughnut | Family::Radar => &[],
        }
    }

    /// The labels as a file can hold them for a series of this plot: a
    /// position Excel refuses there is left to it, and only pies have
    /// leader lines to speak of.
    pub fn labels(self, labels: &DataLabels) -> DataLabels {
        let position = if self.label_positions().contains(&labels.position) {
            labels.position
        } else {
            LabelPosition::Auto
        };
        DataLabels {
            position,
            leader_lines: labels.leader_lines || !self.is_round(),
            ..labels.clone()
        }
    }

    /// Excel has no error bars in pies, radar charts and 3-D charts.
    pub fn takes_error_bars(self) -> bool {
        !self.is_3d && !self.is_round() && self.family != Family::Radar
    }

    /// Nor trendlines, which stacked series do not have either.
    pub fn takes_trendlines(self) -> bool {
        self.takes_error_bars() && self.grouping == Grouping::Standard
    }

    /// Whether a chart of such series can have a data table under it.
    pub fn takes_data_table(self) -> bool {
        matches!(
            self.family,
            Family::Column | Family::Bar | Family::Line | Family::Area
        )
    }
}

/// Drawing coordinates are in English Metric Units: 914 400 to the inch,
/// hence 9 525 to the pixel at 96 dpi.
const EMU_PER_PX: f64 = 9525.0;

/// Whether a reference covers several rows and several columns.
fn is_block(r: &RefExpr) -> bool {
    let area = r.rect();
    area.rows() > 1 && area.cols() > 1
}

/// One column (or one row) of the cells a reference covers: the first or
/// the last one.
fn line_of(r: &RefExpr, column: bool, last: bool) -> RefExpr {
    let area = r.rect();
    // Whole columns say nothing of rows, whole rows nothing of columns.
    let (abs_row, abs_col) = match r.kind {
        RefKind::Cells => (r.start.abs_row, r.start.abs_col),
        RefKind::Cols => (r.start.abs_col, r.start.abs_col),
        RefKind::Rows => (r.start.abs_row, r.start.abs_row),
    };
    let corner = |row: u32, col: u32| CellRef {
        row,
        col,
        abs_row,
        abs_col,
    };
    let (start, end) = if column {
        let col = if last { area.c1 } else { area.c0 };
        (corner(area.r0, col), corner(area.r1, col))
    } else {
        let row = if last { area.r1 } else { area.r0 };
        (corner(row, area.c0), corner(row, area.c1))
    };
    // A column of whole columns is a whole column, a row of whole rows a
    // whole row; anything else is plain cells.
    let kind = match (r.kind, column) {
        (RefKind::Cols, true) => RefKind::Cols,
        (RefKind::Rows, false) => RefKind::Rows,
        _ => RefKind::Cells,
    };
    RefExpr {
        sheet: r.sheet.clone(),
        start,
        end: (kind != RefKind::Cells || start != end).then_some(end),
        kind,
    }
}

/// The values of a series, as a file can hold them. Excel takes them from
/// a single row or column and stops with an error message on a file where
/// they come from a block. Of a block, this is the first column, or the
/// first row when it is wider than tall: the way the engine itself lays
/// series out in a table.
pub(crate) fn series_line(values: &RefExpr) -> RefExpr {
    if !is_block(values) {
        return values.clone();
    }
    let area = values.rect();
    line_of(values, area.rows() >= area.cols(), false)
}

/// The cell a series is named after: the engine reads the name in the
/// first cell of the range, and a block of cells is no name to Excel.
pub(crate) fn name_cell(name: &RefExpr) -> RefExpr {
    if !is_block(name) {
        return name.clone();
    }
    let area = name.rect();
    let across = line_of(name, false, false);
    RefExpr {
        start: CellRef {
            row: area.r0,
            col: area.c0,
            ..across.start
        },
        end: None,
        kind: RefKind::Cells,
        ..across
    }
}

/// Category labels in several columns (year, then quarter) are a list of
/// plain labels to the engine: this is the level next to the plot.
/// `values` tells which way the series runs.
pub(crate) fn innermost_level(categories: &RefExpr, values: &RefExpr) -> RefExpr {
    if !is_block(categories) {
        return categories.clone();
    }
    let (area, plotted) = (categories.rect(), values.rect());
    let in_columns = plotted.cols() == 1 || (plotted.rows() > 1 && area.rows() >= area.cols());
    line_of(categories, in_columns, true)
}

/// The widths of the columns (or the heights of the rows) of a sheet, in
/// pixels at 100 %. Anchors in a drawing are a cell plus an offset inside
/// it; this converts between them and distances in pixels.
pub(crate) struct Track<'a> {
    /// Size of the cells that have no size of their own.
    pub default: f32,
    pub sizes: &'a BTreeMap<u32, f32>,
    pub hidden: &'a BTreeSet<u32>,
    /// Largest size a cell can have.
    pub max: f32,
    /// Number of cells: the columns or rows of a sheet.
    pub count: u32,
}

impl Track<'_> {
    fn clamped(&self, px: f32) -> f64 {
        if px.is_finite() {
            px.clamp(0.0, self.max.max(0.0)) as f64
        } else {
            0.0
        }
    }

    fn size(&self, i: u32) -> f64 {
        if self.hidden.contains(&i) {
            return 0.0;
        }
        self.clamped(self.sizes.get(&i).copied().unwrap_or(self.default))
    }

    /// Pixels from the leading edge of cell `a` to that of cell `b`
    /// (negative when `b` comes first).
    pub fn span(&self, a: u32, b: u32) -> f64 {
        if b < a {
            return -self.span(b, a);
        }
        let default = self.clamped(self.default);
        let mut total = (b - a) as f64 * default;
        for (i, px) in self.sizes.range(a..b) {
            if !self.hidden.contains(i) {
                total += self.clamped(*px) - default;
            }
        }
        total - self.hidden.range(a..b).count() as f64 * default
    }

    /// The cell holding the point `offset` pixels past the leading edge of
    /// cell `start`, and how far into that cell the point lies.
    pub fn locate(&self, start: u32, offset: f64) -> (u32, f64) {
        let last = self.count.saturating_sub(1);
        let default = self.clamped(self.default);
        let mut i = start.min(last);
        let mut rest = if offset.is_finite() {
            offset.max(0.0)
        } else {
            0.0
        };
        while i < last && rest > 0.0 {
            // The cells before the next one with a size of its own all have
            // the default size: step over them at once.
            let special = [
                self.sizes.range(i..).next().map(|(k, _)| *k),
                self.hidden.range(i..).next().copied(),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(last, |k| k.min(last));
            if special > i {
                let run = (special - i) as f64;
                if default > 0.0 && rest < run * default {
                    let whole = (rest / default).floor().min(run - 1.0);
                    return (i + whole as u32, (rest - whole * default).max(0.0));
                }
                rest -= run * default;
                i = special;
            } else {
                let size = self.size(i);
                if rest < size {
                    break;
                }
                rest -= size;
                i += 1;
            }
        }
        if i == last {
            // Nothing lies beyond the last cell.
            rest = rest.min(self.size(last));
        }
        (i, rest.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_positions() {
        let sizes = BTreeMap::from([(2, 100.0f32), (5, 10.0), (6, 30.0)]);
        let hidden = BTreeSet::from([3u32, 6]);
        let t = Track {
            default: 64.0,
            sizes: &sizes,
            hidden: &hidden,
            max: 1000.0,
            count: 100,
        };
        // Widths: 64 64 100 0 64 10 0 64 64 ...
        assert_eq!(t.span(0, 0), 0.0);
        assert_eq!(t.span(0, 2), 128.0);
        assert_eq!(t.span(0, 3), 228.0);
        assert_eq!(t.span(0, 4), 228.0);
        assert_eq!(t.span(2, 8), 100.0 + 64.0 + 10.0 + 64.0);
        assert_eq!(t.span(8, 2), -238.0);
        assert_eq!(t.span(10, 20), 640.0);

        assert_eq!(t.locate(0, 0.0), (0, 0.0));
        assert_eq!(t.locate(0, 63.5), (0, 63.5));
        assert_eq!(t.locate(0, 64.0), (1, 0.0));
        assert_eq!(t.locate(0, 130.0), (2, 2.0));
        // The far edge of a cell is the start of the next one; hidden cells
        // are stepped over as soon as there is something left to cover.
        assert_eq!(t.locate(2, 100.0), (3, 0.0));
        assert_eq!(t.locate(2, 101.0), (4, 1.0));
        assert_eq!(t.locate(0, 228.0 + 64.0 + 10.0 + 5.0), (7, 5.0));
        // A position is never moved off the cell it starts in.
        assert_eq!(t.locate(3, 0.0), (3, 0.0));
        assert_eq!(t.locate(20, 64.0 * 30.0 + 1.0), (50, 1.0));
        // The sheet ends somewhere.
        assert_eq!(t.locate(98, 1e12), (99, 64.0));
        assert_eq!(t.locate(5000, 3.0), (99, 3.0));
        assert_eq!(t.locate(0, f64::NAN), (0, 0.0));
        assert_eq!(t.locate(0, -5.0), (0, 0.0));
        // Every position is where `span` says it is.
        for start in [0u32, 2, 3, 6, 40] {
            for offset in [0.5, 10.0, 99.0, 250.0, 1234.5] {
                let (cell, inside) = t.locate(start, offset);
                assert_eq!(t.span(start, cell) + inside, offset, "{start} + {offset}");
            }
        }
    }

    #[test]
    fn blocks_are_reduced_to_lines() {
        use opensheet_engine::Locale;
        use opensheet_engine::ast::Expr;
        use opensheet_engine::parser::parse_formula;
        let range = |text: &str| match parse_formula(text, Locale::en()) {
            Ok(Expr::Ref(r)) => r,
            other => panic!("{text}: {other:?}"),
        };
        let values = |text: &str| series_line(&range(text)).to_text();
        // Rows, columns and cells are what they are.
        for text in ["S!$B$2:$B$9", "S!$B$2:$F$2", "S!$C$3", "S!$B:$B", "S!$4:$4"] {
            assert_eq!(values(text), text);
            assert_eq!(name_cell(&range(text)).to_text(), text);
            let labels = innermost_level(&range(text), &range("S!$A$1:$A$9"));
            assert_eq!(labels.to_text(), text);
        }
        // Blocks: the first column when tall or square, else the first row.
        assert_eq!(values("S!$B$2:$C$9"), "S!$B$2:$B$9");
        assert_eq!(values("S!$B$2:$C$3"), "S!$B$2:$B$3");
        assert_eq!(values("S!$B$2:$F$3"), "S!$B$2:$F$2");
        assert_eq!(values("S!$C$9:$B$2"), "S!$B$2:$B$9");
        assert_eq!(values("'a b'!B2:C9"), "'a b'!B2:B9");
        assert_eq!(values("S!$B:$D"), "S!$B:$B");
        assert_eq!(values("S!$2:$5"), "S!$2:$2");
        assert_eq!(values("S!$A:$XFD"), "S!$A:$A");
        assert_eq!(values("S!$1:$1048576"), "S!$A$1:$A$1048576");
        assert_eq!(values("S!$A$1:$XFD$1048576"), "S!$A$1:$A$1048576");
        // Names: the first cell.
        assert_eq!(name_cell(&range("S!$B$2:$C$9")).to_text(), "S!$B$2");
        assert_eq!(name_cell(&range("S!$C$9:$B$2")).to_text(), "S!$B$2");
        assert_eq!(name_cell(&range("S!$B:$D")).to_text(), "S!$B$1");
        // Categories: the column next to values in a column, the row next
        // to values in a row.
        let labels =
            |text: &str, values: &str| innermost_level(&range(text), &range(values)).to_text();
        assert_eq!(labels("S!$A$2:$B$9", "S!$C$2:$C$9"), "S!$B$2:$B$9");
        assert_eq!(labels("S!$B$1:$F$2", "S!$B$3:$F$3"), "S!$B$2:$F$2");
        assert_eq!(labels("S!$A$2:$B$3", "S!$C$2:$C$3"), "S!$B$2:$B$3");
        assert_eq!(labels("S!$A$2:$B$3", "S!$C$4:$D$4"), "S!$A$3:$B$3");
        assert_eq!(labels("S!$A$2:$C$3", "S!$D$2"), "S!$C$2:$C$3");
        assert_eq!(labels("S!$A:$B", "S!$C:$C"), "S!$B:$B");
        assert_eq!(labels("S!$1:$2", "S!$3:$3"), "S!$2:$2");
    }

    #[test]
    fn degenerate_tracks() {
        let none = BTreeMap::new();
        let hidden = BTreeSet::new();
        let t = Track {
            default: 0.0,
            sizes: &none,
            hidden: &hidden,
            max: f32::INFINITY,
            count: 1_048_576,
        };
        assert_eq!(t.locate(0, 10.0), (1_048_575, 0.0));
        assert_eq!(t.span(0, 1_000_000), 0.0);
        let t = Track { count: 0, ..t };
        assert_eq!(t.locate(7, 10.0), (0, 0.0));
        let t = Track {
            default: f32::NAN,
            count: 10,
            ..t
        };
        assert_eq!(t.locate(0, 10.0), (9, 0.0));
    }
}
