//! A text box of several lines with word wrap and IME support: where a
//! note is typed in place, and the fields of a comment card.
//!
//! It answers the keys of the single-line `TextInput` (same key context),
//! plus those that only make sense on several lines. Return starts a new
//! line; ⌘Return is reported as `AreaEvent::Submit`.

use crate::text_input as line;
use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, FontWeight, GlobalElementId,
    Hsla, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    SharedString, Style, TextAlign, TextRun, TextStyle, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions,
    div, fill, point, prelude::*, px, relative, rgb, rgba, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_area,
    [SelectUp, SelectDown, TextStart, TextEnd, SelectTextStart, SelectTextEnd, Submit]
);

/// Key context of the keys a text box adds to those of `TextInput`.
const CONTEXT: &str = "TextArea";
/// A text box answers the keys of both.
const CONTEXTS: &str = "TextInput TextArea";

pub fn bind_keys(cx: &mut App) {
    let c = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("cmd-up", TextStart, c),
        KeyBinding::new("cmd-down", TextEnd, c),
        KeyBinding::new("cmd-shift-up", SelectTextStart, c),
        KeyBinding::new("cmd-shift-down", SelectTextEnd, c),
        KeyBinding::new("cmd-enter", Submit, c),
        KeyBinding::new("ctrl-enter", Submit, c),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaEvent {
    /// The text changed through typing, pasting or deleting.
    Changed,
    /// ⌘Return: post the comment.
    Submit,
    Escape,
    Focused,
    Blurred,
}

// ------------------------------------------------------------------- buffer

/// The text and its selection: everything about editing that needs no
/// layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Buffer {
    pub text: String,
    /// Byte range; empty when it is only a caret.
    pub selected: Range<usize>,
    /// The caret is at the start of the selection.
    pub reversed: bool,
}

impl Buffer {
    pub fn caret(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    pub fn move_to(&mut self, offset: usize) {
        let offset = offset.min(self.text.len());
        self.selected = offset..offset;
        self.reversed = false;
    }

    /// Extend the selection from its fixed end to `offset`.
    pub fn select_to(&mut self, offset: usize) {
        let offset = offset.min(self.text.len());
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
    }

    /// Replace a range and leave the caret after the new text.
    pub fn replace(&mut self, range: Range<usize>, new: &str) {
        let start = range.start.min(self.text.len());
        let end = range.end.clamp(start, self.text.len());
        self.text.replace_range(start..end, new);
        self.move_to(start + new.len());
    }

    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.text.len())
    }

    pub fn previous_word(&self, offset: usize) -> usize {
        let mut last = 0;
        for (index, word) in self.text.split_word_bound_indices() {
            if index >= offset {
                break;
            }
            if !word.trim().is_empty() {
                last = index;
            }
        }
        last
    }

    pub fn next_word(&self, offset: usize) -> usize {
        for (index, word) in self.text.split_word_bound_indices() {
            let end = index + word.len();
            if end > offset && !word.trim().is_empty() {
                return end;
            }
        }
        self.text.len()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let (mut utf8, mut utf16) = (0, 0);
        for ch in self.text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let (mut utf8, mut utf16) = (0, 0);
        for ch in self.text.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

// ------------------------------------------------------------------- layout

/// One line as shown: a stretch of a paragraph between two wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    paragraph: usize,
    /// Byte offsets inside the paragraph.
    start: usize,
    end: usize,
}

/// The text laid out for a width.
struct Shaped {
    /// One per paragraph (the text between two line breaks).
    lines: Vec<WrappedLine>,
    /// Offset of each paragraph in the whole text.
    starts: Vec<usize>,
    rows: Vec<Row>,
    line_height: Pixels,
}

