//! The dialogs of form controls: Format Control (its Control tab, which
//! differs with the kind of control, and Properties), Assign Macro and
//! Edit Text.

use super::{Controls, selected};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent, automate, object_id};
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::widgets::{self, Checkbox, GroupBox, ListBox, RadioGroup, RefEdit, TabStrip, TextField, dialog, label, labeled, ok_cancel};
use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Window, div, prelude::*, px};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::control::{CheckState, ControlKind, FormControl, MAX_CONTROL_VALUE, SelectionType};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::Locale;

fn warn(app: &mut ExcelApp, message: &'static str, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// True when the selected control may be changed.
fn may_edit(app: &mut ExcelApp, id: u32, cx: &mut Cx) -> bool {
    app.may(Intent::EditObject(Some(object_id(&Controls, id))), cx)
}

/// A reference of a control as a dialog shows it: `$A$1`, with its sheet
/// when it is another one.
fn reference_text(r: Option<&RefExpr>) -> String {
    r.map(RefExpr::to_text).unwrap_or_default()
}

/// What a reference typed in a dialog stands for; `Ok(None)` for an empty
/// box, `Err` for text that is no reference to cells.
pub(super) fn typed_reference(app: &ExcelApp, text: &str) -> Result<Option<RefExpr>, ()> {
    let text = text.trim().trim_start_matches('=');
    if text.is_empty() {
        return Ok(None);
    }
    let (row, col) = app.active();
    let areas = app.wb.reference_areas(app.sheet, row, col, text).ok_or(())?;
    let [area] = areas.as_slice() else {
        return Err(());
    };
    // As files keep it: absolute, naming its sheet when it is another.
    let style = RefStyle {
        absolute: true,
        sheet: area.sheet != app.sheet,
        equals: false,
    };
    match parse_formula(&style.format(area.rect, &app.wb.sheets[area.sheet].name), Locale::en()) {
        Ok(Expr::Ref(r)) => Ok(Some(r)),
        _ => Err(()),
    }
}

// ----------------------------------------------------------- Format Control

const POSITIONING: [(bool, bool); 3] = [(true, true), (true, false), (false, false)];

pub(crate) struct FormatDialog {
    modal: Modal,
    id: u32,
    kind: ControlKind,
    /// The tab shown: Control (when the kind has one), then Properties.
    pub(super) tab: usize,
    pub(super) checked: CheckState,
    pub(super) shaded: bool,
    pub(super) link: Entity<TextInput>,
    pub(super) input: Entity<TextInput>,
    /// Current, minimum and maximum value, incremental and page change.
    pub(super) numbers: [Entity<TextInput>; 5],
    pub(super) drop_lines: Entity<TextInput>,
    pub(super) selection: SelectionType,
    /// "Object positioning": a place in [`POSITIONING`].
    pub(super) positioning: usize,
    pub(super) print: bool,
}

/// True for the kinds whose Format Control has a Control tab.
fn has_control_tab(kind: ControlKind) -> bool {
    !matches!(kind, ControlKind::Button | ControlKind::Label)
}

pub(super) fn open_format(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<FormatDialog>> {
    let control = selected(app)?.clone();
    if !may_edit(app, control.id, cx) {
        return None;
    }
    // The dialog shows the control as it shows on the sheet.
    let now = app.wb.control_state(app.sheet, &control);
    let positioning = POSITIONING.iter().position(|p| *p == (control.move_with_cells, control.size_with_cells)).unwrap_or(1);
    Some(app.open_modal(window, cx, move |modal, _, cx| {
        let number = |value: i32, cx: &mut Context<FormatDialog>| widgets::input(cx, value.to_string());
        FormatDialog {
            id: control.id,
            kind: control.kind,
            tab: 0,
            checked: now.checked,
            shaded: control.shaded,
            link: modal.ref_input(RefStyle::absolute(), reference_text(control.link.as_ref()), cx),
            input: modal.ref_input(RefStyle::absolute(), reference_text(control.input.as_ref()), cx),
            numbers: [
                number(now.value, cx),
                number(control.min, cx),
                number(control.max, cx),
                number(control.inc, cx),
                number(control.page, cx),
            ],
            drop_lines: widgets::input(cx, control.drop_lines.to_string()),
            selection: control.selection,
            positioning,
            print: control.print,
            modal,
        }
    }))
}

impl FormatDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let read = |field: &Entity<TextInput>, cx: &Context<Self>| field.read(cx).text().trim().to_string();
        let (link, input) = (read(&self.link, cx), read(&self.input, cx));
        let numbers: Vec<Option<i32>> = self.numbers.iter().map(|field| read(field, cx).parse::<i32>().ok().filter(|n| (0..=MAX_CONTROL_VALUE).contains(n))).collect();
        let drop_lines = read(&self.drop_lines, cx).parse::<u32>().ok().filter(|n| (1..=MAX_CONTROL_VALUE as u32).contains(n));
        let (id, kind) = (self.id, self.kind);
        let (checked, shaded, selection, print) = (self.checked, self.shaded, self.selection, self.print);
        let (moves, sizes) = POSITIONING[self.positioning.min(2)];
        let done = self.modal.update_app(cx, |app, cx| {
            let (Ok(link), Ok(input)) = (typed_reference(app, &link), typed_reference(app, &input)) else {
                warn(app, t("Reference is not valid.", "La référence n'est pas valide."), cx);
                return false;
            };
            let valued = kind.has_value();
            if (valued && numbers.iter().any(Option::is_none)) || (kind == ControlKind::ComboBox && drop_lines.is_none()) {
                warn(
                    app,
                    t("Scroll value must be between 0 and 30000.", "La valeur de défilement doit être comprise entre 0 et 30000."),
                    cx,
                );
                return false;
            }
            let numbers: Vec<i32> = numbers.into_iter().flatten().collect();
            app.wb.update_control(id, |control| {
                control.move_with_cells = moves;
                control.size_with_cells = sizes;
                control.print = print;
                if !has_control_tab(kind) {
                    return;
                }
                control.shaded = shaded;
                if kind.has_link() {
                    control.link = link;
                }
                if kind.has_list() {
                    control.input = input;
                    control.selection = selection;
                }
                if let (ControlKind::ComboBox, Some(lines)) = (kind, drop_lines) {
                    control.drop_lines = lines;
                }
                if valued {
                    control.min = numbers[1];
                    control.max = numbers[2];
                    control.inc = numbers[3];
                    control.page = numbers[4];
                }
            });
            // The state goes to the control and to its cell, as a click
            // would put it.
            match kind {
                ControlKind::CheckBox => {
                    app.wb.set_control_checked(id, checked);
                }
                ControlKind::OptionButton if checked == CheckState::Checked => {
                    app.wb.choose_option(id);
                }
                ControlKind::OptionButton => {
                    app.wb.update_control(id, |control| control.checked = CheckState::Unchecked);
                }
                ControlKind::SpinButton | ControlKind::ScrollBar => {
                    app.wb.set_control_value(id, numbers[0]);
                }
                _ => {}
            }
            app.commit(t("Format Control", "Format de contrôle"), cx);
            true
        });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }

    fn field(text: &'static str, control: impl IntoElement) -> gpui::Div {
        div().flex().items_center().gap(px(8.)).child(label(text).w(px(150.))).child(control)
    }

    fn control_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let kind = self.kind;
        let mut column = div().flex().flex_col().gap(px(10.));
        if matches!(kind, ControlKind::CheckBox | ControlKind::OptionButton) {
            let mut states: Vec<SharedString> = vec![t("Unchecked", "Non activée").into(), t("Checked", "Activée").into()];
            if kind == ControlKind::CheckBox {
                states.push(t("Mixed", "Mixte").into());
            }
            const ORDER: [CheckState; 3] = [CheckState::Unchecked, CheckState::Checked, CheckState::Mixed];
            let chosen = ORDER.iter().position(|state| *state == self.checked).filter(|k| *k < states.len());
            column = column.child(GroupBox::new(t_ctx("control", "Value", "Valeur")).child(RadioGroup::new("control-state", states, chosen).on_change(cx.listener(
                |this, index: &usize, _, cx| {
                    this.checked = ORDER[*index];
                    cx.notify();
                },
            ))));
        }
        if kind.has_value() {
            let labels = [
                t("Current value:", "Valeur actuelle :"),
                t("Minimum value:", "Valeur minimale :"),
                t("Maximum value:", "Valeur maximale :"),
                t("Incremental change:", "Changement de pas :"),
                t("Page change:", "Changement de page :"),
            ];
            // A spin button has no page to turn.
            let shown = if kind == ControlKind::ScrollBar { 5 } else { 4 };
            for (text, number) in labels.into_iter().zip(&self.numbers).take(shown) {
                column = column.child(Self::field(text, TextField::new(number).width(90.0)));
            }
        }
        if kind.has_list() {
            column = column.child(Self::field(t("Input range:", "Plage d’entrée :"), RefEdit::new(&self.input, &self.modal).width(200.0)));
        }
        if kind.has_link() {
            column = column.child(Self::field(t("Cell link:", "Cellule liée :"), RefEdit::new(&self.link, &self.modal).width(200.0)));
        }
        if kind == ControlKind::ComboBox {
            column = column.child(Self::field(t("Drop down lines:", "Nombre de lignes :"), TextField::new(&self.drop_lines).width(90.0)));
        }
        if kind == ControlKind::ListBox {
            const ORDER: [SelectionType; 3] = [SelectionType::Single, SelectionType::Multi, SelectionType::Extend];
            let types: Vec<SharedString> = vec![
                t_ctx("control", "Single", "Simple").into(),
                t_ctx("control", "Multi", "Multiple").into(),
                t_ctx("control", "Extend", "Étendue").into(),
            ];
            let chosen = ORDER.iter().position(|kind| *kind == self.selection);
            column = column.child(GroupBox::new(t("Selection type", "Type de sélection")).child(RadioGroup::new("control-selection", types, chosen).on_change(
                cx.listener(|this, index: &usize, _, cx| {
                    this.selection = ORDER[*index];
                    cx.notify();
                }),
            )));
        }
        column
            .child(Checkbox::new("control-shaded", t("3-D shading", "Ombrage 3D"), self.shaded).on_change(cx.listener(|this, on: &bool, _, cx| {
                this.shaded = *on;
                cx.notify();
            })))
            .into_any_element()
    }

    fn properties_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let options: Vec<SharedString> = vec![
            t("Move and size with cells", "Déplacer et dimensionner avec les cellules").into(),
            t("Move but don't size with cells", "Déplacer sans dimensionner avec les cellules").into(),
            t("Don't move or size with cells", "Ne pas déplacer ou dimensionner avec les cellules").into(),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(GroupBox::new(t("Object positioning", "Positionnement de l'objet")).child(RadioGroup::new("control-positioning", options, Some(self.positioning)).on_change(
                cx.listener(|this, index: &usize, _, cx| {
                    this.positioning = *index;
                    cx.notify();
                }),
            )))
            .child(Checkbox::new("control-print", t("Print object", "Imprimer l'objet"), self.print).on_change(cx.listener(|this, on: &bool, _, cx| {
                this.print = *on;
                cx.notify();
            })))
            .into_any_element()
    }
}

