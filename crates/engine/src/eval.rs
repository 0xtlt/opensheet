//! Formula evaluation.
//!
//! A [`Calc`] is one calculation pass over an immutable [`Workbook`]: the
//! results of the formulas it calculates are memoised in a side table and
//! written back to the workbook once the pass is done. A pass calculates
//! either every formula ([`run_full`]) or only the formulas downstream of
//! what changed, the others keeping the value stored in their cell
//! ([`run_dirty`]). An [`EvalSession`] evaluates loose expressions against
//! the stored values. [`EvalCtx`] is the handle functions use to evaluate
//! their arguments.

use crate::ast::{BinaryOp, CellRef, Expr, RefExpr, RefKind, UnaryOp};
use crate::deps::{self, DepGraph, FastMap, FastSet, Pos, Precedents, pos, unpos};
use crate::functions::{self, FunctionDef, Kind};
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, Reference};
use crate::value::{Array, ErrorKind, R, Value};
use crate::workbook::{Cell as SheetCell, Content, Spill, Workbook};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::rc::Rc;
use std::sync::Arc;

type CellKey = (usize, u32, u32);

enum Slot {
    /// To be calculated in this pass.
    Pending,
    Computing,
    Done(Value),
}

/// Where a formula stands in the pass.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// Not calculated yet.
    Due,
    /// Being calculated, further up the stack.
    Busy,
    /// Calculated, or not part of this pass: its value can be read.
    Known,
}

/// Where a dynamic array landed: anchor cell plus covered rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpillRect {
    pub anchor: (u32, u32),
    pub rect: Rect,
}

impl SpillRect {
    fn of(spill: &Spill) -> SpillRect {
        SpillRect {
            anchor: (spill.row, spill.col),
            rect: Rect::new(
                spill.row,
                spill.col,
                spill.row + spill.array.rows as u32 - 1,
                spill.col + spill.array.cols as u32 - 1,
            ),
        }
    }
}

/// A spilled array as a pass sees it.
#[derive(Clone)]
struct Live {
    at: SpillRect,
    array: Arc<Array>,
}

/// A formula to calculate.
#[derive(Clone, Copy)]
struct Root<'a> {
    at: Pos,
    cell: &'a SheetCell,
    expr: &'a Expr,
}

fn root(wb: &Workbook, at: Pos) -> Option<Root<'_>> {
    let (sheet, row, col) = unpos(at);
    let cell = wb.sheets.get(sheet)?.cells.get(&(row, col))?;
    match &cell.content {
        Content::Formula(expr) => Some(Root { at, cell, expr }),
        _ => None,
    }
}

/// Outcome of a recalculation.
pub struct CalcResult {
    /// Value of every formula cell that was calculated (arrays are kept
    /// whole on the anchor).
    pub values: Vec<(CellKey, Value)>,
    /// Spilled arrays per sheet, sorted by anchor.
    pub spills: Vec<Vec<(SpillRect, Arc<Array>)>>,
    pub circular: bool,
    /// The cells of the circular references that were found, in sheet
    /// order. Empty with iterative calculation, which has no such error.
    pub circular_cells: Vec<CellKey>,
    /// The outcome depends on the order in which formulas were calculated
    /// (a circular reference, two arrays spilling over the same cells, a
    /// formula reading the area of its own array), or the spilled arrays
    /// did not find their place in the allowed number of passes. Such a
    /// workbook is calculated in full, in sheet order, every time.
    pub(crate) unsettled: bool,
    /// Arrays that could not spill because a cell in their way holds
    /// something: they are calculated again when one of those cells
    /// changes.
    pub(crate) blocked: Vec<(usize, SpillRect)>,
    /// Areas that arrays spilled into or withdrew from.
    pub(crate) moved: Vec<(usize, Rect)>,
    /// A formula read a cell that is not among its precedents (see
    /// `deps::set_audit`).
    pub(crate) audit: Option<String>,
}

/// What a recalculation is asked to do besides calculating.
#[derive(Clone, Copy)]
pub(crate) struct Options {
    /// Iterative calculation of circular references: maximum number of
    /// iterations and maximum change.
    pub iterate: Option<(u32, f64)>,
    pub audit: bool,
    /// Recursion allowed between formulas reading one another, see
    /// `SETTLE_DEPTH`.
    pub settle_depth: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            iterate: None,
            audit: false,
            settle_depth: SETTLE_DEPTH,
        }
    }
}

/// Which formulas a partial recalculation covers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Everything downstream of the changes.
    Workbook,
    /// The same, on one sheet only ("Calculate Sheet").
    Sheet(usize),
    /// The changed formulas themselves and no other: what manual
    /// calculation does when a formula is entered.
    Entered,
}

/// What changed since the workbook was last calculated.
pub(crate) struct Changes<'c> {
    /// Cells whose content changed.
    pub cells: &'c [Pos],
    /// Cells whose value may have changed in another way.
    pub rects: &'c [(usize, Rect)],
    /// Rows were hidden, shown or filtered.
    pub visibility: bool,
    /// Per sheet, whether the last row / last column holding typed data
    /// moved, and where they are now.
    pub stretched: &'c [(bool, bool)],
    pub typed: &'c [(u32, u32)],
    pub blocked: &'c [(usize, SpillRect)],
}

/// Recursion allowed between formulas reading one another. Deeper than
/// that, the precedents of a formula are calculated ahead of it in a loop
/// (see `Calc::settle_precedents`), so that a long chain of formulas costs
/// no stack.
pub(crate) const SETTLE_DEPTH: u32 = 128;

/// Recursion limit where that does not apply: references worked out at
/// calculation time (`INDIRECT`) and circular references.
const MAX_DEPTH: u32 = 20_000;

/// How deep defined names may refer to one another. A name defined in
/// terms of itself would otherwise never finish evaluating.
pub(crate) const MAX_NAME_DEPTH: u32 = 64;

/// A range of more cells than this is read up to the last row and column
/// holding data.
pub(crate) const CLAMP_CELLS: u64 = 200_000;

/// How many times a calculation is repeated for spilled arrays to find
/// their place.
const MAX_PASSES: u32 = 4;

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

