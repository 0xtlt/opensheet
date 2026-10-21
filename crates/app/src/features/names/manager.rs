//! The Name Manager: the list of names and tables, with New, Edit, Delete,
//! the Filter menu and the "Refers to" box that redefines the selected
//! name in place.

use super::{
    Column, NameFilter, NameRow, RowKind, dialogs, filter_rows, invalid_reference, locked,
    name_rows, sort_rows,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{Button, ListBox, RefEdit, dialog, icon, label};
use gpui::{AnyElement, Context, Entity, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::names::NameScope;
use std::rc::Rc;

/// Widths of the columns; the comment takes what is left.
const WIDTHS: [f32; 4] = [150.0, 130.0, 180.0, 90.0];

/// What the dialog reads from the workbook when it opens and each time
/// the workbook changes under it.
struct Snapshot {
    generation: u64,
    rows: Vec<NameRow>,
    /// The current sheet is protected: names can only be looked at.
    locked: bool,
}

fn snapshot(app: &ExcelApp) -> Snapshot {
    let (row, col) = app.active();
    Snapshot {
        generation: app.wb.generation,
        rows: name_rows(&app.wb, app.sheet, row, col),
        locked: locked(app).is_some(),
    }
}

pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let first = snapshot(app);
    app.open_modal(window, cx, move |modal, _, cx| {
        Manager::new(modal, first, cx)
    });
}

/// Redefine a name with the text of the "Refers to" box. Returns false,
/// after saying why, when the text is not a formula.
fn store_refers(app: &mut ExcelApp, row: &NameRow, text: &str, cx: &mut Cx) -> bool {
    let RowKind::Name(scope) = row.kind else {
        return false;
    };
    let (r, c) = app.active();
    let from = match scope {
        NameScope::Sheet(index) => index,
        NameScope::Workbook => app.sheet,
    };
    let Ok(expr) = app.wb.parse_name_formula(text, from, r, c) else {
        app.alert(Alert::new("OpenSheet", invalid_reference()).warning(), cx);
        return false;
    };
    if app
        .wb
        .edit_name(scope, &row.name, &row.name, expr, &row.comment)
        .is_err()
    {
        return false;
    }
    app.commit(t("Edit Name", "Modifier le nom"), cx);
    true
}

/// What the user was about to do when asked whether the edited reference
/// should be kept.
#[derive(Clone, Copy)]
enum Next {
    Select(usize),
    Close,
}

struct Manager {
    modal: Modal,
    /// Every name of the workbook, as last read.
    all: Vec<NameRow>,
    /// The rows shown: filtered and sorted.
    rows: Rc<Vec<NameRow>>,
    generation: u64,
    locked: bool,
    selected: Option<usize>,
    filter: NameFilter,
    sort: (Column, bool),
    refers: Entity<TextInput>,
    /// What the "Refers to" box held when the row was selected.
    refers_original: String,
}

impl Manager {
    fn new(modal: Modal, first: Snapshot, cx: &mut Context<Self>) -> Self {
        let mut manager = Manager {
            refers: modal.ref_input(RefStyle::formula(), "", cx),
            modal,
            all: Vec::new(),
            rows: Rc::new(Vec::new()),
            generation: 0,
            locked: false,
            selected: None,
            filter: NameFilter::default(),
            sort: (Column::Name, true),
            refers_original: String::new(),
        };
        manager.load(first, cx);
        manager
    }

    fn row(&self) -> Option<&NameRow> {
        self.selected.and_then(|index| self.rows.get(index))
    }

    fn load(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.generation = snapshot.generation;
        self.locked = snapshot.locked;
        self.all = snapshot.rows;
        self.arrange(cx);
    }

    /// Filter and sort again, keeping the selected name selected.
    fn arrange(&mut self, cx: &mut Context<Self>) {
        let kept = self
            .row()
            .map(|row| (row.kind.clone(), row.name.to_lowercase()));
        let mut rows = filter_rows(self.all.clone(), &self.filter);
        sort_rows(&mut rows, self.sort.0, self.sort.1);
        let found = kept.and_then(|(kind, name)| {
            rows.iter()
                .position(|row| row.kind == kind && row.name.to_lowercase() == name)
        });
        // A name that was renamed or deleted leaves the selection where
        // it was in the list.
        let fallback = self.selected.unwrap_or(0).min(rows.len().saturating_sub(1));
        self.rows = Rc::new(rows);
        self.show(found.or((!self.rows.is_empty()).then_some(fallback)), cx);
    }

    /// Select a row and show what it refers to.
    fn show(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.selected = index;
        self.refers_original = self
            .row()
            .map(|row| row.refers_to.clone())
            .unwrap_or_default();
        let text = self.refers_original.clone();
        self.refers.update(cx, |input, cx| {
            if input.text() != text {
                input.set_text(text, cx);
            }
        });
        cx.notify();
    }

    /// The "Refers to" box holds something that was not stored yet.
    fn refers_edited(&self, cx: &Context<Self>) -> bool {
        self.row().is_some_and(|row| row.is_name())
            && self.refers.read(cx).text() != self.refers_original
    }

    fn accept_refers(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.row().cloned() else {
            return;
        };
        let text = self.refers.read(cx).text().to_string();
        // The list is read again at the next frame: the workbook changed.
        self.modal
            .update_app(cx, |app, cx| store_refers(app, &row, &text, cx));
    }

    fn cancel_refers(&mut self, cx: &mut Context<Self>) {
        self.show(self.selected, cx);
    }

    /// Go on to `next`, first asking what to do with an edited reference.
    fn leave(&mut self, next: Next, window: &mut Window, cx: &mut Context<Self>) {
        if !self.refers_edited(cx) {
            self.go(next, window, cx);
            return;
        }
        let Some(row) = self.row().cloned() else {
            return;
        };
        let text = self.refers.read(cx).text().to_string();
        let me = cx.weak_entity();
        let modal = self.modal.clone();
        let proceed = move |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| match next {
            Next::Close => app.close_modal_of(&modal.focus, window, cx),
            Next::Select(index) => {
                me.update(cx, |this, cx| this.show(Some(index), cx)).ok();
            }
        };
        let keep = proceed.clone();
        let alert = Alert::new(
            "OpenSheet",
            t(
                "Do you want to save the changes you made to the name reference?",
                "Voulez-vous enregistrer les modifications apportées à la référence de nom ?",
            ),
        )
        .warning()
        .button(t("Yes", "Oui"), move |app, window, cx| {
            if store_refers(app, &row, &text, cx) {
                keep(app, window, cx);
            }
        })
        .button(t("No", "Non"), proceed);
        self.modal.update_app(cx, |app, cx| app.alert(alert, cx));
    }

    fn go(&mut self, next: Next, window: &mut Window, cx: &mut Context<Self>) {
        match next {
            Next::Close => self.modal.close(window, cx),
            Next::Select(index) => self.show(Some(index), cx),
        }
    }

    fn new_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal
            .update_app(cx, |app, cx| dialogs::open_new_name(app, window, cx));
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }
        let Some(row) = self
            .row()
            .filter(|row| row.is_name() || row.is_table())
            .cloned()
        else {
            return;
        };
        self.modal
            .update_app(cx, |app, cx| dialogs::open_edit_name(app, &row, window, cx));
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.row().cloned() else {
            return;
        };
        let RowKind::Name(scope) = row.kind else {
            return;
        };
        let question = t(
            "Are you sure you want to delete the name {}?",
            "Êtes-vous sûr de vouloir supprimer le nom « {} » ?",
        )
        .replace("{}", &row.name);
        let alert = Alert::new("OpenSheet", question)
            .warning()
            .button(t("OK", "OK"), move |app, _, cx| {
                if app.wb.delete_name(scope, &row.name) {
                    app.commit(t("Delete Name", "Supprimer le nom"), cx);
                }
            })
            .close_button(t("Cancel", "Annuler"));
        self.modal.update_app(cx, |app, cx| app.alert(alert, cx));
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = if self.sort.0 == column {
            (column, !self.sort.1)
        } else {
            (column, true)
        };
        self.arrange(cx);
    }

    /// The Filter menu: three pairs of exclusive choices.
    fn filter_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.weak_entity();
        let filter = self.filter;
        let entry = |label: &'static str, checked: bool, change: fn(&mut NameFilter, bool)| {
            let me = me.clone();
            MenuEntry::new(label, move |_, _, cx| {
                me.update(cx, |this, cx| {
                    change(&mut this.filter, !checked);
                    this.arrange(cx);
                })
                .ok();
            })
            .checked(checked)
        };
        let entries = vec![
            entry(t("Clear Filter", "Effacer le filtre"), false, |f, _| {
                *f = NameFilter::default()
            })
            .disabled(filter.is_empty()),
            MenuEntry::separator(),
            entry(
                t(
                    "Names Scoped to Worksheet",
                    "Noms étendus à la feuille de calcul",
                ),
                filter.sheet_scoped == Some(true),
                |f, on| f.sheet_scoped = on.then_some(true),
            ),
            entry(
                t("Names Scoped to Workbook", "Noms étendus au classeur"),
                filter.sheet_scoped == Some(false),
                |f, on| f.sheet_scoped = on.then_some(false),
            ),
            MenuEntry::separator(),
            entry(
                t("Names with Errors", "Noms comportant des erreurs"),
                filter.with_errors == Some(true),
                |f, on| f.with_errors = on.then_some(true),
            ),
            entry(
                t("Names without Errors", "Noms sans erreur"),
                filter.with_errors == Some(false),
                |f, on| f.with_errors = on.then_some(false),
            ),
            MenuEntry::separator(),
            entry(
                t("Defined Names", "Noms définis"),
                filter.tables == Some(false),
                |f, on| f.tables = on.then_some(false),
            ),
            entry(
                t("Table Names", "Noms de tableaux"),
                filter.tables == Some(true),
                |f, on| f.tables = on.then_some(true),
            ),
        ];
        let position = window.mouse_position();
        self.modal
            .update_app(cx, |app, cx| app.open_context_menu(position, entries, cx));
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let (sorted, ascending) = self.sort;
        let mut cells: Vec<AnyElement> = Vec::new();
        for (i, column) in Column::ALL.into_iter().enumerate() {
            let cell = div()
                .id(("column", i))
                .h_full()
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(4.))
                .border_r_1()
                .border_color(c(theme::BORDER))
                .hover(|s| s.bg(c(theme::HOVER)))
                .when_some(WIDTHS.get(i), |d, width| d.w(px(*width)).flex_none())
                .when(i >= WIDTHS.len(), |d| d.flex_1().min_w(px(0.)))
                .child(div().flex_1().truncate().child(column.title()))
                .when(sorted == column, |d| {
                    d.child(icon(
                        if ascending {
                            "chevron-up"
                        } else {
                            "chevron-down"
                        },
                        9.0,
                        theme::TEXT_MUTED,
                    ))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx)));
            cells.push(cell.into_any_element());
        }
        div()
            .h(px(22.))
            .flex()
            .bg(c(theme::CHROME_BG))
            .border_1()
            .border_color(c(0xABABAB))
            .rounded_t(px(3.))
            .children(cells)
            .into_any_element()
    }
}

