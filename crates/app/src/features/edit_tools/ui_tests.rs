//! The editing tools driven in GPUI's headless window: commands, keys,
//! typing and clicks on the dialogs. What these tests cannot tell is how
//! things look.

use super::*;
use crate::app::PopupKind;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use crate::features::MenuTarget;
use opensheet_engine::ops::PasteMode;
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

// ---------------------------------------------------------------- hyperlinks

use opensheet_engine::hyperlink::{FOLLOWED_COLOR, Hyperlink, LINK_COLOR, LinkKind};
use std::time::Duration;

fn link(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Option<Hyperlink> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.hyperlink_at(app.sheet, at.r0, at.c0).cloned())
}

fn font_color(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Option<opensheet_engine::Color> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.style_at(app.sheet, at.r0, at.c0).font.color)
}

/// The middle of a cell, in window coordinates.
fn centre(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Point<Pixels> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let (x0, y0, x1, y1) = layout.rect_px(at);
        layout.to_window((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    })
}

/// The labels of the context menu of a cell.
fn cell_menu(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Vec<String> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| {
        let mut menu = crate::features::ContextMenu::default();
        EditTools.context_menu(app, &MenuTarget::Cell(at.r0, at.c0), &mut menu);
        menu.build().into_iter().filter(|entry| !entry.separator).map(|entry| entry.label.to_string()).collect()
    })
}

#[gpui::test]
fn the_link_dialog_inserts_a_web_link(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, "B2");
    // Command-K opens the dialog in the Address box.
    cx.simulate_keystrokes("cmd-k");
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_input("www.example.org");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 0);
    let made = link(&app, cx, "B2").expect("a link");
    assert_eq!((made.kind(), made.address.as_str()), (LinkKind::Web, "http://www.example.org/"));
    // An empty cell shows the address, with the look of a link.
    assert_eq!(value(&app, cx, "B2"), Value::text("http://www.example.org/"));
    assert_eq!(font_color(&app, cx, "B2"), Some(LINK_COLOR));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Hyperlink"));
    assert_eq!(cell_menu(&app, cx, "B2"), ["Paste Special…", "Edit Link…", "Open Link", "Remove Link"]);
    assert_eq!(cell_menu(&app, cx, "C5"), ["Paste Special…", "Link"]);
    // One step back: no link, no text, no colour.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(link(&app, cx, "B2"), None);
    assert_eq!(value(&app, cx, "B2"), Value::Empty);
    assert_eq!(font_color(&app, cx, "B2"), None);
}

#[gpui::test]
fn the_link_dialog_edits_places_mail_and_screen_tips(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "Totals"), ("A2", "12")]);
        wb.add_sheet(1);
    });
    // A place in this document, through the dialog's own pieces.
    select(&app, cx, "A1");
    let range = Rect::parse("A1").unwrap();
    assert!(app.update(cx, |app, cx| links::insert(app, Hyperlink::place(range, "Sheet2!C3"), Some("Go".into()), cx)));
    assert_eq!(value(&app, cx, "A1"), Value::text("Go"));
    // Editing it: the dialog opens on "This Document"; the ScreenTip box
    // opens over it and gives its text back.
    run(&app, cx, Cmd::EditLink);
    assert_eq!(dialogs(&app, cx), 1);
    press(cx, "link-screen-tip");
    assert_eq!(dialogs(&app, cx), 2);
    cx.simulate_input("Where the totals are");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 1);
    press(cx, "link-ok");
    assert_eq!(dialogs(&app, cx), 0);
    let edited = link(&app, cx, "A1").expect("a link");
    assert_eq!((edited.location.as_str(), edited.tooltip.as_str()), ("Sheet2!C3", "Where the totals are"));
    // A number keeps its value under a link: the text is not asked.
    select(&app, cx, "A2");
    assert!(app.update(cx, |app, cx| {
        links::insert(app, Hyperlink::email(Rect::parse("A2").unwrap(), "ann@example.org", "Hi"), None, cx)
    }));
    assert_eq!(value(&app, cx, "A2"), Value::Number(12.0));
    // Remove Link in the dialog takes the link and its look away.
    run(&app, cx, Cmd::EditLink);
    press(cx, "link-remove");
    assert_eq!(dialogs(&app, cx), 0);
    assert_eq!(link(&app, cx, "A2"), None);
    assert_eq!(font_color(&app, cx, "A2"), None);
    assert_eq!(value(&app, cx, "A2"), Value::Number(12.0));
}