impl Shaped {
    fn new(mut lines: Vec<WrappedLine>, line_height: Pixels) -> Shaped {
        // Whatever happens there is a row for the caret.
        if lines.is_empty() {
            lines.push(WrappedLine::default());
        }
        let mut starts = Vec::with_capacity(lines.len());
        let mut rows = Vec::new();
        let mut offset = 0;
        for (paragraph, line) in lines.iter().enumerate() {
            starts.push(offset);
            let mut start = 0;
            for boundary in line.wrap_boundaries() {
                let end = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                rows.push(Row { paragraph, start, end });
                start = end;
            }
            rows.push(Row {
                paragraph,
                start,
                end: line.len(),
            });
            // The line break is not part of the paragraph.
            offset += line.len() + 1;
        }
        Shaped {
            lines,
            starts,
            rows,
            line_height,
        }
    }

    fn height(&self) -> Pixels {
        self.line_height * self.rows.len() as f32
    }

    fn row_range(&self, row: &Row) -> Range<usize> {
        let base = self.starts[row.paragraph];
        base + row.start..base + row.end
    }

    /// The row showing the caret at `index`: at a wrap, the end of the
    /// upper one.
    fn row_of(&self, index: usize) -> usize {
        self.rows
            .iter()
            .position(|row| index <= self.row_range(row).end)
            .unwrap_or(self.rows.len().saturating_sub(1))
    }

    fn x_in_row(&self, row: &Row, index: usize) -> Pixels {
        let layout = &self.lines[row.paragraph].unwrapped_layout;
        let local = index.saturating_sub(self.starts[row.paragraph]).clamp(row.start, row.end);
        layout.x_for_index(local) - layout.x_for_index(row.start)
    }

    /// Top-left corner of the caret at `index`, from the corner of the text.
    fn position(&self, index: usize) -> Point<Pixels> {
        let row = self.row_of(index);
        point(self.x_in_row(&self.rows[row], index), self.line_height * row as f32)
    }

    /// The offset closest to a point given from the corner of the text.
    fn index_at(&self, at: Point<Pixels>) -> usize {
        let row = ((at.y / self.line_height).floor() as isize).clamp(0, self.rows.len() as isize - 1) as usize;
        self.index_in_row(row, at.x)
    }

    fn index_in_row(&self, row: usize, x: Pixels) -> usize {
        let row = &self.rows[row];
        let layout = &self.lines[row.paragraph].unwrapped_layout;
        let local = layout
            .closest_index_for_x(x.max(px(0.)) + layout.x_for_index(row.start))
            .clamp(row.start, row.end);
        self.starts[row.paragraph] + local
    }
}

// --------------------------------------------------------------------- view

pub struct TextArea {
    focus_handle: FocusHandle,
    buffer: Buffer,
    /// Shown in grey while the box is empty.
    pub placeholder: SharedString,
    /// Drawn in bold when the text starts with it: the "Author:" that
    /// opens a note.
    pub bold_prefix: Option<String>,
    /// Lines the box shows at least.
    pub min_rows: usize,
    /// The box grows with its text up to this height, then scrolls.
    pub max_height: Option<f32>,
    marked: Option<Range<usize>>,
    shaped: Option<Shaped>,
    bounds: Option<Bounds<Pixels>>,
    scroll_y: Pixels,
    /// Where Up and Down aim horizontally while they follow one another.
    goal_x: Option<Pixels>,
    selecting: bool,
    was_focused: bool,
    pending_caret: Option<Point<Pixels>>,
}

impl EventEmitter<AreaEvent> for TextArea {}

impl TextArea {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TextArea {
            focus_handle: cx.focus_handle(),
            buffer: Buffer::default(),
            placeholder: SharedString::default(),
            bold_prefix: None,
            min_rows: 1,
            max_height: None,
            marked: None,
            shaped: None,
            bounds: None,
            scroll_y: px(0.),
            goal_x: None,
            selecting: false,
            was_focused: false,
            pending_caret: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.buffer.text
    }

