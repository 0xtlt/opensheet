//! Workbook model: sheets, cells, styles, undo history, recalculation.

use crate::ast::Expr;
use crate::eval::Calc;
use crate::format::{self, Formatted};
use crate::input;
use crate::locale::Locale;
use crate::parser::ParseError;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Color, Style, StyleId, StyleTable};
use crate::value::{Array, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// Default column width in pixels at 100 % zoom, in a new workbook. A
/// sheet read from a file may have another: `Sheet::default_col_width`.
pub const DEFAULT_COL_WIDTH: f32 = 64.0;
/// Default row height in pixels at 100 % zoom, in a new workbook. A sheet
/// read from a file may have another: `Sheet::default_row_height`.
pub const DEFAULT_ROW_HEIGHT: f32 = 20.0;

const MAX_UNDO: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// No content (the cell exists only to carry formatting).
    Empty,
    /// A constant typed by the user.
    Value(Value),
    Formula(Arc<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub content: Content,
    /// Cached result (equal to the constant for non-formula cells; for a
    /// spilling formula, the top-left element).
    pub value: Value,
    pub style: StyleId,
}

/// A dynamic array result occupying a block of cells.
#[derive(Clone, Debug)]
pub struct Spill {
    pub row: u32,
    pub col: u32,
    pub array: Arc<Array>,
}

#[derive(Clone, Debug)]
pub struct Sheet {
    pub name: String,
    pub cells: BTreeMap<(u32, u32), Cell>,
    /// Explicit column widths (pixels at 100 %).
    pub col_widths: BTreeMap<u32, f32>,
    /// Explicit row heights (pixels at 100 %).
    pub row_heights: BTreeMap<u32, f32>,
    /// Width of the columns that have none of their own and height of the
    /// rows that have none (pixels at 100 %). They follow from the default
    /// font of the workbook, or were set for the sheet ("Default Width").
    pub default_col_width: f32,
    pub default_row_height: f32,
    pub hidden_cols: BTreeSet<u32>,
    pub hidden_rows: BTreeSet<u32>,
    /// Formatting applied to whole columns / rows (inherited by new cells).
    pub col_styles: BTreeMap<u32, StyleId>,
    pub row_styles: BTreeMap<u32, StyleId>,
    pub merges: Vec<Rect>,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    pub show_gridlines: bool,
    pub show_headers: bool,
    pub tab_color: Option<Color>,
    /// Dynamic array results of the last recalculation.
    pub spills: Vec<Spill>,
    /// Charts floating over the cells, in drawing order (last on top).
    pub charts: Vec<crate::chart::Chart>,
    /// The sheet's filter buttons (Data > Filter), see `filter.rs`.
    pub auto_filter: Option<crate::filter::AutoFilter>,
    /// Rows hidden by a filter (of the sheet or of a table), kept apart
    /// from the rows the user hid by hand.
    pub filtered_rows: BTreeSet<u32>,
    /// The Advanced Filter applied in place, see `filter/advanced.rs`.
    pub advanced_filter: Option<crate::filter::AdvancedFilter>,
    /// The settings of the last Data > Consolidate into this sheet.
    pub consolidation: Option<crate::consolidate::Consolidation>,
    /// Scenarios and data tables (What-If Analysis), see `whatif.rs`.
    pub scenarios: Vec<crate::whatif::Scenario>,
    pub data_tables: Vec<crate::whatif::DataTable>,
    /// The text files imported into this sheet, see `text_columns.rs`.
    pub text_imports: Vec<crate::text_columns::TextImport>,
    /// Tables ("Format as Table"), see `table.rs`.
    pub tables: Vec<crate::table::Table>,
    /// Conditional formats, in priority order, see `condfmt.rs`.
    pub cond_formats: Vec<crate::condfmt::CondFormat>,
    /// Data validation rules, see `validation.rs`.
    pub validations: Vec<crate::validation::Validation>,
    /// Notes and comments by cell, see `comment.rs`.
    pub comments: BTreeMap<(u32, u32), crate::comment::Comment>,
    /// Sheet protection, see `protect.rs`.
    pub protection: Option<crate::protect::SheetProtection>,
    /// Page setup for printing, see `page.rs`.
    pub page: crate::page::PageSetup,
    /// Pivot tables, see `pivot.rs`.
    pub pivots: Vec<crate::pivot::PivotTable>,
    /// Pictures and shapes floating over the cells, see `drawing.rs`.
    pub drawings: Vec<crate::drawing::Drawing>,
    /// Slicers and timelines floating over the cells, see `slicer.rs`.
    pub slicers: Vec<crate::slicer::Slicer>,
    pub timelines: Vec<crate::slicer::Timeline>,
    /// Strokes drawn by hand (the Draw tab), see `ink.rs`.
    pub inks: Vec<crate::ink::Ink>,
    /// The part this sheet has in each custom view, see `custom_view.rs`.
    pub custom_views: Vec<crate::custom_view::CustomSheetView>,
    /// Hyperlinks on cells, see `hyperlink.rs`.
    pub hyperlinks: Vec<crate::hyperlink::Hyperlink>,
    /// Defined names whose scope is this sheet, keyed by uppercase name
    /// (see `names.rs`). They travel with the sheet when it is moved,
    /// copied or deleted.
    pub names: BTreeMap<String, DefinedName>,
    /// Grouped rows and columns (Data > Group), see `outline.rs`.
    pub outline: crate::outline::Outline,
    /// Sparklines, by group (Insert > Sparklines), see `sparkline.rs`.
    pub sparklines: Vec<crate::sparkline::SparklineGroup>,
    /// The picture repeated behind the cells (Page Layout ▸ Background).
    /// It is shown on screen only: a background is not printed.
    pub background: Option<crate::drawing::Picture>,
    /// The name the VBA project of the workbook knows this sheet by, see
    /// `vba.rs`.
    pub code_name: Option<String>,
}

impl Sheet {
    pub fn new(name: impl Into<String>) -> Sheet {
        Sheet {
            name: name.into(),
            cells: BTreeMap::new(),
            col_widths: BTreeMap::new(),
            row_heights: BTreeMap::new(),
            default_col_width: DEFAULT_COL_WIDTH,
            default_row_height: DEFAULT_ROW_HEIGHT,
            hidden_cols: BTreeSet::new(),
            hidden_rows: BTreeSet::new(),
            col_styles: BTreeMap::new(),
            row_styles: BTreeMap::new(),
            merges: Vec::new(),
            frozen_rows: 0,
            frozen_cols: 0,
            show_gridlines: true,
            show_headers: true,
            tab_color: None,
            spills: Vec::new(),
            charts: Vec::new(),
            auto_filter: None,
            filtered_rows: BTreeSet::new(),
            advanced_filter: None,
            consolidation: None,
            scenarios: Vec::new(),
            data_tables: Vec::new(),
            text_imports: Vec::new(),
            tables: Vec::new(),
            cond_formats: Vec::new(),
            validations: Vec::new(),
            comments: BTreeMap::new(),
            protection: None,
            page: crate::page::PageSetup::default(),
            pivots: Vec::new(),
            drawings: Vec::new(),
            slicers: Vec::new(),
            timelines: Vec::new(),
            inks: Vec::new(),
            custom_views: Vec::new(),
            hyperlinks: Vec::new(),
            names: BTreeMap::new(),
            outline: crate::outline::Outline::default(),
            sparklines: Vec::new(),
            background: None,
            code_name: None,
        }
    }

    /// Column width in pixels at 100 % (0 when hidden).
    pub fn col_width(&self, col: u32) -> f32 {
        if self.hidden_cols.contains(&col) {
            return 0.0;
        }
        self.col_widths.get(&col).copied().unwrap_or(self.default_col_width)
    }

    /// Row height in pixels at 100 % (0 when hidden by hand or by a filter).
    pub fn row_height(&self, row: u32) -> f32 {
        if self.is_row_hidden(row) {
            return 0.0;
        }
        self.row_heights.get(&row).copied().unwrap_or(self.default_row_height)
    }

    /// True when the row is not shown: hidden by hand or by a filter.
    pub fn is_row_hidden(&self, row: u32) -> bool {
        self.hidden_rows.contains(&row) || self.filtered_rows.contains(&row)
    }

    /// The rows of `r0..=r1` that are shown, top to bottom.
    pub fn visible_rows(&self, r0: u32, r1: u32) -> impl Iterator<Item = u32> + '_ {
        (r0..=r1).filter(|r| !self.is_row_hidden(*r))
    }

    /// The parts of `rect` a filter does not hide, top to bottom: what
    /// formatting, clearing and filling act on when a selection runs over
    /// filtered rows, as in Excel. Rows hidden by hand are part of them.
    pub fn shown_parts(&self, rect: Rect) -> Vec<Rect> {
        let mut parts = Vec::new();
        let mut start = rect.r0;
        for &row in self.filtered_rows.range(rect.r0..=rect.r1) {
            if row > start {
                parts.push(Rect::new(start, rect.c0, row - 1, rect.c1));
            }
            start = row + 1;
        }
        if start <= rect.r1 {
            parts.push(Rect::new(start, rect.c0, rect.r1, rect.c1));
        }
        parts
    }

    /// Every row that is not shown, hidden by hand or by a filter (what a
    /// grid lays its rows out with).
    pub fn rows_not_shown(&self) -> std::borrow::Cow<'_, BTreeSet<u32>> {
        if self.filtered_rows.is_empty() {
            std::borrow::Cow::Borrowed(&self.hidden_rows)
        } else {
            std::borrow::Cow::Owned(self.hidden_rows.union(&self.filtered_rows).copied().collect())
        }
    }

    /// (max row, max col) of cells holding a constant or a formula, ignoring
    /// spilled results; (0, 0) for an empty sheet.
    pub fn cell_extent(&self) -> (u32, u32) {
        let mut r = 0;
        let mut c = 0;
        for ((row, col), cell) in self.cells.iter() {
            if !matches!(cell.content, Content::Empty) {
                r = r.max(*row);
                c = c.max(*col);
            }
        }
        (r, c)
    }

    /// (max row, max col) of cells with content; (0, 0) for an empty sheet.
    pub fn content_extent(&self) -> (u32, u32) {
        let mut r = 0;
        let mut c = 0;
        for ((row, col), cell) in self.cells.iter() {
            if !matches!(cell.content, Content::Empty) {
                r = r.max(*row);
                c = c.max(*col);
            }
        }
        for sp in &self.spills {
            r = r.max(sp.row + sp.array.rows as u32 - 1);
            c = c.max(sp.col + sp.array.cols as u32 - 1);
        }
        (r, c)
    }

    /// Bounding rectangle of all cells with content or formatting.
    pub fn used_rect(&self) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for (row, col) in self.cells.keys() {
            let r = Rect::cell(*row, *col);
            out = Some(match out {
                Some(o) => o.union(&r),
                None => r,
            });
        }
        for sp in &self.spills {
            let r = Rect::new(
                sp.row,
                sp.col,
                sp.row + sp.array.rows as u32 - 1,
                sp.col + sp.array.cols as u32 - 1,
            );
            out = Some(match out {
                Some(o) => o.union(&r),
                None => r,
            });
        }
        out
    }

    /// The merged area containing a cell, if any.
    pub fn merge_at(&self, row: u32, col: u32) -> Option<Rect> {
        self.merges.iter().find(|m| m.contains(row, col)).copied()
    }

    fn spill_value(&self, row: u32, col: u32) -> Option<Value> {
        for sp in &self.spills {
            if row >= sp.row
                && col >= sp.col
                && ((row - sp.row) as usize) < sp.array.rows
                && ((col - sp.col) as usize) < sp.array.cols
            {
                return Some(
                    sp.array
                        .get((row - sp.row) as usize, (col - sp.col) as usize)
                        .clone(),
                );
            }
        }
        None
    }

    /// The spill block covering a cell, if any.
    pub fn spill_at(&self, row: u32, col: u32) -> Option<&Spill> {
        self.spills.iter().find(|sp| {
            row >= sp.row
                && col >= sp.col
                && ((row - sp.row) as usize) < sp.array.rows
                && ((col - sp.col) as usize) < sp.array.cols
        })
    }

    /// Calculated value shown in a cell (including spilled array elements).
    pub fn value_at(&self, row: u32, col: u32) -> Value {
        match self.cells.get(&(row, col)) {
            Some(cell) if !matches!(cell.content, Content::Empty) => cell.value.clone(),
            _ => self.spill_value(row, col).unwrap_or(Value::Empty),
        }
    }

    /// True when the cell shows nothing.
    pub fn is_blank(&self, row: u32, col: u32) -> bool {
        match self.cells.get(&(row, col)) {
            Some(cell) if !matches!(cell.content, Content::Empty) => false,
            _ => self.spills.is_empty() || self.spill_value(row, col).is_none_or(|v| v.is_empty()),
        }
    }

    pub fn inherited_style(&self, row: u32, col: u32) -> StyleId {
        if let Some(s) = self.row_styles.get(&row) {
            return *s;
        }
        self.col_styles.get(&col).copied().unwrap_or_default()
    }

    pub fn style_id_at(&self, row: u32, col: u32) -> StyleId {
        match self.cells.get(&(row, col)) {
            Some(cell) => cell.style,
            None => self.inherited_style(row, col),
        }
    }
}

