//! Lookup & reference functions, including the dynamic-array family
//! (`FILTER`, `SORT`, `UNIQUE`, `TAKE`, ...).
//!
//! # Sizes
//!
//! The evaluator hands ranges over clamped to the used part of the sheet.
//! [`Grid`] puts the true size of a referenced range back, so positions
//! (`MATCH`, `INDEX`, `TAKE(..., -1)`) and shape checks are those of the
//! range as written. Two exceptions:
//!
//! * a range running to the edge of the sheet (`A:A`, `2:2`) given to a
//!   function that reproduces its input is trimmed to its own data (trailing
//!   blank rows / columns are dropped) instead of yielding a million blank
//!   rows;
//! * a function whose result is itself a reference (`OFFSET`, `INDIRECT`,
//!   `INDEX` and `XLOOKUP` into a range) evaluates exactly like that
//!   reference written out, i.e. as the evaluator would hand it over.

use super::{Category, FunctionDef, Kind, VAR, has_wildcards};
use crate::ast::{Expr, write_sheet_name};
use crate::eval::{EvalCtx, compare_text, compare_values, wrap};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, Reference, col_to_letters};
use crate::value::{Array, ErrorKind, R, Value};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

/// Ranges with more cells than this are trimmed to their data before being
/// copied, and `EXPAND` refuses to build anything larger.
const MAX_CELLS: usize = 1 << 24;

// ---- arguments -----------------------------------------------------------

/// True when the expression may denote cells: a reference, a name, or a call
/// to a function able to return a reference (`OFFSET`, `INDEX`, ...).
fn may_be_ref(e: &Expr) -> bool {
    match e {
        Expr::Ref(_) | Expr::Name(_) => true,
        Expr::Paren(inner) => may_be_ref(inner),
        Expr::Func(name, _) => super::lookup(name).is_some_and(|d| d.ref_func.is_some()),
        _ => false,
    }
}

/// An evaluated array argument together with its true dimensions.
struct Grid {
    /// The values. May cover fewer rows / columns than the grid when the
    /// range extends past the used part of the sheet: the rest is blank.
    arr: Arc<Array>,
    rows: usize,
    cols: usize,
    /// Dimensions as written; they differ from `rows` / `cols` only once a
    /// range has been [`trimmed`](Grid::trimmed).
    full_rows: usize,
    full_cols: usize,
    /// The cells the values came from, when the argument was a reference.
    reference: Option<Reference>,
}

impl Grid {
    fn from_value(v: Value) -> Grid {
        let arr = match v {
            Value::Array(a) if !a.is_empty() => a,
            Value::Array(_) => Arc::new(Array::row(vec![Value::Error(ErrorKind::Calc)])),
            v => Arc::new(Array::row(vec![v])),
        };
        Grid {
            rows: arr.rows,
            cols: arr.cols,
            full_rows: arr.rows,
            full_cols: arr.cols,
            arr,
            reference: None,
        }
    }

    fn from_ref(ctx: &EvalCtx, reference: Reference) -> Grid {
        let rect = reference.rect;
        let arr = if reference.is_single() {
            Arc::new(Array::row(vec![ctx.cell_value(
                reference.sheet,
                rect.r0,
                rect.c0,
            )]))
        } else {
            ctx.range_values(&reference)
        };
        let (rows, cols) = (rect.rows() as usize, rect.cols() as usize);
        Grid {
            arr,
            rows,
            cols,
            full_rows: rows,
            full_cols: cols,
            reference: Some(reference),
        }
    }

    /// Evaluate an argument, keeping the true size of a referenced range.
    fn eval(ctx: &EvalCtx, e: &Expr) -> Grid {
        if may_be_ref(e)
            && let Ok(reference) = ctx.eval_ref(e)
        {
            return Grid::from_ref(ctx, reference);
        }
        Grid::from_value(ctx.eval(e))
    }

    /// Evaluate the array argument of a function whose result is shaped
    /// after it. A lone error value propagates.
    fn data(ctx: &EvalCtx, e: &Expr) -> R<Grid> {
        let g = Grid::eval(ctx, e).trimmed();
        match g.scalar_error() {
            Some(err) => Err(err),
            None => Ok(g),
        }
    }

    /// Cut a range running to the edge of the sheet (or one too large to
    /// copy) down to its data, dropping trailing blank rows and columns.
    ///
    /// Trimming to the range's own content rather than to the used part of
    /// the sheet keeps the result independent of what else is on the sheet,
    /// in particular of the cells the result itself spills into.
    fn trimmed(mut self) -> Grid {
        let Some(rect) = self.reference.map(|r| r.rect) else {
            return self;
        };
        let huge = self.rows.saturating_mul(self.cols) > MAX_CELLS;
        let trim_rows = huge || rect.r1 == MAX_ROWS - 1;
        let trim_cols = huge || rect.c1 == MAX_COLS - 1;
        if trim_rows {
            self.rows = self.rows.min(self.arr.rows);
        }
        if trim_cols {
            self.cols = self.cols.min(self.arr.cols);
        }
        if trim_rows {
            while self.rows > 1 && (0..self.cols).all(|c| self.get(self.rows - 1, c).is_empty()) {
                self.rows -= 1;
            }
        }
        if trim_cols {
            while self.cols > 1 && (0..self.rows).all(|r| self.get(r, self.cols - 1).is_empty()) {
                self.cols -= 1;
            }
        }
        self
    }

    /// Element at (row, col); blank past the materialised values.
    fn get(&self, r: usize, c: usize) -> &Value {
        self.arr.get(r, c)
    }

    fn is_scalar(&self) -> bool {
        self.rows == 1 && self.cols == 1
    }

    /// The error to propagate when the argument was a lone error value
    /// (as opposed to a cell or array that happens to contain one).
    fn scalar_error(&self) -> Option<ErrorKind> {
        if self.reference.is_none() && self.is_scalar() {
            self.get(0, 0).as_error()
        } else {
            None
        }
    }

    /// Copy out a rectangular part.
    fn block(&self, r0: usize, rows: usize, c0: usize, cols: usize) -> Array {
        let mut data = Vec::with_capacity(rows * cols);
        for r in r0..r0 + rows {
            for c in c0..c0 + cols {
                data.push(self.get(r, c).clone());
            }
        }
        Array { rows, cols, data }
    }

    /// Selected rows (or columns) in the given order.
    fn pick(&self, lines: &[usize], columns: bool) -> R<Value> {
        let (rows, cols) = if columns {
            (self.rows, lines.len())
        } else {
            (lines.len(), self.cols)
        };
        let mut data = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let v = if columns {
                    self.get(r, lines[c])
                } else {
                    self.get(lines[r], c)
                };
                data.push(v.clone());
            }
        }
        finish(Array { rows, cols, data })
    }
}

/// Turn a result array into a value: an empty array is `#CALC!`, a single
/// element is returned as a scalar.
fn finish(a: Array) -> R<Value> {
    match a.len() {
        0 => Err(ErrorKind::Calc),
        1 => Ok(a.data.into_iter().next().unwrap_or(Value::Empty)),
        _ => Ok(Value::array(a)),
    }
}

/// Whether the row counts (or column counts) of two arguments agree, going
/// by their sizes as written.
///
/// A computed array such as `B1:B10>0` only has the rows of the used part of
/// the sheet, so it is also accepted against a range whose used part has
/// that many rows; whatever lies beyond is blank on both sides.
fn lines_up(a: &Grid, b: &Grid, rows: bool) -> bool {
    let (a_len, a_have, b_len, b_have) = if rows {
        (a.full_rows, a.arr.rows, b.full_rows, b.arr.rows)
    } else {
        (a.full_cols, a.arr.cols, b.full_cols, b.arr.cols)
    };
    a_len == b_len
        || (b.reference.is_none() && b_len == a_have)
        || (a.reference.is_none() && a_len == b_have)
}

/// Optional numeric argument: `None` when absent or omitted.
fn opt_num(ctx: &EvalCtx, args: &[Expr], index: usize) -> R<Option<f64>> {
    match args.get(index) {
        None | Some(Expr::Missing) => Ok(None),
        Some(e) => ctx.num(e).map(Some),
    }
}

/// Optional argument that is present and not omitted.
fn given<'e>(ctx: &EvalCtx, args: &'e [Expr], index: usize) -> Option<&'e Expr> {
    args.get(index).filter(|e| !ctx.is_missing(e))
}

/// Apply `f` to every element of an array argument (or once to a scalar).
fn lift1(a: &Value, f: impl Fn(&Value) -> R<Value>) -> R<Value> {
    match a {
        Value::Array(arr) if arr.len() != 1 => {
            let data = arr.iter().map(|v| wrap(f(v)).into_scalar()).collect();
            Ok(Value::array(Array {
                rows: arr.rows,
                cols: arr.cols,
                data,
            }))
        }
        Value::Array(arr) => f(arr.get(0, 0)),
        v => f(v),
    }
}

/// Apply `f` element-wise over two broadcast arguments.
fn lift2(a: &Value, b: &Value, f: impl Fn(&Value, &Value) -> R<Value>) -> R<Value> {
    let (ar, ac) = a.dims();
    let (br, bc) = b.dims();
    let (rows, cols) = (ar.max(br), ac.max(bc));
    if rows == 1 && cols == 1 {
        return f(&a.get_broadcast(0, 0), &b.get_broadcast(0, 0));
    }
    let mut out = Array::new(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let v = f(&a.get_broadcast(r, c), &b.get_broadcast(r, c));
            out.set(r, c, wrap(v).into_scalar());
        }
    }
    Ok(Value::array(out))
}

// ---- searching -----------------------------------------------------------

/// One row or one column of a grid, as searched by the lookup functions.
struct Vector<'a> {
    grid: &'a Grid,
    vertical: bool,
    /// Number of positions (the true size of the range).
    len: usize,
    /// Positions holding materialised values; all later ones are blank.
    have: usize,
}

