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
