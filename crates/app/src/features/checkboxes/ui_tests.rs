//! Checkboxes in cells driven in GPUI's headless window: the command, a
//! click on the box, Space, and undo.

use super::*;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::Value;

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// Insert > Checkbox on a range.
fn insert(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, rect: Rect) {
    app.update_in(cx, |app, window, cx| {
        app.select_rect(rect, cx);
        app.run_command(Cmd::Insert, window, cx);
    });
    redraw(cx);
}

fn value(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) -> Value {
    app.update(cx, |app, _| app.wb.value_at(0, row, col))
}

/// Window position of the middle of the box of a checkbox cell.
fn box_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let style = app.wb.style_at(0, row, col);
        let b = crate::grid::checkbox_box(style, layout.rect_px(Rect::cell(row, col)), layout.zoom);
        layout.to_window((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
    })
}

#[gpui::test]
fn insert_makes_empty_cells_false_in_one_undo_step(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    insert(&app, cx, Rect::new(0, 0, 2, 0));
    assert_eq!(value(&app, cx, 0, 0), Value::Bool(false));
    assert_eq!(
        app.update(cx, |app, _| app.wb.checkbox_at(0, 2, 0)),
        Some(false)
    );
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
    assert_eq!(value(&app, cx, 0, 0), Value::Empty);
    assert!(!app.update(cx, |app, _| app.wb.style_at(0, 0, 0).checkbox));
}

#[gpui::test]
fn a_click_on_the_box_ticks_it_and_undo_takes_it_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    insert(&app, cx, Rect::new(0, 0, 2, 0));
    app.update(cx, |app, cx| app.select_cell(5, 5, cx));
    redraw(cx);
    let at = box_of(&app, cx, 1, 0);
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(value(&app, cx, 1, 0), Value::Bool(true));
    // The click selects the cell too.
    assert_eq!(app.update(cx, |app, _| app.sel().active), (1, 0));
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(value(&app, cx, 1, 0), Value::Bool(false));
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
    assert_eq!(value(&app, cx, 1, 0), Value::Bool(true));
}

#[gpui::test]
fn space_turns_over_the_checkboxes_of_the_selection(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    insert(&app, cx, Rect::new(0, 0, 2, 0));
    app.update(cx, |app, cx| app.select_rect(Rect::new(0, 0, 2, 0), cx));
    redraw(cx);
    cx.simulate_input(" ");
    redraw(cx);
    for row in 0..3 {
        assert_eq!(value(&app, cx, row, 0), Value::Bool(true));
    }
    assert!(
        app.update(cx, |app, _| app.editing.is_none()),
        "no edit started"
    );
    // Elsewhere Space starts typing as usual.
    app.update(cx, |app, cx| app.select_cell(4, 4, cx));
    redraw(cx);
    cx.simulate_input(" ");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.editing.is_some()));
}
