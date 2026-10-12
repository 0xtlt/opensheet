//! The small dialogs of outlines: Group / Ungroup (Rows or Columns) and
//! the Settings behind the corner of the Outline group.

use super::{Cmd, apply_group, auto_outline};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::{t, t_ctx};
use crate::shell::Modal;
use crate::widgets::{Button, Checkbox, GroupBox, RadioGroup, dialog, ok_cancel};
use gpui::{Context, Entity, Window, div, prelude::*, px};
use opensheet_engine::outline::Axis;

// ---------------------------------------------------------- Group / Ungroup

/// Ask whether the rows or the columns of the selected cells are meant.
pub(super) fn open_group(app: &mut ExcelApp, ungroup: bool, window: &mut Window, cx: &mut Cx) -> Entity<GroupDialog> {
    app.open_modal(window, cx, move |modal, _, _| GroupDialog {
        modal,
        ungroup,
        axis: Axis::Rows,
    })
}

pub(super) struct GroupDialog {
    modal: Modal,
    ungroup: bool,
    axis: Axis,
}

impl GroupDialog {
    pub(super) fn choose(&mut self, axis: Axis, cx: &mut Context<Self>) {
        self.axis = axis;
        cx.notify();
    }

    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (axis, ungroup) = (self.axis, self.ungroup);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| apply_group(app, axis, ungroup, cx));
    }
}

impl Render for GroupDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.ungroup { Cmd::UngroupMenu.label() } else { Cmd::GroupMenu.label() };
        let options = vec![t("Rows", "Lignes").into(), t("Columns", "Colonnes").into()];
        let selected = if self.axis == Axis::Rows { 0 } else { 1 };
        dialog(&self.modal, title)
            .width(230.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                RadioGroup::new("outline-axis", options, Some(selected)).on_change(cx.listener(|this, index: &usize, _, cx| {
                    this.choose(if *index == 0 { Axis::Rows } else { Axis::Cols }, cx)
                })),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------------------------- Settings

pub(super) fn open_settings(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<SettingsDialog> {
    let outline = &app.wb.sheets[app.sheet].outline;
    let (below, right) = (outline.summary_below, outline.summary_right);
    app.open_modal(window, cx, move |modal, _, _| SettingsDialog { modal, below, right })
}

pub(super) struct SettingsDialog {
    modal: Modal,
    below: bool,
    right: bool,
}

/// Store where summaries lie; true when the sheet allows it.
fn store(app: &mut ExcelApp, below: bool, right: bool, cx: &mut Cx) -> bool {
    if !app.may(Intent::ChangeSheet, cx) {
        return false;
    }
    let sheet = app.sheet;
    let shown = app.wb.sheets[sheet].outline.show_symbols;
    app.wb.set_outline_settings(sheet, below, right, shown);
    true
}

impl SettingsDialog {
    pub(super) fn set(&mut self, below: bool, right: bool, cx: &mut Context<Self>) {
        self.below = below;
        self.right = right;
        cx.notify();
    }

    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (below, right) = (self.below, self.right);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if store(app, below, right, cx) {
                app.commit(t("Outline Settings", "Paramètres du plan"), cx);
            }
        });
    }

    /// Create: an automatic outline with these settings.
    pub(super) fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (below, right) = (self.below, self.right);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if store(app, below, right, cx) {
                auto_outline(app, cx);
                // Settings changed without an outline to make are kept too.
                app.commit(t("Outline Settings", "Paramètres du plan"), cx);
            }
        });
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (below, right) = (self.below, self.right);
        dialog(&self.modal, t_ctx("outline", "Settings", "Paramètres"))
            .width(430.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                GroupBox::new(t("Direction", "Sens"))
                    .child(
                        Checkbox::new(
                            "summary-below",
                            t("Summary rows below detail", "Lignes de synthèse sous les lignes de détail"),
                            below,
                        )
                        .on_change(cx.listener(move |this, on: &bool, _, cx| this.set(*on, this.right, cx))),
                    )
                    .child(
                        Checkbox::new(
                            "summary-right",
                            t(
                                "Summary columns to right of detail",
                                "Colonnes de synthèse à droite des colonnes de détail",
                            ),
                            right,
                        )
                        .on_change(cx.listener(move |this, on: &bool, _, cx| this.set(this.below, *on, cx))),
                    ),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("outline-create", t_ctx("outline", "Create", "Créer"))
                            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(ok_cancel(&self.modal, cx.listener(Self::ok))),
            )
    }
}
