//! Choosing a chart type: the buttons of the Insert tab with their
//! galleries, and the dialog that is "Insert Chart" for a selection of
//! cells and "Change Chart Type" for a chart, with its Combo page.

use super::elements::series_name;
use super::gallery::{Gallery, Item, Picture, Section};
use super::{Cmd, open_ribbon_view};
use crate::app::ExcelApp;
use crate::chart_view;
use crate::features::{Command, Cx};
use crate::i18n::t;
use crate::ribbon::{chart_kind_name, row, rows};
use crate::shell::Modal;
use crate::theme::{self, c};
use crate::widgets::{self, Checkbox, ListBox, ScrollArea, Select, icon, tooltip};
use gpui::{AnyElement, Context, FontWeight, Render, SharedString, Window, div, prelude::*, px};
use opensheet_engine::Locale;
use opensheet_engine::ast::RefExpr;
use opensheet_engine::chart::{
    Chart, ChartData, ChartKind, ChartTitle, Family, Fill, LegendPos, Series, SeriesData, TickLabels, palette_color,
};
use std::rc::Rc;

/// A family of chart types: a button of the Insert tab, a line of the
/// list of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Column,
    Line,
    Pie,
    Bar,
    Area,
    Scatter,
    Radar,
    Combo,
}

impl Page {
    /// In the order of the "All Charts" list.
    pub const ALL: [Page; 8] = [Page::Column, Page::Line, Page::Pie, Page::Bar, Page::Area, Page::Scatter, Page::Radar, Page::Combo];

    fn name(self) -> &'static str {
        match self {
            Page::Column => t("Column", "Histogramme"),
            Page::Line => crate::i18n::t_ctx("chart", "Line", "Courbes"),
            Page::Pie => t("Pie", "Secteurs"),
            Page::Bar => t("Bar", "Barres"),
            Page::Area => t("Area", "Aires"),
            Page::Scatter => t("X Y (Scatter)", "Nuage de points (XY)"),
            Page::Radar => t("Radar", "Radar"),
            Page::Combo => t("Combo", "Graphique combiné"),
        }
    }

    /// The types of the page, in the order of Excel's dialog.
    pub fn kinds(self) -> &'static [ChartKind] {
        use ChartKind::*;
        match self {
            Page::Column => &[Column, StackedColumn, PercentStackedColumn, Column3D, StackedColumn3D, PercentStackedColumn3D],
            Page::Line => &[
                Line,
                StackedLine,
                PercentStackedLine,
                LineMarkers,
                StackedLineMarkers,
                PercentStackedLineMarkers,
                Line3D,
            ],
            Page::Pie => &[Pie, Pie3D, Doughnut],
            Page::Bar => &[Bar, StackedBar, PercentStackedBar, Bar3D, StackedBar3D, PercentStackedBar3D],
            Page::Area => &[Area, StackedArea, PercentStackedArea, Area3D, StackedArea3D, PercentStackedArea3D],
            Page::Scatter => &[Scatter, ScatterSmoothMarkers, ScatterSmooth, ScatterLinesMarkers, ScatterLines, Bubble],
            Page::Radar => &[Radar, RadarMarkers, RadarFilled],
            Page::Combo => &[],
        }
    }

    pub fn of(kind: ChartKind) -> Page {
        match kind.family() {
            Family::Column => Page::Column,
            Family::Bar => Page::Bar,
            Family::Line => Page::Line,
            Family::Area => Page::Area,
            Family::Pie | Family::Doughnut => Page::Pie,
            Family::Scatter | Family::Bubble => Page::Scatter,
            Family::Radar => Page::Radar,
        }
    }
}

// ------------------------------------------------------------------- samples

