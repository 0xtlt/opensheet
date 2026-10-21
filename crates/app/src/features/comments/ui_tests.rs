//! The notes and comments interface driven in GPUI's headless window:
//! commands, keys, typing, the pointer and its timers. What these tests
//! cannot tell is how things look.

use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext, point};

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

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: Cell) {
    app.update(cx, |app, cx| app.select_cell(cell.0, cell.1, cx));
    redraw(cx);
}

/// A point of the window given in the cell's own pixels.
fn in_cell(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    cell: Cell,
    at: (f32, f32),
) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let (x0, y0, _, _) = layout.rect_px(Rect::cell(cell.0, cell.1));
        layout.to_window(x0 + at.0, y0 + at.1)
    })
}

fn centre(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: Cell) -> Point<Pixels> {
    in_cell(app, cx, cell, (30.0, 10.0))
}

/// Where the box of a note shows, in window coordinates (x0, y0, x1, y1).
fn box_of(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    cell: Cell,
) -> Option<(f32, f32, f32, f32)> {
    app.update(cx, |app, _| {
        let layout = app.current_layout()?;
        let shown = boxes(app, &layout)
            .into_iter()
            .find(|shown| shown.cell == cell)?;
        let (a, b) = (
            layout.to_window(shown.rect.0, shown.rect.1),
            layout.to_window(shown.rect.2, shown.rect.3),
        );
        Some((
            f32::from(a.x),
            f32::from(a.y),
            f32::from(b.x),
            f32::from(b.y),
        ))
    })
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

/// Press at `from`, move to `to` in two steps, release.
fn drag(cx: &mut VisualTestContext, from: (f32, f32), to: (f32, f32)) {
    let at = |p: (f32, f32)| point(px(p.0), px(p.1));
    let middle = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
    cx.simulate_mouse_down(at(from), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(at(middle), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(at(to), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(at(to), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

fn wait(cx: &mut VisualTestContext, time: Duration) {
    cx.executor().advance_clock(time);
    cx.run_until_parked();
}

fn note(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: Cell) -> Option<Note> {
    app.update(cx, |app, _| {
        app.wb
            .comment_at(app.sheet, cell.0, cell.1)
            .and_then(Comment::as_note)
            .cloned()
    })
}

fn note_text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: Cell) -> Option<String> {
    note(app, cx, cell).map(|note| note.text())
}

fn thread(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    cell: Cell,
) -> Option<comment::Thread> {
    app.update(cx, |app, _| {
        app.wb
            .comment_at(app.sheet, cell.0, cell.1)
            .and_then(Comment::as_thread)
            .cloned()
    })
}

/// What the feature remembers, looked at.
fn ui<R>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, f: impl FnOnce(&Ui) -> R) -> R {
    app.update(cx, |app, _| f(app.ext_mut::<Ui>()))
}

fn editing(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<Cell> {
    ui(app, cx, |ui| ui.note_edit.as_ref().map(|edit| edit.cell))
}

/// The cell of the floating card, and whether it stays when the pointer leaves.
fn card(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<(Cell, bool)> {
    ui(app, cx, |ui| {
        ui.card.as_ref().map(|card| (card.cell, card.pinned))
    })
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

fn bob() -> Person {
    Person::new("Bob Martin")
}

// -------------------------------------------------------------------- notes

#[gpui::test]
fn a_new_note_is_typed_in_place(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (2, 1));
    cx.simulate_keystrokes("shift-f2");
    assert_eq!(editing(&app, cx), Some((2, 1)));
    // The name of the author is there already; Return starts a new line.
    cx.simulate_input("Check");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("this");
    // Nothing is in the workbook while the box is open.
    assert_eq!(note_text(&app, cx, (2, 1)), None);
    // Escape keeps what was typed and gives the keyboard back to the grid.
    cx.simulate_keystrokes("escape");
    assert_eq!(editing(&app, cx), None);
    let written = note(&app, cx, (2, 1)).expect("a note");
    assert_eq!(written.text(), format!("{}:\nCheck\nthis", user_name()));
    assert_eq!(written.author, user_name());
    assert!(written.runs[0].font.bold && !written.visible);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("New Note"));
    // Typing goes to the cell again.
    cx.simulate_input("7");
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| {
        assert_eq!(
            app.wb.sheets[0].value_at(2, 1),
            opensheet_engine::Value::Number(7.0)
        )
    });
    // One undo step for the cell, one for the note.
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(note_text(&app, cx, (2, 1)), None);
}

#[gpui::test]
fn a_click_outside_finishes_the_note(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 1, 1, "Ada", "Ada:\nFirst");
    });
    select(&app, cx, (1, 1));
    // ⇧F2 on a cell that has a note opens it, the cursor at its end.
    cx.simulate_keystrokes("shift-f2");
    assert_eq!(editing(&app, cx), Some((1, 1)));
    cx.simulate_input(" draft");
    cx.simulate_keystrokes("backspace backspace backspace backspace backspace");
    cx.simulate_input("line");
    let elsewhere = centre(&app, cx, (8, 0));
    click(cx, elsewhere);
    assert_eq!(editing(&app, cx), None);
    assert_eq!(
        note_text(&app, cx, (1, 1)).as_deref(),
        Some("Ada:\nFirst line")
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Edit Note"));
    // The click went to the cell it landed on.
    app.update(cx, |app, _| assert_eq!(app.active(), (8, 0)));
    // The note keeps its author, whoever edits it.
    assert_eq!(note(&app, cx, (1, 1)).unwrap().author, "Ada");
}

#[gpui::test]
fn moving_in_the_text_of_a_note(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 0, 0, "Ada", "Ada:\nabc\ndef");
    });
    cx.simulate_keystrokes("shift-f2");
    // Up keeps the column; Home goes to the start of the line.
    cx.simulate_keystrokes("up");
    cx.simulate_input("X");
    cx.simulate_keystrokes("home");
    cx.simulate_input("Y");
    // ⌘Down goes to the end of the text, ⇧⌘Up selects back to its start.
    cx.simulate_keystrokes("cmd-down");
    cx.simulate_input("Z");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        note_text(&app, cx, (0, 0)).as_deref(),
        Some("Ada:\nYabcX\ndefZ")
    );
    cx.simulate_keystrokes("shift-f2 cmd-shift-up");
    cx.simulate_input("All new");
    cx.simulate_keystrokes("escape");
    assert_eq!(note_text(&app, cx, (0, 0)).as_deref(), Some("All new"));
}

#[gpui::test]
fn the_note_commands(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 1, 1, "Ada", "Ada:\nOne");
        wb.set_note(0, 4, 2, "Ada", "Ada:\nTwo");
    });
    select(&app, cx, (1, 1));
    // Show/Hide Note leaves the box of the active cell on the sheet.
    assert_eq!(box_of(&app, cx, (1, 1)), None);
    run(&app, cx, Cmd::ShowHideNote);
    assert!(note(&app, cx, (1, 1)).unwrap().visible);
    assert!(box_of(&app, cx, (1, 1)).is_some() && box_of(&app, cx, (4, 2)).is_none());
    // Show All Notes shows them all, then hides them all.
    app.update(cx, |app, _| assert!(!Cmd::ShowAllNotes.checked(app)));
    run(&app, cx, Cmd::ShowAllNotes);
    assert!(note(&app, cx, (4, 2)).unwrap().visible);
    app.update(cx, |app, _| assert!(Cmd::ShowAllNotes.checked(app)));
    assert!(box_of(&app, cx, (4, 2)).is_some());
    run(&app, cx, Cmd::ShowAllNotes);
    assert!(!note(&app, cx, (1, 1)).unwrap().visible && !note(&app, cx, (4, 2)).unwrap().visible);
    // Delete Note; the commands of a note are greyed on a bare cell.
    run(&app, cx, Cmd::DeleteNote);
    assert_eq!(note_text(&app, cx, (1, 1)), None);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Delete Note"));
    app.update(cx, |app, _| {
        assert!(
            !Cmd::EditNote.enabled(app)
                && !Cmd::DeleteNote.enabled(app)
                && !Cmd::ShowHideNote.enabled(app)
        );
        assert!(Cmd::NewNote.enabled(app) && Cmd::ConvertToComments.enabled(app));
    });
    // Review ▸ Delete takes the notes of the whole selection.
    app.update(cx, |app, cx| app.select_rect(Rect::new(0, 0, 9, 9), cx));
    app.update(cx, |app, _| assert!(Cmd::Delete.enabled(app)));
    run(&app, cx, Cmd::Delete);
    assert_eq!(note_text(&app, cx, (4, 2)), None);
    app.update(cx, |app, _| {
        assert!(!Cmd::Delete.enabled(app) && !Cmd::ConvertToComments.enabled(app))
    });
}

