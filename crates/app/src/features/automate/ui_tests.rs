//! The Automate tab, the Code Editor pane and the Macro dialog driven in
//! GPUI's headless window. Scripts run on an engine made for these tests
//! (`run::testing::FakeEngine`); their files are in a temporary folder.

use super::macros::{MacroDialog, MacroUi, VbaViewer};
use super::pane::{Bottom, CodeEditorPane, Page, RenameDialog, ScriptRef};
use super::run::testing::FakeEngine;
use super::run::{OutputKind, Runs};
use super::*;
use gpui::{Entity, Modifiers, Pixels, Point, Render, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::protect::SheetProtection;
use opensheet_engine::vba::VbaProject;
use opensheet_engine::{Rect, Value, Workbook};
use opensheet_io::vba::{ModuleKind, build_project};
use opensheet_script::{LogLevel, Script};
use std::sync::Arc;
use std::time::Duration;

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    run::use_engine(Arc::new(FakeEngine));
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

fn at(cx: &mut VisualTestContext, name: &'static str) -> Point<Pixels> {
    cx.debug_bounds(name).unwrap_or_else(|| panic!("{name} is not drawn")).center()
}

fn click(cx: &mut VisualTestContext, name: &'static str) {
    let position = at(cx, name);
    cx.simulate_click(position, Modifiers::none());
    redraw(cx);
}

/// Click a control of the ribbon, found by the anchor it registers.
fn click_ribbon(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: &'static str) {
    let position = app.update(cx, |app, _| app.anchors.borrow().get(id).map(|bounds| bounds.center())).unwrap_or_else(|| panic!("{id} is not on the ribbon"));
    cx.simulate_click(position, Modifiers::none());
    redraw(cx);
}

fn the_pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<CodeEditorPane> {
    app.update(cx, |app, _| pane(app).expect("the Code Editor pane"))
}

fn dialog<V: Render>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<V> {
    app.update(cx, |app, _| app.front_modal_view().expect("a dialog is open").downcast::<V>().ok().expect("the dialog"))
}

fn modals(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.modals.len())
}

/// Press a button of the message box in front, by its place: Tab walks
/// the buttons from the first one, Space presses.
fn press_alert_button(cx: &mut VisualTestContext, index: usize) {
    for _ in 0..=index {
        cx.simulate_keystrokes("tab");
    }
    cx.simulate_keystrokes("space");
    redraw(cx);
}

fn value(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, sheet: usize, cell: (u32, u32)) -> Value {
    app.update(cx, |app, _| app.wb.value_at(sheet, cell.0, cell.1))
}

/// A script of the user's, written to the folder of the test.
fn script(name: &str, body: &str) {
    scripts::save(&Script {
        body: body.to_string(),
        ..Script::new(name)
    })
    .unwrap();
}

/// Open the editor on a script of the user's.
fn edit(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Entity<CodeEditorPane> {
    let pane = app.update_in(cx, |app, window, cx| {
        let pane = open_pane(app, cx);
        pane.update(cx, |pane, cx| {
            pane.show_script(ScriptRef::Mine(name.to_string()), window, cx);
            pane.edit(window, cx);
        });
        pane
    });
    redraw(cx);
    pane
}

fn editor_text(pane: &Entity<CodeEditorPane>, cx: &mut VisualTestContext) -> String {
    pane.update(cx, |pane, cx| pane.editor.read(cx).text().to_string())
}

/// The text of the editor with `|` at the caret.
fn editor_shown(pane: &Entity<CodeEditorPane>, cx: &mut VisualTestContext) -> String {
    pane.update(cx, |pane, cx| {
        let editor = pane.editor.read(cx);
        let mut text = editor.text().to_string();
        text.insert(editor.buffer().caret(), '|');
        text
    })
}

fn output(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(OutputKind, String, Option<u32>)> {
    app.update(cx, |app, _| {
        app.ext::<Runs>().map(|runs| runs.output.iter().map(|line| (line.kind, line.text.clone(), line.line)).collect()).unwrap_or_default()
    })
}

// ------------------------------------------------------------------ the tab

#[gpui::test]
fn the_tab_shows_new_script_all_scripts_and_the_gallery(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log tidy");
    app.update(cx, |app, cx| show_tab(app, cx));
    redraw(cx);
    // One script of the user's and three samples in the gallery.
    for tile in ["au-tile-0", "au-tile-1", "au-tile-2", "au-tile-3"] {
        assert!(cx.debug_bounds(tile).is_some(), "{tile}");
    }
    assert!(cx.debug_bounds("au-tile-4").is_none());

    // A tile opens the page of its script in the Code Editor.
    click(cx, "au-tile-0");
    let pane = the_pane(&app, cx);
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Mine("Tidy".to_string())));
    assert!(app.update(cx, |app, _| app.pane_open(PANE_ID)));
    click(cx, "au-tile-1");
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Sample(0)));

    // All Scripts shows the list; New Script opens the editor on a new one.
    click_ribbon(&app, cx, "au-all");
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::List);
    assert!(cx.debug_bounds("ce-script-0").is_some() && cx.debug_bounds("ce-sample-5").is_some());
    click_ribbon(&app, cx, "au-new");
    assert_eq!(pane.update(cx, |pane, _| (pane.page.clone(), pane.open.clone())), (Page::Editor, Some("Script".to_string())));
    assert_eq!(editor_text(&pane, cx), Script::new("Script").body);
    assert!(scripts::exists("Script"));
    // The commands are no longer the placeholders of the ribbon.
    assert_eq!(app.update(cx, |app, _| app.status_message.clone()), None);
}

