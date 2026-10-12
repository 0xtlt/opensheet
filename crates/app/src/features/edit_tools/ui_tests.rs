//! The editing tools driven in GPUI's headless window: commands, keys,
//! typing and clicks on the dialogs. What these tests cannot tell is how
//! things look.

use super::*;
use crate::app::PopupKind;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::{Rect, Value, Workbook};

type Cell = (u32, u32);

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

fn enter(wb: &mut Workbook, cells: &[(&str, &str)]) {
    for (name, text) in cells {
        let at = Rect::parse(name).unwrap();
        wb.set_cell_text(0, at.r0, at.c0, text).unwrap();
    }
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    let rect = Rect::parse(range).unwrap();
    app.update(cx, |app, cx| app.select_rect(rect, cx));
    redraw(cx);
}

fn active(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Cell {
    app.update(cx, |app, _| app.active())
}

fn value(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Value {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.value_at(app.sheet, at.r0, at.c0))
}

fn text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> String {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.edit_text(app.sheet, at.r0, at.c0))
}

/// The centre of something a view named for the tests.
fn middle(cx: &mut VisualTestContext, name: &'static str) -> Point<Pixels> {
    cx.debug_bounds(name).unwrap_or_else(|| panic!("{name} is not drawn")).center()
}

fn press(cx: &mut VisualTestContext, name: &'static str) {
    let at = middle(cx, name);
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

/// How many dialogs and message boxes are open.
fn dialogs(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.modals.len())
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

/// The labels of the menu that is open, separators left out.
fn menu(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<String> {
    app.update(cx, |app, _| match app.popup.as_ref().map(|popup| &popup.kind) {
        Some(PopupKind::Menu(entries)) => {
            entries.iter().filter(|entry| !entry.separator).map(|entry| entry.label.to_string()).collect()
        }
        _ => Vec::new(),
    })
}

// --------------------------------------------------------- find and replace

#[gpui::test]
fn the_replace_dialog_finds_and_replaces(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("A1", "tea"), ("B2", "Teapot"), ("A3", "=LEN(\"tea\")"), ("C4", "coffee")]));
    // Control-H opens the dialog on its Replace tab, in "Find what".
    cx.simulate_keystrokes("ctrl-h");
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_input("tea");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("mate");
    // Return is Find Next: from A1 the next cell holding the text.
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&app, cx), (1, 1));
    assert_eq!(dialogs(&app, cx), 1);
    // Replace changes that cell and goes on to the next one.
    press(cx, "find-replace-one");
    assert_eq!(value(&app, cx, "B2"), Value::text("matepot"));
    assert_eq!(active(&app, cx), (2, 0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Replace"));
    // Replace All does the rest in one step and says how many.
    press(cx, "find-replace-all");
    assert_eq!(value(&app, cx, "A1"), Value::text("mate"));
    assert_eq!(text(&app, cx, "A3"), "=LEN(\"mate\")");
    assert_eq!(value(&app, cx, "A3"), Value::Number(4.0));
    assert_eq!(dialogs(&app, cx), 2, "the message over the dialog");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 1);
    press(cx, "find-close");
    assert_eq!(dialogs(&app, cx), 0);
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(value(&app, cx, "A1"), Value::text("tea"));
    assert_eq!(text(&app, cx, "A3"), "=LEN(\"tea\")");
    assert_eq!(value(&app, cx, "B2"), Value::text("matepot"));
    // The dialog remembers what was typed.
    run(&app, cx, Cmd::Replace);
    let kept = app.update(cx, |app, _| find::choices(app));
    assert_eq!((kept.find.as_str(), kept.replace.as_str()), ("tea", "mate"));
}

#[gpui::test]
fn find_says_when_there_is_nothing_and_keeps_to_the_selection(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("A1", "x"), ("B2", "x"), ("B3", "x"), ("D5", "x")]));
    run(&app, cx, Cmd::Find);
    cx.simulate_input("zzz");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 2, "Excel's message");
    cx.simulate_keystrokes("enter");
    press(cx, "find-close");
    // With several cells selected the search stays among them, and the
    // selection stays.
    select(&app, cx, "B1:B4");
    let choices = find::Choices {
        find: "x".into(),
        ..Default::default()
    };
    for expected in [(1, 1), (2, 1), (1, 1)] {
        app.update_in(cx, |app, window, cx| assert!(find::find_next(app, &choices, false, window, cx)));
        assert_eq!(active(&app, cx), expected);
        assert_eq!(app.update(cx, |app, _| app.sel().ranges.clone()), vec![Rect::parse("B1:B4").unwrap()]);
    }
}

#[gpui::test]
fn replace_all_spares_locked_cells_and_refuses_broken_formulas(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "old"), ("B1", "old"), ("C1", "=SUM(A2:A3)")]);
        wb.update_style(0, Rect::parse("B1").unwrap(), &|s| s.locked = false);
    });
    // A formula that would no longer be one: nothing changes.
    let broken = find::Choices {
        find: ")".into(),
        ..Default::default()
    };
    app.update_in(cx, |app, window, cx| find::replace_all(app, &broken, window, cx));
    assert_eq!(text(&app, cx, "C1"), "=SUM(A2:A3)");
    assert_eq!(active(&app, cx), (0, 2), "the cell at fault is shown");
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    // On a protected sheet only the unlocked cell is replaced.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    let choices = find::Choices {
        find: "old".into(),
        replace: "new".into(),
        ..Default::default()
    };
    app.update_in(cx, |app, window, cx| find::replace_all(app, &choices, window, cx));
    assert_eq!(value(&app, cx, "A1"), Value::text("old"));
    assert_eq!(value(&app, cx, "B1"), Value::text("new"));
    cx.simulate_keystrokes("enter");
    // Nothing left but the locked cell: the message of protected sheets.
    app.update_in(cx, |app, window, cx| find::replace_all(app, &choices, window, cx));
    assert_eq!(value(&app, cx, "A1"), Value::text("old"));
    assert_eq!(dialogs(&app, cx), 1);
    // The Replace button asks the protection too.
    cx.simulate_keystrokes("enter");
    select(&app, cx, "A1");
    app.update_in(cx, |app, window, cx| find::replace(app, &choices, window, cx));
    assert_eq!(value(&app, cx, "A1"), Value::text("old"));
}

