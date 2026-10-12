//! Tables ("Format as Table"): creating and resizing them, their styles,
//! the totals row, the "Table Design" tab, and what a table does on the
//! sheet (its corner handle, the lists of its totals cells, growing under
//! what is typed, Tab adding rows, structured references while pointing).
//!
//! The table model and everything that changes it are the engine's
//! (`opensheet_engine::table`); the filter buttons of header cells belong
//! to `filters.rs`. This file holds the commands and the hooks; the parts
//! are in `tables/`:
//!
//! * `actions.rs`: the questions asked of the workbook, as functions of
//!   the workbook alone;
//! * `dialogs.rs`: Create Table and Resize Table;
//! * `styles.rs`: the style gallery and its thumbnails;
//! * `design.rs`: the Table Design tab;
//! * `sheet.rs`: what is drawn and clicked on the grid, and what follows
//!   an edit;
//! * `menu.rs`: the context menu of table cells.

mod actions;
mod design;
mod dialogs;
mod menu;
mod sheet;
mod styles;

use self::actions::AutoChange;
use super::{
    CellEdit, Command, ContextMenu, ContextTab, Cx, Feature, Intent, MenuTarget, Painter, PendingEdit, Verdict, Zone,
    ZoneEvent, ZoneResponse, filters, pivot,
};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::shell::{Alert, RefStyle};
use gpui::{AnyElement, Window};
use opensheet_engine::Rect;
use opensheet_engine::table::{Table, TableError, TableOption};
use std::cell::Cell;
use std::rc::{Rc, Weak};
use std::time::Instant;

pub use self::sheet::{advance, go_to, pointed_reference, tab_pressed};

pub struct Tables;

const DESIGN_TAB: &str = "table-design";

commands! {
    /// What the user can ask of tables.
    pub enum Cmd {
        Insert = "table", t("Table", "Tableau");
        FormatAs = "table", t("Format as Table", "Mettre sous forme de tableau");
        RemoveDuplicates = "delete-cells", t("Remove Duplicates", "Supprimer les doublons");
        // Table Design
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
        NewStyle = "", t("New Table Style…", "Nouveau style de tableau…");
        ClearStyle = "clear", t("Clear", "Effacer");
        // The context menu of table cells
        InsertColumnsLeft = "", t("Table Columns to the Left", "Colonnes de tableau à gauche");
        InsertColumnRight = "", t("Table Column to the Right", "Colonne de tableau à droite");
        InsertRowsAbove = "", t("Table Rows Above", "Lignes de tableau en haut");
        InsertRowBelow = "", t("Table Row Below", "Lignes de tableau en dessous");
        DeleteColumns = "", t("Table Columns", "Colonnes de tableau");
        DeleteRows = "", t("Table Rows", "Lignes de tableau");
        SelectColumnData = "", t("Table Column Data", "Données de colonne de tableau");
        SelectEntireColumn = "", t("Entire Table Column", "Toute la colonne de tableau");
        SelectRow = "", t("Table Row", "Ligne de tableau");
        TotalsRow = "", t("Totals Row", "Ligne Totaux");
    }
}

/// What the module remembers between events.
#[derive(Default)]
struct TablesUi {
    /// The Table Name box of the Table Design tab.
    name_box: Option<design::NameBox>,
    /// The corner handle being dragged.
    resize: Option<sheet::ResizeDrag>,
    /// The range a dialog of tables is showing on the sheet; dead once the
    /// dialog is gone.
    shown: Weak<Cell<Option<(usize, Rect)>>>,
    /// The last thing a table did by itself, while it can be taken back.
    auto: Option<AutoChange>,
    /// Tab was pressed and the move it asks for has not been made yet.
    tab: sheet::TabKey,
    /// When the button of the open style gallery was pressed: the press
    /// closes the gallery, and the click it starts must not open it again.
    gallery_put_away: Rc<Cell<Option<Instant>>>,
}

/// How dialogs of tables write a range: `=$A$1:$C$9`, as Excel's do.
const RANGE_FIELD: RefStyle = RefStyle {
    absolute: true,
    sheet: false,
    equals: true,
};

/// The table the active cell is in.
fn active_table(app: &ExcelApp) -> Option<&Table> {
    let (row, col) = app.active();
    app.wb.table_at(app.sheet, row, col)
}

/// What was being done when the engine refused: Excel words its refusals
/// accordingly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Doing {
    Creating,
    /// Creating a table without headers (its data moves down a row to
    /// make room for them) on a sheet that has a filter.
    CreatingOverFilter,
    Resizing,
    Other,
}

