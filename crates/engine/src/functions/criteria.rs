//! Criteria matching shared by SUMIF / COUNTIF / AVERAGEIF and friends,
//! plus the argument helpers the math and statistical families have in
//! common (ranges with their real size, element-wise lifting, matrices).

use super::{has_wildcards, wildcard_match};
use crate::ast::Expr;
use crate::eval::{EvalCtx, compare_text, wrap};
use crate::input::parse_numeric_text;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, Reference};
use crate::value::{Array, ErrorKind, R, Value};
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl Op {
    fn holds(self, ord: Option<Ordering>) -> bool {
        let Some(ord) = ord else {
            return false;
        };
        match self {
            Op::Eq => ord == Ordering::Equal,
            Op::Ne => ord != Ordering::Equal,
            Op::Lt => ord == Ordering::Less,
            Op::Gt => ord == Ordering::Greater,
            Op::Le => ord != Ordering::Greater,
            Op::Ge => ord != Ordering::Less,
        }
    }
}

/// One element of a compiled wildcard pattern (ASCII, lower case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glob {
    /// `*`: any run of characters.
    Any,
    /// `?`: exactly one character.
    One,
    Lit(u8),
}

/// How a text criterion is compared for `=` / `<>`.
#[derive(Clone, Debug, PartialEq)]
enum TextMatch {
    /// Case-insensitive equality.
    Exact,
    /// Wildcards; `ascii` is the compiled pattern when it is pure ASCII
    /// (used for ASCII cell text, the common case).
    Wildcard { ascii: Option<Vec<Glob>> },
}

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Number(f64),
    Text {
        pattern: String,
        how: TextMatch,
    },
    Bool(bool),
    Error(ErrorKind),
    /// Nothing after the operator (`"="`, `"<>"`): truly empty cells.
    Blank,
    /// The empty string `""`: empty cells and empty text.
    EmptyText,
}

/// A parsed criterion as accepted by `SUMIF`, `COUNTIF`, `AVERAGEIFS`, ...
///
/// * a number, boolean or error matches cells holding that value;
/// * text may start with `=`, `<>`, `<`, `>`, `<=`, `>=`; what follows is
///   read like cell input (number, date, percentage, boolean, error) and
///   otherwise compared as text, case-insensitively, with the wildcards
///   `*`, `?` and `~` for `=` / `<>`;
/// * `""` matches blank cells and empty text, `"="` only blank cells and
///   `"<>"` every non-blank cell.
#[derive(Clone)]
pub struct Criteria {
    op: Op,
    operand: Operand,
    locale: &'static Locale,
}

impl Criteria {
    pub fn new(ctx: &EvalCtx, criterion: &Value) -> Criteria {
        Criteria::parse(criterion, ctx.locale())
    }

    pub fn parse(criterion: &Value, locale: &'static Locale) -> Criteria {
        let (op, operand) = match criterion {
            Value::Array(a) => return Criteria::parse(a.get(0, 0), locale),
            // A reference to an empty cell is treated as the value 0.
            Value::Empty => (Op::Eq, Operand::Number(0.0)),
            Value::Number(n) => (Op::Eq, Operand::Number(*n)),
            Value::Bool(b) => (Op::Eq, Operand::Bool(*b)),
            Value::Error(e) => (Op::Eq, Operand::Error(*e)),
            Value::Text(s) => parse_text(s, locale),
        };
        Criteria {
            op,
            operand,
            locale,
        }
    }

    /// Does a cell value satisfy the criterion?
    pub fn matches(&self, v: &Value) -> bool {
        match &self.operand {
            Operand::Number(n) => match self.op {
                Op::Eq => self.cell_number(v) == Some(*n),
                Op::Ne => self.cell_number(v) != Some(*n),
                // Ordering comparisons only look at real numbers.
                op => match v {
                    Value::Number(x) => op.holds(x.partial_cmp(n)),
                    _ => false,
                },
            },
            Operand::Text { pattern, how } => {
                let Value::Text(s) = v else {
                    return self.op == Op::Ne;
                };
                match self.op {
                    Op::Eq => text_matches(pattern, how, s),
                    Op::Ne => !text_matches(pattern, how, s),
                    op => op.holds(Some(compare_text(s, pattern))),
                }
            }
            Operand::Bool(b) => match v {
                Value::Bool(x) => self.op.holds(Some(x.cmp(b))),
                _ => self.op == Op::Ne,
            },
            Operand::Error(e) => {
                let same = matches!(v, Value::Error(x) if x == e);
                match self.op {
                    Op::Eq => same,
                    Op::Ne => !same,
                    _ => false,
                }
            }
            Operand::Blank => match self.op {
                Op::Eq => v.is_empty(),
                _ => !v.is_empty(),
            },
            Operand::EmptyText => match v {
                Value::Empty => true,
                Value::Text(s) => s.is_empty(),
                _ => false,
            },
        }
    }