#[gpui::test]
fn the_pointer_shows_a_note_after_a_moment(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 3, 2, "Ada", "Ada:\nHidden");
    });
    let on_cell = centre(&app, cx, (3, 2));
    let away = centre(&app, cx, (9, 6));
    cx.simulate_mouse_move(on_cell, None, Modifiers::none());
    assert_eq!(box_of(&app, cx, (3, 2)), None);
    wait(cx, REST - Duration::from_millis(50));
    assert_eq!(box_of(&app, cx, (3, 2)), None);
    wait(cx, Duration::from_millis(60));
    // The box is beside the cell: 15 pixels right of it, a little above.
    let shown = box_of(&app, cx, (3, 2)).expect("the note shows");
    let corner = in_cell(&app, cx, (3, 2), (0.0, 0.0));
    assert_eq!(shown.0 - f32::from(corner.x), 64.0 + 15.0);
    assert_eq!(f32::from(corner.y) - shown.1, 10.0);
    assert_eq!((shown.2 - shown.0, shown.3 - shown.1), (144.0, 79.0));
    // The status bar says whose note it is.
    app.update(cx, |app, _| {
        let items = Comments.status_items(app);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text.as_ref(), "Cell C4 note by Ada");
    });
    // Moving inside the cell keeps it; leaving takes it away.
    let still_on = in_cell(&app, cx, (3, 2), (5.0, 4.0));
    cx.simulate_mouse_move(still_on, None, Modifiers::none());
    wait(cx, Duration::from_millis(500));
    assert!(box_of(&app, cx, (3, 2)).is_some());
    cx.simulate_mouse_move(away, None, Modifiers::none());
    wait(cx, LINGER_NOTE + Duration::from_millis(10));
    assert_eq!(box_of(&app, cx, (3, 2)), None);
    app.update(cx, |app, _| assert!(Comments.status_items(app).is_empty()));
    // A pointer that only passes shows nothing.
    cx.simulate_mouse_move(on_cell, None, Modifiers::none());
    wait(cx, Duration::from_millis(100));
    cx.simulate_mouse_move(away, None, Modifiers::none());
    wait(cx, REST * 2);
    assert_eq!(box_of(&app, cx, (3, 2)), None);
}

#[gpui::test]
fn a_shown_note_is_edited_moved_and_resized_with_the_mouse(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 2, 1, "Ada", "Ada:\nShown");
        wb.set_note_visible(0, 2, 1, true);
    });
    let (x0, y0, x1, y1) = box_of(&app, cx, (2, 1)).expect("a note left showing has its box");
    let frame = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1).unwrap());

    // A click inside opens the text at the pointer: here before "Ada".
    click(cx, point(px(x0 + 5.0), px(y0 + 6.0)));
    assert_eq!(editing(&app, cx), Some((2, 1)));
    cx.simulate_input(">");
    // A press on the border keeps the text and moves the box.
    drag(cx, (x0 + 40.0, y0), (x0 + 70.0, y0 + 25.0));
    assert_eq!(editing(&app, cx), None);
    assert_eq!(note_text(&app, cx, (2, 1)).as_deref(), Some(">Ada:\nShown"));
    let moved = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1).unwrap());
    assert_eq!((moved.x - frame.x, moved.y - frame.y), (30.0, 25.0));
    assert_eq!((moved.width, moved.height), (frame.width, frame.height));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Move Object"));
    // The box is selected now: its bottom-right handle resizes it.
    assert_eq!(ui(&app, cx, |ui| ui.note_sel), Some((2, 1)));
    redraw(cx);
    let (_, _, x1, y1) = (x0 + 30.0, y0 + 25.0, x1 + 30.0, y1 + 25.0);
    drag(cx, (x1 + 2.0, y1 + 2.0), (x1 + 42.0, y1 + 12.0));
    let sized = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1).unwrap());
    assert_eq!((sized.x, sized.y), (moved.x, moved.y));
    assert_eq!(
        (sized.width - moved.width, sized.height - moved.height),
        (40.0, 10.0)
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Size Object"));
    // The arrows nudge the selected box, Delete removes the note.
    redraw(cx);
    cx.simulate_keystrokes("right down");
    let nudged = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1).unwrap());
    assert_eq!((nudged.x - sized.x, nudged.y - sized.y), (1.0, 1.0));
    cx.simulate_keystrokes("backspace");
    assert_eq!(note_text(&app, cx, (2, 1)), None);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Delete Note"));
    // Each of these was one undo step.
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z cmd-z cmd-z");
    let back = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1).unwrap());
    assert_eq!(back, frame);
    assert_eq!(note_text(&app, cx, (2, 1)).as_deref(), Some(">Ada:\nShown"));
}

#[gpui::test]
fn next_and_previous_note_walk_the_workbook(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_sheet(1);
        wb.set_note(0, 2, 1, "Ada", "first");
        wb.add_comment(0, 3, 0, &bob(), 45_000.0, "a thread between them");
        wb.set_note(1, 0, 4, "Ada", "second");
    });
    app.update(cx, |app, _| {
        assert!(Cmd::NextNote.enabled(app) && Cmd::Next.enabled(app))
    });
    run(&app, cx, Cmd::NextNote);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (0, (2, 1)))
    });
    // The note that was reached shows, without the pointer.
    assert!(box_of(&app, cx, (2, 1)).is_some());
    run(&app, cx, Cmd::NextNote);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (1, (0, 4)))
    });
    assert!(box_of(&app, cx, (0, 4)).is_some());
    // Past the last note Excel asks before starting again; Cancel stays.
    run(&app, cx, Cmd::NextNote);
    app.update(cx, |app, _| assert!(app.modal_open()));
    cx.simulate_keystrokes("escape");
    app.update(cx, |app, _| {
        assert_eq!(
            (app.modal_open(), app.sheet, app.active()),
            (false, 1, (0, 4))
        )
    });
    run(&app, cx, Cmd::NextNote);
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| {
        assert_eq!(
            (app.modal_open(), app.sheet, app.active()),
            (false, 0, (2, 1))
        )
    });
    // Backwards from the first one, to the last.
    run(&app, cx, Cmd::PreviousNote);
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (1, (0, 4)))
    });
    // The box goes when the selection moves on.
    select(&app, cx, (5, 5));
    assert_eq!(box_of(&app, cx, (0, 4)), None);
}

#[gpui::test]
fn notes_are_converted_after_a_question(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 1, 1, "Ada", "Ada:\nOne");
        wb.set_note(0, 4, 2, "Bob", "Two");
    });
    run(&app, cx, Cmd::ConvertToComments);
    app.update(cx, |app, _| assert!(app.modal_open()));
    cx.simulate_keystrokes("escape");
    assert!(note(&app, cx, (1, 1)).is_some());
    run(&app, cx, Cmd::ConvertToComments);
    cx.simulate_keystrokes("enter");
    let first = thread(&app, cx, (1, 1)).expect("a thread").first;
    assert_eq!(
        (first.author.name.as_str(), first.text.as_str()),
        ("Ada", "One")
    );
    assert_eq!(thread(&app, cx, (4, 2)).unwrap().first.text, "Two");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Convert to Comments"));
    cx.simulate_keystrokes("cmd-z");
    assert!(note(&app, cx, (1, 1)).is_some() && note(&app, cx, (4, 2)).is_some());

    // New Comment on a cell that has a note offers to convert that one.
    select(&app, cx, (4, 2));
    cx.simulate_keystrokes("cmd-shift-m");
    app.update(cx, |app, _| assert!(app.modal_open()));
    cx.simulate_keystrokes("enter");
    assert_eq!(thread(&app, cx, (4, 2)).unwrap().first.text, "Two");
    assert!(note(&app, cx, (1, 1)).is_some());
    // Its card is open for the reply.
    assert_eq!(card(&app, cx), Some(((4, 2), true)));
    cx.simulate_input("Seen");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(thread(&app, cx, (4, 2)).unwrap().replies[0].text, "Seen");
}

