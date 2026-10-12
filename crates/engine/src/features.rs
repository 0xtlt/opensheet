//! Glue between the workbook core and the features that live in their own
//! modules (filters, tables, conditional formats, validation, comments,
//! protection, page setup, pivot tables, drawings).
//!
//! The core calls the hooks below; each one forwards to a function of the
//! same name in every feature module, so a feature only ever edits its own
//! file.

use std::borrow::Cow;
use std::sync::Arc;

use crate::ast::{Expr, RefExpr, RefKind, StructuralEdit, same_name};
use crate::condfmt::CondFormat;
use crate::ops::{Clip, FindOptions, PasteMode, shift_interval};
use crate::protect::{Action, Denied};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Style, StylePatch};
use crate::table::Table;
use crate::validation::Validation;
use crate::value::ErrorKind;
use crate::workbook::{ClearMode, Sheet, Workbook};
use crate::{comment, condfmt, drawing, filter, hyperlink, page, pivot, protect, table, validation};

/// Per-feature caches of things derived from the cells. Compare against
/// `Workbook::generation` to know when they are stale. The workbook is
/// shared with the recalculation thread, so these must be `Sync`.
#[derive(Default)]
pub struct Caches {
    pub cond: condfmt::Cache,
    pub filter: filter::Cache,
    pub pivot: pivot::Cache,
    pub table: table::Cache,
    pub validation: validation::Cache,
}

/// Rows or columns were inserted or deleted on the sheet named
/// `edited_sheet`. Called for every sheet of the workbook (`sheet` is the
/// edited one when `sheet.name == edited_sheet`), after cells and formulas
/// were moved.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    filter::on_structural_edit(sheet, edit, edited_sheet);
    table::on_structural_edit(sheet, edit, edited_sheet);
    condfmt::on_structural_edit(sheet, edit, edited_sheet);
    validation::on_structural_edit(sheet, edit, edited_sheet);
    comment::on_structural_edit(sheet, edit, edited_sheet);
    protect::on_structural_edit(sheet, edit, edited_sheet);
    page::on_structural_edit(sheet, edit, edited_sheet);
    pivot::on_structural_edit(sheet, edit, edited_sheet);
    drawing::on_structural_edit(sheet, edit, edited_sheet);
    hyperlink::on_structural_edit(sheet, edit, edited_sheet);
    crate::consolidate::on_structural_edit(sheet, edit, edited_sheet);
    crate::whatif::on_structural_edit(sheet, edit, edited_sheet);
    crate::text_columns::on_structural_edit(sheet, edit, edited_sheet);
    crate::slicer::on_structural_edit(sheet, edit, edited_sheet);
}

/// A sheet was renamed from `old` to `new`. Called for every sheet (the
/// renamed one already carries its new name).
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    filter::on_sheet_renamed(sheet, old, new);
    table::on_sheet_renamed(sheet, old, new);
    condfmt::on_sheet_renamed(sheet, old, new);
    validation::on_sheet_renamed(sheet, old, new);
    comment::on_sheet_renamed(sheet, old, new);
    protect::on_sheet_renamed(sheet, old, new);
    page::on_sheet_renamed(sheet, old, new);
    pivot::on_sheet_renamed(sheet, old, new);
    drawing::on_sheet_renamed(sheet, old, new);
    hyperlink::on_sheet_renamed(sheet, old, new);
    crate::consolidate::on_sheet_renamed(sheet, old, new);
}

/// `wb.sheets[copy]` was just inserted as a copy of the sheet named
/// `original`: whatever must be unique in the workbook (table names, ids)
/// or must follow the copy (references to the original sheet) is fixed here.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    filter::on_sheet_duplicated(wb, copy, original);
    table::on_sheet_duplicated(wb, copy, original);
    condfmt::on_sheet_duplicated(wb, copy, original);
    validation::on_sheet_duplicated(wb, copy, original);
    comment::on_sheet_duplicated(wb, copy, original);
    protect::on_sheet_duplicated(wb, copy, original);
    page::on_sheet_duplicated(wb, copy, original);
    pivot::on_sheet_duplicated(wb, copy, original);
    drawing::on_sheet_duplicated(wb, copy, original);
    hyperlink::on_sheet_duplicated(wb, copy, original);
    crate::slicer::on_sheet_duplicated(wb, copy, original);
}

