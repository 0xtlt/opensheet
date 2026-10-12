//! Tables ("Format as Table"): a named range with a header row, named
//! columns, an optional totals row, a filter and a style (`Sheet::tables`).
//!
//! The model is what structured references (`Table1[Column]`) are resolved
//! against. The rest of the file is the behaviour: creating, renaming,
//! resizing and deleting tables, their options and totals row, growing
//! when data is typed next to them, inserting and deleting table rows and
//! columns, calculated columns, and the built-in table styles.

use crate::ast::{Expr, StructuralEdit, TableCols, TableRef, TableRows, same_name};
use crate::features::CellShift;
use crate::filter::AutoFilter;
use crate::ops::shift_interval;
use crate::parser::ParseError;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Border, BorderStyle, Color, StyleId, StylePatch};
use crate::value::{ErrorKind, Value};
use crate::workbook::{Cell, ClearMode, Content, Sheet, Workbook};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

/// The style new tables get, as in Excel.
pub const DEFAULT_TABLE_STYLE: &str = "TableStyleMedium2";

/// What the totals row shows under a column (`totalsRowFunction`).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum TotalsFunction {
    #[default]
    None,
    Average,
    Count,
    CountNums,
    Max,
    Min,
    Sum,
    StdDev,
    Var,
    /// Any other formula. While the totals row is shown its cell holds the
    /// formula (see `Workbook::totals_formula`); this copy brings it back
    /// when the row is hidden and shown again.
    Custom(Arc<Expr>),
}

impl TotalsFunction {
    /// The function number of the `SUBTOTAL` written in the totals row:
    /// the 101-111 series, which leaves out rows hidden by hand too.
    pub fn subtotal_code(&self) -> Option<u32> {
        Some(match self {
            TotalsFunction::Average => 101,
            TotalsFunction::CountNums => 102,
            TotalsFunction::Count => 103,
            TotalsFunction::Max => 104,
            TotalsFunction::Min => 105,
            TotalsFunction::StdDev => 107,
            TotalsFunction::Sum => 109,
            TotalsFunction::Var => 110,
            TotalsFunction::None | TotalsFunction::Custom(_) => return None,
        })
    }

    pub fn from_subtotal_code(code: u32) -> Option<TotalsFunction> {
        Some(match code {
            101 => TotalsFunction::Average,
            102 => TotalsFunction::CountNums,
            103 => TotalsFunction::Count,
            104 => TotalsFunction::Max,
            105 => TotalsFunction::Min,
            107 => TotalsFunction::StdDev,
            109 => TotalsFunction::Sum,
            110 => TotalsFunction::Var,
            _ => return None,
        })
    }

    /// The value of `totalsRowFunction` in files.
    pub fn ooxml_name(&self) -> &'static str {
        match self {
            TotalsFunction::None => "none",
            TotalsFunction::Average => "average",
            TotalsFunction::Count => "count",
            TotalsFunction::CountNums => "countNums",
            TotalsFunction::Max => "max",
            TotalsFunction::Min => "min",
            TotalsFunction::Sum => "sum",
            TotalsFunction::StdDev => "stdDev",
            TotalsFunction::Var => "var",
            TotalsFunction::Custom(_) => "custom",
        }
    }

    /// The reverse of `ooxml_name`; "custom" needs its formula and gives
    /// `None`, like unknown names.
    pub fn from_ooxml_name(name: &str) -> Option<TotalsFunction> {
        Some(match name {
            "none" => TotalsFunction::None,
            "average" => TotalsFunction::Average,
            "count" => TotalsFunction::Count,
            "countNums" => TotalsFunction::CountNums,
            "max" => TotalsFunction::Max,
            "min" => TotalsFunction::Min,
            "sum" => TotalsFunction::Sum,
            "stdDev" => TotalsFunction::StdDev,
            "var" => TotalsFunction::Var,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableColumn {
    /// The header text; unique within the table, compared without case.
    pub name: String,
    /// What the totals row computes under the column.
    pub totals: TotalsFunction,
    /// Text the totals row shows under the column instead ("Total").
    pub totals_label: Option<String>,
}

impl TableColumn {
    pub fn new(name: impl Into<String>) -> TableColumn {
        TableColumn {
            name: name.into(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// Unique in the workbook.
    pub id: u32,
    /// Unique in the workbook, compared without case; a valid defined name
    /// ("Table1", "Tableau1").
    pub name: String,
    /// The whole table: header row, data rows and totals row.
    pub range: Rect,
    /// Whether the first row of `range` holds the column headers.
    pub header_row: bool,
    /// Whether the last row of `range` holds the totals.
    pub totals_row: bool,
    /// One per column of `range`, left to right (see
    /// `Workbook::insert_table` for tables built by hand).
    pub columns: Vec<TableColumn>,
    /// Name of the table style ("TableStyleMedium2"); `None` for no style.
    pub style: Option<String>,
    /// Table Design options (`showRowStripes`, `showColumnStripes`,
    /// `showFirstColumn`, `showLastColumn`).
    pub banded_rows: bool,
    pub banded_cols: bool,
    pub first_col: bool,
    pub last_col: bool,
    /// The table's own filter: `Some` while its buttons are shown, which
    /// needs the header row. Its range is the header and data rows.
    pub auto_filter: Option<AutoFilter>,
}

/// The check boxes of the Table Design tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableOption {
    HeaderRow,
    TotalsRow,
    BandedRows,
    BandedCols,
    FirstCol,
    LastCol,
    FilterButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableError {
    /// No such sheet, table or column.
    NotFound,
    /// The range leaves the sheet or is too small for the table.
    InvalidRange,
    /// "A table cannot overlap another table."
    Overlap,
    /// The range holds merged cells.
    MergedCells,
    /// Names start with a letter, `_` or `\`, go on with letters, digits,
    /// `.` and `_`, and cannot read as a cell reference.
    InvalidName,
    /// Another table or a defined name is already called that.
    NameTaken,
    /// "The table headers must remain in the same row, and the resulting
    /// table range must overlap the original table range."
    InvalidResize,
    /// "This operation is not allowed. The operation is attempting to
    /// shift cells in a table on your worksheet."
    Blocked,
    /// Filter buttons sit on the header row, which is hidden.
    NoHeaderRow,
}

/// The part of a table a cell is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableArea {
    Header,
    Data,
    Totals,
}

/// The table, column and area of a cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableCell<'a> {
    pub table: &'a Table,
    /// Index in `table.columns`.
    pub column: usize,
    pub area: TableArea,
}

impl TableCell<'_> {
    pub fn column_name(&self) -> &str {
        &self.table.columns[self.column].name
    }
}

/// What `Workbook::tables_after_edit` did, so the app can offer to undo it
/// ("Undo Table AutoExpansion", "Undo Calculated Column").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableEdit {
    /// The table (id) that grew over the edited cells.
    pub expanded: Option<u32>,
    /// A column was renamed by typing in its header.
    pub renamed: bool,
    /// The formula typed in a cell was copied to the rest of its column:
    /// table id and column index.
    pub filled: Option<(u32, usize)>,
}

impl Table {
    pub fn header_range(&self) -> Option<Rect> {
        self.header_row
            .then(|| Rect::new(self.range.r0, self.range.c0, self.range.r0, self.range.c1))
    }

    pub fn totals_range(&self) -> Option<Rect> {
        self.totals_row
            .then(|| Rect::new(self.range.r1, self.range.c0, self.range.r1, self.range.c1))
    }

    /// The data rows; `None` for a table without any.
    pub fn data_range(&self) -> Option<Rect> {
        let r0 = self.range.r0 + self.header_row as u32;
        let r1 = self.range.r1.checked_sub(self.totals_row as u32)?;
        (r0 <= r1).then(|| Rect::new(r0, self.range.c0, r1, self.range.c1))
    }

    /// Index of the column named `name`, compared without case.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        let wanted = name.to_lowercase();
        self.columns.iter().position(|c| c.name.to_lowercase() == wanted)
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.range.contains(row, col)
    }

    /// Index of the table column lying in sheet column `col`.
    pub fn column_at(&self, col: u32) -> Option<usize> {
        (self.range.c0 <= col && col <= self.range.c1).then(|| (col - self.range.c0) as usize)
    }

    /// The data cells of column `index`.
    pub fn column_range(&self, index: usize) -> Option<Rect> {
        let data = self.data_range()?;
        let col = self.range.c0 + index as u32;
        (col <= self.range.c1).then(|| Rect::new(data.r0, col, data.r1, col))
    }

    /// The state of a Table Design check box.
    pub fn option(&self, option: TableOption) -> bool {
        match option {
            TableOption::HeaderRow => self.header_row,
            TableOption::TotalsRow => self.totals_row,
            TableOption::BandedRows => self.banded_rows,
            TableOption::BandedCols => self.banded_cols,
            TableOption::FirstCol => self.first_col,
            TableOption::LastCol => self.last_col,
            TableOption::FilterButton => self.auto_filter.is_some(),
        }
    }

    /// The table's shape and options, as table styles need them.
    pub fn look(&self) -> TableLook {
        TableLook {
            rows: self.range.rows(),
            cols: self.range.cols(),
            header_row: self.header_row,
            totals_row: self.totals_row,
            banded_rows: self.banded_rows,
            banded_cols: self.banded_cols,
            first_col: self.first_col,
            last_col: self.last_col,
        }
    }

    /// Replace by `f(formula)` the totals formulas the table remembers
    /// while its totals row is hidden (`TotalsFunction::Custom`).
    pub(crate) fn map_formulas(&mut self, f: &mut dyn FnMut(&Expr) -> Expr) {
        for column in &mut self.columns {
            if let TotalsFunction::Custom(formula) = &mut column.totals {
                let mapped = f(formula);
                if mapped != **formula {
                    *formula = Arc::new(mapped);
                }
            }
        }
    }

    /// What the table's filter covers: the header and data rows.
    fn filter_range(&self) -> Rect {
        Rect::new(
            self.range.r0,
            self.range.c0,
            self.range.r1 - self.totals_row as u32,
            self.range.c1,
        )
    }

    /// Keep the filter on the table after it moved or changed size.
    fn sync_filter(&mut self) {
        if !self.header_row {
            self.auto_filter = None;
        }
        let range = self.filter_range();
        if let Some(filter) = &mut self.auto_filter {
            filter.range = range;
        }
    }
}

// ---- styles ---------------------------------------------------------------

/// The Office theme colours table styles are computed from, numbered as in
/// SpreadsheetML: 0 = light 1 (white), 1 = dark 1 (black), 2 = light 2,
/// 3 = dark 2, 4 to 9 = accents 1 to 6. The same theme as the colour
/// pickers and the chart palette.
pub const OFFICE_THEME: [u32; 10] = [
    0xFFFFFF, 0x000000, 0xE8E8E8, 0x0E2841, 0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E,
];

/// Lighten (`amount > 0`) or darken (`amount < 0`) a theme colour the way
/// Excel does: on the luminance of the Windows colour dialog's integer
/// HLS space (0..=240), which is why its shades differ by a unit or two
/// from a floating-point HSL computation.
pub fn tint(color: Color, amount: f64) -> Color {
    const HLS_MAX: i32 = 240;
    const RGB_MAX: i32 = 255;
    if amount == 0.0 || !amount.is_finite() {
        return color;
    }
    let (r, g, b) = (color.r() as i32, color.g() as i32, color.b() as i32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    let lum = (sum * HLS_MAX + RGB_MAX) / (2 * RGB_MAX);
    let (hue, sat) = if max == min {
        (0, 0)
    } else {
        let d = max - min;
        let sat = if lum <= HLS_MAX / 2 {
            (d * HLS_MAX + sum / 2) / sum
        } else {
            (d * HLS_MAX + (2 * RGB_MAX - sum) / 2) / (2 * RGB_MAX - sum)
        };
        let delta = |c: i32| ((max - c) * (HLS_MAX / 6) + d / 2) / d;
        let hue = if r == max {
            delta(b) - delta(g)
        } else if g == max {
            HLS_MAX / 3 + delta(r) - delta(b)
        } else {
            2 * HLS_MAX / 3 + delta(g) - delta(r)
        };
        (hue.rem_euclid(HLS_MAX), sat)
    };
    let amount = amount.clamp(-1.0, 1.0);
    let scaled = if amount < 0.0 {
        lum as f64 * (1.0 + amount)
    } else {
        lum as f64 * (1.0 - amount) + HLS_MAX as f64 * amount
    };
    // The bias keeps a nominal 80 % from landing a hair under a whole unit.
    let lum = ((scaled + 0.01) as i32).clamp(0, HLS_MAX);

    let byte = |v: i32| ((v * RGB_MAX + HLS_MAX / 2) / HLS_MAX).clamp(0, 255) as u8;
    if sat == 0 {
        let v = byte(lum);
        return Color::rgb(v, v, v);
    }
    let m2 = if lum <= HLS_MAX / 2 {
        (lum * (HLS_MAX + sat) + HLS_MAX / 2) / HLS_MAX
    } else {
        lum + sat - (lum * sat + HLS_MAX / 2) / HLS_MAX
    };
    let m1 = 2 * lum - m2;
    let channel = |h: i32| {
        let h = h.rem_euclid(HLS_MAX);
        if h < HLS_MAX / 6 {
            m1 + ((m2 - m1) * h + HLS_MAX / 12) / (HLS_MAX / 6)
        } else if h < HLS_MAX / 2 {
            m2
        } else if h < HLS_MAX * 2 / 3 {
            m1 + ((m2 - m1) * (HLS_MAX * 2 / 3 - h) + HLS_MAX / 12) / (HLS_MAX / 6)
        } else {
            m1
        }
    };
    Color::rgb(
        byte(channel(hue + HLS_MAX / 3)),
        byte(channel(hue)),
        byte(channel(hue - HLS_MAX / 3)),
    )
}

/// The formatting of one element of a table style (whole table, header
/// row, a stripe...): what the `dxf` of a `tableStyleElement` holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ElementStyle {
    pub bold: bool,
    pub font_color: Option<Color>,
    pub fill: Option<Color>,
    /// Borders around the element...
    pub left: Option<Border>,
    pub right: Option<Border>,
    pub top: Option<Border>,
    pub bottom: Option<Border>,
    /// ...and between its columns and its rows.
    pub vertical: Option<Border>,
    pub horizontal: Option<Border>,
}

impl ElementStyle {
    fn bold() -> ElementStyle {
        ElementStyle {
            bold: true,
            ..Default::default()
        }
    }

    fn filled(fill: Color) -> ElementStyle {
        ElementStyle {
            fill: Some(fill),
            ..Default::default()
        }
    }

    /// Bold text of colour `font` on a `fill` background.
    fn heading(font: Color, fill: Color) -> ElementStyle {
        ElementStyle {
            bold: true,
            font_color: Some(font),
            fill: Some(fill),
            ..Default::default()
        }
    }

    fn outlined(self, line: Option<Border>) -> ElementStyle {
        ElementStyle {
            left: line,
            right: line,
            top: line,
            bottom: line,
            ..self
        }
    }

    fn gridded(self, line: Option<Border>) -> ElementStyle {
        ElementStyle {
            vertical: line,
            horizontal: line,
            ..self
        }
    }

    /// Lay this element over `out` for a cell on the given edges of the
    /// element's area.
    fn lay(&self, out: &mut StylePatch, top: bool, bottom: bool, left: bool, right: bool) {
        if self.bold {
            out.bold = Some(true);
        }
        if let Some(color) = self.font_color {
            out.font_color = Some(Some(color));
        }
        if let Some(color) = self.fill {
            out.fill = Some(Some(color));
        }
        let side = |edge: bool, outer: Option<Border>, inner: Option<Border>, slot: &mut Option<Border>| {
            if let Some(border) = if edge { outer } else { inner } {
                *slot = Some(border);
            }
        };
        side(top, self.top, self.horizontal, &mut out.border_top);
        side(bottom, self.bottom, self.horizontal, &mut out.border_bottom);
        side(left, self.left, self.vertical, &mut out.border_left);
        side(right, self.right, self.vertical, &mut out.border_right);
    }
}

/// A table style: the elements built-in styles use, each band one row or
/// column wide.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableStyle {
    pub name: String,
    pub whole_table: ElementStyle,
    pub header_row: ElementStyle,
    pub total_row: ElementStyle,
    pub first_column: ElementStyle,
    pub last_column: ElementStyle,
    pub first_row_stripe: ElementStyle,
    pub second_row_stripe: ElementStyle,
    pub first_column_stripe: ElementStyle,
    pub second_column_stripe: ElementStyle,
}

/// The shape and options of a table, for `TableStyle::patch` (also what a
/// style thumbnail is drawn from).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableLook {
    /// Rows and columns of the whole table, header and totals included.
    pub rows: u32,
    pub cols: u32,
    pub header_row: bool,
    pub totals_row: bool,
    pub banded_rows: bool,
    pub banded_cols: bool,
    pub first_col: bool,
    pub last_col: bool,
}

impl TableStyle {
    /// What the style changes for the cell at (`row`, `col`), counted from
    /// the table's top-left corner. `band` is the cell's rank among the
    /// data rows that are shown: bands keep alternating when a filter
    /// hides rows.
    pub fn patch(&self, look: &TableLook, row: u32, col: u32, band: u32) -> StylePatch {
        let (last_row, last_col) = (look.rows.saturating_sub(1), look.cols.saturating_sub(1));
        let header = look.header_row && row == 0;
        let total = look.totals_row && row == last_row;
        let data = !header && !total;
        let (first, last) = (col == 0, col == last_col);
        let mut out = StylePatch::default();
        // Later elements win, in the order files define.
        self.whole_table.lay(&mut out, row == 0, row == last_row, first, last);
        if data && look.banded_cols {
            let stripe = if col % 2 == 0 {
                &self.first_column_stripe
            } else {
                &self.second_column_stripe
            };
            let top = row == look.header_row as u32;
            let bottom = row + look.totals_row as u32 == last_row;
            stripe.lay(&mut out, top, bottom, true, true);
        }
        if data && look.banded_rows {
            let stripe = if band % 2 == 0 {
                &self.first_row_stripe
            } else {
                &self.second_row_stripe
            };
            stripe.lay(&mut out, true, true, first, last);
        }
        if look.last_col && last {
            self.last_column.lay(&mut out, row == 0, row == last_row, true, true);
        }
        if look.first_col && first {
            self.first_column.lay(&mut out, row == 0, row == last_row, true, true);
        }
        if header {
            self.header_row.lay(&mut out, true, true, first, last);
        }
        if total {
            self.total_row.lay(&mut out, true, true, first, last);
        }
        out
    }
}

/// The built-in styles of the gallery, in its order: TableStyleLight1-21,
/// TableStyleMedium1-28, TableStyleDark1-11.
pub fn builtin_table_styles() -> &'static [TableStyle] {
    static STYLES: OnceLock<Vec<TableStyle>> = OnceLock::new();
    STYLES.get_or_init(|| build_styles(&OFFICE_THEME))
}

