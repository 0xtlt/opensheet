//! Formula evaluation.
//!
//! A [`Calc`] is one recalculation pass over an immutable [`Workbook`]:
//! formula results are memoised in a side table and written back to the
//! workbook once the pass is done. [`EvalCtx`] is the handle functions use
//! to evaluate their arguments.

use crate::ast::{BinaryOp, CellRef, Expr, RefExpr, RefKind, UnaryOp};
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

/// How deep defined names may refer to one another. A name defined in
/// terms of itself would otherwise never finish evaluating.
const MAX_NAME_DEPTH: u32 = 64;

/// Functions that treat all their arguments (from the given index on) as
/// one pool of cells. A reference with several areas — a union such as
/// `(A1:A3,C1:C3)`, a 3-D reference such as `Sheet1:Sheet3!A1` — can stand
/// there, and is handed over as one argument per area. Anywhere else such
/// a reference is `#VALUE!`, except in the functions reading a whole
/// argument through `EvalCtx::numbers` / `flatten` (`SMALL`, `RANK`, ...).
fn takes_areas(name: &str) -> Option<usize> {
    match name {
        "SUM" | "SUMSQ" | "PRODUCT" | "AVERAGE" | "AVERAGEA" | "COUNT" | "COUNTA" | "MAX"
        | "MAXA" | "MIN" | "MINA" | "MEDIAN" | "MODE" | "MODE.SNGL" | "MODE.MULT" | "STDEV"
        | "STDEV.S" | "STDEV.P" | "STDEVP" | "STDEVA" | "STDEVPA" | "VAR" | "VAR.S" | "VAR.P"
        | "VARP" | "VARA" | "VARPA" | "AVEDEV" | "DEVSQ" | "GEOMEAN" | "HARMEAN" | "KURT"
        | "SKEW" | "SKEW.P" | "AND" | "OR" | "XOR" | "CONCAT" | "HSTACK" | "VSTACK" => Some(0),
        "NPV" | "SUBTOTAL" => Some(1),
        "TEXTJOIN" => Some(2),
        _ => None,
    }
}

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
    name_depth: Cell<u32>,
    now: f64,
}

