//! Dependency tracking: which formulas read which cells.
//!
//! [`precedents`] derives from a formula's syntax tree the cells it reads.
//! [`DepGraph`] indexes them the other way round, from a cell to the
//! formulas reading it, so that after an edit only the formulas downstream
//! of what changed are calculated again (`eval::run_dirty`).
//!
//! The graph never under-approximates: when in doubt a formula is listed as
//! a reader. A reader listed by mistake only costs a calculation whose
//! result is the value the cell already had.

use crate::ast::{Expr, RefExpr};
use crate::eval::{CLAMP_CELLS, MAX_NAME_DEPTH};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::workbook::{Content, Workbook};
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};

/// A cell packed in 64 bits: sheet, row (20 bits), column (14 bits). Packed
/// positions sort in sheet order, row by row.
pub(crate) type Pos = u64;

pub(crate) fn pos(sheet: usize, row: u32, col: u32) -> Pos {
    ((sheet as u64) << 34) | ((row as u64) << 14) | col as u64
}

pub(crate) fn unpos(p: Pos) -> (usize, u32, u32) {
    (
        (p >> 34) as usize,
        ((p >> 14) & 0xF_FFFF) as u32,
        (p & 0x3FFF) as u32,
    )
}

/// Hasher for packed positions and other small integer keys. The standard
/// hasher is several times slower, and these maps are probed for every
/// cell a formula reads.
#[derive(Clone, Copy, Default)]
pub(crate) struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        // The multiplication leaves the low bits weak; the table index is
        // taken from them.
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;
pub(crate) type FastSet<K> = HashSet<K, BuildHasherDefault<FastHasher>>;

// ---- precedents of one formula ----------------------------------------------

/// What a formula reads, as far as its text tells.
#[derive(Clone, Debug, Default)]
pub(crate) struct Precedents {
    /// Cells and ranges, each on one sheet.
    pub areas: Vec<(usize, Rect)>,
    /// Calculated at every recalculation (`NOW`, `RAND`, `OFFSET`, ...).
    pub volatile: bool,
    /// May read cells that are not in `areas` (`OFFSET`, `INDIRECT`).
    pub dynamic: bool,
    /// Depends on which rows are hidden or filtered out (`SUBTOTAL`,
    /// `AGGREGATE`).
    pub visibility: bool,
    /// Draws random numbers: two calculations never agree on it.
    pub random: bool,
}

impl Precedents {
    pub fn covers(&self, sheet: usize, rect: &Rect) -> bool {
        self.areas.iter().any(|(s, area)| {
            *s == sheet
                && area.r0 <= rect.r0
                && area.c0 <= rect.c0
                && area.r1 >= rect.r1
                && area.c1 >= rect.c1
        })
    }
}

/// Functions whose result changes without any cell changing. Excel treats
/// the same ones as volatile, but for `GETPIVOTDATA`, which reads a
/// PivotTable rather than cells and is simply calculated every time here.
fn is_volatile(name: &str) -> bool {
    matches!(
        name,
        "NOW"
            | "TODAY"
            | "RAND"
            | "RANDBETWEEN"
            | "RANDARRAY"
            | "OFFSET"
            | "INDIRECT"
            | "CELL"
            | "INFO"
            | "GETPIVOTDATA"
    )
}

/// Functions that work out at calculation time which cells they read.
fn is_dynamic(name: &str) -> bool {
    matches!(name, "OFFSET" | "INDIRECT" | "CELL")
}

fn reads_visibility(name: &str) -> bool {
    matches!(name, "SUBTOTAL" | "AGGREGATE")
}

/// True for an argument that cannot be a reference.
fn is_literal(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Number(_)
            | Expr::Text(_)
            | Expr::Bool(_)
            | Expr::Error(_)
            | Expr::Missing
            | Expr::Value(_)
            | Expr::Array(_)
    )
}

struct Walker<'a> {
    wb: &'a Workbook,
    row: u32,
    col: u32,
    out: Precedents,
    /// Names being expanded, so that a name defined in terms of itself is
    /// expanded once.
    names: Vec<(usize, String)>,
}

