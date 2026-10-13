//! `Table`, `TableColumn`, the filter of a column (`Filter`) and
//! `TableSort`.

use serde_json::Value as J;

use opensheet_engine::features::Edit;
use opensheet_engine::filter::{Criteria, FilterScope, SortState};
use opensheet_engine::ops::SortSpec;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;
use opensheet_engine::table::{Table, TableOption, table_style};

use super::range::{Entry, entered, sort_levels, write_cell};
use super::util::{range_arg, table_fail};
use super::worksheet::{
    auto_filter_object, color_filter, criteria_from, custom_filter, dynamic_filter, set_criteria, top_filter, values_filter,
};
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

pub(super) fn table_object(id: u32) -> J {
    object("Table", vec![id.into()])
}

/// `getTable`: by name or by ID, in the workbook or on one sheet.
pub(super) fn find(h: &Host, call: &Call, sheet: Option<usize>) -> Res<Ret> {
    let key = call.str(0)?;
    let found = h
        .wb
        .table_by_name(key)
        .or_else(|| key.parse().ok().and_then(|id| h.wb.table_by_id(id)))
        .filter(|(on, _)| sheet.is_none_or(|s| s == *on));
    Ok(Ret::maybe(found.map(|(_, table)| table_object(table.id))))
}

/// `addTable`: the second argument says whether the range has headers.
pub(super) fn add(h: &mut Host, call: &Call, sheet: usize, rect: Rect) -> Res<Ret> {
    let headers = call.bool(1)?;
    h.permit(call, &Action::ChangeTable { sheet })?;
    h.calculate();
    let id = h.wb.add_table(sheet, rect, headers).map_err(|e| table_fail(call, e))?;
    h.restructured();
    Ok(Ret::Object(table_object(id)))
}

/// The sheet and the table a Table object (or one of its parts) stands for.
fn this_table(h: &Host, call: &Call) -> Res<(usize, Table)> {
    let id = u32::try_from(call.this_int(0)).map_err(|_| call.gone())?;
    let (sheet, table) = h.wb.table_by_id(id).ok_or_else(|| call.gone())?;
    Ok((sheet, table.clone()))
}

/// The same, with the column a TableColumn or a Filter object stands for.
fn this_column(h: &Host, call: &Call) -> Res<(usize, Table, usize)> {
    let (sheet, table) = this_table(h, call)?;
    let index = usize::try_from(call.this_int(1)).ok().filter(|i| *i < table.columns.len()).ok_or_else(|| call.gone())?;
    Ok((sheet, table, index))
}

fn column_object(table: &Table, index: usize) -> J {
    object("TableColumn", vec![table.id.into(), index.into()])
}

/// A Table Design check box.
fn option(h: &mut Host, call: &Call, option: TableOption) -> Res<Ret> {
    Ok(this_table(h, call)?.1.option(option).into())
}

fn set_option(h: &mut Host, call: &Call, option: TableOption) -> Res<Ret> {
    let (sheet, table) = this_table(h, call)?;
    let on = call.bool(0)?;
    h.permit(call, &Action::ChangeTable { sheet })?;
    h.wb.set_table_option(sheet, table.id, option, on).map_err(|e| table_fail(call, e))?;
    h.restructured();
    Ok(Ret::Nothing)
}

/// Write values in a row or a column of cells of a table.
fn fill(h: &mut Host, call: &Call, sheet: usize, cells: Rect, values: &[J]) -> Res<()> {
    let count = cells.rows() as usize * cells.cols() as usize;
    if values.len() != count {
        return Err(call.fail("The number of rows or columns in the input array doesn't match the size or dimensions of the range."));
    }
    h.spend(call, count as u64)?;
    let places = (cells.r0..=cells.r1).flat_map(|row| (cells.c0..=cells.c1).map(move |col| (row, col)));
    for ((row, col), value) in places.zip(values) {
        write_cell(h, call, sheet, row, col, value, Entry::English)?;
    }
    entered(h, sheet, cells);
    Ok(())
}

