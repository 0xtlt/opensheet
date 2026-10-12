//! Data ▸ What-If Analysis: Goal Seek and its status dialog, the Scenario
//! Manager with its dialogs (Add / Edit Scenario, Scenario Values,
//! Scenario Summary) and Data Table. The engine side is `whatif.rs`.

use super::{typed_range, warn};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ListBox, RadioGroup, RefEdit, ScrollArea, TextField, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::input::parse_numeric_text;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::cell_name;
use opensheet_engine::whatif::{
    DataTableError, GoalSeek, GoalSeekError, MAX_SCENARIO_CELLS, Scenario, ScenarioCell, ScenarioError, SummaryLabels,
};
use opensheet_engine::{Rect, Value};

/// `$A$1`.
fn absolute(row: u32, col: u32) -> String {
    RefStyle::absolute().format(Rect::cell(row, col), "")
}

/// The single cell of the current sheet a reference typed in a dialog
/// names.
fn typed_cell(app: &ExcelApp, text: &str) -> Option<(u32, u32)> {
    match typed_range(app, text) {
        Some((sheet, rect)) if sheet == app.sheet && rect.is_single() => Some((rect.r0, rect.c0)),
        _ => None,
    }
}

/// A control with its label in a column of fixed width.
fn field(text: &'static str, width: f32, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(width)).flex_none().child(label(text)))
        .child(control)
}

// ---------------------------------------------------------------- Goal Seek

fn goal_seek_label() -> &'static str {
    t("Goal Seek", "Valeur cible")
}

/// Data ▸ What-If Analysis ▸ Goal Seek: "Set cell" is the active cell.
pub(super) fn open_goal_seek(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<GoalSeekDialog> {
    let (row, col) = app.active();
    let set = absolute(row, col);
    app.open_modal(window, cx, move |modal, _, cx| GoalSeekDialog {
        set: modal.ref_input(RefStyle::absolute(), set, cx),
        value: widgets::input(cx, ""),
        changing: modal.ref_input(RefStyle::absolute(), "", cx),
        modal,
    })
}

/// Run Goal Seek as the dialog asks. `None` when it cannot start: the
/// user has been told why and the dialog stays open. The result is left
/// uncommitted for the status dialog to keep or take back.
fn seek(app: &mut ExcelApp, set: &str, value: &str, changing: &str, cx: &mut Cx) -> Option<(String, GoalSeek)> {
    let sheet = app.sheet;
    let (Some(set), Some(changing)) = (typed_cell(app, set), typed_cell(app, changing)) else {
        warn(app, t("Reference isn't valid.", "La référence n’est pas valide."), cx);
        return None;
    };
    let Some(target) = parse_numeric_text(value.trim(), app.wb.locale) else {
        warn(app, t("Your entry cannot be used. An integer or decimal number may be required.", "Impossible d'utiliser votre saisie. Un nombre entier ou un nombre décimal est peut-être requis."), cx);
        return None;
    };
    let cell = Rect::cell(changing.0, changing.1);
    if app.wb.splits_data_table(sheet, cell) || app.wb.data_table_at(sheet, changing.0, changing.1).is_some() {
        warn(app, part_of_table(), cx);
        return None;
    }
    if !app.may(Intent::EditCells(cell), cx) {
        return None;
    }
    match app.wb.goal_seek((sheet, set.0, set.1), target, (sheet, changing.0, changing.1)) {
        Ok(result) => {
            app.refresh(cx);
            Some((cell_name(set.0, set.1), result))
        }
        Err(error) => {
            let message = match error {
                GoalSeekError::NotFormula => t("Cell must contain a formula.", "La cellule doit contenir une formule."),
                GoalSeekError::NotValue => t("Cell must contain a value.", "La cellule doit contenir une valeur."),
            };
            warn(app, message, cx);
            None
        }
    }
}

pub(super) struct GoalSeekDialog {
    modal: Modal,
    set: Entity<TextInput>,
    value: Entity<TextInput>,
    changing: Entity<TextInput>,
}

impl GoalSeekDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, set: &str, value: &str, changing: &str, cx: &mut Context<Self>) {
        for (field, text) in [(&self.set, set), (&self.value, value), (&self.changing, changing)] {
            field.update(cx, |field, cx| field.set_text(text, cx));
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let [set, value, changing] = [&self.set, &self.value, &self.changing].map(|field| field.read(cx).text().to_string());
        let found = self.modal.update_app(cx, |app, cx| seek(app, &set, &value, &changing, cx)).flatten();
        let Some((cell, result)) = found else {
            return;
        };
        // The status dialog takes the place of this one.
        self.modal.close(window, cx);
        let text = self.modal.read_app(cx, |app| {
            let (row, col) = typed_cell(app, &set).unwrap_or(app.active());
            app.wb.display_text(app.sheet, row, col).text
        });
        let current = text.unwrap_or_default();
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, _| GoalSeekStatus {
                modal,
                cell,
                result,
                current,
            });
        });
    }
}

