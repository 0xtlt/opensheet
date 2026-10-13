//! Managing rules: the `Workbook` functions behind the ribbon and the Rules
//! Manager, carrying rules along with copied or moved cells, and the hooks
//! the core calls when rows, columns or sheets change.
//!
//! Like the other editing operations, the functions that change rules take
//! the undo snapshot themselves and leave `commit` to the caller.
//!
//! Whenever the ranges of a rule change, its formulas are rewritten for the
//! new anchor so that every cell that stays in the rule keeps reading the
//! same cells as before.

use super::formula::shifted;
use super::model::CondFormat;
use crate::ast::{Expr, RefExpr, RefKind, StructuralEdit};
use crate::features::{CellShift, TIDY_LIMIT, join_sorted, without_bands};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::workbook::{Sheet, Workbook};

// ---- ranges ----------------------------------------------------------------------

/// `rect` without the cells of `hole`: up to four pieces, in reading order.
fn subtract(rect: Rect, hole: &Rect, out: &mut Vec<Rect>) {
    let Some(cut) = rect.intersection(hole) else {
        out.push(rect);
        return;
    };
    if cut.r0 > rect.r0 {
        out.push(Rect::new(rect.r0, rect.c0, cut.r0 - 1, rect.c1));
    }
    if cut.c0 > rect.c0 {
        out.push(Rect::new(cut.r0, rect.c0, cut.r1, cut.c0 - 1));
    }
    if cut.c1 < rect.c1 {
        out.push(Rect::new(cut.r0, cut.c1 + 1, cut.r1, rect.c1));
    }
    if cut.r1 < rect.r1 {
        out.push(Rect::new(cut.r1 + 1, rect.c0, rect.r1, rect.c1));
    }
}

/// The ranges without the cells of any of `holes`.
fn without(ranges: &[Rect], holes: &[Rect]) -> Vec<Rect> {
    let mut pieces = ranges.to_vec();
    for hole in holes {
        if pieces.iter().any(|p| p.intersects(hole)) {
            let mut next = Vec::with_capacity(pieces.len() + 3);
            for piece in pieces {
                subtract(piece, hole, &mut next);
            }
            pieces = next;
        }
    }
    pieces
}

/// The same cells as rectangles that do not overlap.
pub(super) fn disjoint(ranges: &[Rect]) -> Vec<Rect> {
    let mut out = Vec::with_capacity(ranges.len());
    for (i, r) in ranges.iter().enumerate() {
        out.extend(without(&[*r], &ranges[..i]));
    }
    out
}

/// One rectangle covering exactly the cells of both, when there is one.
fn joined(a: &Rect, b: &Rect) -> Option<Rect> {
    let touch = |a0: u32, a1: u32, b0: u32, b1: u32| a0 <= b1.saturating_add(1) && b0 <= a1.saturating_add(1);
    let inside = |a: &Rect, b: &Rect| a.r0 >= b.r0 && a.r1 <= b.r1 && a.c0 >= b.c0 && a.c1 <= b.c1;
    let stacked = a.c0 == b.c0 && a.c1 == b.c1 && touch(a.r0, a.r1, b.r0, b.r1);
    let side_by_side = a.r0 == b.r0 && a.r1 == b.r1 && touch(a.c0, a.c1, b.c0, b.c1);
    (stacked || side_by_side || inside(a, b) || inside(b, a)).then(|| a.union(b))
}

/// Tidy a list of ranges without changing the cells it covers: ranges
/// lying inside another are dropped and ranges that together make a
/// rectangle are joined. Keeps rules from crumbling into ever more ranges
/// as cells are pasted and cleared.
///
/// A long list (a rule left on the rows a filter shows, one block of rows
/// after the other) is joined by sorting instead, and comes out in reading
/// order: comparing every pair, over and over, would take for ever.
pub fn normalize_ranges(mut ranges: Vec<Rect>) -> Vec<Rect> {
    if ranges.len() > TIDY_LIMIT {
        ranges = join_sorted(ranges);
        if ranges.len() > TIDY_LIMIT {
            return ranges;
        }
    }
    let mut i = 0;
    while i < ranges.len() {
        let mut grew = false;
        let mut j = i + 1;
        while j < ranges.len() {
            match joined(&ranges[i], &ranges[j]) {
                Some(both) => {
                    ranges[i] = both;
                    ranges.remove(j);
                    grew = true;
                }
                None => j += 1,
            }
        }
        // A range that grew may now join ranges it was compared with before.
        i = if grew { 0 } else { i + 1 };
    }
    ranges
}

