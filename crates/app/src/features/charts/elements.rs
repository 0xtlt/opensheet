//! The elements of a chart as the user knows them: their names, which of
//! them a chart has, deleting one, giving one its automatic look back, and
//! the context menu of each.

use super::{Cmd, insert};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, ContextMenu, MenuSection, chart_format};
use crate::i18n::{Lang, lang, t};
use opensheet_engine::chart::{
    Axis, Chart, ChartData, ChartTitle, DataLabels, Family, LegendPos, Line, ShapeFormat,
    TextFormat, TrendKind, Trendline,
};
use opensheet_engine::chart_layout::{
    AxisId, ChartElement, LayoutOptions, TextMeasure, layout_chart_elements,
};

/// Put `args` into a pattern at `{0}`, `{1}`...
pub(crate) fn fill(pattern: &str, args: &[&str]) -> String {
    let mut out = pattern.to_string();
    for (i, arg) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), arg);
    }
    out
}

/// Whether an axis runs across the chart (the value axis of a bar chart
/// does, like the category axis of the others).
pub(crate) fn is_horizontal(chart: &Chart, axis: AxisId) -> bool {
    axis.is_category() != chart.kind.is_horizontal()
}

/// The axis that runs across (`horizontal`) or up the chart.
pub(crate) fn axis_of(chart: &Chart, horizontal: bool, secondary: bool) -> AxisId {
    match (horizontal != chart.kind.is_horizontal(), secondary) {
        (true, false) => AxisId::Category,
        (true, true) => AxisId::SecondaryCategory,
        (false, false) => AxisId::Value,
        (false, true) => AxisId::SecondaryValue,
    }
}

/// Whether the chart has axes at all, and this one in particular.
pub(crate) fn has_axis(chart: &Chart, axis: AxisId) -> bool {
    if chart.kind.is_round() {
        return false;
    }
    !axis.is_secondary() || chart.has_secondary_axis()
}

/// "Horizontal (Category) Axis", as Excel names axes.
pub(crate) fn axis_name(chart: &Chart, axis: AxisId) -> String {
    if chart.kind.family() == Family::Radar {
        return if axis.is_category() {
            t("Category Labels", "Étiquettes de catégorie").to_string()
        } else {
            t("Radar (Value) Axis", "Axe Radar (Valeur)").to_string()
        };
    }
    let direction = if is_horizontal(chart, axis) {
        t("Horizontal", "Horizontal")
    } else {
        t("Vertical", "Vertical")
    };
    let role = if axis.is_category() && !chart.kind.is_xy() {
        t("Category", "Catégorie")
    } else {
        t("Value", "Valeur")
    };
    let pattern = if axis.is_secondary() {
        t("Secondary {0} ({1}) Axis", "Axe {0} secondaire ({1})")
    } else {
        t("{0} ({1}) Axis", "Axe {0} ({1})")
    };
    fill(pattern, &[direction, role])
}

/// The name of an axis inside a longer name: French writes "Titre de
/// l'axe Horizontal (Catégorie)".
fn axis_in_phrase(chart: &Chart, axis: AxisId) -> String {
    let name = axis_name(chart, axis);
    match lang() {
        Lang::Fr => {
            let mut chars = name.chars();
            chars
                .next()
                .map(|first| first.to_lowercase().chain(chars).collect())
                .unwrap_or_default()
        }
        _ => name,
    }
}

pub(crate) fn series_name(data: &ChartData, series: usize) -> String {
    match data.series.get(series) {
        Some(s) if !s.name.is_empty() => s.name.clone(),
        _ => format!("{}{}", t("Series", "Série"), series + 1),
    }
}

