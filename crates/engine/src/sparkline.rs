//! Sparklines (Insert > Sparklines): small charts drawn inside cells.
//!
//! A sheet keeps its sparklines in groups (`Sheet::sparklines`). A group
//! shares its type, colours and options; each of its sparklines sits in
//! one cell and plots one row or one column of cells. What a sparkline
//! looks like is worked out here (`shapes`), as lines and rectangles in
//! the box of its cell, so that the screen and the printed page draw the
//! same thing.

use crate::ast::{StructuralEdit, same_name};
use crate::features::CellShift;
use crate::ops::shift_interval;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::Color;
use crate::value::Value;
use crate::workbook::{Sheet, Workbook};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparklineType {
    Line,
    Column,
    WinLoss,
}

/// "Show empty cells as".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyCells {
    Gaps,
    Zero,
    /// "Connect data points with line".
    Connect,
}

/// An end of the vertical axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisBound {
    /// "Automatic for each sparkline".
    Individual,
    /// "Same for all sparklines" of the group.
    Group,
    Custom(f64),
}

/// The colours of a group. Excel's galleries give them all at once
/// (`style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SparklineColors {
    pub series: Color,
    pub negative: Color,
    pub axis: Color,
    pub markers: Color,
    pub first: Color,
    pub last: Color,
    pub high: Color,
    pub low: Color,
}

impl Default for SparklineColors {
    /// What Excel gives a new sparkline.
    fn default() -> SparklineColors {
        let red = Color(0xD00000);
        SparklineColors {
            series: Color(0x376092),
            negative: red,
            axis: Color::BLACK,
            markers: red,
            first: red,
            last: red,
            high: red,
            low: red,
        }
    }
}

