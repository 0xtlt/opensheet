//! What the conditional formats of a sheet do to one cell.
//!
//! [`result_at`] is called for every visible cell at every repaint, so
//! nothing here may cost more than a few lookups per call. Whatever a rule
//! needs from its whole range (the tenth largest number, the average, the
//! ends of a colour scale, which values repeat) is computed the first time
//! a cell of the rule is drawn and kept until the workbook recalculates or
//! the rules change. Formulas are evaluated a block of cells at a time and
//! their answers kept as one bit per cell.

use super::formula::{self, Position};
use super::manage::disjoint;
use super::model::{
    BarAxis, BarDirection, CellOp, Cfvo, ColorScale, CondFormat, DataBar, Icon, IconRule, IconSet,
    Rule, TextOp, TimePeriod,
};
use crate::ast::Expr;
use crate::datetime;
use crate::eval::{compare_text, compare_values};
use crate::functions::wildcard_match;
use crate::input::parse_numeric_text;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Color, StylePatch};
use crate::value::{ErrorKind, Value};
use crate::workbook::{Content, Sheet, Workbook};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering as Atomic};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

// ---- what the grid gets --------------------------------------------------------

/// A data bar, ready to draw. Positions are fractions of the cell's width
/// measured from its left edge (the grid insets the bar by its own
/// padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    /// Where the bar starts: the edge of the cell or the axis.
    pub start: f32,
    /// Where the bar ends: its tip. Smaller than `start` when the bar grows
    /// leftwards; equal to it for a bar of no length.
    pub end: f32,
    pub fill: Color,
    /// The fill fades from `fill` at `start` to nearly white at `end`.
    pub gradient: bool,
    /// Outline of the bar, when it has one.
    pub border: Option<Color>,
    /// The bar stands for a negative number (its colours are already the
    /// negative ones).
    pub negative: bool,
    /// Where the axis crosses the cell, when the rule has one: a vertical
    /// dashed line over the whole height.
    pub axis: Option<f32>,
    pub axis_color: Color,
}

/// An icon of an icon set, drawn at the left of the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellIcon {
    pub set: IconSet,
    /// Index into `set.icons()`: 0 is the icon of the lowest numbers.
    pub index: u8,
}

impl CellIcon {
    /// The picture to draw.
    pub fn icon(self) -> Icon {
        self.set.icons()[self.index as usize]
    }
}

/// What a conditional format draws inside a cell besides restyling it.
#[derive(Clone, Debug, PartialEq)]
pub struct Adornment {
    pub bar: Option<Bar>,
    pub icon: Option<CellIcon>,
    /// "Show Bar Only" / "Show Icon Only": the cell's own text is not drawn.
    pub hide_value: bool,
}

/// What the conditional formats of a sheet do to one cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CondResult {
    /// Laid over the cell's own format. The fill of a colour scale is here
    /// too.
    pub patch: StylePatch,
    pub adornment: Option<Adornment>,
}

/// The effect of the sheet's conditional formats on a cell; `None` when no
/// rule changes anything. Rules are looked at in priority order: each
/// format property comes from the first matching rule that sets it, a cell
/// gets at most one colour scale, one data bar and one icon, and a matching
/// rule with "Stop If True" hides the rules after it.
pub fn result_at(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<CondResult> {
    let sh = wb.sheets.get(sheet)?;
    if sh.cond_formats.is_empty() {
        return None;
    }
    let cache = &wb.caches.cond;
    let epoch = cache.epoch.load(Atomic::Relaxed);
    let mut state = cache.state.lock().unwrap_or_else(PoisonError::into_inner);
    let state = &mut *state;
    // One slot per sheet of the workbook, no more: sheets come and go.
    if state.sheets.len() != wb.sheets.len() {
        state.sheets.resize_with(wb.sheets.len(), || None);
    }
    let rules = (sh.cond_formats.as_ptr() as usize, sh.cond_formats.len());
    // The clock is only read for sheets whose rules can depend on it.
    let today = match &state.sheets[sheet] {
        Some(sc) if !sc.uses_clock && sc.key.same_rules(wb.generation, epoch, rules) => {
            sc.key.today
        }
        _ => state.clock.today(),
    };
    let key = Key {
        generation: wb.generation,
        epoch,
        rules,
        today,
    };
    let sc = match &mut state.sheets[sheet] {
        Some(sc) if sc.key == key => sc,
        slot => slot.insert(SheetCache::new(key, &sh.cond_formats)),
    };
    let cx = Cx {
        wb,
        sheet,
        sh,
        today,
    };
    // The rules the cell belongs to, in priority order (the list is kept
    // from call to call to spare an allocation).
    let mut applying = std::mem::take(&mut sc.applying);
    applying.clear();
    match &sc.index {
        Some(index) => index.rules_at(row, col, &mut applying),
        None => applying.extend(
            (0..rules.1 as u32).filter(|i| sh.cond_formats[*i as usize].applies_to(row, col)),
        ),
    }
    let mut acc = Acc::default();
    if !applying.is_empty() {
        let v = sh.value_at(row, col);
        for &i in &applying {
            let cf = &sh.cond_formats[i as usize];
            let data = sc.rules[i as usize].get_or_insert_with(|| prepare(&cx, cf));
            if apply(&cx, cf, data, row, col, &v, &mut acc) && cf.stop_if_true {
                break;
            }
        }
    }
    sc.applying = applying;
    acc.finish()
}

// ---- cache ---------------------------------------------------------------------

/// Cache of what the rules derive from the cells (see `features::Caches`).
/// It follows `Workbook::generation`, so anything that ends in `commit`,
/// `undo`, `redo` or `recalc` refreshes it, as do the functions of this
/// module that change rules. Code that edits `Sheet::cond_formats` in place
/// without recalculating calls [`Cache::invalidate`].
#[derive(Default)]
pub struct Cache {
    state: Mutex<State>,
    /// Counts the calls to `invalidate`.
    epoch: AtomicU64,
}

impl Cache {
    /// Forget everything derived from the rules and the cells.
    pub fn invalidate(&self) {
        self.epoch.fetch_add(1, Atomic::Relaxed);
    }

    /// Fix the date the "A Date Occurring" rules count from, as a serial
    /// number (for tests and reproducible pictures); `None` follows the
    /// computer's clock again. Formulas calling `TODAY()` are not affected.
    pub fn set_today(&self, today: Option<f64>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.clock.fixed = today.map(f64::floor);
    }
}

impl fmt::Debug for Cache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cache").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct State {
    /// By sheet index.
    sheets: Vec<Option<SheetCache>>,
    clock: Clock,
}

