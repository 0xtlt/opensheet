//! What the Picture Format and Shape Format tabs do to how a drawing
//! shows: cropping a picture, its transparency, corrections and colours,
//! and the effects that are drawn (shadow, glow, soft edges). The menus,
//! and the changes they make; the pixels are made in `picture.rs` and the
//! shadows in `paint.rs`.

use super::{Cmd, DrawingUi, cropping, edit_leaves, edit_picture, end_crop, end_text_edit, pane, selected, selected_ids};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, Cx};
use crate::i18n::{labelled, lang, t, t_ctx};
use gpui::Window;
use opensheet_engine::Color;
use opensheet_engine::drawing::{
    Drawing, Effects, Frame, Glow, PictureAdjust, Recolor, SEPIA, Shadow, ShadowPreset, ShapeTool,
};

/// The proportions of the Aspect Ratio menu, under their headings: width
/// to height.
const RATIOS: [(&str, &[(u32, u32)]); 3] = [
    ("Square", &[(1, 1)]),
    ("Portrait", &[(2, 3), (3, 4), (3, 5), (4, 5)]),
    ("Landscape", &[(3, 2), (4, 3), (5, 3), (5, 4), (16, 9), (16, 10)]),
];

/// What the Transparency gallery offers, percent.
const TRANSPARENCIES: [u32; 7] = [0, 15, 30, 50, 65, 80, 95];
/// Brightness and contrast of the Corrections gallery, percent.
const CORRECTIONS: [i32; 5] = [-40, -20, 0, 20, 40];
/// Saturations of the Color gallery, percent.
const SATURATIONS: [u32; 7] = [0, 33, 66, 100, 200, 300, 400];
/// Temperatures of the Color gallery, kelvins.
const TEMPERATURES: [u32; 7] = [4700, 5300, 5900, 6500, 7200, 8800, 11200];
/// Radii of the Glow gallery and of the Soft Edges list, points.
const GLOWS: [f32; 4] = [5.0, 8.0, 11.0, 18.0];
const SOFT_EDGES: [f32; 6] = [1.0, 2.5, 5.0, 10.0, 25.0, 50.0];

/// A number of percent as the language writes it: "15%", "15 %".
fn percent(value: i32, signed: bool) -> String {
    let number = if signed && value > 0 { format!("+{value}") } else { value.to_string() };
    if lang().locale().id == "fr" { format!("{number} %") } else { format!("{number}%") }
}

/// A length in points with at most one decimal, in the decimal separator
/// of the language.
fn points(value: f32) -> String {
    let text = crate::widgets::number_text((value as f64 * 10.0).round() / 10.0, lang().locale().decimal_sep);
    format!("{text} pt")
}

/// The picture the commands of the Picture Format tab show the state of.
fn picture_adjust(app: &ExcelApp) -> Option<PictureAdjust> {
    selected(app).and_then(Drawing::as_picture).map(|picture| picture.adjust)
}

// -------------------------------------------------------------------- crop

/// "Crop": show the crop marks of the selected picture, or put them away.
pub(super) fn toggle_crop(app: &mut ExcelApp, cx: &mut Cx) {
    if end_crop(app, cx) {
        return;
    }
    let Some(id) = selected(app).filter(|drawing| drawing.as_picture().is_some()).map(|drawing| drawing.id) else {
        return;
    };
    if selected_ids(app).len() != 1 || !app.may_edit_selected_objects(cx) {
        return;
    }
    end_text_edit(app, cx);
    app.ext_mut::<DrawingUi>().crop = Some(id);
    cx.notify();
}

/// Where a frame goes on the sheet when, seen from a picture in `frame`
/// turned by `rotation`, it is `local` (the picture itself being at the
/// origin): the picture turns about its middle, wherever that moves.
fn placed_from(frame: Frame, rotation: f32, local: Frame) -> Frame {
    let shift = (
        local.x + local.width / 2.0 - frame.width / 2.0,
        local.y + local.height / 2.0 - frame.height / 2.0,
    );
    let (sin, cos) = (rotation as f64).to_radians().sin_cos();
    let centre = (
        frame.x + frame.width / 2.0 + shift.0 * cos - shift.1 * sin,
        frame.y + frame.height / 2.0 + shift.0 * sin + shift.1 * cos,
    );
    Frame::new(centre.0 - local.width / 2.0, centre.1 - local.height / 2.0, local.width, local.height)
}