/// The cells a sparkline plots: one row or one column on a sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataRef {
    pub sheet: String,
    pub range: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sparkline {
    pub row: u32,
    pub col: u32,
    pub data: DataRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SparklineGroup {
    pub kind: SparklineType,
    pub sparklines: Vec<Sparkline>,
    /// In points.
    pub line_weight: f64,
    pub markers: bool,
    pub high: bool,
    pub low: bool,
    pub first: bool,
    pub last: bool,
    pub negative: bool,
    pub show_axis: bool,
    /// "Show data in hidden rows and columns".
    pub show_hidden: bool,
    pub empty: EmptyCells,
    pub right_to_left: bool,
    pub min: AxisBound,
    pub max: AxisBound,
    pub colors: SparklineColors,
}

impl SparklineGroup {
    /// A group as Insert > Sparklines makes it.
    pub fn new(kind: SparklineType) -> SparklineGroup {
        SparklineGroup {
            kind,
            sparklines: Vec::new(),
            line_weight: 0.75,
            markers: false,
            high: false,
            low: false,
            first: false,
            last: false,
            // Win/Loss tells losses from wins by their colour.
            negative: kind == SparklineType::WinLoss,
            show_axis: false,
            show_hidden: false,
            empty: EmptyCells::Gaps,
            right_to_left: false,
            min: AxisBound::Individual,
            max: AxisBound::Individual,
            colors: SparklineColors::default(),
        }
    }

    /// The same settings around other sparklines.
    fn with(&self, sparklines: Vec<Sparkline>) -> SparklineGroup {
        SparklineGroup {
            sparklines,
            ..self.clone()
        }
    }
}

// ------------------------------------------------------------------ styles

/// The colours of the Office theme the styles are built from: light 1,
/// dark 1, light 2, dark 2 and the six accents.
const THEME: [u32; 10] = [0xFFFFFF, 0x000000, 0xE7E6E6, 0x44546A, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47];

/// A theme colour made lighter (`tint` > 0) or darker, as Excel does it:
/// on the luminance of the colour.
fn tinted(theme: usize, tint: f64) -> Color {
    let rgb = THEME[theme];
    if tint == 0.0 {
        return Color(rgb);
    }
    let [r, g, b] = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|x| x as f64 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h / 6.0, s)
    };
    let l = if tint < 0.0 { l * (1.0 + tint) } else { l * (1.0 - tint) + tint };
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let channel = |t: f64| {
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

/// How many styles the gallery has: six columns of six.
pub const STYLE_COUNT: usize = 36;

/// The colours of the style at `index` of Excel's gallery (0-based, row
/// by row): the six accents darker by 50 %, darker by 25 %, as they are
/// and lighter by 40 %, then the six dark and the six colourful styles.
pub fn style(index: usize) -> SparklineColors {
    const DARK: f64 = -0.499984740745262;
    const DARKER: f64 = -0.249977111117893;
    const LIGHT: f64 = 0.39997558519241921;
    let all = |series: Color, negative: Color, points: Color| SparklineColors {
        series,
        negative,
        axis: Color::BLACK,
        markers: points,
        first: points,
        last: points,
        high: points,
        low: points,
    };
    let (row, column) = (index / 6, index % 6);
    let accent = 4 + column;
    let next = 4 + (column + 1) % 6;
    match row {
        0 => SparklineColors {
            series: tinted(accent, DARK),
            negative: tinted(next, 0.0),
            axis: Color::BLACK,
            markers: tinted(accent, DARK),
            first: tinted(accent, LIGHT),
            last: tinted(accent, LIGHT),
            high: tinted(accent, 0.0),
            low: tinted(accent, 0.0),
        },
        1 => all(tinted(accent, DARKER), tinted(next, 0.0), tinted(next, DARKER)),
        2 => all(tinted(accent, 0.0), tinted(next, 0.0), tinted(accent, DARKER)),
        3 => SparklineColors {
            series: tinted(accent, LIGHT),
            negative: tinted(0, DARK),
            axis: Color::BLACK,
            markers: tinted(accent, 0.79998168889431442),
            first: tinted(accent, DARKER),
            last: tinted(accent, DARKER),
            high: tinted(accent, DARK),
            low: tinted(accent, DARK),
        },
        4 => match column {
            0 => all(tinted(1, 0.499984740745262), tinted(1, 0.249977111117893), tinted(1, 0.249977111117893)),
            1 => all(tinted(1, 0.34998626667073579), tinted(0, DARKER), tinted(0, DARKER)),
            2 => all(Color(0x323232), Color(0xD00000), Color(0xD00000)),
            3 => all(Color(0x000000), Color(0x0070C0), Color(0x0070C0)),
            4 => all(Color(0x376092), Color(0xD00000), Color(0xD00000)),
            _ => all(Color(0x0070C0), Color(0x000000), Color(0x000000)),
        },
        _ => {
            let each = |c: [u32; 7]| SparklineColors {
                series: Color(c[0]),
                negative: Color(c[1]),
                axis: Color::BLACK,
                markers: Color(c[2]),
                first: Color(c[3]),
                last: Color(c[4]),
                high: Color(c[5]),
                low: Color(c[6]),
            };
            match column {
                0 => each([0x5F5F5F, 0xFFB620, 0xD70077, 0x5687C2, 0x359CEB, 0x56BE79, 0xFF5055]),
                1 => each([0x5687C2, 0xFFB620, 0xD70077, 0x777777, 0x359CEB, 0x56BE79, 0xFF5055]),
                2 => each([0xC6EFCE, 0xFFC7CE, 0x8CADD6, 0xFFDC47, 0xFFEB9C, 0x60D276, 0xFF5367]),
                3 => each([0x00B050, 0xFF0000, 0x0070C0, 0xFFC000, 0xFFC000, 0x00B050, 0xFF0000]),
                4 => each([THEME[3], THEME[9], THEME[8], THEME[4], THEME[5], THEME[6], THEME[7]]),
                _ => each([THEME[1], THEME[9], THEME[8], THEME[4], THEME[5], THEME[6], THEME[7]]),
            }
        }
    }
}

// ----------------------------------------------------------------- drawing

/// A piece of the picture of a sparkline, in the box of its cell (pixels
/// from its top-left corner).
#[derive(Clone, Debug, PartialEq)]
pub enum SparkShape {
    Line { points: Vec<(f32, f32)>, color: Color, width: f32 },
    Rect { x0: f32, y0: f32, x1: f32, y1: f32, color: Color },
}

/// The ends of the vertical axis a whole group shares (`None`: each
/// sparkline has its own).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SharedScale {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Draw a sparkline of `group` over `values` (`None`: an empty cell) in
/// a cell `width` by `height`; sizes that do not follow the cell (the
/// room around the plot, line weights, markers) are multiplied by `zoom`.
pub fn shapes(group: &SparklineGroup, values: &[Option<f64>], shared: SharedScale, width: f32, height: f32, zoom: f32) -> Vec<SparkShape> {
    let mut out = Vec::new();
    let mut values: Vec<Option<f64>> = values.to_vec();
    if group.right_to_left {
        values.reverse();
    }
    let numbers: Vec<f64> = values.iter().flatten().copied().collect();
    let n = values.len();
    if numbers.is_empty() || width <= 0.0 || height <= 0.0 {
        return out;
    }
    let pad = (3.0 * zoom).max(1.0);
    let (x0, x1, y0, y1) = (pad, (width - pad).max(pad + 1.0), pad, (height - pad).max(pad + 1.0));
    let data_min = numbers.iter().copied().fold(f64::INFINITY, f64::min);
    let data_max = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let bound = |b: AxisBound, shared: Option<f64>, own: f64| match b {
        AxisBound::Custom(v) => v,
        AxisBound::Group => shared.unwrap_or(own),
        AxisBound::Individual => own,
    };
    let (mut lo, mut hi) = (bound(group.min, shared.min, data_min), bound(group.max, shared.max, data_max));
    if hi < lo {
        std::mem::swap(&mut lo, &mut hi);
    }
    // Which points stand out, and in which colour.
    let first = values.iter().position(Option::is_some);
    let last = values.iter().rposition(Option::is_some);
    let colors = &group.colors;
    let special = |i: usize, v: f64| -> Option<Color> {
        if group.high && v == data_max {
            Some(colors.high)
        } else if group.low && v == data_min {
            Some(colors.low)
        } else if group.first && Some(i) == first {
            Some(colors.first)
        } else if group.last && Some(i) == last {
            Some(colors.last)
        } else if group.negative && v < 0.0 {
            Some(colors.negative)
        } else {
            None
        }
    };
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32, color: Color| SparkShape::Rect { x0, y0, x1, y1, color };
    let axis = |y: f32| rect(x0, y.round(), x1, y.round() + 1.0, colors.axis);
    match group.kind {
        SparklineType::Line => {
            let span = hi - lo;
            let y_of = |v: f64| {
                if span <= 0.0 {
                    (y0 + y1) / 2.0
                } else {
                    y1 - (((v - lo) / span).clamp(0.0, 1.0) as f32) * (y1 - y0)
                }
            };
            let x_of = |i: usize| if n > 1 { x0 + i as f32 * (x1 - x0) / (n - 1) as f32 } else { (x0 + x1) / 2.0 };
            if group.show_axis && lo <= 0.0 && hi >= 0.0 && span > 0.0 {
                out.push(axis(y_of(0.0)));
            }
            let weight = (group.line_weight as f32 * 96.0 / 72.0 * zoom).max(0.5);
            let mut run: Vec<(f32, f32)> = Vec::new();
            let flush = |run: &mut Vec<(f32, f32)>, out: &mut Vec<SparkShape>| {
                if run.len() > 1 {
                    out.push(SparkShape::Line {
                        points: std::mem::take(run),
                        color: colors.series,
                        width: weight,
                    });
                }
                run.clear();
            };
            for (i, value) in values.iter().enumerate() {
                match value {
                    Some(v) => run.push((x_of(i), y_of(*v))),
                    // A gap breaks the line; "connect" runs over it.
                    None if group.empty == EmptyCells::Gaps => flush(&mut run, &mut out),
                    None => {}
                }
            }
            flush(&mut run, &mut out);
            let half = ((2.0 * zoom + weight / 2.0) / 2.0).max(1.0).round() + 0.5;
            for (i, value) in values.iter().enumerate() {
                let Some(v) = value else {
                    continue;
                };
                let color = special(i, *v).or(group.markers.then_some(colors.markers));
                if let Some(color) = color {
                    let (x, y) = (x_of(i), y_of(*v));
                    out.push(rect(x - half, y - half, x + half, y + half, color));
                }
            }
        }
        SparklineType::Column | SparklineType::WinLoss => {
            let win_loss = group.kind == SparklineType::WinLoss;
            // Columns stand on zero when the axis shows it, else on the
            // end of the axis nearest to zero.
            let (lo, hi) = if win_loss { (-1.0, 1.0) } else { (lo, hi) };
            let base = 0.0f64.clamp(lo, hi);
            let span = hi - lo;
            let y_of = |v: f64| {
                if span <= 0.0 {
                    if v > 0.0 { y0 } else { y1 }
                } else {
                    y1 - (((v - lo) / span).clamp(0.0, 1.0) as f32) * (y1 - y0)
                }
            };
            let base_y = if span <= 0.0 { if lo > 0.0 { y1 } else { y0 } } else { y_of(base) };
            let slot = (x1 - x0) / n as f32;
            let gap = (slot * 0.15).min(2.0 * zoom);
            for (i, value) in values.iter().enumerate() {
                let Some(v) = value else {
                    continue;
                };
                let shown = if win_loss { v.signum() } else { *v };
                if win_loss && *v == 0.0 {
                    continue;
                }
                let (left, right) = (x0 + i as f32 * slot + gap, x0 + (i + 1) as f32 * slot - gap);
                let y = y_of(shown);
                // The smallest column still shows.
                let (top, bottom) = if y <= base_y { (y.min(base_y - 1.0), base_y) } else { (base_y, y.max(base_y + 1.0)) };
                let color = special(i, *v).unwrap_or(colors.series);
                out.push(rect(left, top, right.max(left + 1.0), bottom, color));
            }
            if group.show_axis && (win_loss || (lo <= 0.0 && hi >= 0.0)) {
                out.push(axis(base_y));
            }
        }
    }
    out
}

// ------------------------------------------------------------------ errors

/// Why sparklines cannot be made or changed as asked (the refusals of
/// the Create Sparklines dialog).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SparklineError {
    /// The location is not cells of a single row or column of this sheet.
    LocationShape,
    /// The data range names a sheet that is not there.
    BadData,
    /// The data range does not give one row or column for each cell of
    /// the location.
    Mismatch,
}