/// State of an iterative calculation of circular references.
#[derive(Default)]
struct Iteration {
    /// Results of the previous iteration: what a formula reads when it
    /// refers back to a cell that is being calculated.
    previous: RefCell<FastMap<Pos, Value>>,
    /// The cells that were read that way.
    targets: RefCell<FastSet<Pos>>,
}

/// Checks that formulas only read their precedents (`deps::set_audit`).
#[derive(Default)]
struct Audit {
    /// The formula being calculated.
    reader: Cell<Option<Pos>>,
    precedents: RefCell<FastMap<Pos, Rc<Precedents>>>,
    violation: RefCell<Option<String>>,
}

pub struct Calc<'a> {
    wb: &'a Workbook,
    /// A full pass calculates every formula. Otherwise only the formulas
    /// given a slot in `cache` are calculated; the others are read from
    /// their cell.
    all: bool,
    cache: RefCell<FastMap<Pos, Slot>>,
    /// The formulas calculated, in the order they were: results are
    /// written back to the sheets in that order, which is close to the
    /// order of the cells.
    order: RefCell<Vec<Pos>>,
    /// The spilled arrays in place, per sheet: those of the formulas
    /// calculated so far and, in a partial pass, those of the formulas
    /// that are not calculated.
    spills: RefCell<Vec<Vec<Live>>>,
    /// Where the arrays of the formulas still to calculate spilled last
    /// time: reading one of these cells calculates the formula first.
    hints: Vec<Vec<SpillRect>>,
    /// Arrays placed since the pass (or the round of a partial pass) began.
    placed: RefCell<Vec<(usize, SpillRect)>>,
    blocked: RefCell<Vec<(usize, SpillRect)>>,
    /// Last row and last column holding typed data per sheet, looked up
    /// when first needed.
    typed: RefCell<Vec<(Option<u32>, Option<u32>)>>,
    /// The large ranges read by the formula being calculated, once one of
    /// them was read (see `clamp_rect`).
    large: RefCell<Option<Rc<Vec<(usize, Rect)>>>>,
    /// Ranges already materialised, by reference and by where they were
    /// clamped.
    range_cache: RefCell<FastMap<(Reference, u32, u32), Arc<Array>>>,
    /// A formula read a cell that was being calculated.
    cycles: Cell<bool>,
    /// The formulas being calculated, outermost first, and the cells found
    /// to be part of a circular reference.
    path: RefCell<Vec<Pos>>,
    looped: RefCell<FastSet<Pos>>,
    unsettled: Cell<bool>,
    depth: Cell<u32>,
    settle_depth: u32,
    name_depth: Cell<u32>,
    now: f64,
    iteration: Option<Iteration>,
    audit: Option<Audit>,
}

/// Recalculate every formula in the workbook, starting from the spilled
/// arrays of the last calculation.
///
/// Formulas are calculated in sheet order, each one calculating first the
/// formulas it reads. An array that spills changes what the cells it covers
/// read as, so the pass is repeated until the arrays land where the pass
/// expected them (`graph` is only needed for iterative calculation).
pub(crate) fn run_full(wb: &Workbook, graph: Option<&DepGraph>, options: &Options) -> CalcResult {
    full(wb, graph, options, crate::datetime::now_serial(), &[])
}

/// `run_full` at a given time, with the results of some formulas taken as
/// given rather than calculated (`pinned`).
fn full(
    wb: &Workbook,
    graph: Option<&DepGraph>,
    options: &Options,
    now: f64,
    pinned: &[(Pos, &Value)],
) -> CalcResult {
    let mut hints: Vec<Vec<SpillRect>> = wb
        .sheets
        .iter()
        .map(|s| {
            let mut list: Vec<SpillRect> = s.spills.iter().map(SpillRect::of).collect();
            list.sort_unstable_by_key(|s| s.anchor);
            list
        })
        .collect();
    let mut pass = 0;
    // The last pass that found a circular reference, while passes go on.
    let mut circular: Option<CalcResult> = None;
    loop {
        let mut calc = Calc::new(wb, true, now, options);
        calc.hints = hints.clone();
        for (at, value) in pinned {
            calc.pin(*at, value);
        }
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for ((row, col), cell) in sheet.cells.iter() {
                if let Content::Formula(expr) = &cell.content {
                    calc.formula_value(si, *row, *col, cell, expr);
                }
            }
        }
        if let Some((iterations, tolerance)) = options.iterate {
            calc.iterate(graph, iterations, tolerance);
        }
        let found = calc.spill_rects();
        pass += 1;
        let settled = found == hints;
        let last = settled || pass >= MAX_PASSES;
        if !last && !calc.cycles.get() {
            hints = found;
            continue;
        }
        let mut result = calc.finish(Vec::new());
        if settled {
            return result;
        }
        result.unsettled = true;
        if !last {
            circular = Some(result);
            hints = found;
            continue;
        }
        // The arrays never came to rest. A formula reading the cells its
        // own array spills into does that: it is a circular reference
        // when the array is there, and none once the array has given way.
        // The answer is then the circular one, wherever the passes began.
        return match circular {
            Some(found) if !result.circular => found,
            _ => result,
        };
    }
}

/// Recalculate the formulas downstream of `changes`, every other formula
/// keeping its value. `None` when the outcome may depend on the order of
/// calculation: the workbook must then be calculated in full, which
/// settles that order. With a `scope` narrower than the workbook the pass
/// is not exhaustive anyway and always succeeds.
///
/// The formulas to calculate are those reading a changed cell, those
/// reading these formulas, and so on. Spilled arrays add to that: a cell
/// typed into the way of an array (or out of it) calculates its anchor,
/// and an array that moves calculates what reads the cells it covered. An
/// array that grows changes cells the formulas calculated so far may have
/// read as blank; they are calculated again in a further round.
pub(crate) fn run_dirty(
    wb: &Workbook,
    graph: Option<&DepGraph>,
    changes: &Changes,
    scope: Scope,
    options: &Options,
) -> Option<CalcResult> {
    dirty(
        wb,
        graph,
        changes,
        scope,
        options,
        crate::datetime::now_serial(),
    )
}

