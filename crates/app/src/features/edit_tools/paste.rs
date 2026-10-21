//! Paste Special: the dialog, the entries of the Paste drop-down and Paste
//! Link. What each choice does to the cells is the engine's
//! (`paste_special.rs`); here is what is on the clipboard and what the
//! protection of the sheet is asked.

use super::named;
use crate::app::{ClipState, ExcelApp};
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::Modal;
use crate::widgets::{Button, Checkbox, GroupBox, ListBox, RadioGroup, dialog, label};
use gpui::{Context, SharedString, Window, div, prelude::*, px};
use opensheet_engine::ops::PasteMode;
use opensheet_engine::paste_special::{PasteOp, PasteSpecial, PasteWhat};

/// What Paste Special finds on the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// Cells copied in this workbook.
    Cells,
    /// Cells that were cut: they can only be moved, by a plain paste.
    CutCells,
    /// Text from another application.
    Text,
    Nothing,
}

pub(super) fn source(app: &ExcelApp, cx: &mut Cx) -> Source {
    let system = cx.read_from_clipboard().and_then(|item| item.text());
    match (&app.clip, system) {
        // The cells are still what the system clipboard holds.
        (Some(state), Some(text)) if state.clip.text == text => cells(state),
        (Some(state), None) => cells(state),
        (_, Some(_)) => Source::Text,
        (None, None) => Source::Nothing,
    }
}

fn cells(state: &ClipState) -> Source {
    if state.cut {
        Source::CutCells
    } else {
        Source::Cells
    }
}

/// Paste the copied cells on the selection as `spec` says, in one undo
/// step. Without copied cells this is a plain paste of what the clipboard
/// holds.
pub(super) fn paste(app: &mut ExcelApp, spec: PasteSpecial, cx: &mut Cx) {
    if source(app, cx) != Source::Cells {
        app.paste(PasteMode::All, cx);
        return;
    }
    let Some(state) = app.clip.take() else {
        return;
    };
    let dest = app.sel().current();
    let target = app
        .wb
        .paste_special_area(app.sheet, dest, &state.clip, &spec);
    let intent = match spec.what.mode() {
        PasteMode::Formats => Intent::FormatCells(target),
        PasteMode::Comments => Intent::EditComment,
        PasteMode::Validation => Intent::ChangeSheet,
        PasteMode::ColumnWidths => Intent::FormatColumns,
        _ => Intent::EditCells(target),
    };
    if !app.may(intent, cx) {
        app.clip = Some(state);
        return;
    }
    let written = app.wb.paste_special(app.sheet, dest, &state.clip, &spec);
    let cause = ChangeCause::Paste {
        // Turned cells are not where the features would look for what
        // travels with them.
        from: (!spec.transpose).then_some((state.clip.sheet, state.clip.rect)),
        cut: false,
        mode: spec.what.mode(),
    };
    app.clip = Some(state);
    app.select_rect(written, cx);
    app.commit_cells(t("Paste Special", "Collage spécial"), &[written], cause, cx);
}

/// Paste Link: formulas reading the copied cells.
pub(super) fn paste_link(app: &mut ExcelApp, cx: &mut Cx) {
    if source(app, cx) != Source::Cells {
        return;
    }
    let Some(state) = app.clip.take() else {
        return;
    };
    let dest = app.sel().current();
    let (rows, cols) = (state.clip.rows.len() as u32, state.clip.rect.cols());
    let target = opensheet_engine::Rect::new(
        dest.r0,
        dest.c0,
        (dest.r0 + rows - 1).min(opensheet_engine::MAX_ROWS - 1),
        (dest.c0 + cols - 1).min(opensheet_engine::MAX_COLS - 1),
    );
    if !app.may(Intent::EditCells(target), cx) {
        app.clip = Some(state);
        return;
    }
    let written = app.wb.paste_link(app.sheet, dest, &state.clip);
    app.clip = Some(state);
    app.select_rect(written, cx);
    let cause = ChangeCause::Paste {
        from: None,
        cut: false,
        mode: PasteMode::Formulas,
    };
    app.commit_cells(
        t("Paste Link", "Coller avec liaison"),
        &[written],
        cause,
        cx,
    );
}

// ------------------------------------------------------------------- dialog