/// A built-in table style by name; `None` for names that are not built in
/// (custom styles of a file are kept by name but not drawn).
pub fn table_style(name: &str) -> Option<&'static TableStyle> {
    let families = [
        ("TableStyleLight", 0, 21),
        ("TableStyleMedium", 21, 28),
        ("TableStyleDark", 49, 11),
    ];
    let (number, first, count) = families
        .iter()
        .find_map(|(prefix, first, count)| Some((name.strip_prefix(prefix)?, *first, *count)))?;
    let number: usize = number.parse().ok()?;
    (1..=count)
        .contains(&number)
        .then(|| &builtin_table_styles()[first + number - 1])
}

/// The built-in styles for a theme, as Excel's preset table styles define
/// them. Within a family the first style is the neutral one (black, white
/// and greys) and the next six use the accents 1 to 6.
fn build_styles(theme: &[u32; 10]) -> Vec<TableStyle> {
    let color = |index: usize, amount: f64| tint(Color(theme[index]), amount);
    let (white, black) = (color(0, 0.0), color(1, 0.0));
    let line = |style: BorderStyle, color: Color| {
        Some(Border {
            style,
            color: Some(color),
        })
    };
    let thin = |c: Color| line(BorderStyle::Thin, c);
    let medium = |c: Color| line(BorderStyle::Medium, c);
    let thick = |c: Color| line(BorderStyle::Thick, c);
    let double = |c: Color| line(BorderStyle::Double, c);
    let plain = ElementStyle::default();
    let bold = ElementStyle::bold();
    // Variant k of a family: its main colour, and a shade of it (the
    // neutral variant shades white or black instead).
    let main = |k: usize| if k == 0 { black } else { color(3 + k, 0.0) };
    let shade = |k: usize, accent: f64, neutral: (usize, f64)| {
        if k == 0 {
            color(neutral.0, neutral.1)
        } else {
            color(3 + k, accent)
        }
    };
    let pale = |k: usize| shade(k, 0.8, (0, -0.15));
    let mid = |k: usize| shade(k, 0.6, (0, -0.35));
    let mut styles = Vec::with_capacity(60);

    // Light 1-7: coloured text between two thin rules.
    for k in 0..7 {
        let a = main(k);
        let text = (k > 0).then(|| color(3 + k, -0.25));
        let colored = |e: ElementStyle| ElementStyle { font_color: text, ..e };
        styles.push(TableStyle {
            whole_table: colored(ElementStyle {
                top: thin(a),
                bottom: thin(a),
                ..plain
            }),
            header_row: colored(ElementStyle {
                bottom: thin(a),
                ..bold
            }),
            total_row: colored(ElementStyle { top: thin(a), ..bold }),
            first_column: colored(bold),
            last_column: colored(bold),
            first_row_stripe: ElementStyle::filled(pale(k)),
            first_column_stripe: ElementStyle::filled(pale(k)),
            ..Default::default()
        });
    }
    // Light 8-14: solid header, outline, a rule between rows.
    for k in 0..7 {
        let a = main(k);
        let rule = ElementStyle { top: thin(a), ..plain };
        let bar = ElementStyle { left: thin(a), ..plain };
        styles.push(TableStyle {
            whole_table: plain.outlined(thin(a)),
            header_row: ElementStyle::heading(white, a),
            total_row: ElementStyle { top: double(a), ..bold },
            first_column: bold,
            last_column: bold,
            first_row_stripe: rule,
            second_row_stripe: rule,
            first_column_stripe: bar,
            second_column_stripe: bar,
            ..Default::default()
        });
    }
    // Light 15-21: a full grid, pale bands.
    for k in 0..7 {
        let a = main(k);
        styles.push(TableStyle {
            whole_table: plain.outlined(thin(a)).gridded(thin(a)),
            header_row: ElementStyle {
                bottom: medium(a),
                ..bold
            },
            total_row: ElementStyle { top: double(a), ..bold },
            first_column: bold,
            last_column: bold,
            first_row_stripe: ElementStyle::filled(pale(k)),
            first_column_stripe: ElementStyle::filled(pale(k)),
            ..Default::default()
        });
    }
    // Medium 1-7: solid header, pale bands, soft rules between rows.
    for k in 0..7 {
        let a = main(k);
        let edge = thin(shade(k, 0.4, (1, 0.0)));
        styles.push(TableStyle {
            whole_table: ElementStyle {
                horizontal: edge,
                ..plain.outlined(edge)
            },
            header_row: ElementStyle::heading(white, a),
            total_row: ElementStyle { top: double(a), ..bold },
            first_column: bold,
            last_column: bold,
            first_row_stripe: ElementStyle::filled(pale(k)),
            first_column_stripe: ElementStyle::filled(pale(k)),
            ..Default::default()
        });
    }
    // Medium 8-14: every cell filled, white grid, solid header and edges.
    for k in 0..7 {
        let a = main(k);
        let solid = ElementStyle::heading(white, a);
        styles.push(TableStyle {
            whole_table: ElementStyle::filled(pale(k)).gridded(thin(white)),
            header_row: ElementStyle {
                bottom: thick(white),
                ..solid
            },
            total_row: ElementStyle {
                top: thick(white),
                ..solid
            },
            first_column: solid,
            last_column: solid,
            first_row_stripe: ElementStyle::filled(mid(k)),
            first_column_stripe: ElementStyle::filled(mid(k)),
            ..Default::default()
        });
    }
    // Medium 15-21: solid header and edge columns between black rules,
    // grey bands; the neutral one adds a grid.
    for k in 0..7 {
        let a = main(k);
        let solid = ElementStyle::heading(white, a);
        let rules = ElementStyle {
            top: medium(black),
            bottom: medium(black),
            ..plain
        };
        let grey = ElementStyle::filled(color(0, -0.15));
        styles.push(TableStyle {
            whole_table: if k == 0 {
                ElementStyle {
                    left: thin(black),
                    right: thin(black),
                    ..rules.gridded(thin(black))
                }
            } else {
                rules
            },
            header_row: ElementStyle {
                bottom: medium(black),
                ..solid
            },
            total_row: ElementStyle {
                top: double(black),
                ..plain
            },
            first_column: solid,
            last_column: solid,
            first_row_stripe: grey,
            first_column_stripe: grey,
            ..Default::default()
        });
    }
    // Medium 22-28: every cell filled, a soft grid.
    for k in 0..7 {
        let edge = thin(shade(k, 0.4, (1, 0.0)));
        styles.push(TableStyle {
            whole_table: ElementStyle::filled(pale(k)).outlined(edge).gridded(edge),
            header_row: bold,
            total_row: ElementStyle {
                top: medium(main(k)),
                ..bold
            },
            first_column: bold,
            last_column: bold,
            first_row_stripe: ElementStyle::filled(mid(k)),
            first_column_stripe: ElementStyle::filled(mid(k)),
            ..Default::default()
        });
    }
    // Dark 1-7: white text on the colour, black header.
    for k in 0..7 {
        let body = shade(k, 0.0, (1, 0.45));
        let dark = shade(k, -0.25, (1, 0.25));
        let edge = ElementStyle::heading(white, dark);
        styles.push(TableStyle {
            whole_table: ElementStyle {
                font_color: Some(white),
                ..ElementStyle::filled(body)
            },
            header_row: ElementStyle {
                bottom: medium(white),
                ..ElementStyle::heading(white, black)
            },
            total_row: ElementStyle {
                top: medium(white),
                ..ElementStyle::heading(white, shade(k, -0.5, (1, 0.15)))
            },
            first_column: ElementStyle {
                right: medium(white),
                ..edge
            },
            last_column: ElementStyle {
                left: medium(white),
                ..edge
            },
            first_row_stripe: ElementStyle::filled(dark),
            first_column_stripe: ElementStyle::filled(dark),
            ..Default::default()
        });
    }
    // Dark 8-11: pale cells under a header of the next accent.
    for j in 0..4 {
        let (body, band, head) = if j == 0 {
            (color(0, -0.15), color(0, -0.35), black)
        } else {
            (color(2 + 2 * j, 0.8), color(2 + 2 * j, 0.6), color(3 + 2 * j, 0.0))
        };
        styles.push(TableStyle {
            whole_table: ElementStyle::filled(body),
            header_row: ElementStyle {
                bold: false,
                ..ElementStyle::heading(white, head)
            },
            total_row: ElementStyle {
                top: double(black),
                ..bold
            },
            first_column: bold,
            last_column: bold,
            first_row_stripe: ElementStyle::filled(band),
            first_column_stripe: ElementStyle::filled(band),
            ..Default::default()
        });
    }
    let names = (1..=21)
        .map(|n| format!("TableStyleLight{n}"))
        .chain((1..=28).map(|n| format!("TableStyleMedium{n}")))
        .chain((1..=11).map(|n| format!("TableStyleDark{n}")));
    for (style, name) in styles.iter_mut().zip(names) {
        style.name = name;
    }
    styles
}

/// The table style at a cell (header fill, banding, borders); `None`
/// outside tables. Called for every visible cell at every repaint.
pub fn style_patch_at(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<StylePatch> {
    let sh = wb.sheets.get(sheet)?;
    let table = sh.tables.iter().find(|t| t.contains(row, col))?;
    let style = table_style(table.style.as_deref()?)?;
    let band = match table.data_range() {
        Some(data) if table.banded_rows && data.r0 <= row && row <= data.r1 => {
            row - data.r0 - hidden_between(wb, sheet, sh, data.r0, row)
        }
        _ => 0,
    };
    let patch = style.patch(&table.look(), row - table.range.r0, col - table.range.c0, band);
    (!patch.is_empty()).then_some(patch)
}

/// How many of the rows `from..to` are not shown.
fn hidden_between(wb: &Workbook, index: usize, sheet: &Sheet, from: u32, to: u32) -> u32 {
    if sheet.hidden_rows.is_empty() && sheet.filtered_rows.is_empty() {
        return 0;
    }
    let rows = wb.caches.table.hidden_rows(wb.generation, index, sheet);
    (rows.partition_point(|r| *r < to) - rows.partition_point(|r| *r < from)) as u32
}

/// Cache of values derived from the cells (see `features::Caches`): per
/// sheet, the rows that are not shown, sorted, so that the band of a table
/// row is found by binary search rather than by walking the rows above it.
#[derive(Debug, Default)]
pub struct Cache {
    hidden: RwLock<HashMap<usize, HiddenRows>>,
}

#[derive(Debug)]
struct HiddenRows {
    /// What the list was built from: the recalculation it followed and the
    /// sizes of the two sets, so an edit not yet committed is noticed too.
    generation: u64,
    sizes: (usize, usize),
    rows: Arc<Vec<u32>>,
}

impl Cache {
    fn hidden_rows(&self, generation: u64, index: usize, sheet: &Sheet) -> Arc<Vec<u32>> {
        let sizes = (sheet.hidden_rows.len(), sheet.filtered_rows.len());
        if let Ok(map) = self.hidden.read() {
            if let Some(known) = map
                .get(&index)
                .filter(|h| h.generation == generation && h.sizes == sizes)
            {
                return known.rows.clone();
            }
        }
        let rows: Arc<Vec<u32>> = Arc::new(sheet.hidden_rows.union(&sheet.filtered_rows).copied().collect());
        if let Ok(mut map) = self.hidden.write() {
            map.insert(
                index,
                HiddenRows {
                    generation,
                    sizes,
                    rows: rows.clone(),
                },
            );
        }
        rows
    }
}

// ---- shifting cells ---------------------------------------------------------

/// True when shifting cells would tear another table or a merged area
/// that is not wholly inside the band.
fn tears(shift: &CellShift, sheet: &Sheet, own: Option<u32>) -> bool {
    let area = shift.area();
    sheet
        .tables
        .iter()
        .any(|t| Some(t.id) != own && t.range.intersects(&area))
        || sheet
            .merges
            .iter()
            .any(|m| m.intersects(&area) && !(area.contains(m.r0, m.c0) && area.contains(m.r1, m.c1)))
}

/// True when shifting cells is not allowed: it would tear a table, a
/// merged area or the sheet's filter.
fn blocked(shift: &CellShift, sheet: &Sheet, own: Option<u32>) -> bool {
    tears(shift, sheet, own)
        || sheet
            .auto_filter
            .as_ref()
            .is_some_and(|f| f.range.intersects(&shift.area()))
}

/// Shift cells of a sheet: move the cells, the merged areas inside the
/// band and the references to them, then let every feature follow (notes,
/// validation, conditional formats). The caller has taken the undo
/// snapshot.
fn shift_cells(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    let area = shift.area();
    let edited = wb.sheets[sheet].name.clone();
    let sh = &mut wb.sheets[sheet];
    let keys: Vec<(u32, u32)> = sh
        .cells
        .range((area.r0, 0)..=(area.r1, u32::MAX))
        .filter(|((_, c), _)| area.c0 <= *c && *c <= area.c1)
        .map(|(k, _)| *k)
        .collect();
    let lifted: Vec<((u32, u32), Cell)> = keys
        .into_iter()
        .filter_map(|k| sh.cells.remove(&k).map(|cell| (k, cell)))
        .collect();
    for ((row, col), cell) in lifted {
        let pos = if shift.down {
            shift.moved(row).map(|row| (row, col))
        } else {
            shift.moved(col).map(|col| (row, col))
        };
        if let Some(pos) = pos {
            sh.cells.insert(pos, cell);
        }
    }
    sh.merges = std::mem::take(&mut sh.merges)
        .into_iter()
        .filter_map(|m| {
            if !(area.contains(m.r0, m.c0) && area.contains(m.r1, m.c1)) {
                return Some(m);
            }
            let (lo, hi) = if shift.down { (m.r0, m.r1) } else { (m.c0, m.c1) };
            let (lo, hi) = shift_interval(lo, hi, shift.insert, shift.at, shift.count, shift.limit())?;
            let m = if shift.down {
                Rect::new(lo, m.c0, hi, m.c1)
            } else {
                Rect::new(m.r0, lo, m.r1, hi)
            };
            (!m.is_single()).then_some(m)
        })
        .collect();
    for sh in &mut wb.sheets {
        let own = sh.name.clone();
        for cell in sh.cells.values_mut() {
            if let Content::Formula(e) = &cell.content {
                let fixed = e.map_refs(&mut |r| shift.fix(r, &edited, &own));
                if fixed != **e {
                    cell.content = Content::Formula(Arc::new(fixed));
                }
            }
        }
        for def in sh.names.values_mut() {
            def.expr = Arc::new(def.expr.map_refs(&mut |r| shift.fix(r, &edited, &own)));
        }
        for chart in &mut sh.charts {
            chart.map_refs(&mut |r| match shift.fix(r, &edited, &own) {
                Expr::Ref(r) => Some(r),
                _ => None,
            });
        }
    }
    for def in wb.names.values_mut() {
        def.expr = Arc::new(def.expr.map_refs(&mut |r| shift.fix(r, &edited, &edited)));
    }
    crate::features::on_cells_shifted(wb, sheet, shift);
}

/// Give the cells of `blank` the conditional formats and the validation
/// rules of `model` (one row or one column of cells, copied over them), or
/// none with `None`: cells a table makes for itself must look like their
/// neighbours in the table, whatever the cells before them look like.
fn dress(wb: &mut Workbook, sheet: usize, blank: Rect, model: Option<Rect>) {
    match model {
        Some(model) => {
            wb.copy_cond_formats(sheet, model, sheet, blank);
            wb.tile_validation(sheet, model, sheet, &[blank]);
        }
        None => {
            wb.clear_cond_formats(sheet, &[blank]);
            wb.clear_validation(sheet, &[blank]);
        }
    }
}

// ---- names ------------------------------------------------------------------

/// Excel's rules for table names.
fn valid_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts = chars.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '\\');
    if !starts || !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') || name.chars().count() > 255 {
        return false;
    }
    // Neither an A1 nor an R1C1 reference ("R", "C", "R2C3", "RC").
    let upper = name.to_uppercase();
    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let r1c1 = match upper.strip_prefix('R') {
        Some(rest) => match rest.split_once('C') {
            Some((row, col)) => digits(row) && digits(col),
            None => digits(rest),
        },
        None => upper.strip_prefix('C').is_some_and(digits),
    };
    !r1c1 && crate::refs::parse_cell_name(name).is_none()
}

/// `stem` with the first number from `from` up that makes a name not yet
/// in `taken` (lowercase names).
fn numbered(stem: &str, from: usize, taken: &HashSet<String>) -> String {
    (from..)
        .map(|n| format!("{stem}{n}"))
        .find(|name| !taken.contains(&name.to_lowercase()))
        .unwrap_or_default()
}

/// Give every column a name, as Excel does when a table is made: blank
/// headers become "Column1"... (numbered by position) and repeated ones
/// get a number appended.
fn unique_names(raw: Vec<String>, prefix: &str) -> Vec<String> {
    let mut taken: HashSet<String> = raw
        .iter()
        .filter(|n| !n.trim().is_empty())
        .map(|n| n.to_lowercase())
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for (i, name) in raw.into_iter().enumerate() {
        let name = if name.trim().is_empty() {
            numbered(prefix, i + 1, &taken)
        } else if seen.contains(&name.to_lowercase()) {
            numbered(&name, 2, &taken)
        } else {
            name
        };
        taken.insert(name.to_lowercase());
        seen.insert(name.to_lowercase());
        out.push(name);
    }
    out
}

/// A column specifier inside a structured reference: `[`, `]`, `#` and `'`
/// are escaped with `'`.
fn escape_column(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '[' | ']' | '#' | '\'') {
            out.push('\'');
        }
        out.push(c);
    }
    out
}

/// `SUBTOTAL(code, Table[Column])`: what the totals row holds under a
/// column, as Excel writes it (shown as `=SUBTOTAL(109,[Column])` in the
/// table itself).
fn totals_expr(code: u32, table: &str, column: &str) -> Expr {
    let column = Expr::Table(TableRef {
        table: Some(Arc::from(table)),
        rows: TableRows::Unspecified,
        cols: TableCols::One(Arc::from(column)),
    });
    Expr::Func(Arc::from("SUBTOTAL"), vec![Expr::Number(code as f64), column])
}

/// The formula a column's cells share, as it reads in the first data row:
/// `Some` when every cell of `column` (but `skip`, the one being edited)
/// holds the same formula, relative references apart.
fn column_formula(sheet: &Sheet, column: Rect, skip: Option<u32>) -> Option<Expr> {
    let mut shared: Option<Expr> = None;
    for row in (column.r0..=column.r1).filter(|r| Some(*r) != skip) {
        let Content::Formula(e) = &sheet.cells.get(&(row, column.c0))?.content else {
            return None;
        };
        let first = e.shifted(column.r0 as i64 - row as i64, 0);
        match &shared {
            None => shared = Some(first),
            Some(other) if *other == first => {}
            Some(_) => return None,
        }
    }
    shared
}

/// The rows a filter hid stay with the table rows they are: when table
/// rows are inserted at `at` or deleted from there, the hidden rows from
/// `at` down to `last` (the table's last data row before the change) move
/// with them, without the criteria being looked at again.
fn shift_filtered_rows(sheet: &mut Sheet, at: u32, last: u32, count: u32, insert: bool) {
    // Rows added under the last one: nothing below them to move.
    if at > last {
        return;
    }
    let rows: Vec<u32> = sheet.filtered_rows.range(at..=last).copied().collect();
    for row in &rows {
        sheet.filtered_rows.remove(row);
    }
    for row in rows {
        if insert {
            sheet.filtered_rows.insert(row + count);
        } else if row >= at + count {
            sheet.filtered_rows.insert(row - count);
        }
    }
}