impl Walker<'_> {
    fn sheet_of(&self, own: usize, r: &RefExpr) -> Option<usize> {
        match &r.sheet {
            None => Some(own),
            Some(name) => self.wb.sheet_index(name),
        }
    }

    fn walk(&mut self, sheet: usize, e: &Expr) {
        match e {
            Expr::Ref(r) => {
                if let Some(s) = self.sheet_of(sheet, r) {
                    self.out.areas.push((s, r.rect()));
                }
            }
            Expr::Ref3D(r) => {
                let ends = (self.wb.sheet_index(&r.first), self.wb.sheet_index(&r.last));
                if let (Some(a), Some(b)) = ends {
                    let rect = r.cells.rect();
                    for s in a.min(b)..=a.max(b) {
                        self.out.areas.push((s, rect));
                    }
                }
            }
            Expr::Name(name) => self.name(sheet, name, &mut |w, s, def| w.walk(s, def)),
            Expr::SheetName(of, name) => {
                if let Some(from) = self.wb.sheet_index(of) {
                    self.name(from, name, &mut |w, s, def| w.walk(s, def));
                }
            }
            Expr::Table(t) => {
                // The table as it stands: when it grows the graph is
                // rebuilt (see `Structure`).
                if let Ok(r) = crate::structured::resolve(self.wb, t, sheet, self.row, self.col) {
                    self.out.areas.push((r.sheet, r.rect));
                }
            }
            Expr::Func(name, args) => {
                self.out.volatile |= is_volatile(name);
                self.out.random |= matches!(&**name, "RAND" | "RANDBETWEEN" | "RANDARRAY");
                self.out.dynamic |= is_dynamic(name);
                self.out.visibility |= reads_visibility(name);
                match (&**name, args.as_slice()) {
                    // The third argument is read with the shape of the first.
                    ("SUMIF" | "AVERAGEIF", [range, _, sum]) => {
                        self.stretched(sheet, sum, range, false)
                    }
                    // A result vector shorter than the lookup vector is read
                    // past its end.
                    ("LOOKUP", [_, vector, result]) => self.stretched(sheet, result, vector, true),
                    _ => {}
                }
                for a in args {
                    self.walk(sheet, a);
                }
            }
            Expr::Unary(_, inner)
            | Expr::Percent(inner)
            | Expr::Paren(inner)
            | Expr::Spill(inner)
            | Expr::Implicit(inner) => self.walk(sheet, inner),
            Expr::Binary(_, a, b) | Expr::Intersect(a, b) => {
                self.walk(sheet, a);
                self.walk(sheet, b);
            }
            Expr::Union(items) => {
                for item in items {
                    self.walk(sheet, item);
                }
            }
            Expr::Array(rows) => {
                for item in rows.iter().flatten() {
                    self.walk(sheet, item);
                }
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Bool(_)
            | Expr::Error(_)
            | Expr::Missing
            | Expr::Value(_)
            | Expr::RefError(_) => {}
        }
    }

    /// Hand `f` what `name` stands for in a formula on `sheet`, the way
    /// `EvalCtx::with_name` resolves it: a defined name (its relative
    /// references taken from the formula's cell), else a table.
    fn name(&mut self, sheet: usize, name: &str, f: &mut dyn FnMut(&mut Self, usize, &Expr)) {
        if let Some((_, def)) = self.wb.lookup_name(sheet, name) {
            let key = (sheet, name.to_uppercase());
            if self.names.len() >= MAX_NAME_DEPTH as usize || self.names.contains(&key) {
                return;
            }
            self.names.push(key);
            if def.expr.has_relative_refs() {
                f(self, sheet, &def.expr_at(self.row, self.col));
            } else {
                f(self, sheet, &def.expr);
            }
            self.names.pop();
        } else if let Some((table_sheet, table)) = self.wb.table_by_name(name)
            && let Some(data) = table.data_range()
        {
            let cells = Expr::Ref(RefExpr {
                sheet: Some(self.wb.sheets[table_sheet].name.as_str().into()),
                ..RefExpr::range(data)
            });
            f(self, sheet, &cells);
        }
    }

    /// The cells an argument refers to when that can be told from the
    /// formula alone.
    fn static_area(&mut self, sheet: usize, e: &Expr) -> Option<(usize, Rect)> {
        match e {
            Expr::Ref(r) => Some((self.sheet_of(sheet, r)?, r.rect())),
            Expr::Paren(inner) => self.static_area(sheet, inner),
            Expr::Table(t) => crate::structured::resolve(self.wb, t, sheet, self.row, self.col)
                .ok()
                .map(|r| (r.sheet, r.rect)),
            Expr::Name(name) => {
                let mut found = None;
                self.name(sheet, name, &mut |w, s, def| found = w.static_area(s, def));
                found
            }
            Expr::SheetName(of, name) => {
                let from = self.wb.sheet_index(of)?;
                let mut found = None;
                self.name(from, name, &mut |w, s, def| found = w.static_area(s, def));
                found
            }
            _ => None,
        }
    }

    /// `read` is read from its top-left corner over the extent of `shape`:
    /// with the same number of rows and columns, or (`along`) as a row and
    /// as a column as long as the longer side of `shape`. When the cells
    /// cannot be told in advance the formula is calculated every time, as
    /// Excel does.
    fn stretched(&mut self, sheet: usize, read: &Expr, shape: &Expr, along: bool) {
        if is_literal(read) {
            return;
        }
        let (Some((s, base)), Some((_, shape))) = (
            self.static_area(sheet, read),
            self.static_area(sheet, shape),
        ) else {
            self.out.volatile = true;
            self.out.dynamic = true;
            return;
        };
        let to = |from: u32, len: u32, max: u32| {
            (from as u64 + len as u64 - 1).min(max as u64 - 1) as u32
        };
        if along {
            let len = shape.rows().max(shape.cols());
            self.out.areas.push((
                s,
                Rect::new(base.r0, base.c0, to(base.r0, len, MAX_ROWS), base.c0),
            ));
            self.out.areas.push((
                s,
                Rect::new(base.r0, base.c0, base.r0, to(base.c0, len, MAX_COLS)),
            ));
        } else {
            let (r1, c1) = (
                to(base.r0, shape.rows(), MAX_ROWS),
                to(base.c0, shape.cols(), MAX_COLS),
            );
            self.out
                .areas
                .push((s, Rect::new(base.r0, base.c0, r1, c1)));
        }
    }
}