// --------------------------------------------------------------- the editor

#[gpui::test]
fn typing_in_the_editor(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    click_ribbon_new(&app, cx);
    let pane = the_pane(&app, cx);
    // The keys go to the editor, not to the grid.
    pane.update(cx, |pane, cx| pane.editor.update(cx, |editor, cx| editor.set_text("", cx)));
    cx.simulate_input("function main() {");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("let a = [1");
    assert_eq!(editor_shown(&pane, cx), "function main() {\n  let a = [1|]\n}");
    // The closing bracket is stepped over, not doubled.
    cx.simulate_input("];");
    assert_eq!(editor_shown(&pane, cx), "function main() {\n  let a = [1];|\n}");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("log(\"hi");
    assert_eq!(editor_shown(&pane, cx), "function main() {\n  let a = [1];\n  log(\"hi|\")\n}");
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].cells.len()), 0, "nothing was typed into the sheet");

    // Tab and ⇧Tab on a selection of lines.
    cx.simulate_keystrokes("shift-up");
    cx.simulate_keystrokes("tab");
    assert_eq!(editor_text(&pane, cx), "function main() {\n    let a = [1];\n    log(\"hi\")\n}");
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert_eq!(editor_text(&pane, cx), "function main() {\nlet a = [1];\nlog(\"hi\")\n}");

    // The editor has its own undo: the workbook's is not touched.
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(editor_text(&pane, cx), "function main() {\n    let a = [1];\n    log(\"hi\")\n}");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(editor_text(&pane, cx), "function main() {\n  let a = [1];\n  log(\"hi\")\n}");
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), None);

    // ⌘/ comments the line of the caret, ⌘A and typing replace everything.
    cx.simulate_keystrokes("cmd-up cmd-/");
    assert!(editor_text(&pane, cx).starts_with("// function main() {"));
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("x");
    assert_eq!(editor_shown(&pane, cx), "x|");
    // The status line follows the caret.
    assert_eq!(pane.update(cx, |pane, cx| pane.editor.read(cx).caret_position()), (1, 2));
}

fn click_ribbon_new(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update(cx, |app, cx| show_tab(app, cx));
    redraw(cx);
    click_ribbon(app, cx, "au-new");
}

#[gpui::test]
fn saving_and_the_mark_of_unsaved_changes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    click_ribbon_new(&app, cx);
    let pane = the_pane(&app, cx);
    assert!(!pane.update(cx, |pane, cx| pane.unsaved(cx)));
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("// more");
    redraw(cx);
    assert!(pane.update(cx, |pane, cx| pane.unsaved(cx)));
    // The file still holds what was saved.
    assert_eq!(scripts::load("Script").unwrap().body, Script::new("Script").body);

    // ⌘S in the editor saves the script, not the workbook.
    cx.simulate_keystrokes("cmd-s");
    redraw(cx);
    assert!(!pane.update(cx, |pane, cx| pane.unsaved(cx)));
    assert!(scripts::load("Script").unwrap().body.ends_with("// more"));
    assert_eq!(app.update(cx, |app, _| app.path.clone()), None);

    // The Save button does the same.
    cx.simulate_input("!");
    redraw(cx);
    click(cx, "ce-save");
    assert!(!pane.update(cx, |pane, cx| pane.unsaved(cx)));
    assert!(scripts::load("Script").unwrap().body.ends_with("// more!"));
}

#[gpui::test]
fn leaving_a_changed_script_asks_first(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log one");
    let pane = edit(&app, cx, "Tidy");
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" two");
    redraw(cx);

    // Back to the list: the question; Cancel stays in the editor.
    click(cx, "ce-back");
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Editor);
    assert_eq!(editor_text(&pane, cx), "log one two");

    // Don't Save: the list, and the file as it was.
    click(cx, "ce-back");
    press_alert_button(cx, 1);
    assert_eq!(modals(&app, cx), 0);
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::List);
    assert_eq!(scripts::load("Tidy").unwrap().body, "log one");

    // Save (the default): the file changed, then the page asked for.
    let pane = edit(&app, cx, "Tidy");
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" three");
    redraw(cx);
    pane.update_in(cx, |pane, window, cx| pane.show_script(ScriptRef::Sample(1), window, cx));
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(scripts::load("Tidy").unwrap().body, "log one three");
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Sample(1)));
}