impl<'a> Vector<'a> {
    /// The first column of a grid.
    fn column(grid: &'a Grid) -> Vector<'a> {
        Vector {
            grid,
            vertical: true,
            len: grid.rows,
            have: grid.arr.rows.min(grid.rows),
        }
    }

    /// The first row of a grid.
    fn row(grid: &'a Grid) -> Vector<'a> {
        Vector {
            grid,
            vertical: false,
            len: grid.cols,
            have: grid.arr.cols.min(grid.cols),
        }
    }

    /// A grid that is a single column or a single row.
    fn of(grid: &'a Grid) -> Option<Vector<'a>> {
        if grid.cols == 1 {
            Some(Vector::column(grid))
        } else if grid.rows == 1 {
            Some(Vector::row(grid))
        } else {
            None
        }
    }

    fn get(&self, i: usize) -> &Value {
        if self.vertical {
            self.grid.get(i, 0)
        } else {
            self.grid.get(0, i)
        }
    }
}

/// Order of a candidate relative to the key. Lookups never compare across
/// types: a number only matches numbers, text only text, and so on.
fn key_cmp(v: &Value, key: &Value) -> Option<Ordering> {
    match (v, key) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
        (Value::Text(a), Value::Text(b)) => Some(compare_text(a, b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Any,
    One,
    Lit(char),
}

/// A wildcard pattern compiled once for a whole scan. Same rules as
/// [`wildcard_match`](super::wildcard_match): `?` matches one character, `*`
/// any run, `~` escapes the next character; case-insensitive.
struct Pattern {
    toks: Vec<Tok>,
    /// Case-folded characters of the text under test (non-ASCII text only).
    scratch: RefCell<Vec<char>>,
}

impl Pattern {
    fn new(pattern: &str) -> Pattern {
        let mut toks = Vec::new();
        let mut chars = pattern.chars();
        while let Some(c) = chars.next() {
            match c {
                '*' => {
                    if toks.last() != Some(&Tok::Any) {
                        toks.push(Tok::Any);
                    }
                }
                '?' => toks.push(Tok::One),
                '~' => match chars.next() {
                    Some(n) => toks.extend(n.to_lowercase().map(Tok::Lit)),
                    None => toks.push(Tok::Lit('~')),
                },
                c => toks.extend(c.to_lowercase().map(Tok::Lit)),
            }
        }
        Pattern {
            toks,
            scratch: RefCell::new(Vec::new()),
        }
    }

    fn matches(&self, text: &str) -> bool {
        if text.is_ascii() {
            let bytes = text.as_bytes();
            return self.glob(bytes.len(), |i| bytes[i].to_ascii_lowercase() as char);
        }
        let mut folded = self.scratch.borrow_mut();
        folded.clear();
        folded.extend(text.chars().flat_map(char::to_lowercase));
        self.glob(folded.len(), |i| folded[i])
    }

    /// Glob matching with backtracking to the last `*`, over `len`
    /// case-folded characters.
    fn glob(&self, len: usize, at: impl Fn(usize) -> char) -> bool {
        let toks = &self.toks;
        let (mut t, mut p) = (0usize, 0usize);
        let mut star: Option<(usize, usize)> = None;
        while t < len {
            match toks.get(p) {
                Some(Tok::Lit(c)) if *c == at(t) => {
                    t += 1;
                    p += 1;
                }
                Some(Tok::One) => {
                    t += 1;
                    p += 1;
                }
                Some(Tok::Any) => {
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
        while toks.get(p) == Some(&Tok::Any) {
            p += 1;
        }
        p == toks.len()
    }
}

/// An exact-match test.
struct Probe {
    key: Value,
    /// Set when the key is text to be matched as a wildcard pattern.
    pattern: Option<Pattern>,
    /// The key is blank and so matches blank cells (`XLOOKUP` / `XMATCH`).
    blank: bool,
    /// The key is plain ASCII text.
    ascii: bool,
}

impl Probe {
    /// `wildcards` says whether `*`, `?` and `~` in a text key are special.
    fn new(key: Value, wildcards: bool, blank: bool) -> Probe {
        let (pattern, ascii) = match &key {
            Value::Text(s) => {
                let pattern = (wildcards && has_wildcards(s)).then(|| Pattern::new(s));
                (pattern, s.is_ascii())
            }
            _ => (None, false),
        };
        Probe {
            key,
            pattern,
            blank,
            ascii,
        }
    }

    fn hit(&self, v: &Value) -> bool {
        match (&self.key, v) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => match &self.pattern {
                Some(pattern) => pattern.matches(b),
                // Scans spend their time here, mostly on mismatches: settle
                // those bytewise and keep the full Unicode comparison for
                // text that is not plain ASCII.
                None => {
                    a.eq_ignore_ascii_case(b)
                        || (!(self.ascii && b.is_ascii()) && compare_text(a, b) == Ordering::Equal)
                }
            },
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (_, Value::Empty) => self.blank,
            _ => false,
        }
    }
}

/// Linear scan for the first (or, in reverse, last) exact match.
fn find_exact(vec: &Vector, probe: &Probe, reverse: bool) -> Option<usize> {
    // Everything past the materialised part is blank.
    let tail = probe.blank && vec.len > vec.have;
    if reverse {
        if tail {
            return Some(vec.len - 1);
        }
        (0..vec.have).rev().find(|&i| probe.hit(vec.get(i)))
    } else {
        // The hot loop of exact lookups: walk the stored values directly.
        let arr = &vec.grid.arr;
        let stride = if vec.vertical { arr.cols.max(1) } else { 1 };
        let mut values = arr.data.iter().step_by(stride).take(vec.have);
        let hit = values.position(|v| probe.hit(v));
        hit.or(if tail { Some(vec.have) } else { None })
    }
}

/// Linear scan for an exact match or else the closest value on one side of
/// the key: `side` is `Less` for the next smaller item, `Greater` for the
/// next larger. The data need not be sorted.
fn find_nearest(vec: &Vector, key: &Value, side: Ordering, reverse: bool) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut visit = |i: usize| -> bool {
        let v = vec.get(i);
        match key_cmp(v, key) {
            Some(Ordering::Equal) => {
                best = Some(i);
                return true;
            }
            Some(o) if o == side => {
                // Closer to the key than the best so far?
                let closer = match best {
                    None => true,
                    Some(b) => key_cmp(v, vec.get(b)) == Some(side.reverse()),
                };
                if closer {
                    best = Some(i);
                }
            }
            _ => {}
        }
        false
    };
    if reverse {
        for i in (0..vec.have).rev() {
            if visit(i) {
                break;
            }
        }
    } else {
        for i in 0..vec.have {
            if visit(i) {
                break;
            }
        }
    }
    best
}

/// Binary search for the last position whose value satisfies `pred`, in
/// data ordered so that `pred` holds for a leading run. Values of another
/// type than the key (and blanks, and errors) are stepped over, which is
/// what makes `LOOKUP(2, 1/(...), ...)` find the last numeric entry.
fn bisect_last(vec: &Vector, key: &Value, pred: impl Fn(Ordering) -> bool) -> Option<usize> {
    let (mut lo, mut hi) = (0usize, vec.len);
    let mut found = None;
    while lo < hi {
        let mid = lo + (hi - lo - 1) / 2;
        // Nearest comparable value at or after the midpoint.
        let stop = hi.min(vec.have);
        let mut probe = None;
        let mut i = mid;
        while i < stop {
            if let Some(o) = key_cmp(vec.get(i), key) {
                probe = Some((i, o));
                break;
            }
            i += 1;
        }
        match probe {
            Some((i, o)) if pred(o) => {
                found = Some(i);
                lo = i + 1;
            }
            _ => hi = mid,
        }
    }
    found
}

/// Binary search for the first position whose value satisfies `pred`, in
/// data ordered so that `pred` holds for a trailing run.
fn bisect_first(vec: &Vector, key: &Value, pred: impl Fn(Ordering) -> bool) -> Option<usize> {
    let (mut lo, mut hi) = (0usize, vec.len);
    let mut found = None;
    while lo < hi {
        let mid = lo + (hi - lo - 1) / 2;
        // Nearest comparable value at or before the midpoint.
        let mut probe = None;
        let mut i = (mid + 1).min(vec.have);
        while i > lo {
            i -= 1;
            if let Some(o) = key_cmp(vec.get(i), key) {
                probe = Some((i, o));
                break;
            }
        }
        match probe {
            Some((i, o)) if pred(o) => {
                found = Some(i);
                hi = i;
            }
            _ => lo = mid + 1,
        }
    }
    found
}

/// Value is at or below the key.
fn at_most(o: Ordering) -> bool {
    o != Ordering::Greater
}

/// Value is at or above the key.
fn at_least(o: Ordering) -> bool {
    o != Ordering::Less
}

/// Match behaviour of `MATCH`, `VLOOKUP`, `HLOOKUP` and `LOOKUP`.
#[derive(Clone, Copy)]
enum Mode {
    /// First equal value; text keys may use wildcards.
    Exact,
    /// Largest value <= key in ascending data.
    Ascending,
    /// Smallest value >= key in descending data.
    Descending,
}

/// 0-based position of `key` in `vec`; `#N/A` when there is none.
fn classic_find(vec: &Vector, key: &Value, mode: Mode) -> R<usize> {
    let key = match key {
        Value::Error(e) => return Err(*e),
        // A blank lookup value counts as 0.
        Value::Empty => Value::Number(0.0),
        v => v.clone(),
    };
    let found = match mode {
        Mode::Exact => {
            if matches!(&key, Value::Text(s) if s.chars().count() > 255) {
                return Err(ErrorKind::Value);
            }
            find_exact(vec, &Probe::new(key, true, false), false)
        }
        Mode::Ascending => bisect_last(vec, &key, at_most),
        Mode::Descending => bisect_last(vec, &key, at_least),
    };
    found.ok_or(ErrorKind::NA)
}

/// `match_mode` and `search_mode` of `XMATCH` / `XLOOKUP`, validated.
fn modern_modes(ctx: &EvalCtx, args: &[Expr], at: usize) -> R<(i64, i64)> {
    let match_mode = ctx.num_or(args, at, 0.0)?.trunc();
    let search_mode = ctx.num_or(args, at + 1, 1.0)?.trunc();
    if ![0.0, -1.0, 1.0, 2.0].contains(&match_mode)
        || ![1.0, -1.0, 2.0, -2.0].contains(&search_mode)
    {
        return Err(ErrorKind::Value);
    }
    Ok((match_mode as i64, search_mode as i64))
}

/// 0-based position of `key` under `XMATCH` rules; `None` when not found.
fn modern_find(vec: &Vector, key: &Value, match_mode: i64, search_mode: i64) -> R<Option<usize>> {
    let (key, blank) = match key {
        Value::Error(e) => return Err(*e),
        Value::Empty => (Value::text(""), true),
        Value::Text(s) => (key.clone(), s.is_empty()),
        v => (v.clone(), false),
    };
    let reverse = search_mode < 0;
    let found = match (search_mode.abs(), match_mode) {
        (1, 0 | 2) => find_exact(vec, &Probe::new(key, match_mode == 2, blank), reverse),
        (1, m) => {
            let side = if m < 0 {
                Ordering::Less
            } else {
                Ordering::Greater
            };
            find_nearest(vec, &key, side, reverse)
        }
        // Wildcards cannot be combined with a binary search.
        (_, 2) => return Err(ErrorKind::Value),
        (_, m) => {
            // In ascending data the next smaller item is the last value at
            // or below the key; in descending data it is the first one.
            let hit = match (search_mode > 0, m) {
                (true, 1) => bisect_first(vec, &key, at_least),
                (true, _) => bisect_last(vec, &key, at_most),
                (false, -1) => bisect_first(vec, &key, at_most),
                (false, _) => bisect_last(vec, &key, at_least),
            };
            if m == 0 {
                hit.filter(|&i| key_cmp(vec.get(i), &key) == Some(Ordering::Equal))
            } else {
                hit
            }
        }
    };
    Ok(found)
}

// ---- VLOOKUP, HLOOKUP, LOOKUP, MATCH --------------------------------------

fn table_lookup(ctx: &EvalCtx, args: &[Expr], vertical: bool) -> R<Value> {
    let keys = ctx.eval(&args[0]);
    let table = Grid::eval(ctx, &args[1]);
    if let Some(e) = table.scalar_error() {
        return Err(e);
    }
    let index = ctx.eval(&args[2]);
    // Left out entirely the match is approximate; `VLOOKUP(a,b,2,)` is exact.
    let mode = match args.get(3) {
        None => Mode::Ascending,
        Some(Expr::Missing) => Mode::Exact,
        Some(e) => {
            if ctx.boolean(e)? {
                Mode::Ascending
            } else {
                Mode::Exact
            }
        }
    };
    let (vec, width) = if vertical {
        (Vector::column(&table), table.cols)
    } else {
        (Vector::row(&table), table.rows)
    };
    lift2(&keys, &index, |key, index| {
        if let Value::Error(e) = key {
            return Err(*e);
        }
        let n = ctx.to_number(index)?.trunc();
        if n < 1.0 {
            return Err(ErrorKind::Value);
        }
        if n > width as f64 {
            return Err(ErrorKind::Ref);
        }
        let pos = classic_find(&vec, key, mode)?;
        let k = n as usize - 1;
        let v = if vertical {
            table.get(pos, k)
        } else {
            table.get(k, pos)
        };
        Ok(v.clone())
    })
}

fn f_vlookup(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    table_lookup(ctx, args, true)
}

fn f_hlookup(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    table_lookup(ctx, args, false)
}

fn f_lookup(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let keys = ctx.eval(&args[0]);
    let lookup = Grid::eval(ctx, &args[1]);
    if let Some(e) = lookup.scalar_error() {
        return Err(e);
    }
    let result = match given(ctx, args, 2) {
        Some(e) => {
            let g = Grid::eval(ctx, e);
            if let Some(err) = g.scalar_error() {
                return Err(err);
            }
            Some(g)
        }
        None => None,
    };
    // A block wider than tall is searched along its first row, anything
    // else down its first column.
    let vec = if lookup.cols > lookup.rows {
        Vector::row(&lookup)
    } else {
        Vector::column(&lookup)
    };
    lift1(&keys, |key| {
        let i = classic_find(&vec, key, Mode::Ascending)?;
        let Some(res) = &result else {
            // Array form: answer from the last row / column.
            let v = if vec.vertical {
                lookup.get(i, lookup.cols - 1)
            } else {
                lookup.get(lookup.rows - 1, i)
            };
            return Ok(v.clone());
        };
        let down = if res.is_scalar() {
            vec.vertical
        } else if res.cols == 1 {
            true
        } else if res.rows == 1 {
            false
        } else {
            return Err(ErrorKind::NA);
        };
        let (r, c) = if down { (i, 0) } else { (0, i) };
        if r < res.rows && c < res.cols {
            return Ok(res.get(r, c).clone());
        }
        // A result range that is too short is read past its end.
        let Some(reference) = res.reference else {
            return Err(ErrorKind::NA);
        };
        let row = reference.rect.r0 as u64 + r as u64;
        let col = reference.rect.c0 as u64 + c as u64;
        if row >= MAX_ROWS as u64 || col >= MAX_COLS as u64 {
            return Err(ErrorKind::NA);
        }
        Ok(ctx.cell_value(reference.sheet, row as u32, col as u32))
    })
}

fn f_match(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let keys = ctx.eval(&args[0]);
    let grid = Grid::eval(ctx, &args[1]);
    if let Some(e) = grid.scalar_error() {
        return Err(e);
    }
    let match_type = match args.get(2) {
        None => 1.0,
        Some(Expr::Missing) => 0.0,
        Some(e) => ctx.num(e)?,
    };
    let mode = if match_type > 0.0 {
        Mode::Ascending
    } else if match_type < 0.0 {
        Mode::Descending
    } else {
        Mode::Exact
    };
    let vec = Vector::of(&grid).ok_or(ErrorKind::NA)?;
    lift1(&keys, |key| {
        let i = classic_find(&vec, key, mode)?;
        Ok(Value::Number(i as f64 + 1.0))
    })
}

// ---- XMATCH, XLOOKUP -------------------------------------------------------

fn f_xmatch(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let keys = ctx.eval(&args[0]);
    let grid = Grid::eval(ctx, &args[1]);
    if let Some(e) = grid.scalar_error() {
        return Err(e);
    }
    let (match_mode, search_mode) = modern_modes(ctx, args, 2)?;
    let vec = Vector::of(&grid).ok_or(ErrorKind::Value)?;
    lift1(&keys, |key| {
        match modern_find(&vec, key, match_mode, search_mode)? {
            Some(i) => Ok(Value::Number(i as f64 + 1.0)),
            None => Err(ErrorKind::NA),
        }
    })
}

/// The evaluated arguments of an `XLOOKUP` call.
struct XLookup {
    keys: Value,
    lookup: Grid,
    ret: Grid,
    /// The lookup array is a column, so a match selects a row of `ret`.
    vertical: bool,
    match_mode: i64,
    search_mode: i64,
}

impl XLookup {
    fn new(ctx: &EvalCtx, args: &[Expr]) -> R<XLookup> {
        let keys = ctx.eval(&args[0]);
        let lookup = Grid::eval(ctx, &args[1]);
        if let Some(e) = lookup.scalar_error() {
            return Err(e);
        }
        let ret = Grid::eval(ctx, &args[2]);
        if let Some(e) = ret.scalar_error() {
            return Err(e);
        }
        let (match_mode, search_mode) = modern_modes(ctx, args, 4)?;
        let vertical = if lookup.is_scalar() {
            ret.rows == 1
        } else if lookup.cols == 1 {
            true
        } else if lookup.rows == 1 {
            false
        } else {
            return Err(ErrorKind::Value);
        };
        if !lines_up(&lookup, &ret, vertical) {
            return Err(ErrorKind::Value);
        }
        Ok(XLookup {
            keys,
            lookup,
            ret,
            vertical,
            match_mode,
            search_mode,
        })
    }

    fn find(&self, key: &Value) -> R<Option<usize>> {
        let vec = if self.vertical {
            Vector::column(&self.lookup)
        } else {
            Vector::row(&self.lookup)
        };
        modern_find(&vec, key, self.match_mode, self.search_mode)
    }

    /// The cells of the return range selected by match `i`, when the return
    /// array is a range.
    fn matched_ref(&self, i: usize) -> Option<Reference> {
        let base = self.ret.reference?;
        let rect = base.rect;
        let i = i as u32;
        let rect = if self.vertical {
            Rect::new(rect.r0 + i, rect.c0, rect.r0 + i, rect.c1)
        } else {
            Rect::new(rect.r0, rect.c0 + i, rect.r1, rect.c0 + i)
        };
        Some(Reference::new(base.sheet, rect))
    }
}

fn f_xlookup(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = XLookup::new(ctx, args)?;
    // `if_not_found` is only evaluated when a lookup fails.
    let if_not_found = given(ctx, args, 3);
    match &x.keys {
        // Several lookup values: one result each (the first cell of the
        // matching row / column).
        Value::Array(keys) if keys.len() != 1 => {
            let mut fallback: Option<Value> = None;
            let mut data = Vec::with_capacity(keys.len());
            for key in keys.iter() {
                let v = match x.find(key) {
                    Err(e) => Value::Error(e),
                    Ok(Some(i)) => {
                        let v = if x.vertical {
                            x.ret.get(i, 0)
                        } else {
                            x.ret.get(0, i)
                        };
                        v.clone()
                    }
                    Ok(None) => match if_not_found {
                        Some(e) => fallback.get_or_insert_with(|| ctx.eval_scalar(e)).clone(),
                        None => Value::Error(ErrorKind::NA),
                    },
                };
                data.push(v);
            }
            Ok(Value::array(Array {
                rows: keys.rows,
                cols: keys.cols,
                data,
            }))
        }
        key => {
            let key = key.clone().into_scalar();
            match x.find(&key)? {
                Some(i) => {
                    if let Some(reference) = x.matched_ref(i) {
                        return Ok(ctx.deref(&reference));
                    }
                    let block = if x.vertical {
                        x.ret.block(i, 1, 0, x.ret.cols)
                    } else {
                        x.ret.block(0, x.ret.rows, i, 1)
                    };
                    finish(block)
                }
                None => match if_not_found {
                    Some(e) => Ok(ctx.eval(e)),
                    None => Err(ErrorKind::NA),
                },
            }
        }
    }
}

/// `XLOOKUP` as a reference: the matching row / column of the return range.
fn r_xlookup(ctx: &EvalCtx, args: &[Expr]) -> R<Reference> {
    if !may_be_ref(&args[2]) {
        return Err(ErrorKind::Value);
    }
    let x = XLookup::new(ctx, args)?;
    if x.keys.dims() != (1, 1) {
        return Err(ErrorKind::Value);
    }
    let key = x.keys.clone().into_scalar();
    match x.find(&key)? {
        Some(i) => x.matched_ref(i).ok_or(ErrorKind::Value),
        None => match given(ctx, args, 3) {
            Some(e) => ctx.eval_ref(e),
            None => Err(ErrorKind::NA),
        },
    }
}

// ---- INDEX, CHOOSE ---------------------------------------------------------

/// Resolve `INDEX` row / column numbers against a `rows` x `cols` block.
/// Returns 0-based positions; `None` selects the whole dimension.
fn index_pos(
    rows: usize,
    cols: usize,
    row: f64,
    col: Option<f64>,
) -> R<(Option<usize>, Option<usize>)> {
    let (r, c) = match col {
        Some(c) => (row, c),
        // A lone index into a single row counts along that row.
        None if rows == 1 => (1.0, row),
        None => (row, 0.0),
    };
    if r < 0.0 || c < 0.0 {
        return Err(ErrorKind::Value);
    }
    if r > rows as f64 || c > cols as f64 {
        return Err(ErrorKind::Ref);
    }
    let pick = |n: f64, len: usize| {
        if n >= 1.0 {
            Some(n as usize - 1)
        } else if len == 1 {
            Some(0)
        } else {
            None
        }
    };
    Ok((pick(r, rows), pick(c, cols)))
}

/// `area_num` can only be 1: references here are always a single area.
fn index_area(ctx: &EvalCtx, args: &[Expr]) -> R<()> {
    match opt_num(ctx, args, 3)? {
        Some(n) if n.trunc() != 1.0 => Err(ErrorKind::Ref),
        _ => Ok(()),
    }
}

fn index_numbers(ctx: &EvalCtx, row: &Value, col: Option<&Value>) -> R<(f64, Option<f64>)> {
    let r = ctx.to_number(row)?.trunc();
    let c = match col {
        Some(v) => Some(ctx.to_number(v)?.trunc()),
        None => None,
    };
    Ok((r, c))
}

/// The part of `base` selected by `INDEX` row / column numbers.
fn index_ref(ctx: &EvalCtx, base: Reference, row: &Value, col: Option<&Value>) -> R<Reference> {
    let (r, c) = index_numbers(ctx, row, col)?;
    let (r, c) = index_pos(base.rows() as usize, base.cols() as usize, r, c)?;
    let rect = base.rect;
    let (r0, r1) = match r {
        Some(r) => (rect.r0 + r as u32, rect.r0 + r as u32),
        None => (rect.r0, rect.r1),
    };
    let (c0, c1) = match c {
        Some(c) => (rect.c0 + c as u32, rect.c0 + c as u32),
        None => (rect.c0, rect.c1),
    };
    Ok(Reference::new(base.sheet, Rect::new(r0, c0, r1, c1)))
}

/// `INDEX` row and column arguments: an omitted row is 0, an absent column
/// is `None`.
fn index_args(ctx: &EvalCtx, args: &[Expr]) -> (Value, Option<Value>) {
    let row = match given(ctx, args, 1) {
        Some(e) => ctx.eval(e),
        None => Value::Number(0.0),
    };
    let col = match args.get(2) {
        None => None,
        Some(Expr::Missing) => Some(Value::Number(0.0)),
        Some(e) => Some(ctx.eval(e)),
    };
    (row, col)
}

fn f_index(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    index_area(ctx, args)?;
    let (row, col) = index_args(ctx, args);
    let lifted = row.dims() != (1, 1) || col.as_ref().is_some_and(|c| c.dims() != (1, 1));

    let mut base = None;
    if may_be_ref(&args[0]) {
        base = ctx.eval_ref(&args[0]).ok();
    }
    let grid = match base {
        // Scalar positions into a reference select cells: only those are
        // read, exactly as if they had been referred to directly.
        Some(base) if !lifted => {
            let part = index_ref(ctx, base, &row, col.as_ref())?;
            return Ok(ctx.deref(&part));
        }
        Some(base) => Grid::from_ref(ctx, base),
        None => Grid::from_value(ctx.eval(&args[0])),
    };
    if let Some(e) = grid.scalar_error() {
        return Err(e);
    }
    let pos = |row: &Value, col: Option<&Value>| {
        let (r, c) = index_numbers(ctx, row, col)?;
        index_pos(grid.rows, grid.cols, r, c)
    };
    if !lifted {
        let (r, c) = pos(&row, col.as_ref())?;
        let grid = grid.trimmed();
        let (r0, rows) = r.map_or((0, grid.rows), |r| (r, 1));
        let (c0, cols) = c.map_or((0, grid.cols), |c| (c, 1));
        return finish(grid.block(r0, rows, c0, cols));
    }
    // Arrays of positions: one element per (row, column) pair.
    let (rr, rc) = row.dims();
    let (cr, cc) = col.as_ref().map_or((1, 1), |c| c.dims());
    let (rows, cols) = (rr.max(cr), rc.max(cc));
    let mut out = Array::new(rows, cols);
    for i in 0..rows {
        for j in 0..cols {
            let rv = row.get_broadcast(i, j);
            let cv = col.as_ref().map(|c| c.get_broadcast(i, j));
            let v = match pos(&rv, cv.as_ref()) {
                Ok((r, c)) => grid.get(r.unwrap_or(0), c.unwrap_or(0)).clone(),
                Err(e) => Value::Error(e),
            };
            out.set(i, j, v);
        }
    }
    finish(out)
}

fn r_index(ctx: &EvalCtx, args: &[Expr]) -> R<Reference> {
    let base = ctx.eval_ref(&args[0])?;
    index_area(ctx, args)?;
    let (row, col) = index_args(ctx, args);
    index_ref(ctx, base, &row, col.as_ref())
}

fn choose_index(ctx: &EvalCtx, v: &Value, count: usize) -> R<usize> {
    let n = ctx.to_number(v)?.trunc();
    if n < 1.0 || n > 254.0 || n > count as f64 {
        return Err(ErrorKind::Value);
    }
    Ok(n as usize)
}

/// `CHOOSE` is lazy: only the chosen value is evaluated. With an array of
/// indexes every value is evaluated and the result is built element-wise.
fn f_choose(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let index = ctx.eval(&args[0]);
    let count = args.len() - 1;
    match &index {
        Value::Array(a) if a.len() != 1 => {
            let choices: Vec<Value> = args[1..].iter().map(|e| ctx.eval(e)).collect();
            let (mut rows, mut cols) = (a.rows, a.cols);
            for v in &choices {
                let (r, c) = v.dims();
                rows = rows.max(r);
                cols = cols.max(c);
            }
            let mut out = Array::new(rows, cols);
            for r in 0..rows {
                for c in 0..cols {
                    let v = match choose_index(ctx, &a.get_broadcast(r, c), count) {
                        Ok(i) => choices[i - 1].get_broadcast(r, c),
                        Err(e) => Value::Error(e),
                    };
                    out.set(r, c, v);
                }
            }
            Ok(Value::array(out))
        }
        _ => {
            let i = choose_index(ctx, &index, count)?;
            Ok(ctx.eval(&args[i]))
        }
    }
}

fn r_choose(ctx: &EvalCtx, args: &[Expr]) -> R<Reference> {
    let index = ctx.eval_scalar(&args[0]);
    let i = choose_index(ctx, &index, args.len() - 1)?;
    ctx.eval_ref(&args[i])
}

// ---- ROW, COLUMN, ROWS, COLUMNS, AREAS --------------------------------------

fn row_or_column(ctx: &EvalCtx, args: &[Expr], want_row: bool) -> R<Value> {
    let Some(arg) = given(ctx, args, 0) else {
        let n = if want_row { ctx.row } else { ctx.col };
        return Ok(Value::Number(n as f64 + 1.0));
    };
    let reference = ctx.eval_ref(arg)?;
    let rect = reference.rect;
    let (first, mut count, open) = if want_row {
        (rect.r0, rect.rows(), rect.r1 == MAX_ROWS - 1)
    } else {
        (rect.c0, rect.cols(), rect.c1 == MAX_COLS - 1)
    };
    if open && count > 1 {
        // A range running to the edge of the sheet is numbered over the used
        // part of the sheet only, in step with the values of that range as
        // the evaluator delivers them (`(A:A<>"")*ROW(A:A)`).
        let used = ctx.clamped(&reference);
        count = if want_row { used.rows() } else { used.cols() };
    }
    if count == 1 {
        return Ok(Value::Number(first as f64 + 1.0));
    }
    let numbers = (0..count)
        .map(|i| Value::Number((first + i) as f64 + 1.0))
        .collect();
    Ok(Value::array(if want_row {
        Array::column(numbers)
    } else {
        Array::row(numbers)
    }))
}

fn f_row(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    row_or_column(ctx, args, true)
}

fn f_column(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    row_or_column(ctx, args, false)
}

/// (rows, columns) of an argument: the full size of a reference, else the
/// size of the evaluated array.
fn dims(ctx: &EvalCtx, e: &Expr) -> R<(usize, usize)> {
    if may_be_ref(e)
        && let Ok(reference) = ctx.eval_ref(e)
    {
        return Ok((reference.rows() as usize, reference.cols() as usize));
    }
    match ctx.eval(e) {
        Value::Error(err) => Err(err),
        v => Ok(v.dims()),
    }
}

fn f_rows(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(dims(ctx, &args[0])?.0 as f64))
}

fn f_columns(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(dims(ctx, &args[0])?.1 as f64))
}

fn f_areas(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    ctx.eval_ref(&args[0])?;
    Ok(Value::Number(1.0))
}

// ---- OFFSET, INDIRECT, ADDRESS ----------------------------------------------

fn r_offset(ctx: &EvalCtx, args: &[Expr]) -> R<Reference> {
    let base = ctx.eval_ref(&args[0])?;
    let rect = base.rect;
    let down = ctx.int(&args[1])?;
    let right = ctx.int(&args[2])?;
    let height = match given(ctx, args, 3) {
        Some(e) => ctx.int(e)?,
        None => rect.rows() as i64,
    };
    let width = match given(ctx, args, 4) {
        Some(e) => ctx.int(e)?,
        None => rect.cols() as i64,
    };
    if height == 0 || width == 0 {
        return Err(ErrorKind::Ref);
    }
    // A negative height / width extends up / left from the anchor.
    let span = |start: i64, size: i64| {
        if size > 0 {
            (start, start + size - 1)
        } else {
            (start + size + 1, start)
        }
    };
    let (r0, r1) = span(rect.r0 as i64 + down, height);
    let (c0, c1) = span(rect.c0 as i64 + right, width);
    if r0 < 0 || c0 < 0 || r1 >= MAX_ROWS as i64 || c1 >= MAX_COLS as i64 {
        return Err(ErrorKind::Ref);
    }
    Ok(Reference::new(
        base.sheet,
        Rect::new(r0 as u32, c0 as u32, r1 as u32, c1 as u32),
    ))
}

fn f_offset(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(ctx.deref(&r_offset(ctx, args)?))
}

/// Split `Sheet!rest` or `'Quoted sheet'!rest`; without a prefix the sheet
/// is the calling one.
fn split_sheet<'t>(ctx: &EvalCtx, text: &'t str) -> Option<(usize, &'t str)> {
    let Some(bang) = text.rfind('!') else {
        return Some((ctx.sheet, text));
    };
    let (name, rest) = (&text[..bang], &text[bang + 1..]);
    let name = match name.strip_prefix('\'').and_then(|n| n.strip_suffix('\'')) {
        Some(quoted) => quoted.replace("''", "'"),
        None => name.to_string(),
    };
    ctx.workbook().sheet_index(&name).map(|sheet| (sheet, rest))
}

/// The coordinate following `R` or `C`: absolute (`7`), relative (`[-2]`)
/// or, when nothing follows, that of the calling cell.
fn r1c1_number(s: &str, current: u32, max: u32) -> Option<(u32, &str)> {
    if let Some(inner) = s.strip_prefix('[') {
        let (offset, tail) = inner.split_once(']')?;
        let n = current as i64 + offset.trim().parse::<i64>().ok()?;
        return (0..max as i64).contains(&n).then_some((n as u32, tail));
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Some((current, s));
    }
    let n: u32 = s[..digits].parse().ok()?;
    (1..=max).contains(&n).then(|| (n - 1, &s[digits..]))
}

/// One corner in R1C1 notation: `R2C3`, `R[-1]C`, `R5` (a whole row) or
/// `C[2]` (a whole column).
fn r1c1_corner(ctx: &EvalCtx, s: &str) -> Option<(Option<u32>, Option<u32>)> {
    let mut rest = s.trim();
    let mut row = None;
    let mut col = None;
    if let Some(after) = rest.strip_prefix(['R', 'r']) {
        let (n, tail) = r1c1_number(after, ctx.row, MAX_ROWS)?;
        row = Some(n);
        rest = tail;
    }
    if let Some(after) = rest.strip_prefix(['C', 'c']) {
        let (n, tail) = r1c1_number(after, ctx.col, MAX_COLS)?;
        col = Some(n);
        rest = tail;
    }
    if !rest.is_empty() || (row.is_none() && col.is_none()) {
        return None;
    }
    Some((row, col))
}

/// Parse R1C1-style reference text, relative to the calling cell.
fn parse_r1c1(ctx: &EvalCtx, text: &str) -> Option<Reference> {
    let (sheet, body) = split_sheet(ctx, text)?;
    let (first, second) = match body.split_once(':') {
        Some((a, b)) => (a, Some(b)),
        None => (body, None),
    };
    let (r0, c0) = r1c1_corner(ctx, first)?;
    let (r1, c1) = match second {
        Some(s) => r1c1_corner(ctx, s)?,
        None => (r0, c0),
    };
    let span = |a: Option<u32>, b: Option<u32>, max: u32| match (a, b) {
        (Some(a), Some(b)) => Some((a.min(b), a.max(b))),
        (None, None) => Some((0, max - 1)),
        _ => None,
    };
    let (r0, r1) = span(r0, r1, MAX_ROWS)?;
    let (c0, c1) = span(c0, c1, MAX_COLS)?;
    Some(Reference::new(sheet, Rect::new(r0, c0, r1, c1)))
}

fn r_indirect(ctx: &EvalCtx, args: &[Expr]) -> R<Reference> {
    let text = ctx.text(&args[0])?;
    // `INDIRECT("RC[-1]",)` is the customary shorthand for FALSE.
    let a1 = match args.get(1) {
        None => true,
        Some(Expr::Missing) => false,
        Some(e) => ctx.boolean(e)?,
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(ErrorKind::Ref);
    }
    if a1 {
        return ctx.parse_reference(text);
    }
    if let Some(reference) = parse_r1c1(ctx, text) {
        return Ok(reference);
    }
    // Defined names are valid in either notation.
    match ctx.workbook().defined_name(text) {
        Some(expr) => ctx.eval_ref(expr).map_err(|_| ErrorKind::Ref),
        None => Err(ErrorKind::Ref),
    }
}

fn f_indirect(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(ctx.deref(&r_indirect(ctx, args)?))
}

fn f_address(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let row = ctx.num(&args[0])?.trunc();
    let col = ctx.num(&args[1])?.trunc();
    let abs = ctx.num_or(args, 2, 1.0)?.trunc();
    let a1 = ctx.bool_or(args, 3, true)?;
    if row < 1.0
        || row > MAX_ROWS as f64
        || col < 1.0
        || col > MAX_COLS as f64
        || !(1.0..=4.0).contains(&abs)
    {
        return Err(ErrorKind::Value);
    }
    let (row, col) = (row as u32, col as u32);
    // 1: $A$1, 2: A$1, 3: $A1, 4: A1.
    let abs_row = abs == 1.0 || abs == 2.0;
    let abs_col = abs == 1.0 || abs == 3.0;
    let mut out = String::new();
    if let Some(e) = given(ctx, args, 4) {
        let sheet = ctx.text(e)?;
        if !sheet.is_empty() {
            write_sheet_name(&sheet, &mut out);
        }
        out.push('!');
    }
    if a1 {
        if abs_col {
            out.push('$');
        }
        out.push_str(&col_to_letters(col - 1));
        if abs_row {
            out.push('$');
        }
        out.push_str(&row.to_string());
    } else {
        out.push_str(&if abs_row {
            format!("R{row}")
        } else {
            format!("R[{row}]")
        });
        out.push_str(&if abs_col {
            format!("C{col}")
        } else {
            format!("C[{col}]")
        });
    }
    Ok(Value::from(out))
}

// ---- HYPERLINK, FORMULATEXT ---------------------------------------------------

fn f_hyperlink(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(match given(ctx, args, 1) {
        Some(friendly_name) => ctx.eval_scalar(friendly_name),
        None => ctx.eval_scalar(&args[0]),
    })
}

fn f_formulatext(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let reference = ctx.eval_ref(&args[0])?;
    let rect = reference.rect;
    let text = |r: u32, c: u32| match ctx.formula_at(reference.sheet, r, c) {
        Some(expr) => Value::from(format!("={}", expr.to_text(ctx.locale()))),
        None => Value::Error(ErrorKind::NA),
    };
    if reference.is_single() {
        return Ok(text(rect.r0, rect.c0));
    }
    let g = Grid::from_ref(ctx, reference).trimmed();
    let mut out = Array::new(g.rows, g.cols);
    for r in 0..g.rows {
        for c in 0..g.cols {
            out.set(r, c, text(rect.r0 + r as u32, rect.c0 + c as u32));
        }
    }
    finish(out)
}

// ---- GETPIVOTDATA -------------------------------------------------------------

/// `GETPIVOTDATA(data_field, pivot_table, [field1, item1], ...)`: the value
/// a PivotTable shows for a value field at the given items. `#REF!` when the
/// reference is not in a PivotTable, or when the report shows no such value.
fn f_getpivotdata(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    if !args.len().is_multiple_of(2) {
        return Err(ErrorKind::Ref);
    }
    let data_field = ctx.text(&args[0])?;
    let reference = ctx.eval_ref(&args[1]).map_err(|_| ErrorKind::Ref)?;
    let mut pairs = Vec::with_capacity(args.len() / 2 - 1);
    for pair in args[2..].chunks(2) {
        let field = ctx.text(&pair[0])?;
        match ctx.eval_scalar(&pair[1]) {
            Value::Error(e) => return Err(e),
            item => pairs.push((field, item)),
        }
    }
    let at = (reference.sheet, reference.rect.r0, reference.rect.c0);
    crate::pivot::get_pivot_data(ctx.workbook(), at, &data_field, &pairs)
}

// ---- dynamic arrays -----------------------------------------------------------

fn f_transpose(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let mut data = Vec::with_capacity(g.rows * g.cols);
    for c in 0..g.cols {
        for r in 0..g.rows {
            data.push(g.get(r, c).clone());
        }
    }
    finish(Array {
        rows: g.cols,
        cols: g.rows,
        data,
    })
}

fn f_filter(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let include = Grid::eval(ctx, &args[1]).trimmed();
    if let Some(e) = include.scalar_error() {
        return Err(e);
    }
    // A column of flags filters rows, a row of flags filters columns.
    let by_rows = include.full_cols == 1 && lines_up(&g, &include, true);
    let by_cols = !by_rows && include.full_rows == 1 && lines_up(&g, &include, false);
    if !by_rows && !by_cols {
        return Err(ErrorKind::Value);
    }
    let count = if by_rows { g.rows } else { g.cols };
    let mut keep = Vec::new();
    for i in 0..count {
        let flag = if by_rows {
            include.get(i, 0)
        } else {
            include.get(0, i)
        };
        let on = match flag {
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0,
            Value::Empty => false,
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        };
        if on {
            keep.push(i);
        }
    }
    if keep.is_empty() {
        return match args.get(2) {
            Some(if_empty) => Ok(ctx.eval(if_empty)),
            None => Err(ErrorKind::Calc),
        };
    }
    g.pick(&keep, by_cols)
}

/// Ordering used by `SORT` and `SORTBY`: numbers, then text, then FALSE,
/// TRUE and errors; blanks come last whatever the direction.
fn sort_cmp(a: &Value, b: &Value, descending: bool) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let o = if a.is_error() && b.is_error() {
                Ordering::Equal
            } else {
                compare_values(a, b)
            };
            if descending { o.reverse() } else { o }
        }
    }
}

