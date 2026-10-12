//! Tables in `.xlsx` files: what the writer produces, that it comes back
//! unchanged, and what is made of the table parts Excel writes.

mod common;

use common::{assert_well_formed, at, enter, formula, package, part, part_names, value};
use opensheet_engine::filter::{
    ColorFilter, Condition, Criteria, CustomFilter, CustomOp, DateGroup, DynamicFilter,
    FilterScope, SortState, Top10, ValueFilter,
};
use opensheet_engine::ops::{SortLevel, SortOn};
use opensheet_engine::table::{Table, TableOption, TotalsFunction};
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::collections::HashSet;
use std::io::Cursor;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Set `OPENSHEET_TABLE_SAMPLES` to a directory to keep the packages these
/// tests build, to open them in Excel.
fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_TABLE_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

fn table<'a>(wb: &'a Workbook, name: &str) -> (usize, &'a Table) {
    wb.table_by_name(name)
        .unwrap_or_else(|| panic!("no table {name}"))
}

/// Fill a block of cells from rows of text, as typed.
fn fill(wb: &mut Workbook, sheet: usize, top_left: &str, rows: &[&[&str]]) {
    let (r0, c0) = at(top_left);
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if !text.is_empty() {
                wb.set_cell_text(sheet, r0 + r as u32, c0 + c as u32, text)
                    .unwrap_or_else(|e| panic!("cannot enter {text:?}: {e:?}"));
            }
        }
    }
}

/// A workbook with tables of every sort on two of its three sheets, and
/// formulas that read them.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Sales"));
    assert!(wb.rename_sheet(1, "More & more"));
    assert!(wb.rename_sheet(2, "Summary"));

    // Orders: a header row, a calculated column, a totals row with a
    // label, a function, a count and a formula of its own.
    fill(
        &mut wb,
        0,
        "B2",
        &[
            &["Item", "Qty", "Unit Price", "Amount"],
            &["Keyboard", "3", "25.5", ""],
            &["Mouse", "10", "8", ""],
            &["Screen", "2", "149.99", ""],
            &["Cable", "25", "1.2", ""],
        ],
    );
    let orders = wb.add_table(0, rect("B2:E6"), true).unwrap();
    wb.rename_table(0, orders, "Orders").unwrap();
    enter(&mut wb, 0, "E3", "=[@Qty]*[@[Unit Price]]");
    wb.tables_after_edit(0, rect("E3"));
    wb.set_table_option(0, orders, TableOption::TotalsRow, true)
        .unwrap();
    assert!(wb.set_totals_label(0, orders, 0, Some("Total")));
    assert!(wb.set_totals_function(0, orders, 1, TotalsFunction::Sum));
    assert!(wb.set_totals_function(0, orders, 2, TotalsFunction::Average));
    assert!(
        wb.set_totals_formula(0, orders, 3, "=SUM(Orders[Amount])/2")
            .unwrap()
    );
    wb.commit("orders");

    // People, below: no style, every option the other way round, a filter
    // that hides rows and a sort made through it.
    fill(
        &mut wb,
        0,
        "B10",
        &[
            &["Name", "Team", "Joined", "Score"],
            &["Ann", "Red", "3/1/2024", "12"],
            &["Bob", "Blue", "3/15/2024", "7"],
            &["Cy", "Red", "4/2/2024", "30"],
            &["Di", "", "12/24/2023", "18"],
            &["Ed", "Blue", "1/5/2025", "4"],
        ],
    );
    let people = wb.add_table(0, rect("B10:E15"), true).unwrap();
    wb.rename_table(0, people, "People").unwrap();
    assert!(wb.set_table_style(0, people, None));
    for option in [
        TableOption::BandedRows,
        TableOption::BandedCols,
        TableOption::FirstCol,
        TableOption::LastCol,
    ] {
        let on = option != TableOption::BandedRows;
        wb.set_table_option(0, people, option, on).unwrap();
    }
    let scope = FilterScope::Table(people);
    assert!(wb.set_filter_criteria(
        0,
        scope,
        1,
        Some(Criteria::Values(ValueFilter {
            values: vec!["Red".into()],
            blanks: true,
            dates: Vec::new(),
        })),
    ));
    wb.sort_filter(
        0,
        scope,
        SortState {
            levels: vec![SortLevel::values(3, true)],
            case_sensitive: false,
        },
    )
    .unwrap();
    wb.commit("people");

    // A row hidden by hand, outside any filter.
    wb.set_rows_hidden(0, 19, 19, true);

    // On the next sheet: a table without a header row and without
    // buttons, in another style, and one of a single column, with a name
    // and a heading that need care.
    fill(&mut wb, 1, "A1", &[&["a", "1"], &["b", "2"], &["c", "3"]]);
    let bare = wb.add_table(1, rect("A1:B3"), false).unwrap();
    wb.set_table_option(1, bare, TableOption::HeaderRow, false)
        .unwrap();
    assert!(wb.set_table_style(1, bare, Some("TableStyleDark3")));
    fill(&mut wb, 1, "E5", &[&["Élan <1> & \"co\""], &["5"], &["6"]]);
    let french = wb.add_table(1, rect("E5:E7"), true).unwrap();
    wb.rename_table(1, french, "Tableau_été").unwrap();
    wb.set_table_option(1, french, TableOption::FilterButton, false)
        .unwrap();

    // Stock: a criterion of every form, one per column, a button that is
    // not shown, and a sort on four levels.
    fill(
        &mut wb,
        1,
        "H1",
        &[
            &["Kind", "Qty", "Size", "Rate", "Mark", "When", "Note"],
            &["nut", "5", "10", "1", "x", "2/1/2024", "a"],
            &["bolt", "50", "90", "8", "x", "5/20/2024", "b"],
            &["nut", "7", "80", "9", "x", "5/21/2024", "c"],
            &["", "70", "70", "7", "x", "7/4/2023", "d"],
            &["gear", "8", "60", "6", "x", "5/5/2024", "e"],
            &["bolt", "9", "75", "8", "x", "5/6/2024", "f"],
            &["nut", "3", "85", "9", "x", "9/9/2024", "g"],
            &["bolt", "4", "95", "9", "x", "5/30/2024", "h"],
        ],
    );
    let yellow = Color(0xFFFF00);
    for cell in ["L2", "L3", "L4", "L5", "L7", "L9"] {
        wb.update_style(1, rect(cell), &|s| s.fill = Some(yellow));
    }
    wb.update_style(1, rect("I4"), &|s| s.font.color = Some(Color(0xC00000)));
    let stock = wb.add_table(1, rect("H1:N9"), true).unwrap();
    wb.rename_table(1, stock, "Stock").unwrap();
    let scope = FilterScope::Table(stock);
    wb.sort_filter(
        1,
        scope,
        SortState {
            case_sensitive: true,
            levels: vec![
                SortLevel {
                    key: 0,
                    on: SortOn::Values,
                    descending: false,
                    custom_list: Some(vec!["nut".into(), "bolt".into(), "gear".into()]),
                },
                SortLevel::values(2, true),
                SortLevel {
                    key: 4,
                    on: SortOn::CellColor(Some(yellow)),
                    descending: false,
                    custom_list: None,
                },
                SortLevel {
                    key: 1,
                    on: SortOn::FontColor(Some(Color(0xC00000))),
                    descending: true,
                    custom_list: None,
                },
            ],
        },
    )
    .unwrap();
    let criteria = [
        Criteria::Values(ValueFilter {
            values: vec!["nut".into(), "bolt".into()],
            blanks: true,
            dates: Vec::new(),
        }),
        Criteria::Custom(CustomFilter {
            first: Condition {
                op: CustomOp::GreaterOrEqual,
                value: "4".into(),
            },
            second: Some(Condition {
                op: CustomOp::NotEqual,
                value: "8".into(),
            }),
            and: true,
        }),
        Criteria::Top10(Top10::new(true, true, 75.0)),
        Criteria::Dynamic(DynamicFilter::AboveAverage),
        Criteria::Color(ColorFilter {
            color: Some(yellow),
            font: false,
        }),
        Criteria::Values(ValueFilter {
            values: Vec::new(),
            blanks: false,
            dates: vec![DateGroup::month(2024, 5), DateGroup::year(2023)],
        }),
    ];
    for (id, criteria) in criteria.into_iter().enumerate() {
        assert!(wb.set_filter_criteria(1, scope, id as u32, Some(criteria)));
    }
    wb.sheets[1]
        .tables
        .iter_mut()
        .find(|t| t.id == stock)
        .and_then(|t| t.auto_filter.as_mut())
        .unwrap()
        .hidden_buttons
        .insert(6);
    wb.commit("more");

    // Formulas reading the tables from elsewhere.
    for (cell, text) in [
        ("A1", "=SUM(Orders[Qty])"),
        ("A2", "=SUM(Orders[Amount])"),
        ("A3", "=Orders[[#Totals],[Qty]]"),
        ("A4", "=COUNTA(Orders[[#Headers],[Item]:[Amount]])"),
        ("A5", "=ROWS(Orders[#All])"),
        ("A6", "=SUM(People[Score])"),
        ("A7", "=SUBTOTAL(109,People[Score])"),
        ("A8", "=SUM(Tableau_été[Élan <1> & \"co\"])"),
        ("A10", "=SUM(Stock[Qty])"),
        ("A9", "=INDEX(Orders[Item],2)"),
    ] {
        enter(&mut wb, 2, cell, text);
    }
    wb.commit("summary");
    wb
}

