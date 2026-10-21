//! Cell styles: the gallery of the Home tab, with each style drawn as it
//! looks, and the New / Modify Cell Style dialog. The styles themselves
//! are the engine's (`cell_style.rs`).

use super::{Cmd, named};
use crate::app::{ExcelApp, MenuEntry, Popup, PopupKind};
use crate::features::Command;
use crate::features::{Cx, Intent, format_cells};
use crate::grid::FontResolver;
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, Sketch, TextField, dialog, label};
use gpui::{
    AnyElement, Context, Entity, Focusable, FontStyle, FontWeight, Hsla, MouseButton,
    MouseDownEvent, SharedString, WeakEntity, Window, div, point, prelude::*, px,
};
use opensheet_engine::cell_style::{CellStyle, StyleError, StyleGroup, StyleParts};
use opensheet_engine::style::{Border, BorderStyle, Underline};
use opensheet_engine::{Color, Style};
use std::rc::Rc;

// --------------------------------------------------------------------- names

/// The name the interface gives a style: Excel's built-in styles are
/// called in the language of the interface, the others as they were named.
pub(super) fn display_name(style: &CellStyle) -> String {
    let Some(id) = style.builtin else {
        return style.name.clone();
    };
    // The themed styles: a strength and the number of an accent.
    if let 29..=52 = id {
        let accent = ((id - 29) / 4 + 1).to_string();
        let pattern = match (id - 29) % 4 {
            0 => t("Accent{}", "Accent{}"),
            1 => t("20% - Accent{}", "20 % - Accent{}"),
            2 => t("40% - Accent{}", "40 % - Accent{}"),
            _ => t("60% - Accent{}", "60 % - Accent{}"),
        };
        return pattern.replace("{}", &accent);
    }
    if let 16..=19 = id {
        return t("Heading {}", "Titre {}").replace("{}", &(id - 15).to_string());
    }
    let name = match id {
        0 => t_ctx("style", "Normal", "Normal"),
        3 => t_ctx("style", "Comma", "Milliers"),
        4 => t_ctx("style", "Currency", "Monétaire"),
        5 => t_ctx("style", "Percent", "Pourcentage"),
        6 => t_ctx("style", "Comma [0]", "Milliers [0]"),
        7 => t_ctx("style", "Currency [0]", "Monétaire [0]"),
        8 => t_ctx("style", "Hyperlink", "Lien hypertexte"),
        9 => t_ctx("style", "Followed Hyperlink", "Lien hypertexte visité"),
        10 => t_ctx("style", "Note", "Note"),
        11 => t_ctx("style", "Warning Text", "Avertissement"),
        15 => t_ctx("style", "Title", "Titre"),
        20 => t_ctx("style", "Input", "Entrée"),
        21 => t_ctx("style", "Output", "Sortie"),
        22 => t_ctx("style", "Calculation", "Calcul"),
        23 => t_ctx("style", "Check Cell", "Vérification"),
        24 => t_ctx("style", "Linked Cell", "Cellule liée"),
        25 => t_ctx("style", "Total", "Total"),
        26 => t_ctx("style", "Good", "Satisfaisant"),
        27 => t_ctx("style", "Bad", "Insatisfaisant"),
        28 => t_ctx("style", "Neutral", "Neutre"),
        53 => t_ctx("style", "Explanatory Text", "Texte explicatif"),
        _ => return style.name.clone(),
    };
    name.to_string()
}

fn group_title(group: StyleGroup) -> &'static str {
    match group {
        StyleGroup::Custom => t_ctx("styles", "Custom", "Personnalisé"),
        StyleGroup::GoodBadNeutral => t(
            "Good, Bad and Neutral",
            "Satisfaisant, insatisfaisant et neutre",
        ),
        StyleGroup::DataModel => t("Data and Model", "Données et modèle"),
        StyleGroup::TitlesHeadings => t("Titles and Headings", "Titres et en-têtes"),
        StyleGroup::Themed => t("Themed Cell Styles", "Styles de cellules avec thème"),
        StyleGroup::NumberFormat => t("Number Format", "Format de nombre"),
    }
}

