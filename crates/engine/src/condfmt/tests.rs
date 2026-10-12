//! Tests of conditional formatting. Expected results are worked out from
//! Excel's behaviour: the formulas it stores for each rule kind, `LARGE` /
//! `AVERAGE` / `PERCENTILE.INC` for the rules over a whole range, and the
//! proportions of bars, scales and icon bands.

use super::*;
use crate::ast::Expr;
use crate::datetime::{date_to_serial, now_serial};
use crate::eval::Calc;
use crate::locale::Locale;
use crate::parser::parse_formula;
use crate::refs::{Rect, parse_cell_name};
use crate::style::{Border, BorderStyle, Color, StylePatch};
use crate::tests::wb;
use crate::value::Value;
use crate::workbook::Workbook;
use std::time::Instant;

const RED: Color = Color(0xFF0000);
const GREEN: Color = Color(0x00FF00);
const BLUE: Color = Color(0x0000FF);
const BLACK: Color = Color(0x000000);
const WHITE: Color = Color(0xFFFFFF);

fn rects(s: &str) -> Vec<Rect> {
    s.split(',').map(|r| Rect::parse(r).unwrap()).collect()
}

fn rect(s: &str) -> Rect {
    Rect::parse(s).unwrap()
}

fn fill(color: Color) -> StylePatch {
    StylePatch {
        fill: Some(Some(color)),
        ..StylePatch::default()
    }
}

/// A formula in English syntax, without `=`.
fn f(text: &str) -> Expr {
    parse_formula(text, Locale::en()).unwrap()
}

fn num(n: f64) -> Expr {
    Expr::Number(n)
}

fn text(s: &str) -> Expr {
    Expr::Text(s.into())
}

/// A workbook whose column A holds `values`, from A1 down.
fn column(values: &[&str]) -> Workbook {
    let names: Vec<String> = (1..=values.len()).map(|i| format!("A{i}")).collect();
    let cells: Vec<(&str, &str)> = names.iter().map(String::as_str).zip(values.iter().copied()).collect();
    wb(&cells)
}

/// Make `rule` over `ranges`, with a red fill, the only rule of sheet 0.
fn only(w: &mut Workbook, ranges: &str, rule: Rule) {
    w.set_cond_formats(0, vec![CondFormat::new(rects(ranges), rule).with_format(fill(RED))]);
    w.commit("rule");
}

fn at(w: &Workbook, cell: &str) -> Option<CondResult> {
    let (r, c) = parse_cell_name(cell).unwrap();
    result_at(w, 0, r, c)
}

/// The cells of `range` on sheet `sheet` that the rules change, in reading
/// order: "A1 B1 A2".
fn hits_on(w: &Workbook, sheet: usize, range: &str) -> String {
    let area = rect(range);
    let mut out = Vec::new();
    for r in area.r0..=area.r1 {
        for c in area.c0..=area.c1 {
            if result_at(w, sheet, r, c).is_some() {
                out.push(Rect::cell(r, c).name());
            }
        }
    }
    out.join(" ")
}

fn hits(w: &Workbook, range: &str) -> String {
    hits_on(w, 0, range)
}

/// What `rule` over `ranges` matches inside `range`.
fn matches(w: &mut Workbook, ranges: &str, rule: Rule, range: &str) -> String {
    only(w, ranges, rule);
    hits(w, range)
}

fn fill_at(w: &Workbook, cell: &str) -> Option<Color> {
    at(w, cell)?.patch.fill?
}

fn bar_at(w: &Workbook, cell: &str) -> Option<Bar> {
    at(w, cell)?.adornment?.bar
}

fn icon_at(w: &Workbook, cell: &str) -> Option<CellIcon> {
    at(w, cell)?.adornment?.icon
}

/// The formulas of rule `index` of sheet `sheet`, in English.
fn formulas(w: &Workbook, sheet: usize, index: usize) -> String {
    let exprs = w.sheets[sheet].cond_formats[index].rule.exprs();
    exprs
        .iter()
        .map(|e| e.to_text(Locale::en()))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The "Applies to" text of rule `index` of sheet `sheet`.
fn applies_to(w: &Workbook, sheet: usize, index: usize) -> String {
    ranges_text(&w.sheets[sheet].cond_formats[index].ranges, Locale::en())
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

// ---- cell value ------------------------------------------------------------------

#[test]
fn cell_value_rules() {
    // 1, 5, 10, text, blank, TRUE, an error, the text "5".
    let mut w = column(&["1", "5", "10", "abc", "", "TRUE", "=1/0", "'5"]);
    let all = "A1:A8";
    let mut test =
        |op: CellOp, first: Expr, second: Option<Expr>| matches(&mut w, all, Rule::cell_is(op, first, second), all);
    // Text and booleans rank above every number; a blank cell counts as
    // zero; errors never match.
    assert_eq!(test(CellOp::Greater, num(5.0), None), "A3 A4 A6 A8");
    assert_eq!(test(CellOp::Less, num(5.0), None), "A1 A5");
    assert_eq!(test(CellOp::Equal, num(5.0), None), "A2");
    assert_eq!(test(CellOp::NotEqual, num(5.0), None), "A1 A3 A4 A5 A6 A8");
    assert_eq!(test(CellOp::GreaterOrEqual, num(5.0), None), "A2 A3 A4 A6 A8");
    assert_eq!(test(CellOp::LessOrEqual, num(5.0), None), "A1 A2 A5");
    assert_eq!(test(CellOp::Between, num(5.0), Some(num(10.0))), "A2 A3");
    assert_eq!(test(CellOp::Between, num(10.0), Some(num(5.0))), "A2 A3");
    assert_eq!(test(CellOp::NotBetween, num(5.0), Some(num(10.0))), "A1 A4 A5 A6 A8");
    assert_eq!(test(CellOp::Between, num(5.0), None), "");
    // Text compares without case, and after numbers.
    assert_eq!(test(CellOp::Equal, text("ABC"), None), "A4");
    assert_eq!(test(CellOp::Greater, text("abb"), None), "A4 A6");
    assert_eq!(test(CellOp::Less, text("abc"), None), "A1 A2 A3 A5 A8");
    // The famous one: "equal to 0" also formats empty cells.
    assert_eq!(test(CellOp::Equal, num(0.0), None), "A5");
    assert_eq!(test(CellOp::Equal, text(""), None), "A5");
    assert_eq!(test(CellOp::Equal, Expr::Bool(true), None), "A6");
    // An operand that is an error matches nothing, even "not equal".
    assert_eq!(test(CellOp::NotEqual, f("1/0"), None), "");
    assert_eq!(test(CellOp::Greater, f("-5"), None), "A1 A2 A3 A4 A5 A6 A8");
}

#[test]
fn cell_value_rules_with_formula_operands() {
    let mut w = column(&["1", "5", "10", "abc", "", "TRUE", "=1/0", "'5"]);
    for (cell, input) in [("B1", "0"), ("B2", "9"), ("B3", "10"), ("C1", "5"), ("C2", "10")] {
        let (r, c) = parse_cell_name(cell).unwrap();
        w.set_cell_text(0, r, c, input).unwrap();
    }
    let other = w.add_sheet(1);
    w.rename_sheet(other, "Other");
    w.set_cell_text(other, 0, 0, "7").unwrap();
    w.commit("setup");
    let all = "A1:A8";
    // A fixed reference: one value for the whole range.
    assert_eq!(matches(&mut w, all, Rule::greater_than(f("$C$1")), all), "A3 A4 A6 A8");
    assert_eq!(matches(&mut w, all, Rule::between(f("$C$1"), f("$C$2")), all), "A2 A3");
    assert_eq!(matches(&mut w, all, Rule::equal_to(f("$C$1*2")), all), "A3");
    // A relative reference: each cell against its neighbour in column B
    // (blank from row 4 down, where text and TRUE are above "nothing").
    assert_eq!(matches(&mut w, all, Rule::greater_than(f("B1")), all), "A1 A4 A6 A8");
    assert_eq!(matches(&mut w, all, Rule::between(f("B1"), f("$C$2")), all), "A1 A3 A5");
    // Another sheet.
    assert_eq!(
        matches(&mut w, all, Rule::greater_than(f("Other!$A$1")), all),
        "A3 A4 A6 A8"
    );
    // The operands follow the cells.
    w.set_cell_text(other, 0, 0, "0").unwrap();
    w.commit("edit");
    assert_eq!(hits(&w, all), "A1 A2 A3 A4 A6 A8");
}

// ---- formulas ----------------------------------------------------------------------

#[test]
fn formula_rules() {
    let cells = [
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("A4", "4"),
        ("B1", "4"),
        ("B2", "1"),
        ("B3", "3"),
        ("B4", "0"),
        ("D1", "3"),
    ];
    let mut w = wb(&cells);
    let all = "A1:B4";
    let mut test = |formula: &str| matches(&mut w, all, Rule::Formula(f(formula)), all);
    // Mixed references: the whole row is formatted where A > B.
    assert_eq!(test("$A1>$B1"), "A2 B2 A4 B4");
    // Relative references are read from each cell in turn.
    assert_eq!(test("A1>$D$1"), "B1 A4");
    // Fixed references give one answer for every cell.
    assert_eq!(test("$D$1>2"), "A1 B1 A2 B2 A3 B3 A4 B4");
    assert_eq!(test("$D$1>5"), "");
    // ROW() and COLUMN() are those of the cell being formatted.
    assert_eq!(test("MOD(ROW(),2)=0"), "A2 B2 A4 B4");
    assert_eq!(test("COLUMN()=2"), "B1 B2 B3 B4");
    assert_eq!(test("AND(ROW()>=2,COLUMN()=1,A1<4)"), "A2 A3");
    // So are the functions that only the evaluator can place.
    assert_eq!(test("CELL(\"row\")=3"), "A3 B3");
    assert_eq!(test("INDIRECT(\"RC\",FALSE)>=4"), "B1 A4");
    // Only TRUE and non-zero numbers count.
    assert_eq!(test("2"), "A1 B1 A2 B2 A3 B3 A4 B4");
    assert_eq!(test("0"), "");
    assert_eq!(test("\"TRUE\""), "");
    assert_eq!(test("1/0"), "");
    assert_eq!(test("$Z$1"), "");
    assert_eq!(test("A1/(A1-2)>0"), "B1 A3 B3 A4");
    // Aggregates over the range.
    assert_eq!(test("A1=MAX($A$1:$B$4)"), "B1 A4");
    assert_eq!(test("COUNTIF($A$1:$B$4,A1)>1"), "A1 B1 B2 A3 B3 A4");
}

#[test]
fn formulas_are_written_for_the_first_range() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "2"),
        ("A3", "3"),
        ("A4", "4"),
        ("B1", "4"),
        ("B2", "1"),
        ("B3", "3"),
        ("B4", "0"),
    ]);
    // The anchor is B2, the top-left cell of the first range, not A1.
    only(&mut w, "B2:B4,A1:A4", Rule::Formula(f("B2>2")));
    assert_eq!(hits(&w, "A1:B4"), "A3 B3 A4");
    // The same cells listed the other way round: the anchor is A1, and
    // "B2" now means the cell one down and one to the right.
    only(&mut w, "A1:A4,B2:B4", Rule::Formula(f("B2>2")));
    assert_eq!(hits(&w, "A1:C4"), "A2");
}

#[test]
fn relative_references_wrap_around_the_sheet() {
    // "Differs from the cell above", written for A1: the cell above A1 is
    // the last one of the column.
    let mut w = column(&["a", "a", "b", "b", "c"]);
    only(&mut w, "A1:A5", Rule::Formula(f("A1<>A1048576")));
    assert_eq!(hits(&w, "A1:A5"), "A1 A3 A5");
    // Typed for the active cell A2 as "differs from A1".
    let (a1, a2) = ((0, 0), (1, 0));
    let typed = parse_operand("=A2<>A1", Locale::en(), a2, a1).unwrap();
    assert_eq!(typed.to_text(Locale::en()), "A1<>A1048576");
    assert_eq!(operand_text(&typed, Locale::en(), a1, a2, false), "=A2<>A1");
    // Columns wrap too: the cell on the left of A1 is XFD1.
    w.set_cell_text(0, 0, 1, "a").unwrap();
    w.set_cell_text(0, 0, 2, "z").unwrap();
    w.commit("row");
    only(&mut w, "A1:C1", Rule::Formula(f("A1=XFD1")));
    assert_eq!(hits(&w, "A1:C1"), "B1");
    // A rule moved to where its formula was written from reads the same
    // cells again.
    assert!(w.set_cond_format_ranges(0, 0, rects("B1:C1")));
    assert_eq!(formulas(&w, 0, 0), "B1=A1");
}

#[test]
fn formulas_with_names_and_other_sheets() {
    let mut w = wb(&[("A1", "1"), ("A2", "5"), ("A3", "9"), ("D1", "4")]);
    let other = w.add_sheet(1);
    w.rename_sheet(other, "Limits");
    w.set_cell_text(other, 1, 0, "6").unwrap();
    w.set_cell_text(other, 2, 0, "20").unwrap();
    w.define_name("Limit", f("Sheet1!$D$1"));
    w.define_name("Here", f("ROW()"));
    w.commit("setup");
    assert_eq!(matches(&mut w, "A1:A3", Rule::Formula(f("A1>Limit")), "A1:A3"), "A2 A3");
    assert_eq!(matches(&mut w, "A1:A3", Rule::Formula(f("Here=2")), "A1:A3"), "A2");
    // Relative references to another sheet move with the cell too.
    assert_eq!(
        matches(&mut w, "A1:A3", Rule::Formula(f("A1>Limits!A1")), "A1:A3"),
        "A1"
    );
    assert_eq!(
        matches(&mut w, "A1:A3", Rule::Formula(f("A1>Limits!$A$2")), "A1:A3"),
        "A3"
    );
    assert_eq!(
        matches(&mut w, "A1:A3", Rule::Formula(f("A1>Missing!$A$2")), "A1:A3"),
        ""
    );
}

