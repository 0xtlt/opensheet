//! What the window offers to host the interface of features: modal
//! dialogs (stacked: an alert over a dialog), message boxes, and the panes
//! docked right of the grid.

use crate::app::{ExcelApp, MenuAction};
use crate::i18n::t;
use crate::text_input::TextInput;
use crate::theme::{self, c, ca};
use crate::view::icon;
use crate::widgets::{self, Button};
use gpui::{
    AnyElement, AnyView, App, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, Render, SharedString, WeakEntity, Window, div, prelude::*, px,
};
use opensheet_engine::Rect;
use opensheet_engine::refs::{col_to_letters, MAX_COLS, MAX_ROWS};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ------------------------------------------------------------------- modals

/// How a range field writes the cells picked on the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefStyle {
    /// `$A$1` rather than `A1`.
    pub absolute: bool,
    /// Prefix with the sheet name (`Sheet1!`).
    pub sheet: bool,
    /// Start with `=`.
    pub equals: bool,
}

impl RefStyle {
    /// `$A$1:$B$5`.
    pub fn absolute() -> RefStyle {
        RefStyle {
            absolute: true,
            sheet: false,
            equals: false,
        }
    }

    /// `=Sheet1!$A$1:$B$5`, what most of Excel's dialogs write.
    pub fn formula() -> RefStyle {
        RefStyle {
            absolute: true,
            sheet: true,
            equals: true,
        }
    }

    /// The text of a reference to `rect` on the sheet named `sheet`.
    pub fn format(&self, rect: Rect, sheet: &str) -> String {
        let dollar = if self.absolute { "$" } else { "" };
        let cell = |row: u32, col: u32| format!("{dollar}{}{dollar}{}", col_to_letters(col), row + 1);
        let mut out = String::new();
        if self.equals {
            out.push('=');
        }
        if self.sheet {
            opensheet_engine::ast::write_sheet_name(sheet, &mut out);
            out.push('!');
        }
        if rect.c0 == 0 && rect.c1 == MAX_COLS - 1 {
            out.push_str(&format!("{dollar}{}:{dollar}{}", rect.r0 + 1, rect.r1 + 1));
        } else if rect.r0 == 0 && rect.r1 == MAX_ROWS - 1 {
            out.push_str(&format!("{dollar}{}:{dollar}{}", col_to_letters(rect.c0), col_to_letters(rect.c1)));
        } else if rect.is_single() {
            out.push_str(&cell(rect.r0, rect.c0));
        } else {
            out.push_str(&format!("{}:{}", cell(rect.r0, rect.c0), cell(rect.r1, rect.c1)));
        }
        out
    }
}

/// What a dialog shares with the window that hosts it.
pub struct ModalShared {
    /// Where the user dragged the dialog, from the centre of the window.
    pub(crate) offset: Cell<Point<Pixels>>,
    /// Title bar being dragged: pointer position and offset at the start.
    pub(crate) drag: Cell<Option<(Point<Pixels>, Point<Pixels>)>>,
    /// The range field the dialog is shrunk to while cells are picked.
    pub(crate) collapsed: RefCell<Option<Entity<TextInput>>>,
    /// The range fields of the dialog.
    pub(crate) ref_inputs: RefCell<Vec<(WeakEntity<TextInput>, RefStyle)>>,
}

/// The handle a dialog receives when it is opened
/// (`ExcelApp::open_modal`): how it reaches the application and closes
/// itself. The dialog passes it to `widgets::dialog`.
#[derive(Clone)]
pub struct Modal {
    pub app: WeakEntity<ExcelApp>,
    /// Focus handle of the dialog frame.
    pub focus: FocusHandle,
    pub(crate) shared: Rc<ModalShared>,
}

impl Modal {
    /// Close the dialog (and whatever was opened over it).
    pub fn close(&self, window: &mut Window, cx: &mut App) {
        let focus = self.focus.clone();
        self.app
            .update(cx, |app, cx| app.close_modal_of(&focus, window, cx))
            .ok();
    }