const GROUPS: [StyleGroup; 6] = [
    StyleGroup::Custom,
    StyleGroup::GoodBadNeutral,
    StyleGroup::DataModel,
    StyleGroup::TitlesHeadings,
    StyleGroup::Themed,
    StyleGroup::NumberFormat,
];

/// The sections of the gallery, in order, with their styles; a section
/// without styles (no custom ones yet) is left out.
pub(super) fn sections(app: &ExcelApp) -> Vec<(StyleGroup, Vec<CellStyle>)> {
    let all = app.wb.all_cell_styles();
    GROUPS
        .into_iter()
        .map(|group| {
            (
                group,
                all.iter()
                    .filter(|style| style.group() == group)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        })
        .filter(|(_, styles)| !styles.is_empty())
        .collect()
}

// ------------------------------------------------------------------- changes

/// Give the selection a style, in one undo step.
pub(super) fn apply(app: &mut ExcelApp, name: &str, cx: &mut Cx) {
    if !app.may_all(Intent::FormatCells, cx) {
        return;
    }
    let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
    if app.wb.apply_cell_style(sheet, &ranges, name).is_ok() {
        app.commit(t("Cell Styles", "Styles de cellules"), cx);
    }
}

/// Delete a style; its cells go back to Normal.
pub(super) fn delete(app: &mut ExcelApp, name: &str, cx: &mut Cx) {
    if app.wb.delete_cell_style(name).is_ok() {
        app.commit(t("Delete Cell Style", "Supprimer le style de cellule"), cx);
    }
}

fn error_text(error: StyleError) -> &'static str {
    match error {
        StyleError::NameTaken => t(
            "This style name already exists.",
            "Ce nom de style existe déjà.",
        ),
        _ => t("Style name is not valid.", "Nom de style non valide."),
    }
}

/// What the dialog is open for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Purpose {
    /// A new style, or a copy of one.
    New,
    /// Changing the style of this name.
    Modify(String),
}

/// What OK does in the dialog. False, after saying why, when the name
/// will not do.
pub(super) fn save(
    app: &mut ExcelApp,
    purpose: &Purpose,
    name: &str,
    style: Style,
    includes: StyleParts,
    cx: &mut Cx,
) -> bool {
    let (done, label) = match purpose {
        Purpose::New => (
            app.wb.add_cell_style(name, style, includes),
            t("New Cell Style", "Nouveau style de cellule"),
        ),
        Purpose::Modify(old) => (
            app.wb.modify_cell_style(old, style, includes),
            t("Modify Cell Style", "Modifier le style de cellule"),
        ),
    };
    match done {
        Ok(()) => {
            app.commit(label, cx);
            true
        }
        Err(error) => {
            app.alert(Alert::new("OpenSheet", error_text(error)).warning(), cx);
            false
        }
    }
}

// ------------------------------------------------------------------- drawing

fn hsla(color: Option<Color>, default: u32) -> Hsla {
    c(color.map_or(default, |color| color.value()))
}

fn line_width(style: BorderStyle) -> f32 {
    match style {
        BorderStyle::None => 0.0,
        BorderStyle::Medium => 2.0,
        BorderStyle::Thick => 3.0,
        _ => 1.0,
    }
}