/// The largest part of an image with the proportions `ratio` (width over
/// height), from its middle.
pub(super) fn ratio_frame(image: Frame, ratio: f64) -> Frame {
    let (width, height) = if image.width / image.height.max(1e-9) > ratio {
        (image.height * ratio, image.height)
    } else {
        (image.width, image.width / ratio)
    };
    Frame::new(
        image.x + (image.width - width) / 2.0,
        image.y + (image.height - height) / 2.0,
        width,
        height,
    )
}

/// The frame an image `natural` in proportions takes to fill `frame`
/// (covering it all, parts cut off) or to fit in it (all of it showing,
/// room left over), centred on it.
pub(super) fn image_frame(frame: Frame, natural: (f64, f64), fit: bool) -> Frame {
    let (sx, sy) = (frame.width / natural.0.max(1e-9), frame.height / natural.1.max(1e-9));
    let scale = if fit { sx.min(sy) } else { sx.max(sy) };
    let (width, height) = (natural.0 * scale, natural.1 * scale);
    Frame::new(
        frame.x + (frame.width - width) / 2.0,
        frame.y + (frame.height - height) / 2.0,
        width,
        height,
    )
}

/// Change the frame and the crop of the selected picture: `change` gets
/// the picture, with its frame at the origin of a box of its own, and
/// the frame of its whole image there; it returns the new frame in that
/// box and the new crop.
fn recrop(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&Drawing, Frame, Frame) -> (Frame, [f32; 4])) {
    let Some(drawing) = selected(app).filter(|drawing| drawing.as_picture().is_some()) else {
        return;
    };
    let (id, sheet) = (drawing.id, app.sheet);
    let Some(frame) = app.wb.drawing_frame(sheet, id) else {
        return;
    };
    if !app.may_edit_selected_objects(cx) {
        return;
    }
    app.wb.update_drawing(sheet, id, |drawing| {
        let Some(adjust) = drawing.as_picture().map(|picture| picture.adjust) else {
            return;
        };
        // Sides as the screen shows them: a flipped picture shows its
        // image the other way round.
        let flips = (drawing.flip_h, drawing.flip_v);
        let seen = PictureAdjust {
            crop: super::tool::seen_crop(adjust.crop, flips),
            ..adjust
        };
        let own = Frame::new(0.0, 0.0, frame.width, frame.height);
        let (local, crop) = change(drawing, own, seen.uncropped(own));
        let placed = placed_from(frame, drawing.rotation, local);
        drawing.anchor = (0, 0);
        drawing.offset = (placed.x.max(0.0) as f32, placed.y.max(0.0) as f32);
        drawing.size = (placed.width as f32, placed.height as f32);
        if let Some(picture) = drawing.as_picture_mut() {
            picture.adjust.crop = super::tool::seen_crop(crop, flips);
        }
    });
    app.commit(label, cx);
}

/// "Aspect Ratio": the picture is cropped from the middle of its image
/// to the proportions `ratio`, and its crop marks show.
fn crop_to_ratio(app: &mut ExcelApp, ratio: (u32, u32), cx: &mut Cx) {
    recrop(app, t("Crop Picture", "Rogner l'image"), cx, |_, _, image| {
        let kept = ratio_frame(image, ratio.0 as f64 / ratio.1 as f64);
        (kept, PictureAdjust::crop_between(image, kept))
    });
    if cropping(app).is_none() {
        toggle_crop(app, cx);
    }
}