#[gpui::test]
fn unsaved_text_outlives_the_pane(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log one");
    edit(&app, cx, "Tidy");
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" two");
    redraw(cx);
    // Another workbook: the panes of the window go.
    app.update(cx, |app, cx| {
        let wb = Workbook::new(app.wb.locale);
        app.set_workbook(wb, None, "Other".to_string(), cx);
    });
    redraw(cx);
    assert!(app.update(cx, |app, _| pane(app).is_none()));
    // The pane that comes next takes the text up, still unsaved.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::AllScripts, window, cx));
    redraw(cx);
    // (The question of the changed script: stay in the editor.)
    cx.simulate_keystrokes("escape");
    redraw(cx);
    let pane = the_pane(&app, cx);
    assert_eq!(pane.update(cx, |pane, cx| (pane.page.clone(), pane.open.clone(), pane.unsaved(cx))), (Page::Editor, Some("Tidy".to_string()), true));
    assert_eq!(editor_text(&pane, cx), "log one two");
    assert_eq!(scripts::load("Tidy").unwrap().body, "log one");
    // Saved, there is nothing left to take up.
    pane.update(cx, |pane, cx| pane.save(cx));
    app.update(cx, |app, cx| {
        let wb = Workbook::new(app.wb.locale);
        app.set_workbook(wb, None, "Third".to_string(), cx);
    });
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::AllScripts, window, cx));
    redraw(cx);
    assert_eq!(the_pane(&app, cx).update(cx, |pane, _| pane.page.clone()), Page::List);
}

#[gpui::test]
fn problems_appear_after_a_pause_and_lead_to_their_line(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log one\nlog two\nlog three");
    let pane = edit(&app, cx, "Tidy");
    assert!(pane.update(cx, |pane, _| pane.problems.is_empty()));
    cx.simulate_keystrokes("cmd-down enter");
    cx.simulate_input("syntax error here");
    redraw(cx);
    // Not while typing…
    assert!(pane.update(cx, |pane, _| pane.problems.is_empty()));
    cx.executor().advance_clock(Duration::from_millis(600));
    redraw(cx);
    // …but once the typing pauses: on its line and in the list.
    let problems = pane.update(cx, |pane, _| pane.problems.iter().map(|p| (p.line, p.message.clone())).collect::<Vec<_>>());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].0, Some(4));
    assert_eq!(pane.update(cx, |pane, _| pane.bottom), Bottom::Problems);

    // A click on the problem puts the caret on its line.
    cx.simulate_keystrokes("cmd-up");
    assert_eq!(pane.update(cx, |pane, cx| pane.editor.read(cx).caret_position()), (1, 1));
    click(cx, "ce-problem-0");
    assert_eq!(pane.update(cx, |pane, cx| pane.editor.read(cx).caret_position().0), 4);
    // Typing goes on there.
    cx.simulate_keystrokes("cmd-shift-right backspace");
    cx.simulate_input("log four");
    cx.executor().advance_clock(Duration::from_millis(600));
    redraw(cx);
    assert!(pane.update(cx, |pane, _| pane.problems.is_empty()));
    assert_eq!(pane.update(cx, |pane, _| pane.bottom), Bottom::Output);
    assert_eq!(editor_text(&pane, cx), "log one\nlog two\nlog three\nlog four");

    // A script opened with a problem shows it at once.
    cx.simulate_keystrokes("cmd-s");
    script("Broken", "syntax error");
    let pane = edit(&app, cx, "Broken");
    assert_eq!(pane.update(cx, |pane, _| (pane.open.clone(), pane.problems.len())), (Some("Broken".to_string()), 1));
}

// ------------------------------------------------------------------ running

