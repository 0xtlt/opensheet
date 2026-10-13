//! Review ▸ Spelling (F7): the check of the sheet in front and its dialog.
//!
//! What is read, in Excel's order: the cells of the selection, or those of
//! the sheet from the active cell on (text typed in, not formulas nor
//! numbers), then the notes and comments, the text of the shapes, the
//! chart titles and the headers and footers; and, when the check did not
//! start in A1, the cells before the active cell once the user agrees to
//! go on from the top. The words and the suggestions are the system
//! checker's (`system::SpellChecker`).

use super::system::{self, SpellChecker};
use crate::app::ExcelApp;
use crate::features::preferences::settings;
use crate::features::{self, ChangeCause, Cx, Intent, charts::Charts, drawings::Drawings, object_id};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::widgets::{self, Button, ListBox, Select, TextField, dialog, label};
use gpui::{Context, Entity, FocusHandle, SharedString, Window, div, prelude::*, px};
use opensheet_engine::chart::ChartTitle;
use opensheet_engine::comment::Comment;
use opensheet_engine::page::HfVariant;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Rect, Value, Workbook};
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

// ------------------------------------------------------------------- places

/// A text of the sheet the checker reads.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Place {
    Cell(u32, u32),
    Note(u32, u32),
    /// A comment of a thread: the first one, or a reply.
    Comment(u32, u32, Option<usize>),
    /// The text of a shape, by the id of its drawing.
    Shape(u32),
    ChartTitle(u32),
    /// A header (or a footer, when the flag is set).
    Header(HfVariant, bool),
}

impl Place {
    fn cell(&self) -> Option<(u32, u32)> {
        match self {
            Place::Cell(row, col) | Place::Note(row, col) | Place::Comment(row, col, _) => Some((*row, *col)),
            _ => None,
        }
    }

    /// What changing this text is, for the sheet's protection.
    fn intent(&self) -> Intent {
        match self {
            Place::Cell(row, col) => Intent::EditCells(Rect::cell(*row, *col)),
            Place::Note(..) | Place::Comment(..) => Intent::EditComment,
            Place::Shape(id) => Intent::EditObject(Some(object_id(&Drawings, *id))),
            Place::ChartTitle(id) => Intent::EditObject(Some(object_id(&Charts, *id))),
            Place::Header(..) => Intent::ChangeSheet,
        }
    }
}

fn text_at(wb: &Workbook, sheet: usize, place: &Place) -> Option<String> {
    let sh = wb.sheets.get(sheet)?;
    match place {
        Place::Cell(row, col) => match &sh.cells.get(&(*row, *col))?.content {
            Content::Value(Value::Text(text)) => Some(text.to_string()),
            _ => None,
        },
        Place::Note(row, col) => sh.comments.get(&(*row, *col))?.as_note().map(|note| note.text()),
        Place::Comment(row, col, reply) => {
            let thread = sh.comments.get(&(*row, *col))?.as_thread()?;
            match reply {
                None => Some(thread.first.text.clone()),
                Some(index) => thread.replies.get(*index).map(|post| post.text.clone()),
            }
        }
        Place::Shape(id) => wb.drawing(sheet, *id)?.as_shape().map(|shape| shape.text.clone()),
        Place::ChartTitle(id) => match &wb.chart(sheet, *id)?.title {
            ChartTitle::Text(text) => Some(text.clone()),
            _ => None,
        },
        Place::Header(variant, footer) => {
            let both = &sh.page.header_footer;
            Some(if *footer { both.footer(*variant) } else { both.header(*variant) }.to_string())
        }
    }
}

