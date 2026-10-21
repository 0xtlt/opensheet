//! The filter of a sheet (AutoFilter) in `.xlsx` files: the element of the
//! worksheet, the rows it hides and the name Excel keeps beside it. The
//! criteria themselves are those of tables, tested in `xlsx_tables.rs`.

mod common;

use common::{enter, package, part, part_names, value};
use opensheet_engine::filter::{
    AutoFilter, ColorFilter, Condition, Criteria, CustomFilter, CustomOp, FilterScope, SortState,
    ValueFilter,
};
use opensheet_engine::ops::{SortLevel, SortOn};
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
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

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

const STAFF: &[[&str; 3]] = &[
    ["Name", "Team", "Age"],
    ["Ann", "Red", "31"],
    ["Bob", "Blue", "45"],
    ["Cid", "Red", "28"],
    ["Dee", "", "39"],
    ["Eve", "Green", "52"],
    ["Fay", "Red", "47"],
];

/// Three sheets: a filter with criteria, a sort and a row hidden by hand
/// below it; a filter that filters nothing; no filter.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Staff & co"));
    assert!(wb.rename_sheet(1, "Plain"));
    for sheet in 0..2 {
        for (r, row) in STAFF.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if !text.is_empty() {
                    wb.set_cell_text(sheet, r as u32 + 1, c as u32 + 1, text)
                        .unwrap();
                }
            }
        }
    }
    enter(&mut wb, 0, "B10", "below");
    wb.set_rows_hidden(0, 9, 9, true);
    wb.set_auto_filter(0, rect("B2:D8")).unwrap();
    let scope = FilterScope::Sheet;
    wb.sort_filter(
        0,
        scope,
        SortState {
            levels: vec![
                SortLevel::values(2, true),
                SortLevel {
                    key: 0,
                    on: SortOn::FontColor(Some(Color(0xC00000))),
                    descending: false,
                    custom_list: None,
                },
            ],
            case_sensitive: true,
        },
    )
    .unwrap();
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
    assert!(wb.set_filter_criteria(
        0,
        scope,
        2,
        Some(Criteria::Custom(CustomFilter::one(Condition {
            op: CustomOp::Greater,
            value: "30".into(),
        }))),
    ));
    assert!(wb.set_filter_criteria(
        0,
        scope,
        0,
        Some(Criteria::Color(ColorFilter {
            color: None,
            font: false,
        })),
    ));
    wb.sheets[0]
        .auto_filter
        .as_mut()
        .unwrap()
        .hidden_buttons
        .insert(0);
    wb.set_auto_filter(1, rect("B2:D8")).unwrap();
    wb.commit("filters");
    wb
}

#[test]
fn sheet_filters_round_trip() {
    let original = sample();
    let filter = original.sheets[0].auto_filter.clone().expect("a filter");
    assert_eq!(filter.columns.len(), 3);
    // Sorted by age, largest first; kept: the Reds and the blank over 30.
    let hidden: Vec<u32> = original.sheets[0].filtered_rows.iter().copied().collect();
    assert_eq!(hidden, [2, 4, 7]);
    assert_eq!(value(&original, 0, "B3"), Value::Text("Eve".into()));

    let bytes = to_bytes(&original);
    let loaded = load(&bytes);
    for (a, b) in original.sheets.iter().zip(&loaded.sheets) {
        assert_eq!(a.auto_filter, b.auto_filter, "{}", a.name);
        // Hidden by the filter and hidden by hand are still told apart.
        assert_eq!(a.filtered_rows, b.filtered_rows, "{}", a.name);
        assert_eq!(a.hidden_rows, b.hidden_rows, "{}", a.name);
    }
    assert!(loaded.sheets[0].hidden_rows.contains(&9));
    assert!(loaded.sheets[2].auto_filter.is_none());
    assert!(!loaded.modified && !loaded.can_undo());
    // The name beside the filter is not a name of the workbook.
    assert!(loaded.all_names().is_empty());

    // The filter still filters: with the criterion on the team gone, the
    // rows over 30 come back.
    let mut changed = load(&bytes);
    assert!(changed.set_filter_criteria(0, FilterScope::Sheet, 1, None));
    let hidden: Vec<u32> = changed.sheets[0].filtered_rows.iter().copied().collect();
    assert_eq!(hidden, [7]);

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
    }
}

