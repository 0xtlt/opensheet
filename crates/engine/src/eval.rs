//! Formula evaluation.
//!
//! A [`Calc`] is one recalculation pass over an immutable [`Workbook`]:
//! formula results are memoised in a side table and written back to the
//! workbook once the pass is done. [`EvalCtx`] is the handle functions use
//! to evaluate their arguments.

use crate::ast::{BinaryOp, Expr, RefExpr, UnaryOp};
use crate::functions::{self, FunctionDef, Kind};
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, Reference};
use crate::value::{Array, ErrorKind, R, Value};
use crate::workbook::{Content, Workbook};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

type CellKey = (usize, u32, u32);

enum Slot {
    Computing,
    Done(Value),
}

/// Where a dynamic array landed: anchor cell plus covered rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpillRect {
    pub anchor: (u32, u32),
    pub rect: Rect,
}

/// Outcome of a recalculation pass.
pub struct CalcResult {
    /// Value of every formula cell (arrays are kept whole on the anchor).
    pub values: Vec<(CellKey, Value)>,
    /// Spilled arrays per sheet.
    pub spills: Vec<Vec<(SpillRect, Arc<Array>)>>,
    pub circular: bool,
}

const MAX_DEPTH: u32 = 20_000;

pub struct Calc<'a> {
    wb: &'a Workbook,
    cache: RefCell<HashMap<CellKey, Slot>>,
    /// Spills registered during this pass, per sheet.
    spills: RefCell<Vec<Vec<SpillRect>>>,
    /// Spill geometry of the previous pass, used to find anchors that have
    /// not been evaluated yet.
    hints: Vec<Vec<SpillRect>>,
    /// (max row, max col) holding data per sheet, grown as arrays spill.
    extents: RefCell<Vec<(u32, u32)>>,
    range_cache: RefCell<HashMap<Reference, Arc<Array>>>,
    circular: Cell<bool>,
    depth: Cell<u32>,
    now: f64,
}

