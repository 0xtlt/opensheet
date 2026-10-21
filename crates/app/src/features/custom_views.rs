//! View ▸ Custom Views: the dialog that lists the views of the workbook
//! (Show, Close, Add…, Delete) and Add View.
//!
//! The engine keeps the views (`opensheet_engine::custom_view`) and puts
//! back what a sheet holds itself: hidden rows and columns, filters, print
//! settings, frozen panes, gridlines and headings. The window is this
//! file's business: zoom, selection, scroll position and view mode of each
//! sheet, and the sheet in front.

use super::{Command, Cx, Feature, ViewMode};
use crate::app::ExcelApp;
use crate::grid::Selection;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::widgets::{self, Button, Checkbox, ListBox, TextField, dialog, label, ok_cancel};
use gpui::{
    Context, Entity, Focusable, SharedString, Stateful, Window, actions, div, prelude::*, px,
};
use opensheet_engine::Sheet;
use opensheet_engine::custom_view::{Shown, ViewKind, WindowState};

pub struct CustomViews;

commands! {
    /// What the user can ask of custom views.
    pub enum Cmd {
        Open = "eye", t("Custom Views", "Affichages personnalisés");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    /// Excel has no custom views while a sheet holds a table.
    fn enabled(self, app: &ExcelApp) -> bool {
        app.wb.custom_views_available()
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        app.popup = None;
        let names = view_names(app);
        app.open_modal(window, cx, |modal, _, _| ViewsDialog::new(modal, names));
    }
}

impl Feature for CustomViews {
    fn name(&self) -> &'static str {
        "custom-views"
    }
}

actions!(custom_views, [CustomViewsDialog]);

/// Make the window answer the View ▸ Custom Views… menu item.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(
        cx.listener(|app, _: &CustomViewsDialog, window, cx| {
            app.run_command(Cmd::Open, window, cx)
        }),
    )
}

// ------------------------------------------------------------------- window

fn kind_of(mode: ViewMode) -> ViewKind {
    match mode {
        ViewMode::Normal => ViewKind::Normal,
        ViewMode::PageLayout => ViewKind::PageLayout,
        ViewMode::PageBreakPreview => ViewKind::PageBreakPreview,
    }
}

fn mode_of(kind: ViewKind) -> ViewMode {
    match kind {
        ViewKind::Normal => ViewMode::Normal,
        ViewKind::PageLayout => ViewMode::PageLayout,
        ViewKind::PageBreakPreview => ViewMode::PageBreakPreview,
    }
}

/// Where the frozen rows and columns of a sheet end, in pixels at 100 %.
fn frozen_edge(sheet: &Sheet) -> (f64, f64) {
    sheet.corner_point(((sheet.frozen_rows, sheet.frozen_cols), (0.0, 0.0)))
}

/// The first cell shown in the scrolling part of a sheet's window.
fn top_left(sheet: &Sheet, scroll: (f64, f64), zoom: f32) -> (u32, u32) {
    let (fx, fy) = frozen_edge(sheet);
    let z = zoom.max(0.01) as f64;
    // Half a pixel in: a scroll position that rounding left just before a
    // cell edge still names that cell.
    sheet
        .corner_at(fx + scroll.0 / z + 0.5, fy + scroll.1 / z + 0.5)
        .0
}

/// The scroll position that shows a cell first.
fn scroll_to(sheet: &Sheet, cell: (u32, u32), zoom: f32) -> (f64, f64) {
    let (fx, fy) = frozen_edge(sheet);
    let (x, y) = sheet.corner_point((cell, (0.0, 0.0)));
    (
        ((x - fx) * zoom as f64).max(0.0),
        ((y - fy) * zoom as f64).max(0.0),
    )
}

/// The window of every sheet as it is now.
pub(crate) fn window_states(app: &ExcelApp) -> Vec<WindowState> {
    app.wb
        .sheets
        .iter()
        .enumerate()
        .map(|(index, sheet)| {
            let fallback = crate::app::SheetView::default();
            let view = app.views.get(index).unwrap_or(&fallback);
            WindowState {
                zoom: (app.zoom * 100.0).round() as u32,
                active: view.sel.active,
                ranges: view.sel.ranges.clone(),
                top_left: top_left(sheet, (view.scroll_x, view.scroll_y), app.zoom),
                frozen_rows: sheet.frozen_rows,
                frozen_cols: sheet.frozen_cols,
                show_gridlines: sheet.show_gridlines,
                show_headers: sheet.show_headers,
                mode: kind_of(view.mode),
                selected: index == app.sheet,
            }
        })
        .collect()
}