impl CondFormat {
    /// Give the rule new ranges (the "Applies to" box). Its formulas are
    /// written for the top-left cell of the first range: they move with
    /// it, so that each cell of the rule goes on reading the same cells,
    /// and a rule comparing each cell with its left neighbour still does
    /// so on the new ranges.
    pub fn set_ranges(&mut self, ranges: Vec<Rect>) {
        let old = self.anchor();
        self.ranges = ranges;
        let new = self.anchor();
        if new != old && !self.ranges.is_empty() {
            let (d_row, d_col) = (new.0 as i64 - old.0 as i64, new.1 as i64 - old.1 as i64);
            self.rule.map_exprs(&mut |e| shifted(e, d_row, d_col));
        }
    }
}

/// The copies of `parts` (pieces of `src`) laid over `dest`, which holds a
/// whole number of copies of `src` (or one copy cut short by the edge of
/// the sheet). Pieces as tall or as wide as `src` come out as one range
/// across the copies instead of one per copy.
fn tiled(parts: &[Rect], src: &Rect, dest: &Rect) -> Vec<Rect> {
    // (start, length) of a piece in each copy along one axis.
    let spans = |p0: u32, p1: u32, s0: u32, s1: u32, d0: u32, d1: u32| -> Vec<(u32, u32)> {
        let size = s1 - s0 + 1;
        let copies = ((d1 - d0 + 1) / size).max(1);
        let list: Vec<(u32, u32)> = if p0 == s0 && p1 == s1 {
            vec![(d0, d1 - d0 + 1)]
        } else {
            (0..copies).map(|i| (d0 + i * size + (p0 - s0), p1 - p0 + 1)).collect()
        };
        list.into_iter()
            .filter(|(at, _)| *at <= d1)
            .map(|(at, len)| (at, len.min(d1 - at + 1)))
            .collect()
    };
    let mut out = Vec::new();
    for p in parts {
        let rows = spans(p.r0, p.r1, src.r0, src.r1, dest.r0, dest.r1);
        let cols = spans(p.c0, p.c1, src.c0, src.c1, dest.c0, dest.c1);
        for &(r, height) in &rows {
            for &(c, width) in &cols {
                out.push(Rect::new(r, c, r + height - 1, c + width - 1));
            }
        }
    }
    out
}

/// The cells written when `src` is pasted on `dest`: `dest` itself when it
/// holds a whole number of copies, else one copy from its top-left corner.
fn pasted_area(src: &Rect, dest: &Rect) -> Rect {
    if dest.rows() % src.rows() == 0 && dest.cols() % src.cols() == 0 {
        return *dest;
    }
    Rect::new(
        dest.r0,
        dest.c0,
        (dest.r0 + src.rows() - 1).min(MAX_ROWS - 1),
        (dest.c0 + src.cols() - 1).min(MAX_COLS - 1),
    )
}

// ---- the rules of a sheet ----------------------------------------------------------

impl Workbook {
    /// The rules of a sheet in priority order (what the Rules Manager lists
    /// for "This Worksheet"); empty for a sheet that does not exist.
    pub fn cond_formats(&self, sheet: usize) -> &[CondFormat] {
        self.sheets
            .get(sheet)
            .map(|sh| sh.cond_formats.as_slice())
            .unwrap_or_default()
    }

    /// The rules that apply to at least one cell of `selection`, as indexes
    /// into the sheet's rules, in priority order (what the Rules Manager
    /// lists for "Current Selection").
    pub fn cond_formats_in(&self, sheet: usize, selection: &[Rect]) -> Vec<usize> {
        let rules = self.cond_formats(sheet);
        (0..rules.len())
            .filter(|i| selection.iter().any(|rect| rules[*i].intersects(rect)))
            .collect()
    }