impl Render for FormatDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let control_tab = has_control_tab(self.kind);
        let mut tabs: Vec<SharedString> = Vec::new();
        if control_tab {
            tabs.push(t_ctx("control", "Control", "Contrôle").into());
        }
        tabs.push(t_ctx("control", "Properties", "Propriétés").into());
        let tab = self.tab.min(tabs.len() - 1);
        let body = if control_tab && tab == 0 { self.control_tab(cx) } else { self.properties_tab(cx) };
        dialog(&self.modal, t("Format Control", "Format de contrôle"))
            .width(440.0)
            .on_ok(cx.listener(Self::ok))
            .child(TabStrip::new("control-tabs", tabs, tab).on_change(cx.listener(|this, index: &usize, _, cx| {
                this.tab = *index;
                cx.notify();
            })))
            .child(div().min_h(px(210.)).pt(px(6.)).child(body))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------- Assign Macro

pub(crate) struct AssignDialog {
    modal: Modal,
    id: u32,
    pub(super) name: Entity<TextInput>,
    /// The macros of the workbook, by name.
    pub(super) macros: Vec<String>,
    _subscription: Subscription,
}

pub(super) fn open_assign_macro(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<AssignDialog>> {
    let control = selected(app)?;
    let (id, current) = (control.id, control.macro_short_name().unwrap_or_default().to_string());
    if !may_edit(app, id, cx) {
        return None;
    }
    let macros: Vec<String> = match automate::vba_project(app) {
        Some(Ok(project)) => project.macros().into_iter().map(|found| found.name).collect(),
        _ => Vec::new(),
    };
    Some(app.open_modal(window, cx, move |modal, _, cx| {
        let name = widgets::input(cx, current);
        let subscription = cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                cx.notify();
            }
        });
        AssignDialog {
            modal,
            id,
            name,
            macros,
            _subscription: subscription,
        }
    }))
}

