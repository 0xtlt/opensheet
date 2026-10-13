//! What the tests of the script engine share: a workbook to run on, the
//! contract of the caller around `run`, and checks written in the scripts
//! themselves.
#![allow(dead_code)]

use opensheet_engine::locale::Locale;
use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::value::Value;
use opensheet_engine::workbook::Workbook;
use opensheet_script::{Context, Outcome, ScriptError, run};

/// Checks a script makes on what it reads: they are declared after the
/// script, so its lines keep their numbers.
const CHECKS: &str = r#"
// Objects are compared whatever the order of their keys: the one plain
// data comes back in depends on how the JSON library was built.
function sorted(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sorted);
  }
  if (value !== null && typeof value === "object" && Object.getPrototypeOf(value) === Object.prototype) {
    const out: { [key: string]: unknown } = {};
    for (const key of Object.keys(value).sort()) {
      out[key] = sorted((value as { [key: string]: unknown })[key]);
    }
    return out;
  }
  return value;
}
function eq(actual: unknown, expected: unknown, what?: string): void {
  const a = JSON.stringify(sorted(actual));
  const e = JSON.stringify(sorted(expected));
  if (a !== e) {
    throw new Error(`${what ?? "value"}: expected ${e}, got ${a}`);
  }
}
function near(actual: number, expected: number, what?: string): void {
  if (!(Math.abs(actual - expected) < 0.01)) {
    throw new Error(`${what ?? "value"}: expected about ${expected}, got ${actual}`);
  }
}
function throws(run: () => void, part: string): void {
  let message = "";
  try {
    run();
  } catch (error) {
    message = String((error as Error).message);
    if (message.includes(part)) {
      return;
    }
  }
  throw new Error(`expected an error saying "${part}", got "${message}"`);
}
"#;

/// A workbook with cells typed on its first sheet, as after a load: no
/// history.
pub fn book(cells: &[(&str, &str)]) -> Workbook {
    book_in(Locale::en(), cells)
}

pub fn book_in(locale: &'static Locale, cells: &[(&str, &str)]) -> Workbook {
    let mut wb = Workbook::new(locale);
    for (name, text) in cells {
        let (row, col) = parse_cell_name(name).expect("a cell name");
        wb.set_cell_text(0, row, col, text).expect("an entry");
    }
    wb.commit("setup");
    wb.clear_history();
    wb
}

/// Everything undo must bring back.
pub fn state(wb: &Workbook) -> String {
    format!("{:?}\n{:?}", wb.sheets, wb.names)
}

pub fn value(wb: &Workbook, sheet: usize, name: &str) -> Value {
    let (row, col) = parse_cell_name(name).expect("a cell name");
    wb.value_at(sheet, row, col)
}

pub fn number(wb: &Workbook, name: &str) -> f64 {
    match value(wb, 0, name) {
        Value::Number(n) => n,
        other => panic!("{name} holds {other:?}"),
    }
}

pub fn text(wb: &Workbook, name: &str) -> String {
    let (row, col) = parse_cell_name(name).expect("a cell name");
    wb.display_text(0, row, col).text
}

/// `main` around statements; `sheet` is the active worksheet.
pub fn script(body: &str) -> String {
    format!("function main(workbook: ExcelScript.Workbook) {{ const sheet = workbook.getActiveWorksheet();\n{body}\n}}\n")
}

pub fn run_with(wb: &mut Workbook, source: &str, context: &Context) -> Outcome {
    wb.snapshot();
    run(&format!("{source}\n{CHECKS}"), wb, context)
}

fn explain(source: &str, error: &ScriptError) -> String {
    let line = error.line.and_then(|l| source.lines().nth(l as usize - 1)).unwrap_or_default();
    format!("{:?} at {:?}:{:?}: {}\n    {}", error.kind, error.line, error.column, error.message, line.trim())
}

/// Run a script that must reach its end, as the application does: the
/// changes are one undo step, which is undone and redone here.
pub fn ok_with(wb: &mut Workbook, source: &str, context: &Context) -> Outcome {
    let before = state(wb);
    let outcome = run_with(wb, source, context);
    if let Some(error) = &outcome.error {
        panic!("the script failed: {}\nlogs: {:?}", explain(source, error), outcome.logs);
    }
    if wb.commit("Script") {
        let after = state(wb);
        assert!(wb.undo(), "one step to undo");
        assert!(state(wb) == before, "undo brings the workbook back");
        assert!(wb.redo());
        assert!(state(wb) == after, "redo makes the changes again");
    }
    outcome
}

pub fn ok(wb: &mut Workbook, source: &str) -> Outcome {
    ok_with(wb, source, &Context::default())
}

/// The statements of a `main`, run to their end.
pub fn ok_body(wb: &mut Workbook, body: &str) -> Outcome {
    ok(wb, &script(body))
}

/// Run a script that must fail: the workbook is as it was, before and
/// after the caller's rollback.
pub fn fails_with(wb: &mut Workbook, source: &str, context: &Context) -> (ScriptError, Outcome) {
    let before = state(wb);
    let outcome = run_with(wb, source, context);
    let error = outcome.error.clone().unwrap_or_else(|| panic!("the script should have failed; logs: {:?}", outcome.logs));
    assert!(state(wb) == before, "a failed script leaves the workbook as it was");
    wb.rollback();
    assert!(state(wb) == before, "the caller's rollback changes nothing more");
    assert!(!wb.commit("nothing"), "nothing is left to commit");
    (error, outcome)
}

pub fn fails(wb: &mut Workbook, source: &str) -> ScriptError {
    fails_with(wb, source, &Context::default()).0
}
