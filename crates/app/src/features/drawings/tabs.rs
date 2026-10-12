//! The "Picture Format" and "Shape Format" tabs, the gallery of shapes and
//! the drop-downs of the tabs: fill, outline (colour, weight, dashes,
//! arrows), alignment.

use super::paint;
use super::{Cmd, Drawings, DrawingUi, edit, edit_shape, selected, tool};
use crate::app::{ExcelApp, MenuEntry, Popup, PopupKind};
use crate::features::{Command, Cx, Intent, object_id};
use crate::i18n::{Lang, lang, t, t_ctx};
use crate::ribbon::{group, launcher_group, row, rows};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use crate::widgets::{self, ColorGrid, NumberField};
use gpui::{
    AnyElement, AnyView, App, Context, Entity, Focusable, FontWeight, Subscription, WeakEntity, Window, div,
    point, prelude::*, px,
};
use opensheet_engine::Locale;
use opensheet_engine::drawing::{Dash, Drawing, LineEnd, Outline, ShapeTool};
use opensheet_engine::style::Color;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// What the ribbon keeps between repaints.
#[derive(Default)]
pub(super) struct TabState {
    size: Option<SizeFields>,
    /// The control whose drop-down was just closed, and when: the click
    /// that closed it must not open it again.
    closed: Rc<Cell<Option<(&'static str, Instant)>>>,
}

/// The Height and Width fields of the Size group.
struct SizeFields {
    height: Entity<TextInput>,
    width: Entity<TextInput>,
    /// The drawing and the size (in the unit shown) the fields were last
    /// filled with.
    shown: Option<(u32, f64, f64)>,
    _subscriptions: Vec<Subscription>,
}

// ------------------------------------------------------------------- sizes

/// The unit sizes are shown in, and how many pixels at 100 % it is:
/// inches in English, centimetres elsewhere.
pub(super) fn length_unit() -> (&'static str, f64) {
    match lang() {
        Lang::En => ("\"", 96.0),
        _ => ("cm", 96.0 / 2.54),
    }
}

/// A length in pixels at 100 % in the unit shown, to two decimals.
pub(super) fn in_units(pixels: f64) -> f64 {
    (pixels / length_unit().1 * 100.0).round() / 100.0
}

/// The size of a drawing after one of its sides was typed: the other side
/// follows when the proportions are locked.
pub(super) fn with_side(size: (f32, f32), lock_aspect: bool, height: bool, value: f32) -> (f32, f32) {
    let value = value.max(0.0);
    let (w, h) = size;
    match (height, lock_aspect && w > 0.0 && h > 0.0) {
        (true, true) => (w * value / h, value),
        (true, false) => (w, value),
        (false, true) => (value, h * value / w),
        (false, false) => (value, h),
    }
}

/// Give the selected drawing a height or a width, in the unit shown.
pub(super) fn set_side(app: &mut ExcelApp, height: bool, units: f64, cx: &mut Cx) {
    let pixels = (units * length_unit().1) as f32;
    edit(app, t("Size Object", "Dimensionner l'objet"), cx, |drawing| {
        drawing.size = with_side(drawing.size, drawing.lock_aspect, height, pixels);
    });
}

/// The size of the selected drawing in the unit shown: (id, width,
/// height).
fn shown_size(app: &ExcelApp) -> Option<(u32, f64, f64)> {
    let drawing = selected(app)?;
    let frame = app.wb.drawing_frame(app.sheet, drawing.id)?;
    Some((drawing.id, in_units(frame.width), in_units(frame.height)))
}

/// A number typed in a size field was confirmed (Return, Tab, leaving the
/// field) or stepped with the spinner.
fn on_size_event(app: &mut ExcelApp, height: bool, field: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Cx) {
    let confirmed = match event {
        InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => true,
        // The spinner leaves the whole number selected; typing does not.
        InputEvent::Changed => {
            let field = field.read(cx);
            !field.text().is_empty() && field.selection() == (0..field.text().len())
        }
        _ => false,
    };
    if !confirmed {
        return;
    }
    let typed = field.read(cx).number();
    let current = shown_size(app).map(|(_, width, height_now)| if height { height_now } else { width });
    if let (Some(typed), Some(current)) = (typed, current) {
        if (typed - current).abs() > 0.004 && typed > 0.0 {
            set_side(app, height, typed, cx);
        }
    }
    if matches!(event, InputEvent::Enter { .. }) {
        window.focus(&app.grid_focus);
    }
    // Whatever was typed, the fields show the size again.
    if let Some(fields) = &mut app.ext_mut::<DrawingUi>().tabs.size {
        fields.shown = None;
    }
    cx.notify();
}

/// The two fields (height, width), created on first use and filled with
/// the size of the selected drawing unless one of them is being typed in.
fn size_fields(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<(Entity<TextInput>, Entity<TextInput>)> {
    if app.ext_mut::<DrawingUi>().tabs.size.is_none() {
        let limit = 5000.0 / length_unit().1;
        let height = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let width = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let subscriptions = vec![
            cx.subscribe_in(&height, window, |app, field, event, window, cx| {
                on_size_event(app, true, field, event, window, cx)
            }),
            cx.subscribe_in(&width, window, |app, field, event, window, cx| {
                on_size_event(app, false, field, event, window, cx)
            }),
        ];
        app.ext_mut::<DrawingUi>().tabs.size = Some(SizeFields {
            height,
            width,
            shown: None,
            _subscriptions: subscriptions,
        });
    }
    let now = shown_size(app);
    let decimal = lang().locale().decimal_sep;
    let fields = app.ext_mut::<DrawingUi>().tabs.size.as_mut()?;
    let (height, width) = (fields.height.clone(), fields.width.clone());
    let typing = [&height, &width].iter().any(|field| field.read(cx).focus_handle(cx).is_focused(window));
    if fields.shown != now && !typing {
        fields.shown = now;
        let text = |value: Option<f64>| value.map(|v| widgets::number_text(v, decimal)).unwrap_or_default();
        height.update(cx, |field, cx| field.set_text(text(now.map(|n| n.2)), cx));
        width.update(cx, |field, cx| field.set_text(text(now.map(|n| n.1)), cx));
    }
    Some((height, width))
}

/// A size field with its label and its unit.
pub(super) fn length_field(id: &'static str, label: &'static str, field: &Entity<TextInput>) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(div().w(px(52.)).flex_none().text_size(px(11.5)).child(label))
        .child(NumberField::new(id, field).width(72.0))
        .child(div().text_color(c(theme::TEXT_MUTED)).child(length_unit().0))
}

fn size_group(app: &mut ExcelApp, crop: bool, window: &mut Window, cx: &mut Cx) -> gpui::Div {
    let fields: Vec<AnyElement> = size_fields(app, window, cx)
        .map(|(height, width)| {
            vec![
                length_field("dw-height", t("Height:", "Hauteur :"), &height).into_any_element(),
                length_field("dw-width", t("Width:", "Largeur :"), &width).into_any_element(),
            ]
        })
        .unwrap_or_default();
    launcher_group(
        t("Size", "Taille"),
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(crop, |d| d.child(app.cmd_large("pf-crop", Cmd::Crop, true, cx)))
            .child(rows(fields)),
        Some(app.launcher(
            "dw-size-launcher",
            t("Size and Properties", "Taille et propriétés"),
            cx,
            |this, window, cx| Cmd::Size.run(this, window, cx),
        )),
    )
}

// ---------------------------------------------------------------- the tabs

/// A button that acts at once, with a caret that opens the menu of its
/// variants (Bring Forward / Bring to Front).
fn split(app: &ExcelApp, id: &'static str, caret: &'static str, menu: [Cmd; 2], cx: &mut Cx) -> AnyElement {
    row(vec![
        app.cmd_labeled(id, menu[0], false, cx),
        app.caret_btn(caret, menu[0].label(), cx, move |this, window, cx| {
            let entries = menu.iter().map(|cmd| MenuEntry::command(*cmd, this)).collect();
            this.open_menu(id, entries, 240.0, window, cx);
        }),
    ])
}

/// The Arrange group of the two tabs.
fn arrange_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(
        t("Arrange", "Organiser"),
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(rows(vec![
                split(app, "dw-forward", "dw-forward-more", [Cmd::BringForward, Cmd::BringToFront], cx),
                split(app, "dw-backward", "dw-backward-more", [Cmd::SendBackward, Cmd::SendToBack], cx),
                app.cmd_labeled("dw-selection", Cmd::SelectionPane, false, cx),
            ]))
            .child(rows(vec![
                app.cmd_labeled("dw-align", Cmd::Align, true, cx),
                app.cmd_labeled("dw-group", Cmd::Group, true, cx),
                app.cmd_labeled("dw-rotate", Cmd::Rotate, true, cx),
            ])),
    )
}

