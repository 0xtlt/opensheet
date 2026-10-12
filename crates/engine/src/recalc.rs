//! Recalculation: the calculation settings of a workbook, what changed
//! since it was last calculated, and the choice between calculating every
//! formula and calculating what depends on the changes.
//!
//! Cell contents are tracked where they change: `Workbook::put_cell`, undo
//! and redo record the cell; what was done directly to the sheets after
//! `Workbook::snapshot` is found by comparing them with the snapshot.
//! Sheets, defined names and tables are not tracked but compared with what
//! they were at the last calculation: any difference there calculates
//! everything again. So does `Workbook::recalc`, which trusts nothing.

use crate::ast::Expr;
use crate::deps::{self, DepGraph, GraphStats, Pos, pos, unpos};
use crate::eval::{self, CalcResult, Changes, EvalSession, Options, Scope, SpillRect};
use crate::refs::Rect;
use crate::workbook::{Cell, Content, Sheet, Spill, Workbook};
use std::sync::Arc;

/// When formulas are calculated (Formulas > Calculation Options).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CalcMode {
    /// After every change.
    #[default]
    Automatic,
    /// Excel leaves data tables out; there are none here, so this is
    /// `Automatic`.
    AutomaticExceptTables,
    /// On request only (`Workbook::calculate_now`, `calculate_sheet`). A
    /// formula is still calculated when it is entered.
    Manual,
}

/// Iterative calculation of circular references (File > Options >
/// Formulas > Enable iterative calculation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IterativeCalc {
    pub enabled: bool,
    /// How many times the formulas of a circular reference are calculated
    /// at most.
    pub max_iterations: u32,
    /// Iterating stops once no value changes by this much.
    pub max_change: f64,
}

impl Default for IterativeCalc {
    fn default() -> IterativeCalc {
        IterativeCalc {
            enabled: false,
            max_iterations: 100,
            max_change: 0.001,
        }
    }
}

/// Set on an entry of `CalcState::touched` when the cell held a formula
/// before it changed.
const REPLACED: u64 = 1 << 62;

/// True when two states of a cell hold the same thing, whatever their
/// formatting and cached result.
pub(crate) fn same_content(a: Option<&Cell>, b: Option<&Cell>) -> bool {
    match (a.map(|c| &c.content), b.map(|c| &c.content)) {
        (Some(Content::Formula(x)), Some(Content::Formula(y))) => Arc::ptr_eq(x, y) || x == y,
        (Some(x), Some(y)) => x == y,
        (Some(x), None) | (None, Some(x)) => matches!(x, Content::Empty),
        (None, None) => true,
    }
}

/// A formula of a defined name; clones of a workbook share them.
struct SameExpr(Arc<Expr>);

