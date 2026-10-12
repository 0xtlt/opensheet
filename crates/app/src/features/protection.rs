//! Protection of sheets and of the workbook structure: the dialogs, and
//! refusing the changes a protected sheet does not allow.
//!
//! Not written yet, except "Lock Cell": the commands are in place and say
//! so. See `features/mod.rs` for the hooks (`guard` is asked before every
//! change, with an `Intent`; the dialogs are opened with `open_modal`).

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Protection;

commands! {
    /// What the user can ask of protection.
    pub enum Cmd {
        ProtectSheet = "lock", t("Protect Sheet", "Protéger la feuille");
        ProtectWorkbook = "lock", t("Protect Workbook", "Protéger le classeur");
        AllowEditRanges = "lock", t("Allow Edit Ranges", "Autoriser la modification des plages");
        UnshareWorkbook = "share", t("Unshare Workbook", "Annuler le partage du classeur");
        LockCell = "lock", t("Lock Cell", "Verrouiller la cellule");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn checked(self, app: &ExcelApp) -> bool {
        self == Cmd::LockCell && app.active_style().locked
    }

    fn enabled(self, _app: &ExcelApp) -> bool {
        // A workbook is never shared here.
        self != Cmd::UnshareWorkbook
    }

    fn run(self, app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::LockCell => {
                let locked = !app.active_style().locked;
                app.apply_style(self.label(), cx, move |s| s.locked = locked);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

impl Feature for Protection {
    fn name(&self) -> &'static str {
        "protection"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::SheetTab(_)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::ProtectSheet, app));
        }
    }
}