fn accessibility_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(
        t("Accessibility", "Accessibilité"),
        row(vec![app.cmd_large("dw-alt", Cmd::AltText, false, cx)]),
    )
}

/// The groups of the "Picture Format" tab.
pub(super) fn picture_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Adjust", "Ajuster"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("pf-background", Cmd::RemoveBackground, false, cx))
                .child(app.cmd_large("pf-corrections", Cmd::Corrections, true, cx))
                .child(app.cmd_large("pf-color", Cmd::Color, true, cx))
                .child(app.cmd_large("pf-artistic", Cmd::ArtisticEffects, true, cx))
                .child(app.cmd_large("pf-transparency", Cmd::Transparency, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("pf-compress", Cmd::Compress, false, cx),
                    app.cmd_labeled("pf-change", Cmd::ChangePicture, true, cx),
                    app.cmd_labeled("pf-reset", Cmd::ResetPicture, true, cx),
                ])),
        )
        .into_any_element(),
        launcher_group(
            t("Picture Styles", "Styles d'image"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("pf-styles", Cmd::PictureStyles, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("pf-border", Cmd::PictureBorder, true, cx),
                    app.cmd_labeled("pf-effects", Cmd::PictureEffects, true, cx),
                ])),
            Some(app.launcher("pf-styles-launcher", t("Format Picture", "Format de l'image"), cx, |this, window, cx| {
                Cmd::FormatPicture.run(this, window, cx)
            })),
        )
        .into_any_element(),
        accessibility_group(app, cx).into_any_element(),
        arrange_group(app, cx).into_any_element(),
        size_group(app, true, window, cx).border_r_0().into_any_element(),
    ]
}