impl Render for GoalSeekDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, goal_seek_label())
            .width(330.0)
            .on_ok(cx.listener(Self::ok))
            .child(field(t("Set cell:", "Cellule à définir :"), 130.0, RefEdit::new(&self.set, &self.modal).width(140.0)))
            .child(field(t("To value:", "Valeur à atteindre :"), 130.0, TextField::new(&self.value).width(140.0)))
            .child(field(
                t("By changing cell:", "Cellule à modifier :"),
                130.0,
                RefEdit::new(&self.changing, &self.modal).width(140.0),
            ))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// "Goal Seek Status": what was found, kept by OK and taken back by
/// Cancel.
struct GoalSeekStatus {
    modal: Modal,
    /// The name of the "Set cell".
    cell: String,
    result: GoalSeek,
    /// What the cell shows now.
    current: String,
}

impl GoalSeekStatus {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| app.commit(goal_seek_label(), cx));
        self.modal.close(window, cx);
    }

    fn cancel(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| {
            app.wb.goal_seek_restore();
            app.refresh(cx);
        });
        self.modal.close(window, cx);
    }
}

impl Render for GoalSeekStatus {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let with_cell = t("Goal Seeking with Cell |", "Recherche sur la cellule |").replace('|', &self.cell);
        let outcome = if self.result.found {
            t("found a solution.", "a trouvé une solution.")
        } else {
            t("may not have found a solution.", "n'a pas trouvé de solution.")
        };
        let decimal_sep = self.modal.read_app(cx, |app| app.wb.locale.decimal_sep).unwrap_or('.');
        let target = widgets::number_text(self.result.target, decimal_sep);
        dialog(&self.modal, t("Goal Seek Status", "État de la recherche"))
            .width(330.0)
            .on_ok(cx.listener(Self::ok))
            .on_cancel(cx.listener(Self::cancel))
            .child(div().child(with_cell))
            .child(div().child(outcome))
            .child(field(t("Target value:", "Valeur cible :"), 120.0, label(target)).pt(px(6.)))
            .child(field(t("Current value:", "Valeur actuelle :"), 120.0, label(self.current.clone())))
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("status-ok", t("OK", "OK")).primary().on_click(cx.listener(Self::ok)))
                    .child(Button::new("status-cancel", t("Cancel", "Annuler")).on_click(cx.listener(Self::cancel))),
            )
    }
}

// --------------------------------------------------------------- Data Table

fn part_of_table() -> &'static str {
    t("Can't change part of a data table.", "Impossible de modifier une partie d'une table de données.")
}

/// The message for a change that would cut into a data table, when
/// `rect` does.
pub(super) fn table_guard(app: &ExcelApp, rect: Rect) -> Option<SharedString> {
    app.wb.splits_data_table(app.sheet, rect).then(|| part_of_table().into())
}