    /// Replace the whole text and put the caret at its end (no event).
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.buffer.text = text.into();
        self.buffer.move_to(self.buffer.text.len());
        self.marked = None;
        self.scroll_y = px(0.);
        self.goal_x = None;
        cx.notify();
    }

    /// Put the caret under a point of the window that lies beside the
    /// text (the margin of the box around it). Clicks on the text itself
    /// are handled by the box.
    pub fn caret_to_point(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if self.bounds.is_some_and(|bounds| bounds.contains(&position)) {
            return;
        }
        let index = self.index_for_point(position);
        self.buffer.move_to(index);
        self.goal_x = None;
        cx.notify();
    }

    /// Put the caret under a point of the window once the text is laid
    /// out: the click that opened the box.
    pub fn caret_at_next_paint(&mut self, position: Point<Pixels>) {
        self.pending_caret = Some(position);
    }

    fn index_for_point(&self, position: Point<Pixels>) -> usize {
        match (&self.shaped, &self.bounds) {
            (Some(shaped), Some(bounds)) if !self.buffer.text.is_empty() => {
                shaped.index_at(point(position.x - bounds.left(), position.y - bounds.top() + self.scroll_y))
            }
            _ => 0,
        }
    }

    /// The runs of the text as drawn: the bold prefix, the underline of
    /// what an input method is composing.
    fn runs(&self, style: &TextStyle) -> Vec<TextRun> {
        let text = &self.buffer.text;
        let bold = self
            .bold_prefix
            .as_ref()
            .filter(|prefix| !prefix.is_empty() && text.starts_with(prefix.as_str()))
            .map_or(0, |prefix| prefix.len());
        let marked = self.marked.clone().unwrap_or(0..0);
        let mut cuts = vec![0, bold, marked.start.min(text.len()), marked.end.min(text.len()), text.len()];
        cuts.sort_unstable();
        cuts.dedup();
        let mut runs = Vec::new();
        for pair in cuts.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let mut font = style.font();
            if end <= bold {
                font.weight = FontWeight::BOLD;
            }
            runs.push(TextRun {
                len: end - start,
                font,
                color: style.color,
                background_color: None,
                underline: (start >= marked.start && end <= marked.end && !marked.is_empty()).then_some(
                    UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.),
                        wavy: false,
                    },
                ),
                strikethrough: None,
            });
        }
        runs
    }

    fn shape(&self, width: Pixels, style: &TextStyle, font_size: Pixels, line_height: Pixels, window: &Window) -> Shaped {
        let (text, runs): (SharedString, Vec<TextRun>) = if self.buffer.text.is_empty() {
            let run = TextRun {
                len: self.placeholder.len(),
                font: style.font(),
                color: placeholder_color(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            (self.placeholder.clone(), vec![run])
        } else {
            (self.buffer.text.clone().into(), self.runs(style))
        };
        let lines = window
            .text_system()
            .shape_text(text, font_size, &runs, Some(width.max(px(8.))), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();
        Shaped::new(lines, line_height)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.goal_x = None;
        cx.emit(AreaEvent::Changed);
        cx.notify();
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let range = self.marked.take().unwrap_or(self.buffer.selected.clone());
        self.buffer.replace(range, text);
        self.changed(cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buffer.move_to(offset);
        self.goal_x = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.buffer.select_to(offset);
        self.goal_x = None;
        cx.notify();
    }

    /// The offset one row above or below the caret, keeping its column.
    fn vertical(&mut self, down: bool) -> usize {
        let caret = self.buffer.caret();
        let Some(shaped) = &self.shaped else {
            return caret;
        };
        let row = shaped.row_of(caret);
        let x = self.goal_x.unwrap_or_else(|| shaped.x_in_row(&shaped.rows[row], caret));
        self.goal_x = Some(x);
        match (down, row) {
            (false, 0) => 0,
            (true, last) if last + 1 >= shaped.rows.len() => self.buffer.text.len(),
            (false, _) => shaped.index_in_row(row - 1, x),
            (true, _) => shaped.index_in_row(row + 1, x),
        }
    }

    /// Start or end of the row the caret is on.
    fn row_edge(&self, end: bool) -> usize {
        let caret = self.buffer.caret();
        match &self.shaped {
            Some(shaped) if !self.buffer.text.is_empty() => {
                let range = shaped.row_range(&shaped.rows[shaped.row_of(caret)]);
                if end { range.end } else { range.start }
            }
            _ => caret,
        }
    }

    fn left(&mut self, _: &line::Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.buffer.selected.is_empty() {
            self.buffer.previous_boundary(self.buffer.caret())
        } else {
            self.buffer.selected.start
        };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &line::Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.buffer.selected.is_empty() {
            self.buffer.next_boundary(self.buffer.caret())
        } else {
            self.buffer.selected.end
        };
        self.move_to(to, cx);
    }

    fn up(&mut self, _: &line::Up, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(false);
        self.buffer.move_to(to);
        cx.notify();
    }

    fn down(&mut self, _: &line::Down, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(true);
        self.buffer.move_to(to);
        cx.notify();
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(false);
        self.buffer.select_to(to);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let to = self.vertical(true);
        self.buffer.select_to(to);
        cx.notify();
    }

    fn select_left(&mut self, _: &line::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.previous_boundary(self.buffer.caret()), cx);
    }

    fn select_right(&mut self, _: &line::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.next_boundary(self.buffer.caret()), cx);
    }

    fn select_all(&mut self, _: &line::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.buffer.selected = 0..self.buffer.text.len();
        self.buffer.reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &line::Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(false), cx);
    }

    fn end(&mut self, _: &line::End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.row_edge(true), cx);
    }

    fn select_home(&mut self, _: &line::SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_edge(false), cx);
    }

    fn select_end(&mut self, _: &line::SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.row_edge(true), cx);
    }

    fn text_start(&mut self, _: &TextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn text_end(&mut self, _: &TextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.buffer.text.len(), cx);
    }

    fn select_text_start(&mut self, _: &SelectTextStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_text_end(&mut self, _: &SelectTextEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.buffer.text.len(), cx);
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
        if self.buffer.selected.is_empty() {
            self.buffer.select_to(self.buffer.previous_word(self.buffer.caret()));
        }
        self.insert("", cx);
    }

    fn backspace(&mut self, _: &line::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.selected.is_empty() {
            self.buffer.select_to(self.buffer.previous_boundary(self.buffer.caret()));
        }
        self.insert("", cx);
    }

    fn delete(&mut self, _: &line::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.buffer.selected.is_empty() {
            self.buffer.select_to(self.buffer.next_boundary(self.buffer.caret()));
        }
        self.insert("", cx);
    }

    fn enter(&mut self, _: &line::Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", cx);
    }

    fn shift_enter(&mut self, _: &line::ShiftEnter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", cx);
    }

    fn alt_enter(&mut self, _: &line::AltEnter, _: &mut Window, cx: &mut Context<Self>) {
        self.insert("\n", cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AreaEvent::Submit);
    }

    fn escape(&mut self, _: &line::Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(AreaEvent::Escape);
    }

    fn show_character_palette(&mut self, _: &line::ShowCharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &line::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.insert(&text.replace("\r\n", "\n").replace('\r', "\n"), cx);
        }
    }

    fn copy(&mut self, _: &line::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.selected.is_empty() {
            let text = self.buffer.text[self.buffer.selected.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn cut(&mut self, _: &line::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.buffer.selected.is_empty() {
            let text = self.buffer.text[self.buffer.selected.clone()].to_string();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.insert("", cx);
        }
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = true;
        self.goal_x = None;
        let at = self.index_for_point(event.position);
        if event.click_count >= 2 {
            let start = self.buffer.previous_word(at);
            let end = self.buffer.next_word(start).max(start);
            self.buffer.selected = start..end;
            self.buffer.reversed = false;
        } else if event.modifiers.shift {
            self.buffer.select_to(at);
        } else {
            self.buffer.move_to(at);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let at = self.index_for_point(event.position);
            self.buffer.select_to(at);
            cx.notify();
        }
    }
}

fn placeholder_color() -> Hsla {
    gpui::hsla(0., 0., 0., 0.45)
}

impl EntityInputHandler for TextArea {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.buffer.range_from_utf16(&range_utf16);
        actual_range.replace(self.buffer.range_to_utf16(&range));
        Some(self.buffer.text[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.buffer.range_to_utf16(&self.buffer.selected),
            reversed: self.buffer.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.buffer.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or(self.marked.take())
            .unwrap_or(self.buffer.selected.clone());
        self.marked = None;
        self.buffer.replace(range, text);
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.buffer.range_from_utf16(range))
            .or(self.marked.take())
            .unwrap_or(self.buffer.selected.clone());
        let start = range.start.min(self.buffer.text.len());
        self.buffer.replace(range, text);
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
        if let Some(selected) = new_selected_utf16 {
            // Given inside the composed text.
            let composed = Buffer {
                text: text.to_string(),
                ..Buffer::default()
            };
            let selected = composed.range_from_utf16(&selected);
            self.buffer.selected = start + selected.start..start + selected.end;
        }
        self.changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let shaped = self.shaped.as_ref()?;
        let range = self.buffer.range_from_utf16(&range_utf16);
        let origin = bounds.origin + shaped.position(range.start) - point(px(0.), self.scroll_y);
        Some(Bounds::new(origin, size(px(1.), shaped.line_height)))
    }

    fn character_index_for_point(&mut self, at: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.buffer.offset_to_utf16(self.index_for_point(at)))
    }
}

struct AreaElement {
    area: Entity<TextArea>,
}

struct Prepared {
    shaped: Shaped,
    scroll_y: Pixels,
    caret: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for AreaElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AreaElement {
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
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let area = self.area.clone();
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        // As tall as the wrapped text, within the limits of the box.
        let layout = window.request_measured_layout(style, move |known, available, window, cx| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(width) => width,
                _ => px(240.),
            });
            let area = area.read(cx);
            let shaped = area.shape(width, &text_style, font_size, line_height, window);
            let mut height = line_height * shaped.rows.len().max(area.min_rows) as f32;
            if let Some(max) = area.max_height {
                height = height.min(px(max));
            }
            size(width, height)
        });
        (layout, ())
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
        let area = self.area.read(cx);
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let shaped = area.shape(bounds.size.width, &text_style, font_size, line_height, window);
        let empty = area.buffer.text.is_empty();
        let caret_at = if empty { Point::default() } else { shaped.position(area.buffer.caret()) };

        // Keep the caret in view.
        let mut scroll_y = area.scroll_y;
        if caret_at.y < scroll_y {
            scroll_y = caret_at.y;
        }
        if caret_at.y + line_height > scroll_y + bounds.size.height {
            scroll_y = caret_at.y + line_height - bounds.size.height;
        }
        scroll_y = scroll_y.min(shaped.height() - bounds.size.height).max(px(0.));

        let origin = point(bounds.left(), bounds.top() - scroll_y);
        let selected = area.buffer.selected.clone();
        let mut selection = Vec::new();
        let mut caret = None;
        if selected.is_empty() || empty {
            caret = Some(fill(
                Bounds::new(origin + caret_at, size(px(1.), line_height)),
                rgb(0x000000),
            ));
        } else {
            for (index, row) in shaped.rows.iter().enumerate() {
                let range = shaped.row_range(row);
                // A line break taken in the selection shows as a little
                // more than the text of its row.
                let breaks = shaped.rows.get(index + 1).is_none_or(|next| next.paragraph != row.paragraph);
                let touches = selected.start < range.end || (breaks && selected.start == range.end);
                if !(touches && selected.end > range.start) {
                    continue;
                }
                let x0 = shaped.x_in_row(row, selected.start.max(range.start));
                let mut x1 = shaped.x_in_row(row, selected.end.min(range.end));
                if breaks && selected.end > range.end {
                    x1 += font_size * 0.4;
                }
                let y = line_height * index as f32;
                selection.push(fill(
                    Bounds::from_corners(origin + point(x0, y), origin + point(x1, y + line_height)),
                    rgba(0x0a64d240),
                ));
            }
        }
        Some(Prepared {
            shaped,
            scroll_y,
            caret,
            selection,
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
        let focus_handle = self.area.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.area.clone()), cx);
        let focused = focus_handle.is_focused(window);
        let line_height = prepared.shaped.line_height;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused {
                for quad in prepared.selection {
                    window.paint_quad(quad);
                }
            }
            let mut y = bounds.top() - prepared.scroll_y;
            for line in &prepared.shaped.lines {
                let _ = line.paint(point(bounds.left(), y), line_height, TextAlign::Left, None, window, cx);
                y += line_height * (line.wrap_boundaries().len() + 1) as f32;
            }
            if let Some(caret) = prepared.caret.filter(|_| focused) {
                window.paint_quad(caret);
            }
        });
        let (shaped, scroll_y) = (prepared.shaped, prepared.scroll_y);
        self.area.update(cx, |area, cx| {
            area.shaped = Some(shaped);
            area.bounds = Some(bounds);
            area.scroll_y = scroll_y;
            if let Some(position) = area.pending_caret.take() {
                let index = area.index_for_point(position);
                area.buffer.move_to(index);
                cx.notify();
            }
            if focused != area.was_focused {
                area.was_focused = focused;
                cx.emit(if focused { AreaEvent::Focused } else { AreaEvent::Blurred });
            }
        });
    }
}

