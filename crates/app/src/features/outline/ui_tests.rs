//! Outlines driven in GPUI's headless window: commands, dialogs, keys and
//! clicks in the margin. What these tests cannot tell is how things look.

use super::*;
use crate::app::ExcelApp;
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::protect::SheetProtection;
use opensheet_engine::subtotal::SubtotalFn;
use opensheet_engine::{MAX_COLS, MAX_ROWS, Rect, Workbook};

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

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

/// Change the workbook as a command would, and draw the result.
fn edit(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, f: impl FnOnce(&mut Workbook)) {
    app.update(cx, |app, cx| {
        f(&mut app.wb);
        app.commit("setup", cx);
    });
    redraw(cx);
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, rect: Rect) {
    app.update(cx, |app, cx| app.select_rect(rect, cx));
    redraw(cx);
}

fn rows(lo: u32, hi: u32) -> Rect {
    Rect::new(lo, 0, hi, MAX_COLS - 1)
}

fn cols(lo: u32, hi: u32) -> Rect {
    Rect::new(0, lo, MAX_ROWS - 1, hi)
}

fn cells(name: &str) -> Rect {
    Rect::parse(name).expect("a range")
}

fn modal_open(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> bool {
    app.update(cx, |app, _| app.modal_open())
}

fn row_levels(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, lo: u32, hi: u32) -> Vec<u8> {
    app.update(cx, |app, _| {
        (lo..=hi)
            .map(|r| app.wb.sheets[app.sheet].outline.rows.level(r))
            .collect()
    })
}

fn col_levels(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, lo: u32, hi: u32) -> Vec<u8> {
    app.update(cx, |app, _| {
        (lo..=hi)
            .map(|c| app.wb.sheets[app.sheet].outline.cols.level(c))
            .collect()
    })
}

fn hidden_rows(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u32> {
    app.update(cx, |app, _| {
        app.wb.sheets[app.sheet]
            .hidden_rows
            .iter()
            .copied()
            .collect()
    })
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

fn text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, name: &str) -> String {
    let (row, col) = opensheet_engine::refs::parse_cell_name(name).expect("a cell name");
    app.update(cx, |app, _| app.wb.display_text(app.sheet, row, col).text)
}

/// The middle of the zone the margin painted for `hit`, in the window.
fn zone_of(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    wanted: impl Fn(&Hit) -> bool,
) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let zone = app
            .zones
            .iter()
            .rev()
            .find(|zone| zone.payload::<Hit>().is_some_and(&wanted))
            .expect("the symbol is painted");
        let layout = app.current_layout().unwrap();
        layout.to_window(
            (zone.bounds.0 + zone.bounds.2) / 2.0,
            (zone.bounds.1 + zone.bounds.3) / 2.0,
        )
    })
}

/// Rows 2-4 and 6-8 grouped under rows 5 and 9, all of them under row 10.
fn nested(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    edit(app, cx, |wb| {
        wb.outline_group(0, Axis::Rows, 1, 8).unwrap();
        wb.outline_group(0, Axis::Rows, 1, 3).unwrap();
        wb.outline_group(0, Axis::Rows, 5, 7).unwrap();
    });
}

#[gpui::test]
fn whole_rows_group_at_once_and_with_the_keys(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, rows(1, 3));
    run(&app, cx, Cmd::Group);
    assert!(!modal_open(&app, cx));
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 1, 1, 1, 0]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Group"));
    // The margin took its room beside the row headings.
    let margin = app.update(cx, |app, _| app.current_layout().unwrap().margin_w);
    assert!(margin > 0.0);
    cx.simulate_keystrokes("alt-shift-right");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 2, 2, 2, 0]);
    assert!(app.update(cx, |app, _| app.current_layout().unwrap().margin_w) > margin);
    cx.simulate_keystrokes("alt-shift-left");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 1, 1, 1, 0]);
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 0, 0, 0, 0]);
    assert_eq!(
        app.update(cx, |app, _| app.current_layout().unwrap().margin_w),
        0.0
    );
    // Whole columns likewise.
    select(&app, cx, cols(2, 3));
    cx.simulate_keystrokes("cmd-shift-k");
    redraw(cx);
    assert_eq!(col_levels(&app, cx, 1, 4), [0, 1, 1, 0]);
    assert!(app.update(cx, |app, _| app.current_layout().unwrap().margin_h) > 0.0);
    run(&app, cx, Cmd::Ungroup);
    assert_eq!(col_levels(&app, cx, 1, 4), [0, 0, 0, 0]);
}

#[gpui::test]
fn cells_ask_rows_or_columns(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, cells("B2:C4"));
    run(&app, cx, Cmd::Group);
    assert!(modal_open(&app, cx));
    // Rows are proposed: Return groups them.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 1, 1, 1, 0]);
    // Columns, chosen in the dialog.
    let dialog = app
        .update_in(cx, |app, window, cx| {
            group_selection(app, false, window, cx)
        })
        .expect("the dialog opens");
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.choose(Axis::Cols, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(col_levels(&app, cx, 0, 3), [0, 1, 1, 0]);
    // Escape leaves everything as it is.
    run(&app, cx, Cmd::Ungroup);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 0, 4), [0, 1, 1, 1, 0]);
    // Ungrouping what is not grouped is refused with Excel's message.
    select(&app, cx, rows(20, 21));
    run(&app, cx, Cmd::Ungroup);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
}