#[gpui::test]
fn the_find_bar_searches_like_the_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("A1", "=2+3"), ("A2", "5")]));
    // Formulas are what is looked in until the dialog says otherwise.
    app.update(cx, |app, cx| find_bar_next(app, "5", cx));
    assert_eq!(active(&app, cx), (1, 0));
    app.update(cx, |app, cx| find_bar_next(app, "5", cx));
    assert_eq!(active(&app, cx), (1, 0));
    app.update(cx, |app, _| app.ext_mut::<find::Choices>().options.look_in = opensheet_engine::find::LookIn::Values);
    app.update(cx, |app, cx| find_bar_next(app, "5", cx));
    assert_eq!(active(&app, cx), (0, 0));
}

#[gpui::test]
fn find_and_select_reaches_the_cells_of_a_kind(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("A1", "1"), ("A2", "=A1+1"), ("B2", "text")]));
    run(&app, cx, Cmd::SelectFormulas);
    assert_eq!(app.update(cx, |app, _| app.sel().ranges.clone()), vec![Rect::parse("A2").unwrap()]);
    run(&app, cx, Cmd::SelectConstants);
    let mut found = app.update(cx, |app, _| app.sel().ranges.clone());
    found.sort_by_key(|range| (range.r0, range.c0));
    assert_eq!(found, vec![Rect::parse("A1").unwrap(), Rect::parse("B2").unwrap()]);
}

// ------------------------------------------------------------ paste special

fn copy(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    select(app, cx, range);
    app.update(cx, |app, cx| app.copy(false, cx));
}

#[gpui::test]
fn the_paste_special_dialog_transposes_and_operates(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("A1", "1"), ("B1", "2"), ("C1", "=A1+B1"), ("E1", "x")]));
    copy(&app, cx, "A1:C1");
    select(&app, cx, "E1");
    // Control-Command-V opens the dialog; Transpose, then Return.
    cx.simulate_keystrokes("ctrl-cmd-v");
    assert_eq!(dialogs(&app, cx), 1);
    press(cx, "paste-transpose");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 0);
    assert_eq!(value(&app, cx, "E2"), Value::Number(2.0));
    assert_eq!(text(&app, cx, "E3"), "=E1+E2");
    assert_eq!(app.update(cx, |app, _| app.sel().current()), Rect::parse("E1:E3").unwrap());
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Paste Special"));
    // One step to undo.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(value(&app, cx, "E1"), Value::text("x"));
    assert_eq!(value(&app, cx, "E3"), Value::Empty);
}

#[gpui::test]
fn the_paste_drop_down_pastes_each_kind(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "=1/4")]);
        wb.update_style(0, Rect::parse("A1").unwrap(), &|s| {
            s.num_fmt = "0%".into();
            s.font.bold = true;
        });
    });
    // Nothing copied: the choices are greyed out.
    assert!(!app.update(cx, |app, _| Cmd::PasteValues.enabled(app)));
    copy(&app, cx, "A1");
    run(&app, cx, Cmd::PasteMenu);
    let entries = menu(&app, cx);
    assert_eq!(entries.first().map(String::as_str), Some("Paste"));
    assert_eq!(entries.last().map(String::as_str), Some("Paste Special…"));
    assert_eq!(entries.len(), 13);
    app.update(cx, |app, cx| {
        app.popup = None;
        cx.notify();
    });
    for (cell, cmd) in [
        ("C1", Cmd::PasteValues),
        ("D1", Cmd::PasteFormulas),
        ("E1", Cmd::PasteValuesNumbers),
        ("F1", Cmd::PasteFormatting),
        ("G1", Cmd::PasteLink),
        ("H1", Cmd::PasteValuesSource),
    ] {
        select(&app, cx, cell);
        run(&app, cx, cmd);
    }
    assert_eq!(text(&app, cx, "C1"), "0.25");
    assert_eq!(text(&app, cx, "D1"), "=1/4");
    assert_eq!(text(&app, cx, "E1"), "25%");
    assert!(!app.update(cx, |app, _| app.wb.style_at(0, 0, 4).font.bold));
    assert_eq!(value(&app, cx, "F1"), Value::Empty);
    assert!(app.update(cx, |app, _| app.wb.style_at(0, 0, 5).font.bold));
    assert_eq!(text(&app, cx, "G1"), "=$A$1");
    assert_eq!(text(&app, cx, "H1"), "25%");
    assert!(app.update(cx, |app, _| app.wb.style_at(0, 0, 7).font.bold));
    // A protected sheet refuses, whatever the kind.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    select(&app, cx, "C3");
    run(&app, cx, Cmd::PasteValues);
    assert_eq!(value(&app, cx, "C3"), Value::Empty);
    assert_eq!(dialogs(&app, cx), 1, "the message of protected sheets");
}

#[gpui::test]
fn text_from_another_application_is_offered_as_text(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("a\tb\n1\t2".into()));
    select(&app, cx, "B2");
    run(&app, cx, Cmd::PasteSpecial);
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 0);
    assert_eq!(value(&app, cx, "C2"), Value::text("b"));
    assert_eq!(value(&app, cx, "B3"), Value::Number(1.0));
}
