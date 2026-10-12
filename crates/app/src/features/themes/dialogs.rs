//! "Customize Colors…" and "Customize Fonts…": the dialogs that make a
//! colour set or a font pair of one's own. OK gives it to the workbook
//! and keeps it in the menu, under "Custom".

use super::gallery::swatch_strip;
use super::{ThemesUi, new_custom_name, set_colors, set_fonts};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::Modal;
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{ColorButton, ComboBox, TextField, dialog, input, label, labeled, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::style::{Color, ThemeSlot};
use opensheet_engine::theme::{ThemeColors, ThemeFonts};

/// The twelve colours in the order of the dialog (that of a theme part),
/// with the names Excel gives them.
fn slot_rows() -> [(ThemeSlot, &'static str); 12] {
    [
        (ThemeSlot::Dark1, t("Text/Background - Dark 1", "Texte/arrière-plan - Sombre 1")),
        (ThemeSlot::Light1, t("Text/Background - Light 1", "Texte/arrière-plan - Clair 1")),
        (ThemeSlot::Dark2, t("Text/Background - Dark 2", "Texte/arrière-plan - Sombre 2")),
        (ThemeSlot::Light2, t("Text/Background - Light 2", "Texte/arrière-plan - Clair 2")),
        (ThemeSlot::Accent1, t("Accent 1", "Accentuation 1")),
        (ThemeSlot::Accent2, t("Accent 2", "Accentuation 2")),
        (ThemeSlot::Accent3, t("Accent 3", "Accentuation 3")),
        (ThemeSlot::Accent4, t("Accent 4", "Accentuation 4")),
        (ThemeSlot::Accent5, t("Accent 5", "Accentuation 5")),
        (ThemeSlot::Accent6, t("Accent 6", "Accentuation 6")),
        (ThemeSlot::Hyperlink, t("Hyperlink", "Lien hypertexte")),
        (ThemeSlot::FollowedHyperlink, t("Followed Hyperlink", "Lien hypertexte visité")),
    ]
}

/// The name a set is kept under: what was typed, or the name proposed.
fn typed_name(field: &Entity<TextInput>, proposed: &str, cx: &gpui::App) -> String {
    let typed = field.read(cx).text().trim().to_string();
    if typed.is_empty() { proposed.to_string() } else { typed }
}

/// Keep `item` in a list of the user's own, in place of one of the same
/// name.
fn keep<T>(list: &mut Vec<T>, item: T, name_of: impl Fn(&T) -> &str) {
    let name = name_of(&item).to_string();
    match list.iter().position(|other| name_of(other).eq_ignore_ascii_case(&name)) {
        Some(at) => list[at] = item,
        None => list.push(item),
    }
}

// ------------------------------------------------------------ Customize Colors

pub(super) fn open_colors(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let colors = app.wb.theme().colors.clone();
    let taken: Vec<String> = app.ext_mut::<ThemesUi>().colors.iter().map(|set| set.name.clone()).collect();
    let proposed = new_custom_name(&taken);
    app.open_modal(window, cx, move |modal, _, cx| CustomColors {
        name: input(cx, proposed.clone()),
        proposed,
        colors,
        modal,
    });
}

struct CustomColors {
    modal: Modal,
    colors: ThemeColors,
    name: Entity<TextInput>,
    proposed: String,
}

impl CustomColors {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let mut colors = self.colors.clone();
        colors.name = typed_name(&self.name, &self.proposed, cx);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            keep(&mut app.ext_mut::<ThemesUi>().colors, colors.clone(), |set| &set.name);
            set_colors(app, colors, cx);
        });
    }
}

