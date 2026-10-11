//! Charts: the model stored in a sheet, building a chart from a selection,
//! and resolving the data a chart plots. Drawing is done by the application.

use crate::ast::{CellRef, Expr, RefExpr, RefKind, StructuralEdit};
use crate::format;
use crate::refs::Rect;
use crate::style::Color;
use crate::value::Value;
use crate::workbook::Workbook;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartKind {
    /// Vertical bars, one cluster per category.
    Column,
    StackedColumn,
    /// Horizontal bars.
    Bar,
    StackedBar,
    Line,
    Area,
    Pie,
    Doughnut,
    /// X/Y points: the first range holds the X values.
    Scatter,
}

impl ChartKind {
    pub const ALL: [ChartKind; 9] = [
        ChartKind::Column,
        ChartKind::StackedColumn,
        ChartKind::Bar,
        ChartKind::StackedBar,
        ChartKind::Line,
        ChartKind::Area,
        ChartKind::Pie,
        ChartKind::Doughnut,
        ChartKind::Scatter,
    ];

    /// One series shown as shares of a whole.
    pub fn is_round(self) -> bool {
        matches!(self, ChartKind::Pie | ChartKind::Doughnut)
    }

    pub fn is_stacked(self) -> bool {
        matches!(self, ChartKind::StackedColumn | ChartKind::StackedBar)
    }

