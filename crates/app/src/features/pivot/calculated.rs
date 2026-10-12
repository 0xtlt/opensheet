//! "Fields, Items, & Sets": the Insert Calculated Field and Insert
//! Calculated Item dialogs, Solve Order and List Formulas. Sets belong to
//! OLAP sources, which the engine does not read: their entries are listed
//! as in Excel and stay grey.

use super::{active_cell, active_pivot, entry, logic, perform};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::widgets::{self, Button, ComboBox, ListBox, TextField, dialog, label, labeled};
use gpui::{Context, Entity, Focusable, SharedString, Window, div, prelude::*, px};
use opensheet_engine::pivot::{ListLabels, PivotError};

/// The drop-down of the ribbon button.
pub(super) fn menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let has_items = active_pivot(app).is_some_and(|p| !p.cache.calc_items.is_empty());
    let has_any = has_items || active_pivot(app).is_some_and(|p| p.cache.fields.iter().any(|f| f.formula.is_some()));
    let unavailable = |text: &'static str| entry(text, |_, _, _| {}).disabled(true);
    let entries = vec![
        entry(t("Calculated Field...", "Champ calculé..."), open_field),
        entry(t("Calculated Item...", "Élément calculé..."), open_item).disabled(item_field(app).is_none()),
        entry(t("Solve Order...", "Ordre de résolution..."), open_solve_order).disabled(!has_items),
        entry(t("List Formulas", "Liste des formules"), list_formulas).disabled(!has_any),
        crate::app::MenuEntry::separator(),
        unavailable(t("Create Set Based on Row Items...", "Créer un jeu d’après les éléments de ligne...")),
        unavailable(t("Create Set Based on Column Items...", "Créer un jeu d’après les éléments de colonne...")),
        unavailable(t("Manage Sets...", "Gérer les jeux...")),
    ];
    app.open_command_menu(entries, 300.0, window, cx);
}

/// The field a calculated item would join: the row or column field the
/// active cell is a label or the header of.
pub(super) fn item_field(app: &ExcelApp) -> Option<usize> {
    let cell = active_cell(app)?;
    let pivot = active_pivot(app)?;
    let field = cell.field.or(cell.fields.first().copied())?;
    matches!(pivot.axis_of(field), Some(opensheet_engine::pivot::Area::Rows | opensheet_engine::pivot::Area::Columns)).then_some(field)
}

fn refuse(app: &mut ExcelApp, err: &PivotError, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", logic::error_text(err)).warning(), cx);
}

/// What the dialog edits: the calculated fields of the cache, or the
/// calculated items of one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Field,
    Item(usize),
}

/// Insert Calculated Field / Insert Calculated Item.
pub(super) struct CalcDialog {
    modal: Modal,
    pivot: u32,
    pub(super) kind: Kind,
    /// The field the items belong to, for the title.
    field_name: SharedString,
    pub(super) name: Entity<TextInput>,
    pub(super) formula: Entity<TextInput>,
    /// The calculated fields or items there are: (index or item, name,
    /// formula).
    pub(super) existing: Vec<(u32, String, String)>,
    /// The fields of the cache, and the items of the field.
    pub(super) fields: Vec<SharedString>,
    pub(super) items: Vec<SharedString>,
    pub(super) field_at: Option<usize>,
    pub(super) item_at: Option<usize>,
}

fn existing(app: &ExcelApp, pivot: u32, kind: Kind) -> Vec<(u32, String, String)> {
    let locale = app.wb.locale;
    let Some((_, p)) = app.wb.pivot(pivot) else {
        return Vec::new();
    };
    match kind {
        Kind::Field => p.calculated_fields(locale).into_iter().map(|(f, name, formula)| (f as u32, name, formula)).collect(),
        Kind::Item(field) => p.calculated_items(field, locale),
    }
}