/// How many columns of a table's filter have a criterion.
fn criteria_of(table: &Table) -> usize {
    table.auto_filter.as_ref().map_or(0, |f| f.columns.len())
}

/// True when a row of table columns is free for the table to move into.
fn row_free(sheet: &Sheet, row: u32, c0: u32, c1: u32, own: u32) -> bool {
    let cells = Rect::new(row, c0, row, c1);
    row < MAX_ROWS
        && (c0..=c1).all(|c| sheet.is_blank(row, c))
        && !sheet.tables.iter().any(|t| t.id != own && t.range.intersects(&cells))
        && !sheet.merges.iter().any(|m| m.intersects(&cells))
}

/// Tables and merged cells may not share cells with a table.
fn check_free(sheet: &Sheet, range: &Rect, own: Option<u32>) -> Result<(), TableError> {
    if sheet
        .tables
        .iter()
        .any(|t| Some(t.id) != own && t.range.intersects(range))
    {
        return Err(TableError::Overlap);
    }
    if sheet.merges.iter().any(|m| m.intersects(range)) {
        return Err(TableError::MergedCells);
    }
    Ok(())
}

impl Workbook {
    // ---- lookup ---------------------------------------------------------

    /// The table named `name` (compared without case) and its sheet.
    pub fn table_by_name(&self, name: &str) -> Option<(usize, &Table)> {
        let wanted = name.to_lowercase();
        self.sheets.iter().enumerate().find_map(|(si, sh)| {
            sh.tables
                .iter()
                .find(|t| t.name.to_lowercase() == wanted)
                .map(|t| (si, t))
        })
    }

    /// The table a cell belongs to.
    pub fn table_at(&self, sheet: usize, row: u32, col: u32) -> Option<&Table> {
        self.sheets.get(sheet)?.tables.iter().find(|t| t.contains(row, col))
    }

    /// The table a formula typed in a cell belongs to, for its references
    /// without a table name (`[@Qty]`): the table holding the cell, or the
    /// one that grows over it once something is entered there (the cell is
    /// right under its last data row, or right beside its rows), which is
    /// how a formula typed next to a table starts a calculated column.
    pub fn table_for_entry(&self, sheet: usize, row: u32, col: u32) -> Option<&Table> {
        let sh = self.sheets.get(sheet)?;
        let free = |cells: Rect, own: u32| {
            !sh.tables.iter().any(|t| t.id != own && t.range.intersects(&cells))
                && !sh.merges.iter().any(|m| m.intersects(&cells))
        };
        self.table_at(sheet, row, col).or_else(|| {
            sh.tables.iter().find(|t| {
                let range = t.range;
                let last = range.r1 - t.totals_row as u32;
                let under = !t.totals_row && row == range.r1 + 1 && range.c0 <= col && col <= range.c1;
                let beside = col == range.c1 + 1 && range.r0 <= row && row <= last;
                (under && free(Rect::new(row, range.c0, row, range.c1), t.id))
                    || (beside && free(Rect::new(range.r0, col, last, col), t.id))
            })
        })
    }

    /// The table with id `id` on a sheet.
    pub fn table(&self, sheet: usize, id: u32) -> Option<&Table> {
        self.sheets.get(sheet)?.tables.iter().find(|t| t.id == id)
    }

    /// The table with id `id` and its sheet.
    pub fn table_by_id(&self, id: u32) -> Option<(usize, &Table)> {
        self.sheets
            .iter()
            .enumerate()
            .find_map(|(si, sh)| sh.tables.iter().find(|t| t.id == id).map(|t| (si, t)))
    }

    fn table_mut(&mut self, sheet: usize, id: u32) -> Option<&mut Table> {
        self.sheets.get_mut(sheet)?.tables.iter_mut().find(|t| t.id == id)
    }

