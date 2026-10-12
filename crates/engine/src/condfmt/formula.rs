//! The formulas inside conditional formats: how they depend on the cell
//! they are read from, evaluating them for many cells at once, typing and
//! showing them in dialogs, and the formulas Excel stores in files for the
//! rules that have none of their own.

use super::model::{CondFormat, Rule, TextOp, TimePeriod};
use crate::ast::{BinaryOp, CellRef, Expr, RefExpr, RefKind, UnaryOp};
use crate::eval::Calc;
use crate::input;
use crate::locale::Locale;
use crate::parser::{ParseError, parse_formula};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, col_to_letters};
use crate::value::Value;
use crate::workbook::{Content, Workbook};
use std::sync::Arc;

// ---- how a formula depends on the cell reading it ---------------------------

/// A rule's formula as it reads from a cell (`d_row`, `d_col`) away from
/// the one it is written for: relative references move along, absolute
/// ones stay. Unlike a copied cell formula (`Expr::shifted`), a reference
/// pushed past an edge of the sheet comes back in at the opposite edge, as
/// in Excel: "the cell above", written for A1, is A1048576.
pub(super) fn shifted(e: &Expr, d_row: i64, d_col: i64) -> Expr {
    if (d_row, d_col) == (0, 0) {
        return e.clone();
    }
    e.map_refs(&mut |r| {
        let moved = |c: &CellRef| CellRef {
            row: match c.abs_row || r.kind == RefKind::Cols {
                true => c.row,
                false => (c.row as i64 + d_row).rem_euclid(MAX_ROWS as i64) as u32,
            },
            col: match c.abs_col || r.kind == RefKind::Rows {
                true => c.col,
                false => (c.col as i64 + d_col).rem_euclid(MAX_COLS as i64) as u32,
            },
            ..*c
        };
        Expr::Ref(RefExpr {
            sheet: r.sheet.clone(),
            start: moved(&r.start),
            end: r.end.as_ref().map(moved),
            kind: r.kind,
        })
    })
}

/// How the value of a rule's formula changes from cell to cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Position {
    /// The same value for every cell: evaluated once.
    Fixed,
    /// Relative references, `ROW()` or `COLUMN()`: one value per cell, but
    /// many cells can be evaluated in one pass.
    Relative,
    /// Depends on the cell in a way only the evaluator knows (`CELL`,
    /// `INDIRECT` in R1C1 style): evaluated cell by cell.
    Opaque,
}

fn ref_is_relative(r: &RefExpr) -> bool {
    let end = r.end.unwrap_or(r.start);
    let rows = r.kind != RefKind::Cols && !(r.start.abs_row && end.abs_row);
    let cols = r.kind != RefKind::Rows && !(r.start.abs_col && end.abs_col);
    rows || cols
}

/// True when the formula has a relative reference, so that it reads
/// different cells depending on the cell it is evaluated for. Excel refuses
/// such formulas in the thresholds of colour scales, data bars and icon
/// sets.
pub fn is_relative(e: &Expr) -> bool {
    let mut relative = false;
    e.walk(&mut |x| {
        if let Expr::Ref(r) = x {
            relative |= ref_is_relative(r);
        }
    });
    relative
}

/// `ROW()` or `COLUMN()` without an argument: the row or column of the cell
/// the formula is evaluated for.
fn own_position_call(e: &Expr) -> Option<bool> {
    let Expr::Func(name, args) = e else {
        return None;
    };
    if !args.iter().all(|a| matches!(a, Expr::Missing)) {
        return None;
    }
    match &**name {
        "ROW" => Some(true),
        "COLUMN" => Some(false),
        _ => None,
    }
}

fn reads_own_position(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |x| found |= own_position_call(x).is_some());
    found
}

fn is_opaque(wb: &Workbook, e: &Expr, depth: u32) -> bool {
    let mut opaque = false;
    e.walk(&mut |x| match x {
        Expr::Func(name, args) => {
            opaque |= &**name == "CELL" || (&**name == "INDIRECT" && args.len() > 1);
        }
        // A name is evaluated where the formula is: what its definition
        // reads from the position cannot be replaced from here.
        Expr::Name(name) => {
            if let Some(def) = wb.defined_name(name) {
                opaque |= depth >= 8 || reads_own_position(def) || is_opaque(wb, def, depth + 1);
            }
        }
        _ => {}
    });
    opaque
}

pub(super) fn position(wb: &Workbook, e: &Expr) -> Position {
    if is_opaque(wb, e, 0) {
        Position::Opaque
    } else if is_relative(e) || reads_own_position(e) {
        Position::Relative
    } else {
        Position::Fixed
    }
}