impl<'a> Calc<'a> {
    fn new(wb: &'a Workbook, hints: Vec<Vec<SpillRect>>, now: f64) -> Calc<'a> {
        let mut extents = Vec::with_capacity(wb.sheets.len());
        for sheet in wb.sheets.iter() {
            // Typed cells only: spilled arrays extend the extent as they are
            // registered during the pass. Counting last pass's spills here
            // would let `=A:A` grow a little on every recalculation.
            extents.push(sheet.cell_extent());
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
            name_depth: Cell::new(0),
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
                    cache.insert(
                        (si, sp.row, sp.col),
                        Slot::Done(Value::Array(sp.array.clone())),
                    );
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
            // Blank elements of an array result read as 0, like a formula
            // returning a blank cell.
            if a.len() > 1 && a.iter().any(|x| x.is_empty()) {
                let mut filled = (**a).clone();
                for x in &mut filled.data {
                    if x.is_empty() {
                        *x = Value::Number(0.0);
                    }
                }
                v = Value::array(filled);
            }
        }
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

    /// The part of a range that is materialised: large ranges (whole
    /// columns / rows above all) are clamped to the part of the sheet
    /// holding data; ordinary ranges keep their shape.
    fn clamp_rect(&self, reference: &Reference) -> Rect {
        let rect = reference.rect;
        if reference.sheet >= self.wb.sheets.len() {
            return rect;
        }
        self.ensure_hinted(reference.sheet, &rect);
        if rect.rows() as u64 * rect.cols() as u64 <= 200_000 {
            return rect;
        }
        let (er, ec) = self.extents.borrow()[reference.sheet];
        Rect::new(
            rect.r0,
            rect.c0,
            rect.r1.min(er.max(rect.r0)),
            rect.c1.min(ec.max(rect.c0)),
        )
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
        let clamped = self.clamp_rect(reference);
        let (r1, c1) = (clamped.r1, clamped.c1);
        let rows = (r1 - rect.r0 + 1) as usize;
        let cols = (c1 - rect.c0 + 1) as usize;
        let mut arr = Array::new(rows, cols);

        let mut fill = |r: u32, c: u32, content: &Content| {
            let v = match content {
                Content::Formula(expr) => self
                    .formula_value(reference.sheet, r, c, expr)
                    .into_scalar(),
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
            self.range_cache
                .borrow_mut()
                .insert(*reference, arr.clone());
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
            Expr::Table(_) | Expr::Spill(_) => match self.eval_ref(e) {
                Err(err) => Value::Error(err),
                Ok(reference) => self.deref(&reference),
            },
            Expr::Ref3D(_) | Expr::Union(_) | Expr::Intersect(..) => {
                match self.eval_areas(e).as_deref() {
                    Err(err) => Value::Error(*err),
                    Ok([reference]) => self.deref(reference),
                    // Several areas have no value as a whole.
                    Ok(_) => Value::Error(ErrorKind::Value),
                }
            }
            Expr::RefError(_) => Value::Error(ErrorKind::Ref),
            Expr::Name(name) => {
                wrap(self.with_name(self.sheet, name, |ctx, def| Ok(ctx.eval(def))))
            }
            Expr::SheetName(sheet, name) => wrap(match self.calc.wb.sheet_index(sheet) {
                Some(from) => self.with_name(from, name, |ctx, def| Ok(ctx.eval(def))),
                None => Err(ErrorKind::Ref),
            }),
            Expr::Implicit(inner) => match self.eval_ref(inner) {
                // Of a range, the cell on the formula's row or column.
                Ok(reference) => match self.implicit_cell(&reference) {
                    Ok(cell) => self.deref(&cell),
                    Err(err) => Value::Error(err),
                },
                // Of an array, the first value.
                Err(_) => self.eval(inner).into_scalar(),
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
            Expr::Name(name) => self.with_name(self.sheet, name, |ctx, def| ctx.eval_ref(def)),
            Expr::SheetName(sheet, name) => {
                let from = self.calc.wb.sheet_index(sheet).ok_or(ErrorKind::Ref)?;
                self.with_name(from, name, |ctx, def| ctx.eval_ref(def))
            }
            Expr::Table(t) => {
                crate::structured::resolve(self.calc.wb, t, self.sheet, self.row, self.col)
            }
            Expr::Spill(anchor) => self.spilled_ref(anchor),
            Expr::Implicit(inner) => self.implicit_cell(&self.eval_ref(inner)?),
            Expr::Ref3D(_) | Expr::Union(_) | Expr::Intersect(..) => {
                match self.eval_areas(e)?.as_slice() {
                    [reference] => Ok(*reference),
                    _ => Err(ErrorKind::Value),
                }
            }
            Expr::RefError(_) => Err(ErrorKind::Ref),
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

    /// Evaluate an argument that must be a reference, which may be made of
    /// several areas: a union (`(A1:A3,C1:C3)`), a 3-D reference (one area
    /// per sheet), an intersection. `#NULL!` when an intersection is empty,
    /// `#VALUE!` for a union across sheets.
    pub fn eval_areas(&self, e: &Expr) -> R<Vec<Reference>> {
        match e {
            Expr::Paren(inner) => self.eval_areas(inner),
            Expr::Ref3D(r) => {
                let wb = self.calc.wb;
                let first = wb.sheet_index(&r.first).ok_or(ErrorKind::Ref)?;
                let last = wb.sheet_index(&r.last).ok_or(ErrorKind::Ref)?;
                let rect = r.cells.rect();
                Ok((first.min(last)..=first.max(last))
                    .map(|sheet| Reference { sheet, rect })
                    .collect())
            }
            Expr::Union(items) => {
                let mut areas: Vec<Reference> = Vec::new();
                for item in items {
                    areas.extend(self.eval_areas(item)?);
                }
                // As in Excel, the areas of a union lie on one sheet.
                if areas.iter().any(|area| area.sheet != areas[0].sheet) {
                    return Err(ErrorKind::Value);
                }
                Ok(areas)
            }
            Expr::Intersect(a, b) => {
                let left = self.eval_areas(a)?;
                let right = self.eval_areas(b)?;
                let mut areas = Vec::new();
                for x in &left {
                    for y in right.iter().filter(|y| y.sheet == x.sheet) {
                        if let Some(rect) = x.rect.intersection(&y.rect) {
                            areas.push(Reference {
                                sheet: x.sheet,
                                rect,
                            });
                        }
                    }
                }
                if areas.is_empty() {
                    return Err(ErrorKind::Null);
                }
                Ok(areas)
            }
            Expr::Name(name) => self.with_name(self.sheet, name, |ctx, def| ctx.eval_areas(def)),
            Expr::SheetName(sheet, name) => {
                let from = self.calc.wb.sheet_index(sheet).ok_or(ErrorKind::Ref)?;
                self.with_name(from, name, |ctx, def| ctx.eval_areas(def))
            }
            _ => Ok(vec![self.eval_ref(e)?]),
        }
    }

    /// True when the expression may be a reference with several areas
    /// (told from its shape, without evaluating anything).
    fn may_have_areas(&self, e: &Expr) -> bool {
        fn shape(e: &Expr) -> bool {
            match e {
                Expr::Ref3D(_) | Expr::Union(_) | Expr::Intersect(..) => true,
                Expr::Paren(inner) => shape(inner),
                _ => false,
            }
        }
        let wb = self.calc.wb;
        match e {
            Expr::Paren(inner) => self.may_have_areas(inner),
            Expr::Name(name) => wb
                .lookup_name(self.sheet, name)
                .is_some_and(|(_, def)| shape(&def.expr)),
            Expr::SheetName(sheet, name) => wb
                .sheet_index(sheet)
                .and_then(|from| wb.lookup_name(from, name))
                .is_some_and(|(_, def)| shape(&def.expr)),
            other => shape(other),
        }
    }

    /// What an aggregate reads from one argument: its value, or one value
    /// per area when it is a reference with several areas.
    fn area_values(&self, arg: &Expr) -> Vec<Value> {
        if !self.may_have_areas(arg) {
            return vec![self.eval(arg)];
        }
        match self.eval_areas(arg) {
            Ok(areas) => areas.iter().map(|area| self.deref(area)).collect(),
            Err(err) => vec![Value::Error(err)],
        }
    }

    /// A resolved area as an expression (absolute, with its sheet name).
    fn area_expr(&self, area: &Reference) -> Expr {
        let corner = |row: u32, col: u32| CellRef {
            row,
            col,
            abs_row: true,
            abs_col: true,
        };
        let rect = area.rect;
        Expr::Ref(RefExpr {
            sheet: self
                .calc
                .wb
                .sheets
                .get(area.sheet)
                .map(|s| Arc::from(s.name.as_str())),
            start: corner(rect.r0, rect.c0),
            end: (!rect.is_single()).then(|| corner(rect.r1, rect.c1)),
            kind: RefKind::Cells,
        })
    }

    /// Evaluate with `f` what `name` stands for in a formula on `sheet`:
    /// the definition of the sheet's own name, else of the workbook's, else
    /// the data rows of the table by that name. `#NAME?` when there is
    /// none. The relative references of a name are taken from the cell
    /// being calculated.
    fn with_name<T>(
        &self,
        sheet: usize,
        name: &str,
        f: impl FnOnce(&EvalCtx<'a>, &Expr) -> R<T>,
    ) -> R<T> {
        let calc = self.calc;
        let ctx = EvalCtx { sheet, ..*self };
        if let Some((_, def)) = calc.wb.lookup_name(sheet, name) {
            if calc.name_depth.get() >= MAX_NAME_DEPTH {
                return Err(ErrorKind::Name);
            }
            calc.name_depth.set(calc.name_depth.get() + 1);
            let out = if def.expr.has_relative_refs() {
                f(&ctx, &def.expr_at(self.row, self.col))
            } else {
                f(&ctx, &def.expr)
            };
            calc.name_depth.set(calc.name_depth.get() - 1);
            return out;
        }
        if let Some((table_sheet, table)) = calc.wb.table_by_name(name) {
            let data = table.data_range().ok_or(ErrorKind::Ref)?;
            return f(&ctx, &self.area_expr(&Reference::new(table_sheet, data)));
        }
        Err(ErrorKind::Name)
    }

    /// `A1#`: the range filled by the array spilling from a cell. `#REF!`
    /// when nothing spills from it.
    fn spilled_ref(&self, anchor: &Expr) -> R<Reference> {
        let cell = self.eval_ref(anchor)?;
        if !cell.is_single() {
            return Err(ErrorKind::Ref);
        }
        let (sheet, row, col) = (cell.sheet, cell.rect.r0, cell.rect.c0);
        // The anchor must have been calculated for its spill to be known.
        self.calc.cell_value(sheet, row, col);
        match self.calc.spill_at(sheet, row, col) {
            Some(spill) if spill.anchor == (row, col) => Ok(Reference::new(sheet, spill.rect)),
            _ => Err(ErrorKind::Ref),
        }
    }

    /// The cell `@` picks out of a range: the one on the row of the formula
    /// for cells in a column, on its column for cells in a row. `#VALUE!`
    /// when the formula is not level with the range.
    fn implicit_cell(&self, reference: &Reference) -> R<Reference> {
        let rect = reference.rect;
        if rect.is_single() {
            return Ok(*reference);
        }
        if rect.cols() == 1 && self.row >= rect.r0 && self.row <= rect.r1 {
            return Ok(Reference::cell(reference.sheet, self.row, rect.c0));
        }
        if rect.rows() == 1 && self.col >= rect.c0 && self.col <= rect.c1 {
            return Ok(Reference::cell(reference.sheet, rect.r0, self.col));
        }
        Err(ErrorKind::Value)
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

    /// The rectangle `range_values` / `eval` actually deliver for a
    /// reference (whole columns and rows are clamped to the used sheet).
    pub fn clamped(&self, reference: &Reference) -> Rect {
        self.calc.clamp_rect(reference)
    }

    /// Scalar value of one cell.
    pub fn cell_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.calc.cell_value(sheet, row, col)
    }

    /// The formula stored in a cell, if any.
    pub fn formula_at(&self, sheet: usize, row: u32, col: u32) -> Option<&'a Expr> {
        match &self
            .calc
            .wb
            .sheets
            .get(sheet)?
            .cells
            .get(&(row, col))?
            .content
        {
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
            Expr::Ref(_)
            | Expr::Name(_)
            | Expr::SheetName(..)
            | Expr::Table(_)
            | Expr::Spill(_) => self.eval_ref(&expr).map_err(|_| ErrorKind::Ref),
            _ => Err(ErrorKind::Ref),
        }
    }

    fn call(&self, def: &FunctionDef, args: &[Expr]) -> Value {
        if args.len() < def.min_args as usize
            || (def.max_args != u8::MAX && args.len() > def.max_args as usize)
        {
            return Value::Error(ErrorKind::Value);
        }
        if def.kind == Kind::Raw
            && args.iter().any(|a| self.may_have_areas(a))
            && let Some(first) = takes_areas(def.name)
        {
            // One argument per area: `SUM((A1:A3,C1:C3))` is `SUM(A1:A3,C1:C3)`.
            let mut expanded: Vec<Expr> = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                if i < first || !self.may_have_areas(arg) {
                    expanded.push(arg.clone());
                    continue;
                }
                match self.eval_areas(arg) {
                    Ok(areas) => expanded.extend(areas.iter().map(|area| self.area_expr(area))),
                    Err(err) => expanded.push(Expr::Error(err)),
                }
            }
            return wrap((def.func)(self, &expanded));
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
            Value::Text(s) => {
                crate::input::parse_numeric_text(s, self.locale()).ok_or(ErrorKind::Value)
            }
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
            for value in self.area_values(arg) {
                match value {
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
                    // Text / boolean read from a single cell is skipped,
                    // exactly like inside a range.
                    _ if is_reference(arg) => {}
                    v => out.push(self.to_number(&v)?),
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
            for value in self.area_values(arg) {
                match value {
                    Value::Array(a) => out.extend(a.iter().cloned()),
                    v => out.push(v),
                }
            }
        }
        out
    }
}

/// True when the expression denotes cells (so text found there is "cell
/// content" rather than a literal typed into the formula).
pub fn is_reference(e: &Expr) -> bool {
    match e {
        Expr::Ref(_)
        | Expr::Ref3D(_)
        | Expr::Name(_)
        | Expr::SheetName(..)
        | Expr::Table(_)
        | Expr::Spill(_)
        | Expr::Intersect(..)
        | Expr::Union(_) => true,
        Expr::Paren(inner) | Expr::Implicit(inner) => is_reference(inner),
        // OFFSET, INDIRECT, INDEX... hand back cells too.
        Expr::Func(name, _) => functions::lookup(name).is_some_and(|def| def.ref_func.is_some()),
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