#[gpui::test]
fn a_click_follows_a_link_and_a_long_press_selects(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("B2", "Site"), ("B3", "There"), ("B4", "=HYPERLINK(\"mailto:ann@example.org\",\"Ann\")")]);
        wb.set_hyperlink(0, Hyperlink::external(Rect::parse("B2").unwrap(), "https://example.org/"));
        wb.format_as_link(0, Rect::parse("B2").unwrap(), false);
        wb.set_hyperlink(0, Hyperlink::place(Rect::parse("B3").unwrap(), "Sheet1!E9"));
    });
    // The hand and the ScreenTip are there for the three cells.
    let zones = app.update(cx, |app, _| {
        app.zones.iter().filter(|zone| zone.cursor == Some(gpui::CursorStyle::PointingHand)).filter_map(|zone| zone.tooltip.clone()).collect::<Vec<_>>()
    });
    assert_eq!(zones.len(), 3);
    assert!(zones.iter().any(|tip| tip.as_ref() == "https://example.org/"));
    // A click on the web link hands it to the system and marks it followed.
    let at = centre(&app, cx, "B2");
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.org/"));
    assert_eq!(font_color(&app, cx, "B2"), Some(FOLLOWED_COLOR));
    // A click on a place selects it.
    let at = centre(&app, cx, "B3");
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(active(&app, cx), (8, 4));
    // The HYPERLINK function is followed the same way.
    let at = centre(&app, cx, "B4");
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(cx.opened_url().as_deref(), Some("mailto:ann@example.org"));
    // A long press selects the cell instead; so do the arrow keys.
    app.update_in(cx, |app, window, cx| links::released(app, (2, 1), (2, 1), Duration::from_secs(1), window, cx));
    assert_eq!(active(&app, cx), (2, 1));
    cx.simulate_keystrokes("up");
    assert_eq!(active(&app, cx), (1, 1));
    // A place that is nowhere says so.
    edit(&app, cx, |wb| wb.set_hyperlink(0, Hyperlink::place(Rect::parse("D1").unwrap(), "Nowhere!A1")));
    app.update_in(cx, |app, window, cx| links::follow(app, 0, 3, window, cx));
    assert_eq!(dialogs(&app, cx), 1);
}

#[gpui::test]
fn typing_an_address_makes_a_link_and_links_follow_their_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, "A1");
    cx.simulate_input("ann@example.org");
    cx.simulate_keystrokes("enter");
    let made = link(&app, cx, "A1").expect("AutoFormat made a link");
    assert_eq!(made.address, "mailto:ann@example.org");
    assert_eq!(font_color(&app, cx, "A1"), Some(LINK_COLOR));
    // The entry and its link are one step.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!((link(&app, cx, "A1"), value(&app, cx, "A1")), (None, Value::Empty));
    app.update(cx, |app, cx| app.redo(cx));
    // Ordinary text stays text.
    select(&app, cx, "A2");
    cx.simulate_input("plain");
    cx.simulate_keystrokes("enter");
    assert_eq!(link(&app, cx, "A2"), None);
    // A copy of the cell has the link; a cut takes it along.
    copy(&app, cx, "A1");
    select(&app, cx, "C1");
    app.update(cx, |app, cx| app.paste(PasteMode::All, cx));
    assert_eq!(link(&app, cx, "C1").map(|link| link.address), Some("mailto:ann@example.org".into()));
    select(&app, cx, "C1");
    app.update(cx, |app, cx| app.copy(true, cx));
    select(&app, cx, "E5");
    app.update(cx, |app, cx| app.paste(PasteMode::All, cx));
    assert_eq!(link(&app, cx, "C1"), None);
    assert!(link(&app, cx, "E5").is_some());
    // Clear Hyperlinks leaves the look, Remove Hyperlinks does not, and
    // clearing the contents takes the link with them.
    select(&app, cx, "E5");
    run(&app, cx, Cmd::ClearHyperlinks);
    assert_eq!(link(&app, cx, "E5"), None);
    assert_eq!(font_color(&app, cx, "E5"), Some(LINK_COLOR));
    assert!(!app.update(cx, |app, _| Cmd::ClearHyperlinks.enabled(app)));
    select(&app, cx, "A1");
    run(&app, cx, Cmd::RemoveHyperlinks);
    assert_eq!((link(&app, cx, "A1"), font_color(&app, cx, "A1")), (None, None));
    assert_eq!(value(&app, cx, "A1"), Value::text("ann@example.org"));
    app.update(cx, |app, cx| app.undo(cx));
    assert!(link(&app, cx, "A1").is_some());
    app.update(cx, |app, cx| app.clear(opensheet_engine::workbook::ClearMode::Contents, cx));
    assert_eq!(link(&app, cx, "A1"), None);
    // A protected sheet takes no link.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    select(&app, cx, "B9");
    run(&app, cx, Cmd::Link);
    assert_eq!(dialogs(&app, cx), 1, "the message of protected sheets, not the dialog");
    assert!(!app.update(cx, |app, cx| links::insert(app, Hyperlink::external(Rect::parse("B9").unwrap(), "https://example.org"), None, cx)));
}

