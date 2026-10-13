//! The "Slicer" and "Timeline" tabs: the caption box, the settings and
//! connections buttons, the gallery of styles, and the fields giving the
//! number of columns and the sizes.

use super::logic::{self, SLICER_STYLES, TIMELINE_STYLES};
use super::{Cmd, SlicerUi, edit_slicer, edit_timeline, items_of, selected_slicer, selected_timeline};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::i18n::{Lang, lang, t};
use crate::ribbon::{group, row, rows};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{self, NumberField, Sketch, TextField, icon, preview, tooltip};
use gpui::{
    AnyElement, Context, Entity, Focusable, FontWeight, Pixels, Point, SharedString, Subscription, WeakEntity, Window, div,
    point, prelude::*, px,
};
use opensheet_engine::theme::Theme;
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// Control of the ribbon the gallery of styles hangs from.
const ANCHOR: &str = "sl-styles";
const THUMB_W: f32 = 46.0;
const THUMB_H: f32 = 40.0;

/// A field of the two tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum FieldKind {
    Caption,
    Columns,
    ButtonHeight,
    ButtonWidth,
    Height,
    Width,
}

struct Field {
    input: Entity<TextInput>,
    /// What the field was last filled with.
    shown: Option<String>,
    _subscription: Subscription,
}

/// What the tabs remember between events.
#[derive(Default)]
pub(super) struct TabState {
    fields: HashMap<FieldKind, Field>,
    /// Where the press that closed the gallery landed: on its own button,
    /// it must not open it again.
    gallery_closed: Rc<Cell<Option<Point<Pixels>>>>,
}

/// The unit sizes are shown in, and how many pixels at 100 % it is:
/// inches in English, centimetres elsewhere.
fn length_unit() -> (&'static str, f64) {
    match lang() {
        Lang::En => ("\"", 96.0),
        _ => ("cm", 96.0 / 2.54),
    }
}

fn in_units(pixels: f32) -> f64 {
    (pixels as f64 / length_unit().1 * 100.0).round() / 100.0
}

/// What a field shows for the selected slicer or timeline.
pub(super) fn field_value(app: &ExcelApp, kind: FieldKind) -> Option<String> {
    let decimal = lang().locale().decimal_sep;
    let length = |pixels: f32| widgets::number_text(in_units(pixels), decimal);
    if let Some(timeline) = selected_timeline(app) {
        return match kind {
            FieldKind::Caption => Some(timeline.caption.clone()),
            FieldKind::Height => Some(length(timeline.size.1)),
            FieldKind::Width => Some(length(timeline.size.0)),
            _ => None,
        };
    }
    let slicer = selected_slicer(app)?;
    Some(match kind {
        FieldKind::Caption => slicer.caption.clone(),
        FieldKind::Columns => slicer.columns.to_string(),
        FieldKind::ButtonHeight => length(slicer.button_height),
        FieldKind::ButtonWidth => length(logic::button_width(slicer.size.0, slicer.columns, scrolling(app, slicer.id))),
        FieldKind::Height => length(slicer.size.1),
        FieldKind::Width => length(slicer.size.0),
    })
}

/// Whether a slicer shows its scroll bar.
fn scrolling(app: &ExcelApp, id: u32) -> bool {
    let Some((_, slicer)) = app.wb.slicer(id) else {
        return false;
    };
    let items = items_of(app, id).len();
    logic::slicer_layout(slicer.size, slicer.show_header, slicer.columns, slicer.button_height, items, 0).scroll.is_some()
}

