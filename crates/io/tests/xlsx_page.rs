//! Page setup in `.xlsx` files: paper, margins, scaling, headers and
//! footers, page breaks, print area and print titles; our own round trip,
//! sheets as Excel writes them, and damaged input.

mod common;

use common::{
    assert_well_formed, assert_workbook_order, assert_worksheet_order, enter, package, part, value,
};
use opensheet_engine::page::{
    Margins, Orientation, PageOrder, PageSetup, PaginateOptions, PaperSize, PrintComments,
    PrintErrors, paginate,
};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
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

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

/// Three sheets: one set up in every way, one with a few settings, one
/// left alone.
fn sample() -> Workbook {
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Report"));
    assert!(wb.rename_sheet(1, "Joe's data"));
    assert!(wb.rename_sheet(2, "Plain"));
    for row in 0..60u32 {
        for col in 0..12u32 {
            wb.set_value(0, row, col, Value::Number((row * 12 + col) as f64));
        }
    }
    enter(&mut wb, 1, "A1", "x");
    enter(&mut wb, 2, "A1", "y");
    wb.set_tab_color(0, Some(Color(0x00B050)));
    wb.define_name("Rate", parse_formula("0.2", en).unwrap());

    assert!(wb.set_print_area(0, &[rect("A1:L40"), rect("A45:F60")]));
    assert!(wb.set_print_titles(0, Some((0, 1)), Some((0, 0))));
    assert!(wb.insert_page_break(0, Some(20), None));
    assert!(wb.insert_page_break(0, Some(30), Some(6)));
    assert!(wb.update_page_setup(0, |page| {
        page.paper = Some(PaperSize::A4);
        page.orientation = Orientation::Landscape;
        page.scale = 75;
        page.fit_to_page = true;
        page.fit_width = 1;
        page.fit_height = 0;
        page.first_page_number = Some(3);
        page.page_order = PageOrder::OverThenDown;
        page.horizontal_dpi = Some(600);
        page.vertical_dpi = Some(600);
        page.black_and_white = true;
        page.draft = true;
        page.comments = PrintComments::AsDisplayed;
        page.errors = PrintErrors::NA;
        page.center_horizontally = true;
        page.gridlines = true;
        page.headings = true;
        page.margins = Margins::NARROW;
        let hf = &mut page.header_footer;
        hf.odd_header = "&L&\"Arial,Bold\"&14Report &A&C&D &T&R&F".into();
        hf.odd_footer = "&CPage &P of &N".into();
        hf.even_header = "&LEven <page> & \"more\"".into();
        hf.even_footer = "&Rline 1\nline 2".into();
        hf.first_header = "&C&KFF0000First".into();
        hf.first_footer = "&L&G".into();
        hf.different_odd_even = true;
        hf.different_first = true;
        hf.scale_with_doc = false;
        hf.align_with_margins = false;
    }));
    assert!(wb.set_print_area(1, &[rect("B2:D9")]));
    assert!(wb.set_print_titles(1, None, Some((0, 2))));
    assert!(wb.insert_page_break(1, None, Some(4)));
    assert!(wb.update_page_setup(1, |page| {
        page.margins = Margins {
            left: 1.25,
            header: 0.0,
            ..Margins::WIDE
        };
        page.header_footer.odd_footer = "&P".into();
        page.center_vertically = true;
    }));
    wb.commit("page setup");
    wb
}

#[test]
fn page_setup_round_trips() {
    let original = sample();
    let bytes = to_bytes(&original);
    common::keep_sample("page-setup.xlsx", &bytes);
    let loaded = load(&bytes);
    for (a, b) in original.sheets.iter().zip(&loaded.sheets) {
        assert_eq!(a.page, b.page, "{}", a.name);
    }
    assert_eq!(loaded.sheets[2].page, PageSetup::default());
    assert_eq!(loaded.sheets[0].tab_color, Some(Color(0x00B050)));
    assert!(!loaded.modified && !loaded.can_undo());

    // The print area and the print titles are settings, not names: the
    // names of the workbook are the ones the user defined.
    assert_eq!(loaded.names.keys().collect::<Vec<_>>(), ["RATE"]);
    assert!(loaded.sheets.iter().all(|sheet| sheet.names.is_empty()));
    assert_eq!(loaded.all_names().len(), 1);

    // The setup is alive: both workbooks print the same pages.
    let pages = |wb: &Workbook| {
        let pagination = paginate(wb, 0, &PaginateOptions::default());
        let pages: Vec<_> = pagination
            .pages
            .iter()
            .map(|page| (page.number, page.rows, page.cols, page.title_rows, page.title_cols))
            .collect();
        (pagination.paper, pagination.scale_percent, pages)
    };
    let printed = pages(&loaded);
    assert_eq!(printed, pages(&original));
    assert!(printed.2.len() >= 3, "{printed:?}");
    assert_eq!(printed.2[0].0, 3, "the first page number");
    // The first page holds the title rows themselves; the next repeats them.
    assert_eq!(printed.2[1].3, Some((0, 1)));

    // A second generation is the same file.
    let again = to_bytes(&loaded);
    for name in ["xl/workbook.xml", "xl/worksheets/sheet1.xml", "xl/worksheets/sheet2.xml"] {
        assert_eq!(part(&bytes, name), part(&again, name), "{name}");
    }
}

