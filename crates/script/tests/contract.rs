//! The contract of `run` and `check` with the application: failures and
//! where they are, what is not supported, time and memory, what a run
//! reports.

mod common;
use common::*;
use std::time::{Duration, Instant};

use opensheet_engine::Rect;
use opensheet_engine::protect::{Permission, SheetProtection};
use opensheet_engine::recalc::CalcMode;
use opensheet_engine::value::Value;
use opensheet_engine::workbook::Workbook;
use opensheet_script::{Context, ErrorKind, LogLevel, Script, check, run};

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
        ("D2", "=B2*C2"),
    ])
}

fn quick() -> Context {
    Context {
        time_limit: Duration::from_millis(300),
        ..Context::default()
    }
}

#[test]
fn the_language_of_real_scripts() {
    let mut wb = stock();
    let out = ok(&mut wb, include_str!("model/language.ts"));
    assert_eq!(out.returned.as_deref(), Some(r#"{"total":54.5,"names":["Ink","Pad","Pen"]}"#));
    assert_eq!(text(&wb, "E1"), "3 items, 65.4 gross");
}

#[test]
fn a_failed_script_changes_nothing() {
    let mut wb = stock();
    wb.sheets[0].tables.clear();
    let error = fails(
        &mut wb,
        r#"function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  sheet.getRange("B2").setValue(1000);
  if (sheet.getRange("D2").getValue() !== 1500) throw new Error("not calculated");
  sheet.getRange("A1:C1").getFormat().getFont().setBold(true);
  sheet.getRange("1:1").insert(ExcelScript.InsertShiftDirection.down);
  sheet.addTable("A2:C5", true);
  workbook.addWorksheet("More").getRange("A1").setValue(1);
  workbook.addNamedItem("Thing", "=1");
  workbook.getApplication().setCalculationMode(ExcelScript.CalculationMode.manual);
  sheet.getRange("A:A").getFormat().setColumnWidth(200);
  sheet.setName("Renamed");
  sheet.getRange("Z1").setValues([[1, 2]]);
}"#,
    );
    assert_eq!(error.kind, ErrorKind::Runtime);
    assert!(error.message.starts_with("Range setValues: The number of rows or columns"), "{}", error.message);
    // The column is that of the member called.
    assert_eq!((error.line, error.column), (Some(13), Some(24)));
    // The results of formulas are those of the workbook as it was.
    assert_eq!(number(&wb, "D2"), 15.0);
    assert_eq!(wb.calc_mode(), CalcMode::Automatic);
    assert!(!wb.can_undo());
}

#[test]
fn a_script_runs_without_the_callers_snapshot_too() {
    let mut wb = stock();
    let before = state(&wb);
    let out = run(
        "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange('2:2').delete(ExcelScript.DeleteShiftDirection.up);\n  workbook.addWorksheet();\n}",
        &mut wb,
        &Context::default(),
    );
    assert_eq!(out.error, None);
    assert!(wb.commit("Script"));
    assert_eq!(wb.sheets.len(), 2);
    assert!(wb.undo() && state(&wb) == before);
}

#[test]
fn syntax_errors_are_placed_without_running() {
    let errors = check("function main(workbook: ExcelScript.Workbook) {\n  let total = ;\n}\n");
    assert_eq!(errors.len(), 1);
    assert_eq!((errors[0].kind, errors[0].line, errors[0].column), (ErrorKind::Syntax, Some(2), Some(15)));
    let errors = check("function main(workbook: ExcelScript.Workbook) {\n  const a = 1;\n  const a = 2;\n  return (;\n");
    assert!(!errors.is_empty() && errors.iter().all(|e| e.kind == ErrorKind::Syntax && e.line.is_some()));
    let errors = check("function helper() { return 1; }\nconst main = 3;\n");
    assert_eq!(errors.len(), 1);
    assert!(errors[0].message.contains("function named main"), "{}", errors[0].message);
    assert!(check("function main(workbook: ExcelScript.Workbook) {}").is_empty());
    assert!(check(&Script::new("New").body).is_empty());
    // Types are removed, not checked.
    assert!(check("function main(workbook: ExcelScript.Workbook) { const n: number = 'text'; workbook.nope(); }").is_empty());

    // Running such a text gives the same error and touches nothing.
    let mut wb = stock();
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange('A1').setValue(1;\n}");
    assert_eq!((error.kind, error.line), (ErrorKind::Syntax, Some(2)));
    let error = fails(&mut wb, "function notMain() {}");
    assert_eq!(error.kind, ErrorKind::Syntax);
}

#[test]
fn runtime_errors_are_placed_in_the_typescript_text() {
    let mut wb = stock();
    // An unknown sheet is undefined, as in Excel: using it is the error.
    let error = fails(
        &mut wb,
        "interface Row { name: string }\n\nfunction main(workbook: ExcelScript.Workbook) {\n  const rows: Row[] = [];\n  const sheet = workbook.getWorksheet('Nope');\n  sheet.getRange('A1').setValue(rows.length as number);\n}",
    );
    assert_eq!((error.kind, error.line, error.column), (ErrorKind::Runtime, Some(6), Some(3)));
    assert!(error.message.starts_with("TypeError:") && error.message.contains("getRange"), "{}", error.message);

    // Thrown in a helper: the place is where it was thrown.
    let error = fails(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange('A1').setValue('x');\n  check(3);\n}\n\nfunction check(n: number): void {\n  if (n > 2) {\n    throw new Error(`too many: ${n}`);\n  }\n}",
    );
    assert_eq!((error.message.as_str(), error.line), ("too many: 3", Some(8)));
    // The column is a place of the statement at fault, not always its start.
    assert!(error.column.is_some_and(|column| (5..=38).contains(&column)), "{:?}", error.column);

    // Rejected after an await.
    let error = fails(
        &mut wb,
        "async function main(workbook: ExcelScript.Workbook) {\n  await Promise.resolve();\n  const n: number = undefinedName + 1;\n}",
    );
    assert_eq!((error.kind, error.line), (ErrorKind::Runtime, Some(3)));
    assert!(error.message.starts_with("ReferenceError:"), "{}", error.message);

    // Something that is not an Error.
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  throw { code: 7 };\n}");
    assert_eq!((error.kind, error.message.as_str()), (ErrorKind::Runtime, "{code: 7}"));

    // A wrong argument, worded as Excel words it, at the call.
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  const sheet = workbook.getActiveWorksheet();\n  sheet.getRange('A1:B2').getFormat().getFont().setSize('big' as unknown as number);\n}");
    assert_eq!((error.kind, error.line), (ErrorKind::Runtime, Some(3)));
    assert_eq!(error.message, "RangeFont setSize: The argument is invalid or missing or has an incorrect format.");

    // A promise nothing settles.
    let error = fails(&mut wb, "async function main(workbook: ExcelScript.Workbook) {\n  await new Promise(() => {});\n}");
    assert!(error.message.contains("never"), "{}", error.message);
}

