//! Splitting text into columns: the logic behind Excel's Convert Text to
//! Columns Wizard, also used to import a text file.
//!
//! A [`Spec`] says how a line is cut ([`Layout`]: at delimiters or at fixed
//! positions) and what becomes of each piece ([`ColumnFormat`]). Pieces
//! kept as "General" are stored the way typing them into a cell would, in
//! the locale of the workbook.

use crate::datetime::{date_to_serial, days_in_month};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Content, Workbook};
use std::sync::Arc;

/// The characters that end a field of delimited text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delimiters {
    pub tab: bool,
    pub semicolon: bool,
    pub comma: bool,
    pub space: bool,
    pub other: Option<char>,
    /// A run of delimiters ends one field only.
    pub consecutive_as_one: bool,
    /// Delimiters between two of these are part of the field (`"` by
    /// default; `None` for the wizard's "{none}").
    pub qualifier: Option<char>,
}

impl Default for Delimiters {
    /// What the wizard proposes: Tab alone, `"` as the text qualifier.
    fn default() -> Delimiters {
        Delimiters {
            tab: true,
            semicolon: false,
            comma: false,
            space: false,
            other: None,
            consecutive_as_one: false,
            qualifier: Some('"'),
        }
    }
}

impl Delimiters {
    /// Fields separated by `ch` alone.
    pub fn only(ch: char) -> Delimiters {
        let mut d = Delimiters {
            tab: false,
            ..Delimiters::default()
        };
        match ch {
            '\t' => d.tab = true,
            ';' => d.semicolon = true,
            ',' => d.comma = true,
            ' ' => d.space = true,
            other => d.other = Some(other),
        }
        d
    }

    fn is_delimiter(&self, ch: char) -> bool {
        (self.tab && ch == '\t')
            || (self.semicolon && ch == ';')
            || (self.comma && ch == ',')
            || (self.space && ch == ' ')
            || self.other == Some(ch)
    }
}

/// How a line is cut into fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    Delimited(Delimiters),
    /// Break lines, as numbers of characters from the start of the line,
    /// in increasing order.
    FixedWidth(Vec<usize>),
}

/// The order of day, month and year in a column converted to dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    MDY,
    DMY,
    YMD,
    MYD,
    DYM,
    YDM,
}

impl DateOrder {
    pub const ALL: [DateOrder; 6] = [
        DateOrder::MDY,
        DateOrder::DMY,
        DateOrder::YMD,
        DateOrder::MYD,
        DateOrder::DYM,
        DateOrder::YDM,
    ];

    /// The letters Excel shows ("DMY"); the French interface shows "JMA".
    pub fn letters(self) -> &'static str {
        match self {
            DateOrder::MDY => "MDY",
            DateOrder::DMY => "DMY",
            DateOrder::YMD => "YMD",
            DateOrder::MYD => "MYD",
            DateOrder::DYM => "DYM",
            DateOrder::YDM => "YDM",
        }
    }

    /// Year, month and day out of three numbers read in this order.
    fn arrange(self, n: [i64; 3]) -> (i64, i64, i64) {
        match self {
            DateOrder::MDY => (n[2], n[0], n[1]),
            DateOrder::DMY => (n[2], n[1], n[0]),
            DateOrder::YMD => (n[0], n[1], n[2]),
            DateOrder::MYD => (n[1], n[0], n[2]),
            DateOrder::DYM => (n[1], n[2], n[0]),
            DateOrder::YDM => (n[0], n[2], n[1]),
        }
    }

    /// Position of the year among the three numbers.
    fn year_index(self) -> usize {
        match self {
            DateOrder::MDY | DateOrder::DMY => 2,
            DateOrder::YMD | DateOrder::YDM => 0,
            DateOrder::MYD | DateOrder::DYM => 1,
        }
    }
}

/// What becomes of a column of fields ("Column data format").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColumnFormat {
    /// Numbers become numbers, dates become dates, the rest stays text.
    #[default]
    General,
    /// Kept as typed, in cells formatted as Text.
    Text,
    Date(DateOrder),
    /// "Do not import column (skip)".
    Skip,
}

/// Everything the wizard collects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub layout: Layout,
    /// Format of each field, in order; fields beyond the list are General.
    pub formats: Vec<ColumnFormat>,
}

impl Spec {
    pub fn new(layout: Layout) -> Spec {
        Spec {
            layout,
            formats: Vec::new(),
        }
    }