/// The sheet `deleted` was just taken out of the workbook; `order` lists
/// the sheet names in tab order as they were before. The formulas of the
/// cells and of the defined names already read `#REF!` where they named
/// the sheet: here every feature does the same for what it keeps.
pub(crate) fn on_sheet_deleted(wb: &mut Workbook, deleted: &Sheet, order: &[&str]) {
    filter::on_sheet_deleted(wb, deleted, order);
    table::on_sheet_deleted(wb, deleted, order);
    condfmt::on_sheet_deleted(wb, deleted, order);
    validation::on_sheet_deleted(wb, deleted, order);
    comment::on_sheet_deleted(wb, deleted, order);
    protect::on_sheet_deleted(wb, deleted, order);
    page::on_sheet_deleted(wb, deleted, order);
    pivot::on_sheet_deleted(wb, deleted, order);
    drawing::on_sheet_deleted(wb, deleted, order);
    hyperlink::on_sheet_deleted(wb, deleted, order);
}

/// The sheet named `moved` changed place among the tabs; `before` and
/// `after` list the sheet names in tab order around the move. Whatever is
/// kept by position of the sheet, or reads a run of sheets (3-D
/// references), follows here.
pub(crate) fn on_sheet_moved(wb: &mut Workbook, moved: &str, before: &[&str], after: &[&str]) {
    filter::on_sheet_moved(wb, moved, before, after);
    table::on_sheet_moved(wb, moved, before, after);
    condfmt::on_sheet_moved(wb, moved, before, after);
    validation::on_sheet_moved(wb, moved, before, after);
    comment::on_sheet_moved(wb, moved, before, after);
    protect::on_sheet_moved(wb, moved, before, after);
    page::on_sheet_moved(wb, moved, before, after);
    pivot::on_sheet_moved(wb, moved, before, after);
    drawing::on_sheet_moved(wb, moved, before, after);
    hyperlink::on_sheet_moved(wb, moved, before, after);
}

/// Cells of `wb.sheets[sheet]` were shifted inside a band (see
/// `CellShift`), after the cells and their formulas were moved: what sits
/// on the shifted cells follows them.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    filter::on_cells_shifted(wb, sheet, shift);
    table::on_cells_shifted(wb, sheet, shift);
    condfmt::on_cells_shifted(wb, sheet, shift);
    validation::on_cells_shifted(wb, sheet, shift);
    comment::on_cells_shifted(wb, sheet, shift);
    protect::on_cells_shifted(wb, sheet, shift);
    page::on_cells_shifted(wb, sheet, shift);
    pivot::on_cells_shifted(wb, sheet, shift);
    drawing::on_cells_shifted(wb, sheet, shift);
    hyperlink::on_cells_shifted(wb, sheet, shift);
}

/// Excel's "shift cells": `count` cells are inserted at `at` (or deleted
/// from there) in every line of a band, and the cells beyond move away (or
/// close up). This is how table rows and columns come and go without
/// touching what lies beside the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CellShift {
    /// Cells move down / up inside columns `lo..=hi`; otherwise right /
    /// left inside rows `lo..=hi`.
    pub down: bool,
    pub lo: u32,
    pub hi: u32,
    pub at: u32,
    pub count: u32,
    pub insert: bool,
}

impl CellShift {
    /// How many rows (columns) the sheet has along the direction of the
    /// move.
    pub(crate) fn limit(&self) -> u32 {
        if self.down { MAX_ROWS } else { MAX_COLS }
    }

    /// The band between two positions along the direction of the move.
    fn band(&self, from: u32, to: u32) -> Rect {
        if self.down {
            Rect::new(from, self.lo, to, self.hi)
        } else {
            Rect::new(self.lo, from, self.hi, to)
        }
    }

    /// The cells that move or go: the band from `at` to the sheet's edge.
    pub(crate) fn area(&self) -> Rect {
        self.band(self.at, self.limit() - 1)
    }

    /// Where a position at or after `at` ends up; `None` when its cell is
    /// deleted or pushed off the sheet.
    pub(crate) fn moved(&self, pos: u32) -> Option<u32> {
        if self.insert {
            Some(pos + self.count).filter(|p| *p < self.limit())
        } else {
            (pos >= self.at + self.count).then(|| pos - self.count)
        }
    }

    /// The cells that are still on the sheet afterwards, where they were
    /// before the move and where they are after it; `None` when none is.
    pub(crate) fn carried(&self) -> Option<(Rect, Rect)> {
        let last = self.limit() - 1;
        let first = if self.insert { self.at } else { self.at + self.count };
        let end = if self.insert { last.checked_sub(self.count)? } else { last };
        (first <= end).then(|| {
            let landing = self.moved(first).unwrap_or(first);
            (self.band(first, end), self.band(landing, landing + (end - first)))
        })
    }

