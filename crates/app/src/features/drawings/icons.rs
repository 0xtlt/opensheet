//! Vector pictures: SVG files brought in like any picture, the icons of
//! Insert ▸ Icons, how both are drawn (from their document, at the size
//! they are shown) and the "Graphics Format" tab that goes with them.
//!
//! A vector picture is a picture whose `svg` holds the document; its
//! `data` is the PNG image made of it when it was inserted or last
//! repainted, which is what a file keeps beside the document and what is
//! printed.
//!
//! Not done: Convert to Shape (the engine has no free-form geometry to
//! convert the outlines to) and Graphics Effects.

use super::geometry::Placed;
use super::icon_set::{self, Icon};
use super::{Cmd, Drawings, GRAPHICS_TAB, edit, picture, selected, svg, tabs};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent, object_id};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, launcher_group, rows};
use crate::shell::Modal;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::view::tooltip;
use crate::widgets::{self, Button, ColorGrid, ListBox, ScrollArea};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, Pixels, RenderImage, SharedString, Subscription, WeakEntity, Window, canvas, div,
    prelude::*, px,
};
use opensheet_engine::drawing::{Drawing, DrawingKind, Picture};
use opensheet_engine::style::Color;
use opensheet_engine::svg::{Repaint, is_svg, repaint, resized, svg_size};
use std::io::Cursor;
use std::sync::Arc;

/// The longest side of the image kept beside the document of a vector
/// picture, in pixels.
const FALLBACK_SIDE: f32 = 1024.0;