/// The shapes of the gallery, three rows of them: a click takes one in
/// hand.
fn mini_gallery(cx: &mut Cx) -> gpui::Div {
    let button = |index: usize, tool: ShapeTool, cx: &mut Cx| {
        div()
            .id(("dw-mini", index))
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .active(|s| s.bg(c(theme::PRESSED)))
            .child(paint::thumbnail(tool, 18.0))
            .tooltip(tooltip(tool_name(tool, lang().locale())))
            .on_click(cx.listener(move |app, _, window, cx| tool::arm(app, tool, window, cx)))
    };
    let tools = gallery_order();
    let per_row = tools.len().div_ceil(3);
    div()
        .flex()
        .flex_col()
        .p(px(1.))
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(theme::BORDER))
        .rounded(px(3.))
        .children(tools.chunks(per_row).enumerate().map(|(r, chunk)| {
            div()
                .flex()
                .children(chunk.iter().enumerate().map(|(i, tool)| button(r * per_row + i, *tool, cx)))
        }))
}

/// The groups of the "Shape Format" tab.
pub(super) fn shape_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Insert Shapes", "Insérer des formes"),
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(mini_gallery(cx))
                .child(rows(vec![
                    app.cmd_labeled("sf-shapes", Cmd::Shapes, true, cx),
                    app.cmd_labeled("sf-edit", Cmd::EditShape, true, cx),
                    app.cmd_labeled("sf-textbox", Cmd::TextBox, false, cx),
                ])),
        )
        .into_any_element(),
        launcher_group(
            t("Shape Styles", "Styles de formes"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sf-styles", Cmd::ShapeStyles, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sf-fill", Cmd::ShapeFill, true, cx),
                    app.cmd_labeled("sf-outline", Cmd::ShapeOutline, true, cx),
                    app.cmd_labeled("sf-effects", Cmd::ShapeEffects, true, cx),
                ])),
            Some(app.launcher("sf-styles-launcher", t("Format Shape", "Format de la forme"), cx, |this, window, cx| {
                Cmd::FormatShape.run(this, window, cx)
            })),
        )
        .into_any_element(),
        group(
            t("WordArt Styles", "Styles WordArt"),
            rows(vec![
                app.cmd_labeled("sf-text-fill", Cmd::TextFill, true, cx),
                app.cmd_labeled("sf-text-outline", Cmd::TextOutline, true, cx),
                app.cmd_labeled("sf-text-effects", Cmd::TextEffects, true, cx),
            ]),
        )
        .into_any_element(),
        accessibility_group(app, cx).into_any_element(),
        arrange_group(app, cx).into_any_element(),
        size_group(app, false, window, cx).border_r_0().into_any_element(),
    ]
}

