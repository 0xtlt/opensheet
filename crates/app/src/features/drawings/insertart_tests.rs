//! WordArt, icons, SmartArt and equations driven in GPUI's headless
//! window, on the real application. What these tests cannot tell is how
//! things look.

use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::style::Color;
use opensheet_engine::wordart::WordArtStyle;

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

fn drawing(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Drawing {
    app.update(cx, |app, _| app.wb.drawing(app.sheet, id).expect("the drawing").clone())
}

fn ids(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<u32> {
    app.update(cx, |app, _| app.wb.sheets[app.sheet].drawings.iter().map(|d| d.id).collect())
}

fn undo(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

// ------------------------------------------------------------------ WordArt

#[gpui::test]
fn wordart_is_inserted_with_its_text_ready_to_be_replaced(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // The gallery opens under its button and draws its samples.
    run(&app, cx, Cmd::WordArt);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    app.update_in(cx, |app, window, cx| {
        app.popup = None;
        wordart::insert(app, WordArtStyle(3), window, cx);
    });
    redraw(cx);
    let id = ids(&app, cx)[0];
    let art = drawing(&app, cx, id);
    let shape = art.as_shape().unwrap();
    assert_eq!(shape.text, "Your text here");
    assert!(shape.text_box && shape.fill.is_none() && shape.outline.is_none() && !shape.wrap);
    assert!(shape.effects.outline.is_some() && shape.effects.shadow.is_some());
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert WordArt"));
    // It is selected, its tab shows, and typing replaces the placeholder.
    assert_eq!(app.update(cx, |app, _| (editing_text(app), app.context_tab)), (Some(id), Some(SHAPE_TAB)));
    let wide = art.size.0;
    assert!(wide > 300.0 && art.size.1 > 80.0, "{:?}", art.size);
    cx.simulate_input("Hi");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    let art = drawing(&app, cx, id);
    assert_eq!(art.as_shape().unwrap().text, "Hi");
    // The box fits its text again.
    assert!(art.size.0 < wide / 2.0, "{:?}", art.size);
    // One step for the text, one for the insertion.
    undo(&app, cx);
    assert_eq!(drawing(&app, cx, id).as_shape().unwrap().text, "Your text here");
    undo(&app, cx);
    assert!(ids(&app, cx).is_empty());
}

#[gpui::test]
fn formatting_reaches_the_selected_characters_of_a_shape(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = app.update(cx, |app, cx| {
        let id = app.wb.add_shape(app.sheet, ShapeTool::Rectangle, Frame::new(100.0, 60.0, 220.0, 90.0)).unwrap();
        app.wb.update_drawing(app.sheet, id, |drawing| drawing.as_shape_mut().unwrap().text = "Total".to_string());
        app.commit("setup", cx);
        app.select_object(Some(object_id(&Drawings, id)), cx);
        id
    });
    redraw(cx);
    // Type after the text, select the last two characters, make them bold.
    cx.simulate_input(" sales");
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_keystrokes("shift-left shift-left");
    app.update(cx, |app, cx| app.toggle_bold(cx));
    redraw(cx);
    let shape = drawing(&app, cx, id).as_shape().unwrap().clone();
    assert_eq!(shape.text, "Total sales");
    assert_eq!(shape.runs.iter().map(|run| (run.len, run.font.bold)).collect::<Vec<_>>(), vec![(9, false), (2, true)]);
    // Still typing: what is typed next keeps the runs.
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(drawing(&app, cx, id).as_shape().unwrap().runs.len(), 2);
    // With the shape selected and no text being typed, commands format
    // all of it: a style of the gallery, an outline, a shadow, a fill.
    run(&app, cx, Cmd::WordArtStyles);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    app.update(cx, |app, cx| {
        app.popup = None;
        wordart::format(app, "style", cx, &|font, effects| WordArtStyle(2).apply(font, effects));
    });
    redraw(cx);
    let shape = drawing(&app, cx, id).as_shape().unwrap().clone();
    assert!(shape.runs.iter().all(|run| run.effects.outline.is_some() && run.font.color == Some(Color(0xE97132))));
    assert!(shape.runs[1].font.bold && !shape.runs[0].font.bold);
    app.update(cx, |app, cx| wordart::set_fill(app, None, cx));
    assert!(drawing(&app, cx, id).as_shape().unwrap().effects.hollow);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text Fill"));
    undo(&app, cx);
    assert!(!drawing(&app, cx, id).as_shape().unwrap().effects.hollow);
    // The drop-downs of the tab open on a shape with text.
    for cmd in [Cmd::TextOutline, Cmd::TextEffects, Cmd::TextFill] {
        assert!(app.update(cx, |app, _| cmd.enabled(app)), "{cmd:?}");
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.popup.take().is_some()), "{cmd:?}");
    }
    // A protected sheet keeps the look of its locked shapes.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(sheet, opensheet_engine::protect::SheetProtection::default(), "");
        app.commit("setup", cx);
        assert!(!wordart::format(app, "style", cx, &|font, _| font.italic = true));
    });
    assert!(!drawing(&app, cx, id).as_shape().unwrap().font.italic);
}