#[test]
fn tables_round_trip() {
    let original = sample();
    assert_eq!(value(&original, 2, "A1"), Value::Number(40.0));
    assert_eq!(value(&original, 2, "A6"), Value::Number(71.0));
    assert_eq!(
        value(&original, 2, "A7"),
        Value::Number(60.0),
        "the rows the filter shows"
    );
    assert_eq!(value(&original, 2, "A8"), Value::Number(11.0));

    let bytes = to_bytes(&original);
    keep("opensheet-tables.xlsx", &bytes);
    let loaded = load(&bytes);
    for (index, sheet) in original.sheets.iter().enumerate() {
        assert_eq!(sheet.tables, loaded.sheets[index].tables, "{}", sheet.name);
        assert_eq!(
            sheet.filtered_rows, loaded.sheets[index].filtered_rows,
            "{}",
            sheet.name
        );
        assert_eq!(
            sheet.hidden_rows, loaded.sheets[index].hidden_rows,
            "{}",
            sheet.name
        );
    }
    assert_eq!(
        loaded.sheets[0]
            .filtered_rows
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [11, 14]
    );
    assert_eq!(
        loaded.sheets[0]
            .hidden_rows
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [19]
    );
    assert!(!loaded.modified && !loaded.can_undo());

    // The formulas read the tables as before.
    for cell in ["A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9", "A10"] {
        assert_eq!(
            formula(&original, 2, cell),
            formula(&loaded, 2, cell),
            "{cell}"
        );
        assert_eq!(value(&original, 2, cell), value(&loaded, 2, cell), "{cell}");
    }
    assert_eq!(
        formula(&loaded, 2, "A4").unwrap(),
        "COUNTA(Orders[[#Headers],[Item]:[Amount]])"
    );
    assert_eq!(
        formula(&loaded, 0, "E4").unwrap(),
        "Orders[@Qty]*Orders[@[Unit Price]]"
    );
    assert_eq!(value(&loaded, 0, "E4"), Value::Number(80.0));
    assert_eq!(value(&loaded, 0, "C7"), Value::Number(40.0));
    assert_eq!(value(&loaded, 0, "B7"), Value::text("Total"));

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
    }
}

/// The targets of the relationships of a part, resolved to part names,
/// with the last word of their types.
fn targets(bytes: &[u8], rels: &str, base: &str) -> Vec<(String, String)> {
    part(bytes, rels)
        .split("<Relationship ")
        .skip(1)
        .map(|rel| {
            let attr = |name: &str| {
                let start = rel.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                rel[start..].split('"').next().unwrap().to_string()
            };
            let target = attr("Target");
            let target = match target.strip_prefix("../") {
                Some(up) => format!("xl/{up}"),
                None => format!("{base}{target}"),
            };
            (attr("Type").rsplit('/').next().unwrap().to_string(), target)
        })
        .collect()
}

