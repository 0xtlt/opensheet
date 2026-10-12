//! Tables and the formula language together: totals rows, calculated
//! columns and the rewrites of structured references when a table or one
//! of its columns is renamed, deleted or turned back into a range.

mod common;

use common::{at, enter, err, num, shown, value};
use opensheet_engine::ErrorKind::Ref;
use opensheet_engine::names::NameScope;
use opensheet_engine::ops::PasteMode;
use opensheet_engine::table::{TableEdit, TableOption, TotalsFunction};
use opensheet_engine::workbook::Content;
use opensheet_engine::{Locale, Rect, Value, Workbook};

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

/// Type in a cell as the app does: the entry, then what tables make of it.
fn type_in(wb: &mut Workbook, sheet: usize, name: &str, text: &str) -> TableEdit {
    enter(wb, sheet, name, text);
    let (row, col) = at(name);
    let edit = wb.tables_after_edit(sheet, Rect::cell(row, col));
    wb.commit("entry");
    edit
}

/// The formula of a cell as it is stored (and written to files).
fn stored(wb: &Workbook, sheet: usize, name: &str) -> String {
    let (row, col) = at(name);
    match wb.cell(sheet, row, col).map(|c| &c.content) {
        Some(Content::Formula(e)) => e.to_text(Locale::en()),
        _ => String::new(),
    }
}

/// Sheet1!A1:C4 is the table "Table1": Item, Qty and Price, three rows.
fn orders(locale: &'static Locale) -> (Workbook, u32) {
    let mut wb = Workbook::new(locale);
    let rows = [["Item", "Qty", "Price"], ["nut", "1", "10"], ["bolt", "2", "20"], ["gear", "3", "30"]];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    let id = wb.add_table(0, rect("A1:C4"), true).unwrap();
    wb.commit("table");
    (wb, id)
}

#[test]
fn totals_rows_hold_structured_formulas() {
    let (mut wb, id) = orders(Locale::en());
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    wb.set_totals_function(0, id, 1, TotalsFunction::Average);
    wb.commit("totals");
    // Excel's form: the column by name, shown without the table's name
    // inside the table and stored (as in files) with it.
    assert_eq!(shown(&wb, 0, "C5"), "=SUBTOTAL(109,[Price])");
    assert_eq!(stored(&wb, 0, "C5"), "SUBTOTAL(109,Table1[Price])");
    assert_eq!(shown(&wb, 0, "B5"), "=SUBTOTAL(101,[Qty])");
    assert_eq!([value(&wb, 0, "B5"), value(&wb, 0, "C5")], [num(2.0), num(60.0)]);
    // A row added to the table is counted without the formula changing.
    wb.table_insert_rows(0, id, 4, 1).unwrap();
    enter(&mut wb, 0, "C5", "40");
    wb.commit("row");
    assert_eq!(shown(&wb, 0, "C6"), "=SUBTOTAL(109,[Price])");
    assert_eq!(value(&wb, 0, "C6"), num(100.0));
    // Typing the same formula in the totals row is choosing the function.
    assert_eq!(type_in(&mut wb, 0, "B6", "=SUBTOTAL(104,[Qty])"), TableEdit::default());
    assert_eq!(wb.table(0, id).unwrap().columns[1].totals, TotalsFunction::Max);
    assert_eq!(value(&wb, 0, "B6"), num(3.0));
    // A formula of one's own is stored with the table's name too.
    wb.set_totals_formula(0, id, 0, "=COUNTA([Item])").unwrap();
    wb.commit("custom");
    assert_eq!(shown(&wb, 0, "A6"), "=COUNTA([Item])");
    assert_eq!(stored(&wb, 0, "A6"), "COUNTA(Table1[Item])");
    assert_eq!(value(&wb, 0, "A6"), num(3.0));

    // In French, with the names of that language.
    let (mut fr, id) = orders(Locale::fr());
    fr.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    fr.commit("totals");
    assert_eq!(shown(&fr, 0, "C5"), "=SOUS.TOTAL(109;[Price])");
    assert_eq!(stored(&fr, 0, "C5"), "SUBTOTAL(109,Tableau1[Price])");
    assert_eq!(value(&fr, 0, "C5"), num(60.0));
}

