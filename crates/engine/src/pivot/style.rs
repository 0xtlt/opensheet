//! PivotTable styles: the built-in "PivotStyleLight1" .. "PivotStyleDark28"
//! galleries, computed from the Office theme colours.
//!
//! Each gallery has four rows of seven styles: one neutral style followed
//! by one per accent colour. A style is a set of *looks* (fill, font,
//! borders) for the parts of a report; `style_patch_at` lays the looks of
//! the parts a cell belongs to over one another.

use super::compute::{Line, LineKind, Report};
use crate::style::{Border, BorderStyle, Color, StylePatch};
use crate::workbook::Workbook;

/// The style of a PivotTable and the "PivotTable Style Options"
/// (`pivotTableStyleInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotStyle {
    /// "PivotStyleLight16"; an empty or unknown name gives no style.
    pub name: String,
    /// "Row Headers": the label columns and the rows of items above their
    /// detail take the style's emphasis.
    pub row_headers: bool,
    /// "Column Headers".
    pub col_headers: bool,
    /// "Banded Rows".
    pub row_stripes: bool,
    /// "Banded Columns".
    pub col_stripes: bool,
}

impl Default for PivotStyle {
    fn default() -> PivotStyle {
        PivotStyle {
            name: "PivotStyleLight16".to_string(),
            row_headers: true,
            col_headers: true,
            row_stripes: false,
            col_stripes: false,
        }
    }
}

impl PivotStyle {
    /// The names of the built-in styles, in gallery order.
    pub fn builtin_names() -> Vec<String> {
        ["Light", "Medium", "Dark"]
            .iter()
            .flat_map(|family| (1..=28).map(move |n| format!("PivotStyle{family}{n}")))
            .collect()
    }

    /// What the style does to a small sample report, row by row: a header
    /// row, two items shown above two rows of detail each, and the grand
    /// total, over a column of labels and three columns of values. This is
    /// what a thumbnail of the styles gallery draws; an unknown style gives
    /// empty patches.
    pub fn sample(&self) -> Vec<Vec<StylePatch>> {
        use LineKind::{Grand, Group, Header, Item};
        let rows = [Header, Group, Item, Item, Group, Item, Item, Grand];
        let cols = [Header, Item, Item, Item];
        let report = Report {
            rows: rows.len() as u32,
            cols: cols.len() as u32,
            header_rows: 1,
            label_cols: 1,
            row_lines: rows
                .iter()
                .map(|kind| Line::sample(*kind, (*kind == Item) as u8))
                .collect(),
            col_lines: cols.iter().map(|kind| Line::sample(*kind, 0)).collect(),
            ..Report::default()
        };
        (0..report.rows)
            .map(|r| {
                (0..report.cols)
                    .map(|c| patch(self, &report, r, c).unwrap_or_default())
                    .collect()
            })
            .collect()
    }
}

/// Accent colours of the Office theme, and the dark colour the neutral
/// style of each row is built on.
const ACCENTS: [u32; 6] = [0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E];
const NEUTRAL: u32 = 0x404040;

