//! Tests of the page setup model, its edits and pagination.

use super::*;
use crate::chart::{Chart, ChartKind, ChartTitle, LegendPos};
use crate::style::{Font, HAlign};
use crate::value::Value;

/// An English workbook (Letter paper) with numbers in the first `rows`
/// rows and `cols` columns of sheet 0.
fn grid(rows: u32, cols: u32) -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    fill(&mut wb, rows, cols);
    wb
}

fn fill(wb: &mut Workbook, rows: u32, cols: u32) {
    for r in 0..rows {
        for c in 0..cols {
            wb.set_value(0, r, c, Value::Number((r * cols + c) as f64));
        }
    }
    wb.commit("setup");
}

fn pages(wb: &Workbook) -> Pagination {
    paginate(wb, 0, &PaginateOptions::default())
}

/// (first row, last row, first col, last col) of every page.
fn cells(p: &Pagination) -> Vec<(u32, u32, u32, u32)> {
    p.pages
        .iter()
        .map(|p| (p.rows.0, p.rows.1, p.cols.0, p.cols.1))
        .collect()
}

/// Every character is 7 pixels wide.
struct Fixed;

impl TextMeasure for Fixed {
    fn text_width(&self, text: &str, _font: &Font) -> f32 {
        text.chars().count() as f32 * 7.0
    }
}

#[test]
fn defaults_follow_excel() {
    let p = PageSetup::default();
    assert_eq!(p.paper_size(Locale::en()), PaperSize::LETTER);
    assert_eq!(p.paper_size(Locale::fr()), PaperSize::A4);
    assert_eq!(p.paper_size(Locale::de()), PaperSize::A4);
    assert_eq!(p.orientation, Orientation::Portrait);
    assert_eq!((p.scale, p.fit_to_page, p.fit_width, p.fit_height), (100, false, 1, 1));
    assert_eq!(p.margins, Margins::NORMAL);
    assert_eq!(p.margins.preset(), Some(MarginPreset::Normal));
    assert_eq!(Margins::WIDE.preset(), Some(MarginPreset::Wide));
    assert_eq!(Margins::from_preset(MarginPreset::Narrow).left, 0.25);
    assert_eq!(
        Margins {
            left: 0.5,
            ..Margins::NORMAL
        }
        .preset(),
        None
    );
    assert!(p.header_footer.scale_with_doc && p.header_footer.align_with_margins);
    assert_eq!(p.paper_pt(Locale::en()), (612.0, 792.0));
    let landscape = PageSetup {
        orientation: Orientation::Landscape,
        ..PageSetup::default()
    };
    assert_eq!(landscape.paper_pt(Locale::en()), (792.0, 612.0));
    // The OOXML spellings round-trip.
    for o in [PageOrder::DownThenOver, PageOrder::OverThenDown] {
        assert_eq!(PageOrder::from_ooxml(o.as_ooxml()), o);
    }
    for c in [PrintComments::None, PrintComments::AtEnd, PrintComments::AsDisplayed] {
        assert_eq!(PrintComments::from_ooxml(c.as_ooxml()), c);
    }
    for e in [
        PrintErrors::Displayed,
        PrintErrors::Blank,
        PrintErrors::Dash,
        PrintErrors::NA,
    ] {
        assert_eq!(PrintErrors::from_ooxml(e.as_ooxml()), e);
    }
    assert_eq!(Orientation::from_ooxml("landscape"), Orientation::Landscape);
    assert_eq!(Orientation::from_ooxml("default"), Orientation::Portrait);
}

