//! Charts: the model stored in a sheet, building a chart from a selection,
//! and resolving the data a chart plots. `chart_layout` turns a chart and
//! its data into vector primitives; painting them is the application's
//! business.
//!
//! The model follows Excel's: a chart has a type, every series may have a
//! type of its own (combination charts) and sit on the secondary axes, and
//! every option left alone is "automatic" (`None`, or the `Default` of its
//! type), which means "what Excel does for a chart of this type".

use crate::ast::{CellRef, Expr, RefExpr, RefKind, StructuralEdit};
use crate::format;
use crate::refs::Rect;
use crate::style::Color;
use crate::value::Value;
use crate::workbook::Workbook;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// Vertical bars, one cluster per category.
    Column,
    StackedColumn,
    /// Stacked, every category scaled to 100 %.
    PercentStackedColumn,
    Column3D,
    StackedColumn3D,
    PercentStackedColumn3D,
    Line,
    StackedLine,
    PercentStackedLine,
    /// A line with a marker on every point.
    LineMarkers,
    StackedLineMarkers,
    PercentStackedLineMarkers,
    /// Ribbons, one behind the other.
    Line3D,
    Pie,
    Pie3D,
    Doughnut,
    /// Horizontal bars.
    Bar,
    StackedBar,
    PercentStackedBar,
    Bar3D,
    StackedBar3D,
    PercentStackedBar3D,
    Area,
    StackedArea,
    PercentStackedArea,
    Area3D,
    StackedArea3D,
    PercentStackedArea3D,
    /// X/Y points, markers only: the categories of a series are its X values.
    Scatter,
    ScatterSmoothMarkers,
    ScatterSmooth,
    /// Straight lines from point to point, with markers.
    ScatterLinesMarkers,
    ScatterLines,
    /// X/Y points drawn as discs whose area follows a third range.
    Bubble,
    Radar,
    RadarMarkers,
    RadarFilled,
}

/// What a kind draws, whatever its stacking and its depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// Vertical bars.
    Column,
    /// Horizontal bars.
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    Bubble,
    Radar,
}

/// How the series of one type share a category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grouping {
    /// Side by side (bars) or each on its own (lines, areas).
    Standard,
    Stacked,
    /// Stacked and scaled so that every category adds up to 100 %.
    Percent,
}

impl ChartKind {
    /// Every kind, in the order of Excel's "Change Chart Type" dialog.
    pub const ALL: [ChartKind; 37] = [
        ChartKind::Column,
        ChartKind::StackedColumn,
        ChartKind::PercentStackedColumn,
        ChartKind::Column3D,
        ChartKind::StackedColumn3D,
        ChartKind::PercentStackedColumn3D,
        ChartKind::Line,
        ChartKind::StackedLine,
        ChartKind::PercentStackedLine,
        ChartKind::LineMarkers,
        ChartKind::StackedLineMarkers,
        ChartKind::PercentStackedLineMarkers,
        ChartKind::Line3D,
        ChartKind::Pie,
        ChartKind::Pie3D,
        ChartKind::Doughnut,
        ChartKind::Bar,
        ChartKind::StackedBar,
        ChartKind::PercentStackedBar,
        ChartKind::Bar3D,
        ChartKind::StackedBar3D,
        ChartKind::PercentStackedBar3D,
        ChartKind::Area,
        ChartKind::StackedArea,
        ChartKind::PercentStackedArea,
        ChartKind::Area3D,
        ChartKind::StackedArea3D,
        ChartKind::PercentStackedArea3D,
        ChartKind::Scatter,
        ChartKind::ScatterSmoothMarkers,
        ChartKind::ScatterSmooth,
        ChartKind::ScatterLinesMarkers,
        ChartKind::ScatterLines,
        ChartKind::Bubble,
        ChartKind::Radar,
        ChartKind::RadarMarkers,
        ChartKind::RadarFilled,
    ];

    pub fn family(self) -> Family {
        use ChartKind::*;
        match self {
            Column | StackedColumn | PercentStackedColumn | Column3D | StackedColumn3D | PercentStackedColumn3D => {
                Family::Column
            }
            Bar | StackedBar | PercentStackedBar | Bar3D | StackedBar3D | PercentStackedBar3D => Family::Bar,
            Line | StackedLine | PercentStackedLine | LineMarkers | StackedLineMarkers | PercentStackedLineMarkers
            | Line3D => Family::Line,
            Area | StackedArea | PercentStackedArea | Area3D | StackedArea3D | PercentStackedArea3D => Family::Area,
            Pie | Pie3D => Family::Pie,
            Doughnut => Family::Doughnut,
            Scatter | ScatterSmoothMarkers | ScatterSmooth | ScatterLinesMarkers | ScatterLines => Family::Scatter,
            Bubble => Family::Bubble,
            Radar | RadarMarkers | RadarFilled => Family::Radar,
        }
    }

    pub fn grouping(self) -> Grouping {
        use ChartKind::*;
        match self {
            StackedColumn | StackedColumn3D | StackedBar | StackedBar3D | StackedLine | StackedLineMarkers
            | StackedArea | StackedArea3D => Grouping::Stacked,
            PercentStackedColumn | PercentStackedColumn3D | PercentStackedBar | PercentStackedBar3D
            | PercentStackedLine | PercentStackedLineMarkers | PercentStackedArea | PercentStackedArea3D => {
                Grouping::Percent
            }
            _ => Grouping::Standard,
        }
    }

    pub fn is_3d(self) -> bool {
        use ChartKind::*;
        matches!(
            self,
            Column3D
                | StackedColumn3D
                | PercentStackedColumn3D
                | Bar3D
                | StackedBar3D
                | PercentStackedBar3D
                | Line3D
                | Area3D
                | StackedArea3D
                | PercentStackedArea3D
                | Pie3D
        )
    }

    /// One series shown as shares of a whole.
    pub fn is_round(self) -> bool {
        matches!(self.family(), Family::Pie | Family::Doughnut)
    }

    /// Stacked, to 100 % or not.
    pub fn is_stacked(self) -> bool {
        self.grouping() != Grouping::Standard
    }

    pub fn is_percent(self) -> bool {
        self.grouping() == Grouping::Percent
    }

    /// Categories run along the vertical axis.
    pub fn is_horizontal(self) -> bool {
        self.family() == Family::Bar
    }

    /// Both axes measure numbers: the categories of a series are X values.
    pub fn is_xy(self) -> bool {
        matches!(self.family(), Family::Scatter | Family::Bubble)
    }

    /// Whether a series of this kind has a marker on every point when it
    /// says nothing about markers itself.
    pub fn has_markers(self) -> bool {
        use ChartKind::*;
        matches!(
            self,
            LineMarkers
                | StackedLineMarkers
                | PercentStackedLineMarkers
                | Scatter
                | ScatterSmoothMarkers
                | ScatterLinesMarkers
                | RadarMarkers
        )
    }

    /// Whether a series of this kind joins its points with a line when it
    /// says nothing about its line itself.
    pub fn has_lines(self) -> bool {
        match self.family() {
            Family::Line | Family::Radar => true,
            Family::Scatter => self != ChartKind::Scatter,
            _ => false,
        }
    }

    /// Whether the line of a series of this kind is a curve by default.
    pub fn is_smooth(self) -> bool {
        matches!(self, ChartKind::ScatterSmooth | ChartKind::ScatterSmoothMarkers)
    }

    /// Automatic width of the line of a series of this kind, in points.
    pub fn default_line_width(self) -> f32 {
        match self.family() {
            Family::Scatter => 1.5,
            _ => 2.25,
        }
    }

    /// Automatic gap between the clusters of bars, in percent of the width
    /// of a bar (Excel's defaults).
    pub fn default_gap_width(self) -> u32 {
        match (self.family(), self.grouping(), self.is_3d()) {
            (_, _, true) => 150,
            (_, Grouping::Stacked | Grouping::Percent, _) => 150,
            (Family::Bar, ..) => 182,
            _ => 219,
        }
    }

