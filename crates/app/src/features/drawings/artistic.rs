//! Picture Format ▸ Artistic Effects: the gallery, and the second image
//! of a picture (`picture_style::Baked`) — its image with the artistic
//! effect, the saturation and the colour temperature done to it, which
//! is what a file keeps in the picture's blip and what the sheet shows.
//!
//! Filtering a photograph takes a moment: the second image is made off
//! the window's thread and shown when it is ready. Until then the
//! picture shows without its artistic effect.

use super::picture::{self, Bitmap};
use super::{Cmd, DrawingUi, edit_picture, filters, pane, selected, tabs};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::i18n::{lang, t, t_ctx};
use crate::theme::{self, c};
use crate::view::tooltip;
use gpui::{App, Bounds, Context, Pixels, RenderImage, WeakEntity, Window, canvas, div, prelude::*, px};
use opensheet_engine::Workbook;
use opensheet_engine::drawing::{Drawing, DrawingKind, ImageFormat, NEUTRAL_TEMPERATURE, Picture, PictureAdjust};
use opensheet_engine::picture_style::{Artistic, ArtisticKind, Baked};
use std::collections::HashSet;
use std::sync::Arc;

/// The room a thumbnail of the gallery has.
const THUMB: (f32, f32) = (84.0, 63.0);
const PER_ROW: usize = 4;

/// What a second image is made from: the image and the settings.
#[derive(Clone)]
struct Recipe {
    data: Arc<[u8]>,
    format: ImageFormat,
    saturation: f32,
    temperature: f32,
    artistic: Option<Artistic>,
}

/// A recipe as numbers: the address of the image (it is kept alive by
/// the recipe) and the settings.
type Key = (usize, [u32; 5]);

impl Recipe {
    fn of(picture: &Picture) -> Recipe {
        Recipe {
            data: picture.data.clone(),
            format: picture.format,
            saturation: picture.adjust.saturation,
            temperature: picture.adjust.temperature,
            artistic: picture.artistic,
        }
    }

    fn key(&self) -> Key {
        let artistic = self.artistic.map_or([u32::MAX, 0, 0], |a| [a.kind as u32, a.transparency.to_bits(), a.amount.to_bits()]);
        (
            Arc::as_ptr(&self.data) as *const u8 as usize,
            [self.saturation.to_bits(), self.temperature.to_bits(), artistic[0], artistic[1], artistic[2]],
        )
    }

    /// Make the image: the colours first, then the effect, at the full
    /// size of the image. A JPEG stays a JPEG (a photograph as a PNG
    /// would be many times larger); anything else becomes a PNG.
    fn cook(&self) -> Option<Baked> {
        let mut pixels = image::load_from_memory(&self.data).ok()?.into_rgba8();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let (width, height) = pixels.dimensions();
        let mut bitmap = Bitmap {
            width,
            height,
            data: pixels.into_raw(),
        };
        let colours = PictureAdjust {
            saturation: self.saturation,
            temperature: self.temperature,
            ..PictureAdjust::default()
        };
        if colours != PictureAdjust::default() {
            bitmap = bitmap.adjusted(&colours, 0.0, (width as f32, height as f32));
        }
        if let Some(artistic) = &self.artistic {
            bitmap = filters::apply(&bitmap, artistic);
        }
        let data = match self.format {
            ImageFormat::Jpeg => picture::jpeg_of(&bitmap)?,
            _ => picture::png_of(&bitmap)?,
        };
        Some(Baked {
            data: data.into(),
            saturation: self.saturation,
            temperature: self.temperature,
            artistic: self.artistic,
        })
    }
}

/// The second images being made and those made lately.
#[derive(Default)]
pub(super) struct Baking {
    /// On their way, or impossible (an image that cannot be decoded is
    /// not tried again).
    asked: HashSet<Key>,
    /// The last ones made, with what they were made from: an undo and a
    /// redo ask for the same image again.
    made: Vec<(Recipe, Baked)>,
}