/// The PNG image of an SVG document, at twice the size it asks for: what
/// stands for a vector picture where vectors cannot be drawn. `None` when
/// the document cannot be drawn.
fn fallback(document: &[u8]) -> Option<Vec<u8>> {
    let (width, height) = svg_size(document)?;
    let fit = (FALLBACK_SIDE / width.max(height)).min(2.0);
    let sized = resized(document, (width * fit).round().max(1.0), (height * fit).round().max(1.0))?;
    let bitmap = svg::rasterize(&sized)?;
    // Back to red, green, blue: the order of a file.
    let mut pixels = bitmap.data;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = image::RgbaImage::from_raw(bitmap.width, bitmap.height, pixels)?;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// A vector picture from an SVG document.
pub(super) fn vector(document: Vec<u8>) -> Option<Picture> {
    let image = fallback(&document)?;
    Picture::vector(document, image)
}

/// The picture a file holds: a vector picture for an SVG document, a
/// bitmap for an image file.
pub(super) fn picture_from(bytes: Vec<u8>) -> Option<Picture> {
    if is_svg(&bytes) { vector(bytes) } else { Picture::new(bytes) }
}

/// True for a vector picture: it gets the "Graphics Format" tab.
pub(super) fn is_graphic(drawing: &Drawing) -> bool {
    drawing.as_picture().is_some_and(|picture| picture.svg.is_some())
}

/// What to paint for a vector picture that stands upright or on a side:
/// its document drawn at the size it is shown, so that it is sharp at any
/// zoom. `None` for bitmaps, and for a picture turned by an odd angle,
/// which is painted from its image like any picture.
pub(super) fn image_for(picture: &Picture, placed: &Placed, scale: f32) -> Option<(Arc<RenderImage>, (f32, f32, f32, f32))> {
    let document = picture.svg.as_deref()?;
    let quarters = (placed.rotation / 90.0).round();
    if (placed.rotation - quarters * 90.0).abs() >= 0.05 {
        return None;
    }
    let (width, height) = (placed.width().abs() * scale, placed.height().abs() * scale);
    let fit = (svg::MAX_SIDE / width.max(height).max(1.0)).min(1.0);
    let sized = resized(document, (width * fit).round().max(1.0), (height * fit).round().max(1.0))?;
    let image = svg::image(
        std::str::from_utf8(&sized).ok()?,
        (placed.flip_h, placed.flip_v),
        (quarters as i32).rem_euclid(4) as u8,
    )?;
    Some((image, placed.extent()))
}

// ---- inserting -----------------------------------------------------------

/// Add icons to the current sheet as one undo step, from the active cell,
/// each a little lower and further right than the one before; the last
/// one is selected.
pub(super) fn insert_icons(app: &mut ExcelApp, icons: &[&Icon], cx: &mut Cx) {
    if icons.is_empty() || !app.may(Intent::EditObject(None), cx) {
        return;
    }
    svg::install(cx);
    let sheet = app.sheet;
    let (x, y) = app.wb.sheets[sheet].corner_point((app.active(), (0.0, 0.0)));
    let mut last = None;
    for (i, icon) in icons.iter().enumerate() {
        let Some(picture) = vector(icon.svg().into_bytes()) else {
            continue;
        };
        let mut drawing = Drawing::picture(picture);
        drawing.alt_text = icon.name.to_string();
        let step = 16.0 * i as f64;
        drawing.offset = ((x + step) as f32, (y + step) as f32);
        last = app.wb.add_drawing(sheet, drawing).or(last);
    }
    let Some(id) = last else {
        return;
    };
    app.commit(t("Insert Icons", "Insérer des icônes"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(GRAPHICS_TAB);
}

/// "Change Graphic ▸ From Icons": the icon takes the place of the selected
/// graphic, in its frame.
fn replace_with(app: &mut ExcelApp, icon: &Icon, cx: &mut Cx) {
    svg::install(cx);
    let Some(picture) = vector(icon.svg().into_bytes()) else {
        return;
    };
    edit(app, Cmd::ChangeGraphic.label(), cx, |drawing| {
        if matches!(drawing.kind, DrawingKind::Picture(_)) {
            drawing.kind = DrawingKind::Picture(picture);
        }
    });
}

/// Graphics Fill and Graphics Outline: the document of the selected
/// graphic is repainted, and the image that stands for it made anew.
pub(super) fn repaint_selected(app: &mut ExcelApp, label: &str, change: Repaint, cx: &mut Cx) -> bool {
    let Some(document) = selected(app).and_then(Drawing::as_picture).and_then(|picture| picture.svg.clone()) else {
        return false;
    };
    svg::install(cx);
    let Some(picture) = repaint(&document, change).and_then(vector) else {
        return false;
    };
    edit(app, label, cx, |drawing| drawing.kind = DrawingKind::Picture(picture))
}

// ---- the dialog ----------------------------------------------------------

/// A pictogram drawn in a square of `side` pixels, in `ink`.
fn thumbnail(icon: &Icon, side: f32, ink: u32) -> impl IntoElement {
    let document = repaint(icon.svg().as_bytes(), Repaint::Fill(Some(Color(ink)))).unwrap_or_default();
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let pixels = (side * window.scale_factor()).round();
            let image = resized(&document, pixels, pixels)
                .and_then(|sized| svg::image(std::str::from_utf8(&sized).ok()?, (false, false), 0));
            if let Some(image) = image {
                let _ = window.paint_image(bounds, Default::default(), image, 0, false);
            }
        },
    )
    .size(px(side))
    .flex_none()
}

/// The icons a search finds: those whose name, in the language of the
/// interface or in English, holds every word typed.
fn matching(icons: &[Icon], category: Option<usize>, search: &str) -> Vec<usize> {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    icons
        .iter()
        .enumerate()
        .filter(|(_, icon)| category.is_none_or(|category| icon.category == category))
        .filter(|(_, icon)| {
            let names = format!("{} {}", icon.name.to_lowercase(), icon.english.to_lowercase());
            words.iter().all(|word| names.contains(word.as_str()))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Insert ▸ Icons: a search box, the categories, the icons of the one
/// chosen; a click ticks an icon, Insert adds those that are ticked.
pub(super) struct IconsDialog {
    modal: Modal,
    icons: Vec<Icon>,
    search: Entity<TextInput>,
    /// `None`: all categories.
    category: Option<usize>,
    /// The icons ticked, in the order they were.
    chosen: Vec<usize>,
    /// The icon takes the place of the selected graphic.
    replace: bool,
    _subscription: Subscription,
}

impl IconsDialog {
    fn new(modal: Modal, replace: bool, cx: &mut Context<Self>) -> IconsDialog {
        let search = widgets::input(cx, "");
        let subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed) {
                cx.notify();
            }
        });
        IconsDialog {
            modal,
            icons: icon_set::icons(),
            search,
            category: None,
            chosen: Vec::new(),
            replace,
            _subscription: subscription,
        }
    }

    pub(super) fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.replace {
            // One icon takes the place of the graphic.
            self.chosen = vec![index];
        } else if let Some(at) = self.chosen.iter().position(|chosen| *chosen == index) {
            self.chosen.remove(at);
        } else {
            self.chosen.push(index);
        }
        cx.notify();
    }

    pub(super) fn insert(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let chosen: Vec<&Icon> = self.chosen.iter().filter_map(|index| self.icons.get(*index)).collect();
        if chosen.is_empty() {
            return;
        }
        let replace = self.replace;
        self.modal.update_app(cx, |app, cx| {
            if replace {
                replace_with(app, chosen[0], cx);
            } else {
                insert_icons(app, &chosen, cx);
            }
        });
        self.modal.close(window, cx);
    }

    #[cfg(test)]
    pub(super) fn search_for(&mut self, text: &str, cx: &mut Context<Self>) -> Vec<usize> {
        self.search.update(cx, |input, cx| input.set_text(text, cx));
        matching(&self.icons, self.category, text)
    }
}

