//! Pictures and shapes driven in GPUI's headless window: clicks, drags,
//! handles, keys and typing on the real application. What these tests
//! cannot tell is how things look.

use super::*;
use crate::features::ViewMode;
use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point};
use opensheet_engine::drawing::Placement as Anchoring;
use opensheet_engine::protect::{Permission, SheetProtection};

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

/// Add a shape of the gallery in a frame of the sheet.
fn add(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, tool: ShapeTool, frame: (f64, f64, f64, f64)) -> u32 {
    let id = app.update(cx, |app, cx| {
        let id = app.wb.add_shape(app.sheet, tool, Frame::new(frame.0, frame.1, frame.2, frame.3)).expect("a shape");
        app.commit("setup", cx);
        id
    });
    redraw(cx);
    id
}

fn change(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, f: impl FnOnce(&mut Drawing)) {
    app.update(cx, |app, cx| {
        app.wb.update_drawing(app.sheet, id, f);
        app.commit("setup", cx);
    });
    redraw(cx);
}

/// Where a drawing is on its sheet: (x, y, width, height), rounded.
fn frame(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (i64, i64, i64, i64) {
    app.update(cx, |app, _| {
        let frame = app.wb.drawing_frame(app.sheet, id).expect("the drawing");
        (frame.x.round() as i64, frame.y.round() as i64, frame.width.round() as i64, frame.height.round() as i64)
    })
}

fn drawing(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Drawing {
    app.update(cx, |app, _| app.wb.drawing(app.sheet, id).expect("the drawing").clone())
}

fn ids(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u32> {
    app.update(cx, |app, _| app.wb.sheets[app.sheet].drawings.iter().map(|d| d.id).collect())
}

fn selection(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<u32> {
    app.update(cx, |app, _| selected(app).map(|drawing| drawing.id))
}

/// The window position of a point of the sheet, in the pane that scrolls.
fn on_sheet(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let pane = *paint::panes(&layout).last().expect("a pane");
        let (zx, zy) = paint::zoomed(&layout, &app.wb.sheets[app.sheet], x, y);
        layout.to_window((zx + pane.dx) as f32, (zy + pane.dy) as f32)
    })
}

/// The window position of a handle of the selected drawing.
fn grip(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, wanted: Grip) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let pane = *paint::panes(&layout).last().expect("a pane");
        let drawing = selected(app).expect("a selected drawing");
        let placed = paint::placed(&layout, &app.wb.sheets[app.sheet], drawing, &pane);
        let (_, at) = paint::grips(drawing, &placed).into_iter().find(|(grip, _)| *grip == wanted).expect("the handle");
        layout.to_window(at.0, at.1)
    })
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

/// Press at `from`, move to `to` in two steps, release.
fn drag_with(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>, modifiers: Modifiers) {
    let middle = point((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    cx.simulate_mouse_down(from, MouseButton::Left, modifiers);
    cx.simulate_mouse_move(middle, MouseButton::Left, modifiers);
    cx.simulate_mouse_move(to, MouseButton::Left, modifiers);
    cx.simulate_mouse_up(to, MouseButton::Left, modifiers);
    redraw(cx);
}

fn drag(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    drag_with(cx, from, to, Modifiers::none());
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn take(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, tool: ShapeTool) {
    app.update_in(cx, |app, window, cx| tool::arm(app, tool, window, cx));
    redraw(cx);
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

/// A small PNG file.
fn png(width: u32, height: u32) -> Vec<u8> {
    let pixels = image::RgbaImage::from_raw(width, height, vec![128; (width * height * 4) as usize]).expect("pixels");
    let mut file = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut file, image::ImageFormat::Png).expect("a PNG");
    file.into_inner()
}

// ------------------------------------------------------------ what is shown

#[gpui::test]
fn a_click_selects_what_is_drawn_there(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let rectangle = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let line = add(&app, cx, ShapeTool::Line, (300.0, 60.0, 100.0, 100.0));
    // Nothing is selected, and no tab shows.
    assert_eq!(selection(&app, cx), None);
    let at = on_sheet(&app, cx, 150.0, 100.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), Some(rectangle));
    app.update(cx, |app, _| assert_eq!(Drawings.context_tabs(app)[0].id, SHAPE_TAB));
    // A line is taken along its length; the rest of its frame is cells.
    let at = on_sheet(&app, cx, 380.0, 80.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), None);
    let at = on_sheet(&app, cx, 350.0, 111.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), Some(line));
    // A turned shape is taken where it was turned to (and its text is
    // drawn turned with it).
    let tall = add(&app, cx, ShapeTool::Rectangle, (100.0, 240.0, 200.0, 20.0));
    change(&app, cx, tall, |d| {
        d.rotation = 90.0;
        d.as_shape_mut().unwrap().text = "Turned with its shape, on two lines".into();
    });
    let at = on_sheet(&app, cx, 110.0, 250.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), None);
    let at = on_sheet(&app, cx, 200.0, 170.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), Some(tall));
    // Escape lets go of it.
    cx.simulate_keystrokes("escape");
    assert_eq!(selection(&app, cx), None);
}

#[gpui::test]
fn the_topmost_drawing_is_taken_and_the_order_can_change(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let below = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let above = add(&app, cx, ShapeTool::Ellipse, (140.0, 80.0, 120.0, 80.0));
    let overlap = on_sheet(&app, cx, 200.0, 120.0);
    click(cx, overlap);
    assert_eq!(selection(&app, cx), Some(above));
    run(&app, cx, Cmd::SendToBack);
    assert_eq!(ids(&app, cx), vec![above, below]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Change Order"));
    cx.simulate_keystrokes("escape");
    click(cx, overlap);
    assert_eq!(selection(&app, cx), Some(below));
    run(&app, cx, Cmd::SendBackward);
    assert_eq!(ids(&app, cx), vec![below, above]);
    run(&app, cx, Cmd::BringToFront);
    assert_eq!(ids(&app, cx), vec![above, below]);
}

// ----------------------------------------------------- moving and resizing

#[gpui::test]
fn dragging_moves_and_the_handles_resize(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let (from, to) = (on_sheet(&app, cx, 150.0, 100.0), on_sheet(&app, cx, 190.0, 130.0));
    drag(cx, from, to);
    assert_eq!(selection(&app, cx), Some(id));
    assert_eq!(frame(&app, cx, id), (140, 90, 120, 80));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Move Object"));
    // The bottom-right handle: the top-left corner stays.
    let (from, to) = (grip(&app, cx, Grip::Resize(4)), on_sheet(&app, cx, 300.0, 200.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, id), (140, 90, 160, 110));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Size Object"));
    // The left handle moves the left side only.
    let (from, to) = (grip(&app, cx, Grip::Resize(7)), on_sheet(&app, cx, 100.0, 300.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, id), (100, 90, 200, 110));
    // Shift keeps the proportions from a corner.
    let (from, to) = (grip(&app, cx, Grip::Resize(4)), on_sheet(&app, cx, 400.0, 205.0));
    drag_with(cx, from, to, Modifiers::shift());
    assert_eq!(frame(&app, cx, id), (100, 90, 300, 165));
    // One undo per gesture.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(frame(&app, cx, id), (100, 90, 200, 110));
}

#[gpui::test]
fn the_round_handle_turns_and_a_turned_shape_resizes_along_itself(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (200.0, 100.0, 100.0, 60.0));
    let centre = on_sheet(&app, cx, 250.0, 130.0);
    click(cx, centre);
    // Drag the handle from above the shape to its right: a quarter turn.
    let (from, to) = (grip(&app, cx, Grip::Rotate), on_sheet(&app, cx, 330.0, 130.0));
    drag(cx, from, to);
    assert_eq!(drawing(&app, cx, id).rotation, 90.0);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Rotate Object"));
    // The frame itself did not change.
    assert_eq!(frame(&app, cx, id), (200, 100, 100, 60));
    // Its "right" handle now points down: pulling it down widens it,
    // and the opposite side (on top) stays where it is.
    let from = grip(&app, cx, Grip::Resize(3));
    let to = point(from.x, from.y + px(40.));
    drag(cx, from, to);
    let resized = frame(&app, cx, id);
    assert_eq!((resized.2, resized.3), (140, 60));
    assert_eq!(resized.1, 120);
    // Shift snaps the turn to fifteen degrees.
    let (from, to) = (grip(&app, cx, Grip::Rotate), on_sheet(&app, cx, 200.0, 148.0));
    drag_with(cx, from, to, Modifiers::shift());
    assert_eq!(drawing(&app, cx, id).rotation % 15.0, 0.0);
}

