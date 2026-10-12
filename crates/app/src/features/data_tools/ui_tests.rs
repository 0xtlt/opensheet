//! The data tools driven in GPUI's headless window: commands, dialogs,
//! keys and typing. What these tests cannot tell is how things look.

use super::*;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::text_columns::{ColumnFormat, DateOrder};
use opensheet_engine::{Rect, Value, Workbook};

/// A window on an empty workbook, with the key bindings of the application.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// Change the workbook as a command would, and draw the result.
fn edit(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, f: impl FnOnce(&mut Workbook)) {
    app.update(cx, |app, cx| {
        f(&mut app.wb);
        app.commit("setup", cx);
    });
    redraw(cx);
}

/// Type entries into cells of the current sheet ("A1", text).
fn fill(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cells: &[(&str, &str)]) {
    edit(app, cx, |wb| {
        for (name, text) in cells {
            let (row, col) = opensheet_engine::refs::parse_cell_name(name).expect("a cell name");
            wb.set_cell_text(0, row, col, text).expect("an entry");
        }
    });
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

fn text(s: &str) -> Value {
    Value::text(s)
}

fn modal_open(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> bool {
    app.update(cx, |app, _| app.modal_open())
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

// ---------------------------------------------------------- Text to Columns

fn wizard(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<text_wizard::Wizard> {
    let wizard = app.update_in(cx, |app, window, cx| text_wizard::open_text_to_columns(app, window, cx));
    redraw(cx);
    wizard.expect("the wizard opens")
}

#[gpui::test]
fn text_to_columns_with_return_alone(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Jean;Dupont;42"), ("A2", "Marie;Curie;7")]);
    select(&app, cx, "A1:A2");
    run(&app, cx, Cmd::TextToColumns);
    assert!(modal_open(&app, cx));
    // The semicolon was recognised: Return walks the three steps.
    cx.simulate_keystrokes("enter enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("Jean"));
    assert_eq!(value(&app, cx, "B2"), text("Curie"));
    assert_eq!(value(&app, cx, "C1"), Value::Number(42.0));
    // One step back.
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text to Columns"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "A1"), text("Jean;Dupont;42"));
    assert_eq!(value(&app, cx, "B1"), Value::Empty);
}

#[gpui::test]
fn text_to_columns_formats_and_destination(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "007,31/01/2024,x,y")]);
    select(&app, cx, "A1");
    let wizard = wizard(&app, cx);
    wizard.update(cx, |w, cx| {
        w.go_to_step(1, cx);
        w.go_to_step(2, cx);
        w.set_format(ColumnFormat::Text, cx);
        w.select_column(1, cx);
        w.set_format(ColumnFormat::Date(DateOrder::DMY), cx);
        w.select_column(2, cx);
        w.set_format(ColumnFormat::Skip, cx);
        w.set_destination("$B$3", cx);
    });
    redraw(cx);
    wizard.update_in(cx, |w, window, cx| w.finish(window, cx));
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("007,31/01/2024,x,y"));
    assert_eq!(value(&app, cx, "B3"), text("007"));
    assert!(matches!(value(&app, cx, "C3"), Value::Number(n) if n > 45000.0));
    assert_eq!(value(&app, cx, "D3"), text("y"));
}

#[gpui::test]
fn text_to_columns_asks_before_replacing(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "a b"), ("B1", "busy")]);
    select(&app, cx, "A1");
    let wizard = wizard(&app, cx);
    wizard.update_in(cx, |w, window, cx| w.finish(window, cx));
    redraw(cx);
    // The wizard is gone and Excel's question is there; Escape answers no.
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B1"), text("busy"));
    // Again, and Return answers OK.
    let wizard = self::wizard(&app, cx);
    wizard.update_in(cx, |w, window, cx| w.finish(window, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("a"));
    assert_eq!(value(&app, cx, "B1"), text("b"));
}

