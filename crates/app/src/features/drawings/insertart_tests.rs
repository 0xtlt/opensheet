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
    // The colours of a style are those of the workbook's theme, and follow it.
    let accent_2 = app.update(cx, |app, _| app.wb.theme().accent(2, 0.0));
    assert!(accent_2.is_themed());
    assert!(shape.runs.iter().all(|run| run.effects.outline.is_some() && run.font.color == Some(accent_2)));
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

// ----------------------------------------------------------------- SmartArt

use opensheet_engine::smartart::{ColorScheme, Layout, QuickStyle, SmartArt};

fn art(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> SmartArt {
    drawing(app, cx, id).as_smartart().expect("a graphic").clone()
}

fn outline(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> String {
    art(app, cx, id).outline().iter().map(|(level, text)| format!("{}{text}", ">".repeat(*level))).collect::<Vec<_>>().join("|")
}

fn choose_line(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, line: usize) {
    app.update(cx, |app, _| app.ext_mut::<smartart::SmartUi>().set_node(id, line));
}

#[gpui::test]
fn a_smartart_graphic_is_chosen_typed_and_restructured(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // Insert ▸ SmartArt opens the dialog; a layout is picked and OK inserts.
    run(&app, cx, Cmd::SmartArt);
    assert!(app.update(cx, |app, _| app.modal_open()));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    let dialog = app.update_in(cx, |app, window, cx| smartart::open_dialog(app, window, cx)).expect("the dialog");
    redraw(cx);
    dialog.update_in(cx, |dialog, window, cx| {
        dialog.layout = Layout::Process;
        dialog.ok(&(), window, cx);
    });
    redraw(cx);
    let id = ids(&app, cx)[0];
    let inserted = drawing(&app, cx, id);
    assert_eq!((inserted.name.as_str(), inserted.size), ("Diagram 1", (576.0, 384.0)));
    assert_eq!(art(&app, cx, id), SmartArt::new(Layout::Process));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert SmartArt Graphic"));
    // It is selected, its tab shows and its Text Pane is open on the
    // first line: what is typed goes there, and the graphic follows.
    assert_eq!(app.update(cx, |app, _| (selected(app).map(|d| d.id), app.context_tab, smartart::pane_shown(app))), (Some(id), Some(SMARTART_TAB), true));
    cx.simulate_input("Plan");
    assert_eq!(outline(&app, cx, id), "Plan||");
    // Return adds a line under it; Tab makes it a detail of the first.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_input("Scope");
    cx.simulate_keystrokes("tab");
    redraw(cx);
    assert_eq!(outline(&app, cx, id), "Plan|>Scope||");
    cx.simulate_keystrokes("shift-tab");
    redraw(cx);
    assert_eq!(outline(&app, cx, id), "Plan|Scope||");
    // Down goes to the next line; Backspace on an empty line removes it.
    cx.simulate_keystrokes("down");
    redraw(cx);
    cx.simulate_keystrokes("backspace");
    redraw(cx);
    assert_eq!(outline(&app, cx, id), "Plan|Scope|");
    let pane = app.update(cx, |app, _| smartart::pane(app)).expect("the pane");
    assert_eq!(pane.read_with(cx, |pane, cx| pane.texts(cx)), vec!["Plan", "Scope", ""]);
    // Typing is one undo step, each change of the outline another.
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Delete"));
    undo(&app, cx);
    undo(&app, cx);
    undo(&app, cx);
    assert_eq!(outline(&app, cx, id), "Plan|Scope||");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Typing"));
    undo(&app, cx);
    assert_eq!(outline(&app, cx, id), "Plan|||");
    redraw(cx);
    assert_eq!(pane.read_with(cx, |pane, cx| pane.texts(cx)), vec!["Plan", "", "", ""]);
}

#[gpui::test]
fn the_smartart_design_tab_changes_the_graphic(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = app.update_in(cx, |app, window, cx| smartart::insert(app, Layout::OrgChart, window, cx)).expect("a graphic");
    redraw(cx);
    app.update(cx, |app, cx| {
        app.wb.update_drawing(app.sheet, id, |drawing| {
            let lines = [(0, "Head"), (1, "Sales"), (1, "Works"), (2, "Plant")];
            drawing.as_smartart_mut().unwrap().set_outline(&lines.map(|(level, text)| (level, text.to_string())));
        });
        app.commit("setup", cx);
    });
    redraw(cx);
    // A click on a box of the selected graphic chooses its entry.
    let at = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let pane = *paint::panes(&layout).last().unwrap();
        let drawing = selected(app).unwrap();
        let frame = app.wb.sheets[app.sheet].drawing_frame(drawing);
        let placed = paint::placed(&layout, &app.wb.sheets[app.sheet], drawing, &pane);
        let pieces = drawing.as_smartart().unwrap().pieces(app.wb.theme(), frame.width as f32, frame.height as f32);
        let works = pieces.iter().find(|piece| piece.shape.text == "Works").unwrap();
        let at = smartart::placed_piece(works, &placed, (frame.width as f32, frame.height as f32)).centre();
        layout.to_window(at.0, at.1)
    });
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| smartart::current_line(app)), Some(2));
    assert_eq!(frame_of(&app, cx, id), frame_of(&app, cx, id));
    // Add Shape, around the chosen entry.
    run(&app, cx, Cmd::SmartAddShape);
    assert!(app.update(cx, |app, _| app.popup.take().is_some()));
    run(&app, cx, Cmd::SmartAddAfter);
    assert_eq!(outline(&app, cx, id), "Head|>Sales|>Works|>>Plant|>");
    assert_eq!(app.update(cx, |app, _| smartart::current_line(app)), Some(4));
    undo(&app, cx);
    choose_line(&app, cx, id, 2);
    run(&app, cx, Cmd::SmartAddAbove);
    assert_eq!(outline(&app, cx, id), "Head|>Sales|>|>>Works|>>>Plant");
    undo(&app, cx);
    choose_line(&app, cx, id, 2);
    run(&app, cx, Cmd::SmartAddBullet);
    assert_eq!(outline(&app, cx, id), "Head|>Sales|>Works|>>Plant|>>");
    undo(&app, cx);
    // Move Up, Promote, and what cannot be done is greyed out.
    choose_line(&app, cx, id, 2);
    assert!(app.update(cx, |app, _| Cmd::SmartMoveUp.enabled(app) && !Cmd::SmartMoveDown.enabled(app)));
    run(&app, cx, Cmd::SmartMoveUp);
    assert_eq!(outline(&app, cx, id), "Head|>Works|>>Plant|>Sales");
    assert_eq!(app.update(cx, |app, _| smartart::current_line(app)), Some(1));
    run(&app, cx, Cmd::SmartPromote);
    assert_eq!(outline(&app, cx, id), "Head|Works|>Plant|>Sales");
    assert!(app.update(cx, |app, _| !Cmd::SmartPromote.enabled(app) && Cmd::SmartDemote.enabled(app)));
    run(&app, cx, Cmd::SmartDemote);
    assert_eq!(outline(&app, cx, id), "Head|>Works|>>Plant|>>Sales");
    // Right to Left is a switch.
    run(&app, cx, Cmd::SmartRightToLeft);
    assert!(art(&app, cx, id).rtl && app.update(cx, |app, _| Cmd::SmartRightToLeft.checked(app)));
    run(&app, cx, Cmd::SmartRightToLeft);
    // The galleries open and draw; what they choose keeps the text.
    for (cmd, choice) in [(Cmd::SmartLayouts, smartart::Choice::Layouts), (Cmd::SmartColors, smartart::Choice::Colors), (Cmd::SmartStyles, smartart::Choice::Styles)] {
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.popup.take().is_some()), "{choice:?}");
    }
    let before = outline(&app, cx, id);
    app.update(cx, |app, cx| {
        smartart::change(app, "layout", cx, |art, _| {
            art.set_layout(Layout::Cycle);
            true
        });
        smartart::change(app, "look", cx, |art, _| {
            (art.colors, art.style) = (ColorScheme::Colorful(1), QuickStyle::WhiteOutline);
            true
        });
    });
    redraw(cx);
    assert_eq!((art(&app, cx, id).layout, outline(&app, cx, id)), (Layout::Cycle, before));
    run(&app, cx, Cmd::SmartReset);
    assert_eq!((art(&app, cx, id).colors, art(&app, cx, id).style), (ColorScheme::default(), QuickStyle::Simple));
    // Nothing to reset: no undo step.
    let label = undo_label(&app, cx);
    run(&app, cx, Cmd::SmartReset);
    assert_eq!(undo_label(&app, cx), label);
    // A protected sheet keeps its graphics as they are.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(sheet, opensheet_engine::protect::SheetProtection::default(), "");
        app.commit("setup", cx);
        assert!(!smartart::add_shape(app, smartart::Side::After, cx));
        app.wb.unprotect_sheet(sheet, "").expect("no password");
        app.commit("setup", cx);
    });
    // The message that said so is dismissed.
    assert!(app.update(cx, |app, _| app.modal_open()));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    redraw(cx);
    // Convert to Shapes: the two boxes of a process and the arrow between
    // them, where they were; the graphic and its pane are gone.
    app.update(cx, |app, cx| {
        assert_eq!(selected(app).map(|d| d.id), Some(id), "still selected");
        smartart::change(app, "setup", cx, |art, _| {
            art.set_layout(Layout::Process);
            art.set_outline(&[(0, "Head".to_string()), (0, "Works".to_string())]);
            true
        });
    });
    let frame = frame_of(&app, cx, id);
    run(&app, cx, Cmd::SmartConvert);
    let shapes: Vec<Drawing> = ids(&app, cx).into_iter().map(|id| drawing(&app, cx, id)).collect();
    assert_eq!(shapes.len(), 3);
    assert!(shapes.iter().all(|shape| shape.as_shape().is_some()));
    assert_eq!((shapes[0].as_shape().unwrap().text.as_str(), shapes[2].as_shape().unwrap().text.as_str()), ("Head", "Works"));
    assert_eq!(shapes[1].as_shape().unwrap().preset, opensheet_engine::drawing::ShapePreset::RightArrow);
    let first = frame_of(&app, cx, shapes[0].id);
    assert!(first.0 >= frame.0 && first.1 >= frame.1 && first.0 + first.2 <= frame.0 + frame.2);
    assert!(!app.update(cx, |app, _| smartart::pane_shown(app)));
    undo(&app, cx);
    assert_eq!(ids(&app, cx), vec![id]);
}

