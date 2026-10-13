//! Find and Replace: what a search looks at, in which order it walks the
//! cells, Excel's wildcards, and replacing inside constants and formulas.
//!
//! A search reads a cell the way the user does: its formula in the
//! language of the workbook (Look in: Formulas), what the cell shows
//! (Values) or its note or comment (Comments). A replacement edits that
//! text and enters it again as if it had been typed, so numbers and dates
//! stay numbers and dates and a formula is parsed again; one that no
//! longer parses is refused and nothing is changed.

use crate::ast::Expr;
use crate::parser::ParseError;
use crate::protect::Action;
use crate::refs::{Rect, col_to_letters};
use crate::workbook::Workbook;

/// What a search reads in each cell (the "Look in" box).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LookIn {
    /// What the cells show.
    #[default]
    Values,
    /// What the formula bar shows: formulas and constants as typed.
    Formulas,
    /// Notes and comments.
    Comments,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_cell: bool,
    pub look_in: LookIn,
    /// Walk the cells column after column ("Search: By Columns") instead
    /// of row after row.
    pub by_columns: bool,
}

/// A cell of the workbook: sheet, row, column.
pub type Place = (usize, u32, u32);

/// A search as the Find and Replace dialogs describe it.
#[derive(Clone, Copy, Debug)]
pub struct Search<'a> {
    pub needle: &'a str,
    pub options: &'a FindOptions,
    /// Every sheet ("Within: Workbook") instead of one.
    pub workbook: bool,
    /// The ranges searched on the sheet the search starts from: Excel
    /// keeps to the selection when it is more than one cell. Empty for
    /// the whole sheet; never used across a workbook.
    pub within: &'a [Rect],
}

/// What Replace All did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Replaced {
    /// Cells changed.
    pub cells: usize,
    /// Cells holding the text that were left alone because their sheet
    /// is protected and they are locked.
    pub locked: usize,
}

/// A replacement that cannot be made: the formula of `place` would no
/// longer be one. Nothing was changed.
#[derive(Clone, Debug, PartialEq)]
pub struct BrokenFormula {
    pub place: Place,
    /// The text the cell would have held.
    pub text: String,
    pub error: ParseError,
}

/// One line of the list Find All shows: where the cell is and what it
/// holds, as Excel's columns Sheet, Name, Cell, Value and Formula read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub place: Place,
    pub sheet: String,
    /// The defined name that stands for exactly this cell, if there is one.
    pub name: String,
    /// The cell as an absolute reference (`$B$4`).
    pub cell: String,
    /// What the cell shows.
    pub value: String,
    /// Its formula, when it holds one that may be read.
    pub formula: String,
}

// ---- patterns ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Char(char),
    /// `?`: any one character.
    One,
    /// `*`: any run of characters.
    Many,
}

/// The text looked for, with Excel's wildcards: `?` and `*`, and `~`
/// before one of them (or before itself) for the character as it stands.
#[derive(Clone, Debug)]
struct Pattern {
    tokens: Vec<Token>,
    match_case: bool,
}

/// One character without its case: enough to compare texts letter by
/// letter while keeping their positions.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

impl Pattern {
    fn new(needle: &str, match_case: bool) -> Pattern {
        let mut tokens = Vec::new();
        let mut chars = needle.chars().peekable();
        while let Some(c) = chars.next() {
            let token = match c {
                '~' if matches!(chars.peek(), Some('*' | '?' | '~')) => Token::Char(chars.next().unwrap_or('~')),
                '?' => Token::One,
                '*' => Token::Many,
                c => Token::Char(c),
            };
            // Two stars in a row say no more than one.
            if token == Token::Many && tokens.last() == Some(&Token::Many) {
                continue;
            }
            tokens.push(match token {
                Token::Char(c) if !match_case => Token::Char(fold(c)),
                other => other,
            });
        }
        Pattern { tokens, match_case }
    }

