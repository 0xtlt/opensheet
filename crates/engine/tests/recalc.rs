//! Recalculation of what depends on a change: every kind of precedent,
//! spilled arrays, volatile functions, hidden rows, circular references,
//! iterative and manual calculation, evaluation sessions.

mod common;

use common::{add_table, at, enter, err, num, value, workbook};
use opensheet_engine::eval::Calc;
use opensheet_engine::parser::parse_formula;
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{CalcMode, ErrorKind, IterativeCalc, Locale, Rect, Value, Workbook};

/// Type into cells and commit, as one edit.
fn edit(wb: &mut Workbook, sheet: usize, cells: &[(&str, &str)]) {
    for (name, text) in cells {
        enter(wb, sheet, name, text);
    }
    wb.commit("edit");
}

/// A workbook calculated once, so that what follows is incremental.
fn sheets(n: usize) -> Workbook {
    opensheet_engine::deps::set_audit(true);
    let mut wb = workbook(n);
    wb.recalc();
    wb
}

#[test]
fn an_edit_reaches_every_kind_of_reader() {
    let mut wb = sheets(3);
    edit(&mut wb, 1, &[("A1", "5")]);
    edit(&mut wb, 2, &[("A1", "7")]);
    let rate = wb.parse_name_formula("Sheet1!$A$1*10", 0, 0, 0).unwrap();
    wb.define_name("Rate", rate);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("B1", "=A1+1"),
            ("B2", "=SUM(A1:A2)"),
            ("B3", "=SUM(A:A)"),
            ("B4", "=SUM(1:1)"),
            ("B5", "=Sheet2!A1+A1"),
            ("B6", "=SUM(Sheet1:Sheet3!A1)"),
            ("B7", "=SUM((A1:A2,A2)) + SUM(A1:A2 A2:A3)"),
            ("B8", "=Rate"),
            ("B9", "=B1*2"),
            ("C1", "=INDEX(A1:A2,1)+CHOOSE(1,A1,A2)+IF(A2>0,A1,0)"),
            ("C2", "=XLOOKUP(2,A1:A2,A1:A2)+VLOOKUP(2,A1:B2,2,FALSE)"),
        ],
    );
    let (full, partial) = wb.calc_runs();
    edit(&mut wb, 0, &[("A1", "10")]);
    // Calculated from what changed, not from scratch.
    assert_eq!(wb.calc_runs(), (full, partial + 1));
    let expect = [
        ("B1", 11.0),
        ("B2", 12.0),
        ("B3", 12.0),
        ("B4", 51.0),
        ("B5", 15.0),
        ("B6", 22.0),
        ("B7", 16.0),
        ("B8", 100.0),
        ("B9", 22.0),
        ("C1", 30.0),
        ("C2", 14.0),
    ];
    for (cell, n) in expect {
        assert_eq!(value(&wb, 0, cell), num(n), "{cell}");
    }
    // Another sheet, and the far end of a 3-D reference.
    edit(&mut wb, 1, &[("A1", "6")]);
    assert_eq!(value(&wb, 0, "B5"), num(16.0));
    edit(&mut wb, 2, &[("A1", "8")]);
    assert_eq!(value(&wb, 0, "B6"), num(24.0));
    // Undo and redo are edits like any other.
    let (full, partial) = wb.calc_runs();
    assert!(wb.undo() && wb.undo() && wb.undo());
    assert_eq!(value(&wb, 0, "B6"), num(13.0));
    assert_eq!(value(&wb, 0, "B9"), num(4.0));
    assert!(wb.redo());
    assert_eq!(value(&wb, 0, "B9"), num(22.0));
    assert_eq!(wb.calc_runs(), (full, partial + 4));
}

#[test]
fn pastes_fills_and_clears_are_incremental() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("B1", "=A1*10"),
            ("D1", "=SUM(B1:B3)"),
        ],
    );
    let (full, partial) = wb.calc_runs();
    wb.fill_down(0, Rect::parse("B1:B3").unwrap());
    wb.commit("fill");
    assert_eq!(value(&wb, 0, "D1"), num(60.0));
    let clip = wb.copy(0, Rect::parse("A1:B3").unwrap());
    wb.paste(
        0,
        Rect::parse("A5").unwrap(),
        &clip,
        opensheet_engine::ops::PasteMode::All,
    );
    wb.commit("paste");
    assert_eq!(value(&wb, 0, "B7"), num(30.0));
    wb.clear(0, Rect::parse("A2").unwrap(), ClearMode::Contents);
    wb.commit("clear");
    assert_eq!(value(&wb, 0, "D1"), num(40.0));
    wb.sort_rows(0, Rect::parse("A5:A7").unwrap(), 0, false, false);
    wb.commit("sort");
    assert_eq!(value(&wb, 0, "B5"), num(30.0));
    assert!(wb.undo() && wb.undo() && wb.undo() && wb.undo());
    assert_eq!(value(&wb, 0, "D1"), num(10.0));
    assert_eq!(wb.calc_runs(), (full, partial + 8));
}