#[gpui::test]
fn boxes_are_drawn_at_any_zoom_and_with_frozen_panes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        for row in 0..40 {
            wb.set_note(
                0,
                row,
                row % 7,
                "Ada",
                "Ada:\nA note long enough to wrap over two or three lines.",
            );
            wb.add_comment(0, row, 8, &bob(), 45_000.0, "thread");
        }
        wb.set_all_notes_visible(0, true);
        wb.set_frozen(0, 2, 1);
    });
    for zoom in [0.5, 1.0, 2.5] {
        app.update(cx, |app, cx| app.set_zoom(zoom, cx));
        redraw(cx);
        for (sx, sy) in [(0.0, 0.0), (37.0, 211.0), (400.0, 90.0)] {
            app.update(cx, |app, cx| {
                app.views[0].scroll_x = sx;
                app.views[0].scroll_y = sy;
                cx.notify();
            });
            redraw(cx);
        }
    }
    // A box anchored right of the frozen column slides under it: what it
    // may draw on starts after the frozen pane.
    app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let limit = layout.header_w + layout.frozen_w;
        let scrolled: Vec<_> = boxes(app, &layout)
            .into_iter()
            .filter(|shown| shown.cell.1 >= 1)
            .collect();
        assert!(!scrolled.is_empty() && scrolled.iter().all(|shown| shown.clip.0 == limit));
    });
}

// ----------------------------------------------------------------- comments

#[gpui::test]
fn a_comment_is_written_posted_and_answered(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (3, 1));
    // ⌘⇧M opens an empty card with the cursor in it.
    cx.simulate_keystrokes("cmd-shift-m");
    assert_eq!(card(&app, cx), Some(((3, 1), true)));
    assert_eq!(thread(&app, cx, (3, 1)), None);
    cx.simulate_input("Is this right?");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("Second line");
    // ⌘Return posts; the card stays, the keyboard goes back to the grid.
    cx.simulate_keystrokes("cmd-enter");
    let posted = thread(&app, cx, (3, 1)).expect("a thread");
    assert_eq!(posted.first.text, "Is this right?\nSecond line");
    assert_eq!(posted.first.author.name, user_name());
    assert!(posted.replies.is_empty() && !posted.resolved);
    assert!((posted.first.time - comment::now_utc()).abs() < 0.01);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("New Comment"));
    assert_eq!(card(&app, cx), Some(((3, 1), true)));
    redraw(cx);
    // The same shortcut on a cell that has a thread is a reply.
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Yes.");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(thread(&app, cx, (3, 1)).unwrap().replies.len(), 1);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Reply"));
    // An empty box posts nothing.
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("   ");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(thread(&app, cx, (3, 1)).unwrap().replies.len(), 1);
    // Escape empties the box, then puts the card away.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(card(&app, cx), Some(((3, 1), true)));
    cx.simulate_keystrokes("cmd-shift-m escape");
    redraw(cx);
    assert_eq!(card(&app, cx), None);
    // Typing goes to the cell again.
    cx.simulate_input("5");
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| {
        assert_eq!(
            app.wb.sheets[0].value_at(3, 1),
            opensheet_engine::Value::Number(5.0)
        )
    });
}

#[gpui::test]
fn a_new_comment_can_be_given_up(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (1, 1));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Never mind");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!((card(&app, cx), thread(&app, cx, (1, 1))), (None, None));
    app.update(cx, |app, _| assert!(!app.wb.can_undo()));
}

#[gpui::test]
fn a_click_on_a_comment_opens_its_card(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
        wb.add_comment(0, 12, 0, &bob(), 45_000.0, "Another");
    });
    let first = centre(&app, cx, (2, 2));
    click(cx, first);
    app.update(cx, |app, _| assert_eq!(app.active(), (2, 2)));
    assert_eq!(card(&app, cx), Some(((2, 2), true)));
    // It is beside its cell, inside the grid.
    let cell = in_cell(&app, cx, (2, 2), (0.0, 0.0));
    let shown = cx.debug_bounds("comment-card").expect("the card is drawn");
    assert_eq!(f32::from(shown.left() - cell.x), 64.0 + 6.0);
    assert_eq!(shown.top(), cell.y);
    assert_eq!(f32::from(shown.size.width), card::WIDTH);
    // A click on the card is not a click on the cells under it.
    click(cx, point(shown.left() + px(20.), shown.top() + px(12.)));
    app.update(cx, |app, _| assert_eq!(app.active(), (2, 2)));
    // A click on another cell puts it away; on another comment, opens that one.
    let other = centre(&app, cx, (12, 0));
    click(cx, other);
    assert_eq!(card(&app, cx), Some(((12, 0), true)));
    let bare = centre(&app, cx, (1, 0));
    click(cx, bare);
    assert_eq!(card(&app, cx), None);
    // So do the arrow keys.
    click(cx, first);
    cx.simulate_keystrokes("down");
    redraw(cx);
    assert_eq!(card(&app, cx), None);
}

#[gpui::test]
fn the_pointer_shows_the_card_of_a_comment(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
    });
    let on_cell = centre(&app, cx, (2, 2));
    let away = centre(&app, cx, (14, 1));
    cx.simulate_mouse_move(on_cell, None, Modifiers::none());
    wait(cx, REST + Duration::from_millis(10));
    redraw_at(cx, on_cell);
    assert_eq!(card(&app, cx), Some(((2, 2), false)));
    let shown = cx.debug_bounds("comment-card").expect("the card is drawn");
    // Going from the cell onto the card keeps it...
    let on_card = point(shown.left() + px(30.), shown.top() + px(14.));
    cx.simulate_mouse_move(on_card, None, Modifiers::none());
    wait(cx, LINGER_CARD * 3);
    redraw_at(cx, on_card);
    assert_eq!(card(&app, cx), Some(((2, 2), false)));
    // ...a press on it makes it stay...
    cx.simulate_click(on_card, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(card(&app, cx), Some(((2, 2), true)));
    // ...and one that was only looked at goes once the pointer has left.
    app.update(cx, |app, cx| close_card(app, 0, (2, 2), cx));
    cx.simulate_mouse_move(away, None, Modifiers::none());
    cx.simulate_mouse_move(on_cell, None, Modifiers::none());
    wait(cx, REST + Duration::from_millis(10));
    redraw_at(cx, on_cell);
    assert_eq!(card(&app, cx), Some(((2, 2), false)));
    cx.simulate_mouse_move(away, None, Modifiers::none());
    wait(cx, LINGER_CARD + Duration::from_millis(10));
    assert_eq!(card(&app, cx), None);
}

/// Draw the window without moving the pointer away from `at`.
fn redraw_at(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.run_until_parked();
    cx.simulate_mouse_move(at, None, Modifiers::none());
}

#[gpui::test]
fn threads_are_edited_resolved_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
        wb.add_reply(0, 2, 2, &Person::new("Ada"), 45_000.1, "Not yet");
        wb.add_reply(0, 2, 2, &bob(), 45_000.2, "Please do");
    });
    select(&app, cx, (2, 2));
    run(&app, cx, Cmd::Reply);
    let view = ui(&app, cx, |ui| ui.card.as_ref().unwrap().view.clone());
    // Rewriting a reply in its card: ⌘Return saves, Escape gives up.
    view.update_in(cx, |card, window, cx| {
        card.start_edit(Some(0), &Post::new(&bob(), 45_000.0, "Not yet"), window, cx)
    });
    redraw(cx);
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("Done now");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        thread(&app, cx, (2, 2)).unwrap().replies[0].text,
        "Done now"
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Edit Comment"));
    view.update_in(cx, |card, window, cx| {
        card.start_edit(None, &Post::new(&bob(), 45_000.0, "Checked?"), window, cx)
    });
    redraw(cx);
    cx.simulate_input(" really");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().first.text, "Checked?");
    assert_eq!(card(&app, cx), Some(((2, 2), true)));

    // What the "…" menus do.
    app.update(cx, |app, cx| delete_reply(app, 0, (2, 2), 1, cx));
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies.len(), 1);
    app.update(cx, |app, cx| resolve(app, 0, (2, 2), true, cx));
    assert!(thread(&app, cx, (2, 2)).unwrap().resolved);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Resolve thread"));
    redraw(cx);
    // A resolved thread takes no reply until it is reopened.
    app.update(cx, |app, cx| {
        assert!(!post(app, 0, (2, 2), "Too late", Vec::new(), cx))
    });
    app.update(cx, |app, cx| resolve(app, 0, (2, 2), false, cx));
    app.update(cx, |app, cx| {
        assert!(post(app, 0, (2, 2), "In time", Vec::new(), cx))
    });
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies.len(), 2);
    // Deleting the thread takes its card with it.
    app.update(cx, |app, cx| delete_thread(app, 0, (2, 2), cx));
    redraw(cx);
    assert_eq!((thread(&app, cx, (2, 2)), card(&app, cx)), (None, None));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies.len(), 2);
    // Review ▸ Delete does the same for the active cell.
    app.update(cx, |app, _| {
        assert!(Cmd::DeleteComment.enabled(app) && !Cmd::NewNote.enabled(app))
    });
    run(&app, cx, Cmd::DeleteComment);
    assert_eq!(thread(&app, cx, (2, 2)), None);
}

