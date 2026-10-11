//! Built-in number format ids.
//!
//! Formats 0..=49 are implied by their id and normally not spelled out in
//! `styles.xml`. Id 14 (and the date part of 22) is the "system short date":
//! it follows the locale of whoever opens the file, so it maps to and from
//! the locale's short date format.

use opensheet_engine::Locale;
use std::borrow::Cow;

/// First id available for custom formats.
pub(crate) const FIRST_CUSTOM_ID: u32 = 164;

/// Ids whose code is the same everywhere.
const FIXED: &[(u32, &str)] = &[
    (0, "General"),
    (1, "0"),
    (2, "0.00"),
    (3, "#,##0"),
    (4, "#,##0.00"),
    (9, "0%"),
    (10, "0.00%"),
    (11, "0.00E+00"),
    (12, "# ?/?"),
    (13, "# ??/??"),
    (15, "d-mmm-yy"),
    (16, "d-mmm"),
    (17, "mmm-yy"),
    (18, "h:mm AM/PM"),
    (19, "h:mm:ss AM/PM"),
    (20, "h:mm"),
    (21, "h:mm:ss"),
    (37, "#,##0 ;(#,##0)"),
    (38, "#,##0 ;[Red](#,##0)"),
    (39, "#,##0.00;(#,##0.00)"),
    (40, "#,##0.00;[Red](#,##0.00)"),
    (45, "mm:ss"),
    (46, "[h]:mm:ss"),
    (47, "mmss.0"),
    (48, "##0.0E+0"),
    (49, "@"),
];

/// Ids that are only ever read: currency / accounting formats that Excel
/// localises (and spells out in the file whenever they differ from these).
const READ_ONLY: &[(u32, &str)] = &[
    (5, "\"$\"#,##0_);(\"$\"#,##0)"),
    (6, "\"$\"#,##0_);[Red](\"$\"#,##0)"),
    (7, "\"$\"#,##0.00_);(\"$\"#,##0.00)"),
    (8, "\"$\"#,##0.00_);[Red](\"$\"#,##0.00)"),
    (41, "_(* #,##0_);_(* (#,##0);_(* \"-\"_);_(@_)"),
    (
        42,
        "_(\"$\"* #,##0_);_(\"$\"* (#,##0);_(\"$\"* \"-\"_);_(@_)",
    ),
    (43, "_(* #,##0.00_);_(* (#,##0.00);_(* \"-\"??_);_(@_)"),
    (
        44,
        "_(\"$\"* #,##0.00_);_(\"$\"* (#,##0.00);_(\"$\"* \"-\"??_);_(@_)",
    ),
    // East Asian date ids, as rendered outside those locales.
    (27, "m/d/yyyy"),
    (28, "m/d/yyyy"),
    (29, "m/d/yyyy"),
    (30, "m/d/yy"),
    (31, "yyyy\"/\"m\"/\"d"),
    (32, "h:mm"),
    (33, "h:mm:ss"),
    (34, "h:mm AM/PM"),
    (35, "h:mm:ss AM/PM"),
    (36, "m/d/yyyy"),
    (50, "m/d/yyyy"),
    (51, "m/d/yyyy"),
    (52, "m/d/yyyy"),
    (53, "m/d/yyyy"),
    (54, "m/d/yyyy"),
    (55, "h:mm AM/PM"),
    (56, "h:mm:ss AM/PM"),
    (57, "m/d/yyyy"),
    (58, "m/d/yyyy"),
];

/// The format code of a built-in id, if it is one.
pub(crate) fn builtin_code(id: u32, locale: &Locale) -> Option<Cow<'static, str>> {
    match id {
        14 => Some(Cow::Borrowed(locale.short_date_format)),
        22 => Some(Cow::Owned(format!("{} h:mm", locale.short_date_format))),
        _ => FIXED
            .iter()
            .chain(READ_ONLY)
            .find(|(i, _)| *i == id)
            .map(|(_, code)| Cow::Borrowed(*code)),
    }
}

/// The built-in id to write for a format code, if there is one.
pub(crate) fn builtin_id(code: &str, locale: &Locale) -> Option<u32> {
    if code.eq_ignore_ascii_case("General") || code.is_empty() {
        return Some(0);
    }
    if code == locale.short_date_format {
        return Some(14);
    }
    FIXED.iter().find(|(_, c)| *c == code).map(|(id, _)| *id)
}

/// True when a format code shows a calendar date (as opposed to a plain
/// number or a time of day): it has a year or day token outside quotes and
/// brackets.
pub(crate) fn shows_date(code: &str) -> bool {
    if code.eq_ignore_ascii_case("General") {
        return false;
    }
    let mut in_quote = false;
    let mut in_bracket = false;
    let mut escaped = false;
    for c in code.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if !in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if !in_quote => in_bracket = false,
            'y' | 'Y' | 'd' | 'D' if !in_quote && !in_bracket => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(builtin_code(0, en).unwrap(), "General");
        assert_eq!(builtin_code(4, en).unwrap(), "#,##0.00");
        assert_eq!(builtin_code(9, en).unwrap(), "0%");
        assert_eq!(builtin_code(10, en).unwrap(), "0.00%");
        assert_eq!(builtin_code(14, en).unwrap(), "m/d/yyyy");
        assert_eq!(builtin_code(14, fr).unwrap(), "dd/mm/yyyy");
        assert_eq!(builtin_code(22, fr).unwrap(), "dd/mm/yyyy h:mm");
        assert!(builtin_code(44, en).unwrap().contains("\"$\"* #,##0.00"));
        assert_eq!(builtin_code(164, en), None);
        // Every writable id reads back as the same code.
        for (id, code) in FIXED {
            assert_eq!(builtin_id(code, en), Some(*id));
            assert_eq!(builtin_code(*id, en).unwrap(), *code);
        }
        assert_eq!(builtin_id("dd/mm/yyyy", fr), Some(14));
        assert_eq!(builtin_id("dd/mm/yyyy", en), None);
        assert_eq!(builtin_id("general", en), Some(0));
        assert_eq!(builtin_id("0.000", en), None);
    }

    #[test]
    fn date_codes() {
        assert!(shows_date("m/d/yyyy"));
        assert!(shows_date("dd/mm/yyyy hh:mm"));
        assert!(!shows_date("h:mm:ss"));
        assert!(!shows_date("[Red]0.00"));
        assert!(!shows_date("0 \"days\""));
        assert!(!shows_date("General"));
    }
}
