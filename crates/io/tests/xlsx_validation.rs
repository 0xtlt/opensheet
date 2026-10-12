//! Data validation in `.xlsx` files: our own round trip, rules as Excel
//! writes them (both the plain form and the one of the extension list),
//! and damaged input.

mod common;

use common::{assert_well_formed, assert_worksheet_order, at, enter, package, part};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::validation::{
    AlertStyle, EntryCheck, ImeMode, Operator, Validation, ValidationKind,
};
use opensheet_engine::{Locale, Rect, Value, Workbook};
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

fn rects(names: &str) -> Vec<Rect> {
    names
        .split(' ')
        .map(|name| Rect::parse(name).unwrap())
        .collect()
}

/// Give `ranges` (the first cell being the active one) a rule filled the
/// way the dialog fills it.
fn validate(
    wb: &mut Workbook,
    ranges: &str,
    kind: ValidationKind,
    operator: Operator,
    operands: (&str, &str),
    tune: impl FnOnce(&mut Validation),
) {
    let ranges = rects(ranges);
    let mut rule = Validation {
        kind,
        operator,
        ..Validation::default()
    };
    rule.set_operands(operands.0, operands.1, Locale::en())
        .unwrap_or_else(|e| panic!("{operands:?}: {}", e.message(Locale::en())));
    tune(&mut rule);
    let active = (ranges[0].r0, ranges[0].c0);
    assert!(wb.set_validation(0, &ranges, active, rule));
}

/// A workbook with every kind of rule.
fn sample() -> Workbook {
    use Operator::*;
    use ValidationKind::*;
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Form"));
    assert!(wb.rename_sheet(1, "Lists"));
    assert!(wb.rename_sheet(2, "Année 2025"));
    for (cell, text) in [("A1", "red"), ("A2", "green"), ("A3", "blue")] {
        enter(&mut wb, 1, cell, text);
    }
    enter(&mut wb, 1, "C1", "0.5");
    enter(&mut wb, 1, "C2", "9.5");
    for (cell, text) in [("H1", "north"), ("H2", "south"), ("H3", "west")] {
        enter(&mut wb, 0, cell, text);
    }
    enter(&mut wb, 2, "A1", "taken");
    wb.define_name("Limit", parse_formula("42", en).unwrap());

    validate(&mut wb, "A2:A20", Whole, Between, ("1", "10"), |rule| {
        rule.error_title = "Out of range".into();
        rule.error_message = "Between 1 & 10, \"please\".\nTry <again>.".into();
        rule.input_title = "Quantity".into();
        rule.input_message = "A whole number\tfrom 1 to 10\n_x000A_ is not a line break".into();
    });
    validate(
        &mut wb,
        "B2:B20",
        Decimal,
        GreaterThan,
        ("0.5", ""),
        |rule| {
            rule.error_style = AlertStyle::Warning;
        },
    );
    validate(
        &mut wb,
        "C2:C20 E5",
        List,
        Between,
        ("Yes,No,Maybe", ""),
        |rule| rule.in_cell_dropdown = false,
    );
    validate(&mut wb, "D2:D20", List, Between, ("=$H$1:$H$3", ""), |_| {});
    validate(
        &mut wb,
        "E2:E4",
        List,
        Between,
        ("=Lists!$A$1:$A$3", ""),
        |rule| rule.input_message = "Pick a colour".into(),
    );
    validate(
        &mut wb,
        "F2:F20",
        Date,
        Between,
        ("1/1/2025", "12/31/2025"),
        |rule| rule.error_style = AlertStyle::Information,
    );
    validate(&mut wb, "G2:G20", Time, LessOrEqual, ("12:00", ""), |_| {});
    validate(
        &mut wb,
        "I2:I20",
        TextLength,
        NotBetween,
        ("3", "5"),
        |rule| {
            rule.allow_blank = false;
            rule.show_error = false;
            rule.show_input = false;
        },
    );
    validate(
        &mut wb,
        "J2:J20",
        Custom,
        Between,
        ("=AND(ISNUMBER(J2),J2>$A$2)", ""),
        |_| {},
    );
    validate(
        &mut wb,
        "K2:K20",
        Custom,
        Between,
        ("=COUNTIF('Année 2025'!$A:$A,K2)=0", ""),
        |_| {},
    );
    validate(&mut wb, "L2", Any, Between, ("", ""), |rule| {
        rule.input_title = "Free".into();
        rule.input_message = "Anything goes here".into();
    });
    validate(&mut wb, "M2:M20", Whole, Equal, ("=Limit", ""), |_| {});
    validate(&mut wb, "N2:N20", Decimal, NotEqual, ("0", ""), |rule| {
        rule.ime_mode = ImeMode::Hiragana;
    });
    validate(
        &mut wb,
        "O2:O5",
        Custom,
        Between,
        ("=ISNUMBER(XMATCH(O2,$H$1:$H$3))", ""),
        |_| {},
    );
    validate(
        &mut wb,
        "P2 P4:Q9",
        Decimal,
        Between,
        ("=Lists!$C$1", "=Lists!$C$2"),
        |_| {},
    );
    validate(&mut wb, "R2:R9", Whole, GreaterOrEqual, ("0", ""), |_| {});
    validate(&mut wb, "S2:S9", Whole, LessThan, ("=$A2*2", ""), |_| {});
    wb.commit("rules");
    wb
}

