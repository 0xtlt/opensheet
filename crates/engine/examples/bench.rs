//! Recalculation benchmark and long differential test.
//!
//! ```text
//! cargo run -p opensheet_engine --release --example bench            # timings
//! cargo run -p opensheet_engine --release --example bench -- memory  # the graph of a million formulas
//! cargo run -p opensheet_engine --release --example bench -- fuzz 2000 300 [first seed]
//! ```

use opensheet_engine::eval::Calc;
use opensheet_engine::ops::PasteMode;
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Locale, Rect, Workbook};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[path = "../tests/common/recalc_fuzz.rs"]
mod recalc_fuzz;

/// Counts the bytes in use, to weigh the dependency graph.
struct Counting;

static IN_USE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        IN_USE.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        IN_USE.fetch_add(new_size, Ordering::Relaxed);
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn megabytes(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
}

fn timed(mut f: impl FnMut()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn best(n: u32, mut f: impl FnMut()) -> Duration {
    (0..n).map(|_| timed(&mut f)).min().unwrap()
}

/// Best of `n` runs of an edit followed by a commit.
fn edit(wb: &mut Workbook, row: u32, col: u32, n: u32) -> Duration {
    let mut i = 1;
    best(n, || {
        i += 1;
        wb.set_cell_text(0, row, col, &i.to_string()).unwrap();
        wb.commit("edit");
    })
}

/// `rows` rows of a number and two formulas: one reading the number and a
/// range shared by all rows, one reading the number and the first formula.
fn typical(rows: u32) -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for r in 0..rows {
        wb.set_cell_text(0, r, 0, &format!("{}", r % 97)).unwrap();
        wb.set_cell_text(0, r, 1, &format!("=A{}*2+SUM($A$1:$A$10)", r + 1))
            .unwrap();
        wb.set_cell_text(
            0,
            r,
            2,
            &format!("=IF(B{0}>100,\"high\",\"low\")&A{0}", r + 1),
        )
        .unwrap();
    }
    wb
}

/// The time of a full recalculation, and what building the dependency
/// graph adds to it.
fn full_and_graph(wb: &mut Workbook) -> (Duration, Duration) {
    wb.set_incremental(false);
    let full = best(3, || wb.recalc());
    wb.set_incremental(true);
    let both = best(3, || wb.recalc());
    (full, both.saturating_sub(full))
}

fn timings() {
    let mut wb = typical(100_000);
    let first = timed(|| {
        wb.commit("fill");
    });
    println!("200 000 formulas");
    println!("  first calculation, graph included   {first:?}");
    let (full, graph) = full_and_graph(&mut wb);
    println!("  full recalculation                  {full:?}");
    println!("  building the graph                  {graph:?}");
    println!(
        "  edit a cell nothing reads           {:?}",
        edit(&mut wb, 0, 25, 9)
    );
    println!(
        "  edit a cell two formulas read       {:?}",
        edit(&mut wb, 50_000, 0, 9)
    );
    println!("  edit a formula                      {:?}", {
        let mut i = 1;
        best(9, || {
            i += 1;
            wb.set_cell_text(0, 60_000, 1, &format!("=A60001*{i}"))
                .unwrap();
            wb.commit("edit");
        })
    });
    println!(
        "  undo, redo                          {:?}",
        best(9, || {
            wb.undo();
            wb.redo();
        }) / 2
    );
    let clip = wb.copy(0, Rect::new(0, 0, 999, 0));
    println!(
        "  paste 1000 cells 2000 formulas read {:?}",
        best(5, || {
            wb.paste(0, Rect::cell(70_000, 0), &clip, PasteMode::Values);
            wb.commit("paste");
            wb.undo();
        }) / 2
    );
    println!(
        "  fill 1000 formulas down             {:?}",
        best(5, || {
            wb.fill_down(0, Rect::new(80_000, 1, 80_999, 1));
            wb.commit("fill");
            wb.undo();
        }) / 2
    );
    println!(
        "  change the sheet, no cell (freeze)  {:?}",
        best(5, || {
            wb.set_frozen(0, 1, 0);
            wb.commit("freeze");
            wb.set_frozen(0, 0, 0);
            wb.commit("freeze");
        }) / 2
    );
    println!(
        "  insert a row at the top             {:?}",
        timed(|| {
            wb.insert_rows(0, 0, 1);
            wb.commit("insert");
        })
    );
    wb.undo();
    println!(
        "  edit a cell every formula reads     {:?}",
        edit(&mut wb, 0, 0, 3)
    );
    let expr = parse_formula("AND(A5>3,COUNTIF($A$1:$A$100,A5)>1)", Locale::en()).unwrap();
    let once = timed(|| {
        for r in 0..1000 {
            Calc::eval_once(&wb, 0, r, 5, &expr);
        }
    });
    let session = timed(|| {
        let session = wb.eval_session();
        for r in 0..1000 {
            session.eval(0, r, 5, &expr);
        }
    });
    println!(
        "  evaluate an expression: eval_once   {:?}, in a session {:?}",
        once / 1000,
        session / 1000
    );

    for down in [false, true] {
        let mut wb = Workbook::new(Locale::en());
        let n = 100_000u32;
        let (head, tail) = if down { (n, 0) } else { (0, n) };
        wb.set_cell_text(0, head, 0, "1").unwrap();
        for r in 0..n {
            let (row, read) = if down { (r, r + 1) } else { (r + 1, r) };
            wb.set_cell_text(0, row, 0, &format!("=A{}+1", read + 1))
                .unwrap();
        }
        let first = timed(|| {
            wb.commit("fill");
        });
        let way = if down { "the next" } else { "the previous" };
        println!("chain of 100 000 formulas, each reading {way} one");
        println!("  first calculation, graph included   {first:?}");
        let (full, graph) = full_and_graph(&mut wb);
        println!("  full recalculation                  {full:?}");
        println!("  building the graph                  {graph:?}");
        println!(
            "  edit the head of the chain          {:?}",
            edit(&mut wb, head, 0, 5)
        );
        println!(
            "  the other end reads                 {:?}",
            wb.value_at(0, tail, 0)
        );
    }

    let mut wb = Workbook::new(Locale::en());
    for r in 0..50_000u32 {
        wb.set_cell_text(0, r, 0, &format!("k{r}")).unwrap();
        wb.set_cell_text(0, r, 1, &format!("{r}")).unwrap();
    }
    for r in 0..1000u32 {
        wb.set_cell_text(0, r, 3, &format!("=VLOOKUP(\"k{}\",A:B,2,FALSE)", r * 37))
            .unwrap();
    }
    println!("1000 VLOOKUPs over two whole columns of 50 000 rows");
    println!(
        "  first calculation                   {:?}",
        timed(|| {
            wb.commit("lookup");
        })
    );
    println!(
        "  edit a cell of the columns          {:?}",
        edit(&mut wb, 40_000, 1, 3)
    );
    println!(
        "  edit a cell beside them             {:?}",
        edit(&mut wb, 10, 7, 5)
    );
}

