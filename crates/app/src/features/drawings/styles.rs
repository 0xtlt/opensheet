//! The Picture Styles group of the Picture Format tab: the gallery of
//! styles, Picture Border, and the families of Picture Effects that a
//! picture alone has (reflection, bevel, 3-D rotation). The pixels are
//! made in `render.rs`.

use super::picture::{self, Bitmap};
use super::render::Dress;
use super::{Cmd, adjust, edit_leaves, edit_picture, pane, selected, tabs};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, Cx};
use crate::i18n::{lang, t, t_ctx};
use crate::theme::{self, c};
use crate::view::tooltip;
use crate::widgets::ColorGrid;
use gpui::{
    App, Bounds, Context, Pixels, RenderImage, WeakEntity, Window, canvas, div, point, prelude::*,
    px, size,
};
use opensheet_engine::drawing::{Drawing, Outline, Picture, PictureAdjust};
use opensheet_engine::picture_style::{
    Bevel, BevelPreset, Camera, PictureStyle, Reflection, Rotation3d,
};
use opensheet_engine::style::Color;
use std::sync::Arc;

/// The width, in pixels at 100 %, the picture is taken to have when a
/// style is shown on it in the gallery: small enough for the frames and
/// shadows of the styles to show on a thumbnail.
const SAMPLE_WIDTH: f32 = 150.0;
/// The room a thumbnail of the gallery has.
const THUMB: (f32, f32) = (64.0, 52.0);
const PER_ROW: usize = 7;

/// Give the selected pictures a style of the gallery: its outline, its
/// border and its effects replace theirs, as one undo step.
pub(super) fn apply_style(app: &mut ExcelApp, style: PictureStyle, cx: &mut Cx) {
    let look = style.look();
    edit_leaves(app, t("Picture Style", "Style d'image"), cx, |drawing| {
        let Some(picture) = drawing.as_picture_mut() else {
            return;
        };
        picture.adjust.shape = look.shape;
        picture.outline = look.outline;
        drawing.effects = look.effects;
    });
}

/// The style the selected picture wears, if it is one of the gallery.
pub(super) fn style_of(drawing: &Drawing) -> Option<PictureStyle> {
    let picture = drawing.as_picture()?;
    PictureStyle::of(picture.adjust.shape, picture.outline, &drawing.effects)
}

/// A picture as a style of the gallery would show it, small: the bitmap
/// and the share of it the picture's frame takes.
pub(super) fn style_sample(
    picture: &Picture,
    proportions: (f32, f32),
    style: PictureStyle,
) -> Option<(Bitmap, (f32, f32))> {
    let look = style.look();
    let natural = (
        SAMPLE_WIDTH,
        (SAMPLE_WIDTH * proportions.1 / proportions.0.max(1.0)).clamp(40.0, 400.0),
    );
    let small = picture::small_copy(picture, (THUMB.0 as u32 * 2, THUMB.1 as u32 * 2))?;
    let (_, shown) = picture.shown();
    let adjust = PictureAdjust {
        shape: look.shape,
        ..shown
    };
    let shorter = natural.0.min(natural.1);
    let soft = (crate::grid::pt_to_px(look.effects.soft_edge) / shorter).min(0.5);
    let body = small.adjusted(&adjust, soft, natural);
    let dress = Dress {
        outline: look.outline,
        effects: look.effects,
        turn: 0.0,
    };
    Some(body.dressed(look.shape, natural, (false, false), &dress))
}

/// The drop-down of Quick Styles: every style shown on the picture.
struct StyleGallery {
    app: WeakEntity<ExcelApp>,
    thumbs: Vec<(PictureStyle, Arc<RenderImage>)>,
}

