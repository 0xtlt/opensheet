//! Colouring of source text: TypeScript for the scripts, VBA for the
//! modules of a project. One line at a time, each line starting in the
//! state the line before it ended in (inside a block comment or a
//! template string), so that only the lines shown need colouring.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    TypeScript,
    Vba,
}

/// What a stretch of a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    Plain,
    Keyword,
    String,
    Comment,
    Number,
    /// A name of the Office Scripts API (`ExcelScript`, `Workbook` after
    /// it) or a type of the language.
    Type,
}

/// What a line starts inside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LineState {
    #[default]
    Code,
    BlockComment,
    /// A template string: `…` over several lines.
    Template,
}

const TS_KEYWORDS: [&str; 52] = [
    "function", "let", "const", "var", "if", "else", "for", "while", "do", "return", "break", "continue", "switch", "case",
    "default", "new", "typeof", "instanceof", "in", "of", "true", "false", "null", "undefined", "try", "catch", "finally",
    "throw", "class", "extends", "implements", "interface", "type", "enum", "async", "await", "this", "super", "import",
    "export", "from", "as", "void", "delete", "static", "public", "private", "protected", "readonly", "get", "set", "yield",
];

const TS_TYPES: [&str; 12] =
    ["string", "number", "boolean", "any", "unknown", "never", "object", "ExcelScript", "Array", "Date", "Math", "console"];

const VBA_KEYWORDS: [&str; 96] = [
    "sub", "function", "end", "dim", "as", "if", "then", "else", "elseif", "for", "each", "in", "next", "to", "step", "do",
    "loop", "while", "wend", "until", "select", "case", "with", "set", "let", "get", "property", "public", "private",
    "friend", "static", "const", "option", "explicit", "base", "compare", "module", "call", "exit", "goto", "gosub", "on",
    "error", "resume", "return", "new", "nothing", "true", "false", "null", "empty", "and", "or", "not", "xor", "mod", "is",
    "like", "byval", "byref", "optional", "paramarray", "type", "enum", "declare", "lib", "alias", "redim", "preserve",
    "erase", "stop", "attribute", "me", "withevents", "event", "raiseevent", "implements", "global", "typeof", "addressof",
    "string", "integer", "long", "longlong", "longptr", "single", "double", "boolean", "variant", "object", "date", "byte",
    "currency", "ptrsafe", "eqv", "imp",
];

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

/// The end of the number starting at `at`.
fn number_end(line: &str, at: usize) -> usize {
    let bytes = line.as_bytes();
    let mut end = at;
    let hex = line[at..].starts_with("0x") || line[at..].starts_with("0X") || line[at..].starts_with("&H") || line[at..].starts_with("&h");
    if hex {
        end += 2;
        while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
            end += 1;
        }
        return end;
    }
    while end < bytes.len() {
        let b = bytes[end];
        let exponent_sign = (b == b'+' || b == b'-') && matches!(bytes[end - 1], b'e' | b'E') && end > at;
        if b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'e' || b == b'E' || exponent_sign {
            end += 1;
        } else {
            break;
        }
    }
    end
}

fn push(out: &mut Vec<(Range<usize>, Token)>, range: Range<usize>, token: Token) {
    if range.is_empty() {
        return;
    }
    match out.last_mut() {
        Some((last, kind)) if *kind == token && last.end == range.start => last.end = range.end,
        _ => out.push((range, token)),
    }
}