#[test]
fn print_names_round_trip() {
    let mut p = PageSetup::default();
    assert_eq!(p.print_area_formula("Sheet1"), None);
    assert_eq!(p.print_titles_formula("Sheet1"), None);
    p.print_area = vec![Rect::new(0, 0, 8, 2), Rect::cell(4, 4)];
    p.title_rows = Some((0, 1));
    p.title_cols = Some((0, 0));
    assert_eq!(p.print_area_formula("Sheet1").unwrap(), "Sheet1!$A$1:$C$9,Sheet1!$E$5");
    assert_eq!(
        p.print_area_formula("Q1 'draft'").unwrap(),
        "'Q1 ''draft'''!$A$1:$C$9,'Q1 ''draft'''!$E$5"
    );
    assert_eq!(p.print_titles_formula("Sheet1").unwrap(), "Sheet1!$A:$A,Sheet1!$1:$2");

    let mut q = PageSetup::default();
    assert!(q.set_print_area_formula("'Q1, ''draft'''!$A$1:$C$9,'Q1, ''draft'''!$E$5"));
    assert_eq!(q.print_area, p.print_area);
    assert!(q.set_print_titles_formula("Sheet1!$A:$A,Sheet1!$1:$2"));
    assert_eq!((q.title_rows, q.title_cols), (Some((0, 1)), Some((0, 0))));
    assert!(q.set_print_titles_formula("Sheet1!$3:$4"));
    assert_eq!((q.title_rows, q.title_cols), (Some((2, 3)), None));
    // Not ranges: nothing changes.
    assert!(!q.set_print_area_formula("Sheet1!#REF!"));
    assert!(!q.set_print_titles_formula("Sheet1!$A$1:$B$2"));
    assert_eq!(q.print_area, p.print_area);
    assert_eq!(q.title_rows, Some((2, 3)));
}

#[test]
fn letter_portrait_pages() {
    // 7.1 × 9.5 inches inside the margins: 681.6 × 912 pixels, that is 10
    // default columns and 45 default rows.
    let wb = grid(100, 12);
    let p = pages(&wb);
    assert_eq!(p.paper, (612.0, 792.0));
    assert_eq!(p.scale_percent, 100);
    assert_eq!(p.ranges.len(), 1);
    assert_eq!(p.ranges[0].rect, Rect::new(0, 0, 99, 11));
    assert_eq!(p.ranges[0].col_bands, vec![(0, 9), (10, 11)]);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 44), (45, 89), (90, 99)]);
    assert_eq!(
        p.ranges[0].row_breaks,
        vec![PageBreak { at: 45, manual: false }, PageBreak { at: 90, manual: false }]
    );
    // Down, then over.
    assert_eq!(
        cells(&p),
        vec![
            (0, 44, 0, 9),
            (45, 89, 0, 9),
            (90, 99, 0, 9),
            (0, 44, 10, 11),
            (45, 89, 10, 11),
            (90, 99, 10, 11)
        ]
    );
    assert_eq!(
        p.pages.iter().map(|p| p.number).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_eq!(p.last_number(), Some(6));
    let first = &p.pages[0];
    assert!((first.origin.0 - 50.4).abs() < 1e-9 && (first.origin.1 - 54.0).abs() < 1e-9);
    assert!((first.size.0 - 480.0).abs() < 1e-9 && (first.size.1 - 675.0).abs() < 1e-9);
    assert_eq!(p.page_of(50, 11).map(|p| p.index), Some(4));
    assert_eq!(p.page_of(100, 0), None);
}

#[test]
fn page_order_and_first_number() {
    let mut wb = grid(100, 12);
    wb.update_page_setup(0, |p| {
        p.page_order = PageOrder::OverThenDown;
        p.first_page_number = Some(5);
    });
    let p = pages(&wb);
    assert_eq!(
        cells(&p),
        vec![
            (0, 44, 0, 9),
            (0, 44, 10, 11),
            (45, 89, 0, 9),
            (45, 89, 10, 11),
            (90, 99, 0, 9),
            (90, 99, 10, 11)
        ]
    );
    assert_eq!(p.pages[0].number, 5);
    assert_eq!(p.last_number(), Some(10));
    // The sheet's own number wins over the place in a larger job.
    let opts = PaginateOptions {
        first_page_number: Some(40),
        ..PaginateOptions::default()
    };
    assert_eq!(paginate(&wb, 0, &opts).pages[0].number, 5);
    wb.update_page_setup(0, |p| p.first_page_number = None);
    assert_eq!(paginate(&wb, 0, &opts).pages[0].number, 40);
}

#[test]
fn a4_and_landscape() {
    let mut wb = Workbook::new(Locale::fr());
    fill(&mut wb, 100, 20);
    // A4 inside the normal margins: 659 × 978 pixels.
    let p = pages(&wb);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 9), (10, 19)]);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 47), (48, 95), (96, 99)]);
    wb.update_page_setup(0, |p| p.orientation = Orientation::Landscape);
    // Landscape: 989 × 649 pixels.
    let p = pages(&wb);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 14), (15, 19)]);
    assert_eq!(p.ranges[0].row_bands[0], (0, 31));
    // Narrow margins make room for one more column.
    wb.update_page_setup(0, |p| p.margins = Margins::NARROW);
    assert_eq!(pages(&wb).ranges[0].col_bands[0], (0, 15));
}