    /// The cells that go: those deleted, or those pushed off the sheet.
    pub(crate) fn removed(&self) -> Rect {
        let last = self.limit() - 1;
        if self.insert {
            self.band(last.saturating_sub(self.count - 1).max(self.at), last)
        } else {
            self.band(self.at, (self.at + self.count - 1).min(last))
        }
    }

    /// The blank cells an insertion makes, and the cells just before them
    /// (`None` at the edge of the sheet), from which they take what
    /// inserted cells inherit: formats, rules.
    pub(crate) fn inserted(&self) -> Option<(Rect, Option<Rect>)> {
        let last = (self.at + self.count - 1).min(self.limit() - 1);
        let before = self.at.checked_sub(1).map(|line| self.band(line, line));
        self.insert.then(|| (self.band(self.at, last), before))
    }

    /// A reference after the move (`edited` is the sheet the cells moved
    /// on, `own` the sheet holding the formula): those to cells inside
    /// the band follow them, the others stay.
    pub(crate) fn fix(&self, r: &RefExpr, edited: &str, own: &str) -> Expr {
        let target = r.sheet.as_deref().unwrap_or(own);
        if !same_name(target, edited) || r.kind != RefKind::Cells {
            return Expr::Ref(r.clone());
        }
        let end = r.end.unwrap_or(r.start);
        let (across, along) = if self.down {
            ((r.start.col, end.col), (r.start.row, end.row))
        } else {
            ((r.start.row, end.row), (r.start.col, end.col))
        };
        if across.0.min(across.1) < self.lo || across.0.max(across.1) > self.hi {
            return Expr::Ref(r.clone());
        }
        let (lo, hi) = (along.0.min(along.1), along.0.max(along.1));
        let Some((lo, hi)) = shift_interval(lo, hi, self.insert, self.at, self.count, self.limit()) else {
            // The sheet is still there, its cells are not: `Sheet2!#REF!`.
            return match &r.sheet {
                Some(sheet) => Expr::RefError(sheet.clone()),
                None => Expr::Error(ErrorKind::Ref),
            };
        };
        let (start, end) = if along.0 > along.1 { (hi, lo) } else { (lo, hi) };
        let mut out = r.clone();
        if self.down {
            out.start.row = start;
            if let Some(e) = &mut out.end {
                e.row = end;
            }
        } else {
            out.start.col = start;
            if let Some(e) = &mut out.end {
                e.col = end;
            }
        }
        Expr::Ref(out)
    }

    /// As `fix`, for the formula of a rule (conditional format, data
    /// validation). Its relative references belong to whichever cell
    /// reads the rule: only fixed ones name the cells that moved.
    pub(crate) fn fix_rule(&self, e: &Expr, edited: &str, own: &str) -> Expr {
        e.map_refs(&mut |r| {
            let end = r.end.unwrap_or(r.start);
            if r.start.abs_row && r.start.abs_col && end.abs_row && end.abs_col {
                self.fix(r, edited, own)
            } else {
                Expr::Ref(r.clone())
            }
        })
    }
}

// ---- copies ---------------------------------------------------------------------

/// The copies of `part`, a piece of `src`, when `src` is laid over `dest`:
/// `dest` holds a whole number of copies of `src` side by side and one
/// under the other (one cell pasted over a selection, a row filled down),
/// or a single copy cut short by its edges. A piece as tall or as wide as
/// `src` comes out as one range across the copies.
pub(crate) fn tiled(part: Rect, src: &Rect, dest: &Rect) -> Vec<Rect> {
    // (start, length) of the piece in each copy along one axis.
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
    let rows = spans(part.r0, part.r1, src.r0, src.r1, dest.r0, dest.r1);
    let cols = spans(part.c0, part.c1, src.c0, src.c1, dest.c0, dest.c1);
    let mut out = Vec::with_capacity(rows.len() * cols.len());
    for &(r, height) in &rows {
        for &(c, width) in &cols {
            out.push(Rect::new(r, c, r + height - 1, c + width - 1));
        }
    }
    out
}

