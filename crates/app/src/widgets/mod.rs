//! The building blocks of dialogs, panes and drop-downs.
//!
//! Widgets are plain builders (`Checkbox::new(..).on_change(..)`) placed in
//! the element tree of a view; what they need to remember between frames
//! (keyboard focus, scroll position, an open list) is kept for them under
//! their id, so the view only holds its own data:
//!
//! ```ignore
//! struct Options { modal: Modal, wrap: bool, kind: usize, name: Entity<TextInput> }
//!
//! impl Render for Options {
//!     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//!         dialog(&self.modal, t("Options", "Options"))
//!             .width(320.0)
//!             .on_ok(cx.listener(Self::ok))
//!             .child(labeled(t("Name:", "Nom :"), TextField::new(&self.name)))
//!             .child(Select::new("kind", kinds(), Some(self.kind)).on_change(cx.listener(
//!                 |this, ix: &usize, _, cx| { this.kind = *ix; cx.notify() },
//!             )))
//!             .child(Checkbox::new("wrap", t("Wrap text", "Renvoyer à la ligne"), self.wrap).on_change(cx.listener(
//!                 |this, on: &bool, _, cx| { this.wrap = *on; cx.notify() },
//!             )))
//!             .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
//!     }
//! }
//! ```
//!
//! Rules of the house:
//!
//! * every widget takes an id unique within its view;
//! * callbacks are `Fn(&T, &mut Window, &mut App)`, which is what
//!   `cx.listener(|this, value: &T, window, cx| ..)` produces;
//! * text is typed in an `Entity<TextInput>` the view creates with
//!   [`input`] (or [`number_input`]) and reads when it needs the text;
//! * Tab and Shift-Tab walk the widgets in the order they are drawn, Space
//!   activates the one with the focus, the arrows move inside lists, radio
//!   groups and tab strips;
//! * widgets that open a list (`Select`, `ComboBox`, `ColorButton`) draw it
//!   with `deferred`, so a view using them must not itself be wrapped in
//!   `deferred` (GPUI forbids nesting). Dialogs, panes and
//!   `ExcelApp::open_popup_view` are fine.

mod color;
pub mod gallery;
mod lists;
pub mod text_area;

pub use color::{ColorButton, ColorGrid};
pub use lists::{CheckItem, CheckList, ListBox, Tree, TreeNode};

use crate::grid::{Draw, Layer};
use crate::shell::Modal;
use crate::text_input::{self, Spin, TextInput};
use crate::theme::{self, c};
pub use crate::view::{icon, tooltip};
use gpui::{
    AnyElement, App, BorderStyle, Bounds, Context, DispatchPhase, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    Hsla, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    RenderOnce, ScrollHandle, ScrollWheelEvent, SharedString, TextRun, Window, actions, anchored, canvas, deferred, div,
    fill, point, prelude::*, px, quad, relative, size,
};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A widget callback: what `cx.listener(|this, value: &T, window, cx| ..)`
/// produces.
pub type Handler<T> = Rc<dyn Fn(&T, &mut Window, &mut App)>;

actions!(dialog, [FocusNext, FocusPrev, Accept, Dismiss]);

/// Key context of dialogs, panes and drop-down views.
pub const CONTEXT: &str = "Dialog";

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, c),
        KeyBinding::new("shift-tab", FocusPrev, c),
        KeyBinding::new("enter", Accept, c),
        KeyBinding::new("escape", Dismiss, c),
    ]);
}

// Metrics shared by the widgets (logical pixels).
pub const CONTROL_H: f32 = 24.0;
pub const ROW_H: f32 = 20.0;
const SCROLLBAR_W: f32 = 12.0;

/// Colours of the selected row of a list.
pub(crate) const SELECTED_BG: u32 = theme::ACCENT_LIGHT;
const FIELD_BORDER: u32 = 0xABABAB;

// ------------------------------------------------------------------ helpers

/// On / off / "the selection disagrees".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tri {
    #[default]
    Off,
    On,
    Mixed,
}

impl From<bool> for Tri {
    fn from(on: bool) -> Tri {
        if on { Tri::On } else { Tri::Off }
    }
}

/// `None` is the mixed state.
impl From<Option<bool>> for Tri {
    fn from(on: Option<bool>) -> Tri {
        match on {
            Some(true) => Tri::On,
            Some(false) => Tri::Off,
            None => Tri::Mixed,
        }
    }
}

impl Tri {
    /// `None` for the mixed state.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Tri::On => Some(true),
            Tri::Off => Some(false),
            Tri::Mixed => None,
        }
    }
}

/// The keyboard focus of a widget, kept under its id. `tab_stop`: whether
/// Tab stops on it (not when disabled).
pub(crate) fn focus_of(id: &ElementId, tab_stop: bool, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone()
        .tab_stop(tab_stop)
}

/// True for a key pressed alone (no Cmd / Ctrl / Alt).
pub(crate) fn plain(ev: &KeyDownEvent) -> bool {
    let m = &ev.keystroke.modifiers;
    !m.platform && !m.control && !m.alt
}

/// Move the focus to the next (or previous) widget inside `root`: Tab does
/// not leave a dialog.
pub fn step_focus(root: &FocusHandle, forward: bool, window: &mut Window, cx: &mut App) {
    // Other tab stops of the window (a pane behind a dialog) are skipped.
    for _ in 0..512 {
        if forward {
            window.focus_next();
        } else {
            window.focus_prev();
        }
        if root.contains_focused(window, cx) && !root.is_focused(window) {
            return;
        }
    }
}

/// An invisible element that follows the mouse anywhere in the window
/// while `active`: `on_move` gets each position with the button held,
/// `on_end` the release.
pub(crate) fn drag_tracker(
    active: bool,
    on_move: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    on_end: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            if !active {
                return;
            }
            let on_end = Rc::new(on_end);
            let ended = on_end.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                if ev.pressed_button == Some(MouseButton::Left) {
                    on_move(ev.position, window, cx);
                } else {
                    ended(window, cx);
                }
            });
            window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    on_end(window, cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// A text label.
pub fn label(text: impl Into<SharedString>) -> gpui::Div {
    div().flex_none().whitespace_nowrap().child(text.into())
}

/// A control with its label on the left.
///
/// ```ignore
/// labeled(t("Decimal places:", "Nombre de décimales :"), NumberField::new("dec", &self.decimals).width(60.0))
/// ```
pub fn labeled(text: impl Into<SharedString>, control: impl IntoElement) -> gpui::Div {
    div().flex().items_center().gap(px(8.)).child(label(text)).child(control)
}

/// A thin horizontal line.
pub fn separator() -> gpui::Div {
    div().h(px(1.)).w_full().flex_none().bg(c(theme::BORDER))
}

/// A titled group of controls: the title, a rule, then the children.
///
/// ```ignore
/// GroupBox::new(t("Text control", "Contrôle du texte"))
///     .child(Checkbox::new("wrap", t("Wrap text", "Renvoyer à la ligne automatiquement"), self.wrap))
/// ```
#[derive(IntoElement)]
pub struct GroupBox {
    title: SharedString,
    children: Vec<AnyElement>,
}

impl GroupBox {
    pub fn new(title: impl Into<SharedString>) -> GroupBox {
        GroupBox {
            title: title.into(),
            children: Vec::new(),
        }
    }
}

impl ParentElement for GroupBox {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for GroupBox {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(label(self.title).text_color(c(theme::TEXT_MUTED)))
                    .child(div().flex_1().h(px(1.)).bg(c(theme::BORDER))),
            )
            .child(div().pl(px(10.)).flex().flex_col().gap(px(6.)).children(self.children))
    }
}