impl Render for IconsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search = self.search.read(cx).text().to_string();
        let shown = matching(&self.icons, self.category, &search);
        let mut categories: Vec<SharedString> = vec![t_ctx("icons", "All", "Toutes").into()];
        categories.extend(icon_set::categories().into_iter().map(SharedString::from));
        let cells: Vec<AnyElement> = shown
            .iter()
            .map(|index| {
                let index = *index;
                let icon = &self.icons[index];
                let on = self.chosen.contains(&index);
                div()
                    .id(("icon", index))
                    .size(px(64.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .border_2()
                    .border_color(if on { c(theme::ACCENT) } else { gpui::transparent_black() })
                    .when(on, |d| d.bg(c(theme::TOGGLED)))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(thumbnail(icon, 40.0, 0x262626))
                    .tooltip(tooltip(icon.name))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, cx)))
                    .into_any_element()
            })
            .collect();
        let grid: AnyElement = if cells.is_empty() {
            div()
                .h(px(120.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(theme::TEXT_MUTED))
                .child(t("No icons match your search.", "Aucune icône ne correspond à votre recherche."))
                .into_any_element()
        } else {
            div().flex().flex_wrap().gap(px(4.)).p(px(4.)).children(cells).into_any_element()
        };
        let label = if self.chosen.is_empty() || self.replace {
            t_ctx("button", "Insert", "Insérer").to_string()
        } else {
            format!("{} ({})", t_ctx("button", "Insert", "Insérer"), self.chosen.len())
        };
        let modal = self.modal.clone();
        widgets::dialog(&self.modal, t("Icons", "Icônes"))
            .width(660.0)
            .on_ok(cx.listener(Self::insert))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(widgets::TextField::new(&self.search).width(628.0))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                ListBox::new("icon-categories", categories, Some(self.category.map_or(0, |category| category + 1)))
                                    .width(190.0)
                                    .height(340.0)
                                    .on_change(cx.listener(|this, index: &usize, _, cx| {
                                        this.category = index.checked_sub(1);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(430.))
                                    .border_1()
                                    .border_color(c(theme::BORDER))
                                    .bg(c(0xFFFFFF))
                                    .child(ScrollArea::new("icon-grid", 338.0).child(grid)),
                            ),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("icons-insert", label)
                            .primary()
                            .disabled(self.chosen.is_empty())
                            .on_click(cx.listener(Self::insert)),
                    )
                    .child(Button::new("icons-cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx))),
            )
    }
}

/// Open the Icons dialog; with `replace`, for the icon that takes the
/// place of the selected graphic.
pub(super) fn open_dialog(app: &mut ExcelApp, replace: bool, window: &mut Window, cx: &mut Cx) -> Option<Entity<IconsDialog>> {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return None;
    }
    let intent = match (replace, selected(app)) {
        (true, Some(drawing)) => Intent::EditObject(Some(object_id(&Drawings, drawing.id))),
        (true, None) => return None,
        (false, _) => Intent::EditObject(None),
    };
    if !app.may(intent, cx) {
        return None;
    }
    Some(app.open_modal(window, cx, move |modal, _, cx| IconsDialog::new(modal, replace, cx)))
}

// ---- the tab -------------------------------------------------------------

/// What a drop-down of the Graphics Styles group colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Fill,
    Outline,
}

/// The drop-down of Graphics Fill and Graphics Outline. A document does
/// not say in one place what its colours are, so no colour is shown as
/// the current one.
struct PaintMenu {
    app: WeakEntity<ExcelApp>,
    part: Part,
}

