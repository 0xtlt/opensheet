//! Locale data: separators, localized function / error / boolean names,
//! month and day names. Formulas are stored canonically (English names) and
//! translated at the parse / print boundary.

use crate::value::ErrorKind;
use std::collections::HashMap;
use std::sync::OnceLock;

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
    fn_pairs: &'static [(&'static str, &'static str)],
    maps: OnceLock<(HashMap<&'static str, &'static str>, HashMap<&'static str, &'static str>)>,
}

impl Locale {
    pub fn en() -> &'static Locale {
        &EN
    }

    pub fn fr() -> &'static Locale {
        &FR
    }

    pub fn by_id(id: &str) -> &'static Locale {
        if id.to_ascii_lowercase().starts_with("fr") {
            &FR
        } else {
            &EN
        }
    }

    fn maps(&self) -> &(HashMap<&'static str, &'static str>, HashMap<&'static str, &'static str>) {
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
    /// trailing `%`). Used for implicit text → number coercion.
    pub fn parse_number(&self, s: &str) -> Option<f64> {
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
        let chars: Vec<char> = t.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            match ch {
                '0'..='9' => {
                    seen_digit = true;
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
                    seen_exp = true;
                    out.push('e');
                }
                c if c == self.decimal_sep => {
                    if seen_dec || seen_exp {
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
                }
                _ => return None,
            }
        }
        if !seen_digit {
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
    fn_pairs: FR_FUNCTIONS,
    maps: OnceLock::new(),
};

/// (canonical English name, French name). Functions spelled identically in
/// both languages are omitted.
const FR_FUNCTIONS: &[(&str, &str)] = &[
    // Math
    ("SUM", "SOMME"),
    ("SUMIF", "SOMME.SI"),
    ("SUMIFS", "SOMME.SI.ENS"),
    ("SUMPRODUCT", "SOMMEPROD"),
    ("SUMSQ", "SOMME.CARRES"),
    ("SUMX2MY2", "SOMME.X2MY2"),
    ("SUMX2PY2", "SOMME.X2PY2"),
    ("SUMXMY2", "SOMME.XMY2"),
    ("PRODUCT", "PRODUIT"),
    ("SIGN", "SIGNE"),
    ("SQRT", "RACINE"),
    ("SQRTPI", "RACINE.PI"),
    ("POWER", "PUISSANCE"),
    ("ROUND", "ARRONDI"),
    ("ROUNDUP", "ARRONDI.SUP"),
    ("ROUNDDOWN", "ARRONDI.INF"),
    ("MROUND", "ARRONDI.AU.MULTIPLE"),
    ("INT", "ENT"),
    ("TRUNC", "TRONQUE"),
    ("CEILING", "PLAFOND"),
    ("FLOOR", "PLANCHER"),
    ("CEILING.MATH", "PLAFOND.MATH"),
    ("FLOOR.MATH", "PLANCHER.MATH"),
    ("CEILING.PRECISE", "PLAFOND.PRECIS"),
    ("FLOOR.PRECISE", "PLANCHER.PRECIS"),
    ("EVEN", "PAIR"),
    ("ODD", "IMPAIR"),
    ("RAND", "ALEA"),
    ("RANDBETWEEN", "ALEA.ENTRE.BORNES"),
    ("RANDARRAY", "TABLEAU.ALEAT"),
    ("DEGREES", "DEGRES"),
    ("PERMUT", "PERMUTATION"),
    ("GCD", "PGCD"),
    ("LCM", "PPCM"),
    ("SUBTOTAL", "SOUS.TOTAL"),
    ("AGGREGATE", "AGREGAT"),
    ("ROMAN", "ROMAIN"),
    ("ARABIC", "CHIFFRE.ARABE"),
    ("MMULT", "PRODUITMAT"),
    ("MINVERSE", "INVERSEMAT"),
    ("MDETERM", "DETERMAT"),
    ("MUNIT", "MATRICE.UNITAIRE"),
    // Statistical
    ("AVERAGE", "MOYENNE"),
    ("AVERAGEIF", "MOYENNE.SI"),
    ("AVERAGEIFS", "MOYENNE.SI.ENS"),
    ("COUNT", "NB"),
    ("COUNTA", "NBVAL"),
    ("COUNTBLANK", "NB.VIDE"),
    ("COUNTIF", "NB.SI"),
    ("COUNTIFS", "NB.SI.ENS"),
    ("MAXIFS", "MAX.SI.ENS"),
    ("MINIFS", "MIN.SI.ENS"),
    ("MEDIAN", "MEDIANE"),
    ("MODE.SNGL", "MODE.SIMPLE"),
    ("MODE.MULT", "MODE.MULTIPLE"),
    ("STDEV", "ECARTYPE"),
    ("STDEV.S", "ECARTYPE.STANDARD"),
    ("STDEV.P", "ECARTYPE.PEARSON"),
    ("STDEVP", "ECARTYPEP"),
    ("VAR.P", "VAR.P.N"),
    ("VARP", "VAR.P"),
    ("LARGE", "GRANDE.VALEUR"),
    ("SMALL", "PETITE.VALEUR"),
    ("RANK", "RANG"),
    ("RANK.EQ", "EQUATION.RANG"),
    ("RANK.AVG", "MOYENNE.RANG"),
    ("PERCENTILE", "CENTILE"),
    ("PERCENTILE.INC", "CENTILE.INCLURE"),
    ("PERCENTILE.EXC", "CENTILE.EXCLURE"),
    ("QUARTILE.INC", "QUARTILE.INCLURE"),
    ("QUARTILE.EXC", "QUARTILE.EXCLURE"),
    ("PERCENTRANK", "RANG.POURCENTAGE"),
    ("PERCENTRANK.INC", "RANG.POURCENTAGE.INCLURE"),
    ("PERCENTRANK.EXC", "RANG.POURCENTAGE.EXCLURE"),
    ("CORREL", "COEFFICIENT.CORRELATION"),
    ("COVAR", "COVARIANCE"),
    ("COVARIANCE.P", "COVARIANCE.PEARSON"),
    ("COVARIANCE.S", "COVARIANCE.STANDARD"),
    ("SLOPE", "PENTE"),
    ("INTERCEPT", "ORDONNEE.ORIGINE"),
    ("RSQ", "COEFFICIENT.DETERMINATION"),
    ("FORECAST", "PREVISION"),
    ("FORECAST.LINEAR", "PREVISION.LINEAIRE"),
    ("TREND", "TENDANCE"),
    ("GROWTH", "CROISSANCE"),
    ("LINEST", "DROITEREG"),
    ("GEOMEAN", "MOYENNE.GEOMETRIQUE"),
    ("HARMEAN", "MOYENNE.HARMONIQUE"),
    ("TRIMMEAN", "MOYENNE.REDUITE"),
    ("AVEDEV", "ECART.MOYEN"),
    ("DEVSQ", "SOMME.CARRES.ECARTS"),
    ("SKEW", "COEFFICIENT.ASYMETRIE"),
    ("KURT", "KURTOSIS"),
    ("STANDARDIZE", "CENTREE.REDUITE"),
    ("NORM.DIST", "LOI.NORMALE.N"),
    ("NORMDIST", "LOI.NORMALE"),
    ("NORM.INV", "LOI.NORMALE.INVERSE.N"),
    ("NORMINV", "LOI.NORMALE.INVERSE"),
    ("NORM.S.DIST", "LOI.NORMALE.STANDARD.N"),
    ("NORMSDIST", "LOI.NORMALE.STANDARD"),
    ("NORM.S.INV", "LOI.NORMALE.STANDARD.INVERSE.N"),
    ("NORMSINV", "LOI.NORMALE.STANDARD.INVERSE"),
    ("FREQUENCY", "FREQUENCE"),
    ("BINOM.DIST", "LOI.BINOMIALE.N"),
    ("BINOMDIST", "LOI.BINOMIALE"),
    ("POISSON.DIST", "LOI.POISSON.N"),
    ("POISSON", "LOI.POISSON"),
    ("EXPON.DIST", "LOI.EXPONENTIELLE.N"),
    ("EXPONDIST", "LOI.EXPONENTIELLE"),
    ("CONFIDENCE", "INTERVALLE.CONFIANCE"),
    ("CONFIDENCE.NORM", "INTERVALLE.CONFIANCE.NORMAL"),
    // Logical
    ("IF", "SI"),
    ("IFS", "SI.CONDITIONS"),
    ("IFERROR", "SIERREUR"),
    ("IFNA", "SI.NON.DISP"),
    ("AND", "ET"),
    ("OR", "OU"),
    ("NOT", "NON"),
    ("XOR", "OUX"),
    ("TRUE", "VRAI"),
    ("FALSE", "FAUX"),
    ("SWITCH", "SI.MULTIPLE"),
    // Text
    ("CONCATENATE", "CONCATENER"),
    ("TEXTJOIN", "JOINDRE.TEXTE"),
    ("LEFT", "GAUCHE"),
    ("RIGHT", "DROITE"),
    ("MID", "STXT"),
    ("LEN", "NBCAR"),
    ("UPPER", "MAJUSCULE"),
    ("LOWER", "MINUSCULE"),
    ("PROPER", "NOMPROPRE"),
    ("TRIM", "SUPPRESPACE"),
    ("CLEAN", "EPURAGE"),
    ("SUBSTITUTE", "SUBSTITUE"),
    ("REPLACE", "REMPLACER"),
    ("FIND", "TROUVE"),
    ("SEARCH", "CHERCHE"),
    ("TEXT", "TEXTE"),
    ("VALUE", "CNUM"),
    ("NUMBERVALUE", "VALEURNOMBRE"),
    ("CHAR", "CAR"),
    ("UNICHAR", "UNICAR"),
    ("FIXED", "CTXT"),
    ("DOLLAR", "DEVISE"),
    ("TEXTBEFORE", "TEXTE.AVANT"),
    ("TEXTAFTER", "TEXTE.APRES"),
    ("TEXTSPLIT", "FRACTIONNER.TEXTE"),
    ("VALUETOTEXT", "VALEUR.EN.TEXTE"),
    ("ARRAYTOTEXT", "TABLEAU.EN.TEXTE"),
    // Date & time
    ("TODAY", "AUJOURDHUI"),
    ("NOW", "MAINTENANT"),
    ("DATEVALUE", "DATEVAL"),
    ("TIME", "TEMPS"),
    ("TIMEVALUE", "TEMPSVAL"),
    ("YEAR", "ANNEE"),
    ("MONTH", "MOIS"),
    ("DAY", "JOUR"),
    ("HOUR", "HEURE"),
    ("SECOND", "SECONDE"),
    ("WEEKDAY", "JOURSEM"),
    ("WEEKNUM", "NO.SEMAINE"),
    ("ISOWEEKNUM", "NO.SEMAINE.ISO"),
    ("EDATE", "MOIS.DECALER"),
    ("EOMONTH", "FIN.MOIS"),
    ("DAYS", "JOURS"),
    ("DAYS360", "JOURS360"),
    ("NETWORKDAYS", "NB.JOURS.OUVRES"),
    ("NETWORKDAYS.INTL", "NB.JOURS.OUVRES.INTL"),
    ("WORKDAY", "SERIE.JOUR.OUVRE"),
    ("WORKDAY.INTL", "SERIE.JOUR.OUVRE.INTL"),
    ("YEARFRAC", "FRACTION.ANNEE"),
    // Lookup & reference
    ("VLOOKUP", "RECHERCHEV"),
    ("HLOOKUP", "RECHERCHEH"),
    ("XLOOKUP", "RECHERCHEX"),
    ("LOOKUP", "RECHERCHE"),
    ("MATCH", "EQUIV"),
    ("XMATCH", "EQUIVX"),
    ("CHOOSE", "CHOISIR"),
    ("ROW", "LIGNE"),
    ("COLUMN", "COLONNE"),
    ("ROWS", "LIGNES"),
    ("COLUMNS", "COLONNES"),
    ("OFFSET", "DECALER"),
    ("ADDRESS", "ADRESSE"),
    ("AREAS", "ZONES"),
    ("FILTER", "FILTRE"),
    ("SORT", "TRIER"),
    ("SORTBY", "TRIERPAR"),
    ("HYPERLINK", "LIEN_HYPERTEXTE"),
    ("FORMULATEXT", "FORMULETEXTE"),
    ("TAKE", "PRENDRE"),
    ("DROP", "EXCLURE"),
    ("CHOOSECOLS", "CHOISIRCOLS"),
    ("CHOOSEROWS", "CHOISIRLIGNES"),
    ("VSTACK", "ASSEMB.V"),
    ("HSTACK", "ASSEMB.H"),
    ("TOCOL", "DANSCOL"),
    ("TOROW", "DANSLIGNE"),
    ("WRAPROWS", "ORGA.LIGNES"),
    ("WRAPCOLS", "ORGA.COLS"),
    ("EXPAND", "ETENDRE"),
    // Information
    ("ISBLANK", "ESTVIDE"),
    ("ISNUMBER", "ESTNUM"),
    ("ISTEXT", "ESTTEXTE"),
    ("ISNONTEXT", "ESTNONTEXTE"),
    ("ISERROR", "ESTERREUR"),
    ("ISERR", "ESTERR"),
    ("ISNA", "ESTNA"),
    ("ISLOGICAL", "ESTLOGIQUE"),
    ("ISEVEN", "EST.PAIR"),
    ("ISODD", "EST.IMPAIR"),
    ("ISREF", "ESTREF"),
    ("ISFORMULA", "ESTFORMULE"),
    ("ERROR.TYPE", "TYPE.ERREUR"),
    ("CELL", "CELLULE"),
    ("INFO", "INFORMATIONS"),
    ("SHEET", "FEUILLE"),
    ("SHEETS", "FEUILLES"),
    // Financial
    ("PMT", "VPM"),
    ("PV", "VA"),
    ("FV", "VC"),
    ("NPV", "VAN"),
    ("IRR", "TRI"),
    ("XNPV", "VAN.PAIEMENTS"),
    ("XIRR", "TRI.PAIEMENTS"),
    ("MIRR", "TRIM"),
    ("RATE", "TAUX"),
    ("NPER", "NPM"),
    ("IPMT", "INTPER"),
    ("PPMT", "PRINCPER"),
    ("CUMIPMT", "CUMUL.INTER"),
    ("CUMPRINC", "CUMUL.PRINCPER"),
    ("SLN", "AMORLIN"),
    ("EFFECT", "TAUX.EFFECTIF"),
    ("NOMINAL", "TAUX.NOMINAL"),
    // Engineering
    ("DEC2BIN", "DECBIN"),
    ("BIN2DEC", "BINDEC"),
    ("DEC2HEX", "DECHEX"),
    ("HEX2DEC", "HEXDEC"),
    ("DEC2OCT", "DECOCT"),
    ("OCT2DEC", "OCTDEC"),
    ("BITAND", "BITET"),
    ("BITOR", "BITOU"),
    ("BITXOR", "BITOUEXCLUSIF"),
    ("BITLSHIFT", "BITDECALG"),
    ("BITRSHIFT", "BITDECALD"),
    ("BIN2HEX", "BINHEX"),
    ("BIN2OCT", "BINOCT"),
    ("HEX2BIN", "HEXBIN"),
    ("HEX2OCT", "HEXOCT"),
    ("OCT2BIN", "OCTBIN"),
    ("OCT2HEX", "OCTHEX"),
    ("GESTEP", "SUP.SEUIL"),
    // Later additions
    ("ISO.CEILING", "ISO.PLAFOND"),
    ("MULTINOMIAL", "MULTINOMIALE"),
    ("SERIESSUM", "SOMME.SERIES"),
    ("STEYX", "ERREUR.TYPE.XY"),
    ("FISHERINV", "FISHER.INVERSE"),
    ("GAMMALN", "LNGAMMA"),
    ("LEFTB", "GAUCHEB"),
    ("RIGHTB", "DROITEB"),
    ("REPLACEB", "REMPLACERB"),
    ("FINDB", "TROUVERB"),
    ("SEARCHB", "CHERCHERB"),
];

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
    }
}