/// Data ▸ What-If Analysis ▸ Data Table, on the selection.
pub(super) fn open_data_table(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<DataTableDialog> {
    app.open_modal(window, cx, move |modal, _, cx| DataTableDialog {
        row: modal.ref_input(RefStyle::absolute(), "", cx),
        column: modal.ref_input(RefStyle::absolute(), "", cx),
        modal,
    })
}

/// Make the selection a data table. False when it cannot be done; the
/// user has been told why and the dialog stays open.
pub(super) fn create_table(app: &mut ExcelApp, row: &str, column: &str, cx: &mut Cx) -> bool {
    let sheet = app.sheet;
    let selection = app.sel().current();
    let mut inputs = [None, None];
    for (slot, text) in inputs.iter_mut().zip([row, column]) {
        if text.trim().is_empty() {
            continue;
        }
        *slot = typed_cell(app, text);
        if slot.is_none() {
            warn(app, t("Input cell reference is not valid.", "Référence de cellule d'entrée non valide."), cx);
            return false;
        }
    }
    let results = Rect::new(selection.r0 + 1, selection.c0 + 1, selection.r1.max(selection.r0 + 1), selection.c1.max(selection.c0 + 1));
    if !app.may(Intent::EditCells(results), cx) {
        return false;
    }
    match app.wb.data_table_create(sheet, selection, inputs[0], inputs[1]) {
        Ok(_) => {
            app.commit(t("Data Table", "Table de données"), cx);
            true
        }
        Err(error) => {
            app.wb.rollback();
            let message = match error {
                DataTableError::BadSelection => t("Selection is not valid.", "Sélection non valide."),
                DataTableError::NoInput | DataTableError::InputInside => {
                    t("Input cell reference is not valid.", "Référence de cellule d'entrée non valide.")
                }
                DataTableError::Overlap => part_of_table(),
            };
            warn(app, message, cx);
            false
        }
    }
}

pub(super) struct DataTableDialog {
    modal: Modal,
    row: Entity<TextInput>,
    column: Entity<TextInput>,
}

impl DataTableDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, row: &str, column: &str, cx: &mut Context<Self>) {
        self.row.update(cx, |field, cx| field.set_text(row, cx));
        self.column.update(cx, |field, cx| field.set_text(column, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let row = self.row.read(cx).text().to_string();
        let column = self.column.read(cx).text().to_string();
        if self.modal.update_app(cx, |app, cx| create_table(app, &row, &column, cx)) == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for DataTableDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Data Table", "Table de données"))
            .width(370.0)
            .on_ok(cx.listener(Self::ok))
            .child(field(
                t("Row input cell:", "Cellule d’entrée en ligne :"),
                190.0,
                RefEdit::new(&self.row, &self.modal).width(130.0),
            ))
            .child(field(
                t("Column input cell:", "Cellule d’entrée en colonne :"),
                190.0,
                RefEdit::new(&self.column, &self.modal).width(130.0),
            ))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------- Scenario Manager

/// The name scenarios are signed with.
fn user_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

/// "Created by Ann on 10/12/2026", as Excel fills the comment in.
fn signed(prefix: &str, app: &ExcelApp) -> String {
    let locale = app.wb.locale;
    let today = opensheet_engine::datetime::now_serial().floor();
    let date = opensheet_engine::format::format_value(&Value::Number(today), locale.short_date_format, locale).text;
    format!("{prefix} {} {} {date}", user_name(), t("on", "le"))
}

/// Why the scenarios of the current sheet cannot be changed, if its
/// protection forbids it.
fn scenarios_locked(app: &ExcelApp) -> Option<String> {
    app.wb.permits(&Action::EditScenarios { sheet: app.sheet }).err().map(|denied| denied.message(app.wb.locale))
}

fn scenario_error(error: ScenarioError) -> &'static str {
    match error {
        ScenarioError::NoName => t("Scenario names must be unique.", "Les noms des scénarios doivent être uniques."),
        ScenarioError::Duplicate => t("Scenario names must be unique.", "Les noms des scénarios doivent être uniques."),
        ScenarioError::NoCells | ScenarioError::Missing => t("Changing Cells are required.", "Des cellules variables sont nécessaires."),
        ScenarioError::TooManyCells => t(
            "Too many changing cells.  A scenario can have no more than 32 changing cells.",
            "Trop de cellules variables sont définies. Un scénario ne peut pas avoir plus de 32 cellules variables.",
        ),
    }
}

/// The cells a "Changing cells" or "Result cells" box names on the
/// current sheet, row by row; `None` when the text is not a reference.
fn typed_cells(app: &ExcelApp, text: &str) -> Option<Vec<(u32, u32)>> {
    let (row, col) = app.active();
    // Several areas are separated the way the arguments of a function are.
    let union = if text.contains([',', ';']) && !text.trim_start().starts_with('(') { format!("({text})") } else { text.to_string() };
    let areas = app.wb.reference_areas(app.sheet, row, col, &union)?;
    let mut cells = Vec::new();
    for area in areas {
        if area.sheet != app.sheet {
            return None;
        }
        let rect = app.wb.clamp_to_used(app.sheet, area.rect);
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                if !cells.contains(&(r, c)) {
                    cells.push((r, c));
                }
                if cells.len() > MAX_SCENARIO_CELLS + 1 {
                    return Some(cells);
                }
            }
        }
    }
    Some(cells)
}