/// The precedents of the formula `expr` held by the cell (`sheet`, `row`,
/// `col`): its references, on this and other sheets, each sheet of a 3-D
/// reference, every area of a union or an intersection, what its defined
/// names stand for, the current range of its structured references. A
/// spill reference `A1#` depends on its anchor, which is calculated again
/// whenever the spilled array changes.
pub(crate) fn precedents(
    wb: &Workbook,
    sheet: usize,
    row: u32,
    col: u32,
    expr: &Expr,
) -> Precedents {
    let mut walker = Walker {
        wb,
        row,
        col,
        out: Precedents::default(),
        names: Vec::new(),
    };
    walker.walk(sheet, expr);
    let mut out = walker.out;
    out.areas.retain(|(s, _)| *s < wb.sheets.len());
    out.areas
        .sort_unstable_by_key(|(s, r)| (*s, r.r0, r.c0, r.r1, r.c1));
    out.areas.dedup();
    out
}

/// True for a range large enough to be clamped to the used part of the
/// sheet when it is read (see `Calc::clamp_rect`).
pub(crate) fn is_clamped(rect: &Rect) -> bool {
    rect.rows() as u64 * rect.cols() as u64 > CLAMP_CELLS
}

// ---- the graph ----------------------------------------------------------------

/// Set on readers that are an index into `DepGraph::lists` rather than the
/// one reader itself.
const LIST: u64 = 1 << 63;