#[test]
fn the_worksheet_says_what_the_model_says() {
    let bytes = to_bytes(&sample());
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    common::assert_worksheet_order("sheet1", &sheet);
    // After the cells and before anything else of the sheet, with the
    // sort inside it.
    assert!(sheet.contains(concat!(
        "</sheetData><autoFilter ref=\"B2:D8\">",
        "<filterColumn colId=\"0\" hiddenButton=\"1\"><colorFilter dxfId=\"0\"/></filterColumn>",
        "<filterColumn colId=\"1\"><filters blank=\"1\"><filter val=\"Red\"/></filters></filterColumn>",
        "<filterColumn colId=\"2\"><customFilters><customFilter operator=\"greaterThan\" val=\"30\"/></customFilters></filterColumn>",
        "<sortState ref=\"B3:D8\" caseSensitive=\"1\"><sortCondition descending=\"1\" ref=\"D3:D8\"/><sortCondition sortBy=\"fontColor\" ref=\"B3:B8\" dxfId=\"1\"/></sortState>",
        "</autoFilter>"
    )));
    // The rows the filter hides are hidden rows, like the one hidden by
    // hand.
    for (row, hidden) in [(3, true), (4, false), (5, true), (8, true), (10, true)] {
        assert_eq!(
            sheet.contains(&format!("<row r=\"{row}\" hidden=\"1\"")),
            hidden,
            "row {row}"
        );
    }
    assert!(part(&bytes, "xl/styles.xml").contains(
        "<dxfs count=\"2\"><dxf><fill><patternFill patternType=\"none\"/></fill></dxf><dxf><font><color rgb=\"FFC00000\"/></font></dxf></dxfs>"
    ));
    assert!(
        part(&bytes, "xl/worksheets/sheet2.xml")
            .contains("</sheetData><autoFilter ref=\"B2:D8\"/>")
    );
    assert!(!part(&bytes, "xl/worksheets/sheet3.xml").contains("autoFilter"));

    // Excel keeps the range of each filter as a hidden name of its sheet.
    let workbook = part(&bytes, "xl/workbook.xml");
    common::assert_workbook_order(&workbook);
    assert!(workbook.contains(concat!(
        "<definedNames>",
        "<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">'Staff &amp; co'!$B$2:$D$8</definedName>",
        "<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"1\" hidden=\"1\">Plain!$B$2:$D$8</definedName>",
        "</definedNames>"
    )));
    assert!(
        !part(&to_bytes(&Workbook::new(Locale::en())), "xl/workbook.xml").contains("definedNames")
    );
}

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

/// The `<autoFilter>` of a sheet the way Excel writes it, with elements
/// the engine has nothing for.
fn excel_filter() -> &'static str {
    r#"<autoFilter ref="A1:C6" xr:uid="{00000000-0009-0000-0000-000000000000}"><filterColumn colId="1"><filters><filter val="Red"/></filters></filterColumn><filterColumn colId="2"><iconFilter iconSet="3Arrows" iconId="0"/></filterColumn><sortState xmlns:xlrd2="http://schemas.microsoft.com/office/spreadsheetml/2017/richdata2" ref="A2:C6"><sortCondition ref="A2:A6"/></sortState></autoFilter>"#
}