fn typescript(line: &str, mut state: LineState) -> (Vec<(Range<usize>, Token)>, LineState) {
    let mut out = Vec::new();
    let mut at = 0;
    // The name before this one was `ExcelScript.`: this one is of the API.
    let mut after_namespace = false;
    while at < line.len() {
        let rest = &line[at..];
        match state {
            LineState::BlockComment => {
                let end = rest.find("*/").map_or(line.len(), |i| at + i + 2);
                if rest.contains("*/") {
                    state = LineState::Code;
                }
                push(&mut out, at..end, Token::Comment);
                at = end;
                continue;
            }
            LineState::Template => {
                let mut end = line.len();
                let mut escaped = false;
                for (i, ch) in rest.char_indices() {
                    if escaped {
                        escaped = false;
                    } else if ch == '\\' {
                        escaped = true;
                    } else if ch == '`' {
                        end = at + i + 1;
                        state = LineState::Code;
                        break;
                    }
                }
                push(&mut out, at..end, Token::String);
                at = end;
                continue;
            }
            LineState::Code => {}
        }
        let ch = rest.chars().next().unwrap_or(' ');
        if rest.starts_with("//") {
            push(&mut out, at..line.len(), Token::Comment);
            break;
        }
        if rest.starts_with("/*") {
            state = LineState::BlockComment;
            push(&mut out, at..at + 2, Token::Comment);
            at += 2;
            continue;
        }
        if ch == '`' {
            state = LineState::Template;
            push(&mut out, at..at + 1, Token::String);
            at += 1;
            continue;
        }
        if ch == '"' || ch == '\'' {
            // A string ends on its line, closed or not.
            let mut end = line.len();
            let mut escaped = false;
            for (i, c) in rest.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == ch {
                    end = at + i + 1;
                    break;
                }
            }
            push(&mut out, at..end, Token::String);
            at = end;
            after_namespace = false;
            continue;
        }
        if ch.is_ascii_digit() {
            let end = number_end(line, at);
            push(&mut out, at..end, Token::Number);
            at = end;
            continue;
        }
        if is_word(ch) {
            let end = at + rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
            let word = &line[at..end];
            // A name after a dot is a member, whatever it spells.
            let member = line[..at].trim_end().ends_with('.');
            let token = if after_namespace {
                Token::Type
            } else if member {
                Token::Plain
            } else if TS_TYPES.contains(&word) {
                Token::Type
            } else if TS_KEYWORDS.contains(&word) {
                Token::Keyword
            } else {
                Token::Plain
            };
            after_namespace = word == "ExcelScript" && !member;
            push(&mut out, at..end, token);
            at = end;
            continue;
        }
        if !ch.is_whitespace() && ch != '.' {
            after_namespace = false;
        }
        push(&mut out, at..at + ch.len_utf8(), Token::Plain);
        at += ch.len_utf8();
    }
    (out, state)
}

fn vba(line: &str) -> Vec<(Range<usize>, Token)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < line.len() {
        let rest = &line[at..];
        let ch = rest.chars().next().unwrap_or(' ');
        if ch == '\'' {
            push(&mut out, at..line.len(), Token::Comment);
            break;
        }
        if ch == '"' {
            // Quotes inside a string are doubled.
            let bytes = rest.as_bytes();
            let mut i = 1;
            let mut end = line.len();
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    if bytes.get(i + 1) == Some(&b'"') {
                        i += 2;
                        continue;
                    }
                    end = at + i + 1;
                    break;
                }
                i += 1;
            }
            push(&mut out, at..end, Token::String);
            at = end;
            continue;
        }
        if ch.is_ascii_digit() || rest.starts_with("&H") || rest.starts_with("&h") {
            let end = number_end(line, at);
            push(&mut out, at..end, Token::Number);
            at = end;
            continue;
        }
        if is_word(ch) {
            let end = at + rest.find(|c: char| !is_word(c)).unwrap_or(rest.len());
            let word = line[at..end].to_lowercase();
            // `Rem` starts a comment where a statement can start.
            if word == "rem" && line[..at].trim().is_empty() {
                push(&mut out, at..line.len(), Token::Comment);
                break;
            }
            let member = line[..at].ends_with('.');
            let token = if !member && VBA_KEYWORDS.contains(&word.as_str()) { Token::Keyword } else { Token::Plain };
            push(&mut out, at..end, token);
            at = end;
            continue;
        }
        push(&mut out, at..at + ch.len_utf8(), Token::Plain);
        at += ch.len_utf8();
    }
    out
}

/// The coloured stretches of one line (they cover it from end to end),
/// and the state the next line starts in.
pub(crate) fn highlight_line(language: Language, line: &str, state: LineState) -> (Vec<(Range<usize>, Token)>, LineState) {
    match language {
        Language::TypeScript => typescript(line, state),
        Language::Vba => (vba(line), LineState::Code),
    }
}

