//! End-to-end engine tests.

use crate::locale::Locale;
use crate::refs::Rect;
use crate::value::{ErrorKind, Value};
use crate::workbook::Workbook;

/// Build a workbook (English locale) from (cell, input) pairs on sheet 0.
pub fn wb(cells: &[(&str, &str)]) -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (name, text) in cells {
        let (r, c) = crate::refs::parse_cell_name(name).unwrap();
        wb.set_cell_text(0, r, c, text).unwrap();
    }
    wb.commit("setup");
    wb
}

/// Value of a cell by name on sheet 0.
pub fn val(wb: &Workbook, name: &str) -> Value {
    let (r, c) = crate::refs::parse_cell_name(name).unwrap();
    wb.value_at(0, r, c)
}

/// Evaluate a formula (English syntax, no leading `=`) on an empty workbook.
pub fn eval(formula: &str) -> Value {
    let w = wb(&[("Z100", &format!("={formula}"))]);
    val(&w, "Z100")
}

pub fn num(formula: &str) -> f64 {
    match eval(formula) {
        Value::Number(n) => n,
        other => panic!("{formula} gave {other:?}, expected a number"),
    }
}

#[test]
fn arithmetic() {
    assert_eq!(num("1+2*3"), 7.0);
    assert_eq!(num("(1+2)*3"), 9.0);
    assert_eq!(num("2^3^2"), 64.0);
    assert_eq!(num("-2^2"), 4.0);
    assert_eq!(num("10%"), 0.1);
    assert_eq!(num("\"3\"+1"), 4.0);
    assert_eq!(eval("1/0"), Value::Error(ErrorKind::Div0));
    assert_eq!(eval("\"a\"+1"), Value::Error(ErrorKind::Value));
    assert_eq!(eval("\"a\"&1&TRUE"), Value::text("a1TRUE"));
    assert_eq!(eval("1=1"), Value::Bool(true));
    assert_eq!(eval("\"A\"=\"a\""), Value::Bool(true));
    assert_eq!(eval("1<\"a\""), Value::Bool(true));
    assert_eq!(eval("NOSUCHFN(1)"), Value::Error(ErrorKind::Name));
    assert_eq!(num("0.1+0.2-0.3"), 0.0);
}

#[test]
fn references_and_recalc() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "=A1+A2"),
        ("B1", "=SUM(A1:A3)"),
    ]);
    assert_eq!(val(&w, "A3"), Value::Number(3.0));
    assert_eq!(val(&w, "B1"), Value::Number(6.0));
    w.set_cell_text(0, 0, 0, "10").unwrap();
    w.commit("edit");
    assert_eq!(val(&w, "B1"), Value::Number(24.0));
    assert!(w.undo());
    assert_eq!(val(&w, "B1"), Value::Number(6.0));
    assert!(w.redo());
    assert_eq!(val(&w, "B1"), Value::Number(24.0));
}

#[test]
fn forward_references_and_cycles() {
    let w = wb(&[("A1", "=A2+1"), ("A2", "=A3+1"), ("A3", "5")]);
    assert_eq!(val(&w, "A1"), Value::Number(7.0));
    let w = wb(&[("A1", "=B1"), ("B1", "=A1")]);
    assert!(w.circular);
}

#[test]
fn deep_chain() {
    // A forward-pointing chain forces deep recursion.
    let mut w = Workbook::new(Locale::en());
    let n = 20_000u32;
    for r in 0..n {
        w.set_cell_text(0, r, 0, &format!("=A{}+1", r + 2)).unwrap();
    }
    w.set_cell_text(0, n, 0, "0").unwrap();
    w.commit("chain");
    assert_eq!(w.value_at(0, 0, 0), Value::Number(n as f64));
}

#[test]
fn spills() {
    let w = wb(&[
        ("A1", "={1,2;3,4}"),
        ("D1", "=A2+B2"),
        ("E1", "=SUM(A1:B2)"),
    ]);
    assert_eq!(val(&w, "A1"), Value::Number(1.0));
    assert_eq!(val(&w, "B2"), Value::Number(4.0));
    assert_eq!(val(&w, "D1"), Value::Number(7.0));
    assert_eq!(val(&w, "E1"), Value::Number(10.0));
    // Blocked spill.
    let w = wb(&[("A1", "={1,2;3,4}"), ("B2", "x")]);
    assert_eq!(val(&w, "A1"), Value::Error(ErrorKind::Spill));
    // Array arithmetic.
    let w = wb(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("C1", "=A1:A3*10")]);
    assert_eq!(val(&w, "C3"), Value::Number(30.0));
    // Scalar function lifted over a range.
    let w = wb(&[("A1", "-1"), ("A2", "2"), ("C1", "=ABS(A1:A2)")]);
    assert_eq!(val(&w, "C1"), Value::Number(1.0));
    assert_eq!(val(&w, "C2"), Value::Number(2.0));
}

