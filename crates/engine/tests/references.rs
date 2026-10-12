//! The reference operators end to end: spilled ranges, 3-D references,
//! unions, intersections and implicit intersection, through evaluation,
//! copying, structural edits and sheet operations.

mod common;

use common::{calc, enter, err, num, shown, value, workbook};
use opensheet_engine::ErrorKind::{Null, Ref, Value as ValueError};
use opensheet_engine::chart::ChartKind;
use opensheet_engine::ops::PasteMode;
use opensheet_engine::parser::{parse_formula, scan_refs_in};
use opensheet_engine::{Locale, Rect, Value, Workbook};

/// Copy one cell and paste it at (`row`, `col`) of `dest_sheet`.
fn copy_cell(wb: &mut Workbook, sheet: usize, cell: &str, dest_sheet: usize, row: u32, col: u32) {
    let clip = wb.copy(sheet, Rect::parse(cell).unwrap());
    wb.paste(dest_sheet, Rect::cell(row, col), &clip, PasteMode::All);
}

/// Three sheets with A1:A3 = 1,2,3 times the sheet number and C1:C3 ten
/// times that.
fn three_sheets() -> Workbook {
    let mut wb = workbook(3);
    for s in 0..3 {
        for i in 1..=3 {
            enter(&mut wb, s, &format!("A{i}"), &format!("{}", i * (s + 1)));
            enter(
                &mut wb,
                s,
                &format!("C{i}"),
                &format!("{}", 10 * i * (s + 1)),
            );
        }
    }
    wb.commit("data");
    wb
}

#[test]
fn spilled_range_operator() {
    let mut wb = workbook(2);
    enter(&mut wb, 0, "A1", "=SEQUENCE(3,2)");
    enter(&mut wb, 1, "B2", "=SEQUENCE(2)*10");
    enter(&mut wb, 0, "D1", "5");
    enter(&mut wb, 0, "D2", "=D1+1");
    wb.commit("data");
    assert_eq!(calc(&mut wb, 0, "F1", "=SUM(A1#)"), num(21.0));
    assert_eq!(
        calc(&mut wb, 0, "F2", "=ROWS(A1#)*10+COLUMNS(A1#)"),
        num(32.0)
    );
    assert_eq!(
        calc(&mut wb, 0, "F3", "=SUM(Sheet2!B2#)+COUNT(Sheet2!B2#)"),
        num(32.0)
    );
    assert_eq!(calc(&mut wb, 0, "F4", "=INDEX(A1#,3,2)"), num(6.0));
    // The array itself spills again.
    assert_eq!(calc(&mut wb, 0, "H1", "=A1#*2"), num(2.0));
    assert_eq!(value(&wb, 0, "I3"), num(12.0));
    // Nothing spills from a constant, a plain formula, a blank cell, a cell
    // inside (not at the top of) an array, or several cells.
    for anchor in [
        "D1",
        "D2",
        "E9",
        "A2",
        "B1",
        "A1:A2",
        "Sheet2!B3",
        "Nowhere!A1",
    ] {
        assert_eq!(
            calc(&mut wb, 0, "F6", &format!("={anchor}#")),
            err(Ref),
            "{anchor}#"
        );
    }
    // The reference follows the array as it changes size.
    enter(&mut wb, 0, "A1", "=SEQUENCE(4,2)");
    wb.commit("grow");
    assert_eq!(value(&wb, 0, "F1"), num(36.0));
    assert_eq!(value(&wb, 0, "F2"), num(42.0));
    // French: same syntax.
    let mut fr = Workbook::new(Locale::fr());
    enter(&mut fr, 0, "A1", "=SEQUENCE(3)");
    assert_eq!(calc(&mut fr, 0, "C1", "=SOMME(A1#)"), num(6.0));
    assert_eq!(shown(&fr, 0, "C1"), "=SOMME(A1#)");
}