    /// Categories run along the vertical axis.
    pub fn is_horizontal(self) -> bool {
        matches!(self, ChartKind::Bar | ChartKind::StackedBar)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SeriesName {
    None,
    Text(String),
    /// The name is read from a cell.
    Cell(RefExpr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: SeriesName,
    /// The plotted numbers.
    pub values: RefExpr,
    /// Category labels, or the X values of a scatter chart.
    pub categories: Option<RefExpr>,
    /// `None` = automatic colour from the palette.
    pub color: Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegendPos {
    None,
    Bottom,
    Right,
    Top,
    Left,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChartTitle {
    None,
    /// The series name when there is a single series, else a placeholder.
    Auto,
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    /// Unique within the workbook (assigned by `Workbook::add_chart`).
    pub id: u32,
    pub kind: ChartKind,
    pub title: ChartTitle,
    pub legend: LegendPos,
    pub series: Vec<Series>,
    pub data_labels: bool,
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell, pixels at 100 % (x, y).
    pub offset: (f32, f32),
    /// Width and height, pixels at 100 %.
    pub size: (f32, f32),
}

/// Default chart size (12.7 cm × 7.62 cm, as in Excel).
pub const DEFAULT_CHART_SIZE: (f32, f32) = (480.0, 288.0);

/// Automatic series colours (Office theme accents).
pub const PALETTE: [u32; 6] = [0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E];

/// Colour of the `index`-th series / pie slice; later rounds are lightened
/// or darkened variants of the six accents, as Office does.
pub fn palette_color(index: usize) -> Color {
    let base = PALETTE[index % PALETTE.len()];
    let round = index / PALETTE.len();
    let amount: f32 = match round % 6 {
        0 => 0.0,
        1 => 0.4,
        2 => -0.25,
        3 => 0.6,
        4 => -0.5,
        _ => 0.2,
    };
    let ch = |shift: u32| -> u32 {
        let v = ((base >> shift) & 0xFF) as f32;
        let out = if amount >= 0.0 {
            v + (255.0 - v) * amount
        } else {
            v * (1.0 + amount)
        };
        (out.round().clamp(0.0, 255.0) as u32) << shift
    };
    Color(ch(16) | ch(8) | ch(0))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeriesData {
    pub name: String,
    /// One entry per point; `None` for blanks and text.
    pub values: Vec<Option<f64>>,
    pub color: Color,
}

/// What a chart plots, read from the workbook.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChartData {
    /// Category labels (formatted cell text); numbered 1.. when absent.
    pub categories: Vec<String>,
    /// X values (scatter charts).
    pub x_values: Vec<Option<f64>>,
    pub series: Vec<SeriesData>,
    /// Number format of the plotted values (for axis and data labels).
    pub value_format: String,
    /// Number format of the X values (scatter charts).
    pub x_format: String,
    /// Title to display, when the chart has one that can be resolved.
    pub title: Option<String>,
}

/// A value axis: bounds and the distance between gridlines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisScale {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl AxisScale {
    pub fn ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if self.step <= 0.0 || !self.step.is_finite() {
            return vec![self.min, self.max];
        }
        let n = ((self.max - self.min) / self.step).round() as i64;
        for i in 0..=n.clamp(1, 200) {
            let v = self.min + self.step * i as f64;
            // Remove float noise (0.30000000000000004).
            out.push((v / self.step).round() * self.step);
        }
        out
    }

    /// Position of a value on the axis, 0 at `min` and 1 at `max`.
    pub fn fraction(&self, v: f64) -> f64 {
        if self.max == self.min {
            0.0
        } else {
            (v - self.min) / (self.max - self.min)
        }
    }
}

/// A readable axis for data between `lo` and `hi`: bounds on round
/// numbers, zero included unless the data sits far from it, and room above
/// the largest value.
pub fn nice_axis(lo: f64, hi: f64) -> AxisScale {
    let (mut lo, mut hi) = if lo.is_finite() && hi.is_finite() && lo <= hi {
        (lo, hi)
    } else {
        (0.0, 1.0)
    };
    // Like Excel, start at zero unless the values vary by less than a sixth
    // of their size (then zooming in on them reads better).
    if lo > 0.0 && (hi - lo) / hi > 1.0 / 6.0 {
        lo = 0.0;
    }
    if hi < 0.0 && (hi - lo) / lo.abs() > 1.0 / 6.0 {
        hi = 0.0;
    }
    if hi == lo {
        if hi == 0.0 {
            hi = 1.0;
        } else if hi > 0.0 {
            lo = 0.0;
        } else {
            hi = 0.0;
        }
    }
    let raw = (hi - lo) / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = mag
        * if norm <= 1.0 {
            1.0
        } else if norm <= 2.0 {
            2.0
        } else if norm <= 5.0 {
            5.0
        } else {
            10.0
        };
    let min = (lo / step).floor() * step;
    let mut max = (hi / step).ceil() * step;
    // Keep the tallest point clear of the top edge.
    if hi > 0.0 && max - hi < step * 0.05 {
        max += step;
    }
    let mut min = min;
    if lo < 0.0 && lo - min < step * 0.05 {
        min -= step;
    }
    AxisScale { min, max, step }
}

fn abs_cell(row: u32, col: u32) -> CellRef {
    CellRef {
        row,
        col,
        abs_row: true,
        abs_col: true,
    }
}

fn abs_ref(sheet: &str, rect: Rect) -> RefExpr {
    RefExpr {
        sheet: Some(Arc::from(sheet)),
        start: abs_cell(rect.r0, rect.c0),
        end: (!rect.is_single()).then(|| abs_cell(rect.r1, rect.c1)),
        kind: RefKind::Cells,
    }
}

impl Chart {
    /// Rewrite every range the chart reads. `f` returns `None` when the
    /// range no longer exists; a series whose values are gone is dropped.
    pub fn map_refs(&mut self, f: &mut dyn FnMut(&RefExpr) -> Option<RefExpr>) {
        self.series.retain_mut(|s| {
            match f(&s.values) {
                Some(r) => s.values = r,
                None => return false,
            }
            if let Some(c) = &s.categories {
                s.categories = f(c);
            }
            if let SeriesName::Cell(c) = &s.name {
                s.name = match f(c) {
                    Some(r) => SeriesName::Cell(r),
                    None => SeriesName::None,
                };
            }
            true
        });
    }

    /// Follow rows / columns inserted into or deleted from `edited_sheet`.
    /// `own_sheet` is the sheet holding the chart.
    pub fn adjust(&mut self, edit: StructuralEdit, edited_sheet: &str, own_sheet: &str) {
        self.map_refs(&mut |r| {
            match Expr::Ref(r.clone()).adjusted(edit, edited_sheet, own_sheet) {
                Expr::Ref(r) => Some(r),
                _ => None,
            }
        });
        if !edited_sheet.eq_ignore_ascii_case(own_sheet) {
            return;
        }
        let shift = |pos: u32, insert: bool, at: u32, count: u32| -> u32 {
            if insert {
                if pos >= at { pos + count } else { pos }
            } else if pos >= at + count {
                pos - count
            } else if pos >= at {
                at
            } else {
                pos
            }
        };
        match edit {
            StructuralEdit::InsertRows { at, count } => self.anchor.0 = shift(self.anchor.0, true, at, count),
            StructuralEdit::DeleteRows { at, count } => self.anchor.0 = shift(self.anchor.0, false, at, count),
            StructuralEdit::InsertCols { at, count } => self.anchor.1 = shift(self.anchor.1, true, at, count),
            StructuralEdit::DeleteCols { at, count } => self.anchor.1 = shift(self.anchor.1, false, at, count),
        }
    }

    pub fn rename_sheet(&mut self, old: &str, new: &str) {
        self.map_refs(&mut |r| {
            let mut out = r.clone();
            if r.sheet.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(old)) {
                out.sheet = Some(Arc::from(new));
            }
            Some(out)
        });
    }

    /// Swap the roles of rows and columns of the source block ("Switch
    /// Row/Column"). Only possible when the series form one rectangle.
    pub fn switch_row_column(&mut self, wb: &Workbook, own_sheet: usize) -> bool {
        let Some(first) = self.series.first() else {
            return false;
        };
        let sheet_name = first.values.sheet.clone();
        let mut block = first.values.rect();
        for s in &self.series {
            if s.values.sheet != sheet_name {
                return false;
            }
            block = block.union(&s.values.rect());
            if let Some(c) = &s.categories {
                block = block.union(&c.rect());
            }
            if let SeriesName::Cell(c) = &s.name {
                block = block.union(&c.rect());
            }
        }
        let sheet = match &sheet_name {
            Some(name) => match wb.sheet_index(name) {
                Some(i) => i,
                None => return false,
            },
            None => own_sheet,
        };
        let by_rows = first.values.rect().rows() == 1 && first.values.rect().cols() > 1;
        let rebuilt = wb.chart_from_block(sheet, block, self.kind, Some(!by_rows));
        if rebuilt.series.is_empty() {
            return false;
        }
        self.series = rebuilt.series;
        true
    }
}

impl Workbook {
    fn resolve_chart_ref(&self, r: &RefExpr, own_sheet: usize) -> Option<(usize, Rect)> {
        let sheet = match &r.sheet {
            Some(name) => self.sheet_index(name)?,
            None => own_sheet,
        };
        Some((sheet, self.clamp_to_used(sheet, r.rect())))
    }

    /// Build a chart of the given kind from a selection, guessing which
    /// row / column holds names and categories the way Excel does. The
    /// chart is not added to the sheet.
    pub fn chart_from_range(&self, sheet: usize, rect: Rect, kind: ChartKind) -> Chart {
        let rect = if rect.is_single() {
            self.current_region(sheet, rect.r0, rect.c0)
        } else {
            self.clamp_to_used(sheet, rect)
        };
        self.chart_from_block(sheet, rect, kind, None)
    }

    /// `by_rows`: force series to run along rows (`Some(true)`) or columns
    /// (`Some(false)`); `None` picks the orientation with fewer series.
    pub fn chart_from_block(&self, sheet: usize, rect: Rect, kind: ChartKind, by_rows: Option<bool>) -> Chart {
        let mut chart = Chart {
            id: 0,
            kind,
            title: ChartTitle::Auto,
            legend: LegendPos::None,
            series: Vec::new(),
            data_labels: false,
            anchor: (0, 0),
            offset: (0.0, 0.0),
            size: DEFAULT_CHART_SIZE,
        };
        let Some(sh) = self.sheets.get(sheet) else {
            return chart;
        };
        let name = sh.name.as_str();
        // A cell is a label when it holds text or a date; numbers are data.
        let is_number = |r: u32, c: u32| match sh.value_at(r, c) {
            Value::Number(_) => !format::is_date_format(&self.styles.get(sh.style_id_at(r, c)).num_fmt),
            _ => false,
        };
        let is_blank = |r: u32, c: u32| sh.is_blank(r, c);

        let header_row = rect.rows() > 1
            && (rect.c0..=rect.c1).all(|c| !is_number(rect.r0, c))
            && (rect.c0..=rect.c1).any(|c| !is_blank(rect.r0, c));
        let first_data_row = if header_row { rect.r0 + 1 } else { rect.r0 };
        let label_col = rect.cols() > 1
            && kind != ChartKind::Scatter
            && (first_data_row..=rect.r1).all(|r| !is_number(r, rect.c0))
            && (first_data_row..=rect.r1).any(|r| !is_blank(r, rect.c0));
        let first_data_col = if label_col { rect.c0 + 1 } else { rect.c0 };
        if first_data_row > rect.r1 || first_data_col > rect.c1 {
            return chart;
        }
        let data = Rect::new(first_data_row, first_data_col, rect.r1, rect.c1);

        if kind == ChartKind::Scatter && data.cols() >= 2 {
            // First column: X values; every other column is a Y series.
            let x = abs_ref(name, Rect::new(data.r0, data.c0, data.r1, data.c0));
            for c in data.c0 + 1..=data.c1 {
                chart.series.push(Series {
                    name: if header_row {
                        SeriesName::Cell(abs_ref(name, Rect::cell(rect.r0, c)))
                    } else {
                        SeriesName::None
                    },
                    values: abs_ref(name, Rect::new(data.r0, c, data.r1, c)),
                    categories: Some(x.clone()),
                    color: None,
                });
            }
        } else {
            let in_rows = by_rows.unwrap_or(data.cols() > data.rows());
            if in_rows {
                for r in data.r0..=data.r1 {
                    chart.series.push(Series {
                        name: if label_col {
                            SeriesName::Cell(abs_ref(name, Rect::cell(r, rect.c0)))
                        } else {
                            SeriesName::None
                        },
                        values: abs_ref(name, Rect::new(r, data.c0, r, data.c1)),
                        categories: header_row.then(|| abs_ref(name, Rect::new(rect.r0, data.c0, rect.r0, data.c1))),
                        color: None,
                    });
                }
            } else {
                for c in data.c0..=data.c1 {
                    chart.series.push(Series {
                        name: if header_row {
                            SeriesName::Cell(abs_ref(name, Rect::cell(rect.r0, c)))
                        } else {
                            SeriesName::None
                        },
                        values: abs_ref(name, Rect::new(data.r0, c, data.r1, c)),
                        categories: label_col.then(|| abs_ref(name, Rect::new(data.r0, rect.c0, data.r1, rect.c0))),
                        color: None,
                    });
                }
            }
        }
        // Columns without a single number are not worth a series.
        chart.series.retain(|s| {
            let r = s.values.rect();
            (r.r0..=r.r1).any(|row| (r.c0..=r.c1).any(|col| matches!(sh.value_at(row, col), Value::Number(_))))
        });
        if kind.is_round() {
            chart.series.truncate(1);
        }
        chart.legend = if kind.is_round() || chart.series.len() > 1 {
            LegendPos::Bottom
        } else {
            LegendPos::None
        };
        chart
    }

    /// Read the data a chart plots.
    pub fn chart_data(&self, own_sheet: usize, chart: &Chart) -> ChartData {
        let mut data = ChartData {
            value_format: "General".to_string(),
            x_format: "General".to_string(),
            ..Default::default()
        };
        let mut have_format = false;
        let mut points = 0usize;
        for (i, s) in chart.series.iter().enumerate() {
            let mut values = Vec::new();
            if let Some((si, rect)) = self.resolve_chart_ref(&s.values, own_sheet) {
                let sh = &self.sheets[si];
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        let v = sh.value_at(r, c);
                        if !have_format && matches!(v, Value::Number(_)) {
                            data.value_format = self.styles.get(sh.style_id_at(r, c)).num_fmt.to_string();
                            have_format = true;
                        }
                        values.push(match v {
                            Value::Number(n) if n.is_finite() => Some(n),
                            _ => None,
                        });
                    }
                }
            }
            points = points.max(values.len());
            let name = match &s.name {
                SeriesName::None => String::new(),
                SeriesName::Text(t) => t.clone(),
                SeriesName::Cell(r) => match self.resolve_chart_ref(r, own_sheet) {
                    Some((si, rect)) => self.display_text(si, rect.r0, rect.c0).text,
                    None => String::new(),
                },
            };
            data.series.push(SeriesData {
                name,
                values,
                color: s.color.unwrap_or_else(|| palette_color(i)),
            });
        }
        // Categories / X values come from the first series that has them.
        if let Some(cats) = chart.series.iter().find_map(|s| s.categories.as_ref()) {
            if let Some((si, rect)) = self.resolve_chart_ref(cats, own_sheet) {
                let sh = &self.sheets[si];
                let mut first = true;
                for r in rect.r0..=rect.r1 {
                    for c in rect.c0..=rect.c1 {
                        let v = sh.value_at(r, c);
                        if first && matches!(v, Value::Number(_)) {
                            data.x_format = self.styles.get(sh.style_id_at(r, c)).num_fmt.to_string();
                            first = false;
                        }
                        data.x_values.push(match v {
                            Value::Number(n) if n.is_finite() => Some(n),
                            _ => None,
                        });
                        data.categories.push(self.display_text(si, r, c).text);
                    }
                }
            }
        }
        if data.categories.len() < points {
            for i in data.categories.len()..points {
                data.categories.push((i + 1).to_string());
            }
        }
        if data.x_values.len() < points {
            for i in data.x_values.len()..points {
                data.x_values.push(Some(i as f64 + 1.0));
            }
        }
        data.title = match &chart.title {
            ChartTitle::None => None,
            ChartTitle::Text(t) => Some(t.clone()),
            ChartTitle::Auto => match data.series.as_slice() {
                [only] if !only.name.is_empty() => Some(only.name.clone()),
                _ => None,
            },
        };
        data
    }

