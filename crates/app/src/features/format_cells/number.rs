//! The Number tab of Format Cells without its widgets: the categories,
//! the format code each choice produces, and reading an existing code back
//! into the choices that produce it.

use crate::i18n::{t, t_ctx};
use opensheet_engine::Locale;
use opensheet_engine::format;
use opensheet_engine::locale::DateOrder;

pub const MAX_DECIMALS: usize = 30;

/// 14 March 2012, 13:30:55: the date Excel shows its date and time types
/// with.
pub const SAMPLE_DATE: f64 = 40982.563_136_574;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    General,
    Number,
    Currency,
    Accounting,
    Date,
    Time,
    Percentage,
    Fraction,
    Scientific,
    Text,
    Special,
    Custom,
}

impl Category {
    pub const ALL: [Category; 12] = [
        Category::General,
        Category::Number,
        Category::Currency,
        Category::Accounting,
        Category::Date,
        Category::Time,
        Category::Percentage,
        Category::Fraction,
        Category::Scientific,
        Category::Text,
        Category::Special,
        Category::Custom,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::General => t("General", "Standard"),
            Category::Number => t_ctx("format", "Number", "Nombre"),
            Category::Currency => t("Currency", "Monétaire"),
            Category::Accounting => t("Accounting", "Comptabilité"),
            Category::Date => t("Date", "Date"),
            Category::Time => t("Time", "Heure"),
            Category::Percentage => t("Percentage", "Pourcentage"),
            Category::Fraction => t("Fraction", "Fraction"),
            Category::Scientific => t("Scientific", "Scientifique"),
            Category::Text => t("Text", "Texte"),
            Category::Special => t("Special", "Spécial"),
            Category::Custom => t("Custom", "Personnalisée"),
        }
    }

    /// The line of explanation under the options.
    pub fn description(self) -> &'static str {
        match self {
            Category::General => t(
                "General format cells have no specific number format.",
                "Les cellules de format Standard n'ont pas de format de nombre spécifique.",
            ),
            Category::Number => t(
                "Number is used for general display of numbers. Currency and Accounting offer specialized formatting for monetary value.",
                "Le format Nombre est utilisé pour l'affichage général des nombres. Les catégories Monétaire et Comptabilité offrent des formats spéciaux pour les valeurs monétaires.",
            ),
            Category::Currency => t(
                "Currency formats are used for general monetary values. Use Accounting formats to align decimal points in a column.",
                "Les formats Monétaire sont utilisés pour des valeurs monétaires générales. Utilisez les formats Comptabilité pour aligner les décimaux dans une colonne.",
            ),
            Category::Accounting => t(
                "Accounting formats line up the currency symbols and decimal points in a column.",
                "Les formats Comptabilité alignent les symboles monétaires et les décimaux dans une colonne.",
            ),
            Category::Date => t(
                "Date formats display date and time serial numbers as date values.",
                "Les formats Date affichent les numéros de date et heure comme valeurs de date.",
            ),
            Category::Time => t(
                "Time formats display date and time serial numbers as time values.",
                "Les formats Heure affichent les numéros de date et heure comme valeurs d'heure.",
            ),
            Category::Percentage => t(
                "Percentage formats multiply the cell value by 100 and display the result with a percent symbol.",
                "Les formats Pourcentage multiplient la valeur de la cellule par 100 et affichent le résultat avec le symbole pourcentage.",
            ),
            Category::Fraction | Category::Scientific => "",
            Category::Text => t(
                "Text format cells are treated as text even when a number is in the cell. The cell is displayed exactly as entered.",
                "Les cellules de format Texte sont traitées comme du texte même si c'est un nombre qui se trouve dans la cellule. La cellule est affichée exactement comme elle a été entrée.",
            ),
            Category::Special => t(
                "Special formats are useful for tracking list and database values.",
                "Les formats Spécial sont utiles pour contrôler des valeurs de liste et de base de données.",
            ),
            Category::Custom => t(
                "Type the number format code, using one of the existing codes as a starting point.",
                "Entrez le code du format de nombre, en utilisant un des codes existants comme point de départ.",
            ),
        }
    }
}

