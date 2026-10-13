//! Review ▸ Translate: the Translator pane. The text of the selected cell
//! on top, its translation below, the two languages, and Insert, which
//! puts the translation in the cell.
//!
//! The translating is macOS's own (`system::Translator`), done on the
//! Mac with the languages downloaded in System Settings. Where the system
//! cannot translate, the pane opens all the same and says why.

use super::system::{self, TranslateError, Translation, Translator};
use crate::app::ExcelApp;
use crate::features::preferences::settings;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::{t, t_ctx};
use crate::theme::{self, c};
use crate::widgets::text_area::{AreaEvent, TextArea};
use crate::widgets::{Button, Select, label};
use gpui::{Context, Entity, Focusable, MouseButton, MouseDownEvent, SharedString, Subscription, Task, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use std::sync::Arc;
use std::time::Duration;

pub(super) const PANE: &str = "proofing-translator";

/// How long the pane waits after a key before it translates what is typed.
const PAUSE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct TranslateUi {
    pub pane: Option<Entity<TranslatePane>>,
}

/// Where the translation of the text shown stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum State {
    /// Nothing to translate.
    Empty,
    Working,
    Done(Translation),
    Failed(TranslateError),
}

pub(super) struct TranslatePane {
    app: WeakEntity<ExcelApp>,
    translator: Arc<dyn Translator>,
    source: Entity<TextArea>,
    /// The cell text the box was last filled with: the box follows the
    /// selection, not what is typed in it.
    from_cell: String,
    /// The languages of the system, sorted by name; `None` until known.
    pub languages: Option<Result<Vec<String>, TranslateError>>,
    /// `None`: find out what language the text is in.
    pub from: Option<String>,
    pub to: String,
    pub state: State,
    /// The translation under way; dropping it gives it up.
    task: Option<Task<()>>,
    _subscription: Subscription,
}

/// What the pane says when the system cannot translate.
pub(super) fn explain(error: &TranslateError) -> String {
    match error {
        TranslateError::NoHelper => t(
            "Translation is not available: the translation helper of OpenSheet (opensheet-translate) is not installed with the application.",
            "La traduction n'est pas disponible : l'assistant de traduction d'OpenSheet (opensheet-translate) n'est pas installé avec l'application.",
        )
        .to_string(),
        TranslateError::UnsupportedSystem => t(
            "Translation is not available: it needs the translation built into macOS 26 or later.",
            "La traduction n'est pas disponible : elle nécessite la traduction intégrée à macOS 26 ou version ultérieure.",
        )
        .to_string(),
        TranslateError::NotInstalled => t(
            "The languages of this translation are not downloaded on this Mac.",
            "Les langues de cette traduction ne sont pas téléchargées sur ce Mac.",
        )
        .to_string(),
        TranslateError::UnsupportedPair => t(
            "Translation is not available for the selected language pair",
            "La fonctionnalité de traduction dans le couple de langues sélectionné n'est pas disponible",
        )
        .to_string(),
        TranslateError::UnknownLanguage => t(
            "The language of this text could not be detected. Choose it under From.",
            "La langue de ce texte n'a pas pu être détectée. Choisissez-la sous De.",
        )
        .to_string(),
        TranslateError::Failed(detail) => t("The translation failed: {}", "La traduction a échoué : {}").replace("{}", detail),
    }
}

/// Where the languages macOS translates with are downloaded.
pub(super) fn where_languages_are() -> &'static str {
    t(
        "Translated on this Mac by macOS. Languages are downloaded in System Settings ▸ General ▸ Language & Region ▸ Translation Languages.",
        "Traduction effectuée sur ce Mac par macOS. Les langues se téléchargent dans Réglages Système ▸ Général ▸ Langue et région ▸ Langues de traduction.",
    )
}

/// The text of the active cell, as it shows.
fn active_cell_text(app: &ExcelApp) -> String {
    let (row, col) = app.active();
    app.wb.display_text(app.sheet, row, col).text
}

/// Put the translation in the active cell, as one undo step.
pub(super) fn insert(app: &mut ExcelApp, text: &str, cx: &mut Cx) -> bool {
    let (row, col) = app.active();
    let cell = Rect::cell(row, col);
    if !app.may(Intent::EditCells(cell), cx) {
        return false;
    }
    let sheet = app.sheet;
    // A translation is a text, whatever it looks like.
    let typed = if text.starts_with(['=', '+', '-', '\'']) { format!("'{text}") } else { text.to_string() };
    if app.wb.set_cell_text(sheet, row, col, &typed).is_err() {
        return false;
    }
    app.commit_cells(t("Translate", "Traduire"), &[cell], ChangeCause::Typing, cx);
    true
}

