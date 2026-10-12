//! What the pivot commands do to the workbook. Each of them is one undo
//! step made through `perform`.

use super::logic::{self, MoveTo, SelectWhat, Target};
use super::{Outcome, active_cell, active_pivot, active_target, dialogs, perform, sheet_inserted, update};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::Alert;
use gpui::Window;
use opensheet_engine::pivot::{Aggregation, Area, PivotTable, ShowAs, SortOrder, Subtotals};

/// Tell the user something Excel says in a message box.
pub(super) fn warn(app: &mut ExcelApp, text: &'static str, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", text).warning(), cx);
}

// -------------------------------------------------------------------- names

pub(super) fn rename_pivot(app: &mut ExcelApp, name: String, window: &mut Window, cx: &mut Cx) {
    let Some(id) = active_pivot(app).map(|p| p.id) else {
        return;
    };
    perform(app, t("Rename PivotTable", "Renommer le tableau croisé dynamique"), window, cx, move |app, _| {
        app.wb.pivot_rename(id, &name)
    });
}

/// Would this caption be accepted for a field or a value field?
fn name_is_free(pivot: &PivotTable, target: Target, name: &str) -> bool {
    let mut probe = pivot.clone();
    match target {
        Target::Field(field) => probe.set_field_caption(field, name),
        Target::Data(data) => probe.set_data_name(data, name),
    }
}

/// Give a field or a value field a custom name. Returns false when the
/// name was refused (and the user told).
pub(super) fn rename_target(
    app: &mut ExcelApp,
    id: u32,
    target: Target,
    name: String,
    window: &mut Window,
    cx: &mut Cx,
) -> bool {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return false;
    };
    if !name_is_free(pivot, target, &name) {
        warn(app, logic::name_exists(), cx);
        return false;
    }
    update(app, id, t("Rename Field", "Renommer le champ"), window, cx, move |p| {
        match target {
            Target::Field(field) => p.set_field_caption(field, &name),
            Target::Data(data) => p.set_data_name(data, &name),
        };
    }) != Outcome::Refused
}

pub(super) fn rename_active_field(app: &mut ExcelApp, name: String, window: &mut Window, cx: &mut Cx) {
    if let (Some(id), Some(target)) = (active_pivot(app).map(|p| p.id), active_target(app)) {
        rename_target(app, id, target, name, window, cx);
    }
}

// ------------------------------------------------------- expand and collapse

/// The field "Expand Field" and "Collapse Field" act on.
pub(super) fn drill_field(app: &ExcelApp) -> Option<(u32, usize)> {
    let pivot = active_pivot(app)?;
    logic::drill_field(pivot, &active_cell(app)?).map(|field| (pivot.id, field))
}

pub(super) fn collapse_field(app: &mut ExcelApp, id: u32, field: usize, collapsed: bool, window: &mut Window, cx: &mut Cx) {
    let label = if collapsed {
        t("Collapse Entire Field", "Réduire le champ entier")
    } else {
        t("Expand Entire Field", "Développer le champ entier")
    };
    update(app, id, label, window, cx, move |p| p.set_field_collapsed(field, collapsed));
}

pub(super) fn collapse_active_field(app: &mut ExcelApp, collapsed: bool, window: &mut Window, cx: &mut Cx) {
    if let Some((id, field)) = drill_field(app) {
        collapse_field(app, id, field, collapsed, window, cx);
    }
}

/// Expand or collapse one item (its "+" / "-" button, a double click).
pub(super) fn collapse_item(
    app: &mut ExcelApp,
    id: u32,
    field: usize,
    item: u32,
    collapsed: bool,
    window: &mut Window,
    cx: &mut Cx,
) {
    let label = if collapsed { t("Collapse", "Réduire") } else { t("Expand", "Développer") };
    update(app, id, label, window, cx, move |p| p.set_collapsed(field, item, collapsed));
}

// ----------------------------------------------------------------- grouping

/// The field under the active cell that groups by interval or by period.
pub(super) fn group_field(app: &ExcelApp) -> Option<(u32, usize)> {
    let pivot = active_pivot(app)?;
    logic::range_group_field(pivot, &active_cell(app)?).map(|field| (pivot.id, field))
}

pub(super) fn can_group(app: &ExcelApp) -> bool {
    group_field(app).is_some()
        || logic::selected_items(&app.wb, app.sheet, &app.sel().ranges).is_some_and(|(_, _, items)| items.len() > 1)
}