#[test]
fn package_lists_the_table_parts() {
    let mut wb = sample();
    // A sheet with everything a worksheet can point at: a note comes
    // before the tables.
    assert!(wb.set_note(1, 0, 3, "Ann", "Ann:\nbeside tables"));
    let bytes = to_bytes(&wb);
    let names = part_names(&bytes);
    for n in 1..=5 {
        assert!(
            names.contains(&format!("xl/tables/table{n}.xml")),
            "table{n}"
        );
    }
    assert!(
        !names
            .iter()
            .any(|n| n.contains("table6") || n.contains("sheet3.xml.rels"))
    );
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    for name in names.iter().filter(|n| !n.ends_with(".vml")) {
        assert_well_formed(name, &part(&bytes, name));
    }
    let types = part(&bytes, "[Content_Types].xml");
    for name in names
        .iter()
        .filter(|n| n.ends_with(".xml") && !n.starts_with('['))
    {
        let kind = if name.starts_with("xl/tables/") {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"
        } else if name.starts_with("xl/theme/") {
            "application/vnd.openxmlformats-officedocument.theme+xml"
        } else {
            ""
        };
        assert!(
            types.contains(&format!(
                "<Override PartName=\"/{name}\" ContentType=\"{kind}"
            )),
            "{name} is not declared as {kind:?}"
        );
    }

    // Sheet → tables, by relationships whose targets exist, under the ids
    // `<tableParts>` uses, in the order of the tables.
    let pair = |kind: &str, target: &str| (kind.to_string(), target.to_string());
    assert_eq!(
        targets(
            &bytes,
            "xl/worksheets/_rels/sheet1.xml.rels",
            "xl/worksheets/"
        ),
        [
            pair("table", "xl/tables/table1.xml"),
            pair("table", "xl/tables/table2.xml"),
        ]
    );
    assert_eq!(
        targets(
            &bytes,
            "xl/worksheets/_rels/sheet2.xml.rels",
            "xl/worksheets/"
        ),
        [
            pair("vmlDrawing", "xl/drawings/vmlDrawing1.vml"),
            pair("comments", "xl/comments1.xml"),
            pair("table", "xl/tables/table3.xml"),
            pair("table", "xl/tables/table4.xml"),
            pair("table", "xl/tables/table5.xml"),
        ]
    );
    let rels = part(&bytes, "xl/worksheets/_rels/sheet2.xml.rels");
    assert!(rels.starts_with(&format!(
        "{DECL}<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" "
    )));
    assert!(rels.contains(&format!(
        "<Relationship Id=\"rId5\" Type=\"{REL}/table\" Target=\"../tables/table5.xml\"/>"
    )));
    let sheet = |n: usize| part(&bytes, &format!("xl/worksheets/sheet{n}.xml"));
    assert!(sheet(1).ends_with(
        "/><tableParts count=\"2\"><tablePart r:id=\"rId1\"/><tablePart r:id=\"rId2\"/></tableParts></worksheet>"
    ));
    assert!(sheet(2).ends_with(
        "/><legacyDrawing r:id=\"rId1\"/><tableParts count=\"3\"><tablePart r:id=\"rId3\"/><tablePart r:id=\"rId4\"/><tablePart r:id=\"rId5\"/></tableParts></worksheet>"
    ));
    assert!(!sheet(3).contains("tablePart"));
    // Each table is where its part says, under its own id.
    let ids: Vec<String> = (1..=5)
        .map(|n| {
            let xml = part(&bytes, &format!("xl/tables/table{n}.xml"));
            xml.split(" id=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(ids, ["1", "2", "3", "4", "5"]);

    // The theme of the engine is in every package, tables or not.
    let empty = to_bytes(&Workbook::new(Locale::en()));
    for bytes in [&bytes, &empty] {
        let theme = part(bytes, "xl/theme/theme1.xml");
        assert!(theme.contains("<a:accent1><a:srgbClr val=\"156082\"/></a:accent1>"));
        assert!(theme.contains("<a:latin typeface=\"Aptos Narrow\""));
        assert!(part(bytes, "xl/_rels/workbook.xml.rels").contains(&format!(
            "Type=\"{REL}/theme\" Target=\"theme/theme1.xml\"/>"
        )));
    }
    assert!(!part_names(&empty).iter().any(|n| n.contains("tables")));
    assert!(part(&empty, "xl/styles.xml").contains("<dxfs count=\"0\"/>"));
}

#[test]
fn parts_say_what_the_model_says() {
    let bytes = to_bytes(&sample());
    let table = |n: usize| part(&bytes, &format!("xl/tables/table{n}.xml"));
    // Orders: the totals row counts, the label, the functions, the formula
    // of the calculated column and that of the total in the long form
    // files use.
    assert_eq!(
        table(1),
        format!(
            concat!(
                "{}<table xmlns=\"{}\" id=\"1\" name=\"Orders\" displayName=\"Orders\" ref=\"B2:E7\" totalsRowCount=\"1\">",
                "<autoFilter ref=\"B2:E6\"/>",
                "<tableColumns count=\"4\"><tableColumn id=\"1\" name=\"Item\" totalsRowLabel=\"Total\"/>",
                "<tableColumn id=\"2\" name=\"Qty\" totalsRowFunction=\"sum\"/>",
                "<tableColumn id=\"3\" name=\"Unit Price\" totalsRowFunction=\"average\"/>",
                "<tableColumn id=\"4\" name=\"Amount\" totalsRowFunction=\"custom\">",
                "<calculatedColumnFormula>Orders[[#This Row],[Qty]]*Orders[[#This Row],[Unit Price]]</calculatedColumnFormula>",
                "<totalsRowFormula>SUM(Orders[Amount])/2</totalsRowFormula></tableColumn></tableColumns>",
                "<tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>"
            ),
            DECL, MAIN
        )
    );
    // People: no style, the options, the filter and, after it, the sort.
    let people = table(2);
    assert!(people.contains(concat!(
        " ref=\"B10:E15\" totalsRowShown=\"0\"><autoFilter ref=\"B10:E15\"><filterColumn colId=\"1\"><filters blank=\"1\"><filter val=\"Red\"/></filters></filterColumn></autoFilter>",
        "<sortState ref=\"B11:E15\"><sortCondition descending=\"1\" ref=\"E11:E15\"/></sortState><tableColumns count=\"4\">"
    )), "{people}");
    assert!(people.ends_with("</tableColumns><tableStyleInfo showFirstColumn=\"1\" showLastColumn=\"1\" showRowStripes=\"0\" showColumnStripes=\"1\"/></table>"));
    // No header row: no filter, and the columns keep their names.
    let bare = table(3);
    assert!(bare.contains(" name=\"Table1\" displayName=\"Table1\" ref=\"A2:B4\" headerRowCount=\"0\" totalsRowShown=\"0\"><tableColumns count=\"2\"><tableColumn id=\"1\" name=\"Column1\"/><tableColumn id=\"2\" name=\"Column2\"/></tableColumns><tableStyleInfo name=\"TableStyleDark3\" "), "{bare}");
    // Buttons off: no filter either; names are escaped.
    let french = table(4);
    assert!(french.contains(" name=\"Tableau_été\" displayName=\"Tableau_été\" ref=\"E5:E7\" totalsRowShown=\"0\"><tableColumns count=\"1\"><tableColumn id=\"1\" name=\"Élan &lt;1&gt; &amp; &quot;co&quot;\"/></tableColumns>"), "{french}");
    // Stock: every form of criterion, and the colours in the styles part.
    let stock = table(5);
    assert!(stock.contains(concat!(
        "<autoFilter ref=\"H1:N9\">",
        "<filterColumn colId=\"0\"><filters blank=\"1\"><filter val=\"nut\"/><filter val=\"bolt\"/></filters></filterColumn>",
        "<filterColumn colId=\"1\"><customFilters and=\"1\"><customFilter operator=\"greaterThanOrEqual\" val=\"4\"/><customFilter operator=\"notEqual\" val=\"8\"/></customFilters></filterColumn>",
        "<filterColumn colId=\"2\"><top10 percent=\"1\" val=\"75\" filterVal=\"70\"/></filterColumn>",
        "<filterColumn colId=\"3\"><dynamicFilter type=\"aboveAverage\" val=\"7.125\"/></filterColumn>",
        "<filterColumn colId=\"4\"><colorFilter dxfId=\"0\"/></filterColumn>",
        "<filterColumn colId=\"5\"><filters><dateGroupItem year=\"2024\" month=\"5\" dateTimeGrouping=\"month\"/><dateGroupItem year=\"2023\" dateTimeGrouping=\"year\"/></filters></filterColumn>",
        "<filterColumn colId=\"6\" hiddenButton=\"1\"/></autoFilter>",
        "<sortState ref=\"H2:N9\" caseSensitive=\"1\"><sortCondition ref=\"H2:H9\" customList=\"nut,bolt,gear\"/><sortCondition descending=\"1\" ref=\"J2:J9\"/>",
        "<sortCondition sortBy=\"cellColor\" ref=\"L2:L9\" dxfId=\"0\"/><sortCondition descending=\"1\" sortBy=\"fontColor\" ref=\"I2:I9\" dxfId=\"1\"/></sortState>",
        "<tableColumns count=\"7\">"
    )), "{stock}");
    let styles = part(&bytes, "xl/styles.xml");
    assert!(styles.contains(concat!(
        "</cellStyles><dxfs count=\"2\">",
        "<dxf><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFFF00\"/><bgColor rgb=\"FFFFFF00\"/></patternFill></fill></dxf>",
        "<dxf><font><color rgb=\"FFC00000\"/></font></dxf></dxfs><tableStyles "
    )), "{styles}");

    // The rows a filter hides are hidden in the sheet, like those hidden
    // by hand; the cells of a table are cells like any others.
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    let hidden: Vec<&str> = sheet
        .split("<row r=\"")
        .skip(1)
        .filter(|row| row.split('>').next().unwrap().contains("hidden=\"1\""))
        .map(|row| row.split('"').next().unwrap())
        .collect();
    assert_eq!(hidden, ["12", "15", "20"]);
    assert!(sheet.contains("<c r=\"E3\"><f>Orders[[#This Row],[Qty]]*Orders[[#This Row],[Unit Price]]</f><v>76.5</v></c>"));
    assert!(sheet.contains("<c r=\"E7\"><f>SUM(Orders[Amount])/2</f><v>243.24</v></c>"));
    let summary = part(&bytes, "xl/worksheets/sheet3.xml");
    assert!(
        summary.contains("<f>Orders[[#Totals],[Qty]]</f><v>40</v>"),
        "{summary}"
    );
    assert!(summary.contains("<f>SUM(Tableau_été[[Élan &lt;1&gt; &amp; \"co\"]])</f><v>11</v>"));
}

// ---- element order ---------------------------------------------------------------

/// The children an element of a table part or of a worksheet may have, in
/// the order of the schema (ECMA-376 `sml.xsd`).
const CONTENT: &[(&str, &[&str])] = &[
    (
        "worksheet",
        &[
            "sheetPr",
            "dimension",
            "sheetViews",
            "sheetFormatPr",
            "cols",
            "sheetData",
            "sheetCalcPr",
            "sheetProtection",
            "protectedRanges",
            "scenarios",
            "autoFilter",
            "sortState",
            "dataConsolidate",
            "customSheetViews",
            "mergeCells",
            "phoneticPr",
            "conditionalFormatting",
            "dataValidations",
            "hyperlinks",
            "printOptions",
            "pageMargins",
            "pageSetup",
            "headerFooter",
            "rowBreaks",
            "colBreaks",
            "customProperties",
            "cellWatches",
            "ignoredErrors",
            "smartTags",
            "drawing",
            "legacyDrawing",
            "legacyDrawingHF",
            "drawingHF",
            "picture",
            "oleObjects",
            "controls",
            "webPublishItems",
            "tableParts",
            "extLst",
        ],
    ),
    ("tableParts", &["tablePart"]),
    (
        "table",
        &[
            "autoFilter",
            "sortState",
            "tableColumns",
            "tableStyleInfo",
            "extLst",
        ],
    ),
    ("autoFilter", &["filterColumn", "sortState", "extLst"]),
    (
        "filterColumn",
        &[
            "filters",
            "top10",
            "customFilters",
            "dynamicFilter",
            "colorFilter",
            "iconFilter",
            "extLst",
        ],
    ),
    ("filters", &["filter", "dateGroupItem"]),
    ("customFilters", &["customFilter"]),
    ("sortState", &["sortCondition", "extLst"]),
    ("tableColumns", &["tableColumn"]),
    (
        "tableColumn",
        &[
            "calculatedColumnFormula",
            "totalsRowFormula",
            "xmlColumnPr",
            "extLst",
        ],
    ),
    ("dxfs", &["dxf"]),
    (
        "dxf",
        &[
            "font",
            "numFmt",
            "fill",
            "alignment",
            "border",
            "protection",
            "extLst",
        ],
    ),
];

/// Elements that may follow one another.
const REPEATABLE: &[&str] = &[
    "tablePart",
    "filterColumn",
    "filter",
    "dateGroupItem",
    "customFilter",
    "sortCondition",
    "tableColumn",
    "dxf",
    "conditionalFormatting",
];

/// Check the order of the children of the elements listed in [`CONTENT`].
fn assert_schema_order(part_name: &str, xml: &str) {
    use quick_xml::events::Event;
    // (element, position of its last child in its content model)
    let mut open: Vec<(String, Option<usize>)> = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        let (name, empty) = match reader.read_event().unwrap() {
            Event::Start(e) => (
                String::from_utf8(e.name().as_ref().to_vec()).unwrap(),
                false,
            ),
            Event::Empty(e) => (String::from_utf8(e.name().as_ref().to_vec()).unwrap(), true),
            Event::End(_) => {
                open.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if let Some((parent, last)) = open.last_mut()
            && let Some((_, allowed)) = CONTENT.iter().find(|(element, _)| element == parent)
        {
            let at = allowed
                .iter()
                .position(|child| *child == name)
                .unwrap_or_else(|| panic!("{part_name}: <{name}> is not allowed in <{parent}>"));
            assert!(
                last.is_none_or(
                    |last| at > last || (at == last && REPEATABLE.contains(&name.as_str()))
                ),
                "{part_name}: <{name}> is out of order in <{parent}>"
            );
            *last = Some(at);
        }
        if !empty {
            open.push((name, None));
        }
    }
}

#[test]
fn elements_come_in_schema_order() {
    let mut wb = sample();
    assert!(wb.set_note(1, 0, 3, "Ann", "Ann:\nbeside tables"));
    let bytes = to_bytes(&wb);
    let mut checked = 0;
    for name in part_names(&bytes) {
        if name.starts_with("xl/worksheets/sheet")
            || name.starts_with("xl/tables/")
            || name == "xl/styles.xml"
        {
            assert_schema_order(&name, &part(&bytes, &name));
            checked += 1;
        }
    }
    assert_eq!(checked, 3 + 5 + 1);
    // The check itself notices what it is there for.
    let sheet = part(&bytes, "xl/worksheets/sheet2.xml");
    let stock = part(&bytes, "xl/tables/table5.xml");
    for (xml, good, bad) in [
        (
            &sheet,
            "<legacyDrawing r:id=\"rId1\"/><tableParts",
            "<tableParts count=\"0\"/><legacyDrawing r:id=\"rId1\"/><tableParts",
        ),
        (
            &stock,
            "</sortState><tableColumns count=\"7\">",
            "</sortState><tableStyleInfo/><tableColumns count=\"7\">",
        ),
        (
            &stock,
            "<customFilters and=\"1\">",
            "<customFilters and=\"1\"><filter/>",
        ),
    ] {
        assert!(xml.contains(good), "{good}");
        let broken = xml.replacen(good, bad, 1);
        assert!(
            std::panic::catch_unwind(|| assert_schema_order("broken", &broken)).is_err(),
            "{bad} should be refused"
        );
    }
}

// ---- files from Excel ------------------------------------------------------------

fn relationships(list: &[(&str, &str, &str)]) -> String {
    let mut x = format!("{DECL}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in list {
        x.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"
        ));
    }
    x.push_str("</Relationships>");
    x
}

/// What Excel 365 puts on the root element of a table part.
const EXCEL_TABLE_NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"xr xr3\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\" xmlns:xr3=\"http://schemas.microsoft.com/office/spreadsheetml/2016/revision3\"";

/// A table as Excel 365 writes it: a filter that hides a row, a sort
/// beside the filter, a totals row with a label, a function and a formula,
/// a calculated column, formats of whole columns.
fn excel_table() -> String {
    format!(
        r#"{DECL}<table {EXCEL_TABLE_NS} id="4" xr:uid="{{6C0A2C4F-5B0E-4D3C-9B7A-0F1E2D3C4B5A}}" name="Regions" displayName="Regions" ref="B2:E7" totalsRowCount="1" headerRowDxfId="3" dataDxfId="2"><autoFilter ref="B2:E6" xr:uid="{{6C0A2C4F-5B0E-4D3C-9B7A-0F1E2D3C4B5B}}"><filterColumn colId="1"><filters><filter val="East"/><filter val="West"/></filters></filterColumn><filterColumn colId="2"><colorFilter dxfId="1"/></filterColumn></autoFilter><sortState xmlns:xlrd2="http://schemas.microsoft.com/office/spreadsheetml/2017/richdata2" ref="B3:E6"><sortCondition descending="1" ref="D3:D6"/><sortCondition sortBy="fontColor" ref="B3:B6" dxfId="0"/></sortState><tableColumns count="4"><tableColumn id="1" xr3:uid="{{00000000-0010-0000-0000-000001000000}}" name="Region" totalsRowLabel="Total"/><tableColumn id="2" xr3:uid="{{00000000-0010-0000-0000-000002000000}}" name="Zone" totalsRowFunction="count"/><tableColumn id="3" xr3:uid="{{00000000-0010-0000-0000-000003000000}}" name="Sales_x000a_2024" totalsRowFunction="sum" dataDxfId="1"/><tableColumn id="5" xr3:uid="{{00000000-0010-0000-0000-000005000000}}" name="Share" totalsRowFunction="custom" dataDxfId="0"><calculatedColumnFormula>Regions[[#This Row],[Sales
2024]]/SUM(Regions[[Sales
2024]])</calculatedColumnFormula><totalsRowFormula>SUM(Regions[Share])*100</totalsRowFormula></tableColumn></tableColumns><tableStyleInfo name="TableStyleLight9" showFirstColumn="1" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>"#
    )
}