/// One row or column of `data` for each cell of `location` (cells of a
/// single row or column), as Excel pairs them: by rows when the
/// location runs down and the data has as many rows, by columns
/// otherwise when that fits.
fn pair(data: Rect, location: Rect) -> Result<Vec<((u32, u32), Rect)>, SparklineError> {
    if location.rows() > 1 && location.cols() > 1 {
        return Err(SparklineError::LocationShape);
    }
    let cells: Vec<(u32, u32)> = (location.r0..=location.r1).flat_map(|r| (location.c0..=location.c1).map(move |c| (r, c))).collect();
    let n = cells.len() as u32;
    let rows = |i: u32| Rect::new(data.r0 + i, data.c0, data.r0 + i, data.c1);
    let cols = |i: u32| Rect::new(data.r0, data.c0 + i, data.r1, data.c0 + i);
    let by_rows = data.rows() == n;
    let by_cols = data.cols() == n;
    let slice: &dyn Fn(u32) -> Rect = if n == 1 {
        // One sparkline plots one row or one column.
        if data.rows() > 1 && data.cols() > 1 {
            return Err(SparklineError::Mismatch);
        }
        &|_| data
    } else if by_rows && (location.rows() > 1 || !by_cols) {
        &rows
    } else if by_cols {
        &cols
    } else {
        return Err(SparklineError::Mismatch);
    };
    Ok(cells.into_iter().enumerate().map(|(i, cell)| (cell, slice(i as u32))).collect())
}

impl Sheet {
    /// The sparkline of a cell: its group and its place in the group.
    pub fn sparkline_at(&self, row: u32, col: u32) -> Option<(usize, usize)> {
        self.sparklines.iter().enumerate().find_map(|(g, group)| {
            let i = group.sparklines.iter().position(|s| (s.row, s.col) == (row, col))?;
            Some((g, i))
        })
    }

    /// The groups that have a sparkline in one of `ranges`.
    pub fn sparkline_groups_in(&self, ranges: &[Rect]) -> Vec<usize> {
        let inside = |s: &Sparkline| ranges.iter().any(|r| r.contains(s.row, s.col));
        (0..self.sparklines.len()).filter(|g| self.sparklines[*g].sparklines.iter().any(inside)).collect()
    }

    /// Take the sparklines of some cells away; groups left empty go.
    fn drop_sparklines(&mut self, gone: impl Fn(&Sparkline) -> bool) {
        for group in &mut self.sparklines {
            group.sparklines.retain(|s| !gone(s));
        }
        self.sparklines.retain(|group| !group.sparklines.is_empty());
    }
}

impl Workbook {
    /// The numbers a sparkline plots, in the order of its cells: `None`
    /// for an empty cell (or one that holds no number). Hidden rows and
    /// columns are left out unless the group shows them, and empty cells
    /// count as zero when the group says so.
    pub fn sparkline_values(&self, group: &SparklineGroup, sparkline: &Sparkline) -> Vec<Option<f64>> {
        let Some(sheet) = self.sheet_index(&sparkline.data.sheet) else {
            return Vec::new();
        };
        let sh = &self.sheets[sheet];
        let range = sparkline.data.range;
        let mut out = Vec::new();
        for row in range.r0..=range.r1 {
            if !group.show_hidden && sh.is_row_hidden(row) {
                continue;
            }
            for col in range.c0..=range.c1 {
                if !group.show_hidden && sh.hidden_cols.contains(&col) {
                    continue;
                }
                out.push(match sh.value_at(row, col) {
                    Value::Number(n) if n.is_finite() => Some(n),
                    _ if group.empty == EmptyCells::Zero => Some(0.0),
                    _ => None,
                });
            }
        }
        out
    }

    /// The ends of the vertical axis the sparklines of a group share
    /// ("Same for all sparklines").
    pub fn sparkline_scale(&self, group: &SparklineGroup) -> SharedScale {
        if group.min != AxisBound::Group && group.max != AxisBound::Group {
            return SharedScale::default();
        }
        let all: Vec<f64> = group.sparklines.iter().flat_map(|s| self.sparkline_values(group, s)).flatten().collect();
        SharedScale {
            min: (group.min == AxisBound::Group).then(|| all.iter().copied().fold(f64::INFINITY, f64::min)).filter(|v| v.is_finite()),
            max: (group.max == AxisBound::Group).then(|| all.iter().copied().fold(f64::NEG_INFINITY, f64::max)).filter(|v| v.is_finite()),
        }
    }

