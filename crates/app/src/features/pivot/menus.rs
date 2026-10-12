//! The drop-downs of the "PivotTable Analyze" and "Design" tabs, and the
//! context menu of the cells of a PivotTable.

use super::logic::{self, Act, Item, SelectWhat, Target};
use super::{PANE, actions, active_pivot, create, dialogs, entry, filter, toggle_field_list, update, update_active};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, Cx, format_cells};
use crate::i18n::t;
use gpui::Window;
use opensheet_engine::Value;
use opensheet_engine::pivot::{FieldLayout, PivotTable, Subtotals};
use std::sync::Arc;

/// An entry that is listed as in Excel but that the engine cannot serve.
fn unavailable(label: &'static str) -> MenuEntry {
    entry(label, |_, _, _| {}).disabled(true)
}

/// Insert > PivotTable: where the data comes from.
pub(super) fn insert_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entries = vec![
        entry(t("From Table/Range", "À partir d'un tableau ou d'une plage"), create::open_create).icon("table"),
        unavailable(t("From External Data Source", "À partir d'une source de données externe")),
        unavailable(t("From Data Model", "À partir du modèle de données")),
    ];
    app.open_command_menu(entries, 280.0, window, cx);
}

pub(super) fn options_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let has_filters = active_pivot(app).is_some_and(|p| !p.page_fields.is_empty());
    let entries = vec![
        entry(t("Options...", "Options..."), dialogs::open_options),
        entry(
            t("Show Report Filter Pages...", "Afficher les pages de filtre du rapport..."),
            dialogs::open_filter_pages,
        )
        .disabled(!has_filters),
        unavailable(t("Generate GetPivotData", "Générer GetPivotData")),
    ];
    app.open_command_menu(entries, 290.0, window, cx);
}

pub(super) fn refresh_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entries = vec![
        entry(t("Refresh", "Actualiser"), |app, window, cx| actions::refresh(app, false, window, cx)).icon("refresh"),
        entry(t("Refresh All", "Actualiser tout"), |app, window, cx| actions::refresh(app, true, window, cx)),
    ];
    app.open_command_menu(entries, 200.0, window, cx);
}

pub(super) fn source_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entries = vec![
        entry(
            t("Change Data Source...", "Changer la source de données..."),
            create::open_change_source,
        )
        .icon("data"),
        unavailable(t("Connection Properties...", "Propriétés de connexion...")),
    ];
    app.open_command_menu(entries, 270.0, window, cx);
}

pub(super) fn clear_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let filtered = active_pivot(app).is_some_and(|p| (0..p.fields.len()).any(|field| p.is_filtered(field)));
    let entries = vec![
        entry(t("Clear All", "Effacer tout"), actions::clear_all),
        entry(t("Clear Filters", "Effacer les filtres"), actions::clear_filters).disabled(!filtered),
    ];
    app.open_command_menu(entries, 200.0, window, cx);
}

pub(super) fn select_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let has = |what: SelectWhat| active_pivot(app).is_some_and(|p| logic::select_rect(p, what).is_some());
    let pick = |label: &'static str, what: SelectWhat| {
        entry(label, move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| actions::select(app, what, cx))
    };
    let entries = vec![
        pick(t("Labels and Values", "Étiquettes et valeurs"), SelectWhat::Body),
        pick(t("Values", "Valeurs"), SelectWhat::Values).disabled(!has(SelectWhat::Values)),
        pick(t("Labels", "Étiquettes"), SelectWhat::Labels).disabled(!has(SelectWhat::Labels)),
        pick(t("Entire PivotTable", "Tableau croisé dynamique complet"), SelectWhat::Entire),
        MenuEntry::separator(),
        // Selecting parts of a PivotTable by pointing is always on.
        unavailable(t("Enable Selection", "Activer la sélection")).checked(true),
    ];
    app.open_command_menu(entries, 270.0, window, cx);
}

/// An entry of a Design menu: a change to the definition, ticked when it
/// describes the report as it is.
fn design(label: &'static str, ticked: bool, change: impl Fn(&mut PivotTable) + Clone + 'static) -> MenuEntry {
    entry(label, move |app, window, cx| {
        update_active(app, label, window, cx, change.clone());
    })
    .checked(ticked)
}

