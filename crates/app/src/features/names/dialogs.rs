//! The small dialogs of defined names: New Name / Edit Name, Create Names
//! from Selection, Paste Name and Apply Names.

use super::{
    LabelSides, NameRow, RowKind, absolute_ref, apply_names, formula_names, guess_label_sides, insert_in_formula,
    invalid_reference, label_to_name, locked, name_error_message, names_from_labels, paste_list, selection_formula,
};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{self, Button, Checkbox, ListBox, RefEdit, Select, TextField, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::names::{NameError, NameScope};
use opensheet_engine::table::TableError;
use opensheet_engine::{Expr, MAX_COLS, MAX_ROWS, Rect, Value};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Tell the user why what was asked cannot be done.
fn refuse(app: &mut ExcelApp, message: impl Into<SharedString>, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// A control with its label in a column of fixed width, so that the
/// fields of a dialog line up.
fn field(text: &'static str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(110.)).flex_none().child(label(text)))
        .child(control)
}

// ------------------------------------------------------ New Name / Edit Name

/// What the dialog defines.
#[derive(Clone)]
enum Target {
    New,
    /// An existing name: its scope and its spelling.
    Name(NameScope, String),
    /// A table (sheet, id): only its name can be changed here.
    Table(usize, u32),
}

struct NameInit {
    target: Target,
    name: String,
    scopes: Vec<SharedString>,
    scope: usize,
    comment: String,
    refers: String,
}

/// The Scope list: the workbook, then every sheet.
fn scopes(app: &ExcelApp) -> Vec<SharedString> {
    let mut scopes: Vec<SharedString> = vec![t("Workbook", "Classeur").into()];
    scopes.extend(app.wb.sheets.iter().map(|sheet| SharedString::from(sheet.name.clone())));
    scopes
}

fn scope_at(index: usize) -> NameScope {
    match index.checked_sub(1) {
        Some(sheet) => NameScope::Sheet(sheet),
        None => NameScope::Workbook,
    }
}

/// The name Excel proposes for the selection: the label in the active
/// cell, else the one on its left, else the one above it.
fn suggested_name(app: &ExcelApp) -> String {
    let (row, col) = app.active();
    let near = [Some((row, col)), col.checked_sub(1).map(|c| (row, c)), row.checked_sub(1).map(|r| (r, col))];
    near.into_iter()
        .flatten()
        .filter_map(|(r, c)| match app.wb.value_at(app.sheet, r, c) {
            Value::Text(text) => label_to_name(&text, app.wb.locale),
            _ => None,
        })
        .find(|name| app.wb.check_name(NameScope::Workbook, name, None).is_ok())
        .unwrap_or_default()
}

/// Define Name: name the selection.
pub(super) fn open_new_name(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(message) = locked(app) {
        refuse(app, message, cx);
        return;
    }
    let init = NameInit {
        target: Target::New,
        name: suggested_name(app),
        scopes: scopes(app),
        scope: 0,
        comment: String::new(),
        refers: selection_formula(app),
    };
    app.open_modal(window, cx, move |modal, _, cx| NameDialog::new(modal, init, cx));
}

/// Edit Name, on a line of the Name Manager.
pub(super) fn open_edit_name(app: &mut ExcelApp, row: &NameRow, window: &mut Window, cx: &mut Cx) {
    let (target, scope) = match row.kind {
        RowKind::Name(scope) => {
            let index = match scope {
                NameScope::Workbook => 0,
                NameScope::Sheet(sheet) => sheet + 1,
            };
            (Target::Name(scope, row.name.clone()), index)
        }
        RowKind::Table(sheet, id) => (Target::Table(sheet, id), 0),
    };
    let init = NameInit {
        target,
        name: row.name.clone(),
        scopes: scopes(app),
        scope,
        comment: row.comment.clone(),
        refers: row.refers_to.clone(),
    };
    app.open_modal(window, cx, move |modal, _, cx| NameDialog::new(modal, init, cx));
}

/// Store what the dialog holds. Returns false, after saying why, when
/// Excel would refuse it.
fn store_name(app: &mut ExcelApp, target: &Target, scope: usize, name: &str, refers: &str, comment: &str, cx: &mut Cx) -> bool {
    if let Target::Table(sheet, id) = target {
        return match app.wb.rename_table(*sheet, *id, name) {
            Ok(()) => {
                app.commit(t("Rename Table", "Renommer le tableau"), cx);
                true
            }
            Err(error) => {
                let error = if error == TableError::NameTaken { NameError::Duplicate } else { NameError::Invalid };
                refuse(app, name_error_message(error), cx);
                false
            }
        };
    }
    // The scope of an existing name cannot change.
    let (scope, replacing) = match target {
        Target::Name(scope, old) => (*scope, Some(old.as_str())),
        _ => (scope_at(scope), None),
    };
    if let Err(error) = app.wb.check_name(scope, name, replacing) {
        refuse(app, name_error_message(error), cx);
        return false;
    }
    let (row, col) = app.active();
    let from = match scope {
        NameScope::Sheet(sheet) => sheet,
        NameScope::Workbook => app.sheet,
    };
    let Ok(expr) = app.wb.parse_name_formula(refers, from, row, col) else {
        refuse(app, invalid_reference(), cx);
        return false;
    };
    let (stored, label) = match replacing {
        Some(old) => (app.wb.edit_name(scope, old, name, expr, comment), t("Edit Name", "Modifier le nom")),
        None => (app.wb.add_name(scope, name, expr, comment), t("Define Name", "Définir un nom")),
    };
    match stored {
        Ok(()) => {
            app.commit(label, cx);
            true
        }
        Err(error) => {
            refuse(app, name_error_message(error), cx);
            false
        }
    }
}

struct NameDialog {
    modal: Modal,
    target: Target,
    name: Entity<TextInput>,
    scopes: Vec<SharedString>,
    scope: usize,
    comment: Entity<TextInput>,
    refers: Entity<TextInput>,
}

impl NameDialog {
    fn new(modal: Modal, init: NameInit, cx: &mut Context<Self>) -> Self {
        NameDialog {
            name: widgets::input(cx, init.name),
            comment: widgets::input(cx, init.comment),
            refers: modal.ref_input(RefStyle::formula(), init.refers, cx),
            modal,
            target: init.target,
            scopes: init.scopes,
            scope: init.scope,
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_string();
        let refers = self.refers.read(cx).text().to_string();
        let comment = self.comment.read(cx).text().to_string();
        let (target, scope) = (self.target.clone(), self.scope);
        let stored = self
            .modal
            .update_app(cx, |app, cx| store_name(app, &target, scope, &name, &refers, &comment, cx));
        if stored == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for NameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, table) = match self.target {
            Target::New => (t("New Name", "Nouveau nom"), false),
            Target::Name(..) => (t("Edit Name", "Modifier le nom"), false),
            Target::Table(..) => (t("Edit Name", "Modifier le nom"), true),
        };
        let fixed_scope = !matches!(self.target, Target::New);
        dialog(&self.modal, title)
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(field(t("Name:", "Nom :"), TextField::new(&self.name)))
            .child(field(
                t("Scope:", "Étendue :"),
                Select::new("scope", self.scopes.clone(), Some(self.scope))
                    .width(200.0)
                    .disabled(fixed_scope)
                    .on_change(cx.listener(|this, index: &usize, _, cx| {
                        this.scope = *index;
                        cx.notify();
                    })),
            ))
            .when(!table, |d| {
                d.child(field(t("Comment:", "Commentaire :"), TextField::new(&self.comment)))
                    .child(field(t("Refers to:", "Fait référence à :"), RefEdit::new(&self.refers, &self.modal)))
            })
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------- Create Names from Selection

pub(super) fn open_create_names(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(message) = locked(app) {
        refuse(app, message, cx);
        return;
    }
    if app.sel().ranges.len() > 1 {
        refuse(
            app,
            t(
                "This action won't work on multiple selections.",
                "Cette action ne peut pas être effectuée sur des sélections multiples.",
            ),
            cx,
        );
        return;
    }
    let sides = guess_label_sides(&app.wb, app.sheet, app.sel().current());
    app.open_modal(window, cx, move |modal, _, _| CreateNames { modal, sides });
}

/// Define the names one after the other, asking before one that exists is
/// redefined; everything is one undo step.
fn create_names(app: &mut ExcelApp, mut queue: VecDeque<(String, Expr)>, mut made: usize, cx: &mut Cx) {
    let scope = NameScope::Workbook;
    while let Some((name, expr)) = queue.pop_front() {
        match app.wb.check_name(scope, &name, None) {
            Ok(()) => {
                if app.wb.add_name(scope, &name, expr, "").is_ok() {
                    made += 1;
                }
            }
            Err(NameError::Duplicate) => {
                let question = t(
                    "Replace existing definition of '{}'?",
                    "Voulez-vous remplacer la définition existante de '{}'?",
                )
                .replace("{}", &name);
                // The buttons are asked for once each, but may be pressed
                // only once altogether: the rest of the work is handed to
                // whichever is.
                let rest = Rc::new(RefCell::new(Some((queue, name, expr))));
                let (yes, no, cancel) = (rest.clone(), rest.clone(), rest);
                let alert = Alert::new("OpenSheet", question)
                    .warning()
                    .button(t("Yes", "Oui"), move |app, _, cx| {
                        if let Some((queue, name, expr)) = yes.borrow_mut().take() {
                            let comment = app.wb.name_in(scope, &name).map(|def| def.comment.clone()).unwrap_or_default();
                            let replaced = app.wb.edit_name(scope, &name, &name, expr, &comment).is_ok();
                            create_names(app, queue, made + replaced as usize, cx);
                        }
                    })
                    .button(t("No", "Non"), move |app, _, cx| {
                        if let Some((queue, _, _)) = no.borrow_mut().take() {
                            create_names(app, queue, made, cx);
                        }
                    })
                    .button(t("Cancel", "Annuler"), move |app, _, cx| {
                        if cancel.borrow_mut().take().is_some() {
                            create_names(app, VecDeque::new(), made, cx);
                        }
                    });
                app.alert(alert, cx);
                return;
            }
            // A table is called that: its name is not redefined.
            Err(_) => {}
        }
    }
    if made > 0 {
        app.commit(t("Create Names", "Créer des noms"), cx);
    }
}

struct CreateNames {
    modal: Modal,
    sides: LabelSides,
}

impl CreateNames {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let sides = self.sides;
        self.modal.close(window, cx);
        if !sides.any() {
            return;
        }
        self.modal.update_app(cx, |app, cx| {
            let sheet = &app.wb.sheets[app.sheet].name;
            let queue: VecDeque<(String, Expr)> = names_from_labels(&app.wb, app.sheet, app.sel().current(), sides)
                .into_iter()
                .map(|(name, cells)| (name, absolute_ref(sheet, cells)))
                .collect();
            create_names(app, queue, 0, cx);
        });
    }
}

impl Render for CreateNames {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let side = |id: &'static str, text: &'static str, on: bool, set: fn(&mut LabelSides, bool), cx: &mut Context<Self>| {
            Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
                set(&mut this.sides, *on);
                cx.notify();
            }))
        };
        let sides = self.sides;
        dialog(&self.modal, t("Create Names from Selection", "Créer des noms à partir de la sélection"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Create names from values in the:", "Créer des noms à partir des valeurs de la :")))
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(side("top", t("Top row", "Ligne du haut"), sides.top, |s, on| s.top = on, cx))
                    .child(side("left", t("Left column", "Colonne de gauche"), sides.left, |s, on| s.left = on, cx))
                    .child(side("bottom", t("Bottom row", "Ligne du bas"), sides.bottom, |s, on| s.bottom = on, cx))
                    .child(side("right", t("Right column", "Colonne de droite"), sides.right, |s, on| s.right = on, cx)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------------- Paste Name

pub(super) fn open_paste_names(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let names = formula_names(&app.wb, app.sheet);
    if names.is_empty() {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| PasteNames {
        modal,
        names,
        selected: None,
    });
}

struct PasteNames {
    modal: Modal,
    names: Vec<String>,
    selected: Option<usize>,
}

impl PasteNames {
    /// OK puts the chosen name into the formula being typed.
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let name = self.selected.and_then(|index| self.names.get(index)).cloned();
        self.modal.close(window, cx);
        if let Some(name) = name {
            self.modal.update_app(cx, |app, cx| insert_in_formula(app, &name, window, cx));
        }
    }

    fn paste_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            // The list goes into cells: an entry in progress is given up.
            if app.is_editing() {
                app.cancel_edit(window, cx);
            }
            paste_list(app, cx);
        });
    }
}