/// How many second images are remembered.
const REMEMBERED: usize = 12;

/// Do something to every picture of a workbook that waits for its second
/// image.
fn each_waiting(wb: &mut Workbook, sheets: std::ops::Range<usize>, change: &mut dyn FnMut(&mut Picture)) {
    fn visit(drawing: &mut Drawing, change: &mut dyn FnMut(&mut Picture)) {
        match &mut drawing.kind {
            DrawingKind::Picture(picture) if picture.needs_baking() => change(picture),
            DrawingKind::Group(group) => group.members.iter_mut().for_each(|member| visit(member, change)),
            _ => {}
        }
    }
    for sheet in sheets {
        if let Some(sheet) = wb.sheets.get_mut(sheet) {
            sheet.drawings.iter_mut().for_each(|drawing| visit(drawing, change));
        }
    }
}

/// Called at every repaint: the pictures of the sheet that wait for
/// their second image get it — at once when it was made before, when it
/// is ready otherwise. Giving a picture its second image is not a change
/// of the workbook: nothing is added to the undo history.
pub(super) fn follow(app: &mut ExcelApp, cx: &mut Cx) {
    let mut wanted: Vec<Recipe> = Vec::new();
    let sheet = app.sheet;
    each_waiting(&mut app.wb, sheet..sheet + 1, &mut |picture| wanted.push(Recipe::of(picture)));
    if wanted.is_empty() {
        return;
    }
    let mut start = Vec::new();
    for recipe in wanted {
        let key = recipe.key();
        let baking = &mut app.ext_mut::<DrawingUi>().baking;
        if let Some((_, baked)) = baking.made.iter().find(|(made, _)| made.key() == key) {
            let baked = baked.clone();
            serve(&mut app.wb, key, &baked);
        } else if baking.asked.insert(key) {
            start.push(recipe);
        }
    }
    for recipe in start {
        let cooking = cx.background_executor().spawn({
            let recipe = recipe.clone();
            async move { recipe.cook() }
        });
        cx.spawn(async move |this, cx| {
            let baked = cooking.await;
            this.update(cx, |app, cx| {
                // One that cannot be made stays asked for, so that it is
                // not tried at every repaint.
                let Some(baked) = baked else {
                    return;
                };
                let key = recipe.key();
                serve(&mut app.wb, key, &baked);
                let baking = &mut app.ext_mut::<DrawingUi>().baking;
                baking.asked.remove(&key);
                baking.made.push((recipe, baked));
                if baking.made.len() > REMEMBERED {
                    baking.made.remove(0);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Give its second image to every picture that waits for this one.
fn serve(wb: &mut Workbook, key: Key, baked: &Baked) {
    let sheets = 0..wb.sheets.len();
    each_waiting(wb, sheets, &mut |picture| {
        if Recipe::of(picture).key() == key {
            picture.baked = Some(baked.clone());
        }
    });
}

/// Before a workbook is written: every picture that still waits for its
/// second image gets it now, so that the file shows in Excel what the
/// sheet shows here.
pub(crate) fn bake_all(wb: &mut Workbook) {
    let sheets = 0..wb.sheets.len();
    let mut made: Vec<(Key, Option<Baked>)> = Vec::new();
    each_waiting(wb, sheets, &mut |picture| {
        let recipe = Recipe::of(picture);
        let key = recipe.key();
        let baked = match made.iter().find(|(known, _)| *known == key) {
            Some((_, baked)) => baked.clone(),
            None => {
                let baked = recipe.cook();
                made.push((key, baked.clone()));
                baked
            }
        };
        if baked.is_some() {
            picture.baked = baked;
        }
    });
}

/// Give the selected pictures an artistic effect, or take it away.
pub(super) fn set_artistic(app: &mut ExcelApp, artistic: Option<Artistic>, cx: &mut Cx) {
    edit_picture(app, Cmd::ArtisticEffects.label(), cx, |picture| picture.artistic = artistic);
}

/// A small copy of a picture with an effect of the gallery, or as it is.
pub(super) fn sample(picture: &Picture, kind: Option<ArtisticKind>) -> Option<Bitmap> {
    // From the image as it was, with its colours set: the effect comes
    // on top of them.
    let plain = Picture {
        artistic: None,
        baked: None,
        ..picture.clone()
    };
    let small = picture::small_copy(&plain, (THUMB.0 as u32, THUMB.1 as u32))?;
    let colours = PictureAdjust {
        saturation: picture.adjust.saturation,
        temperature: picture.adjust.temperature,
        ..PictureAdjust::default()
    };
    let small = if colours.saturation != 1.0 || colours.temperature != NEUTRAL_TEMPERATURE {
        small.adjusted(&colours, 0.0, (small.width as f32, small.height as f32))
    } else {
        small
    };
    Some(match kind {
        Some(kind) => filters::apply(&small, &Artistic::preset(kind)),
        None => small,
    })
}

/// The drop-down of Artistic Effects: the picture with each effect.
struct EffectGallery {
    app: WeakEntity<ExcelApp>,
    thumbs: Vec<(Option<ArtisticKind>, Arc<RenderImage>)>,
}

impl Render for EffectGallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self
            .app
            .upgrade()
            .and_then(|app| selected(app.read(cx)).and_then(Drawing::as_picture).map(|picture| picture.artistic.map(|artistic| artistic.kind)));
        let locale = lang().locale();
        let tiles = self.thumbs.iter().enumerate().map(|(index, (kind, image))| {
            let (kind, image, app) = (*kind, image.clone(), self.app.clone());
            let name = kind.map_or(t_ctx("artistic", "None", "Aucun"), |kind| kind.name(locale));
            let shown = canvas(
                |_, _, _| {},
                move |bounds: Bounds<Pixels>, _, window, _| {
                    let _ = window.paint_image(bounds, Default::default(), image.clone(), 0, false);
                },
            )
            .w(px(THUMB.0))
            .h(px(THUMB.1));
            div()
                .id(("pf-artistic", index))
                .p(px(3.))
                .rounded(px(3.))
                .border_1()
                .border_color(c(if current == Some(kind) { theme::ACCENT } else { 0xFFFFFF }))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(shown)
                .tooltip(tooltip(name))
                .on_click(move |_, _, cx| {
                    app.update(cx, |app, cx| {
                        app.popup = None;
                        set_artistic(app, kind.map(Artistic::preset), cx);
                        cx.notify();
                    })
                    .ok();
                })
        });
        let app = self.app.clone();
        div()
            .w(px(PER_ROW as f32 * (THUMB.0 + 8.0) + 12.0))
            .p(px(6.))
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(div().flex().flex_wrap().children(tiles))
            .child(div().h(px(1.)).my(px(4.)).bg(c(theme::BORDER)))
            .child(
                div()
                    .id("pf-artistic-options")
                    .h(px(24.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(t("Artistic Effects Options…", "Options d'effets artistiques…"))
                    .on_click(move |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.popup = None;
                            pane::open_format(app, pane::Section::Picture, window, cx);
                        })
                        .ok();
                    }),
            )
    }
}

pub(super) fn open_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(picture) = selected(app).and_then(Drawing::as_picture) else {
        return;
    };
    let thumbs: Vec<(Option<ArtisticKind>, Arc<RenderImage>)> = std::iter::once(None)
        .chain(ArtisticKind::ALL.into_iter().map(Some))
        .filter_map(|kind| Some((kind, sample(picture, kind)?.into_image()?)))
        .collect();
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |cx: &mut Context<EffectGallery>| {
        cx.on_release(|gallery: &mut EffectGallery, cx: &mut App| {
            for (_, image) in gallery.thumbs.drain(..) {
                cx.drop_image(image, None);
            }
        })
        .detach();
        EffectGallery { app: weak, thumbs }
    });
}