// ------------------------------------------------------------- dialog frame

/// The frame of a dialog: title bar, content, row of buttons.
///
/// The frame keeps the keyboard inside the dialog (Tab / Shift-Tab), runs
/// `on_ok` on Return and `on_cancel` on Escape or the close box (by default
/// `on_cancel` closes the dialog), and lets the user drag the dialog by its
/// title bar.
///
/// ```ignore
/// dialog(&self.modal, t("Go To", "Atteindre"))
///     .width(300.0)
///     .on_ok(cx.listener(Self::ok))
///     .child(labeled(t("Reference:", "Référence :"), TextField::new(&self.reference)))
///     .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
/// ```
pub fn dialog(modal: &Modal, title: impl Into<SharedString>) -> DialogFrame {
    DialogFrame {
        modal: modal.clone(),
        title: title.into(),
        width: 420.0,
        children: Vec::new(),
        footer: None,
        on_ok: None,
        on_cancel: None,
    }
}

#[derive(IntoElement)]
pub struct DialogFrame {
    modal: Modal,
    title: SharedString,
    width: f32,
    children: Vec<AnyElement>,
    footer: Option<AnyElement>,
    on_ok: Option<Handler<()>>,
    on_cancel: Option<Handler<()>>,
}

impl DialogFrame {
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// The row of buttons at the bottom, right-aligned (see [`ok_cancel`]).
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }

    /// What Return does (the default button).
    pub fn on_ok(mut self, f: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_ok = Some(Rc::new(f));
        self
    }

    /// What Escape and the close box do instead of closing the dialog.
    pub fn on_cancel(mut self, f: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(f));
        self
    }
}

impl ParentElement for DialogFrame {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for DialogFrame {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let modal = self.modal.clone();
        let root = modal.focus.clone();
        let shared = modal.shared.clone();
        let offset = shared.offset.get();
        let view = window.current_view();
        let cancel: Handler<()> = self.on_cancel.unwrap_or_else(|| {
            let modal = modal.clone();
            Rc::new(move |_: &(), window: &mut Window, cx: &mut App| modal.close(window, cx))
        });
        let ok = self.on_ok;
        // While cells are picked for a range field, only that field shows.
        let collapsed = shared.collapsed.borrow().clone();

        let title_bar = div()
            .id("dialog-title")
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .pl(px(12.))
            .pr(px(4.))
            .bg(c(theme::CHROME_BG))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .rounded_t(px(6.))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title),
            )
            .on_mouse_down(MouseButton::Left, {
                let shared = shared.clone();
                move |ev: &MouseDownEvent, _, cx| {
                    shared.drag.set(Some((ev.position, shared.offset.get())));
                    cx.notify(view);
                }
            })
            .child(
                div()
                    .id("dialog-close")
                    .size(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(icon("close", 12.0, theme::TEXT_MUTED))
                    // Not the start of a drag.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click({
                        let cancel = cancel.clone();
                        move |_, window, cx| cancel(&(), window, cx)
                    }),
            );

        let dragging = shared.drag.get().is_some();
        let tracker = drag_tracker(
            dragging,
            {
                let shared = shared.clone();
                move |position, _, cx| {
                    if let Some((start, origin)) = shared.drag.get() {
                        shared.offset.set(origin + (position - start));
                        cx.notify(view);
                    }
                }
            },
            {
                let shared = shared.clone();
                move |_, cx| {
                    shared.drag.set(None);
                    cx.notify(view);
                }
            },
        );

        let body: AnyElement = match collapsed {
            Some(field) => div()
                .p(px(8.))
                .child(TextField::new(&field).trailing(collapse_button(&modal, &field, window)))
                .into_any_element(),
            None => div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .p(px(12.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .children(self.children),
                )
                .children(self.footer.map(|footer| {
                    div()
                        .px(px(12.))
                        .pb(px(12.))
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(footer)
                }))
                .into_any_element(),
        };

        div()
            .id("dialog")
            .relative()
            .left(offset.x)
            .top(offset.y)
            .w(px(self.width))
            .track_focus(&root)
            .key_context(CONTEXT)
            .occlude()
            .flex()
            .flex_col()
            .bg(c(theme::PANEL_BG))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(6.))
            .shadow_lg()
            .text_size(px(12.))
            .text_color(c(theme::TEXT))
            // A click on the frame itself leaves the focus where it is.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_action({
                let root = root.clone();
                move |_: &FocusNext, window, cx| step_focus(&root, true, window, cx)
            })
            .on_action({
                let root = root.clone();
                move |_: &text_input::Tab, window, cx| step_focus(&root, true, window, cx)
            })
            .on_action({
                let root = root.clone();
                move |_: &FocusPrev, window, cx| step_focus(&root, false, window, cx)
            })
            .on_action({
                let root = root.clone();
                move |_: &text_input::ShiftTab, window, cx| step_focus(&root, false, window, cx)
            })
            .on_action({
                let ok = ok.clone();
                move |_: &Accept, window, cx| {
                    if let Some(ok) = &ok {
                        ok(&(), window, cx)
                    }
                }
            })
            .on_action({
                let ok = ok.clone();
                move |_: &text_input::Enter, window, cx| {
                    if let Some(ok) = &ok {
                        ok(&(), window, cx)
                    }
                }
            })
            .on_action({
                let cancel = cancel.clone();
                move |_: &Dismiss, window, cx| cancel(&(), window, cx)
            })
            .on_action({
                let cancel = cancel.clone();
                move |_: &text_input::Escape, window, cx| cancel(&(), window, cx)
            })
            .child(title_bar)
            .child(body)
            .child(tracker)
    }
}

/// The usual pair of buttons: OK (the default) and Cancel (closes).
///
/// ```ignore
/// .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
/// ```
pub fn ok_cancel(modal: &Modal, on_ok: impl Fn(&(), &mut Window, &mut App) + 'static) -> impl IntoElement {
    let modal = modal.clone();
    div()
        .flex()
        .gap(px(8.))
        .child(Button::new("dialog-ok", crate::i18n::t("OK", "OK")).primary().on_click(on_ok))
        .child(
            Button::new("dialog-cancel", crate::i18n::t("Cancel", "Annuler"))
                .on_click(move |_, window, cx| modal.close(window, cx)),
        )
}

