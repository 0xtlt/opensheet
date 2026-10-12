//! The card of a comment thread: who said what and when, the replies, the
//! box to answer in. The same card floats beside its cell on the grid and
//! lines up with the others in the Comments pane.

use super::text_area::{AreaEvent, TextArea};
use super::user_name;
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::theme::{self, c};
use crate::widgets::{Button, icon, tooltip};
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Element, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    GlobalElementId,
    IntoElement, LayoutId, MouseButton, MouseDownEvent, Pixels, Point, Position, Render, SharedString, Style,
    Subscription, WeakEntity, Window, div, point, prelude::*, px,
};
use opensheet_engine::comment::{Comment, Post, Thread, utc_to_local};
use opensheet_engine::refs::cell_name;
use opensheet_engine::{Locale, format};

/// Width of a card floating over the grid.
pub(super) const WIDTH: f32 = 300.0;
/// Gap between a cell and the card beside it.
const GAP: f32 = 6.0;

/// The post of a thread being rewritten.
struct PostEdit {
    /// `None`: the comment that starts the thread.
    reply: Option<usize>,
    area: Entity<TextArea>,
    original: String,
    _subscription: Subscription,
}

pub(super) struct CommentCard {
    app: WeakEntity<ExcelApp>,
    pub sheet: usize,
    pub cell: (u32, u32),
    in_pane: bool,
    /// A comment is being written on a cell that has none yet.
    pub composing: bool,
    /// The new comment, or the reply.
    input: Entity<TextArea>,
    editing: Option<PostEdit>,
    /// Height the card may take beside its cell.
    room: f32,
    _subscription: Subscription,
}

/// The two letters shown in the round badge of an author.
pub(super) fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter_map(|word| word.chars().find(|ch| ch.is_alphanumeric()));
    let first = words.next();
    let last = words.last();
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// The colour of the badge of an author: always the same for a name.
pub(super) fn badge_color(name: &str) -> u32 {
    const COLORS: [u32; 12] = [
        0x750B1C, 0xA4262C, 0xCA5010, 0x986F0B, 0x498205, 0x038387, 0x004E8C, 0x4F6BED, 0x8764B8, 0x881798, 0xC239B3,
        0x69797E,
    ];
    let hash = name.bytes().fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as u32));
    COLORS[hash as usize % COLORS.len()]
}

/// When a post was written, on the clocks of this computer: the short
/// date and the time without seconds, as the workbook's language writes
/// them.
pub(super) fn time_text(local: f64, locale: &Locale) -> String {
    let code = format!("{} {}", locale.short_date_format, locale.time_format.replace(":ss", ""));
    format::format_number(local, &code, locale).text
}

fn badge(name: &str) -> impl IntoElement {
    div()
        .size(px(28.))
        .flex_none()
        .rounded_full()
        .bg(c(badge_color(name)))
        .text_color(c(0xFFFFFF))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initials(name))
}

/// A small square button showing an icon.
fn icon_button(id: impl Into<ElementId>, name: &'static str, tip: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(22.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .hover(|s| s.bg(c(theme::HOVER)))
        .active(|s| s.bg(c(theme::PRESSED)))
        .child(icon(name, 14.0, theme::TEXT_MUTED))
        .tooltip(tooltip(tip))
}