impl PartialEq for SameExpr {
    fn eq(&self, other: &SameExpr) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

/// What formulas depend on besides cells: the sheets and their order, the
/// defined names, the tables.
#[derive(Default, PartialEq)]
struct Structure {
    sheets: Vec<String>,
    /// Scope (a sheet, or `usize::MAX` for the workbook), key, formula.
    names: Vec<(usize, String, SameExpr)>,
    /// Sheet, name, range, header row, totals row, column names.
    tables: Vec<(usize, String, Rect, bool, bool, Vec<String>)>,
}

impl Structure {
    fn of(wb: &Workbook) -> Structure {
        let named =
            |scope: usize,
             names: &std::collections::BTreeMap<String, crate::workbook::DefinedName>| {
                names
                    .iter()
                    .map(|(key, def)| (scope, key.clone(), SameExpr(def.expr.clone())))
                    .collect::<Vec<_>>()
            };
        let mut names = named(usize::MAX, &wb.names);
        let mut tables = Vec::new();
        for (si, sheet) in wb.sheets.iter().enumerate() {
            names.extend(named(si, &sheet.names));
            for t in &sheet.tables {
                let columns = t.columns.iter().map(|c| c.name.clone()).collect();
                tables.push((
                    si,
                    t.name.clone(),
                    t.range,
                    t.header_row,
                    t.totals_row,
                    columns,
                ));
            }
        }
        Structure {
            sheets: wb.sheets.iter().map(|s| s.name.clone()).collect(),
            names,
            tables,
        }
    }
}

/// Calculation settings and state of a workbook.
pub(crate) struct CalcState {
    mode: CalcMode,
    iterative: IterativeCalc,
    /// Off, every recalculation calculates every formula.
    incremental: bool,
    graph: Option<DepGraph>,
    structure: Structure,
    /// Cells whose content changed since the last calculation.
    touched: Vec<u64>,
    /// How many of them the graph has been told about.
    synced: usize,
    /// How many of them were there before the current transaction.
    settled: usize,
    /// Cells whose value changed since without being calculated in full:
    /// the areas arrays spilled into or left while only part of the
    /// workbook was calculated.
    rects: Vec<(usize, Rect)>,
    /// Rows were hidden, shown or filtered.
    visibility: bool,
    /// Something only volatile functions read has changed (a PivotTable).
    refreshed: bool,
    /// Something changed that was not followed cell by cell.
    untracked: bool,
    /// Per sheet, last row and column holding typed data at the last
    /// calculation.
    typed: Vec<(u32, u32)>,
    /// Arrays that could not spill over the cells in their way.
    blocked: Vec<(usize, SpillRect)>,
    /// The last calculation depended on the order of calculation.
    unsettled: bool,
    /// Formulas wait to be calculated (manual calculation).
    stale: bool,
    /// How many calculations covered every formula, and how many only
    /// what had changed.
    runs: (u64, u64),
    /// See `eval::SETTLE_DEPTH`.
    settle_depth: u32,
}

impl Default for CalcState {
    fn default() -> CalcState {
        CalcState {
            mode: CalcMode::default(),
            iterative: IterativeCalc::default(),
            incremental: true,
            graph: None,
            structure: Structure::default(),
            touched: Vec::new(),
            synced: 0,
            settled: 0,
            rects: Vec::new(),
            visibility: false,
            refreshed: false,
            // Nothing is known of a workbook that was never calculated.
            untracked: true,
            typed: Vec::new(),
            blocked: Vec::new(),
            unsettled: false,
            stale: false,
            runs: (0, 0),
            settle_depth: eval::SETTLE_DEPTH,
        }
    }
}

impl CalcState {
    /// The state of a workbook that is up to date and holds no spilled
    /// array (a new one).
    pub(crate) fn of_calculated(wb: &Workbook) -> CalcState {
        CalcState {
            graph: Some(DepGraph::build(wb)),
            structure: Structure::of(wb),
            typed: wb.sheets.iter().map(|s| s.cell_extent()).collect(),
            untracked: false,
            ..CalcState::default()
        }
    }

    /// The content of a cell changed. `replaced` tells whether it held a
    /// formula.
    pub(crate) fn touch(&mut self, sheet: usize, row: u32, col: u32, replaced: bool) {
        self.touched
            .push(pos(sheet, row, col) | if replaced { REPLACED } else { 0 });
    }

    /// The sheets were changed directly, or replaced by an undo snapshot:
    /// `new` is what `old` became. Every cell whose content differs is
    /// recorded. The others keep the result they had in `old`: `new` may
    /// hold the results of another time (a snapshot), or of another place
    /// (the same formula moved into the cell from elsewhere).
    pub(crate) fn sheets_changed(&mut self, old: &[Sheet], new: &mut [Sheet]) {
        if self.untracked {
            return;
        }
        if old.len() != new.len() || old.iter().zip(new.iter()).any(|(a, b)| a.name != b.name) {
            self.untracked = true;
            return;
        }
        // Past this many changes it is as fast to start afresh.
        let cells: usize = old.iter().map(|s| s.cells.len()).sum();
        let limit = self.touched.len() + cells / 4 + 4096;
        for (si, (old, new)) in old.iter().zip(new.iter_mut()).enumerate() {
            if old.hidden_rows != new.hidden_rows || old.filtered_rows != new.filtered_rows {
                self.visibility = true;
            }
            // A PivotTable that was touched holds another report or cache.
            let same = |a: &crate::pivot::PivotTable, b: &crate::pivot::PivotTable| {
                Arc::ptr_eq(&a.report, &b.report) && Arc::ptr_eq(&a.cache, &b.cache)
            };
            if old.pivots.len() != new.pivots.len()
                || !old.pivots.iter().zip(&new.pivots).all(|(a, b)| same(a, b))
            {
                self.refreshed = true;
            }
            new.spills = old.spills.clone();
            let formula = |cell: &Cell| matches!(cell.content, Content::Formula(_));
            let mut before = old.cells.iter().peekable();
            for (key, cell) in new.cells.iter_mut() {
                while let Some((gone, was)) = before.next_if(|(k, _)| *k < key) {
                    if !same_content(Some(was), None) {
                        self.touch(si, gone.0, gone.1, formula(was));
                    }
                }
                match before.next_if(|(k, _)| *k == key) {
                    Some((_, was)) if same_content(Some(was), Some(cell)) => {
                        if cell.value != was.value {
                            cell.value = was.value.clone();
                        }
                    }
                    Some((_, was)) => self.touch(si, key.0, key.1, formula(was)),
                    None if same_content(None, Some(cell)) => {}
                    None => self.touch(si, key.0, key.1, false),
                }
            }
            for (gone, was) in before {
                if !same_content(Some(was), None) {
                    self.touch(si, gone.0, gone.1, formula(was));
                }
            }
            if self.touched.len() > limit {
                self.untracked = true;
                return;
            }
        }
    }