/// Where a drawing is on its sheet: (x, y, width, height), rounded.
fn frame_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (i64, i64, i64, i64) {
    app.update(cx, |app, _| {
        let frame = app.wb.drawing_frame(app.sheet, id).expect("the drawing");
        (frame.x.round() as i64, frame.y.round() as i64, frame.width.round() as i64, frame.height.round() as i64)
    })
}

#[gpui::test]
fn a_graphic_filled_with_a_theme_colour_follows_the_theme(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let icon = &icon_set::icons()[2];
    app.update(cx, |app, cx| icons::insert_icons(app, &[icon], cx));
    redraw(cx);
    let id = ids(&app, cx)[0];
    let accent = app.update(cx, |app, cx| {
        let accent = app.wb.theme().accent(2, 0.0);
        assert!(icons::repaint_selected(app, "Graphics Fill", opensheet_engine::svg::Repaint::Fill(Some(accent)), cx));
        accent
    });
    redraw(cx);
    let rgb = |color: Color| [color.r(), color.g(), color.b(), 255];
    assert!(decoded(&drawing(&app, cx, id).as_picture().unwrap().data).pixels().any(|pixel| pixel.0 == rgb(accent)));
    // Another theme: the document and, at the next repaint, the image
    // kept beside it take the colour the theme gives accent 2.
    let other = opensheet_engine::theme::Theme::office_2013();
    let moved = other.accent(2, 0.0);
    assert_ne!(moved.value(), accent.value());
    app.update(cx, |app, cx| {
        app.wb.set_theme(other);
        app.commit("Theme", cx);
    });
    redraw(cx);
    let picture = drawing(&app, cx, id).as_picture().unwrap().clone();
    assert!(std::str::from_utf8(picture.svg.as_deref().unwrap()).unwrap().contains(&format!("fill=\"#{}\"", moved.to_hex())));
    assert!(decoded(&picture.data).pixels().any(|pixel| pixel.0 == rgb(moved)));
    // Undo brings back the theme, the document and its image.
    undo(&app, cx);
    redraw(cx);
    assert!(decoded(&drawing(&app, cx, id).as_picture().unwrap().data).pixels().any(|pixel| pixel.0 == rgb(accent)));
}