/// `run_dirty`, then a full recalculation of the same workbook at the same
/// time: `Err` says where the two differ. Formulas drawing random numbers
/// cannot be compared; the full recalculation takes their results from the
/// incremental one, and everything reading them is compared.
pub(crate) fn run_dirty_checked(
    wb: &Workbook,
    graph: &DepGraph,
    changes: &Changes,
    options: &Options,
) -> Result<Option<CalcResult>, String> {
    let now = crate::datetime::now_serial();
    let Some(result) = dirty(wb, Some(graph), changes, Scope::Workbook, options, now) else {
        return Ok(None);
    };
    let random = |at: &CellKey| {
        root(wb, pos(at.0, at.1, at.2))
            .is_some_and(|f| deps::precedents(wb, at.0, at.1, at.2, f.expr).random)
    };
    let pinned: Vec<(Pos, &Value)> = result
        .values
        .iter()
        .filter(|(at, _)| random(at))
        .map(|(at, value)| (pos(at.0, at.1, at.2), value))
        .collect();
    let expected = full(wb, Some(graph), options, now, &pinned);
    let at = |key: &CellKey| {
        let name = crate::refs::cell_name(key.1, key.2);
        format!(
            "{}!{name} ({})",
            wb.sheets[key.0].name,
            wb.edit_text(key.0, key.1, key.2)
        )
    };
    if expected.circular || expected.unsettled {
        return Err(
            "a full recalculation finds a circular reference or colliding arrays, \
                    the incremental one does not"
                .to_string(),
        );
    }
    let found: FastMap<CellKey, &Value> = result.values.iter().map(|(k, v)| (*k, v)).collect();
    for (key, value) in &expected.values {
        let stored = || wb.cell(key.0, key.1, key.2).map(|c| &c.value);
        let Some(got) = found.get(key).copied().or_else(stored) else {
            continue;
        };
        let (got, value) = (got.clone().into_scalar(), value.clone().into_scalar());
        if got != value {
            return Err(format!(
                "incremental recalculation gives {got:?} in {}, a full one {value:?}",
                at(key)
            ));
        }
    }
    for (sheet, (got, wanted)) in result.spills.iter().zip(&expected.spills).enumerate() {
        let same = got.len() == wanted.len()
            && got
                .iter()
                .zip(wanted)
                .all(|(a, b)| a.0 == b.0 && a.1 == b.1);
        if !same {
            let rects = |list: &[(SpillRect, Arc<Array>)]| {
                let names: Vec<String> = list.iter().map(|(s, _)| s.rect.name()).collect();
                names.join(", ")
            };
            return Err(format!(
                "incremental recalculation spills arrays over [{}] of {}, a full one over [{}], \
                 or with other values",
                rects(got),
                wb.sheets[sheet].name,
                rects(wanted)
            ));
        }
    }
    Ok(Some(result))
}

fn dirty(
    wb: &Workbook,
    graph: Option<&DepGraph>,
    changes: &Changes,
    scope: Scope,
    options: &Options,
    now: f64,
) -> Option<CalcResult> {
    let mut calc = Calc::new(wb, false, now, options);
    *calc.spills.get_mut() = stored_spills(wb);
    *calc.blocked.get_mut() = changes.blocked.to_vec();
    for (typed, known) in calc.typed.get_mut().iter_mut().zip(changes.typed) {
        *typed = (Some(known.0), Some(known.1));
    }
    let mut moved: Vec<(usize, Rect)> = Vec::new();
    let mut queue = Queue::default();
    // Anchors of the arrays a changed cell lies in the way of.
    let mut anchors: FastSet<Pos> = FastSet::default();

    // An array whose anchor no longer holds a formula is gone.
    for (si, lives) in calc.spills.get_mut().iter_mut().enumerate() {
        lives.retain(|live| {
            let kept = root(wb, pos(si, live.at.anchor.0, live.at.anchor.1)).is_some();
            if !kept {
                moved.push((si, live.at.rect));
            }
            kept
        });
    }
    for &cell in changes.cells {
        queue.push(cell);
        let (sheet, row, col) = unpos(cell);
        // What the cell holds now may block an array, or let one spill.
        let lives = calc.spills.get_mut().get(sheet).into_iter().flatten();
        let placed = lives.map(|live| &live.at);
        let blocked = changes.blocked.iter().filter(|(s, _)| *s == sheet);
        for spill in placed.chain(blocked.map(|(_, spill)| spill)) {
            if spill.rect.contains(row, col) {
                let anchor = pos(sheet, spill.anchor.0, spill.anchor.1);
                anchors.insert(anchor);
                queue.push(anchor);
            }
        }
    }
    if let (Some(graph), true) = (graph, scope != Scope::Entered) {
        for &cell in changes.cells {
            graph.readers_of_cell(cell, &mut |p| queue.push(p));
        }
        for (sheet, rect) in changes.rects.iter().chain(&moved) {
            graph.readers_of_rect(*sheet, rect, &mut |p| queue.push(p));
        }
        for p in graph.volatile() {
            queue.push(*p);
        }
        if changes.visibility {
            for p in graph.visibility() {
                queue.push(*p);
            }
        }
        for (sheet, (rows, cols)) in changes.stretched.iter().enumerate() {
            let tall = graph.tall(sheet).iter().filter(|_| *rows);
            let wide = graph.wide(sheet).iter().filter(|_| *cols);
            for p in tall.chain(wide) {
                queue.push(*p);
            }
        }
    }

    for _ in 0..MAX_PASSES {
        let mut roots = calc.downstream(queue, graph.filter(|_| scope != Scope::Entered));
        if let Scope::Sheet(sheet) = scope {
            // An array is never left spilling over a cell that holds
            // something, whatever its sheet.
            roots.retain(|r| unpos(r.at).0 == sheet || anchors.contains(&r.at));
        }
        calc.begin(&roots);
        for r in &roots {
            let (sheet, row, col) = unpos(r.at);
            calc.formula_value(sheet, row, col, r.cell, r.expr);
        }
        if scope == Scope::Workbook && (calc.unsettled.get() || calc.cycles.get()) {
            return None;
        }
        // Cells that were blank and now show part of an array: whoever
        // read them must be calculated again.
        let grown = calc.spread(&mut moved);
        queue = Queue::default();
        if let (Some(graph), false) = (graph, grown.is_empty() || scope == Scope::Entered) {
            for (sheet, rect) in &grown {
                graph.readers_of_rect(*sheet, rect, &mut |p| queue.push(p));
            }
            for p in graph.dynamic() {
                queue.push(*p);
            }
        }
        if queue.list.is_empty() {
            return Some(calc.finish(moved));
        }
    }
    (scope != Scope::Workbook).then(|| calc.finish(moved))
}

