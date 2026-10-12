//! Office Open XML spreadsheets (`.xlsx`).
//!
//! The reader and writer work directly on the package (a zip archive of XML
//! parts) and map it to and from the engine's [`Workbook`] model. What the
//! engine has no model for is ignored on load and therefore not written
//! back.
//!
//! `read.rs` and `write.rs` hold the package, the workbook part, the styles
//! and the cells. Each feature stored inside the worksheet or the workbook
//! part has a file of its own with a `read` / `write` pair (`validation.rs`,
//! `protection.rs`, `defaults.rs`, `condfmt.rs`, `page.rs`); the reader and
//! the writer only call them, at the place the schema gives their elements.
//! `dxf.rs` holds the differential formats rules and tables share.
//!
//! Features stored in parts of their own (`comments.rs`, `table.rs` with
//! the filters of `filter.rs`, `pivot.rs`, `drawing.rs`) add their parts,
//! relationships and content types through `parts.rs`, which also numbers
//! the relationships of each worksheet. `theme.rs` is the theme written
//! into every package, `stylerefs.rs` what those parts name in the styles
//! part of a file that is read.
//!
//! [`Workbook`]: opensheet_engine::Workbook

/// `write!` into a `String` cannot fail.
macro_rules! w {
    ($dst:expr, $($arg:tt)*) => {
        let _ = write!($dst, $($arg)*);
    };
}

mod background;
mod cell_style;
mod chart;
mod color;
mod comments;
mod condfmt;
mod datatools;
mod defaults;
mod drawing;
mod dxf;
mod filter;
mod formula;
mod hyperlink;
mod numfmt;
mod page;
mod parts;
mod pivot;
mod protection;
pub(crate) mod read;
mod stylerefs;
mod table;
mod theme;
mod validation;
pub(crate) mod write;
mod xml;

/// Column width as stored in a file → pixels at 100 %.
///
/// Files count in characters: `digit_width` is the width in pixels of the
/// widest digit of the workbook's default font (7 for Calibri 11, see
/// `defaults.rs`). The stored width already includes the 5 pixels of cell
/// padding, so a column shown as "8.43 characters" in Excel (8.43 × 7 + 5
/// = 64 px) is stored as 64 / 7 = 9.14. Excel truncates stored widths to
/// 1/256 of a character; values that land just below a whole pixel are
/// snapped to it.
pub(crate) fn col_width_to_px(width: f64, digit_width: f64) -> f32 {
    let px = (width * digit_width).max(0.0);
    let nearest = px.round();
    if (nearest - px).abs() < digit_width / 256.0 + 0.02 {
        nearest as f32
    } else {
        ((px * 100.0).round() / 100.0) as f32
    }
}

/// Pixels at 100 % → column width as stored in a file.
pub(crate) fn px_to_col_width(px: f32, digit_width: f64) -> f64 {
    // Widths are kept to 1/100 of a pixel; what an `f32` carries beyond
    // that is noise ("88.41" is not 88.41000366).
    let px = (px.max(0.0) as f64 * 100.0).round() / 100.0;
    ((px / digit_width) * 1e8).round() / 1e8
}

/// Row height in points → pixels at 100 % (96 dpi).
pub(crate) fn points_to_px(points: f64) -> f32 {
    let px = (points * 96.0 / 72.0).max(0.0);
    ((px * 1000.0).round() / 1000.0) as f32
}

/// Pixels at 100 % → row height in points.
pub(crate) fn px_to_points(px: f32) -> f64 {
    ((px.max(0.0) as f64 * 72.0 / 96.0) * 1e4).round() / 1e4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_widths() {
        // Values written by Excel for 64, 61, 145 and 100 pixel columns of
        // a workbook in Calibri 11 (digits 7 pixels wide).
        assert_eq!(col_width_to_px(9.140625, 7.0), 64.0);
        assert_eq!(col_width_to_px(8.7109375, 7.0), 61.0);
        assert_eq!(col_width_to_px(20.7109375, 7.0), 145.0);
        assert_eq!(col_width_to_px(14.28515625, 7.0), 100.0);
        assert_eq!(col_width_to_px(0.0, 7.0), 0.0);
        // The same in Calibri 12 (8 pixels): 72 and 100 pixels.
        assert_eq!(col_width_to_px(9.0, 8.0), 72.0);
        assert_eq!(col_width_to_px(12.5, 8.0), 100.0);
        // A width from Excel for Mac, which does not count in whole pixels.
        assert_eq!(col_width_to_px(10.83203125, 8.0), 86.66);
        for digit in [5.0, 7.0, 8.0, 11.0, 19.0] {
            for px in [0.0f32, 1.0, 12.0, 64.0, 64.5, 77.25, 100.0, 255.0, 1800.0] {
                let width = px_to_col_width(px, digit);
                assert_eq!(col_width_to_px(width, digit), px, "{px} px, digit {digit}");
                // As Excel would store it: cut to 1/256 of a character.
                if px.fract() == 0.0 {
                    let stored = (width * 256.0).floor() / 256.0;
                    assert_eq!(col_width_to_px(stored, digit), px, "{px} px, digit {digit}");
                }
            }
        }
    }

    #[test]
    fn row_heights() {
        assert_eq!(points_to_px(15.0), 20.0);
        assert_eq!(points_to_px(12.75), 17.0);
        assert_eq!(px_to_points(20.0), 15.0);
        for px in [0.0f32, 1.0, 17.0, 20.0, 25.0, 33.5, 409.0, 546.0] {
            assert_eq!(points_to_px(px_to_points(px)), px);
        }
    }
}