#[test]
fn spilled_arrays() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "3"),
            ("B1", "=SEQUENCE(A1)"),
            ("C1", "=B3*2"),
            ("C2", "=SUM(B1#)"),
            ("C3", "=SUM(B:B)"),
            ("C4", "=B5"),
        ],
    );
    assert_eq!(value(&wb, 0, "B3"), num(3.0));
    assert_eq!(value(&wb, 0, "C1"), num(6.0));
    assert_eq!(value(&wb, 0, "C2"), num(6.0));
    let (full, _) = wb.calc_runs();
    // The array grows: a cell that read as blank now shows a value.
    edit(&mut wb, 0, &[("A1", "5")]);
    assert_eq!(value(&wb, 0, "C2"), num(15.0));
    assert_eq!(value(&wb, 0, "C3"), num(15.0));
    assert_eq!(value(&wb, 0, "C4"), num(5.0));
    // A value typed in its way blocks it; what read the array follows.
    edit(&mut wb, 0, &[("B3", "x")]);
    assert_eq!(value(&wb, 0, "B1"), err(ErrorKind::Spill));
    assert_eq!(value(&wb, 0, "B2"), Value::Empty);
    assert_eq!(value(&wb, 0, "C2"), err(ErrorKind::Ref));
    assert_eq!(value(&wb, 0, "C4"), num(0.0));
    assert!(wb.sheets[0].spills.is_empty());
    // Out of the way again.
    wb.clear(0, Rect::parse("B3").unwrap(), ClearMode::Contents);
    wb.commit("clear");
    assert_eq!(value(&wb, 0, "B1"), num(1.0));
    assert_eq!(value(&wb, 0, "C1"), num(6.0));
    assert_eq!(value(&wb, 0, "C2"), num(15.0));
    // It shrinks: cells go blank.
    edit(&mut wb, 0, &[("A1", "2")]);
    assert_eq!(value(&wb, 0, "C1"), num(0.0));
    assert_eq!(value(&wb, 0, "C4"), num(0.0));
    // Its formula goes away.
    edit(&mut wb, 0, &[("B1", "9")]);
    assert_eq!(value(&wb, 0, "C3"), num(9.0));
    assert_eq!(value(&wb, 0, "C2"), err(ErrorKind::Ref));
    assert_eq!(
        wb.calc_runs().0,
        full,
        "all of it without calculating everything"
    );
}

#[test]
fn structured_references_follow_the_table() {
    let mut wb = sheets(2);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "Item"),
            ("B1", "Qty"),
            ("A2", "a"),
            ("B2", "1"),
            ("A3", "b"),
            ("B3", "2"),
            ("B4", "4"),
        ],
    );
    add_table(&mut wb, 0, "Sales", "A1:B3", false);
    edit(
        &mut wb,
        1,
        &[("A1", "=SUM(Sales[Qty])"), ("A2", "=ROWS(Sales)")],
    );
    assert_eq!(value(&wb, 1, "A1"), num(3.0));
    // A cell of the table.
    edit(&mut wb, 0, &[("B2", "10")]);
    assert_eq!(value(&wb, 1, "A1"), num(12.0));
    // The table grows, in a transaction.
    wb.snapshot();
    wb.sheets[0].tables[0].range = Rect::parse("A1:B4").unwrap();
    wb.commit("resize");
    assert_eq!(value(&wb, 1, "A1"), num(16.0));
    assert_eq!(value(&wb, 1, "A2"), num(3.0));
    assert!(wb.undo());
    assert_eq!(value(&wb, 1, "A1"), num(12.0));
    // And behind the workbook's back: the next calculation notices.
    wb.sheets[0].tables[0].range = Rect::parse("A1:B4").unwrap();
    edit(&mut wb, 1, &[("C1", "1")]);
    assert_eq!(value(&wb, 1, "A1"), num(16.0));
}