// ---------------------------------------------------------------- equations

use opensheet_engine::math::{BUILT_IN, Equation, Structure};

fn equation_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (String, Equation) {
    let shape = drawing(app, cx, id).as_shape().expect("a shape").clone();
    (shape.text, shape.math.expect("an equation"))
}

#[gpui::test]
fn a_built_in_equation_is_inserted_and_shown(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // Insert ▸ Equation opens the gallery of the built-in equations.
    run(&app, cx, Cmd::Equation);
    assert!(app.update(cx, |app, _| app.popup.take().is_some()));
    let quadratic = BUILT_IN[5].1;
    let id = app.update_in(cx, |app, window, cx| equation::insert(app, quadratic, window, cx)).expect("an equation");
    redraw(cx);
    let (text, math) = equation_of(&app, cx, id);
    assert_eq!((text.as_str(), &math), (quadratic, &Equation::parse(quadratic)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Equation"));
    // A text box without fill, as large as its equation.
    let inserted = drawing(&app, cx, id);
    let shape = inserted.as_shape().unwrap();
    assert!(shape.text_box && shape.fill.is_none() && shape.outline.is_none());
    assert!(inserted.size.0 >= 180.0 && inserted.size.1 > 40.0, "{:?}", inserted.size);
    // Selected, with the Equation tab beside Shape Format.
    let tabs: Vec<&str> = app.update(cx, |app, _| crate::features::context_tabs(app).into_iter().map(|(_, tab)| tab.title).collect());
    assert_eq!(tabs, vec!["Shape Format", "Equation"]);
    assert_eq!(app.update(cx, |app, _| app.context_tab), Some(EQUATION_TAB));
    // Linear and Professional switch how it is shown.
    assert!(app.update(cx, |app, _| Cmd::EqProfessional.checked(app) && !Cmd::EqLinear.checked(app)));
    run(&app, cx, Cmd::EqLinear);
    assert!(equation_of(&app, cx, id).1.linear && app.update(cx, |app, _| Cmd::EqLinear.checked(app)));
    assert!(drawing(&app, cx, id).size.1 < inserted.size.1);
    run(&app, cx, Cmd::EqProfessional);
    assert!(!equation_of(&app, cx, id).1.linear);
    // Every gallery of the tab opens and draws its structures.
    let galleries = [Cmd::EqSymbols, Cmd::EqFraction, Cmd::EqScript, Cmd::EqRadical, Cmd::EqIntegral, Cmd::EqLargeOperator, Cmd::EqBracket, Cmd::EqFunction, Cmd::EqAccent, Cmd::EqLimit, Cmd::EqOperator, Cmd::EqMatrix];
    for cmd in galleries {
        assert!(app.update(cx, |app, _| cmd.enabled(app)), "{cmd:?}");
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.popup.take().is_some()), "{cmd:?}");
    }
    assert_eq!(galleries[1..].iter().filter_map(|cmd| equation::structure_of(*cmd)).collect::<Vec<_>>(), Structure::ALL.to_vec());
    // A structure chosen while the shape is only selected goes to the
    // end of the equation, a symbol too.
    app.update_in(cx, |app, window, cx| {
        assert!(equation::insert_text(app, "+", window, cx));
        assert!(equation::insert_text(app, Structure::Fraction.templates()[0], window, cx));
    });
    assert_eq!(equation_of(&app, cx, id).0, format!("{quadratic}+⬚/⬚"));
    undo(&app, cx);
    undo(&app, cx);
    // The size of the letters is the shape's: Home ▸ Font Size.
    app.update(cx, |app, cx| app.set_font_size(24.0, cx));
    redraw(cx);
    let larger = drawing(&app, cx, id);
    assert_eq!(larger.as_shape().unwrap().font.size_pt(), 24.0);
    assert!(larger.size.1 > inserted.size.1 * 1.5, "{:?}", larger.size);
    // On another shape the commands of the tab are greyed out.
    app.update(cx, |app, cx| {
        let other = app.wb.add_shape(app.sheet, ShapeTool::Rectangle, Frame::new(10.0, 10.0, 80.0, 40.0)).unwrap();
        app.commit("setup", cx);
        app.select_object(Some(object_id(&Drawings, other)), cx);
        assert!(!Cmd::EqFraction.enabled(app) && !Cmd::EqLinear.enabled(app) && Cmd::Equation.enabled(app));
    });
}