/// The name of an element in the Chart Elements list.
pub fn element_name(chart: &Chart, data: &ChartData, element: ChartElement) -> String {
    let series = |s: u32| {
        fill(
            t("Series \"{0}\"", "Série \"{0}\""),
            &[&series_name(data, s as usize)],
        )
    };
    match element {
        ChartElement::ChartArea => t("Chart Area", "Zone de graphique").to_string(),
        ChartElement::PlotArea => t("Plot Area", "Zone de traçage").to_string(),
        ChartElement::Title => t("Chart Title", "Titre du graphique").to_string(),
        ChartElement::Legend => t("Legend", "Légende").to_string(),
        ChartElement::LegendEntry(_) => t("Legend Entry", "Élément de légende").to_string(),
        ChartElement::Axis(axis) => axis_name(chart, axis),
        ChartElement::AxisTitle(axis) => fill(
            t("{0} Title", "Titre de l'{0}"),
            &[&axis_in_phrase(chart, axis)],
        ),
        ChartElement::MajorGridlines(axis) => fill(
            t("{0} Major Gridlines", "Quadrillage principal de l'{0}"),
            &[&axis_in_phrase(chart, axis)],
        ),
        ChartElement::MinorGridlines(axis) => fill(
            t("{0} Minor Gridlines", "Quadrillage secondaire de l'{0}"),
            &[&axis_in_phrase(chart, axis)],
        ),
        ChartElement::Series(s) => series(s),
        ChartElement::Point(s, k) => {
            let point = data
                .categories
                .get(k as usize)
                .cloned()
                .unwrap_or_else(|| (k + 1).to_string());
            fill(
                t("{0} Point \"{1}\"", "{0} Point \"{1}\""),
                &[&series(s), &point],
            )
        }
        ChartElement::DataLabels(s) => fill(
            t("{0} Data Labels", "{0} Étiquettes de données"),
            &[&series(s)],
        ),
        ChartElement::Trendline(s, i) => fill(
            t("{0} Trendline {1}", "{0} Courbe de tendance {1}"),
            &[&series(s), &(i + 1).to_string()],
        ),
        ChartElement::ErrorBars(s, i) => {
            let horizontal = chart
                .series
                .get(s as usize)
                .and_then(|x| x.error_bars.get(i as usize))
                .is_some_and(|b| b.horizontal);
            let pattern = if horizontal {
                t("{0} X Error Bars", "{0} Barres d'erreur X")
            } else {
                t("{0} Y Error Bars", "{0} Barres d'erreur Y")
            };
            fill(pattern, &[&series(s)])
        }
        ChartElement::DataTable => t("Data Table", "Table de données").to_string(),
    }
}

/// The title of the Format pane for an element ("Format Axis").
pub fn pane_title(element: ChartElement) -> &'static str {
    match element {
        ChartElement::ChartArea => t("Format Chart Area", "Format de la zone de graphique"),
        ChartElement::PlotArea => t("Format Plot Area", "Format de zone de traçage"),
        ChartElement::Title => t("Format Chart Title", "Format du titre du graphique"),
        ChartElement::Legend | ChartElement::LegendEntry(_) => {
            t("Format Legend", "Format de légende")
        }
        ChartElement::Axis(_) => t("Format Axis", "Format de l'axe"),
        ChartElement::AxisTitle(_) => t("Format Axis Title", "Format du titre d'axe"),
        ChartElement::MajorGridlines(_) => {
            t("Format Major Gridlines", "Format du quadrillage principal")
        }
        ChartElement::MinorGridlines(_) => {
            t("Format Minor Gridlines", "Format du quadrillage secondaire")
        }
        ChartElement::Series(_) => t("Format Data Series", "Format de série de données"),
        ChartElement::Point(..) => t("Format Data Point", "Format de point de données"),
        ChartElement::DataLabels(_) => t("Format Data Labels", "Format des étiquettes de données"),
        ChartElement::Trendline(..) => t("Format Trendline", "Format de courbe de tendance"),
        ChartElement::ErrorBars(..) => t("Format Error Bars", "Format des barres d'erreur"),
        ChartElement::DataTable => t("Format Data Table", "Format de table de données"),
    }
}