    pub fn format(&self, field: usize) -> ColumnFormat {
        self.formats.get(field).copied().unwrap_or_default()
    }

    /// The pieces of a line that are imported, with their formats.
    pub fn fields(&self, line: &str) -> Vec<(String, ColumnFormat)> {
        split_line(line, &self.layout)
            .into_iter()
            .enumerate()
            .map(|(i, field)| (field, self.format(i)))
            .filter(|(_, format)| *format != ColumnFormat::Skip)
            .collect()
    }
}

/// Why text cannot be converted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColumnsError {
    /// The source is more than one column wide.
    SeveralColumns,
    /// Nothing to convert in the selection.
    NoData,
    /// The result does not fit on the sheet.
    OffSheet,
}

/// Cut a line into its fields.
pub fn split_line(line: &str, layout: &Layout) -> Vec<String> {
    match layout {
        Layout::Delimited(delimiters) => split_delimited(line, delimiters),
        Layout::FixedWidth(breaks) => split_fixed(line, breaks),
    }
}

fn split_delimited(line: &str, d: &Delimiters) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    // The field in progress started with a qualifier: a closing one is
    // looked for, and what follows it up to the delimiter is kept as is.
    let mut chars = line.chars().peekable();
    let mut at_start = true;
    while let Some(ch) = chars.next() {
        if in_quotes {
            if Some(ch) == d.qualifier {
                // A doubled qualifier stands for one.
                if chars.peek().copied() == d.qualifier {
                    field.push(ch);
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
        } else if at_start && Some(ch) == d.qualifier {
            in_quotes = true;
            at_start = false;
        } else if d.is_delimiter(ch) {
            if d.consecutive_as_one {
                while chars.peek().is_some_and(|next| d.is_delimiter(*next)) {
                    chars.next();
                }
            }
            fields.push(std::mem::take(&mut field));
            at_start = true;
        } else {
            field.push(ch);
            at_start = false;
        }
    }
    fields.push(field);
    fields
}

fn split_fixed(line: &str, breaks: &[usize]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut fields = Vec::with_capacity(breaks.len() + 1);
    let mut start = 0usize;
    for &at in breaks.iter().chain(std::iter::once(&usize::MAX)) {
        let end = at.min(chars.len());
        let from = start.min(end);
        // The blanks that pad a column are not part of its text.
        let field: String = chars[from..end].iter().collect();
        fields.push(field.trim_end_matches(' ').to_string());
        start = at;
    }
    fields
}

/// The number of fields the widest of these lines gives.
pub fn field_count<S: AsRef<str>>(lines: &[S], layout: &Layout) -> usize {
    lines.iter().map(|line| split_line(line.as_ref(), layout).len()).max().unwrap_or(0)
}

/// Where the wizard proposes break lines for fixed-width text: at the
/// start of each run of characters that follows a column blank in every
/// line.
pub fn guess_breaks<S: AsRef<str>>(lines: &[S]) -> Vec<usize> {
    let lines: Vec<Vec<char>> = lines
        .iter()
        .map(|line| line.as_ref().chars().collect::<Vec<char>>())
        .filter(|line| !line.is_empty())
        .collect();
    let width = lines.iter().map(Vec::len).max().unwrap_or(0);
    let blank = |i: usize| lines.iter().all(|line| line.get(i).is_none_or(|ch| *ch == ' '));
    (1..width).filter(|&i| blank(i - 1) && !blank(i)).collect()
}

/// Whether text looks delimited (true) or laid out in columns of fixed
/// width (false): what the first step of the wizard preselects.
pub fn looks_delimited<S: AsRef<str>>(lines: &[S]) -> bool {
    let has = |ch: char| lines.iter().any(|line| line.as_ref().contains(ch));
    if has('\t') || has(';') || has(',') {
        return true;
    }
    // Columns padded with blanks: several spaces in a row somewhere, and
    // the same breaks on every line.
    let padded = lines.iter().any(|line| line.as_ref().trim().contains("  "));
    !(padded && !guess_breaks(lines).is_empty())
}

/// The delimiters the wizard ticks for this text: the tab, plus the
/// first of `;`, `,` and the space that every line has.
pub fn guess_delimiters<S: AsRef<str>>(lines: &[S]) -> Delimiters {
    let mut d = Delimiters::default();
    let lines: Vec<&str> = lines.iter().map(AsRef::as_ref).filter(|line| !line.is_empty()).collect();
    if lines.is_empty() || lines.iter().any(|line| line.contains('\t')) {
        return d;
    }
    let every = |ch: char| lines.iter().all(|line| line.contains(ch));
    if every(';') {
        d.semicolon = true;
    } else if every(',') {
        d.comma = true;
    } else if every(' ') {
        d.space = true;
    }
    d
}

/// Read a date written with its parts in a given order: three numbers
/// separated by `/`, `-`, `.` or blanks, or eight (six) digits in a row.
/// Years on two digits are in 1930–2029, as when typing.
pub fn parse_date(text: &str, order: DateOrder) -> Option<f64> {
    let text = text.trim();
    let tokens: Vec<&str> = text
        .split(|ch: char| matches!(ch, '/' | '-' | '.') || ch.is_whitespace())
        .filter(|token| !token.is_empty())
        .collect();
    let digits = |s: &str| !s.is_empty() && s.chars().all(|ch| ch.is_ascii_digit());
    let tokens: Vec<&str> = if tokens.len() == 1 && digits(tokens[0]) && matches!(tokens[0].len(), 6 | 8) {
        // "20240131": the year takes four digits when there are eight.
        let s = tokens[0];
        let year_len = s.len() - 4;
        let mut cuts = [2usize; 3];
        cuts[order.year_index()] = year_len;
        let (a, rest) = s.split_at(cuts[0]);
        let (b, c) = rest.split_at(cuts[1]);
        vec![a, b, c]
    } else {
        tokens
    };
    if tokens.len() != 3 || !tokens.iter().all(|token| digits(token) && token.len() <= 4) {
        return None;
    }
    let n = [tokens[0].parse().ok()?, tokens[1].parse().ok()?, tokens[2].parse().ok()?];
    let (mut year, month, day): (i64, i64, i64) = order.arrange(n);
    if tokens[order.year_index()].len() <= 2 {
        year += if year < 30 { 2000 } else { 1900 };
    }
    if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    if day > i64::from(days_in_month(year, month as u32)) {
        return None;
    }
    date_to_serial(year, month, day)
}

/// The text of a cell as the wizard sees it.
fn source_text(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    match wb.value_at(sheet, row, col) {
        Value::Text(text) => text.to_string(),
        Value::Empty => String::new(),
        _ => wb.display_text(sheet, row, col).text,
    }
}

impl Workbook {
    /// The lines Text to Columns would cut: the text of each cell of a
    /// one-column range, down to the last cell that holds something.
    pub fn text_column_lines(&self, sheet: usize, source: Rect) -> Result<Vec<String>, TextColumnsError> {
        if source.cols() != 1 {
            return Err(TextColumnsError::SeveralColumns);
        }
        let Some(sh) = self.sheets.get(sheet) else {
            return Err(TextColumnsError::NoData);
        };
        let last = sh
            .cells
            .range((source.r0, 0)..=(source.r1, u32::MAX))
            .filter(|((_, col), cell)| *col == source.c0 && !matches!(cell.content, Content::Empty))
            .map(|((row, _), _)| *row)
            .max()
            .ok_or(TextColumnsError::NoData)?;
        Ok((source.r0..=last).map(|row| source_text(self, sheet, row, source.c0)).collect())
    }

    /// Where `lines` cut by `spec` would land from `dest` (row, column)
    /// downwards: the rectangle covered, or why they cannot.
    pub fn text_columns_extent(&self, dest: (u32, u32), lines: &[String], spec: &Spec) -> Result<Rect, TextColumnsError> {
        let width = lines.iter().map(|line| spec.fields(line).len()).max().unwrap_or(0).max(1) as u32;
        let height = lines.len().max(1) as u32;
        if dest.0 + height > MAX_ROWS || dest.1 + width > MAX_COLS {
            return Err(TextColumnsError::OffSheet);
        }
        Ok(Rect::new(dest.0, dest.1, dest.0 + height - 1, dest.1 + width - 1))
    }

    /// True when writing `lines` from `dest` would replace data, the
    /// cells of `source` aside (the column being converted is expected to
    /// be overwritten): the case of Excel's "There's already data here".
    pub fn text_columns_overwrites(&self, sheet: usize, source: Option<Rect>, dest: (u32, u32), lines: &[String], spec: &Spec) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        lines.iter().enumerate().any(|(i, line)| {
            let row = dest.0 + i as u32;
            (0..spec.fields(line).len() as u32).any(|j| {
                let col = dest.1 + j;
                !source.is_some_and(|s| s.contains(row, col)) && !sh.is_blank(row, col)
            })
        })
    }

    /// Write `lines`, cut by `spec`, one per row from `dest`. The caller
    /// commits. Returns the rectangle written.
    pub fn write_text_columns(&mut self, sheet: usize, dest: (u32, u32), lines: &[String], spec: &Spec) -> Result<Rect, TextColumnsError> {
        let extent = self.text_columns_extent(dest, lines, spec)?;
        let date_format: Arc<str> = Arc::from(self.locale.short_date_format);
        for (i, line) in lines.iter().enumerate() {
            let row = dest.0 + i as u32;
            for (j, (field, format)) in spec.fields(line).into_iter().enumerate() {
                let col = dest.1 + j as u32;
                match format {
                    ColumnFormat::Text => {
                        self.update_style(sheet, Rect::cell(row, col), &|s| s.num_fmt = Arc::from("@"));
                        let content = if field.is_empty() { Content::Empty } else { Content::Value(Value::text(&field)) };
                        self.set_content(sheet, row, col, content);
                    }
                    ColumnFormat::Date(order) => match parse_date(&field, order) {
                        Some(serial) => {
                            if self.style_at(sheet, row, col).is_general() {
                                let fmt = date_format.clone();
                                self.update_style(sheet, Rect::cell(row, col), &move |s| s.num_fmt = fmt.clone());
                            }
                            self.set_content(sheet, row, col, Content::Value(Value::Number(serial)));
                        }
                        // What is not a date in that order stays as it is.
                        None => self.enter_general(sheet, row, col, &field),
                    },
                    ColumnFormat::General | ColumnFormat::Skip => self.enter_general(sheet, row, col, &field),
                }
            }
        }
        Ok(extent)
    }

    /// Store a field the way typing it would; what cannot be read as a
    /// formula stays text.
    fn enter_general(&mut self, sheet: usize, row: u32, col: u32, field: &str) {
        if self.set_cell_text(sheet, row, col, field).is_err() {
            self.set_content(sheet, row, col, Content::Value(Value::text(field)));
        }
    }

    /// Convert Text to Columns: cut the text of the one-column range
    /// `source` and write the pieces from `dest`. The caller commits.
    pub fn text_to_columns(&mut self, sheet: usize, source: Rect, dest: (u32, u32), spec: &Spec) -> Result<Rect, TextColumnsError> {
        let lines = self.text_column_lines(sheet, source)?;
        self.text_columns_extent(dest, &lines, spec)?;
        // The column converted elsewhere keeps its text, as in Excel.
        self.write_text_columns(sheet, dest, &lines, spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::tests::{val, wb};

    fn delimited(line: &str, d: &Delimiters) -> Vec<String> {
        split_line(line, &Layout::Delimited(d.clone()))
    }

    #[test]
    fn delimiters_cut_fields() {
        let comma = Delimiters::only(',');
        assert_eq!(delimited("a,b,,c", &comma), ["a", "b", "", "c"]);
        assert_eq!(delimited("", &comma), [""]);
        assert_eq!(delimited("a,", &comma), ["a", ""]);
        // Several kinds at once.
        let mut both = Delimiters::only(';');
        both.space = true;
        assert_eq!(delimited("a b;c", &both), ["a", "b", "c"]);
        // Tab is the default; other characters are data.
        assert_eq!(delimited("a\tb,c", &Delimiters::default()), ["a", "b,c"]);
        assert_eq!(delimited("a|b|c", &Delimiters::only('|')), ["a", "b", "c"]);
    }

    #[test]
    fn consecutive_delimiters_as_one() {
        let mut space = Delimiters::only(' ');
        assert_eq!(delimited("a  b   c", &space), ["a", "", "b", "", "", "c"]);
        space.consecutive_as_one = true;
        assert_eq!(delimited("a  b   c", &space), ["a", "b", "c"]);
        // A leading run still gives an empty first field, as in Excel.
        assert_eq!(delimited("  a", &space), ["", "a"]);
    }

    #[test]
    fn text_qualifier() {
        let comma = Delimiters::only(',');
        assert_eq!(delimited("\"Dupont, Jean\",42", &comma), ["Dupont, Jean", "42"]);
        assert_eq!(delimited("\"say \"\"hi\"\"\",x", &comma), ["say \"hi\"", "x"]);
        // A quote inside a field is data.
        assert_eq!(delimited("5\" disk,x", &comma), ["5\" disk", "x"]);
        // Text after the closing qualifier stays in the field.
        assert_eq!(delimited("\"a\"b,c", &comma), ["ab", "c"]);
        let mut single = Delimiters::only(',');
        single.qualifier = Some('\'');
        assert_eq!(delimited("'a,b',\"c", &single), ["a,b", "\"c"]);
        let mut none = Delimiters::only(',');
        none.qualifier = None;
        assert_eq!(delimited("\"a,b\"", &none), ["\"a", "b\""]);
    }

    #[test]
    fn fixed_width() {
        let layout = Layout::FixedWidth(vec![5, 9]);
        assert_eq!(split_line("Jean Dup 12", &layout), ["Jean", "Dup", "12"]);
        // A short line has empty fields at the end.
        assert_eq!(split_line("Jean", &layout), ["Jean", "", ""]);
        assert_eq!(split_line("Éric Zoé 7", &layout), ["Éric", "Zoé", "7"]);
        assert_eq!(split_line("abc", &Layout::FixedWidth(Vec::new())), ["abc"]);
    }

    #[test]
    fn guesses() {
        let lines = ["Jean     12  Paris", "Martine   7  Lyon", "Zoé     100  Nice"];
        assert_eq!(guess_breaks(&lines), vec![8, 13]);
        assert!(!looks_delimited(&lines));
        assert!(looks_delimited(&["a;b", "c;d"]));
        assert!(looks_delimited(&["Jean Dupont", "Marie Curie"]));
        assert_eq!(guess_breaks(&["abc", "de"]), Vec::<usize>::new());
        assert!(guess_delimiters(&["a;b", "c;d"]).semicolon);
        assert!(guess_delimiters(&["a,b", "c,d"]).comma);
        assert!(guess_delimiters(&["Jean Dupont", "Marie Curie"]).space);
        let tab = guess_delimiters(&["a\tb c", "d\te"]);
        assert!(tab.tab && !tab.space);
        assert_eq!(field_count(&["a;b", "c;d;e"], &Layout::Delimited(Delimiters::only(';'))), 3);
    }

    #[test]
    fn dates_in_every_order() {
        let serial = |y, m, d| date_to_serial(y, m, d);
        assert_eq!(parse_date("31/01/2024", DateOrder::DMY), serial(2024, 1, 31));
        assert_eq!(parse_date("01/31/2024", DateOrder::MDY), serial(2024, 1, 31));
        assert_eq!(parse_date("2024-01-31", DateOrder::YMD), serial(2024, 1, 31));
        assert_eq!(parse_date("01.2024.31", DateOrder::MYD), serial(2024, 1, 31));
        assert_eq!(parse_date("31 2024 01", DateOrder::DYM), serial(2024, 1, 31));
        assert_eq!(parse_date("2024/31/01", DateOrder::YDM), serial(2024, 1, 31));
        assert_eq!(parse_date("20240131", DateOrder::YMD), serial(2024, 1, 31));
        assert_eq!(parse_date("31012024", DateOrder::DMY), serial(2024, 1, 31));
        assert_eq!(parse_date("310124", DateOrder::DMY), serial(2024, 1, 31));
        assert_eq!(parse_date("1/2/99", DateOrder::DMY), serial(1999, 2, 1));
        assert_eq!(parse_date("31/01/2024", DateOrder::MDY), None);
        assert_eq!(parse_date("30/02/2024", DateOrder::DMY), None);
        assert_eq!(parse_date("hello", DateOrder::DMY), None);
        assert_eq!(parse_date("1/2", DateOrder::DMY), None);
    }

    #[test]
    fn converts_a_column_in_place() {
        let mut w = wb(&[("A1", "Jean,Dupont,42"), ("A2", "Marie,Curie,1.5"), ("A4", "Zoé")]);
        let spec = Spec::new(Layout::Delimited(Delimiters::only(',')));
        let source = Rect::new(0, 0, 9, 0);
        let lines = w.text_column_lines(0, source).unwrap();
        assert_eq!(lines.len(), 4);
        assert!(!w.text_columns_overwrites(0, Some(source), (0, 0), &lines, &spec));
        let written = w.text_to_columns(0, source, (0, 0), &spec).unwrap();
        w.commit("Text to Columns");
        assert_eq!(written, Rect::new(0, 0, 3, 2));
        assert_eq!(val(&w, "A1"), Value::text("Jean"));
        assert_eq!(val(&w, "B1"), Value::text("Dupont"));
        assert_eq!(val(&w, "C1"), Value::Number(42.0));
        assert_eq!(val(&w, "C2"), Value::Number(1.5));
        assert_eq!(val(&w, "A4"), Value::text("Zoé"));
        assert_eq!(val(&w, "A3"), Value::Empty);
        // One step back.
        w.undo();
        assert_eq!(val(&w, "A1"), Value::text("Jean,Dupont,42"));
        assert_eq!(val(&w, "B1"), Value::Empty);
    }

    #[test]
    fn column_formats_and_destination() {
        let mut w = wb(&[("A1", "007;31/01/2024;x;1,5"), ("D1", "busy")]);
        let mut spec = Spec::new(Layout::Delimited(Delimiters::only(';')));
        spec.formats = vec![ColumnFormat::Text, ColumnFormat::Date(DateOrder::DMY), ColumnFormat::Skip];
        let source = Rect::cell(0, 0);
        let lines = w.text_column_lines(0, source).unwrap();
        // C1:E1 would be written: D1 is in the way.
        assert!(w.text_columns_overwrites(0, Some(source), (0, 2), &lines, &spec));
        assert!(!w.text_columns_overwrites(0, Some(source), (1, 0), &lines, &spec));
        w.text_to_columns(0, source, (1, 0), &spec).unwrap();
        w.commit("Text to Columns");
        // The source stays when the result goes elsewhere.
        assert_eq!(val(&w, "A1"), Value::text("007;31/01/2024;x;1,5"));
        assert_eq!(val(&w, "A2"), Value::text("007"));
        assert_eq!(w.style_at(0, 1, 0).num_fmt.as_ref(), "@");
        assert_eq!(Some(val(&w, "B2")), date_to_serial(2024, 1, 31).map(Value::Number));
        assert_eq!(w.style_at(0, 1, 1).num_fmt.as_ref(), "m/d/yyyy");
        // The skipped column leaves no gap; "1,5" is 15 when typed in English.
        assert_eq!(val(&w, "C2"), Value::Number(15.0));
        assert_eq!(val(&w, "D2"), Value::Empty);
    }

    #[test]
    fn general_follows_the_locale() {
        let mut w = Workbook::new(Locale::fr());
        w.set_cell_text(0, 0, 0, "'1,5 31/01/2024 VRAI =1+1").unwrap();
        w.commit("setup");
        let spec = Spec::new(Layout::Delimited(Delimiters::only(' ')));
        w.text_to_columns(0, Rect::cell(0, 0), (0, 1), &spec).unwrap();
        w.commit("Text to Columns");
        assert_eq!(w.value_at(0, 0, 1), Value::Number(1.5));
        assert_eq!(Some(w.value_at(0, 0, 2)), date_to_serial(2024, 1, 31).map(Value::Number));
        assert_eq!(w.value_at(0, 0, 3), Value::Bool(true));
        assert_eq!(w.value_at(0, 0, 4), Value::Number(2.0));
    }

    #[test]
    fn refusals() {
        let mut w = wb(&[("A1", "a b"), ("B1", "c")]);
        let spec = Spec::new(Layout::Delimited(Delimiters::only(' ')));
        assert_eq!(w.text_column_lines(0, Rect::new(0, 0, 0, 1)), Err(TextColumnsError::SeveralColumns));
        assert_eq!(w.text_column_lines(0, Rect::cell(5, 5)), Err(TextColumnsError::NoData));
        assert_eq!(
            w.text_to_columns(0, Rect::cell(0, 0), (0, MAX_COLS - 1), &spec),
            Err(TextColumnsError::OffSheet)
        );
        // Numbers are cut as they show.
        w.set_cell_text(0, 2, 0, "12.5").unwrap();
        w.commit("setup");
        assert_eq!(w.text_column_lines(0, Rect::cell(2, 0)).unwrap(), ["12.5"]);
    }
}