/// Store what was typed in a field. Sizes are in the unit shown.
pub(super) fn set_field(app: &mut ExcelApp, kind: FieldKind, text: &str, cx: &mut Cx) {
    let number = text.trim().replace(lang().locale().decimal_sep, ".").parse::<f64>().ok().filter(|n| n.is_finite());
    let pixels = number.map(|n| (n * length_unit().1) as f32);
    if selected_timeline(app).is_some() {
        match (kind, pixels) {
            (FieldKind::Caption, _) => {
                let caption = text.to_string();
                edit_timeline(app, t("Timeline Caption", "Légende de la chronologie"), cx, |tl| tl.caption = caption);
            }
            (FieldKind::Height, Some(h)) if h >= 70.0 => {
                edit_timeline(app, t("Resize Timeline", "Redimensionner la chronologie"), cx, |tl| tl.size.1 = h)
            }
            (FieldKind::Width, Some(w)) if w >= 150.0 => {
                edit_timeline(app, t("Resize Timeline", "Redimensionner la chronologie"), cx, |tl| tl.size.0 = w)
            }
            _ => {}
        }
        return;
    }
    let Some(id) = selected_slicer(app).map(|s| s.id) else {
        return;
    };
    let resize = t("Resize Slicer", "Redimensionner le segment");
    match (kind, pixels) {
        (FieldKind::Caption, _) => {
            let caption = text.to_string();
            edit_slicer(app, t("Slicer Caption", "Légende du segment"), cx, |s| s.caption = caption);
        }
        (FieldKind::Columns, _) => {
            if let Some(columns) = number.filter(|n| *n >= 1.0) {
                edit_slicer(app, t("Columns", "Colonnes"), cx, |s| s.columns = columns.min(100.0) as u32);
            }
        }
        (FieldKind::ButtonHeight, Some(h)) if h >= 4.0 => {
            edit_slicer(app, t("Button Height", "Hauteur des boutons"), cx, |s| s.button_height = h.min(2000.0))
        }
        (FieldKind::ButtonWidth, Some(w)) if w >= 4.0 => {
            // The buttons fill the slicer: it takes the width they need.
            let scrolling = scrolling(app, id);
            edit_slicer(app, t("Button Width", "Largeur des boutons"), cx, |s| {
                s.size.0 = logic::width_for_buttons(w, s.columns, scrolling)
            });
        }
        (FieldKind::Height, Some(h)) if h >= 50.0 => edit_slicer(app, resize, cx, |s| s.size.1 = h),
        (FieldKind::Width, Some(w)) if w >= 60.0 => edit_slicer(app, resize, cx, |s| s.size.0 = w),
        _ => {}
    }
}

/// Something happened in a field: what was typed is stored on Return, on
/// Tab, on leaving the field, and at each step of a spinner.
fn on_field_event(app: &mut ExcelApp, kind: FieldKind, field: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Cx) {
    let confirmed = match event {
        InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => true,
        // The spinner leaves the whole number selected; typing does not.
        InputEvent::Changed if kind != FieldKind::Caption => {
            let field = field.read(cx);
            !field.text().is_empty() && field.selection() == (0..field.text().len())
        }
        _ => false,
    };
    if !confirmed {
        return;
    }
    let typed = field.read(cx).text().to_string();
    if field_value(app, kind).is_some_and(|current| current != typed) {
        set_field(app, kind, &typed, cx);
    }
    if matches!(event, InputEvent::Enter { .. }) {
        window.focus(&app.grid_focus);
    }
    // Whatever was typed, the field shows the value again.
    if let Some(field) = app.ext_mut::<SlicerUi>().tabs.fields.get_mut(&kind) {
        field.shown = None;
    }
    cx.notify();
}

/// A field, created on first use and filled with the value of the
/// selected object unless it is being typed in.
fn field(app: &mut ExcelApp, kind: FieldKind, window: &mut Window, cx: &mut Cx) -> Entity<TextInput> {
    if !app.ext_mut::<SlicerUi>().tabs.fields.contains_key(&kind) {
        let input = match kind {
            FieldKind::Caption => widgets::input(cx, ""),
            FieldKind::Columns => widgets::number_input(cx, None, 1.0, 100.0, 1.0),
            _ => widgets::number_input(cx, None, 0.0, 5000.0 / length_unit().1, if lang() == Lang::En { 0.05 } else { 0.1 }),
        };
        let subscription = cx.subscribe_in(&input, window, move |app, field, event, window, cx| {
            on_field_event(app, kind, field, event, window, cx)
        });
        app.ext_mut::<SlicerUi>().tabs.fields.insert(
            kind,
            Field {
                input,
                shown: None,
                _subscription: subscription,
            },
        );
    }
    let now = field_value(app, kind);
    let state = app.ext_mut::<SlicerUi>().tabs.fields.get_mut(&kind).expect("the field was just created");
    let input = state.input.clone();
    if state.shown != now && !input.read(cx).focus_handle(cx).is_focused(window) {
        state.shown = now.clone();
        input.update(cx, |field, cx| {
            field.set_text(now.unwrap_or_default(), cx);
            field.set_selection(0..0, cx);
        });
    }
    input
}