impl CommentCard {
    pub fn new(
        app: WeakEntity<ExcelApp>,
        sheet: usize,
        cell: (u32, u32),
        in_pane: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut area = if in_pane { TextArea::field(cx) } else { TextArea::new(cx) };
            area.max_height = Some(120.0);
            area
        });
        let subscription = cx.subscribe_in(&input, window, |card, _, event: &AreaEvent, window, cx| match event {
            AreaEvent::Submit => card.post(window, cx),
            AreaEvent::Escape => card.cancel(window, cx),
            _ => cx.notify(),
        });
        CommentCard {
            app,
            sheet,
            cell,
            in_pane,
            composing: false,
            input,
            editing: None,
            room: 480.0,
            _subscription: subscription,
        }
    }

    /// Put the cursor in the box of the card: to write the comment, or a
    /// reply.
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    pub fn set_room(&mut self, room: f32) {
        self.room = room;
    }

    /// What can hold the keyboard in this card.
    pub fn fields(&self, cx: &App) -> Vec<FocusHandle> {
        let edit = self.editing.as_ref().map(|edit| edit.area.focus_handle(cx));
        std::iter::once(self.input.focus_handle(cx)).chain(edit).collect()
    }

    /// Words were typed that are not in the workbook yet.
    pub fn has_draft(&self, cx: &App) -> bool {
        !self.input.read(cx).text().trim().is_empty()
            || self
                .editing
                .as_ref()
                .is_some_and(|edit| edit.area.read(cx).text() != edit.original)
    }

    /// Rewrite a post in place (`reply` `None`: the first comment).
    pub fn start_edit(&mut self, reply: Option<usize>, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let in_pane = self.in_pane;
        let area = cx.new(|cx| {
            let mut area = if in_pane { TextArea::field(cx) } else { TextArea::new(cx) };
            area.max_height = Some(160.0);
            area.set_text(text.clone(), cx);
            area
        });
        let subscription = cx.subscribe_in(&area, window, |card, _, event: &AreaEvent, window, cx| match event {
            AreaEvent::Submit => card.save_edit(window, cx),
            AreaEvent::Escape => card.cancel(window, cx),
            _ => cx.notify(),
        });
        window.focus(&area.focus_handle(cx));
        self.editing = Some(PostEdit {
            reply,
            area,
            original: text,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Run something on the application once this card is done with the
    /// event it is handling (the application may then look at the card).
    fn tell_app(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static,
    ) {
        let app = self.app.clone();
        window.defer(cx, move |window, cx| {
            app.update(cx, |app, cx| f(app, window, cx)).ok();
        });
    }

    /// Post what the box holds: the comment that starts the thread, or a
    /// reply to it.
    fn post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let (sheet, cell) = (self.sheet, self.cell);
        let card = cx.weak_entity();
        self.tell_app(window, cx, move |app, window, cx| {
            if super::post(app, sheet, cell, &text, cx) {
                card.update(cx, |card, cx| {
                    card.composing = false;
                    card.input.update(cx, |input, cx| input.set_text("", cx));
                    cx.notify();
                })
                .ok();
                window.focus(&app.grid_focus);
            }
        });
    }

    fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.editing else {
            return;
        };
        let text = edit.area.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let (sheet, cell, reply) = (self.sheet, self.cell, edit.reply);
        let changed = text != edit.original;
        let card = cx.weak_entity();
        self.tell_app(window, cx, move |app, window, cx| {
            if !changed || super::edit_post(app, sheet, cell, reply, &text, cx) {
                card.update(cx, |card, cx| {
                    card.editing = None;
                    cx.notify();
                })
                .ok();
                window.focus(&app.grid_focus);
            }
        });
    }

    /// Escape, or Cancel: drop what is being typed; with nothing in hand,
    /// put the card away.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (sheet, cell, in_pane) = (self.sheet, self.cell, self.in_pane);
        let had_text = !self.input.read(cx).text().is_empty();
        let mut close = false;
        if self.editing.take().is_none() {
            self.input.update(cx, |input, cx| input.set_text("", cx));
            close = self.composing || !had_text;
            self.composing = false;
        }
        cx.notify();
        self.tell_app(window, cx, move |app, window, cx| {
            if close && !in_pane {
                super::close_card(app, sheet, cell, cx);
            }
            window.focus(&app.grid_focus);
            cx.notify();
        });
    }

    /// The "…" menu of a post.
    fn open_menu(
        &mut self,
        reply: Option<usize>,
        post: &Post,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (sheet, cell) = (self.sheet, self.cell);
        let card = cx.weak_entity();
        let text = post.text.clone();
        let mut entries = vec![MenuEntry::new(t("Edit comment", "Modifier le commentaire"), move |app, window, cx| {
            if super::may_edit(app, cx) {
                let text = text.clone();
                card.update(cx, |card, cx| card.start_edit(reply, text, window, cx)).ok();
            }
        })];
        match reply {
            None => {
                entries.push(MenuEntry::new(t("Delete thread", "Supprimer la conversation"), move |app, _, cx| {
                    super::delete_thread(app, sheet, cell, cx)
                }));
                entries.push(MenuEntry::new(t("Resolve thread", "Résoudre la conversation"), move |app, _, cx| {
                    super::resolve(app, sheet, cell, true, cx)
                }));
            }
            Some(index) => entries.push(MenuEntry::new(t("Delete comment", "Supprimer le commentaire"), move |app, _, cx| {
                super::delete_reply(app, sheet, cell, index, cx)
            })),
        }
        self.tell_app(window, cx, move |app, _, cx| app.open_context_menu(position, entries, cx));
    }

    /// A field with its text box, and under it the two buttons once it is
    /// in use.
    fn field(
        &self,
        id: &'static str,
        area: &Entity<TextArea>,
        accept: &'static str,
        on_accept: fn(&mut Self, &mut Window, &mut Context<Self>),
        active: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = area.focus_handle(cx).is_focused(window);
        let empty = area.read(cx).text().trim().is_empty();
        let clicked = area.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .id(id)
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(if focused { theme::ACCENT } else { theme::BORDER_STRONG }))
                    .cursor(CursorStyle::IBeam)
                    .line_height(px(17.))
                    // A press beside the text still puts the cursor in it.
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                        window.focus(&clicked.focus_handle(cx));
                        clicked.update(cx, |area, cx| area.caret_to_point(event.position, cx));
                    })
                    .child(area.clone()),
            )
            .when(active || focused || !empty, |d| {
                d.child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            Button::new((id, 1usize), accept)
                                .primary()
                                .disabled(empty)
                                .width(64.0)
                                .on_click(cx.listener(move |card, _, window, cx| on_accept(card, window, cx))),
                        )
                        .child(
                            Button::new((id, 2usize), t("Cancel", "Annuler"))
                                .width(64.0)
                                .on_click(cx.listener(|card, _, window, cx| card.cancel(window, cx))),
                        ),
                )
            })
            .into_any_element()
    }

    /// One post: the badge, the name and the time of its author, then its
    /// text (or the field in which it is being rewritten).
    fn post_view(
        &self,
        reply: Option<usize>,
        post: &Post,
        resolved: bool,
        locale: &'static Locale,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = if post.author.name.is_empty() { t("Unknown", "Inconnu") } else { post.author.name.as_str() };
        let index = reply.map_or(0, |index| index + 1);
        let edit = self.editing.as_ref().filter(|edit| edit.reply == reply);
        let menu_post = post.clone();
        let (row, col) = self.cell;
        div()
            .flex()
            .gap(px(8.))
            .child(badge(name))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(name.to_string())),
                            )
                            .when(self.in_pane && reply.is_none(), |d| {
                                d.child(div().text_color(c(theme::TEXT_MUTED)).child(cell_name(row, col)))
                            })
                            .when(!resolved && edit.is_none(), |d| {
                                d.child(
                                    icon_button(
                                        ("post-menu", index),
                                        "more",
                                        match reply {
                                            None => t("More thread actions", "Autres actions sur la conversation"),
                                            Some(_) => t("More comment actions", "Plus d’actions de commentaires"),
                                        },
                                    )
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |card, event: &MouseDownEvent, window, cx| {
                                            card.open_menu(reply, &menu_post, event.position, window, cx)
                                        }),
                                    ),
                                )
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(c(theme::TEXT_MUTED))
                            .child(time_text(utc_to_local(post.time), locale)),
                    )
                    .child(match edit {
                        Some(edit) => div()
                            .pt(px(4.))
                            .child(self.field("edit-field", &edit.area, t("Save", "Enregistrer"), Self::save_edit, true, window, cx))
                            .into_any_element(),
                        None => div()
                            .pt(px(2.))
                            .line_height(px(17.))
                            .children(post.text.split('\n').map(|line| {
                                div().min_h(px(17.)).child(SharedString::from(line.to_string()))
                            }))
                            .into_any_element(),
                    }),
            )
            .into_any_element()
    }

    /// The band of a resolved thread: it says so, and offers to reopen or
    /// delete it.
    fn resolved_band(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (sheet, cell) = (self.sheet, self.cell);
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_color(c(theme::TEXT_MUTED))
            .child(icon("check", 13.0, theme::TEXT_MUTED))
            .child(div().flex_1().child(t("Resolved", "Résolu")))
            .child(
                icon_button("reopen", "comments-reopen", t("Reopen thread", "Rouvrir la conversation")).on_click(
                    cx.listener(move |card, _, window, cx| {
                        card.tell_app(window, cx, move |app, _, cx| super::resolve(app, sheet, cell, false, cx))
                    }),
                ),
            )
            .child(
                icon_button("delete-thread", "trash", t("Delete thread", "Supprimer la conversation")).on_click(
                    cx.listener(move |card, _, window, cx| {
                        card.tell_app(window, cx, move |app, _, cx| super::delete_thread(app, sheet, cell, cx))
                    }),
                ),
            )
    }
}