impl Render for StyleGallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self
            .app
            .upgrade()
            .and_then(|app| selected(app.read(cx)).and_then(style_of));
        let locale = lang().locale();
        let tiles = self
            .thumbs
            .iter()
            .enumerate()
            .map(|(index, (style, image))| {
                let (style, image, app) = (*style, image.clone(), self.app.clone());
                let picture = canvas(
                    |_, _, _| {},
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        // The whole of it, as large as the room allows.
                        let (w, h) = (
                            i32::from(image.size(0).width) as f32,
                            i32::from(image.size(0).height) as f32,
                        );
                        let fit = (f32::from(bounds.size.width) / w.max(1.0))
                            .min(f32::from(bounds.size.height) / h.max(1.0));
                        let (w, h) = (w * fit, h * fit);
                        let origin = point(
                            bounds.origin.x + (bounds.size.width - px(w)) / 2.0,
                            bounds.origin.y + (bounds.size.height - px(h)) / 2.0,
                        );
                        let _ = window.paint_image(
                            Bounds::new(origin, size(px(w), px(h))),
                            Default::default(),
                            image.clone(),
                            0,
                            false,
                        );
                    },
                )
                .w(px(THUMB.0))
                .h(px(THUMB.1));
                div()
                    .id(("pf-style", index))
                    .p(px(3.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(c(if current == Some(style) {
                        theme::ACCENT
                    } else {
                        0xFFFFFF
                    }))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(picture)
                    .tooltip(tooltip(style.name(locale)))
                    .on_click(move |_, _, cx| {
                        app.update(cx, |app, cx| {
                            app.popup = None;
                            apply_style(app, style, cx);
                            cx.notify();
                        })
                        .ok();
                    })
            });
        div()
            .w(px(PER_ROW as f32 * (THUMB.0 + 8.0) + 12.0))
            .p(px(6.))
            .flex()
            .flex_wrap()
            .children(tiles)
    }
}

/// The thumbnails of the gallery for a picture.
fn thumbnails(picture: &Picture, proportions: (f32, f32)) -> Vec<(PictureStyle, Arc<RenderImage>)> {
    PictureStyle::ALL
        .into_iter()
        .filter_map(|style| {
            let (bitmap, _) = style_sample(picture, proportions, style)?;
            Some((style, bitmap.into_image()?))
        })
        .collect()
}

pub(super) fn open_styles(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(drawing) = selected(app).filter(|drawing| drawing.as_picture().is_some()) else {
        return;
    };
    let proportions = app
        .wb
        .drawing_frame(app.sheet, drawing.id)
        .map_or((4.0, 3.0), |frame| {
            (frame.width as f32, frame.height as f32)
        });
    let thumbs = drawing
        .as_picture()
        .map(|picture| thumbnails(picture, proportions))
        .unwrap_or_default();
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |cx: &mut Context<StyleGallery>| {
        // The textures of the thumbnails go with the drop-down.
        cx.on_release(|gallery: &mut StyleGallery, cx: &mut App| {
            for (_, image) in gallery.thumbs.drain(..) {
                cx.drop_image(image, None);
            }
        })
        .detach();
        StyleGallery { app: weak, thumbs }
    });
}

// ------------------------------------------------------------------ border

/// The border a picture gets when one is asked for and it has none.
fn new_border() -> Outline {
    Outline::solid(Color::BLACK, 1.0)
}

/// Change the border of the selected pictures.
pub(super) fn set_border(app: &mut ExcelApp, cx: &mut Cx, change: impl Fn(&mut Option<Outline>)) {
    edit_picture(app, Cmd::PictureBorder.label(), cx, |picture| {
        change(&mut picture.outline)
    });
}

/// The width of the border, from the pane: a border of no width is none.
pub(super) fn set_border_width(app: &mut ExcelApp, points: f32, cx: &mut Cx) {
    set_border(app, cx, |outline| {
        if points <= 0.0 {
            *outline = None;
        } else {
            outline.get_or_insert_with(new_border).width = points;
        }
    });
}

/// The drop-down of Picture Border: no outline, a colour, a weight.
struct BorderMenu {
    app: WeakEntity<ExcelApp>,
}