/// A bare picture of a chart type: a few made-up numbers, no axes, no
/// title, no legend. What the galleries show.
pub(crate) fn sample(kind: ChartKind) -> (Chart, ChartData) {
    let mut chart = Chart::new(kind);
    chart.title = ChartTitle::None;
    chart.legend = LegendPos::None;
    chart.chart_area.fill = Fill::None;
    chart.chart_area.border.shown = Some(false);
    for axis in [&mut chart.axes.category, &mut chart.axes.value] {
        axis.labels = TickLabels::None;
        axis.major_gridlines = false;
    }
    let rows: &[&[f64]] = match kind.family() {
        Family::Column | Family::Bar => &[&[4.0, 2.5, 3.5], &[2.5, 4.5, 2.0], &[2.0, 2.0, 3.0]],
        Family::Line => &[&[2.0, 3.2, 2.6, 4.4], &[3.6, 2.2, 3.4, 2.8]],
        Family::Area => &[&[3.0, 4.5, 3.2, 4.0], &[1.5, 2.0, 2.8, 1.8]],
        Family::Pie | Family::Doughnut => &[&[45.0, 25.0, 18.0, 12.0]],
        Family::Scatter => &[&[1.2, 2.8, 2.2, 3.6, 3.0], &[2.6, 1.4, 3.4, 2.0, 4.2]],
        Family::Bubble => &[&[2.0, 3.5, 1.5, 3.0]],
        Family::Radar => &[&[4.0, 3.0, 4.5, 2.5, 3.5], &[2.0, 4.0, 2.5, 4.0, 2.0]],
    };
    let points = rows[0].len();
    let mut data = ChartData {
        categories: (1..=points).map(|k| k.to_string()).collect(),
        x_values: (1..=points).map(|k| Some(k as f64)).collect(),
        value_format: "General".into(),
        x_format: "General".into(),
        ..Default::default()
    };
    for (i, values) in rows.iter().enumerate() {
        chart.series.push(Series::new(RefExpr::cell(0, i as u32)));
        data.series.push(SeriesData {
            name: String::new(),
            values: values.iter().map(|v| Some(*v)).collect(),
            color: palette_color(i),
            x_values: (1..=points).map(|k| Some(k as f64)).collect(),
            sizes: if kind.family() == Family::Bubble { vec![Some(3.0), Some(8.0), Some(5.0), Some(2.0)] } else { Vec::new() },
            format: "General".into(),
        });
    }
    (chart, data)
}

// -------------------------------------------------------------------- combos

/// The chart types the series of a combination chart can take.
const COMBO_KINDS: [ChartKind; 12] = [
    ChartKind::Column,
    ChartKind::StackedColumn,
    ChartKind::PercentStackedColumn,
    ChartKind::Line,
    ChartKind::StackedLine,
    ChartKind::PercentStackedLine,
    ChartKind::LineMarkers,
    ChartKind::StackedLineMarkers,
    ChartKind::PercentStackedLineMarkers,
    ChartKind::Area,
    ChartKind::StackedArea,
    ChartKind::PercentStackedArea,
];

/// Excel's ready-made combinations; the last entry of the page is the
/// custom one.
const PRESETS: usize = 3;

fn preset_name(preset: usize) -> &'static str {
    match preset {
        0 => t("Clustered Column - Line", "Histogramme groupé - Courbe"),
        1 => t(
            "Clustered Column - Line on Secondary Axis",
            "Histogramme groupé - Courbe sur l'axe secondaire",
        ),
        2 => t("Stacked Area - Clustered Column", "Aires empilées - Histogramme groupé"),
        _ => t("Custom Combination", "Combinaison personnalisée"),
    }
}

/// What a preset makes of series `index` of `count`: its type and whether
/// it goes on the secondary axis. The first half of the series takes the
/// first type of the pair.
pub fn preset_series(preset: usize, index: usize, count: usize) -> (ChartKind, bool) {
    let first = index < count.div_ceil(2);
    match (preset, first) {
        (0, true) | (1, true) => (ChartKind::Column, false),
        (0, false) => (ChartKind::Line, false),
        (1, false) => (ChartKind::Line, true),
        (2, true) => (ChartKind::StackedArea, false),
        _ => (ChartKind::Column, false),
    }
}

