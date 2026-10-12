//! The words of conditional formatting: the names of the presets, of the
//! periods and of the icon sets, and how the Rules Manager describes a
//! rule. Wording is Excel's.

use crate::i18n::t;
use opensheet_engine::Locale;
use opensheet_engine::condfmt::{
    BarColor, CellOp, CondFormat, HighlightFormat, IconSet, Rule, ScalePreset, TextOp, TimePeriod, operand_text,
};

/// The entries of the "with" list of the Highlight Cells dialogs.
pub fn highlight_name(format: HighlightFormat) -> &'static str {
    match format {
        HighlightFormat::LightRedFillDarkRedText => {
            t("Light Red Fill with Dark Red Text", "Remplissage rouge clair avec texte rouge foncé")
        }
        HighlightFormat::YellowFillDarkYellowText => {
            t("Yellow Fill with Dark Yellow Text", "Remplissage jaune avec texte jaune foncé")
        }
        HighlightFormat::GreenFillDarkGreenText => {
            t("Green Fill with Dark Green Text", "Remplissage vert avec texte vert foncé")
        }
        HighlightFormat::LightRedFill => t("Light Red Fill", "Remplissage rouge clair"),
        HighlightFormat::RedText => t("Red Text", "Texte rouge"),
        HighlightFormat::RedBorder => t("Red Border", "Bordure rouge"),
    }
}

pub fn custom_format_name() -> &'static str {
    t("Custom Format...", "Format personnalisé...")
}

/// A period as the lists of the dialogs name it.
pub fn period_name(period: TimePeriod) -> &'static str {
    match period {
        TimePeriod::Yesterday => t("Yesterday", "Hier"),
        TimePeriod::Today => t("Today", "Aujourd'hui"),
        TimePeriod::Tomorrow => t("Tomorrow", "Demain"),
        TimePeriod::Last7Days => t("In the last 7 days", "Dans les 7 derniers jours"),
        TimePeriod::LastWeek => t("Last week", "La semaine dernière"),
        TimePeriod::ThisWeek => t("This week", "Cette semaine"),
        TimePeriod::NextWeek => t("Next week", "Semaine prochaine"),
        TimePeriod::LastMonth => t("Last month", "Le mois dernier"),
        TimePeriod::ThisMonth => t("This month", "Ce mois"),
        TimePeriod::NextMonth => t("Next month", "Le mois prochain"),
    }
}

/// A period as the Rules Manager describes a rule on it.
fn period_rule(period: TimePeriod) -> &'static str {
    match period {
        TimePeriod::Last7Days => t("Last 7 days", "7 derniers jours"),
        other => period_name(other),
    }
}

/// The comparisons of a "Cell Value" rule, as the rule dialog lists them.
pub fn cell_op_name(op: CellOp) -> &'static str {
    match op {
        CellOp::Between => t("between", "comprise entre"),
        CellOp::NotBetween => t("not between", "non comprise entre"),
        CellOp::Equal => t("equal to", "égale à"),
        CellOp::NotEqual => t("not equal to", "différente de"),
        CellOp::Greater => t("greater than", "supérieure à"),
        CellOp::Less => t("less than", "inférieure à"),
        CellOp::GreaterOrEqual => t("greater than or equal to", "supérieure ou égale à"),
        CellOp::LessOrEqual => t("less than or equal to", "inférieure ou égale à"),
    }
}

pub fn text_op_name(op: TextOp) -> &'static str {
    match op {
        TextOp::Contains => t("containing", "contenant"),
        TextOp::NotContains => t("not containing", "ne contenant pas"),
        TextOp::BeginsWith => t("beginning with", "commençant par"),
        TextOp::EndsWith => t("ending with", "se terminant par"),
    }
}

/// The choices of an "above or below average" rule, in the order of the
/// rule dialog: (above, or equal, standard deviations).
pub const AVERAGES: [(bool, bool, u32); 10] = [
    (true, false, 0),
    (false, false, 0),
    (true, true, 0),
    (false, true, 0),
    (true, false, 1),
    (false, false, 1),
    (true, false, 2),
    (false, false, 2),
    (true, false, 3),
    (false, false, 3),
];

/// The choices of [`AVERAGES`] as the rule dialog lists them.
pub fn average_names() -> [&'static str; 10] {
    [
        t("above", "au-dessus"),
        t("below", "en dessous"),
        t("equal or above", "égales ou au-dessus"),
        t("equal or below", "égales ou en dessous"),
        t("1 std dev above", "1 écart-type au-dessus"),
        t("1 std dev below", "1 écart-type en dessous"),
        t("2 std dev above", "2 écarts-type au-dessus"),
        t("2 std dev below", "2 écarts-type en dessous"),
        t("3 std dev above", "3 écarts-type au-dessus"),
        t("3 std dev below", "3 écarts-type en dessous"),
    ]
}