// --------------------------------------------------------------- cell styles

fn style_name(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> String {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.cell_style_at(app.sheet, at.r0, at.c0).to_string())
}

fn fill(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Option<opensheet_engine::Color> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| app.wb.style_at(app.sheet, at.r0, at.c0).fill)
}

#[gpui::test]
fn the_gallery_shows_excels_styles_and_applies_one(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("B2", "ok"), ("B3", "also")]));
    select(&app, cx, "B2:B3");
    run(&app, cx, Cmd::CellStyles);
    // Excel's sections, in Excel's order, each style drawn.
    let shown = app.update(cx, |app, _| styles::sections(app));
    let titles: Vec<(opensheet_engine::cell_style::StyleGroup, usize)> = shown.iter().map(|(group, styles)| (*group, styles.len())).collect();
    use opensheet_engine::cell_style::StyleGroup::*;
    assert_eq!(titles, [(GoodBadNeutral, 4), (DataModel, 10), (TitlesHeadings, 6), (Themed, 24), (NumberFormat, 5)]);
    for name in ["Normal", "Good", "Heading 1", "20% - Accent1", "Accent6", "Percent"] {
        assert!(cx.debug_bounds(Box::leak(format!("cell-style-{name}").into_boxed_str())).is_some(), "{name} is drawn");
    }
    // A click applies the style to the selection and closes the gallery.
    press(cx, "cell-style-Good");
    assert!(app.update(cx, |app, _| app.popup.is_none()));
    assert_eq!(style_name(&app, cx, "B2"), "Good");
    assert_eq!(style_name(&app, cx, "B3"), "Good");
    assert_eq!(fill(&app, cx, "B2"), Some(opensheet_engine::Color(0xC6EFCE)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Cell Styles"));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!((style_name(&app, cx, "B2"), fill(&app, cx, "B2")), ("Normal".to_string(), None));
    // The names follow the language of the interface, not the file's.
    let good = app.update(cx, |app, _| app.wb.cell_style("Good").unwrap());
    assert_eq!(styles::display_name(&good), "Good");
    // A protected sheet keeps its formats.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    app.update(cx, |app, cx| styles::apply(app, "Bad", cx));
    assert_eq!(style_name(&app, cx, "B2"), "Normal");
    assert_eq!(dialogs(&app, cx), 1);
}

#[gpui::test]
fn a_new_style_is_made_modified_duplicated_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "mine"), ("A2", "other")]);
        wb.update_style(0, Rect::parse("A1").unwrap(), &|s| {
            s.fill = Some(opensheet_engine::Color(0x00B0F0));
            s.font.bold = true;
        });
    });
    select(&app, cx, "A1");
    // New Cell Style from the gallery: the name Excel proposes is there,
    // selected; the style is the format of the active cell.
    run(&app, cx, Cmd::CellStyles);
    press(cx, "new-cell-style");
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_input("Blue box");
    // The font is left out of what the style includes.
    press(cx, "style-font");
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 0);
    let made = app.update(cx, |app, _| app.wb.cell_style("Blue box").expect("the style"));
    assert_eq!(made.style.fill, Some(opensheet_engine::Color(0x00B0F0)));
    assert!(made.includes.fill && !made.includes.font);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("New Cell Style"));
    // It leads the gallery, under Custom.
    let first = app.update(cx, |app, _| styles::sections(app).remove(0));
    assert_eq!((first.0, first.1[0].name.as_str()), (opensheet_engine::cell_style::StyleGroup::Custom, "Blue box"));
    // Applied to another cell, then modified: the cell follows.
    select(&app, cx, "A2");
    app.update(cx, |app, cx| styles::apply(app, "Blue box", cx));
    assert_eq!(fill(&app, cx, "A2"), Some(opensheet_engine::Color(0x00B0F0)));
    app.update_in(cx, |app, window, cx| styles::open_dialog(app, styles::Purpose::Modify("Blue box".into()), None, window, cx));
    redraw(cx);
    assert_eq!(dialogs(&app, cx), 1);
    // Format… opens Format Cells over the dialog; Return keeps the format.
    press(cx, "style-format");
    assert_eq!(dialogs(&app, cx), 2);
    cx.simulate_keystrokes("enter");
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_keystrokes("escape");
    let mut changed = made.style.clone();
    changed.fill = Some(opensheet_engine::Color(0xFF0000));
    assert!(app.update(cx, |app, cx| styles::save(app, &styles::Purpose::Modify("Blue box".into()), "", changed, made.includes, cx)));
    assert_eq!(fill(&app, cx, "A2"), Some(opensheet_engine::Color(0xFF0000)));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(fill(&app, cx, "A2"), Some(opensheet_engine::Color(0x00B0F0)));
    // A name that is taken is refused, as Excel does.
    assert!(!app.update(cx, |app, cx| styles::save(app, &styles::Purpose::New, "good", made.style.clone(), made.includes, cx)));
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    // The menu of a style: Normal cannot be modified, Excel's styles
    // cannot be deleted, the user's can.
    let entries = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str| {
        app.update(cx, |app, _| {
            let style = app.wb.cell_style(name).unwrap();
            styles::style_menu(app, &style).into_iter().map(|entry| (entry.label.to_string(), entry.disabled)).collect::<Vec<_>>()
        })
    };
    let all = |disabled: [bool; 4]| ["Apply", "Modify...", "Duplicate...", "Delete"].iter().map(|l| l.to_string()).zip(disabled).collect::<Vec<_>>();
    assert_eq!(entries(&app, cx, "Blue box"), all([false, false, false, false]));
    assert_eq!(entries(&app, cx, "Good"), all([false, false, false, true]));
    assert_eq!(entries(&app, cx, "Normal"), all([false, true, false, true]));
    // Duplicate proposes a name of its own.
    app.update_in(cx, |app, window, cx| styles::open_dialog(app, styles::Purpose::New, Some("Blue box"), window, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    assert!(app.update(cx, |app, _| app.wb.cell_style("Blue box 2").is_some()));
    // Delete: the cells that had the style are Normal again.
    app.update(cx, |app, cx| styles::delete(app, "Blue box", cx));
    assert_eq!((style_name(&app, cx, "A2"), fill(&app, cx, "A2")), ("Normal".to_string(), None));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(style_name(&app, cx, "A2"), "Blue box");
}

// -------------------------------------------------------------------- symbol

#[gpui::test]
fn a_symbol_goes_into_the_cell_being_edited(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| enter(wb, &[("B2", "Price ")]));
    select(&app, cx, "B2");
    // The command opens the cell (and, on a Mac, the Emoji & Symbols
    // viewer); what the viewer sends is typed like any text.
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.is_editing()));
    cx.simulate_input("€");
    // Asked again while typing, the entry stays open.
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    cx.simulate_input("→");
    cx.simulate_keystrokes("enter");
    assert_eq!(value(&app, cx, "B2"), Value::text("Price €→"));
    // A locked cell of a protected sheet is not opened.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    select(&app, cx, "B2");
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    assert!(!app.update(cx, |app, _| app.is_editing()));
}