    /// The ranges a chart reads on `sheet`, for highlighting them when the
    /// chart is selected: (rectangle, role) with role 0 = values, 1 =
    /// categories, 2 = series names.
    pub fn chart_ranges(&self, sheet: usize, chart: &Chart) -> Vec<(Rect, u8)> {
        let mut out: Vec<(Rect, u8)> = Vec::new();
        let mut push = |r: &RefExpr, role: u8, out: &mut Vec<(Rect, u8)>| {
            if let Some((si, rect)) = self.resolve_chart_ref(r, sheet) {
                if si == sheet && !out.iter().any(|(x, _)| *x == rect) {
                    out.push((rect, role));
                }
            }
        };
        for s in &chart.series {
            push(&s.values, 0, &mut out);
            if let Some(c) = &s.categories {
                push(c, 1, &mut out);
            }
            if let SeriesName::Cell(c) = &s.name {
                push(c, 2, &mut out);
            }
        }
        out
    }

    fn next_chart_id(&self) -> u32 {
        self.sheets
            .iter()
            .flat_map(|s| s.charts.iter().map(|c| c.id))
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Add a chart to a sheet; returns its id.
    pub fn add_chart(&mut self, sheet: usize, mut chart: Chart) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        self.snapshot();
        chart.id = self.next_chart_id();
        let id = chart.id;
        self.sheets[sheet].charts.push(chart);
        Some(id)
    }

