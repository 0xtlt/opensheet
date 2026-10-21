//! Locale data: separators, localized function / error / boolean names,
//! month and day names. Formulas are stored canonically (English names) and
//! translated at the parse / print boundary.

use crate::value::ErrorKind;
use std::collections::HashMap;
use std::sync::OnceLock;

mod names_de;
mod names_es;
mod names_fr;
mod names_it;
#[cfg(test)]
mod names_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    MDY,
    DMY,
}

pub struct Locale {
    pub id: &'static str,
    pub decimal_sep: char,
    pub thousands_sep: char,
    /// Function argument separator (`,` in English, `;` in French).
    pub arg_sep: char,
    /// Column separator inside `{...}` array constants.
    pub array_col_sep: char,
    /// Row separator inside `{...}` array constants.
    pub array_row_sep: char,
    pub true_name: &'static str,
    pub false_name: &'static str,
    /// Error spellings, in `ErrorKind::ALL` order.
    pub errors: [&'static str; 9],
    pub months: [&'static str; 12],
    pub months_short: [&'static str; 12],
    /// Sunday first.
    pub days: [&'static str; 7],
    pub days_short: [&'static str; 7],
    pub date_order: DateOrder,
    /// Number format applied when the user types a date.
    pub short_date_format: &'static str,
    pub long_date_format: &'static str,
    pub time_format: &'static str,
    pub currency_symbol: &'static str,
    /// Number format applied by the "currency" button.
    pub currency_format: &'static str,
    pub accounting_format: &'static str,
    /// Default name prefix for new sheets ("Sheet", "Feuil").
    pub sheet_prefix: &'static str,
    /// Default name prefix for new tables ("Table", "Tableau").
    pub table_prefix: &'static str,
    /// Default name prefix for table columns without a header ("Column",
    /// "Colonne").
    pub column_prefix: &'static str,
    /// Label written in the first cell of a table's totals row.
    pub total_label: &'static str,
    /// Separator accepted in typed dates besides `/` and `-` (`.` in German).
    pub date_sep: char,
    /// Letters standing for day and year in number-format codes written in
    /// this language (`j` / `a` in French, `t` / `j` in German), lowercase.
    pub fmt_day: char,
    pub fmt_year: char,
    /// Name of the language in that language.
    pub display_name: &'static str,
    /// Item specifiers of structured references (`Table1[#All]`), in
    /// `TableItem::ALL` order: all, data, headers, totals, this row.
    pub table_items: [&'static str; 5],
    /// Other spellings of the item specifiers accepted when typed.
    pub table_item_aliases: &'static [(&'static str, usize)],
    /// Row and column letters of the R1C1 notation (`L1C1` in French),
    /// uppercase: a defined name may not look like such a reference.
    pub r1c1: (char, char),
    fn_pairs: &'static [(&'static str, &'static str)],
    maps: OnceLock<(
        HashMap<&'static str, &'static str>,
        HashMap<&'static str, &'static str>,
    )>,
}

impl Locale {
    pub fn en() -> &'static Locale {
        &EN
    }

    pub fn fr() -> &'static Locale {
        &FR
    }

    pub fn es() -> &'static Locale {
        &ES
    }

    pub fn de() -> &'static Locale {
        &DE
    }