impl AssignDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_string();
        let id = self.id;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            // As files name a macro of the workbook itself; an empty box
            // takes the macro away.
            let assigned = (!name.is_empty()).then(|| if name.contains('!') { name.clone() } else { format!("[0]!{name}") });
            if app.wb.sheets[app.sheet].control(id).is_some_and(|control| control.macro_name != assigned) {
                app.wb.update_control(id, |control| control.macro_name = assigned);
                app.commit(t("Assign Macro", "Affecter une macro"), cx);
            }
        });
    }
}

impl Render for AssignDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let typed = self.name.read(cx).text().trim().to_lowercase();
        let chosen = self.macros.iter().position(|name| name.to_lowercase() == typed);
        let names: Vec<SharedString> = self.macros.iter().map(|name| name.clone().into()).collect();
        dialog(&self.modal, t("Assign Macro", "Affecter une macro"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label(t("Macro name:", "Nom de la macro :")))
                    .child(div().flex().child(TextField::new(&self.name).width(336.0)))
                    .child(
                        ListBox::new("assign-macros", names, chosen)
                            .width(336.0)
                            .height(140.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                if let Some(name) = this.macros.get(*index).cloned() {
                                    this.name.update(cx, |input, cx| input.set_text(name, cx));
                                    cx.notify();
                                }
                            }))
                            .on_activate(cx.listener(|this, index: &usize, window, cx| {
                                if let Some(name) = this.macros.get(*index).cloned() {
                                    this.name.update(cx, |input, cx| input.set_text(name, cx));
                                    this.ok(&(), window, cx);
                                }
                            })),
                    ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ---------------------------------------------------------------- Edit Text

pub(crate) struct TextDialog {
    modal: Modal,
    id: u32,
    pub(super) text: Entity<TextInput>,
}

pub(super) fn open_edit_text(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<TextDialog>> {
    let control: &FormControl = selected(app).filter(|control| control.kind.has_text())?;
    let (id, text) = (control.id, control.text.clone());
    if !may_edit(app, id, cx) {
        return None;
    }
    Some(app.open_modal(window, cx, move |modal, _, cx| TextDialog {
        modal,
        id,
        text: widgets::input(cx, text),
    }))
}

impl TextDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).text().to_string();
        let id = self.id;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if app.wb.sheets[app.sheet].control(id).is_some_and(|control| control.text != text) {
                app.wb.update_control(id, |control| control.text = text);
                app.commit(t("Edit Text", "Modifier le texte"), cx);
            }
        });
    }
}

impl Render for TextDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Edit Text", "Modifier le texte"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(labeled(t("Text:", "Texte :"), TextField::new(&self.text).width(280.0)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