/// Today's date, read from the system at most once a second: rules that
/// depend on the date change at midnight without any recalculation.
#[derive(Default)]
struct Clock {
    fixed: Option<f64>,
    read: Option<(Instant, f64)>,
}

impl Clock {
    fn today(&mut self) -> f64 {
        if let Some(today) = self.fixed {
            return today;
        }
        let now = Instant::now();
        match self.read {
            Some((at, today)) if now.duration_since(at) < Duration::from_secs(1) => today,
            _ => {
                let today = datetime::now_serial().floor();
                self.read = Some((now, today));
                today
            }
        }
    }
}

/// What a sheet's cache was computed from.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    generation: u64,
    epoch: u64,
    /// Address and length of the rule list: undo, redo and rollback swap
    /// the whole list for another one.
    rules: (usize, usize),
    today: f64,
}

impl Key {
    fn same_rules(&self, generation: u64, epoch: u64, rules: (usize, usize)) -> bool {
        self.generation == generation && self.epoch == epoch && self.rules == rules
    }
}

struct SheetCache {
    key: Key,
    /// `None` when the index would be too large: every rule is then tried
    /// for every cell.
    index: Option<Index>,
    /// By rule index; filled when a cell of the rule is first asked for.
    rules: Vec<Option<RuleData>>,
    /// Some rule depends on the date or evaluates a formula (which may).
    uses_clock: bool,
    /// Scratch list of `result_at`.
    applying: Vec<u32>,
}

impl SheetCache {
    fn new(key: Key, rules: &[CondFormat]) -> SheetCache {
        SheetCache {
            key,
            index: Index::new(rules),
            rules: rules.iter().map(|_| None).collect(),
            uses_clock: rules
                .iter()
                .any(|cf| matches!(cf.rule, Rule::DateOccurring(_)) || !cf.rule.exprs().is_empty()),
            applying: Vec::new(),
        }
    }
}

/// Which ranges cross each row. Files that went through years of copy and
/// paste hold rules crumbled into thousands of ranges; trying each of them
/// for every cell would be felt.
struct Index {
    /// First row of each band of rows, ascending, starting with row 0. No
    /// range starts or ends inside a band.
    starts: Vec<u32>,
    /// By band: the ranges crossing it as (rule, first column, last
    /// column), in priority order.
    bands: Vec<Vec<(u32, u32, u32)>>,
}

impl Index {
    /// Above this many entries the index would cost more than it saves.
    const LIMIT: usize = 2_000_000;

    fn new(rules: &[CondFormat]) -> Option<Index> {
        let mut starts = vec![0];
        for r in rules.iter().flat_map(|cf| &cf.ranges) {
            starts.push(r.r0);
            if r.r1 < MAX_ROWS - 1 {
                starts.push(r.r1 + 1);
            }
        }
        starts.sort_unstable();
        starts.dedup();
        let mut bands = vec![Vec::new(); starts.len()];
        let mut entries = 0;
        for (i, cf) in rules.iter().enumerate() {
            for r in &cf.ranges {
                let first = starts.partition_point(|s| *s < r.r0);
                let end = starts.partition_point(|s| *s <= r.r1);
                entries += end - first;
                if entries > Index::LIMIT {
                    return None;
                }
                for band in &mut bands[first..end] {
                    band.push((i as u32, r.c0, r.c1));
                }
            }
        }
        Some(Index { starts, bands })
    }

    /// The rules a cell belongs to, in priority order.
    fn rules_at(&self, row: u32, col: u32, out: &mut Vec<u32>) {
        let band = &self.bands[self.starts.partition_point(|s| *s <= row) - 1];
        for &(rule, c0, c1) in band {
            // Ranges of one rule may overlap: the rule still counts once.
            if col >= c0 && col <= c1 && out.last() != Some(&rule) {
                out.push(rule);
            }
        }
    }
}