/// The state each line of a text starts in.
pub(crate) fn line_states(language: Language, text: &str) -> Vec<LineState> {
    let mut states = Vec::new();
    let mut state = LineState::Code;
    for line in text.split('\n') {
        states.push(state);
        // Only what opens or closes a comment or a template matters.
        if language == Language::TypeScript && (state != LineState::Code || line.contains("/*") || line.contains('`')) {
            state = highlight_line(language, line, state).1;
        }
    }
    states
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(language: Language, line: &str) -> Vec<(&str, Token)> {
        let (spans, _) = highlight_line(language, line, LineState::Code);
        // The spans cover the line, in order.
        let mut at = 0;
        for (range, _) in &spans {
            assert_eq!(range.start, at, "{line}");
            at = range.end;
        }
        assert_eq!(at, line.len(), "{line}");
        spans.into_iter().filter(|(range, _)| !line[range.clone()].trim().is_empty()).map(|(range, token)| (line[range].trim(), token)).collect()
    }

    #[test]
    fn typescript_lines() {
        use Token::*;
        assert_eq!(
            tokens(Language::TypeScript, "function main(workbook: ExcelScript.Workbook) {"),
            [("function", Keyword), ("main(workbook:", Plain), ("ExcelScript", Type), (".", Plain), ("Workbook", Type), (") {", Plain)]
        );
        assert_eq!(
            tokens(Language::TypeScript, "  let n: number = 0x1F + 12.5e-3; // sum"),
            [("let", Keyword), ("n:", Plain), ("number", Type), ("=", Plain), ("0x1F", Number), ("+", Plain), ("12.5e-3", Number), (";", Plain), ("// sum", Comment)]
        );
        assert_eq!(
            tokens(Language::TypeScript, "sheet.getRange(\"A1\").setValue('it\\'s'); x"),
            [("sheet.getRange(", Plain), ("\"A1\"", String), (").setValue(", Plain), ("'it\\'s'", String), ("); x", Plain)]
        );
        // A member is not a keyword, even when it spells one.
        assert_eq!(tokens(Language::TypeScript, "range.delete(x.in)"), [("range.delete(x.in)", Plain)]);
        // A string left open ends with its line.
        assert_eq!(tokens(Language::TypeScript, "let s = \"open"), [("let", Keyword), ("s =", Plain), ("\"open", String)]);
        assert_eq!(tokens(Language::TypeScript, "console.log(`n=${n}`)"), [("console", Type), (".log(", Plain), ("`n=${n}`", String), (")", Plain)]);
    }

    #[test]
    fn comments_and_templates_run_over_lines() {
        let text = "let a = 1; /* one\n two */ let b = `x\ny` + 2;\nlet c;";
        let states = line_states(Language::TypeScript, text);
        assert_eq!(states, [LineState::Code, LineState::BlockComment, LineState::Template, LineState::Code]);
        let lines: Vec<&str> = text.split('\n').collect();
        let (spans, state) = highlight_line(Language::TypeScript, lines[1], states[1]);
        assert_eq!(state, LineState::Template);
        assert_eq!(spans[0], (0..7, Token::Comment));
        assert_eq!(spans.last(), Some(&(16..18, Token::String)));
        let (spans, state) = highlight_line(Language::TypeScript, lines[2], states[2]);
        assert_eq!((state, spans[0].clone()), (LineState::Code, (0..2, Token::String)));
        assert_eq!(spans.last().map(|(_, token)| *token), Some(Token::Plain));
    }

    #[test]
    fn vba_lines() {
        use Token::*;
        assert_eq!(
            tokens(Language::Vba, "Public Sub Total() ' adds up"),
            [("Public", Keyword), ("Sub", Keyword), ("Total()", Plain), ("' adds up", Comment)]
        );
        assert_eq!(
            tokens(Language::Vba, "    Range(\"A1\").Value = \"say \"\"hi\"\"\" & 12 & &HFF"),
            [("Range(", Plain), ("\"A1\"", String), (").Value =", Plain), ("\"say \"\"hi\"\"\"", String), ("&", Plain), ("12", Number), ("&", Plain), ("&HFF", Number)]
        );
        assert_eq!(tokens(Language::Vba, "  Rem a remark"), [("Rem a remark", Comment)]);
        assert_eq!(tokens(Language::Vba, "x = \"'not a comment'\""), [("x =", Plain), ("\"'not a comment'\"", String)]);
        // A member is not a keyword: `.End(xlUp)`, `.Select`.
        assert_eq!(tokens(Language::Vba, "c.End(3).Select"), [("c.End(", Plain), ("3", Number), (").Select", Plain)]);
        assert_eq!(tokens(Language::Vba, "End If"), [("End", Keyword), ("If", Keyword)]);
    }
}
