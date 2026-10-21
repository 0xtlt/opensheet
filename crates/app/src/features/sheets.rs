//! Hidden sheets: Hide and Unhide… in the menu of a sheet tab and in
//! Home ▸ Format, and the Unhide dialog of Excel for Mac.
//!
//! The engine keeps the state of each sheet (`Sheet::visibility`) and
//! refuses to hide the last one that has a tab. A hidden sheet has no
//! tab and cannot come in front; formulas, charts and scripts go on
//! reading it. A very hidden sheet (only scripts make one) is not offered
//! by Unhide….

use super::{Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::widgets::{ListBox, dialog, label, ok_cancel};
use gpui::{Context, SharedString, Window, prelude::*};
use opensheet_engine::workbook::SheetVisibility;

pub struct Sheets;

commands! {
    /// What the user can ask about the tabs of sheets.
    pub enum Cmd {
        Hide = "", t("Hide Sheet", "Masquer la feuille");
        Unhide = "", t("Unhide Sheet…", "Afficher la feuille…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    /// Unhide… needs a sheet to offer.
    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::Hide => true,
            Cmd::Unhide => !hidden_sheets(app).is_empty(),
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        app.popup = None;
        match self {
            Cmd::Hide => {
                hide(app, app.sheet, cx);
            }
            Cmd::Unhide => {
                if !app.may(Intent::UnhideSheet, cx) {
                    return;
                }
                let sheets = hidden_sheets(app);
                app.open_modal(window, cx, |modal, _, _| UnhideDialog::new(modal, sheets));
            }
        }
    }
}

impl Feature for Sheets {
    fn name(&self) -> &'static str {
        "sheets"
    }

    /// The menu of a tab says Hide and Unhide…, as Excel's: the tab says
    /// which sheet.
    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::SheetTab(_)) {
            let entry = |label: &'static str, cmd: Cmd| {
                MenuEntry::new(
                    label,
                    move |a: &mut ExcelApp, window: &mut Window, cx: &mut Cx| {
                        cmd.run(a, window, cx)
                    },
                )
                .disabled(!cmd.enabled(app))
            };
            menu.add(MenuSection::Format, entry(t("Hide", "Masquer"), Cmd::Hide));
            menu.add(
                MenuSection::Format,
                entry(t("Unhide…", "Afficher…"), Cmd::Unhide),
            );
        }
    }
}

/// The sheets Unhide… offers, in the order of the workbook: index and
/// name. Very hidden sheets are not among them.
pub(crate) fn hidden_sheets(app: &ExcelApp) -> Vec<(usize, String)> {
    app.wb
        .sheets
        .iter()
        .enumerate()
        .filter(|(_, sheet)| sheet.visibility == SheetVisibility::Hidden)
        .map(|(index, sheet)| (index, sheet.name.clone()))
        .collect()
}

/// The sheets that have a tab, in the order of the tabs.
pub(crate) fn tabs(app: &ExcelApp) -> Vec<usize> {
    (0..app.wb.sheets.len())
        .filter(|index| app.wb.sheets[*index].is_visible())
        .collect()
}

/// The tab next to the sheet in front: the one after it or the one
/// before it, the sheet itself at either end of the strip.
pub(crate) fn neighbour(app: &ExcelApp, forward: bool) -> usize {
    app.wb
        .next_visible_sheet(app.sheet, forward)
        .unwrap_or(app.sheet)
}

/// Excel's refusal to hide or delete the last sheet that has a tab.
pub(crate) fn last_visible_message() -> &'static str {
    t(
        "A workbook must contain at least one visible worksheet. \n\nTo hide, delete, or move the selected sheet(s), you must first insert a new sheet or unhide a sheet that is already hidden.",
        "Un classeur doit contenir au moins une feuille visible. \n\nPour masquer, supprimer ou déplacer les feuilles sélectionnées, insérez d'abord une nouvelle feuille ou affichez une feuille masquée.",
    )
}

/// Hide a sheet, as one undo step. The next tab comes in front. False
/// when protection or the last visible sheet refused, after the message.
pub(crate) fn hide(app: &mut ExcelApp, index: usize, cx: &mut Cx) -> bool {
    if !app.may(Intent::HideSheet, cx) {
        return false;
    }
    if !app.wb.set_sheet_visibility(index, SheetVisibility::Hidden) {
        app.alert(
            Alert::new("OpenSheet", last_visible_message()).warning(),
            cx,
        );
        return false;
    }
    app.deselect_object(cx);
    // `refresh` brings the nearest tab in front.
    app.commit(t("Hide Sheet", "Masquer la feuille"), cx);
    true
}

/// Give sheets their tabs back, as one undo step; the first of them
/// comes in front.
pub(crate) fn unhide(app: &mut ExcelApp, sheets: &[usize], cx: &mut Cx) -> bool {
    if sheets.is_empty() || !app.may(Intent::UnhideSheet, cx) {
        return false;
    }
    for index in sheets {
        app.wb
            .set_sheet_visibility(*index, SheetVisibility::Visible);
    }
    app.deselect_object(cx);
    app.sheet = sheets[0].min(app.wb.sheets.len() - 1);
    app.commit(t("Unhide Sheet", "Afficher la feuille"), cx);
    true
}

// ------------------------------------------------------------------- Unhide

/// Excel for Mac's Unhide: the hidden sheets, of which one or more are
/// chosen.
struct UnhideDialog {
    modal: Modal,
    sheets: Vec<(usize, String)>,
    selected: Vec<usize>,
}

impl UnhideDialog {
    fn new(modal: Modal, sheets: Vec<(usize, String)>) -> Self {
        UnhideDialog {
            modal,
            selected: if sheets.is_empty() {
                Vec::new()
            } else {
                vec![0]
            },
            sheets,
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let chosen: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|row| self.sheets.get(*row))
            .map(|(index, _)| *index)
            .collect();
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| unhide(app, &chosen, cx));
    }
}

impl Render for UnhideDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<SharedString> = self
            .sheets
            .iter()
            .map(|(_, name)| SharedString::from(name.clone()))
            .collect();
        dialog(&self.modal, t("Unhide", "Afficher"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Unhide one or more sheets:",
                "Afficher une ou plusieurs feuilles :",
            )))
            .child(
                ListBox::multi("unhide-list", items, self.selected.clone())
                    .height(180.0)
                    .on_change_multi(cx.listener(|this, rows: &Vec<usize>, _, cx| {
                        this.selected = rows.clone();
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, row: &usize, window, cx| {
                        this.selected = vec![*row];
                        this.ok(&(), window, cx);
                    })),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod ui_tests;