/// Add a reader to the readers of a cell or of a range: one reader is kept
/// in place, more of them in a list.
fn add_reader(lists: &mut Vec<Vec<Pos>>, readers: &mut u64, reader: Pos) {
    if *readers & LIST != 0 {
        let list = &mut lists[(*readers & !LIST) as usize];
        // A formula entered again registers again.
        if list.last() != Some(&reader) {
            list.push(reader);
        }
    } else if *readers != reader {
        lists.push(vec![*readers, reader]);
        *readers = LIST | (lists.len() as u64 - 1);
    }
}

/// A range and the formulas reading it.
#[derive(Clone, Copy)]
struct RangeReaders {
    rect: Rect,
    readers: u64,
}

/// The square holding a cell in the grid of a size class (`level`: the
/// squares are `2^level.0` rows by `2^level.1` columns), as a key.
fn bucket(sheet: usize, level: (u8, u8), row: u32, col: u32) -> u64 {
    ((sheet as u64) << 43)
        | ((level.0 as u64) << 38)
        | ((level.1 as u64) << 34)
        | (((row >> level.0) as u64) << 14)
        | (col >> level.1) as u64
}

/// The smallest `n` with `2^n >= len`.
fn level_of(len: u32) -> u8 {
    (32 - (len - 1).leading_zeros()) as u8
}

/// From cells to the formulas reading them.
///
/// Readers of single cells are kept in a hash map keyed by the cell.
/// Readers of ranges are filed by the size class of the range: a range at
/// most `2^i` rows by `2^j` columns lies in at most four squares of a grid
/// of that mesh, so the ranges containing a cell are found by looking at
/// one square per size class in use on the sheet, whatever the size of the
/// ranges (whole columns included).
///
/// Entries are not removed when a formula goes away: a reader that is no
/// longer one is harmless, and the graph is rebuilt once enough of it is
/// stale.
#[derive(Default)]
pub(crate) struct DepGraph {
    cells: FastMap<Pos, u64>,
    lists: Vec<Vec<Pos>>,
    ranges: FastMap<u64, Vec<RangeReaders>>,
    /// Size classes in use, per sheet.
    levels: Vec<Vec<(u8, u8)>>,
    /// Per sheet, formulas reading a range that is clamped to the rows
    /// (`tall`) or the columns (`wide`) holding data.
    tall: Vec<Vec<Pos>>,
    wide: Vec<Vec<Pos>>,
    volatile: FastSet<Pos>,
    dynamic: FastSet<Pos>,
    visibility: FastSet<Pos>,
    /// Formulas in the graph, and formulas replaced since it was built.
    formulas: usize,
    stale: usize,
    cell_edges: usize,
    range_edges: usize,
}

/// Size of the graph, for reporting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphStats {
    pub formulas: usize,
    /// Formula-to-cell and formula-to-range links.
    pub cell_links: usize,
    pub range_links: usize,
    /// Approximate memory held by the graph.
    pub bytes: usize,
}