#[test]
fn spilled_range_follows_edits() {
    let mut wb = workbook(2);
    enter(&mut wb, 1, "B2", "=SEQUENCE(3)");
    enter(&mut wb, 0, "A1", "=SUM(Sheet2!B2#)");
    enter(&mut wb, 1, "D5", "=SUM(B2#)+SUM($B$2#)");
    wb.commit("data");
    // Copying moves the relative anchor.
    copy_cell(&mut wb, 1, "D5", 1, 5, 4);
    wb.commit("copy");
    assert_eq!(shown(&wb, 1, "E6"), "=SUM(C3#)+SUM($B$2#)");
    // Rows inserted above the anchor.
    wb.insert_rows(1, 0, 2);
    wb.commit("insert");
    assert_eq!(shown(&wb, 0, "A1"), "=SUM(Sheet2!B4#)");
    assert_eq!(shown(&wb, 1, "D7"), "=SUM(B4#)+SUM($B$4#)");
    assert_eq!(value(&wb, 0, "A1"), num(6.0));
    // The sheet renamed.
    assert!(wb.rename_sheet(1, "Data set"));
    wb.commit("rename");
    assert_eq!(shown(&wb, 0, "A1"), "=SUM('Data set'!B4#)");
    // The anchor deleted: nothing is left to spill.
    wb.delete_rows(1, 3, 1);
    wb.commit("delete");
    assert_eq!(shown(&wb, 0, "A1"), "=SUM('Data set'!#REF!)");
    assert_eq!(shown(&wb, 1, "D6"), "=SUM(#REF!)+SUM(#REF!)");
    assert_eq!(value(&wb, 0, "A1"), err(Ref));
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "A1"), "=SUM('Data set'!B4#)");
    assert_eq!(value(&wb, 0, "A1"), num(6.0));
}

#[test]
fn three_d_references_in_functions() {
    let mut wb = three_sheets();
    wb.add_sheet(3);
    wb.commit("sheet 4");
    let cases: [(&str, f64); 22] = [
        ("=SUM(Sheet1:Sheet3!A1)", 6.0),
        ("=SUM(Sheet1:Sheet3!A1:A3,1)", 37.0),
        ("=SUM(Sheet3:Sheet1!A1:A3)", 36.0),
        ("=SUM('Sheet1:Sheet3'!A:A)", 36.0),
        ("=AVERAGE(Sheet1:Sheet3!A1:A3)", 4.0),
        ("=AVERAGEA(Sheet1:Sheet3!A1:A3)", 4.0),
        ("=COUNT(Sheet1:Sheet3!A:A)", 9.0),
        ("=COUNTA(Sheet1:Sheet3!A1:C3)", 18.0),
        ("=MAX(Sheet2:Sheet3!C1:C3)", 90.0),
        ("=MAXA(Sheet1:Sheet2!A1:A3)", 6.0),
        ("=MIN(Sheet2:Sheet3!A1:A3)", 2.0),
        ("=MINA(Sheet2:Sheet3!A1:A3)", 2.0),
        ("=PRODUCT(Sheet1:Sheet3!A2)", 48.0),
        ("=VAR.P(Sheet1:Sheet3!A1)", 2.0 / 3.0),
        ("=STDEV.S(Sheet1:Sheet3!A1)", 1.0),
        ("=MEDIAN(Sheet1:Sheet3!A1:A3)", 3.0),
        ("=LARGE(Sheet1:Sheet3!A1:A3,2)", 6.0),
        ("=SMALL(Sheet1:Sheet3!A1:A3,2)", 2.0),
        ("=RANK(3,Sheet1:Sheet3!A1:A3)", 5.0),
        ("=SUMSQ(Sheet1:Sheet2!A1)", 5.0),
        ("=SHEETS(Sheet1:Sheet3!A1)*10+SHEET(Sheet2:Sheet3!A1)", 32.0),
        (
            "=ROWS(VSTACK(Sheet1:Sheet3!A1:A3))*10+COLUMNS(HSTACK(Sheet1:Sheet3!A1:A3))",
            93.0,
        ),
    ];
    for (formula, expected) in cases {
        match calc(&mut wb, 3, "A1", formula) {
            Value::Number(n) => assert!((n - expected).abs() < 1e-12, "{formula} gave {n}"),
            other => panic!("{formula} gave {other:?}"),
        }
    }
    assert_eq!(
        calc(
            &mut wb,
            3,
            "A1",
            "=AND(Sheet1:Sheet3!A1)&OR(Sheet1:Sheet3!A1)&XOR(Sheet1:Sheet3!A1)"
        ),
        Value::text("TRUETRUETRUE")
    );
    assert_eq!(
        calc(
            &mut wb,
            3,
            "A1",
            "=CONCAT(Sheet1:Sheet3!A1)&TEXTJOIN(\"-\",TRUE,Sheet1:Sheet3!A2)"
        ),
        Value::text("1232-4-6")
    );
    // Where one range is expected, several sheets are `#VALUE!`.
    for formula in [
        "=Sheet1:Sheet3!A1",
        "=Sheet1:Sheet3!A1+1",
        "=ABS(Sheet1:Sheet3!A1)",
        "=SUMIF(Sheet1:Sheet3!A1:A3,\">1\")",
        "=VLOOKUP(1,Sheet1:Sheet3!A1:C3,2)",
        "=ROWS(Sheet1:Sheet3!A1:A3)",
        "=SUMPRODUCT(Sheet1:Sheet3!A1:A3)",
        "=IF(TRUE,Sheet1:Sheet3!A1)",
    ] {
        assert_eq!(
            calc(&mut wb, 3, "A1", formula),
            err(ValueError),
            "{formula}"
        );
    }
    assert_eq!(calc(&mut wb, 3, "A1", "=SUM(Sheet1:Nowhere!A1)"), err(Ref));
    // French.
    let mut fr = Workbook::new(Locale::fr());
    fr.add_sheet(1);
    enter(&mut fr, 0, "A1", "1,5");
    enter(&mut fr, 1, "A1", "2,5");
    assert_eq!(
        calc(
            &mut fr,
            0,
            "B1",
            "=SOMME(Feuil1:Feuil2!A1)+NB('Feuil1:Feuil2'!A:A)"
        ),
        num(6.0)
    );
    assert_eq!(
        shown(&fr, 0, "B1"),
        "=SOMME(Feuil1:Feuil2!A1)+NB(Feuil1:Feuil2!A:A)"
    );
}