#[gpui::test]
fn words_not_posted_are_not_lost(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (1, 1));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Half a thought");
    // Going to another cell keeps the card and its words.
    let elsewhere = centre(&app, cx, (9, 3));
    click(cx, elsewhere);
    app.update(cx, |app, _| assert_eq!(app.active(), (9, 3)));
    assert_eq!(card(&app, cx), Some(((1, 1), true)));
    // A new comment elsewhere asks first: back to the draft...
    cx.simulate_keystrokes("cmd-shift-m");
    app.update(cx, |app, _| assert!(app.modal_open()));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!((app.modal_open(), app.active()), (false, (1, 1)))
    });
    cx.simulate_input(", finished");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        thread(&app, cx, (1, 1)).unwrap().first.text,
        "Half a thought, finished"
    );
    // ...or drop it and write the new one.
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Reply never sent");
    select(&app, cx, (9, 3));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(card(&app, cx), Some(((9, 3), true)));
    cx.simulate_input("The other one");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        thread(&app, cx, (9, 3)).unwrap().first.text,
        "The other one"
    );
    assert!(thread(&app, cx, (1, 1)).unwrap().replies.is_empty());
}

#[gpui::test]
fn next_and_previous_comment_open_the_card(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_sheet(1);
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "one");
        wb.set_note(0, 3, 0, "Ada", "a note between them");
        wb.add_comment(1, 5, 1, &bob(), 45_000.0, "two");
    });
    run(&app, cx, Cmd::Next);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (0, (2, 2)))
    });
    assert_eq!(card(&app, cx), Some(((2, 2), true)));
    run(&app, cx, Cmd::Next);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (1, (5, 1)))
    });
    assert_eq!(card(&app, cx), Some(((5, 1), true)));
    // At the end of the workbook: the question, then the first one again.
    run(&app, cx, Cmd::Next);
    app.update(cx, |app, _| assert!(app.modal_open()));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (0, (2, 2)))
    });
    run(&app, cx, Cmd::Previous);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (1, (5, 1)))
    });
    run(&app, cx, Cmd::Previous);
    app.update(cx, |app, _| {
        assert_eq!((app.sheet, app.active()), (0, (2, 2)))
    });
}

// --------------------------------------------------------------------- pane

#[gpui::test]
fn the_comments_pane_lists_and_writes_threads(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 6, 4, &bob(), 45_000.0, "later in the sheet");
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "earlier");
        wb.set_note(0, 4, 0, "Ada", "notes are not listed");
    });
    app.update(cx, |app, _| assert!(!Cmd::ShowPane.checked(app)));
    run(&app, cx, Cmd::ShowPane);
    app.update(cx, |app, _| {
        assert!(app.pane_open(PANE) && Cmd::ShowPane.checked(app))
    });
    let pane = ui(&app, cx, |ui| ui.pane.clone().expect("the pane"));
    let listed = |cx: &mut VisualTestContext| pane.update(cx, |pane, cx| pane.cells(cx));
    assert_eq!(listed(cx), [(2, 2), (6, 4)]);
    // With the pane open, a click on a comment opens no card on the grid.
    let on_comment = centre(&app, cx, (6, 4));
    click(cx, on_comment);
    assert_eq!(card(&app, cx), None);
    // A click on a card of the pane goes to its cell.
    let first = cx
        .debug_bounds("pane-card-0")
        .expect("the first card is drawn");
    click(cx, point(first.left() + px(40.), first.top() + px(16.)));
    app.update(cx, |app, _| assert_eq!(app.active(), (2, 2)));
    // New Comment writes in the pane, at the place of its cell.
    select(&app, cx, (4, 1));
    cx.simulate_keystrokes("cmd-shift-m");
    redraw(cx);
    assert_eq!(listed(cx), [(2, 2), (4, 1), (6, 4)]);
    assert_eq!(card(&app, cx), None);
    cx.simulate_input("From the pane");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        thread(&app, cx, (4, 1)).unwrap().first.text,
        "From the pane"
    );
    // A reply from the pane, to the thread of the active cell.
    select(&app, cx, (6, 4));
    run(&app, cx, Cmd::Reply);
    cx.simulate_input("Answered here");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        thread(&app, cx, (6, 4)).unwrap().replies[0].text,
        "Answered here"
    );
    // A comment given up leaves the list; a deleted thread too.
    select(&app, cx, (9, 0));
    cx.simulate_keystrokes("cmd-shift-m");
    redraw(cx);
    assert_eq!(listed(cx).len(), 4);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(listed(cx), [(2, 2), (4, 1), (6, 4)]);
    app.update(cx, |app, cx| delete_thread(app, 0, (2, 2), cx));
    redraw(cx);
    assert_eq!(listed(cx), [(4, 1), (6, 4)]);
    // Closing the pane brings the cards back to the grid.
    run(&app, cx, Cmd::ShowPane);
    app.update(cx, |app, _| assert!(!app.pane_open(PANE)));
    click(cx, on_comment);
    assert_eq!(card(&app, cx), Some(((6, 4), true)));
}

#[gpui::test]
fn cut_cells_take_their_comments_along(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 1, 0, "c").unwrap();
        wb.set_cell_text(0, 2, 0, "a").unwrap();
        wb.set_note(0, 1, 0, "Ada", "on c");
        wb.add_comment(0, 2, 0, &bob(), 45_000.0, "on a");
    });
    // Cut A2:A3, paste on C5: the engine moves both with their cells.
    app.update(cx, |app, cx| {
        app.select_rect(Rect::new(1, 0, 2, 0), cx);
        app.copy(true, cx);
        app.select_cell(4, 2, cx);
        app.paste(opensheet_engine::ops::PasteMode::All, cx);
    });
    redraw(cx);
    assert_eq!(note_text(&app, cx, (4, 2)).as_deref(), Some("on c"));
    assert_eq!(thread(&app, cx, (5, 2)).unwrap().first.text, "on a");
    assert_eq!(
        (note(&app, cx, (1, 0)), thread(&app, cx, (2, 0))),
        (None, None)
    );
    // Inserting a row above pushes them down.
    app.update(cx, |app, cx| {
        app.select_cell(0, 0, cx);
        app.insert_rows(cx);
    });
    redraw(cx);
    assert_eq!(note_text(&app, cx, (5, 2)).as_deref(), Some("on c"));
    assert_eq!(thread(&app, cx, (6, 2)).unwrap().first.text, "on a");
}

// ------------------------------------------------ menus, buttons, the views

/// The centre of something a view named for the tests.
fn middle(cx: &mut VisualTestContext, name: &'static str) -> Point<Pixels> {
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("{name} is not drawn"))
        .center()
}