impl DepGraph {
    pub fn build(wb: &Workbook) -> DepGraph {
        let mut graph = DepGraph {
            levels: vec![Vec::new(); wb.sheets.len()],
            tall: vec![Vec::new(); wb.sheets.len()],
            wide: vec![Vec::new(); wb.sheets.len()],
            ..DepGraph::default()
        };
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for ((row, col), cell) in sheet.cells.iter() {
                if let Content::Formula(expr) = &cell.content {
                    graph.add(wb, si, *row, *col, expr);
                    graph.formulas += 1;
                }
            }
        }
        graph
    }

    /// True when the graph carries so many stale entries that building it
    /// again pays off.
    pub fn outdated(&self) -> bool {
        self.stale > self.formulas / 2 + 4096
    }

    /// Bring the graph up to date with the content of one cell. `replaced`
    /// tells whether the cell held a formula before.
    pub fn update(&mut self, wb: &Workbook, at: Pos, replaced: bool) {
        self.volatile.remove(&at);
        self.dynamic.remove(&at);
        self.visibility.remove(&at);
        if replaced {
            // What the graph holds of the old formula stays behind.
            self.stale += 1;
            self.formulas = self.formulas.saturating_sub(1);
        }
        let (sheet, row, col) = unpos(at);
        if let Some(Content::Formula(expr)) = wb.cell(sheet, row, col).map(|c| &c.content) {
            self.add(wb, sheet, row, col, expr);
            self.formulas += 1;
        }
    }

    fn add(&mut self, wb: &Workbook, sheet: usize, row: u32, col: u32, expr: &Expr) {
        let reader = pos(sheet, row, col);
        let found = precedents(wb, sheet, row, col, expr);
        if found.volatile {
            self.volatile.insert(reader);
        }
        if found.dynamic {
            self.dynamic.insert(reader);
        }
        if found.visibility {
            self.visibility.insert(reader);
        }
        for (s, rect) in found.areas {
            if rect.is_single() {
                self.add_cell(pos(s, rect.r0, rect.c0), reader);
            } else {
                self.add_range(s, rect, reader);
            }
        }
    }

    fn add_cell(&mut self, cell: Pos, reader: Pos) {
        self.cell_edges += 1;
        match self.cells.get_mut(&cell) {
            None => {
                self.cells.insert(cell, reader);
            }
            Some(readers) => add_reader(&mut self.lists, readers, reader),
        }
    }

    fn add_range(&mut self, sheet: usize, rect: Rect, reader: Pos) {
        self.range_edges += 1;
        let level = (level_of(rect.rows()), level_of(rect.cols()));
        if !self.levels[sheet].contains(&level) {
            self.levels[sheet].push(level);
        }
        let rows = [rect.r0, rect.r1];
        let cols = [rect.c0, rect.c1];
        let mut done = [u64::MAX; 4];
        for (i, (row, col)) in rows
            .iter()
            .flat_map(|r| cols.iter().map(move |c| (*r, *c)))
            .enumerate()
        {
            let key = bucket(sheet, level, row, col);
            done[i] = key;
            if done[..i].contains(&key) {
                continue;
            }
            let square = self.ranges.entry(key).or_default();
            // A formula filled down a column reads the same absolute range
            // on every row: such a range is listed once, with all its
            // readers.
            match square
                .iter_mut()
                .rev()
                .take(4)
                .find(|known| known.rect == rect)
            {
                Some(known) => add_reader(&mut self.lists, &mut known.readers, reader),
                None => square.push(RangeReaders {
                    rect,
                    readers: reader,
                }),
            }
        }
        if is_clamped(&rect) {
            self.tall[sheet].push(reader);
            if rect.c0 < rect.c1 {
                self.wide[sheet].push(reader);
            }
        }
    }

    fn emit(&self, readers: u64, f: &mut dyn FnMut(Pos)) {
        if readers & LIST != 0 {
            for reader in &self.lists[(readers & !LIST) as usize] {
                f(*reader);
            }
        } else {
            f(readers);
        }
    }

    /// Call `f` with every formula reading the cell.
    pub fn readers_of_cell(&self, cell: Pos, f: &mut dyn FnMut(Pos)) {
        if let Some(readers) = self.cells.get(&cell) {
            self.emit(*readers, f);
        }
        let (sheet, row, col) = unpos(cell);
        let Some(levels) = self.levels.get(sheet) else {
            return;
        };
        for level in levels {
            if let Some(list) = self.ranges.get(&bucket(sheet, *level, row, col)) {
                for known in list {
                    if known.rect.contains(row, col) {
                        self.emit(known.readers, f);
                    }
                }
            }
        }
    }

    /// Call `f` with every formula reading a cell of `rect` (possibly more
    /// than once).
    pub fn readers_of_rect(&self, sheet: usize, rect: &Rect, f: &mut dyn FnMut(Pos)) {
        if rect.is_single() {
            return self.readers_of_cell(pos(sheet, rect.r0, rect.c0), f);
        }
        // Whichever is shorter: look every cell of the rectangle up, or go
        // through every cell that has readers.
        if rect.rows() as u64 * rect.cols() as u64 <= self.cells.len() as u64 {
            for row in rect.r0..=rect.r1 {
                for col in rect.c0..=rect.c1 {
                    if let Some(readers) = self.cells.get(&pos(sheet, row, col)) {
                        self.emit(*readers, f);
                    }
                }
            }
        } else {
            for (cell, readers) in &self.cells {
                let (s, row, col) = unpos(*cell);
                if s == sheet && rect.contains(row, col) {
                    self.emit(*readers, f);
                }
            }
        }
        let Some(levels) = self.levels.get(sheet) else {
            return;
        };
        let squares = |level: &(u8, u8)| {
            ((rect.r1 >> level.0) - (rect.r0 >> level.0) + 1) as u64
                * ((rect.c1 >> level.1) - (rect.c0 >> level.1) + 1) as u64
        };
        if levels.iter().map(squares).sum::<u64>() <= self.ranges.len() as u64 {
            for level in levels {
                for row in (rect.r0 >> level.0)..=(rect.r1 >> level.0) {
                    for col in (rect.c0 >> level.1)..=(rect.c1 >> level.1) {
                        let key = bucket(sheet, *level, row << level.0, col << level.1);
                        for known in self.ranges.get(&key).into_iter().flatten() {
                            if known.rect.intersects(rect) {
                                self.emit(known.readers, f);
                            }
                        }
                    }
                }
            }
        } else {
            for (key, list) in &self.ranges {
                if (*key >> 43) as usize != sheet {
                    continue;
                }
                for known in list {
                    if known.rect.intersects(rect) {
                        self.emit(known.readers, f);
                    }
                }
            }
        }
    }

    pub fn volatile(&self) -> &FastSet<Pos> {
        &self.volatile
    }

    /// Formulas that may read cells other than their precedents.
    pub fn dynamic(&self) -> &FastSet<Pos> {
        &self.dynamic
    }

    pub fn visibility(&self) -> &FastSet<Pos> {
        &self.visibility
    }

    /// Formulas reading a range of `sheet` that is clamped to its last row
    /// holding data.
    pub fn tall(&self, sheet: usize) -> &[Pos] {
        self.tall.get(sheet).map_or(&[], |v| v.as_slice())
    }

    /// Formulas reading a range of `sheet` that is clamped to its last
    /// column holding data.
    pub fn wide(&self, sheet: usize) -> &[Pos] {
        self.wide.get(sheet).map_or(&[], |v| v.as_slice())
    }

    pub fn stats(&self) -> GraphStats {
        use std::mem::size_of;
        let table = |capacity: usize, entry: usize| capacity * (entry + 1);
        let lists: usize = self
            .lists
            .iter()
            .map(|l| l.capacity() * size_of::<Pos>() + size_of::<Vec<Pos>>())
            .sum();
        let ranges: usize = self
            .ranges
            .values()
            .map(|l| l.capacity() * size_of::<RangeReaders>())
            .sum();
        let sets = [&self.volatile, &self.dynamic, &self.visibility];
        let per_sheet = |lists: &[Vec<Pos>]| {
            lists
                .iter()
                .map(|l| l.capacity() * size_of::<Pos>())
                .sum::<usize>()
        };
        GraphStats {
            formulas: self.formulas,
            cell_links: self.cell_edges,
            range_links: self.range_edges,
            bytes: table(self.cells.capacity(), size_of::<(Pos, u64)>())
                + lists
                + table(
                    self.ranges.capacity(),
                    size_of::<(u64, Vec<RangeReaders>)>(),
                )
                + ranges
                + sets
                    .iter()
                    .map(|s| table(s.capacity(), size_of::<Pos>()))
                    .sum::<usize>()
                + per_sheet(&self.tall)
                + per_sheet(&self.wide),
        }
    }
}