fn average_rule(above: bool, or_equal: bool, std_dev: u32) -> String {
    match (std_dev, or_equal, above) {
        (0, false, true) => t("Above Average", "Valeurs supérieures à la moyenne").to_string(),
        (0, false, false) => t("Below Average", "Valeurs inférieures à la moyenne").to_string(),
        (0, true, true) => t("Equal to or Above Average", "Égales ou au-dessus de la moyenne").to_string(),
        (0, true, false) => t("Equal to or Below Average", "Égales ou en dessous de la moyenne").to_string(),
        (n, _, true) => match n {
            1 => t("1 std dev above Average", "1 écart type au-dessus de la moyenne").to_string(),
            _ => format!("{n} {}", t("std dev above Average", "écarts types au-dessus de la moyenne")),
        },
        (n, _, false) => match n {
            1 => t("1 std dev below Average", "1 écart type en dessous de la moyenne").to_string(),
            _ => format!("{n} {}", t("std dev below Average", "écarts types en dessous de la moyenne")),
        },
    }
}

pub fn bar_color_name(color: BarColor) -> &'static str {
    match color {
        BarColor::Blue => t("Blue Data Bar", "Barre de données bleue"),
        BarColor::Green => t("Green Data Bar", "Barre de données verte"),
        BarColor::Red => t("Red Data Bar", "Barre de données rouge"),
        BarColor::Orange => t("Orange Data Bar", "Barre de données orange"),
        BarColor::LightBlue => t("Light Blue Data Bar", "Barre de données bleu clair"),
        BarColor::Purple => t("Purple Data Bar", "Barre de données violette"),
    }
}

pub fn scale_name(scale: ScalePreset) -> &'static str {
    match scale {
        ScalePreset::GreenYellowRed => t("Green - Yellow - Red Color Scale", "Échelle de couleur Vert - Jaune - Rouge"),
        ScalePreset::RedYellowGreen => t("Red - Yellow - Green Color Scale", "Échelle de couleur Rouge - Jaune - Vert"),
        ScalePreset::GreenWhiteRed => t("Green - White - Red Color Scale", "Échelle de couleur Vert - Blanc - Rouge"),
        ScalePreset::RedWhiteGreen => t("Red - White - Green Color Scale", "Échelle de couleur Rouge - Blanc - Vert"),
        ScalePreset::BlueWhiteRed => t("Blue - White - Red Color Scale", "Échelle de couleur Bleu - Blanc - Rouge"),
        ScalePreset::RedWhiteBlue => t("Red - White - Blue Color Scale", "Échelle de couleur Rouge - Blanc - Bleu"),
        ScalePreset::WhiteRed => t("White - Red Color Scale", "Échelle de couleur Blanc - Rouge"),
        ScalePreset::RedWhite => t("Red - White Color Scale", "Échelle de couleur Rouge - Blanc"),
        ScalePreset::GreenWhite => t("Green - White Color Scale", "Échelle de couleur Vert - Blanc"),
        ScalePreset::WhiteGreen => t("White - Green Color Scale", "Échelle de couleur Blanc - Vert"),
        ScalePreset::GreenYellow => t("Green - Yellow Color Scale", "Échelle de couleur Vert - Jaune"),
        ScalePreset::YellowGreen => t("Yellow - Green Color Scale", "Échelle de couleur Jaune - Vert"),
    }
}