#[gpui::test]
fn a_line_is_resized_by_its_two_ends(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Arrow, (100.0, 100.0, 100.0, 50.0));
    let middle = on_sheet(&app, cx, 150.0, 125.0);
    click(cx, middle);
    assert_eq!(selection(&app, cx), Some(id));
    // The end goes up and left of the start: the frame flips both ways.
    let (from, to) = (grip(&app, cx, Grip::LineEnd(true)), on_sheet(&app, cx, 60.0, 40.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, id), (60, 40, 40, 60));
    let line = drawing(&app, cx, id);
    assert_eq!((line.flip_h, line.flip_v), (true, true));
    // The start follows its own handle; the end stays.
    let (from, to) = (grip(&app, cx, Grip::LineEnd(false)), on_sheet(&app, cx, 20.0, 40.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, id), (20, 40, 40, 0));
    let line = drawing(&app, cx, id);
    assert_eq!((line.flip_h, line.flip_v), (false, false));
}

#[gpui::test]
fn keys_nudge_delete_copy_and_paste(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Diamond, (100.0, 60.0, 80.0, 80.0));
    change(&app, cx, id, |d| d.rotation = 30.0);
    let at = on_sheet(&app, cx, 140.0, 100.0);
    click(cx, at);
    cx.simulate_keystrokes("right right down");
    assert_eq!(frame(&app, cx, id), (102, 61, 80, 80));
    // The copy lands a little lower and to the right, turned the same.
    cx.simulate_keystrokes("cmd-c cmd-v");
    let all = ids(&app, cx);
    assert_eq!(all.len(), 2);
    let copy = all[1];
    assert_eq!(selection(&app, cx), Some(copy));
    assert_eq!(frame(&app, cx, copy), (118, 77, 80, 80));
    let pasted = drawing(&app, cx, copy);
    assert_eq!(pasted.rotation, 30.0);
    assert_ne!(pasted.name, drawing(&app, cx, id).name);
    cx.simulate_keystrokes("backspace");
    assert_eq!(ids(&app, cx), vec![id]);
    assert_eq!(selection(&app, cx), None);
    // Tab walks the drawings once one is selected.
    let other = add(&app, cx, ShapeTool::Ellipse, (300.0, 60.0, 80.0, 80.0));
    click(cx, at);
    cx.simulate_keystrokes("tab");
    assert_eq!(selection(&app, cx), Some(other));
}

// --------------------------------------------------------------- inserting

#[gpui::test]
fn a_shape_is_drawn_by_dragging(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    take(&app, cx, ShapeTool::Ellipse);
    let (from, to) = (on_sheet(&app, cx, 100.0, 80.0), on_sheet(&app, cx, 260.0, 180.0));
    drag(cx, from, to);
    let all = ids(&app, cx);
    assert_eq!(all.len(), 1);
    assert_eq!(frame(&app, cx, all[0]), (100, 80, 160, 100));
    // It is selected, its tab shows, and the tool is put down.
    assert_eq!(selection(&app, cx), Some(all[0]));
    app.update(cx, |app, _| {
        assert_eq!(app.context_tab, Some(SHAPE_TAB));
        assert!(app.ext_mut::<DrawingUi>().tool.is_none());
    });
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Shape"));
    // Shift draws a square; dragging up and left works too.
    take(&app, cx, ShapeTool::Rectangle);
    let (from, to) = (on_sheet(&app, cx, 500.0, 300.0), on_sheet(&app, cx, 440.0, 270.0));
    drag_with(cx, from, to, Modifiers::shift());
    let square = *ids(&app, cx).last().unwrap();
    assert_eq!(frame(&app, cx, square), (440, 240, 60, 60));
    // A click alone inserts the shape at its usual size.
    take(&app, cx, ShapeTool::RightArrow);
    let at = on_sheet(&app, cx, 300.0, 400.0);
    click(cx, at);
    let arrow = *ids(&app, cx).last().unwrap();
    assert_eq!(frame(&app, cx, arrow), (300, 400, 96, 48));
    // A line remembers the way it was drawn.
    take(&app, cx, ShapeTool::Arrow);
    let (from, to) = (on_sheet(&app, cx, 700.0, 200.0), on_sheet(&app, cx, 620.0, 260.0));
    drag(cx, from, to);
    let last = *ids(&app, cx).last().unwrap();
    let line = drawing(&app, cx, last);
    assert_eq!((line.flip_h, line.flip_v), (true, false));
    assert_eq!(ids(&app, cx).len(), 4);
}

#[gpui::test]
fn escape_puts_the_tool_down(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    take(&app, cx, ShapeTool::Star5);
    cx.simulate_keystrokes("escape");
    let at = on_sheet(&app, cx, 200.0, 200.0);
    click(cx, at);
    assert!(ids(&app, cx).is_empty());
    // The click went to the cell under it.
    app.update(cx, |app, _| assert_ne!(app.active(), (0, 0)));
}

#[gpui::test]
fn pictures_come_in_at_their_size_on_the_active_cell(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| app.select_cell(3, 2, cx));
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![png(120, 40), png(30, 30)], None, cx));
    redraw(cx);
    let all = ids(&app, cx);
    assert_eq!(all.len(), 2);
    assert_eq!(frame(&app, cx, all[0]), (128, 60, 120, 40));
    assert_eq!(frame(&app, cx, all[1]), (144, 76, 30, 30));
    // The last one is selected and the Picture Format tab is shown.
    assert_eq!(selection(&app, cx), Some(all[1]));
    app.update(cx, |app, _| assert_eq!(app.context_tab, Some(PICTURE_TAB)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Picture"));
    // A corner keeps the proportions of a picture.
    let (from, to) = (grip(&app, cx, Grip::Resize(4)), on_sheet(&app, cx, 204.0, 110.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, all[1]), (144, 76, 60, 60));
    // Turned, flipped, resized... and back as it came in.
    run(&app, cx, Cmd::RotateRight);
    run(&app, cx, Cmd::FlipHorizontal);
    let turned = drawing(&app, cx, all[1]);
    assert_eq!((turned.rotation, turned.flip_h), (270.0, true));
    run(&app, cx, Cmd::ResetPictureAndSize);
    let reset = drawing(&app, cx, all[1]);
    assert_eq!((reset.rotation, reset.flip_h), (0.0, false));
    assert_eq!(frame(&app, cx, all[1]), (144, 76, 30, 30));
    // What is no picture is left out.
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![b"not a picture".to_vec()], None, cx));
    assert_eq!(ids(&app, cx).len(), 2);
}