#[test]
fn formula_answers_cover_whole_blocks() {
    // More cells than one block of answers in each direction, each asked
    // for in a scattered order.
    let mut w = Workbook::new(Locale::en());
    for r in 0..100 {
        for c in 0..40 {
            w.set_value(0, r, c, Value::Number((r * 40 + c) as f64));
        }
    }
    w.commit("setup");
    only(&mut w, "A1:AN100", Rule::Formula(f("MOD(A1+ROW()+COLUMN(),7)=0")));
    for step in 0..4000u32 {
        let (r, c) = ((step * 37) % 100, (step * 11) % 40);
        let expected = (r * 40 + c + r + 1 + c + 1) % 7 == 0;
        assert_eq!(result_at(&w, 0, r, c).is_some(), expected, "row {r} col {c}");
    }
    assert!(result_at(&w, 0, 100, 0).is_none());
    assert!(result_at(&w, 0, 0, 40).is_none());
}

// ---- top / bottom, average ------------------------------------------------------------

#[test]
fn top_and_bottom_items() {
    let mut w = column(&["5", "5", "5", "4", "3", "text", "", "TRUE", "=1/0"]);
    let all = "A1:A9";
    let mut test = |rule: Rule| matches(&mut w, all, rule, all);
    // Ties with the last item that qualifies qualify too: x >= LARGE(range, n).
    assert_eq!(test(Rule::top_items(1)), "A1 A2 A3");
    assert_eq!(test(Rule::top_items(3)), "A1 A2 A3");
    assert_eq!(test(Rule::top_items(4)), "A1 A2 A3 A4");
    // More items than numbers: every number, and only numbers.
    assert_eq!(test(Rule::top_items(10)), "A1 A2 A3 A4 A5");
    assert_eq!(test(Rule::bottom_items(1)), "A5");
    assert_eq!(test(Rule::bottom_items(2)), "A4 A5");
    assert_eq!(test(Rule::bottom_items(3)), "A1 A2 A3 A4 A5");
    assert_eq!(test(Rule::top_items(0)), "");
}

#[test]
fn top_and_bottom_percent() {
    let twenty: Vec<String> = (1..=20).map(|n| n.to_string()).collect();
    let mut w = column(&twenty.iter().map(String::as_str).collect::<Vec<_>>());
    let all = "A1:A20";
    // 10 % of 20 numbers is 2 of them.
    assert_eq!(matches(&mut w, all, Rule::top_percent(10), all), "A19 A20");
    assert_eq!(matches(&mut w, all, Rule::bottom_percent(10), all), "A1 A2");
    assert_eq!(matches(&mut w, all, Rule::top_percent(25), all), "A16 A17 A18 A19 A20");
    assert_eq!(matches(&mut w, all, Rule::top_percent(100), all).split(' ').count(), 20);
    // The count rounds down: 10 % of 19 is 1.
    assert_eq!(matches(&mut w, "A1:A19", Rule::top_percent(10), all), "A19");
    // But never to nothing: 10 % of 5 numbers is still the top one.
    assert_eq!(matches(&mut w, "A1:A5", Rule::top_percent(10), all), "A5");
    assert_eq!(matches(&mut w, "A1:A5", Rule::bottom_percent(10), all), "A1");
    // Several ranges are ranked together.
    assert_eq!(matches(&mut w, "A1:A3,A18:A20", Rule::top_items(2), all), "A19 A20");
    // A range without numbers.
    assert_eq!(matches(&mut w, "C1:C5", Rule::top_items(2), "C1:C5"), "");
}

#[test]
fn above_and_below_average() {
    // Mean 5, standard deviation (of the population) 2.
    let mut w = column(&["2", "4", "4", "4", "5", "5", "7", "9", "text", "", "TRUE"]);
    let all = "A1:A11";
    let mut test = |above: bool, or_equal: bool, std_dev: u32| {
        let rule = Rule::Average {
            above,
            or_equal,
            std_dev,
        };
        matches(&mut w, all, rule, all)
    };
    assert_eq!(test(true, false, 0), "A7 A8");
    assert_eq!(test(false, false, 0), "A1 A2 A3 A4");
    assert_eq!(test(true, true, 0), "A5 A6 A7 A8");
    assert_eq!(test(false, true, 0), "A1 A2 A3 A4 A5 A6");
    // One standard deviation: above 7, below 3.
    assert_eq!(test(true, false, 1), "A8");
    assert_eq!(test(true, true, 1), "A7 A8");
    assert_eq!(test(false, false, 1), "A1");
    assert_eq!(test(false, true, 2), "");
    assert_eq!(test(true, true, 2), "A8");
    assert_eq!(test(true, false, 3), "");
    // A single number is neither above nor below itself.
    assert_eq!(matches(&mut w, "A1", Rule::above_average(), all), "");
    assert_eq!(matches(&mut w, "A1", Rule::below_average(), all), "");
    assert_eq!(matches(&mut w, "A9:A11", Rule::above_average(), all), "");
}

// ---- duplicates ------------------------------------------------------------------------

#[test]
fn duplicate_and_unique_values() {
    let values = [
        "apple", "Apple", "pear", "1", "'1", "", "", "=1/0", "=1/0", "2", "TRUE", "FALSE", "=\"\"", "=\"\"",
    ];
    let mut w = column(&values);
    let all = "A1:A14";
    // Without case; text that reads as a number equals the number; errors
    // repeat like anything else; blanks and empty text never count.
    assert_eq!(matches(&mut w, all, Rule::DuplicateValues, all), "A1 A2 A4 A5 A8 A9");
    assert_eq!(matches(&mut w, all, Rule::UniqueValues, all), "A3 A10 A11 A12");
    // Values are counted over all the ranges of the rule.
    w.set_cell_text(0, 0, 2, "PEAR").unwrap();
    w.set_cell_text(0, 1, 2, "kiwi").unwrap();
    w.commit("more");
    assert_eq!(matches(&mut w, "A3,C1:C2", Rule::DuplicateValues, "A1:C3"), "C1 A3");
    assert_eq!(matches(&mut w, "A3,C1:C2", Rule::UniqueValues, "A1:C3"), "C2");
    // Ranges that overlap do not make a value its own duplicate.
    assert_eq!(matches(&mut w, "A3:A4,A3", Rule::DuplicateValues, "A1:A5"), "");
}

// ---- text --------------------------------------------------------------------------------

#[test]
fn text_rules() {
    // Text, text, text, the number 123, blank, an error, text with a star,
    // a boolean.
    let mut w = column(&["Hello World", "hello", "World", "123", "", "=1/0", "a*b", "TRUE"]);
    let all = "A1:A8";
    let mut test = |op: TextOp, text: Expr| matches(&mut w, all, Rule::Text { op, text }, all);
    // NOT(ISERROR(SEARCH(text, cell))): no case, wildcards, numbers as text.
    assert_eq!(test(TextOp::Contains, text("hello")), "A1 A2");
    assert_eq!(test(TextOp::Contains, text("WORLD")), "A1 A3");
    assert_eq!(test(TextOp::Contains, text("2")), "A4");
    assert_eq!(test(TextOp::Contains, text("h?llo")), "A1 A2");
    assert_eq!(test(TextOp::Contains, text("o*d")), "A1 A3");
    assert_eq!(test(TextOp::Contains, text("a~*b")), "A7");
    assert_eq!(test(TextOp::Contains, text("*")), "A1 A2 A3 A4 A5 A7 A8");
    assert_eq!(test(TextOp::Contains, text("~")), "");
    assert_eq!(test(TextOp::Contains, text("true")), "A8");
    assert_eq!(test(TextOp::Contains, text("")), "A1 A2 A3 A4 A5 A7 A8");
    // ISERROR(SEARCH(text, cell)): blanks and errors "do not contain".
    assert_eq!(test(TextOp::NotContains, text("hello")), "A3 A4 A5 A6 A7 A8");
    assert_eq!(test(TextOp::NotContains, text("")), "A6");
    // LEFT(cell, LEN(text)) = text: no case, no wildcards.
    assert_eq!(test(TextOp::BeginsWith, text("HEL")), "A1 A2");
    assert_eq!(test(TextOp::BeginsWith, text("12")), "A4");
    assert_eq!(test(TextOp::BeginsWith, text("a*")), "A7");
    assert_eq!(test(TextOp::BeginsWith, text("h*")), "");
    assert_eq!(test(TextOp::BeginsWith, text("hello world!")), "");
    assert_eq!(test(TextOp::BeginsWith, text("")), "A1 A2 A3 A4 A5 A7 A8");
    // RIGHT(cell, LEN(text)) = text.
    assert_eq!(test(TextOp::EndsWith, text("world")), "A1 A3");
    assert_eq!(test(TextOp::EndsWith, text("23")), "A4");
    assert_eq!(test(TextOp::EndsWith, text("ue")), "A8");
    assert_eq!(test(TextOp::EndsWith, text("hello")), "A2");
}

#[test]
fn text_rules_with_formula_operands() {
    let mut w = column(&["Hello World", "hello", "World", "123", "", "=1/0", "a*b", "TRUE"]);
    for (cell, input) in [
        ("B1", "hello"),
        ("B2", "x"),
        ("B3", "world"),
        ("C1", "wor"),
        ("C2", "12"),
        ("C3", "=1/0"),
    ] {
        let (r, c) = parse_cell_name(cell).unwrap();
        w.set_cell_text(0, r, c, input).unwrap();
    }
    w.commit("setup");
    let all = "A1:A8";
    let mut test = |op: TextOp, text: &str| matches(&mut w, all, Rule::Text { op, text: f(text) }, all);
    assert_eq!(test(TextOp::Contains, "$C$1"), "A1 A3");
    // A number is searched for as text...
    assert_eq!(test(TextOp::Contains, "$C$2"), "A4");
    // ...but "begins with 12" compares LEFT(cell, 2) with the number 12.
    assert_eq!(test(TextOp::BeginsWith, "$C$2"), "");
    assert_eq!(test(TextOp::BeginsWith, "$C$2&\"\""), "A4");
    // An error to look for is never found.
    assert_eq!(test(TextOp::Contains, "$C$3"), "");
    assert_eq!(test(TextOp::NotContains, "$C$3"), "A1 A2 A3 A4 A5 A6 A7 A8");
    // Each cell against its neighbour (blank from row 4 down: found
    // everywhere but in the error).
    assert_eq!(test(TextOp::Contains, "B1"), "A1 A3 A4 A5 A7 A8");
    assert_eq!(test(TextOp::EndsWith, "B1"), "A3 A4 A5 A7 A8");
}

// ---- blanks and errors -----------------------------------------------------------------------

#[test]
fn blanks_and_errors() {
    let mut w = column(&["", "x", "0", "=1/0", "=\"\"", "FALSE", "=NA()"]);
    w.set_value(0, 7, 0, Value::text(" "));
    w.set_value(0, 8, 0, Value::text("   "));
    w.set_value(0, 9, 0, Value::text(" a "));
    w.commit("spaces");
    let all = "A1:A10";
    // LEN(TRIM(cell))=0: spaces only is blank; an error is neither blank
    // nor "not blank".
    assert_eq!(matches(&mut w, all, Rule::Blanks, all), "A1 A5 A8 A9");
    assert_eq!(matches(&mut w, all, Rule::NoBlanks, all), "A2 A3 A6 A10");
    assert_eq!(matches(&mut w, all, Rule::Errors, all), "A4 A7");
    assert_eq!(matches(&mut w, all, Rule::NoErrors, all), "A1 A2 A3 A5 A6 A8 A9 A10");
}

/// The text, blank and error rules give what the formula Excel stores for
/// them gives when evaluated for the cell.
#[test]
fn rules_agree_with_their_stored_formulas() {
    let values = [
        "Hello World",
        "hello",
        "wor",
        "World",
        "123",
        "12",
        "",
        "=1/0",
        "a*b",
        "TRUE",
        "=\"\"",
        "1.5",
        "-3",
        "=NA()",
    ];
    let mut w = column(&values);
    w.set_value(0, values.len() as u32, 0, Value::text("  "));
    w.commit("spaces");
    let rows = values.len() as u32 + 1;
    let all = format!("A1:A{rows}");
    let mut rules = vec![Rule::Blanks, Rule::NoBlanks, Rule::Errors, Rule::NoErrors];
    for op in TextOp::ALL {
        for needle in ["hello", "WOR", "12", "o*d", "a~*b", "?", "", "true", "1.5", "-"] {
            rules.push(Rule::Text { op, text: text(needle) });
        }
    }
    for rule in rules {
        only(&mut w, &all, rule.clone());
        let stored = ooxml_formulas(&w.sheets[0].cond_formats[0]);
        assert_eq!(stored.len(), 1, "{rule:?}");
        for row in 0..rows {
            let expected = match Calc::eval_once(&w, 0, row, 0, &stored[0].shifted(row as i64, 0)).into_scalar() {
                Value::Bool(b) => b,
                _ => false,
            };
            assert_eq!(
                result_at(&w, 0, row, 0).is_some(),
                expected,
                "{rule:?} on row {}",
                row + 1
            );
        }
    }
}