/// The rules of a sheet, in an order that does not depend on the file.
fn sorted(wb: &Workbook, sheet: usize) -> Vec<Validation> {
    let mut rules = wb.sheets[sheet].validations.clone();
    rules.sort_by_key(|rule| rule.anchor());
    rules
}

#[test]
fn rules_round_trip() {
    let original = sample();
    assert_eq!(original.sheets[0].validations.len(), 17);
    let bytes = to_bytes(&original);
    common::keep_sample("validation.xlsx", &bytes);
    let loaded = load(&bytes);
    assert_eq!(sorted(&original, 0), sorted(&loaded, 0));
    assert!(loaded.sheets[1].validations.is_empty());
    assert!(!loaded.modified && !loaded.can_undo());

    // The rules are alive, not just stored.
    let mut wb = loaded;
    let (row, col) = at("A2");
    assert_eq!(wb.check_entry(0, row, col, "7"), EntryCheck::Accepted);
    let EntryCheck::Invalid(alert) = wb.check_entry(0, row, col, "11") else {
        panic!("11 is not between 1 and 10");
    };
    assert_eq!(alert.title, "Out of range");
    assert_eq!(alert.message, "Between 1 & 10, \"please\".\nTry <again>.");
    assert_eq!(
        wb.input_message(0, row, col),
        Some((
            "Quantity",
            "A whole number\tfrom 1 to 10\n_x000A_ is not a line break"
        ))
    );
    let items = |wb: &Workbook, cell: &str| -> Vec<String> {
        let (row, col) = at(cell);
        wb.validation_list(0, row, col)
            .unwrap()
            .into_iter()
            .map(|item| item.text)
            .collect()
    };
    assert_eq!(items(&wb, "C2"), ["Yes", "No", "Maybe"]);
    assert_eq!(items(&wb, "E5"), ["Yes", "No", "Maybe"]);
    assert_eq!(items(&wb, "D9"), ["north", "south", "west"]);
    assert_eq!(items(&wb, "E3"), ["red", "green", "blue"]);
    let (row, col) = at("C2");
    assert!(!wb.has_validation_dropdown(0, row, col));
    let (row, col) = at("E3");
    assert!(wb.has_validation_dropdown(0, row, col));
    let (row, col) = at("K5");
    assert!(matches!(
        wb.check_entry(0, row, col, "taken"),
        EntryCheck::Invalid(_)
    ));
    assert_eq!(wb.check_entry(0, row, col, "free"), EntryCheck::Accepted);
    // Relative references still read from the cell they are checked in.
    wb.set_value(0, 4, 0, Value::Number(3.0));
    wb.commit("A5");
    let (row, col) = at("S5");
    assert_eq!(wb.check_entry(0, row, col, "5"), EntryCheck::Accepted);
    assert!(matches!(
        wb.check_entry(0, row, col, "6"),
        EntryCheck::Invalid(_)
    ));

    // A second generation is the same file.
    let again = to_bytes(&load(&bytes));
    for name in ["xl/worksheets/sheet1.xml", "xl/workbook.xml"] {
        assert_eq!(part(&bytes, name), part(&again, name), "{name}");
    }
}

