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

#[gpui::test]
fn the_general_pane_names_the_folder_of_the_scripts(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let usual = crate::features::automate::scripts_folder();
    let view = pane(&app, cx, Pane::General);
    // Another folder is chosen: the scripts are looked for there.
    let other = usual.join("elsewhere");
    view.update(cx, |view, cx| view.set_scripts_folder(Some(other.clone()), cx));
    redraw(cx);
    assert_eq!(crate::features::automate::scripts_folder(), other);
    assert_eq!(settings::get().scripts_folder, other.to_string_lossy());
    // "Use Default" comes back to the folder of the application.
    view.update(cx, |view, cx| view.set_scripts_folder(None, cx));
    redraw(cx);
    assert_eq!(crate::features::automate::scripts_folder(), usual);
    assert!(settings::get().scripts_folder.is_empty());
}

// ------------------------------------------------------------- AutoCorrect

fn text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: (u32, u32)) -> String {
    app.update(cx, |app, _| app.wb.display_text(0, cell.0, cell.1).text)
}

/// Type into a cell without confirming.
fn begin(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: (u32, u32), typed: &str) {
    app.update(cx, |app, cx| app.select_cell(cell.0, cell.1, cx));
    cx.simulate_input(typed);
    redraw(cx);
}

fn editor_text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    app.update(cx, |app, cx| app.edit_text(cx))
}

#[gpui::test]
fn what_is_typed_is_corrected_at_the_end_of_each_word(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // In the cell editor, as the words end.
    begin(&app, cx, (0, 0), "teh cat (c) THe end. it");
    assert_eq!(editor_text(&app, cx), "the cat © The end. it");
    // The formula bar shows the same text.
    assert_eq!(app.update(cx, |app, cx| app.formula_input.read(cx).text().to_string()), "the cat © The end. it");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // The last word is corrected when the entry is confirmed, as a step
    // of its own: one Undo gives back what was typed.
    assert_eq!(text(&app, cx, (0, 0)), "the cat © The end. It");
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("AutoCorrect"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&app, cx, (0, 0)), "the cat © The end. it");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&app, cx, (0, 0)), "");
    // Formulas and numbers are left alone.
    type_in(&app, cx, (1, 0), "=LEN(\"teh \")");
    assert_eq!(value(&app, cx, (1, 0)), Value::Number(4.0));
    type_in(&app, cx, (2, 0), "recieve");
    assert_eq!(text(&app, cx, (2, 0)), "receive");
}

#[gpui::test]
fn undo_while_typing_takes_a_correction_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    begin(&app, cx, (0, 0), "see teh ");
    assert_eq!(editor_text(&app, cx), "see the ");
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(editor_text(&app, cx), "see teh ");
    // The word is then left alone, here and when the entry is confirmed.
    cx.simulate_input("map, teh");
    assert_eq!(editor_text(&app, cx), "see teh map, teh");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(text(&app, cx, (0, 0)), "see teh map, teh");
    // A second Undo with nothing to take back changes nothing.
    begin(&app, cx, (1, 0), "adn ");
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(editor_text(&app, cx), "adn ");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // The next entry starts afresh.
    type_in(&app, cx, (2, 0), "teh adn");
    assert_eq!(text(&app, cx, (2, 0)), "the and");
}

