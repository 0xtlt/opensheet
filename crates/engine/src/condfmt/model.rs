//! The conditional-format model. Every type mirrors one OOXML construct;
//! the `ooxml` / `from_ooxml` functions give the spellings used in files.

use crate::ast::Expr;
use crate::refs::Rect;
use crate::style::{Color, StylePatch};

/// One conditional format: a rule, the cells it applies to and the format
/// it gives them (one `cfRule` with the `sqref` of its parent).
#[derive(Clone, Debug, PartialEq)]
pub struct CondFormat {
    /// "Applies to". Formulas in `rule` are written for the top-left cell
    /// of the first range.
    pub ranges: Vec<Rect>,
    pub rule: Rule,
    /// The format of the cells that match (the `dxf`). Colour scales, data
    /// bars and icon sets do not use it.
    pub format: StylePatch,
    /// Lower-priority rules are not looked at for a cell this rule matches.
    pub stop_if_true: bool,
}

impl CondFormat {
    pub fn new(ranges: Vec<Rect>, rule: Rule) -> CondFormat {
        CondFormat {
            ranges,
            rule,
            format: StylePatch::default(),
            stop_if_true: false,
        }
    }

    pub fn with_format(mut self, format: StylePatch) -> CondFormat {
        self.format = format;
        self
    }

    pub fn with_stop_if_true(mut self, stop: bool) -> CondFormat {
        self.stop_if_true = stop;
        self
    }

    /// The cell the rule's formulas are written for: the top-left cell of
    /// the first range.
    pub fn anchor(&self) -> (u32, u32) {
        self.ranges.first().map(|r| (r.r0, r.c0)).unwrap_or((0, 0))
    }

    pub fn applies_to(&self, row: u32, col: u32) -> bool {
        self.ranges.iter().any(|r| r.contains(row, col))
    }

    pub fn intersects(&self, rect: &Rect) -> bool {
        self.ranges.iter().any(|r| r.intersects(rect))
    }
}

/// What a rule tests or draws (`cfRule/@type` and what goes with it).
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// `cellIs`: the cell's value compared with one or two operands.
    /// `second` is only used by `Between` and `NotBetween`.
    CellIs {
        op: CellOp,
        first: Expr,
        second: Option<Expr>,
    },
    /// `expression`: the formula is true (a non-zero number or TRUE).
    Formula(Expr),
    /// `top10`: among the `rank` largest (or smallest) numbers of the
    /// ranges, or the top `rank` percent of them.
    TopBottom {
        rank: u32,
        percent: bool,
        bottom: bool,
    },
    /// `aboveAverage`: above or below the average of the ranges, optionally
    /// "or equal", or beyond `std_dev` standard deviations from it.
    Average {
        above: bool,
        or_equal: bool,
        std_dev: u32,
    },
    /// `duplicateValues`
    DuplicateValues,
    /// `uniqueValues`
    UniqueValues,
    /// `containsText`, `notContainsText`, `beginsWith`, `endsWith`. `text`
    /// is a text constant or a formula giving the text.
    Text {
        op: TextOp,
        text: Expr,
    },
    /// `containsBlanks`
    Blanks,
    /// `notContainsBlanks`
    NoBlanks,
    /// `containsErrors`
    Errors,
    /// `notContainsErrors`
    NoErrors,
    /// `timePeriod`: a date in a period counted from today.
    DateOccurring(TimePeriod),
    ColorScale(ColorScale),
    DataBar(DataBar),
    IconSet(IconRule),
}

