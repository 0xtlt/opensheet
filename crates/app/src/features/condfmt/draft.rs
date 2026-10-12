//! A rule as the New / Edit Formatting Rule dialog holds it: every choice
//! of every rule type side by side, with the values as the text typed in
//! their boxes. [`Draft::of`] reads a rule into it and [`Draft::rule`]
//! makes the rule back, checking what Excel checks.

use super::words;
use crate::i18n::t;
use opensheet_engine::condfmt::{
    BarColor, CellOp, Cfvo, ColorScale, ColorStop, CondFormat, DataBar, IconRule, IconSet, IconThreshold, Rule,
    ScalePreset, TextOp, TimePeriod, is_relative, operand_text, parse_operand, parse_text_operand,
};
use opensheet_engine::style::{Color, StylePatch};
use opensheet_engine::{Expr, Locale};

/// The six rule types of the dialog's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Values,
    Contains,
    Ranked,
    Average,
    Unique,
    Formula,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Values,
        Kind::Contains,
        Kind::Ranked,
        Kind::Average,
        Kind::Unique,
        Kind::Formula,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Values => t(
                "Format all cells based on their values",
                "Appliquer une mise en forme à toutes les cellules d'après leur valeur",
            ),
            Kind::Contains => t(
                "Format only cells that contain",
                "Appliquer une mise en forme uniquement aux cellules contenant",
            ),
            Kind::Ranked => t(
                "Format only top or bottom ranked values",
                "Appliquer une mise en forme uniquement aux valeurs situées dans la plage haute/basse",
            ),
            Kind::Average => t(
                "Format only values that are above or below average",
                "Appliquer une mise en forme uniquement aux valeurs au-dessus ou en dessous de la moyenne",
            ),
            Kind::Unique => t(
                "Format only unique or duplicate values",
                "Appliquer une mise en forme uniquement aux valeurs uniques ou en double",
            ),
            Kind::Formula => t(
                "Use a formula to determine which cells to format",
                "Utiliser une formule pour déterminer pour quelles cellules le format sera appliqué",
            ),
        }
    }
}

/// The "Format Style" of a rule based on values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Graded {
    TwoColor,
    ThreeColor,
    DataBar,
    IconSet,
}

impl Graded {
    pub const ALL: [Graded; 4] = [Graded::TwoColor, Graded::ThreeColor, Graded::DataBar, Graded::IconSet];

    pub fn name(self) -> &'static str {
        match self {
            Graded::TwoColor => t("2-Color Scale", "Échelle à deux couleurs"),
            Graded::ThreeColor => t("3-Color Scale", "Échelle à trois couleurs"),
            Graded::DataBar => t("Data Bar", "Barre de données"),
            Graded::IconSet => t("Icon Sets", "Jeux d'icônes"),
        }
    }
}

/// What "Format only cells with" tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contains {
    CellValue,
    Text,
    Dates,
    Blanks,
    NoBlanks,
    Errors,
    NoErrors,
}

impl Contains {
    pub const ALL: [Contains; 7] = [
        Contains::CellValue,
        Contains::Text,
        Contains::Dates,
        Contains::Blanks,
        Contains::NoBlanks,
        Contains::Errors,
        Contains::NoErrors,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Contains::CellValue => t("Cell Value", "Valeur de la cellule"),
            Contains::Text => t("Specific Text", "Texte spécifique"),
            Contains::Dates => t("Dates Occurring", "Dates se produisant"),
            Contains::Blanks => t("Blanks", "Cellules vides"),
            Contains::NoBlanks => t("No Blanks", "Aucune cellule vide"),
            Contains::Errors => t("Errors", "Erreurs"),
            Contains::NoErrors => t("No Errors", "Aucune erreur"),
        }
    }
}

/// The "Type" of a threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Lowest,
    Highest,
    Automatic,
    Number,
    Percent,
    Formula,
    Percentile,
}

