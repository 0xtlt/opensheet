//! The editing tools driven in GPUI's headless window: commands, keys,
//! typing and clicks on the dialogs. What these tests cannot tell is how
//! things look.

use super::*;
use crate::app::PopupKind;
use crate::features::MenuTarget;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
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
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("{name} is not drawn"))
        .center()
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
    app.update(cx, |app, _| {
        match app.popup.as_ref().map(|popup| &popup.kind) {
            Some(PopupKind::Menu(entries)) => entries
                .iter()
                .filter(|entry| !entry.separator)
                .map(|entry| entry.label.to_string())
                .collect(),
            _ => Vec::new(),
        }
    })
}

// --------------------------------------------------------- find and replace

#[gpui::test]
fn the_replace_dialog_finds_and_replaces(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(
            wb,
            &[
                ("A1", "tea"),
                ("B2", "Teapot"),
                ("A3", "=LEN(\"tea\")"),
                ("C4", "coffee"),
            ],
        )
    });
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
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "x"), ("B2", "x"), ("B3", "x"), ("D5", "x")])
    });
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
        app.update_in(cx, |app, window, cx| {
            assert!(find::find_next(app, &choices, false, window, cx))
        });
        assert_eq!(active(&app, cx), expected);
        assert_eq!(
            app.update(cx, |app, _| app.sel().ranges.clone()),
            vec![Rect::parse("B1:B4").unwrap()]
        );
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
    app.update_in(cx, |app, window, cx| {
        find::replace_all(app, &broken, window, cx)
    });
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
    app.update_in(cx, |app, window, cx| {
        find::replace_all(app, &choices, window, cx)
    });
    assert_eq!(value(&app, cx, "A1"), Value::text("old"));
    assert_eq!(value(&app, cx, "B1"), Value::text("new"));
    cx.simulate_keystrokes("enter");
    // Nothing left but the locked cell: the message of protected sheets.
    app.update_in(cx, |app, window, cx| {
        find::replace_all(app, &choices, window, cx)
    });
    assert_eq!(value(&app, cx, "A1"), Value::text("old"));
    assert_eq!(dialogs(&app, cx), 1);
    // The Replace button asks the protection too.
    cx.simulate_keystrokes("enter");
    select(&app, cx, "A1");
    app.update_in(cx, |app, window, cx| {
        find::replace(app, &choices, window, cx)
    });
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
    app.update(cx, |app, _| {
        app.ext_mut::<find::Choices>().options.look_in = opensheet_engine::find::LookIn::Values
    });
    app.update(cx, |app, cx| find_bar_next(app, "5", cx));
    assert_eq!(active(&app, cx), (0, 0));
}

#[gpui::test]
fn find_and_select_reaches_the_cells_of_a_kind(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(wb, &[("A1", "1"), ("A2", "=A1+1"), ("B2", "text")])
    });
    run(&app, cx, Cmd::SelectFormulas);
    assert_eq!(
        app.update(cx, |app, _| app.sel().ranges.clone()),
        vec![Rect::parse("A2").unwrap()]
    );
    run(&app, cx, Cmd::SelectConstants);
    let mut found = app.update(cx, |app, _| app.sel().ranges.clone());
    found.sort_by_key(|range| (range.r0, range.c0));
    assert_eq!(
        found,
        vec![Rect::parse("A1").unwrap(), Rect::parse("B2").unwrap()]
    );
}

/// The cells the Find All list of the open dialog holds, drawn again
/// first so that it has followed the workbook.
fn listed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<Vec<(usize, u32, u32)>> {
    redraw(cx);
    let dialog = app
        .update(cx, |app, _| {
            app.front_modal_view()
                .expect("a dialog is open")
                .downcast::<find::FindReplace>()
        })
        .ok()
        .expect("the Find dialog");
    dialog.update(cx, |dialog, _| dialog.listed())
}