impl Rule {
    /// `cfRule/@type`.
    pub fn ooxml_type(&self) -> &'static str {
        match self {
            Rule::CellIs { .. } => "cellIs",
            Rule::Formula(_) => "expression",
            Rule::TopBottom { .. } => "top10",
            Rule::Average { .. } => "aboveAverage",
            Rule::DuplicateValues => "duplicateValues",
            Rule::UniqueValues => "uniqueValues",
            Rule::Text { op, .. } => op.ooxml_type(),
            Rule::Blanks => "containsBlanks",
            Rule::NoBlanks => "notContainsBlanks",
            Rule::Errors => "containsErrors",
            Rule::NoErrors => "notContainsErrors",
            Rule::DateOccurring(_) => "timePeriod",
            Rule::ColorScale(_) => "colorScale",
            Rule::DataBar(_) => "dataBar",
            Rule::IconSet(_) => "iconSet",
        }
    }

    /// Colour scales, data bars and icon sets: rules that grade every
    /// number of the range instead of testing a condition.
    pub fn is_graded(&self) -> bool {
        matches!(
            self,
            Rule::ColorScale(_) | Rule::DataBar(_) | Rule::IconSet(_)
        )
    }

    /// Every formula of the rule: operands, the formula of a formula rule,
    /// threshold values.
    pub fn exprs(&self) -> Vec<&Expr> {
        let mut out = Vec::new();
        match self {
            Rule::CellIs { first, second, .. } => {
                out.push(first);
                out.extend(second);
            }
            Rule::Formula(e) | Rule::Text { text: e, .. } => out.push(e),
            Rule::ColorScale(scale) => out.extend(scale.stops.iter().filter_map(|s| s.at.value())),
            Rule::DataBar(bar) => {
                out.extend(bar.min.value());
                out.extend(bar.max.value());
            }
            Rule::IconSet(icons) => {
                out.extend(icons.thresholds.iter().filter_map(|t| t.value.value()))
            }
            _ => {}
        }
        out
    }

    /// Replace every formula of the rule by `f(formula)`.
    pub fn map_exprs(&mut self, f: &mut dyn FnMut(&Expr) -> Expr) {
        match self {
            Rule::CellIs { first, second, .. } => {
                *first = f(first);
                if let Some(second) = second {
                    *second = f(second);
                }
            }
            Rule::Formula(e) | Rule::Text { text: e, .. } => *e = f(e),
            Rule::ColorScale(scale) => scale.stops.iter_mut().for_each(|s| s.at.map_expr(f)),
            Rule::DataBar(bar) => {
                bar.min.map_expr(f);
                bar.max.map_expr(f);
            }
            Rule::IconSet(icons) => icons
                .thresholds
                .iter_mut()
                .for_each(|t| t.value.map_expr(f)),
            _ => {}
        }
    }
}

/// `cfRule/@operator` of a `cellIs` rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellOp {
    Between,
    NotBetween,
    Equal,
    NotEqual,
    Greater,
    Less,
    GreaterOrEqual,
    LessOrEqual,
}

impl CellOp {
    /// In the order of the Edit Rule dialog.
    pub const ALL: [CellOp; 8] = [
        CellOp::Between,
        CellOp::NotBetween,
        CellOp::Equal,
        CellOp::NotEqual,
        CellOp::Greater,
        CellOp::Less,
        CellOp::GreaterOrEqual,
        CellOp::LessOrEqual,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            CellOp::Between => "between",
            CellOp::NotBetween => "notBetween",
            CellOp::Equal => "equal",
            CellOp::NotEqual => "notEqual",
            CellOp::Greater => "greaterThan",
            CellOp::Less => "lessThan",
            CellOp::GreaterOrEqual => "greaterThanOrEqual",
            CellOp::LessOrEqual => "lessThanOrEqual",
        }
    }

    pub fn from_ooxml(s: &str) -> Option<CellOp> {
        CellOp::ALL.into_iter().find(|op| op.ooxml() == s)
    }

    /// `Between` and `NotBetween` take two operands.
    pub fn takes_two(self) -> bool {
        matches!(self, CellOp::Between | CellOp::NotBetween)
    }
}

/// The test of a text rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextOp {
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
}