/// Make a chart a combination: columns as its own type, every series as
/// `rows` says.
pub fn apply_combo(chart: &mut Chart, rows: &[(ChartKind, bool)]) {
    chart.set_kind(ChartKind::Column);
    for (series, (kind, secondary)) in chart.series.iter_mut().zip(rows) {
        series.kind = (*kind != ChartKind::Column).then_some(*kind);
        series.secondary = *secondary;
    }
}

// ------------------------------------------------------------- the Insert tab

/// The controls of the "Charts" group of the Insert tab.
pub fn insert_group(app: &ExcelApp, cx: &mut Cx) -> AnyElement {
    let drop = |id: &'static str, icon_name: &'static str, tip: &'static str, page: Page, cx: &mut Cx| {
        app.small(
            id,
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .px(px(1.))
                .child(icon(icon_name, 16.0, theme::TEXT))
                .child(icon("chevron-down", 8.0, theme::TEXT_MUTED)),
            tip,
            false,
            cx,
            move |this, window, cx| open_gallery(this, id, page, window, cx),
        )
        .into_any_element()
    };
    row(vec![
        app.cmd_large("in-chart-rec", Cmd::Recommended, false, cx),
        rows(vec![
            row(vec![
                drop(
                    "in-chart-col",
                    "chart-bar",
                    t("Insert Column or Bar Chart", "Insérer un histogramme ou un graphique à barres"),
                    Page::Column,
                    cx,
                ),
                drop(
                    "in-chart-radar",
                    "charts-radar",
                    t(
                        "Insert Waterfall, Funnel, Stock, Surface, or Radar Chart",
                        "Insérer un graphique en cascade, en entonnoir, boursier, en surface ou en radar",
                    ),
                    Page::Radar,
                    cx,
                ),
            ]),
            row(vec![
                drop(
                    "in-chart-line",
                    "chart-line",
                    t("Insert Line or Area Chart", "Insérer un graphique en courbes ou en aires"),
                    Page::Line,
                    cx,
                ),
                drop("in-chart-combo", "charts-combo", t("Insert Combo Chart", "Insérer un graphique combiné"), Page::Combo, cx),
            ]),
            row(vec![
                drop(
                    "in-chart-pie",
                    "chart-pie",
                    t("Insert Pie or Doughnut Chart", "Insérer un graphique en secteurs ou en anneau"),
                    Page::Pie,
                    cx,
                ),
                drop(
                    "in-chart-scatter",
                    "chart-scatter",
                    t(
                        "Insert Scatter (X, Y) or Bubble Chart",
                        "Insérer un graphique en nuage de points (X, Y) ou en bulles",
                    ),
                    Page::Scatter,
                    cx,
                ),
            ]),
        ])
        .into_any_element(),
    ])
}