    /// The sparklines `data` (on the sheet `data_sheet`) gives the cells
    /// of `location`, when the two go together.
    fn paired_sparklines(&self, data_sheet: usize, data: Rect, location: Rect) -> Result<Vec<Sparkline>, SparklineError> {
        let name = self.sheets.get(data_sheet).ok_or(SparklineError::BadData)?.name.clone();
        let pairs = pair(self.clamp_data(data_sheet, data), location)?;
        Ok(pairs
            .into_iter()
            .map(|((row, col), range)| Sparkline {
                row,
                col,
                data: DataRef {
                    sheet: name.clone(),
                    range,
                },
            })
            .collect())
    }

    /// Whole rows or columns given as data stop at the last cell in use.
    fn clamp_data(&self, sheet: usize, data: Rect) -> Rect {
        if data.is_whole_rows() || data.is_whole_cols() { self.clamp_to_used(sheet, data) } else { data }
    }

    /// Insert > Sparklines: a new group of `kind` in the cells of
    /// `location` on `sheet`, plotting `data`. Sparklines already in
    /// those cells are replaced. Returns the index of the group.
    pub fn add_sparklines(
        &mut self,
        sheet: usize,
        kind: SparklineType,
        data: (usize, Rect),
        location: Rect,
    ) -> Result<usize, SparklineError> {
        if sheet >= self.sheets.len() {
            return Err(SparklineError::LocationShape);
        }
        let sparklines = self.paired_sparklines(data.0, data.1, location)?;
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        sh.drop_sparklines(|s| location.contains(s.row, s.col));
        sh.sparklines.push(SparklineGroup::new(kind).with(sparklines));
        Ok(sh.sparklines.len() - 1)
    }

    /// Edit Group Location & Data: the group keeps its settings and gets
    /// other cells and other data.
    pub fn edit_sparkline_group(&mut self, sheet: usize, group: usize, data: (usize, Rect), location: Rect) -> Result<(), SparklineError> {
        if self.sheets.get(sheet).and_then(|sh| sh.sparklines.get(group)).is_none() {
            return Err(SparklineError::LocationShape);
        }
        let sparklines = self.paired_sparklines(data.0, data.1, location)?;
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let settings = sh.sparklines[group].with(sparklines);
        sh.sparklines[group].sparklines.clear();
        sh.drop_sparklines(|s| location.contains(s.row, s.col));
        // Emptied, the group was dropped with the others: it comes back
        // at the end with its new cells.
        sh.sparklines.push(settings);
        Ok(())
    }

    /// Edit Single Sparkline's Data: one row or one column of cells.
    pub fn edit_sparkline_data(&mut self, sheet: usize, row: u32, col: u32, data: (usize, Rect)) -> Result<(), SparklineError> {
        let Some((group, index)) = self.sheets.get(sheet).and_then(|sh| sh.sparkline_at(row, col)) else {
            return Err(SparklineError::LocationShape);
        };
        let mut found = self.paired_sparklines(data.0, data.1, Rect::cell(row, col))?;
        self.snapshot();
        self.sheets[sheet].sparklines[group].sparklines[index] = found.remove(0);
        Ok(())
    }