/// `ranges` without the cells of `bands`: blocks of rows over the same
/// columns, apart from each other and listed top to bottom, as a filter
/// leaves them in a selection (`Sheet::shown_parts`). One pass over the
/// bands each range meets, however many there are.
pub(crate) fn without_bands(ranges: &[Rect], bands: &[Rect]) -> Vec<Rect> {
    let Some((c0, c1)) = bands.first().map(|b| (b.c0, b.c1)) else {
        return ranges.to_vec();
    };
    let mut out = Vec::with_capacity(ranges.len());
    for r in ranges {
        let start = bands.partition_point(|b| b.r1 < r.r0);
        let met = &bands[start..];
        if r.c1 < c0 || r.c0 > c1 || met.first().is_none_or(|b| b.r0 > r.r1) {
            out.push(*r);
            continue;
        }
        // Beside the bands the range keeps its height; between them it
        // keeps the rows of no band.
        if r.c0 < c0 {
            out.push(Rect::new(r.r0, r.c0, r.r1, c0 - 1));
        }
        if r.c1 > c1 {
            out.push(Rect::new(r.r0, c1 + 1, r.r1, r.c1));
        }
        let (left, right) = (r.c0.max(c0), r.c1.min(c1));
        let mut row = r.r0;
        for band in met.iter().take_while(|b| b.r0 <= r.r1) {
            if band.r0 > row {
                out.push(Rect::new(row, left, band.r0 - 1, right));
            }
            row = band.r1 + 1;
        }
        if row <= r.r1 {
            out.push(Rect::new(row, left, r.r1, right));
        }
    }
    out
}

/// A list of ranges longer than this is tidied by `join_sorted` alone:
/// the full tidying the rules have (`condfmt::normalize_ranges`, the
/// validation rules' own) compares every pair of ranges again and again.
pub(crate) const TIDY_LIMIT: usize = 64;

/// Join the ranges that lie one under the other over the same columns, or
/// side by side over the same rows, without changing the cells covered.
/// It takes sorts, where comparing every pair would not do for the
/// thousands of pieces the rows of a filtered list make. The list comes
/// out in reading order, whatever order it had.
pub(crate) fn join_sorted(mut ranges: Vec<Rect>) -> Vec<Rect> {
    // Lines `a` and columns `b`, or the other way round.
    fn pass(ranges: &mut Vec<Rect>, key: fn(&Rect) -> (u32, u32, u32, u32), back: fn((u32, u32, u32, u32)) -> Rect) {
        ranges.sort_by_key(key);
        let mut out: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(ranges.len());
        for (b0, b1, a0, a1) in ranges.iter().map(key) {
            match out.last_mut() {
                Some(last) if (last.0, last.1) == (b0, b1) && a0 <= last.3.saturating_add(1) => {
                    last.3 = last.3.max(a1);
                }
                _ => out.push((b0, b1, a0, a1)),
            }
        }
        *ranges = out.into_iter().map(back).collect();
    }
    loop {
        let before = ranges.len();
        pass(&mut ranges, |r| (r.c0, r.c1, r.r0, r.r1), |(c0, c1, r0, r1)| Rect::new(r0, c0, r1, c1));
        pass(&mut ranges, |r| (r.r0, r.r1, r.c0, r.c1), |(r0, r1, c0, c1)| Rect::new(r0, c0, r1, c1));
        if ranges.len() == before {
            ranges.sort_by_key(|r| (r.r0, r.c0));
            return ranges;
        }
    }
}

// ---- formulas kept outside the cells ------------------------------------------

/// Replace by `f(formula)` every formula the features keep outside the
/// cells: those of conditional formats and of validation rules, and the
/// totals formulas tables remember while their totals row is hidden.
fn map_kept(
    cond_formats: &mut [CondFormat],
    validations: &mut [Validation],
    tables: &mut [Table],
    f: &mut dyn FnMut(&Expr) -> Expr,
) {
    for cf in cond_formats {
        cf.rule.map_exprs(f);
    }
    for rule in validations {
        for formula in [&mut rule.formula1, &mut rule.formula2].into_iter().flatten() {
            let mapped = f(formula);
            if mapped != **formula {
                *formula = Arc::new(mapped);
            }
        }
    }
    for table in tables {
        table.map_formulas(f);
    }
}

/// See `map_kept`: the formulas kept by the features of one sheet. The
/// caller has taken the undo snapshot, and refreshes the cache of the
/// conditional formats.
pub(crate) fn map_formulas(sheet: &mut Sheet, f: &mut dyn FnMut(&Expr) -> Expr) {
    map_kept(&mut sheet.cond_formats, &mut sheet.validations, &mut sheet.tables, f);
}

/// The formulas kept by the features of a sheet after a rewrite that was
/// worked out while the workbook could only be read (see
/// `Workbook::rewrite_formulas`).
pub(crate) struct Rewritten {
    cond_formats: Vec<CondFormat>,
    validations: Vec<Validation>,
    tables: Vec<Table>,
}