impl TextOp {
    /// In the order of the Edit Rule dialog.
    pub const ALL: [TextOp; 4] = [
        TextOp::Contains,
        TextOp::NotContains,
        TextOp::BeginsWith,
        TextOp::EndsWith,
    ];

    /// `cfRule/@type`.
    pub fn ooxml_type(self) -> &'static str {
        match self {
            TextOp::Contains => "containsText",
            TextOp::NotContains => "notContainsText",
            TextOp::BeginsWith => "beginsWith",
            TextOp::EndsWith => "endsWith",
        }
    }

    /// `cfRule/@operator` (Excel writes it next to the type).
    pub fn ooxml_operator(self) -> &'static str {
        match self {
            TextOp::Contains => "containsText",
            TextOp::NotContains => "notContains",
            TextOp::BeginsWith => "beginsWith",
            TextOp::EndsWith => "endsWith",
        }
    }

    pub fn from_ooxml_type(s: &str) -> Option<TextOp> {
        TextOp::ALL.into_iter().find(|op| op.ooxml_type() == s)
    }
}

/// `cfRule/@timePeriod`. Weeks run from Sunday to Saturday.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimePeriod {
    Yesterday,
    Today,
    Tomorrow,
    Last7Days,
    LastWeek,
    ThisWeek,
    NextWeek,
    LastMonth,
    ThisMonth,
    NextMonth,
}

impl TimePeriod {
    /// In the order of the "A Date Occurring" dialog.
    pub const ALL: [TimePeriod; 10] = [
        TimePeriod::Yesterday,
        TimePeriod::Today,
        TimePeriod::Tomorrow,
        TimePeriod::Last7Days,
        TimePeriod::LastWeek,
        TimePeriod::ThisWeek,
        TimePeriod::NextWeek,
        TimePeriod::LastMonth,
        TimePeriod::ThisMonth,
        TimePeriod::NextMonth,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            TimePeriod::Yesterday => "yesterday",
            TimePeriod::Today => "today",
            TimePeriod::Tomorrow => "tomorrow",
            TimePeriod::Last7Days => "last7Days",
            TimePeriod::LastWeek => "lastWeek",
            TimePeriod::ThisWeek => "thisWeek",
            TimePeriod::NextWeek => "nextWeek",
            TimePeriod::LastMonth => "lastMonth",
            TimePeriod::ThisMonth => "thisMonth",
            TimePeriod::NextMonth => "nextMonth",
        }
    }

    pub fn from_ooxml(s: &str) -> Option<TimePeriod> {
        TimePeriod::ALL.into_iter().find(|p| p.ooxml() == s)
    }
}

/// A threshold of a colour scale, a data bar or an icon set (`cfvo`).
/// A value is a number or a formula; a formula is evaluated once, for the
/// rule's anchor cell (Excel refuses relative references here).
#[derive(Clone, Debug, PartialEq)]
pub enum Cfvo {
    /// The lowest number of the ranges.
    Min,
    /// The highest number of the ranges.
    Max,
    /// Data bars only (`autoMin` / `autoMax`): the lowest number or zero,
    /// whichever is smaller, for the short end; the highest number or zero,
    /// whichever is larger, for the long end.
    Automatic,
    Number(Expr),
    /// A percentage of the way from the lowest to the highest number.
    Percent(Expr),
    /// A percentile of the numbers (0 to 100, as `PERCENTILE.INC`).
    Percentile(Expr),
    Formula(Expr),
}

impl Cfvo {
    pub fn number(n: f64) -> Cfvo {
        Cfvo::Number(Expr::Number(n))
    }

    pub fn percent(p: f64) -> Cfvo {
        Cfvo::Percent(Expr::Number(p))
    }

    pub fn percentile(p: f64) -> Cfvo {
        Cfvo::Percentile(Expr::Number(p))
    }