impl At {
    /// The types offered for the low end, the middle and the high end of a
    /// colour scale.
    pub const SCALE_LOW: [At; 5] = [At::Lowest, At::Number, At::Percent, At::Formula, At::Percentile];
    pub const MIDDLE: [At; 4] = [At::Number, At::Percent, At::Formula, At::Percentile];
    pub const SCALE_HIGH: [At; 5] = [At::Highest, At::Number, At::Percent, At::Formula, At::Percentile];
    /// The types offered for the shortest and the longest data bar.
    pub const BAR_LOW: [At; 6] = [
        At::Lowest,
        At::Number,
        At::Percent,
        At::Formula,
        At::Percentile,
        At::Automatic,
    ];
    pub const BAR_HIGH: [At; 6] = [
        At::Highest,
        At::Number,
        At::Percent,
        At::Formula,
        At::Percentile,
        At::Automatic,
    ];

    pub fn name(self) -> &'static str {
        match self {
            At::Lowest => t("Lowest Value", "Valeur inférieure"),
            At::Highest => t("Highest Value", "Valeur supérieure"),
            At::Automatic => t("Automatic", "Automatique"),
            At::Number => t("Number", "Nombre"),
            At::Percent => t("Percent", "Pourcentage"),
            At::Formula => t("Formula", "Formule"),
            At::Percentile => t("Percentile", "Centile"),
        }
    }

    /// Whether the type comes with a value to type.
    pub fn has_value(self) -> bool {
        !matches!(self, At::Lowest | At::Highest | At::Automatic)
    }

    /// What the value box shows for a type without a value.
    pub fn placeholder(self) -> &'static str {
        match self {
            At::Lowest => t("(Lowest value)", "(Valeur inférieure)"),
            At::Highest => t("(Highest value)", "(Valeur supérieure)"),
            At::Automatic => t("(Automatic)", "(Automatique)"),
            _ => "",
        }
    }

    /// The value Excel puts in the box when the type is chosen, for the
    /// low end (0), the middle (1) or the high end (2).
    pub fn default_value(self, place: usize) -> &'static str {
        match (self, place) {
            (At::Number, _) => "0",
            (At::Percent, 0) => "0",
            (At::Percent, 1) => "50",
            (At::Percent, _) => "100",
            (At::Percentile, 0) => "10",
            (At::Percentile, 1) => "50",
            (At::Percentile, _) => "90",
            _ => "",
        }
    }
}

/// A threshold: its type and the text of its value box.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub at: At,
    pub value: String,
}

impl Level {
    pub fn new(at: At, value: &str) -> Level {
        Level {
            at,
            value: value.to_string(),
        }
    }

    /// The threshold as the dialog shows it; formulas read from `active`.
    fn of(cfvo: &Cfvo, locale: &Locale, anchor: (u32, u32), active: (u32, u32)) -> Level {
        let text = |e: &Expr| operand_text(e, locale, anchor, active, false);
        match cfvo {
            Cfvo::Min => Level::new(At::Lowest, ""),
            Cfvo::Max => Level::new(At::Highest, ""),
            Cfvo::Automatic => Level::new(At::Automatic, ""),
            Cfvo::Number(e) => Level::new(At::Number, &text(e)),
            Cfvo::Percent(e) => Level::new(At::Percent, &text(e)),
            Cfvo::Percentile(e) => Level::new(At::Percentile, &text(e)),
            Cfvo::Formula(e) => Level::new(At::Formula, &text(e)),
        }
    }

    /// The number typed, when the value is a plain number.
    fn number(&self, locale: &Locale) -> Option<f64> {
        locale.parse_number(self.value.trim())
    }

