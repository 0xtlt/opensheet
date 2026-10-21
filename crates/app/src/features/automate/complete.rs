//! What the Code Editor proposes while a script is typed: the members of
//! what stands before a dot, the enums of `ExcelScript.`, and the
//! parameters of the call the caret is in.
//!
//! Everything proposed comes from the table the script engine is built
//! from: what is listed is what runs. What stands before the dot is
//! asked of the engine first (`opensheet_script::complete`), which
//! follows types as its checker does. Where it does not say — a list, a
//! text, `console`, a name it will not vouch for — the text alone is
//! read here: a chain of calls from a name whose declaration says what it
//! is (`let sheet = workbook.getActiveWorksheet()`, `for (const table of
//! workbook.getTables())`, the parameter of a `forEach` callback). Where
//! neither tells, nothing is proposed: a list of guesses would be worse
//! than none.

use opensheet_script::{ApiClass, ApiEnum, ApiMember};

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// What is inserted.
    pub name: String,
    /// What follows the name in the list: `(row: number, column:
    /// number): Range` for a method, `= "Center"` for a member of an enum.
    pub detail: String,
    /// One line about it.
    pub about: &'static str,
}

/// The list to show at a place of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Completion {
    /// Where the word being typed starts: accepting an entry replaces
    /// the text from here to the caret.
    pub start: usize,
    /// The entries that match what was typed, best first.
    pub entries: Vec<Entry>,
}

/// The parameters of the call the caret is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hint {
    /// `getCell(row: number, column: number): Range`.
    pub signature: String,
    /// The part of `signature` that is the parameter being typed.
    pub active: Option<std::ops::Range<usize>>,
    /// Where the `(` of the call is.
    pub open: usize,
}

/// What an expression is, as far as completion cares.
#[derive(Clone)]
enum Type {
    Class(&'static ApiClass),
    Enum(&'static ApiEnum),
    List(Box<Type>),
    /// `ExcelScript` itself.
    Namespace,
    Console,
    Text,
    Other,
}

fn class(name: &str) -> Option<&'static ApiClass> {
    opensheet_script::api()
        .iter()
        .find(|class| class.name == name)
}

fn an_enum(name: &str) -> Option<&'static ApiEnum> {
    opensheet_script::enums().iter().find(|e| e.name == name)
}

/// A type as the signatures of the API write it.
fn type_of_text(text: &str) -> Type {
    let text = text.trim();
    // `Range | undefined` is a Range for what can be called on it.
    if let Some((first, rest)) = text.split_once('|') {
        if rest
            .split('|')
            .all(|part| matches!(part.trim(), "undefined" | "null"))
        {
            return type_of_text(first);
        }
    }
    if let Some(inner) = text.strip_suffix("[]") {
        let inner = inner.trim();
        let inner = inner
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .unwrap_or(inner);
        return Type::List(Box::new(type_of_text(inner)));
    }
    if let Some(inner) = text
        .strip_prefix("Array<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        return Type::List(Box::new(type_of_text(inner)));
    }
    let name = text.strip_prefix("ExcelScript.").unwrap_or(text);
    if let Some(class) = class(name) {
        return Type::Class(class);
    }
    if let Some(found) = an_enum(name) {
        return Type::Enum(found);
    }
    match name {
        "string" => Type::Text,
        _ => Type::Other,
    }
}

/// What a method gives back: the text after the `):` of its signature.
fn return_type(member: &ApiMember) -> Type {
    member
        .signature
        .rsplit_once("): ")
        .map_or(Type::Other, |(_, text)| type_of_text(text))
}

// ---------------------------------------------------------------- the text

/// Which bytes of a script are code: not inside a string, a template or
/// a comment (the quotes themselves are not code either). Also tells
/// whether the text ends inside one of those.
fn code_mask(source: &str) -> (Vec<bool>, bool) {
    let bytes = source.as_bytes();
    let mut mask = vec![true; bytes.len()];
    let mut open = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    mask[i] = false;
                    i += 1;
                }
                open = i == bytes.len();
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = source[i + 2..].find("*/").map(|at| i + 2 + at + 2);
                open = end.is_none();
                let end = end.unwrap_or(bytes.len());
                mask[i..end].fill(false);
                i = end;
            }
            quote @ (b'"' | b'\'' | b'`') => {
                mask[i] = false;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    // A plain string ends with its line.
                    if bytes[i] == b'\n' && quote != b'`' {
                        break;
                    }
                    mask[i] = false;
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        mask[i + 1] = false;
                        i += 1;
                    }
                    i += 1;
                }
                open = i >= bytes.len();
                if i < bytes.len() && bytes[i] == quote {
                    mask[i] = false;
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    (mask, open)
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

/// The text of a script with what is code in it.
struct Text<'a> {
    source: &'a str,
    bytes: &'a [u8],
    code: Vec<bool>,
}