/// A table without a header row, and one whose buttons are off, on the
/// same sheet.
fn excel_small_tables() -> (String, String) {
    (
        format!(
            r#"{DECL}<table {EXCEL_TABLE_NS} id="7" name="Table7" displayName="NoHeader" ref="G2:H4" headerRowCount="0" totalsRowShown="0"><tableColumns count="2"><tableColumn id="1" name="Column1"/><tableColumn id="2" name="Column2"/></tableColumns><tableStyleInfo showFirstColumn="0" showLastColumn="0" showRowStripes="0" showColumnStripes="1"/></table>"#
        ),
        format!(
            r#"{DECL}<x:table xmlns:x="{MAIN}" id="9" name="Quiet" displayName="Quiet" ref="$G$8:$G$10" totalsRowShown="0"><x:tableColumns count="1"><x:tableColumn id="1" name="Only"/></x:tableColumns></x:table>"#
        ),
    )
}

fn excel_sheet() -> String {
    let cell = |name: &str, text: &str| {
        format!("<c r=\"{name}\" t=\"inlineStr\"><is><t>{text}</t></is></c>")
    };
    let row = |r: u32, hidden: bool, region: &str, zone: &str, sales: f64| {
        format!(
            "<row r=\"{r}\"{}>{}{}<c r=\"D{r}\"><v>{sales}</v></c><c r=\"E{r}\" s=\"1\"><f>Regions[[#This Row],[Sales\n2024]]/SUM(Regions[[Sales\n2024]])</f><v>{}</v></c></row>",
            if hidden { " hidden=\"1\"" } else { "" },
            cell(&format!("B{r}"), region),
            cell(&format!("C{r}"), zone),
            sales / 100.0,
        )
    };
    format!(
        r#"{DECL}<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><dimension ref="B2:H12"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><sheetData><row r="2">{}{}{}{}<c r="G2"><v>1</v></c><c r="H2"><v>2</v></c></row>{}{}{}{}<row r="7">{}<c r="C7"><f>SUBTOTAL(103,Regions[Zone])</f><v>3</v></c><c r="D7"><f>SUBTOTAL(109,Regions[[Sales
2024]])</f><v>70</v></c><c r="E7"><f>SUM(Regions[Share])*100</f><v>100</v></c></row><row r="8">{}</row><row r="9"><c r="G9"><v>5</v></c></row><row r="10"><c r="G10"><v>6</v></c></row><row r="12" hidden="1"><c r="B12"><f>SUM(Regions[[Sales
2024]])+SUM(NoHeader[Column2])+SUM(Quiet[Only])</f><v>113</v></c></row></sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><tableParts count="3"><tablePart r:id="rId1"/><tablePart r:id="rId2"/><tablePart r:id="rId3"/></tableParts></worksheet>"#,
        cell("B2", "Region"),
        cell("C2", "Zone"),
        cell("D2", "Sales\n2024"),
        cell("E2", "Share"),
        row(3, false, "North", "East", 40.0),
        row(4, true, "South", "Mid", 30.0),
        row(5, false, "Centre", "West", 20.0),
        row(6, false, "Islands", "East", 10.0),
        cell("B7", "Total"),
        cell("G8", "Only"),
    )
}