#[test]
fn what_opensheet_does_not_have_says_so() {
    let mut wb = stock();
    let unsupported = |wb: &mut Workbook, body: &str, message: &str| {
        let source = format!("function main(workbook: ExcelScript.Workbook) {{\n  const sheet = workbook.getActiveWorksheet();\n  sheet.getRange('A1').setValue('changed');\n  {body}\n}}");
        let error = fails(wb, &source);
        assert_eq!((error.kind, error.message.as_str(), error.line), (ErrorKind::Unsupported, message, Some(4)), "{body}");
    };
    unsupported(&mut wb, "sheet.getRange('A1').getPivotTables();", "Range.getPivotTables is not supported in OpenSheet");
    unsupported(&mut wb, "workbook.getSlicers();", "Workbook.getSlicers is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getProtection().protect();", "Worksheet.getProtection is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getRange('A1').getFormat().getFont().setSuperscript(true);", "RangeFont.setSuperscript is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getRanges('A1,C3');", "Worksheet.getRanges is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getRange('A1').setValue(ExcelScript.ChartType.pie);", "ExcelScript.ChartType is not supported in OpenSheet");
    unsupported(&mut wb, "const x = ExcelScript.ClearApplyTo.resetContents;", "ExcelScript.ClearApplyTo.resetContents is not supported in OpenSheet");
    unsupported(&mut wb, "fetch('https://example.org');", "fetch is not supported in OpenSheet");
    unsupported(&mut wb, "OfficeScript.saveCopyAs('copy.xlsx');", "OfficeScript.saveCopyAs is not supported in OpenSheet");
    unsupported(&mut wb, "setTimeout(() => {}, 10);", "setTimeout is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getRange('A1').setVisibility('Hidden');", "Range.setVisibility is not supported in OpenSheet");
    unsupported(&mut wb, "sheet.getRange('A1:B2').getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeTop).setStyle(ExcelScript.BorderLineStyle.slantDashDot);", "ExcelScript.BorderLineStyle.slantDashDot is not supported in OpenSheet");
    // A script can step around what is missing.
    ok(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  try {\n    workbook.getPivotTables();\n  } catch (error) {\n    console.log((error as Error).message);\n  }\n}",
    );

    // Scripts called from Power Automate take more than the workbook.
    let error = fails(&mut wb, "\nfunction main(workbook: ExcelScript.Workbook, name: string, rows?: number) {\n  workbook.getActiveWorksheet().setName(name);\n}");
    assert_eq!((error.kind, error.line, error.column), (ErrorKind::Unsupported, Some(2), Some(1)));
    assert!(error.message.contains("Power Automate"));
}

