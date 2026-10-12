//! Conditional formatting: rules over ranges that change how cells look
//! (`Sheet::cond_formats`).
//!
//! * `model` — the rules, one type per OOXML construct (`cfRule` and its
//!   `colorScale` / `dataBar` / `iconSet` children, with the x14 extensions),
//!   so files load and save without loss.
//! * `presets` — what the ribbon galleries create, with Excel's colours.
//! * `eval` — [`result_at`]: what the rules do to one cell, and the cache
//!   that keeps it cheap enough to call for every cell of every repaint.
//! * `formula` — the formulas inside rules: typing and showing them,
//!   evaluating them for many cells at once, the text Excel stores in files.
//! * `manage` — the `Workbook` API behind the ribbon and the Rules Manager,
//!   copy / paste, and the hooks the core calls when the sheet changes shape.
//!
//! `Sheet::cond_formats` is in priority order: index 0 is the rule Excel
//! shows at the top of the Rules Manager (`priority="1"` in a file).
//!
//! Formulas in a rule are written for the top-left cell of the rule's first
//! range (its *anchor*) and read by every other cell as if copied there:
//! relative references move (around the edges of the sheet if need be),
//! absolute ones stay.

mod eval;
mod formula;
mod manage;
mod model;
mod presets;

pub use eval::{Adornment, Bar, Cache, CellIcon, CondResult, result_at};
pub use formula::{
    is_relative, ooxml_formulas, operand_text, parse_operand, parse_ranges, parse_text_operand, ranges_text,
    text_operand_from_ooxml,
};
pub use manage::normalize_ranges;
pub(crate) use manage::{
    on_cells_shifted, on_sheet_deleted, on_sheet_duplicated, on_sheet_moved, on_sheet_renamed, on_structural_edit,
};
pub use model::{
    BarAxis, BarDirection, CellOp, Cfvo, ColorScale, ColorStop, CondFormat, DataBar, Icon, IconRule, IconSet,
    IconThreshold, Rule, TextOp, TimePeriod,
};
pub use presets::{BarColor, HighlightFormat, ScalePreset};

#[cfg(test)]
mod tests;