    /// Change the application: what OK does.
    ///
    /// ```ignore
    /// self.modal.update_app(cx, |app, cx| {
    ///     app.apply_style(t("Format Cells", "Format de cellule"), cx, |s| s.wrap = true);
    /// });
    /// self.modal.close(window, cx);
    /// ```
    pub fn update_app<R>(&self, cx: &mut App, f: impl FnOnce(&mut ExcelApp, &mut Context<ExcelApp>) -> R) -> Option<R> {
        self.app.update(cx, f).ok()
    }

    /// Look at the application (the workbook, the selection).
    pub fn read_app<R>(&self, cx: &App, f: impl FnOnce(&ExcelApp) -> R) -> Option<R> {
        self.app.upgrade().map(|app| f(app.read(cx)))
    }

    /// A text field for `widgets::RefEdit`: while it has the keyboard,
    /// selecting cells on the sheet writes their reference into it.
    pub fn ref_input<V: 'static>(&self, style: RefStyle, text: impl Into<String>, cx: &mut Context<V>) -> Entity<TextInput> {
        let field = widgets::input(cx, text);
        self.shared.ref_inputs.borrow_mut().push((field.downgrade(), style));
        field
    }
}

pub(crate) struct ModalEntry {
    view: AnyView,
    modal: Modal,
    /// What had the keyboard when the dialog opened.
    restore: Option<FocusHandle>,
    /// The dialog has not been given the keyboard yet.
    needs_focus: bool,
}

/// Icon of a message box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Info,
    Warning,
    Stop,
}

/// A message box: a title, a text and buttons. The first button is the
/// default one (Return) unless `default_button` names another, the last
/// one answers Escape.
///
/// ```ignore
/// app.alert(
///     Alert::new("OpenSheet", t("This value doesn't match the data validation restrictions defined for this cell.",
///                               "Cette valeur ne respecte pas les restrictions de validation des données définies pour cette cellule."))
///         .stop()
///         .button(t("Retry", "Réessayer"), |app, window, cx| app.focus_editor(window, cx))
///         .button(t("Cancel", "Annuler"), |app, window, cx| app.cancel_edit(window, cx)),
///     cx,
/// );
/// ```
pub struct Alert {
    title: SharedString,
    message: SharedString,
    kind: AlertKind,
    buttons: Vec<(SharedString, Option<MenuAction>)>,
    /// Index of the button Return presses.
    default: usize,
}

impl Alert {
    /// A message with a single OK button (until `button` adds others).
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Alert {
        Alert {
            title: title.into(),
            message: message.into(),
            kind: AlertKind::Info,
            buttons: Vec::new(),
            default: 0,
        }
    }

    /// Make the button at `index` the default one: Excel's warnings
    /// propose "No".
    pub fn default_button(mut self, index: usize) -> Alert {
        self.default = index;
        self
    }

    pub fn warning(mut self) -> Alert {
        self.kind = AlertKind::Warning;
        self
    }

    pub fn stop(mut self) -> Alert {
        self.kind = AlertKind::Stop;
        self
    }

