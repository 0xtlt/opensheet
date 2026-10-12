//! The background of a sheet (Page Layout ▸ Background): a picture
//! repeated behind the cells from the corner of A1. It is something of the
//! screen only: nothing here reaches the pages that are printed.

use super::super::drawings::geometry::Placed;
use super::super::drawings::picture;
use super::super::{Cx, Intent, PaintLayer, Painter, ViewMode};
use crate::app::ExcelApp;
use crate::grid::Draw;
use crate::i18n::t;
use crate::shell::Alert;
use gpui::{PathPromptOptions, Window};
use opensheet_engine::drawing::Picture;

/// No more tiles than this are painted: a picture of a few pixels would
/// otherwise be drawn tens of thousands of times in a large window.
const MAX_TILES: usize = 6000;

pub(super) fn has_background(app: &ExcelApp) -> bool {
    app.wb.sheets[app.sheet].background.is_some()
}

/// "Background…": the system file dialog, then the picture behind the
/// cells; "Delete Background" when the sheet has one.
pub(super) fn run(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    if has_background(app) {
        delete(app, cx);
        return;
    }
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(crate::i18n::t_ctx("button", "Insert", "Insérer").into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = paths.await else {
            return;
        };
        let Some(path) = paths.into_iter().next() else {
            return;
        };
        let bytes = std::fs::read(path).unwrap_or_default();
        this.update_in(cx, |app, _, cx| {
            apply(app, bytes, cx);
        })
        .ok();
    })
    .detach();
}

/// Give the current sheet the picture of an image file as its background.
/// Returns false, and says so, when the bytes are no picture.
pub(crate) fn apply(app: &mut ExcelApp, bytes: Vec<u8>, cx: &mut Cx) -> bool {
    if !app.may(Intent::ChangeSheet, cx) {
        return false;
    }
    let Some(background) = picture::importable(bytes).and_then(Picture::new) else {
        let message = t("An error occurred while importing this file.", "Une erreur s'est produite lors de l'importation de ce fichier.");
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
        return false;
    };
    app.wb.set_sheet_background(app.sheet, Some(background));
    app.commit(t("Background", "Arrière-plan"), cx);
    true
}

/// "Delete Background".
pub(super) fn delete(app: &mut ExcelApp, cx: &mut Cx) {
    if has_background(app) && app.may(Intent::ChangeSheet, cx) {
        app.wb.set_sheet_background(app.sheet, None);
        app.commit(t("Delete Background", "Supprimer l'arrière-plan"), cx);
    }
}

/// Where the tiles of a picture `tile` pixels wide and high go in a pane
/// that shows the box `clip` of a sheet whose corner is at `origin`: the
/// top-left corners of those that show, row by row.
pub(super) fn tiles(origin: (f64, f64), tile: (f32, f32), clip: (f32, f32, f32, f32)) -> Vec<(f32, f32)> {
    let (tw, th) = (tile.0 as f64, tile.1 as f64);
    if tw < 1.0 || th < 1.0 || clip.2 <= clip.0 || clip.3 <= clip.1 {
        return Vec::new();
    }
    let first = |start: f32, origin: f64, size: f64| ((start as f64 - origin) / size).floor().max(0.0);
    let (i0, j0) = (first(clip.0, origin.0, tw), first(clip.1, origin.1, th));
    let mut out = Vec::new();
    let mut j = j0;
    while origin.1 + j * th < clip.3 as f64 {
        let mut i = i0;
        while origin.0 + i * tw < clip.2 as f64 {
            if out.len() >= MAX_TILES {
                return out;
            }
            out.push(((origin.0 + i * tw) as f32, (origin.1 + j * th) as f32));
            i += 1.0;
        }
        j += 1.0;
    }
    out
}

