//! What the ribbon's Conditional Formatting menus create: the formats of
//! the Highlight Cells dialogs, the Top/Bottom rules and the three
//! galleries (data bars, colour scales, icon sets), with Excel's colours.

use super::model::{
    BarAxis, BarDirection, CellOp, Cfvo, ColorScale, ColorStop, DataBar, IconRule, IconSet,
    IconThreshold, Rule, TextOp, TimePeriod,
};
use crate::ast::Expr;
use crate::refs::Rect;
use crate::style::{Border, BorderStyle, Color, StylePatch};
use crate::workbook::Workbook;

/// The formats offered by the "with" list of the Highlight Cells and
/// Top/Bottom dialogs, in the order of the list. The first is the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightFormat {
    LightRedFillDarkRedText,
    YellowFillDarkYellowText,
    GreenFillDarkGreenText,
    LightRedFill,
    RedText,
    RedBorder,
}

impl HighlightFormat {
    pub const ALL: [HighlightFormat; 6] = [
        HighlightFormat::LightRedFillDarkRedText,
        HighlightFormat::YellowFillDarkYellowText,
        HighlightFormat::GreenFillDarkGreenText,
        HighlightFormat::LightRedFill,
        HighlightFormat::RedText,
        HighlightFormat::RedBorder,
    ];

    pub fn patch(self) -> StylePatch {
        let dark_red = Color(0x9C0006);
        let light_red = Color(0xFFC7CE);
        let filled = |fill: Color, text: Color| StylePatch {
            fill: Some(Some(fill)),
            font_color: Some(Some(text)),
            ..StylePatch::default()
        };
        match self {
            HighlightFormat::LightRedFillDarkRedText => filled(light_red, dark_red),
            HighlightFormat::YellowFillDarkYellowText => filled(Color(0xFFEB9C), Color(0x9C5700)),
            HighlightFormat::GreenFillDarkGreenText => filled(Color(0xC6EFCE), Color(0x006100)),
            HighlightFormat::LightRedFill => StylePatch {
                fill: Some(Some(light_red)),
                ..StylePatch::default()
            },
            HighlightFormat::RedText => StylePatch {
                font_color: Some(Some(dark_red)),
                ..StylePatch::default()
            },
            HighlightFormat::RedBorder => {
                let line = Some(Border {
                    style: BorderStyle::Thin,
                    color: Some(dark_red),
                });
                StylePatch {
                    border_left: line,
                    border_right: line,
                    border_top: line,
                    border_bottom: line,
                    ..StylePatch::default()
                }
            }
        }
    }
}

/// The rules of the Highlight Cells Rules and Top/Bottom Rules menus.
/// Operands are number or text constants, or formulas written for the
/// rule's anchor cell (see `parse_operand`).
impl Rule {
    pub fn cell_is(op: CellOp, first: Expr, second: Option<Expr>) -> Rule {
        Rule::CellIs { op, first, second }
    }

    /// "Greater Than..."
    pub fn greater_than(value: Expr) -> Rule {
        Rule::cell_is(CellOp::Greater, value, None)
    }

    /// "Less Than..."
    pub fn less_than(value: Expr) -> Rule {
        Rule::cell_is(CellOp::Less, value, None)
    }

    /// "Between..."
    pub fn between(low: Expr, high: Expr) -> Rule {
        Rule::cell_is(CellOp::Between, low, Some(high))
    }

    /// "Equal To..."
    pub fn equal_to(value: Expr) -> Rule {
        Rule::cell_is(CellOp::Equal, value, None)
    }

    /// "Text that Contains..."
    pub fn text_contains(text: Expr) -> Rule {
        Rule::Text {
            op: TextOp::Contains,
            text,
        }
    }

    /// "A Date Occurring..."
    pub fn date_occurring(period: TimePeriod) -> Rule {
        Rule::DateOccurring(period)
    }

    /// "Top 10 Items..."
    pub fn top_items(rank: u32) -> Rule {
        Rule::TopBottom {
            rank,
            percent: false,
            bottom: false,
        }
    }