/// Excel's message for a change the engine refused. `None` for the
/// refusals the commands never run into (they look the table up and grey
/// out what cannot be asked).
fn refusal(error: TableError, doing: Doing) -> Option<&'static str> {
    Some(match (error, doing) {
        (TableError::Overlap, _) => t(
            "A table can't overlap another table.",
            "Un tableau ne peut pas en chevaucher un autre.",
        ),
        (TableError::MergedCells, _) => t(
            "Cannot complete operation: A table cannot overlap with a PivotTable report, query results, a table, merged cells, or an XML mapping.",
            "Impossible d'effectuer l'opération : une liste ne peut pas chevaucher un rapport de tableau croisé dynamique, les résultats d'une requête, un tableau, des cellules fusionnées ou un mappage XML.",
        ),
        (TableError::InvalidResize | TableError::InvalidRange, Doing::Resizing) => t(
            "This range won't work because it doesn't align with the existing table. The table requires headers to remain in the same row and include at least one row of data, and the new range should overlap the existing table.",
            "Cette plage n’est pas utilisable, car elle ne s’aligne pas sur le tableau existant dans lequel les en-têtes doivent rester sur la même ligne et doivent contenir au moins une ligne de données. De plus, la nouvelle plage doit chevaucher la table existante.",
        ),
        (TableError::InvalidRange | TableError::InvalidResize, _) => invalid_reference(),
        (TableError::Blocked, Doing::CreatingOverFilter) => t(
            "To create a table from an AutoFiltered range, turn off AutoFilter. Or, when inserting the table, enable the option: My table has headers.",
            "Pour créer un tableau à partir d’une plage filtrée automatiquement, désactivez le filtre automatique. Au moment de l’insertion du tableau, vous pouvez également cocher l’option « Mon tableau comporte des en-têtes ».",
        ),
        (TableError::Blocked, _) => t(
            "This won't work because it would move cells in a table on your worksheet.",
            "Nous ne pouvons effectuer cette action, car cela impliquerait le déplacement de cellules d’un tableau de votre feuille de calcul.",
        ),
        (TableError::InvalidName | TableError::NameTaken, _) => t(
            "The syntax of this name isn't correct.\n\nVerify that the name:\n\t-Starts with a letter or underscore (_)\n\t-Doesn't include a space or character that isn't allowed\n\t-Doesn't conflict with an existing name in the workbook.",
            "La syntaxe de ce nom est incorrecte.\n\nVérifiez que le nom :\n\t-commence par une lettre ou un trait de soulignement (_) ;\n\t-ne contient pas d’espace ou de caractère non autorisé ;\n\t-n’entre pas en conflit avec un nom existant dans le classeur.",
        ),
        (TableError::NotFound | TableError::NoHeaderRow, _) => return None,
    })
}

fn invalid_reference() -> &'static str {
    t("Reference isn't valid.", "La référence n’est pas valide.")
}

/// Tell the user why the engine refused a change.
fn refuse(app: &mut ExcelApp, error: TableError, doing: Doing, cx: &mut Cx) {
    if let Some(message) = refusal(error, doing) {
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
    }
}

/// Make a table of `range`. `style`: the one picked in the Format as
/// Table gallery (`Some(None)`: no style); `None` leaves Excel's default.
fn create(
    app: &mut ExcelApp,
    sheet: usize,
    range: Rect,
    headers: bool,
    style: Option<Option<&'static str>>,
    cx: &mut Cx,
) {
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    // Headers are text: Excel says so before a formula is lost.
    if headers && actions::header_formulas(&app.wb, sheet, range) {
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "Formulas or rich data types in the header row will be removed and converted to static text.  Do you want to continue?",
                    "Les formules ou les types de données enrichies figurant dans la ligne d’en-tête seront supprimés et convertis en texte statique. Voulez-vous continuer ?",
                ),
            )
            .warning()
            .button(t("Yes", "Oui"), move |app, _, cx| add(app, sheet, range, headers, style, cx))
            .close_button(t("No", "Non")),
            cx,
        );
        return;
    }
    add(app, sheet, range, headers, style, cx);
}

fn add(app: &mut ExcelApp, sheet: usize, range: Rect, headers: bool, style: Option<Option<&'static str>>, cx: &mut Cx) {
    match app.wb.add_table(sheet, range, headers) {
        Ok(id) => {
            if let Some(style) = style {
                app.wb.set_table_style(sheet, id, style);
            }
            app.commit(t("Create Table", "Créer un tableau"), cx);
            // The new table is selected and its tab comes to the front.
            if let Some(range) = app
                .wb
                .table(sheet, id)
                .map(|table| table.range)
                .filter(|_| sheet == app.sheet)
            {
                app.select_rect(range, cx);
                app.context_tab = Some(DESIGN_TAB);
            }
        }
        Err(error) => {
            let filtered = app.wb.sheets.get(sheet).is_some_and(|sh| sh.auto_filter.is_some());
            let doing = if filtered && !headers {
                Doing::CreatingOverFilter
            } else {
                Doing::Creating
            };
            refuse(app, error, doing, cx);
        }
    }
}