/// Open the dialog that fits what the clipboard holds.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    match source(app, cx) {
        Source::Cells => {
            app.open_modal(window, cx, |modal, _, _| PasteSpecialDialog {
                modal,
                spec: PasteSpecial::default(),
            });
        }
        Source::Text => {
            app.open_modal(window, cx, |modal, _, _| PasteTextDialog {
                modal,
                choice: 0,
            });
        }
        // Cut cells move, whatever is asked; nothing to choose.
        Source::CutCells => app.paste(PasteMode::All, cx),
        Source::Nothing => {}
    }
}

/// The "Paste" choices in the order of the dialog: its left column, then
/// its right one.
pub(super) const CHOICES: [PasteWhat; 12] = [
    PasteWhat::All,
    PasteWhat::Formulas,
    PasteWhat::Values,
    PasteWhat::Formats,
    PasteWhat::Comments,
    PasteWhat::Validation,
    PasteWhat::AllUsingSourceTheme,
    PasteWhat::AllExceptBorders,
    PasteWhat::ColumnWidths,
    PasteWhat::FormulasAndNumberFormats,
    PasteWhat::ValuesAndNumberFormats,
    PasteWhat::AllMergingConditionalFormats,
];

fn choice_label(what: PasteWhat) -> &'static str {
    match what {
        PasteWhat::All => t("All", "Tout"),
        PasteWhat::Formulas => t("Formulas", "Formules"),
        PasteWhat::Values => t("Values", "Valeurs"),
        PasteWhat::Formats => t("Formats", "Formats"),
        PasteWhat::Comments => t("Comments and Notes", "Commentaires et notes"),
        PasteWhat::Validation => t("Validation", "Validation"),
        PasteWhat::AllUsingSourceTheme => t("All using Source theme", "Tout, avec le thème source"),
        PasteWhat::AllExceptBorders => t("All except borders", "Tout sauf la bordure"),
        PasteWhat::ColumnWidths => t("Column widths", "Largeurs de colonne"),
        PasteWhat::FormulasAndNumberFormats => {
            t("Formula and number formats", "Formats de nombre et formule")
        }
        PasteWhat::ValuesAndNumberFormats => {
            t("Values and number formats", "Valeurs et formats de nombre")
        }
        PasteWhat::AllMergingConditionalFormats => t(
            "All, merge conditional formats",
            "Tout, fusionner la mise en forme conditionnelle",
        ),
        PasteWhat::ValuesAndSourceFormats => {
            t("Values & Source Formatting", "Valeurs et format source")
        }
        PasteWhat::KeepSourceColumnWidths => t(
            "Keep Source Column Widths",
            "Conserver les largeurs de colonnes sources",
        ),
    }
}

const OPERATIONS: [PasteOp; 5] = [
    PasteOp::None,
    PasteOp::Add,
    PasteOp::Subtract,
    PasteOp::Multiply,
    PasteOp::Divide,
];

fn operation_label(op: PasteOp) -> &'static str {
    match op {
        PasteOp::None => t("None", "Aucun"),
        PasteOp::Add => t("Add", "Ajouter"),
        PasteOp::Subtract => t("Subtract", "Soustraction"),
        PasteOp::Multiply => t("Multiply", "Multiplication"),
        PasteOp::Divide => t("Divide", "Division"),
    }
}

/// Paste Link is offered for a plain paste of everything, as in Excel.
pub(super) fn can_link(spec: &PasteSpecial) -> bool {
    matches!(spec.what, PasteWhat::All | PasteWhat::AllExceptBorders)
        && spec.operation == PasteOp::None
        && !spec.skip_blanks
        && !spec.transpose
}

struct PasteSpecialDialog {
    modal: Modal,
    spec: PasteSpecial,
}