#[test]
fn rules_are_written_the_way_excel_writes_them() {
    let bytes = to_bytes(&sample());
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert_worksheet_order("sheet1.xml", &sheet);
    let contains = |text: &str| assert!(sheet.contains(text), "missing {text}\nin {sheet}");

    // Rules that stay on their sheet, with only the attributes that say
    // something.
    contains("</sheetData><dataValidations count=\"14\"><dataValidation ");
    contains(concat!(
        "<dataValidation type=\"whole\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" ",
        "errorTitle=\"Out of range\" error=\"Between 1 &amp; 10, &quot;please&quot;._x000A_Try &lt;again&gt;.\" ",
        "promptTitle=\"Quantity\" prompt=\"A whole number_x0009_from 1 to 10_x000A__x005F_x000A_ is not a line break\" ",
        "sqref=\"A2:A20\"><formula1>1</formula1><formula2>10</formula2></dataValidation>"
    ));
    contains(concat!(
        "<dataValidation type=\"decimal\" errorStyle=\"warning\" operator=\"greaterThan\" allowBlank=\"1\" ",
        "showInputMessage=\"1\" showErrorMessage=\"1\" sqref=\"B2:B20\"><formula1>0.5</formula1></dataValidation>"
    ));
    contains(concat!(
        "<dataValidation type=\"list\" allowBlank=\"1\" showDropDown=\"1\" showInputMessage=\"1\" ",
        "showErrorMessage=\"1\" sqref=\"C2:C20 E5\"><formula1>\"Yes,No,Maybe\"</formula1></dataValidation>"
    ));
    contains("sqref=\"D2:D20\"><formula1>$H$1:$H$3</formula1></dataValidation>");
    contains(concat!(
        "<dataValidation type=\"date\" errorStyle=\"information\" allowBlank=\"1\" showInputMessage=\"1\" ",
        "showErrorMessage=\"1\" sqref=\"F2:F20\"><formula1>45658</formula1><formula2>46022</formula2></dataValidation>"
    ));
    contains("<dataValidation type=\"time\" operator=\"lessThanOrEqual\" ");
    contains("sqref=\"G2:G20\"><formula1>0.5</formula1></dataValidation>");
    contains(concat!(
        "<dataValidation type=\"textLength\" operator=\"notBetween\" sqref=\"I2:I20\">",
        "<formula1>3</formula1><formula2>5</formula2></dataValidation>"
    ));
    contains(
        "<dataValidation type=\"custom\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" sqref=\"J2:J20\"><formula1>AND(ISNUMBER(J2),J2&gt;$A$2)</formula1></dataValidation>",
    );
    contains(
        "<dataValidation allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" promptTitle=\"Free\" prompt=\"Anything goes here\" sqref=\"L2\"/>",
    );
    contains(
        "operator=\"equal\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" sqref=\"M2:M20\"><formula1>Limit</formula1>",
    );
    contains("<dataValidation type=\"decimal\" imeMode=\"hiragana\" operator=\"notEqual\" ");
    contains("<formula1>ISNUMBER(_xlfn.XMATCH(O2,$H$1:$H$3))</formula1>");
    contains("operator=\"greaterThanOrEqual\"");
    contains("sqref=\"S2:S9\"><formula1>$A2*2</formula1>");

    // Rules that look at another sheet live in the extension list, the
    // last element of the worksheet.
    contains(concat!(
        "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" ",
        "uri=\"{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}\">",
        "<x14:dataValidations xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\" count=\"3\">"
    ));
    contains(concat!(
        "<x14:dataValidation type=\"list\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\" ",
        "prompt=\"Pick a colour\"><x14:formula1><xm:f>Lists!$A$1:$A$3</xm:f></x14:formula1>",
        "<xm:sqref>E2:E4</xm:sqref></x14:dataValidation>"
    ));
    contains(concat!(
        "<x14:formula1><xm:f>COUNTIF('Année 2025'!$A:$A,K2)=0</xm:f></x14:formula1>",
        "<xm:sqref>K2:K20</xm:sqref></x14:dataValidation>"
    ));
    contains(concat!(
        "<x14:formula1><xm:f>Lists!$C$1</xm:f></x14:formula1><x14:formula2><xm:f>Lists!$C$2</xm:f></x14:formula2>",
        "<xm:sqref>P2 P4:Q9</xm:sqref></x14:dataValidation>"
    ));
    assert!(sheet.ends_with("</x14:dataValidations></ext></extLst></worksheet>"));
    assert!(!sheet.contains("<formula1>Lists!") && !sheet.contains("type=\"none\""));

    // Sheets without rules say nothing about them.
    let other = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(!other.contains("dataValidation") && !other.contains("extLst"));
    assert_worksheet_order("sheet2.xml", &other);
}

