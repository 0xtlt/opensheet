//! The ribbon: tab strip and the command groups of each tab.
//!
//! The controls of features that live in their own module run that
//! module's commands (`features::Command`); contextual tabs are drawn by
//! the feature they belong to, with the helpers of this file.

use crate::app::*;
use crate::features::{
    self, Command, ViewMode, charts, comments, condfmt, data_tools, drawings, edit_tools, filters, format_cells, misc, names, page,
    pivot, protection, slicers, tables, themes, validation,
};
use crate::i18n::t;
use crate::ribbon_fold as fold;
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use gpui::{AnyElement, Context, FontWeight, SharedString, Stateful, Window, div, prelude::*, px};
use opensheet_engine::chart::ChartKind;
use opensheet_engine::format::{self, FormatKind};
use opensheet_engine::functions::{self, Category};
use opensheet_engine::ops::{BorderPreset, Direction, PasteMode};
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

/// A group of the ribbon: its controls over its name. Groups are separated
/// by a line; end the last one of a tab with `.border_r_0()`.
pub fn group(label: &'static str, content: impl IntoElement) -> gpui::Div {
    launcher_group(label, content, None)
}

/// A group with the small arrow that opens its dialog (see
/// `ExcelApp::launcher`).
pub fn launcher_group(label: &'static str, content: impl IntoElement, launcher: Option<AnyElement>) -> gpui::Div {
    // When the window is too narrow the group is folded into one button;
    // its drop-down is the group as it is built here (`ribbon_fold.rs`).
    let slot = fold::next_group();
    let whole = whole_group(label, content, launcher);
    if slot.mode != fold::Mode::Folded {
        return whole;
    }
    if slot.open {
        fold::show_panel(whole.border_r_0());
    }
    div()
        .h_full()
        .flex()
        .items_center()
        .px(px(4.))
        .border_r_1()
        .border_color(c(theme::BORDER))
        .child(fold::folded_button(label, &slot))
}

/// A group in full: its controls over its name.
fn whole_group(label: &'static str, content: impl IntoElement, launcher: Option<AnyElement>) -> gpui::Div {
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
                .relative()
                .h(px(16.))
                .flex()
                .justify_center()
                .text_size(px(10.5))
                .text_color(c(theme::TEXT_MUTED))
                .whitespace_nowrap()
                .child(label)
                .children(launcher.map(|l| div().absolute().right(px(-4.)).top(px(1.)).child(l))),
        )
}

/// A long label of a large button on two lines, like Office: broken at
/// the space nearest to its middle.
pub(crate) fn split_label(label: &str) -> (&str, &str) {
    let middle = label.len() / 2;
    let split = label
        .match_indices(' ')
        .map(|(i, _)| i)
        .min_by_key(|i| i.abs_diff(middle));
    match split {
        Some(i) if label.chars().count() > 11 => (&label[..i], &label[i + 1..]),
        _ => (label, ""),
    }
}

/// Controls stacked in up to three rows.
pub fn rows(children: Vec<AnyElement>) -> gpui::Div {
    div().flex().flex_col().justify_center().gap(px(2.)).children(children)
}

/// Controls side by side.
pub fn row(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(1.))
        .children(children)
        .into_any_element()
}

/// A thin vertical line between controls of a row.
pub fn sep() -> AnyElement {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx(px(3.))
        .bg(c(theme::BORDER))
        .into_any_element()
}

impl ExcelApp {
    /// Run a command from a ribbon control: its drop-down, if it opens
    /// one, hangs from that control.
    fn run_from<C: Command>(&mut self, id: &'static str, cmd: C, window: &mut Window, cx: &mut Cx) {
        if !cmd.enabled(self) {
            return;
        }
        self.command_anchor = Some(id);
        cmd.run(self, window, cx);
        self.command_anchor = None;
    }

    /// A large button running a command. `dropdown` adds the small arrow
    /// of buttons that open a menu.
    pub fn cmd_large<C: Command>(&self, id: &'static str, cmd: C, dropdown: bool, cx: &mut Cx) -> AnyElement {
        let icon_name = if cmd.icon().is_empty() { "more" } else { cmd.icon() };
        self.large_state(id, icon_name, cmd.label_in(self), dropdown, cmd.checked(self), cmd.enabled(self), cx, move |this, window, cx| {
            this.run_from(id, cmd, window, cx)
        })
    }

    /// A drop-down button that is large while the window is wide and takes
    /// one row when the ribbon is short of room.
    fn cmd_adaptive<C: Command>(&self, id: &'static str, cmd: C, cx: &mut Cx) -> AnyElement {
        if self.ribbon_level >= 2 {
            self.cmd_labeled(id, cmd, true, cx)
        } else {
            self.cmd_large(id, cmd, true, cx)
        }
    }

