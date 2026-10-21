//! Flash Fill: guessing, from a few results typed by hand next to a list,
//! the transformation that gives them, and applying it to the other rows.
//!
//! A transformation is a [`Program`]: a list of pieces put end to end,
//! each one a constant or a part of a source column (a word, a run of
//! digits, what lies between two landmarks, a few characters of a token),
//! possibly with its case changed. [`learn`] looks for the cheapest
//! program that reproduces every example: parts of the sources cost less
//! than constants and simple landmarks less than contrived ones, so the
//! program found is the one a person would describe ("the last word, a
//! comma, the first word").
//!
//! The search walks the examples together. The candidate pieces are those
//! that can produce a part of the first example; a state is a position in
//! each example's result, and a piece leads from one state to the next
//! when it produces what comes next in every example. The cheapest path
//! from the starts to the ends (Dijkstra) is the program.

use crate::refs::Rect;
use crate::value::Value;
use crate::workbook::{Content, Workbook};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// The kinds of pieces a source is cut into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Class {
    /// A run of letters.
    Word,
    /// A run of digits.
    Digits,
    /// A run of letters and digits.
    Alnum,
    /// A run of anything but blanks.
    Field,
    /// One occurrence of a punctuation character.
    Punct(char),
}

/// A token of a class: the `index`-th from the start, or from the end
/// when negative (-1 is the last).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Token {
    class: Class,
    index: i32,
}

/// A place in a source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pos {
    Begin,
    End,
    /// Where a token starts.
    Before(Token),
    /// Where a token ends.
    After(Token),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Case {
    Keep,
    Upper,
    Lower,
    /// Each word capitalised.
    Proper,
}

const CASES: [Case; 4] = [Case::Keep, Case::Upper, Case::Lower, Case::Proper];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Piece {
    Const(String),
    /// What lies between two places of a source column.
    Span {
        col: usize,
        start: Pos,
        end: Pos,
        case: Case,
    },
    /// `len` characters of a token, from `offset` characters after its
    /// start (or before its end when negative).
    Slice {
        col: usize,
        token: Token,
        offset: i32,
        len: usize,
        case: Case,
    },
}

/// A transformation learnt from examples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pieces: Vec<Piece>,
}

/// A source cut into characters, with the tokens of every class.
struct Source {
    chars: Vec<char>,
    /// (class, start, end) in characters; the tokens of a class in order.
    tokens: Vec<(Class, usize, usize)>,
}