    /// Add a button; `action` runs after the box has closed.
    pub fn button(
        mut self,
        label: impl Into<SharedString>,
        action: impl Fn(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static,
    ) -> Alert {
        self.buttons.push((label.into(), Some(Rc::new(action))));
        self
    }

    /// Add a button that only closes the box.
    pub fn close_button(mut self, label: impl Into<SharedString>) -> Alert {
        self.buttons.push((label.into(), None));
        self
    }
}

struct MessageBox {
    modal: Modal,
    alert: Alert,
}

impl MessageBox {
    fn press(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.alert.buttons.get(index).and_then(|(_, action)| action.clone());
        self.modal.close(window, cx);
        if let Some(action) = action {
            self.modal.update_app(cx, |app, cx| action(app, window, cx));
        }
    }
}

impl Render for MessageBox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let last = self.alert.buttons.len().saturating_sub(1);
        let default = self.alert.default.min(last);
        let buttons: Vec<AnyElement> = self
            .alert
            .buttons
            .iter()
            .enumerate()
            .map(|(i, (label, _))| {
                let mut button = Button::new(("alert-button", i), label.clone())
                    .on_click(cx.listener(move |this, _, window, cx| this.press(i, window, cx)));
                if i == default {
                    button = button.primary();
                }
                button.into_any_element()
            })
            .collect();
        let (glyph, color) = match self.alert.kind {
            AlertKind::Info => ("i", 0x0F6CBD),
            AlertKind::Warning => ("!", 0xC19C00),
            AlertKind::Stop => ("×", 0xC42B1C),
        };
        widgets::dialog(&self.modal, self.alert.title.clone())
            .width(440.0)
            .on_ok(cx.listener(move |this, _, window, cx| this.press(default, window, cx)))
            .on_cancel(cx.listener(move |this, _, window, cx| this.press(last, window, cx)))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .rounded_full()
                            .bg(c(color))
                            .text_color(c(0xFFFFFF))
                            .text_size(px(17.))
                            .font_weight(FontWeight::BOLD)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(glyph),
                    )
                    .child(
                        div()
                            .flex_1()
                            // Long messages wrap instead of running out of the box.
                            .min_w(px(0.))
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .children(self.alert.message.split('\n').map(|line| {
                                div().min_h(px(8.)).child(SharedString::from(line.to_string()))
                            })),
                    ),
            )
            .footer(div().flex().gap(px(8.)).children(buttons))
    }
}

impl ExcelApp {
    fn new_modal(&self, cx: &mut Context<Self>) -> Modal {
        Modal {
            app: cx.weak_entity(),
            focus: cx.focus_handle(),
            shared: Rc::new(ModalShared {
                offset: Cell::new(Point::default()),
                drag: Cell::new(None),
                collapsed: RefCell::new(None),
                ref_inputs: RefCell::new(Vec::new()),
            }),
        }
    }

    fn push_modal(&mut self, view: AnyView, modal: Modal, restore: Option<FocusHandle>, cx: &mut Context<Self>) {
        self.popup = None;
        self.modals.push(ModalEntry {
            view,
            modal,
            restore,
            needs_focus: true,
        });
        cx.notify();
    }