impl<'a> Calc<'a> {
    fn new(wb: &'a Workbook, hints: Vec<Vec<SpillRect>>, now: f64) -> Calc<'a> {
        let mut extents = Vec::with_capacity(wb.sheets.len());
        for (i, sheet) in wb.sheets.iter().enumerate() {
            let (mut r, mut c) = sheet.content_extent();
            for h in hints.get(i).map(|v| v.as_slice()).unwrap_or(&[]) {
                r = r.max(h.rect.r1);
                c = c.max(h.rect.c1);
            }
            extents.push((r, c));
        }
        Calc {
            wb,
            cache: RefCell::new(HashMap::new()),
            spills: RefCell::new(vec![Vec::new(); wb.sheets.len()]),
            hints,
            extents: RefCell::new(extents),
            range_cache: RefCell::new(HashMap::new()),
            circular: Cell::new(false),
            depth: Cell::new(0),
            now,
        }
    }

    /// Recalculate every formula in the workbook.
    pub fn run(wb: &Workbook) -> CalcResult {
        let now = crate::datetime::now_serial();
        let mut hints: Vec<Vec<SpillRect>> = wb
            .sheets
            .iter()
            .map(|s| {
                s.spills
                    .iter()
                    .map(|sp| SpillRect {
                        anchor: (sp.row, sp.col),
                        rect: Rect::new(
                            sp.row,
                            sp.col,
                            sp.row + sp.array.rows as u32 - 1,
                            sp.col + sp.array.cols as u32 - 1,
                        ),
                    })
                    .collect()
            })
            .collect();
        let mut iteration = 0;
        loop {
            let calc = Calc::new(wb, hints.clone(), now);
            for (si, sheet) in wb.sheets.iter().enumerate() {
                for ((row, col), cell) in sheet.cells.iter() {
                    if let Content::Formula(expr) = &cell.content {
                        calc.formula_value(si, *row, *col, expr);
                    }
                }
            }
            let new_spills = calc.spills.borrow().clone();
            iteration += 1;
            if new_spills == hints || iteration >= 4 {
                let cache = calc.cache.into_inner();
                let mut spills = Vec::with_capacity(new_spills.len());
                for (si, list) in new_spills.iter().enumerate() {
                    let mut out = Vec::with_capacity(list.len());
                    for sp in list {
                        if let Some(Slot::Done(Value::Array(a))) =
                            cache.get(&(si, sp.anchor.0, sp.anchor.1))
                        {
                            out.push((*sp, a.clone()));
                        }
                    }
                    spills.push(out);
                }
                let values = cache
                    .into_iter()
                    .filter_map(|(k, slot)| match slot {
                        Slot::Done(v) => Some((k, v)),
                        Slot::Computing => None,
                    })
                    .collect();
                return CalcResult {
                    values,
                    spills,
                    circular: calc.circular.get(),
                };
            }
            hints = new_spills;
        }
    }

    /// Evaluate a standalone expression against the workbook's current
    /// (already calculated) state, as if it lived in `(sheet, row, col)`.
    pub fn eval_once(wb: &Workbook, sheet: usize, row: u32, col: u32, expr: &Expr) -> Value {
        let hints = vec![Vec::new(); wb.sheets.len()];
        let calc = Calc::new(wb, hints, crate::datetime::now_serial());
        // Seed with stored results so nothing is recomputed.
        {
            let mut cache = calc.cache.borrow_mut();
            let mut spills = calc.spills.borrow_mut();
            for (si, sh) in wb.sheets.iter().enumerate() {
                for ((r, c), cell) in sh.cells.iter() {
                    if matches!(cell.content, Content::Formula(_)) {
                        cache.insert((si, *r, *c), Slot::Done(cell.value.clone()));
                    }
                }
                for sp in &sh.spills {
                    cache.insert((si, sp.row, sp.col), Slot::Done(Value::Array(sp.array.clone())));
                    spills[si].push(SpillRect {
                        anchor: (sp.row, sp.col),
                        rect: Rect::new(
                            sp.row,
                            sp.col,
                            sp.row + sp.array.rows as u32 - 1,
                            sp.col + sp.array.cols as u32 - 1,
                        ),
                    });
                }
            }
        }
        let ctx = EvalCtx {
            calc: &calc,
            sheet,
            row,
            col,
        };
        ctx.eval(expr)
    }

    fn formula_value(&self, sheet: usize, row: u32, col: u32, expr: &Expr) -> Value {
        let key = (sheet, row, col);
        match self.cache.borrow().get(&key) {
            Some(Slot::Done(v)) => return v.clone(),
            Some(Slot::Computing) => {
                self.circular.set(true);
                return Value::Number(0.0);
            }
            None => {}
        }
        if self.depth.get() >= MAX_DEPTH {
            return Value::Error(ErrorKind::Calc);
        }
        self.cache.borrow_mut().insert(key, Slot::Computing);
        self.depth.set(self.depth.get() + 1);
        let ctx = EvalCtx {
            calc: self,
            sheet,
            row,
            col,
        };
        let mut v = ctx.eval(expr);
        self.depth.set(self.depth.get() - 1);

        if let Value::Array(a) = &v {
            if a.len() == 0 {
                v = Value::Error(ErrorKind::Calc);
            } else if a.len() == 1 {
                v = a.data[0].clone();
            } else if !self.register_spill(sheet, row, col, a) {
                v = Value::Error(ErrorKind::Spill);
            }
        }
        if let Value::Empty = v {
            // A formula never yields a blank: an empty result reads as 0.
            v = Value::Number(0.0);
        }
        self.cache.borrow_mut().insert(key, Slot::Done(v.clone()));
        v
    }

    fn register_spill(&self, sheet: usize, row: u32, col: u32, a: &Array) -> bool {
        let r1 = row as u64 + a.rows as u64 - 1;
        let c1 = col as u64 + a.cols as u64 - 1;
        if r1 >= MAX_ROWS as u64 || c1 >= MAX_COLS as u64 {
            return false;
        }
        let rect = Rect::new(row, col, r1 as u32, c1 as u32);
        let sh = &self.wb.sheets[sheet];
        for ((r, c), cell) in sh.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
            if *c < rect.c0 || *c > rect.c1 || (*r == row && *c == col) {
                continue;
            }
            if !matches!(cell.content, Content::Empty) {
                return false;
            }
        }
        let mut spills = self.spills.borrow_mut();
        if spills[sheet].iter().any(|s| s.rect.intersects(&rect)) {
            return false;
        }
        spills[sheet].push(SpillRect {
            anchor: (row, col),
            rect,
        });
        let mut ext = self.extents.borrow_mut();
        ext[sheet].0 = ext[sheet].0.max(rect.r1);
        ext[sheet].1 = ext[sheet].1.max(rect.c1);
        true
    }