/// What a rule keeps between repaints.
enum RuleData {
    /// Nothing: the test only looks at the cell.
    Direct,
    CellIs {
        first: Operand,
        second: Option<Operand>,
        cells: Bits,
    },
    Formula {
        /// The answer, when it is the same for every cell.
        fixed: Option<bool>,
        opaque: bool,
        cells: Bits,
    },
    Text {
        text: Operand,
        cells: Bits,
    },
    /// Top / bottom: the number a cell must reach. `None` = no number in
    /// the ranges.
    Rank(Option<f64>),
    /// Above / below average: the number a cell is compared with.
    Average(Option<f64>),
    /// Duplicate / unique values: the cells that match.
    Cells(HashSet<(u32, u32)>),
    /// The value and colour of each stop. `None` here and below = nothing
    /// to draw (no number in the ranges, or a threshold that is an error).
    Scale(Option<Vec<(f64, Color)>>),
    /// The numbers shown with the shortest and the longest bar.
    Bar(Option<(f64, f64)>),
    /// The floor of each band.
    Icons(Option<Vec<f64>>),
}

/// An operand of a rule, as far as it could be worked out ahead.
enum Operand {
    /// The same value for every cell.
    Fixed(Value),
    /// A formula to evaluate for each cell.
    PerCell { opaque: bool },
}

const BLOCK_ROWS: u32 = 32;
const BLOCK_COLS: u32 = 32;
const BLOCK_WORDS: usize = (BLOCK_ROWS * BLOCK_COLS / 64) as usize;

/// One yes / no answer per cell, worked out a block of cells at a time:
/// evaluating formulas for a block costs little more than for one cell, and
/// scrolling then finds most answers ready.
#[derive(Default)]
struct Bits {
    blocks: HashMap<(u32, u32), [u64; BLOCK_WORDS]>,
}

impl Bits {
    /// The answer for a cell. `fill` gives the answers for the cells of the
    /// rule in a block the first time one of them is asked for.
    fn get(
        &mut self,
        row: u32,
        col: u32,
        ranges: &[Rect],
        fill: &mut dyn FnMut(&[(u32, u32)]) -> Vec<bool>,
    ) -> bool {
        let bit = |r: u32, c: u32| ((r % BLOCK_ROWS) * BLOCK_COLS + c % BLOCK_COLS) as usize;
        let block = (row / BLOCK_ROWS, col / BLOCK_COLS);
        let words = self.blocks.entry(block).or_insert_with(|| {
            let (r0, c0) = (block.0 * BLOCK_ROWS, block.1 * BLOCK_COLS);
            let area = Rect::new(
                r0,
                c0,
                (r0 + BLOCK_ROWS - 1).min(MAX_ROWS - 1),
                (c0 + BLOCK_COLS - 1).min(MAX_COLS - 1),
            );
            let mut cells = Vec::new();
            for part in ranges.iter().filter_map(|r| r.intersection(&area)) {
                cells.extend(
                    (part.r0..=part.r1).flat_map(|r| (part.c0..=part.c1).map(move |c| (r, c))),
                );
            }
            cells.sort_unstable();
            cells.dedup();
            let mut words = [0; BLOCK_WORDS];
            for (&(r, c), yes) in cells.iter().zip(fill(&cells)) {
                if yes {
                    words[bit(r, c) / 64] |= 1 << (bit(r, c) % 64);
                }
            }
            words
        });
        words[bit(row, col) / 64] & (1 << (bit(row, col) % 64)) != 0
    }
}

// ---- preparing a rule ------------------------------------------------------------

/// The sheet a result is computed for.
struct Cx<'a> {
    wb: &'a Workbook,
    sheet: usize,
    sh: &'a Sheet,
    /// Serial number of today's date.
    today: f64,
}

fn prepare(cx: &Cx, cf: &CondFormat) -> RuleData {
    let anchor = cf.anchor();
    let operand = |e: &Expr| match formula::literal(e) {
        Some(v) => Operand::Fixed(v),
        None => match formula::position(cx.wb, e) {
            Position::Fixed => Operand::Fixed(formula::eval_at(cx.wb, cx.sheet, anchor, e)),
            Position::Relative => Operand::PerCell { opaque: false },
            Position::Opaque => Operand::PerCell { opaque: true },
        },
    };
    match &cf.rule {
        Rule::CellIs { first, second, .. } => RuleData::CellIs {
            first: operand(first),
            second: second.as_ref().map(operand),
            cells: Bits::default(),
        },
        Rule::Formula(e) => {
            let position = formula::position(cx.wb, e);
            RuleData::Formula {
                fixed: (position == Position::Fixed)
                    .then(|| is_true(&formula::eval_at(cx.wb, cx.sheet, anchor, e))),
                opaque: position == Position::Opaque,
                cells: Bits::default(),
            }
        }
        Rule::Text { text, .. } => RuleData::Text {
            text: operand(text),
            cells: Bits::default(),
        },
        Rule::TopBottom {
            rank,
            percent,
            bottom,
        } => RuleData::Rank(rank_threshold(
            numbers(cx.sh, &cf.ranges),
            *rank,
            *percent,
            *bottom,
        )),
        Rule::Average { above, std_dev, .. } => RuleData::Average(average_threshold(
            &numbers(cx.sh, &cf.ranges),
            *above,
            *std_dev,
        )),
        Rule::DuplicateValues => RuleData::Cells(repeated(cx, &cf.ranges, true)),
        Rule::UniqueValues => RuleData::Cells(repeated(cx, &cf.ranges, false)),
        Rule::ColorScale(scale) => RuleData::Scale(scale_stops(cx, cf, scale)),
        Rule::DataBar(bar) => RuleData::Bar(bar_ends(cx, cf, bar)),
        Rule::IconSet(icons) => RuleData::Icons(icon_floors(cx, cf, icons)),
        Rule::Blanks | Rule::NoBlanks | Rule::Errors | Rule::NoErrors | Rule::DateOccurring(_) => {
            RuleData::Direct
        }
    }
}

