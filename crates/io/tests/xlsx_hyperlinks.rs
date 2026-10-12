//! Hyperlinks in `.xlsx` files: our own round trip, links as Excel writes
//! them (with their relationships), and damaged input.

mod common;

use common::{assert_well_formed, assert_worksheet_order, enter, package, part};
use opensheet_engine::hyperlink::{Hyperlink, LinkKind};
use opensheet_engine::{Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    enter(&mut wb, 0, "A1", "Site");
    enter(&mut wb, 0, "A2", "Totals");
    enter(&mut wb, 0, "A3", "Ann");
    enter(&mut wb, 0, "A5", "note");
    wb.set_note(0, 4, 0, "Me", "a note, for a second relationship");
    let mut site = Hyperlink::external(rect("A1"), "https://example.org/a?b=1&c=2#part");
    site.tooltip = "Open the \"site\" <now>".to_string();
    wb.set_hyperlink(0, site);
    wb.set_hyperlink(0, Hyperlink::place(rect("A2:B2"), "'Sheet2'!C3"));
    wb.set_hyperlink(0, Hyperlink::email(rect("A3"), "ann@example.org", "Budget & plans"));
    wb.set_hyperlink(0, Hyperlink::external(rect("A4"), "Reports/2024 final.xlsx"));
    wb.set_hyperlink(1, Hyperlink::place(rect("B2"), "Sheet1!A1"));
    wb.commit("links");
    wb
}

#[test]
fn links_survive_a_round_trip() {
    let wb = sample();
    let bytes = to_bytes(&wb);
    common::keep_sample("hyperlinks.xlsx", &bytes);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert_worksheet_order("sheet1.xml", &sheet);
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    assert_well_formed("sheet1.xml.rels", &rels);
    // The addresses are relationships leading outside the package, after
    // those of the notes; a place needs none.
    assert!(sheet.contains("<hyperlink ref=\"A1\" r:id=\"rId3\" location=\"part\" tooltip=\"Open the &quot;site&quot; &lt;now&gt;\"/>"), "{sheet}");
    assert!(sheet.contains("<hyperlink ref=\"A2:B2\" location=\"'Sheet2'!C3\"/>"), "{sheet}");
    assert!(rels.contains(
        "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"https://example.org/a?b=1&amp;c=2\" TargetMode=\"External\"/>"
    ), "{rels}");
    assert_eq!(rels.matches("TargetMode=\"External\"").count(), 3);
    // The second sheet has a link and no relationship at all.
    assert!(part(&bytes, "xl/worksheets/sheet2.xml").contains("<hyperlinks><hyperlink ref=\"B2\" location=\"Sheet1!A1\"/></hyperlinks>"));
    assert!(!common::part_names(&bytes).iter().any(|name| name.ends_with("sheet2.xml.rels")));

    let back = load(&bytes);
    assert_eq!(back.sheets[0].hyperlinks, wb.sheets[0].hyperlinks);
    assert_eq!(back.sheets[1].hyperlinks, wb.sheets[1].hyperlinks);
    assert_eq!(back.hyperlink_at(0, 2, 0).and_then(Hyperlink::email_parts), Some(("ann@example.org".into(), "Budget & plans".into())));
    // The note still has its parts.
    assert!(back.comment_at(0, 4, 0).is_some());
    // And once more, unchanged.
    assert_eq!(part(&to_bytes(&back), "xl/worksheets/sheet1.xml"), sheet);
}

const WORKBOOK: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Feuil1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;

/// A worksheet with four links in the form Excel gives them: revision ids,
/// `display` texts, relationship ids out of order.
const EXCEL_SHEET: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr xr2 xr3" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision"><dimension ref="A1:A4"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr baseColWidth="10" defaultRowHeight="16" x14ac:dyDescent="0.2"/><sheetData><row r="1" spans="1:1"><c r="A1" t="inlineStr"><is><t>Microsoft</t></is></c></row></sheetData><hyperlinks><hyperlink ref="A1" r:id="rId1" tooltip="Le site" xr:uid="{00000000-0004-0000-0000-000000000000}"/><hyperlink ref="A2" location="Feuil1!C10" display="Feuil1!C10" xr:uid="{00000000-0004-0000-0000-000001000000}"/><hyperlink ref="A3" r:id="rId2" xr:uid="{00000000-0004-0000-0000-000002000000}"/><hyperlink ref="A4" r:id="rId3" location="Plan" display="Autre classeur" xr:uid="{00000000-0004-0000-0000-000003000000}"/></hyperlinks><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"#;

const EXCEL_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="Budget.xlsx" TargetMode="External"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="mailto:ann@example.org?subject=Bonjour%20Ann" TargetMode="External"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://www.microsoft.com/" TargetMode="External"/></Relationships>"#;

#[test]
fn links_from_excel() {
    let bytes = package(&[
        ("xl/workbook.xml", WORKBOOK),
        ("xl/worksheets/sheet1.xml", EXCEL_SHEET),
        ("xl/worksheets/_rels/sheet1.xml.rels", EXCEL_RELS),
    ]);
    let wb = load(&bytes);
    let links = &wb.sheets[0].hyperlinks;
    let seen: Vec<(String, LinkKind, &str, &str, &str)> = links
        .iter()
        .map(|link| (link.range.name(), link.kind(), link.address.as_str(), link.location.as_str(), link.tooltip.as_str()))
        .collect();
    assert_eq!(
        seen,
        [
            ("A1".to_string(), LinkKind::Web, "https://www.microsoft.com/", "", "Le site"),
            ("A2".to_string(), LinkKind::Place, "", "Feuil1!C10", ""),
            ("A3".to_string(), LinkKind::Email, "mailto:ann@example.org?subject=Bonjour%20Ann", "", ""),
            ("A4".to_string(), LinkKind::File, "Budget.xlsx", "Plan", ""),
        ]
    );
    assert_eq!(links[2].email_parts(), Some(("ann@example.org".into(), "Bonjour Ann".into())));
    // Written again, the same links come back.
    let again = load(&to_bytes(&wb));
    assert_eq!(&again.sheets[0].hyperlinks, links);
}

#[test]
fn damaged_links_are_dropped() {
    let sheet = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData/><hyperlinks><hyperlink ref="A1" r:id="rId9"/><hyperlink ref="nowhere" location="A1"/><hyperlink location="A1"/><hyperlink ref="B2" location="Feuil1!A1"/><hyperlink ref="B3" r:id="rId1""#;
    let bytes = package(&[("xl/workbook.xml", WORKBOOK), ("xl/worksheets/sheet1.xml", sheet)]);
    let wb = load(&bytes);
    let ranges: Vec<String> = wb.sheets[0].hyperlinks.iter().map(|link| link.range.name()).collect();
    assert_eq!(ranges, ["B2"]);
}