#[gpui::test]
fn text_to_columns_fixed_width_break_lines(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Jean     12  Paris"), ("A2", "Martine   7  Lyon")]);
    select(&app, cx, "A1:A2");
    let wizard = wizard(&app, cx);
    // Columns padded with blanks: fixed width, with the breaks found.
    assert_eq!(wizard.update(cx, |w, _| w.breaks().to_vec()), vec![9, 13]);
    wizard.update(cx, |w, cx| {
        w.go_to_step(1, cx);
        // A click adds a line, a drag moves it, a double click removes one.
        w.ruler_press(4, 1, cx);
        w.ruler_drag(5, cx);
        w.ruler_release(cx);
        assert_eq!(w.breaks(), [5, 9, 13]);
        w.ruler_press(9, 1, cx);
        w.ruler_release(cx);
        w.ruler_press(9, 2, cx);
        assert_eq!(w.breaks(), [5, 13]);
        w.ruler_press(5, 2, cx);
        w.ruler_press(9, 1, cx);
        w.ruler_release(cx);
        assert_eq!(w.breaks(), [9, 13]);
    });
    redraw(cx);
    wizard.update_in(cx, |w, window, cx| w.finish(window, cx));
    redraw(cx);
    assert_eq!(value(&app, cx, "A2"), text("Martine"));
    assert_eq!(value(&app, cx, "B1"), Value::Number(12.0));
    assert_eq!(value(&app, cx, "C2"), text("Lyon"));
}

#[gpui::test]
fn text_to_columns_refusals(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "a b"), ("B1", "c d")]);
    // Two columns, then nothing at all: a message, no wizard.
    for range in ["A1:B1", "D5"] {
        select(&app, cx, range);
        let wizard = app.update_in(cx, |app, window, cx| text_wizard::open_text_to_columns(app, window, cx));
        redraw(cx);
        assert!(wizard.is_none(), "{range}");
        assert!(modal_open(&app, cx));
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert!(!modal_open(&app, cx));
    }
    // A destination that is not a reference keeps the wizard open.
    select(&app, cx, "A1");
    let wizard = wizard(&app, cx);
    wizard.update(cx, |w, cx| w.set_destination("nowhere", cx));
    wizard.update_in(cx, |w, window, cx| w.finish(window, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("a b"));
}

// --------------------------------------------------------------- Flash Fill

#[gpui::test]
fn flash_fill_from_the_keyboard(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Jean Dupont"), ("A2", "Marie Curie"), ("A3", "Victor Hugo"), ("B1", "Dupont, J.")]);
    select(&app, cx, "B2");
    cx.simulate_keystrokes("ctrl-e");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B2"), text("Curie, M."));
    assert_eq!(value(&app, cx, "B3"), text("Hugo, V."));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Flash Fill"));
    // One step back, and the command of the ribbon does the same.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "B2"), Value::Empty);
    assert_eq!(value(&app, cx, "B3"), Value::Empty);
    run(&app, cx, Cmd::FlashFill);
    assert_eq!(value(&app, cx, "B3"), text("Hugo, V."));
}

#[gpui::test]
fn flash_fill_without_a_pattern_says_so(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Jean Dupont"), ("A2", "Marie Curie"), ("B1", "xyz")]);
    select(&app, cx, "B2");
    run(&app, cx, Cmd::FlashFill);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B2"), Value::Empty);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
}