/// A workbook laid out like those of Excel 365, with three tables on its
/// sheet. Written by hand. `replace` swaps parts (or removes them, with
/// `None`).
fn excel_package(replace: &[(&str, Option<&str>)]) -> Vec<u8> {
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/tables/table1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/tables/table2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/><Override PartName="/xl/tables/table3.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/></Types>"#
    );
    let workbook = format!(
        r#"{DECL}<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Regions by zone" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    // Four differential formats: a text colour, a fill as filters name it,
    // and the formats of the table's own cells.
    let styles = format!(
        r#"{DECL}<styleSheet xmlns="{MAIN}"><numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><fonts count="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="4"><dxf><font><color rgb="FFC00000"/></font></dxf><dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor rgb="FF000000"/></patternFill></fill></dxf><dxf><numFmt numFmtId="164" formatCode="0.0%"/></dxf><dxf><font><b/></font></dxf></dxfs><tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/></styleSheet>"#
    );
    let (no_header, quiet) = excel_small_tables();
    let mut parts: Vec<(String, String)> = [
        ("[Content_Types].xml", content_types),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", workbook),
        (
            "xl/_rels/workbook.xml.rels",
            relationships(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "styles", "styles.xml"),
            ]),
        ),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", excel_sheet()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            relationships(&[
                ("rId3", "table", "../tables/table3.xml"),
                ("rId2", "table", "/xl/tables/table2.xml"),
                ("rId1", "table", "../tables/table1.xml"),
            ]),
        ),
        ("xl/tables/table1.xml", excel_table()),
        ("xl/tables/table2.xml", no_header),
        ("xl/tables/table3.xml", quiet),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_string(), text))
    .collect();
    for (name, text) in replace {
        parts.retain(|(n, _)| n != name);
        if let Some(text) = text {
            parts.push((name.to_string(), text.to_string()));
        }
    }
    let parts: Vec<(&str, &str)> = parts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    package(&parts)
}

