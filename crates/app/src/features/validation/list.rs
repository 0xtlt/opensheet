//! The drop-down list of a cell: the entries its rule allows, or, for a
//! cell without a list ("Pick From Drop-down List"), the texts already
//! typed in its column.

use super::{ARROW_SIZE, active_rect};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::theme::{self, c};
use crate::widgets::{self, ROW_H, ScrollState};
use gpui::{Context, Entity, FocusHandle, KeyDownEvent, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::validation::ListItem;
use opensheet_engine::{MAX_ROWS, Rect, Value, Workbook};

/// Lines shown at a time, as in Excel.
const VISIBLE: usize = 8;

/// The texts typed in the block of filled cells around a cell of a column,
/// once each, in alphabetical order: what Excel offers to pick from.
fn column_entries(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Vec<ListItem> {
    let sh = &wb.sheets[sheet];
    let mut first = row;
    while first > 0 && !sh.is_blank(first - 1, col) {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < MAX_ROWS && !sh.is_blank(last + 1, col) {
        last += 1;
    }
    let mut texts: Vec<String> = (first..=last)
        .filter(|r| *r != row)
        .filter_map(|r| match sh.value_at(r, col) {
            Value::Text(text) => Some(text.to_string()),
            _ => None,
        })
        .collect();
    texts.sort_by_key(|text| text.to_lowercase());
    texts.dedup_by_key(|text| text.to_lowercase());
    texts
        .into_iter()
        .map(|text| ListItem {
            value: Value::text(&text),
            text,
        })
        .collect()
}

/// The lines to offer for a cell: those of its list when it shows an
/// arrow, else the entries of its column.
fn entries(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Vec<ListItem> {
    if wb.has_validation_dropdown(sheet, row, col) {
        wb.validation_list(sheet, row, col).unwrap_or_default()
    } else {
        column_entries(wb, sheet, row, col)
    }
}

/// The line the list opens on: the one the cell holds now.
fn current_line(items: &[ListItem], value: &Value) -> Option<usize> {
    if value.is_empty() {
        return None;
    }
    items.iter().position(|item| item.value == *value)
}

/// The line reached by typing a letter: the next one starting with it
/// after `from`, going round.
fn line_starting_with(items: &[ListItem], letter: &str, from: Option<usize>) -> Option<usize> {
    let letter = letter.to_lowercase();
    let count = items.len();
    let start = from.map_or(0, |at| at + 1);
    (0..count)
        .map(|step| (start + step) % count)
        .find(|at| items[*at].text.to_lowercase().starts_with(&letter))
}

/// Open the list of the active cell under it.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let (row, col) = app.active();
    // What is being typed gives way to the choice.
    if app.is_editing() {
        app.cancel_edit(window, cx);
    }
    let items = entries(&app.wb, sheet, row, col);
    if items.is_empty() || !app.may(Intent::EditCells(Rect::cell(row, col)), cx) {
        return;
    }
    let rect = active_rect(app);
    // As wide as the cell and its arrow.
    let width = match app.current_layout() {
        Some(layout) => {
            let (x0, _, x1, _) = layout.rect_px(rect);
            x1 - x0 + ARROW_SIZE
        }
        None => 0.0,
    };
    let cursor = current_line(&items, &app.wb.value_at(sheet, row, col));
    let weak = cx.weak_entity();
    let view = cx.new(|cx| DropList {
        app: weak,
        focus: cx.focus_handle(),
        sheet,
        cell: (row, col),
        items,
        cursor,
        scroll: cx.new(|_| ScrollState::new()),
        width: width.max(72.0),
    });
    if let Some(at) = cursor {
        let scroll = view.read(cx).scroll.clone();
        scroll.update(cx, |scroll, _| scroll.reveal(at, at, ROW_H, VISIBLE as f32 * ROW_H));
    }
    let focus = view.read(cx).focus.clone();
    app.open_popup_view(app.cell_anchor(rect), view, cx);
    window.focus(&focus);
}

struct DropList {
    app: WeakEntity<ExcelApp>,
    focus: FocusHandle,
    sheet: usize,
    cell: (u32, u32),
    items: Vec<ListItem>,
    /// The highlighted line.
    cursor: Option<usize>,
    scroll: Entity<ScrollState>,
    width: f32,
}

impl DropList {
    fn height(&self) -> f32 {
        self.items.len().min(VISIBLE) as f32 * ROW_H
    }

    fn move_to(&mut self, line: usize, cx: &mut Context<Self>) {
        self.cursor = Some(line);
        let height = self.height();
        self.scroll.update(cx, |scroll, _| scroll.reveal(line, line, ROW_H, height));
        cx.notify();
    }

    fn key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.items.len() - 1;
        let next = match (ev.keystroke.key.as_str(), self.cursor) {
            ("up", Some(at)) => at.saturating_sub(1),
            ("down", Some(at)) => (at + 1).min(last),
            ("pageup", Some(at)) => at.saturating_sub(VISIBLE),
            ("pagedown", Some(at)) => (at + VISIBLE).min(last),
            ("up" | "down" | "pageup" | "pagedown" | "home", _) => 0,
            ("end", _) => last,
            _ => {
                let typed = ev.keystroke.key_char.as_deref().filter(|ch| !ch.trim().is_empty());
                match typed.filter(|_| widgets::plain(ev)) {
                    Some(letter) => match line_starting_with(&self.items, letter, self.cursor) {
                        Some(line) => line,
                        None => return,
                    },
                    None => return,
                }
            }
        };
        self.move_to(next, cx);
        cx.stop_propagation();
    }

    /// Put the value of a line in the cell and close the list.
    fn pick(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(line).cloned() else {
            return;
        };
        let (sheet, (row, col)) = (self.sheet, self.cell);
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                if app.sheet != sheet {
                    return;
                }
                // The value of the line, not its text: a date stays a date.
                app.wb.set_value(sheet, row, col, item.value);
                app.commit_cells(t("Typing", "Saisie"), &[Rect::cell(row, col)], ChangeCause::Typing, cx);
            })
            .ok();
    }
}