impl Rewritten {
    /// What `f` makes of the formulas kept by the features of `sheet`
    /// (`None` leaves a formula alone); `None` when nothing changes.
    pub(crate) fn of(sheet: &Sheet, f: &mut dyn FnMut(&Expr) -> Option<Expr>) -> Option<Rewritten> {
        if sheet.cond_formats.is_empty() && sheet.validations.is_empty() && sheet.tables.is_empty() {
            return None;
        }
        let mut out = Rewritten {
            cond_formats: sheet.cond_formats.clone(),
            validations: sheet.validations.clone(),
            tables: sheet.tables.clone(),
        };
        let mut changed = false;
        map_kept(&mut out.cond_formats, &mut out.validations, &mut out.tables, &mut |e| match f(e) {
            Some(new) if new != *e => {
                changed = true;
                new
            }
            _ => e.clone(),
        });
        changed.then_some(out)
    }

    /// Put the rewritten formulas in place.
    pub(crate) fn store(self, sheet: &mut Sheet) {
        sheet.cond_formats = self.cond_formats;
        sheet.validations = self.validations;
        sheet.tables = self.tables;
    }
}

// ---- protection -------------------------------------------------------------------

/// A change the user asks for, named after the editing function that
/// carries it out. Those functions never ask whether the change is
/// allowed: the app asks `Workbook::permits_edit` first, and shows
/// `Denied::message` instead of making the change when it is not.
///
/// What is not listed here is an `Action` as it stands, to ask `permits`
/// about: filters (`ToggleAutoFilter`, `UseAutoFilter`), tables
/// (`ChangeTable`), notes and comments (`EditComment`), charts, pictures
/// and shapes (`EditObject`), conditional formats, data validation and
/// the tab colour (`ChangeSheet`), and whole sheets (`InsertSheet`...).
#[derive(Clone, Copy, Debug)]
pub enum Edit<'a> {
    /// Typing in cells: `set_cell_text`, `set_value`, `paste_text`.
    Enter { sheet: usize, ranges: &'a [Rect] },
    /// `clear`, with the choice of the Clear menu.
    Clear {
        sheet: usize,
        ranges: &'a [Rect],
        mode: ClearMode,
    },
    /// `paste` with the same arguments (the format painter is a paste of
    /// `PasteMode::Formats`).
    Paste {
        sheet: usize,
        dest: Rect,
        clip: &'a Clip,
        mode: PasteMode,
    },
    /// `move_range` with the same arguments: cut and paste, dragging a
    /// selection.
    Move {
        src_sheet: usize,
        src: Rect,
        dest_sheet: usize,
        dest_row: u32,
        dest_col: u32,
    },
    /// `fill_down`, `fill_right` and `autofill`, over the cells filled.
    Fill { sheet: usize, dest: Rect },
    /// `update_style` and `apply_borders`: fonts, fills, borders, number
    /// formats, alignment. (Locking or unlocking cells is
    /// `Action::ChangeSheet`.)
    Format { sheet: usize },
    /// `merge` and `unmerge`.
    Merge { sheet: usize },
    /// `insert_rows`.
    InsertRows { sheet: usize },
    /// `insert_cols`.
    InsertCols { sheet: usize },
    /// `delete_rows` with the same arguments.
    DeleteRows { sheet: usize, at: u32, count: u32 },
    /// `delete_cols` with the same arguments.
    DeleteCols { sheet: usize, at: u32, count: u32 },
    /// `set_row_height` and `set_rows_hidden`.
    SizeRows { sheet: usize },
    /// `set_col_width` and `set_cols_hidden`.
    SizeCols { sheet: usize },
    /// `sort_range` and `sort_rows`; with `Action::UseAutoFilter`,
    /// `sort_by_filter` and `sort_filter` too.
    Sort { sheet: usize, range: Rect },
    /// `remove_duplicates`.
    RemoveDuplicates { sheet: usize },
    /// `replace_all` with the same arguments.
    Replace {
        sheet: usize,
        needle: &'a str,
        options: &'a FindOptions,
    },
}