#[gpui::test]
fn find_all_lists_the_cells_and_a_click_goes_to_one(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(
            wb,
            &[
                ("A1", "tea"),
                ("B2", "Teapot"),
                ("A3", "=LEN(\"tea\")"),
                ("C4", "coffee"),
            ],
        );
        let second = wb.add_sheet(1);
        wb.set_cell_text(second, 5, 2, "green tea").unwrap();
    });
    // Within the sheet: the two constants and the formula.
    run(&app, cx, Cmd::Find);
    cx.simulate_input("tea");
    assert_eq!(listed(&app, cx), None, "no list before Find All");
    press(cx, "find-all");
    assert_eq!(
        listed(&app, cx),
        Some(vec![(0, 0, 0), (0, 1, 1), (0, 2, 0)])
    );
    // A click on a line goes to its cell and leaves the dialog open.
    press(cx, "find-result-2");
    assert_eq!(active(&app, cx), (2, 0));
    assert_eq!(dialogs(&app, cx), 1);
    press(cx, "find-close");

    // Within the workbook, looking in values with the case respected:
    // the formula shows 3, "Teapot" has a capital, the other sheet counts.
    app.update(cx, |app, _| {
        let choices = app.ext_mut::<find::Choices>();
        choices.workbook = true;
        choices.options.look_in = opensheet_engine::find::LookIn::Values;
        choices.options.match_case = true;
    });
    run(&app, cx, Cmd::Find);
    press(cx, "find-all");
    assert_eq!(listed(&app, cx), Some(vec![(0, 0, 0), (1, 5, 2)]));
    press(cx, "find-result-1");
    assert_eq!(
        app.update(cx, |app, _| app.sheet),
        1,
        "the line of another sheet shows that sheet"
    );
    assert_eq!(active(&app, cx), (5, 2));
    press(cx, "find-close");

    // Entire cells only.
    app.update_in(cx, |app, window, cx| {
        app.switch_sheet(0, window, cx);
        let choices = app.ext_mut::<find::Choices>();
        choices.workbook = false;
        choices.options.match_case = false;
        choices.options.whole_cell = true;
    });
    run(&app, cx, Cmd::Find);
    press(cx, "find-all");
    assert_eq!(listed(&app, cx), Some(vec![(0, 0, 0)]));
    // Nothing found: Excel's message, and no list.
    press(cx, "find-close");
    run(&app, cx, Cmd::Find);
    cx.simulate_input("mate");
    press(cx, "find-all");
    assert_eq!(dialogs(&app, cx), 2);
    cx.simulate_keystrokes("enter");
    assert_eq!(listed(&app, cx), None);
}

#[gpui::test]
fn the_list_of_find_all_follows_a_replacement(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(
            wb,
            &[("A1", "tea"), ("B2", "Teapot"), ("A3", "=LEN(\"tea\")")],
        )
    });
    cx.simulate_keystrokes("ctrl-h");
    cx.simulate_input("tea");
    cx.simulate_keystrokes("tab");
    cx.simulate_input("mate");
    press(cx, "find-all");
    assert_eq!(listed(&app, cx).map(|cells| cells.len()), Some(3));
    // Replace changes the active cell (A1): two lines are left.
    press(cx, "find-replace-one");
    assert_eq!(value(&app, cx, "A1"), Value::text("mate"));
    assert_eq!(listed(&app, cx), Some(vec![(0, 1, 1), (0, 2, 0)]));
    // Replace All empties the list; undoing it brings the lines back.
    press(cx, "find-replace-all");
    cx.simulate_keystrokes("enter");
    assert_eq!(listed(&app, cx), Some(Vec::new()));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(listed(&app, cx), Some(vec![(0, 1, 1), (0, 2, 0)]));
}

// ------------------------------------------------------------ paste special

fn copy(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    select(app, cx, range);
    app.update(cx, |app, cx| app.copy(false, cx));
}

#[gpui::test]
fn the_paste_special_dialog_transposes_and_operates(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        enter(
            wb,
            &[("A1", "1"), ("B1", "2"), ("C1", "=A1+B1"), ("E1", "x")],
        )
    });
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
    assert_eq!(
        app.update(cx, |app, _| app.sel().current()),
        Rect::parse("E1:E3").unwrap()
    );
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
    assert_eq!(entries.len(), 15);
    assert_eq!(&entries[12..14], ["Picture", "Linked Picture"]);
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

// ---------------------------------------------------------- pictures of cells

use opensheet_engine::Color;
use opensheet_engine::drawing::Drawing;

