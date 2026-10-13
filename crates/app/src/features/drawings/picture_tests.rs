//! Picture styles, borders, the effects made in pixels and what a page
//! shows of them, on the real application in GPUI's headless window and
//! on bitmaps. The samples these tests keep (`OPENSHEET_SAMPLES`) are
//! what tells how things look.

use super::picture::Bitmap;
use super::*;
use crate::features::page::PaperPainter;
use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::Color;
use opensheet_engine::drawing::Outline;
use opensheet_engine::picture_style::{BevelPreset, Camera, PictureStyle, Reflection};
use opensheet_print::{FontDb, Op, PrintJob, PrintWhat, plan, print_pdf, render_page};

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// A photograph on the sheet, selected.
fn add_photo(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> u32 {
    let id = app.update(cx, |app, cx| {
        let id = app.wb.add_picture(app.sheet, styles::sample_photo_png(240, 160), (2, 1)).expect("a picture");
        app.commit("setup", cx);
        app.select_object(Some(object_id(&Drawings, id)), cx);
        id
    });
    redraw(cx);
    id
}

fn drawing(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Drawing {
    app.update(cx, |app, _| app.wb.drawing(app.sheet, id).expect("the drawing").clone())
}

/// Where the samples go, when they are asked for.
fn samples() -> Option<std::path::PathBuf> {
    std::env::var_os("OPENSHEET_SAMPLES").map(std::path::PathBuf::from)
}

/// Bitmaps side by side on white, as a PNG file: what a test shows.
pub(super) fn contact_sheet(tiles: &[Bitmap], columns: usize, cell: (u32, u32)) -> image::RgbaImage {
    let rows = tiles.len().div_ceil(columns) as u32;
    let mut sheet = image::RgbaImage::from_pixel(columns as u32 * cell.0, rows * cell.1, image::Rgba([255, 255, 255, 255]));
    for (i, tile) in tiles.iter().enumerate() {
        let (x0, y0) = ((i % columns) as u32 * cell.0, (i / columns) as u32 * cell.1);
        let (dx, dy) = (cell.0.saturating_sub(tile.width) / 2, cell.1.saturating_sub(tile.height) / 2);
        for y in 0..tile.height.min(cell.1) {
            for x in 0..tile.width.min(cell.0) {
                let at = tile.offset(x, y);
                let a = tile.data[at + 3] as u32;
                let over = |v: u8| ((v as u32 * a + 255 * (255 - a)) / 255) as u8;
                sheet.put_pixel(x0 + dx + x, y0 + dy + y, image::Rgba([over(tile.data[at + 2]), over(tile.data[at + 1]), over(tile.data[at]), 255]));
            }
        }
    }
    sheet
}

#[gpui::test]
fn a_style_is_one_undo_step(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add_photo(&app, cx);
    assert!(app.update(cx, |app, _| Cmd::PictureStyles.enabled(app) && Cmd::PictureBorder.enabled(app)));
    // The gallery opens under its button, with every style.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::PictureStyles, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()), "the gallery");
    app.update(cx, |app, cx| {
        app.popup = None;
        styles::apply_style(app, PictureStyle::BeveledOvalBlack, cx);
    });
    redraw(cx);
    let styled = drawing(&app, cx, id);
    let look = PictureStyle::BeveledOvalBlack.look();
    assert_eq!(styled.as_picture().unwrap().adjust.shape, ShapePreset::Ellipse);
    assert_eq!(styled.as_picture().unwrap().outline, look.outline);
    assert_eq!(styled.effects, look.effects);
    assert_eq!(styles::style_of(&styled), Some(PictureStyle::BeveledOvalBlack));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Picture Style"));
    // Another style replaces all of the first.
    app.update(cx, |app, cx| styles::apply_style(app, PictureStyle::DropShadowRectangle, cx));
    let second = drawing(&app, cx, id);
    assert_eq!((second.as_picture().unwrap().outline, second.effects.bevel), (None, None));
    assert_eq!(second.as_picture().unwrap().adjust.shape, ShapePreset::Rect);
    app.update(cx, |app, cx| {
        app.undo(cx);
        app.undo(cx);
    });
    let plain = drawing(&app, cx, id);
    assert!(plain.effects.is_none() && plain.as_picture().unwrap().outline.is_none());
    assert_eq!(styles::style_of(&plain), None);
    app.update(cx, |app, cx| app.redo(cx));
    assert_eq!(styles::style_of(&drawing(&app, cx, id)), Some(PictureStyle::BeveledOvalBlack));
    // The window draws it without trouble, turned too.
    app.update(cx, |app, cx| {
        app.wb.update_drawing(app.sheet, id, |d| d.rotation = 30.0);
        app.commit("setup", cx);
    });
    redraw(cx);
    // Reset Picture takes the style away.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::ResetPictureOnly, window, cx));
    let reset = drawing(&app, cx, id);
    assert!(reset.effects.is_none() && reset.as_picture().unwrap().outline.is_none());
}

#[gpui::test]
fn border_and_effects_of_a_picture(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = add_photo(&app, cx);
    // Picture Border: its drop-down, a colour, a weight, none.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::PictureBorder, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()), "the border drop-down");
    app.update(cx, |app, cx| {
        app.popup = None;
        styles::set_border(app, cx, |outline| *outline = Some(Outline::solid(Color(0xC00000), 1.0)));
        styles::set_border_width(app, 4.5, cx);
    });
    assert_eq!(drawing(&app, cx, id).as_picture().unwrap().outline, Some(Outline::solid(Color(0xC00000), 4.5)));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Picture Border"));
    // The pane's numbers read and write the same things.
    app.update(cx, |app, cx| {
        assert_eq!(pane::Look::BorderWidth.of(selected(app).unwrap()), Some(4.5));
        pane::Look::ReflectionSize.set(app, 55.0, cx);
        pane::Look::ReflectionDistance.set(app, 4.0, cx);
        pane::Look::BevelWidth.set(app, 9.0, cx);
        pane::Look::BevelHeight.set(app, 3.0, cx);
        pane::Look::RotationX.set(app, 20.0, cx);
        pane::Look::Perspective.set(app, 45.0, cx);
    });
    let dressed = drawing(&app, cx, id);
    assert_eq!(dressed.effects.reflection, Some(Reflection::presets()[4]));
    assert_eq!(dressed.effects.bevel.map(|b| (b.preset, b.width, b.height)), Some((BevelPreset::Circle, 9.0, 3.0)));
    assert_eq!(dressed.effects.rotation, Some(Camera::PerspectiveLeft.rotation()));
    redraw(cx);
    // Turned back flat, seen in parallel: no rotation at all.
    app.update(cx, |app, cx| {
        pane::Look::RotationX.set(app, 0.0, cx);
        pane::Look::Perspective.set(app, 0.0, cx);
        pane::Look::BevelWidth.set(app, 0.0, cx);
        pane::Look::BorderWidth.set(app, 0.0, cx);
    });
    let flat = drawing(&app, cx, id);
    assert_eq!((flat.effects.rotation, flat.effects.bevel, flat.as_picture().unwrap().outline), (None, None, None));
    // The lists of Picture Effects: none, the presets, the options.
    app.update(cx, |app, _| {
        assert_eq!(styles::reflection_menu(app).len(), 1 + 1 + 9 + 2);
        assert_eq!(styles::bevel_menu(app).len(), 1 + 1 + 12 + 2);
        assert_eq!(styles::rotation_menu(app).len(), 1 + 2 * 2 + 21 + 2);
    });
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::PictureEffects, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()), "the Picture Effects menu");
    // A protected sheet refuses.
    app.update(cx, |app, cx| {
        app.popup = None;
        app.wb.sheets[0].protection = Some(opensheet_engine::protect::SheetProtection::default());
        styles::apply_style(app, PictureStyle::MetalOval, cx);
    });
    assert_eq!(styles::style_of(&drawing(&app, cx, id)), None);
}