/// Call `f` for every cell of the ranges that shows a value, once per cell.
fn for_each_value(sh: &Sheet, ranges: &[Rect], f: &mut dyn FnMut(u32, u32, &Value)) {
    for rect in disjoint(ranges) {
        let area = rect.rows() as u64 * rect.cols() as u64;
        // A small range on a big sheet is read cell by cell; otherwise the
        // stored cells of its rows are walked (whole columns stay cheap).
        if area.saturating_mul(8) < sh.cells.len() as u64 {
            for row in rect.r0..=rect.r1 {
                for col in rect.c0..=rect.c1 {
                    if let Some(cell) = sh
                        .cells
                        .get(&(row, col))
                        .filter(|c| !matches!(c.content, Content::Empty))
                    {
                        f(row, col, &cell.value);
                    }
                }
            }
        } else {
            for ((row, col), cell) in sh.cells.range((rect.r0, rect.c0)..=(rect.r1, rect.c1)) {
                if *col >= rect.c0 && *col <= rect.c1 && !matches!(cell.content, Content::Empty) {
                    f(*row, *col, &cell.value);
                }
            }
        }
        for sp in &sh.spills {
            let spilled = Rect::new(
                sp.row,
                sp.col,
                sp.row + sp.array.rows as u32 - 1,
                sp.col + sp.array.cols as u32 - 1,
            );
            let Some(part) = spilled.intersection(&rect) else {
                continue;
            };
            for row in part.r0..=part.r1 {
                for col in part.c0..=part.c1 {
                    // The first element is the value of the formula's cell,
                    // seen above.
                    if (row, col) != (sp.row, sp.col) {
                        f(
                            row,
                            col,
                            sp.array
                                .get((row - sp.row) as usize, (col - sp.col) as usize),
                        );
                    }
                }
            }
        }
    }
}

/// The numbers in the ranges. Text, booleans, blanks and errors take no
/// part in ranks, averages and scales.
pub(super) fn numbers(sh: &Sheet, ranges: &[Rect]) -> Vec<f64> {
    let mut out = Vec::new();
    for_each_value(sh, ranges, &mut |_, _, v| {
        if let Value::Number(n) = v {
            out.push(*n);
        }
    });
    out
}

/// The number a cell must reach to be among the top (or bottom) `rank`
/// items or percent. Numbers equal to the last one that qualifies qualify
/// too, as `x >= LARGE(range, rank)` would have it.
fn rank_threshold(mut numbers: Vec<f64>, rank: u32, percent: bool, bottom: bool) -> Option<f64> {
    let count = numbers.len() as u64;
    if count == 0 || rank == 0 {
        return None;
    }
    // A percentage of the cells rounds down, but never to nothing.
    let wanted = if percent {
        (count * rank as u64 / 100).max(1)
    } else {
        rank as u64
    };
    let last = wanted.min(count) as usize - 1;
    let (_, nth, _) = numbers.select_nth_unstable_by(last, |a, b| {
        if bottom {
            a.total_cmp(b)
        } else {
            b.total_cmp(a)
        }
    });
    Some(*nth)
}

/// The average of the numbers, moved by `std_dev` standard deviations (of
/// the whole population) up for "above" rules and down for "below" ones.
fn average_threshold(numbers: &[f64], above: bool, std_dev: u32) -> Option<f64> {
    if numbers.is_empty() {
        return None;
    }
    let count = numbers.len() as f64;
    let mean = numbers.iter().sum::<f64>() / count;
    if std_dev == 0 {
        return Some(mean);
    }
    let variance = numbers.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / count;
    let spread = variance.sqrt() * std_dev as f64;
    Some(if above { mean + spread } else { mean - spread })
}

/// What makes two cells hold "the same value" for the duplicate rules.
#[derive(PartialEq, Eq, Hash)]
enum Same {
    Number(u64),
    Text(String),
    Bool(bool),
    Error(ErrorKind),
}

/// The cells whose value occurs more than once in the ranges (`twice`) or
/// exactly once. Blank cells never count. Text is compared without case,
/// and text that reads as a number equals that number, as in `COUNTIF`.
fn repeated(cx: &Cx, ranges: &[Rect], twice: bool) -> HashSet<(u32, u32)> {
    let locale = cx.wb.locale;
    let number = |n: f64| Same::Number(if n == 0.0 { 0 } else { n.to_bits() });
    let mut ids: HashMap<Same, usize> = HashMap::new();
    let mut counts: Vec<u32> = Vec::new();
    let mut cells: Vec<((u32, u32), usize)> = Vec::new();
    for_each_value(cx.sh, ranges, &mut |row, col, v| {
        let same = match v {
            Value::Number(n) => number(*n),
            Value::Text(s) if s.is_empty() => return,
            // Only text holding a digit can read as a number.
            Value::Text(s) => match s
                .bytes()
                .any(|b| b.is_ascii_digit())
                .then(|| parse_numeric_text(s, locale))
            {
                Some(Some(n)) => number(n),
                _ => Same::Text(s.to_lowercase()),
            },
            Value::Bool(b) => Same::Bool(*b),
            Value::Error(e) => Same::Error(*e),
            Value::Empty | Value::Array(_) => return,
        };
        let next = counts.len();
        let id = *ids.entry(same).or_insert(next);
        if id == next {
            counts.push(0);
        }
        counts[id] += 1;
        cells.push(((row, col), id));
    });
    cells
        .into_iter()
        .filter(|(_, id)| (counts[*id] > 1) == twice)
        .map(|(cell, _)| cell)
        .collect()
}