// ---- checking the precedents ----------------------------------------------------

static AUDIT: AtomicBool = AtomicBool::new(cfg!(test));
static VERIFY: AtomicBool = AtomicBool::new(cfg!(test));

/// The environment variable that switches both checks on in any program
/// using the engine: `OPENSHEET_CHECK_RECALC=1 cargo test`.
fn asked_for() -> bool {
    static ASKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ASKED.get_or_init(|| std::env::var_os("OPENSHEET_CHECK_RECALC").is_some())
}

/// When on, every recalculation checks that what a formula reads is among
/// the precedents derived from its text, and panics otherwise. On in the
/// engine's unit tests; the differential tests switch it on as well.
#[doc(hidden)]
pub fn set_audit(on: bool) {
    AUDIT.store(on, Ordering::Relaxed);
}

pub(crate) fn audit() -> bool {
    AUDIT.load(Ordering::Relaxed) || asked_for()
}

/// When on, every incremental recalculation is followed by a full one of
/// the same workbook, and the engine panics if they differ on the value of
/// a cell, a spilled array or the circular-reference warning. On in the
/// engine's unit tests.
#[doc(hidden)]
pub fn set_verify(on: bool) {
    VERIFY.store(on, Ordering::Relaxed);
}

pub(crate) fn verify() -> bool {
    VERIFY.load(Ordering::Relaxed) || asked_for()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;

    fn areas(wb: &Workbook, sheet: usize, at: &str, formula: &str) -> Vec<String> {
        let (row, col) = crate::refs::parse_cell_name(at).unwrap();
        let expr = crate::parser::parse_formula(formula, wb.locale).unwrap();
        precedents(wb, sheet, row, col, &expr)
            .areas
            .iter()
            .map(|(s, r)| format!("{}!{}", wb.sheets[*s].name, r.name()))
            .collect()
    }

    #[test]
    fn positions_pack_in_sheet_order() {
        let cells = [
            (0, 0, 0),
            (0, 0, 16_383),
            (0, 1, 0),
            (0, 1_048_575, 16_383),
            (1, 0, 0),
            (7, 12, 5),
        ];
        for cell in cells {
            assert_eq!(unpos(pos(cell.0, cell.1, cell.2)), cell);
        }
        let packed: Vec<Pos> = cells.iter().map(|c| pos(c.0, c.1, c.2)).collect();
        assert!(packed.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn precedents_of_every_kind_of_reference() {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        wb.add_sheet(2);
        wb.define_name(
            "Rate",
            crate::parser::parse_formula("Sheet2!$B$1", wb.locale).unwrap(),
        );
        wb.define_name(
            "Twice",
            crate::parser::parse_formula("Rate*2+Sheet1!$C$3", wb.locale).unwrap(),
        );
        wb.define_name(
            "Left",
            crate::parser::parse_formula("Sheet1!A1", wb.locale).unwrap(),
        );
        wb.commit("setup");
        assert_eq!(
            areas(&wb, 0, "D4", "A1+B2:C3*2"),
            ["Sheet1!A1", "Sheet1!B2:C3"]
        );
        assert_eq!(
            areas(&wb, 0, "D4", "SUM(A:A,3:4)"),
            ["Sheet1!A:A", "Sheet1!3:4"]
        );
        assert_eq!(
            areas(&wb, 0, "D4", "Sheet2!A1+SUM(Sheet2:Sheet3!B2)"),
            ["Sheet2!A1", "Sheet2!B2", "Sheet3!B2"]
        );
        assert_eq!(
            areas(&wb, 0, "D4", "SUM((A1:A3,C1:C3) A2:C2)"),
            ["Sheet1!A1:A3", "Sheet1!C1:C3", "Sheet1!A2:C2"]
        );
        assert_eq!(areas(&wb, 0, "D4", "Twice"), ["Sheet1!C3", "Sheet2!B1"]);
        // A relative name is read from the cell using it.
        assert_eq!(areas(&wb, 0, "D4", "Left"), ["Sheet1!D4"]);
        assert_eq!(areas(&wb, 1, "B2", "Left+A1#"), ["Sheet1!B2", "Sheet2!A1"]);
        assert_eq!(
            areas(&wb, 0, "D4", "NoSuchSheet!A1+NoSuchName"),
            Vec::<String>::new()
        );
        // The third argument of SUMIF is read with the shape of the first.
        assert_eq!(
            areas(&wb, 0, "D4", "SUMIF(A1:A10,\">0\",C5)"),
            ["Sheet1!A1:A10", "Sheet1!C5", "Sheet1!C5:C14"]
        );
        assert_eq!(
            areas(&wb, 0, "D4", "LOOKUP(1,A1:A4,C1:C2)"),
            [
                "Sheet1!A1:A4",
                "Sheet1!C1:F1",
                "Sheet1!C1:C2",
                "Sheet1!C1:C4"
            ]
        );
    }

    #[test]
    fn flags() {
        let wb = Workbook::new(Locale::en());
        let of = |formula: &str| {
            let expr = crate::parser::parse_formula(formula, wb.locale).unwrap();
            let p = precedents(&wb, 0, 0, 0, &expr);
            (p.volatile, p.dynamic, p.visibility)
        };
        assert_eq!(of("A1+1"), (false, false, false));
        assert_eq!(of("IF(A1,NOW(),2)"), (true, false, false));
        assert_eq!(of("SUM(OFFSET(A1,1,1))"), (true, true, false));
        assert_eq!(of("SUBTOTAL(9,B1:B9)"), (false, false, true));
        assert_eq!(of("SUMIF(A1:A9,1,INDEX(B:B,3))"), (true, true, false));
    }

    #[test]
    fn readers_are_found_through_cells_and_ranges() {
        let mut wb = Workbook::new(Locale::en());
        let formulas = [
            ("C1", "=A1+1"),
            ("C2", "=A1*2"),
            ("C3", "=SUM(A1:A10)"),
            ("C4", "=SUM(A:A)"),
            ("C5", "=SUM(A5:B300)"),
            ("C6", "=SUM(2:2)"),
            ("C7", "=B7"),
            ("C8", "=SUM(A:B)"),
        ];
        for (at, text) in formulas {
            let (row, col) = crate::refs::parse_cell_name(at).unwrap();
            wb.set_cell_text(0, row, col, text).unwrap();
        }
        let graph = DepGraph::build(&wb);
        let readers = |at: &str| {
            let (row, col) = crate::refs::parse_cell_name(at).unwrap();
            let mut found = Vec::new();
            graph.readers_of_cell(pos(0, row, col), &mut |p| {
                found.push(crate::refs::cell_name(unpos(p).1, unpos(p).2))
            });
            found.sort();
            found
        };
        assert_eq!(readers("A1"), ["C1", "C2", "C3", "C4", "C8"]);
        assert_eq!(readers("A2"), ["C3", "C4", "C6", "C8"]);
        assert_eq!(readers("A7"), ["C3", "C4", "C5", "C8"]);
        assert_eq!(readers("B200"), ["C5", "C8"]);
        assert_eq!(readers("A500000"), ["C4", "C8"]);
        assert_eq!(readers("Z2"), ["C6"]);
        assert_eq!(readers("Z3"), Vec::<String>::new());
        let over = |range: &str| {
            let mut found = Vec::new();
            graph.readers_of_rect(0, &Rect::parse(range).unwrap(), &mut |p| {
                found.push(crate::refs::cell_name(unpos(p).1, unpos(p).2))
            });
            found.sort();
            found.dedup();
            found
        };
        assert_eq!(over("B6:B8"), ["C5", "C7", "C8"]);
        assert_eq!(
            over("A1:XFD1048576"),
            ["C1", "C2", "C3", "C4", "C5", "C6", "C7", "C8"]
        );
        assert_eq!(over("D1:E9"), ["C6"]);
        assert_eq!(graph.tall(0).len(), 2);
        assert_eq!(graph.wide(0).len(), 1);
    }
}