fn open(app: &mut ExcelApp, kind: Kind, window: &mut Window, cx: &mut Cx) {
    let Some(p) = active_pivot(app) else {
        return;
    };
    if let Kind::Item(field) = kind
        && let Some(err) = p.calculated_item_refusal(field)
    {
        refuse(app, &err, cx);
        return;
    }
    let pivot = p.id;
    let fields: Vec<SharedString> = (0..p.fields.len()).map(|f| p.field_caption(f).to_string().into()).collect();
    let (items, field_name): (Vec<SharedString>, SharedString) = match kind {
        Kind::Field => (Vec::new(), SharedString::default()),
        Kind::Item(field) => (
            p.cache.item_names(field).into_iter().map(SharedString::from).collect(),
            p.field_caption(field).to_string().into(),
        ),
    };
    let list = existing(app, pivot, kind);
    // Excel proposes "Field1", "Formula1": the first number that is free.
    let stem = match kind {
        Kind::Field => t("Field", "Champ"),
        Kind::Item(_) => t("Formula", "Formule"),
    };
    let taken = |name: &str| {
        fields.iter().chain(&items).any(|known| known.to_lowercase() == name.to_lowercase())
            || list.iter().any(|(_, known, _)| known.to_lowercase() == name.to_lowercase())
    };
    let proposed = (1..).map(|n| format!("{stem}{n}")).find(|name| !taken(name)).unwrap_or_default();
    app.open_modal(window, cx, move |modal, _, cx| CalcDialog {
        modal,
        pivot,
        kind,
        field_name,
        name: widgets::input(cx, proposed),
        formula: widgets::input(cx, "= 0"),
        existing: list,
        fields,
        items,
        field_at: Some(0),
        item_at: None,
    });
}

pub(super) fn open_field(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    open(app, Kind::Field, window, cx);
}

pub(super) fn open_item(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if let Some(field) = item_field(app) {
        open(app, Kind::Item(field), window, cx);
    }
}

/// A name as a formula writes it.
fn in_formula(name: &str) -> String {
    let plain = name.chars().all(|ch| ch.is_alphanumeric() || ch == '_') && name.chars().next().is_some_and(char::is_alphabetic);
    if plain { name.to_string() } else { format!("'{}'", name.replace('\'', "''")) }
}

impl CalcDialog {
    /// The calculated field or item the Name box names, if there is one.
    fn named(&self, cx: &Context<Self>) -> Option<&(u32, String, String)> {
        let name = self.name.read(cx).text().trim().to_lowercase();
        self.existing.iter().find(|(_, known, _)| known.to_lowercase() == name)
    }