    /// Numeric reading of a cell for `=` / `<>`: numbers, and text that
    /// looks like a number ("numbers stored as text" do match).
    fn cell_number(&self, v: &Value) -> Option<f64> {
        match v {
            Value::Number(x) => Some(*x),
            Value::Text(s) if s.bytes().any(|b| b.is_ascii_digit()) => {
                parse_numeric_text(s, self.locale)
            }
            _ => None,
        }
    }
}

fn text_matches(pattern: &str, how: &TextMatch, text: &str) -> bool {
    match how {
        TextMatch::Exact => {
            // Cheap byte comparison first; full Unicode folding only when
            // non-ASCII characters are involved.
            (text.len() == pattern.len() && text.eq_ignore_ascii_case(pattern))
                || (!(text.is_ascii() && pattern.is_ascii())
                    && compare_text(pattern, text) == Ordering::Equal)
        }
        TextMatch::Wildcard { ascii: Some(glob) } if text.is_ascii() => {
            glob_match(glob, text.as_bytes())
        }
        TextMatch::Wildcard { .. } => wildcard_match(pattern, text),
    }
}

/// Compile a wildcard pattern (`*`, `?`, `~` escapes the next character)
/// for matching against ASCII text; `None` when the pattern is not ASCII.
fn compile_glob(pattern: &str) -> Option<Vec<Glob>> {
    if !pattern.is_ascii() {
        return None;
    }
    let mut out = Vec::with_capacity(pattern.len());
    let mut bytes = pattern.bytes();
    while let Some(b) = bytes.next() {
        match b {
            b'*' => {
                if out.last() != Some(&Glob::Any) {
                    out.push(Glob::Any);
                }
            }
            b'?' => out.push(Glob::One),
            b'~' => out.push(Glob::Lit(bytes.next().unwrap_or(b'~').to_ascii_lowercase())),
            b => out.push(Glob::Lit(b.to_ascii_lowercase())),
        }
    }
    Some(out)
}

