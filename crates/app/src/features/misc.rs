//! Commands of the ribbon that belong to no feature module yet (sharing,
//! proofing, themes, outlines, links, ...). They say they are not
//! available; a feature that takes one over moves it to its own module.

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::{t, t_ctx};
use gpui::Window;

pub struct Misc;

commands! {
    /// Ribbon commands without a home.
    pub enum Cmd {
        Share = "share", t("Share", "Partager");
        // Home
        CellStyles = "cell-styles", t("Cell Styles", "Styles de cellules");
        ClearHyperlinks = "", t("Clear Hyperlinks", "Effacer les liens hypertexte");
        Replace = "", t("Replace…", "Remplacer…");
        GoTo = "", t("Go To…", "Atteindre…");
        GoToSpecial = "", t("Go To Special…", "Sélectionner les cellules…");
        // Insert
        Sparklines = "chart-line", t("Sparklines", "Graphiques sparkline");
        Link = "link", t("Link", "Lien");
        Equation = "function", t("Equation", "Équation");
        Symbol = "more", t("Symbol", "Symbole");
        // Draw
        Draw = "brush", t_ctx("button", "Draw", "Dessiner");
        Eraser = "clear", t("Eraser", "Gomme");
        InkToShape = "shapes", t("Ink to Shape", "Entrée manuscrite en forme");
        // Page Layout
        Themes = "cell-styles", t("Themes", "Thèmes");
        ThemeColors = "fill", t("Colors", "Couleurs");
        ThemeFonts = "font-grow", t("Fonts", "Polices");
        ThemeEffects = "shapes", t("Effects", "Effets");
        // Data
        GetData = "data", t("Get Data", "Obtenir des données");
        RefreshAll = "refresh", t("Refresh All", "Actualiser tout");
        TextToColumns = "columns", t("Text to Columns", "Convertir");
        FlashFill = "fill-down", t("Flash Fill", "Remplissage instantané");
        Consolidate = "grid", t("Consolidate", "Consolider");
        WhatIf = "calculator", t("What-If Analysis", "Analyse de scénarios");
        GroupRows = "group-objects", t_ctx("outline", "Group", "Grouper");
        UngroupRows = "group-objects", t_ctx("outline", "Ungroup", "Dissocier");
        Subtotal = "calculator", t("Subtotal", "Sous-total");
        // Review
        Spelling = "spell", t("Spelling", "Orthographe");
        Thesaurus = "book", t("Thesaurus", "Dictionnaire des synonymes");
        WorkbookStatistics = "calculator", t("Workbook Statistics", "Statistiques du classeur");
        CheckAccessibility = "eye", t("Check Accessibility", "Vérifier l'accessibilité");
        Translate = "book", t("Translate", "Traduire");
        // Automate, Help
        NewScript = "function", t("New Script", "Nouveau script");
        AllScripts = "book", t("All Scripts", "Tous les scripts");
        Help = "book", t("Help", "Aide");
        Feedback = "comment", t("Feedback", "Commentaires");
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

impl Feature for Misc {
    fn name(&self) -> &'static str {
        "misc"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Link, app).shortcut("⌘K"));
        }
    }
}