    /// Change the rules of a sheet in one undoable step.
    fn edit_cond_formats(&mut self, sheet: usize, edit: impl FnOnce(&mut Vec<CondFormat>)) {
        self.snapshot();
        edit(&mut self.sheets[sheet].cond_formats);
        self.caches.cond.invalidate();
    }

    /// Add a rule with the highest priority, as the ribbon and the "New
    /// Rule" button do. False when the sheet does not exist or the rule
    /// has no range.
    pub fn add_cond_format(&mut self, sheet: usize, cf: CondFormat) -> bool {
        self.insert_cond_format(sheet, 0, cf)
    }

    /// Add a rule at a given place in the priority order.
    pub fn insert_cond_format(&mut self, sheet: usize, index: usize, cf: CondFormat) -> bool {
        if cf.ranges.is_empty() || index > self.cond_formats(sheet).len() || sheet >= self.sheets.len() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| rules.insert(index, cf));
        true
    }

    /// Edit a rule in place: its test, its format, "Stop If True". To move
    /// a rule to other cells use `CondFormat::set_ranges` rather than
    /// assigning `ranges`: it keeps the formulas pointing at the same cells.
    pub fn update_cond_format(&mut self, sheet: usize, index: usize, f: impl FnOnce(&mut CondFormat)) -> bool {
        if index >= self.cond_formats(sheet).len() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| f(&mut rules[index]));
        true
    }

    /// Replace all the rules of a sheet (the Rules Manager's OK button,
    /// after editing a copy of the list).
    pub fn set_cond_formats(&mut self, sheet: usize, rules: Vec<CondFormat>) -> bool {
        if sheet >= self.sheets.len() || self.sheets[sheet].cond_formats == rules {
            return false;
        }
        self.edit_cond_formats(sheet, |old| *old = rules);
        true
    }

    /// Delete a rule.
    pub fn remove_cond_format(&mut self, sheet: usize, index: usize) -> bool {
        if index >= self.cond_formats(sheet).len() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| {
            rules.remove(index);
        });
        true
    }

    /// Change the priority of a rule ("Move Up" / "Move Down"): it ends up
    /// at index `to`.
    pub fn reorder_cond_format(&mut self, sheet: usize, from: usize, to: usize) -> bool {
        let count = self.cond_formats(sheet).len();
        if from >= count || to >= count || from == to {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| {
            let cf = rules.remove(from);
            rules.insert(to, cf);
        });
        true
    }

    /// Change the cells a rule applies to, as `CondFormat::set_ranges`.
    pub fn set_cond_format_ranges(&mut self, sheet: usize, index: usize, ranges: Vec<Rect>) -> bool {
        if ranges.is_empty() || index >= self.cond_formats(sheet).len() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| rules[index].set_ranges(ranges));
        true
    }

    /// "Clear Rules from Selected Cells": the cells of `selection` leave
    /// every rule. Ranges are split around them and rules left without a
    /// cell are deleted. False when no rule touched the selection.
    pub fn clear_cond_formats(&mut self, sheet: usize, selection: &[Rect]) -> bool {
        if self.cond_formats_in(sheet, selection).is_empty() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| remove_cells(rules, selection));
        true
    }

    /// "Clear Rules from Entire Sheet".
    pub fn clear_all_cond_formats(&mut self, sheet: usize) -> bool {
        if self.cond_formats(sheet).is_empty() {
            return false;
        }
        self.edit_cond_formats(sheet, Vec::clear);
        true
    }

    /// Carry conditional formats along with copied cells (`paste`, the
    /// fills and the format painter call this). `src` is the copied block
    /// and `dest` the block written (`dest` may hold several copies of
    /// `src`, as when one cell is pasted over a selection). The
    /// destination loses the rules it had and takes those of the source
    /// cells, whose formulas read from each pasted cell as they did from
    /// the cell it was copied from. On the same sheet the rules simply
    /// gain the pasted cells; on another sheet they are added as new rules
    /// of the highest priority, in their own order. False when no rule
    /// touched either block.
    pub fn copy_cond_formats(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest: Rect) -> bool {
        self.carry_cond_formats(src_sheet, src, dest_sheet, dest, false)
    }

    /// Carry conditional formats along with moved cells (`move_range`
    /// calls this): as `copy_cond_formats` to the block of the same
    /// size at (`dest_row`, `dest_col`), after which the cells left behind
    /// have no rule. Absolute references of any rule to the moved cells
    /// follow them when the move stays on one sheet.
    pub fn move_cond_formats(
        &mut self,
        src_sheet: usize,
        src: Rect,
        dest_sheet: usize,
        dest_row: u32,
        dest_col: u32,
    ) -> bool {
        let dest = Rect::new(
            dest_row,
            dest_col,
            (dest_row + src.rows() - 1).min(MAX_ROWS - 1),
            (dest_col + src.cols() - 1).min(MAX_COLS - 1),
        );
        if src_sheet == dest_sheet && dest == src {
            return false;
        }
        self.carry_cond_formats(src_sheet, src, dest_sheet, dest, true)
    }

    /// Carry conditional formats along with cells pasted with their rows
    /// and columns exchanged (Paste Special ▸ Transpose): `out` is where
    /// one turned copy of `src` lands. The cells written lose the rules
    /// they had; each rule of the copied cells is given again for the
    /// turned cells, as a rule of its own of the highest priority, its
    /// relative references turned with them.
    pub(crate) fn transpose_cond_formats(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, out: Rect) -> bool {
        use crate::paste_special::{transposed, turned};
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return false;
        }
        let mut copies = Vec::new();
        for cf in self.cond_formats(src_sheet) {
            let pieces: Vec<Rect> = cf.ranges.iter().filter_map(|r| r.intersection(&src)).collect();
            let ranges: Vec<Rect> = pieces.iter().filter_map(|piece| turned(piece, &src, &out)).collect();
            let (Some(first), false) = (pieces.first(), ranges.is_empty()) else {
                continue;
            };
            // The formulas are read from the first copied cell, turned
            // for the cell it becomes, then follow the ranges given.
            let mut copy = cf.clone();
            copy.set_ranges(vec![Rect::cell(first.r0, first.c0)]);
            let to = (out.r0 + (first.c0 - src.c0), out.c0 + (first.r0 - src.r0));
            copy.rule.map_exprs(&mut |e| transposed(e, (first.r0, first.c0), to));
            copy.ranges = vec![Rect::cell(to.0, to.1)];
            copy.set_ranges(normalize_ranges(ranges));
            copies.push(copy);
        }
        if copies.is_empty() && !self.cond_formats(dest_sheet).iter().any(|cf| cf.intersects(&out)) {
            return false;
        }
        self.snapshot();
        let rules = &mut self.sheets[dest_sheet].cond_formats;
        remove_cells(rules, &[out]);
        rules.splice(0..0, copies);
        self.caches.cond.invalidate();
        true
    }

    fn carry_cond_formats(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest: Rect, moved: bool) -> bool {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return false;
        }
        let area = pasted_area(&src, &dest);
        let from_source = self.cond_formats(src_sheet).iter().any(|cf| cf.intersects(&src));
        if !from_source && !self.cond_formats(dest_sheet).iter().any(|cf| cf.intersects(&area)) {
            return false;
        }
        self.snapshot();
        let mut source = std::mem::take(&mut self.sheets[src_sheet].cond_formats);
        let pieces = |cf: &CondFormat| -> Vec<Rect> { cf.ranges.iter().filter_map(|r| r.intersection(&src)).collect() };
        if src_sheet == dest_sheet {
            // Each rule loses the cells written over and gains the copies
            // of its own cells.
            let holes = if moved { vec![area, src] } else { vec![area] };
            let mut rules = Vec::with_capacity(source.len());
            for mut cf in source {
                let copied = pieces(&cf);
                if copied.is_empty() && !cf.intersects(&area) {
                    rules.push(cf);
                    continue;
                }
                let mut ranges = without(&cf.ranges, &holes);
                ranges.extend(tiled(&copied, &src, &area));
                if !ranges.is_empty() {
                    cf.set_ranges(normalize_ranges(ranges));
                    rules.push(cf);
                }
            }
            self.sheets[src_sheet].cond_formats = rules;
        } else {
            let mut copies = Vec::new();
            for cf in &source {
                let ranges = tiled(&pieces(cf), &src, &area);
                if !ranges.is_empty() {
                    let mut copy = cf.clone();
                    copy.set_ranges(normalize_ranges(ranges));
                    copies.push(copy);
                }
            }
            if moved {
                remove_cells(&mut source, &[src]);
            }
            self.sheets[src_sheet].cond_formats = source;
            let rules = &mut self.sheets[dest_sheet].cond_formats;
            remove_cells(rules, &[area]);
            rules.splice(0..0, copies);
        }
        if moved && src_sheet == dest_sheet {
            let (d_row, d_col) = (dest.r0 as i64 - src.r0 as i64, dest.c0 as i64 - src.c0 as i64);
            let moved_sheet = self.sheets[src_sheet].name.clone();
            for sh in &mut self.sheets {
                let own = sh.name.clone();
                for cf in &mut sh.cond_formats {
                    cf.rule.map_exprs(&mut |e| {
                        // Relative references belong to whichever cell
                        // reads the rule: only fixed ones name the cells
                        // that moved.
                        e.map_refs(&mut |r| match is_absolute(r) {
                            true => Expr::Ref(r.clone()).with_moved_range(&moved_sheet, &own, src, d_row, d_col),
                            false => Expr::Ref(r.clone()),
                        })
                    });
                }
            }
        }
        self.caches.cond.invalidate();
        true
    }
}

