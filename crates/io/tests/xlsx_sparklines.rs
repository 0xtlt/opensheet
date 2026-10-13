//! Sparklines in `.xlsx` files: the `x14:sparklineGroups` extension of a
//! worksheet, beside the other extensions of the sheet.

mod common;

use common::{assert_well_formed, assert_worksheet_order, enter, keep_sample, package, part};
use opensheet_engine::condfmt::{BarColor, CondFormat, DataBar, Rule};
use opensheet_engine::sparkline::{AxisBound, EmptyCells, SparklineColors, SparklineType, style};
use opensheet_engine::{Color, Locale, Rect, Workbook};
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

/// Numbers in A1:D3 of the first sheet and of a second one whose name
/// needs quotes.
fn book() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    assert!(wb.rename_sheet(1, "Year's data"));
    for sheet in 0..2 {
        for (r, row) in [["1", "3", "2", "5"], ["4", "-2", "", "1"], ["2", "-2", "2", "2"]].iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if !text.is_empty() {
                    enter(&mut wb, sheet, &Rect::cell(r as u32, c as u32).name(), text);
                }
            }
        }
    }
    wb.commit("setup");
    wb
}

#[test]
fn groups_round_trip_with_every_option() {
    let mut wb = book();
    wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
    wb.add_sparklines(0, SparklineType::Column, (1, rect("A1:D3")), rect("A5:D5")).unwrap();
    wb.add_sparklines(0, SparklineType::WinLoss, (0, rect("A2:D2")), rect("G2")).unwrap();
    wb.update_sparkline_groups(0, &[rect("F1")], |g| {
        g.line_weight = 2.25;
        g.markers = true;
        g.high = true;
        g.low = true;
        g.first = true;
        g.last = true;
        g.negative = true;
        g.show_axis = true;
        g.show_hidden = true;
        g.right_to_left = true;
        g.empty = EmptyCells::Connect;
        g.min = AxisBound::Custom(-2.5);
        g.max = AxisBound::Group;
        g.colors = style(31);
    });
    wb.update_sparkline_groups(0, &[rect("A5")], |g| {
        g.empty = EmptyCells::Zero;
        g.min = AxisBound::Group;
        g.max = AxisBound::Custom(10.0);
    });
    wb.commit("sparklines");
    let bytes = to_bytes(&wb);
    keep_sample("sparklines.xlsx", &bytes);
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &xml);
    assert_worksheet_order("sheet1.xml", &xml);
    // As Excel writes them: namespaces, uri, attributes, eight colours.
    assert!(
        xml.contains(concat!(
            "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" ",
            "uri=\"{05C60535-1F16-4fd2-B633-F4F36F0B64E0}\">",
            "<x14:sparklineGroups xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">",
            "<x14:sparklineGroup manualMin=\"-2.5\" lineWeight=\"2.25\" displayEmptyCellsAs=\"span\" markers=\"1\" high=\"1\" ",
            "low=\"1\" first=\"1\" last=\"1\" negative=\"1\" displayXAxis=\"1\" displayHidden=\"1\" minAxisType=\"custom\" ",
            "maxAxisType=\"group\" rightToLeft=\"1\"><x14:colorSeries rgb=\"FF5687C2\"/><x14:colorNegative rgb=\"FFFFB620\"/>",
            "<x14:colorAxis rgb=\"FF000000\"/><x14:colorMarkers rgb=\"FFD70077\"/><x14:colorFirst rgb=\"FF777777\"/>",
            "<x14:colorLast rgb=\"FF359CEB\"/><x14:colorHigh rgb=\"FF56BE79\"/><x14:colorLow rgb=\"FFFF5055\"/>",
            "<x14:sparklines><x14:sparkline><xm:f>Sheet1!A1:D1</xm:f><xm:sqref>F1</xm:sqref></x14:sparkline>",
        )),
        "{xml}"
    );
    assert!(xml.contains("<x14:sparklineGroup manualMax=\"10\" type=\"column\" displayEmptyCellsAs=\"zero\" minAxisType=\"group\" maxAxisType=\"custom\">"), "{xml}");
    assert!(xml.contains("<xm:f>'Year''s data'!A1:A3</xm:f><xm:sqref>A5</xm:sqref>"), "{xml}");
    assert!(xml.contains("<x14:sparklineGroup type=\"stacked\" displayEmptyCellsAs=\"gap\" negative=\"1\"><x14:colorSeries rgb=\"FF376092\"/>"), "{xml}");
    assert!(xml.ends_with("</x14:sparklineGroups></ext></extLst></worksheet>"), "{xml}");

    let back = load(&bytes);
    assert_eq!(back.sheets[0].sparklines, wb.sheets[0].sparklines);
    assert!(back.sheets[1].sparklines.is_empty());
    // Written again, the same file.
    assert_eq!(part(&to_bytes(&back), "xl/worksheets/sheet1.xml"), xml);
}