    /// Tell the graph about the cells that changed since it last heard.
    fn sync(&mut self, wb: &Workbook) {
        let Some(graph) = &mut self.graph else {
            return;
        };
        let mut fresh: Vec<u64> = self.touched[self.synced..].to_vec();
        // A cell changed several times is registered once; it replaced a
        // formula if it did the first time.
        fresh.sort_by_key(|t| *t & !REPLACED);
        fresh.dedup_by_key(|t| *t & !REPLACED);
        for t in fresh {
            graph.update(wb, t & !REPLACED, t & REPLACED != 0);
        }
        self.synced = self.touched.len();
    }

    /// Where typed data ends on each sheet now, and whether that moved
    /// since the last calculation (rows, columns).
    fn typed_now(&self, wb: &Workbook) -> (Vec<(u32, u32)>, Vec<(bool, bool)>) {
        let mut now = self.typed.clone();
        let mut scan = vec![false; now.len()];
        for t in &self.touched {
            let (sheet, row, col) = unpos(*t & !REPLACED);
            let Some(extent) = now.get_mut(sheet) else {
                continue;
            };
            let filled = wb
                .cell(sheet, row, col)
                .is_some_and(|c| !matches!(c.content, Content::Empty));
            if filled {
                *extent = (extent.0.max(row), extent.1.max(col));
            } else if row == self.typed[sheet].0 || col == self.typed[sheet].1 {
                // The last row or column may have emptied.
                scan[sheet] = true;
            }
        }
        for (sheet, extent) in now.iter_mut().enumerate() {
            if scan[sheet] {
                *extent = wb.sheets[sheet].cell_extent();
            }
        }
        let moved = now
            .iter()
            .zip(&self.typed)
            .map(|(a, b)| (a.0 != b.0, a.1 != b.1))
            .collect();
        (now, moved)
    }

    fn forget_changes(&mut self) {
        self.touched.clear();
        self.synced = 0;
        self.settled = 0;
        self.rects.clear();
        self.visibility = false;
        self.refreshed = false;
        self.untracked = false;
        self.stale = false;
    }
}

/// What a calculation is to cover.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Demand {
    /// Every formula, on a dependency graph built anew.
    Everything,
    /// What depends on the changes since the last calculation, and the
    /// volatile functions.
    Now,
    /// The same after a transaction, if it changed anything a formula can
    /// read: formatting a cell calculates nothing, as in Excel.
    Committed,
    /// What depends on the changes, on one sheet.
    Sheet(usize),
    /// The formulas entered in the transaction being committed.
    Entered,
}

impl Workbook {
    /// Recalculate every formula and rebuild the dependency graph, whatever
    /// was or was not tracked: everything is up to date when this returns.
    /// This is the call to make after changing a workbook behind its back
    /// (a file being loaded, the locale).
    pub fn recalc(&mut self) {
        self.calculate(Demand::Everything);
    }

    /// Calculate what changed since the last calculation and everything
    /// that depends on it (F9). With automatic calculation there is never
    /// anything left to do.
    pub fn calculate_now(&mut self) {
        self.calculate(Demand::Now);
    }

    /// Calculate what is out of date on one sheet (Shift+F9), reading the
    /// other sheets as they are.
    pub fn calculate_sheet(&mut self, sheet: usize) {
        if sheet < self.sheets.len() {
            self.calculate(Demand::Sheet(sheet));
        }
    }

    /// True when formulas wait for `calculate_now` (manual calculation):
    /// the status bar then shows "Calculate".
    pub fn needs_calculation(&self) -> bool {
        self.calc.stale
    }

    pub fn calc_mode(&self) -> CalcMode {
        self.calc.mode
    }

    /// Change when formulas are calculated. Leaving manual calculation
    /// calculates what was waiting.
    pub fn set_calc_mode(&mut self, mode: CalcMode) {
        self.modified |= self.calc.mode != mode;
        self.calc.mode = mode;
        if mode != CalcMode::Manual && self.calc.stale {
            self.calculate(Demand::Now);
        }
    }