impl Render for BorderMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.app.upgrade().and_then(|app| {
            selected(app.read(cx))
                .and_then(Drawing::as_picture)
                .map(|picture| picture.outline)
        });
        let apply = |app: &WeakEntity<ExcelApp>,
                     cx: &mut App,
                     change: Box<dyn Fn(&mut Option<Outline>)>| {
            app.update(cx, |app, cx| {
                app.popup = None;
                set_border(app, cx, &*change);
                cx.notify();
            })
            .ok();
        };
        let app = self.app.clone();
        let grid = ColorGrid::new(
            "pf-border-colors",
            current.map(|outline| outline.map(|line| line.color)),
        )
        .none_label(t("No Outline", "Sans contour"))
        .more(&self.app)
        .on_pick(move |color: &Option<Color>, _, cx| {
            let color = *color;
            apply(
                &app,
                cx,
                Box::new(move |outline| match color {
                    Some(color) => outline.get_or_insert_with(new_border).color = color,
                    None => *outline = None,
                }),
            );
        });
        let weights = tabs::WEIGHTS
            .iter()
            .enumerate()
            .map(|(i, (points, label))| {
                let (points, app) = (*points, self.app.clone());
                let on = current.flatten().is_some_and(|line| line.width == points);
                div()
                    .id(("pf-border-weight", i))
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(3.))
                    .when(on, |d| d.bg(c(theme::TOGGLED)))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(div().w(px(40.)).child(*label))
                    .child(
                        div()
                            .w(px(70.))
                            .h(px(crate::grid::pt_to_px(points).max(1.0)))
                            .bg(c(0x404040)),
                    )
                    .on_click(move |_, _, cx| {
                        apply(
                            &app,
                            cx,
                            Box::new(move |outline| {
                                outline.get_or_insert_with(new_border).width = points
                            }),
                        );
                    })
            });
        div()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.))
            .child(grid)
            .child(div().h(px(1.)).my(px(3.)).bg(c(theme::BORDER)))
            .child(
                div()
                    .px(px(4.))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t("Weight", "Épaisseur")),
            )
            .children(weights)
    }
}

pub(super) fn open_border(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |_| BorderMenu { app: weak });
}

// ----------------------------------------------------------------- effects

/// The entry that opens the Effects page of the Format pane.
fn options(label: &'static str) -> MenuEntry {
    MenuEntry::new(label, |app, window, cx| {
        pane::open_format(app, pane::Section::Effects, window, cx)
    })
}

/// The Reflection list of Picture Effects.
pub(super) fn reflection_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let current = selected(app).and_then(|drawing| drawing.effects.reflection);
    let label = t_ctx("effect", "Reflection", "Réflexion");
    let mut entries = vec![
        MenuEntry::new(
            t("No Reflection", "Aucune réflexion"),
            move |app, _, cx| {
                adjust::set_effects(app, label, cx, |effects| effects.reflection = None);
            },
        )
        .checked(current.is_none()),
        MenuEntry::separator(),
    ];
    for (index, preset) in Reflection::presets().into_iter().enumerate() {
        entries.push(
            MenuEntry::new(
                Reflection::preset_name(index, app.wb.locale),
                move |app, _, cx| {
                    adjust::set_effects(app, label, cx, |effects| {
                        effects.reflection = Some(preset)
                    });
                },
            )
            .checked(current == Some(preset)),
        );
    }
    entries.push(MenuEntry::separator());
    entries.push(options(t("Reflection Options…", "Options de réflexion…")));
    entries
}

/// The Bevel list of Picture Effects.
pub(super) fn bevel_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let current = selected(app).and_then(|drawing| drawing.effects.bevel);
    let label = t("Bevel", "Biseau");
    let mut entries = vec![
        MenuEntry::new(t("No Bevel", "Aucun biseau"), move |app, _, cx| {
            adjust::set_effects(app, label, cx, |effects| effects.bevel = None);
        })
        .checked(current.is_none()),
        MenuEntry::separator(),
    ];
    for preset in BevelPreset::ALL {
        entries.push(
            MenuEntry::new(preset.name(app.wb.locale), move |app, _, cx| {
                adjust::set_effects(app, label, cx, |effects| {
                    effects.bevel = Some(Bevel::preset(preset))
                });
            })
            .checked(current == Some(Bevel::preset(preset))),
        );
    }
    entries.push(MenuEntry::separator());
    entries.push(options(t("3-D Options…", "Options 3D…")));
    entries
}

