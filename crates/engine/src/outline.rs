//! Outlines (Data > Group / Ungroup): rows and columns carry a level, and
//! the rows or columns of a group fold away under their summary.
//!
//! A group is a run of neighbouring rows (columns) whose level is at least
//! some level `n`; its summary is the row just below it (or above, see
//! `Outline::summary_below`). Folding a group hides its rows through
//! `Sheet::hidden_rows`, so filters, SUBTOTAL, charts and printing see what
//! the screen shows. A group is folded when all its rows are hidden; the
//! `collapsed` mark of its summary only remembers that an inner group was
//! folded while an outer one hides it, as the file format does.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{Expr, RefKind, StructuralEdit, same_name};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::workbook::{Content, Sheet, Workbook};

/// The deepest level a row or a column can have: with the level of what
/// is not grouped, Excel's eight levels.
pub const MAX_LEVEL: u8 = 7;

/// Rows or columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Cols,
}

/// A run of rows (columns) grouped at `level` or deeper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub level: u8,
    pub first: u32,
    pub last: u32,
}

/// The outline along one axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisOutline {
    /// The level of the rows (columns) that are grouped; the others are
    /// at level 0.
    pub levels: BTreeMap<u32, u8>,
    /// Summary rows (columns) whose group was folded.
    pub collapsed: BTreeSet<u32>,
}

impl AxisOutline {
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    pub fn level(&self, index: u32) -> u8 {
        self.levels.get(&index).copied().unwrap_or(0)
    }

    /// The deepest level in use.
    pub fn max_level(&self) -> u8 {
        self.levels.values().copied().max().unwrap_or(0)
    }

    fn set_level(&mut self, index: u32, level: u8) {
        if level == 0 {
            self.levels.remove(&index);
        } else {
            self.levels.insert(index, level.min(MAX_LEVEL));
        }
    }

    /// The group of `level` holding `index`: the run around it whose
    /// level is `level` or deeper.
    pub fn group_at(&self, index: u32, level: u8) -> Option<Group> {
        if level == 0 || self.level(index) < level {
            return None;
        }
        let (mut first, mut last) = (index, index);
        while first > 0 && self.level(first - 1) >= level {
            first -= 1;
        }
        while self.level(last + 1) >= level {
            last += 1;
        }
        Some(Group { level, first, last })
    }

    /// Every group that has a row (column) in `lo..=hi` or whose summary
    /// may be there, outermost first.
    pub fn groups(&self, lo: u32, hi: u32) -> Vec<Group> {
        // Groups reach as far as grouped rows follow one another.
        let (mut start, mut end) = (lo.saturating_sub(1), hi.saturating_add(1));
        while start > 0 && self.levels.contains_key(&(start - 1)) {
            start -= 1;
        }
        while self.levels.contains_key(&(end + 1)) {
            end += 1;
        }
        let mut out = Vec::new();
        // The group being walked at each level.
        let mut open: [Option<u32>; MAX_LEVEL as usize + 1] = [None; MAX_LEVEL as usize + 1];
        let mut previous: Option<u32> = None;
        let close = |open: &mut [Option<u32>], from: u8, last: u32, out: &mut Vec<Group>| {
            for level in (from..=MAX_LEVEL).rev() {
                if let Some(first) = open[level as usize].take() {
                    out.push(Group { level, first, last });
                }
            }
        };
        for (&index, &level) in self.levels.range(start..=end) {
            if let Some(p) = previous {
                // A gap ends every group; a shallower row ends the deeper ones.
                let from = if index == p + 1 { level + 1 } else { 1 };
                close(&mut open, from, p, &mut out);
            }
            for l in 1..=level.min(MAX_LEVEL) {
                open[l as usize].get_or_insert(index);
            }
            previous = Some(index);
        }
        if let Some(p) = previous {
            close(&mut open, 1, p, &mut out);
        }
        out.retain(|g| g.first <= hi.saturating_add(1) && g.last.saturating_add(1) >= lo);
        out.sort_by_key(|g| (g.level, g.first));
        out
    }

    /// Where the summary of a group lies: after it or before it; `None`
    /// at the edge of the sheet.
    pub fn summary_of(group: &Group, after: bool, count: u32) -> Option<u32> {
        if after {
            Some(group.last + 1).filter(|i| *i < count)
        } else {
            group.first.checked_sub(1)
        }
    }