    /// "Add" or "Modify": store what the two boxes hold. True when it was
    /// taken.
    pub(super) fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let name = self.name.read(cx).text().trim().to_string();
        let formula = self.formula.read(cx).text().to_string();
        let (pivot, kind) = (self.pivot, self.kind);
        let label = match kind {
            Kind::Field => t("Calculated Field", "Champ calculé"),
            Kind::Item(_) => t("Calculated Item", "Élément calculé"),
        };
        let done = self.modal.update_app(cx, |app, cx| {
            let outcome = perform(app, label, window, cx, move |app, overwrite| match kind {
                Kind::Field => app.wb.pivot_set_calculated_field(pivot, &name, &formula, overwrite).map(|_| ()),
                Kind::Item(field) => app.wb.pivot_set_calculated_item(pivot, field, &name, &formula, overwrite).map(|_| ()),
            });
            (outcome == super::Outcome::Done, existing(app, pivot, kind))
        });
        let Some((done, list)) = done else {
            return false;
        };
        self.existing = list;
        cx.notify();
        done
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((at, ..)) = self.named(cx).cloned() else {
            return;
        };
        let (pivot, kind) = (self.pivot, self.kind);
        let list = self.modal.update_app(cx, |app, cx| {
            perform(app, t("Delete", "Supprimer"), window, cx, move |app, overwrite| match kind {
                Kind::Field => app.wb.pivot_delete_calculated_field(pivot, at as usize, overwrite),
                Kind::Item(field) => app.wb.pivot_delete_calculated_item(pivot, field, at, overwrite),
            });
            existing(app, pivot, kind)
        });
        self.existing = list.unwrap_or_default();
        self.name.update(cx, |input, cx| input.set_text("", cx));
        self.formula.update(cx, |input, cx| input.set_text("= 0", cx));
        cx.notify();
    }

    /// Put a name at the caret of the formula ("Insert Field", "Insert
    /// Item", a double click in a list).
    pub(super) fn insert(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = in_formula(name);
        self.formula.update(cx, |input, cx| {
            // A formula still as proposed is replaced, not added to.
            if input.text().trim() == "= 0" {
                input.set_text("=", cx);
            }
            input.insert(&text, window, cx);
        });
        window.focus(&self.formula.read(cx).focus_handle(cx));
    }

    /// The formula of the name just picked in the Name box.
    fn show_named(&mut self, cx: &mut Context<Self>) {
        if let Some((_, _, formula)) = self.named(cx).cloned() {
            self.formula.update(cx, |input, cx| input.set_text(format!("={formula}"), cx));
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        // OK stores what is typed unless it is the proposal left as it
        // was, then closes.
        let untouched = self.formula.read(cx).text().trim() == "= 0" && self.named(cx).is_none();
        if untouched || self.add(window, cx) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for CalcDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title: SharedString = match self.kind {
            Kind::Field => t("Insert Calculated Field", "Insertion d'un champ calculé").into(),
            Kind::Item(_) => t("Insert Calculated Item in \"{}\"", "Insérer un élément calculé dans «\u{a0}{}\u{a0}»").replace("{}", &self.field_name).into(),
        };
        let known = self.named(cx).is_some();
        let names: Vec<SharedString> = self.existing.iter().map(|(_, name, _)| name.clone().into()).collect();
        let picked = self.existing.clone();
        let list = |id: &'static str, entries: &[SharedString], at: Option<usize>, item: bool, cx: &mut Context<Self>| {
            let labels = entries.to_vec();
            ListBox::new(id, entries.to_vec(), at)
                .width(200.0)
                .height(widgets::ROW_H * 7.0)
                .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                    if item {
                        this.item_at = Some(*ix);
                    } else {
                        this.field_at = Some(*ix);
                    }
                    cx.notify();
                }))
                .on_activate(cx.listener(move |this, ix: &usize, window, cx| {
                    if let Some(name) = labels.get(*ix) {
                        // For an item, a field of the list is only chosen;
                        // its formula computes from items.
                        if item || this.kind == Kind::Field {
                            this.insert(name, window, cx);
                        }
                    }
                }))
        };
        let field_list = list("fields", &self.fields.clone(), self.field_at, false, cx);
        let insert_field = Button::new("insert-field", t("Insert Field", "Insérer un champ"))
            .disabled(self.kind != Kind::Field || self.field_at.is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(name) = this.field_at.and_then(|at| this.fields.get(at).cloned()) {
                    this.insert(&name, window, cx);
                }
            }));
        let mut lists = div().flex().gap(px(12.)).child(
            div().flex().flex_col().gap(px(4.)).child(label(t("Fields:", "Champs\u{a0}:"))).child(field_list).child(div().flex().child(insert_field)),
        );
        if matches!(self.kind, Kind::Item(_)) {
            let item_list = list("items", &self.items.clone(), self.item_at, true, cx);
            let insert_item = Button::new("insert-item", t("Insert Item", "Insérer l’élément"))
                .disabled(self.item_at.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(name) = this.item_at.and_then(|at| this.items.get(at).cloned()) {
                        this.insert(&name, window, cx);
                    }
                }));
            lists = lists.child(
                div().flex().flex_col().gap(px(4.)).child(label(t("Items:", "Éléments\u{a0}:"))).child(item_list).child(div().flex().child(insert_item)),
            );
        }
        let buttons = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                Button::new("add", if known { t("Modify", "Modifier") } else { t("Add", "Ajouter") })
                    .width(90.0)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add(window, cx);
                    })),
            )
            .child(
                Button::new("delete", t("Delete", "Supprimer"))
                    .width(90.0)
                    .disabled(!known)
                    .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
            );
        dialog(&self.modal, title)
            .width(if matches!(self.kind, Kind::Item(_)) { 580.0 } else { 470.0 })
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(labeled(
                                t("Name:", "Nom\u{a0}:"),
                                ComboBox::new("name", &self.name, names).width(250.0).on_pick(cx.listener(move |this, ix: &usize, _, cx| {
                                    if let Some((_, name, _)) = picked.get(*ix) {
                                        let name = name.clone();
                                        this.name.update(cx, |input, cx| input.set_text(name, cx));
                                        this.show_named(cx);
                                    }
                                })),
                            ))
                            .child(labeled(t("Formula:", "Formule\u{a0}:"), TextField::new(&self.formula).width(250.0))),
                    )
                    .child(buttons),
            )
            .child(lists)
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------------- solve order

/// "Calculated Item Solve Order".
pub(super) struct SolveOrder {
    modal: Modal,
    pivot: u32,
    /// The formulas, in solve order.
    pub(super) lines: Vec<SharedString>,
    pub(super) selected: usize,
}

fn solve_lines(app: &ExcelApp, pivot: u32) -> Vec<SharedString> {
    let locale = app.wb.locale;
    app.wb.pivot(pivot).map_or(Vec::new(), |(_, p)| p.solve_order(locale).into_iter().map(SharedString::from).collect())
}

pub(super) fn open_solve_order(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app).map(|p| p.id) else {
        return;
    };
    let lines = solve_lines(app, pivot);
    app.open_modal(window, cx, move |modal, _, _| SolveOrder {
        modal,
        pivot,
        lines,
        selected: 0,
    });
}

