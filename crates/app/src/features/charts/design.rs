//! The "Chart Design" tab: Add Chart Element and its submenus, Quick
//! Layout, Change Colors, Chart Styles, and the two lists opened by the
//! buttons that float beside a selected chart.

use super::elements::{add_data_labels, add_trendline, axis_of, has_axis, series_name, takes_trendlines, target_series};
use super::gallery::{Gallery, Item, Picture, Section};
use super::{Cmd, insert};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Cx, chart_format};
use crate::i18n::t;
use crate::ribbon::{group, row};
use crate::shell::Modal;
use crate::theme::{self, c};
use crate::widgets::{self, Checkbox, ListBox};
use gpui::{AnyElement, Context, Entity, FontWeight, Render, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::chart::{
    Axes, Chart, ChartTitle, DataLabels, DataTable, ErrorAmount, ErrorBars, ErrorDirection, Family, Fill, LabelPosition,
    LegendPos, Line, PALETTE, ShapeFormat, TrendKind, Trendline,
};
use opensheet_engine::chart_layout::{AxisId, ChartElement};
use opensheet_engine::style::Color;
use std::rc::Rc;

/// The groups of the "Chart Design" tab.
pub(super) fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Chart Layouts", "Dispositions du graphique"),
            row(vec![
                app.cmd_large("ch-elements", Cmd::AddElement, true, cx),
                app.cmd_large("ch-quick-layout", chart_format::Cmd::QuickLayout, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Chart Styles", "Styles du graphique"),
            row(vec![
                app.cmd_large("ch-colors", chart_format::Cmd::ChangeColors, true, cx),
                app.cmd_large("ch-styles", chart_format::Cmd::Styles, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Data", "Données"),
            row(vec![
                app.cmd_large("ch-switch", Cmd::SwitchRowColumn, false, cx),
                app.cmd_large("ch-select-data", chart_format::Cmd::SelectData, false, cx),
            ]),
        )
        .into_any_element(),
        group(t("Type", "Type"), row(vec![app.cmd_large("ch-type", Cmd::ChangeType, false, cx)])).into_any_element(),
        group(
            t("Location", "Emplacement"),
            row(vec![app.cmd_large("ch-move", chart_format::Cmd::MoveChart, false, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

// --------------------------------------------------------- Add Chart Element

/// A menu entry that changes the selected chart.
fn edit(label: impl Into<SharedString>, undo: &'static str, f: impl Fn(&mut Chart) + 'static) -> MenuEntry {
    MenuEntry::new(label, move |a, _, cx| a.edit_chart(undo, cx, |c| f(c)))
}

/// "More … Options…": select an element and open its pane.
fn more(label: &'static str, element: ChartElement) -> MenuEntry {
    MenuEntry::new(label, move |a, window, cx| {
        let present = a.selected_chart().is_some_and(|c| super::elements::exists(c, element));
        a.select_chart_element(if present { element } else { ChartElement::ChartArea }, cx);
        chart_format::open_pane(a, window, cx);
    })
}

/// The label positions Excel offers for a kind of chart, with their names.
pub(crate) fn label_positions(chart: &Chart) -> Vec<(LabelPosition, &'static str)> {
    let center = (LabelPosition::Center, crate::i18n::t_ctx("label", "Center", "Centre"));
    let inside_end = (LabelPosition::InsideEnd, t("Inside End", "Bord intérieur"));
    let inside_base = (LabelPosition::InsideBase, t("Inside Base", "Intérieur base"));
    let outside_end = (LabelPosition::OutsideEnd, t("Outside End", "Bord extérieur"));
    match chart.kind.family() {
        Family::Column | Family::Bar if chart.kind.is_stacked() || chart.kind.is_3d() => vec![center, inside_end, inside_base],
        Family::Column | Family::Bar => vec![center, inside_end, inside_base, outside_end],
        Family::Line | Family::Scatter | Family::Bubble | Family::Radar => vec![
            center,
            (LabelPosition::Left, t("Left", "Gauche")),
            (LabelPosition::Right, t("Right", "Droite")),
            (LabelPosition::Above, t("Above", "Au-dessus")),
            (LabelPosition::Below, t("Below", "En dessous")),
        ],
        Family::Pie => vec![center, inside_end, outside_end, (LabelPosition::BestFit, t("Best Fit", "Ajuster"))],
        Family::Area | Family::Doughnut => Vec::new(),
    }
}

/// Error bars of one kind on a series (`None`: on every series).
fn set_error_bars(chart: &mut Chart, series: Option<usize>, amount: Option<ErrorAmount>) {
    let xy = chart.kind.is_xy();
    for (i, s) in chart.series.iter_mut().enumerate() {
        if series.is_some_and(|only| only != i) {
            continue;
        }
        s.error_bars.clear();
        if let Some(amount) = amount {
            let bars = |horizontal: bool| ErrorBars {
                horizontal,
                direction: ErrorDirection::Both,
                amount,
                end_cap: true,
            };
            // Scatter and bubble charts get them along both axes.
            if xy {
                s.error_bars.push(bars(true));
            }
            s.error_bars.push(bars(false));
        }
    }
}

/// Add a trendline to the series the selection points at, asking which
/// one when it does not.
fn add_trend(app: &mut ExcelApp, trend: Trendline, window: &mut Window, cx: &mut Cx) {
    let Some(chart) = app.selected_chart() else {
        return;
    };
    match target_series(chart, app.chart_element()) {
        Some(series) => add_trend_to(app, series, trend, cx),
        None => {
            let data = app.wb.chart_data(app.sheet, chart);
            let names: Vec<SharedString> = (0..chart.series.len()).map(|i| series_name(&data, i).into()).collect();
            app.open_modal(window, cx, move |modal, _, _| PickSeries {
                modal,
                names,
                selected: 0,
                trend,
            });
        }
    }
}

fn add_trend_to(app: &mut ExcelApp, series: usize, trend: Trendline, cx: &mut Cx) {
    add_trendline(app, series, trend.kind, cx);
    if trend.forward != 0.0 {
        app.edit_chart(t("Add Trendline", "Ajouter une courbe de tendance"), cx, move |c| {
            if let Some(last) = c.series.get_mut(series).and_then(|s| s.trendlines.last_mut()) {
                last.forward = trend.forward;
            }
        });
    }
}

/// "Add a Trendline based on Series:".
struct PickSeries {
    modal: Modal,
    names: Vec<SharedString>,
    selected: usize,
    trend: Trendline,
}

impl PickSeries {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (series, trend) = (self.selected, self.trend.clone());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| add_trend_to(app, series, trend, cx));
    }
}

impl Render for PickSeries {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::dialog(&self.modal, t("Add Trendline", "Ajouter une courbe de tendance"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(widgets::label(t(
                "Add a Trendline based on Series:",
                "Ajouter une courbe de tendance en fonction des séries :",
            )))
            .child(
                ListBox::new("series", self.names.clone(), Some(self.selected))
                    .height(140.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.selected = *ix;
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, ix: &usize, window, cx| {
                        this.selected = *ix;
                        this.ok(&(), window, cx);
                    })),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// The menu of "Add Chart Element", every submenu reflecting the chart.
pub(super) fn add_element_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let Some(chart) = app.selected_chart() else {
        return Vec::new();
    };
    let element = app.chart_element();
    let has_axes = !chart.kind.is_round();
    let cartesian = has_axes && chart.kind.family() != Family::Radar;
    let secondary = chart.has_secondary_axis();
    // The four axes by where they stand, as the submenus name them.
    let mut places: Vec<(AxisId, &'static str)> = vec![
        (axis_of(chart, true, false), t("Primary Horizontal", "Horizontal principal")),
        (axis_of(chart, false, false), t("Primary Vertical", "Vertical principal")),
    ];
    if secondary {
        places.push((axis_of(chart, true, true), t("Secondary Horizontal", "Horizontal secondaire")));
        places.push((axis_of(chart, false, true), t("Secondary Vertical", "Vertical secondaire")));
    }

    let mut axes: Vec<MenuEntry> = places
        .iter()
        .map(|&(axis, label)| {
            let on = axis.of(&chart.axes).visible;
            edit(label, t("Axes", "Axes"), move |c| axis.of_mut(&mut c.axes).visible = !on).checked(on)
        })
        .collect();
    axes.push(MenuEntry::separator());
    axes.push(more(t("More Axis Options…", "Autres options de l'axe…"), ChartElement::Axis(places[0].0)));

    let mut titles: Vec<MenuEntry> = places
        .iter()
        .map(|&(axis, label)| {
            let on = axis.of(&chart.axes).title.is_some();
            edit(label, t("Axis Titles", "Titres des axes"), move |c| {
                axis.of_mut(&mut c.axes).title = (!on).then(|| t("Axis Title", "Titre de l'axe").to_string())
            })
            .checked(on)
        })
        .collect();
    titles.push(MenuEntry::separator());
    titles.push(more(t("More Axis Title Options…", "Autres options de titre de l'axe…"), ChartElement::AxisTitle(places[0].0)));

    let titled = chart.title != ChartTitle::None;
    let title = vec![
        edit(t("None", "Aucun"), t("Chart Title", "Titre du graphique"), |c| c.title = ChartTitle::None).checked(!titled),
        edit(t("Above Chart", "Au-dessus du graphique"), t("Chart Title", "Titre du graphique"), |c| {
            if c.title == ChartTitle::None {
                c.title = ChartTitle::Auto;
            }
            c.title_overlay = false;
        })
        .checked(titled && !chart.title_overlay),
        edit(t("Centered Overlay", "Superposé centré"), t("Chart Title", "Titre du graphique"), |c| {
            if c.title == ChartTitle::None {
                c.title = ChartTitle::Auto;
            }
            c.title_overlay = true;
        })
        .checked(titled && chart.title_overlay),
        MenuEntry::separator(),
        more(t("More Title Options…", "Autres options de titre…"), ChartElement::Title),
    ];

    // Data labels: of the selected series, else of all of them.
    let series = element.series().filter(|s| *s < chart.series.len());
    let shown: Option<&DataLabels> = match series {
        Some(s) => chart.labels_for(&chart.series[s]),
        None => chart.series.iter().find_map(|s| chart.labels_for(s)),
    };
    let undo_labels = t("Data Labels", "Étiquettes de données");
    let mut labels = vec![
        edit(t("None", "Aucun"), undo_labels, move |c| match series {
            Some(s) => {
                super::elements::delete(c, ChartElement::DataLabels(s as u32));
            }
            None => {
                c.data_labels = false;
                for s in &mut c.series {
                    s.labels = None;
                }
            }
        })
        .checked(shown.is_none()),
    ];
    let positions = label_positions(chart);
    if positions.is_empty() {
        labels.push(edit(t("Show", "Afficher"), undo_labels, move |c| add_data_labels(c, series)).checked(shown.is_some()));
    }
    for (position, name) in positions {
        labels.push(
            edit(name, undo_labels, move |c| {
                add_data_labels(c, series);
                match series {
                    Some(s) => c.series[s].labels.get_or_insert_with(DataLabels::default).position = position,
                    None => c.label_options.position = position,
                }
            })
            .checked(shown.is_some_and(|l| l.position == position)),
        );
    }
    labels.push(MenuEntry::separator());
    let first_labelled = chart.series.iter().position(|s| chart.labels_for(s).is_some()).unwrap_or(0);
    labels.push(more(
        t("More Data Label Options…", "Autres options d'étiquettes de données…"),
        ChartElement::DataLabels(series.unwrap_or(first_labelled) as u32),
    ));

    let undo_table = t("Data Table", "Table de données");
    let table = vec![
        edit(t("None", "Aucun"), undo_table, |c| c.data_table = None).checked(chart.data_table.is_none()),
        edit(t("With Legend Keys", "Avec symboles de légende"), undo_table, |c| {
            c.data_table = Some(DataTable {
                legend_keys: true,
                ..c.data_table.unwrap_or_default()
            })
        })
        .checked(chart.data_table.is_some_and(|d| d.legend_keys)),
        edit(t("No Legend Keys", "Aucun symbole de légende"), undo_table, |c| {
            c.data_table = Some(DataTable {
                legend_keys: false,
                ..c.data_table.unwrap_or_default()
            })
        })
        .checked(chart.data_table.is_some_and(|d| !d.legend_keys)),
        MenuEntry::separator(),
        more(t("More Data Table Options…", "Autres options de table de données…"), ChartElement::DataTable),
    ];

    let undo_bars = t("Error Bars", "Barres d'erreur");
    let barred = match series {
        Some(s) => chart.series[s].error_bars.first(),
        None => chart.series.iter().find_map(|s| s.error_bars.first()),
    };
    let amount = barred.map(|b| b.amount);
    let first_barred = chart.series.iter().position(|s| !s.error_bars.is_empty()).unwrap_or(0);
    let errors = vec![
        edit(t("None", "Aucun"), undo_bars, move |c| set_error_bars(c, series, None)).checked(barred.is_none()),
        edit(t("Standard Error", "Erreur standard"), undo_bars, move |c| set_error_bars(c, series, Some(ErrorAmount::StdErr)))
            .checked(amount == Some(ErrorAmount::StdErr)),
        edit(t("Percentage", "Pourcentage"), undo_bars, move |c| {
            set_error_bars(c, series, Some(ErrorAmount::Percent(5.0)))
        })
        .checked(matches!(amount, Some(ErrorAmount::Percent(_)))),
        edit(t("Standard Deviation", "Écart type"), undo_bars, move |c| {
            set_error_bars(c, series, Some(ErrorAmount::StdDev(1.0)))
        })
        .checked(matches!(amount, Some(ErrorAmount::StdDev(_)))),
        MenuEntry::separator(),
        more(
            t("More Error Bars Options…", "Autres options de barres d'erreur…"),
            ChartElement::ErrorBars(series.unwrap_or(first_barred) as u32, 0),
        ),
    ];

    // Horizontal gridlines are those of the axis that runs up the chart.
    let (across, up) = (axis_of(chart, true, false), axis_of(chart, false, false));
    let undo_grid = t("Gridlines", "Quadrillage");
    let gridline = |axis: AxisId, major: bool, label: &'static str| {
        let on = if major { axis.of(&chart.axes).major_gridlines } else { axis.of(&chart.axes).minor_gridlines };
        edit(label, undo_grid, move |c| {
            let axis = axis.of_mut(&mut c.axes);
            if major { axis.major_gridlines = !on } else { axis.minor_gridlines = !on }
        })
        .checked(on)
    };
    let gridlines = vec![
        gridline(up, true, t("Primary Major Horizontal", "Horizontal majeur principal")),
        gridline(across, true, t("Primary Major Vertical", "Vertical majeur principal")),
        gridline(up, false, t("Primary Minor Horizontal", "Horizontal mineur principal")),
        gridline(across, false, t("Primary Minor Vertical", "Vertical mineur principal")),
        MenuEntry::separator(),
        more(t("More Gridline Options…", "Autres options de quadrillage…"), ChartElement::MajorGridlines(up)),
    ];

    let mut legend: Vec<MenuEntry> = [
        (LegendPos::None, t("None", "Aucun")),
        (LegendPos::Right, t("Right", "Droite")),
        (LegendPos::Top, t("Top", "Haut")),
        (LegendPos::Left, t("Left", "Gauche")),
        (LegendPos::Bottom, t("Bottom", "Bas")),
    ]
    .into_iter()
    .map(|(pos, label)| {
        // A side chosen here is where the legend goes, wherever it had
        // been dragged.
        let place = move |c: &mut Chart| {
            c.legend = pos;
            c.legend_layout = None;
        };
        edit(label, t("Legend", "Légende"), place).checked(chart.legend == pos && chart.legend_layout.is_none())
    })
    .collect();
    legend.push(MenuEntry::separator());
    legend.push(more(t("More Legend Options…", "Autres options de légende…"), ChartElement::Legend));

    let trended = match series {
        Some(s) => !chart.series[s].trendlines.is_empty(),
        None => chart.series.iter().any(|s| !s.trendlines.is_empty()),
    };
    let trend = |label: &'static str, line: Trendline| {
        MenuEntry::new(label, move |a, window, cx| add_trend(a, line.clone(), window, cx))
    };
    let first_trended = chart.series.iter().position(|s| !s.trendlines.is_empty()).unwrap_or(0);
    let trends = vec![
        edit(t("None", "Aucun"), t("Trendline", "Courbe de tendance"), move |c| {
            for (i, s) in c.series.iter_mut().enumerate() {
                if series.is_none_or(|only| only == i) {
                    s.trendlines.clear();
                }
            }
        })
        .checked(!trended),
        trend(t("Linear", "Linéaire"), Trendline::new(TrendKind::Linear)),
        trend(t("Exponential", "Exponentielle"), Trendline::new(TrendKind::Exponential)),
        trend(
            t("Linear Forecast", "Prévision linéaire"),
            Trendline {
                forward: 2.0,
                ..Trendline::new(TrendKind::Linear)
            },
        ),
        trend(t("Moving Average", "Moyenne mobile"), Trendline::new(TrendKind::MovingAverage(2))),
        MenuEntry::separator(),
        more(
            t("More Trendline Options…", "Autres options de la courbe de tendance…"),
            ChartElement::Trendline(series.unwrap_or(first_trended) as u32, 0),
        ),
    ];

    let sub = |label: &'static str, icon: &'static str, entries: Vec<MenuEntry>, available: bool| {
        MenuEntry::submenu(label, entries).icon(icon).disabled(!available)
    };
    let flat = !chart.kind.is_3d();
    vec![
        sub(t("Axes", "Axes"), "charts-el-axes", axes, has_axes),
        sub(t("Axis Titles", "Titres des axes"), "charts-el-axis-titles", titles, cartesian),
        sub(t("Chart Title", "Titre du graphique"), "charts-el-title", title, true),
        sub(t("Data Labels", "Étiquettes de données"), "charts-el-labels", labels, true),
        sub(t("Data Table", "Table de données"), "charts-el-table", table, cartesian && !chart.kind.is_xy() && !chart.kind.is_horizontal() && flat),
        sub(t("Error Bars", "Barres d'erreur"), "charts-el-error", errors, takes_trendlines(chart)),
        sub(t("Gridlines", "Quadrillage"), "charts-el-grid", gridlines, has_axes),
        sub(t("Legend", "Légende"), "charts-el-legend", legend, true),
        // Drop lines, high-low lines and up/down bars are not in the model.
        sub(t("Lines", "Lignes"), "charts-el-lines", Vec::new(), false),
        sub(t("Trendline", "Courbe de tendance"), "charts-el-trend", trends, takes_trendlines(chart)),
        sub(t("Up/Down Bars", "Barres haut/bas"), "charts-el-updown", Vec::new(), false),
    ]
}

// --------------------------------------------------------------- Quick Layout

/// One of Excel's quick layouts, as a combination of chart elements.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuickLayout {
    pub title: bool,
    pub legend: LegendPos,
    /// Titles of the horizontal and of the vertical axis.
    pub axis_titles: (bool, bool),
    pub labels: bool,
    pub table: bool,
    /// Major gridlines across the chart, major gridlines up the chart,
    /// minor gridlines of both.
    pub gridlines: (bool, bool, bool),
    pub value_axis: bool,
    /// Bars touching each other (Excel's layout 8).
    pub no_gap: bool,
}

/// Excel's eleven layouts of a column chart.
pub const QUICK_LAYOUTS: [QuickLayout; 11] = {
    const BASE: QuickLayout = QuickLayout {
        title: true,
        legend: LegendPos::Right,
        axis_titles: (false, false),
        labels: false,
        table: false,
        gridlines: (true, false, false),
        value_axis: true,
        no_gap: false,
    };
    [
        BASE,
        QuickLayout {
            legend: LegendPos::Top,
            labels: true,
            gridlines: (false, false, false),
            value_axis: false,
            ..BASE
        },
        QuickLayout {
            legend: LegendPos::Bottom,
            ..BASE
        },
        QuickLayout {
            title: false,
            legend: LegendPos::Bottom,
            labels: true,
            gridlines: (false, false, false),
            ..BASE
        },
        QuickLayout {
            legend: LegendPos::None,
            axis_titles: (false, true),
            table: true,
            ..BASE
        },
        QuickLayout {
            axis_titles: (false, true),
            labels: true,
            ..BASE
        },
        QuickLayout {
            title: false,
            axis_titles: (true, true),
            gridlines: (true, true, true),
            ..BASE
        },
        QuickLayout {
            legend: LegendPos::None,
            axis_titles: (true, true),
            gridlines: (false, false, false),
            no_gap: true,
            ..BASE
        },
        QuickLayout {
            axis_titles: (true, true),
            ..BASE
        },
        QuickLayout {
            labels: true,
            gridlines: (false, false, false),
            ..BASE
        },
        QuickLayout { title: false, ..BASE },
    ]
};

/// Give a chart the elements of a quick layout.
pub fn apply_quick_layout(chart: &mut Chart, layout: &QuickLayout) {
    // A layout says where everything goes.
    chart.clear_manual_layout();
    let axis_title = || t("Axis Title", "Titre de l'axe").to_string();
    chart.title = match (&chart.title, layout.title) {
        (_, false) => ChartTitle::None,
        (ChartTitle::None, true) => ChartTitle::Auto,
        (kept, true) => kept.clone(),
    };
    chart.title_overlay = false;
    chart.legend = layout.legend;
    chart.legend_overlay = false;
    chart.data_labels = layout.labels;
    for series in &mut chart.series {
        series.labels = None;
    }
    if layout.labels {
        chart.label_options = DataLabels {
            // A pie tells its shares.
            percent: chart.kind.is_round(),
            value: !chart.kind.is_round(),
            ..DataLabels::default()
        };
    }
    let (across, up) = (axis_of(chart, true, false), axis_of(chart, false, false));
    let table_fits = !chart.kind.is_round() && !chart.kind.is_xy() && !chart.kind.is_horizontal() && !chart.kind.is_3d();
    chart.data_table = (layout.table && table_fits).then(DataTable::default);
    for (axis, wanted, major) in [(across, layout.axis_titles.0, layout.gridlines.1), (up, layout.axis_titles.1, layout.gridlines.0)] {
        let axis = axis.of_mut(&mut chart.axes);
        axis.title = match (axis.title.take(), wanted) {
            (_, false) => None,
            (Some(kept), true) => Some(kept),
            (None, true) => Some(axis_title()),
        };
        axis.major_gridlines = major;
        axis.minor_gridlines = layout.gridlines.2;
        axis.visible = true;
    }
    chart.axes.value.visible = layout.value_axis;
    chart.gap_width = layout.no_gap.then_some(0);
    if layout.no_gap {
        chart.overlap = Some(0);
    } else if chart.overlap == Some(0) {
        chart.overlap = None;
    }
}

pub(crate) fn open_quick_layouts(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(chart) = app.selected_chart() else {
        return;
    };
    let (sample, data) = insert::sample(chart.kind);
    let items = QUICK_LAYOUTS
        .iter()
        .enumerate()
        .map(|(i, layout)| {
            let mut picture = sample.clone();
            // The sample is a bare picture of the type: give it axes back.
            picture.axes = Axes::for_kind(picture.kind);
            picture.chart_area = ShapeFormat::default();
            apply_quick_layout(&mut picture, layout);
            let layout = *layout;
            Item::new(
                Picture::Chart(Rc::new(picture), Rc::new(data.clone())),
                super::elements::fill(t("Layout {0}", "Disposition {0}"), &[&(i + 1).to_string()]),
                move |a, _, cx| a.edit_chart(t("Quick Layout", "Disposition rapide"), cx, |c| apply_quick_layout(c, &layout)),
            )
        })
        .collect();
    let gallery = Gallery::new(
        app,
        cx,
        vec![Section {
            title: SharedString::default(),
            items,
        }],
    )
    .thumb(92.0, 62.0, 0.34)
    .columns(3);
    let view = cx.new(|_| gallery);
    let anchor = app.command_anchor;
    super::open_ribbon_view(app, anchor, view, window, cx);
}

// -------------------------------------------------------------- Change Colors

/// Number of colourful and of monochromatic palettes, as in Excel.
pub const COLORFUL: usize = 4;
pub const MONOCHROMATIC: usize = 13;

/// A colour moved towards white (`amount` > 0) or black.
fn shade(color: u32, amount: f32) -> Color {
    Color(theme::tint(color, amount))
}

/// The colours palette `index` gives `count` series: the colourful ones
/// first (0 is the automatic palette), then the monochromatic ones.
pub fn palette(index: usize, count: usize) -> Vec<Color> {
    // Later rounds of a colourful palette are lighter and darker variants.
    const ROUNDS: [f32; 6] = [0.0, 0.4, -0.25, 0.6, -0.5, 0.2];
    let colorful = |picks: &[usize]| -> Vec<Color> {
        (0..count).map(|i| shade(PALETTE[picks[i % picks.len()]], ROUNDS[(i / picks.len()) % ROUNDS.len()])).collect()
    };
    match index {
        0 => colorful(&[0, 1, 2, 3, 4, 5]),
        1 => colorful(&[0, 2, 4]),
        2 => colorful(&[1, 3, 5]),
        3 => colorful(&[5, 4, 3]),
        mono => {
            let k = mono - COLORFUL;
            // Six run from dark to light, the greys follow, six run back.
            let (base, reversed) = match k {
                0..=5 => (PALETTE[k], false),
                6 => (0x7F7F7F, false),
                _ => (PALETTE[(k - 7) % PALETTE.len()], true),
            };
            (0..count)
                .map(|i| {
                    let step = if count > 1 { i as f32 / (count - 1) as f32 } else { 0.4 };
                    let step = if reversed { 1.0 - step } else { step };
                    shade(base, -0.4 + step)
                })
                .collect()
        }
    }
}

pub fn palette_name(index: usize) -> String {
    if index < COLORFUL {
        super::elements::fill(t("Colorful Palette {0}", "Palette de couleurs {0}"), &[&(index + 1).to_string()])
    } else {
        super::elements::fill(t("Monochromatic Palette {0}", "Palette monochrome {0}"), &[&(index + 1 - COLORFUL).to_string()])
    }
}

/// Colour the series of a chart (the points, when each has its own colour)
/// from a palette. `points` is the number of points of the first series.
pub fn apply_palette(chart: &mut Chart, index: usize, points: usize) {
    let by_point = chart.varies_colors() && (chart.kind.is_round() || chart.series.len() == 1);
    let count = chart.series.len();
    for (i, series) in chart.series.iter_mut().enumerate() {
        for point in &mut series.points {
            point.color = None;
        }
        series.points.retain(|p| p.explosion.is_some());
        if index == 0 {
            // The first palette is the automatic one.
            series.color = None;
        } else if by_point {
            for (k, color) in palette(index, points).into_iter().enumerate() {
                series.point_mut(k).color = Some(color);
            }
        } else {
            series.color = Some(palette(index, count)[i]);
        }
    }
}

fn color_sections(app: &ExcelApp) -> Vec<Section> {
    let points = app
        .selected_chart()
        .map(|chart| app.wb.chart_data(app.sheet, chart))
        .and_then(|data| data.series.first().map(|s| s.values.len()))
        .unwrap_or(0);
    let item = |index: usize| {
        Item::new(Picture::Colors(palette(index, 6)), palette_name(index), move |a, _, cx| {
            a.edit_chart(t("Change Colors", "Modifier les couleurs"), cx, |c| apply_palette(c, index, points))
        })
    };
    vec![
        Section {
            title: t("Colorful", "Coloré").into(),
            items: (0..COLORFUL).map(item).collect(),
        },
        Section {
            title: t("Monochromatic", "Monochrome").into(),
            items: (COLORFUL..COLORFUL + MONOCHROMATIC).map(item).collect(),
        },
    ]
}

pub(crate) fn open_colors(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let gallery = Gallery::new(app, cx, color_sections(app)).thumb(132.0, 16.0, 1.0).columns(1);
    let view = cx.new(|_| gallery);
    let anchor = app.command_anchor;
    super::open_ribbon_view(app, anchor, view, window, cx);
}

// --------------------------------------------------------------- Chart Styles

/// Number of chart styles offered. They are combinations of what the model
/// can express (fills, borders, gaps, labels, gridlines), not Excel's own.
pub const STYLES: usize = 8;

/// Give a chart the look of style `index` (0 is the default look).
pub fn apply_style(chart: &mut Chart, index: usize) {
    let bars = matches!(chart.kind.family(), Family::Column | Family::Bar);
    let up = axis_of(chart, false, false);
    // Every style starts from the default look.
    chart.chart_area = ShapeFormat::default();
    chart.plot_area = ShapeFormat::default();
    chart.gap_width = None;
    chart.overlap = None;
    chart.data_labels = false;
    for series in &mut chart.series {
        series.labels = None;
        if bars || chart.kind.is_round() {
            series.line = Line::default();
        }
    }
    if has_axis(chart, up) {
        let axis = up.of_mut(&mut chart.axes);
        axis.visible = true;
        axis.major_gridlines = true;
        axis.minor_gridlines = false;
    }
    let grey = Fill::Solid(Color(0xF2F2F2));
    let outline = |color: u32, width: f32| Line {
        shown: Some(true),
        color: Some(Color(color)),
        width: Some(width),
        ..Line::default()
    };
    match index {
        1 => {
            // Values on the bars instead of a scale.
            chart.data_labels = true;
            chart.label_options = DataLabels {
                percent: chart.kind.is_round(),
                value: !chart.kind.is_round(),
                ..DataLabels::default()
            };
            if has_axis(chart, up) {
                let axis = up.of_mut(&mut chart.axes);
                axis.visible = false;
                axis.major_gridlines = false;
            }
        }
        2 => {
            chart.chart_area.fill = grey;
            chart.plot_area.fill = Fill::Solid(Color::WHITE);
        }
        3 => {
            chart.plot_area.fill = grey;
            chart.gap_width = bars.then_some(100);
        }
        4 => {
            chart.gap_width = bars.then_some(50);
            chart.chart_area.border = outline(0x7F7F7F, 1.0);
            if has_axis(chart, up) {
                up.of_mut(&mut chart.axes).minor_gridlines = true;
            }
        }
        5 => {
            // Shapes kept apart by a white line.
            chart.gap_width = bars.then_some(60);
            chart.overlap = bars.then_some(0);
            if bars || chart.kind.is_round() || chart.kind.family() == Family::Area {
                for series in &mut chart.series {
                    series.line = outline(0xFFFFFF, 1.5);
                }
            }
        }
        6 => {
            chart.chart_area.fill = Fill::Solid(shade(PALETTE[0], 0.88));
            chart.chart_area.border.shown = Some(false);
            chart.plot_area.fill = Fill::Solid(Color::WHITE);
        }
        7 => {
            chart.chart_area.border = outline(0x404040, 1.5);
            chart.plot_area.border = outline(0xBFBFBF, 0.75);
            chart.gap_width = bars.then_some(30);
        }
        _ => {}
    }
}

fn style_section(app: &ExcelApp) -> Option<Section> {
    let chart = app.selected_chart()?;
    let (sample, data) = insert::sample(chart.kind);
    let items = (0..STYLES)
        .map(|index| {
            let mut picture = sample.clone();
            picture.axes = Axes::for_kind(picture.kind);
            picture.axes.category.visible = false;
            apply_style(&mut picture, index);
            Item::new(
                Picture::Chart(Rc::new(picture), Rc::new(data.clone())),
                super::elements::fill(t("Style {0}", "Style {0}"), &[&(index + 1).to_string()]),
                move |a, _, cx| a.edit_chart(t("Chart Style", "Style du graphique"), cx, |c| apply_style(c, index)),
            )
        })
        .collect();
    Some(Section {
        title: t("Style", "Style").into(),
        items,
    })
}

pub(crate) fn open_styles(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let sections: Vec<Section> = style_section(app).into_iter().collect();
    let gallery = Gallery::new(app, cx, sections).thumb(92.0, 62.0, 0.34).columns(4);
    let view = cx.new(|_| gallery);
    let anchor = app.command_anchor;
    super::open_ribbon_view(app, anchor, view, window, cx);
}

/// What the brush beside a chart opens: the styles, then the colours.
pub(super) fn styles_flyout(app: &mut ExcelApp, cx: &mut Cx) -> Entity<Gallery> {
    let mut sections: Vec<Section> = style_section(app).into_iter().collect();
    sections.extend(color_sections(app));
    let gallery = Gallery::new(app, cx, sections).thumb(74.0, 50.0, 0.3).columns(4);
    cx.new(|_| gallery)
}

// ------------------------------------------------------------ Chart Elements

/// The elements of the "+" button, in Excel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Axes,
    AxisTitles,
    Title,
    DataLabels,
    DataTable,
    ErrorBars,
    Gridlines,
    Legend,
    Trendline,
}

impl Toggle {
    fn label(self) -> &'static str {
        match self {
            Toggle::Axes => t("Axes", "Axes"),
            Toggle::AxisTitles => t("Axis Titles", "Titres des axes"),
            Toggle::Title => t("Chart Title", "Titre du graphique"),
            Toggle::DataLabels => t("Data Labels", "Étiquettes de données"),
            Toggle::DataTable => t("Data Table", "Table de données"),
            Toggle::ErrorBars => t("Error Bars", "Barres d'erreur"),
            Toggle::Gridlines => t("Gridlines", "Quadrillage"),
            Toggle::Legend => t("Legend", "Légende"),
            Toggle::Trendline => t("Trendline", "Courbe de tendance"),
        }
    }

    /// The check boxes a chart of this kind gets.
    pub fn available(chart: &Chart) -> Vec<Toggle> {
        let mut out = Vec::new();
        let axes = !chart.kind.is_round();
        let cartesian = axes && chart.kind.family() != Family::Radar;
        if axes {
            out.push(Toggle::Axes);
        }
        if cartesian {
            out.push(Toggle::AxisTitles);
        }
        out.push(Toggle::Title);
        out.push(Toggle::DataLabels);
        if cartesian && !chart.kind.is_xy() && !chart.kind.is_horizontal() && !chart.kind.is_3d() {
            out.push(Toggle::DataTable);
        }
        if takes_trendlines(chart) {
            out.push(Toggle::ErrorBars);
        }
        if axes {
            out.push(Toggle::Gridlines);
        }
        out.push(Toggle::Legend);
        if takes_trendlines(chart) {
            out.push(Toggle::Trendline);
        }
        out
    }

    pub fn is_on(self, chart: &Chart) -> bool {
        let primary = [AxisId::Category, AxisId::Value];
        match self {
            Toggle::Axes => primary.iter().any(|a| a.of(&chart.axes).visible),
            Toggle::AxisTitles => primary.iter().any(|a| a.of(&chart.axes).title.is_some()),
            Toggle::Title => chart.title != ChartTitle::None,
            Toggle::DataLabels => chart.series.iter().any(|s| chart.labels_for(s).is_some()),
            Toggle::DataTable => chart.data_table.is_some(),
            Toggle::ErrorBars => chart.series.iter().any(|s| !s.error_bars.is_empty()),
            Toggle::Gridlines => primary.iter().any(|a| a.of(&chart.axes).major_gridlines || a.of(&chart.axes).minor_gridlines),
            Toggle::Legend => chart.legend != LegendPos::None,
            Toggle::Trendline => chart.series.iter().any(|s| !s.trendlines.is_empty()),
        }
    }

    /// Show the element as Excel adds it by default, or remove it.
    /// `series`: the series a trendline goes on.
    pub fn set(self, chart: &mut Chart, on: bool, series: usize) {
        let primary = [AxisId::Category, AxisId::Value];
        match self {
            Toggle::Axes => primary.iter().for_each(|a| a.of_mut(&mut chart.axes).visible = on),
            Toggle::AxisTitles => {
                for axis in primary {
                    axis.of_mut(&mut chart.axes).title = on.then(|| t("Axis Title", "Titre de l'axe").to_string());
                }
            }
            Toggle::Title => chart.title = if on { ChartTitle::Auto } else { ChartTitle::None },
            Toggle::DataLabels if on => add_data_labels(chart, None),
            Toggle::DataLabels => {
                chart.data_labels = false;
                for s in &mut chart.series {
                    s.labels = None;
                }
            }
            Toggle::DataTable => chart.data_table = on.then(DataTable::default),
            Toggle::ErrorBars => set_error_bars(chart, None, on.then_some(ErrorAmount::StdErr)),
            Toggle::Gridlines => {
                let fresh = Axes::for_kind(chart.kind);
                for axis in primary {
                    let major = on && axis.of(&fresh).major_gridlines;
                    let axis = axis.of_mut(&mut chart.axes);
                    axis.major_gridlines = major;
                    axis.minor_gridlines = false;
                }
            }
            Toggle::Legend => chart.legend = if on { LegendPos::Right } else { LegendPos::None },
            Toggle::Trendline if on => {
                if let Some(s) = chart.series.get_mut(series) {
                    s.trendlines.push(Trendline::new(TrendKind::Linear));
                }
            }
            Toggle::Trendline => chart.series.iter_mut().for_each(|s| s.trendlines.clear()),
        }
    }
}

/// The check-list of the "+" button beside a chart.
pub(crate) struct ElementsFlyout {
    app: WeakEntity<ExcelApp>,
}

pub(super) fn elements_flyout(_app: &mut ExcelApp, cx: &mut Cx) -> Entity<ElementsFlyout> {
    let app = cx.weak_entity();
    cx.new(|cx| {
        // The boxes follow the chart, whatever changes it.
        if let Some(app) = app.upgrade() {
            cx.observe(&app, |_, _, cx| cx.notify()).detach();
        }
        ElementsFlyout { app }
    })
}

impl Render for ElementsFlyout {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<(Toggle, bool)> = self
            .app
            .upgrade()
            .and_then(|app| {
                let app = app.read(cx);
                app.selected_chart().map(|chart| Toggle::available(chart).into_iter().map(|t| (t, t.is_on(chart))).collect())
            })
            .unwrap_or_default();
        let app = self.app.clone();
        div()
            .w(px(190.))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .text_size(px(12.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::ACCENT_DARK))
                    .child(t("Chart Elements", "Éléments de graphique")),
            )
            .children(rows.into_iter().map(|(toggle, on)| {
                let app = app.clone();
                Checkbox::new(("chart-element", toggle as usize), toggle.label(), on).on_change(move |on: &bool, _, cx| {
                    let on = *on;
                    app.update(cx, |app, cx| {
                        let series = app.selected_chart().and_then(|c| target_series(c, app.chart_element())).unwrap_or(0);
                        app.edit_chart(toggle.label(), cx, |c| toggle.set(c, on, series));
                    })
                    .ok();
                })
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::ast::RefExpr;
    use opensheet_engine::chart::{ChartKind, Series};

    fn chart(kind: ChartKind, series: usize) -> Chart {
        let mut chart = Chart::new(kind);
        chart.series = (0..series).map(|i| Series::new(RefExpr::cell(0, i as u32))).collect();
        chart
    }

    #[test]
    fn quick_layouts_combine_elements() {
        let mut c = chart(ChartKind::Column, 2);
        apply_quick_layout(&mut c, &QUICK_LAYOUTS[1]);
        assert_eq!(c.legend, LegendPos::Top);
        assert!(c.data_labels && !c.axes.value.visible && !c.axes.value.major_gridlines);
        apply_quick_layout(&mut c, &QUICK_LAYOUTS[4]);
        assert!(c.data_table.is_some() && c.axes.value.title.is_some() && c.axes.category.title.is_none());
        assert!(c.axes.value.visible && c.axes.value.major_gridlines && !c.data_labels);
        apply_quick_layout(&mut c, &QUICK_LAYOUTS[7]);
        assert_eq!((c.gap_width, c.legend), (Some(0), LegendPos::None));
        // A title typed by the user survives a layout that has a title.
        c.title = ChartTitle::Text("Sales".into());
        c.axes.value.title = Some("Units".into());
        apply_quick_layout(&mut c, &QUICK_LAYOUTS[8]);
        assert_eq!(c.title, ChartTitle::Text("Sales".into()));
        assert_eq!(c.axes.value.title.as_deref(), Some("Units"));
        assert_eq!(c.gap_width, None);
        // Bars: the horizontal axis is the value axis.
        let mut bar = chart(ChartKind::Bar, 1);
        apply_quick_layout(&mut bar, &QUICK_LAYOUTS[4]);
        assert!(bar.axes.category.title.is_some() && bar.data_table.is_none());
    }

    #[test]
    fn palettes() {
        assert_eq!(palette(0, 3), vec![Color(PALETTE[0]), Color(PALETTE[1]), Color(PALETTE[2])]);
        assert_eq!(palette(1, 4)[..3], [Color(PALETTE[0]), Color(PALETTE[2]), Color(PALETTE[4])]);
        // Monochromatic: one accent from dark to light, or back.
        let mono = palette(COLORFUL, 3);
        assert!(mono[0].r() < mono[2].r() && mono[0].b() < mono[2].b());
        let back = palette(COLORFUL + 7, 3);
        assert_eq!(back[0], mono[2]);
        let mut c = chart(ChartKind::Column, 2);
        apply_palette(&mut c, 2, 3);
        assert_eq!(c.series[1].color, Some(Color(PALETTE[3])));
        apply_palette(&mut c, 0, 3);
        assert!(c.series.iter().all(|s| s.color.is_none()));
        // A pie is coloured point by point.
        let mut pie = chart(ChartKind::Pie, 1);
        apply_palette(&mut pie, 1, 3);
        assert_eq!(pie.series[0].point(2).and_then(|p| p.color), Some(Color(PALETTE[4])));
        assert_eq!(pie.series[0].color, None);
    }

    #[test]
    fn element_check_boxes() {
        let mut c = chart(ChartKind::Column, 2);
        assert!(Toggle::available(&c).contains(&Toggle::Trendline));
        assert!(Toggle::Gridlines.is_on(&c) && !Toggle::Legend.is_on(&c));
        Toggle::Gridlines.set(&mut c, false, 0);
        assert!(!c.axes.value.major_gridlines);
        Toggle::Gridlines.set(&mut c, true, 0);
        assert!(c.axes.value.major_gridlines && !c.axes.category.major_gridlines);
        Toggle::Trendline.set(&mut c, true, 1);
        assert_eq!(c.series[1].trendlines.len(), 1);
        Toggle::ErrorBars.set(&mut c, true, 0);
        assert!(c.series.iter().all(|s| s.error_bars.len() == 1));
        Toggle::DataLabels.set(&mut c, true, 0);
        assert!(Toggle::DataLabels.is_on(&c));
        let pie = chart(ChartKind::Pie, 1);
        assert_eq!(Toggle::available(&pie), [Toggle::Title, Toggle::DataLabels, Toggle::Legend]);
    }

    #[test]
    fn styles_start_from_the_default_look() {
        let mut c = chart(ChartKind::Column, 2);
        apply_style(&mut c, 1);
        assert!(c.data_labels && !c.axes.value.visible);
        apply_style(&mut c, 5);
        assert!(!c.data_labels && c.axes.value.visible);
        assert_eq!(c.series[0].line.color, Some(Color::WHITE));
        apply_style(&mut c, 0);
        assert_eq!(c.series[0].line, Line::default());
        assert_eq!((c.gap_width, c.chart_area.clone()), (None, ShapeFormat::default()));
    }
}
