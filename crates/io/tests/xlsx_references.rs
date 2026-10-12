//! The reference syntaxes and the defined names through `.xlsx` files:
//! what is written, what comes back, and what files written by Excel hold.

mod common;

use common::{assert_well_formed, enter, formula, package, part, value};
use opensheet_engine::names::NameScope;
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{ErrorKind, Locale, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn reloaded(wb: &Workbook, locale: &'static Locale) -> Workbook {
    read_xlsx(Cursor::new(to_bytes(wb)), locale).unwrap()
}

/// (cell on Summary, formula as typed in English, value)
const FORMULAS: [(&str, &str, f64); 17] = [
    ("B1", "=SUM(D1#)", 6.0),
    ("B2", "=SUM(Data!F1#)+ROWS(Data!F1#)", 32.0),
    ("B3", "=SUM(Data:More!A1)", 11.0),
    ("B4", "=AVERAGE('Data:Last one'!A1:A3)", 246.0 / 9.0),
    ("B5", "=SUM((Data!A1:A3,Data!C1:C3))", 66.0),
    ("B6", "=SUM(Data!A1:C3 Data!C2:D4)", 50.0),
    ("B7", "=@Data!A1:A20", 7.0),
    (
        "B8",
        "=AREAS((Data!A1,Data!C1,Data!A1:C3))+INDEX((Data!A1:A3,Data!C1:C3),2,1,2)",
        23.0,
    ),
    ("B9", "=Rate*100+Data!Local+Both", 127.0),
    ("B10", "=SUM(Pair)+More!Local", 122.0),
    ("B11", "=COUNT(Data:More!A:A)+SHEETS(Data:More!A1)", 9.0),
    ("B12", "=LARGE((Data!A1:A3,Data!C1:C3),2)", 20.0),
    ("B13", "=IFERROR(SUM(Data!A1:A3 Data!C1:C3),-1)", -1.0),
    ("B14", "=SUM(Data!A1:A3 Data!A2:A9,(Data!C1,Data!C3))", 45.0),
    ("B15", "=SUM(@Data!C1:C99,Above)", 190.0),
    ("B16", "=MAX(Data!$A$1:$A$3 Data!$1:$2)", 2.0),
    // A union across sheets is an error (`#VALUE!`), in Excel too.
    (
        "B17",
        "=IFERROR(SUM((Data!A1,More!A1)),-2)+IFERROR(SUM(Cross),-3)",
        -5.0,
    ),
];

/// A workbook using every reference syntax files can hold without tables.
fn sample(locale: &'static Locale) -> Workbook {
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    for i in 1..4 {
        wb.add_sheet(i);
    }
    for (i, name) in ["Summary", "Data", "More", "Last one"].iter().enumerate() {
        assert!(wb.rename_sheet(i, name));
    }
    for sheet in 1..4 {
        for i in 1..=3 {
            let n = i * [0, 1, 10, 30][sheet];
            enter(&mut wb, sheet, &format!("A{i}"), &n.to_string());
            enter(&mut wb, sheet, &format!("C{i}"), &(10 * n).to_string());
        }
    }
    enter(&mut wb, 1, "A7", "7");
    enter(&mut wb, 1, "C15", "150");
    enter(&mut wb, 1, "F1", "=SEQUENCE(2,2)*A3");
    enter(&mut wb, 0, "D1", "=SEQUENCE(3)");

    let name = |text: &str| parse_formula(text, en).unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Rate",
        name("0.2"),
        "VAT, as a fraction",
    )
    .unwrap();
    wb.add_name(NameScope::Workbook, "Pair", name("Data!$A$1,Data!$C$1"), "")
        .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Cross",
        name("Data!$A$1,More!$A$1"),
        "",
    )
    .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Both",
        name("SUM(Data:More!$A$1)"),
        "two sheets & \"quotes\"",
    )
    .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Local",
        name("1000"),
        "hidden behind the sheets' own",
    )
    .unwrap();
    wb.add_name(
        NameScope::Sheet(1),
        "Local",
        name("Data!$C$1*10-4"),
        "of Data",
    )
    .unwrap();
    wb.add_name(NameScope::Sheet(2), "Local", name("More!$C$1+11"), "")
        .unwrap();
    // "The cell above mine, in column A of Data", defined from B2.
    let above = wb.parse_name_formula("=Data!$A1+40", 0, 1, 1).unwrap();
    wb.add_name(NameScope::Workbook, "Above", above, "")
        .unwrap();
    for (cell, text, _) in FORMULAS {
        enter(&mut wb, 0, cell, text);
    }
    wb.commit("setup");
    wb.locale = locale;
    wb
}