#[gpui::test]
fn an_equation_is_typed_in_the_linear_format(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // A new equation is empty and ready to be typed in.
    let id = app.update_in(cx, |app, window, cx| equation::insert(app, "", window, cx)).expect("an equation");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(id));
    cx.simulate_input("x=\\alpha^2/b");
    redraw(cx);
    // As it is typed, it is built up under its shape.
    let preview = app.update(cx, |app, _| app.ext::<equation::Typing>().and_then(|typing| typing.preview()));
    assert_eq!(preview, Some(Equation::parse("x=α^2/b")));
    // A structure of the galleries goes where the caret is.
    app.update_in(cx, |app, window, cx| assert!(equation::insert_text(app, "+√⬚", window, cx)));
    redraw(cx);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    let (text, math) = equation_of(&app, cx, id);
    assert_eq!(text, "x=α^2/b+√⬚");
    assert_eq!(math, Equation::parse("x=α^2/b+√⬚"));
    assert!(app.update(cx, |app, _| app.ext::<equation::Typing>().and_then(|typing| typing.preview()).is_none()));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Typing"));
    // Typed over again.
    app.update_in(cx, |app, window, cx| begin_text_edit(app, id, TextStart::All, window, cx));
    cx.simulate_input("a^2+b^2=c^2");
    cx.simulate_keystrokes("escape");
    assert_eq!(equation_of(&app, cx, id).1, Equation::parse(BUILT_IN[4].1));
    undo(&app, cx);
    assert_eq!(equation_of(&app, cx, id).0, "x=α^2/b+√⬚");
    undo(&app, cx);
    undo(&app, cx);
    assert!(ids(&app, cx).is_empty());
}
