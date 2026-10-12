//! The sizes that follow from a font: the width of its digits, which is
//! the unit column widths are counted in, and the width of a column and
//! the height of a row nobody sized.
//!
//! Excel counts column widths in characters: multiples of the width of
//! the widest digit of the workbook's default font (the font of the Normal
//! style). A width of 10 is 70 pixels in a workbook set in Calibri 11 and
//! 80 pixels in one set in Calibri 12. The default row height is the
//! height of a line of that font.
//!
//! Excel measures the font as Windows draws it at 96 dpi, hinting
//! included. The table below gives the same numbers for the fonts
//! workbooks are commonly set in; for an unusual font or size the result
//! may be a pixel off per character.

use crate::style::{DEFAULT_FONT, DEFAULT_FONT_SIZE, Font};
use crate::workbook::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, Workbook};

/// Widest column and tallest row the engine keeps, in pixels.
const MAX_COL_PX: f32 = 1800.0;
const MAX_ROW_PX: f32 = 546.0;

/// Advance of the widest digit and height of a line (ascent + descent as
/// Windows takes them), both as fractions of the font size.
struct Face {
    /// Lower-case family names, including the native names East Asian
    /// versions of Excel write.
    names: &'static [&'static str],
    digit: f64,
    line: f64,
    /// Calibri and the font that took its place as Excel's default: same
    /// digits, same hinting, and the sizes Excel shows for them are known.
    calibri: bool,
}

const fn face(names: &'static [&'static str], digit: f64, line: f64) -> Face {
    Face {
        names,
        digit,
        line,
        calibri: false,
    }
}

const fn calibri(names: &'static [&'static str], line: f64) -> Face {
    Face {
        names,
        digit: 0.5068,
        line,
        calibri: true,
    }
}

/// Measured from the font files (`hmtx` and `OS/2` tables).
const FACES: &[Face] = &[
    calibri(&["calibri", "calibri light", "carlito"], 1.2207),
    calibri(&["aptos narrow"], 1.2847),
    face(&["aptos", "aptos display"], 0.5342, 1.2847),
    face(&["arial", "liberation sans", "arimo"], 0.5562, 1.1172),
    face(&["arial narrow"], 0.4561, 1.1323),
    face(&["arial black"], 0.6670, 1.4102),
    face(&["arial unicode ms"], 0.5562, 1.3398),
    face(&["helvetica", "helvetica neue"], 0.5562, 1.1753),
    face(&["microsoft sans serif", "ms sans serif"], 0.5562, 1.1318),
    face(&["verdana", "ms reference sans serif"], 0.6357, 1.2153),
    face(&["tahoma"], 0.5459, 1.2070),
    face(&["segoe ui"], 0.5391, 1.3301),
    face(&["trebuchet ms"], 0.5244, 1.1611),
    face(&["century gothic"], 0.5542, 1.1914),
    face(&["franklin gothic book"], 0.5864, 1.1338),
    face(&["gill sans mt"], 0.5000, 1.1597),
    face(&["tw cen mt"], 0.5518, 1.0889),
    face(&["lucida sans", "lucida sans unicode"], 0.6323, 1.1387),
    face(&["candara"], 0.5518, 1.2207),
    face(&["corbel"], 0.5244, 1.2207),
    face(&["comic sans ms"], 0.6104, 1.3936),
    face(&["impact"], 0.5415, 1.2197),
    face(&["open sans"], 0.5718, 1.3618),
    face(&["roboto"], 0.5615, 1.3184),
    face(
        &["times new roman", "liberation serif", "tinos"],
        0.5000,
        1.1074,
    ),
    face(&["cambria"], 0.5537, 1.1724),
    face(&["constantia"], 0.5464, 1.2207),
    face(&["georgia"], 0.6138, 1.1362),
    face(&["garamond"], 0.4688, 1.1250),
    face(&["palatino linotype"], 0.5000, 1.3491),
    face(&["book antiqua"], 0.5000, 1.2427),
    face(&["bookman old style"], 0.6201, 1.1250),
    face(&["century schoolbook"], 0.5562, 1.2017),
    face(&["rockwell"], 0.5420, 1.1743),
    face(
        &["courier new", "liberation mono", "cousine"],
        0.6001,
        1.1328,
    ),
    face(&["consolas"], 0.5498, 1.1709),
    face(
        &[
            "ms gothic",
            "ms pgothic",
            "ms ui gothic",
            "ｍｓ ゴシック",
            "ｍｓ ｐゴシック",
        ],
        0.5000,
        1.0000,
    ),
    face(&["yu gothic", "yu gothic ui", "游ゴシック"], 0.5562, 1.2871),
    face(&["meiryo", "meiryo ui", "メイリオ"], 0.6211, 1.5000),
    face(
        &[
            "simsun",
            "nsimsun",
            "宋体",
            "新宋体",
            "pmingliu",
            "mingliu",
            "新細明體",
            "細明體",
        ],
        0.5000,
        1.0000,
    ),
    face(&["dengxian", "等线"], 0.5269, 1.0420),
    face(&["microsoft yahei", "微软雅黑"], 0.5864, 1.3198),
    face(&["microsoft jhenghei", "微軟正黑體"], 0.5796, 1.3301),
    face(&["malgun gothic", "맑은 고딕"], 0.5508, 1.3301),
];