/// Take the cells of `bands` (see `without_bands`) out of every rule;
/// rules left without a cell go.
fn remove_bands(rules: &mut Vec<CondFormat>, bands: &[Rect]) {
    rules.retain_mut(|cf| {
        let ranges = without_bands(&cf.ranges, bands);
        if ranges.is_empty() {
            return false;
        }
        if ranges != cf.ranges {
            cf.set_ranges(normalize_ranges(ranges));
        }
        true
    });
}

impl Workbook {
    /// As `clear_cond_formats`, for the blocks of rows a filter leaves
    /// shown in a selection (`Sheet::shown_parts`): one pass, however
    /// many blocks there are.
    pub(crate) fn clear_cond_formats_in_bands(&mut self, sheet: usize, bands: &[Rect]) -> bool {
        if self.cond_formats_in(sheet, bands).is_empty() {
            return false;
        }
        self.edit_cond_formats(sheet, |rules| remove_bands(rules, bands));
        true
    }

    /// As `copy_cond_formats`, laying `src` over each of `bands` (see
    /// `without_bands`; each holds a whole number of copies of `src`):
    /// what filling or pasting one row over a filtered list does, where
    /// the rows the filter hides keep their rules. One pass, so that the
    /// rules do not crumble a little more with every block.
    pub(crate) fn copy_cond_formats_to_bands(
        &mut self,
        src_sheet: usize,
        src: Rect,
        dest_sheet: usize,
        bands: &[Rect],
    ) -> bool {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return false;
        }
        let from_source = self.cond_formats(src_sheet).iter().any(|cf| cf.intersects(&src));
        if !from_source && self.cond_formats_in(dest_sheet, bands).is_empty() {
            return false;
        }
        self.snapshot();
        let copies = |cf: &CondFormat| -> Vec<Rect> {
            let pieces: Vec<Rect> = cf.ranges.iter().filter_map(|r| r.intersection(&src)).collect();
            bands.iter().flat_map(|band| tiled(&pieces, &src, band)).collect()
        };
        if src_sheet == dest_sheet {
            let rules = std::mem::take(&mut self.sheets[src_sheet].cond_formats);
            self.sheets[src_sheet].cond_formats = rules
                .into_iter()
                .filter_map(|mut cf| {
                    let mut ranges = without_bands(&cf.ranges, bands);
                    ranges.extend(copies(&cf));
                    if ranges != cf.ranges {
                        cf.set_ranges(normalize_ranges(ranges));
                    }
                    (!cf.ranges.is_empty()).then_some(cf)
                })
                .collect();
        } else {
            let mut added = Vec::new();
            for cf in &self.sheets[src_sheet].cond_formats {
                let ranges = copies(cf);
                if !ranges.is_empty() {
                    let mut copy = cf.clone();
                    copy.set_ranges(normalize_ranges(ranges));
                    added.push(copy);
                }
            }
            let rules = &mut self.sheets[dest_sheet].cond_formats;
            remove_bands(rules, bands);
            rules.splice(0..0, added);
        }
        self.caches.cond.invalidate();
        true
    }
}

