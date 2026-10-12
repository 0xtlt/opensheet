//! The PivotTable Styles gallery of the Design tab: thumbnails drawn from
//! the styles themselves, in the ribbon and in the drop-down that shows
//! all of them.

use super::{Cmd, Dismissed, PivotUi, active_pivot, just_closed, update};
use crate::app::ExcelApp;
use crate::features::{Command, Cx};
use crate::i18n::t;
use crate::theme::{self, c};
use crate::widgets::{Sketch, icon, label, preview, tooltip};
use gpui::{AnyElement, Context, FontWeight, SharedString, WeakEntity, Window, div, point, prelude::*, px};
use opensheet_engine::pivot::PivotStyle;
use opensheet_engine::style::StylePatch;

/// Size of a thumbnail, and how many make a row of the gallery.
const THUMB_W: f32 = 52.0;
const THUMB_H: f32 = 40.0;
const PER_ROW: usize = 7;
/// Control of the ribbon the drop-down hangs from.
const ANCHOR: &str = "pd-styles";

/// Draw the sample report of a style: each cell with its fill and rules,
/// and a dash standing for its text.
fn draw_thumbnail(s: &mut Sketch, sample: &[Vec<StylePatch>]) {
    s.rect(0.0, 0.0, s.width, s.height, c(0xFFFFFF));
    let rows = sample.len().max(1) as f32;
    let cols = sample.first().map_or(1, Vec::len).max(1) as f32;
    let (cw, ch) = (s.width / cols, s.height / rows);
    for (r, row) in sample.iter().enumerate() {
        for (k, patch) in row.iter().enumerate() {
            let (x0, y0) = (k as f32 * cw, r as f32 * ch);
            let (x1, y1) = (x0 + cw, y0 + ch);
            if let Some(Some(fill)) = patch.fill {
                s.rect(x0, y0, x1, y1, theme::engine_color(fill));
            }
            let ink = match patch.font_color {
                Some(Some(color)) => theme::engine_color(color),
                _ => c(0x5A5A5A),
            };
            let thick = if patch.bold == Some(true) { 2.0 } else { 1.0 };
            let mid = (y0 + ch / 2.0).floor();
            s.rect(x0 + 2.0, mid, x1 - 3.0, mid + thick, ink);
            for (border, y) in [(patch.border_top, y0), (patch.border_bottom, y1 - 1.0)] {
                if let Some(border) = border.filter(|b| !b.is_none()) {
                    let color = border.color.map(theme::engine_color).unwrap_or(c(0x000000));
                    s.rect(x0, y, x1, y + 1.0, color);
                }
            }
            for (border, x) in [(patch.border_left, x0), (patch.border_right, x1 - 1.0)] {
                if let Some(border) = border.filter(|b| !b.is_none()) {
                    let color = border.color.map(theme::engine_color).unwrap_or(c(0x000000));
                    s.rect(x, y0, x + 1.0, y1, color);
                }
            }
        }
    }
}

/// The name Excel shows for a built-in style: "Pivot Style Light 16".
fn display_name(name: &str) -> SharedString {
    let Some(rest) = name.strip_prefix("PivotStyle") else {
        return t("None", "Aucun").into();
    };
    let split = rest.find(|ch: char| ch.is_ascii_digit()).unwrap_or(rest.len());
    let (family, number) = rest.split_at(split);
    let pattern = match family {
        "Light" => t("Pivot Style Light {}", "Style de tableau croisé dynamique clair {}"),
        "Medium" => t("Pivot Style Medium {}", "Style de tableau croisé dynamique moyen {}"),
        _ => t("Pivot Style Dark {}", "Style de tableau croisé dynamique foncé {}"),
    };
    pattern.replace("{}", number).into()
}

/// One thumbnail: the style `name` with the style options of `options`.
fn thumbnail(
    id: usize,
    name: &str,
    options: &PivotStyle,
    on_pick: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let style = PivotStyle {
        name: name.to_string(),
        ..options.clone()
    };
    let sample = style.sample_in(&crate::theme::document_theme());
    let current = options.name == name;
    div()
        .id(("pivot-style", id))
        .p(px(2.))
        .border_1()
        .border_color(if current { c(theme::ACCENT) } else { gpui::transparent_black() })
        .rounded(px(2.))
        .hover(|s| s.border_color(c(0xF2994A)))
        .child(
            div()
                .border_1()
                .border_color(c(theme::BORDER))
                .child(preview(THUMB_W, THUMB_H, move |s| draw_thumbnail(s, &sample))),
        )
        .tooltip(tooltip(display_name(name)))
        .on_click(move |_, window, cx| on_pick(window, cx))
        .into_any_element()
}

fn set_style(app: &mut ExcelApp, id: u32, name: String, window: &mut Window, cx: &mut Cx) {
    update(app, id, t("PivotTable Style", "Style de tableau croisé dynamique"), window, cx, move |p| {
        p.style.name = name.clone()
    });
}