#[gpui::test]
fn flash_fill_respects_protection(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Jean Dupont"), ("A2", "Marie Curie"), ("B1", "Jean")]);
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    select(&app, cx, "B2");
    run(&app, cx, Cmd::FlashFill);
    // The protection message, and nothing written.
    assert!(modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B2"), Value::Empty);
}

// ---------------------------------------------------------- Advanced Filter

/// A list in A1:B5 with a criteria range in D1:D2.
fn staff(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    fill(
        app,
        cx,
        &[
            ("A1", "Name"),
            ("B1", "City"),
            ("A2", "Ann"),
            ("B2", "Paris"),
            ("A3", "Bob"),
            ("B3", "Lyon"),
            ("A4", "Cid"),
            ("B4", "Paris"),
            ("A5", "Dee"),
            ("B5", "Nice"),
            ("D1", "City"),
            ("D2", "Paris"),
        ],
    );
}

fn hidden_rows(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u32> {
    app.update(cx, |app, _| app.wb.sheets[app.sheet].filtered_rows.iter().copied().collect())
}

#[gpui::test]
fn advanced_filter_in_place_and_clear(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    staff(&app, cx);
    select(&app, cx, "A2");
    let dialog = app.update_in(cx, |app, window, cx| advanced::open(app, window, cx));
    redraw(cx);
    assert!(modal_open(&app, cx));
    // The list is proposed; the keyboard goes to the criteria range.
    // (The first Tab enters the dialog: no frame has given its first
    // control the keyboard in this window.)
    cx.simulate_keystrokes("tab tab tab");
    cx.simulate_input("D1:D2");
    assert_eq!(dialog.update(cx, |d, cx| d.texts(cx)), ["$A$1:$B$5".to_string(), "D1:D2".to_string(), String::new()]);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(hidden_rows(&app, cx), [2, 4]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Advanced Filter"));
    // Data > Clear shows the rows again; undo twice goes back to the start.
    assert!(app.update(cx, |app, _| crate::features::Command::enabled(crate::features::filters::Cmd::Clear, app)));
    app.update_in(cx, |app, window, cx| app.run_command(crate::features::filters::Cmd::Clear, window, cx));
    redraw(cx);
    assert_eq!(hidden_rows(&app, cx), Vec::<u32>::new());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(hidden_rows(&app, cx), [2, 4]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(hidden_rows(&app, cx), Vec::<u32>::new());
}

#[gpui::test]
fn advanced_filter_copies_to_another_location(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    staff(&app, cx);
    fill(&app, cx, &[("G1", "City"), ("H1", "Name")]);
    select(&app, cx, "A1");
    let dialog = app.update_in(cx, |app, window, cx| advanced::open(app, window, cx));
    redraw(cx);
    let request = advanced::Request {
        copy: true,
        list: "$A$1:$B$5".into(),
        criteria: "$D$1:$D$2".into(),
        copy_to: "$G$1:$H$1".into(),
        unique: false,
    };
    dialog.update(cx, |dialog, cx| dialog.set(&request, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    // The headings in place choose and order the columns.
    assert_eq!(value(&app, cx, "G2"), text("Paris"));
    assert_eq!(value(&app, cx, "H2"), text("Ann"));
    assert_eq!(value(&app, cx, "H3"), text("Cid"));
    assert_eq!(value(&app, cx, "G4"), Value::Empty);
    assert_eq!(hidden_rows(&app, cx), Vec::<u32>::new());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "G2"), Value::Empty);
}

#[gpui::test]
fn advanced_filter_refusals_keep_the_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    staff(&app, cx);
    fill(&app, cx, &[("G1", "Colour")]);
    select(&app, cx, "A1");
    let dialog = app.update_in(cx, |app, window, cx| advanced::open(app, window, cx));
    redraw(cx);
    let mut request = advanced::Request {
        copy: true,
        list: "$A$1:$B$5".into(),
        criteria: String::new(),
        copy_to: "$G$1".into(),
        unique: true,
    };
    for (criteria, copy_to) in [("", "$G$1"), ("nowhere", "$J$1"), ("", "")] {
        request.criteria = criteria.into();
        request.copy_to = copy_to.into();
        dialog.update(cx, |dialog, cx| dialog.set(&request, cx));
        redraw(cx);
        cx.simulate_keystrokes("enter");
        redraw(cx);
        // A message over the dialog; once read, the dialog is still there.
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert!(modal_open(&app, cx), "{criteria:?} {copy_to:?}");
    }
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
}

// -------------------------------------------------------------- Consolidate

#[gpui::test]
fn consolidate_by_category_and_again(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(
        &app,
        cx,
        &[
            ("B1", "Q1"),
            ("A2", "Apples"),
            ("B2", "10"),
            ("A3", "Pears"),
            ("B3", "5"),
            ("E1", "Q1"),
            ("D2", "Pears"),
            ("E2", "1"),
        ],
    );
    select(&app, cx, "A8");
    let dialog = app.update_in(cx, |app, window, cx| consolidate::open(app, window, cx));
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        dialog.set_reference("A1:B3", cx);
        assert!(dialog.add(cx));
        // The same range twice is listed once.
        assert!(dialog.add(cx));
        dialog.set_reference("$D$1:$E$2", cx);
        assert!(dialog.add(cx));
        dialog.set_reference("G1:G3", cx);
        assert!(dialog.add(cx));
        // Delete takes the selected line away.
        dialog.select(2, cx);
        dialog.delete(cx);
        let spec = dialog.spec_mut();
        assert_eq!(spec.sources.len(), 2);
        spec.top_row = true;
        spec.left_column = true;
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B8"), text("Q1"));
    assert_eq!(value(&app, cx, "A10"), text("Pears"));
    assert_eq!(value(&app, cx, "B10"), Value::Number(6.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Consolidate"));
    // The dialog opens on the same settings.
    let again = app.update_in(cx, |app, window, cx| consolidate::open(app, window, cx));
    redraw(cx);
    again.update(cx, |dialog, _| {
        let spec = dialog.spec_mut();
        assert_eq!(spec.sources.len(), 2);
        assert!(spec.top_row && spec.left_column);
    });
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // One step back: the cells and the settings.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "A10"), Value::Empty);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].consolidation.is_none()));
}

#[gpui::test]
fn consolidate_refusals(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "1"), ("A2", "2")]);
    select(&app, cx, "A1");
    run(&app, cx, Cmd::Consolidate);
    assert!(modal_open(&app, cx));
    // No reference at all: a message, the dialog stays.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // A reference that is none, then one that covers the destination.
    let dialog = app.update_in(cx, |app, window, cx| consolidate::open(app, window, cx));
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        dialog.set_reference("not a range", cx);
        assert!(!dialog.add(cx));
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.set_reference("A1:A2", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
}

// ----------------------------------------------------------- What-If Analysis

fn number(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> f64 {
    match value(app, cx, name) {
        Value::Number(n) => n,
        other => panic!("{name} holds {other:?}"),
    }
}

fn modal_count(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    let mut count = 0;
    // Close them one by one, counting, then nothing is left open.
    while modal_open(app, cx) {
        app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
        count += 1;
    }
    count
}

#[gpui::test]
fn what_if_menu_opens_the_three_dialogs(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::WhatIf);
    assert!(!modal_open(&app, cx));
    for cmd in [Cmd::ScenarioManager, Cmd::GoalSeek, Cmd::DataTable] {
        run(&app, cx, cmd);
        assert!(modal_open(&app, cx), "{cmd:?}");
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert!(!modal_open(&app, cx), "{cmd:?}");
    }
}

#[gpui::test]
fn goal_seek_ok_keeps_and_cancel_restores(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "1"), ("B1", "=A1*3+2")]);
    select(&app, cx, "B1");
    // Set cell is the active cell; the value and the changing cell are typed.
    run(&app, cx, Cmd::GoalSeek);
    cx.simulate_keystrokes("tab tab");
    cx.simulate_input("14");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("A1");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // The status dialog is up, the sheet shows the solution, nothing is
    // committed yet.
    assert!(modal_open(&app, cx));
    assert!((number(&app, cx, "A1") - 4.0).abs() < 1e-6);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A1"), Value::Number(1.0));
    assert_eq!(value(&app, cx, "B1"), Value::Number(5.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));

    // Again, kept with Return, undone in one step.
    let dialog = app.update_in(cx, |app, window, cx| whatif::open_goal_seek(app, window, cx));
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.set("$B$1", "14", "$A$1", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert!((number(&app, cx, "B1") - 14.0).abs() < 1e-6);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Goal Seek"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "A1"), Value::Number(1.0));
}

#[gpui::test]
fn goal_seek_refusals(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "1"), ("B1", "=A1*3+2")]);
    select(&app, cx, "B1");
    // No formula in the set cell, a formula in the changing cell, a value
    // that is no number, a reference that is none.
    for (set, value, changing) in [("A1", "5", "A1"), ("B1", "5", "B1"), ("B1", "five", "A1"), ("B1", "5", "")] {
        let dialog = app.update_in(cx, |app, window, cx| whatif::open_goal_seek(app, window, cx));
        redraw(cx);
        dialog.update(cx, |dialog, cx| dialog.set(set, value, changing, cx));
        cx.simulate_keystrokes("enter");
        redraw(cx);
        // A message; once read, the dialog is still there.
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modal_count(&app, cx), 1, "{set} {value} {changing}");
    }
    assert_eq!(value(&app, cx, "A1"), Value::Number(1.0));
}