#[test]
fn scale_changes_what_fits() {
    let mut wb = grid(100, 12);
    wb.update_page_setup(0, |p| p.scale = 50);
    let p = pages(&wb);
    assert_eq!(p.pages.len(), 2);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 90), (91, 99)]);
    assert!((p.unit() - 0.375).abs() < 1e-12);
    wb.update_page_setup(0, |p| p.scale = 200);
    let p = pages(&wb);
    assert_eq!(p.ranges[0].col_bands.len(), 3);
    assert_eq!(p.ranges[0].row_bands[0], (0, 21));
}

#[test]
fn manual_breaks_and_hidden_rows() {
    let mut wb = grid(100, 5);
    assert!(wb.insert_page_break(0, Some(10), Some(2)));
    let p = pages(&wb);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 1), (2, 4)]);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 9), (10, 54), (55, 99)]);
    assert!(p.ranges[0].row_breaks[0].manual && !p.ranges[0].row_breaks[1].manual);
    assert!(p.ranges[0].col_breaks[0].manual);
    // A break before the first row or column is not a break.
    assert!(!wb.insert_page_break(0, Some(0), Some(0)));

    // Hidden and filtered rows take no room; a break on a hidden row
    // applies at the next row that prints.
    wb.set_rows_hidden(0, 8, 12, true);
    wb.sheets[0].filtered_rows.extend(20..40);
    let p = pages(&wb);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 7), (13, 77), (78, 99)]);
    assert_eq!(p.ranges[0].row_breaks[0], PageBreak { at: 13, manual: true });
    // Hidden columns likewise.
    wb.set_cols_hidden(0, 0, 0, true);
    assert_eq!(pages(&wb).ranges[0].col_bands, vec![(1, 1), (2, 4)]);

    assert!(wb.remove_page_break(0, Some(10), None));
    assert!(!wb.remove_page_break(0, Some(10), None));
    assert_eq!(pages(&wb).ranges[0].col_bands.len(), 2);
    assert!(wb.reset_page_breaks(0));
    assert_eq!(pages(&wb).ranges[0].col_bands.len(), 1);
    assert_eq!(breaks_for_selection(Rect::cell(4, 2)), (Some(4), Some(2)));
    assert_eq!(breaks_for_selection(Rect::new(4, 0, 4, MAX_COLS - 1)), (Some(4), None));
    assert_eq!(breaks_for_selection(Rect::new(0, 3, MAX_ROWS - 1, 3)), (None, Some(3)));
    assert_eq!(breaks_for_selection(Rect::cell(0, 0)), (None, None));
}

#[test]
fn fit_to_pages() {
    // 768 × 2000 pixels of cells.
    let mut wb = grid(100, 12);
    wb.insert_page_break(0, Some(10), None);
    wb.update_page_setup(0, |p| p.fit_to_page = true);
    let p = pages(&wb);
    // 912 / 2000 = 45.6 %; manual breaks are ignored.
    assert_eq!(p.scale_percent, 45);
    assert_eq!(cells(&p), vec![(0, 99, 0, 11)]);
    // One page wide, as tall as needed: 681.6 / 768 = 88.75 %.
    wb.update_page_setup(0, |p| p.fit_height = 0);
    let p = pages(&wb);
    assert_eq!(p.scale_percent, 88);
    assert_eq!(p.ranges[0].col_bands.len(), 1);
    assert_eq!(p.ranges[0].row_bands.len(), 2);
    // Two pages tall.
    wb.update_page_setup(0, |p| {
        p.fit_width = 0;
        p.fit_height = 2;
    });
    let p = pages(&wb);
    assert_eq!(p.scale_percent, 91);
    assert_eq!(p.ranges[0].row_bands.len(), 2);
    // Fit-to never enlarges.
    let mut small = grid(3, 3);
    small.update_page_setup(0, |p| p.fit_to_page = true);
    assert_eq!(pages(&small).scale_percent, 100);
    // And never goes below 10 %.
    let mut huge = grid(1, 1);
    huge.set_value(0, 5000, 0, Value::Number(1.0));
    huge.commit("far");
    huge.update_page_setup(0, |p| p.fit_to_page = true);
    let p = pages(&huge);
    assert_eq!(p.scale_percent, 10);
    assert!(p.pages.len() > 1);
}