impl SolveOrder {
    /// Move the selected formula one place up or down.
    pub(super) fn shift(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (at, count) = (self.selected, self.lines.len());
        let to = if down { at + 1 } else { at.wrapping_sub(1) };
        if to >= count {
            return;
        }
        let mut order: Vec<usize> = (0..count).collect();
        order.swap(at, to);
        let pivot = self.pivot;
        let lines = self.modal.update_app(cx, |app, cx| {
            perform(app, t("Solve Order", "Ordre de résolution"), window, cx, move |app, overwrite| {
                app.wb.pivot_set_solve_order(pivot, &order, overwrite)
            });
            solve_lines(app, pivot)
        });
        self.lines = lines.unwrap_or_default();
        self.selected = to;
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (pivot, at) = (self.pivot, self.selected);
        let lines = self.modal.update_app(cx, |app, cx| {
            let target = app.wb.pivot(pivot).and_then(|(_, p)| {
                let item = p.cache.calc_items.get(at)?;
                Some((item.field, p.cache.calc_item_index(item)?))
            });
            if let Some((field, x)) = target {
                perform(app, t("Delete", "Supprimer"), window, cx, move |app, overwrite| {
                    app.wb.pivot_delete_calculated_item(pivot, field, x, overwrite)
                });
            }
            solve_lines(app, pivot)
        });
        self.lines = lines.unwrap_or_default();
        self.selected = self.selected.min(self.lines.len().saturating_sub(1));
        cx.notify();
    }
}

impl Render for SolveOrder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (at, count) = (self.selected, self.lines.len());
        let close = self.modal.clone();
        dialog(&self.modal, t("Calculated Item Solve Order", "Ordre de résolution des éléments calculés"))
            .width(430.0)
            .child(label(t("Solve order:", "Ordre de résolution\u{a0}:")))
            .child(
                div().flex().child(
                    ListBox::new("order", self.lines.clone(), (count > 0).then_some(at))
                        .width(400.0)
                        .height(widgets::ROW_H * 7.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.selected = *ix;
                            cx.notify();
                        })),
                ),
            )
            .child(div().child(t(
                "If a PivotTable value is affected by two or more calculated items, the last formula in the solve order will calculate that value.",
                "Si la valeur d’un tableau croisé dynamique est affectée par deux éléments calculés ou plus, la dernière formule dans l’ordre de résolution calculera cette valeur.",
            )))
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("up", t("Move Up", "Monter"))
                            .disabled(count == 0 || at == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.shift(false, window, cx))),
                    )
                    .child(
                        Button::new("down", t("Move Down", "Descendre"))
                            .disabled(at + 1 >= count)
                            .on_click(cx.listener(|this, _, window, cx| this.shift(true, window, cx))),
                    )
                    .child(
                        Button::new("delete", t("Delete", "Supprimer"))
                            .disabled(count == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(Button::new("close", t("Close", "Fermer")).primary().on_click(move |_, window, cx| close.close(window, cx))),
            )
    }
}

// -------------------------------------------------------------- list formulas

/// "List Formulas": the calculated fields and items on a new sheet.
pub(super) fn list_formulas(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app).map(|p| p.id) else {
        return;
    };
    if !app.may(crate::features::Intent::InsertSheet, cx) {
        return;
    }
    let labels = ListLabels {
        calculated_field: t("Calculated Field", "Champ calculé").to_string(),
        calculated_item: t("Calculated Item", "Élément calculé").to_string(),
        solve_order: t("Solve Order", "Ordre de résolution").to_string(),
        field: t("Field", "Champ").to_string(),
        item: t("Item", "Élément").to_string(),
        formula: t("Formula", "Formule").to_string(),
        note: vec![
            t("Note:", "Remarque\u{a0}:").to_string(),
            format!(
                "{} {}",
                t("When a cell is updated by more than one formula,", "Lorsqu'une cellule est mise à jour par plusieurs formules,"),
                t("the value is set by the formula with the last solve order.", "la valeur est établie par la dernière formule dans l'ordre de résolution.")
            ),
            String::new(),
            format!(
                "{} {}",
                t("To change the solve order for multiple calculated items or fields,", "Pour modifier l'ordre de résolution de plusieurs champs ou éléments calculés,"),
                t(
                    "on the Options tab, in the Calculations group, click Fields, Items, & Sets, and then click Solve Order.",
                    "dans l’onglet Options, groupe Calculs, cliquez sur Champs, éléments et jeux, puis sur Ordre de résolution."
                )
            ),
        ],
    };
    match app.wb.pivot_list_formulas(pivot, &labels) {
        Ok(sheet) => {
            app.commit(t("List Formulas", "Liste des formules"), cx);
            app.switch_sheet(sheet, window, cx);
        }
        Err(err) => {
            app.wb.rollback();
            refuse(app, &err, cx);
        }
    }
}
