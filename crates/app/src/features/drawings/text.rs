//! The text of shapes: how it is laid out in a box (lines wrapped at the
//! width of the box, aligned, held at the top, the middle or the bottom),
//! and the field that types it in place. The same field is the description
//! box of the Alt Text pane.

use super::geometry::wrap;
use crate::app as grid_keys;
use crate::text_input as keys;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, GlobalElementId, Hsla, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ShapedLine, SharedString, StrikethroughStyle, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, WindowTextSystem, div, fill, point, prelude::*, px, relative, rgba,
    size,
};
use opensheet_engine::drawing::{TextAlign, TextAnchor};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// How the text of a box looks.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLook {
    pub font: Font,
    /// Size in pixels.
    pub size: f32,
    pub color: Hsla,
    pub underline: bool,
    pub strike: bool,
    pub align: TextAlign,
    pub anchor: TextAnchor,
}

impl TextLook {
    pub fn line_height(&self) -> f32 {
        (self.size * 1.2).round().max(1.0)
    }

    fn run(&self, len: usize, marked: bool) -> TextRun {
        let line = |on: bool| {
            on.then_some(UnderlineStyle {
                color: Some(self.color),
                thickness: px((self.size / 14.0).max(1.0)),
                wavy: false,
            })
        };
        TextRun {
            len,
            font: self.font.clone(),
            color: self.color,
            background_color: None,
            underline: line(self.underline || marked),
            strikethrough: self.strike.then_some(StrikethroughStyle {
                color: Some(self.color),
                thickness: px((self.size / 14.0).max(1.0)),
            }),
        }
    }

    /// Shape one line; `marked` is the part being composed (underlined),
    /// in bytes of `text`.
    fn shape(&self, text: &str, marked: Option<Range<usize>>, ts: &WindowTextSystem) -> ShapedLine {
        let shown: SharedString = text.replace(['\n', '\r', '\t'], " ").into();
        let mut runs = Vec::new();
        match marked.filter(|m| m.start < m.end && m.end <= text.len()) {
            Some(marked) => {
                for (len, on) in [(marked.start, false), (marked.len(), true), (text.len() - marked.end, false)] {
                    if len > 0 {
                        runs.push(self.run(len, on));
                    }
                }
            }
            None => runs.push(self.run(shown.len(), false)),
        }
        ts.shape_line(shown, px(self.size), &runs, None)
    }
}

/// One line of a laid-out text.
pub struct TextLine {
    /// The bytes of the text it shows; a line feed that ends it is left
    /// out.
    pub range: Range<usize>,
    pub shaped: ShapedLine,
    /// Its left end and its top, from the top-left corner of the box.
    pub x: f32,
    pub y: f32,
    /// Bytes before the blanks that end the line.
    shown: usize,
    /// The line ends because the box is too narrow, not because the text
    /// says so.
    wrapped: bool,
}

/// A text laid out in a box.
pub struct Laid {
    pub lines: Vec<TextLine>,
    pub line_height: f32,
}

/// Lay a text out in a box `width` wide and `height` high. `marked` is the
/// part an input method is composing.
pub fn lay_out(
    text: &str,
    look: &TextLook,
    width: f32,
    height: f32,
    marked: Option<&Range<usize>>,
    ts: &WindowTextSystem,
) -> Laid {
    let line_height = look.line_height();
    let width = width.max(1.0);
    let mut lines = Vec::new();
    let mut start = 0;
    for paragraph in text.split('\n') {
        let whole = look.shape(paragraph, None, ts);
        let x_at = |index: usize| f32::from(whole.x_for_index(index));
        let pieces = wrap(paragraph, width, &x_at);
        let count = pieces.len();
        for (i, piece) in pieces.into_iter().enumerate() {
            let range = start + piece.start..start + piece.end;
            let part = &paragraph[piece];
            // The composed part, in the bytes of this line.
            let composing = marked.and_then(|m| {
                let (from, to) = (m.start.max(range.start), m.end.min(range.end));
                (from < to).then(|| from - range.start..to - range.start)
            });
            let shaped = look.shape(part, composing, ts);
            let shown = part.trim_end().len();
            let shown_width = f32::from(shaped.x_for_index(shown));
            let x = match look.align {
                TextAlign::Left | TextAlign::Justify => 0.0,
                TextAlign::Center => (width - shown_width) / 2.0,
                TextAlign::Right => width - shown_width,
            };
            lines.push(TextLine {
                range,
                shaped,
                x,
                y: 0.0,
                shown,
                wrapped: i + 1 < count,
            });
        }
        start += paragraph.len() + 1;
    }
    let total = lines.len() as f32 * line_height;
    let top = match look.anchor {
        TextAnchor::Top => 0.0,
        TextAnchor::Middle => (height - total) / 2.0,
        TextAnchor::Bottom => height - total,
    };
    for (i, line) in lines.iter_mut().enumerate() {
        line.y = (top + i as f32 * line_height).round();
    }
    Laid { lines, line_height }
}

