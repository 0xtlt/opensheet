//! The editing tools of the Home and Insert tabs: Find and Replace, Paste
//! Special, hyperlinks, cell styles and symbols.
//!
//! Each tool has a file of `edit_tools/`; this one holds the commands, the
//! hooks and the shortcuts.

mod find;
#[cfg(test)]
mod ui_tests;

use super::misc::{self, Kinds, Special};
use super::{Command, Cx, Feature};
use crate::app::ExcelApp;
use crate::i18n::t;
use gpui::{App, Context, KeyBinding, Stateful, Window, actions, div, prelude::*};

pub use find::find_bar_next;

pub struct EditTools;

commands! {
    /// What the user can ask of the editing tools.
    pub enum Cmd {
        // Home ▸ Find & Select
        Find = "search", t("Find…", "Rechercher…");
        Replace = "", t("Replace…", "Remplacer…");
        SelectFormulas = "", t("Formulas", "Formules");
        SelectNotes = "", t("Notes", "Notes");
        SelectCondFormats = "", t("Conditional Formatting", "Mise en forme conditionnelle");
        SelectConstants = "", t("Constants", "Constantes");
        SelectValidation = "", t("Data Validation", "Validation des données");
        SelectObjects = "", t("Select Objects", "Sélectionner les objets");
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
        // The tools work on the sheet, once the entry in progress is stored.
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Find => find::open(app, false, window, cx),
            Cmd::Replace => find::open(app, true, window, cx),
            Cmd::SelectFormulas => misc::select_special(app, Special::Formulas(Kinds::ALL), cx),
            Cmd::SelectNotes => misc::select_special(app, Special::Notes, cx),
            Cmd::SelectCondFormats => misc::select_special(app, Special::ConditionalFormats { same: false }, cx),
            Cmd::SelectConstants => misc::select_special(app, Special::Constants(Kinds::ALL), cx),
            Cmd::SelectValidation => misc::select_special(app, Special::DataValidation { same: false }, cx),
            Cmd::SelectObjects => misc::select_special(app, Special::Objects, cx),
        }
    }
}

impl Feature for EditTools {
    fn name(&self) -> &'static str {
        "edit-tools"
    }
}

/// An element the headless tests find by name (`debug_bounds`).
fn named(name: &'static str, element: impl IntoElement) -> gpui::Div {
    div().flex_none().debug_selector(|| name.to_string()).child(element)
}

// ---------------------------------------------------------------- shortcuts

actions!(edit_tools, [FindDialog, Replace]);

/// The shortcuts of the editing tools.
pub fn bind_keys(cx: &mut App) {
    let w = Some("Workbook");
    cx.bind_keys([KeyBinding::new("ctrl-f", FindDialog, w), KeyBinding::new("ctrl-h", Replace, w)]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &FindDialog, window, cx| app.run_command(Cmd::Find, window, cx)))
        .on_action(cx.listener(|app, _: &Replace, window, cx| app.run_command(Cmd::Replace, window, cx)))
}