    /// The table, column and area (header, data, totals) of a cell: what
    /// the formula editor needs to write a structured reference.
    pub fn table_cell(&self, sheet: usize, row: u32, col: u32) -> Option<TableCell<'_>> {
        let table = self.table_at(sheet, row, col)?;
        let area = if table.header_row && row == table.range.r0 {
            TableArea::Header
        } else if table.totals_row && row == table.range.r1 {
            TableArea::Totals
        } else {
            TableArea::Data
        };
        Some(TableCell {
            table,
            column: table.column_at(col)?,
            area,
        })
    }

    /// The structured reference Excel writes when `rect` is pointed at
    /// while typing a formula in cell `from` of the same sheet (`None`
    /// when the formula is on another sheet): `Table1[Sales]`,
    /// `Table1[[#Headers],[Sales]]`, `[@Sales]`... `None` when `rect` is
    /// not a whole part of a table, where a plain reference is written.
    pub fn structured_reference(&self, sheet: usize, rect: Rect, from: Option<(u32, u32)>) -> Option<String> {
        let table = self
            .table_at(sheet, rect.r0, rect.c0)
            .filter(|t| t.contains(rect.r1, rect.c1))?;
        let range = table.range;
        let data = table.data_range()?;
        let (first, last) = (table.column_at(rect.c0)?, table.column_at(rect.c1)?);
        let all_columns = first == 0 && last + 1 == table.columns.len();
        let column = |i: usize| escape_column(&table.columns[i].name);
        // "[A]:[B]" for several columns, "[A]" for one.
        let columns = if first == last {
            format!("[{}]", column(first))
        } else {
            format!("[{}]:[{}]", column(first), column(last))
        };
        let rows = (rect.r0, rect.r1);
        let item = if rows == (data.r0, data.r1) {
            None
        } else if table.header_row && rows == (range.r0, range.r0) {
            Some("#Headers")
        } else if table.totals_row && rows == (range.r1, range.r1) {
            Some("#Totals")
        } else if rows == (range.r0, range.r1) {
            Some("#All")
        } else if rect.r0 == rect.r1 && from.is_some_and(|(row, _)| row == rect.r0) {
            // The row of the formula itself: "[@Sales]" inside the table,
            // "Table1[@Sales]" beside it.
            let inside = from.is_some_and(|(row, col)| table.contains(row, col));
            let name = if inside { "" } else { table.name.as_str() };
            let simple = first == last
                && table.columns[first]
                    .name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_');
            return Some(if all_columns {
                format!("{name}[@]")
            } else if simple {
                format!("{name}[@{}]", column(first))
            } else {
                format!("{name}[@{columns}]")
            });
        } else {
            return None;
        };
        Some(match (item, all_columns) {
            (None, true) => table.name.clone(),
            (None, false) if first == last => format!("{}{columns}", table.name),
            (None, false) => format!("{}[{columns}]", table.name),
            (Some(item), true) => format!("{}[{item}]", table.name),
            (Some(item), false) => format!("{}[[{item}],{columns}]", table.name),
        })
    }

    // ---- creating, renaming, deleting --------------------------------------

    fn next_table_id(&self) -> u32 {
        self.sheets
            .iter()
            .flat_map(|s| s.tables.iter().map(|t| t.id))
            .max()
            .unwrap_or(0)
            + 1
    }

    fn name_taken(&self, name: &str, own: Option<u32>) -> bool {
        let lower = name.to_lowercase();
        self.names.contains_key(&name.to_uppercase())
            || self
                .sheets
                .iter()
                .flat_map(|s| &s.tables)
                .any(|t| Some(t.id) != own && t.name.to_lowercase() == lower)
    }

    /// The name a new table gets: "Table1", "Tableau2"...
    pub fn next_table_name(&self) -> String {
        (1u32..)
            .map(|n| format!("{}{n}", self.locale.table_prefix))
            .find(|name| !self.name_taken(name, None))
            .unwrap_or_default()
    }

    /// What the Create Table dialog proposes for a selection: the range (a
    /// single cell stands for the block of data around it) and whether its
    /// first row looks like headers.
    pub fn suggest_table_range(&self, sheet: usize, selection: Rect) -> (Rect, bool) {
        let range = if selection.is_single() {
            self.current_region(sheet, selection.r0, selection.c0)
        } else {
            self.clamp_to_used(sheet, selection)
        };
        (range, self.guess_header_row(sheet, range))
    }

    /// Make a table of `range`; returns its id. With `has_headers` the
    /// first row becomes the header row (blank or repeated headers are
    /// renamed, numbers and dates become text); without, the data moves
    /// down one row to make room for "Column1", "Column2"... A filter of
    /// the sheet on these cells becomes the table's.
    pub fn add_table(&mut self, sheet: usize, range: Rect, has_headers: bool) -> Result<u32, TableError> {
        let sh = self.sheets.get(sheet).ok_or(TableError::NotFound)?;
        let range = self.clamp_to_used(sheet, range);
        // A table has a header row and at least one data row.
        let extra = if has_headers { (range.r0 == range.r1) as u32 } else { 1 };
        if range.r1 + extra >= MAX_ROWS {
            return Err(TableError::InvalidRange);
        }
        let full = Rect::new(range.r0, range.c0, range.r1 + extra, range.c1);
        check_free(sh, &full, None)?;
        let replaces_filter = sh.auto_filter.as_ref().is_some_and(|f| f.range.intersects(&full));
        let room = CellShift {
            down: true,
            lo: range.c0,
            hi: range.c1,
            at: range.r0,
            count: 1,
            insert: true,
        };
        // The sheet's filter is in the way only when it is not the one
        // the table takes over.
        if !has_headers && (tears(&room, sh, None) || (!replaces_filter && blocked(&room, sh, None))) {
            return Err(TableError::Blocked);
        }
        self.snapshot();
        if replaces_filter {
            self.sheets[sheet].auto_filter = None;
        }
        if !has_headers {
            shift_cells(self, sheet, room);
            // The row made for the headers has no rule of the data.
            dress(self, sheet, Rect::new(range.r0, range.c0, range.r0, range.c1), None);
        }
        let headers = (full.c0..=full.c1)
            .map(|c| {
                if has_headers {
                    self.header_text(sheet, full.r0, c)
                } else {
                    String::new()
                }
            })
            .collect();
        let id = self.next_table_id();
        let table = Table {
            id,
            name: self.next_table_name(),
            range: full,
            header_row: true,
            columns: unique_names(headers, self.locale.column_prefix)
                .into_iter()
                .map(TableColumn::new)
                .collect(),
            style: Some(DEFAULT_TABLE_STYLE.to_string()),
            banded_rows: true,
            auto_filter: Some(AutoFilter::new(full)),
            ..Default::default()
        };
        self.sheets[sheet].tables.push(table);
        self.write_headers(sheet, id);
        if replaces_filter {
            self.refilter(sheet);
        }
        Ok(id)
    }

    /// Add a table read from a file (or built by hand) to a sheet; returns
    /// its id. The cells are left as they are: the header cells are taken
    /// to hold the column names already. What must hold for the rest of
    /// this module is made to hold: one column per column of the range,
    /// with distinct names; an id and a name no other table has (the
    /// table's own are kept when they are free); a filter only with a
    /// header row, on the header and data rows.
    pub fn insert_table(&mut self, sheet: usize, mut table: Table) -> Result<u32, TableError> {
        let sh = self.sheets.get(sheet).ok_or(TableError::NotFound)?;
        let range = table.range;
        if range.r1 >= MAX_ROWS || range.c1 >= MAX_COLS || table.data_range().is_none() {
            return Err(TableError::InvalidRange);
        }
        check_free(sh, &range, None)?;
        table.columns.resize_with(range.cols() as usize, TableColumn::default);
        let names = unique_names(
            table.columns.iter().map(|c| c.name.clone()).collect(),
            self.locale.column_prefix,
        );
        for (column, name) in table.columns.iter_mut().zip(names) {
            column.name = name;
        }
        if table.id == 0 || self.table_by_id(table.id).is_some() {
            table.id = self.next_table_id();
        }
        if !valid_table_name(&table.name) || self.name_taken(&table.name, None) {
            table.name = self.next_table_name();
        }
        if let Some(filter) = &mut table.auto_filter {
            filter.shift_columns(range.cols(), MAX_COLS, false);
        }
        table.sync_filter();
        let id = table.id;
        self.snapshot();
        self.sheets[sheet].tables.push(table);
        Ok(id)
    }

    /// Rename a table (Table Design > Table Name).
    pub fn rename_table(&mut self, sheet: usize, id: u32, name: &str) -> Result<(), TableError> {
        let name = name.trim();
        if self.table(sheet, id).ok_or(TableError::NotFound)?.name == name {
            return Ok(());
        }
        if !valid_table_name(name) {
            return Err(TableError::InvalidName);
        }
        if self.name_taken(name, Some(id)) {
            return Err(TableError::NameTaken);
        }
        self.snapshot();
        let mut old = String::new();
        if let Some(table) = self.table_mut(sheet, id) {
            old = std::mem::replace(&mut table.name, name.to_string());
        }
        // The formulas that named the table follow.
        self.rename_table_refs(&old, name);
        Ok(())
    }

    /// Delete a table with everything in its cells.
    pub fn delete_table(&mut self, sheet: usize, id: u32) -> bool {
        let Some(range) = self.table(sheet, id).map(|t| t.range) else {
            return false;
        };
        let name = self.table(sheet, id).map(|t| t.name.clone()).unwrap_or_default();
        self.snapshot();
        self.sheets[sheet].tables.retain(|t| t.id != id);
        self.table_refs_to_errors(&[name]);
        // First show the rows its filter hid, so they are cleared too.
        self.refilter(sheet);
        self.clear(sheet, range, ClearMode::All);
        true
    }

    /// Structured references to tables that are no more read `#REF!`, as
    /// in Excel. Takes the undo snapshot when a formula changes.
    fn table_refs_to_errors(&mut self, tables: &[String]) -> bool {
        if tables.is_empty() {
            return false;
        }
        let gone = |name: &str| tables.iter().any(|t| same_name(t, name));
        self.rewrite_formulas(&|wb, _, e| {
            Some(e.map(&mut |x| match x {
                Expr::Table(t) if t.table.as_deref().is_some_and(gone) => Expr::Error(ErrorKind::Ref),
                // The bare name of a table stands for its data.
                Expr::Name(n) if gone(&n) && wb.defined_name(&n).is_none() => Expr::Error(ErrorKind::Ref),
                other => other,
            }))
        })
    }

    /// Make the formulas follow columns of the table named `table` whose
    /// names changed: (old name, new name).
    fn rename_columns_in_formulas(&mut self, table: &str, renames: &[(String, String)]) {
        // When a new name is the old name of another column (two headers
        // swapped), no reference may be renamed twice: the columns go
        // through names no column has.
        let clash = renames
            .iter()
            .any(|(_, new)| renames.iter().any(|(old, _)| same_name(old, new)));
        if !clash {
            for (old, new) in renames {
                self.rename_table_column_refs(table, old, new);
            }
            return;
        }
        let parked: Vec<String> = (0..renames.len()).map(|i| format!("\u{1}{i}")).collect();
        for ((old, _), park) in renames.iter().zip(&parked) {
            self.rename_table_column_refs(table, old, park);
        }
        for ((_, new), park) in renames.iter().zip(&parked) {
            self.rename_table_column_refs(table, park, new);
        }
    }

    /// Table Design > Convert to Range: the cells keep their contents and
    /// the look the table style gave them; the table and its filter go.
    pub fn convert_table_to_range(&mut self, sheet: usize, id: u32) -> bool {
        let Some(range) = self.table(sheet, id).map(|t| t.range) else {
            return false;
        };
        let mut looks = Vec::new();
        for row in range.r0..=range.r1 {
            for col in range.c0..=range.c1 {
                if let Some(patch) = style_patch_at(self, sheet, row, col) {
                    looks.push((row, col, patch));
                }
            }
        }
        self.snapshot();
        for (row, col, patch) in looks {
            // As on screen, the cell's own formatting wins over the style.
            let id = self.sheets[sheet].style_id_at(row, col);
            let mut style = self.styles.get(id).clone();
            if style.fill.is_none() {
                style.fill = patch.fill.flatten();
            }
            if style.font.color.is_none() {
                style.font.color = patch.font_color.flatten();
            }
            style.font.bold |= patch.bold == Some(true);
            let borders = &mut style.borders;
            for (own, table) in [
                (&mut borders.left, patch.border_left),
                (&mut borders.right, patch.border_right),
                (&mut borders.top, patch.border_top),
                (&mut borders.bottom, patch.border_bottom),
            ] {
                if own.is_none() {
                    *own = table.unwrap_or_default();
                }
            }
            let new = self.styles.intern(style);
            if new != id {
                self.set_style_id(sheet, row, col, new);
            }
        }
        // Structured references become the plain ranges they stood for.
        let name = self.table(sheet, id).map(|t| t.name.clone()).unwrap_or_default();
        self.table_refs_to_ranges(&name);
        self.sheets[sheet].tables.retain(|t| t.id != id);
        self.refilter(sheet);
        true
    }

    // ---- look and options ---------------------------------------------------

    /// Pick a style from the gallery (`None` = no style).
    pub fn set_table_style(&mut self, sheet: usize, id: u32, style: Option<&str>) -> bool {
        if self.table(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            table.style = style.map(str::to_string);
        }
        true
    }

    /// Tick or untick a check box of the Table Design tab.
    pub fn set_table_option(&mut self, sheet: usize, id: u32, option: TableOption, on: bool) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        if table.option(option) == on {
            return Ok(());
        }
        match option {
            TableOption::HeaderRow => return self.set_header_row(sheet, id, on),
            TableOption::TotalsRow => return self.set_totals_row(sheet, id, on),
            TableOption::FilterButton if on && !table.header_row => return Err(TableError::NoHeaderRow),
            _ => {}
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            let range = table.filter_range();
            match option {
                TableOption::BandedRows => table.banded_rows = on,
                TableOption::BandedCols => table.banded_cols = on,
                TableOption::FirstCol => table.first_col = on,
                TableOption::LastCol => table.last_col = on,
                _ => table.auto_filter = on.then(|| AutoFilter::new(range)),
            }
        }
        if option == TableOption::FilterButton && !on {
            self.refilter(sheet);
        }
        Ok(())
    }

    /// Make the row at `row` free for the table by pushing the cells of its
    /// columns down from there.
    fn make_room(&mut self, sheet: usize, id: u32, row: u32) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let room = CellShift {
            down: true,
            lo: table.range.c0,
            hi: table.range.c1,
            at: row,
            count: 1,
            insert: true,
        };
        if row >= MAX_ROWS || blocked(&room, &self.sheets[sheet], Some(id)) {
            return Err(TableError::Blocked);
        }
        let freed = Rect::new(row, room.lo, row, room.hi);
        self.snapshot();
        shift_cells(self, sheet, room);
        // A header or a totals row has no rule of the data rows.
        dress(self, sheet, freed, None);
        Ok(())
    }

    fn set_header_row(&mut self, sheet: usize, id: u32, on: bool) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        if !on {
            // The header cells are emptied and leave the table; the names
            // stay in the columns.
            self.snapshot();
            self.clear(
                sheet,
                Rect::new(range.r0, range.c0, range.r0, range.c1),
                ClearMode::Contents,
            );
            if let Some(table) = self.table_mut(sheet, id) {
                table.header_row = false;
                table.range.r0 += 1;
                table.sync_filter();
            }
            self.refilter(sheet);
            return Ok(());
        }
        // The headers come back in the row above when it is free; else the
        // table moves down a row to let them in.
        let above = range
            .r0
            .checked_sub(1)
            .filter(|r| row_free(&self.sheets[sheet], *r, range.c0, range.c1, id));
        if above.is_none() {
            self.make_room(sheet, id, range.r0)?;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            match above {
                Some(row) => table.range.r0 = row,
                None => table.range.r1 += 1,
            }
            table.header_row = true;
            // The filter buttons come back with the row they sit on.
            let range = table.filter_range();
            table.auto_filter = Some(AutoFilter::new(range));
        }
        self.write_headers(sheet, id);
        self.write_totals(sheet, id);
        Ok(())
    }

    fn set_totals_row(&mut self, sheet: usize, id: u32, on: bool) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        if !on {
            // Remember the formulas typed in the row for the next time.
            let customs: Vec<(usize, Arc<Expr>)> = (0..table.columns.len())
                .filter(|i| matches!(table.columns[*i].totals, TotalsFunction::Custom(_)))
                .filter_map(|i| match &self.cell(sheet, range.r1, range.c0 + i as u32)?.content {
                    Content::Formula(e) => Some((i, e.clone())),
                    _ => None,
                })
                .collect();
            self.snapshot();
            self.clear(
                sheet,
                Rect::new(range.r1, range.c0, range.r1, range.c1),
                ClearMode::Contents,
            );
            if let Some(table) = self.table_mut(sheet, id) {
                for (i, formula) in customs {
                    table.columns[i].totals = TotalsFunction::Custom(formula);
                }
                table.totals_row = false;
                table.range.r1 -= 1;
                table.sync_filter();
            }
            return Ok(());
        }
        // The first time, as in Excel: "Total" under the first column and
        // the sum (or the count, for text) of the last one.
        let fresh = table
            .columns
            .iter()
            .all(|c| c.totals == TotalsFunction::None && c.totals_label.is_none());
        let last = table.columns.len() - 1;
        let numeric = table.column_range(last).is_some_and(|column| {
            (column.r0..=column.r1)
                .all(|r| matches!(self.value_at(sheet, r, column.c0), Value::Number(_) | Value::Empty))
        });
        let label = self.locale.total_label;
        let row = range.r1 + 1;
        if !row_free(&self.sheets[sheet], row, range.c0, range.c1, id) {
            self.make_room(sheet, id, row)?;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            table.totals_row = true;
            table.range.r1 = row;
            table.sync_filter();
            if fresh {
                table.columns[0].totals_label = Some(label.to_string());
                if last > 0 {
                    table.columns[last].totals = if numeric {
                        TotalsFunction::Sum
                    } else {
                        TotalsFunction::Count
                    };
                }
            }
        }
        self.write_totals(sheet, id);
        Ok(())
    }

    /// The text a header cell gives its column. A formula just typed there
    /// has no result yet, so it is worked out here: headers are plain
    /// text, the formula itself does not stay.
    fn header_text(&self, sheet: usize, row: u32, col: u32) -> String {
        let typed = self
            .formula_text(sheet, row, col)
            .and_then(|text| self.evaluate(sheet, &text).ok());
        match typed {
            Some(value) => {
                let format = &self.style_at(sheet, row, col).num_fmt;
                crate::format::format_value(&value.into_scalar(), format, self.locale).text
            }
            None => self.display_text(sheet, row, col).text,
        }
    }

    /// Write the column names in the header row, as text.
    fn write_headers(&mut self, sheet: usize, id: u32) {
        let Some(table) = self.table(sheet, id).filter(|t| t.header_row) else {
            return;
        };
        let (row, c0) = (table.range.r0, table.range.c0);
        let names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        for (i, name) in names.into_iter().enumerate() {
            self.set_content(sheet, row, c0 + i as u32, Content::Value(Value::text(name)));
        }
    }

    /// Write the totals row from the columns' settings. Formulas typed by
    /// the user (`Custom`) are left as they are.
    fn write_totals(&mut self, sheet: usize, id: u32) {
        let Some(table) = self.table(sheet, id).filter(|t| t.totals_row) else {
            return;
        };
        let (row, c0) = (table.range.r1, table.range.c0);
        let mut cells = Vec::with_capacity(table.columns.len());
        for (i, column) in table.columns.iter().enumerate() {
            let col = c0 + i as u32;
            let label = || match &column.totals_label {
                Some(label) => Content::Value(Value::text(label)),
                None => Content::Empty,
            };
            let content = match &column.totals {
                TotalsFunction::None => label(),
                TotalsFunction::Custom(formula) => match self.cell(sheet, row, col).map(|c| &c.content) {
                    Some(Content::Formula(_)) => continue,
                    _ => Content::Formula(formula.clone()),
                },
                function => match function.subtotal_code() {
                    Some(code) => Content::Formula(Arc::new(totals_expr(code, &table.name, &column.name))),
                    None => label(),
                },
            };
            cells.push((col, content));
        }
        for (col, content) in cells {
            self.set_content(sheet, row, col, content);
        }
    }

    // ---- totals row -----------------------------------------------------------

    /// Choose what the totals row computes under a column (the list in its
    /// cells: None, Average, Count...). For a formula of one's own, see
    /// `set_totals_formula`.
    pub fn set_totals_function(&mut self, sheet: usize, id: u32, column: usize, function: TotalsFunction) -> bool {
        if !self.table(sheet, id).is_some_and(|t| column < t.columns.len()) {
            return false;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            table.columns[column].totals = function;
            table.columns[column].totals_label = None;
        }
        // The cell is written afresh, whatever formula was typed in it.
        if let Some(table) = self.table(sheet, id).filter(|t| t.totals_row) {
            let (row, col) = (table.range.r1, table.range.c0 + column as u32);
            self.set_content(sheet, row, col, Content::Empty);
        }
        self.write_totals(sheet, id);
        true
    }

    /// Put text under a column in the totals row (`None` leaves it empty).
    pub fn set_totals_label(&mut self, sheet: usize, id: u32, column: usize, label: Option<&str>) -> bool {
        if !self.table(sheet, id).is_some_and(|t| column < t.columns.len()) {
            return false;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            table.columns[column].totals = TotalsFunction::None;
            table.columns[column].totals_label = label.map(str::to_string);
        }
        self.write_totals(sheet, id);
        true
    }

    /// Put a formula of one's own under a column in the totals row
    /// ("More Functions..."), written in the workbook's language.
    pub fn set_totals_formula(&mut self, sheet: usize, id: u32, column: usize, text: &str) -> Result<bool, ParseError> {
        if !self.table(sheet, id).is_some_and(|t| column < t.columns.len()) {
            return Ok(false);
        }
        let Some(name) = self.table(sheet, id).map(|t| t.name.clone()) else {
            return Ok(false);
        };
        let typed = crate::parser::parse_formula(text.strip_prefix('=').unwrap_or(text), self.locale)?;
        // Like every stored formula, it names the table it reads.
        let formula = Arc::new(typed.with_table_qualified(&name));
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            table.columns[column].totals = TotalsFunction::Custom(formula.clone());
            table.columns[column].totals_label = None;
        }
        if let Some(table) = self.table(sheet, id).filter(|t| t.totals_row) {
            let (row, col) = (table.range.r1, table.range.c0 + column as u32);
            self.set_content(sheet, row, col, Content::Formula(formula));
        }
        Ok(true)
    }

    /// The formula of a column's totals cell (`totalsRowFormula` for a
    /// custom one): the cell's while the totals row is shown, else what
    /// showing it would write.
    pub fn totals_formula(&self, sheet: usize, id: u32, column: usize) -> Option<Expr> {
        let table = self.table(sheet, id)?;
        let settings = table.columns.get(column)?;
        if table.totals_row {
            let cell = self.cell(sheet, table.range.r1, table.range.c0 + column as u32);
            if let Some(Content::Formula(e)) = cell.map(|c| &c.content) {
                return Some((**e).clone());
            }
        }
        match &settings.totals {
            TotalsFunction::Custom(formula) => Some((**formula).clone()),
            function => Some(totals_expr(function.subtotal_code()?, &table.name, &settings.name)),
        }
    }

    /// The formula of a calculated column, as it reads in the first data
    /// row (`calculatedColumnFormula`): `Some` when every data cell of the
    /// column holds the same formula.
    pub fn calculated_column_formula(&self, sheet: usize, id: u32, column: usize) -> Option<Expr> {
        let range = self.table(sheet, id)?.column_range(column)?;
        column_formula(&self.sheets[sheet], range, None)
    }

    // ---- size -----------------------------------------------------------------

    /// Table Design > Resize Table. The header row stays where it is and
    /// the new range must overlap the old one; columns that come in are
    /// named after their header cell, or "Column5"...
    pub fn resize_table(&mut self, sheet: usize, id: u32, range: Rect) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let old = table.range;
        let fixed = table.header_row as u32 + table.totals_row as u32;
        if range.r0 != old.r0 || !range.intersects(&old) {
            return Err(TableError::InvalidResize);
        }
        if range.rows() <= fixed || range.r1 >= MAX_ROWS || range.c1 >= MAX_COLS {
            return Err(TableError::InvalidRange);
        }
        check_free(&self.sheets[sheet], &range, Some(id))?;
        self.snapshot();
        self.apply_resize(sheet, id, range);
        Ok(())
    }

    /// Give a table a new range with the same top row. No checks, no
    /// snapshot: callers do both.
    fn apply_resize(&mut self, sheet: usize, id: u32, range: Rect) {
        let Some(table) = self.table(sheet, id) else {
            return;
        };
        let old = table.range;
        if table.totals_row && range.r1 != old.r1 {
            // The totals are written again under the new last row.
            self.clear(sheet, Rect::new(old.r1, old.c0, old.r1, old.c1), ClearMode::Contents);
        }
        let Some(table) = self.table(sheet, id) else {
            return;
        };
        // Columns keep their settings where the old and new ranges meet;
        // those coming in take the text of their header cell.
        let mut columns: Vec<TableColumn> = (range.c0..=range.c1)
            .map(|c| match table.column_at(c) {
                Some(i) => table.columns[i].clone(),
                None if table.header_row => TableColumn::new(self.header_text(sheet, range.r0, c)),
                None => TableColumn::default(),
            })
            .collect();
        let names = unique_names(
            columns.iter().map(|c| c.name.clone()).collect(),
            self.locale.column_prefix,
        );
        for (column, name) in columns.iter_mut().zip(names) {
            column.name = name;
        }
        // What the formulas must be told: the columns left outside, and
        // those a column coming in forced to take another name.
        let name = table.name.clone();
        let mut left = Vec::new();
        let mut renames = Vec::new();
        for (i, column) in table.columns.iter().enumerate() {
            let col = old.c0 + i as u32;
            if col < range.c0 || col > range.c1 {
                left.push(column.name.clone());
            } else if columns[(col - range.c0) as usize].name != column.name {
                renames.push((column.name.clone(), columns[(col - range.c0) as usize].name.clone()));
            }
        }
        let (criteria, last) = (criteria_of(table), table.filter_range().r1);
        for column in &left {
            self.delete_table_column_refs(&name, column);
        }
        let mut dropped = false;
        if let Some(table) = self.table_mut(sheet, id) {
            if let Some(filter) = &mut table.auto_filter {
                // Criteria follow their columns; those left outside go.
                if range.c0 > old.c0 {
                    filter.shift_columns(0, range.c0 - old.c0, false);
                } else {
                    filter.shift_columns(0, old.c0 - range.c0, true);
                }
                filter.shift_columns(range.cols(), MAX_COLS, false);
            }
            table.range = range;
            table.columns = columns;
            table.sync_filter();
            dropped = criteria_of(table) < criteria;
            // Rows that left the table are shown; rows that joined it stay
            // shown until the filter is applied again.
            let kept = table.filter_range().r1;
            if kept < last {
                shift_filtered_rows(&mut self.sheets[sheet], kept + 1, last, last - kept, false);
            }
        }
        self.rename_columns_in_formulas(&name, &renames);
        self.write_headers(sheet, id);
        self.write_totals(sheet, id);
        if dropped {
            self.refilter(sheet);
        }
    }

    /// Insert `count` table rows above sheet row `at` (a data row, or the
    /// row right under the last one to add at the end). Only the cells of
    /// the table's columns move down; calculated columns fill the new rows.
    pub fn table_insert_rows(&mut self, sheet: usize, id: u32, at: u32, count: u32) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        let first = range.r0 + table.header_row as u32;
        let end = range.r1 + 1 - table.totals_row as u32;
        if count == 0 || at < first || at > end || range.r1 + count >= MAX_ROWS {
            return Err(TableError::InvalidRange);
        }
        let shift = CellShift {
            down: true,
            lo: range.c0,
            hi: range.c1,
            at,
            count,
            insert: true,
        };
        let sh = &self.sheets[sheet];
        if blocked(&shift, sh, Some(id)) {
            return Err(TableError::Blocked);
        }
        let formulas: Vec<Option<Expr>> = (0..table.columns.len())
            .map(|i| column_formula(sh, table.column_range(i)?, None))
            .collect();
        self.snapshot();
        shift_cells(self, sheet, shift);
        // The new rows are shown, whatever the table's filter would say.
        shift_filtered_rows(&mut self.sheets[sheet], at, end - 1, count, true);
        if let Some(table) = self.table_mut(sheet, id) {
            table.range.r1 += count;
            table.sync_filter();
        }
        // The new rows look like the data row they were inserted under (or
        // over, at the top), down to its rules.
        let model = if at > first { at - 1 } else { at + count };
        dress(
            self,
            sheet,
            Rect::new(at, range.c0, at + count - 1, range.c1),
            Some(Rect::new(model, range.c0, model, range.c1)),
        );
        for (i, formula) in formulas.into_iter().enumerate() {
            let col = range.c0 + i as u32;
            let style = self.sheets[sheet].style_id_at(model, col);
            for row in at..at + count {
                if style != self.sheets[sheet].style_id_at(row, col) {
                    self.set_style_id(sheet, row, col, style);
                }
                if let Some(formula) = &formula {
                    self.set_formula(sheet, row, col, formula.shifted(row as i64 - first as i64, 0));
                }
            }
        }
        self.write_totals(sheet, id);
        Ok(())
    }

    /// Delete `count` table rows from sheet row `at`; the cells below move
    /// up in the table's columns only. A table keeps one data row: when
    /// they all go, the first is emptied instead.
    pub fn table_delete_rows(&mut self, sheet: usize, id: u32, at: u32, count: u32) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        let data = table.data_range().ok_or(TableError::InvalidRange)?;
        if count == 0 || at < data.r0 || at + count - 1 > data.r1 {
            return Err(TableError::InvalidRange);
        }
        let all = count == data.rows();
        let (at, count) = if all { (at + 1, count - 1) } else { (at, count) };
        let shift = CellShift {
            down: true,
            lo: range.c0,
            hi: range.c1,
            at,
            count,
            insert: false,
        };
        if count > 0 && blocked(&shift, &self.sheets[sheet], Some(id)) {
            return Err(TableError::Blocked);
        }
        self.snapshot();
        if all {
            self.sheets[sheet].filtered_rows.remove(&data.r0);
            self.clear(
                sheet,
                Rect::new(data.r0, range.c0, data.r0, range.c1),
                ClearMode::Contents,
            );
        }
        if count > 0 {
            shift_cells(self, sheet, shift);
            shift_filtered_rows(&mut self.sheets[sheet], at, data.r1, count, false);
            if let Some(table) = self.table_mut(sheet, id) {
                table.range.r1 -= count;
                table.sync_filter();
            }
        }
        self.write_totals(sheet, id);
        Ok(())
    }

    /// Insert `count` table columns to the left of sheet column `at` (a
    /// column of the table, or the one right after it to add at the end).
    /// Only the cells of the table's rows move right.
    pub fn table_insert_cols(&mut self, sheet: usize, id: u32, at: u32, count: u32) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        if count == 0 || at < range.c0 || at > range.c1 + 1 || range.c1 + count >= MAX_COLS {
            return Err(TableError::InvalidRange);
        }
        let shift = CellShift {
            down: false,
            lo: range.r0,
            hi: range.r1,
            at,
            count,
            insert: true,
        };
        if blocked(&shift, &self.sheets[sheet], Some(id)) {
            return Err(TableError::Blocked);
        }
        self.snapshot();
        shift_cells(self, sheet, shift);
        let prefix = self.locale.column_prefix;
        if let Some(table) = self.table_mut(sheet, id) {
            let index = (at - range.c0) as usize;
            table
                .columns
                .splice(index..index, (0..count).map(|_| TableColumn::default()));
            let names = unique_names(table.columns.iter().map(|c| c.name.clone()).collect(), prefix);
            for (column, name) in table.columns.iter_mut().zip(names) {
                column.name = name;
            }
            table.range.c1 += count;
            if let Some(filter) = &mut table.auto_filter {
                filter.shift_columns(at - range.c0, count, true);
            }
            table.sync_filter();
        }
        // The new columns look like the one on their left (or right, when
        // they come first), down to its rules.
        let model = if at > range.c0 { at - 1 } else { at + count };
        dress(
            self,
            sheet,
            Rect::new(range.r0, at, range.r1, at + count - 1),
            Some(Rect::new(range.r0, model, range.r1, model)),
        );
        for row in range.r0..=range.r1 {
            let style = self.sheets[sheet].style_id_at(row, model);
            for col in at..at + count {
                if style != self.sheets[sheet].style_id_at(row, col) {
                    self.set_style_id(sheet, row, col, style);
                }
            }
        }
        self.write_headers(sheet, id);
        self.write_totals(sheet, id);
        Ok(())
    }

    /// Delete `count` table columns from sheet column `at`; the cells to
    /// the right move left in the table's rows only. Deleting every column
    /// deletes the table.
    pub fn table_delete_cols(&mut self, sheet: usize, id: u32, at: u32, count: u32) -> Result<(), TableError> {
        let table = self.table(sheet, id).ok_or(TableError::NotFound)?;
        let range = table.range;
        if count == 0 || at < range.c0 || at + count - 1 > range.c1 {
            return Err(TableError::InvalidRange);
        }
        if count == range.cols() {
            self.delete_table(sheet, id);
            return Ok(());
        }
        let shift = CellShift {
            down: false,
            lo: range.r0,
            hi: range.r1,
            at,
            count,
            insert: false,
        };
        if blocked(&shift, &self.sheets[sheet], Some(id)) {
            return Err(TableError::Blocked);
        }
        let criteria = criteria_of(table);
        let name = table.name.clone();
        let doomed: Vec<String> = (at..at + count)
            .filter_map(|c| Some(table.columns[table.column_at(c)?].name.clone()))
            .collect();
        self.snapshot();
        // What read the columns reads `#REF!`, as in Excel.
        for column in &doomed {
            self.delete_table_column_refs(&name, column);
        }
        shift_cells(self, sheet, shift);
        let mut dropped = false;
        if let Some(table) = self.table_mut(sheet, id) {
            let index = (at - range.c0) as usize;
            table.columns.drain(index..index + count as usize);
            table.range.c1 -= count;
            if let Some(filter) = &mut table.auto_filter {
                filter.shift_columns(at - range.c0, count, false);
            }
            table.sync_filter();
            dropped = criteria_of(table) < criteria;
        }
        self.write_totals(sheet, id);
        // A criterion went with its column: show the rows it hid.
        if dropped {
            self.refilter(sheet);
        }
        Ok(())
    }

    // ---- following edits ------------------------------------------------------

    /// To call after cells were typed, pasted or cleared (`edited` is the
    /// rectangle written), before committing. As Excel does, a table grows
    /// over data entered in the row right under it or the column right
    /// beside it, text typed in a header cell renames the column, what is
    /// typed in the totals row becomes the column's total, and a formula
    /// entered in a column that is empty (or already calculated) fills the
    /// column.
    pub fn tables_after_edit(&mut self, sheet: usize, edited: Rect) -> TableEdit {
        let mut out = TableEdit::default();
        let Some(sh) = self.sheets.get(sheet).filter(|s| !s.tables.is_empty()) else {
            return out;
        };
        let edited = self.clamp_to_used(sheet, edited);
        let ids: Vec<u32> = sh.tables.iter().map(|t| t.id).collect();
        for id in ids {
            if let Some(range) = self.grown_range(sheet, id, edited) {
                self.grow_table(sheet, id, range);
                out.expanded = Some(id);
            }
            out.renamed |= self.sync_headers(sheet, id, edited);
            self.sync_totals(sheet, id, edited);
            if let Some(column) = self.fill_calculated_column(sheet, id, edited) {
                out.filled = Some((id, column));
            }
        }
        out
    }

    /// The range a table takes when data was just entered against its
    /// bottom or right edge.
    fn grown_range(&self, sheet: usize, id: u32, edited: Rect) -> Option<Rect> {
        let sh = &self.sheets[sheet];
        let table = self.table(sheet, id)?;
        let mut range = table.range;
        let filled = |cells: Rect| {
            let entered = cells
                .intersection(&edited)
                .is_some_and(|hit| (hit.r0..=hit.r1).any(|r| (hit.c0..=hit.c1).any(|c| !sh.is_blank(r, c))));
            entered
                && !sh.tables.iter().any(|t| t.id != id && t.range.intersects(&cells))
                && !sh.merges.iter().any(|m| m.intersects(&cells))
        };
        // Rows under the table, unless its totals row is in the way.
        while !table.totals_row
            && range.r1 + 1 < MAX_ROWS
            && filled(Rect::new(range.r1 + 1, range.c0, range.r1 + 1, range.c1))
        {
            range.r1 += 1;
        }
        // Columns beside the header and data rows.
        let last = range.r1 - table.totals_row as u32;
        while range.c1 + 1 < MAX_COLS && filled(Rect::new(range.r0, range.c1 + 1, last, range.c1 + 1)) {
            range.c1 += 1;
        }
        (range != table.range).then_some(range)
    }

    /// Extend a table over rows and columns just entered: the new rows
    /// take the formats, the rules (conditional formats, validation) and
    /// the calculated columns of the last data row.
    fn grow_table(&mut self, sheet: usize, id: u32, range: Rect) {
        let Some(table) = self.table(sheet, id) else {
            return;
        };
        let old = table.range;
        let sh = &self.sheets[sheet];
        let first = old.r0 + table.header_row as u32;
        let last = table.filter_range().r1;
        let formulas: Vec<Option<Expr>> = (0..table.columns.len())
            .map(|i| column_formula(sh, table.column_range(i)?, None))
            .collect();
        self.snapshot();
        self.apply_resize(sheet, id, range);
        // Rows that join the table take the rules of its last data row
        // with its formats: a list or a highlight goes on down the column.
        if range.r1 > old.r1 {
            dress(
                self,
                sheet,
                Rect::new(old.r1 + 1, old.c0, range.r1, old.c1),
                Some(Rect::new(last, old.c0, last, old.c1)),
            );
        }
        for (i, formula) in formulas.into_iter().enumerate() {
            let col = old.c0 + i as u32;
            let style = self.sheets[sheet].style_id_at(last, col);
            for row in old.r1 + 1..=range.r1 {
                let sh = &self.sheets[sheet];
                let blank = sh.is_blank(row, col);
                if style != StyleId(0) && sh.style_id_at(row, col) == StyleId(0) {
                    self.set_style_id(sheet, row, col, style);
                }
                if let Some(formula) = formula.as_ref().filter(|_| blank) {
                    self.set_formula(sheet, row, col, formula.shifted(row as i64 - first as i64, 0));
                }
            }
        }
    }

    /// Header cells that were edited give their text to their columns: a
    /// blank one takes the default name for its position, a name another
    /// column already has gets a number.
    fn sync_headers(&mut self, sheet: usize, id: u32, edited: Rect) -> bool {
        let Some(table) = self.table(sheet, id) else {
            return false;
        };
        let Some(hit) = table.header_range().and_then(|h| h.intersection(&edited)) else {
            return false;
        };
        let prefix = self.locale.column_prefix;
        let mut names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let edited = (hit.c0 - table.range.c0) as usize..=(hit.c1 - table.range.c0) as usize;
        let texts: Vec<String> = (hit.c0..=hit.c1).map(|col| self.header_text(sheet, hit.r0, col)).collect();
        // The other columns keep their names. Headers entered together (a
        // paste) are judged together: the names they replace are free,
        // and a default name does not take one of those just entered.
        let mut taken: HashSet<String> = names
            .iter()
            .enumerate()
            .filter(|(j, _)| !edited.contains(j))
            .map(|(_, n)| n.to_lowercase())
            .collect();
        let mut reserved = taken.clone();
        reserved.extend(texts.iter().filter(|t| !t.trim().is_empty()).map(|t| t.to_lowercase()));
        for (i, text) in edited.zip(texts) {
            let name = if text.trim().is_empty() {
                numbered(prefix, i + 1, &reserved)
            } else if taken.contains(&text.to_lowercase()) {
                numbered(&text, 2, &reserved)
            } else {
                text
            };
            taken.insert(name.to_lowercase());
            reserved.insert(name.to_lowercase());
            names[i] = name;
        }
        let renames: Vec<(String, String)> = table
            .columns
            .iter()
            .zip(&names)
            .filter(|(c, n)| c.name != **n)
            .map(|(c, n)| (c.name.clone(), n.clone()))
            .collect();
        let renamed = !renames.is_empty();
        if renamed {
            let table_name = table.name.clone();
            self.snapshot();
            if let Some(table) = self.table_mut(sheet, id) {
                for (column, name) in table.columns.iter_mut().zip(names) {
                    column.name = name;
                }
            }
            // The formulas that named the columns follow.
            self.rename_columns_in_formulas(&table_name, &renames);
        }
        // Headers are text, whatever was typed.
        self.write_headers(sheet, id);
        renamed
    }

    /// Totals cells that were edited become their columns' settings: a
    /// `SUBTOTAL` over the column is one of the listed functions, another
    /// formula a custom one, anything else a label.
    fn sync_totals(&mut self, sheet: usize, id: u32, edited: Rect) {
        let Some(table) = self.table(sheet, id) else {
            return;
        };
        let Some(hit) = table.totals_range().and_then(|t| t.intersection(&edited)) else {
            return;
        };
        let mut settings = Vec::new();
        for col in hit.c0..=hit.c1 {
            let i = (col - table.range.c0) as usize;
            let (totals, label) = match self.cell(sheet, hit.r0, col).map(|c| &c.content) {
                Some(Content::Formula(e)) => {
                    // The column itself, as `[Column]` or as its cells.
                    let whole_column = |over: &Expr| match over {
                        Expr::Ref(r) => r.sheet.is_none() && Some(r.rect()) == table.column_range(i),
                        Expr::Table(t) => {
                            t.is_to(&table.name, true)
                                && matches!(t.rows, TableRows::Unspecified | TableRows::Data)
                                && matches!(&t.cols, TableCols::One(c) if same_name(c, &table.columns[i].name))
                        }
                        _ => false,
                    };
                    let listed = match &**e {
                        Expr::Func(name, args) if &**name == "SUBTOTAL" => match args.as_slice() {
                            [Expr::Number(code), over] if whole_column(over) => {
                                TotalsFunction::from_subtotal_code(*code as u32)
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    (listed.unwrap_or_else(|| TotalsFunction::Custom(e.clone())), None)
                }
                Some(Content::Value(_)) => (TotalsFunction::None, Some(self.display_text(sheet, hit.r0, col).text)),
                _ => (TotalsFunction::None, None),
            };
            let column = &table.columns[i];
            if column.totals != totals || column.totals_label != label {
                settings.push((i, totals, label));
            }
        }
        if settings.is_empty() {
            return;
        }
        self.snapshot();
        if let Some(table) = self.table_mut(sheet, id) {
            for (i, totals, label) in settings {
                table.columns[i].totals = totals;
                table.columns[i].totals_label = label;
            }
        }
    }

    /// A formula entered in one data cell fills its column when the other
    /// cells are empty, or all hold another formula (a calculated column
    /// being changed). Returns the column's index when it was filled.
    fn fill_calculated_column(&mut self, sheet: usize, id: u32, edited: Rect) -> Option<usize> {
        if !edited.is_single() {
            return None;
        }
        let (row, col) = (edited.r0, edited.c0);
        let table = self.table(sheet, id)?;
        let index = table.column_at(col)?;
        let column = table
            .column_range(index)
            .filter(|c| c.r0 <= row && row <= c.r1 && c.r0 < c.r1)?;
        let sh = &self.sheets[sheet];
        let Content::Formula(formula) = &sh.cells.get(&(row, col))?.content else {
            return None;
        };
        let formula = formula.shifted(column.r0 as i64 - row as i64, 0);
        let others_blank = (column.r0..=column.r1).all(|r| r == row || sh.is_blank(r, col));
        let others_differ = column_formula(sh, column, Some(row)).is_some_and(|other| other != formula);
        if !others_blank && !others_differ {
            return None;
        }
        for r in (column.r0..=column.r1).filter(|r| *r != row) {
            self.set_formula(sheet, r, col, formula.shifted(r as i64 - column.r0 as i64, 0));
        }
        Some(index)
    }
}

// ---- hooks --------------------------------------------------------------------

/// A table after rows or columns were inserted into or deleted from its
/// sheet; `None` when nothing is left of its data.
fn adjusted(mut table: Table, edit: StructuralEdit) -> Option<Table> {
    let range = table.range;
    match edit {
        StructuralEdit::InsertRows { at, count } => {
            // Rows inserted at the header push the table down; rows
            // inserted further in become data rows.
            if at <= range.r0 {
                if range.r0 + count >= MAX_ROWS {
                    return None;
                }
                table.range.r0 += count;
            }
            if at <= range.r1 {
                table.range.r1 = (range.r1 + count).min(MAX_ROWS - 1);
            }
        }
        StructuralEdit::DeleteRows { at, count } => {
            let gone = |row: u32| at <= row && row < at + count;
            let (r0, r1) = shift_interval(range.r0, range.r1, false, at, count, MAX_ROWS)?;
            if table.header_row && gone(range.r0) {
                table.header_row = false;
            }
            if table.totals_row && gone(range.r1) {
                table.totals_row = false;
            }
            table.range = Rect::new(r0, range.c0, r1, range.c1);
        }
        StructuralEdit::InsertCols { at, count } => {
            if at <= range.c0 {
                let (c0, c1) = shift_interval(range.c0, range.c1, true, at, count, MAX_COLS)?;
                table.range = Rect::new(range.r0, c0, range.r1, c1);
            } else if at <= range.c1 {
                // New columns are named once the workbook's language is at
                // hand, in `after_structural_edit`.
                let count = count.min(MAX_COLS - 1 - range.c1);
                let index = (at - range.c0) as usize;
                table
                    .columns
                    .splice(index..index, (0..count).map(|_| TableColumn::default()));
                table.range.c1 += count;
                if let Some(filter) = &mut table.auto_filter {
                    filter.shift_columns(at - range.c0, count, true);
                }
            }
        }
        StructuralEdit::DeleteCols { at, count } => {
            let (c0, c1) = shift_interval(range.c0, range.c1, false, at, count, MAX_COLS)?;
            let (first, last) = (at.max(range.c0), (at + count - 1).min(range.c1));
            if first <= last {
                let index = (first - range.c0) as usize;
                table.columns.drain(index..=index + (last - first) as usize);
                if let Some(filter) = &mut table.auto_filter {
                    filter.shift_columns(first - range.c0, last - first + 1, false);
                }
            }
            table.range = Rect::new(range.r0, c0, range.r1, c1);
        }
    }
    table.sync_filter();
    table.data_range().map(|_| table)
}

/// See `features::on_structural_edit`.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    let own = sheet.name.clone();
    for table in &mut sheet.tables {
        table.map_formulas(&mut |e| e.adjusted(edit, edited_sheet, &own));
    }
    if sheet.name != edited_sheet || sheet.tables.is_empty() {
        return;
    }
    sheet.tables = std::mem::take(&mut sheet.tables)
        .into_iter()
        .filter_map(|t| adjusted(t, edit))
        .collect();
}

/// What must be done while the tables are still whole, before the rows or
/// columns of `edits` (applied in that order) are inserted into or deleted
/// from a sheet: structured references to the columns, or to the whole
/// tables, that the deletions take read `#REF!`, as in Excel.
pub(crate) fn before_structural_edits(wb: &mut Workbook, sheet: usize, edits: &[StructuralEdit]) {
    let Some(sh) = wb.sheets.get(sheet).filter(|s| !s.tables.is_empty()) else {
        return;
    };
    let mut tables = Vec::new();
    let mut columns = Vec::new();
    for table in &sh.tables {
        let after = edits.iter().try_fold(table.clone(), |t, edit| adjusted(t, *edit));
        match after {
            None => tables.push(table.name.clone()),
            Some(after) => {
                let gone = table.columns.iter().filter(|c| after.column_index(&c.name).is_none());
                columns.extend(gone.map(|c| (table.name.clone(), c.name.clone())));
            }
        }
    }
    wb.table_refs_to_errors(&tables);
    for (table, column) in columns {
        wb.delete_table_column_refs(&table, &column);
    }
}

/// What a structural edit leaves for a pass that can see the workbook:
/// columns inserted into tables get their names ("Column4") and header
/// cells, and the totals rows are written again.
pub(crate) fn after_structural_edit(wb: &mut Workbook, sheet: usize) {
    let Some(sh) = wb.sheets.get(sheet) else {
        return;
    };
    let ids: Vec<u32> = sh.tables.iter().map(|t| t.id).collect();
    let prefix = wb.locale.column_prefix;
    for id in ids {
        if let Some(table) = wb
            .table_mut(sheet, id)
            .filter(|t| t.columns.iter().any(|c| c.name.is_empty()))
        {
            let names = unique_names(table.columns.iter().map(|c| c.name.clone()).collect(), prefix);
            for (column, name) in table.columns.iter_mut().zip(names) {
                column.name = name;
            }
            wb.write_headers(sheet, id);
        }
        wb.write_totals(sheet, id);
    }
}

/// See `features::on_sheet_renamed`. Tables are known by their own names;
/// only the totals formulas they remember may name the sheet.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for table in &mut sheet.tables {
        table.map_formulas(&mut |e| e.with_sheet_renamed(old, new));
    }
}