/// Give a table another range (the Resize Table dialog, the corner
/// handle).
fn resize(app: &mut ExcelApp, sheet: usize, id: u32, range: Rect, cx: &mut Cx) {
    let unchanged = app.wb.table(sheet, id).is_none_or(|table| table.range == range);
    if unchanged || !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let lost = app
        .wb
        .table(sheet, id)
        .and_then(|table| actions::totals_overwrite(&app.wb, sheet, table, range));
    let Some(lost) = lost else {
        return apply_resize(app, sheet, id, range, cx);
    };
    let message = t(
        "Because the total row is turned on, resizing this table will overwrite the data in |.  Do you want to proceed?",
        "L'affichage de la ligne des totaux étant activé, le redimensionnement de ce tableau va remplacer les données dans |. Voulez-vous continuer ?",
    )
    .replace('|', &RefStyle::absolute().format(lost, ""));
    app.alert(
        Alert::new("OpenSheet", message)
            .warning()
            .button(t("OK", "OK"), move |app, _, cx| apply_resize(app, sheet, id, range, cx))
            .close_button(t("Cancel", "Annuler")),
        cx,
    );
}

fn apply_resize(app: &mut ExcelApp, sheet: usize, id: u32, range: Rect, cx: &mut Cx) {
    match app.wb.resize_table(sheet, id, range) {
        Ok(()) => app.commit(t("Table Resize", "Redimensionnement de tableau"), cx),
        Err(error) => refuse(app, error, Doing::Resizing, cx),
    }
}

/// Give the table of the active cell a style of the gallery (`None`: no
/// style).
fn set_style(app: &mut ExcelApp, style: Option<&'static str>, cx: &mut Cx) {
    let Some(id) = active_table(app).map(|table| table.id) else {
        return;
    };
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let sheet = app.sheet;
    app.wb.set_table_style(sheet, id, style);
    app.commit(t("Table Style", "Style de tableau"), cx);
}

/// Tick or untick a box of Table Style Options. `label` names the undo
/// step.
fn toggle_option(app: &mut ExcelApp, option: TableOption, label: &str, cx: &mut Cx) {
    let Some((id, on)) = active_table(app).map(|table| (table.id, table.option(option))) else {
        return;
    };
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let sheet = app.sheet;
    match app.wb.set_table_option(sheet, id, option, !on) {
        Ok(()) => app.commit(label, cx),
        Err(error) => refuse(app, error, Doing::Other, cx),
    }
}

/// Table Design > Convert to Range, after Excel's question.
fn convert_to_range(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(id) = active_table(app).map(|table| table.id) else {
        return;
    };
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let sheet = app.sheet;
    app.alert(
        Alert::new(
            "OpenSheet",
            t(
                "Do you want to convert the table to a normal range?",
                "Voulez-vous convertir le tableau en plage normale ?",
            ),
        )
        .warning()
        .button(t("Yes", "Oui"), move |app, _, cx| {
            if app.wb.convert_table_to_range(sheet, id) {
                app.commit(t("Convert Table to Range", "Convertir un tableau en plage"), cx);
            }
        })
        .close_button(t("No", "Non")),
        cx,
    );
}