fn drawings(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<Drawing> {
    app.update(cx, |app, _| app.wb.sheets[app.sheet].drawings.clone())
}

/// The colour of a picture at a point given in pixels of the sheet from
/// its corner (the picture is drawn at twice that size).
fn colour_at(drawing: &Drawing, x: u32, y: u32) -> [u8; 4] {
    let picture = drawing.as_picture().expect("a picture");
    let pixels = image::load_from_memory(&picture.data)
        .expect("a PNG file")
        .into_rgba8();
    pixels.get_pixel(x * 2, y * 2).0
}

/// A1:B2 with a red A1, a bold number in B1 and a green B2.
fn painted(wb: &mut Workbook) {
    enter(
        wb,
        &[("A1", "Tea"), ("B1", "12.5"), ("A2", "Coffee"), ("B2", "7")],
    );
    wb.update_style(0, Rect::parse("A1").unwrap(), &|s| {
        s.fill = Some(Color(0xFF0000))
    });
    wb.update_style(0, Rect::parse("B2").unwrap(), &|s| {
        s.fill = Some(Color(0x00B050))
    });
    wb.update_style(0, Rect::parse("B1").unwrap(), &|s| s.font.bold = true);
}

/// Width and height of a range on the first sheet, in pixels at 100 %.
fn extent(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) -> (f32, f32) {
    let rect = Rect::parse(range).unwrap();
    app.update(cx, |app, _| {
        let sheet = &app.wb.sheets[0];
        let (x0, y0) = sheet.corner_point(((rect.r0, rect.c0), (0.0, 0.0)));
        let (x1, y1) = sheet.corner_point(((rect.r1 + 1, rect.c1 + 1), (0.0, 0.0)));
        ((x1 - x0) as f32, (y1 - y0) as f32)
    })
}

#[gpui::test]
fn the_copied_cells_are_pasted_as_a_picture(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, painted);
    assert!(
        !app.update(cx, |app, _| Cmd::PastePicture.enabled(app)),
        "nothing is copied"
    );
    copy(&app, cx, "A1:B2");
    select(&app, cx, "D5");
    run(&app, cx, Cmd::PastePicture);
    let pictures = drawings(&app, cx);
    assert_eq!(pictures.len(), 1);
    let picture = &pictures[0];
    // An ordinary picture, the size of the cells, its corner on D5.
    assert!(
        picture
            .as_picture()
            .is_some_and(|p| p.link.is_none() && p.svg.is_none())
    );
    let (width, height) = extent(&app, cx, "A1:B2");
    assert!(
        (picture.size.0 - width).abs() < 0.5 && (picture.size.1 - height).abs() < 0.5,
        "{:?}",
        picture.size
    );
    let frame = app.update(cx, |app, _| app.wb.drawing_frame(0, picture.id).unwrap());
    let corner = app.update(cx, |app, _| {
        app.wb.sheets[0].corner_point(((4, 3), (0.0, 0.0)))
    });
    assert!((frame.x - corner.0).abs() < 0.5 && (frame.y - corner.1).abs() < 0.5);
    // It looks like the cells: the fills are where the cells are.
    let (column, row) = extent(&app, cx, "A1");
    let red = colour_at(picture, column as u32 / 2, row as u32 / 2);
    assert!(red[0] > 200 && red[1] < 60 && red[2] < 60, "{red:?}");
    let green = colour_at(picture, column as u32 + 8, row as u32 + row as u32 / 2);
    assert!(
        green[0] < 60 && green[1] > 140 && green[2] < 120,
        "{green:?}"
    );
    // A cell without a fill lets what is under the picture show, as in
    // Excel.
    let blank = colour_at(picture, column as u32 + 4, 3);
    assert_eq!(blank[3], 0, "B1 has no fill: {blank:?}");
    // The picture is selected; one step of undo takes it away.
    assert!(app.update(cx, |app, _| app.object_sel.is_some()));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Paste Picture"));
    app.update(cx, |app, cx| app.undo(cx));
    assert!(drawings(&app, cx).is_empty());
    // A protected sheet takes no picture.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    copy(&app, cx, "A1:B2");
    run(&app, cx, Cmd::PastePicture);
    assert!(drawings(&app, cx).is_empty());
}