#[test]
fn if_is_lazy() {
    assert_eq!(num("IF(TRUE,1,1/0)"), 1.0);
    assert_eq!(eval("IF(FALSE,1)"), Value::Bool(false));
}

#[test]
fn copy_paste_shifts_references() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "=A1*2"),
        ("C1", "=$A$1+A1"),
    ]);
    let clip = w.copy(0, Rect::parse("B1:C1").unwrap());
    w.paste(
        0,
        Rect::parse("B2").unwrap(),
        &clip,
        crate::ops::PasteMode::All,
    );
    w.commit("paste");
    assert_eq!(w.formula_text(0, 1, 1).unwrap(), "=A2*2");
    assert_eq!(w.formula_text(0, 1, 2).unwrap(), "=$A$1+A2");
    assert_eq!(val(&w, "C2"), Value::Number(3.0));
}

#[test]
fn insert_delete_rows() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "=SUM(A1:A2)"),
        ("B1", "=A2"),
    ]);
    w.insert_rows(0, 1, 1);
    w.commit("insert");
    assert_eq!(w.formula_text(0, 3, 0).unwrap(), "=SUM(A1:A3)");
    assert_eq!(w.formula_text(0, 0, 1).unwrap(), "=A3");
    assert_eq!(w.value_at(0, 3, 0), Value::Number(3.0));
    w.delete_rows(0, 2, 1);
    w.commit("delete");
    assert_eq!(w.formula_text(0, 0, 1).unwrap(), "=#REF!");
    assert_eq!(w.formula_text(0, 2, 0).unwrap(), "=SUM(A1:A2)");
    w.undo();
    w.undo();
    assert_eq!(w.formula_text(0, 2, 0).unwrap(), "=SUM(A1:A2)");
    assert_eq!(val(&w, "A3"), Value::Number(3.0));
}

#[test]
fn autofill_series() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "3"),
        ("B1", "Item 1"),
        ("C1", "=A1*2"),
        ("D1", "Monday"),
    ]);
    w.autofill(
        0,
        Rect::parse("A1:A2").unwrap(),
        Rect::parse("A1:A5").unwrap(),
    );
    w.autofill(0, Rect::parse("B1").unwrap(), Rect::parse("B1:B3").unwrap());
    w.autofill(0, Rect::parse("C1").unwrap(), Rect::parse("C1:C3").unwrap());
    w.autofill(0, Rect::parse("D1").unwrap(), Rect::parse("D1:D3").unwrap());
    w.commit("fill");
    assert_eq!(val(&w, "A5"), Value::Number(9.0));
    assert_eq!(val(&w, "B3"), Value::text("Item 3"));
    assert_eq!(w.formula_text(0, 2, 2).unwrap(), "=A3*2");
    assert_eq!(val(&w, "D3"), Value::text("Wednesday"));
}

#[test]
fn sort() {
    let mut w = wb(&[
        ("A1", "n"),
        ("A2", "3"),
        ("A3", "1"),
        ("A4", "2"),
        ("B2", "c"),
        ("B3", "a"),
        ("B4", "b"),
    ]);
    w.sort_rows(0, Rect::parse("A1:B4").unwrap(), 0, true, true);
    w.commit("sort");
    assert_eq!(val(&w, "A2"), Value::Number(1.0));
    assert_eq!(val(&w, "B2"), Value::text("a"));
    assert_eq!(val(&w, "B4"), Value::text("c"));
}

#[test]
fn french_locale() {
    let mut w = Workbook::new(Locale::fr());
    w.set_cell_text(0, 0, 0, "1,5").unwrap();
    w.set_cell_text(0, 1, 0, "=SOMME(A1;2,5)").unwrap();
    w.set_cell_text(0, 2, 0, "=SI(A2>3;\"oui\";\"non\")")
        .unwrap();
    w.commit("fr");
    assert_eq!(w.value_at(0, 1, 0), Value::Number(4.0));
    assert_eq!(w.value_at(0, 2, 0), Value::text("oui"));
    assert_eq!(w.edit_text(0, 1, 0), "=SOMME(A1;2,5)");
    assert_eq!(w.edit_text(0, 0, 0), "1,5");
    assert_eq!(w.sheets[0].name, "Feuil1");
}

