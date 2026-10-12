//! The Data Validation dialog: Settings, Input Message, Error Alert.
//!
//! It opens on the rule of the selection. When the selected cells do not
//! all have the same rule, Excel's question comes first: whether to extend
//! the rule to the cells without one, or to erase the rules and start
//! afresh.

use super::textarea::{TextArea, text_box};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::c;
use crate::widgets::{self, Button, Checkbox, RefEdit, Select, TabStrip, TextField, dialog, label};
use gpui::{AnyElement, Context, Entity, FocusHandle, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::validation::{
    AlertStyle, Field, FormulaWarning, MAX_MESSAGE_LEN, MAX_TITLE_LEN, Operator, SelectionValidation, Validation,
    ValidationKind, operand_fields,
};
use opensheet_engine::{Locale, Rect};

/// What the dialog opens on.
#[derive(Clone)]
struct Opening {
    sheet: usize,
    /// The cells OK gives the rule to. The formulas read from `active`.
    ranges: Vec<Rect>,
    active: (u32, u32),
    /// The rule shown: that of the selection, or a new one.
    rule: Validation,
    /// The other cells that have this very rule.
    alike: Vec<Rect>,
    locale: &'static Locale,
}

/// The parts of the selected ranges that lie in `cells`.
fn overlap(ranges: &[Rect], cells: &[Rect]) -> Vec<Rect> {
    ranges
        .iter()
        .flat_map(|range| cells.iter().filter_map(|other| range.intersection(other)))
        .collect()
}

/// Open the dialog on the selection, after the question its rules call for.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let sheet = app.sheet;
    let locale = app.wb.locale;
    let ranges = app.sel().ranges.clone();
    let found = app.wb.selection_validation(sheet, &ranges);
    let question = found.question(locale);
    let rule = match found {
        SelectionValidation::Uniform(rule) | SelectionValidation::Partial(rule) => Some(rule.clone()),
        SelectionValidation::None | SelectionValidation::Mixed => None,
    };
    let partial = matches!(found, SelectionValidation::Partial(_));
    // Every cell that has the rule shown, selected or not.
    let alike = match rule.as_ref().and_then(|rule| rule.anchor()) {
        Some((row, col)) => app.wb.validation_ranges_like(sheet, row, col),
        None => Vec::new(),
    };
    let opening = Opening {
        sheet,
        ranges,
        active: app.active(),
        rule: rule.unwrap_or_default(),
        alike,
        locale,
    };
    let show = |opening: Opening| {
        move |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| {
            let opening = opening.clone();
            app.open_modal(window, cx, move |modal, _, cx| Dialog::new(modal, opening, cx));
        }
    };
    match question {
        None => show(opening)(app, window, cx),
        // Extend the rule to the whole selection, or keep to its cells.
        Some(question) if partial => {
            let mut narrowed = opening.clone();
            narrowed.ranges = overlap(&opening.ranges, &opening.alike);
            app.alert(
                Alert::new("OpenSheet", question)
                    .warning()
                    .button(t("Yes", "Oui"), show(opening))
                    .button(t("No", "Non"), show(narrowed))
                    .close_button(t("Cancel", "Annuler")),
                cx,
            );
        }
        // Different rules: start from none.
        Some(question) => app.alert(
            Alert::new("OpenSheet", question)
                .warning()
                .button(t("OK", "OK"), show(opening))
                .close_button(t("Cancel", "Annuler")),
            cx,
        ),
    }
}

/// Give the cells their rule, as one undo step.
fn apply(app: &mut ExcelApp, sheet: usize, ranges: &[Rect], active: (u32, u32), rule: Validation, cx: &mut Cx) {
    // Nothing to remove and nothing to set: no step to undo.
    let untouched = rule.is_noop() && matches!(app.wb.selection_validation(sheet, ranges), SelectionValidation::None);
    if !untouched && app.wb.set_validation(sheet, ranges, active, rule) {
        app.commit(t("Data Validation", "Validation des données"), cx);
    }
}

