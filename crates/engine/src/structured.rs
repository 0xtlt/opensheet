//! Structured references (`Table1[Col]`, `[@Col]`): the cells they stand
//! for, and the formula rewrites a table needs when it is renamed, when
//! one of its columns is renamed or deleted, and when it is turned back
//! into a plain range.

use crate::ast::{CellRef, Expr, RefExpr, RefKind, TableCols, TableRef, TableRows, same_name};
use crate::parser::ParseError;
use crate::refs::{Rect, Reference};
use crate::table::Table;
use crate::value::{ErrorKind, R};
use crate::workbook::Workbook;
use std::sync::Arc;

/// The cells of `table` a structured reference covers. `row` is the row of
/// the cell holding the formula, which `#This Row` means.
///
/// `#REF!` for a column the table does not have and for a header or totals
/// row that is switched off; `#VALUE!` for `#This Row` used on a row that
/// is not one of the table's data rows.
pub fn rect_in(table: &Table, reference: &TableRef, row: u32) -> R<Rect> {
    let range = table.range;
    let column = |name: &str| -> R<u32> {
        let index = table.column_index(name).ok_or(ErrorKind::Ref)? as u32;
        if index >= range.cols() {
            return Err(ErrorKind::Ref);
        }
        Ok(range.c0 + index)
    };
    let (c0, c1) = match &reference.cols {
        TableCols::All => (range.c0, range.c1),
        TableCols::One(c) => {
            let c = column(c)?;
            (c, c)
        }
        TableCols::Range(a, b) => (column(a)?, column(b)?),
    };
    let data = table.data_range();
    let (r0, r1) = match reference.rows {
        TableRows::Unspecified | TableRows::Data => {
            let d = data.ok_or(ErrorKind::Ref)?;
            (d.r0, d.r1)
        }
        TableRows::All => (range.r0, range.r1),
        TableRows::Headers => {
            let h = table.header_range().ok_or(ErrorKind::Ref)?;
            (h.r0, h.r1)
        }
        TableRows::Totals => {
            let t = table.totals_range().ok_or(ErrorKind::Ref)?;
            (t.r0, t.r1)
        }
        TableRows::HeadersData => {
            let last = range
                .r1
                .checked_sub(table.totals_row as u32)
                .ok_or(ErrorKind::Ref)?;
            (range.r0, last.max(range.r0))
        }
        TableRows::DataTotals => ((range.r0 + table.header_row as u32).min(range.r1), range.r1),
        TableRows::ThisRow => match data {
            Some(d) if row >= d.r0 && row <= d.r1 => (row, row),
            _ => return Err(ErrorKind::Value),
        },
    };
    Ok(Rect::new(r0, c0, r1, c1))
}

/// The cells a structured reference written in the cell (`sheet`, `row`,
/// `col`) stands for. `#REF!` when the table does not exist, or when a
/// reference without a table name is used outside a table.
pub fn resolve(
    wb: &Workbook,
    reference: &TableRef,
    sheet: usize,
    row: u32,
    col: u32,
) -> R<Reference> {
    let (table_sheet, table) = match &reference.table {
        Some(name) => wb.table_by_name(name).ok_or(ErrorKind::Ref)?,
        None => (sheet, wb.table_at(sheet, row, col).ok_or(ErrorKind::Ref)?),
    };
    // "This row" is the formula's row: it has no meaning from another sheet.
    if reference.rows == TableRows::ThisRow && table_sheet != sheet {
        return Err(ErrorKind::Value);
    }
    Ok(Reference::new(table_sheet, rect_in(table, reference, row)?))
}

/// The plain reference Excel leaves in place of a structured reference
/// when the table becomes an ordinary range: absolute, with the sheet
/// name; a `#This Row` reference keeps a relative row (`Sheet1!$B2`).
fn as_plain_ref(sheet_name: &str, rect: Rect, this_row: bool) -> Expr {
    let corner = |row: u32, col: u32| CellRef {
        row,
        col,
        abs_row: !this_row,
        abs_col: true,
    };
    Expr::Ref(RefExpr {
        sheet: Some(Arc::from(sheet_name)),
        start: corner(rect.r0, rect.c0),
        end: (!rect.is_single()).then(|| corner(rect.r1, rect.c1)),
        kind: RefKind::Cells,
    })
}