#[test]
fn sheets() {
    let mut w = wb(&[("A1", "5")]);
    let i = w.add_sheet(1);
    w.set_cell_text(i, 0, 0, "=Sheet1!A1*2").unwrap();
    w.commit("sheet");
    assert_eq!(w.value_at(1, 0, 0), Value::Number(10.0));
    assert!(w.rename_sheet(0, "Data 2024"));
    w.commit("rename");
    assert_eq!(w.formula_text(1, 0, 0).unwrap(), "='Data 2024'!A1*2");
    assert_eq!(w.value_at(1, 0, 0), Value::Number(10.0));
}

fn wb_in(locale: &'static Locale, cells: &[(&str, &str)]) -> Workbook {
    let mut wb = Workbook::new(locale);
    for (name, text) in cells {
        let (r, c) = crate::refs::parse_cell_name(name).unwrap();
        wb.set_cell_text(0, r, c, text).unwrap();
    }
    wb.commit("setup");
    wb
}

#[test]
fn german_locale() {
    let w = wb_in(
        Locale::de(),
        &[
            ("A1", "1,5"),
            ("A2", "=SUMME(A1;2,5)"),
            ("A3", "=WENN(A2>3;WAHR;FALSCH)"),
            ("A4", "12.03.2025"),
            ("A5", "1.000"),
            ("A6", "=1/0"),
            ("A7", "=TEXT(1234,5;\"#.##0,00\")"),
            ("A8", "=TEXT(A4;\"TT.MM.JJJJ\")"),
            ("A9", "1.5"),
        ],
    );
    assert_eq!(val(&w, "A2"), Value::Number(4.0));
    assert_eq!(val(&w, "A3"), Value::Bool(true));
    assert_eq!(
        val(&w, "A4"),
        Value::Number(crate::datetime::date_to_serial(2025, 3, 12).unwrap())
    );
    assert_eq!(w.display_text(0, 3, 0).text, "12.03.2025");
    assert_eq!(val(&w, "A5"), Value::Number(1000.0));
    assert_eq!(w.display_text(0, 5, 0).text, "#DIV/0!");
    assert_eq!(val(&w, "A7"), Value::text("1.234,50"));
    assert_eq!(val(&w, "A8"), Value::text("12.03.2025"));
    // "1.5" is the first of May, not a number.
    assert!(matches!(val(&w, "A9"), Value::Number(n) if n > 40000.0));
    assert_eq!(w.edit_text(0, 1, 0), "=SUMME(A1;2,5)");
    assert_eq!(w.edit_text(0, 2, 0), "=WENN(A2>3;WAHR;FALSCH)");
    assert_eq!(w.sheets[0].name, "Tabelle1");
}

#[test]
fn spanish_and_italian_locales() {
    let w = wb_in(
        Locale::es(),
        &[
            ("A1", "2,5"),
            ("A2", "=SUMA(A1;1)"),
            ("A3", "=SI(A2>3;VERDADERO;FALSO)"),
            ("A4", "=\"x\"+1"),
            ("A5", "12/03/2025"),
            ("C1", "={1\\2;3\\4}"),
        ],
    );
    assert_eq!(val(&w, "A2"), Value::Number(3.5));
    assert_eq!(val(&w, "A3"), Value::Bool(true));
    assert_eq!(w.display_text(0, 3, 0).text, "#¡VALOR!");
    assert_eq!(w.display_text(0, 4, 0).text, "12/03/2025");
    assert_eq!(val(&w, "D2"), Value::Number(4.0));
    assert_eq!(w.edit_text(0, 1, 0), "=SUMA(A1;1)");
    assert_eq!(w.sheets[0].name, "Hoja1");

    let w = wb_in(
        Locale::it(),
        &[
            ("A1", "2,5"),
            ("A2", "=SOMMA(A1;1)"),
            ("A3", "=SE(A2>3;VERO;FALSO)"),
            ("A4", "=NOPE()"),
        ],
    );
    assert_eq!(val(&w, "A2"), Value::Number(3.5));
    assert_eq!(val(&w, "A3"), Value::Bool(true));
    assert_eq!(w.display_text(0, 3, 0).text, "#NOME?");
    assert_eq!(w.edit_text(0, 2, 0), "=SE(A2>3;VERO;FALSO)");
    assert_eq!(w.sheets[0].name, "Foglio1");
}

