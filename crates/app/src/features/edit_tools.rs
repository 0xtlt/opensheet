//! The editing tools of the Home and Insert tabs: Find and Replace, Paste
//! Special, hyperlinks, cell styles and symbols.
//!
//! Each tool has a file of `edit_tools/`; this one holds the commands, the
//! hooks and the shortcuts.

mod find;
mod links;
mod paste;
mod styles;
#[cfg(test)]
mod ui_tests;

use super::misc::{self, Kinds, Special};
use super::{
    CellEdit, Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget, Painter, Zone, ZoneEvent, ZoneResponse,
};
use crate::app::{EditMode, EditSource, ExcelApp, MenuEntry};
use crate::i18n::t;
use opensheet_engine::ops::PasteMode;
use opensheet_engine::paste_special::{PasteSpecial, PasteWhat};
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
        // Home ▸ Paste
        PasteMenu = "paste", t("Paste", "Coller");
        PasteSpecial = "", t("Paste Special…", "Collage spécial…");
        PasteFormulas = "", t("Formulas", "Formules");
        PasteFormulasNumbers = "", t("Formulas & Number Formatting", "Formules et format de nombre");
        PasteKeepSource = "", t("Keep Source Formatting", "Conserver la mise en forme source");
        PasteNoBorders = "", t("No Borders", "Aucune bordure");
        PasteKeepWidths = "", t("Keep Source Column Widths", "Conserver les largeurs de colonnes sources");
        PasteTranspose = "", t("Transpose", "Transposer");
        PasteValues = "", t("Paste Values", "Coller les valeurs");
        PasteValuesNumbers = "", t("Values & Number Formatting", "Valeurs et format de nombre");
        PasteValuesSource = "", t("Values & Source Formatting", "Valeurs et format source");
        PasteFormatting = "", t("Formatting", "Mise en forme");
        PasteLink = "", t("Paste Link", "Coller avec liaison");
        // Insert ▸ Link, the context menu of a cell, Home ▸ Clear
        Link = "link", t("Link", "Lien");
        EditLink = "link", t("Edit Link…", "Modifier le lien…");
        OpenLink = "", t("Open Link", "Ouvrir le lien");
        RemoveLink = "", t("Remove Link", "Supprimer le lien");
        ClearHyperlinks = "", t("Clear Hyperlinks", "Effacer les liens hypertexte");
        RemoveHyperlinks = "", t("Remove Hyperlinks", "Supprimer les liens hypertexte");
        // Home ▸ Cell Styles
        CellStyles = "cell-styles", t("Cell Styles", "Styles de cellules");
        NewCellStyle = "", t("New Cell Style…", "Nouveau style de cellule…");
        // Insert ▸ Symbol
        Symbol = "more", t("Symbol", "Symbole");
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
        // A symbol goes into the entry being typed; the other tools work
        // on the sheet, once that entry is stored.
        if self != Cmd::Symbol && app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Symbol => insert_symbol(app, window, cx),
            Cmd::Find => find::open(app, false, window, cx),
            Cmd::Replace => find::open(app, true, window, cx),
            Cmd::SelectFormulas => misc::select_special(app, Special::Formulas(Kinds::ALL), cx),
            Cmd::SelectNotes => misc::select_special(app, Special::Notes, cx),
            Cmd::SelectCondFormats => misc::select_special(app, Special::ConditionalFormats { same: false }, cx),
            Cmd::SelectConstants => misc::select_special(app, Special::Constants(Kinds::ALL), cx),
            Cmd::SelectValidation => misc::select_special(app, Special::DataValidation { same: false }, cx),
            Cmd::SelectObjects => app.toggle_object_pointer(cx),
            Cmd::PasteMenu => {
                let mut entries = vec![
                    MenuEntry::new(t("Paste", "Coller"), |a, _, cx| a.paste(PasteMode::All, cx)).icon("paste").shortcut("⌘V"),
                    MenuEntry::separator(),
                ];
                entries.extend(PASTE_MENU.iter().map(|cmd| MenuEntry::command(*cmd, app)));
                entries.push(MenuEntry::separator());
                entries.push(MenuEntry::command(Cmd::PasteSpecial, app).shortcut("⌃⌘V"));
                app.open_command_menu(entries, 330.0, window, cx);
            }
            Cmd::Link | Cmd::EditLink => links::open(app, window, cx),
            Cmd::OpenLink => {
                let (row, col) = app.active();
                links::follow(app, row, col, window, cx);
            }
            Cmd::RemoveLink | Cmd::RemoveHyperlinks => links::remove(app, true, cx),
            Cmd::ClearHyperlinks => links::remove(app, false, cx),
            Cmd::CellStyles => styles::open_gallery(app, window, cx),
            Cmd::NewCellStyle => styles::open_dialog(app, styles::Purpose::New, None, window, cx),
            Cmd::PasteSpecial => paste::open(app, window, cx),
            Cmd::PasteLink => paste::paste_link(app, cx),
            Cmd::PasteFormulas
            | Cmd::PasteFormulasNumbers
            | Cmd::PasteKeepSource
            | Cmd::PasteNoBorders
            | Cmd::PasteKeepWidths
            | Cmd::PasteTranspose
            | Cmd::PasteValues
            | Cmd::PasteValuesNumbers
            | Cmd::PasteValuesSource
            | Cmd::PasteFormatting => {
                if let Some(spec) = self.paste_spec() {
                    paste::paste(app, spec, cx);
                }
            }
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            // The choices of the drop-down are for cells that were copied
            // (cut cells can only be moved).
            Cmd::PasteLink => app.clip.as_ref().is_some_and(|state| !state.cut),
            _ if self.paste_spec().is_some() => app.clip.as_ref().is_some_and(|state| !state.cut),
            Cmd::ClearHyperlinks | Cmd::RemoveHyperlinks => {
                let links = &app.wb.sheets[app.sheet].hyperlinks;
                app.sel().ranges.iter().any(|range| links.iter().any(|link| link.range.intersects(range)))
            }
            _ => true,
        }
    }
}

