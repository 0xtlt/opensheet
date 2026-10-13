//! Help ▸ Feedback: Excel's three kinds of feedback, a box to write in,
//! and Send.
//!
//! Excel sends what is written to Microsoft. OpenSheet has no server and
//! no address of its own to write to, so Send hands the text, with the
//! version of the application and of macOS, to the share sheet of macOS:
//! the user chooses Mail (a new message without recipient) or another
//! service there. The dialog says so, and never claims anything was sent.

use crate::features::share::system::{Item, system};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::theme::{self, c};
use crate::widgets::text_area::{AreaEvent, TextArea};
use crate::widgets::{Button, RadioGroup, dialog};
use gpui::{
    Bounds, Context, Entity, Focusable, MouseButton, MouseDownEvent, Pixels, SharedString, Subscription, Window,
    canvas, div, prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// What the feedback is about: Excel's three choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Like,
    Dislike,
    Idea,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Like, Kind::Dislike, Kind::Idea];

    pub fn title(self) -> &'static str {
        match self {
            Kind::Like => t("I like something", "Quelque chose me plaît"),
            Kind::Dislike => t("I don't like something", "Quelque chose ne me plaît pas"),
            Kind::Idea => t("I have a suggestion", "J’ai une suggestion"),
        }
    }

    /// What the empty box says.
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Like => t("Tell us what you like", "Dites-nous ce que vous avez aimé."),
            Kind::Dislike => t("Tell us what you didn't like", "Dites-nous ce que vous n’avez pas apprécié"),
            Kind::Idea => t("Tell us your idea", "Dites-nous-en plus sur votre idée"),
        }
    }
}

/// The message handed to the share sheet: what the feedback is about,
/// what was written, and the versions it was written on.
pub(crate) fn message(kind: Kind, text: &str, os: &str) -> String {
    format!("{}\n\n{}\n\nOpenSheet {} — {os}", kind.title(), text.trim(), env!("CARGO_PKG_VERSION"))
}

pub(crate) struct FeedbackDialog {
    modal: Modal,
    pub kind: Kind,
    text: Entity<TextArea>,
    /// Where the Send button was drawn: the share sheet hangs under it.
    send_at: Rc<Cell<Bounds<Pixels>>>,
    /// The box has not been given the keyboard yet.
    needs_focus: bool,
    _subscription: Subscription,
}

impl FeedbackDialog {
    pub fn new(modal: Modal, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| {
            let mut area = TextArea::field(cx);
            area.max_height = Some(120.0);
            area.placeholder = Kind::Like.hint().into();
            area
        });
        let subscription = cx.subscribe_in(&text, window, |dialog, _, event: &AreaEvent, window, cx| match event {
            AreaEvent::Submit => dialog.send(&(), window, cx),
            AreaEvent::Escape => dialog.modal.close(window, cx),
            _ => cx.notify(),
        });
        FeedbackDialog {
            modal,
            kind: Kind::Like,
            text,
            send_at: Rc::new(Cell::new(Bounds::default())),
            needs_focus: true,
            _subscription: subscription,
        }
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.kind = Kind::ALL[index.min(Kind::ALL.len() - 1)];
        let hint = self.kind.hint();
        self.text.update(cx, |text, cx| {
            text.placeholder = hint.into();
            cx.notify();
        });
        cx.notify();
    }

    /// Hand the feedback to the share sheet. The dialog stays open: the
    /// user may leave the sheet without sending, and the text is theirs.
    fn send(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let system = system();
        let body = message(self.kind, &text, &system.os_version());
        if let Err(err) = system.share(&[Item::Text(body)], self.send_at.get(), window, cx) {
            let message = format!(
                "{}\n\n{err}",
                t(
                    "Something went wrong and your feedback couldn't be handed to the share sheet.",
                    "Un problème est survenu et vos commentaires n’ont pas pu être transmis au menu de partage."
                )
            );
            self.modal.update_app(cx, |app, cx| {
                app.alert(Alert::new(t("Send Feedback", "Envoyer des commentaires"), message).warning(), cx)
            });
        }
    }
}

#[cfg(test)]
impl FeedbackDialog {
    pub fn draft(&self, cx: &gpui::App) -> String {
        self.text.read(cx).text().to_string()
    }

    pub fn text_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.text.focus_handle(cx).is_focused(window)
    }

    pub fn hint(&self, cx: &gpui::App) -> String {
        self.text.read(cx).placeholder.to_string()
    }
}

impl Render for FeedbackDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A dialog gives the keyboard to its first control, the choices;
        // here one writes first.
        if std::mem::take(&mut self.needs_focus) {
            window.focus(&self.text.focus_handle(cx));
        }
        let options: Vec<SharedString> = Kind::ALL.iter().map(|kind| kind.title().into()).collect();
        let selected = Kind::ALL.iter().position(|kind| *kind == self.kind);
        let empty = self.text.read(cx).text().trim().is_empty();
        let field = self.text.clone();
        let (send_at, close) = (self.send_at.clone(), self.modal.clone());
        dialog(&self.modal, t("Send Feedback", "Envoyer des commentaires"))
            .width(440.0)
            // Send is the default button: Return presses it, except in the
            // box, where it starts a line.
            .on_ok(cx.listener(Self::send))
            .child(
                div().debug_selector(|| "feedback-kind".into()).child(
                    RadioGroup::new("feedback-kind", options, selected)
                        .on_change(cx.listener(|this, index: &usize, _, cx| this.choose(*index, cx))),
                ),
            )
            .child(
                div()
                    .id("feedback-text")
                    .debug_selector(|| "feedback-text".into())
                    .h(px(132.))
                    .w_full()
                    .p(px(5.))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(0xABABAB))
                    .rounded(px(3.))
                    .line_height(px(17.))
                    .overflow_hidden()
                    // A click below the text still lands in the box.
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                        window.focus(&field.read(cx).focus_handle(cx));
                        field.update(cx, |field, cx| field.caret_to_point(event.position, cx));
                    })
                    .child(self.text.clone()),
            )
            .child(div().text_color(c(theme::TEXT_MUTED)).child(t(
                "OpenSheet has no feedback service. Send opens the macOS share sheet with your text and the versions of OpenSheet and macOS: choose Mail or another service there, and who receives it.",
                "OpenSheet n’a pas de service de commentaires. Envoyer ouvre le menu de partage de macOS avec votre texte et les versions d’OpenSheet et de macOS : choisissez-y Mail ou un autre service, et le destinataire."
            )))
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .relative()
                            .debug_selector(|| "feedback-send".into())
                            .child(
                                Button::new("feedback-send", t("Send", "Envoyer"))
                                    .primary()
                                    .disabled(empty)
                                    .on_click(cx.listener(Self::send)),
                            )
                            .child(
                                canvas(move |bounds, _, _| send_at.set(bounds), |_, _, _, _| {})
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full(),
                            ),
                    )
                    .child(
                        div().debug_selector(|| "feedback-close".into()).child(
                            Button::new("feedback-close", t("Close", "Fermer"))
                                .on_click(move |_, window, cx| close.close(window, cx)),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_says_what_where_and_on_which_versions() {
        let text = message(Kind::Idea, "  Add sparkline dates.\n", "macOS 15.1");
        assert_eq!(
            text,
            format!("I have a suggestion\n\nAdd sparkline dates.\n\nOpenSheet {} — macOS 15.1", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(Kind::ALL.map(Kind::title), ["I like something", "I don't like something", "I have a suggestion"]);
    }
}
