//! The code editor: a box of source text in a fixed-width font with line
//! numbers, the line of the caret marked, colours for the language, and
//! the editing a programmer expects (see `buffer.rs`). It fills the
//! element that holds it and scrolls both ways.
//!
//! The same view shows the modules of a VBA project, read-only: the text
//! can be selected and copied, not changed.
//!
//! Only the lines in view are shaped and coloured, so the length of a
//! script costs nothing while it is drawn.

use super::buffer::CodeBuffer;
use super::complete::{self, Completion, Hint};
use super::search;
use super::syntax::{Language, LineState, Token, highlight_line, line_states};
use crate::i18n::t;
use crate::text_input::{self as line, InputEvent, TextInput};
use crate::theme::{self, c, ca};
use crate::view::{icon, tooltip};
use crate::widgets::text_area as area;
use crate::widgets::{self, Button};
use gpui::{
    AnyElement, App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, GlobalElementId, Hsla, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString,
    Style, Subscription, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, font, point, prelude::*, px, relative, size,
};
use std::ops::Range;

actions!(code_editor, [Save, Undo, Redo, ToggleComment, PageUp, PageDown, Find, Replace, FindNext, FindPrevious, GoToLine]);

/// Key context of the keys the editor adds to those of the text boxes.
const CONTEXT: &str = "CodeEditor";
/// The editor answers the keys of all three.
const CONTEXTS: &str = "TextInput TextArea CodeEditor";

const FONT_SIZE: f32 = 12.0;
const LINE_HEIGHT: f32 = 18.0;
/// Room left of the text, after the line numbers.
const TEXT_GAP: f32 = 10.0;

// The colours of the light theme of Excel's own editor.
const COLOR_KEYWORD: u32 = 0x0000FF;
const COLOR_STRING: u32 = 0xA31515;
const COLOR_COMMENT: u32 = 0x008000;
const COLOR_NUMBER: u32 = 0x098658;
const COLOR_TYPE: u32 = 0x267F99;
const COLOR_PROBLEM: u32 = 0xE51400;
const COLOR_CURRENT_LINE: u32 = 0xF3F8F5;
const COLOR_GUTTER_TEXT: u32 = 0x8A8A8A;
const COLOR_FOUND: u32 = 0xF2DC9C;
const COLOR_FOUND_CURRENT: u32 = 0xF0A93B;
const COLOR_BRACKET: u32 = 0xC9D7E8;

/// The list of completions: how many entries show at once, the height
/// of one and the width of the list.
const PROPOSAL_ROWS: usize = 8;
const PROPOSAL_ROW: f32 = 20.0;
const PROPOSAL_WIDTH: f32 = 440.0;

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, c),
        KeyBinding::new("cmd-z", Undo, c),
        KeyBinding::new("cmd-shift-z", Redo, c),
        KeyBinding::new("cmd-y", Redo, c),
        KeyBinding::new("cmd-/", ToggleComment, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("cmd-f", Find, c),
        KeyBinding::new("cmd-alt-f", Replace, c),
        KeyBinding::new("cmd-g", FindNext, c),
        KeyBinding::new("cmd-shift-g", FindPrevious, c),
        // Go to Line: ⌃G as in the editor of Excel, and ⌘L.
        KeyBinding::new("ctrl-g", GoToLine, c),
        KeyBinding::new("cmd-l", GoToLine, c),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorEvent {
    /// The text changed.
    Changed,
    /// ⌘S was pressed in the editor.
    Save,
}

fn token_color(token: Token) -> Hsla {
    c(match token {
        Token::Plain => theme::TEXT,
        Token::Keyword => COLOR_KEYWORD,
        Token::String => COLOR_STRING,
        Token::Comment => COLOR_COMMENT,
        Token::Number => COLOR_NUMBER,
        Token::Type => COLOR_TYPE,
    })
}

fn code_font() -> Font {
    font("Menlo")
}

/// What the last paint laid out, for the mouse and the keys that move by
/// what is on screen.
struct Geometry {
    bounds: Bounds<Pixels>,
    /// Where the text starts, right of the line numbers.
    text_left: Pixels,
    char_width: Pixels,
    /// The lines in view: index of the line and its shaped text.
    lines: Vec<(usize, ShapedLine)>,
    /// The entries of the list of completions that are drawn: their
    /// place in the list and where they are.
    proposal_rows: Vec<(usize, Bounds<Pixels>)>,
}

/// What the bar over the text is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bar {
    Find,
    Replace,
    GoToLine,
}

/// Find and Replace, and Go to Line: the fields of the bar and what was
/// found. It is made the first time the bar opens and stays, so that
/// Find Next goes on with the last search once the bar is closed.
struct Finder {
    query: Entity<TextInput>,
    replacement: Entity<TextInput>,
    line: Entity<TextInput>,
    options: search::Options,
    /// Where the query stands in the text.
    matches: Vec<Range<usize>>,
    _subscriptions: Vec<Subscription>,
}

/// The list of completions while it is open.
struct Proposal {
    completion: Completion,
    /// The entry Return and Tab take.
    selected: usize,
    /// The first entry shown.
    first: usize,
}

pub(crate) struct CodeEditor {
    focus_handle: FocusHandle,
    buffer: CodeBuffer,
    language: Language,
    read_only: bool,
    /// Lines with a problem (1-based) and what the problem is.
    marks: Vec<(u32, SharedString)>,
    /// What an input method is composing.
    marked: Option<Range<usize>>,
    /// The state each line starts in, for the colours.
    states: Vec<LineState>,
    scroll_top: f32,
    scroll_left: f32,
    /// The column Up and Down aim at while they follow one another.
    goal_col: Option<usize>,
    selecting: bool,
    /// Bring the caret into view at the next paint.
    reveal: bool,
    geometry: Option<Geometry>,
    /// What is proposed after the dot being typed.
    proposal: Option<Proposal>,
    /// The parameters of the call the caret is in.
    hint: Option<Hint>,
    finder: Option<Finder>,
    /// The bar shown over the text.
    bar: Option<Bar>,
}

impl EventEmitter<EditorEvent> for CodeEditor {}

impl CodeEditor {
    pub fn new(text: &str, language: Language, cx: &mut Context<Self>) -> Self {
        let buffer = CodeBuffer::new(text);
        CodeEditor {
            focus_handle: cx.focus_handle().tab_stop(true),
            states: line_states(language, buffer.text()),
            buffer,
            language,
            read_only: false,
            marks: Vec::new(),
            marked: None,
            scroll_top: 0.0,
            scroll_left: 0.0,
            goal_col: None,
            selecting: false,
            reveal: false,
            geometry: None,
            proposal: None,
            hint: None,
            finder: None,
            bar: None,
        }
    }