#[test]
fn defined_names_follow_their_definition() {
    let mut wb = sheets(1);
    edit(&mut wb, 0, &[("A1", "1"), ("A2", "2"), ("B1", "=Pick*2")]);
    assert_eq!(value(&wb, 0, "B1"), err(ErrorKind::Name));
    let a1 = wb.parse_name_formula("Sheet1!$A$1", 0, 0, 0).unwrap();
    wb.define_name("Pick", a1);
    wb.commit("name");
    assert_eq!(value(&wb, 0, "B1"), num(2.0));
    edit(&mut wb, 0, &[("A1", "5")]);
    assert_eq!(value(&wb, 0, "B1"), num(10.0));
    let a2 = wb.parse_name_formula("Sheet1!$A$2", 0, 0, 0).unwrap();
    wb.define_name("Pick", a2);
    wb.commit("name");
    assert_eq!(value(&wb, 0, "B1"), num(4.0));
    edit(&mut wb, 0, &[("A2", "7")]);
    assert_eq!(value(&wb, 0, "B1"), num(14.0));
}

#[test]
fn volatile_functions_are_calculated_every_time() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "=RAND()"),
            ("A2", "=A1+1"),
            ("A3", "=RANDBETWEEN(1,1000000000)"),
            ("A4", "=NOW()"),
        ],
    );
    edit(
        &mut wb,
        0,
        &[
            ("B1", "=OFFSET(C1,1,0)"),
            ("B2", "=INDIRECT(\"C\"&2)*2"),
            ("C2", "4"),
        ],
    );
    let before = (value(&wb, 0, "A1"), value(&wb, 0, "A3"));
    let (full, partial) = wb.calc_runs();
    // An edit that has nothing to do with them.
    edit(&mut wb, 0, &[("H9", "1")]);
    assert_ne!((value(&wb, 0, "A1"), value(&wb, 0, "A3")), before);
    let (Value::Number(a1), Value::Number(a2)) = (value(&wb, 0, "A1"), value(&wb, 0, "A2")) else {
        panic!("numbers");
    };
    assert_eq!(a2, a1 + 1.0, "what reads a volatile cell follows it");
    // Cells only named at calculation time.
    edit(&mut wb, 0, &[("C2", "6")]);
    assert_eq!(value(&wb, 0, "B1"), num(6.0));
    assert_eq!(value(&wb, 0, "B2"), num(12.0));
    assert_eq!(wb.calc_runs(), (full, partial + 2));
}

#[test]
fn ranges_read_past_what_is_written() {
    let mut wb = sheets(1);
    // The third argument of SUMIF takes the shape of the first; LOOKUP
    // reads a short result vector past its end.
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("B1", "10"),
            ("B2", "20"),
            ("B3", "30"),
            ("D1", "=SUMIF(A1:A3,\">1\",B1)"),
            ("D2", "=LOOKUP(3,A1:A3,B1)"),
            ("D3", "=AVERAGEIF(A1:A3,\">1\",B1:B2)"),
        ],
    );
    assert_eq!(value(&wb, 0, "D1"), num(50.0));
    assert_eq!(value(&wb, 0, "D2"), num(30.0));
    edit(&mut wb, 0, &[("B3", "40")]);
    assert_eq!(value(&wb, 0, "D1"), num(60.0));
    assert_eq!(value(&wb, 0, "D2"), num(40.0));
    assert_eq!(value(&wb, 0, "D3"), num(30.0));
}

#[test]
fn whole_columns_follow_the_end_of_the_data() {
    let mut wb = sheets(2);
    edit(&mut wb, 0, &[("A1", "1"), ("A2", "2")]);
    edit(
        &mut wb,
        1,
        &[
            ("A1", "=ROWS(Sheet1!A:A&\"\")"),
            ("A2", "=COLUMNS(Sheet1!1:1048576&\"\")"),
            ("A3", "=COUNTA(Sheet1!A:B)"),
        ],
    );
    assert_eq!(value(&wb, 1, "A1"), num(2.0));
    assert_eq!(value(&wb, 1, "A2"), num(1.0));
    // Typed further down and further right, in other columns.
    edit(&mut wb, 0, &[("C5", "x")]);
    assert_eq!(value(&wb, 1, "A1"), num(5.0));
    assert_eq!(value(&wb, 1, "A2"), num(3.0));
    assert_eq!(value(&wb, 1, "A3"), num(2.0));
    wb.clear(0, Rect::parse("C5").unwrap(), ClearMode::Contents);
    wb.commit("clear");
    assert_eq!(value(&wb, 1, "A1"), num(2.0));
    assert_eq!(value(&wb, 1, "A2"), num(1.0));
}