/// The changing cells of a scenario as the manager lists them:
/// `$A$1:$A$2,$C$5`.
fn cells_text(cells: &[ScenarioCell], sep: char) -> String {
    // Cells that follow one another down a column or along a row are
    // written as a range.
    let mut parts: Vec<Rect> = Vec::new();
    for cell in cells {
        match parts.last_mut() {
            Some(last) if last.c0 == last.c1 && cell.col == last.c0 && cell.row == last.r1 + 1 => last.r1 += 1,
            Some(last) if last.r0 == last.r1 && cell.row == last.r0 && cell.col == last.c1 + 1 => last.c1 += 1,
            _ => parts.push(Rect::cell(cell.row, cell.col)),
        }
    }
    let texts: Vec<String> = parts.iter().map(|rect| RefStyle::absolute().format(*rect, "")).collect();
    texts.join(&sep.to_string())
}

/// Data ▸ What-If Analysis ▸ Scenario Manager.
pub(super) fn open_scenario_manager(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<ScenarioManager> {
    let selected = (!app.wb.sheets[app.sheet].scenarios.is_empty()).then_some(0);
    app.open_modal(window, cx, move |modal, _, _| ScenarioManager { modal, selected })
}

/// Show a scenario on the sheet: one undo step.
pub(super) fn show_scenario(app: &mut ExcelApp, index: usize, cx: &mut Cx) -> bool {
    let sheet = app.sheet;
    let Some(scenario) = app.wb.sheets[sheet].scenarios.get(index) else {
        return false;
    };
    let cells: Vec<Rect> = scenario.cells.iter().map(|cell| Rect::cell(cell.row, cell.col)).collect();
    if cells.iter().any(|cell| app.wb.splits_data_table(sheet, *cell) || app.wb.data_table_at(sheet, cell.r0, cell.c0).is_some()) {
        warn(
            app,
            t(
                "Some changing cells are part of an array or data table and cannot be changed.",
                "Certaines cellules variables appartiennent à une matrice ou à un tableau et ne peuvent pas être modifiées.",
            ),
            cx,
        );
        return false;
    }
    if !cells.iter().all(|cell| app.may(Intent::EditCells(*cell), cx)) {
        return false;
    }
    match app.wb.scenario_show(sheet, index) {
        Some(changed) => {
            app.commit_cells(t("Show Scenario", "Afficher le scénario"), &changed, ChangeCause::Fill, cx);
            true
        }
        None => false,
    }
}

/// Delete a scenario of the current sheet.
pub(super) fn delete_scenario(app: &mut ExcelApp, index: usize, cx: &mut Cx) -> bool {
    if let Some(message) = scenarios_locked(app) {
        warn(app, message, cx);
        return false;
    }
    let deleted = app.wb.scenario_delete(app.sheet, index);
    if deleted {
        app.commit(t("Delete Scenario", "Supprimer le scénario"), cx);
    }
    deleted
}

/// The words of the summary sheet.
fn summary_labels() -> SummaryLabels {
    SummaryLabels {
        title: t("Scenario Summary", "Synthèse de scénarios").to_string(),
        current_values: t("Current Values:", "Valeurs actuelles :").to_string(),
        changing_cells: t("Changing Cells:", "Cellules variables :").to_string(),
        result_cells: t("Result Cells:", "Cellules résultantes :").to_string(),
        notes: vec![
            t(
                "Notes:  Current Values column represents values of changing cells at",
                "La colonne Valeurs actuelles affiche les valeurs des cellules variables",
            )
            .to_string(),
            t(
                "time Scenario Summary Report was created.  Changing cells for each",
                "au moment de la création du rapport de synthèse. Les cellules variables",
            )
            .to_string(),
            t("scenario are highlighted in gray.", "de chaque scénario se situent dans les colonnes grisées.").to_string(),
        ],
    }
}

/// Scenario Manager ▸ Summary: the report on a new sheet, which is shown.
pub(super) fn summarize(app: &mut ExcelApp, results: &str, cx: &mut Cx) -> bool {
    let cells = match results.trim() {
        "" => Vec::new(),
        text => match typed_cells(app, text) {
            Some(cells) => cells,
            None => {
                warn(app, t("Reference isn't valid.", "La référence n’est pas valide."), cx);
                return false;
            }
        },
    };
    if !app.may(Intent::InsertSheet, cx) {
        return false;
    }
    let sheet = app.sheet;
    match app.wb.scenario_summary(sheet, &cells, &summary_labels()) {
        Some(report) => {
            app.views.insert(report.min(app.views.len()), crate::app::SheetView::default());
            app.sheet = report;
            app.commit(t("Scenario Summary", "Synthèse de scénarios"), cx);
            true
        }
        None => false,
    }
}

pub(super) struct ScenarioManager {
    modal: Modal,
    selected: Option<usize>,
}

impl ScenarioManager {
    fn scenarios(&self, cx: &Context<Self>) -> Vec<Scenario> {
        self.modal.read_app(cx, |app| app.wb.sheets[app.sheet].scenarios.clone()).unwrap_or_default()
    }

    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        cx.notify();
    }

    pub(super) fn show(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected {
            self.modal.update_app(cx, |app, cx| show_scenario(app, index, cx));
        }
    }

    pub(super) fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected else {
            return;
        };
        if self.modal.update_app(cx, |app, cx| delete_scenario(app, index, cx)) == Some(true) {
            let left = self.scenarios(cx).len();
            self.selected = (left > 0).then(|| index.min(left - 1));
            cx.notify();
        }
    }

    /// Add (`None`) or Edit: the dialog of one scenario, over this one.
    pub(super) fn edit(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<ScenarioDialog>> {
        let manager = cx.entity().downgrade();
        self.modal
            .update_app(cx, |app, cx| {
                if let Some(message) = scenarios_locked(app) {
                    warn(app, message, cx);
                    return None;
                }
                let existing = index.and_then(|index| app.wb.sheets[app.sheet].scenarios.get(index).cloned());
                let sep = app.wb.locale.arg_sep;
                let (name, cells, comment, locked, hidden) = match &existing {
                    Some(scenario) => (
                        scenario.name.clone(),
                        cells_text(&scenario.cells, sep),
                        format!("{}\n{}", scenario.comment, signed(t("Modified by", "Modifié par"), app)).trim().to_string(),
                        scenario.locked,
                        scenario.hidden,
                    ),
                    None => {
                        let selection = RefStyle::absolute().format(app.sel().current(), "");
                        (String::new(), selection, signed(t("Created by", "Créé par"), app), true, false)
                    }
                };
                Some(app.open_modal(window, cx, move |modal, _, cx| ScenarioDialog {
                    name: widgets::input(cx, name),
                    cells: modal.ref_input(RefStyle::absolute(), cells, cx),
                    comment: widgets::input(cx, comment.replace('\n', " ")),
                    modal,
                    index: existing.as_ref().and(index),
                    existing,
                    locked,
                    hidden,
                    manager,
                }))
            })
            .flatten()
    }

    pub(super) fn summary(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<SummaryDialog>> {
        let manager = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| SummaryDialog {
                results: modal.ref_input(RefStyle::absolute(), "", cx),
                modal,
                manager,
            })
        })
    }
}

