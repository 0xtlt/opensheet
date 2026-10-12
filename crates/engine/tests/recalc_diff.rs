//! Incremental recalculation against full recalculation, on random
//! workbooks and random edits (see `common/recalc_fuzz.rs`).
//!
//! The seeds are fixed: a failure names its seed and lists the steps. To
//! run far more of them:
//! `cargo run -p opensheet_engine --release --example bench -- fuzz 2000 300`.

#[path = "common/recalc_fuzz.rs"]
mod recalc_fuzz;

const STEPS: usize = 160;

fn workbooks(seeds: std::ops::Range<u64>) {
    let mut total = recalc_fuzz::Tally::default();
    for seed in seeds {
        match recalc_fuzz::run(seed, STEPS) {
            Ok(tally) => total.add(&tally),
            Err(what) => panic!("{what}"),
        }
    }
    println!("{total}");
    // The test is only worth something while most steps are recalculated
    // incrementally, and while it meets what is hard about it.
    assert!(total.partial > 2 * total.full, "{total}");
    assert!(
        total.circular > 0 && total.spilling > total.steps / 4 && total.blocked > 0,
        "{total}"
    );
}

#[test]
fn incremental_recalculation_matches_a_full_one_1() {
    workbooks(0..16);
}

#[test]
fn incremental_recalculation_matches_a_full_one_2() {
    workbooks(16..32);
}

#[test]
fn incremental_recalculation_matches_a_full_one_3() {
    workbooks(32..48);
}

#[test]
fn incremental_recalculation_matches_a_full_one_4() {
    workbooks(48..64);
}