// -------------------------------------------------------------------- text

#[gpui::test]
fn typing_with_a_shape_selected_writes_in_it(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 160.0, 80.0));
    let at = on_sheet(&app, cx, 150.0, 100.0);
    click(cx, at);
    cx.simulate_input("Total");
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_keystrokes("enter");
    cx.simulate_input("2026");
    // Nothing is in the workbook while it is being typed.
    assert_eq!(drawing(&app, cx, id).as_shape().unwrap().text, "");
    // Escape keeps the text and the shape stays selected.
    cx.simulate_keystrokes("escape");
    assert_eq!(app.update(cx, |app, _| editing_text(app)), None);
    assert_eq!(drawing(&app, cx, id).as_shape().unwrap().text, "Total\n2026");
    assert_eq!(selection(&app, cx), Some(id));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Typing"));
    // No cell took the keys.
    app.update(cx, |app, _| assert!(!app.is_editing() && app.wb.sheets[0].cells.is_empty()));
    // Typing again goes on after what is there; a click on a cell ends it.
    cx.simulate_input("!");
    let cell = on_sheet(&app, cx, 500.0, 300.0);
    click(cx, cell);
    assert_eq!(drawing(&app, cx, id).as_shape().unwrap().text, "Total\n2026!");
    assert_eq!(selection(&app, cx), None);
    // A double click edits too; a line holds no text.
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    cx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_keystrokes("escape");
    let line = add(&app, cx, ShapeTool::Line, (300.0, 200.0, 100.0, 0.0));
    let on_line = on_sheet(&app, cx, 350.0, 200.0);
    click(cx, on_line);
    assert_eq!(selection(&app, cx), Some(line));
    cx.simulate_input("x");
    assert_eq!(app.update(cx, |app, _| editing_text(app)), None);
}

#[gpui::test]
fn a_text_box_is_typed_in_as_soon_as_it_is_drawn(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::TextBox);
    let (from, to) = (on_sheet(&app, cx, 100.0, 80.0), on_sheet(&app, cx, 260.0, 140.0));
    drag(cx, from, to);
    let id = ids(&app, cx)[0];
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_input("Note");
    cx.simulate_keystrokes("escape");
    let shape = drawing(&app, cx, id);
    assert!(shape.as_shape().unwrap().text_box);
    assert_eq!(shape.as_shape().unwrap().text, "Note");
}

#[gpui::test]
fn home_tab_formatting_goes_to_the_selected_shape(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 160.0, 80.0));
    let at = on_sheet(&app, cx, 150.0, 100.0);
    click(cx, at);
    app.update(cx, |app, cx| app.toggle_bold(cx));
    assert!(drawing(&app, cx, id).as_shape().unwrap().font.bold);
    // The cell behind keeps its style.
    app.update(cx, |app, _| assert!(!app.active_style().font.bold));
}

// -------------------------------------------------- panes, zoom, protection

#[gpui::test]
fn drawings_follow_zoom_scrolling_and_frozen_panes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (200.0, 100.0, 120.0, 80.0));
    app.update(cx, |app, cx| app.set_zoom(1.5, cx));
    redraw(cx);
    let at = on_sheet(&app, cx, 260.0, 140.0);
    click(cx, at);
    assert_eq!(selection(&app, cx), Some(id));
    // Dragged 60 pixels on screen, it moves 40 on the sheet.
    let to = point(at.x + px(60.), at.y);
    drag(cx, at, to);
    assert_eq!(frame(&app, cx, id), (240, 100, 120, 80));
    // With the first column and row frozen and the sheet scrolled, the
    // shape is found where it scrolled to, not where it was.
    app.update(cx, |app, cx| {
        app.set_zoom(1.0, cx);
        app.wb.sheets[0].frozen_rows = 1;
        app.wb.sheets[0].frozen_cols = 1;
        app.views[0].scroll_x = 64.0;
        app.views[0].scroll_y = 40.0;
        cx.notify();
    });
    redraw(cx);
    cx.simulate_keystrokes("escape");
    let moved = on_sheet(&app, cx, 300.0, 140.0);
    let layout = app.update(cx, |app, _| app.current_layout().unwrap());
    assert_eq!(layout.local(moved), (layout.header_w + 300.0 - 64.0, layout.header_h + 140.0 - 40.0));
    click(cx, moved);
    assert_eq!(selection(&app, cx), Some(id));
}

#[gpui::test]
fn drawings_show_in_the_page_views(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (200.0, 100.0, 120.0, 80.0));
    for mode in [ViewMode::PageLayout, ViewMode::PageBreakPreview, ViewMode::Normal] {
        app.update(cx, |app, cx| app.set_view_mode(mode, cx));
        redraw(cx);
        cx.simulate_keystrokes("escape");
        // Wherever the view puts the cells, the shape is over its own.
        let at = on_sheet(&app, cx, 260.0, 140.0);
        click(cx, at);
        assert_eq!(selection(&app, cx), Some(id), "{mode:?}");
        let extent = app.update(cx, |app, _| Drawings.object_extent(app, id, &app.current_layout().unwrap()));
        assert!(extent.is_some_and(|extent| extent.2 > extent.0), "{mode:?}");
    }
}

#[gpui::test]
fn a_shape_sized_with_its_cells_follows_them(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add(&app, cx, ShapeTool::Rectangle, (70.0, 30.0, 100.0, 50.0));
    assert_eq!(drawing(&app, cx, id).placement, Anchoring::MoveAndSize);
    app.update(cx, |app, cx| {
        app.wb.set_col_width(0, 1, Some(164.0));
        app.commit("width", cx);
    });
    redraw(cx);
    assert_eq!(frame(&app, cx, id), (70, 30, 200, 50));
    // Its far handle is found where the column put it.
    let at = on_sheet(&app, cx, 120.0, 50.0);
    click(cx, at);
    let handle = grip(&app, cx, Grip::Resize(4));
    assert_eq!(handle, on_sheet(&app, cx, 270.0, 80.0));
}

#[gpui::test]
fn a_protected_sheet_keeps_its_locked_drawings(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let locked = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let free = add(&app, cx, ShapeTool::Ellipse, (300.0, 60.0, 120.0, 80.0));
    change(&app, cx, free, |d| d.locked = false);
    app.update(cx, |app, _| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
    });
    redraw(cx);
    // The locked one does not move, is not deleted, takes no text.
    let (from, to) = (on_sheet(&app, cx, 150.0, 100.0), on_sheet(&app, cx, 190.0, 130.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, locked), (100, 60, 120, 80));
    cx.simulate_keystrokes("backspace");
    app.update_in(cx, |app, window, cx| {
        assert!(app.modal_open());
        app.close_modal(window, cx);
    });
    assert_eq!(ids(&app, cx), vec![locked, free]);
    run(&app, cx, Cmd::RotateRight);
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    assert_eq!(drawing(&app, cx, locked).rotation, 0.0);
    // Nothing new can be drawn either.
    take(&app, cx, ShapeTool::Star5);
    app.update_in(cx, |app, window, cx| {
        assert!(app.ext_mut::<DrawingUi>().tool.is_none());
        app.close_modal(window, cx);
    });
    // The one that was unlocked beforehand still moves.
    let (from, to) = (on_sheet(&app, cx, 350.0, 100.0), on_sheet(&app, cx, 390.0, 130.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, free), (340, 90, 120, 80));
    // With "Edit objects" allowed, the locked one moves again.
    app.update(cx, |app, _| {
        app.wb.protect_sheet(0, SheetProtection::allowing(&[Permission::EditObjects]), "");
    });
    redraw(cx);
    cx.simulate_keystrokes("escape");
    let (from, to) = (on_sheet(&app, cx, 150.0, 100.0), on_sheet(&app, cx, 190.0, 130.0));
    drag(cx, from, to);
    assert_eq!(frame(&app, cx, locked), (140, 90, 120, 80));
}

