//! Text functions.
//!
//! Character positions and lengths count Unicode scalar values.

use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::format;
use crate::input::parse_numeric_text;
use crate::value::{Array, ErrorKind, R, Value};

/// Longest text a cell can hold.
const MAX_TEXT: usize = 32_767;

/// A text result, `#VALUE!` when longer than a cell can hold.
fn text_value(s: String) -> R<Value> {
    if s.len() > MAX_TEXT && s.chars().count() > MAX_TEXT {
        Err(ErrorKind::Value)
    } else {
        Ok(Value::from(s))
    }
}

/// A character-count argument: truncated, `#VALUE!` when negative. `absent`
/// applies when the argument is not written at all; an empty argument
/// (`LEFT(x,)`) counts as 0.
fn count_arg(ctx: &EvalCtx, args: &[Expr], index: usize, absent: usize) -> R<usize> {
    let Some(e) = args.get(index) else {
        return Ok(absent);
    };
    let n = ctx.num(e)?.trunc();
    if n < 0.0 {
        return Err(ErrorKind::Value);
    }
    Ok(n.min(u32::MAX as f64) as usize)
}

/// A 1-based character position: truncated, `#VALUE!` below 1.
fn position_arg(ctx: &EvalCtx, e: &Expr) -> R<usize> {
    let n = ctx.num(e)?.trunc();
    if n < 1.0 {
        return Err(ErrorKind::Value);
    }
    Ok(n.min(u32::MAX as f64) as usize)
}

/// A 0 / 1 switch (`match_mode`, `match_end`, `format`).
fn switch_arg(ctx: &EvalCtx, args: &[Expr], index: usize) -> R<bool> {
    let n = ctx.num_or(args, index, 0.0)?.trunc();
    if n == 0.0 {
        Ok(false)
    } else if n == 1.0 {
        Ok(true)
    } else {
        Err(ErrorKind::Value)
    }
}

fn is_given(args: &[Expr], index: usize) -> bool {
    !matches!(args.get(index), None | Some(Expr::Missing))
}

fn eq_ignore_case(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

// ---- joining ---------------------------------------------------------------

fn f_concatenate(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut out = String::new();
    for a in args {
        out.push_str(&ctx.text(a)?);
    }
    text_value(out)
}

fn f_concat(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut out = String::new();
    for v in ctx.flatten(args) {
        out.push_str(&ctx.to_text(&v)?);
    }
    text_value(out)
}

fn f_textjoin(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut delims = Vec::new();
    for v in ctx.flatten(&args[..1]) {
        delims.push(ctx.to_text(&v)?);
    }
    let ignore_empty = ctx.boolean(&args[1])?;
    let mut out = String::new();
    let mut count = 0;
    for v in ctx.flatten(&args[2..]) {
        let t = ctx.to_text(&v)?;
        if ignore_empty && t.is_empty() {
            continue;
        }
        if count > 0 && !delims.is_empty() {
            // Several delimiters are used in turn.
            out.push_str(&delims[(count - 1) % delims.len()]);
        }
        out.push_str(&t);
        count += 1;
    }
    text_value(out)
}

// ---- extracting ------------------------------------------------------------

fn f_left(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let n = count_arg(ctx, args, 1, 1)?;
    Ok(Value::from(s.chars().take(n).collect::<String>()))
}

fn f_right(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let n = count_arg(ctx, args, 1, 1)?;
    let len = s.chars().count();
    Ok(Value::from(
        s.chars().skip(len.saturating_sub(n)).collect::<String>(),
    ))
}

fn f_mid(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let start = position_arg(ctx, &args[1])?;
    let n = count_arg(ctx, args, 2, 0)?;
    Ok(Value::from(
        s.chars().skip(start - 1).take(n).collect::<String>(),
    ))
}

fn f_len(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(ctx.text(&args[0])?.chars().count() as f64))
}

// ---- case and cleaning -----------------------------------------------------

fn f_upper(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::from(ctx.text(&args[0])?.to_uppercase()))
}

fn f_lower(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::from(ctx.text(&args[0])?.to_lowercase()))
}

/// Capitalises every letter that follows a non-letter.
fn f_proper(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let mut out = String::with_capacity(s.len());
    let mut in_word = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if in_word {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            in_word = true;
        } else {
            out.push(c);
            in_word = false;
        }
    }
    Ok(Value::from(out))
}

/// Removes leading and trailing spaces and collapses inner runs to one.
fn f_trim(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let words: Vec<&str> = s.split(' ').filter(|w| !w.is_empty()).collect();
    Ok(Value::from(words.join(" ")))
}

/// Removes the non-printing characters 0..=31.
fn f_clean(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    Ok(Value::from(
        s.chars().filter(|c| *c as u32 >= 32).collect::<String>(),
    ))
}

// ---- replacing -------------------------------------------------------------

fn f_substitute(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let text = ctx.text(&args[0])?;
    let old = ctx.text(&args[1])?;
    let new = ctx.text(&args[2])?;
    let instance = if is_given(args, 3) {
        let n = ctx.num(&args[3])?.trunc();
        if n < 1.0 {
            return Err(ErrorKind::Value);
        }
        Some(n.min(u32::MAX as f64) as usize)
    } else {
        None
    };
    if old.is_empty() {
        return Ok(Value::from(text));
    }
    let Some(instance) = instance else {
        return text_value(text.replace(&old, &new));
    };
    // With an instance number, occurrences are counted one character at a
    // time (overlapping matches count), as Excel does.
    let mut seen = 0;
    for (i, _) in text.char_indices() {
        if text[i..].starts_with(&old) {
            seen += 1;
            if seen == instance {
                let mut out = String::with_capacity(text.len() + new.len());
                out.push_str(&text[..i]);
                out.push_str(&new);
                out.push_str(&text[i + old.len()..]);
                return text_value(out);
            }
        }
    }
    Ok(Value::from(text))
}

fn f_replace(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let old: Vec<char> = ctx.text(&args[0])?.chars().collect();
    let start = position_arg(ctx, &args[1])?;
    let n = count_arg(ctx, args, 2, 0)?;
    let new = ctx.text(&args[3])?;
    let from = (start - 1).min(old.len());
    let to = from.saturating_add(n).min(old.len());
    let mut out: String = old[..from].iter().collect();
    out.push_str(&new);
    out.extend(&old[to..]);
    text_value(out)
}

// ---- searching -------------------------------------------------------------

/// One character of a `SEARCH` pattern.
#[derive(Clone, Copy)]
enum Pat {
    /// `?`
    Any,
    Lit(char),
}

/// A wildcard pattern split on `*` into runs of single-character matchers.
/// `~` escapes the next character.
fn compile_pattern(pattern: &str) -> Vec<Vec<Pat>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => segments.push(std::mem::take(&mut current)),
            '?' => current.push(Pat::Any),
            '~' => current.push(Pat::Lit(chars.next().unwrap_or('~'))),
            c => current.push(Pat::Lit(c)),
        }
    }
    segments.push(current);
    segments
}

fn segment_matches(segment: &[Pat], text: &[char], at: usize) -> bool {
    at + segment.len() <= text.len()
        && segment.iter().zip(&text[at..]).all(|(p, c)| match p {
            Pat::Any => true,
            Pat::Lit(l) => eq_ignore_case(*l, *c),
        })
}

/// Does the pattern match some text starting exactly at `at`? The first
/// segment is anchored, the following ones float (leftmost match is enough
/// because the end is not anchored).
fn pattern_matches_at(segments: &[Vec<Pat>], text: &[char], at: usize) -> bool {
    let mut pos = at;
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            match (pos..=text.len()).find(|&p| segment_matches(segment, text, p)) {
                Some(p) => pos = p,
                None => return false,
            }
        } else if !segment_matches(segment, text, pos) {
            return false;
        }
        pos += segment.len();
    }
    true
}