#[gpui::test]
fn scenarios_from_the_manager(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "100"), ("A2", "0.1"), ("A3", "=A1*(1+A2)")]);
    select(&app, cx, "A1:A2");
    let manager = app.update_in(cx, |app, window, cx| whatif::open_scenario_manager(app, window, cx));
    redraw(cx);
    // Add: the changing cells are the selection, the values what they hold.
    for (name, values) in [("Low", ["50", "0.05"]), ("High", ["200", "0.2"])] {
        let scenario = manager.update_in(cx, |manager, window, cx| manager.edit(None, window, cx)).expect("Add Scenario");
        redraw(cx);
        cx.simulate_keystrokes("tab");
        cx.simulate_input(name);
        let values_dialog = scenario.update_in(cx, |dialog, window, cx| dialog.next(window, cx)).expect("Scenario Values");
        redraw(cx);
        values_dialog.update(cx, |dialog, cx| dialog.set(&values, cx));
        cx.simulate_keystrokes("enter");
        redraw(cx);
    }
    let names = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| -> Vec<String> {
        app.update(cx, |app, _| app.wb.sheets[app.sheet].scenarios.iter().map(|s| s.name.clone()).collect())
    };
    assert_eq!(names(&app, cx), ["Low", "High"]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Add Scenario"));
    // The manager is still there, on the scenario just added: Return shows it.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), Value::Number(200.0));
    assert!((number(&app, cx, "A3") - 240.0).abs() < 1e-9);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Show Scenario"));
    // A name already taken is refused and the dialog stays.
    let scenario = manager.update_in(cx, |manager, window, cx| manager.edit(None, window, cx)).expect("Add Scenario");
    redraw(cx);
    scenario.update(cx, |dialog, cx| dialog.set("low", "A1", cx));
    assert!(scenario.update_in(cx, |dialog, window, cx| dialog.next(window, cx)).is_none());
    redraw(cx);
    cx.simulate_keystrokes("enter escape");
    redraw(cx);
    // Edit the first one: its values are proposed again.
    manager.update(cx, |manager, cx| manager.select(0, cx));
    let scenario = manager.update_in(cx, |manager, window, cx| manager.edit(Some(0), window, cx)).expect("Edit Scenario");
    redraw(cx);
    let values_dialog = scenario.update_in(cx, |dialog, window, cx| dialog.next(window, cx)).expect("Scenario Values");
    redraw(cx);
    values_dialog.update(cx, |dialog, cx| dialog.set(&["60"], cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let low = app.update(cx, |app, _| app.wb.sheets[0].scenarios[0].clone());
    assert_eq!((low.cells[0].value.as_str(), low.cells[1].value.as_str()), ("60", "0.05"));
    assert!(low.comment.contains("Created by") && low.comment.contains("Modified by"), "{}", low.comment);
    // Delete the second, then undo brings it back.
    manager.update(cx, |manager, cx| {
        manager.select(1, cx);
        manager.delete(cx);
    });
    assert_eq!(names(&app, cx), ["Low"]);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(names(&app, cx), ["Low", "High"]);
}

#[gpui::test]
fn scenario_summary_makes_a_sheet(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "100"), ("A3", "=A1*2")]);
    edit(&app, cx, |wb| {
        let cell = |value: &str| opensheet_engine::whatif::ScenarioCell {
            row: 0,
            col: 0,
            value: value.into(),
        };
        for (name, value) in [("Low", "50"), ("High", "200")] {
            let scenario = opensheet_engine::whatif::Scenario {
                name: name.into(),
                cells: vec![cell(value)],
                ..Default::default()
            };
            wb.scenario_add(0, scenario).unwrap();
        }
    });
    let manager = app.update_in(cx, |app, window, cx| whatif::open_scenario_manager(app, window, cx));
    redraw(cx);
    let summary = manager.update_in(cx, |manager, window, cx| manager.summary(window, cx)).expect("Scenario Summary");
    redraw(cx);
    summary.update(cx, |dialog, cx| dialog.set("A3", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // Both dialogs are gone and the report is the sheet in view.
    assert!(!modal_open(&app, cx));
    let (sheet, name, count) = app.update(cx, |app, _| (app.sheet, app.wb.sheets[app.sheet].name.clone(), app.wb.sheets.len()));
    assert_eq!((sheet, name.as_str(), count), (0, "Scenario Summary", 2));
    assert_eq!(value(&app, cx, "E3"), text("Low"));
    assert_eq!(value(&app, cx, "F5"), Value::Number(200.0));
    assert_eq!(value(&app, cx, "C7"), text("$A$3"));
    assert_eq!(value(&app, cx, "E7"), Value::Number(100.0));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.wb.sheets.len()), 1);
    assert_eq!(value(&app, cx, "A1"), Value::Number(100.0));
}