/// A defined name. Its scope is where it is stored: `Workbook::names` for
/// the whole workbook, `Sheet::names` for one sheet (see `names.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct DefinedName {
    pub name: String,
    /// What the name stands for: a reference, a constant or any formula,
    /// as it reads from cell A1. Relative references are relative to the
    /// cell using the name (`DefinedName::expr_at`).
    pub expr: Arc<Expr>,
    /// The note shown in the Name Manager.
    pub comment: String,
}

impl DefinedName {
    pub fn new(name: impl Into<String>, expr: Expr) -> DefinedName {
        DefinedName {
            name: name.into(),
            expr: Arc::new(expr),
            comment: String::new(),
        }
    }
}

#[derive(Clone)]
struct SheetsState {
    sheets: Vec<Sheet>,
    names: BTreeMap<String, DefinedName>,
    cell_styles: Vec<crate::cell_style::CellStyle>,
    /// The theme the sheets were in (see `theme.rs`).
    theme: Arc<crate::theme::Theme>,
}

#[derive(Clone)]
enum Change {
    Cell {
        sheet: usize,
        pos: (u32, u32),
        before: Option<Cell>,
        after: Option<Cell>,
    },
    ColWidth {
        sheet: usize,
        col: u32,
        before: Option<f32>,
        after: Option<f32>,
    },
    RowHeight {
        sheet: usize,
        row: u32,
        before: Option<f32>,
        after: Option<f32>,
    },
    ColStyle {
        sheet: usize,
        col: u32,
        before: Option<StyleId>,
        after: Option<StyleId>,
    },
    RowStyle {
        sheet: usize,
        row: u32,
        before: Option<StyleId>,
        after: Option<StyleId>,
    },
    /// Whole-workbook snapshot, used for structural edits.
    Sheets {
        before: Box<SheetsState>,
        after: Option<Box<SheetsState>>,
    },
}