// ------------------------------------------------------------- the gallery

/// The name of a shape of the gallery, as its tooltip shows it.
pub(super) fn tool_name(tool: ShapeTool, locale: &Locale) -> &'static str {
    match tool {
        ShapeTool::Line => t_ctx("shape", "Line", "Trait"),
        ShapeTool::Arrow => t("Line Arrow", "Flèche"),
        ShapeTool::DoubleArrow => t("Double Arrow", "Double flèche"),
        ShapeTool::TextBox => t("Text Box", "Zone de texte"),
        ShapeTool::Callout => t("Rectangular Callout", "Bulle rectangulaire"),
        other => other.name(locale),
    }
}

/// The sections of the gallery, as Excel orders them.
fn gallery_sections() -> Vec<(&'static str, Vec<ShapeTool>)> {
    use ShapeTool::*;
    vec![
        (t("Lines", "Lignes"), vec![Line, Arrow, DoubleArrow]),
        (t("Rectangles", "Rectangles"), vec![Rectangle, RoundedRectangle]),
        (
            t("Basic Shapes", "Formes de base"),
            vec![TextBox, Ellipse, Triangle, RightTriangle, Diamond, Pentagon, Hexagon],
        ),
        (t("Block Arrows", "Flèches pleines"), vec![RightArrow, LeftArrow, UpArrow, DownArrow]),
        (t("Stars and Banners", "Étoiles et bannières"), vec![Star5]),
        (t("Callouts", "Bulles et légendes"), vec![Callout]),
    ]
}

/// Every shape of the gallery, section after section.
fn gallery_order() -> Vec<ShapeTool> {
    gallery_sections().into_iter().flat_map(|(_, tools)| tools).collect()
}

/// The drop-down of Insert ▸ Shapes.
struct Gallery {
    app: WeakEntity<ExcelApp>,
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let locale = lang().locale();
        let mut index = 0usize;
        let mut sections: Vec<AnyElement> = Vec::new();
        for (title, tools) in gallery_sections() {
            sections.push(
                div()
                    .mt(px(4.))
                    .px(px(4.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .bg(c(theme::CHROME_BG))
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::TEXT_MUTED))
                    .child(title)
                    .into_any_element(),
            );
            let buttons: Vec<AnyElement> = tools
                .into_iter()
                .map(|tool| {
                    index += 1;
                    let app = self.app.clone();
                    div()
                        .id(("dw-gallery", index))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .active(|s| s.bg(c(theme::PRESSED)))
                        .child(paint::thumbnail(tool, 22.0))
                        .tooltip(tooltip(tool_name(tool, locale)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |app, cx| tool::arm(app, tool, window, cx)).ok();
                        })
                        .into_any_element()
                })
                .collect();
            sections.push(div().flex().flex_wrap().gap(px(2.)).py(px(2.)).children(buttons).into_any_element());
        }
        div().w(px(262.)).p(px(6.)).text_size(px(12.)).children(sections)
    }
}

/// Hang a view under the ribbon control whose command is running (or at
/// the pointer). A click on the control while its drop-down is open closes
/// it: the press closes it and the click must not open it again.
fn open_dropdown<V: Render>(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx, build: impl FnOnce(&mut Context<V>) -> V) {
    let anchor = app.command_anchor;
    let closed = app.ext_mut::<DrawingUi>().tabs.closed.clone();
    if let (Some((control, at)), Some(anchor)) = (closed.take(), anchor) {
        if control == anchor && at.elapsed() < Duration::from_millis(400) {
            return;
        }
    }
    let position = anchor
        .and_then(|id| app.anchors.borrow().get(id).copied())
        .map(|bounds| point(bounds.origin.x, bounds.origin.y + bounds.size.height + px(2.)))
        .unwrap_or_else(|| window.mouse_position());
    let view = cx.new(|cx| {
        if let Some(anchor) = anchor {
            cx.on_release(move |_, _| closed.set(Some((anchor, Instant::now())))).detach();
        }
        build(cx)
    });
    let view: AnyView = view.into();
    app.popup = Some(Popup::new(PopupKind::View(view), position, 0.0).max_height(600.0));
    cx.notify();
}

pub(super) fn open_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    open_dropdown(app, window, cx, |_| Gallery { app: weak });
}

// ------------------------------------------------------- fill and outline

/// What a colour drop-down colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Paint {
    Fill,
    Outline,
    Text,
}