#[test]
fn tables_from_excel() {
    let bytes = excel_package(&[]);
    keep("excel-style-tables.xlsx", &bytes);
    let wb = load(&bytes);
    let names: Vec<&str> = wb.sheets[0]
        .tables
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), 3, "{names:?}");

    let (_, regions) = table(&wb, "Regions");
    assert_eq!(regions.id, 4);
    assert_eq!(regions.range, rect("B2:E7"));
    assert!(regions.header_row && regions.totals_row);
    assert_eq!(regions.style.as_deref(), Some("TableStyleLight9"));
    assert!(regions.first_col && !regions.last_col && regions.banded_rows && !regions.banded_cols);
    let columns: Vec<&str> = regions.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(columns, ["Region", "Zone", "Sales\n2024", "Share"]);
    assert_eq!(regions.columns[0].totals, TotalsFunction::None);
    assert_eq!(regions.columns[0].totals_label.as_deref(), Some("Total"));
    assert_eq!(regions.columns[1].totals, TotalsFunction::Count);
    assert_eq!(regions.columns[2].totals, TotalsFunction::Sum);
    let TotalsFunction::Custom(custom) = &regions.columns[3].totals else {
        panic!("{:?}", regions.columns[3].totals);
    };
    assert_eq!(custom.to_text(Locale::en()), "SUM(Regions[Share])*100");
    // The filter with its two criteria, and the sort written beside it.
    let filter = regions.auto_filter.as_ref().expect("buttons");
    assert_eq!(filter.range, rect("B2:E6"));
    assert_eq!(
        filter.columns.get(&1),
        Some(&Criteria::Values(ValueFilter {
            values: vec!["East".into(), "West".into()],
            blanks: false,
            dates: Vec::new(),
        }))
    );
    assert_eq!(
        filter.columns.get(&2),
        Some(&Criteria::Color(ColorFilter {
            color: Some(Color(0xFFFF00)),
            font: false
        }))
    );
    assert_eq!(
        filter.sort,
        Some(SortState {
            case_sensitive: false,
            levels: vec![
                SortLevel::values(2, true),
                SortLevel {
                    key: 0,
                    on: SortOn::FontColor(Some(Color(0xC00000))),
                    descending: false,
                    custom_list: None,
                },
            ],
        })
    );
    // The row the filter hides is filtered; the one hidden by hand stays
    // hidden by hand.
    assert_eq!(
        wb.sheets[0]
            .filtered_rows
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(
        wb.sheets[0].hidden_rows.iter().copied().collect::<Vec<_>>(),
        [11]
    );

    let (_, no_header) = table(&wb, "NoHeader");
    assert_eq!(no_header.range, rect("G2:H4"));
    assert!(!no_header.header_row && !no_header.totals_row && no_header.auto_filter.is_none());
    assert_eq!(no_header.style, None);
    assert!(no_header.banded_cols && !no_header.banded_rows);
    let (_, quiet) = table(&wb, "Quiet");
    assert_eq!(quiet.range, rect("G8:G10"));
    assert!(quiet.header_row && quiet.auto_filter.is_none());
    assert_eq!(quiet.columns[0].name, "Only");

    // The formulas calculate: the calculated column, the totals, a cell
    // reading the three tables.
    assert_eq!(
        formula(&wb, 0, "E3").unwrap(),
        "Regions[@[Sales\n2024]]/SUM(Regions[[Sales\n2024]])"
    );
    assert_eq!(value(&wb, 0, "E3"), Value::Number(0.4));
    assert_eq!(
        value(&wb, 0, "C7"),
        Value::Number(3.0),
        "rows the filter shows"
    );
    assert_eq!(value(&wb, 0, "D7"), Value::Number(70.0));
    assert_eq!(value(&wb, 0, "B12"), Value::Number(100.0 + 2.0 + 11.0));

    // Written back, the file says the same, in the spelling of Excel.
    let saved = to_bytes(&wb);
    keep("excel-style-tables-saved.xlsx", &saved);
    let again = load(&saved);
    assert_eq!(again.sheets[0].tables, wb.sheets[0].tables);
    assert_eq!(again.sheets[0].filtered_rows, wb.sheets[0].filtered_rows);
    let written = part(&saved, "xl/tables/table1.xml");
    assert!(written.contains(" id=\"4\" name=\"Regions\" displayName=\"Regions\" ref=\"B2:E7\" totalsRowCount=\"1\"><autoFilter ref=\"B2:E6\"><filterColumn colId=\"1\"><filters><filter val=\"East\"/><filter val=\"West\"/></filters></filterColumn><filterColumn colId=\"2\"><colorFilter dxfId=\"0\"/></filterColumn></autoFilter><sortState ref=\"B3:E6\"><sortCondition descending=\"1\" ref=\"D3:D6\"/><sortCondition sortBy=\"fontColor\" ref=\"B3:B6\" dxfId=\"1\"/></sortState>"), "{written}");
    assert!(
        written.contains(
            "<tableColumn id=\"3\" name=\"Sales_x000A_2024\" totalsRowFunction=\"sum\"/>"
        ),
        "{written}"
    );
    assert!(written.contains("<tableColumn id=\"4\" name=\"Share\" totalsRowFunction=\"custom\"><calculatedColumnFormula>Regions[[#This Row],[Sales\n2024]]/SUM(Regions[[Sales\n2024]])</calculatedColumnFormula><totalsRowFormula>SUM(Regions[Share])*100</totalsRowFormula></tableColumn>"), "{written}");
    assert_schema_order("table1.xml", &written);
}