#[gpui::test]
fn the_tabs_and_the_menu_follow_what_is_selected(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let shape = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let line = add(&app, cx, ShapeTool::Line, (300.0, 60.0, 100.0, 0.0));
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![png(40, 40)], Some((100.0, 300.0)), cx));
    redraw(cx);
    let entries = |app: &ExcelApp, id: u32| -> Vec<String> {
        let mut menu = ContextMenu::default();
        Drawings.context_menu(app, &MenuTarget::Object(object_id(&Drawings, id)), &mut menu);
        menu.build().into_iter().filter(|entry| !entry.separator).map(|entry| entry.label.to_string()).collect()
    };
    let picture = selection(&app, cx).unwrap();
    app.update(cx, |app, _| {
        assert_eq!(Drawings.context_tabs(app)[0].id, PICTURE_TAB);
        assert!(Cmd::ResetPicture.enabled(app) && !Cmd::ShapeFill.enabled(app) && !Cmd::EditText.enabled(app));
        assert_eq!(entries(app, picture), vec!["View Alt Text…", "Size and Properties…", "Format Picture…"]);
        assert_eq!(
            entries(app, shape),
            vec!["Edit Text", "View Alt Text…", "Size and Properties…", "Format Shape…"]
        );
        assert_eq!(entries(app, line), vec!["View Alt Text…", "Size and Properties…", "Format Shape…"]);
    });
    let at = on_sheet(&app, cx, 150.0, 100.0);
    click(cx, at);
    app.update(cx, |app, _| {
        assert_eq!(Drawings.context_tabs(app)[0].id, SHAPE_TAB);
        assert!(Cmd::ShapeFill.enabled(app) && Cmd::EditText.enabled(app) && !Cmd::ResetPicture.enabled(app));
        // One object: nothing to group, align or let go.
        assert!(Cmd::Group.enabled(app) && !Cmd::GroupObjects.enabled(app) && !Cmd::Ungroup.enabled(app));
        assert!(!Cmd::AlignLeft.enabled(app) && !Cmd::DistributeHorizontally.enabled(app));
    });
    // The commands of the tab change the selected shape.
    app.update(cx, |app, cx| {
        tabs::set_fill(app, None, cx);
        tabs::set_outline(app, cx, |outline| outline.width = 3.0);
        tabs::set_side(app, true, tabs::in_units(160.0), cx);
    });
    let changed = drawing(&app, cx, shape);
    assert_eq!(changed.as_shape().unwrap().fill, None);
    assert_eq!(changed.as_shape().unwrap().outline.unwrap().width, 3.0);
    assert_eq!(frame(&app, cx, shape).3, 160);
}

#[gpui::test]
fn the_panes_and_the_drop_downs_open_on_the_selected_drawing(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let shape = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 80.0));
    let at = on_sheet(&app, cx, 150.0, 100.0);
    click(cx, at);
    let open_panes = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| [pane::FORMAT, pane::ALT_TEXT, pane::SELECTION].map(|id| app.pane_open(id)))
    };
    // The Format pane, on either page, then the Selection pane.
    run(&app, cx, Cmd::FormatShape);
    run(&app, cx, Cmd::Size);
    assert_eq!(open_panes(&app, cx), [true, false, false]);
    run(&app, cx, Cmd::SelectionPane);
    assert_eq!(open_panes(&app, cx), [true, false, true]);
    run(&app, cx, Cmd::SelectionPane);
    assert_eq!(open_panes(&app, cx), [true, false, false]);
    // Every drop-down of the tab opens and is drawn.
    for cmd in [Cmd::ShapeFill, Cmd::ShapeOutline, Cmd::TextFill, Cmd::Shapes] {
        run(&app, cx, cmd);
        app.update(cx, |app, cx| {
            assert!(matches!(app.popup.as_ref().map(|popup| &popup.kind), Some(crate::app::PopupKind::View(_))), "{cmd:?}");
            app.popup = None;
            cx.notify();
        });
    }
    for cmd in [Cmd::Rotate, Cmd::Align, Cmd::Group, Cmd::ShapeEffects, Cmd::EditShape, Cmd::Pictures] {
        run(&app, cx, cmd);
        app.update(cx, |app, cx| {
            assert!(matches!(app.popup.as_ref().map(|popup| &popup.kind), Some(crate::app::PopupKind::Menu(_))), "{cmd:?}");
            app.popup = None;
            cx.notify();
        });
    }
    // What is not available says so.
    run(&app, cx, Cmd::ShapeStyles);
    app.update(cx, |app, _| assert!(app.status_message.as_ref().is_some_and(|message| message.contains(Cmd::ShapeStyles.label()))));
    // A description typed in the Alt Text pane is kept when the selection
    // goes elsewhere, as one undo step.
    run(&app, cx, Cmd::AltText);
    assert_eq!(open_panes(&app, cx), [true, true, false]);
    cx.simulate_input("A blue box");
    assert_eq!(drawing(&app, cx, shape).alt_text, "");
    let cell = on_sheet(&app, cx, 500.0, 300.0);
    click(cx, cell);
    assert_eq!(drawing(&app, cx, shape).alt_text, "A blue box");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Alt Text"));
    // The panes stay open with nothing selected, and follow a picture.
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![png(40, 40)], Some((300.0, 300.0)), cx));
    redraw(cx);
    run(&app, cx, Cmd::FormatPicture);
    app.update(cx, |app, _| {
        let title = app.panes.iter().find(|pane| pane.id == pane::FORMAT).map(|pane| pane.title.to_string());
        assert_eq!(title.as_deref(), Some("Format Picture"));
    });
}

// ------------------------------------------------- several objects at once

/// The selected drawings, the one selected last first.
fn selected_all(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u32> {
    app.update(cx, |app, _| selected_ids(app))
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::none()
    }
}

fn click_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) {
    let at = on_sheet(app, cx, x, y);
    click(cx, at);
}

fn shift_click_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) {
    let at = on_sheet(app, cx, x, y);
    cx.simulate_click(at, shift());
    redraw(cx);
}

fn drag_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, from: (f64, f64), to: (f64, f64), modifiers: Modifiers) {
    let (from, to) = (on_sheet(app, cx, from.0, from.1), on_sheet(app, cx, to.0, to.1));
    drag_with(cx, from, to, modifiers);
}

/// A chart of nothing, at a place of the sheet.
fn add_chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, at: (f32, f32), size: (f32, f32)) -> u32 {
    use opensheet_engine::chart::{Chart, ChartKind};
    let id = app.update(cx, |app, cx| {
        let chart = Chart {
            offset: at,
            size,
            ..Chart::new(ChartKind::Column)
        };
        let id = app.wb.add_chart(app.sheet, chart).expect("a chart");
        app.commit("setup", cx);
        id
    });
    redraw(cx);
    id
}