/// Put a corrected text back. The engine takes the undo snapshot; the
/// caller commits.
fn set_text(wb: &mut Workbook, sheet: usize, place: &Place, text: &str) -> bool {
    match place {
        Place::Cell(row, col) => {
            // A text that would read as a number or a formula is typed
            // behind an apostrophe: the corrected one keeps it.
            let old = text_at(wb, sheet, place).unwrap_or_default();
            let typed = wb.edit_text(sheet, *row, *col);
            let prefix = typed.strip_suffix(old.as_str()).unwrap_or_default();
            wb.set_cell_text(sheet, *row, *col, &format!("{prefix}{text}")).is_ok()
        }
        Place::Note(row, col) => wb.set_note(sheet, *row, *col, "", text),
        Place::Comment(row, col, reply) => wb.edit_comment(sheet, *row, *col, *reply, text),
        Place::Shape(id) => wb.update_drawing(sheet, *id, |drawing| {
            if let Some(shape) = drawing.as_shape_mut() {
                shape.text = text.to_string();
            }
        }),
        Place::ChartTitle(id) => wb.update_chart(sheet, *id, |chart| chart.title = ChartTitle::Text(text.to_string())),
        Place::Header(variant, footer) => {
            if sheet >= wb.sheets.len() {
                return false;
            }
            wb.snapshot();
            let both = &mut wb.sheets[sheet].page.header_footer;
            *(if *footer { both.footer_mut(*variant) } else { both.header_mut(*variant) }) = text.to_string();
            true
        }
    }
}

/// A header or a footer with its `&` codes (`&L`, `&P`, `&"Arial,Bold"`,
/// `&14`, `&KFF0000`) blanked out, byte for byte: what is left are its
/// words, where they are in the real text.
fn without_codes(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let blank = |out: &mut String, ch: char| out.extend(std::iter::repeat_n(' ', ch.len_utf8()));
    let mut chars = code.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        blank(&mut out, ch);
        let Some(next) = chars.next() else {
            break;
        };
        blank(&mut out, next);
        match next {
            '"' => {
                for inner in chars.by_ref() {
                    blank(&mut out, inner);
                    if inner == '"' {
                        break;
                    }
                }
            }
            'K' => {
                // Six hexadecimal digits, or a theme colour and its tint.
                for _ in 0..6 {
                    match chars.peek() {
                        Some(inner) if inner.is_ascii_hexdigit() || matches!(inner, '+' | '-') => {
                            blank(&mut out, *inner);
                            chars.next();
                        }
                        _ => break,
                    }
                }
            }
            digit if digit.is_ascii_digit() => {
                while let Some(inner) = chars.peek().filter(|inner| inner.is_ascii_digit()) {
                    blank(&mut out, *inner);
                    chars.next();
                }
            }
            _ => {}
        }
    }
    out
}

/// The text the checker is shown for a place: the same length as the real
/// one, so that what it finds is where it says. The codes of a header and
/// the name that opens a note ("Ada Lovelace:") are blanked out.
fn shown_to_checker(wb: &Workbook, sheet: usize, place: &Place, text: &str) -> String {
    match place {
        Place::Header(..) => without_codes(text),
        Place::Note(row, col) => {
            let author = wb.comment_at(sheet, *row, *col).and_then(Comment::as_note).map(|note| format!("{}:", note.author));
            match author.filter(|header| header.len() > 1 && text.starts_with(header.as_str())) {
                Some(header) => format!("{}{}", " ".repeat(header.len()), &text[header.len()..]),
                None => text.to_string(),
            }
        }
        _ => text.to_string(),
    }
}

/// Words Excel passes over whatever the dictionary says: those in
/// capitals and those holding a digit.
fn passed_over(word: &str) -> bool {
    let letters: Vec<char> = word.chars().filter(|ch| ch.is_alphabetic()).collect();
    word.chars().any(|ch| ch.is_numeric()) || (letters.len() > 1 && letters.iter().all(|ch| ch.is_uppercase()))
}

/// Where `word` stands as a whole word in `text`, from `from` on.
fn whole_words(text: &str, word: &str, from: usize) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    if word.is_empty() {
        return found;
    }
    let mut at = from.min(text.len());
    while let Some(offset) = text[at..].find(word) {
        let range = at + offset..at + offset + word.len();
        let before = text[..range.start].chars().next_back();
        let after = text[range.end..].chars().next();
        if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
            found.push(range.clone());
        }
        at = range.end;
    }
    found
}