#[test]
fn hidden_rows_reach_subtotal_and_aggregate() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "4"),
            ("B1", "=SUBTOTAL(109,A1:A3)"),
            ("B2", "=AGGREGATE(9,5,A1:A3)"),
            ("B3", "=B1+B2"),
        ],
    );
    assert_eq!(value(&wb, 0, "B3"), num(14.0));
    let (full, _) = wb.calc_runs();
    // Hidden in a transaction.
    wb.set_rows_hidden(0, 1, 1, true);
    wb.commit("hide");
    assert_eq!(value(&wb, 0, "B1"), num(5.0));
    assert_eq!(value(&wb, 0, "B3"), num(10.0));
    assert!(wb.undo());
    assert_eq!(value(&wb, 0, "B3"), num(14.0));
    // Hidden by a filter, which says so.
    wb.sheets[0].hidden_rows.insert(2);
    wb.visibility_changed(0);
    wb.calculate_now();
    assert_eq!(value(&wb, 0, "B3"), num(6.0));
    assert_eq!(wb.calc_runs().0, full);
}

#[test]
fn circular_references_come_and_go() {
    let mut wb = sheets(1);
    edit(&mut wb, 0, &[("A1", "1"), ("A2", "=A1+1"), ("A3", "=A2+1")]);
    assert!(!wb.circular);
    edit(&mut wb, 0, &[("A1", "=A3+1")]);
    assert!(wb.circular);
    // An unrelated edit leaves the warning on.
    edit(&mut wb, 0, &[("C1", "5")]);
    assert!(wb.circular);
    edit(&mut wb, 0, &[("A1", "2")]);
    assert!(!wb.circular);
    assert_eq!(value(&wb, 0, "A3"), num(4.0));
    // A formula that reads itself.
    edit(&mut wb, 0, &[("B1", "=SUM(B1:B3)")]);
    assert!(wb.circular);
    assert!(wb.undo());
    assert!(!wb.circular);
}

#[test]
fn iterative_calculation() {
    let mut wb = sheets(1);
    assert_eq!(
        wb.iterative_calc(),
        IterativeCalc {
            enabled: false,
            max_iterations: 100,
            max_change: 0.001
        }
    );
    wb.set_iterative_calc(IterativeCalc {
        enabled: true,
        ..IterativeCalc::default()
    });
    edit(&mut wb, 0, &[("A1", "=A1+1"), ("B1", "=A1*2")]);
    // No warning; the formula was calculated a hundred times over.
    assert!(!wb.circular);
    assert_eq!(value(&wb, 0, "A1"), num(100.0));
    assert_eq!(value(&wb, 0, "B1"), num(200.0));
    // And again at each calculation, as in Excel.
    edit(&mut wb, 0, &[("C1", "1")]);
    assert_eq!(value(&wb, 0, "A1"), num(200.0));
    // Changing the limits calculates.
    wb.set_iterative_calc(IterativeCalc {
        enabled: true,
        max_iterations: 5,
        max_change: 0.001,
    });
    assert_eq!(value(&wb, 0, "A1"), num(205.0));
    edit(&mut wb, 0, &[("C1", "2")]);
    assert_eq!(value(&wb, 0, "A1"), num(210.0));
    assert_eq!(value(&wb, 0, "B1"), num(420.0));

    // A reference that settles: iterating stops when nothing moves by
    // the maximum change.
    let mut wb = sheets(1);
    wb.set_iterative_calc(IterativeCalc {
        enabled: true,
        max_iterations: 1000,
        max_change: 0.0001,
    });
    edit(&mut wb, 0, &[("A1", "=B1/2+1"), ("B1", "=A1")]);
    let Value::Number(a1) = value(&wb, 0, "A1") else {
        panic!("a number");
    };
    assert!((a1 - 2.0).abs() < 0.001, "{a1}");
    assert!(!wb.circular);
    // Switched off, the warning is back.
    wb.set_iterative_calc(IterativeCalc::default());
    assert!(wb.circular);
}

