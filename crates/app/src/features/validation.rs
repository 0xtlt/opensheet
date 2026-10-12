//! Data validation: the dialog, the in-cell drop-down list, the input
//! message, the error alerts, the circles around invalid data.
//!
//! Not written yet: the commands are in place and say so. See
//! `features/mod.rs` for the hooks (`may_commit` checks an entry and shows
//! the alert, `selection_changed` shows the input message, the drop-down
//! arrow is a zone painted beside the active cell).

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Validation;

commands! {
    /// What the user can ask of data validation.
    pub enum Cmd {
        Menu = "validation", t("Data Validation", "Validation des données");
        Open = "validation", t("Data Validation…", "Validation des données…");
        CircleInvalid = "", t("Circle Invalid Data", "Entourer les données non valides");
        ClearCircles = "", t("Clear Validation Circles", "Effacer les cercles de validation");
        PickFromList = "", t("Pick From Drop-down List…", "Liste déroulante de choix…");
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
            Cmd::Menu => {
                let entries = vec![
                    MenuEntry::command(Cmd::Open, app),
                    MenuEntry::command(Cmd::CircleInvalid, app),
                    MenuEntry::command(Cmd::ClearCircles, app),
                ];
                app.open_command_menu(entries, 270.0, window, cx);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

impl Feature for Validation {
    fn name(&self) -> &'static str {
        "validation"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::PickFromList, app));
        }
    }
}