#[test]
fn page_setup_is_written_the_way_excel_writes_it() {
    let bytes = to_bytes(&sample());
    for n in 1..=3 {
        let name = format!("xl/worksheets/sheet{n}.xml");
        let sheet = part(&bytes, &name);
        assert_well_formed(&name, &sheet);
        assert_worksheet_order(&name, &sheet);
    }
    let workbook = part(&bytes, "xl/workbook.xml");
    assert_well_formed("workbook.xml", &workbook);
    assert_workbook_order(&workbook);

    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains("<sheetPr><tabColor rgb=\"FF00B050\"/><pageSetUpPr fitToPage=\"1\"/></sheetPr><dimension "),
        "{sheet}"
    );
    let from = sheet.find("</sheetData>").unwrap();
    assert_eq!(
        &sheet[from..],
        concat!(
            "</sheetData>",
            "<printOptions horizontalCentered=\"1\" headings=\"1\" gridLines=\"1\"/>",
            "<pageMargins left=\"0.25\" right=\"0.25\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>",
            "<pageSetup paperSize=\"9\" scale=\"75\" firstPageNumber=\"3\" fitToHeight=\"0\" pageOrder=\"overThenDown\" ",
            "orientation=\"landscape\" blackAndWhite=\"1\" draft=\"1\" cellComments=\"asDisplayed\" ",
            "useFirstPageNumber=\"1\" errors=\"NA\" horizontalDpi=\"600\" verticalDpi=\"600\"/>",
            "<headerFooter differentOddEven=\"1\" differentFirst=\"1\" scaleWithDoc=\"0\" alignWithMargins=\"0\">",
            "<oddHeader>&amp;L&amp;\"Arial,Bold\"&amp;14Report &amp;A&amp;C&amp;D &amp;T&amp;R&amp;F</oddHeader>",
            "<oddFooter>&amp;CPage &amp;P of &amp;N</oddFooter>",
            "<evenHeader>&amp;LEven &lt;page&gt; &amp; \"more\"</evenHeader>",
            "<evenFooter>&amp;Rline 1\nline 2</evenFooter>",
            "<firstHeader>&amp;C&amp;KFF0000First</firstHeader><firstFooter>&amp;L&amp;G</firstFooter></headerFooter>",
            "<rowBreaks count=\"2\" manualBreakCount=\"2\"><brk id=\"20\" max=\"16383\" man=\"1\"/><brk id=\"30\" max=\"16383\" man=\"1\"/></rowBreaks>",
            "<colBreaks count=\"1\" manualBreakCount=\"1\"><brk id=\"6\" max=\"1048575\" man=\"1\"/></colBreaks>",
            "</worksheet>"
        )
    );
    let sheet2 = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(!sheet2.contains("<sheetPr") && !sheet2.contains("<pageSetup"));
    assert!(sheet2.ends_with(concat!(
        "</sheetData><printOptions verticalCentered=\"1\"/>",
        "<pageMargins left=\"1.25\" right=\"1\" top=\"1\" bottom=\"1\" header=\"0\" footer=\"0.5\"/>",
        "<headerFooter><oddFooter>&amp;P</oddFooter></headerFooter>",
        "<colBreaks count=\"1\" manualBreakCount=\"1\"><brk id=\"4\" max=\"1048575\" man=\"1\"/></colBreaks></worksheet>"
    )));
    // A sheet nobody set up has its margins and nothing else.
    let sheet3 = part(&bytes, "xl/worksheets/sheet3.xml");
    assert!(sheet3.ends_with(
        "</sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>"
    ));

    // The print areas and titles are names of the workbook part with
    // reserved spellings, each for its sheet.
    assert!(workbook.contains(concat!(
        "<definedNames>",
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">Report!$A$1:$L$40,Report!$A$45:$F$60</definedName>",
        "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"0\">Report!$A:$A,Report!$1:$2</definedName>",
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"1\">'Joe''s data'!$B$2:$D$9</definedName>",
        "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"1\">'Joe''s data'!$A:$C</definedName>",
        "<definedName name=\"Rate\">0.2</definedName></definedNames>"
    )), "{workbook}");

    // Without any, and without names, the element is not there.
    let plain = to_bytes(&Workbook::new(Locale::en()));
    assert!(!part(&plain, "xl/workbook.xml").contains("definedNames"));
    let mut only_area = Workbook::new(Locale::en());
    assert!(only_area.set_print_area(0, &[rect("A1:B2")]));
    only_area.commit("area");
    assert!(part(&to_bytes(&only_area), "xl/workbook.xml").contains(
        "<definedNames><definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">Sheet1!$A$1:$B$2</definedName></definedNames>"
    ));
}

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