// ------------------------------------------------------------------- button

/// A push button. `primary()` gives it the look of the default button
/// (the one Return triggers through `DialogFrame::on_ok`).
///
/// ```ignore
/// Button::new("apply", t("Apply", "Appliquer")).on_click(cx.listener(|this, _, window, cx| this.apply(window, cx)))
/// ```
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    primary: bool,
    disabled: bool,
    width: Option<f32>,
    on_click: Option<Handler<()>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        Button {
            id: id.into(),
            label: label.into(),
            primary: false,
            disabled: false,
            width: None,
            on_click: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn on_click(mut self, f: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, !self.disabled, window, cx);
        let focused = focus.is_focused(window);
        let handler = self.on_click.filter(|_| !self.disabled);
        let (bg, fg, border) = match (self.disabled, self.primary) {
            (true, _) => (theme::CHROME_BG, theme::TEXT_DISABLED, theme::BORDER),
            (false, true) => (theme::ACCENT, 0xFFFFFF, theme::ACCENT),
            (false, false) => (0xFFFFFF, theme::TEXT, FIELD_BORDER),
        };
        let key_handler = handler.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex_none()
            .h(px(CONTROL_H))
            .min_w(px(self.width.unwrap_or(76.0)))
            .rounded(px(4.))
            .border_1()
            .border_color(c(if focused && !self.primary { theme::ACCENT } else { border }))
            .bg(c(bg))
            .text_color(c(fg))
            // Space presses the button that has the focus.
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "space" && plain(ev) {
                    if let Some(handler) = &key_handler {
                        handler(&(), window, cx);
                    }
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .id("hit")
                    .size_full()
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .whitespace_nowrap()
                    // The focus shows as a line inside the button.
                    .when(focused, |d| {
                        d.border_1().border_color(c(if self.primary { 0xFFFFFF } else { theme::ACCENT }))
                    })
                    .when(!self.disabled, |d| {
                        d.hover(|s| s.bg(c(if self.primary { theme::ACCENT_DARK } else { theme::HOVER })))
                            .active(|s| s.bg(c(if self.primary { theme::ACCENT_DARK } else { theme::PRESSED })))
                    })
                    .child(self.label)
                    .on_click(move |_, window, cx| {
                        if let Some(handler) = &handler {
                            handler(&(), window, cx);
                        }
                    }),
            )
    }
}

// ----------------------------------------------------------------- checkbox

/// A check box. `state` is a `bool`, an `Option<bool>` (`None` shows the
/// mixed state) or a [`Tri`].
///
/// ```ignore
/// Checkbox::new("locked", t("Locked", "Verrouillée"), self.locked)   // Option<bool>
///     .on_change_tri(cx.listener(|this, state: &Tri, _, cx| { this.locked = state.as_bool(); cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    state: Tri,
    disabled: bool,
    on_change: Option<Handler<bool>>,
    on_change_tri: Option<Handler<Tri>>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, state: impl Into<Tri>) -> Checkbox {
        Checkbox {
            id: id.into(),
            label: label.into(),
            state: state.into(),
            disabled: false,
            on_change: None,
            on_change_tri: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new state; a mixed box becomes checked.
    pub fn on_change(mut self, f: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// Like `on_change`, for a box that cycles checked → unchecked → mixed.
    pub fn on_change_tri(mut self, f: impl Fn(&Tri, &mut Window, &mut App) + 'static) -> Self {
        self.on_change_tri = Some(Rc::new(f));
        self
    }
}

/// The square of a check box (also used by lists and trees).
pub(crate) fn check_mark(state: Tri, disabled: bool) -> gpui::Div {
    let on = state != Tri::Off;
    let color = if disabled { theme::TEXT_DISABLED } else { theme::ACCENT };
    div()
        .size(px(14.))
        .flex_none()
        .rounded(px(3.))
        .border_1()
        .border_color(c(if on { color } else if disabled { theme::TEXT_DISABLED } else { theme::TEXT_MUTED }))
        .bg(c(if on { color } else { 0xFFFFFF }))
        .flex()
        .items_center()
        .justify_center()
        .when(state == Tri::On, |d| d.child(icon("check", 11.0, 0xFFFFFF)))
        .when(state == Tri::Mixed, |d| d.child(div().w(px(8.)).h(px(2.)).bg(c(0xFFFFFF))))
}

impl RenderOnce for Checkbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, !self.disabled, window, cx);
        let focused = focus.is_focused(window);
        let state = self.state;
        let (on_change, on_change_tri, disabled) = (self.on_change, self.on_change_tri, self.disabled);
        let toggle: Rc<dyn Fn(&mut Window, &mut App)> = Rc::new(move |window, cx| {
            if disabled {
                return;
            }
            if let Some(f) = &on_change_tri {
                let next = match state {
                    Tri::On => Tri::Off,
                    Tri::Off => Tri::Mixed,
                    Tri::Mixed => Tri::On,
                };
                f(&next, window, cx);
            }
            if let Some(f) = &on_change {
                f(&(state != Tri::On), window, cx);
            }
        });
        let key_toggle = toggle.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex_none()
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "space" && plain(ev) {
                    key_toggle(window, cx);
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .id("hit")
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(check_mark(state, disabled))
                    .child(
                        div()
                            .px(px(2.))
                            .border_1()
                            .border_color(if focused { c(theme::ACCENT) } else { gpui::transparent_black() })
                            .rounded(px(2.))
                            .whitespace_nowrap()
                            .when(disabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
                            .child(self.label),
                    )
                    .on_click(move |_, window, cx| toggle(window, cx)),
            )
    }
}

// -------------------------------------------------------------- radio group

/// Mutually exclusive options. The group is one Tab stop; the arrows move
/// the choice.
///
/// ```ignore
/// RadioGroup::new("shift", vec![t("Shift cells right", "Décaler les cellules vers la droite").into(),
///                               t("Shift cells down", "Décaler les cellules vers le bas").into()], Some(self.shift))
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.shift = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct RadioGroup {
    id: ElementId,
    options: Vec<SharedString>,
    selected: Option<usize>,
    horizontal: bool,
    disabled: bool,
    on_change: Option<Handler<usize>>,
}

impl RadioGroup {
    pub fn new(id: impl Into<ElementId>, options: Vec<SharedString>, selected: Option<usize>) -> RadioGroup {
        RadioGroup {
            id: id.into(),
            options,
            selected,
            horizontal: false,
            disabled: false,
            on_change: None,
        }
    }