    /// The groups whose summary is `index`, outermost first.
    pub fn summarized_by(&self, index: u32, after: bool) -> Vec<Group> {
        let neighbour = if after {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        let Some(neighbour) = neighbour else {
            return Vec::new();
        };
        let own = self.level(index);
        (own + 1..=self.level(neighbour))
            .filter_map(|level| self.group_at(neighbour, level))
            .collect()
    }

    /// Forget the marks of summaries that summarize nothing any more.
    fn tidy(&mut self, after: bool) {
        let stale: Vec<u32> = self
            .collapsed
            .iter()
            .copied()
            .filter(|i| self.summarized_by(*i, after).is_empty())
            .collect();
        for index in stale {
            self.collapsed.remove(&index);
        }
    }

    /// Rows (columns) were inserted or deleted. Inserted ones join the
    /// group of the neighbour they take their formats from on the side
    /// away from the summary.
    fn shift(&mut self, insert: bool, at: u32, count: u32, max: u32, after: bool) {
        let inherited = if !insert {
            0
        } else if after {
            at.checked_sub(1).map_or(0, |i| self.level(i))
        } else {
            self.level(at)
        };
        let moved = |index: u32| -> Option<u32> {
            if insert {
                Some(if index >= at {
                    index.checked_add(count)?
                } else {
                    index
                })
                .filter(|i| *i < max)
            } else if index < at {
                Some(index)
            } else {
                (index >= at.saturating_add(count)).then(|| index - count)
            }
        };
        self.levels = self
            .levels
            .iter()
            .filter_map(|(i, level)| Some((moved(*i)?, *level)))
            .collect();
        self.collapsed = self.collapsed.iter().filter_map(|i| moved(*i)).collect();
        if inherited > 0 {
            for index in at..at.saturating_add(count).min(max) {
                self.levels.insert(index, inherited);
            }
        }
        self.tidy(after);
    }
}

/// The outline of a sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub rows: AxisOutline,
    pub cols: AxisOutline,
    /// "Summary rows below detail".
    pub summary_below: bool,
    /// "Summary columns to right of detail".
    pub summary_right: bool,
    /// "Show outline symbols": the margin with the bars and the buttons.
    pub show_symbols: bool,
}

impl Default for Outline {
    fn default() -> Outline {
        Outline {
            rows: AxisOutline::default(),
            cols: AxisOutline::default(),
            summary_below: true,
            summary_right: true,
            show_symbols: true,
        }
    }
}

impl Outline {
    pub fn axis(&self, axis: Axis) -> &AxisOutline {
        match axis {
            Axis::Rows => &self.rows,
            Axis::Cols => &self.cols,
        }
    }

    fn axis_mut(&mut self, axis: Axis) -> &mut AxisOutline {
        match axis {
            Axis::Rows => &mut self.rows,
            Axis::Cols => &mut self.cols,
        }
    }

    /// Summaries come after their group (below, to the right).
    pub fn summary_after(&self, axis: Axis) -> bool {
        match axis {
            Axis::Rows => self.summary_below,
            Axis::Cols => self.summary_right,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.cols.is_empty()
    }
}

/// How many rows or columns a sheet has.
fn count(axis: Axis) -> u32 {
    match axis {
        Axis::Rows => MAX_ROWS,
        Axis::Cols => MAX_COLS,
    }
}

impl Sheet {
    fn hidden(&self, axis: Axis) -> &BTreeSet<u32> {
        match axis {
            Axis::Rows => &self.hidden_rows,
            Axis::Cols => &self.hidden_cols,
        }
    }

    fn hidden_mut(&mut self, axis: Axis) -> &mut BTreeSet<u32> {
        match axis {
            Axis::Rows => &mut self.hidden_rows,
            Axis::Cols => &mut self.hidden_cols,
        }
    }

    /// True when every row (column) of the group is hidden: its button
    /// shows "+".
    pub fn outline_collapsed(&self, axis: Axis, group: &Group) -> bool {
        let hidden = self.hidden(axis);
        hidden.range(group.first..=group.last).count() as u32 == group.last - group.first + 1
    }