/// Whether the chart (still) has an element.
pub fn exists(chart: &Chart, element: ChartElement) -> bool {
    let series = |s: u32| chart.series.get(s as usize);
    match element {
        ChartElement::ChartArea | ChartElement::PlotArea => true,
        ChartElement::Title => chart.title != ChartTitle::None,
        ChartElement::Legend | ChartElement::LegendEntry(_) => chart.legend != LegendPos::None,
        ChartElement::Axis(axis) => has_axis(chart, axis) && axis.of(&chart.axes).visible,
        ChartElement::AxisTitle(axis) => {
            has_axis(chart, axis) && axis.of(&chart.axes).title.is_some()
        }
        ChartElement::MajorGridlines(axis) => {
            has_axis(chart, axis) && axis.of(&chart.axes).major_gridlines
        }
        ChartElement::MinorGridlines(axis) => {
            has_axis(chart, axis) && axis.of(&chart.axes).minor_gridlines
        }
        ChartElement::Series(s) | ChartElement::Point(s, _) => series(s).is_some(),
        ChartElement::DataLabels(s) => series(s).is_some_and(|x| chart.labels_for(x).is_some()),
        ChartElement::Trendline(s, i) => {
            series(s).is_some_and(|x| (i as usize) < x.trendlines.len())
        }
        ChartElement::ErrorBars(s, i) => {
            series(s).is_some_and(|x| (i as usize) < x.error_bars.len())
        }
        ChartElement::DataTable => chart.data_table.is_some(),
    }
}

/// Widths good enough to tell which elements a chart shows.
struct Rough;