/// Sheet 0 holds `=SUM(...)` over spans of the four data sheets.
fn spans() -> Workbook {
    let mut wb = workbook(5);
    for s in 0..5 {
        enter(&mut wb, s, "A1", &format!("{}", s + 1));
    }
    enter(&mut wb, 0, "B1", "=SUM(Sheet2:Sheet4!A1)");
    enter(&mut wb, 0, "B2", "=SUM(Sheet2:Sheet3!A1)+Sheet3!A1");
    wb.commit("data");
    wb
}

fn b1_b2(wb: &Workbook) -> (String, Value, String, Value) {
    let s = wb.sheet_index("Sheet1").unwrap();
    (
        shown(wb, s, "B1"),
        value(wb, s, "B1"),
        shown(wb, s, "B2"),
        value(wb, s, "B2"),
    )
}

#[test]
fn three_d_references_follow_the_sheets() {
    let initial = (
        "=SUM(Sheet2:Sheet4!A1)".to_string(),
        num(9.0),
        "=SUM(Sheet2:Sheet3!A1)+Sheet3!A1".to_string(),
        num(8.0),
    );
    let mut wb = spans();
    assert_eq!(b1_b2(&wb), initial);

    // Rename an end sheet.
    assert!(wb.rename_sheet(1, "First one"));
    wb.commit("rename");
    assert_eq!(
        b1_b2(&wb),
        (
            "=SUM('First one:Sheet4'!A1)".into(),
            num(9.0),
            "=SUM('First one:Sheet3'!A1)+Sheet3!A1".into(),
            num(8.0)
        )
    );
    assert!(wb.undo());
    assert_eq!(b1_b2(&wb), initial);

    // A sheet moved into the span counts, one moved out does not.
    wb.move_sheet(4, 2);
    wb.commit("move in");
    assert_eq!(
        b1_b2(&wb),
        (initial.0.clone(), num(14.0), initial.2.clone(), num(13.0))
    );
    assert!(wb.undo());
    // Sheet3 lies inside the first span and ends the second one: carried
    // in front of both, it leaves the first and the second lets go of it.
    wb.move_sheet(2, 0);
    wb.commit("move out");
    assert_eq!(
        b1_b2(&wb),
        (
            initial.0.clone(),
            num(6.0),
            "=SUM(Sheet2!A1)+Sheet3!A1".into(),
            num(5.0)
        )
    );
    assert!(wb.undo());

    // An end sheet moved, the span stretching with it.
    wb.move_sheet(3, 4);
    wb.commit("stretch");
    assert_eq!(b1_b2(&wb).1, num(14.0));
    assert!(wb.undo());

    // An end sheet carried past the other end: the span lets go of it.
    wb.move_sheet(1, 4);
    wb.commit("first past last");
    assert_eq!(
        b1_b2(&wb),
        (
            "=SUM(Sheet3:Sheet4!A1)".into(),
            num(7.0),
            "=SUM(Sheet3!A1)+Sheet3!A1".into(),
            num(6.0)
        )
    );
    assert!(wb.undo());
    wb.move_sheet(3, 1);
    wb.commit("last before first");
    assert_eq!(
        b1_b2(&wb),
        (
            "=SUM(Sheet2:Sheet3!A1)".into(),
            num(5.0),
            initial.2.clone(),
            num(8.0)
        )
    );
    assert!(wb.undo());
    assert_eq!(b1_b2(&wb), initial);

    // A sheet inserted inside the span.
    let new = wb.add_sheet(2);
    enter(&mut wb, new, "A1", "100");
    wb.commit("insert");
    assert_eq!(
        b1_b2(&wb),
        (initial.0.clone(), num(109.0), initial.2.clone(), num(108.0))
    );
    assert!(wb.undo());
    assert_eq!(b1_b2(&wb), initial);

    // A sheet deleted inside the span, then the end sheets one by one.
    assert!(wb.delete_sheet(2));
    wb.commit("delete middle");
    assert_eq!(
        b1_b2(&wb),
        (
            "=SUM(Sheet2:Sheet4!A1)".into(),
            num(6.0),
            "=SUM(Sheet2!A1)+#REF!A1".into(),
            err(Ref)
        )
    );
    assert!(wb.delete_sheet(1));
    wb.commit("delete first");
    assert_eq!(
        b1_b2(&wb),
        (
            "=SUM(Sheet4!A1)".into(),
            num(4.0),
            "=SUM(#REF!A1)+#REF!A1".into(),
            err(Ref)
        )
    );
    assert!(wb.delete_sheet(1));
    wb.commit("delete last");
    assert_eq!(b1_b2(&wb).0, "=SUM(#REF!A1)");
    assert_eq!(b1_b2(&wb).1, err(Ref));
    assert!(wb.undo() && wb.undo() && wb.undo());
    assert_eq!(b1_b2(&wb), initial);
    assert!(wb.redo() && wb.redo() && wb.redo());
    assert_eq!(b1_b2(&wb).0, "=SUM(#REF!A1)");
}