// ---- dates -----------------------------------------------------------------------------------

/// A column of dates, one per day from `first` to `last` days after
/// `today`; the rule's matches are given as those day offsets.
fn days_matching(period: TimePeriod, today: f64, first: i32, last: i32) -> Vec<i32> {
    let mut w = Workbook::new(Locale::en());
    for (row, offset) in (first..=last).enumerate() {
        w.set_value(0, row as u32, 0, Value::Number(today + offset as f64));
    }
    w.commit("dates");
    w.caches.cond.set_today(Some(today));
    let rows = (last - first + 1) as u32;
    only(&mut w, &format!("A1:A{rows}"), Rule::date_occurring(period));
    (0..rows)
        .filter(|row| result_at(&w, 0, *row, 0).is_some())
        .map(|row| first + row as i32)
        .collect()
}

fn span(first: i32, last: i32) -> Vec<i32> {
    (first..=last).collect()
}

#[test]
fn dates_occurring() {
    // Wednesday 15 May 2024.
    let today = date_to_serial(2024, 5, 15).unwrap();
    let test = |period: TimePeriod| days_matching(period, today, -50, 50);
    assert_eq!(test(TimePeriod::Today), [0]);
    assert_eq!(test(TimePeriod::Yesterday), [-1]);
    assert_eq!(test(TimePeriod::Tomorrow), [1]);
    assert_eq!(test(TimePeriod::Last7Days), span(-6, 0));
    // Sunday 12 to Saturday 18.
    assert_eq!(test(TimePeriod::ThisWeek), span(-3, 3));
    assert_eq!(test(TimePeriod::LastWeek), span(-10, -4));
    assert_eq!(test(TimePeriod::NextWeek), span(4, 10));
    // 1 to 31 May, 1 to 30 April, 1 to 30 June.
    assert_eq!(test(TimePeriod::ThisMonth), span(-14, 16));
    assert_eq!(test(TimePeriod::LastMonth), span(-44, -15));
    assert_eq!(test(TimePeriod::NextMonth), span(17, 46));
}

#[test]
fn date_periods_at_the_edges_of_weeks_and_years() {
    // On a Sunday the week has just started; on a Saturday it is ending.
    let sunday = date_to_serial(2024, 5, 12).unwrap();
    assert_eq!(days_matching(TimePeriod::ThisWeek, sunday, -20, 20), span(0, 6));
    assert_eq!(days_matching(TimePeriod::LastWeek, sunday, -20, 20), span(-7, -1));
    assert_eq!(days_matching(TimePeriod::NextWeek, sunday, -20, 20), span(7, 13));
    let saturday = date_to_serial(2024, 5, 18).unwrap();
    assert_eq!(days_matching(TimePeriod::ThisWeek, saturday, -20, 20), span(-6, 0));
    assert_eq!(days_matching(TimePeriod::LastWeek, saturday, -20, 20), span(-13, -7));
    assert_eq!(days_matching(TimePeriod::NextWeek, saturday, -20, 20), span(1, 7));
    // Months wrap around the year.
    let january = date_to_serial(2024, 1, 10).unwrap();
    assert_eq!(days_matching(TimePeriod::LastMonth, january, -60, 60), span(-40, -10));
    assert_eq!(days_matching(TimePeriod::NextMonth, january, -60, 60), span(22, 50));
    let december = date_to_serial(2024, 12, 31).unwrap();
    assert_eq!(days_matching(TimePeriod::NextMonth, december, -60, 60), span(1, 31));
    assert_eq!(days_matching(TimePeriod::LastMonth, december, -60, 60), span(-60, -31));
    // The last day of a long month has a "next month" all the same.
    let end_of_january = date_to_serial(2025, 1, 31).unwrap();
    assert_eq!(
        days_matching(TimePeriod::NextMonth, end_of_january, -40, 40),
        span(1, 28)
    );
}

#[test]
fn date_rules_look_at_the_day_only() {
    let today = date_to_serial(2024, 5, 15).unwrap();
    let mut w = column(&["", "abc", "TRUE", "=1/0", "'5/15/2024", "'15 mai", "0"]);
    w.set_value(0, 7, 0, Value::Number(today + 0.99));
    w.set_value(0, 8, 0, Value::Number(today - 0.01));
    w.set_value(0, 9, 0, Value::Number(-today));
    w.commit("values");
    w.caches.cond.set_today(Some(today));
    // A time of day does not matter; text that reads as a date counts.
    assert_eq!(
        matches(&mut w, "A1:A10", Rule::date_occurring(TimePeriod::Yesterday), "A1:A10"),
        "A9"
    );
    assert_eq!(
        matches(&mut w, "A1:A10", Rule::date_occurring(TimePeriod::ThisMonth), "A1:A10"),
        "A5 A8 A9"
    );
    assert_eq!(
        matches(&mut w, "A1:A10", Rule::date_occurring(TimePeriod::Today), "A1:A10"),
        "A5 A8"
    );
    // The day changes: so do the answers, without any recalculation.
    w.caches.cond.set_today(Some(today + 1.0));
    assert_eq!(hits(&w, "A1:A10"), "");
    assert_eq!(
        matches(&mut w, "A1:A10", Rule::date_occurring(TimePeriod::Yesterday), "A1:A10"),
        "A5 A8"
    );
}

/// With the computer's own date, each period gives what the formula Excel
/// stores for it gives.
#[test]
fn date_rules_agree_with_their_stored_formulas() {
    let attempt = || -> Option<()> {
        let today = now_serial().floor();
        let mut w = Workbook::new(Locale::en());
        let offsets: Vec<i32> = (-70..=70).collect();
        for (row, offset) in offsets.iter().enumerate() {
            w.set_value(0, row as u32, 0, Value::Number(today + *offset as f64 + 0.25));
        }
        w.commit("dates");
        for period in TimePeriod::ALL {
            only(&mut w, "A1:A141", Rule::date_occurring(period));
            let stored = ooxml_formulas(&w.sheets[0].cond_formats[0]);
            assert_eq!(stored.len(), 1);
            for row in 0..offsets.len() as u32 {
                let value = Calc::eval_once(&w, 0, row, 0, &stored[0].shifted(row as i64, 0)).into_scalar();
                let got = result_at(&w, 0, row, 0).is_some();
                if now_serial().floor() != today {
                    return None;
                }
                assert_eq!(
                    got,
                    value == Value::Bool(true),
                    "{period:?}, {} days from today",
                    offsets[row as usize]
                );
            }
        }
        Some(())
    };
    // Midnight may pass while the test runs: once more, then.
    if attempt().is_none() {
        attempt().expect("midnight passes once");
    }
}

// ---- colour scales -------------------------------------------------------------------------------

fn scale(stops: &[(Cfvo, Color)]) -> Rule {
    Rule::ColorScale(ColorScale {
        stops: stops
            .iter()
            .map(|(at, color)| ColorStop {
                at: at.clone(),
                color: *color,
            })
            .collect(),
    })
}

#[test]
fn two_colour_scales() {
    let mut w = column(&["0", "25", "50", "75", "100", "text", "", "=1/0", "TRUE"]);
    only(&mut w, "A1:A9", Rule::ColorScale(ColorScale::two(BLACK, WHITE)));
    assert_eq!(fill_at(&w, "A1"), Some(BLACK));
    assert_eq!(fill_at(&w, "A2"), Some(Color(0x404040)));
    assert_eq!(fill_at(&w, "A3"), Some(Color(0x808080)));
    assert_eq!(fill_at(&w, "A4"), Some(Color(0xBFBFBF)));
    assert_eq!(fill_at(&w, "A5"), Some(WHITE));
    // Only numbers are graded.
    assert_eq!(hits(&w, "A1:A9"), "A1 A2 A3 A4 A5");
    // The format of the rule itself is not used.
    assert_eq!(at(&w, "A1").unwrap().adornment, None);

    // Fixed numbers: values beyond the ends take the end colours.
    only(
        &mut w,
        "A1:A9",
        scale(&[(Cfvo::number(20.0), BLACK), (Cfvo::number(80.0), WHITE)]),
    );
    assert_eq!(fill_at(&w, "A1"), Some(BLACK));
    assert_eq!(fill_at(&w, "A2"), Some(Color(0x151515)));
    assert_eq!(fill_at(&w, "A3"), Some(Color(0x808080)));
    assert_eq!(fill_at(&w, "A5"), Some(WHITE));

    // Percent: of the way from the lowest to the highest number.
    only(
        &mut w,
        "A1:A9",
        scale(&[(Cfvo::percent(50.0), BLACK), (Cfvo::percent(100.0), WHITE)]),
    );
    assert_eq!(fill_at(&w, "A2"), Some(BLACK));
    assert_eq!(fill_at(&w, "A3"), Some(BLACK));
    assert_eq!(fill_at(&w, "A4"), Some(Color(0x808080)));

    // A formula, here a cell.
    w.set_cell_text(0, 0, 2, "50").unwrap();
    w.commit("limit");
    only(
        &mut w,
        "A1:A9",
        scale(&[(Cfvo::Min, BLACK), (Cfvo::Formula(f("$C$1")), WHITE)]),
    );
    assert_eq!(fill_at(&w, "A2"), Some(Color(0x808080)));
    assert_eq!(fill_at(&w, "A4"), Some(WHITE));
    w.set_cell_text(0, 0, 2, "100").unwrap();
    w.commit("limit");
    assert_eq!(fill_at(&w, "A2"), Some(Color(0x404040)));
    // A threshold that is an error: no scale at all.
    w.set_cell_text(0, 0, 2, "=1/0").unwrap();
    w.commit("limit");
    assert_eq!(hits(&w, "A1:A9"), "");
}

#[test]
fn three_colour_scales() {
    let mut w = column(&["0", "25", "50", "75", "100"]);
    only(&mut w, "A1:A5", Rule::ColorScale(ScalePreset::GreenYellowRed.scale()));
    assert_eq!(fill_at(&w, "A1"), Some(Color(0xF8696B)));
    assert_eq!(fill_at(&w, "A2"), Some(Color(0xFCAA78)));
    assert_eq!(fill_at(&w, "A3"), Some(Color(0xFFEB84)));
    assert_eq!(fill_at(&w, "A4"), Some(Color(0xB1D580)));
    assert_eq!(fill_at(&w, "A5"), Some(Color(0x63BE7B)));

    // The middle colour sits on the median, wherever that is.
    let mut w = column(&["0", "1", "2", "3", "100"]);
    only(
        &mut w,
        "A1:A5",
        Rule::ColorScale(ColorScale::three(BLACK, Color(0x646464), WHITE)),
    );
    assert_eq!(fill_at(&w, "A2"), Some(Color(0x323232)));
    assert_eq!(fill_at(&w, "A3"), Some(Color(0x646464)));
    assert_eq!(fill_at(&w, "A4"), Some(Color(0x666666)));
    assert_eq!(fill_at(&w, "A5"), Some(WHITE));

    // PERCENTILE.INC interpolates between numbers: the 90th percentile of
    // 0, 1, 2, 3, 100 is 3 + 0.6 * 97 = 61.2.
    let stops = [
        (Cfvo::Min, BLACK),
        (Cfvo::percentile(90.0), Color(0x646464)),
        (Cfvo::Max, Color(0x646464)),
    ];
    only(&mut w, "A1:A5", scale(&stops));
    assert_eq!(fill_at(&w, "A4"), Some(Color(0x050505)));

    // All numbers equal: every stop is the same value.
    let mut w = column(&["7", "7", "7"]);
    only(&mut w, "A1:A3", Rule::ColorScale(ScalePreset::GreenYellowRed.scale()));
    assert_eq!(fill_at(&w, "A2"), Some(Color(0xF8696B)));
}

// ---- data bars -------------------------------------------------------------------------------------

