//! The Custom Lists pane of Preferences, as Excel for Mac lays it out:
//! "Custom lists:" on the left (NEW LIST, the four built-in lists, then
//! the user's), "List entries:" on the right with Add and Delete, and
//! "Import list from cells:" under them.
//!
//! The lists are kept in the settings file and handed to the workbook
//! (`apply_to_workbook`): the fill handle continues them and the Sort
//! dialog offers them, both from `Workbook::custom_lists`.

use super::settings::clean_list;
use super::window::{PreferencesWindow, note};
use crate::i18n::t;
use crate::shell::Alert;
use crate::theme::{self, c};
use crate::widgets::{Button, ListBox, ROW_H, RefEdit, label};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use opensheet_engine::{Rect, Value, Workbook};

/// A list with more characters than this is cut, as Excel cuts it.
const MAX_CHARS: usize = 255;

/// What "Add" makes of the box of entries: one entry per line (commas
/// separate entries too, as in Excel).
pub(super) fn entries_of(text: &str) -> Vec<String> {
    let mut list = clean_list(text.split(['\n', ',']));
    // The whole list, separators included, fits in 255 characters.
    let mut used = 0;
    list.retain(|entry| {
        used += entry.chars().count() + 1;
        used <= MAX_CHARS + 1
    });
    list
}

/// The lists "Import" reads from a range of a sheet, and whether cells
/// were skipped because they hold something other than text. A row or a
/// column gives one list; a block gives one list per column.
pub(super) fn lists_in(wb: &Workbook, sheet: usize, range: Rect) -> (Vec<Vec<String>>, bool) {
    let range = wb.clamp_to_used(sheet, range);
    let mut skipped = false;
    let mut text_at = |row: u32, col: u32| match wb.value_at(sheet, row, col) {
        Value::Text(text) => Some(text.to_string()),
        Value::Empty => None,
        _ => {
            skipped = true;
            None
        }
    };
    let mut lists = Vec::new();
    if range.r0 == range.r1 {
        lists.push(
            (range.c0..=range.c1)
                .filter_map(|col| text_at(range.r0, col))
                .collect::<Vec<_>>(),
        );
    } else {
        for col in range.c0..=range.c1 {
            lists.push(
                (range.r0..=range.r1)
                    .filter_map(|row| text_at(row, col))
                    .collect(),
            );
        }
    }
    let lists = lists
        .into_iter()
        .map(|list| entries_of(&list.join("\n")))
        .filter(|list| !list.is_empty())
        .collect();
    (lists, skipped)
}

/// The range typed or picked in "Import list from cells:", on the sheet
/// in front (`$A$1:$A$5`, with or without the name of that sheet).
fn range_of(text: &str, sheet_name: &str) -> Option<Rect> {
    let text = text.trim().trim_start_matches('=');
    let cells = match text.rsplit_once('!') {
        Some((sheet, cells)) => {
            let sheet = sheet.trim().trim_matches('\'').replace("''", "'");
            if sheet.to_lowercase() != sheet_name.to_lowercase() {
                return None;
            }
            cells
        }
        None => text,
    };
    Rect::parse(&cells.replace('$', ""))
}

impl PreferencesWindow {
    fn builtin_count(&self) -> usize {
        self.shown.builtin_lists.len()
    }

    /// The list chosen on the left, when it is one of the user's: its
    /// place among them.
    pub(super) fn own_list_at(&self) -> Option<usize> {
        self.list_at
            .checked_sub(1 + self.builtin_count())
            .filter(|at| *at < self.shown.settings.custom_lists.len())
    }

    /// A line of "Custom lists:" was clicked: its entries show on the right.
    pub(super) fn choose_list(&mut self, at: usize, cx: &mut Context<Self>) {
        self.list_at = at;
        let builtin = self.builtin_count();
        let entries = match at.checked_sub(1) {
            None => Vec::new(),
            Some(index) if index < builtin => self.shown.builtin_lists[index].clone(),
            Some(index) => self
                .shown
                .settings
                .custom_lists
                .get(index - builtin)
                .cloned()
                .unwrap_or_default(),
        };
        self.list_entries
            .update(cx, |area, cx| area.set_text(entries.join("\n"), cx));
        cx.notify();
    }

    fn store_lists(&mut self, lists: Vec<Vec<String>>, cx: &mut Context<Self>) {
        self.set(cx, |settings| settings.custom_lists = lists.clone());
    }

    /// Add: the entries typed become a new list, or replace the list of
    /// the user that is chosen. False when there is nothing to add.
    pub(super) fn add_list(&mut self, cx: &mut Context<Self>) -> bool {
        let entries = entries_of(self.list_entries.read(cx).text());
        if entries.is_empty() {
            return false;
        }
        if entries
            .iter()
            .any(|entry| entry.starts_with(|ch: char| ch.is_ascii_digit()))
        {
            self.modal.update_app(cx, |app, cx| {
                let message = t(
                    "The first character of a list entry cannot be a number.",
                    "Le premier caractère d'une entrée de liste ne peut pas être un chiffre.",
                );
                app.alert(Alert::new("OpenSheet", message).warning(), cx)
            });
            return false;
        }
        let mut lists = self.shown.settings.custom_lists.clone();
        let builtin = self.builtin_count();
        let known = |lists: &[Vec<String>]| {
            self.shown
                .builtin_lists
                .iter()
                .chain(lists)
                .position(|list| *list == entries)
        };
        let at = match (self.own_list_at(), known(&lists)) {
            // The same list twice is one list.
            (_, Some(known)) => known,
            (Some(own), None) => {
                lists[own] = entries;
                builtin + own
            }
            (None, None) => {
                lists.push(entries);
                builtin + lists.len() - 1
            }
        };
        self.store_lists(lists, cx);
        self.choose_list(1 + at, cx);
        true
    }