/// "Group Selection": several labels become "Group1"; a date or a number
/// opens the Grouping dialog, as in Excel.
pub(super) fn group_selection(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if group_field(app).is_some() {
        open_group_field(app, window, cx);
        return;
    }
    match logic::selected_items(&app.wb, app.sheet, &app.sel().ranges) {
        Some((id, field, items)) if items.len() > 1 => {
            perform(app, t("Group Selection", "Grouper la sélection"), window, cx, move |app, overwrite| {
                app.wb.pivot_group_items(id, field, &items, overwrite).map(|_| ())
            });
        }
        _ => warn(app, t("Cannot group that selection.", "Impossible de grouper cette sélection."), cx),
    }
}

pub(super) fn open_group_field(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    match group_field(app) {
        Some((id, field)) => dialogs::open_grouping(app, id, field, window, cx),
        None => warn(app, t("Cannot group that selection.", "Impossible de grouper cette sélection."), cx),
    }
}

/// The grouped field under the active cell, and the group to dissolve.
fn ungroup_target(app: &ExcelApp) -> Option<(u32, usize, Option<u32>)> {
    let pivot = active_pivot(app)?;
    logic::ungroup_target(pivot, &active_cell(app)?).map(|(field, item)| (pivot.id, field, item))
}

pub(super) fn can_ungroup(app: &ExcelApp) -> bool {
    ungroup_target(app).is_some()
}

pub(super) fn ungroup(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some((id, field, item)) = ungroup_target(app) else {
        warn(
            app,
            t("Cannot ungroup that selection.", "Impossible de dissocier le groupe de cette sélection."),
            cx,
        );
        return;
    };
    perform(app, t("Ungroup", "Dissocier"), window, cx, move |app, overwrite| {
        app.wb.pivot_ungroup(id, field, item, overwrite)
    });
}

// --------------------------------------------------------------------- data

pub(super) fn refresh(app: &mut ExcelApp, all: bool, window: &mut Window, cx: &mut Cx) {
    let id = active_pivot(app).map(|p| p.id);
    perform(app, t("Refresh", "Actualiser"), window, cx, move |app, overwrite| match id {
        Some(id) if !all => app.wb.pivot_refresh(id, overwrite),
        _ => app.wb.pivot_refresh_all(overwrite),
    });
}

pub(super) fn clear_all(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    super::update_active(app, t("Clear All", "Effacer tout"), window, cx, |p| p.clear());
}

pub(super) fn clear_filters(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    super::update_active(app, t("Clear Filters", "Effacer les filtres"), window, cx, |p| {
        for field in 0..p.fields.len() {
            p.clear_filters(field);
        }
    });
}

pub(super) fn clear_field_filters(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("Clear Filter", "Effacer le filtre"), window, cx, move |p| p.clear_filters(field));
}

pub(super) fn select(app: &mut ExcelApp, what: SelectWhat, cx: &mut Cx) {
    if let Some(rect) = active_pivot(app).and_then(|p| logic::select_rect(p, what)) {
        app.select_rect(rect, cx);
    }
}

/// "Show Details": the records behind a value, listed as a table on a new
/// sheet placed before the PivotTable's.
pub(super) fn show_details(app: &mut ExcelApp, row: u32, col: u32, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let ts = window.text_system().clone();
    let done = perform(app, t("Show Details", "Afficher les détails"), window, cx, move |app, _| {
        let (target, rect) = app.wb.pivot_show_details(sheet, row, col)?;
        sheet_inserted(app, target);
        // Excel lists the records as a table, its columns fitted.
        let _ = app.wb.add_table(target, rect, true);
        for col in rect.c0..=rect.c1 {
            if let Some(width) = crate::grid::autofit_col_width(&app.wb, target, col, &app.fonts, &ts) {
                // Room for the filter button of the table header.
                app.wb.set_col_width(target, col, Some(width + 18.0));
            }
        }
        app.sheet = target;
        Ok(())
    });
    if done == Outcome::Done {
        app.select_cell(0, 0, cx);
    }
}

/// "Show Report Filter Pages": one sheet per item of a filter field.
pub(super) fn filter_pages(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    perform(
        app,
        t("Show Report Filter Pages", "Afficher les pages de filtre du rapport"),
        window,
        cx,
        move |app, _| {
            let sheets = app.wb.pivot_filter_pages(id, field)?;
            for index in sheets {
                sheet_inserted(app, index);
            }
            Ok(())
        },
    );
}