// ------------------------------------------------------------------ session

/// A word the checker stopped on.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Issue {
    pub place: Place,
    /// Where the word is in the text of the place, in bytes.
    pub range: Range<usize>,
    pub word: String,
    pub suggestions: Vec<String>,
}

/// What a check does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// `Session::current` is the next word.
    Issue,
    /// The end of the sheet was reached and the check did not start at
    /// its beginning.
    AskWrap,
    Done,
}

/// What Undo Last takes back.
#[derive(Clone, Debug, PartialEq)]
enum Done {
    Ignored,
    IgnoredAll(String),
    Learned(String),
    Changed,
    ChangedAll(String),
    /// A word replaced without asking, because of an earlier Change All.
    Automatic,
}

/// One run of the spelling check over a sheet.
pub(super) struct Session {
    sheet: usize,
    checker: Rc<dyn SpellChecker>,
    pub language: String,
    /// The texts of this pass, in order, and how far the check is.
    places: Vec<Place>,
    index: usize,
    offset: usize,
    /// The cells before the one the check started on.
    earlier: Vec<Place>,
    /// The selection the check keeps to, when it is more than a cell.
    scope: Vec<Rect>,
    /// What Change All replaces, by word.
    change_all: BTreeMap<String, String>,
    /// The steps of this pass, with where the check was before each.
    history: Vec<(usize, usize, Done)>,
    pub current: Option<Issue>,
    /// Grows with every new word shown: the dialog fills its boxes anew.
    pub serial: u64,
}

impl Session {
    pub fn new(app: &ExcelApp, checker: Rc<dyn SpellChecker>) -> Session {
        let sheet = app.sheet;
        let sh = &app.wb.sheets[sheet];
        let sel = app.sel();
        let (row, col) = sel.active;
        let one_cell = match sel.ranges.as_slice() {
            [range] => range.is_single() || sh.merge_at(row, col) == Some(*range),
            _ => false,
        };
        let is_text = |cell: &opensheet_engine::workbook::Cell| matches!(cell.content, Content::Value(Value::Text(_)));
        let text_cells = sh.cells.iter().filter(|(_, cell)| is_text(cell)).map(|(at, _)| *at);
        let (mut places, mut earlier, mut scope) = (Vec::new(), Vec::new(), Vec::new());
        if one_cell {
            // The cells are kept row by row: so does the check read them.
            for at in text_cells {
                let list = if at >= (row, col) { &mut places } else { &mut earlier };
                list.push(Place::Cell(at.0, at.1));
            }
            for (at, comment) in &sh.comments {
                match comment {
                    Comment::Note(_) => places.push(Place::Note(at.0, at.1)),
                    Comment::Thread(thread) => {
                        places.push(Place::Comment(at.0, at.1, None));
                        places.extend((0..thread.replies.len()).map(|reply| Place::Comment(at.0, at.1, Some(reply))));
                    }
                }
            }
            places.extend(sh.drawings.iter().filter(|drawing| drawing.as_shape().is_some()).map(|drawing| Place::Shape(drawing.id)));
            places.extend(sh.charts.iter().map(|chart| Place::ChartTitle(chart.id)));
            for variant in [HfVariant::Odd, HfVariant::Even, HfVariant::First] {
                places.extend([Place::Header(variant, false), Place::Header(variant, true)]);
            }
        } else {
            scope = sel.ranges.clone();
            places.extend(text_cells.filter(|at| scope.iter().any(|range| range.contains(at.0, at.1))).map(|at| Place::Cell(at.0, at.1)));
        }
        let languages = checker.languages();
        let kept = settings::get().spelling_language;
        let language = languages
            .iter()
            .position(|language| *language == kept)
            .or_else(|| system::interface_language(&languages))
            .map(|index| languages[index].clone())
            .or_else(|| languages.first().cloned())
            .unwrap_or_default();
        Session {
            sheet,
            checker,
            language,
            places,
            index: 0,
            offset: 0,
            earlier,
            scope,
            change_all: BTreeMap::new(),
            history: Vec::new(),
            current: None,
            serial: 0,
        }
    }

