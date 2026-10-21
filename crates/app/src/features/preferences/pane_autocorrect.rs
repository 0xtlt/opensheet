//! The AutoCorrect pane of Preferences, as Excel for Mac lays it out: the
//! four boxes, "Replace text as you type", then "Replace:" and "With:"
//! over the list of replacements, and Add (which reads Replace when the
//! word is already listed) and Delete under it.

use super::autocorrect;
use super::window::PreferencesWindow;
use crate::i18n::t;
use crate::widgets::{Button, ListBox, ROW_H, TextField, label};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

const FROM_W: f32 = 150.0;
const WITH_W: f32 = 290.0;

impl PreferencesWindow {
    /// The list as the pane shows it.
    pub(super) fn replacements(&self) -> Vec<(String, String)> {
        autocorrect::replacements(&self.shown.settings)
    }

    /// Where the word typed in "Replace:" stands in the list.
    fn replacement_at(&self, cx: &Context<Self>) -> Option<usize> {
        let from = self.replace_from.read(cx).text().trim().to_string();
        self.replacements()
            .iter()
            .position(|(known, _)| *known == from)
    }

    /// Add, or Replace: list the word typed with what replaces it. The
    /// list stays in alphabetical order, as Excel shows it.
    pub(super) fn add_replacement(&mut self, cx: &mut Context<Self>) {
        let from = self.replace_from.read(cx).text().trim().to_string();
        let to = self.replace_with.read(cx).text().trim().to_string();
        if from.is_empty() || to.is_empty() || from == to {
            return;
        }
        let mut list = self.replacements();
        list.retain(|(known, _)| *known != from);
        list.push((from, to));
        list.sort_by_key(|(from, _)| from.to_lowercase());
        self.set(cx, |settings| settings.replacements = Some(list.clone()));
    }

    /// Delete: the word typed in "Replace:" leaves the list.
    pub(super) fn delete_replacement(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self.replacement_at(cx) else {
            return;
        };
        let mut list = self.replacements();
        list.remove(at);
        self.set(cx, |settings| settings.replacements = Some(list.clone()));
        for field in [&self.replace_from, &self.replace_with] {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
    }

    /// A line of the list was clicked: the two boxes show it.
    pub(super) fn show_replacement(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some((from, to)) = self.replacements().into_iter().nth(at) else {
            return;
        };
        self.replace_from
            .update(cx, |field, cx| field.set_text(from, cx));
        self.replace_with
            .update(cx, |field, cx| field.set_text(to, cx));
        cx.notify();
    }

    pub(super) fn render_autocorrect(&self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.shown.settings;
        let list = self.replacements();
        let at = self.replacement_at(cx);
        let from = self.replace_from.read(cx).text().trim().to_string();
        let to = self.replace_with.read(cx).text().trim().to_string();
        let same = at.is_some_and(|at| list[at].1 == to);
        let can_add = s.replace_text && !from.is_empty() && !to.is_empty() && from != to && !same;
        let add_label = if at.is_some() {
            t("Replace", "Remplacer")
        } else {
            t("Add", "Ajouter")
        };
        let names: Vec<SharedString> = list
            .iter()
            .map(|(from, _)| SharedString::from(from.clone()))
            .collect();
        let rows = list.clone();
        let boxes = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(self.check(
                "prefs-ac-links",
                t(
                    "Replace internet and network paths with hyperlinks",
                    "Remplacer les chemins d’accès Internet et réseau par des liens hypertexte",
                ),
                s.link_addresses,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.link_addresses = on),
            ))
            .child(self.check(
                "prefs-ac-capitals",
                t(
                    "Correct TWo INitial CApitals",
                    "Supprimer la 2e majuscule d’un mot",
                ),
                s.correct_two_capitals,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.correct_two_capitals = on),
            ))
            .child(self.check(
                "prefs-ac-sentences",
                t(
                    "Capitalize first letter of sentences",
                    "Majuscule en début de phrase",
                ),
                s.capitalize_sentences,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.capitalize_sentences = on),
            ))
            .child(self.check(
                "prefs-ac-days",
                t(
                    "Capitalize names of days",
                    "Majuscules aux jours de la semaine",
                ),
                s.capitalize_days,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.capitalize_days = on),
            ))
            .child(self.check(
                "prefs-ac-replace",
                t("Replace text as you type", "Correction en cours de frappe"),
                s.replace_text,
                cx,
                |this, on, _, cx| this.set(cx, |settings| settings.replace_text = on),
            ));
        let column = |title: &'static str, width: f32, field: TextField| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .w(px(width))
                .child(label(title))
                .child(field.width(width))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(boxes)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .pl(px(20.))
                    .child(column(
                        t("Replace:", "Remplacer\u{a0}:"),
                        FROM_W,
                        TextField::new(&self.replace_from),
                    ))
                    .child(column(
                        t("With:", "Par\u{a0}:"),
                        WITH_W,
                        TextField::new(&self.replace_with),
                    )),
            )
            .child(
                div().pl(px(20.)).child(
                    ListBox::new("prefs-ac-list", names, at)
                        .width(FROM_W + 8.0 + WITH_W)
                        .height(ROW_H * 7.0)
                        .row(move |index, _| {
                            let (from, to) = &rows[index];
                            div()
                                .flex()
                                .child(
                                    div()
                                        .w(px(FROM_W))
                                        .flex_none()
                                        .truncate()
                                        .child(from.clone()),
                                )
                                .child(div().flex_1().truncate().child(to.clone()))
                                .into_any_element()
                        })
                        .on_change(
                            cx.listener(|this, at: &usize, _, cx| this.show_replacement(*at, cx)),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("prefs-ac-add", add_label)
                            .disabled(!can_add)
                            .on_click(cx.listener(|this, _, _, cx| this.add_replacement(cx))),
                    )
                    .child(
                        Button::new("prefs-ac-delete", t("Delete", "Supprimer"))
                            .disabled(at.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.delete_replacement(cx))),
                    ),
            )
            .into_any_element()
    }
}