impl PasteSpecialDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let spec = self.spec;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| paste(app, spec, cx));
    }

    fn link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, paste_link);
    }

    /// One column of the "Paste" choices: `CHOICES[from..from + 6]`.
    fn column(&self, id: &'static str, from: usize, cx: &mut Context<Self>) -> RadioGroup {
        let labels: Vec<SharedString> = CHOICES[from..from + 6]
            .iter()
            .map(|what| choice_label(*what).into())
            .collect();
        let at = CHOICES.iter().position(|what| *what == self.spec.what);
        let selected = at
            .and_then(|at| at.checked_sub(from))
            .filter(|index| *index < 6);
        RadioGroup::new(id, labels, selected).on_change(cx.listener(
            move |this, index: &usize, _, cx| {
                this.spec.what = CHOICES[from + *index];
                cx.notify();
            },
        ))
    }

    /// One column of the "Operation" choices.
    fn operations(
        &self,
        id: &'static str,
        from: usize,
        count: usize,
        cx: &mut Context<Self>,
    ) -> RadioGroup {
        let labels: Vec<SharedString> = OPERATIONS[from..from + count]
            .iter()
            .map(|op| operation_label(*op).into())
            .collect();
        let at = OPERATIONS.iter().position(|op| *op == self.spec.operation);
        let selected = at
            .and_then(|at| at.checked_sub(from))
            .filter(|index| *index < count);
        RadioGroup::new(id, labels, selected).on_change(cx.listener(
            move |this, index: &usize, _, cx| {
                this.spec.operation = OPERATIONS[from + *index];
                cx.notify();
            },
        ))
    }
}

impl Render for PasteSpecialDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = |left: RadioGroup, right: RadioGroup| {
            div()
                .flex()
                .gap(px(24.))
                .child(div().w(px(200.)).child(left))
                .child(div().flex_1().child(right))
        };
        let modal = self.modal.clone();
        dialog(&self.modal, t("Paste Special", "Collage spécial"))
            .width(520.0)
            .on_ok(cx.listener(Self::ok))
            .child(GroupBox::new(t("Paste", "Coller")).child(columns(
                self.column("paste-left", 0, cx),
                self.column("paste-right", 6, cx),
            )))
            .child(GroupBox::new(t("Operation", "Opération")).child(columns(
                self.operations("op-left", 0, 3, cx),
                self.operations("op-right", 3, 2, cx),
            )))
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(
                        div().w(px(200.)).child(named(
                            "paste-skip-blanks",
                            Checkbox::new(
                                "skip-blanks",
                                t("Skip Blanks", "Blancs non compris"),
                                self.spec.skip_blanks,
                            )
                            .on_change(cx.listener(
                                |this, on: &bool, _, cx| {
                                    this.spec.skip_blanks = *on;
                                    cx.notify();
                                },
                            )),
                        )),
                    )
                    .child(named(
                        "paste-transpose",
                        Checkbox::new(
                            "transpose",
                            t("Transpose", "Transposer"),
                            self.spec.transpose,
                        )
                        .on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.spec.transpose = *on;
                                cx.notify();
                            },
                        )),
                    )),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(named(
                        "paste-link",
                        Button::new("link", t("Paste Link", "Coller avec liaison"))
                            .disabled(!can_link(&self.spec))
                            .on_click(cx.listener(|this, _, window, cx| this.link(window, cx))),
                    ))
                    .child(div().flex_1())
                    .child(
                        Button::new("cancel", t("Cancel", "Annuler"))
                            .on_click(move |_, window, cx| modal.close(window, cx)),
                    )
                    .child(named(
                        "paste-ok",
                        Button::new("ok", t("OK", "OK"))
                            .primary()
                            .on_click(cx.listener(Self::ok)),
                    )),
            )
    }
}

/// Paste Special for text copied in another application: the formats
/// Excel offers for it, which are one here.
struct PasteTextDialog {
    modal: Modal,
    choice: usize,
}

impl PasteTextDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| app.paste(PasteMode::All, cx));
    }
}

impl Render for PasteTextDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let formats: Vec<SharedString> = vec![
            t("Unicode Text", "Texte Unicode").into(),
            t("Text", "Texte").into(),
        ];
        let modal = self.modal.clone();
        dialog(&self.modal, t("Paste Special", "Collage spécial"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("As:", "En tant que :")))
            .child(
                div().flex().child(
                    ListBox::new("formats", formats, Some(self.choice))
                        .height(96.0)
                        .on_change(cx.listener(|this, index: &usize, _, cx| {
                            this.choice = *index;
                            cx.notify();
                        }))
                        .on_activate(
                            cx.listener(|this, _: &usize, window, cx| this.ok(&(), window, cx)),
                        ),
                ),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("cancel", t("Cancel", "Annuler"))
                            .on_click(move |_, window, cx| modal.close(window, cx)),
                    )
                    .child(
                        Button::new("ok", t("OK", "OK"))
                            .primary()
                            .on_click(cx.listener(Self::ok)),
                    ),
            )
    }
}