#[test]
fn manual_calculation() {
    let mut wb = sheets(2);
    edit(&mut wb, 0, &[("A1", "1"), ("B1", "=A1+1")]);
    edit(&mut wb, 1, &[("A1", "=Sheet1!B1*10"), ("A2", "=A1+1")]);
    assert_eq!(wb.calc_mode(), CalcMode::Automatic);
    assert!(!wb.needs_calculation());
    wb.set_calc_mode(CalcMode::Manual);
    let generation = wb.generation;
    edit(&mut wb, 0, &[("A1", "5")]);
    // Nothing follows, but the caches are told.
    assert_eq!(value(&wb, 0, "B1"), num(2.0));
    assert!(wb.needs_calculation());
    assert!(wb.generation > generation);
    // A formula is calculated when it is entered, from what is there.
    edit(&mut wb, 0, &[("C1", "=A1+B1")]);
    assert_eq!(value(&wb, 0, "C1"), num(7.0));
    // Calculate Sheet: the other sheet as it is.
    wb.calculate_sheet(1);
    assert_eq!(value(&wb, 1, "A1"), num(20.0));
    assert_eq!(value(&wb, 0, "B1"), num(2.0));
    assert!(wb.needs_calculation());
    wb.calculate_sheet(0);
    assert_eq!(value(&wb, 0, "B1"), num(6.0));
    assert_eq!(value(&wb, 0, "C1"), num(11.0));
    assert_eq!(value(&wb, 1, "A2"), num(21.0));
    // Calculate Now.
    wb.calculate_now();
    assert_eq!(value(&wb, 1, "A1"), num(60.0));
    assert_eq!(value(&wb, 1, "A2"), num(61.0));
    assert!(!wb.needs_calculation());
    // Back to automatic: what waited is calculated.
    edit(&mut wb, 0, &[("A1", "7")]);
    assert_eq!(value(&wb, 1, "A1"), num(60.0));
    wb.set_calc_mode(CalcMode::AutomaticExceptTables);
    assert_eq!(value(&wb, 1, "A2"), num(81.0));
    assert!(!wb.needs_calculation());
}

#[test]
fn the_generation_moves_with_every_change() {
    let mut wb = sheets(1);
    let mut last = wb.generation;
    let mut moved = |wb: &Workbook, what: &str| {
        assert!(wb.generation > last, "{what}");
        last = wb.generation;
    };
    edit(&mut wb, 0, &[("A1", "1")]);
    moved(&wb, "a value");
    edit(&mut wb, 0, &[("B1", "=A1")]);
    moved(&wb, "a formula");
    wb.undo();
    moved(&wb, "undo");
    wb.redo();
    moved(&wb, "redo");
    wb.set_frozen(0, 1, 0);
    wb.commit("freeze");
    moved(&wb, "a change to the sheet");
    wb.calculate_now();
    moved(&wb, "calculate now");
    wb.recalc();
    moved(&wb, "recalc");
}

#[test]
fn recalc_makes_up_for_changes_made_behind_the_workbook() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("B1", "=SUM(A1:A2)"),
            ("B2", "=SUBTOTAL(109,A1:A2)"),
        ],
    );
    let (row, col) = at("A1");
    wb.sheets[0].cells.get_mut(&(row, col)).unwrap().content =
        opensheet_engine::Content::Value(Value::Number(40.0));
    wb.sheets[0].hidden_rows.insert(1);
    wb.recalc();
    assert_eq!(value(&wb, 0, "B1"), num(42.0));
    assert_eq!(value(&wb, 0, "B2"), num(40.0));
    // And the edits that follow are incremental again.
    let (full, partial) = wb.calc_runs();
    edit(&mut wb, 0, &[("A2", "3")]);
    assert_eq!(value(&wb, 0, "B1"), num(43.0));
    assert_eq!(wb.calc_runs(), (full, partial + 1));
}

#[test]
fn long_chains_cost_no_stack() {
    const N: u32 = 150_000;
    // Each formula reads the one below: calculating the first one needs
    // all the others.
    let mut wb = Workbook::new(Locale::en());
    for r in 0..N {
        wb.set_cell_text(0, r, 0, &format!("=A{}+1", r + 2))
            .unwrap();
    }
    wb.set_cell_text(0, N, 0, "1").unwrap();
    wb.commit("fill");
    assert_eq!(wb.value_at(0, 0, 0), num(N as f64 + 1.0));
    wb.set_cell_text(0, N, 0, "5").unwrap();
    wb.commit("edit");
    assert_eq!(wb.value_at(0, 0, 0), num(N as f64 + 5.0));
    assert_eq!(wb.calc_runs(), (1, 1));
    wb.recalc();
    assert_eq!(wb.value_at(0, 0, 0), num(N as f64 + 5.0));
    assert!(!wb.circular);
}

