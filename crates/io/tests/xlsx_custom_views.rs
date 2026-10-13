//! Custom views in `.xlsx` files: `<customWorkbookViews>`, the
//! `<customSheetViews>` of the worksheets and the hidden names that go
//! with them, written as Excel writes them and read back.

mod common;

use common::{assert_well_formed, enter, part};
use opensheet_engine::custom_view::{ViewKind, WindowState};
use opensheet_engine::filter::{Criteria, FilterScope, ValueFilter};
use opensheet_engine::page::Orientation;
use opensheet_engine::{Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Two sheets; the first has a filter that hides rows, hidden rows and
/// columns, frozen panes and a landscape page with a print area.
fn workbook() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    for (i, text) in ["Fruit", "apple", "pear", "apple", "fig", "pear"].into_iter().enumerate() {
        enter(&mut wb, 0, &format!("A{}", i + 1), text);
        enter(&mut wb, 0, &format!("B{}", i + 1), if i == 0 { "Count" } else { "3" });
    }
    wb.set_auto_filter(0, Rect::new(0, 0, 5, 1)).unwrap();
    let apples = Criteria::Values(ValueFilter {
        values: vec!["apple".to_string()],
        blanks: false,
        dates: Vec::new(),
    });
    assert!(wb.set_filter_criteria(0, FilterScope::Sheet, 0, Some(apples)));
    wb.snapshot();
    let sheet = &mut wb.sheets[0];
    sheet.hidden_rows.extend([9, 10, 11, 20]);
    sheet.hidden_cols.extend([3, 4]);
    sheet.frozen_rows = 1;
    sheet.frozen_cols = 1;
    sheet.show_gridlines = false;
    sheet.page.orientation = Orientation::Landscape;
    sheet.page.print_area = vec![Rect::new(0, 0, 5, 1)];
    sheet.page.title_rows = Some((0, 0));
    wb.commit("setup");
    wb
}

fn windows() -> Vec<WindowState> {
    let first = WindowState {
        zoom: 85,
        active: (2, 1),
        ranges: vec![Rect::new(1, 0, 1, 0), Rect::new(2, 1, 4, 1)],
        top_left: (3, 2),
        mode: ViewKind::PageBreakPreview,
        ..WindowState::default()
    };
    let second = WindowState {
        selected: true,
        top_left: (40, 3),
        active: (41, 4),
        ranges: vec![Rect::cell(41, 4)],
        ..WindowState::default()
    };
    vec![first, second]
}

#[test]
fn views_come_back_whole() {
    let mut wb = workbook();
    wb.add_custom_view("Apples, landscape", true, true, &windows()).unwrap();
    wb.add_custom_view("Window only", false, false, &windows()).unwrap();
    wb.commit("Custom View");

    let bytes = to_bytes(&wb);
    common::keep_sample("custom-views.xlsx", &bytes);
    let book = part(&bytes, "xl/workbook.xml");
    let sheet1 = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("workbook.xml", &book);
    assert_well_formed("sheet1.xml", &sheet1);
    // The workbook part lists the views after the calculation properties.
    assert!(book.contains("<customWorkbookViews><customWorkbookView name=\"Apples, landscape\" guid=\"{"), "{book}");
    assert!(book.contains("includePrintSettings=\"0\" includeHiddenRowCol=\"0\""), "{book}");
    assert!(book.contains("activeSheetId=\"2\"/>"), "{book}");
    assert!(book.find("<calcPr").unwrap() < book.find("<customWorkbookViews>").unwrap());
    // The hidden names of the first view, on the first sheet.
    for (suffix, text) in [
        ("Rows", "Sheet1!$10:$12,Sheet1!$21:$21"),
        ("Cols", "Sheet1!$D:$E"),
        ("FilterData", "Sheet1!$A$1:$B$6"),
        ("PrintArea", "Sheet1!$A$1:$B$6"),
        ("PrintTitles", "Sheet1!$1:$1"),
    ] {
        let wanted = format!("_.wvu.{suffix}\" localSheetId=\"0\" hidden=\"1\">{text}</definedName>");
        assert!(book.contains(&wanted), "{wanted} in {book}");
    }
    // The sheet's part: after the filter, before the page setup.
    let views = sheet1.find("<customSheetViews>").expect("custom sheet views");
    assert!(sheet1.find("<autoFilter").unwrap() < views);
    assert!(views < sheet1.find("<pageMargins").unwrap());
    assert!(sheet1.contains(" scale=\"85\" showGridLines=\"0\" filter=\"1\" showAutoFilter=\"1\" hiddenRows=\"1\" hiddenColumns=\"1\" view=\"pageBreakPreview\">"), "{sheet1}");
    assert!(sheet1.contains("<pane xSplit=\"1\" ySplit=\"1\" topLeftCell=\"C4\" activePane=\"bottomRight\" state=\"frozen\"/><selection pane=\"bottomRight\" activeCell=\"B3\" sqref=\"A2 B3:B5\"/>"), "{sheet1}");
    assert!(part(&bytes, "xl/worksheets/sheet2.xml").contains(" topLeftCell=\"D41\"><selection activeCell=\"E42\" sqref=\"E42\"/>"));

    let back = load(&bytes);
    assert_eq!(back.custom_views(), wb.custom_views());
    for (was, is) in wb.sheets.iter().zip(&back.sheets) {
        assert_eq!(is.custom_views, was.custom_views, "sheet {}", was.name);
    }
    // The names of the views are not names of the workbook.
    assert!(back.all_names().is_empty());
    // Written again, nothing changes.
    let again = to_bytes(&back);
    assert_eq!(part(&again, "xl/workbook.xml"), book);
    assert_eq!(part(&again, "xl/worksheets/sheet1.xml"), sheet1);
}

