//! Pictures of cells: Paste ▸ Picture and Linked Picture, and Copy ▸ Copy
//! as Picture…
//!
//! A range is drawn the way a page is printed (`opensheet_print`: fills,
//! borders, fonts, number formats, merged cells, conditional formats,
//! what floats over the cells), written as an SVG document and turned
//! into pixels by the renderer the window draws its vectors with, at
//! twice the size of the screen so that the picture stays sharp when it
//! is enlarged. What lands on the sheet is an ordinary picture.
//!
//! A linked picture keeps the formula of its range. Each time the
//! workbook changes, the ranges of the linked pictures are drawn again
//! and a picture whose cells look different gets its new image; that is
//! part of the step that changed the cells, not a step of its own.

use super::named;
use crate::app::ExcelApp;
use crate::features::page::PaperPainter;
use crate::features::{Cx, drawings};
use crate::i18n::t;
use crate::shell::Modal;
use crate::widgets::{RadioGroup, dialog, ok_cancel};
use gpui::{App, ClipboardItem, Context, Window, prelude::*};
use opensheet_engine::ast::{CellRef, Expr, RefExpr, RefKind};
use opensheet_engine::chart::Chart;
use opensheet_engine::condfmt::Adornment;
use opensheet_engine::drawing::{Drawing, Picture};
use opensheet_engine::{Rect, Workbook};
use opensheet_print::{Canvas, ObjectPainter, RangeLook, png_file, render_range, scene_svg};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// How many pixels of the picture stand for one of the sheet.
const SHARPNESS: f64 = 2.0;
/// The longest side of a picture of cells, in its own pixels.
const MAX_SIDE: f64 = 4096.0;

/// Draws what floats over the cells of a picture, except linked pictures:
/// one that lies on its own cells would show itself, smaller each time.
struct Floating<'a>(&'a PaperPainter);

impl ObjectPainter for Floating<'_> {
    fn paint_chart(
        &self,
        wb: &Workbook,
        sheet: usize,
        chart: &Chart,
        size: (f64, f64),
        canvas: &mut Canvas,
    ) {
        self.0.paint_chart(wb, sheet, chart, size, canvas);
    }

    fn paint_drawing(
        &self,
        wb: &Workbook,
        sheet: usize,
        drawing: &Drawing,
        size: (f64, f64),
        canvas: &mut Canvas,
    ) {
        if drawing
            .as_picture()
            .is_none_or(|picture| picture.link.is_none())
        {
            self.0.paint_drawing(wb, sheet, drawing, size, canvas);
        }
    }

    fn paint_adornment(
        &self,
        wb: &Workbook,
        sheet: usize,
        cell: (u32, u32),
        adornment: &Adornment,
        size: (f64, f64),
        canvas: &mut Canvas,
    ) {
        self.0
            .paint_adornment(wb, sheet, cell, adornment, size, canvas);
    }
}

/// A range of cells as a drawing: the SVG document and the size of the
/// range on the sheet, in pixels at 100 %. `None` when the range shows
/// nothing.
fn range_document(
    wb: &Workbook,
    sheet: usize,
    range: Rect,
    look: RangeLook,
    painter: &PaperPainter,
) -> Option<(String, (f64, f64))> {
    let scene = render_range(wb, sheet, range, look, None, Some(&Floating(painter)))?;
    // The scene is in points: three quarters of a pixel.
    let size = (scene.size.0 / 0.75, scene.size.1 / 0.75);
    let sharp = SHARPNESS.min(MAX_SIDE / size.0.max(size.1)).max(0.05);
    let pixels = (
        (size.0 * sharp).round().max(1.0),
        (size.1 * sharp).round().max(1.0),
    );
    Some((scene_svg(&scene, pixels), size))
}

/// The PNG file of a document made by `range_document`, saying how large
/// it is meant to be seen: the size of the cells.
fn document_png(document: &str, size: (f64, f64), cx: &App) -> Option<Vec<u8>> {
    let (width, height, pixels) = drawings::svg_pixels(document.as_bytes(), cx)?;
    png_file(
        width,
        height,
        &pixels,
        Some(96.0 * width as f64 / size.0.max(1.0)),
    )
}

/// How the window shows the cells of a sheet.
fn on_screen(app: &ExcelApp, sheet: usize) -> RangeLook {
    RangeLook::Screen {
        gridlines: app.wb.sheets.get(sheet).is_some_and(|sh| sh.show_gridlines),
    }
}

/// A picture of a range: the image file and the size of the range.
pub(super) fn range_picture(
    app: &ExcelApp,
    sheet: usize,
    range: Rect,
    look: RangeLook,
    cx: &App,
) -> Option<(Vec<u8>, (f64, f64))> {
    let painter = PaperPainter::new(cx);
    let (document, size) = range_document(&app.wb, sheet, range, look, &painter)?;
    Some((document_png(&document, size, cx)?, size))
}

/// The formula of a linked picture on `sheet`: the range, named with its
/// sheet when it is on another one (`=$A$1:$C$4`, `=Data!$A$1:$C$4`).
fn link_formula(wb: &Workbook, sheet: usize, from: usize, range: Rect) -> Expr {
    let corner = |row: u32, col: u32| CellRef {
        row,
        col,
        abs_row: true,
        abs_col: true,
    };
    Expr::Ref(RefExpr {
        sheet: (from != sheet).then(|| Arc::from(wb.sheets[from].name.as_str())),
        start: corner(range.r0, range.c0),
        end: (!range.is_single()).then(|| corner(range.r1, range.c1)),
        kind: RefKind::Cells,
    })
}

