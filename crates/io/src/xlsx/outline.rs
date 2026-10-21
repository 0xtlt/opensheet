//! Outlines (grouped rows and columns). A worksheet part spreads them
//! over several elements: `outlineLevel` and `collapsed` on each `<row>`
//! and `<col>`, the deepest levels on `<sheetFormatPr>`, where summaries
//! lie in `<outlinePr>` (a child of `<sheetPr>`), and whether the symbols
//! show on the `<sheetView>`. Folded rows are hidden rows.

use super::xml::Tag;
use opensheet_engine::outline::{AxisOutline, MAX_LEVEL};
use opensheet_engine::workbook::Sheet;
use std::fmt::Write as _;

/// Whether `<sheetPr>` has something of the outline to hold.
pub(crate) fn needs_sheet_pr(sheet: &Sheet) -> bool {
    !sheet.outline.summary_below || !sheet.outline.summary_right
}

/// Append the child of `<sheetPr>` that belongs to the outline (after
/// `tabColor`, before `pageSetUpPr`).
pub(crate) fn push_sheet_pr(x: &mut String, sheet: &Sheet) {
    if !needs_sheet_pr(sheet) {
        return;
    }
    x.push_str("<outlinePr");
    if !sheet.outline.summary_below {
        x.push_str(" summaryBelow=\"0\"");
    }
    if !sheet.outline.summary_right {
        x.push_str(" summaryRight=\"0\"");
    }
    x.push_str("/>");
}

pub(crate) fn read_sheet_pr(t: &Tag, sheet: &mut Sheet) {
    sheet.outline.summary_below = t.attr_bool("summaryBelow").unwrap_or(true);
    sheet.outline.summary_right = t.attr_bool("summaryRight").unwrap_or(true);
}

/// The attribute of `<sheetView>` that hides the outline symbols.
pub(crate) fn push_view_attrs(x: &mut String, sheet: &Sheet) {
    if !sheet.outline.show_symbols {
        x.push_str(" showOutlineSymbols=\"0\"");
    }
}

pub(crate) fn read_view(view: &Tag, sheet: &mut Sheet) {
    sheet.outline.show_symbols = view.attr_bool("showOutlineSymbols").unwrap_or(true);
}

/// The attributes of `<sheetFormatPr>` that say how deep the outline
/// goes: Excel sizes the outline margin from them.
pub(crate) fn push_format_attrs(x: &mut String, sheet: &Sheet) {
    let (rows, cols) = (
        sheet.outline.rows.max_level(),
        sheet.outline.cols.max_level(),
    );
    if rows > 0 {
        w!(x, " outlineLevelRow=\"{rows}\"");
    }
    if cols > 0 {
        w!(x, " outlineLevelCol=\"{cols}\"");
    }
}

/// The attributes of a `<row>` or a `<col>`.
pub(crate) fn push_attrs(x: &mut String, lines: &AxisOutline, index: u32) {
    push_level(x, lines.level(index), lines.collapsed.contains(&index));
}

pub(crate) fn push_level(x: &mut String, level: u8, collapsed: bool) {
    if level > 0 {
        w!(x, " outlineLevel=\"{level}\"");
    }
    if collapsed {
        x.push_str(" collapsed=\"1\"");
    }
}

/// Take the outline attributes of a `<row>` or a `<col>` covering the
/// rows (columns) `lo..=hi`.
pub(crate) fn read_attrs(t: &Tag, lines: &mut AxisOutline, lo: u32, hi: u32) {
    let level = t
        .attr_u32("outlineLevel")
        .unwrap_or(0)
        .min(MAX_LEVEL as u32) as u8;
    let collapsed = t.attr_bool("collapsed").unwrap_or(false);
    if level == 0 && !collapsed {
        return;
    }
    for index in lo..=hi {
        if level > 0 {
            lines.levels.insert(index, level);
        }
        if collapsed {
            lines.collapsed.insert(index);
        }
    }
}