/// `addRow` and `addRows`: rows of values at an index among the data
/// rows, at the end without one.
fn add_rows(h: &mut Host, call: &Call, rows: Vec<&[J]>) -> Res<Ret> {
    let (sheet, table) = this_table(h, call)?;
    h.permit(call, &Action::ChangeTable { sheet })?;
    let width = table.range.cols() as usize;
    if rows.is_empty() || rows.iter().any(|row| row.len() != width) {
        return Err(call.fail("The number of rows or columns in the input array doesn't match the size or dimensions of the range."));
    }
    let first = table.range.r0 + table.header_row as u32;
    let end = table.range.r1 + 1 - table.totals_row as u32;
    let at = match call.opt_int(0)? {
        None | Some(-1) => end,
        Some(index) => u32::try_from(index).ok().map(|i| first + i).filter(|row| *row <= end).ok_or_else(|| call.invalid())?,
    };
    // A table made of headers alone has one blank row waiting: the first
    // row added goes there, as in Excel.
    let blank = end - first == 1 && (table.range.c0..=table.range.c1).all(|col| h.wb.sheets[sheet].is_blank(first, col));
    let count = rows.len() as u32 - blank as u32;
    let at = if blank { first } else { at };
    if count > 0 {
        h.wb.table_insert_rows(sheet, table.id, if blank { first + 1 } else { at }, count).map_err(|e| table_fail(call, e))?;
        h.restructured();
    }
    for (offset, row) in rows.iter().enumerate() {
        let cells = Rect::new(at + offset as u32, table.range.c0, at + offset as u32, table.range.c1);
        fill(h, call, sheet, cells, row)?;
    }
    Ok(Ret::Nothing)
}

/// Filter a column of a table, putting the filter buttons back when the
/// table has none.
fn filter_column(h: &mut Host, call: &Call, criteria: Option<Criteria>) -> Res<Ret> {
    let (sheet, table, index) = this_column(h, call)?;
    if table.auto_filter.is_none() {
        if criteria.is_none() {
            return Ok(Ret::Nothing);
        }
        h.permit(call, &Action::ChangeTable { sheet })?;
        h.wb.set_table_option(sheet, table.id, TableOption::FilterButton, true).map_err(|e| table_fail(call, e))?;
    }
    set_criteria(h, call, sheet, FilterScope::Table(table.id), index as i64, criteria)?;
    Ok(Ret::Nothing)
}