#[test]
fn renaming_a_table_rewrites_what_names_it() {
    let (mut wb, id) = orders(Locale::en());
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    wb.add_sheet(1);
    enter(&mut wb, 0, "E1", "=SUM(Table1[Qty])");
    enter(&mut wb, 1, "A1", "=ROWS(Table1)+Table1[[#Totals],[Price]]");
    let refers_to = wb.parse_name_formula("=Table1[Price]", 0, 0, 0).unwrap();
    wb.add_name(NameScope::Workbook, "Prices", refers_to, "").unwrap();
    enter(&mut wb, 0, "E2", "=SUM(Prices)");
    // A totals formula the table only remembers while its row is hidden.
    wb.set_totals_formula(0, id, 0, "=COUNTA(Table1[Item])").unwrap();
    wb.set_table_option(0, id, TableOption::TotalsRow, false).unwrap();
    wb.commit("setup");

    wb.rename_table(0, id, "Orders").unwrap();
    wb.commit("rename");
    assert_eq!(shown(&wb, 0, "E1"), "=SUM(Orders[Qty])");
    assert_eq!(shown(&wb, 1, "A1"), "=ROWS(Orders)+Orders[[#Totals],[Price]]");
    let name = wb.name_in(NameScope::Workbook, "Prices").unwrap();
    assert_eq!(name.refers_to(Locale::en(), 0, 0), "=Orders[Price]");
    assert_eq!([value(&wb, 0, "E1"), value(&wb, 0, "E2")], [num(6.0), num(60.0)]);
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    wb.commit("totals");
    assert_eq!(stored(&wb, 0, "C5"), "SUBTOTAL(109,Orders[Price])");
    assert_eq!(stored(&wb, 0, "A5"), "COUNTA(Orders[Item])");
    assert_eq!([value(&wb, 0, "A5"), value(&wb, 0, "C5"), value(&wb, 1, "A1")], [num(3.0), num(60.0), num(63.0)]);
    // One step back for each step.
    assert!(wb.undo() && wb.undo());
    assert_eq!(shown(&wb, 0, "E1"), "=SUM(Table1[Qty])");
    assert_eq!(value(&wb, 0, "E1"), num(6.0));
}

#[test]
fn renaming_columns_rewrites_what_names_them() {
    let (mut wb, id) = orders(Locale::en());
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    enter(&mut wb, 0, "F1", "=SUM(Table1[Price])");
    enter(&mut wb, 0, "F2", "=SUM(Table1[[Qty]:[Price]])");
    type_in(&mut wb, 0, "D2", "=[@Qty]*[@Price]");
    // Typing in a header renames the column wherever it is used.
    assert!(type_in(&mut wb, 0, "C1", "Unit Price").renamed);
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Table1[Unit Price])");
    assert_eq!(shown(&wb, 0, "F2"), "=SUM(Table1[[Qty]:[Unit Price]])");
    assert_eq!(shown(&wb, 0, "D3"), "=[@Qty]*[@[Unit Price]]");
    assert_eq!(shown(&wb, 0, "C5"), "=SUBTOTAL(109,[Unit Price])");
    assert_eq!([value(&wb, 0, "F1"), value(&wb, 0, "D3"), value(&wb, 0, "C5")], [num(60.0), num(40.0), num(60.0)]);
    // Two headers swapped in one go: each reference follows its column.
    wb.paste_text(0, 0, 1, "Unit Price\tQty");
    assert!(wb.tables_after_edit(0, rect("B1:C1")).renamed);
    wb.commit("swap");
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Table1[Qty])");
    assert_eq!(shown(&wb, 0, "D3"), "=[@[Unit Price]]*[@Qty]");
    assert_eq!([value(&wb, 0, "F1"), value(&wb, 0, "D3")], [num(60.0), num(40.0)]);
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Table1[Unit Price])");
}