/// A sort order argument: 1 ascending, -1 descending. Returns "descending".
fn sort_direction(n: f64) -> R<bool> {
    if n == 1.0 {
        Ok(false)
    } else if n == -1.0 {
        Ok(true)
    } else {
        Err(ErrorKind::Value)
    }
}

fn f_sort(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let by_col = ctx.bool_or(args, 3, false)?;
    let (lines, width) = if by_col {
        (g.cols, g.rows)
    } else {
        (g.rows, g.cols)
    };
    // `sort_index` and `sort_order` may be arrays for multi-level sorts.
    let list = |index: usize| -> Vec<Value> {
        match given(ctx, args, index) {
            Some(e) => match ctx.eval(e) {
                Value::Array(a) => a.data.clone(),
                v => vec![v],
            },
            None => vec![Value::Number(1.0)],
        }
    };
    let indexes = list(1);
    let orders = list(2);
    let mut keys: Vec<(usize, bool)> = Vec::with_capacity(indexes.len());
    for (k, index) in indexes.iter().enumerate() {
        let n = ctx.to_number(index)?.trunc();
        if n < 1.0 || n > width as f64 {
            return Err(ErrorKind::Value);
        }
        let order = if orders.len() == 1 {
            &orders[0]
        } else {
            orders.get(k).ok_or(ErrorKind::Value)?
        };
        keys.push((n as usize - 1, sort_direction(ctx.to_number(order)?)?));
    }
    let cell = |line: usize, k: usize| {
        if by_col {
            g.get(k, line)
        } else {
            g.get(line, k)
        }
    };
    let mut order: Vec<usize> = (0..lines).collect();
    order.sort_by(|&x, &y| {
        keys.iter()
            .map(|&(k, descending)| sort_cmp(cell(x, k), cell(y, k), descending))
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    });
    g.pick(&order, by_col)
}