#[test]
fn what_excel_would_refuse_is_not_written() {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    let long = "é".repeat(300);
    let rule = Validation {
        kind: ValidationKind::List,
        formula1: Some(std::sync::Arc::new(opensheet_engine::Expr::Text(
            (0..80)
                .map(|i| format!("choice {i}"))
                .collect::<Vec<_>>()
                .join(",")
                .into(),
        ))),
        input_title: long.clone(),
        input_message: long.clone(),
        error_title: long.clone(),
        error_message: long.clone(),
        ..Validation::default()
    };
    wb.sheets[0].validations.push(Validation {
        ranges: rects("A1:A5"),
        ..rule.clone()
    });
    // Rules that apply to nothing, or say nothing.
    wb.sheets[0].validations.push(rule);
    wb.sheets[0].validations.push(Validation {
        ranges: rects("B1"),
        ..Validation::default()
    });
    // A list on a sheet that was deleted since.
    let mut gone = Validation {
        kind: ValidationKind::List,
        ranges: rects("C1"),
        ..Validation::default()
    };
    gone.set_operands("=Sheet2!$A$1:$A$3", "", Locale::en())
        .unwrap();
    wb.sheets[0].validations.push(gone);
    assert!(wb.delete_sheet(1));
    wb.commit("delete");

    let bytes = to_bytes(&wb);
    common::keep_sample("validation-limits.xlsx", &bytes);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert!(sheet.contains("<dataValidations count=\"2\">"), "{sheet}");
    assert!(!sheet.contains("extLst"));
    let loaded = load(&bytes);
    let rules = &loaded.sheets[0].validations;
    assert_eq!(rules.len(), 2);
    // Excel takes titles and messages of 255 characters and reports a
    // file with longer ones as damaged; the same goes for a typed list.
    assert_eq!(rules[0].input_title.chars().count(), 255);
    assert_eq!(rules[0].error_title.chars().count(), 255);
    assert_eq!(rules[0].input_message.chars().count(), 255);
    assert_eq!(rules[0].error_message.chars().count(), 255);
    let (row, col) = at("A1");
    let items = loaded.validation_list(0, row, col).unwrap();
    assert_eq!(items.len(), 26, "the items that fit in 255 characters");
    assert_eq!(items.last().unwrap().text, "choice 25");
    // The sheet is gone: its cells are `#REF!`, which names no sheet.
    assert!(
        sheet.contains("sqref=\"C1\"><formula1>#REF!</formula1></dataValidation>"),
        "{sheet}"
    );
    assert_eq!(
        rules[1].formula1.as_ref().unwrap().to_text(Locale::en()),
        "#REF!"
    );
}

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn workbook_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {MAIN}><sheets><sheet name="Saisie" sheetId="1" r:id="rId1"/><sheet name="Listes" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="Villes">Listes!$A$1:$A$3</definedName></definedNames></workbook>"#
    )
}

const LISTS: &str = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Paris</t></is></c><c r="B1"><v>1.5</v></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>Lyon</t></is></c><c r="B2"><v>9.5</v></c></row>
<row r="3"><c r="A3" t="inlineStr"><is><t>Lille</t></is></c></row>
</sheetData></worksheet>"#;

