//! Data > Remove Duplicates: the dialog (which columns are compared, "My
//! data has headers") and the message that tells what was removed.

use super::sort::{Asking, ask_to_expand};
use super::{APP_NAME, Cmd, block_around, cannot_apply, column_title, single_selection};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ROW_H, ScrollState, Tri, check_mark, dialog, virtual_rows};
use gpui::{App, Context, Entity, FocusHandle, FontWeight, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use opensheet_engine::ops::{DuplicatesRemoved, SortError};
use opensheet_engine::refs::col_to_letters;
use opensheet_engine::{Rect, Workbook};

/// What Remove Duplicates does with the selection it finds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Plan {
    /// A lone cell with nothing around it.
    Nothing,
    /// The rows to look at. `fixed_header`: they are those of a table,
    /// whose header row is not a matter of choice.
    Rows {
        rect: Rect,
        has_header: bool,
        fixed_header: bool,
    },
    /// The selection has data beside it: ask whether it follows.
    Ask { selection: Rect, expanded: Rect },
}

pub(super) fn plan(wb: &Workbook, sheet: usize, selection: Rect, active: (u32, u32)) -> Plan {
    if let Some(table) = wb.table_at(sheet, active.0, active.1) {
        let range = table.range;
        return Plan::Rows {
            rect: Rect::new(range.r0, range.c0, range.r1 - table.totals_row as u32, range.c1),
            has_header: table.header_row,
            fixed_header: true,
        };
    }
    let rect = if selection.is_single() {
        let region = wb.current_region(sheet, selection.r0, selection.c0);
        if region.is_single() && wb.sheets[sheet].is_blank(region.r0, region.c0) {
            return Plan::Nothing;
        }
        region
    } else {
        let selection = wb.clamp_to_used(sheet, selection);
        let expanded = block_around(wb, sheet, selection);
        if expanded != selection {
            return Plan::Ask { selection, expanded };
        }
        selection
    };
    Plan::Rows {
        rect,
        has_header: wb.guess_header_row(sheet, rect),
        fixed_header: false,
    }
}

/// The columns as the dialog lists them: their headers, or "Column B".
pub(super) fn column_names(wb: &Workbook, sheet: usize, rect: Rect, has_header: bool) -> Vec<String> {
    (rect.c0..=rect.c1)
        .map(|col| {
            if has_header {
                column_title(&wb.display_text(sheet, rect.r0, col).text, col)
            } else {
                t("Column {}", "Colonne {}").replace("{}", &col_to_letters(col))
            }
        })
        .collect()
}

/// The message shown once the duplicates are gone.
pub(super) fn outcome_message(outcome: DuplicatesRemoved) -> String {
    if outcome.removed == 0 {
        return t("No duplicate values found.", "Aucune valeur en double trouvée.").to_string();
    }
    t(
        "{0} duplicate values found and removed; {1} unique values remain. Note that counts may include empty cells, spaces, etc.",
        "{0} valeurs dupliquées trouvées et supprimées ; {1} valeurs uniques sont conservées. Notez que les nombres peuvent inclure des cellules vides, des espaces, etc.",
    )
    .replace("{0}", &outcome.removed.to_string())
    .replace("{1}", &outcome.remaining.to_string())
}

/// Open the dialog on what the selection stands for.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(selection) = single_selection(app, cx) else {
        return;
    };
    match plan(&app.wb, app.sheet, selection, app.active()) {
        Plan::Nothing => cannot_apply(app, cx),
        Plan::Rows {
            rect,
            has_header,
            fixed_header,
        } => open_on(app, rect, has_header, fixed_header, window, cx),
        Plan::Ask { selection, expanded } => ask_to_expand(
            app,
            Asking::Duplicates,
            selection,
            expanded,
            |app, rect, window, cx| {
                let has_header = app.wb.guess_header_row(app.sheet, rect);
                open_on(app, rect, has_header, false, window, cx);
            },
            window,
            cx,
        ),
    }
}

fn open_on(app: &mut ExcelApp, rect: Rect, has_header: bool, fixed_header: bool, window: &mut Window, cx: &mut Cx) {
    // A protected sheet never allows it, as in Excel.
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let sheet = app.sheet;
    let names = names_of(&app.wb, sheet, rect, has_header);
    app.open_modal(window, cx, move |modal, _, cx| DuplicatesDialog {
        modal,
        sheet,
        rect,
        has_header,
        fixed_header,
        checked: vec![true; names.len()],
        names,
        scroll: cx.new(|_| ScrollState::new()),
        cursor: 0,
        focus: cx.focus_handle().tab_stop(true),
    });
}

fn names_of(wb: &Workbook, sheet: usize, rect: Rect, has_header: bool) -> Vec<SharedString> {
    column_names(wb, sheet, rect, has_header).into_iter().map(SharedString::from).collect()
}

/// What OK does once the dialog is closed. `columns`: offsets of the
/// compared columns in `rect`.
fn run(app: &mut ExcelApp, sheet: usize, rect: Rect, columns: Vec<u32>, has_header: bool, cx: &mut Cx) {
    if app.sheet != sheet || !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    match app.wb.remove_duplicates(sheet, rect, &columns, has_header) {
        Ok(outcome) => {
            if outcome.removed > 0 {
                app.commit(Cmd::RemoveDuplicates.label(), cx);
            }
            app.alert(Alert::new(APP_NAME, outcome_message(outcome)), cx);
        }
        Err(SortError::MergedCells) => {
            app.wb.rollback();
            let message = t(
                "You can't use this command on a table or range that contains merged cells.",
                "Vous ne pouvez pas utiliser cette commande sur un tableau ou une plage qui contient des cellules fusionnées.",
            );
            app.alert(Alert::new(APP_NAME, message).warning(), cx);
        }
    }
}