impl Render for ScenarioManager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scenarios = self.scenarios(cx);
        let sep = self.modal.read_app(cx, |app| app.wb.locale.arg_sep).unwrap_or(',');
        let selected = self.selected.filter(|index| *index < scenarios.len());
        let chosen = selected.map(|index| &scenarios[index]);
        let names: Vec<SharedString> = scenarios.iter().map(|scenario| scenario.name.clone().into()).collect();
        let none = scenarios.is_empty();
        let side = |button: Button| button.width(110.0);
        let list = if none {
            div()
                .w(px(250.))
                .h(px(widgets::ROW_H * 7.0))
                .p(px(6.))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .bg(c(theme::PANEL_BG))
                .child(t(
                    "No Scenarios defined. Choose Add to add scenarios.",
                    "Aucun scénario défini : choisissez Ajouter pour définir des scénarios.",
                ))
                .into_any_element()
        } else {
            ListBox::new("scenarios", names, selected)
                .width(250.0)
                .height(widgets::ROW_H * 7.0)
                .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx)))
                .on_activate(cx.listener(|this, index: &usize, _, cx| {
                    this.select(*index, cx);
                    this.show(cx);
                }))
                .into_any_element()
        };
        let modal = self.modal.clone();
        dialog(&self.modal, t("Scenario Manager", "Gestionnaire de scénarios"))
            .width(410.0)
            .on_ok(cx.listener(|this, _, _, cx| this.show(cx)))
            .child(label(t("Scenarios:", "Scénarios :")))
            .child(
                div().flex().gap(px(12.)).child(list).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(side(Button::new("scenario-add", t("Add...", "Ajouter..."))).on_click(cx.listener(|this, _, window, cx| {
                            this.edit(None, window, cx);
                        })))
                        .child(side(Button::new("scenario-delete", t("Delete", "Supprimer"))).disabled(none).on_click(cx.listener(
                            |this, _, _, cx| this.delete(cx),
                        )))
                        .child(side(Button::new("scenario-edit", t("Edit...", "Modifier..."))).disabled(none).on_click(cx.listener(
                            |this, _, window, cx| {
                                let index = this.selected;
                                if index.is_some() {
                                    this.edit(index, window, cx);
                                }
                            },
                        )))
                        // Merging takes scenarios from another open
                        // workbook: there is only one here.
                        .child(side(Button::new("scenario-merge", t("Merge...", "Fusionner..."))).disabled(true))
                        .child(side(Button::new("scenario-summary", t("Summary...", "Synthèse..."))).disabled(none).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.summary(window, cx);
                            }),
                        )),
                ),
            )
            .child(field(
                t("Changing cells:", "Cellules variables :"),
                130.0,
                label(chosen.map(|scenario| cells_text(&scenario.cells, sep)).unwrap_or_default()),
            ))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(div().w(px(130.)).flex_none().child(label(t("Comment:", "Commentaire :"))))
                    .child(div().flex_1().min_h(px(34.)).child(chosen.map(|scenario| scenario.comment.clone()).unwrap_or_default())),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("scenario-show", t("Show", "Afficher")).primary().disabled(none).on_click(cx.listener(
                        |this, _, _, cx| this.show(cx),
                    )))
                    .child(Button::new("scenario-close", t("Close", "Fermer")).on_click(move |_, window, cx| modal.close(window, cx))),
            )
    }
}