#[gpui::test]
fn a_linked_picture_follows_its_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, painted);
    copy(&app, cx, "A1:B2");
    select(&app, cx, "D5");
    run(&app, cx, Cmd::PasteLinkedPicture);
    assert_eq!(
        undo_label(&app, cx).as_deref(),
        Some("Paste Linked Picture")
    );
    let link = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        redraw(cx);
        let picture = drawings(app, cx).remove(0);
        let text = app.update(cx, |app, _| {
            picture.as_picture().unwrap().link_text(app.wb.locale)
        });
        (picture, text.unwrap_or_default())
    };
    let (before, formula) = link(&app, cx);
    assert_eq!(formula, "=$A$1:$B$2");
    let (column, row) = extent(&app, cx, "A1");
    let at = (column as u32 / 2, row as u32 / 2);
    assert!(colour_at(&before, at.0, at.1)[0] > 200);
    // The cell turns blue: so does the picture, without a step of its own.
    edit(&app, cx, |wb| {
        wb.update_style(0, Rect::parse("A1").unwrap(), &|s| {
            s.fill = Some(Color(0x0000FF))
        })
    });
    let (after, _) = link(&app, cx);
    let blue = colour_at(&after, at.0, at.1);
    assert!(blue[2] > 200 && blue[0] < 60, "{blue:?}");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
    // Undoing the fill brings the red back in the picture too.
    app.update(cx, |app, cx| app.undo(cx));
    let (undone, _) = link(&app, cx);
    assert!(colour_at(&undone, at.0, at.1)[0] > 200);
    // A row inserted above: the picture goes on showing the same cells.
    edit(&app, cx, |wb| wb.insert_rows(0, 0, 1));
    let (moved, formula) = link(&app, cx);
    assert_eq!(formula, "=$A$2:$B$3");
    assert!(colour_at(&moved, at.0, at.1)[0] > 200);
    // On another sheet the formula names the sheet, and follows its name.
    let second = app.update(cx, |app, cx| {
        let second = app.wb.add_sheet(1);
        app.commit("setup", cx);
        second
    });
    copy(&app, cx, "A2:B3");
    app.update_in(cx, |app, window, cx| app.switch_sheet(second, window, cx));
    run(&app, cx, Cmd::PasteLinkedPicture);
    let (_, formula) = link(&app, cx);
    assert_eq!(formula, "=Sheet1!$A$2:$B$3");
    edit(&app, cx, |wb| {
        wb.rename_sheet(0, "Stock");
    });
    let (_, formula) = link(&app, cx);
    assert_eq!(formula, "=Stock!$A$2:$B$3");
    // Saved and opened again, both pictures still name their cells.
    let bytes = app.update(cx, |app, _| {
        opensheet_io::write_xlsx(&app.wb, std::io::Cursor::new(Vec::new()))
            .unwrap()
            .into_inner()
    });
    if let Some(dir) = std::env::var_os("OPENSHEET_SAMPLES") {
        std::fs::write(
            std::path::Path::new(&dir).join("opensheet-linked-picture-of-cells.xlsx"),
            &bytes,
        )
        .unwrap();
    }
    let back =
        opensheet_io::read_xlsx(std::io::Cursor::new(&bytes), opensheet_engine::Locale::en())
            .unwrap();
    let sources: Vec<_> = back
        .linked_pictures()
        .into_iter()
        .map(|(sheet, _, source)| (sheet, source.sheet, source.rect.name()))
        .collect();
    assert_eq!(
        sources,
        vec![(0, 0, "A2:B3".to_string()), (1, 0, "A2:B3".to_string())]
    );
}