#[test]
fn three_d_references_and_cell_edits() {
    let mut wb = three_sheets();
    enter(
        &mut wb,
        0,
        "E5",
        "=SUM(Sheet1:Sheet3!A1:A3)+SUM(Sheet2:Sheet3!$C$1)",
    );
    wb.commit("formula");
    // Copying shifts the cells, the same on every sheet.
    copy_cell(&mut wb, 0, "E5", 0, 5, 6);
    wb.commit("copy");
    assert_eq!(
        shown(&wb, 0, "G6"),
        "=SUM(Sheet1:Sheet3!C2:C4)+SUM(Sheet2:Sheet3!$C$1)"
    );
    assert_eq!(value(&wb, 0, "G6"), num(350.0));
    let e = parse_formula("SUM(Sheet1:Sheet3!A1)", Locale::en()).unwrap();
    assert_eq!(e.shifted(-1, 0).to_text(Locale::en()), "SUM(#REF!)");
    // Rows inserted on one sheet of the span leave the reference alone:
    // its cells are those of every sheet.
    wb.insert_rows(1, 0, 1);
    wb.commit("insert");
    assert_eq!(
        shown(&wb, 0, "E5"),
        "=SUM(Sheet1:Sheet3!A1:A3)+SUM(Sheet2:Sheet3!$C$1)"
    );
    assert_eq!(value(&wb, 0, "E5"), num(36.0 - 6.0 + 30.0));
}