impl Workbook {
    /// The formula as it is stored when typed into the cell (`sheet`,
    /// `row`, `col`): structured references written without a table name
    /// (`[@Col]`) get the name of the table holding the cell, or about to
    /// grow over it (`table_for_entry`). They are an error anywhere else.
    pub fn qualify_table_refs(
        &self,
        e: &Expr,
        sheet: usize,
        row: u32,
        col: u32,
    ) -> Result<Expr, ParseError> {
        let mut unqualified = false;
        e.walk(&mut |x| unqualified |= matches!(x, Expr::Table(t) if t.table.is_none()));
        if !unqualified {
            return Ok(e.clone());
        }
        match self.table_for_entry(sheet, row, col) {
            Some(table) => Ok(e.with_table_qualified(&table.name)),
            None => Err(ParseError {
                pos: 0,
                message: "a column reference without a table name is only valid inside a table"
                    .to_string(),
            }),
        }
    }

    /// True when the cell lies inside the table named `table`.
    fn in_table(&self, table: &str, sheet: usize, row: u32, col: u32) -> bool {
        self.table_at(sheet, row, col)
            .is_some_and(|t| same_name(&t.name, table))
    }

    /// Make every formula follow a table renamed from `old` to `new`.
    /// Returns whether a formula changed. Like the other rewrites below
    /// it takes the undo snapshot itself; the caller commits.
    pub fn rename_table_refs(&mut self, old: &str, new: &str) -> bool {
        self.rewrite_formulas(&|_, _, e| Some(e.with_table_renamed(old, new)))
    }

    /// Make every formula follow a column of the table named `table`
    /// renamed from `old` to `new`. The table must (still or already) be
    /// called `table` and cover its cells.
    pub fn rename_table_column_refs(&mut self, table: &str, old: &str, new: &str) -> bool {
        self.rewrite_formulas(&|wb, site, e| {
            let inside = site
                .cell()
                .is_some_and(|(s, r, c)| wb.in_table(table, s, r, c));
            Some(e.with_table_column_renamed(table, inside, old, new))
        })
    }

    /// References to a column of `table` that is being deleted become
    /// `#REF!`.
    pub fn delete_table_column_refs(&mut self, table: &str, column: &str) -> bool {
        self.rewrite_formulas(&|wb, site, e| {
            let inside = site
                .cell()
                .is_some_and(|(s, r, c)| wb.in_table(table, s, r, c));
            Some(e.with_table_column_deleted(table, inside, column))
        })
    }

    /// Replace the structured references to the table named `table` with
    /// the plain A1 references they stand for (absolute, with the sheet
    /// name), as "Convert to Range" does. Call it while the table still
    /// exists, also before deleting a table.
    pub fn table_refs_to_ranges(&mut self, table: &str) -> bool {
        self.rewrite_formulas(&|wb, site, e| {
            let (table_sheet, t) = wb.table_by_name(table)?;
            let sheet_name = wb.sheets[table_sheet].name.as_str();
            let cell = site.cell();
            let inside = cell.is_some_and(|(s, r, c)| wb.in_table(table, s, r, c));
            Some(e.map(&mut |x| match x {
                Expr::Table(r) if r.is_to(table, inside) => {
                    let this_row = r.rows == TableRows::ThisRow;
                    // "This row" needs a formula on a row of the table.
                    let row = match cell {
                        Some((s, row, _)) if s == table_sheet => Some(row),
                        _ => None,
                    };
                    let rect = match (this_row, row) {
                        (true, None) => Err(ErrorKind::Value),
                        (_, row) => rect_in(t, &r, row.unwrap_or(0)),
                    };
                    match rect {
                        Ok(rect) => as_plain_ref(sheet_name, rect, this_row),
                        Err(kind) => Expr::Error(kind),
                    }
                }
                // The bare table name stands for its data.
                Expr::Name(n) if same_name(&n, table) && wb.defined_name(&n).is_none() => {
                    match t.data_range() {
                        Some(rect) => as_plain_ref(sheet_name, rect, false),
                        None => Expr::Error(ErrorKind::Ref),
                    }
                }
                other => other,
            }))
        })
    }
}