#[gpui::test]
fn charts_and_shapes_share_one_stacking_order(cx: &mut TestAppContext) {
    use crate::objects::object_of;
    use opensheet_engine::drawing::ObjectRef;
    let (app, cx) = open(cx);
    let under = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 200.0, 150.0));
    let chart = add_chart(&app, cx, (150.0, 100.0), (300.0, 200.0));
    let over = add(&app, cx, ShapeTool::Ellipse, (200.0, 150.0, 100.0, 80.0));
    let stack = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.wb.object_stack(app.sheet));
    let (r, c, e) = (ObjectRef::Drawing(under), ObjectRef::Chart(chart), ObjectRef::Drawing(over));
    assert_eq!(stack(&app, cx), vec![r, c, e]);
    let lead = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.object_sel);
    // Where the three overlap, the ellipse is taken; where only the
    // rectangle and the chart do, the chart: it lies over the rectangle.
    let all = on_sheet(&app, cx, 250.0, 190.0);
    click(cx, all);
    assert_eq!(lead(&app, cx), Some(object_of(e)));
    let two = on_sheet(&app, cx, 170.0, 120.0);
    click(cx, two);
    assert_eq!(lead(&app, cx), Some(object_of(c)));
    // The chart sent back one step goes under the rectangle.
    run(&app, cx, Cmd::SendBackward);
    assert_eq!(stack(&app, cx), vec![c, r, e]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Change Order"));
    app.update(cx, |app, cx| app.deselect_object(cx));
    click(cx, two);
    assert_eq!(lead(&app, cx), Some(object_of(r)));
    // And brought to the front, over the ellipse.
    app.update(cx, |app, cx| {
        app.select_object(Some(object_of(c)), cx);
        app.reorder_object(ZMove::ToFront, cx);
    });
    redraw(cx);
    assert_eq!(stack(&app, cx), vec![r, e, c]);
    click(cx, all);
    assert_eq!(lead(&app, cx), Some(object_of(c)));
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(stack(&app, cx), vec![c, r, e]);
    // A hidden object is neither drawn nor taken.
    app.update(cx, |app, cx| pane::set_hidden(app, &[e], true, cx));
    redraw(cx);
    click(cx, all);
    assert_eq!(lead(&app, cx), Some(object_of(r)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Hide Object"));
}

#[gpui::test]
fn several_objects_are_selected_moved_and_deleted_together(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let a = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let b = add(&app, cx, ShapeTool::Ellipse, (300.0, 60.0, 80.0, 80.0));
    let c = add(&app, cx, ShapeTool::Rectangle, (100.0, 240.0, 60.0, 40.0));
    let (at_a, at_b, at_c) = (on_sheet(&app, cx, 150.0, 90.0), on_sheet(&app, cx, 340.0, 100.0), on_sheet(&app, cx, 130.0, 260.0));
    click(cx, at_a);
    cx.simulate_click(at_b, shift());
    redraw(cx);
    assert_eq!(selected_all(&app, cx), vec![b, a]);
    // ⌘ adds too, and takes out again.
    let command = Modifiers {
        platform: true,
        ..Modifiers::none()
    };
    cx.simulate_click(at_c, command);
    assert_eq!(selected_all(&app, cx), vec![c, b, a]);
    cx.simulate_click(at_c, command);
    redraw(cx);
    assert_eq!(selected_all(&app, cx), vec![b, a]);
    // Dragging one moves both, as one step.
    drag(cx, at_a, point(at_a.x + px(40.), at_a.y + px(30.)));
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((140, 90, 100, 60), (340, 90, 80, 80)));
    assert_eq!(frame(&app, cx, c), (100, 240, 60, 40));
    assert_eq!(selected_all(&app, cx), vec![a, b]);
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((100, 60, 100, 60), (300, 60, 80, 80)));
    // The arrow keys too.
    cx.simulate_keystrokes("right down");
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((101, 61, 100, 60), (301, 61, 80, 80)));
    // A handle of one resizes both by the same proportion, each from its
    // own corner: half as wide again, twice as high.
    let corner = grip(&app, cx, Grip::Resize(4));
    drag(cx, corner, point(corner.x + px(50.), corner.y + px(60.)));
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((101, 61, 150, 120), (301, 61, 120, 160)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Size Object"));
    app.update(cx, |app, cx| app.undo(cx));
    // The round handle turns both.
    let round = grip(&app, cx, Grip::Rotate);
    let centre = on_sheet(&app, cx, 151.0, 91.0);
    drag(cx, round, point(centre.x + px(100.), centre.y));
    assert_eq!((drawing(&app, cx, a).rotation, drawing(&app, cx, b).rotation), (90.0, 90.0));
    app.update(cx, |app, cx| app.undo(cx));
    // A fill goes to every selected shape.
    app.update(cx, |app, cx| tabs::set_fill(app, Some(opensheet_engine::Color(0xFF0000)), cx));
    for id in [a, b] {
        assert_eq!(drawing(&app, cx, id).as_shape().unwrap().fill, Some(opensheet_engine::Color(0xFF0000)));
    }
    assert_ne!(drawing(&app, cx, c).as_shape().unwrap().fill, Some(opensheet_engine::Color(0xFF0000)));
    // ⌘A with an object selected: every object of the sheet.
    cx.simulate_keystrokes("cmd-a");
    assert_eq!(selected_all(&app, cx).len(), 3);
    // Copy and paste: three copies, placed as the originals are among
    // themselves, all selected.
    cx.simulate_keystrokes("cmd-c cmd-v");
    redraw(cx);
    let copies = selected_all(&app, cx);
    assert_eq!(copies.len(), 3);
    assert!(copies.iter().all(|id| ![a, b, c].contains(id)));
    let (fa, fb) = (frame(&app, cx, a), frame(&app, cx, b));
    let moved: Vec<(i64, i64)> = copies.iter().map(|id| frame(&app, cx, *id)).map(|f| (f.0, f.1)).collect();
    assert!(moved.contains(&(fa.0 + 16, fa.1 + 16)) && moved.contains(&(fb.0 + 16, fb.1 + 16)), "{moved:?}");
    assert_eq!(ids(&app, cx).len(), 6);
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(ids(&app, cx).len(), 3);
    // Delete removes them all, in one step.
    click_at(&app, cx, 150.0, 90.0);
    cx.simulate_keystrokes("cmd-a delete");
    assert!(ids(&app, cx).is_empty());
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(ids(&app, cx).len(), 3);
}