/// Add Scenario / Edit Scenario: the name, the changing cells, the
/// comment and the protection. OK goes on to the values.
pub(super) struct ScenarioDialog {
    modal: Modal,
    /// The scenario being edited and its place in the list.
    index: Option<usize>,
    existing: Option<Scenario>,
    name: Entity<TextInput>,
    cells: Entity<TextInput>,
    comment: Entity<TextInput>,
    locked: bool,
    hidden: bool,
    manager: gpui::WeakEntity<ScenarioManager>,
}

impl ScenarioDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, name: &str, cells: &str, cx: &mut Context<Self>) {
        self.name.update(cx, |field, cx| field.set_text(name, cx));
        self.cells.update(cx, |field, cx| field.set_text(cells, cx));
    }

    /// OK: check the name and the cells, then ask for the values.
    pub(super) fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<ValuesDialog>> {
        let name = self.name.read(cx).text().trim().to_string();
        let cells = self.cells.read(cx).text().to_string();
        let comment = self.comment.read(cx).text().to_string();
        let (index, existing, locked, hidden) = (self.index, self.existing.clone(), self.locked, self.hidden);
        let (parent, manager) = (self.modal.clone(), self.manager.clone());
        self.modal
            .update_app(cx, |app, cx| {
                let taken = app.wb.sheets[app.sheet]
                    .scenarios
                    .iter()
                    .enumerate()
                    .any(|(i, other)| Some(i) != index && other.name.to_lowercase() == name.to_lowercase());
                let cells = typed_cells(app, &cells);
                let refusal = match &cells {
                    _ if name.is_empty() || taken => Some(scenario_error(ScenarioError::Duplicate)),
                    None => Some(scenario_error(ScenarioError::NoCells)),
                    Some(cells) if cells.is_empty() => Some(scenario_error(ScenarioError::NoCells)),
                    Some(cells) if cells.len() > MAX_SCENARIO_CELLS => Some(scenario_error(ScenarioError::TooManyCells)),
                    Some(_) => None,
                };
                if let Some(message) = refusal {
                    warn(app, message, cx);
                    return None;
                }
                // Each cell starts from what the scenario gave it, else
                // from what it holds now.
                let sheet = app.sheet;
                let cells: Vec<ScenarioCell> = cells
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(row, col)| {
                        let kept = existing.as_ref().and_then(|scenario| scenario.cells.iter().find(|cell| (cell.row, cell.col) == (row, col)));
                        let value = match kept {
                            Some(cell) => cell.value.clone(),
                            None => match app.wb.formula_text(sheet, row, col) {
                                // A scenario holds constants: the result of a formula.
                                Some(_) => app.wb.display_text(sheet, row, col).text,
                                None => app.wb.edit_text(sheet, row, col),
                            },
                        };
                        ScenarioCell { row, col, value }
                    })
                    .collect();
                let draft = Scenario {
                    name,
                    comment,
                    user: user_name(),
                    locked,
                    hidden,
                    cells,
                };
                Some(app.open_modal(window, cx, move |modal, _, cx| ValuesDialog {
                    inputs: draft.cells.iter().map(|cell| widgets::input(cx, cell.value.clone())).collect(),
                    modal,
                    parent,
                    manager,
                    index,
                    draft,
                }))
            })
            .flatten()
    }
}

