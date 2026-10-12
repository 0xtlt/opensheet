//! Commands that belong to no feature module of their own.
//!
//! Two kinds live here. The tools around formulas, which are written (each
//! in a file of `misc/`): Go To and Go To Special, Insert Function and
//! Function Arguments, formula auditing (Show Formulas, tracer arrows,
//! Error Checking, Evaluate Formula, the Watch Window) and the Calculation
//! group. And the commands of the ribbon nobody wrote yet (sharing,
//! proofing, themes, outlines, links, ...): they say they are not
//! available; a feature that takes one over moves it to its own module.

mod audit;
mod calc;
mod fn_help;
mod functions;
mod goto;
mod links;

pub use audit::{shown_formula, shows_formulas};
pub use functions::function_arguments;

use self::audit::Trace;
use self::calc::CalcMode;
use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget, Painter, names};
use crate::app::{ExcelApp, MenuEntry, NameManager};
use crate::grid::GridLayout;
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, rows};
use gpui::{AnyElement, App, Context, KeyBinding, Stateful, Window, actions, div, prelude::*, px};

pub struct Misc;

commands! {
    /// The formula tools, and the ribbon commands without a home.
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
        // Formulas
        InsertFunction = "function", t("Insert Function", "Insérer une fonction");
        TracePrecedents = "misc-trace-precedents", t("Trace Precedents", "Repérer les antécédents");
        TraceDependents = "misc-trace-dependents", t("Trace Dependents", "Repérer les dépendants");
        RemoveArrowsMenu = "misc-remove-arrows", t("Remove Arrows", "Supprimer les flèches");
        RemoveArrows = "misc-remove-arrows", t("Remove Arrows", "Supprimer les flèches");
        RemovePrecedentArrows = "", t("Remove Precedent Arrows", "Supprimer les flèches des antécédents");
        RemoveDependentArrows = "", t("Remove Dependent Arrows", "Supprimer les flèches des dépendants");
        ShowFormulas = "misc-show-formulas", t("Show Formulas", "Afficher les formules");
        ErrorCheckingMenu = "misc-error-checking", t("Error Checking", "Vérification des erreurs");
        ErrorChecking = "misc-error-checking", t("Error Checking…", "Vérification des erreurs…");
        TraceError = "", t("Trace Error", "Repérer une erreur");
        EvaluateFormula = "misc-evaluate", t("Evaluate Formula", "Évaluer la formule");
        WatchWindow = "misc-watch", t("Watch Window", "Fenêtre Espion");
        CalcOptions = "calculator", t("Calculation Options", "Options de calcul");
        CalcAutomatic = "", t("Automatic", "Automatique");
        CalcAutomaticExceptTables = "", t("Automatic Except for Data Tables", "Automatique sauf dans les tableaux de données");
        CalcManual = "", t("Manual", "Manuel");
        CalcNow = "misc-calc-now", t("Calculate Now", "Calculer maintenant");
        CalcSheet = "misc-calc-sheet", t("Calculate Sheet", "Calculer la feuille");
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

impl Cmd {
    /// The commands that are written.
    fn is_formula_tool(self) -> bool {
        use Cmd::*;
        matches!(
            self,
            GoTo | GoToSpecial
                | InsertFunction
                | TracePrecedents
                | TraceDependents
                | RemoveArrowsMenu
                | RemoveArrows
                | RemovePrecedentArrows
                | RemoveDependentArrows
                | ShowFormulas
                | ErrorCheckingMenu
                | ErrorChecking
                | TraceError
                | EvaluateFormula
                | WatchWindow
                | CalcOptions
                | CalcAutomatic
                | CalcAutomaticExceptTables
                | CalcManual
                | CalcNow
                | CalcSheet
        )
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
        if !self.is_formula_tool() {
            app.not_implemented(self.label(), cx);
            return;
        }
        // Insert Function works on the formula being typed; the others
        // work on the sheet, once the entry in progress is stored. Menus
        // only open: their entries come back here.
        let menu = matches!(self, Cmd::RemoveArrowsMenu | Cmd::ErrorCheckingMenu | Cmd::CalcOptions);
        if !menu && self != Cmd::InsertFunction && app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::GoTo => goto::open(app, window, cx),
            Cmd::GoToSpecial => goto::open_special(app, window, cx),
            Cmd::InsertFunction => functions::insert_function(app, window, cx),
            Cmd::TracePrecedents => audit::trace(app, Trace::Precedents, cx),
            Cmd::TraceDependents => audit::trace(app, Trace::Dependents, cx),
            Cmd::RemoveArrowsMenu => {
                let entries = vec![
                    MenuEntry::command(Cmd::RemoveArrows, app),
                    MenuEntry::command(Cmd::RemovePrecedentArrows, app),
                    MenuEntry::command(Cmd::RemoveDependentArrows, app),
                ];
                app.open_command_menu(entries, 300.0, window, cx);
            }
            Cmd::RemoveArrows => audit::remove_arrows(app, None, cx),
            Cmd::RemovePrecedentArrows => audit::remove_arrows(app, Some(Trace::Precedents), cx),
            Cmd::RemoveDependentArrows => audit::remove_arrows(app, Some(Trace::Dependents), cx),
            Cmd::ShowFormulas => audit::toggle_formulas(app, cx),
            Cmd::ErrorCheckingMenu => {
                let entries = vec![MenuEntry::command(Cmd::ErrorChecking, app), MenuEntry::command(Cmd::TraceError, app)];
                app.open_command_menu(entries, 240.0, window, cx);
            }
            Cmd::ErrorChecking => audit::open_error_checking(app, window, cx),
            Cmd::TraceError => audit::trace_error(app, cx),
            Cmd::EvaluateFormula => audit::open_evaluate(app, window, cx),
            Cmd::WatchWindow => audit::toggle_watch_window(app, cx),
            Cmd::CalcOptions => {
                let entries = vec![
                    MenuEntry::command(Cmd::CalcAutomatic, app),
                    MenuEntry::command(Cmd::CalcAutomaticExceptTables, app),
                    MenuEntry::command(Cmd::CalcManual, app),
                ];
                app.open_command_menu(entries, 320.0, window, cx);
            }
            Cmd::CalcAutomatic => set_calc_mode(app, CalcMode::Automatic, cx),
            Cmd::CalcAutomaticExceptTables => set_calc_mode(app, CalcMode::AutomaticExceptTables, cx),
            Cmd::CalcManual => set_calc_mode(app, CalcMode::Manual, cx),
            Cmd::CalcNow => {
                calc::calculate_now(app);
                app.refresh(cx);
            }
            Cmd::CalcSheet => {
                calc::calculate_sheet(app, app.sheet);
                app.refresh(cx);
            }
            _ => {}
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::ShowFormulas => audit::shows_formulas(app),
            Cmd::WatchWindow => audit::watch_open(app),
            Cmd::CalcAutomatic => calc::calc_mode(app) == CalcMode::Automatic,
            Cmd::CalcAutomaticExceptTables => calc::calc_mode(app) == CalcMode::AutomaticExceptTables,
            Cmd::CalcManual => calc::calc_mode(app) == CalcMode::Manual,
            _ => false,
        }
    }

}

