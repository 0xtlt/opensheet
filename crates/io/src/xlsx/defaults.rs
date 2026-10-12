//! What a sheet gives the columns and rows that were given nothing:
//! `<sheetFormatPr>`.
//!
//! Files measure column widths in characters: multiples of the width of
//! the widest digit of the workbook's default font, so nothing about
//! columns can be read or written without the sizes that follow from that
//! font. The engine works them out (`opensheet_engine::metrics`).

use super::xml::Tag;
use super::{col_width_to_px, points_to_px, px_to_col_width, px_to_points};
use opensheet_engine::metrics::standard_col_width;
use opensheet_engine::workbook::Sheet;
use std::fmt::Write as _;

/// The sizes that follow from the default font of a workbook.
pub(crate) use opensheet_engine::metrics::FontMetrics as Defaults;

/// Tallest row the engine keeps, in pixels.
const MAX_ROW_PX: f32 = 546.0;

/// Set the default sizes of a sheet from its `<sheetFormatPr>`. The sheet
/// arrives with the sizes of the default font. Returns the default row
/// height exactly as the file states it, in pixels: rows repeating it have
/// no height of their own.
pub(crate) fn read(tag: &Tag, defaults: &Defaults, sheet: &mut Sheet) -> Option<f32> {
    // `defaultColWidth` is in the unit of `<col width>`; `baseColWidth`
    // is a number of characters, from which the default width derives.
    let width = tag.attr_f64("defaultColWidth").filter(|w| *w > 0.0);
    let base = tag
        .attr_f64("baseColWidth")
        .filter(|chars| (0.0..=255.0).contains(chars));
    let stated = match (width, base) {
        (Some(width), _) => Some(col_width_to_px(width, defaults.digit_width)),
        (None, Some(chars)) => Some(standard_col_width(chars, defaults.digit_width)),
        (None, None) => None,
    };
    if let Some(px) = stated {
        sheet.default_col_width = px.clamp(1.0, defaults.max_col_width());
    }

    let stated = tag
        .attr_f64("defaultRowHeight")
        .map(points_to_px)
        .filter(|px| *px > 0.0)?;
    sheet.default_row_height = if tag.attr_bool("customHeight").unwrap_or(false) {
        stated.min(MAX_ROW_PX)
    } else if defaults.exact_row_height && (stated - defaults.row_height).abs() <= 1.0 {
        // Not set by anyone: the height Excel worked out from the default
        // font on the screen of whoever saved the file (14.4 points for
        // Calibri 11 on a screen scaled to 125 %). Excel works it out
        // again on opening; so does this.
        defaults.row_height
    } else {
        // Row heights are whole pixels where the font is not known.
        stated.round().clamp(1.0, MAX_ROW_PX)
    };
    Some(stated)
}

/// Append the `<sheetFormatPr>` of a sheet.
pub(crate) fn write(x: &mut String, sheet: &Sheet, defaults: &Defaults) {
    x.push_str("<sheetFormatPr");
    // Excel derives the default width from the default font; only a width
    // that is not the font's is stated.
    if (sheet.default_col_width - defaults.col_width).abs() > 0.01 {
        let width = sheet.default_col_width.min(defaults.max_col_width());
        w!(
            x,
            " defaultColWidth=\"{}\"",
            px_to_col_width(width, defaults.digit_width)
        );
    }
    w!(
        x,
        " defaultRowHeight=\"{}\"",
        px_to_points(sheet.default_row_height).min(super::write::MAX_ROW_HEIGHT_PT)
    );
    // Without `customHeight` Excel ignores the height and takes the one
    // of the default font. Where that one is an estimate, a difference of
    // a pixel is not taken for somebody's choice.
    let tolerance = if defaults.exact_row_height { 0.01 } else { 1.5 };
    if (sheet.default_row_height - defaults.row_height).abs() > tolerance {
        x.push_str(" customHeight=\"1\"");
    }
    x.push_str("/>");
}