impl ScenarioDialog {
    fn on_next(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.next(window, cx);
    }
}

impl Render for ScenarioDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.index.is_some() { t("Edit Scenario", "Modifier un scénario") } else { t("Add Scenario", "Ajouter un scénario") };
        dialog(&self.modal, title)
            .width(400.0)
            .on_ok(cx.listener(Self::on_next))
            .child(label(t("Scenario name:", "Nom du scénario :")))
            .child(TextField::new(&self.name))
            .child(label(t("Changing cells:", "Cellules variables :")))
            .child(RefEdit::new(&self.cells, &self.modal))
            .child(div().text_color(c(theme::TEXT_MUTED)).child(t(
                "Cmd+click cells to select non-adjacent changing cells.",
                "Cliquez sur des cellules tout en appuyant sur la touche Commande pour sélectionner des cellules variables non adjacentes.",
            )))
            .child(label(t("Comment:", "Commentaire :")))
            .child(TextField::new(&self.comment))
            .child(label(t("Protection:", "Protection :")))
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(Checkbox::new("prevent", t("Prevent changes", "Changements interdits"), self.locked).on_change(cx.listener(
                        |this, on: &bool, _, cx| {
                            this.locked = *on;
                            cx.notify();
                        },
                    )))
                    .child(Checkbox::new("hide", t("Hide", "Masquer"), self.hidden).on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.hidden = *on;
                        cx.notify();
                    }))),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::on_next)))
    }
}

