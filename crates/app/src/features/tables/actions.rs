//! The questions the table commands ask of the workbook, and the few
//! things they do to it that the engine has no single call for. Everything
//! here is a function of the workbook alone, so it is tested without a
//! window.

use crate::i18n::t_ctx;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::table::{Table, TableError, TotalsFunction};
use opensheet_engine::{Cell, Expr, Locale, Rect, Reference, Workbook, parser, structured};

/// The range typed in the field of a table dialog: `A1:C9` or
/// `=$A$1:$C$9`, possibly after the name of the sheet the dialog was
/// opened on (what picking cells on the sheet writes). `None` for anything
/// else.
pub fn parse_range(text: &str, sheet_name: &str) -> Option<Rect> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text).trim();
    let cells = match text.rsplit_once('!') {
        Some((sheet, cells)) => {
            let sheet = sheet.trim();
            let name = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                Some(quoted) => quoted.replace("''", "'"),
                None => sheet.to_string(),
            };
            if name.to_lowercase() != sheet_name.to_lowercase() {
                return None;
            }
            cells
        }
        None => text,
    };
    Rect::parse(cells)
}

/// Whether the first row of `range` holds a formula: Excel warns before
/// making it the header row of a table, where it becomes plain text.
pub fn header_formulas(wb: &Workbook, sheet: usize, range: Rect) -> bool {
    let range = wb.clamp_to_used(sheet, range);
    (range.c0..=range.c1).any(|col| wb.formula_text(sheet, range.r0, col).is_some())
}

/// The range a table takes when its corner handle is dragged onto the
/// cell `pointer`: the top-left corner stays where it is, and the table
/// keeps its header row, its totals row and one row of data.
pub fn drag_range(table: &Table, pointer: (u32, u32)) -> Rect {
    let range = table.range;
    let least = range.r0 + table.header_row as u32 + table.totals_row as u32;
    Rect::new(range.r0, range.c0, pointer.0.max(least), pointer.1.max(range.c0))
}

/// The cells whose contents resizing a table to `new` would replace with
/// its totals row (Excel asks before): the last row of `new`, when the
/// totals row moves there and something is in the way.
pub fn totals_overwrite(wb: &Workbook, sheet: usize, table: &Table, new: Rect) -> Option<Rect> {
    if !table.totals_row || new.r1 == table.range.r1 {
        return None;
    }
    let sh = wb.sheets.get(sheet)?;
    (new.c0..=new.c1)
        .any(|col| !sh.is_blank(new.r1, col))
        .then(|| Rect::new(new.r1, new.c0, new.r1, new.c1))
}

/// Why a name was refused for a table, as Excel tells the two cases apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameProblem {
    /// Not a valid name, or the name of something that is not a table.
    Syntax,
    /// Another table is called that.
    TableExists,
}

pub fn name_problem(wb: &Workbook, error: TableError, name: &str) -> NameProblem {
    if error == TableError::NameTaken && wb.table_by_name(name.trim()).is_some() {
        NameProblem::TableExists
    } else {
        NameProblem::Syntax
    }
}

/// The list of a totals cell, in Excel's order ("More Functions..." comes
/// after it).
pub fn totals_choices() -> [(&'static str, TotalsFunction); 9] {
    [
        (t_ctx("totals", "None", "Aucun"), TotalsFunction::None),
        (t_ctx("totals", "Average", "Moyenne"), TotalsFunction::Average),
        (t_ctx("totals", "Count", "Nombre"), TotalsFunction::Count),
        (t_ctx("totals", "Count Numbers", "Chiffres"), TotalsFunction::CountNums),
        (t_ctx("totals", "Max", "Max."), TotalsFunction::Max),
        (t_ctx("totals", "Min", "Min."), TotalsFunction::Min),
        (t_ctx("totals", "Sum", "Somme"), TotalsFunction::Sum),
        (t_ctx("totals", "StdDev", "Écartype"), TotalsFunction::StdDev),
        (t_ctx("totals", "Var", "Var"), TotalsFunction::Var),
    ]
}

// ---------------------------------------------------------------------- Tab

/// Where Tab takes the active cell of a table when that is not the next
/// cell of the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabMove {
    /// Onto this cell: the first of the next table row, or the last of the
    /// previous one.
    To(u32, u32),
    /// A row is added to the table at sheet row `at`, and the active cell
    /// goes to its first cell (column `col`).
    AddRow { table: u32, at: u32, col: u32 },
}