// ---- damage ----------------------------------------------------------------------

#[test]
fn a_bad_table_costs_only_itself() {
    let cells = |wb: &Workbook| value(wb, 0, "D3") == Value::Number(40.0) && wb.sheets.len() == 1;
    let names = |wb: &Workbook| -> Vec<String> {
        let mut names: Vec<String> = wb.sheets[0].tables.iter().map(|t| t.name.clone()).collect();
        names.sort();
        names
    };
    // A part that is not there, or holds something else.
    for missing in [
        None,
        Some(""),
        Some("<chartsheet/>"),
        Some("<table id=\"1\"/>"),
    ] {
        let wb = load(&excel_package(&[("xl/tables/table1.xml", missing)]));
        assert!(cells(&wb));
        assert_eq!(names(&wb), ["NoHeader", "Quiet"], "{missing:?}");
        // The formulas that read the table keep the results they had.
        assert_eq!(formula(&wb, 0, "E3"), None);
        assert_eq!(value(&wb, 0, "E3"), Value::Number(0.4));
        assert_eq!(value(&wb, 0, "B12"), Value::Number(113.0));
        // And the rows its filter hid are rows hidden by hand.
        assert_eq!(
            wb.sheets[0].hidden_rows.iter().copied().collect::<Vec<_>>(),
            [3, 11]
        );
        load(&to_bytes(&wb));
    }
    // No relationships: no tables.
    let wb = load(&excel_package(&[(
        "xl/worksheets/_rels/sheet1.xml.rels",
        None,
    )]));
    assert!(cells(&wb) && wb.sheets[0].tables.is_empty());

    // Tables that are off the sheet, have no data row, or lie over one
    // read before them: left out.
    let (no_header, _) = excel_small_tables();
    for (bad, kept) in [
        (no_header.replace("G2:H4", "XFA1:XFZ9"), 2),
        (
            no_header
                .replace("G2:H4", "G2:H2")
                .replace("headerRowCount=\"0\" ", ""),
            2,
        ),
        (no_header.replace("G2:H4", "E6:H9"), 2),
        (no_header.replace("G2:H4", "nowhere"), 2),
        // Too many columns named, too few, twice the same name, names and
        // ids taken by another table: put right.
        (
            no_header.replace(
                "</tableColumns>",
                "<tableColumn id=\"3\" name=\"Column1\"/><tableColumn id=\"4\"/></tableColumns>",
            ),
            3,
        ),
        (
            no_header.replace("<tableColumn id=\"2\" name=\"Column2\"/>", ""),
            3,
        ),
        (
            no_header.replace(
                "id=\"7\" name=\"Table7\" displayName=\"NoHeader\"",
                "id=\"4\" name=\"Regions\" displayName=\"Regions\"",
            ),
            3,
        ),
        (
            no_header.replace("displayName=\"NoHeader\"", "displayName=\"not a name!\""),
            3,
        ),
    ] {
        let wb = load(&excel_package(&[("xl/tables/table2.xml", Some(&bad))]));
        assert!(cells(&wb));
        assert_eq!(wb.sheets[0].tables.len(), kept, "{bad}");
        let (_, regions) = table(&wb, "Regions");
        assert_eq!(regions.range, rect("B2:E7"));
        for t in &wb.sheets[0].tables {
            assert_eq!(t.columns.len() as u32, t.range.cols(), "{}", t.name);
            let unique: HashSet<String> = t.columns.iter().map(|c| c.name.to_lowercase()).collect();
            assert_eq!(unique.len(), t.columns.len(), "{}", t.name);
        }
        let ids: HashSet<u32> = wb.sheets[0].tables.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), kept);
        assert_eq!(names(&wb).iter().collect::<HashSet<_>>().len(), kept);
        // Whatever was read can be written, and read again.
        let again = load(&to_bytes(&wb));
        assert_eq!(again.sheets[0].tables, wb.sheets[0].tables);
    }
}

