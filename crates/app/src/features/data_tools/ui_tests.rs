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
fn consolidate_with_links_to_the_sources(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "Q1"), ("B1", "Q2"), ("A2", "1"), ("B2", "2"), ("A3", "3"), ("B3", "4")]);
    // On the sheet of the sources Excel refuses the links.
    select(&app, cx, "E1");
    let dialog = app.update_in(cx, |app, window, cx| consolidate::open(app, window, cx));
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        dialog.set_reference("A1:B3", cx);
        assert!(dialog.add(cx));
        let spec = dialog.spec_mut();
        spec.top_row = true;
        spec.link = true;
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "E1"), Value::Empty);
    // From another sheet: links folded under their totals.
    let source = app.update(cx, |app, _| app.wb.sheets[0].name.clone());
    app.update_in(cx, |app, window, cx| {
        let sheet = app.wb.add_sheet(1);
        app.commit("setup", cx);
        app.switch_sheet(sheet, window, cx);
    });
    redraw(cx);
    select(&app, cx, "A1");
    let dialog = app.update_in(cx, |app, window, cx| consolidate::open(app, window, cx));
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        dialog.set_reference(&format!("{source}!$A$1:$B$3"), cx);
        assert!(dialog.add(cx));
        let spec = dialog.spec_mut();
        spec.top_row = true;
        spec.link = true;
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A3"), Value::Number(1.0));
    assert_eq!(value(&app, cx, "B5"), Value::Number(4.0));
    let (hidden, formula) = app.update(cx, |app, _| {
        let hidden: Vec<u32> = app.wb.sheets[app.sheet].hidden_rows.iter().copied().collect();
        (hidden, app.wb.formula_text(app.sheet, 1, 0))
    });
    assert_eq!(hidden, [1, 3]);
    assert_eq!(formula, Some(format!("={source}!$A$2")));
    // The outline shows in the margin, and one step takes all of it back.
    assert!(app.update(cx, |app, _| app.current_layout().unwrap().margin_w) > 0.0);
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(value(&app, cx, "A3"), Value::Empty);
    assert!(app.update(cx, |app, _| app.wb.sheets[app.sheet].outline.is_empty()));
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

// ------------------------------------------------- Get Data and Refresh All

/// A text file of this test, in a directory of its own.
fn text_file(name: &str, content: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("opensheet-datatools-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory for the test");
    let path = dir.join("sales.csv");
    std::fs::write(&path, content).expect("the file is written");
    path
}

fn import(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, path: &std::path::Path) -> Entity<text_wizard::Wizard> {
    let wizard = app.update_in(cx, |app, window, cx| import::import_file(app, path, window, cx));
    redraw(cx);
    wizard.expect("the Text Import Wizard")
}