impl TranslatePane {
    fn new(app: WeakEntity<ExcelApp>, translator: Arc<dyn Translator>, text: String, cx: &mut Context<Self>) -> Self {
        let source = cx.new(|cx| {
            let mut area = TextArea::field(cx);
            area.max_height = Some(108.0);
            area.set_text(text.clone(), cx);
            area
        });
        let subscription = cx.subscribe(&source, |this: &mut TranslatePane, _, event: &AreaEvent, cx| match event {
            AreaEvent::Changed => this.translate_after(PAUSE, cx),
            AreaEvent::Submit => this.translate_after(Duration::ZERO, cx),
            _ => {}
        });
        let kept = settings::get();
        let mut pane = TranslatePane {
            app,
            translator,
            source,
            from_cell: text,
            languages: None,
            from: Some(kept.translate_from).filter(|language| !language.is_empty()),
            to: if kept.translate_to.is_empty() { crate::i18n::lang().code().to_string() } else { kept.translate_to },
            state: State::Empty,
            task: None,
            _subscription: subscription,
        };
        pane.load_languages(cx);
        pane
    }

    /// Ask the system what it translates, then translate what is shown.
    fn load_languages(&mut self, cx: &mut Context<Self>) {
        let translator = self.translator.clone();
        let asked = cx.background_executor().spawn(async move { translator.languages() });
        self.task = Some(cx.spawn(async move |this, cx| {
            let answer = asked.await;
            this.update(cx, |this, cx| {
                let answer = answer.map(|mut languages| {
                    languages.sort_by_key(|code| system::language_name(code).to_lowercase());
                    languages
                });
                // A language kept from another Mac may not exist here.
                if let Ok(languages) = &answer {
                    if this.from.as_ref().is_some_and(|from| !languages.contains(from)) {
                        this.from = None;
                    }
                    if !languages.contains(&this.to) {
                        this.to = languages.iter().find(|code| code.as_str() == "en").or(languages.first()).cloned().unwrap_or_default();
                    }
                }
                this.languages = Some(answer);
                this.translate_after(Duration::ZERO, cx);
            })
            .ok();
        }));
    }

    pub fn source_text(&self, cx: &gpui::App) -> String {
        self.source.read(cx).text().to_string()
    }

    /// Translate the text of the box once `pause` has passed without
    /// another request.
    pub fn translate_after(&mut self, pause: Duration, cx: &mut Context<Self>) {
        let text = self.source_text(cx);
        if let Some(Err(error)) = &self.languages {
            self.state = State::Failed(error.clone());
            self.task = None;
        } else if self.languages.is_none() {
            self.state = State::Working;
        } else if text.trim().is_empty() {
            self.state = State::Empty;
            self.task = None;
        } else {
            self.state = State::Working;
            let translator = self.translator.clone();
            let (from, to) = (self.from.clone(), self.to.clone());
            self.task = Some(cx.spawn(async move |this, cx| {
                if !pause.is_zero() {
                    cx.background_executor().timer(pause).await;
                }
                let answer = cx.background_executor().spawn(async move { translator.translate(&text, from.as_deref(), &to) }).await;
                this.update(cx, |this, cx| {
                    this.state = match answer {
                        Ok(translation) => State::Done(translation),
                        Err(error) => State::Failed(error),
                    };
                    cx.notify();
                })
                .ok();
            }));
        }
        cx.notify();
    }

    /// The selection or the cell changed: the box takes the text of the
    /// active cell when that is another text than the one it took last.
    pub fn follow(&mut self, text: String, cx: &mut Context<Self>) {
        if text == self.from_cell {
            return;
        }
        self.from_cell = text.clone();
        self.source.update(cx, |area, cx| area.set_text(text, cx));
        self.translate_after(Duration::ZERO, cx);
    }

    /// Choose the language translated from (`None`: detect it).
    pub fn set_from(&mut self, from: Option<String>, cx: &mut Context<Self>) {
        settings::update(|settings| settings.translate_from = from.clone().unwrap_or_default());
        self.from = from;
        self.translate_after(Duration::ZERO, cx);
    }

    pub fn set_to(&mut self, to: String, cx: &mut Context<Self>) {
        settings::update(|settings| settings.translate_to = to.clone());
        self.to = to;
        self.translate_after(Duration::ZERO, cx);
    }

    pub fn insert(&mut self, cx: &mut Context<Self>) {
        let (State::Done(translation), Some(app)) = (&self.state, self.app.upgrade()) else {
            return;
        };
        let text = translation.text.clone();
        // The cell now holds the translation: it is not to be translated
        // in its turn.
        if app.update(cx, |app, cx| insert(app, &text, cx)) {
            self.from_cell = text;
        }
    }

    /// A box of the pane: white, framed.
    fn frame() -> gpui::Div {
        div()
            .w_full()
            .min_h(px(84.))
            .p(px(6.))
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(0xABABAB))
            .rounded(px(3.))
            .line_height(px(17.))
            .overflow_hidden()
    }
}

