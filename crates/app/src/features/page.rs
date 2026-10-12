//! Page setup, the Page Layout view, the Page Break Preview and printing.
//!
//! Not written yet: the commands are in place and say so; the three view
//! buttons already switch `ExcelApp::view_mode`, which nothing draws
//! differently so far. See `features/mod.rs`: page breaks are drawn in
//! `paint`; `adjust_layout` lays the sheet out in pages (`Axis::set_gaps`
//! leaves room between them) for the Page Layout view; the breaks of the
//! Page Break Preview are zones that capture the mouse.

use super::{Command, Cx, Feature};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Page;

commands! {
    /// What the user can ask of page setup and printing.
    pub enum Cmd {
        Print = "print", t("Print…", "Imprimer…");
        Setup = "page", t("Page Setup", "Mise en page");
        Margins = "margins", t("Margins", "Marges");
        MarginsNormal = "", t("Normal", "Normales");
        MarginsWide = "", t("Wide", "Larges");
        MarginsNarrow = "", t("Narrow", "Étroites");
        CustomMargins = "", t("Custom Margins…", "Marges personnalisées…");
        Orientation = "page", t("Orientation", "Orientation");
        Portrait = "", t("Portrait", "Portrait");
        Landscape = "", t("Landscape", "Paysage");
        Size = "page", t("Size", "Taille");
        PrintArea = "print", t("Print Area", "ZoneImpr");
        SetPrintArea = "", t("Set Print Area", "Définir");
        ClearPrintArea = "", t("Clear Print Area", "Annuler");
        Breaks = "breaks", t("Breaks", "Sauts de page");
        InsertBreak = "", t("Insert Page Break", "Insérer un saut de page");
        RemoveBreak = "", t("Remove Page Break", "Supprimer le saut de page");
        ResetBreaks = "", t("Reset All Page Breaks", "Rétablir tous les sauts de page");
        Background = "picture", t("Background", "Arrière-plan");
        PrintTitles = "table", t("Print Titles", "Imprimer les titres");
        FitWidth = "columns", t("Width", "Largeur");
        FitHeight = "align-middle", t("Height", "Hauteur");
        Scale = "zoom", t("Scale", "Échelle");
        PrintGridlines = "", t("Print", "Imprimer");
        PrintHeadings = "", t("Print", "Imprimer");
        Ruler = "", t("Ruler", "Règle");
        CustomViews = "eye", t("Custom Views", "Affichages personnalisés");
        HeaderFooter = "header-footer", t("Header & Footer", "En-tête et pied de page");
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
        let list = |app: &ExcelApp, commands: &[Cmd]| -> Vec<MenuEntry> {
            commands.iter().map(|cmd| MenuEntry::command(*cmd, app)).collect()
        };
        let entries = match self {
            Cmd::Margins => {
                let mut entries = list(app, &[Cmd::MarginsNormal, Cmd::MarginsWide, Cmd::MarginsNarrow]);
                entries.push(MenuEntry::separator());
                entries.push(MenuEntry::command(Cmd::CustomMargins, app));
                entries
            }
            Cmd::Orientation => list(app, &[Cmd::Portrait, Cmd::Landscape]),
            Cmd::PrintArea => list(app, &[Cmd::SetPrintArea, Cmd::ClearPrintArea]),
            Cmd::Breaks => list(app, &[Cmd::InsertBreak, Cmd::RemoveBreak, Cmd::ResetBreaks]),
            _ => {
                app.not_implemented(self.label(), cx);
                return;
            }
        };
        app.open_command_menu(entries, 250.0, window, cx);
    }
}

impl Feature for Page {
    fn name(&self) -> &'static str {
        "page"
    }
}
