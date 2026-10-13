//! Warped text, gradients and reflections driven in GPUI's headless
//! window, on the real application. What these tests cannot tell is how
//! things look.

use super::*;
use crate::app::{MenuEntry, PopupKind};
use gpui::{Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point};
use opensheet_engine::style::Color;
use opensheet_engine::textwarp::{TextWarp, WarpPreset};
use opensheet_engine::wordart::{GradientKind, TextReflection, WordArtStyle};

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

fn shape(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Shape {
    app.update(cx, |app, _| app.wb.drawing(app.sheet, id).and_then(Drawing::as_shape).expect("the shape").clone())
}

fn size(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (f32, f32) {
    app.update(cx, |app, _| app.wb.drawing(app.sheet, id).expect("the drawing").size)
}

fn undo(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

/// A WordArt on the sheet, selected, its text typed.
fn wordart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> u32 {
    app.update_in(cx, |app, window, cx| wordart::insert(app, WordArtStyle(1), window, cx));
    redraw(cx);
    cx.simulate_input("Warped");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    app.update(cx, |app, _| selected(app).expect("the WordArt is selected").id)
}

/// Run the entry of the open menu found by following `path` through its
/// submenus, as a click on it does.
fn choose(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, path: &[&str]) {
    app.update_in(cx, |app, window, cx| {
        let popup = app.popup.take().expect("a menu is open");
        let PopupKind::Menu(entries) = popup.kind else {
            panic!("not a menu");
        };
        let mut level: &[MenuEntry] = &entries;
        let mut found = None;
        for name in path {
            let entry = level.iter().find(|entry| entry.label.as_ref() == *name).unwrap_or_else(|| panic!("no entry {name}"));
            level = entry.submenu.as_deref().unwrap_or(&[]);
            found = Some(entry);
        }
        let action = found.and_then(|entry| entry.action.clone()).expect("an entry that does something");
        action(app, window, cx);
    });
    redraw(cx);
}

fn warp_grip(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, handle: u8) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid was drawn");
        let pane = *paint::panes(&layout).last().expect("a pane");
        let drawing = selected(app).expect("a selected drawing");
        let placed = paint::placed(&layout, &app.wb.sheets[app.sheet], drawing, &pane);
        let (_, at) = paint::warp_grips(drawing, &placed, layout.zoom).into_iter().find(|(grip, _)| *grip == Grip::Warp(handle)).expect("the handle");
        layout.to_window(at.0, at.1)
    })
}

#[gpui::test]
fn the_transform_gallery_warps_the_selected_text(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = wordart(&app, cx);
    let fitted = size(&app, cx, id);
    // Text Effects ▸ Transform opens the gallery under the button.
    run(&app, cx, Cmd::TextEffects);
    choose(&app, cx, &["Transform"]);
    let gallery = app.update(cx, |app, _| {
        let popup = app.popup.as_ref().expect("the gallery");
        assert!(matches!(popup.kind, PopupKind::View(_)));
        popup.pos
    });
    // A click on the first sample under "Follow Path": Arch Up.
    let sample = point(gallery.x + px(8. + 36.), gallery.y + px(8. + 22. + 52. + 22. + 26.));
    cx.simulate_click(sample, Modifiers::none());
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_none()));
    assert_eq!(shape(&app, cx, id).warp, Some(TextWarp::from(WarpPreset::ArchUp)));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Transform"));
    // The box keeps its size: the text now fills it.
    assert_eq!(size(&app, cx, id), fitted);
    // The gallery shows which warp the text has.
    app.update_in(cx, |app, window, cx| wordart::open_transform(app, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.take().is_some()));
    // Another warp replaces it; the same one again keeps its adjustments.
    app.update(cx, |app, cx| {
        wordart::set_warp(app, Some(WarpPreset::Triangle), cx);
        let sheet = app.sheet;
        app.wb.update_drawing(sheet, id, |drawing| drawing.as_shape_mut().unwrap().warp.as_mut().unwrap().adjust[0] = Some(20_000));
        app.commit("setup", cx);
        wordart::set_warp(app, Some(WarpPreset::Triangle), cx);
    });
    redraw(cx);
    assert_eq!(shape(&app, cx, id).warp.map(|warp| warp.adjust), Some([Some(20_000), None]));
    // Typing in a warped shape keeps its box too.
    cx.simulate_input("!");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(shape(&app, cx, id).text, "Warped!");
    assert_eq!(size(&app, cx, id), fitted);
    // No Transform: the text is level and the box fits it again.
    app.update(cx, |app, cx| {
        wordart::set_warp(app, None, cx);
    });
    redraw(cx);
    assert_eq!(shape(&app, cx, id).warp, None);
    assert!(size(&app, cx, id).0 > fitted.0);
    undo(&app, cx);
    assert_eq!(shape(&app, cx, id).warp.map(|warp| warp.preset), Some(WarpPreset::Triangle));
    // A protected sheet keeps the text of its locked shapes as it is.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(sheet, opensheet_engine::protect::SheetProtection::default(), "");
        app.commit("setup", cx);
        assert!(!wordart::set_warp(app, Some(WarpPreset::Wave1), cx));
    });
    assert_eq!(shape(&app, cx, id).warp.map(|warp| warp.preset), Some(WarpPreset::Triangle));
}