/// A sheet as Excel 365 saves it: revision ids on every rule, attributes
/// left out when they have their default, line breaks escaped, a typed
/// list with its newer alternative, and the rules that refer to another
/// sheet in the extension list.
fn excel_sheet() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr xr2 xr3" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xmlns:xr2="http://schemas.microsoft.com/office/spreadsheetml/2015/revision2" xmlns:xr3="http://schemas.microsoft.com/office/spreadsheetml/2016/revision3" xr:uid="{{00000000-0001-0000-0000-000000000000}}">
<dimension ref="A1:H10"/>
<sheetViews><sheetView tabSelected="1" workbookViewId="0"><selection activeCell="G2" sqref="G2"/></sheetView></sheetViews>
<sheetFormatPr baseColWidth="10" defaultRowHeight="15" x14ac:dyDescent="0.25"/>
<sheetData><row r="1" spans="1:8" x14ac:dyDescent="0.25"><c r="A1" t="inlineStr"><is><t>Choix</t></is></c></row></sheetData>
<dataValidations count="6">
<dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" sqref="A2:A10 C2" xr:uid="{{00000000-0002-0000-0000-000000000000}}"><formula1>"Oui,Non"</formula1></dataValidation>
<dataValidation type="whole" errorStyle="warning" operator="greaterThanOrEqual" allowBlank="1" showInputMessage="1" showErrorMessage="1" errorTitle="Trop petit" error="Au moins 10._x000a_Continuer ?" promptTitle="Quantité" prompt="Un entier &gt;= 10" sqref="B2:B10" xr:uid="{{00000000-0002-0000-0000-000001000000}}"><formula1>10</formula1></dataValidation>
<dataValidation type="date" allowBlank="1" showInputMessage="1" showErrorMessage="1" sqref="C3:C10" xr:uid="{{00000000-0002-0000-0000-000002000000}}"><formula1>45658</formula1><formula2>46022</formula2></dataValidation>
<dataValidation type="custom" showInputMessage="1" showErrorMessage="1" sqref="D2:D10" xr:uid="{{00000000-0002-0000-0000-000003000000}}"><formula1>ISNUMBER(D2)</formula1></dataValidation>
<dataValidation type="list" allowBlank="1" showDropDown="1" showInputMessage="1" showErrorMessage="1" sqref="E2" xr:uid="{{00000000-0002-0000-0000-000004000000}}"><mc:AlternateContent xmlns:x12ac="http://schemas.microsoft.com/office/spreadsheetml/2011/1/ac" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="x12ac"><x12ac:list>1,"2,5",3</x12ac:list></mc:Choice><mc:Fallback><formula1>"1,2,5,3"</formula1></mc:Fallback></mc:AlternateContent></dataValidation>
<dataValidation allowBlank="1" showInputMessage="1" showErrorMessage="1" promptTitle="Note" prompt="Libre" sqref="F2" xr:uid="{{00000000-0002-0000-0000-000005000000}}"/>
</dataValidations>
<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>
<extLst><ext uri="{{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:dataValidations count="3" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">
<x14:dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" xr:uid="{{00000000-0002-0000-0000-000006000000}}"><x14:formula1><xm:f>Listes!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>G2:G10</xm:sqref></x14:dataValidation>
<x14:dataValidation type="decimal" allowBlank="1" showInputMessage="1" showErrorMessage="1" errorTitle="Hors limites" xr:uid="{{00000000-0002-0000-0000-000007000000}}"><x14:formula1><xm:f>Listes!$B$1</xm:f></x14:formula1><x14:formula2><xm:f>Listes!$B$2</xm:f></x14:formula2><xm:sqref>H2:H10 J2</xm:sqref></x14:dataValidation>
<x14:dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" xr:uid="{{00000000-0002-0000-0000-000008000000}}"><x14:formula1><xm:f>[1]Autre!$A$1:$A$3</xm:f></x14:formula1><xm:sqref>K2</xm:sqref></x14:dataValidation>
</x14:dataValidations></ext><ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:sparklineGroup><x14:sparklines><x14:sparkline><xm:f>Listes!B1:B2</xm:f><xm:sqref>Z1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup></x14:sparklineGroups></ext></extLst>
</worksheet>"#
    )
}

fn excel_file() -> Vec<u8> {
    package(&[
        ("xl/workbook.xml", &workbook_xml()),
        ("xl/worksheets/sheet1.xml", &excel_sheet()),
        ("xl/worksheets/sheet2.xml", LISTS),
    ])
}