impl Cmd {
    /// The Table Style Options box a command ticks.
    fn option(self) -> Option<TableOption> {
        Some(match self {
            Cmd::HeaderRow => TableOption::HeaderRow,
            Cmd::TotalRow | Cmd::TotalsRow => TableOption::TotalsRow,
            Cmd::BandedRows => TableOption::BandedRows,
            Cmd::FirstColumn => TableOption::FirstCol,
            Cmd::LastColumn => TableOption::LastCol,
            Cmd::BandedColumns => TableOption::BandedCols,
            Cmd::FilterButton => TableOption::FilterButton,
            _ => return None,
        })
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
        match self.option() {
            Some(option) => active_table(app).is_some_and(|table| table.option(option)),
            None => false,
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        let table = active_table(app);
        match self {
            // Excel greys Insert > Table out inside a table.
            Cmd::Insert => table.is_none(),
            Cmd::FormatAs | Cmd::RemoveDuplicates | Cmd::NewStyle => true,
            // No table is connected to a source of data outside the workbook.
            Cmd::Refresh => false,
            // The buttons sit on the header row.
            Cmd::FilterButton => table.is_some_and(|table| table.header_row),
            Cmd::InsertColumnsLeft
            | Cmd::InsertColumnRight
            | Cmd::InsertRowsAbove
            | Cmd::InsertRowBelow
            | Cmd::DeleteColumns
            | Cmd::DeleteRows
            | Cmd::SelectColumnData
            | Cmd::SelectEntireColumn
            | Cmd::SelectRow => menu::enabled(self, app),
            _ => table.is_some(),
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        // A ribbon button pressed while a cell is being typed stores the
        // entry first.
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        if let Some(option) = self.option() {
            let label = match option {
                TableOption::HeaderRow => t("Toggle Header Row", "Activer/Désactiver la ligne d'en-têtes"),
                TableOption::TotalsRow => t("Toggle Total Row", "Afficher/Masquer la ligne Total"),
                _ => self.label(),
            };
            return toggle_option(app, option, label, cx);
        }
        match self {
            Cmd::Insert => dialogs::open_create(app, None, window, cx),
            Cmd::FormatAs => styles::open_gallery(app, false, window, cx),
            Cmd::Styles => styles::open_gallery(app, true, window, cx),
            Cmd::ClearStyle => set_style(app, None, cx),
            Cmd::Resize => dialogs::open_resize(app, window, cx),
            Cmd::ConvertToRange => convert_to_range(app, cx),
            // Both dialogs belong to their own modules and open on the table
            // the active cell is in.
            Cmd::SummarizeWithPivot => pivot::Cmd::Insert.run(app, window, cx),
            Cmd::RemoveDuplicates => filters::Cmd::RemoveDuplicates.run(app, window, cx),
            Cmd::InsertColumnsLeft
            | Cmd::InsertColumnRight
            | Cmd::InsertRowsAbove
            | Cmd::InsertRowBelow
            | Cmd::DeleteColumns
            | Cmd::DeleteRows
            | Cmd::SelectColumnData
            | Cmd::SelectEntireColumn
            | Cmd::SelectRow => menu::run(self, app, cx),
            _ => app.not_implemented(self.label(), cx),
        }
    }
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

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        design::groups(app, window, cx)
    }

    fn paint(&self, p: &mut Painter) {
        sheet::paint(p);
    }

    fn zone_mouse_down(
        &self,
        app: &mut ExcelApp,
        zone: &Zone,
        ev: &ZoneEvent,
        window: &mut Window,
        cx: &mut Cx,
    ) -> ZoneResponse {
        sheet::zone_mouse_down(app, zone, ev, window, cx)
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        sheet::zone_drag(app, zone, ev, cx);
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, _ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        sheet::zone_mouse_up(app, zone, cx);
    }

    fn may_commit(&self, app: &mut ExcelApp, _edit: &PendingEdit, _window: &mut Window, _cx: &mut Cx) -> Verdict {
        sheet::entry_committing(app);
        Verdict::Accept
    }

    fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _cx: &mut Cx) {
        sheet::cells_written(app, edit);
    }

    fn cells_changed(&self, app: &mut ExcelApp, _edit: &CellEdit, _cx: &mut Cx) {
        sheet::cells_changed(app);
    }

    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        design::selection_changed(app, cx);
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        menu::add_entries(app, target, menu);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_have_excels_words() {
        assert_eq!(
            refusal(TableError::Overlap, Doing::Creating),
            Some("A table can't overlap another table.")
        );
        // A range that does not suit is told apart by what was being done.
        assert_eq!(
            refusal(TableError::InvalidRange, Doing::Creating),
            Some("Reference isn't valid.")
        );
        assert!(
            refusal(TableError::InvalidRange, Doing::Resizing)
                .unwrap()
                .starts_with("This range won't work")
        );
        assert!(
            refusal(TableError::InvalidResize, Doing::Resizing)
                .unwrap()
                .starts_with("This range won't work")
        );
        assert!(
            refusal(TableError::Blocked, Doing::Other)
                .unwrap()
                .contains("move cells in a table")
        );
        assert!(
            refusal(TableError::Blocked, Doing::Creating)
                .unwrap()
                .contains("move cells in a table")
        );
        assert!(
            refusal(TableError::Blocked, Doing::CreatingOverFilter)
                .unwrap()
                .contains("AutoFilter")
        );
        assert!(
            refusal(TableError::InvalidName, Doing::Other)
                .unwrap()
                .starts_with("The syntax of this name")
        );
        // Nothing to say about what the commands never ask.
        assert_eq!(refusal(TableError::NotFound, Doing::Other), None);
        assert_eq!(refusal(TableError::NoHeaderRow, Doing::Other), None);
    }

    #[test]
    fn option_commands() {
        assert_eq!(Cmd::TotalRow.option(), Some(TableOption::TotalsRow));
        assert_eq!(Cmd::TotalsRow.option(), Some(TableOption::TotalsRow));
        assert_eq!(Cmd::FilterButton.option(), Some(TableOption::FilterButton));
        assert_eq!(Cmd::Resize.option(), None);
    }
}