/// Scenario Values: one box per changing cell.
pub(super) struct ValuesDialog {
    modal: Modal,
    /// The Add / Edit Scenario dialog underneath, closed with this one.
    parent: Modal,
    manager: gpui::WeakEntity<ScenarioManager>,
    index: Option<usize>,
    draft: Scenario,
    inputs: Vec<Entity<TextInput>>,
}

impl ValuesDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, values: &[&str], cx: &mut Context<Self>) {
        for (field, value) in self.inputs.iter().zip(values) {
            field.update(cx, |field, cx| field.set_text(*value, cx));
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let mut scenario = self.draft.clone();
        for (cell, field) in scenario.cells.iter_mut().zip(&self.inputs) {
            cell.value = field.read(cx).text().to_string();
        }
        let index = self.index;
        let stored = self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            let (outcome, label) = match index {
                Some(index) => (app.wb.scenario_edit(sheet, index, scenario), t("Edit Scenario", "Modifier un scénario")),
                None => (app.wb.scenario_add(sheet, scenario), t("Add Scenario", "Ajouter un scénario")),
            };
            match outcome {
                Ok(()) => {
                    app.commit(label, cx);
                    Some(app.wb.sheets[sheet].scenarios.len())
                }
                Err(error) => {
                    warn(app, scenario_error(error), cx);
                    None
                }
            }
        });
        if let Some(Some(count)) = stored {
            // The manager shows the scenario just stored.
            self.manager
                .update(cx, |manager, cx| manager.select(index.unwrap_or(count.saturating_sub(1)), cx))
                .ok();
            self.parent.close(window, cx);
        }
    }
}

impl Render for ValuesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.draft.cells.iter().zip(&self.inputs).enumerate().map(|(i, (cell, input))| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(24.)).flex_none().text_color(c(theme::TEXT_MUTED)).child(format!("{}:", i + 1)))
                .child(div().w(px(80.)).flex_none().child(absolute(cell.row, cell.col)))
                .child(TextField::new(input).width(180.0))
        });
        // Five cells show at once, as in Excel; the others scroll.
        let shown = self.inputs.len().clamp(1, 5) as f32;
        dialog(&self.modal, t("Scenario Values", "Valeurs de scénarios"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Enter values for each of the changing cells.", "Tapez des valeurs pour chacune des cellules variables.")))
            .child(ScrollArea::new("values", shown * (widgets::CONTROL_H + 6.0)).child(div().flex().flex_col().gap(px(6.)).children(rows)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// Scenario Summary: the kind of report and the result cells.
pub(super) struct SummaryDialog {
    modal: Modal,
    /// The Scenario Manager, closed once the report is made.
    manager: Modal,
    results: Entity<TextInput>,
}

impl SummaryDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, results: &str, cx: &mut Context<Self>) {
        self.results.update(cx, |field, cx| field.set_text(results, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let results = self.results.read(cx).text().to_string();
        if self.modal.update_app(cx, |app, cx| summarize(app, &results, cx)) == Some(true) {
            self.manager.close(window, cx);
        }
    }
}

impl Render for SummaryDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Scenario Summary", "Synthèse de scénarios"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Report type:", "Type de rapport :")))
            .child(
                // The PivotTable report is not built: one choice.
                div().pl(px(10.)).child(RadioGroup::new("report", vec![t("Scenario summary", "Synthèse de scénarios").into()], Some(0))),
            )
            .child(label(t("Result cells:", "Cellules résultantes :")))
            .child(RefEdit::new(&self.results, &self.modal))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