#[test]
fn a_script_is_stopped_at_its_time_limit() {
    let mut wb = stock();
    let timed = |wb: &mut Workbook, source: &str| {
        let started = Instant::now();
        let (error, _) = fails_with(wb, source, &quick());
        assert_eq!(error.kind, ErrorKind::Timeout, "{source}: {}", error.message);
        assert!(started.elapsed() < Duration::from_secs(5), "stopped late: {:?}", started.elapsed());
        error
    };
    let error = timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange('A1').setValue('x');\n  while (true) {}\n}");
    // An empty loop has no place of its own: the line is the loop's or
    // the one before.
    assert!(matches!(error.line, Some(2 | 3)), "{:?}", error.line);
    assert_eq!(error.message, "The script was stopped: it ran longer than 0.3 seconds.");
    // The script cannot catch it.
    timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  for (;;) {\n    try {\n      while (true) {}\n    } catch (e) {}\n  }\n}");
    // After an await, in a callback, in calls of the model, in a regular expression.
    timed(&mut wb, "async function main(workbook: ExcelScript.Workbook) {\n  await null;\n  for (;;) {}\n}");
    timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  [1].forEach(() => { for (;;) {} });\n}");
    timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  const sheet = workbook.getActiveWorksheet();\n  let n = 0;\n  while (true) { sheet.getRange('A1').setValue(n++); sheet.getRange('D2').getValue(); }\n}");
    timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  /^(a+)+$/.test('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa!');\n}");
    timed(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  const f = (): void => f();\n  for (;;) { try { f(); } catch (e) {} }\n}");
}