    /// A viewer: the text can be selected and copied, not changed.
    pub fn viewer(text: &str, language: Language, cx: &mut Context<Self>) -> Self {
        let mut editor = CodeEditor::new(text, language, cx);
        editor.read_only = true;
        editor
    }

    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// The text with its selection, for the tests that type into it.
    #[cfg(test)]
    pub fn buffer(&self) -> &CodeBuffer {
        &self.buffer
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Show another text, saved as it is: nothing to undo, the caret at
    /// the top.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.buffer = CodeBuffer::new(text);
        self.marks.clear();
        self.marked = None;
        self.scroll_top = 0.0;
        self.scroll_left = 0.0;
        self.goal_col = None;
        self.proposal = None;
        self.hint = None;
        self.states = line_states(self.language, self.buffer.text());
        self.refresh_matches(cx);
        cx.notify();
    }

    /// Replace the whole text as an edit: it can be undone, and the text
    /// is no longer the saved one.
    pub fn replace_all(&mut self, text: &str, cx: &mut Context<Self>) {
        self.buffer.replace(0..self.buffer.text().len(), text);
        self.buffer.move_to(0);
        self.changed(cx);
    }

    /// The text was written to its file.
    pub fn mark_saved(&mut self, cx: &mut Context<Self>) {
        self.buffer.mark_saved();
        cx.notify();
    }

    /// Mark the lines that have a problem.
    pub fn set_marks(&mut self, marks: Vec<(u32, SharedString)>, cx: &mut Context<Self>) {
        if self.marks != marks {
            self.marks = marks;
            cx.notify();
        }
    }

    /// Put the caret on a line (1-based) and show it.
    pub fn go_to_line(&mut self, line: u32, column: Option<u32>, cx: &mut Context<Self>) {
        self.buffer.go_to_line(line, column);
        self.goal_col = None;
        self.moved(cx);
    }

    /// Line and column of the caret, 1-based as the status shows them.
    pub fn caret_position(&self) -> (usize, usize) {
        let (line, col) = self.buffer.line_col(self.buffer.caret());
        (line + 1, col + 1)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.states = line_states(self.language, self.buffer.text());
        self.goal_col = None;
        self.reveal = true;
        self.proposal = None;
        self.refresh_hint();
        self.refresh_matches(cx);
        cx.emit(EditorEvent::Changed);
        cx.notify();
    }

    fn moved(&mut self, cx: &mut Context<Self>) {
        self.reveal = true;
        // The list was for the place the caret has left.
        self.proposal = None;
        self.refresh_hint();
        cx.notify();
    }

    // ---- find and replace ---------------------------------------------------

    /// The fields of the bar, made when it first opens.
    fn finder(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &mut Finder {
        if self.finder.is_none() {
            let query = widgets::input(cx, "");
            let replacement = widgets::input(cx, "");
            let line = widgets::input(cx, "");
            let subscriptions = vec![
                cx.subscribe_in(&query, window, Self::on_query_event),
                cx.subscribe_in(&replacement, window, Self::on_replacement_event),
                cx.subscribe_in(&line, window, Self::on_line_event),
            ];
            self.finder = Some(Finder {
                query,
                replacement,
                line,
                options: search::Options::default(),
                matches: Vec::new(),
                _subscriptions: subscriptions,
            });
        }
        self.finder.as_mut().expect("just made")
    }

    /// Open the bar for a search, a replacement or a line number, the
    /// keys going to its first field.
    pub(crate) fn open_bar(&mut self, bar: Bar, window: &mut Window, cx: &mut Context<Self>) {
        // Replacing changes the text: not in a viewer.
        let bar = if self.read_only && bar == Bar::Replace { Bar::Find } else { bar };
        // What is selected on one line is what is looked for.
        let selected = Some(self.buffer.selected_text().to_string()).filter(|text| !text.is_empty() && !text.contains('\n'));
        let finder = self.finder(window, cx);
        let field = match bar {
            Bar::GoToLine => {
                finder.line.update(cx, |input, cx| input.set_text("", cx));
                finder.line.clone()
            }
            _ => {
                if let Some(text) = selected {
                    finder.query.update(cx, |input, cx| input.set_text(text, cx));
                }
                finder.query.clone()
            }
        };
        field.update(cx, |input, cx| input.select_all_text(cx));
        window.focus(&field.read(cx).focus_handle(cx));
        self.bar = Some(bar);
        self.proposal = None;
        self.refresh_matches(cx);
        cx.notify();
    }

    pub(crate) fn close_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.bar.take().is_some() {
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    /// True while a field of the bar has the keys: what it lets through
    /// (Return, Tab, Escape, the arrows) is not meant for the text.
    fn keys_in_bar(&self, window: &Window) -> bool {
        self.bar.is_some() && !self.focus_handle.is_focused(window)
    }

    /// Look the query up again: the text or the query changed.
    fn refresh_matches(&mut self, cx: &mut Context<Self>) {
        if let Some(finder) = &mut self.finder {
            let query = finder.query.read(cx).text().to_string();
            finder.matches = search::find_all(self.buffer.text(), &query, finder.options);
        }
    }

    /// The match the selection is on, when it is on one.
    fn current_match(&self) -> Option<usize> {
        let selected = self.buffer.selection();
        self.finder.as_ref()?.matches.iter().position(|range| *range == selected)
    }

    /// Select the next (or the previous) match, round the ends of the
    /// text. `from` is where to look from: the end of the selection to go
    /// on, its start to stay on a match that still holds while the query
    /// is typed.
    fn find_from(&mut self, from: usize, forward: bool, cx: &mut Context<Self>) -> bool {
        self.refresh_matches(cx);
        let Some(finder) = &self.finder else {
            return false;
        };
        let Some(index) = search::next_match(&finder.matches, from, forward) else {
            return false;
        };
        let range = finder.matches[index].clone();
        self.buffer.select(range);
        self.goal_col = None;
        self.moved(cx);
        true
    }

    /// Find Next and Find Previous.
    pub(crate) fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let selected = self.buffer.selection();
        self.find_from(if forward { selected.end } else { selected.start }, forward, cx)
    }

    /// Replace the match the selection is on, then go to the next one;
    /// when the selection is on none, go to the next one first.
    pub(crate) fn replace_one(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.refresh_matches(cx);
        if self.current_match().is_some() {
            let replacement = self.finder.as_ref().map(|finder| finder.replacement.read(cx).text().to_string()).unwrap_or_default();
            let range = self.buffer.selection();
            self.edit(cx, |buffer| buffer.replace(range, &replacement));
        }
        self.find_step(true, cx);
    }

    /// Replace every match, as one change of the text: one undo step.
    /// Returns how many were replaced.
    pub(crate) fn replace_everything(&mut self, cx: &mut Context<Self>) -> usize {
        if self.read_only {
            return 0;
        }
        self.refresh_matches(cx);
        let Some(finder) = &self.finder else {
            return 0;
        };
        let replacement = finder.replacement.read(cx).text().to_string();
        let count = finder.matches.len();
        if let Some((range, text)) = search::replace_all(self.buffer.text(), &finder.matches, &replacement) {
            self.edit(cx, |buffer| buffer.replace(range, &text));
        }
        count
    }

    fn toggle_option(&mut self, whole_word: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(finder) = &mut self.finder {
            if whole_word {
                finder.options.whole_word = !finder.options.whole_word;
            } else {
                finder.options.match_case = !finder.options.match_case;
            }
            // The keys stay in the field of the query.
            window.focus(&finder.query.read(cx).focus_handle(cx));
        }
        self.refresh_matches(cx);
        cx.notify();
    }

    fn on_query_event(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            // The first match from where the caret is shows as the query
            // is typed.
            InputEvent::Changed => {
                let from = self.buffer.selection().start;
                if !self.find_from(from, true, cx) {
                    cx.notify();
                }
            }
            InputEvent::Enter { shift } => {
                self.find_step(!shift, cx);
            }
            InputEvent::Tab { .. } if self.bar == Some(Bar::Replace) => {
                if let Some(finder) = &self.finder {
                    finder.replacement.update(cx, |input, cx| input.select_all_text(cx));
                    window.focus(&finder.replacement.read(cx).focus_handle(cx));
                }
            }
            InputEvent::Escape => self.close_bar(window, cx),
            _ => {}
        }
    }

