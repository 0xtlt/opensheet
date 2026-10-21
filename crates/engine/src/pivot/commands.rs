//! The PivotTable commands of the workbook: create, change, refresh, move,
//! group, drill down. Each one checks everything before it touches the
//! workbook, so a refused command leaves no trace; a successful one has
//! taken the undo snapshot and waits for the caller's `commit`.

use super::compute::{self, FormatOf, NONE, Tag, Tree};
use super::{
    Area, AxisField, FieldGroup, GroupBy, GroupKind, PivotCache, PivotCell, PivotError,
    PivotSource, PivotTable, Report, cuts, same_name,
};
use crate::ast::StructuralEdit;
use crate::refs::Rect;
use crate::style::{HAlign, StyleId};
use crate::value::Value;
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::collections::HashMap;
use std::sync::Arc;

/// Where a PivotTable is created or moved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PivotDestination {
    /// A new sheet inserted at this index; the report starts in A3, as
    /// Excel leaves room for a filter above it.
    NewSheet { index: usize },
    /// The top-left cell of the PivotTable on an existing sheet.
    Existing { sheet: usize, row: u32, col: u32 },
}

/// A PivotTable to write: on `sheet`, replacing the one at `index` or new.
pub(super) type Pending = (usize, Option<usize>, PivotTable);

struct Placed {
    sheet: usize,
    index: Option<usize>,
    pivot: PivotTable,
    report: Report,
    body: Rect,
    pages: Option<Rect>,
}

impl Placed {
    fn rects(&self) -> impl Iterator<Item = Rect> {
        [Some(self.body), self.pages].into_iter().flatten()
    }
}

/// Remove the cells of `rect`, formatting included, except those inside
/// one of `keep`.
fn clear_rect(sh: &mut Sheet, rect: Rect, keep: &[Rect]) {
    let keys: Vec<(u32, u32)> = sh
        .cells
        .range((rect.r0, rect.c0)..=(rect.r1, rect.c1))
        .map(|(k, _)| *k)
        .filter(|(r, c)| *c >= rect.c0 && *c <= rect.c1 && !keep.iter().any(|k| k.contains(*r, *c)))
        .collect();
    for k in keys {
        sh.cells.remove(&k);
    }
}

fn put(sh: &mut Sheet, row: u32, col: u32, value: Value, style: StyleId) {
    if value.is_empty() && style == sh.inherited_style(row, col) {
        sh.cells.remove(&(row, col));
        return;
    }
    let content = if value.is_empty() {
        Content::Empty
    } else {
        Content::Value(value.clone())
    };
    sh.cells.insert(
        (row, col),
        Cell {
            content,
            value,
            style,
        },
    );
}

/// Insert the fields `new` where the first of the fields `old` (indices in
/// the definition `before` the cache changed) sat, in its area.
fn replace_in_area(
    before: &PivotTable,
    p: &mut PivotTable,
    old: &[usize],
    new: &[usize],
    locale: &crate::Locale,
) {
    let Some(area) = old.iter().find_map(|f| before.axis_of(*f)) else {
        return;
    };
    let entries: Vec<Option<usize>> = match area {
        Area::Rows => before.row_fields.iter().map(AxisField::field).collect(),
        Area::Columns => before.col_fields.iter().map(AxisField::field).collect(),
        _ => before.page_fields.iter().map(|pf| Some(pf.field)).collect(),
    };
    let Some(at) = entries
        .iter()
        .position(|e| e.is_some_and(|f| old.contains(&f)))
    else {
        return;
    };
    for (k, f) in new.iter().enumerate() {
        p.add_field(*f, area, Some(at + k), locale);
    }
}

impl Workbook {
    // ---- finding -----------------------------------------------------------

    /// Every PivotTable of the workbook with the index of its sheet.
    pub fn pivots(&self) -> impl Iterator<Item = (usize, &PivotTable)> {
        self.sheets
            .iter()
            .enumerate()
            .flat_map(|(si, sh)| sh.pivots.iter().map(move |p| (si, p)))
    }

