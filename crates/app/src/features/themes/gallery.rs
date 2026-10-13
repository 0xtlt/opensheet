//! The drop-downs of the Themes group: the gallery of themes, the menu of
//! colour sets and the menu of font pairs. Each entry is drawn from what
//! it would give the workbook: its colours, its fonts.

use super::{Cmd, ThemesUi, color_choices, custom_sets, current_theme, font_choices, gallery_themes, set_colors, set_fonts, set_theme, shown_name};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::grid::FontResolver;
use crate::i18n::t;
use crate::theme::{self, c};
use crate::view::tooltip;
use gpui::{AnyElement, Context, FontWeight, SharedString, WeakEntity, Window, div, point, prelude::*, px};
use opensheet_engine::style::ThemeSlot;
use opensheet_engine::theme::{Theme, ThemeColors, ThemeFonts};
use std::rc::Rc;

/// Themes in a row of the gallery, and the size of a thumbnail.
pub(super) const PER_ROW: usize = 4;
// Wide enough for the longest names under the thumbnails ("Office 2013 - 2022").
const THUMB_W: f32 = 98.0;
const THUMB_H: f32 = 52.0;
const ITEM_W: f32 = THUMB_W + 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Themes,
    Colors,
    Fonts,
}

struct Gallery {
    app: WeakEntity<ExcelApp>,
    kind: Kind,
    /// The theme of the workbook when the drop-down opened.
    theme: Theme,
    custom: Rc<ThemesUi>,
    fonts: Rc<FontResolver>,
}

/// Open a drop-down under the control that asked for it.
pub(super) fn open(app: &mut ExcelApp, kind: Kind, window: &mut Window, cx: &mut Cx) {
    let anchor = app.command_anchor.and_then(|id| app.anchors.borrow().get(id).copied());
    let position = match anchor {
        Some(bounds) => point(bounds.origin.x, bounds.origin.y + bounds.size.height + px(2.)),
        None => window.mouse_position(),
    };
    let custom = Rc::new(custom_sets());
    let gallery = Gallery {
        app: cx.weak_entity(),
        kind,
        theme: app.wb.theme().clone(),
        custom,
        fonts: app.fonts.clone(),
    };
    let view = cx.new(|_| gallery);
    app.open_popup_view(position, view, cx);
}

/// The heading of a part of a drop-down ("Custom", "Office").
fn heading(text: &'static str) -> AnyElement {
    div()
        .h(px(22.))
        .px(px(6.))
        .flex()
        .items_center()
        .bg(c(theme::CHROME_BG))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(theme::TEXT_MUTED))
        .child(text)
        .into_any_element()
}

/// The colours a set is shown with, side by side.
pub(super) fn swatch_strip(colors: &ThemeColors, size: f32) -> gpui::Div {
    div().flex().flex_none().border_1().border_color(c(theme::BORDER_STRONG)).children(
        colors
            .swatches()
            .into_iter()
            .map(move |rgb| div().w(px(size)).h(px(size)).bg(c(rgb))),
    )
}

impl Gallery {
    /// Run something on the application once the drop-down is closed.
    fn pick(&self, action: impl Fn(&mut ExcelApp, &mut Cx) + 'static) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
        let app = self.app.clone();
        move |_, window, cx| {
            app.update(cx, |app, cx| {
                app.close_popup(window, cx);
                action(app, cx);
            })
            .ok();
        }
    }

