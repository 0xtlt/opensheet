//! Review ▸ Thesaurus (⇧F7): the pane that lists the meanings of a word
//! and, under each, the words that share it.
//!
//! The synonyms are those of the thesaurus dictionaries installed on the
//! Mac (`system::Thesaurus`). Apple ships such dictionaries for English
//! only; for a word they do not hold, and on a Mac that has none, the
//! pane says so.

use super::system::{self, Sense, Synonyms};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{self, Button, TextField};
use gpui::{ClipboardItem, Context, Entity, MouseButton, MouseDownEvent, ScrollHandle, SharedString, Subscription, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::Rect;

pub(super) const PANE: &str = "proofing-thesaurus";

/// The pane, kept so that the command can hand it another word.
#[derive(Default)]
pub(super) struct ThesaurusUi {
    pub pane: Option<Entity<ThesaurusPane>>,
}

pub(super) struct ThesaurusPane {
    app: WeakEntity<ExcelApp>,
    search: Entity<TextInput>,
    /// The word whose meanings are listed; empty before any search.
    pub word: String,
    pub result: Option<Synonyms>,
    /// The words looked up before this one, for Back.
    history: Vec<String>,
    /// The synonym Insert and Copy work on.
    pub chosen: Option<String>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

/// Excel's short names for the parts of speech, from the word the
/// dictionary uses.
fn part_of_speech(name: &str) -> String {
    let name = name.trim().to_lowercase();
    let known = match name.as_str() {
        "noun" | "plural noun" | "mass noun" => t("noun", "nom"),
        "adjective" => t("adj.", "adj."),
        "verb" => t("verb", "verbe"),
        "adverb" => t("adverb", "adverbe"),
        "pronoun" => t("pronoun", "pronom"),
        "conjunction" => t("conj.", "conj."),
        "preposition" => t("prep.", "prép."),
        "exclamation" | "interjection" => t("interj.", "interj."),
        _ => return name,
    };
    known.to_string()
}

/// The heading of a meaning: its first word and what kind of word it is,
/// as Excel writes it ("cheerful (adj.)").
pub(super) fn heading(sense: &Sense) -> String {
    let word = sense.synonyms.first().or(sense.antonyms.first()).cloned().unwrap_or_default();
    let kind = part_of_speech(&sense.part_of_speech);
    if kind.is_empty() { word } else { format!("{word} ({kind})") }
}

/// The word of the active cell, as the thesaurus is asked for it.
pub(super) fn word_of_active_cell(app: &ExcelApp) -> String {
    let (row, col) = app.active();
    let text = app.wb.display_text(app.sheet, row, col).text;
    text.trim().trim_matches(|ch: char| ch.is_ascii_punctuation()).to_string()
}

/// Put a word in the active cell in place of what it holds, as one undo
/// step. False when the sheet's protection refuses (it said why).
pub(super) fn insert(app: &mut ExcelApp, word: &str, cx: &mut Cx) -> bool {
    let (row, col) = app.active();
    let cell = Rect::cell(row, col);
    if !app.may(Intent::EditCells(cell), cx) {
        return false;
    }
    let sheet = app.sheet;
    if app.wb.set_cell_text(sheet, row, col, word).is_err() {
        return false;
    }
    app.commit_cells(t("Thesaurus", "Dictionnaire des synonymes"), &[cell], ChangeCause::Typing, cx);
    true
}

impl ThesaurusPane {
    fn new(app: WeakEntity<ExcelApp>, cx: &mut Context<Self>) -> Self {
        let search = widgets::input(cx, "");
        search.update(cx, |field, _| field.placeholder = t("Search", "Rechercher").into());
        let subscription = cx.subscribe(&search, |this: &mut ThesaurusPane, field, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Enter { .. }) {
                let word = field.read(cx).text().to_string();
                this.look_up(&word, cx);
            }
        });
        ThesaurusPane {
            app,
            search,
            word: String::new(),
            result: None,
            history: Vec::new(),
            chosen: None,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }

    /// List the meanings of a word (the one looked up before goes to the
    /// history of Back).
    pub fn look_up(&mut self, word: &str, cx: &mut Context<Self>) {
        let word = word.trim().to_string();
        if !self.word.is_empty() && self.word != word {
            self.history.push(std::mem::take(&mut self.word));
        }
        self.show(word, cx);
    }

    fn show(&mut self, word: String, cx: &mut Context<Self>) {
        self.result = match self.app.upgrade() {
            Some(app) if !word.is_empty() => Some(system::thesaurus(app.read(cx)).lookup(&word)),
            _ => None,
        };
        self.chosen = None;
        self.scroll = ScrollHandle::new();
        self.search.update(cx, |field, cx| field.set_text(word.clone(), cx));
        self.word = word;
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(word) = self.history.pop() {
            self.show(word, cx);
        }
    }

    pub fn choose(&mut self, word: &str, cx: &mut Context<Self>) {
        self.chosen = Some(word.to_string());
        cx.notify();
    }

    pub fn insert_chosen(&mut self, cx: &mut Context<Self>) {
        if let (Some(word), Some(app)) = (self.chosen.clone(), self.app.upgrade()) {
            app.update(cx, |app, cx| insert(app, &word, cx));
        }
    }

    pub fn copy_chosen(&mut self, cx: &mut Context<Self>) {
        if let Some(word) = self.chosen.clone() {
            cx.write_to_clipboard(ClipboardItem::new_string(word));
        }
    }

    /// One word of a list: a press chooses it, a second one looks it up.
    fn word_row(&self, id: usize, word: &str, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let chosen = self.chosen.as_deref() == Some(word);
        let text = word.to_string();
        div()
            .id(("thesaurus-word", id))
            .debug_selector(move || format!("thesaurus-word-{id}"))
            .pl(px(22.))
            .pr(px(8.))
            .py(px(2.))
            .rounded(px(3.))
            .when(chosen, |d| d.bg(c(theme::ACCENT_LIGHT)))
            .when(!chosen, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .child(SharedString::from(word.to_string()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count >= 2 {
                        this.look_up(&text, cx);
                    } else {
                        this.choose(&text, cx);
                    }
                }),
            )
    }

    fn results(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let muted = |text: String| div().px(px(4.)).text_color(c(theme::TEXT_MUTED)).child(text).into_any_element();
        let plain = |text: &'static str| div().px(px(4.)).child(text).into_any_element();
        match &self.result {
            None => Vec::new(),
            Some(Synonyms::NoThesaurus) => vec![
                plain(t("Proofing not available", "Aucun correcteur disponible")),
                muted(
                    t(
                        "No thesaurus is installed on this Mac. macOS has thesauruses for English only: turn one on in the Dictionary app (Dictionary ▸ Settings), then look the word up again.",
                        "Aucun dictionnaire des synonymes n'est installé sur ce Mac. macOS n'en propose que pour l'anglais : activez-en un dans l'app Dictionnaire (Dictionnaire ▸ Réglages), puis relancez la recherche.",
                    )
                    .to_string(),
                ),
            ],
            Some(Synonyms::NotFound) => {
                let sources = self.app.upgrade().map(|app| system::thesaurus(app.read(cx)).sources()).unwrap_or_default();
                vec![
                    plain(t("No results were found.", "Aucun résultat trouvé.")),
                    muted(
                        t(
                            "Synonyms come from the thesauruses installed on this Mac ({}). macOS has none for other languages than English.",
                            "Les synonymes proviennent des dictionnaires des synonymes installés sur ce Mac ({}). macOS n'en propose pas pour d'autres langues que l'anglais.",
                        )
                        .replace("{}", &sources.join(", ")),
                    ),
                ]
            }
            Some(Synonyms::Found { senses, .. }) => {
                let mut rows = Vec::new();
                let mut id = 0;
                for sense in senses {
                    rows.push(
                        div()
                            .px(px(4.))
                            .pt(px(6.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(heading(sense))
                            .into_any_element(),
                    );
                    if !sense.example.is_empty() {
                        rows.push(div().pl(px(22.)).pr(px(8.)).italic().text_color(c(theme::TEXT_MUTED)).child(sense.example.clone()).into_any_element());
                    }
                    for word in &sense.synonyms {
                        rows.push(self.word_row(id, word, cx).into_any_element());
                        id += 1;
                    }
                    if !sense.antonyms.is_empty() {
                        rows.push(div().pl(px(12.)).pt(px(3.)).text_color(c(theme::TEXT_MUTED)).child(t("antonyms", "antonymes")).into_any_element());
                    }
                    for word in &sense.antonyms {
                        rows.push(self.word_row(id, word, cx).into_any_element());
                        id += 1;
                    }
                }
                rows
            }
        }
    }
}

impl Render for ThesaurusPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.results(cx);
        let none = self.chosen.is_none();
        let headword = match &self.result {
            Some(Synonyms::Found { headword, .. }) if !headword.is_empty() => headword.clone(),
            _ => self.word.clone(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .line_height(px(18.))
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .pb(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        Button::new("thesaurus-back", t("Back", "Précédent"))
                            .width(40.0)
                            .disabled(self.history.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(TextField::new(&self.search))),
            )
            .when(!headword.is_empty(), |d| {
                d.child(div().flex_none().px(px(16.)).pb(px(4.)).text_size(px(15.)).child(headword))
            })
            .child(
                div()
                    .id("thesaurus-results")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .px(px(12.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .children(rows),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("thesaurus-insert", t("Insert", "Insérer"))
                            .primary()
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.insert_chosen(cx))),
                    )
                    .child(
                        Button::new("thesaurus-copy", t("Copy", "Copier"))
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_chosen(cx))),
                    ),
            )
    }
}

/// Review ▸ Thesaurus: open the pane on the word of the active cell.
pub(super) fn open(app: &mut ExcelApp, cx: &mut Cx) {
    let word = word_of_active_cell(app);
    let pane = match app.ext::<ThesaurusUi>().and_then(|ui| ui.pane.clone()).filter(|_| app.pane_open(PANE)) {
        Some(pane) => pane,
        None => {
            let weak = cx.entity().downgrade();
            let pane = cx.new(|cx| ThesaurusPane::new(weak, cx));
            app.ext_mut::<ThesaurusUi>().pane = Some(pane.clone());
            pane
        }
    };
    app.open_pane(PANE, t("Thesaurus", "Dictionnaire des synonymes"), pane.clone(), cx);
    // The pane reads the application to find its thesaurus: once this
    // command has let go of it.
    cx.defer(move |cx| pane.update(cx, |pane, cx| pane.look_up(&word, cx)));
}