/// A step of a chain: `.name`, `(...)`, `[...]`.
#[derive(Debug, PartialEq)]
enum Link<'a> {
    Name(&'a str),
    Call,
    Index,
}

impl<'a> Text<'a> {
    fn new(source: &'a str) -> Text<'a> {
        Text {
            source,
            bytes: source.as_bytes(),
            code: code_mask(source).0,
        }
    }

    /// The place before `end` where blanks stop.
    fn before_blanks(&self, mut end: usize) -> usize {
        while end > 0 && self.bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        end
    }

    fn after_blanks(&self, mut at: usize) -> usize {
        while at < self.bytes.len() && self.bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        at
    }

    /// Where the word that ends at `end` starts.
    fn word_start(&self, end: usize) -> usize {
        let mut start = end;
        while start > 0 && is_word(self.bytes[start - 1]) && self.code[start - 1] {
            start -= 1;
        }
        start
    }

    /// The opening bracket that matches the closing one at `close`.
    fn opening(&self, close: usize) -> Option<usize> {
        let mut depth = 0usize;
        let mut at = close + 1;
        while at > 0 {
            at -= 1;
            if !self.code[at] {
                continue;
            }
            match self.bytes[at] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(at);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The chain of names, calls and indexes that ends at `end`, first
    /// link first: `workbook.getTables()[0]` is workbook, getTables, a
    /// call, an index. `None` when what ends there is not such a chain.
    fn chain(&self, end: usize) -> Option<Vec<Link<'a>>> {
        let mut links = Vec::new();
        let mut end = self.before_blanks(end);
        loop {
            if end == 0 {
                return None;
            }
            let last = end - 1;
            match self.bytes[last] {
                b')' if self.code[last] => {
                    end = self.before_blanks(self.opening(last)?);
                    links.push(Link::Call);
                }
                b']' if self.code[last] => {
                    end = self.before_blanks(self.opening(last)?);
                    links.push(Link::Index);
                }
                // `value!` is the value.
                b'!' if self.code[last] => end = self.before_blanks(last),
                byte if is_word(byte) && self.code[last] => {
                    let start = self.word_start(end);
                    let word = &self.source[start..end];
                    if word.as_bytes()[0].is_ascii_digit() {
                        return None;
                    }
                    links.push(Link::Name(word));
                    // A dot before it: the chain goes on to the left.
                    let before = self.before_blanks(start);
                    if before > 0 && self.bytes[before - 1] == b'.' && self.code[before - 1] {
                        let mut dot = before - 1;
                        if dot > 0 && self.bytes[dot - 1] == b'?' {
                            dot -= 1;
                        }
                        end = self.before_blanks(dot);
                    } else {
                        links.reverse();
                        return Some(links);
                    }
                }
                _ => return None,
            }
        }
    }

    /// The places where `name` stands as a whole word of the code.
    fn occurrences(&self, name: &str) -> Vec<usize> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = self.source[from..].find(name).map(|at| at + from) {
            let end = at + name.len();
            let whole = (at == 0 || !is_word(self.bytes[at - 1]))
                && (end == self.bytes.len() || !is_word(self.bytes[end]));
            if whole && self.code[at] && !(at > 0 && self.bytes[at - 1] == b'.') {
                found.push(at);
            }
            from = end;
        }
        found
    }

    /// Where the expression that starts at `start` ends: at the `;`, or
    /// at the end of a line the next one does not carry on with a dot.
    fn expression_end(&self, start: usize) -> usize {
        let mut depth = 0i32;
        let mut at = start;
        while at < self.bytes.len() {
            if self.code[at] {
                match self.bytes[at] {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => {
                        depth -= 1;
                        if depth < 0 {
                            return at;
                        }
                    }
                    b';' | b',' if depth == 0 => return at,
                    b'\n' if depth == 0 => {
                        let next = self.after_blanks(at);
                        if self.bytes.get(next) != Some(&b'.')
                            && self.bytes.get(next) != Some(&b'?')
                        {
                            return at;
                        }
                    }
                    _ => {}
                }
            }
            at += 1;
        }
        at
    }

    /// The type a name was declared with, looking at the declaration
    /// nearest before `at`.
    fn declared(&self, name: &str, at: usize, depth: u8) -> Option<Type> {
        match name {
            "ExcelScript" => return Some(Type::Namespace),
            "console" => return Some(Type::Console),
            _ => {}
        }
        if depth > 8 {
            return None;
        }
        let places = self.occurrences(name);
        // The nearest declaration above; failing that, any.
        let before = places.iter().rev().filter(|place| **place < at);
        let after = places.iter().filter(|place| **place >= at);
        before
            .chain(after)
            .find_map(|place| self.declaration(name, *place, depth))
    }

    /// What the occurrence of a name at `place` says of its type, when
    /// it is a declaration.
    fn declaration(&self, name: &str, place: usize, depth: u8) -> Option<Type> {
        let after = self.after_blanks(place + name.len());
        let before = self.before_blanks(place);
        let ahead = &self.source[after..];
        let behind = &self.source[..before];
        // `name: ExcelScript.Range`: a parameter or a typed variable.
        if let Some(rest) = ahead.strip_prefix(':') {
            let end = rest
                .find(['=', ',', ')', ';', '{', '\n'])
                .unwrap_or(rest.len());
            let declared = type_of_text(&rest[..end]);
            if !matches!(declared, Type::Other) {
                return Some(declared);
            }
        }
        let keyword = |text: &str| {
            ["let", "const", "var"].iter().any(|word| {
                text.ends_with(word) && !text[..text.len() - word.len()].ends_with(is_word_char)
            })
        };
        // `let name = expression`.
        if keyword(behind) {
            if let Some(rest) = ahead
                .strip_prefix('=')
                .filter(|rest| !rest.starts_with('='))
            {
                let start = self.after_blanks(after + 1);
                let _ = rest;
                return self.type_at(self.expression_end(start), depth + 1);
            }
            // `for (const name of expression)`: one of the list.
            if let Some(rest) = ahead
                .strip_prefix("of")
                .filter(|rest| rest.starts_with(|c: char| c.is_whitespace()))
            {
                let start = self.after_blanks(after + 2);
                let _ = rest;
                return match self.type_at(self.expression_end(start), depth + 1)? {
                    Type::List(item) => Some(*item),
                    _ => None,
                };
            }
        }
        // `list.forEach((name) => ...)`, `list.map(name => ...)`: the
        // first parameter of the callback is one of the list.
        let arrow = ahead.starts_with("=>") || ahead.starts_with(',') || ahead.starts_with(')');
        let opened = behind
            .strip_suffix('(')
            .map(|rest| rest.strip_suffix('(').unwrap_or(rest));
        if let (true, Some(call)) = (arrow, opened) {
            for method in [
                "forEach",
                "map",
                "filter",
                "find",
                "some",
                "every",
                "findIndex",
            ] {
                if let Some(receiver) = call
                    .strip_suffix(method)
                    .and_then(|rest| rest.trim_end().strip_suffix('.'))
                {
                    return match self.type_at(receiver.len(), depth + 1)? {
                        Type::List(item) => Some(*item),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// The type of the chain that ends at `end`.
    fn type_at(&self, end: usize, depth: u8) -> Option<Type> {
        let links = self.chain(end)?;
        let mut links = links.into_iter().peekable();
        let Some(Link::Name(first)) = links.next() else {
            return None;
        };
        let mut current = self.declared(first, end, depth)?;
        while let Some(link) = links.next() {
            current = match (current, link) {
                (Type::Class(class), Link::Name(name)) => {
                    let member = class.members.iter().find(|member| member.name == name)?;
                    // Every member is a method: it must be called.
                    links.next_if_eq(&Link::Call)?;
                    return_type(member)
                }
                (Type::Namespace, Link::Name(name)) => match an_enum(name) {
                    Some(found) => Type::Enum(found),
                    None => Type::Other,
                },
                (Type::Enum(_), Link::Name(_)) => Type::Text,
                (Type::List(item), Link::Index) => *item,
                (Type::List(item), Link::Name(name)) => {
                    let called = links.next_if_eq(&Link::Call).is_some();
                    match (name, called) {
                        ("find" | "pop" | "shift" | "at", true) => *item,
                        ("filter" | "slice" | "concat" | "reverse" | "sort", true) => {
                            Type::List(item)
                        }
                        ("join", true) => Type::Text,
                        _ => Type::Other,
                    }
                }
                (Type::Text, Link::Name(name)) => {
                    let called = links.next_if_eq(&Link::Call).is_some();
                    match (name, called) {
                        (
                            "toUpperCase" | "toLowerCase" | "trim" | "substring" | "slice"
                            | "replace" | "concat",
                            true,
                        ) => Type::Text,
                        ("split", true) => Type::List(Box::new(Type::Text)),
                        _ => Type::Other,
                    }
                }
                _ => return None,
            };
        }
        Some(current)
    }
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

// ----------------------------------------------------------------- entries

const LIST_MEMBERS: &[(&str, &str, &str)] = &[
    ("length", ": number", "How many items the list has."),
    (
        "forEach",
        "(callback: (item, index) => void): void",
        "Calls a function with each item.",
    ),
    (
        "map",
        "(callback: (item, index) => U): U[]",
        "A list of what a function gives for each item.",
    ),
    (
        "filter",
        "(callback: (item, index) => boolean): T[]",
        "The items a function keeps.",
    ),
    (
        "find",
        "(callback: (item, index) => boolean): T | undefined",
        "The first item a function accepts.",
    ),
    (
        "some",
        "(callback: (item, index) => boolean): boolean",
        "Whether a function accepts an item.",
    ),
    (
        "every",
        "(callback: (item, index) => boolean): boolean",
        "Whether a function accepts every item.",
    ),
    (
        "indexOf",
        "(item: T): number",
        "The place of an item; -1 when it is not there.",
    ),
    (
        "includes",
        "(item: T): boolean",
        "Whether an item is in the list.",
    ),
    (
        "slice",
        "(start?: number, end?: number): T[]",
        "A part of the list.",
    ),
    (
        "concat",
        "(...items: T[]): T[]",
        "This list followed by others.",
    ),
    (
        "join",
        "(separator?: string): string",
        "The items as one text.",
    ),
    ("push", "(...items: T[]): number", "Adds items at the end."),
    ("pop", "(): T | undefined", "Takes the last item away."),
    ("reverse", "(): T[]", "Turns the list round."),
    (
        "sort",
        "(compare?: (a: T, b: T) => number): T[]",
        "Sorts the list.",
    ),
];

const TEXT_MEMBERS: &[(&str, &str, &str)] = &[
    ("length", ": number", "How many characters the text has."),
    ("toUpperCase", "(): string", "The text in capitals."),
    ("toLowerCase", "(): string", "The text in small letters."),
    (
        "trim",
        "(): string",
        "The text without the blanks around it.",
    ),
    (
        "includes",
        "(search: string): boolean",
        "Whether the text holds another.",
    ),
    (
        "startsWith",
        "(search: string): boolean",
        "Whether the text starts with another.",
    ),
    (
        "endsWith",
        "(search: string): boolean",
        "Whether the text ends with another.",
    ),
    (
        "indexOf",
        "(search: string): number",
        "Where another text is in this one; -1 when it is not.",
    ),
    (
        "split",
        "(separator: string): string[]",
        "The parts of the text between a separator.",
    ),
    (
        "replace",
        "(search: string, replacement: string): string",
        "The text with one part replaced.",
    ),
    (
        "substring",
        "(start: number, end?: number): string",
        "A part of the text.",
    ),
    (
        "slice",
        "(start?: number, end?: number): string",
        "A part of the text.",
    ),
];

const CONSOLE_MEMBERS: &[(&str, &str, &str)] = &[
    (
        "log",
        "(...data: any[]): void",
        "Writes to the Output list.",
    ),
    (
        "info",
        "(...data: any[]): void",
        "Writes to the Output list.",
    ),
    (
        "debug",
        "(...data: any[]): void",
        "Writes to the Output list.",
    ),
    (
        "warn",
        "(...data: any[]): void",
        "Writes a warning to the Output list.",
    ),
    (
        "error",
        "(...data: any[]): void",
        "Writes an error to the Output list.",
    ),
];

fn fixed(members: &[(&'static str, &'static str, &'static str)]) -> Vec<Entry> {
    members
        .iter()
        .map(|(name, detail, about)| Entry {
            name: name.to_string(),
            detail: detail.to_string(),
            about,
        })
        .collect()
}

/// Everything that can follow a dot after a value of a type.
fn members_of(of: &Type) -> Vec<Entry> {
    let mut entries = match of {
        Type::Class(class) => class
            .members
            .iter()
            .map(|member| Entry {
                name: member.name.to_string(),
                detail: member.signature[member.name.len()..].to_string(),
                about: member.description,
            })
            .collect(),
        Type::Enum(found) => found
            .members
            .iter()
            .map(|(name, value)| Entry {
                name: name.to_string(),
                detail: format!("= \"{value}\""),
                about: "",
            })
            .collect(),
        Type::Namespace => {
            let enums = opensheet_script::enums().iter().map(|e| Entry {
                name: e.name.to_string(),
                detail: "enum".to_string(),
                about: "",
            });
            let classes = opensheet_script::api().iter().map(|class| Entry {
                name: class.name.to_string(),
                detail: "interface".to_string(),
                about: class.description,
            });
            enums.chain(classes).collect()
        }
        Type::List(_) => fixed(LIST_MEMBERS),
        Type::Text => fixed(TEXT_MEMBERS),
        Type::Console => fixed(CONSOLE_MEMBERS),
        Type::Other => Vec::new(),
    };
    // Members of an enum keep the order Excel lists them in.
    if !matches!(of, Type::Enum(_)) {
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
    entries
}

/// The entries that match what was typed: those that start with it,
/// then those that hold it, whatever the case.
fn matching(entries: Vec<Entry>, typed: &str) -> Vec<Entry> {
    if typed.is_empty() {
        return entries;
    }
    let typed = typed.to_lowercase();
    let (starts, rest): (Vec<Entry>, Vec<Entry>) = entries
        .into_iter()
        .partition(|entry| entry.name.to_lowercase().starts_with(&typed));
    let holds = rest
        .into_iter()
        .filter(|entry| entry.name.to_lowercase().contains(&typed));
    starts.into_iter().chain(holds).collect()
}

/// The list to show with the caret at `caret`: `None` when the caret is
/// not after a dot (and the start of a member), or when the text does
/// not tell what stands before the dot.
pub(crate) fn complete(source: &str, caret: usize) -> Option<Completion> {
    if caret > source.len() || !source.is_char_boundary(caret) {
        return None;
    }
    // Not while a string or a comment is typed.
    if code_mask(&source[..caret]).1 {
        return None;
    }
    // The engine first: it knows the types of the model.
    let known = opensheet_script::complete(source, caret);
    if known.of.is_some() && !known.items.is_empty() && known.start <= caret {
        let ordered = known
            .items
            .iter()
            .all(|item| item.kind == opensheet_script::CompletionKind::EnumMember);
        let mut entries: Vec<Entry> = known
            .items
            .iter()
            .map(|item| Entry {
                name: item.name.to_string(),
                detail: match item.kind {
                    opensheet_script::CompletionKind::Method => item
                        .signature
                        .strip_prefix(item.name)
                        .unwrap_or(&item.signature)
                        .to_string(),
                    opensheet_script::CompletionKind::EnumMember => item
                        .signature
                        .strip_prefix(item.name)
                        .unwrap_or(&item.signature)
                        .trim_start()
                        .to_string(),
                    opensheet_script::CompletionKind::Enum => "enum".to_string(),
                },
                about: item.description,
            })
            .collect();
        // Members of an enum keep the order Excel lists them in.
        if !ordered {
            entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
        let entries = matching(entries, &source[known.start..caret]);
        return (!entries.is_empty()).then_some(Completion {
            start: known.start,
            entries,
        });
    }
    let text = Text::new(source);
    let start = text.word_start(caret);
    let before = text.before_blanks(start);
    if before == 0 || text.bytes[before - 1] != b'.' || !text.code[before - 1] {
        return None;
    }
    let mut dot = before - 1;
    if dot > 0 && text.bytes[dot - 1] == b'?' {
        dot -= 1;
    }
    let of = text.type_at(dot, 0)?;
    let entries = matching(members_of(&of), &source[start..caret]);
    (!entries.is_empty()).then_some(Completion { start, entries })
}

/// The call the caret is in, with the parameter being typed: `None`
/// outside the parentheses of a call of a member of the API.
pub(crate) fn hint(source: &str, caret: usize) -> Option<Hint> {
    if caret > source.len() || !source.is_char_boundary(caret) {
        return None;
    }
    if code_mask(&source[..caret]).1 {
        return None;
    }
    let text = Text::new(source);
    // The parenthesis the caret is in, and the commas since.
    let mut depth = 0usize;
    let mut commas = 0usize;
    let mut at = caret;
    let open = loop {
        if at == 0 {
            return None;
        }
        at -= 1;
        if !text.code[at] {
            continue;
        }
        match text.bytes[at] {
            b')' | b']' | b'}' => depth += 1,
            b'(' if depth == 0 => break at,
            // An object or a list being typed as an argument, or the
            // body of a function: not the call itself.
            b'[' | b'{' if depth == 0 => {
                commas = 0;
                if text.bytes[at] == b'{'
                    && !text
                        .before_blanks(at)
                        .checked_sub(1)
                        .is_some_and(|last| matches!(text.bytes[last], b'(' | b',' | b':' | b'['))
                {
                    return None;
                }
            }
            b'(' | b'[' | b'{' => depth -= 1,
            b',' if depth == 0 => commas += 1,
            b';' if depth == 0 => return None,
            _ => {}
        }
    };
    // What is called: the member before the parenthesis, of the type
    // before its dot.
    let end = text.before_blanks(open);
    let start = text.word_start(end);
    let name = &source[start..end];
    let before = text.before_blanks(start);
    if name.is_empty() || before == 0 || text.bytes[before - 1] != b'.' {
        return None;
    }
    // The class before the dot: the engine's word, else what the text
    // tells.
    let known = opensheet_script::complete(source, start);
    let signature = match known
        .items
        .iter()
        .find(|item| item.name == name && item.kind == opensheet_script::CompletionKind::Method)
    {
        Some(item) => item.signature.clone(),
        None => {
            let Type::Class(class) = text.type_at(before - 1, 0)? else {
                return None;
            };
            class
                .members
                .iter()
                .find(|member| member.name == name)?
                .signature
                .to_string()
        }
    };
    // The parameters are between the first parenthesis and the `):`.
    let list_start = signature.find('(')? + 1;
    let list_end = signature.rfind("): ")?;
    let mut active = None;
    let mut from = list_start;
    let mut depth = 0i32;
    let mut index = 0;
    for (offset, ch) in signature[list_start..list_end]
        .char_indices()
        .chain([(list_end - list_start, ',')])
    {
        match ch {
            '(' | '[' | '<' | '{' => depth += 1,
            ')' | ']' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let to = list_start + offset;
                if index == commas && to > from {
                    let lead = signature[from..to].len() - signature[from..to].trim_start().len();
                    active = Some(from + lead..to);
                }
                index += 1;
                from = to + 1;
            }
            _ => {}
        }
    }
    Some(Hint {
        signature,
        active,
        open,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names proposed with the caret at `|`.
    fn names(marked: &str) -> Vec<String> {
        let caret = marked.find('|').expect("a caret");
        let source = marked.replace('|', "");
        complete(&source, caret)
            .map(|completion| {
                completion
                    .entries
                    .into_iter()
                    .map(|entry| entry.name)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn has(marked: &str, name: &str) -> bool {
        names(marked).iter().any(|entry| entry == name)
    }

    const MAIN: &str = "function main(workbook: ExcelScript.Workbook) {\n";

    #[test]
    fn members_follow_the_type_before_the_dot() {
        // The parameter of main.
        let all = names(&format!("{MAIN}  workbook.|"));
        assert!(
            all.contains(&"getActiveWorksheet".to_string())
                && all.contains(&"addTable".to_string())
        );
        assert!(!all.contains(&"getRange".to_string()));
        // A chain of calls.
        assert!(has(
            &format!("{MAIN}  workbook.getActiveWorksheet().|"),
            "getRange"
        ));
        assert!(has(
            &format!("{MAIN}  workbook.getActiveWorksheet().getRange(\"A1:B2\").getFormat().|"),
            "getFill"
        ));
        assert!(has(
            &format!(
                "{MAIN}  workbook.getActiveWorksheet().getRange(\"A1\").getFormat().getFont().|"
            ),
            "setBold"
        ));
        // A variable, declared with a value or with a type.
        let script = format!(
            "{MAIN}  let sheet = workbook.getActiveWorksheet();\n  const range = sheet\n    .getRange(\"A1\");\n  range.|"
        );
        assert!(has(&script, "setValues"));
        assert!(has(
            &format!("{MAIN}  let table: ExcelScript.Table;\n  table.|"),
            "getColumnByName"
        ));
        // What may be undefined is still what it is.
        assert!(has(
            &format!("{MAIN}  workbook.getTable(\"T\").|"),
            "setShowTotals"
        ));
        assert!(has(
            &format!("{MAIN}  workbook.getTable(\"T\")!.|"),
            "setShowTotals"
        ));
        assert!(has(
            &format!("{MAIN}  workbook.getWorksheet(\"S\")?.|"),
            "getRange"
        ));
        // Every list is sorted by name.
        let mut sorted = all.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(all, sorted);
    }

    #[test]
    fn lists_loops_and_callbacks() {
        assert!(has(&format!("{MAIN}  workbook.getTables().|"), "forEach"));
        assert!(has(
            &format!("{MAIN}  workbook.getTables()[0].|"),
            "getName"
        ));
        assert!(has(
            &format!("{MAIN}  for (const table of workbook.getTables()) {{\n    table.|"),
            "getHeaderRowRange"
        ));
        assert!(has(
            &format!("{MAIN}  workbook.getWorksheets().forEach((sheet) => {{\n    sheet.|"),
            "getUsedRange"
        ));
        assert!(has(
            &format!("{MAIN}  workbook.getWorksheets().forEach(sheet => sheet.|"),
            "getName"
        ));
        assert!(has(
            &format!(
                "{MAIN}  let first = workbook.getWorksheets().find(s => s.getName() == \"a\");\n  first.|"
            ),
            "activate"
        ));
        // A text has the members of texts.
        assert!(has(
            &format!("{MAIN}  workbook.getActiveWorksheet().getName().|"),
            "toUpperCase"
        ));
        assert!(has(&format!("{MAIN}  console.|"), "log"));
    }

    #[test]
    fn enums_of_the_namespace() {
        let all = names(&format!("{MAIN}  let a = ExcelScript.|"));
        assert!(
            all.contains(&"HorizontalAlignment".to_string())
                && all.contains(&"ChartType".to_string())
        );
        assert_eq!(
            names(&format!(
                "{MAIN}  let a = ExcelScript.HorizontalAlignment.|"
            ))[..3],
            ["general", "left", "center"]
        );
        assert_eq!(
            names(&format!("{MAIN}  let a = ExcelScript.Horiz|"))[0],
            "HorizontalAlignment"
        );
        assert_eq!(
            names(&format!(
                "{MAIN}  x.setHorizontalAlignment(ExcelScript.HorizontalAlignment.cen|"
            )),
            ["center", "centerAcrossSelection"]
        );
    }

    #[test]
    fn what_is_typed_filters_the_list() {
        let found = names(&format!("{MAIN}  workbook.getActiveWorksheet().getRangeB|"));
        assert_eq!(found, ["getRangeByIndexes"]);
        // Small letters find capitals; what starts with the word comes
        // before what holds it.
        let found = names(&format!("{MAIN}  workbook.getActiveWorksheet().range|"));
        assert_eq!(found[..2], ["getRange", "getRangeByIndexes"]);
        assert!(
            found.contains(&"getUsedRange".to_string())
                && found
                    .iter()
                    .all(|name| name.to_lowercase().contains("range"))
        );
        assert!(names(&format!("{MAIN}  workbook.getActiveWorksheet().zzz|")).is_empty());
        // The place the word starts at.
        let source = format!("{MAIN}  workbook.getAc");
        assert_eq!(
            complete(&source, source.len()).map(|c| c.start),
            Some(source.len() - 5)
        );
    }

    #[test]
    fn nothing_is_proposed_where_the_text_does_not_tell() {
        assert!(names(&format!("{MAIN}  unknown.|")).is_empty());
        assert!(names(&format!("{MAIN}  let n = 12.|")).is_empty());
        assert!(names(&format!("{MAIN}  workbook.getName|")).len() == 3);
        // A member that is not called is not what it returns.
        assert!(names(&format!("{MAIN}  workbook.getActiveWorksheet.|")).is_empty());
        // Not in a string, not in a comment.
        assert!(names(&format!("{MAIN}  let s = \"workbook.|")).is_empty());
        assert!(names(&format!("{MAIN}  // workbook.|")).is_empty());
        assert!(names(&format!("{MAIN}  /* workbook.| */")).is_empty());
        // A string before does not hide the code after it.
        assert!(has(
            &format!("{MAIN}  let s = \"a.b\"; workbook.|"),
            "getTables"
        ));
        assert!(has(
            &format!("{MAIN}  // workbook.\n  workbook.|"),
            "getTables"
        ));
        // No caret after a dot.
        assert!(names(&format!("{MAIN}  workbook|")).is_empty());
    }

    #[test]
    fn the_details_are_the_signatures() {
        let source = format!("{MAIN}  workbook.getActiveWorksheet().getCell");
        let entry = complete(&source, source.len())
            .expect("a list")
            .entries
            .remove(0);
        assert_eq!(
            (entry.name.as_str(), entry.detail.as_str()),
            ("getCell", "(row: number, column: number): Range")
        );
        assert!(!entry.about.is_empty());
    }

    /// The hint with the caret at `|`: its signature with the active
    /// parameter between brackets.
    fn hinted(marked: &str) -> Option<String> {
        let caret = marked.find('|').expect("a caret");
        let source = marked.replace('|', "");
        hint(&source, caret).map(|hint| match hint.active {
            Some(active) => format!(
                "{}[{}]{}",
                &hint.signature[..active.start],
                &hint.signature[active.clone()],
                &hint.signature[active.end..]
            ),
            None => hint.signature,
        })
    }

    #[test]
    fn the_parameters_of_the_call_the_caret_is_in() {
        let sheet = format!("{MAIN}  let sheet = workbook.getActiveWorksheet();\n");
        assert_eq!(
            hinted(&format!("{sheet}  sheet.getCell(|")).as_deref(),
            Some("getCell([row: number], column: number): Range")
        );
        assert_eq!(
            hinted(&format!("{sheet}  sheet.getCell(1, |")).as_deref(),
            Some("getCell(row: number, [column: number]): Range")
        );
        assert_eq!(
            hinted(&format!("{sheet}  sheet.getCell(1, 2|)")).as_deref(),
            Some("getCell(row: number, [column: number]): Range")
        );
        // A comma of an inner call or of a string is not one of this call.
        assert_eq!(
            hinted(&format!(
                "{sheet}  sheet.getRangeByIndexes(Math.max(1, 2), \"a,b\", |"
            ))
            .as_deref(),
            Some(
                "getRangeByIndexes(startRow: number, startColumn: number, [rowCount: number], columnCount: number): Range"
            )
        );
        // Past the last parameter, and without parameters.
        assert_eq!(
            hinted(&format!("{sheet}  sheet.getCell(1, 2, |")).as_deref(),
            Some("getCell(row: number, column: number): Range")
        );
        assert_eq!(
            hinted(&format!("{sheet}  sheet.activate(|")).as_deref(),
            Some("activate(): void")
        );
        // The innermost call.
        assert_eq!(
            hinted(&format!("{sheet}  sheet.getRange(sheet.getCell(0, |")).as_deref(),
            Some("getCell(row: number, [column: number]): Range")
        );
        // An object typed as an argument is still in the call.
        assert_eq!(
            hinted(&format!(
                "{sheet}  sheet.getAutoFilter().apply(\"A1:C5\", 0, {{ filterOn: |"
            ))
            .as_deref(),
            Some(
                "apply(range: Range | string, columnIndex?: number, [criteria?: ExcelScript.FilterCriteria]): void"
            )
        );
        // Outside a call, after it, or in a call of something unknown.
        assert_eq!(hinted(&format!("{sheet}  sheet.getCell(1, 2)|")), None);
        assert_eq!(hinted(&format!("{sheet}  sheet|")), None);
        assert_eq!(hinted(&format!("{sheet}  thing.go(|")), None);
        assert_eq!(hinted(&format!("{sheet}  if (|")), None);
    }

    #[test]
    fn every_member_of_the_api_has_a_signature_that_reads() {
        for class in opensheet_script::api() {
            for member in class.members {
                assert!(
                    member.signature.starts_with(member.name) && member.signature.contains("): "),
                    "{}.{}",
                    class.name,
                    member.signature
                );
            }
        }
    }
}