#[gpui::test]
fn select_objects_drags_a_rectangle_and_go_to_special_takes_all(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let a = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let b = add(&app, cx, ShapeTool::Ellipse, (300.0, 60.0, 80.0, 80.0));
    let c = add(&app, cx, ShapeTool::Rectangle, (100.0, 240.0, 60.0, 40.0));
    // Without the mode, a drag over the cells selects cells.
    let (from, to) = (on_sheet(&app, cx, 80.0, 40.0), on_sheet(&app, cx, 400.0, 160.0));
    drag(cx, from, to);
    assert!(selected_all(&app, cx).is_empty());
    let cells = app.update(cx, |app, _| app.sel().current());
    assert!(!cells.is_single());
    app.update_in(cx, |app, window, cx| app.run_command(crate::features::edit_tools::Cmd::SelectObjects, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.object_pointer()));
    // The rectangle holds the two shapes of the first row whole.
    drag(cx, from, to);
    let mut found = selected_all(&app, cx);
    found.sort();
    assert_eq!(found, vec![a, b]);
    // The cells did not move.
    assert_eq!(app.update(cx, |app, _| app.sel().current()), cells);
    // One that is only touched is left out; Shift keeps what was selected.
    drag_at(&app, cx, (80.0, 200.0), (130.0, 300.0), shift());
    assert_eq!(selected_all(&app, cx).len(), 2);
    drag_at(&app, cx, (80.0, 200.0), (200.0, 300.0), shift());
    assert_eq!(selected_all(&app, cx).len(), 3);
    // A click on a cell deselects; an object can still be clicked.
    click_at(&app, cx, 500.0, 300.0);
    assert!(selected_all(&app, cx).is_empty());
    click_at(&app, cx, 130.0, 260.0);
    assert_eq!(selected_all(&app, cx), vec![c]);
    // Escape puts the pointer back.
    app.update(cx, |app, cx| app.deselect_object(cx));
    cx.simulate_keystrokes("escape");
    assert!(!app.update(cx, |app, _| app.object_pointer()));
    // Go To Special ▸ Objects.
    app.update(cx, |app, cx| crate::features::misc::select_special(app, crate::features::misc::Special::Objects, cx));
    assert_eq!(selected_all(&app, cx).len(), 3);
}

#[gpui::test]
fn align_distribute_and_snap(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let a = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let b = add(&app, cx, ShapeTool::Ellipse, (300.0, 100.0, 80.0, 80.0));
    let c = add(&app, cx, ShapeTool::Rectangle, (600.0, 240.0, 60.0, 40.0));
    let select = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, ids: &[u32]| {
        let ids: Vec<_> = ids.iter().map(|id| object_id(&Drawings, *id)).collect();
        app.update(cx, |app, cx| app.select_objects(&ids, cx));
        redraw(cx);
    };
    select(&app, cx, &[a, b]);
    app.update(cx, |app, _| {
        assert!(Cmd::AlignLeft.enabled(app) && Cmd::AlignMiddle.enabled(app) && !Cmd::DistributeHorizontally.enabled(app));
    });
    run(&app, cx, Cmd::AlignTop);
    assert_eq!((frame(&app, cx, a).1, frame(&app, cx, b).1), (60, 60));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Align Objects"));
    run(&app, cx, Cmd::AlignRight);
    assert_eq!((frame(&app, cx, a).0, frame(&app, cx, b).0), (280, 300));
    app.update(cx, |app, cx| {
        app.undo(cx);
        app.undo(cx);
    });
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((100, 60, 100, 60), (300, 100, 80, 80)));
    // Three: the middle one goes half way between the gaps.
    select(&app, cx, &[a, b, c]);
    run(&app, cx, Cmd::DistributeHorizontally);
    // From 100 to 660, 240 taken: two gaps of 160.
    assert_eq!((frame(&app, cx, a).0, frame(&app, cx, b).0, frame(&app, cx, c).0), (100, 360, 600));
    run(&app, cx, Cmd::AlignMiddle);
    // The box runs from 60 to 280: its middle is 170.
    assert_eq!((frame(&app, cx, a).1, frame(&app, cx, b).1, frame(&app, cx, c).1), (140, 130, 150));
    // Snap to Grid: a dragged object lands on a cell corner.
    select(&app, cx, &[a]);
    run(&app, cx, Cmd::SnapToGrid);
    assert!(app.update(cx, |app, _| app.snap_to_grid()));
    let at = on_sheet(&app, cx, 150.0, 170.0);
    drag(cx, at, point(at.x + px(33.), at.y + px(7.)));
    let moved = frame(&app, cx, a);
    let corner = app.update(cx, |app, _| app.wb.sheets[app.sheet].corner_at(moved.0 as f64, moved.1 as f64));
    assert_eq!(corner.1, (0.0, 0.0), "{moved:?}");
    assert_ne!((moved.0, moved.1), (100, 140));
}

#[gpui::test]
fn group_ungroup_and_regroup(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let a = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let b = add(&app, cx, ShapeTool::Ellipse, (300.0, 100.0, 80.0, 80.0));
    click_at(&app, cx, 150.0, 90.0);
    app.update(cx, |app, _| assert!(!Cmd::GroupObjects.enabled(app)));
    shift_click_at(&app, cx, 340.0, 140.0);
    app.update(cx, |app, _| assert!(Cmd::GroupObjects.enabled(app) && !Cmd::Ungroup.enabled(app) && !Cmd::Regroup.enabled(app)));
    run(&app, cx, Cmd::GroupObjects);
    let group = selection(&app, cx).expect("the group is selected");
    assert_eq!(ids(&app, cx), vec![group]);
    assert_eq!(frame(&app, cx, group), (100, 60, 280, 120));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Group Objects"));
    app.update(cx, |app, _| {
        assert_eq!(Drawings.context_tabs(app)[0].id, SHAPE_TAB);
        assert!(Cmd::Ungroup.enabled(app) && !Cmd::GroupObjects.enabled(app) && Cmd::ShapeFill.enabled(app));
    });
    // A click on a member takes the group; dragging moves it whole.
    app.update(cx, |app, cx| app.deselect_object(cx));
    let on_member = on_sheet(&app, cx, 340.0, 140.0);
    click(cx, on_member);
    assert_eq!(selection(&app, cx), Some(group));
    // Between the members there is nothing to take.
    app.update(cx, |app, cx| app.deselect_object(cx));
    click_at(&app, cx, 250.0, 70.0);
    assert_eq!(selection(&app, cx), None);
    click(cx, on_member);
    drag(cx, on_member, point(on_member.x + px(20.), on_member.y + px(10.)));
    assert_eq!(frame(&app, cx, group), (120, 70, 280, 120));
    // Its corner resizes it and what it holds.
    let corner = grip(&app, cx, Grip::Resize(4));
    drag(cx, corner, point(corner.x + px(280.), corner.y));
    assert_eq!(frame(&app, cx, group), (120, 70, 560, 120));
    // A fill goes to the shapes in the group.
    app.update(cx, |app, cx| tabs::set_fill(app, Some(opensheet_engine::Color(0x00FF00)), cx));
    let members = drawing(&app, cx, group).as_group().unwrap().members.clone();
    assert!(members.iter().all(|m| m.as_shape().unwrap().fill == Some(opensheet_engine::Color(0x00FF00))));
    // Ungroup: the members are back, twice as wide, both selected.
    run(&app, cx, Cmd::Ungroup);
    let mut freed = selected_all(&app, cx);
    freed.sort();
    assert_eq!(freed, vec![a, b]);
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((120, 70, 200, 60), (520, 110, 160, 80)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Ungroup Objects"));
    // Regroup with one of them selected.
    click_at(&app, cx, 150.0, 90.0);
    app.update(cx, |app, _| assert!(Cmd::Regroup.enabled(app)));
    run(&app, cx, Cmd::Regroup);
    let again = selection(&app, cx).expect("the group again");
    assert_eq!(frame(&app, cx, again), (120, 70, 560, 120));
    app.update(cx, |app, _| assert!(!Cmd::Regroup.enabled(app)));
    // Every step undoes.
    for _ in 0..6 {
        app.update(cx, |app, cx| app.undo(cx));
    }
    assert_eq!((frame(&app, cx, a), frame(&app, cx, b)), ((100, 60, 100, 60), (300, 100, 80, 80)));
    // A chart stays out of groups.
    let chart = add_chart(&app, cx, (500.0, 300.0), (200.0, 150.0));
    app.update(cx, |app, cx| {
        let ids = [object_id(&Drawings, a), object_id(&Drawings, b), crate::objects::object_of(opensheet_engine::drawing::ObjectRef::Chart(chart))];
        app.select_objects(&ids, cx);
        assert!(!Cmd::GroupObjects.enabled(app));
        // But they are arranged together.
        assert!(Cmd::AlignLeft.enabled(app));
    });
}

#[gpui::test]
fn the_selection_pane_lists_renames_hides_and_restacks(cx: &mut TestAppContext) {
    use opensheet_engine::drawing::ObjectRef;
    let (app, cx) = open(cx);
    let a = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let chart = add_chart(&app, cx, (300.0, 60.0), (200.0, 150.0));
    let b = add(&app, cx, ShapeTool::Ellipse, (100.0, 240.0, 80.0, 80.0));
    click_at(&app, cx, 150.0, 90.0);
    run(&app, cx, Cmd::SelectionPane);
    let rows = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| {
            pane::selection_rows(app)
                .into_iter()
                .map(|row| (row.name.to_string(), row.hidden, row.selected))
                .collect::<Vec<_>>()
        })
    };
    // The one on top first.
    assert_eq!(
        rows(&app, cx),
        vec![("Oval 3".to_string(), false, false), ("Chart 2".to_string(), false, false), ("Rectangle 1".to_string(), false, true)]
    );
    let (ra, rc, rb) = (ObjectRef::Drawing(a), ObjectRef::Chart(chart), ObjectRef::Drawing(b));
    // Dragged onto the top row, the rectangle goes on top.
    app.update(cx, |app, cx| pane::move_in_stack(app, ra, rb, cx));
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.wb.object_stack(app.sheet)), vec![rc, rb, ra]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Change Order"));
    // And back under the chart.
    app.update(cx, |app, cx| pane::move_in_stack(app, ra, rc, cx));
    assert_eq!(app.update(cx, |app, _| app.wb.object_stack(app.sheet)), vec![ra, rc, rb]);
    // Hide All, Show All; a hidden object leaves the selection.
    app.update(cx, |app, cx| pane::set_hidden(app, &[ra, rc, rb], true, cx));
    redraw(cx);
    assert!(rows(&app, cx).iter().all(|row| row.1 && !row.2));
    assert!(app.update(cx, |app, _| crate::features::objects(app, app.sheet).is_empty()));
    app.update(cx, |app, cx| app.undo(cx));
    assert!(rows(&app, cx).iter().all(|row| !row.1));
    // A name typed in the pane.
    app.update(cx, |app, cx| {
        assert!(app.wb.rename_object(app.sheet, rc, "Sales"));
        app.commit("Rename Object", cx);
    });
    redraw(cx);
    assert_eq!(rows(&app, cx)[1].0, "Sales");
}