impl TextMeasure for Rough {
    fn width(&self, text: &str, size: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
}

/// The elements of a chart for the Chart Elements list: the chart area,
/// the others by name as Excel lists them, then what belongs to each
/// series.
pub fn listed(chart: &Chart, data: &ChartData) -> Vec<ChartElement> {
    let locale = crate::i18n::lang().locale();
    let options = LayoutOptions {
        scale: 1.0,
        locale,
        labels: Default::default(),
    };
    let drawn = layout_chart_elements(chart, data, chart.size, &options, &Rough).elements();
    let (mut general, mut of_series): (Vec<ChartElement>, Vec<ChartElement>) = drawn
        .into_iter()
        .filter(|e| *e != ChartElement::ChartArea)
        .partition(|e| e.series().is_none());
    general.sort_by_key(|e| element_name(chart, data, *e).to_lowercase());
    of_series.sort_by_key(|e| {
        let rank = match e {
            ChartElement::Series(_) | ChartElement::Point(..) => 0,
            ChartElement::DataLabels(_) => 1,
            ChartElement::Trendline(_, i) => 2 + *i,
            ChartElement::ErrorBars(_, i) => 100 + *i,
            _ => 200,
        };
        (e.series(), rank)
    });
    let mut out = vec![ChartElement::ChartArea];
    out.extend(general);
    out.extend(of_series);
    out
}

/// Remove an element from a chart (the Delete key). Returns false when
/// there is nothing to remove.
pub fn delete(chart: &mut Chart, element: ChartElement) -> bool {
    match element {
        ChartElement::ChartArea => return false,
        // Like Excel: the plot area cannot go, it loses its formatting.
        ChartElement::PlotArea => chart.plot_area = ShapeFormat::default(),
        // What is removed loses the place it was dragged to.
        ChartElement::Title => {
            chart.title = ChartTitle::None;
            chart.title_layout = None;
        }
        ChartElement::Legend => {
            chart.legend = LegendPos::None;
            chart.legend_layout = None;
        }
        ChartElement::LegendEntry(i) => {
            if !chart.legend_hidden.contains(&i) {
                chart.legend_hidden.push(i);
            }
        }
        ChartElement::Axis(axis) => axis.of_mut(&mut chart.axes).visible = false,
        ChartElement::AxisTitle(axis) => {
            let axis = axis.of_mut(&mut chart.axes);
            axis.title = None;
            axis.title_layout = None;
        }
        ChartElement::MajorGridlines(axis) => axis.of_mut(&mut chart.axes).major_gridlines = false,
        ChartElement::MinorGridlines(axis) => axis.of_mut(&mut chart.axes).minor_gridlines = false,
        ChartElement::Series(s) | ChartElement::Point(s, _) => {
            if (s as usize) >= chart.series.len() {
                return false;
            }
            chart.series.remove(s as usize);
            chart.legend_hidden.clear();
        }
        ChartElement::DataLabels(s) => {
            let Some(series) = chart.series.get_mut(s as usize) else {
                return false;
            };
            series.labels = Some(DataLabels::none());
            series.label_offsets.clear();
            // Once no series is labelled, the chart is back to "no labels".
            if chart
                .series
                .iter()
                .all(|x| x.labels.as_ref().is_some_and(|l| !l.any()))
            {
                chart.data_labels = false;
                for x in &mut chart.series {
                    x.labels = None;
                }
            }
        }
        ChartElement::Trendline(s, i) => match chart.series.get_mut(s as usize) {
            Some(series) if (i as usize) < series.trendlines.len() => {
                series.trendlines.remove(i as usize);
                chart.legend_hidden.clear();
            }
            _ => return false,
        },
        ChartElement::ErrorBars(s, i) => match chart.series.get_mut(s as usize) {
            Some(series) if (i as usize) < series.error_bars.len() => {
                series.error_bars.remove(i as usize);
            }
            _ => return false,
        },
        ChartElement::DataTable => chart.data_table = None,
    }
    true
}

/// "Reset to Match Style": the element takes its automatic look back,
/// and the place the chart gives it. The chart area stands for the whole
/// chart, as far as places and fonts go.
pub fn reset(chart: &mut Chart, element: ChartElement) {
    match element {
        ChartElement::ChartArea => {
            chart.chart_area = ShapeFormat::default();
            chart.clear_manual_layout();
            for text in chart.text_formats_mut() {
                *text = TextFormat::default();
            }
        }
        ChartElement::PlotArea => {
            chart.plot_area = ShapeFormat::default();
            chart.plot_layout = None;
            chart.plot_layout_outer = false;
        }
        ChartElement::Title => {
            chart.title_text = TextFormat::default();
            chart.title_layout = None;
        }
        ChartElement::Legend | ChartElement::LegendEntry(_) => {
            chart.legend_text = TextFormat::default();
            chart.legend_layout = None;
        }
        ChartElement::AxisTitle(axis) => {
            let axis = axis.of_mut(&mut chart.axes);
            axis.title_text = TextFormat::default();
            axis.title_layout = None;
        }
        ChartElement::MajorGridlines(axis) => {
            axis.of_mut(&mut chart.axes).major_gridline = Line::default()
        }
        ChartElement::MinorGridlines(axis) => {
            axis.of_mut(&mut chart.axes).minor_gridline = Line::default()
        }
        ChartElement::Series(s) => {
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.color = None;
                series.line = Line::default();
                series.marker = None;
                series.smooth = None;
                for point in &mut series.points {
                    point.color = None;
                }
                series.points.retain(|p| p.explosion.is_some());
            }
        }
        ChartElement::Point(s, k) => {
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.points.retain(|p| p.index != k);
            }
        }
        ChartElement::Axis(axis) => {
            let old = axis.of(&chart.axes).clone();
            // What the axis shows stays; how it shows it is automatic again.
            // Its title and its gridlines are elements of their own.
            *axis.of_mut(&mut chart.axes) = Axis {
                visible: old.visible,
                title: old.title,
                title_text: old.title_text,
                title_layout: old.title_layout,
                major_gridlines: old.major_gridlines,
                minor_gridlines: old.minor_gridlines,
                major_gridline: old.major_gridline,
                minor_gridline: old.minor_gridline,
                ..Axis::new()
            };
        }
        ChartElement::DataLabels(s) => {
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.labels = Some(DataLabels::default());
                series.label_offsets.clear();
            }
        }
        ChartElement::Trendline(s, i) => {
            if let Some(trend) = chart
                .series
                .get_mut(s as usize)
                .and_then(|x| x.trendlines.get_mut(i as usize))
            {
                trend.line = Line::default();
            }
        }
        _ => {}
    }
}

/// Give a series the labels of the chart to change on its own.
pub(crate) fn own_labels(chart: &mut Chart, series: usize) -> Option<&mut DataLabels> {
    let inherited = chart
        .labels_for(chart.series.get(series)?)
        .cloned()
        .unwrap_or_default();
    let series = chart.series.get_mut(series)?;
    Some(series.labels.get_or_insert(inherited))
}