pub(super) const TABLE: ApiClass = class!("Table", "A table: a range with a header row, columns that have names, its own filter and style.", {
    getName "(): string" "The name of the table." => |h, c| Ok(this_table(h, c)?.1.name.into());
    setName "(name: string): void" "Renames the table; formulas that name it follow." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.rename_table(sheet, table.id, c.str(0)?).map_err(|e| table_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
    getId "(): string" "An identifier that stays the same when the table is renamed." => |h, c| Ok(this_table(h, c)?.1.id.to_string().into());
    getWorksheet "(): Worksheet" "The worksheet the table is on." => |h, c| {
        let (sheet, _) = this_table(h, c)?;
        Ok(Ret::Object(h.worksheet(sheet)))
    };
    getRange "(): Range" "The whole table: header row, data and total row." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        Ok(Ret::Object(h.range(sheet, table.range)))
    };
    getHeaderRowRange "(): Range" "The header row. Fails when the table does not show it." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let header = table.header_range().ok_or_else(|| c.fail("The table has no header row."))?;
        Ok(Ret::Object(h.range(sheet, header)))
    };
    getRangeBetweenHeaderAndTotal "(): Range" "The data rows." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let data = table.data_range().ok_or_else(|| c.gone())?;
        Ok(Ret::Object(h.range(sheet, data)))
    };
    getTotalRowRange "(): Range" "The total row. Fails when the table does not show it." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let totals = table.totals_range().ok_or_else(|| c.fail("The table has no total row."))?;
        Ok(Ret::Object(h.range(sheet, totals)))
    };
    getRowCount "(): number" "How many data rows the table has." => |h, c| {
        Ok(this_table(h, c)?.1.data_range().map_or(0, |data| data.rows()).into())
    };
    getColumns "(): TableColumn[]" "The columns, left to right." => |h, c| {
        let (_, table) = this_table(h, c)?;
        Ok(Ret::Object(J::from((0..table.columns.len()).map(|i| column_object(&table, i)).collect::<Vec<J>>())))
    };
    getColumn "(key: number | string): TableColumn | undefined" "The column with this name or ID; undefined when there is none." => |h, c| {
        let (_, table) = this_table(h, c)?;
        let index = match c.arg(0) {
            J::String(name) => table.column_index(name),
            J::Number(id) => id.as_u64().and_then(|id| (id as usize).checked_sub(1)).filter(|i| *i < table.columns.len()),
            _ => return Err(c.invalid()),
        };
        Ok(Ret::maybe(index.map(|i| column_object(&table, i))))
    };
    getColumnByName "(name: string): TableColumn | undefined" "The column with this name; undefined when there is none." => |h, c| {
        let (_, table) = this_table(h, c)?;
        Ok(Ret::maybe(table.column_index(c.str(0)?).map(|i| column_object(&table, i))))
    };
    getColumnById "(id: number): TableColumn | undefined" "The column with this ID; undefined when there is none." => |h, c| {
        let (_, table) = this_table(h, c)?;
        let index = usize::try_from(c.int(0)? - 1).ok().filter(|i| *i < table.columns.len());
        Ok(Ret::maybe(index.map(|i| column_object(&table, i))))
    };
    addColumn "(index?: number, values?: (boolean | string | number)[], name?: string): TableColumn" "Adds a column at an index (at the end without one), with its cells from the header down, and its name." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        let index = match c.opt_int(0)? {
            None | Some(-1) => table.columns.len(),
            Some(index) => usize::try_from(index).ok().filter(|i| *i <= table.columns.len()).ok_or_else(|| c.invalid())?,
        };
        let at = table.range.c0 + index as u32;
        h.wb.table_insert_cols(sheet, table.id, at, 1).map_err(|e| table_fail(c, e))?;
        h.restructured();
        if c.has(1) {
            let cells = Rect::new(table.range.r0, at, table.range.r1, at);
            fill(h, c, sheet, cells, c.array(1)?)?;
        }
        if let Some(name) = c.opt_str(2)? {
            if !table.header_row {
                return Err(c.fail("The table has no header row."));
            }
            fill(h, c, sheet, Rect::cell(table.range.r0, at), &[J::from(name)])?;
        }
        Ok(Ret::Object(column_object(&table, index)))
    };
    addRow "(index?: number, values?: (boolean | string | number)[]): void" "Adds a row of values at an index among the data rows (at the end without one)." => |h, c| {
        add_rows(h, c, vec![c.array(1)?])
    };
    addRows "(index?: number, values?: (boolean | string | number)[][]): void" "Adds several rows of values at an index among the data rows (at the end without one)." => |h, c| {
        let rows = c.array(1)?.iter().map(|row| row.as_array().map(Vec::as_slice).ok_or_else(|| c.invalid())).collect::<Res<Vec<&[J]>>>()?;
        add_rows(h, c, rows)
    };
    deleteRowsAt "(index: number, count?: number): void" "Deletes data rows from an index (one without a count)." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        let data = table.data_range().ok_or_else(|| c.gone())?;
        let index = u32::try_from(c.int(0)?).map_err(|_| c.invalid())?;
        let count = u32::try_from(c.opt_int(1)?.unwrap_or(1)).map_err(|_| c.invalid())?;
        h.wb.table_delete_rows(sheet, table.id, data.r0.saturating_add(index), count).map_err(|e| table_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
    resize "(newRange: Range | string): void" "Changes the range of the table; its header row stays where it is." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let (on, rect) = range_arg(h, c, 0, sheet)?;
        if on != sheet {
            return Err(c.invalid());
        }
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.resize_table(sheet, table.id, rect).map_err(|e| table_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
    getShowHeaders "(): boolean" "Whether the header row shows." => |h, c| option(h, c, TableOption::HeaderRow);
    setShowHeaders "(showHeaders: boolean): void" "Shows or hides the header row." => |h, c| set_option(h, c, TableOption::HeaderRow);
    getShowTotals "(): boolean" "Whether the total row shows." => |h, c| option(h, c, TableOption::TotalsRow);
    setShowTotals "(showTotals: boolean): void" "Shows or hides the total row." => |h, c| set_option(h, c, TableOption::TotalsRow);
    getShowBandedRows "(): boolean" "Whether rows are striped." => |h, c| option(h, c, TableOption::BandedRows);
    setShowBandedRows "(showBandedRows: boolean): void" "Stripes the rows or not." => |h, c| set_option(h, c, TableOption::BandedRows);
    getShowBandedColumns "(): boolean" "Whether columns are striped." => |h, c| option(h, c, TableOption::BandedCols);
    setShowBandedColumns "(showBandedColumns: boolean): void" "Stripes the columns or not." => |h, c| set_option(h, c, TableOption::BandedCols);
    getHighlightFirstColumn "(): boolean" "Whether the first column stands out." => |h, c| option(h, c, TableOption::FirstCol);
    setHighlightFirstColumn "(highlightFirstColumn: boolean): void" "Makes the first column stand out or not." => |h, c| set_option(h, c, TableOption::FirstCol);
    getHighlightLastColumn "(): boolean" "Whether the last column stands out." => |h, c| option(h, c, TableOption::LastCol);
    setHighlightLastColumn "(highlightLastColumn: boolean): void" "Makes the last column stand out or not." => |h, c| set_option(h, c, TableOption::LastCol);
    getShowFilterButton "(): boolean" "Whether the header cells have filter buttons." => |h, c| option(h, c, TableOption::FilterButton);
    setShowFilterButton "(showFilterButton: boolean): void" "Shows or hides the filter buttons; hiding them shows every row." => |h, c| set_option(h, c, TableOption::FilterButton);
    getPredefinedTableStyle "(): string" "The name of the table style (TableStyleMedium2); empty text for none." => |h, c| {
        Ok(this_table(h, c)?.1.style.unwrap_or_default().into())
    };
    setPredefinedTableStyle "(predefinedTableStyle: string): void" "Gives the table one of the built-in styles: TableStyleLight1 to 21, TableStyleMedium1 to 28, TableStyleDark1 to 11. Empty text removes the style." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let name = c.str(0)?.trim();
        let style = match name {
            "" => None,
            name => Some(table_style(name).ok_or_else(|| c.fail(format!("There is no table style named {name}.")))?.name.as_str()),
        };
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.set_table_style(sheet, table.id, style);
        Ok(Ret::Nothing)
    };
    getSort "(): TableSort" "The sort of the table." => |h, c| {
        this_table(h, c)?;
        Ok(Ret::object("TableSort", c.this.to_vec()))
    };
    getAutoFilter "(): AutoFilter" "The filter of the table." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        Ok(Ret::Object(auto_filter_object(h, sheet, Some(table.id))))
    };
    clearFilters "(): void" "Shows every row again, keeping the filter buttons." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::UseAutoFilter { sheet })?;
        h.wb.clear_filters(sheet, FilterScope::Table(table.id));
        h.restructured();
        Ok(Ret::Nothing)
    };
    reapplyFilters "(): void" "Filters again with the values the cells have now." => |h, c| {
        let (sheet, _) = this_table(h, c)?;
        h.permit(c, &Action::UseAutoFilter { sheet })?;
        h.calculate();
        h.wb.reapply_filters(sheet);
        h.restructured();
        Ok(Ret::Nothing)
    };
    convertToRange "(): Range" "Makes the table plain cells again: values and the look of the style stay." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.convert_table_to_range(sheet, table.id);
        h.restructured();
        Ok(Ret::Object(h.range(sheet, table.range)))
    };
    delete "(): void" "Deletes the table with what its cells hold." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.delete_table(sheet, table.id);
        h.restructured();
        Ok(Ret::Nothing)
    };
});