    fn on_replacement_event(&mut self, _: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Enter { .. } => self.replace_one(cx),
            InputEvent::Tab { .. } => {
                if let Some(finder) = &self.finder {
                    window.focus(&finder.query.read(cx).focus_handle(cx));
                }
            }
            InputEvent::Escape => self.close_bar(window, cx),
            _ => {}
        }
    }

    fn on_line_event(&mut self, input: &Entity<TextInput>, event: &InputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InputEvent::Enter { .. } => {
                if let Ok(line) = input.read(cx).text().trim().parse::<u32>() {
                    let line = line.clamp(1, self.buffer.line_count() as u32);
                    self.go_to_line(line, None, cx);
                }
                self.close_bar(window, cx);
            }
            InputEvent::Escape => self.close_bar(window, cx),
            _ => {}
        }
    }

    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_bar(Bar::Find, window, cx);
    }

    fn replace(&mut self, _: &Replace, window: &mut Window, cx: &mut Context<Self>) {
        self.open_bar(Bar::Replace, window, cx);
    }

    fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(true, cx);
    }

    fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.find_step(false, cx);
    }

    fn go_to_line_bar(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.open_bar(Bar::GoToLine, window, cx);
    }

    /// The bar shown, the matches and the one the selection is on.
    #[cfg(test)]
    pub fn found(&self) -> (Option<Bar>, usize, Option<usize>) {
        (self.bar, self.finder.as_ref().map_or(0, |finder| finder.matches.len()), self.current_match())
    }

    /// A small button of the bar.
    fn bar_button(
        id: &'static str,
        content: AnyElement,
        tip: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .flex_none()
            .h(px(22.))
            .min_w(px(22.))
            .px(px(3.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(3.))
            .when(on, |d| d.bg(c(theme::PRESSED)))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(content)
            .tooltip(tooltip(tip))
            .on_click(cx.listener(move |editor, _, window, cx| action(editor, window, cx)))
            .into_any_element()
    }

    fn render_bar(&self, bar: Bar, cx: &mut Context<Self>) -> Option<AnyElement> {
        let finder = self.finder.as_ref()?;
        let close = Self::bar_button("ce-find-close", icon("close", 12.0, theme::TEXT).into_any_element(), t("Close", "Fermer"), false, cx, |editor, window, cx| {
            editor.close_bar(window, cx)
        });
        let frame = div().flex_none().px(px(6.)).py(px(4.)).flex().flex_col().gap(px(4.)).bg(c(theme::PANEL_BG)).border_b_1().border_color(c(theme::BORDER)).cursor(CursorStyle::Arrow);
        if bar == Bar::GoToLine {
            let label = crate::i18n::labelled(t("Go to line", "Atteindre la ligne"), "");
            return Some(
                frame
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().flex_none().child(label.trim_end().to_string()))
                            .child(widgets::TextField::new(&finder.line).height(22.0))
                            .child(close),
                    )
                    .into_any_element(),
            );
        }
        let total = finder.matches.len();
        let count = match (finder.query.read(cx).text().is_empty(), self.current_match()) {
            (true, _) => String::new(),
            (false, _) if total == 0 => t("No results", "Aucun résultat").to_string(),
            (false, Some(current)) => format!("{}/{total}", current + 1),
            (false, None) => format!("{total}"),
        };
        let letters = |text: &'static str| div().text_size(px(11.)).child(text).into_any_element();
        let find_row = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(widgets::TextField::new(&finder.query).height(22.0))
            .child(div().flex_none().px(px(4.)).text_size(px(11.)).text_color(c(theme::TEXT_MUTED)).debug_selector(|| "ce-find-count".to_string()).child(count))
            .child(Self::bar_button("ce-find-case", letters("Aa"), t("Match case", "Respecter la casse"), finder.options.match_case, cx, |editor, window, cx| editor.toggle_option(false, window, cx)))
            .child(Self::bar_button("ce-find-word", letters("ab|"), t("Find whole words only", "Mot entier"), finder.options.whole_word, cx, |editor, window, cx| editor.toggle_option(true, window, cx)))
            .child(Self::bar_button("ce-find-prev", icon("chevron-up", 12.0, theme::TEXT).into_any_element(), t("Find Previous", "Précédent"), false, cx, |editor, _, cx| {
                editor.find_step(false, cx);
            }))
            .child(Self::bar_button("ce-find-next", icon("chevron-down", 12.0, theme::TEXT).into_any_element(), t("Find Next", "Suivant"), false, cx, |editor, _, cx| {
                editor.find_step(true, cx);
            }))
            .child(close);
        let replace_row = (bar == Bar::Replace).then(|| {
            let editor = cx.entity();
            let (one, all) = (editor.clone(), editor);
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(widgets::TextField::new(&finder.replacement).height(22.0))
                .child(div().debug_selector(|| "ce-find-replace".to_string()).child(Button::new("ce-find-replace", t("Replace", "Remplacer")).on_click(move |_, _, cx| one.update(cx, |editor, cx| editor.replace_one(cx)))))
                .child(div().debug_selector(|| "ce-find-all".to_string()).child(Button::new("ce-find-all", t("Replace All", "Remplacer tout")).on_click(move |_, _, cx| {
                    all.update(cx, |editor, cx| {
                        editor.replace_everything(cx);
                    })
                })))
        });
        Some(frame.child(find_row).children(replace_row).into_any_element())
    }

    // ---- completion -------------------------------------------------------

    /// Whether the editor proposes anything: for a script being written.
    fn completes(&self) -> bool {
        !self.read_only && self.language == Language::TypeScript
    }

    /// Show the parameters of the call the caret is in, if it is in one.
    fn refresh_hint(&mut self) {
        self.hint = if self.completes() && self.buffer.selection().is_empty() { complete::hint(self.buffer.text(), self.buffer.caret()) } else { None };
    }

    /// Open, narrow or close the list for what stands before the caret:
    /// what was typed decides again, the best match is selected.
    fn propose(&mut self, cx: &mut Context<Self>) {
        self.proposal = None;
        if self.completes() && self.buffer.selection().is_empty() {
            if let Some(completion) = complete::complete(self.buffer.text(), self.buffer.caret()) {
                self.proposal = Some(Proposal {
                    completion,
                    selected: 0,
                    first: 0,
                });
            }
        }
        cx.notify();
    }

    /// Move the selection of the list by `by` entries, round the ends.
    fn step_proposal(&mut self, by: isize, cx: &mut Context<Self>) -> bool {
        let Some(proposal) = &mut self.proposal else {
            return false;
        };
        let count = proposal.completion.entries.len() as isize;
        proposal.selected = (proposal.selected as isize + by).rem_euclid(count) as usize;
        // Keep the selected entry among those shown.
        if proposal.selected < proposal.first {
            proposal.first = proposal.selected;
        } else if proposal.selected >= proposal.first + PROPOSAL_ROWS {
            proposal.first = proposal.selected + 1 - PROPOSAL_ROWS;
        }
        cx.notify();
        true
    }

    /// Put an entry of the list in the text, in place of what was typed
    /// of it. False when the list is not open.
    fn accept_proposal(&mut self, index: Option<usize>, cx: &mut Context<Self>) -> bool {
        let Some(proposal) = self.proposal.take() else {
            return false;
        };
        let Some(entry) = proposal.completion.entries.get(index.unwrap_or(proposal.selected)) else {
            return false;
        };
        let range = proposal.completion.start..self.buffer.caret();
        let name = entry.name.clone();
        self.edit(cx, |buffer| buffer.replace(range, &name));
        true
    }

    /// The names the list shows, the selected one and the first shown.
    #[cfg(test)]
    pub fn proposed(&self) -> Option<(Vec<String>, usize)> {
        self.proposal.as_ref().map(|proposal| (proposal.completion.entries.iter().map(|entry| entry.name.clone()).collect(), proposal.selected))
    }

    /// Where an entry of the list was drawn, when it is among those shown.
    #[cfg(test)]
    pub fn proposal_row(&self, index: usize) -> Option<Point<Pixels>> {
        self.geometry.as_ref()?.proposal_rows.iter().find(|(row, _)| *row == index).map(|(_, place)| place.center())
    }

    /// What stands beside the selected entry of the list.
    #[cfg(test)]
    pub fn proposed_detail(&self) -> Option<String> {
        self.proposal.as_ref().and_then(|proposal| proposal.completion.entries.get(proposal.selected)).map(|entry| entry.detail.clone())
    }

    /// The hint shown, the parameter being typed between brackets.
    #[cfg(test)]
    pub fn hinted(&self) -> Option<String> {
        self.hint.as_ref().map(|hint| match &hint.active {
            Some(active) => format!("{}[{}]{}", &hint.signature[..active.start], &hint.signature[active.clone()], &hint.signature[active.end..]),
            None => hint.signature.clone(),
        })
    }

    /// Run an edit unless the text is read-only.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut CodeBuffer)) {
        if self.read_only {
            return;
        }
        let before = self.buffer.text().to_string();
        change(&mut self.buffer);
        if self.buffer.text() != before {
            self.marked = None;
            self.changed(cx);
        } else {
            self.goal_col = None;
            self.moved(cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buffer.move_to(offset);
        self.goal_col = None;
        self.moved(cx);
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buffer.select_to(offset);
        self.goal_col = None;
        self.moved(cx);
    }

    /// The offset `lines` lines below (or above) the caret, in its column.
    fn vertical(&mut self, lines: isize) -> usize {
        let (line, col) = self.buffer.line_col(self.buffer.caret());
        let col = *self.goal_col.get_or_insert(col);
        let target = line as isize + lines;
        if target < 0 {
            return 0;
        }
        if target as usize >= self.buffer.line_count() {
            return self.buffer.text().len();
        }
        self.buffer.offset_of(target as usize, col)
    }

    /// How many lines the box shows.
    fn page(&self) -> isize {
        let height = self.geometry.as_ref().map_or(LINE_HEIGHT * 10.0, |g| f32::from(g.bounds.size.height));
        ((height / LINE_HEIGHT).floor() as isize - 1).max(1)
    }

    fn line_end(&self) -> usize {
        self.buffer.line_range(self.buffer.line_of(self.buffer.caret())).end
    }

    // ---- keys -----------------------------------------------------------

    fn left(&mut self, _: &line::Left, _: &mut Window, cx: &mut Context<Self>) {
        let selected = self.buffer.selection();
        let to = if selected.is_empty() { self.buffer.previous_boundary(selected.start) } else { selected.start };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &line::Right, _: &mut Window, cx: &mut Context<Self>) {
        let selected = self.buffer.selection();
        let to = if selected.is_empty() { self.buffer.next_boundary(selected.end) } else { selected.end };
        self.move_to(to, cx);
    }

    fn up(&mut self, _: &line::Up, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        if self.step_proposal(-1, cx) {
            return;
        }
        let to = self.vertical(-1);
        self.buffer.move_to(to);
        self.moved(cx);
    }

    fn down(&mut self, _: &line::Down, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        if self.step_proposal(1, cx) {
            return;
        }
        let to = self.vertical(1);
        self.buffer.move_to(to);
        self.moved(cx);
    }

    fn select_up(&mut self, _: &area::SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(-1);
        self.buffer.select_to(to);
        self.moved(cx);
    }

    fn select_down(&mut self, _: &area::SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(1);
        self.buffer.select_to(to);
        self.moved(cx);
    }

    fn page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.step_proposal(-(PROPOSAL_ROWS as isize - 1), cx) {
            return;
        }
        let to = self.vertical(-self.page());
        self.buffer.move_to(to);
        self.moved(cx);
    }

    fn page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.step_proposal(PROPOSAL_ROWS as isize - 1, cx) {
            return;
        }
        let to = self.vertical(self.page());
        self.buffer.move_to(to);
        self.moved(cx);
    }

    fn select_left(&mut self, _: &line::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.previous_boundary(self.buffer.caret()), cx);
    }

    fn select_right(&mut self, _: &line::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.next_boundary(self.buffer.caret()), cx);
    }

    fn select_all(&mut self, _: &line::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.select_all();
        cx.notify();
    }

    fn home(&mut self, _: &line::Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.smart_home(self.buffer.caret()), cx);
    }

    fn end(&mut self, _: &line::End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.line_end(), cx);
    }

    fn select_home(&mut self, _: &line::SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.smart_home(self.buffer.caret()), cx);
    }

    fn select_end(&mut self, _: &line::SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_end(), cx);
    }

    fn text_start(&mut self, _: &area::TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn text_end(&mut self, _: &area::TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.text().len(), cx);
    }

    fn select_text_start(&mut self, _: &area::SelectTextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_text_end(&mut self, _: &area::SelectTextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.text().len(), cx);
    }

    fn word_left(&mut self, _: &line::WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.previous_word(self.buffer.caret()), cx);
    }

    fn word_right(&mut self, _: &line::WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.next_word(self.buffer.caret()), cx);
    }

    fn select_word_left(&mut self, _: &line::SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.previous_word(self.buffer.caret()), cx);
    }

    fn select_word_right(&mut self, _: &line::SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.next_word(self.buffer.caret()), cx);
    }

    fn delete_word_left(&mut self, _: &line::DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |buffer| {
            if buffer.selection().is_empty() {
                buffer.select_to(buffer.previous_word(buffer.caret()));
            }
            buffer.replace(buffer.selection(), "");
        });
    }

    fn backspace(&mut self, _: &line::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        // A list that is open follows what is left of the word.
        let proposing = self.proposal.is_some();
        self.edit(cx, CodeBuffer::backspace);
        if proposing {
            self.propose(cx);
        }
    }

    fn delete(&mut self, _: &line::Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, CodeBuffer::delete_forward);
    }

    fn enter(&mut self, _: &line::Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        if self.accept_proposal(None, cx) {
            return;
        }
        self.edit(cx, CodeBuffer::newline);
    }

    fn shift_enter(&mut self, _: &line::ShiftEnter, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        self.edit(cx, CodeBuffer::newline);
    }

    fn tab(&mut self, _: &line::Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        // A viewer has nothing to indent: Tab goes on to the next control.
        if self.read_only {
            cx.propagate();
            return;
        }
        if self.accept_proposal(None, cx) {
            return;
        }
        self.edit(cx, CodeBuffer::indent);
    }

    fn shift_tab(&mut self, _: &line::ShiftTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.keys_in_bar(window) {
            return;
        }
        if self.read_only {
            cx.propagate();
            return;
        }
        self.edit(cx, CodeBuffer::outdent);
    }

    fn escape(&mut self, _: &line::Escape, window: &mut Window, cx: &mut Context<Self>) {
        // Escape closes the list, then the hint, then the bar; with none
        // of them it leaves the editor for whatever holds it.
        if self.keys_in_bar(window) {
            return;
        }
        if self.proposal.take().is_some() || self.hint.take().is_some() {
            cx.notify();
            return;
        }
        if self.bar.is_some() {
            self.close_bar(window, cx);
            return;
        }
        cx.propagate();
    }

    fn toggle_comment(&mut self, _: &ToggleComment, _: &mut Window, cx: &mut Context<Self>) {
        let marker = match self.language {
            Language::TypeScript => "//",
            Language::Vba => "'",
        };
        self.edit(cx, |buffer| buffer.toggle_comment(marker));
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |buffer| {
            buffer.undo();
        });
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |buffer| {
            buffer.redo();
        });
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            cx.propagate();
            return;
        }
        cx.emit(EditorEvent::Save);
    }

    fn show_character_palette(&mut self, _: &line::ShowCharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &line::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |buffer| buffer.insert(&text));
        }
    }

    fn copy(&mut self, _: &line::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.selected_text().to_string()));
        }
    }

    fn cut(&mut self, _: &line::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.selection().is_empty() && !self.read_only {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buffer.selected_text().to_string()));
            self.edit(cx, |buffer| buffer.insert(""));
        }
    }

    // ---- mouse ----------------------------------------------------------

    /// The offset under a point of the window.
    fn index_for_point(&self, position: Point<Pixels>) -> usize {
        let Some(geometry) = &self.geometry else {
            return 0;
        };
        let y = f32::from(position.y - geometry.bounds.top()) + self.scroll_top;
        let line = ((y / LINE_HEIGHT).floor().max(0.0) as usize).min(self.buffer.line_count() - 1);
        let x = position.x - geometry.text_left + px(self.scroll_left);
        let start = self.buffer.line_range(line).start;
        match geometry.lines.iter().find(|(index, _)| *index == line) {
            Some((_, shaped)) => start + shaped.closest_index_for_x(x.max(px(0.))),
            None => {
                let col = (f32::from(x) / f32::from(geometry.char_width)).round().max(0.0) as usize;
                self.buffer.offset_of(line, col)
            }
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The keys come back from the bar to the text.
        window.focus(&self.focus_handle);
        // A click on an entry of the list takes it.
        let clicked = self.geometry.as_ref().and_then(|geometry| geometry.proposal_rows.iter().find(|(_, row)| row.contains(&event.position)).map(|(index, _)| *index));
        if let Some(index) = clicked.filter(|_| self.proposal.is_some()) {
            self.accept_proposal(Some(index), cx);
            return;
        }
        self.proposal = None;
        self.selecting = true;
        self.goal_col = None;
        let at = self.index_for_point(event.position);
        if event.click_count >= 3 {
            let range = self.buffer.line_range(self.buffer.line_of(at));
            self.buffer.select(range);
        } else if event.click_count == 2 {
            let word = self.buffer.word_at(at);
            self.buffer.select(word);
        } else if event.modifiers.shift {
            self.buffer.select_to(at);
        } else {
            self.buffer.move_to(at);
        }
        self.refresh_hint();
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let at = self.index_for_point(event.position);
            self.buffer.select_to(at);
            self.moved(cx);
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        self.scroll_top -= f32::from(delta.y);
        self.scroll_left -= f32::from(delta.x);
        // The paint keeps both within the text.
        cx.notify();
    }
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range_utf16);
        actual_range.replace(self.buffer.range_to_utf16(&range));
        Some(self.buffer.text()[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selection()),
            reversed: self.buffer.reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.buffer.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let range = range_utf16.as_ref().map(|range| self.buffer.range_from_utf16(range)).or(self.marked.take());
        self.marked = None;
        let typed = range.is_none();
        match range {
            // What an input method composed replaces its draft.
            Some(range) => self.buffer.replace(range, text),
            // Plain typing: pairs, stepping over a closer.
            None => self.buffer.type_text(text),
        }
        self.changed(cx);
        // After a dot, and while the word after it is typed, the list of
        // what can stand there; typing goes on whatever it shows.
        if typed {
            self.propose(cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or(self.marked.take())
            .unwrap_or(self.buffer.selection());
        let start = range.start.min(self.buffer.text().len());
        self.buffer.replace(range, text);
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
        if let Some(selected) = new_selected_utf16 {
            // Given inside the composed text.
            let composed = CodeBuffer::new(text);
            let selected = composed.range_from_utf16(&selected);
            self.buffer.select(start + selected.start..start + selected.end);
        }
        self.changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry.as_ref()?;
        let range = self.buffer.range_from_utf16(&range_utf16);
        let (line, col) = self.buffer.line_col(range.start);
        let x = geometry.text_left + geometry.char_width * col as f32 - px(self.scroll_left);
        let y = geometry.bounds.top() + px(line as f32 * LINE_HEIGHT - self.scroll_top);
        Some(Bounds::new(point(x, y), size(px(1.), px(LINE_HEIGHT))))
    }

    fn character_index_for_point(&mut self, at: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.buffer.offset_to_utf16(self.index_for_point(at)))
    }
}