impl Workbook {
    /// May this edit proceed? The answer `permits` gives for what the
    /// edit does to the sheet, as Excel has it: contents go into unlocked
    /// cells only, formats need the "Format cells" permission whatever
    /// the cells, and so on. Sheets that are not protected allow
    /// everything.
    pub fn permits_edit(&self, edit: &Edit) -> Result<(), Denied> {
        match *edit {
            Edit::Enter { sheet, ranges } => self.permits(&Action::EditCells { sheet, ranges }),
            Edit::Clear { sheet, ranges, mode } => match mode {
                ClearMode::Contents => self.permits(&Action::EditCells { sheet, ranges }),
                ClearMode::Formats => self.permits(&Action::FormatCells { sheet }),
                ClearMode::Comments => self.permits(&Action::EditComment { sheet }),
                ClearMode::All => {
                    self.permits(&Action::EditCells { sheet, ranges })?;
                    self.permits(&Action::FormatCells { sheet })
                }
            },
            Edit::Paste { sheet, dest, clip, mode } => match mode {
                PasteMode::Formats => self.permits(&Action::FormatCells { sheet }),
                PasteMode::Comments => self.permits(&Action::EditComment { sheet }),
                PasteMode::Validation => self.permits(&Action::ChangeSheet { sheet }),
                PasteMode::ColumnWidths => self.permits(&Action::FormatColumns { sheet }),
                // A paste brings its formats into unlocked cells without
                // asking for more, as in Excel.
                PasteMode::All
                | PasteMode::AllExceptBorders
                | PasteMode::Formulas
                | PasteMode::Values
                | PasteMode::FormulasAndNumberFormats
                | PasteMode::ValuesAndNumberFormats => {
                    let ranges = self.paste_blocks(sheet, dest, clip);
                    self.permits(&Action::EditCells { sheet, ranges: &ranges })
                }
            },
            Edit::Move {
                src_sheet,
                src,
                dest_sheet,
                dest_row,
                dest_col,
            } => {
                // Both the cells emptied and the cells written change.
                self.permits(&Action::EditCells {
                    sheet: src_sheet,
                    ranges: &[src],
                })?;
                let landed = Rect::new(
                    dest_row.min(MAX_ROWS - 1),
                    dest_col.min(MAX_COLS - 1),
                    (dest_row as u64 + src.rows() as u64 - 1).min(MAX_ROWS as u64 - 1) as u32,
                    (dest_col as u64 + src.cols() as u64 - 1).min(MAX_COLS as u64 - 1) as u32,
                );
                self.permits(&Action::EditCells {
                    sheet: dest_sheet,
                    ranges: &[landed],
                })
            }
            Edit::Fill { sheet, dest } => self.permits(&Action::EditCells { sheet, ranges: &[dest] }),
            Edit::Format { sheet } => self.permits(&Action::FormatCells { sheet }),
            Edit::Merge { sheet } | Edit::RemoveDuplicates { sheet } => self.permits(&Action::ChangeSheet { sheet }),
            Edit::InsertRows { sheet } => self.permits(&Action::InsertRows { sheet }),
            Edit::InsertCols { sheet } => self.permits(&Action::InsertColumns { sheet }),
            Edit::DeleteRows { sheet, at, count } => self.permits(&Action::DeleteRows {
                sheet,
                first: at,
                last: at.saturating_add(count.max(1) - 1).min(MAX_ROWS - 1),
            }),
            Edit::DeleteCols { sheet, at, count } => self.permits(&Action::DeleteColumns {
                sheet,
                first: at,
                last: at.saturating_add(count.max(1) - 1).min(MAX_COLS - 1),
            }),
            Edit::SizeRows { sheet } => self.permits(&Action::FormatRows { sheet }),
            Edit::SizeCols { sheet } => self.permits(&Action::FormatColumns { sheet }),
            Edit::Sort { sheet, range } => self.permits(&Action::Sort { sheet, range }),
            Edit::Replace { sheet, needle, options } => {
                if !self.is_sheet_protected(sheet) {
                    return Ok(());
                }
                // Only the cells that would change count.
                let in_formulas = FindOptions {
                    look_in: crate::find::LookIn::Formulas,
                    ..options.clone()
                };
                let hits: Vec<Rect> = self
                    .find_all(sheet, needle, &in_formulas)
                    .into_iter()
                    .map(|(row, col)| Rect::cell(row, col))
                    .collect();
                self.permits(&Action::EditCells { sheet, ranges: &hits })
            }
        }
    }
}

/// How a cell is drawn: its own formatting with the table style and the
/// conditional formats laid over it, plus what conditional formats draw
/// inside the cell (data bar, icon).
#[derive(Clone, Debug)]
pub struct CellDisplay<'a> {
    pub style: Cow<'a, Style>,
    pub adornment: Option<condfmt::Adornment>,
}