/// The gallery of a button of the Insert tab: Excel's headings, a picture
/// per type, and the way into the dialog.
pub(super) fn open_gallery(app: &mut ExcelApp, anchor: &'static str, page: Page, window: &mut Window, cx: &mut Cx) {
    use ChartKind::*;
    let kinds = |title: &'static str, kinds: &[ChartKind]| Section {
        title: title.into(),
        items: kinds
            .iter()
            .map(|&kind| {
                let (chart, data) = sample(kind);
                Item::new(Picture::Chart(Rc::new(chart), Rc::new(data)), chart_kind_name(kind), move |a, window, cx| {
                    a.insert_chart(kind, window, cx)
                })
            })
            .collect(),
    };
    let (sections, more) = match page {
        Page::Column | Page::Bar => (
            vec![
                kinds(t("2-D Column", "Histogramme 2D"), &[Column, StackedColumn, PercentStackedColumn]),
                kinds(t("3-D Column", "Histogramme 3D"), &[Column3D, StackedColumn3D, PercentStackedColumn3D]),
                kinds(t("2-D Bar", "Barres 2D"), &[Bar, StackedBar, PercentStackedBar]),
                kinds(t("3-D Bar", "Barres 3D"), &[Bar3D, StackedBar3D, PercentStackedBar3D]),
            ],
            t("More Column Charts…", "Autres histogrammes…"),
        ),
        Page::Line | Page::Area => (
            vec![
                kinds(
                    t("2-D Line", "Courbe 2D"),
                    &[Line, StackedLine, PercentStackedLine, LineMarkers, StackedLineMarkers, PercentStackedLineMarkers],
                ),
                kinds(t("3-D Line", "Courbe 3D"), &[Line3D]),
                kinds(t("2-D Area", "Aires 2D"), &[Area, StackedArea, PercentStackedArea]),
                kinds(t("3-D Area", "Aires 3D"), &[Area3D, StackedArea3D, PercentStackedArea3D]),
            ],
            t("More Line Charts…", "Autres graphiques en courbes…"),
        ),
        Page::Pie => (
            vec![
                kinds(t("2-D Pie", "Secteurs 2D"), &[Pie]),
                kinds(t("3-D Pie", "Secteurs 3D"), &[Pie3D]),
                kinds(t("Doughnut", "Anneau"), &[Doughnut]),
            ],
            t("More Pie Charts…", "Autres graphiques en secteurs…"),
        ),
        Page::Scatter => (
            vec![
                kinds(
                    t("Scatter", "Nuages de points"),
                    &[Scatter, ScatterSmoothMarkers, ScatterSmooth, ScatterLinesMarkers, ScatterLines],
                ),
                kinds(t("Bubble", "Bulles"), &[Bubble]),
            ],
            t("More Scatter Charts…", "Autres nuages de points…"),
        ),
        Page::Radar => (
            vec![kinds(t("Radar", "Radar"), &[Radar, RadarMarkers, RadarFilled])],
            t("More Radar Charts…", "Autres graphiques en radar…"),
        ),
        Page::Combo => (
            vec![Section {
                title: t("Combo", "Combiné").into(),
                items: (0..PRESETS)
                    .map(|preset| {
                        let (chart, data) = combo_sample(preset);
                        Item::new(Picture::Chart(Rc::new(chart), Rc::new(data)), preset_name(preset), move |a, window, cx| {
                            let mut chart = a.wb.chart_from_range(a.sheet, a.sel().current(), Column);
                            let count = chart.series.len();
                            let rows: Vec<(ChartKind, bool)> = (0..count).map(|i| preset_series(preset, i, count)).collect();
                            apply_combo(&mut chart, &rows);
                            a.insert_built_chart(chart, window, cx);
                        })
                    })
                    .collect(),
            }],
            t("Create Custom Combo Chart…", "Créer un graphique combiné personnalisé…"),
        ),
    };
    let gallery = Gallery::new(app, cx, sections).footer(more, move |a, window, cx| open_insert_dialog(a, Some(page), window, cx));
    let view = cx.new(|_| gallery);
    open_ribbon_view(app, Some(anchor), view, window, cx);
}

/// The picture of a preset of the Combo page.
fn combo_sample(preset: usize) -> (Chart, ChartData) {
    let (mut chart, data) = sample(ChartKind::Column);
    chart.series.truncate(2);
    let rows: Vec<(ChartKind, bool)> = (0..2).map(|i| preset_series(preset, i, 2)).collect();
    apply_combo(&mut chart, &rows);
    // The secondary scale would only crowd a picture this small.
    chart.axes.secondary_value.visible = false;
    (chart, data)
}

// ---------------------------------------------------------------- the dialog

/// What is chosen in the dialog.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    page: Page,
    kind: ChartKind,
    /// Type and secondary axis of every series (the Combo page).
    rows: Vec<(ChartKind, bool)>,
}