/// Put the windows of a view back, after the engine restored the sheets.
fn restore_windows(app: &mut ExcelApp, shown: &Shown, window: &mut Window, cx: &mut Cx) {
    let front = shown
        .iter()
        .find(|(_, state)| state.selected)
        .or(shown.first());
    if let Some((index, _)) = front {
        app.switch_sheet(*index, window, cx);
    }
    // The view mode goes through the application, which tells the page
    // layout feature; it is set sheet by sheet with that sheet in front.
    let back = app.sheet;
    for (index, state) in shown {
        let mode = mode_of(state.mode);
        if app.views.get(*index).is_some_and(|view| view.mode != mode) {
            app.sheet = *index;
            app.set_view_mode(mode, cx);
        }
    }
    app.sheet = back;
    // The zoom is that of the window: the sheet in front decides. It comes
    // after the view mode, which may bring a zoom of its own.
    if let Some((_, state)) = front {
        app.set_zoom(state.zoom as f32 / 100.0, cx);
    }
    for (index, state) in shown {
        let Some(sheet) = app.wb.sheets.get(*index) else {
            continue;
        };
        let scroll = scroll_to(sheet, state.top_left, app.zoom);
        let Some(view) = app.views.get_mut(*index) else {
            continue;
        };
        let mut selection = Selection::single(state.active.0, state.active.1);
        if !state.ranges.is_empty() {
            selection.ranges = state.ranges.clone();
        }
        view.sel = selection;
        (view.scroll_x, view.scroll_y) = scroll;
    }
}

/// Show a view, as one undo step. False when there is none of that name.
pub(crate) fn show(app: &mut ExcelApp, name: &str, window: &mut Window, cx: &mut Cx) -> bool {
    let Some(shown) = app.wb.show_custom_view(name) else {
        return false;
    };
    app.deselect_object(cx);
    app.commit(t("Custom View", "Affichage personnalisé"), cx);
    restore_windows(app, &shown, window, cx);
    app.refresh(cx);
    true
}

/// Add a view of the workbook as it is now (a view of the same name is
/// replaced), as one undo step.
pub(crate) fn add(
    app: &mut ExcelApp,
    name: &str,
    print_settings: bool,
    hidden_settings: bool,
    cx: &mut Cx,
) -> bool {
    let windows = window_states(app);
    if app
        .wb
        .add_custom_view(name, print_settings, hidden_settings, &windows)
        .is_err()
    {
        return false;
    }
    app.commit(t("Add View", "Ajouter un affichage"), cx);
    true
}

pub(crate) fn delete(app: &mut ExcelApp, name: &str, cx: &mut Cx) -> bool {
    if !app.wb.delete_custom_view(name) {
        return false;
    }
    app.commit(t("Delete View", "Supprimer l'affichage"), cx);
    true
}

/// A message of Excel with the name of a view in place of its `|`.
fn with_name(message: &str, name: &str) -> String {
    message.replacen('|', name, 1)
}

/// The names of the views, as the dialog lists them.
fn view_names(app: &ExcelApp) -> Vec<String> {
    app.wb
        .custom_views()
        .into_iter()
        .map(|view| view.name)
        .collect()
}

// ------------------------------------------------------------- Custom Views

struct ViewsDialog {
    modal: Modal,
    names: Vec<String>,
    selected: Option<usize>,
}

impl ViewsDialog {
    fn new(modal: Modal, names: Vec<String>) -> Self {
        ViewsDialog {
            modal,
            selected: if names.is_empty() { None } else { Some(0) },
            names,
        }
    }

    /// The views changed: list them again, with one selected near where
    /// the selection was.
    fn set_names(&mut self, names: Vec<String>, cx: &mut Context<Self>) {
        self.selected = match (self.selected, names.len()) {
            (_, 0) => None,
            (Some(index), count) => Some(index.min(count - 1)),
            (None, _) => Some(0),
        };
        self.names = names;
        cx.notify();
    }

    fn chosen(&self) -> Option<String> {
        self.selected
            .and_then(|index| self.names.get(index).cloned())
    }

    fn show(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.chosen() else {
            return;
        };
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| show(app, &name, window, cx));
    }

    /// Add… closes this dialog and opens Add View, as in Excel.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, |modal, _, cx| AddDialog::new(modal, cx));
        });
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.chosen() else {
            return;
        };
        let this = cx.weak_entity();
        let message = with_name(
            t(
                "Do you wish to delete the view '|'?",
                "Voulez-vous supprimer l'affichage '|'?",
            ),
            &name,
        );
        self.modal.update_app(cx, |app, cx| {
            let alert = Alert::new("OpenSheet", message)
                .warning()
                .button(t("Yes", "Oui"), move |app, _, cx| {
                    delete(app, &name, cx);
                    let names = view_names(app);
                    this.update(cx, |dialog, cx| dialog.set_names(names, cx))
                        .ok();
                })
                .close_button(t("No", "Non"));
            app.alert(alert, cx);
        });
    }
}