#[gpui::test]
fn a_script_runs_as_one_undo_step_and_the_window_follows(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        app.wb.set_cell_text(0, 0, 0, "1").unwrap();
        app.commit("setup", cx);
    });
    script("Report", "set A1 5\nbold A1\nlog hello\nwarn careful\nadd-sheet\nset B2 =Sheet1!A1*2\nselect B2:C3\nreturn 42");
    let pane = edit(&app, cx, "Report");
    click(cx, "ce-run");

    // The values, the format and the new sheet are there.
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(5.0));
    assert!(app.update(cx, |app, _| app.wb.style_at(0, 0, 0).font.bold));
    assert_eq!(app.update(cx, |app, _| app.wb.sheets.len()), 2);
    assert_eq!(value(&app, cx, 1, (1, 1)), Value::Number(10.0));
    // The window went to the sheet the script put in front and to its
    // selection.
    assert_eq!(app.update(cx, |app, _| (app.sheet, app.sel().ranges.clone())), (1, vec![Rect::new(1, 1, 2, 2)]));

    // The output: the logs with their level and line, the value returned.
    let lines = output(&app, cx);
    assert_eq!(lines[0], (OutputKind::Log(LogLevel::Log), "hello".to_string(), Some(3)));
    assert_eq!(lines[1], (OutputKind::Log(LogLevel::Warn), "careful".to_string(), Some(4)));
    assert!(lines[2].1.ends_with("42") && lines[2].0 == OutputKind::Note);
    assert_eq!(lines.len(), 4);
    assert_eq!(pane.update(cx, |pane, _| pane.bottom), Bottom::Output);
    assert!(cx.debug_bounds("ce-output-0").is_some());

    // One step, named as Excel names the command; undo takes all back.
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), Some("Run Script".to_string()));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(1.0));
    assert_eq!(app.update(cx, |app, _| (app.wb.sheets.len(), app.wb.style_at(0, 0, 0).font.bold)), (1, false));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), Some("setup".to_string()));
    app.update(cx, |app, cx| app.redo(cx));
    assert_eq!(value(&app, cx, 1, (1, 1)), Value::Number(10.0));
}

#[gpui::test]
fn a_script_that_fails_changes_nothing(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        app.wb.set_cell_text(0, 0, 0, "1").unwrap();
        app.commit("setup", cx);
    });
    script("Broken", "log before\nset A1 5\nadd-sheet\nthrow Boom");
    let pane = edit(&app, cx, "Broken");
    cx.simulate_keystrokes("cmd-up");
    click(cx, "ce-run");
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(1.0));
    assert_eq!(app.update(cx, |app, _| app.wb.sheets.len()), 1);
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), Some("setup".to_string()));
    // What it logged before failing, then the error with its line.
    let lines = output(&app, cx);
    assert_eq!(lines[0], (OutputKind::Log(LogLevel::Log), "before".to_string(), Some(1)));
    assert_eq!(lines[1], (OutputKind::Error, "Boom".to_string(), Some(4)));
    assert_eq!(lines.len(), 2);
    // The caret is on the line (and the column) of the error.
    assert_eq!(pane.update(cx, |pane, cx| pane.editor.read(cx).caret_position()), (4, 3));
    // A click on a line of the output goes to its line of the script.
    click(cx, "ce-output-0");
    assert_eq!(pane.update(cx, |pane, cx| pane.editor.read(cx).caret_position().0), 1);

    // A script that runs too long says so in plain words.
    script("Slow", "timeout");
    edit(&app, cx, "Slow");
    click(cx, "ce-run");
    let lines = output(&app, cx);
    assert_eq!((lines.len(), lines[0].0), (1, OutputKind::Error));
    assert!(lines[0].1.contains("too long"));
}

#[gpui::test]
fn a_script_cannot_be_started_twice_and_can_be_stopped(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Slow", "set A1 5");
    let pane = edit(&app, cx, "Slow");
    // Started: until the other thread is given time, it runs.
    pane.update_in(cx, |pane, window, cx| pane.run(window, cx));
    assert!(app.update(cx, |app, _| run::is_running(app)));
    assert_eq!(app.update(cx, |app, _| app.ext::<Runs>().and_then(|runs| runs.running.as_ref().map(|r| r.name.clone()))), Some("Slow".to_string()));
    // A second start is refused.
    assert!(!app.update_in(cx, |app, window, cx| run::start(app, "Other", "set B1 1", window, cx)));
    // The pane says so and offers to stop, instead of Run. (Drawn by
    // hand: an event would give the other thread its time.)
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear();
    });
    assert!(cx.debug_bounds("ce-running").is_some() && cx.debug_bounds("ce-stop").is_some());

    // Stopped: what the script does afterwards is dropped.
    pane.update(cx, |pane, cx| pane.stop(cx));
    assert!(!app.update(cx, |app, _| run::is_running(app)));
    redraw(cx);
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Empty);
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), None);
    let lines = output(&app, cx);
    assert_eq!((lines.len(), lines[0].0), (1, OutputKind::Note));

    // It can be run again, to its end this time.
    click(cx, "ce-run");
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(5.0));
}

#[gpui::test]
fn what_the_user_changes_meanwhile_wins(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Slow", "set A1 5");
    let pane = edit(&app, cx, "Slow");
    pane.update_in(cx, |pane, window, cx| pane.run(window, cx));
    // While it runs, a cell is typed.
    app.update(cx, |app, cx| {
        app.wb.set_cell_text(0, 3, 3, "mine").unwrap();
        app.commit("typing", cx);
    });
    redraw(cx);
    // The result of the script is refused, the typed cell is there.
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Empty);
    assert_eq!(value(&app, cx, 0, (3, 3)), Value::Text("mine".into()));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), Some("typing".to_string()));
    let lines = output(&app, cx);
    assert_eq!(lines.last().map(|line| line.0), Some(OutputKind::Error));
    assert!(!app.update(cx, |app, _| run::is_running(app)));
}