/// Paste ▸ Picture (and Linked Picture with `linked`): the copied cells
/// as a picture whose top-left corner is that of the active cell.
pub(super) fn paste(app: &mut ExcelApp, linked: bool, cx: &mut Cx) {
    let Some((from, range)) = app
        .clip
        .as_ref()
        .filter(|state| !state.cut)
        .map(|state| (state.clip.sheet, state.clip.rect))
    else {
        return;
    };
    if from >= app.wb.sheets.len() {
        return;
    }
    let Some((image, size)) = range_picture(app, from, range, on_screen(app, from), cx) else {
        return;
    };
    let Some(mut picture) = Picture::new(image) else {
        return;
    };
    if linked {
        picture.link = Some(Arc::new(link_formula(&app.wb, app.sheet, from, range)));
    }
    let mut drawing = Drawing::picture(picture);
    drawing.size = (size.0 as f32, size.1 as f32);
    let cell = app.sel().current();
    let at = app.wb.sheets[app.sheet].corner_point(((cell.r0, cell.c0), (0.0, 0.0)));
    let label = if linked {
        t("Paste Linked Picture", "Coller l’image liée")
    } else {
        t("Paste Picture", "Coller l’image")
    };
    drawings::insert_picture(app, drawing, at, label, cx);
}

// ------------------------------------------------------- linked pictures

/// What the linked pictures were last drawn from.
#[derive(Default)]
pub(super) struct Followed {
    generation: Option<u64>,
    /// For each linked picture (sheet, id), the document its image was
    /// made from, as a number.
    shown: HashMap<(usize, u32), u64>,
}

fn fingerprint(document: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    document.hash(&mut hasher);
    hasher.finish()
}

/// Called at every repaint: when the workbook changed, give each linked
/// picture whose cells look different the image of what they look like
/// now. A picture met for the first time is drawn too: a file may hold
/// an image made before its cells last changed.
pub(super) fn follow_links(app: &mut ExcelApp, cx: &App) {
    let generation = app.wb.generation;
    if app
        .ext::<Followed>()
        .is_some_and(|followed| followed.generation == Some(generation))
    {
        return;
    }
    let linked = app.wb.linked_pictures();
    let mut shown = HashMap::new();
    if !linked.is_empty() {
        let painter = PaperPainter::new(cx);
        for (sheet, id, source) in linked {
            let look = on_screen(app, source.sheet);
            let Some((document, size)) =
                range_document(&app.wb, source.sheet, source.rect, look, &painter)
            else {
                continue;
            };
            let print = fingerprint(&document);
            let known = app
                .ext::<Followed>()
                .and_then(|followed| followed.shown.get(&(sheet, id)).copied());
            if known != Some(print)
                && let Some(image) = document_png(&document, size, cx)
            {
                app.wb.refresh_linked_picture(sheet, id, image);
            }
            shown.insert((sheet, id), print);
        }
    }
    let followed = app.ext_mut::<Followed>();
    followed.generation = Some(generation);
    followed.shown = shown;
}

// -------------------------------------------------------- copy as picture

/// Home ▸ Copy ▸ Copy as Picture…: Excel's question, then the picture of
/// the selection on the clipboard.
pub(super) fn open_copy(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.sel().ranges.len() > 1 {
        app.set_status(
            t(
                "This action won't work on multiple selections.",
                "Cette action ne peut pas être effectuée sur des sélections multiples.",
            ),
            cx,
        );
        return;
    }
    app.open_modal(window, cx, |modal, _, _| CopyPicture {
        modal,
        printed: false,
    });
}

/// Put the picture of the selection on the clipboard: what Paste then
/// inserts, here or in another application.
pub(super) fn copy(app: &mut ExcelApp, printed: bool, cx: &mut Cx) -> bool {
    let range = app.sel().current();
    // Whole rows and columns stop where the sheet does.
    let range = app.wb.copy(app.sheet, range).rect;
    let look = if printed {
        RangeLook::Printed
    } else {
        on_screen(app, app.sheet)
    };
    let Some((image, _)) = range_picture(app, app.sheet, range, look, cx) else {
        return false;
    };
    // The cells that were copied before are no longer what Paste pastes.
    app.clip = None;
    app.object_clip = None;
    cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
        gpui::ImageFormat::Png,
        image,
    )));
    cx.notify();
    true
}

struct CopyPicture {
    modal: Modal,
    /// "As shown when printed" rather than "As shown on screen".
    printed: bool,
}

impl CopyPicture {
    fn accept(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let printed = self.printed;
        self.modal.update_app(cx, |app, cx| copy(app, printed, cx));
        self.modal.close(window, cx);
    }
}

impl Render for CopyPicture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = RadioGroup::new(
            "appearance",
            vec![
                t("As shown on screen", "Telle qu’à l’écran").into(),
                t("As shown when printed", "Telle qu’à l’impression").into(),
            ],
            Some(self.printed as usize),
        )
        .on_change(cx.listener(|this, index: &usize, _, cx| {
            this.printed = *index == 1;
            cx.notify();
        }));
        dialog(&self.modal, t("Copy Picture", "Copier l’image"))
            .width(300.0)
            .on_ok(cx.listener(Self::accept))
            .child(named("copy-picture-appearance", appearance))
            .footer(ok_cancel(&self.modal, cx.listener(Self::accept)))
    }
}