#[gpui::test]
fn scenarios_respect_protection(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "100")]);
    edit(&app, cx, |wb| {
        let mut protection = opensheet_engine::protect::SheetProtection::default();
        protection.set(opensheet_engine::protect::Permission::EditScenarios, false);
        wb.protect_sheet(0, protection, "");
    });
    let manager = app.update_in(cx, |app, window, cx| whatif::open_scenario_manager(app, window, cx));
    redraw(cx);
    assert!(manager.update_in(cx, |manager, window, cx| manager.edit(None, window, cx)).is_none());
    redraw(cx);
    // The manager, and the message over it.
    assert_eq!(modal_count(&app, cx), 2);
}

#[gpui::test]
fn data_table_from_the_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(
        &app,
        cx,
        &[("B1", "10"), ("B2", "3"), ("B3", "=B1*B2"), ("D1", "=B3"), ("E1", "10"), ("F1", "20"), ("D2", "1"), ("D3", "2")],
    );
    select(&app, cx, "D1:F3");
    run(&app, cx, Cmd::DataTable);
    cx.simulate_keystrokes("tab");
    cx.simulate_input("B1");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("B2");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "F3"), Value::Number(40.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Data Table"));
    // The formula bar shows Excel's array formula.
    assert_eq!(app.update(cx, |app, _| app.wb.formula_bar_text(0, 2, 5)), "{=TABLE(B1,B2)}");
    // The table follows the model.
    fill(&app, cx, &[("B3", "=B1*B2+1")]);
    assert_eq!(value(&app, cx, "F3"), Value::Number(41.0));

    // One result cannot be typed over or cleared: Excel's message.
    select(&app, cx, "F3");
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert_eq!(modal_count(&app, cx), 1);
    assert_eq!(value(&app, cx, "F3"), Value::Number(41.0));
    cx.simulate_input("7");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    modal_count(&app, cx);
    app.update_in(cx, |app, window, cx| {
        if app.is_editing() {
            app.cancel_edit(window, cx);
        }
    });
    assert_eq!(value(&app, cx, "F3"), Value::Number(41.0));
    // Cleared as a whole, it is gone; undo brings it back, live.
    select(&app, cx, "E2:F3");
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "F3"), Value::Empty);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].data_tables.is_empty()));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "F3"), Value::Number(41.0));
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].data_tables.len()), 1);
}

#[gpui::test]
fn data_table_refusals(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("B1", "10"), ("D1", "=B1"), ("E1", "1"), ("D2", "2")]);
    for (selection, row, column) in [("D1:E2", "", ""), ("D1:E2", "nowhere", ""), ("D1:E2", "E2", ""), ("D1", "B1", "")] {
        select(&app, cx, selection);
        let dialog = app.update_in(cx, |app, window, cx| whatif::open_data_table(app, window, cx));
        redraw(cx);
        dialog.update(cx, |dialog, cx| dialog.set(row, column, cx));
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modal_count(&app, cx), 2, "{selection} {row:?}");
    }
    assert!(app.update(cx, |app, _| app.wb.sheets[0].data_tables.is_empty()));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
}
