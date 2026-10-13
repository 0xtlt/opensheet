//! Picture Format ▸ Compress Pictures: the images of pictures are made
//! again with no more pixels than a resolution asks for at the size
//! each picture has on the sheet, and, when asked, without the parts
//! their crop leaves out. What is thrown away is gone from the file:
//! the crop of such a picture can no longer be widened.

use super::picture::{self, Bitmap};
use super::{Cmd, selected_ids};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::i18n::t;
use crate::shell::Modal;
use crate::widgets::{Checkbox, RadioGroup, Select, dialog, labeled, ok_cancel};
use gpui::{Context, Window, div, prelude::*, px};
use opensheet_engine::drawing::{Drawing, DrawingKind, ImageFormat, Picture, image_info};

/// The choices of "Picture Quality": pixels to the inch, or the image
/// as it is.
const QUALITIES: [Option<f32>; 4] = [Some(220.0), Some(150.0), Some(96.0), None];

/// What the dialog asks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Options {
    /// Pixels to the inch of the picture as it shows; `None` keeps
    /// those the image has.
    pub ppi: Option<f32>,
    pub delete_cropped: bool,
    /// The selected pictures only, not all those of the workbook.
    pub selected_only: bool,
}

/// The image of a picture made again for `options`, the picture showing
/// `size` pixels wide and high at 100 %: the new file, its crop, and
/// its size in pixels. `None` when nothing would be gained.
pub(super) fn compressed(picture: &Picture, size: (f32, f32), options: &Options) -> Option<(Vec<u8>, [f32; 4])> {
    // A vector picture has no pixels to spare, and a picture of cells
    // is made again whenever they change.
    if picture.svg.is_some() || picture.link.is_some() {
        return None;
    }
    let decoded = image::load_from_memory(&picture.data).ok()?.into_rgba8();
    let (full_w, full_h) = decoded.dimensions();
    let mut crop = picture.adjust.crop;
    let mut pixels = decoded;
    let mut changed = false;
    // The parts the crop cuts off: sides cropped inwards. (A side whose
    // crop is negative leaves room around the image: nothing to delete,
    // but its fraction is of the image, which gets smaller.)
    let inwards = crop.map(|side| side.max(0.0));
    if options.delete_cropped && inwards.iter().any(|side| *side > 0.0) {
        let left = (inwards[0] * full_w as f32).round() as u32;
        let top = (inwards[1] * full_h as f32).round() as u32;
        let right = (inwards[2] * full_w as f32).round() as u32;
        let bottom = (inwards[3] * full_h as f32).round() as u32;
        let (w, h) = (full_w.saturating_sub(left + right).max(1), full_h.saturating_sub(top + bottom).max(1));
        pixels = image::imageops::crop_imm(&pixels, left.min(full_w - 1), top.min(full_h - 1), w, h).to_image();
        let (sx, sy) = (full_w as f32 / w as f32, full_h as f32 / h as f32);
        crop = [
            if crop[0] > 0.0 { 0.0 } else { crop[0] * sx },
            if crop[1] > 0.0 { 0.0 } else { crop[1] * sy },
            if crop[2] > 0.0 { 0.0 } else { crop[2] * sx },
            if crop[3] > 0.0 { 0.0 } else { crop[3] * sy },
        ];
        changed = true;
    }
    if let Some(ppi) = options.ppi {
        // The frame shows the part of the image the crop keeps: the
        // whole image is that much larger than the frame.
        let kept = |a: f32, b: f32| (1.0 - a - b).clamp(0.05, 10.0);
        let whole = (size.0 / kept(crop[0], crop[2]), size.1 / kept(crop[1], crop[3]));
        // A pixel of the sheet is a 96th of an inch.
        let wanted = ((whole.0 / 96.0 * ppi).ceil().max(1.0) as u32, (whole.1 / 96.0 * ppi).ceil().max(1.0) as u32);
        let (w, h) = pixels.dimensions();
        if wanted.0 < w && wanted.1 < h {
            pixels = image::imageops::resize(&pixels, wanted.0, wanted.1, image::imageops::FilterType::Lanczos3);
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    let (width, height) = pixels.dimensions();
    let mut data = pixels.into_raw();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let bitmap = Bitmap { width, height, data };
    // A photograph stays a JPEG; anything else may be transparent.
    let file = match picture.format {
        ImageFormat::Jpeg => picture::jpeg_of(&bitmap)?,
        _ => picture::png_of(&bitmap)?,
    };
    // Resampling alone must make the file smaller to be worth it.
    if crop == picture.adjust.crop && file.len() >= picture.data.len() {
        return None;
    }
    Some((file, crop))
}

/// Compress one drawing, or the pictures of a group. True when
/// something changed.
fn compress_drawing(drawing: &mut Drawing, options: &Options) -> bool {
    let size = drawing.size;
    match &mut drawing.kind {
        DrawingKind::Picture(picture) => {
            let Some((file, crop)) = compressed(picture, size, options) else {
                return false;
            };
            let Some(info) = image_info(&file) else {
                return false;
            };
            picture.data = file.into();
            picture.format = info.format;
            picture.pixels = info.pixels;
            picture.adjust.crop = crop;
            // Its second image was made from the pixels it had.
            picture.baked = None;
            true
        }
        DrawingKind::Group(group) => {
            let mut any = false;
            for member in &mut group.members {
                any |= compress_drawing(member, options);
            }
            any
        }
        _ => false,
    }
}

/// Compress the pictures the options name, as one undo step. Returns
/// how many bytes the images take before and after.
pub(super) fn compress(app: &mut ExcelApp, options: Options, cx: &mut Cx) -> (usize, usize) {
    fn weight(drawing: &Drawing) -> usize {
        match &drawing.kind {
            DrawingKind::Picture(picture) => picture.data.len(),
            DrawingKind::Group(group) => group.members.iter().map(weight).sum(),
            _ => 0,
        }
    }
    let chosen = selected_ids(app);
    let sheets = if options.selected_only { app.sheet..app.sheet + 1 } else { 0..app.wb.sheets.len() };
    // A protected sheet keeps its pictures as they are.
    let targets: Vec<(usize, u32)> = sheets
        .filter(|sheet| *sheet == app.sheet || app.wb.sheets[*sheet].protection.is_none())
        .flat_map(|sheet| app.wb.sheets[sheet].drawings.iter().map(move |drawing| (sheet, drawing.id)))
        .filter(|(_, id)| !options.selected_only || chosen.contains(id))
        .collect();
    if targets.iter().any(|(sheet, _)| *sheet == app.sheet) && !app.may_edit_selected_objects(cx) {
        return (0, 0);
    }
    let (mut before, mut after) = (0, 0);
    let mut any = false;
    for (sheet, id) in targets {
        before += app.wb.drawing(sheet, id).map_or(0, weight);
        any |= app.wb.update_drawing(sheet, id, |drawing| {
            compress_drawing(drawing, &options);
        });
        after += app.wb.drawing(sheet, id).map_or(0, weight);
    }
    if any && after != before {
        app.commit(Cmd::Compress.label(), cx);
    }
    (before, after)
}

/// Excel's Compress Pictures dialog.
struct CompressDialog {
    modal: Modal,
    quality: usize,
    delete_cropped: bool,
    selected_only: bool,
    /// Whether a picture is selected: without one, all it is.
    has_selection: bool,
}

impl CompressDialog {
    fn accept(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let options = Options {
            ppi: QUALITIES[self.quality.min(QUALITIES.len() - 1)],
            delete_cropped: self.delete_cropped,
            selected_only: self.selected_only && self.has_selection,
        };
        self.modal.update_app(cx, |app, cx| {
            compress(app, options, cx);
        });
        self.modal.close(window, cx);
    }
}

impl Render for CompressDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let qualities = vec![
            t("Best for printing (220 ppi)", "Optimale pour l'impression (220 ppp)").into(),
            t("Best for viewing on screen (150 ppi)", "Optimale pour l'affichage à l'écran (150 ppp)").into(),
            t("Best for sending in e-mail (96 ppi)", "Optimale pour l'envoi par courrier (96 ppp)").into(),
            t("Keep current resolution", "Conserver la résolution actuelle").into(),
        ];
        let quality = Select::new("compress-quality", qualities, Some(self.quality)).width(270.0).on_change(cx.listener(|this, choice: &usize, _, cx| {
            this.quality = *choice;
            cx.notify();
        }));
        let delete = Checkbox::new(
            "compress-delete",
            t("Delete cropped areas of pictures", "Supprimer les zones de rognage des images"),
            self.delete_cropped,
        )
        .on_change(cx.listener(|this, on: &bool, _, cx| {
            this.delete_cropped = *on;
            cx.notify();
        }));
        let scope = RadioGroup::new(
            "compress-scope",
            vec![
                t("All pictures in this file", "Toutes les images dans ce fichier").into(),
                t("Selected pictures only", "Uniquement les images sélectionnées").into(),
            ],
            Some((self.selected_only && self.has_selection) as usize),
        )
        .disabled(!self.has_selection)
        .on_change(cx.listener(|this, index: &usize, _, cx| {
            this.selected_only = *index == 1;
            cx.notify();
        }));
        dialog(&self.modal, t("Compress Pictures", "Compresser les images"))
            .width(420.0)
            .on_ok(cx.listener(Self::accept))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(labeled(t("Picture Quality:", "Qualité de l'image :"), quality))
                    .child(delete)
                    .child(scope),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::accept)))
    }
}

pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let has_selection = super::selected(app).is_some_and(|drawing| super::any_leaf(drawing, &super::is_picture));
    app.open_modal(window, cx, move |modal, _, _| CompressDialog {
        modal,
        quality: 0,
        delete_cropped: true,
        selected_only: has_selection,
        has_selection,
    });
}