/// The labels of the menu that is open, separators left out.
fn menu(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<String> {
    app.update(cx, |app, _| {
        match app.popup.as_ref().map(|popup| &popup.kind) {
            Some(crate::app::PopupKind::Menu(entries)) => entries
                .iter()
                .filter(|entry| !entry.separator)
                .map(|entry| entry.label.to_string())
                .collect(),
            _ => Vec::new(),
        }
    })
}

/// Choose an entry of the open menu by its label.
fn choose(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, label: &str) {
    app.update_in(cx, |app, window, cx| {
        let Some(crate::app::PopupKind::Menu(entries)) = app.popup.take().map(|popup| popup.kind)
        else {
            panic!("no menu is open");
        };
        let entry = entries
            .iter()
            .find(|entry| entry.label.as_ref() == label)
            .expect("the entry");
        assert!(!entry.disabled, "{label} is greyed");
        (entry.action.clone().expect("an action"))(app, window, cx);
    });
    redraw(cx);
}

#[gpui::test]
fn the_card_is_used_with_the_mouse(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
        wb.add_reply(0, 2, 2, &Person::new("Ada"), 45_000.1, "Not yet");
    });
    let on_cell = centre(&app, cx, (2, 2));
    click(cx, on_cell);
    // A click in the reply box puts the cursor there; Post sends the reply.
    let field = middle(cx, "reply-field@C3");
    click(cx, field);
    cx.simulate_input("On it");
    redraw(cx);
    let post = middle(cx, "reply-field-accept@C3");
    click(cx, post);
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies[1].text, "On it");
    assert_eq!(card(&app, cx), Some(((2, 2), true)));
    // Cancel drops what was typed.
    let field = middle(cx, "reply-field@C3");
    click(cx, field);
    cx.simulate_input("Forget it");
    redraw(cx);
    let cancel = middle(cx, "reply-field-cancel@C3");
    click(cx, cancel);
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies.len(), 2);
    let view = ui(&app, cx, |ui| ui.card.as_ref().unwrap().view.clone());
    assert!(!view.update(cx, |card, cx| card.has_draft(cx)));

    // The "…" of the first comment: edit it, delete or resolve the thread.
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    assert_eq!(
        menu(&app, cx),
        ["Edit comment", "Delete thread", "Resolve thread"]
    );
    choose(&app, cx, "Edit comment");
    cx.simulate_input("!");
    redraw(cx);
    let save = middle(cx, "edit-field-accept@C3");
    click(cx, save);
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().first.text, "Checked?!");
    // The "…" of a reply: edit or delete that reply.
    let more = middle(cx, "post-menu-1@C3");
    click(cx, more);
    assert_eq!(menu(&app, cx), ["Edit comment", "Delete comment"]);
    choose(&app, cx, "Delete comment");
    let left = thread(&app, cx, (2, 2)).unwrap();
    assert_eq!(
        (left.replies.len(), left.replies[0].text.as_str()),
        (1, "On it")
    );
    // Resolve: no reply box, no "…", but Reopen.
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Resolve thread");
    assert!(thread(&app, cx, (2, 2)).unwrap().resolved);
    let reopen = middle(cx, "reopen-thread@C3");
    click(cx, reopen);
    assert!(!thread(&app, cx, (2, 2)).unwrap().resolved);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Reopen thread"));
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Delete thread");
    assert_eq!((thread(&app, cx, (2, 2)), card(&app, cx)), (None, None));
}

#[gpui::test]
fn the_menu_of_a_cell_follows_what_it_carries(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 1, 1, "Ada", "Ada:\nA note");
        wb.add_comment(0, 3, 1, &bob(), 45_000.0, "A thread");
    });
    let menu_of = |cell: Cell, cx: &mut VisualTestContext| {
        let at = centre(&app, cx, cell);
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        let labels = menu(&app, cx);
        app.update(cx, |app, _| assert_eq!(app.active(), cell));
        labels
    };
    let has = |labels: &[String], label: &str| labels.iter().any(|entry| entry == label);
    let bare = menu_of((6, 1), cx);
    assert!(has(&bare, "New Comment") && has(&bare, "New Note"));
    assert!(
        !has(&bare, "Edit Note") && !has(&bare, "Delete Note") && !has(&bare, "Delete Comment")
    );
    let noted = menu_of((1, 1), cx);
    assert!(
        has(&noted, "Edit Note") && has(&noted, "Delete Note") && has(&noted, "Show/Hide Note")
    );
    assert!(
        has(&noted, "New Comment") && !has(&noted, "New Note") && !has(&noted, "Delete Comment")
    );
    let commented = menu_of((3, 1), cx);
    assert!(has(&commented, "Reply to Comment") && has(&commented, "Delete Comment"));
    assert!(
        !has(&commented, "New Note")
            && !has(&commented, "New Comment")
            && !has(&commented, "Edit Note")
    );
    // The entries act on the cell that was pressed.
    choose(&app, cx, "Delete Comment");
    assert_eq!(thread(&app, cx, (3, 1)), None);
    menu_of((1, 1), cx);
    choose(&app, cx, "Show/Hide Note");
    assert!(note(&app, cx, (1, 1)).unwrap().visible);
    menu_of((1, 1), cx);
    choose(&app, cx, "Edit Note");
    assert_eq!(editing(&app, cx), Some((1, 1)));
    cx.simulate_input(" edited");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        note_text(&app, cx, (1, 1)).as_deref(),
        Some("Ada:\nA note edited")
    );
    menu_of((6, 1), cx);
    choose(&app, cx, "New Note");
    assert_eq!(editing(&app, cx), Some((6, 1)));
    cx.simulate_input("From the menu");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        note_text(&app, cx, (6, 1)),
        Some(format!("{}:\nFrom the menu", user_name()))
    );
}

#[gpui::test]
fn the_notes_menu_of_the_review_tab(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 1, 1, "Ada", "Ada:\nA note");
    });
    // The Review tab draws with its Comments group and its Notes button.
    app.update(cx, |app, cx| {
        app.ribbon_tab = 7;
        cx.notify();
    });
    redraw(cx);
    run(&app, cx, Cmd::NotesMenu);
    assert_eq!(
        menu(&app, cx),
        [
            "New Note",
            "Previous Note",
            "Next Note",
            "Show/Hide Note",
            "Show All Notes",
            "Convert to Comments"
        ]
    );
    choose(&app, cx, "New Note");
    assert_eq!(editing(&app, cx), Some((0, 0)));
    cx.simulate_keystrokes("escape");
    // On a cell that has a note the first entry edits it.
    select(&app, cx, (1, 1));
    run(&app, cx, Cmd::NotesMenu);
    assert_eq!(menu(&app, cx)[0], "Edit Note");
    choose(&app, cx, "Show All Notes");
    assert!(note(&app, cx, (0, 0)).unwrap().visible && note(&app, cx, (1, 1)).unwrap().visible);
    run(&app, cx, Cmd::NotesMenu);
    choose(&app, cx, "Next Note");
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| assert_eq!(app.active(), (0, 0)));
    // Home ▸ Clear ▸ Clear Comments and Notes empties the selection of them.
    app.update(cx, |app, cx| app.select_rect(Rect::new(0, 0, 5, 5), cx));
    run(&app, cx, Cmd::Clear);
    assert_eq!(
        (note(&app, cx, (0, 0)), note(&app, cx, (1, 1))),
        (None, None)
    );
    assert_eq!(
        undo_label(&app, cx).as_deref(),
        Some("Clear Comments and Notes")
    );
}

#[gpui::test]
fn the_pane_is_used_with_the_mouse(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "A thread");
    });
    run(&app, cx, Cmd::ShowPane);
    // "New" writes a comment on the active cell.
    select(&app, cx, (5, 0));
    let new = middle(cx, "pane-new-comment");
    click(cx, new);
    cx.simulate_input("Started in the pane");
    redraw(cx);
    let post = middle(cx, "new-field-accept@A6");
    click(cx, post);
    assert_eq!(
        thread(&app, cx, (5, 0)).unwrap().first.text,
        "Started in the pane"
    );
    // The reply box of a card of the pane.
    let first = cx.debug_bounds("pane-card-0").expect("the first card");
    let field = middle(cx, "reply-field@C3");
    assert!(first.contains(&field));
    click(cx, field);
    app.update(cx, |app, _| assert_eq!(app.active(), (2, 2)));
    cx.simulate_input("Answer");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies[0].text, "Answer");
    // Resolving from the pane.
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Resolve thread");
    assert!(thread(&app, cx, (2, 2)).unwrap().resolved);
}

#[gpui::test]
fn the_views_of_page_layout_keep_boxes_beside_their_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 3, 2, "Ada", "Ada:\nShown");
        wb.set_note_visible(0, 3, 2, true);
        wb.add_comment(0, 5, 1, &bob(), 45_000.0, "A thread");
    });
    for mode in [
        crate::features::ViewMode::Normal,
        crate::features::ViewMode::PageLayout,
        crate::features::ViewMode::PageBreakPreview,
    ] {
        app.update(cx, |app, cx| app.set_view_mode(mode, cx));
        redraw(cx);
        redraw(cx);
        // Wherever the view puts the cell, the box is 15 zoomed pixels
        // right of it and 10 above.
        let (zoom, cell) = app.update(cx, |app, _| {
            let layout = app.current_layout().unwrap();
            (layout.zoom, layout.rect_px(Rect::cell(3, 2)))
        });
        let shown = app.update(cx, |app, _| {
            let layout = app.current_layout().unwrap();
            boxes(app, &layout)
                .into_iter()
                .find(|shown| shown.cell == (3, 2))
                .expect("the box")
                .rect
        });
        assert!(
            (shown.0 - (cell.2 + 15.0 * zoom)).abs() <= 1.0,
            "{mode:?}: {shown:?} beside {cell:?}"
        );
        assert!(
            (shown.1 - (cell.1 - 10.0 * zoom)).abs() <= 1.0,
            "{mode:?}: {shown:?} beside {cell:?}"
        );
        // The card of a comment opens there too.
        let on_comment = centre(&app, cx, (5, 1));
        click(cx, on_comment);
        assert_eq!(card(&app, cx), Some(((5, 1), true)), "{mode:?}");
        assert!(cx.debug_bounds("comment-card").is_some(), "{mode:?}");
        select(&app, cx, (0, 0));
    }
}