fn f_sortby(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let mut keys: Vec<(Grid, bool)> = Vec::new();
    let mut by_col: Option<bool> = None;
    // (by_array, [sort_order]) pairs.
    for pair in args[1..].chunks(2) {
        let by = Grid::eval(ctx, &pair[0]).trimmed();
        if let Some(e) = by.scalar_error() {
            return Err(e);
        }
        let across = if by.full_cols == 1 && lines_up(&g, &by, true) {
            false
        } else if by.full_rows == 1 && lines_up(&g, &by, false) {
            true
        } else {
            return Err(ErrorKind::Value);
        };
        if *by_col.get_or_insert(across) != across {
            return Err(ErrorKind::Value);
        }
        let descending = sort_direction(ctx.num_or(pair, 1, 1.0)?)?;
        keys.push((by, descending));
    }
    let by_col = by_col.unwrap_or(false);
    let lines = if by_col { g.cols } else { g.rows };
    let mut order: Vec<usize> = (0..lines).collect();
    order.sort_by(|&x, &y| {
        keys.iter()
            .map(|(by, descending)| {
                let (a, b) = if by_col {
                    (by.get(0, x), by.get(0, y))
                } else {
                    (by.get(x, 0), by.get(y, 0))
                };
                sort_cmp(a, b, *descending)
            })
            .find(|o| *o != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    });
    g.pick(&order, by_col)
}

/// A value reduced to what `UNIQUE` compares: type plus case-folded content.
#[derive(PartialEq, Eq, Hash)]
enum Distinct {
    Blank,
    Number(u64),
    Text(String),
    Bool(bool),
    Error(ErrorKind),
}

fn distinct(v: &Value) -> Distinct {
    match v {
        Value::Empty | Value::Array(_) => Distinct::Blank,
        // +0 and -0 are the same number.
        Value::Number(n) => Distinct::Number(if *n == 0.0 { 0 } else { n.to_bits() }),
        Value::Text(s) => Distinct::Text(s.chars().flat_map(char::to_lowercase).collect()),
        Value::Bool(b) => Distinct::Bool(*b),
        Value::Error(e) => Distinct::Error(*e),
    }
}

fn f_unique(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let by_col = ctx.bool_or(args, 1, false)?;
    let exactly_once = ctx.bool_or(args, 2, false)?;
    let (lines, width) = if by_col {
        (g.cols, g.rows)
    } else {
        (g.rows, g.cols)
    };
    let cell = |line: usize, k: usize| {
        if by_col {
            g.get(k, line)
        } else {
            g.get(line, k)
        }
    };
    // First occurrence of every distinct row / column, with its count.
    let mut slots: HashMap<Vec<Distinct>, usize> = HashMap::new();
    let mut firsts: Vec<(usize, usize)> = Vec::new();
    for line in 0..lines {
        let key: Vec<Distinct> = (0..width).map(|k| distinct(cell(line, k))).collect();
        let slot = *slots.entry(key).or_insert(firsts.len());
        if slot == firsts.len() {
            firsts.push((line, 1));
        } else {
            firsts[slot].1 += 1;
        }
    }
    let keep: Vec<usize> = firsts
        .iter()
        .filter(|(_, count)| !exactly_once || *count == 1)
        .map(|(line, _)| *line)
        .collect();
    g.pick(&keep, by_col)
}

fn f_take(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    // (start, length) kept along one dimension.
    let span = |index: usize, len: usize| -> R<(usize, usize)> {
        let Some(n) = opt_num(ctx, args, index)? else {
            return Ok((0, len));
        };
        let n = n.trunc();
        if n == 0.0 {
            return Err(ErrorKind::Calc);
        }
        let k = n.abs().min(len as f64) as usize;
        Ok(if n > 0.0 { (0, k) } else { (len - k, k) })
    };
    let (r0, rows) = span(1, g.rows)?;
    let (c0, cols) = span(2, g.cols)?;
    finish(g.block(r0, rows, c0, cols))
}

fn f_drop(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let span = |index: usize, len: usize| -> R<(usize, usize)> {
        let Some(n) = opt_num(ctx, args, index)? else {
            return Ok((0, len));
        };
        let n = n.trunc();
        if n.abs() >= len as f64 {
            return Err(ErrorKind::Calc);
        }
        let k = n.abs() as usize;
        Ok(if n >= 0.0 { (k, len - k) } else { (0, len - k) })
    };
    let (r0, rows) = span(1, g.rows)?;
    let (c0, cols) = span(2, g.cols)?;
    finish(g.block(r0, rows, c0, cols))
}

fn choose_lines(ctx: &EvalCtx, args: &[Expr], columns: bool) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let len = if columns { g.cols } else { g.rows };
    let mut lines = Vec::new();
    for v in ctx.flatten(&args[1..]) {
        let n = ctx.to_number(&v)?.trunc();
        if n == 0.0 || n.abs() > len as f64 {
            return Err(ErrorKind::Value);
        }
        // Negative numbers count from the end.
        lines.push(if n > 0.0 {
            n as usize - 1
        } else {
            len - (-n) as usize
        });
    }
    g.pick(&lines, columns)
}

fn f_choosecols(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    choose_lines(ctx, args, true)
}

fn f_chooserows(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    choose_lines(ctx, args, false)
}

fn stack(ctx: &EvalCtx, args: &[Expr], vertical: bool) -> R<Value> {
    let grids: Vec<Grid> = args.iter().map(|e| Grid::eval(ctx, e).trimmed()).collect();
    let (mut rows, mut cols) = (0, 0);
    for g in &grids {
        if vertical {
            rows += g.rows;
            cols = cols.max(g.cols);
        } else {
            rows = rows.max(g.rows);
            cols += g.cols;
        }
    }
    // Shorter arrays are padded with #N/A.
    let mut out = Array {
        rows,
        cols,
        data: vec![Value::Error(ErrorKind::NA); rows * cols],
    };
    let mut at = 0;
    for g in &grids {
        for r in 0..g.rows {
            for c in 0..g.cols {
                let (row, col) = if vertical { (at + r, c) } else { (r, at + c) };
                out.set(row, col, g.get(r, c).clone());
            }
        }
        at += if vertical { g.rows } else { g.cols };
    }
    finish(out)
}

fn f_vstack(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    stack(ctx, args, true)
}

fn f_hstack(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    stack(ctx, args, false)
}

fn to_vector(ctx: &EvalCtx, args: &[Expr], as_column: bool) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    // 0 keep everything, 1 ignore blanks, 2 ignore errors, 3 ignore both.
    let ignore = ctx.num_or(args, 1, 0.0)?.trunc();
    if !(0.0..=3.0).contains(&ignore) {
        return Err(ErrorKind::Value);
    }
    let (skip_blanks, skip_errors) = (ignore == 1.0 || ignore == 3.0, ignore >= 2.0);
    let scan_by_column = ctx.bool_or(args, 2, false)?;
    let mut items = Vec::with_capacity(g.rows * g.cols);
    let mut push = |v: &Value| {
        if !(skip_blanks && v.is_empty()) && !(skip_errors && v.is_error()) {
            items.push(v.clone());
        }
    };
    if scan_by_column {
        for c in 0..g.cols {
            for r in 0..g.rows {
                push(g.get(r, c));
            }
        }
    } else {
        for r in 0..g.rows {
            for c in 0..g.cols {
                push(g.get(r, c));
            }
        }
    }
    finish(if as_column {
        Array::column(items)
    } else {
        Array::row(items)
    })
}

fn f_tocol(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    to_vector(ctx, args, true)
}

fn f_torow(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    to_vector(ctx, args, false)
}

fn wrap_vector(ctx: &EvalCtx, args: &[Expr], by_rows: bool) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    if g.rows != 1 && g.cols != 1 {
        return Err(ErrorKind::Value);
    }
    let count = ctx.num(&args[1])?.trunc();
    if count < 1.0 {
        return Err(ErrorKind::Num);
    }
    let pad = match given(ctx, args, 2) {
        Some(e) => ctx.eval_scalar(e),
        None => Value::Error(ErrorKind::NA),
    };
    let n = g.rows * g.cols;
    // A vector shorter than one line is returned as that single line.
    let count = count.min(n as f64) as usize;
    let lines = n.div_ceil(count);
    let item = |i: usize| {
        if i >= n {
            pad.clone()
        } else if g.rows == 1 {
            g.get(0, i).clone()
        } else {
            g.get(i, 0).clone()
        }
    };
    let mut out = if by_rows {
        Array::new(lines, count)
    } else {
        Array::new(count, lines)
    };
    for line in 0..lines {
        for k in 0..count {
            let v = item(line * count + k);
            if by_rows {
                out.set(line, k, v);
            } else {
                out.set(k, line, v);
            }
        }
    }
    finish(out)
}

fn f_wraprows(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    wrap_vector(ctx, args, true)
}

fn f_wrapcols(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    wrap_vector(ctx, args, false)
}

fn f_expand(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let g = Grid::data(ctx, &args[0])?;
    let target = |index: usize, have: usize, limit: u32| -> R<usize> {
        let Some(n) = opt_num(ctx, args, index)? else {
            return Ok(have);
        };
        let n = n.trunc();
        if n < have as f64 {
            Err(ErrorKind::Value)
        } else if n > limit as f64 {
            Err(ErrorKind::Num)
        } else {
            Ok(n as usize)
        }
    };
    let rows = target(1, g.rows, MAX_ROWS)?;
    let cols = target(2, g.cols, MAX_COLS)?;
    if rows.saturating_mul(cols) > MAX_CELLS {
        return Err(ErrorKind::Num);
    }
    let pad = match given(ctx, args, 3) {
        Some(e) => ctx.eval_scalar(e),
        None => Value::Error(ErrorKind::NA),
    };
    let mut out = Array {
        rows,
        cols,
        data: vec![pad; rows * cols],
    };
    for r in 0..g.rows {
        for c in 0..g.cols {
            out.set(r, c, g.get(r, c).clone());
        }
    }
    finish(out)
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("VLOOKUP", 3, 4, Kind::Raw, Category::Lookup, f_vlookup),
    FunctionDef::new("HLOOKUP", 3, 4, Kind::Raw, Category::Lookup, f_hlookup),
    FunctionDef::new("LOOKUP", 2, 3, Kind::Raw, Category::Lookup, f_lookup),
    FunctionDef::new("MATCH", 2, 3, Kind::Raw, Category::Lookup, f_match),
    FunctionDef::new("XMATCH", 2, 4, Kind::Raw, Category::Lookup, f_xmatch),
    FunctionDef::new("XLOOKUP", 3, 6, Kind::Raw, Category::Lookup, f_xlookup).with_ref(r_xlookup),
    FunctionDef::new("INDEX", 2, 4, Kind::Raw, Category::Lookup, f_index).with_ref(r_index),
    FunctionDef::new("CHOOSE", 2, VAR, Kind::Raw, Category::Lookup, f_choose).with_ref(r_choose),
    FunctionDef::new("ROW", 0, 1, Kind::Raw, Category::Lookup, f_row),
    FunctionDef::new("COLUMN", 0, 1, Kind::Raw, Category::Lookup, f_column),
    FunctionDef::new("ROWS", 1, 1, Kind::Raw, Category::Lookup, f_rows),
    FunctionDef::new("COLUMNS", 1, 1, Kind::Raw, Category::Lookup, f_columns),
    FunctionDef::new("AREAS", 1, 1, Kind::Raw, Category::Lookup, f_areas),
    FunctionDef::new("OFFSET", 3, 5, Kind::Raw, Category::Lookup, f_offset).with_ref(r_offset),
    FunctionDef::new("INDIRECT", 1, 2, Kind::Raw, Category::Lookup, f_indirect).with_ref(r_indirect),
    FunctionDef::new("ADDRESS", 2, 5, Kind::Scalar, Category::Lookup, f_address),
    FunctionDef::new("HYPERLINK", 1, 2, Kind::Scalar, Category::Lookup, f_hyperlink),
    FunctionDef::new("FORMULATEXT", 1, 1, Kind::Raw, Category::Lookup, f_formulatext),
    FunctionDef::new("GETPIVOTDATA", 2, VAR, Kind::Raw, Category::Lookup, f_getpivotdata),
    FunctionDef::new("TRANSPOSE", 1, 1, Kind::Raw, Category::Lookup, f_transpose),
    FunctionDef::new("FILTER", 2, 3, Kind::Raw, Category::Lookup, f_filter),
    FunctionDef::new("SORT", 1, 4, Kind::Raw, Category::Lookup, f_sort),
    FunctionDef::new("SORTBY", 2, VAR, Kind::Raw, Category::Lookup, f_sortby),
    FunctionDef::new("UNIQUE", 1, 3, Kind::Raw, Category::Lookup, f_unique),
    FunctionDef::new("TAKE", 2, 3, Kind::Raw, Category::Lookup, f_take),
    FunctionDef::new("DROP", 2, 3, Kind::Raw, Category::Lookup, f_drop),
    FunctionDef::new("CHOOSECOLS", 2, VAR, Kind::Raw, Category::Lookup, f_choosecols),
    FunctionDef::new("CHOOSEROWS", 2, VAR, Kind::Raw, Category::Lookup, f_chooserows),
    FunctionDef::new("VSTACK", 1, VAR, Kind::Raw, Category::Lookup, f_vstack),
    FunctionDef::new("HSTACK", 1, VAR, Kind::Raw, Category::Lookup, f_hstack),
    FunctionDef::new("TOCOL", 1, 3, Kind::Raw, Category::Lookup, f_tocol),
    FunctionDef::new("TOROW", 1, 3, Kind::Raw, Category::Lookup, f_torow),
    FunctionDef::new("WRAPROWS", 2, 3, Kind::Raw, Category::Lookup, f_wraprows),
    FunctionDef::new("WRAPCOLS", 2, 3, Kind::Raw, Category::Lookup, f_wrapcols),
    FunctionDef::new("EXPAND", 2, 4, Kind::Raw, Category::Lookup, f_expand),
];