    /// `cfvo/@type`. `low_end` tells the two automatic spellings apart: it
    /// is true for the first threshold of a data bar.
    pub fn ooxml_type(&self, low_end: bool) -> &'static str {
        match self {
            Cfvo::Min => "min",
            Cfvo::Max => "max",
            Cfvo::Automatic if low_end => "autoMin",
            Cfvo::Automatic => "autoMax",
            Cfvo::Number(_) => "num",
            Cfvo::Percent(_) => "percent",
            Cfvo::Percentile(_) => "percentile",
            Cfvo::Formula(_) => "formula",
        }
    }

    /// The threshold for a `cfvo/@type` and its value (`@val`, or `xm:f` in
    /// the x14 form). `None` for an unknown type or a missing value.
    pub fn from_ooxml(ty: &str, value: Option<Expr>) -> Option<Cfvo> {
        Some(match ty {
            "min" => Cfvo::Min,
            "max" => Cfvo::Max,
            "autoMin" | "autoMax" => Cfvo::Automatic,
            "num" => Cfvo::Number(value?),
            "percent" => Cfvo::Percent(value?),
            "percentile" => Cfvo::Percentile(value?),
            "formula" => Cfvo::Formula(value?),
            _ => return None,
        })
    }

    /// The number or formula of the threshold, when it has one.
    pub fn value(&self) -> Option<&Expr> {
        match self {
            Cfvo::Number(e) | Cfvo::Percent(e) | Cfvo::Percentile(e) | Cfvo::Formula(e) => Some(e),
            Cfvo::Min | Cfvo::Max | Cfvo::Automatic => None,
        }
    }

    fn map_expr(&mut self, f: &mut dyn FnMut(&Expr) -> Expr) {
        match self {
            Cfvo::Number(e) | Cfvo::Percent(e) | Cfvo::Percentile(e) | Cfvo::Formula(e) => {
                *e = f(e)
            }
            Cfvo::Min | Cfvo::Max | Cfvo::Automatic => {}
        }
    }
}

/// One point of a colour scale: a threshold and the colour cells have there.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorStop {
    pub at: Cfvo,
    pub color: Color,
}

/// `colorScale`: the fill of every number is interpolated between the
/// colours of the stops around it. Two or three stops, lowest first.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorScale {
    pub stops: Vec<ColorStop>,
}

/// `x14:dataBar/@axisPosition`: where bars of negative numbers go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BarAxis {
    /// The axis sits where zero falls between the two ends; no axis when
    /// there is no negative number.
    #[default]
    Automatic,
    /// The axis is in the middle of the cell.
    Midpoint,
    /// No axis: negative bars grow in the same direction as positive ones.
    None,
}

impl BarAxis {
    pub const ALL: [BarAxis; 3] = [BarAxis::Automatic, BarAxis::Midpoint, BarAxis::None];

    pub fn ooxml(self) -> &'static str {
        match self {
            BarAxis::Automatic => "automatic",
            BarAxis::Midpoint => "middle",
            BarAxis::None => "none",
        }
    }

    pub fn from_ooxml(s: &str) -> Option<BarAxis> {
        BarAxis::ALL.into_iter().find(|a| a.ooxml() == s)
    }
}

/// `x14:dataBar/@direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BarDirection {
    /// The reading direction of the sheet (left to right).
    #[default]
    Context,
    LeftToRight,
    RightToLeft,
}

impl BarDirection {
    pub const ALL: [BarDirection; 3] = [
        BarDirection::Context,
        BarDirection::LeftToRight,
        BarDirection::RightToLeft,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            BarDirection::Context => "context",
            BarDirection::LeftToRight => "leftToRight",
            BarDirection::RightToLeft => "rightToLeft",
        }
    }

    pub fn from_ooxml(s: &str) -> Option<BarDirection> {
        BarDirection::ALL.into_iter().find(|d| d.ooxml() == s)
    }
}