fn find_impl(ctx: &EvalCtx, args: &[Expr], wildcards: bool) -> R<Value> {
    let needle = ctx.text(&args[0])?;
    let hay: Vec<char> = ctx.text(&args[1])?.chars().collect();
    let start = if is_given(args, 2) {
        position_arg(ctx, &args[2])?
    } else {
        1
    };
    let from = start - 1;
    if from > hay.len() {
        return Err(ErrorKind::Value);
    }
    let found = if wildcards {
        let pattern = compile_pattern(&needle);
        (from..=hay.len()).find(|&i| pattern_matches_at(&pattern, &hay, i))
    } else {
        let needle: Vec<char> = needle.chars().collect();
        (from..=hay.len()).find(|&i| hay[i..].starts_with(&needle))
    };
    match found {
        Some(i) => Ok(Value::Number(i as f64 + 1.0)),
        None => Err(ErrorKind::Value),
    }
}

/// Case-sensitive, no wildcards.
fn f_find(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    find_impl(ctx, args, false)
}

/// Case-insensitive, with `*`, `?` and `~`.
fn f_search(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    find_impl(ctx, args, true)
}

fn f_exact(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(ctx.text(&args[0])? == ctx.text(&args[1])?))
}

fn f_rept(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let n = count_arg(ctx, args, 1, 0)?;
    if s.chars().count().saturating_mul(n) > MAX_TEXT {
        return Err(ErrorKind::Value);
    }
    Ok(Value::from(s.repeat(n)))
}

// ---- numbers <-> text ------------------------------------------------------

/// `TEXT(value, format)`: the format code may be written in the workbook
/// locale's syntax ("0,00", "jj/mm/aaaa" in French) or in canonical syntax.
fn f_text(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let locale = ctx.locale();
    let value = match ctx.eval_scalar(&args[0]) {
        Value::Error(e) => return Err(e),
        Value::Empty => Value::Number(0.0),
        v => v,
    };
    let code = ctx.text(&args[1])?;
    let code = format::code_from_locale(&code, locale);
    let value = match value {
        // Text that reads as a number (or a date) is formatted as one,
        // unless the code is the text format itself.
        Value::Text(s) if format::format_kind(&code) != format::FormatKind::Text => {
            match parse_numeric_text(&s, locale) {
                Some(n) => Value::Number(n),
                None => Value::Text(s),
            }
        }
        v => v,
    };
    if code.is_empty() {
        return Ok(match value {
            Value::Number(_) => Value::from(""),
            v => Value::from(ctx.to_text(&v)?),
        });
    }
    match value {
        // A value the code cannot show (a negative date) is an error here,
        // where the grid would display ########.
        Value::Number(n) => format::try_format_number(n, &code, locale)
            .map(|f| Value::from(f.text))
            .ok_or(ErrorKind::Value),
        v => Ok(Value::from(format::format_value(&v, &code, locale).text)),
    }
}

fn f_value(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Number(n) => Ok(Value::Number(n)),
        Value::Empty => Ok(Value::Number(0.0)),
        Value::Text(s) => parse_numeric_text(&s, ctx.locale())
            .map(Value::Number)
            .ok_or(ErrorKind::Value),
        Value::Error(e) => Err(e),
        _ => Err(ErrorKind::Value),
    }
}

/// `NUMBERVALUE(text, [decimal_separator], [group_separator])`.
fn f_numbervalue(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let locale = ctx.locale();
    let text = match ctx.eval_scalar(&args[0]) {
        Value::Number(n) => return Ok(Value::Number(n)),
        Value::Empty => return Ok(Value::Number(0.0)),
        Value::Text(s) => s,
        Value::Error(e) => return Err(e),
        _ => return Err(ErrorKind::Value),
    };
    // Only the first character of a separator argument is used.
    let separator = |index: usize, default: char| -> R<char> {
        if !is_given(args, index) {
            return Ok(default);
        }
        ctx.text(&args[index])?
            .chars()
            .next()
            .ok_or(ErrorKind::Value)
    };
    let decimal = separator(1, locale.decimal_sep)?;
    // A custom decimal separator that is the locale's group separator
    // ("1,5" with "," in English) swaps the two roles.
    let default_group = if decimal == locale.thousands_sep {
        locale.decimal_sep
    } else {
        locale.thousands_sep
    };
    let group = separator(2, default_group)?;
    if decimal == group {
        return Err(ErrorKind::Value);
    }
    // Spaces are ignored wherever they are.
    let mut t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.is_empty() {
        return Ok(Value::Number(0.0));
    }
    let mut percents = 0;
    while t.ends_with('%') {
        t.pop();
        percents += 1;
    }
    let mut canonical = String::with_capacity(t.len());
    let mut seen_decimal = false;
    for c in t.chars() {
        if c == decimal {
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
            seen_decimal = true;
            canonical.push('.');
        } else if c == group {
            // Group separators are only allowed in the integer part.
            if seen_decimal {
                return Err(ErrorKind::Value);
            }
        } else if c.is_ascii_digit() || matches!(c, '+' | '-' | 'e' | 'E') {
            canonical.push(c);
        } else {
            return Err(ErrorKind::Value);
        }
    }
    if !canonical.contains(|c: char| c.is_ascii_digit()) {
        return Err(ErrorKind::Value);
    }
    let n: f64 = canonical.parse().map_err(|_| ErrorKind::Value)?;
    let n = n / 100f64.powi(percents);
    if n.is_finite() {
        Ok(Value::Number(n))
    } else {
        Err(ErrorKind::Value)
    }
}

/// Number format code with a fixed number of decimals.
fn decimals_code(grouping: bool, places: usize) -> String {
    let mut code = String::from(if grouping { "#,##0" } else { "0" });
    if places > 0 {
        code.push('.');
        code.extend(std::iter::repeat_n('0', places));
    }
    code
}

/// The `decimals` argument of `FIXED` / `DOLLAR`: the number rounded to the
/// left of the decimal point when negative, and the decimals to display.
fn display_decimals(n: f64, decimals: f64) -> R<(f64, usize)> {
    if decimals > 127.0 {
        return Err(ErrorKind::Value);
    }
    if decimals >= 0.0 {
        return Ok((n, decimals as usize));
    }
    let unit = 10f64.powf(-decimals);
    let rounded = (n / unit).round() * unit;
    Ok((if rounded.is_finite() { rounded } else { 0.0 }, 0))
}

/// `FIXED(number, [decimals = 2], [no_commas = FALSE])`.
fn f_fixed(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?;
    let decimals = ctx.num_or(args, 1, 2.0)?.trunc();
    let no_commas = ctx.bool_or(args, 2, false)?;
    let (n, places) = display_decimals(n, decimals)?;
    let code = decimals_code(!no_commas, places);
    Ok(Value::from(
        format::format_number(n, &code, ctx.locale()).text,
    ))
}

/// `DOLLAR(number, [decimals = 2])`: the locale's currency format, negative
/// amounts in parentheses when the symbol comes first ("($1,234.57)") and
/// with a minus sign otherwise ("-1 234,57 €").
fn f_dollar(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let locale = ctx.locale();
    let n = ctx.num(&args[0])?;
    let decimals = ctx.num_or(args, 1, 2.0)?.trunc();
    let (n, places) = display_decimals(n, decimals)?;
    let no_decimals = format::adjust_decimals(locale.currency_format, -30);
    let positive = format::adjust_decimals(&no_decimals, places.min(30) as i32);
    let code = if locale.currency_format.starts_with(locale.currency_symbol) {
        format!("{positive};({positive})")
    } else {
        format!("{positive};-{positive}")
    };
    Ok(Value::from(format::format_number(n, &code, locale).text))
}

/// Text is returned as is, anything else gives "".
fn f_t(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Text(s) => Ok(Value::Text(s)),
        Value::Error(e) => Err(e),
        _ => Ok(Value::from("")),
    }
}

