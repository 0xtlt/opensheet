//! Tables ("Format as Table"): creating and resizing them, their styles,
//! the totals row, and the "Table Design" tab.
//!
//! Not written yet: the commands are in place and say so; the "Table
//! Design" tab appears while the active cell is in a table and its check
//! boxes show the table's options. See `features/mod.rs` for the hooks:
//! `cells_written` is where `Workbook::tables_after_edit` grows a table
//! under what was typed (before the commit, as the engine asks), the
//! resize handle at the corner is a zone that captures the mouse, the
//! style gallery a view opened with `open_popup_view`.

use super::{Command, ContextMenu, ContextTab, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use gpui::{AnyElement, Window, div, prelude::*, px};
use opensheet_engine::table::{Table, TableOption};

pub struct Tables;

const DESIGN_TAB: &str = "table-design";

commands! {
    /// What the user can ask of tables.
    pub enum Cmd {
        Insert = "table", t("Table", "Tableau");
        FormatAs = "table", t("Format as Table", "Mettre sous forme de tableau");
        RemoveDuplicates = "delete-cells", t("Remove Duplicates", "Supprimer les doublons");
        InsertSlicer = "slicer", t("Slicer", "Segment");
        Rename = "", t("Table Name", "Nom du tableau");
        Resize = "table", t("Resize Table", "Redimensionner le tableau");
        SummarizeWithPivot = "pivot", t("Summarize with PivotTable", "Synthétiser avec un tableau croisé dynamique");
        ConvertToRange = "grid", t("Convert to Range", "Convertir en plage");
        Export = "share", t("Export", "Exporter");
        Refresh = "refresh", t("Refresh", "Actualiser");
        HeaderRow = "", t("Header Row", "Ligne d'en-tête");
        TotalRow = "", t("Total Row", "Ligne des totaux");
        BandedRows = "", t("Banded Rows", "Lignes à bandes");
        FirstColumn = "", t("First Column", "Première colonne");
        LastColumn = "", t("Last Column", "Dernière colonne");
        BandedColumns = "", t("Banded Columns", "Colonnes à bandes");
        FilterButton = "", t("Filter Button", "Bouton de filtre");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
    }
}

/// The table the active cell is in.
fn active_table(app: &ExcelApp) -> Option<&Table> {
    let (row, col) = app.active();
    app.wb.table_at(app.sheet, row, col)
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let option = match self {
            Cmd::HeaderRow => TableOption::HeaderRow,
            Cmd::TotalRow => TableOption::TotalsRow,
            Cmd::BandedRows => TableOption::BandedRows,
            Cmd::FirstColumn => TableOption::FirstCol,
            Cmd::LastColumn => TableOption::LastCol,
            Cmd::BandedColumns => TableOption::BandedCols,
            Cmd::FilterButton => TableOption::FilterButton,
            _ => return false,
        };
        active_table(app).is_some_and(|table| table.option(option))
    }

    fn run(self, app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
        app.not_implemented(self.label(), cx)
    }
}

/// The groups of the "Table Design" tab.
fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let name = active_table(app).map(|table| table.name.clone()).unwrap_or_default();
    vec![
        group(
            t("Properties", "Propriétés"),
            rows(vec![
                div().px(px(4.)).child(t("Table Name:", "Nom du tableau :")).into_any_element(),
                app.combo("td-name", name, 130.0, Cmd::Rename.label(), cx, |this, window, cx| {
                    Cmd::Rename.run(this, window, cx)
                }),
                app.cmd_labeled("td-resize", Cmd::Resize, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Tools", "Outils"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(rows(vec![
                    app.cmd_labeled("td-pivot", Cmd::SummarizeWithPivot, false, cx),
                    app.cmd_labeled("td-duplicates", Cmd::RemoveDuplicates, false, cx),
                    app.cmd_labeled("td-range", Cmd::ConvertToRange, false, cx),
                ]))
                .child(app.cmd_large("td-slicer", Cmd::InsertSlicer, false, cx)),
        )
        .into_any_element(),
        group(
            t("External Table Data", "Données de tableau externe"),
            row(vec![
                app.cmd_large("td-export", Cmd::Export, true, cx),
                app.cmd_large("td-refresh", Cmd::Refresh, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Table Style Options", "Options de style de tableau"),
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    app.cmd_check("td-header", Cmd::HeaderRow, cx),
                    app.cmd_check("td-total", Cmd::TotalRow, cx),
                    app.cmd_check("td-banded-rows", Cmd::BandedRows, cx),
                ]))
                .child(rows(vec![
                    app.cmd_check("td-first", Cmd::FirstColumn, cx),
                    app.cmd_check("td-last", Cmd::LastColumn, cx),
                    app.cmd_check("td-banded-cols", Cmd::BandedColumns, cx),
                ]))
                .child(rows(vec![app.cmd_check("td-filter", Cmd::FilterButton, cx)])),
        )
        .into_any_element(),
        group(
            t("Table Styles", "Styles de tableau"),
            row(vec![app.cmd_large("td-styles", Cmd::Styles, true, cx)]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

impl Feature for Tables {
    fn name(&self) -> &'static str {
        "tables"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match active_table(app) {
            Some(_) => vec![ContextTab {
                id: DESIGN_TAB,
                title: t("Table Design", "Création de tableau"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        design_tab(app, cx)
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) && active_table(app).is_some() {
            menu.add(
                MenuSection::Data,
                MenuEntry::submenu(
                    t("Table", "Tableau"),
                    vec![
                        MenuEntry::command(Cmd::TotalRow, app),
                        MenuEntry::command(Cmd::ConvertToRange, app),
                    ],
                ),
            );
        }
    }
}