    /// Open a dialog over the window. `build` makes its view; the `Modal`
    /// it receives is what the view passes to `widgets::dialog` and uses
    /// to reach the application and to close itself.
    ///
    /// ```ignore
    /// let name = app.wb.sheets[app.sheet].name.clone();
    /// app.open_modal(window, cx, move |modal, _, cx| RenameDialog::new(modal, name, cx));
    /// ```
    ///
    /// The view cannot look at the application while it is being built
    /// (the application is busy opening it): hand it what it needs.
    pub fn open_modal<V: Render>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        build: impl FnOnce(Modal, &mut Window, &mut Context<V>) -> V,
    ) -> Entity<V> {
        let modal = self.new_modal(cx);
        let restore = window.focused(cx);
        let handle = modal.clone();
        let view = cx.new(|cx| build(handle, window, cx));
        self.push_modal(view.clone().into(), modal, restore, cx);
        view
    }

    /// Show a message box.
    pub fn alert(&mut self, mut alert: Alert, cx: &mut Context<Self>) {
        if alert.buttons.is_empty() {
            alert = alert.close_button(t("OK", "OK"));
        }
        let modal = self.new_modal(cx);
        let handle = modal.clone();
        let view = cx.new(|_| MessageBox { modal: handle, alert });
        self.push_modal(view.into(), modal, None, cx);
    }

    /// Close the dialog in front.
    pub fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.modals.pop() else {
            return;
        };
        self.pick = None;
        self.drag = None;
        // The keyboard goes back to the dialog underneath, else to the
        // cell being edited, else to the grid.
        match (self.modals.last(), entry.restore) {
            (Some(_), Some(handle)) => window.focus(&handle),
            (Some(under), None) => window.focus(&under.modal.focus),
            (None, _) if self.editing.is_some() => self.focus_editor(window, cx),
            (None, _) => window.focus(&self.grid_focus),
        }
        cx.notify();
    }

    /// Close a given dialog and everything opened over it.
    pub(crate) fn close_modal_of(&mut self, focus: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.modals.iter().position(|entry| entry.modal.focus == *focus) else {
            return;
        };
        while self.modals.len() > index {
            self.close_modal(window, cx);
        }
    }

    /// The range field of the dialog in front that has the keyboard.
    fn focused_ref_input(&self, window: &Window, cx: &App) -> Option<(Entity<TextInput>, RefStyle)> {
        let entry = self.modals.last()?;
        let fields = entry.modal.shared.ref_inputs.borrow();
        fields.iter().find_map(|(field, style)| {
            let field = field.upgrade()?;
            field.read(cx).focus_handle(cx).is_focused(window).then_some((field, *style))
        })
    }

    /// A press beside the dialog: nothing happens, unless a range field is
    /// waiting for cells and the press is on the sheet.
    fn modal_backdrop_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Whatever happens, the dialog keeps the keyboard.
        window.prevent_default();
        let Some((field, style)) = self.focused_ref_input(window, cx) else {
            return;
        };
        if self.begin_pick(field.clone(), style, ev.position, cx) {
            if let Some(entry) = self.modals.last() {
                *entry.modal.shared.collapsed.borrow_mut() = Some(field);
                App::notify(cx, entry.view.entity_id());
            }
        }
    }

    /// The cells have been picked: the dialog comes back in full.
    pub(crate) fn end_pick(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.modals.last() {
            *entry.modal.shared.collapsed.borrow_mut() = None;
            App::notify(cx, entry.view.entity_id());
        }
        cx.notify();
    }

    /// The layer of open dialogs: each one over a veil that keeps the
    /// mouse away from what is behind it.
    pub(crate) fn render_modals(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        // A dialog that just opened takes the keyboard, then hands it to
        // its first control once that control exists.
        if let Some(entry) = self.modals.last_mut() {
            if entry.needs_focus {
                entry.needs_focus = false;
                let root = entry.modal.focus.clone();
                window.focus(&root);
                cx.on_next_frame(window, move |_, window, cx| {
                    if root.is_focused(window) {
                        widgets::step_focus(&root, true, window, cx);
                    }
                });
            }
        }
        let picking = self.pick.is_some() && matches!(self.drag, Some(crate::app::Drag::Pick(_)));
        let count = self.modals.len();
        self.modals
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let front = i + 1 == count;
                div()
                    .id(("modal-layer", i))
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    // Only the veil of the dialog in front is tinted, and
                    // not while cells are being picked behind it.
                    .when(front && !picking, |d| d.bg(ca(0x000000, 0.18)))
                    .when(front, |d| {
                        d.on_mouse_down(MouseButton::Left, cx.listener(Self::modal_backdrop_down))
                    })
                    .child(entry.view.clone())
                    .into_any_element()
            })
            .collect()
    }
}

// -------------------------------------------------------------------- panes

/// A pane docked right of the grid.
pub struct PaneEntry {
    pub id: &'static str,
    pub title: SharedString,
    pub view: AnyView,
}

const PANE_MIN: f32 = 200.0;
const PANE_MAX: f32 = 640.0;

impl ExcelApp {
    /// Dock a view right of the grid and show it (replacing the pane of
    /// the same id, if any). Several panes can be open: the user switches
    /// between them with the tabs along the pane's edge.
    ///
    /// ```ignore
    /// let view = cx.new(|cx| FieldList::new(cx.weak_entity() /* of the app */, ...));
    /// app.open_pane("pivot-fields", t("PivotTable Fields", "Champs de tableau croisé dynamique"), view, cx);
    /// ```
    pub fn open_pane(&mut self, id: &'static str, title: impl Into<SharedString>, view: impl Into<AnyView>, cx: &mut Context<Self>) {
        let entry = PaneEntry {
            id,
            title: title.into(),
            view: view.into(),
        };
        match self.panes.iter().position(|p| p.id == id) {
            Some(i) => {
                self.panes[i] = entry;
                self.active_pane = i;
            }
            None => {
                self.panes.push(entry);
                self.active_pane = self.panes.len() - 1;
            }
        }
        cx.notify();
    }