impl Workbook {
    /// The appearance of a cell on screen and on paper. Cheap when the
    /// sheet has neither tables nor conditional formats.
    pub fn display(&self, sheet: usize, row: u32, col: u32) -> CellDisplay<'_> {
        let base = self.style_at(sheet, row, col);
        let Some(sh) = self.sheets.get(sheet) else {
            return CellDisplay {
                style: Cow::Borrowed(base),
                adornment: None,
            };
        };
        let mut patch = StylePatch::default();
        let mut adornment = None;
        if !sh.tables.is_empty() {
            if let Some(p) = table::style_patch_at(self, sheet, row, col) {
                patch.merge(&p);
            }
        }
        if !sh.pivots.is_empty() {
            if let Some(p) = pivot::style_patch_at(self, sheet, row, col) {
                patch.merge(&p);
            }
        }
        // A table style only shows where the cell has no formatting of its
        // own: its fill, font colour and borders win over the table's.
        let mut style = Cow::Borrowed(base);
        if !patch.is_empty() {
            // "No formatting of its own" is measured against the workbook's
            // default style, which a file may set to another font.
            let d = self.styles.default_style();
            let mut s = d.clone();
            patch.apply(&mut s);
            let own = base;
            if own.fill != d.fill {
                s.fill = own.fill;
            }
            if own.font != d.font {
                let table_color = s.font.color;
                let table_bold = s.font.bold;
                s.font = own.font.clone();
                if own.font.color.is_none() {
                    s.font.color = table_color;
                }
                s.font.bold |= table_bold;
            }
            if !own.borders.left.is_none() {
                s.borders.left = own.borders.left;
            }
            if !own.borders.right.is_none() {
                s.borders.right = own.borders.right;
            }
            if !own.borders.top.is_none() {
                s.borders.top = own.borders.top;
            }
            if !own.borders.bottom.is_none() {
                s.borders.bottom = own.borders.bottom;
            }
            s.h_align = own.h_align;
            s.v_align = own.v_align;
            s.wrap = own.wrap;
            s.shrink_to_fit = own.shrink_to_fit;
            s.indent = own.indent;
            s.rotation = own.rotation;
            s.num_fmt = own.num_fmt.clone();
            s.locked = own.locked;
            s.hidden = own.hidden;
            style = Cow::Owned(s);
        }
        if !sh.cond_formats.is_empty() {
            if let Some(result) = condfmt::result_at(self, sheet, row, col) {
                if !result.patch.is_empty() {
                    result.patch.apply(style.to_mut());
                }
                adornment = result.adornment;
            }
        }
        CellDisplay { style, adornment }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    /// The cells of a list of ranges, and how many times ranges overlap.
    fn cells(ranges: &[Rect]) -> (BTreeSet<(u32, u32)>, usize) {
        let all = ranges
            .iter()
            .flat_map(|r| (r.r0..=r.r1).flat_map(move |row| (r.c0..=r.c1).map(move |col| (row, col))));
        let set: BTreeSet<(u32, u32)> = all.clone().collect();
        let overlaps = all.count() - set.len();
        (set, overlaps)
    }

    /// Numbers that look random and are the same at every run.
    fn dice(seed: &mut u32, below: u32) -> u32 {
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (*seed >> 16) % below
    }

    fn some_rect(seed: &mut u32) -> Rect {
        let (r0, c0) = (dice(seed, 12), dice(seed, 8));
        Rect::new(r0, c0, r0 + dice(seed, 5), c0 + dice(seed, 4))
    }

    #[test]
    fn copies_laid_over_a_destination() {
        let src = rect("B2:C3");
        // Two copies down, two across; a piece as wide as the source is
        // one range across them.
        assert_eq!(tiled(rect("B2"), &src, &rect("E5:H8")), ["E5", "G5", "E7", "G7"].map(rect));
        assert_eq!(tiled(rect("B3:C3"), &src, &rect("E5:H8")), [rect("E6:H6"), rect("E8:H8")]);
        assert_eq!(tiled(src, &src, &rect("E5:H8")), [rect("E5:H8")]);
        // One copy cut short by the destination.
        assert_eq!(tiled(rect("C2:C3"), &src, &rect("E5:E6")), []);
        assert_eq!(tiled(rect("B2:B3"), &src, &rect("E5:E5")), [rect("E5")]);
        // One row filled down.
        assert_eq!(tiled(rect("C2"), &rect("B2:C2"), &rect("B3:C9")), [rect("C3:C9")]);
    }

    #[test]
    fn ranges_without_the_blocks_a_filter_shows() {
        let bands = [rect("C2:D3"), rect("C6:D6"), rect("C9:D12")];
        assert_eq!(
            without_bands(&[rect("A1:E7")], &bands),
            [rect("A1:B7"), rect("E1:E7"), rect("C1:D1"), rect("C4:D5"), rect("C7:D7")]
        );
        // Ranges that meet no band are left whole.
        let apart = [rect("A1:B20"), rect("C4:D5"), rect("C13:F15"), rect("C7:C8")];
        assert_eq!(without_bands(&apart, &bands), apart);
        assert_eq!(without_bands(&[rect("C9:D12"), rect("D2")], &bands), []);
        assert_eq!(without_bands(&apart, &[]), apart);

        // On scattered ranges, the same cells as taking the bands out one
        // by one, and no cell twice.
        let mut seed = 7;
        for _ in 0..200 {
            let ranges: Vec<Rect> = (0..4).map(|_| some_rect(&mut seed)).collect();
            let (c0, width) = (dice(&mut seed, 6), dice(&mut seed, 4));
            let mut bands = Vec::new();
            let mut row = dice(&mut seed, 3);
            while row < 16 {
                let last = row + dice(&mut seed, 3);
                bands.push(Rect::new(row, c0, last, c0 + width));
                row = last + 2 + dice(&mut seed, 3);
            }
            let (kept, _) = cells(&without_bands(&ranges, &bands));
            let (all, _) = cells(&ranges);
            let (cut, _) = cells(&bands);
            let expected: BTreeSet<(u32, u32)> = all.difference(&cut).copied().collect();
            assert_eq!(kept, expected, "{ranges:?} without {bands:?}");
        }
    }