#[test]
fn deleting_a_sheet() {
    let mut wb = workbook(3);
    assert!(wb.rename_sheet(1, "Data"));
    for i in 1..=3 {
        enter(&mut wb, 1, &format!("A{i}"), &format!("label {i}"));
        enter(&mut wb, 1, &format!("B{i}"), &format!("{i}"));
        enter(&mut wb, 2, &format!("B{i}"), &format!("{}", i * 10));
    }
    enter(&mut wb, 0, "A1", "=Data!B1+1");
    enter(&mut wb, 0, "A2", "=SUM(Data!$B$1:B3,Sheet3!B1)");
    enter(&mut wb, 0, "A3", "=SUM(Data!B:B)+COUNT(Data!1:2)");
    enter(&mut wb, 0, "A4", "=SUM(Data:Sheet3!B1)");
    enter(&mut wb, 0, "A5", "=Data!Local+Sheet3!B1");
    enter(
        &mut wb,
        0,
        "A6",
        "=SUM((Data!B1,Sheet3!B1))+SUM(Data!B1:B3 Data!B2:C2)+@Data!B1:B3",
    );
    enter(&mut wb, 2, "D1", "=data!B2*2");
    enter(&mut wb, 1, "D1", "=B1+Sheet3!B1");
    wb.define_name(
        "Total",
        parse_formula("SUM(Data!$B$1:$B$3)", Locale::en()).unwrap(),
    );
    wb.define_name("Other", parse_formula("Sheet3!$B$1", Locale::en()).unwrap());
    wb.add_name(
        opensheet_engine::names::NameScope::Sheet(1),
        "Local",
        parse_formula("Data!$B$3", Locale::en()).unwrap(),
        "",
    )
    .unwrap();
    // One chart on the first sheet plotting the data sheet, one plotting
    // the third sheet.
    let chart = wb.chart_from_range(1, Rect::parse("A1:B3").unwrap(), ChartKind::Column);
    let on_data = wb.add_chart(0, chart).unwrap();
    let chart = wb.chart_from_range(2, Rect::parse("B1:B3").unwrap(), ChartKind::Line);
    let on_third = wb.add_chart(0, chart).unwrap();
    wb.commit("setup");
    assert_eq!(value(&wb, 0, "A2"), num(16.0));
    assert_eq!(value(&wb, 0, "A5"), num(13.0));
    assert_eq!(wb.chart(0, on_data).unwrap().series.len(), 1);

    assert!(wb.delete_sheet(1));
    wb.commit("delete");
    let formulas: Vec<String> = (1..=6).map(|i| shown(&wb, 0, &format!("A{i}"))).collect();
    assert_eq!(
        formulas,
        [
            "=#REF!B1+1",
            "=SUM(#REF!$B$1:B3,Sheet3!B1)",
            "=SUM(#REF!B:B)+COUNT(#REF!1:2)",
            "=SUM(Sheet3!B1)",
            "=#REF!+Sheet3!B1",
            "=SUM((#REF!B1,Sheet3!B1))+SUM(#REF!B1:B3 #REF!B2:C2)+@#REF!B1:B3",
        ]
    );
    for i in [1, 2, 3, 5, 6] {
        assert_eq!(value(&wb, 0, &format!("A{i}")), err(Ref), "A{i}");
    }
    assert_eq!(value(&wb, 0, "A4"), num(10.0));
    assert_eq!(shown(&wb, 1, "D1"), "=#REF!B2*2");
    assert_eq!(
        wb.names["TOTAL"].expr.to_text(Locale::en()),
        "SUM(#REF!$B$1:$B$3)"
    );
    assert_eq!(wb.names["OTHER"].expr.to_text(Locale::en()), "Sheet3!$B$1");
    assert!(
        wb.all_names().iter().all(|(_, def)| def.name != "Local"),
        "the sheet's own names go with it"
    );
    assert_eq!(calc(&mut wb, 0, "C1", "=Total"), err(Ref));
    // The series of the chart plotting the deleted sheet has nothing left
    // to plot; the other chart is untouched.
    let series = &wb.chart(0, on_data).unwrap().series[0];
    assert_eq!(series.values.to_text(), "#REF!$B$1:$B$3");
    assert_eq!(
        series.categories.as_ref().unwrap().to_text(),
        "#REF!$A$1:$A$3"
    );
    assert!(series.values.sheet_deleted());
    assert_eq!(
        wb.chart(0, on_third).unwrap().series[0].values.to_text(),
        "Sheet3!$B$1:$B$3"
    );
    // Broken references print and parse in every locale, and still move
    // when copied.
    copy_cell(&mut wb, 0, "A1", 0, 1, 4);
    wb.commit("copy");
    assert_eq!(shown(&wb, 0, "E2"), "=#REF!F2+1");
    wb.locale = Locale::de();
    assert_eq!(shown(&wb, 0, "A2"), "=SUMME(#BEZUG!$B$1:B3;Sheet3!B1)");
    wb.locale = Locale::en();

    // Undo brings everything back, redo deletes again.
    assert!(wb.undo() && wb.undo() && wb.undo());
    assert_eq!(wb.sheets.len(), 3);
    assert_eq!(shown(&wb, 0, "A1"), "=Data!B1+1");
    assert_eq!(shown(&wb, 0, "A4"), "=SUM(Data:Sheet3!B1)");
    assert_eq!(shown(&wb, 2, "D1"), "=data!B2*2");
    assert_eq!(value(&wb, 0, "A2"), num(16.0));
    assert_eq!(value(&wb, 0, "A5"), num(13.0));
    assert_eq!(
        wb.names["TOTAL"].expr.to_text(Locale::en()),
        "SUM(Data!$B$1:$B$3)"
    );
    assert_eq!(
        wb.sheets[1].names["LOCAL"].expr.to_text(Locale::en()),
        "Data!$B$3"
    );
    assert_eq!(
        wb.chart(0, on_data).unwrap().series[0].values.to_text(),
        "Data!$B$1:$B$3"
    );
    assert!(wb.redo());
    assert_eq!(shown(&wb, 0, "A1"), "=#REF!B1+1");
    assert!(
        wb.chart(0, on_data).unwrap().series[0]
            .values
            .sheet_deleted()
    );
    // A new sheet taking the name of the deleted one does not revive them.
    let new = wb.add_sheet(1);
    assert!(wb.rename_sheet(new, "Data"));
    enter(&mut wb, new, "B1", "5");
    wb.commit("same name");
    assert_eq!(value(&wb, 0, "A1"), err(Ref));
    assert!(
        wb.chart(0, on_data).unwrap().series[0]
            .values
            .sheet_deleted()
    );
    assert!(wb.undo());
    // The last sheet cannot be deleted.
    assert!(wb.delete_sheet(1));
    assert!(!wb.delete_sheet(0));
}