#[gpui::test]
fn odd_cases(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "A thread");
        wb.set_note(0, 4, 2, "Ada", "Ada:\nA note");
    });
    // No comment on a hidden cell.
    app.update(cx, |app, cx| {
        app.select_cell(7, 1, cx);
        app.hide_rows(true, cx);
    });
    app.update(cx, |app, _| assert!(app.wb.sheets[0].is_row_hidden(7)));
    app.update_in(cx, |app, window, cx| {
        // The selection left the hidden row: go back to it as Go To would.
        app.views[0].sel = crate::grid::Selection::single(7, 1);
        Cmd::NewComment.run(app, window, cx);
        assert!(app.modal_open());
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(card(&app, cx), None);
    // Undoing the post that made a thread puts its card away.
    select(&app, cx, (9, 0));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Soon undone");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(card(&app, cx), Some(((9, 0), true)));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!((thread(&app, cx, (9, 0)), card(&app, cx)), (None, None));
    // A note is not typed on a cell that has a comment: ⇧F2 does nothing.
    select(&app, cx, (2, 2));
    cx.simulate_keystrokes("shift-f2");
    assert_eq!(editing(&app, cx), None);
    // A double click on a cell with a comment is for the cell.
    let on_comment = centre(&app, cx, (2, 2));
    cx.simulate_event(gpui::MouseDownEvent {
        position: on_comment,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    app.update(cx, |app, _| assert!(app.is_editing()));
    cx.simulate_keystrokes("escape");
    // Moving a box at another zoom: the frame moves by sheet pixels.
    edit(&app, cx, |wb| {
        wb.set_note_visible(0, 4, 2, true);
    });
    app.update(cx, |app, cx| app.set_zoom(2.0, cx));
    redraw(cx);
    let before = app.update(cx, |app, _| app.wb.sheets[0].note_frame(4, 2).unwrap());
    let (x0, y0, _, _) = box_of(&app, cx, (4, 2)).unwrap();
    drag(cx, (x0 + 50.0, y0), (x0 + 110.0, y0 + 40.0));
    let after = app.update(cx, |app, _| app.wb.sheets[0].note_frame(4, 2).unwrap());
    assert_eq!((after.x - before.x, after.y - before.y), (30.0, 20.0));
    // Escape lets go of the box; the arrows move the selection again.
    assert_eq!(ui(&app, cx, |ui| ui.note_sel), Some((4, 2)));
    redraw(cx);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(ui(&app, cx, |ui| ui.note_sel), None);
    let active = app.update(cx, |app, _| app.active());
    cx.simulate_keystrokes("down");
    app.update(cx, |app, _| {
        assert_eq!(app.active(), (active.0 + 1, active.1))
    });
}

#[gpui::test]
fn a_protected_sheet_refuses_changes_to_notes_and_comments(cx: &mut TestAppContext) {
    use opensheet_engine::protect::{Permission, SheetProtection};
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 2, 1, "Ada", "Ada:\nShown");
        wb.set_note_visible(0, 2, 1, true);
        wb.add_comment(0, 5, 1, &bob(), 45_000.0, "A thread");
    });
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
        cx.notify();
    });
    redraw(cx);
    // Each attempt is answered by Excel's message, and changes nothing.
    let refused = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, what: &str| {
        app.update(cx, |app, _| {
            assert!(app.modal_open(), "{what} was not refused")
        });
        cx.simulate_keystrokes("enter");
        app.update(cx, |app, _| {
            assert!(
                !app.modal_open() && !app.wb.can_undo(),
                "{what} changed the workbook"
            )
        });
        redraw(cx);
    };
    select(&app, cx, (8, 3));
    cx.simulate_keystrokes("shift-f2");
    assert_eq!(editing(&app, cx), None);
    refused(&app, cx, "New Note");
    cx.simulate_keystrokes("cmd-shift-m");
    assert_eq!(card(&app, cx), None);
    refused(&app, cx, "New Comment");
    select(&app, cx, (2, 1));
    run(&app, cx, Cmd::DeleteNote);
    refused(&app, cx, "Delete Note");
    run(&app, cx, Cmd::ShowHideNote);
    refused(&app, cx, "Show/Hide Note");
    // The box of a shown note is neither opened nor moved.
    let (x0, y0, _, _) = box_of(&app, cx, (2, 1)).unwrap();
    click(cx, point(px(x0 + 30.0), px(y0 + 30.0)));
    assert_eq!(editing(&app, cx), None);
    refused(&app, cx, "typing in a note");
    let frame = app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1));
    drag(cx, (x0 + 40.0, y0), (x0 + 90.0, y0 + 30.0));
    refused(&app, cx, "moving a note");
    assert_eq!(
        app.update(cx, |app, _| app.wb.sheets[0].note_frame(2, 1)),
        frame
    );
    // A thread can be read (its card opens), not answered.
    let on_comment = centre(&app, cx, (5, 1));
    click(cx, on_comment);
    assert_eq!(card(&app, cx), Some(((5, 1), true)));
    app.update(cx, |app, cx| {
        assert!(!post(app, 0, (5, 1), "No", Vec::new(), cx))
    });
    refused(&app, cx, "a reply");
    app.update(cx, |app, cx| resolve(app, 0, (5, 1), true, cx));
    refused(&app, cx, "Resolve thread");

    // With "Edit objects" allowed, they all work again.
    app.update(cx, |app, cx| {
        app.wb.unprotect_sheet(0, "").unwrap();
        app.wb
            .protect_sheet(0, SheetProtection::allowing(&[Permission::EditObjects]), "");
        cx.notify();
    });
    app.update(cx, |app, cx| {
        assert!(post(app, 0, (5, 1), "Yes", Vec::new(), cx))
    });
    assert_eq!(thread(&app, cx, (5, 1)).unwrap().replies.len(), 1);
}

#[gpui::test]
fn the_pointer_finds_notes_under_frozen_panes_and_near_the_edges(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 0, 0, "Ada", "Ada:\nIn the frozen corner");
        wb.set_note(0, 30, 3, "Ada", "Ada:\nFar down");
        wb.set_frozen(0, 2, 1);
    });
    app.update(cx, |app, cx| {
        app.set_zoom(1.5, cx);
        app.views[0].scroll_y = 20.0 * 28.0 * 1.5;
        cx.notify();
    });
    redraw(cx);
    let rests_on = |cell: Cell, cx: &mut VisualTestContext| {
        let at = centre(&app, cx, cell);
        cx.simulate_mouse_move(at, None, Modifiers::none());
        wait(cx, REST + Duration::from_millis(10));
        redraw_at(cx, at);
        box_of(&app, cx, cell)
    };
    // The frozen cell is where it always is, whatever the scrolling.
    let frozen = rests_on((0, 0), cx).expect("the note of the frozen cell shows");
    let corner = in_cell(&app, cx, (0, 0), (0.0, 0.0));
    assert!(frozen.0 > f32::from(corner.x) && frozen.1 >= f32::from(corner.y));
    // The scrolled one shows beside its cell, at the zoom of the sheet.
    let scrolled = rests_on((30, 3), cx).expect("the note of the scrolled cell shows");
    assert_eq!(
        (scrolled.2 - scrolled.0, scrolled.3 - scrolled.1),
        (216.0, 119.0)
    );
    assert_eq!(box_of(&app, cx, (0, 0)), None);

    // A cell against the right edge of the grid: its box opens on the
    // other side and stays whole inside the cells.
    let (last_col, grid) = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let col = layout.col_at(layout.width() - 20.0);
        let (a, b) = (
            layout.to_window(layout.header_w, layout.header_h),
            layout.to_window(layout.width(), layout.height()),
        );
        (
            col,
            (
                f32::from(a.x),
                f32::from(a.y),
                f32::from(b.x),
                f32::from(b.y),
            ),
        )
    });
    edit(&app, cx, |wb| {
        wb.set_note(0, 31, last_col, "Ada", "Ada:\nAt the edge");
    });
    let edge = rests_on((31, last_col), cx).expect("the note at the edge shows");
    assert!(
        edge.0 >= grid.0 && edge.1 >= grid.1 && edge.2 <= grid.2 && edge.3 <= grid.3,
        "{edge:?} in {grid:?}"
    );
    let cell = in_cell(&app, cx, (31, last_col), (0.0, 0.0));
    assert!(edge.2 < f32::from(cell.x), "left of its cell");
}