#[test]
fn print_titles_repeat() {
    let mut wb = grid(100, 12);
    assert!(wb.set_print_titles(0, Some((1, 0)), Some((0, 0))));
    assert_eq!(wb.sheets[0].page.title_rows, Some((0, 1)));
    let p = pages(&wb);
    // Later pages give 40 pixels to the two title rows: 43 rows fit; and
    // 64 pixels to the title column: 9 columns fit.
    assert_eq!(p.ranges[0].row_bands, vec![(0, 44), (45, 87), (88, 99)]);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 9), (10, 11)]);
    assert_eq!(p.pages[0].title_rows, None);
    assert_eq!(p.pages[0].title_cols, None);
    assert_eq!(p.pages[1].title_rows, Some((0, 1)));
    assert_eq!(p.pages[1].title_cols, None);
    assert_eq!(p.pages[4].title_rows, Some((0, 1)));
    assert_eq!(p.pages[4].title_cols, Some((0, 0)));
    // The page's size counts the repeated titles.
    assert!((p.pages[4].size.0 - 3.0 * 48.0).abs() < 1e-9);
    assert!((p.pages[4].size.1 - 45.0 * 15.0).abs() < 1e-9);

    let layout = p.layout(&wb.sheets[0], &p.pages[4]);
    assert_eq!((layout.title_rows, layout.title_cols), (2, 1));
    assert_eq!(layout.rows.len(), 45);
    assert_eq!(layout.rows[0].index, 0);
    assert_eq!(layout.rows[2].index, 45);
    assert_eq!(layout.cols.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0, 10, 11]);
    assert!((layout.body.0 - (50.4 + 48.0)).abs() < 1e-9);
    assert!((layout.body.1 - (54.0 + 30.0)).abs() < 1e-9);
    assert!((layout.col(11).unwrap().pos - (50.4 + 96.0)).abs() < 1e-9);
    assert_eq!(layout.body_px, (640.0, 900.0));
    let (x, y) = layout.to_paper(650.0, 920.0);
    assert!((x - (layout.body.0 + 7.5)).abs() < 1e-9 && (y - (layout.body.1 + 15.0)).abs() < 1e-9);

    // A print area below the titles repeats them on its first page too.
    wb.set_print_area(0, &[Rect::new(10, 2, 30, 4)]);
    let p = pages(&wb);
    assert_eq!(p.pages.len(), 1);
    assert_eq!(
        (p.pages[0].title_rows, p.pages[0].title_cols),
        (Some((0, 1)), Some((0, 0)))
    );
}

#[test]
fn headings_take_room() {
    let mut wb = grid(100, 12);
    wb.update_page_setup(0, |p| p.headings = true);
    let p = pages(&wb);
    // Three digits of row numbers: 34.5 pixels; one row of letters.
    assert_eq!(p.heading_size, (34.5, 20.0));
    assert_eq!(p.ranges[0].row_bands[0], (0, 43));
    assert_eq!(p.ranges[0].col_bands[0], (0, 9));
    let layout = p.layout(&wb.sheets[0], &p.pages[0]);
    assert!((layout.heading.0 - 34.5 * 0.75).abs() < 1e-9);
    assert!((layout.cols[0].pos - (50.4 + 34.5 * 0.75)).abs() < 1e-9);
    assert!((layout.rows[0].pos - (54.0 + 15.0)).abs() < 1e-9);
}

#[test]
fn centring_offsets() {
    let mut wb = grid(10, 5);
    wb.update_page_setup(0, |p| {
        p.center_horizontally = true;
        p.center_vertically = true;
    });
    let p = pages(&wb);
    let page = &p.pages[0];
    assert_eq!(page.size, (240.0, 150.0));
    assert!((page.origin.0 - (612.0 - 240.0) / 2.0).abs() < 1e-9);
    assert!((page.origin.1 - (792.0 - 150.0) / 2.0).abs() < 1e-9);
}

