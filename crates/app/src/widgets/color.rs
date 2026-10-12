//! Colour pickers: the grid of theme and standard colours, the button that
//! drops it down, and the "More Colors" dialog.

use super::{
    Button, CONTROL_H, DropState, FIELD_BORDER, Handler, NumberField, TextField, dialog, focus_of, icon, input, label,
    labeled, number_input, plain,
};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::shell::Modal;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use gpui::{
    AnyElement, App, Context, ElementId, Entity, FontWeight, IntoElement, KeyDownEvent, RenderOnce, SharedString,
    WeakEntity, Window, anchored, deferred, div, prelude::*, px, relative,
};
use opensheet_engine::style::Color;
use std::rc::Rc;
use std::time::Instant;

const SWATCH: f32 = 18.0;
const GAP: f32 = 2.0;

/// The grid of colours of Excel's pickers: theme colours with their
/// lighter and darker variants, the standard colours, optionally a "no
/// colour" line on top and "More Colors…" at the bottom.
///
/// One Tab stop: the arrows move over the swatches, Space picks.
///
/// ```ignore
/// ColorGrid::new("fill", Some(self.fill))            // Option<Option<Color>>: None when the cells disagree
///     .none_label(t("No Color", "Aucune couleur"))
///     .more(&self.modal.app)
///     .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| { this.fill = *color; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct ColorGrid {
    id: ElementId,
    /// `Some(None)`: "no colour" is the current choice.
    selected: Option<Option<Color>>,
    none_label: Option<SharedString>,
    app: Option<WeakEntity<ExcelApp>>,
    on_pick: Option<Handler<Option<Color>>>,
}

impl ColorGrid {
    pub fn new(id: impl Into<ElementId>, selected: Option<Option<Color>>) -> ColorGrid {
        ColorGrid {
            id: id.into(),
            selected,
            none_label: None,
            app: None,
            on_pick: None,
        }
    }

    /// Offer "no colour" under this name ("No Color", "Automatic").
    pub fn none_label(mut self, text: impl Into<SharedString>) -> Self {
        self.none_label = Some(text.into());
        self
    }

    /// Offer "More Colors…", which opens a dialog over the application.
    pub fn more(mut self, app: &WeakEntity<ExcelApp>) -> Self {
        self.app = Some(app.clone());
        self
    }