#[test]
fn unions() {
    let mut wb = three_sheets();
    let cases: [(&str, f64); 14] = [
        ("=SUM((A1:A3,C1:C3))", 66.0),
        ("=SUM((A1:A3,C1:C3),(A1,A1),1)", 69.0),
        ("=COUNT((A1:A3,C1:C3,E1:E9))", 6.0),
        ("=COUNTA((A1:A3,C1,E1:E2))", 4.0),
        ("=AVERAGE((A1,C1))", 5.5),
        ("=MAX((Sheet3!A1:A3,Sheet3!C3))", 90.0),
        ("=SMALL((A1:A3,C1:C3),4)", 10.0),
        ("=LARGE((A1:A3,C1:C3),1)", 30.0),
        ("=RANK(2,(A1:A3,C1:C3),1)", 2.0),
        ("=AREAS((A1:A3,C1:C3,A1))", 3.0),
        ("=AREAS(A1:C3)*10+AREAS((A1))", 11.0),
        ("=INDEX((A1:A3,C1:C3),2,1,2)", 20.0),
        ("=INDEX((A1:A3,C1:C3),3,1)", 3.0),
        (
            "=SUM(INDEX((A1:A3,C1:C3),0,1,2))+SUBTOTAL(9,(A1:A3,C1:C3))",
            126.0,
        ),
    ];
    for (formula, expected) in cases {
        assert_eq!(calc(&mut wb, 0, "G1", formula), num(expected), "{formula}");
    }
    assert_eq!(
        calc(&mut wb, 0, "G1", "=INDEX((A1:A3,C1:C3),1,1,3)"),
        err(Ref)
    );
    assert_eq!(calc(&mut wb, 0, "G1", "=ISREF((A1,C1))"), Value::Bool(true));
    // A union has no value of its own.
    for formula in [
        "=(A1:A3,C1:C3)",
        "=(A1,C1)+1",
        "=ABS((A1,C1))",
        "=SUMIF((A1:A3,C1:C3),\">1\")",
    ] {
        assert_eq!(
            calc(&mut wb, 0, "G1", formula),
            err(ValueError),
            "{formula}"
        );
    }
    // Its areas lie on one sheet.
    for formula in [
        "=SUM((A1:A3,Sheet2!A1))",
        "=AREAS((Sheet2!A1,Sheet3!A1))",
        "=SUM((Sheet1:Sheet2!A1,A2))",
    ] {
        assert_eq!(
            calc(&mut wb, 0, "G1", formula),
            err(ValueError),
            "{formula}"
        );
    }
    // One area between parentheses is just that area.
    assert_eq!(calc(&mut wb, 0, "G1", "=SUM((A1:A3))+(A2)"), num(8.0));
    // French: the union operator is the semicolon.
    let mut fr = Workbook::new(Locale::fr());
    enter(&mut fr, 0, "A1", "1");
    enter(&mut fr, 0, "C1", "2");
    assert_eq!(
        calc(&mut fr, 0, "E1", "=SOMME((A1;C1))+ZONES((A1;C1))"),
        num(5.0)
    );
    assert_eq!(shown(&fr, 0, "E1"), "=SOMME((A1;C1))+ZONES((A1;C1))");

    // Copying, inserting and deleting act on each area.
    enter(&mut wb, 0, "G5", "=SUM((A1:A3,$C$1:$C$3))");
    wb.commit("formula");
    copy_cell(&mut wb, 0, "G5", 0, 5, 7);
    wb.insert_rows(0, 1, 1);
    wb.commit("edit");
    assert_eq!(shown(&wb, 0, "G6"), "=SUM((A1:A4,$C$1:$C$4))");
    assert_eq!(shown(&wb, 0, "H7"), "=SUM((B3:B5,$C$1:$C$4))");
    assert_eq!(value(&wb, 0, "G6"), num(66.0));
    wb.delete_cols(0, 0, 1);
    wb.commit("delete");
    assert_eq!(shown(&wb, 0, "F6"), "=SUM((#REF!,$B$1:$B$4))");
    assert_eq!(value(&wb, 0, "F6"), err(Ref));
}