/// The numbers of a rule's ranges, for its thresholds.
struct Stats {
    numbers: Vec<f64>,
    sorted: bool,
    min: f64,
    max: f64,
}

impl Stats {
    /// `None` when the ranges hold no number: there is nothing to grade.
    fn of(sh: &Sheet, ranges: &[Rect]) -> Option<Stats> {
        let numbers = numbers(sh, ranges);
        let min = numbers.iter().copied().reduce(f64::min)?;
        let max = numbers.iter().copied().reduce(f64::max)?;
        Some(Stats {
            numbers,
            sorted: false,
            min,
            max,
        })
    }

    /// As `PERCENTILE.INC`, with `p` from 0 to 1.
    fn percentile(&mut self, p: f64) -> f64 {
        if !self.sorted {
            self.numbers.sort_unstable_by(f64::total_cmp);
            self.sorted = true;
        }
        let at = p.clamp(0.0, 1.0) * (self.numbers.len() - 1) as f64;
        let below = self.numbers[at.floor() as usize];
        let above = self.numbers[at.ceil() as usize];
        below + (above - below) * at.fract()
    }
}

/// The number a threshold stands for; `None` when its formula does not
/// give a number. `low_end` is true for the threshold at the low end of a
/// scale or a bar.
fn threshold(
    cx: &Cx,
    anchor: (u32, u32),
    cfvo: &Cfvo,
    stats: &mut Stats,
    low_end: bool,
) -> Option<f64> {
    let number = |e: &Expr| match formula::eval_at(cx.wb, cx.sheet, anchor, e) {
        Value::Number(n) => Some(n),
        Value::Bool(b) => Some(b as u8 as f64),
        Value::Empty => Some(0.0),
        Value::Text(s) => parse_numeric_text(&s, cx.wb.locale),
        Value::Error(_) | Value::Array(_) => None,
    };
    Some(match cfvo {
        Cfvo::Min => stats.min,
        Cfvo::Max => stats.max,
        Cfvo::Automatic if low_end => stats.min,
        Cfvo::Automatic => stats.max,
        Cfvo::Number(e) | Cfvo::Formula(e) => number(e)?,
        Cfvo::Percent(e) => stats.min + (stats.max - stats.min) * number(e)? / 100.0,
        Cfvo::Percentile(e) => stats.percentile(number(e)? / 100.0),
    })
}

fn scale_stops(cx: &Cx, cf: &CondFormat, scale: &ColorScale) -> Option<Vec<(f64, Color)>> {
    let mut stats = Stats::of(cx.sh, &cf.ranges)?;
    let last = scale.stops.len().checked_sub(1)?;
    scale
        .stops
        .iter()
        .enumerate()
        .map(|(i, stop)| {
            Some((
                threshold(cx, cf.anchor(), &stop.at, &mut stats, i < last)?,
                stop.color,
            ))
        })
        .collect()
}

fn bar_ends(cx: &Cx, cf: &CondFormat, bar: &DataBar) -> Option<(f64, f64)> {
    let mut stats = Stats::of(cx.sh, &cf.ranges)?;
    let mut low = threshold(cx, cf.anchor(), &bar.min, &mut stats, true)?;
    let mut high = threshold(cx, cf.anchor(), &bar.max, &mut stats, false)?;
    // Automatic ends always take in zero, so that bars are in proportion
    // to the numbers themselves.
    if bar.min == Cfvo::Automatic {
        low = low.min(0.0);
    }
    if bar.max == Cfvo::Automatic {
        high = high.max(0.0);
    }
    (low <= high).then_some((low, high))
}

fn icon_floors(cx: &Cx, cf: &CondFormat, icons: &IconRule) -> Option<Vec<f64>> {
    let mut stats = Stats::of(cx.sh, &cf.ranges)?;
    icons
        .thresholds
        .iter()
        .enumerate()
        .map(|(i, t)| match i {
            // The lowest band has no floor.
            0 => Some(f64::NEG_INFINITY),
            _ => threshold(cx, cf.anchor(), &t.value, &mut stats, false),
        })
        .collect()
}

// ---- one cell --------------------------------------------------------------------

/// What the rules seen so far do to the cell.
#[derive(Default)]
struct Acc {
    patch: StylePatch,
    bar: Option<Bar>,
    icon: Option<CellIcon>,
    /// An icon set had its say, even if it was "No Cell Icon".
    icon_set: bool,
    hide_value: bool,
}

impl Acc {
    /// Add the format of a rule of lower priority: it only sets what the
    /// rules before it left alone.
    fn lay_under(&mut self, format: &StylePatch) {
        if format.is_empty() {
            return;
        }
        let mut patch = format.clone();
        patch.merge(&self.patch);
        self.patch = patch;
    }