    fn cfvo(&self, locale: &Locale, active: (u32, u32), anchor: (u32, u32)) -> Result<Cfvo, String> {
        if !self.at.has_value() {
            return Ok(match self.at {
                At::Lowest => Cfvo::Min,
                At::Highest => Cfvo::Max,
                _ => Cfvo::Automatic,
            });
        }
        let formula = self.value.trim_start().starts_with('=');
        let expr = match parse_operand(&self.value, locale, active, anchor) {
            Ok(expr) => expr,
            Err(_) if formula || self.at == At::Formula => return Err(enter_valid_formula().to_string()),
            Err(_) => return Err(not_a_number().to_string()),
        };
        if is_relative(&expr) {
            return Err(t(
                "You cannot use relative references in Conditional Formatting criteria for color scales, data bars, and icon sets.",
                "Vous ne pouvez pas utiliser de références relatives dans les critères Mise en forme conditionnelle pour des nuances de couleurs, des barres de données et des jeux d'icônes.",
            )
            .to_string());
        }
        let number = match &expr {
            Expr::Number(n) => Some(*n),
            _ => None,
        };
        match self.at {
            At::Formula if !formula => Err(enter_valid_formula().to_string()),
            At::Formula => Ok(Cfvo::Formula(expr)),
            _ if !formula && number.is_none() => Err(not_a_number().to_string()),
            At::Percent if number.is_some_and(|n| !(0.0..=100.0).contains(&n)) => Err(t(
                "One or more specified values are not valid percentages.",
                "Une ou plusieurs valeurs spécifiées ne correspondent pas à un pourcentage valide.",
            )
            .to_string()),
            At::Percent => Ok(Cfvo::Percent(expr)),
            At::Percentile if number.is_some_and(|n| !(0.0..=100.0).contains(&n)) => Err(t(
                "One or more specified values are not valid percentiles.",
                "Une ou plusieurs valeurs spécifiées ne correspondent pas à un centile valide.",
            )
            .to_string()),
            At::Percentile => Ok(Cfvo::Percentile(expr)),
            _ => Ok(Cfvo::Number(expr)),
        }
    }
}

fn enter_valid_formula() -> &'static str {
    t("Enter a valid formula.", "Entrez une formule valide.")
}

fn not_a_number() -> &'static str {
    t(
        "The value you entered is not a valid number, date or time.",
        "La valeur entrée n'est pas un nombre, une date ou une heure valide.",
    )
}

/// One band of an icon set above the lowest: "when value is >= 67 Percent".
#[derive(Clone, Debug, PartialEq)]
pub struct IconRow {
    /// `>=` rather than `>`.
    pub gte: bool,
    pub level: Level,
}

/// Everything the dialog holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub kind: Kind,

    // Format all cells based on their values
    pub style: Graded,
    /// Low end, middle and high end of a colour scale (the middle is only
    /// used by three-colour scales).
    pub stops: [Level; 3],
    pub colors: [Color; 3],
    /// The data bar, but for its two ends.
    pub bar: DataBar,
    pub bar_low: Level,
    pub bar_high: Level,
    pub set: IconSet,
    pub show_icon_only: bool,
    /// The icon of each band, lowest band first; `None` is "No Cell Icon".
    pub icons: Vec<Option<(IconSet, u8)>>,
    /// The bands above the lowest, lowest first.
    pub icon_rows: Vec<IconRow>,

    // Format only cells that contain
    pub contains: Contains,
    pub cell_op: CellOp,
    pub first: String,
    pub second: String,
    pub text_op: TextOp,
    pub text: String,
    pub period: TimePeriod,

    // Top or bottom ranked values
    pub bottom: bool,
    pub rank: String,
    pub percent: bool,

    /// Position in `words::AVERAGES`.
    pub average: usize,
    pub unique: bool,
    pub formula: String,

    /// The format of the rules that have one.
    pub format: StylePatch,
}

/// The icons of a set, one per band, lowest band first.
pub fn set_icons(set: IconSet) -> Vec<Option<(IconSet, u8)>> {
    (0..set.len()).map(|i| Some((set, i as u8))).collect()
}

/// The thresholds Excel gives a set: bands of equal width, in percent.
fn preset_rows(set: IconSet) -> Vec<IconRow> {
    IconRule::preset(set).thresholds[1..]
        .iter()
        .map(|threshold| IconRow {
            gte: threshold.gte,
            level: Level::of(&threshold.value, Locale::en(), (0, 0), (0, 0)),
        })
        .collect()
}