#[gpui::test]
fn the_margin_folds_and_unfolds(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    nested(&app, cx);
    let inner = Group {
        level: 2,
        first: 1,
        last: 3,
    };
    // The − button of the first inner group.
    let button = zone_of(&app, cx, |hit| *hit == Hit::Fold(Axis::Rows, inner, true));
    click(cx, button);
    assert_eq!(hidden_rows(&app, cx), [1, 2, 3]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Hide Detail"));
    // It became a + button, which brings the rows back.
    let button = zone_of(&app, cx, |hit| *hit == Hit::Fold(Axis::Rows, inner, false));
    click(cx, button);
    assert!(hidden_rows(&app, cx).is_empty());
    // Level buttons.
    let one = zone_of(&app, cx, |hit| *hit == Hit::Level(Axis::Rows, 1));
    click(cx, one);
    assert_eq!(hidden_rows(&app, cx), [1, 2, 3, 4, 5, 6, 7, 8]);
    let two = zone_of(&app, cx, |hit| *hit == Hit::Level(Axis::Rows, 2));
    click(cx, two);
    assert_eq!(hidden_rows(&app, cx), [1, 2, 3, 5, 6, 7]);
    let three = zone_of(&app, cx, |hit| *hit == Hit::Level(Axis::Rows, 3));
    click(cx, three);
    assert!(hidden_rows(&app, cx).is_empty());
    // A click in the margin selects no cell.
    let before = app.update(cx, |app, _| app.sel().clone());
    let margin = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        layout.to_window(layout.margin_w - 3.0, layout.header_h + 200.0)
    });
    click(cx, margin);
    assert_eq!(app.update(cx, |app, _| app.sel().clone()), before);
    assert!(hidden_rows(&app, cx).is_empty());
    // Cells are still where the pointer finds them, past the margin.
    let cell = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let (x0, y0, x1, y1) = layout.rect_px(Rect::cell(11, 1));
        layout.to_window((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    });
    click(cx, cell);
    assert_eq!(app.update(cx, |app, _| app.active()), (11, 1));
}

#[gpui::test]
fn detail_commands_and_the_symbols_switch(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    nested(&app, cx);
    select(&app, cx, cells("B5"));
    run(&app, cx, Cmd::HideDetail);
    assert_eq!(hidden_rows(&app, cx), [1, 2, 3]);
    run(&app, cx, Cmd::ShowDetail);
    assert!(hidden_rows(&app, cx).is_empty());
    // Nothing to show or hide: Excel's message.
    select(&app, cx, cells("B30"));
    run(&app, cx, Cmd::HideDetail);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // Ctrl+8 takes the margin away and brings it back.
    cx.simulate_keystrokes("ctrl-8");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| app.current_layout().unwrap().margin_w),
        0.0
    );
    assert!(app.update(cx, |app, _| {
        app.zones.iter().all(|zone| zone.payload::<Hit>().is_none())
    }));
    cx.simulate_keystrokes("ctrl-8");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.current_layout().unwrap().margin_w) > 0.0);
    // Clear Outline on a single cell clears the sheet's.
    run(&app, cx, Cmd::ClearOutline);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].outline.is_empty()));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 1, 4), [2, 2, 2, 1]);
}

#[gpui::test]
fn auto_outline_and_the_settings(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // Without formulas there is nothing to make an outline from.
    run(&app, cx, Cmd::AutoOutline);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    edit(&app, cx, |wb| {
        for (row, entry) in ["Sales", "1", "2", "=SUM(A2:A3)", "4", "5", "=SUM(A5:A6)"]
            .iter()
            .enumerate()
        {
            wb.set_cell_text(0, row as u32, 0, entry).unwrap();
        }
    });
    run(&app, cx, Cmd::AutoOutline);
    assert!(!modal_open(&app, cx));
    assert_eq!(row_levels(&app, cx, 0, 6), [0, 1, 1, 0, 1, 1, 0]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Auto Outline"));
    // Ctrl+8 on a sheet without outline offers to make one.
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    cx.simulate_keystrokes("ctrl-8");
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(row_levels(&app, cx, 0, 3), [0, 1, 1, 0]);
    // Settings: summaries above.
    let dialog = app.update_in(cx, |app, window, cx| {
        dialogs::open_settings(app, window, cx)
    });
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.set(false, true, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert!(!app.update(cx, |app, _| app.wb.sheets[0].outline.summary_below));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].outline.summary_below));
}