#[test]
fn evaluation_sessions_read_the_calculated_state() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "3"),
            ("A2", "4"),
            ("B1", "=SEQUENCE(A1)"),
            ("C1", "=SUM(A1:A2)"),
        ],
    );
    let session = wb.eval_session();
    let eval = |text: &str, cell: &str| {
        let (row, col) = at(cell);
        session.eval(0, row, col, &parse_formula(text, Locale::en()).unwrap())
    };
    assert_eq!(eval("C1*2", "D1"), num(14.0));
    assert_eq!(eval("SUM(B:B)+B3", "D1"), num(9.0));
    assert_eq!(eval("SUM(B1#)", "D1"), num(6.0));
    assert_eq!(eval("COUNTIF($A:$A,A1)", "D2"), num(1.0));
    // Relative to the cell it is evaluated for.
    assert_eq!(eval("ROW()+COLUMN()", "D2"), num(6.0));
    for row in 0..500 {
        assert_eq!(
            session.eval(
                0,
                row,
                5,
                &parse_formula("$C$1+ROW()", Locale::en()).unwrap()
            ),
            num(8.0 + row as f64)
        );
    }
    // One at a time gives the same.
    let expr = parse_formula("SUM(B1#)+C1", Locale::en()).unwrap();
    assert_eq!(Calc::eval_once(&wb, 0, 0, 3, &expr), num(13.0));
    assert_eq!(wb.evaluate(0, "=C1+1").unwrap(), num(8.0));
}

#[test]
fn inserting_rows_keeps_everything_right() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "=SUM(A1:A2)"),
            ("B1", "=A3*2"),
            ("C1", "=SEQUENCE(2)"),
            ("D1", "=C2+A2"),
        ],
    );
    wb.insert_rows(0, 1, 1);
    wb.commit("insert");
    assert_eq!(value(&wb, 0, "A4"), num(3.0));
    assert_eq!(value(&wb, 0, "B1"), num(6.0));
    // The array still fills C1:C2; the reference to C2 moved down.
    assert_eq!(value(&wb, 0, "D1"), num(2.0));
    edit(&mut wb, 0, &[("A2", "10")]);
    assert_eq!(value(&wb, 0, "A4"), num(13.0));
    assert_eq!(value(&wb, 0, "B1"), num(26.0));
    assert!(wb.undo() && wb.undo());
    assert_eq!(value(&wb, 0, "A3"), num(3.0));
    assert_eq!(value(&wb, 0, "D1"), num(4.0));
}

#[test]
fn whole_columns_pair_up_over_spilled_arrays() {
    // Two arrays of different lengths, nothing typed below the first row:
    // the columns still line up cell for cell.
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "=SEQUENCE(10)"),
            ("B1", "=SEQUENCE(E1)"),
            ("E1", "8"),
            ("D1", "=SUMPRODUCT(A:A,B:B)"),
            ("D2", "=SUM(A:A*B:B)"),
            ("D3", "=SUMPRODUCT((A:A>2)*(B:B>2))"),
            ("D4", "=SUM(A:A+C:C)"),
            ("D5", "=ROWS(A:A&\"\")&\"/\"&ROWS(B:B&\"\")"),
        ],
    );
    assert_eq!(value(&wb, 0, "D1"), num(204.0));
    assert_eq!(value(&wb, 0, "D2"), num(204.0));
    assert_eq!(value(&wb, 0, "D3"), num(6.0));
    assert_eq!(value(&wb, 0, "D4"), num(55.0));
    assert_eq!(value(&wb, 0, "D5"), Value::text("10/10"));
    // The shorter one becomes the longer one.
    edit(&mut wb, 0, &[("E1", "12")]);
    assert_eq!(value(&wb, 0, "D1"), num(385.0));
    assert_eq!(value(&wb, 0, "D2"), num(385.0));
    assert_eq!(value(&wb, 0, "D5"), Value::text("12/12"));
    let expr = parse_formula("SUM(A:A*B:B)+SUM(B:B+C:C)", Locale::en()).unwrap();
    assert_eq!(wb.eval_session().eval(0, 0, 6, &expr), num(385.0 + 78.0));
}

#[test]
fn a_reference_named_at_calculation_time_sees_a_new_array() {
    let mut wb = sheets(1);
    // The reader first: it is calculated before the array is.
    edit(&mut wb, 0, &[("A1", "1"), ("D1", "=INDIRECT(\"B3\")+A1*0")]);
    edit(&mut wb, 0, &[("B1", "=SEQUENCE(A1)")]);
    assert_eq!(value(&wb, 0, "D1"), num(0.0));
    let runs = wb.calc_runs();
    edit(&mut wb, 0, &[("A1", "3")]);
    assert_eq!(value(&wb, 0, "D1"), num(3.0));
    assert_eq!(wb.calc_runs(), (runs.0, runs.1 + 1));
}