#[gpui::test]
fn the_yellow_handle_adjusts_the_warp(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = wordart(&app, cx);
    app.update(cx, |app, cx| {
        wordart::set_warp(app, Some(WarpPreset::Triangle), cx);
    });
    redraw(cx);
    // The handle of a triangle is on its left side, half way down; a
    // level text has none.
    let from = warp_grip(&app, cx, 0);
    let to = point(from.x, from.y - px(15.));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(point(from.x, from.y - px(7.)), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    let warp = shape(&app, cx, id).warp.expect("still warped");
    let adjusted = warp.adjust[0].expect("the adjustment changed");
    assert!(adjusted < 50_000 && adjusted > 0, "{adjusted}");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Transform"));
    // The handle is where the pointer left it.
    let now = warp_grip(&app, cx, 0);
    assert!((f32::from(now.y) - f32::from(to.y)).abs() < 1.5, "{now:?} {to:?}");
    // One step back.
    undo(&app, cx);
    assert_eq!(shape(&app, cx, id).warp, Some(TextWarp::from(WarpPreset::Triangle)));
    // Turned with its shape, the handle still follows the pointer.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.update_drawing(sheet, id, |drawing| drawing.rotation = 90.0);
        app.commit("setup", cx);
    });
    redraw(cx);
    let from = warp_grip(&app, cx, 0);
    let to = point(from.x + px(12.), from.y);
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(point(from.x + px(6.), from.y), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    let turned = shape(&app, cx, id).warp.and_then(|warp| warp.adjust[0]).expect("the adjustment changed");
    assert!(turned < 50_000, "{turned}");
}

#[gpui::test]
fn letters_take_a_gradient_and_a_reflection(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = wordart(&app, cx);
    let color = shape(&app, cx, id).font.color.expect("a style colours its letters");
    // Text Fill has a Gradient row; its menu lists the variations of the
    // colour of the text.
    run(&app, cx, Cmd::TextFill);
    assert!(app.update(cx, |app, _| app.popup.take().is_some()));
    app.update_in(cx, |app, window, cx| wordart::open_gradient(app, window, cx));
    choose(&app, cx, &["Light Variations", "Linear Down"]);
    let gradient = shape(&app, cx, id).effects.gradient.expect("a gradient");
    assert_eq!(gradient.kind, GradientKind::Linear(90.0));
    assert_eq!(gradient.stops().len(), 2);
    assert_eq!(gradient.stops()[1].color, color.plain());
    assert_ne!(gradient.stops()[0].color, color.plain());
    assert_eq!(shape(&app, cx, id).font.color, Some(color));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text Gradient"));
    // The menu ticks it; a dark one from the centre replaces it.
    app.update_in(cx, |app, window, cx| wordart::open_gradient(app, window, cx));
    let ticked = app.update(cx, |app, _| match &app.popup.as_ref().expect("the menu").kind {
        PopupKind::Menu(entries) => entries[2].submenu.as_ref().map(|light| light.iter().filter(|entry| entry.checked).map(|entry| entry.label.to_string()).collect::<Vec<_>>()),
        _ => None,
    });
    assert_eq!(ticked, Some(vec!["Linear Down".to_string()]));
    choose(&app, cx, &["Dark Variations", "From Center"]);
    let gradient = shape(&app, cx, id).effects.gradient.expect("a gradient");
    assert_eq!((gradient.kind, gradient.stops()[0].color), (GradientKind::FromCentre, color.plain()));
    // A reflection from Text Effects.
    run(&app, cx, Cmd::TextEffects);
    choose(&app, cx, &["Reflection", "Half Reflection: 4 point offset"]);
    assert_eq!(shape(&app, cx, id).effects.reflection, Some(TextReflection::PRESETS[4]));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text Reflection"));
    // All of it is drawn (as outlines) without anything going wrong.
    redraw(cx);
    assert!(wordart::needs_outlines(&shape(&app, cx, id)));
    run(&app, cx, Cmd::TextEffects);
    choose(&app, cx, &["Reflection", "No Reflection"]);
    assert_eq!(shape(&app, cx, id).effects.reflection, None);
    // One colour again takes the gradient away; so does No Gradient.
    app.update(cx, |app, cx| wordart::set_fill(app, Some(Color(0xC00000)), cx));
    assert_eq!(shape(&app, cx, id).effects.gradient, None);
    undo(&app, cx);
    assert!(shape(&app, cx, id).effects.gradient.is_some());
    app.update_in(cx, |app, window, cx| wordart::open_gradient(app, window, cx));
    choose(&app, cx, &["No Gradient"]);
    assert_eq!(shape(&app, cx, id).effects.gradient, None);
    assert!(!wordart::needs_outlines(&shape(&app, cx, id)));
}