/// Office's tint: positive values lighten towards white, negative ones
/// darken towards black, on the lightness of the colour.
fn tint(color: u32, amount: f32) -> Color {
    let (r, g, b) = (
        ((color >> 16) & 0xFF) as f32 / 255.0,
        ((color >> 8) & 0xFF) as f32 / 255.0,
        (color & 0xFF) as f32 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    let l = if amount < 0.0 {
        l * (1.0 + amount)
    } else {
        l * (1.0 - amount) + amount
    };
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    Color::rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// What a style does to one part of a report.
#[derive(Clone, Copy, Default)]
struct Look {
    fill: Option<Color>,
    font: Option<Color>,
    bold: bool,
    top: Option<Border>,
    bottom: Option<Border>,
    /// Lines on both sides (banded columns, grids).
    sides: Option<Border>,
}

impl Look {
    fn apply(&self, patch: &mut StylePatch) {
        if let Some(c) = self.fill {
            patch.fill = Some(Some(c));
        }
        if let Some(c) = self.font {
            patch.font_color = Some(Some(c));
        }
        if self.bold {
            patch.bold = Some(true);
        }
        if let Some(b) = self.top {
            patch.border_top = Some(b);
        }
        if let Some(b) = self.bottom {
            patch.border_bottom = Some(b);
        }
        if let Some(b) = self.sides {
            patch.border_left = Some(b);
            patch.border_right = Some(b);
        }
    }
}

/// The looks of a built-in style.
#[derive(Clone, Copy, Default)]
struct Built {
    /// Every cell of the report.
    whole: Look,
    /// The header rows.
    header: Look,
    /// The grand total row.
    total: Look,
    /// The label columns.
    labels: Look,
    /// Rows of items shown above their detail, outermost level first.
    heading: [Look; 2],
    /// Subtotal rows, outermost level first.
    subtotal: [Look; 2],
    /// Every other row, every other column of values.
    row_stripe: Look,
    col_stripe: Look,
}

fn line(style: BorderStyle, color: Color) -> Option<Border> {
    Some(Border {
        style,
        color: Some(color),
    })
}

/// The looks of "PivotStyle<family><number>".
fn builtin(name: &str) -> Option<Built> {
    let rest = name.strip_prefix("PivotStyle")?;
    let (family, number) = ["Light", "Medium", "Dark"]
        .iter()
        .enumerate()
        .find_map(|(k, f)| rest.strip_prefix(f).map(|n| (k, n)))?;
    let number: usize = number.parse().ok().filter(|n| (1..=28).contains(n))?;
    let (variant, column) = ((number - 1) / 7, (number - 1) % 7);
    let base = if column == 0 { NEUTRAL } else { ACCENTS[column - 1] };
    let solid = Color(base);
    let (t80, t60, t40) = (tint(base, 0.8), tint(base, 0.6), tint(base, 0.4));
    let (s25, s50) = (tint(base, -0.25), tint(base, -0.5));
    let (white, black) = (Color::WHITE, Color::BLACK);
    let bold = Look {
        bold: true,
        ..Look::default()
    };
    let filled = |fill: Color, font: Option<Color>| Look {
        fill: Some(fill),
        font,
        bold: true,
        ..Look::default()
    };
    let thin = |c: Color| line(BorderStyle::Thin, c);
    let mut b = Built {
        heading: [bold, Look::default()],
        subtotal: [bold, Look::default()],
        ..Built::default()
    };
    match (family, variant) {
        // Light 1-7: rules above and below, text in the accent colour.
        (0, 0) => {
            b.whole.font = (column > 0).then_some(s25);
            b.header = Look {
                bold: true,
                bottom: thin(solid),
                ..Look::default()
            };
            b.total = Look {
                bold: true,
                top: thin(solid),
                ..Look::default()
            };
            b.row_stripe.fill = Some(t80);
            b.col_stripe.fill = Some(t80);
        }
        // Light 8-14: a solid header.
        (0, 1) => {
            b.header = filled(solid, Some(white));
            b.total = Look {
                bold: true,
                top: line(BorderStyle::Double, solid),
                ..Look::default()
            };
            b.row_stripe = Look {
                top: thin(solid),
                bottom: thin(solid),
                ..Look::default()
            };
            b.col_stripe.sides = thin(solid);
        }
        // Light 15-21 (Light 16 is Excel's default): a pale header and
        // grand total, ruled off in a darker shade.
        (0, 2) => {
            b.header = Look {
                bottom: thin(t40),
                ..filled(t80, None)
            };
            b.total = Look {
                top: thin(t40),
                ..filled(t80, None)
            };
            b.row_stripe = Look {
                top: thin(t40),
                bottom: thin(t40),
                ..Look::default()
            };
            b.col_stripe.sides = thin(t40);
        }
        // Light 22-28: a grid.
        (0, _) => {
            b.whole = Look {
                top: thin(t40),
                bottom: thin(t40),
                sides: thin(t40),
                ..Look::default()
            };
            b.header = filled(t80, None);
            b.total = Look {
                bold: true,
                top: line(BorderStyle::Double, solid),
                ..Look::default()
            };
            b.labels = bold;
            b.row_stripe.fill = Some(t80);
            b.col_stripe.fill = Some(t80);
        }
        // Medium 1-7: a solid header over ruled rows.
        (1, 0) => {
            b.whole.bottom = thin(t40);
            b.header = filled(solid, Some(white));
            b.total = Look {
                bold: true,
                top: line(BorderStyle::Double, solid),
                ..Look::default()
            };
            b.labels = bold;
            b.row_stripe.fill = Some(t80);
            b.col_stripe.fill = Some(t80);
        }
        // Medium 8-14: tinted cells separated by white lines.
        (1, 1) => {
            b.whole = Look {
                fill: Some(t80),
                top: thin(white),
                bottom: thin(white),
                sides: thin(white),
                ..Look::default()
            };
            b.header = filled(solid, Some(white));
            b.total = filled(solid, Some(white));
            b.labels = bold;
            b.heading[0] = filled(t60, None);
            b.subtotal[0] = filled(t60, None);
            b.row_stripe.fill = Some(t60);
            b.col_stripe.fill = Some(t60);
        }
        // Medium 15-21: a black header, labels in the accent colour.
        (1, 2) => {
            b.header = filled(black, Some(white));
            b.total = Look {
                bold: true,
                top: line(BorderStyle::Double, black),
                ..Look::default()
            };
            b.labels = filled(solid, Some(white));
            b.heading[0] = filled(solid, Some(white));
            b.row_stripe.fill = Some(t80);
            b.col_stripe.fill = Some(t80);
        }
        // Medium 22-28: tinted cells in a grid of the accent colour.
        (1, _) => {
            b.whole = Look {
                fill: Some(t80),
                top: thin(t40),
                bottom: thin(t40),
                sides: thin(t40),
                ..Look::default()
            };
            b.header = filled(t60, None);
            b.total = filled(t60, None);
            b.labels = bold;
            b.heading[0] = filled(t60, None);
            b.row_stripe.fill = Some(t60);
            b.col_stripe.fill = Some(t60);
        }
        // Dark 1-7: white text on the darkened accent colour.
        (_, 0) => {
            b.whole = Look {
                fill: Some(s25),
                font: Some(white),
                ..Look::default()
            };
            b.header = filled(black, Some(white));
            b.total = Look {
                top: line(BorderStyle::Medium, white),
                ..filled(s50, Some(white))
            };
            b.labels = filled(s50, Some(white));
            b.heading[0] = filled(s50, Some(white));
            b.subtotal[0] = filled(s50, Some(white));
            b.row_stripe.fill = Some(s50);
            b.col_stripe.fill = Some(s50);
        }
        // Dark 8-14: a black header over tinted cells.
        (_, 1) => {
            b.whole.fill = Some(t80);
            b.header = filled(black, Some(white));
            b.total = Look {
                bold: true,
                top: line(BorderStyle::Double, black),
                ..Look::default()
            };
            b.labels = bold;
            b.heading[0] = filled(t60, None);
            b.row_stripe.fill = Some(t60);
            b.col_stripe.fill = Some(t60);
        }
        // Dark 15-21: darkened cells separated by white lines.
        (_, 2) => {
            b.whole = Look {
                fill: Some(s25),
                font: Some(white),
                top: thin(white),
                bottom: thin(white),
                sides: thin(white),
                ..Look::default()
            };
            b.header = filled(s50, Some(white));
            b.total = filled(s50, Some(white));
            b.labels = bold;
            b.heading[0] = filled(s50, Some(white));
            b.row_stripe.fill = Some(solid);
            b.col_stripe.fill = Some(solid);
        }
        // Dark 22-28: a solid header and total around mid-tone cells.
        _ => {
            b.whole = Look {
                fill: Some(t60),
                top: thin(white),
                bottom: thin(white),
                ..Look::default()
            };
            b.header = filled(solid, Some(white));
            b.total = filled(solid, Some(white));
            b.labels = bold;
            b.heading[0] = filled(t40, None);
            b.row_stripe.fill = Some(t40);
            b.col_stripe.fill = Some(t40);
        }
    }
    Some(b)
}

/// The looks of the parts the cell (`r`, `c`) of a report belongs to, the
/// more specific over the more general.
fn patch(style: &PivotStyle, report: &Report, r: u32, c: u32) -> Option<StylePatch> {
    let built = builtin(&style.name)?;
    let (rows, cols) = report.size();
    if report.placeholder || r >= rows || c >= cols {
        return None;
    }
    let row = &report.row_lines[r as usize];
    let col = &report.col_lines[c as usize];
    let header = r < report.header_rows;
    let label = c < report.label_cols;
    let mut out = StylePatch::default();
    built.whole.apply(&mut out);
    if !header {
        let item_row = matches!(row.kind, LineKind::Item | LineKind::Group);
        if style.col_stripes
            && !label
            && col.kind == LineKind::Item
            && item_row
            && (c - report.label_cols).is_multiple_of(2)
        {
            built.col_stripe.apply(&mut out);
        }
        if style.row_stripes && item_row && (r - report.header_rows).is_multiple_of(2) {
            built.row_stripe.apply(&mut out);
        }
        if style.row_headers {
            if label && row.kind != LineKind::Blank {
                built.labels.apply(&mut out);
            }
            if row.kind == LineKind::Group {
                built.heading[(row.level as usize).min(1)].apply(&mut out);
            }
        }
        match row.kind {
            LineKind::Subtotal => built.subtotal[(row.level as usize).min(1)].apply(&mut out),
            LineKind::Grand => built.total.apply(&mut out),
            _ => {}
        }
    } else if style.col_headers {
        built.header.apply(&mut out);
    }
    (!out.is_empty()).then_some(out)
}

/// The pivot style at a cell; `None` outside pivot tables. Called for
/// every visible cell at every repaint: it only reads the report computed
/// at the last update.
pub fn style_patch_at(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<StylePatch> {
    let sh = wb.sheets.get(sheet)?;
    let (p, body) = sh
        .pivots
        .iter()
        .find_map(|p| p.location.filter(|b| b.contains(row, col)).map(|b| (p, b)))?;
    patch(&p.style, &p.report, row - body.r0, col - body.c0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tints_follow_office() {
        // The "Lighter 80 %", "Lighter 40 %" and "Darker 25 %" swatches of
        // the first accent in Excel's colour picker.
        assert_eq!(tint(0x156082, 0.8).to_hex(), "C1E5F5");
        assert_eq!(tint(0x156082, 0.4).to_hex(), "46B1E1");
        assert_eq!(tint(0x156082, -0.25).to_hex(), "104862");
        assert_eq!(tint(0xFFFFFF, -0.5).to_hex(), "808080");
    }

    #[test]
    fn a_sample_shows_the_parts_of_a_style() {
        let mut style = PivotStyle::default();
        let sample = style.sample();
        assert_eq!((sample.len(), sample[0].len()), (8, 4));
        // Light 16: a pale header and grand total, bold items above their
        // detail, plain detail rows.
        let pale = Some(Some(tint(ACCENTS[0], 0.8)));
        assert_eq!(sample[0][2].fill, pale);
        assert_eq!(sample[7][0].fill, pale);
        assert_eq!(sample[1][0].bold, Some(true));
        assert!(sample[2][1].is_empty());
        // The options of the style show in its sample: every other row
        // below the header is banded.
        assert!(sample[3][1].is_empty());
        style.row_stripes = true;
        assert!(!style.sample()[3][1].is_empty());
        assert!(style.sample()[2][1].is_empty());
        style.name = "No such style".to_string();
        assert!(style.sample().iter().flatten().all(StylePatch::is_empty));
    }

    #[test]
    fn every_built_in_style_exists() {
        let names = PivotStyle::builtin_names();
        assert_eq!(names.len(), 84);
        assert!(names.iter().all(|n| builtin(n).is_some()));
        assert!(builtin("PivotStyleLight29").is_none());
        assert!(builtin("TableStyleMedium2").is_none());
        assert!(builtin("").is_none());
    }
}