impl Render for TranslatePane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let languages: Vec<String> = match &self.languages {
            Some(Ok(languages)) => languages.clone(),
            _ => Vec::new(),
        };
        let names: Vec<SharedString> = languages.iter().map(|code| system::language_name(code).into()).collect();
        let mut from_names = vec![SharedString::from(t("Auto-Detect", "Détecter automatiquement"))];
        from_names.extend(names.iter().cloned());
        let from_index = match &self.from {
            None => Some(0),
            Some(from) => languages.iter().position(|code| code == from).map(|index| index + 1),
        };
        let to_index = languages.iter().position(|code| *code == self.to);
        let unavailable = languages.is_empty();
        let from_codes = languages.clone();
        let to_codes = languages.clone();
        let field = self.source.clone();
        let source_box = Self::frame()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                window.focus(&field.read(cx).focus_handle(cx));
                field.update(cx, |field, cx| field.caret_to_point(event.position, cx));
            })
            .child(self.source.clone());
        let muted = |text: String| div().text_color(c(theme::TEXT_MUTED)).child(text);
        let (translation, detected, note) = match &self.state {
            State::Empty => (div(), None, None),
            State::Working => (muted(t("Translating…", "Traduction en cours…").to_string()), None, None),
            State::Done(translation) => {
                let detected = self.from.is_none().then(|| system::language_name(&translation.source));
                (div().child(translation.text.clone()), detected, None)
            }
            State::Failed(error) => (div(), None, Some(explain(error))),
        };
        let done = matches!(self.state, State::Done(_));
        div()
            .id("translator")
            .size_full()
            .overflow_y_scroll()
            .px(px(12.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .line_height(px(17.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t_ctx("translate", "From", "De")))
                    .child(
                        Select::new("translate-from", from_names, from_index)
                            .width(190.0)
                            .disabled(unavailable)
                            .on_change(cx.listener(move |this, index: &usize, _, cx| {
                                let from = index.checked_sub(1).and_then(|index| from_codes.get(index).cloned());
                                this.set_from(from, cx);
                            })),
                    ),
            )
            .when_some(detected, |d, language| {
                d.child(muted(t("Detected: {}", "Langue détectée : {}").replace("{}", &language)))
            })
            .child(source_box)
            .child(
                div()
                    .pt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t_ctx("translate", "To", "À")))
                    .child(
                        Select::new("translate-to", names, to_index)
                            .width(190.0)
                            .disabled(unavailable)
                            .on_change(cx.listener(move |this, index: &usize, _, cx| {
                                if let Some(to) = to_codes.get(*index) {
                                    this.set_to(to.clone(), cx);
                                }
                            })),
                    ),
            )
            .child(Self::frame().debug_selector(|| "translation".into()).bg(c(theme::CHROME_BG)).child(translation))
            .when_some(note, |d, note| d.child(div().text_color(c(0xA4262C)).child(note)))
            .child(
                div().pt(px(4.)).flex().child(
                    Button::new("translate-insert", t("Insert", "Insérer"))
                        .primary()
                        .disabled(!done)
                        .on_click(cx.listener(|this, _, _, cx| this.insert(cx))),
                ),
            )
            .child(div().pt(px(8.)).child(muted(where_languages_are().to_string())))
    }
}

/// Review ▸ Translate: open the pane on the text of the active cell.
pub(super) fn open(app: &mut ExcelApp, cx: &mut Cx) {
    let text = active_cell_text(app);
    match app.ext::<TranslateUi>().and_then(|ui| ui.pane.clone()).filter(|_| app.pane_open(PANE)) {
        Some(pane) => {
            let again = pane.clone();
            cx.defer(move |cx| again.update(cx, |pane, cx| pane.follow(text, cx)));
            app.open_pane(PANE, t("Translator", "Traducteur"), pane, cx);
        }
        None => {
            let weak = cx.entity().downgrade();
            let translator = system::translator(app);
            let pane = cx.new(|cx| TranslatePane::new(weak, translator, text, cx));
            app.ext_mut::<TranslateUi>().pane = Some(pane.clone());
            app.open_pane(PANE, t("Translator", "Traducteur"), pane, cx);
        }
    }
}

/// The selection or what the active cell holds changed: the pane follows.
pub(super) fn follow(app: &mut ExcelApp, cx: &mut Cx) {
    if !app.pane_open(PANE) {
        return;
    }
    let Some(pane) = app.ext::<TranslateUi>().and_then(|ui| ui.pane.clone()) else {
        return;
    };
    let text = active_cell_text(app);
    cx.defer(move |cx| pane.update(cx, |pane, cx| pane.follow(text, cx)));
}