impl Draft {
    /// What the dialog shows for a new rule.
    pub fn new(kind: Kind, style: Graded) -> Draft {
        let three = ScalePreset::GreenYellowRed.colors();
        let set = IconSet::ThreeTrafficLights1;
        Draft {
            kind,
            style,
            stops: [Level::new(At::Lowest, ""), Level::new(At::Percentile, "50"), Level::new(At::Highest, "")],
            colors: [three[0], three[1], three[2]],
            bar: DataBar::preset(BarColor::Blue, false),
            bar_low: Level::new(At::Automatic, ""),
            bar_high: Level::new(At::Automatic, ""),
            set,
            show_icon_only: false,
            icons: set_icons(set),
            icon_rows: preset_rows(set),
            contains: Contains::CellValue,
            cell_op: CellOp::Between,
            first: String::new(),
            second: String::new(),
            text_op: TextOp::Contains,
            text: String::new(),
            period: TimePeriod::Yesterday,
            bottom: false,
            rank: "10".to_string(),
            percent: false,
            average: 0,
            unique: false,
            formula: String::new(),
            format: StylePatch::default(),
        }
    }

    /// An existing rule as the dialog shows it; formulas read from
    /// `active`.
    pub fn of(cf: &CondFormat, locale: &Locale, active: (u32, u32)) -> Draft {
        let anchor = cf.anchor();
        let text = |e: &Expr, bare: bool| operand_text(e, locale, anchor, active, bare);
        let level = |cfvo: &Cfvo| Level::of(cfvo, locale, anchor, active);
        let mut draft = Draft::new(Kind::Values, Graded::TwoColor);
        draft.format = cf.format.clone();
        match &cf.rule {
            Rule::ColorScale(scale) => {
                let (first, last) = (0, scale.stops.len().saturating_sub(1));
                draft.style = if scale.stops.len() >= 3 { Graded::ThreeColor } else { Graded::TwoColor };
                for (place, index) in [(0, first), (2, last)] {
                    if let Some(stop) = scale.stops.get(index) {
                        draft.stops[place] = level(&stop.at);
                        draft.colors[place] = stop.color;
                    }
                }
                if let (true, Some(stop)) = (scale.stops.len() >= 3, scale.stops.get(1)) {
                    draft.stops[1] = level(&stop.at);
                    draft.colors[1] = stop.color;
                }
            }
            Rule::DataBar(bar) => {
                draft.style = Graded::DataBar;
                draft.bar = bar.clone();
                draft.bar_low = level(&bar.min);
                draft.bar_high = level(&bar.max);
            }
            Rule::IconSet(icons) => {
                draft.style = Graded::IconSet;
                draft.set = icons.set;
                draft.show_icon_only = !icons.show_value;
                // The icon each band ends up with: its own pick, or the
                // icon of the set, counted from the other end when the
                // order is reversed.
                let count = icons.set.len();
                draft.icons = (0..count)
                    .map(|band| {
                        let slot = if icons.reverse { count - 1 - band } else { band };
                        match icons.custom.as_ref().and_then(|custom| custom.get(slot)) {
                            Some(picked) => *picked,
                            None => Some((icons.set, slot as u8)),
                        }
                    })
                    .collect();
                draft.icon_rows = (1..count)
                    .map(|band| match icons.thresholds.get(band) {
                        Some(threshold) => IconRow {
                            gte: threshold.gte,
                            level: level(&threshold.value),
                        },
                        None => IconRow {
                            gte: true,
                            level: Level::new(At::Percent, "0"),
                        },
                    })
                    .collect();
            }
            Rule::CellIs { op, first, second } => {
                draft.kind = Kind::Contains;
                draft.contains = Contains::CellValue;
                draft.cell_op = *op;
                draft.first = text(first, false);
                draft.second = second.as_ref().map(|e| text(e, false)).unwrap_or_default();
            }
            Rule::Text { op, text: operand } => {
                draft.kind = Kind::Contains;
                draft.contains = Contains::Text;
                draft.text_op = *op;
                draft.text = text(operand, true);
            }
            Rule::DateOccurring(period) => {
                draft.kind = Kind::Contains;
                draft.contains = Contains::Dates;
                draft.period = *period;
            }
            Rule::Blanks | Rule::NoBlanks | Rule::Errors | Rule::NoErrors => {
                draft.kind = Kind::Contains;
                draft.contains = match cf.rule {
                    Rule::Blanks => Contains::Blanks,
                    Rule::NoBlanks => Contains::NoBlanks,
                    Rule::Errors => Contains::Errors,
                    _ => Contains::NoErrors,
                };
            }
            Rule::TopBottom { rank, percent, bottom } => {
                draft.kind = Kind::Ranked;
                draft.rank = rank.to_string();
                draft.percent = *percent;
                draft.bottom = *bottom;
            }
            Rule::Average { above, or_equal, std_dev } => {
                draft.kind = Kind::Average;
                draft.average = words::AVERAGES
                    .iter()
                    .position(|choice| *choice == (*above, *or_equal && *std_dev == 0, (*std_dev).min(3)))
                    .unwrap_or(0);
            }
            Rule::DuplicateValues | Rule::UniqueValues => {
                draft.kind = Kind::Unique;
                draft.unique = matches!(cf.rule, Rule::UniqueValues);
            }
            Rule::Formula(e) => {
                draft.kind = Kind::Formula;
                draft.formula = text(e, false);
            }
        }
        draft
    }