/// How negative numbers are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Negative {
    /// -1234.10
    Minus,
    /// 1234.10 in red
    Red,
    /// (1234.10)
    Parens,
    /// (1234.10) in red
    RedParens,
    /// -1234.10 in red
    RedMinus,
}

/// The negative styles offered in a locale, in the order of the list.
/// Parentheses are an English-speaking habit.
pub fn negatives(locale: &Locale) -> &'static [Negative] {
    if locale.id == "en" {
        &[
            Negative::Minus,
            Negative::Red,
            Negative::Parens,
            Negative::RedParens,
        ]
    } else {
        &[Negative::Minus, Negative::Red, Negative::RedMinus]
    }
}

fn with_negative(base: &str, negative: Negative) -> String {
    match negative {
        Negative::Minus => base.to_string(),
        Negative::Red => format!("{base};[Red]{base}"),
        Negative::Parens => format!("{base}_);({base})"),
        Negative::RedParens => format!("{base}_);[Red]({base})"),
        Negative::RedMinus => format!("{base};[Red]-{base}"),
    }
}

fn digits(decimals: usize, thousands: bool) -> String {
    let mut out = String::from(if thousands { "#,##0" } else { "0" });
    if decimals > 0 {
        out.push('.');
        out.push_str(&"0".repeat(decimals.min(MAX_DECIMALS)));
    }
    out
}

pub fn number_code(decimals: usize, thousands: bool, negative: Negative) -> String {
    with_negative(&digits(decimals, thousands), negative)
}

/// The currency symbols offered: none, the locale's, then the usual ones.
pub fn symbols(locale: &Locale) -> Vec<&'static str> {
    let mut out = vec!["", locale.currency_symbol];
    for symbol in ["$", "€", "£", "¥", "CHF"] {
        if !out.contains(&symbol) {
            out.push(symbol);
        }
    }
    out
}

/// A symbol as written in a format code: letters must be quoted.
fn symbol_in_code(symbol: &str) -> String {
    if symbol.chars().any(|c| c.is_alphanumeric()) {
        format!("\"{symbol}\"")
    } else {
        symbol.to_string()
    }
}

pub fn currency_code(decimals: usize, symbol: &str, negative: Negative, locale: &Locale) -> String {
    let number = digits(decimals, true);
    let base = if symbol.is_empty() {
        number
    } else if locale.currency_format.starts_with(locale.currency_symbol) {
        // The symbol leads where the locale's own format puts it first.
        format!("{}{number}", symbol_in_code(symbol))
    } else {
        format!("{number} {}", symbol_in_code(symbol))
    };
    with_negative(&base, negative)
}

pub fn accounting_code(decimals: usize, symbol: &str, locale: &Locale) -> String {
    let code = format::adjust_decimals(locale.accounting_format, decimals as i32 - 2);
    let own = locale.currency_symbol;
    if symbol.is_empty() {
        // Without a symbol, the space that set it apart goes too.
        code.replace(&format!(" {own}"), "").replace(own, "")
    } else {
        code.replace(own, &symbol_in_code(symbol))
    }
}

pub fn percent_code(decimals: usize) -> String {
    format!("{}%", digits(decimals, false))
}

pub fn scientific_code(decimals: usize) -> String {
    format!("{}E+00", digits(decimals, false))
}