/// "Fill" and "Fit": the image is put back in its own proportions in
/// the frame, which stays as it is.
pub(super) fn crop_to_frame(app: &mut ExcelApp, fit: bool, cx: &mut Cx) {
    recrop(app, if fit { Cmd::CropFit.label() } else { Cmd::CropFill.label() }, cx, |drawing, own, _| {
        let pixels = drawing.as_picture().map_or((1, 1), |picture| picture.pixels);
        let image = image_frame(own, (pixels.0 as f64, pixels.1 as f64), fit);
        (own, PictureAdjust::crop_between(image, own))
    });
    if cropping(app).is_none() {
        toggle_crop(app, cx);
    }
}

/// The menu of the Crop button.
pub(super) fn open_crop_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = picture_adjust(app).map(|adjust| adjust.shape);
    let shapes = ShapeTool::ALL
        .into_iter()
        .filter(|tool| !tool.preset().is_line() && *tool != ShapeTool::TextBox)
        .map(|tool| {
            MenuEntry::new(super::tabs::tool_name(tool, app.wb.locale), move |app, _, cx| {
                edit_picture(app, t("Crop to Shape", "Rogner jusqu'à la forme"), cx, |picture| {
                    picture.adjust.shape = tool.preset();
                });
            })
            .checked(current == Some(tool.preset()))
        })
        .collect();
    let mut ratios = Vec::new();
    for (heading, list) in RATIOS {
        let heading = match heading {
            "Square" => t("Square", "Carré"),
            "Portrait" => t("Portrait", "Portrait"),
            _ => t("Landscape", "Paysage"),
        };
        if !ratios.is_empty() {
            ratios.push(MenuEntry::separator());
        }
        ratios.push(MenuEntry::new(heading, |_, _, _| {}).disabled(true));
        for ratio in list {
            let ratio = *ratio;
            ratios.push(MenuEntry::new(format!("{}:{}", ratio.0, ratio.1), move |app, _, cx| crop_to_ratio(app, ratio, cx)));
        }
    }
    let entries = vec![
        MenuEntry::command(Cmd::CropPicture, app).checked(cropping(app).is_some()),
        MenuEntry::submenu(t("Crop to Shape", "Rogner jusqu'à la forme"), shapes),
        MenuEntry::submenu(t("Aspect Ratio", "Rapport hauteur-largeur"), ratios),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::CropFill, app),
        MenuEntry::command(Cmd::CropFit, app),
    ];
    app.open_command_menu(entries, 230.0, window, cx);
}

// ------------------------------------------------------------- adjustments

/// Change how the selected pictures show, as one undo step.
pub(super) fn adjust(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut PictureAdjust)) {
    edit_picture(app, label, cx, |picture| change(&mut picture.adjust));
}

pub(super) fn open_transparency(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = picture_adjust(app).map(|adjust| (adjust.transparency * 100.0).round() as u32);
    let mut entries: Vec<MenuEntry> = TRANSPARENCIES
        .into_iter()
        .map(|amount| {
            MenuEntry::new(labelled(t("Transparency", "Transparence"), percent(amount as i32, false)), move |app, _, cx| {
                adjust(app, t("Picture Transparency", "Transparence d'image"), cx, |adjust| {
                    adjust.transparency = amount as f32 / 100.0;
                });
            })
            .checked(current == Some(amount))
        })
        .collect();
    entries.push(MenuEntry::separator());
    entries.push(MenuEntry::new(
        t("Picture Transparency Options…", "Options de transparence d'image…"),
        |app, window, cx| pane::open_format(app, pane::Section::Picture, window, cx),
    ));
    app.open_command_menu(entries, 260.0, window, cx);
}