#[gpui::test]
fn a_protected_sheet_is_not_changed_by_a_script(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        app.wb.set_cell_text(0, 0, 0, "1").unwrap();
        app.wb.snapshot();
        app.wb.sheets[0].protection = Some(SheetProtection::default());
        app.commit("protect", cx);
    });
    script("Write", "set A1 5");
    edit(&app, cx, "Write");
    click(cx, "ce-run");
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(1.0));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)), Some("protect".to_string()));
    let lines = output(&app, cx);
    assert_eq!(lines.last().map(|line| line.0), Some(OutputKind::Error));
    assert!(lines.last().unwrap().1.contains("protected"));

    // A script that only reads and logs runs on it.
    script("Read", "log fine");
    edit(&app, cx, "Read");
    click(cx, "ce-run");
    assert_eq!(output(&app, cx)[0].1, "fine");
    assert_eq!(output(&app, cx).last().map(|line| line.0), Some(OutputKind::Note));
}

#[gpui::test]
fn a_script_runs_from_its_page_and_what_is_typed_in_a_cell_is_stored_first(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Fill", "set B1 7\nlog done");
    app.update_in(cx, |app, window, cx| {
        let pane = open_pane(app, cx);
        pane.update(cx, |pane, cx| pane.show_script(ScriptRef::Mine("Fill".to_string()), window, cx));
    });
    redraw(cx);
    // An entry is being typed in A1.
    app.update_in(cx, |app, window, cx| {
        app.select_cell(0, 0, cx);
        window.focus(&app.grid_focus);
    });
    cx.simulate_input("12");
    assert!(app.update(cx, |app, _| app.is_editing()));
    click(cx, "ce-run");
    assert!(!app.update(cx, |app, _| app.is_editing()));
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(12.0));
    assert_eq!(value(&app, cx, 0, (0, 1)), Value::Number(7.0));
    // The page shows the output of its script.
    assert!(cx.debug_bounds("ce-output-0").is_some());
    // Two steps: the entry, then the script.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!((value(&app, cx, 0, (0, 0)), value(&app, cx, 0, (0, 1))), (Value::Number(12.0), Value::Empty));
}

// -------------------------------------------------------- scripts as files