/// The chart the dialog would make: built from the selected cells, or the
/// selected chart given its new type.
fn build(app: &ExcelApp, change: bool, choice: &Choice) -> Option<Chart> {
    let combo = choice.page == Page::Combo;
    let mut chart = if change {
        app.selected_chart()?.clone()
    } else {
        let kind = if combo { ChartKind::Column } else { choice.kind };
        app.wb.chart_from_range(app.sheet, app.sel().current(), kind)
    };
    if combo {
        apply_combo(&mut chart, &choice.rows);
    } else if change {
        chart.set_kind(choice.kind);
        // A pie shows one series; nothing is lost by keeping the others.
    }
    Some(chart)
}

struct ChartTypeDialog {
    modal: Modal,
    /// "Change Chart Type" of the selected chart, not "Insert Chart".
    change: bool,
    choice: Choice,
    /// Names of the series, for the Combo page.
    names: Vec<SharedString>,
    preview: Option<(Rc<Chart>, Rc<ChartData>)>,
    font: SharedString,
    locale: &'static Locale,
}

/// The rows of the Combo page for a chart as it stands.
fn combo_rows(chart: &Chart) -> Vec<(ChartKind, bool)> {
    let mixed = chart.series.iter().any(|s| s.kind.is_some() || s.secondary);
    let count = chart.series.len();
    chart
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let kind = chart.series_kind(s);
            if mixed && COMBO_KINDS.contains(&kind) { (kind, s.secondary) } else { preset_series(0, i, count) }
        })
        .collect()
}

fn open_dialog(app: &mut ExcelApp, change: bool, page: Option<Page>, window: &mut Window, cx: &mut Cx) {
    if app.editing.is_some() && !app.commit_edit(None, window, cx) {
        return;
    }
    let base = if change {
        let Some(chart) = app.selected_chart() else { return };
        chart.clone()
    } else {
        app.wb.chart_from_range(app.sheet, app.sel().current(), ChartKind::Column)
    };
    let mixed = base.series.iter().any(|s| s.kind.is_some());
    let page = page.unwrap_or(if change && mixed { Page::Combo } else { Page::of(base.kind) });
    let kind = match page.kinds().contains(&base.kind) {
        true => base.kind,
        false => page.kinds().first().copied().unwrap_or(ChartKind::Column),
    };
    let choice = Choice {
        page,
        kind,
        rows: combo_rows(&base),
    };
    let data = app.wb.chart_data(app.sheet, &base);
    let names = (0..base.series.len()).map(|i| series_name(&data, i).into()).collect();
    let preview = build(app, change, &choice).map(|chart| {
        let data = app.wb.chart_data(app.sheet, &chart);
        (Rc::new(chart), Rc::new(data))
    });
    let (font, locale) = (app.chart_font(), app.wb.locale);
    app.open_modal(window, cx, move |modal, _, _| ChartTypeDialog {
        modal,
        change,
        choice,
        names,
        preview,
        font,
        locale,
    });
}

/// "Insert Chart" for the selected cells, on a given page of the list.
pub(crate) fn open_insert_dialog(app: &mut ExcelApp, page: Option<Page>, window: &mut Window, cx: &mut Cx) {
    open_dialog(app, false, page, window, cx);
}

/// "Change Chart Type" for the selected chart.
pub(crate) fn open_change_type_dialog(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    open_dialog(app, true, None, window, cx);
}