/// A workbook laid out like Excel's: a filtered list whose rows 3 and 5
/// are hidden by the filter, and row 8 by hand.
fn excel_package(filter: &str) -> Vec<u8> {
    let cell = |name: &str, text: &str| {
        format!("<c r=\"{name}\" t=\"inlineStr\"><is><t>{text}</t></is></c>")
    };
    let mut rows = String::new();
    for (r, (name, team, hidden)) in [
        ("Name", "Team", false),
        ("Ann", "Red", false),
        ("Bob", "Blue", true),
        ("Cid", "Red", false),
        ("Dee", "Green", true),
        ("Eve", "Red", false),
    ]
    .into_iter()
    .enumerate()
    {
        let n = r + 1;
        let hidden = if hidden { " hidden=\"1\"" } else { "" };
        rows.push_str(&format!(
            "<row r=\"{n}\"{hidden}>{}{}<c r=\"C{n}\"><v>{n}</v></c></row>",
            cell(&format!("A{n}"), name),
            cell(&format!("B{n}"), team)
        ));
    }
    rows.push_str("<row r=\"8\" hidden=\"1\"><c r=\"A8\"><v>8</v></c></row>");
    let sheet = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision"><sheetPr filterMode="1"/><dimension ref="A1:C8"/><sheetData>{rows}</sheetData>{filter}<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#
    );
    let workbook = format!(
        r#"{DECL}<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Liste" sheetId="1" r:id="rId1"/></sheets><definedNames><definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">Liste!$A$1:$C$6</definedName><definedName name="Teams">Liste!$B$2:$B$6</definedName></definedNames></workbook>"#
    );
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
    );
    package(&[
        ("[Content_Types].xml", &content_types),
        (
            "_rels/.rels",
            &relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", &workbook),
        (
            "xl/_rels/workbook.xml.rels",
            &relationships(&[("rId1", "worksheet", "worksheets/sheet1.xml")]),
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

#[test]
fn sheet_filters_from_excel() {
    let wb = load(&excel_package(excel_filter()));
    let sheet = &wb.sheets[0];
    let mut expected = AutoFilter::new(rect("A1:C6"));
    expected.columns.insert(
        1,
        Criteria::Values(ValueFilter {
            values: vec!["Red".into()],
            blanks: false,
            dates: Vec::new(),
        }),
    );
    expected.sort = Some(SortState {
        levels: vec![SortLevel::values(0, false)],
        case_sensitive: false,
    });
    // The filter on icons is not one the engine has; its column is free.
    assert_eq!(sheet.auto_filter, Some(expected));
    // The file is the truth for what is hidden, and says by which hand.
    let rows = |set: &std::collections::BTreeSet<u32>| set.iter().copied().collect::<Vec<u32>>();
    assert_eq!(rows(&sheet.filtered_rows), [2, 4]);
    assert_eq!(rows(&sheet.hidden_rows), [7]);
    // Only the user's own name is a name.
    let names: Vec<String> = wb
        .all_names()
        .into_iter()
        .map(|(_, n)| n.name.clone())
        .collect();
    assert_eq!(names, ["Teams"]);
    assert_eq!(value(&wb, 0, "C6"), Value::Number(6.0));

    // Saved by us and read again, nothing moves.
    let again = load(&to_bytes(&wb));
    assert_eq!(again.sheets[0].auto_filter, sheet.auto_filter);
    assert_eq!(again.sheets[0].filtered_rows, sheet.filtered_rows);
    assert_eq!(again.sheets[0].hidden_rows, sheet.hidden_rows);

    // A filter without criteria hides nothing: rows hidden inside it were
    // hidden by hand.
    let plain = load(&excel_package("<autoFilter ref=\"A1:C6\"/>"));
    assert!(plain.sheets[0].filtered_rows.is_empty());
    assert_eq!(rows(&plain.sheets[0].hidden_rows), [2, 4, 7]);
    // No filter at all, or one that is nowhere: the sheet is in filter
    // mode and its list has a name, which is how Excel keeps an Advanced
    // Filter applied in place. The rows of the list are hidden by it.
    for none in [
        "",
        "<autoFilter/>",
        "<autoFilter ref=\"nowhere\"><filterColumn colId=\"0\"/></autoFilter>",
    ] {
        let wb = load(&excel_package(none));
        assert!(wb.sheets[0].auto_filter.is_none(), "{none}");
        let advanced = wb.sheets[0]
            .advanced_filter
            .as_ref()
            .expect("an Advanced Filter");
        assert_eq!(advanced.range, rect("A1:C6"), "{none}");
        assert_eq!(rows(&wb.sheets[0].filtered_rows), [2, 4], "{none}");
        assert_eq!(rows(&wb.sheets[0].hidden_rows), [7], "{none}");
    }
}

#[test]
fn damaged_sheet_filters_never_panic() {
    // Cut the element short at every character: the sheet loads, with
    // what was whole.
    let filter = excel_filter();
    let mut kept = 0;
    for cut in (0..filter.len()).filter(|i| filter.is_char_boundary(*i)) {
        let wb = load(&excel_package(&filter[..cut]));
        assert_eq!(wb.sheets.len(), 1, "cut at {cut}");
        assert_eq!(value(&wb, 0, "A1"), Value::Text("Name".into()));
        kept += usize::from(wb.sheets[0].auto_filter.is_some());
        if cut % 23 == 0 {
            load(&to_bytes(&wb));
        }
    }
    assert!(kept > 300, "{kept}");
}