/// The weights of the Weight menu, in points, and how Excel writes them.
pub(super) const WEIGHTS: [(f32, &str); 9] = [
    (0.25, "¼ pt"),
    (0.5, "½ pt"),
    (0.75, "¾ pt"),
    (1.0, "1 pt"),
    (1.5, "1½ pt"),
    (2.25, "2¼ pt"),
    (3.0, "3 pt"),
    (4.5, "4½ pt"),
    (6.0, "6 pt"),
];

/// The dashes of the Dashes menu and of the pane, with their names.
pub(super) fn dashes() -> [(Dash, &'static str); 8] {
    [
        (Dash::Solid, t("Solid", "Uni")),
        (Dash::SysDot, t("Round Dot", "Point rond")),
        (Dash::SysDash, t("Square Dot", "Point carré")),
        (Dash::Dash, t("Dash", "Tiret")),
        (Dash::DashDot, t("Dash Dot", "Tiret-point")),
        (Dash::LongDash, t("Long Dash", "Tiret cadratin")),
        (Dash::LongDashDot, t("Long Dash Dot", "Tiret cadratin-point")),
        (Dash::LongDashDotDot, t("Long Dash Dot Dot", "Tiret cadratin-point-point")),
    ]
}

/// The pairs of ends of the Arrows menu.
const ARROWS: [(LineEnd, LineEnd); 12] = [
    (LineEnd::None, LineEnd::None),
    (LineEnd::None, LineEnd::Triangle),
    (LineEnd::Triangle, LineEnd::None),
    (LineEnd::Triangle, LineEnd::Triangle),
    (LineEnd::None, LineEnd::Open),
    (LineEnd::Open, LineEnd::Open),
    (LineEnd::None, LineEnd::Stealth),
    (LineEnd::Stealth, LineEnd::Stealth),
    (LineEnd::Diamond, LineEnd::Triangle),
    (LineEnd::Oval, LineEnd::Triangle),
    (LineEnd::Diamond, LineEnd::Diamond),
    (LineEnd::Oval, LineEnd::Oval),
];

/// The outline a shape gets when one is asked for and it has none.
fn new_outline() -> Outline {
    Outline::solid(Color::BLACK, 1.0)
}

pub(super) fn set_fill(app: &mut ExcelApp, color: Option<Color>, cx: &mut Cx) {
    edit_shape(app, Cmd::ShapeFill.label(), cx, |shape| shape.fill = color);
}

pub(super) fn set_text_color(app: &mut ExcelApp, color: Option<Color>, cx: &mut Cx) {
    edit_shape(app, Cmd::TextFill.label(), cx, |shape| shape.font.color = color);
}

/// The colour of the outline; `None` takes the outline away.
pub(super) fn set_outline_color(app: &mut ExcelApp, color: Option<Color>, cx: &mut Cx) {
    edit_shape(app, Cmd::ShapeOutline.label(), cx, |shape| match color {
        Some(color) => shape.outline.get_or_insert_with(new_outline).color = color,
        None => shape.outline = None,
    });
}

/// Change the outline of the selected shape (giving it one if it has
/// none).
pub(super) fn set_outline(app: &mut ExcelApp, cx: &mut Cx, change: impl Fn(&mut Outline)) {
    edit_shape(app, Cmd::ShapeOutline.label(), cx, |shape| change(shape.outline.get_or_insert_with(new_outline)));
}

/// The part of the outline drop-down shown beside the colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Weight,
    Dashes,
    Arrows,
}

/// The drop-down of Shape Fill, Shape Outline and Text Fill.
struct ColorMenu {
    app: WeakEntity<ExcelApp>,
    paint: Paint,
    side: Option<Side>,
}

impl ColorMenu {
    /// Run a change on the application and close the drop-down.
    fn apply(app: &WeakEntity<ExcelApp>, cx: &mut App, change: impl FnOnce(&mut ExcelApp, &mut Cx)) {
        app.update(cx, |app, cx| {
            app.popup = None;
            change(app, cx);
            cx.notify();
        })
        .ok();
    }