pub(super) fn open_corrections(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = picture_adjust(app);
    let level = |value: f32| (value * 100.0).round() as i32;
    let brightness = CORRECTIONS
        .into_iter()
        .map(|amount| {
            MenuEntry::new(labelled(t("Brightness", "Luminosité"), percent(amount, true)), move |app, _, cx| {
                adjust(app, t("Picture Corrections", "Corrections de l'image"), cx, |adjust| {
                    adjust.brightness = amount as f32 / 100.0;
                });
            })
            .checked(current.is_some_and(|adjust| level(adjust.brightness) == amount))
        })
        .collect();
    let contrast = CORRECTIONS
        .into_iter()
        .map(|amount| {
            MenuEntry::new(labelled(t("Contrast", "Contraste"), percent(amount, true)), move |app, _, cx| {
                adjust(app, t("Picture Corrections", "Corrections de l'image"), cx, |adjust| {
                    adjust.contrast = amount as f32 / 100.0;
                });
            })
            .checked(current.is_some_and(|adjust| level(adjust.contrast) == amount))
        })
        .collect();
    let entries = vec![
        MenuEntry::submenu(t("Brightness", "Luminosité"), brightness),
        MenuEntry::submenu(t("Contrast", "Contraste"), contrast),
        MenuEntry::separator(),
        MenuEntry::new(
            t("Picture Corrections Options…", "Options de correction des images…"),
            |app, window, cx| pane::open_format(app, pane::Section::Picture, window, cx),
        ),
    ];
    app.open_command_menu(entries, 270.0, window, cx);
}

/// The six accent colours of the workbook's theme.
fn accents(app: &ExcelApp) -> [Color; 6] {
    let theme = app.wb.theme();
    std::array::from_fn(|n| theme.chart_color(n))
}

/// The Recolor list: its names and what each makes of the picture.
fn recolourings(accents: [Color; 6]) -> Vec<(String, Recolor)> {
    let mut list = vec![
        (t("No Recolor", "Aucune recolorisation").to_string(), Recolor::None),
        (t("Grayscale", "Nuances de gris").to_string(), Recolor::Grayscale),
        (t("Sepia", "Sépia").to_string(), Recolor::Duotone(SEPIA.0, SEPIA.1)),
        (t("Washout", "Estompé").to_string(), Recolor::Washout),
    ];
    for threshold in [25, 50, 75] {
        list.push((
            labelled(t("Black and White", "Noir et blanc"), percent(threshold, false)),
            Recolor::BlackAndWhite(threshold as f32 / 100.0),
        ));
    }
    // The accent colours: from black to the colour, then from the colour
    // to white.
    for (dark, tone) in [(true, t("Dark", "Foncé")), (false, t("Light", "Clair"))] {
        for (n, accent) in accents.into_iter().enumerate() {
            let recolor = if dark {
                Recolor::Duotone(Color(0x000000), accent)
            } else {
                Recolor::Duotone(accent, Color(0xFFFFFF))
            };
            list.push((format!("{} {} – {tone}", t("Accent", "Accentuation"), n + 1), recolor));
        }
    }
    list
}

pub(super) fn open_color(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = picture_adjust(app);
    let label = t("Picture Color", "Couleur de l'image");
    let saturation = SATURATIONS
        .into_iter()
        .map(|amount| {
            MenuEntry::new(labelled(t("Saturation", "Saturation"), percent(amount as i32, false)), move |app, _, cx| {
                adjust(app, label, cx, |adjust| adjust.saturation = amount as f32 / 100.0);
            })
            .checked(current.is_some_and(|adjust| (adjust.saturation * 100.0).round() as u32 == amount))
        })
        .collect();
    let tone = TEMPERATURES
        .into_iter()
        .map(|kelvins| {
            MenuEntry::new(labelled(t("Temperature", "Température"), format!("{kelvins} K")), move |app, _, cx| {
                adjust(app, label, cx, |adjust| adjust.temperature = kelvins as f32);
            })
            .checked(current.is_some_and(|adjust| adjust.temperature.round() as u32 == kelvins))
        })
        .collect();
    let recolor = recolourings(accents(app))
        .into_iter()
        .map(|(name, recolor)| {
            MenuEntry::new(name, move |app, _, cx| adjust(app, label, cx, |adjust| adjust.recolor = recolor))
                .checked(current.is_some_and(|adjust| adjust.recolor == recolor))
        })
        .collect();
    let entries = vec![
        MenuEntry::submenu(t("Saturation", "Saturation"), saturation),
        MenuEntry::submenu(t("Color Tone", "Nuance des couleurs"), tone),
        MenuEntry::submenu(t("Recolor", "Recolorier"), recolor),
        MenuEntry::separator(),
        MenuEntry::new(
            t("Picture Color Options…", "Options de couleur de l'image…"),
            |app, window, cx| pane::open_format(app, pane::Section::Picture, window, cx),
        ),
    ];
    app.open_command_menu(entries, 250.0, window, cx);
}

