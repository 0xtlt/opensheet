//! Protection of sheets (`Sheet::protection`) and of the workbook
//! structure (`Workbook::protection`).
//!
//! Skeleton: the model and the behaviour are still to be written.

use crate::ast::StructuralEdit;
use crate::workbook::{Sheet, Workbook};

/// A protected sheet: locked cells cannot be edited; the flags say what is
/// still allowed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetProtection {}

/// Protection of the workbook structure (adding, deleting, renaming and
/// moving sheets).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkbookProtection {}

/// Cache of values derived from the cells (see `features::Caches`).
#[derive(Debug, Default)]
pub struct Cache;

/// See `features::on_structural_edit`.
pub(crate) fn on_structural_edit(_sheet: &mut Sheet, _edit: StructuralEdit, _edited_sheet: &str) {}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}