#[test]
fn print_area_selection_and_blank_pages() {
    // A long column and one cell far to the right: the page right of the
    // lower part of the column is blank and last, so it does not print.
    let mut wb = grid(60, 1);
    wb.set_value(0, 0, 11, Value::Number(1.0));
    wb.commit("far cell");
    let p = pages(&wb);
    assert_eq!(cells(&p), vec![(0, 44, 0, 9), (45, 59, 0, 9), (0, 44, 10, 11)]);
    // Over then down, the blank page is in the middle and prints.
    wb.update_page_setup(0, |p| p.page_order = PageOrder::OverThenDown);
    assert_eq!(pages(&wb).pages.len(), 3);
    assert_eq!(cells(&pages(&wb))[2], (45, 59, 0, 9));
    wb.update_page_setup(0, |p| p.page_order = PageOrder::DownThenOver);

    // A print area prints as drawn, blank pages included.
    assert!(wb.set_print_area(0, &[Rect::new(0, 0, 59, 11)]));
    assert_eq!(pages(&wb).pages.len(), 4);
    // Several ranges: each on pages of its own, in order.
    assert!(wb.add_to_print_area(0, Rect::new(0, 11, 0, 11)));
    assert!(!wb.add_to_print_area(0, Rect::new(0, 11, 0, 11)));
    let p = pages(&wb);
    assert_eq!(p.ranges.len(), 2);
    assert_eq!(p.pages.len(), 5);
    assert_eq!(p.pages[4].range, 1);
    assert_eq!(cells(&p)[4], (0, 0, 11, 11));
    // Whole columns print as far as the sheet is used.
    wb.set_print_area(0, &[Rect::parse("A:B").unwrap()]);
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 59, 1));
    // Nothing but blank cells: nothing to print.
    wb.set_print_area(0, &[Rect::new(70, 0, 200, 3)]);
    assert!(pages(&wb).pages.is_empty());

    // A selection replaces the print area; so does ignoring it.
    let selection = [Rect::new(2, 0, 5, 0)];
    let p = paginate(
        &wb,
        0,
        &PaginateOptions {
            selection: Some(&selection),
            ..PaginateOptions::default()
        },
    );
    assert_eq!(cells(&p), vec![(2, 5, 0, 0)]);
    let p = paginate(
        &wb,
        0,
        &PaginateOptions {
            ignore_print_area: true,
            ..PaginateOptions::default()
        },
    );
    assert_eq!(p.pages.len(), 3);
    assert!(wb.clear_print_area(0));
    assert_eq!(pages(&wb).pages.len(), 3);
}

#[test]
fn nothing_to_print() {
    let wb = Workbook::new(Locale::en());
    let p = pages(&wb);
    assert!(p.pages.is_empty() && p.ranges.is_empty());
    assert!(paginate(&wb, 7, &PaginateOptions::default()).pages.is_empty());
}

#[test]
fn formatting_merges_and_objects_extend_the_range() {
    let mut wb = grid(2, 2);
    // A number format alone leaves no mark; a fill or a border does.
    wb.update_style(0, Rect::cell(60, 0), &|s| s.num_fmt = "0.00".into());
    wb.commit("format");
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 1, 1));
    wb.update_style(0, Rect::cell(50, 3), &|s| s.fill = Some(crate::style::Color(0xFFFF00)));
    wb.commit("fill");
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 50, 3));
    // A merged area counts whole.
    wb.set_value(0, 0, 4, Value::text("title"));
    wb.merge(0, Rect::new(0, 4, 0, 10));
    wb.commit("merge");
    let p = pages(&wb);
    assert_eq!(p.ranges[0].rect, Rect::new(0, 0, 50, 10));
    assert_eq!(p.pages.len(), 3);

    // A chart anchored at D61, 480 × 288 pixels: 7.5 columns, 14.4 rows.
    let chart = Chart {
        id: 0,
        kind: ChartKind::Column,
        title: ChartTitle::None,
        legend: LegendPos::None,
        series: Vec::new(),
        data_labels: false,
        anchor: (60, 3),
        offset: (10.0, 5.0),
        size: (480.0, 288.0),
        ..Chart::new(ChartKind::Column)
    };
    wb.add_chart(0, chart);
    wb.commit("chart");
    let objects = floating_objects(&wb.sheets[0]);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].kind, ObjectKind::Chart);
    assert_eq!(objects[0].cells(&wb.sheets[0]), Rect::new(60, 3, 74, 10));
    assert_eq!(objects[0].sheet_box(&wb.sheets[0]), (202.0, 1205.0, 480.0, 288.0));
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 74, 10));
}