// ----------------------------------------------------------------- effects

/// Change the effects of the selected drawings (and of the members of
/// the selected groups), as one undo step.
pub(super) fn set_effects(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut Effects)) {
    edit_leaves(app, label, cx, |drawing| change(&mut drawing.effects));
}

/// The menu of Shape Effects and Picture Effects: the effects that are
/// drawn, and greyed out those that are not.
pub(super) fn open_effects(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = selected(app).map(|drawing| drawing.effects).unwrap_or_default();
    let options = |label: &'static str| {
        MenuEntry::new(label, |app, window, cx| pane::open_format(app, pane::Section::Effects, window, cx))
    };
    let shadow_label = t("Shadow", "Ombre");
    let mut shadow = vec![
        MenuEntry::new(t("No Shadow", "Aucune ombre"), move |app, _, cx| {
            set_effects(app, shadow_label, cx, |effects| effects.shadow = None);
        })
        .checked(current.shadow.is_none()),
        MenuEntry::separator(),
        MenuEntry::new(t("Outer", "Externe"), |_, _, _| {}).disabled(true),
    ];
    for preset in ShadowPreset::ALL {
        shadow.push(
            MenuEntry::new(preset.name(app.wb.locale), move |app, _, cx| {
                set_effects(app, shadow_label, cx, |effects| effects.shadow = Some(preset.shadow()));
            })
            .checked(current.shadow == Some(preset.shadow())),
        );
    }
    shadow.push(MenuEntry::separator());
    shadow.push(options(t("Shadow Options…", "Options d'ombres…")));

    let glow_label = t("Glow", "Lumière");
    let mut glow = vec![
        MenuEntry::new(t("No Glow", "Aucun éclat"), move |app, _, cx| {
            set_effects(app, glow_label, cx, |effects| effects.glow = None);
        })
        .checked(current.glow.is_none()),
        MenuEntry::separator(),
    ];
    for radius in GLOWS {
        let colours = accents(app)
            .into_iter()
            .enumerate()
            .map(|(n, color)| {
                let made = Glow {
                    color,
                    transparency: 0.6,
                    radius,
                };
                MenuEntry::new(format!("{} {}", t("Accent", "Accentuation"), n + 1), move |app, _, cx| {
                    set_effects(app, glow_label, cx, |effects| effects.glow = Some(made));
                })
                .checked(current.glow == Some(made))
            })
            .collect();
        glow.push(MenuEntry::submenu(points(radius), colours));
    }
    glow.push(MenuEntry::separator());
    glow.push(options(t("Glow Options…", "Options d'éclat…")));

    let soft_label = t("Soft Edges", "Bordures adoucies");
    let mut soft = vec![
        MenuEntry::new(t("No Soft Edges", "Aucun contour adouci"), move |app, _, cx| {
            set_effects(app, soft_label, cx, |effects| effects.soft_edge = 0.0);
        })
        .checked(current.soft_edge <= 0.0),
        MenuEntry::separator(),
    ];
    for radius in SOFT_EDGES {
        soft.push(
            MenuEntry::new(points(radius), move |app, _, cx| {
                set_effects(app, soft_label, cx, |effects| effects.soft_edge = radius);
            })
            .checked(current.soft_edge == radius),
        );
    }
    soft.push(MenuEntry::separator());
    soft.push(options(t("Soft Edges Options…", "Options de bord estompé…")));

    // Soft edges are made in the pixels of pictures; a shape has none.
    let pictures = selected(app).is_some_and(|drawing| drawing.as_picture().is_some());
    let off = |label: &'static str| MenuEntry::new(label, |_, _, _| {}).disabled(true);
    let entries = vec![
        MenuEntry::submenu(shadow_label, shadow),
        off(t_ctx("effect", "Reflection", "Réflexion")),
        MenuEntry::submenu(glow_label, glow),
        MenuEntry::submenu(soft_label, soft).disabled(!pictures),
        off(t("Bevel", "Biseau")),
        off(t("3-D Rotation", "Rotation 3D")),
    ];
    app.open_command_menu(entries, 220.0, window, cx);
}