/// Formulas waiting to be calculated, in the order they were found.
#[derive(Default)]
struct Queue {
    seen: FastSet<Pos>,
    list: Vec<Pos>,
}

impl Queue {
    fn push(&mut self, at: Pos) {
        if self.seen.insert(at) {
            self.list.push(at);
        }
    }
}

fn stored_spills(wb: &Workbook) -> Vec<Vec<Live>> {
    wb.sheets
        .iter()
        .map(|s| {
            let mut lives: Vec<Live> = s
                .spills
                .iter()
                .map(|sp| Live {
                    at: SpillRect::of(sp),
                    array: sp.array.clone(),
                })
                .collect();
            lives.sort_unstable_by_key(|l| l.at.anchor);
            lives
        })
        .collect()
}

/// A scalar as it compares between two iterations: the change of a number,
/// nothing for equal values, no convergence otherwise.
fn change(before: &Value, after: &Value) -> f64 {
    match (before, after) {
        (Value::Number(a), Value::Number(b)) => (a - b).abs(),
        (a, b) if a == b => 0.0,
        _ => f64::INFINITY,
    }
}

impl<'a> Calc<'a> {
    fn new(wb: &'a Workbook, all: bool, now: f64, options: &Options) -> Calc<'a> {
        let sheets = wb.sheets.len();
        Calc {
            wb,
            all,
            cache: RefCell::default(),
            order: RefCell::default(),
            spills: RefCell::new(vec![Vec::new(); sheets]),
            hints: vec![Vec::new(); sheets],
            placed: RefCell::default(),
            blocked: RefCell::default(),
            typed: RefCell::new(vec![(None, None); sheets]),
            large: RefCell::default(),
            range_cache: RefCell::default(),
            cycles: Cell::new(false),
            path: RefCell::default(),
            looped: RefCell::default(),
            unsettled: Cell::new(false),
            depth: Cell::new(0),
            settle_depth: options.settle_depth,
            name_depth: Cell::new(0),
            now,
            iteration: options.iterate.map(|_| Iteration::default()),
            audit: options.audit.then(Audit::default),
        }
    }

    /// Recalculate every formula in the workbook.
    pub fn run(wb: &Workbook) -> CalcResult {
        run_full(wb, None, &Options::default())
    }

    /// Evaluate a standalone expression against the workbook's current
    /// (already calculated) state, as if it lived in `(sheet, row, col)`.
    /// To evaluate several, open one [`EvalSession`] for all of them.
    pub fn eval_once(wb: &Workbook, sheet: usize, row: u32, col: u32, expr: &Expr) -> Value {
        EvalSession::new(wb).eval(sheet, row, col, expr)
    }

    /// Take the result of a formula as given: it is not calculated.
    fn pin(&mut self, at: Pos, value: &Value) {
        let (sheet, row, col) = unpos(at);
        if let Value::Array(array) = value {
            let rect = Rect::new(
                row,
                col,
                row + array.rows as u32 - 1,
                col + array.cols as u32 - 1,
            );
            self.spills.get_mut()[sheet].push(Live {
                at: SpillRect {
                    anchor: (row, col),
                    rect,
                },
                array: array.clone(),
            });
        }
        self.cache.get_mut().insert(at, Slot::Done(value.clone()));
        self.order.get_mut().push(at);
    }

    /// The formulas among `queue` and, with a graph, every formula reading
    /// one of them, directly or not, or reading a cell one of their arrays
    /// covers.
    fn downstream(&self, mut queue: Queue, graph: Option<&DepGraph>) -> Vec<Root<'a>> {
        let anchors: FastMap<Pos, Rect> = self
            .spills
            .borrow()
            .iter()
            .enumerate()
            .flat_map(|(si, lives)| {
                lives
                    .iter()
                    .map(move |l| (pos(si, l.at.anchor.0, l.at.anchor.1), l.at.rect))
            })
            .collect();
        let mut roots = Vec::with_capacity(queue.list.len());
        let mut next = 0;
        while let Some(&at) = queue.list.get(next) {
            next += 1;
            // The graph may still list a formula that is gone.
            let Some(found) = root(self.wb, at) else {
                continue;
            };
            roots.push(found);
            let Some(graph) = graph else {
                continue;
            };
            graph.readers_of_cell(at, &mut |p| queue.push(p));
            if let Some(rect) = anchors.get(&at) {
                graph.readers_of_rect(unpos(at).0, rect, &mut |p| queue.push(p));
            }
        }
        roots
    }