    /// The summary row (column) of a group.
    pub fn outline_summary(&self, axis: Axis, group: &Group) -> Option<u32> {
        AxisOutline::summary_of(group, self.outline.summary_after(axis), count(axis))
    }

    fn collapse(&mut self, axis: Axis, group: &Group) {
        let summary = self.outline_summary(axis, group);
        self.hidden_mut(axis).extend(group.first..=group.last);
        if let Some(summary) = summary {
            self.outline.axis_mut(axis).collapsed.insert(summary);
        }
    }

    fn expand(&mut self, axis: Axis, group: &Group) {
        if let Some(summary) = self.outline_summary(axis, group) {
            self.outline.axis_mut(axis).collapsed.remove(&summary);
        }
        for index in group.first..=group.last {
            self.hidden_mut(axis).remove(&index);
        }
        // Inner groups that were folded stay folded.
        let after = self.outline.summary_after(axis);
        let lines = self.outline.axis(axis);
        let inner: Vec<Group> = lines
            .groups(group.first, group.last)
            .into_iter()
            .filter(|g| g.level > group.level && g.first >= group.first && g.last <= group.last)
            .filter(|g| {
                AxisOutline::summary_of(g, after, count(axis))
                    .is_some_and(|s| lines.collapsed.contains(&s))
            })
            .collect();
        for g in inner {
            self.hidden_mut(axis).extend(g.first..=g.last);
        }
    }
}

/// Why an outline command did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineError {
    /// The rows are already at the deepest level.
    TooDeep,
    /// Nothing is grouped there.
    NothingToUngroup,
    /// No formula of the sheet summarizes rows or columns.
    NoSummaries,
    /// The selection touches no group.
    NoGroup,
}

impl Workbook {
    /// Data > Group: one level deeper for the rows (columns) `lo..=hi`.
    pub fn outline_group(
        &mut self,
        sheet: usize,
        axis: Axis,
        lo: u32,
        hi: u32,
    ) -> Result<(), OutlineError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let lines = sh.outline.axis(axis);
        let hi = hi.min(count(axis) - 1);
        if (lo..=hi).all(|i| lines.level(i) >= MAX_LEVEL)
            || lines.levels.range(lo..=hi).any(|(_, l)| *l >= MAX_LEVEL)
        {
            return Err(OutlineError::TooDeep);
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let after = sh.outline.summary_after(axis);
        let lines = sh.outline.axis_mut(axis);
        for index in lo..=hi {
            lines.set_level(index, lines.level(index) + 1);
        }
        lines.tidy(after);
        Ok(())
    }

    /// Data > Ungroup: one level less for the rows (columns) `lo..=hi`.
    pub fn outline_ungroup(
        &mut self,
        sheet: usize,
        axis: Axis,
        lo: u32,
        hi: u32,
    ) -> Result<(), OutlineError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let grouped: Vec<(u32, u8)> = sh
            .outline
            .axis(axis)
            .levels
            .range(lo..=hi)
            .map(|(i, l)| (*i, *l))
            .collect();
        if grouped.is_empty() {
            return Err(OutlineError::NothingToUngroup);
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let after = sh.outline.summary_after(axis);
        for (index, level) in grouped {
            sh.outline.axis_mut(axis).set_level(index, level - 1);
            // What leaves the outline cannot stay folded away.
            if level == 1 {
                sh.hidden_mut(axis).remove(&index);
            }
        }
        sh.outline.axis_mut(axis).tidy(after);
        Ok(())
    }