    /// "Top 10 %..."
    pub fn top_percent(rank: u32) -> Rule {
        Rule::TopBottom {
            rank,
            percent: true,
            bottom: false,
        }
    }

    /// "Bottom 10 Items..."
    pub fn bottom_items(rank: u32) -> Rule {
        Rule::TopBottom {
            rank,
            percent: false,
            bottom: true,
        }
    }

    /// "Bottom 10 %..."
    pub fn bottom_percent(rank: u32) -> Rule {
        Rule::TopBottom {
            rank,
            percent: true,
            bottom: true,
        }
    }

    /// "Above Average..."
    pub fn above_average() -> Rule {
        Rule::Average {
            above: true,
            or_equal: false,
            std_dev: 0,
        }
    }

    /// "Below Average..."
    pub fn below_average() -> Rule {
        Rule::Average {
            above: false,
            or_equal: false,
            std_dev: 0,
        }
    }
}

/// The lowest and the highest number in the cells of `ranges`: what the
/// Highlight Cells dialogs work their proposed values out from (the middle
/// of the two for "Greater Than", three and seven tenths of the way for
/// "Between"). `None` when the cells hold no number.
pub fn number_span(wb: &Workbook, sheet: usize, ranges: &[Rect]) -> Option<(f64, f64)> {
    let numbers = super::eval::numbers(wb.sheets.get(sheet)?, ranges);
    let low = numbers.iter().copied().min_by(f64::total_cmp)?;
    let high = numbers.iter().copied().max_by(f64::total_cmp)?;
    Some((low, high))
}

/// The six colours of the data bar gallery, in gallery order (the same for
/// the Gradient Fill and the Solid Fill rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BarColor {
    Blue,
    Green,
    Red,
    Orange,
    LightBlue,
    Purple,
}

impl BarColor {
    pub const ALL: [BarColor; 6] = [
        BarColor::Blue,
        BarColor::Green,
        BarColor::Red,
        BarColor::Orange,
        BarColor::LightBlue,
        BarColor::Purple,
    ];

    pub fn color(self) -> Color {
        Color(match self {
            BarColor::Blue => 0x638EC6,
            BarColor::Green => 0x63C384,
            BarColor::Red => 0xFF555A,
            BarColor::Orange => 0xFFB628,
            BarColor::LightBlue => 0x008AEF,
            BarColor::Purple => 0xD6007B,
        })
    }
}

impl DataBar {
    /// A data bar of the ribbon gallery: automatic ends, red bars for
    /// negative numbers on the other side of a black axis. Gradient bars
    /// are outlined in their own colour, solid bars have no border.
    pub fn preset(color: BarColor, gradient: bool) -> DataBar {
        let fill = color.color();
        let red = Color(0xFF0000);
        DataBar {
            min: Cfvo::Automatic,
            max: Cfvo::Automatic,
            fill,
            gradient,
            border: gradient.then_some(fill),
            negative_fill: Some(red),
            negative_border: gradient.then_some(red),
            axis: BarAxis::Automatic,
            axis_color: Color::BLACK,
            direction: BarDirection::Context,
            min_length: 0,
            max_length: 100,
            show_value: true,
        }
    }
}

/// The twelve colour scales of the ribbon gallery, row by row. The name
/// lists the colours from the highest numbers to the lowest, as Excel's
/// tooltips do ("Green - Yellow - Red Color Scale").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalePreset {
    GreenYellowRed,
    RedYellowGreen,
    GreenWhiteRed,
    RedWhiteGreen,
    BlueWhiteRed,
    RedWhiteBlue,
    WhiteRed,
    RedWhite,
    GreenWhite,
    WhiteGreen,
    GreenYellow,
    YellowGreen,
}

