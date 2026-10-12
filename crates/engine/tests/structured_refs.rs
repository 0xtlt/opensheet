//! Structured references to tables: evaluation, how they are typed and
//! shown, and the rewrites tables rely on.

mod common;

use common::{add_table, calc, enter, err, num, shown, value, workbook};
use opensheet_engine::ErrorKind::{Ref, Value as ValueError};
use opensheet_engine::ops::PasteMode;
use opensheet_engine::parser::{parse_formula, scan_refs_in};
use opensheet_engine::{Locale, Rect, Value, Workbook};

/// Sheet1!B2:E6 is the table "Sales" (header row, three data rows, totals
/// row) with the columns Item, Qty, Unit Price and Total $.
fn sales(locale: &'static Locale) -> Workbook {
    let mut wb = Workbook::new(locale);
    wb.add_sheet(1);
    let rows = [
        ["Item", "Qty", "Unit Price", "Total $"],
        ["nut", "1", "10", ""],
        ["bolt", "2", "20", ""],
        ["gear", "3", "30", ""],
        ["Total", "6", "", ""],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, 1 + r as u32, 1 + c as u32, text)
                .unwrap();
        }
    }
    add_table(&mut wb, 0, "Sales", "B2:E6", true);
    wb.commit("table");
    wb
}

#[test]
fn every_form_evaluates() {
    let mut wb = sales(Locale::en());
    // (formula typed on row 4, i.e. the table's second data row, value)
    let cases: [(&str, Value); 24] = [
        ("=SUM(Sales[Qty])", num(6.0)),
        ("=SUM(Sales[[Qty]:[Unit Price]])", num(66.0)),
        ("=SUM(Sales[[Unit Price]:[Qty]])", num(66.0)),
        ("=ROWS(Sales)*10+COLUMNS(Sales)", num(34.0)),
        ("=ROWS(Sales[#All])*10+COLUMNS(Sales[#All])", num(54.0)),
        ("=ROWS(Sales[#Data])", num(3.0)),
        (
            "=COLUMNS(Sales[#Headers])*10+ROWS(Sales[#Headers])",
            num(41.0),
        ),
        ("=INDEX(Sales[#Headers],1,3)", Value::text("Unit Price")),
        ("=INDEX(Sales[#Totals],1,2)", num(6.0)),
        ("=Sales[[#Headers],[Qty]]", Value::text("Qty")),
        ("=Sales[[#Totals],[Qty]]", num(6.0)),
        ("=SUM(Sales[[#All],[Qty]])", num(12.0)),
        ("=SUM(Sales[[#Data],[Qty]])", num(6.0)),
        ("=ROWS(Sales[[#Headers],[#Data]])", num(4.0)),
        (
            "=ROWS(Sales[[#Data],[#Totals]])*10+COUNT(Sales[[#Data],[#Totals],[Qty]])",
            num(44.0),
        ),
        ("=COUNTA(Sales[[#Headers],[#Data],[Item]:[Qty]])", num(8.0)),
        ("=Sales[@Qty]", num(2.0)),
        ("=Sales[@Qty]*Sales[@[Unit Price]]", num(40.0)),
        ("=Sales[[#This Row],[Qty]]+1", num(3.0)),
        (
            "=COUNTA(Sales[@])*10+SUM(Sales[@[Qty]:[Unit Price]])",
            num(52.0),
        ),
        ("=ROW(Sales[@])*10+COLUMNS(Sales[#This Row])", num(44.0)),
        ("=SUM(INDIRECT(\"Sales[Qty]\"))+ROW(Sales)", num(9.0)),
        (
            "=SUM((Sales[Qty],Sales[Unit Price]))+SUM(Sales[Qty] Sales[@])",
            num(68.0),
        ),
        ("=sum(SALES[qty])+@Sales[Qty]", num(8.0)),
    ];
    for (formula, expected) in cases {
        assert_eq!(calc(&mut wb, 0, "H4", formula), expected, "{formula}");
    }
    // The same from another sheet, except for "this row".
    assert_eq!(
        calc(&mut wb, 1, "A1", "=SUM(Sales[Qty])+ROWS(Sales[#All])"),
        num(11.0)
    );
    assert_eq!(calc(&mut wb, 1, "A4", "=Sales[@Qty]"), err(ValueError));
    // "This row" above, below and beside the data.
    assert_eq!(calc(&mut wb, 0, "H2", "=Sales[@Qty]"), err(ValueError));
    assert_eq!(calc(&mut wb, 0, "H6", "=Sales[@Qty]"), err(ValueError));
    assert_eq!(calc(&mut wb, 0, "H9", "=Sales[@]"), err(ValueError));
    // What does not exist.
    assert_eq!(calc(&mut wb, 0, "H4", "=SUM(Sales[Price])"), err(Ref));
    assert_eq!(
        calc(&mut wb, 0, "H4", "=SUM(Sales[[Qty]:[Nope]])"),
        err(Ref)
    );
    assert_eq!(calc(&mut wb, 0, "H4", "=SUM(Orders[Qty])"), err(Ref));
    assert_eq!(calc(&mut wb, 0, "H4", "=Orders[@Qty]"), err(Ref));
    wb.sheets[0].tables[0].totals_row = false;
    wb.sheets[0].tables[0].range = Rect::parse("B2:E5").unwrap();
    assert_eq!(calc(&mut wb, 0, "H4", "=Sales[[#Totals],[Qty]]"), err(Ref));
    assert_eq!(
        calc(
            &mut wb,
            0,
            "H4",
            "=ROWS(Sales[[#Data],[#Totals]])*10+ROWS(Sales[#All])"
        ),
        num(34.0)
    );
    wb.sheets[0].tables[0].header_row = false;
    wb.sheets[0].tables[0].range = Rect::parse("B3:E5").unwrap();
    assert_eq!(calc(&mut wb, 0, "H4", "=Sales[[#Headers],[Qty]]"), err(Ref));
    assert_eq!(
        calc(
            &mut wb,
            0,
            "H4",
            "=ROWS(Sales[[#Headers],[#Data]])*10+SUM(Sales[Qty])"
        ),
        num(36.0)
    );
}