/// The gallery as the Design tab shows it: the row of styles holding the
/// current one, and the button that opens the whole gallery. A narrow
/// window only keeps the button.
pub(super) fn ribbon_gallery(app: &mut ExcelApp, cx: &mut Cx) -> AnyElement {
    let Some(pivot) = active_pivot(app).filter(|_| app.ribbon_level < 3) else {
        return app.cmd_large(ANCHOR, Cmd::Styles, true, cx);
    };
    let (id, options) = (pivot.id, pivot.style.clone());
    let names = PivotStyle::builtin_names();
    let at = names.iter().position(|name| *name == options.name).unwrap_or(0);
    let first = at / PER_ROW * PER_ROW;
    let weak = cx.weak_entity();
    let thumbs: Vec<AnyElement> = names[first..(first + PER_ROW).min(names.len())]
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let (weak, name) = (weak.clone(), name.clone());
            let pick = name.clone();
            thumbnail(first + k, &name, &options, move |window, cx| {
                let name = pick.clone();
                weak.update(cx, |app, cx| set_style(app, id, name, window, cx)).ok();
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
    pivot: u32,
    options: PivotStyle,
    /// Where a press outside the gallery (which closes it) landed.
    closed: Dismissed,
}

pub(super) fn open_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    // The press of this very click may have just closed the gallery.
    let closed = app.ext_mut::<PivotUi>().gallery_closed.clone();
    if just_closed(&closed, window.mouse_position()) {
        return;
    }
    let Some(pivot) = active_pivot(app) else {
        return;
    };
    let (id, options) = (pivot.id, pivot.style.clone());
    let position = match app.anchors.borrow().get(ANCHOR) {
        // Under the gallery of the ribbon, its right edge under the button.
        Some(b) => point(
            (b.right() - px(PER_ROW as f32 * (THUMB_W + 8.0) + 40.0)).max(px(6.)),
            b.bottom() + px(2.),
        ),
        None => window.mouse_position(),
    };
    let weak = cx.weak_entity();
    let view = cx.new(|_| Gallery {
        app: weak,
        pivot: id,
        options,
        closed,
    });
    app.open_popup_view(position, view, cx);
}

impl Gallery {
    fn pick(&self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (id, name) = (self.pivot, name.to_string());
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                set_style(app, id, name, window, cx);
            })
            .ok();
    }

    fn section(&self, title: &'static str, names: &[String], offset: usize, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = names
            .chunks(PER_ROW)
            .enumerate()
            .map(|(r, row)| {
                div()
                    .flex()
                    .gap(px(2.))
                    .children(row.iter().enumerate().map(|(k, name)| {
                        let pick = name.clone();
                        let this = cx.weak_entity();
                        thumbnail(offset + r * PER_ROW + k, name, &self.options, move |window, cx| {
                            this.update(cx, |this, cx| this.pick(&pick, window, cx)).ok();
                        })
                    }))
                    .into_any_element()
            })
            .collect();
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
        let names = PivotStyle::builtin_names();
        // "None" comes first, as in Excel.
        let mut light = vec![String::new()];
        light.extend(names.iter().take(28).cloned());
        let closed = self.closed.clone();
        let entry = |id: &'static str, text: &'static str, enabled: bool| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .when(!enabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
                .child(label(text))
        };
        div()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .text_size(px(12.))
            // A press outside closes the gallery; when it lands on the
            // button it hangs from, that button must not open it again.
            .on_mouse_down_out(move |ev, _, _| closed.set(Some(ev.position)))
            .child(
                div()
                    .id("pivot-style-gallery")
                    .max_h(px(430.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(self.section(t("Light", "Clair"), &light, 0, cx))
                    .child(self.section(t("Medium", "Moyen"), &names[28..56], 100, cx))
                    .child(self.section(t("Dark", "Foncé"), &names[56..], 200, cx)),
            )
            .child(div().h(px(1.)).bg(c(theme::BORDER)))
            // Custom styles are not supported: the entry is listed as in
            // Excel but does not answer.
            .child(entry(
                "new-style",
                t("New PivotTable Style...", "Nouveau style de tableau croisé dynamique..."),
                false,
            ))
            .child(
                entry("clear-style", t("Clear", "Effacer"), true)
                    .on_click(cx.listener(|this, _, window, cx| this.pick("", window, cx))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_are_named_as_in_excel() {
        assert_eq!(display_name("PivotStyleLight16"), "Pivot Style Light 16");
        assert_eq!(display_name("PivotStyleMedium2"), "Pivot Style Medium 2");
        assert_eq!(display_name("PivotStyleDark28"), "Pivot Style Dark 28");
        assert_eq!(display_name(""), "None");
    }
}