    /// The PivotTable with this id and the index of its sheet.
    pub fn pivot(&self, id: u32) -> Option<(usize, &PivotTable)> {
        self.pivots().find(|(_, p)| p.id == id)
    }

    fn pivot_position(&self, id: u32) -> Result<(usize, usize), PivotError> {
        self.sheets
            .iter()
            .enumerate()
            .find_map(|(si, sh)| sh.pivots.iter().position(|p| p.id == id).map(|pi| (si, pi)))
            .ok_or(PivotError::NoSuchPivot)
    }

    /// The PivotTable a cell belongs to (report or filter block).
    pub fn pivot_at(&self, sheet: usize, row: u32, col: u32) -> Option<&PivotTable> {
        self.sheets
            .get(sheet)?
            .pivots
            .iter()
            .find(|p| p.contains(row, col))
    }

    /// Whether a cell is part of a PivotTable, and which part.
    pub fn pivot_cell(&self, sheet: usize, row: u32, col: u32) -> Option<PivotCell> {
        self.pivot_at(sheet, row, col)?.cell(row, col)
    }

    /// The PivotTables a range touches: (id, whether the range covers the
    /// whole PivotTable). Excel only lets a range be cleared or deleted when
    /// it covers the PivotTables it touches, which then go with it.
    pub fn pivots_in(&self, sheet: usize, rect: Rect) -> Vec<(u32, bool)> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        sh.pivots
            .iter()
            .filter(|p| p.footprint().any(|r| r.intersects(&rect)))
            .map(|p| {
                (
                    p.id,
                    p.footprint()
                        .all(|r| rect.contains(r.r0, r.c0) && rect.contains(r.r1, r.c1)),
                )
            })
            .collect()
    }

    /// True when inserting or deleting these rows or columns would cut
    /// through a PivotTable of the sheet, which Excel refuses ("We can't
    /// make this change for the selected cells because it will affect a
    /// PivotTable").
    pub fn pivot_blocks_edit(&self, sheet: usize, edit: StructuralEdit) -> bool {
        self.sheets.get(sheet).is_some_and(|sh| {
            sh.pivots
                .iter()
                .any(|p| p.footprint().any(|r| cuts(r, edit)))
        })
    }

    /// The name Excel would give the next PivotTable ("PivotTable1").
    pub fn pivot_default_name(&self) -> String {
        let prefix = super::caption(super::Caption::PivotTable, self.locale);
        (1..)
            .map(|k| format!("{prefix}{k}"))
            .find(|name| !self.pivots().any(|(_, p)| same_name(&p.name, name)))
            .unwrap_or_else(|| prefix.to_string())
    }

    // ---- writing -----------------------------------------------------------

    /// Why a report cannot be written where it is placed.
    fn pivot_conflict(
        &self,
        x: &Placed,
        all: &[Placed],
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let sh = &self.sheets[x.sheet];
        let own: Vec<Rect> = x
            .index
            .map(|pi| sh.pivots[pi].footprint().collect())
            .unwrap_or_default();
        for rect in x.rects() {
            for (oi, other) in sh.pivots.iter().enumerate() {
                if Some(oi) == x.index {
                    continue;
                }
                // A PivotTable changed along with this one counts where it
                // is going, not where it was.
                let moved = all
                    .iter()
                    .find(|y| y.sheet == x.sheet && y.index == Some(oi));
                let hit = match moved {
                    Some(y) => y.rects().any(|r| r.intersects(&rect)),
                    None => other.footprint().any(|r| r.intersects(&rect)),
                };
                if hit {
                    return Err(PivotError::OverlapsPivot(other.name.clone()));
                }
            }
            if sh.tables.iter().any(|t| t.range.intersects(&rect)) {
                return Err(PivotError::OverlapsTable);
            }
            if sh.merges.iter().any(|m| m.intersects(&rect)) {
                return Err(PivotError::OverlapsMerged);
            }
            if overwrite {
                continue;
            }
            let mine = |r: u32, c: u32| own.iter().any(|o| o.contains(r, c));
            let data =
                sh.cells
                    .range((rect.r0, rect.c0)..=(rect.r1, rect.c1))
                    .any(|((r, c), cell)| {
                        *c >= rect.c0
                            && *c <= rect.c1
                            && !matches!(cell.content, Content::Empty)
                            && !mine(*r, *c)
                    });
            let spill = sh.spills.iter().any(|sp| {
                let area = Rect::new(
                    sp.row,
                    sp.col,
                    sp.row + sp.array.rows as u32 - 1,
                    sp.col + sp.array.cols as u32 - 1,
                );
                area.intersection(&rect)
                    .is_some_and(|o| !(mine(o.r0, o.c0) && mine(o.r1, o.c1)))
            });
            if data || spill {
                return Err(PivotError::WouldOverwrite(x.body));
            }
        }
        Ok(())
    }

    /// Write a report into the cells: values, number formats, the
    /// indentation of compact labels and the room for the expand / collapse
    /// buttons. Other formatting of the cells stays.
    fn pivot_write(&mut self, x: &Placed) {
        let (p, report) = (&x.pivot, &x.report);
        let general: Arc<str> = Arc::from("General");
        let mut memo: HashMap<(StyleId, FormatOf, Option<u8>), StyleId> = HashMap::new();
        let mut styled =
            |wb: &mut Workbook, row: u32, col: u32, of: FormatOf, indent: Option<u8>| -> StyleId {
                let current = wb.sheets[x.sheet].style_id_at(row, col);
                *memo.entry((current, of, indent)).or_insert_with(|| {
                    let mut s = wb.styles.get(current).clone();
                    let code = match of {
                        FormatOf::Nothing => None,
                        FormatOf::Field(f) => p.label_format(f),
                        FormatOf::Data(d) => p.data_format(d),
                        FormatOf::Subtotal(d) => {
                            p.data_fields.get(d).and_then(|d| d.num_fmt.clone())
                        }
                    };
                    s.num_fmt = code.unwrap_or_else(|| general.clone());
                    s.indent = indent.unwrap_or(0);
                    s.h_align = if indent.is_some() {
                        HAlign::Left
                    } else {
                        HAlign::General
                    };
                    wb.styles.intern(s)
                })
            };
        let (rows, cols) = report.size();
        for r in 0..rows {
            for c in 0..cols {
                let (row, col) = (x.body.r0 + r, x.body.c0 + c);
                let indent = match (report.indent(r, c), report.drill_margin(r, c, p)) {
                    (Some(depth), margin) => Some(depth + margin as u8),
                    (None, true) => Some(1),
                    (None, false) => None,
                };
                let style = styled(self, row, col, report.format_of(r, c, p), indent);
                put(
                    &mut self.sheets[x.sheet],
                    row,
                    col,
                    report.value(r, c).clone(),
                    style,
                );
            }
        }
        let Some(pages) = x.pages else {
            return;
        };
        for r in pages.r0..=pages.r1 {
            for c in pages.c0..=pages.c1 {
                let (pr, pc) = (r - pages.r0, c - pages.c0);
                let label = report.pages.iter().find(|pg| pg.row == pr && pg.col == pc);
                let value = report
                    .pages
                    .iter()
                    .find(|pg| pg.row == pr && pg.col + 1 == pc);
                let (v, of) = match (label, value) {
                    (Some(pg), _) => (Value::text(p.field_caption(pg.field)), FormatOf::Nothing),
                    (None, Some(pg)) => (pg.value.clone(), FormatOf::Field(pg.field)),
                    _ => (Value::Empty, FormatOf::Nothing),
                };
                let style = styled(self, r, c, of, None);
                put(&mut self.sheets[x.sheet], r, c, v, style);
            }
        }
    }

    /// Compute and write PivotTables, all or none.
    pub(super) fn pivot_install(
        &mut self,
        batch: Vec<Pending>,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let locale = self.locale;
        let mut placed = Vec::with_capacity(batch.len());
        for (sheet, index, mut pivot) in batch {
            if sheet >= self.sheets.len() || !pivot.cache.is_valid() {
                return Err(PivotError::InvalidSource);
            }
            pivot.normalize();
            let report = compute::compute(&pivot, locale);
            let (body, pages) = pivot.place(&report)?;
            placed.push(Placed {
                sheet,
                index,
                pivot,
                report,
                body,
                pages,
            });
        }
        for x in &placed {
            self.pivot_conflict(x, &placed, overwrite)?;
        }
        self.snapshot();
        for x in &placed {
            if let Some(pi) = x.index {
                let keep: Vec<Rect> = x.rects().collect();
                let old: Vec<Rect> = self.sheets[x.sheet].pivots[pi].footprint().collect();
                for rect in old {
                    clear_rect(&mut self.sheets[x.sheet], rect, &keep);
                }
            }
        }
        let written: Vec<u32> = placed.iter().map(|x| x.pivot.id).collect();
        for mut x in placed {
            self.pivot_write(&x);
            x.pivot.location = Some(x.body);
            x.pivot.page_location = x.pages;
            x.pivot.anchor = (x.body.r0, x.body.c0);
            x.pivot.report = Arc::new(x.report);
            let pivots = &mut self.sheets[x.sheet].pivots;
            match x.index {
                Some(pi) => pivots[pi] = x.pivot,
                None => pivots.push(x.pivot),
            }
        }
        // PivotCharts follow the reports just written.
        self.sync_pivot_charts(&written);
        Ok(())
    }

    /// Change a PivotTable and write its report again. `edit` works on a
    /// copy of the definition (see the methods of [`PivotTable`]); nothing
    /// changes when the new report cannot be written. With `overwrite`, the
    /// report may cover cells holding data.
    ///
    /// After loading a file, `pivot_update(id, true, |_| {})` computes the
    /// report of a PivotTable built from its definition and cache.
    pub fn pivot_update(
        &mut self,
        id: u32,
        overwrite: bool,
        edit: impl FnOnce(&mut PivotTable),
    ) -> Result<(), PivotError> {
        let (si, pi) = self.pivot_position(id)?;
        let mut p = self.sheets[si].pivots[pi].clone();
        edit(&mut p);
        self.pivot_install(vec![(si, Some(pi), p)], overwrite)
    }

    /// `pivot_update` for several PivotTables at once (those a slicer
    /// drives): all their reports are written, or none.
    pub fn pivot_update_many(
        &mut self,
        ids: &[u32],
        overwrite: bool,
        edit: impl Fn(&mut PivotTable),
    ) -> Result<(), PivotError> {
        let mut batch = Vec::with_capacity(ids.len());
        for id in ids {
            let (si, pi) = self.pivot_position(*id)?;
            let mut p = self.sheets[si].pivots[pi].clone();
            edit(&mut p);
            batch.push((si, Some(pi), p));
        }
        self.pivot_install(batch, overwrite)
    }

    /// Every PivotTable built on `old`, moved onto `new`.
    pub(super) fn pivot_rebound(
        &self,
        old: &Arc<PivotCache>,
        new: &Arc<PivotCache>,
    ) -> Vec<Pending> {
        let mut batch = Vec::new();
        for (si, sh) in self.sheets.iter().enumerate() {
            for (pi, p) in sh.pivots.iter().enumerate() {
                if Arc::ptr_eq(&p.cache, old) {
                    let mut q = p.clone();
                    q.rebind(new.clone());
                    batch.push((si, Some(pi), q));
                }
            }
        }
        batch
    }

    // ---- commands ----------------------------------------------------------

    /// Insert a PivotTable without fields; returns its id. A PivotTable on
    /// the same source as another one shares its cache, as in Excel: it
    /// shows the data as of that cache's last refresh.
    pub fn pivot_create(
        &mut self,
        source: PivotSource,
        dest: PivotDestination,
        overwrite: bool,
    ) -> Result<u32, PivotError> {
        let shared = self
            .pivots()
            .find(|(_, p)| p.source == source)
            .map(|(_, p)| p.cache.clone());
        let cache = match shared {
            Some(c) => c,
            None => Arc::new(PivotCache::read(self, &source)?),
        };
        let id = self.pivots().map(|(_, p)| p.id).max().unwrap_or(0) + 1;
        let name = self.pivot_default_name();
        match dest {
            PivotDestination::Existing { sheet, row, col } => {
                let p = PivotTable::new(id, name, source, cache, (row, col));
                self.pivot_install(vec![(sheet, None, p)], overwrite)?;
            }
            PivotDestination::NewSheet { index } => {
                let sheet = self.add_sheet(index);
                let p = PivotTable::new(id, name, source, cache, (2, 0));
                self.pivot_install(vec![(sheet, None, p)], true)?;
            }
        }
        Ok(id)
    }

    /// Read the source again and recompute the PivotTable, and with it the
    /// others built on the same cache.
    pub fn pivot_refresh(&mut self, id: u32, overwrite: bool) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let (old, source) = (p.cache.clone(), p.source.clone());
        let mut cache = PivotCache::read(self, &source)?;
        cache.carry_groups(&old, self.locale);
        cache.carry_calculated(&old);
        let batch = self.pivot_rebound(&old, &Arc::new(cache));
        self.pivot_install(batch, overwrite)
    }

    /// "Refresh All".
    pub fn pivot_refresh_all(&mut self, overwrite: bool) -> Result<(), PivotError> {
        let mut caches: Vec<(Arc<PivotCache>, PivotSource)> = Vec::new();
        for (_, p) in self.pivots() {
            if !caches.iter().any(|(c, _)| Arc::ptr_eq(c, &p.cache)) {
                caches.push((p.cache.clone(), p.source.clone()));
            }
        }
        let mut batch = Vec::new();
        for (old, source) in caches {
            let mut cache = PivotCache::read(self, &source)?;
            cache.carry_groups(&old, self.locale);
            cache.carry_calculated(&old);
            batch.extend(self.pivot_rebound(&old, &Arc::new(cache)));
        }
        if batch.is_empty() {
            return Ok(());
        }
        self.pivot_install(batch, overwrite)
    }

    /// "Change Data Source": the PivotTable gets a cache of its own on the
    /// new source; fields are matched by name.
    pub fn pivot_change_source(
        &mut self,
        id: u32,
        source: PivotSource,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (si, pi) = self.pivot_position(id)?;
        let mut p = self.sheets[si].pivots[pi].clone();
        let mut cache = PivotCache::read(self, &source)?;
        cache.carry_groups(&p.cache, self.locale);
        cache.carry_calculated(&p.cache);
        p.rebind(Arc::new(cache));
        p.source = source;
        self.pivot_install(vec![(si, Some(pi), p)], overwrite)
    }

    /// Rename a PivotTable. The name must not be empty nor used by another
    /// PivotTable of the sheet.
    pub fn pivot_rename(&mut self, id: u32, name: &str) -> Result<(), PivotError> {
        let (si, pi) = self.pivot_position(id)?;
        let name = name.trim();
        if name.is_empty()
            || self.sheets[si]
                .pivots
                .iter()
                .enumerate()
                .any(|(k, p)| k != pi && same_name(&p.name, name))
        {
            return Err(PivotError::InvalidName);
        }
        self.snapshot();
        self.sheets[si].pivots[pi].name = name.to_string();
        Ok(())
    }

    /// Delete a PivotTable and what it wrote.
    pub fn pivot_delete(&mut self, id: u32) -> Result<(), PivotError> {
        let (si, pi) = self.pivot_position(id)?;
        self.snapshot();
        let p = self.sheets[si].pivots.remove(pi);
        for rect in p.footprint() {
            clear_rect(&mut self.sheets[si], rect, &[]);
        }
        Ok(())
    }

    /// "Move PivotTable": `dest` is where the top-left cell of the whole
    /// PivotTable goes, filters included.
    pub fn pivot_move(
        &mut self,
        id: u32,
        dest: PivotDestination,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (si, pi) = self.pivot_position(id)?;
        let page_rows = self.sheets[si].pivots[pi].report.page_size().0;
        let above = if page_rows > 0 { page_rows + 1 } else { 0 };
        let (sheet, row, col) = match dest {
            PivotDestination::Existing { sheet, row, col } => (sheet, row, col),
            PivotDestination::NewSheet { index } => (self.add_sheet(index), 2, 0),
        };
        // Inserting the sheet may have moved the PivotTable's own.
        let (si, pi) = self.pivot_position(id)?;
        if sheet == si {
            return self.pivot_update(id, overwrite, |p| p.anchor = (row + above, col));
        }
        let mut p = self.sheets[si].pivots[pi].clone();
        let old: Vec<Rect> = p.footprint().collect();
        p.location = None;
        p.page_location = None;
        p.anchor = (row + above, col);
        self.pivot_install(vec![(sheet, None, p)], overwrite)?;
        self.sheets[si].pivots.remove(pi);
        for rect in old {
            clear_rect(&mut self.sheets[si], rect, &[]);
        }
        Ok(())
    }

    /// Add a field to an area, as checking or dragging it in the field
    /// list does. A date field added to the rows or columns is grouped by
    /// Excel's automatic periods: the new fields ("Years (Date)", ...) go
    /// in, collapsed, with the field itself below them.
    pub fn pivot_add_field(
        &mut self,
        id: u32,
        field: usize,
        area: Area,
        position: Option<usize>,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let locale = self.locale;
        let old = self
            .pivot(id)
            .ok_or(PivotError::NoSuchPivot)?
            .1
            .cache
            .clone();
        let periods = match area {
            Area::Rows | Area::Columns => old.auto_date_periods(field),
            _ => Vec::new(),
        };
        if periods.is_empty() {
            return self.pivot_update(id, overwrite, |p| {
                p.add_field(field, area, position, locale)
            });
        }
        let (cache, created) = old.with_date_groups(field, &periods, None, None, 1, locale)?;
        let base = cache
            .field_index(&old.fields[field].name)
            .ok_or(PivotError::NoSuchField)?;
        let mut batch = self.pivot_rebound(&old, &Arc::new(cache));
        for (_, _, q) in batch.iter_mut().filter(|(_, _, q)| q.id == id) {
            let at = position.unwrap_or(match area {
                Area::Rows => q.row_fields.len(),
                _ => q.col_fields.len(),
            });
            for (k, f) in created.iter().enumerate() {
                q.add_field(*f, area, Some(at + k), locale);
                q.set_field_collapsed(*f, true);
            }
            q.add_field(base, area, Some(at + created.len()), locale);
        }
        self.pivot_install(batch, overwrite)
    }

    /// Group a date field by periods (the Grouping dialog): the periods
    /// become fields that replace it in its area. `None` bounds follow the
    /// data; `days` is the "Number of days" of a grouping by days.
    #[allow(clippy::too_many_arguments)]
    pub fn pivot_group_dates(
        &mut self,
        id: u32,
        field: usize,
        periods: &[GroupBy],
        start: Option<f64>,
        end: Option<f64>,
        days: u32,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let locale = self.locale;
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let old = p.cache.clone();
        // Grouping one of the period fields regroups the dates themselves.
        let base = match old.fields.get(field).and_then(|f| f.group.as_ref()) {
            Some(FieldGroup {
                base: Some(b),
                kind: GroupKind::Range { .. },
                ..
            }) => *b,
            _ => field,
        };
        let (cache, created) = old.with_date_groups(base, periods, start, end, days, locale)?;
        let mut replaced = vec![base];
        replaced.extend(old.group_children(base));
        let new_base = cache
            .field_index(&old.fields[base].name)
            .ok_or(PivotError::NoSuchField)?;
        let mut batch = self.pivot_rebound(&old, &Arc::new(cache));
        for (si, pi, q) in &mut batch {
            let Some(before) = pi.map(|pi| &self.sheets[*si].pivots[pi]) else {
                continue;
            };
            if replaced.iter().any(|f| before.axis_of(*f).is_some()) {
                replace_in_area(before, q, &replaced, &created, locale);
                q.remove_field(new_base);
            }
        }
        self.pivot_install(batch, overwrite)
    }

    /// Group a number field by interval ("Starting at", "Ending at", "By").
    pub fn pivot_group_numbers(
        &mut self,
        id: u32,
        field: usize,
        start: Option<f64>,
        end: Option<f64>,
        interval: f64,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let old = p.cache.clone();
        let cache = old.with_number_groups(field, start, end, interval, self.locale)?;
        let batch = self.pivot_rebound(&old, &Arc::new(cache));
        self.pivot_install(batch, overwrite)
    }

    /// "Group Selection": put items of a field together as "Group1". The
    /// groups are the items of a new field ("Region2") placed before the
    /// grouped one; its index is returned.
    pub fn pivot_group_items(
        &mut self,
        id: u32,
        field: usize,
        items: &[u32],
        overwrite: bool,
    ) -> Result<usize, PivotError> {
        let locale = self.locale;
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let old = p.cache.clone();
        let (cache, parent) = old.with_item_group(field, items, locale)?;
        let mut batch = self.pivot_rebound(&old, &Arc::new(cache));
        for (_, _, q) in batch.iter_mut().filter(|(_, _, q)| q.id == id) {
            if q.axis_of(parent).is_some() {
                continue;
            }
            let at = match q.axis_of(field) {
                Some(Area::Rows) => q
                    .row_fields
                    .iter()
                    .position(|a| *a == AxisField::Field(field)),
                Some(Area::Columns) => q
                    .col_fields
                    .iter()
                    .position(|a| *a == AxisField::Field(field)),
                _ => None,
            };
            if let (Some(area), Some(at)) = (q.axis_of(field), at) {
                q.add_field(parent, area, Some(at), locale);
            }
        }
        self.pivot_install(batch, overwrite)?;
        Ok(parent)
    }

    /// "Ungroup". For hand-made groups, `item` is the group to dissolve
    /// (all of them when `None`); date periods and number intervals go as
    /// a whole, and the date field takes back the place of its periods.
    pub fn pivot_ungroup(
        &mut self,
        id: u32,
        field: usize,
        item: Option<u32>,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let locale = self.locale;
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let old = p.cache.clone();
        let cache = old.without_group(field, item)?;
        let by_range = |f: usize| {
            matches!(
                old.fields[f].group,
                Some(FieldGroup {
                    kind: GroupKind::Range { .. },
                    ..
                })
            )
        };
        let base = match old.fields.get(field).and_then(|f| f.group.as_ref()) {
            Some(FieldGroup { base: Some(b), .. }) if by_range(field) => Some(*b),
            None if old.group_children(field).into_iter().any(by_range) => Some(field),
            _ => None,
        };
        let mut batch = self.pivot_rebound(&old, &Arc::new(cache));
        if let Some(base) = base {
            let periods: Vec<usize> = old
                .group_children(base)
                .into_iter()
                .filter(|f| by_range(*f))
                .collect();
            for (si, pi, q) in &mut batch {
                let Some(before) = pi.map(|pi| &self.sheets[*si].pivots[pi]) else {
                    continue;
                };
                if let Some(new_base) = q.cache.field_index(&old.fields[base].name)
                    && before.axis_of(base).is_none()
                {
                    replace_in_area(before, q, &periods, &[new_base], locale);
                }
            }
        }
        self.pivot_install(batch, overwrite)
    }

    /// "Show Details" (double-click on a value): a new sheet, inserted
    /// before the PivotTable's, lists the source records behind the value.
    /// Returns the sheet and the range written, header row included.
    pub fn pivot_show_details(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
    ) -> Result<(usize, Rect), PivotError> {
        let p = self
            .pivot_at(sheet, row, col)
            .ok_or(PivotError::NotAValue)?;
        let body = p
            .location
            .filter(|b| b.contains(row, col))
            .ok_or(PivotError::NotAValue)?;
        let (r, c) = (row - body.r0, col - body.c0);
        let report = p.report.clone();
        if report.tag(r, c) != Tag::Value {
            return Err(PivotError::NotAValue);
        }
        let under = |tree: &Tree, target: u32| {
            // Parents come before their children in the list of nodes.
            let mut inside = vec![false; tree.nodes.len()];
            for (n, node) in tree.nodes.iter().enumerate() {
                inside[n] =
                    n as u32 == target || (node.parent != NONE && inside[node.parent as usize]);
            }
            inside
        };
        let rows_in = under(&report.row_tree, report.row_lines[r as usize].node);
        let cols_in = under(&report.col_tree, report.col_lines[c as usize].node);
        let records: Vec<usize> = (0..report.rec_row.len())
            .filter(|&k| {
                let (a, b) = (report.rec_row[k], report.rec_col[k]);
                a != NONE && b != NONE && rows_in[a as usize] && cols_in[b as usize]
            })
            .collect();
        let cache = p.cache.clone();
        let target = self.add_sheet(sheet);
        let mut c = 0u32;
        for f in cache.fields.iter().filter(|f| f.is_database()) {
            let style = if f.num_fmt.eq_ignore_ascii_case("General") {
                StyleId(0)
            } else {
                self.styles.intern(crate::Style {
                    num_fmt: f.num_fmt.clone(),
                    ..crate::Style::default()
                })
            };
            let sh = &mut self.sheets[target];
            put(sh, 0, c, Value::text(&f.name), StyleId(0));
            for (k, rec) in records.iter().enumerate() {
                let v = f
                    .records
                    .get(*rec)
                    .and_then(|i| f.shared.get(*i as usize))
                    .cloned()
                    .unwrap_or_default();
                let style = if v.is_number() { style } else { StyleId(0) };
                put(sh, k as u32 + 1, c, v, style);
            }
            c += 1;
        }
        Ok((target, Rect::new(0, 0, records.len() as u32, c.max(1) - 1)))
    }

    /// "Show Report Filter Pages": one new sheet per visible item of a
    /// filter field, named after the item, each with a copy of the
    /// PivotTable filtered on that item. Returns the new sheets.
    pub fn pivot_filter_pages(&mut self, id: u32, field: usize) -> Result<Vec<usize>, PivotError> {
        let locale = self.locale;
        let (si, pi) = self.pivot_position(id)?;
        let p = self.sheets[si].pivots[pi].clone();
        if !p.page_fields.iter().any(|pf| pf.field == field) {
            return Err(PivotError::NoSuchField);
        }
        let items: Vec<u32> = p.fields[field]
            .items
            .iter()
            .filter(|it| !it.hidden)
            .map(|it| it.x)
            .collect();
        let mut next_id = self.pivots().map(|(_, p)| p.id).max().unwrap_or(0);
        let mut sheets = Vec::new();
        for (k, x) in items.into_iter().enumerate() {
            let target = self.add_sheet(si + k);
            let label = p.item_text(field, x, locale);
            // A label that cannot name a sheet leaves the default name.
            self.rename_sheet(target, &label);
            let mut copy = p.clone();
            next_id += 1;
            copy.id = next_id;
            copy.location = None;
            copy.page_location = None;
            copy.set_page_item(field, Some(x));
            self.pivot_install(vec![(target, None, copy)], true)?;
            sheets.push(target);
        }
        Ok(sheets)
    }
}