    pub fn languages(&self) -> Vec<String> {
        self.checker.languages()
    }

    /// Check in another language from the word shown on.
    pub fn set_language(&mut self, language: &str) {
        self.language = language.to_string();
        if let Some(issue) = self.current.take() {
            self.offset = issue.range.start;
        }
    }

    /// The first word after `from` the user is to be asked about.
    fn misspelling(&self, shown: &str, mut from: usize) -> Option<Range<usize>> {
        loop {
            let range = self.checker.next_misspelling(shown, from, &self.language)?;
            if range.is_empty() || range.end > shown.len() {
                return None;
            }
            if !passed_over(&shown[range.clone()]) {
                return Some(range);
            }
            from = range.end;
        }
    }

    /// Go on to the next word.
    pub fn advance(&mut self, app: &mut ExcelApp, cx: &mut Cx) -> Step {
        self.current = None;
        loop {
            let Some(place) = self.places.get(self.index).cloned() else {
                return if self.earlier.is_empty() { Step::Done } else { Step::AskWrap };
            };
            let found = text_at(&app.wb, self.sheet, &place).and_then(|text| {
                let shown = shown_to_checker(&app.wb, self.sheet, &place, &text);
                let range = self.misspelling(&shown, self.offset)?;
                Some((text, shown, range))
            });
            let Some((text, shown, range)) = found else {
                self.index += 1;
                self.offset = 0;
                continue;
            };
            let word = text[range.clone()].to_string();
            if let Some(to) = self.change_all.get(&word).cloned() {
                // Asked once, with Change All: replaced where the sheet
                // allows it.
                let before = (self.index, self.offset);
                self.offset = range.end;
                if features::guard(app, &place.intent()).is_none() {
                    let fixed = format!("{}{to}{}", &text[..range.start], &text[range.end..]);
                    if set_text(&mut app.wb, self.sheet, &place, &fixed) {
                        commit(app, &[place], cx);
                        self.offset = range.start + to.len();
                        self.history.push((before.0, before.1, Done::Automatic));
                    }
                }
                continue;
            }
            let suggestions = self.checker.guesses(&shown, range.clone(), &self.language);
            self.current = Some(Issue {
                place,
                range,
                word,
                suggestions,
            });
            self.serial += 1;
            return Step::Issue;
        }
    }

    /// The user agreed to go on from the beginning of the sheet.
    pub fn wrap(&mut self) {
        self.places = std::mem::take(&mut self.earlier);
        self.index = 0;
        self.offset = 0;
        // Undo Last does not reach back over the end of the sheet.
        self.history.clear();
    }

    fn done(&mut self, what: Done) {
        self.history.push((self.index, self.offset, what));
    }

    pub fn ignore_once(&mut self) {
        if let Some(issue) = self.current.take() {
            self.done(Done::Ignored);
            self.offset = issue.range.end;
        }
    }

    pub fn ignore_all(&mut self) {
        if let Some(issue) = self.current.take() {
            self.checker.ignore(&issue.word);
            self.done(Done::IgnoredAll(issue.word));
            self.offset = issue.range.end;
        }
    }

    pub fn add_to_dictionary(&mut self) {
        if let Some(issue) = self.current.take() {
            self.checker.learn(&issue.word);
            self.done(Done::Learned(issue.word));
            self.offset = issue.range.end;
        }
    }