impl ChartTypeDialog {
    /// Draw the chart the choice would make.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let (change, choice) = (self.change, self.choice.clone());
        self.preview = self
            .modal
            .read_app(cx, |app| {
                build(app, change, &choice).map(|chart| {
                    let data = app.wb.chart_data(app.sheet, &chart);
                    (Rc::new(chart), Rc::new(data))
                })
            })
            .flatten();
        cx.notify();
    }

    fn set_page(&mut self, index: usize, cx: &mut Context<Self>) {
        let page = Page::ALL[index.min(Page::ALL.len() - 1)];
        self.choice.page = page;
        if let Some(first) = page.kinds().first() {
            if !page.kinds().contains(&self.choice.kind) {
                self.choice.kind = *first;
            }
        }
        self.refresh(cx);
    }

    /// The preset the rows of the Combo page amount to (`PRESETS` when
    /// they are a combination of the user's own).
    fn preset(&self) -> usize {
        let count = self.choice.rows.len();
        (0..PRESETS)
            .find(|preset| self.choice.rows.iter().enumerate().all(|(i, row)| *row == preset_series(*preset, i, count)))
            .unwrap_or(PRESETS)
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (change, choice) = (self.change, self.choice.clone());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            let Some(chart) = build(app, change, &choice) else {
                return;
            };
            if change {
                let (kind, series) = (chart.kind, chart.series);
                app.edit_chart(Cmd::ChangeType.label(), cx, move |c| {
                    c.kind = kind;
                    c.series = series;
                });
            } else {
                app.insert_built_chart(chart, window, cx);
            }
        });
    }

    /// One picture of the row of types above the preview.
    fn thumb(&self, id: usize, picture: (Chart, ChartData), tip: &'static str, selected: bool, cx: &mut Context<Self>, pick: impl Fn(&mut Self, &mut Context<Self>) + 'static) -> AnyElement {
        div()
            .id(("chart-type", id))
            .p(px(4.))
            .border_1()
            .rounded(px(3.))
            .border_color(if selected { c(theme::ACCENT) } else { c(theme::BORDER) })
            .when(selected, |d| d.bg(c(theme::ACCENT_LIGHT)))
            .hover(|s| s.border_color(c(theme::BORDER_STRONG)))
            .child(chart_view::chart_canvas(Rc::new(picture.0), Rc::new(picture.1), 46.0, 36.0, 0.4, self.font.clone(), self.locale))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, _, cx| pick(this, cx)))
            .into_any_element()
    }

    fn combo_table(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let kinds: Vec<SharedString> = COMBO_KINDS.iter().map(|k| chart_kind_name(*k).into()).collect();
        let header = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(c(theme::TEXT_MUTED))
            .child(div().w(px(120.)).child(t("Series Name", "Nom de la série")))
            .child(div().w(px(220.)).child(t("Chart Type", "Type de graphique")))
            .child(t("Secondary Axis", "Axe secondaire"));
        let rows: Vec<AnyElement> = self
            .choice
            .rows
            .iter()
            .enumerate()
            .map(|(i, (kind, secondary))| {
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(120.))
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(div().size(px(9.)).flex_none().bg(c(palette_color(i).0)))
                            .child(div().truncate().child(self.names.get(i).cloned().unwrap_or_default())),
                    )
                    .child(
                        Select::new(("combo-kind", i), kinds.clone(), COMBO_KINDS.iter().position(|k| k == kind))
                            .width(220.0)
                            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                                this.choice.rows[i].0 = COMBO_KINDS[*ix];
                                this.refresh(cx);
                            })),
                    )
                    .child(Checkbox::new(("combo-secondary", i), "", *secondary).on_change(cx.listener(
                        move |this, on: &bool, _, cx| {
                            this.choice.rows[i].1 = *on;
                            this.refresh(cx);
                        },
                    )))
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(widgets::label(t(
                "Choose the chart type and axis for your data series:",
                "Choisissez le type de graphique et l'axe pour votre série de données :",
            )))
            .child(header)
            .child(ScrollArea::new("combo-rows", 88.0).children(rows))
            .into_any_element()
    }
}

