//! Share driven in GPUI's headless window. The share sheet of macOS is
//! the stand-in of `system::fake`: these tests check what it is handed
//! and from where it hangs, and can never send anything.

use super::system::{Item, fake};
use super::*;
use crate::app::PopupKind;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::{Locale, Value};
use std::path::PathBuf;

pub(crate) fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    // Whatever an earlier test of this thread left behind.
    fake::taken();
    fake::fail(false);
    (app, cx)
}

pub(crate) fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// The labels of a level of the open menu, separators as "-".
pub(crate) fn menu(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    path: &[usize],
) -> Vec<String> {
    app.update(cx, |app, _| {
        let Some(PopupKind::Menu(entries)) = app.popup.as_ref().map(|popup| &popup.kind) else {
            return Vec::new();
        };
        let mut level: &[MenuEntry] = entries;
        for index in path {
            level = level[*index].submenu.as_deref().expect("a submenu");
        }
        level
            .iter()
            .map(|entry| {
                if entry.separator {
                    "-".to_string()
                } else {
                    entry.label.to_string()
                }
            })
            .collect()
    })
}

/// Where entry `index` of a panel of the open menu is drawn: entries are
/// 28 pixels tall, a separator takes 9, under a margin of 5.
fn entry_at(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    level: usize,
    index: usize,
) -> gpui::Point<Pixels> {
    let path: Vec<usize> = app.update(cx, |app, _| {
        app.popup.as_ref().expect("a menu").open_path[..level].to_vec()
    });
    let labels = menu(app, cx, &path);
    let above: f32 = labels[..index]
        .iter()
        .map(|label| if label == "-" { 9.0 } else { 28.0 })
        .sum();
    let panel = app.update(cx, |app, _| {
        app.popup.as_ref().expect("a menu").geometry.borrow().panels[level]
    });
    point(panel.left() + px(40.), panel.top() + px(5.0 + above + 14.0))
}