    /// Automatic overlap of the bars of a cluster, in percent of the width
    /// of a bar: negative leaves a gap, 100 puts them on top of each other.
    pub fn default_overlap(self) -> i32 {
        match (self.family(), self.grouping(), self.is_3d()) {
            (_, Grouping::Stacked | Grouping::Percent, _) => 100,
            (Family::Column, _, false) => -27,
            _ => 0,
        }
    }

    /// Whether a series of kind `other` can be drawn in a chart of this
    /// kind (a combination chart). Columns, lines and areas share a
    /// category axis; scatter variants mix; radar variants mix; horizontal
    /// bars only mix with horizontal bars. Pies, bubbles and 3-D charts
    /// stand alone.
    pub fn combines_with(self, other: ChartKind) -> bool {
        if self == other {
            return true;
        }
        if self.is_3d() || other.is_3d() {
            return false;
        }
        let upright = |f: Family| matches!(f, Family::Column | Family::Line | Family::Area);
        match (self.family(), other.family()) {
            (a, b) if upright(a) && upright(b) => true,
            (Family::Bar, Family::Bar) | (Family::Scatter, Family::Scatter) | (Family::Radar, Family::Radar) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SeriesName {
    None,
    Text(String),
    /// The name is read from a cell.
    Cell(RefExpr),
}

/// Excel's dash types (Format ▸ Line ▸ Dash type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Dash {
    #[default]
    Solid,
    RoundDot,
    SquareDot,
    Dash,
    DashDot,
    LongDash,
    LongDashDot,
    LongDashDotDot,
}

impl Dash {
    /// Lengths of the strokes and gaps, in units of the line width; empty
    /// for a solid line.
    pub fn pattern(self) -> &'static [f32] {
        match self {
            Dash::Solid => &[],
            Dash::RoundDot => &[1.0, 2.0],
            Dash::SquareDot => &[1.0, 1.0],
            Dash::Dash => &[4.0, 3.0],
            Dash::DashDot => &[4.0, 3.0, 1.0, 3.0],
            Dash::LongDash => &[8.0, 3.0],
            Dash::LongDashDot => &[8.0, 3.0, 1.0, 3.0],
            Dash::LongDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        }
    }
}

/// A stroke: the line of a series, the outline of a shape, a trendline.
/// The default is automatic in every respect.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Line {
    /// `Some(false)` = no line; `None` = automatic (what is being
    /// formatted decides: lines have a line, bars have no outline).
    pub shown: Option<bool>,
    /// `None` = automatic (the colour of the series, grey for frames).
    pub color: Option<Color>,
    /// Width in points; `None` = automatic.
    pub width: Option<f32>,
    pub dash: Dash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkerShape {
    None,
    Circle,
    Square,
    Diamond,
    Triangle,
    X,
    Star,
    /// A short horizontal stroke.
    Dash,
    /// A shorter one.
    Dot,
    Plus,
}

/// The marker drawn on the points of a series.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub shape: MarkerShape,
    /// Size in points, 2 to 72 (Excel's default is 5).
    pub size: u8,
    /// `None` = the colour of the series.
    pub fill: Option<Color>,
    /// Outline; `None` = the colour of the series.
    pub line: Option<Color>,
}

impl Marker {
    pub const DEFAULT_SIZE: u8 = 5;

    pub fn new(shape: MarkerShape) -> Marker {
        Marker {
            shape,
            size: Marker::DEFAULT_SIZE,
            fill: None,
            line: None,
        }
    }
}

/// Where a data label sits relative to its point. Not every position
/// exists for every chart type (bars have no "above"); one that does not
/// is drawn as `Auto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LabelPosition {
    /// What Excel picks for the type: outside end for clustered bars,
    /// centre for stacked ones, right of the point for lines and scatter
    /// plots, best fit for pies.
    #[default]
    Auto,
    Center,
    InsideEnd,
    InsideBase,
    OutsideEnd,
    /// Pies: inside the slice when the text fits, outside otherwise.
    BestFit,
    Above,
    Below,
    Left,
    Right,
}

/// What the data labels of a series show ("Format Data Labels").
#[derive(Clone, Debug, PartialEq)]
pub struct DataLabels {
    pub value: bool,
    pub category: bool,
    pub series_name: bool,
    /// Share of the point in its series (pies and doughnuts).
    pub percent: bool,
    /// A swatch of the series' colour before the text.
    pub legend_key: bool,
    pub bubble_size: bool,
    /// Text between the parts; `None` = automatic (", " in Excel, the
    /// list separator of the locale followed by a space).
    pub separator: Option<String>,
    pub position: LabelPosition,
    /// `None` = the number format of the cells.
    pub number_format: Option<String>,
    /// Pies: a line from an outside label to its slice.
    pub leader_lines: bool,
}

impl Default for DataLabels {
    /// The value alone, placed automatically.
    fn default() -> Self {
        DataLabels {
            value: true,
            category: false,
            series_name: false,
            percent: false,
            legend_key: false,
            bubble_size: false,
            separator: None,
            position: LabelPosition::Auto,
            number_format: None,
            leader_lines: true,
        }
    }
}

impl DataLabels {
    /// Whether the labels show anything at all.
    pub fn any(&self) -> bool {
        self.value || self.category || self.series_name || self.percent || self.bubble_size
    }

