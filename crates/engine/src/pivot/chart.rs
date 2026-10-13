//! PivotCharts: charts bound to a PivotTable.
//!
//! The chart itself is an ordinary chart of the sheet. What makes it a
//! PivotChart is a link kept by its PivotTable (`PivotTable::charts`):
//! every time the report is written, the series of the linked charts are
//! made again from it — one series per column of values, one category per
//! row of items, totals left out —, as ranges over the cells of the
//! report, which is how files store them too. Formatting given to a
//! series stays with the series at the same place.

use super::{PivotError, PivotTable, ReportLineKind};
use crate::ast::{Expr, RefExpr, write_sheet_name};
use crate::chart::{Chart, ChartKind, LegendPos, Series, SeriesName};
use crate::locale::Locale;
use crate::parser::parse_formula;
use crate::refs::{Rect, col_to_letters};
use crate::workbook::Workbook;

/// The four kinds of field buttons of a PivotChart ("Field Buttons" of
/// the PivotChart Analyze tab shows and hides each kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChartButtonKind {
    /// "Report Filter": the filter fields.
    Filter,
    /// "Legend (Series)": the column fields.
    Legend,
    /// "Axis (Categories)": the row fields.
    Axis,
    /// "Values": the value fields.
    Value,
}

impl ChartButtonKind {
    pub const ALL: [ChartButtonKind; 4] = [ChartButtonKind::Filter, ChartButtonKind::Legend, ChartButtonKind::Axis, ChartButtonKind::Value];
}

/// A chart bound to a PivotTable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PivotChartLink {
    /// `Chart::id`.
    pub chart: u32,
    /// The kinds of field buttons shown on the chart, in the order of
    /// [`ChartButtonKind::ALL`].
    pub shown: [bool; 4],
}

impl PivotChartLink {
    /// A link showing every field button, as a new PivotChart does.
    pub fn new(chart: u32) -> PivotChartLink {
        PivotChartLink { chart, shown: [true; 4] }
    }

    pub fn shows(&self, kind: ChartButtonKind) -> bool {
        self.shown[kind as usize]
    }

    /// Some field buttons are shown.
    pub fn buttons(&self) -> bool {
        self.shown.iter().any(|shown| *shown)
    }
}

/// The fields a PivotChart shows as buttons, by zone: the index of the
/// field (of the value field, for the values) and its caption.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PivotChartFields {
    /// "Report Filter": the filter fields.
    pub filters: Vec<(usize, String)>,
    /// "Legend (Series)": the column fields.
    pub legend: Vec<(usize, String)>,
    /// "Axis (Categories)": the row fields.
    pub axis: Vec<(usize, String)>,
    /// "Values": the value fields.
    pub values: Vec<(usize, String)>,
}

impl PivotChartFields {
    pub fn of(&self, kind: ChartButtonKind) -> &[(usize, String)] {
        match kind {
            ChartButtonKind::Filter => &self.filters,
            ChartButtonKind::Legend => &self.legend,
            ChartButtonKind::Axis => &self.axis,
            ChartButtonKind::Value => &self.values,
        }
    }
}

/// The points a PivotChart plots among the rows its series cover (they
/// run from the first item of the report to the last, and the lines of
/// outer items and subtotals between them are not points), and their
/// labels, one level per row field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PivotChartPoints {
    /// Positions, in the ranges of the series, of the rows plotted.
    pub keep: Vec<usize>,
    /// The labels next to the plot: the innermost item of each row.
    pub labels: Vec<String>,
    /// The labels of the outer row fields, the one next to `labels`
    /// first; empty where the row continues the group of the row before.
    pub outer_levels: Vec<Vec<String>>,
}

/// False for the chart types a PivotChart cannot have: XY (scatter),
/// bubble and stock.
pub fn chart_kind_allowed(kind: ChartKind) -> bool {
    // The engine has no stock charts; scatter and bubble plot numbers
    // against numbers, which a report does not hold.
    !kind.is_xy()
}

fn range(sheet: &str, rect: Rect) -> Option<RefExpr> {
    let mut text = String::new();
    write_sheet_name(sheet, &mut text);
    let cell = |r: u32, c: u32| format!("${}${}", col_to_letters(c), r + 1);
    text.push('!');
    text.push_str(&cell(rect.r0, rect.c0));
    if !rect.is_single() {
        text.push(':');
        text.push_str(&cell(rect.r1, rect.c1));
    }
    match parse_formula(&text, Locale::en()) {
        Ok(Expr::Ref(r)) => Some(r),
        _ => None,
    }
}