/// Label the points of a series (`None`: of every series).
pub(crate) fn add_data_labels(chart: &mut Chart, series: Option<usize>) {
    match series {
        Some(s) => {
            if let Some(labels) = own_labels(chart, s) {
                if !labels.any() {
                    *labels = DataLabels::default();
                }
            }
        }
        None => {
            chart.data_labels = true;
            if !chart.label_options.any() {
                chart.label_options = DataLabels::default();
            }
            for series in &mut chart.series {
                series.labels = None;
            }
        }
    }
}

/// Whether trendlines and error bars can be added to this chart.
pub(crate) fn takes_trendlines(chart: &Chart) -> bool {
    let kind = chart.kind;
    !kind.is_3d() && !kind.is_round() && kind.family() != Family::Radar && !kind.is_stacked()
}

/// The series an "Add …" command applies to when no question needs
/// asking: the one the selection belongs to, or the only one.
pub(crate) fn target_series(chart: &Chart, element: ChartElement) -> Option<usize> {
    element
        .series()
        .filter(|s| *s < chart.series.len())
        .or((chart.series.len() == 1).then_some(0))
}

/// What the context menu of an element offers, in Excel's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Delete,
    Reset,
    EditText,
    ChangeType,
    SelectData,
    MoveChart,
    Rotation,
    AddDataLabels,
    AddTrendline,
    AddMajorGridlines,
    AddMinorGridlines,
    FormatAxis,
    FormatLabels,
    Format,
}

/// The entries of the context menu of an element.
pub fn menu_items(chart: &Chart, element: ChartElement) -> Vec<MenuItem> {
    let mut out = Vec::new();
    if element != ChartElement::ChartArea {
        out.push(MenuItem::Delete);
    }
    out.push(MenuItem::Reset);
    if matches!(element, ChartElement::Title | ChartElement::AxisTitle(_)) {
        out.push(MenuItem::EditText);
    }
    out.push(MenuItem::ChangeType);
    out.push(MenuItem::SelectData);
    if element == ChartElement::ChartArea {
        out.push(MenuItem::MoveChart);
    }
    if chart.kind.is_3d() {
        out.push(MenuItem::Rotation);
    }
    match element {
        ChartElement::Series(s) | ChartElement::Point(s, _) => {
            let labelled = chart
                .series
                .get(s as usize)
                .is_some_and(|x| chart.labels_for(x).is_some());
            out.push(if labelled {
                MenuItem::FormatLabels
            } else {
                MenuItem::AddDataLabels
            });
            if takes_trendlines(chart) {
                out.push(MenuItem::AddTrendline);
            }
        }
        ChartElement::Axis(axis) => {
            let axis = axis.of(&chart.axes);
            if !axis.major_gridlines {
                out.push(MenuItem::AddMajorGridlines);
            }
            if !axis.minor_gridlines {
                out.push(MenuItem::AddMinorGridlines);
            }
        }
        ChartElement::MajorGridlines(_) | ChartElement::MinorGridlines(_) => {
            out.push(MenuItem::FormatAxis)
        }
        _ => {}
    }
    out.push(MenuItem::Format);
    out
}

/// "Format Data Series…": the last entry of every context menu.
fn format_entry_label(element: ChartElement) -> String {
    let title = pane_title(element);
    // Menus name gridlines without "major" / "minor".
    let title = match element {
        ChartElement::MajorGridlines(_) | ChartElement::MinorGridlines(_) => {
            t("Format Gridlines", "Format du quadrillage")
        }
        _ => title,
    };
    format!("{title}…")
}