#[gpui::test]
fn the_autocorrect_pane_switches_each_correction(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let view = pane(&app, cx, Pane::AutoCorrect);
    // A word of the user's joins the list, in alphabetical order.
    view.update(cx, |view, cx| {
        view.replace_from.update(cx, |field, cx| field.set_text("osht", cx));
        view.replace_with.update(cx, |field, cx| field.set_text("OpenSheet", cx));
        view.add_replacement(cx);
        let list = view.replacements();
        let at = list.iter().position(|(from, _)| from == "osht").unwrap();
        assert!(list[at - 1].0.to_lowercase() < "osht".to_string() && list.iter().any(|(from, _)| from == "teh"));
        // "Replace" gives a listed word another replacement.
        view.replace_with.update(cx, |field, cx| field.set_text("Open Sheet", cx));
        view.add_replacement(cx);
        assert_eq!(view.replacements().iter().filter(|(from, _)| from == "osht").count(), 1);
        // A line of the list fills the two boxes; Delete takes it away.
        let teh = view.replacements().iter().position(|(from, _)| from == "teh").unwrap();
        view.show_replacement(teh, cx);
        assert_eq!(view.replace_with.read(cx).text(), "the");
        view.delete_replacement(cx);
        assert!(!view.replacements().iter().any(|(from, _)| from == "teh"));
    });
    redraw(cx);
    let stored = settings::get().replacements.expect("the list is now the user's");
    assert!(stored.contains(&("osht".to_string(), "Open Sheet".to_string())));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    type_in(&app, cx, (0, 0), "osht, teh end");
    assert_eq!(text(&app, cx, (0, 0)), "Open Sheet, teh end");
    // Each box switches its correction off.
    settings::update(|settings| {
        settings.replace_text = false;
        settings.correct_two_capitals = false;
        settings.capitalize_sentences = false;
        settings.capitalize_days = false;
    });
    type_in(&app, cx, (1, 0), "osht. THe monday (c)");
    assert_eq!(text(&app, cx, (1, 0)), "osht. THe monday (c)");
    // An address typed becomes a link, unless its box is cleared.
    type_in(&app, cx, (2, 0), "www.example.com");
    assert!(app.update(cx, |app, _| app.wb.hyperlink_at(0, 2, 0).is_some()));
    settings::update(|settings| settings.link_addresses = false);
    type_in(&app, cx, (3, 0), "www.example.com");
    assert!(app.update(cx, |app, _| app.wb.hyperlink_at(0, 3, 0).is_none()));
    assert_eq!(text(&app, cx, (3, 0)), "www.example.com");
}

#[gpui::test]
fn a_corrected_header_renames_its_table_column(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for (cell, typed) in [((0, 0), "Name"), ((0, 1), "Qty"), ((1, 0), "Pen"), ((1, 1), "3")] {
        type_in(&app, cx, cell, typed);
    }
    app.update(cx, |app, cx| {
        app.wb.add_table(0, opensheet_engine::Rect::new(0, 0, 1, 1), true).unwrap();
        app.commit("table", cx);
    });
    type_in(&app, cx, (0, 1), "recieve");
    app.update(cx, |app, _| {
        let table = &app.wb.sheets[0].tables[0];
        assert_eq!(table.columns[1].name, "receive");
    });
}

// ------------------------------------------------ Custom Lists and Tables

fn column(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, col: u32, rows: u32) -> Vec<String> {
    (0..rows).map(|row| text(app, cx, (row, col))).collect()
}

#[gpui::test]
fn the_lists_of_the_user_drive_the_fill_handle_and_the_sort_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let view = pane(&app, cx, Pane::CustomLists);
    view.update(cx, |view, cx| {
        // The built-in lists show their entries and cannot be replaced.
        view.choose_list(1, cx);
        assert_eq!(view.list_entries.read(cx).text(), "Sun\nMon\nTue\nWed\nThu\nFri\nSat");
        assert_eq!(view.own_list_at(), None);
        // NEW LIST: what is typed becomes a list, one entry per line.
        view.choose_list(0, cx);
        view.list_entries.update(cx, |area, cx| area.set_text("North\n East \n\nSouth\nnorth\nWest", cx));
        assert!(view.add_list(cx));
        assert_eq!(view.own_list_at(), Some(0));
        // The same list again is not a second list.
        view.choose_list(0, cx);
        view.list_entries.update(cx, |area, cx| area.set_text("North, East, South, West", cx));
        assert!(view.add_list(cx));
    });
    redraw(cx);
    let wanted = vec![vec!["North".to_string(), "East".to_string(), "South".to_string(), "West".to_string()]];
    assert_eq!(settings::get().custom_lists, wanted);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // The fill handle continues it, and the Sort dialog offers it: both
    // read the lists of the workbook.
    type_in(&app, cx, (0, 0), "South");
    app.update(cx, |app, cx| {
        app.wb.autofill(0, opensheet_engine::Rect::cell(0, 0), opensheet_engine::Rect::new(0, 0, 3, 0));
        app.commit("fill", cx);
        assert_eq!(app.wb.custom_lists().last(), wanted.last());
    });
    assert_eq!(column(&app, cx, 0, 4), ["South", "West", "North", "East"]);
    // An entry is edited: the list of the user is replaced.
    let view = pane(&app, cx, Pane::CustomLists);
    view.update(cx, |view, cx| {
        view.choose_list(5, cx);
        assert_eq!(view.list_entries.read(cx).text(), "North\nEast\nSouth\nWest");
        view.list_entries.update(cx, |area, cx| area.set_text("North\nEast\nSouth\nWest\nCentre", cx));
        assert!(view.add_list(cx));
        // An entry that starts with a digit is refused, with a message.
        view.choose_list(0, cx);
        view.list_entries.update(cx, |area, cx| area.set_text("1st\n2nd", cx));
        assert!(!view.add_list(cx));
    });
    redraw(cx);
    assert!(app.update(cx, |app, cx| app.front_alert(cx)).is_some());
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(settings::get().custom_lists.len(), 1);
    assert_eq!(settings::get().custom_lists[0].len(), 5);
    // Delete, once agreed, takes the list away from the workbook too.
    view.update(cx, |view, cx| {
        view.choose_list(5, cx);
        view.delete_list(cx);
        assert_eq!(view.list_at, 0);
    });
    assert!(settings::get().custom_lists.is_empty());
    assert_eq!(app.update(cx, |app, _| app.wb.custom_lists().len()), 4);
}