#[test]
fn intersections() {
    let mut wb = three_sheets();
    let cases: [(&str, Value); 12] = [
        ("=SUM(A1:C3 C2:D4)", num(50.0)),
        ("=A1:C3 C2", num(20.0)),
        ("=A:A 2:2", num(2.0)),
        ("=SUM(A1:C3 A2:C3 C1:C9)", num(50.0)),
        ("=ROWS(A1:C3 A2:C9)*10+COLUMNS(A1:C3 A2:C9)", num(23.0)),
        ("=SUM((A1:A3,C1:C3) A2:C2)", num(22.0)),
        ("=AREAS((A1:A3,C1:C3) A2:C2)", num(2.0)),
        ("=SUM(Sheet2!A1:A3 Sheet2!A2:B2)", num(4.0)),
        ("=SUM(A1:A3 C1:C3)", err(Null)),
        ("=A1 C1", err(Null)),
        ("=SUM(Sheet1!A1:A3 Sheet2!A1:A3)", err(Null)),
        ("=IFERROR(A1 B1,\"none\")", Value::text("none")),
    ];
    for (formula, expected) in cases {
        assert_eq!(calc(&mut wb, 0, "G1", formula), expected, "{formula}");
    }
    assert_eq!(calc(&mut wb, 0, "G1", "=ERROR.TYPE(A1 C1)"), num(1.0));
    let mut fr = Workbook::new(Locale::fr());
    enter(&mut fr, 0, "B2", "7");
    assert_eq!(calc(&mut fr, 0, "E1", "=SOMME(A1:C3 B2:D4)"), num(7.0));
    assert_eq!(calc(&mut fr, 0, "E2", "=A1:A3 C1:C3"), err(Null));
    assert_eq!(fr.display_text(0, 1, 4).text, "#NUL!");

    enter(&mut wb, 0, "G5", "=SUM(A1:C3 $C$2:D4)");
    wb.commit("formula");
    copy_cell(&mut wb, 0, "G5", 0, 5, 7);
    wb.insert_cols(0, 1, 1);
    wb.commit("edit");
    assert_eq!(shown(&wb, 0, "H5"), "=SUM(A1:D3 $D$2:E4)");
    assert_eq!(shown(&wb, 0, "I6"), "=SUM(C2:E4 $D$2:F5)");
    assert_eq!(value(&wb, 0, "H5"), num(50.0));
}