#[gpui::test]
fn copy_as_picture_puts_a_picture_on_the_clipboard(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, painted);
    copy(&app, cx, "A1:B2");
    // Home ▸ Copy offers Copy and Copy as Picture…
    run(&app, cx, Cmd::CopyMenu);
    assert_eq!(menu(&app, cx), ["Copy", "Copy as Picture…"]);
    app.update(cx, |app, cx| {
        app.popup = None;
        cx.notify();
    });
    run(&app, cx, Cmd::CopyAsPicture);
    assert_eq!(dialogs(&app, cx), 1);
    // "As shown on screen" is chosen; Return copies.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(dialogs(&app, cx), 0);
    assert!(
        app.update(cx, |app, _| app.clip.is_none()),
        "the cells are no longer what Paste pastes"
    );
    let item = cx.read_from_clipboard().expect("something was copied");
    let image = item
        .entries()
        .iter()
        .find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        })
        .expect("a picture");
    assert!(image.bytes().starts_with(b"\x89PNG"));
    // Paste puts it on the sheet as a picture the size of the cells.
    select(&app, cx, "D5");
    app.update(cx, |app, cx| app.paste(PasteMode::All, cx));
    redraw(cx);
    let pictures = drawings(&app, cx);
    assert_eq!(pictures.len(), 1);
    let (width, height) = extent(&app, cx, "A1:B2");
    assert!(
        (pictures[0].size.0 - width).abs() < 1.0 && (pictures[0].size.1 - height).abs() < 1.0,
        "{:?}",
        pictures[0].size
    );
    assert_eq!(value(&app, cx, "D5"), Value::Empty, "no cell was pasted");

    // "As shown when printed": the sheet prints in black and white, so
    // the picture has no fill.
    edit(&app, cx, |wb| {
        wb.update_page_setup(0, |setup| setup.black_and_white = true);
    });
    select(&app, cx, "A1:B2");
    assert!(app.update(cx, |app, cx| picture::copy(app, true, cx)));
    let printed = cx.read_from_clipboard().and_then(|item| {
        item.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => Some(image.bytes().to_vec()),
            _ => None,
        })
    });
    let pixels = image::load_from_memory(&printed.expect("a picture"))
        .unwrap()
        .into_rgba8();
    let (column, row) = extent(&app, cx, "A1");
    let pixel = pixels.get_pixel(column as u32, row as u32).0;
    assert_eq!(pixel[3], 0, "{pixel:?}");
}

// ---------------------------------------------------------------- hyperlinks

use opensheet_engine::hyperlink::{FOLLOWED_COLOR, Hyperlink, LINK_COLOR, LinkKind};
use std::time::Duration;

fn link(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> Option<Hyperlink> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| {
        app.wb.hyperlink_at(app.sheet, at.r0, at.c0).cloned()
    })
}