    pub fn iterative_calc(&self) -> IterativeCalc {
        self.calc.iterative
    }

    /// Switch iterative calculation on or off, or change its limits. The
    /// workbook is calculated again when that changes what circular
    /// references evaluate to.
    pub fn set_iterative_calc(&mut self, iterative: IterativeCalc) {
        if self.calc.iterative == iterative {
            return;
        }
        self.calc.iterative = iterative;
        self.modified = true;
        // Circular references are calculated in full every time.
        if self.calc.unsettled && self.calc.mode != CalcMode::Manual {
            self.calculate(Demand::Now);
        }
    }

    /// Rows of `sheet` were hidden, shown or filtered: the formulas that
    /// skip hidden rows (`SUBTOTAL`, `AGGREGATE`) are calculated again by
    /// the next calculation. Hiding rows between `snapshot` and `commit`
    /// is noticed without this call.
    pub fn visibility_changed(&mut self, sheet: usize) {
        if sheet < self.sheets.len() {
            self.calc.visibility = true;
            self.calc.stale |= self.calc.mode == CalcMode::Manual;
        }
    }

    /// Evaluate expressions against the current values, see
    /// [`EvalSession`].
    pub fn eval_session(&self) -> EvalSession<'_> {
        EvalSession::new(self)
    }

    /// Size of the dependency graph, once the workbook was calculated.
    pub fn graph_stats(&self) -> Option<GraphStats> {
        self.calc.graph.as_ref().map(|g| g.stats())
    }

    /// For tests: how deep formulas reading one another may recurse
    /// before the precedents of a formula are calculated ahead of it. The
    /// results do not depend on it.
    #[doc(hidden)]
    pub fn set_settle_depth(&mut self, depth: u32) {
        self.calc.settle_depth = depth;
    }

    /// For tests and benchmarks: how many calculations covered every
    /// formula, and how many only what had changed.
    #[doc(hidden)]
    pub fn calc_runs(&self) -> (u64, u64) {
        self.calc.runs
    }

    /// For tests and benchmarks: with `false`, every recalculation
    /// calculates every formula, as if nothing were known of what changed.
    #[doc(hidden)]
    pub fn set_incremental(&mut self, on: bool) {
        self.calc.incremental = on;
        if !on {
            self.calc.graph = None;
        }
    }

    /// A transaction was committed, undone or redone: calculate what the
    /// calculation mode asks for.
    pub(crate) fn changed(&mut self) {
        match self.calc.mode {
            CalcMode::Manual => self.calculate(Demand::Entered),
            CalcMode::Automatic | CalcMode::AutomaticExceptTables => {
                self.calculate(Demand::Committed)
            }
        }
        self.calc.settled = self.calc.touched.len();
    }

    fn calculate(&mut self, demand: Demand) {
        self.generation += 1;
        // The state is taken out for the time of the calculation; the
        // settings stay readable.
        let mut st = std::mem::take(&mut self.calc);
        self.calc.mode = st.mode;
        self.calc.iterative = st.iterative;
        let options = Options {
            iterate: st
                .iterative
                .enabled
                .then_some((st.iterative.max_iterations, st.iterative.max_change)),
            audit: deps::audit(),
            settle_depth: st.settle_depth,
        };
        let structure = Structure::of(self);
        if structure != st.structure {
            st.untracked = true;
        }
        // When most cells changed, following the changes costs more than
        // starting afresh.
        let cells: usize = self.sheets.iter().map(|s| s.cells.len()).sum();
        if st.touched.len() > cells / 2 + 4096 {
            st.untracked = true;
        }
        let known = !st.untracked && st.graph.is_some() && st.typed.len() == self.sheets.len();
        // Nothing a formula can read has changed since the last
        // calculation.
        let quiet = !st.untracked
            && st.touched.is_empty()
            && st.rects.is_empty()
            && !st.visibility
            && !st.refreshed;
        if demand == Demand::Committed && quiet {
            // Formatting, column widths, comments: nothing to calculate.
        } else if demand == Demand::Entered {
            st.stale |= !quiet;
            self.calculate_entered(&mut st, &options);
        } else if demand != Demand::Everything && known && !st.unsettled {
            st.sync(self);
            if st.graph.as_ref().is_some_and(|g| g.outdated()) {
                st.graph = Some(DepGraph::build(self));
            }
            let (typed, stretched) = st.typed_now(self);
            let cells: Vec<Pos> = st.touched.iter().map(|t| *t & !REPLACED).collect();
            let changes = Changes {
                cells: &cells,
                rects: &st.rects,
                visibility: st.visibility,
                stretched: &stretched,
                typed: &typed,
                blocked: &st.blocked,
            };
            let scope = match demand {
                Demand::Sheet(sheet) => Scope::Sheet(sheet),
                _ => Scope::Workbook,
            };
            let graph = st.graph.as_ref();
            let check = graph.filter(|_| scope == Scope::Workbook && deps::verify());
            let outcome = self.on_calc_thread(|wb| match check {
                Some(graph) => eval::run_dirty_checked(wb, graph, &changes, &options),
                None => Ok(eval::run_dirty(wb, graph, &changes, scope, &options)),
            });
            let outcome = match outcome {
                Some(Err(difference)) => panic!("{difference}"),
                Some(Ok(result)) => Some(result),
                None => None,
            };
            match outcome {
                Some(Some(result)) if scope == Scope::Workbook => {
                    st.runs.1 += 1;
                    st.forget_changes();
                    st.typed = typed;
                    self.store(result, &mut st, false);
                }
                Some(Some(result)) => self.store(result, &mut st, true),
                // The order of calculation matters: in full, then.
                Some(None) => self.calculate_all(&mut st, structure, &options),
                None => st.untracked = true,
            }
        } else {
            // Everything is calculated. The graph is built anew unless it
            // is known to be right.
            let rebuild = demand == Demand::Everything || !known;
            if !rebuild {
                st.sync(self);
            }
            if rebuild || st.graph.as_ref().is_some_and(|g| g.outdated()) {
                st.graph = st.incremental.then(|| DepGraph::build(self));
            }
            self.calculate_all(&mut st, structure, &options);
        }
        self.calc = st;
    }

    fn calculate_all(&mut self, st: &mut CalcState, structure: Structure, options: &Options) {
        let graph = st.graph.as_ref();
        let Some(result) = self.on_calc_thread(|wb| eval::run_full(wb, graph, options)) else {
            st.untracked = true;
            return;
        };
        st.runs.0 += 1;
        st.forget_changes();
        st.structure = structure;
        st.typed = self.sheets.iter().map(|s| s.cell_extent()).collect();
        self.store(result, st, false);
    }

    /// Manual calculation: the formulas just entered are calculated, and
    /// nothing that depends on them. Everything stays on record for
    /// `calculate_now`.
    fn calculate_entered(&mut self, st: &mut CalcState, options: &Options) {
        let cells: Vec<Pos> = st.touched[st.settled..]
            .iter()
            .map(|t| *t & !REPLACED)
            .collect();
        let changes = Changes {
            cells: &cells,
            rects: &[],
            visibility: false,
            stretched: &[],
            typed: &[],
            blocked: &st.blocked,
        };
        if cells.is_empty() {
            return;
        }
        if let Some(Some(result)) =
            self.on_calc_thread(|wb| eval::run_dirty(wb, None, &changes, Scope::Entered, options))
        {
            self.store(result, st, true);
        }
    }

    /// Run a calculation on a thread of its own: formulas referring to one
    /// another recurse, and need more stack than the caller may have.
    fn on_calc_thread<T: Send>(&self, f: impl FnOnce(&Workbook) -> T + Send) -> Option<T> {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("recalc".into())
                .stack_size(512 << 20)
                .spawn_scoped(scope, || f(self))
                .ok()
                .and_then(|h| h.join().ok())
        })
    }

    /// Write the results of a calculation to the cells. A `partial`
    /// calculation adds to what is known of the workbook instead of
    /// replacing it.
    fn store(&mut self, result: CalcResult, st: &mut CalcState, partial: bool) {
        if let Some(violation) = result.audit {
            panic!("{violation}");
        }
        for ((si, r, c), v) in result.values {
            if let Some(cell) = self
                .sheets
                .get_mut(si)
                .and_then(|s| s.cells.get_mut(&(r, c)))
            {
                cell.value = v.into_scalar();
            }
        }
        for (sheet, list) in self.sheets.iter_mut().zip(result.spills) {
            sheet.spills = list
                .into_iter()
                .map(|(sp, array)| Spill {
                    row: sp.anchor.0,
                    col: sp.anchor.1,
                    array,
                })
                .collect();
        }
        st.blocked = result.blocked;
        if partial {
            self.circular |= result.circular;
            st.unsettled |= result.unsettled;
            st.rects.extend(result.moved);
        } else {
            self.circular = result.circular;
            st.unsettled = result.unsettled;
        }
    }
}
