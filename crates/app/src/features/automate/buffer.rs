//! The text of the code editor and everything about editing it that
//! needs no screen: the selection, typing with its automatic indentation
//! and its pairs of brackets and quotes, indenting a selection, and the
//! editor's own undo and redo.

use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// One change to the text, as undo and redo replay it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    at: usize,
    removed: String,
    inserted: String,
    selection_before: (Range<usize>, bool),
    selection_after: (Range<usize>, bool),
    /// Characters typed one after the other are undone together.
    typing: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CodeBuffer {
    text: String,
    /// Byte range; empty when it is only a caret.
    selected: Range<usize>,
    /// The caret is at the start of the selection.
    reversed: bool,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// The text as last saved, to tell whether there are unsaved changes.
    saved: String,
    /// One step of indentation, as the text had it when it was opened.
    unit: String,
}

/// What one step of indentation is in a text: a tab when lines start
/// with tabs, else the smallest step by which one line is indented more
/// or less than the line before it (two blanks when no line is indented).
fn indent_unit_of(text: &str) -> String {
    let mut step = usize::MAX;
    let mut previous = 0;
    for line in text.split('\n') {
        if line.starts_with('\t') {
            return "\t".to_string();
        }
        if line.trim().is_empty() {
            continue;
        }
        let lead = line.len() - line.trim_start_matches(' ').len();
        // A difference of one blank is an alignment, not a step.
        let difference = lead.abs_diff(previous);
        if difference > 1 {
            step = step.min(difference);
        }
        previous = lead;
    }
    " ".repeat(if step == usize::MAX { 2 } else { step.min(8) })
}

const OPENERS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
const QUOTES: [char; 3] = ['"', '\'', '`'];