#[test]
fn a_runaway_script_does_not_take_memory_or_the_stack() {
    let mut wb = stock();
    // Endless recursion is an error of the script, which it may catch.
    let out = ok(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  const down = (n: number): number => down(n + 1) + 1;\n  try {\n    down(0);\n    return 'no';\n  } catch (error) {\n    return (error as Error).name;\n  }\n}",
    );
    assert_eq!(out.returned.as_deref(), Some("\"RangeError\""));
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  const down = (n: number): number => down(n + 1) + 1;\n  down(0);\n}");
    assert_eq!((error.kind, error.line), (ErrorKind::Runtime, Some(2)));

    // Memory: the script hits its own ceiling long before the machine's.
    let limits = Context {
        time_limit: Duration::from_secs(20),
        ..Context::default()
    };
    let started = Instant::now();
    let (error, _) = fails_with(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  const hoard: number[][] = [];\n  while (true) {\n    hoard.push(new Array(1000000).fill(1));\n  }\n}",
        &limits,
    );
    assert_eq!(error.kind, ErrorKind::Runtime);
    assert!(error.message.contains("out of memory"), "{}", error.message);
    assert!(started.elapsed() < Duration::from_secs(15));
    let (error, _) = fails_with(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  let text = 'x';\n  while (true) {\n    text += text;\n  }\n}",
        &limits,
    );
    assert_eq!(error.kind, ErrorKind::Runtime);

    // The workbook: no array of a whole sheet, no endless sheets or logs.
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange().getValues();\n}");
    assert!(error.message.contains("too large"), "{}", error.message);
    let error = fails(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().getRange('A:XFD').setValue(1);\n}");
    assert!(error.message.contains("too large"), "{}", error.message);
    let (error, _) = fails_with(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  while (true) workbook.addWorksheet();\n}", &limits);
    assert!(error.message.contains("more than 5000 worksheets"), "{}", error.message);
    let (error, out) = fails_with(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  while (true) console.log('again');\n}", &quick());
    assert_eq!(error.kind, ErrorKind::Timeout);
    assert_eq!(out.logs.len(), 5001);
    assert!(out.logs[5000].text.contains("The log is full"));
}

#[test]
fn a_script_has_the_language_and_the_workbook_and_nothing_else() {
    let mut wb = stock();
    let out = ok(
        &mut wb,
        r#"function main(workbook: ExcelScript.Workbook) {
  const g = globalThis as Record<string, unknown>;
  const absent = ["process", "require", "std", "os", "Deno", "Bun", "window", "document", "navigator", "localStorage",
    "XMLHttpRequest", "WebSocket", "Worker", "importScripts", "scriptArgs", "print", "__op", "op", "logNative"]
    .filter(name => { try { return typeof g[name] === "function" ? (g[name] as () => void)() !== undefined : g[name] !== undefined; } catch (e) { return false; } });
  // The model cannot be replaced or reached around.
  const fixed = ["ExcelScript", "console", "fetch"].filter(name => { try { g[name] = 1; } catch (e) {} return g[name] !== 1; });
  try { (ExcelScript.ClearApplyTo as Record<string, string>).contents = "All"; } catch (e) {}
  const proto = Object.getPrototypeOf(workbook.getActiveWorksheet());
  return { absent, fixed, contents: ExcelScript.ClearApplyTo.contents, keys: Object.keys(workbook), text: String(workbook.getActiveWorksheet()), json: JSON.stringify(proto) };
}"#,
    );
    assert_eq!(
        out.returned.as_deref(),
        Some(r#"{"absent":[],"fixed":["ExcelScript","console","fetch"],"contents":"Contents","keys":["_s"],"text":"[object Worksheet]","json":"{\"$\":3}"}"#)
    );
    // Modules are not loaded.
    let error = fails(&mut wb, "import * as fs from 'fs';\nfunction main(workbook: ExcelScript.Workbook) {\n  fs.readFileSync('/etc/passwd');\n}");
    assert_eq!(error.kind, ErrorKind::Syntax);
    let error = fails(&mut wb, "async function main(workbook: ExcelScript.Workbook) {\n  const fs = await import('fs');\n  return fs;\n}");
    assert_eq!(error.kind, ErrorKind::Runtime);
}