/// What the dependency graph of a million formulas weighs.
fn memory() {
    let shapes: [(&str, fn(u32) -> Workbook); 2] = [
        ("two per row: cells and one shared range", |rows| {
            typical(rows / 2)
        }),
        (
            "each summing its own ten cells and reading two more",
            |rows| {
                let mut wb = Workbook::new(Locale::en());
                for r in 0..rows {
                    wb.set_cell_text(0, r, 0, &format!("{}", r % 97)).unwrap();
                    let formula = format!("=SUM(A{}:A{})+A{}*C{}", r + 1, r + 10, r + 1, r + 2);
                    wb.set_cell_text(0, r, 1, &formula).unwrap();
                }
                wb
            },
        ),
    ];
    for (what, build) in shapes {
        let mut wb = build(1_000_000);
        wb.set_incremental(false);
        wb.commit("fill");
        let without = IN_USE.load(Ordering::Relaxed);
        wb.set_incremental(true);
        let both = timed(|| wb.recalc());
        let with = IN_USE.load(Ordering::Relaxed);
        let stats = wb.graph_stats().expect("a graph");
        println!("1 000 000 formulas, {what}");
        println!(
            "  graph: {} in use ({} by its own count), {} links to cells, {} to ranges",
            megabytes(with.saturating_sub(without)),
            megabytes(stats.bytes),
            stats.cell_links,
            stats.range_links
        );
        println!("  the workbook without it: {}", megabytes(without));
        println!("  full recalculation, graph included  {both:?}");
        println!(
            "  edit a cell nothing reads           {:?}",
            edit(&mut wb, 0, 25, 9)
        );
        println!(
            "  edit a cell two formulas read       {:?}",
            edit(&mut wb, 500_000, 0, 9)
        );
    }
}

/// The differential test of `tests/recalc_diff.rs`, for as long as wanted.
fn fuzz(args: &[String]) {
    let number =
        |i: usize, default: u64| args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default);
    let (seeds, steps, first) = (number(0, 1000), number(1, 200), number(2, 0));
    let started = Instant::now();
    let mut total = recalc_fuzz::Tally::default();
    for seed in first..first + seeds {
        match recalc_fuzz::run(seed, steps as usize) {
            Ok(tally) => total.add(&tally),
            Err(what) => {
                eprintln!("{what}");
                std::process::exit(1);
            }
        }
    }
    println!("{} workbooks in {:?}: {total}", seeds, started.elapsed());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fuzz") => fuzz(&args[1..]),
        Some("memory") => memory(),
        _ => timings(),
    }
}
