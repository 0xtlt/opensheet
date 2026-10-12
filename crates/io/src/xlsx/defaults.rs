//! What a workbook gives the columns and rows that were given nothing:
//! the sizes that follow from its default font, and `<sheetFormatPr>`.
//!
//! Files measure column widths in characters: multiples of the width of
//! the widest digit of the workbook's default font (the font of the Normal
//! style). A width of 10 is 70 pixels in a workbook set in Calibri 11 and
//! 80 pixels in one set in Calibri 12, so nothing about columns can be
//! read or written without knowing that font. The default row height is
//! the height of a line of it.
//!
//! Excel measures the font as Windows draws it at 96 dpi, hinting
//! included. The table below gives the same numbers for the fonts
//! workbooks are commonly set in; for an unusual font or size the result
//! may be a pixel off per character.

use super::xml::Tag;
use super::{col_width_to_px, points_to_px, px_to_col_width, px_to_points};
use opensheet_engine::style::{DEFAULT_FONT, DEFAULT_FONT_SIZE, Font};
use opensheet_engine::workbook::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, Sheet};
use std::fmt::Write as _;

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
pub(crate) struct Defaults {
    /// Width in pixels of the widest digit: the unit of column widths.
    pub digit_width: f64,
    /// Width of a column nobody sized, in pixels.
    pub col_width: f32,
    /// Height of a row nobody sized, in pixels.
    pub row_height: f32,
    /// Whether `row_height` is known to be what Excel shows, not an
    /// estimate.
    exact_row_height: bool,
}

/// Width in pixels of a column of `chars` digits: the digits, two pixels
/// of padding on each side and the grid line, rounded up to a multiple of
/// eight pixels as Excel does for its default width.
fn standard_col_width(chars: f64, digit_width: f64) -> f32 {
    let px = chars * digit_width + 5.0;
    (((px / 8.0).ceil() * 8.0) as f32).clamp(8.0, MAX_COL_PX)
}

impl Defaults {
    /// The sizes the engine uses for a workbook of its own.
    pub fn engine() -> Defaults {
        Defaults {
            digit_width: 7.0,
            col_width: DEFAULT_COL_WIDTH,
            row_height: DEFAULT_ROW_HEIGHT,
            exact_row_height: true,
        }
    }

    pub fn of(font: &Font) -> Defaults {
        if font.name.as_ref() == DEFAULT_FONT && font.size == DEFAULT_FONT_SIZE {
            return Defaults::engine();
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
        Defaults {
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
            let d = Defaults::of(&font(name, points));
            assert_eq!(d.digit_width, digit, "{name} {points}: digit");
            assert_eq!(d.col_width, col, "{name} {points}: column");
            if let Some(row) = row {
                assert_eq!(d.row_height, row, "{name} {points}: row");
            }
        }
        assert_eq!(Defaults::of(&Font::default()), Defaults::engine());
        // Absurd sizes stay within what the engine accepts.
        let huge = Defaults::of(&font("Arial", 409.0));
        assert!(huge.col_width <= MAX_COL_PX && huge.row_height <= MAX_ROW_PX);
        let tiny = Defaults::of(&font("Arial", 1.0));
        assert!(tiny.digit_width >= 1.0 && tiny.row_height >= 1.0);
    }
}