impl Render for ViewsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<SharedString> = self
            .names
            .iter()
            .map(|name| SharedString::from(name.clone()))
            .collect();
        let none = self.selected.is_none();
        let close = self.modal.clone();
        let buttons = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                Button::new("views-show", t("Show", "Afficher"))
                    .primary()
                    .width(96.0)
                    .disabled(none)
                    .on_click(cx.listener(Self::show)),
            )
            .child(
                Button::new("views-close", t("Close", "Fermer"))
                    .width(96.0)
                    .on_click(move |_, window, cx| close.close(window, cx)),
            )
            .child(
                Button::new("views-add", t("Add…", "Ajouter…"))
                    .width(96.0)
                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
            )
            .child(
                Button::new("views-delete", t("Delete", "Supprimer"))
                    .width(96.0)
                    .disabled(none)
                    .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
            );
        dialog(&self.modal, Cmd::Open.label())
            .width(420.0)
            .on_ok(cx.listener(Self::show))
            .child(label(t("Views:", "Affichages :")))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        ListBox::new("views-list", items, self.selected)
                            .width(280.0)
                            .height(200.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                this.selected = Some(*index);
                                cx.notify();
                            }))
                            .on_activate(cx.listener(|this, index: &usize, window, cx| {
                                this.selected = Some(*index);
                                this.show(&(), window, cx);
                            })),
                    )
                    .child(buttons),
            )
    }
}

// ----------------------------------------------------------------- Add View

struct AddDialog {
    modal: Modal,
    name: Entity<TextInput>,
    print_settings: bool,
    hidden_settings: bool,
}

impl AddDialog {
    fn new(modal: Modal, cx: &mut Context<Self>) -> Self {
        AddDialog {
            modal,
            name: widgets::input(cx, ""),
            print_settings: true,
            hidden_settings: true,
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_string();
        let (print_settings, hidden_settings) = (self.print_settings, self.hidden_settings);
        if name.is_empty() {
            // The keyboard goes back to the field once the message is read.
            let field = self.name.read(cx).focus_handle(cx);
            self.modal.update_app(cx, |app, cx| {
                let message = t(
                    "You must enter a name for the view.",
                    "Vous devez entrer un nom pour l'affichage.",
                );
                let alert = Alert::new("OpenSheet", message)
                    .warning()
                    .button(t("OK", "OK"), move |_, window, _| window.focus(&field));
                app.alert(alert, cx);
            });
            return;
        }
        let exists = self
            .modal
            .read_app(cx, |app| app.wb.custom_view(&name).is_some())
            .unwrap_or(false);
        if !exists {
            self.modal.close(window, cx);
            self.modal.update_app(cx, |app, cx| {
                add(app, &name, print_settings, hidden_settings, cx)
            });
            return;
        }
        // Excel asks before a view takes the place of another.
        let frame = self.modal.focus.clone();
        let field = self.name.read(cx).focus_handle(cx);
        let message = with_name(
            t(
                "A view named '|' already exists. Do you want to delete it and continue?",
                "Un affichage nommé '|' existe déjà. Voulez-vous le supprimer et continuer ?",
            ),
            &name,
        );
        self.modal.update_app(cx, |app, cx| {
            let alert = Alert::new("OpenSheet", message)
                .warning()
                .button(t("Yes", "Oui"), move |app, window, cx| {
                    add(app, &name, print_settings, hidden_settings, cx);
                    app.close_modal_of(&frame, window, cx);
                })
                .button(t("No", "Non"), move |_, window, _| window.focus(&field));
            app.alert(alert, cx);
        });
    }
}

impl Render for AddDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Add View", "Ajouter un affichage"))
            .width(400.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Name:", "Nom :")))
                    .child(TextField::new(&self.name).width(300.0)),
            )
            .child(label(t("Include in view:", "Inclure dans l'affichage :")))
            .child(
                div()
                    .pl(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        Checkbox::new(
                            "view-print",
                            t("Print settings", "Paramètres d'impression"),
                            self.print_settings,
                        )
                        .on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.print_settings = *on;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Checkbox::new(
                            "view-hidden",
                            t(
                                "Hidden rows, columns, and filter settings",
                                "Paramètres masqués des lignes, colonnes et filtres",
                            ),
                            self.hidden_settings,
                        )
                        .on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.hidden_settings = *on;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
    use opensheet_engine::Rect;
    use opensheet_engine::page::Orientation;

    /// A window on an empty workbook, with the key bindings of the application.
    fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
        cx.update(crate::bind_keys);
        let (app, cx) = cx.add_window_view(ExcelApp::new);
        redraw(cx);
        (app, cx)
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    }

    fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
        app.update_in(cx, |app, window, cx| app.run_command(Cmd::Open, window, cx));
        redraw(cx);
    }