pub(super) fn subtotals_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(p) = active_pivot(app) else {
        return;
    };
    // What every field agrees on, if they do.
    let all = |test: fn(&opensheet_engine::pivot::PivotField) -> bool| p.fields.iter().all(test);
    let none = all(|f| f.subtotals == Subtotals::None);
    let bottom = all(|f| f.subtotals == Subtotals::Automatic && !f.subtotal_top);
    let top = all(|f| f.subtotals == Subtotals::Automatic && f.subtotal_top);
    let entries = vec![
        design(
            t("Do Not Show Subtotals", "Ne pas afficher les sous-totaux"),
            none,
            |p| p.set_subtotals(false, true),
        ),
        design(
            t("Show all Subtotals at Bottom of Group", "Afficher tous les sous-totaux au bas du groupe"),
            bottom,
            |p| p.set_subtotals(true, false),
        ),
        design(
            t("Show all Subtotals at Top of Group", "Afficher tous les sous-totaux en haut du groupe"),
            top,
            |p| p.set_subtotals(true, true),
        ),
        MenuEntry::separator(),
        unavailable(t("Include Filtered Items in Totals", "Inclure les éléments filtrés dans les totaux")),
    ];
    app.open_command_menu(entries, 330.0, window, cx);
}

pub(super) fn grand_totals_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(p) = active_pivot(app) else {
        return;
    };
    let now = (p.options.row_grand_totals, p.options.col_grand_totals);
    let totals = |label: &'static str, rows: bool, cols: bool| {
        design(label, now == (rows, cols), move |p| {
            p.options.row_grand_totals = rows;
            p.options.col_grand_totals = cols;
        })
    };
    let entries = vec![
        totals(t("Off for Rows and Columns", "Désactivé pour les lignes et les colonnes"), false, false),
        totals(t("On for Rows and Columns", "Activé pour les lignes et les colonnes"), true, true),
        totals(t("On for Rows Only", "Activé pour les lignes uniquement"), true, false),
        totals(t("On for Columns Only", "Activé pour les colonnes uniquement"), false, true),
    ];
    app.open_command_menu(entries, 300.0, window, cx);
}

pub(super) fn layout_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(p) = active_pivot(app) else {
        return;
    };
    let form = logic::report_layout(p);
    let repeated = p.fields.iter().all(|f| f.repeat_labels);
    let layout = |label: &'static str, layout: FieldLayout| {
        design(label, form == Some(layout), move |p| p.set_report_layout(layout))
    };
    let entries = vec![
        layout(t("Show in Compact Form", "Afficher sous forme compactée"), FieldLayout::Compact),
        layout(t("Show in Outline Form", "Afficher sous forme de plan"), FieldLayout::Outline),
        layout(t("Show in Tabular Form", "Afficher sous forme tabulaire"), FieldLayout::Tabular),
        MenuEntry::separator(),
        design(
            t("Repeat All Item Labels", "Répéter toutes les étiquettes d’éléments"),
            repeated,
            |p| p.set_repeat_labels(true),
        ),
        design(
            t("Do Not Repeat Item Labels", "Ne pas répéter les étiquettes d’éléments"),
            !repeated,
            |p| p.set_repeat_labels(false),
        ),
    ];
    app.open_command_menu(entries, 300.0, window, cx);
}

pub(super) fn blank_rows_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entries = vec![
        design(
            t("Insert Blank Line after Each Item", "Insérer une ligne vide après chaque élément"),
            false,
            |p| p.set_blank_lines(true),
        ),
        design(
            t("Remove Blank Line after Each Item", "Supprimer la ligne vide après chaque élément"),
            false,
            |p| p.set_blank_lines(false),
        ),
    ];
    app.open_command_menu(entries, 320.0, window, cx);
}

// --------------------------------------------------------- the context menu

/// Excel's context menu for the cell of a PivotTable at (`row`, `col`).
pub(super) fn context_entries(app: &ExcelApp, row: u32, col: u32) -> Vec<MenuEntry> {
    let Some(id) = app.wb.pivot_at(app.sheet, row, col).map(|p| p.id) else {
        return Vec::new();
    };
    let plan = logic::context_menu(&app.wb, app.sheet, row, col, &app.sel().ranges, app.pane_open(PANE));
    entries_of(&plan, id, row, col)
}

fn entries_of(items: &[Item], id: u32, row: u32, col: u32) -> Vec<MenuEntry> {
    items
        .iter()
        .map(|item| {
            if item.separator {
                return MenuEntry::separator();
            }
            if !item.children.is_empty() {
                return MenuEntry::submenu(item.label.clone(), entries_of(&item.children, id, row, col))
                    .disabled(!item.enabled);
            }
            let act = item.act;
            let menu_entry = entry(item.label.clone(), move |app, window, cx| {
                if let Some(act) = act {
                    run(app, id, row, col, act, window, cx);
                }
            })
            .checked(item.checked)
            .disabled(!item.enabled);
            match act {
                Some(Act::Copy) => menu_entry.icon("copy").shortcut("⌘C"),
                Some(Act::FormatCells) => menu_entry.icon("format-cells").shortcut("⌘1"),
                Some(Act::Refresh) => menu_entry.icon("refresh"),
                _ => menu_entry,
            }
        })
        .collect()
}