fn dedup(codes: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for code in codes {
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

/// The date types of a locale (format codes).
pub fn date_types(locale: &Locale) -> Vec<String> {
    let extra: &[&str] = match locale.date_order {
        DateOrder::DMY => &[
            "d/m",
            "d/m/yy",
            "dd/mm/yy",
            "d-mmm",
            "d-mmm-yy",
            "dd-mmm-yy",
            "mmm-yy",
            "mmmm-yy",
            "d mmmm yyyy",
            "d/m/yy h:mm",
            "dd/mm/yyyy hh:mm",
            "yyyy-mm-dd",
        ],
        DateOrder::MDY => &[
            "m/d",
            "m/d/yy",
            "mm/dd/yy",
            "d-mmm",
            "d-mmm-yy",
            "dd-mmm-yy",
            "mmm-yy",
            "mmmm-yy",
            "mmmm d, yyyy",
            "m/d/yy h:mm AM/PM",
            "m/d/yy h:mm",
            "yyyy-mm-dd",
        ],
    };
    let mut codes = vec![
        locale.short_date_format.to_string(),
        locale.long_date_format.to_string(),
    ];
    // Numeric dates use the separator of the locale ("14.03.2012").
    let sep = locale.date_sep.to_string();
    codes.extend(extra.iter().map(|code| {
        if sep == "/" {
            code.to_string()
        } else {
            code.replace('/', &sep)
        }
    }));
    dedup(codes)
}

/// The time types of a locale (format codes).
pub fn time_types(locale: &Locale) -> Vec<String> {
    let mut codes = vec![
        locale.time_format.to_string(),
        "h:mm".to_string(),
        "h:mm:ss".to_string(),
    ];
    if locale.id == "en" {
        codes.push("h:mm AM/PM".to_string());
    } else {
        codes.push("hh:mm".to_string());
    }
    codes.push("mm:ss.0".to_string());
    codes.push("[h]:mm:ss".to_string());
    codes.push(format!("{} h:mm", locale.short_date_format));
    dedup(codes)
}

/// The fraction types: (code, name).
pub fn fraction_types() -> Vec<(&'static str, &'static str)> {
    vec![
        ("# ?/?", t("Up to one digit (1/4)", "D'un chiffre (1/4)")),
        (
            "# ??/??",
            t("Up to two digits (21/25)", "De deux chiffres (21/25)"),
        ),
        (
            "# ???/???",
            t(
                "Up to three digits (312/943)",
                "De trois chiffres (312/943)",
            ),
        ),
        ("# ?/2", t("As halves (1/2)", "Demis (1/2)")),
        ("# ?/4", t("As quarters (2/4)", "Quarts (2/4)")),
        ("# ?/8", t("As eighths (4/8)", "Huitièmes (4/8)")),
        ("# ??/16", t("As sixteenths (8/16)", "Seizièmes (8/16)")),
        ("# ?/10", t("As tenths (3/10)", "Dixièmes (3/10)")),
        (
            "# ??/100",
            t("As hundredths (30/100)", "Centièmes (30/100)"),
        ),
    ]
}

/// The special types of a locale: (code, name).
pub fn special_types(locale: &Locale) -> Vec<(&'static str, &'static str)> {
    match locale.id {
        "en" => vec![
            ("00000", t("Zip Code", "Code postal")),
            ("00000-0000", t("Zip Code + 4", "Code postal + 4")),
            (
                "[<=9999999]###-####;(###) ###-####",
                t("Phone Number", "Numéro de téléphone"),
            ),
            (
                "000-00-0000",
                t("Social Security Number", "Numéro de sécurité sociale"),
            ),
        ],
        "fr" => vec![
            ("00000", t("Zip Code", "Code postal")),
            (
                "0 00 00 00 000 000 00",
                t("Social Security Number", "Numéro de sécurité sociale"),
            ),
            ("0# ## ## ## ##", t("Phone Number", "Numéro de téléphone")),
        ],
        _ => vec![("00000", t("Zip Code", "Code postal"))],
    }
}

/// The codes listed under Custom: Excel's built-in ones for the locale,
/// then those the workbook uses.
pub fn custom_codes(locale: &Locale, used: &[String]) -> Vec<String> {
    let mut codes: Vec<String> = [
        "General",
        "0",
        "0.00",
        "#,##0",
        "#,##0.00",
        "#,##0;-#,##0",
        "#,##0;[Red]-#,##0",
        "#,##0.00;-#,##0.00",
        "#,##0.00;[Red]-#,##0.00",
    ]
    .iter()
    .map(|code| code.to_string())
    .collect();
    let own = locale.currency_symbol;
    for decimals in [0, 2] {
        for negative in negatives(locale).iter().skip(1) {
            codes.push(currency_code(decimals, own, *negative, locale));
        }
    }
    codes.extend(
        ["0%", "0.00%", "0.00E+00", "##0.0E+0", "# ?/?", "# ??/??"]
            .iter()
            .map(|code| code.to_string()),
    );
    codes.push(locale.short_date_format.to_string());
    codes.extend(
        ["d-mmm-yy", "d-mmm", "mmm-yy"]
            .iter()
            .map(|code| code.to_string()),
    );
    if locale.id == "en" {
        codes.extend(
            ["h:mm AM/PM", "h:mm:ss AM/PM"]
                .iter()
                .map(|code| code.to_string()),
        );
    }
    codes.extend(["h:mm", "h:mm:ss"].iter().map(|code| code.to_string()));
    codes.push(format!("{} h:mm", locale.short_date_format));
    codes.extend(
        ["mm:ss", "mm:ss.0", "@", "[h]:mm:ss"]
            .iter()
            .map(|code| code.to_string()),
    );
    codes.push(accounting_code(0, own, locale));
    codes.push(accounting_code(0, "", locale));
    codes.push(accounting_code(2, own, locale));
    codes.push(accounting_code(2, "", locale));
    codes.extend(used.iter().cloned());
    dedup(codes)
}

/// The choices of the Number tab.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub category: Category,
    pub decimals: usize,
    pub thousands: bool,
    /// Index into [`negatives`].
    pub negative: usize,
    /// Index into [`symbols`].
    pub symbol: usize,
    /// Index into the type list of the category (date, time, fraction,
    /// special).
    pub kind: usize,
}

impl Choice {
    pub fn general() -> Choice {
        Choice {
            category: Category::General,
            decimals: 2,
            thousands: false,
            negative: 0,
            symbol: 1,
            kind: 0,
        }
    }

    /// The format code these choices produce; `None` for Custom, whose
    /// code is typed.
    pub fn code(&self, locale: &Locale) -> Option<String> {
        let negative = negatives(locale)
            .get(self.negative)
            .copied()
            .unwrap_or(Negative::Minus);
        let symbol = symbols(locale).get(self.symbol).copied().unwrap_or("");
        let pick = |codes: Vec<String>| codes.get(self.kind).or(codes.first()).cloned();
        match self.category {
            Category::General => Some("General".to_string()),
            Category::Number => Some(number_code(self.decimals, self.thousands, negative)),
            Category::Currency => Some(currency_code(self.decimals, symbol, negative, locale)),
            Category::Accounting => Some(accounting_code(self.decimals, symbol, locale)),
            Category::Date => pick(date_types(locale)),
            Category::Time => pick(time_types(locale)),
            Category::Percentage => Some(percent_code(self.decimals)),
            Category::Fraction => pick(
                fraction_types()
                    .iter()
                    .map(|(code, _)| code.to_string())
                    .collect(),
            ),
            Category::Scientific => Some(scientific_code(self.decimals)),
            Category::Text => Some("@".to_string()),
            Category::Special => pick(
                special_types(locale)
                    .iter()
                    .map(|(code, _)| code.to_string())
                    .collect(),
            ),
            Category::Custom => None,
        }
    }

    /// The choices that produce `code`: Custom when no other category
    /// does.
    pub fn from_code(code: &str, locale: &Locale) -> Choice {
        let mut choice = Choice::general();
        if code.eq_ignore_ascii_case("General") {
            return choice;
        }
        let list = |category: Category, codes: Vec<String>, choice: &mut Choice| -> bool {
            match codes.iter().position(|c| c == code) {
                Some(kind) => {
                    choice.category = category;
                    choice.kind = kind;
                    true
                }
                None => false,
            }
        };
        if code == "@" {
            choice.category = Category::Text;
            return choice;
        }
        if list(Category::Date, date_types(locale), &mut choice)
            || list(Category::Time, time_types(locale), &mut choice)
            || list(
                Category::Fraction,
                fraction_types()
                    .iter()
                    .map(|(c, _)| c.to_string())
                    .collect(),
                &mut choice,
            )
        {
            return choice;
        }
        // Categories with options: find the options that give this code.
        let negative_count = negatives(locale).len();
        let symbol_count = symbols(locale).len();
        for decimals in 0..=MAX_DECIMALS {
            let mut candidate = Choice {
                decimals,
                ..Choice::general()
            };
            for category in [Category::Percentage, Category::Scientific] {
                candidate.category = category;
                if candidate.code(locale).as_deref() == Some(code) {
                    return candidate;
                }
            }
            for negative in 0..negative_count {
                candidate.negative = negative;
                candidate.category = Category::Number;
                for thousands in [false, true] {
                    candidate.thousands = thousands;
                    if candidate.code(locale).as_deref() == Some(code) {
                        return candidate;
                    }
                }
                candidate.thousands = false;
                candidate.category = Category::Currency;
                // The locale's own symbol first: "#,##0.00" with no symbol
                // is a Number.
                for symbol in (1..symbol_count).chain([0]) {
                    candidate.symbol = symbol;
                    if candidate.code(locale).as_deref() == Some(code) {
                        return candidate;
                    }
                }
                candidate.symbol = 1;
            }
            candidate.negative = 0;
            candidate.category = Category::Accounting;
            for symbol in (1..symbol_count).chain([0]) {
                candidate.symbol = symbol;
                if candidate.code(locale).as_deref() == Some(code) {
                    return candidate;
                }
            }
        }
        if list(
            Category::Special,
            special_types(locale)
                .iter()
                .map(|(c, _)| c.to_string())
                .collect(),
            &mut choice,
        ) {
            return choice;
        }
        choice.category = Category::Custom;
        choice
    }
}

/// A code as the dialog shows it: in the syntax of the locale.
pub fn shown(code: &str, locale: &Locale) -> String {
    if code.eq_ignore_ascii_case("General") {
        return t("General", "Standard").to_string();
    }
    format::code_to_locale(code, locale)
}

/// The code a text typed in the Custom box stands for. The codes of the
/// list are recognised as shown, so that picking one and pressing OK gives
/// that very code back.
pub fn typed(text: &str, known: &[String], locale: &Locale) -> String {
    let text = text.trim();
    // "Standard" is General in several languages; both are understood
    // whatever the language.
    if text.is_empty()
        || ["General", "Standard"]
            .iter()
            .any(|name| text.eq_ignore_ascii_case(name))
    {
        return "General".to_string();
    }
    if let Some(code) = known.iter().find(|code| shown(code, locale) == text) {
        return code.clone();
    }
    format::code_from_locale(text, locale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_formats_are_recognised() {
        for locale in Locale::all() {
            let currency = Choice::from_code(locale.currency_format, locale);
            assert_eq!(currency.category, Category::Currency, "{}", locale.id);
            assert_eq!((currency.decimals, currency.symbol), (2, 1));
            let accounting = Choice::from_code(locale.accounting_format, locale);
            assert_eq!(accounting.category, Category::Accounting, "{}", locale.id);
            assert_eq!(
                Choice::from_code(locale.short_date_format, locale).category,
                Category::Date
            );
            assert_eq!(
                Choice::from_code(locale.long_date_format, locale).category,
                Category::Date
            );
            assert_eq!(
                Choice::from_code(locale.time_format, locale).category,
                Category::Time
            );
        }
    }

    #[test]
    fn choices_and_codes_round_trip() {
        let en = Locale::en();
        for (code, category) in [
            ("General", Category::General),
            ("0.00", Category::Number),
            ("#,##0", Category::Number),
            ("#,##0.00_);[Red](#,##0.00)", Category::Number),
            ("0%", Category::Percentage),
            ("0.000E+00", Category::Scientific),
            ("# ?/?", Category::Fraction),
            ("@", Category::Text),
            ("000-00-0000", Category::Special),
            ("0.0\" kg\"", Category::Custom),
        ] {
            let choice = Choice::from_code(code, en);
            assert_eq!(choice.category, category, "{code}");
            if category != Category::Custom {
                assert_eq!(choice.code(en).as_deref(), Some(code));
            }
        }
        assert_eq!(
            accounting_code(2, "", en),
            "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)"
        );
        assert_eq!(
            currency_code(0, "CHF", Negative::Minus, Locale::fr()),
            "#,##0 \"CHF\""
        );
    }

    #[test]
    fn custom_codes_are_typed_back() {
        let fr = Locale::fr();
        let known = custom_codes(fr, &["0.0\" kg\"".to_string()]);
        for code in &known {
            assert_eq!(&typed(&shown(code, fr), &known, fr), code);
        }
        assert_eq!(typed("# ##0,000", &known, fr), "#,##0.000");
        assert_eq!(typed("jj/mm/aaaa", &known, fr), "dd/mm/yyyy");
        assert_eq!(typed("Standard", &known, fr), "General");
    }
}
