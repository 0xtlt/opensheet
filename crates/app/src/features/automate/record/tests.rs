//! The recorder proved by replay: each test records real commands of the
//! window, stops, undoes everything the commands did, runs the script the
//! recorder wrote through the real script engine, and compares the
//! workbook with the one the commands gave.

use super::super::pane::{CodeEditorPane, Page};
use super::super::run::{OutputKind, Runs};
use super::super::{Cmd, pane as the_pane};
use super::differences;
use crate::app::ExcelApp;
use opensheet_engine::ops::PasteMode;
use gpui::{Context, Entity, Modifiers, TestAppContext, VisualTestContext, Window, point, px};
use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::{Rect, Workbook};

pub(super) fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    settle(cx);
    (app, cx)
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("{name} is not a range"))
}

/// One action of the user: a command of the window, after which the
/// window says it changed.
pub(super) fn act(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, command: impl FnOnce(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>)) {
    app.update_in(cx, command);
    cx.run_until_parked();
}

/// Select a range, as a click and a drag do.
pub(super) fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    let range = rect(range);
    act(app, cx, |app, _, cx| app.select_rect(range, cx));
}

/// Type something in a cell and press Return.
pub(super) fn enter(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: &str, text: &str) {
    let (row, col) = parse_cell_name(cell).expect("a cell");
    act(app, cx, |app, window, cx| {
        app.select_cell(row, col, cx);
        window.focus(&app.grid_focus);
    });
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

/// Fill a block with numbers and a header row, without recording: what a
/// test starts from.
pub(super) fn seed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, rows: &[&[&str]]) {
    act(app, cx, |app, _, cx| {
        for (row, cells) in rows.iter().enumerate() {
            for (col, text) in cells.iter().enumerate() {
                app.wb.set_cell_text(app.sheet, row as u32, col as u32, text).expect("a value");
            }
        }
        app.commit("seed", cx);
    });
}

fn pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<CodeEditorPane> {
    app.update(cx, |app, _| the_pane(app).expect("the Code Editor pane"))
}

pub(super) fn start(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    act(app, cx, |app, window, cx| app.run_command(Cmd::RecordActions, window, cx));
    assert_eq!(pane(app, cx).update(cx, |pane, _| pane.page.clone()), Page::Recording);
}

/// What the pane lists: each action with whether it was recorded.
pub(super) fn listed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    pane(app, cx).update(cx, |pane, _| pane.recorder.as_ref().expect("recording").actions.iter().map(|action| (action.label.clone(), action.recorded)).collect())
}

/// Stop recording; the script the recorder wrote, as the editor shows it.
pub(super) fn stop(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    let pane = pane(app, cx);
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.stop_recording(window, cx)));
    cx.run_until_parked();
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Editor);
    pane.update(cx, |pane, cx| pane.editor.read(cx).text().to_string())
}

/// Record what `actions` does, then prove the script: everything is
/// undone, the script runs, and the workbook must be the one the actions
/// gave. Every action must have been recorded. Returns the script.
pub(super) fn replayed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, actions: impl FnOnce(&Entity<ExcelApp>, &mut VisualTestContext)) -> String {
    let depth = app.update(cx, |app, _| app.wb.history_depth().0);
    start(app, cx);
    actions(app, cx);
    let actions = listed(app, cx);
    let script = stop(app, cx);
    assert!(actions.iter().all(|(_, recorded)| *recorded), "not recorded: {actions:?}\n{script}");
    assert!(!actions.is_empty(), "nothing was recorded");
    let wanted: Workbook = app.update(cx, |app, _| app.wb.fork());
    // Back to where the recording started, on the sheet it started on.
    act(app, cx, |app, window, cx| {
        while app.wb.history_depth().0 > depth {
            app.undo(cx);
        }
        app.switch_sheet(0, window, cx);
        app.select_cell(0, 0, cx);
    });
    let pane = pane(app, cx);
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.run(window, cx)));
    cx.run_until_parked();
    let errors: Vec<String> = app.update(cx, |app, _| {
        app.ext::<Runs>().map(|runs| runs.output.iter().filter(|line| line.kind == OutputKind::Error).map(|line| line.text.clone()).collect()).unwrap_or_default()
    });
    assert!(errors.is_empty(), "{errors:?}\n{script}");
    let left = app.update(cx, |app, _| differences(&app.wb, &wanted));
    assert!(left.is_empty(), "the replay differs: {left:#?}\n{script}");
    script
}