/// One line of the list: an icon telling names from tables, then the
/// columns.
fn line(row: &NameRow) -> AnyElement {
    let cell = |text: &str, width: Option<f32>| {
        div()
            .px(px(6.))
            .when_some(width, |d, width| d.w(px(width)).flex_none())
            .when(width.is_none(), |d| d.flex_1().min_w(px(0.)))
            .truncate()
            .child(SharedString::from(text.to_string()))
    };
    div()
        .w_full()
        .flex()
        .items_center()
        .child(
            // The row is already padded like the cells of the header.
            div()
                .w(px(WIDTHS[0] - 6.0))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(
                    if row.is_table() { "table" } else { "tag" },
                    12.0,
                    theme::TEXT_MUTED,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .child(SharedString::from(row.name.clone())),
                ),
        )
        .child(cell(&row.value, Some(WIDTHS[1])))
        .child(cell(&row.refers_to, Some(WIDTHS[2])))
        .child(cell(&row.scope, Some(WIDTHS[3])))
        .child(cell(&row.comment, None))
        .into_any_element()
}

impl Render for Manager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A name was added, changed or deleted (or the change undone).
        if self.modal.read_app(cx, |app| app.wb.generation) != Some(self.generation) {
            if let Some(fresh) = self.modal.read_app(cx, snapshot) {
                self.load(fresh, cx);
            }
        }
        let rows = self.rows.clone();
        let names: Vec<SharedString> = rows
            .iter()
            .map(|row| SharedString::from(row.name.clone()))
            .collect();
        let row = self.row();
        let is_name = row.is_some_and(|row| row.is_name());
        let edited = self.refers_edited(cx);
        // A table can be renamed here; print areas belong to Page Setup.
        let can_edit = row.is_some_and(|row| row.is_name() || row.is_table()) && !self.locked;
        let can_delete = is_name && !self.locked;

        let mark = |id: &'static str, icon_name: &'static str, color: u32| {
            div()
                .id(id)
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .bg(c(0xFFFFFF))
                .when(edited, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(icon(
                    icon_name,
                    12.0,
                    if edited { color } else { theme::TEXT_DISABLED },
                ))
        };
        let refers: AnyElement = if is_name && !self.locked {
            RefEdit::new(&self.refers, &self.modal).into_any_element()
        } else {
            // Tables are resized on the sheet, print areas set in Page Setup.
            div()
                .flex_1()
                .h(px(24.))
                .px(px(6.))
                .flex()
                .items_center()
                .border_1()
                .border_color(c(theme::BORDER))
                .rounded(px(3.))
                .bg(c(theme::CHROME_BG))
                .text_color(c(theme::TEXT_MUTED))
                .child(
                    div()
                        .truncate()
                        .child(SharedString::from(self.refers_original.clone())),
                )
                .into_any_element()
        };

        dialog(&self.modal, t("Name Manager", "Gestionnaire de noms"))
            .width(680.0)
            .on_ok(cx.listener(|this, _, _, cx| {
                if this.refers_edited(cx) {
                    this.accept_refers(cx);
                }
            }))
            .on_cancel(cx.listener(|this, _, window, cx| {
                if this.refers_edited(cx) {
                    this.cancel_refers(cx);
                } else {
                    this.modal.close(window, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("new", t("New...", "Nouveau..."))
                            .disabled(self.locked)
                            .on_click(cx.listener(|this, _, window, cx| this.new_name(window, cx))),
                    )
                    .child(
                        Button::new("edit", t("Edit...", "Modifier..."))
                            .disabled(!can_edit)
                            .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                    )
                    .child(
                        Button::new("delete", t("Delete", "Supprimer"))
                            .disabled(!can_delete)
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("filter")
                            .h(px(24.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(c(0xABABAB))
                            .bg(c(0xFFFFFF))
                            .hover(|s| s.bg(c(theme::HOVER)))
                            .when(!self.filter.is_empty(), |d| {
                                d.font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(icon("filter", 12.0, theme::TEXT_MUTED))
                            .child(t("Filter", "Filtrer"))
                            .child(icon("chevron-down", 9.0, theme::TEXT_MUTED))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.filter_menu(window, cx)),
                            ),
                    ),
            )
            .child(
                div().flex().flex_col().child(self.header(cx)).child(
                    div().flex().child(
                        ListBox::new("names", names, self.selected)
                            .height(220.0)
                            .row(move |index, _| line(&rows[index]))
                            .on_change(cx.listener(|this, index: &usize, window, cx| {
                                if this.selected != Some(*index) {
                                    this.leave(Next::Select(*index), window, cx);
                                }
                            }))
                            .on_activate(
                                cx.listener(|this, _: &usize, window, cx| this.edit(window, cx)),
                            ),
                    ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(t("Refers to:", "Fait référence à :")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                mark("refers-cancel", "close", 0xC42B1C)
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_refers(cx))),
                            )
                            .child(mark("refers-accept", "check", theme::ACCENT).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if this.refers_edited(cx) {
                                        this.accept_refers(cx);
                                    }
                                }),
                            ))
                            .child(refers),
                    ),
            )
            .footer(
                Button::new("close", t("Close", "Fermer"))
                    .primary()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.leave(Next::Close, window, cx)),
                    ),
            )
    }
}