/// Every style of the gallery on a photograph: in a workbook for Excel,
/// on a page, and side by side as the screen composes them.
#[test]
fn every_style_draws_and_prints() {
    let photo = styles::sample_photo(360, 240);
    let png = picture::png_of(&photo).unwrap();
    let mut wb = opensheet_engine::Workbook::new(opensheet_engine::Locale::en());
    let mut tiles = Vec::new();
    for (i, style) in PictureStyle::ALL.into_iter().enumerate() {
        let (row, col) = (1 + 14 * (i as u32 / 4), 1 + 4 * (i as u32 % 4));
        wb.set_cell_text(0, row - 1, col, style.name(opensheet_engine::Locale::en())).unwrap();
        let id = wb.add_picture(0, png.clone(), (row, col)).expect("a picture");
        let look = style.look();
        wb.update_drawing(0, id, |d| {
            d.size = (180.0, 120.0);
            d.name = style.name(opensheet_engine::Locale::en()).to_string();
            d.effects = look.effects;
            let picture = d.as_picture_mut().unwrap();
            picture.adjust.shape = look.shape;
            picture.outline = look.outline;
        });
        // As the screen makes it at 100 %.
        let drawing = wb.drawing(0, id).unwrap();
        let picture = drawing.as_picture().unwrap();
        let dress = dress_for_page(drawing, picture);
        let soft = (crate::grid::pt_to_px(look.effects.soft_edge) / 120.0).min(0.5);
        let body = photo.halved().adjusted(&picture.adjust, soft, (180.0, 120.0));
        let (bitmap, share) = body.dressed(look.shape, (180.0, 120.0), (false, false), &dress);
        // The frame is in the middle of what was made, and something of
        // the picture is still there.
        assert!(share.0 <= 1.0 && share.1 <= 1.0 && share.0 > 0.2, "{style:?} {share:?}");
        let middle = bitmap.offset(bitmap.width / 2, bitmap.height / 2);
        assert!(bitmap.data[middle + 3] > 200, "{style:?}");
        // Whatever reaches beyond the frame makes the bitmap larger.
        let plain = dress.is_plain();
        assert_eq!(plain, bitmap.width == 180 && bitmap.height == 120, "{style:?}");
        tiles.push(bitmap);
    }
    wb.commit("styles");

    // On a page: one image for each picture, larger than its frame when
    // it wears something.
    let fonts = FontDb::standard_only();
    let painter = PaperPainter::headless();
    let mut job = PrintJob::new(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    job.painter = Some(&painter);
    let planned = plan(&wb, &job);
    let mut images = 0;
    for page in 0..planned.pages.len() {
        let scene = render_page(&wb, &job, &planned, page).unwrap();
        images += scene.ops.iter().filter(|op| matches!(op, Op::Image { .. })).count();
    }
    assert!(images >= PictureStyle::ALL.len(), "{images} images on {} pages", planned.pages.len());
    let pdf = print_pdf(&wb, &job);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    if let Some(directory) = samples() {
        std::fs::write(directory.join("opensheet-picture-styles.pdf"), &pdf).unwrap();
        opensheet_io::save_xlsx(&wb, &directory.join("opensheet-picture-styles.xlsx")).unwrap();
        contact_sheet(&tiles, 6, (330, 300)).save(directory.join("opensheet-picture-styles.png")).unwrap();
    }
    // The workbook says the same once it is read back.
    let bytes = opensheet_io::write_xlsx(&wb, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
    let back = opensheet_io::read_xlsx(std::io::Cursor::new(bytes), opensheet_engine::Locale::en()).unwrap();
    for (style, drawing) in PictureStyle::ALL.into_iter().zip(&back.sheets[0].drawings) {
        assert_eq!(styles::style_of(drawing), Some(style));
    }
}

/// The gallery shows each style on the picture itself.
#[test]
fn the_gallery_samples_the_picture() {
    let picture = opensheet_engine::drawing::Picture::new(styles::sample_photo_png(300, 200)).unwrap();
    let mut seen = std::collections::HashSet::new();
    for style in PictureStyle::ALL {
        let (bitmap, share) = styles::style_sample(&picture, (300.0, 200.0), style).unwrap_or_else(|| panic!("{style:?}"));
        assert!(bitmap.width >= 64 && bitmap.width < 900, "{style:?}: {}", bitmap.width);
        assert!(share.0 > 0.2);
        seen.insert(bitmap.data);
    }
    // No two styles look the same.
    assert_eq!(seen.len(), PictureStyle::ALL.len());
}

// ------------------------------------------------------- artistic effects

#[gpui::test]
fn an_artistic_effect_is_made_off_the_window_thread(cx: &mut TestAppContext) {
    use opensheet_engine::picture_style::{Artistic, ArtisticKind};
    let (app, cx) = open(cx);
    let id = add_photo(&app, cx);
    assert!(app.update(cx, |app, _| Cmd::ArtisticEffects.enabled(app)));
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::ArtisticEffects, window, cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()), "the gallery");
    app.update(cx, |app, cx| {
        app.popup = None;
        artistic::set_artistic(app, Some(Artistic::preset(ArtisticKind::Cutout)), cx);
    });
    // Chosen: the picture has the effect, its second image is to come.
    let chosen = drawing(&app, cx, id);
    assert_eq!(chosen.as_picture().unwrap().artistic, Some(Artistic::preset(ArtisticKind::Cutout)));
    assert!(chosen.as_picture().unwrap().needs_baking());
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Artistic Effects"));
    // The window is drawn, the image is made in the background, and the
    // picture gets it without a step in the undo history.
    redraw(cx);
    redraw(cx);
    let made = drawing(&app, cx, id);
    let picture = made.as_picture().unwrap();
    assert!(!picture.needs_baking(), "the second image arrived");
    let baked = picture.baked().unwrap().data.clone();
    assert!(baked.starts_with(b"\x89PNG") && baked != picture.data);
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Artistic Effects"));
    // The pane's two numbers.
    app.update(cx, |app, cx| {
        assert_eq!(pane::Look::ArtisticAmount.of(selected(app).unwrap()), Some(1.0));
        pane::Look::ArtisticAmount.set(app, 40.0, cx);
        pane::Look::ArtisticTransparency.set(app, 25.0, cx);
    });
    let tuned = drawing(&app, cx, id).as_picture().unwrap().artistic.unwrap();
    // A cutout has at most six shades.
    assert_eq!((tuned.amount, tuned.transparency), (6.0, 0.25));
    redraw(cx);
    redraw(cx);
    assert!(!drawing(&app, cx, id).as_picture().unwrap().needs_baking());
    // Undone twice the first image is wanted again: it was kept.
    app.update(cx, |app, cx| {
        app.undo(cx);
        app.undo(cx);
    });
    redraw(cx);
    let again = drawing(&app, cx, id);
    assert_eq!(again.as_picture().unwrap().baked().map(|b| b.data.clone()), Some(baked));
    // Saturation is kept in the second image too.
    app.update(cx, |app, cx| {
        artistic::set_artistic(app, None, cx);
        pane::Look::Saturation.set(app, 200.0, cx);
    });
    redraw(cx);
    redraw(cx);
    let vivid = drawing(&app, cx, id);
    let picture = vivid.as_picture().unwrap();
    assert!(picture.artistic.is_none() && picture.baked().is_some());
    assert_eq!(picture.shown().1.saturation, 1.0);
    // Reset Picture: the image as it was.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::ResetPictureOnly, window, cx));
    let reset = drawing(&app, cx, id);
    assert!(!reset.as_picture().unwrap().has_baked_effects() && reset.as_picture().unwrap().baked.is_none());
}