/// What Tab (`backward`: Shift+Tab) does from the cell `active` of a
/// table. As in Excel, Tab in the last column goes on with the first cell
/// of the next row, and from the last cell of the data it adds a row; rows
/// and columns that are not shown are passed over. `None` leaves the move
/// to the sheet.
pub fn tab_move(wb: &Workbook, sheet: usize, active: (u32, u32), backward: bool) -> Option<TabMove> {
    let (row, col) = active;
    let table = wb.table_at(sheet, row, col)?;
    let sh = wb.sheets.get(sheet)?;
    let range = table.range;
    let shown = |col: &u32| !sh.hidden_cols.contains(col);
    let first = (range.c0..=range.c1).find(shown)?;
    let last = (range.c0..=range.c1).rev().find(shown)?;
    if backward {
        if col != first {
            return None;
        }
        let above = (range.r0..row).rev().find(|r| !sh.is_row_hidden(*r))?;
        return Some(TabMove::To(above, last));
    }
    if col != last {
        return None;
    }
    if table.data_range().is_some_and(|data| row == data.r1) {
        return Some(TabMove::AddRow {
            table: table.id,
            at: row + 1,
            col: first,
        });
    }
    let below = (row + 1..=range.r1).find(|r| !sh.is_row_hidden(*r))?;
    Some(TabMove::To(below, first))
}

// ------------------------------------------------------------- context menu

/// The part of a table a selection covers: what the Insert, Delete and
/// Select submenus of a table cell work on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    pub table: u32,
    /// The table columns of the selection: first sheet column and count.
    pub cols: (u32, u32),
    /// The table rows of the selection, header and totals rows included:
    /// first sheet row and count.
    pub rows: (u32, u32),
    /// Its data rows among them; `None` for a selection in the header or
    /// the totals row alone.
    pub data_rows: Option<(u32, u32)>,
    /// Where "Table Rows Above" inserts, and how many rows: above the data
    /// rows of the selection, or above the totals row. Nothing goes above
    /// the header row.
    pub rows_above: Option<(u32, u32)>,
    /// The sheet row of "Table Row Below", offered from the last data row.
    pub row_below: Option<u32>,
    /// The sheet column of "Table Column to the Right", offered from the
    /// last column.
    pub col_right: Option<u32>,
}

impl Reach {
    /// What `selection` covers of the table holding the cell `active`.
    pub fn of(wb: &Workbook, sheet: usize, active: (u32, u32), selection: Rect) -> Option<Reach> {
        let table = wb.table_at(sheet, active.0, active.1)?;
        let range = table.range;
        let hit = selection.intersection(&range).unwrap_or(Rect::cell(active.0, active.1));
        let data = table.data_range();
        let in_data = data.and_then(|d| d.intersection(&hit));
        let data_rows = in_data.map(|d| (d.r0, d.rows()));
        let on_totals = table.totals_row && hit.r1 == range.r1;
        Some(Reach {
            table: table.id,
            cols: (hit.c0, hit.cols()),
            rows: (hit.r0, hit.rows()),
            data_rows,
            rows_above: data_rows.or(on_totals.then_some((range.r1, 1))),
            row_below: match (data, in_data) {
                (Some(data), Some(d)) if d.r1 == data.r1 => Some(data.r1 + 1),
                _ => None,
            },
            col_right: (hit.c1 == range.c1).then_some(range.c1 + 1),
        })
    }