#[test]
fn deleted_columns_and_tables_leave_ref_errors() {
    let (mut wb, id) = orders(Locale::en());
    enter(&mut wb, 0, "F1", "=SUM(Table1[Qty])");
    enter(&mut wb, 0, "F2", "=SUM(Table1[Price])");
    enter(&mut wb, 0, "F3", "=ROWS(Table1)");
    wb.commit("setup");
    // A table column deleted from the table...
    wb.table_delete_cols(0, id, 1, 1).unwrap();
    wb.commit("delete");
    assert_eq!(shown(&wb, 0, "E1"), "=SUM(#REF!)");
    assert_eq!(shown(&wb, 0, "E2"), "=SUM(Table1[Price])");
    assert_eq!([value(&wb, 0, "E1"), value(&wb, 0, "E2")], [err(Ref), num(60.0)]);
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Table1[Qty])");
    // ...or with its sheet column.
    wb.delete_cols(0, 2, 1);
    wb.commit("delete");
    assert_eq!([shown(&wb, 0, "E1"), shown(&wb, 0, "E2")], ["=SUM(Table1[Qty])", "=SUM(#REF!)"]);
    assert!(wb.undo());
    // A column left outside when the table is resized is gone too.
    wb.resize_table(0, id, rect("A1:B4")).unwrap();
    wb.commit("resize");
    assert_eq!([shown(&wb, 0, "F1"), shown(&wb, 0, "F2")], ["=SUM(Table1[Qty])", "=SUM(#REF!)"]);
    assert!(wb.undo());

    // The whole table: deleted, or gone with the rows of its sheet.
    assert!(wb.delete_table(0, id));
    wb.commit("delete");
    assert_eq!([shown(&wb, 0, "F1"), shown(&wb, 0, "F3")], ["=SUM(#REF!)", "=ROWS(#REF!)"]);
    assert!(wb.undo());
    wb.delete_rows(0, 0, 4);
    wb.commit("rows");
    assert!(wb.sheets[0].tables.is_empty());
    assert!(wb.sheets[0].cells.is_empty());
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "F3"), "=ROWS(Table1)");
    // And with its sheet.
    wb.add_sheet(1);
    enter(&mut wb, 1, "A1", "=SUM(Table1[Qty])");
    wb.commit("other sheet");
    assert!(wb.delete_sheet(0));
    wb.commit("delete sheet");
    assert_eq!(shown(&wb, 0, "A1"), "=SUM(#REF!)");
    assert_eq!(wb.next_table_name(), "Table1");
}

#[test]
fn a_table_turned_into_a_range_leaves_plain_references() {
    let (mut wb, id) = orders(Locale::en());
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    type_in(&mut wb, 0, "D2", "=[@Qty]*[@Price]");
    enter(&mut wb, 0, "F1", "=SUM(Table1[Qty])");
    enter(&mut wb, 0, "F2", "=ROWS(Table1)");
    wb.commit("setup");
    assert!(wb.convert_table_to_range(0, id));
    wb.commit("convert");
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Sheet1!$B$2:$B$4)");
    assert_eq!(shown(&wb, 0, "F2"), "=ROWS(Sheet1!$A$2:$D$4)");
    assert_eq!(shown(&wb, 0, "D3"), "=Sheet1!$B3*Sheet1!$C3");
    assert_eq!(shown(&wb, 0, "C5"), "=SUBTOTAL(109,Sheet1!$C$2:$C$4)");
    assert_eq!([value(&wb, 0, "F1"), value(&wb, 0, "D3"), value(&wb, 0, "C5")], [num(6.0), num(40.0), num(60.0)]);
}