    fn finish(self) -> Option<CondResult> {
        let adornment =
            (self.bar.is_some() || self.icon.is_some() || self.hide_value).then_some(Adornment {
                bar: self.bar,
                icon: self.icon,
                hide_value: self.hide_value,
            });
        (!self.patch.is_empty() || adornment.is_some()).then_some(CondResult {
            patch: self.patch,
            adornment,
        })
    }
}

/// Apply one rule to a cell holding `v`. True when the rule takes effect
/// there (which is what "Stop If True" looks at).
fn apply(
    cx: &Cx,
    cf: &CondFormat,
    data: &mut RuleData,
    row: u32,
    col: u32,
    v: &Value,
    acc: &mut Acc,
) -> bool {
    let locale = cx.wb.locale;
    let number = match v {
        Value::Number(n) => Some(*n),
        _ => None,
    };
    let matched = match (&cf.rule, data) {
        (Rule::ColorScale(_), RuleData::Scale(stops)) => {
            let Some(color) = number
                .zip(stops.as_deref())
                .and_then(|(n, stops)| scale_color(stops, n))
            else {
                return false;
            };
            if acc.patch.fill.is_none() {
                acc.patch.fill = Some(Some(color));
            }
            return true;
        }
        (Rule::DataBar(bar), RuleData::Bar(ends)) => {
            let Some((n, ends)) = number.zip(*ends) else {
                return false;
            };
            if acc.bar.is_none() {
                acc.bar = Some(bar_at(bar, ends, n));
                acc.hide_value |= !bar.show_value;
            }
            return true;
        }
        (Rule::IconSet(icons), RuleData::Icons(floors)) => {
            let Some((n, floors)) = number.zip(floors.as_deref()) else {
                return false;
            };
            if !acc.icon_set {
                acc.icon_set = true;
                acc.icon = icon_at(icons, floors, n);
                acc.hide_value |= !icons.show_value;
            }
            return true;
        }
        (
            Rule::CellIs { op, .. },
            RuleData::CellIs {
                first: Operand::Fixed(a),
                second: None,
                ..
            },
        ) => cell_is(*op, v, a, None),
        (
            Rule::CellIs { op, .. },
            RuleData::CellIs {
                first: Operand::Fixed(a),
                second: Some(Operand::Fixed(b)),
                ..
            },
        ) => cell_is(*op, v, a, Some(b)),
        (
            Rule::CellIs { op, first, second },
            RuleData::CellIs {
                first: a,
                second: b,
                cells,
            },
        ) => {
            cells.get(row, col, &cf.ranges, &mut |block| {
                // Only the operands that change from cell to cell are
                // evaluated, in the order they are read back below.
                let mut exprs = Vec::new();
                let mut opaque = false;
                for (expr, operand) in [(Some(first), Some(&*a)), (second.as_ref(), b.as_ref())] {
                    if let (Some(expr), Some(Operand::PerCell { opaque: o })) = (expr, operand) {
                        exprs.push(expr);
                        opaque |= o;
                    }
                }
                let mut values =
                    formula::eval_cells(cx.wb, cx.sheet, cf.anchor(), &exprs, opaque, block)
                        .into_iter();
                let mut value_of = |operand: &Operand| match operand {
                    Operand::Fixed(value) => value.clone(),
                    Operand::PerCell { .. } => values.next().unwrap_or_default(),
                };
                block
                    .iter()
                    .map(|&(r, c)| {
                        let x = value_of(a);
                        let y = b.as_ref().map(&mut value_of);
                        cell_is(*op, &cx.sh.value_at(r, c), &x, y.as_ref())
                    })
                    .collect()
            })
        }
        (
            Rule::Formula(_),
            RuleData::Formula {
                fixed: Some(answer),
                ..
            },
        ) => *answer,
        (Rule::Formula(e), RuleData::Formula { opaque, cells, .. }) => {
            cells.get(row, col, &cf.ranges, &mut |block| {
                formula::eval_cells(cx.wb, cx.sheet, cf.anchor(), &[e], *opaque, block)
                    .iter()
                    .map(is_true)
                    .collect()
            })
        }
        (Rule::Text { op, text: e }, RuleData::Text { text, cells }) => {
            cells.get(row, col, &cf.ranges, &mut |block| match text {
                Operand::Fixed(text) => block
                    .iter()
                    .map(|&(r, c)| text_is(*op, &cx.sh.value_at(r, c), text, locale))
                    .collect(),
                Operand::PerCell { opaque } => {
                    let texts =
                        formula::eval_cells(cx.wb, cx.sheet, cf.anchor(), &[e], *opaque, block);
                    block
                        .iter()
                        .zip(&texts)
                        .map(|(&(r, c), text)| text_is(*op, &cx.sh.value_at(r, c), text, locale))
                        .collect()
                }
            })
        }
        (Rule::TopBottom { bottom, .. }, RuleData::Rank(threshold)) => number
            .zip(*threshold)
            .is_some_and(|(n, t)| if *bottom { n <= t } else { n >= t }),
        (
            Rule::Average {
                above, or_equal, ..
            },
            RuleData::Average(threshold),
        ) => number
            .zip(*threshold)
            .is_some_and(|(n, t)| match n.total_cmp(&t) {
                Ordering::Greater => *above,
                Ordering::Less => !*above,
                Ordering::Equal => *or_equal,
            }),
        (Rule::DuplicateValues | Rule::UniqueValues, RuleData::Cells(cells)) => {
            cells.contains(&(row, col))
        }
        (Rule::Blanks, _) => is_blank(v) == Some(true),
        (Rule::NoBlanks, _) => is_blank(v) == Some(false),
        (Rule::Errors, _) => v.is_error(),
        (Rule::NoErrors, _) => !v.is_error(),
        (Rule::DateOccurring(period), _) => date_occurs(*period, v, cx.today, locale),
        // A rule is always prepared for its own kind.
        _ => false,
    };
    if matched {
        acc.lay_under(&cf.format);
    }
    matched
}