pub(super) const TABLE_COLUMN: ApiClass = class!("TableColumn", "A column of a table.", {
    getName "(): string" "The name of the column: the text of its header." => |h, c| {
        let (_, table, index) = this_column(h, c)?;
        Ok(table.columns[index].name.clone().into())
    };
    setName "(name: string): void" "Renames the column; formulas that name it follow." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        let name = c.str(0)?;
        if !table.header_row {
            return Err(c.fail("The table has no header row."));
        }
        h.permit(c, &Action::ChangeTable { sheet })?;
        fill(h, c, sheet, Rect::cell(table.range.r0, table.range.c0 + index as u32), &[J::from(name)])?;
        Ok(Ret::Nothing)
    };
    getId "(): number" "The number of the column in the table, from 1." => |h, c| Ok((this_column(h, c)?.2 as u64 + 1).into());
    getIndex "(): number" "The place of the column in the table, from 0." => |h, c| Ok((this_column(h, c)?.2 as u64).into());
    getRange "(): Range" "The whole column: header, data and total." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        let col = table.range.c0 + index as u32;
        Ok(Ret::Object(h.range(sheet, Rect::new(table.range.r0, col, table.range.r1, col))))
    };
    getHeaderRowRange "(): Range" "The header cell of the column. Fails when the table does not show its header row." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        let header = table.header_range().ok_or_else(|| c.fail("The table has no header row."))?;
        Ok(Ret::Object(h.range(sheet, Rect::cell(header.r0, table.range.c0 + index as u32))))
    };
    getRangeBetweenHeaderAndTotal "(): Range" "The data cells of the column." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        let data = table.column_range(index).ok_or_else(|| c.gone())?;
        Ok(Ret::Object(h.range(sheet, data)))
    };
    getTotalRowRange "(): Range" "The total cell of the column. Fails when the table does not show its total row." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        let totals = table.totals_range().ok_or_else(|| c.fail("The table has no total row."))?;
        Ok(Ret::Object(h.range(sheet, Rect::cell(totals.r0, table.range.c0 + index as u32))))
    };
    getFilter "(): Filter" "The filter of the column." => |h, c| {
        this_column(h, c)?;
        Ok(Ret::object("Filter", c.this.to_vec()))
    };
    delete "(): void" "Deletes the column from the table." => |h, c| {
        let (sheet, table, index) = this_column(h, c)?;
        h.permit(c, &Action::ChangeTable { sheet })?;
        h.wb.table_delete_cols(sheet, table.id, table.range.c0 + index as u32, 1).map_err(|e| table_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
});

