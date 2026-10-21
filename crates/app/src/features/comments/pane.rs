//! The Comments pane: every thread of the sheet as a card, in sheet order.

use super::card::CommentCard;
use crate::app::ExcelApp;
use crate::i18n::{t, t_ctx};
use crate::theme::{self, c};
use crate::widgets::icon;
use gpui::{
    Context, Entity, IntoElement, MouseButton, Render, ScrollHandle, WeakEntity, Window, div,
    prelude::*, px,
};
use opensheet_engine::comment::CommentKind;
use std::collections::BTreeSet;

pub(super) struct CommentsPane {
    app: WeakEntity<ExcelApp>,
    /// The sheet the cards belong to.
    sheet: usize,
    /// In sheet order.
    cards: Vec<Entity<CommentCard>>,
    scroll: ScrollHandle,
    /// The card to bring into view when the pane is next drawn.
    reveal: Option<(u32, u32)>,
}

impl CommentsPane {
    pub fn new(app: WeakEntity<ExcelApp>) -> CommentsPane {
        CommentsPane {
            app,
            sheet: 0,
            cards: Vec::new(),
            scroll: ScrollHandle::new(),
            reveal: None,
        }
    }

    /// Bring the card of a cell into view (the selection went to it).
    pub fn reveal(&mut self, cell: (u32, u32), cx: &mut Context<Self>) {
        self.reveal = Some(cell);
        cx.notify();
    }

    /// Show the card of a cell of the current sheet with the cursor in its
    /// box: to reply to its thread, or (`composing`) to write the comment
    /// of a cell that has none.
    pub fn write(
        &mut self,
        sheet: usize,
        cell: (u32, u32),
        composing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if sheet != self.sheet {
            self.cards.clear();
            self.sheet = sheet;
        }
        let card = match self.cards.iter().find(|card| card.read(cx).cell == cell) {
            Some(card) => card.clone(),
            None => {
                let app = self.app.clone();
                let card = cx.new(|cx| CommentCard::new(app, sheet, cell, true, window, cx));
                self.cards.push(card.clone());
                card
            }
        };
        card.update(cx, |card, cx| {
            card.composing = composing;
            card.focus_input(window, cx);
        });
        self.reveal(cell, cx);
    }

    /// The cells of the cards, in the order they are listed.
    #[cfg(test)]
    pub fn cells(&self, cx: &gpui::App) -> Vec<(u32, u32)> {
        self.cards.iter().map(|card| card.read(cx).cell).collect()
    }

    /// Line the cards up with the threads of the current sheet, keeping
    /// those in which a new comment is being written.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            self.cards.clear();
            return;
        };
        let (sheet, threads) = {
            let app = app.read(cx);
            (
                app.sheet,
                app.wb.sheets[app.sheet].comment_cells(Some(CommentKind::Thread)),
            )
        };
        if sheet != self.sheet {
            self.cards.clear();
            self.sheet = sheet;
        }
        let mut wanted: BTreeSet<(u32, u32)> = threads.into_iter().collect();
        wanted.extend(
            self.cards
                .iter()
                .map(|card| card.read(cx))
                .filter(|card| card.composing)
                .map(|card| card.cell),
        );
        self.cards
            .retain(|card| wanted.contains(&card.read(cx).cell));
        for cell in wanted {
            if !self.cards.iter().any(|card| card.read(cx).cell == cell) {
                let app = self.app.clone();
                self.cards
                    .push(cx.new(|cx| CommentCard::new(app, sheet, cell, true, window, cx)));
            }
        }
        self.cards.sort_by_key(|card| card.read(cx).cell);
    }
}

impl Render for CommentsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let (active, elsewhere) = match self.app.upgrade() {
            Some(app) => {
                let app = app.read(cx);
                (
                    Some(app.active()),
                    app.wb.first_comment(Some(CommentKind::Thread)).is_some(),
                )
            }
            None => (None, false),
        };
        if let Some(cell) = self.reveal.take() {
            if let Some(index) = self
                .cards
                .iter()
                .position(|card| card.read(cx).cell == cell)
            {
                self.scroll.scroll_to_item(index);
            }
        }
        let cards: Vec<_> = self
            .cards
            .iter()
            .enumerate()
            .map(|(index, card)| {
                let cell = card.read(cx).cell;
                let app = self.app.clone();
                div()
                    .id(("pane-card", index))
                    .debug_selector(|| format!("pane-card-{index}"))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(if active == Some(cell) {
                        c(theme::ACCENT)
                    } else {
                        gpui::transparent_black()
                    })
                    // A press on a card goes to its cell.
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        let app = app.clone();
                        window.defer(cx, move |_, cx| {
                            app.update(cx, |app, cx| {
                                if app.active() != cell {
                                    app.select_cell(cell.0, cell.1, cx);
                                }
                            })
                            .ok();
                        });
                    })
                    .child(card.clone())
            })
            .collect();
        let app = self.app.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().flex_none().px(px(12.)).pb(px(8.)).flex().child(
                    div()
                        .id("new-comment")
                        .debug_selector(|| "pane-new-comment".into())
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .active(|s| s.bg(c(theme::PRESSED)))
                        .child(icon("comment", 14.0, theme::TEXT))
                        .child(t_ctx("comment", "New", "Nouveau"))
                        .on_click(move |_, window, cx| {
                            let app = app.clone();
                            window.defer(cx, move |window, cx| {
                                app.update(cx, |app, cx| {
                                    app.run_command(super::Cmd::NewComment, window, cx)
                                })
                                .ok();
                            });
                        }),
                ),
            )
            .child(
                div()
                    .id("comment-cards")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(10.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .when(cards.is_empty(), |d| {
                        d.child(
                            div()
                                .px(px(2.))
                                .pt(px(6.))
                                .text_color(c(theme::TEXT_MUTED))
                                .child(if elsewhere {
                                    t(
                                        "There are comments in this file, but not here.",
                                        "Ce fichier contient des commentaires, mais pas ici.",
                                    )
                                } else {
                                    t(
                                        "There are no comments in this file.",
                                        "Ce fichier ne comporte aucun commentaire.",
                                    )
                                }),
                        )
                    })
                    .children(cards),
            )
    }
}
