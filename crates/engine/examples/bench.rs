//! Rough recalculation benchmark: `cargo run -p opensheet_engine --release --example bench`.

use opensheet_engine::{Locale, Workbook};
use std::time::Instant;

fn main() {
    for rows in [10_000u32, 100_000] {
        let mut wb = Workbook::new(Locale::en());
        let t = Instant::now();
        for r in 0..rows {
            wb.set_cell_text(0, r, 0, &format!("{}", r % 97)).unwrap();
            wb.set_cell_text(0, r, 1, &format!("=A{}*2+SUM($A$1:$A$10)", r + 1)).unwrap();
            wb.set_cell_text(0, r, 2, &format!("=IF(B{0}>100,\"high\",\"low\")&A{0}", r + 1)).unwrap();
        }
        let build = t.elapsed();
        let t = Instant::now();
        wb.commit("fill");
        let first = t.elapsed();
        let t = Instant::now();
        wb.set_cell_text(0, 0, 0, "5").unwrap();
        wb.commit("edit");
        let edit = t.elapsed();
        println!(
            "{rows} rows ({} formulas): build {build:?}, first recalc {first:?}, recalc after one edit {edit:?}",
            rows * 2
        );
    }
    let mut wb = Workbook::new(Locale::en());
    for r in 0..50_000u32 {
        wb.set_cell_text(0, r, 0, &format!("k{r}")).unwrap();
        wb.set_cell_text(0, r, 1, &format!("{r}")).unwrap();
    }
    for r in 0..1000u32 {
        wb.set_cell_text(0, r, 3, &format!("=VLOOKUP(\"k{}\",A:B,2,FALSE)", r * 37)).unwrap();
    }
    let t = Instant::now();
    wb.commit("lookup");
    println!("1000 VLOOKUPs over 50k rows: {:?}", t.elapsed());
}