    /// Replace the word shown by `to` (everywhere in what is left to check
    /// with `all`), as one undo step. False when the sheet's protection
    /// refuses: it said why, and the word stays.
    pub fn change(&mut self, app: &mut ExcelApp, to: &str, all: bool, cx: &mut Cx) -> bool {
        let Some(issue) = self.current.clone() else {
            return false;
        };
        if to == issue.word || !app.may(issue.place.intent(), cx) {
            return false;
        }
        let mut changed = Vec::new();
        let last = if all { self.places.len() } else { self.index + 1 };
        for index in self.index..last {
            let place = self.places[index].clone();
            let here = index == self.index;
            // Elsewhere, what is protected is left as it is.
            if !here && features::guard(app, &place.intent()).is_some() {
                continue;
            }
            let Some(text) = text_at(&app.wb, self.sheet, &place) else {
                continue;
            };
            let mut ranges = match (here, all) {
                (true, false) => vec![issue.range.clone()],
                (true, true) => whole_words(&shown_to_checker(&app.wb, self.sheet, &place, &text), &issue.word, issue.range.start),
                _ => whole_words(&shown_to_checker(&app.wb, self.sheet, &place, &text), &issue.word, 0),
            };
            if here && !ranges.contains(&issue.range) {
                ranges.insert(0, issue.range.clone());
            }
            if ranges.is_empty() {
                continue;
            }
            let mut fixed = text.clone();
            for range in ranges.iter().rev() {
                fixed.replace_range(range.clone(), to);
            }
            if set_text(&mut app.wb, self.sheet, &place, &fixed) {
                changed.push(place);
            }
        }
        if changed.is_empty() {
            return false;
        }
        commit(app, &changed, cx);
        if all {
            self.change_all.insert(issue.word.clone(), to.to_string());
            self.done(Done::ChangedAll(issue.word));
        } else {
            self.done(Done::Changed);
        }
        self.current = None;
        self.offset = issue.range.start + to.len();
        true
    }

    pub fn can_undo_last(&self) -> bool {
        !self.history.is_empty()
    }

    /// Undo Last: take the last answer back and show its word again.
    pub fn undo_last(&mut self, app: &mut ExcelApp, cx: &mut Cx) -> bool {
        loop {
            let Some((index, offset, what)) = self.history.pop() else {
                return false;
            };
            (self.index, self.offset) = (index, offset);
            self.current = None;
            match what {
                Done::Ignored => {}
                Done::IgnoredAll(word) => self.checker.unignore(&word),
                Done::Learned(word) => self.checker.unlearn(&word),
                Done::Changed => app.undo(cx),
                Done::ChangedAll(word) => {
                    self.change_all.remove(&word);
                    app.undo(cx);
                }
                // Not an answer of the user: the one before it is.
                Done::Automatic => {
                    app.undo(cx);
                    continue;
                }
            }
            return true;
        }
    }
}

/// Close the undo step of a correction.
fn commit(app: &mut ExcelApp, places: &[Place], cx: &mut Cx) {
    let label = t("Spelling", "Orthographe");
    let cells: Vec<Rect> = places
        .iter()
        .filter_map(|place| match place {
            Place::Cell(row, col) => Some(Rect::cell(*row, *col)),
            _ => None,
        })
        .collect();
    if cells.is_empty() {
        app.commit(label, cx);
    } else {
        app.commit_cells(label, &cells, ChangeCause::Typing, cx);
    }
}

// --------------------------------------------------------------- the check

/// The check in progress and its dialog.
#[derive(Default)]
pub(super) struct Spelling {
    pub session: Option<Session>,
    pub dialog: Option<(Entity<SpellingDialog>, FocusHandle)>,
}

/// Run something on the session with the application at hand.
fn with_session<R>(app: &mut ExcelApp, f: impl FnOnce(&mut Session, &mut ExcelApp) -> R) -> Option<R> {
    let mut session = app.ext_mut::<Spelling>().session.take()?;
    let result = f(&mut session, app);
    app.ext_mut::<Spelling>().session = Some(session);
    Some(result)
}

/// Review ▸ Spelling.
pub(super) fn start(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let checker = system::spell_checker(app);
    if checker.languages().is_empty() {
        let message = t("Cannot initialize spelling checker.", "Impossible d'initialiser le vérificateur d'orthographe.");
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
        return;
    }
    let session = Session::new(app, checker);
    *app.ext_mut::<Spelling>() = Spelling {
        session: Some(session),
        dialog: None,
    };
    proceed(app, window, cx);
}

