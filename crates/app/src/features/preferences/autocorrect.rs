//! AutoCorrect: what Excel changes by itself in the text typed into a
//! cell, as the AutoCorrect pane of Preferences sets it.
//!
//! * "Replace text as you type": a word of the list is replaced when it
//!   ends (a space or a punctuation mark is typed after it, or the entry
//!   is confirmed); an entry of the list that ends with a sign, such as
//!   "(c)", as soon as its last character is typed.
//! * "Correct TWo INitial CApitals", "Capitalize first letter of
//!   sentences", "Capitalize names of days": the same moments.
//!
//! Formulas are never corrected. ⌘Z while typing takes the last
//! correction back, and the word is then left alone until the entry is
//! confirmed; a correction made when the entry is confirmed is an undo
//! step of its own, so that one Undo gives back what was typed.
//!
//! The starting list is OpenSheet's own, kept short on purpose: the signs
//! everybody knows and a few slips per language.

use super::settings::{self, Settings};
use crate::app::ExcelApp;
use crate::features::{CellEdit, ChangeCause, Cx};
use crate::i18n::{Lang, lang, t};
use opensheet_engine::workbook::Content;
use opensheet_engine::{Rect, Value};
use std::ops::Range;

/// The replacements a new installation starts with, in a language.
pub fn starting_list(lang: Lang) -> Vec<(String, String)> {
    const SIGNS: &[(&str, &str)] = &[
        ("(c)", "©"),
        ("(r)", "®"),
        ("(tm)", "™"),
        ("(e)", "€"),
        ("...", "…"),
        ("-->", "→"),
        ("<--", "←"),
        ("<->", "↔"),
    ];
    let words: &[(&str, &str)] = match lang {
        Lang::En => &[
            ("i", "I"),
            ("teh", "the"),
            ("adn", "and"),
            ("taht", "that"),
            ("thier", "their"),
            ("wich", "which"),
            ("untill", "until"),
            ("recieve", "receive"),
            ("seperate", "separate"),
            ("occured", "occurred"),
            ("definately", "definitely"),
        ],
        Lang::Fr => &[
            ("oeuvre", "œuvre"),
            ("coeur", "cœur"),
            ("soeur", "sœur"),
            ("oeil", "œil"),
            ("oeuf", "œuf"),
            ("boeuf", "bœuf"),
            ("noeud", "nœud"),
            ("voeu", "vœu"),
            ("deja", "déjà"),
            ("voila", "voilà"),
            ("tres", "très"),
        ],
        Lang::Es => &[
            ("qeu", "que"),
            ("tambien", "también"),
            ("despues", "después"),
            ("ademas", "además"),
            ("asi", "así"),
            ("numero", "número"),
            ("telefono", "teléfono"),
            ("informacion", "información"),
        ],
        Lang::De => &[
            ("udn", "und"),
            ("nciht", "nicht"),
            ("fuer", "für"),
            ("ueber", "über"),
            ("koennen", "können"),
            ("moeglich", "möglich"),
            ("natuerlich", "natürlich"),
        ],
        Lang::It => &[
            ("nno", "non"),
            ("qeusto", "questo"),
            ("perche", "perché"),
            ("poiche", "poiché"),
            ("piu", "più"),
            ("cosi", "così"),
            ("gia", "già"),
            ("cioe", "cioè"),
            ("citta", "città"),
        ],
    };
    SIGNS.iter().chain(words).map(|(from, to)| (from.to_string(), to.to_string())).collect()
}

/// The list in use: the user's, or the starting one of the language.
pub fn replacements(settings: &Settings) -> Vec<(String, String)> {
    settings.replacements.clone().unwrap_or_else(|| starting_list(lang()))
}

/// What to correct and how, taken from the settings once per keystroke.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rules {
    /// Empty when "Replace text as you type" is off.
    pub replacements: Vec<(String, String)>,
    pub two_capitals: bool,
    pub sentences: bool,
    /// The names of the days in small letters; empty when the box is off
    /// or the language writes them without a capital.
    pub days: Vec<String>,
}

impl Rules {
    pub fn current() -> Rules {
        let settings = settings::get();
        let lang = lang();
        // French, Spanish and Italian write the days in small letters.
        let days = if settings.capitalize_days && matches!(lang, Lang::En | Lang::De) {
            lang.locale().days.iter().map(|day| day.to_lowercase()).collect()
        } else {
            Vec::new()
        };
        Rules {
            replacements: if settings.replace_text { replacements(&settings) } else { Vec::new() },
            two_capitals: settings.correct_two_capitals,
            sentences: settings.capitalize_sentences,
            days,
        }
    }
}