impl Laid {
    /// The line holding a byte offset.
    fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|line| line.range.start <= index).unwrap_or(0)
    }

    /// Where the caret stands before a byte offset: its x and the top of
    /// its line.
    pub fn caret(&self, index: usize) -> (f32, f32) {
        let Some(line) = self.lines.get(self.line_of(index)) else {
            return (0.0, 0.0);
        };
        let local = index.saturating_sub(line.range.start).min(line.range.len());
        (line.x + f32::from(line.shaped.x_for_index(local)), line.y)
    }

    /// The byte offset closest to a point of the box.
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        let Some(first) = self.lines.first() else {
            return 0;
        };
        let row = (((y - first.y) / self.line_height).floor().max(0.0) as usize).min(self.lines.len() - 1);
        self.index_on(row, x)
    }

    /// The byte offset closest to `x` on a line.
    fn index_on(&self, row: usize, x: f32) -> usize {
        let line = &self.lines[row];
        let local = line.shaped.closest_index_for_x(px(x - line.x));
        // After the blanks of a wrapped line the caret would show on the
        // next one.
        let limit = if line.wrapped { line.shown } else { line.range.len() };
        line.range.start + local.min(limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextAreaEvent {
    Changed,
    /// Escape: the owner decides what leaving means.
    Escape,
    Blurred,
}

/// A text of several lines being typed.
pub struct TextArea {
    focus: FocusHandle,
    content: String,
    selection: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    pub look: TextLook,
    laid: Option<Laid>,
    bounds: Option<Bounds<Pixels>>,
    selecting: bool,
    was_focused: bool,
    /// The x the caret keeps while it moves up and down.
    column: Option<f32>,
    undone: Vec<(String, Range<usize>)>,
    redone: Vec<(String, Range<usize>)>,
}

impl EventEmitter<TextAreaEvent> for TextArea {}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextArea {
    /// A field holding `text`, with the caret at its end.
    pub fn new(text: impl Into<String>, look: TextLook, cx: &mut Context<Self>) -> TextArea {
        let content = text.into();
        let end = content.len();
        TextArea {
            focus: cx.focus_handle().tab_stop(true),
            content,
            selection: end..end,
            reversed: false,
            marked: None,
            look,
            laid: None,
            bounds: None,
            selecting: false,
            was_focused: false,
            column: None,
            undone: Vec::new(),
            redone: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// Replace the whole text (no event), keeping the caret where it can.
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selection = end..end;
        self.reversed = false;
        self.marked = None;
        self.undone.clear();
        self.redone.clear();
        cx.notify();
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.selection = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    /// Put the caret before a byte offset of the text.
    pub fn set_caret(&mut self, index: usize, cx: &mut Context<Self>) {
        let mut index = index.min(self.content.len());
        while !self.content.is_char_boundary(index) {
            index -= 1;
        }
        self.move_to(index, cx);
    }

    /// Insert text at the caret, as if typed.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.replace(self.selection.clone(), text, cx);
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    fn move_to(&mut self, index: usize, cx: &mut Context<Self>) {
        let index = index.min(self.content.len());
        self.selection = index..index;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, index: usize, cx: &mut Context<Self>) {
        let index = index.min(self.content.len());
        if self.reversed {
            self.selection.start = index;
        } else {
            self.selection.end = index;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        cx.notify();
    }

    /// Change the text: what every edit goes through.
    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.undone.push((self.content.clone(), self.selection.clone()));
        if self.undone.len() > 200 {
            self.undone.remove(0);
        }
        self.redone.clear();
        self.content.replace_range(range.clone(), text);
        let caret = range.start + text.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.column = None;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn previous_word(&self, offset: usize) -> usize {
        let mut last = 0;
        for (index, word) in self.content.split_word_bound_indices() {
            if index >= offset {
                break;
            }
            if !word.trim().is_empty() {
                last = index;
            }
        }
        last
    }

    fn next_word(&self, offset: usize) -> usize {
        for (index, word) in self.content.split_word_bound_indices() {
            let end = index + word.len();
            if end > offset && !word.trim().is_empty() {
                return end;
            }
        }
        self.content.len()
    }

    /// The ends of the line (as drawn) the caret is on.
    fn line_ends(&self) -> (usize, usize) {
        match &self.laid {
            Some(laid) if !laid.lines.is_empty() => {
                let line = &laid.lines[laid.line_of(self.cursor())];
                let end = if line.wrapped { line.range.start + line.shown } else { line.range.end };
                (line.range.start, end)
            }
            _ => (0, self.content.len()),
        }
    }

    /// The offset one line above (`-1`) or below (`1`) the caret, at the
    /// column it had when it started moving.
    fn vertical(&mut self, by: i32) -> usize {
        let Some(laid) = &self.laid else {
            return self.cursor();
        };
        let row = laid.line_of(self.cursor());
        let x = self.column.unwrap_or_else(|| laid.caret(self.cursor()).0);
        let target = row as i32 + by;
        let index = if target < 0 {
            0
        } else if target as usize >= laid.lines.len() {
            self.content.len()
        } else {
            laid.index_on(target as usize, x)
        };
        self.column = Some(x);
        index
    }

    fn index_for(&self, position: Point<Pixels>) -> usize {
        match (&self.laid, &self.bounds) {
            (Some(laid), Some(bounds)) => laid.index_at(
                f32::from(position.x - bounds.origin.x),
                f32::from(position.y - bounds.origin.y),
            ),
            _ => self.content.len(),
        }
    }

    // ---- keys ---------------------------------------------------------------

    fn left(&mut self, _: &keys::Left, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        if self.selection.is_empty() {
            self.move_to(self.previous_boundary(self.cursor()), cx);
        } else {
            self.move_to(self.selection.start, cx);
        }
    }

    fn right(&mut self, _: &keys::Right, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        if self.selection.is_empty() {
            self.move_to(self.next_boundary(self.cursor()), cx);
        } else {
            self.move_to(self.selection.end, cx);
        }
    }

    fn up(&mut self, _: &keys::Up, _: &mut Window, cx: &mut Context<Self>) {
        let index = self.vertical(-1);
        self.move_to(index, cx);
    }

    fn down(&mut self, _: &keys::Down, _: &mut Window, cx: &mut Context<Self>) {
        let index = self.vertical(1);
        self.move_to(index, cx);
    }

    fn select_left(&mut self, _: &keys::SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &keys::SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    fn select_up(&mut self, _: &grid_keys::ExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        let index = self.vertical(-1);
        self.select_to(index, cx);
    }

    fn select_down(&mut self, _: &grid_keys::ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        let index = self.vertical(1);
        self.select_to(index, cx);
    }

    fn select_all_text(&mut self, _: &keys::SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn home(&mut self, _: &keys::Home, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        self.move_to(self.line_ends().0, cx);
    }

    fn end(&mut self, _: &keys::End, _: &mut Window, cx: &mut Context<Self>) {
        self.column = None;
        self.move_to(self.line_ends().1, cx);
    }

    fn select_home(&mut self, _: &keys::SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_ends().0, cx);
    }

    fn select_end(&mut self, _: &keys::SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.line_ends().1, cx);
    }

    fn word_left(&mut self, _: &keys::WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor()), cx);
    }

    fn word_right(&mut self, _: &keys::WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &keys::SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word(self.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &keys::SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word(self.cursor()), cx);
    }

    fn delete_word_left(&mut self, _: &keys::DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.previous_word(self.cursor()), cx);
        }
        self.replace(self.selection.clone(), "", cx);
    }

    fn backspace(&mut self, _: &keys::Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.replace(self.selection.clone(), "", cx);
    }

    fn delete(&mut self, _: &keys::Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.replace(self.selection.clone(), "", cx);
    }

    fn new_line(&mut self, cx: &mut Context<Self>) {
        self.replace(self.selection.clone(), "\n", cx);
    }

    fn paste(&mut self, _: &keys::Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selection.clone(), &text.replace("\r\n", "\n").replace('\r', "\n"), cx);
        }
    }

    fn copy(&mut self, _: &keys::Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selection.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &keys::Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selection.clone()].to_string()));
            self.replace(self.selection.clone(), "", cx);
        }
    }

    /// Undo and redo of what was typed in the field.
    fn step_back(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (from, to) = if forward {
            (&mut self.redone, &mut self.undone)
        } else {
            (&mut self.undone, &mut self.redone)
        };
        let Some((text, selection)) = from.pop() else {
            return;
        };
        to.push((std::mem::replace(&mut self.content, text), self.selection.clone()));
        self.selection = selection.start.min(self.content.len())..selection.end.min(self.content.len());
        self.reversed = false;
        self.marked = None;
        cx.emit(TextAreaEvent::Changed);
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.column = None;
        let at = self.index_for(event.position);
        if event.click_count >= 3 {
            // The whole paragraph.
            let start = self.content[..at].rfind('\n').map_or(0, |i| i + 1);
            let end = self.content[at..].find('\n').map_or(self.content.len(), |i| at + i);
            self.selection = start..end;
            self.reversed = false;
            cx.notify();
        } else if event.click_count == 2 {
            let start = self.previous_word(self.next_boundary(at));
            let end = self.next_word(start);
            self.selection = start..end.max(start);
            self.reversed = false;
            cx.notify();
        } else if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.selecting = true;
            self.move_to(at, cx);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let at = self.index_for(event.position);
            self.select_to(at, cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())].chars().map(char::len_utf16).sum()
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