impl Render for TextArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .key_context(CONTEXTS)
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
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
            .on_action(cx.listener(Self::alt_enter))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::escape))
            // Tab has nowhere to go from here and must not reach the grid.
            .on_action(|_: &line::Tab, _, _| {})
            .on_action(|_: &line::ShiftTab, _, _| {})
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(AreaElement { area: cx.entity() })
    }
}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str, caret: usize) -> Buffer {
        let mut buffer = Buffer {
            text: text.to_string(),
            ..Buffer::default()
        };
        buffer.move_to(caret);
        buffer
    }

    #[test]
    fn selecting_and_replacing() {
        let mut b = buffer("Ada:\nCheck", 5);
        // Extending backwards turns the selection round; the caret leads.
        b.select_to(2);
        assert_eq!((b.selected.clone(), b.reversed, b.caret()), (2..5, true, 2));
        b.select_to(8);
        assert_eq!((b.selected.clone(), b.reversed, b.caret()), (5..8, false, 8));
        b.replace(b.selected.clone(), "Lo");
        assert_eq!((b.text.as_str(), b.selected.clone()), ("Ada:\nLock", 7..7));
        // Out-of-range offsets are brought back inside the text.
        b.replace(40..90, "!");
        assert_eq!(b.text, "Ada:\nLock!");
        b.move_to(400);
        assert_eq!(b.caret(), b.text.len());
    }

    #[test]
    fn boundaries_follow_characters_and_words() {
        let b = buffer("né à Sète\nici", 0);
        // "é" and "à" take two bytes each.
        assert_eq!(b.next_boundary(1), 3);
        assert_eq!(b.previous_boundary(3), 1);
        assert_eq!(b.next_word(0), 3);
        assert_eq!(b.next_word(3), 6);
        assert_eq!(b.previous_word(6), 4);
        // Words are found across the line break.
        assert_eq!(b.next_word(12), b.text.len());
        assert_eq!(b.previous_word(b.text.len()), 13);
        assert_eq!(b.previous_boundary(0), 0);
        assert_eq!(b.next_boundary(b.text.len()), b.text.len());
    }

    #[test]
    fn utf16_offsets() {
        let b = buffer("a😀é", 0);
        // The emoji is two UTF-16 units and four bytes.
        assert_eq!(b.range_to_utf16(&(1..5)), 1..3);
        assert_eq!(b.range_from_utf16(&(1..3)), 1..5);
        assert_eq!(b.offset_to_utf16(b.text.len()), 4);
    }
}