#[test]
fn damaged_table_parts_never_panic() {
    // Cut the parts of a table short at every character, in a package
    // laid out like Excel's and, at many places, in one of ours.
    let table1 = excel_table();
    for cut in (0..table1.len()).filter(|i| table1.is_char_boundary(*i)) {
        let wb = load(&excel_package(&[(
            "xl/tables/table1.xml",
            Some(&table1[..cut]),
        )]));
        assert!(wb.sheets[0].tables.len() >= 2, "cut at {cut}");
        assert_eq!(value(&wb, 0, "D3"), Value::Number(40.0));
        if cut % 101 == 0 {
            load(&to_bytes(&wb));
        }
    }
    let ours = to_bytes(&sample());
    let names = part_names(&ours);
    let parts: Vec<(String, String)> = names.iter().map(|n| (n.clone(), part(&ours, n))).collect();
    let mut attempts = 0;
    for (victim, text) in parts.iter().filter(|(name, _)| {
        name.contains("table5") || name.contains("sheet2.xml.rels") || name == "xl/styles.xml"
    }) {
        for cut in (0..text.len())
            .step_by(5)
            .filter(|i| text.is_char_boundary(*i))
        {
            let damaged: Vec<(&str, &str)> = parts
                .iter()
                .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                .collect();
            let wb = read_xlsx(Cursor::new(package(&damaged)), Locale::en())
                .expect("the rest of the workbook loads");
            assert_eq!(wb.sheets.len(), 3);
            assert_eq!(wb.sheets[0].tables.len(), 2, "{victim} cut at {cut}");
            attempts += 1;
        }
    }
    assert!(attempts > 500, "{attempts}");

    for junk in [
        "not xml at all",
        "<table ref=\"A1:B2\"><tableColumns><tableColumn name=\"&#0;\" totalsRowFunction=\"custom\"><totalsRowFormula>((((</totalsRowFormula>",
        "<table ref=\"A1:XFD1048576\" id=\"4294967295\" headerRowCount=\"7\" totalsRowCount=\"9\"><autoFilter ref=\"Z9\"><filterColumn colId=\"99999999\"><top10 val=\"1e999\"/>",
        "<table ref=\"B2:E7\"><autoFilter/><sortState><sortCondition/></sortState><tableStyleInfo name=\"\"/></table>",
    ] {
        for victim in ["xl/tables/table1.xml", "xl/tables/table2.xml"] {
            let wb = load(&excel_package(&[(victim, Some(junk))]));
            assert_eq!(wb.sheets.len(), 1, "{victim}: {junk}");
            load(&to_bytes(&wb));
        }
    }
}