impl Render for ChartTypeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.choice.page;
        let combo = page == Page::Combo;
        let preset = self.preset();
        let thumbs: Vec<AnyElement> = if combo {
            (0..=PRESETS)
                .map(|p| {
                    self.thumb(p, combo_sample(p.min(PRESETS - 1)), preset_name(p), p == preset, cx, move |this, cx| {
                        if p < PRESETS {
                            let count = this.choice.rows.len();
                            this.choice.rows = (0..count).map(|i| preset_series(p, i, count)).collect();
                            this.refresh(cx);
                        }
                    })
                })
                .collect()
        } else {
            page.kinds()
                .iter()
                .enumerate()
                .map(|(i, &kind)| {
                    self.thumb(i, sample(kind), chart_kind_name(kind), kind == self.choice.kind, cx, move |this, cx| {
                        this.choice.kind = kind;
                        this.refresh(cx);
                    })
                })
                .collect()
        };
        let name = if combo { preset_name(preset) } else { chart_kind_name(self.choice.kind) };
        let preview_h = if combo { 150.0 } else { 270.0 };
        let preview = match &self.preview {
            Some((chart, data)) if !chart.series.is_empty() => {
                chart_view::chart_canvas(chart.clone(), data.clone(), 430.0, preview_h, 0.8, self.font.clone(), self.locale)
                    .into_any_element()
            }
            _ => div()
                .w(px(430.))
                .h(px(preview_h))
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(theme::TEXT_MUTED))
                .child(t(
                    "Select data for your chart and try again.",
                    "Sélectionnez les données de votre graphique et réessayez.",
                ))
                .into_any_element(),
        };
        let pages: Vec<SharedString> = Page::ALL.iter().map(|p| p.name().into()).collect();
        let table = combo.then(|| self.combo_table(cx));
        let title = if self.change {
            t("Change Chart Type", "Modifier le type de graphique")
        } else {
            t("Insert Chart", "Insérer un graphique")
        };
        widgets::dialog(&self.modal, title)
            .width(640.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::ACCENT_DARK))
                    .child(t("All Charts", "Tous les graphiques")),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        ListBox::new("pages", pages, Page::ALL.iter().position(|p| *p == page))
                            .width(158.0)
                            .height(380.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| this.set_page(*ix, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().flex().gap(px(4.)).children(thumbs))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(name))
                            .child(div().border_1().border_color(c(theme::BORDER)).child(preview))
                            .children(table),
                    ),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_on_a_page() {
        for kind in ChartKind::ALL {
            assert!(Page::of(kind).kinds().contains(&kind), "{kind:?}");
        }
        let listed: usize = Page::ALL.iter().map(|p| p.kinds().len()).sum();
        assert_eq!(listed, ChartKind::ALL.len());
    }

    #[test]
    fn presets_split_the_series() {
        // Three series: two columns and a line.
        let kinds: Vec<ChartKind> = (0..3).map(|i| preset_series(0, i, 3).0).collect();
        assert_eq!(kinds, [ChartKind::Column, ChartKind::Column, ChartKind::Line]);
        assert_eq!(preset_series(1, 1, 2), (ChartKind::Line, true));
        assert_eq!(preset_series(2, 0, 2), (ChartKind::StackedArea, false));
        let (mut chart, _) = sample(ChartKind::Pie);
        chart.series.push(Series::new(RefExpr::cell(0, 1)));
        apply_combo(&mut chart, &[(ChartKind::Column, false), (ChartKind::LineMarkers, true)]);
        assert_eq!(chart.kind, ChartKind::Column);
        assert_eq!(chart.series[0].kind, None);
        assert_eq!((chart.series[1].kind, chart.series[1].secondary), (Some(ChartKind::LineMarkers), true));
        assert_eq!(combo_rows(&chart), [(ChartKind::Column, false), (ChartKind::LineMarkers, true)]);
        // Every type of the Combo page can stand next to columns.
        assert!(COMBO_KINDS.iter().all(|k| ChartKind::Column.combines_with(*k)));
    }

    #[test]
    fn samples_have_something_to_draw() {
        for kind in ChartKind::ALL {
            let (chart, data) = sample(kind);
            assert_eq!(chart.series.len(), data.series.len());
            assert!(!data.series.is_empty() && data.series.iter().all(|s| !s.values.is_empty()));
        }
    }
}