/// Twelve warps, a gradient with a reflection and a hollow outlined
/// text on one sheet, as a page prints them.
#[test]
fn warped_text_prints_as_the_outlines_the_screen_shows() {
    use crate::features::page::PaperPainter;
    use opensheet_engine::wordart::{GradientStop, TextGradient, TextOutline};
    use opensheet_print::{Op, PrintJob, PrintWhat, plan, print_pdf, render_page};
    let mut wb = opensheet_engine::Workbook::new(opensheet_engine::Locale::en());
    let presets = [
        WarpPreset::ArchUp,
        WarpPreset::Circle,
        WarpPreset::Triangle,
        WarpPreset::ChevronInverted,
        WarpPreset::CanUp,
        WarpPreset::Wave1,
        WarpPreset::Inflate,
        WarpPreset::Deflate,
        WarpPreset::DeflateInflateDeflate,
        WarpPreset::FadeRight,
        WarpPreset::SlantUp,
        WarpPreset::CascadeDown,
    ];
    let place = |wb: &mut opensheet_engine::Workbook, shape: Shape, i: usize| {
        let mut drawing = Drawing::shape(shape, (200.0, 100.0));
        drawing.offset = (20.0 + 220.0 * (i % 3) as f32, 20.0 + 125.0 * (i / 3) as f32);
        wb.add_drawing(0, drawing).unwrap();
    };
    for (i, preset) in presets.into_iter().enumerate() {
        let mut shape = WordArtStyle((i % 4) as u8).shape(preset.name());
        shape.font.set_name("Arial");
        shape.font.set_size_pt(24.0);
        if preset == WarpPreset::DeflateInflateDeflate {
            shape.text = "Deflate\nInflate\nDeflate".to_string();
        }
        shape.warp = Some(TextWarp::from(preset));
        place(&mut wb, shape, i);
    }
    let mut shine = WordArtStyle(1).shape("Gradient");
    shine.font.set_name("Arial");
    shine.font.set_size_pt(32.0);
    shine.effects.shadow = None;
    let stops = [(0.0, 0xFFC000), (0.5, 0xED7D31), (1.0, 0xC00000)].map(|(at, rgb)| GradientStop { at, color: Color(rgb) });
    shine.effects.gradient = TextGradient::new(GradientKind::Linear(90.0), &stops);
    shine.effects.reflection = Some(TextReflection::PRESETS[1]);
    place(&mut wb, shine, 12);
    let mut hollow = WordArtStyle(0).shape("Hollow");
    hollow.font.set_name("Arial");
    hollow.font.set_size_pt(32.0);
    hollow.effects.hollow = true;
    hollow.effects.outline = Some(TextOutline {
        color: Color(0x0070C0),
        width: 1.0,
    });
    place(&mut wb, hollow, 13);
    let mut sizes = ShapeTool::TextBox.shape();
    sizes.font.set_name("Arial");
    sizes.text = "Small and LARGE".to_string();
    sizes.format_text(10..15, &|font, _| font.set_size_pt(28.0));
    place(&mut wb, sizes, 14);
    wb.commit("setup");

    let painter = PaperPainter::headless();
    let mut job = PrintJob::new(PrintWhat::Sheet(0));
    job.painter = Some(&painter);
    let planned = plan(&wb, &job);
    let page = render_page(&wb, &job, &planned, 0).expect("a page");
    let texts: Vec<&str> = page
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let fills = page.ops.iter().filter(|op| matches!(op, Op::Fill { .. })).count();
    if texts.is_empty() {
        // The fonts of this machine gave their outlines: nothing is
        // printed as text; the gradient makes many fills by itself, and
        // the hollow text is stroked in its colour.
        assert!(fills > 12 + 32, "{fills} fills");
        assert!(page.ops.iter().any(|op| matches!(op, Op::Stroke { stroke, .. } if stroke.paint.color == Color(0x0070C0))));
        assert!(page.ops.iter().any(|op| matches!(op, Op::Fill { paint, .. } if paint.opacity < 0.5 && paint.color != Color(0x000000))));
    }
    let pdf = print_pdf(&wb, &job);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    if let Some(dir) = std::env::var_os("OPENSHEET_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join("opensheet-warp-print.pdf"), &pdf).expect("the sample is written");
        opensheet_io::save(&wb, 0, &std::path::Path::new(&dir).join("opensheet-warp-print.xlsx")).expect("the workbook is written");
    }
}
