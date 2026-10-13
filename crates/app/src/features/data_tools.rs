//! The data tools of the Data tab: Text to Columns, Flash Fill, the
//! Advanced Filter, Consolidate, What-If Analysis (Goal Seek, the Scenario
//! Manager, Data Table), Get Data (text files, the web, workbooks, JSON
//! and XML documents), the queries it leaves and Refresh All.
//!
//! Each tool has its file in `data_tools/`; the logic is in the engine
//! (`text_columns.rs`, `flashfill.rs`, `filter.rs`, `consolidate.rs`,
//! `whatif.rs`) and the files here hold the dialogs. `ui_tests.rs` drives
//! them in GPUI's headless window.

mod advanced;
mod connections;
mod consolidate;
mod flash;
mod import;
mod navigator;
mod sources;
mod text_wizard;
mod whatif;
#[cfg(test)]
mod ui_tests;

use super::{CellEdit, Command, Cx, Feature, Intent};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::shell::Alert;
use gpui::{App, Context, KeyBinding, SharedString, Stateful, Window, actions, prelude::*};
use opensheet_engine::Rect;

pub struct DataTools;

commands! {
    /// What the user can ask of the data tools.
    pub enum Cmd {
        TextToColumns = "columns", t("Text to Columns", "Convertir");
        FlashFill = "fill-down", t("Flash Fill", "Remplissage instantané");
        AdvancedFilter = "filter", t("Advanced", "Avancé");
        Consolidate = "grid", t("Consolidate", "Consolider");
        WhatIf = "calculator", t("What-If Analysis", "Analyse de scénarios");
        ScenarioManager = "", t("Scenario Manager…", "Gestionnaire de scénarios…");
        GoalSeek = "", t("Goal Seek…", "Valeur cible…");
        DataTable = "", t("Data Table…", "Table de données…");
        GetData = "data", t("Get Data", "Obtenir des données");
        FromText = "", t("From Text/CSV…", "À partir d’un fichier texte/CSV…");
        FromWeb = "", t("From Web…", "À partir du web…");
        FromWorkbook = "", t("From Excel Workbook…", "À partir d’un classeur Excel…");
        FromJson = "", t("From JSON…", "À partir de JSON…");
        FromXml = "", t("From XML File…", "À partir d'un fichier XML…");
        FromSqlServer = "", t("From SQL Server ODBC", "Depuis SQL Server ODBC");
        FromDatabase = "", t("From Database (Microsoft Query)", "À partir de la base de données (Microsoft Query)");
        Connections = "link", t("Queries & Connections", "Requêtes et connexions");
        RangeProperties = "format-cells", t("Properties", "Propriétés");
        RefreshMenu = "refresh", t("Refresh All", "Actualiser tout");
        RefreshAll = "refresh", t("Refresh All", "Actualiser tout");
        Refresh = "", t("Refresh", "Actualiser");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::Refresh => import::can_refresh_here(app),
            Cmd::RangeProperties => {
                let (row, col) = app.active();
                app.wb.text_import_at(app.sheet, row, col).is_some()
            }
            _ => true,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::TextToColumns => {
                text_wizard::open_text_to_columns(app, window, cx);
            }
            Cmd::FlashFill => flash::flash_fill(app, cx),
            Cmd::AdvancedFilter => {
                advanced::open(app, window, cx);
            }
            Cmd::Consolidate => {
                consolidate::open(app, window, cx);
            }
            Cmd::WhatIf => {
                let entries = vec![
                    MenuEntry::command(Cmd::ScenarioManager, app),
                    MenuEntry::command(Cmd::GoalSeek, app),
                    MenuEntry::command(Cmd::DataTable, app),
                ];
                app.open_command_menu(entries, 250.0, window, cx);
            }
            Cmd::ScenarioManager => {
                whatif::open_scenario_manager(app, window, cx);
            }
            Cmd::GoalSeek => {
                whatif::open_goal_seek(app, window, cx);
            }
            Cmd::DataTable => {
                whatif::open_data_table(app, window, cx);
            }
            Cmd::GetData => {
                // Excel for Mac: the sources of Get Data (Power Query),
                // then the two that go through ODBC.
                let entries = vec![
                    MenuEntry::command(Cmd::FromText, app),
                    MenuEntry::command(Cmd::FromWeb, app),
                    MenuEntry::command(Cmd::FromWorkbook, app),
                    MenuEntry::command(Cmd::FromJson, app),
                    MenuEntry::command(Cmd::FromXml, app),
                    MenuEntry::separator(),
                    MenuEntry::command(Cmd::FromSqlServer, app),
                    MenuEntry::command(Cmd::FromDatabase, app),
                ];
                app.open_command_menu(entries, 330.0, window, cx);
            }
            Cmd::FromWeb => {
                if app.is_editing() && !app.commit_edit(None, window, cx) {
                    return;
                }
                navigator::open_web(app, "", None, window, cx);
            }
            Cmd::FromWorkbook | Cmd::FromJson | Cmd::FromXml => import::choose_document(app, window, cx),
            // Databases are reached through an ODBC driver manager, which
            // this program does not have: Excel's own words for that.
            Cmd::FromSqlServer | Cmd::FromDatabase => warn(
                app,
                t(
                    "Microsoft Excel cannot complete this operation because the ODBC Driver Manager is not installed.  To install the ODBC Driver Manager, run Setup and install the database driver for the type of database(s) you want to access.",
                    "Microsoft Excel ne peut pas terminer cette opération. L’administrateur de Microsoft ODBC n’est pas installé. Pour l’installer, exécutez le programme d’installation et installez le pilote correspondant au type de base(s) de données auquel vous voulez accéder.",
                ),
                cx,
            ),
            Cmd::Connections => {
                connections::open(app, window, cx);
            }
            Cmd::RangeProperties => {
                connections::open_range_properties(app, window, cx);
            }
            Cmd::FromText => import::choose_file(app, window, cx),
            Cmd::RefreshMenu => {
                let entries = vec![MenuEntry::command(Cmd::RefreshAll, app), MenuEntry::command(Cmd::Refresh, app)];
                app.open_command_menu(entries, 220.0, window, cx);
            }
            Cmd::RefreshAll => import::refresh_all(app, window, cx),
            Cmd::Refresh => import::refresh_here(app, window, cx),
        }
    }
}