impl EntityInputHandler for TextArea {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(&mut self, _ignore_disabled: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        new_selection_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone())
            .unwrap_or(self.selection.clone());
        self.replace(range.clone(), text, cx);
        self.marked = (!text.is_empty()).then(|| range.start..range.start + text.len());
        if let Some(selected) = new_selection_utf16 {
            // Offsets inside the composed text, which is plain UTF-16 here.
            let inner = |offset: usize| {
                let mut utf8 = 0;
                let mut utf16 = 0;
                for ch in text.chars() {
                    if utf16 >= offset {
                        break;
                    }
                    utf16 += ch.len_utf16();
                    utf8 += ch.len_utf8();
                }
                range.start + utf8
            };
            self.selection = inner(selected.start)..inner(selected.end);
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let laid = self.laid.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let (x, y) = laid.caret(range.start);
        Some(Bounds::new(
            point(bounds.origin.x + px(x), bounds.origin.y + px(y)),
            size(px(2.0), px(laid.line_height)),
        ))
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for(position)))
    }
}

struct TextAreaElement {
    area: Entity<TextArea>,
}

impl IntoElement for TextAreaElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextAreaElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Laid>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
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
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let area = self.area.read(cx);
        Some(lay_out(
            &area.content,
            &area.look,
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            area.marked.as_ref(),
            window.text_system(),
        ))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(laid) = prepaint.take() else {
            return;
        };
        let (focus, selection, cursor, color) = {
            let area = self.area.read(cx);
            (area.focus.clone(), area.selection.clone(), area.cursor(), area.look.color)
        };
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.area.clone()), cx);
        let focused = focus.is_focused(window);
        let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
        let line_height = px(laid.line_height);
        if focused && !selection.is_empty() {
            for line in &laid.lines {
                let (from, to) = (selection.start.max(line.range.start), selection.end.min(line.range.end));
                if from > to {
                    continue;
                }
                let x0 = line.x + f32::from(line.shaped.x_for_index(from - line.range.start));
                let mut x1 = line.x + f32::from(line.shaped.x_for_index(to - line.range.start));
                // The end of a line that is selected with what follows it.
                if selection.end > line.range.end {
                    x1 += laid.line_height * 0.3;
                }
                if x1 > x0 {
                    window.paint_quad(fill(Bounds::from_corners(at(x0, line.y), at(x1, line.y + laid.line_height)), rgba(0x0a64d240)));
                }
            }
        }
        for line in &laid.lines {
            let _ = line.shaped.paint(at(line.x, line.y), line_height, window, cx);
        }
        if focused && selection.is_empty() {
            let (x, y) = laid.caret(cursor);
            window.paint_quad(fill(Bounds::new(at(x.round(), y), size(px(1.0), line_height)), color));
        }
        self.area.update(cx, |area, cx| {
            area.laid = Some(laid);
            area.bounds = Some(bounds);
            if focused != area.was_focused {
                area.was_focused = focused;
                if !focused {
                    area.selecting = false;
                    cx.emit(TextAreaEvent::Blurred);
                }
            }
        });
    }
}

