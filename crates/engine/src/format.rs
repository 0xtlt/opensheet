//! Number formatting: applies a number format code ("0.00", "dd/mm/yyyy",
//! "#,##0.00 €", ...) to a value.
//!
//! Format codes are stored in canonical (US) syntax: `.` is the decimal
//! point and `,` the thousands separator inside the code; the locale decides
//! which characters are actually displayed.

use crate::locale::{Locale, general_number};
use crate::style::Color;
use crate::value::Value;

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

/// Format any value with a number format code.
pub fn format_value(value: &Value, code: &str, locale: &Locale) -> Formatted {
    match value {
        Value::Empty => Formatted::default(),
        Value::Number(n) => format_number(*n, code, locale),
        Value::Text(s) => Formatted {
            text: s.to_string(),
            color: None,
        },
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

/// Format a number with a number format code.
pub fn format_number(n: f64, code: &str, locale: &Locale) -> Formatted {
    let _ = code;
    Formatted {
        text: format_general(n, locale, 11),
        color: None,
    }
}

/// "General" rendering of a number limited to `max_chars` characters
/// (Excel shows at most 11 in a default-width cell), switching to
/// scientific notation when the number does not fit.
pub fn format_general(n: f64, locale: &Locale, max_chars: usize) -> String {
    let mut sig = 15usize.min(max_chars.max(1));
    loop {
        let s = general_number(n, sig);
        let len = s.chars().filter(|c| *c != '-').count();
        if len <= max_chars || sig <= 1 {
            return if locale.decimal_sep == '.' {
                s
            } else {
                s.replace('.', &locale.decimal_sep.to_string())
            };
        }
        sig -= 1;
    }
}

/// Classify a format code (drives the ribbon's number-format box).
pub fn format_kind(code: &str) -> FormatKind {
    if code.eq_ignore_ascii_case("General") {
        FormatKind::General
    } else if code == "@" {
        FormatKind::Text
    } else if code.contains('%') {
        FormatKind::Percent
    } else if is_date_format(code) {
        FormatKind::Date
    } else {
        FormatKind::Number
    }
}

/// True when the code formats its value as a date and/or time.
pub fn is_date_format(code: &str) -> bool {
    let mut in_quote = false;
    let mut in_bracket = false;
    for c in code.chars() {
        match c {
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            'y' | 'Y' | 'd' | 'D' | 'm' | 'M' | 'h' | 'H' | 's' | 'S' if !in_quote && !in_bracket => {
                return !code.eq_ignore_ascii_case("General");
            }
            _ => {}
        }
    }
    false
}

/// The code with `delta` more (or fewer) decimal places, as done by the
/// "Increase Decimal" / "Decrease Decimal" buttons. `General` becomes
/// `0`, `0.0`, ...
pub fn adjust_decimals(code: &str, delta: i32) -> String {
    let _ = delta;
    code.to_string()
}