// -------------------------------------------------- pictures and effects

/// Insert a picture 200 x 100 at (100, 100) and select it.
fn add_picture(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> u32 {
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![png(200, 100)], Some((100.0, 100.0)), cx));
    redraw(cx);
    selection(app, cx).expect("the picture")
}

fn crop_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> [i32; 4] {
    drawing(app, cx, id).as_picture().expect("a picture").adjust.crop.map(|v| (v * 1000.0).round() as i32)
}

#[gpui::test]
fn a_picture_is_cropped_by_its_marks(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add_picture(&app, cx);
    let (x, y, w, h) = frame(&app, cx, id);
    run(&app, cx, Cmd::CropPicture);
    assert_eq!(app.update(cx, |app, _| cropping(app)), Some(id));
    // The mark of the right side, pulled in by a quarter of the width.
    let right = on_sheet(&app, cx, (x + w) as f64, (y + h / 2) as f64);
    drag(cx, right, point(right.x - px(w as f32 / 4.0), right.y));
    assert_eq!(frame(&app, cx, id), (x, y, w * 3 / 4, h));
    assert_eq!(crop_of(&app, cx, id), [0, 0, 250, 0]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Crop Picture"));
    // The image dragged under the frame: more of its right side shows.
    let middle = on_sheet(&app, cx, (x + w / 4) as f64, (y + h / 2) as f64);
    drag(cx, middle, point(middle.x - px(w as f32 / 10.0), middle.y));
    assert_eq!(frame(&app, cx, id), (x, y, w * 3 / 4, h));
    assert_eq!(crop_of(&app, cx, id), [100, 0, 150, 0]);
    // Escape puts the marks away; the picture stays selected.
    cx.simulate_keystrokes("escape");
    assert_eq!(app.update(cx, |app, _| cropping(app)), None);
    assert_eq!(selection(&app, cx), Some(id));
    // The whole picture is drawn cropped, and moves as one.
    drag(cx, middle, point(middle.x + px(10.), middle.y));
    assert_eq!(frame(&app, cx, id), (x + 10, y, w * 3 / 4, h));
    assert_eq!(crop_of(&app, cx, id), [100, 0, 150, 0]);
    // Aspect ratio 1:1: the middle square of the image, marks showing.
    app.update(cx, |app, cx| adjust::crop_to_frame(app, false, cx));
    cx.simulate_keystrokes("escape");
    let menu = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        run(app, cx, Cmd::Crop);
        app.update(cx, |app, cx| {
            let Some(crate::app::PopupKind::Menu(entries)) = app.popup.take().map(|popup| popup.kind) else {
                panic!("the Crop menu");
            };
            cx.notify();
            entries
        })
    };
    let entries = menu(&app, cx);
    let labels: Vec<String> = entries.iter().filter(|entry| !entry.separator).map(|entry| entry.label.to_string()).collect();
    assert_eq!(labels, ["Crop", "Crop to Shape", "Aspect Ratio", "Fill", "Fit"]);
    let ratios = entries[2].submenu.clone().expect("the ratios");
    let square = ratios.iter().find(|entry| entry.label.as_ref() == "1:1").and_then(|entry| entry.action.clone()).expect("1:1");
    app.update_in(cx, |app, window, cx| square(app, window, cx));
    redraw(cx);
    let cropped = frame(&app, cx, id);
    assert_eq!((cropped.2, cropped.3), (h, h));
    assert_eq!(crop_of(&app, cx, id), [250, 0, 250, 0]);
    assert_eq!(app.update(cx, |app, _| cropping(app)), Some(id));
    // Crop to Shape.
    let shapes = entries[1].submenu.clone().expect("the shapes");
    let oval = shapes.iter().find(|entry| entry.label.as_ref() == "Oval").and_then(|entry| entry.action.clone()).expect("Oval");
    app.update_in(cx, |app, window, cx| oval(app, window, cx));
    redraw(cx);
    assert_eq!(drawing(&app, cx, id).as_picture().unwrap().adjust.shape, ShapePreset::Ellipse);
    // Fit: the whole image in the square frame, room above and below.
    run(&app, cx, Cmd::CropFit);
    assert_eq!(crop_of(&app, cx, id), [0, -500, 0, -500]);
    // Reset Picture & Size: as it was inserted.
    run(&app, cx, Cmd::ResetPictureAndSize);
    assert_eq!(crop_of(&app, cx, id), [0, 0, 0, 0]);
    let reset = frame(&app, cx, id);
    assert_eq!((reset.2, reset.3), (w, h));
    assert_eq!(drawing(&app, cx, id).as_picture().unwrap().adjust, PictureAdjust::default());
    assert_eq!(app.update(cx, |app, _| cropping(app)), None);
}