/// Add the entries of an element to the context menu of its chart.
pub fn context_menu(app: &ExcelApp, chart: &Chart, element: ChartElement, menu: &mut ContextMenu) {
    for item in menu_items(chart, element) {
        let (section, entry) = match item {
            MenuItem::Delete => (
                MenuSection::Edit,
                MenuEntry::new(t("Delete", "Supprimer"), move |a, _, cx| {
                    super::delete_selected_element(a, cx)
                }),
            ),
            MenuItem::Reset => (
                MenuSection::Edit,
                MenuEntry::command(chart_format::Cmd::ResetStyle, app),
            ),
            MenuItem::EditText => (
                MenuSection::Edit,
                MenuEntry::new(t("Edit Text", "Modifier le texte"), move |a, window, cx| {
                    if let Some(id) = a.selected_chart().map(|c| c.id) {
                        super::begin_text_edit(a, id, element, window, cx);
                    }
                }),
            ),
            MenuItem::ChangeType => {
                let label = match element {
                    ChartElement::Series(_) | ChartElement::Point(..) => t(
                        "Change Series Chart Type…",
                        "Modifier le type de graphique Série de données…",
                    ),
                    _ => t("Change Chart Type…", "Modifier le type de graphique…"),
                };
                (
                    MenuSection::Data,
                    MenuEntry::new(label, |a, window, cx| {
                        insert::open_change_type_dialog(a, window, cx)
                    })
                    .icon(Cmd::ChangeType.icon()),
                )
            }
            MenuItem::SelectData => (
                MenuSection::Data,
                MenuEntry::new(
                    t("Select Data…", "Sélectionner des données…"),
                    |a, window, cx| chart_format::Cmd::SelectData.run(a, window, cx),
                )
                .icon(chart_format::Cmd::SelectData.icon()),
            ),
            MenuItem::MoveChart => (
                MenuSection::Data,
                MenuEntry::new(
                    t("Move Chart…", "Déplacer le graphique…"),
                    |a, window, cx| chart_format::Cmd::MoveChart.run(a, window, cx),
                ),
            ),
            MenuItem::Rotation => (
                MenuSection::Data,
                MenuEntry::new(t("3-D Rotation…", "Rotation 3D…"), |a, window, cx| {
                    a.select_chart_element(ChartElement::ChartArea, cx);
                    chart_format::open_pane(a, window, cx);
                }),
            ),
            MenuItem::AddDataLabels => (
                MenuSection::Format,
                MenuEntry::new(
                    t("Add Data Labels", "Ajouter des étiquettes de données"),
                    move |a, _, cx| {
                        a.edit_chart(t("Data Labels", "Étiquettes de données"), cx, move |c| {
                            add_data_labels(c, element.series())
                        })
                    },
                ),
            ),
            MenuItem::AddTrendline => (
                MenuSection::Format,
                MenuEntry::new(
                    t("Add Trendline…", "Ajouter une courbe de tendance…"),
                    move |a, window, cx| {
                        let Some(series) = element.series() else {
                            return;
                        };
                        add_trendline(a, series, TrendKind::Linear, cx);
                        chart_format::open_pane(a, window, cx);
                    },
                ),
            ),
            MenuItem::AddMajorGridlines | MenuItem::AddMinorGridlines => {
                let major = item == MenuItem::AddMajorGridlines;
                let ChartElement::Axis(axis) = element else {
                    continue;
                };
                let label = if major {
                    t("Add Major Gridlines", "Ajouter un quadrillage principal")
                } else {
                    t("Add Minor Gridlines", "Ajouter un quadrillage secondaire")
                };
                (
                    MenuSection::Format,
                    MenuEntry::new(label, move |a, _, cx| {
                        a.edit_chart(t("Gridlines", "Quadrillage"), cx, move |c| {
                            let axis = axis.of_mut(&mut c.axes);
                            if major {
                                axis.major_gridlines = true
                            } else {
                                axis.minor_gridlines = true
                            }
                        })
                    }),
                )
            }
            MenuItem::FormatAxis => {
                let (ChartElement::MajorGridlines(axis) | ChartElement::MinorGridlines(axis)) =
                    element
                else {
                    continue;
                };
                (
                    MenuSection::Format,
                    MenuEntry::new(
                        format_entry_label(ChartElement::Axis(axis)),
                        move |a, window, cx| {
                            a.select_chart_element(ChartElement::Axis(axis), cx);
                            chart_format::open_pane(a, window, cx);
                        },
                    ),
                )
            }
            MenuItem::FormatLabels => {
                let Some(series) = element.series() else {
                    continue;
                };
                let labels = ChartElement::DataLabels(series as u32);
                (
                    MenuSection::Format,
                    MenuEntry::new(format_entry_label(labels), move |a, window, cx| {
                        a.select_chart_element(labels, cx);
                        chart_format::open_pane(a, window, cx);
                    }),
                )
            }
            MenuItem::Format => (
                MenuSection::Format,
                MenuEntry::new(format_entry_label(element), |a, window, cx| {
                    chart_format::open_pane(a, window, cx)
                })
                .icon(chart_format::Cmd::FormatSelection.icon()),
            ),
        };
        menu.add(section, entry);
    }
}