fn is_absolute(r: &RefExpr) -> bool {
    let end = r.end.unwrap_or(r.start);
    r.kind == RefKind::Cells && r.start.abs_row && r.start.abs_col && end.abs_row && end.abs_col
}

/// Take the cells of `holes` out of every rule; rules left without a cell
/// go.
fn remove_cells(rules: &mut Vec<CondFormat>, holes: &[Rect]) {
    rules.retain_mut(|cf| {
        if holes.iter().any(|hole| cf.intersects(hole)) {
            let ranges = without(&cf.ranges, holes);
            if ranges.is_empty() {
                return false;
            }
            cf.set_ranges(normalize_ranges(ranges));
        }
        true
    });
}

// ---- hooks -------------------------------------------------------------------------

/// Where the lines `lo..=hi` end up after an insertion or a deletion along
/// their axis; `None` when all of them were deleted.
fn adjust_span(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32)> {
    // Whole rows stay whole rows whatever happens to the columns, and the
    // other way round.
    if lo == 0 && hi == max - 1 {
        return Some((lo, hi));
    }
    if insert {
        // Lines inserted right after the last one take its formatting, so
        // the range grows over them.
        let new_lo = if lo >= at { lo + count } else { lo };
        let new_hi = if hi + 1 >= at { hi + count } else { hi };
        (new_lo < max).then(|| (new_lo, new_hi.min(max - 1)))
    } else {
        let end = at + count - 1;
        if lo >= at && hi <= end {
            return None;
        }
        let new_lo = if lo > end { lo - count } else { lo.min(at) };
        let new_hi = if hi > end { hi - count } else { hi.min(at - 1) };
        Some((new_lo, new_hi))
    }
}

