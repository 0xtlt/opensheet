//! The ribbon: tab strip and the command groups of each tab.

use crate::app::*;
use crate::i18n::t;
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use gpui::{AnyElement, Context, FontWeight, SharedString, Stateful, Window, div, prelude::*, px};
use opensheet_engine::format::{self, FormatKind};
use opensheet_engine::functions::{self, Category};
use opensheet_engine::ops::{BorderPreset, PasteMode};
use opensheet_engine::style::{HAlign, Underline, VAlign};
use opensheet_engine::workbook::ClearMode;

type Cx<'a> = Context<'a, ExcelApp>;

const ROW_H: f32 = 24.0;

fn tab_names() -> [&'static str; 11] {
    [
        t("File", "Fichier"),
        t("Home", "Accueil"),
        t("Insert", "Insertion"),
        t("Draw", "Dessin"),
        t("Page Layout", "Mise en page"),
        t("Formulas", "Formules"),
        t("Data", "Données"),
        t("Review", "Révision"),
        t("View", "Affichage"),
        t("Automate", "Automatiser"),
        t("Help", "Aide"),
    ]
}

fn group(label: &'static str, content: impl IntoElement) -> gpui::Div {
    div()
        .h_full()
        .flex()
        .flex_col()
        .px(px(6.))
        .border_r_1()
        .border_color(c(theme::BORDER))
        .child(div().flex_1().flex().items_center().gap(px(2.)).child(content))
        .child(
            div()
                .h(px(16.))
                .flex()
                .justify_center()
                .text_size(px(10.5))
                .text_color(c(theme::TEXT_MUTED))
                .whitespace_nowrap()
                .child(label),
        )
}

fn rows(children: Vec<AnyElement>) -> gpui::Div {
    div().flex().flex_col().justify_center().gap(px(2.)).children(children)
}

fn row(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(1.))
        .children(children)
        .into_any_element()
}

fn sep() -> AnyElement {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx(px(3.))
        .bg(c(theme::BORDER))
        .into_any_element()
}