impl Render for PasteNames {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.names.iter().map(|name| SharedString::from(name.clone())).collect();
        dialog(&self.modal, t("Paste Name", "Coller un nom"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Paste name", "Coller un nom")))
            .child(
                div().flex().child(
                    ListBox::new("names", names, self.selected)
                        .height(180.0)
                        .on_change(cx.listener(|this, index: &usize, _, cx| {
                            this.selected = Some(*index);
                            cx.notify();
                        }))
                        .on_activate(cx.listener(|this, index: &usize, window, cx| {
                            this.selected = Some(*index);
                            this.ok(&(), window, cx);
                        })),
                ),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("paste-list", t("Paste List", "Coller une liste"))
                            .on_click(cx.listener(|this, _, window, cx| this.paste_list(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(ok_cancel(&self.modal, cx.listener(Self::ok))),
            )
    }
}

// -------------------------------------------------------------- Apply Names

pub(super) fn open_apply_names(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(message) = locked(app) {
        refuse(app, message, cx);
        return;
    }
    let names = formula_names(&app.wb, app.sheet);
    if names.is_empty() {
        return;
    }
    let selected = (0..names.len()).collect();
    app.open_modal(window, cx, move |modal, _, _| ApplyNames {
        modal,
        names,
        selected,
        ignore_kind: true,
    });
}

struct ApplyNames {
    modal: Modal,
    names: Vec<String>,
    selected: Vec<usize>,
    /// "Ignore Relative/Absolute".
    ignore_kind: bool,
}

impl ApplyNames {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let names: Vec<String> = self.selected.iter().filter_map(|index| self.names.get(*index)).cloned().collect();
        let ignore_kind = self.ignore_kind;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            // One cell selected: the whole sheet, as Excel does.
            let ranges = if app.sel().is_single_cell() {
                vec![Rect::new(0, 0, MAX_ROWS - 1, MAX_COLS - 1)]
            } else {
                app.sel().ranges.clone()
            };
            let sheet = app.sheet;
            if apply_names(&mut app.wb, sheet, &ranges, &names, ignore_kind) > 0 {
                app.commit(t("Apply Names", "Appliquer les noms"), cx);
            } else {
                refuse(
                    app,
                    t(
                        "OpenSheet cannot find any references to replace.",
                        "OpenSheet ne trouve pas de références à remplacer.",
                    ),
                    cx,
                );
            }
        });
    }
}

impl Render for ApplyNames {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.names.iter().map(|name| SharedString::from(name.clone())).collect();
        dialog(&self.modal, t("Apply Names", "Appliquer les noms"))
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Apply names:", "Appliquer les noms :")))
            .child(
                div().flex().child(
                    ListBox::multi("names", names, self.selected.clone())
                        .height(180.0)
                        .on_change_multi(cx.listener(|this, rows: &Vec<usize>, _, cx| {
                            this.selected = rows.clone();
                            cx.notify();
                        })),
                ),
            )
            .child(
                Checkbox::new("ignore", t("Ignore Relative/Absolute", "Ignorer relatif/absolu"), self.ignore_kind).on_change(
                    cx.listener(|this, on: &bool, _, cx| {
                        this.ignore_kind = *on;
                        cx.notify();
                    }),
                ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