/// `dataBar` with its `x14:dataBar` extension: a bar in every cell holding
/// a number, as long as the number is large.
#[derive(Clone, Debug, PartialEq)]
pub struct DataBar {
    /// The number shown with the shortest bar.
    pub min: Cfvo,
    /// The number shown with the longest bar.
    pub max: Cfvo,
    pub fill: Color,
    /// The bar fades towards its tip instead of being a solid block.
    pub gradient: bool,
    /// Outline of the bar; `None` = no border.
    pub border: Option<Color>,
    /// Fill of the bars of negative numbers; `None` = same as `fill`.
    pub negative_fill: Option<Color>,
    /// Outline of the bars of negative numbers (when `border` is set);
    /// `None` = same as `border`.
    pub negative_border: Option<Color>,
    pub axis: BarAxis,
    pub axis_color: Color,
    pub direction: BarDirection,
    /// Length of the shortest and of the longest bar, in percent of the
    /// cell width (10 and 90 in files written by Excel 2007, 0 and 100
    /// since).
    pub min_length: u8,
    pub max_length: u8,
    /// False = "Show Bar Only": the cell's value is not drawn.
    pub show_value: bool,
}

impl DataBar {
    /// A data bar as Excel 2007 wrote them, for a `dataBar` element that
    /// has no x14 extension: gradient, no border, no axis, bars between
    /// 10 % and 90 % of the cell from the lowest to the highest number.
    pub fn legacy(fill: Color) -> DataBar {
        DataBar {
            min: Cfvo::Min,
            max: Cfvo::Max,
            fill,
            gradient: true,
            border: None,
            negative_fill: None,
            negative_border: None,
            axis: BarAxis::None,
            axis_color: Color::BLACK,
            direction: BarDirection::Context,
            min_length: 10,
            max_length: 90,
            show_value: true,
        }
    }
}

/// One threshold of an icon set: the icon is used from this value up.
#[derive(Clone, Debug, PartialEq)]
pub struct IconThreshold {
    pub value: Cfvo,
    /// `cfvo/@gte`: true = "value >= threshold", false = "value > threshold".
    pub gte: bool,
}

/// `iconSet` (and `x14:iconSet`): an icon in every cell holding a number,
/// chosen by the band the number falls in.
#[derive(Clone, Debug, PartialEq)]
pub struct IconRule {
    pub set: IconSet,
    /// One per icon, lowest band first. The first is the floor of the
    /// lowest band and is not looked at (Excel writes 0 percent).
    pub thresholds: Vec<IconThreshold>,
    /// The icons are used in the opposite order.
    pub reverse: bool,
    /// False = "Show Icon Only": the cell's value is not drawn.
    pub show_value: bool,
    /// Icons picked one by one (`x14:cfIcon`), one per band, lowest band
    /// first: a set and an index into it, or `None` for "No Cell Icon".
    pub custom: Option<Vec<Option<(IconSet, u8)>>>,
}

/// `iconSet/@iconSet`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconSet {
    ThreeArrows,
    ThreeArrowsGray,
    ThreeFlags,
    ThreeTrafficLights1,
    ThreeTrafficLights2,
    ThreeSigns,
    ThreeSymbols,
    ThreeSymbols2,
    ThreeStars,
    ThreeTriangles,
    FourArrows,
    FourArrowsGray,
    FourRedToBlack,
    FourRating,
    FourTrafficLights,
    FiveArrows,
    FiveArrowsGray,
    FiveRating,
    FiveQuarters,
    FiveBoxes,
}

impl IconSet {
    /// In the order of the ribbon gallery: Directional (7), Shapes (5),
    /// Indicators (3), Ratings (5).
    pub const ALL: [IconSet; 20] = [
        IconSet::ThreeArrows,
        IconSet::ThreeArrowsGray,
        IconSet::ThreeTriangles,
        IconSet::FourArrowsGray,
        IconSet::FourArrows,
        IconSet::FiveArrowsGray,
        IconSet::FiveArrows,
        IconSet::ThreeTrafficLights1,
        IconSet::ThreeTrafficLights2,
        IconSet::ThreeSigns,
        IconSet::FourTrafficLights,
        IconSet::FourRedToBlack,
        IconSet::ThreeSymbols,
        IconSet::ThreeSymbols2,
        IconSet::ThreeFlags,
        IconSet::ThreeStars,
        IconSet::FourRating,
        IconSet::FiveQuarters,
        IconSet::FiveRating,
        IconSet::FiveBoxes,
    ];