impl Render for CommentCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.upgrade();
        let (row, col) = self.cell;
        let (thread, locale): (Option<Thread>, &'static Locale) = match &app {
            Some(app) => {
                let app = app.read(cx);
                (
                    app.wb.comment_at(self.sheet, row, col).and_then(Comment::as_thread).cloned(),
                    app.wb.locale,
                )
            }
            None => (None, Locale::en()),
        };
        // An edit of a post that is no longer there (undone, deleted) ends.
        if let Some(edit) = &self.editing {
            let exists = thread
                .as_ref()
                .is_some_and(|thread| !thread.resolved && edit.reply.is_none_or(|index| index < thread.replies.len()));
            if !exists {
                self.editing = None;
            }
        }
        self.input.update(cx, |input, _| {
            input.placeholder = match thread {
                Some(_) => t("Reply", "Répondre"),
                None => t("Start a conversation", "Démarrez une conversation"),
            }
            .into();
        });

        let (sheet, cell, in_pane) = (self.sheet, self.cell, self.in_pane);
        let key = ((row as usize) << 16) | col as usize;
        let mut card = div()
            .id(("comment-card", key))
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(10.))
            .bg(c(0xFFFFFF))
            .text_size(px(12.5))
            .text_color(c(theme::TEXT))
            .rounded(px(6.))
            .border_1()
            .border_color(c(theme::BORDER_STRONG));
        card = if in_pane {
            card.w_full()
        } else {
            card.w(px(WIDTH))
                .shadow_lg()
                .occlude()
                // A press on the card keeps it open when the pointer leaves.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |card, _, window, cx| {
                        card.tell_app(window, cx, move |app, _, cx| super::pin_card(app, sheet, cell, cx))
                    }),
                )
                .on_hover(cx.listener(move |card, inside: &bool, window, cx| {
                    let inside = *inside;
                    card.tell_app(window, cx, move |app, _, cx| super::pointer_on_card(app, inside, cx));
                }))
        };

        match &thread {
            Some(thread) => {
                let mut posts = vec![self.post_view(None, &thread.first, thread.resolved, locale, window, cx)];
                for (index, reply) in thread.replies.iter().enumerate() {
                    posts.push(self.post_view(Some(index), reply, thread.resolved, locale, window, cx));
                }
                card = card
                    .when(thread.resolved, |d| d.child(self.resolved_band(cx)))
                    .child(
                        div()
                            .id("posts")
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .when(thread.resolved, |d| d.opacity(0.6))
                            .when(!in_pane, |d| d.max_h(px((self.room - 150.0).max(80.0))).overflow_y_scroll())
                            .children(posts),
                    );
                if !thread.resolved {
                    let reply = self.input.clone();
                    card = card.child(self.field("reply-field", &reply, t("Post", "Publier"), Self::post, false, window, cx));
                }
            }
            None => {
                let name = user_name();
                let input = self.input.clone();
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(badge(name))
                            .child(div().flex_1().truncate().font_weight(FontWeight::SEMIBOLD).child(name))
                            .when(in_pane, |d| d.child(div().text_color(c(theme::TEXT_MUTED)).child(cell_name(row, col)))),
                    )
                    .child(self.field("new-field", &input, t("Post", "Publier"), Self::post, true, window, cx));
            }
        }
        card
    }
}

