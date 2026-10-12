//! Data validation: what may be typed into cells, with in-cell drop-down
//! lists, input messages and error alerts (`Sheet::validations`).
//!
//! Skeleton: the model and the behaviour are still to be written.

use crate::ast::StructuralEdit;
use crate::workbook::{Sheet, Workbook};

/// One validation rule and the ranges it applies to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Validation {
    pub ranges: Vec<crate::refs::Rect>,
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