    fn same(&self, pattern: char, text: char) -> bool {
        pattern == if self.match_case { text } else { fold(text) }
    }

    /// The end of the longest match of `tokens` starting at `at`; with
    /// `to_end`, only a match that reaches the end of the text counts.
    fn match_from(&self, tokens: &[Token], hay: &[char], at: usize, to_end: bool) -> Option<usize> {
        match tokens.split_first() {
            None => (!to_end || at == hay.len()).then_some(at),
            Some((Token::Many, rest)) => {
                (at..=hay.len()).rev().find_map(|next| self.match_from(rest, hay, next, to_end))
            }
            Some((token, rest)) => {
                let c = *hay.get(at)?;
                let fits = match token {
                    Token::Char(p) => self.same(*p, c),
                    _ => true,
                };
                if fits { self.match_from(rest, hay, at + 1, to_end) } else { None }
            }
        }
    }

    /// The first match at or after `from`, as (start, end).
    fn find(&self, hay: &[char], from: usize) -> Option<(usize, usize)> {
        (from..=hay.len()).find_map(|start| self.match_from(&self.tokens, hay, start, false).map(|end| (start, end)))
    }

    fn is_whole(&self, hay: &[char]) -> bool {
        self.match_from(&self.tokens, hay, 0, true).is_some()
    }

    fn matches(&self, text: &str, whole_cell: bool) -> bool {
        if text.is_empty() || self.tokens.is_empty() {
            return false;
        }
        let hay: Vec<char> = text.chars().collect();
        if whole_cell { self.is_whole(&hay) } else { self.find(&hay, 0).is_some() }
    }

    /// `text` with every match replaced; `None` when there is none.
    fn replace(&self, text: &str, replacement: &str, whole_cell: bool) -> Option<String> {
        if text.is_empty() || self.tokens.is_empty() {
            return None;
        }
        let hay: Vec<char> = text.chars().collect();
        if whole_cell {
            return self.is_whole(&hay).then(|| replacement.to_string());
        }
        let mut out = String::new();
        let mut at = 0;
        let mut found = false;
        while let Some((start, end)) = self.find(&hay, at) {
            // A star alone can match nothing at all: such a match is no
            // occurrence.
            if end == start {
                if start >= hay.len() {
                    break;
                }
                out.extend(&hay[at..=start]);
                at = start + 1;
                continue;
            }
            found = true;
            out.extend(&hay[at..start]);
            out.push_str(replacement);
            at = end;
        }
        out.extend(&hay[at..]);
        found.then_some(out)
    }
}

// ---- searching --------------------------------------------------------------

impl Workbook {
    /// Whether Find looks at a cell: never in a row a filter hides, and
    /// only when looking in formulas in a row or a column hidden by hand,
    /// as in Excel.
    fn is_searched(&self, sheet: usize, row: u32, col: u32, opts: &FindOptions) -> bool {
        let sh = &self.sheets[sheet];
        !sh.filtered_rows.contains(&row)
            && (opts.look_in == LookIn::Formulas || !(sh.hidden_rows.contains(&row) || sh.hidden_cols.contains(&col)))
    }

    /// The text a search reads in a cell.
    fn searched_text(&self, sheet: usize, row: u32, col: u32, look_in: LookIn) -> String {
        match look_in {
            // A formula hidden by the protection of its sheet is not read.
            LookIn::Formulas => self.formula_bar_text(sheet, row, col),
            LookIn::Values => self.display_text(sheet, row, col).text,
            LookIn::Comments => match self.comment_at(sheet, row, col) {
                Some(comment) => match (comment.as_note(), comment.as_thread()) {
                    (Some(note), _) => note.text(),
                    (_, Some(thread)) => thread.posts().map(|post| post.text.as_str()).collect::<Vec<_>>().join("\n"),
                    _ => String::new(),
                },
                None => String::new(),
            },
        }
    }

