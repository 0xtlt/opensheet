//! AutoFilter: the filter buttons of a range, their drop-downs, sorting
//! from them, and the custom sort dialog.
//!
//! Not written yet: the commands are in place on the ribbon, in the menus
//! and on the keyboard, and say so. See `features/mod.rs` for the hooks
//! (the buttons are zones painted over header cells, their drop-down a
//! view opened with `open_popup_view`, built with `widgets::CheckList`).

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Filters;

commands! {
    /// What the user can ask of sorting and filtering.
    pub enum Cmd {
        Toggle = "filter", t("Filter", "Filtrer");
        Clear = "clear", t("Clear", "Effacer");
        Reapply = "refresh", t("Reapply", "Réappliquer");
        Advanced = "filter", t("Advanced", "Avancé");
        Sort = "sort-filter", t("Sort", "Trier");
        CustomSort = "sort-filter", t("Custom Sort…", "Tri personnalisé…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
        app.not_implemented(self.label(), cx)
    }
}

impl Feature for Filters {
    fn name(&self) -> &'static str {
        "filters"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(
                MenuSection::Data,
                MenuEntry::submenu(
                    t("Filter", "Filtrer"),
                    vec![
                        MenuEntry::command(Cmd::Toggle, app),
                        MenuEntry::command(Cmd::Clear, app),
                        MenuEntry::command(Cmd::Reapply, app),
                    ],
                ),
            );
            menu.add(MenuSection::Data, MenuEntry::command(Cmd::CustomSort, app));
        }
    }
}
