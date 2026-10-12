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

// -------------------------------------------------- icons and SVG pictures

/// The pixels of an image file: red, green, blue, alpha.
fn decoded(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).expect("an image").into_rgba8()
}

#[gpui::test]
fn icons_are_inserted_as_vector_pictures(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // The command opens the dialog, which draws its icons.
    run(&app, cx, Cmd::Icons);
    assert!(app.update(cx, |app, _| app.modal_open()));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    let dialog = app.update_in(cx, |app, window, cx| icons::open_dialog(app, false, window, cx)).expect("the dialog");
    redraw(cx);
    // Search narrows what is shown; two icons are ticked, one of them
    // twice (which unticks it), then a third.
    let found = dialog.update(cx, |dialog, cx| dialog.search_for("heart", cx));
    assert_eq!(found.len(), 1);
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        dialog.toggle(found[0], cx);
        dialog.toggle(0, cx);
        dialog.toggle(0, cx);
        dialog.toggle(5, cx);
    });
    dialog.update_in(cx, |dialog, window, cx| dialog.insert(&(), window, cx));
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.modal_open()));
    let all = ids(&app, cx);
    assert_eq!(all.len(), 2);
    let heart = drawing(&app, cx, all[0]);
    let picture = heart.as_picture().expect("a picture");
    assert!(picture.svg.is_some() && icons::is_graphic(&heart));
    assert_eq!((heart.name.as_str(), heart.alt_text.as_str(), heart.size), ("Graphic 1", "Heart", (96.0, 96.0)));
    // The image kept beside the document: twice its size, and drawn.
    let image = decoded(&picture.data);
    assert_eq!(image.dimensions(), (192, 192));
    assert!(image.pixels().any(|pixel| pixel[3] == 255) && image.pixels().any(|pixel| pixel[3] == 0));
    // The last one is selected and its tab shows.
    assert_eq!(app.update(cx, |app, _| (selected(app).map(|d| d.id), app.context_tab)), (Some(all[1]), Some(GRAPHICS_TAB)));
    assert_eq!(app.update(cx, |app, _| crate::features::context_tabs(app)[0].1.title), "Graphics Format");
    for cmd in [Cmd::GraphicsFill, Cmd::GraphicsOutline, Cmd::ChangeGraphic] {
        assert!(app.update(cx, |app, _| cmd.enabled(app)), "{cmd:?}");
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.popup.take().is_some()), "{cmd:?}");
    }
    assert!(!app.update(cx, |app, _| Cmd::ConvertToShape.enabled(app)));
    // Graphics Fill repaints the document and the image made of it.
    let red = opensheet_engine::svg::Repaint::Fill(Some(Color(0xC00000)));
    assert!(app.update(cx, |app, cx| icons::repaint_selected(app, "Graphics Fill", red, cx)));
    redraw(cx);
    let painted = drawing(&app, cx, all[1]);
    let picture = painted.as_picture().unwrap();
    assert!(std::str::from_utf8(picture.svg.as_deref().unwrap()).unwrap().contains("fill=\"#C00000\""));
    assert!(decoded(&picture.data).pixels().any(|pixel| pixel.0 == [0xC0, 0, 0, 255]));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Graphics Fill"));
    undo(&app, cx);
    assert!(!decoded(&drawing(&app, cx, all[1]).as_picture().unwrap().data).pixels().any(|pixel| pixel[0] > 0x80));
    // Another icon in its frame.
    app.update(cx, |app, cx| {
        app.wb.set_drawing_frame(app.sheet, all[1], Frame::new(300.0, 100.0, 48.0, 48.0));
        app.commit("setup", cx);
    });
    let dialog = app.update_in(cx, |app, window, cx| icons::open_dialog(app, true, window, cx)).expect("the dialog");
    dialog.update(cx, |dialog, cx| {
        dialog.toggle(3, cx);
        dialog.toggle(7, cx);
    });
    dialog.update_in(cx, |dialog, window, cx| dialog.insert(&(), window, cx));
    redraw(cx);
    let changed = drawing(&app, cx, all[1]);
    assert_eq!((ids(&app, cx).len(), changed.size), (2, (48.0, 48.0)));
    assert_ne!(changed.as_picture().unwrap().svg, painted.as_picture().unwrap().svg);
    // The insertion was one step for both icons.
    undo(&app, cx);
    undo(&app, cx);
    undo(&app, cx);
    assert!(ids(&app, cx).is_empty());
}

