//! Office Open XML spreadsheets (`.xlsx`).
//!
//! The reader and writer work directly on the package (a zip archive of XML
//! parts) and map it to and from the engine's [`Workbook`] model. Parts the
//! engine has no model for (images, conditional formatting, data validation,
//! comments, pivot tables, ...) are ignored on load and therefore not written
//! back.
//!
//! [`Workbook`]: opensheet_engine::Workbook

/// `write!` into a `String` cannot fail.
macro_rules! w {
    ($dst:expr, $($arg:tt)*) => {
        let _ = write!($dst, $($arg)*);
    };
}

mod chart;
mod color;
mod formula;
mod numfmt;
pub(crate) mod read;
pub(crate) mod write;
mod xml;

/// Width in pixels of the widest digit of the default font at 100 % zoom.
/// Column widths in files are expressed in this unit.
const MAX_DIGIT_WIDTH: f64 = 7.0;

/// Column width as stored in a file → pixels at 100 %.
///
/// The stored width already includes the 5 pixels of cell padding, so a
/// column shown as "8.43 characters" in Excel (8.43 × 7 + 5 = 64 px) is
/// stored as 64 / 7 = 9.14. Excel truncates stored widths to 1/256 of a
/// character; values that land just below a whole pixel are snapped to it.
pub(crate) fn col_width_to_px(width: f64) -> f32 {
    let px = (width * MAX_DIGIT_WIDTH).max(0.0);
    let nearest = px.round();
    if (nearest - px).abs() < 0.05 {
        nearest as f32
    } else {
        ((px * 100.0).round() / 100.0) as f32
    }
}

/// Pixels at 100 % → column width as stored in a file.
pub(crate) fn px_to_col_width(px: f32) -> f64 {
    ((px.max(0.0) as f64 / MAX_DIGIT_WIDTH) * 1e8).round() / 1e8
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
        // Values written by Excel for 64, 61, 145 and 100 pixel columns.
        assert_eq!(col_width_to_px(9.140625), 64.0);
        assert_eq!(col_width_to_px(8.7109375), 61.0);
        assert_eq!(col_width_to_px(20.7109375), 145.0);
        assert_eq!(col_width_to_px(14.28515625), 100.0);
        assert_eq!(col_width_to_px(0.0), 0.0);
        for px in [0.0f32, 1.0, 12.0, 64.0, 64.5, 77.25, 100.0, 255.0, 1800.0] {
            assert_eq!(col_width_to_px(px_to_col_width(px)), px);
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