// ---- character codes -------------------------------------------------------

/// Windows-1252 characters for codes 128..=159 (`CHAR` / `CODE`).
const CP1252: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}', '\u{178}',
];

fn f_char(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    if !(1.0..=255.0).contains(&n) {
        return Err(ErrorKind::Value);
    }
    let code = n as u32;
    let c = match code {
        128..=159 => CP1252[(code - 128) as usize],
        _ => char::from_u32(code).ok_or(ErrorKind::Value)?,
    };
    Ok(Value::from(c.to_string()))
}

fn f_code(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let c = s.chars().next().ok_or(ErrorKind::Value)?;
    let code = match CP1252.iter().position(|x| *x == c) {
        Some(i) => 128 + i as u32,
        None if (c as u32) < 256 => c as u32,
        // Not in the ANSI code page: "?".
        None => 63,
    };
    Ok(Value::Number(code as f64))
}

fn f_unichar(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    if !(1.0..=1_114_111.0).contains(&n) {
        return Err(ErrorKind::Value);
    }
    // Surrogates are not characters.
    let c = char::from_u32(n as u32).ok_or(ErrorKind::NA)?;
    Ok(Value::from(c.to_string()))
}

fn f_unicode(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = ctx.text(&args[0])?;
    let c = s.chars().next().ok_or(ErrorKind::Value)?;
    Ok(Value::Number(c as u32 as f64))
}

// ---- splitting -------------------------------------------------------------

/// A delimiter argument: one text or an array of alternatives.
fn delimiters(ctx: &EvalCtx, e: &Expr) -> R<Vec<Vec<char>>> {
    let mut out = Vec::new();
    for v in ctx.flatten(std::slice::from_ref(e)) {
        out.push(ctx.to_text(&v)?.chars().collect());
    }
    Ok(out)
}

fn delimiter_matches(delim: &[char], window: &[char], ignore_case: bool) -> bool {
    delim.len() == window.len()
        && delim
            .iter()
            .zip(window)
            .all(|(a, b)| a == b || (ignore_case && eq_ignore_case(*a, *b)))
}

/// Non-overlapping delimiter occurrences as (start, length), scanning from
/// the start of the text. The longest delimiter wins at each position.
fn occurrences(text: &[char], delims: &[Vec<char>], ignore_case: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let hit = delims
            .iter()
            .filter(|d| {
                !d.is_empty()
                    && i + d.len() <= text.len()
                    && delimiter_matches(d, &text[i..i + d.len()], ignore_case)
            })
            .map(|d| d.len())
            .max();
        match hit {
            Some(len) => {
                out.push((i, len));
                i += len;
            }
            None => i += 1,
        }
    }
    out
}

/// Same, scanning from the end of the text (last occurrence first).
fn occurrences_from_end(
    text: &[char],
    delims: &[Vec<char>],
    ignore_case: bool,
) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut end = text.len();
    while end > 0 {
        let hit = delims
            .iter()
            .filter(|d| {
                !d.is_empty()
                    && d.len() <= end
                    && delimiter_matches(d, &text[end - d.len()..end], ignore_case)
            })
            .map(|d| d.len())
            .max();
        match hit {
            Some(len) => {
                out.push((end - len, len));
                end -= len;
            }
            None => end -= 1,
        }
    }
    out
}

/// The part of `text` before / after the `instance`-th delimiter (counted
/// from the end when negative). `Ok(None)` when there is no such delimiter.
fn split_at_delimiter(
    text: &str,
    delims: &[Vec<char>],
    instance: i64,
    ignore_case: bool,
    match_end: bool,
    after: bool,
) -> R<Option<String>> {
    let chars: Vec<char> = text.chars().collect();
    let wanted = instance.unsigned_abs() as usize;
    if instance == 0 || wanted > chars.len() {
        return Err(ErrorKind::Value);
    }
    if delims.iter().any(|d| d.is_empty()) {
        // An empty delimiter matches immediately: at the very start when
        // searching forwards, at the very end when searching backwards.
        let whole = (instance > 0) == after;
        return Ok(Some(if whole {
            text.to_string()
        } else {
            String::new()
        }));
    }
    let found = if instance > 0 {
        occurrences(&chars, delims, ignore_case)
    } else {
        occurrences_from_end(&chars, delims, ignore_case)
    };
    let (start, len) = if wanted <= found.len() {
        found[wanted - 1]
    } else if match_end && wanted == found.len() + 1 {
        // The end (or the start, searching backwards) acts as a delimiter.
        (if instance > 0 { chars.len() } else { 0 }, 0)
    } else {
        return Ok(None);
    };
    let part = if after {
        &chars[start + len..]
    } else {
        &chars[..start]
    };
    Ok(Some(part.iter().collect()))
}

/// `TEXTBEFORE` / `TEXTAFTER(text, delimiter, [instance_num = 1],
/// [match_mode = 0], [match_end = 0], [if_not_found = #N/A])`.
fn text_before_after(ctx: &EvalCtx, args: &[Expr], after: bool) -> R<Value> {
    let text = ctx.eval(&args[0]);
    let delims = delimiters(ctx, &args[1])?;
    let instance = if is_given(args, 2) {
        ctx.int(&args[2])?
    } else {
        1
    };
    let ignore_case = switch_arg(ctx, args, 3)?;
    let match_end = switch_arg(ctx, args, 4)?;
    let if_not_found = is_given(args, 5).then(|| ctx.eval_scalar(&args[5]));
    let one = |v: &Value| -> Value {
        let result = ctx
            .to_text(v)
            .and_then(|s| split_at_delimiter(&s, &delims, instance, ignore_case, match_end, after));
        match result {
            Ok(Some(s)) => Value::from(s),
            Ok(None) => if_not_found.clone().unwrap_or(Value::Error(ErrorKind::NA)),
            Err(e) => Value::Error(e),
        }
    };
    Ok(match &text {
        Value::Array(a) => Value::array(Array {
            rows: a.rows,
            cols: a.cols,
            data: a.iter().map(one).collect(),
        }),
        v => one(v),
    })
}

fn f_textbefore(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    text_before_after(ctx, args, false)
}

fn f_textafter(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    text_before_after(ctx, args, true)
}

/// `TEXTSPLIT(text, col_delimiter, [row_delimiter], [ignore_empty = FALSE],
/// [match_mode = 0], [pad_with = #N/A])`.
fn f_textsplit(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let text: Vec<char> = ctx.text(&args[0])?.chars().collect();
    let optional = |index: usize| -> R<Option<Vec<Vec<char>>>> {
        if !is_given(args, index) {
            return Ok(None);
        }
        let d = delimiters(ctx, &args[index])?;
        if d.is_empty() || d.iter().any(|x| x.is_empty()) {
            return Err(ErrorKind::Value);
        }
        Ok(Some(d))
    };
    let col_delims = optional(1)?;
    let row_delims = optional(2)?;
    if col_delims.is_none() && row_delims.is_none() {
        return Err(ErrorKind::Value);
    }
    let ignore_empty = ctx.bool_or(args, 3, false)?;
    let ignore_case = switch_arg(ctx, args, 4)?;
    let pad = if is_given(args, 5) {
        ctx.eval_scalar(&args[5])
    } else {
        Value::Error(ErrorKind::NA)
    };
    let split = |s: &[char], delims: &Option<Vec<Vec<char>>>| -> Vec<Vec<char>> {
        let Some(delims) = delims else {
            return vec![s.to_vec()];
        };
        let mut parts = Vec::new();
        let mut last = 0;
        for (start, len) in occurrences(s, delims, ignore_case) {
            parts.push(s[last..start].to_vec());
            last = start + len;
        }
        parts.push(s[last..].to_vec());
        if ignore_empty {
            parts.retain(|p| !p.is_empty());
        }
        parts
    };
    let grid: Vec<Vec<Vec<char>>> = split(&text, &row_delims)
        .iter()
        .map(|row| split(row, &col_delims))
        .collect();
    let width = grid.iter().map(|row| row.len()).max().unwrap_or(0);
    if width == 0 {
        return Err(ErrorKind::Calc);
    }
    let mut out = Array::new(grid.len(), width);
    for (r, row) in grid.iter().enumerate() {
        for c in 0..width {
            let v = match row.get(c) {
                Some(cell) => Value::from(cell.iter().collect::<String>()),
                None => pad.clone(),
            };
            out.set(r, c, v);
        }
    }
    Ok(Value::array(out))
}