impl PivotTable {
    /// The captions of the field buttons of a PivotChart of this table.
    pub fn chart_fields(&self) -> PivotChartFields {
        let captions = |list: &[super::AxisField]| -> Vec<(usize, String)> {
            list.iter().filter_map(|a| a.field()).map(|f| (f, self.field_caption(f).to_string())).collect()
        };
        PivotChartFields {
            filters: self.page_fields.iter().map(|p| (p.field, self.field_caption(p.field).to_string())).collect(),
            legend: captions(&self.col_fields),
            axis: captions(&self.row_fields),
            values: self.data_fields.iter().enumerate().map(|(k, d)| (k, d.name.clone())).collect(),
        }
    }

    /// The rows a chart of the report plots and their labels on several
    /// levels; `None` when the rows have a single field (the cells of the
    /// labels then say everything).
    pub fn chart_points(&self, locale: &Locale) -> Option<PivotChartPoints> {
        let depth = self.row_fields.iter().filter(|a| a.field().is_some()).count();
        if depth < 2 || self.data_fields.is_empty() {
            return None;
        }
        let lines = self.report.lines(true, self);
        let first = lines.iter().position(|line| line.kind == ReportLineKind::Item)?;
        let mut points = PivotChartPoints {
            outer_levels: vec![Vec::new(); depth - 1],
            ..Default::default()
        };
        let mut before: Option<&[(usize, u32)]> = None;
        for (k, line) in lines.iter().enumerate().skip(first) {
            if line.kind != ReportLineKind::Item || line.items.is_empty() {
                continue;
            }
            points.keep.push(k - first);
            let text = |at: usize| self.item_text(line.items[at].0, line.items[at].1, locale);
            let last = line.items.len() - 1;
            points.labels.push(text(last));
            // The outer items, where they differ from those of the row
            // before; an item with its detail hidden has no outer label
            // of its own level.
            for (level, texts) in points.outer_levels.iter_mut().enumerate() {
                // Level 0 is next to the innermost one.
                let at = (depth - 2).checked_sub(level).filter(|at| *at < last);
                let same = |at: usize| before.is_some_and(|b| b.len() > at && b[..=at] == line.items[..=at]);
                texts.push(match at {
                    Some(at) if !same(at) => text(at),
                    _ => String::new(),
                });
            }
            before = Some(&line.items);
        }
        Some(points)
    }

    /// The series a chart of the report plots, as ranges of `sheet`.
    pub fn chart_series(&self, sheet: &str, locale: &Locale) -> Vec<Series> {
        let (Some(body), report) = (self.location, &self.report) else {
            return Vec::new();
        };
        if self.data_fields.is_empty() {
            return Vec::new();
        }
        let (first_row, first_col) = (report.first_data_row(), report.first_data_col());
        let plotted = |rows: bool| -> Vec<u32> {
            let lines = report.lines(rows, self);
            let items: Vec<u32> = (0..lines.len() as u32).filter(|k| lines[*k as usize].kind == ReportLineKind::Item).collect();
            // A report without fields on the axis has one line of values.
            if items.is_empty() && lines.len() == 1 { vec![0] } else { items }
        };
        let (rows, cols) = (plotted(true), plotted(false));
        let (Some(r0), Some(r1)) = (rows.first(), rows.last()) else {
            return Vec::new();
        };
        // The rows from the first item to the last: with several row
        // fields the lines between them come along.
        let (top, bottom) = (body.r0 + first_row + r0, body.r0 + first_row + r1);
        let labels = (first_col > 0).then(|| Rect::new(top, body.c0, bottom, body.c0 + first_col - 1));
        let single = self.col_fields.iter().all(|a| a.field().is_none()) && self.data_fields.len() == 1;
        cols.into_iter()
            .filter_map(|c| {
                let col = body.c0 + first_col + c;
                let mut series = Series::new(range(sheet, Rect::new(top, col, bottom, col))?);
                series.categories = labels.and_then(|rect| range(sheet, rect));
                series.name = if single {
                    // "Total", as Excel names the only series (seen by
                    // opening the same PivotChart in Excel).
                    SeriesName::Text(super::text::caption(super::text::Caption::Total, locale).to_string())
                } else {
                    match (first_row > 0).then(|| range(sheet, Rect::cell(body.r0 + first_row - 1, col))).flatten() {
                        Some(cell) => SeriesName::Cell(cell),
                        None => SeriesName::None,
                    }
                };
                Some(series)
            })
            .collect()
    }
}