struct UndoEntry {
    label: String,
    changes: Vec<Change>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearMode {
    Contents,
    /// Formatting, merged cells and conditional formats.
    Formats,
    /// Contents, formats, notes and comments, and data validation.
    All,
    /// Notes and comments only.
    Comments,
}

pub struct Workbook {
    pub sheets: Vec<Sheet>,
    pub styles: StyleTable,
    /// Defined names of workbook scope, keyed by uppercase name. Those
    /// scoped to a sheet are in `Sheet::names`.
    pub names: BTreeMap<String, DefinedName>,
    /// The cell styles the user made and the built-in ones that were
    /// changed, see `cell_style.rs`.
    pub cell_styles: Vec<crate::cell_style::CellStyle>,
    pub locale: &'static Locale,
    /// Set by `recalc` when a circular reference was found.
    pub circular: bool,
    /// True when there are unsaved changes.
    pub modified: bool,
    /// Protection of the workbook structure, see `protect.rs`.
    pub protection: crate::protect::WorkbookProtection,
    /// Incremented whenever a value may have changed (by every committed
    /// edit, undo, redo and recalculation): anything derived from cell
    /// values can be cached against it.
    pub generation: u64,
    /// Caches of the features that live in their own modules.
    pub caches: crate::features::Caches,
    /// Calculation settings, the dependency graph and what changed since
    /// the last calculation, see `recalc.rs`.
    pub(crate) calc: crate::recalc::CalcState,
    /// The VBA project of the workbook, kept as it was read, see `vba.rs`.
    pub vba: crate::vba::Macros,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
    pending: Vec<Change>,
    /// The current transaction holds a snapshot of the sheets.
    snapshot_taken: bool,
}

impl Workbook {
    /// A new workbook with one empty sheet.
    pub fn new(locale: &'static Locale) -> Workbook {
        let mut wb = Workbook {
            sheets: vec![Sheet::new(format!("{}1", locale.sheet_prefix))],
            styles: StyleTable::default(),
            names: BTreeMap::new(),
            cell_styles: Vec::new(),
            locale,
            circular: false,
            modified: false,
            protection: crate::protect::WorkbookProtection::default(),
            generation: 0,
            caches: crate::features::Caches::default(),
            calc: crate::recalc::CalcState::default(),
            vba: crate::vba::Macros::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending: Vec::new(),
            snapshot_taken: false,
        };
        // An empty workbook is calculated: what is typed into it is what
        // there is to calculate.
        wb.calc = crate::recalc::CalcState::of_calculated(&wb);
        wb
    }