/// The value of a constant operand, without going through the evaluator.
pub(super) fn literal(e: &Expr) -> Option<Value> {
    match e {
        Expr::Number(n) => Some(Value::Number(*n)),
        Expr::Text(s) => Some(Value::Text(s.clone())),
        Expr::Bool(b) => Some(Value::Bool(*b)),
        Expr::Error(err) => Some(Value::Error(*err)),
        Expr::Unary(UnaryOp::Neg, inner) => match &**inner {
            Expr::Number(n) => Some(Value::Number(-n)),
            _ => None,
        },
        _ => None,
    }
}

// ---- evaluation --------------------------------------------------------------

/// The formula with `ROW()` and `COLUMN()` replaced by the numbers they
/// give in the cell (`row`, `col`).
fn at_position(e: &Expr, row: u32, col: u32) -> Expr {
    if let Some(want_row) = own_position_call(e) {
        return Expr::Number(if want_row { row } else { col } as f64 + 1.0);
    }
    let sub = |x: &Expr| at_position(x, row, col);
    match e {
        Expr::Unary(op, x) => Expr::Unary(*op, Box::new(sub(x))),
        Expr::Percent(x) => Expr::Percent(Box::new(sub(x))),
        Expr::Paren(x) => Expr::Paren(Box::new(sub(x))),
        Expr::Binary(op, a, b) => Expr::Binary(*op, Box::new(sub(a)), Box::new(sub(b))),
        Expr::Func(name, args) => Expr::Func(name.clone(), args.iter().map(sub).collect()),
        Expr::Array(rows) => Expr::Array(rows.iter().map(|row| row.iter().map(sub).collect()).collect()),
        other => other.clone(),
    }
}

/// Evaluate a formula as if it lived in the cell `at`; arrays give their
/// top-left element.
pub(super) fn eval_at(wb: &Workbook, sheet: usize, at: (u32, u32), e: &Expr) -> Value {
    match literal(e) {
        Some(v) => v,
        None => Calc::eval_once(wb, sheet, at.0, at.1, e).into_scalar(),
    }
}

/// Evaluate many formulas in one pass of the evaluator. Starting a pass
/// costs a walk over the whole workbook, far more than a formula, so the
/// formulas travel together as the elements of one array constant. `None`
/// when the evaluator did not hand back one value per formula.
fn eval_together(wb: &Workbook, sheet: usize, at: (u32, u32), exprs: &mut Vec<Expr>) -> Option<Vec<Value>> {
    let count = exprs.len();
    let array = Expr::Array(vec![std::mem::take(exprs)]);
    let result = Calc::eval_once(wb, sheet, at.0, at.1, &array);
    if let Expr::Array(mut rows) = array {
        *exprs = rows.pop().unwrap_or_default();
    }
    match result {
        Value::Array(a) if a.rows == 1 && a.cols == count => Some(a.data.clone()),
        _ => None,
    }
}

/// The values of the rule formulas `exprs` (written for `anchor`) for each
/// of `cells`: `cells.len() * exprs.len()` values, cell by cell.
pub(super) fn eval_cells(
    wb: &Workbook,
    sheet: usize,
    anchor: (u32, u32),
    exprs: &[&Expr],
    opaque: bool,
    cells: &[(u32, u32)],
) -> Vec<Value> {
    if cells.is_empty() || exprs.is_empty() {
        return Vec::new();
    }
    let reads_position: Vec<bool> = exprs.iter().map(|e| !opaque && reads_own_position(e)).collect();
    let mut together = !opaque;
    let mut placed = Vec::with_capacity(cells.len() * exprs.len());
    for &(row, col) in cells {
        for (e, &reads) in exprs.iter().zip(&reads_position) {
            let mut x = shifted(e, row as i64 - anchor.0 as i64, col as i64 - anchor.1 as i64);
            if reads {
                x = at_position(&x, row, col);
                // A call hidden where the rewrite does not reach: this
                // formula has to be evaluated from its own cell.
                together &= !reads_own_position(&x);
            }
            placed.push(x);
        }
    }
    if together {
        if let Some(values) = eval_together(wb, sheet, cells[0], &mut placed) {
            return values;
        }
    }
    placed
        .iter()
        .enumerate()
        .map(|(i, e)| eval_at(wb, sheet, cells[i / exprs.len()], e))
        .collect()
}

// ---- dialogs ------------------------------------------------------------------