/// Every artistic effect on a photograph: in a workbook for Excel (the
/// filtered image in the blip, the photograph beside it) and side by
/// side for the eye.
#[test]
fn every_artistic_effect_is_written_with_its_image() {
    use opensheet_engine::picture_style::{Artistic, ArtisticKind};
    let photo = styles::sample_photo(480, 320);
    let png = picture::png_of(&photo).unwrap();
    let mut wb = opensheet_engine::Workbook::new(opensheet_engine::Locale::en());
    let mut tiles = vec![photo.clone()];
    for (i, kind) in ArtisticKind::ALL.into_iter().enumerate() {
        let (row, col) = (1 + 12 * (i as u32 / 4), 1 + 4 * (i as u32 % 4));
        wb.set_cell_text(0, row - 1, col, kind.name(opensheet_engine::Locale::en())).unwrap();
        let id = wb.add_picture(0, png.clone(), (row, col)).expect("a picture");
        wb.update_drawing(0, id, |d| {
            d.size = (240.0, 160.0);
            d.name = kind.name(opensheet_engine::Locale::en()).to_string();
            d.as_picture_mut().unwrap().artistic = Some(Artistic::preset(kind));
        });
        tiles.push(filters::apply(&photo, &Artistic::preset(kind)));
    }
    // One with its colours changed only, and one with both.
    let warm = wb.add_picture(0, png.clone(), (49, 1)).unwrap();
    wb.update_drawing(0, warm, |d| {
        d.size = (240.0, 160.0);
        let picture = d.as_picture_mut().unwrap();
        picture.adjust.saturation = 2.0;
        picture.adjust.temperature = 8800.0;
    });
    wb.commit("effects");
    before_save(&mut wb);
    let pictures: Vec<&opensheet_engine::drawing::Picture> = wb.sheets[0].drawings.iter().filter_map(Drawing::as_picture).collect();
    assert!(pictures.iter().all(|picture| !picture.needs_baking()));
    let bytes = opensheet_io::write_xlsx(&wb, std::io::Cursor::new(Vec::new())).unwrap().into_inner();
    if let Some(directory) = samples() {
        std::fs::write(directory.join("opensheet-artistic-effects.xlsx"), &bytes).unwrap();
        contact_sheet(&tiles, 4, (500, 340)).save(directory.join("opensheet-artistic-effects.png")).unwrap();
    }
    // Read back: the photograph, the effect, and the image made of it.
    let back = opensheet_io::read_xlsx(std::io::Cursor::new(bytes), opensheet_engine::Locale::en()).unwrap();
    for (kind, drawing) in ArtisticKind::ALL.into_iter().zip(&back.sheets[0].drawings) {
        let picture = drawing.as_picture().unwrap();
        assert_eq!(picture.artistic, Some(Artistic::preset(kind)));
        assert_eq!(&picture.data[..], &png[..], "{kind:?}");
        let baked = picture.baked().unwrap_or_else(|| panic!("{kind:?}: no second image"));
        let shown = Bitmap::decode(&baked.data).unwrap();
        assert_eq!(shown, filters::apply(&photo, &Artistic::preset(kind)), "{kind:?}");
    }
    let last = back.sheets[0].drawings.last().and_then(Drawing::as_picture).unwrap();
    assert!((last.adjust.saturation - 2.0).abs() < 1e-3 && last.baked().is_some());
}