#[gpui::test]
fn a_protected_sheet_keeps_its_outline(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    nested(&app, cx);
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, SheetProtection::default(), "");
    });
    assert!(app.update(cx, |app, _| !Cmd::Group.enabled(app)
        && !Cmd::Subtotal.enabled(app)
        && !Cmd::ClearOutline.enabled(app)));
    // A click on a button is answered by the message of protected sheets.
    let inner = Group {
        level: 2,
        first: 1,
        last: 3,
    };
    let button = zone_of(&app, cx, |hit| *hit == Hit::Fold(Axis::Rows, inner, true));
    click(cx, button);
    assert!(modal_open(&app, cx));
    assert!(hidden_rows(&app, cx).is_empty());
}

// ------------------------------------------------------------------ Subtotal

fn list(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    edit(app, cx, |wb| {
        let rows = [
            ["Region", "Item", "Amount"],
            ["North", "a", "10"],
            ["North", "b", "20"],
            ["South", "a", "40"],
            ["South", "b", "50"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, entry) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, entry).unwrap();
            }
        }
    });
}

#[gpui::test]
fn subtotal_dialog_adds_and_removes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    list(&app, cx);
    select(&app, cx, cells("B3"));
    run(&app, cx, Cmd::Subtotal);
    assert!(modal_open(&app, cx));
    // The list is selected under the dialog; Return takes what is proposed.
    assert_eq!(app.update(cx, |app, _| app.sel().current()), cells("A1:C5"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(text(&app, cx, "A4"), "North Total");
    assert_eq!(text(&app, cx, "C4"), "30");
    assert_eq!(text(&app, cx, "A8"), "Grand Total");
    assert_eq!(text(&app, cx, "C8"), "120");
    assert_eq!(row_levels(&app, cx, 0, 7), [0, 2, 2, 1, 2, 2, 1, 0]);
    assert_eq!(app.update(cx, |app, _| app.sel().current()), cells("A1:C8"));
    assert!(app.update(cx, |app, _| app.current_layout().unwrap().margin_w) > 0.0);
    // One step back.
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Subtotal"));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(text(&app, cx, "A4"), "South");
    assert!(app.update(cx, |app, _| app.wb.sheets[0].outline.is_empty()));
    cx.simulate_keystrokes("cmd-shift-z");
    redraw(cx);
    assert_eq!(text(&app, cx, "A4"), "North Total");

    // Another function on another column, by the dialog's controls.
    select(&app, cx, cells("A2"));
    let dialog = app
        .update_in(cx, |app, window, cx| subtotal::open(app, window, cx))
        .expect("the dialog opens");
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        assert_eq!(dialog.spec.columns, [2]);
        dialog.edit(cx, |spec| spec.function = SubtotalFn::Count);
        dialog.tick(2, false, cx);
        dialog.tick(1, true, cx);
    });
    redraw(cx);
    dialog.update_in(cx, |dialog, window, cx| dialog.ok(&(), window, cx));
    redraw(cx);
    assert_eq!(text(&app, cx, "A4"), "North Count");
    assert_eq!(text(&app, cx, "B4"), "2");
    assert_eq!(text(&app, cx, "C4"), "");

    // Remove All.
    let dialog = app
        .update_in(cx, |app, window, cx| subtotal::open(app, window, cx))
        .expect("the dialog opens");
    redraw(cx);
    dialog.update_in(cx, |dialog, window, cx| dialog.remove_all(window, cx));
    redraw(cx);
    assert_eq!(text(&app, cx, "A4"), "South");
    assert_eq!(text(&app, cx, "A6"), "");
    assert!(app.update(cx, |app, _| app.wb.sheets[0].outline.is_empty()));
    assert_eq!(app.update(cx, |app, _| app.sel().current()), cells("A1:C5"));
}

#[gpui::test]
fn subtotal_refuses_tables_and_empty_requests(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    list(&app, cx);
    // No column ticked: Excel's message, and nothing changes.
    select(&app, cx, cells("A2"));
    let dialog = app
        .update_in(cx, |app, window, cx| subtotal::open(app, window, cx))
        .expect("the dialog opens");
    redraw(cx);
    dialog.update(cx, |dialog, cx| dialog.tick(2, false, cx));
    dialog.update_in(cx, |dialog, window, cx| dialog.ok(&(), window, cx));
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(text(&app, cx, "A4"), "South");
    assert!(!app.update(cx, |app, _| app.wb.can_undo()
        && app.wb.undo_label() == Some("Subtotal")));
    // A lone cell is no list.
    select(&app, cx, cells("H20"));
    run(&app, cx, Cmd::Subtotal);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    // Inside a table the command is greyed out.
    edit(&app, cx, |wb| {
        wb.add_table(0, Rect::parse("A1:C5").unwrap(), true)
            .unwrap();
    });
    select(&app, cx, cells("B3"));
    assert!(!app.update(cx, |app, _| Cmd::Subtotal.enabled(app)));
    select(&app, cx, cells("H20"));
    assert!(app.update(cx, |app, _| Cmd::Subtotal.enabled(app)));
}
