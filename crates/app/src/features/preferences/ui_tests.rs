//! Preferences driven in GPUI's headless window: the window, its panes,
//! and what the settings change in the application.

use super::window::{Pane, PreferencesWindow};
use super::*;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::{IterativeCalc, Value};

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

/// Open the window on a pane.
fn pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, pane: Pane) -> Entity<PreferencesWindow> {
    let view = app.update_in(cx, |app, window, cx| window::open_on(app, Some(pane), window, cx));
    redraw(cx);
    view
}

fn type_in(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: (u32, u32), text: &str) {
    app.update(cx, |app, cx| app.select_cell(cell.0, cell.1, cx));
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    redraw(cx);
}

fn value(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: (u32, u32)) -> Value {
    app.update(cx, |app, _| app.wb.value_at(0, cell.0, cell.1))
}

#[gpui::test]
fn the_shortcut_opens_the_page_of_panes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    cx.simulate_keystrokes("cmd-,");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 1);
    // Escape closes it, as its Close button does.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 0);
}

#[gpui::test]
fn iterative_calculation_is_turned_on_in_the_calculation_pane(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // A1 adds one to itself: a circular reference, shown as 0.
    type_in(&app, cx, (0, 0), "=A1+1");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.circular));
    let view = pane(&app, cx, Pane::Calculation);
    view.update(cx, |view, cx| {
        view.max_iterations.update(cx, |field, cx| field.set_text("25", cx));
        view.max_change.update(cx, |field, cx| field.set_text("0", cx));
        view.set_iteration(true, cx);
    });
    redraw(cx);
    app.update(cx, |app, _| {
        let wanted = IterativeCalc {
            enabled: true,
            max_iterations: 25,
            max_change: 0.0,
        };
        assert_eq!(app.wb.iterative_calc(), wanted);
        // Twenty-five turns of A1+1.
        assert_eq!(app.wb.value_at(0, 0, 0), Value::Number(25.0));
        assert!(app.wb.modified);
    });
    // A value that makes no sense is refused and the field shows the
    // stored one again.
    view.update(cx, |view, cx| {
        view.max_iterations.update(cx, |field, cx| field.set_text("0", cx));
        view.go(None, cx);
        assert_eq!(view.max_iterations.read(cx).text(), "25");
        assert_eq!(view.pane, None);
    });
    // Manual: a change waits; back to Automatic it is calculated.
    view.update(cx, |view, cx| view.set_calc_mode(CalcMode::Manual, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    type_in(&app, cx, (0, 1), "5");
    type_in(&app, cx, (0, 2), "=B1*2");
    type_in(&app, cx, (0, 1), "6");
    assert_eq!(value(&app, cx, (0, 2)), Value::Number(10.0));
    let view = pane(&app, cx, Pane::Calculation);
    view.update(cx, |view, cx| view.set_calc_mode(CalcMode::Automatic, cx));
    assert_eq!(value(&app, cx, (0, 2)), Value::Number(12.0));
}

#[gpui::test]
fn the_edit_pane_changes_what_return_does(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    type_in(&app, cx, (0, 0), "1");
    assert_eq!(app.update(cx, |app, _| app.active()), (1, 0));
    settings::update(|settings| settings.move_direction = Direction::Right);
    type_in(&app, cx, (0, 0), "2");
    assert_eq!(app.update(cx, |app, _| app.active()), (0, 1));
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(app.update(cx, |app, _| app.active()), (0, 0));
    settings::update(|settings| settings.move_after_return = false);
    type_in(&app, cx, (0, 0), "3");
    assert_eq!(app.update(cx, |app, _| (app.active(), app.is_editing())), ((0, 0), false));
    assert_eq!(value(&app, cx, (0, 0)), Value::Number(3.0));
    // A fixed decimal point, set from the pane.
    let view = pane(&app, cx, Pane::Edit);
    view.update(cx, |view, cx| {
        view.places.update(cx, |field, cx| field.set_text("3", cx));
        view.commit_fields(cx);
    });
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(settings::get().decimal_places, 3);
    settings::update(|settings| settings.fixed_decimal = true);
    type_in(&app, cx, (2, 0), "1234");
    assert_eq!(value(&app, cx, (2, 0)), Value::Number(1.234));
    // A point that is typed wins, and text stays text.
    type_in(&app, cx, (3, 0), "12.5");
    type_in(&app, cx, (4, 0), "abc");
    assert_eq!(value(&app, cx, (3, 0)), Value::Number(12.5));
    assert_eq!(value(&app, cx, (4, 0)), Value::text("abc"));
    // One undo step per entry all the same.
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    assert_eq!(value(&app, cx, (2, 0)), Value::Empty);
}

#[gpui::test]
fn the_general_pane_names_the_user_and_shapes_new_workbooks(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let view = pane(&app, cx, Pane::General);
    view.update(cx, |view, cx| {
        view.user_name.update(cx, |field, cx| field.set_text("  Ada Lovelace ", cx));
        view.sheets.update(cx, |field, cx| field.set_text("4", cx));
        view.font_size.update(cx, |field, cx| field.set_text("500", cx));
        view.commit_fields(cx);
        // 500 points is no font size.
        assert_eq!(view.font_size.read(cx).text(), "11");
    });
    let stored = settings::get();
    assert_eq!((stored.user_name.as_str(), stored.sheets_in_new_workbook, stored.default_font_size), ("Ada Lovelace", 4, 11.0));
    assert_eq!(user_name().as_deref(), Some("Ada Lovelace"));
    let mut wb = Workbook::new(opensheet_engine::Locale::en());
    prepare_new_workbook(&mut wb);
    assert_eq!(wb.sheets.len(), 4);
}

#[gpui::test]
fn sheet_tabs_and_error_corners_follow_their_boxes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    type_in(&app, cx, (0, 0), "=1/0");
    // The rule can be switched off: Error Checking then finds nothing.
    let errors = |app: &ExcelApp| crate::features::misc::error_cells(&app.wb, 0, &Default::default());
    assert_eq!(app.update(cx, |app, _| errors(app)), vec![(0, 0)]);
    settings::update(|settings| settings.check_error_values = false);
    assert!(app.update(cx, |app, _| errors(app)).is_empty());
    assert!(!checks_error_values());
    // The View pane holds the outline symbols of the sheet in front.
    let view = pane(&app, cx, Pane::View);
    view.update(cx, |view, cx| view.set_outline_symbols(false, cx));
    app.update(cx, |app, _| {
        assert!(!app.wb.sheets[0].outline.show_symbols);
        assert_eq!(app.wb.undo_label(), Some("Outline symbols"));
    });
    cx.simulate_keystrokes("escape");
    redraw(cx);
    cx.simulate_keystrokes("cmd-z");
    assert!(app.update(cx, |app, _| app.wb.sheets[0].outline.show_symbols));
    // The window still draws without its sheet tabs.
    settings::update(|settings| settings.show_sheet_tabs = false);
    app.update(cx, |_, cx| cx.notify());
    redraw(cx);
    assert!(!shows_sheet_tabs());
}