/// Add a trendline to a series of the selected chart and select it.
pub(crate) fn add_trendline(
    app: &mut ExcelApp,
    series: usize,
    kind: TrendKind,
    cx: &mut crate::features::Cx,
) {
    let Some(count) = app
        .selected_chart()
        .and_then(|c| c.series.get(series))
        .map(|s| s.trendlines.len())
    else {
        return;
    };
    app.edit_chart(
        t("Add Trendline", "Ajouter une courbe de tendance"),
        cx,
        move |c| {
            c.series[series].trendlines.push(Trendline::new(kind));
        },
    );
    app.select_chart_element(ChartElement::Trendline(series as u32, count as u32), cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::ast::RefExpr;
    use opensheet_engine::chart::{
        ChartKind, ErrorAmount, ErrorBars, ErrorDirection, Series, SeriesData,
    };
    use opensheet_engine::style::Color;

    fn chart(kind: ChartKind, series: usize) -> (Chart, ChartData) {
        let mut chart = Chart::new(kind);
        let mut data = ChartData {
            categories: vec!["North".into(), "South".into(), "East".into()],
            x_values: vec![Some(1.0), Some(2.0), Some(3.0)],
            value_format: "General".into(),
            x_format: "General".into(),
            ..Default::default()
        };
        for i in 0..series {
            chart.series.push(Series::new(RefExpr::cell(0, i as u32)));
            data.series.push(SeriesData {
                name: format!("S{}", i + 1),
                values: vec![Some(4.0 + i as f64), Some(7.0), Some(5.0)],
                color: Color(0x156082),
                x_values: vec![Some(1.0), Some(2.0), Some(3.0)],
                sizes: Vec::new(),
                format: "General".into(),
            });
        }
        chart.legend = LegendPos::Bottom;
        (chart, data)
    }

    #[test]
    fn axes_are_named_by_direction_and_role() {
        crate::i18n::set_lang(Lang::En);
        let (column, _) = chart(ChartKind::Column, 1);
        assert_eq!(
            axis_name(&column, AxisId::Category),
            "Horizontal (Category) Axis"
        );
        assert_eq!(axis_name(&column, AxisId::Value), "Vertical (Value) Axis");
        assert_eq!(
            axis_name(&column, AxisId::SecondaryValue),
            "Secondary Vertical (Value) Axis"
        );
        let (bar, _) = chart(ChartKind::Bar, 1);
        assert_eq!(
            axis_name(&bar, AxisId::Category),
            "Vertical (Category) Axis"
        );
        assert_eq!(axis_name(&bar, AxisId::Value), "Horizontal (Value) Axis");
        assert_eq!(axis_of(&bar, true, false), AxisId::Value);
        let (scatter, _) = chart(ChartKind::Scatter, 1);
        assert_eq!(
            axis_name(&scatter, AxisId::Category),
            "Horizontal (Value) Axis"
        );
    }

    #[test]
    fn the_list_names_what_the_chart_shows() {
        crate::i18n::set_lang(Lang::En);
        let (mut c, data) = chart(ChartKind::Column, 2);
        c.series[1]
            .trendlines
            .push(Trendline::new(TrendKind::Linear));
        c.series[0].error_bars.push(ErrorBars {
            horizontal: false,
            direction: ErrorDirection::Both,
            amount: ErrorAmount::Fixed(1.0),
            end_cap: true,
        });
        c.data_labels = true;
        let names: Vec<String> = listed(&c, &data)
            .into_iter()
            .map(|e| element_name(&c, &data, e))
            .collect();
        assert_eq!(
            names,
            [
                "Chart Area",
                "Chart Title",
                "Horizontal (Category) Axis",
                "Legend",
                "Plot Area",
                "Vertical (Value) Axis",
                "Vertical (Value) Axis Major Gridlines",
                "Series \"S1\"",
                "Series \"S1\" Data Labels",
                "Series \"S1\" Y Error Bars",
                "Series \"S2\"",
                "Series \"S2\" Data Labels",
                "Series \"S2\" Trendline 1",
            ]
        );
    }

    #[test]
    fn deleting_elements() {
        let (mut c, _) = chart(ChartKind::Column, 2);
        assert!(delete(&mut c, ChartElement::Title));
        assert_eq!(c.title, ChartTitle::None);
        assert!(delete(&mut c, ChartElement::MajorGridlines(AxisId::Value)));
        assert!(!c.axes.value.major_gridlines);
        assert!(delete(&mut c, ChartElement::LegendEntry(1)));
        assert_eq!(c.legend_hidden, vec![1]);
        // Labels of one series, then of the other: the chart has none left.
        c.data_labels = true;
        assert!(delete(&mut c, ChartElement::DataLabels(0)));
        assert!(c.labels_for(&c.series[0]).is_none() && c.labels_for(&c.series[1]).is_some());
        assert!(delete(&mut c, ChartElement::DataLabels(1)));
        assert!(!c.data_labels && c.series.iter().all(|s| s.labels.is_none()));
        assert!(delete(&mut c, ChartElement::Point(1, 2)));
        assert_eq!(c.series.len(), 1);
        assert!(!delete(&mut c, ChartElement::ChartArea));
        assert!(!exists(&c, ChartElement::Series(1)));
        assert!(!exists(&c, ChartElement::Title));
    }

    #[test]
    fn context_menus_follow_the_element() {
        use MenuItem::*;
        let (mut c, _) = chart(ChartKind::Column, 2);
        assert_eq!(
            menu_items(&c, ChartElement::ChartArea),
            [Reset, ChangeType, SelectData, MoveChart, Format]
        );
        assert_eq!(
            menu_items(&c, ChartElement::Series(0)),
            [
                Delete,
                Reset,
                ChangeType,
                SelectData,
                AddDataLabels,
                AddTrendline,
                Format
            ]
        );
        c.data_labels = true;
        assert!(menu_items(&c, ChartElement::Point(0, 1)).contains(&FormatLabels));
        assert_eq!(
            menu_items(&c, ChartElement::Axis(AxisId::Value)),
            [
                Delete,
                Reset,
                ChangeType,
                SelectData,
                AddMinorGridlines,
                Format
            ]
        );
        assert_eq!(
            menu_items(&c, ChartElement::Title),
            [Delete, Reset, EditText, ChangeType, SelectData, Format]
        );
        let (pie, _) = chart(ChartKind::Pie3D, 1);
        assert!(menu_items(&pie, ChartElement::Series(0)).contains(&Rotation));
        assert!(!menu_items(&pie, ChartElement::Series(0)).contains(&AddTrendline));
    }

    #[test]
    fn reset_gives_the_automatic_look_back() {
        let (mut c, _) = chart(ChartKind::Line, 1);
        c.series[0].color = Some(Color(0xFF0000));
        c.series[0].smooth = Some(true);
        c.series[0].point_mut(1).color = Some(Color(0x00FF00));
        reset(&mut c, ChartElement::Series(0));
        assert_eq!(c.series[0], Series::new(RefExpr::cell(0, 0)));
        c.axes.value.min = Some(2.0);
        c.axes.value.title = Some("Units".into());
        reset(&mut c, ChartElement::Axis(AxisId::Value));
        assert_eq!(c.axes.value.min, None);
        assert_eq!(c.axes.value.title.as_deref(), Some("Units"));
        assert!(c.axes.value.major_gridlines);
    }
}