// ---- values as text --------------------------------------------------------

/// Text of one value. In strict mode text is quoted so that the result can
/// be read back as a formula operand. Errors are spelled out, not raised.
fn value_text(ctx: &EvalCtx, v: &Value, strict: bool) -> String {
    let locale = ctx.locale();
    match v {
        Value::Empty => String::new(),
        Value::Number(n) => locale.number_to_text(*n),
        Value::Bool(b) => locale.bool_name(*b).to_string(),
        Value::Error(e) => locale.error_name(*e).to_string(),
        Value::Text(s) if strict => format!("\"{}\"", s.replace('"', "\"\"")),
        Value::Text(s) => s.to_string(),
        Value::Array(a) => value_text(ctx, a.get(0, 0), strict),
    }
}

/// `VALUETOTEXT(value, [format = 0])`.
fn f_valuetotext(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let v = ctx.eval_scalar(&args[0]);
    let strict = switch_arg(ctx, args, 1)?;
    Ok(Value::from(value_text(ctx, &v, strict)))
}

/// `ARRAYTOTEXT(array, [format = 0])`: a flat list ("1, 2, 3") or, in
/// strict mode, an array constant ("{1,2;3,4}").
fn f_arraytotext(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let array = ctx.eval(&args[0]).to_array();
    let strict = switch_arg(ctx, args, 1)?;
    let locale = ctx.locale();
    let mut out = String::new();
    if !strict {
        let sep = format!("{} ", locale.arg_sep);
        for (i, v) in array.iter().enumerate() {
            if i > 0 {
                out.push_str(&sep);
            }
            out.push_str(&value_text(ctx, v, false));
        }
        return text_value(out);
    }
    out.push('{');
    for r in 0..array.rows {
        if r > 0 {
            out.push(locale.array_row_sep);
        }
        for c in 0..array.cols {
            if c > 0 {
                out.push(locale.array_col_sep);
            }
            out.push_str(&value_text(ctx, array.get(r, c), true));
        }
    }
    out.push('}');
    text_value(out)
}

const fn def(name: &'static str, min: u8, max: u8, kind: Kind, f: super::FnImpl) -> FunctionDef {
    FunctionDef::new(name, min, max, kind, Category::Text, f)
}

pub const FUNCTIONS: &[FunctionDef] = &[
    def("CONCATENATE", 1, VAR, Kind::Scalar, f_concatenate),
    def("CONCAT", 1, VAR, Kind::Raw, f_concat),
    def("TEXTJOIN", 3, VAR, Kind::Raw, f_textjoin),
    def("LEFT", 1, 2, Kind::Scalar, f_left),
    def("RIGHT", 1, 2, Kind::Scalar, f_right),
    def("MID", 3, 3, Kind::Scalar, f_mid),
    def("LEN", 1, 1, Kind::Scalar, f_len),
    // The byte-counting variants behave like the character ones (no DBCS).
    def("LEFTB", 1, 2, Kind::Scalar, f_left),
    def("RIGHTB", 1, 2, Kind::Scalar, f_right),
    def("MIDB", 3, 3, Kind::Scalar, f_mid),
    def("LENB", 1, 1, Kind::Scalar, f_len),
    def("UPPER", 1, 1, Kind::Scalar, f_upper),
    def("LOWER", 1, 1, Kind::Scalar, f_lower),
    def("PROPER", 1, 1, Kind::Scalar, f_proper),
    def("TRIM", 1, 1, Kind::Scalar, f_trim),
    def("CLEAN", 1, 1, Kind::Scalar, f_clean),
    def("SUBSTITUTE", 3, 4, Kind::Scalar, f_substitute),
    def("REPLACE", 4, 4, Kind::Scalar, f_replace),
    def("REPLACEB", 4, 4, Kind::Scalar, f_replace),
    def("FIND", 2, 3, Kind::Scalar, f_find),
    def("FINDB", 2, 3, Kind::Scalar, f_find),
    def("SEARCH", 2, 3, Kind::Scalar, f_search),
    def("SEARCHB", 2, 3, Kind::Scalar, f_search),
    def("EXACT", 2, 2, Kind::Scalar, f_exact),
    def("REPT", 2, 2, Kind::Scalar, f_rept),
    def("TEXT", 2, 2, Kind::Scalar, f_text),
    def("VALUE", 1, 1, Kind::Scalar, f_value),
    def("NUMBERVALUE", 1, 3, Kind::Scalar, f_numbervalue),
    def("FIXED", 1, 3, Kind::Scalar, f_fixed),
    def("DOLLAR", 1, 2, Kind::Scalar, f_dollar),
    def("T", 1, 1, Kind::Scalar, f_t),
    def("CHAR", 1, 1, Kind::Scalar, f_char),
    def("CODE", 1, 1, Kind::Scalar, f_code),
    def("UNICHAR", 1, 1, Kind::Scalar, f_unichar),
    def("UNICODE", 1, 1, Kind::Scalar, f_unicode),
    def("TEXTBEFORE", 2, 6, Kind::Raw, f_textbefore),
    def("TEXTAFTER", 2, 6, Kind::Raw, f_textafter),
    def("TEXTSPLIT", 2, 6, Kind::Raw, f_textsplit),
    def("VALUETOTEXT", 1, 2, Kind::Scalar, f_valuetotext),
    def("ARRAYTOTEXT", 1, 2, Kind::Raw, f_arraytotext),
];

