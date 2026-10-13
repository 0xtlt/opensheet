//! The Help tab: Help and Feedback.
//!
//! Excel's Help opens a pane that searches Microsoft's help on the web.
//! OpenSheet has no server: its Help pane searches the documentation the
//! application carries, which is generated, not written here — what the
//! project's README says works and is missing (`docs.rs`; those sections
//! exist in French only and are shown as they are), and the keyboard
//! shortcuts the application binds (`shortcuts.rs`).
//!
//! Feedback (`feedback.rs`) hands what the user writes to the share sheet
//! of macOS. Excel's other buttons of this tab are left out because they
//! could only be empty: Contact Support (there is no support service),
//! Show Training (there are no training workbooks) and What's New (the
//! project keeps no list of what each version added that could be shown
//! to a user).

mod docs;
mod feedback;
mod shortcuts;
#[cfg(test)]
pub(crate) mod ui_tests;

use self::docs::{Line, Topic};
use super::{Command, Cx};
use crate::app::ExcelApp;
use crate::i18n::{Lang, t};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, TextField, icon};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Menu, MenuItem, SharedString, Stateful, Window, actions, div,
    prelude::*, px,
};

/// Id of the Help pane.
const PANE: &str = "help";

commands! {
    /// What the Help tab offers.
    pub enum Cmd {
        Help = "book", t("Help", "Aide");
        Feedback = "comment", t("Feedback", "Commentaires");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn checked(self, app: &ExcelApp) -> bool {
        self == Cmd::Help && app.pane_open(PANE)
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::Help => {
                if app.pane_open(PANE) {
                    app.close_pane(PANE, cx);
                    return;
                }
                let pane = cx.new(|cx| HelpPane::new(cx));
                app.open_pane(PANE, Cmd::Help.label(), pane, cx);
            }
            Cmd::Feedback => {
                app.popup = None;
                app.open_modal(window, cx, feedback::FeedbackDialog::new);
            }
        }
    }
}

// --------------------------------------------------------------------- pane

/// The Help pane: a search field over the topics of the documentation.
pub(crate) struct HelpPane {
    search: Entity<TextInput>,
    topics: Vec<Topic>,
    /// The topic being read; `None` shows the list of topics.
    open: Option<usize>,
}

