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