    fn views(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<String> {
        app.update(cx, |app, _| {
            app.wb
                .custom_views()
                .into_iter()
                .map(|view| view.name)
                .collect()
        })
    }

    fn modals(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
        app.update(cx, |app, _| app.modals.len())
    }

    /// Hide a row, rotate the page, zoom, select and scroll.
    fn arrange(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
        app.update(cx, |app, cx| {
            app.wb.snapshot();
            app.wb.sheets[0].hidden_rows.insert(4);
            app.wb.sheets[0].page.orientation = Orientation::Landscape;
            app.wb.sheets[0].frozen_rows = 1;
            app.commit("setup", cx);
            // Page Break Preview brings its own zoom: ours comes after.
            app.set_view_mode(ViewMode::PageBreakPreview, cx);
            app.set_zoom(1.5, cx);
            app.select_rect(Rect::new(6, 1, 8, 2), cx);
            let sheet = &app.wb.sheets[0];
            (app.views[0].scroll_x, app.views[0].scroll_y) = scroll_to(sheet, (30, 3), app.zoom);
        });
        redraw(cx);
    }

    /// Undo everything `arrange` did, by other means than undo.
    fn disturb(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
        app.update(cx, |app, cx| {
            app.wb.snapshot();
            app.wb.sheets[0].hidden_rows.clear();
            app.wb.sheets[0].page.orientation = Orientation::Portrait;
            app.wb.sheets[0].frozen_rows = 0;
            app.commit("changes", cx);
            app.set_zoom(1.0, cx);
            app.set_view_mode(ViewMode::Normal, cx);
            app.select_cell(0, 0, cx);
            (app.views[0].scroll_x, app.views[0].scroll_y) = (0.0, 0.0);
        });
        redraw(cx);
    }

    #[gpui::test]
    fn a_view_is_added_shown_and_deleted_from_the_dialogs(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        arrange(&app, cx);
        // Custom Views, then Add…: the first dialog gives way to Add View.
        run(&app, cx);
        assert_eq!(modals(&app, cx), 1);
        cx.simulate_keystrokes("escape");
        let dialog = app.update_in(cx, |app, window, cx| {
            app.open_modal(window, cx, |modal, _, _| {
                ViewsDialog::new(modal, Vec::new())
            })
        });
        dialog.update_in(cx, |dialog, window, cx| dialog.add(window, cx));
        redraw(cx);
        assert_eq!(modals(&app, cx), 1);
        // Without a name, Excel insists.
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modals(&app, cx), 2);
        cx.simulate_keystrokes("enter");
        redraw(cx);
        cx.simulate_input("Review");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modals(&app, cx), 0);
        assert_eq!(views(&app, cx), vec!["Review"]);
        assert_eq!(
            app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
                .as_deref(),
            Some("Add View")
        );