    /// Open a round of a partial pass: `roots` are to be calculated
    /// (again). Their arrays leave the sheet until then.
    fn begin(&mut self, roots: &[Root<'a>]) {
        self.range_cache.get_mut().clear();
        self.placed.get_mut().clear();
        let cache = self.cache.get_mut();
        for r in roots {
            cache.insert(r.at, Slot::Pending);
        }
        let due = |sheet: usize, anchor: (u32, u32)| {
            matches!(
                cache.get(&pos(sheet, anchor.0, anchor.1)),
                Some(Slot::Pending)
            )
        };
        for (si, lives) in self.spills.get_mut().iter_mut().enumerate() {
            self.hints[si].clear();
            lives.retain(|live| {
                if due(si, live.at.anchor) {
                    self.hints[si].push(live.at);
                }
                !due(si, live.at.anchor)
            });
        }
        self.blocked
            .get_mut()
            .retain(|(sheet, spill)| !due(*sheet, spill.anchor));
    }

    /// Close a round: the areas newly covered by an array, which formulas
    /// calculated earlier may have read as blank. Every area an array left
    /// or took is added to `moved`.
    fn spread(&self, moved: &mut Vec<(usize, Rect)>) -> Vec<(usize, Rect)> {
        let placed = self.placed.borrow();
        let mut grown = Vec::new();
        for (sheet, new) in placed.iter() {
            let old = self.hints[*sheet].iter().find(|h| h.anchor == new.anchor);
            if old.is_some_and(|old| old.rect == new.rect) {
                continue;
            }
            moved.push((*sheet, new.rect));
            let inside = |old: &SpillRect| old.rect.intersection(&new.rect) == Some(new.rect);
            if !old.is_some_and(inside) {
                grown.push((*sheet, new.rect));
            }
        }
        for (sheet, hints) in self.hints.iter().enumerate() {
            for old in hints {
                let same = |(s, new): &(usize, SpillRect)| *s == sheet && *new == *old;
                if !placed.iter().any(same) {
                    moved.push((sheet, old.rect));
                }
            }
        }
        grown
    }

    /// Iterative calculation, after a first pass in which formulas referred
    /// back to cells being calculated and read their previous value: the
    /// formulas downstream of those cells are calculated again until no
    /// value changes by `tolerance` or more, `iterations` times in all.
    fn iterate(&mut self, graph: Option<&DepGraph>, iterations: u32, tolerance: f64) {
        for _ in 1..iterations {
            let Some(iteration) = &self.iteration else {
                return;
            };
            let mut queue = Queue::default();
            for target in iteration.targets.borrow().iter() {
                queue.push(*target);
            }
            if queue.list.is_empty() {
                return;
            }
            match graph {
                Some(graph) => graph.dynamic().iter().for_each(|p| queue.push(*p)),
                // Without a graph every formula is calculated again.
                None => {
                    for (si, sheet) in self.wb.sheets.iter().enumerate() {
                        for (row, col) in sheet.cells.keys() {
                            queue.push(pos(si, *row, *col));
                        }
                    }
                }
            }
            let mut roots = self.downstream(queue, graph);
            // In sheet order, like the first pass.
            roots.sort_unstable_by_key(|r| r.at);
            {
                let cache = self.cache.borrow();
                let mut previous = iteration.previous.borrow_mut();
                previous.clear();
                for r in &roots {
                    if let Some(Slot::Done(v)) = cache.get(&r.at) {
                        previous.insert(r.at, v.clone().into_scalar());
                    }
                }
            }
            self.begin(&roots);
            let mut largest: f64 = 0.0;
            for r in &roots {
                let (sheet, row, col) = unpos(r.at);
                let after = self
                    .formula_value(sheet, row, col, r.cell, r.expr)
                    .into_scalar();
                let iteration = self.iteration.as_ref().expect("iterating");
                if let Some(before) = iteration.previous.borrow().get(&r.at) {
                    largest = largest.max(change(before, &after));
                }
            }
            if largest < tolerance {
                return;
            }
        }
    }

    /// Where the arrays are, sorted by anchor.
    fn spill_rects(&self) -> Vec<Vec<SpillRect>> {
        self.spills
            .borrow()
            .iter()
            .map(|lives| {
                let mut list: Vec<SpillRect> = lives.iter().map(|l| l.at).collect();
                list.sort_unstable_by_key(|s| s.anchor);
                list
            })
            .collect()
    }

    fn finish(self, moved: Vec<(usize, Rect)>) -> CalcResult {
        let spills = self
            .spills
            .into_inner()
            .into_iter()
            .map(|mut lives| {
                lives.sort_unstable_by_key(|l| l.at.anchor);
                lives.into_iter().map(|l| (l.at, l.array)).collect()
            })
            .collect();
        let mut cache = self.cache.into_inner();
        let values = self
            .order
            .into_inner()
            .into_iter()
            .filter_map(|at| match cache.remove(&at) {
                Some(Slot::Done(v)) => Some((unpos(at), v)),
                // Calculated more than once: already taken.
                _ => None,
            })
            .collect();
        let mut circular_cells: Vec<CellKey> =
            self.looped.into_inner().into_iter().map(unpos).collect();
        circular_cells.sort_unstable();
        CalcResult {
            values,
            spills,
            // With iterative calculation a circular reference is no error.
            circular: self.cycles.get() && self.iteration.is_none(),
            circular_cells,
            unsettled: self.unsettled.get() || self.cycles.get(),
            blocked: self.blocked.into_inner(),
            moved,
            audit: self.audit.and_then(|a| a.violation.into_inner()),
        }
    }

    fn progress(&self, at: Pos) -> Progress {
        match self.cache.borrow().get(&at) {
            Some(Slot::Done(_)) => Progress::Known,
            Some(Slot::Computing) => Progress::Busy,
            Some(Slot::Pending) => Progress::Due,
            None if self.all => Progress::Due,
            None => Progress::Known,
        }
    }

    fn formula_value(
        &self,
        sheet: usize,
        row: u32,
        col: u32,
        cell: &'a SheetCell,
        expr: &'a Expr,
    ) -> Value {
        let at = pos(sheet, row, col);
        match self.cache.borrow().get(&at) {
            Some(Slot::Done(v)) => return v.clone(),
            Some(Slot::Computing) => return self.reentered(at, cell),
            Some(Slot::Pending) => {}
            None if self.all => {}
            // Not part of this pass: the value of the last calculation.
            None => return cell.value.clone(),
        }
        if self.depth.get() >= self.settle_depth {
            self.settle_precedents(Root { at, cell, expr });
            // A reference worked out at calculation time may have led back
            // here.
            if let Some(Slot::Done(v)) = self.cache.borrow().get(&at) {
                return v.clone();
            }
        }
        self.calculate(sheet, row, col, expr)
    }

    /// A formula reads a cell that is being calculated: a circular
    /// reference. As in Excel every cell of the loop then shows 0; with
    /// iterative calculation the cell reads as it was at the previous
    /// iteration.
    fn reentered(&self, at: Pos, cell: &SheetCell) -> Value {
        self.cycles.set(true);
        match &self.iteration {
            None => {
                // The loop: from that cell to the formula reading it.
                let path = self.path.borrow();
                let from = path.iter().rposition(|p| *p == at).unwrap_or(0);
                self.looped.borrow_mut().extend(&path[from..]);
                Value::Number(0.0)
            }
            Some(iteration) => {
                iteration.targets.borrow_mut().insert(at);
                let previous = iteration.previous.borrow();
                previous.get(&at).unwrap_or(&cell.value).clone()
            }
        }
    }

    fn calculate(&self, sheet: usize, row: u32, col: u32, expr: &'a Expr) -> Value {
        if self.depth.get() >= MAX_DEPTH {
            self.unsettled.set(true);
            return Value::Error(ErrorKind::Calc);
        }
        let at = pos(sheet, row, col);
        self.cache.borrow_mut().insert(at, Slot::Computing);
        self.depth.set(self.depth.get() + 1);
        self.path.borrow_mut().push(at);
        let outer = self.audit.as_ref().map(|a| a.reader.replace(Some(at)));
        let large = self.large.take();
        let ctx = EvalCtx {
            calc: self,
            sheet,
            row,
            col,
            root: expr,
            home: sheet,
        };
        let mut v = ctx.eval(expr);
        *self.large.borrow_mut() = large;
        if let (Some(audit), Some(outer)) = (&self.audit, outer) {
            audit.reader.set(outer);
        }
        self.depth.set(self.depth.get() - 1);
        self.path.borrow_mut().pop();
        if self.looped.borrow().contains(&at) {
            v = Value::Number(0.0);
        }

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
        self.cache.borrow_mut().insert(at, Slot::Done(v.clone()));
        self.order.borrow_mut().push(at);
        v
    }

    /// Calculate the formulas `root` reads before it, and those they read
    /// before them, with a list of work to do instead of recursion: a chain
    /// of a million formulas each reading the next is then calculated from
    /// its far end, at no cost in stack.
    ///
    /// A formula is only calculated ahead of its turn when that cannot
    /// change its result: once everything it reads is calculated. One in a
    /// loop of references is left alone, with everything reading it, and
    /// so is one that works out what it reads as it goes (`INDIRECT`):
    /// calculated in its turn, it finds a circular reference where it
    /// has always been found.
    fn settle_precedents(&self, root: Root<'a>) {
        struct Frame<'a> {
            formula: Root<'a>,
            todo: Vec<Pos>,
            /// Everything the formula reads is known and calculated.
            clean: bool,
        }
        let frame = |formula: Root<'a>| {
            let (sheet, row, col) = unpos(formula.at);
            let found = deps::precedents(self.wb, sheet, row, col, formula.expr);
            let mut frame = Frame {
                formula,
                todo: Vec::new(),
                clean: !found.dynamic,
            };
            let mut reads = |at: Pos| match self.progress(at) {
                Progress::Known => {}
                Progress::Busy => frame.clean = false,
                Progress::Due => frame.todo.push(at),
            };
            for (s, rect) in found.areas {
                self.formulas_in(s, &rect, &mut reads);
                // The arrays expected to spill into the area are read too.
                for hint in self.hints[s].iter().filter(|h| h.rect.intersects(&rect)) {
                    reads(pos(s, hint.anchor.0, hint.anchor.1));
                }
            }
            frame
        };
        let mut open: FastSet<Pos> = FastSet::default();
        open.insert(root.at);
        let mut stack = vec![frame(root)];
        while let Some(top) = stack.last_mut() {
            if let Some(at) = top.todo.pop() {
                match self.progress(at) {
                    Progress::Known => {}
                    Progress::Busy => top.clean = false,
                    Progress::Due => match crate::eval::root(self.wb, at) {
                        // Already waiting further down: a loop.
                        Some(_) if open.contains(&at) => top.clean = false,
                        Some(formula) => {
                            open.insert(at);
                            stack.push(frame(formula));
                        }
                        None => {}
                    },
                }
                continue;
            }
            let done = stack.pop().expect("a frame");
            open.remove(&done.formula.at);
            let Some(parent) = stack.last_mut() else {
                return;
            };
            if !done.clean {
                parent.clean = false;
            } else if self.progress(done.formula.at) == Progress::Due {
                let (sheet, row, col) = unpos(done.formula.at);
                self.calculate(sheet, row, col, done.formula.expr);
            }
        }
    }

    /// Call `f` with every formula cell of an area.
    fn formulas_in(&self, sheet: usize, rect: &Rect, f: &mut dyn FnMut(Pos)) {
        let Some(sh) = self.wb.sheets.get(sheet) else {
            return;
        };
        let Some(last) = sh.cells.keys().next_back() else {
            return;
        };
        let r1 = rect.r1.min(last.0);
        let mut visit = |row: u32, col: u32, cell: &SheetCell| {
            if matches!(cell.content, Content::Formula(_)) {
                f(pos(sheet, row, col));
            }
        };
        if rect.cols() <= 4 {
            for row in rect.r0..=r1 {
                for col in rect.c0..=rect.c1 {
                    if let Some(cell) = sh.cells.get(&(row, col)) {
                        visit(row, col, cell);
                    }
                }
            }
        } else if rect.r0 <= r1 {
            for ((row, col), cell) in sh.cells.range((rect.r0, rect.c0)..=(r1, rect.c1)) {
                if *col >= rect.c0 && *col <= rect.c1 {
                    visit(*row, *col, cell);
                }
            }
        }
    }

    fn register_spill(&self, sheet: usize, row: u32, col: u32, a: &Arc<Array>) -> bool {
        let r1 = row as u64 + a.rows as u64 - 1;
        let c1 = col as u64 + a.cols as u64 - 1;
        if r1 >= MAX_ROWS as u64 || c1 >= MAX_COLS as u64 {
            return false;
        }
        let rect = Rect::new(row, col, r1 as u32, c1 as u32);
        let at = SpillRect {
            anchor: (row, col),
            rect,
        };
        let sh = &self.wb.sheets[sheet];
        for ((r, c), cell) in sh.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
            if *c < rect.c0 || *c > rect.c1 || (*r == row && *c == col) {
                continue;
            }
            if !matches!(cell.content, Content::Empty) {
                self.blocked.borrow_mut().push((sheet, at));
                return false;
            }
        }
        let mut spills = self.spills.borrow_mut();
        if spills[sheet].iter().any(|s| s.at.rect.intersects(&rect)) {
            // Which of two arrays gets the cells depends on which formula
            // is calculated first.
            self.unsettled.set(true);
            return false;
        }
        spills[sheet].push(Live {
            at,
            array: a.clone(),
        });
        self.placed.borrow_mut().push((sheet, at));
        true
    }

    fn spill_at(&self, sheet: usize, row: u32, col: u32) -> Option<SpillRect> {
        self.spills.borrow()[sheet]
            .iter()
            .find(|s| s.at.rect.contains(row, col))
            .map(|s| s.at)
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
            match self.progress(pos(sheet, h.anchor.0, h.anchor.1)) {
                Progress::Known => continue,
                // A formula reads, directly or not, the cells its own array
                // covers: what it finds there depends on where calculation
                // started.
                Progress::Busy => {
                    self.unsettled.set(true);
                    continue;
                }
                Progress::Due => {}
            }
            if let Some(cell) = self.wb.sheets[sheet].cells.get(&h.anchor) {
                if let Content::Formula(expr) = &cell.content {
                    self.formula_value(sheet, h.anchor.0, h.anchor.1, cell, expr);
                }
            }
        }
    }

    fn spilled_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.ensure_hinted(sheet, &Rect::cell(row, col));
        let spills = self.spills.borrow();
        match spills[sheet].iter().find(|s| s.at.rect.contains(row, col)) {
            Some(s) => s
                .array
                .get(
                    (row - s.at.anchor.0) as usize,
                    (col - s.at.anchor.1) as usize,
                )
                .clone(),
            None => Value::Empty,
        }
    }

    /// Scalar value of a cell.
    fn cell_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        let Some(sh) = self.wb.sheets.get(sheet) else {
            return Value::Error(ErrorKind::Ref);
        };
        self.audit_read(sheet, &Rect::cell(row, col));
        match sh.cells.get(&(row, col)) {
            Some(cell) => match &cell.content {
                Content::Formula(expr) => self
                    .formula_value(sheet, row, col, cell, expr)
                    .into_scalar(),
                Content::Value(v) => v.clone(),
                Content::Empty => self.spilled_value(sheet, row, col),
            },
            None => self.spilled_value(sheet, row, col),
        }
    }

    /// Last row holding typed data on a sheet.
    fn last_row(&self, sheet: usize) -> u32 {
        if let Some(row) = self.typed.borrow()[sheet].0 {
            return row;
        }
        let cells = &self.wb.sheets[sheet].cells;
        let last = cells
            .iter()
            .rev()
            .find(|(_, cell)| !matches!(cell.content, Content::Empty));
        let row = last.map_or(0, |((row, _), _)| *row);
        self.typed.borrow_mut()[sheet].0 = Some(row);
        row
    }

    /// Last column holding typed data on a sheet.
    fn last_col(&self, sheet: usize) -> u32 {
        if let Some(col) = self.typed.borrow()[sheet].1 {
            return col;
        }
        let (row, col) = self.wb.sheets[sheet].cell_extent();
        self.typed.borrow_mut()[sheet] = (Some(row), Some(col));
        col
    }

    /// The large ranges the formula of `ctx` reads, as far as its text
    /// tells.
    fn large_areas(&self, ctx: &EvalCtx) -> Rc<Vec<(usize, Rect)>> {
        if let Some(known) = &*self.large.borrow() {
            return known.clone();
        }
        let mut areas = deps::precedents(self.wb, ctx.home, ctx.row, ctx.col, ctx.root).areas;
        areas.retain(|(_, rect)| deps::is_clamped(rect));
        let areas = Rc::new(areas);
        *self.large.borrow_mut() = Some(areas.clone());
        areas
    }

    /// The part of a range that is materialised: large ranges (whole
    /// columns / rows above all) are clamped to the part of the sheet
    /// holding data; ordinary ranges keep their shape.
    ///
    /// The data is what was typed anywhere on the sheet, and the arrays
    /// spilling into the range. All the large ranges one formula reads on
    /// a sheet are clamped alike, so that `A:A*B:B` pairs the two columns
    /// cell for cell however far an array runs down one of them.
    fn clamp_rect(&self, reference: &Reference, ctx: &EvalCtx) -> Rect {
        let rect = reference.rect;
        let sheet = reference.sheet;
        if sheet >= self.wb.sheets.len() {
            return rect;
        }
        self.ensure_hinted(sheet, &rect);
        if !deps::is_clamped(&rect) {
            return rect;
        }
        let areas = self.large_areas(ctx);
        let alike = || {
            areas
                .iter()
                .filter(|(s, _)| *s == sheet)
                .map(|(_, area)| area)
        };
        for area in alike() {
            self.ensure_hinted(sheet, area);
        }
        let mut er = self.last_row(sheet);
        // One column is never narrowed.
        let mut ec = match rect.c0 < rect.c1 {
            true => self.last_col(sheet),
            false => rect.c1,
        };
        for s in self.spills.borrow()[sheet].iter() {
            if s.at.rect.intersects(&rect) || alike().any(|area| s.at.rect.intersects(area)) {
                er = er.max(s.at.rect.r1);
                ec = ec.max(s.at.rect.c1);
            }
        }
        Rect::new(
            rect.r0,
            rect.c0,
            rect.r1.min(er.max(rect.r0)),
            rect.c1.min(ec.max(rect.c0)),
        )
    }

    /// Materialise a range, clamped to the part of the sheet holding data.
    fn range_array(&self, reference: &Reference, ctx: &EvalCtx) -> Arc<Array> {
        let Some(sh) = self.wb.sheets.get(reference.sheet) else {
            return Arc::new(Array::row(vec![Value::Error(ErrorKind::Ref)]));
        };
        self.audit_read(reference.sheet, &reference.rect);
        let rect = reference.rect;
        let clamped = self.clamp_rect(reference, ctx);
        let (r1, c1) = (clamped.r1, clamped.c1);
        if let Some(a) = self.range_cache.borrow().get(&(*reference, r1, c1)) {
            return a.clone();
        }
        let rows = (r1 - rect.r0 + 1) as usize;
        let cols = (c1 - rect.c0 + 1) as usize;
        let mut arr = Array::new(rows, cols);

        let mut fill = |r: u32, c: u32, cell: &'a SheetCell| {
            let v = match &cell.content {
                Content::Formula(expr) => self
                    .formula_value(reference.sheet, r, c, cell, expr)
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
                        fill(r, c, cell);
                    }
                }
            }
        } else {
            for ((r, c), cell) in sh.cells.range((rect.r0, rect.c0)..=(r1, c1)) {
                if *c >= rect.c0 && *c <= c1 {
                    fill(*r, *c, cell);
                }
            }
        }
        // Overlay spilled arrays.
        let spills: Vec<Live> = self.spills.borrow()[reference.sheet]
            .iter()
            .filter(|s| s.at.rect.intersects(&clamped))
            .cloned()
            .collect();
        for sp in spills {
            let Some(overlap) = sp.at.rect.intersection(&clamped) else {
                continue;
            };
            let (r0, c0) = sp.at.anchor;
            for r in overlap.r0..=overlap.r1 {
                for c in overlap.c0..=overlap.c1 {
                    // The anchor holds its formula, read above.
                    if (r, c) == sp.at.anchor {
                        continue;
                    }
                    arr.set(
                        (r - rect.r0) as usize,
                        (c - rect.c0) as usize,
                        sp.array.get((r - r0) as usize, (c - c0) as usize).clone(),
                    );
                }
            }
        }
        let arr = Arc::new(arr);
        // Only cache when nothing is mid-computation (no cycle in flight), so
        // a partial view is never reused.
        if !self.cycles.get() {
            self.range_cache
                .borrow_mut()
                .insert((*reference, r1, c1), arr.clone());
        }
        arr
    }

    /// With the audit on: the formula being calculated reads `rect`, which
    /// must be among the precedents the dependency graph knows it by.
    fn audit_read(&self, sheet: usize, rect: &Rect) {
        let Some(audit) = &self.audit else {
            return;
        };
        let Some(reader) = audit.reader.get() else {
            return;
        };
        let known = audit.precedents.borrow().get(&reader).cloned();
        let found = match known {
            Some(found) => found,
            None => {
                let Some(formula) = root(self.wb, reader) else {
                    return;
                };
                let (s, row, col) = unpos(reader);
                let found = Rc::new(deps::precedents(self.wb, s, row, col, formula.expr));
                audit.precedents.borrow_mut().insert(reader, found.clone());
                found
            }
        };
        if found.volatile || found.covers(sheet, rect) {
            return;
        }
        // `A1#` reads the array of its anchor.
        let within = |s: &Live| {
            s.at.rect.intersection(rect) == Some(*rect)
                && found.covers(sheet, &Rect::cell(s.at.anchor.0, s.at.anchor.1))
        };
        if self.spills.borrow()[sheet].iter().any(within) {
            return;
        }
        let (s, row, col) = unpos(reader);
        let name = |sheet: usize| self.wb.sheets[sheet].name.as_str();
        audit.violation.borrow_mut().get_or_insert_with(|| {
            format!(
                "the formula in {}!{} reads {}!{}, which is not among its precedents",
                name(s),
                crate::refs::cell_name(row, col),
                name(sheet),
                rect.name()
            )
        });
    }
}