/// See `features::on_sheet_duplicated`. The tables of the copy get ids and
/// names of their own (the original's name with a number appended), and
/// the formulas of the copy read the copy's tables.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    let Some(sh) = wb.sheets.get_mut(copy) else {
        return;
    };
    let mut tables = std::mem::take(&mut sh.tables);
    if tables.is_empty() {
        return;
    }
    let mut taken: HashSet<String> = HashSet::new();
    let mut renames: Vec<(String, Arc<str>)> = Vec::new();
    for (table, id) in tables.iter_mut().zip(wb.next_table_id()..) {
        table.id = id;
        let name = (2u32..)
            .map(|n| format!("{}{n}", table.name))
            .find(|name| !wb.name_taken(name, None) && !taken.contains(&name.to_lowercase()))
            .unwrap_or_default();
        taken.insert(name.to_lowercase());
        renames.push((std::mem::replace(&mut table.name, name.clone()), Arc::from(name)));
    }
    // All the tables at once: a new name may be the old name of another.
    let renamed = |name: &str| renames.iter().find(|(old, _)| same_name(old, name)).map(|(_, new)| new.clone());
    let follow = |e: &Expr| -> Expr {
        e.map(&mut |x| match x {
            Expr::Table(mut t) => {
                if let Some(new) = t.table.as_deref().and_then(renamed) {
                    t.table = Some(new);
                }
                Expr::Table(t)
            }
            Expr::Name(n) => Expr::Name(renamed(&n).unwrap_or(n)),
            other => other,
        })
    };
    let sh = &mut wb.sheets[copy];
    sh.tables = tables;
    for cell in sh.cells.values_mut() {
        if let Content::Formula(e) = &cell.content {
            let new = follow(e);
            if new != **e {
                cell.content = Content::Formula(Arc::new(new));
            }
        }
    }
    for def in sh.names.values_mut() {
        def.expr = Arc::new(follow(&def.expr));
    }
    crate::features::map_formulas(sh, &mut |e| follow(e));
}

/// See `features::on_sheet_deleted`. The tables of the sheet went with it:
/// what read them reads `#REF!`. The totals formulas the other tables
/// remember follow like the formulas of cells.
pub(crate) fn on_sheet_deleted(wb: &mut Workbook, deleted: &Sheet, order: &[&str]) {
    let gone: Vec<String> = deleted.tables.iter().map(|t| t.name.clone()).collect();
    wb.table_refs_to_errors(&gone);
    for table in wb.sheets.iter_mut().flat_map(|sh| &mut sh.tables) {
        table.map_formulas(&mut |e| e.with_sheet_deleted(&deleted.name, order));
    }
}

/// See `features::on_sheet_moved`: tables stay on their sheet; 3-D
/// references in the totals formulas they remember follow their end sheets.
pub(crate) fn on_sheet_moved(wb: &mut Workbook, moved: &str, before: &[&str], after: &[&str]) {
    for table in wb.sheets.iter_mut().flat_map(|sh| &mut sh.tables) {
        table.map_formulas(&mut |e| e.with_sheet_moved(moved, before, after));
    }
}