/// A formula rule holds when its formula gives TRUE or a number other than
/// zero. Text, blanks and errors do not.
fn is_true(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        _ => false,
    }
}

/// The comparison of a `cellIs` rule, with the ordering formulas use:
/// numbers before text before booleans, text without case, a blank cell as
/// zero or as empty text. An error on either side matches nothing.
fn cell_is(op: CellOp, v: &Value, a: &Value, b: Option<&Value>) -> bool {
    if v.is_error() || a.is_error() || b.is_some_and(Value::is_error) {
        return false;
    }
    let order = compare_values(v, a);
    match op {
        CellOp::Equal => order == Ordering::Equal,
        CellOp::NotEqual => order != Ordering::Equal,
        CellOp::Greater => order == Ordering::Greater,
        CellOp::Less => order == Ordering::Less,
        CellOp::GreaterOrEqual => order != Ordering::Less,
        CellOp::LessOrEqual => order != Ordering::Greater,
        CellOp::Between | CellOp::NotBetween => {
            let Some(b) = b else {
                return false;
            };
            // The two bounds are accepted in either order.
            let (low, high) = if compare_values(a, b) == Ordering::Greater {
                (b, a)
            } else {
                (a, b)
            };
            let inside = compare_values(v, low) != Ordering::Less
                && compare_values(v, high) != Ordering::Greater;
            inside == (op == CellOp::Between)
        }
    }
}

/// A value as the text functions see it; `None` for an error.
fn text_of<'a>(v: &'a Value, locale: &Locale) -> Option<Cow<'a, str>> {
    match v {
        Value::Empty => Some(Cow::Borrowed("")),
        Value::Text(s) => Some(Cow::Borrowed(s)),
        Value::Number(n) => Some(Cow::Owned(locale.number_to_text(*n))),
        Value::Bool(b) => Some(Cow::Borrowed(locale.bool_name(*b))),
        Value::Error(_) | Value::Array(_) => None,
    }
}

/// The test of a text rule, as the formula Excel stores for it decides:
/// * contains: `NOT(ISERROR(SEARCH(text, cell)))` — without case, with the
///   wildcards `?`, `*` and `~`; numbers are searched as text;
/// * does not contain: `ISERROR(SEARCH(text, cell))` — so error cells match;
/// * begins / ends with: `LEFT(cell, LEN(text)) = text` — without case, no
///   wildcards, and never true for a number given as the text.
fn text_is(op: TextOp, v: &Value, text: &Value, locale: &Locale) -> bool {
    match op {
        TextOp::Contains | TextOp::NotContains => {
            let found =
                text_of(text, locale)
                    .zip(text_of(v, locale))
                    .is_some_and(|(needle, hay)| {
                        // A `~` left dangling at the end stands for itself.
                        let dangling =
                            needle.chars().rev().take_while(|c| *c == '~').count() % 2 == 1;
                        wildcard_match(
                            &format!("*{needle}{}*", if dangling { "~" } else { "" }),
                            &hay,
                        )
                    });
            found == (op == TextOp::Contains)
        }
        TextOp::BeginsWith | TextOp::EndsWith => {
            let needle = match text {
                Value::Text(s) => &**s,
                Value::Empty => "",
                _ => return false,
            };
            let Some(hay) = text_of(v, locale) else {
                return false;
            };
            // As many characters of the cell as the text has, from its
            // start or from its end.
            let wanted = needle.chars().count();
            let cut = match op {
                TextOp::BeginsWith => {
                    let end = hay.char_indices().nth(wanted).map_or(hay.len(), |(i, _)| i);
                    &hay[..end]
                }
                _ => {
                    let skipped = hay.chars().count().saturating_sub(wanted);
                    let start = hay
                        .char_indices()
                        .nth(skipped)
                        .map_or(hay.len(), |(i, _)| i);
                    &hay[start..]
                }
            };
            compare_text(cut, needle) == Ordering::Equal
        }
    }
}

/// `LEN(TRIM(cell))=0`: nothing, or nothing but spaces. `None` for an
/// error, which is neither blank nor "not blank".
fn is_blank(v: &Value) -> Option<bool> {
    match v {
        Value::Empty => Some(true),
        Value::Text(s) => Some(s.chars().all(|c| c == ' ')),
        Value::Number(_) | Value::Bool(_) => Some(false),
        Value::Error(_) | Value::Array(_) => None,
    }
}