#[test]
fn overflowing_text_prints() {
    let mut wb = Workbook::new(Locale::en());
    // 150 characters of 7 pixels: 1056 pixels with the padding, 16.5
    // default columns.
    wb.set_value(0, 0, 0, Value::text("x".repeat(150)));
    wb.set_value(0, 1, 0, Value::text("short"));
    wb.commit("text");
    assert_eq!(pages(&wb).ranges[0].rect, Rect::cell(0, 0).union(&Rect::cell(1, 0)));
    let opts = PaginateOptions {
        measure: Some(&Fixed),
        ..PaginateOptions::default()
    };
    let p = paginate(&wb, 0, &opts);
    assert_eq!(p.ranges[0].rect, Rect::new(0, 0, 1, 16));
    assert_eq!(p.pages.len(), 2);
    // Wrapped text stays in its cell; so does a number.
    wb.update_style(0, Rect::cell(0, 0), &|s| s.wrap = true);
    wb.set_value(0, 2, 0, Value::Number(123456789012345.0));
    wb.commit("wrap");
    assert_eq!(paginate(&wb, 0, &opts).ranges[0].rect, Rect::new(0, 0, 2, 0));
    // Centred text flows half to each side.
    wb.update_style(0, Rect::cell(0, 0), &|s| {
        s.wrap = false;
        s.h_align = HAlign::Center;
    });
    wb.commit("centre");
    assert_eq!(paginate(&wb, 0, &opts).ranges[0].rect, Rect::new(0, 0, 2, 8));
    // The next cell with something in it stops the text.
    wb.set_value(0, 0, 3, Value::Number(1.0));
    wb.commit("stop");
    assert_eq!(paginate(&wb, 0, &opts).ranges[0].rect, Rect::new(0, 0, 2, 3));
}

#[test]
fn structural_edits_move_the_setup() {
    let mut wb = grid(100, 12);
    wb.set_print_area(0, &[Rect::new(5, 1, 60, 8)]);
    wb.set_print_titles(0, Some((2, 3)), Some((1, 1)));
    wb.insert_page_break(0, Some(20), Some(4));
    wb.insert_page_break(0, Some(40), None);
    wb.commit("setup");

    wb.insert_rows(0, 10, 3);
    wb.commit("insert rows");
    let p = &wb.sheets[0].page;
    assert_eq!(p.print_area, vec![Rect::new(5, 1, 63, 8)]);
    assert_eq!(p.title_rows, Some((2, 3)));
    assert_eq!(p.row_breaks.iter().copied().collect::<Vec<_>>(), vec![23, 43]);

    wb.insert_rows(0, 0, 1);
    wb.commit("insert at top");
    let p = &wb.sheets[0].page;
    assert_eq!(p.print_area, vec![Rect::new(6, 1, 64, 8)]);
    assert_eq!(p.title_rows, Some((3, 4)));
    assert_eq!(p.row_breaks.iter().copied().collect::<Vec<_>>(), vec![24, 44]);

    // Deleting the row under a break deletes the break; the print area
    // and the titles shrink.
    wb.delete_rows(0, 4, 21);
    wb.commit("delete rows");
    let p = &wb.sheets[0].page;
    assert_eq!(p.print_area, vec![Rect::new(4, 1, 43, 8)]);
    assert_eq!(p.title_rows, Some((3, 3)));
    assert_eq!(p.row_breaks.iter().copied().collect::<Vec<_>>(), vec![23]);

    wb.insert_cols(0, 1, 2);
    wb.commit("insert cols");
    let p = &wb.sheets[0].page;
    assert_eq!(p.print_area, vec![Rect::new(4, 3, 43, 10)]);
    assert_eq!(p.title_cols, Some((3, 3)));
    assert_eq!(p.col_breaks.iter().copied().collect::<Vec<_>>(), vec![6]);

    wb.delete_cols(0, 3, 1);
    wb.commit("delete cols");
    let p = &wb.sheets[0].page;
    assert_eq!(p.print_area, vec![Rect::new(4, 3, 43, 9)]);
    assert_eq!(p.title_cols, None);
    assert_eq!(p.col_breaks.iter().copied().collect::<Vec<_>>(), vec![5]);

    // Deleting all of the print area clears it.
    wb.delete_cols(0, 0, 12);
    wb.commit("delete all");
    assert!(wb.sheets[0].page.print_area.is_empty());
    assert!(wb.sheets[0].page.col_breaks.is_empty());

    // Edits of another sheet leave this one alone.
    let before = wb.sheets[0].page.clone();
    let other = wb.add_sheet(1);
    wb.insert_rows(other, 0, 5);
    wb.commit("other sheet");
    assert_eq!(wb.sheets[0].page, before);
}