#[test]
fn a_formula_typed_next_to_a_table_makes_a_calculated_column() {
    let (mut wb, id) = orders(Locale::en());
    // Right of a data row: the table takes the column and fills it.
    let edit = type_in(&mut wb, 0, "D3", "=[@Qty]*[@Price]");
    assert_eq!((edit.expanded, edit.filled), (Some(id), Some((id, 3))));
    let table = wb.table(0, id).unwrap();
    assert_eq!((table.range, table.columns[3].name.as_str()), (rect("A1:D4"), "Column4"));
    for (cell, product) in [("D2", 10.0), ("D3", 40.0), ("D4", 90.0)] {
        assert_eq!(shown(&wb, 0, cell), "=[@Qty]*[@Price]");
        assert_eq!(stored(&wb, 0, cell), "Table1[@Qty]*Table1[@Price]");
        assert_eq!(value(&wb, 0, cell), num(product));
    }
    // A row typed under the table gets the calculated column.
    assert_eq!(type_in(&mut wb, 0, "B5", "4").expanded, Some(id));
    enter(&mut wb, 0, "C5", "5");
    wb.commit("row");
    assert_eq!((shown(&wb, 0, "D5"), value(&wb, 0, "D5")), ("=[@Qty]*[@Price]".to_string(), num(20.0)));
    // So does a row inserted in the table, and the formula can be changed
    // for the whole column from any of its cells.
    wb.table_insert_rows(0, id, 2, 1).unwrap();
    wb.commit("insert");
    assert_eq!(shown(&wb, 0, "D3"), "=[@Qty]*[@Price]");
    assert_eq!(type_in(&mut wb, 0, "D2", "=[@Qty]*2").filled, Some((id, 3)));
    assert_eq!((shown(&wb, 0, "D6"), value(&wb, 0, "D6")), ("=[@Qty]*2".to_string(), num(8.0)));
    // A formula in the row under the table, where it is about to grow.
    assert_eq!(type_in(&mut wb, 0, "D7", "=[@Qty]+1").expanded, Some(id));
    assert_eq!(stored(&wb, 0, "D7"), "Table1[@Qty]+1");
    // Anywhere else a column without its table means nothing.
    let (row, col) = at("H9");
    assert!(wb.set_cell_text(0, row, col, "=[@Qty]*2").is_err());
    assert!(wb.table_for_entry(0, row, col).is_none());
    // Under a totals row the table does not grow.
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    let (row, col) = at("B9");
    assert!(wb.set_cell_text(0, row, col, "=[@Qty]").is_err());
}

#[test]
fn a_copied_sheet_reads_its_own_tables() {
    let (mut wb, id) = orders(Locale::en());
    wb.set_table_option(0, id, TableOption::TotalsRow, true).unwrap();
    type_in(&mut wb, 0, "D2", "=[@Qty]*[@Price]");
    enter(&mut wb, 0, "F1", "=SUM(Table1[Qty])+Sheet1!B2");
    enter(&mut wb, 0, "G1", "x");
    let second = wb.add_table(0, rect("G1:G2"), true).unwrap();
    // The name the first copy would take, so that it takes another.
    wb.rename_table(0, second, "Table12").unwrap();
    enter(&mut wb, 0, "F2", "=ROWS(Table12)");
    wb.commit("setup");

    let copy = wb.duplicate_sheet(0).unwrap();
    wb.commit("copy");
    let names: Vec<&str> = wb.sheets[copy].tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Table13", "Table122"]);
    // The copy's formulas name the copy's tables, and its own sheet.
    assert_eq!(shown(&wb, copy, "F1"), "=SUM(Table13[Qty])+'Sheet1 (2)'!B2");
    assert_eq!(shown(&wb, copy, "F2"), "=ROWS(Table122)");
    assert_eq!(stored(&wb, copy, "D3"), "Table13[@Qty]*Table13[@Price]");
    assert_eq!(stored(&wb, copy, "C5"), "SUBTOTAL(109,Table13[Price])");
    // The original's are as they were.
    assert_eq!(shown(&wb, 0, "F1"), "=SUM(Table1[Qty])+Sheet1!B2");
    assert_eq!(stored(&wb, 0, "C5"), "SUBTOTAL(109,Table1[Price])");
    // Each computes from its own cells.
    enter(&mut wb, copy, "B2", "100");
    wb.commit("edit");
    assert_eq!([value(&wb, copy, "F1"), value(&wb, 0, "F1")], [num(205.0), num(7.0)]);
    assert_eq!([value(&wb, copy, "D2"), value(&wb, 0, "D2")], [num(1000.0), num(10.0)]);
}