    /// Lay the options out in a row.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for RadioGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, !self.disabled, window, cx);
        let focused = focus.is_focused(window);
        let count = self.options.len();
        let selected = self.selected;
        let handler = self.on_change.filter(|_| !self.disabled);
        let key_handler = handler.clone();
        let disabled = self.disabled;
        let mut items: Vec<AnyElement> = Vec::new();
        for (i, option) in self.options.into_iter().enumerate() {
            let on = selected == Some(i);
            let handler = handler.clone();
            let color = if disabled { theme::TEXT_DISABLED } else { theme::ACCENT };
            items.push(
                div()
                    .id(("radio", i))
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_none()
                            .rounded_full()
                            .border_1()
                            .border_color(c(if on { color } else { theme::TEXT_MUTED }))
                            .bg(c(0xFFFFFF))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(on, |d| d.child(div().size(px(8.)).rounded_full().bg(c(color)))),
                    )
                    .child(
                        div()
                            .px(px(2.))
                            .border_1()
                            .border_color(if focused && (on || (selected.is_none() && i == 0)) {
                                c(theme::ACCENT)
                            } else {
                                gpui::transparent_black()
                            })
                            .rounded(px(2.))
                            .whitespace_nowrap()
                            .when(disabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
                            .child(option),
                    )
                    .on_click(move |_, window, cx| {
                        if let Some(handler) = &handler {
                            handler(&i, window, cx);
                        }
                    })
                    .into_any_element(),
            );
        }
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex()
            .when(!self.horizontal, |d| d.flex_col().gap(px(2.)))
            .when(self.horizontal, |d| d.gap(px(16.)))
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                let Some(handler) = &key_handler else {
                    return;
                };
                if count == 0 || !plain(ev) {
                    return;
                }
                let next = match ev.keystroke.key.as_str() {
                    "down" | "right" => Some(selected.map(|s| (s + 1) % count).unwrap_or(0)),
                    "up" | "left" => Some(selected.map(|s| (s + count - 1) % count).unwrap_or(count - 1)),
                    "space" if selected.is_none() => Some(0),
                    _ => None,
                };
                if let Some(next) = next {
                    handler(&next, window, cx);
                    cx.stop_propagation();
                }
            })
            .children(items)
    }
}

// ---------------------------------------------------------------- tab strip

/// The tabs along the top of a dialog. One Tab stop; Left / Right switch.
///
/// ```ignore
/// TabStrip::new("tabs", vec![t("Number", "Nombre").into(), t("Font", "Police").into()], self.tab)
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.tab = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct TabStrip {
    id: ElementId,
    tabs: Vec<SharedString>,
    selected: usize,
    on_change: Option<Handler<usize>>,
}

impl TabStrip {
    pub fn new(id: impl Into<ElementId>, tabs: Vec<SharedString>, selected: usize) -> TabStrip {
        TabStrip {
            id: id.into(),
            tabs,
            selected,
            on_change: None,
        }
    }

    pub fn on_change(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for TabStrip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let count = self.tabs.len();
        let selected = self.selected;
        let handler = self.on_change;
        let key_handler = handler.clone();
        let mut tabs: Vec<AnyElement> = Vec::new();
        for (i, name) in self.tabs.into_iter().enumerate() {
            let on = i == selected;
            let handler = handler.clone();
            tabs.push(
                div()
                    .id(("tab", i))
                    .relative()
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded_t(px(4.))
                    .whitespace_nowrap()
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD).text_color(c(theme::ACCENT_DARK)))
                    .when(!on, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                    .child(
                        div()
                            .px(px(2.))
                            .border_1()
                            .border_color(if on && focused { c(theme::ACCENT) } else { gpui::transparent_black() })
                            .rounded(px(2.))
                            .child(name),
                    )
                    .when(on, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left(px(10.))
                                .right(px(10.))
                                .h(px(2.))
                                .bg(c(theme::ACCENT)),
                        )
                    })
                    .on_click(move |_, window, cx| {
                        if let Some(handler) = &handler {
                            handler(&i, window, cx);
                        }
                    })
                    .into_any_element(),
            );
        }
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex()
            .flex_none()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                let Some(handler) = &key_handler else {
                    return;
                };
                if count == 0 || !plain(ev) {
                    return;
                }
                let next = match ev.keystroke.key.as_str() {
                    "right" => (selected + 1) % count,
                    "left" => (selected + count - 1) % count,
                    _ => return,
                };
                handler(&next, window, cx);
                cx.stop_propagation();
            })
            .children(tabs)
    }
}

// -------------------------------------------------------------- text fields

/// A text field for a view: created once (in the view's constructor), read
/// with `field.read(cx).text()`. The view is redrawn whenever the text
/// changes.
///
/// ```ignore
/// let name = widgets::input(cx, "Table1");
/// ```
pub fn input<V: 'static>(cx: &mut Context<V>, text: impl Into<String>) -> Entity<TextInput> {
    let text = text.into();
    let field = cx.new(|cx| {
        let mut field = TextInput::field(cx);
        field.set_text(text, cx);
        field
    });
    cx.observe(&field, |_, _, cx| cx.notify()).detach();
    field
}

/// A numeric field for [`NumberField`]: Up / Down and the spinner step it
/// by `step` between `min` and `max`. Read it with
/// `field.read(cx).number()` (`None` when it is empty or not a number).
///
/// ```ignore
/// let decimals = widgets::number_input(cx, Some(2.0), 0.0, 30.0, 1.0);
/// ```
pub fn number_input<V: 'static>(cx: &mut Context<V>, value: Option<f64>, min: f64, max: f64, step: f64) -> Entity<TextInput> {
    let field = input(cx, "");
    let decimal_sep = crate::i18n::lang().locale().decimal_sep;
    field.update(cx, |field, cx| {
        field.spin = Some(Spin {
            min,
            max,
            step,
            decimal_sep,
        });
        if let Some(value) = value {
            field.set_text(number_text(value, decimal_sep), cx);
        }
    });
    field
}

/// A number the way a numeric field shows it.
pub fn number_text(value: f64, decimal_sep: char) -> String {
    let mut text = format!("{:.6}", value);
    while text.contains('.') && (text.ends_with('0') || text.ends_with('.')) {
        text.pop();
    }
    text.replace('.', &decimal_sep.to_string())
}

/// The box around a text field.
///
/// ```ignore
/// TextField::new(&self.name).width(180.0)
/// ```
#[derive(IntoElement)]
pub struct TextField {
    input: Entity<TextInput>,
    width: Option<f32>,
    trailing: Option<AnyElement>,
}

impl TextField {
    pub fn new(input: &Entity<TextInput>) -> TextField {
        TextField {
            input: input.clone(),
            width: None,
            trailing: None,
        }
    }