    /// "Table Column Data": the data cells of the columns.
    pub fn column_data(&self, table: &Table) -> Option<Rect> {
        let data = table.data_range()?;
        Some(Rect::new(data.r0, self.cols.0, data.r1, self.cols.0 + self.cols.1 - 1))
    }

    /// "Entire Table Column": the columns with their header and total.
    pub fn entire_columns(&self, table: &Table) -> Rect {
        let range = table.range;
        Rect::new(range.r0, self.cols.0, range.r1, self.cols.0 + self.cols.1 - 1)
    }

    /// "Table Row": the rows across the table.
    pub fn table_rows(&self, table: &Table) -> Rect {
        let range = table.range;
        Rect::new(self.rows.0, range.c0, self.rows.0 + self.rows.1 - 1, range.c1)
    }
}

// ------------------------------------------------- names typed and pointed at

/// The cells a table name or a structured reference typed in the Name Box
/// stands for, seen from the cell `active`: `Table1` is the data of the
/// table, `Table1[#All]` all of it, `Table1[Sales]` one column...
pub fn go_to_target(wb: &Workbook, sheet: usize, active: (u32, u32), text: &str) -> Option<Reference> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text);
    match parser::parse_formula(text, wb.locale).ok()? {
        Expr::Name(name) => {
            let (table_sheet, table) = wb.table_by_name(&name)?;
            Some(Reference::new(table_sheet, table.data_range()?))
        }
        Expr::Table(reference) => structured::resolve(wb, &reference, sheet, active.0, active.1).ok(),
        _ => None,
    }
}

/// What pointing at `rect` writes in the formula being typed in the cell
/// `from` (`None`: a cell of another sheet) when Excel writes a structured
/// reference there (`Table1[Sales]`, `[@Sales]`) rather than `C2:C9`.
pub fn pointed_reference(wb: &Workbook, sheet: usize, rect: Rect, from: Option<(u32, u32)>) -> Option<String> {
    // A formula typed right beside a table belongs to it, since the table
    // grows over it: the name of the table is left out there too.
    let from = from.map(|(row, col)| match wb.table_for_entry(sheet, row, col) {
        Some(table) if !table.contains(row, col) && table.contains(row, table.range.c1) => (row, table.range.c1),
        _ => (row, col),
    });
    let reference = wb.structured_reference(sheet, rect, from)?;
    // The engine spells the item specifiers in English (`#Headers`); a
    // formula is typed in the language of the workbook.
    Some(match parser::parse_formula(&reference, Locale::en()) {
        Ok(expr) => expr.to_text(wb.locale),
        Err(_) => reference,
    })
}

// ------------------------------------------------- what tables do by themselves

/// What a table did by itself after cells were changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoKind {
    /// It grew over what was entered beside it.
    Expansion,
    /// A formula entered in one cell filled its column.
    CalculatedColumn,
}

/// The last thing a table did by itself, kept so that it can be taken back
/// while the cells stay as the user entered them ("Undo Table
/// AutoExpansion", "Undo Calculated Column").
#[derive(Clone, Debug)]
pub struct AutoChange {
    pub sheet: usize,
    /// The cells the user changed.
    pub rect: Rect,
    pub kind: AutoKind,
    /// Those cells as the user left them, before the table reacted.
    cells: Vec<((u32, u32), Option<Cell>)>,
    /// `Workbook::generation` once the change was committed: the offer
    /// stands until the workbook changes again.
    pub generation: Option<u64>,
}

/// More cells than this are not kept aside for an `AutoChange`.
const MAX_KEPT: u32 = 4096;

/// Whether changing `rect` may make a table of the sheet react: it lies
/// in one, or against its bottom or right edge.
pub fn near_tables(wb: &Workbook, sheet: usize, rect: Rect) -> bool {
    wb.sheets.get(sheet).is_some_and(|sh| {
        sh.tables.iter().any(|table| {
            let r = table.range;
            Rect::new(r.r0, r.c0, (r.r1 + 1).min(MAX_ROWS - 1), (r.c1 + 1).min(MAX_COLS - 1)).intersects(&rect)
        })
    })
}