fn offset(from: (u32, u32), to: (u32, u32)) -> (i64, i64) {
    (to.0 as i64 - from.0 as i64, to.1 as i64 - from.1 as i64)
}

/// What the user typed in a value box of a rule dialog ("5", "12/03/2025",
/// "abc", "=$B$1", "=B1*2"), as an operand of the rule. Numbers, dates and
/// formulas are read in the workbook locale. A formula is typed for the
/// active cell and stored for the rule's anchor (the top-left cell of its
/// first range): pass both.
pub fn parse_operand(text: &str, locale: &Locale, active: (u32, u32), anchor: (u32, u32)) -> Result<Expr, ParseError> {
    let (d_row, d_col) = offset(active, anchor);
    match input::parse_input(text.trim(), locale)?.content {
        Content::Formula(e) => Ok(shifted(&e, d_row, d_col)),
        Content::Value(Value::Number(n)) => Ok(Expr::Number(n)),
        Content::Value(Value::Text(s)) => Ok(Expr::Text(s)),
        Content::Value(Value::Bool(b)) => Ok(Expr::Bool(b)),
        Content::Value(Value::Error(e)) => Ok(Expr::Error(e)),
        Content::Value(_) | Content::Empty => Err(ParseError {
            pos: 0,
            message: "a value is required".to_string(),
        }),
    }
}

/// What the user typed in the box of a text rule ("Text that Contains"):
/// the text itself, or a formula giving it when it starts with `=`.
pub fn parse_text_operand(
    text: &str,
    locale: &Locale,
    active: (u32, u32),
    anchor: (u32, u32),
) -> Result<Expr, ParseError> {
    match text.strip_prefix('=') {
        Some(formula) if !formula.trim().is_empty() => {
            let (d_row, d_col) = offset(active, anchor);
            Ok(shifted(&parse_formula(formula, locale)?, d_row, d_col))
        }
        _ => Ok(Expr::Text(Arc::from(text))),
    }
}

/// The text to show in a dialog box for an operand: the reverse of
/// `parse_operand` (and of `parse_text_operand` when `bare_text` is set,
/// where a text constant is shown without quotes). Formulas are shown as
/// they read from the active cell.
pub fn operand_text(e: &Expr, locale: &Locale, anchor: (u32, u32), active: (u32, u32), bare_text: bool) -> String {
    match e {
        Expr::Number(n) => locale.number_to_text(*n),
        Expr::Text(s) if bare_text => s.to_string(),
        _ => {
            let (d_row, d_col) = offset(anchor, active);
            format!("={}", shifted(e, d_row, d_col).to_text(locale))
        }
    }
}

/// The "Applies to" box of the Rules Manager: `=$A$1:$B$10,$D$1` (the list
/// separator is the locale's: `;` in French).
pub fn ranges_text(ranges: &[Rect], locale: &Locale) -> String {
    let mut out = String::from("=");
    for (i, r) in ranges.iter().enumerate() {
        if i > 0 {
            out.push(locale.arg_sep);
        }
        let (c0, c1) = (col_to_letters(r.c0), col_to_letters(r.c1));
        let (r0, r1) = (r.r0 + 1, r.r1 + 1);
        if r.is_whole_cols() && !r.is_whole_rows() {
            out.push_str(&format!("${c0}:${c1}"));
        } else if r.is_whole_rows() && !r.is_whole_cols() {
            out.push_str(&format!("${r0}:${r1}"));
        } else if r.is_single() {
            out.push_str(&format!("${c0}${r0}"));
        } else {
            out.push_str(&format!("${c0}${r0}:${c1}${r1}"));
        }
    }
    out
}

/// Read an "Applies to" box: ranges separated by `,` or `;`, with or
/// without `=`, `$` and a sheet name. `None` when a part is not a range or
/// when there is none.
pub fn parse_ranges(text: &str) -> Option<Vec<Rect>> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    let mut out = Vec::new();
    for part in text.split([',', ';']) {
        let part = part.trim();
        let part = part.rsplit_once('!').map_or(part, |(_, cells)| cells);
        out.push(Rect::parse(part)?);
    }
    (!out.is_empty()).then_some(out)
}

// ---- files --------------------------------------------------------------------