impl ScalePreset {
    pub const ALL: [ScalePreset; 12] = [
        ScalePreset::GreenYellowRed,
        ScalePreset::RedYellowGreen,
        ScalePreset::GreenWhiteRed,
        ScalePreset::RedWhiteGreen,
        ScalePreset::BlueWhiteRed,
        ScalePreset::RedWhiteBlue,
        ScalePreset::WhiteRed,
        ScalePreset::RedWhite,
        ScalePreset::GreenWhite,
        ScalePreset::WhiteGreen,
        ScalePreset::GreenYellow,
        ScalePreset::YellowGreen,
    ];

    /// The colours from the lowest numbers to the highest (two or three).
    pub fn colors(self) -> Vec<Color> {
        const RED: u32 = 0xF8696B;
        const YELLOW: u32 = 0xFFEB84;
        const GREEN: u32 = 0x63BE7B;
        const WHITE: u32 = 0xFCFCFF;
        const BLUE: u32 = 0x5A8AC6;
        // The yellow of the two-colour scales is paler than the midpoint
        // yellow of the three-colour ones.
        const PALE_YELLOW: u32 = 0xFFEF9C;
        let rgb: &[u32] = match self {
            ScalePreset::GreenYellowRed => &[RED, YELLOW, GREEN],
            ScalePreset::RedYellowGreen => &[GREEN, YELLOW, RED],
            ScalePreset::GreenWhiteRed => &[RED, WHITE, GREEN],
            ScalePreset::RedWhiteGreen => &[GREEN, WHITE, RED],
            ScalePreset::BlueWhiteRed => &[RED, WHITE, BLUE],
            ScalePreset::RedWhiteBlue => &[BLUE, WHITE, RED],
            ScalePreset::WhiteRed => &[RED, WHITE],
            ScalePreset::RedWhite => &[WHITE, RED],
            ScalePreset::GreenWhite => &[WHITE, GREEN],
            ScalePreset::WhiteGreen => &[GREEN, WHITE],
            ScalePreset::GreenYellow => &[PALE_YELLOW, GREEN],
            ScalePreset::YellowGreen => &[GREEN, PALE_YELLOW],
        };
        rgb.iter().map(|c| Color(*c)).collect()
    }

    pub fn scale(self) -> ColorScale {
        let colors = self.colors();
        match colors[..] {
            [min, mid, max] => ColorScale::three(min, mid, max),
            _ => ColorScale::two(colors[0], colors[1]),
        }
    }
}

impl ColorScale {
    /// From the lowest number to the highest.
    pub fn two(min: Color, max: Color) -> ColorScale {
        ColorScale {
            stops: vec![
                ColorStop {
                    at: Cfvo::Min,
                    color: min,
                },
                ColorStop {
                    at: Cfvo::Max,
                    color: max,
                },
            ],
        }
    }

    /// From the lowest number to the highest, through the median.
    pub fn three(min: Color, mid: Color, max: Color) -> ColorScale {
        ColorScale {
            stops: vec![
                ColorStop {
                    at: Cfvo::Min,
                    color: min,
                },
                ColorStop {
                    at: Cfvo::percentile(50.0),
                    color: mid,
                },
                ColorStop {
                    at: Cfvo::Max,
                    color: max,
                },
            ],
        }
    }
}

impl IconRule {
    /// An icon set of the ribbon gallery: bands of equal width between the
    /// lowest and the highest number (percent 0 / 33 / 67, 0 / 25 / 50 /
    /// 75 or 0 / 20 / 40 / 60 / 80).
    pub fn preset(set: IconSet) -> IconRule {
        let percents: &[f64] = match set.len() {
            3 => &[0.0, 33.0, 67.0],
            4 => &[0.0, 25.0, 50.0, 75.0],
            _ => &[0.0, 20.0, 40.0, 60.0, 80.0],
        };
        IconRule {
            set,
            thresholds: percents
                .iter()
                .map(|p| IconThreshold {
                    value: Cfvo::percent(*p),
                    gte: true,
                })
                .collect(),
            reverse: false,
            show_value: true,
            custom: None,
        }
    }
}