    /// Fixed width; without it the field takes the room left in its row.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Something shown inside the box, after the text (a button).
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = self.input.read(cx).focus_handle(cx);
        let focused = handle.is_focused(window);
        let input = self.input.clone();
        div()
            .h(px(CONTROL_H))
            .when_some(self.width, |d, w| d.w(px(w)).flex_none())
            .when(self.width.is_none(), |d| d.flex_1().min_w(px(40.)))
            .flex()
            .items_center()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
            .rounded(px(3.))
            .pl(px(6.))
            .pr(px(if self.trailing.is_some() { 1.0 } else { 6.0 }))
            .line_height(px(18.))
            .overflow_hidden()
            // A click beside the text still lands in the field.
            .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, window, cx| {
                window.focus(&handle);
                input.update(cx, |input, cx| input.caret_to_point(ev.position, cx));
            })
            .child(div().flex_1().min_w(px(0.)).overflow_hidden().child(self.input))
            .children(self.trailing)
    }
}

/// A numeric field with a spinner, around a field made by [`number_input`].
///
/// ```ignore
/// NumberField::new("indent", &self.indent).width(56.0)
/// ```
#[derive(IntoElement)]
pub struct NumberField {
    id: ElementId,
    input: Entity<TextInput>,
    width: f32,
}

impl NumberField {
    pub fn new(id: impl Into<ElementId>, input: &Entity<TextInput>) -> NumberField {
        NumberField {
            id: id.into(),
            input: input.clone(),
            width: 64.0,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
}

impl RenderOnce for NumberField {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let arrow = |name: &'static str, icon_name: &'static str, steps: f64, input: Entity<TextInput>| {
            div()
                .id(name)
                .w(px(14.))
                .h(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(c(theme::HOVER)))
                .active(|s| s.bg(c(theme::PRESSED)))
                .child(icon(icon_name, 8.0, theme::TEXT_MUTED))
                // Stepping does not move the caret into the padding.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, _, cx| input.update(cx, |input, cx| input.step(steps, cx)))
        };
        let spinner = div()
            .id(self.id)
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(c(theme::BORDER))
            .child(arrow("up", "chevron-up", 1.0, self.input.clone()))
            .child(arrow("down", "chevron-down", -1.0, self.input.clone()));
        TextField::new(&self.input).width(self.width).trailing(spinner)
    }
}

// ------------------------------------------------------------ range field

/// A field for a cell reference, around a field made by
/// `Modal::ref_input`. While it has the keyboard, selecting cells on the
/// sheet behind the dialog writes their reference into it, and the dialog
/// shrinks to this field during the selection (and when its button is
/// pressed), as Excel's does.
///
/// ```ignore
/// // in the constructor
/// let source = modal.ref_input(RefStyle::absolute(), "=$A$1:$C$9", cx);
/// // in render
/// labeled(t("Table/Range:", "Tableau/Plage :"), RefEdit::new(&self.source, &self.modal))
/// ```
#[derive(IntoElement)]
pub struct RefEdit {
    input: Entity<TextInput>,
    modal: Modal,
    width: Option<f32>,
}

impl RefEdit {
    pub fn new(input: &Entity<TextInput>, modal: &Modal) -> RefEdit {
        RefEdit {
            input: input.clone(),
            modal: modal.clone(),
            width: None,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

/// The button that shrinks a dialog to a range field and back.
fn collapse_button(modal: &Modal, field: &Entity<TextInput>, window: &Window) -> impl IntoElement + use<> {
    let view = window.current_view();
    let shared = modal.shared.clone();
    let collapsed = shared.collapsed.borrow().is_some();
    let field = field.clone();
    div()
        .id(("ref-collapse", field.entity_id()))
        .w(px(20.))
        .h(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .hover(|s| s.bg(c(theme::HOVER)))
        .child(icon(if collapsed { "chevron-down" } else { "chevron-up" }, 10.0, theme::ACCENT))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            let mut collapsed = shared.collapsed.borrow_mut();
            *collapsed = match collapsed.take() {
                Some(_) => None,
                None => Some(field.clone()),
            };
            // Typing and picking go on in the field.
            window.focus(&field.read(cx).focus_handle(cx));
            cx.notify(view);
        })
}

impl RenderOnce for RefEdit {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let mut text = TextField::new(&self.input).trailing(collapse_button(&self.modal, &self.input, window));
        if let Some(width) = self.width {
            text = text.width(width);
        }
        text
    }
}

// -------------------------------------------------------------- scroll area

/// What a scrolling list remembers between frames.
pub(crate) struct ScrollState {
    /// Pixels scrolled off the top.
    pub top: f32,
    /// Thumb being dragged: pointer y and `top` at the start.
    pub drag: Option<(f32, f32)>,
}

impl ScrollState {
    pub fn new() -> ScrollState {
        ScrollState { top: 0.0, drag: None }
    }

    /// Scroll as little as needed to show the rows `first..=last`.
    pub fn reveal(&mut self, first: usize, last: usize, row_h: f32, height: f32) {
        let (y0, y1) = (first as f32 * row_h, (last + 1) as f32 * row_h);
        if y0 < self.top {
            self.top = y0;
        } else if y1 > self.top + height {
            self.top = y1 - height;
        }
    }
}

/// A list of `count` rows of equal height of which only the visible ones
/// are built, with a scrollbar. `row` builds one row.
pub(crate) fn virtual_rows(
    state: &Entity<ScrollState>,
    count: usize,
    row_h: f32,
    height: f32,
    cx: &mut App,
    mut row: impl FnMut(usize) -> AnyElement,
) -> gpui::Div {
    let content = count as f32 * row_h;
    let max_top = (content - height).max(0.0);
    let top = state.read(cx).top.clamp(0.0, max_top);
    let dragging = state.read(cx).drag;
    let first = (top / row_h).floor() as usize;
    let last = (((top + height) / row_h).ceil() as usize).min(count);
    let scrollable = content > height + 0.5;
    let mut rows: Vec<AnyElement> = Vec::with_capacity(last.saturating_sub(first));
    for i in first..last {
        rows.push(
            div()
                .absolute()
                .left_0()
                .right(px(if scrollable { SCROLLBAR_W } else { 0.0 }))
                .top(px(i as f32 * row_h - top))
                .h(px(row_h))
                .child(row(i))
                .into_any_element(),
        );
    }
    let mut list = div().relative().h(px(height)).overflow_hidden().children(rows).on_scroll_wheel({
        let state = state.clone();
        move |ev: &ScrollWheelEvent, _, cx| {
            let delta = f32::from(ev.delta.pixel_delta(px(row_h)).y);
            state.update(cx, |s, cx| {
                s.top = (s.top - delta).clamp(0.0, max_top);
                cx.notify();
            });
            cx.stop_propagation();
        }
    });
    if scrollable {
        let thumb_h = (height / content * height).max(20.0);
        let thumb_y = if max_top > 0.0 { top / max_top * (height - thumb_h) } else { 0.0 };
        let grab = state.clone();
        let follow = state.clone();
        let release = state.clone();
        // A press on the track above or below the thumb pages that way.
        let page = |name: &'static str, y0: f32, y1: f32, by: f32| {
            let state = state.clone();
            div()
                .id(name)
                .absolute()
                .left_0()
                .right_0()
                .top(px(y0))
                .h(px((y1 - y0).max(0.0)))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    state.update(cx, |s, cx| {
                        s.top = (s.top + by).clamp(0.0, max_top);
                        cx.notify();
                    });
                    cx.stop_propagation();
                })
        };
        list = list
            .child(
                div()
                    .id("scroll-track")
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(px(SCROLLBAR_W))
                    .h_full()
                    .bg(c(0xF7F7F7))
                    .border_l_1()
                    .border_color(c(theme::BORDER))
                    .child(page("page-up", 0.0, thumb_y, -height * 0.9))
                    .child(page("page-down", thumb_y + thumb_h, height, height * 0.9))
                    .child(
                        div()
                            .id("scroll-thumb")
                            .absolute()
                            .left(px(2.))
                            .right(px(2.))
                            .top(px(thumb_y))
                            .h(px(thumb_h))
                            .rounded(px(3.))
                            .bg(c(if dragging.is_some() { 0x8A8A8A } else { 0xC1C1C1 }))
                            .hover(|s| s.bg(c(0xA8A8A8)))
                            .on_mouse_down(MouseButton::Left, move |ev: &MouseDownEvent, _, cx| {
                                grab.update(cx, |s, cx| {
                                    s.drag = Some((f32::from(ev.position.y), s.top));
                                    cx.notify();
                                });
                                cx.stop_propagation();
                            }),
                    ),
            )
            .child(drag_tracker(
                dragging.is_some(),
                move |position, _, cx| {
                    follow.update(cx, |s, cx| {
                        if let Some((start_y, start_top)) = s.drag {
                            let travel = (height - thumb_h).max(1.0);
                            s.top = (start_top + (f32::from(position.y) - start_y) / travel * max_top).clamp(0.0, max_top);
                            cx.notify();
                        }
                    });
                },
                move |_, cx| {
                    release.update(cx, |s, cx| {
                        s.drag = None;
                        cx.notify();
                    });
                },
            ));
    }
    list
}