    pub fn it() -> &'static Locale {
        &IT
    }

    /// Every supported locale.
    pub fn all() -> [&'static Locale; 5] {
        [&EN, &FR, &ES, &DE, &IT]
    }

    /// Locale for a language tag ("fr", "de-DE", "es_ES"...); English when
    /// the language is not supported.
    pub fn by_id(id: &str) -> &'static Locale {
        let lower = id.to_ascii_lowercase();
        Locale::all()
            .into_iter()
            .find(|l| lower.starts_with(l.id))
            .unwrap_or(&EN)
    }

    fn maps(
        &self,
    ) -> &(
        HashMap<&'static str, &'static str>,
        HashMap<&'static str, &'static str>,
    ) {
        self.maps.get_or_init(|| {
            let mut to_canon = HashMap::new();
            let mut to_local = HashMap::new();
            for (canon, local) in self.fn_pairs {
                to_canon.insert(*local, *canon);
                to_local.insert(*canon, *local);
            }
            (to_canon, to_local)
        })
    }

    /// Translate a function name typed by the user (already uppercased) to
    /// its canonical English name. Unknown names pass through unchanged, so
    /// English names are always accepted.
    pub fn fn_to_canonical(&self, upper: &str) -> String {
        match self.maps().0.get(upper) {
            Some(c) => (*c).to_string(),
            None => upper.to_string(),
        }
    }

    /// Translate a canonical function name to its localized spelling.
    pub fn fn_to_local<'a>(&self, canonical: &'a str) -> &'a str
    where
        'static: 'a,
    {
        match self.maps().1.get(canonical) {
            Some(l) => l,
            None => canonical,
        }
    }

    pub fn error_name(&self, e: ErrorKind) -> &'static str {
        let idx = ErrorKind::ALL.iter().position(|k| *k == e).unwrap_or(0);
        self.errors[idx]
    }

    /// Parse an error literal in this locale (falls back to English spelling).
    pub fn parse_error(&self, s: &str) -> Option<ErrorKind> {
        let up = s.to_uppercase();
        for (i, name) in self.errors.iter().enumerate() {
            if *name == up {
                return Some(ErrorKind::ALL[i]);
            }
        }
        ErrorKind::from_str(&up)
    }

    /// Index (in `TableItem::ALL` order) of a structured-reference item
    /// specifier such as `#Headers`, in this locale or in English, compared
    /// without case.
    pub fn parse_table_item(&self, s: &str) -> Option<usize> {
        let wanted = s.trim().to_lowercase();
        let find = |names: &[&str; 5]| names.iter().position(|n| n.to_lowercase() == wanted);
        find(&self.table_items)
            .or_else(|| find(&EN.table_items))
            .or_else(|| {
                self.table_item_aliases
                    .iter()
                    .find(|(alias, _)| alias.to_lowercase() == wanted)
                    .map(|(_, index)| *index)
            })
    }

    pub fn bool_name(&self, b: bool) -> &'static str {
        if b { self.true_name } else { self.false_name }
    }

    pub fn parse_bool(&self, s: &str) -> Option<bool> {
        let up = s.to_uppercase();
        if up == self.true_name || up == "TRUE" {
            Some(true)
        } else if up == self.false_name || up == "FALSE" {
            Some(false)
        } else {
            None
        }
    }

    /// Parse a plain number written in this locale ("1,5" in French, "1.5" in
    /// English, optional thousands separators, exponent, leading sign,
    /// trailing `%`). Thousands separators must delimit groups of exactly
    /// three digits ("1.000" is a thousand in German, "1.5" is not a number).
    pub fn parse_number(&self, s: &str) -> Option<f64> {
        self.parse_number_impl(s, true)
    }

    /// Like `parse_number`, but thousands separators may appear anywhere
    /// between digits ("1,5" reads as 15 in English, as Excel does).
    pub fn parse_number_lenient(&self, s: &str) -> Option<f64> {
        self.parse_number_impl(s, false)
    }

    fn parse_number_impl(&self, s: &str, strict: bool) -> Option<f64> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        let (t, percent) = match t.strip_suffix('%') {
            Some(rest) => (rest.trim_end(), true),
            None => (t, false),
        };
        let mut out = String::with_capacity(t.len());
        let mut seen_digit = false;
        let mut seen_dec = false;
        let mut seen_exp = false;
        // Digits since the start / the last thousands separator.
        let mut group = 0usize;
        let mut groups = 0usize;
        let chars: Vec<char> = t.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            match ch {
                '0'..='9' => {
                    seen_digit = true;
                    if !seen_dec && !seen_exp {
                        group += 1;
                    }
                    out.push(ch);
                }
                '+' | '-' => {
                    let after_exp = i > 0 && matches!(chars[i - 1], 'e' | 'E');
                    if i == 0 || after_exp {
                        out.push(ch);
                    } else {
                        return None;
                    }
                }
                'e' | 'E' => {
                    if !seen_digit || seen_exp {
                        return None;
                    }
                    if strict && groups > 0 && group != 3 {
                        return None;
                    }
                    seen_exp = true;
                    out.push('e');
                }
                c if c == self.decimal_sep => {
                    if seen_dec || seen_exp {
                        return None;
                    }
                    if strict && groups > 0 && group != 3 {
                        return None;
                    }
                    seen_dec = true;
                    out.push('.');
                }
                c if is_thousands_sep(c, self.thousands_sep) => {
                    // Only valid between digits, before the decimal point.
                    if !seen_digit || seen_dec || seen_exp {
                        return None;
                    }
                    if !chars.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
                        return None;
                    }
                    if strict
                        && ((groups == 0 && !(1..=3).contains(&group))
                            || (groups > 0 && group != 3))
                    {
                        return None;
                    }
                    groups += 1;
                    group = 0;
                }
                _ => return None,
            }
        }
        if !seen_digit {
            return None;
        }
        if strict && groups > 0 && !seen_dec && !seen_exp && group != 3 {
            return None;
        }
        let n: f64 = out.parse().ok()?;
        if !n.is_finite() {
            return None;
        }
        Some(if percent { n / 100.0 } else { n })
    }

    /// Text form of a number as produced by implicit number → text coercion
    /// (`=A1&""`): up to 15 significant digits, locale decimal separator.
    pub fn number_to_text(&self, n: f64) -> String {
        let s = general_number(n, 15);
        if self.decimal_sep == '.' {
            s
        } else {
            s.replace('.', &self.decimal_sep.to_string())
        }
    }
}