/// An ordinary sans serif, for the fonts the table does not know.
const UNKNOWN_FACE: Face = face(&[], 0.55, 1.2);

/// Row heights Excel gives the sizes of Calibri, in pixels: (size in
/// points, height).
const CALIBRI_ROWS: [(u16, f32); 9] = [
    (8, 15.0),
    (9, 16.0),
    (10, 17.0),
    (11, 20.0),
    (12, 21.0),
    (14, 25.0),
    (16, 28.0),
    (18, 31.0),
    (20, 35.0),
];

/// The sizes that follow from the default font of a workbook.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Width in pixels of the widest digit: the unit of column widths.
    pub digit_width: f64,
    /// Width of a column nobody sized, in pixels.
    pub col_width: f32,
    /// Height of a row nobody sized, in pixels.
    pub row_height: f32,
    /// Whether `row_height` is known to be what Excel shows, not an
    /// estimate.
    pub exact_row_height: bool,
}

/// Width in pixels of a column of `chars` digits: the digits, two pixels
/// of padding on each side and the grid line, rounded up to a multiple of
/// eight pixels as Excel does for its default width.
pub fn standard_col_width(chars: f64, digit_width: f64) -> f32 {
    let px = chars * digit_width + 5.0;
    (((px / 8.0).ceil() * 8.0) as f32).clamp(8.0, MAX_COL_PX)
}

impl FontMetrics {
    /// The sizes the engine uses for a workbook of its own.
    pub fn engine() -> FontMetrics {
        FontMetrics {
            digit_width: 7.0,
            col_width: DEFAULT_COL_WIDTH,
            row_height: DEFAULT_ROW_HEIGHT,
            exact_row_height: true,
        }
    }

    pub fn of(font: &Font) -> FontMetrics {
        if font.name.as_ref() == DEFAULT_FONT && font.size == DEFAULT_FONT_SIZE {
            return FontMetrics::engine();
        }
        let name = font.name.trim().to_lowercase();
        let face = FACES
            .iter()
            .find(|face| face.names.contains(&name.as_str()))
            .unwrap_or(&UNKNOWN_FACE);
        // Windows draws a font at a whole number of pixels per em.
        let half_points = font.size.max(2) as f64;
        let ppem = (half_points / 2.0 * 96.0 / 72.0 + 0.5).floor().max(1.0);
        // The hinting of Calibri narrows its digits at 11 points: 7
        // pixels, where the outline alone would give 8.
        let digit_width = if face.calibri && font.size == 22 {
            7.0
        } else {
            (face.digit * ppem + 0.5).floor().max(1.0)
        };
        let known_row = CALIBRI_ROWS
            .iter()
            .find(|(points, _)| face.calibri && *points * 2 == font.size)
            .map(|(_, px)| *px);
        let row_height = known_row
            .unwrap_or_else(|| ((face.line * ppem).ceil() + 2.0) as f32)
            .clamp(1.0, MAX_ROW_PX);
        FontMetrics {
            digit_width,
            col_width: standard_col_width(8.0, digit_width),
            row_height,
            exact_row_height: known_row.is_some(),
        }
    }

    /// Widest column a file can state: 255 characters (and no wider
    /// than the engine keeps).
    pub fn max_col_width(&self) -> f32 {
        ((255.0 * self.digit_width + 5.0) as f32).min(MAX_COL_PX)
    }
}


/// A column width as Excel shows it in the Column Width dialog and counts
/// it in `CELL("width")`: the number of digits of the default font that
/// fit between the paddings of the cell (5 pixels in all), to 1/100.
pub fn col_width_in_chars(px: f32, digit_width: f64) -> f64 {
    let chars = (px as f64 - 5.0) / digit_width.max(1.0);
    ((chars * 100.0 + 0.5).floor() / 100.0).max(0.0)
}

/// The width in pixels of a column of `chars` characters, the opposite of
/// [`col_width_in_chars`] (what typing a width in the dialog gives).
pub fn chars_to_col_width(chars: f64, digit_width: f64) -> f32 {
    if chars <= 0.0 {
        return 0.0;
    }
    ((chars * digit_width + 5.0).round() as f32).min(MAX_COL_PX)
}