#[gpui::test]
fn rename_delete_and_make_a_copy(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log one");
    script("Other", "log two");
    let pane = app.update_in(cx, |app, window, cx| {
        let pane = open_pane(app, cx);
        pane.update(cx, |pane, cx| pane.show_script(ScriptRef::Mine("Tidy".to_string()), window, cx));
        pane
    });
    redraw(cx);
    // The "…" menu: Rename, Delete, Make a Copy, then Export.
    let labels = pane.update(cx, |pane, cx| {
        pane.menu_entries(&ScriptRef::Mine("Tidy".to_string()), cx).iter().map(|e| (e.label.to_string(), e.disabled)).collect::<Vec<_>>()
    });
    assert_eq!(
        labels,
        [("Rename".to_string(), false), ("Delete".to_string(), false), ("Make a Copy".to_string(), false), (String::new(), false), ("Export…".to_string(), false)]
    );
    click(cx, "ce-more");
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    cx.simulate_keystrokes("escape");
    app.update(cx, |app, _| app.popup = None);

    // Rename: a name that is taken is refused in the dialog.
    pane.update_in(cx, |pane, window, cx| pane.ask_rename("Tidy", window, cx));
    redraw(cx);
    let rename = dialog::<RenameDialog>(&app, cx);
    rename.update(cx, |dialog, cx| dialog.name.update(cx, |input, cx| input.set_text("other", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    assert!(rename.update(cx, |dialog, _| dialog.problem.is_some()));
    assert!(scripts::exists("Tidy"));
    // A free one is taken: the file is renamed and the page follows.
    rename.update(cx, |dialog, cx| dialog.name.update(cx, |input, cx| input.set_text("Clean up", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    assert!(!scripts::exists("Tidy") && scripts::exists("Clean up"));
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Mine("Clean up".to_string())));
    // Escape leaves the name alone.
    pane.update_in(cx, |pane, window, cx| pane.ask_rename("Clean up", window, cx));
    redraw(cx);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(modals(&app, cx) == 0 && scripts::exists("Clean up"));

    // Make a copy: a second file, shown on its page.
    pane.update(cx, |pane, cx| pane.make_copy(&ScriptRef::Mine("Clean up".to_string()), cx));
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Mine("Clean up - Copy".to_string())));
    assert_eq!(scripts::load("Clean up - Copy").unwrap().body, "log one");

    // Delete asks; Cancel keeps the script, Delete removes its file.
    pane.update(cx, |pane, cx| pane.ask_delete("Clean up", cx));
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(scripts::exists("Clean up"));
    pane.update(cx, |pane, cx| pane.ask_delete("Clean up", cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!scripts::exists("Clean up"));
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::List);
    assert_eq!(scripts::list().len(), 2);
}

#[gpui::test]
fn a_renamed_or_deleted_script_leaves_the_editor_in_step(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    script("Tidy", "log one");
    let pane = edit(&app, cx, "Tidy");
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("!");
    // Renamed while it is edited: the editor goes on, under the new name.
    assert!(pane.update(cx, |pane, cx| pane.rename("Tidy", "Neat", cx)));
    assert_eq!(pane.update(cx, |pane, _| pane.open.clone()), Some("Neat".to_string()));
    cx.simulate_keystrokes("cmd-s");
    redraw(cx);
    assert_eq!(scripts::load("Neat").unwrap().body, "log one!");
    assert!(!scripts::exists("Tidy"));
    // Deleted: nothing is left open, nothing to ask about.
    pane.update(cx, |pane, cx| pane.delete("Neat", cx));
    assert_eq!(pane.update(cx, |pane, cx| (pane.page.clone(), pane.open.clone(), pane.unsaved(cx))), (Page::List, None, false));
}

#[gpui::test]
fn scripts_are_imported_and_exported(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let outside = scripts::default_folder().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let file = outside.join("From Excel.osts");
    std::fs::write(&file, Script { body: "log imported".to_string(), ..Script::new("x") }.to_osts()).unwrap();
    let pane = app.update(cx, |app, cx| open_pane(app, cx));
    pane.update(cx, |pane, cx| pane.import(&file, cx));
    redraw(cx);
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Mine("From Excel".to_string())));
    assert_eq!(scripts::load("From Excel").unwrap().body, "log imported");
    // What is not a script is refused, in words.
    let bad = outside.join("bad.osts");
    std::fs::write(&bad, "nothing").unwrap();
    pane.update(cx, |pane, cx| pane.import(&bad, cx));
    assert!(pane.update(cx, |pane, _| pane.trouble.is_some()));

    // Export writes the text being edited, saved or not.
    edit(&app, cx, "From Excel");
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input(" twice");
    let target = outside.join("out.osts");
    pane.update(cx, |pane, cx| pane.export(&ScriptRef::Mine("From Excel".to_string()), &target, cx));
    let back = Script::from_osts("out", &std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(back.body, "log imported twice");
    // A sample can be exported too.
    let sample = outside.join("sample.osts");
    pane.update(cx, |pane, cx| pane.export(&ScriptRef::Sample(0), &sample, cx));
    assert!(Script::from_osts("sample", &std::fs::read_to_string(&sample).unwrap()).unwrap().body.contains("getUsedRange"));
}

#[gpui::test]
fn samples_are_listed_run_and_copied_to_be_edited(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::AllScripts, window, cx));
    redraw(cx);
    let pane = the_pane(&app, cx);
    // No script of the user's yet, the six samples.
    assert!(cx.debug_bounds("ce-script-0").is_none());
    assert!(cx.debug_bounds("ce-sample-0").is_some() && cx.debug_bounds("ce-sample-5").is_some());
    click(cx, "ce-sample-3");
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Script(ScriptRef::Sample(3)));
    // A sample cannot be renamed or deleted.
    let disabled = pane.update(cx, |pane, cx| pane.menu_entries(&ScriptRef::Sample(3), cx).iter().map(|e| e.disabled).collect::<Vec<_>>());
    assert_eq!(disabled, [true, true, false, false, false]);
    // It runs from its page.
    click(cx, "ce-run");
    assert_eq!(app.update(cx, |app, _| app.ext::<Runs>().map(|runs| runs.output_of.clone())), Some(SAMPLES[3].name().to_string()));
    // Edit copies it among the user's scripts and opens the copy.
    click(cx, "ce-edit");
    let name = SAMPLES[3].name().to_string();
    assert_eq!(pane.update(cx, |pane, _| (pane.page.clone(), pane.open.clone())), (Page::Editor, Some(name.clone())));
    assert_eq!(scripts::load(&name).unwrap().body, SAMPLES[3].body);
    assert_eq!(editor_text(&pane, cx), SAMPLES[3].body);
    // Back to the list (nothing changed, nothing asked): the copy is there.
    click(cx, "ce-back");
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::List);
    assert!(cx.debug_bounds("ce-script-0").is_some());
    // Every sample has a name and a description in every language.
    for lang in crate::i18n::Lang::ALL {
        crate::i18n::set_lang(lang);
        for sample in &SAMPLES {
            assert!(!sample.name().is_empty() && !sample.description().is_empty());
        }
        let mut names: Vec<&str> = SAMPLES.iter().map(|sample| sample.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SAMPLES.len(), "{lang:?}");
    }
}