/// The shadow the Shadow section of the pane works on: the one the
/// drawing has, or the one its first control would give it.
pub(super) fn shadow_of(drawing: &Drawing) -> Shadow {
    drawing.effects.shadow.unwrap_or_default()
}

/// Change the shadow of the selected drawings from the pane: one that
/// has none gets Excel's first preset, then the change.
pub(super) fn set_shadow(app: &mut ExcelApp, cx: &mut Cx, change: impl Fn(&mut Shadow)) {
    set_effects(app, t("Shadow", "Ombre"), cx, |effects| change(effects.shadow.get_or_insert_with(Shadow::default)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_for_ratios_fill_and_fit() {
        let image = Frame::new(0.0, 0.0, 400.0, 200.0);
        assert_eq!(ratio_frame(image, 1.0), Frame::new(100.0, 0.0, 200.0, 200.0));
        assert_eq!(ratio_frame(image, 4.0), Frame::new(0.0, 50.0, 400.0, 100.0));
        assert_eq!(ratio_frame(image, 2.0), image);
        // A 2:1 image in a square frame: filling cuts its sides, fitting
        // leaves room above and below.
        let frame = Frame::new(0.0, 0.0, 100.0, 100.0);
        let filled = image_frame(frame, (400.0, 200.0), false);
        assert_eq!(filled, Frame::new(-50.0, 0.0, 200.0, 100.0));
        assert_eq!(PictureAdjust::crop_between(filled, frame), [0.25, 0.0, 0.25, 0.0]);
        let fitted = image_frame(frame, (400.0, 200.0), true);
        assert_eq!(fitted, Frame::new(0.0, 25.0, 100.0, 50.0));
        assert_eq!(PictureAdjust::crop_between(fitted, frame), [0.0, -0.5, 0.0, -0.5]);
    }

    #[test]
    fn a_cropped_frame_stays_under_its_image_when_turned() {
        let frame = Frame::new(100.0, 100.0, 200.0, 100.0);
        // Upright: the box of the picture is the sheet moved.
        assert_eq!(placed_from(frame, 0.0, Frame::new(50.0, 0.0, 100.0, 100.0)), Frame::new(150.0, 100.0, 100.0, 100.0));
        // The same frame as it is: nothing moves, whatever the turn.
        let same = placed_from(frame, 37.0, Frame::new(0.0, 0.0, 200.0, 100.0));
        assert!((same.x - 100.0).abs() < 1e-9 && (same.y - 100.0).abs() < 1e-9);
        // A quarter turn: the right half of the picture is the lower
        // half on the sheet. Middle (200, 150); the half's middle is 50
        // to the right, so 50 down.
        let half = placed_from(frame, 90.0, Frame::new(100.0, 0.0, 100.0, 100.0));
        assert!((half.x - 150.0).abs() < 1e-9 && (half.y - 150.0).abs() < 1e-9, "{half:?}");
    }

    #[test]
    fn every_recolouring_has_a_name_of_its_own() {
        let list = recolourings(std::array::from_fn(|n| Color(n as u32 + 1)));
        assert_eq!(list.len(), 19);
        let names: std::collections::HashSet<&str> = list.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names.len(), list.len());
        assert_eq!(list[0].1, Recolor::None);
        assert_eq!(list[2].1, Recolor::Duotone(SEPIA.0, SEPIA.1));
    }
}