    pub fn ooxml(self) -> &'static str {
        match self {
            IconSet::ThreeArrows => "3Arrows",
            IconSet::ThreeArrowsGray => "3ArrowsGray",
            IconSet::ThreeFlags => "3Flags",
            IconSet::ThreeTrafficLights1 => "3TrafficLights1",
            IconSet::ThreeTrafficLights2 => "3TrafficLights2",
            IconSet::ThreeSigns => "3Signs",
            IconSet::ThreeSymbols => "3Symbols",
            IconSet::ThreeSymbols2 => "3Symbols2",
            IconSet::ThreeStars => "3Stars",
            IconSet::ThreeTriangles => "3Triangles",
            IconSet::FourArrows => "4Arrows",
            IconSet::FourArrowsGray => "4ArrowsGray",
            IconSet::FourRedToBlack => "4RedToBlack",
            IconSet::FourRating => "4Rating",
            IconSet::FourTrafficLights => "4TrafficLights",
            IconSet::FiveArrows => "5Arrows",
            IconSet::FiveArrowsGray => "5ArrowsGray",
            IconSet::FiveRating => "5Rating",
            IconSet::FiveQuarters => "5Quarters",
            IconSet::FiveBoxes => "5Boxes",
        }
    }

    /// `None` for an unknown name and for "NoIcons" (which only exists as
    /// the set of a custom icon and is stored as `None` there).
    pub fn from_ooxml(s: &str) -> Option<IconSet> {
        IconSet::ALL.into_iter().find(|set| set.ooxml() == s)
    }

    /// Sets that only exist in the x14 extension (Excel 2010 and later).
    pub fn is_x14(self) -> bool {
        matches!(
            self,
            IconSet::ThreeStars | IconSet::ThreeTriangles | IconSet::FiveBoxes
        )
    }

    /// The icons of the set, the one for the lowest numbers first (the
    /// order of `x14:cfIcon/@iconId`).
    pub fn icons(self) -> &'static [Icon] {
        use Icon::*;
        match self {
            IconSet::ThreeArrows => &[RedDownArrow, YellowSideArrow, GreenUpArrow],
            IconSet::ThreeArrowsGray => &[GrayDownArrow, GraySideArrow, GrayUpArrow],
            IconSet::ThreeFlags => &[RedFlag, YellowFlag, GreenFlag],
            IconSet::ThreeTrafficLights1 => &[RedCircleWithBorder, YellowCircle, GreenCircle],
            IconSet::ThreeTrafficLights2 => {
                &[RedTrafficLight, YellowTrafficLight, GreenTrafficLight]
            }
            IconSet::ThreeSigns => &[RedDiamond, YellowTriangle, GreenCircle],
            IconSet::ThreeSymbols => &[RedCrossSymbol, YellowExclamationSymbol, GreenCheckSymbol],
            IconSet::ThreeSymbols2 => &[RedCross, YellowExclamation, GreenCheck],
            IconSet::ThreeStars => &[SilverStar, HalfGoldStar, GoldStar],
            IconSet::ThreeTriangles => &[RedDownTriangle, YellowDash, GreenUpTriangle],
            IconSet::FourArrows => &[
                RedDownArrow,
                YellowDownInclineArrow,
                YellowUpInclineArrow,
                GreenUpArrow,
            ],
            IconSet::FourArrowsGray => &[
                GrayDownArrow,
                GrayDownInclineArrow,
                GrayUpInclineArrow,
                GrayUpArrow,
            ],
            IconSet::FourRedToBlack => &[BlackCircle, GrayCircle, PinkCircle, RedCircle],
            IconSet::FourRating => &[OneBar, TwoBars, ThreeBars, FourBars],
            IconSet::FourTrafficLights => &[
                BlackCircleWithBorder,
                RedCircleWithBorder,
                YellowCircle,
                GreenCircle,
            ],
            IconSet::FiveArrows => &[
                RedDownArrow,
                YellowDownInclineArrow,
                YellowSideArrow,
                YellowUpInclineArrow,
                GreenUpArrow,
            ],
            IconSet::FiveArrowsGray => &[
                GrayDownArrow,
                GrayDownInclineArrow,
                GraySideArrow,
                GrayUpInclineArrow,
                GrayUpArrow,
            ],
            IconSet::FiveRating => &[ZeroBars, OneBar, TwoBars, ThreeBars, FourBars],
            IconSet::FiveQuarters => &[
                WhiteCircle,
                CircleWithThreeWhiteQuarters,
                CircleWithTwoWhiteQuarters,
                CircleWithOneWhiteQuarter,
                BlackCircle,
            ],
            IconSet::FiveBoxes => &[
                ZeroFilledBoxes,
                OneFilledBox,
                TwoFilledBoxes,
                ThreeFilledBoxes,
                FourFilledBoxes,
            ],
        }
    }

    /// Number of icons (3, 4 or 5).
    pub fn len(self) -> usize {
        self.icons().len()
    }

    pub fn is_empty(self) -> bool {
        false
    }
}

