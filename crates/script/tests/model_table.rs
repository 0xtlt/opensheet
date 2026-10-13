//! Formats, frozen panes, filters and tables.

mod common;
use common::*;
use opensheet_engine::style::BorderStyle;
use opensheet_engine::workbook::Workbook;
use opensheet_script::Autofit;

fn stock() -> Workbook {
    book(&[
        ("A1", "Name"),
        ("B1", "Qty"),
        ("C1", "Price"),
        ("A2", "Pen"),
        ("B2", "10"),
        ("C2", "1.5"),
        ("A3", "Ink"),
        ("B3", "2"),
        ("C3", "12.25"),
        ("A4", "Pad"),
        ("B4", "5"),
        ("C4", "3"),
    ])
}

#[test]
fn fonts_fills_borders_alignment_and_sizes() {
    let mut wb = stock();
    let out = ok(&mut wb, include_str!("model/format.ts"));
    let header = wb.style_at(0, 0, 1).clone();
    assert!(header.font.bold && header.font.italic && !header.font.strike && header.wrap);
    assert_eq!((header.font.name.as_ref(), header.font.size), ("Arial", 28));
    assert_eq!(header.fill, None);
    assert_eq!(wb.style_at(0, 0, 2).fill.map(|c| c.value()), Some(0x4472C4));
    // The lines inside are the bottom of a cell and the top of the next.
    let middle = wb.style_at(0, 2, 1).borders;
    assert_eq!((middle.top.style, middle.bottom.style), (BorderStyle::Dotted, BorderStyle::Dotted));
    assert_eq!(middle.left.color.map(|c| c.value()), Some(0x808080));
    assert_eq!(wb.style_at(0, 1, 0).borders.top.style, BorderStyle::Hair);
    assert_eq!(wb.style_at(0, 1, 0).borders.left.style, BorderStyle::Thin);
    assert_eq!(wb.style_at(0, 3, 2).borders.right.style, BorderStyle::Medium);
    assert_eq!(wb.style_at(0, 3, 0).borders.bottom.style, BorderStyle::None);
    assert!(!wb.style_at(0, 2, 1).locked && wb.style_at(0, 2, 1).hidden);
    assert_eq!(
        out.autofit,
        [
            Autofit { sheet: 0, columns: true, first: 0, last: 4 },
            Autofit { sheet: 0, columns: false, first: 0, last: 3 },
        ]
    );
}

#[test]
fn frozen_panes_and_the_filter_of_a_sheet() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/filter.ts"));
    assert_eq!((wb.sheets[0].frozen_rows, wb.sheets[0].frozen_cols), (1, 1));
    assert!(wb.sheets[0].auto_filter.is_some() && wb.sheets[0].filtered_rows.is_empty());
}

#[test]
fn tables_their_columns_and_their_filters() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/tables.ts"));
    assert_eq!(wb.sheets[0].tables.len(), 1);
    assert_eq!(wb.sheets[0].tables[0].columns.len(), 2);
}