    /// Change the settings of the groups that have a sparkline in
    /// `ranges` (the commands of the Sparkline tab). False when there is
    /// none.
    pub fn update_sparkline_groups(&mut self, sheet: usize, ranges: &[Rect], change: impl Fn(&mut SparklineGroup)) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let groups = sh.sparkline_groups_in(ranges);
        if groups.is_empty() {
            return false;
        }
        self.snapshot();
        for g in groups {
            change(&mut self.sheets[sheet].sparklines[g]);
        }
        true
    }

    /// Group: the sparklines in `ranges` become one group, with the
    /// settings of the group of `lead` (the active cell) when it has a
    /// sparkline, else of the first one found.
    pub fn group_sparklines(&mut self, sheet: usize, ranges: &[Rect], lead: (u32, u32)) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let inside = |s: &Sparkline| ranges.iter().any(|r| r.contains(s.row, s.col));
        let touched = sh.sparkline_groups_in(ranges);
        let Some(model) = sh.sparkline_at(lead.0, lead.1).map(|(g, _)| g).or(touched.first().copied()) else {
            return false;
        };
        let mut members: Vec<Sparkline> = sh.sparklines.iter().flat_map(|g| g.sparklines.iter().filter(|s| inside(s)).cloned()).collect();
        members.sort_by_key(|s| (s.row, s.col));
        let grouped = sh.sparklines[model].with(members);
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        sh.drop_sparklines(inside);
        sh.sparklines.push(grouped);
        true
    }

    /// Ungroup: each sparkline in `ranges` becomes a group of its own.
    pub fn ungroup_sparklines(&mut self, sheet: usize, ranges: &[Rect]) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let inside = |s: &Sparkline| ranges.iter().any(|r| r.contains(s.row, s.col));
        let singles: Vec<SparklineGroup> = sh
            .sparklines
            .iter()
            .flat_map(|g| g.sparklines.iter().filter(|s| inside(s)).map(|s| g.with(vec![s.clone()])))
            .collect();
        if singles.is_empty() {
            return false;
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        sh.drop_sparklines(inside);
        sh.sparklines.extend(singles);
        true
    }

    /// Clear Selected Sparklines (`whole_groups` false) or Clear Selected
    /// Sparkline Groups.
    pub fn clear_sparklines(&mut self, sheet: usize, ranges: &[Rect], whole_groups: bool) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let groups = sh.sparkline_groups_in(ranges);
        if groups.is_empty() {
            return false;
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        if whole_groups {
            let mut index = 0;
            sh.sparklines.retain(|_| {
                index += 1;
                !groups.contains(&(index - 1))
            });
        } else {
            sh.drop_sparklines(|s| ranges.iter().any(|r| r.contains(s.row, s.col)));
        }
        true
    }

    /// Cells were copied (or filled) from `src` on `src_sheet` to the
    /// block starting at `dest` on `dest_sheet`: their sparklines come
    /// along, each plotting the cells that lie as far from it as its
    /// model's data lay from the model. The copies join the group of
    /// their model on its own sheet. With `cut` the sparklines move and
    /// keep their data.
    pub fn copy_sparklines(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest: (u32, u32), cut: bool) {
        let Some(sh) = self.sheets.get(src_sheet) else {
            return;
        };
        if dest_sheet >= self.sheets.len() {
            return;
        }
        let (d_row, d_col) = (dest.0 as i64 - src.r0 as i64, dest.1 as i64 - src.c0 as i64);
        let moved = |row: u32, col: u32| -> Option<(u32, u32)> {
            let (r, c) = (row as i64 + d_row, col as i64 + d_col);
            (r >= 0 && c >= 0 && r < MAX_ROWS as i64 && c < MAX_COLS as i64).then_some((r as u32, c as u32))
        };
        let own_sheet = same_name(&sh.name, &self.sheets[dest_sheet].name);
        // (group of the model, the copy)
        let mut copies: Vec<(usize, Sparkline)> = Vec::new();
        for (g, group) in sh.sparklines.iter().enumerate() {
            for s in group.sparklines.iter().filter(|s| src.contains(s.row, s.col)) {
                let Some((row, col)) = moved(s.row, s.col) else {
                    continue;
                };
                let mut data = s.data.clone();
                if !cut {
                    let (Some(a), Some(b)) = (moved(data.range.r0, data.range.c0), moved(data.range.r1, data.range.c1)) else {
                        continue;
                    };
                    data.range = Rect::new(a.0, a.1, b.0, b.1);
                }
                copies.push((g, Sparkline { row, col, data }));
            }
        }
        let landing = Rect::new(
            dest.0,
            dest.1,
            (dest.0 as u64 + src.rows() as u64 - 1).min(MAX_ROWS as u64 - 1) as u32,
            (dest.1 as u64 + src.cols() as u64 - 1).min(MAX_COLS as u64 - 1) as u32,
        );
        let covered = self.sheets[dest_sheet].sparklines.iter().any(|g| g.sparklines.iter().any(|s| landing.contains(s.row, s.col)));
        if copies.is_empty() && !covered {
            return;
        }
        self.snapshot();
        let models: Vec<SparklineGroup> = self.sheets[src_sheet].sparklines.iter().map(|g| g.with(Vec::new())).collect();
        if cut {
            self.sheets[src_sheet].drop_sparklines(|s| src.contains(s.row, s.col));
        }
        // What is pasted over goes.
        self.sheets[dest_sheet].drop_sparklines(|s| landing.contains(s.row, s.col));
        let mut new_groups: Vec<(usize, SparklineGroup)> = Vec::new();
        for (g, copy) in copies {
            let dest = &mut self.sheets[dest_sheet];
            // The group of the model, when it is on this sheet and still there.
            let home = own_sheet.then(|| dest.sparklines.iter_mut().find(|group| group.with(Vec::new()) == models[g])).flatten();
            match home {
                Some(group) => group.sparklines.push(copy),
                None => match new_groups.iter_mut().find(|(model, _)| *model == g) {
                    Some((_, group)) => group.sparklines.push(copy),
                    None => new_groups.push((g, models[g].with(vec![copy]))),
                },
            }
        }
        self.sheets[dest_sheet].sparklines.extend(new_groups.into_iter().map(|(_, group)| group));
    }

    /// A block of cells was filled from the cells it starts with (Fill
    /// Down, Fill Right, the fill handle): the cells of `dest` that have
    /// no sparkline take a copy of the nearest one before them in their
    /// column or, failing that, in their row.
    pub fn fill_sparklines(&mut self, sheet: usize, dest: Rect) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let inside: Vec<(u32, u32)> =
            sh.sparklines.iter().flat_map(|g| g.sparklines.iter().filter(|s| dest.contains(s.row, s.col)).map(|s| (s.row, s.col))).collect();
        if inside.is_empty() {
            return;
        }
        let has = |row: u32, col: u32| inside.contains(&(row, col));
        let mut plan: Vec<((u32, u32), (u32, u32))> = Vec::new();
        for row in dest.r0..=dest.r1 {
            for col in dest.c0..=dest.c1 {
                if has(row, col) {
                    continue;
                }
                let model = (dest.r0..row)
                    .rev()
                    .find(|r| has(*r, col))
                    .map(|r| (r, col))
                    .or_else(|| (dest.c0..col).rev().find(|c| has(row, *c)).map(|c| (row, c)))
                    .or_else(|| (row + 1..=dest.r1).find(|r| has(*r, col)).map(|r| (r, col)))
                    .or_else(|| (col + 1..=dest.c1).find(|c| has(row, *c)).map(|c| (row, c)));
                if let Some(model) = model {
                    plan.push((model, (row, col)));
                }
            }
        }
        for (model, cell) in plan {
            self.copy_sparklines(sheet, Rect::cell(model.0, model.1), sheet, cell, false);
        }
    }
}

// ------------------------------------------------------------------- hooks

/// Rows or columns were inserted or deleted: sparklines follow their
/// cells, and their data the cells it names. A sparkline goes with its
/// cell, or when all its data goes.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.sparklines.is_empty() {
        return;
    }
    let (rows, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    let max = if rows { MAX_ROWS } else { MAX_COLS };
    let shift = |lo: u32, hi: u32| shift_interval(lo, hi, insert, at, count, max);
    let own = same_name(&sheet.name, edited_sheet);
    for group in &mut sheet.sparklines {
        group.sparklines.retain_mut(|s| {
            if own {
                let moved = if rows { shift(s.row, s.row) } else { shift(s.col, s.col) };
                match moved {
                    Some((p, _)) if rows => s.row = p,
                    Some((p, _)) => s.col = p,
                    None => return false,
                }
            }
            if same_name(&s.data.sheet, edited_sheet) {
                let r = s.data.range;
                let moved = if rows { shift(r.r0, r.r1) } else { shift(r.c0, r.c1) };
                match moved {
                    Some((a, b)) if rows => s.data.range = Rect::new(a, r.c0, b, r.c1),
                    Some((a, b)) => s.data.range = Rect::new(r.r0, a, r.r1, b),
                    None => return false,
                }
            }
            true
        });
    }
    sheet.sparklines.retain(|group| !group.sparklines.is_empty());
}

pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for s in sheet.sparklines.iter_mut().flat_map(|group| group.sparklines.iter_mut()) {
        if same_name(&s.data.sheet, old) {
            s.data.sheet = new.to_string();
        }
    }
}

/// The sparklines of a copied sheet that plotted cells of the original
/// plot those of the copy.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    let name = wb.sheets[copy].name.clone();
    on_sheet_renamed(&mut wb.sheets[copy], original, &name);
}

/// The sparklines that plotted cells of a deleted sheet go with it.
pub(crate) fn on_sheet_deleted(wb: &mut Workbook, deleted: &Sheet) {
    for sheet in &mut wb.sheets {
        if !sheet.sparklines.is_empty() {
            sheet.drop_sparklines(|s| same_name(&s.data.sheet, &deleted.name));
        }
    }
}

/// Cells were shifted inside a band (table rows and columns coming and
/// going): the sparklines of the cells that moved follow them.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    let sh = &mut wb.sheets[sheet];
    if sh.sparklines.is_empty() {
        return;
    }
    let area = shift.area();
    for group in &mut sh.sparklines {
        group.sparklines.retain_mut(|s| {
            if !area.contains(s.row, s.col) {
                return true;
            }
            let along = if shift.down { s.row } else { s.col };
            match shift.moved(along) {
                Some(p) if shift.down => s.row = p,
                Some(p) => s.col = p,
                None => return false,
            }
            true
        });
    }
    sh.sparklines.retain(|group| !group.sparklines.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    /// Three rows of four numbers in A1:D3.
    fn book() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        let rows = [["1", "3", "2", "5"], ["4", "-2", "", "1"], ["2", "2", "2", "2"]];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("setup");
        wb
    }

    fn cells(wb: &Workbook, sheet: usize) -> Vec<(u32, u32, String)> {
        let mut out: Vec<(u32, u32, String)> = wb.sheets[sheet]
            .sparklines
            .iter()
            .flat_map(|g| g.sparklines.iter().map(|s| (s.row, s.col, s.data.range.name())))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn data_and_location_are_paired_as_excel_does() {
        let mut wb = book();
        // A column of cells beside the rows: one row each.
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
        wb.commit("Insert Sparkline");
        assert_eq!(cells(&wb, 0), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (2, 5, "A3:D3".into())]);
        assert_eq!(wb.sheets[0].sparkline_at(1, 5), Some((0, 1)));
        // A row of cells under the columns: one column each; what was in
        // a cell is replaced.
        wb.add_sparklines(0, SparklineType::Column, (0, rect("A1:D3")), rect("A5:D5")).unwrap();
        wb.add_sparklines(0, SparklineType::WinLoss, (0, rect("A2:D2")), rect("F2")).unwrap();
        wb.commit("Insert Sparkline");
        assert_eq!(wb.sheets[0].sparklines.len(), 3);
        assert_eq!(wb.sheets[0].sparklines[0].sparklines.len(), 2);
        assert_eq!(cells(&wb, 0)[3], (4, 0, "A1:A3".into()));
        assert!(wb.sheets[0].sparklines[2].negative && !wb.sheets[0].sparklines[1].negative);
        // What does not go together.
        let add = |wb: &mut Workbook, data: &str, location: &str| wb.add_sparklines(0, SparklineType::Line, (0, rect(data)), rect(location));
        assert_eq!(add(&mut wb, "A1:D3", "F1:G2"), Err(SparklineError::LocationShape));
        assert_eq!(add(&mut wb, "A1:D3", "F1:F2"), Err(SparklineError::Mismatch));
        assert_eq!(add(&mut wb, "A1:D3", "H1"), Err(SparklineError::Mismatch));
        assert_eq!(wb.add_sparklines(0, SparklineType::Line, (7, rect("A1:D1")), rect("H1")), Err(SparklineError::BadData));
        // A square of data along a row of as many cells goes by columns.
        let mut square = book();
        square.add_sparklines(0, SparklineType::Line, (0, rect("A1:C3")), rect("A5:C5")).unwrap();
        assert_eq!(cells(&square, 0)[0], (4, 0, "A1:A3".into()));
        assert!(wb.undo());
        assert_eq!(wb.sheets[0].sparklines.len(), 1);
    }

    #[test]
    fn values_follow_hidden_and_empty_cells() {
        let mut wb = book();
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A2:D2")), rect("F2")).unwrap();
        wb.commit("Insert Sparkline");
        let values = |wb: &Workbook| {
            let group = &wb.sheets[0].sparklines[0];
            wb.sparkline_values(group, &group.sparklines[0])
        };
        assert_eq!(values(&wb), [Some(4.0), Some(-2.0), None, Some(1.0)]);
        wb.update_sparkline_groups(0, &[rect("F2")], |g| g.empty = EmptyCells::Zero);
        assert_eq!(values(&wb), [Some(4.0), Some(-2.0), Some(0.0), Some(1.0)]);
        wb.set_cols_hidden(0, 1, 1, true);
        wb.commit("Hide");
        assert_eq!(values(&wb), [Some(4.0), Some(0.0), Some(1.0)]);
        wb.update_sparkline_groups(0, &[rect("F2")], |g| g.show_hidden = true);
        assert_eq!(values(&wb).len(), 4);
        // The picture follows the data.
        wb.set_cell_text(0, 1, 0, "40").unwrap();
        wb.commit("edit");
        assert_eq!(values(&wb)[0], Some(40.0));
        assert!(!wb.update_sparkline_groups(0, &[rect("A1")], |g| g.markers = true));
    }

    fn rects(shapes: &[SparkShape]) -> Vec<(f32, f32, f32, f32, Color)> {
        shapes
            .iter()
            .filter_map(|s| match s {
                SparkShape::Rect { x0, y0, x1, y1, color } => Some((*x0, *y0, *x1, *y1, *color)),
                _ => None,
            })
            .collect()
    }

    fn lines(shapes: &[SparkShape]) -> Vec<Vec<(f32, f32)>> {
        shapes
            .iter()
            .filter_map(|s| match s {
                SparkShape::Line { points, .. } => Some(points.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_line_with_gaps_markers_and_an_axis() {
        let mut group = SparklineGroup::new(SparklineType::Line);
        let values = [Some(4.0), Some(-2.0), None, Some(1.0), Some(1.0)];
        let drawn = shapes(&group, &values, SharedScale::default(), 106.0, 26.0, 1.0);
        // Two runs around the gap, inside the box, highest point on top.
        let runs = lines(&drawn);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], [(3.0, 3.0), (28.0, 23.0)]);
        assert_eq!(runs[1].len(), 2);
        assert_eq!(runs[1][1].0, 103.0);
        assert!(rects(&drawn).is_empty());
        // Connected over the gap: one run of four points.
        group.empty = EmptyCells::Connect;
        assert_eq!(lines(&shapes(&group, &values, SharedScale::default(), 106.0, 26.0, 1.0))[0].len(), 4);
        // Points that stand out, each in its colour; the axis at zero.
        group.high = true;
        group.negative = true;
        group.last = true;
        group.show_axis = true;
        group.colors = style(30);
        let marks = rects(&shapes(&group, &values, SharedScale::default(), 106.0, 26.0, 1.0));
        assert_eq!(marks.len(), 4);
        assert_eq!((marks[0].0, marks[0].2, marks[0].4), (3.0, 103.0, Color::BLACK));
        assert!((marks[0].1 - 16.0).abs() <= 1.0, "the axis is a third of the way up: {marks:?}");
        let colors: Vec<Color> = marks[1..].iter().map(|m| m.4).collect();
        assert_eq!(colors, [group.colors.high, group.colors.negative, group.colors.last]);
        // All the markers, and right to left.
        group.markers = true;
        group.right_to_left = true;
        let drawn = shapes(&group, &values, SharedScale::default(), 106.0, 26.0, 1.0);
        assert_eq!(rects(&drawn).len(), 5);
        assert_eq!(lines(&drawn)[0][0].1, lines(&drawn)[0][1].1);
        // A thicker line at a larger zoom.
        group.line_weight = 3.0;
        let SparkShape::Line { width, .. } = &shapes(&group, &values, SharedScale::default(), 212.0, 52.0, 2.0)[1] else {
            panic!("a line");
        };
        assert_eq!(*width, 8.0);
        assert!(shapes(&group, &[None, None], SharedScale::default(), 106.0, 26.0, 1.0).is_empty());
    }

    #[test]
    fn columns_and_win_loss() {
        let mut group = SparklineGroup::new(SparklineType::Column);
        let values = [Some(4.0), Some(-2.0), None, Some(1.0)];
        let bars = rects(&shapes(&group, &values, SharedScale::default(), 106.0, 36.0, 1.0));
        // Three columns standing on zero, the negative one hanging.
        assert_eq!(bars.len(), 3);
        let zero = 3.0 + 30.0 * 4.0 / 6.0;
        assert_eq!((bars[0].1, bars[0].3), (3.0, zero));
        assert_eq!((bars[1].1, bars[1].3), (zero, 33.0));
        assert!(bars[0].0 >= 3.0 && bars[0].2 <= 28.0 && bars[2].0 >= 78.0);
        assert!(bars.iter().all(|b| b.4 == group.colors.series));
        // All above zero: the smallest column still shows.
        let up = rects(&shapes(&group, &[Some(5.0), Some(7.0)], SharedScale::default(), 106.0, 36.0, 1.0));
        assert_eq!((up[0].3 - up[0].1, up[1].1), (1.0, 3.0));
        // A custom axis from zero gives it its true height.
        group.min = AxisBound::Custom(0.0);
        let up = rects(&shapes(&group, &[Some(5.0), Some(7.0)], SharedScale::default(), 106.0, 36.0, 1.0));
        assert!((up[0].3 - up[0].1 - 30.0 * 5.0 / 7.0).abs() < 0.01);
        // The same axis for the whole group.
        group.min = AxisBound::Group;
        group.max = AxisBound::Group;
        let shared = SharedScale {
            min: Some(0.0),
            max: Some(14.0),
        };
        let half = rects(&shapes(&group, &[Some(7.0)], shared, 106.0, 36.0, 1.0));
        assert_eq!(half[0].3 - half[0].1, 15.0);

        // Win/Loss: columns of one height, losses in their colour, the
        // axis between them, nothing for zero.
        let mut group = SparklineGroup::new(SparklineType::WinLoss);
        group.show_axis = true;
        let drawn = rects(&shapes(&group, &[Some(4.0), Some(-2.0), Some(0.0), Some(0.5)], SharedScale::default(), 106.0, 36.0, 1.0));
        assert_eq!(drawn.len(), 4);
        assert_eq!((drawn[0].1, drawn[0].3, drawn[0].4), (3.0, 18.0, group.colors.series));
        assert_eq!((drawn[1].1, drawn[1].3, drawn[1].4), (18.0, 33.0, group.colors.negative));
        assert_eq!((drawn[2].1, drawn[2].3), (3.0, 18.0));
        assert_eq!((drawn[3].1, drawn[3].4), (18.0, group.colors.axis));
    }

    #[test]
    fn groups_scale_edit_and_clear() {
        let mut wb = book();
        wb.add_sparklines(0, SparklineType::Column, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
        wb.update_sparkline_groups(0, &[rect("F2")], |g| {
            g.min = AxisBound::Group;
            g.max = AxisBound::Group;
        });
        wb.commit("Sparkline Axis Change");
        let scale = wb.sparkline_scale(&wb.sheets[0].sparklines[0]);
        assert_eq!((scale.min, scale.max), (Some(-2.0), Some(5.0)));
        // Ungroup, then group two of the three again.
        assert!(wb.ungroup_sparklines(0, &[rect("F1:F3")]));
        assert_eq!(wb.sheets[0].sparklines.len(), 3);
        wb.update_sparkline_groups(0, &[rect("F3")], |g| g.kind = SparklineType::Line);
        assert!(wb.group_sparklines(0, &[rect("F2:F3")], (2, 5)));
        assert_eq!(wb.sheets[0].sparklines.len(), 2);
        let (g, _) = wb.sheets[0].sparkline_at(1, 5).unwrap();
        assert_eq!((wb.sheets[0].sparklines[g].kind, wb.sheets[0].sparklines[g].sparklines.len()), (SparklineType::Line, 2));
        // Edit one sparkline's data, then the group's location and data.
        wb.edit_sparkline_data(0, 1, 5, (0, rect("A1:A3"))).unwrap();
        assert_eq!(cells(&wb, 0)[1], (1, 5, "A1:A3".into()));
        assert_eq!(wb.edit_sparkline_data(0, 1, 5, (0, rect("A1:B3"))), Err(SparklineError::Mismatch));
        let (g, _) = wb.sheets[0].sparkline_at(1, 5).unwrap();
        wb.edit_sparkline_group(0, g, (0, rect("A1:D2")), rect("G1:G2")).unwrap();
        assert_eq!(cells(&wb, 0), [(0, 5, "A1:D1".into()), (0, 6, "A1:D1".into()), (1, 6, "A2:D2".into())]);
        assert_eq!(wb.sheets[0].sparklines.last().unwrap().kind, SparklineType::Line);
        // Clear one sparkline, then a whole group.
        assert!(wb.clear_sparklines(0, &[rect("G1")], false));
        assert_eq!(cells(&wb, 0).len(), 2);
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("H1:H3")).unwrap();
        assert!(wb.clear_sparklines(0, &[rect("H2")], true));
        assert_eq!(cells(&wb, 0).len(), 2);
        assert!(!wb.clear_sparklines(0, &[rect("A1:B9")], true));
    }

    #[test]
    fn sparklines_follow_rows_columns_and_sheets() {
        let mut wb = book();
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
        wb.commit("Insert Sparkline");
        wb.insert_rows(0, 1, 2);
        wb.insert_cols(0, 2, 1);
        wb.commit("Insert");
        assert_eq!(cells(&wb, 0), [(0, 6, "A1:E1".into()), (3, 6, "A4:E4".into()), (4, 6, "A5:E5".into())]);
        // The row of a sparkline goes: so does the sparkline. All the
        // data of another goes: that one too.
        wb.delete_rows(0, 0, 1);
        wb.commit("Delete");
        assert_eq!(cells(&wb, 0), [(2, 6, "A3:E3".into()), (3, 6, "A4:E4".into())]);
        wb.delete_cols(0, 1, 2);
        wb.commit("Delete");
        assert_eq!(cells(&wb, 0), [(2, 4, "A3:C3".into()), (3, 4, "A4:C4".into())]);
        assert!(wb.undo() && wb.undo());
        assert_eq!(cells(&wb, 0).len(), 3);

        // Data on another sheet: it follows that sheet's rows and name.
        let other = wb.add_sheet(1);
        wb.add_sparklines(other, SparklineType::Line, (0, rect("A1:E1")), rect("B2")).unwrap();
        wb.commit("Insert Sparkline");
        wb.insert_rows(0, 0, 1);
        assert!(wb.rename_sheet(0, "Data"));
        wb.commit("Rename");
        let s = wb.sheets[other].sparklines[0].sparklines[0].clone();
        assert_eq!((s.data.sheet.as_str(), s.data.range), ("Data", rect("A2:E2")));
        // A copy of a sheet plots its own cells.
        let copy = wb.duplicate_sheet(0).unwrap();
        wb.commit("Copy");
        let copied = &wb.sheets[copy].sparklines[0].sparklines[0];
        assert_eq!(copied.data.sheet, wb.sheets[copy].name);
        // The sheet of the data is deleted: its sparklines elsewhere go.
        let other = wb.sheet_index("Sheet2").unwrap();
        assert!(wb.delete_sheet(0));
        wb.commit("Delete Sheet");
        assert!(wb.sheets[other - 1].sparklines.is_empty());
    }

    #[test]
    fn copies_and_fills() {
        let mut wb = book();
        wb.add_sparklines(0, SparklineType::Column, (0, rect("A1:D1")), rect("F1")).unwrap();
        wb.commit("Insert Sparkline");
        // Filled down: the copies join the group and plot their own rows.
        wb.fill_sparklines(0, rect("F1:F3"));
        wb.commit("Fill");
        assert_eq!(wb.sheets[0].sparklines.len(), 1);
        assert_eq!(cells(&wb, 0), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (2, 5, "A3:D3".into())]);
        // Copied two columns to the right: data two columns to the right.
        wb.copy_sparklines(0, rect("F1:F2"), 0, (5, 7), false);
        wb.commit("Paste");
        assert_eq!(cells(&wb, 0)[3..], [(5, 7, "C6:F6".into()), (6, 7, "C7:F7".into())]);
        assert_eq!(wb.sheets[0].sparklines.len(), 1);
        // Cut: the sparkline moves and keeps its data; what it lands on goes.
        wb.copy_sparklines(0, rect("F3"), 0, (5, 7), true);
        wb.commit("Paste");
        assert_eq!(cells(&wb, 0), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (5, 7, "A3:D3".into()), (6, 7, "C7:F7".into())]);
        // A copy that would plot cells off the sheet is not made.
        let other = wb.add_sheet(1);
        wb.copy_sparklines(0, rect("F1:F2"), other, (0, 0), false);
        assert!(wb.sheets[other].sparklines.is_empty());
        // To another sheet: a group with the same settings there.
        wb.copy_sparklines(0, rect("F1:F2"), other, (0, 9), false);
        wb.commit("Paste");
        assert_eq!(cells(&wb, other), [(0, 9, "E1:H1".into()), (1, 9, "E2:H2".into())]);
        assert_eq!(wb.sheets[other].sparklines.len(), 1);
        assert_eq!(wb.sheets[other].sparklines[0].kind, SparklineType::Column);
        // Pasting plain cells over sparklines takes them away.
        wb.copy_sparklines(0, rect("A1:A2"), other, (0, 9), false);
        assert!(wb.sheets[other].sparklines.is_empty());
    }

    #[test]
    fn the_gallery_has_thirty_six_styles() {
        let all: Vec<SparklineColors> = (0..STYLE_COUNT).map(style).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|b| a != b), "style {i} is unlike the others");
            assert_eq!(a.axis, Color::BLACK);
        }
        // Accent 1, darker 50 %; the plain accent; the style of new sparklines.
        assert_eq!(all[0].series, Color(0x203864));
        assert_eq!(all[12].series, Color(0x4472C4));
        assert_eq!(all[13].negative, Color(0xA5A5A5));
        assert_eq!(all[28], SparklineColors::default());
        assert_eq!(tinted(4, 0.39997558519241921), Color(0x8FAADC));
    }
}