pub(super) const FILTER: ApiClass = class!("Filter", "The filter of one column of a table.", {
    apply "(criteria: ExcelScript.FilterCriteria): void" "Filters the column on criteria: {filterOn, values, criterion1, criterion2, operator, color, dynamicCriteria}." => |h, c| {
        let criteria = criteria_from(c, c.arg(0))?;
        filter_column(h, c, Some(criteria))
    };
    applyValuesFilter "(values: Array<string | ExcelScript.FilterDatetime>): void" "Shows the rows whose cell shows one of these texts (the empty text for blank cells)." => |h, c| {
        let criteria = values_filter(c, c.array(0)?)?;
        filter_column(h, c, Some(criteria))
    };
    applyCustomFilter "(criteria1: string, criteria2?: string, oper?: ExcelScript.FilterOperator): void" "Shows the rows that pass one or two comparisons (\">=10\", \"<>done\", \"=A*\"), joined by And or Or." => |h, c| {
        let criteria = custom_filter(c.str(0)?, c.opt_str(1)?, enum_value(c, 2, "FilterOperator")?);
        filter_column(h, c, Some(criteria))
    };
    applyTopItemsFilter "(count: number): void" "Shows the rows with the largest numbers." => |h, c| {
        let criteria = top_filter(c, c.num(0)?, true, false)?;
        filter_column(h, c, Some(criteria))
    };
    applyBottomItemsFilter "(count: number): void" "Shows the rows with the smallest numbers." => |h, c| {
        let criteria = top_filter(c, c.num(0)?, false, false)?;
        filter_column(h, c, Some(criteria))
    };
    applyTopPercentFilter "(percent: number): void" "Shows the rows whose numbers are in the top percentage." => |h, c| {
        let criteria = top_filter(c, c.num(0)?, true, true)?;
        filter_column(h, c, Some(criteria))
    };
    applyBottomPercentFilter "(percent: number): void" "Shows the rows whose numbers are in the bottom percentage." => |h, c| {
        let criteria = top_filter(c, c.num(0)?, false, true)?;
        filter_column(h, c, Some(criteria))
    };
    applyDynamicFilter "(criteria: ExcelScript.DynamicFilterCriteria): void" "Filters on the average of the column or on a period of dates (AboveAverage, ThisMonth, LastYear…)." => |h, c| {
        let criteria = dynamic_filter(c, 0)?;
        filter_column(h, c, Some(criteria))
    };
    applyCellColorFilter "(color: string): void" "Shows the rows whose cell is filled with a colour." => |h, c| {
        let criteria = color_filter(c, 0, false)?;
        filter_column(h, c, Some(criteria))
    };
    applyFontColorFilter "(color: string): void" "Shows the rows whose cell is written in a colour." => |h, c| {
        let criteria = color_filter(c, 0, true)?;
        filter_column(h, c, Some(criteria))
    };
    clear "(): void" "Removes the filter of the column." => |h, c| filter_column(h, c, None);
});

