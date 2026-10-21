//! Whatever a script passes, a call fails or answers: the engine under
//! it does not panic, and the run is undone like any other.

mod common;
use common::*;
use std::time::Duration;

use opensheet_script::{Context, api};

#[test]
fn no_member_brings_the_engine_down() {
    let classes: Vec<String> = api()
        .iter()
        .map(|class| {
            let members: Vec<String> = class
                .members
                .iter()
                .map(|m| format!("{:?}", m.name))
                .collect();
            format!(
                "{{\"name\":{:?},\"members\":[{}]}}",
                class.name,
                members.join(",")
            )
        })
        .collect();
    let source = format!(
        "const CLASSES = {:?};\n{}",
        format!("[{}]", classes.join(",")),
        include_str!("robust.ts")
    );
    let mut wb = book(&[("A1", "1")]);
    let context = Context {
        time_limit: Duration::from_secs(600),
        ..Context::default()
    };
    // Sheets are protected on the way: the run is kept without a step to
    // undo, so it is not undone and redone here as other scripts are.
    let out = run_with(&mut wb, &source, &context);
    assert_eq!(out.error, None);
    assert!(wb.commit("Script"));
    let members: usize = api().iter().map(|class| class.members.len()).sum();
    assert_eq!(out.returned, Some((members * 30).to_string()));
}