#[test]
fn formulas_survive_a_locale_change() {
    let mut w = wb_in(Locale::fr(), &[("A1", "1,5"), ("A2", "=SOMME(A1;2,5)")]);
    w.locale = Locale::en();
    w.recalc();
    assert_eq!(w.edit_text(0, 1, 0), "=SUM(A1,2.5)");
    w.locale = Locale::de();
    assert_eq!(w.edit_text(0, 1, 0), "=SUMME(A1;2,5)");
    assert_eq!(w.value_at(0, 1, 0), Value::Number(4.0));
}

#[test]
fn row_format_keeps_column_format() {
    let mut w = wb(&[("A1", "x")]);
    w.update_style(0, Rect::parse("B:B").unwrap(), &|s| s.font.bold = true);
    w.update_style(0, Rect::parse("3:3").unwrap(), &|s| {
        s.fill = Some(crate::style::Color(0xFFFF00))
    });
    w.commit("format");
    let s = w.style_at(0, 2, 1);
    assert!(s.font.bold && s.fill.is_some());
    assert!(w.style_at(0, 2, 5).fill.is_some() && !w.style_at(0, 2, 5).font.bold);
    assert!(w.style_at(0, 7, 1).font.bold && w.style_at(0, 7, 1).fill.is_none());
}

#[test]
fn redo_after_cells_were_written_then_moved() {
    // One transaction that types a value and then deletes the row above
    // it: redo must give the sheet as it was left, not the value back
    // where it was typed.
    let mut w = wb(&[("A1", "top"), ("A2", "old")]);
    w.snapshot();
    w.set_value(0, 1, 0, Value::text("new"));
    w.delete_rows(0, 0, 1);
    w.commit("typed then moved");
    assert_eq!(val(&w, "A1"), Value::text("new"));
    assert_eq!(val(&w, "A2"), Value::Empty);
    w.undo();
    assert_eq!(
        (val(&w, "A1"), val(&w, "A2")),
        (Value::text("top"), Value::text("old"))
    );
    w.redo();
    assert_eq!(
        (val(&w, "A1"), val(&w, "A2")),
        (Value::text("new"), Value::Empty)
    );
}

#[test]
fn hidden_sheets_keep_calculating_and_one_sheet_stays_visible() {
    use crate::workbook::SheetVisibility::{Hidden, VeryHidden, Visible};
    let mut w = wb(&[("A1", "2")]);
    let data = w.add_sheet(1);
    w.set_cell_text(data, 0, 0, "40").unwrap();
    w.set_cell_text(0, 0, 1, &format!("=A1+{}!A1", w.sheets[data].name))
        .unwrap();
    w.commit("setup");
    assert_eq!(val(&w, "B1"), Value::Number(42.0));

    assert!(w.set_sheet_visibility(data, Hidden));
    w.commit("hide");
    assert_eq!(
        (w.visible_sheet_count(), w.nearest_visible_sheet(data)),
        (1, 0)
    );
    // The last sheet with a tab can be neither hidden nor deleted.
    assert!(!w.set_sheet_visibility(0, Hidden) && !w.set_sheet_visibility(0, VeryHidden));
    assert!(!w.delete_sheet(0));
    // Formulas go on reading the hidden sheet, and follow its cells.
    w.set_cell_text(data, 0, 0, "98").unwrap();
    w.commit("edit");
    assert_eq!(val(&w, "B1"), Value::Number(100.0));

    assert!(w.set_sheet_visibility(data, VeryHidden));
    w.commit("very hidden");
    w.undo();
    assert_eq!(w.sheets[data].visibility, Hidden);
    w.undo();
    w.undo();
    assert_eq!(w.sheets[data].visibility, Visible);
    w.redo();
    assert_eq!(w.sheets[data].visibility, Hidden);
    assert_eq!(
        (
            w.next_visible_sheet(0, true),
            w.next_visible_sheet(1, false)
        ),
        (None, Some(0))
    );
    assert!(w.set_sheet_visibility(data, Visible) && !w.set_sheet_visibility(7, Hidden));
}