    /// Delete, once the user agreed: the chosen list of the user goes.
    pub(super) fn delete_list(&mut self, cx: &mut Context<Self>) {
        let Some(own) = self.own_list_at() else {
            return;
        };
        let mut lists = self.shown.settings.custom_lists.clone();
        lists.remove(own);
        self.store_lists(lists, cx);
        self.choose_list(0, cx);
    }

    /// Delete was pressed: Excel warns first.
    fn ask_delete_list(&mut self, cx: &mut Context<Self>) {
        if self.own_list_at().is_none() {
            return;
        }
        let this = cx.entity().downgrade();
        self.modal.update_app(cx, |app, cx| {
            let message = t(
                "List will be permanently deleted.",
                "La liste sera supprimée définitivement.",
            );
            app.alert(
                Alert::new("OpenSheet", message)
                    .warning()
                    .button(t("OK", "OK"), move |_, _, cx| {
                        this.update(cx, |this, cx| this.delete_list(cx)).ok();
                    })
                    .close_button(t("Cancel", "Annuler")),
                cx,
            )
        });
    }

    /// Import: the texts of the range typed or picked become lists.
    pub(super) fn import_lists(&mut self, cx: &mut Context<Self>) {
        let (sheet, name) = (self.shown.sheet, self.shown.sheet_name.clone());
        let range = range_of(self.import_range.read(cx).text(), &name);
        let Some(range) = range else {
            self.modal.update_app(cx, |app, cx| {
                let message = t("Reference isn't valid.", "La référence n’est pas valide.");
                app.alert(Alert::new("OpenSheet", message).warning(), cx)
            });
            return;
        };
        let Some((found, skipped)) = self
            .modal
            .read_app(cx, |app| lists_in(&app.wb, sheet, range))
        else {
            return;
        };
        let mut lists = self.shown.settings.custom_lists.clone();
        let mut last = None;
        for list in found {
            let known = self
                .shown
                .builtin_lists
                .iter()
                .chain(&lists)
                .position(|known| *known == list);
            last = Some(known.unwrap_or_else(|| {
                lists.push(list);
                self.builtin_count() + lists.len() - 1
            }));
        }
        self.store_lists(lists, cx);
        if let Some(at) = last {
            self.choose_list(1 + at, cx);
        }
        if skipped {
            self.modal.update_app(cx, |app, cx| {
                let message = t(
                    "Cells without simple text were ignored.",
                    "Les cellules ne contenant pas de texte simple ont été ignorées.",
                );
                app.alert(Alert::new("OpenSheet", message), cx)
            });
        }
    }

    pub(super) fn render_lists(&self, cx: &mut Context<Self>) -> AnyElement {
        let name = |list: &Vec<String>| SharedString::from(list.join(", "));
        let mut names: Vec<SharedString> = vec![t("NEW LIST", "Nouvelle liste").into()];
        names.extend(self.shown.builtin_lists.iter().map(name));
        names.extend(self.shown.settings.custom_lists.iter().map(name));
        let own = self.own_list_at().is_some();
        // The built-in lists show their entries but cannot be changed.
        let builtin = !own && self.list_at != 0;
        let empty = entries_of(self.list_entries.read(cx).text()).is_empty();
        let column = |title: &'static str| div().flex().flex_col().gap(px(4.)).child(label(title));
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        column(t("Custom lists:", "Listes personnalisées\u{a0}:")).child(
                            ListBox::new("prefs-lists", names, Some(self.list_at))
                                .width(230.0)
                                .height(ROW_H * 10.0)
                                .on_change(
                                    cx.listener(|this, at: &usize, _, cx| {
                                        this.choose_list(*at, cx)
                                    }),
                                ),
                        ),
                    )
                    .child(
                        column(t("List entries:", "Entrées de la liste\u{a0}:")).child(
                            div()
                                .w(px(190.))
                                .h(px(ROW_H * 10.0))
                                .border_1()
                                .border_color(c(theme::BORDER_STRONG))
                                .bg(c(theme::PANEL_BG))
                                .overflow_hidden()
                                .child(self.list_entries.clone()),
                        ),
                    )
                    .child(
                        div()
                            .pt(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                Button::new("prefs-lists-add", t("Add", "Ajouter"))
                                    .width(96.0)
                                    .disabled(builtin || empty)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.add_list(cx);
                                    })),
                            )
                            .child(
                                Button::new("prefs-lists-delete", t("Delete", "Supprimer"))
                                    .width(96.0)
                                    .disabled(!own)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.ask_delete_list(cx)),
                                    ),
                            ),
                    ),
            )
            .child(note(t(
                "Press Return to separate list entries.",
                "Appuyez sur Retour pour séparer les entrées de la liste.",
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t(
                        "Import list from cells:",
                        "Importer la liste à partir des cellules :",
                    )))
                    .child(RefEdit::new(&self.import_range, &self.modal).width(170.0))
                    .child(
                        Button::new("prefs-lists-import", t("Import", "Importer"))
                            .on_click(cx.listener(|this, _, _, cx| this.import_lists(cx))),
                    ),
            )
            .into_any_element()
    }
}