impl Feature for DataTools {
    fn name(&self) -> &'static str {
        "data-tools"
    }

    // The results of a data table change together or not at all.
    fn guard(&self, app: &ExcelApp, intent: &Intent) -> Option<SharedString> {
        match intent {
            Intent::EditCells(rect) => whatif::table_guard(app, *rect),
            _ => None,
        }
    }

    // The queries that refresh by themselves are watched from here: the
    // one hook that sees the window each time it is drawn.
    fn overlay_elements(&self, app: &mut ExcelApp, _: &crate::grid::GridLayout, window: &mut Window, cx: &mut Cx) -> Vec<gpui::AnyElement> {
        import::watch(app, window, cx);
        Vec::new()
    }

    // A data table cleared or overwritten as a whole is no table any
    // more, in the undo step of what replaced it.
    fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _cx: &mut Cx) {
        app.wb.remove_data_tables_in(edit.sheet, edit.rect);
    }
}

// ---------------------------------------------------------------- shortcuts

actions!(data_tools, [FlashFill]);

/// The shortcuts of the data tools.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-e", FlashFill, Some("Workbook"))]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &FlashFill, window, cx| app.run_command(Cmd::FlashFill, window, cx)))
}

/// Tell the user why what was asked cannot be done.
fn warn(app: &mut ExcelApp, message: impl Into<SharedString>, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// The single range a reference typed in a dialog stands for, with its
/// sheet; `None` when the text is not a reference to one range.
fn typed_range(app: &ExcelApp, text: &str) -> Option<(usize, Rect)> {
    let (row, col) = app.active();
    let areas = app.wb.reference_areas(app.sheet, row, col, text)?;
    match areas.as_slice() {
        [area] => Some((area.sheet, area.rect)),
        _ => None,
    }
}