/// Where a line that exists after the edit was before it. An inserted line
/// was nowhere: it gives the line that stood in its place.
fn line_before(line: u32, insert: bool, at: u32, count: u32) -> u32 {
    match (insert, line >= at) {
        (_, false) => line,
        (true, true) => line.saturating_sub(count).max(at),
        (false, true) => line + count,
    }
}

/// Where a line that survives the edit is after it.
fn line_after(line: u32, insert: bool, at: u32, count: u32) -> u32 {
    match (insert, line >= at) {
        (_, false) => line,
        (true, true) => line + count,
        (false, true) => line - count,
    }
}

/// See `features::on_structural_edit`: ranges on the edited sheet move,
/// grow or shrink (a rule whose cells are all deleted goes), and formulas
/// on every sheet follow the cells they refer to.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    let own = sheet.name.clone();
    let edited = own.eq_ignore_ascii_case(edited_sheet);
    let (rows, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    sheet.cond_formats.retain_mut(|cf| {
        if !edited {
            cf.rule.map_exprs(&mut |e| e.adjusted(edit, edited_sheet, &own));
            return true;
        }
        let ranges: Vec<Rect> = cf
            .ranges
            .iter()
            .filter_map(|r| {
                if rows {
                    let (r0, r1) = adjust_span(r.r0, r.r1, insert, at, count, MAX_ROWS)?;
                    Some(Rect::new(r0, r.c0, r1, r.c1))
                } else {
                    let (c0, c1) = adjust_span(r.c0, r.c1, insert, at, count, MAX_COLS)?;
                    Some(Rect::new(r.r0, c0, r.r1, c1))
                }
            })
            .collect();
        if ranges.is_empty() {
            return false;
        }
        // The formulas are adjusted like those of a cell, and that cell
        // must survive the edit: the one that ends up as the new anchor
        // (the old anchor may be deleted). Where the anchor does not move
        // with its cell (the first row of a whole column when rows are
        // inserted above it), the formulas are then brought back to it.
        let old = cf.anchor();
        let new = (ranges[0].r0, ranges[0].c0);
        let (before, after) = if rows {
            let line = line_before(new.0, insert, at, count);
            ((line, new.1), (line_after(line, insert, at, count), new.1))
        } else {
            let line = line_before(new.1, insert, at, count);
            ((new.0, line), (new.0, line_after(line, insert, at, count)))
        };
        let offset = |from: (u32, u32), to: (u32, u32)| (to.0 as i64 - from.0 as i64, to.1 as i64 - from.1 as i64);
        let (to_cell, to_anchor) = (offset(old, before), offset(after, new));
        cf.rule.map_exprs(&mut |e| {
            let adjusted = shifted(e, to_cell.0, to_cell.1).adjusted(edit, edited_sheet, &own);
            shifted(&adjusted, to_anchor.0, to_anchor.1)
        });
        cf.ranges = ranges;
        true
    });
}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for cf in &mut sheet.cond_formats {
        cf.rule.map_exprs(&mut |e| e.with_sheet_renamed(old, new));
    }
}