#[test]
fn typed_inside_and_outside_the_table() {
    let mut wb = sales(Locale::en());
    // Inside the table the name may be left out; it never shows.
    enter(&mut wb, 0, "E3", "=[@Qty]*[@[Unit Price]]");
    enter(
        &mut wb,
        0,
        "E4",
        "=Sales[@Qty]*Sales[[#This Row],[Unit Price]]",
    );
    enter(
        &mut wb,
        0,
        "E5",
        "=[@Qty]*[@[Unit Price]]+SUM([Qty])-SUM(Sales[Qty])",
    );
    enter(
        &mut wb,
        0,
        "E6",
        "=SUBTOTAL(109,[Total $])+COUNTA([#Headers])+ROWS([[#Data],[Qty]])",
    );
    wb.commit("formulas");
    assert_eq!(value(&wb, 0, "E3"), num(10.0));
    assert_eq!(value(&wb, 0, "E4"), num(40.0));
    assert_eq!(value(&wb, 0, "E5"), num(90.0));
    assert_eq!(value(&wb, 0, "E6"), num(147.0));
    assert_eq!(shown(&wb, 0, "E3"), "=[@Qty]*[@[Unit Price]]");
    assert_eq!(shown(&wb, 0, "E4"), "=[@Qty]*[@[Unit Price]]");
    assert_eq!(
        shown(&wb, 0, "E5"),
        "=[@Qty]*[@[Unit Price]]+SUM([Qty])-SUM([Qty])"
    );
    assert_eq!(
        shown(&wb, 0, "E6"),
        "=SUBTOTAL(109,[[Total $]])+COUNTA([#Headers])+ROWS([[#Data],[Qty]])"
    );
    assert_eq!(wb.edit_text(0, 2, 4), "=[@Qty]*[@[Unit Price]]");
    // The stored formula always names its table...
    let stored = |wb: &Workbook, cell: &str| {
        common::at(cell).pipe(|(r, c)| match &wb.cell(0, r, c).unwrap().content {
            opensheet_engine::Content::Formula(e) => e.to_text(Locale::en()),
            other => panic!("{other:?}"),
        })
    };
    assert_eq!(stored(&wb, "E3"), "Sales[@Qty]*Sales[@[Unit Price]]");
    // ...so that it keeps its meaning when copied out of the table.
    let clip = wb.copy(0, Rect::parse("E3").unwrap());
    wb.paste(0, Rect::parse("H3").unwrap(), &clip, PasteMode::All);
    wb.paste(1, Rect::parse("A1").unwrap(), &clip, PasteMode::All);
    wb.commit("paste");
    assert_eq!(shown(&wb, 0, "H3"), "=Sales[@Qty]*Sales[@[Unit Price]]");
    assert_eq!(value(&wb, 0, "H3"), num(10.0));
    assert_eq!(shown(&wb, 1, "A1"), "=Sales[@Qty]*Sales[@[Unit Price]]");
    assert_eq!(value(&wb, 1, "A1"), err(ValueError));
    // Filling down keeps the columns and follows the row.
    wb.fill_down(0, Rect::parse("H3:H5").unwrap());
    wb.commit("fill");
    assert_eq!(shown(&wb, 0, "H5"), "=Sales[@Qty]*Sales[@[Unit Price]]");
    assert_eq!(value(&wb, 0, "H5"), num(90.0));
    // Outside a table, a reference without a table name is refused.
    for text in ["=[@Qty]", "=SUM([Qty])", "=[#All]", "=1+[[#Headers],[Qty]]"] {
        assert!(wb.set_cell_text(0, 2, 9, text).is_err(), "{text}");
        assert!(wb.set_cell_text(1, 5, 5, text).is_err(), "{text}");
    }
    assert!(wb.cell(0, 2, 9).is_none());
    // Inserting rows or columns changes the table, not the references.
    wb.insert_rows(0, 0, 3);
    wb.insert_cols(0, 0, 1);
    wb.commit("insert");
    assert_eq!(shown(&wb, 0, "I6"), "=Sales[@Qty]*Sales[@[Unit Price]]");
}