// -------------------------------------------------------------------- macros

const MODULE: &str = "Option Explicit\n\nSub FormatReport()\n    Range(\"A1\").Font.Bold = True\nEnd Sub\n\nSub ClearReport()\n    Range(\"A2:C9\").ClearContents\nEnd Sub\n";

/// Put a workbook with a VBA project in the window, as File ▸ Open does.
fn with_macros(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u8> {
    let project = build_project(&[
        ("ThisWorkbook", ModuleKind::Document, ""),
        ("Sheet1", ModuleKind::Document, "Sub Refresh()\n    Me.Calculate\nEnd Sub\n"),
        ("Module1", ModuleKind::Standard, MODULE),
    ]);
    let bin = project.clone();
    app.update(cx, |app, cx| {
        let mut wb = Workbook::new(app.wb.locale);
        wb.set_cell_text(0, 0, 0, "7").unwrap();
        wb.commit("x");
        wb.clear_history();
        wb.modified = false;
        wb.vba.project = Some(VbaProject {
            bin: bin.into(),
            signatures: Vec::new(),
        });
        wb.vba.workbook_code_name = Some("ThisWorkbook".to_string());
        wb.sheets[0].code_name = Some("Sheet1".to_string());
        app.set_workbook(wb, None, "Budget".to_string(), cx);
    });
    redraw(cx);
    project
}

#[gpui::test]
fn a_workbook_with_macros_says_so_once(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // A plain workbook has no bar.
    assert!(cx.debug_bounds("macro-bar").is_none());
    assert!(app.update(cx, |app, cx| message_bar(app, cx).is_none()));
    with_macros(&app, cx);
    assert!(cx.debug_bounds("macro-bar").is_some());
    // Its button opens the Macro dialog.
    click(cx, "macro-bar-view");
    assert_eq!(modals(&app, cx), 1);
    dialog::<MacroDialog>(&app, cx);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // Closed, it stays closed for this workbook…
    assert!(app.update(cx, |app, cx| message_bar(app, cx).is_some()));
    click(cx, "macro-bar-close");
    assert!(app.update(cx, |app, _| app.ext::<MacroUi>().is_some_and(|ui| ui.bar_closed)));
    app.update(cx, |app, cx| app.select_cell(3, 3, cx));
    redraw(cx);
    assert!(app.update(cx, |app, cx| message_bar(app, cx).is_none()));
    // …and the next workbook with macros shows its own.
    with_macros(&app, cx);
    assert!(app.update(cx, |app, cx| message_bar(app, cx).is_some()));
    // Opening such a workbook changed nothing in it.
    assert!(!app.update(cx, |app, _| app.wb.modified));
}

#[gpui::test]
fn the_macro_dialog_lists_and_shows_macros_and_does_not_run_them(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_macros(&app, cx);
    // ⌥F8, as in Excel.
    cx.simulate_keystrokes("alt-f8");
    redraw(cx);
    let view = dialog::<MacroDialog>(&app, cx);
    let names = view.update(cx, |view, _| view.macros.iter().map(|m| m.name.clone()).collect::<Vec<_>>());
    assert_eq!(names, ["ClearReport", "FormatReport", "Sheet1.Refresh"]);
    // The first macro is proposed.
    assert_eq!(view.update(cx, |view, cx| (view.name.read(cx).text().to_string(), view.selected(cx))), ("ClearReport".to_string(), Some(0)));

    // Run and Step Into: the one message, and nothing ran.
    click(cx, "macro-run");
    assert_eq!(modals(&app, cx), 2);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    click(cx, "macro-step");
    assert_eq!(modals(&app, cx), 2);
    // Its other button shows the Automate tab.
    press_alert_button(cx, 0);
    assert_eq!(app.update(cx, |app, _| (app.modals.len(), app.ribbon_tab)), (1, TAB));
    assert_eq!(value(&app, cx, 0, (0, 0)), Value::Number(7.0));
    assert!(!app.update(cx, |app, _| app.wb.modified));

    // What would change the project does nothing.
    for disabled in ["macro-create", "macro-delete", "macro-options"] {
        click(cx, disabled);
        assert_eq!(modals(&app, cx), 1, "{disabled}");
    }
    // A name that is no macro leaves nothing to run or edit.
    view.update(cx, |view, cx| view.name.update(cx, |input, cx| input.set_text("Nothing", cx)));
    redraw(cx);
    assert_eq!(view.update(cx, |view, cx| view.selected(cx)), None);
    click(cx, "macro-run");
    click(cx, "macro-edit");
    assert_eq!(modals(&app, cx), 1);
    assert!(dialog::<MacroDialog>(&app, cx) == view);

    // Edit opens the module of the macro, the caret on the macro.
    view.update(cx, |view, cx| view.select(1, cx));
    redraw(cx);
    click(cx, "macro-edit");
    assert_eq!(modals(&app, cx), 1);
    let viewer = dialog::<VbaViewer>(&app, cx);
    assert_eq!(viewer.update(cx, |viewer, _| viewer.module), 2);
    assert_eq!(viewer.update(cx, |viewer, cx| viewer.editor.read(cx).text().to_string()), MODULE);
    assert_eq!(viewer.update(cx, |viewer, cx| viewer.editor.read(cx).caret_position().0), 3);
    // The code can be looked at, not changed.
    let code = at(cx, "vba-code");
    cx.simulate_click(code, Modifiers::none());
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter cmd-v");
    assert_eq!(viewer.update(cx, |viewer, cx| viewer.editor.read(cx).text().to_string()), MODULE);
    // The other modules of the project are a click away.
    viewer.update(cx, |viewer, cx| viewer.show(1, cx));
    redraw(cx);
    assert!(viewer.update(cx, |viewer, cx| viewer.editor.read(cx).text().to_string()).starts_with("Sub Refresh()"));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    assert!(!app.update(cx, |app, _| app.wb.modified));
}

#[gpui::test]
fn the_macro_dialog_of_a_workbook_without_macros_is_empty(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // View ▸ Macros.
    app.update(cx, |app, cx| {
        app.ribbon_tab = 8;
        cx.notify();
    });
    redraw(cx);
    click_ribbon(&app, cx, "vw-macros");
    let view = dialog::<MacroDialog>(&app, cx);
    assert!(view.update(cx, |view, cx| view.macros.is_empty() && view.selected(cx).is_none() && view.trouble.is_none()));
    // Nothing to run: Return does nothing, Escape closes.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    click(cx, "macro-cancel");
    assert_eq!(modals(&app, cx), 0);

    // A project that cannot be read is said to be so.
    app.update(cx, |app, cx| {
        let mut wb = Workbook::new(app.wb.locale);
        wb.vba.project = Some(VbaProject {
            bin: Arc::from(&b"not a project"[..]),
            signatures: Vec::new(),
        });
        app.set_workbook(wb, None, "Odd".to_string(), cx);
    });
    let view = app.update_in(cx, |app, window, cx| macros::open_dialog(app, window, cx));
    redraw(cx);
    assert!(view.update(cx, |view, _| view.macros.is_empty() && view.trouble.is_some()));
}

#[gpui::test]
fn saving_without_the_macros_asks_excel_s_question(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let project = with_macros(&app, cx);
    let dir = scripts::default_folder().join("saved");
    std::fs::create_dir_all(&dir).unwrap();
    // A workbook with macros is proposed as a macro-enabled one, and
    // saved as such without a question.
    assert_eq!(app.update(cx, |app, _| default_extension(app)), "xlsm");
    let xlsm = dir.join("Budget.xlsm");
    assert!(!app.update(cx, |app, cx| asks_before_saving(app, &xlsm, cx)));
    app.update(cx, |app, cx| app.write_unasked(xlsm.clone(), cx));
    let back = opensheet_io::load(&xlsm, opensheet_engine::Locale::en()).unwrap();
    assert_eq!(back.vba.project.as_ref().map(|p| p.bin.to_vec()), Some(project.clone()));
    assert_eq!(back.sheets[0].code_name.as_deref(), Some("Sheet1"));

    // As a macro-free workbook: the question, "No" proposed.
    let xlsx = dir.join("Budget.xlsx");
    assert!(app.update(cx, |app, cx| asks_before_saving(app, &xlsx, cx)));
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modals(&app, cx) == 0 && !xlsx.exists());
    // "Yes" writes the file without the project; the workbook in the
    // window keeps it.
    assert!(app.update(cx, |app, cx| asks_before_saving(app, &xlsx, cx)));
    redraw(cx);
    press_alert_button(cx, 0);
    assert_eq!(modals(&app, cx), 0);
    assert!(xlsx.exists());
    let back = opensheet_io::load(&xlsx, opensheet_engine::Locale::en()).unwrap();
    assert!(!back.has_vba_project());
    assert_eq!(back.value_at(0, 0, 0), Value::Number(7.0));
    assert!(app.update(cx, |app, _| app.wb.has_vba_project()));

    // A workbook without macros is never asked.
    app.update(cx, |app, cx| {
        let wb = Workbook::new(app.wb.locale);
        app.set_workbook(wb, None, "Plain".to_string(), cx);
    });
    assert_eq!(app.update(cx, |app, _| default_extension(app)), "xlsx");
    assert!(!app.update(cx, |app, cx| asks_before_saving(app, &xlsx, cx)));
}