/// The label of one of the boxes that take a value or a formula.
fn field_label(field: Field) -> &'static str {
    match field {
        Field::Minimum => t("Minimum:", "Minimum :"),
        Field::Maximum => t("Maximum:", "Maximum :"),
        Field::Formula => t("Formula:", "Formule :"),
        Field::Source => t("Source:", "Source :"),
        Field::StartDate => t("Start date:", "Date de début :"),
        Field::EndDate => t("End date:", "Date de fin :"),
        Field::StartTime => t("Start time:", "Heure de début :"),
        Field::EndTime => t("End time:", "Heure de fin :"),
        Field::Value => t("Value:", "Valeur :"),
        Field::Date => t("Date:", "Date :"),
        Field::Time => t("Time:", "Heure :"),
        Field::Length => t("Length:", "Longueur :"),
    }
}

/// What a text box keeps of what is typed in it: `max` characters.
fn clipped(text: &str, max: usize) -> Option<String> {
    (text.chars().count() > max).then(|| text.chars().take(max).collect())
}

/// Height of a tab page: the dialog keeps its size from tab to tab.
const PAGE_HEIGHT: f32 = 258.0;

/// Height of the boxes the two messages are typed in.
const MESSAGE_HEIGHT: f32 = 92.0;

const TAB_SETTINGS: usize = 0;
const TAB_INPUT: usize = 1;
const TAB_ERROR: usize = 2;

struct Dialog {
    modal: Modal,
    open: Opening,
    tab: usize,
    // Settings
    kind: ValidationKind,
    operator: Operator,
    /// The boxes filling the first and the second formula of the rule.
    first: Entity<TextInput>,
    second: Entity<TextInput>,
    allow_blank: bool,
    in_cell_dropdown: bool,
    /// "Apply these changes to all other cells with the same settings".
    apply_to_alike: bool,
    // Input Message
    show_input: bool,
    input_title: Entity<TextInput>,
    input_message: Entity<TextArea>,
    // Error Alert
    show_error: bool,
    error_style: AlertStyle,
    error_title: Entity<TextInput>,
    error_message: Entity<TextArea>,
    /// Focus of the dialog frame, to close it from a message box.
    frame: FocusHandle,
}

impl Dialog {
    fn new(modal: Modal, open: Opening, cx: &mut Context<Self>) -> Self {
        let rule = &open.rule;
        let (row, col) = open.active;
        // References picked on the sheet are written `=$A$1:$A$5`.
        let reference = RefStyle {
            absolute: true,
            sheet: false,
            equals: true,
        };
        let first = modal.ref_input(reference, rule.operand1_text(row, col, open.locale), cx);
        let second = modal.ref_input(reference, rule.operand2_text(row, col, open.locale), cx);
        // A title of 32 characters at most, as Excel takes.
        let title = |text: &str, cx: &mut Context<Self>| {
            let field = widgets::input(cx, text);
            cx.subscribe(&field, |_, field, event: &InputEvent, cx| {
                if *event != InputEvent::Changed {
                    return;
                }
                if let Some(kept) = clipped(field.read(cx).text(), MAX_TITLE_LEN) {
                    field.update(cx, |field, cx| field.set_text(kept, cx));
                }
            })
            .detach();
            field
        };
        let message = |text: &str, cx: &mut Context<Self>| {
            let area = cx.new(|cx| TextArea::new(text, MAX_MESSAGE_LEN, cx));
            cx.observe(&area, |_, _, cx| cx.notify()).detach();
            area
        };
        Dialog {
            frame: modal.focus.clone(),
            tab: TAB_SETTINGS,
            kind: rule.kind,
            operator: rule.operator,
            first,
            second,
            allow_blank: rule.allow_blank,
            in_cell_dropdown: rule.in_cell_dropdown,
            apply_to_alike: false,
            show_input: rule.show_input,
            input_title: title(&rule.input_title, cx),
            input_message: message(&rule.input_message, cx),
            show_error: rule.show_error,
            error_style: rule.error_style,
            error_title: title(&rule.error_title, cx),
            error_message: message(&rule.error_message, cx),
            modal,
            open,
        }
    }