/// See `features::on_sheet_duplicated`: references that named the original
/// sheet now name the copy, like the references without a sheet name.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    let Some(sheet) = wb.sheets.get_mut(copy) else {
        return;
    };
    let name = sheet.name.clone();
    for cf in &mut sheet.cond_formats {
        cf.rule.map_exprs(&mut |e| e.with_sheet_renamed(original, &name));
    }
}

/// See `features::on_sheet_deleted`: formulas that read the deleted sheet
/// read `#REF!`.
pub(crate) fn on_sheet_deleted(wb: &mut Workbook, deleted: &Sheet, order: &[&str]) {
    for cf in wb.sheets.iter_mut().flat_map(|sh| &mut sh.cond_formats) {
        cf.rule.map_exprs(&mut |e| e.with_sheet_deleted(&deleted.name, order));
    }
    // The cache knows sheets by their place, which changed.
    wb.caches.cond.invalidate();
}

/// See `features::on_sheet_moved`: 3-D references follow their end sheets.
pub(crate) fn on_sheet_moved(wb: &mut Workbook, moved: &str, before: &[&str], after: &[&str]) {
    for cf in wb.sheets.iter_mut().flat_map(|sh| &mut sh.cond_formats) {
        cf.rule.map_exprs(&mut |e| e.with_sheet_moved(moved, before, after));
    }
    wb.caches.cond.invalidate();
}

/// See `features::on_cells_shifted`: the cells that moved keep their
/// rules, as if pasted where they now are, and deleted cells leave
/// theirs. Inserted cells join the rules of the cell before them, as rows
/// and columns inserted into the sheet do.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    if wb.sheets.iter().all(|sh| sh.cond_formats.is_empty()) {
        return;
    }
    if let Some((from, to)) = shift.carried() {
        wb.copy_cond_formats(sheet, from, sheet, to);
    }
    match shift.inserted() {
        Some((blank, Some(before))) => wb.copy_cond_formats(sheet, before, sheet, blank),
        Some((blank, None)) => wb.clear_cond_formats(sheet, &[blank]),
        None => false,
    };
    // Rules anywhere that name the moved cells go on reading them.
    let edited = wb.sheets[sheet].name.clone();
    for sh in &mut wb.sheets {
        let own = sh.name.clone();
        for cf in &mut sh.cond_formats {
            cf.rule.map_exprs(&mut |e| shift.fix_rule(e, &edited, &own));
        }
    }
    wb.caches.cond.invalidate();
}