#[gpui::test]
fn lists_are_imported_from_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for (row, typed) in ["Low", "Medium", "12", "High", "Low"].iter().enumerate() {
        type_in(&app, cx, (row as u32, 2), typed);
    }
    let view = pane(&app, cx, Pane::CustomLists);
    view.update(cx, |view, cx| {
        view.import_range.update(cx, |field, cx| field.set_text("$C$1:$C$5", cx));
        view.import_lists(cx);
        assert_eq!(view.list_entries.read(cx).text(), "Low\nMedium\nHigh");
    });
    redraw(cx);
    assert_eq!(settings::get().custom_lists, vec![vec!["Low".to_string(), "Medium".to_string(), "High".to_string()]]);
    // The number was skipped, and Excel says so.
    let alert = app.update(cx, |app, cx| app.front_alert(cx)).expect("a message");
    assert_eq!(alert.1, "Cells without simple text were ignored.");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // A range that is none is refused.
    view.update(cx, |view, cx| {
        view.import_range.update(cx, |field, cx| field.set_text("nowhere", cx));
        view.import_lists(cx);
    });
    redraw(cx);
    assert_eq!(app.update(cx, |app, cx| app.front_alert(cx)).map(|alert| alert.1).as_deref(), Some("Reference isn't valid."));
    assert_eq!(settings::get().custom_lists.len(), 1);
}

#[gpui::test]
fn the_tables_pane_stops_tables_from_growing_and_filling(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for (cell, typed) in [((0, 0), "Name"), ((0, 1), "Qty"), ((1, 0), "Pen"), ((1, 1), "3")] {
        type_in(&app, cx, cell, typed);
    }
    app.update(cx, |app, cx| {
        app.wb.add_table(0, opensheet_engine::Rect::new(0, 0, 1, 1), true).unwrap();
        app.commit("table", cx);
    });
    let range = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.wb.sheets[0].tables[0].range);
    type_in(&app, cx, (2, 0), "Ink");
    assert_eq!(range(&app, cx).r1, 2);
    // The window opens on the page; the pane is rendered, its boxes
    // change the settings and the workbook follows at once.
    let view = pane(&app, cx, Pane::Tables);
    view.update(cx, |view, cx| view.set(cx, |settings| settings.tables_expand = false));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!settings::get().tables_expand);
    type_in(&app, cx, (3, 0), "Pad");
    assert_eq!(range(&app, cx).r1, 2);
    // A formula beside the table no longer becomes a calculated column.
    settings::update(|settings| {
        settings.tables_expand = true;
        settings.tables_fill_formulas = false;
    });
    app.update(cx, |app, _| apply_to_workbook(&mut app.wb));
    type_in(&app, cx, (0, 2), "Double");
    type_in(&app, cx, (1, 2), "=B2*2");
    assert_eq!(range(&app, cx).c1, 2);
    assert_eq!(text(&app, cx, (2, 2)), "");
    // A workbook that is opened takes the settings as well.
    app.update(cx, |app, cx| {
        let wb = Workbook::new(opensheet_engine::Locale::en());
        app.set_workbook(wb, None, "Other".to_string(), cx);
        assert!(!app.wb.table_auto.fill_formulas && app.wb.table_auto.expand);
    });
}
