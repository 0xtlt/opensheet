//! The Draw tab driven in GPUI's headless window: pens, the pointer on
//! the sheet, the eraser, the lasso, keys. What these tests cannot tell is
//! how things look.

use super::*;
use gpui::{
    Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use opensheet_engine::Color;
use opensheet_engine::drawing::ShapePreset;
use opensheet_engine::protect::SheetProtection;
use std::time::Duration;

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

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

/// Click a pen of the gallery.
fn pen(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, index: usize) {
    app.update_in(cx, |app, window, cx| {
        tab::pen_clicked(app, index, window, cx)
    });
    redraw(cx);
}

/// The window position of a point of the sheet.
fn on_sheet(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let (gx, gy) = tools::grid_point(&layout, &app.wb.sheets[app.sheet], (x, y));
        layout.to_window(gx, gy)
    })
}

/// Press at the first point of the sheet, pass through the others,
/// release at the last.
fn trace(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, points: &[(f64, f64)]) {
    let at: Vec<Point<Pixels>> = points
        .iter()
        .map(|(x, y)| on_sheet(app, cx, *x, *y))
        .collect();
    cx.simulate_mouse_down(at[0], MouseButton::Left, Modifiers::none());
    for p in &at[1..] {
        cx.simulate_mouse_move(*p, MouseButton::Left, Modifiers::none());
    }
    cx.simulate_mouse_up(at[at.len() - 1], MouseButton::Left, Modifiers::none());
    redraw(cx);
}