#[test]
fn one_extension_list_for_all_the_features_of_a_sheet() {
    let mut wb = book();
    wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D1")), rect("F1")).unwrap();
    // A data bar of the 2010 kind lives in the extension list too.
    wb.snapshot();
    wb.sheets[0].cond_formats.push(CondFormat::new(vec![rect("A1:A3")], Rule::DataBar(DataBar::preset(BarColor::Blue, true))));
    wb.commit("setup");
    let bytes = to_bytes(&wb);
    keep_sample("sparklines-and-data-bars.xlsx", &bytes);
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &xml);
    assert_worksheet_order("sheet1.xml", &xml);
    // The list of the sheet is the last element; the one inside the
    // 2007 rule is its own.
    let tail = &xml[xml.rfind("<extLst>").unwrap()..];
    let bars = tail.find("{78C0D931-6437-407d-A8EE-F0AAD7539E65}").expect("the conditional formats");
    let sparks = tail.find("{05C60535-1F16-4fd2-B633-F4F36F0B64E0}").expect("the sparklines");
    assert!(bars < sparks, "{tail}");
    assert_eq!(tail.matches("<ext ").count(), 2);
    let back = load(&bytes);
    assert_eq!(back.sheets[0].sparklines, wb.sheets[0].sparklines);
    assert_eq!(back.sheets[0].cond_formats.len(), 1);
}

#[test]
fn sparklines_written_by_excel_are_read() {
    let sheet = concat!(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
        "<sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c></row></sheetData>",
        "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" uri=\"{05C60535-1F16-4fd2-B633-F4F36F0B64E0}\">",
        "<x14:sparklineGroups xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">",
        "<x14:sparklineGroup manualMax=\"8\" type=\"column\" dateAxis=\"1\" displayEmptyCellsAs=\"gap\" high=\"1\" maxAxisType=\"custom\" xr2:uid=\"{0}\">",
        "<x14:colorSeries theme=\"4\" tint=\"-0.499984740745262\"/><x14:colorNegative theme=\"5\"/><x14:colorAxis rgb=\"FF000000\"/>",
        "<x14:colorMarkers theme=\"4\" tint=\"-0.499984740745262\"/><x14:colorFirst theme=\"4\" tint=\"0.39997558519241921\"/>",
        "<x14:colorLast theme=\"4\" tint=\"0.39997558519241921\"/><x14:colorHigh rgb=\"FF00B050\"/><x14:colorLow theme=\"4\"/>",
        "<xm:f>Notes!$A$3:$B$3</xm:f>",
        "<x14:sparklines><x14:sparkline><xm:f>Notes!$A$1:$B$1</xm:f><xm:sqref>C1</xm:sqref></x14:sparkline>",
        "<x14:sparkline><xm:sqref>C2</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup>",
        // A group with nothing left to plot, and Excel's defaults.
        "<x14:sparklineGroup><x14:sparklines/></x14:sparklineGroup>",
        "<x14:sparklineGroup><x14:colorSeries rgb=\"FF376092\"/><x14:sparklines>",
        "<x14:sparkline><xm:f>'My notes'!A1:A4</xm:f><xm:sqref>D1</xm:sqref></x14:sparkline></x14:sparklines></x14:sparklineGroup>",
        "</x14:sparklineGroups></ext></extLst></worksheet>"
    );
    let bytes = package(&[
        (
            "xl/workbook.xml",
            concat!(
                "<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
                "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
                "<sheets><sheet name=\"Notes\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
            ),
        ),
        ("xl/worksheets/sheet1.xml", sheet),
    ]);
    let wb = load(&bytes);
    let groups = &wb.sheets[0].sparklines;
    assert_eq!(groups.len(), 2);
    let g = &groups[0];
    assert_eq!((g.kind, g.empty, g.high, g.low), (SparklineType::Column, EmptyCells::Gaps, true, false));
    assert_eq!((g.min, g.max), (AxisBound::Individual, AxisBound::Custom(8.0)));
    // Theme colours come through the palette of the file; the one given
    // outright is kept as it is.
    assert_eq!(g.colors.high, Color(0x00B050));
    assert_ne!(g.colors.series, SparklineColors::default().series);
    assert_eq!(g.sparklines.len(), 1);
    assert_eq!((g.sparklines[0].row, g.sparklines[0].col, g.sparklines[0].data.range), (0, 2, rect("A1:B1")));
    // The schema's defaults: a line, empty cells as zero.
    let plain = &groups[1];
    assert_eq!((plain.kind, plain.empty, plain.line_weight), (SparklineType::Line, EmptyCells::Zero, 0.75));
    assert_eq!((plain.sparklines[0].data.sheet.as_str(), plain.sparklines[0].data.range), ("My notes", rect("A1:A4")));
    assert_eq!(plain.colors, SparklineColors::default());
}