/// How long the effects take on a photograph of twelve million pixels:
/// run with `OPENSHEET_TIMING=1` to see (they are made off the window's
/// thread whatever the answer).
#[test]
fn effects_on_a_large_photograph() {
    use opensheet_engine::picture_style::{Artistic, ArtisticKind};
    let timed = std::env::var_os("OPENSHEET_TIMING").is_some();
    let photo = if timed { styles::sample_photo(4000, 3000) } else { styles::sample_photo(400, 300) };
    for kind in ArtisticKind::ALL {
        let start = std::time::Instant::now();
        let made = filters::apply(&photo, &Artistic::preset(kind));
        assert_eq!(made.data.len(), photo.data.len());
        if timed {
            eprintln!("{kind:?}: {:.2} s", start.elapsed().as_secs_f32());
        }
    }
}

// ---------------------------------------------------------------- printing

/// The colours, the crop, the outline and the effects of pictures and
/// the shadows of shapes come out on paper as the sheet shows them.
#[test]
fn adjustments_and_effects_are_printed() {
    use opensheet_engine::drawing::{Frame, Glow, PictureAdjust, Recolor, Shadow, ShadowPreset, ShapeTool};
    use opensheet_engine::ink::PenKind;
    use opensheet_print::Pixels;
    let png = styles::sample_photo_png(240, 160);
    let mut wb = opensheet_engine::Workbook::new(opensheet_engine::Locale::en());
    let plain = PictureAdjust::default();
    let mut place = 0u32;
    let mut add = |wb: &mut opensheet_engine::Workbook, name: &str, change: &dyn Fn(&mut Drawing)| {
        let (row, col) = (1 + 8 * (place / 3), 1 + 3 * (place % 3));
        place += 1;
        wb.set_cell_text(0, row - 1, col, name).unwrap();
        let id = wb.add_picture(0, png.clone(), (row, col)).unwrap();
        wb.update_drawing(0, id, |d| {
            d.size = (150.0, 100.0);
            d.name = name.to_string();
            change(d);
        });
        id
    };
    let adjust = |d: &mut Drawing, adjust: PictureAdjust| d.as_picture_mut().unwrap().adjust = adjust;
    add(&mut wb, "As inserted", &|_| {});
    add(&mut wb, "Grayscale", &|d| adjust(d, PictureAdjust { recolor: Recolor::Grayscale, ..plain }));
    add(&mut wb, "Sepia, brighter", &|d| adjust(d, PictureAdjust { recolor: Recolor::Duotone(Color(0), Color(0xD9C3A5)), brightness: 0.2, ..plain }));
    add(&mut wb, "Cropped, oval", &|d| adjust(d, PictureAdjust { crop: [0.3, 0.0, 0.0, 0.2], shape: ShapePreset::Ellipse, ..plain }));
    add(&mut wb, "Half transparent", &|d| adjust(d, PictureAdjust { transparency: 0.5, ..plain }));
    add(&mut wb, "Vivid, warm", &|d| adjust(d, PictureAdjust { saturation: 3.0, temperature: 9500.0, contrast: 0.3, ..plain }));
    add(&mut wb, "Turned, shadow", &|d| {
        d.rotation = 20.0;
        d.effects.shadow = Some(ShadowPreset::BottomRight.shadow());
    });
    add(&mut wb, "Flipped, soft", &|d| {
        d.flip_h = true;
        d.effects.soft_edge = 10.0;
    });
    add(&mut wb, "Glow and border", &|d| {
        d.effects.glow = Some(Glow { color: Color(0xE97132), transparency: 0.4, radius: 11.0 });
        d.as_picture_mut().unwrap().outline = Some(Outline::solid(Color(0x156082), 3.0));
    });
    // Shapes with a shadow and a glow, one of them turned.
    let star = wb.add_shape(0, ShapeTool::Star5, Frame::new(80.0, 520.0, 110.0, 100.0)).unwrap();
    wb.update_drawing(0, star, |d| {
        d.rotation = 15.0;
        d.effects.shadow = Some(Shadow { blur: 8.0, distance: 8.0, ..Shadow::default() });
    });
    let callout = wb.add_shape(0, ShapeTool::RoundedRectangle, Frame::new(260.0, 530.0, 140.0, 70.0)).unwrap();
    wb.update_drawing(0, callout, |d| d.effects.glow = Some(Glow { color: Color(0x4EA72E), transparency: 0.3, radius: 12.0 }));
    let line = wb.add_shape(0, ShapeTool::Arrow, Frame::new(450.0, 530.0, 110.0, 60.0)).unwrap();
    wb.update_drawing(0, line, |d| d.effects.shadow = Some(ShadowPreset::Bottom.shadow()));
    wb.draw_ink(0, PenKind::Pen, Color(0xC00000), 3.0, &[(90.0, 690.0), (140.0, 660.0), (190.0, 700.0), (260.0, 670.0)]);
    wb.draw_ink(0, PenKind::Highlighter, Color(0xFFFF00), 14.0, &[(300.0, 680.0), (480.0, 680.0)]);
    wb.commit("pictures");

    let fonts = FontDb::standard_only();
    let painter = PaperPainter::headless();
    let mut job = PrintJob::new(PrintWhat::Sheet(0));
    job.fonts = Some(&fonts);
    job.painter = Some(&painter);
    let planned = plan(&wb, &job);
    assert_eq!(planned.pages.len(), 1);
    // Every image of the page, in the order it is painted.
    let mut images = Vec::new();
    for page in 0..planned.pages.len() {
        for op in render_page(&wb, &job, &planned, page).unwrap().ops {
            if let Op::Image { image, rect } = op {
                images.push((image, rect));
            }
        }
    }
    let rgba = |index: usize| match &images[index].0.pixels {
        Pixels::Rgba(data) => data.clone(),
        other => panic!("image {index} is not made of pixels: {other:?}"),
    };
    // Nine pictures and the shadows of three shapes.
    assert_eq!(images.len(), 12);
    // Grey has no colour; sepia is warm; the plain one has blue sky.
    assert!(rgba(1).chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]));
    let sky = |data: &[u8]| (data[4 * 200] as i32, data[4 * 200 + 2] as i32);
    assert!(sky(&rgba(0)).1 > sky(&rgba(0)).0 + 40, "blue sky: {:?}", sky(&rgba(0)));
    assert!(sky(&rgba(2)).0 > sky(&rgba(2)).1, "sepia: {:?}", sky(&rgba(2)));
    // Cut to an ellipse: the corner is clear, the middle is not; and
    // only what the crop keeps of the image is there (seven tenths of
    // its width, eight of its height), stretched over the frame.
    let cropped = &images[3].0;
    let data = rgba(3);
    assert_eq!(data[3], 0);
    assert_eq!(data[((cropped.height / 2 * cropped.width + cropped.width / 2) * 4 + 3) as usize], 255);
    assert!((cropped.width as f32 / cropped.height as f32 - 168.0 / 128.0).abs() < 0.03);
    // Half transparent.
    assert!((126..=129).contains(&rgba(4)[3]));
    // The turned one is larger than its frame by its shadow, and is
    // painted about the middle of the frame.
    let (turned, rect) = &images[6];
    assert!(turned.width > 240 && rect.w > 150.0 && rect.x < 0.0, "{} {rect:?}", turned.width);
    // The shadows of the shapes are theirs alone: nothing opaque.
    for index in 9..12 {
        let data = rgba(index);
        let strongest = data.chunks_exact(4).map(|p| p[3]).max().unwrap();
        // (A thin line has little to blur: its shadow is faint.)
        let least = if index == 11 { 8 } else { 60 };
        assert!(strongest > least && strongest < 200, "shape {index}: {strongest}");
    }
    let pdf = print_pdf(&wb, &job);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    if let Some(directory) = samples() {
        std::fs::write(directory.join("opensheet-printed-pictures.pdf"), &pdf).unwrap();
        opensheet_io::save_xlsx(&wb, &directory.join("opensheet-printed-pictures.xlsx")).unwrap();
    }
}