#[test]
fn rules_from_excel() {
    let en = Locale::en();
    let mut wb = load(&excel_file());
    let rules = wb.sheets[0].validations.clone();
    let names: Vec<String> = rules
        .iter()
        .map(|rule| {
            let ranges: Vec<String> = rule.ranges.iter().map(Rect::name).collect();
            ranges.join(" ")
        })
        .collect();
    assert_eq!(
        names,
        [
            "A2:A10 C2",
            "B2:B10",
            "C3:C10",
            "D2:D10",
            "E2",
            "F2",
            "G2:G10",
            "H2:H10 J2"
        ],
        "the rule that reads another workbook is dropped, alone"
    );
    let formula = |rule: &Validation, second: bool| {
        let expr = if second {
            &rule.formula2
        } else {
            &rule.formula1
        };
        expr.as_ref().map(|e| e.to_text(en)).unwrap_or_default()
    };

    let list = &rules[0];
    assert_eq!(list.kind, ValidationKind::List);
    assert_eq!(formula(list, false), "\"Oui,Non\"");
    assert!(list.allow_blank && list.in_cell_dropdown && list.show_input && list.show_error);
    assert_eq!(list.error_style, AlertStyle::Stop);

    let whole = &rules[1];
    assert_eq!(
        (whole.kind, whole.operator, whole.error_style),
        (
            ValidationKind::Whole,
            Operator::GreaterOrEqual,
            AlertStyle::Warning
        )
    );
    assert_eq!(formula(whole, false), "10");
    assert_eq!(whole.formula2, None);
    assert_eq!(whole.error_title, "Trop petit");
    assert_eq!(whole.error_message, "Au moins 10.\nContinuer ?");
    assert_eq!(whole.input_title, "Quantité");
    assert_eq!(whole.input_message, "Un entier >= 10");

    let date = &rules[2];
    assert_eq!(
        (date.kind, date.operator),
        (ValidationKind::Date, Operator::Between)
    );
    assert_eq!(
        (formula(date, false), formula(date, true)),
        ("45658".into(), "46022".into())
    );

    let custom = &rules[3];
    assert_eq!(custom.kind, ValidationKind::Custom);
    assert!(!custom.allow_blank, "an absent allowBlank is off");
    assert_eq!(formula(custom, false), "ISNUMBER(D2)");

    // The fallback of the newer way to store a typed list.
    let typed = &rules[4];
    assert_eq!(formula(typed, false), "\"1,2,5,3\"");
    assert!(!typed.in_cell_dropdown, "showDropDown hides the arrow");

    let note = &rules[5];
    assert_eq!(note.kind, ValidationKind::Any);
    assert_eq!(
        (note.input_title.as_str(), note.input_message.as_str()),
        ("Note", "Libre")
    );
    assert_eq!(note.formula1, None);

    let cities = &rules[6];
    assert_eq!(formula(cities, false), "Listes!$A$1:$A$3");
    let bounds = &rules[7];
    assert_eq!(
        (bounds.kind, bounds.operator),
        (ValidationKind::Decimal, Operator::Between)
    );
    assert_eq!(
        (formula(bounds, false), formula(bounds, true)),
        ("Listes!$B$1".into(), "Listes!$B$2".into())
    );
    assert_eq!(bounds.error_title, "Hors limites");

    // And they work.
    let (row, col) = at("G5");
    let items: Vec<String> = wb
        .validation_list(0, row, col)
        .unwrap()
        .into_iter()
        .map(|item| item.text)
        .collect();
    assert_eq!(items, ["Paris", "Lyon", "Lille"]);
    assert!(matches!(
        wb.check_entry(0, row, col, "Rome"),
        EntryCheck::Invalid(_)
    ));
    assert_eq!(wb.check_entry(0, row, col, "Lyon"), EntryCheck::Accepted);
    let (row, col) = at("J2");
    assert_eq!(wb.check_entry(0, row, col, "5"), EntryCheck::Accepted);
    assert!(matches!(
        wb.check_entry(0, row, col, "10"),
        EntryCheck::Invalid(_)
    ));
    // Nothing else of the extension list leaked into the sheet.
    assert!(wb.sheets[0].cells.len() == 1);

    // Saved again, each rule is back in the form it came in.
    let bytes = to_bytes(&wb);
    common::keep_sample("validation-from-excel.xlsx", &bytes);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1.xml", &sheet);
    assert!(sheet.contains("<dataValidations count=\"6\">"), "{sheet}");
    assert!(sheet.contains("<x14:dataValidations xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\" count=\"2\">"));
    assert!(sheet.contains("error=\"Au moins 10._x000A_Continuer ?\""));
    assert_eq!(load(&bytes).sheets[0].validations, rules);
}