/// Click an entry of the open menu as the pointer would: move onto it
/// (which opens its submenu), then press and release.
fn click_entry(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, level: usize, index: usize) {
    let at = entry_at(app, cx, level, index);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

fn button(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    app.update(cx, |app, _| {
        app.anchors
            .borrow()
            .get(BUTTON)
            .copied()
            .expect("the Share button is drawn")
    })
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    cx.run_until_parked();
}

fn alert(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<(String, String)> {
    app.update(cx, |app, cx| app.front_alert(cx))
}

/// The one file handed to the share sheet, and the box the sheet hung from.
fn shared_file(cx: &mut VisualTestContext) -> (PathBuf, Bounds<Pixels>) {
    cx.run_until_parked();
    let mut shared = fake::taken();
    assert_eq!(shared.len(), 1, "one share sheet: {shared:?}");
    let (items, under) = shared.remove(0);
    match items.as_slice() {
        [Item::File(path)] => (path.clone(), under),
        other => panic!("expected one file, got {other:?}"),
    }
}

fn type_in(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: (u32, u32), text: &str) {
    app.update(cx, |app, cx| app.select_cell(cell.0, cell.1, cx));
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
}

#[gpui::test]
fn the_share_button_opens_excels_menu(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let share = button(&app, cx);
    cx.simulate_click(share.center(), Modifiers::none());
    redraw(cx);
    assert_eq!(
        menu(&app, cx, &[]),
        ["Invite People...", "Copy Link", "-", "Send a Copy"]
    );
    assert_eq!(menu(&app, cx, &[3]), ["Workbook", "PDF"]);
    // The menu hangs under the button; a second click on it closes it.
    let panel = app.update(cx, |app, _| {
        app.popup.as_ref().unwrap().geometry.borrow().panels[0]
    });
    assert!(
        panel.top() >= share.bottom() && panel.top() < share.bottom() + px(8.),
        "{panel:?} under {share:?}"
    );
    cx.simulate_click(share.center(), Modifiers::none());
    app.update(cx, |app, _| assert!(app.popup.is_none()));
    // File ▸ Share has Excel's four entries.
    let file = app.update(cx, |app, _| {
        app.anchors
            .borrow()
            .get("file-tab")
            .copied()
            .expect("the File tab")
    });
    cx.simulate_click(file.center(), Modifiers::none());
    redraw(cx);
    let entries = menu(&app, cx, &[]);
    let index = entries
        .iter()
        .position(|label| label == "Share")
        .expect("File has Share");
    assert_eq!(
        menu(&app, cx, &[index]),
        [
            "Invite People...",
            "Copy Link",
            "-",
            "Send Workbook",
            "Send PDF"
        ]
    );
}

#[gpui::test]
fn what_needs_the_cloud_says_so(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for (index, title) in [(0, "Invite People"), (1, "Copy Link")] {
        let share = button(&app, cx).center();
        cx.simulate_click(share, Modifiers::none());
        redraw(cx);
        click_entry(&app, cx, 0, index);
        let (shown, message) = alert(&app, cx).expect("a message");
        assert_eq!(shown, title);
        // Excel's sentence for a file that is not in the cloud, then what
        // can be done here.
        assert!(
            message.starts_with("Save this file to the cloud to share it with other people."),
            "{message}"
        );
        assert!(message.ends_with("use Send a Copy."), "{message}");
        cx.simulate_keystrokes("enter");
        app.update(cx, |app, _| {
            assert!(!app.modal_open() && app.popup.is_none())
        });
    }
    assert!(
        fake::taken().is_empty(),
        "nothing was handed to the share sheet"
    );
    app.update(cx, |app, _| assert!(!app.wb.can_undo()));
}

#[gpui::test]
fn a_workbook_never_saved_is_sent_as_a_temporary_copy(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    type_in(&app, cx, (1, 1), "1250");
    // Typed and not entered yet: it is part of what is sent.
    app.update(cx, |app, cx| app.select_cell(2, 1, cx));
    cx.simulate_input("=B2*2");
    let share = button(&app, cx);
    cx.simulate_click(share.center(), Modifiers::none());
    redraw(cx);
    // Send a Copy ▸ Workbook, with the pointer.
    let send = entry_at(&app, cx, 0, 3);
    cx.simulate_mouse_move(send, None, Modifiers::none());
    redraw(cx);
    cx.simulate_mouse_move(send, None, Modifiers::none());
    cx.run_until_parked();
    click_entry(&app, cx, 1, 0);
    let (path, under) = shared_file(cx);
    // The sheet hangs under the Share button.
    assert_eq!(under, share);
    let title = app.update(cx, |app, _| app.title.clone());
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("{title}.xlsx")
    );
    assert!(path.starts_with(std::env::temp_dir()), "{}", path.display());
    let copy = opensheet_io::load(&path, Locale::en()).expect("the copy is a workbook");
    assert_eq!(copy.sheets[0].value_at(1, 1), Value::Number(1250.0));
    assert_eq!(copy.sheets[0].value_at(2, 1), Value::Number(2500.0));
    // The workbook itself is as it was: no file, changes not saved.
    app.update(cx, |app, _| {
        assert!(app.path.is_none() && app.wb.modified && app.popup.is_none() && !app.modal_open());
    });
    assert!(alert(&app, cx).is_none());
}

#[gpui::test]
fn a_saved_workbook_is_sent_itself_until_it_changes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    type_in(&app, cx, (0, 0), "first");
    let dir = std::env::temp_dir().join(format!("opensheet-share-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = dir.join("Budget.xlsx");
    app.update(cx, |app, _| {
        opensheet_io::save_xlsx(&app.wb, &saved).unwrap();
        app.path = Some(saved.clone());
        app.title = "Budget".to_string();
        app.wb.modified = false;
    });
    run(&app, cx, Cmd::SendWorkbook);
    assert_eq!(shared_file(cx).0, saved);
    // Changed since: a temporary copy of what is on screen, under the same
    // name, and the saved file is left alone.
    type_in(&app, cx, (0, 0), "second");
    run(&app, cx, Cmd::SendWorkbook);
    let (copy, _) = shared_file(cx);
    assert_ne!(copy, saved);
    assert_eq!(copy.file_name(), saved.file_name());
    let text =
        |path: &PathBuf| opensheet_io::load(path, Locale::en()).unwrap().sheets[0].value_at(0, 0);
    assert_eq!(text(&copy), Value::Text("second".into()));
    assert_eq!(text(&saved), Value::Text("first".into()));
    app.update(cx, |app, _| {
        assert!(app.wb.modified && app.path.as_ref() == Some(&saved))
    });
    std::fs::remove_dir_all(&dir).unwrap();
}

#[gpui::test]
fn a_pdf_of_the_workbook_is_sent(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // Nothing to print: Excel's message, and no share sheet.
    run(&app, cx, Cmd::SendPdf);
    let (title, message) = alert(&app, cx).expect("a message");
    assert_eq!(
        (title.as_str(), message.as_str()),
        ("Sharing Error", "We didn't find anything to print.")
    );
    cx.simulate_keystrokes("enter");
    assert!(fake::taken().is_empty());
    type_in(&app, cx, (0, 0), "Printed");
    // From the menu bar: File ▸ Share ▸ Send PDF.
    cx.dispatch_action(SendPdf);
    let (path, under) = shared_file(cx);
    assert_eq!(under, button(&app, cx));
    assert_eq!(path.extension().unwrap(), "pdf");
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.starts_with(b"%PDF-") && bytes.len() > 500,
        "{} bytes",
        bytes.len()
    );
    app.update(cx, |app, _| assert!(app.path.is_none()));
}

#[gpui::test]
fn a_share_sheet_that_fails_is_reported(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    type_in(&app, cx, (0, 0), "x");
    fake::fail(true);
    run(&app, cx, Cmd::SendWorkbook);
    let (title, message) = alert(&app, cx).expect("a message");
    assert_eq!(title, "Sharing Error");
    assert!(
        message.starts_with("Something went wrong and this file can't be shared."),
        "{message}"
    );
    fake::fail(false);
    cx.simulate_keystrokes("enter");
    assert!(fake::taken().is_empty());
    // The other entries of the menu bar reach their commands too.
    cx.dispatch_action(SendWorkbook);
    shared_file(cx);
    cx.dispatch_action(InvitePeople);
    assert_eq!(
        alert(&app, cx).map(|(title, _)| title).as_deref(),
        Some("Invite People")
    );
    cx.simulate_keystrokes("enter");
    cx.dispatch_action(CopyLink);
    assert_eq!(
        alert(&app, cx).map(|(title, _)| title).as_deref(),
        Some("Copy Link")
    );
}