/// A box that scrolls its content vertically when it is taller than
/// `height`, with a scrollbar.
///
/// ```ignore
/// ScrollArea::new("rules", 180.0).child(rule_rows)
/// ```
#[derive(IntoElement)]
pub struct ScrollArea {
    id: ElementId,
    height: f32,
    children: Vec<AnyElement>,
}

impl ScrollArea {
    pub fn new(id: impl Into<ElementId>, height: f32) -> ScrollArea {
        ScrollArea {
            id: id.into(),
            height,
            children: Vec::new(),
        }
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let height = self.height;
        let thumb = handle.clone();
        div()
            .relative()
            .h(px(height))
            .child(
                div()
                    .id(self.id)
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&handle)
                    .children(self.children),
            )
            // The thumb is painted from the handle once the content has
            // been laid out.
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds: Bounds<Pixels>, _, window, _| {
                        let max = f32::from(thumb.max_offset().height);
                        if max <= 0.5 {
                            return;
                        }
                        let offset = -f32::from(thumb.offset().y);
                        let thumb_h = (height / (height + max) * height).max(20.0);
                        let y = offset / max * (height - thumb_h);
                        let origin = point(bounds.right() - px(8.), bounds.top() + px(y));
                        window.paint_quad(fill(Bounds::new(origin, size(px(6.), px(thumb_h))), c(0xC1C1C1)).corner_radii(px(3.)));
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

// ------------------------------------------------------------------- select

/// What an open list (of a `Select` or a `ComboBox`) remembers.
pub(crate) struct DropState {
    pub open: bool,
    /// Row under the keyboard cursor while open.
    pub cursor: usize,
    /// When a press outside closed the list: the click that follows on the
    /// control itself must not reopen it.
    pub closed_at: Option<Instant>,
    /// Letters typed to jump to an option, and when.
    pub typed: String,
    pub typed_at: Option<Instant>,
}

impl DropState {
    pub fn new() -> DropState {
        DropState {
            open: false,
            cursor: 0,
            closed_at: None,
            typed: String::new(),
            typed_at: None,
        }
    }

    pub fn just_closed(&self) -> bool {
        self.closed_at.is_some_and(|at| at.elapsed() < Duration::from_millis(250))
    }

    /// The option reached by typing `ch` (type-to-select), starting after
    /// `from`.
    pub fn type_to_select(&mut self, ch: &str, options: &[SharedString], from: Option<usize>) -> Option<usize> {
        let fresh = self.typed_at.is_none_or(|at| at.elapsed() > Duration::from_millis(800));
        if fresh {
            self.typed.clear();
        }
        self.typed.push_str(&ch.to_lowercase());
        self.typed_at = Some(Instant::now());
        let n = options.len();
        if n == 0 {
            return None;
        }
        // One letter cycles through the options starting with it.
        let start = match (from, self.typed.chars().count()) {
            (Some(i), 1) => i + 1,
            (Some(i), _) => i,
            (None, _) => 0,
        };
        (0..n)
            .map(|k| (start + k) % n)
            .find(|i| options[*i].to_lowercase().starts_with(&self.typed))
    }
}

/// The floating list of a drop-down control, hung under it.
pub(crate) fn drop_panel(
    width: f32,
    scroll: &Entity<ScrollState>,
    options: &Rc<Vec<SharedString>>,
    selected: Option<usize>,
    cursor: usize,
    cx: &mut App,
    on_pick: Rc<dyn Fn(usize, &mut Window, &mut App)>,
    on_dismiss: Rc<dyn Fn(&mut Window, &mut App)>,
) -> impl IntoElement {
    let count = options.len();
    let height = (count as f32 * ROW_H).min(ROW_H * 12.0);
    let options = options.clone();
    let rows = virtual_rows(scroll, count, ROW_H, height, cx, |i| {
        let on_pick = on_pick.clone();
        div()
            .id(("option", i))
            .size_full()
            .px(px(6.))
            .flex()
            .items_center()
            .whitespace_nowrap()
            .when(i == cursor, |d| d.bg(c(SELECTED_BG)))
            .when(i != cursor, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .when(selected == Some(i), |d| d.font_weight(FontWeight::SEMIBOLD))
            .child(options[i].clone())
            .on_click(move |_, window, cx| on_pick(i, window, cx))
            .into_any_element()
    });
    div().absolute().left_0().top(relative(1.)).child(
        deferred(
            anchored().snap_to_window_with_margin(px(6.)).child(
                div()
                    .id("drop-panel")
                    .occlude()
                    .mt(px(1.))
                    .w(px(width))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .rounded(px(3.))
                    .shadow_md()
                    .text_size(px(12.))
                    .text_color(c(theme::TEXT))
                    .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx))
                    .child(rows),
            ),
        )
        .with_priority(3),
    )
}

/// A drop-down list of fixed choices.
///
/// Closed: Up / Down change the choice, letters jump to an option, Space
/// opens the list. Open: Up / Down move, Return picks, Escape closes.
///
/// ```ignore
/// Select::new("underline", vec![t("None", "Aucun").into(), t("Single", "Simple").into()], Some(self.underline))
///     .width(160.0)
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.underline = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct Select {
    id: ElementId,
    options: Vec<SharedString>,
    selected: Option<usize>,
    width: f32,
    disabled: bool,
    on_change: Option<Handler<usize>>,
}

impl Select {
    /// `selected`: `None` shows an empty box (mixed selection).
    pub fn new(id: impl Into<ElementId>, options: Vec<SharedString>, selected: Option<usize>) -> Select {
        Select {
            id: id.into(),
            options,
            selected,
            width: 160.0,
            disabled: false,
            on_change: None,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = focus_of(&self.id, !self.disabled, window, cx);
        let focused = focus.is_focused(window);
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| DropState::new());
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollState::new());
        let options = Rc::new(self.options);
        let count = options.len();
        let selected = self.selected.filter(|s| *s < count);
        let open = state.read(cx).open && !self.disabled;
        let cursor = state.read(cx).cursor.min(count.saturating_sub(1));
        let handler = self.on_change.filter(|_| !self.disabled);