fn caption_box(app: &mut ExcelApp, title: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let input = field(app, FieldKind::Caption, window, cx);
    vec![
        div().px(px(4.)).whitespace_nowrap().child(title).into_any_element(),
        div().px(px(2.)).child(TextField::new(&input).width(140.0)).into_any_element(),
    ]
}

/// A number field with its label and, for a length, its unit.
fn number_row(app: &mut ExcelApp, id: &'static str, title: &'static str, kind: FieldKind, window: &mut Window, cx: &mut Cx) -> AnyElement {
    let input = field(app, kind, window, cx);
    let unit = if kind == FieldKind::Columns { "" } else { length_unit().0 };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(div().w(px(62.)).flex_none().text_size(px(11.5)).child(title))
        // Three of these rows fit in a group of the ribbon.
        .child(NumberField::new(id, &input).width(72.0).height(20.0))
        .child(div().w(px(20.)).flex_none().whitespace_nowrap().text_color(c(theme::TEXT_MUTED)).child(unit))
        .into_any_element()
}

fn size_group(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> AnyElement {
    group(
        t("Size", "Taille"),
        rows(vec![
            number_row(app, "sl-height", t("Height:", "Hauteur :"), FieldKind::Height, window, cx),
            number_row(app, "sl-width", t("Width:", "Largeur :"), FieldKind::Width, window, cx),
        ]),
    )
    .into_any_element()
}

/// The groups of the "Slicer" tab.
pub(super) fn slicer_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let mut caption = caption_box(app, t("Slicer Caption:", "Légende du segment :"), window, cx);
    caption.push(app.cmd_labeled("sl-settings", Cmd::SlicerSettings, false, cx));
    let gallery = ribbon_gallery(app, cx);
    vec![
        group(
            t("Slicer", "Segment"),
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(rows(caption))
                .child(app.cmd_large("sl-connections", Cmd::ReportConnections, false, cx)),
        )
        .into_any_element(),
        group(t("Slicer Styles", "Styles de segments"), gallery).into_any_element(),
        app.arrange_group("ar", cx).into_any_element(),
        group(
            t("Buttons", "Boutons"),
            rows(vec![
                number_row(app, "sl-columns", t("Columns:", "Colonnes :"), FieldKind::Columns, window, cx),
                number_row(app, "sl-button-height", t("Height:", "Hauteur :"), FieldKind::ButtonHeight, window, cx),
                number_row(app, "sl-button-width", t("Width:", "Largeur :"), FieldKind::ButtonWidth, window, cx),
            ]),
        )
        .into_any_element(),
        size_group(app, window, cx),
    ]
}