/// Every picture an icon set can put in a cell (Excel's `XlIcon`). The
/// colour is part of the picture: the grid draws each of these one way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    GreenUpArrow,
    YellowSideArrow,
    RedDownArrow,
    GrayUpArrow,
    GraySideArrow,
    GrayDownArrow,
    GreenFlag,
    YellowFlag,
    RedFlag,
    /// A plain green disc.
    GreenCircle,
    /// A plain yellow disc.
    YellowCircle,
    /// A red disc with a darker rim.
    RedCircleWithBorder,
    /// A black disc with a rim.
    BlackCircleWithBorder,
    /// A green light in a dark rounded square.
    GreenTrafficLight,
    YellowTrafficLight,
    RedTrafficLight,
    /// A yellow warning triangle.
    YellowTriangle,
    /// A red diamond.
    RedDiamond,
    /// A white check mark on a green disc.
    GreenCheckSymbol,
    /// A white exclamation mark on a yellow disc.
    YellowExclamationSymbol,
    /// A white cross on a red disc.
    RedCrossSymbol,
    /// A bare green check mark.
    GreenCheck,
    /// A bare yellow exclamation mark.
    YellowExclamation,
    /// A bare red cross.
    RedCross,
    /// A yellow arrow pointing up and to the right.
    YellowUpInclineArrow,
    /// A yellow arrow pointing down and to the right.
    YellowDownInclineArrow,
    GrayUpInclineArrow,
    GrayDownInclineArrow,
    RedCircle,
    PinkCircle,
    GrayCircle,
    BlackCircle,
    /// A disc three quarters black.
    CircleWithOneWhiteQuarter,
    /// A disc half black.
    CircleWithTwoWhiteQuarters,
    /// A disc one quarter black.
    CircleWithThreeWhiteQuarters,
    /// An empty disc.
    WhiteCircle,
    /// A signal-strength meter of four bars, none filled.
    ZeroBars,
    OneBar,
    TwoBars,
    ThreeBars,
    FourBars,
    GoldStar,
    HalfGoldStar,
    /// An empty star.
    SilverStar,
    GreenUpTriangle,
    /// A yellow horizontal dash.
    YellowDash,
    RedDownTriangle,
    /// Four small squares in a 2 × 2 block, all filled.
    FourFilledBoxes,
    ThreeFilledBoxes,
    TwoFilledBoxes,
    OneFilledBox,
    ZeroFilledBoxes,
}
