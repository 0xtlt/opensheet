//! The Subtotal dialog (Data > Subtotal).

use super::{Cmd, warn};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::t;
use crate::shell::Modal;
use crate::widgets::{Button, Checkbox, ScrollArea, Select, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::refs::col_to_letters;
use opensheet_engine::subtotal::{SubtotalError, SubtotalFn, SubtotalSpec};
use opensheet_engine::{Rect, Value, Workbook};

/// The names of the functions, in the order of `SubtotalFn::ALL`.
fn function_names() -> Vec<SharedString> {
    [
        t("Sum", "Somme"),
        t("Count", "Nombre"),
        t("Average", "Moyenne"),
        t("Max", "Max."),
        t("Min", "Min."),
        t("Product", "Produit"),
        t("Count Numbers", "Chiffres"),
        t("StdDev", "Écartype"),
        t("StdDevp", "Écartypep"),
        t("Var", "Var"),
        t("Varp", "Varp"),
    ]
    .into_iter()
    .map(SharedString::from)
    .collect()
}

/// The list Subtotal works on: the selection when it is a block of
/// several rows, else the cells around the active cell.
pub(super) fn list_for(
    wb: &Workbook,
    sheet: usize,
    selection: Rect,
    active: (u32, u32),
) -> Result<Rect, SubtotalError> {
    if selection.is_single() {
        return wb.subtotal_list(sheet, active.0, active.1);
    }
    if wb.sheets[sheet]
        .tables
        .iter()
        .any(|table| table.range.intersects(&selection))
    {
        return Err(SubtotalError::InTable);
    }
    let list = wb.clamp_to_used(sheet, selection);
    if list.rows() < 2 {
        Err(SubtotalError::NoList)
    } else {
        Ok(list)
    }
}

/// The labels of the list's columns; a column without one is named after
/// its letter, as Excel does.
pub(super) fn column_names(wb: &Workbook, sheet: usize, list: Rect) -> Vec<SharedString> {
    (list.c0..=list.c1)
        .map(|col| {
            let text = wb.display_text(sheet, list.r0, col).text;
            if text.is_empty() {
                format!("({} {})", t("Column", "Colonne"), col_to_letters(col)).into()
            } else {
                text.into()
            }
        })
        .collect()
}

/// What the dialog proposes, as Excel does: a subtotal of the last
/// column at each change in the first one, a sum when that column holds
/// numbers and a count otherwise.
pub(super) fn proposal(wb: &Workbook, sheet: usize, list: Rect) -> SubtotalSpec {
    let numbers = (list.r0 + 1..=list.r1)
        .any(|row| matches!(wb.value_at(sheet, row, list.c1), Value::Number(_)));
    SubtotalSpec {
        group_by: list.c0,
        function: if numbers {
            SubtotalFn::Sum
        } else {
            SubtotalFn::Count
        },
        columns: vec![list.c1],
        replace: true,
        page_breaks: false,
        summary_below: true,
    }
}

fn refusal(error: SubtotalError) -> &'static str {
    match error {
        SubtotalError::NoColumns => t(
            "Select at least one column to add the subtotal to.",
            "Sélectionnez au moins une colonne à laquelle ajouter le sous-total.",
        ),
        _ => t(
            "This can't be applied to the selected range. Select a single cell in a range and try again.",
            "Nous ne pouvons pas appliquer cette action à la plage sélectionnée. Sélectionnez une seule cellule dans une plage, puis réessayez.",
        ),
    }
}