#[test]
fn damaged_rules_cost_nothing_else() {
    let sheet = excel_sheet();
    // Cut the worksheet short everywhere inside the rules and the
    // extension list: the cells before them are always there.
    let start = sheet.find("<dataValidations").unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for cut in (start..sheet.len())
        .step_by(5)
        .filter(|i| sheet.is_char_boundary(*i))
    {
        let bytes = package(&[
            ("xl/workbook.xml", &workbook_xml()),
            ("xl/worksheets/sheet1.xml", &sheet[..cut]),
            ("xl/worksheets/sheet2.xml", LISTS),
        ]);
        let wb = load(&bytes);
        assert_eq!(wb.sheets.len(), 2);
        assert_eq!(common::value(&wb, 0, "A1"), Value::text("Choix"));
        let count = wb.sheets[0].validations.len();
        assert!(count <= 8);
        seen.insert(count);
        // Whatever was read can be written.
        assert_well_formed(
            "sheet1.xml",
            &part(&to_bytes(&wb), "xl/worksheets/sheet1.xml"),
        );
    }
    assert_eq!(seen.len(), 9, "rules are lost one at a time: {seen:?}");

    // Nonsense in every attribute.
    let nonsense = format!(
        r#"<worksheet {MAIN}><sheetData/><dataValidations count="-3" disablePrompts="maybe" xWindow="x">
<dataValidation type="" sqref="A1"/>
<dataValidation type="list" allowBlank="2" showDropDown="yes" sqref="A2:A1 $B$7:$A$3"><formula1>"a,b"</formula1><formula2/></dataValidation>
<dataValidation type="whole" operator="between" sqref="A9999999 XFD1048576"><formula1>1</formula1><formula2>2</formula2></dataValidation>
<dataValidation type="decimal" operator="lessThan" sqref="C1" prompt="_x00" promptTitle="&#0;&bogus;"><formula1>1E400</formula1></dataValidation>
<dataValidation type="textLength" operator="equal" sqref="{}"><formula1>3</formula1></dataValidation>
<dataValidation type="custom" sqref="D1"><formula1>{}</formula1></dataValidation>
<dataValidations><dataValidation type="list" sqref="E1"><formula1>"nested"</formula1></dataValidation></dataValidations>
</dataValidations><extLst><ext/><ext uri="x"><dataValidations/></ext><ext><x14:dataValidations><x14:dataValidation type="list"><x14:formula1><xm:f>"x,y"</xm:f></x14:formula1><xm:sqref> </xm:sqref></x14:dataValidation><x14:dataValidation type="list"><x14:formula1><xm:f>"x,y"</xm:f></x14:formula1><xm:sqref>F1</xm:sqref><xm:sqref>F2</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst></worksheet>"#,
        "A1 ".repeat(5000),
        "(".repeat(5000),
    );
    let bytes = package(&[
        ("xl/workbook.xml", &workbook_xml()),
        ("xl/worksheets/sheet1.xml", &nonsense),
        ("xl/worksheets/sheet2.xml", LISTS),
    ]);
    let wb = load(&bytes);
    let names: Vec<String> = wb.sheets[0]
        .validations
        .iter()
        .map(|rule| format!("{:?} {}", rule.kind, rule.ranges[0].name()))
        .collect();
    assert_eq!(
        names,
        [
            "List A1:A2",
            "Whole XFD1048576",
            "Decimal C1",
            "TextLength A1",
            "List F2"
        ]
    );
    assert_eq!(wb.sheets[0].validations[0].ranges, rects("A1:A2 A3:B7"));
    let reloaded = load(&to_bytes(&wb));
    assert_eq!(reloaded.sheets[0].validations.len(), 5);
}