fn closer_of(open: char) -> Option<char> {
    OPENERS.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

fn is_closer(ch: char) -> bool {
    OPENERS.iter().any(|(_, c)| *c == ch)
}

impl CodeBuffer {
    pub fn new(text: &str) -> CodeBuffer {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        CodeBuffer {
            saved: text.clone(),
            unit: indent_unit_of(&text),
            text,
            ..CodeBuffer::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }

    pub fn reversed(&self) -> bool {
        self.reversed
    }

    pub fn caret(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selected.clone()]
    }

    /// True when the text is not the one last saved.
    pub fn is_dirty(&self) -> bool {
        self.text != self.saved
    }

    /// The text was written to its file as it is.
    pub fn mark_saved(&mut self) {
        self.saved = self.text.clone();
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // ---- positions ------------------------------------------------------

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    pub fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    /// Where a line starts and where it ends (before its line break).
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let mut start = 0;
        for (index, piece) in self.text.split('\n').enumerate() {
            if index == line {
                return start..start + piece.len();
            }
            start += piece.len() + 1;
        }
        self.text.len()..self.text.len()
    }

    /// The line an offset is on (0-based).
    pub fn line_of(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].matches('\n').count()
    }

    /// Line and column (in characters) of an offset, both 0-based.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = self.clamp(offset);
        let line = self.line_of(offset);
        let start = self.line_range(line).start;
        (line, self.text[start..offset].chars().count())
    }

    /// The offset of a line and a column; past the end of the line, its end.
    pub fn offset_of(&self, line: usize, col: usize) -> usize {
        let range = self.line_range(line.min(self.line_count() - 1));
        let text = &self.text[range.clone()];
        range.start + text.char_indices().nth(col).map_or(text.len(), |(i, _)| i)
    }

    pub fn previous_boundary(&self, offset: usize) -> usize {
        self.text[..offset]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    pub fn next_boundary(&self, offset: usize) -> usize {
        self.text[offset..]
            .graphemes(true)
            .next()
            .map_or(self.text.len(), |g| offset + g.len())
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

    /// The word around an offset (a double click selects it).
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        for (index, word) in self.text.split_word_bound_indices() {
            if offset < index + word.len() || index + word.len() == self.text.len() {
                return index..index + word.len();
            }
        }
        offset..offset
    }

    /// Where Home goes: the first character of the line that is not a
    /// blank, or the very start of the line when the caret is there.
    pub fn smart_home(&self, offset: usize) -> usize {
        let range = self.line_range(self.line_of(offset));
        let line = &self.text[range.clone()];
        let first = range.start + (line.len() - line.trim_start().len());
        if offset == first { range.start } else { first }
    }

    // ---- selection ------------------------------------------------------

    pub fn move_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        self.selected = offset..offset;
        self.reversed = false;
        self.break_typing();
    }

    /// Extend the selection from its fixed end to `offset`.
    pub fn select_to(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.break_typing();
    }

    pub fn select(&mut self, range: Range<usize>) {
        self.selected = self.clamp(range.start)..self.clamp(range.end.max(range.start));
        self.reversed = false;
        self.break_typing();
    }

    pub fn select_all(&mut self) {
        self.select(0..self.text.len());
    }

    /// Put the caret at the start of a line (1-based), as a click on a
    /// problem or an error of a script does.
    pub fn go_to_line(&mut self, line: u32, column: Option<u32>) {
        let line = (line.max(1) as usize - 1).min(self.line_count() - 1);
        let range = self.line_range(line);
        let offset = match column {
            Some(column) => self.offset_of(line, column.max(1) as usize - 1),
            None => {
                range.start + (self.text[range.clone()].len() - self.text[range].trim_start().len())
            }
        };
        self.move_to(offset);
    }

    // ---- changes --------------------------------------------------------

    /// The next character typed starts a new undo step.
    fn break_typing(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.typing = false;
        }
    }

    /// Replace a range and leave the selection as asked; one undo step,
    /// or part of the one before when both are typing.
    fn apply(
        &mut self,
        range: Range<usize>,
        new: &str,
        selection_after: (Range<usize>, bool),
        typing: bool,
    ) {
        let range = self.clamp(range.start)..self.clamp(range.end.max(range.start));
        if range.is_empty() && new.is_empty() {
            self.selected = selection_after.0;
            self.reversed = selection_after.1;
            return;
        }
        let removed = self.text[range.clone()].to_string();
        let selection_before = (self.selected.clone(), self.reversed);
        self.text.replace_range(range.clone(), new);
        self.selected = selection_after.0.clone();
        self.reversed = selection_after.1;
        self.redo.clear();
        // Typing goes on where the last typed character ended.
        if let Some(last) = self.undo.last_mut() {
            if typing
                && last.typing
                && removed.is_empty()
                && last.at + last.inserted.len() == range.start
            {
                last.inserted.push_str(new);
                last.selection_after = selection_after;
                return;
            }
        }
        self.undo.push(Edit {
            at: range.start,
            removed,
            inserted: new.to_string(),
            selection_before,
            selection_after,
            typing,
        });
    }

    /// Replace a range with text and leave the caret after it.
    pub fn replace(&mut self, range: Range<usize>, new: &str) {
        let start = self.clamp(range.start);
        let end = start + new.len();
        self.apply(range, new, (end..end, false), false);
    }

    /// Replace the selection (a paste, a cut, a composed character).
    pub fn insert(&mut self, new: &str) {
        self.replace(
            self.selected.clone(),
            &new.replace("\r\n", "\n").replace('\r', "\n"),
        );
    }

    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        self.text
            .replace_range(edit.at..edit.at + edit.inserted.len(), &edit.removed);
        self.selected = edit.selection_before.0.clone();
        self.reversed = edit.selection_before.1;
        self.redo.push(Edit {
            typing: false,
            ..edit
        });
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        self.text
            .replace_range(edit.at..edit.at + edit.removed.len(), &edit.inserted);
        self.selected = edit.selection_after.0.clone();
        self.reversed = edit.selection_after.1;
        self.undo.push(edit);
        true
    }

    // ---- typing ---------------------------------------------------------

    /// One step of indentation: what the text was indented with when it
    /// was opened (it does not change while the text is edited, or Tab
    /// would not undo ⇧Tab).
    pub fn indent_unit(&self) -> String {
        self.unit.clone()
    }

    fn char_before(&self, offset: usize) -> Option<char> {
        self.text[..offset].chars().next_back()
    }

    fn char_after(&self, offset: usize) -> Option<char> {
        self.text[offset..].chars().next()
    }

    /// Characters typed on the keyboard: brackets and quotes come in
    /// pairs, a closing one typed before itself is stepped over, and a
    /// closing brace on a blank line goes back one step.
    pub fn type_text(&mut self, typed: &str) {
        let mut chars = typed.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            let end = self.selected.start + typed.len();
            self.apply(self.selected.clone(), typed, (end..end, false), true);
            return;
        };
        let caret = self.selected.start;
        if !self.selected.is_empty() {
            // An opening bracket or a quote wraps what is selected.
            let pair = closer_of(ch).or(QUOTES.contains(&ch).then_some(ch));
            if let Some(close) = pair {
                let inside = self.selected_text().to_string();
                let wrapped = format!("{ch}{inside}{close}");
                let inner = caret + ch.len_utf8()..caret + ch.len_utf8() + inside.len();
                self.apply(
                    self.selected.clone(),
                    &wrapped,
                    (inner, self.reversed),
                    false,
                );
                return;
            }
            let end = caret + typed.len();
            self.apply(self.selected.clone(), typed, (end..end, false), true);
            return;
        }
        let after = self.char_after(caret);
        // Typing the closer that is already there steps over it.
        if (is_closer(ch) || QUOTES.contains(&ch)) && after == Some(ch) {
            let end = caret + ch.len_utf8();
            self.selected = end..end;
            self.reversed = false;
            self.break_typing();
            return;
        }
        let room = after.is_none_or(|next| {
            next.is_whitespace() || is_closer(next) || matches!(next, ';' | ',' | '.')
        });
        if let Some(close) = closer_of(ch) {
            if room {
                let end = caret + ch.len_utf8();
                self.apply(
                    caret..caret,
                    &format!("{ch}{close}"),
                    (end..end, false),
                    false,
                );
                return;
            }
        }
        if QUOTES.contains(&ch) {
            // Not inside a word ("don't"), not right after another quote.
            let before = self.char_before(caret);
            let apart = before.is_none_or(|prev| {
                !prev.is_alphanumeric() && prev != '_' && !QUOTES.contains(&prev) && prev != '\\'
            });
            if room && apart {
                let end = caret + ch.len_utf8();
                self.apply(caret..caret, &format!("{ch}{ch}"), (end..end, false), false);
                return;
            }
        }
        if ch == '}' {
            // On a line of blanks only, a closing brace goes back one step.
            let line = self.line_range(self.line_of(caret));
            let lead = &self.text[line.start..caret];
            let unit = self.indent_unit();
            if !lead.is_empty()
                && lead.trim().is_empty()
                && lead.ends_with(unit.as_str())
                && self.text[caret..line.end].trim().is_empty()
            {
                let start = caret - unit.len();
                let end = start + 1;
                self.apply(start..caret, "}", (end..end, false), false);
                return;
            }
        }
        let end = caret + typed.len();
        // A word and the blank after it are one step; the next word is
        // another.
        let new_word = ch.is_whitespace()
            && self
                .char_before(caret)
                .is_some_and(|prev| !prev.is_whitespace());
        self.apply(caret..caret, typed, (end..end, false), true);
        if new_word {
            self.break_typing();
        }
    }

    /// Return: the new line starts under the first character of the one
    /// left, one step further after an opening bracket; a closing bracket
    /// waiting right after the caret goes to a line of its own.
    pub fn newline(&mut self) {
        let caret = self.selected.start;
        let line = self.line_range(self.line_of(caret));
        let lead: String = self.text[line.start..caret.max(line.start)]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let before = self.text[line.start..caret].trim_end().chars().next_back();
        let opens = before.is_some_and(|ch| closer_of(ch).is_some());
        let unit = self.indent_unit();
        let mut new = format!("\n{lead}");
        if opens {
            new.push_str(&unit);
        }
        let caret_after = caret + new.len();
        let closes = before.and_then(closer_of).is_some_and(|close| {
            self.text[self.selected.end..]
                .trim_start_matches(' ')
                .starts_with(close)
        });
        if opens && closes {
            new.push_str(&format!("\n{lead}"));
        }
        self.apply(
            self.selected.clone(),
            &new,
            (caret_after..caret_after, false),
            false,
        );
    }

    /// Backspace: the selection, or the character before the caret; an
    /// empty pair goes as a whole, and the blanks that indent a line go
    /// one step at a time.
    pub fn backspace(&mut self) {
        if !self.selected.is_empty() {
            self.replace(self.selected.clone(), "");
            return;
        }
        let caret = self.selected.start;
        if caret == 0 {
            return;
        }
        let (before, after) = (self.char_before(caret), self.char_after(caret));
        if let (Some(open), Some(close)) = (before, after) {
            let pair = closer_of(open) == Some(close) || (QUOTES.contains(&open) && open == close);
            if pair {
                self.replace(caret - open.len_utf8()..caret + close.len_utf8(), "");
                return;
            }
        }
        let line = self.line_range(self.line_of(caret));
        let lead = &self.text[line.start..caret];
        let unit = self.indent_unit();
        if !lead.is_empty() && lead.chars().all(|c| c == ' ') && unit.starts_with(' ') {
            // Back to the previous step of the indentation.
            let step = match lead.len() % unit.len() {
                0 => unit.len(),
                rest => rest,
            };
            self.replace(caret - step..caret, "");
            return;
        }
        self.replace(self.previous_boundary(caret)..caret, "");
    }

    pub fn delete_forward(&mut self) {
        if self.selected.is_empty() {
            let caret = self.selected.start;
            self.replace(caret..self.next_boundary(caret), "");
        } else {
            self.replace(self.selected.clone(), "");
        }
    }

    /// The lines the selection touches: first and last (a selection that
    /// ends at the very start of a line does not touch that line).
    fn selected_lines(&self) -> (usize, usize) {
        let first = self.line_of(self.selected.start);
        let mut last = self.line_of(self.selected.end);
        if last > first && self.line_range(last).start == self.selected.end {
            last -= 1;
        }
        (first, last)
    }

    /// Rewrite the selected lines with `change` (one undo step); the
    /// selection then covers them, or the caret keeps its place in the
    /// line when there was no selection.
    fn change_lines(&mut self, change: impl Fn(&str) -> String) {
        let (first, last) = self.selected_lines();
        let block = self.line_range(first).start..self.line_range(last).end;
        let old = self.text[block.clone()].to_string();
        let new: Vec<String> = old.split('\n').map(&change).collect();
        let new = new.join("\n");
        if new == old {
            return;
        }
        let after = if self.selected.is_empty() {
            // The caret moves with the text of its line.
            let caret = self.selected.start;
            let in_line = caret - block.start;
            let delta = new.len() as isize - old.len() as isize;
            let lead_old = old.len() - old.trim_start_matches([' ', '\t']).len();
            let at = if in_line >= lead_old {
                (in_line as isize + delta).max(0) as usize
            } else {
                in_line.min(new.len() - new.trim_start_matches([' ', '\t']).len())
            };
            let at = block.start + at.min(new.len());
            (at..at, false)
        } else {
            (block.start..block.start + new.len(), self.reversed)
        };
        self.apply(block, &new, after, false);
    }

    /// Tab: with a selection, its lines move one step right; without, the
    /// caret goes to the next step.
    pub fn indent(&mut self) {
        let unit = self.indent_unit();
        if self.selected.is_empty() {
            let caret = self.selected.start;
            let new = if unit == "\t" {
                unit
            } else {
                let (_, col) = self.line_col(caret);
                " ".repeat(unit.len() - col % unit.len())
            };
            let end = caret + new.len();
            self.apply(caret..caret, &new, (end..end, false), false);
            return;
        }
        self.change_lines(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{unit}{line}")
            }
        });
    }

    /// ⇧Tab: the selected lines, or the line of the caret, move one step
    /// left.
    pub fn outdent(&mut self) {
        let unit = self.indent_unit();
        self.change_lines(|line| {
            if let Some(rest) = line.strip_prefix(unit.as_str()) {
                return rest.to_string();
            }
            // Less than a step of blanks: all of them go.
            let lead = line.len() - line.trim_start_matches(' ').len();
            line[lead.min(unit.len())..].to_string()
        });
    }

    /// ⌘/: put `//` before the selected lines, or take it away when all
    /// of them have it.
    pub fn toggle_comment(&mut self, marker: &str) {
        let (first, last) = self.selected_lines();
        let block = self.line_range(first).start..self.line_range(last).end;
        let lines: Vec<&str> = self.text[block].split('\n').collect();
        let all = lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .all(|line| line.trim_start().starts_with(marker));
        // The marker goes at the depth of the least indented line.
        let depth = lines
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.len() - line.trim_start().len())
            .min()
            .unwrap_or(0);
        let marker = marker.to_string();
        self.change_lines(move |line| {
            if line.trim().is_empty() {
                return line.to_string();
            }
            if all {
                let lead = line.len() - line.trim_start().len();
                let rest = &line[lead + marker.len()..];
                format!(
                    "{}{}",
                    &line[..lead],
                    rest.strip_prefix(' ').unwrap_or(rest)
                )
            } else {
                format!("{}{marker} {}", &line[..depth], &line[depth..])
            }
        });
    }

    // ---- UTF-16, for the input methods ----------------------------------

    pub fn offset_from_utf16(&self, offset: usize) -> usize {
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

    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer from a text in which `|` marks the caret, or two of them
    /// the selection.
    fn buffer(marked: &str) -> CodeBuffer {
        let first = marked.find('|').expect("a caret");
        let rest = &marked[first + 1..];
        let (text, range) = match rest.find('|') {
            Some(second) => (
                format!(
                    "{}{}{}",
                    &marked[..first],
                    &rest[..second],
                    &rest[second + 1..]
                ),
                first..first + second,
            ),
            None => (format!("{}{}", &marked[..first], rest), first..first),
        };
        let mut b = CodeBuffer::new(&text);
        b.select(range);
        b
    }

    /// The text with its caret or selection marked the same way.
    fn shown(b: &CodeBuffer) -> String {
        let mut text = b.text().to_string();
        let range = b.selection();
        if !range.is_empty() {
            text.insert(range.end, '|');
        }
        text.insert(range.start, '|');
        text
    }

    fn typed(marked: &str, keys: &str) -> String {
        let mut b = buffer(marked);
        for ch in keys.chars() {
            match ch {
                '\n' => b.newline(),
                '\u{8}' => b.backspace(),
                other => b.type_text(&other.to_string()),
            }
        }
        shown(&b)
    }

    #[test]
    fn brackets_and_quotes_come_in_pairs() {
        assert_eq!(typed("f|", "("), "f(|)");
        assert_eq!(typed("f|", "()"), "f()|");
        assert_eq!(typed("x = |;", "["), "x = [|];");
        assert_eq!(typed("x = |", "\"a\""), "x = \"a\"|");
        assert_eq!(typed("log(|)", "'"), "log('|')");
        // Not before a word, not inside one.
        assert_eq!(typed("|name", "("), "(|name");
        assert_eq!(typed("don|", "'"), "don'|");
        assert_eq!(typed("x = |name", "\""), "x = \"|name");
        // An opener wraps the selection, which stays selected.
        assert_eq!(typed("sum |a + b|", "("), "sum (|a + b|)");
        assert_eq!(typed("say |hi|", "\""), "say \"|hi|\"");
        // Any other character replaces it.
        assert_eq!(typed("say |hi|", "x"), "say x|");
        // Backspace between an empty pair takes both.
        assert_eq!(typed("f(|)", "\u{8}"), "f|");
        assert_eq!(typed("s = \"|\"", "\u{8}"), "s = |");
        assert_eq!(typed("f(|x)", "\u{8}"), "f|x)");
    }

    #[test]
    fn return_keeps_and_deepens_the_indentation() {
        assert_eq!(typed("  let a = 1;|", "\n"), "  let a = 1;\n  |");
        assert_eq!(
            typed("function main() {|}", "\n"),
            "function main() {\n  |\n}"
        );
        assert_eq!(typed("  if (x) {|", "\n"), "  if (x) {\n    |");
        assert_eq!(typed("  call(|)", "\n"), "  call(\n    |\n  )");
        // The step follows the text: four blanks here, a tab there.
        assert_eq!(
            typed("function f() {\n    a();\n    if (x) {|", "\n"),
            "function f() {\n    a();\n    if (x) {\n        |"
        );
        assert_eq!(
            typed("function f() {\n\ta();\n\tif (x) {|", "\n"),
            "function f() {\n\ta();\n\tif (x) {\n\t\t|"
        );
        // Return in the middle of a line carries the rest down.
        assert_eq!(typed("  a();| b();", "\n"), "  a();\n  | b();");
        // A closing brace on a blank line goes back one step.
        assert_eq!(
            typed("  if (x) {\n    a();\n    |", "}"),
            "  if (x) {\n    a();\n  }|"
        );
        assert_eq!(typed("  x = {|", "}"), "  x = {}|");
        // Backspace in the indentation goes back one step.
        assert_eq!(
            typed("  if (x) {\n    |a();", "\u{8}"),
            "  if (x) {\n  |a();"
        );
        assert_eq!(
            typed("  if (x) {\n   |a();", "\u{8}"),
            "  if (x) {\n  |a();"
        );
    }

    #[test]
    fn tab_and_shift_tab() {
        let mut b = buffer("a\n|b\n  c|\nd");
        b.indent();
        assert_eq!(shown(&b), "a\n|  b\n    c|\nd");
        b.indent();
        b.outdent();
        b.outdent();
        assert_eq!(shown(&b), "a\n|b\n  c|\nd");
        b.outdent();
        assert_eq!(shown(&b), "a\n|b\nc|\nd");
        // Nothing left to take: nothing changes, nothing to undo.
        let steps = b.undo.len();
        b.outdent();
        assert_eq!(b.undo.len(), steps);
        // A selection ending at the start of a line leaves that line.
        let mut b = buffer("|a\nb\n|c");
        b.indent();
        assert_eq!(b.text(), "  a\n  b\nc");
        // Without a selection, Tab goes to the next step and ⇧Tab takes a
        // step from the line, the caret staying on its character.
        let mut b = buffer("  a|b");
        b.indent();
        assert_eq!(shown(&b), "  a |b");
        b.outdent();
        assert_eq!(shown(&b), "a |b");
        let mut b = buffer("x\n    y|");
        b.outdent();
        assert_eq!(shown(&b), "x\ny|");
        // One undo per Tab.
        let mut b = buffer("|a\nb|");
        b.indent();
        assert!(b.undo());
        assert_eq!(shown(&b), "|a\nb|");
        assert!(b.redo());
        assert_eq!(shown(&b), "|  a\n  b|");
    }

    #[test]
    fn comments_are_toggled() {
        let mut b = buffer("  |a();\n    b();|\n");
        b.toggle_comment("//");
        assert_eq!(b.text(), "  // a();\n  //   b();\n");
        b.toggle_comment("//");
        assert_eq!(b.text(), "  a();\n    b();\n");
        let mut b = buffer("let x|;");
        b.toggle_comment("//");
        assert_eq!(shown(&b), "// let x|;");
    }

    #[test]
    fn typing_is_undone_word_by_word() {
        let mut b = buffer("|");
        for ch in "let total = 12".chars() {
            b.type_text(&ch.to_string());
        }
        assert_eq!(b.text(), "let total = 12");
        assert!(b.is_dirty());
        assert!(b.undo());
        assert_eq!(b.text(), "let total = ");
        assert!(b.undo());
        assert_eq!(b.text(), "let total ");
        assert!(b.undo() && b.undo());
        assert_eq!(shown(&b), "|");
        assert!(!b.is_dirty() && !b.can_undo());
        assert!(b.redo() && b.redo() && b.redo() && b.redo());
        assert_eq!(shown(&b), "let total = 12|");
        assert!(!b.redo());
        // Typing after an undo drops what could be redone.
        b.undo();
        b.type_text("x");
        assert!(!b.can_redo());
        // Moving the caret ends the run of typing.
        let mut b = buffer("|");
        b.type_text("a");
        b.type_text("b");
        b.move_to(1);
        b.type_text("c");
        assert_eq!(b.text(), "acb");
        b.undo();
        assert_eq!(b.text(), "ab");
        b.undo();
        assert_eq!(b.text(), "");
    }

    #[test]
    fn saving_marks_the_text_clean() {
        let mut b = CodeBuffer::new("a\r\nb");
        assert_eq!(b.text(), "a\nb");
        assert!(!b.is_dirty());
        b.move_to(1);
        b.type_text("x");
        assert!(b.is_dirty());
        b.mark_saved();
        assert!(!b.is_dirty());
        // Undoing past the save is a change again; redoing comes back.
        b.undo();
        assert!(b.is_dirty());
        b.redo();
        assert!(!b.is_dirty());
        // A paste keeps one kind of line break.
        b.insert("1\r\n2\r3");
        assert_eq!(b.text(), "ax1\n2\n3\nb");
    }

    #[test]
    fn lines_columns_and_words() {
        let mut b = CodeBuffer::new("né à\n  Sète();\n");
        assert_eq!(b.line_count(), 3);
        assert_eq!(b.line_range(1), 7..17);
        assert_eq!(b.line_range(2), 18..18);
        assert_eq!(b.line_col(9), (1, 2));
        // "é" and "à" take two bytes: columns count characters.
        assert_eq!(b.line_col(6), (0, 4));
        assert_eq!(b.offset_of(0, 2), 3);
        assert_eq!(b.offset_of(0, 99), 6);
        assert_eq!(b.offset_of(99, 0), 18);
        assert_eq!(b.word_at(10), 9..14);
        // Home goes to the first character, then to the start of the line.
        assert_eq!(b.smart_home(12), 9);
        assert_eq!(b.smart_home(9), 7);
        assert_eq!(b.smart_home(7), 9);
        b.go_to_line(2, None);
        assert_eq!(b.caret(), 9);
        b.go_to_line(2, Some(4));
        assert_eq!(b.caret(), 10);
        b.go_to_line(99, None);
        assert_eq!(b.caret(), 18);
        // Selecting backwards turns the selection round.
        b.move_to(9);
        b.select_to(3);
        assert_eq!((b.selection(), b.reversed(), b.caret()), (3..9, true, 3));
        assert_eq!(b.range_to_utf16(&(0..6)), 0..4);
        assert_eq!(b.range_from_utf16(&(0..4)), 0..6);
    }
}