/// Show the next word, ask about going on from the top, or finish.
fn proceed(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(step) = with_session(app, |session, app| session.advance(app, cx)) else {
        return;
    };
    match step {
        Step::Issue => {
            show_place(app, cx);
            match &app.ext_mut::<Spelling>().dialog {
                // (The dialog may be the one asking: it is told to draw
                // itself again without being borrowed.)
                Some((dialog, _)) => gpui::App::notify(cx, dialog.entity_id()),
                None => {
                    let languages = with_session(app, |session, _| session.languages()).unwrap_or_default();
                    let dialog = app.open_modal(window, cx, |modal, _, cx| SpellingDialog::new(modal, languages, cx));
                    let frame = dialog.read(cx).modal.focus.clone();
                    app.ext_mut::<Spelling>().dialog = Some((dialog, frame));
                }
            }
        }
        Step::AskWrap => {
            let question = t(
                "Do you want to continue checking at the beginning of the sheet?",
                "Voulez-vous continuer la vérification au début de la feuille?",
            );
            let alert = Alert::new("OpenSheet", question)
                .button(t("Yes", "Oui"), |app, window, cx| {
                    with_session(app, |session, _| session.wrap());
                    proceed(app, window, cx);
                })
                .button(t("No", "Non"), |app, window, cx| finish(app, true, window, cx));
            app.alert(alert, cx);
        }
        Step::Done => finish(app, true, window, cx),
    }
}

/// End the check: the dialog goes and, unless the user gave up, Excel's
/// word of the end.
fn finish(app: &mut ExcelApp, complete: bool, window: &mut Window, cx: &mut Cx) {
    let spelling = std::mem::take(app.ext_mut::<Spelling>());
    if let Some((_, frame)) = spelling.dialog {
        app.close_modal_of(&frame, window, cx);
    }
    if complete {
        let message = t("Spell check complete. You're good to go!", "La vérification de l'orthographe est terminée.");
        app.alert(Alert::new("OpenSheet", message), cx);
    }
}

/// Bring what holds the word shown into view: its cell becomes the active
/// cell (inside the selection when the check keeps to it), its shape or
/// its chart is selected.
fn show_place(app: &mut ExcelApp, cx: &mut Cx) {
    let Some((place, scoped)) = app
        .ext::<Spelling>()
        .and_then(|spelling| spelling.session.as_ref())
        .and_then(|session| Some((session.current.as_ref()?.place.clone(), !session.scope.is_empty())))
    else {
        return;
    };
    match place {
        Place::Shape(id) => app.select_object(Some(object_id(&Drawings, id)), cx),
        Place::ChartTitle(id) => app.select_object(Some(object_id(&Charts, id)), cx),
        Place::Header(..) => {}
        _ => {
            let Some((row, col)) = place.cell() else {
                return;
            };
            if app.active() == (row, col) {
                return;
            }
            if scoped {
                let sheet = app.sheet;
                app.views[sheet].sel.active = (row, col);
                app.ensure_visible(row, col);
                app.refresh(cx);
            } else {
                app.select_cell(row, col, cx);
            }
        }
    }
}

// ------------------------------------------------------------------- dialog

pub(super) struct SpellingDialog {
    modal: Modal,
    /// "Not in Dictionary": the word, which may be corrected by hand.
    word: Entity<TextInput>,
    languages: Vec<String>,
    selected: Option<usize>,
    /// The `Session::serial` the boxes were filled for.
    shown: u64,
}

/// What the dialog shows of the session.
struct Shown {
    serial: u64,
    word: String,
    suggestions: Vec<String>,
    language: String,
    can_undo: bool,
}

impl SpellingDialog {
    fn new(modal: Modal, languages: Vec<String>, cx: &mut Context<Self>) -> Self {
        SpellingDialog {
            modal,
            word: widgets::input(cx, ""),
            languages,
            selected: None,
            shown: 0,
        }
    }