impl ExcelApp {
    fn small(
        &self,
        id: &'static str,
        content: impl IntoElement,
        tip: &'static str,
        active: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .relative()
            .h(px(ROW_H))
            .min_w(px(ROW_H))
            .px(px(3.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .rounded(px(4.))
            .when(active, |d| d.bg(c(theme::TOGGLED)))
            .hover(|s| s.bg(c(theme::HOVER)))
            .active(|s| s.bg(c(theme::PRESSED)))
            .child(content)
            .child(self.anchor(id))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
    }

    fn icon_btn(
        &self,
        id: &'static str,
        icon_name: &'static str,
        tip: &'static str,
        active: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        self.small(id, icon(icon_name, 16.0, theme::TEXT), tip, active, cx, f)
            .into_any_element()
    }

    /// A narrow caret that opens a drop-down next to a button.
    fn caret_btn(
        &self,
        id: &'static str,
        tip: &'static str,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .h(px(ROW_H))
            .w(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(icon("chevron-down", 9.0, theme::TEXT_MUTED))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
            .into_any_element()
    }

    /// Whether a labelled button still shows its text at the current
    /// ribbon width (groups lose their labels progressively, as in Office).
    fn label_visible(&self, id: &str) -> bool {
        let level = self.ribbon_level;
        match id {
            "rb-cut" | "rb-copy" | "rb-painter" => level < 1,
            "rb-wrap" | "rb-merge" => level < 3,
            "rb-insert" | "rb-delete" | "rb-format" | "rb-fill-series" | "rb-clear" | "rb-sum" => level < 4,
            _ => true,
        }
    }

    fn labeled(
        &self,
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        dropdown: bool,
        active: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        self.small(
            id,
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(2.))
                .child(icon(icon_name, 16.0, theme::TEXT))
                .when(self.label_visible(id), |d| d.child(div().whitespace_nowrap().child(label)))
                .when(dropdown, |d| d.child(icon("chevron-down", 9.0, theme::TEXT_MUTED))),
            label,
            active,
            cx,
            f,
        )
        .into_any_element()
    }

    fn large(
        &self,
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        dropdown: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        // Break long labels onto two lines like Office does.
        let (line1, line2) = match label.rfind(' ') {
            Some(i) if label.chars().count() > 11 => (&label[..i], &label[i + 1..]),
            _ => (label, ""),
        };
        div()
            .id(id)
            .relative()
            .h(px(68.))
            .min_w(px(44.))
            .px(px(5.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .pt(px(4.))
            .rounded(px(4.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .active(|s| s.bg(c(theme::PRESSED)))
            .child(icon(icon_name, 26.0, theme::TEXT))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .text_size(px(11.))
                    .line_height(px(13.))
                    .whitespace_nowrap()
                    .child(line1.to_string())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(line2.to_string())
                            .when(dropdown, |d| d.child(icon("chevron-down", 8.0, theme::TEXT_MUTED))),
                    ),
            )
            .child(self.anchor(id))
            .tooltip(tooltip(label))
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
            .into_any_element()
    }

    fn combo(
        &self,
        id: &'static str,
        text: impl Into<SharedString>,
        width: f32,
        tip: &'static str,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .relative()
            .w(px(width))
            .h(px(ROW_H - 2.0))
            .flex()
            .items_center()
            .justify_between()
            .px(px(6.))
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(4.))
            .hover(|s| s.border_color(c(theme::TEXT_MUTED)))
            .child(div().truncate().child(text.into()))
            .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
            .child(self.anchor(id))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
            .into_any_element()
    }

    fn glyph(text: &'static str, bold: bool, italic: bool, underline: bool) -> gpui::Div {
        div()
            .w(px(16.))
            .flex()
            .justify_center()
            .text_size(px(14.))
            .when(bold, |d| d.font_weight(FontWeight::BOLD))
            .when(italic, |d| d.italic())
            .when(underline, |d| d.underline())
            .child(text)
    }

    /// A button with a colour bar underneath (fill / font colour).
    fn color_button(&self, id: &'static str, top: impl IntoElement, color: Option<u32>, tip: &'static str, cx: &mut Cx, f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static) -> AnyElement {
        self.small(
            id,
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(top)
                .child(
                    div()
                        .w(px(16.))
                        .h(px(3.5))
                        .bg(color.map(c).unwrap_or(c(0xFFFFFF)))
                        .when(color.is_none(), |d| d.border_1().border_color(c(theme::BORDER_STRONG))),
                ),
            tip,
            false,
            cx,
            f,
        )
        .into_any_element()
    }

    pub fn render_ribbon_tabs(&mut self, cx: &mut Cx) -> impl IntoElement {
        let names = tab_names();
        let mut tabs: Vec<AnyElement> = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let active = i == self.ribbon_tab;
            tabs.push(
                div()
                    .id(("ribbon-tab", i))
                    .relative()
                    .h_full()
                    .px(px(11.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .text_size(px(13.))
                    .when(active, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(c(theme::ACCENT_DARK)))
                    .when(!active, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                    .rounded(px(4.))
                    .child(*name)
                    .when(active, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(2.))
                                .left(px(11.))
                                .right(px(11.))
                                .h(px(3.))
                                .rounded(px(2.))
                                .bg(c(theme::ACCENT)),
                        )
                    })
                    .when(i == 0, |d| d.child(self.anchor("file-tab")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if i == 0 {
                            this.open_file_menu(window, cx);
                        } else {
                            this.ribbon_tab = i;
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            );
        }
        div()
            .h(px(theme::TABS_H))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .bg(c(theme::CHROME_BG))
            .child(div().h_full().flex().items_center().children(tabs))
            .child(div().flex_1())
            .child(
                div()
                    .id("btn-comments")
                    .h(px(26.))
                    .px_3()
                    .mr_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .bg(c(0xFFFFFF))
                    .child(icon("comment", 14.0, theme::TEXT))
                    .child(t("Comments", "Commentaires"))
                    .on_click(cx.listener(|this, _, _, cx| this.not_implemented(t("Comments", "Commentaires"), cx))),
            )
            .child(
                div()
                    .id("btn-share")
                    .h(px(26.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .bg(c(theme::ACCENT))
                    .text_color(c(0xFFFFFF))
                    .child(icon("share", 14.0, 0xFFFFFF))
                    .child(t("Share", "Partager"))
                    .on_click(cx.listener(|this, _, _, cx| this.not_implemented(t("Share", "Partager"), cx))),
            )
    }

    fn open_file_menu(&mut self, window: &mut Window, cx: &mut Cx) {
        let entries = vec![
            MenuEntry::new(t("New", "Nouveau"), |a, w, cx| a.new_workbook(w, cx))
                .icon("new")
                .shortcut("⌘N"),
            MenuEntry::new(t("Open…", "Ouvrir…"), |a, w, cx| a.open(w, cx))
                .icon("folder")
                .shortcut("⌘O"),
            MenuEntry::separator(),
            MenuEntry::new(t("Save", "Enregistrer"), |a, w, cx| a.save(false, w, cx))
                .icon("save")
                .shortcut("⌘S"),
            MenuEntry::new(t("Save As…", "Enregistrer sous…"), |a, w, cx| a.save(true, w, cx)).shortcut("⇧⌘S"),
            MenuEntry::separator(),
            MenuEntry::new(t("Print…", "Imprimer…"), |a, _, cx| a.not_implemented(t("Print", "Imprimer"), cx))
                .icon("print")
                .shortcut("⌘P"),
            MenuEntry::separator(),
        ];
        let mut entries = entries;
        // Language of the interface and of formulas.
        let current = crate::i18n::lang();
        for lang in crate::i18n::Lang::ALL {
            entries.push(
                MenuEntry::new(lang.display_name(), move |a, _, cx| a.set_language(lang, cx)).checked(lang == current),
            );
        }
        self.open_menu("file-tab", entries, 230.0, window, cx);
    }

    pub fn render_ribbon(&mut self, window: &mut Window, cx: &mut Cx) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        self.ribbon_level = if width >= 1730.0 {
            0
        } else if width >= 1580.0 {
            1
        } else if width >= 1510.0 {
            2
        } else if width >= 1290.0 {
            3
        } else {
            4
        };
        let groups: Vec<AnyElement> = match self.ribbon_tab {
            2 => self.insert_tab(cx),
            3 => self.simple_tab(
                cx,
                &[
                    ("brush", crate::i18n::t_ctx("button", "Draw", "Dessiner")),
                    ("clear", t("Eraser", "Gomme")),
                    ("shapes", t("Ink to Shape", "Entrée manuscrite en forme")),
                ],
                t("Tools", "Outils"),
            ),
            4 => self.layout_tab(cx),
            5 => self.formulas_tab(cx),
            6 => self.data_tab(cx),
            7 => self.simple_tab(
                cx,
                &[
                    ("spell", t("Spelling", "Orthographe")),
                    ("book", t("Thesaurus", "Dictionnaire des synonymes")),
                    ("comment", t("New Comment", "Nouveau commentaire")),
                    ("lock", t("Protect Sheet", "Protéger la feuille")),
                    ("lock", t("Protect Workbook", "Protéger le classeur")),
                ],
                t("Review", "Révision"),
            ),
            8 => self.view_tab(cx),
            9 => self.simple_tab(
                cx,
                &[("function", t("New Script", "Nouveau script")), ("book", t("All Scripts", "Tous les scripts"))],
                t("Office Scripts", "Scripts Office"),
            ),
            10 => self.simple_tab(
                cx,
                &[("book", t("Help", "Aide")), ("comment", t("Feedback", "Commentaires"))],
                t("Help", "Aide"),
            ),
            _ => self.home_tab(cx),
        };
        div().w_full().flex_none().px_2().pb(px(4.)).bg(c(theme::CHROME_BG)).child(
            div()
                .id("ribbon")
                .h(px(theme::RIBBON_H))
                .w_full()
                .flex()
                .items_center()
                .overflow_x_scroll()
                .bg(c(theme::PANEL_BG))
                .border_1()
                .border_color(c(theme::BORDER))
                .rounded(px(8.))
                .py(px(3.))
                .px(px(2.))
                .children(groups),
        )
    }

    fn simple_tab(&self, cx: &mut Cx, items: &[(&'static str, &'static str)], label: &'static str) -> Vec<AnyElement> {
        let ids = [
            "st-0", "st-1", "st-2", "st-3", "st-4", "st-5", "st-6", "st-7", "st-8", "st-9",
        ];
        let buttons: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(i, (ic, name))| {
                let name: &'static str = name;
                self.large(ids[i % ids.len()], ic, name, false, cx, move |this, _, cx| {
                    this.not_implemented(name, cx)
                })
            })
            .collect();
        vec![group(label, row(buttons)).into_any_element()]
    }

    fn home_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let style = self.active_style().clone();
        let merged = {
            let (r, col) = self.active();
            self.wb.sheets[self.sheet].merge_at(r, col).is_some()
        };
        let font_name = style.font.name.to_string();
        let size = style.font.size_pt();
        let size_text = if (size - size.round()).abs() < 0.01 {
            format!("{:.0}", size)
        } else {
            format!("{}", size)
        };
        let fmt_name = number_format_label(&style.num_fmt);

        // Clipboard
        let clipboard = group(
            t("Clipboard", "Presse-papiers"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(self.large("rb-paste", "paste", t("Paste", "Coller"), true, cx, |this, _, cx| {
                    this.paste(PasteMode::All, cx)
                }))
                .child(rows(vec![
                    self.labeled("rb-cut", "cut", t("Cut", "Couper"), false, false, cx, |this, _, cx| {
                        this.copy(true, cx)
                    }),
                    self.labeled("rb-copy", "copy", t("Copy", "Copier"), false, false, cx, |this, _, cx| {
                        this.copy(false, cx)
                    }),
                    self.labeled(
                        "rb-painter",
                        "brush",
                        t("Format Painter", "Reproduire la mise en forme"),
                        false,
                        false,
                        cx,
                        |this, _, cx| {
                            this.start_format_painter(cx);
                        },
                    ),
                ])),
        );

        // Font
        let font = group(
            t("Font", "Police"),
            rows(vec![
                row(vec![
                    self.combo("rb-font", font_name, 138.0, t("Font", "Police"), cx, |this, window, cx| {
                        this.open_font_menu(window, cx)
                    }),
                    self.combo("rb-size", size_text, 50.0, t("Font Size", "Taille de police"), cx, |this, window, cx| {
                        let entries = Self::FONT_SIZES
                            .iter()
                            .map(|s| {
                                let s = *s;
                                MenuEntry::new(format!("{:.0}", s), move |a, _, cx| a.set_font_size(s, cx))
                            })
                            .collect();
                        this.open_menu("rb-size", entries, 70.0, window, cx);
                    }),
                    self.icon_btn(
                        "rb-grow",
                        "font-grow",
                        t("Increase Font Size", "Augmenter la taille de police"),
                        false,
                        cx,
                        |this, _, cx| this.step_font_size(true, cx),
                    ),
                    self.icon_btn(
                        "rb-shrink",
                        "font-shrink",
                        t("Decrease Font Size", "Réduire la taille de police"),
                        false,
                        cx,
                        |this, _, cx| this.step_font_size(false, cx),
                    ),
                ]),
                row(vec![
                    self.small(
                        "rb-bold",
                        Self::glyph(t("B", "G"), true, false, false),
                        t("Bold (⌘B)", "Gras (⌘B)"),
                        style.font.bold,
                        cx,
                        |this, _, cx| this.toggle_bold(cx),
                    )
                    .into_any_element(),
                    self.small(
                        "rb-italic",
                        Self::glyph(t("I", "I"), false, true, false),
                        t("Italic (⌘I)", "Italique (⌘I)"),
                        style.font.italic,
                        cx,
                        |this, _, cx| this.toggle_italic(cx),
                    )
                    .into_any_element(),
                    self.small(
                        "rb-underline",
                        Self::glyph(t("U", "S"), false, false, true),
                        t("Underline (⌘U)", "Souligné (⌘U)"),
                        style.font.underline != Underline::None,
                        cx,
                        |this, _, cx| this.toggle_underline(cx),
                    )
                    .into_any_element(),
                    self.small(
                        "rb-strike",
                        div().w(px(16.)).flex().justify_center().text_size(px(13.)).line_through().child("ab"),
                        t("Strikethrough", "Barré"),
                        style.font.strike,
                        cx,
                        |this, _, cx| this.toggle_strike(cx),
                    )
                    .into_any_element(),
                    sep(),
                    self.icon_btn(
                        "rb-borders",
                        "borders",
                        t("Borders", "Bordures"),
                        false,
                        cx,
                        |this, _, cx| {
                            let preset = this.last_border;
                            this.apply_border(preset, cx)
                        },
                    ),
                    self.caret_btn("rb-borders-menu", t("Borders", "Bordures"), cx, |this, window, cx| {
                        this.open_border_menu(window, cx)
                    }),
                    sep(),
                    self.color_button(
                        "rb-fill",
                        icon("fill", 15.0, theme::TEXT),
                        self.last_fill.map(|col| col.0),
                        t("Fill Color", "Couleur de remplissage"),
                        cx,
                        |this, _, cx| {
                            let color = this.last_fill;
                            this.set_fill(color, cx)
                        },
                    ),
                    self.caret_btn(
                        "rb-fill-menu",
                        t("Fill Color", "Couleur de remplissage"),
                        cx,
                        |this, window, cx| this.open_palette("rb-fill", ColorTarget::Fill, window, cx),
                    ),
                    self.color_button(
                        "rb-font-color",
                        div().text_size(px(13.)).line_height(px(13.)).font_weight(FontWeight::SEMIBOLD).child("A"),
                        self.last_font_color.map(|col| col.0),
                        t("Font Color", "Couleur de police"),
                        cx,
                        |this, _, cx| {
                            let color = this.last_font_color;
                            this.set_font_color(color, cx)
                        },
                    ),
                    self.caret_btn("rb-font-color-menu", t("Font Color", "Couleur de police"), cx, |this, window, cx| {
                        this.open_palette("rb-font-color", ColorTarget::Font, window, cx)
                    }),
                ]),
            ]),
        );

        // Alignment
        let alignment = group(
            t("Alignment", "Alignement"),
            rows(vec![
                row(vec![
                    self.icon_btn(
                        "rb-top",
                        "align-top",
                        t("Top Align", "Aligner en haut"),
                        style.v_align == VAlign::Top,
                        cx,
                        |this, _, cx| this.set_v_align(VAlign::Top, cx),
                    ),
                    self.icon_btn(
                        "rb-middle",
                        "align-middle",
                        t("Middle Align", "Aligner au centre"),
                        style.v_align == VAlign::Center,
                        cx,
                        |this, _, cx| this.set_v_align(VAlign::Center, cx),
                    ),
                    self.icon_btn(
                        "rb-bottom",
                        "align-bottom",
                        t("Bottom Align", "Aligner en bas"),
                        style.v_align == VAlign::Bottom,
                        cx,
                        |this, _, cx| this.set_v_align(VAlign::Bottom, cx),
                    ),
                    sep(),
                    self.icon_btn(
                        "rb-orient",
                        "orientation",
                        t("Orientation", "Orientation"),
                        false,
                        cx,
                        |this, _, cx| this.not_implemented(t("Orientation", "Orientation"), cx),
                    ),
                    sep(),
                    self.labeled(
                        "rb-wrap",
                        "wrap",
                        t("Wrap Text", "Renvoyer à la ligne automatiquement"),
                        false,
                        style.wrap,
                        cx,
                        |this, _, cx| this.toggle_wrap(cx),
                    ),
                ]),
                row(vec![
                    self.icon_btn(
                        "rb-left",
                        "align-left",
                        t("Align Left", "Aligner à gauche"),
                        style.h_align == HAlign::Left,
                        cx,
                        |this, _, cx| this.set_h_align(HAlign::Left, cx),
                    ),
                    self.icon_btn(
                        "rb-center",
                        "align-center",
                        t("Center", "Centrer"),
                        style.h_align == HAlign::Center,
                        cx,
                        |this, _, cx| this.set_h_align(HAlign::Center, cx),
                    ),
                    self.icon_btn(
                        "rb-right",
                        "align-right",
                        t("Align Right", "Aligner à droite"),
                        style.h_align == HAlign::Right,
                        cx,
                        |this, _, cx| this.set_h_align(HAlign::Right, cx),
                    ),
                    sep(),
                    self.icon_btn(
                        "rb-outdent",
                        "indent-dec",
                        t("Decrease Indent", "Diminuer le retrait"),
                        false,
                        cx,
                        |this, _, cx| this.change_indent(-1, cx),
                    ),
                    self.icon_btn(
                        "rb-indent",
                        "indent-inc",
                        t("Increase Indent", "Augmenter le retrait"),
                        false,
                        cx,
                        |this, _, cx| this.change_indent(1, cx),
                    ),
                    sep(),
                    self.labeled(
                        "rb-merge",
                        "merge",
                        t("Merge & Center", "Fusionner et centrer"),
                        false,
                        merged,
                        cx,
                        |this, _, cx| this.toggle_merge(true, cx),
                    ),
                ]),
            ]),
        );

        // Number
        let number = group(
            t("Number", "Nombre"),
            rows(vec![
                row(vec![self.combo(
                    "rb-numfmt",
                    fmt_name,
                    150.0,
                    t("Number Format", "Format de nombre"),
                    cx,
                    |this, window, cx| this.open_number_format_menu(window, cx),
                )]),
                row(vec![
                    self.icon_btn(
                        "rb-currency",
                        "currency",
                        t("Accounting Number Format", "Format Nombre Comptabilité"),
                        false,
                        cx,
                        |this, _, cx| {
                            let code = this.wb.locale.accounting_format;
                            this.set_number_format(code, cx)
                        },
                    ),
                    self.small(
                        "rb-percent",
                        Self::glyph("%", false, false, false),
                        t("Percent Style", "Style de pourcentage"),
                        false,
                        cx,
                        |this, _, cx| this.set_number_format("0%", cx),
                    )
                    .into_any_element(),
                    self.small(
                        "rb-comma",
                        div().w(px(22.)).flex().justify_center().text_size(px(11.)).child("000"),
                        t("Comma Style", "Séparateur de milliers"),
                        false,
                        cx,
                        |this, _, cx| this.set_number_format("#,##0.00", cx),
                    )
                    .into_any_element(),
                    sep(),
                    self.icon_btn(
                        "rb-inc-dec",
                        "inc-decimal",
                        t("Increase Decimal", "Ajouter une décimale"),
                        false,
                        cx,
                        |this, _, cx| this.change_decimals(1, cx),
                    ),
                    self.icon_btn(
                        "rb-dec-dec",
                        "dec-decimal",
                        t("Decrease Decimal", "Réduire les décimales"),
                        false,
                        cx,
                        |this, _, cx| this.change_decimals(-1, cx),
                    ),
                ]),
            ]),
        );

        // Styles
        let ni_style = |this: &Self, id: &'static str, ic: &'static str, label: &'static str, cx: &mut Cx| {
            if this.ribbon_level >= 2 {
                this.labeled(id, ic, label, true, false, cx, move |this, _, cx| this.not_implemented(label, cx))
            } else {
                this.large(id, ic, label, true, cx, move |this, _, cx| this.not_implemented(label, cx))
            }
        };
        let style_buttons = vec![
            ni_style(
                self,
                "rb-cond",
                "cond-format",
                t("Conditional Formatting", "Mise en forme conditionnelle"),
                cx,
            ),
            ni_style(
                self,
                "rb-table",
                "table",
                t("Format as Table", "Mettre sous forme de tableau"),
                cx,
            ),
            ni_style(self, "rb-cellstyles", "cell-styles", t("Cell Styles", "Styles de cellules"), cx),
        ];
        let styles = group(
            t("Styles", "Styles"),
            if self.ribbon_level >= 2 {
                rows(style_buttons).into_any_element()
            } else {
                row(style_buttons)
            },
        );

        // Cells
        let cells = group(
            t("Cells", "Cellules"),
            rows(vec![
                self.labeled("rb-insert", "insert-cells", t("Insert", "Insérer"), true, false, cx, |this, window, cx| {
                    let entries = vec![
                        MenuEntry::new(t("Insert Sheet Rows", "Insérer des lignes dans la feuille"), |a, _, cx| {
                            a.insert_rows(cx)
                        }),
                        MenuEntry::new(
                            t("Insert Sheet Columns", "Insérer des colonnes dans la feuille"),
                            |a, _, cx| a.insert_cols(cx),
                        ),
                        MenuEntry::separator(),
                        MenuEntry::new(t("Insert Sheet", "Insérer une feuille"), |a, _, cx| a.add_sheet(cx)),
                    ];
                    this.open_menu("rb-insert", entries, 270.0, window, cx);
                }),
                self.labeled("rb-delete", "delete-cells", t("Delete", "Supprimer"), true, false, cx, |this, window, cx| {
                    let entries = vec![
                        MenuEntry::new(t("Delete Sheet Rows", "Supprimer des lignes dans la feuille"), |a, _, cx| {
                            a.delete_rows(cx)
                        }),
                        MenuEntry::new(
                            t("Delete Sheet Columns", "Supprimer des colonnes dans la feuille"),
                            |a, _, cx| a.delete_cols(cx),
                        ),
                        MenuEntry::separator(),
                        MenuEntry::new(t("Delete Sheet", "Supprimer une feuille"), |a, _, cx| {
                            let i = a.sheet;
                            a.delete_sheet(i, cx)
                        }),
                    ];
                    this.open_menu("rb-delete", entries, 270.0, window, cx);
                }),
                self.labeled("rb-format", "format-cells", t("Format", "Format"), true, false, cx, |this, window, cx| {
                    let entries = vec![
                        MenuEntry::new(
                            t("AutoFit Column Width", "Ajuster la largeur de colonne"),
                            |a, window, cx| {
                                let r = a.wb.clamp_to_used(a.sheet, a.sel().current());
                                for col in r.c0..=r.c1.min(r.c0 + 500) {
                                    a.autofit_col(col, window, cx);
                                }
                            },
                        ),
                        MenuEntry::separator(),
                        MenuEntry::new(t("Hide Rows", "Masquer les lignes"), |a, _, cx| a.hide_rows(true, cx)),
                        MenuEntry::new(t("Hide Columns", "Masquer les colonnes"), |a, _, cx| a.hide_cols(true, cx)),
                        MenuEntry::new(t("Unhide Rows", "Afficher les lignes"), |a, _, cx| a.hide_rows(false, cx)),
                        MenuEntry::new(t("Unhide Columns", "Afficher les colonnes"), |a, _, cx| {
                            a.hide_cols(false, cx)
                        }),
                        MenuEntry::separator(),
                        MenuEntry::new(t("Rename Sheet", "Renommer la feuille"), |a, window, cx| {
                            let i = a.sheet;
                            a.begin_rename_sheet(i, window, cx)
                        }),
                        MenuEntry::new(t("Tab Color…", "Couleur d'onglet…"), |a, window, cx| {
                            a.open_palette("rb-format", ColorTarget::Tab, window, cx)
                        }),
                    ];
                    this.open_menu("rb-format", entries, 250.0, window, cx);
                }),
            ]),
        );

        // Editing
        let editing = group(
            t("Editing", "Édition"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(rows(vec![
                    row(vec![
                        self.small(
                            "rb-sum",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .px(px(2.))
                                .child(div().text_size(px(15.)).child("Σ"))
                                .when(self.label_visible("rb-sum"), |d| d.child(t("AutoSum", "Somme automatique"))),
                            t("AutoSum", "Somme automatique"),
                            false,
                            cx,
                            |this, window, cx| this.auto_sum("SUM", window, cx),
                        )
                        .into_any_element(),
                        self.caret_btn("rb-sum-menu", t("AutoSum", "Somme automatique"), cx, |this, window, cx| {
                            let mk = |label: &'static str, func: &'static str| {
                                MenuEntry::new(label, move |a, window, cx| a.auto_sum(func, window, cx))
                            };
                            let entries = vec![
                                mk(t("Sum", "Somme"), "SUM"),
                                mk(t("Average", "Moyenne"), "AVERAGE"),
                                mk(t("Count Numbers", "Nb"), "COUNT"),
                                mk(t("Max", "Max"), "MAX"),
                                mk(t("Min", "Min"), "MIN"),
                            ];
                            this.open_menu("rb-sum", entries, 180.0, window, cx);
                        }),
                    ]),
                    self.labeled("rb-fill-series", "fill-down", t("Fill", "Remplissage"), true, false, cx, |this, window, cx| {
                        let entries = vec![
                            MenuEntry::new(t("Down", "En bas"), |a, _, cx| {
                                let r = a.wb.clamp_to_used(a.sheet, a.sel().current());
                                a.wb.fill_down(a.sheet, r);
                                a.commit(t("Fill Down", "Recopier vers le bas"), cx);
                            })
                            .shortcut("⌘D"),
                            MenuEntry::new(t("Right", "À droite"), |a, _, cx| {
                                let r = a.wb.clamp_to_used(a.sheet, a.sel().current());
                                a.wb.fill_right(a.sheet, r);
                                a.commit(t("Fill Right", "Recopier à droite"), cx);
                            })
                            .shortcut("⌘R"),
                        ];
                        this.open_menu("rb-fill-series", entries, 180.0, window, cx);
                    }),
                    self.labeled("rb-clear", "clear", t("Clear", "Effacer"), true, false, cx, |this, window, cx| {
                        let entries = vec![
                            MenuEntry::new(t("Clear All", "Effacer tout"), |a, _, cx| a.clear(ClearMode::All, cx)),
                            MenuEntry::new(t("Clear Formats", "Effacer les formats"), |a, _, cx| {
                                a.clear(ClearMode::Formats, cx)
                            }),
                            MenuEntry::new(t("Clear Contents", "Effacer le contenu"), |a, _, cx| {
                                a.clear(ClearMode::Contents, cx)
                            }),
                        ];
                        this.open_menu("rb-clear", entries, 200.0, window, cx);
                    }),
                ]))
                .child(self.large(
                    "rb-sort",
                    "sort-filter",
                    t("Sort & Filter", "Trier et filtrer"),
                    true,
                    cx,
                    |this, window, cx| {
                        let entries = vec![
                            MenuEntry::new(t("Sort A to Z", "Trier de A à Z"), |a, _, cx| a.sort(true, cx)).icon("sort-asc"),
                            MenuEntry::new(t("Sort Z to A", "Trier de Z à A"), |a, _, cx| a.sort(false, cx))
                                .icon("sort-desc"),
                            MenuEntry::separator(),
                            MenuEntry::new(t("Filter", "Filtrer"), |a, _, cx| a.not_implemented(t("Filter", "Filtrer"), cx))
                                .icon("filter"),
                        ];
                        this.open_menu("rb-sort", entries, 200.0, window, cx);
                    },
                ))
                .child(self.large(
                    "rb-find",
                    "search",
                    t("Find & Select", "Rechercher et sélectionner"),
                    true,
                    cx,
                    |this, window, cx| this.toggle_find(window, cx),
                )),
        );

        vec![
            clipboard.into_any_element(),
            font.into_any_element(),
            alignment.into_any_element(),
            number.into_any_element(),
            styles.into_any_element(),
            cells.into_any_element(),
            editing.border_r_0().into_any_element(),
        ]
    }

    fn open_font_menu(&mut self, window: &mut Window, cx: &mut Cx) {
        let current = self.active_style().font.name.to_string();
        let mut names: Vec<String> = Vec::new();
        for office in ["Aptos Narrow", "Aptos", "Calibri", "Arial", "Times New Roman", "Courier New"] {
            names.push(office.to_string());
        }
        for n in &self.fonts.names {
            if !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                names.push(n.clone());
            }
        }
        let entries = names
            .into_iter()
            .map(|n| {
                let checked = n.eq_ignore_ascii_case(&current);
                let name = n.clone();
                MenuEntry::new(n, move |a, _, cx| a.set_font_name(&name, cx)).checked(checked)
            })
            .collect();
        self.open_menu("rb-font", entries, 240.0, window, cx);
    }

    fn open_border_menu(&mut self, window: &mut Window, cx: &mut Cx) {
        let mk = |label: &'static str, ic: &'static str, preset: BorderPreset| {
            MenuEntry::new(label, move |a, _, cx| a.apply_border(preset, cx)).icon(ic)
        };
        let entries = vec![
            mk(t("Bottom Border", "Bordure inférieure"), "border-bottom", BorderPreset::Bottom),
            mk(t("Top Border", "Bordure supérieure"), "border-top", BorderPreset::Top),
            mk(t("Left Border", "Bordure gauche"), "border-left", BorderPreset::Left),
            mk(t("Right Border", "Bordure droite"), "border-right", BorderPreset::Right),
            MenuEntry::separator(),
            mk(t("No Border", "Aucune bordure"), "border-none", BorderPreset::None),
            mk(t("All Borders", "Toutes les bordures"), "borders", BorderPreset::All),
            mk(t("Outside Borders", "Bordures extérieures"), "border-outside", BorderPreset::Outside),
            mk(
                t("Thick Outside Borders", "Bordure extérieure épaisse"),
                "border-thick",
                BorderPreset::ThickOutside,
            ),
            MenuEntry::separator(),
            mk(
                t("Bottom Double Border", "Bordure double en bas"),
                "border-bottom",
                BorderPreset::BottomDouble,
            ),
            mk(
                t("Thick Bottom Border", "Bordure épaisse en bas"),
                "border-bottom",
                BorderPreset::ThickBottom,
            ),
            mk(
                t("Top and Bottom Border", "Bordure en haut et en bas"),
                "border-top",
                BorderPreset::TopAndBottom,
            ),
        ];
        self.open_menu("rb-borders", entries, 250.0, window, cx);
    }

    fn open_number_format_menu(&mut self, window: &mut Window, cx: &mut Cx) {
        let locale = self.wb.locale;
        let current = self.active_style().num_fmt.to_string();
        let items: Vec<(&'static str, String)> = vec![
            (t("General", "Standard"), "General".to_string()),
            (crate::i18n::t_ctx("format", "Number", "Nombre"), "0.00".to_string()),
            (t("Currency", "Monétaire"), locale.currency_format.to_string()),
            (t("Accounting", "Comptabilité"), locale.accounting_format.to_string()),
            (t("Short Date", "Date courte"), locale.short_date_format.to_string()),
            (t("Long Date", "Date longue"), locale.long_date_format.to_string()),
            (t("Time", "Heure"), locale.time_format.to_string()),
            (t("Percentage", "Pourcentage"), "0.00%".to_string()),
            (t("Fraction", "Fraction"), "# ?/?".to_string()),
            (t("Scientific", "Scientifique"), "0.00E+00".to_string()),
            (t("Text", "Texte"), "@".to_string()),
        ];
        let entries = items
            .into_iter()
            .map(|(label, code)| {
                let checked = code == current;
                MenuEntry::new(label, move |a, _, cx| a.set_number_format(&code, cx)).checked(checked)
            })
            .collect();
        self.open_menu("rb-numfmt", entries, 200.0, window, cx);
    }

    /// Menu of functions, optionally restricted to one category.
    pub fn open_function_menu(&mut self, anchor: &'static str, category: Option<Category>, window: &mut Window, cx: &mut Cx) {
        let locale = self.wb.locale;
        let mut entries: Vec<MenuEntry> = Vec::new();
        for def in functions::all() {
            if category.is_some_and(|cat| def.category != cat) {
                continue;
            }
            let canonical = def.name;
            entries.push(MenuEntry::new(locale.fn_to_local(canonical).to_string(), move |a, window, cx| {
                a.insert_function(canonical, window, cx)
            }));
        }
        entries.sort_by(|a, b| a.label.cmp(&b.label));
        self.open_menu(anchor, entries, 240.0, window, cx);
    }

    fn insert_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let ni = |this: &Self, id: &'static str, ic: &'static str, label: &'static str, cx: &mut Cx| {
            this.large(id, ic, label, false, cx, move |this, _, cx| this.not_implemented(label, cx))
        };
        vec![
            group(
                t("Tables", "Tableaux"),
                row(vec![
                    ni(self, "in-pivot", "pivot", t("PivotTable", "Tableau croisé dynamique"), cx),
                    ni(self, "in-table", "table", t("Table", "Tableau"), cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Illustrations", "Illustrations"),
                row(vec![
                    ni(self, "in-pic", "picture", t("Pictures", "Images"), cx),
                    ni(self, "in-shapes", "shapes", t("Shapes", "Formes"), cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Charts", "Graphiques"),
                row(vec![
                    ni(self, "in-chart-rec", "chart-bar", t("Recommended Charts", "Graphiques recommandés"), cx),
                    ni(self, "in-chart-bar", "chart-bar", t("Column", "Histogramme"), cx),
                    ni(self, "in-chart-line", "chart-line", t("Line", "Courbes"), cx),
                    ni(self, "in-chart-pie", "chart-pie", t("Pie", "Secteurs"), cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Links", "Liens"),
                row(vec![ni(self, "in-link", "link", t("Link", "Lien"), cx)]),
            )
            .into_any_element(),
            group(
                t("Comments", "Commentaires"),
                row(vec![ni(self, "in-comment", "comment", t("Comment", "Commentaire"), cx)]),
            )
            .into_any_element(),
            group(
                t("Text", "Texte"),
                row(vec![ni(self, "in-textbox", "textbox", t("Text Box", "Zone de texte"), cx)]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn layout_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let ni = |this: &Self, id: &'static str, ic: &'static str, label: &'static str, cx: &mut Cx| {
            this.large(id, ic, label, true, cx, move |this, _, cx| this.not_implemented(label, cx))
        };
        vec![
            group(
                t("Themes", "Thèmes"),
                row(vec![ni(self, "pl-themes", "cell-styles", t("Themes", "Thèmes"), cx)]),
            )
            .into_any_element(),
            group(
                t("Page Setup", "Mise en page"),
                row(vec![
                    ni(self, "pl-margins", "page", t("Margins", "Marges"), cx),
                    ni(self, "pl-orient", "page", t("Orientation", "Orientation"), cx),
                    ni(self, "pl-size", "page", t("Size", "Taille"), cx),
                    ni(self, "pl-area", "print", t("Print Area", "Zone d'impression"), cx),
                ]),
            )
            .into_any_element(),
            self.sheet_options_group(cx).border_r_0().into_any_element(),
        ]
    }

    fn check(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .h(px(ROW_H - 2.0))
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(4.))
            .rounded(px(4.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(c(if on { theme::ACCENT } else { theme::TEXT_MUTED }))
                    .when(on, |d| d.bg(c(theme::ACCENT)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(on, |d| d.child(icon("check", 11.0, 0xFFFFFF))),
            )
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
            .into_any_element()
    }

    fn sheet_options_group(&self, cx: &mut Cx) -> gpui::Div {
        let sheet = &self.wb.sheets[self.sheet];
        group(
            t("Show", "Afficher"),
            rows(vec![
                self.check("vw-grid", t("Gridlines", "Quadrillage"), sheet.show_gridlines, cx, |this, _, cx| {
                    this.wb.snapshot();
                    let s = &mut this.wb.sheets[this.sheet];
                    s.show_gridlines = !s.show_gridlines;
                    this.commit(t("Gridlines", "Quadrillage"), cx);
                }),
                self.check("vw-head", t("Headings", "En-têtes"), sheet.show_headers, cx, |this, _, cx| {
                    this.wb.snapshot();
                    let s = &mut this.wb.sheets[this.sheet];
                    s.show_headers = !s.show_headers;
                    this.commit(t("Headings", "En-têtes"), cx);
                }),
                self.check(
                    "vw-fbar",
                    t("Formula Bar", "Barre de formule"),
                    self.show_formula_bar,
                    cx,
                    |this, _, cx| {
                        this.show_formula_bar = !this.show_formula_bar;
                        cx.notify();
                    },
                ),
            ]),
        )
    }

    fn formulas_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let lib = |this: &Self, id: &'static str, ic: &'static str, label: &'static str, cat: Category, cx: &mut Cx| {
            this.large(id, ic, label, true, cx, move |this, window, cx| {
                this.open_function_menu(id, Some(cat), window, cx)
            })
        };
        vec![
            group(
                t("Function Library", "Bibliothèque de fonctions"),
                row(vec![
                    self.large(
                        "fx-insert",
                        "function",
                        t("Insert Function", "Insérer une fonction"),
                        false,
                        cx,
                        |this, window, cx| this.open_function_menu("fx-insert", None, window, cx),
                    ),
                    self.large(
                        "fx-sum",
                        "calculator",
                        t("AutoSum", "Somme automatique"),
                        false,
                        cx,
                        |this, window, cx| this.auto_sum("SUM", window, cx),
                    ),
                    lib(self, "fx-fin", "currency", t("Financial", "Financier"), Category::Financial, cx),
                    lib(self, "fx-logic", "validation", t("Logical", "Logique"), Category::Logical, cx),
                    lib(self, "fx-text", "textbox", t("Text", "Texte"), Category::Text, cx),
                    lib(self, "fx-date", "clock", t("Date & Time", "Date et heure"), Category::DateTime, cx),
                    lib(
                        self,
                        "fx-lookup",
                        "search",
                        t("Lookup & Reference", "Recherche et référence"),
                        Category::Lookup,
                        cx,
                    ),
                    lib(
                        self,
                        "fx-math",
                        "calculator",
                        t("Math & Trig", "Maths et trigonométrie"),
                        Category::Math,
                        cx,
                    ),
                    lib(
                        self,
                        "fx-stat",
                        "chart-bar",
                        t("Statistical", "Statistiques"),
                        Category::Statistical,
                        cx,
                    ),
                    lib(
                        self,
                        "fx-info",
                        "book",
                        t("Information", "Informations"),
                        Category::Information,
                        cx,
                    ),
                    lib(
                        self,
                        "fx-eng",
                        "data",
                        t("Engineering", "Ingénierie"),
                        Category::Engineering,
                        cx,
                    ),
                ]),
            )
            .into_any_element(),
            group(
                t("Defined Names", "Noms définis"),
                row(vec![self.large(
                    "fx-names",
                    "tag",
                    t("Name Manager", "Gestionnaire de noms"),
                    false,
                    cx,
                    |this, _, cx| this.not_implemented(t("Name Manager", "Gestionnaire de noms"), cx),
                )]),
            )
            .into_any_element(),
            group(
                t("Calculation", "Calcul"),
                row(vec![self.large(
                    "fx-calc",
                    "refresh",
                    t("Calculate Now", "Calculer maintenant"),
                    false,
                    cx,
                    |this, _, cx| {
                        this.wb.recalc();
                        this.refresh(cx);
                    },
                )]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn data_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let ni = |this: &Self, id: &'static str, ic: &'static str, label: &'static str, cx: &mut Cx| {
            this.large(id, ic, label, false, cx, move |this, _, cx| this.not_implemented(label, cx))
        };
        vec![
            group(
                t("Get & Transform Data", "Récupérer et transformer des données"),
                row(vec![
                    ni(self, "dt-get", "data", t("Get Data", "Obtenir des données"), cx),
                    ni(self, "dt-refresh", "refresh", t("Refresh All", "Actualiser tout"), cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Sort & Filter", "Trier et filtrer"),
                row(vec![
                    self.large("dt-asc", "sort-asc", t("Sort A to Z", "Trier de A à Z"), false, cx, |this, _, cx| {
                        this.sort(true, cx)
                    }),
                    self.large("dt-desc", "sort-desc", t("Sort Z to A", "Trier de Z à A"), false, cx, |this, _, cx| {
                        this.sort(false, cx)
                    }),
                    ni(self, "dt-filter", "filter", t("Filter", "Filtrer"), cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Data Tools", "Outils de données"),
                row(vec![
                    ni(self, "dt-ttc", "columns", t("Text to Columns", "Convertir"), cx),
                    ni(self, "dt-dup", "delete-cells", t("Remove Duplicates", "Supprimer les doublons"), cx),
                    ni(self, "dt-valid", "validation", t("Data Validation", "Validation des données"), cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn view_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let frozen = {
            let s = &self.wb.sheets[self.sheet];
            s.frozen_rows > 0 || s.frozen_cols > 0
        };
        vec![
            group(
                t("Workbook Views", "Modes d'affichage"),
                row(vec![
                    self.large("vw-normal", "view-normal", t("Normal", "Normal"), false, cx, |_, _, _| {}),
                    self.large(
                        "vw-layout",
                        "view-layout",
                        t("Page Layout", "Mise en page"),
                        false,
                        cx,
                        |this, _, cx| this.not_implemented(t("Page Layout", "Mise en page"), cx),
                    ),
                ]),
            )
            .into_any_element(),
            self.sheet_options_group(cx).into_any_element(),
            group(
                t("Zoom", "Zoom"),
                row(vec![
                    self.large("vw-zoom-in", "zoom", t("Zoom In", "Zoom avant"), false, cx, |this, _, cx| {
                        let z = this.zoom + 0.1;
                        this.set_zoom(z, cx)
                    }),
                    self.large("vw-zoom-100", "page", "100 %", false, cx, |this, _, cx| this.set_zoom(1.0, cx)),
                    self.large("vw-zoom-out", "search", t("Zoom Out", "Zoom arrière"), false, cx, |this, _, cx| {
                        let z = this.zoom - 0.1;
                        this.set_zoom(z, cx)
                    }),
                ]),
            )
            .into_any_element(),
            group(
                t("Window", "Fenêtre"),
                row(vec![self.large(
                    "vw-freeze",
                    "freeze",
                    if frozen {
                        t("Unfreeze Panes", "Libérer les volets")
                    } else {
                        t("Freeze Panes", "Figer les volets")
                    },
                    false,
                    cx,
                    |this, _, cx| this.toggle_freeze(cx),
                )]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }
}

/// Name shown in the number-format box for a format code.
fn number_format_label(code: &str) -> &'static str {
    match format::format_kind(code) {
        FormatKind::General => t("General", "Standard"),
        FormatKind::Number => crate::i18n::t_ctx("format", "Number", "Nombre"),
        FormatKind::Currency => t("Currency", "Monétaire"),
        FormatKind::Accounting => t("Accounting", "Comptabilité"),
        FormatKind::Date | FormatKind::DateTime => t("Date", "Date"),
        FormatKind::Time => t("Time", "Heure"),
        FormatKind::Percent => t("Percentage", "Pourcentage"),
        FormatKind::Fraction => t("Fraction", "Fraction"),
        FormatKind::Scientific => t("Scientific", "Scientifique"),
        FormatKind::Text => t("Text", "Texte"),
        FormatKind::Custom => t("Custom", "Personnalisé"),
    }
}