        let close = {
            let state = state.clone();
            move |by_outside_press: bool, cx: &mut App| {
                state.update(cx, |s, cx| {
                    s.open = false;
                    s.closed_at = by_outside_press.then(Instant::now);
                    cx.notify();
                });
            }
        };
        let pick: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let handler = handler.clone();
            let close = close.clone();
            Rc::new(move |i, window, cx| {
                close(false, cx);
                if let Some(handler) = &handler {
                    handler(&i, window, cx);
                }
            })
        };
        let toggle = {
            let state = state.clone();
            let scroll = scroll.clone();
            move |cx: &mut App| {
                state.update(cx, |s, cx| {
                    if s.just_closed() {
                        s.closed_at = None;
                        return;
                    }
                    s.open = !s.open;
                    s.cursor = selected.unwrap_or(0);
                    cx.notify();
                });
                let cursor = selected.unwrap_or(0);
                scroll.update(cx, |s, _| s.reveal(cursor, cursor, ROW_H, ROW_H * 12.0));
            }
        };

        let panel = open.then(|| {
            let dismiss = close.clone();
            drop_panel(
                self.width,
                &scroll,
                &options,
                selected,
                cursor,
                cx,
                pick.clone(),
                Rc::new(move |_, cx| dismiss(true, cx)),
            )
        });

        let key_state = state.clone();
        let key_scroll = scroll.clone();
        let key_options = options.clone();
        let key_pick = pick.clone();
        let key_toggle = toggle.clone();
        let key_handler = handler.clone();
        let accept_pick = pick.clone();
        let dismiss_close = close.clone();
        let click_toggle = toggle.clone();
        div()
            .id(self.id)
            .track_focus(&focus)
            .relative()
            .flex_none()
            .w(px(self.width))
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if count == 0 || !plain(ev) {
                    return;
                }
                let key = ev.keystroke.key.as_str();
                let step = |from: Option<usize>, down: bool| match (from, down) {
                    (None, _) => 0,
                    (Some(i), true) => (i + 1).min(count - 1),
                    (Some(i), false) => i.saturating_sub(1),
                };
                match key {
                    "up" | "down" if open => {
                        let next = step(Some(cursor), key == "down");
                        key_state.update(cx, |s, cx| {
                            s.cursor = next;
                            cx.notify();
                        });
                        key_scroll.update(cx, |s, _| s.reveal(next, next, ROW_H, ROW_H * 12.0));
                    }
                    "up" | "down" => {
                        let next = step(selected, key == "down");
                        if Some(next) != selected {
                            if let Some(handler) = &key_handler {
                                handler(&next, window, cx);
                            }
                        }
                    }
                    "space" if open => key_pick(cursor, window, cx),
                    "space" => key_toggle(cx),
                    _ => {
                        let Some(ch) = ev.keystroke.key_char.as_deref().filter(|ch| !ch.trim().is_empty()) else {
                            return;
                        };
                        let from = if open { Some(cursor) } else { selected };
                        let found = key_state.update(cx, |s, _| s.type_to_select(ch, &key_options, from));
                        match found {
                            Some(i) if open => {
                                key_state.update(cx, |s, cx| {
                                    s.cursor = i;
                                    cx.notify();
                                });
                                key_scroll.update(cx, |s, _| s.reveal(i, i, ROW_H, ROW_H * 12.0));
                            }
                            Some(i) => {
                                if let Some(handler) = &key_handler {
                                    handler(&i, window, cx);
                                }
                            }
                            None => {}
                        }
                    }
                }
                cx.stop_propagation();
            })
            // Return and Escape belong to the open list, else to the dialog.
            .on_action(move |_: &Accept, window, cx| {
                if open {
                    accept_pick(cursor, window, cx);
                } else {
                    cx.propagate();
                }
            })
            .on_action(move |_: &Dismiss, _, cx| {
                if open {
                    dismiss_close(false, cx);
                } else {
                    cx.propagate();
                }
            })
            .child(
                div()
                    .id("hit")
                    .h(px(CONTROL_H))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.))
                    .pl(px(6.))
                    .pr(px(4.))
                    .bg(c(if self.disabled { theme::CHROME_BG } else { 0xFFFFFF }))
                    .border_1()
                    .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
                    .rounded(px(3.))
                    .when(self.disabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(selected.map(|i| options[i].clone()).unwrap_or_default()),
                    )
                    .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
                    .on_click(move |_, _, cx| click_toggle(cx)),
            )
            .children(panel)
    }
}

/// A text field with a list of suggestions (an editable drop-down). The
/// text lives in a field made by [`input`]; picking a suggestion writes it
/// into the field and calls `on_pick`.
///
/// ```ignore
/// ComboBox::new("size", &self.size, sizes).width(64.0)
///     .on_pick(cx.listener(|this, ix: &usize, _, cx| this.size_picked(*ix, cx)))
/// ```
#[derive(IntoElement)]
pub struct ComboBox {
    id: ElementId,
    input: Entity<TextInput>,
    options: Vec<SharedString>,
    width: f32,
    on_pick: Option<Handler<usize>>,
}