    fn shown(&self, cx: &Context<Self>) -> Option<Shown> {
        self.modal
            .read_app(cx, |app| {
                let session = app.ext::<Spelling>()?.session.as_ref()?;
                let issue = session.current.as_ref()?;
                Some(Shown {
                    serial: session.serial,
                    word: issue.word.clone(),
                    suggestions: issue.suggestions.clone(),
                    language: session.language.clone(),
                    can_undo: session.can_undo_last(),
                })
            })
            .flatten()
    }

    /// What Change puts in place of the word: what was typed over it,
    /// else the suggestion chosen.
    fn replacement(&self, shown: &Shown, cx: &Context<Self>) -> Option<String> {
        let typed = self.word.read(cx).text().to_string();
        if typed != shown.word {
            return Some(typed);
        }
        self.selected.and_then(|index| shown.suggestions.get(index).cloned())
    }

    /// Answer for the word shown and go on to the next.
    fn answer(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Session, &mut ExcelApp, &mut Cx) -> bool) {
        self.modal.update_app(cx, |app, cx| {
            if with_session(app, |session, app| f(session, app, cx)) == Some(true) {
                proceed(app, window, cx);
            }
        });
        cx.notify();
    }

    pub(super) fn ignore_once(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(window, cx, |session, _, _| {
            session.ignore_once();
            true
        });
    }

    pub(super) fn ignore_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(window, cx, |session, _, _| {
            session.ignore_all();
            true
        });
    }

    pub(super) fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(window, cx, |session, _, _| {
            session.add_to_dictionary();
            true
        });
    }

    pub(super) fn change(&mut self, all: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(to) = self.shown(cx).and_then(|shown| self.replacement(&shown, cx)) else {
            return;
        };
        self.answer(window, cx, move |session, app, cx| session.change(app, &to, all, cx));
    }

    pub(super) fn undo_last(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.answer(window, cx, |session, app, cx| session.undo_last(app, cx));
    }

    pub(super) fn set_language(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(language) = self.languages.get(index).cloned() else {
            return;
        };
        settings::update(|settings| settings.spelling_language = language.clone());
        self.answer(window, cx, move |session, _, _| {
            session.set_language(&language);
            true
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| finish(app, false, window, cx));
    }

    #[cfg(test)]
    pub(super) fn type_word(&mut self, text: &str, cx: &mut Context<Self>) {
        self.word.update(cx, |word, cx| word.set_text(text, cx));
    }

    #[cfg(test)]
    pub(super) fn select_suggestion(&mut self, index: usize) {
        self.selected = Some(index);
    }
}