#[cfg(test)]
mod tests {
    use crate::locale::Locale;
    use crate::tests::{eval, val, wb};
    use crate::value::{ErrorKind, Value};
    use crate::workbook::Workbook;

    /// Where the formula under test lives: clear of the data in A..F.
    const AT: &str = "AA1";

    const FRUIT: &[(&str, &str)] = &[
        ("A1", "Fruit"),
        ("B1", "Price"),
        ("C1", "Qty"),
        ("A2", "Apple"),
        ("B2", "1.2"),
        ("C2", "10"),
        ("A3", "Banana"),
        ("B3", "0.5"),
        ("C3", "20"),
        ("A4", "Cherry"),
        ("B4", "3"),
        ("C4", "30"),
        ("A5", "Damson"),
        ("B5", "2.5"),
        ("C5", "40"),
    ];

    const NUMS: &[(&str, &str)] = &[
        ("A1", "10"),
        ("A2", "20"),
        ("A3", "30"),
        ("A4", "40"),
        ("A5", "50"),
    ];

    fn show(v: &Value) -> String {
        match v {
            Value::Empty => "0".to_string(),
            Value::Number(n) => format!("{n}"),
            Value::Text(s) => s.to_string(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Error(e) => e.as_str().to_string(),
            Value::Array(_) => "{...}".to_string(),
        }
    }

    /// Result of `formula` entered in `AT`, with any spilled array written as
    /// rows separated by `;` and columns by `,` (blank elements as `_`).
    fn shown(w: &Workbook) -> String {
        match w.sheets[0].spill_at(0, 26) {
            Some(sp) => (0..sp.array.rows)
                .map(|r| {
                    sp.array
                        .row_slice(r)
                        .iter()
                        .map(show)
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect::<Vec<_>>()
                .join(";"),
            None => show(&val(w, AT)),
        }
    }

    fn calc(cells: &[(&str, &str)], formula: &str) -> String {
        let text = format!("={formula}");
        let mut all = cells.to_vec();
        all.push((AT, text.as_str()));
        shown(&wb(&all))
    }

    fn f(formula: &str) -> String {
        calc(&[], formula)
    }

    #[test]
    fn pattern_agrees_with_wildcard_match() {
        let patterns = [
            "", "*", "?", "a", "a*", "*a", "*a*", "a?c", "a*c", "~*x", "~?", "a~", "~~", "*~**",
            "??", "*?", "é*", "*É", "İ?", "a**b", "*apple*", "b*n*a", "Straße", "*ß?",
        ];
        let texts = [
            "",
            "a",
            "A",
            "abc",
            "ABBBC",
            "abbc",
            "*x",
            "ax",
            "?",
            "a~",
            "~",
            "**",
            "été",
            "É",
            "café",
            "İx",
            "i\u{307}x",
            "Green Apple pie",
            "banana",
            "BANDANA",
            "straße",
            "ab",
        ];
        for pattern in patterns {
            let compiled = super::Pattern::new(pattern);
            for text in texts {
                let expected = crate::functions::wildcard_match(pattern, text);
                assert_eq!(
                    compiled.matches(text),
                    expected,
                    "{pattern:?} against {text:?}"
                );
            }
        }
    }

    #[test]
    fn vlookup() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(v("VLOOKUP(\"banana\",A2:C5,2,FALSE)"), "0.5");
        assert_eq!(v("VLOOKUP(\"Cherry\",A:C,3,0)"), "30");
        // Wildcards in exact mode.
        assert_eq!(v("VLOOKUP(\"ch*\",A2:C5,3,FALSE)"), "30");
        assert_eq!(v("VLOOKUP(\"?amson\",A2:C5,2,0)"), "2.5");
        assert_eq!(f("VLOOKUP(\"a~*b\",{\"axb\",1;\"a*b\",2},2,0)"), "2");
        assert_eq!(f("VLOOKUP(\"*É\",{\"abc\",1;\"été\",2},2,0)"), "2");
        assert_eq!(f("VLOOKUP(\"ÉTÉ\",{\"abc\",1;\"été\",2},2,0)"), "2");
        // Errors.
        assert_eq!(v("VLOOKUP(\"Fig\",A2:C5,2,FALSE)"), "#N/A");
        assert_eq!(v("VLOOKUP(\"Apple\",A2:C5,4,FALSE)"), "#REF!");
        assert_eq!(v("VLOOKUP(\"Fig\",A2:C5,4,FALSE)"), "#REF!");
        assert_eq!(v("VLOOKUP(\"Apple\",A2:C5,0,FALSE)"), "#VALUE!");
        assert_eq!(v("VLOOKUP(\"Apple\",A2:C5,\"x\",FALSE)"), "#VALUE!");
        assert_eq!(v("VLOOKUP(1/0,A2:C5,2,FALSE)"), "#DIV/0!");
        // Approximate match on sorted text.
        assert_eq!(v("VLOOKUP(\"Blueberry\",A2:C5,2)"), "0.5");
        assert_eq!(v("VLOOKUP(\"Zebra\",A2:C5,3,TRUE)"), "40");
        assert_eq!(v("VLOOKUP(\"Aardvark\",A2:C5,2)"), "#N/A");
        // An omitted-but-present range_lookup means exact.
        assert_eq!(v("VLOOKUP(\"Apple\",A2:C5,2,)"), "1.2");
        assert_eq!(v("VLOOKUP(\"App\",A2:C5,2,)"), "#N/A");
        // Approximate match on numbers.
        let grades = "{0,\"F\";60,\"D\";70,\"C\";80,\"B\";90,\"A\"}";
        assert_eq!(f(&format!("VLOOKUP(85,{grades},2)")), "B");
        assert_eq!(f(&format!("VLOOKUP(59.9,{grades},2,TRUE)")), "F");
        assert_eq!(f(&format!("VLOOKUP(90,{grades},2)")), "A");
        assert_eq!(f(&format!("VLOOKUP(1000,{grades},2)")), "A");
        assert_eq!(f(&format!("VLOOKUP(-1,{grades},2)")), "#N/A");
        // Types never mix: the text "1" is not the number 1.
        assert_eq!(f("VLOOKUP(\"1\",{1,\"a\"},2,0)"), "#N/A");
        assert_eq!(f("VLOOKUP(1,{\"1\",\"a\";1,\"b\"},2,0)"), "b");
        assert_eq!(f("VLOOKUP(TRUE,{1,\"a\";TRUE,\"b\"},2,0)"), "b");
        // Arrays of lookup values / column numbers spill.
        assert_eq!(
            v("VLOOKUP({\"Cherry\";\"Apple\";\"Fig\"},A2:C5,2,0)"),
            "3;1.2;#N/A"
        );
        assert_eq!(v("VLOOKUP(\"Cherry\",A2:C5,{2,3},0)"), "3,30");
        // A blank found in the table reads as 0.
        assert_eq!(
            calc(&[("A1", "k"), ("C1", "x")], "VLOOKUP(\"k\",A1:C1,2,0)"),
            "0"
        );
    }

    #[test]
    fn hlookup() {
        assert_eq!(f("HLOOKUP(\"b\",{\"a\",\"b\",\"c\";1,2,3},2,FALSE)"), "2");
        assert_eq!(
            f("HLOOKUP(3,{1,2,3;\"a\",\"b\",\"c\";\"d\",\"e\",\"f\"},2,TRUE)"),
            "c"
        );
        assert_eq!(f("HLOOKUP(2.5,{1,2,3;\"a\",\"b\",\"c\"},2)"), "b");
        assert_eq!(f("HLOOKUP(\"B*\",{\"a\",\"b\",\"c\";1,2,3},2,0)"), "2");
        assert_eq!(f("HLOOKUP(\"b\",{\"a\",\"b\",\"c\";1,2,3},3,0)"), "#REF!");
        assert_eq!(f("HLOOKUP(\"b\",{\"a\",\"b\",\"c\";1,2,3},0,0)"), "#VALUE!");
        assert_eq!(f("HLOOKUP(\"z\",{\"a\",\"b\",\"c\";1,2,3},2,0)"), "#N/A");
        assert_eq!(f("HLOOKUP(0,{1,2,3;4,5,6},2)"), "#N/A");
        assert_eq!(calc(FRUIT, "HLOOKUP(\"Qty\",A1:C5,3,FALSE)"), "20");
        assert_eq!(calc(FRUIT, "HLOOKUP(\"price\",A1:C5,5,FALSE)"), "2.5");
    }

    #[test]
    fn lookup() {
        let cells = &[
            ("A2", "4.14"),
            ("B2", "red"),
            ("A3", "4.19"),
            ("B3", "orange"),
            ("A4", "5.17"),
            ("B4", "yellow"),
            ("A5", "5.77"),
            ("B5", "green"),
            ("A6", "6.39"),
            ("B6", "blue"),
        ];
        let v = |formula: &str| calc(cells, formula);
        // Vector form.
        assert_eq!(v("LOOKUP(4.19,A2:A6,B2:B6)"), "orange");
        assert_eq!(v("LOOKUP(5.75,A2:A6,B2:B6)"), "yellow");
        assert_eq!(v("LOOKUP(7.66,A2:A6,B2:B6)"), "blue");
        assert_eq!(v("LOOKUP(0,A2:A6,B2:B6)"), "#N/A");
        assert_eq!(v("LOOKUP(5,A2:A6)"), "4.19");
        // The result vector may run the other way, or be too short.
        assert_eq!(f("LOOKUP(2,{1,2,3},{\"x\";\"y\";\"z\"})"), "y");
        assert_eq!(v("LOOKUP(5.17,A2:A6,B2)"), "yellow");
        assert_eq!(f("LOOKUP(3,{1,2,3},{\"x\",\"y\"})"), "#N/A");
        // Array form: search the first row / column, answer from the last.
        assert_eq!(f("LOOKUP(\"C\",{\"a\",\"b\",\"c\",\"d\";1,2,3,4})"), "3");
        assert_eq!(f("LOOKUP(\"bump\",{\"a\",1;\"b\",2;\"c\",3})"), "2");
        // Errors are stepped over: the classic "last match" idiom.
        assert_eq!(v("LOOKUP(2,1/(A2:A6>5),B2:B6)"), "blue");
        assert_eq!(v("LOOKUP(2,1/(A2:A6<5),B2:B6)"), "orange");
        assert_eq!(v("LOOKUP(1/0,A2:A6,B2:B6)"), "#DIV/0!");
    }

    #[test]
    fn match_() {
        let cells = &[("A2", "25"), ("A3", "38"), ("A4", "40"), ("A5", "41")];
        let v = |formula: &str| calc(cells, formula);
        assert_eq!(v("MATCH(39,A2:A5,1)"), "2");
        assert_eq!(v("MATCH(41,A2:A5,0)"), "4");
        assert_eq!(v("MATCH(40,A2:A5,-1)"), "#N/A");
        assert_eq!(v("MATCH(40,A2:A5)"), "3");
        assert_eq!(v("MATCH(24,A2:A5)"), "#N/A");
        assert_eq!(v("MATCH(1000,A:A)"), "5");
        // Exact, with wildcards, case-insensitive.
        assert_eq!(f("MATCH(\"b*\",{\"apple\",\"banana\",\"cherry\"},0)"), "2");
        assert_eq!(
            f("MATCH(\"BANANA\",{\"apple\";\"banana\";\"cherry\"},0)"),
            "2"
        );
        assert_eq!(
            f("MATCH(\"*rr?\",{\"apple\",\"banana\",\"cherry\"},0)"),
            "3"
        );
        assert_eq!(f("MATCH(2,{1,2,2,2,3},0)"), "2");
        assert_eq!(f("MATCH(\"x\",{1,2,3},0)"), "#N/A");
        assert_eq!(f("MATCH(2,{1,2,3},)"), "2");
        // Ascending: the last value <= the key.
        assert_eq!(f("MATCH(2,{1,2,2,2,3})"), "4");
        assert_eq!(f("MATCH(0,{1,2,3})"), "#N/A");
        assert_eq!(f("MATCH(3,{1,\"a\",3,\"b\",5})"), "3");
        assert_eq!(f("MATCH(\"b\",{1,2,\"a\",\"c\"})"), "3");
        // Descending: the smallest value >= the key.
        assert_eq!(f("MATCH(5,{9,7,5,3},-1)"), "3");
        assert_eq!(f("MATCH(6,{9,7,5,3},-1)"), "2");
        assert_eq!(f("MATCH(10,{9,7,5,3},-1)"), "#N/A");
        // Shape and argument errors.
        assert_eq!(f("MATCH(1,{1,2;3,4},0)"), "#N/A");
        assert_eq!(f("MATCH(1,{1,2,3},\"x\")"), "#VALUE!");
        assert_eq!(f("MATCH(#REF!,{1,2,3},0)"), "#REF!");
        // Several lookup values.
        assert_eq!(f("MATCH({3,1,7},{1,2,3},0)"), "3,1,#N/A");
        // Blank cells never match; a blank lookup value counts as 0.
        assert_eq!(
            calc(&[("A1", "1"), ("A3", "3")], "MATCH(0,A1:A3,0)"),
            "#N/A"
        );
        assert_eq!(f("MATCH(C1,{5,0,7},0)"), "2");
        // INDEX / MATCH.
        assert_eq!(calc(FRUIT, "INDEX(B2:B5,MATCH(\"Cherry\",A2:A5,0))"), "3");
    }

    #[test]
    fn xmatch() {
        let fruit = "{\"Apple\",\"Grape\",\"Pear\",\"Banana\",\"Cherry\"}";
        assert_eq!(f(&format!("XMATCH(\"Gra\",{fruit},1)")), "2");
        assert_eq!(f(&format!("XMATCH(\"Gra\",{fruit})")), "#N/A");
        assert_eq!(f(&format!("XMATCH(\"pear\",{fruit})")), "3");
        assert_eq!(f("XMATCH(4,{5,4,3,2,1})"), "2");
        // Exact or next larger / smaller, on unsorted data.
        assert_eq!(f("XMATCH(4.5,{5,4,3,2,1},1)"), "1");
        assert_eq!(f("XMATCH(4.5,{5,4,3,2,1},-1)"), "2");
        assert_eq!(f("XMATCH(25,{40,10,30,20},-1)"), "4");
        assert_eq!(f("XMATCH(25,{40,10,30,20},1)"), "3");
        assert_eq!(f("XMATCH(5,{40,10,30,20},-1)"), "#N/A");
        assert_eq!(f("XMATCH(50,{40,10,30,20},1)"), "#N/A");
        // Reverse search.
        assert_eq!(f("XMATCH(\"a\",{\"a\",\"b\",\"a\"},0,1)"), "1");
        assert_eq!(f("XMATCH(\"a\",{\"a\",\"b\",\"a\"},0,-1)"), "3");
        assert_eq!(f("XMATCH(2,{1,2,2,3},0,-1)"), "3");
        // Wildcards only in match mode 2.
        assert_eq!(f("XMATCH(\"b*\",{\"apple\",\"banana\"},2)"), "2");
        assert_eq!(f("XMATCH(\"b*\",{\"apple\",\"banana\"},0)"), "#N/A");
        assert_eq!(
            f("XMATCH(\"?pp*\",{\"banana\",\"apple\",\"apply\"},2,-1)"),
            "3"
        );
        // Binary searches.
        assert_eq!(f("XMATCH(3,{1,2,3,4,5},0,2)"), "3");
        assert_eq!(f("XMATCH(3.5,{1,2,3,4,5},0,2)"), "#N/A");
        assert_eq!(f("XMATCH(3.5,{1,2,3,4,5},-1,2)"), "3");
        assert_eq!(f("XMATCH(3.5,{1,2,3,4,5},1,2)"), "4");
        assert_eq!(f("XMATCH(3,{5,4,3,2,1},0,-2)"), "3");
        assert_eq!(f("XMATCH(3.5,{5,4,3,2,1},1,-2)"), "2");
        assert_eq!(f("XMATCH(3.5,{5,4,3,2,1},-1,-2)"), "3");
        assert_eq!(f("XMATCH(0,{1,2,3},-1,2)"), "#N/A");
        assert_eq!(f("XMATCH(9,{1,2,3},1,2)"), "#N/A");
        // Bad modes and shapes.
        assert_eq!(f("XMATCH(1,{1,2},3)"), "#VALUE!");
        assert_eq!(f("XMATCH(1,{1,2},0,0)"), "#VALUE!");
        assert_eq!(f("XMATCH(\"a*\",{\"a\",\"b\"},2,2)"), "#VALUE!");
        assert_eq!(f("XMATCH(1,{1,2;3,4})"), "#VALUE!");
        assert_eq!(f("XMATCH({2;9},{1,2,3})"), "2;#N/A");
    }

    #[test]
    fn xlookup() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(v("XLOOKUP(\"Cherry\",A2:A5,B2:B5)"), "3");
        assert_eq!(v("XLOOKUP(\"cherry\",A:A,C:C)"), "30");
        assert_eq!(v("XLOOKUP(\"Fig\",A2:A5,B2:B5)"), "#N/A");
        assert_eq!(v("XLOOKUP(\"Fig\",A2:A5,B2:B5,\"none\")"), "none");
        // if_not_found is lazy.
        assert_eq!(v("XLOOKUP(\"Apple\",A2:A5,B2:B5,1/0)"), "1.2");
        assert_eq!(v("XLOOKUP(\"Fig\",A2:A5,B2:B5,1/0)"), "#DIV/0!");
        // A 2-D return array gives the whole row / column.
        assert_eq!(v("XLOOKUP(\"Banana\",A2:A5,B2:C5)"), "0.5,20");
        assert_eq!(v("XLOOKUP(\"Qty\",A1:C1,A2:C5)"), "10;20;30;40");
        assert_eq!(v("XLOOKUP(\"Price\",A1:C1,A2:C2)"), "1.2");
        // Match modes.
        assert_eq!(v("XLOOKUP(2.7,B2:B5,A2:A5,,-1)"), "Damson");
        assert_eq!(v("XLOOKUP(2.7,B2:B5,A2:A5,,1)"), "Cherry");
        assert_eq!(v("XLOOKUP(2.7,B2:B5,A2:A5)"), "#N/A");
        assert_eq!(v("XLOOKUP(\"*an*\",A2:A5,C2:C5,,2)"), "20");
        assert_eq!(v("XLOOKUP(\"*an*\",A2:A5,C2:C5)"), "#N/A");
        // Search modes.
        assert_eq!(f("XLOOKUP(\"a\",{\"a\";\"b\";\"a\"},{1;2;3})"), "1");
        assert_eq!(f("XLOOKUP(\"a\",{\"a\";\"b\";\"a\"},{1;2;3},,0,-1)"), "3");
        assert_eq!(
            f("XLOOKUP(3,{1,2,3,4,5},{\"a\",\"b\",\"c\",\"d\",\"e\"},,0,2)"),
            "c"
        );
        assert_eq!(
            f("XLOOKUP(3.5,{5,4,3,2,1},{\"a\",\"b\",\"c\",\"d\",\"e\"},,1,-2)"),
            "b"
        );
        assert_eq!(v("XLOOKUP(\"Cherry\",A2:A5,C2:C5,,0,2)"), "30");
        // Errors.
        assert_eq!(v("XLOOKUP(\"a\",A2:A5,B2:B4)"), "#VALUE!");
        assert_eq!(v("XLOOKUP(\"a\",A2:B5,C2:C5)"), "#VALUE!");
        assert_eq!(v("XLOOKUP(\"Apple\",A2:A5,B2:B5,,7)"), "#VALUE!");
        assert_eq!(v("XLOOKUP(#NAME?,A2:A5,B2:B5)"), "#NAME?");
        // Several lookup values.
        assert_eq!(
            v("XLOOKUP({\"Cherry\";\"Fig\";\"Apple\"},A2:A5,B2:B5,\"?\")"),
            "3;?;1.2"
        );
        // Two-way lookup: the inner call returns a column of the table.
        assert_eq!(
            v("XLOOKUP(\"Cherry\",A2:A5,XLOOKUP(\"Qty\",A1:C1,A2:C5))"),
            "30"
        );
        assert_eq!(
            v("XLOOKUP(\"Price\",A1:C1,XLOOKUP(\"Banana\",A2:A5,A2:C5))"),
            "0.5"
        );
        // Used as a reference.
        assert_eq!(v("ROW(XLOOKUP(\"Cherry\",A2:A5,B2:B5))"), "4");
        assert_eq!(v("SUM(XLOOKUP(\"Banana\",A2:A5,B2:C5))"), "20.5");
        assert_eq!(v("COLUMNS(XLOOKUP(\"Banana\",A2:A5,B2:C5))"), "2");
        // Lookup and return ranges only need the same size.
        let split = &[
            ("A1", "x"),
            ("A2", "y"),
            ("A3", "z"),
            ("B4", "1"),
            ("B5", "2"),
            ("B6", "3"),
        ];
        assert_eq!(calc(split, "XLOOKUP(\"y\",A1:A3,B4:B6)"), "2");
        // A blank lookup value finds the first blank cell.
        let gap = &[
            ("A1", "x"),
            ("A3", "z"),
            ("B1", "1"),
            ("B2", "2"),
            ("B3", "3"),
        ];
        assert_eq!(calc(gap, "XLOOKUP(D1,A1:A3,B1:B3)"), "2");
        assert_eq!(calc(gap, "XMATCH(\"\",A1:A9)"), "2");
        assert_eq!(calc(gap, "XMATCH(\"\",A1:A9,0,-1)"), "9");
        assert_eq!(calc(FRUIT, "XLOOKUP(\"\",A1:A9,B1:B9,\"none\")"), "0");
    }

    #[test]
    fn index() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(v("INDEX(A2:C5,2,3)"), "20");
        assert_eq!(v("INDEX(A:C,3,1)"), "Banana");
        assert_eq!(v("INDEX(A2:C5,5,1)"), "#REF!");
        assert_eq!(v("INDEX(A2:C5,1,4)"), "#REF!");
        assert_eq!(v("INDEX(A2:C5,-1,1)"), "#VALUE!");
        assert_eq!(v("INDEX(A2:C5,1,1,2)"), "#REF!");
        assert_eq!(v("INDEX(A2:C5,1,1,1)"), "Apple");
        // 0 or an omitted number selects the whole column / row.
        assert_eq!(v("INDEX(A2:C5,0,2)"), "1.2;0.5;3;2.5");
        assert_eq!(v("INDEX(A2:C5,,3)"), "10;20;30;40");
        assert_eq!(v("INDEX(A2:C5,3,0)"), "Cherry,3,30");
        assert_eq!(v("INDEX(A2:C5,3)"), "Cherry,3,30");
        assert_eq!(v("SUM(INDEX(B2:C5,0,2))"), "100");
        assert_eq!(v("ROWS(INDEX(A1:C5,0,1))"), "5");
        // A lone index runs along a single row or column.
        assert_eq!(f("INDEX({1,2,3},2)"), "2");
        assert_eq!(f("INDEX({1;2;3},3)"), "3");
        assert_eq!(f("INDEX({1,2,3},4)"), "#REF!");
        assert_eq!(v("INDEX(A1:C1,2)"), "Price");
        // Array constants.
        assert_eq!(f("INDEX({1,2;3,4},2,1)"), "3");
        assert_eq!(f("INDEX({1,2;3,4},0,2)"), "2;4");
        assert_eq!(f("INDEX({1,2;3,4},2,0)"), "3,4");
        assert_eq!(f("INDEX({1,2;3,4},3,1)"), "#REF!");
        assert_eq!(f("INDEX(#N/A,1,1)"), "#N/A");
        // Arrays of positions.
        assert_eq!(v("INDEX(A2:C5,{1;3},1)"), "Apple;Cherry");
        assert_eq!(v("INDEX(A2:C5,{4;1},{1,3})"), "Damson,40;Apple,10");
        // Cells past the used part of the sheet are blank, not an error.
        assert_eq!(v("INDEX(A1:A10,8)"), "0");
        assert_eq!(v("INDEX(A1:A10,11)"), "#REF!");
        // As a reference.
        assert_eq!(v("ROW(INDEX(A1:B5,2,1))"), "2");
        assert_eq!(v("COLUMN(INDEX(A1:B5,2,2))"), "2");
        assert_eq!(v("SUM(OFFSET(INDEX(B1:B5,2),0,0,2,1))"), "1.7");
        assert_eq!(v("COLUMNS(INDEX(A1:C5,2,0))"), "3");
    }