/// Insert ▸ Symbol: the Emoji & Symbols viewer of the system, as in Excel
/// for Mac. What is picked there is typed into the cell being edited; a
/// cell that was only selected is opened for it.
fn insert_symbol(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if !app.is_editing() {
        app.start_edit(None, EditMode::Edit, EditSource::Cell, window, cx);
    }
    if !app.is_editing() {
        // The cell may not be changed (a protected sheet said so).
        return;
    }
    app.focus_editor(window, cx);
    // The headless window of the tests has no such viewer.
    #[cfg(not(test))]
    window.show_character_palette();
}

/// The entries of the Paste drop-down after "Paste", in Excel's order.
const PASTE_MENU: [Cmd; 11] = [
    Cmd::PasteFormulas,
    Cmd::PasteFormulasNumbers,
    Cmd::PasteKeepSource,
    Cmd::PasteNoBorders,
    Cmd::PasteKeepWidths,
    Cmd::PasteTranspose,
    Cmd::PasteValues,
    Cmd::PasteValuesNumbers,
    Cmd::PasteValuesSource,
    Cmd::PasteFormatting,
    Cmd::PasteLink,
];

impl Cmd {
    /// What an entry of the Paste drop-down pastes.
    fn paste_spec(self) -> Option<PasteSpecial> {
        let what = match self {
            Cmd::PasteFormulas => PasteWhat::Formulas,
            Cmd::PasteFormulasNumbers => PasteWhat::FormulasAndNumberFormats,
            Cmd::PasteKeepSource | Cmd::PasteTranspose => PasteWhat::All,
            Cmd::PasteNoBorders => PasteWhat::AllExceptBorders,
            Cmd::PasteKeepWidths => PasteWhat::KeepSourceColumnWidths,
            Cmd::PasteValues => PasteWhat::Values,
            Cmd::PasteValuesNumbers => PasteWhat::ValuesAndNumberFormats,
            Cmd::PasteValuesSource => PasteWhat::ValuesAndSourceFormats,
            Cmd::PasteFormatting => PasteWhat::Formats,
            _ => return None,
        };
        Some(PasteSpecial {
            what,
            transpose: self == Cmd::PasteTranspose,
            ..PasteSpecial::default()
        })
    }
}

impl Feature for EditTools {
    fn name(&self) -> &'static str {
        "edit-tools"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..) | MenuTarget::RowHeader(_) | MenuTarget::ColHeader(_)) {
            menu.add(MenuSection::Clipboard, MenuEntry::command(Cmd::PasteSpecial, app).shortcut("⌃⌘V"));
        }
        if let MenuTarget::Cell(row, col) = target {
            if app.wb.link_action_at(app.sheet, *row, *col).is_none() {
                menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Link, app).shortcut("⌘K"));
                return;
            }
            // A link made by HYPERLINK() is followed, not edited here.
            if app.wb.hyperlink_at(app.sheet, *row, *col).is_some() {
                menu.add(MenuSection::Extra, MenuEntry::command(Cmd::EditLink, app).shortcut("⌘K"));
            }
            menu.add(MenuSection::Extra, MenuEntry::command(Cmd::OpenLink, app));
            if app.wb.hyperlink_at(app.sheet, *row, *col).is_some() {
                menu.add(MenuSection::Extra, MenuEntry::command(Cmd::RemoveLink, app));
            }
        }
    }

    fn paint(&self, p: &mut Painter) {
        links::paint(p);
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _: &mut Window, cx: &mut Cx) -> ZoneResponse {
        links::zone_mouse_down(app, zone, ev, cx)
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        links::zone_mouse_up(app, zone, ev, window, cx);
    }

    fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _: &mut Cx) {
        links::cells_written(app, edit);
    }
}

/// An element the headless tests find by name (`debug_bounds`).
fn named(name: &'static str, element: impl IntoElement) -> gpui::Div {
    div().flex_none().debug_selector(|| name.to_string()).child(element)
}

// ---------------------------------------------------------------- shortcuts

actions!(edit_tools, [FindDialog, Replace, PasteSpecialDialog]);

/// The shortcuts of the editing tools.
pub fn bind_keys(cx: &mut App) {
    let w = Some("Workbook");
    cx.bind_keys([
        KeyBinding::new("ctrl-f", FindDialog, w),
        KeyBinding::new("ctrl-h", Replace, w),
        KeyBinding::new("ctrl-cmd-v", PasteSpecialDialog, w),
    ]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &FindDialog, window, cx| app.run_command(Cmd::Find, window, cx)))
        .on_action(cx.listener(|app, _: &Replace, window, cx| app.run_command(Cmd::Replace, window, cx)))
        .on_action(cx.listener(|app, _: &PasteSpecialDialog, window, cx| app.run_command(Cmd::PasteSpecial, window, cx)))
}
