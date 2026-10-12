//! A box holding several lines of text, for the messages of the dialog:
//! the text wraps at the edge of the box, Return starts a new line, and
//! the box scrolls to keep the caret in view.
//!
//! It answers to the keys of `text_input` (same key context); Tab and
//! Escape are left to the dialog.

use crate::text_input::{
    self, AltEnter, Backspace, Copy, Cut, Delete, Down, End, Enter, Home, Left, Paste, Right, SelectAll, SelectEnd,
    SelectHome, SelectLeft, SelectRight, ShiftEnter, Up,
};
use crate::theme::{self, c};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, GlobalElementId, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window, WrappedLine,
    div, fill, point, prelude::*, px, relative, rgb, rgba, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub struct TextArea {
    focus: FocusHandle,
    text: String,
    selection: Range<usize>,
    /// The caret is at the start of the selection, not at its end.
    reversed: bool,
    /// Text being composed with an input method.
    marked: Option<Range<usize>>,
    /// Characters the box takes at most.
    max_chars: usize,
    /// The paragraphs as last laid out, where, and how far scrolled.
    lines: Vec<WrappedLine>,
    bounds: Option<Bounds<Pixels>>,
    line_height: Pixels,
    scroll: Pixels,
    selecting: bool,
}

/// Where a position in the text falls among the laid-out paragraphs, from
/// the top-left corner of the first.
fn point_for(lines: &[WrappedLine], index: usize, line_height: Pixels) -> Point<Pixels> {
    let mut start = 0;
    let mut top = px(0.);
    for line in lines {
        if index <= start + line.len() {
            let at = line.position_for_index(index - start, line_height).unwrap_or_default();
            return point(at.x, top + at.y);
        }
        start += line.len() + 1;
        top += line.size(line_height).height;
    }
    point(px(0.), top)
}

/// The position in the text closest to a point of the laid-out paragraphs.
fn index_for(lines: &[WrappedLine], position: Point<Pixels>, line_height: Pixels) -> usize {
    let mut start = 0;
    let mut top = px(0.);
    for (i, line) in lines.iter().enumerate() {
        let height = line.size(line_height).height;
        if position.y < top + height || i + 1 == lines.len() {
            let local = point(
                position.x.max(px(0.)),
                (position.y - top).clamp(px(0.), (height - px(1.)).max(px(0.))),
            );
            let (Ok(at) | Err(at)) = line.closest_index_for_position(local, line_height);
            return start + at;
        }
        start += line.len() + 1;
        top += height;
    }
    0
}

/// What is left of `inserted` when the text may only grow to `max_chars`
/// characters and already holds `kept` of them.
fn fitting(inserted: &str, kept: usize, max_chars: usize) -> &str {
    let room = max_chars.saturating_sub(kept);
    match inserted.char_indices().nth(room) {
        Some((at, _)) => &inserted[..at],
        None => inserted,
    }
}

impl TextArea {
    pub fn new(text: &str, max_chars: usize, cx: &mut Context<Self>) -> Self {
        TextArea {
            focus: cx.focus_handle().tab_stop(true),
            text: fitting(text, 0, max_chars).to_string(),
            selection: 0..0,
            reversed: false,
            marked: None,
            max_chars,
            lines: Vec::new(),
            bounds: None,
            line_height: px(16.),
            scroll: px(0.),
            selecting: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.text = fitting(text, 0, self.max_chars).to_string();
        self.selection = 0..0;
        self.reversed = false;
        self.marked = None;
        self.scroll = px(0.);
        cx.notify();
    }

    fn caret(&self) -> usize {
        if self.reversed { self.selection.start } else { self.selection.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selection = offset..offset;
        self.reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selection.start = offset;
        } else {
            self.selection.end = offset;
        }
        if self.selection.end < self.selection.start {
            self.reversed = !self.reversed;
            self.selection = self.selection.end..self.selection.start;
        }
        cx.notify();
    }

    /// Replace a range (the selection when `None`) with `new`, as much of
    /// it as the box has room for.
    fn replace(&mut self, range: Option<Range<usize>>, new: &str, cx: &mut Context<Self>) {
        let range = range.unwrap_or(self.selection.clone());
        let kept = self.text.chars().count() - self.text[range.clone()].chars().count();
        let new = fitting(new, kept, self.max_chars);
        self.text.replace_range(range.clone(), new);
        let caret = range.start + new.len();
        self.selection = caret..caret;
        self.reversed = false;
        self.marked = None;
        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .find_map(|(at, _)| (at < offset).then_some(at))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .find_map(|(at, _)| (at > offset).then_some(at))
            .unwrap_or(self.text.len())
    }

    /// The position one line above or below the caret.
    fn vertical(&self, down: bool) -> usize {
        let at = point_for(&self.lines, self.caret(), self.line_height);
        let y = if down { at.y + self.line_height * 1.5 } else { at.y - self.line_height * 0.5 };
        if y < px(0.) {
            return 0;
        }
        let total = self
            .lines
            .iter()
            .fold(px(0.), |height, line| height + line.size(self.line_height).height);
        if y >= total {
            return self.text.len();
        }
        index_for(&self.lines, point(at.x, y), self.line_height)
    }

    /// Start and end of the paragraph the caret is in.
    fn paragraph(&self) -> Range<usize> {
        let caret = self.caret();
        let start = self.text[..caret].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[caret..].find('\n').map_or(self.text.len(), |at| caret + at);
        start..end
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.previous_boundary(self.caret()), cx);
        }
        self.replace(None, "", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            self.select_to(self.next_boundary(self.caret()), cx);
        }
        self.replace(None, "", cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() { self.previous_boundary(self.caret()) } else { self.selection.start };
        self.move_to(to, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let to = if self.selection.is_empty() { self.next_boundary(self.caret()) } else { self.selection.end };
        self.move_to(to, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.vertical(false), cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.vertical(true), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.caret()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.caret()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selection = 0..self.text.len();
        self.reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.paragraph().start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.paragraph().end, cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.paragraph().start, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.paragraph().end, cx);
    }

    // Return starts a new line here; it does not press OK.
    fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(None, "\n", cx);
    }