        disturb(&app, cx);
        // Custom Views: the view is selected, Return shows it.
        run(&app, cx);
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modals(&app, cx), 0);
        app.update(cx, |app, _| {
            let sheet = &app.wb.sheets[0];
            assert!(sheet.hidden_rows.contains(&4));
            assert_eq!(
                (sheet.page.orientation, sheet.frozen_rows),
                (Orientation::Landscape, 1)
            );
            assert_eq!(app.zoom, 1.5);
            assert_eq!(app.view_mode(), ViewMode::PageBreakPreview);
            assert_eq!(
                (app.sel().active, app.sel().current()),
                ((6, 1), Rect::new(6, 1, 8, 2))
            );
            assert_eq!(
                top_left(
                    sheet,
                    (app.views[0].scroll_x, app.views[0].scroll_y),
                    app.zoom
                ),
                (30, 3)
            );
            assert_eq!(app.wb.undo_label(), Some("Custom View"));
        });
        // Undo takes the sheet back (the window stays, as in Excel).
        cx.simulate_keystrokes("cmd-z");
        assert!(app.update(cx, |app, _| app.wb.sheets[0].hidden_rows.is_empty()));
    }

    #[gpui::test]
    fn views_are_replaced_and_deleted_after_a_question(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        app.update(cx, |app, cx| {
            assert!(add(app, "One", true, true, cx));
            assert!(add(app, "Two", false, true, cx));
        });
        // Add View with a name that is taken: Excel asks; No keeps both.
        let dialog = app.update_in(cx, |app, window, cx| {
            app.open_modal(window, cx, |modal, _, cx| AddDialog::new(modal, cx))
        });
        redraw(cx);
        // (The name field takes the keyboard on the frame after the dialog
        // opens, which a headless window does not draw.)
        dialog.update_in(cx, |dialog, window, cx| {
            window.focus(&dialog.name.read(cx).focus_handle(cx))
        });
        cx.simulate_input("one");
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(modals(&app, cx), 2);
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert_eq!(
            (modals(&app, cx), views(&app, cx)),
            (1, vec!["One".to_string(), "Two".to_string()])
        );
        // Yes replaces: the new view takes the name as typed.
        cx.simulate_keystrokes("enter");
        redraw(cx);
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(
            (modals(&app, cx), views(&app, cx)),
            (0, vec!["Two".to_string(), "one".to_string()])
        );
        // Delete asks too: No keeps the view, Yes deletes it.
        let dialog = app.update_in(cx, |app, window, cx| {
            let names = view_names(app);
            app.open_modal(window, cx, |modal, _, _| ViewsDialog::new(modal, names))
        });
        dialog.update(cx, |dialog, cx| dialog.delete(cx));
        redraw(cx);
        assert_eq!(modals(&app, cx), 2);
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert_eq!(views(&app, cx).len(), 2);
        dialog.update(cx, |dialog, cx| dialog.delete(cx));
        redraw(cx);
        cx.simulate_keystrokes("enter");
        redraw(cx);
        assert_eq!(views(&app, cx), vec!["one"]);
        // The list of the dialog follows.
        dialog.update(cx, |dialog, _| {
            assert_eq!(
                (dialog.names.clone(), dialog.selected),
                (vec!["one".to_string()], Some(0))
            )
        });
        cx.simulate_keystrokes("escape");
        redraw(cx);
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(views(&app, cx).len(), 2);
    }

    #[gpui::test]
    fn a_view_brings_its_sheet_to_the_front(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        app.update_in(cx, |app, window, cx| {
            app.add_sheet(cx);
            app.switch_sheet(1, window, cx);
            app.select_cell(9, 9, cx);
            assert!(add(app, "Second sheet", true, true, cx));
            app.switch_sheet(0, window, cx);
            assert!(show(app, "second sheet", window, cx));
            assert_eq!((app.sheet, app.active()), (1, (9, 9)));
            assert!(!show(app, "missing", window, cx));
        });
    }

    #[gpui::test]
    fn a_table_takes_custom_views_away(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        app.update(cx, |app, cx| {
            assert!(Cmd::Open.enabled(app));
            app.wb.set_cell_text(0, 0, 0, "Name").unwrap();
            app.wb.set_cell_text(0, 1, 0, "a").unwrap();
            app.wb.add_table(0, Rect::new(0, 0, 1, 0), true).unwrap();
            app.commit("setup", cx);
            assert!(!Cmd::Open.enabled(app));
            assert!(!add(app, "Nope", true, true, cx));
        });
        // The command does nothing then.
        run(&app, cx);
        assert_eq!(modals(&app, cx), 0);
    }

    #[test]
    fn scroll_positions_name_cells() {
        let mut sheet = Sheet::new("S");
        let (w, h) = (
            sheet.default_col_width as f64,
            sheet.default_row_height as f64,
        );
        assert_eq!(top_left(&sheet, (0.0, 0.0), 1.0), (0, 0));
        assert_eq!(top_left(&sheet, (3.0 * w, 10.0 * h), 1.0), (10, 3));
        // At another zoom the scroll position is in zoomed pixels.
        assert_eq!(
            top_left(&sheet, (3.0 * w * 2.0, 10.0 * h * 2.0), 2.0),
            (10, 3)
        );
        assert_eq!(scroll_to(&sheet, (10, 3), 2.0), (6.0 * w, 20.0 * h));
        // Frozen panes: the scrolling part starts after them.
        sheet.frozen_rows = 2;
        sheet.frozen_cols = 1;
        assert_eq!(top_left(&sheet, (0.0, 0.0), 1.0), (2, 1));
        assert_eq!(scroll_to(&sheet, (2, 1), 1.0), (0.0, 0.0));
        assert_eq!(scroll_to(&sheet, (12, 4), 1.0), (3.0 * w, 10.0 * h));
        assert_eq!(with_name("Delete '|'?", "A|B"), "Delete 'A|B'?");
    }
}