#[test]
fn filling_sideways_moves_on_to_the_next_column() {
    let (mut wb, _) = orders(Locale::en());
    enter(&mut wb, 0, "F1", "=SUM(Table1[Item])");
    enter(&mut wb, 0, "F2", "=SUM(Table1[[Qty]:[Qty]])");
    enter(&mut wb, 0, "F3", "=COUNTA(Table1[[#Headers],[Qty]])");
    wb.commit("setup");
    // Ctrl+R and the fill handle: one column further for each cell, as in
    // Excel; the column written twice stays, and so does a plain copy.
    wb.fill_right(0, rect("F1:I3"));
    wb.commit("fill");
    assert_eq!(
        [shown(&wb, 0, "G1"), shown(&wb, 0, "H1"), shown(&wb, 0, "I1")],
        ["=SUM(Table1[Qty])", "=SUM(Table1[Price])", "=SUM(#REF!)"]
    );
    assert_eq!(shown(&wb, 0, "H2"), "=SUM(Table1[[Qty]:[Qty]])");
    assert_eq!(shown(&wb, 0, "G3"), "=COUNTA(Table1[[#Headers],[Price]])");
    assert_eq!([value(&wb, 0, "G1"), value(&wb, 0, "H1")], [num(6.0), num(60.0)]);
    wb.autofill(0, rect("F6"), rect("F6:F6"));
    enter(&mut wb, 0, "K1", "=SUM(Table1[Qty])");
    wb.autofill(0, rect("K1"), rect("K1:L1"));
    wb.autofill(0, rect("K1"), rect("K1:K2"));
    let clip = wb.copy(0, rect("K1"));
    wb.paste(0, rect("N1"), &clip, PasteMode::All);
    wb.commit("fill");
    assert_eq!(shown(&wb, 0, "L1"), "=SUM(Table1[Price])");
    assert_eq!([shown(&wb, 0, "K2"), shown(&wb, 0, "N1")], ["=SUM(Table1[Qty])", "=SUM(Table1[Qty])"]);
    assert_eq!(value(&wb, 0, "N1"), Value::Number(6.0));
}

#[test]
fn entries_without_an_equals_sign_are_formulas_too() {
    let (mut wb, id) = orders(Locale::en());
    // As Excel reads them: a sign, then any reference.
    assert_eq!(type_in(&mut wb, 0, "D2", "-[@Qty]").filled, Some((id, 3)));
    assert_eq!((shown(&wb, 0, "D4"), value(&wb, 0, "D4")), ("=-[@Qty]".to_string(), num(-3.0)));
    type_in(&mut wb, 0, "F1", "-SUM(Table1[Price])");
    type_in(&mut wb, 0, "F2", "+Table1[[#Headers],[Qty]]");
    type_in(&mut wb, 0, "H1", "=B2:B4*2");
    type_in(&mut wb, 0, "F3", "-SUM(H1#)");
    type_in(&mut wb, 0, "J1", "+H1#");
    assert_eq!(shown(&wb, 0, "F1"), "=-SUM(Table1[Price])");
    assert_eq!([value(&wb, 0, "F1"), value(&wb, 0, "F3"), value(&wb, 0, "J3")], [num(-60.0), num(-12.0), num(6.0)]);
    assert_eq!(value(&wb, 0, "F2"), Value::text("Qty"));
    // Text that starts with a sign needs its apostrophe, as in Excel.
    let (row, col) = at("F5");
    assert!(wb.set_cell_text(0, row, col, "- not a formula!").is_err());
    type_in(&mut wb, 0, "F5", "'- not a formula!");
    assert_eq!(value(&wb, 0, "F5"), Value::text("- not a formula!"));
}