    fn spill_at(&self, sheet: usize, row: u32, col: u32) -> Option<SpillRect> {
        self.spills.borrow()[sheet]
            .iter()
            .find(|s| s.rect.contains(row, col))
            .copied()
    }

    /// Make sure anchors that spilled over `rect` last time are evaluated.
    fn ensure_hinted(&self, sheet: usize, rect: &Rect) {
        let Some(hints) = self.hints.get(sheet) else {
            return;
        };
        for h in hints {
            if !h.rect.intersects(rect) {
                continue;
            }
            let key = (sheet, h.anchor.0, h.anchor.1);
            if self.cache.borrow().contains_key(&key) {
                continue;
            }
            if let Some(cell) = self.wb.sheets[sheet].cells.get(&h.anchor) {
                if let Content::Formula(expr) = &cell.content {
                    self.formula_value(sheet, h.anchor.0, h.anchor.1, expr);
                }
            }
        }
    }

    fn spilled_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.ensure_hinted(sheet, &Rect::cell(row, col));
        if let Some(sp) = self.spill_at(sheet, row, col) {
            if let Some(Slot::Done(Value::Array(a))) =
                self.cache.borrow().get(&(sheet, sp.anchor.0, sp.anchor.1))
            {
                return a
                    .get((row - sp.anchor.0) as usize, (col - sp.anchor.1) as usize)
                    .clone();
            }
        }
        Value::Empty
    }

    /// Scalar value of a cell.
    fn cell_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        let Some(sh) = self.wb.sheets.get(sheet) else {
            return Value::Error(ErrorKind::Ref);
        };
        match sh.cells.get(&(row, col)).map(|c| &c.content) {
            Some(Content::Formula(expr)) => self.formula_value(sheet, row, col, expr).into_scalar(),
            Some(Content::Value(v)) => v.clone(),
            _ => self.spilled_value(sheet, row, col),
        }
    }

    /// Materialise a range, clamped to the part of the sheet holding data.
    fn range_array(&self, reference: &Reference) -> Arc<Array> {
        if let Some(a) = self.range_cache.borrow().get(reference) {
            return a.clone();
        }
        let Some(sh) = self.wb.sheets.get(reference.sheet) else {
            return Arc::new(Array::row(vec![Value::Error(ErrorKind::Ref)]));
        };
        let rect = reference.rect;
        self.ensure_hinted(reference.sheet, &rect);
        let (er, ec) = self.extents.borrow()[reference.sheet];
        let r1 = rect.r1.min(er.max(rect.r0));
        let c1 = rect.c1.min(ec.max(rect.c0));
        let rows = (r1 - rect.r0 + 1) as usize;
        let cols = (c1 - rect.c0 + 1) as usize;
        let mut arr = Array::new(rows, cols);

        let mut fill = |r: u32, c: u32, content: &Content| {
            let v = match content {
                Content::Formula(expr) => self.formula_value(reference.sheet, r, c, expr).into_scalar(),
                Content::Value(v) => v.clone(),
                Content::Empty => return,
            };
            arr.set((r - rect.r0) as usize, (c - rect.c0) as usize, v);
        };
        if cols <= 4 {
            for c in rect.c0..=c1 {
                // Per-column scan over existing rows only.
                for r in rect.r0..=r1 {
                    if let Some(cell) = sh.cells.get(&(r, c)) {
                        fill(r, c, &cell.content);
                    }
                }
            }
        } else {
            for ((r, c), cell) in sh.cells.range((rect.r0, rect.c0)..=(r1, c1)) {
                if *c >= rect.c0 && *c <= c1 {
                    fill(*r, *c, &cell.content);
                }
            }
        }
        // Overlay spilled arrays.
        let clamped = Rect::new(rect.r0, rect.c0, r1, c1);
        let spills: Vec<SpillRect> = self.spills.borrow()[reference.sheet]
            .iter()
            .filter(|s| s.rect.intersects(&clamped))
            .copied()
            .collect();
        for sp in spills {
            let Some(Slot::Done(Value::Array(a))) = self
                .cache
                .borrow()
                .get(&(reference.sheet, sp.anchor.0, sp.anchor.1))
                .map(|s| match s {
                    Slot::Done(v) => Slot::Done(v.clone()),
                    Slot::Computing => Slot::Computing,
                })
            else {
                continue;
            };
            let Some(overlap) = sp.rect.intersection(&clamped) else {
                continue;
            };
            for r in overlap.r0..=overlap.r1 {
                for c in overlap.c0..=overlap.c1 {
                    arr.set(
                        (r - rect.r0) as usize,
                        (c - rect.c0) as usize,
                        a.get((r - sp.anchor.0) as usize, (c - sp.anchor.1) as usize)
                            .clone(),
                    );
                }
            }
        }
        let arr = Arc::new(arr);
        // Only cache when nothing is mid-computation (no cycle in flight), so
        // a partial view is never reused.
        if !self.circular.get() {
            self.range_cache.borrow_mut().insert(*reference, arr.clone());
        }
        arr
    }
}

