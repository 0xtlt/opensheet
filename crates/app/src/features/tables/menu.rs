//! The context menu of a table cell: Insert, Delete and Select work on the
//! rows and columns of the table rather than those of the sheet, and
//! Table holds Totals Row and Convert to Range.

use super::actions::Reach;
use super::{Cmd, Doing, refuse};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, ContextMenu, Cx, Intent, MenuSection, MenuTarget};
use crate::i18n::{t, t_ctx};
use opensheet_engine::table::TableError;

/// What the selection covers of the table of the active cell.
fn reach(app: &ExcelApp) -> Option<Reach> {
    Reach::of(&app.wb, app.sheet, app.active(), app.sel().current())
}

/// Whether a command of the menu has something to work on.
pub(super) fn enabled(cmd: Cmd, app: &ExcelApp) -> bool {
    let Some(reach) = reach(app) else {
        return false;
    };
    match cmd {
        Cmd::InsertRowsAbove => reach.rows_above.is_some(),
        Cmd::InsertRowBelow => reach.row_below.is_some(),
        Cmd::InsertColumnRight => reach.col_right.is_some(),
        Cmd::DeleteRows => reach.data_rows.is_some(),
        _ => true,
    }
}

/// The names of the submenus and the commands in each, as Excel lists
/// them for this selection: a row below and a column to the right are
/// only offered from the last ones.
fn submenus(reach: &Reach) -> Vec<(&'static str, Vec<Cmd>)> {
    let mut insert = vec![Cmd::InsertColumnsLeft];
    if reach.col_right.is_some() {
        insert.push(Cmd::InsertColumnRight);
    }
    insert.push(Cmd::InsertRowsAbove);
    if reach.row_below.is_some() {
        insert.push(Cmd::InsertRowBelow);
    }
    vec![
        (t_ctx("table", "Insert", "Insérer"), insert),
        (
            t_ctx("table", "Delete", "Supprimer"),
            vec![Cmd::DeleteColumns, Cmd::DeleteRows],
        ),
        (
            t_ctx("table", "Select", "Sélectionner"),
            vec![Cmd::SelectColumnData, Cmd::SelectEntireColumn, Cmd::SelectRow],
        ),
    ]
}

pub(super) fn add_entries(app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
    if !matches!(target, MenuTarget::Cell(..)) {
        return;
    }
    let Some(reach) = reach(app) else {
        return;
    };
    for (title, commands) in submenus(&reach) {
        let entries = commands.into_iter().map(|cmd| MenuEntry::command(cmd, app)).collect();
        menu.add(MenuSection::Edit, MenuEntry::submenu(title, entries));
    }
    menu.add(
        MenuSection::Data,
        MenuEntry::submenu(
            t("Table", "Tableau"),
            vec![
                MenuEntry::command(Cmd::TotalsRow, app),
                MenuEntry::command(Cmd::ConvertToRange, app),
            ],
        ),
    );
}

/// Run a command of the Insert, Delete or Select submenu.
pub(super) fn run(cmd: Cmd, app: &mut ExcelApp, cx: &mut Cx) {
    let Some(reach) = reach(app) else {
        return;
    };
    let (sheet, id) = (app.sheet, reach.table);
    // Selecting changes nothing: it is always allowed.
    let selection = app.wb.table(sheet, id).and_then(|table| match cmd {
        Cmd::SelectColumnData => reach.column_data(table),
        Cmd::SelectEntireColumn => Some(reach.entire_columns(table)),
        Cmd::SelectRow => Some(reach.table_rows(table)),
        _ => None,
    });
    if let Some(rect) = selection {
        return app.select_rect(rect, cx);
    }
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let at = |place: Option<(u32, u32)>| place.ok_or(TableError::InvalidRange);
    let result = match cmd {
        Cmd::InsertColumnsLeft => app.wb.table_insert_cols(sheet, id, reach.cols.0, reach.cols.1),
        Cmd::InsertColumnRight => at(reach.col_right.map(|col| (col, 1)))
            .and_then(|(col, count)| app.wb.table_insert_cols(sheet, id, col, count)),
        Cmd::InsertRowsAbove => {
            at(reach.rows_above).and_then(|(row, count)| app.wb.table_insert_rows(sheet, id, row, count))
        }
        Cmd::InsertRowBelow => at(reach.row_below.map(|row| (row, 1)))
            .and_then(|(row, count)| app.wb.table_insert_rows(sheet, id, row, count)),
        Cmd::DeleteColumns => app.wb.table_delete_cols(sheet, id, reach.cols.0, reach.cols.1),
        Cmd::DeleteRows => at(reach.data_rows).and_then(|(row, count)| app.wb.table_delete_rows(sheet, id, row, count)),
        _ => return,
    };
    match result {
        Ok(()) => {
            let label = match cmd {
                Cmd::DeleteColumns => t("Delete Column", "Supprimer la colonne"),
                Cmd::DeleteRows => t("Delete Row", "Supprimer la ligne"),
                _ => cmd.label(),
            };
            app.commit(label, cx);
        }
        Err(error) => refuse(app, error, Doing::Other, cx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::{Locale, Rect, Workbook};

    fn reach_of(active: (u32, u32), selection: Rect) -> Reach {
        let mut wb = Workbook::new(Locale::en());
        for row in 0..4 {
            for col in 0..3 {
                wb.set_cell_text(0, row, col, &format!("v{row}{col}")).unwrap();
            }
        }
        wb.add_table(0, Rect::new(0, 0, 3, 2), true).unwrap();
        wb.commit("table");
        Reach::of(&wb, 0, active, selection).unwrap()
    }

    fn listed(reach: &Reach) -> Vec<Vec<Cmd>> {
        submenus(reach).into_iter().map(|(_, commands)| commands).collect()
    }

    #[test]
    fn submenus_are_excels() {
        // In the middle of the table: no row below, no column to the right.
        let menus = listed(&reach_of((2, 1), Rect::cell(2, 1)));
        assert_eq!(menus.len(), 3);
        assert_eq!(menus[0], [Cmd::InsertColumnsLeft, Cmd::InsertRowsAbove]);
        assert_eq!(menus[1], [Cmd::DeleteColumns, Cmd::DeleteRows]);
        assert_eq!(
            menus[2],
            [Cmd::SelectColumnData, Cmd::SelectEntireColumn, Cmd::SelectRow]
        );
        // From the last cell both are offered, in Excel's order.
        let menus = listed(&reach_of((3, 2), Rect::cell(3, 2)));
        assert_eq!(
            menus[0],
            [
                Cmd::InsertColumnsLeft,
                Cmd::InsertColumnRight,
                Cmd::InsertRowsAbove,
                Cmd::InsertRowBelow
            ]
        );
        // The last column alone.
        let menus = listed(&reach_of((1, 2), Rect::cell(1, 2)));
        assert_eq!(
            menus[0],
            [Cmd::InsertColumnsLeft, Cmd::InsertColumnRight, Cmd::InsertRowsAbove]
        );
    }
}