pub(super) const TABLE_SORT: ApiClass = class!("TableSort", "The sort of a table.", {
    apply "(fields: ExcelScript.SortField[], matchCase?: boolean, method?: ExcelScript.SortMethod): void" "Sorts the rows of the table on one or several columns ({key, ascending, sortOn, color}; key is 0-based in the table)." => |h, c| {
        let (sheet, table) = this_table(h, c)?;
        let levels = sort_levels(c, c.array(0)?)?;
        if levels.is_empty() || levels.iter().any(|level| level.key as usize >= table.columns.len()) {
            return Err(c.invalid());
        }
        if enum_value(c, 2, "SortMethod")?.is_some_and(|method| method != "PinYin") {
            return Err(c.unsupported("ExcelScript.SortMethod.strokeCount"));
        }
        let case_sensitive = c.opt_bool(1)?.unwrap_or(false);
        let rows = Rect::new(table.range.r0, table.range.c0, table.range.r1 - table.totals_row as u32, table.range.c1);
        h.permit_edit(c, &Edit::Sort { sheet, range: rows })?;
        h.calculate();
        let sorted = if table.auto_filter.is_some() {
            // With its buttons, the table remembers the sort and shows it.
            h.wb.sort_filter(sheet, FilterScope::Table(table.id), SortState { levels, case_sensitive })
        } else {
            let spec = SortSpec { levels, has_header: table.header_row, case_sensitive, left_to_right: false };
            h.wb.sort_range(sheet, rows, &spec)
        };
        sorted.map_err(|_| c.fail("To do this, all the merged cells need to be the same size."))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
});