fn is_thousands_sep(c: char, locale_sep: char) -> bool {
    if c == locale_sep {
        return true;
    }
    // French groups digits with a (narrow) no-break space; accept any space.
    locale_sep.is_whitespace() && (c == ' ' || c == '\u{a0}' || c == '\u{202f}')
}

/// Shortest decimal rendering of `n` with at most `sig` significant digits,
/// using `.` as decimal separator and Excel-style `E+NN` scientific notation
/// for very large / very small magnitudes.
pub fn general_number(n: f64, sig: usize) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".to_string();
    }
    let sig = sig.clamp(1, 17);
    // Round to `sig` significant digits via scientific formatting.
    let sci = format!("{:.*e}", sig - 1, n);
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if exp >= sig as i32 || exp < -9 {
        // Scientific: d.dddE+NN
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('E');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    } else if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            out.push_str(digits);
            out.extend(std::iter::repeat_n('0', int_len - digits.len()));
        } else {
            out.push_str(&digits[..int_len]);
            out.push('.');
            out.push_str(&digits[int_len..]);
        }
    } else {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
        out.push_str(digits);
    }
    out
}

const EN_ERRORS: [&str; 9] = [
    "#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#SPILL!", "#CALC!",
];

const FR_ERRORS: [&str; 9] = [
    "#NUL!",
    "#DIV/0!",
    "#VALEUR!",
    "#REF!",
    "#NOM?",
    "#NOMBRE!",
    "#N/A",
    "#PROPAGATION!",
    "#CALC!",
];

static EN: Locale = Locale {
    id: "en",
    decimal_sep: '.',
    thousands_sep: ',',
    arg_sep: ',',
    array_col_sep: ',',
    array_row_sep: ';',
    true_name: "TRUE",
    false_name: "FALSE",
    errors: EN_ERRORS,
    months: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    months_short: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    days: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    days_short: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    date_order: DateOrder::MDY,
    short_date_format: "m/d/yyyy",
    long_date_format: "dddd, mmmm d, yyyy",
    time_format: "h:mm:ss AM/PM",
    currency_symbol: "$",
    currency_format: "$#,##0.00",
    accounting_format: "_($* #,##0.00_);_($* (#,##0.00);_($* \"-\"??_);_(@_)",
    sheet_prefix: "Sheet",
    table_prefix: "Table",
    column_prefix: "Column",
    total_label: "Total",
    date_sep: '/',
    fmt_day: 'd',
    fmt_year: 'y',
    display_name: "English",
    table_items: ["#All", "#Data", "#Headers", "#Totals", "#This Row"],
    table_item_aliases: &[],
    r1c1: ('R', 'C'),
    fn_pairs: &[],
    maps: OnceLock::new(),
};