/// Draw `text` as a cell of this format would show it, in the whole box
/// of the sketch.
fn draw_sample(s: &mut Sketch, style: &Style, text: &str, fonts: &FontResolver) {
    let (w, h) = (s.width, s.height);
    s.rect(0.0, 0.0, w, h, hsla(style.fill, 0xFFFFFF));
    let font = gpui::Font {
        family: fonts.resolve(&style.font.name),
        features: Default::default(),
        fallbacks: None,
        weight: if style.font.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if style.font.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    };
    // Points to pixels, kept inside the box.
    let size = (style.font.size_pt() * 96.0 / 72.0).min(h - 8.0);
    let color = hsla(style.font.color, 0x000000);
    let underline = style.font.underline != Underline::None;
    s.styled_text(
        text,
        6.0,
        (h - size * 1.25) / 2.0,
        &font,
        size,
        color,
        underline,
        style.font.strike,
    );
    let mut side = |border: &Border, x0: f32, y0: f32, x1: f32, y1: f32, across: bool| {
        if border.is_none() {
            return;
        }
        let color = hsla(border.color, 0x000000);
        if border.style == BorderStyle::Double {
            // Two hairlines with a gap, drawn inside the box.
            for offset in [0.0, 2.0] {
                if across {
                    let y = if y0 == 0.0 { offset } else { y0 - 1.0 - offset };
                    s.rect(x0, y, x1, y + 1.0, color);
                } else {
                    let x = if x0 == 0.0 { offset } else { x0 - 1.0 - offset };
                    s.rect(x, y0, x + 1.0, y1, color);
                }
            }
            return;
        }
        let width = line_width(border.style);
        if across {
            let y = if y0 == 0.0 { 0.0 } else { y0 - width };
            s.rect(x0, y, x1, y + width, color);
        } else {
            let x = if x0 == 0.0 { 0.0 } else { x0 - width };
            s.rect(x, y0, x + width, y1, color);
        }
    };
    let borders = style.borders;
    side(&borders.top, 0.0, 0.0, w, 0.0, true);
    side(&borders.bottom, 0.0, h, w, h, true);
    side(&borders.left, 0.0, 0.0, 0.0, h, false);
    side(&borders.right, w, 0.0, w, h, false);
}

// ------------------------------------------------------------------- gallery

const THUMB_W: f32 = 112.0;
const THUMB_H: f32 = 26.0;
const PER_ROW: usize = 6;

pub(super) fn open_gallery(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let anchor = app
        .command_anchor
        .and_then(|id| app.anchors.borrow().get(id).copied());
    let pos = match anchor {
        Some(bounds) => point(
            bounds.origin.x,
            bounds.origin.y + bounds.size.height + px(2.),
        ),
        None => window.mouse_position(),
    };
    let (row, col) = app.active();
    let gallery = Gallery {
        app: cx.weak_entity(),
        sections: sections(app),
        current: app.wb.cell_style_at(app.sheet, row, col).to_string(),
        fonts: app.fonts.clone(),
    };
    let view = cx.new(|_| gallery);
    app.open_popup_view(pos, view, cx);
}

struct Gallery {
    app: WeakEntity<ExcelApp>,
    sections: Vec<(StyleGroup, Vec<CellStyle>)>,
    /// The style of the active cell, framed in the gallery.
    current: String,
    fonts: Rc<FontResolver>,
}

/// The menu of a style of the gallery (a secondary click on it): Apply,
/// Modify, Duplicate, Delete.
pub(super) fn style_menu(app: &ExcelApp, style: &CellStyle) -> Vec<MenuEntry> {
    let name = style.name.clone();
    let normal = style.builtin == Some(0);
    // Only what the workbook itself keeps can be deleted: a style of the
    // user's, or a built-in one that was changed (Excel's comes back).
    let kept = app.wb.cell_styles.iter().any(|own| own.name == style.name);
    let (apply, modify, duplicate, remove) = (name.clone(), name.clone(), name.clone(), name);
    vec![
        MenuEntry::new(t("Apply", "Appliquer"), move |a, _, cx| {
            self::apply(a, &apply, cx)
        }),
        MenuEntry::new(t("Modify...", "Modifier..."), move |a, window, cx| {
            open_dialog(a, Purpose::Modify(modify.clone()), None, window, cx)
        })
        .disabled(normal),
        MenuEntry::new(t("Duplicate...", "Dupliquer..."), move |a, window, cx| {
            open_dialog(a, Purpose::New, Some(&duplicate), window, cx)
        }),
        MenuEntry::new(t("Delete", "Supprimer"), move |a, _, cx| {
            delete(a, &remove, cx)
        })
        .disabled(!kept),
    ]
}