// ---------------------------------------------------------- what is painted

/// What the feature paints on a layer of the grid, and the zones it makes.
fn painted(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    layer: PaintLayer,
) -> (Vec<crate::grid::Draw>, Vec<Zone>) {
    app.update_in(cx, |app, window, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let text_system = window.text_system().clone();
        let everything = (0.0, 0.0, layout.width(), layout.height());
        let mut painter = Painter::new(
            app,
            &layout,
            &text_system,
            layer,
            Rect::new(0, 0, 40, 15),
            everything,
            1.0,
        );
        Comments.paint(&mut painter);
        (
            std::mem::take(&mut painter.items),
            std::mem::take(&mut painter.zones),
        )
    })
}

#[gpui::test]
fn marks_and_boxes_are_painted_where_excel_puts_them(cx: &mut TestAppContext) {
    use crate::grid::Draw;
    use gpui::Background;
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_note(0, 4, 2, "Ada", "Ada:\nShown");
        wb.set_note_visible(0, 4, 2, true);
        wb.set_note(0, 7, 0, "Ada", "Ada:\nHidden");
        wb.add_comment(0, 9, 1, &bob(), 45_000.0, "A thread");
    });
    let solid = |color: u32| Background::from(c(color));

    // Under the boxes: one mark per annotated cell, red for a note and
    // purple for a comment, and the whole cell answers the pointer.
    let (marks, zones) = painted(&app, cx, PaintLayer::Over);
    let colours: Vec<_> = marks
        .iter()
        .map(|draw| match draw {
            Draw::Path(_, colour) => *colour,
            _ => panic!("a mark is a filled shape"),
        })
        .collect();
    assert_eq!(colours, [c(NOTE_MARK), c(NOTE_MARK), c(COMMENT_MARK)]);
    let spots: Vec<Spot> = zones
        .iter()
        .map(|zone| *zone.payload::<Spot>().unwrap())
        .collect();
    assert_eq!(
        spots,
        [Spot::Cell(4, 2), Spot::Cell(7, 0), Spot::Cell(9, 1)]
    );
    let (layout, cell) = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let cell = layout.rect_px(Rect::cell(4, 2));
        (layout, cell)
    });
    assert_eq!(zones[0].bounds, cell);

    // Over everything: the box of the note left showing, and only that one.
    let (items, zones) = painted(&app, cx, PaintLayer::Top);
    let rect = app.update(cx, |app, _| boxes(app, &layout)[0].rect);
    let quads: Vec<_> = items
        .iter()
        .filter_map(|draw| match draw {
            Draw::Quad(quad) => Some(quad),
            _ => None,
        })
        .collect();
    // The pale yellow paper fills the box...
    let paper: Vec<_> = quads
        .iter()
        .filter(|quad| quad.background == solid(notes::PAPER))
        .collect();
    assert_eq!(paper.len(), 1);
    assert_eq!(
        paper[0].bounds,
        layout.bounds_px(rect.0, rect.1, rect.2, rect.3)
    );
    // ...inside a black line, with a shadow two pixels wide at its right
    // and under it.
    let black: Vec<_> = quads
        .iter()
        .filter(|quad| quad.background == solid(0x000000))
        .map(|quad| quad.bounds)
        .collect();
    assert_eq!(black.len(), 6);
    assert_eq!(
        black[0],
        layout.bounds_px(rect.2, rect.1 + 2.0, rect.2 + 2.0, rect.3 + 2.0)
    );
    assert_eq!(
        black[1],
        layout.bounds_px(rect.0 + 2.0, rect.3, rect.2, rect.3 + 2.0)
    );
    assert!(
        black[2..]
            .iter()
            .all(|edge| edge.size.width == px(1.) || edge.size.height == px(1.))
    );
    // The line to the cell and its arrow head are drawn first, in black.
    assert!(matches!(&items[0], Draw::Path(_, colour) if *colour == c(0x000000)));
    assert!(matches!(&items[1], Draw::Path(_, colour) if *colour == c(0x000000)));
    // The text: the author on the first line, then the note, wrapped to
    // the box and a little inside it.
    let text: Vec<_> = items
        .iter()
        .filter_map(|draw| match draw {
            Draw::Wrapped { lines, origin, .. } => Some((
                lines
                    .iter()
                    .map(|line| line.text.to_string())
                    .collect::<Vec<_>>(),
                *origin,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].0, ["Ada:", "Shown"]);
    assert_eq!(
        text[0].1,
        layout.to_window(rect.0 + notes::INSET.0, rect.1 + notes::INSET.1)
    );
    // The box takes the mouse: its inside, and the four sides of its border.
    let spots: Vec<Spot> = zones
        .iter()
        .map(|zone| *zone.payload::<Spot>().unwrap())
        .collect();
    assert_eq!(spots[0], Spot::Body(4, 2));
    assert_eq!(spots[1..], [Spot::Grip(4, 2, Grip::Move); 4]);

    // Selected, it shows a band and eight handles, each a zone.
    app.update_in(cx, |app, window, cx| select_box(app, (4, 2), window, cx));
    let (items, zones) = painted(&app, cx, PaintLayer::Top);
    let white = items
        .iter()
        .filter(|draw| matches!(draw, Draw::Quad(quad) if quad.background == solid(0xFFFFFF)))
        .count();
    assert_eq!(white, 4);
    let handles = zones
        .iter()
        .filter(|zone| {
            matches!(
                zone.payload::<Spot>(),
                Some(Spot::Grip(_, _, Grip::Handle(_)))
            )
        })
        .count();
    assert_eq!(handles, 8);
}

#[gpui::test]
fn a_press_on_the_ribbon_finishes_the_note_too(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (3, 1));
    cx.simulate_keystrokes("shift-f2");
    cx.simulate_input("Typed");
    redraw(cx);
    // Above the grid: the ribbon, which never takes the keyboard.
    let above = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        layout.to_window(300.0, -60.0)
    });
    click(cx, above);
    assert_eq!(editing(&app, cx), None);
    assert_eq!(
        note_text(&app, cx, (3, 1)),
        Some(format!("{}:\nTyped", user_name()))
    );
    // The keyboard is back on the grid.
    cx.simulate_input("9");
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, _| {
        assert_eq!(
            app.wb.sheets[0].value_at(3, 1),
            opensheet_engine::Value::Number(9.0)
        )
    });
}

// ----------------------------------------------------------------- mentions

/// The floating card.
fn card_view(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<CommentCard> {
    ui(app, cx, |ui| {
        ui.card.as_ref().expect("a card is open").view.clone()
    })
}

/// The names in the list of people open under the box, and the selected line.
fn suggested(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<(Vec<String>, usize)> {
    card_view(app, cx).update(cx, |card, _| card.suggested())
}

fn draft(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    card_view(app, cx).update(cx, |card, cx| card.draft_text(cx))
}

/// Who a post names, with the words that name them.
fn named(post: &Post) -> Vec<(String, Person)> {
    post.pieces()
        .into_iter()
        .filter_map(|(text, person)| Some((text.to_string(), person?.clone())))
        .collect()
}

#[gpui::test]
fn an_at_sign_opens_the_list_of_people(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let zed = Person::new("Zed Quux");
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
        wb.add_reply(0, 2, 2, &zed, 45_000.1, "Not yet");
    });
    select(&app, cx, (4, 3));
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("Ask ");
    assert_eq!(suggested(&app, cx), None);
    // The people the workbook knows, then the user.
    cx.simulate_input("@");
    let (people, selected) = suggested(&app, cx).expect("the list is open");
    assert_eq!(&people[..2], ["Bob Martin", "Zed Quux"]);
    assert_eq!(
        (people.last().map(String::as_str), selected),
        (Some(user_name()), 0)
    );
    redraw(cx);
    assert!(
        cx.debug_bounds("mentions@D5").is_some(),
        "the list is drawn under the box"
    );
    // Typing narrows it; the arrows walk it, round its ends.
    cx.simulate_input("q");
    assert_eq!(
        suggested(&app, cx).unwrap().0.first().map(String::as_str),
        Some("Zed Quux")
    );
    // The line that was selected stays selected when the list grows back.
    cx.simulate_keystrokes("backspace");
    assert_eq!(suggested(&app, cx).unwrap(), (people.clone(), 1));
    cx.simulate_keystrokes("up up");
    assert_eq!(suggested(&app, cx).unwrap().1, people.len() - 1);
    cx.simulate_keystrokes("down down");
    assert_eq!(suggested(&app, cx).unwrap().1, 1);
    // Return names the person: nothing is posted, no line is started.
    cx.simulate_keystrokes("enter");
    assert_eq!(suggested(&app, cx), None);
    assert_eq!(draft(&app, cx), "Ask @Zed Quux");
    assert_eq!(thread(&app, cx, (4, 3)), None);
    // More text follows the mention, and Return is a line break again.
    cx.simulate_input(" to look");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("today");
    cx.simulate_keystrokes("cmd-enter");
    let posted = thread(&app, cx, (4, 3)).expect("a thread");
    assert_eq!(posted.first.text, "Ask @Zed Quux to look\ntoday");
    assert_eq!(
        named(&posted.first),
        [("@Zed Quux".to_string(), zed.clone())]
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("New Comment"));
    // One step back and the comment is gone, mention and all.
    cx.simulate_keystrokes("escape");
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(thread(&app, cx, (4, 3)), None);
}