    /// A one-row button with icon and label running a command.
    pub fn cmd_labeled<C: Command>(&self, id: &'static str, cmd: C, dropdown: bool, cx: &mut Cx) -> AnyElement {
        let enabled = cmd.enabled(self);
        let icon_name = if cmd.icon().is_empty() { "more" } else { cmd.icon() };
        fold::saw_icon(icon_name);
        let color = if enabled { theme::TEXT } else { theme::TEXT_DISABLED };
        self.small(
            id,
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(2.))
                .text_color(c(color))
                .child(icon(icon_name, 16.0, color))
                .child(div().whitespace_nowrap().child(cmd.label_in(self)))
                .when(dropdown, |d| d.child(icon("chevron-down", 9.0, theme::TEXT_MUTED))),
            cmd.label_in(self),
            cmd.checked(self),
            cx,
            move |this, window, cx| this.run_from(id, cmd, window, cx),
        )
        .into_any_element()
    }

    /// An icon-only button running a command.
    #[allow(dead_code)] // for the contextual tabs of the feature modules
    pub fn cmd_icon<C: Command>(&self, id: &'static str, cmd: C, cx: &mut Cx) -> AnyElement {
        let color = if cmd.enabled(self) { theme::TEXT } else { theme::TEXT_DISABLED };
        let icon_name = if cmd.icon().is_empty() { "more" } else { cmd.icon() };
        fold::saw_icon(icon_name);
        self.small(id, icon(icon_name, 16.0, color), cmd.label_in(self), cmd.checked(self), cx, move |this, window, cx| {
            this.run_from(id, cmd, window, cx)
        })
        .into_any_element()
    }

    /// A check box showing `cmd.checked` and running the command.
    pub fn cmd_check<C: Command>(&self, id: &'static str, cmd: C, cx: &mut Cx) -> AnyElement {
        self.check(id, cmd.label_in(self), cmd.checked(self), cx, move |this, window, cx| {
            this.run_from(id, cmd, window, cx)
        })
    }

    /// The small arrow at the corner of a group that opens its dialog.
    pub fn launcher(
        &self,
        id: &'static str,
        tip: &'static str,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .size(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(2.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(icon("launcher", 10.0, theme::TEXT_MUTED))
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx)))
            .into_any_element()
    }

    pub fn small(
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

    pub fn icon_btn(
        &self,
        id: &'static str,
        icon_name: &'static str,
        tip: &'static str,
        active: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        fold::saw_icon(icon_name);
        self.small(id, icon(icon_name, 16.0, theme::TEXT), tip, active, cx, f)
            .into_any_element()
    }

    /// A narrow caret that opens a drop-down next to a button.
    pub fn caret_btn(
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

    pub fn labeled(
        &self,
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        dropdown: bool,
        active: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        // A group of small buttons folds under the icon of its first one.
        fold::saw_icon(icon_name);
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

    pub fn large(
        &self,
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        dropdown: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        self.large_state(id, icon_name, label, dropdown, false, true, cx, f)
    }

    /// A large button that can be shown pressed (`active`) or greyed out.
    pub fn large_state(
        &self,
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        dropdown: bool,
        active: bool,
        enabled: bool,
        cx: &mut Cx,
        f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        fold::saw_icon(icon_name);
        let color = if enabled { theme::TEXT } else { theme::TEXT_DISABLED };
        // In a group short of room, a large button is a small one that
        // has lost its label (the tip still names it).
        if fold::compact() {
            let content = div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(icon(icon_name, 16.0, color))
                .when(dropdown, |d| d.child(icon("chevron-down", 9.0, theme::TEXT_MUTED)));
            return self.small(id, content, label, active, cx, f).into_any_element();
        }
        let (line1, line2) = split_label(label);
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
            .text_color(c(color))
            .when(active, |d| d.bg(c(theme::TOGGLED)))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))).active(|s| s.bg(c(theme::PRESSED))))
            .child(icon(icon_name, 26.0, color))
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

    pub fn combo(
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

    pub fn glyph(text: &'static str, bold: bool, italic: bool, underline: bool) -> gpui::Div {
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
    pub fn color_button(&self, id: &'static str, top: impl IntoElement, color: Option<u32>, tip: &'static str, cx: &mut Cx, f: impl Fn(&mut Self, &mut Window, &mut Cx) + 'static) -> AnyElement {
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

    pub fn render_ribbon_tabs(&mut self, window: &Window, cx: &mut Cx) -> impl IntoElement {
        // The permanent tabs, then those of the object that is active.
        let context = features::context_tabs(self);
        if self.context_tab.is_some_and(|id| !context.iter().any(|(_, tab)| tab.id == id)) {
            self.context_tab = None;
        }
        let mut names: Vec<(&'static str, Option<&'static str>)> = tab_names().iter().map(|name| (*name, None)).collect();
        names.extend(context.iter().map(|(_, tab)| (tab.title, Some(tab.id))));
        // Short of room for the names of the tabs, Comments and Share
        // keep their icon only and the tabs stand closer; the two buttons
        // are never cut off.
        let comments_label = t("Comments", "Commentaires");
        let share_label = t("Share", "Partager");
        let text = |label: &str| label.chars().count() as f32 * 7.6;
        let wanted = names.iter().map(|(name, _)| text(name) + 22.0).sum::<f32>()
            + text(comments_label)
            + text(share_label)
            + 120.0;
        let labels = wanted <= f32::from(window.viewport_size().width);
        let pad = if labels { 11.0 } else { 7.0 };
        let mut tabs: Vec<AnyElement> = Vec::new();
        for (i, (name, context_id)) in names.iter().enumerate() {
            let context_id = *context_id;
            let active = match context_id {
                Some(id) => self.context_tab == Some(id),
                None => self.context_tab.is_none() && i == self.ribbon_tab,
            };
            tabs.push(
                div()
                    .id(("ribbon-tab", i))
                    .relative()
                    .h_full()
                    .px(px(pad))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .text_size(px(13.))
                    .whitespace_nowrap()
                    .when(active, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(c(theme::ACCENT_DARK)))
                    .when(!active, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                    .when(context_id.is_some() && !active, |d| d.text_color(c(theme::ACCENT)))
                    .rounded(px(4.))
                    .child(*name)
                    .when(active, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(2.))
                                .left(px(pad))
                                .right(px(pad))
                                .h(px(3.))
                                .rounded(px(2.))
                                .bg(c(theme::ACCENT)),
                        )
                    })
                    .when(i == 0, |d| d.child(self.anchor("file-tab")))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if i == 0 {
                            this.open_file_menu(window, cx);
                        } else if context_id.is_some() {
                            this.context_tab = context_id;
                            cx.notify();
                        } else {
                            this.context_tab = None;
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
            .child(div().h_full().flex_shrink().min_w(px(0.)).overflow_hidden().flex().items_center().children(tabs))
            .child(div().flex_1())
            .child(
                div()
                    .id("btn-comments")
                    .h(px(26.))
                    .px_3()
                    .mr_2()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .bg(c(0xFFFFFF))
                    .child(icon("comment", 14.0, theme::TEXT))
                    .when(labels, |d| d.child(comments_label))
                    .on_click(cx.listener(|this, _, window, cx| comments::Cmd::ShowPane.run(this, window, cx))),
            )
            .child(
                div()
                    .id("btn-share")
                    .h(px(26.))
                    .px_3()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(4.))
                    .bg(c(theme::ACCENT))
                    .text_color(c(0xFFFFFF))
                    .child(icon("share", 14.0, 0xFFFFFF))
                    .when(labels, |d| d.child(share_label))
                    .on_click(cx.listener(|this, _, window, cx| misc::Cmd::Share.run(this, window, cx))),
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
            MenuEntry::command(page::Cmd::Print, self).shortcut("⌘P"),
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

    /// The ribbon: the groups of the tab shown, fitted to the width of the
    /// window by `ribbon_fold.rs`. It scrolls sideways only when the tab
    /// does not fit even with every group folded.
    pub fn render_ribbon(&mut self, window: &mut Window, cx: &mut Cx) -> impl IntoElement {
        // The window less the margins, the border and the padding below.
        let available = f32::from(window.viewport_size().width) - 22.0;
        let fitted = fold::fitted(cx.weak_entity(), self.ribbon_key(), available);
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
                .child(fitted),
        )
    }

    /// What tells the tab shown from the others, in the language shown:
    /// the steps taken to fit one are of no use to another.
    pub(crate) fn ribbon_key(&self) -> String {
        let context = features::context_tabs(self);
        let tab = match self.context_tab.filter(|id| context.iter().any(|(_, tab)| tab.id == *id)) {
            Some(id) => id.to_string(),
            None => self.ribbon_tab.to_string(),
        };
        format!("{tab}/{}", crate::i18n::lang().code())
    }

    /// The groups of the tab shown, at a compaction level of the tabs
    /// that have their own (`ribbon_level`).
    pub(crate) fn ribbon_groups(&mut self, level: u8, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        self.ribbon_level = level;
        let context = features::context_tabs(self);
        let owner = self
            .context_tab
            .and_then(|id| context.iter().find(|(_, tab)| tab.id == id).map(|(owner, _)| (*owner, id)));
        match (owner, self.ribbon_tab) {
            (Some((owner, id)), _) => features::all()[owner].render_tab(self, id, window, cx),
            (None, 2) => self.insert_tab(cx),
            (None, 3) => features::ink::draw_tab(self, cx),
            (None, 4) => self.layout_tab(cx),
            (None, 5) => self.formulas_tab(cx),
            (None, 6) => self.data_tab(cx),
            (None, 7) => self.review_tab(cx),
            (None, 8) => self.view_tab(cx),
            (None, 9) => self.automate_tab(cx),
            (None, 10) => self.help_tab(cx),
            _ => self.home_tab(cx),
        }
    }

    fn automate_tab(&self, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(
                t("Office Scripts", "Scripts Office"),
                row(vec![
                    self.cmd_large("au-new", misc::Cmd::NewScript, false, cx),
                    self.cmd_large("au-all", misc::Cmd::AllScripts, false, cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn help_tab(&self, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(
                t("Help", "Aide"),
                row(vec![
                    self.cmd_large("hp-help", misc::Cmd::Help, false, cx),
                    self.cmd_large("hp-feedback", misc::Cmd::Feedback, false, cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn review_tab(&self, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(
                t("Proofing", "Vérification"),
                row(vec![
                    self.cmd_large("rv-spell", misc::Cmd::Spelling, false, cx),
                    self.cmd_large("rv-thesaurus", misc::Cmd::Thesaurus, false, cx),
                    self.cmd_large("rv-stats", misc::Cmd::WorkbookStatistics, false, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Accessibility", "Accessibilité"),
                row(vec![self.cmd_large("rv-access", misc::Cmd::CheckAccessibility, true, cx)]),
            )
            .into_any_element(),
            group(
                t("Language", "Langue"),
                row(vec![self.cmd_large("rv-translate", misc::Cmd::Translate, false, cx)]),
            )
            .into_any_element(),
            group(
                t("Comments", "Commentaires"),
                row(vec![
                    self.cmd_large("rv-comment", comments::Cmd::NewComment, false, cx),
                    self.cmd_large("rv-comment-del", comments::Cmd::Delete, false, cx),
                    self.cmd_large("rv-comment-prev", comments::Cmd::Previous, false, cx),
                    self.cmd_large("rv-comment-next", comments::Cmd::Next, false, cx),
                    self.cmd_large("rv-comment-show", comments::Cmd::ShowPane, false, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Notes", "Notes"),
                row(vec![self.cmd_large("rv-notes", comments::Cmd::NotesMenu, true, cx)]),
            )
            .into_any_element(),
            group(
                t("Protect", "Protéger"),
                row(vec![
                    self.cmd_large("rv-protect-sheet", protection::Cmd::ProtectSheet, false, cx),
                    self.cmd_large("rv-protect-book", protection::Cmd::ProtectWorkbook, false, cx),
                    self.cmd_large("rv-edit-ranges", protection::Cmd::AllowEditRanges, false, cx),
                    self.cmd_large("rv-unshare", protection::Cmd::UnshareWorkbook, false, cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn home_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let style = self.active_style().clone();
        let merged = {
            let (r, col) = self.active();
            self.wb.sheets[self.sheet].merge_at(r, col).is_some()
        };
        let font_name = theme::font_label(&style.font);
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
                .child(
                    // A split button: the picture pastes, the label and its
                    // arrow open the choices.
                    div()
                        .relative()
                        .child(self.large("rb-paste", "paste", t("Paste", "Coller"), true, cx, |this, _, cx| {
                            this.paste(PasteMode::All, cx)
                        }))
                        .child(
                            div()
                                .id("rb-paste-menu")
                                .absolute()
                                .left_0()
                                .bottom_0()
                                .w_full()
                                .h(px(24.))
                                .occlude()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.run_from("rb-paste", edit_tools::Cmd::PasteMenu, window, cx)
                                })),
                        ),
                )
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
        let font = launcher_group(
            t("Font", "Police"),
            rows(vec![
                row(vec![
                    // Wide enough for "Aptos Narrow (Corps)", the name of the theme's body font.
                    self.combo("rb-font", font_name, 160.0, t("Font", "Police"), cx, |this, window, cx| {
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
                        self.last_fill.map(|col| col.value()),
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
                        self.last_font_color.map(|col| col.value()),
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
            Some(self.launcher("rb-font-launcher", t("Font Settings", "Paramètres de police"), cx, |this, window, cx| {
                format_cells::Cmd::Font.run(this, window, cx)
            })),
        );

        // Alignment
        let alignment = launcher_group(
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
                        |this, window, cx| this.open_orientation_menu(window, cx),
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
                        crate::i18n::t_ctx("button", "Center", "Centrer"),
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
            Some(self.launcher(
                "rb-align-launcher",
                t("Alignment Settings", "Paramètres d'alignement"),
                cx,
                |this, window, cx| format_cells::Cmd::Alignment.run(this, window, cx),
            )),
        );

        // Number
        let number = launcher_group(
            crate::i18n::t_ctx("group", "Number", "Nombre"),
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
            Some(self.launcher("rb-number-launcher", t("Number Format", "Format de nombre"), cx, |this, window, cx| {
                format_cells::Cmd::Number.run(this, window, cx)
            })),
        );

        // Styles
        let style_buttons = vec![
            self.cmd_adaptive("rb-cond", condfmt::Cmd::Menu, cx),
            self.cmd_adaptive("rb-table", tables::Cmd::FormatAs, cx),
            self.cmd_adaptive("rb-cellstyles", edit_tools::Cmd::CellStyles, cx),
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
                                // One refusal, not one for each column.
                                if !a.may(features::Intent::FormatColumns, cx) {
                                    return;
                                }
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
                        MenuEntry::separator(),
                        MenuEntry::command(protection::Cmd::ProtectSheet, this),
                        MenuEntry::command(protection::Cmd::LockCell, this),
                        MenuEntry::command(format_cells::Cmd::Open, this).shortcut("⌘1"),
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
                    self.labeled("rb-fill-series", "fill-down", crate::i18n::t_ctx("command", "Fill", "Remplissage"), true, false, cx, |this, window, cx| {
                        let entries = vec![
                            MenuEntry::new(t("Down", "En bas"), |a, _, cx| a.fill(Direction::Down, cx)).shortcut("⌘D"),
                            MenuEntry::new(crate::i18n::t_ctx("fill", "Right", "À droite"), |a, _, cx| a.fill(Direction::Right, cx)).shortcut("⌘R"),
                            MenuEntry::separator(),
                            MenuEntry::command(data_tools::Cmd::FlashFill, this).shortcut("⌃E"),
                        ];
                        this.open_menu("rb-fill-series", entries, 240.0, window, cx);
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
                            MenuEntry::command(comments::Cmd::Clear, this),
                            MenuEntry::command(edit_tools::Cmd::ClearHyperlinks, this),
                            MenuEntry::command(edit_tools::Cmd::RemoveHyperlinks, this),
                        ];
                        this.open_menu("rb-clear", entries, 260.0, window, cx);
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
                            MenuEntry::command(filters::Cmd::SortAscending, this),
                            MenuEntry::command(filters::Cmd::SortDescending, this),
                            MenuEntry::command(filters::Cmd::CustomSort, this),
                            MenuEntry::separator(),
                            MenuEntry::command(filters::Cmd::Toggle, this).shortcut("⇧⌘L"),
                            MenuEntry::command(filters::Cmd::Clear, this),
                            MenuEntry::command(filters::Cmd::Reapply, this),
                        ];
                        this.open_menu("rb-sort", entries, 220.0, window, cx);
                    },
                ))
                .child(self.large(
                    "rb-find",
                    "search",
                    t("Find & Select", "Rechercher et sélectionner"),
                    true,
                    cx,
                    |this, window, cx| {
                        let entries = vec![
                            MenuEntry::command(edit_tools::Cmd::Find, this).shortcut("⌃F"),
                            MenuEntry::command(edit_tools::Cmd::Replace, this).shortcut("⌃H"),
                            MenuEntry::command(misc::Cmd::GoTo, this),
                            MenuEntry::command(misc::Cmd::GoToSpecial, this),
                            MenuEntry::separator(),
                            MenuEntry::command(edit_tools::Cmd::SelectFormulas, this),
                            MenuEntry::command(edit_tools::Cmd::SelectNotes, this),
                            MenuEntry::command(edit_tools::Cmd::SelectCondFormats, this),
                            MenuEntry::command(edit_tools::Cmd::SelectConstants, this),
                            MenuEntry::command(edit_tools::Cmd::SelectValidation, this),
                            MenuEntry::separator(),
                            MenuEntry::command(edit_tools::Cmd::SelectObjects, this),
                            MenuEntry::command(drawings::Cmd::SelectionPane, this),
                        ];
                        this.open_menu("rb-find", entries, 240.0, window, cx);
                    },
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
        let current = theme::font_label(&self.active_style().font);
        // The two fonts of the theme come first, as in Excel.
        let mut names: Vec<String> = theme::theme_font_labels(self.wb.theme()).to_vec();
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
            MenuEntry::separator(),
            MenuEntry::command(format_cells::Cmd::Border, self),
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
        let mut entries: Vec<MenuEntry> = items
            .into_iter()
            .map(|(label, code)| {
                let checked = code == current;
                MenuEntry::new(label, move |a, _, cx| a.set_number_format(&code, cx)).checked(checked)
            })
            .collect();
        entries.push(MenuEntry::separator());
        entries.push(MenuEntry::new(
            t("More Number Formats…", "Autres formats numériques…"),
            |a, window, cx| format_cells::Cmd::Number.run(a, window, cx),
        ));
        self.open_menu("rb-numfmt", entries, 220.0, window, cx);
    }

    /// The Orientation button of the Alignment group.
    fn open_orientation_menu(&mut self, window: &mut Window, cx: &mut Cx) {
        let current = self.active_style().rotation;
        let angle = |label: &'static str, degrees: i16| {
            // Choosing the current angle again puts the text back level.
            let to = if current == degrees { 0 } else { degrees };
            MenuEntry::new(label, move |a: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
                a.apply_style(t("Orientation", "Orientation"), cx, move |s| s.rotation = to)
            })
            .checked(current == degrees)
        };
        let entries = vec![
            angle(
                t("Angle Counterclockwise", "Rotation dans le sens inverse des aiguilles d'une montre"),
                45,
            ),
            angle(t("Angle Clockwise", "Rotation dans le sens des aiguilles d'une montre"), -45),
            angle(t("Rotate Text Up", "Rotation du texte vers le haut"), 90),
            angle(t("Rotate Text Down", "Rotation du texte vers le bas"), -90),
            MenuEntry::separator(),
            MenuEntry::new(
                t("Format Cell Alignment", "Format de cellule - Alignement"),
                |a, window, cx| format_cells::Cmd::Alignment.run(a, window, cx),
            ),
        ];
        self.open_menu("rb-orient", entries, 360.0, window, cx);
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
            // The function goes into the formula and its arguments open.
            entries.push(MenuEntry::new(locale.fn_to_local(canonical).to_string(), move |a, window, cx| {
                misc::function_arguments(a, canonical, window, cx)
            }));
        }
        entries.sort_by(|a, b| a.label.cmp(&b.label));
        self.open_menu(anchor, entries, 240.0, window, cx);
    }

    fn insert_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(
                t("Tables", "Tableaux"),
                row(vec![
                    self.cmd_large("in-pivot", pivot::Cmd::Insert, true, cx),
                    self.cmd_large("in-pivot-rec", pivot::Cmd::Recommended, false, cx),
                    self.cmd_large("in-table", tables::Cmd::Insert, false, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Illustrations", "Illustrations"),
                row(vec![
                    self.cmd_large("in-pic", drawings::Cmd::Pictures, true, cx),
                    self.cmd_large("in-shapes", drawings::Cmd::Shapes, true, cx),
                    self.cmd_large("in-icons", drawings::Cmd::Icons, false, cx),
                    self.cmd_large("in-smartart", drawings::Cmd::SmartArt, false, cx),
                ]),
            )
            .into_any_element(),
            group(t("Charts", "Graphiques"), charts::insert_group(self, cx)).into_any_element(),
            group(
                t("PivotChart", "Graphique croisé dynamique"),
                row(vec![self.cmd_large("in-pivot-chart", pivot::Cmd::PivotChart, false, cx)]),
            )
            .into_any_element(),
            group(
                t("Sparklines", "Graphiques sparkline"),
                row(vec![self.cmd_large("in-sparklines", features::sparklines::Cmd::Menu, true, cx)]),
            )
            .into_any_element(),
            group(
                t("Filters", "Filtres"),
                row(vec![
                    self.cmd_large("in-slicer", slicers::Cmd::InsertSlicer, false, cx),
                    self.cmd_large("in-timeline", slicers::Cmd::InsertTimeline, false, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Links", "Liens"),
                row(vec![self.cmd_large("in-link", edit_tools::Cmd::Link, false, cx)]),
            )
            .into_any_element(),
            group(
                t("Comments", "Commentaires"),
                row(vec![self.cmd_large("in-comment", comments::Cmd::NewComment, false, cx)]),
            )
            .into_any_element(),
            group(
                t("Text", "Texte"),
                row(vec![
                    self.cmd_large("in-textbox", drawings::Cmd::TextBox, false, cx),
                    self.cmd_large("in-header-footer", page::Cmd::HeaderFooter, false, cx),
                    self.cmd_large("in-wordart", drawings::Cmd::WordArt, true, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Symbols", "Symboles"),
                row(vec![
                    self.cmd_large("in-equation", drawings::Cmd::Equation, false, cx),
                    self.cmd_large("in-symbol", edit_tools::Cmd::Symbol, false, cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    fn layout_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let sheet = &self.wb.sheets[self.sheet];
        let (gridlines, headers) = (sheet.show_gridlines, sheet.show_headers);
        vec![
            group(
                t("Themes", "Thèmes"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(self.cmd_large("pl-themes", themes::Cmd::Themes, true, cx))
                    .child(rows(vec![
                        self.cmd_labeled("pl-theme-colors", themes::Cmd::Colors, true, cx),
                        self.cmd_labeled("pl-theme-fonts", themes::Cmd::Fonts, true, cx),
                        self.cmd_labeled("pl-theme-effects", themes::Cmd::Effects, true, cx),
                    ])),
            )
            .into_any_element(),
            launcher_group(
                t("Page Setup", "Mise en page"),
                row(vec![
                    self.cmd_large("pl-margins", page::Cmd::Margins, true, cx),
                    self.cmd_large("pl-orient", page::Cmd::Orientation, true, cx),
                    self.cmd_large("pl-size", page::Cmd::Size, true, cx),
                    self.cmd_large("pl-area", page::Cmd::PrintArea, true, cx),
                    self.cmd_large("pl-breaks", page::Cmd::Breaks, true, cx),
                    self.cmd_large("pl-background", page::Cmd::Background, false, cx),
                    self.cmd_large("pl-titles", page::Cmd::PrintTitles, false, cx),
                ]),
                Some(self.launcher("pl-setup-launcher", t("Page Setup", "Mise en page"), cx, |this, window, cx| {
                    page::Cmd::Setup.run(this, window, cx)
                })),
            )
            .into_any_element(),
            launcher_group(
                t("Scale to Fit", "Mise à l'échelle"),
                rows(vec![
                    self.cmd_labeled("pl-fit-width", page::Cmd::FitWidth, true, cx),
                    self.cmd_labeled("pl-fit-height", page::Cmd::FitHeight, true, cx),
                    self.cmd_labeled("pl-scale", page::Cmd::Scale, true, cx),
                ]),
                Some(self.launcher("pl-scale-launcher", t("Page Setup", "Mise en page"), cx, |this, window, cx| {
                    page::Cmd::Setup.run(this, window, cx)
                })),
            )
            .into_any_element(),
            launcher_group(
                t("Sheet Options", "Options de la feuille de calcul"),
                div()
                    .flex()
                    .gap(px(10.))
                    .child(rows(vec![
                        div().px(px(4.)).child(t("Gridlines", "Quadrillage")).into_any_element(),
                        self.check("pl-grid-view", crate::i18n::t_ctx("option", "View", "Afficher"), gridlines, cx, |this, _, cx| {
                            this.toggle_gridlines(cx)
                        }),
                        self.cmd_check("pl-grid-print", page::Cmd::PrintGridlines, cx),
                    ]))
                    .child(rows(vec![
                        div().px(px(4.)).child(t("Headings", "En-têtes")).into_any_element(),
                        self.check("pl-head-view", crate::i18n::t_ctx("option", "View", "Afficher"), headers, cx, |this, _, cx| {
                            this.toggle_headings(cx)
                        }),
                        self.cmd_check("pl-head-print", page::Cmd::PrintHeadings, cx),
                    ])),
                Some(self.launcher("pl-sheet-launcher", t("Page Setup", "Mise en page"), cx, |this, window, cx| {
                    page::Cmd::SetupSheet.run(this, window, cx)
                })),
            )
            .into_any_element(),
            self.arrange_group("pl", cx).border_r_0().into_any_element(),
        ]
    }

    /// The Arrange group shared by the Page Layout tab and the tabs of
    /// charts, pictures and shapes. `prefix` keeps the ids of its controls
    /// apart ("pl", "pic", ...).
    pub fn arrange_group(&self, prefix: &'static str, cx: &mut Cx) -> gpui::Div {
        // One set of ids per tab: only one tab is drawn at a time.
        let ids: [&'static str; 6] = match prefix {
            "pl" => ["pl-forward", "pl-backward", "pl-selection", "pl-align", "pl-group", "pl-rotate"],
            _ => ["ar-forward", "ar-backward", "ar-selection", "ar-align", "ar-group", "ar-rotate"],
        };
        group(
            t("Arrange", "Organiser"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(rows(vec![
                    self.cmd_labeled(ids[0], drawings::Cmd::BringForward, false, cx),
                    self.cmd_labeled(ids[1], drawings::Cmd::SendBackward, false, cx),
                    self.cmd_labeled(ids[2], drawings::Cmd::SelectionPane, false, cx),
                ]))
                .child(rows(vec![
                    self.cmd_labeled(ids[3], drawings::Cmd::Align, true, cx),
                    self.cmd_labeled(ids[4], drawings::Cmd::Group, true, cx),
                    self.cmd_labeled(ids[5], drawings::Cmd::Rotate, true, cx),
                ])),
        )
    }

    pub fn toggle_gridlines(&mut self, cx: &mut Cx) {
        self.wb.snapshot();
        let s = &mut self.wb.sheets[self.sheet];
        s.show_gridlines = !s.show_gridlines;
        self.commit(t("Gridlines", "Quadrillage"), cx);
    }

    pub fn toggle_headings(&mut self, cx: &mut Cx) {
        self.wb.snapshot();
        let s = &mut self.wb.sheets[self.sheet];
        s.show_headers = !s.show_headers;
        self.commit(t("Headings", "En-têtes"), cx);
    }

    pub fn check(
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
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    self.cmd_check("vw-ruler", page::Cmd::Ruler, cx),
                    self.check("vw-grid", t("Gridlines", "Quadrillage"), sheet.show_gridlines, cx, |this, _, cx| {
                        this.toggle_gridlines(cx)
                    }),
                ]))
                .child(rows(vec![
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
                    self.check("vw-head", t("Headings", "En-têtes"), sheet.show_headers, cx, |this, _, cx| {
                        this.toggle_headings(cx)
                    }),
                ])),
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
                    self.cmd_large("fx-insert", misc::Cmd::InsertFunction, false, cx),
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
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(self.cmd_large("fx-names", names::Cmd::Manager, false, cx))
                    .child(rows(vec![
                        self.cmd_labeled("fx-define", names::Cmd::DefineMenu, true, cx),
                        self.cmd_labeled("fx-use", names::Cmd::UseInFormula, true, cx),
                        self.cmd_labeled("fx-create", names::Cmd::CreateFromSelection, false, cx),
                    ])),
            )
            .into_any_element(),
            misc::auditing_group(self, cx),
            misc::calculation_group(self, cx).border_r_0().into_any_element(),
        ]
    }

    fn data_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(
                t("Get & Transform Data", "Récupérer et transformer des données"),
                row(vec![
                    self.cmd_large("dt-get", data_tools::Cmd::GetData, true, cx),
                    self.cmd_large("dt-refresh", data_tools::Cmd::RefreshMenu, true, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Sort & Filter", "Trier et filtrer"),
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(rows(vec![
                        self.cmd_icon("dt-asc", filters::Cmd::SortAscending, cx),
                        self.cmd_icon("dt-desc", filters::Cmd::SortDescending, cx),
                    ]))
                    .child(self.cmd_large("dt-sort", filters::Cmd::Sort, false, cx))
                    .child(self.cmd_large("dt-filter", filters::Cmd::Toggle, false, cx))
                    .child(rows(vec![
                        self.cmd_labeled("dt-clear", filters::Cmd::Clear, false, cx),
                        self.cmd_labeled("dt-reapply", filters::Cmd::Reapply, false, cx),
                        self.cmd_labeled("dt-advanced", data_tools::Cmd::AdvancedFilter, false, cx),
                    ])),
            )
            .into_any_element(),
            group(
                t("Data Tools", "Outils de données"),
                row(vec![
                    self.cmd_large("dt-ttc", data_tools::Cmd::TextToColumns, false, cx),
                    self.cmd_large("dt-flash", data_tools::Cmd::FlashFill, false, cx),
                    self.cmd_large("dt-dup", filters::Cmd::RemoveDuplicates, false, cx),
                    self.cmd_large("dt-valid", validation::Cmd::Menu, true, cx),
                    self.cmd_large("dt-consolidate", data_tools::Cmd::Consolidate, false, cx),
                ]),
            )
            .into_any_element(),
            group(
                t("Forecast", "Prévision"),
                row(vec![self.cmd_large("dt-whatif", data_tools::Cmd::WhatIf, true, cx)]),
            )
            .into_any_element(),
            features::outline::ribbon_group(self, cx).border_r_0().into_any_element(),
        ]
    }

    fn view_tab(&mut self, cx: &mut Cx) -> Vec<AnyElement> {
        let frozen = {
            let s = &self.wb.sheets[self.sheet];
            s.frozen_rows > 0 || s.frozen_cols > 0
        };
        let mode = self.view_mode();
        vec![
            group(
                t("Workbook Views", "Modes d'affichage"),
                row(vec![
                    self.large_state(
                        "vw-normal",
                        "view-normal",
                        t("Normal", "Normal"),
                        false,
                        mode == ViewMode::Normal,
                        true,
                        cx,
                        |this, _, cx| this.set_view_mode(ViewMode::Normal, cx),
                    ),
                    self.large_state(
                        "vw-break",
                        "view-break",
                        t("Page Break Preview", "Aperçu des sauts de page"),
                        false,
                        mode == ViewMode::PageBreakPreview,
                        true,
                        cx,
                        |this, _, cx| this.set_view_mode(ViewMode::PageBreakPreview, cx),
                    ),
                    self.large_state(
                        "vw-layout",
                        "view-layout",
                        crate::i18n::t_ctx("view", "Page Layout", "Mise en page"),
                        false,
                        mode == ViewMode::PageLayout,
                        true,
                        cx,
                        |this, _, cx| this.set_view_mode(ViewMode::PageLayout, cx),
                    ),
                    self.cmd_large("vw-custom", features::custom_views::Cmd::Open, false, cx),
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
                    t("Freeze Panes", "Figer les volets"),
                    true,
                    cx,
                    move |this, window, cx| {
                        let freeze = |label: &'static str, rows: u32, cols: u32| {
                            MenuEntry::new(label, move |a: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
                                a.wb.set_frozen(a.sheet, rows, cols);
                                let view = &mut a.views[a.sheet];
                                view.scroll_x = 0.0;
                                view.scroll_y = 0.0;
                                a.commit(t("Freeze Panes", "Figer les volets"), cx);
                            })
                        };
                        let entries = vec![
                            MenuEntry::new(
                                if frozen {
                                    t("Unfreeze Panes", "Libérer les volets")
                                } else {
                                    t("Freeze Panes", "Figer les volets")
                                },
                                |a, _, cx| a.toggle_freeze(cx),
                            )
                            .icon("freeze"),
                            freeze(t("Freeze Top Row", "Figer la ligne supérieure"), 1, 0),
                            freeze(t("Freeze First Column", "Figer la première colonne"), 0, 1),
                        ];
                        this.open_menu("vw-freeze", entries, 230.0, window, cx);
                    },
                )]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }
}

/// Name of a chart type as shown in menus.
pub fn chart_kind_name(kind: ChartKind) -> &'static str {
    match kind {
        ChartKind::Column => t("Clustered Column", "Histogramme groupé"),
        ChartKind::StackedColumn => t("Stacked Column", "Histogramme empilé"),
        ChartKind::PercentStackedColumn => t("100% Stacked Column", "Histogramme empilé 100 %"),
        ChartKind::Column3D => t("3-D Clustered Column", "Histogramme 3D groupé"),
        ChartKind::StackedColumn3D => t("3-D Stacked Column", "Histogramme 3D empilé"),
        ChartKind::PercentStackedColumn3D => t("3-D 100% Stacked Column", "Histogramme 3D empilé 100 %"),
        ChartKind::Line => crate::i18n::t_ctx("chart", "Line", "Courbes"),
        ChartKind::StackedLine => t("Stacked Line", "Courbes empilées"),
        ChartKind::PercentStackedLine => t("100% Stacked Line", "Courbes empilées 100 %"),
        ChartKind::LineMarkers => t("Line with Markers", "Courbes avec marques"),
        ChartKind::StackedLineMarkers => t("Stacked Line with Markers", "Courbes empilées avec marques"),
        ChartKind::PercentStackedLineMarkers => {
            t("100% Stacked Line with Markers", "Courbes empilées 100 % avec marques")
        }
        ChartKind::Line3D => t("3-D Line", "Courbes 3D"),
        ChartKind::Pie => t("Pie", "Secteurs"),
        ChartKind::Pie3D => t("3-D Pie", "Secteurs en 3D"),
        ChartKind::Doughnut => t("Doughnut", "Anneau"),
        ChartKind::Bar => t("Clustered Bar", "Barres groupées"),
        ChartKind::StackedBar => t("Stacked Bar", "Barres empilées"),
        ChartKind::PercentStackedBar => t("100% Stacked Bar", "Barres empilées 100 %"),
        ChartKind::Bar3D => t("3-D Clustered Bar", "Barres 3D groupées"),
        ChartKind::StackedBar3D => t("3-D Stacked Bar", "Barres 3D empilées"),
        ChartKind::PercentStackedBar3D => t("3-D 100% Stacked Bar", "Barres 3D empilées 100 %"),
        ChartKind::Area => t("Area", "Aires"),
        ChartKind::StackedArea => t("Stacked Area", "Aires empilées"),
        ChartKind::PercentStackedArea => t("100% Stacked Area", "Aires empilées 100 %"),
        ChartKind::Area3D => t("3-D Area", "Aires 3D"),
        ChartKind::StackedArea3D => t("3-D Stacked Area", "Aires 3D empilées"),
        ChartKind::PercentStackedArea3D => t("3-D 100% Stacked Area", "Aires 3D empilées 100 %"),
        ChartKind::Scatter => t("Scatter", "Nuage de points"),
        ChartKind::ScatterSmoothMarkers => {
            t("Scatter with Smooth Lines and Markers", "Nuage de points avec courbes lissées et marques")
        }
        ChartKind::ScatterSmooth => t("Scatter with Smooth Lines", "Nuage de points avec courbes lissées"),
        ChartKind::ScatterLinesMarkers => {
            t("Scatter with Straight Lines and Markers", "Nuage de points avec courbes droites et marques")
        }
        ChartKind::ScatterLines => t("Scatter with Straight Lines", "Nuage de points avec courbes droites"),
        ChartKind::Bubble => t("Bubble", "Bulles"),
        ChartKind::Radar => t("Radar", "Radar"),
        ChartKind::RadarMarkers => t("Radar with Markers", "Radar avec marques"),
        ChartKind::RadarFilled => t("Filled Radar", "Radar plein"),
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