/// Evaluation handle passed to function implementations.
#[derive(Clone, Copy)]
pub struct EvalCtx<'a> {
    calc: &'a Calc<'a>,
    /// Sheet index of the cell being calculated.
    pub sheet: usize,
    /// Row of the cell being calculated (0-based).
    pub row: u32,
    /// Column of the cell being calculated (0-based).
    pub col: u32,
}

impl<'a> EvalCtx<'a> {
    pub fn workbook(&self) -> &'a Workbook {
        self.calc.wb
    }

    pub fn locale(&self) -> &'static Locale {
        self.calc.wb.locale
    }

    /// Current date-time serial, fixed for the whole recalculation.
    pub fn now(&self) -> f64 {
        self.calc.now
    }

    /// True when the argument was omitted, as in `IF(A1,,2)`.
    pub fn is_missing(&self, e: &Expr) -> bool {
        matches!(e, Expr::Missing)
    }

    /// Evaluate to a full value: a scalar, or an `Array` for ranges, array
    /// constants and array-returning functions. Ranges are clamped to the
    /// used part of the sheet.
    pub fn eval(&self, e: &Expr) -> Value {
        match e {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(s) => Value::Text(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Error(err) => Value::Error(*err),
            Expr::Missing => Value::Empty,
            Expr::Value(v) => v.clone(),
            Expr::Paren(inner) => self.eval(inner),
            Expr::Ref(r) => match self.resolve(r) {
                Err(err) => Value::Error(err),
                Ok(reference) => self.deref(&reference),
            },
            Expr::Name(name) => match self.calc.wb.defined_name(name) {
                Some(expr) => self.eval(expr),
                None => Value::Error(ErrorKind::Name),
            },
            Expr::Unary(op, inner) => {
                let v = self.eval(inner);
                self.map_unary(v, &|x| match op {
                    UnaryOp::Neg => self.to_number(&x).map(|n| Value::Number(-n)),
                    UnaryOp::Plus => Ok(x),
                })
            }
            Expr::Percent(inner) => {
                let v = self.eval(inner);
                self.map_unary(v, &|x| self.to_number(&x).map(|n| Value::Number(n / 100.0)))
            }
            Expr::Binary(op, a, b) => {
                let a = self.eval(a);
                let b = self.eval(b);
                self.binary(*op, &a, &b)
            }
            Expr::Func(name, args) => match functions::lookup(name) {
                Some(def) => self.call(def, args),
                None => Value::Error(ErrorKind::Name),
            },
            Expr::Array(rows) => {
                let r = rows.len();
                let c = rows.first().map(|x| x.len()).unwrap_or(0);
                let mut data = Vec::with_capacity(r * c);
                for row in rows {
                    for e in row {
                        data.push(self.eval(e).into_scalar());
                    }
                }
                Value::array(Array {
                    rows: r,
                    cols: c,
                    data,
                })
            }
        }
    }

    /// Evaluate to a scalar: arrays and ranges collapse to their top-left
    /// element.
    pub fn eval_scalar(&self, e: &Expr) -> Value {
        self.eval(e).into_scalar()
    }

    /// Evaluate an argument that must be a reference (for `ROW`, `OFFSET`,
    /// `COUNTBLANK`, ...). `#VALUE!` when the expression is not a reference.
    pub fn eval_ref(&self, e: &Expr) -> R<Reference> {
        match e {
            Expr::Ref(r) => self.resolve(r),
            Expr::Paren(inner) => self.eval_ref(inner),
            Expr::Name(name) => match self.calc.wb.defined_name(name) {
                Some(expr) => self.eval_ref(expr),
                None => Err(ErrorKind::Name),
            },
            Expr::Func(name, args) => match functions::lookup(name) {
                Some(def) => match def.ref_func {
                    Some(f) => f(self, args),
                    None => Err(ErrorKind::Value),
                },
                None => Err(ErrorKind::Name),
            },
            Expr::Error(err) => Err(*err),
            _ => Err(ErrorKind::Value),
        }
    }

    /// Resolve a reference as written in a formula to sheet coordinates.
    pub fn resolve(&self, r: &RefExpr) -> R<Reference> {
        let sheet = match &r.sheet {
            None => self.sheet,
            Some(name) => self.calc.wb.sheet_index(name).ok_or(ErrorKind::Ref)?,
        };
        Ok(Reference {
            sheet,
            rect: r.rect(),
        })
    }

    /// Value of a resolved reference: a scalar for one cell, else an array.
    pub fn deref(&self, reference: &Reference) -> Value {
        if reference.is_single() {
            self.calc
                .cell_value(reference.sheet, reference.rect.r0, reference.rect.c0)
        } else {
            Value::Array(self.calc.range_array(reference))
        }
    }

    /// Values of a resolved reference as an array (clamped to used cells).
    pub fn range_values(&self, reference: &Reference) -> Arc<Array> {
        self.calc.range_array(reference)
    }

    /// Scalar value of one cell.
    pub fn cell_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.calc.cell_value(sheet, row, col)
    }

    /// The formula stored in a cell, if any.
    pub fn formula_at(&self, sheet: usize, row: u32, col: u32) -> Option<&'a Expr> {
        match &self.calc.wb.sheets.get(sheet)?.cells.get(&(row, col))?.content {
            Content::Formula(e) => Some(e.as_ref()),
            _ => None,
        }
    }

    /// Parse and resolve reference text such as "A1", "Sheet2!B2:C3"
    /// (for `INDIRECT`).
    pub fn parse_reference(&self, text: &str) -> R<Reference> {
        let expr = crate::parser::parse_formula(text.trim(), self.locale())
            .or_else(|_| crate::parser::parse_formula(text.trim(), Locale::en()))
            .map_err(|_| ErrorKind::Ref)?;
        match &expr {
            Expr::Ref(_) | Expr::Name(_) => self.eval_ref(&expr).map_err(|_| ErrorKind::Ref),
            _ => Err(ErrorKind::Ref),
        }
    }

    fn call(&self, def: &FunctionDef, args: &[Expr]) -> Value {
        if args.len() < def.min_args as usize
            || (def.max_args != u8::MAX && args.len() > def.max_args as usize)
        {
            return Value::Error(ErrorKind::Value);
        }
        match def.kind {
            Kind::Raw => wrap((def.func)(self, args)),
            Kind::Scalar => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval(a)).collect();
                let mut rows = 1;
                let mut cols = 1;
                let mut any = false;
                for v in &vals {
                    if let Value::Array(a) = v {
                        if a.len() != 1 {
                            any = true;
                            rows = rows.max(a.rows);
                            cols = cols.max(a.cols);
                        }
                    }
                }
                let lit = |v: Value, original: &Expr| match original {
                    Expr::Missing => Expr::Missing,
                    _ => Expr::Value(v),
                };
                if !any {
                    let lits: Vec<Expr> = vals
                        .into_iter()
                        .zip(args)
                        .map(|(v, a)| lit(v.into_scalar(), a))
                        .collect();
                    return wrap((def.func)(self, &lits)).into_scalar();
                }
                let mut out = Array::new(rows, cols);
                for r in 0..rows {
                    for c in 0..cols {
                        let lits: Vec<Expr> = vals
                            .iter()
                            .zip(args)
                            .map(|(v, a)| lit(v.get_broadcast(r, c), a))
                            .collect();
                        out.set(r, c, wrap((def.func)(self, &lits)).into_scalar());
                    }
                }
                Value::array(out)
            }
        }
    }

    fn map_unary(&self, v: Value, f: &dyn Fn(Value) -> R<Value>) -> Value {
        match v {
            Value::Array(a) => {
                let data = a.data.iter().map(|x| wrap(f(x.clone()))).collect();
                Value::array(Array {
                    rows: a.rows,
                    cols: a.cols,
                    data,
                })
            }
            x => wrap(f(x)),
        }
    }

    /// Apply a binary operator with array broadcasting.
    pub fn binary(&self, op: BinaryOp, a: &Value, b: &Value) -> Value {
        match (a, b) {
            (Value::Array(_), _) | (_, Value::Array(_)) => {
                let (ar, ac) = a.dims();
                let (br, bc) = b.dims();
                let rows = ar.max(br);
                let cols = ac.max(bc);
                let mut out = Array::new(rows, cols);
                for r in 0..rows {
                    for c in 0..cols {
                        let x = a.get_broadcast(r, c);
                        let y = b.get_broadcast(r, c);
                        out.set(r, c, self.binary_scalar(op, &x, &y));
                    }
                }
                if rows == 1 && cols == 1 {
                    out.data.pop().unwrap()
                } else {
                    Value::array(out)
                }
            }
            _ => self.binary_scalar(op, a, b),
        }
    }

    fn binary_scalar(&self, op: BinaryOp, a: &Value, b: &Value) -> Value {
        if let Value::Error(e) = a {
            return Value::Error(*e);
        }
        if let Value::Error(e) = b {
            return Value::Error(*e);
        }
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Pow => {
                let x = match self.to_number(a) {
                    Ok(x) => x,
                    Err(e) => return Value::Error(e),
                };
                let y = match self.to_number(b) {
                    Ok(y) => y,
                    Err(e) => return Value::Error(e),
                };
                let r = match op {
                    BinaryOp::Add => x + y,
                    BinaryOp::Sub => x - y,
                    BinaryOp::Mul => x * y,
                    BinaryOp::Div => {
                        if y == 0.0 {
                            return Value::Error(ErrorKind::Div0);
                        }
                        x / y
                    }
                    _ => {
                        if x == 0.0 && y == 0.0 {
                            return Value::Error(ErrorKind::Num);
                        }
                        if x == 0.0 && y < 0.0 {
                            return Value::Error(ErrorKind::Div0);
                        }
                        x.powf(y)
                    }
                };
                if r.is_finite() {
                    // Hide binary floating point noise the way Excel does:
                    // results within ~1e-15 relative of zero after +/- snap to 0.
                    let r = if matches!(op, BinaryOp::Add | BinaryOp::Sub)
                        && r != 0.0
                        && r.abs() < 1e-15 * x.abs().max(y.abs())
                    {
                        0.0
                    } else {
                        r
                    };
                    Value::Number(r)
                } else {
                    Value::Error(ErrorKind::Num)
                }
            }
            BinaryOp::Concat => {
                let x = match self.to_text(a) {
                    Ok(x) => x,
                    Err(e) => return Value::Error(e),
                };
                let y = match self.to_text(b) {
                    Ok(y) => y,
                    Err(e) => return Value::Error(e),
                };
                Value::from(x + &y)
            }
            BinaryOp::Eq => Value::Bool(compare_values(a, b) == Ordering::Equal),
            BinaryOp::Ne => Value::Bool(compare_values(a, b) != Ordering::Equal),
            BinaryOp::Lt => Value::Bool(compare_values(a, b) == Ordering::Less),
            BinaryOp::Gt => Value::Bool(compare_values(a, b) == Ordering::Greater),
            BinaryOp::Le => Value::Bool(compare_values(a, b) != Ordering::Greater),
            BinaryOp::Ge => Value::Bool(compare_values(a, b) != Ordering::Less),
        }
    }

    // ---- coercions -------------------------------------------------------

    /// Scalar → number. Empty is 0, booleans are 1/0, numeric text is parsed
    /// in the workbook locale; other text is `#VALUE!`. Arrays use their
    /// top-left element.
    pub fn to_number(&self, v: &Value) -> R<f64> {
        match v {
            Value::Empty => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(s) => crate::input::parse_numeric_text(s, self.locale()).ok_or(ErrorKind::Value),
            Value::Error(e) => Err(*e),
            Value::Array(a) => self.to_number(a.get(0, 0)),
        }
    }

    /// Scalar → text. Numbers use General formatting (15 significant
    /// digits), booleans their localized name.
    pub fn to_text(&self, v: &Value) -> R<String> {
        match v {
            Value::Empty => Ok(String::new()),
            Value::Number(n) => Ok(self.locale().number_to_text(*n)),
            Value::Bool(b) => Ok(self.locale().bool_name(*b).to_string()),
            Value::Text(s) => Ok(s.to_string()),
            Value::Error(e) => Err(*e),
            Value::Array(a) => self.to_text(a.get(0, 0)),
        }
    }

    /// Scalar → boolean. Numbers are true when non-zero; text "TRUE"/"FALSE"
    /// (or localized) is accepted; other text is `#VALUE!`.
    pub fn to_bool(&self, v: &Value) -> R<bool> {
        match v {
            Value::Empty => Ok(false),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Bool(b) => Ok(*b),
            Value::Text(s) => self.locale().parse_bool(s).ok_or(ErrorKind::Value),
            Value::Error(e) => Err(*e),
            Value::Array(a) => self.to_bool(a.get(0, 0)),
        }
    }

    /// Evaluate an argument as a number.
    pub fn num(&self, e: &Expr) -> R<f64> {
        self.to_number(&self.eval_scalar(e))
    }

    /// Evaluate an argument as a number truncated toward zero.
    pub fn int(&self, e: &Expr) -> R<i64> {
        let n = self.num(e)?;
        if n.abs() > 9.0e15 {
            return Err(ErrorKind::Num);
        }
        Ok(n.trunc() as i64)
    }

    /// Evaluate an argument as text.
    pub fn text(&self, e: &Expr) -> R<String> {
        self.to_text(&self.eval_scalar(e))
    }

    /// Evaluate an argument as a boolean.
    pub fn boolean(&self, e: &Expr) -> R<bool> {
        self.to_bool(&self.eval_scalar(e))
    }

    /// Optional numeric argument: `default` when absent or omitted.
    pub fn num_or(&self, args: &[Expr], index: usize, default: f64) -> R<f64> {
        match args.get(index) {
            None | Some(Expr::Missing) => Ok(default),
            Some(e) => self.num(e),
        }
    }

    /// Optional boolean argument: `default` when absent or omitted.
    pub fn bool_or(&self, args: &[Expr], index: usize, default: bool) -> R<bool> {
        match args.get(index) {
            None | Some(Expr::Missing) => Ok(default),
            Some(e) => self.boolean(e),
        }
    }

    /// Optional text argument: `default` when absent or omitted.
    pub fn text_or(&self, args: &[Expr], index: usize, default: &str) -> R<String> {
        match args.get(index) {
            None | Some(Expr::Missing) => Ok(default.to_string()),
            Some(e) => self.text(e),
        }
    }

    /// Flatten arguments into numbers with `SUM` semantics:
    /// * a direct scalar argument is coerced (numeric text and booleans
    ///   count; other text is `#VALUE!`);
    /// * inside ranges and arrays only numbers count (text, booleans and
    ///   blanks are skipped);
    /// * any error propagates.
    pub fn numbers(&self, args: &[Expr]) -> R<Vec<f64>> {
        let mut out = Vec::new();
        for arg in args {
            match self.eval(arg) {
                Value::Array(a) => {
                    for v in a.iter() {
                        match v {
                            Value::Number(n) => out.push(*n),
                            Value::Error(e) => return Err(*e),
                            _ => {}
                        }
                    }
                }
                Value::Empty => {
                    // A reference to a blank cell contributes nothing; an
                    // omitted argument counts as 0.
                    if matches!(arg, Expr::Missing) {
                        out.push(0.0);
                    }
                }
                Value::Number(n) => out.push(n),
                Value::Error(e) => return Err(e),
                v => {
                    if is_reference(arg) {
                        // Text / boolean read from a single cell is skipped,
                        // exactly like inside a range.
                        continue;
                    }
                    out.push(self.to_number(&v)?);
                }
            }
        }
        Ok(out)
    }

    /// Flatten arguments into scalar values (blank cells included as
    /// `Empty`), row-major, argument by argument. Errors are kept as values.
    pub fn flatten(&self, args: &[Expr]) -> Vec<Value> {
        let mut out = Vec::new();
        for arg in args {
            match self.eval(arg) {
                Value::Array(a) => out.extend(a.iter().cloned()),
                v => out.push(v),
            }
        }
        out
    }
}

