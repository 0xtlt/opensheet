//! Hidden sheets driven as a user would: the menu of a tab, the Unhide
//! dialog, the keys that walk the tabs, and what refuses.

use super::*;
use crate::app::PopupKind;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::workbook::SheetVisibility::{Hidden, VeryHidden, Visible};

/// A window on a workbook of three sheets, the first in front.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    app.update(cx, |app, cx| {
        app.wb.add_sheet(1);
        app.wb.add_sheet(2);
        app.wb.set_cell_text(1, 0, 0, "40").unwrap();
        app.wb.set_cell_text(0, 0, 0, "=Sheet2!A1+2").unwrap();
        app.commit("setup", cx);
    });
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// Open the menu of a tab, as a right click on it does.
fn tab_menu(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, index: usize) {
    app.update_in(cx, |app, window, cx| {
        app.switch_sheet(index, window, cx);
        app.open_sheet_menu(index, point(px(200.), px(500.)), cx);
    });
    redraw(cx);
}

/// The entry of the open menu with this label: whether it is greyed.
fn greyed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, label: &str) -> bool {
    app.update(cx, |app, _| match app.popup.as_ref().map(|popup| &popup.kind) {
        Some(PopupKind::Menu(entries)) => entries.iter().find(|e| e.label.as_ref() == label).expect("the entry").disabled,
        _ => panic!("no menu is open"),
    })
}

fn choose(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, label: &str) {
    app.update_in(cx, |app, window, cx| {
        let Some(PopupKind::Menu(entries)) = app.popup.take().map(|popup| popup.kind) else {
            panic!("no menu is open");
        };
        let entry = entries.iter().find(|entry| entry.label.as_ref() == label).expect("the entry");
        assert!(!entry.disabled, "{label} is greyed");
        (entry.action.clone().expect("an action"))(app, window, cx);
    });
    redraw(cx);
}

fn states(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<SheetVisibility> {
    app.update(cx, |app, _| app.wb.sheets.iter().map(|sheet| sheet.visibility).collect())
}

fn front(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.sheet)
}

fn modals(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.modals.len())
}

#[gpui::test]
fn a_sheet_is_hidden_from_its_tab_and_comes_back_through_the_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    tab_menu(&app, cx, 1);
    // Nothing is hidden yet: Unhide… has nothing to offer.
    assert!(greyed(&app, cx, "Unhide…") && !greyed(&app, cx, "Hide"));
    choose(&app, cx, "Hide");
    assert_eq!(states(&app, cx), [Visible, Hidden, Visible]);
    // The tab is gone and the next one came in front.
    assert_eq!((front(&app, cx), app.update(cx, |app, _| tabs(app))), (2, vec![0, 2]));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Hide Sheet"));
    // Formulas go on reading the hidden sheet.
    assert_eq!(app.update(cx, |app, _| app.wb.display_text(0, 0, 0).text.to_string()), "42");

    // The keys that walk the tabs step over it, both ways.
    cx.simulate_keystrokes("ctrl-pageup");
    assert_eq!(front(&app, cx), 0);
    cx.simulate_keystrokes("ctrl-pageup");
    assert_eq!(front(&app, cx), 0);
    cx.simulate_keystrokes("ctrl-pagedown");
    assert_eq!(front(&app, cx), 2);
    cx.simulate_keystrokes("ctrl-pagedown");
    assert_eq!(front(&app, cx), 2);
    // Nothing brings it in front.
    app.update_in(cx, |app, window, cx| app.switch_sheet(1, window, cx));
    assert_eq!(front(&app, cx), 2);

    // Unhide…: the one hidden sheet is selected, Return shows it.
    tab_menu(&app, cx, 0);
    choose(&app, cx, "Unhide…");
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!((modals(&app, cx), states(&app, cx)), (0, vec![Visible, Visible, Visible]));
    assert_eq!(front(&app, cx), 1);
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Unhide Sheet"));

    // Undo hides it again and the window leaves it; undo once more and
    // it is back.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!((states(&app, cx), front(&app, cx)), (vec![Visible, Hidden, Visible], 2));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(states(&app, cx), [Visible, Visible, Visible]);
    cx.simulate_keystrokes("cmd-y");
    assert_eq!(states(&app, cx), [Visible, Hidden, Visible]);
    // Escape leaves the dialog without a change.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Unhide, window, cx));
    redraw(cx);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!((modals(&app, cx), states(&app, cx)), (0, vec![Visible, Hidden, Visible]));
}

