//! Conditional formatting: rules over ranges that change how cells look
//! (`Sheet::cond_formats`).
//!
//! Skeleton: the model and the behaviour are still to be written.

use crate::ast::StructuralEdit;
use crate::workbook::{Sheet, Workbook};
use crate::style::StylePatch;

/// One conditional format: a rule and the ranges it applies to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CondFormat {
    pub ranges: Vec<crate::refs::Rect>,
}

/// What a conditional format draws inside a cell besides restyling it.
#[derive(Clone, Debug, PartialEq)]
pub enum Adornment {}

/// What the conditional formats of a sheet do to one cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CondResult {
    pub patch: StylePatch,
    pub adornment: Option<Adornment>,
}

/// The effect of the sheet's conditional formats on a cell; `None` when no
/// rule applies. Called for every visible cell at every repaint: cache.
pub fn result_at(_wb: &Workbook, _sheet: usize, _row: u32, _col: u32) -> Option<CondResult> {
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
