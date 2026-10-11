//! Interpreting what the user types into a cell, and producing the text
//! shown when a cell is edited.

use crate::datetime::{date_to_serial, days_in_month, serial_to_hms, serial_to_ymd, time_to_serial};
use crate::format;
use crate::locale::{DateOrder, Locale};
use crate::parser::{ParseError, parse_formula};
use crate::value::Value;
use crate::workbook::Content;
use std::sync::Arc;

pub struct ParsedInput {
    pub content: Content,
    /// Number format implied by the entry (dates, percentages, currency).
    pub num_fmt: Option<String>,
}

/// Interpret cell input.
pub fn parse_input(text: &str, locale: &Locale) -> Result<ParsedInput, ParseError> {
    let plain = |content| {
        Ok(ParsedInput {
            content,
            num_fmt: None,
        })
    };
    if text.is_empty() {
        return plain(Content::Empty);
    }
    if let Some(rest) = text.strip_prefix('=') {
        if !rest.trim().is_empty() {
            let expr = parse_formula(rest, locale)?;
            return plain(Content::Formula(Arc::new(expr)));
        }
        return plain(Content::Value(Value::text(text)));
    }
    if let Some(rest) = text.strip_prefix('\'') {
        return plain(Content::Value(Value::text(rest)));
    }
    let trimmed = text.trim();
    if let Some((n, fmt)) = parse_number_input(trimmed, locale, true) {
        return Ok(ParsedInput {
            content: Content::Value(Value::Number(n)),
            num_fmt: fmt,
        });
    }
    if let Some(b) = locale.parse_bool(trimmed) {
        return plain(Content::Value(Value::Bool(b)));
    }
    if let Some(e) = locale.parse_error(trimmed) {
        return plain(Content::Value(Value::Error(e)));
    }
    if let Some((n, fmt)) = parse_date_time(trimmed, locale) {
        return Ok(ParsedInput {
            content: Content::Value(Value::Number(n)),
            num_fmt: Some(fmt),
        });
    }
    // Neither a well-formed number nor a date: accept loosely grouped
    // digits ("1,5" is 15 in English) before giving up on a number.
    if let Some((n, fmt)) = parse_number_input(trimmed, locale, false) {
        return Ok(ParsedInput {
            content: Content::Value(Value::Number(n)),
            num_fmt: fmt,
        });
    }
    // "+A1" / "-A1*2" are formulas too.
    if (trimmed.starts_with('+') || trimmed.starts_with('-')) && trimmed.len() > 1 {
        if let Ok(expr) = parse_formula(trimmed, locale) {
            let has_ref_or_call = {
                let mut found = false;
                expr.walk(&mut |e| {
                    if matches!(e, crate::ast::Expr::Ref(_) | crate::ast::Expr::Func(..)) {
                        found = true;
                    }
                });
                found
            };
            if has_ref_or_call {
                return plain(Content::Formula(Arc::new(expr)));
            }
        }
    }
    plain(Content::Value(Value::text(text)))
}

const CURRENCY_SYMBOLS: [&str; 5] = ["€", "$", "£", "¥", "CHF"];

/// Numbers as typed by a user: plain, with thousands separators, percent,
/// currency, or in parentheses for negatives. Returns the implied format.
fn parse_number_input(s: &str, locale: &Locale, strict: bool) -> Option<(f64, Option<String>)> {
    let plain_number = |s: &str, locale: &Locale| plain_number(s, locale, strict);
    if let Some(n) = plain_number(s, locale) {
        let has_group = s.chars().any(|c| is_group_sep(c, locale)) && !s.contains(['e', 'E']);
        let has_dec = s.contains(locale.decimal_sep);
        let fmt = if s.ends_with('%') {
            Some(if has_dec { "0.00%" } else { "0%" }.to_string())
        } else if has_group {
            Some(if has_dec { "#,##0.00" } else { "#,##0" }.to_string())
        } else if s.contains(['e', 'E']) {
            Some("0.00E+00".to_string())
        } else {
            None
        };
        return Some((n, fmt));
    }
    // (123) → -123
    if let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        if let Some(n) = plain_number(inner.trim(), locale) {
            return Some((-n, None));
        }
    }
    // Currency: symbol before or after, optional sign.
    for sym in CURRENCY_SYMBOLS {
        let (neg, body) = match s.strip_prefix('-') {
            Some(rest) => (true, rest.trim()),
            None => (false, s),
        };
        let inner = body
            .strip_prefix(sym)
            .or_else(|| body.strip_suffix(sym))
            .map(|x| x.trim());
        if let Some(inner) = inner {
            if let Some(n) = plain_number(inner, locale) {
                let fmt = if sym == locale.currency_symbol {
                    locale.currency_format.to_string()
                } else if body.starts_with(sym) {
                    format!("[${sym}]#,##0.00")
                } else {
                    format!("#,##0.00 [${sym}]")
                };
                return Some((if neg { -n } else { n }, Some(fmt)));
            }
        }
    }
    None
}