#[test]
fn what_a_run_reports() {
    let mut wb = stock();
    wb.add_sheet(1);
    wb.commit("sheet");
    wb.clear_history();
    let context = Context {
        active_sheet: 1,
        selection: Rect::new(2, 1, 4, 3),
        active_cell: (2, 1),
        ..Context::default()
    };
    let out = ok_with(
        &mut wb,
        r#"function main(workbook: ExcelScript.Workbook) {
  console.log("start", 1, true, null, undefined);
  console.warn([1, "two", [3, 4], { five: 5 }], { name: "Pen", tags: ["a"], when: undefined });
  console.error(new Error("careful"), workbook.getActiveWorksheet());
  console.log(workbook.getActiveWorksheet().getName(), workbook.getSelectedRange().getAddress(), workbook.getActiveCell().getAddress());
  console.log(new Map([["k", 1]]), new Set([1]), () => 1, 12.50, -0, "", [undefined, null]);
  console.log(workbook.getFirstWorksheet().getRange("A1:B2").getValues());
  console.log();
}"#,
        &context,
    );
    let logs: Vec<(LogLevel, &str, Option<u32>)> = out.logs.iter().map(|l| (l.level, l.text.as_str(), l.line)).collect();
    assert_eq!(
        logs,
        [
            (LogLevel::Log, "start 1 true null undefined", Some(2)),
            (LogLevel::Warn, r#"(4) [1, "two", (2) [3, 4], {five: 5}] {name: "Pen", tags: (1) ["a"], when: undefined}"#, Some(3)),
            (LogLevel::Error, "Error: careful Worksheet {}", Some(4)),
            (LogLevel::Log, "Sheet2 Sheet2!B3:D5 Sheet2!B3", Some(5)),
            (LogLevel::Log, r#"Map(1) {"k" => 1} Set(1) {1} ƒ anonymous() 12.5 0  (2) [undefined, null]"#, Some(6)),
            (LogLevel::Log, r#"(2) [(2) ["Name", "Qty"], (2) ["Pen", 10]]"#, Some(7)),
            (LogLevel::Log, "", Some(8)),
        ]
    );
    assert_eq!((out.returned, out.activated, out.selected), (None, None, None));

    let out = ok_with(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook): number[] {\n  const first = workbook.getFirstWorksheet();\n  first.activate();\n  first.getRange('B2:C3').select();\n  return [1.5, 2];\n}",
        &context,
    );
    assert_eq!((out.returned.as_deref(), out.activated, out.selected), (Some("[1.5,2]"), Some(0), Some((0, Rect::new(1, 1, 2, 2)))));
    // Deleting the sheet in front brings the next one in front.
    let out = ok_with(&mut wb, "function main(workbook: ExcelScript.Workbook) {\n  workbook.getActiveWorksheet().delete();\n  return workbook.getActiveWorksheet().getName();\n}", &context);
    assert_eq!((out.returned.as_deref(), out.activated), (Some("\"Sheet1\""), Some(0)));
}

#[test]
fn a_protected_sheet_refuses_as_it_does_by_hand() {
    let mut wb = stock();
    wb.update_style(0, Rect::new(1, 1, 3, 1), &|style| style.locked = false);
    wb.commit("unlock");
    wb.protect_sheet(0, SheetProtection::allowing(&[Permission::FormatColumns]), "");
    let refused = |wb: &mut Workbook, body: &str| {
        let error = fails(wb, &format!("function main(workbook: ExcelScript.Workbook) {{\n  const sheet = workbook.getActiveWorksheet();\n  sheet.getRange('B2').setValue(99);\n  {body}\n}}"));
        assert_eq!((error.kind, error.line), (ErrorKind::Runtime, Some(4)), "{body}: {}", error.message);
        assert!(error.message.contains("protected"), "{body}: {}", error.message);
    };
    refused(&mut wb, "sheet.getRange('A2').setValue('x');");
    refused(&mut wb, "sheet.getRange('A2:B2').setValues([['x', 1]]);");
    refused(&mut wb, "sheet.getRange('A2').setFormula('=1');");
    refused(&mut wb, "sheet.getRange('A2').clear();");
    refused(&mut wb, "sheet.getRange('B2').getFormat().getFont().setBold(true);");
    refused(&mut wb, "sheet.getRange('B2').setNumberFormat('0.00');");
    refused(&mut wb, "sheet.getRange('2:2').delete(ExcelScript.DeleteShiftDirection.up);");
    refused(&mut wb, "sheet.getRange('2:2').insert(ExcelScript.InsertShiftDirection.down);");
    refused(&mut wb, "sheet.getRange('B2').insert(ExcelScript.InsertShiftDirection.down);");
    refused(&mut wb, "sheet.getRange('A1:C4').getSort().apply([{ key: 0 }]);");
    refused(&mut wb, "sheet.getRange('E1').copyFrom('A1');");
    refused(&mut wb, "sheet.getRange('A2').moveTo('F2');");
    refused(&mut wb, "sheet.getRange('B2:B3').merge();");
    refused(&mut wb, "sheet.getRange('B2').getFormat().setRowHeight(30);");
    refused(&mut wb, "sheet.getRange('B2').getFormat().getProtection().setLocked(true);");
    refused(&mut wb, "sheet.addTable('A1:C4', true);");
    refused(&mut wb, "sheet.getAutoFilter().apply('A1:C4');");
    refused(&mut wb, "sheet.setTabColor('red');");
    refused(&mut wb, "sheet.getRange('A1').setHyperlink({ address: 'https://example.org' });");
    refused(&mut wb, "sheet.getRange('A1:A4').replaceAll('Pen', 'Pencil', {});");
    refused(&mut wb, "sheet.getRange('B2:B3').autoFill('B2:B9');");
    // What the protection allows goes through: unlocked cells, column widths.
    ok(
        &mut wb,
        "function main(workbook: ExcelScript.Workbook) {\n  const sheet = workbook.getActiveWorksheet();\n  sheet.getRange('B2:B4').setValues([[1], [2], [3]]);\n  sheet.getRange('B:B').getFormat().setColumnWidth(80);\n  sheet.getRange('E9').getValue();\n}",
    );
    assert_eq!(value(&wb, 0, "B4"), Value::Number(3.0));

    // The structure of the workbook.
    let mut wb = stock();
    wb.protect_workbook(true, false, "");
    for body in ["workbook.addWorksheet();", "sheet.setName('x');", "sheet.delete();", "sheet.copy();", "sheet.setPosition(0); workbook.getActiveWorksheet().setPosition(1);"] {
        let error = fails(&mut wb, &format!("function main(workbook: ExcelScript.Workbook) {{\n  const sheet = workbook.getActiveWorksheet();\n  workbook.getTables();\n  {body}\n}}"));
        assert!(error.message.contains("protected") || error.message.contains("invalid"), "{body}: {}", error.message);
    }
}

#[test]
fn numbers_cross_unchanged() {
    let mut wb = stock();
    ok(
        &mut wb,
        r#"function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const numbers = [0.1 + 0.2, 1e21, -1e-7, 123456789012345.67, 2 ** 53, 1 / 3, -0.5, 4.35 * 100];
  const cells = sheet.getRange("G1:G8");
  cells.setValues(numbers.map(n => [n]));
  cells.getValues().forEach((row, i) => { if (row[0] !== numbers[i]) throw new Error(`${numbers[i]} came back as ${row[0]}`); });
  sheet.getRange("H1").setFormula("=G1=0.30000000000000004");
  eq(sheet.getRange("H1").getValue(), true);
  const grid: (string | number | boolean)[][] = [];
  for (let r = 0; r < 200; r++) {
    grid.push(Array.from({ length: 50 }, (_, c) => (c % 3 === 0 ? `r${r}c${c}` : r * c + 0.5)));
  }
  sheet.getRange("J1:BG200").setValues(grid);
  eq(sheet.getRange("J1:BG200").getValues(), grid);
  eq(sheet.getRange("K200").getValue(), 199.5);
}"#,
    );
    assert_eq!(number(&wb, "G1"), 0.1 + 0.2);
    assert_eq!(number(&wb, "G4"), 123456789012345.67);
}