/// On screen a shape's shadow is one bitmap, made once for each look.
#[gpui::test]
fn a_shape_draws_its_shadow_from_a_bitmap(cx: &mut TestAppContext) {
    use opensheet_engine::drawing::{Frame, ShadowPreset, ShapeTool};
    let (app, cx) = open(cx);
    let id = app.update(cx, |app, cx| {
        let id = app.wb.add_shape(app.sheet, ShapeTool::Ellipse, Frame::new(120.0, 80.0, 160.0, 90.0)).unwrap();
        app.wb.update_drawing(app.sheet, id, |d| d.effects.shadow = Some(ShadowPreset::BottomRight.shadow()));
        app.commit("setup", cx);
        id
    });
    redraw(cx);
    let items = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, turn: f32| {
        app.update_in(cx, |app, window, _| {
            let drawing = app.wb.drawing(app.sheet, id).unwrap().clone();
            let placed = geometry::Placed {
                rotation: turn,
                ..geometry::Placed::plain(100.0, 100.0, 260.0, 190.0)
            };
            let ts = window.text_system().clone();
            let brush = paint::Brush {
                origin: point(px(0.), px(0.)),
                zoom: 1.0,
                scale: 2.0,
                ts: &ts,
                clip: gpui::Bounds::new(point(px(0.), px(0.)), gpui::size(px(800.), px(600.))),
            };
            let mut out = Vec::new();
            paint::draw(&drawing, &placed, (160.0, 90.0), &brush, None, &mut out);
            out.into_iter()
                .filter_map(|item| match item {
                    crate::grid::Draw::Image { image, bounds } => Some((image, bounds)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    };
    let first = items(&app, cx, 0.0);
    assert_eq!(first.len(), 1, "one bitmap under the shape");
    // Wider than the shape by its blur and its offset, a little more to
    // the right and below (it is centred on the frame, with as much
    // room on the other side).
    let bounds = first[0].1;
    assert!(f32::from(bounds.size.width) > 170.0 && f32::from(bounds.origin.x) < 100.0);
    // Painted again, the same bitmap; turned, another one.
    let again = items(&app, cx, 0.0);
    assert!(std::sync::Arc::ptr_eq(&first[0].0, &again[0].0));
    let turned = items(&app, cx, 30.0);
    assert!(!std::sync::Arc::ptr_eq(&first[0].0, &turned[0].0));
}

// ---------------------------------------------------------------- compress

#[test]
fn compressing_resamples_and_deletes_what_is_cropped() {
    use compress::{Options, compressed};
    use opensheet_engine::drawing::Picture;
    let mut picture = Picture::new(styles::sample_photo_png(800, 600)).unwrap();
    let all = Options { ppi: Some(96.0), delete_cropped: true, selected_only: false };
    // Shown 200 by 150 pixels: at 96 to the inch that is all it needs.
    let (file, crop) = compressed(&picture, (200.0, 150.0), &all).expect("fewer pixels");
    let made = Bitmap::decode(&file).unwrap();
    assert_eq!((made.width, made.height), (200, 150));
    assert_eq!(crop, [0.0; 4]);
    assert!(file.len() < picture.data.len() / 4);
    // At 220 to the inch: 459 by 344.
    let fine = Options { ppi: Some(220.0), ..all };
    let made = Bitmap::decode(&compressed(&picture, (200.0, 150.0), &fine).unwrap().0).unwrap();
    assert_eq!((made.width, made.height), (459, 344));
    // Shown larger than its pixels allow, or kept as it is: nothing to do.
    assert!(compressed(&picture, (2000.0, 1500.0), &fine).is_none());
    assert!(compressed(&picture, (200.0, 150.0), &Options { ppi: None, ..all }).is_none());
    // Cropped: a quarter off the left, half off the bottom, and room
    // left empty on the right. The kept part is 600 by 300 pixels and
    // becomes the image; the empty room is the same on the sheet.
    picture.adjust.crop = [0.25, 0.0, -0.125, 0.5];
    let keep = Options { ppi: None, ..all };
    let (file, crop) = compressed(&picture, (350.0, 150.0), &keep).expect("the cropped parts go");
    let cut = Bitmap::decode(&file).unwrap();
    assert_eq!((cut.width, cut.height), (600, 300));
    assert_eq!(crop, [0.0, 0.0, -0.125 * 800.0 / 600.0, 0.0]);
    // The pixel that was at (200, 0) is now the first one.
    let whole = Bitmap::decode(&picture.data).unwrap();
    assert_eq!(&cut.data[..4], &whole.data[whole.offset(200, 0)..whole.offset(200, 0) + 4]);
    // The frame still shows the same thing: 350 pixels for 7/8 of the
    // old width is 300 for the image and 50 of room.
    let before = picture.adjust.uncropped(opensheet_engine::drawing::Frame::new(0.0, 0.0, 350.0, 150.0));
    picture.adjust.crop = crop;
    let after = picture.adjust.uncropped(opensheet_engine::drawing::Frame::new(0.0, 0.0, 350.0, 150.0));
    assert!((after.width - 300.0).abs() < 1e-3 && (before.width - 400.0).abs() < 1e-3, "{before:?} {after:?}");
    // Without "Delete cropped areas" the crop stays, and the image is
    // resampled whole.
    picture.adjust.crop = [0.25, 0.0, 0.0, 0.5];
    let (file, crop) = compressed(&picture, (150.0, 75.0), &Options { delete_cropped: false, ..all }).unwrap();
    let small = Bitmap::decode(&file).unwrap();
    assert_eq!((small.width, small.height, crop), (200, 150, [0.25, 0.0, 0.0, 0.5]));
}

#[gpui::test]
fn the_compress_dialog_is_one_undo_step(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = app.update(cx, |app, cx| {
        let id = app.wb.add_picture(app.sheet, styles::sample_photo_png(600, 400), (2, 1)).expect("a picture");
        app.wb.update_drawing(app.sheet, id, |d| {
            d.size = (150.0, 100.0);
            d.as_picture_mut().unwrap().adjust.crop = [0.0, 0.0, 0.5, 0.0];
        });
        // A second one, on another sheet.
        let other = app.wb.add_sheet(1);
        let far = app.wb.add_picture(other, styles::sample_photo_png(600, 400), (1, 1)).unwrap();
        app.wb.update_drawing(other, far, |d| d.size = (150.0, 100.0));
        app.commit("setup", cx);
        app.select_object(Some(object_id(&Drawings, id)), cx);
        id
    });
    redraw(cx);
    let weight = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, sheet: usize| {
        app.update(cx, |app, _| app.wb.sheets[sheet].drawings[0].as_picture().unwrap().data.len())
    };
    let (first, second) = (weight(&app, cx, 0), weight(&app, cx, 1));
    // The dialog: best for printing, delete cropped areas, the selected
    // picture only. Return accepts.
    app.update_in(cx, |app, window, cx| app.run_command(Cmd::Compress, window, cx));
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 0);
    let picture = drawing(&app, cx, id).as_picture().unwrap().clone();
    // Half the image is left. (220 pixels to the inch of its 150 would
    // be 344: more than the 300 it has, so they are all kept.)
    assert_eq!(picture.pixels, (300, 400));
    assert_eq!(picture.adjust.crop, [0.0; 4]);
    assert!(weight(&app, cx, 0) < first);
    assert_eq!(weight(&app, cx, 1), second, "the other sheet's picture was not selected");
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Compress Pictures"));
    redraw(cx);
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(drawing(&app, cx, id).as_picture().unwrap().pixels, (600, 400));
    // All pictures of the file, for the screen.
    app.update(cx, |app, cx| {
        compress::compress(app, compress::Options { ppi: Some(96.0), delete_cropped: false, selected_only: false }, cx);
    });
    assert!(weight(&app, cx, 0) < first && weight(&app, cx, 1) < second);
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[1].drawings[0].as_picture().unwrap().pixels), (150, 100));
    // One step for both.
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!((weight(&app, cx, 0), weight(&app, cx, 1)), (first, second));
}