/// Whether the cell holds a date in the period, as the formulas Excel
/// stores for these rules decide (see `ooxml_formulas`). Dates are numbers;
/// text that reads as a date counts too. Weeks start on Sunday.
fn date_occurs(period: TimePeriod, v: &Value, today: f64, locale: &Locale) -> bool {
    let serial = match v {
        Value::Number(n) => *n,
        Value::Text(s) => match parse_numeric_text(s, locale) {
            Some(n) => n,
            None => return false,
        },
        _ => return false,
    };
    if serial < 0.0 {
        return false;
    }
    let day = serial.floor();
    // 1 = Sunday ... 7 = Saturday, as WEEKDAY.
    let weekday = datetime::weekday(today) as f64 + 1.0;
    let same_month = |months: i64| {
        let (year, month, _) = datetime::serial_to_ymd(today);
        let (y, m, _) = datetime::serial_to_ymd(day);
        let wanted = year * 12 + month as i64 - 1 + months;
        y * 12 + m as i64 - 1 == wanted
    };
    match period {
        TimePeriod::Today => day == today,
        TimePeriod::Yesterday => day == today - 1.0,
        TimePeriod::Tomorrow => day == today + 1.0,
        TimePeriod::Last7Days => (today - 6.0..=today).contains(&day),
        TimePeriod::ThisWeek => (today - (weekday - 1.0)..=today + (7.0 - weekday)).contains(&day),
        TimePeriod::LastWeek => (today - weekday - 6.0..=today - weekday).contains(&day),
        TimePeriod::NextWeek => (today + 8.0 - weekday..=today + 14.0 - weekday).contains(&day),
        TimePeriod::ThisMonth => same_month(0),
        TimePeriod::LastMonth => same_month(-1),
        TimePeriod::NextMonth => same_month(1),
    }
}

/// The colour of a number on a scale: the colour of the stop it sits on,
/// or a blend of the two stops around it. Numbers beyond the ends take the
/// end colours.
fn scale_color(stops: &[(f64, Color)], n: f64) -> Option<Color> {
    let (first, rest) = stops.split_first()?;
    if n <= first.0 {
        return Some(first.1);
    }
    let mut below = *first;
    for &above in rest {
        if n <= above.0 {
            let t = (n - below.0) / (above.0 - below.0);
            let blend = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
            let (a, b) = (below.1, above.1);
            return Some(Color::rgb(
                blend(a.r(), b.r()),
                blend(a.g(), b.g()),
                blend(a.b(), b.b()),
            ));
        }
        below = above;
    }
    Some(below.1)
}

/// How far `n` is on the way from `zero` to `full` (`zero <= full`), from
/// 0 to 1.
fn share(n: f64, zero: f64, full: f64) -> f64 {
    if n <= zero {
        0.0
    } else if n >= full {
        1.0
    } else {
        (n - zero) / (full - zero)
    }
}

/// The bar of the number `n` on a rule whose shortest bar stands for `low`
/// and whose longest stands for `high`.
fn bar_at(bar: &DataBar, (low, high): (f64, f64), n: f64) -> Bar {
    let shortest = bar.min_length.min(100) as f64 / 100.0;
    let longest = bar.max_length.min(100) as f64 / 100.0;
    // All bars from the cell's edge, as long as `n` is far from `low`.
    let from_edge = || {
        (
            0.0,
            shortest + share(n, low, high) * (longest - shortest),
            None,
        )
    };
    let (mut start, mut end, mut axis) = match bar.axis {
        BarAxis::None => from_edge(),
        BarAxis::Automatic if low >= 0.0 => from_edge(),
        BarAxis::Automatic => {
            // Zero falls between the two ends in proportion; with nothing
            // positive it is the far edge and every bar grows back from it.
            let high = high.max(0.0);
            let zero = -low / (high - low);
            let length = if n < 0.0 {
                -share(-n, 0.0, -low) * zero
            } else {
                share(n, 0.0, high) * (1.0 - zero)
            };
            (zero, zero + length * longest, Some(zero))
        }
        BarAxis::Midpoint => {
            let reach = low.abs().max(high.abs());
            let length = if reach > 0.0 {
                n.clamp(low, high) / reach * 0.5
            } else {
                0.0
            };
            (0.5, 0.5 + length * longest, Some(0.5))
        }
    };
    if bar.direction == BarDirection::RightToLeft {
        start = 1.0 - start;
        end = 1.0 - end;
        axis = axis.map(|a| 1.0 - a);
    }
    let negative = n < 0.0;
    Bar {
        start: start as f32,
        end: end as f32,
        fill: if negative {
            bar.negative_fill.unwrap_or(bar.fill)
        } else {
            bar.fill
        },
        gradient: bar.gradient,
        border: if negative {
            bar.border.and(bar.negative_border.or(bar.border))
        } else {
            bar.border
        },
        negative,
        axis: axis.map(|a| a as f32),
        axis_color: bar.axis_color,
    }
}

/// The icon of the number `n`: the highest band whose floor it reaches.
/// `None` for a band given "No Cell Icon".
fn icon_at(icons: &IconRule, floors: &[f64], n: f64) -> Option<CellIcon> {
    let bands = floors.len().min(icons.set.len());
    let reached = |i: &usize| n > floors[*i] || (n == floors[*i] && icons.thresholds[*i].gte);
    let mut band = (1..bands).rev().find(reached).unwrap_or(0);
    if icons.reverse {
        band = icons.set.len() - 1 - band;
    }
    match icons.custom.as_ref().and_then(|custom| custom.get(band)) {
        Some(Some((set, index))) => ((*index as usize) < set.len()).then_some(CellIcon {
            set: *set,
            index: *index,
        }),
        Some(None) => None,
        None => Some(CellIcon {
            set: icons.set,
            index: band as u8,
        }),
    }
}