    /// Labels that show nothing: on a series, "no labels for this one".
    pub fn none() -> DataLabels {
        DataLabels {
            value: false,
            ..DataLabels::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrendKind {
    Linear,
    Exponential,
    Logarithmic,
    /// Order 2 to 6.
    Polynomial(u8),
    Power,
    /// Mean of the last `n` points (2 or more).
    MovingAverage(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trendline {
    pub kind: TrendKind,
    /// `None` = automatic: "Linear (Sales)".
    pub name: Option<String>,
    /// Periods (categories, or units of the X axis of a scatter chart) the
    /// line is continued after the last point and before the first one.
    pub forward: f64,
    pub backward: f64,
    /// "Set intercept": the value at x = 0 (linear, polynomial) or the
    /// factor (exponential). Ignored by the other kinds, as in Excel.
    pub intercept: Option<f64>,
    pub show_equation: bool,
    pub show_r2: bool,
    /// Automatic: the colour of the series, 1.5 pt, round dots.
    pub line: Line,
}

impl Trendline {
    pub fn new(kind: TrendKind) -> Trendline {
        Trendline {
            kind,
            name: None,
            forward: 0.0,
            backward: 0.0,
            intercept: None,
            show_equation: false,
            show_r2: false,
            line: Line::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ErrorAmount {
    /// The same amount on every point.
    Fixed(f64),
    /// Percent of the value of the point.
    Percent(f64),
    /// That many standard deviations of the series, around its mean.
    StdDev(f64),
    /// The standard error of the series.
    StdErr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorDirection {
    Both,
    Plus,
    Minus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorBars {
    /// Along the X axis (scatter and bubble charts only) instead of along
    /// the value axis.
    pub horizontal: bool,
    pub direction: ErrorDirection,
    pub amount: ErrorAmount,
    /// A short stroke across each end.
    pub end_cap: bool,
}

/// Formatting of one point of a series (a slice of a pie, a single bar).
#[derive(Clone, Debug, PartialEq)]
pub struct PointFormat {
    /// Position of the point in the series, from 0.
    pub index: u32,
    pub color: Option<Color>,
    /// Pie slices: distance from the centre, in percent of the radius.
    pub explosion: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: SeriesName,
    /// The plotted numbers.
    pub values: RefExpr,
    /// Category labels, or the X values of a scatter or bubble chart.
    pub categories: Option<RefExpr>,
    /// `None` = automatic colour from the palette. The colour of the
    /// series' ink: the fill of bars, areas, slices and markers, the stroke
    /// of lines.
    pub color: Option<Color>,
    /// Bubble charts: the sizes of the bubbles.
    pub bubble_sizes: Option<RefExpr>,
    /// A chart type of its own (combination charts); `None` = the type of
    /// the chart. A type the chart cannot show next to its own (see
    /// `ChartKind::combines_with`) is ignored.
    pub kind: Option<ChartKind>,
    /// Plotted against the secondary axes.
    pub secondary: bool,
    /// The line of a line, scatter or radar series; the outline of bars,
    /// areas and slices (automatic: none, white between the slices of a
    /// pie).
    pub line: Line,
    /// `None` = automatic: a circle of size 5 when the kind has markers,
    /// nothing otherwise.
    pub marker: Option<Marker>,
    /// Curve through the points instead of straight segments; `None` =
    /// what the kind does.
    pub smooth: Option<bool>,
    /// Formatting of single points; at most one entry per point.
    pub points: Vec<PointFormat>,
    /// Pies: every slice pulled out by this many percent of the radius.
    pub explosion: u32,
    /// Labels of its own; `None` = those of the chart (`Chart::data_labels`
    /// and `Chart::label_options`).
    pub labels: Option<DataLabels>,
    pub trendlines: Vec<Trendline>,
    pub error_bars: Vec<ErrorBars>,
}

impl Series {
    /// A series plotting `values`, automatic in every respect.
    pub fn new(values: RefExpr) -> Series {
        Series {
            name: SeriesName::None,
            values,
            categories: None,
            color: None,
            bubble_sizes: None,
            kind: None,
            secondary: false,
            line: Line::default(),
            marker: None,
            smooth: None,
            points: Vec::new(),
            explosion: 0,
            labels: None,
            trendlines: Vec::new(),
            error_bars: Vec::new(),
        }
    }

    /// The marker drawn when the series is of `kind`.
    pub fn marker_for(&self, kind: ChartKind) -> Marker {
        match &self.marker {
            Some(m) => m.clone(),
            None if kind.has_markers() => Marker::new(MarkerShape::Circle),
            None => Marker::new(MarkerShape::None),
        }
    }

    /// Whether the points are joined by a line when the series is of `kind`.
    pub fn line_shown(&self, kind: ChartKind) -> bool {
        self.line.shown.unwrap_or(kind.has_lines())
    }

    pub fn smooth_for(&self, kind: ChartKind) -> bool {
        self.smooth.unwrap_or(kind.is_smooth())
    }

    pub fn point(&self, index: usize) -> Option<&PointFormat> {
        self.points.iter().find(|p| p.index as usize == index)
    }

    /// The entry of a point in `points`, added when missing.
    pub fn point_mut(&mut self, index: usize) -> &mut PointFormat {
        let at = match self.points.iter().position(|p| p.index as usize == index) {
            Some(at) => at,
            None => {
                self.points.push(PointFormat {
                    index: index as u32,
                    color: None,
                    explosion: None,
                });
                self.points.len() - 1
            }
        };
        &mut self.points[at]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegendPos {
    None,
    Bottom,
    Right,
    Top,
    Left,
    /// In the top right corner.
    TopRight,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChartTitle {
    None,
    /// The series name when there is a single series, else a placeholder.
    Auto,
    Text(String),
}

/// Where the other axis crosses an axis.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AxisCrossing {
    /// At zero (or at the end nearest to zero) of a scale, before the
    /// first category of a category axis.
    #[default]
    Auto,
    Min,
    Max,
    /// At this value; on a category axis, at this category number (1 = the
    /// first).
    At(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TickMark {
    #[default]
    None,
    Inside,
    Outside,
    Cross,
}

/// Where the labels of an axis are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TickLabels {
    #[default]
    NextToAxis,
    /// At the high end of the other axis, wherever the axis itself is.
    High,
    Low,
    None,
}

/// "Display units" of a value axis: the labels show the values divided by
/// the unit.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DisplayUnit {
    #[default]
    None,
    Hundreds,
    Thousands,
    TenThousands,
    HundredThousands,
    Millions,
    TenMillions,
    HundredMillions,
    Billions,
    Trillions,
    Custom(f64),
}

impl DisplayUnit {
    pub fn divisor(self) -> f64 {
        match self {
            DisplayUnit::None => 1.0,
            DisplayUnit::Hundreds => 1e2,
            DisplayUnit::Thousands => 1e3,
            DisplayUnit::TenThousands => 1e4,
            DisplayUnit::HundredThousands => 1e5,
            DisplayUnit::Millions => 1e6,
            DisplayUnit::TenMillions => 1e7,
            DisplayUnit::HundredMillions => 1e8,
            DisplayUnit::Billions => 1e9,
            DisplayUnit::Trillions => 1e12,
            DisplayUnit::Custom(v) if v.is_finite() && v > 0.0 => v,
            DisplayUnit::Custom(_) => 1.0,
        }
    }
}

/// One axis ("Format Axis"). Scale options apply to axes that measure
/// numbers: the value axis, and both axes of scatter and bubble charts.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    /// Drawn (line, tick marks and labels). A hidden axis still scales
    /// its series.
    pub visible: bool,
    pub title: Option<String>,
    /// Bounds of the scale; `None` = automatic.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Distance between major gridlines / labels; `None` = automatic.
    pub major_unit: Option<f64>,
    pub minor_unit: Option<f64>,
    /// Logarithmic scale of this base (10 in Excel unless changed).
    pub log_base: Option<f64>,
    /// "Values in reverse order" / "Categories in reverse order".
    pub reversed: bool,
    /// Number format of the labels; `None` = that of the plotted cells.
    pub number_format: Option<String>,
    pub major_gridlines: bool,
    pub minor_gridlines: bool,
    /// Where the other axis crosses this one, in this axis' own terms (in
    /// a file this is stored on the other axis).
    pub crossing: AxisCrossing,
    pub display_unit: DisplayUnit,
    /// Show the name of the display unit next to the axis.
    pub display_unit_label: bool,
    pub major_ticks: TickMark,
    pub minor_ticks: TickMark,
    pub labels: TickLabels,
}

impl Axis {
    /// A visible axis, automatic in every respect, without gridlines.
    pub fn new() -> Axis {
        Axis {
            visible: true,
            title: None,
            min: None,
            max: None,
            major_unit: None,
            minor_unit: None,
            log_base: None,
            reversed: false,
            number_format: None,
            major_gridlines: false,
            minor_gridlines: false,
            crossing: AxisCrossing::Auto,
            display_unit: DisplayUnit::None,
            display_unit_label: true,
            major_ticks: TickMark::None,
            minor_ticks: TickMark::None,
            labels: TickLabels::NextToAxis,
        }
    }
}

impl Default for Axis {
    fn default() -> Self {
        Axis::new()
    }
}

/// The axes of a chart. `category` is the axis of the categories (the
/// horizontal one, the vertical one of bar charts) and the X axis of
/// scatter and bubble charts; `value` is the scale the values are measured
/// on. The secondary pair serves the series with `Series::secondary`; a
/// chart without such a series does not show it. Pies have no axes; a
/// radar chart uses `value` for its rings and `category` for its spokes.
#[derive(Clone, Debug, PartialEq)]
pub struct Axes {
    pub category: Axis,
    pub value: Axis,
    pub secondary_category: Axis,
    pub secondary_value: Axis,
}

impl Axes {
    /// The axes Excel gives a new chart of this kind: major gridlines on
    /// the value axis (on both axes of scatter and bubble charts), a
    /// secondary value axis ready to be shown and a hidden secondary
    /// category axis.
    pub fn for_kind(kind: ChartKind) -> Axes {
        let lined = Axis {
            major_gridlines: true,
            ..Axis::new()
        };
        Axes {
            category: if kind.is_xy() { lined.clone() } else { Axis::new() },
            value: lined,
            secondary_category: Axis {
                visible: false,
                ..Axis::new()
            },
            secondary_value: Axis::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Fill {
    /// White for the chart area, nothing for the plot area.
    #[default]
    Auto,
    None,
    Solid(Color),
}

/// Fill and border of the chart area or of the plot area.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShapeFormat {
    pub fill: Fill,
    /// Automatic: a thin light grey frame around the chart area, nothing
    /// around the plot area.
    pub border: Line,
}

/// The camera of a 3-D chart ("3-D Rotation").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View3D {
    /// Elevation ("Y rotation" in Excel's pane), degrees, -90 to 90.
    pub rot_x: i32,
    /// Rotation around the vertical axis ("X rotation"), degrees, 0 to 360.
    pub rot_y: u32,
    /// Field of view, 0 to 240 (twice the "Perspective" of Excel's pane).
    /// Unused with right-angle axes.
    pub perspective: u32,
    /// "Right angle axes": an oblique projection that keeps the front
    /// faces undistorted.
    pub right_angle_axes: bool,
    /// Depth of the chart in percent of its width, 20 to 2000.
    pub depth_percent: u32,
    /// Height in percent of the width; `None` = automatic (fills the plot).
    pub height_percent: Option<u32>,
}

impl View3D {
    /// The camera Excel gives a new 3-D chart of this kind.
    pub fn for_kind(kind: ChartKind) -> View3D {
        let (rot_x, rot_y, right_angle_axes) = match kind.family() {
            Family::Pie => (30, 0, false),
            Family::Column | Family::Bar => (15, 20, true),
            _ => (15, 20, false),
        };
        View3D {
            rot_x,
            rot_y,
            perspective: 30,
            right_angle_axes,
            depth_percent: 100,
            height_percent: None,
        }
    }
}

/// The table of the plotted values under the plot ("Data Table").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTable {
    pub horizontal_lines: bool,
    pub vertical_lines: bool,
    pub outline: bool,
    /// A swatch of each series' colour before its name.
    pub legend_keys: bool,
}

impl Default for DataTable {
    fn default() -> Self {
        DataTable {
            horizontal_lines: true,
            vertical_lines: true,
            outline: true,
            legend_keys: true,
        }
    }
}

/// How cells without a number are plotted ("Hidden and Empty Cells").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BlanksAs {
    #[default]
    Gap,
    Zero,
    /// Lines run from the point before to the point after.
    Span,
}

/// What a file said about a chart read from it. As long as nobody changes
/// the chart, it is saved as it was read, with everything the model has no
/// place for (fonts, gradients, effects, extensions).
#[derive(Clone, Debug, PartialEq)]
pub struct ForeignChart {
    /// The chart part of the file.
    pub xml: Arc<str>,
    /// `Chart::content` of the chart as the reader understood it.
    pub as_read: Chart,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Unique within the workbook (assigned by `Workbook::add_chart`).
    pub id: u32,
    pub kind: ChartKind,
    pub title: ChartTitle,
    pub legend: LegendPos,
    pub series: Vec<Series>,
    /// Label the points of every series that has no `Series::labels` of
    /// its own, with what `label_options` says.
    pub data_labels: bool,
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell, pixels at 100 % (x, y).
    pub offset: (f32, f32),
    /// Width and height, pixels at 100 %.
    pub size: (f32, f32),
    /// The title is drawn over the plot instead of above it.
    pub title_overlay: bool,
    /// The legend is drawn over the plot instead of beside it.
    pub legend_overlay: bool,
    /// Legend entries removed from the legend, by position: the series
    /// first (the points, for a pie), then the trendlines in series order.
    pub legend_hidden: Vec<u32>,
    pub label_options: DataLabels,
    pub axes: Axes,
    /// "Vary colors by point": every point of a series gets a colour of
    /// the palette. `None` = automatic: on for pies and doughnuts, off
    /// elsewhere.
    pub vary_colors: Option<bool>,
    /// Bars: gap between clusters and overlap within one, in percent of
    /// the width of a bar (0 to 500, -100 to 100); `None` = what Excel
    /// gives the kind (`ChartKind::default_gap_width`, `default_overlap`).
    pub gap_width: Option<u32>,
    pub overlap: Option<i32>,
    /// Pies and doughnuts: where the first slice starts, degrees clockwise
    /// from 12 o'clock (0 to 360).
    pub first_slice_angle: u32,
    /// Doughnuts: diameter of the hole in percent of the whole (10 to 90).
    pub hole_size: u32,
    /// Bubble charts: size of the largest bubble in percent of the default
    /// (0 to 300).
    pub bubble_scale: u32,
    /// 3-D charts: the camera; `None` = `View3D::for_kind`.
    pub view3d: Option<View3D>,
    pub chart_area: ShapeFormat,
    pub plot_area: ShapeFormat,
    pub data_table: Option<DataTable>,
    pub blanks: BlanksAs,
    /// Set by the file reader; see `ForeignChart`.
    pub foreign: Option<Arc<ForeignChart>>,
}

/// Default hole of a doughnut (Excel: 75 %).
pub const DEFAULT_HOLE_SIZE: u32 = 75;

/// Default chart size (12.7 cm × 7.62 cm, as in Excel).
pub const DEFAULT_CHART_SIZE: (f32, f32) = (480.0, 288.0);

/// Automatic series colours (Office theme accents).
pub const PALETTE: [u32; 6] = [0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E];

/// Colour of the `index`-th series / pie slice; later rounds are lightened
/// or darkened variants of the six accents, as Office does.
pub fn palette_color(index: usize) -> Color {
    let base = PALETTE[index % PALETTE.len()];
    let round = index / PALETTE.len();
    let amount: f32 = match round % 6 {
        0 => 0.0,
        1 => 0.4,
        2 => -0.25,
        3 => 0.6,
        4 => -0.5,
        _ => 0.2,
    };
    let ch = |shift: u32| -> u32 {
        let v = ((base >> shift) & 0xFF) as f32;
        let out = if amount >= 0.0 {
            v + (255.0 - v) * amount
        } else {
            v * (1.0 + amount)
        };
        (out.round().clamp(0.0, 255.0) as u32) << shift
    };
    Color(ch(16) | ch(8) | ch(0))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesData {
    pub name: String,
    /// One entry per point; `None` for blanks and text.
    pub values: Vec<Option<f64>>,
    pub color: Color,
    /// X values of the series (scatter and bubble charts): its own
    /// categories read as numbers, 1, 2, 3... when it has none.
    pub x_values: Vec<Option<f64>>,
    /// Bubble sizes; empty when the series has none.
    pub sizes: Vec<Option<f64>>,
    /// Number format of the first number of the series.
    pub format: String,
}

/// What a chart plots, read from the workbook.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChartData {
    /// Category labels (formatted cell text); numbered 1.. when absent.
    pub categories: Vec<String>,
    /// X values (scatter charts).
    pub x_values: Vec<Option<f64>>,
    pub series: Vec<SeriesData>,
    /// Number format of the plotted values (for axis and data labels).
    pub value_format: String,
    /// Number format of the X values (scatter charts).
    pub x_format: String,
    /// Title to display, when the chart has one that can be resolved.
    pub title: Option<String>,
}

/// A value axis: bounds and the distance between gridlines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisScale {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl AxisScale {
    pub fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if self.step <= 0.0 || !self.step.is_finite() {
            return vec![self.min, self.max];
        }
        let n = ((self.max - self.min) / self.step).round() as i64;
        for i in 0..=n.clamp(1, 200) {
            let v = self.min + self.step * i as f64;
            // Remove float noise (0.30000000000000004).
            out.push((v / self.step).round() * self.step);
        }
        out
    }

    /// Position of a value on the axis, 0 at `min` and 1 at `max`.
    pub fn fraction(&self, v: f64) -> f64 {
        if self.max == self.min {
            0.0
        } else {
            (v - self.min) / (self.max - self.min)
        }
    }
}

/// A readable axis for data between `lo` and `hi`: bounds on round
/// numbers, zero included unless the data sits far from it, and room above
/// the largest value.
pub fn nice_axis(lo: f64, hi: f64) -> AxisScale {
    let (mut lo, mut hi) = if lo.is_finite() && hi.is_finite() && lo <= hi {
        (lo, hi)
    } else {
        (0.0, 1.0)
    };
    // Like Excel, start at zero unless the values vary by less than a sixth
    // of their size (then zooming in on them reads better).
    if lo > 0.0 && (hi - lo) / hi > 1.0 / 6.0 {
        lo = 0.0;
    }
    if hi < 0.0 && (hi - lo) / lo.abs() > 1.0 / 6.0 {
        hi = 0.0;
    }
    if hi == lo {
        if hi == 0.0 {
            hi = 1.0;
        } else if hi > 0.0 {
            lo = 0.0;
        } else {
            hi = 0.0;
        }
    }
    let raw = (hi - lo) / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    let min = (lo / step).floor() * step;
    let mut max = (hi / step).ceil() * step;
    // Keep the tallest point clear of the top edge.
    if hi > 0.0 && max - hi < step * 0.05 {
        max += step;
    }
    let mut min = min;
    if lo < 0.0 && lo - min < step * 0.05 {
        min -= step;
    }
    AxisScale { min, max, step }
}

fn abs_cell(row: u32, col: u32) -> CellRef {
    CellRef {
        row,
        col,
        abs_row: true,
        abs_col: true,
    }
}

fn abs_ref(sheet: &str, rect: Rect) -> RefExpr {
    RefExpr {
        sheet: Some(Arc::from(sheet)),
        start: abs_cell(rect.r0, rect.c0),
        end: (!rect.is_single()).then(|| abs_cell(rect.r1, rect.c1)),
        kind: RefKind::Cells,
    }
}

impl Chart {
    /// An empty chart of the given kind, as Excel would insert it: automatic
    /// title, no legend, default size.
    pub fn new(kind: ChartKind) -> Chart {
        Chart {
            id: 0,
            kind,
            title: ChartTitle::Auto,
            legend: LegendPos::None,
            series: Vec::new(),
            data_labels: false,
            anchor: (0, 0),
            offset: (0.0, 0.0),
            size: DEFAULT_CHART_SIZE,
            title_overlay: false,
            legend_overlay: false,
            legend_hidden: Vec::new(),
            label_options: DataLabels::default(),
            axes: Axes::for_kind(kind),
            vary_colors: None,
            gap_width: None,
            overlap: None,
            first_slice_angle: 0,
            hole_size: DEFAULT_HOLE_SIZE,
            bubble_scale: 100,
            view3d: None,
            chart_area: ShapeFormat::default(),
            plot_area: ShapeFormat::default(),
            data_table: None,
            blanks: BlanksAs::Gap,
            foreign: None,
        }
    }

    /// "Change Chart Type": every series takes the new type.
    pub fn set_kind(&mut self, kind: ChartKind) {
        self.kind = kind;
        for s in &mut self.series {
            s.kind = None;
        }
    }

    /// The kind a series is drawn as: its own when the chart can show it
    /// next to the others, else the chart's.
    pub fn series_kind(&self, series: &Series) -> ChartKind {
        match series.kind {
            Some(kind) if self.kind.combines_with(kind) => kind,
            _ => self.kind,
        }
    }

    /// The labels of a series, when it has any: its own, else the chart's.
    pub fn labels_for<'a>(&'a self, series: &'a Series) -> Option<&'a DataLabels> {
        let labels = match &series.labels {
            Some(own) => own,
            None if self.data_labels => &self.label_options,
            None => return None,
        };
        labels.any().then_some(labels)
    }

    /// Whether the points of a series each get a colour of their own.
    pub fn varies_colors(&self) -> bool {
        self.vary_colors.unwrap_or(self.kind.is_round())
    }

    /// Whether a series is plotted against the secondary axes. Pies, radar
    /// and 3-D charts have a single set of axes.
    pub fn on_secondary(&self, series: &Series) -> bool {
        series.secondary && !self.kind.is_3d() && !self.kind.is_round() && self.kind.family() != Family::Radar
    }

    pub fn has_secondary_axis(&self) -> bool {
        self.series.iter().any(|s| self.on_secondary(s))
    }

    pub fn gap_width_or_default(&self) -> u32 {
        self.gap_width.unwrap_or(self.kind.default_gap_width()).min(500)
    }

    /// Overlap of the bars of kind `kind` (stacked bars always sit on top
    /// of each other unless told otherwise).
    pub fn overlap_for(&self, kind: ChartKind) -> i32 {
        self.overlap.unwrap_or(kind.default_overlap()).clamp(-100, 100)
    }

    pub fn view3d_or_default(&self) -> View3D {
        self.view3d.unwrap_or(View3D::for_kind(self.kind))
    }

    /// What the chart shows, without where it sits: a copy with `id`,
    /// `anchor`, `offset`, `size` and `foreign` reset. Two charts with the
    /// same content draw the same picture of the same cells.
    pub fn content(&self) -> Chart {
        Chart {
            id: 0,
            anchor: (0, 0),
            offset: (0.0, 0.0),
            size: DEFAULT_CHART_SIZE,
            foreign: None,
            ..self.clone()
        }
    }

    /// The chart part this chart was read from, when it still says what
    /// the chart shows (nothing but its place was changed since).
    pub fn unchanged_source(&self) -> Option<&str> {
        let foreign = self.foreign.as_ref()?;
        (self.content() == foreign.as_read).then_some(&*foreign.xml)
    }

    /// Rewrite every range the chart reads. `f` returns `None` when the
    /// range no longer exists; a series whose values are gone is dropped.
    pub fn map_refs(&mut self, f: &mut dyn FnMut(&RefExpr) -> Option<RefExpr>) {
        self.series.retain_mut(|s| {
            match f(&s.values) {
                Some(r) => s.values = r,
                None => return false,
            }
            if let Some(c) = &s.categories {
                s.categories = f(c);
            }
            if let Some(c) = &s.bubble_sizes {
                s.bubble_sizes = f(c);
            }
            if let SeriesName::Cell(c) = &s.name {
                s.name = match f(c) {
                    Some(r) => SeriesName::Cell(r),
                    None => SeriesName::None,
                };
            }
            true
        });
    }

    /// Follow rows / columns inserted into or deleted from `edited_sheet`.
    /// `own_sheet` is the sheet holding the chart.
    pub fn adjust(&mut self, edit: StructuralEdit, edited_sheet: &str, own_sheet: &str) {
        self.map_refs(&mut |r| {
            match Expr::Ref(r.clone()).adjusted(edit, edited_sheet, own_sheet) {
                Expr::Ref(r) => Some(r),
                _ => None,
            }
        });
        if !edited_sheet.eq_ignore_ascii_case(own_sheet) {
            return;
        }
        let shift = |pos: u32, insert: bool, at: u32, count: u32| -> u32 {
            if insert {
                if pos >= at { pos + count } else { pos }
            } else if pos >= at + count {
                pos - count
            } else if pos >= at {
                at
            } else {
                pos
            }
        };
        match edit {
            StructuralEdit::InsertRows { at, count } => self.anchor.0 = shift(self.anchor.0, true, at, count),
            StructuralEdit::DeleteRows { at, count } => self.anchor.0 = shift(self.anchor.0, false, at, count),
            StructuralEdit::InsertCols { at, count } => self.anchor.1 = shift(self.anchor.1, true, at, count),
            StructuralEdit::DeleteCols { at, count } => self.anchor.1 = shift(self.anchor.1, false, at, count),
        }
    }

    pub fn rename_sheet(&mut self, old: &str, new: &str) {
        self.map_refs(&mut |r| {
            let mut out = r.clone();
            if r.sheet.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(old)) {
                out.sheet = Some(Arc::from(new));
            }
            Some(out)
        });
    }

    /// Swap the roles of rows and columns of the source block ("Switch
    /// Row/Column"). Only possible when the series form one rectangle.
    pub fn switch_row_column(&mut self, wb: &Workbook, own_sheet: usize) -> bool {
        let Some(first) = self.series.first() else {
            return false;
        };
        let sheet_name = first.values.sheet.clone();
        let mut block = first.values.rect();
        for s in &self.series {
            if s.values.sheet != sheet_name {
                return false;
            }
            block = block.union(&s.values.rect());
            if let Some(c) = &s.categories {
                block = block.union(&c.rect());
            }
            if let Some(c) = &s.bubble_sizes {
                block = block.union(&c.rect());
            }
            if let SeriesName::Cell(c) = &s.name {
                block = block.union(&c.rect());
            }
        }
        let sheet = match &sheet_name {
            Some(name) => match wb.sheet_index(name) {
                Some(i) => i,
                None => return false,
            },
            None => own_sheet,
        };
        let by_rows = first.values.rect().rows() == 1 && first.values.rect().cols() > 1;
        let rebuilt = wb.chart_from_block(sheet, block, self.kind, Some(!by_rows));
        if rebuilt.series.is_empty() {
            return false;
        }
        self.series = rebuilt.series;
        true
    }
}

impl Workbook {
    fn resolve_chart_ref(&self, r: &RefExpr, own_sheet: usize) -> Option<(usize, Rect)> {
        let sheet = match &r.sheet {
            Some(name) => self.sheet_index(name)?,
            None => own_sheet,
        };
        Some((sheet, self.clamp_to_used(sheet, r.rect())))
    }

    /// Build a chart of the given kind from a selection, guessing which
    /// row / column holds names and categories the way Excel does. The
    /// chart is not added to the sheet.
    pub fn chart_from_range(&self, sheet: usize, rect: Rect, kind: ChartKind) -> Chart {
        let rect = if rect.is_single() {
            self.current_region(sheet, rect.r0, rect.c0)
        } else {
            self.clamp_to_used(sheet, rect)
        };
        self.chart_from_block(sheet, rect, kind, None)
    }

    /// `by_rows`: force series to run along rows (`Some(true)`) or columns
    /// (`Some(false)`); `None` picks the orientation with fewer series.
    pub fn chart_from_block(&self, sheet: usize, rect: Rect, kind: ChartKind, by_rows: Option<bool>) -> Chart {
        let mut chart = Chart::new(kind);
        let Some(sh) = self.sheets.get(sheet) else {
            return chart;
        };
        let name = sh.name.as_str();
        // A cell is a label when it holds text or a date; numbers are data.
        let is_number = |r: u32, c: u32| match sh.value_at(r, c) {
            Value::Number(_) => !format::is_date_format(&self.styles.get(sh.style_id_at(r, c)).num_fmt),
            _ => false,
        };
        let is_blank = |r: u32, c: u32| sh.is_blank(r, c);

        let header_row = rect.rows() > 1
            && (rect.c0..=rect.c1).all(|c| !is_number(rect.r0, c))
            && (rect.c0..=rect.c1).any(|c| !is_blank(rect.r0, c));
        let first_data_row = if header_row { rect.r0 + 1 } else { rect.r0 };
        let label_col = rect.cols() > 1
            && !kind.is_xy()
            && (first_data_row..=rect.r1).all(|r| !is_number(r, rect.c0))
            && (first_data_row..=rect.r1).any(|r| !is_blank(r, rect.c0));
        let first_data_col = if label_col { rect.c0 + 1 } else { rect.c0 };
        if first_data_row > rect.r1 || first_data_col > rect.c1 {
            return chart;
        }
        let data = Rect::new(first_data_row, first_data_col, rect.r1, rect.c1);

        let column = |c: u32| abs_ref(name, Rect::new(data.r0, c, data.r1, c));
        let header = |c: u32| {
            if header_row {
                SeriesName::Cell(abs_ref(name, Rect::cell(rect.r0, c)))
            } else {
                SeriesName::None
            }
        };
        if kind.family() == Family::Bubble && data.cols() >= 3 {
            // First column: X values; then pairs of Y values and sizes.
            let mut c = data.c0 + 1;
            while c <= data.c1 {
                chart.series.push(Series {
                    name: header(c),
                    categories: Some(column(data.c0)),
                    bubble_sizes: (c < data.c1).then(|| column(c + 1)),
                    ..Series::new(column(c))
                });
                c += 2;
            }
        } else if kind.is_xy() && data.cols() >= 2 {
            // First column: X values; every other column is a Y series.
            for c in data.c0 + 1..=data.c1 {
                chart.series.push(Series {
                    name: header(c),
                    categories: Some(column(data.c0)),
                    ..Series::new(column(c))
                });
            }
        } else {
            let in_rows = by_rows.unwrap_or(data.cols() > data.rows());
            if in_rows {
                for r in data.r0..=data.r1 {
                    chart.series.push(Series {
                        name: if label_col {
                            SeriesName::Cell(abs_ref(name, Rect::cell(r, rect.c0)))
                        } else {
                            SeriesName::None
                        },
                        categories: header_row.then(|| abs_ref(name, Rect::new(rect.r0, data.c0, rect.r0, data.c1))),
                        ..Series::new(abs_ref(name, Rect::new(r, data.c0, r, data.c1)))
                    });
                }
            } else {
                for c in data.c0..=data.c1 {
                    chart.series.push(Series {
                        name: header(c),
                        categories: label_col.then(|| abs_ref(name, Rect::new(data.r0, rect.c0, data.r1, rect.c0))),
                        ..Series::new(column(c))
                    });
                }
            }
        }
        // Columns without a single number are not worth a series.
        chart.series.retain(|s| {
            let r = s.values.rect();
            (r.r0..=r.r1).any(|row| (r.c0..=r.c1).any(|col| matches!(sh.value_at(row, col), Value::Number(_))))
        });
        // A pie shows one series; a doughnut has a ring for each.
        if kind.family() == Family::Pie {
            chart.series.truncate(1);
        }
        chart.legend = if kind.is_round() || chart.series.len() > 1 {
            LegendPos::Bottom
        } else {
            LegendPos::None
        };
        chart
    }

    /// Read the data a chart plots.
    pub fn chart_data(&self, own_sheet: usize, chart: &Chart) -> ChartData {
        let mut data = ChartData {
            value_format: "General".to_string(),
            x_format: "General".to_string(),
            ..Default::default()
        };
        let mut have_format = false;
        let mut points = 0usize;
        // The numbers of a range, and the format of the first of them.
        let numbers = |r: &RefExpr| -> (Vec<Option<f64>>, Option<String>) {
            let mut values = Vec::new();
            let mut code = None;
            if let Some((si, rect)) = self.resolve_chart_ref(r, own_sheet) {
                let sh = &self.sheets[si];
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        let v = sh.value_at(r, c);
                        if code.is_none() && matches!(v, Value::Number(_)) {
                            code = Some(self.styles.get(sh.style_id_at(r, c)).num_fmt.to_string());
                        }
                        values.push(match v {
                            Value::Number(n) if n.is_finite() => Some(n),
                            _ => None,
                        });
                    }
                }
            }
            (values, code)
        };
        for (i, s) in chart.series.iter().enumerate() {
            let (values, code) = numbers(&s.values);
            if !have_format && let Some(code) = &code {
                data.value_format = code.clone();
                have_format = true;
            }
            let x_values = match &s.categories {
                Some(r) if chart.kind.is_xy() => {
                    let (xs, _) = numbers(r);
                    // Text where numbers are expected: Excel counts the points.
                    if xs.iter().any(Option::is_some) {
                        xs
                    } else {
                        (0..values.len()).map(|k| Some(k as f64 + 1.0)).collect()
                    }
                }
                _ => (0..values.len()).map(|k| Some(k as f64 + 1.0)).collect(),
            };
            let sizes = s.bubble_sizes.as_ref().map(|r| numbers(r).0).unwrap_or_default();
            points = points.max(values.len());
            let name = match &s.name {
                SeriesName::None => String::new(),
                SeriesName::Text(t) => t.clone(),
                SeriesName::Cell(r) => match self.resolve_chart_ref(r, own_sheet) {
                    Some((si, rect)) => self.display_text(si, rect.r0, rect.c0).text,
                    None => String::new(),
                },
            };
            data.series.push(SeriesData {
                name,
                values,
                color: s.color.unwrap_or_else(|| palette_color(i)),
                x_values,
                sizes,
                format: code.unwrap_or_else(|| "General".to_string()),
            });
        }
        // Categories / X values come from the first series that has them.
        if let Some(cats) = chart.series.iter().find_map(|s| s.categories.as_ref()) {
            if let Some((si, rect)) = self.resolve_chart_ref(cats, own_sheet) {
                let sh = &self.sheets[si];
                let mut first = true;
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        let v = sh.value_at(r, c);
                        if first && matches!(v, Value::Number(_)) {
                            data.x_format = self.styles.get(sh.style_id_at(r, c)).num_fmt.to_string();
                            first = false;
                        }
                        data.x_values.push(match v {
                            Value::Number(n) if n.is_finite() => Some(n),
                            _ => None,
                        });
                        data.categories.push(self.display_text(si, r, c).text);
                    }
                }
            }
        }
        if data.categories.len() < points {
            for i in data.categories.len()..points {
                data.categories.push((i + 1).to_string());
            }
        }
        if data.x_values.len() < points {
            for i in data.x_values.len()..points {
                data.x_values.push(Some(i as f64 + 1.0));
            }
        }
        data.title = match &chart.title {
            ChartTitle::None => None,
            ChartTitle::Text(t) => Some(t.clone()),
            // A pie only shows its first series, so that is its title too.
            ChartTitle::Auto => match data.series.as_slice() {
                [only] if !only.name.is_empty() => Some(only.name.clone()),
                [first, ..] if chart.kind.family() == Family::Pie && !first.name.is_empty() => Some(first.name.clone()),
                _ => None,
            },
        };
        data
    }

    /// The ranges a chart reads on `sheet`, for highlighting them when the
    /// chart is selected: (rectangle, role) with role 0 = values, 1 =
    /// categories, 2 = series names.
    pub fn chart_ranges(&self, sheet: usize, chart: &Chart) -> Vec<(Rect, u8)> {
        let mut out: Vec<(Rect, u8)> = Vec::new();
        let push = |r: &RefExpr, role: u8, out: &mut Vec<(Rect, u8)>| {
            if let Some((si, rect)) = self.resolve_chart_ref(r, sheet) {
                if si == sheet && !out.iter().any(|(x, _)| *x == rect) {
                    out.push((rect, role));
                }
            }
        };
        for s in &chart.series {
            push(&s.values, 0, &mut out);
            if let Some(c) = &s.categories {
                push(c, 1, &mut out);
            }
            if let Some(c) = &s.bubble_sizes {
                push(c, 0, &mut out);
            }
            if let SeriesName::Cell(c) = &s.name {
                push(c, 2, &mut out);
            }
        }
        out
    }

    fn next_chart_id(&self) -> u32 {
        self.sheets
            .iter()
            .flat_map(|s| s.charts.iter().map(|c| c.id))
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Add a chart to a sheet; returns its id.
    pub fn add_chart(&mut self, sheet: usize, mut chart: Chart) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        self.snapshot();
        chart.id = self.next_chart_id();
        let id = chart.id;
        self.sheets[sheet].charts.push(chart);
        Some(id)
    }

    pub fn remove_chart(&mut self, sheet: usize, id: u32) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        if !sh.charts.iter().any(|c| c.id == id) {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].charts.retain(|c| c.id != id);
        true
    }

    pub fn chart(&self, sheet: usize, id: u32) -> Option<&Chart> {
        self.sheets.get(sheet)?.charts.iter().find(|c| c.id == id)
    }

    /// Change a chart (recorded for undo).
    pub fn update_chart(&mut self, sheet: usize, id: u32, f: impl FnOnce(&mut Chart)) -> bool {
        if self.chart(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        if let Some(chart) = self.sheets[sheet].charts.iter_mut().find(|c| c.id == id) {
            f(chart);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn table() -> Workbook {
        wb(&[
            ("A1", "Product"),
            ("B1", "Q1"),
            ("C1", "Q2"),
            ("A2", "Keyboard"),
            ("B2", "10"),
            ("C2", "15"),
            ("A3", "Mouse"),
            ("B3", "20"),
            ("C3", "5"),
            ("A4", "Screen"),
            ("B4", "30"),
            ("C4", "45"),
        ])
    }

    #[test]
    fn chart_from_table() {
        let w = table();
        let chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        assert_eq!(chart.series.len(), 2);
        assert_eq!(chart.series[0].values.to_text(), "Sheet1!$B$2:$B$4");
        assert_eq!(chart.series[0].categories.as_ref().unwrap().to_text(), "Sheet1!$A$2:$A$4");
        assert_eq!(chart.legend, LegendPos::Bottom);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.categories, vec!["Keyboard", "Mouse", "Screen"]);
        assert_eq!(data.series[0].name, "Q1");
        assert_eq!(data.series[1].values, vec![Some(15.0), Some(5.0), Some(45.0)]);
        assert_eq!(data.title, None);

        // A single cell expands to the block around it; a pie keeps one series.
        let pie = w.chart_from_range(0, Rect::parse("B2").unwrap(), ChartKind::Pie);
        assert_eq!(pie.series.len(), 1);
        assert_eq!(w.chart_data(0, &pie).title.as_deref(), Some("Q1"));
    }

    #[test]
    fn orientation_and_switch() {
        // Wide block: more columns than rows → one series per row.
        let w = wb(&[
            ("A1", ""),
            ("B1", "Jan"),
            ("C1", "Feb"),
            ("D1", "Mar"),
            ("A2", "North"),
            ("B2", "1"),
            ("C2", "2"),
            ("D2", "3"),
            ("A3", "South"),
            ("B3", "4"),
            ("C3", "5"),
            ("D3", "6"),
        ]);
        let mut chart = w.chart_from_range(0, Rect::parse("A1:D3").unwrap(), ChartKind::Line);
        assert_eq!(chart.series.len(), 2);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.series[0].name, "North");
        assert_eq!(data.categories, vec!["Jan", "Feb", "Mar"]);
        assert!(chart.switch_row_column(&w, 0));
        assert_eq!(chart.series.len(), 3);
        assert_eq!(w.chart_data(0, &chart).series[0].name, "Jan");
    }

    #[test]
    fn scatter_uses_first_column_as_x() {
        let w = wb(&[("A1", "x"), ("B1", "y"), ("A2", "1"), ("B2", "10"), ("A3", "2"), ("B3", "20")]);
        let chart = w.chart_from_range(0, Rect::parse("A1:B3").unwrap(), ChartKind::Scatter);
        assert_eq!(chart.series.len(), 1);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.x_values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(data.series[0].values, vec![Some(10.0), Some(20.0)]);
    }

    #[test]
    fn kinds() {
        use ChartKind::*;
        for kind in ChartKind::ALL {
            // Every kind is in exactly one family, and says so consistently.
            assert_eq!(kind.is_round(), matches!(kind.family(), Family::Pie | Family::Doughnut));
            assert_eq!(kind.is_stacked(), format!("{kind:?}").contains("Stacked"));
            assert_eq!(kind.is_percent(), format!("{kind:?}").starts_with("Percent"));
            assert_eq!(kind.is_3d(), format!("{kind:?}").ends_with("3D"));
            assert!(kind.combines_with(kind));
            assert_eq!(ChartKind::ALL.iter().filter(|k| **k == kind).count(), 1);
        }
        assert!(Column.combines_with(LineMarkers) && StackedArea.combines_with(Column));
        assert!(Scatter.combines_with(ScatterSmooth) && Radar.combines_with(RadarFilled));
        assert!(Bar.combines_with(StackedBar) && !Bar.combines_with(Line));
        assert!(!Column.combines_with(Pie) && !Column3D.combines_with(Line) && !Bubble.combines_with(Scatter));
        // What Excel gives new charts.
        assert_eq!((Column.default_gap_width(), Column.default_overlap()), (219, -27));
        assert_eq!((Bar.default_gap_width(), Bar.default_overlap()), (182, 0));
        assert_eq!((StackedBar.default_gap_width(), PercentStackedColumn.default_overlap()), (150, 100));
        assert_eq!(Column3D.default_gap_width(), 150);
        assert!(LineMarkers.has_markers() && !Line.has_markers() && Scatter.has_markers());
        assert!(Line.has_lines() && !Scatter.has_lines() && ScatterSmooth.is_smooth());
        assert!(View3D::for_kind(Column3D).right_angle_axes && !View3D::for_kind(Line3D).right_angle_axes);
        assert_eq!(View3D::for_kind(Pie3D).rot_x, 30);
        assert!(Axes::for_kind(Scatter).category.major_gridlines && !Axes::for_kind(Column).category.major_gridlines);
    }

    #[test]
    fn automatic_choices() {
        let w = table();
        let mut chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        // A series of another type, when the chart can show it.
        chart.series[1].kind = Some(ChartKind::Line);
        assert_eq!(chart.series_kind(&chart.series[1]), ChartKind::Line);
        chart.series[1].kind = Some(ChartKind::Pie);
        assert_eq!(chart.series_kind(&chart.series[1]), ChartKind::Column);
        chart.series[1].kind = Some(ChartKind::Line);
        chart.set_kind(ChartKind::Area);
        assert!(chart.series.iter().all(|s| s.kind.is_none()));
        // Markers and lines follow the kind until the series says otherwise.
        let s = &mut chart.series[0];
        assert_eq!(s.marker_for(ChartKind::LineMarkers).shape, MarkerShape::Circle);
        assert_eq!(s.marker_for(ChartKind::Line).shape, MarkerShape::None);
        assert!(s.line_shown(ChartKind::Line) && !s.line_shown(ChartKind::Scatter));
        s.marker = Some(Marker::new(MarkerShape::Diamond));
        s.line.shown = Some(false);
        assert_eq!(s.marker_for(ChartKind::Line).shape, MarkerShape::Diamond);
        assert!(!s.line_shown(ChartKind::Line));
        s.point_mut(2).color = Some(Color(0xFF0000));
        s.point_mut(2).explosion = Some(10);
        assert_eq!(s.points.len(), 1);
        assert_eq!(s.point(2).unwrap().color, Some(Color(0xFF0000)));
        assert!(s.point(0).is_none());
        // Labels: the chart's unless the series has its own, nothing when
        // nothing is shown.
        assert!(chart.labels_for(&chart.series[0]).is_none());
        chart.data_labels = true;
        assert!(chart.labels_for(&chart.series[0]).unwrap().value);
        chart.series[0].labels = Some(DataLabels::none());
        assert!(chart.labels_for(&chart.series[0]).is_none());
        chart.series[1].labels = Some(DataLabels {
            percent: true,
            ..DataLabels::none()
        });
        assert!(chart.labels_for(&chart.series[1]).unwrap().percent);
        // Secondary axes only where a chart can have them.
        chart.series[1].secondary = true;
        assert!(chart.has_secondary_axis());
        chart.set_kind(ChartKind::Column3D);
        assert!(!chart.has_secondary_axis());
        chart.set_kind(ChartKind::Pie);
        assert!(chart.varies_colors() && !chart.has_secondary_axis());
        assert_eq!(chart.view3d_or_default(), View3D::for_kind(ChartKind::Pie));
    }

    #[test]
    fn foreign_charts_know_when_they_were_changed() {
        let mut w = table();
        let mut chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        chart.foreign = Some(Arc::new(ForeignChart {
            xml: Arc::from("<c:chartSpace/>"),
            as_read: chart.content(),
        }));
        let id = w.add_chart(0, chart).unwrap();
        w.commit("chart");
        assert_eq!(w.chart(0, id).unwrap().unchanged_source(), Some("<c:chartSpace/>"));
        // Moving and resizing leave what the chart shows alone.
        w.update_chart(0, id, |c| {
            c.anchor = (5, 5);
            c.size = (100.0, 80.0);
        });
        assert!(w.chart(0, id).unwrap().unchanged_source().is_some());
        // Anything else does not; nor does an edit of the sheet that moves
        // the cells it reads.
        w.update_chart(0, id, |c| c.axes.value.reversed = true);
        assert!(w.chart(0, id).unwrap().unchanged_source().is_none());
        w.update_chart(0, id, |c| c.axes.value.reversed = false);
        assert!(w.chart(0, id).unwrap().unchanged_source().is_some());
        w.insert_rows(0, 0, 1);
        w.commit("insert");
        assert!(w.chart(0, id).unwrap().unchanged_source().is_none());
    }

    #[test]
    fn bubbles_and_doughnuts_from_a_block() {
        let w = wb(&[
            ("A1", "x"),
            ("B1", "y"),
            ("C1", "size"),
            ("D1", "y2"),
            ("A2", "1"),
            ("B2", "10"),
            ("C2", "3"),
            ("D2", "7"),
            ("A3", "2"),
            ("B3", "20"),
            ("C3", "5"),
            ("D3", "9"),
        ]);
        let chart = w.chart_from_range(0, Rect::parse("A1:D3").unwrap(), ChartKind::Bubble);
        assert_eq!(chart.series.len(), 2);
        assert_eq!(chart.series[0].bubble_sizes.as_ref().unwrap().to_text(), "Sheet1!$C$2:$C$3");
        assert!(chart.series[1].bubble_sizes.is_none());
        let data = w.chart_data(0, &chart);
        assert_eq!(data.series[0].sizes, vec![Some(3.0), Some(5.0)]);
        assert_eq!(data.series[0].x_values, vec![Some(1.0), Some(2.0)]);
        assert!(data.series[1].sizes.is_empty());
        assert_eq!(w.chart_ranges(0, &chart).len(), 6);
        // A doughnut has a ring per series (here the two rows), a pie
        // shows the first one.
        assert_eq!(w.chart_from_range(0, Rect::parse("A1:D3").unwrap(), ChartKind::Doughnut).series.len(), 2);
        assert_eq!(w.chart_from_range(0, Rect::parse("A1:D3").unwrap(), ChartKind::Pie3D).series.len(), 1);
        // Each series keeps the format of its own numbers.
        assert_eq!(data.series[0].format, "General");
    }

    #[test]
    fn axis() {
        assert_eq!(nice_axis(0.0, 2023.0), AxisScale { min: 0.0, max: 2500.0, step: 500.0 });
        assert_eq!(nice_axis(5.0, 45.0), AxisScale { min: 0.0, max: 50.0, step: 10.0 });
        assert_eq!(nice_axis(-20.0, 30.0).min, -20.0 - 10.0);
        let a = nice_axis(0.0, 0.0);
        assert!(a.max > a.min);
        let a = nice_axis(98.0, 102.0);
        assert!(a.min > 0.0 && a.min <= 98.0 && a.max >= 102.0);
        // A value sitting exactly on a gridline gets one more step of headroom.
        assert_eq!(nice_axis(0.0, 1.0).ticks().len(), 7);
        assert_eq!(nice_axis(0.0, 0.9).ticks().len(), 6);
    }

    #[test]
    fn charts_follow_edits_and_undo() {
        let mut w = table();
        let chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        let id = w.add_chart(0, chart).unwrap();
        w.commit("chart");
        w.insert_rows(0, 0, 2);
        w.commit("insert");
        let chart = w.chart(0, id).unwrap();
        assert_eq!(chart.series[0].values.to_text(), "Sheet1!$B$4:$B$6");
        assert_eq!(chart.anchor.0, 2);
        assert!(w.rename_sheet(0, "Data"));
        w.commit("rename");
        assert_eq!(w.chart(0, id).unwrap().series[0].values.to_text(), "Data!$B$4:$B$6");
        assert_eq!(w.chart_data(0, w.chart(0, id).unwrap()).series[0].values[0], Some(10.0));
        w.undo();
        w.undo();
        assert_eq!(w.chart(0, id).unwrap().series[0].values.to_text(), "Sheet1!$B$2:$B$4");
        w.undo();
        assert!(w.chart(0, id).is_none());
        w.redo();
        assert!(w.chart(0, id).is_some());
    }
}
