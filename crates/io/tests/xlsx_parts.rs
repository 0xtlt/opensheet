//! One workbook with everything that lives in parts of its own: a table,
//! PivotTables, a picture, shapes, a chart, a note and a filtered list.
//! The parts are tested one by one elsewhere; here they share a package.

mod common;

use common::{assert_well_formed, at, enter, part, part_names, value};
use opensheet_engine::ast::Expr;
use opensheet_engine::chart::{
    Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
};
use opensheet_engine::drawing::{Frame, ShapeTool};
use opensheet_engine::filter::{Criteria, FilterScope, ValueFilter};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};
use opensheet_engine::table::{TableOption, TotalsFunction};
use opensheet_engine::{Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::{Cursor, Read};
use std::sync::Arc;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// A PNG image of 96 × 64 pixels: stripes of three colours.
fn picture() -> Vec<u8> {
    let (width, height) = (96u32, 64u32);
    let colors = [[0x15, 0x60, 0x82], [0xE9, 0x71, 0x32], [0x19, 0x6B, 0x24]];
    let mut pixels = Vec::new();
    for y in 0..height {
        pixels.push(0);
        for x in 0..width {
            pixels.extend_from_slice(&colors[(((x + y) / 16) % 3) as usize]);
        }
    }
    // A zlib stream of one block stored as it is, with its checksum.
    let mut stream = vec![0x78, 0x01, 1];
    stream.extend_from_slice(&(pixels.len() as u16).to_le_bytes());
    stream.extend_from_slice(&(!(pixels.len() as u16)).to_le_bytes());
    stream.extend_from_slice(&pixels);
    let (mut a, mut b) = (1u32, 0u32);
    for byte in &pixels {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    stream.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (b"IHDR", header.as_slice()),
        (b"IDAT", stream.as_slice()),
        (b"IEND", &[]),
    ] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    out
}

const SALES: &[[&str; 5]] = &[
    ["Region", "Product", "Date", "Amount", "Units"],
    ["East", "Apples", "1/15/2023", "100", "10"],
    ["East", "Pears", "3/2/2023", "150.5", "5"],
    ["East", "Apples", "7/20/2024", "120", "12"],
    ["West", "Apples", "11/5/2023", "200", "20"],
    ["West", "Pears", "2/14/2024", "50", "2"],
    ["West", "Pears", "8/30/2024", "70", "7"],
    ["North", "Plums", "5/5/2024", "35", "3"],
    ["North", "Apples", "12/24/2024", "64.25", "8"],
    ["South", "Plums", "6/1/2023", "90", "9"],
    ["South", "Pears", "9/9/2024", "110", "11"],
];

fn everything() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for n in 1..=3 {
        wb.add_sheet(n);
    }
    assert!(wb.rename_sheet(0, "Sales"));
    assert!(wb.rename_sheet(1, "Pivots"));
    assert!(wb.rename_sheet(2, "Drawings"));
    assert!(wb.rename_sheet(3, "List"));

    // A table with a calculated column and a totals row, and a note.
    for (r, row) in SALES.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    wb.update_style(0, rect("D2:D11"), &|s| s.num_fmt = Arc::from("#,##0.00"));
    let table = wb.add_table(0, rect("A1:E11"), true).unwrap();
    wb.rename_table(0, table, "Orders").unwrap();
    enter(&mut wb, 0, "F1", "Price");
    enter(&mut wb, 0, "F2", "=[@Amount]/[@Units]");
    wb.tables_after_edit(0, rect("F2"));
    wb.set_table_option(0, table, TableOption::TotalsRow, true)
        .unwrap();
    assert!(wb.set_totals_function(0, table, 3, TotalsFunction::Sum));
    assert!(wb.set_note(0, 0, 7, "Ann", "Ann:\nThe orders of two years."));
    // Calculated before the PivotTables take their values from it.
    wb.commit("table");

    // Two PivotTables on caches of their own: regions by product on the
    // table, and the dates of the range grouped by year, quarter and month.
    let on = |cell: &str| {
        let (row, col) = at(cell);
        PivotDestination::Existing { sheet: 1, row, col }
    };
    let by_region = wb
        .pivot_create(PivotSource::Name("Orders".to_string()), on("A3"), false)
        .unwrap();
    wb.pivot_add_field(by_region, 0, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(by_region, 1, Area::Columns, None, false)
        .unwrap();
    wb.pivot_add_field(by_region, 3, Area::Values, None, false)
        .unwrap();
    let by_date = wb
        .pivot_create(
            PivotSource::Range {
                sheet: "Sales".to_string(),
                range: rect("A1:E11"),
            },
            on("H3"),
            false,
        )
        .unwrap();
    wb.pivot_add_field(by_date, 2, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(by_date, 4, Area::Values, None, false)
        .unwrap();

    // A picture, shapes and a chart on one sheet.
    assert!(wb.add_picture(2, picture(), (1, 1)).is_some());
    let hello = wb
        .add_shape(
            2,
            ShapeTool::RoundedRectangle,
            Frame::new(200.0, 20.0, 140.0, 60.0),
        )
        .unwrap();
    assert!(wb.update_drawing(2, hello, |d| {
        d.as_shape_mut().unwrap().text = "A shape\nwith text".to_string();
    }));
    assert!(
        wb.add_shape(2, ShapeTool::Arrow, Frame::new(200.0, 110.0, 140.0, 30.0))
            .is_some()
    );
    let turned = wb
        .add_shape(
            2,
            ShapeTool::RightArrow,
            Frame::new(380.0, 60.0, 96.0, 48.0),
        )
        .unwrap();
    assert!(wb.update_drawing(2, turned, |d| d.rotation = 90.0));
    let note = wb
        .add_shape(2, ShapeTool::TextBox, Frame::new(20.0, 130.0, 140.0, 40.0))
        .unwrap();
    assert!(wb.update_drawing(2, note, |d| {
        d.as_shape_mut().unwrap().text = "A text box".to_string();
    }));
    let values = match parse_formula("Sales!$D$2:$D$6", Locale::en()) {
        Ok(Expr::Ref(range)) => range,
        other => panic!("not a reference: {other:?}"),
    };
    let chart = Chart {
        id: 0,
        kind: ChartKind::Column,
        title: ChartTitle::Auto,
        legend: LegendPos::None,
        series: vec![Series {
            name: SeriesName::None,
            categories: None,
            color: None,
            ..Series::new(values)
        }],
        data_labels: false,
        anchor: (10, 1),
        offset: (0.0, 0.0),
        size: DEFAULT_CHART_SIZE,
        ..Chart::new(ChartKind::Column)
    };
    assert!(wb.add_chart(2, chart).is_some());

    // A list filtered through the filter of its sheet.
    for (r, row) in [
        ["Name", "Team"],
        ["Ann", "Red"],
        ["Bob", "Blue"],
        ["Cid", "Red"],
        ["Dee", "Green"],
    ]
    .iter()
    .enumerate()
    {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(3, r as u32, c as u32, text).unwrap();
        }
    }
    wb.set_auto_filter(3, rect("A1:B5")).unwrap();
    assert!(wb.set_filter_criteria(
        3,
        FilterScope::Sheet,
        1,
        Some(Criteria::Values(ValueFilter {
            values: vec!["Red".into()],
            blanks: false,
            dates: Vec::new(),
        })),
    ));
    wb.commit("everything");
    wb
}

#[test]
fn everything_shares_a_package() {
    let original = everything();
    let bytes = to_bytes(&original);
    common::keep_sample("tables-pivots-drawings.xlsx", &bytes);
    for name in part_names(&bytes) {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_well_formed(&name, &part(&bytes, &name));
        }
    }
    for n in 1..=4 {
        let name = format!("xl/worksheets/sheet{n}.xml");
        common::assert_worksheet_order(&name, &part(&bytes, &name));
    }
    common::assert_workbook_order(&part(&bytes, "xl/workbook.xml"));
    // The first sheet points at its note boxes, its notes and its table;
    // the second at its PivotTables; the third at its drawing.
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").ends_with(
        "<legacyDrawing r:id=\"rId1\"/><tableParts count=\"1\"><tablePart r:id=\"rId3\"/></tableParts></worksheet>"
    ));
    assert!(part(&bytes, "xl/worksheets/_rels/sheet2.xml.rels").contains("pivotTable2.xml"));
    assert!(part(&bytes, "xl/worksheets/sheet3.xml").contains("<drawing r:id=\"rId1\"/>"));
    assert!(part(&bytes, "xl/worksheets/sheet4.xml").contains("<autoFilter ref=\"A1:B5\">"));

    let loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).expect("the package loads");
    assert!(!loaded.modified);
    for (a, b) in original.sheets.iter().zip(&loaded.sheets) {
        assert_eq!(a.tables, b.tables, "{}", a.name);
        assert_eq!(a.auto_filter, b.auto_filter, "{}", a.name);
        assert_eq!(a.filtered_rows, b.filtered_rows, "{}", a.name);
        assert_eq!(a.comments, b.comments, "{}", a.name);
        assert_eq!(a.charts.len(), b.charts.len(), "{}", a.name);
        assert_eq!(a.drawings.len(), b.drawings.len(), "{}", a.name);
        for (da, db) in a.drawings.iter().zip(&b.drawings) {
            let mut db = db.clone();
            db.id = da.id;
            assert_eq!(*da, db, "{}", da.name);
        }
        assert_eq!(a.pivots.len(), b.pivots.len(), "{}", a.name);
        for (pa, pb) in a.pivots.iter().zip(&b.pivots) {
            let mut pb = pb.clone();
            pb.id = pa.id;
            assert_eq!(*pa, pb, "{}", pa.name);
        }
    }
    // The cells say the same: totals of the table, values of the reports.
    for (sheet, cell) in [(0, "D12"), (0, "F2"), (1, "B5"), (1, "E9"), (1, "I4")] {
        assert_eq!(
            value(&original, sheet, cell),
            value(&loaded, sheet, cell),
            "{cell}"
        );
    }
    assert_eq!(value(&loaded, 0, "D12"), Value::Number(989.75));

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    let raw = |bytes: &[u8], name: &str| {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut data = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut data).unwrap();
        data
    };
    for name in part_names(&bytes) {
        assert_eq!(raw(&bytes, &name), raw(&again, &name), "{name}");
    }
}