#[gpui::test]
fn corrections_colours_transparency_and_effects(cx: &mut TestAppContext) {
    use opensheet_engine::drawing::{Recolor, Shadow, ShadowPreset};
    let (app, cx) = open(cx);
    let id = add_picture(&app, cx);
    let adjust_of = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| drawing(app, cx, id).as_picture().unwrap().adjust;
    // Every menu of the tab opens, and its entries do what they say.
    let pick = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd, path: &[&str]| {
        run(app, cx, cmd);
        let mut entries = app.update(cx, |app, cx| {
            let Some(crate::app::PopupKind::Menu(entries)) = app.popup.take().map(|popup| popup.kind) else {
                panic!("the menu of {cmd:?}");
            };
            cx.notify();
            entries
        });
        for (depth, label) in path.iter().enumerate() {
            let entry = entries.iter().find(|entry| entry.label.as_ref() == *label).unwrap_or_else(|| {
                panic!("{label} among {:?}", entries.iter().map(|e| e.label.to_string()).collect::<Vec<_>>())
            });
            if depth + 1 == path.len() {
                let action = entry.action.clone().expect("an action");
                app.update_in(cx, |app, window, cx| action(app, window, cx));
            } else {
                entries = entry.submenu.clone().expect("a submenu");
            }
        }
        redraw(cx);
    };
    pick(&app, cx, Cmd::Transparency, &["Transparency: 30%"]);
    assert_eq!(adjust_of(&app, cx).transparency, 0.3);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Picture Transparency"));
    pick(&app, cx, Cmd::Corrections, &["Brightness", "Brightness: +20%"]);
    pick(&app, cx, Cmd::Corrections, &["Contrast", "Contrast: -40%"]);
    assert_eq!((adjust_of(&app, cx).brightness, adjust_of(&app, cx).contrast), (0.2, -0.4));
    pick(&app, cx, Cmd::Color, &["Saturation", "Saturation: 200%"]);
    pick(&app, cx, Cmd::Color, &["Color Tone", "Temperature: 4700 K"]);
    pick(&app, cx, Cmd::Color, &["Recolor", "Grayscale"]);
    let now = adjust_of(&app, cx);
    assert_eq!((now.saturation, now.temperature, now.recolor), (2.0, 4700.0, Recolor::Grayscale));
    // The picture is drawn with all of it, and without.
    redraw(cx);
    pick(&app, cx, Cmd::PictureEffects, &["Shadow", "Offset: Bottom Right"]);
    assert_eq!(drawing(&app, cx, id).effects.shadow, Some(Shadow::default()));
    pick(&app, cx, Cmd::PictureEffects, &["Glow", "8 pt", "Accent 2"]);
    let glow = drawing(&app, cx, id).effects.glow.expect("a glow");
    assert_eq!((glow.radius, glow.color.value()), (8.0, app.update(cx, |app, _| app.wb.theme().chart_color(1).value())));
    pick(&app, cx, Cmd::PictureEffects, &["Soft Edges", "5 pt"]);
    assert_eq!(drawing(&app, cx, id).effects.soft_edge, 5.0);
    redraw(cx);
    // The pane: its pages for a picture, and a number typed there.
    app.update_in(cx, |app, window, cx| pane::open_format(app, pane::Section::Effects, window, cx));
    redraw(cx);
    app.update(cx, |app, cx| {
        pane::Look::ShadowDistance.set(app, 12.0, cx);
        pane::Look::ShadowAngle.set(app, 400.0, cx);
        pane::Look::Transparency.set(app, 55.0, cx);
        pane::Look::GlowSize.set(app, 0.0, cx);
    });
    redraw(cx);
    let changed = drawing(&app, cx, id);
    let shadow = changed.effects.shadow.expect("a shadow");
    // Out of range, the angle is brought back in.
    assert_eq!((shadow.distance, shadow.angle), (12.0, 359.9));
    assert_eq!(changed.effects.glow, None);
    assert_eq!(changed.as_picture().unwrap().adjust.transparency, 0.55);
    assert_eq!(pane::Look::ShadowDistance.of(&changed), Some(12.0));
    app.update_in(cx, |app, window, cx| pane::open_format(app, pane::Section::Picture, window, cx));
    redraw(cx);
    // Reset Picture keeps the size, undoes the rest.
    run(&app, cx, Cmd::ResetPictureOnly);
    let reset = drawing(&app, cx, id);
    assert_eq!(reset.as_picture().unwrap().adjust, PictureAdjust::default());
    assert!(reset.effects.is_none());

    // A shape takes shadows and glows too (not soft edges), and a group
    // hands them to its members.
    let a = add(&app, cx, ShapeTool::Rectangle, (400.0, 60.0, 100.0, 60.0));
    let b = add(&app, cx, ShapeTool::Arrow, (400.0, 200.0, 100.0, 60.0));
    app.update(cx, |app, cx| app.select_objects(&[object_id(&Drawings, a), object_id(&Drawings, b)], cx));
    pick(&app, cx, Cmd::ShapeEffects, &["Shadow", "Offset: Top"]);
    for shape in [a, b] {
        assert_eq!(drawing(&app, cx, shape).effects.shadow, Some(ShadowPreset::Top.shadow()));
    }
    run(&app, cx, Cmd::GroupObjects);
    pick(&app, cx, Cmd::ShapeEffects, &["Shadow", "No Shadow"]);
    let group = selection(&app, cx).unwrap();
    assert!(drawing(&app, cx, group).as_group().unwrap().members.iter().all(|m| m.effects.shadow.is_none()));
    app.update_in(cx, |app, window, cx| pane::open_format(app, pane::Section::Effects, window, cx));
    redraw(cx);
}

#[gpui::test]
fn the_format_pane_follows_the_kind_of_object(cx: &mut TestAppContext) {
    use crate::features::chart_format;
    let (app, cx) = open(cx);
    let shape = add(&app, cx, ShapeTool::Rectangle, (100.0, 60.0, 100.0, 60.0));
    let chart = add_chart(&app, cx, (300.0, 60.0), (300.0, 200.0));
    let open_panes = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| (app.pane_open(chart_format::PANE), app.pane_open(pane::FORMAT)))
    };
    let title = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| app.panes.get(app.active_pane).map(|pane| pane.title.to_string()))
    };
    // The pane of the chart, opened on the chart.
    click_at(&app, cx, 450.0, 160.0);
    app.update_in(cx, |app, window, cx| chart_format::open_pane(app, window, cx));
    redraw(cx);
    assert_eq!(open_panes(&app, cx), (true, false));
    // A shape is selected: the pane is the shape's, with its pages.
    click_at(&app, cx, 150.0, 90.0);
    assert_eq!(selection(&app, cx), Some(shape));
    assert_eq!(open_panes(&app, cx), (false, true));
    assert_eq!(title(&app, cx).as_deref(), Some("Format Shape"));
    // A picture: Format Picture.
    let picture = add_picture(&app, cx);
    assert_eq!(selection(&app, cx), Some(picture));
    assert_eq!(title(&app, cx).as_deref(), Some("Format Picture"));
    // And back to the chart.
    click_at(&app, cx, 450.0, 220.0);
    assert_eq!(app.update(cx, |app, _| app.selected_chart().map(|c| c.id)), Some(chart));
    assert_eq!(open_panes(&app, cx), (true, false));
    // With nothing selected the pane stays as it is.
    click_at(&app, cx, 700.0, 400.0);
    assert_eq!(open_panes(&app, cx), (true, false));
}