impl Render for TextArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context(keys::CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all_text))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(|this, _: &keys::Enter, _, cx| this.new_line(cx)))
            .on_action(cx.listener(|this, _: &keys::ShiftEnter, _, cx| this.new_line(cx)))
            .on_action(cx.listener(|this, _: &keys::AltEnter, _, cx| this.new_line(cx)))
            .on_action(cx.listener(|_, _: &keys::Escape, _, cx| cx.emit(TextAreaEvent::Escape)))
            .on_action(|_: &keys::ShowCharacterPalette, window, _| window.show_character_palette())
            // The field sits over the sheet, whose shortcuts would
            // otherwise act on the cells behind it: the ones that mean
            // something in a text do it here, the others do nothing.
            .on_action(cx.listener(|this, _: &grid_keys::JumpUp, _, cx| this.move_to(0, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::GoStart, _, cx| this.move_to(0, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::JumpDown, _, cx| this.move_to(usize::MAX, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::GoEnd, _, cx| this.move_to(usize::MAX, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::ExtendJumpUp, _, cx| this.select_to(0, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::ExtendJumpDown, _, cx| this.select_to(usize::MAX, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::SelectRow, _, cx| this.insert(" ", cx)))
            .on_action(cx.listener(|this, _: &grid_keys::Undo, _, cx| this.step_back(false, cx)))
            .on_action(cx.listener(|this, _: &grid_keys::Redo, _, cx| this.step_back(true, cx)))
            .on_action(|_: &grid_keys::JumpLeft, _, _| {})
            .on_action(|_: &grid_keys::JumpRight, _, _| {})
            .on_action(|_: &grid_keys::ExtendJumpLeft, _, _| {})
            .on_action(|_: &grid_keys::ExtendJumpRight, _, _| {})
            .on_action(|_: &grid_keys::EditCell, _, _| {})
            .on_action(|_: &grid_keys::PasteValues, _, _| {})
            .on_action(|_: &grid_keys::FillDown, _, _| {})
            .on_action(|_: &grid_keys::FillRight, _, _| {})
            .on_action(|_: &grid_keys::PageUp, _, _| {})
            .on_action(|_: &grid_keys::PageDown, _, _| {})
            .on_action(|_: &grid_keys::SelectColumn, _, _| {})
            .on_action(|_: &grid_keys::InsertDate, _, _| {})
            .on_action(|_: &grid_keys::AutoSum, _, _| {})
            .on_action(|_: &grid_keys::InsertRows, _, _| {})
            .on_action(|_: &grid_keys::DeleteRows, _, _| {})
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(TextAreaElement { area: cx.entity() })
    }
}
