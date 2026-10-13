//! Allow Edit Ranges driven in GPUI's headless window.

use super::edit_ranges::{ConfirmRangePassword, EditRanges, RangeDialog, UnlockRange};
use super::*;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::{Rect, Value};

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    let rect = Rect::parse(range).expect("a range");
    app.update(cx, |app, cx| app.select_rect(rect, cx));
    redraw(cx);
}

fn value(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Value {
    let (row, col) = opensheet_engine::refs::parse_cell_name(name).expect("a cell name");
    app.update(cx, |app, _| app.wb.value_at(app.sheet, row, col))
}

fn front<V: 'static>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<V> {
    let view = app.update(cx, |app, _| app.front_modal_view()).expect("a dialog is open");
    view.downcast::<V>().ok().expect("the dialog expected")
}

fn alert_text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    app.update(cx, |app, cx| app.front_alert(cx)).map(|(_, message)| message).unwrap_or_default()
}

fn close(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    redraw(cx);
}

fn ranges(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(String, Vec<Rect>, bool)> {
    app.update(cx, |app, _| app.wb.sheets[0].protected_ranges.iter().map(|range| (range.name.clone(), range.ranges.clone(), range.password.is_some())).collect())
}

/// New… in the open list: a title, cells and a password (typed twice).
fn new_range(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, title: Option<&str>, cells: &str, password: &str) {
    front::<EditRanges>(app, cx).update_in(cx, |list, window, cx| {
        list.edit(None, window, cx);
    });
    redraw(cx);
    let dialog = front::<RangeDialog>(app, cx);
    if let Some(title) = title {
        dialog.update(cx, |dialog, cx| dialog.set(title, cells, password, cx));
    }
    cx.simulate_keystrokes("enter");
    redraw(cx);
    if !password.is_empty() {
        // Confirm Password.
        front::<ConfirmRangePassword>(app, cx).update(cx, |dialog, cx| dialog.set(password, cx));
        cx.simulate_keystrokes("enter");
        redraw(cx);
    }
}

#[gpui::test]
fn allow_edit_ranges_from_the_dialogs(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, "B2:C3");
    run(&app, cx, Cmd::AllowEditRanges);
    // New proposes the selection under the title Range1.
    front::<EditRanges>(&app, cx).update_in(cx, |list, window, cx| {
        list.edit(None, window, cx);
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    new_range(&app, cx, Some("Budget"), "=$E$1:$E$4,$G$1", "pw");
    // A title taken, cells that are none: the dialog stays.
    new_range(&app, cx, Some("budget"), "=$A$9", "");
    assert!(alert_text(&app, cx).contains("already exists"), "{}", alert_text(&app, cx));
    close(&app, cx);
    front::<RangeDialog>(&app, cx).update(cx, |dialog, cx| dialog.set("Other", "nowhere", "", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(alert_text(&app, cx).contains("not valid"));
    close(&app, cx);
    close(&app, cx);
    // Nothing is on the sheet before OK.
    assert!(ranges(&app, cx).is_empty());
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.modal_open()));
    let rect = |name: &str| Rect::parse(name).unwrap();
    assert_eq!(ranges(&app, cx), [("Range1".to_string(), vec![rect("B2:C3")], false), ("Budget".to_string(), vec![rect("E1:E4"), rect("G1")], true)]);
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Allow Edit Ranges"));

    // Modify and Delete, then Undo brings the list back.
    run(&app, cx, Cmd::AllowEditRanges);
    let list = front::<EditRanges>(&app, cx);
    list.update_in(cx, |list, window, cx| {
        list.edit(Some(0), window, cx);
    });
    redraw(cx);
    front::<RangeDialog>(&app, cx).update(cx, |dialog, cx| dialog.set("Inputs", "=$B$2:$C$5", "", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    list.update(cx, |list, cx| {
        list.select(1, cx);
        list.delete(cx);
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(ranges(&app, cx), [("Inputs".to_string(), vec![rect("B2:C5")], false)]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(ranges(&app, cx).len(), 2);
    // Cancel changes nothing.
    run(&app, cx, Cmd::AllowEditRanges);
    front::<EditRanges>(&app, cx).update(cx, |list, cx| list.delete(cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(ranges(&app, cx).len(), 2);
}

#[gpui::test]
fn a_protected_sheet_lets_its_ranges_be_edited(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::AllowEditRanges);
    new_range(&app, cx, Some("Open"), "=$B$2:$C$3", "");
    new_range(&app, cx, Some("Budget"), "=$E$1:$E$4", "pw");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, Default::default(), "");
        app.refresh(cx);
    });
    redraw(cx);
    // The command is greyed out while the sheet is protected.
    assert!(!app.update(cx, |app, _| Cmd::AllowEditRanges.enabled(app)));

    // A range without a password: typed into at once.
    select(&app, cx, "B2");
    cx.simulate_input("12");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "B2"), Value::Number(12.0));
    // Elsewhere, the sheet says no.
    select(&app, cx, "A1");
    cx.simulate_input("x");
    redraw(cx);
    assert!(alert_text(&app, cx).contains("protected sheet"), "{}", alert_text(&app, cx));
    close(&app, cx);
    assert_eq!(value(&app, cx, "A1"), Value::Empty);

    // A range with a password: Unlock Range asks for it.
    select(&app, cx, "E2");
    cx.simulate_input("5");
    redraw(cx);
    let unlock = front::<UnlockRange>(&app, cx);
    unlock.update(cx, |dialog, cx| dialog.set_password("wrong", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(alert_text(&app, cx).contains("password"), "{}", alert_text(&app, cx));
    close(&app, cx);
    cx.simulate_input("5");
    redraw(cx);
    front::<UnlockRange>(&app, cx).update(cx, |dialog, cx| dialog.set_password("pw", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.modal_open()));
    // Asked once: from now on the range takes what is typed.
    cx.simulate_input("5");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "E2"), Value::Number(5.0));
    select(&app, cx, "E4");
    cx.simulate_input("7");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "E4"), Value::Number(7.0));
    assert!(!app.update(cx, |app, _| app.modal_open()));
    // Undo, and the range is still open.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "E4"), Value::Empty);
    select(&app, cx, "E4");
    cx.simulate_input("8");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "E4"), Value::Number(8.0));
}