// ------------------------------------------------------------------ drawing

struct EditorElement {
    editor: gpui::Entity<CodeEditor>,
}

/// Everything one paint draws, worked out in `prepaint`.
struct Prepared {
    geometry: Geometry,
    scroll_top: f32,
    scroll_left: f32,
    gutter: Bounds<Pixels>,
    numbers: Vec<(ShapedLine, Point<Pixels>)>,
    current_line: Option<Bounds<Pixels>>,
    selection: Vec<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
    /// A dot in the gutter for each line in view that has a problem.
    problems: Vec<Bounds<Pixels>>,
    /// The thumb of the vertical scroll bar, when the text is taller than
    /// the box.
    thumb: Option<Bounds<Pixels>>,
    proposal: Option<ProposalBox>,
    hint: Option<(Bounds<Pixels>, ShapedLine)>,
    /// What Find found among the lines in view; true for the match the
    /// selection is on.
    found: Vec<(Bounds<Pixels>, bool)>,
    /// The bracket beside the caret and the one that answers it.
    brackets: Vec<Bounds<Pixels>>,
}

/// The list of completions as one paint draws it.
struct ProposalBox {
    bounds: Bounds<Pixels>,
    /// Each entry shown: its place in the list, its row, its name and
    /// what stands beside it.
    rows: Vec<(usize, Bounds<Pixels>, ShapedLine, ShapedLine)>,
    selected: usize,
    /// The line about the selected entry, under the list.
    about: Option<(Bounds<Pixels>, ShapedLine)>,
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The runs of one line: the colours of its tokens, a wavy line under a
/// line with a problem, a plain one under what is being composed.
fn runs_of(line: &str, spans: &[(Range<usize>, Token)], problem: bool, composed: Option<Range<usize>>) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let composed = composed.unwrap_or(0..0);
    for (range, token) in spans {
        // Cut the span where the composed text starts and ends.
        let mut cuts = vec![range.start, range.end];
        for edge in [composed.start, composed.end] {
            if edge > range.start && edge < range.end && line.is_char_boundary(edge) {
                cuts.push(edge);
            }
        }
        cuts.sort_unstable();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let color = token_color(*token);
            let underline = if start >= composed.start && end <= composed.end && !composed.is_empty() {
                Some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(1.),
                    wavy: false,
                })
            } else if problem && !line[start..end].trim().is_empty() {
                Some(UnderlineStyle {
                    color: Some(c(COLOR_PROBLEM)),
                    thickness: px(1.),
                    wavy: true,
                })
            } else {
                None
            };
            runs.push(TextRun {
                len: end - start,
                font: code_font(),
                color,
                background_color: None,
                underline,
                strikethrough: None,
            });
        }
    }
    runs
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Prepared>;

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
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let editor = self.editor.read(cx);
        let buffer = &editor.buffer;
        let plain = |text: SharedString, color: Hsla, window: &Window| {
            let run = TextRun {
                len: text.len(),
                font: code_font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().shape_line(text, px(FONT_SIZE), &[run], None)
        };
        let char_width = plain("0".into(), c(theme::TEXT), window).width.max(px(1.));
        let line_count = buffer.line_count();
        let digits = line_count.to_string().len().max(2);
        let gutter_width = char_width * (digits as f32 + 2.0);
        let text_left = bounds.left() + gutter_width + px(TEXT_GAP);
        let text_width = (bounds.size.width - gutter_width - px(TEXT_GAP)).max(px(20.));
        let height = f32::from(bounds.size.height);

        let (caret_line, caret_col) = buffer.line_col(buffer.caret());
        let mut scroll_top = editor.scroll_top;
        let mut scroll_left = editor.scroll_left;
        if editor.reveal {
            let top = caret_line as f32 * LINE_HEIGHT;
            scroll_top = scroll_top.min(top).max(top + LINE_HEIGHT - height);
            let x = f32::from(char_width) * caret_col as f32;
            let room = f32::from(text_width) - f32::from(char_width) * 2.0;
            scroll_left = scroll_left.min(x).max(x - room);
        }
        let content_height = line_count as f32 * LINE_HEIGHT;
        scroll_top = scroll_top.min(content_height - height).max(0.0);
        scroll_left = scroll_left.max(0.0);

        let first = (scroll_top / LINE_HEIGHT).floor() as usize;
        let last = (((scroll_top + height) / LINE_HEIGHT).ceil() as usize).min(line_count);
        let selected = buffer.selection();
        let focused = editor.focus_handle.is_focused(window);
        let mut lines = Vec::new();
        let mut numbers = Vec::new();
        let mut selection = Vec::new();
        let mut problems = Vec::new();
        let mut found = Vec::new();
        let mut brackets = Vec::new();
        let mut widest = px(0.);
        let line_top = |line: usize| bounds.top() + px(line as f32 * LINE_HEIGHT - scroll_top);
        // What to mark in the text: the matches of the search while its
        // bar is open, and the pair of brackets at the caret.
        let matches: &[Range<usize>] = match (&editor.finder, editor.bar) {
            (Some(finder), Some(Bar::Find | Bar::Replace)) => &finder.matches,
            _ => &[],
        };
        let pair = (selected.is_empty() && focused && !editor.read_only).then(|| search::bracket_pair(buffer.text(), buffer.caret())).flatten();
        for line in first..last {
            let range = buffer.line_range(line);
            let text = &buffer.text()[range.clone()];
            let state = editor.states.get(line).copied().unwrap_or_default();
            let (spans, _) = highlight_line(editor.language, text, state);
            let problem = editor.marks.iter().any(|(marked, _)| *marked as usize == line + 1);
            let composed = editor
                .marked
                .as_ref()
                .filter(|marked| marked.start < range.end && marked.end > range.start)
                .map(|marked| marked.start.max(range.start) - range.start..marked.end.min(range.end) - range.start);
            let runs = runs_of(text, &spans, problem, composed);
            let shaped = window.text_system().shape_line(text.to_string().into(), px(FONT_SIZE), &runs, None);
            widest = widest.max(shaped.width);

            // The part of the selection on this line; a line break taken
            // in shows as a little more than the text.
            if selected.start <= range.end && selected.end > range.start && !selected.is_empty() {
                let x0 = shaped.x_for_index(selected.start.max(range.start) - range.start);
                let mut x1 = shaped.x_for_index(selected.end.min(range.end) - range.start);
                if selected.end > range.end {
                    x1 += char_width * 0.6;
                }
                if x1 > x0 {
                    let left = text_left - px(scroll_left);
                    selection.push(Bounds::from_corners(
                        point(left + x0, line_top(line)),
                        point(left + x1, line_top(line) + px(LINE_HEIGHT)),
                    ));
                }
            }

            // The matches and the brackets on this line.
            let left = text_left - px(scroll_left);
            let mark = |on_line: Range<usize>| {
                Bounds::from_corners(
                    point(left + shaped.x_for_index(on_line.start), line_top(line)),
                    point(left + shaped.x_for_index(on_line.end), line_top(line) + px(LINE_HEIGHT)),
                )
            };
            let first = matches.partition_point(|found| found.end <= range.start);
            for found_range in matches[first..].iter().take_while(|found| found.start < range.end) {
                let on_line = found_range.start.max(range.start) - range.start..found_range.end.min(range.end) - range.start;
                found.push((mark(on_line), *found_range == selected));
            }
            if let Some((a, b)) = pair {
                for place in [a, b] {
                    if place >= range.start && place < range.end {
                        brackets.push(mark(place - range.start..place - range.start + 1));
                    }
                }
            }

            let number = (line + 1).to_string();
            let color = if line == caret_line { c(theme::TEXT) } else { c(COLOR_GUTTER_TEXT) };
            let shaped_number = plain(number.into(), color, window);
            let x = bounds.left() + gutter_width - char_width - shaped_number.width;
            numbers.push((shaped_number, point(x, line_top(line))));
            if problem {
                let side = px(6.);
                problems.push(Bounds::new(
                    point(bounds.left() + px(3.), line_top(line) + (px(LINE_HEIGHT) - side) / 2.0),
                    size(side, side),
                ));
            }
            lines.push((line, shaped));
        }
        // No further right than the longest line in view.
        scroll_left = scroll_left.min((f32::from(widest) - f32::from(text_width) + f32::from(char_width) * 2.0).max(0.0));

        let caret_shaped = lines.iter().find(|(line, _)| *line == caret_line);
        let caret_x = caret_shaped.map(|(line, shaped)| shaped.x_for_index(buffer.caret() - buffer.line_range(*line).start));
        let current_line = (selected.is_empty() && caret_shaped.is_some()).then(|| {
            Bounds::new(point(bounds.left(), line_top(caret_line)), size(bounds.size.width, px(LINE_HEIGHT)))
        });
        let caret = caret_x.filter(|_| focused && !editor.read_only).map(|x| {
            Bounds::new(point(text_left - px(scroll_left) + x, line_top(caret_line)), size(px(1.5), px(LINE_HEIGHT)))
        });
        let thumb = (content_height > height).then(|| {
            let length = (height * height / content_height).max(24.0);
            let top = scroll_top / (content_height - height) * (height - length);
            Bounds::new(point(bounds.right() - px(7.), bounds.top() + px(top)), size(px(5.), px(length)))
        });

        // The hint of the call and the list of completions hang from the
        // caret: the hint above its line, the list below, each on the
        // other side when it has no room on its own.
        let caret_at = caret_x.map(|x| point(text_left - px(scroll_left) + x, line_top(caret_line)));
        let fit_x = |x: Pixels, width: Pixels| x.min(bounds.right() - width - px(8.)).max(bounds.left() + px(2.));
        let row = px(PROPOSAL_ROW);
        let mut hint_below = false;
        let hint = caret_at.zip(editor.hint.as_ref()).map(|(at, hint)| {
            let active = hint.active.clone().unwrap_or(0..0);
            let run = |len: usize, color: Hsla| TextRun {
                len,
                font: code_font(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let runs: Vec<TextRun> = [
                run(active.start, c(theme::TEXT_MUTED)),
                run(active.len(), c(theme::ACCENT_DARK)),
                run(hint.signature.len() - active.end, c(theme::TEXT_MUTED)),
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect();
            let shaped = window.text_system().shape_line(hint.signature.clone().into(), px(FONT_SIZE), &runs, None);
            let width = (shaped.width + px(12.)).min(bounds.size.width - px(12.));
            hint_below = at.y - row - px(2.) < bounds.top();
            let y = if hint_below { at.y + px(LINE_HEIGHT) + px(2.) } else { at.y - row - px(2.) };
            (Bounds::new(point(fit_x(at.x, width), y), size(width, row)), shaped)
        });
        let proposal = caret_at.zip(editor.proposal.as_ref()).map(|(at, proposal)| {
            let entries = &proposal.completion.entries;
            let shown = entries.iter().enumerate().skip(proposal.first).take(PROPOSAL_ROWS);
            let about = entries.get(proposal.selected).map(|entry| entry.about).filter(|about| !about.is_empty());
            let count = entries.len().min(PROPOSAL_ROWS);
            let width = px(PROPOSAL_WIDTH).min(bounds.size.width - px(12.)).max(px(120.));
            let box_height = row * (count + about.is_some() as usize) as f32;
            let mut below = at.y + px(LINE_HEIGHT) + px(2.);
            if hint_below && hint.is_some() {
                below += row + px(2.);
            }
            let above = at.y - box_height - px(2.) - if hint.is_some() && !hint_below { row + px(2.) } else { px(0.) };
            let y = if below + box_height > bounds.bottom() && above >= bounds.top() { above } else { below };
            let origin = point(fit_x(at.x, width), y);
            let rows = shown
                .enumerate()
                .map(|(line, (index, entry))| {
                    let place = Bounds::new(point(origin.x, origin.y + row * line as f32), size(width, row));
                    (index, place, plain(entry.name.clone().into(), c(theme::TEXT), window), plain(entry.detail.clone().into(), c(theme::TEXT_MUTED), window))
                })
                .collect();
            let about = about.map(|about| {
                let place = Bounds::new(point(origin.x, origin.y + row * count as f32), size(width, row));
                (place, plain(about.into(), c(theme::TEXT_MUTED), window))
            });
            ProposalBox {
                bounds: Bounds::new(origin, size(width, box_height)),
                rows,
                selected: proposal.selected,
                about,
            }
        });
        Some(Prepared {
            geometry: Geometry {
                bounds,
                text_left,
                char_width,
                lines,
                proposal_rows: proposal.iter().flat_map(|proposal| proposal.rows.iter().map(|(index, place, _, _)| (*index, *place))).collect(),
            },
            scroll_top,
            scroll_left,
            gutter: Bounds::new(bounds.origin, size(gutter_width, bounds.size.height)),
            numbers,
            current_line,
            selection,
            caret,
            problems,
            thumb,
            proposal,
            hint,
            found,
            brackets,
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepared) = prepaint.take() else {
            return;
        };
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.editor.clone()), cx);
        let focused = focus_handle.is_focused(window);
        let line_height = px(LINE_HEIGHT);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(band) = prepared.current_line {
                window.paint_quad(fill(band, c(COLOR_CURRENT_LINE)));
            }
            // The text, cut at the line numbers when it is scrolled sideways.
            let text_area = Bounds::from_corners(point(prepared.gutter.right(), bounds.top()), bounds.bottom_right());
            window.with_content_mask(Some(ContentMask { bounds: text_area }), |window| {
                for (quad, current) in &prepared.found {
                    window.paint_quad(fill(*quad, c(if *current { COLOR_FOUND_CURRENT } else { COLOR_FOUND })));
                }
                for quad in &prepared.brackets {
                    window.paint_quad(gpui::quad(*quad, px(2.), c(COLOR_BRACKET), px(0.), gpui::transparent_black(), gpui::BorderStyle::Solid));
                }
                let tint = if focused { ca(0x0A64D2, 0.25) } else { ca(0x808080, 0.22) };
                for quad in &prepared.selection {
                    window.paint_quad(fill(*quad, tint));
                }
                for (line, shaped) in &prepared.geometry.lines {
                    let origin = point(
                        prepared.geometry.text_left - px(prepared.scroll_left),
                        bounds.top() + px(*line as f32 * LINE_HEIGHT - prepared.scroll_top),
                    );
                    let _ = shaped.paint(origin, line_height, window, cx);
                }
                if let Some(caret) = prepared.caret {
                    window.paint_quad(fill(caret, c(0x000000)));
                }
            });
            for (number, origin) in &prepared.numbers {
                let _ = number.paint(*origin, line_height, window, cx);
            }
            for dot in &prepared.problems {
                window.paint_quad(gpui::quad(
                    *dot,
                    px(3.),
                    c(COLOR_PROBLEM),
                    px(0.),
                    gpui::transparent_black(),
                    gpui::BorderStyle::Solid,
                ));
            }
            if let Some(thumb) = prepared.thumb {
                window.paint_quad(gpui::quad(
                    thumb,
                    px(2.5),
                    ca(0x000000, 0.25),
                    px(0.),
                    gpui::transparent_black(),
                    gpui::BorderStyle::Solid,
                ));
            }
            // Over the text: the hint of the call and the list.
            let panel = |place: Bounds<Pixels>, window: &mut Window| {
                window.paint_quad(gpui::quad(place, px(3.), c(0xFFFFFF), px(1.), c(theme::BORDER), gpui::BorderStyle::Solid));
            };
            let text_y = |place: &Bounds<Pixels>| place.top() + (place.size.height - line_height) / 2.0;
            if let Some((place, shaped)) = &prepared.hint {
                panel(*place, window);
                window.with_content_mask(Some(ContentMask { bounds: *place }), |window| {
                    let _ = shaped.paint(point(place.left() + px(6.), text_y(place)), line_height, window, cx);
                });
            }
            if let Some(list) = &prepared.proposal {
                panel(list.bounds, window);
                window.with_content_mask(Some(ContentMask { bounds: list.bounds }), |window| {
                    for (index, place, name, detail) in &list.rows {
                        if *index == list.selected {
                            window.paint_quad(fill(*place, ca(0x0A64D2, 0.16)));
                        }
                        let y = text_y(place);
                        let _ = name.paint(point(place.left() + px(6.), y), line_height, window, cx);
                        let _ = detail.paint(point(place.left() + px(6.) + name.width + px(10.), y), line_height, window, cx);
                    }
                    if let Some((place, about)) = &list.about {
                        window.paint_quad(fill(*place, c(theme::PANEL_BG)));
                        window.paint_quad(fill(Bounds::new(place.origin, size(place.size.width, px(1.))), c(theme::BORDER)));
                        let _ = about.paint(point(place.left() + px(6.), text_y(place)), line_height, window, cx);
                    }
                });
            }
        });
        let (geometry, scroll_top, scroll_left) = (prepared.geometry, prepared.scroll_top, prepared.scroll_left);
        self.editor.update(cx, |editor, _| {
            editor.geometry = Some(geometry);
            editor.scroll_top = scroll_top;
            editor.scroll_left = scroll_left;
            editor.reveal = false;
        });
    }
}