#[gpui::test]
fn several_sheets_are_unhidden_at_once_and_very_hidden_ones_are_not_offered(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        assert!(hide(app, 0, cx) && hide(app, 1, cx));
        app.wb.add_sheet(3);
        app.wb.set_sheet_visibility(3, VeryHidden);
        app.commit("very hidden", cx);
    });
    assert_eq!(front(&app, cx), 2);
    let offered = app.update(cx, |app, _| hidden_sheets(app));
    assert_eq!(offered, [(0, "Sheet1".to_string()), (1, "Sheet2".to_string())]);
    let dialog = app.update_in(cx, |app, window, cx| app.open_modal(window, cx, |modal, _, _| UnhideDialog::new(modal, offered)));
    redraw(cx);
    dialog.update_in(cx, |dialog, window, cx| {
        assert_eq!(dialog.selected, [0]);
        dialog.selected = vec![0, 1];
        dialog.ok(&(), window, cx);
    });
    redraw(cx);
    assert_eq!(states(&app, cx), [Visible, Visible, Visible, VeryHidden]);
    assert_eq!((front(&app, cx), modals(&app, cx)), (0, 0));
    // One step for both.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(states(&app, cx), [Hidden, Hidden, Visible, VeryHidden]);

    // With very hidden sheets only, Unhide… is greyed.
    app.update(cx, |app, cx| {
        assert!(unhide(app, &[0, 1], cx));
        assert!(!Cmd::Unhide.enabled(app));
    });
    // The last tab of the strip has no neighbour to move past.
    tab_menu(&app, cx, 2);
    assert!(greyed(&app, cx, "Move Right") && !greyed(&app, cx, "Move Left"));
}

#[gpui::test]
fn the_last_visible_sheet_stays(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| assert!(hide(app, 0, cx) && hide(app, 2, cx)));
    assert_eq!(front(&app, cx), 1);
    // Hide: Excel's message, and nothing changes.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Hide, window, cx));
    redraw(cx);
    assert_eq!((modals(&app, cx), states(&app, cx)), (1, vec![Hidden, Visible, Hidden]));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    // Delete: the same message; the entry of the tab menu is greyed.
    app.update(cx, |app, cx| app.delete_sheet(1, cx));
    redraw(cx);
    assert_eq!((modals(&app, cx), app.update(cx, |app, _| app.wb.sheets.len())), (1, 3));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    tab_menu(&app, cx, 1);
    assert!(greyed(&app, cx, "Delete") && greyed(&app, cx, "Move Left") && greyed(&app, cx, "Move Right"));
    // A hidden sheet can be deleted from under the last tab.
    app.update(cx, |app, cx| {
        app.popup = None;
        app.delete_sheet(0, cx);
    });
    assert_eq!((states(&app, cx), front(&app, cx)), (vec![Visible, Hidden], 0));
}

#[gpui::test]
fn a_protected_structure_refuses_both(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        assert!(hide(app, 2, cx));
        app.wb.protect_workbook(true, false, "");
        app.refresh(cx);
    });
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Hide, window, cx));
    redraw(cx);
    assert_eq!((modals(&app, cx), states(&app, cx)), (1, vec![Visible, Visible, Hidden]));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // Unhide… says so instead of opening its dialog.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Unhide, window, cx));
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!((modals(&app, cx), states(&app, cx)), (0, vec![Visible, Visible, Hidden]));
}

#[gpui::test]
fn moving_a_tab_steps_over_hidden_sheets(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| assert!(hide(app, 1, cx)));
    let names = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| app.wb.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
    };
    tab_menu(&app, cx, 2);
    choose(&app, cx, "Move Left");
    assert_eq!(names(&app, cx), ["Sheet3", "Sheet1", "Sheet2"]);
    assert_eq!((front(&app, cx), app.update(cx, |app, _| tabs(app))), (0, vec![0, 1]));
}