    /// Whether the rule comes with a format (the Preview box and the
    /// Format button); rules based on values draw instead.
    pub fn has_format(&self) -> bool {
        self.kind != Kind::Values
    }

    /// Give the bands the icons of another set. The thresholds stay when
    /// the new set has as many icons, else they become the set's own.
    pub fn choose_set(&mut self, set: IconSet) {
        if set.len() != self.icon_rows.len() + 1 {
            self.icon_rows = preset_rows(set);
        }
        self.set = set;
        self.icons = set_icons(set);
    }

    /// The set the "Icon Style" box shows: none once the bands hold icons
    /// that are not those of one set in order.
    pub fn shown_set(&self) -> Option<IconSet> {
        (self.icons == set_icons(self.set)).then_some(self.set)
    }

    /// The rule, or the message Excel shows for what is wrong with it.
    /// Formulas are typed for `active` and stored for `anchor`.
    pub fn rule(&self, locale: &Locale, active: (u32, u32), anchor: (u32, u32)) -> Result<Rule, String> {
        let cfvo = |level: &Level| level.cfvo(locale, active, anchor);
        let no_value = || {
            t(
                "The value you entered is not a valid number, date, time, or string.",
                "La valeur entrée n'est pas un nombre, une date, une heure ou une chaîne valide.",
            )
            .to_string()
        };
        let operand = |text: &str| {
            if text.trim().is_empty() {
                return Err(no_value());
            }
            parse_operand(text, locale, active, anchor).map_err(|_| enter_valid_formula().to_string())
        };
        Ok(match self.kind {
            Kind::Values => match self.style {
                Graded::TwoColor => Rule::ColorScale(ColorScale {
                    stops: vec![
                        ColorStop {
                            at: cfvo(&self.stops[0])?,
                            color: self.colors[0],
                        },
                        ColorStop {
                            at: cfvo(&self.stops[2])?,
                            color: self.colors[2],
                        },
                    ],
                }),
                Graded::ThreeColor => {
                    // Ends of the same type must be in order.
                    let ordered = |low: &Level, high: &Level| match (low.number(locale), high.number(locale)) {
                        (Some(a), Some(b)) if low.at == high.at => a < b,
                        _ => true,
                    };
                    if !ordered(&self.stops[1], &self.stops[2]) {
                        return Err(t(
                            "The value for the maximum must be greater than the value for the midpoint.",
                            "La valeur du champ Maximum doit être supérieure à la valeur du champ Milieu.",
                        )
                        .to_string());
                    }
                    if !ordered(&self.stops[0], &self.stops[1]) {
                        return Err(t(
                            "The value for the midpoint must be greater than the value for the minimum.",
                            "La valeur du champ Milieu doit être supérieure à la valeur du champ Minimum.",
                        )
                        .to_string());
                    }
                    let mut stops = Vec::with_capacity(3);
                    for (level, color) in self.stops.iter().zip(self.colors) {
                        stops.push(ColorStop { at: cfvo(level)?, color });
                    }
                    Rule::ColorScale(ColorScale { stops })
                }
                Graded::DataBar => {
                    let mut bar = self.bar.clone();
                    bar.min = cfvo(&self.bar_low)?;
                    bar.max = cfvo(&self.bar_high)?;
                    Rule::DataBar(bar)
                }
                Graded::IconSet => {
                    // From the top band down, each threshold of the same
                    // type must lie below the one above it.
                    for pair in self.icon_rows.windows(2) {
                        if let (Some(low), Some(high)) = (pair[0].level.number(locale), pair[1].level.number(locale)) {
                            if pair[0].level.at == pair[1].level.at && low >= high {
                                return Err(t(
                                    "One or more icon data ranges overlap. Adjust icon data range values so that the ranges do not overlap.",
                                    "Une ou plusieurs plages de données d'icône se chevauchent. Ajustez les valeurs des plages de données d'icône afin que les plages ne se chevauchent pas.",
                                )
                                .to_string());
                            }
                        }
                    }
                    let mut thresholds = vec![IconThreshold {
                        value: Cfvo::percent(0.0),
                        gte: true,
                    }];
                    for row in &self.icon_rows {
                        thresholds.push(IconThreshold {
                            value: cfvo(&row.level)?,
                            gte: row.gte,
                        });
                    }
                    let plain = self.shown_set().is_some();
                    // The icons of the set in the opposite order are the
                    // set reversed, not a set of hand-picked icons.
                    let reversed: Vec<_> = set_icons(self.set).into_iter().rev().collect();
                    let reverse = !plain && self.icons == reversed;
                    Rule::IconSet(IconRule {
                        set: self.set,
                        thresholds,
                        reverse,
                        show_value: !self.show_icon_only,
                        custom: (!plain && !reverse).then(|| self.icons.clone()),
                    })
                }
            },
            Kind::Contains => match self.contains {
                Contains::CellValue => {
                    let first = operand(&self.first)?;
                    let second = if self.cell_op.takes_two() { Some(operand(&self.second)?) } else { None };
                    Rule::cell_is(self.cell_op, first, second)
                }
                Contains::Text => {
                    if self.text.is_empty() {
                        return Err(no_value());
                    }
                    Rule::Text {
                        op: self.text_op,
                        text: parse_text_operand(&self.text, locale, active, anchor)
                            .map_err(|_| enter_valid_formula().to_string())?,
                    }
                }
                Contains::Dates => Rule::DateOccurring(self.period),
                Contains::Blanks => Rule::Blanks,
                Contains::NoBlanks => Rule::NoBlanks,
                Contains::Errors => Rule::Errors,
                Contains::NoErrors => Rule::NoErrors,
            },
            Kind::Ranked => {
                let (limit, message) = if self.percent {
                    (100, t("You must enter a number between 1 and 100.", "Vous devez entrer un nombre entre 1 et 100."))
                } else {
                    (1000, t("Enter a whole number between 1 and 1000.", "Entrez un nombre entier entre 1 et 1 000."))
                };
                let rank = match self.rank.trim().parse::<u32>() {
                    Ok(n) if (1..=limit).contains(&n) => n,
                    _ => return Err(message.to_string()),
                };
                Rule::TopBottom {
                    rank,
                    percent: self.percent,
                    bottom: self.bottom,
                }
            }
            Kind::Average => {
                let (above, or_equal, std_dev) = words::AVERAGES[self.average.min(words::AVERAGES.len() - 1)];
                Rule::Average {
                    above,
                    or_equal,
                    std_dev,
                }
            }
            Kind::Unique if self.unique => Rule::UniqueValues,
            Kind::Unique => Rule::DuplicateValues,
            Kind::Formula => {
                if self.formula.trim().is_empty() {
                    return Err(enter_valid_formula().to_string());
                }
                // Without `=` Excel takes what was typed as a text, which
                // never is true: that is what is stored.
                Rule::Formula(parse_operand(&self.formula, locale, active, anchor).map_err(|_| enter_valid_formula().to_string())?)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Rect;
    use opensheet_engine::condfmt::BarAxis;

    fn en() -> &'static Locale {
        Locale::en()
    }

    fn rule(draft: &Draft) -> Result<Rule, String> {
        draft.rule(en(), (0, 0), (0, 0))
    }

    fn cf(rule: Rule) -> CondFormat {
        CondFormat::new(vec![Rect::new(0, 0, 9, 0)], rule)
    }

    /// Reading a rule into the dialog and making it back gives the rule.
    fn round_trip(original: Rule) {
        let draft = Draft::of(&cf(original.clone()), en(), (0, 0));
        assert_eq!(rule(&draft), Ok(original));
    }

    #[test]
    fn a_new_rule_is_a_two_colour_scale_from_lowest_to_highest() {
        let made = rule(&Draft::new(Kind::Values, Graded::TwoColor)).unwrap();
        let Rule::ColorScale(scale) = made else {
            panic!("a colour scale");
        };
        assert_eq!(scale.stops.len(), 2);
        assert_eq!((&scale.stops[0].at, &scale.stops[1].at), (&Cfvo::Min, &Cfvo::Max));
    }

    #[test]
    fn every_kind_of_rule_survives_the_dialog() {
        round_trip(Rule::ColorScale(ScalePreset::GreenYellowRed.scale()));
        round_trip(Rule::ColorScale(ScalePreset::WhiteRed.scale()));
        round_trip(Rule::DataBar(DataBar::preset(BarColor::Green, true)));
        round_trip(Rule::IconSet(IconRule::preset(IconSet::FiveArrows)));
        round_trip(Rule::between(Expr::Number(1.0), Expr::Number(2.5)));
        round_trip(Rule::greater_than(Expr::Text("m".into())));
        round_trip(Rule::Text {
            op: TextOp::BeginsWith,
            text: Expr::Text("ab".into()),
        });
        round_trip(Rule::date_occurring(TimePeriod::NextWeek));
        round_trip(Rule::Blanks);
        round_trip(Rule::NoErrors);
        round_trip(Rule::bottom_percent(15));
        round_trip(Rule::Average {
            above: false,
            or_equal: false,
            std_dev: 2,
        });
        round_trip(Rule::Average {
            above: true,
            or_equal: true,
            std_dev: 0,
        });
        round_trip(Rule::UniqueValues);
        round_trip(Rule::Formula(parse_operand("=$A1>3", en(), (0, 0), (0, 0)).unwrap()));
    }

    #[test]
    fn thresholds_take_numbers_and_fixed_references_only() {
        let level = |at, value: &str| Level::new(at, value).cfvo(en(), (0, 0), (0, 0));
        assert_eq!(level(At::Number, "12"), Ok(Cfvo::number(12.0)));
        assert_eq!(level(At::Percent, "40"), Ok(Cfvo::percent(40.0)));
        assert!(matches!(level(At::Number, "=$B$2"), Ok(Cfvo::Number(Expr::Ref(_)))));
        assert!(matches!(level(At::Formula, "=MAX($B$2:$B$9)/2"), Ok(Cfvo::Formula(_))));
        assert!(level(At::Number, "abc").is_err());
        assert!(level(At::Number, "").is_err());
        assert!(level(At::Percent, "140").is_err());
        assert!(level(At::Percentile, "-1").is_err());
        assert!(level(At::Formula, "12").is_err());
        // A relative reference would read another cell for every cell.
        assert!(level(At::Number, "=B2").unwrap_err().contains("relative"));
    }

    #[test]
    fn a_three_colour_scale_wants_its_points_in_order() {
        let mut draft = Draft::new(Kind::Values, Graded::ThreeColor);
        draft.stops = [Level::new(At::Number, "10"), Level::new(At::Number, "5"), Level::new(At::Number, "20")];
        assert!(rule(&draft).unwrap_err().contains("midpoint"));
        draft.stops[1] = Level::new(At::Number, "15");
        assert!(matches!(rule(&draft), Ok(Rule::ColorScale(scale)) if scale.stops.len() == 3));
        // Points of different types cannot be compared: Excel lets them be.
        draft.stops[1] = Level::new(At::Percent, "5");
        assert!(rule(&draft).is_ok());
    }

    #[test]
    fn icon_bands_must_not_overlap() {
        let mut draft = Draft::new(Kind::Values, Graded::IconSet);
        assert_eq!(draft.icon_rows.len(), 2);
        draft.icon_rows[0].level = Level::new(At::Percent, "70");
        assert!(rule(&draft).unwrap_err().contains("overlap"));
        draft.icon_rows[0].level = Level::new(At::Number, "70");
        assert!(rule(&draft).is_ok());
    }

    #[test]
    fn hand_picked_icons_are_kept_and_a_reversed_set_is_just_reversed() {
        let mut draft = Draft::new(Kind::Values, Graded::IconSet);
        draft.choose_set(IconSet::ThreeArrows);
        assert_eq!(draft.shown_set(), Some(IconSet::ThreeArrows));
        draft.icons.reverse();
        assert_eq!(draft.shown_set(), None);
        let Ok(Rule::IconSet(reversed)) = rule(&draft) else {
            panic!("an icon set");
        };
        assert!(reversed.reverse && reversed.custom.is_none());
        // Reading it back shows the same icons on the same bands.
        assert_eq!(Draft::of(&cf(Rule::IconSet(reversed)), en(), (0, 0)).icons, draft.icons);

        draft.icons[0] = None;
        draft.icons[1] = Some((IconSet::ThreeFlags, 2));
        let Ok(Rule::IconSet(custom)) = rule(&draft) else {
            panic!("an icon set");
        };
        assert!(!custom.reverse);
        assert_eq!(custom.custom.as_ref().map(|icons| icons[1]), Some(Some((IconSet::ThreeFlags, 2))));
        assert_eq!(Draft::of(&cf(Rule::IconSet(custom)), en(), (0, 0)).icons, draft.icons);
    }

    #[test]
    fn choosing_a_set_of_another_size_resets_the_thresholds() {
        let mut draft = Draft::new(Kind::Values, Graded::IconSet);
        draft.icon_rows[1].level = Level::new(At::Number, "80");
        draft.choose_set(IconSet::ThreeFlags);
        assert_eq!(draft.icon_rows[1].level, Level::new(At::Number, "80"));
        draft.choose_set(IconSet::FiveRating);
        let values: Vec<&str> = draft.icon_rows.iter().map(|row| row.level.value.as_str()).collect();
        assert_eq!(values, ["20", "40", "60", "80"]);
        assert_eq!(draft.icons.len(), 5);
    }

    #[test]
    fn the_data_bar_keeps_its_appearance() {
        let mut draft = Draft::new(Kind::Values, Graded::DataBar);
        draft.bar.axis = BarAxis::Midpoint;
        draft.bar_low = Level::new(At::Number, "-5");
        let Ok(Rule::DataBar(bar)) = rule(&draft) else {
            panic!("a data bar");
        };
        assert_eq!(bar.axis, BarAxis::Midpoint);
        assert_eq!((bar.min, bar.max), (Cfvo::number(-5.0), Cfvo::Automatic));
    }

    #[test]
    fn condition_rules_check_their_values() {
        let mut draft = Draft::new(Kind::Contains, Graded::TwoColor);
        assert!(rule(&draft).is_err());
        draft.first = "1".to_string();
        assert!(rule(&draft).is_err(), "between needs two values");
        draft.second = "9".to_string();
        assert_eq!(rule(&draft), Ok(Rule::between(Expr::Number(1.0), Expr::Number(9.0))));
        draft.cell_op = CellOp::Less;
        assert_eq!(rule(&draft), Ok(Rule::less_than(Expr::Number(1.0))));

        draft.kind = Kind::Ranked;
        draft.rank = "0".to_string();
        assert!(rule(&draft).is_err());
        draft.rank = "250".to_string();
        assert_eq!(rule(&draft), Ok(Rule::top_items(250)));
        draft.percent = true;
        assert!(rule(&draft).is_err());

        draft.kind = Kind::Formula;
        assert!(rule(&draft).is_err());
        draft.formula = "=A1>".to_string();
        assert!(rule(&draft).is_err());
        draft.formula = "=A1>2".to_string();
        assert!(matches!(rule(&draft), Ok(Rule::Formula(_))));
    }

    #[test]
    fn the_type_lists_offer_what_excel_offers() {
        assert_eq!(At::SCALE_LOW[0], At::Lowest);
        assert_eq!(At::SCALE_HIGH[0], At::Highest);
        assert!(At::BAR_LOW.contains(&At::Automatic) && !At::MIDDLE.contains(&At::Automatic));
        assert_eq!(At::Percentile.default_value(0), "10");
        assert_eq!(At::Percentile.default_value(2), "90");
        assert!(!At::Automatic.has_value() && At::Automatic.placeholder().starts_with('('));
    }
}