    // ---- lookup ---------------------------------------------------------

    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name) || s.name.to_lowercase() == name.to_lowercase())
    }

    pub fn defined_name(&self, name: &str) -> Option<&Expr> {
        self.names.get(&name.to_uppercase()).map(|d| d.expr.as_ref())
    }

    pub fn cell(&self, sheet: usize, row: u32, col: u32) -> Option<&Cell> {
        self.sheets.get(sheet)?.cells.get(&(row, col))
    }

    /// Calculated value shown at a cell.
    pub fn value_at(&self, sheet: usize, row: u32, col: u32) -> Value {
        match self.sheets.get(sheet) {
            Some(s) => s.value_at(row, col),
            None => Value::Empty,
        }
    }

    pub fn style_id_at(&self, sheet: usize, row: u32, col: u32) -> StyleId {
        match self.sheets.get(sheet) {
            Some(s) => s.style_id_at(row, col),
            None => StyleId(0),
        }
    }

    pub fn style_at(&self, sheet: usize, row: u32, col: u32) -> &Style {
        self.styles.get(self.style_id_at(sheet, row, col))
    }

    /// Text shown in the grid for a cell, after number formatting.
    pub fn display_text(&self, sheet: usize, row: u32, col: u32) -> Formatted {
        let value = self.value_at(sheet, row, col);
        if value.is_empty() {
            return Formatted::default();
        }
        let style = self.style_at(sheet, row, col);
        format::format_value(&value, &style.num_fmt, self.locale)
    }

    /// Text shown in the formula bar / in-cell editor.
    pub fn edit_text(&self, sheet: usize, row: u32, col: u32) -> String {
        let Some(sh) = self.sheets.get(sheet) else {
            return String::new();
        };
        match sh.cells.get(&(row, col)) {
            Some(cell) => match self.formula_text(sheet, row, col) {
                Some(text) => text,
                None => {
                    let style = self.styles.get(cell.style);
                    input::edit_text(&cell.content, &style.num_fmt, self.locale)
                }
            },
            None => String::new(),
        }
    }

    /// Formula text (with leading `=`) of a cell, if it holds a formula.
    /// Inside a table, references to that table are shown without its name
    /// (`=[@Qty]*2`), as Excel does.
    pub fn formula_text(&self, sheet: usize, row: u32, col: u32) -> Option<String> {
        match &self.cell(sheet, row, col)?.content {
            Content::Formula(e) => Some(match self.table_at(sheet, row, col) {
                Some(table) if e.has_table_refs() => {
                    let shown = e.with_table_unqualified(&table.name);
                    format!("={}", shown.to_text(self.locale))
                }
                _ => format!("={}", e.to_text(self.locale)),
            }),
            _ => None,
        }
    }

    // ---- recording primitives ------------------------------------------

    /// Replace a cell (or remove it with `None`), recording the change.
    pub fn put_cell(&mut self, sheet: usize, row: u32, col: u32, cell: Option<Cell>) {
        if row >= MAX_ROWS || col >= MAX_COLS {
            return;
        }
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        // A cell with nothing in it and no formatting of its own is not stored.
        let inherited = sh.inherited_style(row, col);
        let mut cell = cell.filter(|c| !(matches!(c.content, Content::Empty) && c.style == inherited));
        let current = sh.cells.get(&(row, col));
        if crate::recalc::same_content(current, cell.as_ref()) {
            // The same formula keeps the result it has: only a change of
            // content is calculated again.
            if let (Some(old), Some(new)) = (current, cell.as_mut()) {
                new.value = old.value.clone();
            }
            if current.map(|c| c.style) == cell.as_ref().map(|c| c.style) {
                return;
            }
        } else {
            let replaced = current.is_some_and(|c| matches!(c.content, Content::Formula(_)));
            self.calc.touch(sheet, row, col, replaced);
        }
        let before = match &cell {
            Some(c) => sh.cells.insert((row, col), c.clone()),
            None => sh.cells.remove(&(row, col)),
        };
        self.pending.push(Change::Cell {
            sheet,
            pos: (row, col),
            before,
            after: cell,
        });
    }

    /// Take a snapshot of all sheets before a structural edit (insert/delete
    /// rows, sheet operations, merges, ...). Call once before mutating
    /// `self.sheets` directly.
    pub fn snapshot(&mut self) {
        if self.snapshot_taken {
            return;
        }
        self.snapshot_taken = true;
        self.pending.push(Change::Sheets {
            before: Box::new(SheetsState {
                sheets: self.sheets.clone(),
                names: self.names.clone(),
                cell_styles: self.cell_styles.clone(),
                theme: self.theme_arc(),
            }),
            after: None,
        });
    }

    /// Close the current transaction: push it on the undo stack and
    /// recalculate. Returns false when nothing changed.
    pub fn commit(&mut self, label: &str) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let mut changes = std::mem::take(&mut self.pending);
        self.snapshot_taken = false;
        for ch in &mut changes {
            if let Change::Sheets { before, after } = ch {
                // What was changed directly on the sheets since the
                // snapshot is found by comparing with it.
                self.calc.sheets_changed(&before.sheets, &mut self.sheets);
                *after = Some(Box::new(SheetsState {
                    sheets: self.sheets.clone(),
                    names: self.names.clone(),
                    cell_styles: self.cell_styles.clone(),
                theme: self.theme_arc(),
                }));
            }
        }
        self.undo_stack.push(UndoEntry {
            label: label.to_string(),
            changes,
        });
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.modified = true;
        self.changed();
        true
    }

    /// Drop the changes of the current (uncommitted) transaction.
    pub fn rollback(&mut self) {
        let changes = std::mem::take(&mut self.pending);
        self.snapshot_taken = false;
        self.apply_changes(&changes, true);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo_stack.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo_stack.last().map(|e| e.label.as_str())
    }

    pub fn undo(&mut self) -> bool {
        let Some(entry) = self.undo_stack.pop() else {
            return false;
        };
        self.apply_changes(&entry.changes, true);
        self.redo_stack.push(entry);
        self.modified = true;
        self.changed();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(entry) = self.redo_stack.pop() else {
            return false;
        };
        self.apply_changes(&entry.changes, false);
        self.undo_stack.push(entry);
        self.modified = true;
        self.changed();
        true
    }

    /// Forget the undo history (after loading a file).
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending.clear();
        self.snapshot_taken = false;
    }

    fn apply_changes(&mut self, changes: &[Change], reverse: bool) {
        let apply = |wb: &mut Workbook, ch: &Change| match ch {
            Change::Cell {
                sheet,
                pos,
                before,
                after,
            } => {
                let target = if reverse { before } else { after };
                if let Some(sh) = wb.sheets.get_mut(*sheet) {
                    let mut target = target.clone();
                    let current = sh.cells.get(pos);
                    if crate::recalc::same_content(current, target.as_ref()) {
                        // Only the formatting comes back: the result of a
                        // formula is the one calculated since.
                        if let (Some(old), Some(new)) = (current, target.as_mut()) {
                            new.value = old.value.clone();
                        }
                    } else {
                        let replaced = current.is_some_and(|c| matches!(c.content, Content::Formula(_)));
                        wb.calc.touch(*sheet, pos.0, pos.1, replaced);
                    }
                    match target {
                        Some(c) => {
                            sh.cells.insert(*pos, c);
                        }
                        None => {
                            sh.cells.remove(pos);
                        }
                    }
                }
            }
            Change::ColWidth {
                sheet,
                col,
                before,
                after,
            } => {
                let target = if reverse { before } else { after };
                if let Some(sh) = wb.sheets.get_mut(*sheet) {
                    match target {
                        Some(w) => {
                            sh.col_widths.insert(*col, *w);
                        }
                        None => {
                            sh.col_widths.remove(col);
                        }
                    }
                }
            }
            Change::RowHeight {
                sheet,
                row,
                before,
                after,
            } => {
                let target = if reverse { before } else { after };
                if let Some(sh) = wb.sheets.get_mut(*sheet) {
                    match target {
                        Some(h) => {
                            sh.row_heights.insert(*row, *h);
                        }
                        None => {
                            sh.row_heights.remove(row);
                        }
                    }
                }
            }
            Change::ColStyle {
                sheet,
                col,
                before,
                after,
            } => {
                let target = if reverse { before } else { after };
                if let Some(sh) = wb.sheets.get_mut(*sheet) {
                    match target {
                        Some(s) => {
                            sh.col_styles.insert(*col, *s);
                        }
                        None => {
                            sh.col_styles.remove(col);
                        }
                    }
                }
            }
            Change::RowStyle {
                sheet,
                row,
                before,
                after,
            } => {
                let target = if reverse { before } else { after };
                if let Some(sh) = wb.sheets.get_mut(*sheet) {
                    match target {
                        Some(s) => {
                            sh.row_styles.insert(*row, *s);
                        }
                        None => {
                            sh.row_styles.remove(row);
                        }
                    }
                }
            }
            Change::Sheets { before, after } => {
                let target = if reverse {
                    Some(before)
                } else {
                    after.as_ref()
                };
                if let Some(state) = target {
                    let replaced = std::mem::replace(&mut wb.sheets, state.sheets.clone());
                    wb.names = state.names.clone();
                    wb.cell_styles = state.cell_styles.clone();
                    wb.restore_theme(&state.theme);
                    // The snapshot brings contents back; results are those
                    // of the workbook as it was calculated last.
                    wb.calc.sheets_changed(&replaced, &mut wb.sheets);
                }
            }
        };
        if reverse {
            for ch in changes.iter().rev() {
                apply(self, ch);
            }
        } else {
            for ch in changes {
                apply(self, ch);
            }
        }
    }

    // ---- recalculation: see `recalc.rs` ---------------------------------

    /// Evaluate formula text against the current state without storing it
    /// (used for previews such as the function-argument dialog).
    pub fn evaluate(&self, sheet: usize, text: &str) -> Result<Value, ParseError> {
        let src = text.strip_prefix('=').unwrap_or(text);
        let expr = crate::parser::parse_formula(src, self.locale)?;
        Ok(Calc::eval_once(self, sheet, 0, 0, &expr))
    }

    // ---- cell edits -----------------------------------------------------

    /// Replace a cell's content, keeping its formatting.
    pub fn set_content(&mut self, sheet: usize, row: u32, col: u32, content: Content) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let style = sh.style_id_at(row, col);
        let value = match &content {
            Content::Value(v) => v.clone(),
            _ => Value::Empty,
        };
        self.put_cell(
            sheet,
            row,
            col,
            Some(Cell {
                content,
                value,
                style,
            }),
        );
    }

    pub fn set_value(&mut self, sheet: usize, row: u32, col: u32, value: Value) {
        let content = if value.is_empty() {
            Content::Empty
        } else {
            Content::Value(value)
        };
        self.set_content(sheet, row, col, content);
    }

    pub fn set_formula(&mut self, sheet: usize, row: u32, col: u32, expr: Expr) {
        self.set_content(sheet, row, col, Content::Formula(Arc::new(expr)));
    }

    /// Enter text exactly as the user typed it: formulas, numbers, dates,
    /// percentages, booleans or plain text. A number format is applied when
    /// the entry implies one (dates, percentages, currency) and the cell is
    /// still "General".
    pub fn set_cell_text(&mut self, sheet: usize, row: u32, col: u32, text: &str) -> Result<(), ParseError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let style_id = sh.style_id_at(row, col);
        let style = self.styles.get(style_id);
        // Cells formatted as Text keep whatever is typed verbatim.
        if style.num_fmt.as_ref() == "@" && !text.is_empty() {
            self.set_content(sheet, row, col, Content::Value(Value::text(text)));
            return Ok(());
        }
        let mut parsed = input::parse_input(text, self.locale)?;
        if let Content::Formula(e) = &parsed.content
            && e.has_table_refs()
        {
            let qualified = self.qualify_table_refs(e, sheet, row, col)?;
            parsed.content = Content::Formula(Arc::new(qualified));
        }
        let mut new_style = style_id;
        if let Some(fmt) = &parsed.num_fmt {
            if style.is_general() {
                let mut s = style.clone();
                s.num_fmt = Arc::from(fmt.as_str());
                new_style = self.styles.intern(s);
            }
        }
        let value = match &parsed.content {
            Content::Value(v) => v.clone(),
            _ => Value::Empty,
        };
        self.put_cell(
            sheet,
            row,
            col,
            Some(Cell {
                content: parsed.content,
                value,
                style: new_style,
            }),
        );
        Ok(())
    }

    /// Set the formatting of one cell.
    pub fn set_style_id(&mut self, sheet: usize, row: u32, col: u32, style: StyleId) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let cell = match sh.cells.get(&(row, col)) {
            Some(c) => Cell {
                style,
                ..c.clone()
            },
            None => Cell {
                content: Content::Empty,
                value: Value::Empty,
                style,
            },
        };
        self.put_cell(sheet, row, col, Some(cell));
    }

    fn set_col_style(&mut self, sheet: usize, col: u32, style: Option<StyleId>) {
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let style = style.filter(|s| *s != StyleId(0));
        let before = match style {
            Some(s) => sh.col_styles.insert(col, s),
            None => sh.col_styles.remove(&col),
        };
        if before != style {
            self.pending.push(Change::ColStyle {
                sheet,
                col,
                before,
                after: style,
            });
        }
    }

    fn set_row_style(&mut self, sheet: usize, row: u32, style: Option<StyleId>) {
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let style = style.filter(|s| *s != StyleId(0));
        let before = match style {
            Some(s) => sh.row_styles.insert(row, s),
            None => sh.row_styles.remove(&row),
        };
        if before != style {
            self.pending.push(Change::RowStyle {
                sheet,
                row,
                before,
                after: style,
            });
        }
    }

    /// Apply a formatting change to every cell of a rectangle. Whole-column
    /// and whole-row selections are stored on the column / row so the sheet
    /// does not fill up with empty cells. Rows hidden by a filter keep
    /// their formatting, as in Excel.
    pub fn update_style(&mut self, sheet: usize, rect: Rect, f: &dyn Fn(&mut Style)) {
        if sheet >= self.sheets.len() {
            return;
        }
        let filtered: Vec<u32> = self.sheets[sheet].filtered_rows.range(rect.r0..=rect.r1).copied().collect();
        let is_filtered = |row: u32| filtered.binary_search(&row).is_ok();
        let mut memo: HashMap<StyleId, StyleId> = HashMap::new();
        let mut map = |wb: &mut Workbook, id: StyleId| -> StyleId {
            if let Some(n) = memo.get(&id) {
                return *n;
            }
            let mut s = wb.styles.get(id).clone();
            f(&mut s);
            let n = wb.styles.intern(s);
            memo.insert(id, n);
            n
        };

        let whole_cols = rect.is_whole_cols();
        let whole_rows = rect.is_whole_rows();
        if whole_cols || whole_rows {
            if whole_cols {
                // The cells of filtered rows that have no formatting of
                // their own, in the columns that hold something: they must
                // not take the new formatting of their column.
                let last = self.sheets[sheet].used_rect().map_or(rect.c0, |used| used.c1.min(rect.c1));
                let mut kept: Vec<(u32, u32, StyleId)> = Vec::new();
                for &row in &filtered {
                    let sh = &self.sheets[sheet];
                    if sh.row_styles.contains_key(&row) {
                        continue;
                    }
                    for col in rect.c0..=last {
                        if !sh.cells.contains_key(&(row, col)) {
                            kept.push((row, col, sh.inherited_style(row, col)));
                        }
                    }
                }
                for col in rect.c0..=rect.c1 {
                    let cur = self.sheets[sheet].col_styles.get(&col).copied().unwrap_or_default();
                    let new = map(self, cur);
                    self.set_col_style(sheet, col, Some(new));
                }
                for (row, col, style) in kept {
                    self.set_style_id(sheet, row, col, style);
                }
            }
            if whole_rows && !whole_cols {
                // Where a formatted column crosses these rows, the cell must
                // keep the column's formatting plus the change: give it a
                // real cell before the row style takes precedence.
                let cols: Vec<(u32, StyleId)> = self.sheets[sheet].col_styles.iter().map(|(c, s)| (*c, *s)).collect();
                for row in rect.r0..=rect.r1 {
                    if self.sheets[sheet].row_styles.contains_key(&row) || is_filtered(row) {
                        continue;
                    }
                    for (col, col_style) in &cols {
                        if !self.sheets[sheet].cells.contains_key(&(row, *col)) {
                            let new = map(self, *col_style);
                            self.set_style_id(sheet, row, *col, new);
                        }
                    }
                }
                for row in (rect.r0..=rect.r1).filter(|r| !is_filtered(*r)) {
                    let cur = self.sheets[sheet].row_styles.get(&row).copied().unwrap_or_default();
                    let new = map(self, cur);
                    self.set_row_style(sheet, row, Some(new));
                }
            }
            // Existing cells inside the area.
            let keys: Vec<((u32, u32), StyleId)> = self.sheets[sheet]
                .cells
                .iter()
                .filter(|((r, c), _)| rect.contains(*r, *c) && !is_filtered(*r))
                .map(|(k, cell)| (*k, cell.style))
                .collect();
            for ((r, c), id) in keys {
                let new = map(self, id);
                self.set_style_id(sheet, r, c, new);
            }
            // Row-styled rows crossing a formatted column need real cells.
            if whole_cols && !whole_rows {
                let rows: Vec<u32> = self.sheets[sheet].row_styles.keys().copied().collect();
                for r in rows.into_iter().filter(|r| !is_filtered(*r)) {
                    for c in rect.c0..=rect.c1 {
                        if !self.sheets[sheet].cells.contains_key(&(r, c)) {
                            let cur = self.sheets[sheet].inherited_style(r, c);
                            let new = map(self, cur);
                            self.set_style_id(sheet, r, c, new);
                        }
                    }
                }
            }
            return;
        }
        for r in (rect.r0..=rect.r1).filter(|r| !is_filtered(*r)) {
            for c in rect.c0..=rect.c1 {
                let cur = self.sheets[sheet].style_id_at(r, c);
                let new = map(self, cur);
                if new != cur || !self.sheets[sheet].cells.contains_key(&(r, c)) {
                    self.set_style_id(sheet, r, c, new);
                }
            }
        }
    }

    /// Clear what the cells of a rectangle hold, as the Clear menu does:
    /// contents, formats (with merged cells and conditional formats),
    /// notes and comments, or everything (data validation included). Rows
    /// hidden by a filter are left alone, as in Excel.
    pub fn clear(&mut self, sheet: usize, rect: Rect, mode: ClearMode) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let contents = matches!(mode, ClearMode::Contents | ClearMode::All);
        let formats = matches!(mode, ClearMode::Formats | ClearMode::All);
        let keys: Vec<(u32, u32)> = if contents || formats {
            sh.cells
                .range((rect.r0, 0)..=(rect.r1, u32::MAX))
                .filter(|((r, c), _)| *c >= rect.c0 && *c <= rect.c1 && !sh.filtered_rows.contains(r))
                .map(|(k, _)| *k)
                .collect()
        } else {
            Vec::new()
        };
        for (r, c) in keys {
            let cell = self.sheets[sheet].cells[&(r, c)].clone();
            let new = Cell {
                content: if contents { Content::Empty } else { cell.content },
                value: if contents { Value::Empty } else { cell.value },
                style: if formats { StyleId(0) } else { cell.style },
            };
            self.put_cell(sheet, r, c, Some(new));
        }
        if formats {
            if rect.is_whole_cols() {
                for col in rect.c0..=rect.c1 {
                    self.set_col_style(sheet, col, None);
                }
            }
            if rect.is_whole_rows() {
                for row in rect.r0..=rect.r1 {
                    self.set_row_style(sheet, row, None);
                }
            }
            let had = self.sheets[sheet].merges.iter().any(|m| rect.intersects(m));
            if had {
                self.snapshot();
                self.sheets[sheet].merges.retain(|m| !rect.intersects(m));
            }
        }
        // What the features keep on the cells goes as Excel has it:
        // conditional formats with the formats, notes and comments on
        // request, validation only with everything.
        let shown = self.sheets[sheet].shown_parts(rect);
        if formats {
            self.clear_cond_formats_in_bands(sheet, &shown);
        }
        if mode == ClearMode::All {
            self.clear_validation_in_bands(sheet, &shown);
        }
        if matches!(mode, ClearMode::All | ClearMode::Comments) {
            self.delete_comments(sheet, &shown);
        }
    }

    // ---- geometry -------------------------------------------------------

    pub fn set_col_width(&mut self, sheet: usize, col: u32, width: Option<f32>) {
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let width = width.map(|w| w.clamp(0.0, 1800.0));
        let before = match width {
            Some(w) => sh.col_widths.insert(col, w),
            None => sh.col_widths.remove(&col),
        };
        if before != width {
            self.pending.push(Change::ColWidth {
                sheet,
                col,
                before,
                after: width,
            });
        }
    }

    pub fn set_row_height(&mut self, sheet: usize, row: u32, height: Option<f32>) {
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let height = height.map(|h| h.clamp(0.0, 546.0));
        let before = match height {
            Some(h) => sh.row_heights.insert(row, h),
            None => sh.row_heights.remove(&row),
        };
        if before != height {
            self.pending.push(Change::RowHeight {
                sheet,
                row,
                before,
                after: height,
            });
        }
    }

    // ---- sheets ---------------------------------------------------------

    /// A sheet name not yet used in the workbook ("Sheet2", "Feuil3", ...).
    pub fn next_sheet_name(&self) -> String {
        let mut n = self.sheets.len() + 1;
        loop {
            let name = format!("{}{}", self.locale.sheet_prefix, n);
            if self.sheet_index(&name).is_none() {
                return name;
            }
            n += 1;
        }
    }

    /// Insert a new empty sheet at `index`; returns its index.
    pub fn add_sheet(&mut self, index: usize) -> usize {
        self.snapshot();
        let name = self.next_sheet_name();
        let index = index.min(self.sheets.len());
        let mut sheet = Sheet::new(name);
        // The default sizes follow from the default font of the workbook:
        // a new sheet has those of the sheets already there.
        if let Some(first) = self.sheets.first() {
            sheet.default_col_width = first.default_col_width;
            sheet.default_row_height = first.default_row_height;
        }
        self.sheets.insert(index, sheet);
        index
    }

    /// Delete a sheet (the last remaining sheet cannot be deleted).
    ///
    /// As in Excel, what referred to the sheet is left pointing nowhere:
    /// formulas, defined names and chart series read `#REF!A1`, and 3-D
    /// references ending on the sheet shrink. The names scoped to the
    /// sheet go with it.
    pub fn delete_sheet(&mut self, index: usize) -> bool {
        if self.sheets.len() <= 1 || index >= self.sheets.len() {
            return false;
        }
        self.snapshot();
        let order: Vec<String> = self.sheets.iter().map(|s| s.name.clone()).collect();
        let order: Vec<&str> = order.iter().map(|s| s.as_str()).collect();
        let removed = self.sheets.remove(index);
        let deleted = removed.name.as_str();
        self.map_all_formulas(&|e| e.with_sheet_deleted(deleted, &order));
        for sh in &mut self.sheets {
            for chart in &mut sh.charts {
                chart.map_refs(&mut |r| {
                    let mut r = r.clone();
                    let gone = |s: &str| crate::ast::same_name(s, deleted);
                    if r.sheet.as_deref().is_some_and(gone) {
                        r.sheet = Some(Arc::from(crate::ast::DELETED_SHEET));
                    }
                    Some(r)
                });
            }
        }
        crate::features::on_sheet_deleted(self, &removed, &order);
        true
    }

    /// Replace every formula of the workbook (cells and defined names)
    /// with `f(formula)`. The caller has taken the snapshot.
    pub(crate) fn map_all_formulas(&mut self, f: &dyn Fn(&Expr) -> Expr) {
        let apply = |expr: &mut Arc<Expr>| {
            let new = f(expr);
            if new != **expr {
                *expr = Arc::new(new);
            }
        };
        for sh in &mut self.sheets {
            for cell in sh.cells.values_mut() {
                if let Content::Formula(e) = &mut cell.content {
                    apply(e);
                }
            }
            for def in sh.names.values_mut() {
                apply(&mut def.expr);
            }
        }
        for def in self.names.values_mut() {
            apply(&mut def.expr);
        }
    }

    /// Rename a sheet, rewriting references to it. Fails when the name is
    /// empty, too long, contains forbidden characters or is already taken.
    pub fn rename_sheet(&mut self, index: usize, new_name: &str) -> bool {
        let new_name = new_name.trim();
        if new_name.is_empty()
            || new_name.chars().count() > 31
            || new_name.contains(['\\', '/', '?', '*', '[', ']', ':'])
            || index >= self.sheets.len()
        {
            return false;
        }
        if let Some(existing) = self.sheet_index(new_name) {
            if existing != index {
                return false;
            }
        }
        let old = self.sheets[index].name.clone();
        if old == new_name {
            return true;
        }
        self.snapshot();
        self.sheets[index].name = new_name.to_string();
        self.map_all_formulas(&|e| e.with_sheet_renamed(&old, new_name));
        for sh in &mut self.sheets {
            for chart in &mut sh.charts {
                chart.rename_sheet(&old, new_name);
            }
            crate::features::on_sheet_renamed(sh, &old, new_name);
        }
        true
    }

    /// Copy a sheet; the copy is inserted right after the original.
    pub fn duplicate_sheet(&mut self, index: usize) -> Option<usize> {
        if index >= self.sheets.len() {
            return None;
        }
        self.snapshot();
        let mut copy = self.sheets[index].clone();
        let base = copy.name.clone();
        let mut n = 2;
        loop {
            let name = format!("{base} ({n})");
            if self.sheet_index(&name).is_none() {
                copy.name = name;
                break;
            }
            n += 1;
        }
        // Charts on the copy plot the copy's own cells, with fresh ids.
        let mut next_id = self.sheets.iter().flat_map(|s| s.charts.iter().map(|c| c.id)).max().unwrap_or(0);
        let new_name = copy.name.clone();
        // Formulas that named the original sheet in full read the copy.
        for cell in copy.cells.values_mut() {
            if let Content::Formula(e) = &cell.content {
                let renamed = e.with_sheet_renamed(&base, &new_name);
                if renamed != **e {
                    cell.content = Content::Formula(Arc::new(renamed));
                }
            }
        }
        // The names scoped to the copy stand for the copy's own cells.
        for def in copy.names.values_mut() {
            def.expr = Arc::new(def.expr.with_sheet_renamed(&base, &new_name));
        }
        for chart in &mut copy.charts {
            next_id += 1;
            chart.id = next_id;
            chart.rename_sheet(&base, &new_name);
        }
        self.sheets.insert(index + 1, copy);
        crate::features::on_sheet_duplicated(self, index + 1, &base);
        Some(index + 1)
    }

    pub fn move_sheet(&mut self, from: usize, to: usize) {
        if from >= self.sheets.len() || to >= self.sheets.len() || from == to {
            return;
        }
        self.snapshot();
        let before: Vec<String> = self.sheets.iter().map(|s| s.name.clone()).collect();
        let s = self.sheets.remove(from);
        self.sheets.insert(to, s);
        // 3-D references follow their end sheets, except an end carried
        // past the other one.
        let after: Vec<String> = self.sheets.iter().map(|s| s.name.clone()).collect();
        let before: Vec<&str> = before.iter().map(String::as_str).collect();
        let after: Vec<&str> = after.iter().map(String::as_str).collect();
        let moved = before[from];
        self.map_all_formulas(&|e| e.with_sheet_moved(moved, &before, &after));
        crate::features::on_sheet_moved(self, moved, &before, &after);
    }

    pub fn set_tab_color(&mut self, index: usize, color: Option<Color>) {
        if index < self.sheets.len() {
            self.snapshot();
            self.sheets[index].tab_color = color;
        }
    }

    /// Give a sheet a background picture, or (`None`) take it away.
    /// Returns false when there is no such sheet.
    pub fn set_sheet_background(&mut self, sheet: usize, picture: Option<crate::drawing::Picture>) -> bool {
        if sheet >= self.sheets.len() {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].background = picture;
        true
    }

    pub fn set_frozen(&mut self, sheet: usize, rows: u32, cols: u32) {
        if sheet < self.sheets.len() {
            self.snapshot();
            self.sheets[sheet].frozen_rows = rows;
            self.sheets[sheet].frozen_cols = cols;
        }
    }

    /// Define (or redefine) a workbook-scoped name without checking it.
    /// The Name Manager goes through `add_name` / `edit_name` (`names.rs`),
    /// which validate the name as Excel does. `expr` is in stored form:
    /// its relative references are offsets from the cell using the name,
    /// written as they read from A1 (see `parse_name_formula`).
    pub fn define_name(&mut self, name: &str, expr: Expr) {
        self.snapshot();
        let key = name.to_uppercase();
        self.names.insert(key, DefinedName::new(name, expr));
    }
}