pub fn set_name(set: IconSet) -> &'static str {
    match set {
        IconSet::ThreeArrows => t("3 Arrows (Colored)", "3 Flèches (en couleur)"),
        IconSet::ThreeArrowsGray => t("3 Arrows (Gray)", "3 Flèches (grises)"),
        IconSet::ThreeFlags => t("3 Flags", "3 Drapeaux"),
        IconSet::ThreeTrafficLights1 => t("3 Traffic Lights (Unrimmed)", "3 Indicateurs (sans bordure)"),
        IconSet::ThreeTrafficLights2 => t("3 Traffic Lights (Rimmed)", "3 Indicateurs (avec bordure)"),
        IconSet::ThreeSigns => t("3 Signs", "3 Signes"),
        IconSet::ThreeSymbols => t("3 Symbols (Circled)", "3 Symboles (avec cercle)"),
        IconSet::ThreeSymbols2 => t("3 Symbols (Uncircled)", "3 symboles (sans cercle)"),
        IconSet::ThreeStars => t("3 Stars", "3 étoiles"),
        IconSet::ThreeTriangles => t("3 Triangles", "3 triangles"),
        IconSet::FourArrows => t("4 Arrows (Colored)", "4 Flèches (en couleur)"),
        IconSet::FourArrowsGray => t("4 Arrows (Gray)", "4 Flèches (grises)"),
        IconSet::FourRedToBlack => t("Red To Black", "Rouge à Noir"),
        IconSet::FourRating => t("4 Ratings", "4 Évaluations"),
        IconSet::FourTrafficLights => t("4 Traffic Lights", "4 Indicateurs"),
        IconSet::FiveArrows => t("5 Arrows (Colored)", "5 Flèches (en couleur)"),
        IconSet::FiveArrowsGray => t("5 Arrows (Gray)", "5 Flèches (grises)"),
        IconSet::FiveRating => t("5 Ratings", "5 Évaluations"),
        IconSet::FiveQuarters => t("5 Quarters", "5 Trimestres"),
        IconSet::FiveBoxes => t("5 Boxes", "5 zones"),
    }
}