impl Render for SpellingDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown(cx);
        if let Some(shown) = shown.as_ref().filter(|shown| shown.serial != self.shown) {
            self.shown = shown.serial;
            self.selected = if shown.suggestions.is_empty() { None } else { Some(0) };
            let word = shown.word.clone();
            self.word.update(cx, |field, cx| field.set_text(word, cx));
        }
        let none = shown.is_none();
        let suggestions: Vec<SharedString> = match &shown {
            Some(shown) if !shown.suggestions.is_empty() => shown.suggestions.iter().map(|word| SharedString::from(word.clone())).collect(),
            _ => vec![t("(No Suggestions)", "(Aucune suggestion)").into()],
        };
        let has_suggestions = shown.as_ref().is_some_and(|shown| !shown.suggestions.is_empty());
        let can_change = shown.as_ref().is_some_and(|shown| self.replacement(shown, cx).is_some());
        let can_undo = shown.as_ref().is_some_and(|shown| shown.can_undo);
        let language = shown.as_ref().and_then(|shown| self.languages.iter().position(|language| *language == shown.language));
        let names: Vec<SharedString> = self.languages.iter().map(|code| system::language_name(code).into()).collect();
        const BUTTON: f32 = 176.0;
        let button = |id: &'static str, text: &'static str, off: bool| Button::new(id, text).width(BUTTON).disabled(off);
        let left = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(t("Not in Dictionary:", "Absent du dictionnaire :")))
            .child(TextField::new(&self.word).width(300.0))
            .child(label(t("Suggestions:", "Suggestions :")))
            .child(
                ListBox::new("spelling-suggestions", suggestions, if has_suggestions { self.selected } else { None })
                    .width(300.0)
                    .height(132.0)
                    .on_change(cx.listener(move |this, index: &usize, _, cx| {
                        if has_suggestions {
                            this.selected = Some(*index);
                            cx.notify();
                        }
                    }))
                    .on_activate(cx.listener(move |this, index: &usize, window, cx| {
                        if has_suggestions {
                            this.selected = Some(*index);
                            this.change(false, window, cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Dictionary language:", "Langue du dictionnaire :")))
                    .child(
                        Select::new("spelling-language", names, language)
                            .width(150.0)
                            .on_change(cx.listener(|this, index: &usize, window, cx| this.set_language(*index, window, cx))),
                    ),
            );
        let right = div()
            .pt(px(22.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(button("spelling-ignore", t("Ignore Once", "Ignorer une fois"), none).on_click(cx.listener(|this, _, window, cx| this.ignore_once(window, cx))))
            .child(button("spelling-ignore-all", t("Ignore All", "Ignorer tout"), none).on_click(cx.listener(|this, _, window, cx| this.ignore_all(window, cx))))
            .child(
                button("spelling-add", t("Add to Dictionary", "Ajouter au dictionnaire"), none)
                    .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
            )
            .child(div().h(px(10.)))
            .child(
                button("spelling-change", t_change(), !can_change)
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.change(false, window, cx))),
            )
            .child(
                button("spelling-change-all", t("Change All", "Remplacer tout"), !can_change)
                    .on_click(cx.listener(|this, _, window, cx| this.change(true, window, cx))),
            );
        let footer = div()
            .flex()
            .justify_end()
            .gap(px(8.))
            .child(
                Button::new("spelling-undo", t("Undo Last", "Annuler dernière action"))
                    .disabled(!can_undo)
                    .on_click(cx.listener(|this, _, window, cx| this.undo_last(window, cx))),
            )
            .child(Button::new("spelling-cancel", t("Cancel", "Annuler")).width(86.0).on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))));
        dialog(&self.modal, t("Spelling", "Orthographe"))
            .width(530.0)
            .on_ok(cx.listener(|this, _, window, cx| this.change(false, window, cx)))
            .on_cancel(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
            .child(div().flex().gap(px(14.)).child(left).child(right))
            .footer(footer)
    }
}

/// "Change", the button of this dialog (Excel has other words for other
/// changes).
fn t_change() -> &'static str {
    crate::i18n::t_ctx("spelling", "Change", "Modifier")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_codes_are_not_words() {
        let code = "&L&\"Arial,Bold\"&14Budjet&C&KFF0000Page &P of &N&R&D && more";
        let shown = without_codes(code);
        assert_eq!(shown.len(), code.len());
        let words: Vec<&str> = shown.split_whitespace().collect();
        assert_eq!(words, vec!["Budjet", "Page", "of", "more"]);
        // What is left is where it was.
        let at = shown.find("Budjet").unwrap();
        assert_eq!(&code[at..at + 6], "Budjet");
        assert_eq!(without_codes("Résumé &"), "Résumé  ");
        assert_eq!(without_codes(""), "");
    }

    #[test]
    fn capitals_and_digits_are_passed_over() {
        assert!(passed_over("NASA"));
        assert!(passed_over("Q3"));
        assert!(passed_over("abc123"));
        assert!(!passed_over("Nasa"));
        assert!(!passed_over("I"));
        assert!(!passed_over("tset"));
    }

    #[test]
    fn whole_words_are_found() {
        let text = "teh cat, tehran; (teh) Teh teh";
        let found: Vec<usize> = whole_words(text, "teh", 0).into_iter().map(|range| range.start).collect();
        assert_eq!(found, vec![0, 18, 27]);
        assert_eq!(whole_words(text, "teh", 1).len(), 2);
        assert!(whole_words(text, "", 0).is_empty());
    }
}