fn is_group_sep(c: char, locale: &Locale) -> bool {
    c == locale.thousands_sep
        || (locale.thousands_sep.is_whitespace() && (c == ' ' || c == '\u{a0}' || c == '\u{202f}'))
}

fn plain_number(s: &str, locale: &Locale, strict: bool) -> Option<f64> {
    // Reject things `parse_number` would accept but a user would not mean
    // as a number.
    if s.is_empty() || s.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    if strict {
        locale.parse_number(s)
    } else {
        locale.parse_number_lenient(s)
    }
}

/// Text → number for implicit coercion (`"3"+1`, `VALUE`): plain numbers,
/// percentages, currency amounts, dates and times.
pub fn parse_numeric_text(s: &str, locale: &Locale) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    if let Some((n, _)) = parse_number_input(t, locale, true) {
        return Some(n);
    }
    if let Some((n, _)) = parse_date_time(t, locale) {
        return Some(n);
    }
    parse_number_input(t, locale, false).map(|(n, _)| n)
}

fn month_from_name(token: &str, locale: &Locale) -> Option<u32> {
    let t = token.trim_end_matches('.').to_lowercase();
    // "de" links the parts of Spanish dates ("12 de marzo de 2025").
    if t == "de" {
        return None;
    }
    if t.chars().count() < 3 {
        return None;
    }
    for names in [
        &locale.months,
        &locale.months_short,
        &Locale::en().months,
        &Locale::en().months_short,
    ] {
        for (i, name) in names.iter().enumerate() {
            let n = name.to_lowercase();
            if n == t || (t.chars().count() >= 3 && n.starts_with(&t)) {
                return Some(i as u32 + 1);
            }
        }
    }
    None
}

fn expand_year(y: i64, digits: usize) -> i64 {
    if digits <= 2 {
        if y < 30 { 2000 + y } else { 1900 + y }
    } else {
        y
    }
}