    /// A colour was picked (`None`: no colour).
    pub fn on_pick(mut self, f: impl Fn(&Option<Color>, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

/// Open the "More Colors" dialog; `on_pick` gets the colour chosen.
fn open_more_colors(app: &WeakEntity<ExcelApp>, current: Option<Color>, on_pick: Handler<Option<Color>>, window: &mut Window, cx: &mut App) {
    app.update(cx, |app, cx| {
        app.open_modal(window, cx, move |modal, _, cx| MoreColors::new(modal, current, on_pick, cx));
    })
    .ok();
}

impl RenderOnce for ColorGrid {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        // Keyboard cursor over the swatches: (row, column).
        let cursor = window.use_keyed_state(self.id.clone(), cx, |_, _| (0usize, 0usize));
        let at = *cursor.read(cx);
        // The colours of the theme of the workbook, as references to it;
        // the standard colours are the same everywhere.
        let mut rows = theme::palette_rows();
        rows.push(theme::PALETTE_STANDARD.map(Color));
        let rows = Rc::new(rows);
        let selected = self.selected;
        let handler = self.on_pick;
        let pick = {
            let handler = handler.clone();
            move |color: Option<Color>, window: &mut Window, cx: &mut App| {
                if let Some(handler) = &handler {
                    handler(&color, window, cx);
                }
            }
        };

        let swatch = |row: usize, col: usize, color: Color| {
            let pick = pick.clone();
            let on = selected == Some(Some(color));
            let under_cursor = focused && at == (row, col);
            div()
                .id(("swatch", row * 16 + col))
                .size(px(SWATCH))
                .flex_none()
                .bg(c(color.value()))
                .border_1()
                .border_color(c(if on || under_cursor { 0xE8590C } else { 0xD0D0D0 }))
                .when(on, |d| d.border_2())
                .hover(|s| s.border_color(c(0xF2994A)))
                .on_click(move |_, window, cx| pick(Some(color), window, cx))
        };
        let heading = |text: &'static str| {
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(theme::TEXT_MUTED))
                .pt(px(2.))
                .child(text)
        };
        let line = |row: usize| {
            div()
                .flex()
                .gap(px(GAP))
                .children(rows[row].iter().enumerate().map(|(col, color)| swatch(row, col, *color)))
        };

        let none = self.none_label.map(|text| {
            let pick = pick.clone();
            let on = selected == Some(None);
            div()
                .id("no-color")
                .h(px(22.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(2.))
                .rounded(px(3.))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(
                    div()
                        .size(px(14.))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .bg(c(0xFFFFFF)),
                )
                .child(text)
                .on_click(move |_, window, cx| pick(None, window, cx))
        });
        let more = self.app.zip(handler.clone()).map(|(app, handler)| {
            let current = selected.flatten();
            div()
                .id("more-colors")
                .h(px(22.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(2.))
                .rounded(px(3.))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(icon("fill", 13.0, theme::TEXT_MUTED))
                .child(t("More Colors…", "Autres couleurs…"))
                .on_click(move |_, window, cx| open_more_colors(&app, current, handler.clone(), window, cx))
        });

        let key_rows = rows.clone();
        let key_pick = pick.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if !plain(ev) {
                    return;
                }
                let (row, col) = at;
                let next = match ev.keystroke.key.as_str() {
                    "left" => (row, col.saturating_sub(1)),
                    "right" => (row, (col + 1).min(9)),
                    "up" => (row.saturating_sub(1), col),
                    "down" => ((row + 1).min(key_rows.len() - 1), col),
                    "space" => {
                        key_pick(Some(key_rows[row][col]), window, cx);
                        cx.stop_propagation();
                        return;
                    }
                    _ => return,
                };
                cursor.update(cx, |at, cx| {
                    *at = next;
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .children(none)
            .child(heading(t("Theme Colors", "Couleurs du thème")))
            .child(line(0))
            .child(div().h(px(2.)))
            .children((1..6).map(line))
            .child(heading(t("Standard Colors", "Couleurs standard")))
            .child(line(6))
            .children(more)
    }
}

/// A button showing a colour that drops a [`ColorGrid`] down.
///
/// ```ignore
/// ColorButton::new("font-color", self.color)        // Option<Option<Color>>
///     .none_label(t("Automatic", "Automatique"))
///     .more(&self.modal.app)
///     .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| { this.color = Some(*color); cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct ColorButton {
    id: ElementId,
    selected: Option<Option<Color>>,
    none_label: Option<SharedString>,
    app: Option<WeakEntity<ExcelApp>>,
    width: f32,
    on_pick: Option<Handler<Option<Color>>>,
}

impl ColorButton {
    /// `selected`: `None` when the cells disagree, `Some(None)` for "no
    /// colour" / automatic.
    pub fn new(id: impl Into<ElementId>, selected: Option<Option<Color>>) -> ColorButton {
        ColorButton {
            id: id.into(),
            selected,
            none_label: None,
            app: None,
            width: 120.0,
            on_pick: None,
        }
    }

    pub fn none_label(mut self, text: impl Into<SharedString>) -> Self {
        self.none_label = Some(text.into());
        self
    }

    pub fn more(mut self, app: &WeakEntity<ExcelApp>) -> Self {
        self.app = Some(app.clone());
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&Option<Color>, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for ColorButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| DropState::new());
        let open = state.read(cx).open;
        let close = {
            let state = state.clone();
            move |by_outside_press: bool, cx: &mut App| {
                state.update(cx, |s, cx| {
                    s.open = false;
                    s.closed_at = by_outside_press.then(Instant::now);
                    cx.notify();
                });
            }
        };
        let toggle = {
            let state = state.clone();
            move |cx: &mut App| {
                state.update(cx, |s, cx| {
                    if s.just_closed() {
                        s.closed_at = None;
                        return;
                    }
                    s.open = !s.open;
                    cx.notify();
                });
            }
        };
        let panel = open.then(|| {
            let handler = self.on_pick.clone();
            let picked = close.clone();
            let mut grid = ColorGrid::new("grid", self.selected).on_pick(move |color, window, cx| {
                picked(false, cx);
                if let Some(handler) = &handler {
                    handler(color, window, cx);
                }
            });
            if let Some(text) = self.none_label.clone() {
                grid = grid.none_label(text);
            }
            if let Some(app) = &self.app {
                grid = grid.more(app);
            }
            let dismiss = close.clone();
            div().absolute().left_0().top(relative(1.)).child(
                deferred(
                    anchored().snap_to_window_with_margin(px(6.)).child(
                        div()
                            .id("color-panel")
                            .occlude()
                            .mt(px(1.))
                            .p(px(8.))
                            .bg(c(0xFFFFFF))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .rounded(px(4.))
                            .shadow_md()
                            .text_size(px(12.))
                            .text_color(c(theme::TEXT))
                            .on_mouse_down_out(move |_, _, cx| dismiss(true, cx))
                            .child(grid),
                    ),
                )
                .with_priority(3),
            )
        });
        let swatch: AnyElement = match self.selected {
            Some(Some(color)) => div()
                .flex_1()
                .h(px(12.))
                .bg(c(color.value()))
                .border_1()
                .border_color(c(0xD0D0D0))
                .into_any_element(),
            Some(None) => div()
                .flex_1()
                .truncate()
                .child(self.none_label.clone().unwrap_or_default())
                .into_any_element(),
            None => div().flex_1().into_any_element(),
        };
        let key_toggle = toggle.clone();
        let dismiss = close.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .relative()
            .flex_none()
            .w(px(self.width))
            .on_key_down(move |ev: &KeyDownEvent, _, cx| {
                if plain(ev) && matches!(ev.keystroke.key.as_str(), "space" | "down") && !open {
                    key_toggle(cx);
                    cx.stop_propagation();
                }
            })
            .on_action(move |_: &super::Dismiss, _, cx| {
                if open {
                    dismiss(false, cx);
                } else {
                    cx.propagate();
                }
            })
            .child(
                div()
                    .id("hit")
                    .h(px(CONTROL_H))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(6.))
                    .pr(px(4.))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
                    .rounded(px(3.))
                    .child(swatch)
                    .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
                    .on_click(move |_, _, cx| toggle(cx)),
            )
            .children(panel)
    }
}

/// The "More Colors" dialog: a colour typed as hexadecimal or as red, green
/// and blue.
struct MoreColors {
    modal: Modal,
    hex: Entity<TextInput>,
    rgb: [Entity<TextInput>; 3],
    current: Option<Color>,
    on_pick: Handler<Option<Color>>,
}

impl MoreColors {
    fn new(modal: Modal, current: Option<Color>, on_pick: Handler<Option<Color>>, cx: &mut Context<Self>) -> Self {
        let start = current.unwrap_or(Color(0xFFFFFF));
        let hex = input(cx, format!("#{}", start.to_hex()));
        let channel = |value: u8, cx: &mut Context<Self>| number_input(cx, Some(value as f64), 0.0, 255.0, 1.0);
        let rgb = [channel(start.r(), cx), channel(start.g(), cx), channel(start.b(), cx)];
        // The two ways of writing the colour follow each other.
        cx.subscribe(&hex, |this, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                if let Some(color) = Color::from_hex(this.hex.read(cx).text().trim()) {
                    for (field, value) in this.rgb.clone().iter().zip([color.r(), color.g(), color.b()]) {
                        field.update(cx, |field, cx| field.set_text(value.to_string(), cx));
                    }
                }
            }
        })
        .detach();
        for field in &rgb {
            cx.subscribe(field, |this, _, ev: &InputEvent, cx| {
                if *ev == InputEvent::Changed {
                    if let Some(color) = this.typed_rgb(cx) {
                        this.hex.update(cx, |hex, cx| hex.set_text(format!("#{}", color.to_hex()), cx));
                    }
                }
            })
            .detach();
        }
        MoreColors {
            modal,
            hex,
            rgb,
            current,
            on_pick,
        }
    }

    fn typed_rgb(&self, cx: &App) -> Option<Color> {
        let channel = |i: usize| -> Option<u8> {
            let n = self.rgb[i].read(cx).number()?;
            (0.0..=255.0).contains(&n).then_some(n.round() as u8)
        };
        Some(Color::rgb(channel(0)?, channel(1)?, channel(2)?))
    }

    fn color(&self, cx: &App) -> Option<Color> {
        self.typed_rgb(cx)
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(color) = self.color(cx) else {
            return;
        };
        let on_pick = self.on_pick.clone();
        self.modal.close(window, cx);
        on_pick(&Some(color), window, cx);
    }
}

impl Render for MoreColors {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let new = self.color(cx);
        let swatch = |title: &'static str, color: Option<Color>| {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(label(title).text_color(c(theme::TEXT_MUTED)))
                .child(
                    div()
                        .w(px(64.))
                        .h(px(32.))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .when_some(color, |d, color| d.bg(c(color.value()))),
                )
        };
        let modal = self.modal.clone();
        dialog(&self.modal, t("Colors", "Couleurs"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(labeled(t("Hex:", "Hex :"), TextField::new(&self.hex).width(90.0)).justify_between())
                            .child(labeled(t("Red:", "Rouge :"), NumberField::new("r", &self.rgb[0])).justify_between())
                            .child(labeled(t("Green:", "Vert :"), NumberField::new("g", &self.rgb[1])).justify_between())
                            .child(labeled(t("Blue:", "Bleu :"), NumberField::new("b", &self.rgb[2])).justify_between()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(swatch(t("New", "Nouvelle"), new))
                            .child(swatch(t("Current", "Actuelle"), self.current)),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("ok", t("OK", "OK"))
                            .primary()
                            .disabled(new.is_none())
                            .on_click(cx.listener(Self::ok)),
                    )
                    .child(Button::new("cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx))),
            )
    }
}