    #[test]
    fn choose() {
        assert_eq!(f("CHOOSE(2,\"a\",\"b\",\"c\")"), "b");
        assert_eq!(f("CHOOSE(2.9,\"a\",\"b\",\"c\")"), "b");
        assert_eq!(f("CHOOSE(4,\"a\",\"b\",\"c\")"), "#VALUE!");
        assert_eq!(f("CHOOSE(0,\"a\")"), "#VALUE!");
        assert_eq!(f("CHOOSE(\"x\",1,2)"), "#VALUE!");
        assert_eq!(f("CHOOSE(#N/A,1,2)"), "#N/A");
        // Only the chosen value is evaluated.
        assert_eq!(f("CHOOSE(1,5,1/0)"), "5");
        assert_eq!(f("CHOOSE(2,5,1/0)"), "#DIV/0!");
        // Ranges and arrays.
        assert_eq!(calc(FRUIT, "SUM(CHOOSE(2,A1:A3,B2:B5))"), "7.2");
        assert_eq!(calc(FRUIT, "ROWS(CHOOSE(2,A1:A3,B2:B5))"), "4");
        assert_eq!(calc(FRUIT, "ROW(CHOOSE(1,C3,B2:B5))"), "3");
        assert_eq!(f("CHOOSE({1,2,3},\"a\",\"b\",\"c\")"), "a,b,c");
        assert_eq!(
            calc(FRUIT, "CHOOSE({1,2},C2:C3,A2:A3)"),
            "10,Apple;20,Banana"
        );
    }

    #[test]
    fn row_and_column() {
        assert_eq!(eval("ROW()"), Value::Number(100.0));
        assert_eq!(eval("COLUMN()"), Value::Number(26.0));
        assert_eq!(f("ROW(C7)"), "7");
        assert_eq!(f("COLUMN(C7)"), "3");
        assert_eq!(f("ROW(B2:B4)"), "2;3;4");
        assert_eq!(f("COLUMN(B2:D2)"), "2,3,4");
        assert_eq!(f("ROW(B2:D2)"), "2");
        assert_eq!(f("COLUMN(B2:B4)"), "2");
        assert_eq!(f("COLUMN(XFD1)"), "16384");
        assert_eq!(f("ROW(5)"), "#VALUE!");
        assert_eq!(f("ROW(Sheet9!A1)"), "#REF!");
        // The row numbers of a range do not depend on what the sheet holds.
        assert_eq!(f("SUM(ROW(1:10))"), "55");
        assert_eq!(f("SUM(ROW(A1:A100))"), "5050");
        assert_eq!(f("SUM(COLUMN(A:D))"), "10");
        // ... except for ranges running to the edge of the sheet.
        assert_eq!(calc(NUMS, "SUM(ROW(A:A))"), "15");
        assert_eq!(calc(NUMS, "SUM((A:A>25)*ROW(A:A))"), "12");
    }

    #[test]
    fn rows_and_columns() {
        assert_eq!(f("ROWS(A:A)"), "1048576");
        assert_eq!(f("COLUMNS(A:A)"), "1");
        assert_eq!(f("COLUMNS(1:1)"), "16384");
        assert_eq!(f("ROWS(2:4)"), "3");
        assert_eq!(f("ROWS(A1:A10)"), "10");
        assert_eq!(f("COLUMNS(B2:D9)"), "3");
        assert_eq!(f("ROWS(B2:D9)"), "8");
        assert_eq!(f("ROWS({1,2;3,4;5,6})"), "3");
        assert_eq!(f("COLUMNS({1,2;3,4;5,6})"), "2");
        assert_eq!(f("ROWS(5)"), "1");
        assert_eq!(f("COLUMNS(\"abc\")"), "1");
        assert_eq!(f("ROWS(#REF!)"), "#REF!");
        assert_eq!(calc(NUMS, "ROWS(A1:A5*2)"), "5");
        assert_eq!(f("ROWS(OFFSET(A1,0,0,7,2))"), "7");
        assert_eq!(f("AREAS(A1:B2)"), "1");
        assert_eq!(f("AREAS(OFFSET(A1,1,1))"), "1");
        assert_eq!(f("AREAS(5)"), "#VALUE!");
    }