// ----------------------------------------------------------------- placing

/// Puts a card beside a cell, inside the cells in view: right of the cell
/// when there is room, else left of it, and never past the top or the
/// bottom of the grid.
pub(super) struct Beside {
    child: AnyElement,
    cell: Bounds<Pixels>,
    limits: Bounds<Pixels>,
}

impl Beside {
    /// `cell` and `limits` are in window coordinates.
    pub fn new(child: impl IntoElement, cell: Bounds<Pixels>, limits: Bounds<Pixels>) -> Beside {
        Beside {
            child: child.into_any_element(),
            cell,
            limits,
        }
    }
}

/// Where the corner of a card of a given size goes (see `Beside`). Boxes
/// are (x0, y0, x1, y1).
pub(super) fn beside(size: (f32, f32), cell: (f32, f32, f32, f32), limits: (f32, f32, f32, f32)) -> (f32, f32) {
    let mut x = cell.2 + GAP;
    if x + size.0 > limits.2 {
        x = cell.0 - GAP - size.0;
        if x < limits.0 {
            x = (limits.2 - size.0).max(limits.0);
        }
    }
    let y = cell.1.min(limits.3 - size.1).max(limits.1);
    (x, y)
}

impl IntoElement for Beside {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Beside {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let child = self.child.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let size = window.layout_bounds(*child).size;
        let edges = |b: &Bounds<Pixels>| (f32::from(b.left()), f32::from(b.top()), f32::from(b.right()), f32::from(b.bottom()));
        let (x, y) = beside(
            (f32::from(size.width), f32::from(size.height)),
            edges(&self.cell),
            edges(&self.limits),
        );
        let offset = point(px(x.round()), px(y.round())) - bounds.origin;
        window.with_element_offset(offset, |window| self.child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges() {
        assert_eq!(initials("Thomas Tastet"), "TT");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("Jean-Luc de la Fontaine"), "JF");
        assert_eq!(initials("  "), "");
        assert_eq!(initials("(Éric) Blanc"), "ÉB");
        // A name always gets the same colour, whatever its neighbours.
        assert_eq!(badge_color("Ada"), badge_color("Ada"));
        assert_ne!(badge_color("Ada"), badge_color("Bob"));
    }

