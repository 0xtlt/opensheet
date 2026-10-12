//! PivotTable benchmark: `cargo run -p opensheet_engine --release --example pivot_bench`.
//!
//! 100 000 source rows, two row fields, one column field and two value
//! fields, in two shapes: a report of a few hundred cells, and one of a
//! few hundred thousand.

use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};
use opensheet_engine::{Content, Locale, Rect, Value, Workbook};
use std::time::Instant;

const ROWS: u32 = 100_000;

/// Fill sheet 0 with `ROWS` records: Region, Product, Year, Sales, Units.
fn source(regions: u32, products: u32) -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (c, name) in ["Region", "Product", "Year", "Sales", "Units"].iter().enumerate() {
        wb.set_content(0, 0, c as u32, Content::Value(Value::text(name)));
    }
    // A small generator, so that the data is the same on every run.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as u32
    };
    for r in 1..=ROWS {
        wb.set_content(
            0,
            r,
            0,
            Content::Value(Value::text(format!("Region {}", next(regions)))),
        );
        wb.set_content(
            0,
            r,
            1,
            Content::Value(Value::text(format!("Product {}", next(products)))),
        );
        wb.set_content(0, r, 2, Content::Value(Value::Number(2015.0 + next(10) as f64)));
        wb.set_content(0, r, 3, Content::Value(Value::Number(next(100_000) as f64 / 100.0)));
        wb.set_content(0, r, 4, Content::Value(Value::Number(next(50) as f64)));
    }
    wb.commit("data");
    wb.clear_history();
    wb
}

fn run(label: &str, regions: u32, products: u32) {
    let mut wb = source(regions, products);
    let range = Rect::new(0, 0, ROWS, 4);
    let source = PivotSource::Range {
        sheet: wb.sheets[0].name.clone(),
        range,
    };
    let t = Instant::now();
    let id = wb
        .pivot_create(source, PivotDestination::NewSheet { index: 1 }, false)
        .unwrap();
    wb.commit("create");
    let create = t.elapsed();
    let locale = wb.locale;
    let t = Instant::now();
    wb.pivot_update(id, false, |p| {
        p.add_field(0, Area::Rows, None, locale);
        p.add_field(1, Area::Rows, None, locale);
        p.add_field(2, Area::Columns, None, locale);
        p.add_field(3, Area::Values, None, locale);
        p.add_field(4, Area::Values, None, locale);
    })
    .unwrap();
    let layout = t.elapsed();
    let t = Instant::now();
    wb.commit("layout");
    let commit = t.elapsed();
    // Change the source, then refresh: read, aggregate, lay out, write.
    wb.set_content(0, 1, 3, Content::Value(Value::Number(1.0)));
    wb.commit("edit");
    let t = Instant::now();
    wb.pivot_refresh(id, false).unwrap();
    let refresh = t.elapsed();
    let t = Instant::now();
    wb.commit("refresh");
    let refresh_commit = t.elapsed();
    let size = wb.pivot(id).unwrap().1.report.size();
    println!(
        "{label}: report of {} x {} cells; create (read the source) {create:?}, lay out {layout:?} (+ commit {commit:?}), \
         refresh {refresh:?} (+ commit {refresh_commit:?})",
        size.0, size.1
    );
}

fn main() {
    run("5 regions x 40 products", 5, 40);
    run("50 regions x 400 products", 50, 400);
}
