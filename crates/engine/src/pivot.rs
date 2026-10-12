//! Pivot tables (`Sheet::pivots`).
//!
//! Skeleton: the model and the behaviour are still to be written.

use crate::ast::StructuralEdit;
use crate::features::CellShift;
use crate::workbook::{Sheet, Workbook};
use crate::style::StylePatch;

/// A pivot table: its source, its layout and where it is written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PivotTable {
    pub id: u32,
    pub name: String,
}

/// The pivot style at a cell; `None` outside pivot tables. Called for
/// every visible cell at every repaint.
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

/// See `features::on_sheet_deleted`.
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`.
pub(crate) fn on_cells_shifted(_wb: &mut Workbook, _sheet: usize, _shift: CellShift) {}