    /// A row that opens a list beside the colours.
    fn side_row(&self, side: Side, label: &'static str, enabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let open = self.side == Some(side);
        div()
            .id(label)
            .h(px(24.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(3.))
            .when(open, |d| d.bg(c(theme::HOVER)))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .when(!enabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
            .child(label)
            .child(icon("chevron-right", 10.0, if enabled { theme::TEXT_MUTED } else { theme::TEXT_DISABLED }))
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if enabled && this.side != Some(side) {
                    this.side = Some(side);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.side = Some(side);
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    /// The list beside the colours: weights, dashes or arrows, each drawn
    /// as the line it gives.
    fn side_panel(&self, side: Side, current: Option<Outline>) -> AnyElement {
        let sample = |outline: Outline, width: f32| paint::line_sample(width, 14.0, outline);
        let base = Outline {
            color: Color(0x404040),
            ..current.unwrap_or_else(new_outline)
        };
        let item = |index: usize, on: bool, name: Option<&'static str>, content: AnyElement, change: Rc<dyn Fn(&mut Outline)>| {
            let app = self.app.clone();
            div()
                .id(("dw-outline-item", index))
                .h(px(24.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(3.))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(content)
                .when_some(name, |d, name| d.tooltip(tooltip(name)))
                .on_click(move |_, _, cx| {
                    let change = change.clone();
                    ColorMenu::apply(&app, cx, move |app, cx| set_outline(app, cx, |outline| change(outline)));
                })
                .into_any_element()
        };
        let items: Vec<AnyElement> = match side {
            Side::Weight => WEIGHTS
                .iter()
                .enumerate()
                .map(|(i, (points, label))| {
                    let points = *points;
                    let line = Outline {
                        width: points,
                        dash: Dash::Solid,
                        start: LineEnd::None,
                        end: LineEnd::None,
                        ..base
                    };
                    let content = div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().w(px(36.)).child(*label))
                        .child(sample(line, 96.0))
                        .into_any_element();
                    let on = current.is_some_and(|outline| outline.width == points);
                    item(i, on, None, content, Rc::new(move |outline: &mut Outline| outline.width = points))
                })
                .collect(),
            Side::Dashes => dashes()
                .into_iter()
                .enumerate()
                .map(|(i, (dash, name))| {
                    let line = Outline {
                        width: 1.5,
                        dash,
                        start: LineEnd::None,
                        end: LineEnd::None,
                        ..base
                    };
                    let on = current.is_some_and(|outline| outline.dash == dash);
                    item(i, on, Some(name), sample(line, 140.0).into_any_element(), Rc::new(move |outline: &mut Outline| outline.dash = dash))
                })
                .collect(),
            Side::Arrows => ARROWS
                .iter()
                .enumerate()
                .map(|(i, (start, end))| {
                    let (start, end) = (*start, *end);
                    let line = Outline {
                        width: 1.0,
                        dash: Dash::Solid,
                        start,
                        end,
                        ..base
                    };
                    let on = current.is_some_and(|outline| (outline.start, outline.end) == (start, end));
                    item(
                        i,
                        on,
                        None,
                        sample(line, 140.0).into_any_element(),
                        Rc::new(move |outline: &mut Outline| (outline.start, outline.end) = (start, end)),
                    )
                })
                .collect(),
        };
        div()
            .ml(px(6.))
            .pl(px(6.))
            .border_l_1()
            .border_color(c(theme::BORDER))
            .flex()
            .flex_col()
            .children(items)
            .into_any_element()
    }
}

impl Render for ColorMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // What the selected shape has now.
        let shape = self
            .app
            .upgrade()
            .and_then(|app| selected(app.read(cx)).and_then(Drawing::as_shape).cloned());
        let (current, none_label) = match self.paint {
            Paint::Fill => (shape.as_ref().map(|s| s.fill), t("No Fill", "Aucun remplissage")),
            Paint::Outline => (
                shape.as_ref().map(|s| s.outline.map(|outline| outline.color)),
                t("No Outline", "Sans contour"),
            ),
            Paint::Text => (shape.as_ref().map(|s| s.font.color), t("Automatic", "Automatique")),
        };
        let paint = self.paint;
        let app = self.app.clone();
        let grid = ColorGrid::new("dw-colors", current)
            .none_label(none_label)
            .more(&self.app)
            .on_pick(move |color: &Option<Color>, _, cx| {
                let color = *color;
                ColorMenu::apply(&app, cx, move |app, cx| match paint {
                    Paint::Fill => set_fill(app, color, cx),
                    Paint::Outline => set_outline_color(app, color, cx),
                    Paint::Text => set_text_color(app, color, cx),
                });
            });
        let outline = shape.as_ref().and_then(|s| s.outline);
        let is_line = shape.as_ref().is_some_and(|s| s.preset.is_line());
        let mut column = div().flex().flex_col().gap(px(2.)).child(grid);
        if paint == Paint::Outline {
            column = column
                .child(div().h(px(1.)).my(px(3.)).bg(c(theme::BORDER)))
                .child(self.side_row(Side::Weight, t("Weight", "Épaisseur"), true, cx))
                .child(self.side_row(Side::Dashes, t("Dashes", "Tirets"), true, cx))
                // Only a line has ends to put arrowheads on.
                .child(self.side_row(Side::Arrows, t("Arrows", "Flèches"), is_line, cx));
        }
        div()
            .p(px(8.))
            .flex()
            .text_size(px(12.))
            .child(column)
            .children(self.side.filter(|_| paint == Paint::Outline).map(|side| self.side_panel(side, outline)))
    }
}

pub(super) fn open_fill(app: &mut ExcelApp, paint: Paint, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    open_dropdown(app, window, cx, |_| ColorMenu {
        app: weak,
        paint,
        side: None,
    });
}

pub(super) fn open_outline(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    open_fill(app, Paint::Outline, window, cx);
}

// ------------------------------------------------------------------- align

/// The Align menu: aligning takes two selected objects and distributing
/// three; with fewer the entries are greyed out, as Excel shows them.
pub(super) fn open_align_menu(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let unavailable = |label: &'static str| MenuEntry::new(label, move |app, _, cx| app.not_implemented(label, cx));
    let gridlines = app.wb.sheets[app.sheet].show_gridlines;
    let entries = vec![
        MenuEntry::command(Cmd::AlignLeft, app),
        MenuEntry::command(Cmd::AlignCenter, app),
        MenuEntry::command(Cmd::AlignRight, app),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::AlignTop, app),
        MenuEntry::command(Cmd::AlignMiddle, app),
        MenuEntry::command(Cmd::AlignBottom, app),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::DistributeHorizontally, app),
        MenuEntry::command(Cmd::DistributeVertically, app),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::SnapToGrid, app).checked(app.snap_to_grid()),
        unavailable(t("Snap to Shape", "Aligner sur la forme")),
        MenuEntry::new(t("View Gridlines", "Afficher le quadrillage"), |app, _, cx| app.toggle_gridlines(cx)).checked(gridlines),
    ];
    app.open_command_menu(entries, 250.0, window, cx);
}