/// The statements of a script, without the comments and what every
/// script starts with.
pub(super) fn statements(script: &str) -> Vec<&str> {
    script.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("function") && !line.starts_with("let selectedSheet") && *line != "}").collect()
}

const SALES: &[&[&str]] = &[&["Region", "Units", "Price"], &["North", "12", "3.5"], &["South", "7", "4"], &["East", "30", "2.25"], &["West", "18", "5"]];

// ----------------------------------------------------------------- typing

#[gpui::test]
fn typing_values_and_formulas(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let script = replayed(&app, cx, |app, cx| {
        enter(app, cx, "A1", "Total");
        enter(app, cx, "B1", "12.5");
        enter(app, cx, "B2", "30");
        enter(app, cx, "B3", "=SUM(B1:B2)*2");
        enter(app, cx, "C1", "TRUE");
        enter(app, cx, "C2", "'007");
        enter(app, cx, "C3", "He said \"hi\"");
        enter(app, cx, "D1", "25%");
        enter(app, cx, "D2", "3/14/2024");
    });
    assert_eq!(
        statements(&script)[..4],
        [
            "selectedSheet.getRange(\"A1\").setValue(\"Total\");",
            "selectedSheet.getRange(\"B1\").setValue(12.5);",
            "selectedSheet.getRange(\"B2\").setValue(30);",
            "selectedSheet.getRange(\"B3\").setFormula(\"=SUM(B1:B2)*2\");",
        ]
    );
    assert!(script.contains("selectedSheet.getRange(\"C2\").setValue(\"'007\");"), "{script}");
    assert!(script.starts_with("function main(workbook: ExcelScript.Workbook) {\n\tlet selectedSheet = workbook.getActiveWorksheet();\n\t// Set range A1 on selectedSheet\n"), "{script}");
}

#[gpui::test]
fn clearing_contents_formats_and_all(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    act(&app, cx, |app, _, cx| {
        app.select_rect(rect("A1:C1"), cx);
        app.toggle_bold(cx);
        app.set_fill(Some(opensheet_engine::Color(0xFFFF00)), cx);
    });
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B2:B3");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::Contents, cx));
        select(app, cx, "A1:B1");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::Formats, cx));
        select(app, cx, "C1:C2");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::All, cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"B2:B3\").clear(ExcelScript.ClearApplyTo.contents);",
            "selectedSheet.getRange(\"A1:B1\").clear(ExcelScript.ClearApplyTo.formats);",
            "selectedSheet.getRange(\"C1:C2\").clear(ExcelScript.ClearApplyTo.all);",
        ]
    );
}

#[gpui::test]
fn rows_and_columns_inserted_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A2:A3");
        act(app, cx, |app, _, cx| app.insert_rows(cx));
        select(app, cx, "B1");
        act(app, cx, |app, _, cx| app.insert_cols(cx));
        select(app, cx, "A6");
        act(app, cx, |app, _, cx| app.delete_rows(cx));
        select(app, cx, "D1");
        act(app, cx, |app, _, cx| app.delete_cols(cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"2:3\").insert(ExcelScript.InsertShiftDirection.down);",
            "selectedSheet.getRange(\"B:B\").insert(ExcelScript.InsertShiftDirection.right);",
            "selectedSheet.getRange(\"6:6\").delete(ExcelScript.DeleteShiftDirection.up);",
            "selectedSheet.getRange(\"D:D\").delete(ExcelScript.DeleteShiftDirection.left);",
        ]
    );
}

#[gpui::test]
fn paste_from_a_range_of_the_workbook(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    enter(&app, cx, "D2", "=B2*C2");
    act(&app, cx, |app, _, cx| {
        app.select_rect(rect("A1:C1"), cx);
        app.toggle_bold(cx);
    });
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A1:D2");
        act(app, cx, |app, _, cx| app.copy(false, cx));
        select(app, cx, "F1");
        act(app, cx, |app, _, cx| app.paste(PasteMode::All, cx));
        select(app, cx, "F5");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Values, cx));
        select(app, cx, "F9");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Formats, cx));
        select(app, cx, "F12");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Formulas, cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"F1\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.all, false, false);",
            "selectedSheet.getRange(\"F5\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.values, false, false);",
            "selectedSheet.getRange(\"F9\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.formats, false, false);",
            "selectedSheet.getRange(\"F12\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.formulas, false, false);",
        ]
    );
}
