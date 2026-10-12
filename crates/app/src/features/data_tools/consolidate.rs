//! Data ▸ Consolidate: the dialog. The engine (`consolidate.rs`) combines
//! the ranges and keeps the settings on the sheet, so that the dialog
//! opens on them again.

use super::{typed_range, warn};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::{t, t_ctx};
use crate::shell::{Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{Button, Checkbox, ListBox, RefEdit, Select, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::consolidate::{ConsolidateError, ConsolidateFunction, ConsolidateSource, Consolidation};

/// References as the dialog writes them: `Sheet1!$A$1:$C$9`.
const REFERENCE: RefStyle = RefStyle {
    absolute: true,
    sheet: true,
    equals: false,
};

fn function_name(function: ConsolidateFunction) -> &'static str {
    match function {
        ConsolidateFunction::Sum => t("Sum", "Somme"),
        ConsolidateFunction::Count => t_ctx("consolidate", "Count", "Nombre"),
        ConsolidateFunction::Average => t("Average", "Moyenne"),
        ConsolidateFunction::Max => t("Max", "Max"),
        ConsolidateFunction::Min => t("Min", "Min"),
        ConsolidateFunction::Product => t("Product", "Produit"),
        ConsolidateFunction::CountNumbers => t_ctx("consolidate", "Count Numbers", "Chiffres"),
        ConsolidateFunction::StdDev => t("StdDev", "Ecartype"),
        ConsolidateFunction::StdDevP => t("StdDevp", "Ecartypep"),
        ConsolidateFunction::Var => t("Var", "Var"),
        ConsolidateFunction::VarP => t("Varp", "Varp"),
    }
}

fn error_text(error: ConsolidateError) -> &'static str {
    match error {
        ConsolidateError::NoSource => t(
            "No source references specified for consolidation.",
            "Pas de référence source spécifiée pour la consolidation.",
        ),
        ConsolidateError::OffSheet => t("No data was consolidated.", "Aucune donnée n'a été consolidée."),
        ConsolidateError::Overlap => t("Source reference overlaps destination area.", "La référence source se superpose à la zone de destination."),
        ConsolidateError::LinkToSelf => t(
            "Cannot create links to consolidation sheet.",
            "Impossible de créer des liaisons à la feuille de consolidation.",
        ),
    }
}

/// A reference typed in the dialog as a source, or `None` when it is not
/// one range.
fn source(app: &ExcelApp, text: &str) -> Option<ConsolidateSource> {
    let (sheet, range) = typed_range(app, text)?;
    Some(ConsolidateSource {
        sheet: app.wb.sheets.get(sheet)?.name.clone(),
        range,
    })
}

fn source_text(source: &ConsolidateSource) -> String {
    REFERENCE.format(source.range, &source.sheet)
}

/// OK: consolidate into the selection. False when it cannot be done; the
/// user has been told why and the dialog stays open.
pub(super) fn apply(app: &mut ExcelApp, spec: &Consolidation, cx: &mut Cx) -> bool {
    let (sheet, dest) = (app.sheet, app.sel().current());
    let outcome = app.wb.consolidate_extent(sheet, dest, spec).and_then(|extent| {
        if !app.may(Intent::EditCells(extent), cx) {
            return Ok(false);
        }
        let written = app.wb.consolidate(sheet, dest, spec)?;
        app.commit_cells(t("Consolidate", "Consolider"), &[written], ChangeCause::Fill, cx);
        Ok(true)
    });
    match outcome {
        Ok(done) => done,
        Err(error) => {
            warn(app, error_text(error), cx);
            false
        }
    }
}

/// Data ▸ Consolidate: the dialog opens on what was consolidated into
/// this sheet last.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<ConsolidateDialog> {
    let spec = app.wb.sheets[app.sheet].consolidation.clone().unwrap_or_default();
    app.open_modal(window, cx, move |modal, _, cx| ConsolidateDialog {
        reference: modal.ref_input(REFERENCE, "", cx),
        modal,
        spec,
        selected: None,
    })
}

pub(super) struct ConsolidateDialog {
    modal: Modal,
    spec: Consolidation,
    reference: Entity<TextInput>,
    /// The line of "All references" that is selected.
    selected: Option<usize>,
}

