//! The proofing tools of the Review tab: Spelling (F7), Thesaurus (⇧F7),
//! Check Accessibility and Translate.
//!
//! Each tool has its file. What they need from macOS (the spelling
//! checker, the thesaurus dictionaries, the translation) is in
//! `system.rs`, behind traits the tests replace.

mod accessibility;
mod spelling;
pub mod system;
mod thesaurus;
mod translate;
#[cfg(test)]
mod ui_tests;

use super::{CellEdit, Command, Cx, Feature, StatusItem, drawings};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::{App, Context, KeyBinding, Stateful, Window, actions, prelude::*};

pub struct Proofing;

commands! {
    /// What the user can ask of the proofing tools.
    pub enum Cmd {
        Spelling = "spell", t("Spelling", "Orthographe");
        Thesaurus = "book", t("Thesaurus", "Dictionnaire des synonymes");
        AccessibilityMenu = "eye", t("Check Accessibility", "Vérifier l'accessibilité");
        CheckAccessibility = "eye", t("Check Accessibility", "Vérifier l'accessibilité");
        Translate = "book", t("Translate", "Traduire");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        // The drop-down only opens: its entries come back here.
        if self == Cmd::AccessibilityMenu {
            let entries = vec![
                MenuEntry::command(Cmd::CheckAccessibility, app),
                MenuEntry::command(drawings::Cmd::AltText, app),
            ];
            app.open_command_menu(entries, 240.0, window, cx);
            return;
        }
        // The tools read the sheet: what is being typed is stored first.
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        app.popup = None;
        match self {
            Cmd::Spelling => spelling::start(app, window, cx),
            Cmd::Thesaurus => thesaurus::open(app, cx),
            Cmd::CheckAccessibility => accessibility::open(app, cx),
            Cmd::Translate => translate::open(app, cx),
            Cmd::AccessibilityMenu => {}
        }
    }
}

impl Feature for Proofing {
    fn name(&self) -> &'static str {
        "proofing"
    }

    fn status_items(&self, app: &ExcelApp) -> Vec<StatusItem> {
        accessibility::status_items(app)
    }

    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        translate::follow(app, cx);
    }

    fn cells_changed(&self, app: &mut ExcelApp, _edit: &CellEdit, cx: &mut Cx) {
        translate::follow(app, cx);
    }
}

actions!(proofing, [Spelling, Thesaurus]);

/// F7 and ⇧F7, as in Excel.
pub fn bind_keys(cx: &mut App) {
    let w = Some("Workbook");
    cx.bind_keys([
        KeyBinding::new("f7", Spelling, w),
        KeyBinding::new("shift-f7", Thesaurus, w),
    ]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(
        cx.listener(|app, _: &Spelling, window, cx| app.run_command(Cmd::Spelling, window, cx)),
    )
    .on_action(
        cx.listener(|app, _: &Thesaurus, window, cx| app.run_command(Cmd::Thesaurus, window, cx)),
    )
}