impl HelpPane {
    fn new(cx: &mut Context<Self>) -> HelpPane {
        let search = widgets::input(cx, "");
        search.update(cx, |search, _| search.placeholder = t("Search", "Rechercher").into());
        let mut topics = docs::readme_topics();
        topics.push(Topic {
            section: t("Keyboard shortcuts", "Raccourcis clavier").to_string(),
            title: t("Keyboard shortcuts", "Raccourcis clavier").to_string(),
            lines: shortcuts::lines(cx),
        });
        HelpPane {
            search,
            topics,
            open: None,
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    /// The titles of the topics, in the order of the list.
    #[cfg(test)]
    pub fn titles(&self) -> Vec<&str> {
        self.topics.iter().map(|topic| topic.title.as_str()).collect()
    }

    #[cfg(test)]
    pub fn topic(&self, title: &str) -> &Topic {
        self.topics.iter().find(|topic| topic.title == title).unwrap_or_else(|| panic!("no topic {title}"))
    }

    /// What the pane shows right now: the lines a search found, per topic
    /// (`None` when nothing is searched).
    #[cfg(test)]
    pub fn found(&self, cx: &App) -> Option<Vec<(String, Vec<String>)>> {
        let query = self.query(cx);
        (!query.is_empty()).then(|| {
            docs::search(&self.topics, &query)
                .into_iter()
                .map(|(topic, lines)| {
                    let topic = &self.topics[topic];
                    (topic.title.clone(), lines.into_iter().map(|line| topic.lines[line].text.clone()).collect())
                })
                .collect()
        })
    }

    #[cfg(test)]
    pub fn open_topic(&self) -> Option<&str> {
        self.open.map(|index| self.topics[index].title.as_str())
    }

    fn line(line: &Line) -> AnyElement {
        div()
            .flex()
            .gap(px(6.))
            .when(line.bullet, |d| d.child(div().flex_none().text_color(c(theme::TEXT_MUTED)).child("•")))
            .when_some(line.keys.clone(), |d, keys| {
                d.child(div().flex_none().w(px(104.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(keys)))
            })
            .child(div().flex_1().min_w(px(0.)).child(SharedString::from(line.text.clone())))
            .into_any_element()
    }

    fn heading(text: &str) -> gpui::Div {
        div().pt(px(6.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(text.to_string()))
    }

    /// The list of topics, under the headings of their sections.
    fn contents(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        // The README is written in French: say so to who reads another
        // language.
        if crate::i18n::lang() != Lang::Fr {
            out.push(
                div()
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t(
                        "The description of what OpenSheet does and does not do exists in French only.",
                        "La description de ce qu’OpenSheet fait et ne fait pas n’existe qu’en français.",
                    ))
                    .into_any_element(),
            );
        }
        let mut section = "";
        for (index, topic) in self.topics.iter().enumerate() {
            if topic.section != section {
                section = &topic.section;
                // A section that is its only topic needs no heading.
                if topic.title != topic.section {
                    out.push(Self::heading(section).into_any_element());
                }
            }
            out.push(
                div()
                    .id(("help-topic", index))
                    .debug_selector(|| format!("help-topic-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .h(px(24.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(div().flex_1().truncate().text_color(c(theme::ACCENT_DARK)).child(SharedString::from(topic.title.clone())))
                    .child(icon("chevron-right", 10.0, theme::TEXT_MUTED))
                    .on_click(cx.listener(move |pane, _, _, cx| {
                        pane.open = Some(index);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        out
    }

    /// One topic, with the way back to the list.
    fn page(&self, index: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let topic = &self.topics[index];
        let mut out = vec![
            div()
                .id("help-back")
                .debug_selector(|| "help-back".into())
                .flex()
                .items_center()
                .gap(px(4.))
                .h(px(24.))
                .text_color(c(theme::ACCENT_DARK))
                .child(icon("chevron-left", 10.0, theme::ACCENT_DARK))
                .child(t("Back", "Retour"))
                .on_click(cx.listener(|pane, _, _, cx| {
                    pane.open = None;
                    cx.notify();
                }))
                .into_any_element(),
            Self::heading(&topic.title).text_size(px(14.)).into_any_element(),
        ];
        out.extend(topic.lines.iter().map(Self::line));
        out
    }

    /// What a search found, topic by topic.
    fn results(&self, query: &str) -> Vec<AnyElement> {
        let found = docs::search(&self.topics, query);
        if found.is_empty() {
            return vec![
                div()
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t("No results found", "Aucun résultat trouvé"))
                    .into_any_element(),
            ];
        }
        let mut out = Vec::new();
        for (topic, lines) in found {
            let topic = &self.topics[topic];
            out.push(Self::heading(&topic.title).into_any_element());
            out.extend(lines.into_iter().map(|line| Self::line(&topic.lines[line])));
        }
        out
    }
}

impl Render for HelpPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query(cx);
        let body = if !query.is_empty() {
            self.results(&query)
        } else {
            match self.open.filter(|index| *index < self.topics.len()) {
                Some(index) => self.page(index, cx),
                None => self.contents(cx),
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .px(px(12.))
                    .pb(px(8.))
                    .debug_selector(|| "help-search".into())
                    .child(TextField::new(&self.search).trailing(icon("search", 13.0, theme::TEXT_MUTED))),
            )
            .child(
                div()
                    .id("help-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .px(px(12.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .line_height(px(17.))
                    .children(body),
            )
    }
}

// ---------------------------------------------------------------- menu bar

actions!(help, [OpenHelp, SendFeedback]);

/// The Help menu of the menu bar.
pub fn menu_bar() -> Menu {
    Menu {
        name: Cmd::Help.label().into(),
        items: vec![
            MenuItem::action(t("OpenSheet Help", "Aide d’OpenSheet"), OpenHelp),
            MenuItem::separator(),
            MenuItem::action(Cmd::Feedback.label(), SendFeedback),
        ],
    }
}

/// Make the window answer the Help menu of the menu bar.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &OpenHelp, window, cx| {
        // The menu opens the pane; only the button of the ribbon closes it.
        if !app.pane_open(PANE) {
            app.run_command(Cmd::Help, window, cx);
        }
    }))
    .on_action(cx.listener(|app, _: &SendFeedback, window, cx| app.run_command(Cmd::Feedback, window, cx)))
}