#[gpui::test]
fn svg_files_come_in_as_pictures_and_are_drawn_at_any_turn(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let file = br##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="120" height="60" viewBox="0 0 12 6"><rect width="6" height="6" fill="#0070C0"/><text x="7" y="5" font-size="5">A</text></svg>"##;
    app.update(cx, |app, cx| tool::insert_pictures(app, vec![file.to_vec(), b"no picture".to_vec()], None, cx));
    redraw(cx);
    let all = ids(&app, cx);
    assert_eq!(all.len(), 1);
    let inserted = drawing(&app, cx, all[0]);
    assert_eq!((inserted.size, inserted.lock_aspect), ((120.0, 60.0), true));
    let picture = inserted.as_picture().unwrap();
    assert_eq!(picture.svg.as_deref(), Some(&file[..]));
    // The left half of its image is the blue square.
    let image = decoded(&picture.data);
    assert_eq!(image.dimensions(), (240, 120));
    assert_eq!(image.get_pixel(60, 60).0, [0x00, 0x70, 0xC0, 255]);
    // Upright, on its side, flipped and at an odd angle, zoomed: each is
    // painted (from the document or, turned oddly, from the image).
    for (rotation, flip, zoom) in [(0.0, false, 1.0f32), (90.0, true, 2.5), (30.0, false, 1.0), (270.0, false, 0.4)] {
        app.update(cx, |app, cx| {
            app.wb.update_drawing(app.sheet, all[0], |drawing| {
                drawing.rotation = rotation;
                drawing.flip_h = flip;
            });
            app.commit("setup", cx);
            app.set_zoom(zoom, cx);
        });
        redraw(cx);
        let placed = app.update(cx, |app, _| {
            let layout = app.current_layout().unwrap();
            let pane = *paint::panes(&layout).last().unwrap();
            paint::placed(&layout, &app.wb.sheets[app.sheet], selected(app).unwrap(), &pane)
        });
        let vector = icons::image_for(inserted.as_picture().unwrap(), &placed, 2.0);
        assert_eq!(vector.is_some(), rotation != 30.0, "{rotation}");
        if let Some((image, _)) = vector {
            let (w, h) = (image.size(0).width.0, image.size(0).height.0);
            // Drawn at the size it is shown: sharper as the zoom grows.
            let expected = (120.0 * zoom * 2.0).round() as i32;
            // (To a pixel or two: frames end on cell edges.)
            let drawn = if rotation == 0.0 { w } else { h };
            assert!((drawn - expected).abs() <= 2, "{rotation} {zoom}: {drawn}");
        }
    }
}

/// Every icon of the gallery on one sheet, written where
/// `OPENSHEET_SAMPLES` says: to look at what was drawn.
#[gpui::test]
fn the_icons_draw(cx: &mut TestAppContext) {
    let (_app, cx) = open(cx);
    cx.update(|_, cx| svg::install(cx));
    let all = icon_set::icons();
    let (side, columns) = (72u32, 9u32);
    let rows = (all.len() as u32).div_ceil(columns);
    let mut sheet = image::RgbaImage::from_pixel(columns * side, rows * side, image::Rgba([255, 255, 255, 255]));
    for (i, icon) in all.iter().enumerate() {
        let sized = opensheet_engine::svg::resized(icon.svg().as_bytes(), 56.0, 56.0).unwrap();
        let bitmap = svg::rasterize(&sized).unwrap_or_else(|| panic!("{} does not draw", icon.english));
        // Something is drawn, and not everything.
        let inked = bitmap.data.chunks_exact(4).filter(|pixel| pixel[3] > 128).count();
        assert!(inked > 150 && inked < 56 * 56 * 9 / 10, "{}: {inked}", icon.english);
        let (x0, y0) = ((i as u32 % columns) * side + 8, (i as u32 / columns) * side + 8);
        for (at, pixel) in bitmap.data.chunks_exact(4).enumerate() {
            let (x, y) = (x0 + at as u32 % bitmap.width, y0 + at as u32 / bitmap.width);
            let shade = 255 - pixel[3];
            sheet.put_pixel(x, y, image::Rgba([shade, shade, shade, 255]));
        }
    }
    if let Some(directory) = std::env::var_os("OPENSHEET_SAMPLES") {
        sheet.save(std::path::Path::new(&directory).join("opensheet-icons.png")).expect("the sheet of icons");
    }
}
