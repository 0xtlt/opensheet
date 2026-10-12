//! Page setup and pagination for printing (`Sheet::page`).
//!
//! Skeleton: the model and the behaviour are still to be written.

use crate::ast::StructuralEdit;
use crate::features::CellShift;
use crate::workbook::{Sheet, Workbook};

/// How a sheet prints: paper, orientation, margins, scaling, print area,
/// titles, headers and footers, manual page breaks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageSetup {}

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