    pub fn close_pane(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if let Some(i) = self.panes.iter().position(|p| p.id == id) {
            self.panes.remove(i);
            if self.active_pane >= i && self.active_pane > 0 {
                self.active_pane -= 1;
            }
            cx.notify();
        }
    }

    pub fn pane_open(&self, id: &'static str) -> bool {
        self.panes.iter().any(|p| p.id == id)
    }

    /// The pane border follows the pointer while it is dragged.
    pub(crate) fn pane_resize_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((start_x, start_w)) = self.pane_resize else {
            return;
        };
        if ev.pressed_button != Some(MouseButton::Left) {
            self.pane_resize = None;
            return;
        }
        self.pane_width = (start_w + start_x - f32::from(ev.position.x)).clamp(PANE_MIN, PANE_MAX);
        cx.notify();
    }

    /// The open panes: the one in front, its title bar, and tabs to reach
    /// the others.
    pub(crate) fn render_panes(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.panes.is_empty() {
            return None;
        }
        self.active_pane = self.active_pane.min(self.panes.len() - 1);
        let active = &self.panes[self.active_pane];
        let id = active.id;
        let tabs: Vec<AnyElement> = if self.panes.len() > 1 {
            self.panes
                .iter()
                .enumerate()
                .map(|(i, pane)| {
                    let on = i == self.active_pane;
                    div()
                        .id(("pane-tab", i))
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .max_w(px(150.))
                        .border_b_2()
                        .border_color(if on { c(theme::ACCENT) } else { gpui::transparent_black() })
                        .when(on, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(c(theme::ACCENT_DARK)))
                        .when(!on, |d| d.text_color(c(theme::TEXT_MUTED)).hover(|s| s.bg(c(theme::HOVER))))
                        .child(div().truncate().child(pane.title.clone()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.active_pane = i;
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(
            div()
                .id("pane")
                .relative()
                .h_full()
                .w(px(self.pane_width))
                .flex_none()
                .flex()
                .flex_col()
                .bg(c(theme::PANEL_BG))
                .border_l_1()
                .border_color(c(theme::BORDER_STRONG))
                .text_size(px(12.))
                // Tab walks the controls of the pane.
                .key_context(widgets::CONTEXT)
                .on_action(|_: &widgets::FocusNext, window, _| window.focus_next())
                .on_action(|_: &widgets::FocusPrev, window, _| window.focus_prev())
                .on_action(|_: &crate::text_input::Tab, window, _| window.focus_next())
                .on_action(|_: &crate::text_input::ShiftTab, window, _| window.focus_prev())
                .on_action(cx.listener(|this, _: &widgets::Dismiss, window, _| window.focus(&this.grid_focus)))
                .on_action(cx.listener(|this, _: &crate::text_input::Escape, window, _| window.focus(&this.grid_focus)))
                .child(
                    div()
                        .h(px(34.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .pl(px(12.))
                        .pr(px(6.))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(active.title.clone()),
                        )
                        .child(
                            div()
                                .id("pane-close")
                                .size(px(22.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .hover(|s| s.bg(c(theme::HOVER)))
                                .child(icon("close", 12.0, theme::TEXT_MUTED))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.close_pane(id, cx);
                                    window.focus(&this.grid_focus);
                                })),
                        ),
                )
                .when(!tabs.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .flex()
                            .px(px(4.))
                            .border_b_1()
                            .border_color(c(theme::BORDER))
                            .children(tabs),
                    )
                })
                .child(div().flex_1().min_h(px(0.)).overflow_hidden().child(active.view.clone()))
                // The left border is the handle that resizes the pane.
                .child(
                    div()
                        .id("pane-resize")
                        .absolute()
                        .left(px(-3.))
                        .top_0()
                        .w(px(6.))
                        .h_full()
                        .cursor(gpui::CursorStyle::ResizeLeftRight)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                this.pane_resize = Some((f32::from(ev.position.x), this.pane_width));
                                cx.stop_propagation();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }
}