#[gpui::test]
fn a_mention_is_one_piece(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
    });
    select(&app, cx, (2, 2));
    cx.simulate_keystrokes("cmd-shift-m");
    // Tab names the person too.
    cx.simulate_input("@bo");
    assert_eq!(suggested(&app, cx).unwrap().0, ["Bob Martin"]);
    cx.simulate_keystrokes("tab");
    assert_eq!(draft(&app, cx), "@Bob Martin");
    // Backspace right after the mention removes it whole.
    cx.simulate_keystrokes("backspace");
    assert_eq!(draft(&app, cx), "");
    // Text typed against a mention does not join it; Backspace then
    // removes a letter, and the mention only once the caret is back
    // against it.
    cx.simulate_input("Hi @mart");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("!?");
    cx.simulate_keystrokes("backspace");
    assert_eq!(draft(&app, cx), "Hi @Bob Martin!");
    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(draft(&app, cx), "Hi ");
    // A mention the caret cut into is plain text: posted, it names nobody.
    cx.simulate_input("@bob");
    cx.simulate_keystrokes("enter left left backspace");
    assert_eq!(draft(&app, cx), "Hi @Bob Marin");
    cx.simulate_keystrokes("cmd-enter");
    let reply = &thread(&app, cx, (2, 2)).unwrap().replies[0];
    assert_eq!(
        (reply.text.as_str(), reply.mentions.len()),
        ("Hi @Bob Marin", 0)
    );
    // Escape closes the list and nothing else; the next one empties the box.
    redraw(cx);
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("See @");
    assert!(suggested(&app, cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(
        (suggested(&app, cx), draft(&app, cx).as_str()),
        (None, "See @")
    );
    // The list comes back with the next letter of the name.
    cx.simulate_input("b");
    assert_eq!(suggested(&app, cx).unwrap().0, ["Bob Martin"]);
    cx.simulate_keystrokes("escape escape");
    redraw(cx);
    assert_eq!(draft(&app, cx), "");
    assert_eq!(thread(&app, cx, (2, 2)).unwrap().replies.len(), 1);
}

#[gpui::test]
fn an_address_names_someone_new(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    select(&app, cx, (1, 1));
    cx.simulate_keystrokes("cmd-shift-m");
    // No directory: a name nobody has offers nobody, a whole address does.
    cx.simulate_input("@sam@example");
    assert_eq!(suggested(&app, cx), None);
    cx.simulate_input(".com");
    assert_eq!(
        suggested(&app, cx),
        Some((vec!["sam@example.com".to_string()], 0))
    );
    redraw(cx);
    // A click on the line names the person, and the box keeps the keyboard.
    let line = middle(cx, "mention-0@B2");
    click(cx, line);
    assert_eq!(draft(&app, cx), "@sam@example.com");
    cx.simulate_input(" hello");
    cx.simulate_keystrokes("cmd-enter");
    let posted = thread(&app, cx, (1, 1)).expect("a thread");
    let sam = Person::mentioned("sam@example.com", "sam@example.com");
    assert_eq!(posted.first.text, "@sam@example.com hello");
    assert_eq!(
        named(&posted.first),
        [("@sam@example.com".to_string(), sam.clone())]
    );
    assert_eq!(sam.provider_id, "PeoplePicker");
    // Once named, the person is known to the workbook and offered by name.
    redraw(cx);
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("thanks @sa");
    assert_eq!(suggested(&app, cx).unwrap().0, ["sam@example.com"]);
    cx.simulate_keystrokes("enter cmd-enter");
    let reply = &thread(&app, cx, (1, 1)).unwrap().replies[0];
    assert_eq!(named(reply), [("@sam@example.com".to_string(), sam)]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Reply"));
    // An address inside a sentence is not a mention being typed.
    cx.simulate_keystrokes("cmd-shift-m");
    cx.simulate_input("write to joe@example.org");
    assert_eq!(suggested(&app, cx), None);
}

#[gpui::test]
fn mentions_survive_an_edit_and_show_in_the_pane(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let ann = Person::mentioned("Ann Lee", "ann.lee@example.com");
    let text = "@Ann Lee is this right?";
    edit(&app, cx, |wb| {
        let mention = Mention {
            person: ann.clone(),
            start: 0,
            len: 8,
        };
        wb.add_comment_post(
            0,
            2,
            2,
            Post::new(&bob(), 45_000.0, text).with_mentions(vec![mention]),
        );
    });
    // The card shows the name in the colour of mentions.
    assert_eq!(
        card::lines(&thread(&app, cx, (2, 2)).unwrap().first),
        [(text.to_string(), vec![0..8])]
    );
    let on_cell = centre(&app, cx, (2, 2));
    click(cx, on_cell);
    // Rewriting the comment keeps who it names...
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Edit comment");
    cx.simulate_input(" Thanks");
    cx.simulate_keystrokes("cmd-enter");
    let first = thread(&app, cx, (2, 2)).unwrap().first;
    assert_eq!(first.text, "@Ann Lee is this right? Thanks");
    assert_eq!(named(&first), [("@Ann Lee".to_string(), ann.clone())]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Edit Comment"));
    // ... and can name someone else: the list opens under the box of the
    // post, with the address of those who have one.
    redraw(cx);
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Edit comment");
    cx.simulate_input(" @bob");
    assert_eq!(suggested(&app, cx).unwrap().0, ["Bob Martin"]);
    cx.simulate_keystrokes("enter cmd-enter");
    let first = thread(&app, cx, (2, 2)).unwrap().first;
    assert_eq!(
        named(&first),
        [
            ("@Ann Lee".to_string(), ann.clone()),
            ("@Bob Martin".to_string(), bob())
        ]
    );
    // Backspace at the end of the rewritten text takes the last mention.
    redraw(cx);
    let more = middle(cx, "post-menu-0@C3");
    click(cx, more);
    choose(&app, cx, "Edit comment");
    cx.simulate_keystrokes("backspace backspace cmd-enter");
    let first = thread(&app, cx, (2, 2)).unwrap().first;
    assert_eq!(first.text, "@Ann Lee is this right? Thanks");
    assert_eq!(named(&first).len(), 1);

    // In the Comments pane the same box names people.
    cx.simulate_keystrokes("escape");
    run(&app, cx, Cmd::ShowPane);
    select(&app, cx, (2, 2));
    run(&app, cx, Cmd::Reply);
    cx.simulate_input("@ann");
    redraw(cx);
    assert!(
        cx.debug_bounds("mentions@C3").is_some(),
        "the pane shows the list"
    );
    cx.simulate_keystrokes("enter");
    cx.simulate_input(" yes");
    cx.simulate_keystrokes("cmd-enter");
    let reply = &thread(&app, cx, (2, 2)).unwrap().replies[0];
    assert_eq!(reply.text, "@Ann Lee yes");
    assert_eq!(named(reply), [("@Ann Lee".to_string(), ann)]);
}

#[gpui::test]
fn a_protected_sheet_takes_no_mention(cx: &mut TestAppContext) {
    use opensheet_engine::protect::SheetProtection;
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.add_comment(0, 2, 2, &bob(), 45_000.0, "Checked?");
    });
    let on_cell = centre(&app, cx, (2, 2));
    click(cx, on_cell);
    let field = middle(cx, "reply-field@C3");
    click(cx, field);
    cx.simulate_input("@bob");
    cx.simulate_keystrokes("enter");
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
        cx.notify();
    });
    cx.simulate_keystrokes("cmd-enter");
    app.update(cx, |app, _| {
        assert!(app.modal_open(), "the reply was not refused")
    });
    cx.simulate_keystrokes("enter");
    assert!(thread(&app, cx, (2, 2)).unwrap().replies.is_empty());
}