/// The groups of the "Timeline" tab.
pub(super) fn timeline_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let caption = caption_box(app, t("Timeline Caption:", "Légende de la chronologie :"), window, cx);
    let gallery = ribbon_gallery(app, cx);
    vec![
        group(
            t("Timeline", "Chronologie"),
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(rows(caption))
                .child(app.cmd_large("sl-connections", Cmd::ReportConnections, false, cx)),
        )
        .into_any_element(),
        group(t("Timeline Styles", "Styles de chronologies"), gallery).into_any_element(),
        app.arrange_group("ar", cx).into_any_element(),
        size_group(app, window, cx),
        group(
            t("Show", "Afficher"),
            row(vec![
                rows(vec![
                    app.cmd_check("tl-header", Cmd::ShowHeader, cx),
                    app.cmd_check("tl-scrollbar", Cmd::ShowScrollbar, cx),
                ])
                .into_any_element(),
                rows(vec![
                    app.cmd_check("tl-label", Cmd::ShowSelectionLabel, cx),
                    app.cmd_check("tl-level", Cmd::ShowTimeLevel, cx),
                ])
                .into_any_element(),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

// ------------------------------------------------------------------ gallery

/// A small slicer in the colours of a style: its header and four buttons,
/// two of them selected.
fn draw_slicer(s: &mut Sketch, theme: &Theme, name: &str) {
    let look = logic::slicer_look(theme, name);
    s.rect(0.0, 0.0, s.width, s.height, c(look.background));
    if let Some(fill) = look.header_fill {
        s.rect(0.0, 0.0, s.width, 9.0, c(fill));
    }
    s.rect(4.0, 4.0, s.width * 0.55, 6.0, c(look.header_text));
    s.rect(3.0, 9.0, s.width - 3.0, 10.0, c(look.header_rule));
    for k in 0..4 {
        let tile = look.tile(k < 2, true);
        let y = 13.0 + k as f32 * 6.5;
        s.rect(4.0, y, s.width - 4.0, y + 5.0, c(tile.border));
        s.rect(5.0, y + 1.0, s.width - 5.0, y + 4.0, c(tile.fill));
    }
    s.frame(0.0, 0.0, s.width, s.height, 1.0, c(look.border));
}

/// A small timeline in the colours of a style.
fn draw_timeline(s: &mut Sketch, theme: &Theme, name: &str) {
    let look = logic::timeline_look(theme, name);
    s.rect(0.0, 0.0, s.width, s.height, c(look.background));
    s.rect(4.0, 5.0, s.width * 0.5, 7.0, c(look.header_text));
    s.rect(4.0, 12.0, s.width * 0.35, 14.0, c(look.label_text));
    let tile = (s.width - 8.0) / 5.0;
    for k in 0..5 {
        let x = 4.0 + k as f32 * tile;
        s.rect(x + 1.0, 19.0, x + tile - 1.0, 20.5, c(look.period_text));
        s.rect(x + 0.5, 24.0, x + tile - 0.5, 31.0, c(if (1..=2).contains(&k) { look.selected } else { look.unselected }));
    }
    s.frame(0.0, 0.0, s.width, s.height, 1.0, c(look.border));
}

fn thumbnail(index: usize, name: &'static str, theme: &Rc<Theme>, current: bool, on_pick: impl Fn(&mut Window, &mut gpui::App) + 'static) -> AnyElement {
    let title: SharedString = logic::style_title(name, lang().locale()).into();
    let timeline = name.starts_with("Time");
    let theme = theme.clone();
    div()
        .id(("slicer-style", index))
        .p(px(2.))
        .border_1()
        .border_color(if current { c(theme::ACCENT) } else { gpui::transparent_black() })
        .rounded(px(2.))
        .hover(|s| s.border_color(c(0xF2994A)))
        .child(preview(THUMB_W, THUMB_H, move |s| if timeline { draw_timeline(s, &theme, name) } else { draw_slicer(s, &theme, name) }))
        .tooltip(tooltip(title))
        .on_click(move |_, window, cx| on_pick(window, cx))
        .into_any_element()
}

/// The styles of the selected object, and the one it has.
fn styles_of(app: &ExcelApp) -> Option<(&'static [&'static str], String)> {
    if let Some(timeline) = selected_timeline(app) {
        return Some((&TIMELINE_STYLES, timeline.style.clone()));
    }
    selected_slicer(app).map(|s| (&SLICER_STYLES[..], s.style.clone()))
}

/// Give the selected slicer or timeline a style.
pub(super) fn set_style(app: &mut ExcelApp, name: &str, cx: &mut Cx) {
    let style = name.to_string();
    if selected_timeline(app).is_some() {
        edit_timeline(app, t("Timeline Style", "Style de chronologie"), cx, |tl| tl.style = style);
    } else {
        edit_slicer(app, t("Slicer Style", "Style de segment"), cx, |s| s.style = style);
    }
}

/// The gallery as the tab shows it: the first styles and the button that
/// opens them all. A narrow window only keeps the button.
fn ribbon_gallery(app: &mut ExcelApp, cx: &mut Cx) -> AnyElement {
    let Some((names, current)) = styles_of(app).filter(|_| app.ribbon_level < 3) else {
        return app.cmd_large(ANCHOR, Cmd::Styles, true, cx);
    };
    // The row holding the current style.
    let at = names.iter().position(|name| *name == current).unwrap_or(0);
    let first = at / 6 * 6;
    let weak = cx.weak_entity();
    // The thumbnails are in the colours of the workbook's theme.
    let theme = Rc::new(app.wb.theme().clone());
    let thumbs: Vec<AnyElement> = names[first..(first + 6).min(names.len())]
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let weak = weak.clone();
            thumbnail(first + k, name, &theme, *name == current, move |_, cx| {
                weak.update(cx, |app, cx| set_style(app, name, cx)).ok();
            })
        })
        .collect();
    div()
        .flex()
        .items_center()
        .gap(px(1.))
        .child(
            div()
                .p(px(2.))
                .flex()
                .gap(px(1.))
                .bg(c(0xFFFFFF))
                .border_1()
                .border_color(c(theme::BORDER))
                .rounded(px(3.))
                .children(thumbs),
        )
        .child(app.small(
            ANCHOR,
            icon("chevron-down", 10.0, theme::TEXT_MUTED),
            Cmd::Styles.label(),
            false,
            cx,
            |app, window, cx| open_gallery(app, window, cx),
        ))
        .into_any_element()
}

/// The drop-down with every style.
struct Gallery {
    app: WeakEntity<ExcelApp>,
    names: &'static [&'static str],
    current: String,
    theme: Rc<Theme>,
    closed: Rc<Cell<Option<Point<Pixels>>>>,
}