    #[test]
    fn times_follow_the_language() {
        // 5 March 2024, 14:07:30.
        let serial = 45_356.0 + (14.0 * 3600.0 + 7.0 * 60.0 + 30.0) / 86_400.0;
        assert_eq!(time_text(serial, Locale::en()), "3/5/2024 2:07 PM");
        assert_eq!(time_text(serial, Locale::fr()), "05/03/2024 14:07");
    }

    #[test]
    fn cards_stay_inside_the_grid() {
        let limits = (30.0, 20.0, 800.0, 600.0);
        let size = (300.0, 200.0);
        // Room on the right: beside the cell, level with its top.
        assert_eq!(beside(size, (200.0, 100.0, 264.0, 120.0), limits), (270.0, 100.0));
        // No room: on the other side.
        assert_eq!(beside(size, (600.0, 100.0, 664.0, 120.0), limits), (294.0, 100.0));
        // A cell near the bottom: the card is pushed up; near the top of a
        // scrolled sheet: it stays under the headers.
        assert_eq!(beside(size, (200.0, 560.0, 264.0, 580.0), limits), (270.0, 400.0));
        assert_eq!(beside(size, (200.0, 5.0, 264.0, 25.0), limits), (270.0, 20.0));
        // A grid too narrow for either side: inside it all the same.
        assert_eq!(beside(size, (100.0, 100.0, 164.0, 120.0), (30.0, 20.0, 340.0, 600.0)), (40.0, 100.0));
    }
}