/// Data > Subtotal: select the list and open the dialog on it.
pub(super) fn open(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<SubtotalDialog>> {
    if !app.may(Intent::ChangeSheet, cx) {
        return None;
    }
    let sheet = app.sheet;
    let list = match list_for(&app.wb, sheet, app.sel().current(), app.active()) {
        Ok(list) => list,
        Err(error) => {
            warn(app, refusal(error), cx);
            return None;
        }
    };
    app.select_rect(list, cx);
    let columns = column_names(&app.wb, sheet, list);
    let spec = proposal(&app.wb, sheet, list);
    Some(
        app.open_modal(window, cx, move |modal, _, _| SubtotalDialog {
            modal,
            list,
            columns,
            spec,
        }),
    )
}

pub(super) struct SubtotalDialog {
    modal: Modal,
    list: Rect,
    columns: Vec<SharedString>,
    pub(super) spec: SubtotalSpec,
}

impl SubtotalDialog {
    /// Change what the dialog holds.
    pub(super) fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut SubtotalSpec)) {
        change(&mut self.spec);
        cx.notify();
    }

    /// Tick or clear a column of "Add subtotal to".
    pub(super) fn tick(&mut self, col: u32, on: bool, cx: &mut Context<Self>) {
        self.edit(cx, |spec| {
            spec.columns.retain(|c| *c != col);
            if on {
                spec.columns.push(col);
                spec.columns.sort_unstable();
            }
        });
    }

    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (list, spec) = (self.list, self.spec.clone());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            match app.wb.subtotal(sheet, list, &spec) {
                Ok(done) => {
                    app.select_rect(done, cx);
                    app.commit(Cmd::Subtotal.label(), cx);
                }
                Err(error) => {
                    // Nothing of a refused request is kept.
                    app.wb.rollback();
                    warn(app, refusal(error), cx);
                }
            }
        });
    }

    pub(super) fn remove_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.list;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            let left = app.wb.remove_subtotals(sheet, list);
            app.select_rect(left, cx);
            app.commit(t("Remove Subtotals", "Supprimer les sous-totaux"), cx);
        });
    }
}

impl Render for SubtotalDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = self.spec.clone();
        let first = self.list.c0;
        let function = SubtotalFn::ALL.iter().position(|f| *f == spec.function);
        let ticks = self.columns.iter().enumerate().map(|(index, name)| {
            let col = first + index as u32;
            Checkbox::new(
                ("subtotal-column", index),
                name.clone(),
                spec.columns.contains(&col),
            )
            .on_change(cx.listener(move |this, on: &bool, _, cx| this.tick(col, *on, cx)))
        });
        let option = |id: &'static str,
                      text: &'static str,
                      on: bool,
                      set: fn(&mut SubtotalSpec, bool),
                      cx: &mut Context<Self>| {
            Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
                let on = *on;
                this.edit(cx, |spec| set(spec, on));
            }))
        };
        dialog(&self.modal, Cmd::Subtotal.label())
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("At each change in:", "À chaque changement de :")))
            .child(
                Select::new(
                    "subtotal-group-by",
                    self.columns.clone(),
                    Some((spec.group_by - first) as usize),
                )
                .width(300.0)
                .on_change(cx.listener(move |this, index: &usize, _, cx| {
                    let col = first + *index as u32;
                    this.edit(cx, |spec| spec.group_by = col);
                })),
            )
            .child(label(t("Use function:", "Utiliser la fonction :")))
            .child(
                Select::new("subtotal-function", function_names(), function)
                    .width(300.0)
                    .on_change(cx.listener(|this, index: &usize, _, cx| {
                        let function = SubtotalFn::ALL[*index];
                        this.edit(cx, |spec| spec.function = function);
                    })),
            )
            .child(label(t("Add subtotal to:", "Ajouter un sous-total à :")))
            .child(
                div()
                    .border_1()
                    .border_color(crate::theme::c(crate::theme::BORDER_STRONG))
                    .px(px(6.))
                    .child(
                        ScrollArea::new("subtotal-columns", 96.0).child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .py(px(4.))
                                .children(ticks),
                        ),
                    ),
            )
            .child(option(
                "subtotal-replace",
                t(
                    "Replace current subtotals",
                    "Remplacer les sous-totaux existants",
                ),
                spec.replace,
                |spec, on| spec.replace = on,
                cx,
            ))
            .child(option(
                "subtotal-breaks",
                t(
                    "Page break between groups",
                    "Saut de page entre les groupes",
                ),
                spec.page_breaks,
                |spec, on| spec.page_breaks = on,
                cx,
            ))
            .child(option(
                "subtotal-below",
                t("Summary below data", "Synthèse sous les données"),
                spec.summary_below,
                |spec, on| spec.summary_below = on,
                cx,
            ))
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("subtotal-remove", t("Remove All", "Supprimer tout")).on_click(
                            cx.listener(|this, _, window, cx| this.remove_all(window, cx)),
                        ),
                    )
                    .child(div().flex_1())
                    .child(ok_cancel(&self.modal, cx.listener(Self::ok))),
            )
    }
}