/// The `formula` children Excel writes inside the `cfRule` of this rule,
/// in canonical (English) form: the operands of a `cellIs` rule, the
/// formula of an `expression` rule and, for text, blank, error and date
/// rules, the formula spelling out the test for the anchor cell (Excel
/// stores it next to the attributes and reads it back in preference to
/// them). Empty for the other rules.
pub fn ooxml_formulas(cf: &CondFormat) -> Vec<Expr> {
    let en = Locale::en();
    let anchor = cf.anchor();
    let c = RefExpr::cell(anchor.0, anchor.1).to_text();
    let parsed = |text: String| -> Vec<Expr> { parse_formula(&text, en).into_iter().collect() };
    match &cf.rule {
        Rule::CellIs { first, second, .. } => std::iter::once(first.clone()).chain(second.clone()).collect(),
        Rule::Formula(e) => vec![e.clone()],
        Rule::Text { op, text } => {
            let t = text.to_text(en);
            parsed(match op {
                TextOp::Contains => format!("NOT(ISERROR(SEARCH({t},{c})))"),
                TextOp::NotContains => format!("ISERROR(SEARCH({t},{c}))"),
                TextOp::BeginsWith => format!("LEFT({c},LEN({t}))={t}"),
                TextOp::EndsWith => format!("RIGHT({c},LEN({t}))={t}"),
            })
        }
        Rule::Blanks => parsed(format!("LEN(TRIM({c}))=0")),
        Rule::NoBlanks => parsed(format!("LEN(TRIM({c}))>0")),
        Rule::Errors => parsed(format!("ISERROR({c})")),
        Rule::NoErrors => parsed(format!("NOT(ISERROR({c}))")),
        Rule::DateOccurring(period) => parsed(match period {
            TimePeriod::Today => format!("FLOOR({c},1)=TODAY()"),
            TimePeriod::Yesterday => format!("FLOOR({c},1)=TODAY()-1"),
            TimePeriod::Tomorrow => format!("FLOOR({c},1)=TODAY()+1"),
            TimePeriod::Last7Days => format!("AND(TODAY()-FLOOR({c},1)<=6,FLOOR({c},1)<=TODAY())"),
            TimePeriod::ThisWeek => format!(
                "AND(TODAY()-ROUNDDOWN({c},0)<=WEEKDAY(TODAY())-1,ROUNDDOWN({c},0)-TODAY()<=7-WEEKDAY(TODAY()))"
            ),
            TimePeriod::LastWeek => format!(
                "AND(TODAY()-ROUNDDOWN({c},0)>=(WEEKDAY(TODAY())),TODAY()-ROUNDDOWN({c},0)<(WEEKDAY(TODAY())+7))"
            ),
            TimePeriod::NextWeek => format!(
                "AND(ROUNDDOWN({c},0)-TODAY()>(7-WEEKDAY(TODAY())),ROUNDDOWN({c},0)-TODAY()<(15-WEEKDAY(TODAY())))"
            ),
            TimePeriod::ThisMonth => format!("AND(MONTH({c})=MONTH(TODAY()),YEAR({c})=YEAR(TODAY()))"),
            TimePeriod::LastMonth => {
                format!("AND(MONTH({c})=MONTH(EDATE(TODAY(),0-1)),YEAR({c})=YEAR(EDATE(TODAY(),0-1)))")
            }
            TimePeriod::NextMonth => {
                format!("AND(MONTH({c})=MONTH(EDATE(TODAY(),0+1)),YEAR({c})=YEAR(EDATE(TODAY(),0+1)))")
            }
        }),
        _ => Vec::new(),
    }
}

/// The argument list of a call to `name`.
fn call<'a>(e: &'a Expr, name: &str) -> Option<&'a [Expr]> {
    match e {
        Expr::Func(f, args) if &**f == name => Some(args),
        Expr::Paren(inner) => call(inner, name),
        _ => None,
    }
}

/// The operand of a text rule read from a file: taken out of the rule's
/// `formula` when it has the shape Excel writes (the text may be a cell
/// reference there), else the `text` attribute. `None` when there is a
/// formula of another shape: Excel would apply that formula, so the rule
/// is best loaded as `Rule::Formula`.
pub fn text_operand_from_ooxml(op: TextOp, text: Option<&str>, formula: Option<&Expr>) -> Option<Expr> {
    let Some(formula) = formula else {
        return text.map(|t| Expr::Text(Arc::from(t)));
    };
    let searched = |e: &Expr| call(call(e, "ISERROR")?.first()?, "SEARCH")?.first().cloned();
    let compared = |side: &str| match formula {
        Expr::Binary(BinaryOp::Eq, cut, operand) if call(cut, side).is_some() => Some((**operand).clone()),
        _ => None,
    };
    match op {
        TextOp::Contains => searched(call(formula, "NOT")?.first()?),
        TextOp::NotContains => searched(formula),
        TextOp::BeginsWith => compared("LEFT"),
        TextOp::EndsWith => compared("RIGHT"),
    }
}
