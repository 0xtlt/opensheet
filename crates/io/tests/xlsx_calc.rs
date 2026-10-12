//! The calculation options of a workbook in `.xlsx` files (`<calcPr>`):
//! calculation mode and iterative calculation.

mod common;

use common::{assert_well_formed, assert_workbook_order, enter, package, part, value};
use opensheet_engine::{CalcMode, IterativeCalc, Locale, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn calc_pr(bytes: &[u8]) -> String {
    let xml = part(bytes, "xl/workbook.xml");
    assert_well_formed("xl/workbook.xml", &xml);
    assert_workbook_order(&xml);
    let start = xml.find("<calcPr").expect("a calcPr element");
    let end = start + xml[start..].find("/>").expect("closed") + 2;
    xml[start..end].to_string()
}

#[test]
fn the_defaults_are_left_out() {
    let wb = Workbook::new(Locale::en());
    let bytes = to_bytes(&wb);
    assert_eq!(
        calc_pr(&bytes),
        r#"<calcPr calcId="191029" fullCalcOnLoad="1"/>"#
    );
    let back = load(&bytes);
    assert_eq!(back.calc_mode(), CalcMode::Automatic);
    assert_eq!(back.iterative_calc(), IterativeCalc::default());
    assert!(!back.modified);
}

#[test]
fn calculation_options_round_trip() {
    let mut wb = Workbook::new(Locale::en());
    // A counter: only iterative calculation gives it a value.
    wb.set_iterative_calc(IterativeCalc {
        enabled: true,
        max_iterations: 25,
        max_change: 0.05,
    });
    enter(&mut wb, 0, "A1", "=A1+1");
    enter(&mut wb, 0, "B1", "=A1*2");
    wb.commit("formulas");
    wb.set_calc_mode(CalcMode::Manual);
    assert_eq!(value(&wb, 0, "A1"), Value::Number(25.0));

    let bytes = to_bytes(&wb);
    assert_eq!(
        calc_pr(&bytes),
        r#"<calcPr calcId="191029" calcMode="manual" iterate="1" iterateCount="25" iterateDelta="0.05" fullCalcOnLoad="1"/>"#
    );
    let back = load(&bytes);
    assert_eq!(back.calc_mode(), CalcMode::Manual);
    assert_eq!(back.iterative_calc(), wb.iterative_calc());
    assert!(!back.modified && !back.circular && !back.needs_calculation());
    // Loading calculates once, with the file's options.
    assert_eq!(value(&back, 0, "A1"), Value::Number(50.0));
    assert_eq!(value(&back, 0, "B1"), Value::Number(100.0));
    // And a second trip changes nothing.
    assert_eq!(calc_pr(&to_bytes(&back)), calc_pr(&bytes));

    wb.set_calc_mode(CalcMode::AutomaticExceptTables);
    wb.set_iterative_calc(IterativeCalc::default());
    let bytes = to_bytes(&wb);
    assert_eq!(
        calc_pr(&bytes),
        r#"<calcPr calcId="191029" calcMode="autoNoTable" fullCalcOnLoad="1"/>"#
    );
    let back = load(&bytes);
    assert_eq!(back.calc_mode(), CalcMode::AutomaticExceptTables);
    assert!(back.circular);
}

#[test]
fn calculation_options_as_excel_writes_them() {
    let workbook = |calc_pr: &str| {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>{calc_pr}</workbook>"#
        )
    };
    let sheet = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><f>A1+1</f><v>7</v></c></row></sheetData></worksheet>"#;
    let open = |calc_pr: &str| {
        let bytes = package(&[
            ("xl/workbook.xml", &workbook(calc_pr)),
            ("xl/worksheets/sheet1.xml", sheet),
        ]);
        load(&bytes)
    };
    let wb = open(
        r#"<calcPr calcId="191029" calcMode="manual" iterate="1" iterateCount="10" iterateDelta="1E-4"/>"#,
    );
    assert_eq!(wb.calc_mode(), CalcMode::Manual);
    let expected = IterativeCalc {
        enabled: true,
        max_iterations: 10,
        max_change: 0.0001,
    };
    assert_eq!(wb.iterative_calc(), expected);
    // Iterating goes on from the value the file holds.
    assert_eq!(value(&wb, 0, "A1"), Value::Number(17.0));
    // No element, or values that make no sense: the defaults.
    let wb = open("");
    assert_eq!(
        (wb.calc_mode(), wb.iterative_calc()),
        (CalcMode::Automatic, IterativeCalc::default())
    );
    assert!(wb.circular);
    let wb =
        open(r#"<calcPr calcMode="whenever" iterateCount="0" iterateDelta="-3" iterate="maybe"/>"#);
    assert_eq!(
        (wb.calc_mode(), wb.iterative_calc()),
        (CalcMode::Automatic, IterativeCalc::default())
    );
}