    /// Fold or unfold one group (a click on its button, or on its bar).
    pub fn outline_set_collapsed(
        &mut self,
        sheet: usize,
        axis: Axis,
        group: &Group,
        collapsed: bool,
    ) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        if collapsed {
            sh.collapse(axis, group);
        } else {
            sh.expand(axis, group);
        }
    }

    /// A click on the level button `shown` (1 = the summaries only): the
    /// groups of that level and deeper fold, the others unfold.
    pub fn outline_show_level(&mut self, sheet: usize, axis: Axis, shown: u8) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let lines = sh.outline.axis(axis);
        let grouped: Vec<u32> = lines.levels.keys().copied().collect();
        let groups = lines.groups(0, count(axis) - 1);
        sh.outline.axis_mut(axis).collapsed.clear();
        for index in grouped {
            sh.hidden_mut(axis).remove(&index);
        }
        for group in groups.iter().filter(|g| g.level >= shown.max(1)) {
            sh.collapse(axis, group);
        }
    }

    /// The groups Show Detail and Hide Detail act on for a selection: for
    /// each row (column) of it, the folded group it summarizes when
    /// showing; when hiding, the group it summarizes, else the innermost
    /// one it belongs to.
    fn detail_groups(&self, sheet: usize, axis: Axis, lo: u32, hi: u32, show: bool) -> Vec<Group> {
        let sh = &self.sheets[sheet];
        let lines = sh.outline.axis(axis);
        let after = sh.outline.summary_after(axis);
        let mut out: Vec<Group> = Vec::new();
        let hi = hi.min(lines.levels.keys().next_back().map_or(0, |last| last + 1));
        for index in lo..=hi {
            let summarized = lines.summarized_by(index, after);
            let found = if show {
                // The outermost folded group opens first, as with its button.
                summarized
                    .into_iter()
                    .find(|g| sh.outline_collapsed(axis, g))
            } else {
                summarized
                    .into_iter()
                    .rev()
                    .find(|g| !sh.outline_collapsed(axis, g))
                    .or_else(|| {
                        lines
                            .group_at(index, lines.level(index))
                            .filter(|g| !sh.outline_collapsed(axis, g))
                    })
            };
            if let Some(group) = found {
                if !out.contains(&group) {
                    out.push(group);
                }
            }
        }
        out
    }

    /// Data > Show Detail / Hide Detail on a selection: rows first, and
    /// columns when no group of rows answers.
    pub fn outline_detail(
        &mut self,
        sheet: usize,
        selection: Rect,
        show: bool,
    ) -> Result<(), OutlineError> {
        if sheet >= self.sheets.len() {
            return Ok(());
        }
        let spans = [
            (Axis::Rows, selection.r0, selection.r1),
            (Axis::Cols, selection.c0, selection.c1),
        ];
        for (axis, lo, hi) in spans {
            let groups = self.detail_groups(sheet, axis, lo, hi, show);
            if groups.is_empty() {
                continue;
            }
            self.snapshot();
            for group in groups {
                if show {
                    self.sheets[sheet].expand(axis, &group);
                } else {
                    self.sheets[sheet].collapse(axis, &group);
                }
            }
            return Ok(());
        }
        Err(OutlineError::NoGroup)
    }

    /// Data > Group > Auto Outline: the outline follows from the formulas.
    /// A formula that reads a block of rows lying wholly above it (below,
    /// when summaries are above) makes those rows its detail, and likewise
    /// for columns. `area` limits it to a selection of several cells.
    pub fn auto_outline(&mut self, sheet: usize, area: Option<Rect>) -> Result<(), OutlineError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let (below, right) = (sh.outline.summary_below, sh.outline.summary_right);
        let mut row_spans: BTreeSet<(u32, u32)> = BTreeSet::new();
        let mut col_spans: BTreeSet<(u32, u32)> = BTreeSet::new();
        for ((row, col), cell) in &sh.cells {
            let Content::Formula(formula) = &cell.content else {
                continue;
            };
            if area.is_some_and(|a| !a.contains(*row, *col)) {
                continue;
            }
            formula.walk(&mut |e| {
                let Expr::Ref(r) = e else {
                    return;
                };
                if r.kind != RefKind::Cells
                    || r.sheet.as_deref().is_some_and(|s| !same_name(s, &sh.name))
                {
                    return;
                }
                let mut read = r.rect();
                if let Some(a) = area {
                    match read.intersection(&a) {
                        Some(part) => read = part,
                        None => return,
                    }
                }
                let detail = |lo: u32, hi: u32, own: u32, after: bool| {
                    hi > lo && if after { hi < own } else { lo > own }
                };
                if detail(read.r0, read.r1, *row, below) {
                    row_spans.insert((read.r0, read.r1));
                }
                if detail(read.c0, read.c1, *col, right) {
                    col_spans.insert((read.c0, read.c1));
                }
            });
        }
        if row_spans.is_empty() && col_spans.is_empty() {
            return Err(OutlineError::NoSummaries);
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        for (axis, spans) in [(Axis::Rows, row_spans), (Axis::Cols, col_spans)] {
            if spans.is_empty() {
                continue;
            }
            // The outline is made anew where the formulas give one.
            let within = |i: u32| match (axis, area) {
                (_, None) => true,
                (Axis::Rows, Some(a)) => i >= a.r0 && i <= a.r1,
                (Axis::Cols, Some(a)) => i >= a.c0 && i <= a.c1,
            };
            let old: Vec<u32> = sh
                .outline
                .axis(axis)
                .levels
                .keys()
                .copied()
                .filter(|i| within(*i))
                .collect();
            for index in old {
                sh.outline.axis_mut(axis).levels.remove(&index);
                sh.hidden_mut(axis).remove(&index);
            }
            let lines = sh.outline.axis_mut(axis);
            for (lo, hi) in spans {
                for index in lo..=hi {
                    lines.set_level(index, lines.level(index) + 1);
                }
            }
            let after = sh.outline.summary_after(axis);
            sh.outline.axis_mut(axis).tidy(after);
        }
        Ok(())
    }

    /// Data > Ungroup > Clear Outline: of the whole sheet, or of the rows
    /// and columns of a selection of several cells. What was folded shows
    /// again.
    pub fn clear_outline(&mut self, sheet: usize, area: Option<Rect>) -> Result<(), OutlineError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let span = |axis: Axis| match (axis, area) {
            (_, None) => (0, u32::MAX),
            (Axis::Rows, Some(a)) => (a.r0, a.r1),
            (Axis::Cols, Some(a)) => (a.c0, a.c1),
        };
        let grouped = |axis: Axis| -> Vec<u32> {
            let (lo, hi) = span(axis);
            sh.outline
                .axis(axis)
                .levels
                .range(lo..=hi)
                .map(|(i, _)| *i)
                .collect()
        };
        let (rows, cols) = (grouped(Axis::Rows), grouped(Axis::Cols));
        if rows.is_empty() && cols.is_empty() {
            return Err(OutlineError::NothingToUngroup);
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        for (axis, gone) in [(Axis::Rows, rows), (Axis::Cols, cols)] {
            for index in gone {
                sh.outline.axis_mut(axis).levels.remove(&index);
                sh.hidden_mut(axis).remove(&index);
            }
            let after = sh.outline.summary_after(axis);
            sh.outline.axis_mut(axis).tidy(after);
        }
        Ok(())
    }

    /// The settings of the outline (the dialog behind the Outline group):
    /// where summaries lie, and whether the symbols show.
    pub fn set_outline_settings(
        &mut self,
        sheet: usize,
        summary_below: bool,
        summary_right: bool,
        show_symbols: bool,
    ) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let o = &sh.outline;
        if (o.summary_below, o.summary_right, o.show_symbols)
            == (summary_below, summary_right, show_symbols)
        {
            return;
        }
        self.snapshot();
        let o = &mut self.sheets[sheet].outline;
        o.summary_below = summary_below;
        o.summary_right = summary_right;
        o.show_symbols = show_symbols;
        o.rows.tidy(summary_below);
        o.cols.tidy(summary_right);
    }

    /// Give rows their levels outright (Subtotal builds its outline so):
    /// `levels` holds (first row, last row, level).
    pub(crate) fn set_row_levels(&mut self, sheet: usize, levels: &[(u32, u32, u8)]) {
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        for &(lo, hi, level) in levels {
            for row in lo..=hi {
                sh.outline.rows.set_level(row, level);
            }
        }
        let below = sh.outline.summary_below;
        sh.outline.rows.tidy(below);
    }
}

pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.outline.is_empty() || !same_name(&sheet.name, edited_sheet) {
        return;
    }
    let (below, right) = (sheet.outline.summary_below, sheet.outline.summary_right);
    match edit {
        StructuralEdit::InsertRows { at, count } => {
            sheet.outline.rows.shift(true, at, count, MAX_ROWS, below)
        }
        StructuralEdit::DeleteRows { at, count } => {
            sheet.outline.rows.shift(false, at, count, MAX_ROWS, below)
        }
        StructuralEdit::InsertCols { at, count } => {
            sheet.outline.cols.shift(true, at, count, MAX_COLS, right)
        }
        StructuralEdit::DeleteCols { at, count } => {
            sheet.outline.cols.shift(false, at, count, MAX_COLS, right)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;

    fn workbook() -> Workbook {
        Workbook::new(Locale::en())
    }

    fn levels(wb: &Workbook, axis: Axis, lo: u32, hi: u32) -> Vec<u8> {
        (lo..=hi)
            .map(|i| wb.sheets[0].outline.axis(axis).level(i))
            .collect()
    }

    fn hidden(wb: &Workbook) -> Vec<u32> {
        wb.sheets[0].hidden_rows.iter().copied().collect()
    }

    /// Rows 2-4 and 6-8 at level 2 under summaries in rows 5 and 9, all
    /// of it at level 1 under row 10 (rows counted from 1).
    fn nested() -> Workbook {
        let mut wb = workbook();
        wb.outline_group(0, Axis::Rows, 1, 8).unwrap();
        wb.outline_group(0, Axis::Rows, 1, 3).unwrap();
        wb.outline_group(0, Axis::Rows, 5, 7).unwrap();
        wb.commit("Group");
        wb
    }

    #[test]
    fn grouping_and_ungrouping_change_the_level() {
        let mut wb = nested();
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 9),
            [0, 2, 2, 2, 1, 2, 2, 2, 1, 0]
        );
        let groups = wb.sheets[0].outline.rows.groups(0, 20);
        assert_eq!(
            groups,
            [
                Group {
                    level: 1,
                    first: 1,
                    last: 8
                },
                Group {
                    level: 2,
                    first: 1,
                    last: 3
                },
                Group {
                    level: 2,
                    first: 5,
                    last: 7
                },
            ]
        );
        // Only the groups in sight, and those whose summary is.
        assert_eq!(wb.sheets[0].outline.rows.groups(4, 4).len(), 3);
        assert_eq!(wb.sheets[0].outline.rows.groups(2, 2).len(), 2);
        assert_eq!(
            wb.sheets[0].outline.rows.groups(9, 30),
            [Group {
                level: 1,
                first: 1,
                last: 8
            }]
        );
        assert_eq!(wb.sheets[0].outline.rows.groups(11, 30), []);

        wb.outline_ungroup(0, Axis::Rows, 5, 7).unwrap();
        wb.commit("Ungroup");
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 9),
            [0, 2, 2, 2, 1, 1, 1, 1, 1, 0]
        );
        assert_eq!(
            wb.outline_ungroup(0, Axis::Rows, 20, 30),
            Err(OutlineError::NothingToUngroup)
        );
        wb.undo();
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 9),
            [0, 2, 2, 2, 1, 2, 2, 2, 1, 0]
        );

        // Eight levels and no more.
        for _ in 0..5 {
            wb.outline_group(0, Axis::Rows, 1, 3).unwrap();
        }
        assert_eq!(wb.sheets[0].outline.rows.level(1), MAX_LEVEL);
        assert_eq!(
            wb.outline_group(0, Axis::Rows, 0, 3),
            Err(OutlineError::TooDeep)
        );
        // Columns have their own outline.
        wb.outline_group(0, Axis::Cols, 1, 2).unwrap();
        assert_eq!(levels(&wb, Axis::Cols, 0, 3), [0, 1, 1, 0]);
    }

    #[test]
    fn folding_hides_rows_and_inner_groups_stay_folded() {
        let mut wb = nested();
        let inner = Group {
            level: 2,
            first: 1,
            last: 3,
        };
        let outer = Group {
            level: 1,
            first: 1,
            last: 8,
        };
        wb.outline_set_collapsed(0, Axis::Rows, &inner, true);
        wb.commit("Hide Detail");
        assert_eq!(hidden(&wb), [1, 2, 3]);
        assert!(wb.sheets[0].outline_collapsed(Axis::Rows, &inner));
        assert!(wb.sheets[0].outline.rows.collapsed.contains(&4));
        assert_eq!(wb.sheets[0].row_height(2), 0.0);

        wb.outline_set_collapsed(0, Axis::Rows, &outer, true);
        wb.commit("Hide Detail");
        assert_eq!(hidden(&wb), [1, 2, 3, 4, 5, 6, 7, 8]);
        wb.outline_set_collapsed(0, Axis::Rows, &outer, false);
        wb.commit("Show Detail");
        assert_eq!(hidden(&wb), [1, 2, 3]);
        wb.outline_set_collapsed(0, Axis::Rows, &inner, false);
        wb.commit("Show Detail");
        assert_eq!(hidden(&wb), []);
        assert!(wb.sheets[0].outline.rows.collapsed.is_empty());
        wb.undo();
        assert_eq!(hidden(&wb), [1, 2, 3]);
    }

    #[test]
    fn level_buttons_show_one_level() {
        let mut wb = nested();
        wb.outline_show_level(0, Axis::Rows, 1);
        wb.commit("Level");
        assert_eq!(hidden(&wb), [1, 2, 3, 4, 5, 6, 7, 8]);
        // The inner groups are marked too: they stay folded when the
        // outer one opens.
        assert_eq!(
            wb.sheets[0]
                .outline
                .rows
                .collapsed
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [4, 8, 9]
        );
        wb.outline_set_collapsed(
            0,
            Axis::Rows,
            &Group {
                level: 1,
                first: 1,
                last: 8,
            },
            false,
        );
        assert_eq!(hidden(&wb), [1, 2, 3, 5, 6, 7]);
        wb.outline_show_level(0, Axis::Rows, 2);
        assert_eq!(hidden(&wb), [1, 2, 3, 5, 6, 7]);
        wb.outline_show_level(0, Axis::Rows, 3);
        assert_eq!(hidden(&wb), []);
    }

    #[test]
    fn show_and_hide_detail_follow_the_selection() {
        let mut wb = nested();
        // On a summary row: its group.
        wb.outline_detail(0, Rect::cell(4, 0), false).unwrap();
        assert_eq!(hidden(&wb), [1, 2, 3]);
        wb.outline_detail(0, Rect::cell(4, 0), true).unwrap();
        assert_eq!(hidden(&wb), []);
        // On a detail row: the innermost group around it.
        wb.outline_detail(0, Rect::cell(6, 2), false).unwrap();
        assert_eq!(hidden(&wb), [5, 6, 7]);
        wb.outline_detail(0, Rect::cell(8, 0), true).unwrap();
        assert_eq!(hidden(&wb), []);
        assert_eq!(
            wb.outline_detail(0, Rect::cell(20, 0), false),
            Err(OutlineError::NoGroup)
        );
        // Columns answer when the rows of the selection have no group.
        wb.outline_group(0, Axis::Cols, 1, 2).unwrap();
        wb.outline_detail(0, Rect::cell(20, 3), false).unwrap();
        assert_eq!(
            wb.sheets[0].hidden_cols.iter().copied().collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn summaries_above_put_the_button_before_the_group() {
        let mut wb = workbook();
        wb.set_outline_settings(0, false, true, true);
        wb.outline_group(0, Axis::Rows, 3, 5).unwrap();
        let group = wb.sheets[0].outline.rows.group_at(4, 1).unwrap();
        assert_eq!(wb.sheets[0].outline_summary(Axis::Rows, &group), Some(2));
        wb.outline_detail(0, Rect::cell(2, 0), false).unwrap();
        assert_eq!(hidden(&wb), [3, 4, 5]);
        assert!(wb.sheets[0].outline.rows.collapsed.contains(&2));
        // A group at the top of the sheet has no summary row.
        wb.outline_group(0, Axis::Rows, 0, 0).unwrap();
        let top = wb.sheets[0].outline.rows.group_at(0, 1).unwrap();
        assert_eq!(wb.sheets[0].outline_summary(Axis::Rows, &top), None);
    }

    #[test]
    fn the_outline_follows_inserted_and_deleted_rows() {
        let mut wb = nested();
        wb.outline_set_collapsed(
            0,
            Axis::Rows,
            &Group {
                level: 2,
                first: 5,
                last: 7,
            },
            true,
        );
        wb.commit("Hide Detail");
        // A row inserted inside a group joins it; so does one inserted
        // just above the summary.
        wb.insert_rows(0, 2, 1);
        wb.commit("Insert");
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 10),
            [0, 2, 2, 2, 2, 1, 2, 2, 2, 1, 0]
        );
        assert_eq!(hidden(&wb), [6, 7, 8]);
        assert!(wb.sheets[0].outline.rows.collapsed.contains(&9));
        wb.insert_rows(0, 5, 1);
        wb.commit("Insert");
        assert_eq!(levels(&wb, Axis::Rows, 0, 6), [0, 2, 2, 2, 2, 2, 1]);
        wb.undo();
        // Deleting the rows of a group takes its mark away.
        wb.delete_rows(0, 6, 3);
        wb.commit("Delete");
        assert_eq!(levels(&wb, Axis::Rows, 0, 7), [0, 2, 2, 2, 2, 1, 1, 0]);
        assert!(wb.sheets[0].outline.rows.collapsed.is_empty());
        // Another sheet's rows do not move this outline.
        let other = wb.add_sheet(1);
        wb.insert_rows(other, 0, 3);
        wb.commit("Insert");
        assert_eq!(levels(&wb, Axis::Rows, 0, 2), [0, 2, 2]);
        // Columns.
        wb.outline_group(0, Axis::Cols, 2, 3).unwrap();
        wb.insert_cols(0, 0, 2);
        wb.delete_cols(0, 5, 1);
        wb.commit("Columns");
        assert_eq!(levels(&wb, Axis::Cols, 3, 5), [0, 1, 0]);
    }

    #[test]
    fn a_copied_sheet_keeps_its_outline() {
        let mut wb = nested();
        let copy = wb.duplicate_sheet(0).unwrap();
        wb.commit("Copy");
        assert_eq!(wb.sheets[copy].outline, wb.sheets[0].outline);
        assert_eq!(wb.sheets[copy].outline.rows.max_level(), 2);
    }

    #[test]
    fn auto_outline_reads_the_formulas() {
        let mut wb = workbook();
        for row in 1..=3 {
            wb.set_cell_text(0, row, 1, "10").unwrap();
            wb.set_cell_text(0, row + 4, 1, "20").unwrap();
            wb.set_cell_text(0, row, 2, "1").unwrap();
            wb.set_cell_text(0, row, 3, "=SUM(B2:C2)").unwrap();
        }
        wb.set_cell_text(0, 4, 1, "=SUM(B2:B4)").unwrap();
        wb.set_cell_text(0, 8, 1, "=SUM(B6:B8)").unwrap();
        wb.set_cell_text(0, 9, 1, "=SUBTOTAL(9,B2:B9)").unwrap();
        wb.commit("Enter");
        wb.auto_outline(0, None).unwrap();
        wb.commit("Auto Outline");
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 9),
            [0, 2, 2, 2, 1, 2, 2, 2, 1, 0]
        );
        // The formulas across make the columns' outline.
        assert_eq!(levels(&wb, Axis::Cols, 0, 3), [0, 1, 1, 0]);

        wb.clear_outline(0, Some(Rect::new(0, 0, 4, 0))).unwrap();
        wb.commit("Clear Outline");
        assert_eq!(
            levels(&wb, Axis::Rows, 0, 9),
            [0, 0, 0, 0, 0, 2, 2, 2, 1, 0]
        );
        assert_eq!(levels(&wb, Axis::Cols, 0, 3), [0, 1, 1, 0]);
        wb.clear_outline(0, None).unwrap();
        assert!(wb.sheets[0].outline.is_empty());
        assert_eq!(
            wb.clear_outline(0, None),
            Err(OutlineError::NothingToUngroup)
        );

        let mut plain = workbook();
        plain.set_cell_text(0, 0, 0, "=B1+1").unwrap();
        assert_eq!(plain.auto_outline(0, None), Err(OutlineError::NoSummaries));
    }

    #[test]
    fn folded_rows_leave_subtotal_101() {
        let mut wb = workbook();
        for row in 0..4 {
            wb.set_cell_text(0, row, 0, "5").unwrap();
        }
        wb.set_cell_text(0, 4, 0, "=SUBTOTAL(109,A1:A4)").unwrap();
        wb.outline_group(0, Axis::Rows, 1, 2).unwrap();
        wb.commit("Group");
        assert_eq!(wb.value_at(0, 4, 0), crate::Value::Number(20.0));
        wb.outline_detail(0, Rect::cell(1, 0), false).unwrap();
        wb.commit("Hide Detail");
        assert_eq!(wb.value_at(0, 4, 0), crate::Value::Number(10.0));
    }
}