#[test]
fn page_setup_edits_undo() {
    let mut wb = grid(10, 3);
    assert!(wb.set_print_area(0, &[Rect::new(0, 0, 4, 1)]));
    assert!(wb.commit("Print Area"));
    // Setting the same thing again records nothing.
    assert!(!wb.set_print_area(0, &[Rect::new(0, 0, 4, 1)]));
    assert!(!wb.commit("Print Area"));
    let mut setup = wb.sheets[0].page.clone();
    setup.orientation = Orientation::Landscape;
    setup.header_footer.odd_footer = "&CPage &P".into();
    assert!(wb.set_page_setup(0, setup.clone()));
    wb.commit("Page Setup");
    assert_eq!(wb.sheets[0].page, setup);
    wb.undo();
    assert_eq!(wb.sheets[0].page.orientation, Orientation::Portrait);
    assert_eq!(wb.sheets[0].page.print_area.len(), 1);
    wb.undo();
    assert!(wb.sheets[0].page.print_area.is_empty());
    wb.redo();
    wb.redo();
    assert_eq!(wb.sheets[0].page, setup);
    assert!(!wb.update_page_setup(9, |p| p.draft = true));
}

#[test]
fn duplicated_sheet_keeps_its_setup() {
    let mut wb = grid(10, 3);
    wb.set_print_area(0, &[Rect::new(0, 0, 4, 1)]);
    wb.set_print_titles(0, Some((0, 0)), None);
    wb.insert_page_break(0, Some(3), None);
    wb.update_page_setup(0, |p| p.header_footer.odd_header = "&C&A".into());
    wb.commit("setup");
    let copy = wb.duplicate_sheet(0).unwrap();
    wb.commit("copy");
    assert_eq!(wb.sheets[copy].page, wb.sheets[0].page);
    let name = wb.sheets[copy].name.clone();
    assert_eq!(
        wb.sheets[copy].page.print_area_formula(&name).unwrap(),
        "'Sheet1 (2)'!$A$1:$B$5"
    );
    // Renaming leaves the ranges alone.
    assert!(wb.rename_sheet(copy, "Copy"));
    assert_eq!(
        wb.sheets[copy].page.print_area_formula("Copy").unwrap(),
        "Copy!$A$1:$B$5"
    );
}

#[test]
fn dragging_a_page_break() {
    let mut wb = grid(100, 5);
    // The automatic break after row 45 dragged up to row 31: it becomes
    // manual and nothing else changes.
    assert!(wb.move_page_break(0, BreakAxis::Rows, 45, 30, None));
    let p = pages(&wb);
    assert_eq!(
        wb.sheets[0].page.row_breaks.iter().copied().collect::<Vec<_>>(),
        vec![30]
    );
    assert_eq!(p.scale_percent, 100);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 29), (30, 74), (75, 99)]);
    // Dragged down to row 61: 60 rows must fit where 45 did, so the scale
    // drops to 912 / 1200 = 76 %.
    assert!(wb.move_page_break(0, BreakAxis::Rows, 30, 60, None));
    let p = pages(&wb);
    assert_eq!(p.scale_percent, 76);
    assert_eq!(p.ranges[0].row_bands, vec![(0, 59), (60, 99)]);
    assert!(p.ranges[0].row_breaks[0].manual);
    // Dragged out of the printed range: the break is removed.
    assert!(wb.move_page_break(0, BreakAxis::Rows, 60, 100, None));
    assert!(wb.sheets[0].page.row_breaks.is_empty());
    // There is no break at row 13 to drag.
    assert!(!wb.move_page_break(0, BreakAxis::Rows, 12, 20, None));

    // A sheet fitted to one page keeps its scale as "Adjust to".
    let mut wb = grid(100, 12);
    wb.update_page_setup(0, |p| {
        p.fit_to_page = true;
        p.fit_height = 0;
    });
    assert_eq!(pages(&wb).scale_percent, 88);
    let at = pages(&wb).ranges[0].row_breaks[0].at;
    assert!(wb.move_page_break(0, BreakAxis::Rows, at, at - 5, None));
    let setup = &wb.sheets[0].page;
    assert!(!setup.fit_to_page);
    assert_eq!(setup.scale, 88);
    assert!(setup.row_breaks.contains(&(at - 5)));
    // Columns work the same way.
    let mut wb = grid(10, 12);
    assert!(wb.move_page_break(0, BreakAxis::Cols, 10, 11, None));
    let p = pages(&wb);
    assert_eq!(p.scale_percent, 96);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 10), (11, 11)]);
    // An automatic break dragged to the end of the range: everything
    // left goes on that page, at 681.6 / 768 = 88 %.
    let mut wb = grid(10, 12);
    assert!(wb.move_page_break(0, BreakAxis::Cols, 10, 12, None));
    assert!(wb.sheets[0].page.col_breaks.is_empty());
    let p = pages(&wb);
    assert_eq!(p.scale_percent, 88);
    assert_eq!(p.ranges[0].col_bands, vec![(0, 11)]);
    // Dragged before the start of the range, it stays where it was.
    assert!(!wb.move_page_break(0, BreakAxis::Rows, 5, 0, None));
}

