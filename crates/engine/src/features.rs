//! Glue between the workbook core and the features that live in their own
//! modules (filters, tables, conditional formats, validation, comments,
//! protection, page setup, pivot tables, drawings).
//!
//! The core calls the hooks below; each one forwards to a function of the
//! same name in every feature module, so a feature only ever edits its own
//! file.

use std::borrow::Cow;

use crate::ast::StructuralEdit;
use crate::style::{Style, StylePatch};
use crate::workbook::{Sheet, Workbook};
use crate::{comment, condfmt, drawing, filter, page, pivot, protect, table, validation};

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