/// A date axis: `dateAxis="1"` and the cells of the dates in an `xm:f`
/// of the group, as Excel writes them.
#[test]
fn a_date_axis_goes_through_a_file() {
    let mut wb = Workbook::new(Locale::en());
    let days = ["2024-01-01", "2024-01-02", "2024-01-05", "2024-02-01", "2024-02-02"];
    for (c, day) in days.iter().enumerate() {
        wb.set_cell_text(0, 0, c as u32, day).unwrap();
        wb.set_cell_text(0, 1, c as u32, &format!("{}", (c * 7) % 5 + 1)).unwrap();
        wb.set_cell_text(0, 2, c as u32, &format!("{}", (c * 3) % 4 + 2)).unwrap();
    }
    // Dated lines and columns, and the same data evenly spaced beside them.
    wb.add_sparklines(0, SparklineType::Line, (0, rect("A2:E3")), rect("G2:G3")).unwrap();
    wb.add_sparklines(0, SparklineType::Column, (0, rect("A2:E3")), rect("H2:H3")).unwrap();
    wb.add_sparklines(0, SparklineType::Line, (0, rect("A2:E3")), rect("I2:I3")).unwrap();
    assert_eq!(wb.set_sparkline_date_axis(0, &[rect("G2:H3")], Some((0, rect("A1:E1")))), Ok(true));
    wb.update_sparkline_groups(0, &[rect("G2:I3")], |group| group.markers = true);
    wb.set_col_width(0, 6, Some(160.0));
    wb.set_col_width(0, 7, Some(160.0));
    wb.set_col_width(0, 8, Some(160.0));
    wb.commit("sparklines");
    let bytes = to_bytes(&wb);
    keep_sample("sparkline-date-axis.xlsx", &bytes);
    let xml = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(xml.contains("<x14:sparklineGroup dateAxis=\"1\" displayEmptyCellsAs=\"gap\" markers=\"1\">"), "{xml}");
    assert!(xml.contains("<x14:sparklineGroup type=\"column\" dateAxis=\"1\" displayEmptyCellsAs=\"gap\" markers=\"1\">"), "{xml}");
    assert_eq!(xml.matches("<xm:f>Sheet1!A1:E1</xm:f><x14:sparklines>").count(), 2, "{xml}");
    assert_eq!(xml.matches("dateAxis").count(), 2);
    let back = load(&bytes);
    assert_eq!(back.sheets[0].sparklines, wb.sheets[0].sparklines);
    let dated = &back.sheets[0].sparklines[0];
    assert!(back.sparkline_dates(dated).unwrap().iter().all(|date| date.is_some_and(|day| day > 45000.0)));
    assert_eq!(to_bytes(&back), bytes);
}