#[cfg(test)]
mod tests {
    use crate::locale::Locale;
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};
    use crate::workbook::Workbook;

    const VALUE: Value = Value::Error(ErrorKind::Value);
    const NA: Value = Value::Error(ErrorKind::NA);

    fn text(formula: &str) -> String {
        match eval(formula) {
            Value::Text(s) => s.to_string(),
            other => panic!("{formula} gave {other:?}, expected text"),
        }
    }

    /// Evaluate a formula written in French syntax in a French workbook.
    fn eval_fr(formula: &str) -> Value {
        let mut w = Workbook::new(Locale::fr());
        w.set_cell_text(0, 99, 25, &format!("={formula}")).unwrap();
        w.commit("test");
        w.value_at(0, 99, 25)
    }

    fn text_fr(formula: &str) -> String {
        match eval_fr(formula) {
            Value::Text(s) => s.to_string(),
            other => panic!("{formula} gave {other:?}, expected text"),
        }
    }

    #[test]
    fn joining() {
        assert_eq!(text("CONCATENATE(\"a\",1,TRUE)"), "a1TRUE");
        assert_eq!(text("CONCATENATE(\"a\")"), "a");
        assert_eq!(text("CONCATENATE(1.5,\" \",\"x\")"), "1.5 x");
        assert_eq!(
            eval("CONCATENATE(\"a\",1/0)"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(text("CONCAT(\"a\",\"b\",3)"), "ab3");
        assert_eq!(text("CONCAT({1,2;3,4},\"x\")"), "1234x");
        assert_eq!(eval("CONCAT(\"a\",#N/A)"), NA);
        assert_eq!(text("TEXTJOIN(\", \",TRUE,\"a\",\"\",\"b\")"), "a, b");
        assert_eq!(text("TEXTJOIN(\", \",FALSE,\"a\",\"\",\"b\")"), "a, , b");
        assert_eq!(text("TEXTJOIN(\"-\",TRUE,{1,2;3,4})"), "1-2-3-4");
        assert_eq!(text("TEXTJOIN({\"-\",\"+\"},TRUE,1,2,3,4)"), "1-2+3-4");
        assert_eq!(text("TEXTJOIN(,TRUE,\"a\",\"b\")"), "ab");
        assert_eq!(
            eval("TEXTJOIN(\",\",TRUE,1,1/0)"),
            Value::Error(ErrorKind::Div0)
        );
        let w = wb(&[
            ("A1", "x"),
            ("A2", "y"),
            ("A4", "z"),
            ("B1", "=CONCAT(A1:A4)"),
            ("B2", "=TEXTJOIN(\"/\",TRUE,A1:A4)"),
            ("B3", "=TEXTJOIN(\"/\",FALSE,A1:A4)"),
        ]);
        assert_eq!(val(&w, "B1"), Value::text("xyz"));
        assert_eq!(val(&w, "B2"), Value::text("x/y/z"));
        assert_eq!(val(&w, "B3"), Value::text("x/y//z"));
        // Results longer than a cell can hold.
        assert_eq!(eval("CONCAT(REPT(\"a\",20000),REPT(\"b\",20000))"), VALUE);
        assert_eq!(
            eval("CONCATENATE(REPT(\"a\",20000),REPT(\"b\",20000))"),
            VALUE
        );
    }

    #[test]
    fn extracting() {
        assert_eq!(text("LEFT(\"hello\")"), "h");
        assert_eq!(text("LEFT(\"hello\",3)"), "hel");
        assert_eq!(text("LEFT(\"hello\",10)"), "hello");
        assert_eq!(text("LEFT(\"hello\",0)"), "");
        assert_eq!(text("LEFT(\"hello\",)"), "");
        assert_eq!(text("LEFT(\"hello\",2.9)"), "he");
        assert_eq!(eval("LEFT(\"hello\",-1)"), VALUE);
        assert_eq!(text("LEFT(12345,2)"), "12");
        assert_eq!(text("LEFT(\"héllo\",2)"), "hé");
        assert_eq!(text("RIGHT(\"hello\")"), "o");
        assert_eq!(text("RIGHT(\"hello\",3)"), "llo");
        assert_eq!(text("RIGHT(\"hello\",10)"), "hello");
        assert_eq!(eval("RIGHT(\"hello\",-1)"), VALUE);
        assert_eq!(text("MID(\"hello\",2,3)"), "ell");
        assert_eq!(text("MID(\"hello\",4,10)"), "lo");
        assert_eq!(text("MID(\"hello\",10,2)"), "");
        assert_eq!(text("MID(\"hello\",1,0)"), "");
        assert_eq!(eval("MID(\"hello\",0,2)"), VALUE);
        assert_eq!(eval("MID(\"hello\",1,-1)"), VALUE);
        assert_eq!(num("LEN(\"hello\")"), 5.0);
        assert_eq!(num("LEN(\"\")"), 0.0);
        assert_eq!(num("LEN(\"été €\")"), 5.0);
        assert_eq!(num("LEN(123.45)"), 6.0);
        assert_eq!(num("LEN(TRUE)"), 4.0);
        assert_eq!(eval("LEN(#N/A)"), NA);
        // Byte variants alias the character ones.
        assert_eq!(text("LEFTB(\"hello\",2)"), "he");
        assert_eq!(text("RIGHTB(\"hello\",2)"), "lo");
        assert_eq!(text("MIDB(\"hello\",2,2)"), "el");
        assert_eq!(num("LENB(\"hello\")"), 5.0);
        // Lifting over arrays.
        let w = wb(&[
            ("A1", "abc"),
            ("A2", "de"),
            ("C1", "=LEN(A1:A2)"),
            ("D1", "=LEFT(A1:A2,{1,2})"),
        ]);
        assert_eq!(val(&w, "C1"), Value::Number(3.0));
        assert_eq!(val(&w, "C2"), Value::Number(2.0));
        assert_eq!(val(&w, "D1"), Value::text("a"));
        assert_eq!(val(&w, "E2"), Value::text("de"));
    }

    #[test]
    fn case_and_cleaning() {
        assert_eq!(text("UPPER(\"Hello été\")"), "HELLO ÉTÉ");
        assert_eq!(text("LOWER(\"Hello ÉTÉ\")"), "hello été");
        assert_eq!(text("PROPER(\"hello WORLD\")"), "Hello World");
        assert_eq!(text("PROPER(\"2-way street\")"), "2-Way Street");
        assert_eq!(text("PROPER(\"76BudGet\")"), "76Budget");
        assert_eq!(text("PROPER(\"o'brien\")"), "O'Brien");
        assert_eq!(text("PROPER(\"élise de la tour\")"), "Élise De La Tour");
        assert_eq!(text("TRIM(\"  a   b  c \")"), "a b c");
        assert_eq!(text("TRIM(\"abc\")"), "abc");
        assert_eq!(text("TRIM(\"   \")"), "");
        assert_eq!(text("CLEAN(\"a\"&CHAR(10)&\"b\"&CHAR(9)&CHAR(31))"), "ab");
        assert_eq!(text("CLEAN(\"plain text\")"), "plain text");
        assert_eq!(eval("UPPER(1/0)"), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn replacing() {
        assert_eq!(text("SUBSTITUTE(\"a-b-c\",\"-\",\"+\")"), "a+b+c");
        assert_eq!(text("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"), "a-b+c");
        assert_eq!(text("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",3)"), "a-b-c");
        assert_eq!(text("SUBSTITUTE(\"abc\",\"B\",\"x\")"), "abc");
        assert_eq!(text("SUBSTITUTE(\"abc\",\"\",\"x\")"), "abc");
        assert_eq!(text("SUBSTITUTE(\"aaa\",\"a\",\"\")"), "");
        assert_eq!(text("SUBSTITUTE(\"aaaa\",\"aa\",\"b\")"), "bb");
        assert_eq!(text("SUBSTITUTE(\"aaaa\",\"aa\",\"b\",2)"), "aba");
        assert_eq!(
            text("SUBSTITUTE(\"Sales Data\",\"Sales\",\"Cost\")"),
            "Cost Data"
        );
        assert_eq!(eval("SUBSTITUTE(\"abc\",\"b\",\"x\",0)"), VALUE);
        assert_eq!(text("REPLACE(\"abcdefghijk\",6,5,\"*\")"), "abcde*k");
        assert_eq!(text("REPLACE(\"2009\",3,2,\"10\")"), "2010");
        assert_eq!(text("REPLACE(\"abc\",1,0,\"x\")"), "xabc");
        assert_eq!(text("REPLACE(\"abc\",10,2,\"x\")"), "abcx");
        assert_eq!(text("REPLACE(\"abc\",2,100,\"\")"), "a");
        assert_eq!(eval("REPLACE(\"abc\",0,1,\"x\")"), VALUE);
        assert_eq!(eval("REPLACE(\"abc\",1,-1,\"x\")"), VALUE);
        assert_eq!(text("REPLACEB(\"abc\",2,1,\"x\")"), "axc");
    }

    #[test]
    fn searching() {
        assert_eq!(num("FIND(\"M\",\"Miriam McGovern\")"), 1.0);
        assert_eq!(num("FIND(\"m\",\"Miriam McGovern\")"), 6.0);
        assert_eq!(num("FIND(\"M\",\"Miriam McGovern\",3)"), 8.0);
        assert_eq!(eval("FIND(\"z\",\"abc\")"), VALUE);
        assert_eq!(eval("FIND(\"A\",\"abc\")"), VALUE);
        assert_eq!(num("FIND(\"\",\"abc\")"), 1.0);
        assert_eq!(num("FIND(\"\",\"abc\",2)"), 2.0);
        assert_eq!(num("FIND(\"\",\"\")"), 1.0);
        assert_eq!(eval("FIND(\"a\",\"abc\",0)"), VALUE);
        assert_eq!(eval("FIND(\"a\",\"abc\",5)"), VALUE);
        assert_eq!(eval("FIND(\"a\",\"abc\",2)"), VALUE);
        assert_eq!(eval("FIND(\"*\",\"abc\")"), VALUE);
        assert_eq!(num("FIND(\"?\",\"a?c\")"), 2.0);
        assert_eq!(num("FIND(\"é\",\"été\",2)"), 3.0);
        assert_eq!(num("FINDB(\"c\",\"abc\")"), 3.0);
        assert_eq!(num("SEARCH(\"e\",\"Statements\",6)"), 7.0);
        assert_eq!(num("SEARCH(\"margin\",\"Profit Margin\")"), 8.0);
        assert_eq!(num("SEARCH(\"M\",\"miriam\")"), 1.0);
        assert_eq!(num("SEARCH(\"a?c\",\"xxabcxx\")"), 3.0);
        assert_eq!(num("SEARCH(\"b*x\",\"aabcdxyz\")"), 3.0);
        assert_eq!(num("SEARCH(\"*\",\"abc\")"), 1.0);
        assert_eq!(num("SEARCH(\"~?\",\"what? yes\")"), 5.0);
        assert_eq!(num("SEARCH(\"~*\",\"2*3\")"), 2.0);
        assert_eq!(num("SEARCH(\"c*\",\"abc\")"), 3.0);
        assert_eq!(num("SEARCH(\"\",\"abc\")"), 1.0);
        assert_eq!(eval("SEARCH(\"z\",\"abc\")"), VALUE);
        assert_eq!(eval("SEARCH(\"a?d\",\"abc\")"), VALUE);
        assert_eq!(eval("SEARCH(\"a\",\"abc\",0)"), VALUE);
        assert_eq!(num("SEARCHB(\"C\",\"abc\")"), 3.0);
        assert_eq!(eval("EXACT(\"word\",\"word\")"), Value::Bool(true));
        assert_eq!(eval("EXACT(\"Word\",\"word\")"), Value::Bool(false));
        assert_eq!(eval("EXACT(1,\"1\")"), Value::Bool(true));
        assert_eq!(eval("EXACT(\"a\",#N/A)"), NA);
        assert_eq!(text("REPT(\"ab\",3)"), "ababab");
        assert_eq!(text("REPT(\"ab\",0)"), "");
        assert_eq!(text("REPT(\"ab\",2.9)"), "abab");
        assert_eq!(eval("REPT(\"ab\",-1)"), VALUE);
        assert_eq!(eval("REPT(\"ab\",20000)"), VALUE);
        assert_eq!(num("LEN(REPT(\"a\",32767))"), 32767.0);
    }

    #[test]
    fn text_function() {
        assert_eq!(text("TEXT(1234.567,\"#,##0.00\")"), "1,234.57");
        assert_eq!(text("TEXT(0.285,\"0.0%\")"), "28.5%");
        assert_eq!(text("TEXT(1234.567,\"$#,##0.00\")"), "$1,234.57");
        assert_eq!(text("TEXT(-5,\"0.00;(0.00)\")"), "(5.00)");
        assert_eq!(text("TEXT(45000,\"yyyy-mm-dd\")"), "2023-03-15");
        assert_eq!(
            text("TEXT(45000,\"dddd, mmmm d, yyyy\")"),
            "Wednesday, March 15, 2023"
        );
        assert_eq!(text("TEXT(0.75,\"h:mm AM/PM\")"), "6:00 PM");
        assert_eq!(text("TEXT(1.5,\"[h]:mm\")"), "36:00");
        assert_eq!(text("TEXT(12345.678,\"0.00E+00\")"), "1.23E+04");
        assert_eq!(text("TEXT(1.25,\"# ?/?\")"), "1 1/4");
        assert_eq!(text("TEXT(5,\"000\")"), "005");
        assert_eq!(text("TEXT(1234.5,\"General\")"), "1234.5");
        assert_eq!(text("TEXT(1234.5,\"\")"), "");
        assert_eq!(text("TEXT(\"abc\",\"0.00\")"), "abc");
        assert_eq!(text("TEXT(\"abc\",\"\"\"<\"\"@\"\">\"\"\")"), "<abc>");
        assert_eq!(text("TEXT(\"12\",\"0.00\")"), "12.00");
        assert_eq!(text("TEXT(TRUE,\"0.00\")"), "TRUE");
        assert_eq!(text("TEXT(5,\"@\")"), "5");
        assert_eq!(text("TEXT(\"1/2\",\"@\")"), "1/2");
        assert_eq!(text("TEXT(\"007\",\"@\")"), "007");
        assert_eq!(eval("TEXT(1/0,\"0\")"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval("TEXT(-1,\"dd/mm/yyyy\")"), VALUE);
        let w = wb(&[
            ("A1", "3/15/2023"),
            ("B1", "=TEXT(A1,\"d mmm yy\")"),
            ("B2", "=TEXT(A2,\"0.0\")"),
        ]);
        assert_eq!(val(&w, "B1"), Value::text("15 Mar 23"));
        assert_eq!(val(&w, "B2"), Value::text("0.0"));
        // French workbook: locale output, canonical or localized codes.
        assert_eq!(text_fr("TEXTE(1234,567;\"#,##0.00\")"), "1\u{202f}234,57");
        assert_eq!(text_fr("TEXTE(1234,567;\"# ##0,00\")"), "1\u{202f}234,57");
        assert_eq!(text_fr("TEXTE(1234,567;\"0,0\")"), "1234,6");
        assert_eq!(text_fr("TEXTE(0,5;\"0%\")"), "50%");
        assert_eq!(text_fr("TEXTE(45000;\"jj/mm/aaaa\")"), "15/03/2023");
        assert_eq!(text_fr("TEXTE(45000;\"dd/mm/yyyy\")"), "15/03/2023");
        assert_eq!(
            text_fr("TEXTE(45000;\"jjjj j mmmm aaaa\")"),
            "mercredi 15 mars 2023"
        );
        assert_eq!(text_fr("TEXTE(VRAI;\"0\")"), "VRAI");
    }

    #[test]
    fn value_functions() {
        assert_eq!(num("VALUE(\"123.45\")"), 123.45);
        assert_eq!(num("VALUE(\" 42 \")"), 42.0);
        assert_eq!(num("VALUE(\"1,234.5\")"), 1234.5);
        assert_eq!(num("VALUE(\"$1,000\")"), 1000.0);
        assert_eq!(num("VALUE(\"50%\")"), 0.5);
        assert_eq!(num("VALUE(\"1e3\")"), 1000.0);
        assert_eq!(num("VALUE(\"(5)\")"), -5.0);
        assert_eq!(num("VALUE(\"3/15/2023\")"), 45000.0);
        assert_eq!(num("VALUE(\"12:00\")"), 0.5);
        assert_eq!(num("VALUE(7)"), 7.0);
        assert_eq!(eval("VALUE(\"abc\")"), VALUE);
        assert_eq!(eval("VALUE(\"\")"), VALUE);
        assert_eq!(eval("VALUE(TRUE)"), VALUE);
        assert_eq!(eval("VALUE(#N/A)"), NA);
        assert_eq!(eval_fr("CNUM(\"1 234,5\")"), Value::Number(1234.5));
        assert_eq!(eval_fr("CNUM(\"15/03/2023\")"), Value::Number(45000.0));
        assert_eq!(num("NUMBERVALUE(\"1,234.5\")"), 1234.5);
        assert_eq!(num("NUMBERVALUE(\"2.500,27\",\",\",\".\")"), 2500.27);
        assert_eq!(num("NUMBERVALUE(\"3.5%\")"), 0.035);
        assert_eq!(num("NUMBERVALUE(\"9%%\")"), 0.0009);
        assert_eq!(num("NUMBERVALUE(\" 3 000 \")"), 3000.0);
        assert_eq!(num("NUMBERVALUE(\"\")"), 0.0);
        assert_eq!(num("NUMBERVALUE(\"-1.5e2\")"), -150.0);
        assert_eq!(num("NUMBERVALUE(\"1,5\",\",\")"), 1.5);
        assert_eq!(num("NUMBERVALUE(42)"), 42.0);
        assert_eq!(eval("NUMBERVALUE(\"1.2.3\")"), VALUE);
        assert_eq!(eval("NUMBERVALUE(\"1.5,000\")"), VALUE);
        assert_eq!(eval("NUMBERVALUE(\"abc\")"), VALUE);
        assert_eq!(eval("NUMBERVALUE(\"1\",\".\",\".\")"), VALUE);
        assert_eq!(eval("NUMBERVALUE(\"1\",\"\")"), VALUE);
        assert_eq!(eval("NUMBERVALUE(\"-\")"), VALUE);
        assert_eq!(eval_fr("VALEURNOMBRE(\"1 234,5\")"), Value::Number(1234.5));
        assert_eq!(
            eval_fr("VALEURNOMBRE(\"1,234.5\";\".\";\",\")"),
            Value::Number(1234.5)
        );
        assert_eq!(text("T(\"abc\")"), "abc");
        assert_eq!(text("T(123)"), "");
        assert_eq!(text("T(TRUE)"), "");
        assert_eq!(eval("T(#N/A)"), NA);
    }

    #[test]
    fn fixed_and_dollar() {
        assert_eq!(text("FIXED(1234.567)"), "1,234.57");
        assert_eq!(text("FIXED(1234.567,1)"), "1,234.6");
        assert_eq!(text("FIXED(1234.567,0)"), "1,235");
        assert_eq!(text("FIXED(1234.567,-1)"), "1,230");
        assert_eq!(text("FIXED(1234.567,-2)"), "1,200");
        assert_eq!(text("FIXED(-1234.567,-1,TRUE)"), "-1230");
        assert_eq!(text("FIXED(44.332,2,TRUE)"), "44.33");
        assert_eq!(text("FIXED(1234567.891,2,TRUE)"), "1234567.89");
        assert_eq!(text("FIXED(2.675,2)"), "2.68");
        assert_eq!(text("FIXED(0.5,0)"), "1");
        assert_eq!(text("FIXED(-1234.567)"), "-1,234.57");
        assert_eq!(text("FIXED(1.5,4)"), "1.5000");
        assert_eq!(eval("FIXED(1,128)"), VALUE);
        assert_eq!(eval("FIXED(\"abc\")"), VALUE);
        assert_eq!(text_fr("CTXT(1234,567)"), "1\u{202f}234,57");
        assert_eq!(text_fr("CTXT(1234,567;1;VRAI)"), "1234,6");
        assert_eq!(text("DOLLAR(1234.567)"), "$1,234.57");
        assert_eq!(text("DOLLAR(1234.567,-2)"), "$1,200");
        assert_eq!(text("DOLLAR(-1234.567,-2)"), "($1,200)");
        assert_eq!(text("DOLLAR(-0.123,4)"), "($0.1230)");
        assert_eq!(text("DOLLAR(99.888)"), "$99.89");
        assert_eq!(text("DOLLAR(1234.567,0)"), "$1,235");
        assert_eq!(eval("DOLLAR(\"abc\")"), VALUE);
        assert_eq!(text_fr("DEVISE(1234,567)"), "1\u{202f}234,57 €");
        assert_eq!(text_fr("DEVISE(-1234,567;1)"), "-1\u{202f}234,6 €");
    }

    #[test]
    fn character_codes() {
        assert_eq!(text("CHAR(65)"), "A");
        assert_eq!(text("CHAR(97.9)"), "a");
        assert_eq!(text("CHAR(128)"), "€");
        assert_eq!(text("CHAR(153)"), "™");
        assert_eq!(text("CHAR(233)"), "é");
        assert_eq!(text("CHAR(255)"), "ÿ");
        assert_eq!(eval("CHAR(0)"), VALUE);
        assert_eq!(eval("CHAR(256)"), VALUE);
        assert_eq!(num("CODE(\"A\")"), 65.0);
        assert_eq!(num("CODE(\"Alphabet\")"), 65.0);
        assert_eq!(num("CODE(\"€\")"), 128.0);
        assert_eq!(num("CODE(\"é\")"), 233.0);
        assert_eq!(num("CODE(\"Ω\")"), 63.0);
        assert_eq!(eval("CODE(\"\")"), VALUE);
        assert_eq!(text("UNICHAR(66)"), "B");
        assert_eq!(text("UNICHAR(8364)"), "€");
        assert_eq!(text("UNICHAR(128512)"), "\u{1F600}");
        assert_eq!(eval("UNICHAR(0)"), VALUE);
        assert_eq!(eval("UNICHAR(1114112)"), VALUE);
        assert_eq!(eval("UNICHAR(55296)"), NA);
        assert_eq!(num("UNICODE(\"B\")"), 66.0);
        assert_eq!(num("UNICODE(\"€uro\")"), 8364.0);
        assert_eq!(num("UNICODE(UNICHAR(128512))"), 128512.0);
        assert_eq!(eval("UNICODE(\"\")"), VALUE);
    }

    #[test]
    fn before_and_after() {
        assert_eq!(
            text("TEXTBEFORE(\"Red riding hood's red hood\",\"hood\")"),
            "Red riding "
        );
        assert_eq!(
            text("TEXTBEFORE(\"Red riding hood's red hood\",\"hood\",2)"),
            "Red riding hood's red "
        );
        assert_eq!(
            text("TEXTBEFORE(\"Red riding hood's red hood\",\"hood\",-1)"),
            "Red riding hood's red "
        );
        assert_eq!(
            text("TEXTBEFORE(\"Red riding hood's red hood\",\"hood\",-2)"),
            "Red riding "
        );
        assert_eq!(
            text("TEXTAFTER(\"Red riding hood's red hood\",\"hood\")"),
            "'s red hood"
        );
        assert_eq!(
            text("TEXTAFTER(\"Red riding hood's red hood\",\"hood\",2)"),
            ""
        );
        assert_eq!(
            text("TEXTAFTER(\"Red riding hood's red hood\",\"hood\",-1)"),
            ""
        );
        assert_eq!(
            text("TEXTAFTER(\"Red riding hood's red hood\",\"hood\",-2)"),
            "'s red hood"
        );
        // Not found.
        assert_eq!(eval("TEXTBEFORE(\"abc\",\"x\")"), NA);
        assert_eq!(eval("TEXTAFTER(\"abc\",\"x\")"), NA);
        assert_eq!(eval("TEXTBEFORE(\"a-b\",\"-\",2)"), NA);
        assert_eq!(text("TEXTBEFORE(\"abc\",\"x\",,,,\"none\")"), "none");
        assert_eq!(text("TEXTAFTER(\"abc\",\"x\",1,0,0,\"none\")"), "none");
        assert_eq!(eval("TEXTBEFORE(\"abc\",\"b\",0)"), VALUE);
        assert_eq!(eval("TEXTBEFORE(\"abc\",\"b\",4)"), VALUE);
        assert_eq!(eval("TEXTAFTER(\"abc\",\"b\",1,2)"), VALUE);
        // Case sensitivity.
        assert_eq!(eval("TEXTBEFORE(\"aXb\",\"x\")"), NA);
        assert_eq!(text("TEXTBEFORE(\"aXb\",\"x\",1,1)"), "a");
        assert_eq!(text("TEXTAFTER(\"aXb\",\"x\",1,1)"), "b");
        // The end of the text as a delimiter.
        assert_eq!(text("TEXTBEFORE(\"abc\",\"x\",1,0,1)"), "abc");
        assert_eq!(text("TEXTAFTER(\"abc\",\"x\",1,0,1)"), "");
        assert_eq!(text("TEXTBEFORE(\"abc\",\"x\",-1,0,1)"), "");
        assert_eq!(text("TEXTAFTER(\"abc\",\"x\",-1,0,1)"), "abc");
        assert_eq!(text("TEXTBEFORE(\"a-b\",\"-\",2,0,1)"), "a-b");
        assert_eq!(eval("TEXTBEFORE(\"a-b\",\"-\",3,0,1)"), NA);
        // Empty and multiple delimiters.
        assert_eq!(text("TEXTBEFORE(\"abc\",\"\")"), "");
        assert_eq!(text("TEXTAFTER(\"abc\",\"\")"), "abc");
        assert_eq!(text("TEXTBEFORE(\"abc\",\"\",-1)"), "abc");
        assert_eq!(text("TEXTAFTER(\"abc\",\"\",-1)"), "");
        assert_eq!(text("TEXTBEFORE(\"a;b,c\",{\",\",\";\"})"), "a");
        assert_eq!(text("TEXTAFTER(\"a;b,c\",{\",\",\";\"},2)"), "c");
        assert_eq!(text("TEXTAFTER(\"key: value\",\": \")"), "value");
        assert_eq!(eval("TEXTBEFORE(#N/A,\",\")"), NA);
        // Lifting over a range of texts.
        let w = wb(&[
            ("A1", "a-b"),
            ("A2", "c-d"),
            ("C1", "=TEXTAFTER(A1:A2,\"-\")"),
        ]);
        assert_eq!(val(&w, "C1"), Value::text("b"));
        assert_eq!(val(&w, "C2"), Value::text("d"));
    }

    #[test]
    fn splitting() {
        let w = wb(&[
            ("A1", "=TEXTSPLIT(\"a,b,c\",\",\")"),
            ("A3", "=TEXTSPLIT(\"a,b;c\",\",\",\";\")"),
            ("A6", "=TEXTSPLIT(\"a;b;c\",,\";\")"),
            ("E1", "=TEXTSPLIT(\"a,,b\",\",\")"),
            ("E2", "=TEXTSPLIT(\"a,,b\",\",\",,TRUE)"),
            ("E3", "=TEXTSPLIT(\"aXbxc\",\"x\",,,1)"),
            ("E4", "=TEXTSPLIT(\"a,b;c\",\",\",\";\",,,\"-\")"),
            ("E6", "=TEXTSPLIT(\"a b-c\",{\" \",\"-\"})"),
        ]);
        assert_eq!(val(&w, "A1"), Value::text("a"));
        assert_eq!(val(&w, "B1"), Value::text("b"));
        assert_eq!(val(&w, "C1"), Value::text("c"));
        assert_eq!(val(&w, "A3"), Value::text("a"));
        assert_eq!(val(&w, "B3"), Value::text("b"));
        assert_eq!(val(&w, "A4"), Value::text("c"));
        assert_eq!(val(&w, "B4"), NA);
        assert_eq!(val(&w, "A6"), Value::text("a"));
        assert_eq!(val(&w, "A7"), Value::text("b"));
        assert_eq!(val(&w, "A8"), Value::text("c"));
        assert_eq!(val(&w, "F1"), Value::text(""));
        assert_eq!(val(&w, "G1"), Value::text("b"));
        assert_eq!(val(&w, "F2"), Value::text("b"));
        assert_eq!(val(&w, "G2"), Value::Empty);
        assert_eq!(val(&w, "E3"), Value::text("a"));
        assert_eq!(val(&w, "G3"), Value::text("c"));
        assert_eq!(val(&w, "F5"), Value::text("-"));
        assert_eq!(val(&w, "G6"), Value::text("c"));
        assert_eq!(text("TEXTSPLIT(\"abc\",\",\")"), "abc");
        assert_eq!(num("SUM(LEN(TEXTSPLIT(\"1 22 333\",\" \")))"), 6.0);
        assert_eq!(eval("TEXTSPLIT(\"abc\",\"\")"), VALUE);
        assert_eq!(eval("TEXTSPLIT(\"abc\",,)"), VALUE);
        assert_eq!(
            eval("TEXTSPLIT(\",,\",\",\",,TRUE)"),
            Value::Error(ErrorKind::Calc)
        );
        assert_eq!(eval("TEXTSPLIT(#N/A,\",\")"), NA);
    }

    #[test]
    fn values_as_text() {
        assert_eq!(text("VALUETOTEXT(123.5)"), "123.5");
        assert_eq!(text("VALUETOTEXT(\"hi\")"), "hi");
        assert_eq!(text("VALUETOTEXT(\"hi\",1)"), "\"hi\"");
        assert_eq!(
            text("VALUETOTEXT(\"say \"\"hi\"\"\",1)"),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(text("VALUETOTEXT(TRUE,1)"), "TRUE");
        assert_eq!(text("VALUETOTEXT(12,1)"), "12");
        assert_eq!(text("VALUETOTEXT(#N/A)"), "#N/A");
        assert_eq!(text("VALUETOTEXT(1/0,1)"), "#DIV/0!");
        assert_eq!(eval("VALUETOTEXT(1,2)"), VALUE);
        assert_eq!(text("ARRAYTOTEXT({1,2;3,4})"), "1, 2, 3, 4");
        assert_eq!(text("ARRAYTOTEXT({1,2;3,4},1)"), "{1,2;3,4}");
        assert_eq!(
            text("ARRAYTOTEXT({\"a\",TRUE;1.5,\"b\"},1)"),
            "{\"a\",TRUE;1.5,\"b\"}"
        );
        assert_eq!(text("ARRAYTOTEXT({\"a\",TRUE})"), "a, TRUE");
        assert_eq!(text("ARRAYTOTEXT(5)"), "5");
        assert_eq!(text("ARRAYTOTEXT(\"x\",1)"), "{\"x\"}");
        assert_eq!(eval("ARRAYTOTEXT({1,2},3)"), VALUE);
        let w = wb(&[
            ("A1", "1"),
            ("B1", "x"),
            ("A2", "=1/0"),
            ("D1", "=ARRAYTOTEXT(A1:B2)"),
            ("D2", "=ARRAYTOTEXT(A1:B2,1)"),
        ]);
        assert_eq!(val(&w, "D1"), Value::text("1, x, #DIV/0!, "));
        assert_eq!(val(&w, "D2"), Value::text("{1,\"x\";#DIV/0!,}"));
        assert_eq!(text_fr("TABLEAU.EN.TEXTE({1.2;3.4})"), "1; 2; 3; 4");
        assert_eq!(text_fr("TABLEAU.EN.TEXTE({1.2;3.4};1)"), "{1.2;3.4}");
        assert_eq!(text_fr("VALEUR.EN.TEXTE(1,5)"), "1,5");
    }

    #[test]
    fn registry() {
        for name in [
            "CONCATENATE",
            "CONCAT",
            "TEXTJOIN",
            "LEFT",
            "RIGHT",
            "MID",
            "LEN",
            "UPPER",
            "LOWER",
            "PROPER",
            "TRIM",
            "CLEAN",
            "SUBSTITUTE",
            "REPLACE",
            "FIND",
            "SEARCH",
            "REPT",
            "TEXT",
            "VALUE",
            "NUMBERVALUE",
            "EXACT",
            "CHAR",
            "CODE",
            "UNICHAR",
            "UNICODE",
            "T",
            "FIXED",
            "DOLLAR",
            "TEXTBEFORE",
            "TEXTAFTER",
            "TEXTSPLIT",
            "VALUETOTEXT",
            "ARRAYTOTEXT",
            "LENB",
            "LEFTB",
            "RIGHTB",
            "MIDB",
        ] {
            let def = crate::functions::lookup(name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(def.category, crate::functions::Category::Text, "{name}");
        }
    }
}