    #[test]
    fn offset() {
        let v = |formula: &str| calc(NUMS, formula);
        assert_eq!(v("SUM(OFFSET(A1,1,0,3,1))"), "90");
        assert_eq!(v("ROW(OFFSET(A1,2,0))"), "3");
        assert_eq!(v("COLUMN(OFFSET(A1,0,2))"), "3");
        assert_eq!(v("OFFSET(A1,2,0)"), "30");
        assert_eq!(v("OFFSET(A1,1,0,2,1)"), "20;30");
        assert_eq!(v("OFFSET(A1,0,5)"), "0");
        // Height and width default to those of the reference.
        assert_eq!(v("SUM(OFFSET(A1:A2,1,0))"), "50");
        assert_eq!(v("SUM(OFFSET(A1:A2,3,0,,))"), "90");
        assert_eq!(v("ROWS(OFFSET(A1,0,0,5,2))"), "5");
        assert_eq!(v("COLUMNS(OFFSET(A1,0,0,5,2))"), "2");
        // Negative sizes extend up / left.
        assert_eq!(v("SUM(OFFSET(A5,0,0,-3,1))"), "120");
        assert_eq!(v("ROW(OFFSET(A5,0,0,-3,1))"), "3;4;5");
        assert_eq!(v("COLUMNS(OFFSET(C1,0,0,1,-3))"), "3");
        // Fractions are truncated.
        assert_eq!(v("SUM(OFFSET(A1,1.9,0,2.9,1))"), "50");
        // Errors.
        assert_eq!(v("OFFSET(A1,-1,0)"), "#REF!");
        assert_eq!(v("OFFSET(A1,0,-1)"), "#REF!");
        assert_eq!(v("OFFSET(A1,0,0,0,1)"), "#REF!");
        assert_eq!(v("OFFSET(A1,0,0,1,0)"), "#REF!");
        assert_eq!(v("OFFSET(A1,1048576,0)"), "#REF!");
        assert_eq!(v("OFFSET(A1,0,0,-2,1)"), "#REF!");
        assert_eq!(v("OFFSET(5,1,1)"), "#VALUE!");
        assert_eq!(v("OFFSET(A1,\"x\",0)"), "#VALUE!");
        // Nested.
        assert_eq!(v("SUM(OFFSET(OFFSET(A1,1,0),1,0,2,1))"), "70");
        assert_eq!(v("AVERAGE(OFFSET(A1,COUNT(A:A)-2,0,2,1))"), "45");
    }

    #[test]
    fn indirect() {
        let v = |formula: &str| calc(NUMS, formula);
        assert_eq!(v("SUM(INDIRECT(\"A1:A3\"))"), "60");
        assert_eq!(v("INDIRECT(\"A\"&2)"), "20");
        assert_eq!(v("INDIRECT(\"$A$3\")"), "30");
        assert_eq!(v("INDIRECT(\" a4 \")"), "40");
        assert_eq!(v("INDIRECT(\"B1\")"), "0");
        assert_eq!(v("INDIRECT(\"A2:A3\")"), "20;30");
        assert_eq!(v("SUM(INDIRECT(\"A:A\"))"), "150");
        assert_eq!(v("SUM(INDIRECT(\"2:3\"))"), "50");
        assert_eq!(v("ROW(INDIRECT(\"A5\"))"), "5");
        assert_eq!(v("ROWS(INDIRECT(\"A1:B7\"))"), "7");
        assert_eq!(v("SUM(ROW(INDIRECT(\"1:\"&4)))"), "10");
        assert_eq!(v("SUM(OFFSET(INDIRECT(\"A1\"),1,0,2,1))"), "50");
        // Bad text.
        assert_eq!(v("INDIRECT(\"nope!!\")"), "#REF!");
        assert_eq!(v("INDIRECT(\"\")"), "#REF!");
        assert_eq!(v("INDIRECT(\"1+1\")"), "#REF!");
        assert_eq!(v("INDIRECT(\"Nowhere!A1\")"), "#REF!");
        assert_eq!(v("INDIRECT(#N/A)"), "#N/A");
        // R1C1 notation, absolute and relative to the formula in AA1.
        assert_eq!(v("INDIRECT(\"R2C1\",FALSE)"), "20");
        assert_eq!(v("SUM(INDIRECT(\"R1C1:R3C1\",FALSE))"), "60");
        assert_eq!(v("INDIRECT(\"R[1]C[-26]\",FALSE)"), "20");
        assert_eq!(v("INDIRECT(\"R[2]C[-26]\",)"), "30");
        assert_eq!(v("INDIRECT(\"r3c[-26]\",FALSE)"), "30");
        assert_eq!(v("SUM(INDIRECT(\"C1\",FALSE))"), "150");
        assert_eq!(v("SUM(INDIRECT(\"R4\",FALSE))"), "40");
        assert_eq!(v("COLUMN(INDIRECT(\"RC\",FALSE))"), "27");
        assert_eq!(v("INDIRECT(\"R[-1]C\",FALSE)"), "#REF!");
        assert_eq!(v("INDIRECT(\"A1\",FALSE)"), "#REF!");
        assert_eq!(v("INDIRECT(\"R0C1\",FALSE)"), "#REF!");
    }

    #[test]
    fn indirect_sheets_and_names() {
        let mut w = wb(NUMS);
        let s = w.add_sheet(1);
        w.set_cell_text(s, 0, 0, "7").unwrap();
        w.set_cell_text(s, 1, 0, "8").unwrap();
        assert!(w.rename_sheet(s, "My Data"));
        let range = crate::parser::parse_formula("Sheet1!$A$1:$A$3", Locale::en()).unwrap();
        w.define_name("TopRows", range);
        let formulas = [
            "=INDIRECT(\"'My Data'!A1\")",
            "=SUM(INDIRECT(\"'My Data'!A1:A2\"))",
            "=INDIRECT(\"'My Data'!R2C1\",FALSE)",
            "=SUM(INDIRECT(\"TopRows\"))",
            "=ROWS(INDIRECT(\"toprows\"))",
            "=SUM(INDIRECT(\"TopRows\",FALSE))",
            "=SUM(INDIRECT(\"Sheet1!A4:A5\"))",
        ];
        for (i, formula) in formulas.iter().enumerate() {
            w.set_cell_text(0, i as u32, 3, formula).unwrap();
        }
        w.commit("formulas");
        let got: Vec<String> = (1..=formulas.len())
            .map(|r| show(&val(&w, &format!("D{r}"))))
            .collect();
        assert_eq!(got, ["7", "15", "8", "60", "3", "60", "90"]);
    }

    #[test]
    fn address() {
        assert_eq!(f("ADDRESS(2,3)"), "$C$2");
        assert_eq!(f("ADDRESS(2,3,2)"), "C$2");
        assert_eq!(f("ADDRESS(2,3,3)"), "$C2");
        assert_eq!(f("ADDRESS(2,3,4)"), "C2");
        assert_eq!(f("ADDRESS(2,3,2,FALSE)"), "R2C[3]");
        assert_eq!(f("ADDRESS(2,3,3,FALSE)"), "R[2]C3");
        assert_eq!(f("ADDRESS(2,3,4,FALSE)"), "R[2]C[3]");
        assert_eq!(
            f("ADDRESS(2,3,1,FALSE,\"[Book1]Sheet1\")"),
            "'[Book1]Sheet1'!R2C3"
        );
        assert_eq!(
            f("ADDRESS(2,3,1,FALSE,\"EXCEL SHEET\")"),
            "'EXCEL SHEET'!R2C3"
        );
        assert_eq!(f("ADDRESS(1,1,,,\"Sheet2\")"), "Sheet2!$A$1");
        assert_eq!(f("ADDRESS(1048576,16384,4)"), "XFD1048576");
        assert_eq!(f("ADDRESS(1,27,4,TRUE)"), "AA1");
        assert_eq!(f("ADDRESS(0,1)"), "#VALUE!");
        assert_eq!(f("ADDRESS(1,16385)"), "#VALUE!");
        assert_eq!(f("ADDRESS(1,1,0)"), "#VALUE!");
        assert_eq!(f("ADDRESS(1,1,9)"), "#VALUE!");
        assert_eq!(f("ADDRESS({1;2},1,4)"), "A1;A2");
        assert_eq!(calc(NUMS, "INDIRECT(ADDRESS(3,1))"), "30");
    }

    #[test]
    fn hyperlink_and_formulatext() {
        assert_eq!(f("HYPERLINK(\"https://example.com\",\"Click\")"), "Click");
        assert_eq!(
            f("HYPERLINK(\"https://example.com\")"),
            "https://example.com"
        );
        assert_eq!(f("HYPERLINK(\"https://example.com\",42)"), "42");
        assert_eq!(f("HYPERLINK(\"x\",1/0)"), "#DIV/0!");

        let cells = &[
            ("A1", "=SUM(B1:B3)*2"),
            ("A2", "5"),
            ("A3", "=IF(A2>1,\"yes\",\"no\")"),
        ];
        assert_eq!(calc(cells, "FORMULATEXT(A1)"), "=SUM(B1:B3)*2");
        assert_eq!(calc(cells, "FORMULATEXT(A2)"), "#N/A");
        assert_eq!(calc(cells, "FORMULATEXT(B9)"), "#N/A");
        assert_eq!(
            calc(cells, "FORMULATEXT(A1:A3)"),
            "=SUM(B1:B3)*2;#N/A;=IF(A2>1,\"yes\",\"no\")"
        );
        assert_eq!(
            calc(cells, "FORMULATEXT(OFFSET(A1,2,0))"),
            "=IF(A2>1,\"yes\",\"no\")"
        );
        assert_eq!(calc(cells, "FORMULATEXT(5)"), "#VALUE!");
        assert_eq!(calc(cells, "FORMULATEXT(AA1)"), "=FORMULATEXT(AA1)");

        // The text is written in the workbook's locale.
        let mut w = Workbook::new(Locale::fr());
        w.set_cell_text(0, 0, 0, "=SOMME(B1:B3;2,5)").unwrap();
        w.set_cell_text(0, 1, 0, "=FORMULATEXT(A1)").unwrap();
        w.commit("fr");
        assert_eq!(w.value_at(0, 1, 0), Value::text("=SOMME(B1:B3;2,5)"));
    }

    #[test]
    fn transpose() {
        assert_eq!(f("TRANSPOSE({1,2,3;4,5,6})"), "1,4;2,5;3,6");
        assert_eq!(f("TRANSPOSE({1;2;3})"), "1,2,3");
        assert_eq!(f("TRANSPOSE(5)"), "5");
        assert_eq!(f("TRANSPOSE(#N/A)"), "#N/A");
        assert_eq!(
            calc(FRUIT, "TRANSPOSE(A1:C2)"),
            "Fruit,Apple;Price,1.2;Qty,10"
        );
        assert_eq!(calc(FRUIT, "SUM(TRANSPOSE(C2:C5)*{1,2,3,4})"), "300");
        // The size is that of the range, used or not.
        assert_eq!(calc(FRUIT, "COLUMNS(TRANSPOSE(A1:A20))"), "20");
    }