/// Do what an entry of the context menu stands for.
fn run(app: &mut ExcelApp, id: u32, row: u32, col: u32, act: Act, window: &mut Window, cx: &mut Cx) {
    match act {
        Act::Copy => app.copy(false, cx),
        Act::FormatCells => format_cells::Cmd::Open.run(app, window, cx),
        Act::NumberFormat(data) => number_format(app, id, data, row, col, window, cx),
        Act::Refresh => actions::refresh(app, false, window, cx),
        Act::Sort { field, order, by_value } => actions::sort(app, id, field, order, by_value, window, cx),
        Act::SortOptions(field) => filter::open_sort(app, id, field, window, cx),
        Act::ClearFilter(field) => actions::clear_field_filters(app, id, field, window, cx),
        Act::KeepSelected => actions::filter_selected(app, true, window, cx),
        Act::HideSelected => actions::filter_selected(app, false, window, cx),
        Act::Top10(field) => filter::open_top10(app, id, field, window, cx),
        Act::LabelFilters(field) => filter::open_label_filter(app, id, field, None, window, cx),
        Act::ValueFilters(field) => filter::open_value_filter(app, id, field, None, window, cx),
        Act::ToggleSubtotal(field) => actions::toggle_subtotal(app, id, field, window, cx),
        Act::CollapseItem { field, item, collapsed } => {
            actions::collapse_item(app, id, field, item, collapsed, window, cx)
        }
        Act::CollapseField { field, collapsed } => actions::collapse_field(app, id, field, collapsed, window, cx),
        Act::Group => actions::group_selection(app, window, cx),
        Act::Ungroup => actions::ungroup(app, window, cx),
        Act::MoveItem { field, item, to } => actions::move_item(app, id, field, item, to, window, cx),
        Act::MoveField { field, to } => actions::move_field(app, id, field, to, window, cx),
        Act::MoveFieldTo { field, area } => actions::move_field_to(app, id, field, area, window, cx),
        Act::Remove(target) => actions::remove(app, id, target, window, cx),
        Act::Function { data, function } => actions::set_function(app, id, data, function, window, cx),
        Act::ShowAs { data, kind } => match kind.build(None, None) {
            Some(show) => actions::set_show_as(app, id, data, show, window, cx),
            // The calculation needs a base field: the dialog asks for it.
            None => dialogs::open_show_values_as(app, id, data, kind, window, cx),
        },
        Act::ShowDetails => actions::show_details(app, row, col, window, cx),
        Act::FieldSettings(field) => dialogs::open_field_settings(app, id, field, window, cx),
        Act::ValueSettings { data, tab } => dialogs::open_value_settings(app, id, data, tab, window, cx),
        Act::RemoveGrandTotal => {
            // The total of the row or of the column the label heads.
            let rows = app
                .wb
                .pivot_cell(app.sheet, row, col)
                .is_some_and(|cell| cell.part == opensheet_engine::pivot::PivotPart::ColumnGrandTotalLabel);
            update(app, id, t("Remove Grand Total", "Supprimer le total général"), window, cx, move |p| {
                if rows {
                    p.options.row_grand_totals = false;
                } else {
                    p.options.col_grand_totals = false;
                }
            });
        }
        Act::Options => dialogs::open_options(app, window, cx),
        Act::FieldList => toggle_field_list(app, cx),
    }
}

/// "Number Format...": the Number tab of Format Cells, for a value field.
fn number_format(app: &mut ExcelApp, id: u32, data: usize, row: u32, col: u32, window: &mut Window, cx: &mut Cx) {
    let Some(d) = app.wb.pivot(id).and_then(|(_, p)| p.data_fields.get(data)) else {
        return;
    };
    let code = d.num_fmt.clone().unwrap_or_else(|| Arc::from("General"));
    let sample = Some(app.wb.value_at(app.sheet, row, col))
        .filter(Value::is_number)
        .unwrap_or(Value::Number(1234.5));
    let weak = cx.weak_entity();
    format_cells::open_number_format(app, &code, sample, window, cx, move |code, window, cx| {
        let code: Arc<str> = Arc::from(code);
        weak.update(cx, |app, cx| {
            update(app, id, t("Number Format", "Format de nombre"), window, cx, move |p| {
                if let Some(d) = p.data_fields.get_mut(data) {
                    d.num_fmt = Some(code.clone());
                }
            });
        })
        .ok();
    });
}

/// The settings of a field or of a value field, whichever `target` is.
pub(super) fn open_target_settings(app: &mut ExcelApp, id: u32, target: Target, window: &mut Window, cx: &mut Cx) {
    match target {
        Target::Field(field) => dialogs::open_field_settings(app, id, field, window, cx),
        Target::Data(data) => dialogs::open_value_settings(app, id, data, 0, window, cx),
    }
}
