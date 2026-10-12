//! Pivot tables: the creation dialog, the PivotTable Fields pane, the
//! buttons on pivot cells, and the "PivotTable Analyze" and "Design" tabs.
//!
//! Not written yet: the commands are in place and say so. See
//! `features/mod.rs` for the hooks (the field list is a pane opened with
//! `open_pane` and built with `widgets::Tree` / `widgets::CheckList`, the
//! buttons on cells are zones, `guard` refuses typing into a pivot table,
//! the two tabs appear through `context_tabs`).

use super::{Command, ContextTab, Cx, Feature, tables};
use crate::app::ExcelApp;
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, row, rows};
use gpui::{AnyElement, Window, div, prelude::*, px};
use opensheet_engine::pivot::PivotTable;

pub struct Pivots;

const ANALYZE_TAB: &str = "pivot-analyze";
const DESIGN_TAB: &str = "pivot-design";

commands! {
    /// What the user can ask of pivot tables.
    pub enum Cmd {
        Insert = "pivot", t("PivotTable", "Tableau croisé dynamique");
        Recommended = "pivot", t("Recommended PivotTables", "Tableaux croisés dynamiques recommandés");
        InsertTimeline = "clock", t("Timeline", "Chronologie");
        // PivotTable Analyze
        Options = "pivot", t("Options", "Options");
        FieldSettings = "format-cells", t("Field Settings", "Paramètres de champ");
        ExpandField = "plus", t("Expand Field", "Développer le champ");
        CollapseField = "minus", t("Collapse Field", "Réduire le champ");
        GroupSelection = "group-objects", t("Group Selection", "Grouper la sélection");
        Ungroup = "group-objects", t("Ungroup", "Dissocier");
        GroupField = "group-objects", t("Group Field", "Grouper les champs");
        FilterConnections = "link", t("Filter Connections", "Connexions de filtre");
        Refresh = "refresh", t("Refresh", "Actualiser");
        ChangeSource = "data", t("Change Data Source", "Changer la source de données");
        Clear = "clear", t("Clear", "Effacer");
        Select = "grid", t_ctx("pivot", "Select", "Sélectionner");
        Move = "pivot", t("Move PivotTable", "Déplacer le tableau croisé dynamique");
        CalculatedFields = "function", t("Fields, Items, & Sets", "Champs, éléments et jeux");
        PivotChart = "chart-bar", t("PivotChart", "Graphique croisé dynamique");
        FieldList = "columns", t("Field List", "Liste des champs");
        Buttons = "plus", t("+/- Buttons", "Boutons +/-");
        FieldHeaders = "table", t("Field Headers", "En-têtes de champ");
        // Design
        Subtotals = "calculator", t("Subtotals", "Sous-totaux");
        GrandTotals = "calculator", t("Grand Totals", "Totaux généraux");
        ReportLayout = "table", t("Report Layout", "Disposition du rapport");
        BlankRows = "insert-cells", t("Blank Rows", "Lignes vides");
        RowHeaders = "", t("Row Headers", "En-têtes de lignes");
        ColumnHeaders = "", t("Column Headers", "En-têtes de colonnes");
        BandedRows = "", t("Banded Rows", "Lignes à bandes");
        BandedColumns = "", t("Banded Columns", "Colonnes à bandes");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
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

/// The pivot table the active cell is in. The engine does not say yet
/// where a pivot table is written: none is ever found.
fn active_pivot(_app: &ExcelApp) -> Option<&PivotTable> {
    None
}

/// The groups of the "PivotTable Analyze" tab.
fn analyze_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let name = active_pivot(app).map(|pivot| pivot.name.clone()).unwrap_or_default();
    vec![
        group(
            t("PivotTable", "Tableau croisé dynamique"),
            rows(vec![
                div()
                    .px(px(4.))
                    .child(t("PivotTable Name:", "Nom du tableau croisé dynamique :"))
                    .into_any_element(),
                div().px(px(4.)).child(name).into_any_element(),
                app.cmd_labeled("pa-options", Cmd::Options, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Active Field", "Champ actif"),
            rows(vec![
                app.cmd_labeled("pa-field-settings", Cmd::FieldSettings, false, cx),
                app.cmd_labeled("pa-expand", Cmd::ExpandField, false, cx),
                app.cmd_labeled("pa-collapse", Cmd::CollapseField, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t_ctx("pivot", "Group", "Groupe"),
            rows(vec![
                app.cmd_labeled("pa-group", Cmd::GroupSelection, false, cx),
                app.cmd_labeled("pa-ungroup", Cmd::Ungroup, false, cx),
                app.cmd_labeled("pa-group-field", Cmd::GroupField, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Filter", "Filtrer"),
            row(vec![
                app.cmd_large("pa-slicer", tables::Cmd::InsertSlicer, false, cx),
                app.cmd_large("pa-timeline", Cmd::InsertTimeline, false, cx),
                app.cmd_large("pa-connections", Cmd::FilterConnections, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Data", "Données"),
            row(vec![
                app.cmd_large("pa-refresh", Cmd::Refresh, true, cx),
                app.cmd_large("pa-source", Cmd::ChangeSource, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Actions", "Actions"),
            rows(vec![
                app.cmd_labeled("pa-clear", Cmd::Clear, true, cx),
                app.cmd_labeled("pa-select", Cmd::Select, true, cx),
                app.cmd_labeled("pa-move", Cmd::Move, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Calculations", "Calculs"),
            row(vec![app.cmd_large("pa-calculated", Cmd::CalculatedFields, true, cx)]),
        )
        .into_any_element(),
        group(
            t("Tools", "Outils"),
            row(vec![
                app.cmd_large("pa-chart", Cmd::PivotChart, false, cx),
                app.cmd_large("pa-recommended", Cmd::Recommended, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Show", "Afficher"),
            rows(vec![
                app.cmd_labeled("pa-field-list", Cmd::FieldList, false, cx),
                app.cmd_labeled("pa-buttons", Cmd::Buttons, false, cx),
                app.cmd_labeled("pa-headers", Cmd::FieldHeaders, false, cx),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// The groups of the "Design" tab of pivot tables.
fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Layout", "Disposition"),
            row(vec![
                app.cmd_large("pd-subtotals", Cmd::Subtotals, true, cx),
                app.cmd_large("pd-totals", Cmd::GrandTotals, true, cx),
                app.cmd_large("pd-layout", Cmd::ReportLayout, true, cx),
                app.cmd_large("pd-blank", Cmd::BlankRows, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("PivotTable Style Options", "Options de style de tableau croisé dynamique"),
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    app.cmd_check("pd-row-headers", Cmd::RowHeaders, cx),
                    app.cmd_check("pd-col-headers", Cmd::ColumnHeaders, cx),
                ]))
                .child(rows(vec![
                    app.cmd_check("pd-banded-rows", Cmd::BandedRows, cx),
                    app.cmd_check("pd-banded-cols", Cmd::BandedColumns, cx),
                ])),
        )
        .into_any_element(),
        group(
            t("PivotTable Styles", "Styles de tableau croisé dynamique"),
            row(vec![app.cmd_large("pd-styles", Cmd::Styles, true, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

impl Feature for Pivots {
    fn name(&self) -> &'static str {
        "pivot"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match active_pivot(app) {
            Some(_) => vec![
                ContextTab {
                    id: ANALYZE_TAB,
                    title: t("PivotTable Analyze", "Analyse du tableau croisé dynamique"),
                },
                ContextTab {
                    id: DESIGN_TAB,
                    title: t_ctx("pivot", "Design", "Création"),
                },
            ],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        if tab == DESIGN_TAB {
            design_tab(app, cx)
        } else {
            analyze_tab(app, cx)
        }
    }
}