/// `x.pipe(f)` is `f(x)`.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[test]
fn french() {
    let mut wb = sales(Locale::fr());
    wb.sheets[0].tables[0].name = "Ventes".into();
    let cases = [
        ("=SOMME(Ventes[Qty])", "=SOMME(Ventes[Qty])", num(6.0)),
        (
            "=LIGNES(Ventes[#Tout])+LIGNES(Ventes[#Données])",
            "=LIGNES(Ventes[#Tout])+LIGNES(Ventes[#Données])",
            num(8.0),
        ),
        (
            "=Ventes[[#En-têtes];[Qty]]",
            "=Ventes[[#En-têtes];[Qty]]",
            Value::text("Qty"),
        ),
        (
            "=Ventes[[#Totaux];[Qty]]",
            "=Ventes[[#Totaux];[Qty]]",
            num(6.0),
        ),
        (
            "=Ventes[[#Cette ligne];[Qty]]*1,5",
            "=Ventes[@Qty]*1,5",
            num(3.0),
        ),
        (
            "=Ventes[@Qty]+Ventes[@[Unit Price]]",
            "=Ventes[@Qty]+Ventes[@[Unit Price]]",
            num(22.0),
        ),
        (
            "=NBVAL(Ventes[[#En-têtes];[#Données];[Item]:[Qty]])",
            "=NBVAL(Ventes[[#En-têtes];[#Données];[Item]:[Qty]])",
            num(8.0),
        ),
        (
            "=SOMME(Ventes[[#All],[Qty]])",
            "=SOMME(Ventes[[#Tout];[Qty]])",
            num(12.0),
        ),
    ];
    for (typed, shown_as, expected) in cases {
        assert_eq!(calc(&mut wb, 0, "H4", typed), expected, "{typed}");
        assert_eq!(shown(&wb, 0, "H4"), shown_as);
    }
    enter(
        &mut wb,
        0,
        "E4",
        "=[@Qty]*[@[Unit Price]]+NBVAL([#En-têtes])",
    );
    wb.commit("inside");
    assert_eq!(value(&wb, 0, "E4"), num(44.0));
    assert_eq!(
        shown(&wb, 0, "E4"),
        "=[@Qty]*[@[Unit Price]]+NBVAL([#En-têtes])"
    );
    // The same workbook read in English.
    wb.locale = Locale::en();
    assert_eq!(
        shown(&wb, 0, "E4"),
        "=[@Qty]*[@[Unit Price]]+COUNTA([#Headers])"
    );
    assert_eq!(shown(&wb, 0, "H4"), "=SUM(Ventes[[#All],[Qty]])");
}