    #[test]
    fn ranges_joined_by_sorting() {
        // A column in pieces, then two columns side by side.
        let pieces = ["B5:B6", "B1:B2", "C1:C6", "B3:B4", "E1", "E3"].map(rect);
        assert_eq!(join_sorted(pieces.to_vec()), [rect("B1:C6"), rect("E1"), rect("E3")]);
        assert_eq!(join_sorted(vec![rect("A1:B2"), rect("A2:B5")]), [rect("A1:B5")]);
        assert_eq!(join_sorted(Vec::new()), []);
        // The cells stay the same, and a list that did not overlap still
        // does not.
        let mut seed = 99;
        for _ in 0..200 {
            let mut ranges: Vec<Rect> = Vec::new();
            for _ in 0..6 {
                let next = some_rect(&mut seed);
                if !ranges.iter().any(|r| r.intersects(&next)) {
                    ranges.push(next);
                }
            }
            let joined = join_sorted(ranges.clone());
            assert_eq!(cells(&joined), (cells(&ranges).0, 0), "{ranges:?}");
            assert!(joined.len() <= ranges.len());
        }
    }

    #[test]
    fn cells_shifted_in_a_band() {
        let down = CellShift {
            down: true,
            lo: 1,
            hi: 2,
            at: 4,
            count: 2,
            insert: true,
        };
        assert_eq!(down.area(), Rect::new(4, 1, MAX_ROWS - 1, 2));
        assert_eq!(down.carried(), Some((Rect::new(4, 1, MAX_ROWS - 3, 2), Rect::new(6, 1, MAX_ROWS - 1, 2))));
        assert_eq!(down.inserted(), Some((rect("B5:C6"), Some(rect("B4:C4")))));
        assert_eq!(down.removed(), Rect::new(MAX_ROWS - 2, 1, MAX_ROWS - 1, 2));
        let up = CellShift { insert: false, ..down };
        assert_eq!(up.carried(), Some((Rect::new(6, 1, MAX_ROWS - 1, 2), Rect::new(4, 1, MAX_ROWS - 3, 2))));
        assert_eq!((up.inserted(), up.removed()), (None, rect("B5:C6")));
        let left = CellShift {
            down: false,
            lo: 0,
            hi: 3,
            at: 0,
            count: 1,
            insert: false,
        };
        assert_eq!(left.carried(), Some((Rect::new(0, 1, 3, MAX_COLS - 1), Rect::new(0, 0, 3, MAX_COLS - 2))));
        let right = CellShift { insert: true, ..left };
        assert_eq!(right.inserted(), Some((rect("A1:A4"), None)));

        // References into the band follow the cells; the others stay.
        let fixed = |shift: &CellShift, text: &str| {
            let formula = crate::parser::parse_formula(text, crate::locale::Locale::en()).unwrap();
            let moved = formula.map_refs(&mut |r| shift.fix(r, "Sheet1", "Sheet1"));
            moved.to_text(crate::locale::Locale::en())
        };
        assert_eq!(fixed(&down, "B5+B4+A5+D9+SUM(B2:C8)+SUM(A5:C5)"), "B7+B4+A5+D9+SUM(B2:C10)+SUM(A5:C5)");
        assert_eq!(fixed(&up, "B7+B5+SUM(B5:C6)+Sheet1!C6+Sheet2!B7"), "B5+#REF!+SUM(#REF!)+Sheet1!#REF!+Sheet2!B7");
        // In a rule only the fixed references name cells.
        let rule = crate::parser::parse_formula("$B$5+B5+$B5", crate::locale::Locale::en()).unwrap();
        let moved = down.fix_rule(&rule, "Sheet1", "Sheet1");
        assert_eq!(moved.to_text(crate::locale::Locale::en()), "$B$7+B5+$B5");
    }
}