impl ConsolidateDialog {
    #[cfg(test)]
    pub(super) fn set_reference(&mut self, text: &str, cx: &mut Context<Self>) {
        self.reference.update(cx, |field, cx| field.set_text(text, cx));
    }

    #[cfg(test)]
    pub(super) fn spec_mut(&mut self) -> &mut Consolidation {
        &mut self.spec
    }

    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        // The reference shows in the box, where it can be corrected.
        if let Some(source) = self.spec.sources.get(index) {
            let text = source_text(source);
            self.reference.update(cx, |field, cx| field.set_text(text, cx));
        }
        cx.notify();
    }

    /// Add: the reference of the box joins the list (once). False when it
    /// is not a reference; the user has been told.
    pub(super) fn add(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.reference.read(cx).text().to_string();
        let found = self.modal.read_app(cx, |app| source(app, &text)).flatten();
        match found {
            Some(source) => {
                if !self.spec.sources.contains(&source) {
                    self.spec.sources.push(source);
                }
                self.selected = None;
                cx.notify();
                true
            }
            None => {
                self.modal.update_app(cx, |app, cx| {
                    warn(app, t("Consolidation reference is not valid.", "Référence de consolidation non valide."), cx)
                });
                false
            }
        }
    }

    /// Delete: the selected reference leaves the list.
    pub(super) fn delete(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected.filter(|index| *index < self.spec.sources.len()) {
            self.spec.sources.remove(index);
            self.selected = None;
            self.reference.update(cx, |field, cx| field.set_text("", cx));
            cx.notify();
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        // A reference still in the box counts, as in Excel.
        if !self.reference.read(cx).text().trim().is_empty() && !self.add(cx) {
            return;
        }
        let spec = self.spec.clone();
        if self.modal.update_app(cx, |app, cx| apply(app, &spec, cx)) == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for ConsolidateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let functions: Vec<SharedString> = ConsolidateFunction::ALL.iter().map(|f| function_name(*f).into()).collect();
        let function = ConsolidateFunction::ALL.iter().position(|f| *f == self.spec.function);
        let references: Vec<SharedString> = self.spec.sources.iter().map(|source| source_text(source).into()).collect();
        let side = |button: Button| div().w(px(90.)).flex_none().child(button.width(90.0));
        dialog(&self.modal, t("Consolidate", "Consolider"))
            .width(470.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Function:", "Fonction :")))
            .child(
                Select::new("function", functions, function).width(180.0).on_change(cx.listener(|this, index: &usize, _, cx| {
                    this.spec.function = ConsolidateFunction::ALL[(*index).min(ConsolidateFunction::ALL.len() - 1)];
                    cx.notify();
                })),
            )
            .child(label(t("Reference:", "Référence :")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(RefEdit::new(&self.reference, &self.modal).width(330.0)))
                    .child(side(Button::new("add", t("Add", "Ajouter")).on_click(cx.listener(|this, _, _, cx| {
                        this.add(cx);
                    })))),
            )
            .child(label(t("All references:", "Toutes les références :")))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(
                        div().flex_1().child(
                            ListBox::new("references", references, self.selected)
                                .width(346.0)
                                .height(crate::widgets::ROW_H * 5.0)
                                .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx))),
                        ),
                    )
                    .child(side(
                        Button::new("delete", t("Delete", "Supprimer"))
                            .disabled(self.selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    )),
            )
            .child(label(t("Use labels in:", "Étiquettes dans :")))
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(Checkbox::new("top", t("Top row", "ligne du haut"), self.spec.top_row).on_change(cx.listener(
                        |this, on: &bool, _, cx| {
                            this.spec.top_row = *on;
                            cx.notify();
                        },
                    )))
                    .child(Checkbox::new("left", t("Left column", "colonne de gauche"), self.spec.left_column).on_change(
                        cx.listener(|this, on: &bool, _, cx| {
                            this.spec.left_column = *on;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                Checkbox::new("links", t("Create links to source data", "Lier aux données sources"), self.spec.link).on_change(
                    cx.listener(|this, on: &bool, _, cx| {
                        this.spec.link = *on;
                        cx.notify();
                    }),
                ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