/// True when the sheet's protection lets the selected drawing be changed
/// (the panes grey their controls otherwise).
pub(super) fn editable(app: &ExcelApp) -> bool {
    selected(app).is_some_and(|drawing| {
        crate::features::guard(app, &Intent::EditObject(Some(object_id(&Drawings, drawing.id)))).is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_side_keeps_locked_proportions() {
        assert_eq!(with_side((200.0, 100.0), true, true, 50.0), (100.0, 50.0));
        assert_eq!(with_side((200.0, 100.0), true, false, 50.0), (50.0, 25.0));
        assert_eq!(with_side((200.0, 100.0), false, true, 50.0), (200.0, 50.0));
        assert_eq!(with_side((200.0, 100.0), false, false, 50.0), (50.0, 100.0));
        // A level line has no height to keep in proportion.
        assert_eq!(with_side((96.0, 0.0), true, false, 48.0), (48.0, 0.0));
        assert_eq!(with_side((96.0, 10.0), false, true, -5.0), (96.0, 0.0));
    }

    #[test]
    fn the_gallery_holds_every_shape_once() {
        let mut shown = gallery_order();
        assert_eq!(shown.len(), ShapeTool::ALL.len());
        shown.sort_by_key(|tool| ShapeTool::ALL.iter().position(|other| other == tool));
        assert_eq!(shown, ShapeTool::ALL.to_vec());
        // Lines first, then rectangles, as in Excel's gallery.
        assert_eq!(gallery_order()[0], ShapeTool::Line);
        assert_eq!(gallery_order()[3], ShapeTool::Rectangle);
        // Every dash of the menu is a different one, the solid line first.
        let listed: Vec<Dash> = dashes().iter().map(|(dash, _)| *dash).collect();
        assert_eq!(listed[0], Dash::Solid);
        assert!(listed.iter().enumerate().all(|(i, dash)| !listed[..i].contains(dash)));
        assert!(WEIGHTS.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(ARROWS[0], (LineEnd::None, LineEnd::None));
    }
}
