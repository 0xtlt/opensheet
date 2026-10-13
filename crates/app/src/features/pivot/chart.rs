//! PivotCharts: inserting a chart bound to a PivotTable, the field
//! buttons drawn over it, and the "PivotChart Analyze" tab.
//!
//! The chart is a chart like any other (`charts.rs` draws, moves and
//! formats it); the engine makes its series again whenever the report of
//! its PivotTable is written. The field buttons are drawn without their
//! drop-downs: a field is filtered from the PivotTable or with a slicer.

use super::{Cmd, active_pivot, perform};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent, Painter, chart_format, charts, object_id, slicers};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use crate::shell::Alert;
use crate::theme::c;
use gpui::{AnyElement, IntoElement, Styled, Window};
use opensheet_engine::chart::{Chart, ChartKind};
use opensheet_engine::pivot::{PivotChartFields, PivotTable, chart_kind_allowed};

pub(super) const TAB: &str = "pivot-chart-analyze";

/// The PivotTable the selected chart is bound to.
pub(crate) fn chart_pivot(app: &ExcelApp) -> Option<&PivotTable> {
    let chart = app.selected_chart()?;
    app.wb.pivot_of_chart(chart.id).map(|(_, p, _)| p)
}

/// The PivotTable the commands that take one work on: the one the active
/// cell is in, or the one of the selected PivotChart.
pub(crate) fn target_pivot(app: &ExcelApp) -> Option<&PivotTable> {
    chart_pivot(app).or_else(|| active_pivot(app))
}

fn refusal() -> &'static str {
    t(
        "The selected chart type is not supported for a PivotChart report. PivotChart reports cannot use XY (Scatter), Bubble, or Stock chart types.",
        "Le type de graphique sélectionné n'est pas pris en charge pour un rapport de graphique croisé dynamique. Les graphiques XY (nuage de points), en bulles et boursiers ne peuvent pas être appliqués à des rapports de graphique croisé dynamique.",
    )
}

/// True, with Excel's message shown, when the selected chart is a
/// PivotChart and `wanted` gives it (or one of its series) a type a
/// PivotChart cannot have.
pub(crate) fn refuses_chart_type(app: &mut ExcelApp, wanted: &Chart, cx: &mut Cx) -> bool {
    let kinds = std::iter::once(wanted.kind).chain(wanted.series.iter().filter_map(|s| s.kind));
    let refused = chart_pivot(app).is_some() && !kinds.into_iter().all(chart_kind_allowed);
    if refused {
        app.alert(Alert::new("OpenSheet", refusal()).warning(), cx);
    }
    refused
}

/// Insert a clustered column chart of the PivotTable the active cell is
/// in, in the middle of the window, and select it.
pub(super) fn insert(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app).map(|p| p.id) else {
        return;
    };
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    let sheet = app.sheet;
    let place = app.place_new_object(opensheet_engine::chart::DEFAULT_CHART_SIZE);
    match app.wb.add_pivot_chart(sheet, pivot, ChartKind::Column) {
        Ok(id) => {
            app.wb.update_chart(sheet, id, |chart| {
                chart.anchor = place.anchor;
                chart.offset = place.offset;
            });
            app.commit(t("Insert PivotChart", "Insérer un graphique croisé dynamique"), cx);
            app.select_object(Some(object_id(&charts::Charts, id)), cx);
            app.context_tab = Some(TAB);
        }
        Err(_) => app.wb.rollback(),
    }
}

/// Show or hide the field buttons of the selected PivotChart.
pub(super) fn toggle_buttons(app: &mut ExcelApp, cx: &mut Cx) {
    let Some((id, shown)) = app.selected_chart().and_then(|chart| Some((chart.id, app.wb.pivot_of_chart(chart.id)?.2.buttons))) else {
        return;
    };
    if app.may(Intent::EditObject(Some(object_id(&charts::Charts, id))), cx) {
        app.wb.set_pivot_chart_buttons(id, !shown);
        app.commit(t("Field Buttons", "Boutons de champ"), cx);
    }
}

pub(super) fn buttons_shown(app: &ExcelApp) -> bool {
    app.selected_chart().and_then(|chart| app.wb.pivot_of_chart(chart.id)).is_some_and(|(_, _, link)| link.buttons)
}