#[test]
fn manual_calculation_never_leaves_an_array_over_a_typed_cell() {
    let mut wb = sheets(1);
    edit(
        &mut wb,
        0,
        &[("A1", "=SEQUENCE(3)"), ("B1", "=SUM(A1#)"), ("B2", "=A3")],
    );
    wb.set_calc_mode(CalcMode::Manual);
    edit(&mut wb, 0, &[("A2", "x")]);
    // The array gives way at once; what read it waits to be calculated.
    assert_eq!(value(&wb, 0, "A1"), err(ErrorKind::Spill));
    assert_eq!(value(&wb, 0, "A3"), Value::Empty);
    assert_eq!(value(&wb, 0, "B1"), num(6.0));
    assert_eq!(value(&wb, 0, "B2"), num(3.0));
    wb.calculate_now();
    assert_eq!(value(&wb, 0, "B1"), err(ErrorKind::Ref));
    assert_eq!(value(&wb, 0, "B2"), num(0.0));
    // An array entered in manual mode spills; its readers wait.
    edit(&mut wb, 0, &[("D1", "=SEQUENCE(2)"), ("F1", "=D2")]);
    assert_eq!(value(&wb, 0, "D2"), num(2.0));
    assert_eq!(value(&wb, 0, "F1"), num(2.0));
    edit(&mut wb, 0, &[("D1", "=SEQUENCE(1,2)")]);
    assert_eq!(value(&wb, 0, "D2"), Value::Empty);
    assert_eq!(value(&wb, 0, "E1"), num(2.0));
    assert_eq!(value(&wb, 0, "F1"), num(2.0));
    wb.calculate_now();
    assert_eq!(value(&wb, 0, "F1"), num(0.0));
    edit(&mut wb, 0, &[("F5", "=D2+1")]);
    assert_eq!(value(&wb, 0, "F5"), num(1.0));
}

#[test]
fn formatting_calculates_nothing() {
    let mut wb = sheets(1);
    edit(&mut wb, 0, &[("A1", "=RAND()"), ("A2", "=A1")]);
    let before = value(&wb, 0, "A1");
    let (runs, generation) = (wb.calc_runs(), wb.generation);
    wb.update_style(0, Rect::parse("A1:B2").unwrap(), &|style| {
        style.font.bold = true
    });
    assert!(wb.commit("bold"));
    wb.set_col_width(0, 0, Some(120.0));
    assert!(wb.commit("width"));
    wb.set_frozen(0, 1, 0);
    assert!(wb.commit("freeze"));
    assert!(wb.undo() && wb.undo());
    // Not even the volatile functions, as in Excel; the caches are told.
    assert_eq!(value(&wb, 0, "A1"), before);
    assert_eq!(wb.calc_runs(), runs);
    assert!(wb.generation >= generation + 5);
    // F9 does.
    wb.calculate_now();
    assert_ne!(value(&wb, 0, "A1"), before);
    assert_eq!(value(&wb, 0, "A2"), value(&wb, 0, "A1"));
}

#[test]
fn many_changes_at_once() {
    const N: u32 = 3000;
    let mut wb = sheets(1);
    let fill = |wb: &mut Workbook, factor: u32| {
        for r in 0..N {
            wb.set_cell_text(0, r, 0, &r.to_string()).unwrap();
            wb.set_cell_text(0, r, 1, &format!("=A{}*{factor}", r + 1))
                .unwrap();
        }
        wb.commit("fill");
    };
    fill(&mut wb, 1);
    edit(&mut wb, 0, &[("D1", "=SUM(B:B)")]);
    let sum = (N * (N - 1) / 2) as f64;
    assert_eq!(value(&wb, 0, "D1"), num(sum));
    // Every formula replaced, several times over: the graph is rebuilt on
    // the way, and nothing shows.
    for factor in 2..6 {
        fill(&mut wb, factor);
        assert_eq!(value(&wb, 0, "D1"), num(sum * factor as f64));
    }
    edit(&mut wb, 0, &[("A1", "10")]);
    assert_eq!(value(&wb, 0, "D1"), num(sum * 5.0 + 50.0));
    // A row inserted at the top of a sheet of numbers moves every cell:
    // past a number of changes everything is calculated again. The edits
    // that follow are incremental as before.
    for r in 0..N {
        for col in 4..7 {
            wb.set_value(0, r, col, num(r as f64));
        }
    }
    wb.commit("numbers");
    let runs = wb.calc_runs();
    wb.insert_rows(0, 0, 1);
    wb.commit("insert");
    assert_eq!(wb.calc_runs(), (runs.0 + 1, runs.1));
    assert_eq!(value(&wb, 0, "D2"), num(sum * 5.0 + 50.0));
    assert_eq!(value(&wb, 0, "B3001"), num(2999.0 * 5.0));
    edit(&mut wb, 0, &[("A2", "0")]);
    assert_eq!(value(&wb, 0, "D2"), num(sum * 5.0));
    assert!(wb.undo() && wb.undo());
    assert_eq!(value(&wb, 0, "D1"), num(sum * 5.0 + 50.0));
    assert_eq!(wb.calc_runs().1, runs.1 + 2);
}