    pub fn remove_chart(&mut self, sheet: usize, id: u32) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        if !sh.charts.iter().any(|c| c.id == id) {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].charts.retain(|c| c.id != id);
        true
    }

    pub fn chart(&self, sheet: usize, id: u32) -> Option<&Chart> {
        self.sheets.get(sheet)?.charts.iter().find(|c| c.id == id)
    }

    /// Change a chart (recorded for undo).
    pub fn update_chart(&mut self, sheet: usize, id: u32, f: impl FnOnce(&mut Chart)) -> bool {
        if self.chart(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        if let Some(chart) = self.sheets[sheet].charts.iter_mut().find(|c| c.id == id) {
            f(chart);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn table() -> Workbook {
        wb(&[
            ("A1", "Product"),
            ("B1", "Q1"),
            ("C1", "Q2"),
            ("A2", "Keyboard"),
            ("B2", "10"),
            ("C2", "15"),
            ("A3", "Mouse"),
            ("B3", "20"),
            ("C3", "5"),
            ("A4", "Screen"),
            ("B4", "30"),
            ("C4", "45"),
        ])
    }

    #[test]
    fn chart_from_table() {
        let w = table();
        let chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        assert_eq!(chart.series.len(), 2);
        assert_eq!(chart.series[0].values.to_text(), "Sheet1!$B$2:$B$4");
        assert_eq!(chart.series[0].categories.as_ref().unwrap().to_text(), "Sheet1!$A$2:$A$4");
        assert_eq!(chart.legend, LegendPos::Bottom);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.categories, vec!["Keyboard", "Mouse", "Screen"]);
        assert_eq!(data.series[0].name, "Q1");
        assert_eq!(data.series[1].values, vec![Some(15.0), Some(5.0), Some(45.0)]);
        assert_eq!(data.title, None);

        // A single cell expands to the block around it; a pie keeps one series.
        let pie = w.chart_from_range(0, Rect::parse("B2").unwrap(), ChartKind::Pie);
        assert_eq!(pie.series.len(), 1);
        assert_eq!(w.chart_data(0, &pie).title.as_deref(), Some("Q1"));
    }

    #[test]
    fn orientation_and_switch() {
        // Wide block: more columns than rows → one series per row.
        let w = wb(&[
            ("A1", ""),
            ("B1", "Jan"),
            ("C1", "Feb"),
            ("D1", "Mar"),
            ("A2", "North"),
            ("B2", "1"),
            ("C2", "2"),
            ("D2", "3"),
            ("A3", "South"),
            ("B3", "4"),
            ("C3", "5"),
            ("D3", "6"),
        ]);
        let mut chart = w.chart_from_range(0, Rect::parse("A1:D3").unwrap(), ChartKind::Line);
        assert_eq!(chart.series.len(), 2);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.series[0].name, "North");
        assert_eq!(data.categories, vec!["Jan", "Feb", "Mar"]);
        assert!(chart.switch_row_column(&w, 0));
        assert_eq!(chart.series.len(), 3);
        assert_eq!(w.chart_data(0, &chart).series[0].name, "Jan");
    }

    #[test]
    fn scatter_uses_first_column_as_x() {
        let w = wb(&[("A1", "x"), ("B1", "y"), ("A2", "1"), ("B2", "10"), ("A3", "2"), ("B3", "20")]);
        let chart = w.chart_from_range(0, Rect::parse("A1:B3").unwrap(), ChartKind::Scatter);
        assert_eq!(chart.series.len(), 1);
        let data = w.chart_data(0, &chart);
        assert_eq!(data.x_values, vec![Some(1.0), Some(2.0)]);
        assert_eq!(data.series[0].values, vec![Some(10.0), Some(20.0)]);
    }

    #[test]
    fn axis() {
        assert_eq!(nice_axis(0.0, 2023.0), AxisScale { min: 0.0, max: 2500.0, step: 500.0 });
        assert_eq!(nice_axis(5.0, 45.0), AxisScale { min: 0.0, max: 50.0, step: 10.0 });
        assert_eq!(nice_axis(-20.0, 30.0).min, -20.0 - 10.0);
        let a = nice_axis(0.0, 0.0);
        assert!(a.max > a.min);
        let a = nice_axis(98.0, 102.0);
        assert!(a.min > 0.0 && a.min <= 98.0 && a.max >= 102.0);
        // A value sitting exactly on a gridline gets one more step of headroom.
        assert_eq!(nice_axis(0.0, 1.0).ticks().len(), 7);
        assert_eq!(nice_axis(0.0, 0.9).ticks().len(), 6);
    }

    #[test]
    fn charts_follow_edits_and_undo() {
        let mut w = table();
        let chart = w.chart_from_range(0, Rect::parse("A1:C4").unwrap(), ChartKind::Column);
        let id = w.add_chart(0, chart).unwrap();
        w.commit("chart");
        w.insert_rows(0, 0, 2);
        w.commit("insert");
        let chart = w.chart(0, id).unwrap();
        assert_eq!(chart.series[0].values.to_text(), "Sheet1!$B$4:$B$6");
        assert_eq!(chart.anchor.0, 2);
        assert!(w.rename_sheet(0, "Data"));
        w.commit("rename");
        assert_eq!(w.chart(0, id).unwrap().series[0].values.to_text(), "Data!$B$4:$B$6");
        assert_eq!(w.chart_data(0, w.chart(0, id).unwrap()).series[0].values[0], Some(10.0));
        w.undo();
        w.undo();
        assert_eq!(w.chart(0, id).unwrap().series[0].values.to_text(), "Sheet1!$B$2:$B$4");
        w.undo();
        assert!(w.chart(0, id).is_none());
        w.redo();
        assert!(w.chart(0, id).is_some());
    }
}