impl AutoChange {
    /// Keep the cells of `rect` aside, as they are now. `None` when there
    /// are too many of them to be worth an offer.
    pub fn before(wb: &Workbook, sheet: usize, rect: Rect) -> Option<AutoChange> {
        let rect = wb.clamp_to_used(sheet, rect);
        if rect.rows().saturating_mul(rect.cols()) > MAX_KEPT {
            return None;
        }
        let mut cells = Vec::with_capacity((rect.rows() * rect.cols()) as usize);
        for row in rect.r0..=rect.r1 {
            for col in rect.c0..=rect.c1 {
                cells.push(((row, col), wb.cell(sheet, row, col).cloned()));
            }
        }
        Some(AutoChange {
            sheet,
            rect,
            kind: AutoKind::Expansion,
            cells,
            generation: None,
        })
    }

    /// Whether the change is still the last thing that happened to the
    /// workbook.
    pub fn stands(&self, wb: &Workbook) -> bool {
        self.generation == Some(wb.generation)
    }

    /// Take back what the table did: the last step is undone whole, then
    /// the cells are entered again as the user left them, as a step of the
    /// same name. Returns false when the change no longer stands.
    pub fn undo(&self, wb: &mut Workbook) -> bool {
        if !self.stands(wb) {
            return false;
        }
        let Some(label) = wb.undo_label().map(str::to_string) else {
            return false;
        };
        if !wb.undo() {
            return false;
        }
        for ((row, col), cell) in &self.cells {
            wb.put_cell(self.sheet, *row, *col, cell.clone());
        }
        wb.commit(&label);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Value;
    use opensheet_engine::table::TableOption;

    /// Sheet 0: a table on A1:C4 (headers Product, Qty, Price; three rows
    /// of data).
    fn sample() -> (Workbook, u32) {
        let mut wb = Workbook::new(Locale::en());
        let rows = [
            ["Product", "Qty", "Price"],
            ["Pen", "3", "2"],
            ["Ink", "5", "4"],
            ["Pad", "7", "6"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        let id = wb.add_table(0, Rect::new(0, 0, 3, 2), true).unwrap();
        wb.commit("table");
        (wb, id)
    }

    #[test]
    fn ranges_of_dialogs() {
        assert_eq!(parse_range("=$A$1:$C$4", "Sheet1"), Some(Rect::new(0, 0, 3, 2)));
        assert_eq!(parse_range(" b2:a1 ", "Sheet1"), Some(Rect::new(0, 0, 1, 1)));
        assert_eq!(parse_range("=Sheet1!$A$1", "sheet1"), Some(Rect::cell(0, 0)));
        assert_eq!(parse_range("='It''s'!A1:B2", "It's"), Some(Rect::new(0, 0, 1, 1)));
        assert_eq!(parse_range("=Sheet2!A1:B2", "Sheet1"), None);
        assert_eq!(parse_range("Table1", "Sheet1"), None);
        assert_eq!(parse_range("", "Sheet1"), None);
    }

    #[test]
    fn header_formulas_are_noticed() {
        let (mut wb, _) = sample();
        assert!(!header_formulas(&wb, 0, Rect::new(5, 0, 7, 1)));
        wb.set_cell_text(0, 5, 1, "=1+1").unwrap();
        wb.commit("formula");
        assert!(header_formulas(&wb, 0, Rect::new(5, 0, 7, 1)));
        assert!(!header_formulas(&wb, 0, Rect::new(6, 0, 7, 1)));
    }

    #[test]
    fn handle_keeps_the_table_a_table() {
        let (mut wb, id) = sample();
        let table = wb.table(0, id).unwrap().clone();
        assert_eq!(drag_range(&table, (6, 4)), Rect::new(0, 0, 6, 4));
        // Not above the first data row, nor left of the first column.
        assert_eq!(drag_range(&table, (0, 0)), Rect::new(0, 0, 1, 0));
        wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        wb.commit("totals");
        let table = wb.table(0, id).unwrap().clone();
        assert_eq!(drag_range(&table, (0, 1)), Rect::new(0, 0, 2, 1));
    }

    #[test]
    fn totals_row_asks_before_overwriting() {
        let (mut wb, id) = sample();
        let table = wb.table(0, id).unwrap().clone();
        // Without a totals row nothing is overwritten.
        assert_eq!(totals_overwrite(&wb, 0, &table, Rect::new(0, 0, 2, 2)), None);
        wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        wb.commit("totals");
        let table = wb.table(0, id).unwrap().clone();
        assert_eq!(table.range, Rect::new(0, 0, 4, 2));
        // Shrinking puts the totals on a row of data.
        assert_eq!(
            totals_overwrite(&wb, 0, &table, Rect::new(0, 0, 3, 2)),
            Some(Rect::new(3, 0, 3, 2))
        );
        // Growing over blank rows, or sideways, overwrites nothing.
        assert_eq!(totals_overwrite(&wb, 0, &table, Rect::new(0, 0, 7, 2)), None);
        assert_eq!(totals_overwrite(&wb, 0, &table, Rect::new(0, 0, 4, 3)), None);
    }

    #[test]
    fn name_problems() {
        let (mut wb, id) = sample();
        wb.set_cell_text(0, 0, 5, "Other").unwrap();
        wb.set_cell_text(0, 1, 5, "1").unwrap();
        let other = wb.add_table(0, Rect::new(0, 5, 1, 5), true).unwrap();
        wb.commit("second table");
        let taken = wb.table(0, id).unwrap().name.clone();
        let error = wb.rename_table(0, other, &taken).unwrap_err();
        assert_eq!(name_problem(&wb, error, &taken), NameProblem::TableExists);
        let error = wb.rename_table(0, other, "my table").unwrap_err();
        assert_eq!(name_problem(&wb, error, "my table"), NameProblem::Syntax);
        let error = wb.rename_table(0, other, "A1").unwrap_err();
        assert_eq!(name_problem(&wb, error, "A1"), NameProblem::Syntax);
    }

    #[test]
    fn totals_list_is_excels() {
        let functions: Vec<TotalsFunction> = totals_choices().into_iter().map(|(_, f)| f).collect();
        assert_eq!(functions.first(), Some(&TotalsFunction::None));
        assert_eq!(functions.len(), 9);
        assert_eq!(functions[3], TotalsFunction::CountNums);
        assert_eq!(functions[6], TotalsFunction::Sum);
    }

    #[test]
    fn tab_walks_the_table() {
        let (mut wb, id) = sample();
        // Inside a row, Tab is the sheet's business.
        assert_eq!(tab_move(&wb, 0, (1, 0), false), None);
        assert_eq!(tab_move(&wb, 0, (1, 1), true), None);
        // From the last column to the first cell of the next row.
        assert_eq!(tab_move(&wb, 0, (0, 2), false), Some(TabMove::To(1, 0)));
        assert_eq!(tab_move(&wb, 0, (1, 2), false), Some(TabMove::To(2, 0)));
        // And back.
        assert_eq!(tab_move(&wb, 0, (2, 0), true), Some(TabMove::To(1, 2)));
        assert_eq!(tab_move(&wb, 0, (0, 0), true), None);
        // The last cell adds a row.
        assert_eq!(
            tab_move(&wb, 0, (3, 2), false),
            Some(TabMove::AddRow {
                table: id,
                at: 4,
                col: 0
            })
        );
        // Outside the table nothing changes.
        assert_eq!(tab_move(&wb, 0, (3, 3), false), None);
        // With a totals row the new row comes above it, and the last cell
        // of the totals row leaves the table.
        wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        wb.commit("totals");
        assert_eq!(
            tab_move(&wb, 0, (3, 2), false),
            Some(TabMove::AddRow {
                table: id,
                at: 4,
                col: 0
            })
        );
        assert_eq!(tab_move(&wb, 0, (4, 2), false), None);
        assert_eq!(tab_move(&wb, 0, (4, 0), true), Some(TabMove::To(3, 2)));
    }

    #[test]
    fn tab_passes_over_what_is_hidden() {
        let (mut wb, _) = sample();
        wb.set_rows_hidden(0, 2, 2, true);
        wb.set_cols_hidden(0, 2, 2, true);
        wb.commit("hide");
        // Column C is hidden: B is the last column Tab stops on.
        assert_eq!(tab_move(&wb, 0, (1, 1), false), Some(TabMove::To(3, 0)));
        assert_eq!(tab_move(&wb, 0, (3, 0), true), Some(TabMove::To(1, 1)));
    }

    #[test]
    fn reach_of_a_selection() {
        let (mut wb, id) = sample();
        let table = wb.table(0, id).unwrap().clone();
        // One data cell in the middle.
        let reach = Reach::of(&wb, 0, (2, 1), Rect::cell(2, 1)).unwrap();
        assert_eq!(reach.table, id);
        assert_eq!(reach.cols, (1, 1));
        assert_eq!(reach.data_rows, Some((2, 1)));
        assert_eq!(reach.rows_above, Some((2, 1)));
        assert_eq!(reach.row_below, None);
        assert_eq!(reach.col_right, None);
        assert_eq!(reach.column_data(&table), Some(Rect::new(1, 1, 3, 1)));
        assert_eq!(reach.entire_columns(&table), Rect::new(0, 1, 3, 1));
        assert_eq!(reach.table_rows(&table), Rect::new(2, 0, 2, 2));
        // The last cell: a row below and a column to the right are offered.
        let reach = Reach::of(&wb, 0, (3, 2), Rect::cell(3, 2)).unwrap();
        assert_eq!(reach.row_below, Some(4));
        assert_eq!(reach.col_right, Some(3));
        // A selection reaching outside the table counts for what is in it.
        let reach = Reach::of(&wb, 0, (2, 1), Rect::new(2, 1, 9, 9)).unwrap();
        assert_eq!(reach.cols, (1, 2));
        assert_eq!(reach.data_rows, Some((2, 2)));
        assert_eq!(reach.row_below, Some(4));
        // The header row: no rows to insert above or to delete.
        let reach = Reach::of(&wb, 0, (0, 0), Rect::cell(0, 0)).unwrap();
        assert_eq!(reach.data_rows, None);
        assert_eq!(reach.rows_above, None);
        assert_eq!(reach.rows, (0, 1));
        // The totals row: a row above it is a new last row of data.
        wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
        wb.commit("totals");
        let reach = Reach::of(&wb, 0, (4, 0), Rect::cell(4, 0)).unwrap();
        assert_eq!(reach.data_rows, None);
        assert_eq!(reach.rows_above, Some((4, 1)));
        assert_eq!(reach.row_below, None);
        // Not in a table.
        assert_eq!(Reach::of(&wb, 0, (9, 9), Rect::cell(9, 9)), None);
    }

    #[test]
    fn reach_drives_the_engine() {
        let (mut wb, id) = sample();
        let reach = Reach::of(&wb, 0, (2, 1), Rect::new(2, 1, 3, 1)).unwrap();
        let (at, count) = reach.rows_above.unwrap();
        wb.table_insert_rows(0, id, at, count).unwrap();
        wb.commit("insert");
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 5, 2));
        assert_eq!(wb.value_at(0, 4, 0), Value::text("Ink"));
        let reach = Reach::of(&wb, 0, (5, 2), Rect::cell(5, 2)).unwrap();
        wb.table_insert_cols(0, id, reach.col_right.unwrap(), 1).unwrap();
        wb.table_insert_rows(0, id, reach.row_below.unwrap(), 1).unwrap();
        wb.commit("grow");
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 6, 3));
        let reach = Reach::of(&wb, 0, (2, 0), Rect::new(2, 0, 3, 0)).unwrap();
        let (at, count) = reach.data_rows.unwrap();
        wb.table_delete_rows(0, id, at, count).unwrap();
        wb.table_delete_cols(0, id, reach.cols.0, reach.cols.1).unwrap();
        wb.commit("shrink");
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 4, 2));
        assert_eq!(wb.value_at(0, 0, 0), Value::text("Qty"));
    }

    #[test]
    fn names_in_the_name_box() {
        let (mut wb, id) = sample();
        let name = wb.table(0, id).unwrap().name.clone();
        let at = |wb: &Workbook, text: &str| go_to_target(wb, 0, (0, 0), text).map(|r| (r.sheet, r.rect));
        assert_eq!(at(&wb, &name), Some((0, Rect::new(1, 0, 3, 2))));
        assert_eq!(at(&wb, &name.to_lowercase()), Some((0, Rect::new(1, 0, 3, 2))));
        assert_eq!(at(&wb, &format!("{name}[#All]")), Some((0, Rect::new(0, 0, 3, 2))));
        assert_eq!(at(&wb, &format!("{name}[Qty]")), Some((0, Rect::new(1, 1, 3, 1))));
        assert_eq!(
            at(&wb, &format!("={name}[[#Headers],[Price]]")),
            Some((0, Rect::cell(0, 2)))
        );
        assert_eq!(at(&wb, "Nothing"), None);
        assert_eq!(at(&wb, &format!("{name}[Missing]")), None);
        assert_eq!(at(&wb, "1+1"), None);
        // From another sheet too.
        wb.add_sheet(1);
        wb.commit("sheet");
        assert_eq!(
            go_to_target(&wb, 1, (5, 5), &name).map(|r| (r.sheet, r.rect)),
            Some((0, Rect::new(1, 0, 3, 2)))
        );
    }

    #[test]
    fn pointing_writes_structured_references() {
        let (wb, id) = sample();
        let name = wb.table(0, id).unwrap().name.clone();
        // A whole column of data, from outside the table.
        assert_eq!(
            pointed_reference(&wb, 0, Rect::new(1, 1, 3, 1), Some((6, 6))),
            Some(format!("{name}[Qty]"))
        );
        // A cell of the formula's own row, from inside the table.
        assert_eq!(
            pointed_reference(&wb, 0, Rect::cell(2, 1), Some((2, 2))),
            Some("[@Qty]".to_string())
        );
        // From the cell right beside the table too: the formula will be in
        // the table once entered. Further away the table is named.
        assert_eq!(
            pointed_reference(&wb, 0, Rect::cell(2, 1), Some((2, 3))),
            Some("[@Qty]".to_string())
        );
        assert_eq!(
            pointed_reference(&wb, 0, Rect::cell(2, 1), Some((2, 5))),
            Some(format!("{name}[@Qty]"))
        );
        // Any other single cell is a plain reference.
        assert_eq!(pointed_reference(&wb, 0, Rect::cell(2, 1), Some((6, 6))), None);
        assert_eq!(pointed_reference(&wb, 0, Rect::cell(9, 9), Some((6, 6))), None);
    }

    #[test]
    fn pointed_references_speak_the_workbook_language() {
        let mut wb = Workbook::new(Locale::fr());
        for (c, text) in ["Produit", "Qté"].iter().enumerate() {
            wb.set_cell_text(0, 0, c as u32, text).unwrap();
            wb.set_cell_text(0, 1, c as u32, "1").unwrap();
        }
        let id = wb.add_table(0, Rect::new(0, 0, 1, 1), true).unwrap();
        wb.commit("table");
        let name = wb.table(0, id).unwrap().name.clone();
        let headers = pointed_reference(&wb, 0, Rect::new(0, 0, 0, 1), Some((5, 5))).unwrap();
        assert_eq!(headers, format!("{name}[#En-têtes]"));
        // Column names are kept as they are, accents and all; the separator
        // between an item and a column is the workbook's.
        assert_eq!(
            pointed_reference(&wb, 0, Rect::cell(1, 1), Some((5, 5))),
            Some(format!("{name}[Qté]"))
        );
        let header = pointed_reference(&wb, 0, Rect::cell(0, 1), Some((5, 5))).unwrap();
        assert_eq!(header, format!("{name}[[#En-têtes];[Qté]]"));
        assert_eq!(
            go_to_target(&wb, 0, (5, 5), &header).map(|r| r.rect),
            Some(Rect::cell(0, 1))
        );
        // What is written reads back as the same cells.
        assert_eq!(
            go_to_target(&wb, 0, (5, 5), &headers).map(|r| r.rect),
            Some(Rect::new(0, 0, 0, 1))
        );
    }

    #[test]
    fn expansion_is_taken_back_alone() {
        let (mut wb, id) = sample();
        // Type under the table: it grows, in the step of the typing.
        let edited = Rect::cell(4, 0);
        wb.set_cell_text(0, 4, 0, "Cap").unwrap();
        assert!(near_tables(&wb, 0, edited));
        let mut change = AutoChange::before(&wb, 0, edited).unwrap();
        let did = wb.tables_after_edit(0, edited);
        assert_eq!(did.expanded, Some(id));
        wb.commit("Typing");
        change.generation = Some(wb.generation);
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 4, 2));
        // Taken back: the table is as before, the entry is still there.
        assert!(change.stands(&wb));
        assert!(change.undo(&mut wb));
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 3, 2));
        assert_eq!(wb.value_at(0, 4, 0), Value::text("Cap"));
        assert_eq!(wb.undo_label(), Some("Typing"));
        // Once only.
        assert!(!change.stands(&wb));
        assert!(!change.undo(&mut wb));
        // Plain undo then removes the entry.
        wb.undo();
        assert!(wb.sheets[0].is_blank(4, 0));
    }

    #[test]
    fn calculated_column_is_taken_back_alone() {
        let (mut wb, id) = sample();
        let edited = Rect::cell(1, 3);
        // A column of headers first, so the table has a fourth column.
        wb.set_cell_text(0, 0, 3, "Total").unwrap();
        wb.tables_after_edit(0, Rect::cell(0, 3));
        wb.commit("Typing");
        assert_eq!(wb.table(0, id).unwrap().range, Rect::new(0, 0, 3, 3));
        wb.set_cell_text(0, 1, 3, "=B2*C2").unwrap();
        let mut change = AutoChange::before(&wb, 0, edited).unwrap();
        let did = wb.tables_after_edit(0, edited);
        assert_eq!(did.filled, Some((id, 3)));
        change.kind = AutoKind::CalculatedColumn;
        wb.commit("Typing");
        change.generation = Some(wb.generation);
        assert_eq!(wb.value_at(0, 3, 3), Value::Number(42.0));
        assert!(change.undo(&mut wb));
        // Only the cell typed keeps its formula.
        assert_eq!(wb.value_at(0, 1, 3), Value::Number(6.0));
        assert!(wb.sheets[0].is_blank(2, 3));
        assert!(wb.sheets[0].is_blank(3, 3));
    }

    #[test]
    fn large_edits_are_not_kept_aside() {
        let (wb, _) = sample();
        assert!(AutoChange::before(&wb, 0, Rect::new(0, 0, 99, 99)).is_none());
        assert!(AutoChange::before(&wb, 0, Rect::new(0, 0, 9, 9)).is_some());
        assert!(!near_tables(&wb, 0, Rect::cell(5, 0)));
        assert!(near_tables(&wb, 0, Rect::cell(2, 3)));
        assert!(near_tables(&wb, 0, Rect::new(3, 2, 8, 8)));
    }
}