// ------------------------------------------------------------------- fields

pub(super) fn sort(
    app: &mut ExcelApp,
    id: u32,
    field: usize,
    order: SortOrder,
    by_value: Option<usize>,
    window: &mut Window,
    cx: &mut Cx,
) {
    update(app, id, t("Sort", "Trier"), window, cx, move |p| p.sort_field(field, order, by_value));
}

/// Take a field or a value field out of the report.
pub(super) fn remove(app: &mut ExcelApp, id: u32, target: Target, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("Remove Field", "Supprimer le champ"), window, cx, move |p| match target {
        Target::Field(field) => p.remove_field(field),
        Target::Data(data) => p.remove_data_field(data),
    });
}

pub(super) fn set_function(app: &mut ExcelApp, id: u32, data: usize, function: Aggregation, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    update(app, id, t("Summarize Values By", "Synthèse des valeurs par"), window, cx, move |p| {
        p.set_function(data, function, locale)
    });
}

pub(super) fn set_show_as(app: &mut ExcelApp, id: u32, data: usize, show: ShowAs, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("Show Values As", "Afficher les valeurs"), window, cx, move |p| {
        if let Some(d) = p.data_fields.get_mut(data) {
            d.show_as = show;
        }
    });
}

/// The "Subtotal "Field"" entry of the context menu: on (automatic) or off.
pub(super) fn toggle_subtotal(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("Subtotals", "Sous-totaux"), window, cx, move |p| {
        if let Some(f) = p.fields.get_mut(field) {
            f.subtotals = match f.subtotals {
                Subtotals::None => Subtotals::Automatic,
                _ => Subtotals::None,
            };
        }
    });
}

/// "Keep Only Selected Items" / "Hide Selected Items".
pub(super) fn filter_selected(app: &mut ExcelApp, keep: bool, window: &mut Window, cx: &mut Cx) {
    let Some((id, field, items)) = logic::selected_items(&app.wb, app.sheet, &app.sel().ranges) else {
        return;
    };
    let label = if keep {
        t("Keep Only Selected Items", "Conserver uniquement les éléments sélectionnés")
    } else {
        t("Hide Selected Items", "Masquer les éléments sélectionnés")
    };
    update(app, id, label, window, cx, move |p| {
        let Some(f) = p.fields.get(field) else {
            return;
        };
        let visible: Vec<u32> = f
            .items
            .iter()
            .filter(|it| !it.hidden && items.contains(&it.x) == keep)
            .map(|it| it.x)
            .collect();
        // Hiding everything would leave an empty report: Excel refuses.
        if !visible.is_empty() {
            p.set_visible_items(field, &visible);
        }
    });
}

/// Move an item among the items of its field (the field becomes sorted by
/// hand).
pub(super) fn move_item(app: &mut ExcelApp, id: u32, field: usize, item: u32, to: MoveTo, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("Move", "Déplacer"), window, cx, move |p| {
        // The order on screen becomes the manual order first.
        let shown = p.items_in_order(field);
        let Some(f) = p.fields.get_mut(field) else {
            return;
        };
        f.items.sort_by_key(|it| shown.iter().position(|x| *x == it.x).unwrap_or(usize::MAX));
        let visible: Vec<u32> = f.items.iter().filter(|it| !it.hidden).map(|it| it.x).collect();
        let Some(at) = visible.iter().position(|x| *x == item) else {
            return;
        };
        // Step over the visible items only: hidden ones stay where they are.
        let neighbour = visible[to.target(at, visible.len())];
        let position = f.items.iter().position(|it| it.x == neighbour).unwrap_or(0);
        p.move_item(field, item, position);
    });
}

/// Move a field inside its area.
pub(super) fn move_field(app: &mut ExcelApp, id: u32, field: usize, to: MoveTo, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    update(app, id, t("Move", "Déplacer"), window, cx, move |p| {
        let Some((area, index)) = logic::place_of(p, field) else {
            return;
        };
        let count = logic::area_entries(p, area, locale).len();
        p.move_entry(area, index, area, to.target(index, count), locale);
    });
}

/// Move a field to the end of another area.
pub(super) fn move_field_to(app: &mut ExcelApp, id: u32, field: usize, area: Area, window: &mut Window, cx: &mut Cx) {
    perform(app, t("Move", "Déplacer"), window, cx, move |app, overwrite| {
        app.wb.pivot_add_field(id, field, area, None, overwrite)
    });
}