#[test]
fn data_bars_from_the_gallery() {
    let mut w = column(&["0", "50", "100", "text", ""]);
    only(&mut w, "A1:A5", Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    let bar = bar_at(&w, "A2").unwrap();
    assert!(close(bar.start, 0.0) && close(bar.end, 0.5));
    assert_eq!(
        (bar.fill, bar.gradient, bar.border),
        (Color(0x638EC6), true, Some(Color(0x638EC6)))
    );
    assert_eq!((bar.negative, bar.axis), (false, None));
    assert!(close(bar_at(&w, "A3").unwrap().end, 1.0));
    // Zero has a bar of no length; text and blanks have none.
    assert!(close(bar_at(&w, "A1").unwrap().end, 0.0));
    assert_eq!(hits(&w, "A1:A5"), "A1 A2 A3");
    let result = at(&w, "A2").unwrap();
    assert!(result.patch.is_empty());
    assert!(!result.adornment.unwrap().hide_value);
    assert_eq!(w.display(0, 1, 0).adornment.and_then(|a| a.bar), bar_at(&w, "A2"));

    // Automatic ends start from zero: 50 is half of 100 even when it is
    // the smallest number.
    let mut w = column(&["50", "75", "100"]);
    only(&mut w, "A1:A3", Rule::DataBar(DataBar::preset(BarColor::Green, false)));
    let bar = bar_at(&w, "A1").unwrap();
    assert!(close(bar.end, 0.5));
    assert_eq!((bar.fill, bar.gradient, bar.border), (Color(0x63C384), false, None));
    assert!(close(bar_at(&w, "A2").unwrap().end, 0.75));
}

#[test]
fn data_bars_with_negative_numbers() {
    let mut w = column(&["-50", "0", "100"]);
    only(&mut w, "A1:A3", Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    // The axis sits a third of the way: 50 below zero, 100 above.
    let third = 1.0 / 3.0;
    let low = bar_at(&w, "A1").unwrap();
    assert!(close(low.start, third) && close(low.end, 0.0));
    assert_eq!((low.negative, low.fill, low.border), (true, RED, Some(RED)));
    assert!(close(low.axis.unwrap(), third));
    assert_eq!(low.axis_color, BLACK);
    let zero = bar_at(&w, "A2").unwrap();
    assert!(close(zero.start, third) && close(zero.end, third) && close(zero.axis.unwrap(), third));
    let high = bar_at(&w, "A3").unwrap();
    assert!(close(high.start, third) && close(high.end, 1.0));
    assert_eq!((high.negative, high.fill), (false, Color(0x638EC6)));

    // Solid bars of the gallery have no border, negative or not.
    only(&mut w, "A1:A3", Rule::DataBar(DataBar::preset(BarColor::Purple, false)));
    assert_eq!(bar_at(&w, "A1").unwrap().border, None);
    assert_eq!(bar_at(&w, "A1").unwrap().fill, RED);

    // Negative bars in the colours of the positive ones.
    let mut same = DataBar::preset(BarColor::Orange, true);
    same.negative_fill = None;
    same.negative_border = None;
    only(&mut w, "A1:A3", Rule::DataBar(same));
    let low = bar_at(&w, "A1").unwrap();
    assert_eq!((low.fill, low.border), (Color(0xFFB628), Some(Color(0xFFB628))));

    // The axis in the middle of the cell: each side scaled by the larger
    // of the two ends.
    let mut middle = DataBar::preset(BarColor::Blue, true);
    middle.axis = BarAxis::Midpoint;
    only(&mut w, "A1:A3", Rule::DataBar(middle));
    let low = bar_at(&w, "A1").unwrap();
    assert!(close(low.start, 0.5) && close(low.end, 0.25) && close(low.axis.unwrap(), 0.5));
    assert!(close(bar_at(&w, "A3").unwrap().end, 1.0));

    // No axis: all bars from the edge, in proportion to the distance from
    // the low end.
    let mut none = DataBar::preset(BarColor::Blue, true);
    none.axis = BarAxis::None;
    only(&mut w, "A1:A3", Rule::DataBar(none));
    let low = bar_at(&w, "A1").unwrap();
    assert!(close(low.start, 0.0) && close(low.end, 0.0) && low.negative && low.axis.is_none());
    assert!(close(bar_at(&w, "A2").unwrap().end, third));

    // Nothing positive: the axis is the far edge and bars grow back.
    let mut w = column(&["-100", "-50"]);
    only(&mut w, "A1:A2", Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    let bar = bar_at(&w, "A2").unwrap();
    assert!(close(bar.start, 1.0) && close(bar.end, 0.5) && close(bar.axis.unwrap(), 1.0));
    assert!(close(bar_at(&w, "A1").unwrap().end, 0.0));
}

#[test]
fn data_bar_settings() {
    // Excel 2007 bars: from 10 % to 90 % of the cell between the lowest
    // and the highest number, negative or not.
    let mut w = column(&["-10", "0", "10"]);
    only(&mut w, "A1:A3", Rule::DataBar(DataBar::legacy(BLUE)));
    let low = bar_at(&w, "A1").unwrap();
    assert!(close(low.start, 0.0) && close(low.end, 0.1));
    assert_eq!(
        (low.fill, low.negative, low.axis, low.gradient, low.border),
        (BLUE, true, None, true, None)
    );
    assert!(close(bar_at(&w, "A2").unwrap().end, 0.5));
    assert!(close(bar_at(&w, "A3").unwrap().end, 0.9));

    // Right to left: the same bars mirrored.
    let mut w = column(&["-50", "50", "100"]);
    let mut mirrored = DataBar::preset(BarColor::Blue, true);
    mirrored.direction = BarDirection::RightToLeft;
    only(&mut w, "A1:A3", Rule::DataBar(mirrored));
    let third = 1.0 / 3.0;
    let bar = bar_at(&w, "A2").unwrap();
    assert!(close(bar.start, 1.0 - third) && close(bar.end, third) && close(bar.axis.unwrap(), 1.0 - third));
    assert!(close(bar_at(&w, "A1").unwrap().end, 1.0));

    // Fixed ends: numbers beyond them get the shortest or longest bar.
    let mut w = column(&["50", "300", "-5", "100"]);
    let mut fixed = DataBar::preset(BarColor::Blue, false);
    fixed.min = Cfvo::number(0.0);
    fixed.max = Cfvo::number(200.0);
    fixed.show_value = false;
    only(&mut w, "A1:A4", Rule::DataBar(fixed.clone()));
    assert!(close(bar_at(&w, "A1").unwrap().end, 0.25));
    assert!(close(bar_at(&w, "A2").unwrap().end, 1.0));
    let below = bar_at(&w, "A3").unwrap();
    assert!(close(below.end, 0.0) && below.negative && below.fill == RED);
    // "Show Bar Only".
    assert!(at(&w, "A1").unwrap().adornment.unwrap().hide_value);

    // Percent and percentile ends: 10 % and 90 % of the way from -5 to 300.
    fixed.min = Cfvo::percent(10.0);
    fixed.max = Cfvo::percentile(100.0);
    only(&mut w, "A1:A4", Rule::DataBar(fixed.clone()));
    let bar = bar_at(&w, "A4").unwrap();
    assert!(close(bar.end, ((100.0 - 25.5) / (300.0 - 25.5)) as f32));
    // Ends the wrong way round: no bars.
    fixed.min = Cfvo::number(10.0);
    fixed.max = Cfvo::number(0.0);
    only(&mut w, "A1:A4", Rule::DataBar(fixed));
    assert_eq!(hits(&w, "A1:A4"), "");

    // Nothing but zeros: bars of no length. The same number everywhere
    // else: full bars, zero being the automatic low end.
    let mut w = column(&["0", "0", "7", "7"]);
    only(&mut w, "A1:A2", Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    let bar = bar_at(&w, "A1").unwrap();
    assert!(close(bar.start, 0.0) && close(bar.end, 0.0) && bar.axis.is_none());
    only(&mut w, "A3:A4", Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    assert!(close(bar_at(&w, "A3").unwrap().end, 1.0));
}

// ---- icon sets ---------------------------------------------------------------------------------------

#[test]
fn icon_sets_from_the_gallery() {
    let mut w = column(&["0", "32", "33", "66", "67", "100", "x", "", "TRUE"]);
    only(&mut w, "A1:A9", Rule::IconSet(IconRule::preset(IconSet::ThreeArrows)));
    // Bands start at 33 % and 67 % of the way from 0 to 100, inclusive.
    let indexes: Vec<Option<u8>> = (1..=9)
        .map(|r| icon_at(&w, &format!("A{r}")).map(|i| i.index))
        .collect();
    assert_eq!(
        indexes,
        [Some(0), Some(0), Some(1), Some(1), Some(2), Some(2), None, None, None]
    );
    assert_eq!(icon_at(&w, "A1").unwrap().icon(), Icon::RedDownArrow);
    assert_eq!(icon_at(&w, "A3").unwrap().icon(), Icon::YellowSideArrow);
    assert_eq!(icon_at(&w, "A6").unwrap().icon(), Icon::GreenUpArrow);
    assert_eq!(hits(&w, "A1:A9"), "A1 A2 A3 A4 A5 A6");
    let result = at(&w, "A1").unwrap();
    assert!(result.patch.is_empty() && !result.adornment.unwrap().hide_value);

    // Five icons: bands of 20 %.
    let mut w = column(&["0", "19", "20", "40", "60", "80", "100"]);
    only(&mut w, "A1:A7", Rule::IconSet(IconRule::preset(IconSet::FiveQuarters)));
    let icons: Vec<Icon> = (1..=7).map(|r| icon_at(&w, &format!("A{r}")).unwrap().icon()).collect();
    let expected = [
        Icon::WhiteCircle,
        Icon::WhiteCircle,
        Icon::CircleWithThreeWhiteQuarters,
        Icon::CircleWithTwoWhiteQuarters,
        Icon::CircleWithOneWhiteQuarter,
        Icon::BlackCircle,
        Icon::BlackCircle,
    ];
    assert_eq!(icons, expected);

    // Four icons: bands of 25 %.
    only(&mut w, "A1:A7", Rule::IconSet(IconRule::preset(IconSet::FourRating)));
    let indexes: Vec<u8> = (1..=7).map(|r| icon_at(&w, &format!("A{r}")).unwrap().index).collect();
    assert_eq!(indexes, [0, 0, 0, 1, 2, 3, 3]);
    assert_eq!(icon_at(&w, "A1").unwrap().icon(), Icon::OneBar);
}

#[test]
fn icon_set_settings() {
    let mut w = column(&["0", "32", "33", "66", "67", "100"]);
    let index = |w: &Workbook| -> Vec<u8> { (1..=6).map(|r| icon_at(w, &format!("A{r}")).unwrap().index).collect() };
    // ">" instead of ">=" for the middle band.
    let mut rule = IconRule::preset(IconSet::ThreeTrafficLights1);
    rule.thresholds[1].gte = false;
    only(&mut w, "A1:A6", Rule::IconSet(rule.clone()));
    assert_eq!(index(&w), [0, 0, 0, 1, 2, 2]);
    // Reversed order.
    rule.reverse = true;
    only(&mut w, "A1:A6", Rule::IconSet(rule.clone()));
    assert_eq!(index(&w), [2, 2, 2, 1, 0, 0]);
    assert_eq!(icon_at(&w, "A1").unwrap().icon(), Icon::GreenCircle);
    // Fixed numbers, a percentile and a formula as thresholds.
    let mut rule = IconRule::preset(IconSet::ThreeFlags);
    rule.thresholds[1].value = Cfvo::number(32.0);
    rule.thresholds[2].value = Cfvo::Formula(f("$A$5+1"));
    only(&mut w, "A1:A6", Rule::IconSet(rule.clone()));
    assert_eq!(index(&w), [0, 1, 1, 1, 1, 2]);
    // The median of the six numbers is (33 + 66) / 2 = 49.5.
    rule.thresholds[1].value = Cfvo::percentile(50.0);
    rule.thresholds[2].value = Cfvo::Max;
    only(&mut w, "A1:A6", Rule::IconSet(rule.clone()));
    assert_eq!(index(&w), [0, 0, 0, 1, 1, 2]);

    // Icons picked one by one, one band without any, and "Show Icon Only".
    let mut rule = IconRule::preset(IconSet::ThreeArrows);
    rule.custom = Some(vec![
        Some((IconSet::ThreeFlags, 2)),
        None,
        Some((IconSet::ThreeStars, 0)),
    ]);
    rule.show_value = false;
    only(&mut w, "A1:A6", Rule::IconSet(rule));
    let low = icon_at(&w, "A1").unwrap();
    assert_eq!(
        (low.set, low.index, low.icon()),
        (IconSet::ThreeFlags, 2, Icon::GreenFlag)
    );
    assert_eq!(icon_at(&w, "A6").unwrap().icon(), Icon::SilverStar);
    let middle = at(&w, "A3").unwrap().adornment.unwrap();
    assert_eq!((middle.icon, middle.bar, middle.hide_value), (None, None, true));
}

#[test]
fn ranges_without_numbers_are_left_alone() {
    // Text, a blank, a boolean and an error; then nothing at all.
    let mut w = column(&["a", "", "TRUE", "=1/0"]);
    for ranges in ["A1:A4", "C1:C9", "A2,C:C"] {
        for rule in [
            Rule::ColorScale(ScalePreset::GreenYellowRed.scale()),
            Rule::DataBar(DataBar::preset(BarColor::Blue, true)),
            Rule::DataBar(DataBar::legacy(BLUE)),
            Rule::IconSet(IconRule::preset(IconSet::FiveBoxes)),
            Rule::top_items(3),
            Rule::bottom_percent(50),
            Rule::above_average(),
            Rule::Average {
                above: false,
                or_equal: true,
                std_dev: 1,
            },
        ] {
            assert_eq!(
                matches(&mut w, ranges, rule.clone(), "A1:C9"),
                "",
                "{rule:?} on {ranges}"
            );
        }
    }
    assert_eq!(matches(&mut w, "C1:C9", Rule::DuplicateValues, "A1:C9"), "");
    assert_eq!(matches(&mut w, "C1:C9", Rule::UniqueValues, "A1:C9"), "");
    // Rules that do match empty cells.
    assert_eq!(matches(&mut w, "C1:C2", Rule::Blanks, "A1:C9"), "C1 C2");
    assert_eq!(matches(&mut w, "C1:C2", Rule::less_than(num(1.0)), "A1:C9"), "C1 C2");
    assert_eq!(matches(&mut w, "C1:C2", Rule::Formula(f("C1=0")), "A1:C9"), "C1 C2");
}

// ---- several rules on one cell -------------------------------------------------------------------------

#[test]
fn rules_combine_by_priority() {
    let mut w = column(&["10", "3", "text"]);
    let bold_red = StylePatch {
        bold: Some(true),
        italic: Some(false),
        font_color: Some(Some(RED)),
        ..StylePatch::default()
    };
    let italic_blue_on_green = StylePatch {
        italic: Some(true),
        font_color: Some(Some(BLUE)),
        fill: Some(Some(GREEN)),
        ..StylePatch::default()
    };
    let over_five = CondFormat::new(rects("A1:A3"), Rule::greater_than(num(5.0))).with_format(bold_red.clone());
    let positive =
        CondFormat::new(rects("A1:A3"), Rule::greater_than(num(0.0))).with_format(italic_blue_on_green.clone());
    w.set_cond_formats(0, vec![over_five.clone(), positive.clone()]);
    w.commit("rules");
    // Each property comes from the first rule that sets it.
    let both = at(&w, "A1").unwrap().patch;
    assert_eq!((both.bold, both.italic), (Some(true), Some(false)));
    assert_eq!((both.font_color, both.fill), (Some(Some(RED)), Some(Some(GREEN))));
    // 3 only passes the second rule; text is above both numbers.
    assert_eq!(at(&w, "A2").unwrap().patch, italic_blue_on_green);
    assert_eq!(at(&w, "A3").unwrap().patch.fill, Some(Some(GREEN)));

    // The other way round, the second rule's italic and colour win.
    w.reorder_cond_format(0, 1, 0);
    w.commit("reorder");
    let both = at(&w, "A1").unwrap().patch;
    assert_eq!(
        (both.bold, both.italic, both.font_color),
        (Some(true), Some(true), Some(Some(BLUE)))
    );

    // "Stop If True" hides the rules below, but only where the rule holds.
    w.set_cond_formats(0, vec![over_five.with_stop_if_true(true), positive]);
    w.commit("stop");
    assert_eq!(at(&w, "A1").unwrap().patch, bold_red);
    assert_eq!(at(&w, "A2").unwrap().patch, italic_blue_on_green);

    // What the grid gets: the cell's own style with the rules laid over.
    let shown = w.display(0, 0, 0);
    assert!(shown.style.font.bold && shown.style.font.color == Some(RED) && shown.style.fill.is_none());
    assert!(shown.adornment.is_none());
    assert!(w.display(0, 5, 5).style.fill.is_none());
}

#[test]
fn one_bar_one_scale_one_icon() {
    let mut w = column(&["10", "5", "text"]);
    let on = |rule: Rule| CondFormat::new(rects("A1:A3"), rule);
    let blue_bar = on(Rule::DataBar(DataBar::preset(BarColor::Blue, true)));
    let green_bar = on(Rule::DataBar(DataBar::preset(BarColor::Green, false)));
    let arrows = on(Rule::IconSet(IconRule::preset(IconSet::ThreeArrows)));
    let mut flags = IconRule::preset(IconSet::ThreeFlags);
    flags.show_value = false;
    let flags = on(Rule::IconSet(flags));
    let black_white = on(Rule::ColorScale(ColorScale::two(BLACK, WHITE)));
    let red_green = on(Rule::ColorScale(ColorScale::two(RED, GREEN)));
    let filled = on(Rule::greater_than(num(7.0))).with_format(fill(BLUE));
    let rules = vec![
        blue_bar.clone(),
        green_bar,
        arrows.clone(),
        flags,
        black_white.clone(),
        red_green,
        filled.clone(),
    ];
    w.set_cond_formats(0, rules);
    w.commit("rules");
    // A bar, an icon and a scale together, each from the first of its kind.
    let result = at(&w, "A1").unwrap();
    let adornment = result.adornment.unwrap();
    assert_eq!(adornment.bar.unwrap().fill, Color(0x638EC6));
    assert_eq!(adornment.icon.unwrap().set, IconSet::ThreeArrows);
    assert!(!adornment.hide_value);
    assert_eq!(result.patch.fill, Some(Some(WHITE)));
    assert_eq!(fill_at(&w, "A2"), Some(BLACK));
    // Text gets none of the graded formats, but passes "greater than 7".
    let on_text = at(&w, "A3").unwrap();
    assert_eq!((on_text.patch.fill, on_text.adornment), (Some(Some(BLUE)), None));

    // A fill of higher priority than the scale wins where the rule holds.
    w.set_cond_formats(0, vec![filled.clone(), black_white.clone()]);
    w.commit("fill first");
    assert_eq!(fill_at(&w, "A1"), Some(BLUE));
    assert_eq!(fill_at(&w, "A2"), Some(BLACK));

    // "Stop If True" above graded rules hides them, and a rule without any
    // format is enough for that.
    let stop = on(Rule::greater_than(num(7.0))).with_stop_if_true(true);
    w.set_cond_formats(0, vec![stop, blue_bar, arrows, black_white.clone()]);
    w.commit("stop");
    assert_eq!(at(&w, "A1"), None);
    assert!(at(&w, "A2").unwrap().adornment.unwrap().bar.is_some());
    assert_eq!(at(&w, "A3"), None);

    // A graded rule with "Stop If True" (only files can say so) stops on
    // the cells it grades.
    w.set_cond_formats(0, vec![black_white.with_stop_if_true(true), filled]);
    w.commit("graded stop");
    assert_eq!(fill_at(&w, "A1"), Some(WHITE));
    assert_eq!(fill_at(&w, "A3"), Some(BLUE));
}

#[test]
fn many_rules_are_found_by_row() {
    // Enough rules for the row index, with ranges that overlap, repeat and
    // run to the end of the sheet.
    let mut w = Workbook::new(Locale::en());
    for r in 0..60 {
        for c in 0..4 {
            w.set_value(0, r, c, Value::Number(1.0));
        }
    }
    w.commit("cells");
    let mut rules = Vec::new();
    for i in 0..40u32 {
        let ranges = vec![
            Rect::new(i, 0, i + i % 5, 1),
            Rect::new(i + 2, 2, i + 2, 2),
            Rect::new(59 - i, 3, 59 - i, 3),
        ];
        rules.push(CondFormat::new(ranges, Rule::equal_to(num(1.0))).with_format(fill(Color(i))));
    }
    rules.push(CondFormat::new(rects("C:C"), Rule::equal_to(num(1.0))).with_format(fill(Color(100))));
    w.set_cond_formats(0, rules.clone());
    w.commit("rules");
    for r in 0..62 {
        for c in 0..5 {
            let expected = rules
                .iter()
                .find(|cf| cf.applies_to(r, c) && r < 60 && c < 4)
                .map(|cf| cf.format.fill.unwrap().unwrap());
            assert_eq!(
                result_at(&w, 0, r, c).and_then(|res| res.patch.fill.flatten()),
                expected,
                "row {r} col {c}"
            );
        }
    }
}

#[test]
fn rules_too_entangled_to_index_still_work() {
    // 1500 ranges nested inside one another cross some 3000 bands of rows
    // each: no index is built and every rule is tried instead.
    let mut w = Workbook::new(Locale::en());
    for r in [0, 700, 1499, 1500, 2999] {
        w.set_value(0, r, 0, Value::Number(1.0));
    }
    w.commit("cells");
    let rules: Vec<CondFormat> = (0..1500u32)
        .map(|i| {
            CondFormat::new(vec![Rect::new(i, 0, 2999 - i, 0)], Rule::equal_to(num(1.0))).with_format(fill(Color(i)))
        })
        .collect();
    w.set_cond_formats(0, rules);
    w.commit("rules");
    // Row 700 is inside the first 701 ranges, row 1499 inside all of them.
    for (row, rule) in [(0, 0), (700, 0), (1499, 0), (1500, 0), (2999, 0)] {
        assert_eq!(result_at(&w, 0, row, 0).unwrap().patch.fill, Some(Some(Color(rule))));
    }
    w.update_cond_format(0, 0, |cf| cf.rule = Rule::equal_to(num(2.0)));
    assert_eq!(result_at(&w, 0, 700, 0).unwrap().patch.fill, Some(Some(Color(1))));
    assert_eq!(result_at(&w, 0, 0, 0), None);
    assert_eq!(result_at(&w, 0, 3000, 0), None);
    assert_eq!(result_at(&w, 0, 700, 1), None);
}

// ---- keeping up with the workbook ---------------------------------------------------------------------------

#[test]
fn results_follow_cells_rules_and_undo() {
    let mut w = column(&["1", "2", "3"]);
    only(&mut w, "A1:A3", Rule::top_items(1));
    assert_eq!(hits(&w, "A1:A3"), "A3");
    // A cell changes.
    w.set_cell_text(0, 0, 0, "10").unwrap();
    w.commit("edit");
    assert_eq!(hits(&w, "A1:A3"), "A1");
    assert!(w.undo());
    assert_eq!(hits(&w, "A1:A3"), "A3");
    // A rule changes, before any commit.
    w.update_cond_format(0, 0, |cf| cf.rule = Rule::bottom_items(1));
    assert_eq!(hits(&w, "A1:A3"), "A1");
    w.rollback();
    assert_eq!(hits(&w, "A1:A3"), "A3");
    w.update_cond_format(0, 0, |cf| cf.rule = Rule::bottom_items(2));
    w.commit("rule");
    assert_eq!(hits(&w, "A1:A3"), "A1 A2");
    assert!(w.undo());
    assert_eq!(hits(&w, "A1:A3"), "A3");
    assert!(w.redo());
    assert_eq!(hits(&w, "A1:A3"), "A1 A2");
    // The list edited behind the workbook's back.
    w.sheets[0].cond_formats[0].rule = Rule::top_items(1);
    w.caches.cond.invalidate();
    assert_eq!(hits(&w, "A1:A3"), "A3");
    // A sheet without rules, a sheet that does not exist.
    assert_eq!(result_at(&w, 0, 50, 50), None);
    assert_eq!(result_at(&w, 7, 0, 0), None);
}

#[test]
fn spilled_arrays_take_part() {
    let mut w = wb(&[("A1", "=SEQUENCE(5)"), ("C1", "=SEQUENCE(1,3)*10")]);
    assert_eq!(matches(&mut w, "A1:A5", Rule::top_items(2), "A1:A6"), "A4 A5");
    assert_eq!(
        matches(&mut w, "A1:A5,C1:E1", Rule::above_average(), "A1:E5"),
        "C1 D1 E1"
    );
    assert_eq!(matches(&mut w, "A1:E5", Rule::DuplicateValues, "A1:E5"), "");
    assert_eq!(
        matches(&mut w, "A1:A5", Rule::Formula(f("ISODD(A1)")), "A1:A5"),
        "A1 A3 A5"
    );
    only(&mut w, "A1:A5", Rule::ColorScale(ColorScale::two(BLACK, WHITE)));
    assert_eq!(fill_at(&w, "A3"), Some(Color(0x808080)));
}

// ---- managing rules ------------------------------------------------------------------------------------------

#[test]
fn adding_editing_and_reordering_rules() {
    let mut w = column(&["1", "2", "3"]);
    let on = |ranges: &str, n: f64| CondFormat::new(rects(ranges), Rule::greater_than(num(n)));
    assert!(w.add_cond_format(0, on("A1:A3", 1.0)));
    assert!(w.add_cond_format(0, on("B1:B3", 2.0)));
    assert!(w.insert_cond_format(0, 2, on("C1:C3", 3.0)));
    w.commit("rules");
    // New rules go to the top of the list.
    let order = |w: &Workbook| -> Vec<String> { (0..w.cond_formats(0).len()).map(|i| applies_to(w, 0, i)).collect() };
    assert_eq!(order(&w), ["=$B$1:$B$3", "=$A$1:$A$3", "=$C$1:$C$3"]);
    assert!(w.reorder_cond_format(0, 2, 0));
    assert_eq!(order(&w), ["=$C$1:$C$3", "=$B$1:$B$3", "=$A$1:$A$3"]);
    assert!(w.remove_cond_format(0, 1));
    assert_eq!(order(&w), ["=$C$1:$C$3", "=$A$1:$A$3"]);
    w.commit("manage");
    assert!(w.undo());
    assert_eq!(order(&w), ["=$B$1:$B$3", "=$A$1:$A$3", "=$C$1:$C$3"]);
    // Rules without a range, or out of place, are refused; nothing is left
    // to undo.
    w.clear_history();
    assert!(!w.add_cond_format(0, CondFormat::new(Vec::new(), Rule::Blanks)));
    assert!(!w.add_cond_format(9, on("A1", 1.0)));
    assert!(!w.insert_cond_format(0, 4, on("A1", 1.0)));
    assert!(!w.remove_cond_format(0, 3));
    assert!(!w.reorder_cond_format(0, 1, 1));
    assert!(!w.reorder_cond_format(0, 1, 3));
    assert!(!w.update_cond_format(0, 3, |_| ()));
    assert!(!w.set_cond_format_ranges(0, 0, Vec::new()));
    assert!(!w.clear_cond_formats(0, &rects("H1:H9")));
    assert!(!w.clear_all_cond_formats(1));
    assert!(!w.commit("nothing"));
    // The rules of a selection, as the Rules Manager lists them.
    assert_eq!(w.cond_formats_in(0, &rects("A2")), [1]);
    assert_eq!(w.cond_formats_in(0, &rects("C9,A1:B1")), [0, 1]);
    assert_eq!(w.cond_formats_in(0, &rects("D1:D9")), Vec::<usize>::new());
    assert!(w.clear_all_cond_formats(0));
    w.commit("clear");
    assert!(w.cond_formats(0).is_empty() && w.cond_formats(5).is_empty());
    assert!(w.undo());
    assert_eq!(w.cond_formats(0).len(), 3);
}

#[test]
fn changing_what_a_rule_applies_to() {
    let mut w = wb(&[
        ("A1", "1"),
        ("A2", "5"),
        ("B1", "5"),
        ("B2", "1"),
        ("C1", "0"),
        ("C2", "9"),
    ]);
    // Each cell of column A against its neighbour on the right.
    only(&mut w, "A1:A2", Rule::Formula(f("A1>B1+$E$1")));
    assert_eq!(hits(&w, "A1:C2"), "A2");
    // Moved to column B, the rule still looks one cell to the right.
    assert!(w.set_cond_format_ranges(0, 0, rects("B1:B2")));
    w.commit("applies to");
    assert_eq!(formulas(&w, 0, 0), "B1>C1+$E$1");
    assert_eq!(hits(&w, "A1:C2"), "B1");
    assert!(w.undo());
    assert_eq!(formulas(&w, 0, 0), "A1>B1+$E$1");
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$2");
}

#[test]
fn clearing_rules_from_a_selection() {
    let mut w = wb(&[("A1", "1")]);
    let grid = CondFormat::new(rects("A1:C3"), Rule::Formula(f("A1>$E$1")));
    let column_e = CondFormat::new(rects("E:E"), Rule::NoBlanks);
    let single = CondFormat::new(rects("B2"), Rule::Blanks);
    w.set_cond_formats(0, vec![grid, column_e, single]);
    w.commit("rules");
    // The middle of the block: the range is split around it, and the rule
    // that only had that cell goes.
    assert!(w.clear_cond_formats(0, &rects("B2")));
    w.commit("clear");
    assert_eq!(w.cond_formats(0).len(), 2);
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$C$1,$A$2,$C$2,$A$3:$C$3");
    assert_eq!(formulas(&w, 0, 0), "A1>$E$1");
    // The first rows: the anchor moves down and the formula with it.
    assert!(w.clear_cond_formats(0, &rects("1:2")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$3:$C$3");
    assert_eq!(formulas(&w, 0, 0), "A3>$E$1");
    assert_eq!(applies_to(&w, 0, 1), "=$E$3:$E$1048576");
    // Several areas at once.
    assert!(w.clear_cond_formats(0, &rects("A3,C3:E10")));
    assert_eq!(applies_to(&w, 0, 0), "=$B$3");
    assert_eq!(formulas(&w, 0, 0), "B3>$E$1");
    assert_eq!(applies_to(&w, 0, 1), "=$E$11:$E$1048576");
    w.commit("clear more");
    assert!(w.undo());
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$C$1,$A$2,$C$2,$A$3:$C$3");
    assert!(w.undo());
    assert_eq!(w.cond_formats(0).len(), 3);
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$C$3");
}

#[test]
fn ranges_are_kept_tidy() {
    let tidy = |s: &str| ranges_text(&normalize_ranges(rects(s)), Locale::en());
    assert_eq!(tidy("A1:A3,A4:A6"), "=$A$1:$A$6");
    assert_eq!(tidy("A1:B2,C1:D2"), "=$A$1:$D$2");
    assert_eq!(tidy("A1:C3,B2"), "=$A$1:$C$3");
    assert_eq!(tidy("B2,A1:C3"), "=$A$1:$C$3");
    assert_eq!(tidy("A1:A3,A2:A5"), "=$A$1:$A$5");
    // Pieces that only make a rectangle once others have been joined.
    assert_eq!(tidy("A1:B1,A2,B2"), "=$A$1:$B$2");
    assert_eq!(tidy("A3,A1,A2"), "=$A$1:$A$3");
    // Nothing to join.
    assert_eq!(tidy("A1:A3,B2:B4"), "=$A$1:$A$3,$B$2:$B$4");
    assert_eq!(tidy("A1,B2"), "=$A$1,$B$2");
    assert_eq!(tidy("A:A,B:B"), "=$A:$B");
}

// ---- rows, columns and sheets ------------------------------------------------------------------------------------

#[test]
fn inserting_and_deleting_rows() {
    let mut w = wb(&[("A1", "x")]);
    only(&mut w, "A2:A10", Rule::Formula(f("B2>$C$20")));
    let state = |w: &Workbook| format!("{} {}", applies_to(w, 0, 0), formulas(w, 0, 0));
    // Above the range: everything moves down.
    w.insert_rows(0, 0, 2);
    w.commit("insert");
    assert_eq!(state(&w), "=$A$4:$A$12 B4>$C$22");
    // Inside: the range grows.
    w.insert_rows(0, 5, 1);
    w.commit("insert");
    assert_eq!(state(&w), "=$A$4:$A$13 B4>$C$23");
    // Right below the last row: the new row is formatted like the one
    // above it, conditional formats included.
    w.insert_rows(0, 13, 1);
    w.commit("insert");
    assert_eq!(state(&w), "=$A$4:$A$14 B4>$C$24");
    // Further down: not ours.
    w.insert_rows(0, 15, 1);
    w.commit("insert");
    assert_eq!(state(&w), "=$A$4:$A$14 B4>$C$25");
    // Deleting the first rows of the range, anchor included: the formula
    // is the one the next cell had.
    w.delete_rows(0, 3, 2);
    w.commit("delete");
    assert_eq!(state(&w), "=$A$4:$A$12 B4>$C$23");
    // Deleting the cell an absolute reference names.
    w.delete_rows(0, 22, 1);
    w.commit("delete");
    assert_eq!(state(&w), "=$A$4:$A$12 B4>#REF!");
    // Deleting every row of the rule deletes the rule.
    w.delete_rows(0, 2, 12);
    w.commit("delete");
    assert!(w.cond_formats(0).is_empty());
    assert!(w.undo());
    assert_eq!(state(&w), "=$A$4:$A$12 B4>#REF!");
    assert!(w.undo());
    assert_eq!(state(&w), "=$A$4:$A$12 B4>$C$23");
}

#[test]
fn inserting_and_deleting_columns() {
    let mut w = wb(&[("A1", "x")]);
    let wide = CondFormat::new(rects("B1:C5,E1:E5"), Rule::greater_than(f("$G$1")));
    let relative = CondFormat::new(rects("B1:C5"), Rule::Formula(f("B1>A1")));
    let whole = CondFormat::new(rects("3:4"), Rule::NoBlanks);
    w.set_cond_formats(0, vec![wide, relative, whole]);
    w.commit("rules");
    let state = |w: &Workbook, i: usize| format!("{} {}", applies_to(w, 0, i), formulas(w, 0, i));
    w.insert_cols(0, 0, 1);
    w.commit("insert");
    assert_eq!(state(&w, 0), "=$C$1:$D$5,$F$1:$F$5 $H$1");
    assert_eq!(state(&w, 1), "=$C$1:$D$5 C1>B1");
    assert_eq!(state(&w, 2), "=$3:$4 ");
    // The anchor column goes: the formula is rewritten for the next one.
    w.delete_cols(0, 2, 1);
    w.commit("delete");
    assert_eq!(state(&w, 0), "=$C$1:$C$5,$E$1:$E$5 $G$1");
    assert_eq!(state(&w, 1), "=$C$1:$C$5 C1>#REF!");
    // The first range goes: the second one carries the rule.
    w.delete_cols(0, 2, 1);
    w.commit("delete");
    assert_eq!(state(&w, 0), "=$D$1:$D$5 $F$1");
    assert_eq!(w.cond_formats(0).len(), 2);
    assert_eq!(state(&w, 1), "=$3:$4 ");
    // Rows of a rule on whole rows.
    w.delete_rows(0, 2, 1);
    w.commit("delete");
    assert_eq!(state(&w, 1), "=$3:$3 ");
}

#[test]
fn whole_columns_stay_whole() {
    let mut w = wb(&[("A1", "1"), ("A2", "9"), ("C1", "5")]);
    only(&mut w, "A:A", Rule::Formula(f("A1>$C$1")));
    let state = |w: &Workbook| format!("{} {}", applies_to(w, 0, 0), formulas(w, 0, 0));
    assert_eq!(hits(&w, "A1:A5"), "A2");
    // Rows inserted at the very top: the column is still the whole column
    // and each cell still looks at itself.
    w.insert_rows(0, 0, 2);
    w.commit("insert");
    assert_eq!(state(&w), "=$A:$A A1>$C$3");
    assert_eq!(hits(&w, "A1:A5"), "A4");
    w.insert_rows(0, 3, 1);
    w.commit("insert");
    assert_eq!(state(&w), "=$A:$A A1>$C$3");
    assert_eq!(hits(&w, "A1:A6"), "A5");
    w.delete_rows(0, 0, 2);
    w.commit("delete");
    assert_eq!(state(&w), "=$A:$A A1>$C$1");
    assert_eq!(hits(&w, "A1:A5"), "A3");
    // A column inserted before it moves it; deleting it deletes the rule.
    w.insert_cols(0, 0, 1);
    w.commit("insert");
    assert_eq!(state(&w), "=$B:$B B1>$D$1");
    w.delete_cols(0, 1, 1);
    w.commit("delete");
    assert!(w.cond_formats(0).is_empty());
}

#[test]
fn edits_on_other_sheets_and_sheet_names() {
    let mut w = wb(&[("A1", "5"), ("A2", "50")]);
    let other = w.add_sheet(1);
    w.rename_sheet(other, "Other");
    w.set_cell_text(other, 4, 0, "10").unwrap();
    w.commit("setup");
    only(&mut w, "A1:A2", Rule::greater_than(f("Other!$A$5+Sheet1!$C$1")));
    assert_eq!(hits(&w, "A1:A2"), "A2");
    // Rows inserted on the other sheet: the reference follows, the range
    // does not move.
    w.insert_rows(other, 0, 3);
    w.commit("insert");
    assert_eq!(formulas(&w, 0, 0), "Other!$A$8+Sheet1!$C$1");
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$2");
    assert_eq!(hits(&w, "A1:A2"), "A2");
    // Renaming either sheet.
    assert!(w.rename_sheet(other, "Data 2"));
    assert!(w.rename_sheet(0, "Main"));
    w.commit("rename");
    assert_eq!(formulas(&w, 0, 0), "'Data 2'!$A$8+Main!$C$1");
    assert_eq!(hits(&w, "A1:A2"), "A2");
    // A copy of the sheet has the rule, reading the copy's own cells.
    let copy = w.duplicate_sheet(0).unwrap();
    w.commit("duplicate");
    assert_eq!(formulas(&w, copy, 0), "'Data 2'!$A$8+'Main (2)'!$C$1");
    assert_eq!(formulas(&w, 0, 0), "'Data 2'!$A$8+Main!$C$1");
    w.set_cell_text(copy, 0, 2, "100").unwrap();
    w.commit("edit copy");
    assert_eq!(hits_on(&w, copy, "A1:A2"), "");
    assert_eq!(hits_on(&w, 0, "A1:A2"), "A2");
    // Deleting the sheet a rule refers to leaves it without matches.
    assert!(w.delete_sheet(2));
    w.commit("delete sheet");
    assert_eq!(hits_on(&w, 0, "A1:A2"), "");
    assert!(w.undo());
    assert_eq!(hits_on(&w, 0, "A1:A2"), "A2");
}

// ---- copy, paste, move ------------------------------------------------------------------------------------------------

#[test]
fn copied_cells_take_their_rules_along() {
    let mut w = wb(&[
        ("A1", "10"),
        ("A2", "1"),
        ("A3", "7"),
        ("B1", "5"),
        ("B2", "5"),
        ("B3", "5"),
        ("C7", "9"),
        ("D7", "1"),
    ]);
    let relative = CondFormat::new(rects("A1"), Rule::Formula(f("A1>B1"))).with_format(fill(RED));
    let old = CondFormat::new(rects("A2:B3,C7"), Rule::NoBlanks).with_format(fill(GREEN));
    w.set_cond_formats(0, vec![relative, old]);
    w.commit("rules");
    // A1 pasted over A2:A3: the rule grows over the copies, whose formula
    // reads their own neighbour; what was there before is gone.
    assert!(w.copy_cond_formats(0, rect("A1"), 0, rect("A2:A3")));
    w.commit("paste");
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$3");
    assert_eq!(formulas(&w, 0, 0), "A1>B1");
    assert_eq!(applies_to(&w, 0, 1), "=$B$2:$B$3,$C$7");
    assert_eq!(fill_at(&w, "A1"), Some(RED));
    assert_eq!(fill_at(&w, "A2"), None);
    assert_eq!(fill_at(&w, "A3"), Some(RED));
    assert_eq!(fill_at(&w, "B2"), Some(GREEN));
    // Somewhere else on the sheet: a second range, the same rule.
    assert!(w.copy_cond_formats(0, rect("A1:B1"), 0, rect("C7:D7")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$3,$C$7");
    assert_eq!(applies_to(&w, 0, 1), "=$B$2:$B$3");
    assert_eq!(fill_at(&w, "C7"), Some(RED));
    // Cells without rules pasted over cells with rules clear them.
    assert!(w.copy_cond_formats(0, rect("F1:F2"), 0, rect("A1:A2")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$3,$C$7");
    assert_eq!(formulas(&w, 0, 0), "A3>B3");
    // Nothing to do, nothing to undo.
    w.commit("pastes");
    assert!(!w.copy_cond_formats(0, rect("F1:F2"), 0, rect("G1:G2")));
    assert!(!w.commit("nothing"));
    assert!(w.undo());
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$3");
}

#[test]
fn pasting_several_copies_and_part_of_a_rule() {
    let mut w = wb(&[("A1", "1")]);
    let block = CondFormat::new(rects("A1:B4"), Rule::Formula(f("$A1>B$1"))).with_format(fill(RED));
    w.set_cond_formats(0, vec![block]);
    w.commit("rule");
    // The top row of the rule, pasted three times down column D:E.
    assert!(w.copy_cond_formats(0, rect("A1:C1"), 0, rect("D5:F7")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$B$4,$D$5:$E$7");
    // A two-row piece pasted twice: two ranges that join into one.
    assert!(w.copy_cond_formats(0, rect("B2:B3"), 0, rect("H1:H4")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$B$4,$D$5:$E$7,$H$1:$H$4");
    // A piece that is not the whole height of the copied block.
    assert!(w.copy_cond_formats(0, rect("B4:B5"), 0, rect("J1:J4")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$B$4,$D$5:$E$7,$H$1:$H$4,$J$1,$J$3");
    // A destination that is not a whole number of copies takes one.
    assert!(w.copy_cond_formats(0, rect("A1:A2"), 0, rect("L1:L3")));
    assert_eq!(
        applies_to(&w, 0, 0),
        "=$A$1:$B$4,$D$5:$E$7,$H$1:$H$4,$J$1,$J$3,$L$1:$L$2"
    );
    // Source and destination overlapping (a fill down).
    let mut w = wb(&[("A1", "1")]);
    only(&mut w, "A1:A2", Rule::NoBlanks);
    assert!(w.copy_cond_formats(0, rect("A1:A2"), 0, rect("A1:A6")));
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$6");
}

#[test]
fn rules_copied_to_another_sheet() {
    let mut w = wb(&[("C3", "5"), ("C4", "1"), ("A1", "3")]);
    let other = w.add_sheet(1);
    w.set_cell_text(other, 0, 4, "5").unwrap();
    w.set_cell_text(other, 1, 4, "1").unwrap();
    w.set_cell_text(other, 0, 0, "3").unwrap();
    w.commit("setup");
    let first = CondFormat::new(rects("A1:A9,C3:C4"), Rule::Formula(f("A1>$A$1"))).with_format(fill(RED));
    let second = CondFormat::new(rects("C1:C9"), Rule::NoBlanks).with_format(fill(GREEN));
    w.set_cond_formats(0, vec![first, second]);
    let there = CondFormat::new(rects("E1:F1,A5"), Rule::Blanks);
    w.set_cond_formats(other, vec![there]);
    w.commit("rules");
    assert!(w.copy_cond_formats(0, rect("C3:C4"), other, rect("E1:E2")));
    w.commit("paste");
    // New rules on top, in their order, written for their new anchor; the
    // cells pasted over left the rule that was there.
    assert_eq!(w.cond_formats(other).len(), 3);
    assert_eq!(applies_to(&w, other, 0), "=$E$1:$E$2");
    assert_eq!(formulas(&w, other, 0), "E1>$A$1");
    assert_eq!(w.cond_formats(other)[1].rule, Rule::NoBlanks);
    assert_eq!(applies_to(&w, other, 1), "=$E$1:$E$2");
    assert_eq!(applies_to(&w, other, 2), "=$F$1,$A$5");
    // The copies read the cells of their own sheet.
    assert_eq!(result_at(&w, other, 0, 4).unwrap().patch.fill, Some(Some(RED)));
    assert_eq!(result_at(&w, other, 1, 4).unwrap().patch.fill, Some(Some(GREEN)));
    // The source is untouched.
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$9,$C$3:$C$4");
    assert!(w.undo());
    assert_eq!(w.cond_formats(other).len(), 1);
}

#[test]
fn moved_cells_take_their_rules_along() {
    let mut w = wb(&[("A1", "9"), ("A2", "1"), ("D1", "5")]);
    let moving = CondFormat::new(rects("A1:A4"), Rule::Formula(f("A1>$D$1"))).with_format(fill(RED));
    let watching = CondFormat::new(rects("F1"), Rule::Formula(f("$A$1+$A$3+A2>0")));
    w.set_cond_formats(0, vec![moving, watching]);
    w.commit("rules");
    // The top half of the rule moves to column C.
    assert!(w.move_cond_formats(0, rect("A1:A2"), 0, 0, 2));
    w.commit("move");
    assert_eq!(applies_to(&w, 0, 0), "=$A$3:$A$4,$C$1:$C$2");
    assert_eq!(formulas(&w, 0, 0), "A3>$D$1");
    // Fixed references to the moved cells follow them.
    assert_eq!(formulas(&w, 0, 1), "$C$1+$A$3+A2>0");
    assert!(w.undo());
    assert_eq!(applies_to(&w, 0, 0), "=$A$1:$A$4");
    // To another sheet: the rule is split between the two.
    let other = w.add_sheet(1);
    w.commit("sheet");
    assert!(w.move_cond_formats(0, rect("A1:A2"), other, 4, 4));
    assert_eq!(applies_to(&w, 0, 0), "=$A$3:$A$4");
    assert_eq!(applies_to(&w, other, 0), "=$E$5:$E$6");
    assert_eq!(formulas(&w, other, 0), "E5>$D$1");
    assert!(!w.move_cond_formats(0, rect("A3:A4"), 0, 2, 0));
}

// ---- dialogs and files ------------------------------------------------------------------------------------------------

#[test]
fn typing_and_showing_operands() {
    let en = Locale::en();
    let fr = Locale::fr();
    let (a1, a3) = ((0, 0), (2, 0));
    assert_eq!(parse_operand("5", en, a1, a1), Ok(num(5.0)));
    assert_eq!(parse_operand(" 1,5 ", fr, a1, a1), Ok(num(1.5)));
    assert_eq!(parse_operand("abc", en, a1, a1), Ok(text("abc")));
    assert_eq!(parse_operand("vrai", fr, a1, a1), Ok(Expr::Bool(true)));
    assert_eq!(
        parse_operand("15/05/2024", fr, a1, a1),
        Ok(num(date_to_serial(2024, 5, 15).unwrap()))
    );
    assert!(parse_operand("", en, a1, a1).is_err());
    assert!(parse_operand("=1+", en, a1, a1).is_err());
    // A formula is typed for the active cell and kept for the anchor.
    let typed = parse_operand("=SOMME($B$1:$B$3)+B3*1,5", fr, a3, a1).unwrap();
    assert_eq!(typed.to_text(en), "SUM($B$1:$B$3)+B1*1.5");
    assert_eq!(operand_text(&typed, fr, a1, a3, false), "=SOMME($B$1:$B$3)+B3*1,5");
    assert_eq!(operand_text(&typed, en, a1, a1, false), "=SUM($B$1:$B$3)+B1*1.5");
    assert_eq!(operand_text(&num(1.5), fr, a1, a3, false), "1,5");
    assert_eq!(operand_text(&text("abc"), en, a1, a1, false), "=\"abc\"");
    // The box of a text rule holds the text itself, or a formula.
    assert_eq!(parse_text_operand("abc", en, a1, a1), Ok(text("abc")));
    assert_eq!(parse_text_operand("12", en, a1, a1), Ok(text("12")));
    assert_eq!(parse_text_operand("=", en, a1, a1), Ok(text("=")));
    let reference = parse_text_operand("=$D3", en, a3, a1).unwrap();
    assert_eq!(reference.to_text(en), "$D1");
    assert_eq!(operand_text(&reference, en, a1, a3, true), "=$D3");
    assert_eq!(operand_text(&text("abc"), en, a1, a1, true), "abc");
    // "Applies to".
    let ranges = rects("A1:A10,C1,B:C,2:3");
    assert_eq!(ranges_text(&ranges, en), "=$A$1:$A$10,$C$1,$B:$C,$2:$3");
    assert_eq!(ranges_text(&ranges, fr), "=$A$1:$A$10;$C$1;$B:$C;$2:$3");
    assert_eq!(parse_ranges("=$A$1:$A$10;$C$1; $B:$C ;$2:$3"), Some(ranges.clone()));
    assert_eq!(parse_ranges("a1:a10,c1,b:c,2:3"), Some(ranges));
    assert_eq!(parse_ranges("=Sheet1!$B$2:$C$3"), Some(rects("B2:C3")));
    assert_eq!(parse_ranges("=A1,nonsense"), None);
    assert_eq!(parse_ranges(""), None);
    assert!(is_relative(&f("A1>$B$1")) && is_relative(&f("SUM($A:B)")) && is_relative(&f("$A1")));
    assert!(!is_relative(&f("$A$1+SUM($B:$C)+$2:$3+5")));
}

#[test]
fn formulas_stored_in_files() {
    let en = Locale::en();
    let stored = |ranges: &str, rule: Rule| -> Vec<String> {
        ooxml_formulas(&CondFormat::new(rects(ranges), rule))
            .iter()
            .map(|e| e.to_text(en))
            .collect()
    };
    assert_eq!(stored("B2:B9", Rule::between(num(1.0), f("$A$1"))), ["1", "$A$1"]);
    assert_eq!(stored("B2:B9", Rule::Formula(f("B2>1"))), ["B2>1"]);
    assert_eq!(
        stored("B2:B9", Rule::text_contains(text("a\"b"))),
        ["NOT(ISERROR(SEARCH(\"a\"\"b\",B2)))"]
    );
    let text_rule = |op: TextOp, text: Expr| Rule::Text { op, text };
    assert_eq!(
        stored("B2", text_rule(TextOp::NotContains, f("$D$1"))),
        ["ISERROR(SEARCH($D$1,B2))"]
    );
    assert_eq!(
        stored("B2", text_rule(TextOp::BeginsWith, text("ab"))),
        ["LEFT(B2,LEN(\"ab\"))=\"ab\""]
    );
    assert_eq!(
        stored("B2", text_rule(TextOp::EndsWith, text("ab"))),
        ["RIGHT(B2,LEN(\"ab\"))=\"ab\""]
    );
    assert_eq!(stored("C3:D4,A1", Rule::Blanks), ["LEN(TRIM(C3))=0"]);
    assert_eq!(stored("C3", Rule::NoBlanks), ["LEN(TRIM(C3))>0"]);
    assert_eq!(stored("C3", Rule::Errors), ["ISERROR(C3)"]);
    assert_eq!(stored("C3", Rule::NoErrors), ["NOT(ISERROR(C3))"]);
    assert_eq!(
        stored("C3", Rule::date_occurring(TimePeriod::Today)),
        ["FLOOR(C3,1)=TODAY()"]
    );
    assert_eq!(
        stored("C3", Rule::date_occurring(TimePeriod::LastMonth)),
        ["AND(MONTH(C3)=MONTH(EDATE(TODAY(),0-1)),YEAR(C3)=YEAR(EDATE(TODAY(),0-1)))"]
    );
    for period in TimePeriod::ALL {
        assert_eq!(stored("A1", Rule::date_occurring(period)).len(), 1);
    }
    for rule in [
        Rule::top_items(10),
        Rule::above_average(),
        Rule::DuplicateValues,
        Rule::UniqueValues,
    ] {
        assert!(stored("A1", rule).is_empty());
    }
    assert!(stored("A1", Rule::DataBar(DataBar::preset(BarColor::Blue, true))).is_empty());

    // Reading a text rule back: the operand comes out of the formula.
    for op in TextOp::ALL {
        for operand in [text("a\"b"), f("$D$1"), f("LEFT($D$1,2)")] {
            let cf = CondFormat::new(rects("B2:B9"), text_rule(op, operand.clone()));
            let formula = &ooxml_formulas(&cf)[0];
            assert_eq!(
                text_operand_from_ooxml(op, Some("ignored"), Some(formula)),
                Some(operand)
            );
        }
        assert_eq!(text_operand_from_ooxml(op, Some("abc"), None), Some(text("abc")));
        assert_eq!(text_operand_from_ooxml(op, None, None), None);
        assert_eq!(text_operand_from_ooxml(op, Some("abc"), Some(&f("B2>1"))), None);
    }
}

#[test]
fn spellings_used_in_files() {
    for op in CellOp::ALL {
        assert_eq!(CellOp::from_ooxml(op.ooxml()), Some(op));
    }
    for op in TextOp::ALL {
        assert_eq!(TextOp::from_ooxml_type(op.ooxml_type()), Some(op));
    }
    for period in TimePeriod::ALL {
        assert_eq!(TimePeriod::from_ooxml(period.ooxml()), Some(period));
    }
    for axis in BarAxis::ALL {
        assert_eq!(BarAxis::from_ooxml(axis.ooxml()), Some(axis));
    }
    for direction in BarDirection::ALL {
        assert_eq!(BarDirection::from_ooxml(direction.ooxml()), Some(direction));
    }
    let mut icons: std::collections::HashSet<Icon> = std::collections::HashSet::new();
    for set in IconSet::ALL {
        assert_eq!(IconSet::from_ooxml(set.ooxml()), Some(set));
        assert_eq!(
            set.ooxml().chars().next().and_then(|c| c.to_digit(10)),
            Some(set.len() as u32)
        );
        assert_eq!(set.is_x14(), ["3Stars", "3Triangles", "5Boxes"].contains(&set.ooxml()));
        icons.extend(set.icons());
    }
    // Excel has 52 different icons, all of them in some set.
    assert_eq!(icons.len(), 52);
    assert_eq!(IconSet::from_ooxml("NoIcons"), None);
    assert_eq!(
        (CellOp::Greater.ooxml(), TextOp::NotContains.ooxml_operator()),
        ("greaterThan", "notContains")
    );
    assert_eq!(
        (TimePeriod::Last7Days.ooxml(), BarAxis::Midpoint.ooxml()),
        ("last7Days", "middle")
    );
    assert_eq!(
        (IconSet::FourRedToBlack.ooxml(), IconSet::ThreeTrafficLights2.ooxml()),
        ("4RedToBlack", "3TrafficLights2")
    );

    let thresholds = [
        (Cfvo::Min, "min"),
        (Cfvo::Max, "max"),
        (Cfvo::number(1.0), "num"),
        (Cfvo::percent(1.0), "percent"),
        (Cfvo::percentile(1.0), "percentile"),
        (Cfvo::Formula(f("$A$1")), "formula"),
    ];
    for (cfvo, ty) in thresholds {
        assert_eq!(cfvo.ooxml_type(true), ty);
        assert_eq!(Cfvo::from_ooxml(ty, cfvo.value().cloned()), Some(cfvo));
    }
    assert_eq!(
        (Cfvo::Automatic.ooxml_type(true), Cfvo::Automatic.ooxml_type(false)),
        ("autoMin", "autoMax")
    );
    assert_eq!(Cfvo::from_ooxml("autoMax", None), Some(Cfvo::Automatic));
    assert_eq!(Cfvo::from_ooxml("num", None), None);
    assert_eq!(Cfvo::from_ooxml("other", Some(num(1.0))), None);

    let types = [
        (Rule::greater_than(num(1.0)), "cellIs"),
        (Rule::Formula(num(1.0)), "expression"),
        (Rule::top_items(1), "top10"),
        (Rule::above_average(), "aboveAverage"),
        (Rule::DuplicateValues, "duplicateValues"),
        (Rule::UniqueValues, "uniqueValues"),
        (Rule::text_contains(text("a")), "containsText"),
        (Rule::Blanks, "containsBlanks"),
        (Rule::NoBlanks, "notContainsBlanks"),
        (Rule::Errors, "containsErrors"),
        (Rule::NoErrors, "notContainsErrors"),
        (Rule::date_occurring(TimePeriod::Today), "timePeriod"),
        (Rule::ColorScale(ScalePreset::WhiteRed.scale()), "colorScale"),
        (Rule::DataBar(DataBar::legacy(RED)), "dataBar"),
        (Rule::IconSet(IconRule::preset(IconSet::FiveBoxes)), "iconSet"),
    ];
    for (rule, ty) in types {
        assert_eq!(rule.ooxml_type(), ty);
        assert_eq!(rule.is_graded(), ["colorScale", "dataBar", "iconSet"].contains(&ty));
    }
}

#[test]
fn presets_have_excels_colours() {
    let patch = |format: HighlightFormat| format.patch();
    let both = |fill: u32, font: u32| StylePatch {
        fill: Some(Some(Color(fill))),
        font_color: Some(Some(Color(font))),
        ..StylePatch::default()
    };
    assert_eq!(
        patch(HighlightFormat::LightRedFillDarkRedText),
        both(0xFFC7CE, 0x9C0006)
    );
    assert_eq!(
        patch(HighlightFormat::YellowFillDarkYellowText),
        both(0xFFEB9C, 0x9C5700)
    );
    assert_eq!(patch(HighlightFormat::GreenFillDarkGreenText), both(0xC6EFCE, 0x006100));
    assert_eq!(patch(HighlightFormat::LightRedFill), fill(Color(0xFFC7CE)));
    assert_eq!(patch(HighlightFormat::RedText).font_color, Some(Some(Color(0x9C0006))));
    let line = Some(Border {
        style: BorderStyle::Thin,
        color: Some(Color(0x9C0006)),
    });
    let bordered = patch(HighlightFormat::RedBorder);
    assert_eq!(
        (
            bordered.border_left,
            bordered.border_right,
            bordered.border_top,
            bordered.border_bottom
        ),
        (line, line, line, line)
    );
    assert_eq!(bordered.fill, None);
    assert_eq!(HighlightFormat::ALL.len(), 6);

    let bars: Vec<u32> = BarColor::ALL.iter().map(|c| c.color().value()).collect();
    assert_eq!(bars, [0x638EC6, 0x63C384, 0xFF555A, 0xFFB628, 0x008AEF, 0xD6007B]);
    let gradient = DataBar::preset(BarColor::Red, true);
    assert_eq!(
        (gradient.min.clone(), gradient.max.clone()),
        (Cfvo::Automatic, Cfvo::Automatic)
    );
    assert_eq!(
        (gradient.fill, gradient.border),
        (Color(0xFF555A), Some(Color(0xFF555A)))
    );
    assert_eq!(
        (gradient.negative_fill, gradient.negative_border),
        (Some(RED), Some(RED))
    );
    assert_eq!(
        (gradient.axis, gradient.axis_color, gradient.direction),
        (BarAxis::Automatic, BLACK, BarDirection::Context)
    );
    assert_eq!(
        (gradient.min_length, gradient.max_length, gradient.show_value),
        (0, 100, true)
    );
    let solid = DataBar::preset(BarColor::Red, false);
    assert_eq!(
        (solid.gradient, solid.border, solid.negative_fill, solid.negative_border),
        (false, None, Some(RED), None)
    );

    let colors = |preset: ScalePreset| -> Vec<u32> { preset.scale().stops.iter().map(|s| s.color.value()).collect() };
    let (red, yellow, green, white, blue) = (0xF8696B, 0xFFEB84, 0x63BE7B, 0xFCFCFF, 0x5A8AC6);
    // From the lowest number to the highest.
    assert_eq!(colors(ScalePreset::GreenYellowRed), [red, yellow, green]);
    assert_eq!(colors(ScalePreset::RedYellowGreen), [green, yellow, red]);
    assert_eq!(colors(ScalePreset::GreenWhiteRed), [red, white, green]);
    assert_eq!(colors(ScalePreset::RedWhiteGreen), [green, white, red]);
    assert_eq!(colors(ScalePreset::BlueWhiteRed), [red, white, blue]);
    assert_eq!(colors(ScalePreset::RedWhiteBlue), [blue, white, red]);
    assert_eq!(colors(ScalePreset::WhiteRed), [red, white]);
    assert_eq!(colors(ScalePreset::RedWhite), [white, red]);
    assert_eq!(colors(ScalePreset::GreenWhite), [white, green]);
    assert_eq!(colors(ScalePreset::WhiteGreen), [green, white]);
    assert_eq!(colors(ScalePreset::GreenYellow), [0xFFEF9C, green]);
    assert_eq!(colors(ScalePreset::YellowGreen), [green, 0xFFEF9C]);
    let three = ScalePreset::GreenYellowRed.scale();
    let at: Vec<Cfvo> = three.stops.iter().map(|s| s.at.clone()).collect();
    assert_eq!(at, [Cfvo::Min, Cfvo::percentile(50.0), Cfvo::Max]);
    assert_eq!(ScalePreset::WhiteRed.scale().stops.len(), 2);

    for set in IconSet::ALL {
        let rule = IconRule::preset(set);
        let percents: Vec<f64> = rule
            .thresholds
            .iter()
            .map(|t| match &t.value {
                Cfvo::Percent(Expr::Number(p)) if t.gte => *p,
                other => panic!("{other:?}"),
            })
            .collect();
        match set.len() {
            3 => assert_eq!(percents, [0.0, 33.0, 67.0]),
            4 => assert_eq!(percents, [0.0, 25.0, 50.0, 75.0]),
            _ => assert_eq!(percents, [0.0, 20.0, 40.0, 60.0, 80.0]),
        }
        assert!(rule.show_value && !rule.reverse && rule.custom.is_none());
    }
    assert_eq!(
        Rule::top_percent(10),
        Rule::TopBottom {
            rank: 10,
            percent: true,
            bottom: false
        }
    );
    assert_eq!(
        Rule::below_average(),
        Rule::Average {
            above: false,
            or_equal: false,
            std_dev: 0
        }
    );
}

// ---- speed ---------------------------------------------------------------------------------------------------------------

/// A sheet of 100 000 cells, every one of them under a rule, scrolled
/// through a window of 50 rows by 20 columns.
#[test]
fn a_hundred_thousand_formatted_cells_scroll_smoothly() {
    const ROWS: u32 = 5000;
    let mut w = Workbook::new(Locale::en());
    for r in 0..ROWS {
        for c in 0..20u32 {
            // Numbers spread without pattern, some of them repeated.
            let n = (r.wrapping_mul(2654435761).wrapping_add(c.wrapping_mul(40503)) >> 7) % 1000;
            let value = if c == 18 {
                Value::text(format!("item {n}"))
            } else {
                Value::Number(n as f64 - 200.0)
            };
            w.set_value(0, r, c, value);
        }
    }
    w.commit("cells");
    let on = |cols: &str, rule: Rule| {
        let (a, b) = cols.split_once(':').unwrap();
        CondFormat::new(rects(&format!("{a}1:{b}{ROWS}")), rule).with_format(fill(RED))
    };
    let rules = vec![
        on("A:E", Rule::ColorScale(ScalePreset::GreenYellowRed.scale())),
        on("F:H", Rule::DataBar(DataBar::preset(BarColor::Blue, true))),
        on("I:J", Rule::IconSet(IconRule::preset(IconSet::FiveArrows))),
        on("K:L", Rule::top_percent(10)),
        on(
            "M:M",
            Rule::Average {
                above: true,
                or_equal: false,
                std_dev: 1,
            },
        ),
        on("N:N", Rule::DuplicateValues),
        on("O:P", Rule::between(num(100.0), num(300.0))),
        on("Q:R", Rule::Formula(f("$A1>B1"))),
        on("S:S", Rule::text_contains(text("7"))),
        on("T:T", Rule::Formula(f("MOD(ROW(),2)=0"))),
        on("A:T", Rule::greater_than(f("$A$1+900"))),
    ];
    w.set_cond_formats(0, rules);
    w.commit("rules");

    // What the grid does for a frame: 1000 cells, asked for twice each.
    fn paint(w: &Workbook, top: u32) -> usize {
        let mut formatted = 0;
        for r in top..top + 50 {
            for c in 0..20 {
                formatted += w.display(0, r, c).adornment.is_some() as usize;
                formatted += result_at(w, 0, r, c).is_some() as usize;
            }
        }
        formatted
    }
    // The first frame works out everything the rules need from the ranges.
    let start = Instant::now();
    let formatted = paint(&w, 0);
    let first = start.elapsed();
    assert!(formatted > 300, "{formatted}");
    // The same frame again.
    let start = Instant::now();
    for _ in 0..20 {
        assert_eq!(paint(&w, 0), formatted);
    }
    let again = start.elapsed() / 20;
    // Scrolling to the bottom, ten rows a frame.
    let frames = (ROWS - 50) / 10;
    let start = Instant::now();
    let mut slowest = std::time::Duration::ZERO;
    for frame in 1..=frames {
        let one = Instant::now();
        paint(&w, frame * 10);
        slowest = slowest.max(one.elapsed());
    }
    let scrolling = start.elapsed() / frames;
    // After an edit everything is worked out again.
    w.set_value(0, 0, 0, Value::Number(5.0));
    w.commit("edit");
    let start = Instant::now();
    paint(&w, 2000);
    let after_edit = start.elapsed();
    eprintln!(
        "conditional formats over 100 000 cells, 2 x 1000 cells a frame: first frame {first:?}, same frame again \
         {again:?}, scrolling {scrolling:?} a frame (slowest {slowest:?}), first frame after an edit {after_edit:?}"
    );
    // A frame at 60 Hz lasts 16 ms; these bounds leave room for a busy
    // machine and an unoptimised build.
    assert!(again.as_millis() < 8, "repainting a frame took {again:?}");
    assert!(scrolling.as_millis() < 8, "scrolling took {scrolling:?} a frame");
    assert!(first.as_millis() < 1000, "the first frame took {first:?}");
    assert!(
        after_edit.as_millis() < 1000,
        "the first frame after an edit took {after_edit:?}"
    );
}

#[test]
fn the_number_span_ignores_text_and_blank_cells() {
    let wb = column(&["4", "abc", "", "-2.5", "=A1*3", "TRUE"]);
    assert_eq!(number_span(&wb, 0, &rects("A1:A6")), Some((-2.5, 12.0)));
    assert_eq!(number_span(&wb, 0, &rects("A2:A3,A6")), None);
    // Whole columns and cells counted twice change nothing.
    assert_eq!(number_span(&wb, 0, &rects("A:A,A1:A2")), Some((-2.5, 12.0)));
    assert_eq!(number_span(&wb, 5, &rects("A1:A6")), None);
}