/// A workbook as Excel saves it. The chart sheet between the worksheets
/// makes `localSheetId` differ from the position among the sheets that
/// are read.
fn excel_workbook() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {MAIN}><workbookPr defaultThemeVersion="166925"/>
<bookViews><workbookView xWindow="0" yWindow="0" windowWidth="28800" windowHeight="17600"/></bookViews>
<sheets><sheet name="Ventes 2025" sheetId="1" r:id="rId1"/><sheet name="Graphique" sheetId="2" r:id="rId2"/><sheet name="Détail" sheetId="3" r:id="rId3"/><sheet name="Vide" sheetId="4" r:id="rId4"/></sheets>
<definedNames>
<definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">'Ventes 2025'!$A$1:$D$40</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="0">'Ventes 2025'!$A$1:$H$50</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="2">Détail!$B$2:$E$20,Détail!$G$2:$H$20</definedName>
<definedName name="_xlnm.Print_Titles" localSheetId="0">'Ventes 2025'!$A:$B,'Ventes 2025'!$1:$3</definedName>
<definedName name="_xlnm.Print_Titles" localSheetId="2">Détail!$1:$1</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="3">#REF!</definedName>
<definedName name="_xlnm.Print_Titles" localSheetId="3">Vide!$A$1:$B$2</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="1">Graphique!$A$1</definedName>
<definedName name="_xlnm.Print_Area">'Ventes 2025'!$Z$1:$Z$2</definedName>
<definedName name="Taux">0.2</definedName>
</definedNames><calcPr calcId="191029"/></workbook>"#
    )
}

fn excel_rels() -> String {
    let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{rel}/chartsheet" Target="chartsheets/sheet1.xml"/><Relationship Id="rId3" Type="{rel}/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId4" Type="{rel}/worksheet" Target="worksheets/sheet3.xml"/></Relationships>"#
    )
}

/// The first sheet: set up through the Page Setup dialog, saved by a
/// French Excel on a machine with a printer.
fn excel_sheet() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr xr2 xr3" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac">
<sheetPr filterMode="1"><tabColor theme="5"/><outlinePr summaryBelow="0"/><pageSetUpPr fitToPage="1"/></sheetPr>
<dimension ref="A1:H50"/>
<sheetViews><sheetView showGridLines="0" tabSelected="1" view="pageBreakPreview" zoomScale="60" zoomScaleNormal="100" workbookViewId="0"/></sheetViews>
<sheetFormatPr baseColWidth="10" defaultRowHeight="15" x14ac:dyDescent="0.25"/>
<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
<printOptions horizontalCentered="1" gridLines="1"/>
<pageMargins left="0.70866141732283472" right="0.70866141732283472" top="0.74803149606299213" bottom="0.74803149606299213" header="0.31496062992125984" footer="0.31496062992125984"/>
<pageSetup paperSize="9" scale="67" fitToHeight="0" orientation="landscape" horizontalDpi="4294967293" verticalDpi="0" r:id="rId1"/>
<headerFooter differentFirst="1" scaleWithDoc="0"><oddHeader>&amp;L&amp;"-,Gras"Ventes&amp;R&amp;D</oddHeader><oddFooter>&amp;CPage &amp;P sur &amp;N</oddFooter><firstHeader>&amp;C&amp;G</firstHeader></headerFooter>
<rowBreaks count="2" manualBreakCount="2"><brk id="25" max="7" man="1"/><brk id="40" max="16383" man="1"/></rowBreaks>
<colBreaks count="1" manualBreakCount="1"><brk id="4" max="49" man="1"/></colBreaks>
<legacyDrawingHF r:id="rId2"/>
</worksheet>"#
    )
}