#[test]
fn implicit_intersection() {
    let mut wb = three_sheets();
    enter(&mut wb, 0, "A5", "100");
    enter(&mut wb, 0, "B5", "200");
    enter(&mut wb, 0, "C5", "300");
    wb.commit("row");
    // A column of cells: the one on the formula's row.
    assert_eq!(calc(&mut wb, 0, "F2", "=@A1:A3"), num(2.0));
    assert_eq!(calc(&mut wb, 0, "F3", "=@A:A*2"), num(6.0));
    assert_eq!(calc(&mut wb, 0, "F2", "=@Sheet2!A1:A3"), num(4.0));
    // A row of cells: the one on the formula's column.
    assert_eq!(calc(&mut wb, 0, "B7", "=@A5:C5"), num(200.0));
    assert_eq!(calc(&mut wb, 0, "C9", "=@5:5"), num(300.0));
    // Not level with the range, or a block of cells.
    assert_eq!(calc(&mut wb, 0, "F4", "=@A1:A3"), err(ValueError));
    assert_eq!(calc(&mut wb, 0, "F2", "=@A5:C5"), err(ValueError));
    assert_eq!(calc(&mut wb, 0, "B2", "=@A1:C3"), err(ValueError));
    // One cell, a constant, an array (its first value), a function.
    assert_eq!(
        calc(
            &mut wb,
            0,
            "F9",
            "=@A1+@7+@{5,6;7,8}+@SEQUENCE(3)+@SUM(A1:A3)"
        ),
        num(20.0)
    );
    assert_eq!(calc(&mut wb, 0, "F9", "=@Nowhere!A1"), err(Ref));
    // The result is one cell: nothing spills.
    enter(&mut wb, 0, "H2", "=@A1:A3");
    enter(&mut wb, 0, "I2", "=ROW(@A1:A3)+ISREF(@A1:A3)");
    wb.commit("single");
    assert!(wb.sheets[0].spills.is_empty());
    assert_eq!(value(&wb, 0, "I2"), num(3.0));
    // It moves like any reference.
    copy_cell(&mut wb, 0, "H2", 0, 2, 8);
    wb.insert_rows(0, 0, 1);
    wb.commit("edit");
    assert_eq!(shown(&wb, 0, "H3"), "=@A2:A4");
    assert_eq!(shown(&wb, 0, "I4"), "=@B3:B5");
    assert_eq!(value(&wb, 0, "H3"), num(2.0));
    let mut fr = Workbook::new(Locale::fr());
    enter(&mut fr, 0, "A2", "4");
    assert_eq!(calc(&mut fr, 0, "C2", "=@A1:A3+@SOMME(A:A)"), num(8.0));
    assert_eq!(shown(&fr, 0, "C2"), "=@A1:A3+@SOMME(A:A)");
}

#[test]
fn references_are_coloured_while_typing() {
    let mut wb = three_sheets();
    enter(&mut wb, 0, "E1", "=SEQUENCE(2,2)");
    wb.define_name(
        "Zone",
        parse_formula("Sheet2!$A$1:$A$3", Locale::en()).unwrap(),
    );
    wb.define_name("Rate", parse_formula("0.2", Locale::en()).unwrap());
    wb.commit("data");
    let src = "SUM(E1#,Sheet1:Sheet3!C1,Zone,Rate,(A1:A2,B5) $C$1:C2,Sheet2!E1#,@A:A";
    let found: Vec<(&str, String, Option<String>)> = scan_refs_in(src, &wb, 0, 0, 0)
        .into_iter()
        .map(|s| {
            (
                &src[s.range.clone()],
                s.reference.to_text(),
                s.last_sheet.map(|l| l.to_string()),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("E1#", "E1:F2".to_string(), None),
            (
                "Sheet1:Sheet3!C1",
                "Sheet1!C1".to_string(),
                Some("Sheet3".to_string())
            ),
            ("Zone", "Sheet2!A1:A3".to_string(), None),
            ("A1:A2", "A1:A2".to_string(), None),
            ("B5", "B5".to_string(), None),
            ("$C$1:C2", "$C$1:C2".to_string(), None),
            // Nothing spills from that cell: the anchor alone.
            ("Sheet2!E1#", "Sheet2!E1".to_string(), None),
            ("A:A", "A:A".to_string(), None),
        ]
    );
}