pub(super) fn open_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    // The press of this very click may have just closed the gallery.
    let closed = app.ext_mut::<SlicerUi>().tabs.gallery_closed.clone();
    let at = window.mouse_position();
    if closed.take().is_some_and(|press| (press.x - at.x).abs() < px(4.) && (press.y - at.y).abs() < px(4.)) {
        return;
    }
    let Some((names, current)) = styles_of(app) else {
        return;
    };
    let position = match app.anchors.borrow().get(ANCHOR) {
        Some(b) => point((b.right() - px(6.0 * (THUMB_W + 8.0) + 30.0)).max(px(6.)), b.bottom() + px(2.)),
        None => at,
    };
    let weak = cx.weak_entity();
    let theme = Rc::new(app.wb.theme().clone());
    let view = cx.new(|_| Gallery {
        app: weak,
        names,
        current,
        theme,
        closed,
    });
    app.open_popup_view(position, view, cx);
}

impl Gallery {
    fn section(&self, title: &'static str, family: &[&'static str], cx: &mut Context<Self>) -> AnyElement {
        let listed: Vec<(usize, &'static str)> = self
            .names
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, name)| family.contains(&logic::style_parts(name).0))
            .collect();
        let rows: Vec<AnyElement> = listed
            .chunks(6)
            .map(|chunk| {
                div()
                    .flex()
                    .gap(px(2.))
                    .children(chunk.iter().map(|(index, name)| {
                        let (app, name) = (self.app.clone(), *name);
                        thumbnail(100 + index, name, &self.theme, name == self.current, move |window, cx| {
                            app.update(cx, |app, cx| {
                                app.close_popup(window, cx);
                                set_style(app, name, cx);
                            })
                            .ok();
                        })
                    }))
                    .into_any_element()
            })
            .collect();
        let _ = cx;
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .h(px(22.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .bg(c(theme::CHROME_BG))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(rows)
            .into_any_element()
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let closed = self.closed.clone();
        div()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.))
            .on_mouse_down_out(move |ev, _, _| closed.set(Some(ev.position)))
            .child(self.section(t("Light", "Clair"), &["Light", "Other"], cx))
            .child(self.section(t("Dark", "Foncé"), &["Dark"], cx))
    }
}