impl Workbook {
    /// The sizes that follow from the default font of this workbook.
    pub fn font_metrics(&self) -> FontMetrics {
        FontMetrics::of(&self.styles.default_style().font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn font(name: &str, points: f32) -> Font {
        let mut font = Font {
            name: Arc::from(name),
            ..Font::default()
        };
        font.set_size_pt(points);
        font
    }

    #[test]
    fn sizes_of_common_default_fonts() {
        // (font, size, widest digit, default column width, row height):
        // what Excel shows for workbooks set in these fonts.
        let known = [
            ("Calibri", 11.0, 7.0, 64.0, Some(20.0)),
            ("Calibri", 12.0, 8.0, 72.0, Some(21.0)),
            ("Calibri", 10.0, 7.0, 64.0, Some(17.0)),
            ("Aptos Narrow", 11.0, 7.0, 64.0, Some(20.0)),
            ("Aptos Narrow", 12.0, 8.0, 72.0, Some(21.0)),
            ("Arial", 10.0, 7.0, 64.0, Some(17.0)),
            ("ARIAL", 10.0, 7.0, 64.0, Some(17.0)),
            ("Arial", 12.0, 9.0, 80.0, Some(20.0)),
            ("Liberation Sans", 10.0, 7.0, 64.0, Some(17.0)),
            ("Verdana", 10.0, 8.0, 72.0, None),
            ("Tahoma", 10.0, 7.0, 64.0, None),
            ("Times New Roman", 12.0, 8.0, 72.0, None),
            ("Courier New", 10.0, 8.0, 72.0, None),
            // The 8.38-character default of East Asian versions.
            ("ＭＳ Ｐゴシック", 11.0, 8.0, 72.0, None),
            ("游ゴシック", 11.0, 8.0, 72.0, None),
            ("宋体", 11.0, 8.0, 72.0, None),
            ("等线", 11.0, 8.0, 72.0, None),
            ("맑은 고딕", 11.0, 8.0, 72.0, Some(22.0)),
            ("Some Font Nobody Has", 11.0, 8.0, 72.0, Some(20.0)),
        ];
        for (name, points, digit, col, row) in known {
            let d = FontMetrics::of(&font(name, points));
            assert_eq!(d.digit_width, digit, "{name} {points}: digit");
            assert_eq!(d.col_width, col, "{name} {points}: column");
            if let Some(row) = row {
                assert_eq!(d.row_height, row, "{name} {points}: row");
            }
        }
        assert_eq!(FontMetrics::of(&Font::default()), FontMetrics::engine());
        // Absurd sizes stay within what the engine accepts.
        let huge = FontMetrics::of(&font("Arial", 409.0));
        assert!(huge.col_width <= MAX_COL_PX && huge.row_height <= MAX_ROW_PX);
        let tiny = FontMetrics::of(&font("Arial", 1.0));
        assert!(tiny.digit_width >= 1.0 && tiny.row_height >= 1.0);
    }

    #[test]
    fn widths_in_characters() {
        // What Excel shows for columns of 64, 100 and 145 pixels in a
        // workbook in Calibri 11, and for 72 pixels in Calibri 12.
        assert_eq!(col_width_in_chars(64.0, 7.0), 8.43);
        assert_eq!(col_width_in_chars(100.0, 7.0), 13.57);
        assert_eq!(col_width_in_chars(145.0, 7.0), 20.0);
        assert_eq!(col_width_in_chars(72.0, 8.0), 8.38);
        assert_eq!(col_width_in_chars(0.0, 7.0), 0.0);
        assert_eq!(col_width_in_chars(3.0, 7.0), 0.0);
        for (chars, digit, px) in [(8.43, 7.0, 64.0), (20.0, 7.0, 145.0), (10.0, 8.0, 85.0), (0.0, 7.0, 0.0)] {
            assert_eq!(chars_to_col_width(chars, digit), px);
        }
        assert_eq!(chars_to_col_width(1e9, 7.0), MAX_COL_PX);
    }

    #[test]
    fn cell_width_counts_digits_of_the_default_font() {
        let width_of_a = |set_in: Option<Font>| {
            let mut wb = Workbook::new(crate::Locale::en());
            if let Some(font) = set_in {
                let mut normal = wb.styles.default_style().clone();
                normal.font = font;
                wb.styles.set_default(normal);
            }
            wb.set_col_width(0, 0, Some(149.0));
            wb.set_cell_text(0, 0, 1, "=CELL(\"width\",A1)").unwrap();
            wb.set_cell_text(0, 1, 1, "=CELL(\"width\",C1)").unwrap();
            wb.commit("formulas");
            (wb.value_at(0, 0, 1), wb.value_at(0, 1, 1))
        };
        // 144 pixels of digits: 20.57 digits of 7 pixels, 18 of 8 pixels.
        // A column nobody sized has the width of the workbook's own font.
        let n = crate::Value::Number;
        assert_eq!(width_of_a(None), (n(21.0), n(8.0)));
        assert_eq!(width_of_a(Some(font("Calibri", 12.0))), (n(18.0), n(7.0)));
    }
}