#[test]
fn rewrites_for_tables() {
    let mut wb = sales(Locale::en());
    enter(&mut wb, 0, "E3", "=[@Qty]*[@[Unit Price]]");
    enter(
        &mut wb,
        0,
        "H4",
        "=SUM(Sales[Qty])+Sales[@Qty]+ROWS(Sales)+SUM(Sales[[#All],[Qty]:[Unit Price]])",
    );
    enter(&mut wb, 1, "A1", "=SUM(Sales[Qty])*COUNTA(sales[#Headers])");
    enter(&mut wb, 1, "A2", "=Sales[@Qty]+SUM(Other[Qty])");
    wb.define_name(
        "Quantities",
        parse_formula("Sales[Qty]", Locale::en()).unwrap(),
    );
    wb.commit("formulas");
    let before: Vec<Value> = vec![
        value(&wb, 0, "E3"),
        value(&wb, 0, "H4"),
        value(&wb, 1, "A1"),
    ];
    assert_eq!(before, [num(10.0), num(83.0), num(24.0)]);

    // The table renamed. (Like any direct change to a sheet, changing a
    // table starts with the undo snapshot.)
    wb.snapshot();
    wb.sheets[0].tables[0].name = "Orders".into();
    assert!(wb.rename_table_refs("Sales", "Orders"));
    wb.commit("rename table");
    assert_eq!(shown(&wb, 0, "E3"), "=[@Qty]*[@[Unit Price]]");
    assert_eq!(
        shown(&wb, 0, "H4"),
        "=SUM(Orders[Qty])+Orders[@Qty]+ROWS(Orders)+SUM(Orders[[#All],[Qty]:[Unit Price]])"
    );
    assert_eq!(
        shown(&wb, 1, "A1"),
        "=SUM(Orders[Qty])*COUNTA(Orders[#Headers])"
    );
    assert_eq!(shown(&wb, 1, "A2"), "=Orders[@Qty]+SUM(Other[Qty])");
    assert_eq!(
        wb.names["QUANTITIES"].expr.to_text(Locale::en()),
        "Orders[Qty]"
    );
    assert_eq!(
        vec![
            value(&wb, 0, "E3"),
            value(&wb, 0, "H4"),
            value(&wb, 1, "A1")
        ],
        before
    );
    assert!(
        !wb.rename_table_refs("Sales", "Orders"),
        "nothing left to rename"
    );

    // A column renamed (the header cell and the table's column both).
    wb.snapshot();
    enter(&mut wb, 0, "C2", "Count");
    wb.sheets[0].tables[0].columns[1].name = "Count".into();
    assert!(wb.rename_table_column_refs("Orders", "qty", "Count"));
    wb.commit("rename column");
    assert_eq!(shown(&wb, 0, "E3"), "=[@Count]*[@[Unit Price]]");
    assert_eq!(
        shown(&wb, 0, "H4"),
        "=SUM(Orders[Count])+Orders[@Count]+ROWS(Orders)+SUM(Orders[[#All],[Count]:[Unit Price]])"
    );
    assert_eq!(
        shown(&wb, 1, "A2"),
        "=Orders[@Count]+SUM(Other[Qty])",
        "other tables keep their columns"
    );
    assert_eq!(
        wb.names["QUANTITIES"].expr.to_text(Locale::en()),
        "Orders[Count]"
    );
    assert_eq!(
        vec![
            value(&wb, 0, "E3"),
            value(&wb, 0, "H4"),
            value(&wb, 1, "A1")
        ],
        before
    );
    // A name needing brackets and escapes.
    wb.snapshot();
    wb.sheets[0].tables[0].columns[1].name = "# of [items]".into();
    assert!(wb.rename_table_column_refs("Orders", "Count", "# of [items]"));
    wb.commit("rename column");
    assert_eq!(shown(&wb, 0, "E3"), "=[@['# of '[items']]]*[@[Unit Price]]");
    assert_eq!(
        shown(&wb, 1, "A1"),
        "=SUM(Orders[['# of '[items']]])*COUNTA(Orders[#Headers])"
    );
    assert_eq!(value(&wb, 1, "A1"), num(24.0));
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "E3"), "=[@Count]*[@[Unit Price]]");
    assert_eq!(value(&wb, 0, "E3"), num(10.0));

    // The table turned back into a plain range.
    assert!(wb.table_refs_to_ranges("Orders"));
    wb.sheets[0].tables.clear();
    wb.commit("convert to range");
    assert_eq!(shown(&wb, 0, "E3"), "=Sheet1!$C3*Sheet1!$D3");
    assert_eq!(
        shown(&wb, 0, "H4"),
        "=SUM(Sheet1!$C$3:$C$5)+Sheet1!$C4+ROWS(Sheet1!$B$3:$E$5)+SUM(Sheet1!$C$2:$D$6)"
    );
    assert_eq!(
        shown(&wb, 1, "A1"),
        "=SUM(Sheet1!$C$3:$C$5)*COUNTA(Sheet1!$B$2:$E$2)"
    );
    // "This row" from another sheet had no value; another table is left alone.
    assert_eq!(shown(&wb, 1, "A2"), "=#VALUE!+SUM(Other[Qty])");
    assert_eq!(
        wb.names["QUANTITIES"].expr.to_text(Locale::en()),
        "Sheet1!$C$3:$C$5"
    );
    assert_eq!(
        vec![
            value(&wb, 0, "E3"),
            value(&wb, 0, "H4"),
            value(&wb, 1, "A1")
        ],
        before
    );
    // One undo step brings the table and its references back.
    assert!(wb.undo());
    assert_eq!(wb.sheets[0].tables.len(), 1);
    assert_eq!(shown(&wb, 0, "E3"), "=[@Count]*[@[Unit Price]]");
    assert_eq!(
        shown(&wb, 1, "A1"),
        "=SUM(Orders[Count])*COUNTA(Orders[#Headers])"
    );

    // A column deleted.
    assert!(wb.delete_table_column_refs("Orders", "Unit Price"));
    wb.commit("delete column");
    assert_eq!(shown(&wb, 0, "E3"), "=[@Count]*#REF!");
    assert_eq!(
        shown(&wb, 0, "H4"),
        "=SUM(Orders[Count])+Orders[@Count]+ROWS(Orders)+SUM(#REF!)"
    );
    assert_eq!(
        shown(&wb, 1, "A1"),
        "=SUM(Orders[Count])*COUNTA(Orders[#Headers])"
    );
}