impl Source {
    fn new(text: &str) -> Source {
        let chars: Vec<char> = text.chars().collect();
        let mut tokens = Vec::new();
        let runs = |class: Class,
                    belongs: &dyn Fn(char) -> bool,
                    tokens: &mut Vec<(Class, usize, usize)>| {
            let mut start = None;
            for (i, ch) in chars.iter().enumerate() {
                match (belongs(*ch), start) {
                    (true, None) => start = Some(i),
                    (false, Some(s)) => {
                        tokens.push((class, s, i));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(s) = start {
                tokens.push((class, s, chars.len()));
            }
        };
        runs(Class::Word, &|ch| ch.is_alphabetic(), &mut tokens);
        runs(Class::Digits, &|ch| ch.is_ascii_digit(), &mut tokens);
        runs(Class::Alnum, &|ch| ch.is_alphanumeric(), &mut tokens);
        runs(Class::Field, &|ch| !ch.is_whitespace(), &mut tokens);
        for (i, ch) in chars.iter().enumerate() {
            if !ch.is_alphanumeric() && !ch.is_whitespace() {
                tokens.push((Class::Punct(*ch), i, i + 1));
            }
        }
        Source { chars, tokens }
    }

    fn of_class(&self, class: Class) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.tokens
            .iter()
            .filter(move |(c, _, _)| *c == class)
            .map(|(_, start, end)| (*start, *end))
    }

    /// Where a token is (start, end).
    fn token(&self, token: Token) -> Option<(usize, usize)> {
        if token.index >= 0 {
            self.of_class(token.class).nth(token.index as usize)
        } else {
            let all: Vec<(usize, usize)> = self.of_class(token.class).collect();
            all.len()
                .checked_sub(token.index.unsigned_abs() as usize)
                .map(|i| all[i])
        }
    }

    fn pos(&self, pos: Pos) -> Option<usize> {
        match pos {
            Pos::Begin => Some(0),
            Pos::End => Some(self.chars.len()),
            Pos::Before(token) => self.token(token).map(|(start, _)| start),
            Pos::After(token) => self.token(token).map(|(_, end)| end),
        }
    }

    fn text(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
}

fn apply_case(text: &str, case: Case) -> String {
    match case {
        Case::Keep => text.to_string(),
        Case::Upper => text.to_uppercase(),
        Case::Lower => text.to_lowercase(),
        Case::Proper => {
            let mut out = String::with_capacity(text.len());
            let mut inside = false;
            for ch in text.chars() {
                if !ch.is_alphabetic() {
                    out.push(ch);
                    inside = false;
                } else if inside {
                    out.extend(ch.to_lowercase());
                } else {
                    out.extend(ch.to_uppercase());
                    inside = true;
                }
            }
            out
        }
    }
}

impl Piece {
    /// What the piece gives for a row; `None` when the row lacks what it
    /// refers to (no third word).
    fn eval(&self, sources: &[Source]) -> Option<String> {
        match self {
            Piece::Const(text) => Some(text.clone()),
            Piece::Span {
                col,
                start,
                end,
                case,
            } => {
                let source = sources.get(*col)?;
                let (a, b) = (source.pos(*start)?, source.pos(*end)?);
                (a < b).then(|| apply_case(&source.text(a, b), *case))
            }
            Piece::Slice {
                col,
                token,
                offset,
                len,
                case,
            } => {
                let source = sources.get(*col)?;
                let (start, end) = source.token(*token)?;
                let a = if *offset >= 0 {
                    start + *offset as usize
                } else {
                    end.checked_sub(offset.unsigned_abs() as usize)?
                };
                (a >= start && a + len <= end).then(|| apply_case(&source.text(a, a + len), *case))
            }
        }
    }
}

impl Program {
    /// The result for a row, given the text of its source columns (in the
    /// order of the examples). `None` when the row does not have the
    /// pieces the program uses.
    pub fn apply<S: AsRef<str>>(&self, sources: &[S]) -> Option<String> {
        let sources: Vec<Source> = sources
            .iter()
            .map(|text| Source::new(text.as_ref()))
            .collect();
        self.run(&sources)
    }

    fn run(&self, sources: &[Source]) -> Option<String> {
        let mut out = String::new();
        for piece in &self.pieces {
            out.push_str(&piece.eval(sources)?);
        }
        Some(out)
    }
}

// Costs are in hundredths, so that they add up exactly.

fn class_cost(class: Class) -> u32 {
    match class {
        Class::Word | Class::Digits => 0,
        Class::Alnum => 40,
        Class::Field => 50,
        Class::Punct(_) => 10,
    }
}

/// Naming a token: the first ones and the last one are the natural
/// landmarks.
fn token_cost(token: Token) -> u32 {
    let place = match token.index {
        i if i >= 0 => 10 * i as u32,
        -1 => 5,
        i => 15 + 10 * i.unsigned_abs(),
    };
    200 + place + class_cost(token.class)
}

fn pos_cost(pos: Pos) -> u32 {
    match pos {
        Pos::Begin | Pos::End => 30,
        Pos::Before(token) | Pos::After(token) => token_cost(token) / 2,
    }
}

fn case_cost(case: Case) -> u32 {
    if case == Case::Keep { 0 } else { 50 }
}

fn const_cost(text: &str) -> u32 {
    let len = text.chars().count() as u32;
    // Separators are what constants are for; letters and digits that do
    // not come from the sources must be worth it.
    if text.chars().any(char::is_alphanumeric) {
        400 + 300 * len
    } else {
        100 + 10 * len
    }
}

fn piece_cost(piece: &Piece) -> u32 {
    match piece {
        Piece::Const(text) => const_cost(text),
        Piece::Span {
            start, end, case, ..
        } => {
            let base = match (start, end) {
                // A whole token, the whole column.
                (Pos::Before(a), Pos::After(b)) if a == b => token_cost(*a),
                (Pos::Begin, Pos::End) => 190,
                _ => 150 + pos_cost(*start) + pos_cost(*end),
            };
            base + case_cost(*case)
        }
        Piece::Slice {
            token,
            offset,
            case,
            ..
        } => {
            let from = match offset {
                0 => 0,
                o if *o > 0 => 50,
                _ => 60,
            };
            token_cost(*token) + 100 + from + case_cost(*case)
        }
    }
}

/// The two ways of naming the `i`-th of `count` tokens.
fn token_names(class: Class, i: usize, count: usize) -> [Token; 2] {
    [
        Token {
            class,
            index: i as i32,
        },
        Token {
            class,
            index: i as i32 - count as i32,
        },
    ]
}

/// Every piece that gives a part of `output` out of the sources of one
/// row, with where that part starts and its length in characters.
fn candidates(sources: &[Source], output: &[char]) -> Vec<(usize, usize, Piece)> {
    let text: String = output.iter().collect();
    // Byte offset of each character of the output, to turn what `find`
    // returns into character positions.
    let offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let mut found = Vec::new();
    let mut emit = |produced: &str, make: &dyn Fn() -> Piece| {
        if produced.is_empty() {
            return;
        }
        let len = produced.chars().count();
        let mut from = 0;
        while let Some(at) = text[from..].find(produced) {
            let byte = from + at;
            if let Ok(start) = offsets.binary_search(&byte) {
                found.push((start, len, make()));
            }
            from = byte + text[byte..].chars().next().map_or(1, char::len_utf8);
        }
    };
    for (col, source) in sources.iter().enumerate() {
        // The places a part can start at and end at, with their names.
        let n = source.chars.len();
        let mut starts: Vec<(usize, Pos)> = vec![(0, Pos::Begin)];
        let mut ends: Vec<(usize, Pos)> = vec![(n, Pos::End)];
        let classes: Vec<Class> = {
            let mut seen = Vec::new();
            for (class, _, _) in &source.tokens {
                if !seen.contains(class) {
                    seen.push(*class);
                }
            }
            seen
        };
        for class in classes {
            let tokens: Vec<(usize, usize)> = source.of_class(class).collect();
            for (i, (start, end)) in tokens.iter().enumerate() {
                for name in token_names(class, i, tokens.len()) {
                    // A part starts where a token starts or after a
                    // punctuation mark, and ends where a token ends or
                    // before a mark.
                    if matches!(class, Class::Punct(_)) {
                        starts.push((*end, Pos::After(name)));
                        ends.push((*start, Pos::Before(name)));
                    } else {
                        starts.push((*start, Pos::Before(name)));
                        ends.push((*end, Pos::After(name)));
                    }
                }
                // A few characters of a word or of a number: its
                // beginning, its end, or digits at a fixed place.
                if matches!(class, Class::Word | Class::Digits) {
                    let len = end - start;
                    for a in 0..len {
                        for l in 1..=(len - a) {
                            let whole = a == 0 && l == len;
                            if whole || (class == Class::Word && a != 0 && a + l != len) {
                                continue;
                            }
                            let slice = source.text(start + a, start + a + l);
                            for name in token_names(class, i, tokens.len()) {
                                for offset in [a as i32, a as i32 - len as i32] {
                                    for case in CASES {
                                        emit(&apply_case(&slice, case), &|| Piece::Slice {
                                            col,
                                            token: name,
                                            offset,
                                            len: l,
                                            case,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for (a, start) in &starts {
            for (b, end) in &ends {
                if a >= b {
                    continue;
                }
                let part = source.text(*a, *b);
                for case in CASES {
                    emit(&apply_case(&part, case), &|| Piece::Span {
                        col,
                        start: *start,
                        end: *end,
                        case,
                    });
                }
            }
        }
    }
    found
}

/// Longest text looked at: beyond it a cell is not a list item.
const MAX_LEN: usize = 120;
/// Examples the search walks together; the others only check its result.
const MAX_JOINT: usize = 6;
/// States the search may visit before giving up.
const MAX_STATES: usize = 200_000;

/// Find the transformation that turns the sources of each example (the
/// text of the source columns of a row) into its result. `others` are the
/// sources of the rows to fill: a piece that does not exist in some of
/// them is less likely to be what was meant.
///
/// `None` when no program explains the examples, or only a constant does.
pub fn learn<S: AsRef<str>>(examples: &[(Vec<S>, S)], others: &[Vec<S>]) -> Option<Program> {
    let parsed: Vec<(Vec<Source>, Vec<char>)> = examples
        .iter()
        .map(|(sources, output)| {
            (
                sources
                    .iter()
                    .map(|text| Source::new(text.as_ref()))
                    .collect(),
                output.as_ref().chars().collect(),
            )
        })
        .collect();
    let (first_sources, first_output) = parsed.first()?;
    if parsed.iter().any(|(sources, output)| {
        output.is_empty()
            || output.len() > MAX_LEN
            || sources.iter().any(|s| s.chars.len() > MAX_LEN)
    }) {
        return None;
    }
    let joint = &parsed[..parsed.len().min(MAX_JOINT)];
    let other_rows: Vec<Vec<Source>> = others
        .iter()
        .take(50)
        .map(|sources| {
            sources
                .iter()
                .map(|text| Source::new(text.as_ref()))
                .collect()
        })
        .collect();

    // The pieces that can start at each position of the first result,
    // with what they give in every example walked.
    struct Edge {
        piece: Piece,
        cost: u32,
        /// What the piece gives in each example, in characters.
        gives: Vec<Vec<char>>,
    }
    let mut edges: Vec<Vec<Edge>> = (0..first_output.len()).map(|_| Vec::new()).collect();
    let mut seen: HashMap<(usize, Piece), ()> = HashMap::new();
    for (start, _, piece) in candidates(first_sources, first_output) {
        if seen.insert((start, piece.clone()), ()).is_some() {
            continue;
        }
        let gives: Option<Vec<Vec<char>>> = joint
            .iter()
            .map(|(sources, output)| {
                let given: Vec<char> = piece.eval(sources)?.chars().collect();
                // It must at least occur somewhere in that result.
                output
                    .windows(given.len())
                    .any(|w| w == given.as_slice())
                    .then_some(given)
            })
            .collect();
        let Some(gives) = gives else { continue };
        let missing = other_rows
            .iter()
            .filter(|sources| piece.eval(sources).is_none())
            .count() as u32;
        // Enough to choose between two pieces that are otherwise as good.
        let penalty = if other_rows.is_empty() {
            0
        } else {
            30 * missing / other_rows.len() as u32
        };
        edges[start].push(Edge {
            cost: piece_cost(&piece) + penalty,
            piece,
            gives,
        });
    }

    // Dijkstra over the positions reached in each example.
    type State = Vec<u16>;
    let goal: State = joint
        .iter()
        .map(|(_, output)| output.len() as u16)
        .collect();
    let start: State = vec![0; joint.len()];
    let mut best: HashMap<State, (u32, Option<(State, Piece)>)> = HashMap::new();
    let mut queue: BinaryHeap<Reverse<(u32, State)>> = BinaryHeap::new();
    best.insert(start.clone(), (0, None));
    queue.push(Reverse((0, start)));
    let mut visited = 0usize;
    let mut reached = false;
    while let Some(Reverse((cost, state))) = queue.pop() {
        if best.get(&state).is_some_and(|(known, _)| *known < cost) {
            continue;
        }
        if state == goal {
            reached = true;
            break;
        }
        visited += 1;
        if visited > MAX_STATES {
            break;
        }
        let at = state[0] as usize;
        if at >= first_output.len() {
            continue;
        }
        let mut relax =
            |next: State,
             cost: u32,
             piece: Piece,
             best: &mut HashMap<State, (u32, Option<(State, Piece)>)>| {
                if best.get(&next).is_none_or(|(known, _)| cost < *known) {
                    best.insert(next.clone(), (cost, Some((state.clone(), piece))));
                    queue.push(Reverse((cost, next)));
                }
            };
        for edge in &edges[at] {
            let next: Option<State> = joint
                .iter()
                .zip(&edge.gives)
                .zip(&state)
                .map(|(((_, output), given), pos)| {
                    output[*pos as usize..]
                        .starts_with(given)
                        .then_some(*pos + given.len() as u16)
                })
                .collect();
            if let Some(next) = next {
                relax(next, cost + edge.cost, edge.piece.clone(), &mut best);
            }
        }
        // A constant: the same characters next in every example.
        for len in 1..=(first_output.len() - at) {
            let constant = &first_output[at..at + len];
            let same = joint
                .iter()
                .zip(&state)
                .all(|((_, output), pos)| output[*pos as usize..].starts_with(constant));
            if !same {
                break;
            }
            let text: String = constant.iter().collect();
            let next: State = state.iter().map(|pos| pos + len as u16).collect();
            relax(
                next,
                cost + const_cost(&text),
                Piece::Const(text),
                &mut best,
            );
        }
    }
    if !reached {
        return None;
    }
    let mut pieces = Vec::new();
    let mut state = goal;
    while let Some((_, Some((previous, piece)))) = best.get(&state) {
        pieces.push(piece.clone());
        state = previous.clone();
    }
    pieces.reverse();
    // The same text in every row is not a pattern.
    if pieces.iter().all(|piece| matches!(piece, Piece::Const(_))) {
        return None;
    }
    let program = Program { pieces };
    // The examples beyond those walked must agree too.
    let agrees = parsed.iter().all(|(sources, output)| {
        program
            .run(sources)
            .is_some_and(|out| out.chars().eq(output.iter().copied()))
    });
    agrees.then_some(program)
}

/// Why Flash Fill filled nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlashFillError {
    /// No data beside the column, no example in it, or no pattern that
    /// explains the examples: Excel has one message for all three.
    NoPattern,
}

/// What Flash Fill would write: the column, and the text for each row.
#[derive(Clone, Debug, PartialEq)]
pub struct FlashFill {
    pub col: u32,
    pub cells: Vec<(u32, String)>,
}

impl FlashFill {
    /// The cells written, as one rectangle.
    pub fn extent(&self) -> Option<Rect> {
        let first = self.cells.iter().map(|(row, _)| *row).min()?;
        let last = self.cells.iter().map(|(row, _)| *row).max()?;
        Some(Rect::new(first, self.col, last, self.col))
    }
}

/// The text of a cell as the user reads it.
fn shown(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    match wb.value_at(sheet, row, col) {
        Value::Text(text) => text.to_string(),
        Value::Empty => String::new(),
        _ => wb.display_text(sheet, row, col).text,
    }
}

impl Workbook {
    /// Work out what Flash Fill does for the column of the cell (`row`,
    /// `col`): the examples are the cells already filled in that column
    /// beside the list, the sources the other columns of the list.
    pub fn flash_fill_plan(
        &self,
        sheet: usize,
        row: u32,
        col: u32,
    ) -> Result<FlashFill, FlashFillError> {
        let sh = self.sheets.get(sheet).ok_or(FlashFillError::NoPattern)?;
        // The list is the block of data the column touches: around the
        // active cell, or around its neighbours when the cell is empty.
        let mut region = self.current_region(sheet, row, col);
        for near in [col.checked_sub(1), Some(col + 1)].into_iter().flatten() {
            if !sh.is_blank(row, near) {
                region = region.union(&self.current_region(sheet, row, near));
            }
        }
        let columns: Vec<u32> = (region.c0..=region.c1).filter(|c| *c != col).collect();
        if columns.is_empty() {
            return Err(FlashFillError::NoPattern);
        }
        let sources =
            |r: u32| -> Vec<String> { columns.iter().map(|c| shown(self, sheet, r, *c)).collect() };
        let has_data = |r: u32| columns.iter().any(|c| !sh.is_blank(r, *c));
        let mut examples: Vec<(u32, (Vec<String>, String))> = Vec::new();
        let mut blanks: Vec<u32> = Vec::new();
        for r in region.r0..=region.r1 {
            // A formula in the column is a result, not an example.
            let typed = sh
                .cells
                .get(&(r, col))
                .is_some_and(|cell| matches!(cell.content, Content::Value(_)));
            if typed && !sh.is_blank(r, col) {
                examples.push((r, (sources(r), shown(self, sheet, r, col))));
            } else if sh.is_blank(r, col) && has_data(r) && !sh.filtered_rows.contains(&r) {
                blanks.push(r);
            }
        }
        if examples.is_empty() || blanks.is_empty() {
            return Err(FlashFillError::NoPattern);
        }
        let others: Vec<Vec<String>> = blanks.iter().map(|r| sources(*r)).collect();
        let learn_from = |examples: &[(u32, (Vec<String>, String))]| {
            let list: Vec<(Vec<String>, String)> = examples
                .iter()
                .map(|(_, example)| example.clone())
                .collect();
            learn(&list, &others)
        };
        // A title above the examples is not one of them.
        let program = learn_from(&examples)
            .or_else(|| {
                if examples.len() > 1 && examples[0].0 == region.r0 {
                    learn_from(&examples[1..])
                } else {
                    None
                }
            })
            .ok_or(FlashFillError::NoPattern)?;
        let cells: Vec<(u32, String)> = blanks
            .iter()
            .zip(&others)
            .filter_map(|(r, sources)| program.apply(sources).map(|text| (*r, text)))
            .collect();
        if cells.is_empty() {
            return Err(FlashFillError::NoPattern);
        }
        Ok(FlashFill { col, cells })
    }

    /// Write what [`Workbook::flash_fill_plan`] worked out. The results
    /// are stored the way typing them would (digits become numbers),
    /// except that none becomes a formula. The caller commits.
    pub fn flash_fill_apply(&mut self, sheet: usize, fill: &FlashFill) {
        for (row, text) in &fill.cells {
            let formula_like = text.starts_with(['=', '+', '-'])
                && crate::input::parse_numeric_text(text, self.locale).is_none();
            if formula_like || self.set_cell_text(sheet, *row, fill.col, text).is_err() {
                self.set_content(sheet, *row, fill.col, Content::Value(Value::text(text)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{val, wb};

    /// Learn from (source, result) pairs of one column and apply to others.
    fn fill(examples: &[(&str, &str)], rows: &[&str]) -> Vec<Option<String>> {
        let examples: Vec<(Vec<&str>, &str)> = examples
            .iter()
            .map(|(source, result)| (vec![*source], *result))
            .collect();
        let others: Vec<Vec<&str>> = rows.iter().map(|row| vec![*row]).collect();
        match learn(&examples, &others) {
            Some(program) => rows.iter().map(|row| program.apply(&[*row])).collect(),
            None => vec![None; rows.len()],
        }
    }

    fn some(texts: &[&str]) -> Vec<Option<String>> {
        texts.iter().map(|text| Some(text.to_string())).collect()
    }

    #[test]
    fn first_and_last_name() {
        let rows = ["Marie Curie", "Victor Hugo", "Jean-Paul Sartre"];
        assert_eq!(
            fill(&[("Jean Dupont", "Jean")], &rows),
            some(&["Marie", "Victor", "Jean"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "Dupont")], &rows),
            some(&["Curie", "Hugo", "Sartre"])
        );
        // Three words: one example means the last word, a second one can say otherwise.
        assert_eq!(
            fill(&[("Jean Dupont", "Dupont")], &["Anne de Gaulle"]),
            some(&["Gaulle"])
        );
        assert_eq!(
            fill(
                &[("Jean Dupont", "Dupont"), ("Anne de Gaulle", "de")],
                &["Pierre le Grand"]
            ),
            some(&["le"])
        );
    }

    #[test]
    fn initials() {
        let rows = ["Marie Curie", "victor hugo"];
        assert_eq!(fill(&[("Jean Dupont", "JD")], &rows), some(&["MC", "vh"]));
        assert_eq!(
            fill(&[("Jean Dupont", "J. Dupont")], &rows),
            some(&["M. Curie", "v. hugo"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "J.D.")], &rows),
            some(&["M.C.", "v.h."])
        );
        // A second example says the initials are capitals.
        assert_eq!(
            fill(&[("Jean Dupont", "JD"), ("anne martin", "AM")], &rows),
            some(&["MC", "VH"])
        );
    }

    #[test]
    fn swapped_names() {
        assert_eq!(
            fill(
                &[("Dupont, Jean", "Jean Dupont")],
                &["Curie, Marie", "Hugo, Victor"]
            ),
            some(&["Marie Curie", "Victor Hugo"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "DUPONT Jean")], &["Marie Curie"]),
            some(&["CURIE Marie"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "Dupont, J.")], &["Marie Curie"]),
            some(&["Curie, M."])
        );
    }

    #[test]
    fn email_from_name() {
        assert_eq!(
            fill(
                &[("Jean Dupont", "jean.dupont@example.com")],
                &["Marie Curie", "VICTOR HUGO"]
            ),
            some(&["marie.curie@example.com", "victor.hugo@example.com"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "jdupont@example.com")], &["Marie Curie"]),
            some(&["mcurie@example.com"])
        );
        // And back: the name out of the address, the domain.
        assert_eq!(
            fill(
                &[("jean.dupont@example.com", "Jean Dupont")],
                &["marie.curie@labo.fr"]
            ),
            some(&["Marie Curie"])
        );
        assert_eq!(
            fill(
                &[("jean.dupont@example.com", "example.com")],
                &["marie.curie@labo.fr", "a@b.co.uk"]
            ),
            some(&["labo.fr", "b.co.uk"])
        );
    }

    #[test]
    fn digits_of_a_reference() {
        assert_eq!(
            fill(&[("AB-1234-X", "1234")], &["CD-98-Y", "ZZ-5-Q"]),
            some(&["98", "5"])
        );
        assert_eq!(
            fill(&[("Invoice 2024/017", "017")], &["Invoice 2023/112"]),
            some(&["112"])
        );
        assert_eq!(fill(&[("ref12abc", "abc")], &["ref7xy"]), some(&["xy"]));
        assert_eq!(
            fill(&[("AB-1234-X", "AB1234")], &["CD-98-Y"]),
            some(&["CD98"])
        );
    }

    #[test]
    fn phone_and_date_formatting() {
        assert_eq!(
            fill(&[("0612345678", "06 12 34 56 78")], &["0145670011"]),
            some(&["01 45 67 00 11"])
        );
        assert_eq!(
            fill(&[("5551234567", "(555) 123-4567")], &["2125550199"]),
            some(&["(212) 555-0199"])
        );
        assert_eq!(
            fill(&[("31/01/2024", "2024-01-31")], &["15/08/1999"]),
            some(&["1999-08-15"])
        );
        assert_eq!(
            fill(&[("31/01/2024", "01/2024")], &["15/08/1999"]),
            some(&["08/1999"])
        );
    }

    #[test]
    fn case_changes() {
        assert_eq!(
            fill(&[("jean dupont", "Jean Dupont")], &["MARIE CURIE"]),
            some(&["Marie Curie"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "JEAN DUPONT")], &["marie curie"]),
            some(&["MARIE CURIE"])
        );
        assert_eq!(
            fill(&[("Jean Dupont", "jean")], &["MARIE Curie"]),
            some(&["marie"])
        );
    }

    #[test]
    fn several_source_columns() {
        let examples = [(vec!["Jean", "Dupont"], "Dupont, Jean")];
        let program = learn(&examples, &[]).expect("a program");
        assert_eq!(
            program.apply(&["Marie", "Curie"]).as_deref(),
            Some("Curie, Marie")
        );
        let examples = [(vec!["Jean", "Dupont", "Paris"], "J. Dupont (Paris)")];
        let program = learn(&examples, &[]).expect("a program");
        assert_eq!(
            program.apply(&["Marie", "Curie", "Lyon"]).as_deref(),
            Some("M. Curie (Lyon)")
        );
    }

    #[test]
    fn no_pattern() {
        // Nothing of the source in the result: a constant is not a pattern.
        assert_eq!(
            fill(&[("Jean Dupont", "xyz")], &["Marie Curie"]),
            vec![None]
        );
        // Two examples that no one rule explains.
        assert_eq!(
            fill(
                &[("Jean Dupont", "Jean"), ("Marie Curie", "Curie")],
                &["Victor Hugo"]
            ),
            vec![None]
        );
        // A row without the piece gets nothing.
        assert_eq!(fill(&[("AB-1234", "1234")], &["none here"]), vec![None]);
    }

    #[test]
    fn fills_the_column_beside_a_list() {
        let mut w = wb(&[
            ("A1", "Name"),
            ("B1", "First"),
            ("A2", "Jean Dupont"),
            ("B2", "Jean"),
            ("A3", "Marie Curie"),
            ("A4", "Victor Hugo"),
        ]);
        // From any cell of the column.
        let fill = w.flash_fill_plan(0, 2, 1).unwrap();
        assert_eq!(
            fill.cells,
            vec![(2, "Marie".to_string()), (3, "Victor".to_string())]
        );
        assert_eq!(fill.extent(), Some(Rect::new(2, 1, 3, 1)));
        w.flash_fill_apply(0, &fill);
        w.commit("Flash Fill");
        assert_eq!(val(&w, "B3"), Value::text("Marie"));
        assert_eq!(val(&w, "B4"), Value::text("Victor"));
        w.undo();
        assert_eq!(val(&w, "B3"), Value::Empty);
        assert_eq!(val(&w, "B4"), Value::Empty);
    }

    #[test]
    fn numbers_and_refusals_on_the_sheet() {
        let mut w = wb(&[
            ("A1", "AB-12"),
            ("A2", "CD-345"),
            ("A3", "EF-=6"),
            ("B1", "12"),
            ("D1", "x"),
        ]);
        let fill = w.flash_fill_plan(0, 1, 1).unwrap();
        w.flash_fill_apply(0, &fill);
        w.commit("Flash Fill");
        // Digits are stored as typing them would.
        assert_eq!(val(&w, "B2"), Value::Number(345.0));
        assert_eq!(val(&w, "B3"), Value::Number(6.0));
        // No example, no neighbour, nothing left to fill.
        assert_eq!(w.flash_fill_plan(0, 0, 3), Err(FlashFillError::NoPattern));
        assert_eq!(w.flash_fill_plan(0, 9, 9), Err(FlashFillError::NoPattern));
        assert_eq!(w.flash_fill_plan(0, 0, 1), Err(FlashFillError::NoPattern));
    }
}