const LIST_ROWS: f32 = 8.0;
const FIELD_BORDER: u32 = 0xABABAB;

struct DuplicatesDialog {
    modal: Modal,
    sheet: usize,
    rect: Rect,
    has_header: bool,
    fixed_header: bool,
    /// One per column of `rect`.
    checked: Vec<bool>,
    names: Vec<SharedString>,
    scroll: Entity<ScrollState>,
    /// Line of the list the arrow keys are on.
    cursor: usize,
    focus: FocusHandle,
}

impl DuplicatesDialog {
    fn set_all(&mut self, checked: bool, cx: &mut Context<Self>) {
        self.checked.fill(checked);
        cx.notify();
    }

    fn set_header(&mut self, has_header: bool, cx: &mut Context<Self>) {
        self.has_header = has_header;
        let (sheet, rect) = (self.sheet, self.rect);
        if let Some(names) = self.modal.read_app(cx, |app| names_of(&app.wb, sheet, rect, has_header)) {
            self.names = names;
        }
        cx.notify();
    }

    fn list_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !widgets::plain(event) || self.checked.is_empty() {
            return;
        }
        let last = self.checked.len() - 1;
        let page = LIST_ROWS as usize - 1;
        let next = match event.keystroke.key.as_str() {
            "up" => self.cursor.saturating_sub(1),
            "down" => (self.cursor + 1).min(last),
            "pageup" => self.cursor.saturating_sub(page),
            "pagedown" => (self.cursor + page).min(last),
            "home" => 0,
            "end" => last,
            "space" => {
                self.checked[self.cursor] ^= true;
                self.cursor
            }
            _ => return,
        };
        self.cursor = next;
        self.scroll
            .update(cx, |scroll, _| scroll.reveal(next, next, ROW_H, LIST_ROWS * ROW_H));
        cx.stop_propagation();
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let columns: Vec<u32> = (0..self.checked.len() as u32).filter(|i| self.checked[*i as usize]).collect();
        if columns.is_empty() {
            return;
        }
        let (sheet, rect, has_header) = (self.sheet, self.rect, self.has_header);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| run(app, sheet, rect, columns, has_header, cx));
    }
}

impl Render for DuplicatesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        self.cursor = self.cursor.min(self.checked.len().saturating_sub(1));
        let cursor = self.cursor;
        let view = cx.weak_entity();
        let (names, checked) = (&self.names, &self.checked);
        let rows = virtual_rows(&self.scroll, checked.len(), ROW_H, LIST_ROWS * ROW_H, cx, |i| {
            let view = view.clone();
            div()
                .id(("column", i))
                .size_full()
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when(focused && i == cursor, |d| d.bg(c(widgets::SELECTED_BG)))
                .when(!(focused && i == cursor), |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .child(check_mark(Tri::from(checked[i]), false))
                .child(div().truncate().child(names.get(i).cloned().unwrap_or_default()))
                .on_click(move |_, _, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.checked[i] ^= true;
                        this.cursor = i;
                        cx.notify();
                    })
                    .ok();
                })
                .into_any_element()
        });
        let nothing = !self.checked.contains(&true);
        let modal = self.modal.clone();
        dialog(&self.modal, Cmd::RemoveDuplicates.label())
            .width(460.0)
            .on_ok(cx.listener(Self::ok))
            .child(div().line_height(px(18.)).child(t(
                "To delete duplicate values, select one or more columns that contain duplicates.",
                "Pour supprimer les valeurs en double, sélectionnez une ou plusieurs colonnes contenant des doublons.",
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("duplicates-all", t("Select All", "Sélectionner tout"))
                            .on_click(cx.listener(|this, _, _, cx| this.set_all(true, cx))),
                    )
                    .child(
                        Button::new("duplicates-none", t("Unselect All", "Désélectionner tout"))
                            .on_click(cx.listener(|this, _, _, cx| this.set_all(false, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Checkbox::new(
                            "duplicates-headers",
                            t("My data has headers", "Mes données ont des en-têtes"),
                            self.has_header,
                        )
                        .disabled(self.fixed_header)
                        .on_change(cx.listener(|this, on: &bool, _, cx| this.set_header(*on, cx))),
                    ),
            )
            .child(
                div()
                    .id("duplicates-columns")
                    .track_focus(&self.focus)
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
                    .rounded(px(3.))
                    .overflow_hidden()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.list_key(event, cx)))
                    .child(
                        div()
                            .h(px(22.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .bg(c(theme::CHROME_BG))
                            .border_b_1()
                            .border_color(c(theme::BORDER))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(theme::TEXT_MUTED))
                            .child(t("Columns", "Colonnes")),
                    )
                    .child(rows),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("dialog-ok", t("OK", "OK"))
                            .primary()
                            .disabled(nothing)
                            .on_click(cx.listener(Self::ok)),
                    )
                    .child(
                        Button::new("dialog-cancel", t("Cancel", "Annuler"))
                            .on_click(move |_, window, cx| modal.close(window, cx)),
                    ),
            )
    }
}