fn set_calc_mode(app: &mut ExcelApp, mode: CalcMode, cx: &mut Cx) {
    calc::set_calc_mode(app, mode);
    // Back to automatic: what waited is calculated.
    if mode != CalcMode::Manual {
        calc::calculate_now(app);
    }
    app.refresh(cx);
}

impl Feature for Misc {
    fn name(&self) -> &'static str {
        "misc"
    }

    fn paint(&self, p: &mut Painter) {
        audit::paint(p);
    }

    fn adjust_layout(&self, app: &ExcelApp, layout: &mut GridLayout) {
        audit::adjust_layout(app, layout);
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Link, app).shortcut("⌘K"));
        }
    }
}

// ------------------------------------------------------------------- ribbon

/// The Formula Auditing group of the Formulas tab.
pub fn auditing_group(app: &ExcelApp, cx: &mut Cx) -> AnyElement {
    group(
        t("Formula Auditing", "Vérification des formules"),
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(rows(vec![
                app.cmd_labeled("fx-precedents", Cmd::TracePrecedents, false, cx),
                app.cmd_labeled("fx-dependents", Cmd::TraceDependents, false, cx),
                app.cmd_labeled("fx-remove-arrows", Cmd::RemoveArrowsMenu, true, cx),
            ]))
            .child(rows(vec![
                app.cmd_labeled("fx-show-formulas", Cmd::ShowFormulas, false, cx),
                app.cmd_labeled("fx-error-checking", Cmd::ErrorCheckingMenu, true, cx),
                app.cmd_labeled("fx-evaluate", Cmd::EvaluateFormula, false, cx),
            ]))
            .child(app.cmd_large("fx-watch", Cmd::WatchWindow, false, cx)),
    )
    .into_any_element()
}