    /// A command under the entries, drawn like a menu entry.
    fn command(&self, cmd: Cmd) -> AnyElement {
        let app = self.app.clone();
        div()
            .id(cmd.label())
            .debug_selector(move || format!("command:{cmd:?}"))
            .h(px(28.))
            .flex()
            .items_center()
            .px(px(26.))
            .rounded(px(4.))
            .text_size(px(12.5))
            .hover(|s| s.bg(c(theme::HOVER)))
            .on_click(move |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.close_popup(window, cx);
                    cmd.run(app, window, cx);
                })
                .ok();
            })
            .child(cmd.label())
            .into_any_element()
    }

    /// The thumbnail of a theme: "Aa" in its heading font over a white
    /// page, and its six accents.
    fn theme_thumb(&self, index: usize, theme: &Theme, current: bool) -> AnyElement {
        let name: SharedString = shown_name(&theme.colors.name).into();
        let picked = theme.clone();
        let accents = ThemeSlot::ACCENTS.map(|slot| theme.rgb(slot));
        let english = theme.colors.name.clone();
        div()
            .id(("theme", index))
            // The tests find the entries by their English names.
            .debug_selector(move || format!("theme:{english}"))
            .w(px(ITEM_W))
            .flex_none()
            .p(px(3.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .border_1()
            .border_color(if current { c(theme::ACCENT) } else { gpui::transparent_black() })
            .rounded(px(3.))
            .when(current, |d| d.bg(c(theme::ACCENT_LIGHT)))
            .hover(|s| s.bg(c(theme::HOVER)).border_color(c(theme::TEXT_MUTED)))
            .child(
                div()
                    .w(px(THUMB_W))
                    .h(px(THUMB_H))
                    .flex()
                    .flex_col()
                    .bg(c(theme.rgb(ThemeSlot::Light1)))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .px(px(6.))
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_family(self.fonts.resolve(&theme.fonts.major))
                                    .text_size(px(20.))
                                    .text_color(c(theme.rgb(ThemeSlot::Dark1)))
                                    .child("Aa"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(18.))
                                    .bg(c(theme.rgb(ThemeSlot::Dark2))),
                            ),
                    )
                    .child(
                        div()
                            .h(px(10.))
                            .flex()
                            .gap(px(1.))
                            .px(px(4.))
                            .pb(px(3.))
                            .children(accents.into_iter().map(|rgb| div().flex_1().bg(c(rgb)))),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .text_size(px(11.))
                    .text_center()
                    .truncate()
                    .child(name.clone()),
            )
            .tooltip(tooltip(name))
            .on_click(self.pick(move |app, cx| set_theme(app, picked.clone(), cx)))
            .into_any_element()
    }

    fn themes(&self) -> Vec<AnyElement> {
        let current = current_theme(&self.theme);
        let thumbs: Vec<AnyElement> = gallery_themes()
            .iter()
            .enumerate()
            .map(|(index, theme)| self.theme_thumb(index, theme, current == Some(index)))
            .collect();
        vec![
            heading("Office"),
            div()
                .w(px(PER_ROW as f32 * (ITEM_W + 2.0)))
                .flex()
                .flex_wrap()
                .gap(px(2.))
                .children(thumbs)
                .into_any_element(),
        ]
    }

    /// A line of a menu: ticked when it is the current choice.
    fn line(&self, index: usize, current: bool, height: f32) -> gpui::Stateful<gpui::Div> {
        div()
            .id(("choice", index))
            .h(px(height))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(6.))
            .rounded(px(4.))
            .text_size(px(12.5))
            .when(current, |d| d.bg(c(theme::TOGGLED)))
            .hover(|s| s.bg(c(theme::HOVER)))
    }

    fn colors(&self) -> Vec<AnyElement> {
        let choices = color_choices(&self.theme, &self.custom.colors);
        let mut body = Vec::new();
        let mut last_group = None;
        for (index, choice) in choices.into_iter().enumerate() {
            if last_group != Some(choice.custom) {
                last_group = Some(choice.custom);
                body.push(heading(if choice.custom { t("Custom", "Personnalisé") } else { "Office" }));
            }
            let colors = choice.colors.clone();
            body.push(
                self.line(index, choice.current, 24.0)
                    .debug_selector(|| format!("colors:{}", choice.colors.name))
                    .child(swatch_strip(&choice.colors, 13.0))
                    .child(div().flex_1().truncate().child(shown_name(&choice.colors.name)))
                    .on_click(self.pick(move |app, cx| set_colors(app, colors.clone(), cx)))
                    .into_any_element(),
            );
        }
        body
    }

    fn fonts(&self) -> Vec<AnyElement> {
        let choices = font_choices(&self.theme, &self.custom.fonts);
        let mut body = Vec::new();
        let mut last_group = None;
        for (index, choice) in choices.into_iter().enumerate() {
            if last_group != Some(choice.custom) {
                last_group = Some(choice.custom);
                body.push(heading(if choice.custom { t("Custom", "Personnalisé") } else { "Office" }));
            }
            let fonts: ThemeFonts = choice.fonts.clone();
            let (major, minor) = (self.fonts.resolve(&fonts.major), self.fonts.resolve(&fonts.minor));
            body.push(
                self.line(index, choice.current, 50.0)
                    .debug_selector(|| format!("fonts:{}", choice.fonts.name))
                    .child(
                        div()
                            .size(px(38.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(c(0xFFFFFF))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .font_family(major.clone())
                            .text_size(px(18.))
                            .child("Aa"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(c(theme::TEXT_MUTED))
                                    .truncate()
                                    .child(shown_name(&fonts.name)),
                            )
                            .child(div().font_family(major).text_size(px(13.5)).truncate().child(fonts.major.to_string()))
                            .child(div().font_family(minor).text_size(px(11.5)).truncate().child(fonts.minor.to_string())),
                    )
                    .on_click(self.pick(move |app, cx| set_fonts(app, fonts.clone(), cx)))
                    .into_any_element(),
            );
        }
        body
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // The entries scroll; the commands under them stay in sight.
        let (body, commands, width) = match self.kind {
            Kind::Themes => (self.themes(), vec![Cmd::Browse, Cmd::SaveCurrent], PER_ROW as f32 * (ITEM_W + 2.0) + 28.0),
            Kind::Colors => (self.colors(), vec![Cmd::CustomizeColors], 290.0),
            Kind::Fonts => (self.fonts(), vec![Cmd::CustomizeFonts], 290.0),
        };
        div()
            .w(px(width))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.))
            .child(
                div()
                    .id("themes-drop-down")
                    .max_h(px(500.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(body),
            )
            .child(div().h(px(1.)).my(px(2.)).bg(c(theme::BORDER)))
            .children(commands.into_iter().map(|cmd| self.command(cmd)))
    }
}