#[test]
fn a_view_read_from_a_file_can_be_shown() {
    let mut wb = workbook();
    wb.add_custom_view("Apples", true, true, &windows()).unwrap();
    // Since the view was made: no filter, nothing hidden, portrait.
    wb.remove_auto_filter(0);
    wb.snapshot();
    let sheet = &mut wb.sheets[0];
    sheet.hidden_rows.clear();
    sheet.hidden_cols.clear();
    sheet.page.orientation = Orientation::Portrait;
    sheet.page.print_area.clear();
    wb.commit("changes");

    let mut back = load(&to_bytes(&wb));
    assert!(back.sheets[0].auto_filter.is_none());
    let shown = back.show_custom_view("apples").expect("the view");
    back.commit("Custom View");
    let sheet = &back.sheets[0];
    assert_eq!(sheet.hidden_rows.iter().copied().collect::<Vec<_>>(), vec![9, 10, 11, 20]);
    assert_eq!(sheet.hidden_cols.iter().copied().collect::<Vec<_>>(), vec![3, 4]);
    // The filter is back and hides the rows that are not apples.
    assert_eq!(sheet.filtered_rows.iter().copied().collect::<Vec<_>>(), vec![2, 4, 5]);
    assert_eq!(sheet.page.orientation, Orientation::Landscape);
    assert_eq!(sheet.page.print_area, vec![Rect::new(0, 0, 5, 1)]);
    assert_eq!(shown.len(), 2);
    assert_eq!((shown[0].1.zoom, shown[0].1.mode, shown[1].1.selected), (85, ViewKind::PageBreakPreview, true));
}

#[test]
fn parts_without_a_view_are_dropped() {
    let mut wb = workbook();
    wb.add_custom_view("Kept", false, false, &windows()).unwrap();
    wb.commit("Custom View");
    let bytes = to_bytes(&wb);
    // The same package without the list of the workbook part.
    let book = part(&bytes, "xl/workbook.xml");
    let start = book.find("<customWorkbookViews>").unwrap();
    let end = book.find("</customWorkbookViews>").unwrap() + "</customWorkbookViews>".len();
    let stripped = format!("{}{}", &book[..start], &book[end..]);
    let mut parts: Vec<(String, String)> = common::part_names(&bytes).into_iter().map(|name| (name.clone(), part(&bytes, &name))).collect();
    for (name, text) in &mut parts {
        if name == "xl/workbook.xml" {
            *text = stripped.clone();
        }
    }
    let borrowed: Vec<(&str, &str)> = parts.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
    let back = load(&common::package(&borrowed));
    assert!(back.custom_views().is_empty());
    assert!(back.sheets.iter().all(|sheet| sheet.custom_views.is_empty()));
}