    /// The cells of a sheet a search may stop on, in the order it walks
    /// them.
    fn candidates(&self, sheet: usize, opts: &FindOptions, within: &[Rect]) -> Vec<(u32, u32)> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        let wanted = |row: u32, col: u32| {
            (within.is_empty() || within.iter().any(|r| r.contains(row, col))) && self.is_searched(sheet, row, col, opts)
        };
        let mut cells: Vec<(u32, u32)> = match opts.look_in {
            LookIn::Comments => sh.comments.keys().copied().filter(|(r, c)| wanted(*r, *c)).collect(),
            _ => sh.cells.keys().copied().filter(|(r, c)| wanted(*r, *c)).collect(),
        };
        // The cells a dynamic array spills into show values of their own.
        if opts.look_in == LookIn::Values && !sh.spills.is_empty() {
            for spill in &sh.spills {
                for r in 0..spill.array.rows as u32 {
                    for c in 0..spill.array.cols as u32 {
                        let (row, col) = (spill.row + r, spill.col + c);
                        if (r, c) != (0, 0) && wanted(row, col) {
                            cells.push((row, col));
                        }
                    }
                }
            }
            cells.sort_unstable();
            cells.dedup();
        }
        if opts.by_columns {
            cells.sort_unstable_by_key(|(row, col)| (*col, *row));
        }
        cells
    }

    fn place_matches(&self, place: Place, pattern: &Pattern, opts: &FindOptions) -> bool {
        pattern.matches(&self.searched_text(place.0, place.1, place.2, opts.look_in), opts.whole_cell)
    }

    /// The sheets of a search in the order it visits them from `sheet`,
    /// each with the ranges it is limited to.
    fn search_sheets<'a>(&self, search: &Search<'a>, sheet: usize) -> Vec<(usize, &'a [Rect])> {
        if !search.workbook {
            return vec![(sheet, search.within)];
        }
        let count = self.sheets.len();
        (0..count).map(|i| ((sheet + i) % count, &[][..])).collect()
    }

    /// The next cell holding the text after `from` (before it with
    /// `backward`), in the order of the search and coming back round to
    /// `from` itself, as Find Next does. Cells that are not shown are
    /// passed over as Excel does: those of rows a filter hides always,
    /// those hidden by hand unless the search looks in formulas.
    pub fn find_from(&self, search: &Search, from: Place, backward: bool) -> Option<Place> {
        let pattern = Pattern::new(search.needle, search.options.match_case);
        let opts = search.options;
        if from.0 >= self.sheets.len() {
            return None;
        }
        let key = |(row, col): (u32, u32)| if opts.by_columns { (col, row) } else { (row, col) };
        let here = key((from.1, from.2));
        let sheets = self.search_sheets(search, from.0);
        // The starting sheet is walked in two parts: what comes after the
        // cell first, what comes before it (and the cell) last.
        let mut parts: Vec<Vec<Place>> = Vec::with_capacity(sheets.len() + 1);
        let mut last: Vec<Place> = Vec::new();
        for (i, (sheet, within)) in sheets.iter().enumerate() {
            let cells = self.candidates(*sheet, opts, within).into_iter().map(|(row, col)| (*sheet, row, col));
            if i > 0 {
                parts.push(cells.collect());
                continue;
            }
            let (mut ahead, mut behind) = (Vec::new(), Vec::new());
            for place in cells {
                let k = key((place.1, place.2));
                let is_ahead = if backward { k < here } else { k > here };
                if is_ahead { ahead.push(place) } else { behind.push(place) }
            }
            parts.push(ahead);
            last = behind;
        }
        if backward {
            // The other sheets come in reverse too.
            parts[1..].reverse();
            parts.push(last);
            parts
                .iter()
                .flat_map(|part| part.iter().rev())
                .find(|place| self.place_matches(**place, &pattern, opts))
                .copied()
        } else {
            parts.push(last);
            parts.iter().flatten().find(|place| self.place_matches(**place, &pattern, opts)).copied()
        }
    }

    /// Every cell holding the text, sheet after sheet from `sheet`, in the
    /// order of the search.
    pub fn find_everywhere(&self, search: &Search, sheet: usize) -> Vec<Place> {
        if sheet >= self.sheets.len() {
            return Vec::new();
        }
        let pattern = Pattern::new(search.needle, search.options.match_case);
        let mut out = Vec::new();
        for (sheet, within) in self.search_sheets(search, sheet) {
            for (row, col) in self.candidates(sheet, search.options, within) {
                if self.place_matches((sheet, row, col), &pattern, search.options) {
                    out.push((sheet, row, col));
                }
            }
        }
        out
    }

    /// The defined name that stands for one cell and nothing else: the
    /// first in the order of the Name Manager.
    fn name_of_cell(&self, place: Place) -> Option<&str> {
        let (sheet, row, col) = place;
        self.all_names().into_iter().find_map(|(scope, def)| {
            let mut expr = def.expr_at(row, col);
            while let Expr::Paren(inner) = expr {
                expr = *inner;
            }
            let Expr::Ref(r) = expr else {
                return None;
            };
            let on = match (&r.sheet, scope) {
                (Some(name), _) => self.sheet_index(name)?,
                (None, crate::names::NameScope::Sheet(own)) => own,
                (None, crate::names::NameScope::Workbook) => sheet,
            };
            (on == sheet && r.rect() == Rect::cell(row, col)).then_some(def.name.as_str())
        })
    }

    /// A cell as the list of Find All describes it.
    pub fn found(&self, place: Place) -> Found {
        let (sheet, row, col) = place;
        let formula = match self.formula_text(sheet, row, col) {
            // A formula the protection of its sheet hides stays hidden.
            Some(_) => self.formula_bar_text(sheet, row, col),
            None => String::new(),
        };
        Found {
            place,
            sheet: self.sheets.get(sheet).map(|sh| sh.name.to_string()).unwrap_or_default(),
            name: self.name_of_cell(place).unwrap_or_default().to_string(),
            cell: format!("${}${}", col_to_letters(col), row + 1),
            value: self.display_text(sheet, row, col).text,
            formula,
        }
    }

    /// Next cell after `from` (reading order, wrapping) whose text matches.
    pub fn find_next(&self, sheet: usize, from: (u32, u32), needle: &str, opts: &FindOptions) -> Option<(u32, u32)> {
        let search = Search {
            needle,
            options: opts,
            workbook: false,
            within: &[],
        };
        self.find_from(&search, (sheet, from.0, from.1), false).map(|(_, row, col)| (row, col))
    }

    /// Every cell of a sheet whose text matches, in the order of the
    /// search.
    pub fn find_all(&self, sheet: usize, needle: &str, opts: &FindOptions) -> Vec<(u32, u32)> {
        let search = Search {
            needle,
            options: opts,
            workbook: false,
            within: &[],
        };
        self.find_everywhere(&search, sheet).into_iter().map(|(_, row, col)| (row, col)).collect()
    }

    // ---- replacing ------------------------------------------------------

    /// A replacement always works on what the formula bar shows.
    fn replace_options(opts: &FindOptions) -> FindOptions {
        FindOptions {
            look_in: LookIn::Formulas,
            ..opts.clone()
        }
    }

    /// What a cell would hold after the replacement, as text to enter;
    /// `None` when it does not hold the text looked for.
    pub fn replacement_text(&self, place: Place, search: &Search, replacement: &str) -> Option<String> {
        let (sheet, row, col) = place;
        let opts = Self::replace_options(search.options);
        if sheet >= self.sheets.len() || !self.is_searched(sheet, row, col, &opts) {
            return None;
        }
        let pattern = Pattern::new(search.needle, opts.match_case);
        let text = self.searched_text(sheet, row, col, LookIn::Formulas);
        pattern.replace(&text, replacement, opts.whole_cell).filter(|new| *new != text)
    }

    /// Whether what a cell holds may be changed (its sheet is not
    /// protected, or the cell is unlocked).
    fn is_editable(&self, place: Place) -> bool {
        let ranges = [Rect::cell(place.1, place.2)];
        self.permits(&Action::EditCells {
            sheet: place.0,
            ranges: &ranges,
        })
        .is_ok()
    }

    /// Replace the text in one cell (the Replace button). `Ok(false)` when
    /// the cell does not hold it. The caller asks about protection.
    pub fn replace_at(&mut self, place: Place, search: &Search, replacement: &str) -> Result<bool, BrokenFormula> {
        let Some(text) = self.replacement_text(place, search, replacement) else {
            return Ok(false);
        };
        match self.set_cell_text(place.0, place.1, place.2, &text) {
            Ok(()) => Ok(true),
            Err(error) => Err(BrokenFormula { place, text, error }),
        }
    }

    /// Replace All: every occurrence in the constants and the formulas the
    /// search covers, starting from `sheet`. Locked cells of protected
    /// sheets are left alone and counted. When a formula would be broken
    /// nothing at all is replaced.
    pub fn replace_everywhere(&mut self, search: &Search, sheet: usize, replacement: &str) -> Result<Replaced, BrokenFormula> {
        let opts = Self::replace_options(search.options);
        let search = Search { options: &opts, ..*search };
        let mut report = Replaced::default();
        let mut edits: Vec<(Place, String)> = Vec::new();
        for place in self.find_everywhere(&search, sheet) {
            let Some(text) = self.replacement_text(place, &search, replacement) else {
                continue;
            };
            if !self.is_editable(place) {
                report.locked += 1;
                continue;
            }
            // Formulas are checked before anything is written.
            if let Err(error) = crate::input::parse_input(&text, self.locale) {
                return Err(BrokenFormula { place, text, error });
            }
            edits.push((place, text));
        }
        for ((sheet, row, col), text) in edits {
            match self.set_cell_text(sheet, row, col, &text) {
                Ok(()) => report.cells += 1,
                Err(error) => {
                    // A formula that reads well but cannot be stored (a
                    // table reference without its table): undo the rest.
                    self.rollback();
                    return Err(BrokenFormula {
                        place: (sheet, row, col),
                        text,
                        error,
                    });
                }
            }
        }
        Ok(report)
    }

    /// Replace occurrences in the constants and formulas of one sheet.
    /// Returns the number of cells changed; none when a formula would be
    /// broken. Rows a filter hides are left alone, as in Excel.
    pub fn replace_all(&mut self, sheet: usize, needle: &str, replacement: &str, opts: &FindOptions) -> usize {
        let search = Search {
            needle,
            options: opts,
            workbook: false,
            within: &[],
        };
        self.replace_everywhere(&search, sheet, replacement).map(|done| done.cells).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::protect::SheetProtection;
    use crate::refs::parse_cell_name;
    use crate::tests::{val, wb};
    use crate::value::Value;

    fn at(name: &str) -> (u32, u32) {
        parse_cell_name(name).unwrap()
    }

    fn cells(names: &[&str]) -> Vec<(u32, u32)> {
        names.iter().map(|name| at(name)).collect()
    }

    fn formulas() -> FindOptions {
        FindOptions {
            look_in: LookIn::Formulas,
            ..Default::default()
        }
    }

    fn search<'a>(needle: &'a str, options: &'a FindOptions) -> Search<'a> {
        Search {
            needle,
            options,
            workbook: false,
            within: &[],
        }
    }

    fn pattern_replace(needle: &str, text: &str, with: &str) -> Option<String> {
        Pattern::new(needle, false).replace(text, with, false)
    }

    #[test]
    fn wildcards_match_as_in_excel() {
        let p = |needle: &str| Pattern::new(needle, false);
        assert!(p("an").matches("Banana", false));
        assert!(!p("an").matches("Banana", true));
        assert!(p("b?n*").matches("Banana", true));
        assert!(p("b*a").matches("Banana", true));
        assert!(!p("b?a").matches("Banana", false));
        // A tilde makes the next wildcard an ordinary character.
        assert!(p("~*").matches("2*3", false));
        assert!(!p("~*").matches("23", false));
        assert!(p("~?").matches("why?", false));
        assert!(p("a~~b").matches("a~b", true));
        assert!(p("~x").matches("~x", true));
        // Case counts only when asked.
        assert!(Pattern::new("ban", true).matches("banana", false));
        assert!(!Pattern::new("ban", true).matches("Banana", false));
        // An empty cell holds nothing to find, not even for a star.
        assert!(!p("*").matches("", false));
        assert!(p("*").matches("x", true));
    }

    #[test]
    fn every_occurrence_in_a_text_is_replaced() {
        assert_eq!(pattern_replace("an", "Banana", "o").as_deref(), Some("Booa"));
        assert_eq!(pattern_replace("AN", "Banana", "").as_deref(), Some("Ba"));
        // A star takes as much as it can.
        assert_eq!(pattern_replace("a*", "Banana", "X").as_deref(), Some("BX"));
        assert_eq!(pattern_replace("?", "abc", "-").as_deref(), Some("---"));
        assert_eq!(pattern_replace("*", "abc", "x").as_deref(), Some("x"));
        assert_eq!(pattern_replace("z", "abc", "x"), None);
        assert_eq!(pattern_replace("é", "Été", "e").as_deref(), Some("ete"));
        assert_eq!(Pattern::new("a?c", false).replace("abc", "new", true).as_deref(), Some("new"));
        assert_eq!(Pattern::new("ab", false).replace("abc", "new", true), None);
    }

    #[test]
    fn find_walks_by_rows_or_by_columns_and_wraps() {
        let w = wb(&[("A1", "x1"), ("C1", "x2"), ("A2", "x3"), ("B2", "y"), ("B3", "x4")]);
        let rows = FindOptions::default();
        let columns = FindOptions {
            by_columns: true,
            ..Default::default()
        };
        assert_eq!(w.find_all(0, "x", &rows), cells(&["A1", "C1", "A2", "B3"]));
        assert_eq!(w.find_all(0, "x", &columns), cells(&["A1", "A2", "B3", "C1"]));
        assert_eq!(w.find_next(0, at("A1"), "x", &rows), Some(at("C1")));
        assert_eq!(w.find_next(0, at("A1"), "x", &columns), Some(at("A2")));
        // After the last one the search comes back to the first.
        assert_eq!(w.find_next(0, at("B3"), "x", &rows), Some(at("A1")));
        assert_eq!(w.find_next(0, at("C1"), "x", &columns), Some(at("A1")));
        // A lone occurrence is found again from its own cell.
        assert_eq!(w.find_next(0, at("B2"), "y", &rows), Some(at("B2")));
        assert_eq!(w.find_next(0, at("B2"), "nothing", &rows), None);
        // Backwards (Shift with Find Next).
        let s = search("x", &rows);
        assert_eq!(w.find_from(&s, (0, 1, 0), true), Some((0, 0, 2)));
        assert_eq!(w.find_from(&s, (0, 0, 0), true), Some((0, 2, 1)));
    }

    #[test]
    fn find_looks_in_formulas_values_or_comments() {
        let mut w = wb(&[("A1", "=2+3"), ("A2", "5"), ("A3", "0.5")]);
        w.update_style(0, Rect::cell(2, 0), &|s| s.num_fmt = "0%".into());
        w.set_note(0, 4, 4, "Me", "five to check");
        w.commit("setup");
        let values = FindOptions::default();
        let comments = FindOptions {
            look_in: LookIn::Comments,
            ..Default::default()
        };
        assert_eq!(w.find_all(0, "5", &values), cells(&["A1", "A2", "A3"]));
        assert_eq!(w.find_all(0, "5", &formulas()), cells(&["A2", "A3"]));
        assert_eq!(w.find_all(0, "+", &formulas()), cells(&["A1"]));
        assert_eq!(w.find_all(0, "50%", &values), cells(&["A3"]));
        assert_eq!(w.find_all(0, "five", &comments), cells(&["E5"]));
        assert_eq!(w.find_all(0, "five", &values), Vec::new());
    }

    #[test]
    fn a_dynamic_array_is_searched_in_every_cell_it_fills() {
        let w = wb(&[("A1", "=SEQUENCE(3)")]);
        assert_eq!(w.find_all(0, "3", &FindOptions::default()), cells(&["A3"]));
        assert_eq!(w.find_all(0, "3", &formulas()), cells(&["A1"]));
    }

    #[test]
    fn a_search_keeps_to_the_selection_or_covers_the_workbook() {
        let mut w = wb(&[("A1", "tea"), ("B5", "tea"), ("C9", "teapot")]);
        let second = w.add_sheet(1);
        w.set_cell_text(second, 3, 3, "Tea").unwrap();
        w.commit("setup");
        let opts = FindOptions::default();
        let within = [Rect::parse("B1:C9").unwrap()];
        let selection = Search {
            within: &within,
            ..search("tea", &opts)
        };
        assert_eq!(w.find_everywhere(&selection, 0), vec![(0, 4, 1), (0, 8, 2)]);
        assert_eq!(w.find_from(&selection, (0, 8, 2), false), Some((0, 4, 1)));
        let everywhere = Search {
            workbook: true,
            ..search("tea", &opts)
        };
        assert_eq!(w.find_everywhere(&everywhere, 0), vec![(0, 0, 0), (0, 4, 1), (0, 8, 2), (1, 3, 3)]);
        // From the last cell of a sheet the search goes on to the next
        // sheet, and from the last sheet back to the first.
        assert_eq!(w.find_from(&everywhere, (0, 8, 2), false), Some((1, 3, 3)));
        assert_eq!(w.find_from(&everywhere, (1, 3, 3), false), Some((0, 0, 0)));
        assert_eq!(w.find_from(&everywhere, (0, 0, 0), true), Some((1, 3, 3)));
    }

    #[test]
    fn find_all_describes_each_cell() {
        let mut w = wb(&[("A1", "12.5"), ("B4", "=A1*2"), ("C2", "tea")]);
        let total = crate::parser::parse_formula("Sheet1!$B$4", w.locale).unwrap();
        w.add_name(crate::names::NameScope::Workbook, "Total", total, "").unwrap();
        w.commit("setup");
        let found = w.found((0, 3, 1));
        assert_eq!(
            (found.sheet.as_str(), found.name.as_str(), found.cell.as_str(), found.value.as_str(), found.formula.as_str()),
            ("Sheet1", "Total", "$B$4", "25", "=A1*2")
        );
        let plain = w.found((0, 1, 2));
        assert_eq!((plain.name.as_str(), plain.cell.as_str(), plain.value.as_str(), plain.formula.as_str()), ("", "$C$2", "tea", ""));
    }

    #[test]
    fn replacing_keeps_the_typing_rules() {
        let mut w = wb(&[("A1", "2023"), ("A2", "item 7"), ("A3", "1/2/2023"), ("A4", "=A1+7")]);
        assert_eq!(w.replace_all(0, "7", "8", &FindOptions::default()), 2);
        w.commit("replace");
        assert_eq!(val(&w, "A2"), Value::text("item 8"));
        assert_eq!(w.edit_text(0, 3, 0), "=A1+8");
        assert_eq!(val(&w, "A4"), Value::Number(2031.0));
        // What comes out a number is a number, a date stays a date.
        assert_eq!(w.replace_all(0, "2023", "2024", &FindOptions::default()), 2);
        w.commit("replace");
        assert_eq!(val(&w, "A1"), Value::Number(2024.0));
        assert!(matches!(val(&w, "A3"), Value::Number(n) if n > 45_000.0));
        assert_eq!(w.edit_text(0, 2, 0), "1/2/2024");
        assert!(w.undo());
        assert_eq!(val(&w, "A1"), Value::Number(2023.0));
    }

    #[test]
    fn formulas_are_replaced_as_the_user_reads_them() {
        let mut w = Workbook::new(Locale::by_id("fr"));
        w.set_cell_text(0, 0, 0, "=SOMME(B1:B3;1,5)").unwrap();
        w.commit("setup");
        // The French name of the function and the French separators.
        assert_eq!(w.find_all(0, "somme", &formulas()), cells(&["A1"]));
        assert_eq!(w.replace_all(0, "SOMME", "MOYENNE", &formulas()), 1);
        assert_eq!(w.replace_all(0, "1,5", "2,5", &formulas()), 1);
        w.commit("replace");
        assert_eq!(w.edit_text(0, 0, 0), "=MOYENNE(B1:B3;2,5)");
    }

    #[test]
    fn a_replacement_that_breaks_a_formula_changes_nothing() {
        let mut w = wb(&[("A1", "x)"), ("A2", "=SUM(B1:B3)")]);
        let opts = formulas();
        let broken = w.replace_everywhere(&search(")", &opts), 0, "").unwrap_err();
        assert_eq!(broken.place, (0, 1, 0));
        assert_eq!(broken.text, "=SUM(B1:B3");
        // Not even the constant found before the formula.
        assert_eq!(val(&w, "A1"), Value::text("x)"));
        assert_eq!(w.edit_text(0, 1, 0), "=SUM(B1:B3)");
        assert!(w.replace_at((0, 1, 0), &search(")", &opts), "").is_err());
        assert_eq!(w.replace_at((0, 0, 0), &search(")", &opts), ""), Ok(true));
        assert_eq!(w.replace_at((0, 0, 0), &search(")", &opts), ""), Ok(false));
    }

    #[test]
    fn replace_all_leaves_locked_cells_of_a_protected_sheet() {
        let mut w = wb(&[("A1", "old"), ("B1", "old"), ("B2", "older")]);
        w.update_style(0, Rect::parse("B1:B5").unwrap(), &|s| s.locked = false);
        w.commit("unlock");
        w.protect_sheet(0, SheetProtection::default(), "");
        w.commit("protect");
        let opts = FindOptions::default();
        let done = w.replace_everywhere(&search("old", &opts), 0, "new").unwrap();
        assert_eq!(done, Replaced { cells: 2, locked: 1 });
        w.commit("replace");
        assert_eq!(val(&w, "A1"), Value::text("old"));
        assert_eq!(val(&w, "B1"), Value::text("new"));
        assert_eq!(val(&w, "B2"), Value::text("newer"));
    }

    #[test]
    fn replace_all_across_the_workbook_is_one_undo_step() {
        let mut w = wb(&[("A1", "cat")]);
        let second = w.add_sheet(1);
        w.set_cell_text(second, 0, 0, "catalog").unwrap();
        w.commit("setup");
        let opts = FindOptions::default();
        let everywhere = Search {
            workbook: true,
            ..search("cat", &opts)
        };
        assert_eq!(w.replace_everywhere(&everywhere, 0, "dog").unwrap().cells, 2);
        w.commit("replace");
        assert_eq!(w.value_at(1, 0, 0), Value::text("dogalog"));
        assert!(w.undo());
        assert_eq!(w.value_at(0, 0, 0), Value::text("cat"));
        assert_eq!(w.value_at(1, 0, 0), Value::text("catalog"));
    }
}