fn excel_file(sheet1: &str) -> Vec<u8> {
    let detail = format!(
        r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>2</v></c></row></sheetData><pageMargins left="0.25" right="0.25" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup paperSize="8" orientation="portrait" useFirstPageNumber="1" firstPageNumber="4294967295" pageOrder="overThenDown" cellComments="atEnd" errors="blank" blackAndWhite="1"/></worksheet>"#
    );
    let empty = format!(
        r#"<worksheet {MAIN}><sheetData/><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><pageSetup orientation="landscape"/></worksheet>"#
    );
    package(&[
        ("xl/workbook.xml", &excel_workbook()),
        ("xl/_rels/workbook.xml.rels", &excel_rels()),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/chartsheets/sheet1.xml", "<chartsheet/>"),
        ("xl/worksheets/sheet2.xml", &detail),
        ("xl/worksheets/sheet3.xml", &empty),
    ])
}

#[test]
fn page_setup_from_excel() {
    let wb = load(&excel_file(&excel_sheet()));
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Ventes 2025", "Détail", "Vide"]);

    let page = &wb.sheets[0].page;
    assert_eq!(page.paper, Some(PaperSize::A4));
    assert_eq!(page.orientation, Orientation::Landscape);
    assert!(page.fit_to_page);
    assert_eq!((page.scale, page.fit_width, page.fit_height), (67, 1, 0));
    assert_eq!(page.first_page_number, None);
    assert_eq!((page.horizontal_dpi, page.vertical_dpi), (None, None));
    assert!(page.center_horizontally && !page.center_vertically);
    assert!(page.gridlines && !page.headings);
    // 1.8 cm and 1.9 cm, as a metric Excel stores its normal margins.
    assert!((page.margins.left - 0.7086614).abs() < 1e-6);
    assert!((page.margins.top - 0.7480315).abs() < 1e-6);
    assert!((page.margins.header - 0.3149606).abs() < 1e-6);
    let hf = &page.header_footer;
    assert_eq!(hf.odd_header, "&L&\"-,Gras\"Ventes&R&D");
    assert_eq!(hf.odd_footer, "&CPage &P sur &N");
    assert_eq!(hf.first_header, "&C&G");
    assert!(hf.different_first && !hf.different_odd_even);
    assert!(!hf.scale_with_doc && hf.align_with_margins);
    assert!(hf.pictures.is_empty(), "header pictures are not read");
    assert_eq!(page.row_breaks.iter().copied().collect::<Vec<_>>(), [25, 40]);
    assert_eq!(page.col_breaks.iter().copied().collect::<Vec<_>>(), [4]);
    assert_eq!(page.print_area, [rect("A1:H50")]);
    assert_eq!((page.title_rows, page.title_cols), (Some((0, 2)), Some((0, 1))));
    assert_eq!(wb.sheets[0].tab_color, Some(Color(0xED7D31)));

    // The names say which sheet they are for by its place in the file,
    // chart sheets included.
    let detail = &wb.sheets[1].page;
    assert_eq!(detail.print_area, [rect("B2:E20"), rect("G2:H20")]);
    assert_eq!((detail.title_rows, detail.title_cols), (Some((0, 0)), None));
    assert_eq!(detail.paper, Some(PaperSize(8)));
    assert_eq!(detail.first_page_number, Some(-1));
    assert_eq!(detail.page_order, PageOrder::OverThenDown);
    assert_eq!(detail.comments, PrintComments::AtEnd);
    assert_eq!(detail.errors, PrintErrors::Blank);
    assert!(detail.black_and_white && !detail.fit_to_page);
    assert_eq!(detail.margins, Margins::NARROW);

    // A print area that was deleted, titles that are not whole rows or
    // columns, a paper left to the printer.
    let empty = &wb.sheets[2].page;
    assert!(empty.print_area.is_empty());
    assert_eq!((empty.title_rows, empty.title_cols), (None, None));
    assert_eq!(empty.paper, None);
    assert_eq!(empty.orientation, Orientation::Landscape);

    // Only the name the user defined is a name.
    assert_eq!(wb.names.keys().collect::<Vec<_>>(), ["TAUX"]);
    assert!(wb.sheets.iter().all(|sheet| sheet.names.is_empty()));

    // Saved again: the same setup, the names under the new positions.
    let saved = to_bytes(&wb);
    common::keep_sample("page-setup-from-excel.xlsx", &saved);
    let workbook = part(&saved, "xl/workbook.xml");
    assert!(workbook.contains(concat!(
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"0\">'Ventes 2025'!$A$1:$H$50</definedName>",
        "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"0\">'Ventes 2025'!$A:$B,'Ventes 2025'!$1:$3</definedName>",
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"1\">Détail!$B$2:$E$20,Détail!$G$2:$H$20</definedName>",
        "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"1\">Détail!$1:$1</definedName>",
        "<definedName name=\"Taux\">0.2</definedName>"
    )), "{workbook}");
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1.xml", &sheet);
    assert!(sheet.contains("<pageSetup paperSize=\"9\" scale=\"67\" fitToHeight=\"0\" orientation=\"landscape\"/>"));
    assert!(sheet.contains("<pageMargins left=\"0.7086614173228347\" right=\"0.7086614173228347\" top=\"0.7480314960629921\""));
    assert!(part(&saved, "xl/worksheets/sheet3.xml").contains("<pageSetup orientation=\"landscape\"/>"));
    let again = load(&saved);
    for (a, b) in wb.sheets.iter().zip(&again.sheets) {
        assert_eq!(a.page, b.page, "{}", a.name);
    }
}

