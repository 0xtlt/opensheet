//! Tables ("Format as Table"): a named range with a header row, named
//! columns, an optional totals row and a style (`Sheet::tables`).
//!
//! The model below is what structured references (`Table1[Column]`) are
//! resolved against; the behaviour (creating, resizing, styles, filters,
//! totals) is still to be written.

use crate::ast::StructuralEdit;
use crate::refs::Rect;
use crate::style::StylePatch;
use crate::workbook::{Sheet, Workbook};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableColumn {
    /// The header text; unique within the table, compared without case.
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Unique in the workbook.
    pub id: u32,
    /// Unique in the workbook, compared without case; a valid defined name
    /// ("Table1", "Tableau1").
    pub name: String,
    /// The whole table: header row, data rows and totals row.
    pub range: Rect,
    /// Whether the first row of `range` holds the column headers.
    pub header_row: bool,
    /// Whether the last row of `range` holds the totals.
    pub totals_row: bool,
    /// One per column of `range`, left to right.
    pub columns: Vec<TableColumn>,
}

impl Table {
    pub fn header_range(&self) -> Option<Rect> {
        self.header_row
            .then(|| Rect::new(self.range.r0, self.range.c0, self.range.r0, self.range.c1))
    }

    pub fn totals_range(&self) -> Option<Rect> {
        self.totals_row
            .then(|| Rect::new(self.range.r1, self.range.c0, self.range.r1, self.range.c1))
    }

    /// The data rows; `None` for a table without any.
    pub fn data_range(&self) -> Option<Rect> {
        let r0 = self.range.r0 + self.header_row as u32;
        let r1 = self.range.r1.checked_sub(self.totals_row as u32)?;
        (r0 <= r1).then(|| Rect::new(r0, self.range.c0, r1, self.range.c1))
    }

    /// Index of the column named `name`, compared without case.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        let wanted = name.to_lowercase();
        self.columns.iter().position(|c| c.name.to_lowercase() == wanted)
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.range.contains(row, col)
    }
}

impl Workbook {
    /// The table named `name` (compared without case) and its sheet.
    pub fn table_by_name(&self, name: &str) -> Option<(usize, &Table)> {
        let wanted = name.to_lowercase();
        self.sheets
            .iter()
            .enumerate()
            .find_map(|(si, sh)| sh.tables.iter().find(|t| t.name.to_lowercase() == wanted).map(|t| (si, t)))
    }

    /// The table a cell belongs to.
    pub fn table_at(&self, sheet: usize, row: u32, col: u32) -> Option<&Table> {
        self.sheets.get(sheet)?.tables.iter().find(|t| t.contains(row, col))
    }
}

/// The table style at a cell (header fill, banding, borders); `None`
/// outside tables. Called for every visible cell at every repaint.
pub fn style_patch_at(_wb: &Workbook, _sheet: usize, _row: u32, _col: u32) -> Option<StylePatch> {
    None
}

/// Cache of values derived from the cells (see `features::Caches`).
#[derive(Debug, Default)]
pub struct Cache;

/// See `features::on_structural_edit`.
pub(crate) fn on_structural_edit(_sheet: &mut Sheet, _edit: StructuralEdit, _edited_sheet: &str) {}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}