/// The groups of the icon set gallery: a heading and how many sets of
/// `IconSet::ALL` it takes.
pub fn set_groups() -> [(&'static str, usize); 4] {
    [
        (t("Directional", "Directionnel"), 7),
        (t("Shapes", "Formes"), 5),
        (t("Indicators", "Indicateurs"), 3),
        (t("Ratings", "Contrôle d'accès"), 5),
    ]
}

/// A rule as the Rules Manager words it ("Cell Value > 5", "Top 10",
/// "Data Bar"). Formulas read from `active`, the cell the dialogs speak
/// for.
pub fn describe(cf: &CondFormat, locale: &Locale, active: (u32, u32)) -> String {
    let anchor = cf.anchor();
    // An operand without the `=` that introduces it in a value box.
    let operand = |e| {
        let text = operand_text(e, locale, anchor, active, false);
        text.strip_prefix('=').map(str::to_string).unwrap_or(text)
    };
    match &cf.rule {
        Rule::CellIs { op, first, second } => {
            let value = t("Cell Value", "Valeur de la cellule");
            let first = operand(first);
            let second = second.as_ref().map(&operand).unwrap_or_default();
            match op {
                CellOp::Between => match crate::i18n::lang() {
                    crate::i18n::Lang::Fr => format!("La valeur de la cellule est comprise entre {first} et {second}"),
                    _ => format!("{value} {} {first} {} {second}", t("between", "comprise entre"), t("and", "et")),
                },
                CellOp::NotBetween => match crate::i18n::lang() {
                    crate::i18n::Lang::Fr => {
                        format!("La valeur de la cellule n'est pas comprise entre {first} et {second}")
                    }
                    _ => format!("{value} {} {first} {} {second}", t("not between", "non comprise entre"), t("and", "et")),
                },
                CellOp::Equal => format!("{value} = {first}"),
                CellOp::NotEqual => format!("{value} <> {first}"),
                CellOp::Greater => format!("{value} > {first}"),
                CellOp::Less => format!("{value} < {first}"),
                CellOp::GreaterOrEqual => format!("{value} >= {first}"),
                CellOp::LessOrEqual => format!("{value} <= {first}"),
            }
        }
        Rule::Formula(e) => format!(
            "{} {}",
            t("Formula:", "Formule :"),
            operand_text(e, locale, anchor, active, false)
        ),
        Rule::TopBottom { rank, percent, bottom } => {
            let side = if *bottom { t("Bottom", "Bas") } else { t("Top", "Haut") };
            format!("{side} {rank}{}", if *percent { "%" } else { "" })
        }
        Rule::Average { above, or_equal, std_dev } => average_rule(*above, *or_equal, *std_dev),
        Rule::DuplicateValues => t("Duplicate Values", "Valeurs en double").to_string(),
        Rule::UniqueValues => t("Unique Values", "Valeurs uniques").to_string(),
        Rule::Text { op, text } => {
            let verb = match op {
                TextOp::Contains => t("Cell Value contains", "La valeur de la cellule contient"),
                TextOp::NotContains => t("Cell Value does not contain", "La valeur de la cellule ne contient pas"),
                TextOp::BeginsWith => t("Cell Value begins with", "La valeur de la cellule commence par"),
                TextOp::EndsWith => t("Cell Value ends with", "La valeur de la cellule se termine par"),
            };
            let shown = operand_text(text, locale, anchor, active, true);
            match text {
                opensheet_engine::Expr::Text(_) => match crate::i18n::lang() {
                    crate::i18n::Lang::Fr => format!("{verb} « {shown} »"),
                    _ => format!("{verb} '{shown}'"),
                },
                _ => format!("{verb} {shown}"),
            }
        }
        Rule::Blanks => t("Cell contains a blank value", "La cellule contient une valeur vide").to_string(),
        Rule::NoBlanks => t("Cell does not contain a blank value", "La cellule ne contient pas de valeur vide").to_string(),
        Rule::Errors => t("Cell contains an error", "La cellule contient une erreur").to_string(),
        Rule::NoErrors => t("Cell does not contain an error", "La cellule ne contient pas d'erreur").to_string(),
        Rule::DateOccurring(period) => period_rule(*period).to_string(),
        Rule::ColorScale(_) => t("Graded Color Scale", "Échelle de couleurs graduée").to_string(),
        Rule::DataBar(_) => t("Data Bar", "Barre de données").to_string(),
        Rule::IconSet(_) => t("Icon Set", "Jeu d'icônes").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_lang};
    use opensheet_engine::condfmt::{ColorScale, DataBar, IconRule, parse_operand};
    use opensheet_engine::style::Color;
    use opensheet_engine::{Expr, Rect};

    fn rule(rule: Rule) -> CondFormat {
        CondFormat::new(vec![Rect::new(1, 1, 9, 1)], rule)
    }

    /// The description in English with the English locale, read from the
    /// rule's own anchor.
    fn en(r: Rule) -> String {
        set_lang(Lang::En);
        describe(&rule(r), Locale::en(), (1, 1))
    }

    #[test]
    fn cell_value_rules_show_their_operands() {
        assert_eq!(en(Rule::greater_than(Expr::Number(5.0))), "Cell Value > 5");
        assert_eq!(en(Rule::between(Expr::Number(1.5), Expr::Number(3.0))), "Cell Value between 1.5 and 3");
        assert_eq!(en(Rule::equal_to(Expr::Text("yes".into()))), "Cell Value = \"yes\"");
        let reference = parse_operand("=$D$1", Locale::en(), (1, 1), (1, 1)).unwrap();
        assert_eq!(en(Rule::less_than(reference)), "Cell Value < $D$1");
    }

    #[test]
    fn a_relative_formula_reads_from_the_active_cell() {
        // Written for B2 (the anchor), read from B5: three rows further.
        let formula = parse_operand("=A2>10", Locale::en(), (1, 1), (1, 1)).unwrap();
        set_lang(Lang::En);
        assert_eq!(describe(&rule(Rule::Formula(formula)), Locale::en(), (4, 1)), "Formula: =A5>10");
    }

    #[test]
    fn ranked_and_average_rules() {
        assert_eq!(en(Rule::top_items(10)), "Top 10");
        assert_eq!(en(Rule::bottom_percent(25)), "Bottom 25%");
        assert_eq!(en(Rule::above_average()), "Above Average");
        assert_eq!(
            en(Rule::Average {
                above: false,
                or_equal: false,
                std_dev: 2
            }),
            "2 std dev below Average"
        );
        assert_eq!(
            en(Rule::Average {
                above: true,
                or_equal: true,
                std_dev: 0
            }),
            "Equal to or Above Average"
        );
    }

    #[test]
    fn text_date_and_graded_rules() {
        assert_eq!(en(Rule::text_contains(Expr::Text("abc".into()))), "Cell Value contains 'abc'");
        assert_eq!(en(Rule::date_occurring(TimePeriod::Last7Days)), "Last 7 days");
        assert_eq!(en(Rule::DuplicateValues), "Duplicate Values");
        assert_eq!(en(Rule::Blanks), "Cell contains a blank value");
        assert_eq!(en(Rule::ColorScale(ColorScale::two(Color(0), Color(0xFFFFFF)))), "Graded Color Scale");
        assert_eq!(en(Rule::DataBar(DataBar::legacy(Color(0x638EC6)))), "Data Bar");
        assert_eq!(en(Rule::IconSet(IconRule::preset(IconSet::ThreeArrows))), "Icon Set");
    }

    #[test]
    fn the_average_lists_agree() {
        assert_eq!(AVERAGES.len(), average_names().len());
        assert_eq!(set_groups().iter().map(|(_, count)| count).sum::<usize>(), IconSet::ALL.len());
    }
}