/// See `features::on_cells_shifted`. A shift never tears a table (it is
/// refused); the totals formulas tables remember follow the cells.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    let edited = wb.sheets[sheet].name.clone();
    for sh in &mut wb.sheets {
        let own = sh.name.clone();
        for table in &mut sh.tables {
            table.map_formulas(&mut |e| e.map_refs(&mut |r| shift.fix(r, &edited, &own)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{AutoFilter, Criteria, FilterError, FilterScope, ValueFilter};
    use crate::locale::Locale;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn text(w: &Workbook, name: &str) -> String {
        let r = rect(name);
        w.display_text(0, r.r0, r.c0).text
    }

    fn formula(w: &Workbook, name: &str) -> String {
        let r = rect(name);
        w.formula_text(0, r.r0, r.c0).unwrap_or_default()
    }

    /// A three-column table in A1:C4.
    fn sales() -> (Workbook, u32) {
        let mut w = wb(&[
            ("A1", "Product"),
            ("B1", "Region"),
            ("C1", "Sales"),
            ("A2", "Pen"),
            ("B2", "North"),
            ("C2", "10"),
            ("A3", "Ink"),
            ("B3", "South"),
            ("C3", "20"),
            ("A4", "Pad"),
            ("B4", "North"),
            ("C4", "30"),
        ]);
        let id = w.add_table(0, rect("A1:C4"), true).unwrap();
        w.commit("table");
        (w, id)
    }

    fn only(values: &[&str]) -> Option<Criteria> {
        Some(Criteria::Values(ValueFilter {
            values: values.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }))
    }

    fn names(w: &Workbook, id: u32) -> Vec<String> {
        w.table(0, id).unwrap().columns.iter().map(|c| c.name.clone()).collect()
    }

    #[test]
    fn creating_with_headers() {
        let (mut w, id) = sales();
        let table = w.table(0, id).unwrap();
        assert_eq!((table.name.as_str(), table.range), ("Table1", rect("A1:C4")));
        assert!(table.header_row && !table.totals_row && table.banded_rows && !table.banded_cols);
        assert_eq!(table.style.as_deref(), Some(DEFAULT_TABLE_STYLE));
        assert_eq!(table.auto_filter.as_ref().unwrap().range, rect("A1:C4"));
        assert_eq!(names(&w, id), ["Product", "Region", "Sales"]);
        assert_eq!(table.data_range(), Some(rect("A2:C4")));
        assert_eq!(table.column_range(2), Some(rect("C2:C4")));
        assert_eq!(table.column_index("sales"), Some(2));
        assert_eq!(w.table_by_name("TABLE1").unwrap().1.id, id);
        assert_eq!(w.table_by_id(id).unwrap().0, 0);
        assert_eq!(w.filter_buttons(0).len(), 3);
        assert_eq!(w.suggest_table_range(0, Rect::cell(2, 1)), (rect("A1:C4"), true));

        // Headers become text; blank and repeated ones are renamed.
        let mut other = wb(&[("A1", "Item"), ("B1", "2024"), ("D1", "item"), ("A2", "x"), ("D2", "3")]);
        let id2 = other.add_table(0, rect("A1:D2"), true).unwrap();
        assert_eq!(names(&other, id2), ["Item", "2024", "Column3", "item2"]);
        assert_eq!(other.value_at(0, 0, 1), Value::text("2024"));
        assert_eq!(text(&other, "C1"), "Column3");
        assert_eq!(text(&other, "D1"), "item2");

        // Tables may not share cells with tables or merged cells.
        assert_eq!(w.add_table(0, rect("C4:D6"), true), Err(TableError::Overlap));
        w.set_cell_text(0, 9, 0, "a").unwrap();
        w.merge(0, rect("A10:B10"));
        assert_eq!(w.add_table(0, rect("A10:B12"), true), Err(TableError::MergedCells));
        // Nor can their cells be merged, or carry the sheet's filter.
        w.merge(0, rect("A2:B2"));
        assert_eq!(w.sheets[0].merges, [rect("A10:B10")]);
        assert_eq!(w.set_auto_filter(0, rect("C1:E4")), Err(FilterError::Table));

        // A lone header row gets a data row; the next table is Table2.
        w.set_cell_text(0, 0, 6, "Solo").unwrap();
        let id3 = w.add_table(0, rect("G1"), true).unwrap();
        assert_eq!(w.table(0, id3).unwrap().range, rect("G1:G2"));
        assert_eq!(w.table(0, id3).unwrap().name, "Table2");
        w.commit("more");
        assert!(w.undo());
        assert_eq!(w.sheets[0].tables.len(), 1);
        assert!(w.undo());
        assert!(w.sheets[0].tables.is_empty());
    }

    #[test]
    fn tables_read_from_a_file() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 0, 4, "Year").unwrap();
        w.set_cell_text(0, 1, 4, "2024").unwrap();
        let mut filter = AutoFilter::new(rect("E1:G9"));
        filter.columns.insert(0, only(&["2024"]).unwrap());
        filter.columns.insert(5, only(&["x"]).unwrap());
        let loaded = Table {
            id,
            name: "table1".into(),
            range: rect("E1:G2"),
            header_row: true,
            columns: vec![TableColumn::new("Year"), TableColumn::new("year")],
            auto_filter: Some(filter),
            ..Default::default()
        };
        // The id and the name are taken, a column is missing, another is
        // named twice, the filter is too large: all is put right.
        let new = w.insert_table(0, loaded.clone()).unwrap();
        let table = w.table(0, new).unwrap();
        assert_ne!(new, id);
        assert_eq!(table.name, "Table2");
        assert_eq!(names(&w, new), ["Year", "year2", "Column3"]);
        let filter = table.auto_filter.as_ref().unwrap();
        assert_eq!(filter.range, rect("E1:G2"));
        assert_eq!(filter.columns.keys().copied().collect::<Vec<_>>(), [0]);
        // Its own id and name are kept when they are free.
        let free = Table {
            id: 40,
            name: "Stock".into(),
            range: rect("I1:I3"),
            columns: vec![TableColumn::new("Qty")],
            auto_filter: Some(AutoFilter::new(rect("I1:I3"))),
            ..Default::default()
        };
        assert_eq!(w.insert_table(0, free), Ok(40));
        let table = w.table(0, 40).unwrap();
        // Without a header row there is no filter.
        assert_eq!((table.name.as_str(), table.auto_filter.is_some()), ("Stock", false));
        assert_eq!(w.next_table_name(), "Table3");

        assert_eq!(w.insert_table(0, loaded.clone()), Err(TableError::Overlap));
        let lone = Table {
            range: rect("K1"),
            header_row: true,
            ..loaded.clone()
        };
        assert_eq!(w.insert_table(0, lone), Err(TableError::InvalidRange));
        assert_eq!(w.insert_table(7, loaded), Err(TableError::NotFound));
    }

    #[test]
    fn creating_without_headers() {
        let mut w = wb(&[
            ("A1", "1"),
            ("B1", "2"),
            ("A2", "3"),
            ("B2", "4"),
            ("A3", "under"),
            ("C1", "beside"),
            ("D5", "=SUM(A1:A2)"),
        ]);
        let id = w.add_table(0, rect("A1:B2"), false).unwrap();
        w.commit("table");
        // The data moved down to let the header row in.
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:B3"));
        assert_eq!(names(&w, id), ["Column1", "Column2"]);
        assert_eq!([text(&w, "A1"), text(&w, "B1")], ["Column1", "Column2"]);
        assert_eq!([text(&w, "A2"), text(&w, "B3"), text(&w, "A4")], ["1", "4", "under"]);
        assert_eq!(text(&w, "C1"), "beside");
        assert_eq!(formula(&w, "D5"), "=SUM(A2:A3)");
        assert_eq!(text(&w, "D5"), "4");
        assert!(w.undo());
        assert_eq!([text(&w, "A1"), text(&w, "A3")], ["1", "under"]);

        // A table below in the same columns cannot be pushed down.
        let mut w = wb(&[("A1", "1"), ("A5", "h"), ("A6", "2")]);
        w.add_table(0, rect("A5:A6"), true).unwrap();
        assert_eq!(w.add_table(0, rect("A1:A2"), false), Err(TableError::Blocked));
        assert!(w.add_table(0, rect("A1:A2"), true).is_ok());
    }

    #[test]
    fn names_in_each_language() {
        for (locale, table, column, total) in [
            (Locale::en(), "Table1", "Column1", "Total"),
            (Locale::fr(), "Tableau1", "Colonne1", "Total"),
            (Locale::es(), "Tabla1", "Columna1", "Total"),
            (Locale::de(), "Tabelle1", "Spalte1", "Ergebnis"),
            (Locale::it(), "Tabella1", "Colonna1", "Totale"),
        ] {
            let mut w = Workbook::new(locale);
            w.set_cell_text(0, 0, 0, "1").unwrap();
            w.set_cell_text(0, 0, 1, "2").unwrap();
            let id = w.add_table(0, rect("A1:B1"), false).unwrap();
            w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
            w.commit("table");
            assert_eq!(w.table(0, id).unwrap().name, table);
            assert_eq!(names(&w, id)[0], column);
            assert_eq!(text(&w, "A3"), total);
            assert_eq!(text(&w, "B3"), "2");
        }
    }

    #[test]
    fn renaming() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 0, 4, "x").unwrap();
        let other = w.add_table(0, rect("E1:E2"), true).unwrap();
        w.define_name("Budget", Expr::Number(1.0));
        assert_eq!(w.rename_table(0, id, "Sales_2024.v2"), Ok(()));
        assert_eq!(w.table(0, id).unwrap().name, "Sales_2024.v2");
        assert_eq!(w.rename_table(0, id, "_x"), Ok(()));
        assert_eq!(w.rename_table(0, id, "\\x"), Ok(()));
        for bad in [
            "", "1abc", "my table", "A1", "XFD12", "R1C1", "r", "C", "RC", "a-b", "a!",
        ] {
            assert_eq!(w.rename_table(0, id, bad), Err(TableError::InvalidName), "{bad}");
        }
        assert_eq!(w.rename_table(0, id, "table2"), Err(TableError::NameTaken));
        assert_eq!(w.rename_table(0, id, "BUDGET"), Err(TableError::NameTaken));
        assert_eq!(w.rename_table(0, other, "Table2"), Ok(()));
        assert_eq!(w.rename_table(0, 99, "x"), Err(TableError::NotFound));
        // A deleted table's name is free again for the next new table.
        assert_eq!(w.next_table_name(), "Table1");
    }

    #[test]
    fn options_and_header_row() {
        let (mut w, id) = sales();
        for option in [TableOption::BandedCols, TableOption::FirstCol, TableOption::LastCol] {
            assert!(!w.table(0, id).unwrap().option(option));
            w.set_table_option(0, id, option, true).unwrap();
            assert!(w.table(0, id).unwrap().option(option));
        }
        w.set_table_option(0, id, TableOption::BandedRows, false).unwrap();
        assert!(!w.table(0, id).unwrap().banded_rows);
        assert!(w.set_table_style(0, id, Some("TableStyleLight9")));
        assert_eq!(w.table(0, id).unwrap().style.as_deref(), Some("TableStyleLight9"));
        assert!(w.set_table_style(0, id, None));
        w.commit("look");

        // Hiding the header row empties its cells; the names stay.
        w.set_table_option(0, id, TableOption::HeaderRow, false).unwrap();
        w.commit("no header");
        let table = w.table(0, id).unwrap();
        assert_eq!((table.range, table.header_row), (rect("A2:C4"), false));
        assert!(table.auto_filter.is_none());
        assert_eq!(table.data_range(), Some(rect("A2:C4")));
        assert_eq!(text(&w, "A1"), "");
        assert_eq!(names(&w, id), ["Product", "Region", "Sales"]);
        assert_eq!(
            w.set_table_option(0, id, TableOption::FilterButton, true),
            Err(TableError::NoHeaderRow)
        );
        // Showing it again uses the free row above...
        w.set_table_option(0, id, TableOption::HeaderRow, true).unwrap();
        w.commit("header");
        let table = w.table(0, id).unwrap();
        assert_eq!((table.range, table.header_row), (rect("A1:C4"), true));
        assert!(table.auto_filter.is_some());
        assert_eq!(text(&w, "C1"), "Sales");
        // ...or moves the table down when that row is taken.
        w.set_table_option(0, id, TableOption::HeaderRow, false).unwrap();
        w.set_cell_text(0, 0, 1, "busy").unwrap();
        w.set_cell_text(0, 5, 0, "under").unwrap();
        w.set_table_option(0, id, TableOption::HeaderRow, true).unwrap();
        w.commit("header");
        assert_eq!(w.table(0, id).unwrap().range, rect("A2:C5"));
        assert_eq!(
            [text(&w, "B1"), text(&w, "B2"), text(&w, "A3"), text(&w, "A7")],
            ["busy", "Region", "Pen", "under"]
        );
        assert!(w.undo());
        assert_eq!(text(&w, "A2"), "Pen");
    }

    #[test]
    fn totals_row() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 4, 0, "note").unwrap();
        w.set_cell_text(0, 0, 5, "=SUM(C2:C4)").unwrap();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.commit("totals");
        // "Total" and the sum of the last column; what was under the table
        // moved down.
        let table = w.table(0, id).unwrap();
        assert_eq!((table.range, table.totals_row), (rect("A1:C5"), true));
        assert_eq!(table.auto_filter.as_ref().unwrap().range, rect("A1:C4"));
        assert_eq!(table.totals_range(), Some(rect("A5:C5")));
        assert_eq!(
            [text(&w, "A5"), text(&w, "B5"), text(&w, "C5"), text(&w, "A6")],
            ["Total", "", "60", "note"]
        );
        // As Excel writes it; it names the table in the cell (and in files).
        assert_eq!(formula(&w, "C5"), "=SUBTOTAL(109,[Sales])");
        assert_eq!(
            w.totals_formula(0, id, 2).unwrap().to_text(Locale::en()),
            "SUBTOTAL(109,Table1[Sales])"
        );
        assert_eq!(table.columns[2].totals, TotalsFunction::Sum);
        assert_eq!(table.columns[0].totals_label.as_deref(), Some("Total"));

        // Totals only count the rows the table's filter shows.
        w.set_filter_criteria(0, FilterScope::Table(id), 1, only(&["North"]));
        w.commit("filter");
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2]);
        assert_eq!([text(&w, "C5"), text(&w, "F1")], ["40", "60"]);
        assert_eq!(w.filter_record_count(0, FilterScope::Table(id)), Some((2, 3)));
        assert_eq!(w.sheets[0].row_height(4), crate::workbook::DEFAULT_ROW_HEIGHT);

        // Every function of the list.
        for (function, code, shown) in [
            (TotalsFunction::Average, 101, "20"),
            (TotalsFunction::CountNums, 102, "2"),
            (TotalsFunction::Count, 103, "2"),
            (TotalsFunction::Max, 104, "30"),
            (TotalsFunction::Min, 105, "10"),
            (TotalsFunction::StdDev, 107, "14.14213562"),
            (TotalsFunction::Var, 110, "200"),
            (TotalsFunction::Sum, 109, "40"),
        ] {
            assert_eq!(function.subtotal_code(), Some(code));
            assert_eq!(TotalsFunction::from_subtotal_code(code), Some(function.clone()));
            assert_eq!(
                TotalsFunction::from_ooxml_name(function.ooxml_name()),
                Some(function.clone())
            );
            assert!(w.set_totals_function(0, id, 2, function));
            w.commit("function");
            assert_eq!(formula(&w, "C5"), format!("=SUBTOTAL({code},[Sales])"));
            assert_eq!(text(&w, "C5"), shown);
        }
        assert_eq!(TotalsFunction::None.subtotal_code(), None);
        assert_eq!(TotalsFunction::from_ooxml_name("custom"), None);
        w.clear_filters(0, FilterScope::Table(id));

        // A label, a formula of one's own, nothing.
        assert!(w.set_totals_label(0, id, 1, Some("all regions")));
        assert_eq!(text(&w, "B5"), "all regions");
        assert_eq!(w.set_totals_formula(0, id, 1, "=COUNTIF(B2:B4,\"North\")"), Ok(true));
        w.commit("custom");
        assert_eq!(text(&w, "B5"), "2");
        assert!(matches!(
            w.table(0, id).unwrap().columns[1].totals,
            TotalsFunction::Custom(_)
        ));
        assert_eq!(w.table(0, id).unwrap().columns[1].totals.ooxml_name(), "custom");
        assert!(w.set_totals_formula(0, id, 1, "=SUM(").is_err());
        assert!(w.set_totals_function(0, id, 2, TotalsFunction::None));
        assert_eq!(text(&w, "C5"), "");
        assert!(w.set_totals_function(0, id, 2, TotalsFunction::Max));

        // Hidden and shown again, the row comes back as it was.
        w.set_table_option(0, id, TableOption::TotalsRow, false).unwrap();
        w.commit("no totals");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C4"));
        assert_eq!([text(&w, "A5"), text(&w, "B5"), text(&w, "C5")], ["", "", ""]);
        let written = w.totals_formula(0, id, 1).unwrap();
        assert_eq!(written.to_text(Locale::en()), "COUNTIF(B2:B4,\"North\")");
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.commit("totals");
        assert_eq!([text(&w, "A5"), text(&w, "B5"), text(&w, "C5")], ["Total", "2", "30"]);
        assert_eq!(formula(&w, "B5"), "=COUNTIF(B2:B4,\"North\")");
        assert_eq!(
            w.totals_formula(0, id, 2).unwrap().to_text(Locale::en()),
            "SUBTOTAL(104,Table1[Sales])"
        );
        assert_eq!(w.totals_formula(0, id, 0), None);

        // Typing in the totals row is the same as choosing from its list.
        w.set_cell_text(0, 4, 2, "=SUBTOTAL(101,C2:C4)").unwrap();
        w.set_cell_text(0, 4, 1, "regions").unwrap();
        w.set_cell_text(0, 4, 0, "=1+1").unwrap();
        w.tables_after_edit(0, rect("A5:C5"));
        w.commit("typed");
        let table = w.table(0, id).unwrap();
        assert_eq!(table.columns[2].totals, TotalsFunction::Average);
        assert_eq!(table.columns[1].totals, TotalsFunction::None);
        assert_eq!(table.columns[1].totals_label.as_deref(), Some("regions"));
        assert!(matches!(table.columns[0].totals, TotalsFunction::Custom(_)));
        assert_eq!(table.columns[0].totals_label, None);
        w.clear(0, rect("A5"), ClearMode::Contents);
        w.tables_after_edit(0, rect("A5"));
        assert_eq!(w.table(0, id).unwrap().columns[0].totals, TotalsFunction::None);

        // A text column is counted, not summed, the first time.
        let mut w = wb(&[("A1", "k"), ("B1", "v"), ("A2", "a"), ("B2", "b")]);
        let id = w.add_table(0, rect("A1:B2"), true).unwrap();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.commit("totals");
        assert_eq!(formula(&w, "B3"), "=SUBTOTAL(103,[v])");
        assert_eq!(text(&w, "B3"), "1");
    }

    #[test]
    fn theme_shades() {
        // The shades Excel shows for the accent 1 of its earlier theme, and
        // for black and white.
        let blue = Color(0x4472C4);
        assert_eq!(tint(blue, 0.8), Color(0xD9E1F2));
        assert_eq!(tint(blue, 0.6), Color(0xB4C6E7));
        assert_eq!(tint(blue, 0.4), Color(0x8EA9DB));
        assert_eq!(tint(blue, -0.25), Color(0x305496));
        assert_eq!(tint(blue, -0.5), Color(0x203764));
        assert_eq!(tint(Color::WHITE, -0.15), Color(0xD9D9D9));
        assert_eq!(tint(Color::WHITE, -0.35), Color(0xA6A6A6));
        assert_eq!(tint(Color::BLACK, 0.45), Color(0x737373));
        assert_eq!(tint(Color::BLACK, 0.25), Color(0x404040));
        assert_eq!(tint(Color::BLACK, 0.15), Color(0x262626));
        assert_eq!(tint(blue, 0.0), blue);
    }

    #[test]
    fn built_in_styles() {
        let styles = builtin_table_styles();
        assert_eq!(styles.len(), 60);
        assert_eq!(styles[0].name, "TableStyleLight1");
        assert_eq!(styles[20].name, "TableStyleLight21");
        assert_eq!(styles[21].name, "TableStyleMedium1");
        assert_eq!(styles[49].name, "TableStyleDark1");
        assert_eq!(styles[59].name, "TableStyleDark11");
        for style in styles {
            assert_eq!(table_style(&style.name).unwrap().name, style.name);
        }
        for unknown in [
            "TableStyleMedium29",
            "TableStyleLight0",
            "TableStyleDark12",
            "My style",
            "",
        ] {
            assert!(table_style(unknown).is_none(), "{unknown}");
        }
        let accent = |n: usize| Color(OFFICE_THEME[3 + n]);
        let (white, black) = (Color::WHITE, Color::BLACK);
        let line = |style, color| {
            Some(Border {
                style,
                color: Some(color),
            })
        };

        let medium2 = table_style("TableStyleMedium2").unwrap();
        assert_eq!(medium2.header_row, ElementStyle::heading(white, accent(1)));
        assert_eq!(medium2.first_row_stripe.fill, Some(tint(accent(1), 0.8)));
        assert_eq!(
            medium2.whole_table.horizontal,
            line(BorderStyle::Thin, tint(accent(1), 0.4))
        );
        assert_eq!(medium2.whole_table.vertical, None);
        assert_eq!(medium2.total_row.top, line(BorderStyle::Double, accent(1)));

        let light1 = table_style("TableStyleLight1").unwrap();
        assert_eq!(light1.whole_table.top, line(BorderStyle::Thin, black));
        assert_eq!(light1.whole_table.font_color, None);
        assert_eq!(light1.first_row_stripe.fill, Some(Color(0xD9D9D9)));
        let light7 = table_style("TableStyleLight7").unwrap();
        assert_eq!(light7.header_row.font_color, Some(tint(accent(6), -0.25)));
        assert_eq!(light7.header_row.bottom, line(BorderStyle::Thin, accent(6)));
        let light10 = table_style("TableStyleLight10").unwrap();
        assert_eq!(light10.header_row.fill, Some(accent(2)));
        assert_eq!(light10.second_row_stripe.top, line(BorderStyle::Thin, accent(2)));
        assert_eq!(light10.first_row_stripe.fill, None);
        let light16 = table_style("TableStyleLight16").unwrap();
        assert_eq!(light16.whole_table.vertical, line(BorderStyle::Thin, accent(1)));
        assert_eq!(light16.header_row.bottom, line(BorderStyle::Medium, accent(1)));

        let medium9 = table_style("TableStyleMedium9").unwrap();
        assert_eq!(medium9.whole_table.fill, Some(tint(accent(1), 0.8)));
        assert_eq!(medium9.first_row_stripe.fill, Some(tint(accent(1), 0.6)));
        assert_eq!(medium9.header_row.bottom, line(BorderStyle::Thick, white));
        assert_eq!(medium9.first_column, ElementStyle::heading(white, accent(1)));
        let medium15 = table_style("TableStyleMedium15").unwrap();
        assert_eq!(medium15.whole_table.left, line(BorderStyle::Thin, black));
        assert_eq!(table_style("TableStyleMedium16").unwrap().whole_table.left, None);
        assert_eq!(medium15.first_row_stripe.fill, Some(Color(0xD9D9D9)));
        let medium28 = table_style("TableStyleMedium28").unwrap();
        assert_eq!(
            medium28.whole_table.vertical,
            line(BorderStyle::Thin, tint(accent(6), 0.4))
        );
        assert_eq!(medium28.total_row.top, line(BorderStyle::Medium, accent(6)));
        assert!(medium28.header_row.bold && medium28.header_row.fill.is_none());

        let dark1 = table_style("TableStyleDark1").unwrap();
        assert_eq!(dark1.whole_table.fill, Some(Color(0x737373)));
        assert_eq!(dark1.whole_table.font_color, Some(white));
        assert_eq!(dark1.header_row.fill, Some(black));
        let dark2 = table_style("TableStyleDark2").unwrap();
        assert_eq!(dark2.whole_table.fill, Some(accent(1)));
        assert_eq!(dark2.total_row.fill, Some(tint(accent(1), -0.5)));
        assert_eq!(dark2.first_column.right, line(BorderStyle::Medium, white));
        let dark11 = table_style("TableStyleDark11").unwrap();
        assert_eq!(dark11.whole_table.fill, Some(tint(accent(5), 0.8)));
        assert_eq!(dark11.header_row.fill, Some(accent(6)));
        assert!(!dark11.header_row.bold);
    }

    #[test]
    fn style_of_cells() {
        let (mut w, id) = sales();
        let accent = Color(OFFICE_THEME[4]);
        let (band, edge) = (tint(accent, 0.8), tint(accent, 0.4));
        let thin = Some(Border {
            style: BorderStyle::Thin,
            color: Some(edge),
        });
        // Header: white bold on the accent, the table's outline around.
        let head = style_patch_at(&w, 0, 0, 0).unwrap();
        assert_eq!(head.fill, Some(Some(accent)));
        assert_eq!(head.font_color, Some(Some(Color::WHITE)));
        assert_eq!(head.bold, Some(true));
        assert_eq!(
            (head.border_top, head.border_left, head.border_bottom, head.border_right),
            (thin, thin, thin, None)
        );
        // Data rows: every other one banded.
        let fills: Vec<_> = (1..=3).map(|r| style_patch_at(&w, 0, r, 1).unwrap().fill).collect();
        assert_eq!(fills, [Some(Some(band)), None, Some(Some(band))]);
        let cell = style_patch_at(&w, 0, 3, 2).unwrap();
        assert_eq!(
            (cell.bold, cell.border_bottom, cell.border_right, cell.border_left),
            (None, thin, thin, None)
        );
        assert_eq!(style_patch_at(&w, 0, 4, 0), None);
        assert_eq!(style_patch_at(&w, 0, 0, 3), None);
        // What the grid draws: the style under the cell's own formatting.
        assert_eq!(w.display(0, 0, 0).style.fill, Some(accent));
        assert!(w.display(0, 0, 0).style.font.bold);
        w.update_style(0, rect("B2"), &|s| s.fill = Some(Color(0xFF0000)));
        assert_eq!(w.display(0, 1, 1).style.fill, Some(Color(0xFF0000)));
        assert_eq!(w.display(0, 1, 0).style.fill, Some(band));

        // Bands follow the rows that are shown.
        w.set_filter_criteria(0, FilterScope::Table(id), 1, only(&["North"]));
        w.commit("filter");
        assert_eq!(style_patch_at(&w, 0, 1, 0).unwrap().fill, Some(Some(band)));
        assert_eq!(style_patch_at(&w, 0, 3, 0).unwrap().fill, None);
        w.clear_filters(0, FilterScope::Table(id));
        w.set_rows_hidden(0, 1, 1, true);
        w.commit("hide");
        assert_eq!(style_patch_at(&w, 0, 2, 0).unwrap().fill, Some(Some(band)));
        assert_eq!(style_patch_at(&w, 0, 3, 0).unwrap().fill, None);
        w.set_rows_hidden(0, 1, 1, false);

        // Options: first and last column, banded columns, totals.
        w.set_table_option(0, id, TableOption::FirstCol, true).unwrap();
        w.set_table_option(0, id, TableOption::BandedRows, false).unwrap();
        w.set_table_option(0, id, TableOption::BandedCols, true).unwrap();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.commit("options");
        assert_eq!(style_patch_at(&w, 0, 2, 0).unwrap().bold, Some(true));
        assert_eq!(style_patch_at(&w, 0, 2, 1).unwrap().bold, None);
        let fills: Vec<_> = (0..3).map(|c| style_patch_at(&w, 0, 2, c).unwrap().fill).collect();
        assert_eq!(fills, [Some(Some(band)), None, Some(Some(band))]);
        let total = style_patch_at(&w, 0, 4, 2).unwrap();
        assert_eq!(total.bold, Some(true));
        assert_eq!(total.fill, None);
        assert_eq!(
            total.border_top,
            Some(Border {
                style: BorderStyle::Double,
                color: Some(accent)
            })
        );

        // No style, or one that is not built in: nothing is drawn.
        w.set_table_style(0, id, None);
        assert_eq!(style_patch_at(&w, 0, 0, 0), None);
        w.set_table_style(0, id, Some("Custom style"));
        assert_eq!(style_patch_at(&w, 0, 0, 0), None);
        // A dark style colours every cell.
        w.set_table_style(0, id, Some("TableStyleDark2"));
        let cell = style_patch_at(&w, 0, 2, 1).unwrap();
        assert_eq!(cell.font_color, Some(Some(Color::WHITE)));
        assert_eq!(cell.fill, Some(Some(accent)));
    }

    #[test]
    fn resizing() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 0, 4, "Extra").unwrap();
        w.set_filter_criteria(0, FilterScope::Table(id), 2, only(&["10", "30"]));
        assert_eq!(w.resize_table(0, id, rect("A1:E6")), Ok(()));
        w.commit("resize");
        let table = w.table(0, id).unwrap();
        assert_eq!(table.range, rect("A1:E6"));
        assert_eq!(names(&w, id), ["Product", "Region", "Sales", "Column4", "Extra"]);
        assert_eq!(text(&w, "D1"), "Column4");
        assert_eq!(table.auto_filter.as_ref().unwrap().range, rect("A1:E6"));
        // Smaller, from the left: criteria follow their columns.
        assert_eq!(w.resize_table(0, id, rect("C1:D3")), Ok(()));
        let table = w.table(0, id).unwrap();
        assert_eq!(names(&w, id), ["Sales", "Column4"]);
        assert_eq!(
            table
                .auto_filter
                .as_ref()
                .unwrap()
                .columns
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(text(&w, "A1"), "Product");
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2]);

        assert_eq!(w.resize_table(0, id, rect("C2:D5")), Err(TableError::InvalidResize));
        assert_eq!(w.resize_table(0, id, rect("F1:G5")), Err(TableError::InvalidResize));
        assert_eq!(w.resize_table(0, id, rect("C1:D1")), Err(TableError::InvalidRange));
        w.set_cell_text(0, 0, 6, "h").unwrap();
        w.add_table(0, rect("G1:G3"), true).unwrap();
        assert_eq!(w.resize_table(0, id, rect("C1:G3")), Err(TableError::Overlap));

        // With a totals row, the totals move to the new last row.
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.set_cell_text(0, 5, 2, "5").unwrap();
        assert_eq!(w.resize_table(0, id, rect("A1:C7")), Ok(()));
        w.commit("resize");
        assert_eq!([text(&w, "A5"), text(&w, "A7"), text(&w, "C7")], ["", "Total", "65"]);
        assert_eq!(formula(&w, "C7"), "=SUBTOTAL(109,[Sales])");
    }

    #[test]
    fn growing_and_calculated_columns() {
        let (mut w, id) = sales();
        // A header typed right of the table adds a column.
        w.set_cell_text(0, 0, 3, "Double").unwrap();
        let edit = w.tables_after_edit(0, Rect::cell(0, 3));
        assert_eq!(
            edit,
            TableEdit {
                expanded: Some(id),
                ..Default::default()
            }
        );
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:D4"));
        assert_eq!(names(&w, id)[3], "Double");
        // A formula entered in the empty column fills it.
        w.set_cell_text(0, 2, 3, "=C3*2").unwrap();
        let edit = w.tables_after_edit(0, Rect::cell(2, 3));
        w.commit("formula");
        assert_eq!(edit.filled, Some((id, 3)));
        assert_eq!([formula(&w, "D2"), formula(&w, "D4")], ["=C2*2", "=C4*2"]);
        assert_eq!(text(&w, "D4"), "60");
        assert_eq!(
            w.calculated_column_formula(0, id, 3).unwrap().to_text(Locale::en()),
            "C2*2"
        );
        assert_eq!(w.calculated_column_formula(0, id, 2), None);

        // Typing under the table adds a row, calculated columns included.
        w.set_cell_text(0, 4, 0, "Cap").unwrap();
        let edit = w.tables_after_edit(0, Rect::cell(4, 0));
        w.commit("row");
        assert_eq!(edit.expanded, Some(id));
        let table = w.table(0, id).unwrap();
        assert_eq!(table.range, rect("A1:D5"));
        assert_eq!(table.auto_filter.as_ref().unwrap().range, rect("A1:D5"));
        assert_eq!(formula(&w, "D5"), "=C5*2");
        // Editing one cell of a calculated column changes the column.
        w.set_cell_text(0, 3, 3, "=C4*3").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(3, 3)).filled, Some((id, 3)));
        assert_eq!([formula(&w, "D2"), formula(&w, "D5")], ["=C2*3", "=C5*3"]);
        // A constant among them is left alone, and so are the others then.
        w.set_cell_text(0, 1, 3, "7").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(1, 3)), TableEdit::default());
        w.set_cell_text(0, 2, 3, "=C3*4").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(2, 3)).filled, None);
        assert_eq!(formula(&w, "D5"), "=C5*3");

        // Data pasted beside a data row adds a column with a default name;
        // two rows pasted under the table add two rows.
        w.set_cell_text(0, 2, 4, "x").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(2, 4)).expanded, Some(id));
        assert_eq!(names(&w, id)[4], "Column5");
        assert_eq!(text(&w, "E1"), "Column5");
        w.paste_text(0, 5, 1, "East\t1\nWest\t2");
        assert_eq!(w.tables_after_edit(0, rect("B6:C7")).expanded, Some(id));
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:E7"));
        // Not next to the table, or under its totals row: nothing happens.
        w.set_cell_text(0, 9, 0, "far").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(9, 0)), TableEdit::default());
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.set_cell_text(0, 8, 0, "under totals").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(8, 0)), TableEdit::default());
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:E8"));
    }

    #[test]
    fn renaming_columns_in_the_header() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 0, 0, "Item").unwrap();
        assert!(w.tables_after_edit(0, Rect::cell(0, 0)).renamed);
        assert_eq!(names(&w, id), ["Item", "Region", "Sales"]);
        // A name already taken gets a number, a number becomes text, a
        // cleared header its default name.
        w.set_cell_text(0, 0, 0, "sales").unwrap();
        w.tables_after_edit(0, Rect::cell(0, 0));
        assert_eq!(
            (names(&w, id)[0].as_str(), text(&w, "A1")),
            ("sales2", "sales2".to_string())
        );
        w.set_cell_text(0, 0, 1, "2024").unwrap();
        w.tables_after_edit(0, Rect::cell(0, 1));
        assert_eq!(w.value_at(0, 0, 1), Value::text("2024"));
        // A formula leaves its result.
        w.set_cell_text(0, 0, 1, "=\"Q\"&(1+1)").unwrap();
        w.tables_after_edit(0, Rect::cell(0, 1));
        w.commit("header");
        assert_eq!(
            (names(&w, id)[1].as_str(), w.value_at(0, 0, 1)),
            ("Q2", Value::text("Q2"))
        );
        assert_eq!(w.formula_text(0, 0, 1), None);
        w.clear(0, rect("A1:B1"), ClearMode::Contents);
        assert!(w.tables_after_edit(0, rect("A1:B1")).renamed);
        assert_eq!(names(&w, id), ["Column1", "Column2", "Sales"]);
        assert_eq!([text(&w, "A1"), text(&w, "B1")], ["Column1", "Column2"]);
        // Editing data cells renames nothing.
        w.set_cell_text(0, 1, 0, "Pencil").unwrap();
        assert_eq!(w.tables_after_edit(0, Rect::cell(1, 0)), TableEdit::default());
    }

    #[test]
    fn table_rows() {
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        for (cell, input) in [
            ("E3", "beside"),
            ("A6", "under"),
            ("F1", "=C3"),
            ("F2", "=SUM(C2:C4)"),
            ("D2", "=C2+1"),
        ] {
            let r = rect(cell);
            w.set_cell_text(0, r.r0, r.c0, input).unwrap();
        }
        // D becomes a calculated column of the table.
        w.tables_after_edit(0, rect("D2"));
        w.commit("setup");
        assert_eq!(names(&w, id)[3], "Column4");
        assert_eq!(formula(&w, "D4"), "=C4+1");
        w.update_style(0, rect("A3"), &|s| s.font.italic = true);

        // A row above "Ink": only the table's columns move down.
        assert_eq!(w.table_insert_rows(0, id, 2, 1), Ok(()));
        w.commit("insert");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:D6"));
        assert_eq!(
            [
                text(&w, "A2"),
                text(&w, "A3"),
                text(&w, "A4"),
                text(&w, "A6"),
                text(&w, "A7")
            ],
            ["Pen", "", "Ink", "Total", "under"]
        );
        assert_eq!(text(&w, "E3"), "beside");
        assert_eq!([formula(&w, "F1"), formula(&w, "F2")], ["=C4", "=SUM(C2:C5)"]);
        assert_eq!(formula(&w, "C6"), "=SUBTOTAL(109,[Sales])");
        assert_eq!(text(&w, "C6"), "60");
        // The new row has the calculated column and the look of its
        // neighbour.
        assert_eq!([formula(&w, "D3"), formula(&w, "D4")], ["=C3+1", "=C4+1"]);
        assert!(w.style_at(0, 3, 0).font.italic && !w.style_at(0, 2, 0).font.italic);
        // At the end, before the totals row.
        assert_eq!(w.table_insert_rows(0, id, 5, 2), Ok(()));
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:D8"));
        assert_eq!(
            [text(&w, "A8"), formula(&w, "C8"), formula(&w, "D7")],
            ["Total", "=SUBTOTAL(109,[Sales])", "=C7+1"]
        );
        assert_eq!(w.table_insert_rows(0, id, 0, 1), Err(TableError::InvalidRange));
        assert_eq!(w.table_insert_rows(0, id, 8, 1), Err(TableError::InvalidRange));

        // Deleting them again.
        assert_eq!(w.table_delete_rows(0, id, 5, 2), Ok(()));
        assert_eq!(w.table_delete_rows(0, id, 2, 1), Ok(()));
        w.commit("delete");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:D5"));
        assert_eq!(
            [text(&w, "A3"), text(&w, "A5"), text(&w, "A6"), text(&w, "E3")],
            ["Ink", "Total", "under", "beside"]
        );
        assert_eq!(
            [formula(&w, "F1"), formula(&w, "F2"), formula(&w, "C5")],
            ["=C3", "=SUM(C2:C4)", "=SUBTOTAL(109,[Sales])"]
        );
        assert_eq!(w.table_delete_rows(0, id, 4, 1), Err(TableError::InvalidRange));
        // A reference to a deleted table cell is lost.
        assert_eq!(w.table_delete_rows(0, id, 2, 1), Ok(()));
        assert_eq!(formula(&w, "F1"), "=#REF!");
        // The last data rows cannot all go: one is left, empty.
        assert_eq!(w.table_delete_rows(0, id, 1, 2), Ok(()));
        w.commit("delete all");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:D3"));
        assert_eq!(
            [text(&w, "A2"), text(&w, "C2"), text(&w, "A3"), text(&w, "C3")],
            ["", "", "Total", "0"]
        );

        // Another table below in the same columns blocks the move.
        let (mut w, id) = sales();
        w.set_cell_text(0, 7, 1, "h").unwrap();
        w.add_table(0, rect("B8:B9"), true).unwrap();
        assert_eq!(w.table_insert_rows(0, id, 2, 1), Err(TableError::Blocked));
        assert_eq!(w.table_delete_rows(0, id, 2, 1), Err(TableError::Blocked));
        assert_eq!(w.set_table_option(0, id, TableOption::TotalsRow, true), Ok(()));
        assert_eq!(w.table_insert_cols(0, id, 1, 1), Ok(()));
    }

    #[test]
    fn table_columns() {
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        for (cell, input) in [("E3", "beside"), ("E9", "apart"), ("G1", "=SUM(C2:C4)"), ("G2", "=B2")] {
            let r = rect(cell);
            w.set_cell_text(0, r.r0, r.c0, input).unwrap();
        }
        w.set_filter_criteria(0, FilterScope::Table(id), 2, only(&["10", "20", "30"]));
        w.commit("setup");

        assert_eq!(w.table_insert_cols(0, id, 1, 2), Ok(()));
        w.commit("insert");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:E5"));
        assert_eq!(names(&w, id), ["Product", "Column2", "Column3", "Region", "Sales"]);
        assert_eq!(
            [
                text(&w, "B1"),
                text(&w, "D1"),
                text(&w, "E2"),
                text(&w, "G3"),
                text(&w, "E9")
            ],
            ["Column2", "Region", "10", "beside", "apart"]
        );
        assert_eq!(
            [formula(&w, "I1"), formula(&w, "I2"), formula(&w, "E5")],
            ["=SUM(E2:E4)", "=D2", "=SUBTOTAL(109,[Sales])"]
        );
        let filter = w.table(0, id).unwrap().auto_filter.as_ref().unwrap();
        assert_eq!(filter.columns.keys().copied().collect::<Vec<_>>(), [4]);
        // At the end.
        assert_eq!(w.table_insert_cols(0, id, 5, 1), Ok(()));
        assert_eq!(names(&w, id)[5], "Column6");
        assert_eq!(w.table_insert_cols(0, id, 7, 1), Err(TableError::InvalidRange));

        assert_eq!(w.table_delete_cols(0, id, 5, 1), Ok(()));
        assert_eq!(w.table_delete_cols(0, id, 1, 2), Ok(()));
        w.commit("delete");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C5"));
        assert_eq!(names(&w, id), ["Product", "Region", "Sales"]);
        assert_eq!(
            [text(&w, "E3"), formula(&w, "G1"), formula(&w, "C5")],
            ["beside", "=SUM(C2:C4)", "=SUBTOTAL(109,[Sales])"]
        );
        // Deleting a filtered column drops its criterion and shows the rows.
        w.set_filter_criteria(0, FilterScope::Table(id), 2, only(&["10"]));
        assert_eq!(w.sheets[0].filtered_rows.len(), 2);
        assert_eq!(w.table_delete_cols(0, id, 2, 1), Ok(()));
        assert!(w.sheets[0].filtered_rows.is_empty());
        assert!(!w.table(0, id).unwrap().auto_filter.as_ref().unwrap().is_filtered());
        assert_eq!(w.table_delete_cols(0, id, 1, 2), Err(TableError::InvalidRange));
        // Deleting every column deletes the table.
        assert_eq!(w.table_delete_cols(0, id, 0, 2), Ok(()));
        assert!(w.sheets[0].tables.is_empty());
        assert_eq!(text(&w, "A1"), "");
    }

    #[test]
    fn rows_and_columns_of_the_sheet() {
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.set_filter_criteria(0, FilterScope::Table(id), 2, only(&["10", "30"]));
        w.commit("setup");

        // Rows inserted above push the table down, inside they join it.
        w.insert_rows(0, 0, 2);
        w.insert_rows(0, 3, 1);
        w.commit("insert");
        let table = w.table(0, id).unwrap();
        assert_eq!(table.range, rect("A3:C8"));
        assert_eq!(table.auto_filter.as_ref().unwrap().range, rect("A3:C7"));
        // The new first data row is part of the total.
        assert_eq!(formula(&w, "C8"), "=SUBTOTAL(109,[Sales])");
        assert_eq!(text(&w, "C8"), "40");
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [5]);
        w.delete_rows(0, 3, 1);
        w.delete_rows(0, 0, 2);
        w.commit("delete");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C5"));
        assert_eq!(formula(&w, "C5"), "=SUBTOTAL(109,[Sales])");

        // A column inserted inside is a new table column with a name and a
        // header cell; criteria follow.
        w.insert_cols(0, 1, 1);
        w.commit("insert");
        let table = w.table(0, id).unwrap();
        assert_eq!(table.range, rect("A1:D5"));
        assert_eq!(names(&w, id), ["Product", "Column2", "Region", "Sales"]);
        assert_eq!(text(&w, "B1"), "Column2");
        assert_eq!(
            table
                .auto_filter
                .as_ref()
                .unwrap()
                .columns
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(formula(&w, "D5"), "=SUBTOTAL(109,[Sales])");
        w.insert_cols(0, 0, 1);
        w.delete_cols(0, 2, 1);
        w.commit("columns");
        assert_eq!(w.table(0, id).unwrap().range, rect("B1:D5"));
        assert_eq!(names(&w, id), ["Product", "Region", "Sales"]);
        w.delete_cols(0, 0, 1);
        w.delete_cols(0, 2, 5);
        w.commit("columns");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:B5"));
        assert_eq!(names(&w, id), ["Product", "Region"]);
        assert!(w.sheets[0].filtered_rows.is_empty());

        // Deleting the totals row, then the header row, leaves a table
        // without them; deleting its last rows removes it.
        w.delete_rows(0, 4, 1);
        w.commit("delete");
        let table = w.table(0, id).unwrap();
        assert_eq!((table.range, table.totals_row), (rect("A1:B4"), false));
        w.delete_rows(0, 0, 1);
        w.commit("delete");
        let table = w.table(0, id).unwrap();
        assert_eq!((table.range, table.header_row), (rect("A1:B3"), false));
        assert!(table.auto_filter.is_none());
        assert!(w.filter_buttons(0).is_empty());
        w.delete_rows(0, 0, 3);
        w.commit("delete");
        assert!(w.sheets[0].tables.is_empty());
        assert!(w.undo());
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:B3"));
    }

    #[test]
    fn copies_of_a_sheet() {
        let (mut w, id) = sales();
        w.set_cell_text(0, 0, 4, "h").unwrap();
        let second = w.add_table(0, rect("E1:E2"), true).unwrap();
        w.rename_table(0, second, "Table12").unwrap();
        w.commit("setup");
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        let tables = &w.sheets[copy].tables;
        assert_eq!(tables.len(), 2);
        // "Table12" is taken, and so is the next candidate after the first
        // copy took it.
        assert_eq!(
            [tables[0].name.as_str(), tables[1].name.as_str()],
            ["Table13", "Table122"]
        );
        assert!(tables.iter().all(|t| t.id != id && t.id != second));
        assert_ne!(tables[0].id, tables[1].id);
        assert_eq!(tables[0].range, rect("A1:C4"));
        assert_eq!(w.table_by_name("Table13").unwrap().0, copy);
        assert_eq!(w.table_by_name("Table1").unwrap().0, 0);
    }

    #[test]
    fn the_filter_of_a_table() {
        let (mut w, id) = sales();
        let scope = FilterScope::Table(id);
        // Another list with the sheet's own filter beside the table.
        w.set_cell_text(0, 0, 5, "k").unwrap();
        w.set_cell_text(0, 1, 5, "a").unwrap();
        w.set_cell_text(0, 2, 5, "b").unwrap();
        w.set_cell_text(0, 3, 5, "c").unwrap();
        assert_eq!(w.toggle_auto_filter(0, Rect::cell(1, 5)), Ok(true));
        assert_eq!(w.filter_buttons(0).len(), 4);
        assert_eq!(w.filter_scope_at(0, 2, 1), Some(scope));
        assert_eq!(w.filter_scope_at(0, 2, 5), Some(FilterScope::Sheet));
        assert_eq!(w.filter_button_at(0, 0, 2).unwrap().scope, scope);

        // Each hides rows of the whole sheet; a row is shown when both
        // agree.
        w.set_filter_criteria(0, scope, 1, only(&["North"]));
        w.set_filter_criteria(0, FilterScope::Sheet, 0, only(&["b", "c"]));
        w.commit("filter");
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [1, 2]);
        let drop = w.filter_dropdown(0, scope, 0).unwrap();
        assert_eq!(drop.header, "Product");
        assert_eq!(
            drop.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["Pad", "Pen"]
        );
        w.clear_filters(0, FilterScope::Sheet);
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2]);

        // Sorting from a header button moves the table's rows only.
        w.clear_filters(0, scope);
        w.sort_by_filter(0, scope, 2, crate::ops::SortOn::Values, true).unwrap();
        w.commit("sort");
        assert_eq!([text(&w, "A2"), text(&w, "A4"), text(&w, "F2")], ["Pad", "Pen", "a"]);
        assert_eq!(w.filter_button_at(0, 0, 2).unwrap().sort, Some(true));

        // The Filter button of the ribbon, inside the table, hides and
        // shows the table's buttons.
        w.set_filter_criteria(0, scope, 1, only(&["South"]));
        assert_eq!(w.toggle_auto_filter(0, Rect::cell(2, 0)), Ok(false));
        assert!(w.table(0, id).unwrap().auto_filter.is_none());
        assert!(w.sheets[0].filtered_rows.is_empty());
        assert!(w.sheets[0].auto_filter.is_some());
        assert_eq!(w.toggle_auto_filter(0, Rect::cell(2, 0)), Ok(true));
        assert_eq!(
            w.table(0, id).unwrap().auto_filter.as_ref().unwrap().range,
            rect("A1:C4")
        );

        // A sheet filter on the cells a table is made of becomes the
        // table's.
        let before = w.sheets[0].auto_filter.clone().unwrap().range;
        assert_eq!(before, rect("F1:F4"));
        let list = w.add_table(0, rect("F1:F4"), true).unwrap();
        assert!(w.sheets[0].auto_filter.is_none());
        assert_eq!(
            w.table(0, list).unwrap().auto_filter.as_ref().unwrap().range,
            rect("F1:F4")
        );
    }

    #[test]
    fn rows_of_a_filtered_table() {
        let (mut w, id) = sales();
        let scope = FilterScope::Table(id);
        let hidden = |w: &Workbook| w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>();
        w.set_filter_criteria(0, scope, 1, only(&["North"]));
        w.commit("filter");
        assert_eq!(hidden(&w), [2]);
        // A row typed under the table joins it and stays shown, whatever
        // the criteria, until the filter is applied again.
        w.set_cell_text(0, 4, 0, "Cap").unwrap();
        w.set_cell_text(0, 4, 1, "East").unwrap();
        assert_eq!(w.tables_after_edit(0, rect("A5:B5")).expanded, Some(id));
        w.commit("row");
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C5"));
        assert_eq!(hidden(&w), [2]);
        assert_eq!(w.filter_record_count(0, scope), Some((3, 4)));
        // Inserted rows are shown; the hidden row moves with its cells.
        assert_eq!(w.table_insert_rows(0, id, 1, 2), Ok(()));
        w.commit("insert");
        assert_eq!(hidden(&w), [4]);
        assert_eq!(text(&w, "B5"), "South");
        // Deleting rows above brings it back up; deleting it shows nothing
        // new.
        assert_eq!(w.table_delete_rows(0, id, 1, 2), Ok(()));
        assert_eq!(hidden(&w), [2]);
        w.reapply_filters(0);
        assert_eq!(hidden(&w), [2, 4]);
        assert_eq!(w.table_delete_rows(0, id, 2, 1), Ok(()));
        assert_eq!(hidden(&w), [3]);
        assert_eq!([text(&w, "A3"), text(&w, "A4")], ["Pad", "Cap"]);
        // Rows that leave the table on a resize are shown again.
        assert_eq!(w.resize_table(0, id, rect("A1:C3")), Ok(()));
        assert!(hidden(&w).is_empty());
        assert!(w.table(0, id).unwrap().auto_filter.as_ref().unwrap().is_filtered());
    }

    #[test]
    fn duplicates_in_a_table() {
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.set_cell_text(0, 3, 0, "pen").unwrap();
        w.set_cell_text(0, 5, 0, "under").unwrap();
        w.commit("setup");
        let out = w.remove_duplicates(0, rect("A1:C4"), &[0], true).unwrap();
        w.commit("dedupe");
        assert_eq!((out.removed, out.remaining), (1, 2));
        // The table lost a row and its totals row moved up with what was
        // under it.
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C4"));
        assert_eq!(
            [text(&w, "A3"), text(&w, "A4"), text(&w, "C4"), text(&w, "A5")],
            ["Ink", "Total", "30", "under"]
        );
        assert_eq!(formula(&w, "C4"), "=SUBTOTAL(109,[Sales])");

        // A table right under it keeps the cells from moving up: the rows
        // close up inside the table, which keeps its size.
        let (mut w, id) = sales();
        w.set_cell_text(0, 3, 0, "PEN").unwrap();
        w.set_cell_text(0, 5, 0, "h").unwrap();
        w.add_table(0, rect("A6:A7"), true).unwrap();
        w.commit("setup");
        let out = w.remove_duplicates(0, rect("A1:C4"), &[0], true).unwrap();
        assert_eq!((out.removed, out.remaining), (1, 2));
        assert_eq!(w.table(0, id).unwrap().range, rect("A1:C4"));
        assert_eq!([text(&w, "A3"), text(&w, "A4"), text(&w, "A6")], ["Ink", "", "h"]);
    }

    #[test]
    fn converting_and_deleting() {
        let (mut w, id) = sales();
        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        w.set_filter_criteria(0, FilterScope::Table(id), 1, only(&["North"]));
        w.update_style(0, rect("B2"), &|s| s.fill = Some(Color(0xFF0000)));
        w.commit("setup");
        let accent = Color(OFFICE_THEME[4]);
        assert!(w.convert_table_to_range(0, id));
        w.commit("convert");
        assert!(w.sheets[0].tables.is_empty() && w.sheets[0].filtered_rows.is_empty());
        assert!(w.filter_buttons(0).is_empty());
        // The cells keep the look of the style, under their own formats.
        let head = w.style_at(0, 0, 0);
        assert_eq!(
            (head.fill, head.font.color, head.font.bold),
            (Some(accent), Some(Color::WHITE), true)
        );
        assert_eq!(w.style_at(0, 1, 0).fill, Some(tint(accent, 0.8)));
        assert_eq!(w.style_at(0, 1, 1).fill, Some(Color(0xFF0000)));
        assert_eq!(w.style_at(0, 2, 0).fill, None);
        assert_eq!(w.style_at(0, 4, 2).borders.top.style, BorderStyle::Double);
        assert_eq!([text(&w, "A1"), text(&w, "C5")], ["Product", "60"]);
        assert_eq!(style_patch_at(&w, 0, 0, 0), None);
        assert!(w.undo());
        assert_eq!(w.sheets[0].tables.len(), 1);
        assert_eq!(w.style_at(0, 0, 0).fill, None);

        // Deleting a table clears its cells, the filtered ones too.
        assert!(w.delete_table(0, id));
        w.commit("delete");
        assert!(w.sheets[0].tables.is_empty() && w.sheets[0].filtered_rows.is_empty());
        assert!((0..5).all(|r| (0..3).all(|c| w.sheets[0].is_blank(r, c))));
        assert_eq!(w.style_at(0, 1, 1).fill, None);
        assert!(!w.delete_table(0, id));
        assert!(!w.convert_table_to_range(0, id));
    }

    #[test]
    fn structured_references() {
        let (mut w, id) = sales();
        let at = |w: &Workbook, range: &str, from: Option<&str>| {
            let from = from.map(|f| (rect(f).r0, rect(f).c0));
            w.structured_reference(0, rect(range), from).unwrap_or_default()
        };
        assert_eq!(at(&w, "C2:C4", None), "Table1[Sales]");
        assert_eq!(at(&w, "A2:C4", None), "Table1");
        assert_eq!(at(&w, "B2:C4", None), "Table1[[Region]:[Sales]]");
        assert_eq!(at(&w, "C1", None), "Table1[[#Headers],[Sales]]");
        assert_eq!(at(&w, "A1:C1", None), "Table1[#Headers]");
        assert_eq!(at(&w, "A1:C4", None), "Table1[#All]");
        assert_eq!(at(&w, "B1:B4", None), "Table1[[#All],[Region]]");
        // The row of the formula: inside the table, beside it.
        assert_eq!(at(&w, "C3", Some("B3")), "[@Sales]");
        assert_eq!(at(&w, "C3", Some("F3")), "Table1[@Sales]");
        assert_eq!(at(&w, "B3:C3", Some("F3")), "Table1[@[Region]:[Sales]]");
        assert_eq!(at(&w, "A3:C3", Some("F3")), "Table1[@]");
        // Anything else is an ordinary reference.
        assert_eq!(at(&w, "C3", Some("F2")), "");
        assert_eq!(at(&w, "C3", None), "");
        assert_eq!(at(&w, "C2:C3", None), "");
        assert_eq!(at(&w, "C2:D4", None), "");
        assert_eq!(at(&w, "E5", None), "");

        w.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        assert_eq!(at(&w, "C5", None), "Table1[[#Totals],[Sales]]");
        assert_eq!(at(&w, "A5:C5", None), "Table1[#Totals]");
        // Names with spaces or special characters.
        w.set_cell_text(0, 0, 2, "Unit Price").unwrap();
        w.set_cell_text(0, 0, 1, "Tax [#1]").unwrap();
        w.tables_after_edit(0, rect("B1:C1"));
        assert_eq!(at(&w, "C2:C4", None), "Table1[Unit Price]");
        assert_eq!(at(&w, "C3", Some("A3")), "[@[Unit Price]]");
        assert_eq!(at(&w, "B2:B4", None), "Table1[Tax '['#1']]");

        let cell = w.table_cell(0, 0, 2).unwrap();
        assert_eq!(
            (cell.table.id, cell.column, cell.area, cell.column_name()),
            (id, 2, TableArea::Header, "Unit Price")
        );
        assert_eq!(w.table_cell(0, 2, 0).unwrap().area, TableArea::Data);
        assert_eq!(w.table_cell(0, 4, 1).unwrap().area, TableArea::Totals);
        assert!(w.table_cell(0, 5, 0).is_none());
        assert_eq!(w.table_at(0, 2, 2).unwrap().id, id);
        assert_eq!(w.table(0, id).unwrap().column_at(1), Some(1));
        assert_eq!(w.table(0, id).unwrap().column_at(3), None);
    }
}