impl Gallery {
    fn thumb(&self, index: usize, style: &CellStyle) -> AnyElement {
        let shown = display_name(style);
        let (format, text, fonts) = (style.style.clone(), shown.clone(), self.fonts.clone());
        let current = style.name.eq_ignore_ascii_case(&self.current);
        let (app, name) = (self.app.clone(), style.name.clone());
        let (menu_app, menu_style) = (self.app.clone(), style.clone());
        div()
            .id(("cell-style", index))
            .debug_selector(|| format!("cell-style-{}", style.name))
            .m(px(2.))
            .border_1()
            .border_color(c(if current {
                theme::ACCENT
            } else {
                theme::BORDER
            }))
            .hover(|s| s.border_color(c(theme::ACCENT)))
            .child(widgets::preview(THUMB_W - 6.0, THUMB_H, move |s| {
                draw_sample(s, &format, &text, &fonts)
            }))
            .on_click(move |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.close_popup(window, cx);
                    apply(app, &name, cx);
                })
                .ok();
            })
            .on_mouse_down(MouseButton::Right, move |ev: &MouseDownEvent, _, cx| {
                let position = ev.position;
                menu_app
                    .update(cx, |app, cx| {
                        let entries = style_menu(app, &menu_style);
                        app.popup = Some(Popup::new(PopupKind::Menu(entries), position, 200.0));
                        cx.notify();
                    })
                    .ok();
            })
            .into_any_element()
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        for (group, styles) in &self.sections {
            body.push(
                div()
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .bg(c(theme::CHROME_BG))
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(theme::TEXT_MUTED))
                    .child(group_title(*group))
                    .into_any_element(),
            );
            let thumbs: Vec<AnyElement> = styles
                .iter()
                .map(|style| {
                    index += 1;
                    self.thumb(index, style)
                })
                .collect();
            body.push(
                div()
                    .w(px(PER_ROW as f32 * THUMB_W))
                    .flex()
                    .flex_wrap()
                    .children(thumbs)
                    .into_any_element(),
            );
        }
        let app = self.app.clone();
        div()
            .id("cell-style-gallery")
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(body)
            .child(widgets::separator())
            .child(
                div()
                    .id("new-cell-style")
                    .debug_selector(|| "new-cell-style".to_string())
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .text_size(px(12.5))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(Cmd::NewCellStyle.label())
                    .on_click(move |_, window, cx| {
                        app.update(cx, |app, cx| {
                            app.close_popup(window, cx);
                            Cmd::NewCellStyle.run(app, window, cx);
                        })
                        .ok();
                    }),
            )
    }
}

// -------------------------------------------------------------------- dialog

/// The name Excel proposes for a new style: "Style 1", "Style 2"...
fn proposed_name(app: &ExcelApp) -> String {
    let stem = t("Style {}", "Style {}");
    (1..)
        .map(|n| stem.replace("{}", &n.to_string()))
        .find(|name| app.wb.cell_style(name).is_none())
        .unwrap_or_default()
}

/// Open the dialog: for a new style made from the format of the active
/// cell (or, with `copy_of`, from another style: Duplicate), or to change
/// a style.
pub(super) fn open_dialog(
    app: &mut ExcelApp,
    purpose: Purpose,
    copy_of: Option<&str>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let source = match (&purpose, copy_of) {
        (Purpose::Modify(name), _) => app.wb.cell_style(name),
        (Purpose::New, Some(name)) => app.wb.cell_style(name),
        (Purpose::New, None) => None,
    };
    let name = match (&purpose, &source) {
        (Purpose::Modify(_), Some(style)) => display_name(style),
        (Purpose::New, Some(style)) => app.wb.duplicate_style_name(&display_name(style)),
        _ => proposed_name(app),
    };
    let (style, includes) = match source {
        Some(style) => (style.style, style.includes),
        None => {
            let mut style = app.active_style().clone();
            style.cell_style = None;
            (style, StyleParts::ALL)
        }
    };
    let fonts = app.fonts.clone();
    app.open_modal(window, cx, move |modal, _, cx| StyleDialog {
        modal,
        purpose,
        name: widgets::input(cx, name),
        style,
        includes,
        fonts,
        placed: false,
    });
}