    /// The rule the dialog describes, or the message of what is wrong
    /// with its boxes.
    fn rule(&self, cx: &Context<Self>) -> Result<Validation, String> {
        let text = |field: &Entity<TextInput>| field.read(cx).text().to_string();
        let mut rule = Validation {
            kind: self.kind,
            operator: if self.kind.has_operator() { self.operator } else { Operator::default() },
            allow_blank: self.allow_blank,
            in_cell_dropdown: self.in_cell_dropdown,
            show_input: self.show_input,
            input_title: text(&self.input_title),
            input_message: self.input_message.read(cx).text().to_string(),
            show_error: self.show_error,
            error_style: self.error_style,
            error_title: text(&self.error_title),
            error_message: self.error_message.read(cx).text().to_string(),
            // Nothing in the dialog changes these.
            ime_mode: self.open.rule.ime_mode,
            ranges: Vec::new(),
            formula1: None,
            formula2: None,
        };
        rule.set_operands(&text(&self.first), &text(&self.second), self.open.locale)
            .map_err(|error| error.message(self.open.locale))?;
        Ok(rule)
    }

    /// The cells the rule goes to.
    fn targets(&self) -> Vec<Rect> {
        let mut ranges = self.open.ranges.clone();
        if self.apply_to_alike {
            ranges.extend(self.open.alike.iter().copied());
        }
        ranges
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let rule = match self.rule(cx) {
            Ok(rule) => rule,
            Err(message) => {
                self.tab = TAB_SETTINGS;
                cx.notify();
                self.modal
                    .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
                return;
            }
        };
        let (sheet, active, locale) = (self.open.sheet, self.open.active, self.open.locale);
        let ranges = self.targets();
        let warning = self
            .modal
            .read_app(cx, |app| app.wb.rule_formula_warning(sheet, active, &rule))
            .flatten();
        match warning {
            None => {
                self.modal
                    .update_app(cx, |app, cx| apply(app, sheet, &ranges, active, rule, cx));
                self.modal.close(window, cx);
            }
            Some(FormulaWarning::UnknownName) => {
                let message = FormulaWarning::UnknownName.message(locale);
                self.modal
                    .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
            }
            // A formula that fails for now: the user decides.
            Some(warning) => {
                let frame = self.frame.clone();
                let go_on = move |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| {
                    apply(app, sheet, &ranges, active, rule.clone(), cx);
                    app.close_modal_of(&frame, window, cx);
                };
                self.modal.update_app(cx, |app, cx| {
                    app.alert(
                        Alert::new("OpenSheet", warning.message(locale))
                            .warning()
                            .button(t("Yes", "Oui"), go_on)
                            .close_button(t("No", "Non")),
                        cx,
                    )
                });
            }
        }
    }