impl Render for DropList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = self.cursor;
        let height = self.height();
        let texts: Vec<SharedString> = self.items.iter().map(|item| item.text.clone().into()).collect();
        let view = cx.entity().downgrade();
        let rows = widgets::virtual_rows(&self.scroll, texts.len(), ROW_H, height, cx, |line| {
            let view = view.clone();
            div()
                .id(("line", line))
                .size_full()
                .px(px(5.))
                .flex()
                .items_center()
                .when(cursor == Some(line), |d| d.bg(c(theme::ACCENT)).text_color(c(0xFFFFFF)))
                .when(cursor != Some(line), |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(div().truncate().child(texts[line].clone()))
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.pick(line, window, cx)).ok();
                })
                .into_any_element()
        });
        div()
            .id("validation-list")
            .track_focus(&self.focus)
            .w(px(self.width))
            .text_size(px(12.))
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &widgets::Accept, window, cx| {
                if let Some(line) = this.cursor {
                    this.pick(line, window, cx);
                }
            }))
            .child(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;
    use opensheet_engine::validation::{Operator, Validation, ValidationKind};

    fn workbook(cells: &[(u32, u32, &str)]) -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        for (row, col, text) in cells {
            wb.set_cell_text(0, *row, *col, text).unwrap();
        }
        wb.commit("setup");
        wb
    }

    fn texts(items: &[ListItem]) -> Vec<&str> {
        items.iter().map(|item| item.text.as_str()).collect()
    }

    #[test]
    fn a_cell_without_a_list_offers_the_texts_of_its_column() {
        let wb = workbook(&[
            (0, 0, "Pear"),
            (1, 0, "apple"),
            (2, 0, "12"),
            (3, 0, "Apple"),
            (5, 0, "fig"),
            (7, 0, "Kiwi"),
            (4, 1, "Other column"),
        ]);
        // From A5, the block A1:A6: numbers and repeats left out.
        let items = column_entries(&wb, 0, 4, 0);
        assert_eq!(texts(&items), ["apple", "fig", "Pear"]);
        assert_eq!(items[0].value, Value::text("apple"));
        // A8 stands alone below a blank cell.
        assert!(column_entries(&wb, 0, 7, 0).is_empty());
        // A7 joins the cells above and below it.
        assert_eq!(texts(&column_entries(&wb, 0, 6, 0)), ["fig", "Kiwi"]);
        assert_eq!(texts(&entries(&wb, 0, 4, 0)), ["apple", "fig", "Pear"]);
    }

    #[test]
    fn a_cell_with_a_list_offers_its_list() {
        let mut wb = workbook(&[(0, 0, "Pear"), (1, 0, "apple")]);
        let mut rule = Validation {
            kind: ValidationKind::List,
            operator: Operator::Between,
            ..Default::default()
        };
        rule.set_operands("Yes,No,3", "", Locale::en()).unwrap();
        wb.set_validation(0, &[Rect::cell(2, 0)], (2, 0), rule.clone());
        wb.commit("validation");
        let items = entries(&wb, 0, 2, 0);
        assert_eq!(texts(&items), ["Yes", "No", "3"]);
        // Picking the last line stores a number.
        assert_eq!(items[2].value, Value::Number(3.0));
        assert_eq!(current_line(&items, &Value::text("No")), Some(1));
        assert_eq!(current_line(&items, &Value::Number(3.0)), Some(2));
        assert_eq!(current_line(&items, &Value::text("Maybe")), None);
        assert_eq!(current_line(&items, &Value::Empty), None);
        // Without the arrow, the column is what there is to pick from.
        rule.in_cell_dropdown = false;
        wb.set_validation(0, &[Rect::cell(2, 0)], (2, 0), rule);
        wb.commit("validation");
        assert_eq!(texts(&entries(&wb, 0, 2, 0)), ["apple", "Pear"]);
    }

    #[test]
    fn typing_a_letter_goes_round_the_lines_starting_with_it() {
        let items: Vec<ListItem> = ["Apple", "Banana", "avocado", "Cherry"]
            .into_iter()
            .map(|text| ListItem {
                text: text.to_string(),
                value: Value::text(text),
            })
            .collect();
        assert_eq!(line_starting_with(&items, "a", None), Some(0));
        assert_eq!(line_starting_with(&items, "A", Some(0)), Some(2));
        assert_eq!(line_starting_with(&items, "a", Some(2)), Some(0));
        assert_eq!(line_starting_with(&items, "c", Some(3)), Some(3));
        assert_eq!(line_starting_with(&items, "z", Some(1)), None);
    }
}
