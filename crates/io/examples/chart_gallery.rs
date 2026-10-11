//! Writes a workbook with one chart of every kind, to look at in OpenSheet
//! or in Excel: `cargo run -p opensheet_io --example chart_gallery -- out.xlsx`

use opensheet_engine::chart::{ChartKind, ChartTitle};
use opensheet_engine::locale::Locale;
use opensheet_engine::refs::Rect;
use opensheet_engine::workbook::Workbook;

const DATA: [[&str; 4]; 6] = [
    ["Month", "North", "South", "West"],
    ["Jan", "12", "8", "5"],
    ["Feb", "15", "11", "7"],
    ["Mar", "9", "14", "10"],
    ["Apr", "18", "10", "12"],
    ["May", "21", "13", "9"],
];

const KINDS: [(ChartKind, &str); 9] = [
    (ChartKind::Column, "Column"),
    (ChartKind::StackedColumn, "Stacked column"),
    (ChartKind::Line, "Line"),
    (ChartKind::Bar, "Bar"),
    (ChartKind::StackedBar, "Stacked bar"),
    (ChartKind::Area, "Area"),
    (ChartKind::Pie, "Pie"),
    (ChartKind::Doughnut, "Doughnut"),
    (ChartKind::Scatter, "Scatter"),
];

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "chart_gallery.xlsx".to_string());
    let mut wb = Workbook::new(Locale::en());
    for (r, row) in DATA.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).expect("plain values");
        }
    }
    wb.recalc();
    let block = Rect::parse("A1:D6").expect("a range");
    for (i, (kind, title)) in KINDS.into_iter().enumerate() {
        // The scatter chart reads its x values from numbers, not from months.
        let source = if kind == ChartKind::Scatter { Rect::parse("B1:D6").expect("a range") } else { block };
        let mut chart = wb.chart_from_range(0, source, kind);
        chart.title = ChartTitle::Text(title.to_string());
        chart.anchor = (1 + 10 * (i as u32 / 3), 5 + 5 * (i as u32 % 3));
        chart.offset = (0.0, 0.0);
        chart.size = (310.0, 190.0);
        wb.add_chart(0, chart);
    }
    opensheet_io::save_xlsx(&wb, std::path::Path::new(&path)).expect("the workbook to be written");
    println!("{path}");
}
