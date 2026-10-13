//! PivotCharts: inserting a chart bound to a PivotTable, the field
//! buttons drawn over it, and the "PivotChart Analyze" tab.
//!
//! The chart is a chart like any other (`charts.rs` draws, moves and
//! formats it); the engine makes its series again whenever the report of
//! its PivotTable is written. A field button opens the drop-down the
//! header of its field opens in the report (`filter.rs`), a value field
//! button the menu its entry has in the field list; both act on the
//! PivotTable, and the chart follows.

use super::filter::{self, FilterButton};
use super::{Cmd, Pivots, active_pivot, entry, open_menu_at, pane, perform};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Cx, Intent, Painter, Zone, ZoneEvent, ZoneResponse, chart_format, charts, index_of, object_id, slicers};
use crate::i18n::t;
use crate::ribbon::{group, row};
use crate::shell::Alert;
use crate::theme::c;
use gpui::{AnyElement, IntoElement, MouseButton, Styled, Window};
use opensheet_engine::chart::{Chart, ChartKind};
use opensheet_engine::pivot::{Area, ChartButtonKind, PivotChartFields, PivotTable, chart_kind_allowed};

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

/// The selected PivotChart and the kinds of field buttons it shows.
fn selected_link(app: &ExcelApp) -> Option<(u32, [bool; 4])> {
    let chart = app.selected_chart()?;
    Some((chart.id, app.wb.pivot_of_chart(chart.id)?.2.shown))
}

/// Show or hide field buttons of the selected PivotChart: one kind, or
/// all of them (`None`).
pub(super) fn show_buttons(app: &mut ExcelApp, kind: Option<ChartButtonKind>, shown: bool, cx: &mut Cx) {
    let Some((id, _)) = selected_link(app) else {
        return;
    };
    if app.may(Intent::EditObject(Some(object_id(&charts::Charts, id))), cx) {
        match kind {
            Some(kind) => app.wb.set_pivot_chart_button_kind(id, kind, shown),
            None => app.wb.set_pivot_chart_buttons(id, shown),
        };
        app.commit(t("Field Buttons", "Boutons de champ"), cx);
    }
}

pub(super) fn buttons_shown(app: &ExcelApp) -> bool {
    selected_link(app).is_some_and(|(_, shown)| shown.contains(&true))
}

/// What the "Field Buttons" menu lists: each kind with its tick, then
/// "Hide All".
pub(super) fn buttons_menu_entries(app: &ExcelApp) -> Vec<MenuEntry> {
    let Some((_, shown)) = selected_link(app) else {
        return Vec::new();
    };
    let labels = [
        (ChartButtonKind::Filter, t("Show Report Filter Field Buttons", "Afficher les boutons de champ Filtre de rapport")),
        (ChartButtonKind::Legend, t("Show Legend Field Buttons", "Afficher les boutons du champ Légende")),
        (ChartButtonKind::Axis, t("Show Axis Field Buttons", "Afficher les boutons du champ Axe")),
        (ChartButtonKind::Value, t("Show Value Field Buttons", "Afficher les boutons du champ Valeur")),
    ];
    let mut entries: Vec<MenuEntry> = labels
        .into_iter()
        .map(|(kind, label)| {
            let on = shown[kind as usize];
            entry(label, move |app, _, cx| show_buttons(app, Some(kind), !on, cx)).checked(on)
        })
        .collect();
    entries.push(MenuEntry::separator());
    entries.push(entry(t("Hide All", "Masquer tout"), |app, _, cx| show_buttons(app, None, false, cx)).disabled(!shown.contains(&true)));
    entries
}