/// The 3-D Rotation list of Picture Effects: the cameras under their
/// two headings.
pub(super) fn rotation_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let current = selected(app).and_then(|drawing| drawing.effects.rotation);
    let label = t("3-D Rotation", "Rotation 3D");
    let mut entries = vec![
        MenuEntry::new(t("No Rotation", "Aucune rotation"), move |app, _, cx| {
            adjust::set_effects(app, label, cx, |effects| effects.rotation = None);
        })
        .checked(current.is_none()),
    ];
    for (heading, parallel) in [
        (t("Parallel", "Parallèle"), true),
        (t("Perspective", "Perspective"), false),
    ] {
        entries.push(MenuEntry::separator());
        entries.push(MenuEntry::new(heading, |_, _, _| {}).disabled(true));
        for camera in Camera::ALL
            .into_iter()
            .filter(|camera| camera.parallel() == parallel)
        {
            let made = camera.rotation();
            entries.push(
                MenuEntry::new(camera.name(app.wb.locale), move |app, _, cx| {
                    adjust::set_effects(app, label, cx, |effects| effects.rotation = Some(made));
                })
                .checked(current == Some(made)),
            );
        }
    }
    entries.push(MenuEntry::separator());
    entries.push(options(t(
        "3-D Rotation Options…",
        "Options de rotation 3D…",
    )));
    entries
}

/// The turn in space the pane starts from for a picture that lies flat.
pub(super) const FLAT: Rotation3d = Rotation3d {
    x: 0.0,
    y: 0.0,
    z: 0.0,
    perspective: 0.0,
};

/// A picture that looks like a photograph, for what has to be judged on
/// one (tests and the samples they keep): a sky that darkens upwards
/// with a sun and its halo, hills in two shades, a lake that mirrors
/// the sky, and the grain of a sensor. Blue first, as `Bitmap`.
#[cfg(test)]
pub(crate) fn sample_photo(width: u32, height: u32) -> Bitmap {
    let (w, h) = (width as f32, height as f32);
    // The same grain every time: a small generator of its own.
    let mut seed = 0x2545_F491u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed & 0xFFFF) as f32 / 65535.0 - 0.5
    };
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let (u, v) = (x as f32 / w, y as f32 / h);
            // Red, green, blue.
            let sky = mix([0.16, 0.35, 0.72], [0.98, 0.78, 0.55], (v / 0.6).powf(1.6));
            let sun = ((u - 0.7).powi(2) * (w / h).powi(2) + (v - 0.33).powi(2)).sqrt();
            let mut rgb = mix(sky, [1.0, 0.97, 0.82], (1.0 - sun / 0.22).max(0.0).powi(2));
            if sun < 0.05 {
                rgb = [1.0, 0.98, 0.9];
            }
            let far = 0.52 + 0.07 * (u * 7.0).sin() + 0.03 * (u * 19.0 + 1.0).sin();
            let near = 0.66 + 0.09 * (u * 4.0 + 2.0).sin() + 0.02 * (u * 31.0).sin();
            if v > far {
                rgb = mix([0.36, 0.42, 0.52], [0.28, 0.36, 0.40], (v - far) * 6.0);
            }
            if v > near {
                rgb = mix([0.13, 0.30, 0.16], [0.07, 0.17, 0.09], (v - near) * 4.0);
            }
            if v > 0.84 {
                // The lake: the sky upside down, darker, with ripples.
                let ripple = 0.03 * ((v * 140.0).sin() * (u * 23.0 + v * 9.0).sin());
                let above = mix(
                    [0.16, 0.35, 0.72],
                    [0.98, 0.78, 0.55],
                    (((1.68 - v) / 0.6) + ripple).clamp(0.0, 1.0).powf(1.6),
                );
                rgb = mix(above, [0.05, 0.12, 0.2], 0.35);
            }
            let grain = noise() * 0.035;
            let byte = |value: f32| ((value + grain).clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[byte(rgb[2]), byte(rgb[1]), byte(rgb[0]), 255]);
        }
    }
    Bitmap {
        width,
        height,
        data,
    }
}

/// `sample_photo` as a PNG file.
#[cfg(test)]
pub(crate) fn sample_photo_png(width: u32, height: u32) -> Vec<u8> {
    picture::png_of(&sample_photo(width, height)).expect("a PNG file")
}