impl Workbook {
    /// The points the chart plots when it is a PivotChart whose rows have
    /// several fields.
    pub fn pivot_chart_points(&self, chart: u32) -> Option<PivotChartPoints> {
        let (_, p, _) = self.pivot_of_chart(chart)?;
        p.chart_points(self.locale)
    }

    /// The PivotTable a chart is bound to, with its link.
    pub fn pivot_of_chart(&self, chart: u32) -> Option<(usize, &PivotTable, PivotChartLink)> {
        self.pivots().find_map(|(si, p)| {
            let link = p.charts.iter().find(|link| link.chart == chart)?;
            Some((si, p, *link))
        })
    }

    /// Put a chart of PivotTable `pivot` on `sheet`; returns its id. The
    /// caller places it (`update_chart`).
    pub fn add_pivot_chart(&mut self, sheet: usize, pivot: u32, kind: ChartKind) -> Result<u32, PivotError> {
        if !chart_kind_allowed(kind) {
            return Err(PivotError::ChartKind);
        }
        let (si, p) = self.pivot(pivot).ok_or(PivotError::NoSuchPivot)?;
        let mut chart = Chart::new(kind);
        chart.series = p.chart_series(&self.sheets[si].name, self.locale);
        // Several series need telling apart.
        if chart.series.len() > 1 {
            chart.legend = LegendPos::Right;
        }
        let id = self.add_chart(sheet, chart).ok_or(PivotError::NoSuchPivot)?;
        if let Some(p) = self.sheets[si].pivots.iter_mut().find(|p| p.id == pivot) {
            p.charts.push(PivotChartLink::new(id));
        }
        Ok(id)
    }

    /// Show or hide every field button of a PivotChart.
    pub fn set_pivot_chart_buttons(&mut self, chart: u32, shown: bool) -> bool {
        self.edit_pivot_chart_link(chart, |link| link.shown = [shown; 4])
    }

    /// Show or hide one kind of field buttons of a PivotChart.
    pub fn set_pivot_chart_button_kind(&mut self, chart: u32, kind: ChartButtonKind, shown: bool) -> bool {
        self.edit_pivot_chart_link(chart, |link| link.shown[kind as usize] = shown)
    }

    fn edit_pivot_chart_link(&mut self, chart: u32, change: impl Fn(&mut PivotChartLink)) -> bool {
        let Some((si, p, _)) = self.pivot_of_chart(chart) else {
            return false;
        };
        let pivot = p.id;
        self.snapshot();
        for p in self.sheets[si].pivots.iter_mut().filter(|p| p.id == pivot) {
            for link in p.charts.iter_mut().filter(|link| link.chart == chart) {
                change(link);
            }
        }
        true
    }

    /// Make the series of the charts bound to these PivotTables again
    /// from their reports, and forget the charts that are gone. Called
    /// when reports were written; the caller has taken the undo snapshot.
    pub(super) fn sync_pivot_charts(&mut self, pivots: &[u32]) {
        for id in pivots {
            let Some((si, p)) = self.pivot(*id) else {
                continue;
            };
            if p.charts.is_empty() {
                continue;
            }
            let series = p.chart_series(&self.sheets[si].name, self.locale);
            let links: Vec<u32> = p.charts.iter().map(|link| link.chart).collect();
            let mut alive = Vec::new();
            for sheet in &mut self.sheets {
                for chart in sheet.charts.iter_mut().filter(|chart| links.contains(&chart.id)) {
                    alive.push(chart.id);
                    let old = std::mem::take(&mut chart.series);
                    chart.series = series
                        .iter()
                        .enumerate()
                        .map(|(k, new)| match old.get(k) {
                            // The series at the same place keeps its look.
                            Some(was) => Series {
                                name: new.name.clone(),
                                values: new.values.clone(),
                                categories: new.categories.clone(),
                                ..was.clone()
                            },
                            None => new.clone(),
                        })
                        .collect();
                }
            }
            if let Some(p) = self.sheets[si].pivots.iter_mut().find(|p| p.id == *id) {
                p.charts.retain(|link| alive.contains(&link.chart));
            }
        }
    }
}