fn valid_date(y: i64, m: i64, d: i64) -> Option<f64> {
    if !(1900..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    // 1900-02-29 exists in the 1900 date system.
    let max = if y == 1900 && m == 2 {
        29
    } else {
        days_in_month(y, m as u32) as i64
    };
    if d > max {
        return None;
    }
    if y == 1900 && m == 2 && d == 29 {
        return Some(60.0);
    }
    date_to_serial(y, m, d)
}

fn current_year() -> i64 {
    serial_to_ymd(crate::datetime::now_serial()).0
}

/// Parse a time such as "14:30", "2:30 PM", "14:30:15". Returns the day
/// fraction and a number format.
fn parse_time(s: &str) -> Option<(f64, &'static str)> {
    let lower = s.trim().to_lowercase();
    let (body, ampm) = if let Some(b) = lower.strip_suffix("am").or_else(|| lower.strip_suffix(" a")) {
        (b.trim(), Some(false))
    } else if let Some(b) = lower.strip_suffix("pm").or_else(|| lower.strip_suffix(" p")) {
        (b.trim(), Some(true))
    } else {
        (lower.as_str(), None)
    };
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    if !parts[0].chars().all(|c| c.is_ascii_digit()) || parts[0].is_empty() || parts[0].len() > 4 {
        return None;
    }
    let mut h: f64 = parts[0].parse().ok()?;
    if parts[1].len() != 2 && !(parts[1].len() == 1 && parts.len() == 2) {
        return None;
    }
    let m: f64 = parts[1].parse::<u32>().ok()? as f64;
    let sec: f64 = match parts.get(2) {
        Some(p) => p.replace(',', ".").parse().ok()?,
        None => 0.0,
    };
    if m >= 60.0 || sec >= 60.0 {
        return None;
    }
    if let Some(pm) = ampm {
        if !(1.0..=12.0).contains(&h) {
            return None;
        }
        if pm && h < 12.0 {
            h += 12.0;
        } else if !pm && h == 12.0 {
            h = 0.0;
        }
    }
    let fmt = match (ampm.is_some(), parts.len() == 3) {
        (true, true) => "h:mm:ss AM/PM",
        (true, false) => "h:mm AM/PM",
        (false, true) => "hh:mm:ss",
        (false, false) => "hh:mm",
    };
    Some((time_to_serial(h, m, sec), fmt))
}

/// Parse a date without time. Returns the serial and a number format.
fn parse_date(s: &str, locale: &Locale) -> Option<(f64, String)> {
    let s = s.trim();
    // Split into tokens on the date separators, commas and whitespace.
    let is_sep = |c: char| c == '/' || c == '-' || c == locale.date_sep;
    let tokens: Vec<&str> = s
        .split(|c: char| is_sep(c) || c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 2 || tokens.len() > 3 {
        return None;
    }
    let numeric = tokens.iter().all(|t| t.chars().all(|c| c.is_ascii_digit()));
    let seps = s.chars().filter(|c| is_sep(*c)).count();
    if numeric {
        // Purely numeric dates need / or - separators.
        if seps != tokens.len() - 1 {
            return None;
        }
        let n: Vec<i64> = tokens.iter().map(|t| t.parse().unwrap_or(-1)).collect();
        if tokens.len() == 3 {
            let (y, m, d) = if tokens[0].len() == 4 {
                (n[0], n[1], n[2])
            } else {
                match locale.date_order {
                    DateOrder::DMY => (expand_year(n[2], tokens[2].len()), n[1], n[0]),
                    DateOrder::MDY => (expand_year(n[2], tokens[2].len()), n[0], n[1]),
                }
            };
            return valid_date(y, m, d).map(|v| (v, locale.short_date_format.to_string()));
        }
        // Two parts: day and month of the current year, else month and year.
        let (a, b) = (n[0], n[1]);
        let (d, m) = match locale.date_order {
            DateOrder::DMY => (a, b),
            DateOrder::MDY => (b, a),
        };
        if let Some(v) = valid_date(current_year(), m, d) {
            return Some((v, "d-mmm".to_string()));
        }
        // Otherwise month and year ("3/2014", and "3.14" in German).
        if let Some(v) = valid_date(expand_year(b, tokens[1].len()), a, 1) {
            return Some((v, "mmm-yy".to_string()));
        }
        return None;
    }
    // One token is a month name.
    let mut month = None;
    let mut nums: Vec<(i64, usize)> = Vec::new();
    for t in &tokens {
        if t.chars().all(|c| c.is_ascii_digit()) {
            nums.push((t.parse().ok()?, t.len()));
        } else if month.is_none() {
            month = Some(month_from_name(t, locale)?);
        } else {
            return None;
        }
    }
    let m = month? as i64;
    match nums.as_slice() {
        [(a, la), (b, lb)] => {
            // "12 mars 2025" or "March 12, 2025"
            let (d, y) = if *la <= 2 && (*lb > 2 || *b > 31 || *la <= 2) {
                (*a, expand_year(*b, *lb))
            } else {
                (*b, expand_year(*a, *la))
            };
            valid_date(y, m, d).map(|v| (v, "d-mmm-yy".to_string()))
        }
        [(a, la)] => {
            if *la == 4 || *a > 31 {
                valid_date(expand_year(*a, *la), m, 1).map(|v| (v, "mmm-yy".to_string()))
            } else {
                valid_date(current_year(), m, *a).map(|v| (v, "d-mmm".to_string()))
            }
        }
        _ => None,
    }
}

/// Parse a date, a time, or both ("12/03/2025 14:30"). Returns the serial
/// number and the number format to apply.
pub fn parse_date_time(s: &str, locale: &Locale) -> Option<(f64, String)> {
    let s = s.trim();
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    if s.contains(':') {
        if let Some((t, fmt)) = parse_time(s) {
            return Some((t, fmt.to_string()));
        }
        // date <space> time
        let lower = s.to_lowercase();
        let time_start = lower.find(':').and_then(|colon| lower[..colon].rfind(' '))?;
        let (date_part, time_part) = s.split_at(time_start);
        let (d, _) = parse_date(date_part, locale)?;
        let (t, _) = parse_time(time_part)?;
        return Some((d + t, format!("{} hh:mm", locale.short_date_format)));
    }
    parse_date(s, locale)
}

/// The text placed in the editor when a cell is edited.
pub fn edit_text(content: &Content, num_fmt: &str, locale: &Locale) -> String {
    match content {
        Content::Empty => String::new(),
        Content::Formula(e) => format!("={}", e.to_text(locale)),
        Content::Value(v) => match v {
            Value::Number(n) => {
                if format::is_date_format(num_fmt) {
                    let has_date = *n >= 1.0;
                    let frac = n - n.floor();
                    let has_time = frac.abs() > 1e-9;
                    let mut out = String::new();
                    if has_date || !has_time {
                        out = format::format_number(n.floor(), locale.short_date_format, locale).text;
                    }
                    if has_time {
                        let (h, m, s) = serial_to_hms(*n);
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(&format!("{h:02}:{m:02}:{s:02}"));
                    }
                    out
                } else if num_fmt.contains('%') {
                    format!("{}%", locale.number_to_text(n * 100.0))
                } else {
                    locale.number_to_text(*n)
                }
            }
            Value::Text(s) => s.to_string(),
            Value::Bool(b) => locale.bool_name(*b).to_string(),
            Value::Error(e) => locale.error_name(*e).to_string(),
            Value::Empty | Value::Array(_) => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(s: &str, locale: &Locale) -> Option<(f64, Option<String>)> {
        match parse_input(s, locale).unwrap() {
            ParsedInput {
                content: Content::Value(Value::Number(n)),
                num_fmt,
            } => Some((n, num_fmt)),
            _ => None,
        }
    }

    #[test]
    fn numbers() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(num("42", en), Some((42.0, None)));
        assert_eq!(num("-1.5", en), Some((-1.5, None)));
        assert_eq!(num("1,5", fr), Some((1.5, None)));
        assert_eq!(num("10%", en), Some((0.1, Some("0%".into()))));
        assert_eq!(num("1,234.50", en), Some((1234.5, Some("#,##0.00".into()))));
        assert_eq!(num("$5", en), Some((5.0, Some("$#,##0.00".into()))));
        assert_eq!(num("12,50 €", fr), Some((12.5, Some("#,##0.00 €".into()))));
        assert_eq!(num("(3)", en), Some((-3.0, None)));
        assert_eq!(num("abc", en), None);
    }

    #[test]
    fn dates() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(num("3/12/2025", en).unwrap().0, date_to_serial(2025, 3, 12).unwrap());
        assert_eq!(num("12/03/2025", fr).unwrap().0, date_to_serial(2025, 3, 12).unwrap());
        assert_eq!(num("2025-03-12", fr).unwrap().0, date_to_serial(2025, 3, 12).unwrap());
        assert_eq!(num("12 mars 2025", fr).unwrap().0, date_to_serial(2025, 3, 12).unwrap());
        assert_eq!(num("March 12, 2025", en).unwrap().0, date_to_serial(2025, 3, 12).unwrap());
        assert_eq!(num("31/02/2025", fr), None);
        assert_eq!(num("14:30", fr).unwrap().0, 14.5 / 24.0);
        assert_eq!(num("2:30 PM", en).unwrap().0, 14.5 / 24.0);
        let (v, _) = num("12/03/2025 06:00", fr).unwrap();
        assert_eq!(v, date_to_serial(2025, 3, 12).unwrap() + 0.25);
    }

    #[test]
    fn other() {
        let en = Locale::en();
        assert!(matches!(
            parse_input("=1+1", en).unwrap().content,
            Content::Formula(_)
        ));
        assert!(matches!(
            parse_input("'007", en).unwrap().content,
            Content::Value(Value::Text(_))
        ));
        assert!(matches!(
            parse_input("true", en).unwrap().content,
            Content::Value(Value::Bool(true))
        ));
        assert!(matches!(
            parse_input("+A1", en).unwrap().content,
            Content::Formula(_)
        ));
        assert!(parse_input("=1+", en).is_err());
    }
}