impl Render for PaintMenu {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let apply = |app: &WeakEntity<ExcelApp>, cx: &mut App, label: &'static str, change: Repaint| {
            app.update(cx, |app, cx| {
                app.popup = None;
                repaint_selected(app, label, change, cx);
                cx.notify();
            })
            .ok();
        };
        let (part, app) = (self.part, self.app.clone());
        let (none, label) = match part {
            Part::Fill => (t("No Fill", "Aucun remplissage"), Cmd::GraphicsFill.label()),
            Part::Outline => (t("No Outline", "Sans contour"), Cmd::GraphicsOutline.label()),
        };
        let grid = ColorGrid::new("graphics-colors", None)
            .none_label(none)
            .more(&self.app)
            .on_pick(move |color: &Option<Color>, _, cx| {
                let change = match part {
                    Part::Fill => Repaint::Fill(*color),
                    // In the units of the document: a hairline on an icon.
                    Part::Outline => Repaint::Outline(color.map(|color| (color, 0.5))),
                };
                apply(&app, cx, label, change);
            });
        div().p(px(8.)).text_size(px(12.)).child(grid)
    }
}

pub(super) fn open_paint(app: &mut ExcelApp, part: Part, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |_| PaintMenu { app: weak, part });
}

/// The groups of the "Graphics Format" tab.
pub(super) fn graphics_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t_ctx("graphic", "Change", "Modifier"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("gf-change", Cmd::ChangeGraphic, true, cx))
                .child(app.cmd_large("gf-convert", Cmd::ConvertToShape, false, cx)),
        )
        .into_any_element(),
        launcher_group(
            t("Graphics Styles", "Styles de graphiques"),
            rows(vec![
                app.cmd_labeled("gf-fill", Cmd::GraphicsFill, true, cx),
                app.cmd_labeled("gf-outline", Cmd::GraphicsOutline, true, cx),
                app.cmd_labeled("gf-effects", Cmd::GraphicsEffects, true, cx),
            ]),
            Some(app.launcher("gf-styles-launcher", t("Format Graphic", "Format graphique"), cx, |this, window, cx| {
                Cmd::FormatPicture.run(this, window, cx)
            })),
        )
        .into_any_element(),
        tabs::accessibility_group(app, cx).into_any_element(),
        tabs::arrange_group(app, cx).into_any_element(),
        tabs::size_group(app, false, window, cx).border_r_0().into_any_element(),
    ]
}

/// Files that are pictures, by their extension: what a drop on the cells
/// brings in.
pub(super) fn is_picture_file(extension: &str) -> bool {
    ["png", "jpg", "jpeg", "jpe", "gif", "bmp", "tif", "tiff", "webp", "svg"].contains(&extension.to_ascii_lowercase().as_str())
}

/// The bytes of a file as a workbook keeps them (see
/// `picture::importable`): an SVG document is kept as it is.
pub(super) fn importable(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if is_svg(&bytes) { Some(bytes) } else { picture::importable(bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_found_by_their_names() {
        let icons = icon_set::icons();
        let categories = icon_set::categories();
        assert!(icons.len() >= 60);
        // Every category has icons, every icon a category and a document.
        for (index, name) in categories.iter().enumerate() {
            assert!(icons.iter().any(|icon| icon.category == index), "{name}");
        }
        for icon in &icons {
            assert!(icon.category < categories.len(), "{}", icon.english);
            assert_eq!(svg_size(icon.svg().as_bytes()), Some((96.0, 96.0)), "{}", icon.english);
        }
        let names = |found: Vec<usize>| found.into_iter().map(|index| icons[index].english).collect::<Vec<_>>();
        assert_eq!(names(matching(&icons, None, "heart")), vec!["Heart"]);
        assert_eq!(names(matching(&icons, None, "  CHART  ")), vec!["Bar chart", "Pie chart"]);
        assert_eq!(names(matching(&icons, None, "chart pie")), vec!["Pie chart"]);
        assert!(matching(&icons, None, "zzz").is_empty());
        // A category narrows the search; nothing typed shows all of it.
        let arrows = categories.iter().position(|name| *name == "Arrows").unwrap();
        assert_eq!(matching(&icons, Some(arrows), "").len(), 3);
        assert!(matching(&icons, Some(arrows), "heart").is_empty());
        assert_eq!(matching(&icons, None, "").len(), icons.len());
        assert!(is_picture_file("SVG") && is_picture_file("jpeg") && !is_picture_file("xlsx"));
        assert_eq!(importable(b"<svg/>".to_vec()), Some(b"<svg/>".to_vec()));
        assert_eq!(importable(b"text".to_vec()), None);
    }
}