fn font_color(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    name: &str,
) -> Option<opensheet_engine::Color> {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| {
        app.wb.style_at(app.sheet, at.r0, at.c0).font.color
    })
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
        menu.build()
            .into_iter()
            .filter(|entry| !entry.separator)
            .map(|entry| entry.label.to_string())
            .collect()
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
    assert_eq!(
        (made.kind(), made.address.as_str()),
        (LinkKind::Web, "http://www.example.org/")
    );
    // An empty cell shows the address, with the look of a link.
    assert_eq!(
        value(&app, cx, "B2"),
        Value::text("http://www.example.org/")
    );
    assert_eq!(font_color(&app, cx, "B2"), Some(LINK_COLOR));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Hyperlink"));
    assert_eq!(
        cell_menu(&app, cx, "B2"),
        ["Paste Special…", "Edit Link…", "Open Link", "Remove Link"]
    );
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
    assert!(app.update(cx, |app, cx| links::insert(
        app,
        Hyperlink::place(range, "Sheet2!C3"),
        Some("Go".into()),
        cx
    )));
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
    assert_eq!(
        (edited.location.as_str(), edited.tooltip.as_str()),
        ("Sheet2!C3", "Where the totals are")
    );
    // A number keeps its value under a link: the text is not asked.
    select(&app, cx, "A2");
    assert!(app.update(cx, |app, cx| {
        links::insert(
            app,
            Hyperlink::email(Rect::parse("A2").unwrap(), "ann@example.org", "Hi"),
            None,
            cx,
        )
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
        enter(
            wb,
            &[
                ("B2", "Site"),
                ("B3", "There"),
                ("B4", "=HYPERLINK(\"mailto:ann@example.org\",\"Ann\")"),
            ],
        );
        wb.set_hyperlink(
            0,
            Hyperlink::external(Rect::parse("B2").unwrap(), "https://example.org/"),
        );
        wb.format_as_link(0, Rect::parse("B2").unwrap(), false);
        wb.set_hyperlink(0, Hyperlink::place(Rect::parse("B3").unwrap(), "Sheet1!E9"));
    });
    // The hand and the ScreenTip are there for the three cells.
    let zones = app.update(cx, |app, _| {
        app.zones
            .iter()
            .filter(|zone| zone.cursor == Some(gpui::CursorStyle::PointingHand))
            .filter_map(|zone| zone.tooltip.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(zones.len(), 3);
    assert!(
        zones
            .iter()
            .any(|tip| tip.as_ref() == "https://example.org/")
    );
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
    app.update_in(cx, |app, window, cx| {
        links::released(app, (2, 1), (2, 1), Duration::from_secs(1), window, cx)
    });
    assert_eq!(active(&app, cx), (2, 1));
    cx.simulate_keystrokes("up");
    assert_eq!(active(&app, cx), (1, 1));
    // A place that is nowhere says so.
    edit(&app, cx, |wb| {
        wb.set_hyperlink(
            0,
            Hyperlink::place(Rect::parse("D1").unwrap(), "Nowhere!A1"),
        )
    });
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
    assert_eq!(
        (link(&app, cx, "A1"), value(&app, cx, "A1")),
        (None, Value::Empty)
    );
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
    assert_eq!(
        link(&app, cx, "C1").map(|link| link.address),
        Some("mailto:ann@example.org".into())
    );
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
    assert_eq!(
        (link(&app, cx, "A1"), font_color(&app, cx, "A1")),
        (None, None)
    );
    assert_eq!(value(&app, cx, "A1"), Value::text("ann@example.org"));
    app.update(cx, |app, cx| app.undo(cx));
    assert!(link(&app, cx, "A1").is_some());
    app.update(cx, |app, cx| {
        app.clear(opensheet_engine::workbook::ClearMode::Contents, cx)
    });
    assert_eq!(link(&app, cx, "A1"), None);
    // A protected sheet takes no link.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, Default::default(), "");
    });
    select(&app, cx, "B9");
    run(&app, cx, Cmd::Link);
    assert_eq!(
        dialogs(&app, cx),
        1,
        "the message of protected sheets, not the dialog"
    );
    assert!(!app.update(cx, |app, cx| links::insert(
        app,
        Hyperlink::external(Rect::parse("B9").unwrap(), "https://example.org"),
        None,
        cx
    )));
}

// --------------------------------------------------------------- cell styles

fn style_name(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> String {
    let at = Rect::parse(name).unwrap();
    app.update(cx, |app, _| {
        app.wb.cell_style_at(app.sheet, at.r0, at.c0).to_string()
    })
}

fn fill(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    name: &str,
) -> Option<opensheet_engine::Color> {
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
    let titles: Vec<(opensheet_engine::cell_style::StyleGroup, usize)> = shown
        .iter()
        .map(|(group, styles)| (*group, styles.len()))
        .collect();
    use opensheet_engine::cell_style::StyleGroup::*;
    assert_eq!(
        titles,
        [
            (GoodBadNeutral, 4),
            (DataModel, 10),
            (TitlesHeadings, 6),
            (Themed, 24),
            (NumberFormat, 5)
        ]
    );
    for name in [
        "Normal",
        "Good",
        "Heading 1",
        "20% - Accent1",
        "Accent6",
        "Percent",
    ] {
        assert!(
            cx.debug_bounds(Box::leak(format!("cell-style-{name}").into_boxed_str()))
                .is_some(),
            "{name} is drawn"
        );
    }
    // A click applies the style to the selection and closes the gallery.
    press(cx, "cell-style-Good");
    assert!(app.update(cx, |app, _| app.popup.is_none()));
    assert_eq!(style_name(&app, cx, "B2"), "Good");
    assert_eq!(style_name(&app, cx, "B3"), "Good");
    assert_eq!(
        fill(&app, cx, "B2"),
        Some(opensheet_engine::Color(0xC6EFCE))
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Cell Styles"));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(
        (style_name(&app, cx, "B2"), fill(&app, cx, "B2")),
        ("Normal".to_string(), None)
    );
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
    let made = app.update(cx, |app, _| {
        app.wb.cell_style("Blue box").expect("the style")
    });
    assert_eq!(made.style.fill, Some(opensheet_engine::Color(0x00B0F0)));
    assert!(made.includes.fill && !made.includes.font);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("New Cell Style"));
    // It leads the gallery, under Custom.
    let first = app.update(cx, |app, _| styles::sections(app).remove(0));
    assert_eq!(
        (first.0, first.1[0].name.as_str()),
        (opensheet_engine::cell_style::StyleGroup::Custom, "Blue box")
    );
    // Applied to another cell, then modified: the cell follows.
    select(&app, cx, "A2");
    app.update(cx, |app, cx| styles::apply(app, "Blue box", cx));
    assert_eq!(
        fill(&app, cx, "A2"),
        Some(opensheet_engine::Color(0x00B0F0))
    );
    app.update_in(cx, |app, window, cx| {
        styles::open_dialog(
            app,
            styles::Purpose::Modify("Blue box".into()),
            None,
            window,
            cx,
        )
    });
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
    assert!(app.update(cx, |app, cx| styles::save(
        app,
        &styles::Purpose::Modify("Blue box".into()),
        "",
        changed,
        made.includes,
        cx
    )));
    assert_eq!(
        fill(&app, cx, "A2"),
        Some(opensheet_engine::Color(0xFF0000))
    );
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(
        fill(&app, cx, "A2"),
        Some(opensheet_engine::Color(0x00B0F0))
    );
    // A name that is taken is refused, as Excel does.
    assert!(!app.update(cx, |app, cx| styles::save(
        app,
        &styles::Purpose::New,
        "good",
        made.style.clone(),
        made.includes,
        cx
    )));
    assert_eq!(dialogs(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    // The menu of a style: Normal cannot be modified, Excel's styles
    // cannot be deleted, the user's can.
    let entries = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str| {
        app.update(cx, |app, _| {
            let style = app.wb.cell_style(name).unwrap();
            styles::style_menu(app, &style)
                .into_iter()
                .map(|entry| (entry.label.to_string(), entry.disabled))
                .collect::<Vec<_>>()
        })
    };
    let all = |disabled: [bool; 4]| {
        ["Apply", "Modify...", "Duplicate...", "Delete"]
            .iter()
            .map(|l| l.to_string())
            .zip(disabled)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        entries(&app, cx, "Blue box"),
        all([false, false, false, false])
    );
    assert_eq!(entries(&app, cx, "Good"), all([false, false, false, true]));
    assert_eq!(entries(&app, cx, "Normal"), all([false, true, false, true]));
    // Duplicate proposes a name of its own.
    app.update_in(cx, |app, window, cx| {
        styles::open_dialog(app, styles::Purpose::New, Some("Blue box"), window, cx)
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    assert!(app.update(cx, |app, _| app.wb.cell_style("Blue box 2").is_some()));
    // Delete: the cells that had the style are Normal again.
    app.update(cx, |app, cx| styles::delete(app, "Blue box", cx));
    assert_eq!(
        (style_name(&app, cx, "A2"), fill(&app, cx, "A2")),
        ("Normal".to_string(), None)
    );
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

#[gpui::test]
fn a_symbol_goes_into_a_text_box_and_into_a_note(cx: &mut TestAppContext) {
    use opensheet_engine::drawing::{Frame, ShapeTool};
    let (app, cx) = open(cx);
    // A selected shape is opened for typing, after what it holds; no cell
    // is opened.
    let id = app.update(cx, |app, cx| {
        let id = app
            .wb
            .add_shape(
                app.sheet,
                ShapeTool::TextBox,
                Frame::new(100.0, 60.0, 160.0, 80.0),
            )
            .expect("a shape");
        app.wb.update_drawing(app.sheet, id, |drawing| {
            drawing.as_shape_mut().unwrap().set_text("Sum ".to_string())
        });
        app.commit("setup", cx);
        app.select_object(
            Some(crate::features::object_id(
                &crate::features::drawings::Drawings,
                id,
            )),
            cx,
        );
        id
    });
    redraw(cx);
    let shape_text = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| {
            app.wb
                .drawing(0, id)
                .and_then(|d| d.as_shape().map(|shape| shape.text.clone()))
        })
    };
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.is_editing()));
    cx.simulate_input("∑");
    // Asked again while the text is being typed, the text keeps the keys.
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    cx.simulate_input("√");
    cx.simulate_keystrokes("escape");
    assert_eq!(shape_text(&app, cx).as_deref(), Some("Sum ∑√"));
    app.update(cx, |app, cx| app.select_object(None, cx));
    // A note being typed takes the symbol too.
    select(&app, cx, "C3");
    cx.simulate_keystrokes("shift-f2");
    cx.simulate_input("See ");
    app.update_in(cx, |app, window, cx| Cmd::Symbol.run(app, window, cx));
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.is_editing()));
    cx.simulate_input("§");
    cx.simulate_keystrokes("escape");
    let note = app.update(cx, |app, _| {
        app.wb
            .comment_at(0, 2, 2)
            .and_then(|comment| comment.as_note().map(|note| note.text()))
    });
    assert!(note.is_some_and(|text| text.ends_with("See §")));
    assert!(app.update(cx, |app, _| app.wb.sheets[0].cells.is_empty()));
}