/// Click a point of the sheet.
fn click(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) {
    let at = on_sheet(app, cx, x, y);
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

/// Points a few pixels apart along the outline through `corners`.
fn along(corners: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for w in corners.windows(2) {
        let steps = ((w[1].0 - w[0].0).hypot(w[1].1 - w[0].1) / 6.0).ceil() as usize;
        for i in 0..steps {
            let t = i as f64 / steps as f64;
            out.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
        }
    }
    out.push(corners[corners.len() - 1]);
    out
}

fn inks(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<Ink> {
    app.update(cx, |app, _| app.wb.sheets[app.sheet].inks.clone())
}

/// Where a stroke is on its sheet: (x, y, width, height), rounded.
fn frame(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (i64, i64, i64, i64) {
    app.update(cx, |app, _| {
        let frame = app.wb.ink_frame(app.sheet, id).expect("the stroke");
        (
            frame.x.round() as i64,
            frame.y.round() as i64,
            frame.width.round() as i64,
            frame.height.round() as i64,
        )
    })
}

fn tool(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Tool {
    app.update(cx, |app, _| {
        app.ext::<InkUi>().map(|ui| ui.tool).unwrap_or_default()
    })
}

fn selection(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<u32> {
    app.update(cx, |app, _| selected(app).map(|ink| ink.id))
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

/// Add strokes without the pointer: a zigzag and a straight line under it.
fn two_strokes(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> (u32, u32) {
    let ids = app.update(cx, |app, cx| {
        let a = app
            .wb
            .draw_ink(
                0,
                PenKind::Pen,
                Color(0),
                2.0,
                &[(120.0, 80.0), (150.0, 110.0), (180.0, 80.0), (210.0, 110.0)],
            )
            .expect("a stroke");
        let b = app
            .wb
            .draw_ink(
                0,
                PenKind::Pen,
                Color(0xE71224),
                2.0,
                &[(120.0, 160.0), (220.0, 160.0)],
            )
            .expect("a stroke");
        app.commit("setup", cx);
        (a, b)
    });
    redraw(cx);
    ids
}

#[gpui::test]
fn a_pen_draws_strokes_until_it_is_put_down(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    pen(&app, cx, 1);
    assert_eq!(tool(&app, cx), Tool::Draw(1));
    trace(
        &app,
        cx,
        &along(&[(100.0, 100.0), (160.0, 140.0), (220.0, 100.0)]),
    );
    let drawn = inks(&app, cx);
    assert_eq!(drawn.len(), 1);
    // The second pen of the gallery is red, half a millimetre.
    assert_eq!(
        (drawn[0].kind, drawn[0].color),
        (PenKind::Pen, Color(0xE71224))
    );
    assert!((drawn[0].width - 1.89).abs() < 0.01);
    // Where it was drawn; its corner lost less than a pixel to smoothing.
    let (x, y, w, h) = frame(&app, cx, drawn[0].id);
    assert_eq!((x, y, w), (100, 100, 120));
    assert!((39..=40).contains(&h), "{h}");
    // The straight runs were cleaned to a few points; the corner is kept.
    assert!(
        drawn[0].points.len() < 12,
        "{} points",
        drawn[0].points.len()
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Ink"));
    // The pen stays in hand: a second stroke, and a dot from a click.
    trace(&app, cx, &along(&[(100.0, 200.0), (200.0, 200.0)]));
    trace(&app, cx, &[(300.0, 300.0)]);
    let drawn = inks(&app, cx);
    assert_eq!(drawn.len(), 3);
    assert_eq!(drawn[2].points.len(), 1);
    // One undo step per stroke.
    cx.simulate_keystrokes("cmd-z");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(inks(&app, cx).len(), 1);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(inks(&app, cx).len(), 2);
    // Escape puts the pen down: the next drag selects cells.
    cx.simulate_keystrokes("escape");
    assert_eq!(tool(&app, cx), Tool::Select);
    trace(&app, cx, &[(300.0, 400.0), (360.0, 420.0)]);
    assert_eq!(inks(&app, cx).len(), 2);
}

#[gpui::test]
fn each_pen_draws_its_kind(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for (index, kind) in [(2, PenKind::Pencil), (3, PenKind::Highlighter)] {
        pen(&app, cx, index);
        trace(
            &app,
            cx,
            &along(&[
                (100.0, 100.0 + 60.0 * index as f64),
                (260.0, 104.0 + 60.0 * index as f64),
            ]),
        );
        let last = inks(&app, cx).pop().expect("a stroke");
        assert_eq!(last.kind, kind);
    }
    // The highlighter is six millimetres wide and yellow.
    let marker = inks(&app, cx).pop().unwrap();
    assert_eq!(marker.color, Color(0xFFFC00));
    assert!((marker.width - 22.68).abs() < 0.01);
    // Picking another tool swaps: only one is in hand.
    run(&app, cx, Cmd::Eraser);
    assert_eq!(tool(&app, cx), Tool::Eraser);
    app.update(cx, |app, _| {
        assert!(Cmd::Eraser.checked(app));
        assert!(!Cmd::Select.checked(app));
    });
    run(&app, cx, Cmd::Select);
    assert_eq!(tool(&app, cx), Tool::Select);
}

#[gpui::test]
fn the_eraser_removes_the_strokes_it_touches(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // Nothing to erase yet.
    app.update(cx, |app, _| assert!(!Cmd::Eraser.enabled(app)));
    let (a, b) = two_strokes(&app, cx);
    run(&app, cx, Cmd::Eraser);
    // A drag that crosses the zigzag and then the line: both go, whole.
    trace(
        &app,
        cx,
        &along(&[(100.0, 95.0), (135.0, 95.0), (170.0, 140.0), (170.0, 170.0)]),
    );
    assert!(inks(&app, cx).is_empty());
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Eraser"));
    // One step brings both back.
    cx.simulate_keystrokes("cmd-z");
    let back: Vec<u32> = inks(&app, cx).iter().map(|ink| ink.id).collect();
    assert_eq!(back, vec![a, b]);
    // A drag beside them removes nothing and leaves no undo step.
    run(&app, cx, Cmd::Eraser);
    trace(&app, cx, &along(&[(300.0, 300.0), (360.0, 300.0)]));
    assert_eq!(inks(&app, cx).len(), 2);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
}

#[gpui::test]
fn the_lasso_selects_what_it_encloses(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let (a, b) = two_strokes(&app, cx);
    run(&app, cx, Cmd::Lasso);
    // Around the zigzag only.
    trace(
        &app,
        cx,
        &along(&[
            (100.0, 60.0),
            (240.0, 60.0),
            (240.0, 130.0),
            (100.0, 130.0),
            (100.0, 62.0),
        ]),
    );
    assert_eq!(selection(&app, cx), Some(a));
    app.update(cx, |app, _| assert!(group(app).is_empty()));
    // Around both: the top one is selected, the other goes with it.
    trace(
        &app,
        cx,
        &along(&[
            (90.0, 50.0),
            (260.0, 50.0),
            (260.0, 200.0),
            (90.0, 200.0),
            (90.0, 52.0),
        ]),
    );
    assert_eq!(selection(&app, cx), Some(b));
    app.update(cx, |app, _| assert_eq!(group(app), vec![a]));
    // Dragging one of them moves both, as one step.
    trace(&app, cx, &[(150.0, 160.0), (160.0, 170.0), (190.0, 200.0)]);
    assert_eq!(frame(&app, cx, b), (160, 200, 100, 1));
    assert_eq!(frame(&app, cx, a), (160, 120, 90, 30));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Move Object"));
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(frame(&app, cx, a), (120, 80, 90, 30));
    // A lasso around nothing deselects.
    trace(
        &app,
        cx,
        &along(&[
            (400.0, 300.0),
            (460.0, 300.0),
            (460.0, 360.0),
            (400.0, 360.0),
        ]),
    );
    assert_eq!(selection(&app, cx), None);
    // Both again, and Delete removes them together.
    trace(
        &app,
        cx,
        &along(&[
            (90.0, 50.0),
            (260.0, 50.0),
            (260.0, 200.0),
            (90.0, 200.0),
            (90.0, 52.0),
        ]),
    );
    cx.simulate_keystrokes("backspace");
    assert!(inks(&app, cx).is_empty());
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(inks(&app, cx).len(), 2);
}

#[gpui::test]
fn a_stroke_is_an_object_like_any_other(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let (a, b) = two_strokes(&app, cx);
    // A click in the box of the zigzag but off its line reaches the cell.
    click(&app, cx, 150.0, 84.0);
    assert_eq!(selection(&app, cx), None);
    // On the line it selects the stroke.
    click(&app, cx, 135.0, 95.0);
    assert_eq!(selection(&app, cx), Some(a));
    // Arrow keys nudge it, a drag moves it.
    cx.simulate_keystrokes("right down");
    assert_eq!(frame(&app, cx, a), (121, 81, 90, 30));
    trace(&app, cx, &[(140.0, 95.0), (150.0, 100.0), (180.0, 135.0)]);
    assert_eq!(frame(&app, cx, a), (161, 121, 90, 30));
    // Copy and paste: a second stroke with the same points, selected.
    cx.simulate_keystrokes("cmd-c cmd-v");
    let all = inks(&app, cx);
    assert_eq!(all.len(), 3);
    assert_eq!(selection(&app, cx), Some(all[2].id));
    assert_eq!(all[2].points, all[0].points);
    assert_ne!(all[2].name, all[0].name);
    // Send it to the back, under the others.
    app.update(cx, |app, cx| app.reorder_object(ZMove::ToBack, cx));
    assert_eq!(inks(&app, cx)[0].id, all[2].id);
    // Delete it; the other two stay.
    cx.simulate_keystrokes("backspace");
    let left: Vec<u32> = inks(&app, cx).iter().map(|ink| ink.id).collect();
    assert_eq!(left, vec![a, b]);
    // The bottom-right handle stretches a stroke: its points follow.
    click(&app, cx, 176.0, 136.0);
    assert_eq!(selection(&app, cx), Some(a));
    trace(&app, cx, &[(251.0, 151.0), (270.0, 160.0), (341.0, 181.0)]);
    assert_eq!(frame(&app, cx, a), (161, 121, 180, 60));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Size Object"));
    let stretched = app.update(cx, |app, _| {
        app.wb.sheets[0].ink_points(app.wb.ink(0, a).unwrap())
    });
    assert_eq!(stretched[1], (221.0, 181.0));
}

#[gpui::test]
fn ink_to_shape_turns_what_is_drawn_into_shapes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::InkToShape);
    app.update(cx, |app, _| assert!(Cmd::InkToShape.checked(app)));
    pen(&app, cx, 1);
    trace(
        &app,
        cx,
        &along(&[
            (100.0, 100.0),
            (300.0, 100.0),
            (300.0, 220.0),
            (100.0, 220.0),
            (100.0, 104.0),
        ]),
    );
    assert!(inks(&app, cx).is_empty());
    let shapes = app.update(cx, |app, _| app.wb.sheets[0].drawings.clone());
    assert_eq!(shapes.len(), 1);
    let shape = shapes[0].as_shape().expect("a shape");
    assert_eq!(shape.preset, ShapePreset::Rect);
    // The outline is the pen's; nothing fills it.
    assert_eq!(
        (shape.fill, shape.outline.map(|o| o.color)),
        (None, Some(Color(0xE71224)))
    );
    let at = app.update(cx, |app, _| app.wb.drawing_frame(0, shapes[0].id).unwrap());
    assert_eq!(
        (
            at.x.round(),
            at.y.round(),
            at.width.round(),
            at.height.round()
        ),
        (100.0, 100.0, 200.0, 120.0)
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Ink to Shape"));
    // A line drawn right to left is a line that goes that way.
    trace(&app, cx, &along(&[(300.0, 300.0), (120.0, 260.0)]));
    let line = app.update(cx, |app, _| app.wb.sheets[0].drawings[1].clone());
    assert_eq!(line.as_shape().map(|s| s.preset), Some(ShapePreset::Line));
    assert_eq!((line.flip_h, line.flip_v), (true, true));
    // What looks like nothing stays ink.
    trace(
        &app,
        cx,
        &along(&[
            (400.0, 100.0),
            (460.0, 160.0),
            (410.0, 170.0),
            (470.0, 105.0),
            (440.0, 190.0),
            (500.0, 140.0),
        ]),
    );
    assert_eq!(inks(&app, cx).len(), 1);
    // Switched off, a rectangle stays ink too.
    run(&app, cx, Cmd::InkToShape);
    trace(
        &app,
        cx,
        &along(&[
            (100.0, 300.0),
            (200.0, 300.0),
            (200.0, 380.0),
            (100.0, 380.0),
            (100.0, 304.0),
        ]),
    );
    assert_eq!(inks(&app, cx).len(), 2);
}

#[gpui::test]
fn pens_are_added_changed_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    assert_eq!(pens().len(), 4);
    run(&app, cx, Cmd::AddHighlighter);
    assert_eq!(pens().len(), 5);
    // The new pen is in hand.
    assert_eq!(tool(&app, cx), Tool::Draw(4));
    // A click on the pen in hand opens its menu; a second one closes it.
    pen(&app, cx, 4);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    pen(&app, cx, 4);
    assert!(app.update(cx, |app, _| app.popup.is_none()));
    tab::change_pen(4, |pen| {
        pen.color = 0x00F900;
        pen.mm = 10.0;
    });
    trace(&app, cx, &along(&[(100.0, 100.0), (200.0, 100.0)]));
    let drawn = inks(&app, cx).pop().unwrap();
    assert_eq!(
        (drawn.kind, drawn.color),
        (PenKind::Highlighter, Color(0x00F900))
    );
    assert!((drawn.width - 37.8).abs() < 0.01);
    // Deleting a pen before the one in hand keeps the same pen in hand.
    app.update(cx, |app, cx| tab::delete_pen(app, 0, cx));
    assert_eq!((pens().len(), tool(&app, cx)), (4, Tool::Draw(3)));
    // Deleting the pen in hand puts it down.
    app.update(cx, |app, cx| tab::delete_pen(app, 3, cx));
    assert_eq!((pens().len(), tool(&app, cx)), (3, Tool::Select));
    // The gallery keeps its last pen.
    app.update(cx, |app, cx| {
        tab::delete_pen(app, 0, cx);
        tab::delete_pen(app, 0, cx);
        tab::delete_pen(app, 0, cx);
    });
    assert_eq!(pens().len(), 1);
}

#[gpui::test]
fn ink_replay_draws_the_strokes_again(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    two_strokes(&app, cx);
    run(&app, cx, Cmd::Replay);
    assert!(app.update(cx, |app, _| tools::replaying(app)));
    // It ends by itself and leaves the workbook as it was.
    cx.executor().advance_clock(Duration::from_secs(10));
    redraw(cx);
    assert!(!app.update(cx, |app, _| tools::replaying(app)));
    assert_eq!(inks(&app, cx).len(), 2);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("setup"));
    // Escape stops one that is running.
    run(&app, cx, Cmd::Replay);
    cx.simulate_keystrokes("escape");
    assert!(!app.update(cx, |app, _| tools::replaying(app)));
}

#[gpui::test]
fn a_protected_sheet_takes_no_ink(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let (a, _) = two_strokes(&app, cx);
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
        app.commit("setup", cx);
    });
    redraw(cx);
    // Taking a pen is refused, with Excel's message.
    pen(&app, cx, 0);
    assert_eq!(tool(&app, cx), Tool::Select);
    assert!(app.update(cx, |app, _| app.modal_open()));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // So is deleting a stroke.
    click(&app, cx, 135.0, 95.0);
    assert_eq!(selection(&app, cx), Some(a));
    cx.simulate_keystrokes("backspace");
    assert_eq!(inks(&app, cx).len(), 2);
}