#[gpui::test]
fn get_data_from_text_then_refresh_all(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let path = text_file("refresh", "Sales 2024\nName;Qty\nAnn;3\nBob;4\n");
    // The menu of the ribbon button.
    run(&app, cx, Cmd::GetData);
    assert!(!modal_open(&app, cx));
    select(&app, cx, "B2");
    let wizard = import(&app, cx, &path);
    assert!(modal_open(&app, cx));
    // The title line is skipped; the semicolon is recognised.
    wizard.update(cx, |wizard, cx| wizard.set_first_row(2, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter enter enter");
    redraw(cx);
    // "Import Data" proposes the active cell; Return takes it.
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "B2"), text("Name"));
    assert_eq!(value(&app, cx, "C4"), Value::Number(4.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Import Data"));
    let kept = app.update(cx, |app, _| app.wb.sheets[0].text_imports[0].clone());
    assert_eq!((kept.name.as_str(), kept.first_row, kept.range), ("sales", 2, Rect::parse("B2:C4").unwrap()));

    // The file changes; Refresh All reads it again, in one step.
    std::fs::write(&path, "Sales 2025\nName;Qty\nCid;9\n").unwrap();
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(value(&app, cx, "B3"), text("Cid"));
    assert_eq!(value(&app, cx, "C3"), Value::Number(9.0));
    assert_eq!(value(&app, cx, "B4"), Value::Empty);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Refresh All"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "B4"), text("Bob"));
    // Refresh works where the data is, and only there.
    select(&app, cx, "F9");
    assert!(!app.update(cx, |app, _| crate::features::Command::enabled(Cmd::Refresh, app)));
    select(&app, cx, "C3");
    assert!(app.update(cx, |app, _| crate::features::Command::enabled(Cmd::Refresh, app)));
    run(&app, cx, Cmd::Refresh);
    assert_eq!(value(&app, cx, "B3"), text("Cid"));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Refresh"));

    // The file is gone: Excel's message, and the data stays.
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(modal_count(&app, cx), 1);
    assert_eq!(value(&app, cx, "B3"), text("Cid"));
}

#[gpui::test]
fn get_data_on_a_new_sheet_and_refusals(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let path = text_file("new-sheet", "a,b\n1,2\n");
    let wizard = import(&app, cx, &path);
    wizard.update_in(cx, |wizard, window, cx| wizard.finish(window, cx));
    redraw(cx);
    // A destination that is no reference keeps the dialog.
    let dialog = app.update_in(cx, |app, window, cx| {
        app.close_modal(window, cx);
        let import = opensheet_engine::text_columns::TextImport {
            name: String::new(),
            path: path.to_string_lossy().into_owned(),
            range: Rect::cell(0, 0),
            first_row: 1,
            spec: opensheet_engine::text_columns::Spec::new(opensheet_engine::text_columns::Layout::Delimited(
                opensheet_engine::text_columns::Delimiters::only(','),
            )),
            web: None,
            props: Default::default(),
        };
        import::open_destination(app, import, "a,b\n1,2".to_string(), window, cx)
    });
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.set(false, "nowhere", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    // On a new sheet: it comes after the current one and is shown.
    dialog.update(cx, |dialog, cx| dialog.set(true, "", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(app.update(cx, |app, _| (app.sheet, app.wb.sheets.len())), (1, 2));
    assert_eq!(value(&app, cx, "B2"), Value::Number(2.0));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| (app.sheet, app.wb.sheets.len())), (0, 1));
    // A file that is not there, or empty: a message, no wizard.
    std::fs::write(&path, "").unwrap();
    for missing in [path.clone(), path.with_file_name("none.csv")] {
        assert!(app.update_in(cx, |app, window, cx| import::import_file(app, &missing, window, cx)).is_none());
        redraw(cx);
        assert_eq!(modal_count(&app, cx), 1);
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[gpui::test]
fn refresh_all_recomputes_pivot_tables(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    fill(&app, cx, &[("A1", "City"), ("B1", "Qty"), ("A2", "Paris"), ("B2", "1"), ("A3", "Lyon"), ("B3", "2")]);
    app.update(cx, |app, cx| {
        let source = opensheet_engine::pivot::PivotSource::parse("Sheet1!$A$1:$B$3", 0, &app.wb).expect("a source");
        let dest = opensheet_engine::pivot::PivotDestination::Existing { sheet: 0, row: 0, col: 4 };
        let id = app.wb.pivot_create(source, dest, false).expect("a PivotTable");
        app.wb.pivot_add_field(id, 1, opensheet_engine::pivot::Area::Values, None, false).expect("a value field");
        app.commit("pivot", cx);
    });
    redraw(cx);
    let total = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| -> Vec<Value> {
        // What the report shows, wherever its total sits.
        app.update(cx, |app, _| (0..8).flat_map(|r| (4..8).map(move |c| (r, c))).map(|(r, c)| app.wb.value_at(0, r, c)).collect())
    };
    assert!(total(&app, cx).contains(&Value::Number(3.0)));
    // The source changes; the report waits for a refresh.
    fill(&app, cx, &[("B3", "10")]);
    assert!(total(&app, cx).contains(&Value::Number(3.0)));
    run(&app, cx, Cmd::RefreshAll);
    assert!(total(&app, cx).contains(&Value::Number(11.0)), "{:?}", total(&app, cx));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Refresh All"));
}

// ------------------------------------------- Get Data: web and documents

use super::sources::test_server::{Server, page};

/// The dialog in front, as the view it is.
fn front<V: 'static>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<V> {
    let view = app.update(cx, |app, _| app.front_modal_view()).expect("a dialog is open");
    view.downcast::<V>().ok().expect("the dialog expected")
}

fn alert_text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    app.update(cx, |app, cx| app.front_alert(cx)).map(|(_, message)| message).unwrap_or_default()
}

fn sales_page(rows: &[(&str, &str)]) -> String {
    let body: String = rows.iter().map(|(name, qty)| format!("<tr><td>{name}</td><td>{qty}</td></tr>")).collect();
    format!("<html><body><table><tr><td>Menu</td></tr></table><table id=\"sales\"><tr><th>Name</th><th>Qty</th></tr>{body}</table></body></html>")
}

/// Type an address into From Web and confirm.
fn from_web(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, address: &str) {
    run(app, cx, Cmd::FromWeb);
    let dialog = front::<navigator::WebDialog>(app, cx);
    dialog.update(cx, |dialog, cx| dialog.set_address(address, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
}

#[gpui::test]
fn get_data_from_web_then_refresh(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/sales", page("text/html", &sales_page(&[("Ann", "3"), ("Bob", "4")])));
    // The address is typed without its scheme.
    from_web(&app, cx, &format!("127.0.0.1:{}/sales", server.port));
    // The Navigator lists the tables of the page; the second is chosen.
    let navigator = front::<navigator::Navigator>(&app, cx);
    assert_eq!(navigator.read_with(cx, |navigator, _| navigator.names()), ["Table 0", "sales"]);
    navigator.update(cx, |navigator, cx| navigator.select(1, cx));
    redraw(cx);
    // Load: on a new sheet, as Excel does.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(app.update(cx, |app, _| (app.sheet, app.wb.sheets.len())), (1, 2));
    assert_eq!(value(&app, cx, "A1"), text("Name"));
    assert_eq!(value(&app, cx, "B3"), Value::Number(4.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Import Data"));
    let kept = app.update(cx, |app, _| app.wb.sheets[1].text_imports[0].clone());
    assert_eq!(kept.name, "sales");
    assert_eq!(kept.path, server.url("/sales"));
    assert_eq!(kept.web, Some(opensheet_engine::external::WebQuery { item: Some(opensheet_engine::external::QueryItem::Index(2)) }));
    assert_eq!(kept.range, Rect::parse("A1:B3").unwrap());

    // The page changes: Refresh All fetches it again, off the window
    // thread, and the data arrives as one step.
    server.set("/sales", page("text/html", &sales_page(&[("Cid", "9")])));
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(value(&app, cx, "A2"), text("Cid"));
    assert_eq!(value(&app, cx, "A3"), Value::Empty);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Refresh All"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "A3"), text("Bob"));
    // Refresh, where the data is.
    select(&app, cx, "B2");
    run(&app, cx, Cmd::Refresh);
    assert_eq!(value(&app, cx, "B2"), Value::Number(9.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Refresh"));

    // The table is gone from the page: Excel's message, the data stays.
    server.set("/sales", page("text/html", "<html><table><tr><td>Menu</td></tr></table></html>"));
    run(&app, cx, Cmd::Refresh);
    assert!(alert_text(&app, cx).contains("sales"), "{}", alert_text(&app, cx));
    assert_eq!(modal_count(&app, cx), 1);
    assert_eq!(value(&app, cx, "A2"), text("Cid"));
    // The page is gone.
    server.set("/sales", super::sources::test_server::Answer { status: 404, content_type: "text/html", body: Vec::new(), location: None });
    run(&app, cx, Cmd::RefreshAll);
    assert!(alert_text(&app, cx).contains("404"), "{}", alert_text(&app, cx));
    assert_eq!(modal_count(&app, cx), 1);
    // A protected sheet keeps its data as it is, silently.
    server.set("/sales", page("text/html", &sales_page(&[("Dan", "1")])));
    edit(&app, cx, |wb| {
        wb.protect_sheet(1, Default::default(), "");
    });
    run(&app, cx, Cmd::RefreshAll);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "A2"), text("Cid"));
}

#[gpui::test]
fn get_data_from_web_refusals_and_text(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/empty", page("text/html", "<html><p>No table</p></html>"));
    server.set("/data.csv", page("text/csv", "Name;Qty\nAnn;3\n"));
    // A page without a table, a page that is not there, no server at all.
    from_web(&app, cx, &server.url("/empty"));
    assert!(alert_text(&app, cx).contains("no data"), "{}", alert_text(&app, cx));
    assert_eq!(modal_count(&app, cx), 1);
    from_web(&app, cx, &server.url("/missing"));
    assert!(alert_text(&app, cx).contains("(HTTP/1.0 404)"));
    assert_eq!(modal_count(&app, cx), 1);
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    from_web(&app, cx, &format!("http://127.0.0.1:{free}/x"));
    assert!(alert_text(&app, cx).contains("Cannot locate the Internet server"));
    assert_eq!(modal_count(&app, cx), 1);
    // An empty address keeps the dialog.
    from_web(&app, cx, "  ");
    assert_eq!(modal_count(&app, cx), 1);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].text_imports.is_empty()));
    // A local file is not a web address, as a URL or as a path.
    from_web(&app, cx, "file:///etc/passwd");
    assert!(alert_text(&app, cx).contains("local file"), "{}", alert_text(&app, cx));
    assert_eq!(modal_count(&app, cx), 1);
    from_web(&app, cx, "/etc/passwd");
    assert!(alert_text(&app, cx).contains("local file"), "{}", alert_text(&app, cx));
    assert_eq!(modal_count(&app, cx), 1);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].text_imports.is_empty()));

    // An address that gives CSV goes through the Text Import Wizard, and
    // refreshes from the address.
    from_web(&app, cx, &server.url("/data.csv"));
    let _ = front::<text_wizard::Wizard>(&app, cx);
    cx.simulate_keystrokes("enter enter enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "B2"), Value::Number(3.0));
    let kept = app.update(cx, |app, _| app.wb.sheets[0].text_imports[0].clone());
    assert_eq!((kept.name.as_str(), kept.web.is_none(), kept.path == server.url("/data.csv")), ("data", true, true));
    server.set("/data.csv", page("text/csv", "Name;Qty\nAnn;30\n"));
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(value(&app, cx, "B2"), Value::Number(30.0));
    // The ODBC entries say what Excel says without a driver manager.
    run(&app, cx, Cmd::FromSqlServer);
    assert!(alert_text(&app, cx).contains("ODBC Driver Manager is not installed"));
    assert_eq!(modal_count(&app, cx), 1);
}

#[gpui::test]
fn get_data_from_documents(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let dir = text_file("documents", "unused").parent().unwrap().to_path_buf();
    // JSON: records to rows, nested records to columns; Load To asks where.
    let json = dir.join("people data.json");
    std::fs::write(&json, r#"{"people":[{"name":"Ann","address":{"city":"Paris"},"born":"1990-05-17"},{"name":"Bob","address":{"city":"Lyon"}}]}"#).unwrap();
    let address = super::sources::file_url(&json);
    app.update_in(cx, |app, window, cx| import::open_address(app, address.clone(), None, window, cx));
    redraw(cx);
    let navigator = front::<navigator::Navigator>(&app, cx);
    assert_eq!(navigator.read_with(cx, |navigator, _| navigator.names()), ["people data", "people"]);
    navigator.update_in(cx, |navigator, window, cx| {
        navigator.select(1, cx);
        navigator.load(true, window, cx);
    });
    redraw(cx);
    let place = front::<import::ImportDialog>(&app, cx);
    place.update(cx, |dialog, cx| dialog.set(false, "=$C$2", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(value(&app, cx, "C2"), text("name"));
    assert_eq!(value(&app, cx, "D2"), text("address.city"));
    assert_eq!(value(&app, cx, "D4"), text("Lyon"));
    assert!(matches!(value(&app, cx, "E3"), Value::Number(n) if n > 32000.0));
    // A second query cannot start inside the first.
    app.update_in(cx, |app, window, cx| import::open_address(app, address.clone(), None, window, cx));
    redraw(cx);
    front::<navigator::Navigator>(&app, cx).update_in(cx, |navigator, window, cx| navigator.load(true, window, cx));
    redraw(cx);
    front::<import::ImportDialog>(&app, cx).update(cx, |dialog, cx| dialog.set(false, "=$D$3", cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(alert_text(&app, cx).contains("existing External Data Range"));
    assert_eq!(modal_count(&app, cx), 2);
    // The file changes; Refresh reads it again.
    std::fs::write(&json, r#"{"people":[{"name":"Zoe","address":{"city":"Nice"}}]}"#).unwrap();
    select(&app, cx, "C3");
    run(&app, cx, Cmd::Refresh);
    assert_eq!(value(&app, cx, "C3"), text("Zoe"));
    assert_eq!(value(&app, cx, "C4"), Value::Empty);

    // A workbook: its sheets in the Navigator, refreshed when it changes.
    let book = dir.join("stock.xlsx");
    let write = |qty: &str| {
        let mut wb = Workbook::new(opensheet_engine::Locale::en());
        wb.sheets[0].name = "Stock".into();
        for (cell, entry) in [((0, 0), "Item"), ((0, 1), "Qty"), ((1, 0), "Nut"), ((1, 1), qty)] {
            wb.set_cell_text(0, cell.0, cell.1, entry).unwrap();
        }
        wb.commit("setup");
        opensheet_io::save_xlsx(&wb, &book).unwrap();
    };
    write("12");
    let address = super::sources::file_url(&book);
    app.update_in(cx, |app, window, cx| import::open_address(app, address, None, window, cx));
    redraw(cx);
    let navigator = front::<navigator::Navigator>(&app, cx);
    assert_eq!(navigator.read_with(cx, |navigator, _| navigator.names()), ["Stock"]);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.sheet), 1);
    assert_eq!(value(&app, cx, "B2"), Value::Number(12.0));
    write("15");
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(value(&app, cx, "B2"), Value::Number(15.0));

    // XML: a row per element under the root.
    let xml = dir.join("books.xml");
    std::fs::write(&xml, "<catalog><book id=\"b1\"><title>Dune</title></book><book id=\"b2\"><title>Emma</title></book></catalog>").unwrap();
    app.update_in(cx, |app, window, cx| import::open_address(app, super::sources::file_url(&xml), None, window, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "B3"), text("Emma"));
    std::fs::remove_dir_all(dir).ok();
}

#[gpui::test]
fn queries_and_connections_dialogs(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/a", page("text/html", &sales_page(&[("Ann", "3")])));
    server.set("/b", page("text/html", &sales_page(&[("Bea", "7"), ("Cy", "8")])));
    from_web(&app, cx, &server.url("/a"));
    front::<navigator::Navigator>(&app, cx).update_in(cx, |navigator, window, cx| {
        navigator.select(1, cx);
        navigator.load(false, window, cx);
    });
    redraw(cx);
    // Properties of the range under the active cell.
    assert!(app.update(cx, |app, _| crate::features::Command::enabled(Cmd::RangeProperties, app)));
    run(&app, cx, Cmd::RangeProperties);
    front::<connections::RangeDialog>(&app, cx).update(cx, |dialog, cx| dialog.set(false, false, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let props = app.update(cx, |app, _| app.wb.sheets[1].text_imports[0].props.clone());
    assert_eq!((props.preserve_formatting, props.adjust_width), (false, false));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Connection Properties"));

    // Queries & Connections: the list, then the properties of the one selected.
    run(&app, cx, Cmd::Connections);
    let list = front::<connections::ConnectionsDialog>(&app, cx);
    list.update_in(cx, |list, window, cx| list.properties(window, cx));
    redraw(cx);
    let properties = front::<connections::PropertiesDialog>(&app, cx);
    // A name that is no name keeps the dialog.
    properties.update(cx, |dialog, cx| dialog.set("1 bad", "", None, false, &server.url("/a"), cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(alert_text(&app, cx).contains("not valid"));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    let address = server.url("/b");
    properties.update(cx, |dialog, cx| dialog.set("weekly", "Sales of the week", Some(5), true, &address, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let kept = app.update(cx, |app, _| app.wb.sheets[1].text_imports[0].clone());
    assert_eq!((kept.name.as_str(), kept.props.description.as_str(), kept.props.refresh_minutes, kept.props.refresh_on_open), ("weekly", "Sales of the week", 5, true));
    assert_eq!(kept.path, address);
    // The list is still open, on the new name; Refresh reads the new address.
    let list = front::<connections::ConnectionsDialog>(&app, cx);
    list.update_in(cx, |list, window, cx| list.refresh(window, cx));
    redraw(cx);
    assert_eq!(value(&app, cx, "A3"), text("Cy"));

    // "Refresh every 5 minutes": nothing before the fifth minute.
    server.set("/b", page("text/html", &sales_page(&[("Dee", "1")])));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    for _ in 0..4 {
        app.update_in(cx, |app, window, cx| import::minute_passed(app, window, cx));
    }
    redraw(cx);
    assert_eq!(value(&app, cx, "A2"), text("Bea"));
    app.update_in(cx, |app, window, cx| import::minute_passed(app, window, cx));
    redraw(cx);
    assert_eq!(value(&app, cx, "A2"), text("Dee"));

    // Remove asks first; the data stays, the query is gone, and comes
    // back with Undo.
    run(&app, cx, Cmd::Connections);
    front::<connections::ConnectionsDialog>(&app, cx).update_in(cx, |list, window, cx| list.remove(window, cx));
    redraw(cx);
    assert!(alert_text(&app, cx).contains("Removing connection(s)"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.sheets[1].text_imports.is_empty()));
    assert_eq!(value(&app, cx, "A2"), text("Dee"));
    assert_eq!(modal_count(&app, cx), 1);
    assert!(!app.update(cx, |app, _| crate::features::Command::enabled(Cmd::RangeProperties, app)));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[1].text_imports.len()), 1);
}

#[gpui::test]
fn edit_query_and_refresh_on_open(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/a", page("text/html", &sales_page(&[("Ann", "3")])));
    from_web(&app, cx, &server.url("/a"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    // Edit Query: the same address, another table of the page.
    let query = app.update(cx, |app, _| import::QueryRef::of(app, 1, 0)).unwrap();
    let address = server.url("/a");
    app.update_in(cx, |app, window, cx| {
        navigator::open_web(app, &address, Some(query), window, cx);
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let navigator = front::<navigator::Navigator>(&app, cx);
    navigator.update_in(cx, |navigator, window, cx| {
        navigator.select(1, cx);
        navigator.load(false, window, cx);
    });
    redraw(cx);
    assert_eq!(value(&app, cx, "A2"), text("Ann"));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Edit Query"));
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[1].text_imports.len()), 1);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    assert_eq!(value(&app, cx, "A2"), Value::Empty);

    // A file that refreshes on opening: nothing is fetched before the
    // user agrees.
    edit(&app, cx, |wb| wb.update_import(1, 0, |import| import.props.refresh_on_open = true));
    let bytes = app.update(cx, |app, _| opensheet_io::write_xlsx(&app.wb, std::io::Cursor::new(Vec::new())).unwrap().into_inner());
    let wb = opensheet_io::read_xlsx(std::io::Cursor::new(bytes), opensheet_engine::Locale::en()).unwrap();
    server.set("/a", page("text/html", &sales_page(&[("Zed", "0")]).replace("Menu", "Carte")));
    app.update(cx, |app, cx| app.set_workbook(wb, None, "Opened".into(), cx));
    redraw(cx);
    redraw(cx);
    assert!(alert_text(&app, cx).contains("connects to an external data source"), "{}", alert_text(&app, cx));
    app.update(cx, |app, cx| {
        app.sheet = 1;
        cx.notify();
    });
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Carte"));
}

#[gpui::test]
fn interval_refresh_waits_for_consent(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/a", page("text/html", &sales_page(&[("Ann", "3")])));
    from_web(&app, cx, &server.url("/a"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    // The interval is the workbook's, not something just switched on in
    // Connection Properties, so opening asks before any clock starts.
    edit(&app, cx, |wb| wb.update_import(1, 0, |import| import.props.refresh_minutes = 1));
    let bytes = app.update(cx, |app, _| opensheet_io::write_xlsx(&app.wb, std::io::Cursor::new(Vec::new())).unwrap().into_inner());
    let reopen = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        let wb = opensheet_io::read_xlsx(std::io::Cursor::new(bytes.clone()), opensheet_engine::Locale::en()).unwrap();
        app.update(cx, |app, cx| app.set_workbook(wb, None, "Opened".into(), cx));
        redraw(cx);
        redraw(cx);
    };
    server.set("/a", page("text/html", &sales_page(&[("Zed", "0")]).replace("Menu", "Carte")));
    reopen(&app, cx);
    assert!(alert_text(&app, cx).contains("connects to an external data source"), "{}", alert_text(&app, cx));
    assert!(app.update(cx, |app, _| app.ext::<import::Watch>().is_none_or(|watch| !watch.timer_running())));
    app.update_in(cx, |app, window, cx| import::minute_passed(app, window, cx));
    // Decline: the timer stays stopped, and a minute changes nothing.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert!(app.update(cx, |app, _| app.ext::<import::Watch>().is_some_and(|watch| !watch.timer_running())));
    redraw(cx);
    assert!(!alert_text(&app, cx).contains("external data source"));
    app.update(cx, |app, cx| {
        app.sheet = 1;
        cx.notify();
    });
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    app.update_in(cx, |app, window, cx| import::minute_passed(app, window, cx));
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    // Refresh by hand still reads the address.
    run(&app, cx, Cmd::RefreshAll);
    assert_eq!(value(&app, cx, "A1"), text("Carte"));

    // Opened again, and agreed to: the clock runs, and the next minute refreshes.
    server.set("/a", page("text/html", &sales_page(&[("Bea", "1")]).replace("Menu", "Plat")));
    reopen(&app, cx);
    assert!(alert_text(&app, cx).contains("connects to an external data source"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.ext::<import::Watch>().is_some_and(|watch| watch.timer_running())));
    app.update(cx, |app, cx| {
        app.sheet = 1;
        cx.notify();
    });
    // Agreeing does not refresh at once; the interval does, a minute later.
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    app.update_in(cx, |app, window, cx| import::minute_passed(app, window, cx));
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Plat"));
}

#[gpui::test]
fn workbook_web_query_does_not_read_a_local_file(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let server = Server::start();
    server.set("/a", page("text/html", &sales_page(&[("Ann", "3")])));
    from_web(&app, cx, &server.url("/a"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    let secret = text_file("secret", "TOP-SECRET\n");
    // As a workbook would: the web query's address is a local file the
    // user never picked. Refresh must not read it.
    edit(&app, cx, |wb| {
        wb.update_import(1, 0, |import| import.path = super::sources::file_url(&secret));
    });
    run(&app, cx, Cmd::RefreshAll);
    assert!(alert_text(&app, cx).contains("local file"), "{}", alert_text(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    assert_eq!(modal_count(&app, cx), 1);
    edit(&app, cx, |wb| {
        wb.update_import(1, 0, |import| import.path = secret.to_string_lossy().into_owned());
    });
    run(&app, cx, Cmd::RefreshAll);
    assert!(alert_text(&app, cx).contains("local file"), "{}", alert_text(&app, cx));
    assert_eq!(value(&app, cx, "A1"), text("Menu"));
    std::fs::remove_dir_all(secret.parent().unwrap()).ok();
}