#[test]
fn pictures_and_shapes_take_room() {
    use crate::drawing::{Frame, ShapeTool};
    let mut wb = grid(2, 2);
    // A rectangle 100 pixels in from column E and 30 down from row 71.
    let frame = Frame::new(4.0 * 64.0 + 100.0, 70.0 * 20.0 + 30.0, 200.0, 90.0);
    let id = wb.add_shape(0, ShapeTool::ALL[0], frame).unwrap();
    wb.commit("shape");
    let objects = floating_objects(&wb.sheets[0]);
    assert_eq!(objects.len(), 1);
    assert_eq!((objects[0].kind, objects[0].id), (ObjectKind::Drawing, id));
    assert_eq!(objects[0].sheet_box(&wb.sheets[0]), (frame.x, frame.y, 200.0, 90.0));
    assert_eq!(objects[0].cells(&wb.sheets[0]), Rect::new(71, 5, 75, 8));
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 75, 8));
    // "Print object" off: the shape is not on paper.
    wb.update_drawing(0, id, |d| d.print = false);
    wb.commit("no print");
    assert!(floating_objects(&wb.sheets[0]).is_empty());
    assert_eq!(pages(&wb).ranges[0].rect, Rect::new(0, 0, 1, 1));
}

#[test]
fn whole_sheet_pages() {
    // Letter portrait holds 10 default columns and 45 default rows.
    let mut wb = grid(100, 12);
    let all = sheet_pages(&wb, 0, &pages(&wb));
    assert_eq!(all.paper, (612.0, 792.0));
    assert_eq!(&all.col_starts[..4], &[0, 10, 20, 30]);
    assert_eq!(&all.row_starts[..4], &[0, 45, 90, 135]);
    // Every row and column is on a page, the last ones included.
    assert_eq!(all.col_starts.len(), 1639);
    assert_eq!(all.cols(1638), (16380, 16383));
    assert_eq!(all.row_page(MAX_ROWS - 1), all.row_starts.len() - 1);
    assert_eq!(all.rows(1), (45, 89));
    assert_eq!((all.col_page(9), all.col_page(10)), (0, 1));

    // A manual break, a tall row and a hidden row move the pages that
    // follow; past them the pages repeat.
    wb.insert_page_break(0, Some(20), Some(3));
    wb.set_row_height(0, 30, Some(120.0));
    wb.set_rows_hidden(0, 50, 52, true);
    wb.commit("setup");
    let p = pages(&wb);
    let all = sheet_pages(&wb, 0, &p);
    assert_eq!(&all.col_starts[..3], &[0, 3, 13]);
    // 20 to 59: 36 rows of 20, one of 120 and three hidden ones make 840
    // of the 912 pixels; three more rows fit.
    assert_eq!(&all.row_starts[..4], &[0, 20, 63, 108]);
    // The printed pages are the first of them.
    assert_eq!(p.ranges[0].row_breaks.iter().map(|b| b.at).collect::<Vec<_>>(), vec![20, 63]);

    // Print titles leave less room on the pages after them.
    wb.reset_page_breaks(0);
    wb.set_row_height(0, 30, None);
    wb.set_rows_hidden(0, 50, 52, false);
    wb.set_print_titles(0, Some((0, 4)), None);
    wb.commit("titles");
    let all = sheet_pages(&wb, 0, &pages(&wb));
    assert_eq!(&all.row_starts[..4], &[0, 45, 85, 125]);

    // Half the scale, twice the cells.
    wb.set_print_titles(0, None, None);
    wb.update_page_setup(0, |s| s.scale = 50);
    wb.commit("scale");
    let all = sheet_pages(&wb, 0, &pages(&wb));
    assert_eq!(&all.col_starts[..3], &[0, 21, 42]);
    assert_eq!(&all.row_starts[..3], &[0, 91, 182]);
}