/// Many expressions evaluated against the values the workbook holds, as
/// conditional formats, data validation and charts need: nothing is
/// recalculated, a formula cell reads as the result of the last
/// calculation. Opening a session costs next to nothing, and the ranges it
/// reads are kept for the next expression.
pub struct EvalSession<'a> {
    calc: Calc<'a>,
}

impl<'a> EvalSession<'a> {
    pub fn new(wb: &'a Workbook) -> EvalSession<'a> {
        let now = crate::datetime::now_serial();
        let mut calc = Calc::new(wb, false, now, &Options::default());
        *calc.spills.get_mut() = stored_spills(wb);
        EvalSession { calc }
    }

    /// Evaluate `expr` as if it lived in the cell (`sheet`, `row`, `col`).
    pub fn eval(&self, sheet: usize, row: u32, col: u32, expr: &Expr) -> Value {
        self.calc.large.take();
        let ctx = EvalCtx {
            calc: &self.calc,
            sheet,
            row,
            col,
            root: expr,
            home: sheet,
        };
        ctx.eval(expr)
    }

    /// The cells `expr` stands for when it evaluates to a reference, as
    /// read from the cell (`sheet`, `row`, `col`): one area, or several for
    /// a union or a 3-D reference. Formulas that only give a reference
    /// once calculated (`OFFSET`, `INDEX`, `INDIRECT`, a name defined with
    /// them) are followed. An error value when it is not a reference.
    pub fn areas(&self, sheet: usize, row: u32, col: u32, expr: &Expr) -> R<Vec<Reference>> {
        self.calc.large.take();
        let ctx = EvalCtx {
            calc: &self.calc,
            sheet,
            row,
            col,
            root: expr,
            home: sheet,
        };
        ctx.eval_areas(expr)
    }

    pub fn workbook(&self) -> &'a Workbook {
        self.calc.wb
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
    /// The formula being calculated, and the sheet of its cell (`sheet`
    /// is another one while a defined name is evaluated).
    root: &'a Expr,
    home: usize,
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
            Value::Array(self.calc.range_array(reference, self))
        }
    }

    /// Values of a resolved reference as an array (clamped to used cells).
    pub fn range_values(&self, reference: &Reference) -> Arc<Array> {
        self.calc.range_array(reference, self)
    }

    /// The rectangle `range_values` / `eval` actually deliver for a
    /// reference (whole columns and rows are clamped to the used sheet).
    pub fn clamped(&self, reference: &Reference) -> Rect {
        self.calc.clamp_rect(reference, self)
    }

    /// Scalar value of one cell.
    pub fn cell_value(&self, sheet: usize, row: u32, col: u32) -> Value {
        self.calc.cell_value(sheet, row, col)
    }

    /// The formula stored in a cell, if any.
    pub fn formula_at(&self, sheet: usize, row: u32, col: u32) -> Option<&'a Expr> {
        if sheet < self.calc.wb.sheets.len() {
            self.calc.audit_read(sheet, &Rect::cell(row, col));
        }
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