#[test]
fn coloured_while_typing() {
    let wb = sales(Locale::en());
    let src =
        "SUM(Sales[Qty])+[@Qty]*Sales[[#Headers],[Unit Price]]+ROWS(Sales)+SUM([#All])+Nope[x]";
    // Typed in E4, inside the table.
    let found: Vec<(&str, String)> = scan_refs_in(src, &wb, 0, 3, 4)
        .into_iter()
        .map(|s| (&src[s.range.clone()], s.reference.to_text()))
        .collect();
    assert_eq!(
        found,
        [
            ("Sales[Qty]", "Sheet1!C3:C5".to_string()),
            ("[@Qty]", "Sheet1!C4".to_string()),
            ("Sales[[#Headers],[Unit Price]]", "Sheet1!D2".to_string()),
            ("Sales", "Sheet1!B3:E5".to_string()),
            ("[#All]", "Sheet1!B2:E6".to_string()),
        ]
    );
    // Typed outside: the forms without a table name mean nothing.
    assert_eq!(scan_refs_in(src, &wb, 1, 0, 0).len(), 3);
}

#[test]
fn workbook_evaluate_and_names() {
    let mut wb = sales(Locale::en());
    assert_eq!(wb.evaluate(0, "=SUM(Sales[Qty])*2"), Ok(num(12.0)));
    // A defined name may stand for part of a table.
    wb.define_name(
        "Prices",
        parse_formula("Sales[Unit Price]", Locale::en()).unwrap(),
    );
    assert_eq!(
        calc(&mut wb, 1, "A1", "=SUM(Prices)+MAX(Prices)"),
        num(90.0)
    );
    // The name of a table is not free for a defined name.
    assert!(
        wb.check_name(opensheet_engine::names::NameScope::Workbook, "sales", None)
            .is_err()
    );
    let _ = workbook(1);
}
