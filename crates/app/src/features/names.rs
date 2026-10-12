//! Defined names: the Name Manager, defining and applying names, creating
//! them from the selection, using them in a formula.
//!
//! Not written yet: the commands are in place and say so. See
//! `features/mod.rs` (the dialogs are opened with `open_modal` and built
//! with `widgets`; `widgets::RefEdit` is the "Refers to" field).

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Names;

commands! {
    /// What the user can ask of defined names.
    pub enum Cmd {
        Manager = "tag", t("Name Manager", "Gestionnaire de noms");
        DefineMenu = "tag", t("Define Name", "Définir un nom");
        Define = "tag", t("Define Name…", "Définir un nom…");
        Apply = "", t("Apply Names…", "Appliquer les noms…");
        UseInFormula = "function", t("Use in Formula", "Dans une formule");
        CreateFromSelection = "grid", t("Create from Selection", "Depuis sélection");
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
        match self {
            Cmd::DefineMenu => {
                let entries = vec![MenuEntry::command(Cmd::Define, app), MenuEntry::command(Cmd::Apply, app)];
                app.open_command_menu(entries, 200.0, window, cx);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

impl Feature for Names {
    fn name(&self) -> &'static str {
        "names"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Define, app));
        }
    }
}