    fn shift_enter(&mut self, _: &ShiftEnter, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(None, "\n", cx);
    }

    fn alt_enter(&mut self, _: &AltEnter, _: &mut Window, cx: &mut Context<Self>) {
        self.replace(None, "\n", cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(None, &text.replace("\r\n", "\n").replace('\r', "\n"), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selection.clone()].to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[self.selection.clone()].to_string()));
            self.replace(None, "", cx);
        }
    }

    /// The position in the text under a point of the window.
    fn index_at(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.bounds else {
            return 0;
        };
        let local = point(position.x - bounds.left(), position.y - bounds.top() + self.scroll);
        index_for(&self.lines, local, self.line_height)
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = true;
        let at = self.index_at(event.position);
        if event.modifiers.shift {
            self.select_to(at, cx);
        } else {
            self.move_to(at, cx);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.index_at(event.position), cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
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
        self.text[..offset.min(self.text.len())].chars().map(char::len_utf16).sum()
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
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selection),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or(self.marked.clone());
        let start = range.as_ref().map_or(self.selection.start, |range| range.start);
        self.replace(range, text, cx);
        let caret = self.caret();
        self.marked = (caret > start).then_some(start..caret);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let at = point_for(&self.lines, range.start, self.line_height);
        Some(Bounds::new(
            point(bounds.left() + at.x, bounds.top() + at.y - self.scroll),
            size(px(1.), self.line_height),
        ))
    }

    fn character_index_for_point(&mut self, position: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_at(position)))
    }
}

impl Render for TextArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context(text_input::CONTEXT)
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
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::shift_enter))
            .on_action(cx.listener(Self::alt_enter))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(AreaElement { area: cx.entity() })
    }
}

/// The frame around a text area: `height` pixels tall, as wide as the room
/// left in its row.
pub fn text_box(area: &Entity<TextArea>, height: f32, window: &Window, cx: &App) -> impl IntoElement + use<> {
    let focused = area.read(cx).focus.is_focused(window);
    div()
        .flex_1()
        .min_w(px(40.))
        .h(px(height))
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(if focused { theme::ACCENT } else { 0xABABAB }))
        .rounded(px(3.))
        .px(px(6.))
        .py(px(3.))
        .line_height(px(17.))
        .overflow_hidden()
        .child(area.clone())
}

/// Lays the text out in the box and paints it with its caret.
struct AreaElement {
    area: Entity<TextArea>,
}

struct Prepaint {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
    scroll: Pixels,
    caret: PaintQuad,
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
    type PrepaintState = Prepaint;

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
        let area = self.area.read(cx);
        let style = window.text_style();
        let line_height = window.line_height();
        let text: SharedString = area.text.clone().into();
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let lines: Vec<WrappedLine> = window
            .text_system()
            .shape_text(text, font_size, &[run], Some(bounds.size.width), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();

        // Keep the caret in view.
        let at = point_for(&lines, area.caret(), line_height);
        let mut scroll = area.scroll;
        if at.y < scroll {
            scroll = at.y;
        } else if at.y + line_height > scroll + bounds.size.height {
            scroll = at.y + line_height - bounds.size.height;
        }
        let origin = point(bounds.left(), bounds.top() - scroll);
        let caret = fill(
            Bounds::new(point(origin.x + at.x, origin.y + at.y), size(px(1.), line_height)),
            rgb(0x000000),
        );

        // The selection, one band for each line of text it runs over.
        let mut selection = Vec::new();
        if !area.selection.is_empty() {
            let from = point_for(&lines, area.selection.start, line_height);
            let to = point_for(&lines, area.selection.end, line_height);
            let mut y = from.y;
            while y <= to.y {
                let left = if y == from.y { from.x } else { px(0.) };
                let right = if y == to.y { to.x } else { bounds.size.width };
                selection.push(fill(
                    Bounds::from_corners(
                        point(origin.x + left, origin.y + y),
                        point(origin.x + right, origin.y + y + line_height),
                    ),
                    rgba(0x0a64d240),
                ));
                y += line_height;
            }
        }
        Prepaint {
            lines,
            line_height,
            scroll,
            caret,
            selection,
        }
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
        let focus = self.area.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.area.clone()), cx);
        let focused = focus.is_focused(window);
        let lines = std::mem::take(&mut prepaint.lines);
        let (line_height, scroll) = (prepaint.line_height, prepaint.scroll);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if focused {
                for band in prepaint.selection.drain(..) {
                    window.paint_quad(band);
                }
            }
            let mut y = bounds.top() - scroll;
            for line in &lines {
                let _ = line.paint(point(bounds.left(), y), line_height, TextAlign::Left, None, window, cx);
                y += line.size(line_height).height;
            }
            if focused {
                window.paint_quad(prepaint.caret.clone());
            }
        });
        self.area.update(cx, |area, _| {
            area.lines = lines;
            area.bounds = Some(bounds);
            area.line_height = line_height;
            area.scroll = scroll;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::fitting;

    #[test]
    fn the_box_takes_no_more_than_its_limit() {
        assert_eq!(fitting("hello", 0, 10), "hello");
        assert_eq!(fitting("hello", 7, 10), "hel");
        assert_eq!(fitting("hello", 10, 10), "");
        assert_eq!(fitting("hello", 12, 10), "");
        // Characters, not bytes.
        assert_eq!(fitting("été\nici", 0, 5), "été\ni");
    }
}