    #[test]
    fn filter() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(
            v("FILTER(A2:C5,C2:C5>15)"),
            "Banana,0.5,20;Cherry,3,30;Damson,2.5,40"
        );
        assert_eq!(v("FILTER(A2:A5,(B2:B5>1)*(C2:C5<40))"), "Apple;Cherry");
        assert_eq!(v("FILTER(A2:A5,B2:B5>100,\"none\")"), "none");
        assert_eq!(v("FILTER(A2:A5,B2:B5>100)"), "#CALC!");
        assert_eq!(v("FILTER(A2:A5,B2:B5=3)"), "Cherry");
        // A row of flags filters columns.
        assert_eq!(
            v("FILTER(A2:C5,{1,0,1})"),
            "Apple,10;Banana,20;Cherry,30;Damson,40"
        );
        assert_eq!(v("FILTER(A2:C5,A1:C1=\"Price\")"), "1.2;0.5;3;2.5");
        // The flags must match the array.
        assert_eq!(v("FILTER(A2:C5,{1,0})"), "#VALUE!");
        assert_eq!(v("FILTER(A2:C5,{1;0;1})"), "#VALUE!");
        assert_eq!(v("FILTER(A2:C5,C2:C4>15)"), "#VALUE!");
        assert_eq!(v("FILTER(A2:A5,{1;\"x\";1;1})"), "#VALUE!");
        assert_eq!(v("FILTER(A2:A5,{1;#N/A;1;1})"), "#N/A");
        // Ranges reaching past the data, and whole columns.
        assert_eq!(v("FILTER(A2:A10,B2:B10>1)"), "Apple;Cherry;Damson");
        assert_eq!(v("FILTER(A2:A10,B2:B10)"), "Apple;Banana;Cherry;Damson");
        assert_eq!(v("FILTER(A:A,C:C>=30)"), "Fruit;Cherry;Damson");
        assert_eq!(v("FILTER(A:B,C:C)"), "#VALUE!");
        assert_eq!(v("SUM(FILTER(C2:C5,A2:A5<>\"Apple\"))"), "90");
    }

    #[test]
    fn whole_columns_are_trimmed_to_their_data() {
        // Column C is longer than column A.
        let cells = &[
            ("A1", "3"),
            ("A2", "1"),
            ("A3", "2"),
            ("C1", "9"),
            ("C2", "8"),
            ("C3", "7"),
            ("C4", "6"),
            ("C5", "5"),
        ];
        let v = |formula: &str| calc(cells, formula);
        assert_eq!(v("SORT(A:A)"), "1;2;3");
        assert_eq!(v("TAKE(A:A,-1)"), "2");
        assert_eq!(v("DROP(C:C,3)"), "6;5");
        assert_eq!(v("VSTACK(A:A,C:C)"), "3;1;2;9;8;7;6;5");
        assert_eq!(v("HSTACK(A:A,C:C)"), "3,9;1,8;2,7;#N/A,6;#N/A,5");
        assert_eq!(v("TOCOL(A:C)"), "3;0;9;1;0;8;2;0;7;0;0;6;0;0;5");
        assert_eq!(v("TRANSPOSE(2:2)"), "1;0;8");
        assert_eq!(v("FILTER(A:A,C:C>6)"), "3;1;2");
        assert_eq!(v("FILTER(C:C,A:A)"), "9;8;7");
        assert_eq!(v("FILTER(C:C,A:A<3)"), "8;7;6;5");
        assert_eq!(v("SORTBY(C:C,A:A)"), "8;7;9;6;5");
        assert_eq!(v("ROWS(UNIQUE(B:B))"), "1");
        // The count is that of the sheet for ROW, as for the range itself.
        assert_eq!(v("SUM((C:C>5)*ROW(C:C))"), "10");
        assert_eq!(v("SUM(COLUMN(2:2)*(2:2>2))"), "3");

        // A result does not grow by feeding on the cells it spills into.
        let mut w = wb(&[
            ("A1", "3"),
            ("A2", "1"),
            ("A3", "2"),
            ("E4", "=SORT(A:A)"),
            ("G6", "=VSTACK(A:A,A:A)"),
        ]);
        for _ in 0..3 {
            w.recalc();
        }
        assert_eq!(w.sheets[0].spill_at(3, 4).unwrap().array.rows, 3);
        assert_eq!(w.sheets[0].spill_at(5, 6).unwrap().array.rows, 6);
        assert_eq!(val(&w, "E6"), Value::Number(3.0));
        assert_eq!(val(&w, "G11"), Value::Number(2.0));

        // INDEX and ROW into a column do not depend on the rest of it.
        let w = wb(&[
            ("A1", "=INDEX(A:A,5)"),
            ("A2", "=ROWS(A:A)"),
            ("A3", "=SUM(ROW(A:A))"),
            ("A5", "9"),
        ]);
        assert!(!w.circular);
        assert_eq!(val(&w, "A1"), Value::Number(9.0));
        assert_eq!(val(&w, "A2"), Value::Number(1048576.0));
        assert_eq!(val(&w, "A3"), Value::Number(15.0));
    }

    #[test]
    fn sort() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(f("SORT({3;1;2})"), "1;2;3");
        assert_eq!(f("SORT({3;1;2},,-1)"), "3;2;1");
        assert_eq!(f("SORT({3;1;2},1,-1)"), "3;2;1");
        assert_eq!(f("SORT({3,1,2},,,TRUE)"), "1,2,3");
        assert_eq!(f("SORT({3,1,2})"), "3,1,2");
        assert_eq!(
            v("SORT(A2:C5,2,-1)"),
            "Cherry,3,30;Damson,2.5,40;Apple,1.2,10;Banana,0.5,20"
        );
        assert_eq!(
            v("SORT(A2:C5,2)"),
            "Banana,0.5,20;Apple,1.2,10;Damson,2.5,40;Cherry,3,30"
        );
        assert_eq!(v("SORT(A1:C2,1,-1,TRUE)"), "Qty,Price,Fruit;10,1.2,Apple");
        assert_eq!(v("SORT(A:A)"), "Apple;Banana;Cherry;Damson;Fruit");
        // Numbers, then text (ignoring case), then booleans.
        assert_eq!(
            f("SORT({\"b\";2;TRUE;\"A\";1;FALSE})"),
            "1;2;A;b;FALSE;TRUE"
        );
        assert_eq!(
            f("SORT({\"b\";2;TRUE;\"A\";1;FALSE},,-1)"),
            "TRUE;FALSE;b;A;2;1"
        );
        // Stable, and multi-level with arrays.
        assert_eq!(f("SORT({1,\"b\";1,\"a\";0,\"c\"})"), "0,c;1,b;1,a");
        assert_eq!(f("SORT({1,\"b\";1,\"a\";0,\"c\"},{1,2})"), "0,c;1,a;1,b");
        assert_eq!(
            f("SORT({1,\"b\";1,\"a\";0,\"c\"},{1,2},{-1,1})"),
            "1,a;1,b;0,c"
        );
        // Blanks go last in either direction.
        let gaps = &[("A1", "3"), ("A3", "1"), ("A4", "2")];
        assert_eq!(calc(gaps, "SORT(A1:A4)"), "1;2;3;0");
        assert_eq!(calc(gaps, "SORT(A1:A4,,-1)"), "3;2;1;0");
        assert_eq!(calc(gaps, "ROWS(SORT(A1:A9))"), "9");
        // Errors.
        assert_eq!(v("SORT(A2:C5,4)"), "#VALUE!");
        assert_eq!(v("SORT(A2:C5,0)"), "#VALUE!");
        assert_eq!(v("SORT(A2:C5,1,2)"), "#VALUE!");
        assert_eq!(v("SORT(A2:C5,1,0)"), "#VALUE!");
        assert_eq!(f("SORT(#DIV/0!)"), "#DIV/0!");
    }

    #[test]
    fn sortby() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(v("SORTBY(A2:A5,B2:B5)"), "Banana;Apple;Damson;Cherry");
        assert_eq!(v("SORTBY(A2:A5,B2:B5,-1)"), "Cherry;Damson;Apple;Banana");
        assert_eq!(
            v("SORTBY(A2:B5,C2:C5,-1)"),
            "Damson,2.5;Cherry,3;Banana,0.5;Apple,1.2"
        );
        assert_eq!(
            v("SORTBY(A2:A5,{5;6;6;6},1,B2:B5,-1)"),
            "Apple;Cherry;Damson;Banana"
        );
        // Several keys.
        let letters = "{\"a\";\"b\";\"c\";\"d\"}";
        assert_eq!(
            f(&format!("SORTBY({letters},{{1;2;1;2}},1,{{4;3;2;1}},1)")),
            "c;a;d;b"
        );
        assert_eq!(
            f(&format!("SORTBY({letters},{{1;2;1;2}},-1,{{4;3;2;1}},1)")),
            "d;b;c;a"
        );
        assert_eq!(f(&format!("SORTBY({letters},{{1;2;1;2}},-1)")), "b;d;a;c");
        // By columns.
        assert_eq!(
            f("SORTBY({\"x\",\"y\",\"z\";1,2,3},{3,1,2})"),
            "y,z,x;2,3,1"
        );
        // Errors.
        assert_eq!(v("SORTBY(A2:A5,B2:B4)"), "#VALUE!");
        assert_eq!(v("SORTBY(A2:A5,B2:C5)"), "#VALUE!");
        assert_eq!(v("SORTBY(A2:A5,B2:B5,2)"), "#VALUE!");
        assert_eq!(f("SORTBY({1,2;3,4},{1;2},1,{1,2},1)"), "#VALUE!");
    }

    #[test]
    fn unique() {
        assert_eq!(f("UNIQUE({1;2;2;3;1})"), "1;2;3");
        assert_eq!(f("UNIQUE({1;2;2;3;1},,TRUE)"), "3");
        assert_eq!(f("UNIQUE({1;2;2;3;1},FALSE,TRUE)"), "3");
        assert_eq!(f("UNIQUE({\"a\";\"A\";\"b\"})"), "a;b");
        assert_eq!(f("UNIQUE({1;\"1\";TRUE;1})"), "1;1;TRUE");
        assert_eq!(f("UNIQUE({1,2;1,2;3,4})"), "1,2;3,4");
        assert_eq!(f("UNIQUE({1,2;1,3;1,2})"), "1,2;1,3");
        assert_eq!(f("UNIQUE({1,1,2},TRUE)"), "1,2");
        assert_eq!(f("UNIQUE({1,1,2})"), "1,1,2");
        assert_eq!(f("UNIQUE({1,1,2;3,3,4},TRUE,TRUE)"), "2;4");
        assert_eq!(f("UNIQUE({1;1},,TRUE)"), "#CALC!");
        assert_eq!(f("UNIQUE(7)"), "7");
        let cells = &[
            ("A1", "x"),
            ("A2", "y"),
            ("A3", "X"),
            ("A4", "z"),
            ("A5", "y"),
        ];
        assert_eq!(calc(cells, "UNIQUE(A1:A5)"), "x;y;z");
        assert_eq!(calc(cells, "UNIQUE(A:A)"), "x;y;z");
        assert_eq!(calc(cells, "UNIQUE(A1:A5,,TRUE)"), "z");
        assert_eq!(calc(cells, "SORT(UNIQUE(A1:A5),,-1)"), "z;y;x");
        assert_eq!(calc(cells, "ROWS(UNIQUE(A1:A7))"), "4");
    }

    #[test]
    fn take_and_drop() {
        let a = "{1,2,3;4,5,6;7,8,9}";
        assert_eq!(f(&format!("TAKE({a},2)")), "1,2,3;4,5,6");
        assert_eq!(f(&format!("TAKE({a},-1)")), "7,8,9");
        assert_eq!(f(&format!("TAKE({a},-1,-2)")), "8,9");
        assert_eq!(f(&format!("TAKE({a},,1)")), "1;4;7");
        assert_eq!(f(&format!("TAKE({a},2,2)")), "1,2;4,5");
        assert_eq!(f(&format!("TAKE({a},5)")), "1,2,3;4,5,6;7,8,9");
        assert_eq!(f(&format!("TAKE({a},1.9)")), "1,2,3");
        assert_eq!(f(&format!("TAKE({a},0)")), "#CALC!");
        assert_eq!(f(&format!("TAKE({a},1,0)")), "#CALC!");
        assert_eq!(f(&format!("TAKE({a},\"x\")")), "#VALUE!");

        assert_eq!(f(&format!("DROP({a},1)")), "4,5,6;7,8,9");
        assert_eq!(f(&format!("DROP({a},-1)")), "1,2,3;4,5,6");
        assert_eq!(f(&format!("DROP({a},-1,1)")), "2,3;5,6");
        assert_eq!(f(&format!("DROP({a},,2)")), "3;6;9");
        assert_eq!(f(&format!("DROP({a},0,-2)")), "1;4;7");
        assert_eq!(f(&format!("DROP({a},2,2)")), "9");
        assert_eq!(f(&format!("DROP({a},3)")), "#CALC!");
        assert_eq!(f(&format!("DROP({a},1,-3)")), "#CALC!");

        // Counted on the range as written, not on its used part.
        assert_eq!(calc(NUMS, "TAKE(A1:A5,-2)"), "40;50");
        assert_eq!(calc(NUMS, "TAKE(A1:A8,-1)"), "0");
        assert_eq!(calc(NUMS, "DROP(A1:A8,3)"), "40;50;0;0;0");
        assert_eq!(calc(NUMS, "SUM(DROP(A:A,3))"), "90");
        assert_eq!(calc(NUMS, "TAKE(A:A,-1)"), "50");
    }

    #[test]
    fn choosecols_and_chooserows() {
        let a = "{1,2,3;4,5,6;7,8,9}";
        assert_eq!(f(&format!("CHOOSECOLS({a},3,1)")), "3,1;6,4;9,7");
        assert_eq!(f(&format!("CHOOSECOLS({a},-1)")), "3;6;9");
        assert_eq!(
            f(&format!("CHOOSECOLS({a},{{1,1}},2)")),
            "1,1,2;4,4,5;7,7,8"
        );
        assert_eq!(f(&format!("CHOOSECOLS({a},4)")), "#VALUE!");
        assert_eq!(f(&format!("CHOOSECOLS({a},0)")), "#VALUE!");
        assert_eq!(f(&format!("CHOOSECOLS({a},-4)")), "#VALUE!");
        assert_eq!(f(&format!("CHOOSEROWS({a},{{1,3}})")), "1,2,3;7,8,9");
        assert_eq!(f(&format!("CHOOSEROWS({a},-1,1)")), "7,8,9;1,2,3");
        assert_eq!(f(&format!("CHOOSEROWS({a},2)")), "4,5,6");
        assert_eq!(f(&format!("CHOOSEROWS({a},4)")), "#VALUE!");
        assert_eq!(f(&format!("CHOOSEROWS({a},\"x\")")), "#VALUE!");
        assert_eq!(calc(FRUIT, "CHOOSECOLS(A2:C3,3,1)"), "10,Apple;20,Banana");
        assert_eq!(calc(FRUIT, "CHOOSEROWS(A1:C5,-1)"), "Damson,2.5,40");
    }

    #[test]
    fn vstack_and_hstack() {
        assert_eq!(f("VSTACK({1,2},{3,4})"), "1,2;3,4");
        assert_eq!(f("VSTACK({1,2},{3})"), "1,2;3,#N/A");
        assert_eq!(f("VSTACK(1,2,3)"), "1;2;3");
        assert_eq!(f("VSTACK({1;2},{3,4,5})"), "1,#N/A,#N/A;2,#N/A,#N/A;3,4,5");
        assert_eq!(f("HSTACK({1;2},{3;4})"), "1,3;2,4");
        assert_eq!(f("HSTACK({1;2},{3})"), "1,3;2,#N/A");
        assert_eq!(f("HSTACK(\"a\",{1,2})"), "a,1,2");
        assert_eq!(
            calc(FRUIT, "VSTACK(A2:B3,A5:B5)"),
            "Apple,1.2;Banana,0.5;Damson,2.5"
        );
        assert_eq!(calc(FRUIT, "HSTACK(A2:A3,C2:C3)"), "Apple,10;Banana,20");
        assert_eq!(calc(NUMS, "SUM(VSTACK(A1:A2,A4:A5))"), "120");
    }

    #[test]
    fn tocol_and_torow() {
        assert_eq!(f("TOCOL({1,2;3,4})"), "1;2;3;4");
        assert_eq!(f("TOCOL({1,2;3,4},,TRUE)"), "1;3;2;4");
        assert_eq!(f("TOROW({1,2;3,4})"), "1,2,3,4");
        assert_eq!(f("TOROW({1,2;3,4},0,TRUE)"), "1,3,2,4");
        assert_eq!(f("TOROW({1,#N/A;3,4},2)"), "1,3,4");
        assert_eq!(f("TOCOL({1,#N/A;3,4},1)"), "1;#N/A;3;4");
        assert_eq!(f("TOCOL({1,2},4)"), "#VALUE!");
        assert_eq!(f("TOCOL({1,2},-1)"), "#VALUE!");
        assert_eq!(f("TOCOL({#N/A,#DIV/0!},2)"), "#CALC!");
        let cells = &[("A1", "1"), ("B1", "=1/0"), ("A2", "x"), ("B3", "4")];
        assert_eq!(calc(cells, "TOCOL(A1:B3)"), "1;#DIV/0!;x;0;0;4");
        assert_eq!(calc(cells, "TOCOL(A1:B3,1)"), "1;#DIV/0!;x;4");
        assert_eq!(calc(cells, "TOCOL(A1:B3,2)"), "1;x;0;0;4");
        assert_eq!(calc(cells, "TOCOL(A1:B3,3)"), "1;x;4");
        assert_eq!(calc(cells, "TOROW(A1:B3,3,TRUE)"), "1,x,4");
        assert_eq!(calc(cells, "TOCOL(A:B,3)"), "1;x;4");
    }

    #[test]
    fn wraprows_and_wrapcols() {
        assert_eq!(f("WRAPROWS({1,2,3,4,5},2)"), "1,2;3,4;5,#N/A");
        assert_eq!(f("WRAPROWS({1,2,3,4,5,6},3)"), "1,2,3;4,5,6");
        assert_eq!(f("WRAPROWS({1;2;3;4;5},2,0)"), "1,2;3,4;5,0");
        assert_eq!(f("WRAPCOLS({1,2,3,4,5},2)"), "1,3,5;2,4,#N/A");
        assert_eq!(f("WRAPCOLS({1,2,3,4,5},2,\"x\")"), "1,3,5;2,4,x");
        assert_eq!(f("WRAPCOLS({1,2,3},1)"), "1,2,3");
        assert_eq!(f("WRAPROWS({1,2,3},1)"), "1;2;3");
        assert_eq!(f("WRAPROWS({1,2;3,4},2)"), "#VALUE!");
        assert_eq!(f("WRAPCOLS({1,2;3,4},2)"), "#VALUE!");
        assert_eq!(f("WRAPROWS({1,2,3},0)"), "#NUM!");
        assert_eq!(f("WRAPCOLS({1,2,3},-1)"), "#NUM!");
        assert_eq!(calc(NUMS, "WRAPROWS(A1:A5,3,\"-\")"), "10,20,30;40,50,-");
    }

    #[test]
    fn expand() {
        assert_eq!(
            f("EXPAND({1,2;3,4},3,3)"),
            "1,2,#N/A;3,4,#N/A;#N/A,#N/A,#N/A"
        );
        assert_eq!(f("EXPAND({1,2;3,4},3,3,0)"), "1,2,0;3,4,0;0,0,0");
        assert_eq!(f("EXPAND({1,2;3,4},3)"), "1,2;3,4;#N/A,#N/A");
        assert_eq!(f("EXPAND({1,2;3,4},,3,\"-\")"), "1,2,-;3,4,-");
        assert_eq!(f("EXPAND({1,2;3,4},2,2)"), "1,2;3,4");
        assert_eq!(f("EXPAND(5,2,2,\"\")"), "5,;,");
        assert_eq!(f("EXPAND({1,2;3,4},1)"), "#VALUE!");
        assert_eq!(f("EXPAND({1,2;3,4},2,1)"), "#VALUE!");
        assert_eq!(f("EXPAND({1,2;3,4},2000000)"), "#NUM!");
    }

    #[test]
    fn composed() {
        let v = |formula: &str| calc(FRUIT, formula);
        assert_eq!(v("INDEX(SORT(A2:C5,3,-1),1,1)"), "Damson");
        assert_eq!(v("TAKE(SORTBY(A2:A5,C2:C5,-1),2)"), "Damson;Cherry");
        assert_eq!(v("XLOOKUP(MAX(B2:B5),B2:B5,A2:A5)"), "Cherry");
        assert_eq!(v("INDEX(A2:A5,XMATCH(40,C2:C5))"), "Damson");
        assert_eq!(
            v("VLOOKUP(\"Banana\",FILTER(A2:C5,C2:C5>10),3,FALSE)"),
            "20"
        );
        assert_eq!(
            v("HSTACK(TAKE(A2:A5,2),DROP(C2:C5,2))"),
            "Apple,30;Banana,40"
        );
        assert_eq!(v("TRANSPOSE(UNIQUE(TOCOL({1,2;2,1})))"), "1,2");
        assert_eq!(v("ROWS(FILTER(A:C,C:C>10))"), "4");
        // Results spill into the neighbouring cells.
        let mut cells = FRUIT.to_vec();
        cells.push(("E1", "=SORT(A2:B5,2)"));
        cells.push(("H1", "=SUM(F1:F4)"));
        let w = wb(&cells);
        assert_eq!(val(&w, "E1"), Value::text("Banana"));
        assert_eq!(val(&w, "F1"), Value::Number(0.5));
        assert_eq!(val(&w, "E4"), Value::text("Cherry"));
        assert_eq!(val(&w, "F4"), Value::Number(3.0));
        assert_eq!(val(&w, "H1"), Value::Number(7.2));
        assert_eq!(eval("SUM(SORT({3;1;2}))"), Value::Number(6.0));
        assert_eq!(
            eval("VLOOKUP(2,{1,\"a\";2,\"b\"},3,0)"),
            Value::Error(ErrorKind::Ref)
        );
    }
}