/// Case-insensitive match of ASCII text against a compiled pattern,
/// backtracking to the last `*`.
fn glob_match(glob: &[Glob], text: &[u8]) -> bool {
    let (mut t, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match glob.get(p) {
            Some(Glob::Lit(c)) if *c == text[t].to_ascii_lowercase() => {
                t += 1;
                p += 1;
            }
            Some(Glob::One) => {
                t += 1;
                p += 1;
            }
            Some(Glob::Any) => {
                star = Some((p, t));
                p += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    glob[p.min(glob.len())..].iter().all(|g| *g == Glob::Any)
}

fn parse_text(s: &str, locale: &Locale) -> (Op, Operand) {
    if s.is_empty() {
        return (Op::Eq, Operand::EmptyText);
    }
    const PREFIXES: [(&str, Op); 6] = [
        (">=", Op::Ge),
        ("<=", Op::Le),
        ("<>", Op::Ne),
        (">", Op::Gt),
        ("<", Op::Lt),
        ("=", Op::Eq),
    ];
    let (op, rest) = PREFIXES
        .iter()
        .find_map(|(p, op)| s.strip_prefix(p).map(|rest| (*op, rest)))
        .unwrap_or((Op::Eq, s));
    if rest.is_empty() {
        return match op {
            Op::Eq | Op::Ne => (op, Operand::Blank),
            _ => (
                op,
                Operand::Text {
                    pattern: String::new(),
                    how: TextMatch::Exact,
                },
            ),
        };
    }
    if let Some(n) = parse_numeric_text(rest, locale) {
        return (op, Operand::Number(n));
    }
    if let Some(b) = locale.parse_bool(rest) {
        return (op, Operand::Bool(b));
    }
    if rest.starts_with('#')
        && let Some(e) = locale.parse_error(rest)
    {
        return (op, Operand::Error(e));
    }
    let how = if matches!(op, Op::Eq | Op::Ne) && has_wildcards(rest) {
        TextMatch::Wildcard {
            ascii: compile_glob(rest),
        }
    } else {
        TextMatch::Exact
    };
    (
        op,
        Operand::Text {
            pattern: rest.to_string(),
            how,
        },
    )
}

/// A range or array argument together with its real size: `values` may be
/// smaller because ranges are clamped to the used part of the sheet (cells
/// outside it are blank, which is what [`Grid::get`] returns for them).
pub struct Grid {
    pub values: Arc<Array>,
    pub rows: usize,
    pub cols: usize,
    /// The real size is known (a cell reference). For a computed array
    /// such as `A1:A100="x"` it is not: the array is as large as the used
    /// part of the range it was computed from.
    is_ref: bool,
}

impl Grid {
    /// Evaluate an argument as a block of cells; a scalar becomes 1x1.
    /// An argument that is itself an error propagates.
    pub fn eval(ctx: &EvalCtx, e: &Expr) -> R<Grid> {
        match ctx.eval_ref(e) {
            Ok(r) => Ok(Grid::from_ref(ctx, &r)),
            Err(_) => match ctx.eval(e) {
                Value::Error(err) => Err(err),
                v => {
                    let values = v.to_array();
                    Ok(Grid {
                        rows: values.rows,
                        cols: values.cols,
                        values,
                        is_ref: false,
                    })
                }
            },
        }
    }

    pub fn from_ref(ctx: &EvalCtx, r: &Reference) -> Grid {
        Grid {
            values: ctx.range_values(r),
            rows: r.rows() as usize,
            cols: r.cols() as usize,
            is_ref: true,
        }
    }

    /// The block with the top-left corner of `e` and the size of `shape`
    /// (how `SUMIF` / `AVERAGEIF` read their optional third argument).
    pub fn shaped_like(ctx: &EvalCtx, e: &Expr, shape: &Grid) -> R<Grid> {
        match ctx.eval_ref(e) {
            Ok(r) => {
                let r1 = (r.rect.r0 as u64 + shape.rows.max(1) as u64 - 1).min(MAX_ROWS as u64 - 1);
                let c1 = (r.rect.c0 as u64 + shape.cols.max(1) as u64 - 1).min(MAX_COLS as u64 - 1);
                let rect = Rect::new(r.rect.r0, r.rect.c0, r1 as u32, c1 as u32);
                Ok(Grid::from_ref(ctx, &Reference::new(r.sheet, rect)))
            }
            Err(_) => Grid::eval(ctx, e),
        }
    }

    /// Value at a position; blank outside the materialised part.
    pub fn get(&self, row: usize, col: usize) -> &Value {
        self.values.get(row, col)
    }

    /// Number of cells in the real range.
    pub fn count(&self) -> usize {
        self.rows * self.cols
    }

    /// Do the two blocks have the same shape? When one of them is a
    /// computed array, agreeing on the materialised part is enough, since
    /// such an array may have been cut down to the used cells.
    pub fn same_shape(&self, other: &Grid) -> bool {
        (self.rows == other.rows && self.cols == other.cols)
            || (!(self.is_ref && other.is_ref)
                && self.values.rows == other.values.rows
                && self.values.cols == other.values.cols)
    }

    /// For functions pairing two arrays cell by cell in reading order:
    /// the column counts to number the cells with, or `None` when the
    /// arrays do not hold the same number of cells.
    pub fn pairing(&self, other: &Grid) -> Option<(usize, usize)> {
        if self.count() == other.count() {
            Some((self.cols, other.cols))
        } else if !(self.is_ref && other.is_ref) && self.values.len() == other.values.len() {
            Some((self.values.cols, other.values.cols))
        } else {
            None
        }
    }

    /// The first error in the block, if any.
    pub fn first_error(&self) -> Option<ErrorKind> {
        self.values.iter().find_map(|v| v.as_error())
    }
}

/// Visit every position at which all `(range, criterion)` pairs match.
///
/// Only the materialised part of the ranges (and of `also`, typically the
/// range being summed) is visited; everything beyond is blank in all of
/// them. The return value is the number of further positions out there
/// that match as well (non-zero only when every criterion accepts blanks).
pub fn scan(
    grids: &[Grid],
    criteria: &[Criteria],
    also: Option<&Grid>,
    mut hit: impl FnMut(usize, usize),
) -> usize {
    let Some(first) = grids.first() else {
        return 0;
    };
    let mut rows = 0;
    let mut cols = 0;
    for g in grids.iter().chain(also) {
        rows = rows.max(g.values.rows);
        cols = cols.max(g.values.cols);
    }
    rows = rows.min(first.rows);
    cols = cols.min(first.cols);
    for r in 0..rows {
        'cell: for c in 0..cols {
            for (g, crit) in grids.iter().zip(criteria) {
                if !crit.matches(g.get(r, c)) {
                    continue 'cell;
                }
            }
            hit(r, c);
        }
    }
    let rest = first.count() - rows * cols;
    if rest > 0 && criteria.iter().all(|c| c.matches(&Value::Empty)) {
        rest
    } else {
        0
    }
}

/// Numbers found in `target` at the positions where all criteria match.
/// Text, booleans and blanks are skipped; an error at a matching position
/// propagates.
pub fn matching_numbers(target: &Grid, grids: &[Grid], criteria: &[Criteria]) -> R<Vec<f64>> {
    let mut out = Vec::new();
    let mut err = None;
    scan(grids, criteria, Some(target), |r, c| {
        match target.get(r, c) {
            Value::Number(n) => out.push(*n),
            Value::Error(e) if err.is_none() => err = Some(*e),
            _ => {}
        }
    });
    match err {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Split the `range1, criterion1, range2, criterion2, ...` tail of an
/// `*IFS` function. Every range must have the shape of `shape` (or of the
/// first range), otherwise `#VALUE!`.
pub fn ifs_args<'e>(
    ctx: &EvalCtx,
    pairs: &'e [Expr],
    shape: Option<&Grid>,
) -> R<(Vec<Grid>, Vec<Option<&'e Expr>>)> {
    if pairs.is_empty() || !pairs.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let mut grids: Vec<Grid> = Vec::with_capacity(pairs.len() / 2);
    let mut criteria = Vec::with_capacity(pairs.len() / 2);
    for pair in pairs.chunks(2) {
        let g = Grid::eval(ctx, &pair[0])?;
        if !g.same_shape(shape.or(grids.first()).unwrap_or(&g)) {
            return Err(ErrorKind::Value);
        }
        grids.push(g);
        criteria.push(Some(&pair[1]));
    }
    Ok((grids, criteria))
}

/// Shared body of `SUMIF` / `AVERAGEIF`, called as `(range, criterion,
/// [target])`: `f` receives the numbers of the target cells that match.
pub fn if_function(ctx: &EvalCtx, args: &[Expr], f: impl Fn(&[f64]) -> R<Value>) -> R<Value> {
    let range = Grid::eval(ctx, &args[0])?;
    let target = match args.get(2) {
        None | Some(Expr::Missing) => None,
        Some(e) => Some(Grid::shaped_like(ctx, e, &range)?),
    };
    let ranges = [range];
    with_criteria(ctx, &[args.get(1)], |criteria| {
        let target = target.as_ref().unwrap_or(&ranges[0]);
        f(&matching_numbers(target, &ranges, criteria)?)
    })
}

/// Shared body of `SUMIFS` / `AVERAGEIFS` / `MAXIFS` / `MINIFS`, called as
/// `(target, range1, criterion1, ...)`.
pub fn ifs_function(ctx: &EvalCtx, args: &[Expr], f: impl Fn(&[f64]) -> R<Value>) -> R<Value> {
    let target = Grid::eval(ctx, &args[0])?;
    let (ranges, criteria) = ifs_args(ctx, &args[1..], Some(&target))?;
    with_criteria(ctx, &criteria, |criteria| {
        f(&matching_numbers(&target, &ranges, criteria)?)
    })
}

/// Evaluate "scalar" arguments of a function that otherwise works on whole
/// ranges, and apply `f` to them. When one of them is an array (as `k` in
/// `LARGE(A1:A9,{1,2,3})`), `f` runs once per element and the results form
/// an array. Absent / omitted arguments are passed as `None`.
pub fn lift(
    ctx: &EvalCtx,
    exprs: &[Option<&Expr>],
    f: impl Fn(&[Option<Value>]) -> R<Value>,
) -> R<Value> {
    let vals: Vec<Option<Value>> = exprs
        .iter()
        .map(|e| match e {
            None | Some(Expr::Missing) => None,
            Some(e) => Some(ctx.eval(e)),
        })
        .collect();
    let mut rows = 1;
    let mut cols = 1;
    let mut any = false;
    for v in vals.iter().flatten() {
        if let Value::Array(a) = v
            && a.len() != 1
        {
            any = true;
            rows = rows.max(a.rows);
            cols = cols.max(a.cols);
        }
    }
    if !any {
        let scalars: Vec<Option<Value>> = vals
            .into_iter()
            .map(|v| v.map(Value::into_scalar))
            .collect();
        return f(&scalars);
    }
    let mut out = Array::new(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let scalars: Vec<Option<Value>> = vals
                .iter()
                .map(|v| v.as_ref().map(|v| v.get_broadcast(r, c)))
                .collect();
            out.set(r, c, wrap(f(&scalars)).into_scalar());
        }
    }
    Ok(Value::array(out))
}

/// [`lift`] for criterion arguments: `f` receives the parsed criteria.
pub fn with_criteria(
    ctx: &EvalCtx,
    exprs: &[Option<&Expr>],
    f: impl Fn(&[Criteria]) -> R<Value>,
) -> R<Value> {
    let locale = ctx.locale();
    lift(ctx, exprs, |vals| {
        let criteria: Vec<Criteria> = vals
            .iter()
            .map(|v| Criteria::parse(v.as_ref().unwrap_or(&Value::Empty), locale))
            .collect();
        f(&criteria)
    })
}

/// Number from an optional lifted argument (`default` when absent).
pub fn opt_number(ctx: &EvalCtx, v: &Option<Value>, default: f64) -> R<f64> {
    match v {
        None => Ok(default),
        Some(v) => ctx.to_number(v),
    }
}

/// A matrix argument as `(rows, cols, row-major numbers)`. Errors
/// propagate; blank cells, text and booleans are `#VALUE!`.
pub fn matrix(ctx: &EvalCtx, e: &Expr) -> R<(usize, usize, Vec<f64>)> {
    let g = Grid::eval(ctx, e)?;
    if let Some(err) = g.first_error() {
        return Err(err);
    }
    // A range reaching beyond the used cells necessarily contains blanks.
    if g.values.rows != g.rows || g.values.cols != g.cols || g.values.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut data = Vec::with_capacity(g.values.len());
    for v in g.values.iter() {
        match v {
            Value::Number(n) => data.push(*n),
            _ => return Err(ErrorKind::Value),
        }
    }
    Ok((g.rows, g.cols, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{eval, num, val, wb};

    fn crit(v: impl Into<Value>) -> Criteria {
        Criteria::parse(&v.into(), Locale::en())
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    fn t(s: &str) -> Value {
        Value::text(s)
    }

    #[test]
    fn numeric_equality() {
        let c = crit(5.0);
        assert!(c.matches(&n(5.0)));
        assert!(!c.matches(&n(6.0)));
        // Numbers stored as text match, booleans and blanks do not.
        assert!(c.matches(&t("5")));
        assert!(c.matches(&t(" 5.0 ")));
        assert!(!c.matches(&t("five")));
        assert!(!c.matches(&Value::Bool(true)));
        assert!(!crit(0.0).matches(&Value::Empty));
        assert!(!crit(1.0).matches(&Value::Bool(true)));
        // Numeric text is a numeric criterion.
        assert!(crit("5").matches(&n(5.0)));
        assert!(crit("=5").matches(&n(5.0)));
        assert!(crit("50%").matches(&n(0.5)));
        assert!(crit("1e3").matches(&n(1000.0)));
        assert!(!crit("5").matches(&n(5.5)));
    }

    #[test]
    fn numeric_operators() {
        assert!(crit(">5").matches(&n(6.0)));
        assert!(!crit(">5").matches(&n(5.0)));
        assert!(crit(">=5").matches(&n(5.0)));
        assert!(crit("<5").matches(&n(-1.0)));
        assert!(!crit("<5").matches(&n(5.0)));
        assert!(crit("<=5").matches(&n(5.0)));
        assert!(crit("<>5").matches(&n(4.0)));
        assert!(!crit("<>5").matches(&n(5.0)));
        assert!(crit("> 5").matches(&n(6.0)));
        assert!(crit(">-1.5").matches(&n(-1.0)));
        // Ordering comparisons ignore blanks, text and booleans.
        assert!(!crit("<5").matches(&Value::Empty));
        assert!(!crit(">0").matches(&t("7")));
        assert!(!crit(">0").matches(&Value::Bool(true)));
        assert!(!crit(">0").matches(&t("abc")));
        // "<>" accepts everything that is not that number.
        assert!(crit("<>5").matches(&Value::Empty));
        assert!(crit("<>5").matches(&t("abc")));
        assert!(!crit("<>5").matches(&t("5")));
    }

    #[test]
    fn dates_in_criteria() {
        // 2024-01-15 is serial 45306.
        let serial = crate::datetime::date_to_serial(2024, 1, 15).unwrap();
        assert!(crit(">=1/15/2024").matches(&n(serial)));
        assert!(!crit(">1/15/2024").matches(&n(serial)));
        assert!(crit("1/15/2024").matches(&n(serial)));
        assert!(crit("<1/16/2024").matches(&n(serial)));
    }

    #[test]
    fn locale_aware_numbers() {
        let fr = |text: &str| Criteria::parse(&Value::text(text), Locale::fr());
        assert!(fr(">1,5").matches(&n(2.0)));
        assert!(!fr(">1,5").matches(&n(1.2)));
        assert!(fr("1,5").matches(&n(1.5)));
        assert!(fr("1,5").matches(&t("1,5")));
        assert!(fr("VRAI").matches(&Value::Bool(true)));
    }

    #[test]
    fn text_and_wildcards() {
        let c = crit("apple");
        assert!(c.matches(&t("apple")));
        assert!(c.matches(&t("APPLE")));
        assert!(!c.matches(&t("apples")));
        assert!(!c.matches(&n(1.0)));
        assert!(!c.matches(&Value::Empty));
        assert!(crit("=apple").matches(&t("Apple")));
        assert!(crit("a*").matches(&t("Apple")));
        assert!(crit("*pl*").matches(&t("apple")));
        assert!(!crit("a*").matches(&t("banana")));
        assert!(crit("?pple").matches(&t("apple")));
        assert!(!crit("?pple").matches(&t("pple")));
        assert!(crit("a~*").matches(&t("a*")));
        assert!(!crit("a~*").matches(&t("ab")));
        assert!(crit("~?").matches(&t("?")));
        // "*" is any text (including empty text) but not numbers or blanks.
        assert!(crit("*").matches(&t("x")));
        assert!(crit("*").matches(&t("")));
        assert!(!crit("*").matches(&n(3.0)));
        assert!(!crit("*").matches(&Value::Empty));
        // A pattern never matches a number, even one that looks alike.
        assert!(!crit("1*").matches(&n(12.0)));
        assert!(crit("1*").matches(&t("12")));
        // "<>" with text: everything but the matching text.
        assert!(crit("<>apple").matches(&t("pear")));
        assert!(!crit("<>apple").matches(&t("APPLE")));
        assert!(crit("<>apple").matches(&n(1.0)));
        assert!(crit("<>apple").matches(&Value::Empty));
        assert!(!crit("<>a*").matches(&t("avocado")));
        assert!(crit("<>a*").matches(&t("banana")));
        // Non-ASCII text goes through full case folding.
        assert!(crit("été").matches(&t("ÉTÉ")));
        assert!(!crit("été").matches(&t("ete")));
        assert!(crit("é*").matches(&t("Étoile")));
        assert!(crit("*é").matches(&t("café")));
        assert!(crit("caf?").matches(&t("CAFÉ")));
        assert!(!crit("caf?").matches(&t("cafés")));
        assert!(crit("a*b*c").matches(&t("aXbYbZc")));
        assert!(!crit("a*b*c").matches(&t("aXbYbZ")));
        assert!(crit("*").matches(&t("日本")));
        assert!(crit("??").matches(&t("日本")));
        assert!(crit("a~").matches(&t("a~")));
        assert!(crit("~~").matches(&t("~")));
        assert!(crit("***a***").matches(&t("banana")));
        assert!(!crit("?").matches(&t("")));
        // Ordering on text is alphabetical, case-insensitive, text only.
        assert!(crit(">b").matches(&t("cherry")));
        assert!(crit(">b").matches(&t("Cherry")));
        assert!(!crit(">b").matches(&t("apple")));
        assert!(!crit(">b").matches(&n(100.0)));
        assert!(crit("<=b").matches(&t("B")));
    }

    #[test]
    fn blanks() {
        // "" matches blank cells and empty strings.
        assert!(crit("").matches(&Value::Empty));
        assert!(crit("").matches(&t("")));
        assert!(!crit("").matches(&t("x")));
        assert!(!crit("").matches(&n(0.0)));
        // "=" matches only truly blank cells.
        assert!(crit("=").matches(&Value::Empty));
        assert!(!crit("=").matches(&t("")));
        assert!(!crit("=").matches(&n(0.0)));
        // "<>" matches everything that is not blank.
        assert!(!crit("<>").matches(&Value::Empty));
        assert!(crit("<>").matches(&t("")));
        assert!(crit("<>").matches(&n(0.0)));
        assert!(crit("<>").matches(&t("x")));
        // A reference to an empty cell counts as 0.
        assert!(crit(Value::Empty).matches(&n(0.0)));
        assert!(!crit(Value::Empty).matches(&Value::Empty));
    }

    #[test]
    fn booleans_and_errors() {
        assert!(crit(true).matches(&Value::Bool(true)));
        assert!(!crit(true).matches(&Value::Bool(false)));
        assert!(!crit(true).matches(&n(1.0)));
        assert!(crit("TRUE").matches(&Value::Bool(true)));
        assert!(crit("=false").matches(&Value::Bool(false)));
        assert!(crit("<>TRUE").matches(&Value::Bool(false)));
        assert!(crit("<>TRUE").matches(&n(1.0)));
        assert!(!crit("<>TRUE").matches(&Value::Bool(true)));
        let na = Value::Error(ErrorKind::NA);
        let div = Value::Error(ErrorKind::Div0);
        assert!(crit("#N/A").matches(&na));
        assert!(!crit("#N/A").matches(&div));
        assert!(crit(ErrorKind::Div0).matches(&div));
        assert!(crit("<>#N/A").matches(&div));
        assert!(!crit("<>#N/A").matches(&na));
        assert!(!crit(">0").matches(&na));
        assert!(!crit("x").matches(&na));
    }

    #[test]
    fn scan_counts_blank_tail() {
        // A whole-column range is clamped to the used rows; blanks beyond
        // them still count for criteria that accept blanks.
        let w = wb(&[
            ("A1", "x"),
            ("A3", "y"),
            ("C1", "=COUNTIF(A:A,\"\")"),
            ("C2", "=COUNTIF(A:A,\"<>x\")"),
            ("C3", "=COUNTIF(A1:A10,\"=\")"),
            ("C4", "=COUNTIF(A:A,\"<>\")"),
        ]);
        assert_eq!(val(&w, "C1"), Value::Number(1_048_574.0));
        assert_eq!(val(&w, "C2"), Value::Number(1_048_575.0));
        assert_eq!(val(&w, "C3"), Value::Number(8.0));
        assert_eq!(val(&w, "C4"), Value::Number(2.0));
    }

    #[test]
    fn lifting_over_array_criteria() {
        let w = wb(&[
            ("A1", "a"),
            ("A2", "b"),
            ("A3", "a"),
            ("B1", "1"),
            ("B2", "2"),
            ("B3", "4"),
            ("D1", "=SUMIF(A1:A3,{\"a\",\"b\"},B1:B3)"),
            ("D3", "=SUM(COUNTIF(A1:A3,{\"a\";\"b\";\"c\"}))"),
        ]);
        assert_eq!(val(&w, "D1"), Value::Number(5.0));
        assert_eq!(val(&w, "E1"), Value::Number(2.0));
        assert_eq!(val(&w, "D3"), Value::Number(3.0));
        // Counting distinct values with a criterion per cell.
        let w = wb(&[
            ("A1", "a"),
            ("A2", "b"),
            ("A3", "a"),
            ("A4", "c"),
            ("A5", "b"),
            ("C1", "=SUMPRODUCT(1/COUNTIF(A1:A5,A1:A5))"),
        ]);
        assert_eq!(val(&w, "C1"), Value::Number(3.0));
    }

    #[test]
    fn computed_arrays_next_to_ranges() {
        // Only rows 1-3 are in use, so `A1:A10="x"` is a 3-row array while
        // B1:B10 is known to have 10 rows: they must still line up.
        let w = wb(&[
            ("A1", "x"),
            ("A2", "y"),
            ("A3", "x"),
            ("B1", "1"),
            ("B2", "2"),
            ("B3", "3"),
            ("D1", "=SUMPRODUCT(--(A1:A10=\"x\"),B1:B10)"),
            ("D2", "=SUMPRODUCT((A1:A10=\"x\")*B1:B10)"),
            ("D3", "=SUMPRODUCT(B1:B10,{1;1;1;1;1;1;1;1;1;1})"),
            ("E1", "=CORREL(B1:B10*2,B1:B10)"),
            ("E2", "=SUMXMY2(B1:B10+1,B1:B10)"),
            ("E3", "=SUMPRODUCT(B1:B10,A1:A9)"),
        ]);
        assert_eq!(val(&w, "D1"), Value::Number(4.0));
        assert_eq!(val(&w, "D2"), Value::Number(4.0));
        assert_eq!(val(&w, "D3"), Value::Number(6.0));
        assert_eq!(val(&w, "E1"), Value::Number(1.0));
        assert_eq!(val(&w, "E2"), Value::Number(3.0));
        assert_eq!(val(&w, "E3"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn ranges_on_another_sheet() {
        // Sheet2 uses a single row while Sheet1 uses five: the criteria
        // range is materialised smaller than the range being summed.
        let mut w = wb(&[
            ("B1", "1"),
            ("B2", "2"),
            ("B3", "3"),
            ("B4", "4"),
            ("B5", "5"),
        ]);
        let other = w.add_sheet(1);
        w.set_cell_text(other, 0, 0, "x").unwrap();
        for (row, formula) in [
            "=SUMIF(Sheet2!A1:A5,\"\",B1:B5)",
            "=SUMIF(Sheet2!A1:A5,\"x\",B1:B5)",
            "=SUMIFS(B1:B5,Sheet2!A1:A5,\"<>x\")",
            "=COUNTIFS(Sheet2!A1:A5,\"\",B1:B5,\">2\")",
            "=SUMPRODUCT(Sheet2!A1:A5,B1:B5)",
            "=AVERAGEIF(Sheet2!A:A,\"\",B:B)",
            "=COUNTBLANK(Sheet2!A1:B5)",
        ]
        .iter()
        .enumerate()
        {
            w.set_cell_text(0, row as u32, 3, formula).unwrap();
        }
        w.commit("formulas");
        let d = |row: u32| w.value_at(0, row, 3);
        assert_eq!(d(0), Value::Number(14.0));
        assert_eq!(d(1), Value::Number(1.0));
        assert_eq!(d(2), Value::Number(14.0));
        assert_eq!(d(3), Value::Number(3.0));
        assert_eq!(d(4), Value::Number(0.0));
        assert_eq!(d(5), Value::Number(3.5));
        assert_eq!(d(6), Value::Number(9.0));
    }

    #[test]
    fn matrix_argument() {
        assert_eq!(num("MDETERM({1,2;3,4})"), -2.0);
        assert_eq!(
            eval("MDETERM({1,\"a\";3,4})"),
            Value::Error(ErrorKind::Value)
        );
        // Blank cells make a matrix invalid.
        let w = wb(&[
            ("A1", "1"),
            ("B2", "1"),
            ("D1", "=MDETERM(A1:B2)"),
            ("D2", "=MDETERM(A1:B3)"),
        ]);
        assert_eq!(val(&w, "D1"), Value::Error(ErrorKind::Value));
        assert_eq!(val(&w, "D2"), Value::Error(ErrorKind::Value));
    }
}