fn check(wb: &Workbook) {
    for (cell, text, expected) in FORMULAS {
        assert_eq!(formula(wb, 0, cell).as_deref(), Some(&text[1..]), "{cell}");
        assert_eq!(
            value(wb, 0, cell),
            Value::Number(expected),
            "{cell}: {text}"
        );
    }
    let names: Vec<(NameScope, &str, String, &str)> = wb
        .all_names()
        .into_iter()
        .map(|(scope, def)| {
            (
                scope,
                def.name.as_str(),
                def.expr.to_text(Locale::en()),
                def.comment.as_str(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            (
                NameScope::Workbook,
                "Above",
                "Data!$A1048576+40".to_string(),
                ""
            ),
            (
                NameScope::Workbook,
                "Both",
                "SUM(Data:More!$A$1)".to_string(),
                "two sheets & \"quotes\""
            ),
            (
                NameScope::Workbook,
                "Cross",
                "Data!$A$1,More!$A$1".to_string(),
                ""
            ),
            (
                NameScope::Workbook,
                "Local",
                "1000".to_string(),
                "hidden behind the sheets' own"
            ),
            (
                NameScope::Sheet(1),
                "Local",
                "Data!$C$1*10-4".to_string(),
                "of Data"
            ),
            (NameScope::Sheet(2), "Local", "More!$C$1+11".to_string(), ""),
            (
                NameScope::Workbook,
                "Pair",
                "Data!$A$1,Data!$C$1".to_string(),
                ""
            ),
            (
                NameScope::Workbook,
                "Rate",
                "0.2".to_string(),
                "VAT, as a fraction"
            ),
        ]
    );
}

#[test]
fn every_syntax_round_trips() {
    let original = sample(Locale::en());
    check(&original);
    let loaded = reloaded(&original, Locale::en());
    check(&loaded);
    // Once more, to catch anything the first save normalised.
    check(&reloaded(&loaded, Locale::en()));
    // The file is the same whatever the language of the workbook, and
    // reads back into any language.
    let french = sample(Locale::fr());
    assert_eq!(
        part(&to_bytes(&french), "xl/worksheets/sheet1.xml"),
        part(&to_bytes(&original), "xl/worksheets/sheet1.xml")
    );
    let loaded = reloaded(&french, Locale::fr());
    check(&loaded);
    assert_eq!(
        loaded.formula_text(0, 7, 1).unwrap(),
        "=ZONES((Data!A1;Data!C1;Data!A1:C3))+INDEX((Data!A1:A3;Data!C1:C3);2;1;2)"
    );
    assert_eq!(loaded.formula_text(0, 6, 1).unwrap(), "=@Data!A1:A20");
    assert_eq!(loaded.formula_text(0, 0, 1).unwrap(), "=SOMME(D1#)");
}

#[test]
fn what_the_file_holds() {
    let bytes = to_bytes(&sample(Locale::en()));
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1", &sheet);
    for text in [
        "SUM(_xlfn.ANCHORARRAY(D1))",
        "SUM(_xlfn.ANCHORARRAY(Data!F1))+ROWS(_xlfn.ANCHORARRAY(Data!F1))",
        "SUM(Data:More!A1)",
        "AVERAGE('Data:Last one'!A1:A3)",
        "SUM((Data!A1:A3,Data!C1:C3))",
        "SUM(Data!A1:C3 Data!C2:D4)",
        "_xlfn.SINGLE(Data!A1:A20)",
        "SUM(_xlfn.SINGLE(Data!C1:C99),Above)",
    ] {
        assert!(sheet.contains(&format!(">{text}</f>")), "{text} in {sheet}");
    }
    // The operators that only exist in dynamic array formulas mark the
    // cell as one; the others are plain formulas.
    assert!(
        sheet.contains(
            "<c r=\"B1\" cm=\"1\"><f t=\"array\" ref=\"B1\">SUM(_xlfn.ANCHORARRAY(D1))</f>"
        ),
        "{sheet}"
    );
    assert!(
        sheet.contains("<c r=\"B3\"><f>SUM(Data:More!A1)</f>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<c r=\"B5\"><f>SUM((Data!A1:A3,Data!C1:C3))</f>"),
        "{sheet}"
    );

    let workbook = part(&bytes, "xl/workbook.xml");
    assert_well_formed("workbook", &workbook);
    let names = &workbook
        [workbook.find("<definedNames>").unwrap()..workbook.find("</definedNames>").unwrap()];
    assert_eq!(
        names,
        concat!(
            "<definedNames>",
            "<definedName name=\"Above\">Data!$A1048576+40</definedName>",
            "<definedName name=\"Both\" comment=\"two sheets &amp; &quot;quotes&quot;\">SUM(Data:More!$A$1)</definedName>",
            "<definedName name=\"Cross\">Data!$A$1,More!$A$1</definedName>",
            "<definedName name=\"Local\" comment=\"hidden behind the sheets' own\">1000</definedName>",
            "<definedName name=\"Local\" comment=\"of Data\" localSheetId=\"1\">Data!$C$1*10-4</definedName>",
            "<definedName name=\"Local\" localSheetId=\"2\">More!$C$1+11</definedName>",
            "<definedName name=\"Pair\">Data!$A$1,Data!$C$1</definedName>",
            "<definedName name=\"Rate\" comment=\"VAT, as a fraction\">0.2</definedName>",
        )
    );
}

#[test]
fn after_a_sheet_is_deleted() {
    let mut wb = sample(Locale::en());
    assert!(wb.delete_sheet(2));
    wb.commit("delete More");
    assert_eq!(formula(&wb, 0, "B3").unwrap(), "SUM(Data!A1)");
    assert_eq!(
        formula(&wb, 0, "B17").unwrap(),
        "IFERROR(SUM((Data!A1,#REF!A1)),-2)+IFERROR(SUM(Cross),-3)"
    );
    assert_eq!(formula(&wb, 0, "B10").unwrap(), "SUM(Pair)+#REF!");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    // Nothing in the file names the missing sheet.
    assert!(!sheet.contains("More") && !part(&bytes, "xl/workbook.xml").contains("More"));
    assert!(
        sheet.contains("<f>IFERROR(SUM((Data!A1,#REF!)),-2)+IFERROR(SUM(Cross),-3)</f>"),
        "{sheet}"
    );
    assert!(sheet.contains("<f>AVERAGE('Data:Last one'!A1:A3)</f>"));
    let loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
    assert_eq!(value(&loaded, 0, "B3"), Value::Number(1.0));
    assert_eq!(value(&loaded, 0, "B4"), Value::Number(31.0));
    assert_eq!(value(&loaded, 0, "B10"), Value::Error(ErrorKind::Ref));
    assert_eq!(value(&loaded, 0, "B17"), Value::Number(-5.0));
    assert_eq!(
        loaded.names["CROSS"].expr.to_text(Locale::en()),
        "Data!$A$1,#REF!"
    );
    assert_eq!(
        loaded.names["BOTH"].expr.to_text(Locale::en()),
        "SUM(Data!$A$1)"
    );
    // The names of the sheets after the deleted one moved up with them.
    let scopes: Vec<NameScope> = loaded
        .all_names()
        .into_iter()
        .filter(|(_, def)| def.name == "Local")
        .map(|(scope, _)| scope)
        .collect();
    assert_eq!(scopes, [NameScope::Workbook, NameScope::Sheet(1)]);
}

#[test]
fn files_from_excel() {
    // What Excel itself writes: the function spellings of `#` and `@`, the
    // long form of structured references, what a deleted sheet leaves, and
    // names that belong to a sheet.
    let workbook = r#"<workbook><sheets>
<sheet name="First" sheetId="1" r:id="rId1"/><sheet name="Chart" sheetId="2" r:id="rId2"/><sheet name="Second" sheetId="3" r:id="rId3"/></sheets>
<definedNames>
<definedName name="Area" comment="two blocks">First!$A$1:$A$2,First!$C$2</definedName>
<definedName name="Scoped" localSheetId="2" comment="on Second">Second!$A$1*2</definedName>
<definedName name="Scoped" localSheetId="0">First!$A$2*2</definedName>
<definedName name="Scoped">-1</definedName>
<definedName name="OnChart" localSheetId="1">5</definedName>
<definedName name="Beyond" localSheetId="9">6</definedName>
<definedName name="Lost">#REF!$A$1</definedName>
<definedName name="Deep">First:Second!$A$1</definedName>
</definedNames></workbook>"#;
    let rels = r#"<Relationships>
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/sheet1.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    let first = r#"<worksheet><sheetData>
<row r="1"><c r="A1"><v>1</v></c><c r="C1" cm="1"><f t="array" ref="C1:C3">_xlfn.SEQUENCE(3)</f><v>1</v></c>
 <c r="E1"><f>SUM(_xlfn.ANCHORARRAY(C1))</f><v>0</v></c>
 <c r="F1"><f>#REF!A1+1</f><v>0</v></c>
 <c r="G1"><f>SUM(First:Second!A1,(A1:A2,C2),A1:A2 A2:B2)</f><v>0</v></c>
 <c r="H1"><f>SUM(Area)+Scoped+Second!Scoped+OnChart+Beyond+SUM(Deep)</f><v>0</v></c>
 <c r="I1"><f>Table1[[#This Row],[Qty]]*2</f><v>42</v></c>
 <c r="J1" t="str"><f>Table1[[#Headers],[Qty]]</f><v>Qty</v></c></row>
<row r="2"><c r="A2"><v>2</v></c><c r="C2"><v>2</v></c><c r="E2" cm="1"><f t="array" ref="E2">_xlfn.SINGLE(A1:A3)*10</f><v>0</v></c>
 <c r="F2"><f>Second!#REF!</f><v>0</v></c></row>
<row r="3"><c r="C3"><v>3</v></c></row>
</sheetData></worksheet>"#;
    let second = r#"<worksheet><sheetData><row r="1"><c r="A1"><v>10</v></c><c r="B1"><f>Scoped+First!Scoped+Lost</f><v>0</v></c></row></sheetData></worksheet>"#;
    let bytes = package(&[
        ("xl/workbook.xml", workbook),
        ("xl/_rels/workbook.xml.rels", rels),
        ("xl/worksheets/sheet1.xml", first),
        ("xl/chartsheets/sheet1.xml", "<chartsheet/>"),
        ("xl/worksheets/sheet2.xml", second),
    ]);
    for locale in [Locale::en(), Locale::fr()] {
        let wb = read_xlsx(Cursor::new(&bytes), locale).unwrap();
        let fr = locale.id == "fr";
        let shown = |sheet: usize, cell: &str| {
            let (r, c) = common::at(cell);
            wb.formula_text(sheet, r, c).unwrap_or_default()
        };
        assert_eq!(shown(0, "E1"), if fr { "=SOMME(C1#)" } else { "=SUM(C1#)" });
        assert_eq!(value(&wb, 0, "E1"), Value::Number(6.0));
        assert_eq!(shown(0, "E2"), "=@A1:A3*10");
        assert_eq!(value(&wb, 0, "E2"), Value::Number(20.0));
        assert_eq!(shown(0, "F1"), "=#REF!A1+1");
        assert_eq!(value(&wb, 0, "F1"), Value::Error(ErrorKind::Ref));
        assert_eq!(shown(0, "F2"), "=Second!#REF!");
        assert_eq!(
            shown(0, "G1"),
            if fr {
                "=SOMME(First:Second!A1;(A1:A2;C2);A1:A2 A2:B2)"
            } else {
                "=SUM(First:Second!A1,(A1:A2,C2),A1:A2 A2:B2)"
            }
        );
        assert_eq!(value(&wb, 0, "G1"), Value::Number(18.0));
        // 5 + First's Scoped (4) + Second's (20) + 5 + 6 + 11.
        assert_eq!(value(&wb, 0, "H1"), Value::Number(51.0));
        assert_eq!(value(&wb, 1, "B1"), Value::Error(ErrorKind::Ref));
        // No table was loaded: the cached results stand.
        assert_eq!(shown(0, "I1"), "");
        assert_eq!(value(&wb, 0, "I1"), Value::Number(42.0));
        assert_eq!(value(&wb, 0, "J1"), Value::text("Qty"));

        let names: Vec<(NameScope, &str, String, &str)> = wb
            .all_names()
            .into_iter()
            .map(|(scope, def)| {
                (
                    scope,
                    def.name.as_str(),
                    def.expr.to_text(Locale::en()),
                    def.comment.as_str(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                (
                    NameScope::Workbook,
                    "Area",
                    "First!$A$1:$A$2,First!$C$2".to_string(),
                    "two blocks"
                ),
                // Scoped to sheets that are not there: kept for the workbook.
                (NameScope::Workbook, "Beyond", "6".to_string(), ""),
                (
                    NameScope::Workbook,
                    "Deep",
                    "First:Second!$A$1".to_string(),
                    ""
                ),
                (NameScope::Workbook, "Lost", "#REF!$A$1".to_string(), ""),
                (NameScope::Workbook, "OnChart", "5".to_string(), ""),
                (NameScope::Workbook, "Scoped", "-1".to_string(), ""),
                (
                    NameScope::Sheet(0),
                    "Scoped",
                    "First!$A$2*2".to_string(),
                    ""
                ),
                (
                    NameScope::Sheet(1),
                    "Scoped",
                    "Second!$A$1*2".to_string(),
                    "on Second"
                ),
            ]
        );
    }
}