/// Refresh, or empty, the PivotTable of the selected PivotChart.
pub(super) fn refresh(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(id) = chart_pivot(app).map(|p| p.id) {
        perform(app, t("Refresh", "Actualiser"), window, cx, move |app, overwrite| app.wb.pivot_refresh(id, overwrite));
    }
}

pub(super) fn clear(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(id) = chart_pivot(app).map(|p| p.id) {
        super::update(app, id, t("Clear All", "Effacer tout"), window, cx, |p| p.clear());
    }
}

/// The groups of the "PivotChart Analyze" tab.
pub(super) fn tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Filter", "Filtrer"),
            row(vec![
                app.cmd_large("pc-slicer", slicers::Cmd::InsertSlicer, false, cx),
                app.cmd_large("pc-timeline", slicers::Cmd::InsertTimeline, false, cx),
                app.cmd_large("pc-connections", slicers::Cmd::FilterConnections, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Data", "Données"),
            row(vec![app.cmd_large("pc-refresh", Cmd::ChartRefresh, false, cx), app.cmd_large("pc-clear", Cmd::ChartClear, false, cx)]),
        )
        .into_any_element(),
        group(
            t("Actions", "Actions"),
            row(vec![app.cmd_large("pc-move", chart_format::Cmd::MoveChart, false, cx)]),
        )
        .into_any_element(),
        group(
            t("Show", "Afficher"),
            rows(vec![app.cmd_check("pc-buttons", Cmd::ChartFieldButtons, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// One field button: where it is and what it reads.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FieldButton {
    /// x0, y0, x1, y1 from the corner of the chart, pixels at 100 %.
    pub rect: (f32, f32, f32, f32),
    pub caption: String,
}

/// Where the field buttons of a PivotChart go, as Excel places them: the
/// filter fields at the top left with the value fields under them, the
/// legend fields at the top right, the axis fields at the bottom left.
pub(super) fn field_buttons(fields: &PivotChartFields, size: (f32, f32), width_of: &dyn Fn(&str) -> f32) -> Vec<FieldButton> {
    const H: f32 = 20.0;
    const GAP: f32 = 4.0;
    let mut out = Vec::new();
    let mut line = |captions: &[String], y: f32, from_right: bool| {
        let widths: Vec<f32> = captions.iter().map(|caption| width_of(caption) + 26.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32;
        let mut x = if from_right { (size.0 - GAP - total).max(GAP) } else { GAP };
        for (caption, w) in captions.iter().zip(widths) {
            out.push(FieldButton {
                rect: (x, y, (x + w).min(size.0 - 2.0), y + H),
                caption: caption.clone(),
            });
            x += w + GAP;
        }
    };
    line(&fields.filters, GAP, false);
    let below = if fields.filters.is_empty() { GAP } else { 2.0 * GAP + H };
    line(&fields.values, below, false);
    line(&fields.legend, GAP, true);
    line(&fields.axis, size.1 - GAP - H, false);
    out
}

/// Draw the field buttons over a PivotChart; called by `charts.rs` once
/// the chart itself is drawn in `rect`.
pub(crate) fn paint_buttons(p: &mut Painter, chart: &Chart, rect: (f32, f32, f32, f32)) {
    let Some((_, pivot, link)) = p.app.wb.pivot_of_chart(chart.id) else {
        return;
    };
    if !link.buttons {
        return;
    }
    let z = p.zoom();
    let font = 11.0 * z;
    let fields = pivot.chart_fields();
    let buttons = field_buttons(&fields, chart.size, &|caption| f32::from(p.shape(caption, 11.0, c(0), false).width));
    for button in buttons {
        let (x0, y0) = (rect.0 + button.rect.0 * z, rect.1 + button.rect.1 * z);
        let (x1, y1) = (rect.0 + button.rect.2 * z, rect.1 + button.rect.3 * z);
        p.rounded(x0, y0, x1, y1, 2.0, c(0xE7E6E6), Some(c(0xA6A6A6)));
        p.text(&button.caption, x0 + 6.0 * z, y0 + ((y1 - y0) - font * 1.25) / 2.0, font, c(0x262626));
        // The arrow of the drop-down, as on the buttons of the report.
        let (ax, ay) = (x1 - 10.0 * z, (y0 + y1) / 2.0);
        p.polygon(&[(ax - 3.5 * z, ay - 2.0 * z), (ax + 3.5 * z, ay - 2.0 * z), (ax, ay + 2.5 * z)], c(0x595959));
    }
}
