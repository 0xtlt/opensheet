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
    // A turned shape is taken where it was turned to.
    let tall = add(&app, cx, ShapeTool::Rectangle, (100.0, 240.0, 200.0, 20.0));
    change(&app, cx, tall, |d| d.rotation = 90.0);
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
        assert!(!Cmd::Group.enabled(app));
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
    for cmd in [Cmd::Rotate, Cmd::Align, Cmd::EditShape, Cmd::Pictures] {
        run(&app, cx, cmd);
        app.update(cx, |app, cx| {
            assert!(matches!(app.popup.as_ref().map(|popup| &popup.kind), Some(crate::app::PopupKind::Menu(_))), "{cmd:?}");
            app.popup = None;
            cx.notify();
        });
    }
    // What is not available says so.
    run(&app, cx, Cmd::ShapeEffects);
    app.update(cx, |app, _| assert!(app.status_message.as_ref().is_some_and(|message| message.contains(Cmd::ShapeEffects.label()))));
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