impl Render for CodeEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.bar.and_then(|bar| self.render_bar(bar, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::PANEL_BG))
            .key_context(CONTEXTS)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_text_start))
            .on_action(cx.listener(Self::select_text_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::shift_enter))
            .on_action(cx.listener(Self::tab))
            .on_action(cx.listener(Self::shift_tab))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::toggle_comment))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::replace))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::go_to_line_bar))
            .children(bar)
            // The text: the mouse belongs to it, not to the bar above.
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .child(EditorElement { editor: cx.entity() }),
            )
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_cut_into_runs_that_cover_it() {
        let line = "let n = 12; // é";
        let (spans, _) = highlight_line(Language::TypeScript, line, LineState::Code);
        for (problem, composed) in [(false, None), (true, None), (false, Some(2..6)), (true, Some(0..line.len()))] {
            let runs = runs_of(line, &spans, problem, composed.clone());
            assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), line.len(), "{problem} {composed:?}");
        }
        // A problem underlines the words, not the blanks between them.
        let runs = runs_of(line, &spans, true, None);
        assert!(runs.iter().any(|run| run.underline.is_some_and(|u| u.wavy)));
        // The keyword keeps its colour under the composed text.
        let runs = runs_of(line, &spans, false, Some(0..3));
        assert_eq!(runs[0].len, 3);
        assert_eq!(runs[0].color, c(COLOR_KEYWORD));
        assert!(runs[0].underline.is_some_and(|u| !u.wavy));
    }
}
