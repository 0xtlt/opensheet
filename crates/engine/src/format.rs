//! Number formatting: applies a number format code ("0.00", "dd/mm/yyyy",
//! "#,##0.00 €", ...) to a value.
//!
//! Format codes are stored in canonical (US) syntax: `.` is the decimal
//! point and `,` the thousands separator inside the code; the locale decides
//! which characters are actually displayed.
//!
//! A code is lexed and analysed once into [`Parsed`] sections (cached per
//! thread), then rendered for each value:
//!
//! * up to four sections `positive;negative;zero;text`, with optional
//!   conditions (`[>100]`) and colours (`[Red]`, `[Color 5]`);
//! * numeric sections: `0 # ?` placeholders, thousands separators, scaling
//!   commas, percent, scientific notation and fractions;
//! * date / time sections, including elapsed time (`[h]:mm`);
//! * text sections (`@`) and plain literals.

use crate::datetime::{serial_to_ymd, weekday};
use crate::locale::Locale;
use crate::style::Color;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Result of formatting a value for display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Formatted {
    pub text: String,
    /// Colour requested by the format code (`[Red]`), if any.
    pub color: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatKind {
    General,
    Number,
    Currency,
    Accounting,
    Date,
    Time,
    DateTime,
    Percent,
    Fraction,
    Scientific,
    Text,
    Custom,
}

/// Shown when a value cannot be rendered with its format (negative dates,
/// no section matching the value), like a too-narrow column in the grid.
const HASHES: &str = "########";

/// Characters a General number may use in a default-width cell.
const GENERAL_CHARS: usize = 11;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Format any value with a number format code.
pub fn format_value(value: &Value, code: &str, locale: &Locale) -> Formatted {
    match value {
        Value::Empty => Formatted::default(),
        Value::Number(n) => format_number(*n, code, locale),
        Value::Text(s) => format_text(s, code),
        Value::Bool(b) => Formatted {
            text: locale.bool_name(*b).to_string(),
            color: None,
        },
        Value::Error(e) => Formatted {
            text: locale.error_name(*e).to_string(),
            color: None,
        },
        Value::Array(a) => format_value(a.get(0, 0), code, locale),
    }
}

/// Format a number with a number format code. Values the code cannot
/// represent (negative or out-of-range dates) give `########`.
pub fn format_number(n: f64, code: &str, locale: &Locale) -> Formatted {
    try_format_number(n, code, locale).unwrap_or_else(|| Formatted {
        text: HASHES.to_string(),
        color: None,
    })
}

/// Like [`format_number`], but `None` when the value cannot be represented
/// with this code (where the grid shows `########` and `TEXT` fails).
pub fn try_format_number(n: f64, code: &str, locale: &Locale) -> Option<Formatted> {
    if is_general_code(code) {
        return Some(Formatted {
            text: format_general(n, locale, GENERAL_CHARS),
            color: None,
        });
    }
    parsed(code).format_number(n, locale)
}

/// Format a text value: it goes through the text section of the code (the
/// fourth section, or a last section containing `@`) when there is one.
fn format_text(s: &str, code: &str) -> Formatted {
    if !is_general_code(code) {
        let p = parsed(code);
        if let Some(sec) = p.text_section()
            && let Body::Text(toks) = &sec.body
        {
            return Formatted {
                text: render_text(toks, s),
                color: sec.color,
            };
        }
    }
    Formatted {
        text: s.to_string(),
        color: None,
    }
}

/// "General" rendering of a number limited to `max_chars` characters
/// (Excel shows at most 11 in a default-width cell; the sign does not
/// count), switching to scientific notation when the number does not fit.
pub fn format_general(n: f64, locale: &Locale, max_chars: usize) -> String {
    if n == 0.0 || !n.is_finite() {
        return "0".to_string();
    }
    let max = max_chars.max(1);
    let d = Dec::new(n);
    let body = general_fixed(&d, max).unwrap_or_else(|| general_sci(&d, max));
    let mut out = String::with_capacity(body.len() + 1);
    if n < 0.0 {
        out.push('-');
    }
    for c in body.chars() {
        out.push(if c == '.' { locale.decimal_sep } else { c });
    }
    out
}

/// Classify a format code (drives the ribbon's number-format box).
pub fn format_kind(code: &str) -> FormatKind {
    if is_general_code(code) {
        return FormatKind::General;
    }
    let p = parsed(code);
    let Some(first) = p.sections.first() else {
        return FormatKind::General;
    };
    match &first.body {
        Body::Date(d) => match (d.has_date, d.has_time) {
            (true, true) => FormatKind::DateTime,
            (false, true) => FormatKind::Time,
            _ => FormatKind::Date,
        },
        Body::Text(toks) => {
            if p.sections.len() == 1 && toks.len() == 1 && toks[0] == TextTok::At {
                FormatKind::Text
            } else {
                FormatKind::Custom
            }
        }
        Body::Number(nf) => match nf.kind {
            NumKind::General => {
                if p.sections.len() == 1 && nf.toks.len() == 1 && first.color.is_none() {
                    FormatKind::General
                } else {
                    FormatKind::Custom
                }
            }
            NumKind::Fraction => FormatKind::Fraction,
            NumKind::Scientific => FormatKind::Scientific,
            NumKind::Plain => {
                let any = |f: fn(&Flags) -> bool| p.sections.iter().any(|s| f(&s.flags));
                if first.flags.percent {
                    FormatKind::Percent
                } else if any(|f| f.fill) && (any(|f| f.currency) || any(|f| f.skip)) {
                    FormatKind::Accounting
                } else if first.flags.currency {
                    FormatKind::Currency
                } else if p.sections.iter().all(|s| s.is_simple_number()) {
                    FormatKind::Number
                } else {
                    FormatKind::Custom
                }
            }
        },
    }
}

/// True when the code formats its value as a date and/or time.
pub fn is_date_format(code: &str) -> bool {
    if is_general_code(code) {
        return false;
    }
    // Cheap rejection before touching the cache: no date letter at all.
    if !code.contains(['y', 'Y', 'm', 'M', 'd', 'D', 'h', 'H', 's', 'S', '$']) {
        return false;
    }
    parsed(code)
        .sections
        .iter()
        .take(3)
        .any(|s| matches!(s.body, Body::Date(_)))
}

/// The code with `delta` more (or fewer) decimal places, as done by the
/// "Increase Decimal" / "Decrease Decimal" buttons. `General` becomes
/// `0`, `0.0`, ... Date and text codes are returned unchanged.
pub fn adjust_decimals(code: &str, delta: i32) -> String {
    if delta == 0 {
        return code.to_string();
    }
    let mut out = if code.trim().is_empty() {
        "General".to_string()
    } else {
        code.to_string()
    };
    for _ in 0..delta.unsigned_abs().min(MAX_DECIMALS as u32) {
        let next = adjust_step(&out, delta > 0);
        if next == out {
            break;
        }
        out = next;
    }
    out
}

/// Translate a format code written in the locale's own syntax, as typed in
/// the `TEXT` function (`"0,00"`, `"# ##0,00 €"`, `"jj/mm/aaaa"` in French),
/// to canonical syntax. Canonical codes are recognised and left untouched,
/// so both spellings work; in an English locale this is the identity.
pub fn code_from_locale(code: &str, locale: &Locale) -> String {
    let (day, year) = (locale.fmt_day, locale.fmt_year);
    if locale.decimal_sep == '.' && day == 'd' && year == 'y' {
        return code.to_string();
    }
    let tokens = lex(code);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let date_alias = |t: &Token| match t.lex {
        Lex::Char(ch) if day != 'd' && ch.to_ascii_lowercase() == day => Some("d"),
        Lex::Char(ch) if year != 'y' && ch.to_ascii_lowercase() == year => Some("y"),
        _ => None,
    };
    let is_digit = |t: Option<&Token>| t.is_some_and(|t| matches!(t.lex, Lex::Digit(_)));
    let is_space = |t: &Token| matches!(t.lex, Lex::Char(' ' | '\u{a0}' | '\u{202f}'));
    for sec in tokens.split(|t| t.lex == Lex::Sep) {
        let date = raw_kind(sec) == RawKind::Date || sec.iter().any(|t| date_alias(t).is_some());
        if date {
            for (i, t) in sec.iter().enumerate() {
                if let Some(letter) = date_alias(t) {
                    edits.push((t.start, t.end, letter.to_string()));
                } else if t.lex == Lex::Comma
                    && i > 0
                    && matches!(sec[i - 1].lex, Lex::Date('s', _) | Lex::Elapsed('s', _))
                    && sec.get(i + 1).is_some_and(|n| n.lex == Lex::Digit('0'))
                    && locale.decimal_sep == ','
                {
                    // Fractional seconds: "ss,00".
                    edits.push((t.start, t.end, ".".to_string()));
                }
            }
            continue;
        }
        if locale.decimal_sep != ',' {
            continue;
        }
        let between_digits =
            |i: usize| i > 0 && is_digit(sec.get(i - 1)) && is_digit(sec.get(i + 1));
        let space_grouping = locale.thousands_sep.is_whitespace()
            && sec
                .iter()
                .enumerate()
                .any(|(i, t)| is_space(t) && between_digits(i));
        let has_dot = sec.iter().any(|t| t.lex == Lex::Dot);
        let commas: Vec<usize> = sec
            .iter()
            .enumerate()
            .filter(|(_, t)| t.lex == Lex::Comma)
            .map(|(i, _)| i)
            .collect();
        // A lone comma followed by digit placeholders is the decimal comma,
        // unless it is the classic canonical grouping pattern `#,##0`.
        let decimal_comma = !has_dot && commas.len() == 1 && {
            let i = commas[0];
            let after = sec[i + 1..]
                .iter()
                .take_while(|t| matches!(t.lex, Lex::Digit(_)))
                .count();
            let before: Vec<&Token> = sec[..i]
                .iter()
                .rev()
                .take_while(|t| matches!(t.lex, Lex::Digit(_)))
                .collect();
            let canonical_grouping =
                after == 3 && !before.is_empty() && before.iter().all(|t| t.lex == Lex::Digit('#'));
            after > 0 && !canonical_grouping
        };
        if locale.thousands_sep == '.' {
            // Spanish / German / Italian: "#.##0,00" is "#,##0.00".
            let dots: Vec<usize> = sec
                .iter()
                .enumerate()
                .filter(|(_, t)| t.lex == Lex::Dot)
                .map(|(i, _)| i)
                .collect();
            let localized = match (commas.last(), dots.is_empty()) {
                (Some(c), false) => dots.iter().all(|d| d < c),
                (Some(_), true) => decimal_comma,
                (None, false) => {
                    // A lone dot is grouping only in the `#.##0` shape.
                    dots.len() == 1 && {
                        let i = dots[0];
                        let after = sec[i + 1..]
                            .iter()
                            .take_while(|t| matches!(t.lex, Lex::Digit(_)))
                            .count();
                        let before: Vec<&Token> = sec[..i]
                            .iter()
                            .rev()
                            .take_while(|t| matches!(t.lex, Lex::Digit(_)))
                            .collect();
                        after == 3 && !before.is_empty() && before.iter().all(|t| t.lex == Lex::Digit('#'))
                    }
                }
                (None, true) => false,
            };
            if localized {
                for t in sec.iter() {
                    match t.lex {
                        Lex::Comma => edits.push((t.start, t.end, ".".to_string())),
                        Lex::Dot => edits.push((t.start, t.end, ",".to_string())),
                        _ => {}
                    }
                }
            }
            continue;
        }
        if !(space_grouping || decimal_comma) {
            continue;
        }
        for (i, t) in sec.iter().enumerate() {
            match t.lex {
                Lex::Comma => edits.push((t.start, t.end, ".".to_string())),
                Lex::Dot => edits.push((t.start, t.end, "\\.".to_string())),
                _ if is_space(t) && between_digits(i) && space_grouping => {
                    edits.push((t.start, t.end, ",".to_string()))
                }
                _ => {}
            }
        }
    }
    apply_edits(code, edits)
}

// ---------------------------------------------------------------------------
// Decimal arithmetic
// ---------------------------------------------------------------------------

/// Decimal view of the magnitude of a number, limited to the 15 significant
/// digits a spreadsheet displays: `0.d1 d2 ... dn × 10^exp`. Rounding on this
/// representation is what makes 2.675 → "2.68" and 1.005 → "1.01".
#[derive(Clone, Debug)]
struct Dec {
    /// Significant digits (0..=9), no leading or trailing zero.
    digits: Vec<u8>,
    exp: i32,
}

impl Dec {
    fn new(n: f64) -> Dec {
        let n = n.abs();
        if n == 0.0 || !n.is_finite() {
            return Dec {
                digits: Vec::new(),
                exp: 0,
            };
        }
        let s = format!("{n:.14e}");
        let (mant, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
        let mut digits: Vec<u8> = mant
            .bytes()
            .filter(u8::is_ascii_digit)
            .map(|b| b - b'0')
            .collect();
        while digits.last() == Some(&0) {
            digits.pop();
        }
        Dec {
            digits,
            exp: exp.parse::<i32>().unwrap_or(0) + 1,
        }
    }

    fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// The value multiplied by `10^pow`.
    fn shifted(mut self, pow: i32) -> Dec {
        if !self.is_zero() {
            self.exp += pow;
        }
        self
    }

    /// Round half away from zero to `places` decimals. Returns the integer
    /// digits (no leading zero, empty for 0) and exactly `places` fraction
    /// digits.
    fn round(&self, places: usize) -> (String, String) {
        let mut exp = self.exp as i64;
        let cut = exp + places as i64;
        if cut < 0 {
            return (String::new(), "0".repeat(places));
        }
        let cut = cut as usize;
        let mut kept: Vec<u8>;
        if cut >= self.digits.len() {
            kept = self.digits.clone();
            kept.resize(cut, 0);
        } else {
            kept = self.digits[..cut].to_vec();
            if self.digits[cut] >= 5 {
                let mut carry = true;
                for d in kept.iter_mut().rev() {
                    if *d == 9 {
                        *d = 0;
                    } else {
                        *d += 1;
                        carry = false;
                        break;
                    }
                }
                if carry {
                    kept.insert(0, 1);
                    exp += 1;
                }
            }
        }
        // `kept` now holds exp + places digits.
        let to_string = |d: &[u8]| d.iter().map(|x| (b'0' + x) as char).collect::<String>();
        if exp <= 0 {
            let mut frac = "0".repeat((-exp) as usize);
            frac.push_str(&to_string(&kept));
            (String::new(), frac)
        } else {
            let int_len = exp as usize;
            (to_string(&kept[..int_len]), to_string(&kept[int_len..]))
        }
    }
}

fn general_fixed(d: &Dec, max: usize) -> Option<String> {
    if d.exp > max as i32 {
        return None;
    }
    let sig = d.digits.len() as i32;
    let int_len = d.exp.max(1) as usize;
    let needed = (sig - d.exp).max(0) as usize;
    let avail = max.saturating_sub(int_len + 1);
    let places = needed.min(avail);
    if places < needed && d.exp <= 0 {
        // A small number losing digits stays in fixed notation only while it
        // keeps as many significant digits as scientific notation would.
        let kept = places as i32 + d.exp;
        let sci_capacity = (max as i32 - 5).max(1);
        if kept < sig.min(sci_capacity) {
            return None;
        }
    }
    let (int, frac) = d.round(places);
    if int.len() > max {
        return None;
    }
    let frac = frac.trim_end_matches('0');
    let mut s = if int.is_empty() { "0".to_string() } else { int };
    if !frac.is_empty() {
        s.push('.');
        s.push_str(frac);
    }
    Some(s)
}

fn general_sci(d: &Dec, max: usize) -> String {
    let mut e = d.exp - 1;
    let exp_len = if e.abs() >= 100 { 3 } else { 2 };
    let places = max
        .saturating_sub(4 + exp_len)
        .min(d.digits.len().saturating_sub(1));
    let mant = Dec {
        digits: d.digits.clone(),
        exp: 1,
    };
    let (mut int, mut frac) = mant.round(places);
    if int.len() > 1 {
        e += 1;
        int = "1".to_string();
        frac = String::new();
    }
    let frac = frac.trim_end_matches('0');
    let mut s = int;
    if !frac.is_empty() {
        s.push('.');
        s.push_str(frac);
    }
    s.push('E');
    s.push(if e < 0 { '-' } else { '+' });
    s.push_str(&format!("{:02}", e.abs()));
    s
}

/// Closest fraction to `x` (≥ 0) with a denominator of at most `max_den`,
/// by continued fractions with a final semiconvergent check.
fn best_fraction(x: f64, max_den: u64) -> (u64, u64) {
    let max_den = max_den.max(1);
    let (mut p0, mut q0, mut p1, mut q1) = (0u64, 1u64, 1u64, 0u64);
    let mut r = x;
    for _ in 0..64 {
        let a = r.floor();
        if a * q1 as f64 + q0 as f64 > max_den as f64 {
            if q1 == 0 {
                break;
            }
            let k = (max_den - q0) / q1;
            let (ps, qs) = (k * p1 + p0, k * q1 + q0);
            let e1 = (x - p1 as f64 / q1 as f64).abs();
            let es = (x - ps as f64 / qs as f64).abs();
            return if k > 0 && es < e1 { (ps, qs) } else { (p1, q1) };
        }
        let a = a as u64;
        let (p2, q2) = (a.saturating_mul(p1).saturating_add(p0), a * q1 + q0);
        (p0, q0, p1, q1) = (p1, q1, p2, q2);
        let f = r - r.floor();
        if f < 1e-10 {
            break;
        }
        r = 1.0 / f;
    }
    (p1, q1)
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

type Cond = (Cmp, f64);

fn cond_holds(c: Cond, n: f64) -> bool {
    match c.0 {
        Cmp::Lt => n < c.1,
        Cmp::Le => n <= c.1,
        Cmp::Gt => n > c.1,
        Cmp::Ge => n >= c.1,
        Cmp::Eq => n == c.1,
        Cmp::Ne => n != c.1,
    }
}

/// The condition only admits values ≤ 0: the section is "the negative
/// section" and the automatic minus sign is dropped.
fn negative_only(c: Cond) -> bool {
    matches!(c.0, Cmp::Lt | Cmp::Le) && c.1 <= 0.0
}

/// Same question for the values *rejected* by the condition.
fn complement_negative_only(c: Cond) -> bool {
    matches!(c.0, Cmp::Gt | Cmp::Ge) && c.1 <= 0.0
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AmPm {
    /// `AM/PM`
    Long,
    /// `A/P`
    ShortUpper,
    /// `a/p`
    ShortLower,
}

#[derive(Clone, Debug, PartialEq)]
enum Lex {
    /// Unquoted character passed through as is.
    Char(char),
    /// `"text"`
    Quoted(String),
    /// `\x`
    Escaped(char),
    /// `0`, `#` or `?`
    Digit(char),
    Dot,
    Comma,
    Percent,
    /// `E+` (true) or `E-`
    Exp(bool),
    Slash,
    At,
    General,
    /// `_x`: leave the width of a character.
    Skip,
    /// `*x`: repeat a character to fill the cell.
    Fill,
    Color(Color),
    Cond(Cmp, f64),
    /// `[$€-40C]`: the symbol part.
    Currency(String),
    /// `[$-F800]` (system long date, true) / `[$-F400]` (system time).
    System(bool),
    /// `[h]`, `[mm]`, `[ss]`
    Elapsed(char, u8),
    /// A run of `y`, `m`, `d`, `h` or `s`.
    Date(char, u8),
    AmPm(AmPm),
    /// Unknown `[tag]`.
    Ignored,
    /// `;`
    Sep,
}

#[derive(Clone, Debug)]
struct Token {
    lex: Lex,
    /// Byte range in the code.
    start: usize,
    end: usize,
}

fn starts_with_ci(chars: &[(usize, char)], word: &str) -> bool {
    let mut it = chars.iter();
    word.chars()
        .all(|w| it.next().is_some_and(|(_, c)| c.to_ascii_lowercase() == w))
}

fn lex(code: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = code.char_indices().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let (start, c) = chars[i];
        let mut j = i + 1;
        let lex = match c {
            '"' => {
                let mut s = String::new();
                while j < n && chars[j].1 != '"' {
                    s.push(chars[j].1);
                    j += 1;
                }
                j = (j + 1).min(n);
                Lex::Quoted(s)
            }
            '\\' => {
                if j < n {
                    j += 1;
                    Lex::Escaped(chars[j - 1].1)
                } else {
                    Lex::Ignored
                }
            }
            '_' => {
                j = (j + 1).min(n);
                Lex::Skip
            }
            '*' => {
                j = (j + 1).min(n);
                Lex::Fill
            }
            '[' => match chars[j..].iter().position(|(_, c)| *c == ']') {
                Some(len) => {
                    let inner: String = chars[j..j + len].iter().map(|(_, c)| *c).collect();
                    j += len + 1;
                    lex_bracket(&inner)
                }
                None => Lex::Char('['),
            },
            ';' => Lex::Sep,
            '0' | '#' | '?' => Lex::Digit(c),
            '.' => Lex::Dot,
            ',' => Lex::Comma,
            '%' => Lex::Percent,
            '/' => Lex::Slash,
            '@' => Lex::At,
            'E' | 'e' if j < n && matches!(chars[j].1, '+' | '-') => {
                j += 1;
                Lex::Exp(chars[j - 1].1 == '+')
            }
            'G' | 'g' if starts_with_ci(&chars[i..], "general") => {
                j = i + 7;
                Lex::General
            }
            'S' | 's' if starts_with_ci(&chars[i..], "standard") => {
                j = i + 8;
                Lex::General
            }
            'A' | 'a' if starts_with_ci(&chars[i..], "am/pm") => {
                j = i + 5;
                Lex::AmPm(AmPm::Long)
            }
            'A' | 'a' if starts_with_ci(&chars[i..], "a/p") => {
                j = i + 3;
                Lex::AmPm(if c == 'a' {
                    AmPm::ShortLower
                } else {
                    AmPm::ShortUpper
                })
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' | 'h' | 'H' | 's' | 'S' => {
                let lower = c.to_ascii_lowercase();
                while j < n && chars[j].1.to_ascii_lowercase() == lower {
                    j += 1;
                }
                Lex::Date(lower, (j - i).min(255) as u8)
            }
            c => Lex::Char(c),
        };
        let end = if j < n { chars[j].0 } else { code.len() };
        out.push(Token { lex, start, end });
        i = j;
    }
    out
}

/// Excel's default 56-colour palette (`[Color 1]` .. `[Color 56]`).
const PALETTE: [u32; 56] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000,
    0x008000, 0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366,
    0xFFFFCC, 0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00,
    0x00FFFF, 0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99,
    0x99CCFF, 0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900,
    0xFF6600, 0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366,
    0x333399, 0x333333,
];

fn named_color(lower: &str) -> Option<Color> {
    let rgb = match lower {
        "black" | "noir" => 0x000000,
        "blue" | "bleu" => 0x0000FF,
        "cyan" => 0x00FFFF,
        "green" | "vert" => 0x00FF00,
        "magenta" => 0xFF00FF,
        "red" | "rouge" => 0xFF0000,
        "white" | "blanc" => 0xFFFFFF,
        "yellow" | "jaune" => 0xFFFF00,
        _ => {
            let index = lower
                .strip_prefix("color")
                .or_else(|| lower.strip_prefix("couleur"))?;
            let i: usize = index.trim().parse().ok()?;
            *PALETTE.get(i.checked_sub(1)?)?
        }
    };
    Some(Color(rgb))
}

fn parse_cond(t: &str) -> Option<Cond> {
    let (op, rest) = if let Some(r) = t.strip_prefix("<=") {
        (Cmp::Le, r)
    } else if let Some(r) = t.strip_prefix("<>") {
        (Cmp::Ne, r)
    } else if let Some(r) = t.strip_prefix(">=") {
        (Cmp::Ge, r)
    } else if let Some(r) = t.strip_prefix('<') {
        (Cmp::Lt, r)
    } else if let Some(r) = t.strip_prefix('>') {
        (Cmp::Gt, r)
    } else {
        (Cmp::Eq, t.strip_prefix('=')?)
    };
    let limit: f64 = rest.trim().parse().ok()?;
    limit.is_finite().then_some((op, limit))
}

/// Interpret the inside of a `[...]` tag.
fn lex_bracket(inner: &str) -> Lex {
    let t = inner.trim();
    if let Some(rest) = t.strip_prefix('$') {
        let (symbol, tag) = rest.split_once('-').unwrap_or((rest, ""));
        if symbol.is_empty() {
            match tag.to_ascii_lowercase().as_str() {
                "f800" | "x-sysdate" => return Lex::System(true),
                "f400" | "x-systime" => return Lex::System(false),
                _ => {}
            }
        }
        return Lex::Currency(symbol.to_string());
    }
    let lower = t.to_ascii_lowercase();
    if let Some(color) = named_color(&lower) {
        return Lex::Color(color);
    }
    if let Some((op, limit)) = parse_cond(t) {
        return Lex::Cond(op, limit);
    }
    if let Some(first) = lower.chars().next()
        && matches!(first, 'h' | 'm' | 's')
        && lower.chars().all(|c| c == first)
    {
        return Lex::Elapsed(first, lower.len().min(255) as u8);
    }
    Lex::Ignored
}

/// Text shown for a token used as a plain literal.
fn literal_of(lex: &Lex) -> Option<String> {
    Some(match lex {
        Lex::Char(c) | Lex::Escaped(c) | Lex::Digit(c) => c.to_string(),
        Lex::Quoted(s) | Lex::Currency(s) => s.clone(),
        Lex::Skip => " ".to_string(),
        Lex::Dot => ".".to_string(),
        Lex::Comma => ",".to_string(),
        Lex::Percent => "%".to_string(),
        Lex::Slash => "/".to_string(),
        Lex::Exp(true) => "E+".to_string(),
        Lex::Exp(false) => "E-".to_string(),
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum RawKind {
    Date,
    Number,
    Text,
}

fn raw_kind(sec: &[Token]) -> RawKind {
    let date = |t: &Token| {
        matches!(
            t.lex,
            Lex::Date(..) | Lex::AmPm(_) | Lex::Elapsed(..) | Lex::System(_)
        )
    };
    if sec.iter().any(date) {
        RawKind::Date
    } else if sec
        .iter()
        .any(|t| matches!(t.lex, Lex::Digit(_) | Lex::General))
    {
        RawKind::Number
    } else {
        RawKind::Text
    }
}

fn is_digit_token(t: &Token) -> bool {
    matches!(t.lex, Lex::Digit(_))
}

/// Shape of a fraction such as `# ?/?` or `# ?/8` inside a section.
struct FractionShape {
    /// Index of the first numerator placeholder.
    numer: usize,
    slash: usize,
    /// One past the last denominator token.
    denom_end: usize,
    fixed: Option<u32>,
}

fn find_fraction(sec: &[Token]) -> Option<FractionShape> {
    for (i, t) in sec.iter().enumerate() {
        if t.lex != Lex::Slash {
            continue;
        }
        let mut numer = i;
        while numer > 0 && is_digit_token(&sec[numer - 1]) {
            numer -= 1;
        }
        if numer == i {
            continue;
        }
        let mut end = i + 1;
        // `/8`, `/16`, `/100`: a fixed denominator.
        if matches!(sec.get(end).map(|t| &t.lex), Some(Lex::Char('1'..='9'))) {
            let mut fixed = 0u32;
            while let Some(t) = sec.get(end) {
                let digit = match t.lex {
                    Lex::Char(c) if c.is_ascii_digit() => c as u32 - '0' as u32,
                    Lex::Digit('0') => 0,
                    _ => break,
                };
                fixed = fixed
                    .saturating_mul(10)
                    .saturating_add(digit)
                    .min(9_999_999);
                end += 1;
            }
            return Some(FractionShape {
                numer,
                slash: i,
                denom_end: end,
                fixed: Some(fixed),
            });
        }
        while end < sec.len() && is_digit_token(&sec[end]) {
            end += 1;
        }
        if end > i + 1 {
            return Some(FractionShape {
                numer,
                slash: i,
                denom_end: end,
                fixed: None,
            });
        }
    }
    None
}

/// Index of the `E+` / `E-` token of a scientific format.
fn exponent_index(sec: &[Token]) -> Option<usize> {
    (0..sec.len()).find(|&i| {
        matches!(sec[i].lex, Lex::Exp(_))
            && sec[..i].iter().any(is_digit_token)
            && sec[i + 1..].iter().any(is_digit_token)
    })
}

// ---------------------------------------------------------------------------
// Parsed format
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct Flags {
    fill: bool,
    skip: bool,
    currency: bool,
    percent: bool,
    /// Literal text other than signs, parentheses and spaces.
    extra_literal: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum NumTok {
    Lit(String),
    Int(char),
    Point,
    Dec(char),
    /// `E` and its sign policy (true: always show the sign).
    Exp(bool),
    ExpDigit(char),
    Numer(char),
    Slash,
    Denom(char),
    DenomFixed(u32),
    General,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumKind {
    Plain,
    Scientific,
    Fraction,
    General,
}

#[derive(Clone, Debug)]
struct NumFmt {
    toks: Vec<NumTok>,
    kind: NumKind,
    int_n: usize,
    dec_n: usize,
    exp_n: usize,
    numer_n: usize,
    denom_n: usize,
    denom_fixed: Option<u32>,
    /// The numerator has a `0` placeholder (the fraction is always shown).
    numer_zero: bool,
    /// The numerator has a `?` placeholder (a hidden fraction keeps its width).
    numer_space: bool,
    grouping: bool,
    /// Power of ten applied to the value: +2 per `%`, −3 per scaling comma.
    scale: i32,
}

#[derive(Clone, Debug, PartialEq)]
enum DateTok {
    Lit(String),
    Year(u8),
    Month(u8),
    Day(u8),
    Hour(u8),
    Minute(u8),
    Second(u8),
    /// Fractional seconds with this many digits.
    SubSec(u8),
    AmPm(AmPm),
    Elapsed(char, u8),
}

#[derive(Clone, Debug)]
struct DateFmt {
    toks: Vec<DateTok>,
    twelve_hour: bool,
    /// Digits of fractional seconds displayed (0..=3).
    subsec: u8,
    has_date: bool,
    has_time: bool,
    /// System long date (true) / time (false) requested by `[$-F800]`.
    system: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
enum TextTok {
    Lit(String),
    At,
}

#[derive(Clone, Debug)]
enum Body {
    Number(NumFmt),
    Date(DateFmt),
    /// A text section (`@`) or plain literals.
    Text(Vec<TextTok>),
}

#[derive(Clone, Debug)]
struct Section {
    color: Option<Color>,
    cond: Option<Cond>,
    body: Body,
    flags: Flags,
}

impl Section {
    /// Usable in a built-in "Number" format: digits, signs, parentheses.
    fn is_simple_number(&self) -> bool {
        let f = &self.flags;
        if f.extra_literal || f.currency || f.percent || self.cond.is_some() {
            return false;
        }
        match &self.body {
            Body::Number(nf) => {
                // Nothing but placeholders between the first and last digit.
                let digit = |t: &NumTok| matches!(t, NumTok::Int(_) | NumTok::Dec(_));
                let first = nf.toks.iter().position(digit).unwrap_or(0);
                let last = nf.toks.iter().rposition(digit).unwrap_or(0);
                nf.kind == NumKind::Plain
                    && nf.scale == 0
                    && nf.toks[first..=last.max(first)]
                        .iter()
                        .all(|t| digit(t) || *t == NumTok::Point)
            }
            Body::Text(_) => true,
            Body::Date(_) => false,
        }
    }
}

#[derive(Debug)]
struct Parsed {
    sections: Vec<Section>,
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Rc<Parsed>>> = RefCell::new(HashMap::new());
}

/// Codes kept in the per-thread cache before it is reset (`TEXT` can
/// generate an unbounded number of distinct codes).
const CACHE_LIMIT: usize = 4096;

fn parsed(code: &str) -> Rc<Parsed> {
    CACHE.with(|cache| {
        if let Some(p) = cache.borrow().get(code) {
            return p.clone();
        }
        let p = Rc::new(parse(code));
        let mut map = cache.borrow_mut();
        if map.len() >= CACHE_LIMIT {
            map.clear();
        }
        map.insert(code.to_string(), p.clone());
        p
    })
}

fn is_general_code(code: &str) -> bool {
    let t = code.trim();
    t.is_empty() || t.eq_ignore_ascii_case("general") || t.eq_ignore_ascii_case("standard")
}

fn parse(code: &str) -> Parsed {
    let tokens = lex(code);
    let sections = tokens
        .split(|t| t.lex == Lex::Sep)
        .take(4)
        .map(build_section)
        .collect();
    Parsed { sections }
}

fn is_currency_char(c: char) -> bool {
    matches!(
        c,
        '$' | '€'
            | '£'
            | '¥'
            | '¢'
            | '₩'
            | '₽'
            | '₹'
            | '₺'
            | '₪'
            | '₫'
            | '฿'
            | '₴'
            | '₦'
            | '₱'
            | '₡'
            | '₲'
            | '₵'
            | '₸'
            | '₼'
            | '₾'
            | 'ƒ'
    )
}

fn is_currency_text(s: &str) -> bool {
    const CODES: [&str; 26] = [
        "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "SEK", "NOK", "DKK", "PLN",
        "CZK", "HUF", "BRL", "INR", "RUB", "ZAR", "MXN", "kr", "Kč", "zł", "Ft", "lei", "R$",
    ];
    let t = s.trim();
    t.chars().any(is_currency_char) || CODES.contains(&t)
}

fn section_flags(sec: &[Token]) -> Flags {
    let mut f = Flags::default();
    let plain = |s: &str| {
        s.chars()
            .all(|c| matches!(c, '-' | '+' | '(' | ')' | ' ' | '\u{a0}' | '\u{202f}'))
    };
    for t in sec {
        match &t.lex {
            Lex::Fill => f.fill = true,
            Lex::Skip => f.skip = true,
            Lex::Percent => f.percent = true,
            Lex::Currency(s) => f.currency |= !s.is_empty(),
            Lex::Char(c) | Lex::Escaped(c) => {
                if is_currency_char(*c) {
                    f.currency = true;
                } else if !plain(c.encode_utf8(&mut [0; 4])) {
                    f.extra_literal = true;
                }
            }
            Lex::Quoted(s) => {
                if is_currency_text(s) {
                    f.currency = true;
                } else if !plain(s) {
                    f.extra_literal = true;
                }
            }
            _ => {}
        }
    }
    f
}

fn build_section(sec: &[Token]) -> Section {
    let mut color = None;
    let mut cond = None;
    for t in sec {
        match &t.lex {
            Lex::Color(c) => color = color.or(Some(*c)),
            Lex::Cond(op, limit) => cond = cond.or(Some((*op, *limit))),
            _ => {}
        }
    }
    let has_content = sec.iter().any(|t| match &t.lex {
        Lex::Color(_) | Lex::Cond(..) | Lex::Ignored => false,
        Lex::Currency(s) => !s.is_empty(),
        _ => true,
    });
    let body = match raw_kind(sec) {
        RawKind::Date => Body::Date(build_date(sec)),
        RawKind::Number => Body::Number(build_number(sec)),
        // `[Red]` alone means `[Red]General`.
        RawKind::Text if !has_content && (color.is_some() || cond.is_some()) => {
            let mut nf = NumFmt::new(NumKind::General);
            nf.toks.push(NumTok::General);
            Body::Number(nf)
        }
        RawKind::Text => Body::Text(build_text(sec)),
    };
    Section {
        color,
        cond,
        body,
        flags: section_flags(sec),
    }
}

fn push_num_lit(toks: &mut Vec<NumTok>, s: &str) {
    if s.is_empty() {
        return;
    }
    match toks.last_mut() {
        Some(NumTok::Lit(prev)) => prev.push_str(s),
        _ => toks.push(NumTok::Lit(s.to_string())),
    }
}

impl NumFmt {
    fn new(kind: NumKind) -> NumFmt {
        NumFmt {
            toks: Vec::new(),
            kind,
            int_n: 0,
            dec_n: 0,
            exp_n: 0,
            numer_n: 0,
            denom_n: 0,
            denom_fixed: None,
            numer_zero: false,
            numer_space: false,
            grouping: false,
            scale: 0,
        }
    }
}

fn build_number(sec: &[Token]) -> NumFmt {
    if sec.iter().any(|t| t.lex == Lex::General) {
        let mut f = NumFmt::new(NumKind::General);
        for t in sec {
            match &t.lex {
                Lex::General => f.toks.push(NumTok::General),
                Lex::Digit(_) => {}
                other => {
                    if let Some(s) = literal_of(other) {
                        push_num_lit(&mut f.toks, &s);
                    }
                }
            }
        }
        return f;
    }
    let mut f = NumFmt::new(NumKind::Plain);
    let frac = find_fraction(sec);
    let exp_at = if frac.is_some() {
        None
    } else {
        exponent_index(sec)
    };
    let point_at = if frac.is_some() {
        None
    } else {
        sec[..exp_at.unwrap_or(sec.len())]
            .iter()
            .position(|t| t.lex == Lex::Dot)
    };
    let mut i = 0;
    while i < sec.len() {
        match &sec[i].lex {
            Lex::Digit(c) => {
                let tok = match &frac {
                    Some(fr) if i > fr.slash => {
                        if i < fr.denom_end {
                            f.denom_n += 1;
                            Some(NumTok::Denom(*c))
                        } else {
                            None
                        }
                    }
                    Some(fr) if i >= fr.numer => {
                        f.numer_n += 1;
                        f.numer_zero |= *c == '0';
                        f.numer_space |= *c == '?';
                        Some(NumTok::Numer(*c))
                    }
                    Some(_) => {
                        f.int_n += 1;
                        Some(NumTok::Int(*c))
                    }
                    None if exp_at.is_some_and(|e| i > e) => {
                        f.exp_n += 1;
                        Some(NumTok::ExpDigit(*c))
                    }
                    None if point_at.is_some_and(|p| i > p) => {
                        f.dec_n += 1;
                        Some(NumTok::Dec(*c))
                    }
                    None => {
                        f.int_n += 1;
                        Some(NumTok::Int(*c))
                    }
                };
                f.toks.extend(tok);
            }
            Lex::Comma => {
                let mut j = i;
                while j < sec.len() && sec[j].lex == Lex::Comma {
                    j += 1;
                }
                let after_digit = i > 0 && is_digit_token(&sec[i - 1]);
                let before_digit = j < sec.len() && is_digit_token(&sec[j]);
                if after_digit && before_digit {
                    f.grouping = true;
                } else if after_digit {
                    // Trailing commas scale the value by thousands.
                    f.scale -= 3 * (j - i) as i32;
                } else {
                    push_num_lit(&mut f.toks, &",".repeat(j - i));
                }
                i = j;
                continue;
            }
            Lex::Dot if Some(i) == point_at => f.toks.push(NumTok::Point),
            Lex::Exp(plus) if Some(i) == exp_at => {
                f.kind = NumKind::Scientific;
                f.toks.push(NumTok::Exp(*plus));
            }
            Lex::Slash if frac.as_ref().is_some_and(|fr| fr.slash == i) => {
                f.kind = NumKind::Fraction;
                f.toks.push(NumTok::Slash);
                if let Some(fr) = &frac
                    && let Some(d) = fr.fixed
                {
                    f.denom_fixed = Some(d.max(1));
                    f.toks.push(NumTok::DenomFixed(d.max(1)));
                    i = fr.denom_end;
                    continue;
                }
            }
            Lex::Percent => {
                f.scale += 2;
                push_num_lit(&mut f.toks, "%");
            }
            other => {
                if let Some(s) = literal_of(other) {
                    push_num_lit(&mut f.toks, &s);
                }
            }
        }
        i += 1;
    }
    f
}

fn push_date_lit(toks: &mut Vec<DateTok>, s: &str) {
    if s.is_empty() {
        return;
    }
    match toks.last_mut() {
        Some(DateTok::Lit(prev)) => prev.push_str(s),
        _ => toks.push(DateTok::Lit(s.to_string())),
    }
}

fn build_date(sec: &[Token]) -> DateFmt {
    let mut toks: Vec<DateTok> = Vec::new();
    let mut system = None;
    let mut i = 0;
    while i < sec.len() {
        match &sec[i].lex {
            Lex::Date(c, n) => toks.push(match c {
                'y' => DateTok::Year(*n),
                'm' => DateTok::Month(*n),
                'd' => DateTok::Day(*n),
                'h' => DateTok::Hour(*n),
                _ => DateTok::Second(*n),
            }),
            Lex::AmPm(style) => toks.push(DateTok::AmPm(*style)),
            Lex::Elapsed(c, n) => toks.push(DateTok::Elapsed(*c, *n)),
            Lex::System(long) => system = Some(*long),
            Lex::Dot
                if matches!(
                    toks.last(),
                    Some(DateTok::Second(_) | DateTok::Elapsed('s', _))
                ) =>
            {
                // `ss.00`: fractional seconds.
                let mut j = i + 1;
                while j < sec.len() && sec[j].lex == Lex::Digit('0') {
                    j += 1;
                }
                let digits = j - i - 1;
                if digits > 0 {
                    toks.push(DateTok::SubSec(digits.min(3) as u8));
                    i = j;
                    continue;
                }
                push_date_lit(&mut toks, ".");
            }
            other => {
                if let Some(s) = literal_of(other) {
                    push_date_lit(&mut toks, &s);
                }
            }
        }
        i += 1;
    }
    // `m` / `mm` are minutes right after hours or right before seconds.
    let is_part =
        |t: &DateTok| !matches!(t, DateTok::Lit(_) | DateTok::AmPm(_) | DateTok::SubSec(_));
    for idx in 0..toks.len() {
        let DateTok::Month(n) = toks[idx] else {
            continue;
        };
        if n > 2 {
            continue;
        }
        let after_hour = matches!(
            toks[..idx].iter().rev().find(|t| is_part(t)),
            Some(DateTok::Hour(_) | DateTok::Elapsed('h', _))
        );
        let before_second = matches!(
            toks[idx + 1..].iter().find(|t| is_part(t)),
            Some(DateTok::Second(_) | DateTok::Elapsed('s', _))
        );
        if after_hour || before_second {
            toks[idx] = DateTok::Minute(n);
        }
    }
    let mut f = DateFmt {
        toks,
        twelve_hour: false,
        subsec: 0,
        has_date: system == Some(true),
        has_time: system == Some(false),
        system,
    };
    for t in &f.toks {
        match t {
            DateTok::Year(_) | DateTok::Month(_) | DateTok::Day(_) => f.has_date = true,
            DateTok::AmPm(_) => {
                f.twelve_hour = true;
                f.has_time = true;
            }
            DateTok::SubSec(n) => {
                f.subsec = f.subsec.max(*n);
                f.has_time = true;
            }
            DateTok::Lit(_) => {}
            _ => f.has_time = true,
        }
    }
    f
}

fn build_text(sec: &[Token]) -> Vec<TextTok> {
    let mut toks: Vec<TextTok> = Vec::new();
    for t in sec {
        if t.lex == Lex::At {
            toks.push(TextTok::At);
        } else if let Some(s) = literal_of(&t.lex) {
            match toks.last_mut() {
                Some(TextTok::Lit(prev)) => prev.push_str(&s),
                _ => toks.push(TextTok::Lit(s)),
            }
        }
    }
    toks
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_text(toks: &[TextTok], text: &str) -> String {
    let mut out = String::new();
    for t in toks {
        match t {
            TextTok::Lit(s) => out.push_str(s),
            TextTok::At => out.push_str(text),
        }
    }
    out
}

impl Parsed {
    /// The section text values go through, if any.
    fn text_section(&self) -> Option<&Section> {
        if self.sections.len() >= 4 {
            return self.sections.get(3);
        }
        let last = self.sections.last()?;
        match &last.body {
            Body::Text(toks) if toks.contains(&TextTok::At) => Some(last),
            _ => None,
        }
    }

    /// Number of sections that apply to numbers.
    fn numeric_count(&self) -> usize {
        if self.sections.len() >= 4 {
            3
        } else if self.text_section().is_some() {
            self.sections.len() - 1
        } else {
            self.sections.len()
        }
    }

    /// Section used for the number `n`, and whether the section already
    /// stands for "negative" (the automatic minus sign is then dropped).
    fn pick(&self, n: f64, count: usize) -> Option<(&Section, bool)> {
        let s = &self.sections;
        let c0 = s[0].cond;
        let c1 = if count > 1 { s[1].cond } else { None };
        let ok = |c: Cond| cond_holds(c, n);
        Some(match count {
            1 => match c0 {
                None => (&s[0], false),
                Some(a) if ok(a) => (&s[0], negative_only(a)),
                Some(_) => return None,
            },
            2 => match (c0, c1) {
                (None, None) if n >= 0.0 => (&s[0], false),
                (None, None) => (&s[1], true),
                (Some(a), _) if ok(a) => (&s[0], negative_only(a)),
                (Some(a), None) => (&s[1], complement_negative_only(a)),
                (_, Some(b)) if ok(b) => (&s[1], negative_only(b)),
                (None, Some(_)) => (&s[0], false),
                (Some(_), Some(_)) => return None,
            },
            _ => match (c0, c1) {
                (Some(a), _) if ok(a) => (&s[0], negative_only(a)),
                (_, Some(b)) if ok(b) => (&s[1], negative_only(b)),
                (Some(_), Some(_)) => (&s[2], false),
                (None, _) if n > 0.0 => (&s[0], false),
                (_, None) if n < 0.0 => (&s[1], true),
                _ => (&s[2], false),
            },
        })
    }

    fn format_number(&self, n: f64, locale: &Locale) -> Option<Formatted> {
        let count = self.numeric_count();
        if count == 0 {
            // A text-only code such as `@`: the number is shown as General.
            let sec = self.text_section()?;
            let Body::Text(toks) = &sec.body else {
                return None;
            };
            return Some(Formatted {
                text: render_text(toks, &format_general(n, locale, GENERAL_CHARS)),
                color: sec.color,
            });
        }
        let (sec, absorbed) = self.pick(n, count)?;
        let shown = if absorbed { n.abs() } else { n };
        let text = match &sec.body {
            Body::Number(nf) => nf.render(shown, locale),
            Body::Date(df) => df.render(n, locale)?,
            Body::Text(toks) => {
                if toks.contains(&TextTok::At) {
                    render_text(toks, &format_general(shown, locale, GENERAL_CHARS))
                } else {
                    let mut s = String::new();
                    if shown < 0.0 {
                        s.push('-');
                    }
                    s.push_str(&render_text(toks, ""));
                    s
                }
            }
        };
        Some(Formatted {
            text,
            color: sec.color,
        })
    }
}

/// Emit placeholder `index` (of `total`) of a right-aligned digit string:
/// the first placeholder takes every leading digit, missing digits are
/// padded according to the placeholder.
fn emit_aligned(
    out: &mut String,
    digits: &str,
    total: usize,
    index: usize,
    placeholder: char,
    group_sep: Option<char>,
) {
    let bytes = digits.as_bytes();
    let len = bytes.len();
    // Position of this placeholder counted from the right.
    let pos = total - 1 - index;
    let sep = |out: &mut String, from_right: usize| {
        if let Some(c) = group_sep
            && from_right > 0
            && from_right.is_multiple_of(3)
        {
            out.push(c);
        }
    };
    if index == 0 && len > pos {
        for (i, b) in bytes[..len - pos].iter().enumerate() {
            out.push(*b as char);
            sep(out, len - 1 - i);
        }
    } else if pos < len {
        out.push(bytes[len - 1 - pos] as char);
        sep(out, pos);
    } else {
        match placeholder {
            '0' => {
                out.push('0');
                sep(out, pos);
            }
            '?' => out.push(' '),
            _ => {}
        }
    }
}

impl NumFmt {
    /// Render `n`; a negative `n` gets an automatic leading minus sign
    /// unless it rounds to zero.
    fn render(&self, n: f64, locale: &Locale) -> String {
        let neg = n < 0.0;
        let mut out = String::new();
        match self.kind {
            NumKind::General => {
                let g = format_general(n.abs(), locale, GENERAL_CHARS);
                if neg {
                    out.push('-');
                }
                for t in &self.toks {
                    match t {
                        NumTok::Lit(s) => out.push_str(s),
                        NumTok::General => out.push_str(&g),
                        _ => {}
                    }
                }
            }
            NumKind::Plain => {
                let d = Dec::new(n).shifted(self.scale);
                let (int, frac) = d.round(self.dec_n);
                let zero = int.is_empty() && frac.bytes().all(|b| b == b'0');
                if neg && !zero {
                    out.push('-');
                }
                self.emit(&mut out, &int, &frac, (false, ""), locale);
            }
            NumKind::Scientific => {
                let d = Dec::new(n).shifted(self.scale);
                let step = self.int_n.max(1) as i32;
                let mut e = if d.is_zero() {
                    0
                } else if self.int_n == 0 {
                    d.exp
                } else {
                    (d.exp - 1) - (d.exp - 1).rem_euclid(step)
                };
                let mut mant = Dec {
                    digits: d.digits.clone(),
                    exp: d.exp - e,
                };
                let (mut int, mut frac) = mant.round(self.dec_n);
                if int.len() > self.int_n {
                    // Rounding carried into a new digit (9.99 → 10.0).
                    e += step;
                    mant.exp -= step;
                    (int, frac) = mant.round(self.dec_n);
                }
                if neg && !d.is_zero() {
                    out.push('-');
                }
                let exp_digits = e.unsigned_abs().to_string();
                self.emit(&mut out, &int, &frac, (e < 0, &exp_digits), locale);
            }
            NumKind::Fraction => self.render_fraction(&mut out, n),
        }
        out
    }

    fn emit(&self, out: &mut String, int: &str, frac: &str, exp: (bool, &str), locale: &Locale) {
        let group = self.grouping.then_some(locale.thousands_sep);
        let frac = frac.as_bytes();
        let (mut int_i, mut dec_i, mut exp_i) = (0, 0, 0);
        for t in &self.toks {
            match t {
                NumTok::Lit(s) => out.push_str(s),
                NumTok::Int(ph) => {
                    emit_aligned(out, int, self.int_n, int_i, *ph, group);
                    int_i += 1;
                }
                NumTok::Point => {
                    if self.int_n == 0 {
                        out.push_str(int);
                    }
                    out.push(locale.decimal_sep);
                }
                NumTok::Dec(ph) => {
                    let digit = frac.get(dec_i).copied().unwrap_or(b'0') as char;
                    let trailing = frac[dec_i.min(frac.len())..].iter().all(|b| *b == b'0');
                    match (ph, trailing) {
                        ('0', _) | (_, false) => out.push(digit),
                        ('?', true) => out.push(' '),
                        _ => {}
                    }
                    dec_i += 1;
                }
                NumTok::Exp(always_sign) => {
                    out.push('E');
                    if exp.0 {
                        out.push('-');
                    } else if *always_sign {
                        out.push('+');
                    }
                }
                NumTok::ExpDigit(ph) => {
                    emit_aligned(out, exp.1, self.exp_n, exp_i, *ph, None);
                    exp_i += 1;
                }
                _ => {}
            }
        }
    }

    fn render_fraction(&self, out: &mut String, n: f64) {
        let v = n.abs() * 10f64.powi(self.scale);
        let mixed = self.int_n > 0;
        let (mut whole, part) = if mixed {
            (v.floor(), v - v.floor())
        } else {
            (0.0, v)
        };
        let (mut num, den) = match self.denom_fixed {
            Some(d) => ((part * d as f64).round() as u64, d as u64),
            None => {
                let max_den = 10u64.pow(self.denom_n.clamp(1, 7) as u32) - 1;
                best_fraction(part, max_den)
            }
        };
        if mixed && num >= den {
            whole += 1.0;
            num = 0;
        }
        if n < 0.0 && (whole != 0.0 || num != 0) {
            out.push('-');
        }
        // A whole number hides its fraction but keeps the width of `?`.
        let show_fraction = num != 0 || !mixed || self.numer_zero;
        let int = if whole == 0.0 && show_fraction && num != 0 {
            String::new()
        } else {
            format!("{whole:.0}")
        };
        let numer = num.to_string();
        let denom = den.to_string();
        let blank = |out: &mut String, width: usize| {
            if self.numer_space {
                out.extend(std::iter::repeat_n(' ', width));
            }
        };
        let (mut int_i, mut num_i, mut den_i) = (0, 0, 0);
        for t in &self.toks {
            match t {
                NumTok::Lit(s) => out.push_str(s),
                NumTok::Int(ph) => {
                    emit_aligned(out, &int, self.int_n, int_i, *ph, None);
                    int_i += 1;
                }
                NumTok::Numer(ph) => {
                    if show_fraction {
                        emit_aligned(out, &numer, self.numer_n, num_i, *ph, None);
                    } else if *ph == '?' {
                        out.push(' ');
                    }
                    num_i += 1;
                }
                NumTok::Slash => {
                    if show_fraction {
                        out.push('/');
                    } else {
                        blank(out, 1);
                    }
                }
                NumTok::Denom(ph) => {
                    // The denominator is left-aligned.
                    match denom.as_bytes().get(den_i) {
                        Some(b) if show_fraction => {
                            out.push(*b as char);
                            if den_i + 1 == self.denom_n {
                                out.push_str(&denom[den_i + 1..]);
                            }
                        }
                        _ if *ph == '?' => out.push(' '),
                        _ => {}
                    }
                    den_i += 1;
                }
                NumTok::DenomFixed(d) => {
                    if show_fraction {
                        out.push_str(&d.to_string());
                    } else {
                        blank(out, d.to_string().len());
                    }
                }
                _ => {}
            }
        }
    }
}

/// Last serial number with a date: 9999-12-31.
const MAX_SERIAL: i64 = 2_958_465;

impl DateFmt {
    fn render(&self, n: f64, locale: &Locale) -> Option<String> {
        if let Some(long) = self.system {
            let code = if long {
                locale.long_date_format
            } else {
                locale.time_format
            };
            return parsed(code)
                .sections
                .first()
                .and_then(|sec| match &sec.body {
                    Body::Date(df) if df.system.is_none() => df.render(n, locale),
                    _ => None,
                });
        }
        if !(n >= 0.0 && n < (MAX_SERIAL + 1) as f64) {
            return None;
        }
        // Round to the displayed precision (to the millisecond when no time
        // is shown, which only absorbs floating-point noise), carrying into
        // the next day.
        let unit = if self.has_time {
            10i64.pow(self.subsec as u32)
        } else {
            1000
        };
        let mut day = n.floor() as i64;
        let mut ticks = ((n - n.floor()) * 86_400.0 * unit as f64).round() as i64;
        if ticks >= 86_400 * unit {
            ticks -= 86_400 * unit;
            day += 1;
        }
        if day > MAX_SERIAL {
            return None;
        }
        let secs = ticks / unit;
        let sub = ticks % unit;
        let (h, mi, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
        let (y, mo, d) = serial_to_ymd(day as f64);
        let wd = weekday(day as f64) as usize % 7;
        let month = (mo.clamp(1, 12) - 1) as usize;
        let total_secs = day * 86_400 + secs;

        let mut out = String::new();
        let two = |out: &mut String, n: u8, v: i64| {
            if n >= 2 {
                out.push_str(&format!("{v:02}"));
            } else {
                out.push_str(&v.to_string());
            }
        };
        for t in &self.toks {
            match t {
                DateTok::Lit(text) => out.push_str(text),
                DateTok::Year(n) => {
                    if *n <= 2 {
                        out.push_str(&format!("{:02}", y % 100));
                    } else {
                        out.push_str(&format!("{y:04}"));
                    }
                }
                DateTok::Month(n) => match n {
                    1 | 2 => two(&mut out, *n, mo as i64),
                    3 => out.push_str(locale.months_short[month]),
                    5 => out.extend(locale.months[month].chars().take(1)),
                    _ => out.push_str(locale.months[month]),
                },
                DateTok::Day(n) => match n {
                    1 | 2 => two(&mut out, *n, d as i64),
                    3 => out.push_str(locale.days_short[wd]),
                    _ => out.push_str(locale.days[wd]),
                },
                DateTok::Hour(n) => {
                    let hour = if !self.twelve_hour {
                        h
                    } else if h % 12 == 0 {
                        12
                    } else {
                        h % 12
                    };
                    two(&mut out, *n, hour);
                }
                DateTok::Minute(n) => two(&mut out, *n, mi),
                DateTok::Second(n) => two(&mut out, *n, s),
                DateTok::SubSec(n) => {
                    let shown = sub / 10i64.pow((self.subsec - *n) as u32);
                    out.push(locale.decimal_sep);
                    out.push_str(&format!("{shown:0width$}", width = *n as usize));
                }
                DateTok::AmPm(style) => out.push_str(match (style, h < 12) {
                    (AmPm::Long, true) => "AM",
                    (AmPm::Long, false) => "PM",
                    (AmPm::ShortUpper, true) => "A",
                    (AmPm::ShortUpper, false) => "P",
                    (AmPm::ShortLower, true) => "a",
                    (AmPm::ShortLower, false) => "p",
                }),
                DateTok::Elapsed(unit, width) => {
                    let v = match unit {
                        'h' => total_secs / 3600,
                        'm' => total_secs / 60,
                        _ => total_secs,
                    };
                    out.push_str(&format!("{v:0width$}", width = *width as usize));
                }
            }
        }
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Editing codes
// ---------------------------------------------------------------------------

/// Excel's limit on decimal places in a format.
const MAX_DECIMALS: usize = 30;

/// Apply `(start, end, replacement)` edits on byte ranges of `code`.
fn apply_edits(code: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    let mut out = code.to_string();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));
    for (start, end, text) in edits {
        out.replace_range(start..end, &text);
    }
    out
}

/// One more (or one fewer) decimal place in every numeric section.
fn adjust_step(code: &str, increase: bool) -> String {
    let tokens = lex(code);
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for sec in tokens.split(|t| t.lex == Lex::Sep) {
        match raw_kind(sec) {
            RawKind::Date => {}
            RawKind::Text => {
                // Zero section of an accounting format (`_-* "-" €_-`): the
                // dash is padded with one `?` per decimal place.
                let padded = sec.iter().any(|t| matches!(t.lex, Lex::Skip | Lex::Fill));
                let dash = sec
                    .iter()
                    .find(|t| matches!(&t.lex, Lex::Quoted(s) if s == "-"));
                if let (true, true, Some(dash)) = (increase, padded, dash)
                    && !sec.iter().any(|t| t.lex == Lex::At)
                {
                    edits.push((dash.end, dash.end, "?".to_string()));
                }
            }
            RawKind::Number => {
                if let Some(general) = sec.iter().find(|t| t.lex == Lex::General) {
                    let replacement = if increase { "0.0" } else { "0" };
                    edits.push((general.start, general.end, replacement.to_string()));
                    continue;
                }
                if find_fraction(sec).is_some() {
                    continue;
                }
                let exp_at = exponent_index(sec);
                let mantissa = &sec[..exp_at.unwrap_or(sec.len())];
                if let Some(dot) = mantissa.iter().position(|t| t.lex == Lex::Dot) {
                    let mut end = dot + 1;
                    while end < mantissa.len() && is_digit_token(&mantissa[end]) {
                        end += 1;
                    }
                    let count = end - dot - 1;
                    let last = &mantissa[end - 1];
                    if increase {
                        if count < MAX_DECIMALS {
                            let ch = if last.lex == Lex::Digit('?') {
                                "?"
                            } else {
                                "0"
                            };
                            edits.push((last.end, last.end, ch.to_string()));
                        }
                    } else if count <= 1 {
                        // The decimal point goes with its last digit.
                        edits.push((mantissa[dot].start, last.end, String::new()));
                    } else {
                        edits.push((last.start, last.end, String::new()));
                    }
                    continue;
                }
                let Some(last) = mantissa.iter().rev().find(|t| is_digit_token(t)) else {
                    continue;
                };
                let only_spaces = exp_at.is_none()
                    && mantissa
                        .iter()
                        .filter(|t| is_digit_token(t))
                        .all(|t| t.lex == Lex::Digit('?'));
                if only_spaces {
                    // `"-"??` in an accounting format: one `?` per decimal.
                    if increase {
                        edits.push((last.end, last.end, "?".to_string()));
                    } else {
                        edits.push((last.start, last.end, String::new()));
                    }
                } else if increase {
                    edits.push((last.end, last.end, ".0".to_string()));
                }
            }
        }
    }
    apply_edits(code, edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::date_to_serial;

    fn en(n: f64, code: &str) -> String {
        format_number(n, code, Locale::en()).text
    }

    fn fr(n: f64, code: &str) -> String {
        format_number(n, code, Locale::fr()).text
    }

    fn date(y: i64, m: i64, d: i64) -> f64 {
        date_to_serial(y, m, d).unwrap()
    }

    const RED: Option<Color> = Some(Color(0xFF0000));
    const BLUE: Option<Color> = Some(Color(0x0000FF));

    #[test]
    fn general() {
        assert_eq!(en(0.0, "General"), "0");
        assert_eq!(en(1234.5, "General"), "1234.5");
        assert_eq!(fr(1234.5, "General"), "1234,5");
        assert_eq!(fr(1234.5, "Standard"), "1234,5");
        assert_eq!(en(-0.25, ""), "-0.25");
        assert_eq!(en(1.0 / 3.0, "General"), "0.333333333");
        assert_eq!(en(123456789.123, "General"), "123456789.1");
        assert_eq!(en(12345678901.0, "General"), "12345678901");
        assert_eq!(en(99999999999.9, "General"), "1E+11");
        assert_eq!(en(123456789012.0, "General"), "1.23457E+11");
        assert_eq!(en(1234567890123456.0, "General"), "1.23457E+15");
        assert_eq!(fr(-1234567890123456.0, "General"), "-1,23457E+15");
        assert_eq!(en(0.0001, "General"), "0.0001");
        assert_eq!(en(0.00012345678912, "General"), "0.000123457");
        assert_eq!(en(0.000012345678, "General"), "1.23457E-05");
        assert_eq!(en(0.0000123, "General"), "0.0000123");
        assert_eq!(en(1e-10, "General"), "1E-10");
        assert_eq!(en(1.5e300, "General"), "1.5E+300");
        assert_eq!(en(0.1 + 0.2, "General"), "0.3");
        assert_eq!(
            format_general(123456789012345.0, Locale::en(), 15),
            "123456789012345"
        );
        assert_eq!(format_general(1e15, Locale::en(), 15), "1E+15");
        assert_eq!(en(0.000001234, "General"), "0.000001234");
        assert_eq!(en(1.5e-9, "General"), "1.5E-09");
        assert_eq!(en(1e-9, "General"), "0.000000001");
        assert_eq!(en(1234567.891234, "General"), "1234567.891");
        assert_eq!(en(-1e-20, "General"), "-1E-20");
        // Inside a section, with literals and a colour.
        assert_eq!(en(5.5, "General\" kg\""), "5.5 kg");
        assert_eq!(en(-5.5, "General\" kg\""), "-5.5 kg");
        let f = format_number(-2.0, "General;[Red]-General", Locale::en());
        assert_eq!((f.text.as_str(), f.color), ("-2", RED));
        let f = format_number(3.0, "[Blue]", Locale::en());
        assert_eq!((f.text.as_str(), f.color), ("3", BLUE));
    }

    #[test]
    fn digits_and_rounding() {
        assert_eq!(en(1234.5, "0"), "1235");
        assert_eq!(en(0.5, "0"), "1");
        assert_eq!(en(1.5, "0"), "2");
        assert_eq!(en(2.5, "0"), "3");
        assert_eq!(en(-0.5, "0"), "-1");
        assert_eq!(en(2.675, "0.00"), "2.68");
        assert_eq!(en(1.005, "0.00"), "1.01");
        assert_eq!(en(0.285, "0.00"), "0.29");
        assert_eq!(en(-2.675, "0.00"), "-2.68");
        assert_eq!(en(9.995, "0.00"), "10.00");
        assert_eq!(en(0.1 + 0.2, "0.0000000000000000"), "0.3000000000000000");
        // Negative numbers rounding to zero lose their sign.
        assert_eq!(en(-0.001, "0.00"), "0.00");
        assert_eq!(en(-0.4, "0"), "0");
        assert_eq!(en(-0.004, "#,##0.00"), "0.00");
        assert_eq!(en(5.0, "000"), "005");
        assert_eq!(en(1234.0, "00"), "1234");
        assert_eq!(en(0.5, "#.00"), ".50");
        assert_eq!(en(5.0, "#.##"), "5.");
        assert_eq!(en(5.25, "#.##"), "5.25");
        assert_eq!(en(5.2, "0.0#"), "5.2");
        assert_eq!(en(5.25, "0.0#"), "5.25");
        assert_eq!(en(5.2, "0.0?"), "5.2 ");
        assert_eq!(en(5.0, "???"), "  5");
        assert_eq!(en(12.5, ".00"), "12.50");
        assert_eq!(en(0.0, "#"), "");
        assert_eq!(en(1e20, "0"), "100000000000000000000");
        assert_eq!(en(123456789.0, "000-00-0000"), "123-45-6789");
        assert_eq!(en(5551234567.0, "(000) 000-0000"), "(555) 123-4567");
        assert_eq!(fr(1234.5, "0.00"), "1234,50");
    }

    #[test]
    fn thousands_and_scaling() {
        assert_eq!(en(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(fr(1234.5, "#,##0.00"), "1\u{202f}234,50");
        assert_eq!(en(1234567.891, "#,##0"), "1,234,568");
        assert_eq!(fr(1234567.891, "#,##0"), "1\u{202f}234\u{202f}568");
        assert_eq!(en(999.0, "#,##0"), "999");
        assert_eq!(en(0.0, "#,##0"), "0");
        assert_eq!(en(-1234.0, "#,##0"), "-1,234");
        assert_eq!(en(5.0, "0,000"), "0,005");
        assert_eq!(en(1000.0, "#,###"), "1,000");
        assert_eq!(en(0.0, "#,###"), "");
        assert_eq!(en(1234567.0, "0,"), "1235");
        assert_eq!(en(1234567.0, "0.0,,"), "1.2");
        assert_eq!(en(1234567.0, "#,##0,"), "1,235");
        assert_eq!(en(1234567890.0, "#,##0.0,,\"M\""), "1,234.6M");
        assert_eq!(fr(2500000.0, "0.0,,\" M€\""), "2,5 M€");
    }

    #[test]
    fn percent() {
        assert_eq!(en(0.1234, "0%"), "12%");
        assert_eq!(en(0.1234, "0.00%"), "12.34%");
        assert_eq!(fr(0.1234, "0.00%"), "12,34%");
        assert_eq!(fr(0.5, "0 %"), "50 %");
        assert_eq!(en(-0.075, "0.0%"), "-7.5%");
        assert_eq!(en(1.0, "0%"), "100%");
        assert_eq!(en(0.285, "0%"), "29%");
    }

    #[test]
    fn scientific() {
        assert_eq!(en(12345.678, "0.00E+00"), "1.23E+04");
        assert_eq!(fr(12345.678, "0.00E+00"), "1,23E+04");
        assert_eq!(en(0.00012345, "0.00E+00"), "1.23E-04");
        assert_eq!(en(-12345.678, "0.00E+00"), "-1.23E+04");
        assert_eq!(en(0.0, "0.00E+00"), "0.00E+00");
        assert_eq!(en(12345.678, "0.00E-00"), "1.23E04");
        assert_eq!(en(0.00012345, "0.00E-00"), "1.23E-04");
        assert_eq!(en(9.995, "0.00E+00"), "1.00E+01");
        assert_eq!(en(999999.0, "0.0E+0"), "1.0E+6");
        assert_eq!(en(12345.678, "##0.0E+0"), "12.3E+3");
        assert_eq!(en(1234567.0, "##0.0E+0"), "1.2E+6");
        assert_eq!(en(1e100, "0.00E+00"), "1.00E+100");
        assert_eq!(en(1500.0, "0E+0"), "2E+3");
    }

    #[test]
    fn fractions() {
        assert_eq!(en(1.25, "# ?/?"), "1 1/4");
        assert_eq!(en(0.5, "# ?/?"), " 1/2");
        assert_eq!(en(2.0, "# ?/?"), "2    ");
        assert_eq!(en(0.0, "# ?/?"), "0    ");
        assert_eq!(en(-1.5, "# ?/?"), "-1 1/2");
        assert_eq!(en(5.14159, "# ?/?"), "5 1/7");
        assert_eq!(en(5.14159, "# ??/??"), "5 14/99");
        assert_eq!(en(0.3333, "# ??/??"), "  1/3 ");
        assert_eq!(en(2.999999, "# ?/?"), "3    ");
        assert_eq!(en(1.375, "# ?/8"), "1 3/8");
        assert_eq!(en(1.5, "# ?/8"), "1 4/8");
        assert_eq!(en(2.0, "# ?/8"), "2    ");
        assert_eq!(en(0.25, "# ??/100"), " 25/100");
        assert_eq!(en(1.5, "?/?"), "3/2");
        assert_eq!(en(0.0, "?/?"), "0/1");
        assert_eq!(en(2.0, "# 0/0"), "2 0/1");
        assert_eq!(fr(1.25, "# ?/?"), "1 1/4");
    }

    #[test]
    fn sections_and_conditions() {
        assert_eq!(en(5.0, "0.00;(0.00)"), "5.00");
        assert_eq!(en(-5.0, "0.00;(0.00)"), "(5.00)");
        assert_eq!(en(0.0, "0.00;(0.00)"), "0.00");
        assert_eq!(en(5.0, "0;-0;\"zero\""), "5");
        assert_eq!(en(-5.0, "0;-0;\"zero\""), "-5");
        assert_eq!(en(0.0, "0;-0;\"zero\""), "zero");
        assert_eq!(en(-5.0, "0;0"), "5");
        assert_eq!(en(-5.0, "0;;"), "");
        assert_eq!(en(0.0, "0;;"), "");
        assert_eq!(en(7.0, ";;;"), "");
        // Colours.
        let f = format_number(5.0, "[Red]-0.00;0.00", Locale::en());
        assert_eq!((f.text.as_str(), f.color), ("-5.00", RED));
        let f = format_number(-5.0, "[Red]-0.00;0.00", Locale::en());
        assert_eq!((f.text.as_str(), f.color), ("5.00", None));
        let f = format_number(-1234.5, "#,##0.00;[Red]-#,##0.00", Locale::fr());
        assert_eq!((f.text.as_str(), f.color), ("-1\u{202f}234,50", RED));
        let f = format_number(1.0, "[Color 5]0", Locale::en());
        assert_eq!(f.color, BLUE);
        let f = format_number(1.0, "[color10]0", Locale::en());
        assert_eq!(f.color, Some(Color(0x008000)));
        let f = format_number(1.0, "[Rouge]0", Locale::fr());
        assert_eq!(f.color, RED);
        for (name, rgb) in [
            ("Black", 0x000000),
            ("White", 0xFFFFFF),
            ("Green", 0x00FF00),
            ("Yellow", 0xFFFF00),
            ("Magenta", 0xFF00FF),
            ("Cyan", 0x00FFFF),
        ] {
            let f = format_number(1.0, &format!("[{name}]0"), Locale::en());
            assert_eq!(f.color, Some(Color(rgb)), "{name}");
        }
        // Conditions.
        let code = "[>100]\"big\";[<=0]\"low\";0.0";
        assert_eq!(en(150.0, code), "big");
        assert_eq!(en(-3.0, code), "low");
        assert_eq!(en(50.0, code), "50.0");
        let code = "[>=1000000]0.0,,\"M\";[>=1000]0.0,\"K\";0";
        assert_eq!(en(2500000.0, code), "2.5M");
        assert_eq!(en(2500.0, code), "2.5K");
        assert_eq!(en(25.0, code), "25");
        assert_eq!(en(-25.0, code), "-25");
        assert_eq!(en(1.0, "[=1]\"one\";0"), "one");
        assert_eq!(en(2.0, "[=1]\"one\";0"), "2");
        assert_eq!(en(-2.0, "[=1]\"one\";0"), "-2");
        assert_eq!(en(-5.0, "[<0]\"neg \"0;0"), "neg 5");
        assert_eq!(en(50.0, "[>100]0"), "########");
        assert_eq!(en(-7.0, "[>100]\"big\";0.0"), "-7.0");
        assert_eq!(en(5.0, "[<>5]0;\"five\""), "five");
        let f = format_number(-3.0, "[Blue][>0]0;[Red][<0]-0;0", Locale::en());
        assert_eq!((f.text.as_str(), f.color), ("-3", RED));
    }

    #[test]
    fn literals() {
        assert_eq!(en(5.0, "0\" units\""), "5 units");
        assert_eq!(en(5.0, "\"€\"0.00"), "€5.00");
        assert_eq!(fr(1234.5, "#,##0.00 \"€\""), "1\u{202f}234,50 €");
        assert_eq!(fr(1234.5, "#,##0.00 €"), "1\u{202f}234,50 €");
        assert_eq!(en(1234.5, "$#,##0.00"), "$1,234.50");
        assert_eq!(en(-1234.5, "$#,##0.00"), "-$1,234.50");
        assert_eq!(en(5.0, "£0"), "£5");
        assert_eq!(en(5.0, "0\\x"), "5x");
        assert_eq!(en(1230.0, "00\\:00"), "12:30");
        assert_eq!(en(5.0, "0_)"), "5 ");
        assert_eq!(en(5.0, "*-0"), "5");
        assert_eq!(en(5.0, "0 !"), "5 !");
        assert_eq!(en(5.0, "+0;-0"), "+5");
        assert_eq!(en(5.0, "<0>"), "<5>");
        assert_eq!(en(-5.0, "\"abc\""), "-abc");
        // Currency / locale tags.
        assert_eq!(en(5.0, "[$€-40C]0.00"), "€5.00");
        assert_eq!(en(5.0, "[$$-409]0.00"), "$5.00");
        assert_eq!(fr(5.0, "0.00 [$€-40C]"), "5,00 €");
        assert_eq!(en(5.0, "[$-409]0.00"), "5.00");
        assert_eq!(en(5.0, "[$CHF]\\ 0"), "CHF 5");
        assert_eq!(en(5.0, "[DBNum1]0"), "5");
        assert_eq!(en(1234567.0, "#\\ ###\\ ##0"), "1 234 567");
        assert_eq!(fr(-33.0, "0 \"°C\";-0 \"°C\""), "-33 °C");
        assert_eq!(en(2.5, "d \"days\" h \"hours\""), "2 days 12 hours");
    }

    #[test]
    fn accounting() {
        let code = Locale::en().accounting_format;
        assert_eq!(en(1234.5, code), " $1,234.50 ");
        assert_eq!(en(-1234.5, code), " $(1,234.50)");
        assert_eq!(en(0.0, code), " $-   ");
        assert_eq!(
            format_value(&Value::text("x"), code, Locale::en()).text,
            " x "
        );
        let code = Locale::fr().accounting_format;
        assert_eq!(fr(1234.5, code), " 1\u{202f}234,50 € ");
        assert_eq!(fr(-1234.5, code), "-1\u{202f}234,50 € ");
        assert_eq!(fr(0.0, code), " -   € ");
        assert_eq!(
            format_value(&Value::text("x"), code, Locale::fr()).text,
            " x "
        );
        assert_eq!(fr(-12.5, Locale::fr().currency_format), "-12,50 €");
        assert_eq!(en(12.5, Locale::en().currency_format), "$12.50");
    }

    #[test]
    fn text_and_other_values() {
        let en_ = Locale::en();
        let text = |s: &str, code: &str| format_value(&Value::text(s), code, en_);
        assert_eq!(text("abc", "@").text, "abc");
        assert_eq!(text("abc", "General").text, "abc");
        assert_eq!(text("abc", "0.00").text, "abc");
        assert_eq!(text("abc", "\"<\"@\">\"").text, "<abc>");
        assert_eq!(text("abc", "@@").text, "abcabc");
        assert_eq!(text("abc", "0;-0;0;\"[\"@\"]\"").text, "[abc]");
        assert_eq!(text("abc", "0;@").text, "abc");
        assert_eq!(text("abc", ";;;").text, "");
        assert_eq!(text("abc", "0;-0;0;\"text\"").text, "text");
        let f = text("abc", "0;-0;0;[Red]@");
        assert_eq!((f.text.as_str(), f.color), ("abc", RED));
        let f = text("abc", "[Blue]@");
        assert_eq!((f.text.as_str(), f.color), ("abc", BLUE));
        // Numbers through a text code are shown as General.
        assert_eq!(en(12.5, "@"), "12.5");
        assert_eq!(en(-12.5, "@"), "-12.5");
        assert_eq!(en(12.5, "0;@"), "13");
        // Booleans and errors ignore the code.
        assert_eq!(format_value(&Value::Bool(true), "0.00", en_).text, "TRUE");
        assert_eq!(
            format_value(&Value::Bool(false), "0.00", Locale::fr()).text,
            "FAUX"
        );
        let err = Value::Error(crate::value::ErrorKind::Value);
        assert_eq!(format_value(&err, "0.00", en_).text, "#VALUE!");
        assert_eq!(
            format_value(&err, "dd/mm/yyyy", Locale::fr()).text,
            "#VALEUR!"
        );
        assert_eq!(format_value(&Value::Empty, "0.00", en_).text, "");
        let arr = Value::array(crate::value::Array::row(vec![
            Value::Number(1.5),
            Value::Number(9.0),
        ]));
        assert_eq!(format_value(&arr, "0.00", en_).text, "1.50");
    }

    #[test]
    fn dates() {
        let d = date(2025, 3, 12); // a Wednesday
        assert_eq!(en(d, "dd/mm/yyyy"), "12/03/2025");
        assert_eq!(fr(d, "dd/mm/yyyy"), "12/03/2025");
        assert_eq!(en(d, "m/d/yyyy"), "3/12/2025");
        assert_eq!(en(d, "m/d/yy"), "3/12/25");
        assert_eq!(en(d, "yyyy-mm-dd"), "2025-03-12");
        assert_eq!(en(d, "d mmmm yyyy"), "12 March 2025");
        assert_eq!(fr(d, "d mmmm yyyy"), "12 mars 2025");
        assert_eq!(en(d, "d-mmm-yy"), "12-Mar-25");
        assert_eq!(fr(d, "d-mmm-yy"), "12-mars-25");
        assert_eq!(en(d, "mmm-yy"), "Mar-25");
        assert_eq!(en(d, "mmmmm"), "M");
        assert_eq!(en(d, "ddd"), "Wed");
        assert_eq!(en(d, "dddd"), "Wednesday");
        assert_eq!(fr(d, "dddd"), "mercredi");
        assert_eq!(
            en(d, Locale::en().long_date_format),
            "Wednesday, March 12, 2025"
        );
        assert_eq!(
            fr(d, Locale::fr().long_date_format),
            "mercredi 12 mars 2025"
        );
        assert_eq!(fr(date(2024, 8, 1), "d mmm yyyy"), "1 août 2024");
        assert_eq!(
            fr(date(2024, 12, 25), "dddd d mmmm"),
            "mercredi 25 décembre"
        );
        assert_eq!(en(d, "YYYY/MM/DD"), "2025/03/12");
        assert_eq!(en(d, "yyyy\"y\""), "2025y");
        assert_eq!(en(d, "[$-409]mmmm\\ d\\,\\ yyyy"), "March 12, 2025");
        assert_eq!(
            en(d, "[$-F800]dddd, mmmm dd, yyyy"),
            "Wednesday, March 12, 2025"
        );
        assert_eq!(
            fr(d, "[$-F800]dddd, mmmm dd, yyyy"),
            "mercredi 12 mars 2025"
        );
        assert_eq!(en(d, "dd/mm/yyyy;@"), "12/03/2025");
        // The 1900 date system and its quirks.
        assert_eq!(en(1.0, "dd/mm/yyyy"), "01/01/1900");
        assert_eq!(en(60.0, "dd/mm/yyyy"), "29/02/1900");
        assert_eq!(en(61.0, "dd/mm/yyyy"), "01/03/1900");
        assert_eq!(en(0.0, "dd/mm/yyyy"), "00/01/1900");
        assert_eq!(en(2958465.0, "dd/mm/yyyy"), "31/12/9999");
        // A date-only format does not round the time up to the next day,
        // except for floating-point noise.
        assert_eq!(en(45000.999999, "dd/mm/yyyy"), "15/03/2023");
        assert_eq!(en(45001.0 - 1e-10, "dd/mm/yyyy"), "16/03/2023");
        // Out of range.
        assert_eq!(en(-1.0, "dd/mm/yyyy"), "########");
        assert_eq!(en(2958466.0, "dd/mm/yyyy"), "########");
        assert_eq!(en(-0.5, "h:mm"), "########");
        assert_eq!(en(-0.5, "[h]:mm"), "########");
        assert!(try_format_number(-1.0, "dd/mm/yyyy", Locale::en()).is_none());
    }

    #[test]
    fn times() {
        let t = 14.5 / 24.0; // 14:30
        assert_eq!(en(t, "h:mm"), "14:30");
        assert_eq!(en(t, "hh:mm:ss"), "14:30:00");
        assert_eq!(en(t, "h:mm AM/PM"), "2:30 PM");
        assert_eq!(fr(t, "h:mm AM/PM"), "2:30 PM");
        assert_eq!(en(t, "hh:mm am/pm"), "02:30 PM");
        assert_eq!(en(t, "h:mm A/P"), "2:30 P");
        assert_eq!(en(t, "h:mm a/p"), "2:30 p");
        assert_eq!(en(0.25, "h:mm:ss AM/PM"), "6:00:00 AM");
        assert_eq!(en(0.0, "h:mm AM/PM"), "12:00 AM");
        assert_eq!(en(0.5, "h AM/PM"), "12 PM");
        assert_eq!(en(0.5, "h:mm"), "12:00");
        assert_eq!(
            en(8.0 / 24.0 + 5.0 / 1440.0 + 9.0 / 86400.0, "h:m:s"),
            "8:5:9"
        );
        assert_eq!(
            en(8.0 / 24.0 + 5.0 / 1440.0 + 9.0 / 86400.0, "hh:mm:ss"),
            "08:05:09"
        );
        // Month vs minute.
        let dt = date(2025, 3, 12) + t;
        assert_eq!(en(dt, "dd/mm/yyyy hh:mm"), "12/03/2025 14:30");
        assert_eq!(en(dt, "yyyy-mm-dd hh:mm:ss"), "2025-03-12 14:30:00");
        assert_eq!(en(dt, "m/d/yy h:mm"), "3/12/25 14:30");
        assert_eq!(en(dt, "hh:mm dd/mm"), "14:30 12/03");
        assert_eq!(en(90.5 / 86400.0, "mm:ss"), "01:31");
        assert_eq!(en(90.0 / 86400.0, "m"), "1");
        // Fractional seconds.
        assert_eq!(en(90.25 / 86400.0, "mm:ss.00"), "01:30.25");
        assert_eq!(fr(90.25 / 86400.0, "mm:ss.00"), "01:30,25");
        assert_eq!(en(90.25 / 86400.0, "mm:ss.0"), "01:30.3");
        assert_eq!(en(1.5 / 86400.0, "s.000"), "1.500");
        // Elapsed time.
        assert_eq!(en(1.5, "[h]:mm"), "36:00");
        assert_eq!(en(1.5, "[hh]:mm:ss"), "36:00:00");
        assert_eq!(en(0.25, "[hh]:mm"), "06:00");
        assert_eq!(en(1.5, "[m]"), "2160");
        assert_eq!(en(1.5, "[mm]:ss"), "2160:00");
        assert_eq!(en(90.0 / 86400.0, "[s]"), "90");
        assert_eq!(en(90.0 / 86400.0, "[ss].00"), "90.00");
        assert_eq!(en(1.5, "h:mm"), "12:00");
        // Rounding carries into the next unit / day.
        assert_eq!(en(0.999994, "h:mm:ss"), "23:59:59");
        assert_eq!(en(0.9999999, "h:mm:ss"), "0:00:00");
        assert_eq!(
            en(date(2025, 3, 12) + 0.9999999, "dd/mm/yyyy hh:mm"),
            "13/03/2025 00:00"
        );
        assert_eq!(en(0.5, "[$-F400]h:mm:ss AM/PM"), "12:00:00 PM");
        assert_eq!(fr(0.5, "[$-F400]h:mm:ss AM/PM"), "12:00:00");
    }

    #[test]
    fn kinds() {
        use FormatKind::*;
        for (code, kind) in [
            ("General", General),
            ("general", General),
            ("Standard", General),
            ("", General),
            ("0", Number),
            ("0.00", Number),
            ("#,##0", Number),
            ("#,##0.00", Number),
            ("#,##0.00;[Red]-#,##0.00", Number),
            ("#,##0.00_);(#,##0.00)", Number),
            ("$#,##0.00", Currency),
            ("#,##0.00 €", Currency),
            ("#,##0.00 \"€\"", Currency),
            ("[$€-40C] #,##0.00", Currency),
            ("#,##0.00 [$CHF]", Currency),
            ("$#,##0.00_);[Red]($#,##0.00)", Currency),
            (Locale::en().accounting_format, Accounting),
            (Locale::fr().accounting_format, Accounting),
            (
                "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)",
                Accounting,
            ),
            ("dd/mm/yyyy", Date),
            ("m/d/yyyy", Date),
            ("d mmmm yyyy", Date),
            ("mmm-yy", Date),
            ("[$-F800]dddd, mmmm dd, yyyy", Date),
            ("h:mm", Time),
            ("hh:mm:ss", Time),
            ("h:mm AM/PM", Time),
            ("[h]:mm", Time),
            ("mm:ss.0", Time),
            ("dd/mm/yyyy hh:mm", DateTime),
            ("m/d/yy h:mm", DateTime),
            ("0%", Percent),
            ("0.00%", Percent),
            ("# ?/?", Fraction),
            ("# ??/??", Fraction),
            ("# ?/8", Fraction),
            ("0.00E+00", Scientific),
            ("##0.0E+0", Scientific),
            ("@", Text),
            ("0.00\" kg\"", Custom),
            ("[>100]0;0.0", Custom),
            ("0.0,,", Custom),
            ("\"abc\"", Custom),
            ("General\" kg\"", Custom),
            ("000-00-0000", Custom),
        ] {
            assert_eq!(format_kind(code), kind, "{code}");
        }
    }

    #[test]
    fn date_detection() {
        for code in [
            "dd/mm/yyyy",
            "m/d/yy",
            "yyyy",
            "h:mm",
            "[h]:mm",
            "[mm]:ss",
            "mm:ss.0",
            "h:mm AM/PM",
            "[$-409]d-mmm-yy;@",
            "[$-F800]dddd, mmmm dd, yyyy",
            "[Red]dd/mm/yyyy",
            "\"Date: \"dd/mm",
            "DD/MM/YYYY",
        ] {
            assert!(is_date_format(code), "{code}");
        }
        for code in [
            "General",
            "general",
            "Standard",
            "",
            "0.00",
            "0.00E+00",
            "##0.0E-0",
            "[Red]0.00",
            "[Red]-0.00;[Blue]0.00",
            "[Yellow]0;[Cyan]0",
            "0\" days\"",
            "0\" h\"",
            "\"dd/mm/yyyy\"",
            "0\\d",
            "@",
            "#,##0.00 [$USD]",
            "[$$-409]#,##0.00",
            "[DBNum1]0",
            "[>100]0;0",
            "# ?/?",
        ] {
            assert!(!is_date_format(code), "{code}");
        }
    }

    #[test]
    fn decimals() {
        assert_eq!(adjust_decimals("General", 1), "0.0");
        assert_eq!(adjust_decimals("General", 2), "0.00");
        assert_eq!(adjust_decimals("General", -1), "0");
        assert_eq!(adjust_decimals("General", 0), "General");
        assert_eq!(adjust_decimals("", 1), "0.0");
        assert_eq!(adjust_decimals("Standard", 1), "0.0");
        assert_eq!(adjust_decimals("0.00", -1), "0.0");
        assert_eq!(adjust_decimals("0.00", -2), "0");
        assert_eq!(adjust_decimals("0.00", -5), "0");
        assert_eq!(adjust_decimals("0", -1), "0");
        assert_eq!(adjust_decimals("0", 1), "0.0");
        assert_eq!(adjust_decimals("0", 3), "0.000");
        assert_eq!(adjust_decimals("#,##0.00 €", 1), "#,##0.000 €");
        assert_eq!(adjust_decimals("#,##0 €", 2), "#,##0.00 €");
        assert_eq!(adjust_decimals("#,##0.00 €", -2), "#,##0 €");
        assert_eq!(adjust_decimals("0%", 1), "0.0%");
        assert_eq!(adjust_decimals("0.0%", -1), "0%");
        assert_eq!(adjust_decimals("0.00E+00", 1), "0.000E+00");
        assert_eq!(adjust_decimals("0.00E+00", -2), "0E+00");
        assert_eq!(adjust_decimals("0E+00", 1), "0.0E+00");
        assert_eq!(
            adjust_decimals("#,##0.00;[Red]-#,##0.00", 1),
            "#,##0.000;[Red]-#,##0.000"
        );
        assert_eq!(adjust_decimals("#,##0_);(#,##0)", 1), "#,##0.0_);(#,##0.0)");
        assert_eq!(adjust_decimals("0.00\" kg\"", -1), "0.0\" kg\"");
        assert_eq!(adjust_decimals("\"0.00\"0.0", 1), "\"0.00\"0.00");
        assert_eq!(adjust_decimals("#,##0,", 1), "#,##0.0,");
        assert_eq!(adjust_decimals("0.0?", 1), "0.0??");
        assert_eq!(adjust_decimals("[Red]General", 1), "[Red]0.0");
        // Unchanged: dates, text, fractions.
        assert_eq!(adjust_decimals("dd/mm/yyyy", 1), "dd/mm/yyyy");
        assert_eq!(adjust_decimals("h:mm:ss", -1), "h:mm:ss");
        assert_eq!(adjust_decimals("mm:ss.0", 1), "mm:ss.0");
        assert_eq!(adjust_decimals("@", 1), "@");
        assert_eq!(adjust_decimals("# ?/?", 1), "# ?/?");
        // Accounting formats keep their zero section aligned.
        assert_eq!(
            adjust_decimals(Locale::en().accounting_format, 1),
            "_($* #,##0.000_);_($* (#,##0.000);_($* \"-\"???_);_(@_)"
        );
        assert_eq!(
            adjust_decimals(Locale::en().accounting_format, -2),
            "_($* #,##0_);_($* (#,##0);_($* \"-\"_);_(@_)"
        );
        assert_eq!(
            adjust_decimals("_($* #,##0_);_($* (#,##0);_($* \"-\"_);_(@_)", 2),
            Locale::en().accounting_format
        );
        assert_eq!(
            adjust_decimals(Locale::fr().accounting_format, -1),
            "_-* #,##0.0 €_-;-* #,##0.0 €_-;_-* \"-\"? €_-;_-@_-"
        );
        // The result is still a working code.
        assert_eq!(en(1234.5678, &adjust_decimals("#,##0.00", 1)), "1,234.568");
        assert_eq!(adjust_decimals("0.0", 100).len(), 2 + MAX_DECIMALS);
    }

    #[test]
    fn localized_codes() {
        let fr_ = Locale::fr();
        let en_ = Locale::en();
        assert_eq!(code_from_locale("0,00", en_), "0,00");
        assert_eq!(code_from_locale("0,00", fr_), "0.00");
        assert_eq!(code_from_locale("# ##0,00", fr_), "#,##0.00");
        assert_eq!(code_from_locale("#\u{a0}##0,00 €", fr_), "#,##0.00 €");
        assert_eq!(code_from_locale("# ##0", fr_), "#,##0");
        assert_eq!(code_from_locale("0,0%", fr_), "0.0%");
        assert_eq!(
            code_from_locale("0,00;[Rouge]-0,00", fr_),
            "0.00;[Rouge]-0.00"
        );
        assert_eq!(code_from_locale("jj/mm/aaaa", fr_), "dd/mm/yyyy");
        assert_eq!(
            code_from_locale("JJ/MM/AAAA hh:mm", fr_),
            "dd/MM/yyyy hh:mm"
        );
        assert_eq!(
            code_from_locale("jjjj j mmmm aaaa", fr_),
            "dddd d mmmm yyyy"
        );
        assert_eq!(code_from_locale("mm:ss,00", fr_), "mm:ss.00");
        assert_eq!(code_from_locale("h:mm AM/PM", fr_), "h:mm AM/PM");
        // Canonical codes pass through.
        for code in [
            "#,##0.00",
            "#,##0",
            "0.00",
            "0%",
            "#,##0.00 €",
            "dd/mm/yyyy",
            "dddd, d mmmm yyyy",
            "0,",
            "\"a, b\"0",
            "@",
            "General",
        ] {
            assert_eq!(code_from_locale(code, fr_), code, "{code}");
        }
        assert_eq!(
            fr(1234.5, &code_from_locale("# ##0,00 €", fr_)),
            "1\u{202f}234,50 €"
        );
    }

    /// Arbitrary codes must never panic, whatever the value.
    #[test]
    fn garbage_codes_do_not_panic() {
        const ALPHABET: [&str; 44] = [
            "0",
            "#",
            "?",
            ".",
            ",",
            "%",
            "E+",
            "E-",
            "e+",
            "/",
            "@",
            ";",
            "\"",
            "\\",
            "_",
            "*",
            "[",
            "]",
            "[Red]",
            "[>1]",
            "[<=0]",
            "[$€-40C]",
            "[$-F800]",
            "[h]",
            "[mm]",
            "[ss]",
            "General",
            "y",
            "m",
            "d",
            "h",
            "s",
            "AM/PM",
            "A/P",
            " ",
            "-",
            "(",
            ")",
            "€",
            "8",
            "1",
            "x",
            "é",
            "$",
        ];
        let values = [
            0.0,
            1.0,
            -1.0,
            0.5,
            -0.004,
            1234.5678,
            -98765.4321,
            45000.75,
            1e15,
            1e-7,
            1e300,
            -1e300,
            2958465.9999999,
            f64::NAN,
            f64::INFINITY,
        ];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for _ in 0..4000 {
            let code: String = (0..1 + next(10))
                .map(|_| ALPHABET[next(ALPHABET.len())])
                .collect();
            for locale in [Locale::en(), Locale::fr()] {
                for v in values {
                    let _ = format_number(v, &code, locale);
                }
                let _ = format_value(&Value::text("abc"), &code, locale);
                let canonical = code_from_locale(&code, locale);
                let _ = format_number(1234.5, &canonical, locale);
            }
            let _ = format_kind(&code);
            let _ = is_date_format(&code);
            for delta in [-2, -1, 1, 2] {
                let adjusted = adjust_decimals(&code, delta);
                let _ = format_number(1234.5678, &adjusted, Locale::en());
            }
        }
    }

    #[test]
    fn cache_is_bounded() {
        for i in 0..(CACHE_LIMIT + 10) {
            assert_eq!(en(1.0, &format!("0\"{i}\"")), format!("1{i}"));
        }
        CACHE.with(|c| assert!(c.borrow().len() <= CACHE_LIMIT));
    }
}