static FR: Locale = Locale {
    id: "fr",
    decimal_sep: ',',
    thousands_sep: '\u{202f}',
    arg_sep: ';',
    array_col_sep: '.',
    array_row_sep: ';',
    true_name: "VRAI",
    false_name: "FAUX",
    errors: FR_ERRORS,
    months: [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ],
    months_short: [
        "janv", "févr", "mars", "avr", "mai", "juin", "juil", "août", "sept", "oct", "nov", "déc",
    ],
    days: [
        "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
    ],
    days_short: ["dim", "lun", "mar", "mer", "jeu", "ven", "sam"],
    date_order: DateOrder::DMY,
    short_date_format: "dd/mm/yyyy",
    long_date_format: "dddd d mmmm yyyy",
    time_format: "hh:mm:ss",
    currency_symbol: "€",
    currency_format: "#,##0.00 €",
    accounting_format: "_-* #,##0.00 €_-;-* #,##0.00 €_-;_-* \"-\"?? €_-;_-@_-",
    sheet_prefix: "Feuil",
    table_prefix: "Tableau",
    column_prefix: "Colonne",
    total_label: "Total",
    date_sep: '/',
    fmt_day: 'j',
    fmt_year: 'a',
    display_name: "Français",
    table_items: ["#Tout", "#Données", "#En-têtes", "#Totaux", "#Cette ligne"],
    table_item_aliases: &[],
    r1c1: ('L', 'C'),
    fn_pairs: names_fr::FUNCTIONS,
    maps: OnceLock::new(),
};

static ES: Locale = Locale {
    id: "es",
    decimal_sep: ',',
    thousands_sep: '.',
    arg_sep: ';',
    array_col_sep: '\\',
    array_row_sep: ';',
    true_name: "VERDADERO",
    false_name: "FALSO",
    errors: [
        "#¡NULO!",
        "#¡DIV/0!",
        "#¡VALOR!",
        "#¡REF!",
        "#¿NOMBRE?",
        "#¡NUM!",
        "#N/D",
        "#¡DESBORDAMIENTO!",
        "#¡CALC!",
    ],
    months: [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ],
    months_short: [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
    ],
    days: [
        "domingo",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
    ],
    days_short: ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"],
    date_order: DateOrder::DMY,
    short_date_format: "dd/mm/yyyy",
    long_date_format: "dddd, d \"de\" mmmm \"de\" yyyy",
    time_format: "h:mm:ss",
    currency_symbol: "€",
    currency_format: "#,##0.00 €",
    accounting_format: "_-* #,##0.00 €_-;-* #,##0.00 €_-;_-* \"-\"?? €_-;_-@_-",
    sheet_prefix: "Hoja",
    table_prefix: "Tabla",
    column_prefix: "Columna",
    total_label: "Total",
    date_sep: '/',
    fmt_day: 'd',
    fmt_year: 'a',
    display_name: "Español",
    table_items: ["#Todo", "#Datos", "#Encabezados", "#Totales", "#Esta fila"],
    table_item_aliases: &[("#Todas", 0)],
    r1c1: ('F', 'C'),
    fn_pairs: names_es::FUNCTIONS,
    maps: OnceLock::new(),
};

static DE: Locale = Locale {
    id: "de",
    decimal_sep: ',',
    thousands_sep: '.',
    arg_sep: ';',
    array_col_sep: '.',
    array_row_sep: ';',
    true_name: "WAHR",
    false_name: "FALSCH",
    errors: [
        "#NULL!",
        "#DIV/0!",
        "#WERT!",
        "#BEZUG!",
        "#NAME?",
        "#ZAHL!",
        "#NV",
        "#ÜBERLAUF!",
        "#KALK!",
    ],
    months: [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ],
    months_short: [
        "Jan", "Feb", "Mrz", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez",
    ],
    days: [
        "Sonntag",
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
    ],
    days_short: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    date_order: DateOrder::DMY,
    short_date_format: "dd.mm.yyyy",
    long_date_format: "dddd, d. mmmm yyyy",
    time_format: "hh:mm:ss",
    currency_symbol: "€",
    currency_format: "#,##0.00 €",
    accounting_format: "_-* #,##0.00 €_-;-* #,##0.00 €_-;_-* \"-\"?? €_-;_-@_-",
    sheet_prefix: "Tabelle",
    table_prefix: "Tabelle",
    column_prefix: "Spalte",
    total_label: "Ergebnis",
    date_sep: '.',
    fmt_day: 't',
    fmt_year: 'j',
    display_name: "Deutsch",
    table_items: [
        "#Alle",
        "#Daten",
        "#Kopfzeilen",
        "#Ergebnisse",
        "#Diese Zeile",
    ],
    table_item_aliases: &[],
    r1c1: ('Z', 'S'),
    fn_pairs: names_de::FUNCTIONS,
    maps: OnceLock::new(),
};