/// Paint the background of the current sheet under the cells of a pane.
/// Cells with a fill are painted over it, the gridlines too.
pub(super) fn paint(p: &mut Painter) {
    if p.layer != PaintLayer::Under || p.app.view_mode() != ViewMode::Normal {
        return;
    }
    let Some(background) = &p.app.wb.sheets[p.app.sheet].background else {
        return;
    };
    let layout = p.layout;
    let zoom = layout.zoom;
    let tile = (background.pixels.0 as f32 * zoom, background.pixels.1 as f32 * zoom);
    // Where the corner of A1 would be in this pane, frozen or scrolled.
    let (row, col) = (p.visible.r0, p.visible.c0);
    let origin = (layout.col_x(col) as f64 - layout.cols.start(col), layout.row_y(row) as f64 - layout.rows.start(row));
    let Some((image, _)) = picture::image_for(background, &Placed::plain(0.0, 0.0, tile.0, tile.1), p.scale) else {
        return;
    };
    for (x, y) in tiles(origin, tile, p.clip) {
        p.push(Draw::Image {
            image: image.clone(),
            bounds: layout.bounds_px(x, y, x + tile.0, y + tile.1),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::Cmd;
    use super::*;
    use crate::features::Command;
    use crate::i18n::{Lang, set_lang};
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
    use opensheet_engine::Rect;

    fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
        set_lang(Lang::En);
        cx.update(crate::bind_keys);
        let (app, cx) = cx.add_window_view(ExcelApp::new);
        redraw(cx);
        (app, cx)
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
        cx.run_until_parked();
    }

    /// The bytes of a PNG file of one colour.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 220, 255, 255]));
        let mut file = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut file, image::ImageFormat::Png).expect("a PNG file");
        file.into_inner()
    }

    /// The tiles painted under the cells of a pane, as boxes in the window.
    fn painted(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, visible: Rect) -> Vec<(f32, f32, f32, f32)> {
        app.update_in(cx, |app, window, _| {
            let layout = app.current_layout().expect("the grid was drawn");
            let clip = (layout.header_w, layout.header_h, layout.width(), layout.height());
            let mut p = Painter::new(app, &layout, window.text_system(), PaintLayer::Under, visible, clip, 1.0);
            paint(&mut p);
            p.items
                .iter()
                .map(|item| match item {
                    Draw::Image { bounds, .. } => {
                        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        (x, y, x + f32::from(bounds.size.width), y + f32::from(bounds.size.height))
                    }
                    _ => panic!("the background paints pictures only"),
                })
                .collect()
        })
    }

    #[gpui::test]
    fn the_button_sets_the_background_then_deletes_it(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        let label = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| Cmd::Background.label_in(app));
        let background = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.wb.sheets[0].background.clone());
        let undo_label = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.wb.undo_label().map(str::to_string));
        assert_eq!(label(&app, cx), "Background");
        assert!(app.update(cx, |app, _| Cmd::Background.enabled(app)));
        assert!(painted(&app, cx, Rect::new(0, 0, 30, 20)).is_empty());

        // A file that is no picture is refused, and said to be.
        assert!(!app.update(cx, |app, cx| apply(app, b"not a picture".to_vec(), cx)));
        redraw(cx);
        assert!(app.update(cx, |app, _| app.modal_open()));
        app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
        assert_eq!(background(&app, cx), None);

        assert!(app.update(cx, |app, cx| apply(app, png(80, 60), cx)));
        redraw(cx);
        let picture = background(&app, cx).expect("the sheet has a background");
        assert_eq!(picture.pixels, (80, 60));
        assert_eq!(label(&app, cx), "Delete Background");
        assert_eq!(undo_label(&app, cx).as_deref(), Some("Background"));

        // Tiles from the corner of A1, side by side, enough to fill the pane.
        let tiles = painted(&app, cx, Rect::new(0, 0, 30, 20));
        let corner = app.update(cx, |app, _| {
            let layout = app.current_layout().unwrap();
            let at = layout.to_window(layout.col_x(0), layout.row_y(0));
            (f32::from(at.x), f32::from(at.y), layout.view_w(), layout.view_h())
        });
        assert_eq!(tiles[0], (corner.0, corner.1, corner.0 + 80.0, corner.1 + 60.0));
        assert_eq!(tiles[1], (corner.0 + 80.0, corner.1, corner.0 + 160.0, corner.1 + 60.0));
        let (across, down) = ((corner.2 / 80.0).ceil() as usize, (corner.3 / 60.0).ceil() as usize);
        assert_eq!(tiles.len(), across * down);

        // Zoomed, the tiles are; in the frozen and in the scrolled panes
        // they belong to one tiling.
        app.update(cx, |app, cx| {
            app.wb.set_frozen(0, 2, 1);
            app.commit("freeze", cx);
            app.set_zoom(1.5, cx);
        });
        redraw(cx);
        let frozen = painted(&app, cx, Rect::new(0, 0, 1, 0));
        assert_eq!(frozen[0].2 - frozen[0].0, 120.0);
        let scrolled = painted(&app, cx, Rect::new(2, 1, 40, 20));
        let step = |a: f32, b: f32, size: f32| ((a - b) / size - ((a - b) / size).round()).abs() < 0.01;
        assert!(step(scrolled[0].0, frozen[0].0, 120.0) && step(scrolled[0].1, frozen[0].1, 90.0), "{:?} {:?}", scrolled[0], frozen[0]);
        // Not in the Page Break Preview, which paints its own ground.
        app.update(cx, |app, cx| app.set_view_mode(ViewMode::PageBreakPreview, cx));
        redraw(cx);
        assert!(painted(&app, cx, Rect::new(0, 0, 30, 20)).is_empty());
        app.update(cx, |app, cx| app.set_view_mode(ViewMode::Normal, cx));

        // The button now takes the background away; both are undone.
        app.update_in(cx, |app, window, cx| app.run_command(Cmd::Background, window, cx));
        redraw(cx);
        assert_eq!(background(&app, cx), None);
        assert_eq!(label(&app, cx), "Background");
        assert_eq!(undo_label(&app, cx).as_deref(), Some("Delete Background"));
        app.update(cx, |app, cx| app.undo(cx));
        assert_eq!(background(&app, cx), Some(picture.clone()));

        // A protected sheet keeps its background as it is.
        app.update(cx, |app, cx| {
            app.wb.protect_sheet(0, Default::default(), "");
            app.commit("protect", cx);
        });
        app.update_in(cx, |app, window, cx| app.run_command(Cmd::Background, window, cx));
        redraw(cx);
        assert!(app.update(cx, |app, _| app.modal_open()), "the refusal is said");
        assert_eq!(background(&app, cx), Some(picture));
    }

    #[test]
    fn tiles_cover_what_shows_and_no_more() {
        // A pane 100 × 50 whose sheet corner is its own.
        let all = tiles((10.0, 20.0), (40.0, 30.0), (10.0, 20.0, 110.0, 70.0));
        assert_eq!(all, [(10.0, 20.0), (50.0, 20.0), (90.0, 20.0), (10.0, 50.0), (50.0, 50.0), (90.0, 50.0)]);
        // Scrolled by 95 × 35: the tiling starts off screen.
        let scrolled = tiles((-85.0, -15.0), (40.0, 30.0), (10.0, 20.0, 110.0, 70.0));
        assert_eq!(scrolled[0], (-5.0, 15.0));
        assert_eq!(scrolled.len(), 6);
        // Nothing for a picture without a size or a pane without room.
        assert!(tiles((0.0, 0.0), (0.0, 30.0), (0.0, 0.0, 100.0, 100.0)).is_empty());
        assert!(tiles((0.0, 0.0), (40.0, 30.0), (50.0, 0.0, 50.0, 100.0)).is_empty());
        // A picture of one pixel does not flood the display list.
        assert_eq!(tiles((0.0, 0.0), (1.0, 1.0), (0.0, 0.0, 2000.0, 1000.0)).len(), MAX_TILES);
    }
}