#[test]
fn damaged_page_setup_costs_nothing_else() {
    // Cut the sheet short everywhere after its cells.
    let sheet = excel_sheet();
    let start = sheet.find("<printOptions").unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for cut in (start..sheet.len()).filter(|i| sheet.is_char_boundary(*i)) {
        let wb = load(&excel_file(&sheet[..cut]));
        assert_eq!(value(&wb, 0, "A1"), Value::Number(1.0));
        let page = &wb.sheets[0].page;
        // What comes from elsewhere is not affected.
        assert!(page.fit_to_page);
        assert_eq!(page.print_area, [rect("A1:H50")]);
        seen.insert((
            page.center_horizontally,
            page.paper.is_some(),
            page.header_footer.odd_footer.is_empty(),
            page.row_breaks.len(),
            page.col_breaks.len(),
        ));
        let saved = to_bytes(&wb);
        let xml = part(&saved, "xl/worksheets/sheet1.xml");
        assert_well_formed("sheet1.xml", &xml);
        assert_worksheet_order("sheet1.xml", &xml);
        assert_eq!(load(&saved).sheets[0].page, *page);
    }
    assert!(seen.len() >= 6, "{seen:?}");

    // Names that say nothing usable.
    let workbook = format!(
        r#"<workbook {MAIN}><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets><definedNames>
<definedName name="_xlnm.Print_Area" localSheetId="7">S!$A$1:$B$2</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="x">S!$A$1:$B$2</definedName>
<definedName name="_xlnm.Print_Area" localSheetId="0">S!$A$1:$B$2,OFFSET(S!$A$1,1,1)</definedName>
<definedName name="_xlnm.Print_Titles" localSheetId="0">S!$1:$2,S!$A:$A,S!$B$3</definedName>
<definedName name="_xlnm.Print_Titles" localSheetId="0"></definedName>
<definedName name="_xlnm.Print_Area" localSheetId="0">'S</definedName>
<definedName name="_XLNM.PRINT_TITLES" localSheetId="0">S!$3:$4</definedName>
</definedNames></workbook>"#
    );
    let sheet = format!(
        r#"<worksheet {MAIN}><sheetPr><pageSetUpPr fitToPage="perhaps"/><pageSetUpPr/></sheetPr><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><pageSetup/><pageSetup paperSize="70000" scale="9"/><headerFooter/><rowBreaks/><colBreaks count="x"><brk/></colBreaks></worksheet>"#
    );
    let wb = load(&package(&[
        ("xl/workbook.xml", &workbook),
        ("xl/worksheets/sheet1.xml", &sheet),
    ]));
    let page = &wb.sheets[0].page;
    assert!(page.print_area.is_empty());
    assert_eq!((page.title_rows, page.title_cols), (Some((2, 3)), None));
    assert!(!page.fit_to_page);
    assert_eq!((page.paper, page.scale), (None, 100));
    assert!(wb.names.is_empty() && wb.sheets[0].names.is_empty());
    assert_eq!(value(&wb, 0, "A1"), Value::Number(1.0));
}