/// The Calculation group of the Formulas tab (the last one: the caller
/// takes its right border off).
pub fn calculation_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(
        t("Calculation", "Calcul"),
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(app.cmd_large("fx-calc-options", Cmd::CalcOptions, true, cx))
            .child(rows(vec![
                app.cmd_labeled("fx-calc-now", Cmd::CalcNow, false, cx),
                app.cmd_labeled("fx-calc-sheet", Cmd::CalcSheet, false, cx),
            ])),
    )
}

// ---------------------------------------------------------------- shortcuts

actions!(
    formulas,
    [GoTo, InsertFunction, ShowFormulas, CalculateNow, CalculateSheet, PasteName, CreateNames]
);

/// The shortcuts of the formula tools and of defined names.
pub fn bind_keys(cx: &mut App) {
    let w = Some("Workbook");
    cx.bind_keys([
        KeyBinding::new("ctrl-g", GoTo, w),
        KeyBinding::new("cmd-g", GoTo, w),
        KeyBinding::new("f5", GoTo, w),
        KeyBinding::new("shift-f3", InsertFunction, w),
        KeyBinding::new("ctrl-`", ShowFormulas, w),
        KeyBinding::new("f9", CalculateNow, w),
        KeyBinding::new("shift-f9", CalculateSheet, w),
        KeyBinding::new("cmd-f3", NameManager, w),
        KeyBinding::new("f3", PasteName, w),
        KeyBinding::new("ctrl-shift-f3", CreateNames, w),
    ]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    // These two work on the formula being typed: the entry is not stored
    // first, as `run_command` would.
    fn on_formula<C: Command>(app: &mut ExcelApp, cmd: C, window: &mut Window, cx: &mut Cx) {
        if !app.modal_open() && cmd.enabled(app) {
            app.popup = None;
            cmd.run(app, window, cx);
        }
    }
    root.on_action(cx.listener(|app, _: &GoTo, window, cx| app.run_command(Cmd::GoTo, window, cx)))
        .on_action(cx.listener(|app, _: &ShowFormulas, window, cx| app.run_command(Cmd::ShowFormulas, window, cx)))
        .on_action(cx.listener(|app, _: &CalculateNow, window, cx| app.run_command(Cmd::CalcNow, window, cx)))
        .on_action(cx.listener(|app, _: &CalculateSheet, window, cx| app.run_command(Cmd::CalcSheet, window, cx)))
        .on_action(cx.listener(|app, _: &CreateNames, window, cx| {
            app.run_command(names::Cmd::CreateFromSelection, window, cx)
        }))
        .on_action(cx.listener(|app, _: &InsertFunction, window, cx| on_formula(app, Cmd::InsertFunction, window, cx)))
        .on_action(cx.listener(|app, _: &PasteName, window, cx| on_formula(app, names::Cmd::PasteNames, window, cx)))
}