impl ComboBox {
    pub fn new(id: impl Into<ElementId>, input: &Entity<TextInput>, options: Vec<SharedString>) -> ComboBox {
        ComboBox {
            id: id.into(),
            input: input.clone(),
            options,
            width: 160.0,
            on_pick: None,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn on_pick(mut self, f: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_pick = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for ComboBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| DropState::new());
        let scroll = window.use_keyed_state(self.id.clone(), cx, |_, _| ScrollState::new());
        let options = Rc::new(self.options);
        let count = options.len();
        let open = state.read(cx).open;
        let cursor = state.read(cx).cursor.min(count.saturating_sub(1));
        let text = self.input.read(cx).text().to_string();
        let current = options.iter().position(|o| o.eq_ignore_ascii_case(&text));

        let close = {
            let state = state.clone();
            move |by_outside_press: bool, cx: &mut App| {
                state.update(cx, |s, cx| {
                    s.open = false;
                    s.closed_at = by_outside_press.then(Instant::now);
                    cx.notify();
                });
            }
        };
        let pick: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let handler = self.on_pick.clone();
            let close = close.clone();
            let input = self.input.clone();
            let options = options.clone();
            Rc::new(move |i, window, cx| {
                close(false, cx);
                let text = options[i].to_string();
                input.update(cx, |input, cx| {
                    input.set_text(text, cx);
                    input.select_all_text(cx);
                });
                if let Some(handler) = &handler {
                    handler(&i, window, cx);
                }
            })
        };
        let panel = open.then(|| {
            let dismiss = close.clone();
            drop_panel(
                self.width,
                &scroll,
                &options,
                current,
                cursor,
                cx,
                pick.clone(),
                Rc::new(move |_, cx| dismiss(true, cx)),
            )
        });
        let toggle_state = state.clone();
        let toggle_scroll = scroll.clone();
        let button = div()
            .id("combo-button")
            .w(px(18.))
            .h(px(20.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(2.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                toggle_state.update(cx, |s, cx| {
                    if s.just_closed() {
                        s.closed_at = None;
                        return;
                    }
                    s.open = !s.open;
                    s.cursor = current.unwrap_or(0);
                    cx.notify();
                });
                let cursor = current.unwrap_or(0);
                toggle_scroll.update(cx, |s, _| s.reveal(cursor, cursor, ROW_H, ROW_H * 12.0));
            });

        // The arrows reach us once the field has reported them.
        let arrow = {
            let state = state.clone();
            let scroll = scroll.clone();
            let pick = pick.clone();
            Rc::new(move |down: bool, window: &mut Window, cx: &mut App| {
                if count == 0 {
                    return;
                }
                if open {
                    let next = if down { (cursor + 1).min(count - 1) } else { cursor.saturating_sub(1) };
                    state.update(cx, |s, cx| {
                        s.cursor = next;
                        cx.notify();
                    });
                    scroll.update(cx, |s, _| s.reveal(next, next, ROW_H, ROW_H * 12.0));
                } else {
                    let next = match (current, down) {
                        (None, _) => 0,
                        (Some(i), true) => (i + 1).min(count - 1),
                        (Some(i), false) => i.saturating_sub(1),
                    };
                    pick(next, window, cx);
                }
            })
        };
        let (up, down) = (arrow.clone(), arrow);
        let accept_pick = pick.clone();
        let dismiss_close = close.clone();
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(px(self.width))
            .on_action(move |_: &text_input::Up, window, cx| up(false, window, cx))
            .on_action(move |_: &text_input::Down, window, cx| down(true, window, cx))
            .on_action(move |_: &text_input::Enter, window, cx| {
                if open {
                    accept_pick(cursor, window, cx);
                } else {
                    cx.propagate();
                }
            })
            .on_action(move |_: &text_input::Escape, _, cx| {
                if open {
                    dismiss_close(false, cx);
                } else {
                    cx.propagate();
                }
            })
            .child(TextField::new(&self.input).width(self.width).trailing(button))
            .children(panel)
    }
}

// ------------------------------------------------------------------ preview

/// Drawing commands for a [`preview`], in the coordinates of the preview
/// box (origin at its top-left corner).
pub struct Sketch<'a> {
    pub width: f32,
    pub height: f32,
    origin: Point<Pixels>,
    window: &'a mut Window,
    items: Vec<Draw>,
}

impl Sketch<'_> {
    fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds<Pixels> {
        Bounds::new(
            point(self.origin.x + px(x0.min(x1)), self.origin.y + px(y0.min(y1))),
            size(px((x1 - x0).abs()), px((y1 - y0).abs())),
        )
    }

    pub fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) {
        self.items.push(Draw::Quad(fill(self.bounds(x0, y0, x1, y1), color)));
    }

    pub fn frame(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Hsla) {
        self.items.push(Draw::Quad(quad(
            self.bounds(x0, y0, x1, y1),
            px(0.),
            gpui::transparent_black(),
            px(width),
            color,
            BorderStyle::Solid,
        )));
    }

    /// A straight line of any direction.
    pub fn line(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: Hsla) {
        let mut builder = gpui::PathBuilder::stroke(px(width));
        builder.move_to(point(self.origin.x + px(from.0), self.origin.y + px(from.1)));
        builder.line_to(point(self.origin.x + px(to.0), self.origin.y + px(to.1)));
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    /// Width of a text in a font.
    pub fn measure(&self, text: &str, font: &gpui::Font, size: f32) -> f32 {
        f32::from(self.shape(text, font, size, gpui::black(), false, false).width)
    }

    fn shape(&self, text: &str, font: &gpui::Font, size: f32, color: Hsla, underline: bool, strike: bool) -> gpui::ShapedLine {
        let text: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: underline.then(|| gpui::UnderlineStyle {
                color: Some(color),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: strike.then(|| gpui::StrikethroughStyle {
                color: Some(color),
                thickness: px(1.0),
            }),
        };
        self.window.text_system().shape_line(text, px(size), &[run], None)
    }

    /// Draw text with its top-left corner at (x, y), clipped to the box.
    pub fn text(&mut self, text: &str, x: f32, y: f32, font: &gpui::Font, size: f32, color: Hsla) {
        self.styled_text(text, x, y, font, size, color, false, false);
    }

    pub fn styled_text(&mut self, text: &str, x: f32, y: f32, font: &gpui::Font, size: f32, color: Hsla, underline: bool, strike: bool) {
        let line = self.shape(text, font, size, color, underline, strike);
        let clip = self.bounds(0.0, 0.0, self.width, self.height);
        self.items.push(Draw::Text {
            line,
            origin: point(self.origin.x + px(x.round()), self.origin.y + px(y.round())),
            line_height: px((size * 1.25).round()),
            clip,
        });
    }
}

/// A small canvas for the preview of a dialog (a font sample, a border
/// diagram). `draw` runs at every repaint.
///
/// ```ignore
/// preview(120.0, 60.0, move |s| {
///     s.rect(0.0, 0.0, s.width, s.height, c(0xFFFFFF));
///     s.text("AaBbCcYyZz", 8.0, 20.0, &font, 14.0, c(0x000000));
/// })
/// ```
pub fn preview(width: f32, height: f32, draw: impl Fn(&mut Sketch) + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let mut sketch = Sketch {
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
                origin: bounds.origin,
                window,
                items: Vec::new(),
            };
            draw(&mut sketch);
            let items = sketch.items;
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// The interface font, for previews.
pub fn ui_font() -> gpui::Font {
    crate::features::ui_font(false)
}