#[test]
fn the_cells_of_a_circular_reference_show_zero_and_are_listed() {
    let mut wb = sheets(2);
    edit(
        &mut wb,
        0,
        &[
            ("A1", "5"),
            ("B1", "=C1+A1"),
            ("C1", "=D1*2"),
            ("D1", "=B1+1"),
            ("E1", "=D1+10"),
            ("F1", "=A1*2"),
        ],
    );
    assert!(wb.circular);
    // The loop is B1 -> C1 -> D1 -> B1; E1 only reads it.
    assert_eq!(wb.circular_cells(), [(0, 0, 1), (0, 0, 2), (0, 0, 3)]);
    for cell in ["B1", "C1", "D1"] {
        assert_eq!(value(&wb, 0, cell), num(0.0), "{cell}");
    }
    assert_eq!(value(&wb, 0, "E1"), num(10.0));
    assert_eq!(value(&wb, 0, "F1"), num(10.0));
    // A second loop, on another sheet, and a cell reading itself.
    edit(&mut wb, 1, &[("A1", "=A2"), ("A2", "=A1"), ("C3", "=C3+1")]);
    assert_eq!(
        wb.circular_cells(),
        [
            (0, 0, 1),
            (0, 0, 2),
            (0, 0, 3),
            (1, 0, 0),
            (1, 1, 0),
            (1, 2, 2)
        ]
    );
    // One loop opened: the others stay.
    edit(&mut wb, 0, &[("C1", "7")]);
    assert_eq!(wb.circular_cells(), [(1, 0, 0), (1, 1, 0), (1, 2, 2)]);
    assert_eq!(value(&wb, 0, "E1"), num(23.0));
    wb.clear(1, Rect::parse("A1:C3").unwrap(), ClearMode::Contents);
    wb.commit("clear");
    assert!(!wb.circular);
    assert!(wb.circular_cells().is_empty());
    // Iterative calculation knows no such error.
    wb.set_iterative_calc(IterativeCalc {
        enabled: true,
        max_iterations: 3,
        max_change: 0.001,
    });
    edit(&mut wb, 0, &[("H1", "=H1+1")]);
    assert!(!wb.circular);
    assert!(wb.circular_cells().is_empty());
    assert_eq!(value(&wb, 0, "H1"), num(3.0));
}

#[test]
fn the_areas_an_expression_evaluates_to() {
    let mut wb = sheets(2);
    edit(
        &mut wb,
        0,
        &[("A1", "2"), ("A2", "3"), ("B1", "=SEQUENCE(3)")],
    );
    let grow = wb
        .parse_name_formula("OFFSET(Sheet1!$A$1,0,0,Sheet1!$A$1,1)", 0, 0, 0)
        .unwrap();
    wb.define_name("Grow", grow);
    wb.commit("name");
    let areas = |wb: &Workbook, text: &str| {
        let expr = parse_formula(text, Locale::en()).unwrap();
        wb.evaluate_areas(1, 0, 0, &expr).map(|areas| {
            let names: Vec<String> = areas
                .iter()
                .map(|a| format!("{}!{}", wb.sheets[a.sheet].name, a.rect.name()))
                .collect();
            names.join(" ")
        })
    };
    assert_eq!(areas(&wb, "Grow").as_deref(), Some("Sheet1!A1:A2"));
    assert_eq!(
        areas(&wb, "INDEX(Sheet1!A:A,2)").as_deref(),
        Some("Sheet1!A2")
    );
    assert_eq!(areas(&wb, "Sheet1!B1#").as_deref(), Some("Sheet1!B1:B3"));
    assert_eq!(
        areas(&wb, "(Sheet1!A1,Sheet1!B2:C3)").as_deref(),
        Some("Sheet1!A1 Sheet1!B2:C3")
    );
    assert_eq!(
        areas(&wb, "Sheet1:Sheet2!D4").as_deref(),
        Some("Sheet1!D4 Sheet2!D4")
    );
    assert_eq!(areas(&wb, "1+1"), None);
    // The name grows with the cell it is defined on; Go To follows it.
    edit(&mut wb, 0, &[("A1", "4")]);
    assert_eq!(areas(&wb, "Grow").as_deref(), Some("Sheet1!A1:A4"));
    let found = wb.reference_areas(1, 0, 0, "Grow").unwrap();
    assert_eq!(
        (found[0].sheet, found[0].rect.name()),
        (0, "A1:A4".to_string())
    );
}