struct StyleDialog {
    modal: Modal,
    purpose: Purpose,
    name: Entity<TextInput>,
    style: Style,
    includes: StyleParts,
    fonts: Rc<FontResolver>,
    /// The keyboard was given to the name.
    placed: bool,
}

impl StyleDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().to_string();
        let (purpose, style, includes) = (self.purpose.clone(), self.style.clone(), self.includes);
        let done = self.modal.update_app(cx, |app, cx| {
            save(app, &purpose, &name, style, includes, cx)
        });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }

    /// "Format...": the Format Cells dialog on the format of the style.
    fn format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.entity();
        let style = self.style.clone();
        self.modal.update_app(cx, |app, cx| {
            format_cells::open_for_style(app, style, window, cx, move |style, _, cx| {
                dialog.update(cx, |dialog, cx| {
                    dialog.style = style.clone();
                    cx.notify();
                });
            });
        });
    }

    fn part(
        &self,
        id: &'static str,
        text: &'static str,
        on: bool,
        set: fn(&mut StyleParts, bool),
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        named(
            id,
            Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
                set(&mut this.includes, *on);
                cx.notify();
            })),
        )
    }
}

impl Render for StyleDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.placed {
            self.placed = true;
            let handle = self.name.read(cx).focus_handle(cx);
            window.focus(&handle);
            self.name.update(cx, |input, cx| input.select_all_text(cx));
        }
        let modifying = matches!(self.purpose, Purpose::Modify(_));
        let title = if modifying {
            t("Modify Cell Style", "Modifier le style de cellule")
        } else {
            t("New Cell Style", "Nouveau style de cellule")
        };
        let parts = self.includes;
        let (format, fonts) = (self.style.clone(), self.fonts.clone());
        let sample: SharedString = self.name.read(cx).text().to_string().into();
        // A style keeps its name: Modify shows it without a box to type in.
        let name = if modifying {
            div().flex_1().child(sample.clone()).into_any_element()
        } else {
            TextField::new(&self.name).into_any_element()
        };
        dialog(&self.modal, title)
            .width(440.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Style name:", "Nom du style :")))
                    .child(name)
                    .child(named(
                        "style-format",
                        Button::new("format", t("Format...", "Format..."))
                            .on_click(cx.listener(|this, _, window, cx| this.format(window, cx))),
                    )),
            )
            .child(label(t(
                "The selected style has formatting for the following properties:",
                "Le style sélectionné met en forme les propriétés suivantes :",
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .pl(px(12.))
                    .child(self.part(
                        "style-number",
                        t("Numbers", "Nombres"),
                        parts.number,
                        |p, on| p.number = on,
                        cx,
                    ))
                    .child(self.part(
                        "style-alignment",
                        t("Alignment", "Alignement"),
                        parts.alignment,
                        |p, on| p.alignment = on,
                        cx,
                    ))
                    .child(self.part(
                        "style-font",
                        t("Font", "Police"),
                        parts.font,
                        |p, on| p.font = on,
                        cx,
                    ))
                    .child(self.part(
                        "style-border",
                        t("Border", "Bordure"),
                        parts.border,
                        |p, on| p.border = on,
                        cx,
                    ))
                    .child(self.part(
                        "style-fill",
                        t_ctx("style part", "Fill", "Remplir"),
                        parts.fill,
                        |p, on| p.fill = on,
                        cx,
                    ))
                    .child(self.part(
                        "style-protection",
                        t("Protection", "Protection"),
                        parts.protection,
                        |p, on| p.protection = on,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Preview:", "Aperçu :")))
                    .child(div().border_1().border_color(c(theme::BORDER)).child(
                        widgets::preview(180.0, 32.0, move |s| {
                            draw_sample(s, &format, &sample, &fonts)
                        }),
                    )),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