impl Render for CustomColors {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = slot_rows().into_iter().enumerate().map(|(index, (slot, text))| {
            let current = Color(self.colors.get(slot));
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(label(text))
                .child(
                    ColorButton::new(("theme-slot", index), Some(Some(current)))
                        .width(86.0)
                        .more(&self.modal.app)
                        .on_pick(cx.listener(move |this, color: &Option<Color>, _, cx| {
                            // A colour of the theme is its own value here:
                            // the set being made refers to nothing.
                            if let Some(color) = color {
                                this.colors.set(slot, color.value());
                                cx.notify();
                            }
                        })),
                )
        });
        dialog(&self.modal, t("Create Theme Colors", "Créer couleurs de thème"))
            .width(470.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(label(t(
                                "Click on the colored box to change a theme color",
                                "Cliquez sur la zone colorée pour modifier une couleur de thème",
                            )))
                            .children(rows),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(label(t("Preview", "Aperçu")))
                            .child(sample_page(&self.colors)),
                    ),
            )
            .child(labeled(t("Name:", "Nom :"), TextField::new(&self.name)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// What a colour set looks like: text on the two backgrounds, the
/// accents, a link.
fn sample_page(colors: &ThemeColors) -> gpui::Div {
    let half = |background: ThemeSlot, text: ThemeSlot| {
        div()
            .flex_1()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .bg(c(colors.get(background)))
            .text_size(px(11.))
            .text_color(c(colors.get(text)))
            .child(t("Text", "Texte"))
            .child(
                div()
                    .text_color(c(colors.get(ThemeSlot::Hyperlink)))
                    .child(t("Hyperlink", "Lien hypertexte")),
            )
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .child(half(ThemeSlot::Dark2, ThemeSlot::Light1))
                .child(half(ThemeSlot::Light1, ThemeSlot::Dark1)),
        )
        .child(swatch_strip(colors, 17.0))
}

// ------------------------------------------------------------- Customize Fonts

pub(super) fn open_fonts(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let fonts = app.wb.theme().fonts.clone();
    let taken: Vec<String> = app.ext_mut::<ThemesUi>().fonts.iter().map(|pair| pair.name.clone()).collect();
    let proposed = new_custom_name(&taken);
    let names: Vec<SharedString> = app.fonts.names.iter().cloned().map(SharedString::from).collect();
    let resolver = app.fonts.clone();
    app.open_modal(window, cx, move |modal, _, cx| CustomFonts {
        major: input(cx, fonts.major.to_string()),
        minor: input(cx, fonts.minor.to_string()),
        name: input(cx, proposed.clone()),
        proposed,
        names,
        resolver,
        modal,
    });
}

struct CustomFonts {
    modal: Modal,
    major: Entity<TextInput>,
    minor: Entity<TextInput>,
    name: Entity<TextInput>,
    proposed: String,
    /// The fonts of the system, for the two lists.
    names: Vec<SharedString>,
    resolver: std::rc::Rc<crate::grid::FontResolver>,
}

impl CustomFonts {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let major = self.major.read(cx).text().trim().to_string();
        let minor = self.minor.read(cx).text().trim().to_string();
        // A pair needs its two fonts.
        if major.is_empty() || minor.is_empty() {
            return;
        }
        let fonts = ThemeFonts::new(typed_name(&self.name, &self.proposed, cx), &major, &minor);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            keep(&mut app.ext_mut::<ThemesUi>().fonts, fonts.clone(), |pair| &pair.name);
            set_fonts(app, fonts, cx);
        });
    }
}

impl Render for CustomFonts {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let major = self.major.read(cx).text().trim().to_string();
        let minor = self.minor.read(cx).text().trim().to_string();
        let family = |name: &str| self.resolver.resolve(&std::sync::Arc::from(name));
        let field = |id: &'static str, text: &'static str, input: &Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(label(text))
                .child(ComboBox::new(id, input, self.names.clone()).width(210.0))
        };
        dialog(&self.modal, t("Create Theme Fonts", "Créer polices de thème"))
            .width(470.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .child(field("heading-font", t("Heading font:", "Police du titre :"), &self.major))
                            .child(field("body-font", t("Body font:", "Police du corps de texte :"), &self.minor)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Preview", "Aperçu")))
                            .child(
                                div()
                                    .flex_1()
                                    .p(px(8.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .overflow_hidden()
                                    .bg(c(0xFFFFFF))
                                    .border_1()
                                    .border_color(c(theme::BORDER_STRONG))
                                    .child(
                                        div()
                                            .font_family(family(&major))
                                            .text_size(px(17.))
                                            .child(t("Heading", "Titre")),
                                    )
                                    .child(div().font_family(family(&minor)).text_size(px(12.)).child(t(
                                        "Body text body text body text. Body text body text.",
                                        "Corps de texte corps de texte corps de texte. Corps de texte corps de texte.",
                                    ))),
                            ),
                    ),
            )
            .child(labeled(t("Name:", "Nom :"), TextField::new(&self.name)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
