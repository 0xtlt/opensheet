//! The limited grid driven in GPUI's headless window: the command, the +
//! buttons and the clicks around the grid.

use super::*;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::{Rect, Value};

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

fn toggle(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::Toggle, window, cx)
    });
    redraw(cx);
}

/// Rows and columns the grid lays out.
fn counts(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> (u32, u32) {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        (layout.rows.count(), layout.cols.count())
    })
}

/// Window position of the middle of a box of the grid element.
fn at(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    b: (f32, f32, f32, f32),
) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        layout.to_window((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
    })
}

fn button(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, col: bool) -> Point<Pixels> {
    let b = app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let (c, r) = buttons(&layout);
        if col { c } else { r }
    });
    at(app, cx, b)
}

#[gpui::test]
fn the_command_limits_the_grid_and_gives_it_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    assert_eq!(counts(&app, cx), (MAX_ROWS, MAX_COLS));
    toggle(&app, cx);
    assert!(app.update(cx, |app, _| Cmd::Toggle.checked(app)));
    assert_eq!(counts(&app, cx), (FIRST_ROWS, FIRST_COLS));
    toggle(&app, cx);
    assert!(!app.update(cx, |app, _| Cmd::Toggle.checked(app)));
    assert_eq!(counts(&app, cx), (MAX_ROWS, MAX_COLS));
}

#[gpui::test]
fn the_grid_reaches_the_content_and_the_active_cell(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        app.wb.set_value(0, 29, 7, Value::Number(1.0));
        app.commit("setup", cx);
    });
    toggle(&app, cx);
    assert_eq!(counts(&app, cx), (30, 8));
    // Moving past the edge makes the grid grow with the active cell.
    app.update(cx, |app, cx| app.select_cell(40, 2, cx));
    redraw(cx);
    assert_eq!(counts(&app, cx), (41, 8));
    // Whole columns do not stretch it to the last row.
    app.update(cx, |app, cx| {
        app.select_rect(Rect::new(0, 1, MAX_ROWS - 1, 1), cx)
    });
    redraw(cx);
    assert_eq!(counts(&app, cx), (30, 8));
}

#[gpui::test]
fn the_plus_buttons_add_and_option_click_removes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    toggle(&app, cx);
    let plus_col = button(&app, cx, true);
    cx.simulate_click(plus_col, Modifiers::none());
    redraw(cx);
    assert_eq!(counts(&app, cx), (FIRST_ROWS, FIRST_COLS + 1));
    let plus_row = button(&app, cx, false);
    cx.simulate_click(plus_row, Modifiers::none());
    redraw(cx);
    // The button moved down with the new row.
    let plus_row = button(&app, cx, false);
    cx.simulate_click(plus_row, Modifiers::none());
    redraw(cx);
    assert_eq!(counts(&app, cx), (FIRST_ROWS + 2, FIRST_COLS + 1));
    let plus_col = button(&app, cx, true);
    cx.simulate_click(plus_col, Modifiers::alt());
    redraw(cx);
    assert_eq!(counts(&app, cx), (FIRST_ROWS + 2, FIRST_COLS));
    // Nothing of this is an edit of the workbook.
    assert!(!app.update(cx, |app, _| app.wb.can_undo()));
}

#[gpui::test]
fn a_click_past_the_grid_selects_nothing(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    toggle(&app, cx);
    let outside = app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let (end_x, end_y) = layout.cells_end();
        layout.to_window(end_x + 60.0, end_y + 60.0)
    });
    cx.simulate_click(outside, Modifiers::none());
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.sel().active), (0, 0));
}

#[gpui::test]
fn page_layout_view_shows_the_whole_grid(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    toggle(&app, cx);
    app.update(cx, |app, cx| app.set_view_mode(ViewMode::PageLayout, cx));
    redraw(cx);
    assert_eq!(counts(&app, cx), (MAX_ROWS, MAX_COLS));
}