/// True when the expression denotes cells (so text found there is "cell
/// content" rather than a literal typed into the formula).
pub fn is_reference(e: &Expr) -> bool {
    match e {
        Expr::Ref(_) | Expr::Name(_) => true,
        Expr::Paren(inner) => is_reference(inner),
        _ => false,
    }
}

/// Turn a function result into a value (`Err` becomes an error value).
pub fn wrap(r: R<Value>) -> Value {
    match r {
        Ok(v) => v,
        Err(e) => Value::Error(e),
    }
}

fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Empty | Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        Value::Array(_) => 4,
    }
}

/// Case-insensitive text comparison as used by `=`, `<`, sorting and lookups.
pub fn compare_text(a: &str, b: &str) -> Ordering {
    let mut x = a.chars().flat_map(|c| c.to_lowercase());
    let mut y = b.chars().flat_map(|c| c.to_lowercase());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => match p.cmp(&q) {
                Ordering::Equal => {}
                o => return o,
            },
        }
    }
}

/// Spreadsheet ordering of scalars: numbers < text < booleans; a blank
/// compares as 0, "" or FALSE depending on the other side. Text compares
/// case-insensitively.
pub fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Empty, Value::Empty) => Ordering::Equal,
        (Value::Empty, Value::Text(s)) => {
            if s.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        (Value::Text(s), Value::Empty) => {
            if s.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Value::Empty, Value::Bool(b)) => false.cmp(b),
        (Value::Bool(a), Value::Empty) => a.cmp(&false),
        (Value::Empty, Value::Number(n)) => 0.0f64.partial_cmp(n).unwrap_or(Ordering::Equal),
        (Value::Number(n), Value::Empty) => n.partial_cmp(&0.0).unwrap_or(Ordering::Equal),
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => compare_text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Error(x), Value::Error(y)) => (*x as u8).cmp(&(*y as u8)),
        _ => type_rank(a).cmp(&type_rank(b)),
    }
}