    /// Back to no rule at all, on the three tabs; OK then removes the
    /// rules of the cells.
    fn clear_all(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        let fresh = Validation::default();
        self.kind = fresh.kind;
        self.operator = fresh.operator;
        self.allow_blank = fresh.allow_blank;
        self.in_cell_dropdown = fresh.in_cell_dropdown;
        self.show_input = fresh.show_input;
        self.show_error = fresh.show_error;
        self.error_style = fresh.error_style;
        for field in [&self.first, &self.second, &self.input_title, &self.error_title] {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        for area in [&self.input_message, &self.error_message] {
            area.update(cx, |area, cx| area.set_text("", cx));
        }
        cx.notify();
    }

    /// A label over a control.
    fn titled(title: &'static str, control: impl IntoElement) -> gpui::Div {
        div().flex().flex_col().gap(px(3.)).child(label(title)).child(control)
    }

    /// The heading of a group of controls, and the controls under it.
    fn section(title: &'static str) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(label(title).font_weight(FontWeight::SEMIBOLD))
    }

    fn settings_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.open.locale;
        let kinds: Vec<SharedString> = ValidationKind::ALL.iter().map(|kind| kind.label(locale).into()).collect();
        let operators: Vec<SharedString> = Operator::ALL.iter().map(|op| op.label(locale).into()).collect();
        let kind = ValidationKind::ALL.iter().position(|k| *k == self.kind);
        let operator = Operator::ALL.iter().position(|op| *op == self.operator);
        let (field1, field2) = operand_fields(self.kind, self.operator);
        let restricted = self.kind != ValidationKind::Any;
        let boxes = [(field1, &self.first), (field2, &self.second)]
            .into_iter()
            .filter_map(|(field, input)| Some(Self::titled(field_label(field?), RefEdit::new(input, &self.modal))));
        Self::section(t("Validation criteria", "Critères de validation"))
            .flex_1()
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(Self::titled(
                                t("Allow:", "Autoriser :"),
                                Select::new("allow", kinds, kind).width(190.0).on_change(cx.listener(
                                    |this, ix: &usize, _, cx| {
                                        this.kind = ValidationKind::ALL[*ix];
                                        cx.notify();
                                    },
                                )),
                            ))
                            .child(Self::titled(
                                t("Data:", "Données :"),
                                Select::new("data", operators, operator)
                                    .width(190.0)
                                    .disabled(!self.kind.has_operator())
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        this.operator = Operator::ALL[*ix];
                                        cx.notify();
                                    })),
                            )),
                    )
                    .child(
                        div()
                            .pt(px(17.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                Checkbox::new("blank", t("Ignore blank", "Ignorer si vide"), self.allow_blank)
                                    .disabled(!restricted)
                                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                                        this.allow_blank = *on;
                                        cx.notify();
                                    })),
                            )
                            .when(self.kind == ValidationKind::List, |d| {
                                d.child(
                                    Checkbox::new(
                                        "dropdown",
                                        t("In-cell dropdown", "Liste déroulante dans la cellule"),
                                        self.in_cell_dropdown,
                                    )
                                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                                        this.in_cell_dropdown = *on;
                                        cx.notify();
                                    })),
                                )
                            }),
                    ),
            )
            .child(div().pl(px(10.)).flex().flex_col().gap(px(8.)).children(boxes))
            .child(div().flex_1())
            .child(
                Checkbox::new(
                    "alike",
                    t(
                        "Apply these changes to all other cells with the same settings",
                        "Appliquer ces modifications aux cellules dont les paramètres sont identiques",
                    ),
                    self.apply_to_alike,
                )
                // Only a rule that other cells share can be changed for them too.
                .disabled(self.open.alike.is_empty())
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.apply_to_alike = *on;
                    cx.notify();
                })),
            )
            .into_any_element()
    }

    fn input_tab(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                Checkbox::new(
                    "show-input",
                    t(
                        "Show input message when cell is selected",
                        "Afficher ce message de saisie quand la cellule est sélectionnée",
                    ),
                    self.show_input,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.show_input = *on;
                    cx.notify();
                })),
            )
            .child(
                Self::section(t(
                    "When cell is selected, show this input message:",
                    "Message de saisie à afficher quand la cellule est sélectionnée :",
                ))
                .child(
                    div()
                        .pl(px(10.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(Self::titled(
                            t("Title:", "Titre :"),
                            div().flex().child(TextField::new(&self.input_title)),
                        ))
                        .child(Self::titled(
                            t("Input message:", "Message de saisie :"),
                            div().flex().child(text_box(&self.input_message, MESSAGE_HEIGHT, window, cx)),
                        )),
                ),
            )
            .into_any_element()
    }

    fn error_tab(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.open.locale;
        let styles: Vec<SharedString> = AlertStyle::ALL.iter().map(|style| style.label(locale).into()).collect();
        let style = AlertStyle::ALL.iter().position(|s| *s == self.error_style);
        // The sign the alert will show.
        let (glyph, color) = match self.error_style {
            AlertStyle::Stop => ("×", 0xC42B1C),
            AlertStyle::Warning => ("!", 0xC19C00),
            AlertStyle::Information => ("i", 0x0F6CBD),
        };
        let sign = div()
            .size(px(32.))
            .flex_none()
            .rounded_full()
            .bg(c(color))
            .text_color(c(0xFFFFFF))
            .text_size(px(19.))
            .font_weight(FontWeight::BOLD)
            .flex()
            .items_center()
            .justify_center()
            .child(glyph);
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                Checkbox::new(
                    "show-error",
                    t(
                        "Show error alert after invalid data is entered",
                        "Afficher un message d’erreur lorsque des données incorrectes sont entrées",
                    ),
                    self.show_error,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.show_error = *on;
                    cx.notify();
                })),
            )
            .child(
                Self::section(t(
                    "When user enters invalid data, show this error alert:",
                    "Quand des données non valides sont saisies, afficher ce message d’erreur :",
                ))
                .child(
                    div()
                        .pl(px(10.))
                        .flex()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap(px(10.))
                                .child(Self::titled(
                                    t("Style:", "Style :"),
                                    Select::new("style", styles, style).width(130.0).on_change(cx.listener(
                                        |this, ix: &usize, _, cx| {
                                            this.error_style = AlertStyle::ALL[*ix];
                                            cx.notify();
                                        },
                                    )),
                                ))
                                .child(div().pl(px(4.)).child(sign)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(Self::titled(
                                    t("Title:", "Titre :"),
                                    div().flex().child(TextField::new(&self.error_title)),
                                ))
                                .child(Self::titled(
                                    t("Error message:", "Message d’erreur :"),
                                    div().flex().child(text_box(&self.error_message, MESSAGE_HEIGHT, window, cx)),
                                )),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = [
            t("Settings", "Paramètres"),
            t("Input Message", "Message de saisie"),
            t("Error Alert", "Alerte d’erreur"),
        ]
        .iter()
        .map(|name| (*name).into())
        .collect();
        let body = match self.tab {
            TAB_INPUT => self.input_tab(window, cx),
            TAB_ERROR => self.error_tab(window, cx),
            _ => self.settings_tab(cx),
        };
        let modal = self.modal.clone();
        dialog(&self.modal, t("Data Validation", "Validation des données"))
            .width(500.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                TabStrip::new("tabs", tabs, self.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.tab = *ix;
                    cx.notify();
                })),
            )
            .child(div().h(px(PAGE_HEIGHT)).flex().flex_col().child(body))
            .footer(
                div()
                    .flex_1()
                    .flex()
                    .justify_between()
                    .child(Button::new("clear-all", t("Clear All", "Effacer tout")).on_click(cx.listener(Self::clear_all)))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(Button::new("ok", t("OK", "OK")).primary().on_click(cx.listener(Self::ok)))
                            .child(
                                Button::new("cancel", t("Cancel", "Annuler"))
                                    .on_click(move |_, window, cx| modal.close(window, cx)),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    #[test]
    fn the_rule_stays_on_its_cells_when_not_extended() {
        // A2:B4 selected, the rule on B1:B3 and D4: "No" keeps to B2:B3.
        let kept = overlap(&[rect("A2:B4")], &[rect("B1:B3"), rect("D4")]);
        assert_eq!(kept, [rect("B2:B3")]);
        // Two selected ranges.
        let kept = overlap(&[rect("A1:A9"), rect("C1:C9")], &[rect("A5:C6")]);
        assert_eq!(kept, [rect("A5:A6"), rect("C5:C6")]);
        assert!(overlap(&[rect("A1")], &[rect("B2")]).is_empty());
    }

    #[test]
    fn titles_hold_what_excel_holds() {
        assert_eq!(clipped("short", MAX_TITLE_LEN), None);
        let long = "é".repeat(MAX_TITLE_LEN + 3);
        assert_eq!(clipped(&long, MAX_TITLE_LEN), Some("é".repeat(MAX_TITLE_LEN)));
        assert_eq!(clipped(&"x".repeat(MAX_TITLE_LEN), MAX_TITLE_LEN), None);
    }

    #[test]
    fn every_box_has_its_label() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        use ValidationKind::*;
        let labels = |kind, operator| {
            let (first, second) = operand_fields(kind, operator);
            (first.map(field_label), second.map(field_label))
        };
        assert_eq!(labels(Any, Operator::Between), (None, None));
        assert_eq!(labels(Whole, Operator::Between), (Some("Minimum:"), Some("Maximum:")));
        assert_eq!(labels(Decimal, Operator::LessThan), (Some("Maximum:"), None));
        assert_eq!(labels(List, Operator::Between), (Some("Source:"), None));
        assert_eq!(labels(Date, Operator::NotBetween), (Some("Start date:"), Some("End date:")));
        assert_eq!(labels(Date, Operator::Equal), (Some("Date:"), None));
        assert_eq!(labels(Time, Operator::GreaterOrEqual), (Some("Start time:"), None));
        assert_eq!(labels(Time, Operator::LessOrEqual), (Some("End time:"), None));
        assert_eq!(labels(TextLength, Operator::NotEqual), (Some("Length:"), None));
        assert_eq!(labels(Custom, Operator::Between), (Some("Formula:"), None));
    }
}