static IT: Locale = Locale {
    id: "it",
    decimal_sep: ',',
    thousands_sep: '.',
    arg_sep: ';',
    // Italian Excel writes `{1\2\3.4\5\6}`: `\` between columns, `.` between rows.
    array_col_sep: '\\',
    array_row_sep: '.',
    true_name: "VERO",
    false_name: "FALSO",
    errors: [
        "#NULLO!",
        "#DIV/0!",
        "#VALORE!",
        "#RIF!",
        "#NOME?",
        "#NUM!",
        "#N/D",
        "#ESPANSIONE!",
        "#CALC!",
    ],
    months: [
        "gennaio",
        "febbraio",
        "marzo",
        "aprile",
        "maggio",
        "giugno",
        "luglio",
        "agosto",
        "settembre",
        "ottobre",
        "novembre",
        "dicembre",
    ],
    months_short: [
        "gen", "feb", "mar", "apr", "mag", "giu", "lug", "ago", "set", "ott", "nov", "dic",
    ],
    days: [
        "domenica",
        "lunedì",
        "martedì",
        "mercoledì",
        "giovedì",
        "venerdì",
        "sabato",
    ],
    days_short: ["dom", "lun", "mar", "mer", "gio", "ven", "sab"],
    date_order: DateOrder::DMY,
    short_date_format: "dd/mm/yyyy",
    long_date_format: "dddd d mmmm yyyy",
    time_format: "hh:mm:ss",
    currency_symbol: "€",
    currency_format: "#,##0.00 €",
    accounting_format: "_-* #,##0.00 €_-;-* #,##0.00 €_-;_-* \"-\"?? €_-;_-@_-",
    sheet_prefix: "Foglio",
    table_prefix: "Tabella",
    column_prefix: "Colonna",
    total_label: "Totale",
    date_sep: '/',
    fmt_day: 'g',
    fmt_year: 'a',
    display_name: "Italiano",
    table_items: [
        "#Tutti",
        "#Dati",
        "#Intestazioni",
        "#Totali",
        "#Questa riga",
    ],
    table_item_aliases: &[],
    r1c1: ('R', 'C'),
    fn_pairs: names_it::FUNCTIONS,
    maps: OnceLock::new(),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general() {
        assert_eq!(general_number(1.0, 15), "1");
        assert_eq!(general_number(-1.5, 15), "-1.5");
        assert_eq!(general_number(1.0 / 3.0, 15), "0.333333333333333");
        assert_eq!(general_number(0.1 + 0.2, 15), "0.3");
        assert_eq!(general_number(1e15, 15), "1E+15");
        assert_eq!(general_number(123456789012345.0, 15), "123456789012345");
        assert_eq!(general_number(0.0001, 15), "0.0001");
        assert_eq!(general_number(1.5e-10, 15), "1.5E-10");
    }

    #[test]
    fn numbers() {
        assert_eq!(Locale::en().parse_number("1,234.5"), Some(1234.5));
        assert_eq!(Locale::fr().parse_number("1 234,5"), Some(1234.5));
        assert_eq!(Locale::fr().parse_number("12,5%"), Some(0.125));
        assert_eq!(Locale::en().parse_number("abc"), None);
        assert_eq!(Locale::en().parse_number("1e3"), Some(1000.0));
        assert_eq!(Locale::fr().fn_to_canonical("SOMME"), "SUM");
        assert_eq!(Locale::fr().fn_to_local("VLOOKUP"), "RECHERCHEV");
        assert_eq!(Locale::fr().fn_to_canonical("SUM"), "SUM");
        assert_eq!(Locale::de().parse_number("1.000"), Some(1000.0));
        assert_eq!(Locale::de().parse_number("1.234.567,5"), Some(1234567.5));
        assert_eq!(Locale::de().parse_number("1.5"), None);
        assert_eq!(Locale::en().parse_number("1,5"), None);
        assert_eq!(Locale::en().parse_number_lenient("1,5"), Some(15.0));
        assert_eq!(Locale::by_id("de-DE").id, "de");
        assert_eq!(Locale::by_id("es_ES").id, "es");
        assert_eq!(Locale::by_id("it").id, "it");
        assert_eq!(Locale::by_id("ja").id, "en");
        assert_eq!(Locale::de().fn_to_canonical("SUMME"), "SUM");
        assert_eq!(Locale::es().fn_to_local("SUM"), "SUMA");
        assert_eq!(Locale::it().fn_to_local("IF"), "SE");
    }
}
