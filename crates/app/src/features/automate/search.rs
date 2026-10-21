//! Finding in the text of the editor: the places a word stands at, and
//! the bracket that answers the one beside the caret.

use std::ops::Range;

/// How a text is searched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    /// Capitals and small letters differ.
    pub match_case: bool,
    /// What is found must not touch a letter, a digit or `_` on either
    /// side.
    pub whole_word: bool,
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn same(a: char, b: char, match_case: bool) -> bool {
    a == b || (!match_case && a.to_lowercase().eq(b.to_lowercase()))
}

/// Every place `query` stands at in `text`, one after the other (never
/// one inside another).
pub(crate) fn find_all(text: &str, query: &str, options: Options) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    if query.is_empty() {
        return found;
    }
    let mut at = 0;
    while at < text.len() {
        // The end of a match that starts here.
        let mut rest = text[at..].char_indices();
        let mut end = Some(at);
        for wanted in query.chars() {
            end = match rest.next() {
                Some((offset, ch)) if same(ch, wanted, options.match_case) => {
                    Some(at + offset + ch.len_utf8())
                }
                _ => None,
            };
            if end.is_none() {
                break;
            }
        }
        let whole = |end: usize| {
            !options.whole_word
                || (!text[..at].chars().next_back().is_some_and(is_word)
                    && !text[end..].chars().next().is_some_and(is_word))
        };
        match end.filter(|end| whole(*end)) {
            Some(end) => {
                found.push(at..end);
                at = end;
            }
            None => at += text[at..].chars().next().map_or(1, char::len_utf8),
        }
    }
    found
}

/// The text with every match replaced: the part that changes (from the
/// first match to the end of the last) and what it becomes.
pub(crate) fn replace_all(
    text: &str,
    matches: &[Range<usize>],
    replacement: &str,
) -> Option<(Range<usize>, String)> {
    let (first, last) = (matches.first()?, matches.last()?);
    let mut out = String::new();
    let mut from = first.start;
    for range in matches {
        out.push_str(&text[from..range.start]);
        out.push_str(replacement);
        from = range.end;
    }
    Some((first.start..last.end, out))
}

/// The match to go to from `caret`: the first that starts at or after
/// it going forward, the last that ends at or before it going back,
/// round the ends of the text.
pub(crate) fn next_match(matches: &[Range<usize>], caret: usize, forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(if forward {
        matches
            .iter()
            .position(|range| range.start >= caret)
            .unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|range| range.end <= caret)
            .unwrap_or(matches.len() - 1)
    })
}

/// Which bytes of a script are code: not in a string or a comment.
fn code_mask(text: &str) -> Vec<bool> {
    let bytes = text.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    mask[i] = false;
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |at| i + 2 + at + 2);
                mask[i..end].fill(false);
                i = end;
            }
            quote @ (b'"' | b'\'' | b'`') => {
                mask[i] = false;
                i += 1;
                while i < bytes.len() && bytes[i] != quote && !(bytes[i] == b'\n' && quote != b'`')
                {
                    mask[i] = false;
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        mask[i + 1] = false;
                        i += 1;
                    }
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == quote {
                    mask[i] = false;
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    mask
}

/// The bracket beside the caret (the one before it first) and the one
/// that answers it, as their places in the text; `None` when the caret
/// touches no bracket, or the bracket has no partner.
pub(crate) fn bracket_pair(text: &str, caret: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let code = code_mask(text);
    let bracket = |at: usize| {
        code.get(at).copied().unwrap_or(false)
            && matches!(bytes[at], b'(' | b')' | b'[' | b']' | b'{' | b'}')
    };
    let at = caret
        .checked_sub(1)
        .filter(|at| bracket(*at))
        .or(Some(caret).filter(|at| bracket(*at)))?;
    let (open, close, forward) = match bytes[at] {
        b'(' => (b'(', b')', true),
        b'[' => (b'[', b']', true),
        b'{' => (b'{', b'}', true),
        b')' => (b'(', b')', false),
        b']' => (b'[', b']', false),
        _ => (b'{', b'}', false),
    };
    let mut depth = 0usize;
    let mut places: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new(at..bytes.len())
    } else {
        Box::new((0..=at).rev())
    };
    places.find_map(|place| {
        if !code[place] {
            return None;
        }
        let (deeper, shallower) = if forward {
            (open, close)
        } else {
            (close, open)
        };
        if bytes[place] == deeper {
            depth += 1;
        } else if bytes[place] == shallower {
            depth -= 1;
            if depth == 0 {
                return Some((at, place));
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, query: &str, match_case: bool, whole_word: bool) -> Vec<Range<usize>> {
        find_all(
            text,
            query,
            Options {
                match_case,
                whole_word,
            },
        )
    }

    #[test]
    fn words_are_found_with_and_without_case_and_as_whole_words() {
        let text = "let range = sheet.getRange(\"A1\");\nRANGE.range";
        assert_eq!(
            found(text, "range", false, false),
            [4..9, 21..26, 34..39, 40..45]
        );
        assert_eq!(found(text, "range", true, false), [4..9, 40..45]);
        assert_eq!(found(text, "range", false, true), [4..9, 34..39, 40..45]);
        assert!(found(text, "Range", true, true).is_empty());
        assert!(found(text, "", false, false).is_empty());
        // One after the other, never one inside another.
        assert_eq!(found("aaaa", "aa", true, false), [0..2, 2..4]);
        // Letters of more than one byte.
        assert_eq!(found("é É e", "é", false, false), [0..2, 3..5]);
        assert_eq!(found("été", "T", false, false), [2..3]);
    }

    #[test]
    fn the_next_and_the_previous_match_go_round() {
        let matches = [4..9, 21..26, 34..39];
        assert_eq!(next_match(&matches, 0, true), Some(0));
        assert_eq!(next_match(&matches, 9, true), Some(1));
        assert_eq!(next_match(&matches, 40, true), Some(0));
        assert_eq!(next_match(&matches, 21, false), Some(0));
        assert_eq!(next_match(&matches, 3, false), Some(2));
        assert_eq!(next_match(&[], 3, true), None);
    }

    #[test]
    fn everything_is_replaced_in_one_change() {
        let text = "a cat, a Cat, a dog";
        let matches = found(text, "cat", false, false);
        assert_eq!(
            replace_all(text, &matches, "bird"),
            Some((2..12, "bird, a bird".to_string()))
        );
        assert_eq!(replace_all(text, &[], "bird"), None);
    }

    #[test]
    fn brackets_answer_one_another() {
        let text = "f(a[1], \"(\") { g() } // )";
        assert_eq!(bracket_pair(text, 1), Some((1, 11)));
        assert_eq!(bracket_pair(text, 2), Some((1, 11)));
        assert_eq!(bracket_pair(text, 12), Some((11, 1)));
        assert_eq!(bracket_pair(text, 6), Some((5, 3)));
        assert_eq!(bracket_pair(text, 13), Some((13, 19)));
        // In a string or a comment a bracket is a letter like another.
        assert_eq!(bracket_pair(text, 9), None);
        assert_eq!(bracket_pair(text, text.len()), None);
        assert_eq!(bracket_pair("(()", 1), None);
        assert_eq!(bracket_pair("ab", 1), None);
    }
}