/// One correction: `original`, found at `at` in the text, becomes `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    pub at: usize,
    pub original: String,
    pub text: String,
}

impl Correction {
    fn range(&self) -> Range<usize> {
        self.at..self.at + self.original.len()
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '\'' || ch == '’'
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// What an entry of the list makes of `typed`, which has its length:
/// the entry as it is, or with the capitals of what was typed ("Teh"
/// gives "The", "TEH" gives "THE").
fn replaced(typed: &str, from: &str, to: &str) -> Option<String> {
    if typed == from {
        return Some(to.to_string());
    }
    if from.chars().any(char::is_uppercase) || typed.to_lowercase() != from.to_lowercase() {
        return None;
    }
    let mut chars = typed.chars();
    let first = chars.next()?;
    let rest = chars.as_str();
    if first.is_uppercase() && !rest.chars().any(char::is_uppercase) {
        Some(capitalized(to))
    } else if rest.chars().any(char::is_alphabetic) && !typed.chars().any(char::is_lowercase) {
        Some(to.to_uppercase())
    } else {
        None
    }
}

/// The longest entry of the list that `head` ends with. `signs` chooses
/// the entries that end with a sign (replaced as soon as they are typed)
/// or those that end a word.
fn replacement_at_end(head: &str, rules: &Rules, signs: bool) -> Option<Correction> {
    let mut best: Option<Correction> = None;
    for (from, to) in &rules.replacements {
        let Some(last) = from.chars().last() else {
            continue;
        };
        if is_word_char(last) == signs {
            continue;
        }
        let count = from.chars().count();
        let Some((at, _)) = head.char_indices().rev().nth(count - 1) else {
            continue;
        };
        // The entry starts a word: "(c)" after a letter is left alone,
        // and so is "teh" at the end of a longer word.
        // "..." follows its word, but not a fourth full stop.
        let before = head[..at].chars().last();
        let first = from.chars().next().unwrap_or(last);
        let starts_word = match before {
            None => true,
            Some(before) if is_word_char(first) || matches!(first, '(' | '[' | '{') => !is_word_char(before),
            Some(before) => before != first,
        };
        if !starts_word {
            continue;
        }
        let typed = &head[at..];
        if let Some(text) = replaced(typed, from, to).filter(|text| text != typed) {
            if best.as_ref().is_none_or(|best| typed.len() > best.original.len()) {
                best = Some(Correction {
                    at,
                    original: typed.to_string(),
                    text,
                });
            }
        }
    }
    best
}

/// Short forms after whose full stop a sentence goes on.
const ABBREVIATIONS: &[&str] = &[
    "etc", "e.g", "i.e", "vs", "cf", "no", "nr", "ex", "p", "z.b", "bzw", "ca", "ecc", "es", "p.ej", "approx", "env",
];

/// Whether what precedes a word ends a sentence: a full stop, a question
/// or an exclamation mark, then a space.
fn after_sentence_end(before: &str) -> bool {
    let trimmed = before.trim_end();
    if trimmed.len() == before.len() {
        return false;
    }
    match trimmed.chars().last() {
        Some('!' | '?') => true,
        Some('.') => {
            let sentence = &trimmed[..trimmed.len() - 1];
            // "..." trails off, and a short form does not end a sentence.
            let last_word = sentence.rsplit(|ch: char| ch.is_whitespace()).next().unwrap_or("");
            !sentence.ends_with('.') && !last_word.is_empty() && !ABBREVIATIONS.contains(&last_word.to_lowercase().as_str())
        }
        _ => false,
    }
}

/// The correction of the word that ends at `end` in `text`: an entry of
/// the list first, the capitals otherwise.
pub fn correct_word(text: &str, end: usize, rules: &Rules) -> Option<Correction> {
    let head = &text[..end];
    if let Some(found) = replacement_at_end(head, rules, false) {
        return Some(found);
    }
    let at = head.char_indices().rev().take_while(|(_, ch)| is_word_char(*ch)).last().map(|(at, _)| at)?;
    let word = &head[at..];
    let mut chars: Vec<char> = word.chars().collect();
    // "MOnday" has its second capital taken away: the rest of the word is
    // in small letters, and there is a rest.
    if rules.two_capitals
        && chars.len() >= 3
        && chars[0].is_uppercase()
        && chars[1].is_uppercase()
        && chars[2..].iter().all(|ch| ch.is_lowercase())
    {
        chars[1] = chars[1].to_lowercase().next().unwrap_or(chars[1]);
    }
    let starts_small = chars.first().is_some_and(|ch| ch.is_lowercase());
    let lower: String = chars.iter().collect::<String>().to_lowercase();
    let day = rules.days.contains(&lower) && chars.iter().all(|ch| ch.is_lowercase());
    let sentence = rules.sentences && starts_small && after_sentence_end(&head[..at]);
    let mut corrected: String = chars.into_iter().collect();
    if day || sentence {
        corrected = capitalized(&corrected);
    }
    (corrected != word).then(|| Correction {
        at,
        original: word.to_string(),
        text: corrected,
    })
}

/// The correction that typing the character before `caret` calls for.
pub fn correct_typed(text: &str, caret: usize, rules: &Rules) -> Option<Correction> {
    let head = text.get(..caret)?;
    let typed = head.chars().last()?;
    if is_word_char(typed) || is_formula(text) {
        return None;
    }
    // "(c)" is replaced when its bracket closes; "teh" when a space, a
    // comma… follows it.
    replacement_at_end(head, rules, true).or_else(|| correct_word(text, caret - typed.len_utf8(), rules))
}

/// The correction of the last word of a confirmed entry.
pub fn correct_entry(text: &str, rules: &Rules) -> Option<Correction> {
    if is_formula(text) || !text.chars().last().is_some_and(is_word_char) {
        return None;
    }
    correct_word(text, text.len(), rules)
}

fn is_formula(text: &str) -> bool {
    text.starts_with('=')
}

/// What the corrections of the entry being typed have done so far.
#[derive(Default)]
pub struct Typing {
    /// The cell being typed in.
    cell: Option<(usize, u32, u32)>,
    /// The text after the last keystroke.
    last: String,
    /// The last correction, as it now stands in the text (`text` at
    /// `at`), for ⌘Z to take back.
    done: Option<Correction>,
    /// What the user took back: left alone until the entry is confirmed.
    refused: Vec<String>,
}

/// The text of the editor changed: correct what was just typed. Called
/// by the application before it shows the change.
pub fn text_typed(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(cell) = app.editing.as_ref().map(|edit| (edit.sheet, edit.row, edit.col)) else {
        return;
    };
    let editor = app.editor().clone();
    let (text, selection) = {
        let input = editor.read(cx);
        (input.text().to_string(), input.selection())
    };
    let state = app.ext_mut::<Typing>();
    // Another entry starts: nothing of the previous one applies.
    if state.cell != Some(cell) || text.chars().count() <= 1 {
        *state = Typing {
            cell: Some(cell),
            ..Typing::default()
        };
    }
    let before = std::mem::replace(&mut state.last, text.clone());
    let caret = selection.end;
    // One character was typed at the caret: nothing else is corrected (a
    // deletion, a paste, a reference picked with the mouse).
    let typed_one = selection.is_empty()
        && text[..caret].chars().last().is_some_and(|ch| {
            let start = caret - ch.len_utf8();
            before.len() + ch.len_utf8() == text.len() && before.get(..start) == Some(&text[..start]) && before.get(start..) == Some(&text[caret..])
        });
    if !typed_one {
        return;
    }
    let Some(found) = correct_typed(&text, caret, &Rules::current()) else {
        return;
    };
    if state.refused.contains(&found.original) {
        return;
    }
    let new_caret = caret - found.original.len() + found.text.len();
    editor.update(cx, |input, cx| {
        input.replace_range(found.range(), &found.text, cx);
        input.set_selection(new_caret..new_caret, cx);
    });
    let state = app.ext_mut::<Typing>();
    state.last = editor.read(cx).text().to_string();
    state.done = Some(found);
}

/// ⌘Z while typing: take the last correction back. False when there is
/// none (the text has changed there since, or nothing was corrected).
pub fn undo_correction(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let Some(done) = app.ext_mut::<Typing>().done.take() else {
        return false;
    };
    let editor = app.editor().clone();
    let (text, caret) = {
        let input = editor.read(cx);
        (input.text().to_string(), input.selection().end)
    };
    let corrected = done.at..done.at + done.text.len();
    if text.get(corrected.clone()) != Some(done.text.as_str()) {
        return false;
    }
    // The caret stays on the character it was after.
    let caret = if caret >= corrected.end { caret - done.text.len() + done.original.len() } else { caret.min(done.at + done.original.len()) };
    editor.update(cx, |input, cx| {
        input.replace_range(corrected, &done.original, cx);
        input.set_selection(caret..caret, cx);
    });
    let state = app.ext_mut::<Typing>();
    state.last = editor.read(cx).text().to_string();
    state.refused.push(done.original);
    true
}

/// An entry was confirmed: its last word is corrected as when a space
/// follows it, as an undo step of its own.
pub(super) fn cells_changed(app: &mut ExcelApp, edit: &CellEdit, cx: &mut Cx) {
    if !matches!(edit.cause, ChangeCause::Typing) {
        return;
    }
    let refused = std::mem::take(&mut app.ext_mut::<Typing>().refused);
    *app.ext_mut::<Typing>() = Typing::default();
    let (sheet, row, col) = (edit.sheet, edit.rect.r0, edit.rect.c0);
    if sheet != app.sheet {
        return;
    }
    let typed = match app.wb.cell(sheet, row, col).map(|cell| &cell.content) {
        Some(Content::Value(Value::Text(text))) => text.to_string(),
        _ => return,
    };
    let Some(found) = correct_entry(&typed, &Rules::current()).filter(|found| !refused.contains(&found.original)) else {
        return;
    };
    let mut corrected = typed;
    corrected.replace_range(found.range(), &found.text);
    app.wb.set_value(sheet, row, col, Value::text(&corrected));
    // Through the same door as the entry itself: a table whose header
    // this is takes the corrected name.
    app.commit_cells(t("AutoCorrect", "Correction automatique"), &[Rect::cell(row, col)], ChangeCause::Typing, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules {
            replacements: starting_list(Lang::En),
            two_capitals: true,
            sentences: true,
            days: Lang::En.locale().days.iter().map(|day| day.to_lowercase()).collect(),
        }
    }

    /// What typing `text` one character after the other leaves.
    fn typed(text: &str, rules: &Rules) -> String {
        let mut out = String::new();
        for ch in text.chars() {
            out.push(ch);
            if let Some(found) = correct_typed(&out, out.len(), rules) {
                out.replace_range(found.range(), &found.text);
            }
        }
        if let Some(found) = correct_entry(&out, rules) {
            out.replace_range(found.range(), &found.text);
        }
        out
    }

    #[test]
    fn words_of_the_list_are_replaced_when_they_end() {
        let rules = rules();
        assert_eq!(typed("teh cat adn teh dog", &rules), "the cat and the dog");
        // With the capitals of what was typed.
        assert_eq!(typed("Teh end. TEH END", &rules), "The end. THE END");
        assert_eq!(typed("i think, i am", &rules), "I think, I am");
        // Not inside a longer word, not in a formula.
        assert_eq!(typed("tehran lathe ", &rules), "tehran lathe ");
        assert_eq!(typed("=teh +1", &rules), "=teh +1");
        // The last word, when the entry is confirmed.
        assert_eq!(typed("recieve", &rules), "receive");
        // While the word is being typed nothing moves.
        assert_eq!(correct_typed("teh", 3, &rules), None);
    }

    #[test]
    fn signs_are_replaced_as_soon_as_they_are_typed() {
        let rules = rules();
        assert_eq!(typed("(c) 2026 Ada (tm)", &rules), "© 2026 Ada ™");
        assert_eq!(typed("wait... a --> b", &rules), "wait… a → b");
        assert_eq!(typed("5 (E)", &rules), "5 €");
        // "(c)" glued to a word is a function call or a reference.
        assert_eq!(typed("f(c)", &rules), "f(c)");
        assert_eq!(correct_typed("(c)", 3, &rules).map(|found| found.text), Some("©".to_string()));
    }

    #[test]
    fn capitals_are_corrected() {
        let rules = rules();
        assert_eq!(typed("THe plan. it works! does it? yes", &rules), "The plan. It works! Does it? Yes");
        // The start of a cell is left as typed, like Excel; so are short
        // forms, words in capitals and two-letter words.
        assert_eq!(typed("see p. 4 etc. and so on... or not", &rules), "see p. 4 etc. and so on… or not");
        assert_eq!(typed("USA and IT are OK", &rules), "USA and IT are OK");
        assert_eq!(typed("on monday and FRIDAY, not mondays", &rules), "on Monday and FRIDAY, not mondays");
        let off = Rules::default();
        assert_eq!(typed("THe plan. it works on monday, teh end (c)", &off), "THe plan. it works on monday, teh end (c)");
    }

    #[test]
    fn every_language_has_a_starting_list() {
        for lang in Lang::ALL {
            let list = starting_list(lang);
            assert!(list.len() > 8 && list.len() < 30, "{lang:?}");
            assert!(list.iter().any(|(from, to)| from == "(c)" && to == "©"));
            // Nothing is replaced by itself and nothing is listed twice.
            for (index, (from, to)) in list.iter().enumerate() {
                assert_ne!(from, to);
                assert!(!list[..index].iter().any(|(other, _)| other == from), "{from}");
            }
        }
        assert_eq!(typed("un oeuf, Deja", &Rules { replacements: starting_list(Lang::Fr), ..Rules::default() }), "un œuf, Déjà");
    }
}