pub(super) fn buttons_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entries = buttons_menu_entries(app);
    app.open_command_menu(entries, 330.0, window, cx);
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
            row(vec![app.cmd_large("pc-buttons", Cmd::ChartFieldButtons, true, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// One field button: where it is, what it reads and what it stands for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FieldButton {
    /// x0, y0, x1, y1 from the corner of the chart, pixels at 100 %.
    pub rect: (f32, f32, f32, f32),
    pub caption: String,
    pub kind: ChartButtonKind,
    /// The field; for a value field button, its rank among the values.
    pub index: usize,
}

/// Where the field buttons of a PivotChart go, as Excel places them: the
/// filter fields at the top left with the value fields under them, the
/// legend fields at the top right, the axis fields at the bottom left.
/// `shown`: the kinds that are not hidden.
pub(super) fn field_buttons(fields: &PivotChartFields, shown: [bool; 4], size: (f32, f32), width_of: &dyn Fn(&str) -> f32) -> Vec<FieldButton> {
    const H: f32 = 20.0;
    const GAP: f32 = 4.0;
    let mut out = Vec::new();
    let mut line = |kind: ChartButtonKind, y: f32, from_right: bool| -> bool {
        let list = if shown[kind as usize] { fields.of(kind) } else { &[] };
        // A value field button has no drop-down arrow.
        let arrow = if kind == ChartButtonKind::Value { 12.0 } else { 26.0 };
        let widths: Vec<f32> = list.iter().map(|(_, caption)| width_of(caption) + arrow).collect();
        let total: f32 = widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32;
        let mut x = if from_right { (size.0 - GAP - total).max(GAP) } else { GAP };
        for ((index, caption), w) in list.iter().zip(widths) {
            out.push(FieldButton {
                rect: (x, y, (x + w).min(size.0 - 2.0), y + H),
                caption: caption.clone(),
                kind,
                index: *index,
            });
            x += w + GAP;
        }
        !list.is_empty()
    };
    let below = if line(ChartButtonKind::Filter, GAP, false) { 2.0 * GAP + H } else { GAP };
    line(ChartButtonKind::Value, below, false);
    line(ChartButtonKind::Legend, GAP, true);
    line(ChartButtonKind::Axis, size.1 - GAP - H, false);
    out
}

/// What the zone over a field button carries.
struct ButtonZone {
    chart: u32,
    pivot: u32,
    kind: ChartButtonKind,
    index: usize,
    /// Its rank among the buttons of the chart.
    rank: usize,
}

#[cfg(test)]
pub(super) fn is_button_zone(zone: &Zone) -> bool {
    zone.payload::<ButtonZone>().is_some()
}

/// Draw the field buttons over a PivotChart and make them answer the
/// mouse; called by `charts.rs` once the chart itself is drawn in `rect`
/// and has its own zone (the buttons lie over it).
pub(crate) fn paint_buttons(p: &mut Painter, chart: &Chart, rect: (f32, f32, f32, f32)) {
    let Some((_, pivot, link)) = p.app.wb.pivot_of_chart(chart.id) else {
        return;
    };
    if !link.buttons() {
        return;
    }
    let z = p.zoom();
    let font = 11.0 * z;
    let fields = pivot.chart_fields();
    let buttons = field_buttons(&fields, link.shown, chart.size, &|caption| f32::from(p.shape(caption, 11.0, c(0), false).width));
    // The zones belong to this feature, whoever owns the chart.
    let owner = std::mem::replace(&mut p.owner, index_of(&Pivots));
    for (rank, button) in buttons.into_iter().enumerate() {
        let (x0, y0) = (rect.0 + button.rect.0 * z, rect.1 + button.rect.1 * z);
        let (x1, y1) = (rect.0 + button.rect.2 * z, rect.1 + button.rect.3 * z);
        p.rounded(x0, y0, x1, y1, 2.0, c(0xE7E6E6), Some(c(0xA6A6A6)));
        p.text(&button.caption, x0 + 6.0 * z, y0 + ((y1 - y0) - font * 1.25) / 2.0, font, c(0x262626));
        if button.kind != ChartButtonKind::Value {
            // The arrow of the drop-down, as on the buttons of the report.
            let (ax, ay) = (x1 - 10.0 * z, (y0 + y1) / 2.0);
            p.polygon(&[(ax - 3.5 * z, ay - 2.0 * z), (ax + 3.5 * z, ay - 2.0 * z), (ax, ay + 2.5 * z)], c(0x595959));
        }
        p.zone(
            (x0, y0, x1, y1),
            ButtonZone {
                chart: chart.id,
                pivot: pivot.id,
                kind: button.kind,
                index: button.index,
                rank,
            },
        )
        .cursor(gpui::CursorStyle::Arrow);
    }
    p.owner = owner;
}

/// A press on a field button: the chart is selected and the button opens
/// its list — the drop-down of the field, or the menu of the value field.
/// `None` when the zone is not a field button.
pub(super) fn button_mouse_down(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> Option<ZoneResponse> {
    let hit = zone.payload::<ButtonZone>()?;
    // The other buttons select the chart and open its menu.
    if ev.button != MouseButton::Left {
        return Some(ZoneResponse::Ignored);
    }
    if app.editing.is_some() && !app.commit_edit(None, window, cx) {
        return Some(ZoneResponse::Handled);
    }
    let object = object_id(&charts::Charts, hit.chart);
    if !app.is_object_selected(object) {
        app.select_object(Some(object), cx);
    }
    // Under the button, like the drop-down of a cell.
    let anchor = match app.current_layout() {
        Some(layout) => layout.to_window(zone.bounds.0, zone.bounds.3),
        None => ev.position,
    };
    match hit.kind {
        ChartButtonKind::Value => {
            if app.may(Intent::UsePivotTable, cx) {
                let entries = pane::entry_menu_entries(app, hit.pivot, Area::Values, hit.index, false);
                open_menu_at(app, anchor, entries, cx);
            }
        }
        _ => {
            let button = FilterButton::Chart(hit.chart, hit.rank);
            filter::toggle_drop_down_at(app, hit.pivot, &[hit.index], button, anchor, Some(ev.position), window, cx);
        }
    }
    Some(ZoneResponse::Handled)
}
