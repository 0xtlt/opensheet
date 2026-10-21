//! Splitting text into columns: the logic behind Excel's Convert Text to
//! Columns Wizard, also used to import a text file. An import is
//! remembered on its sheet (`Sheet::text_imports`, the text connection and
//! query table of a file) so that it can be refreshed from the file.
//!
//! A [`Spec`] says how a line is cut ([`Layout`]: at delimiters or at fixed
//! positions) and what becomes of each piece ([`ColumnFormat`]). Pieces
//! kept as "General" are stored the way typing them into a cell would, in
//! the locale of the workbook.

use crate::ast::StructuralEdit;
use crate::datetime::{date_to_serial, days_in_month};
use crate::filter::AutoFilter;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Content, Sheet, Workbook};
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
    lines
        .iter()
        .map(|line| split_line(line.as_ref(), layout).len())
        .max()
        .unwrap_or(0)
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
    let blank = |i: usize| {
        lines
            .iter()
            .all(|line| line.get(i).is_none_or(|ch| *ch == ' '))
    };
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

/// The delimiters the wizard ticks for this text: the tab, plus `;` or
/// `,` when most lines have it (a title line may not), else the space
/// when every line has one.
pub fn guess_delimiters<S: AsRef<str>>(lines: &[S]) -> Delimiters {
    let mut d = Delimiters::default();
    let lines: Vec<&str> = lines
        .iter()
        .map(AsRef::as_ref)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() || lines.iter().any(|line| line.contains('\t')) {
        return d;
    }
    let with = |ch: char| lines.iter().filter(|line| line.contains(ch)).count();
    let (semicolons, commas) = (with(';'), with(','));
    if semicolons * 2 > lines.len() && semicolons >= commas {
        d.semicolon = true;
    } else if commas * 2 > lines.len() {
        d.comma = true;
    } else if with(' ') == lines.len() {
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
    let tokens: Vec<&str> =
        if tokens.len() == 1 && digits(tokens[0]) && matches!(tokens[0].len(), 6 | 8) {
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
    let n = [
        tokens[0].parse().ok()?,
        tokens[1].parse().ok()?,
        tokens[2].parse().ok()?,
    ];
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
    pub fn text_column_lines(
        &self,
        sheet: usize,
        source: Rect,
    ) -> Result<Vec<String>, TextColumnsError> {
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
        Ok((source.r0..=last)
            .map(|row| source_text(self, sheet, row, source.c0))
            .collect())
    }

    /// Where `lines` cut by `spec` would land from `dest` (row, column)
    /// downwards: the rectangle covered, or why they cannot.
    pub fn text_columns_extent(
        &self,
        dest: (u32, u32),
        lines: &[String],
        spec: &Spec,
    ) -> Result<Rect, TextColumnsError> {
        let width = lines
            .iter()
            .map(|line| spec.fields(line).len())
            .max()
            .unwrap_or(0)
            .max(1) as u32;
        let height = lines.len().max(1) as u32;
        if dest.0 + height > MAX_ROWS || dest.1 + width > MAX_COLS {
            return Err(TextColumnsError::OffSheet);
        }
        Ok(Rect::new(
            dest.0,
            dest.1,
            dest.0 + height - 1,
            dest.1 + width - 1,
        ))
    }

    /// True when writing `lines` from `dest` would replace data, the
    /// cells of `source` aside (the column being converted is expected to
    /// be overwritten): the case of Excel's "There's already data here".
    pub fn text_columns_overwrites(
        &self,
        sheet: usize,
        source: Option<Rect>,
        dest: (u32, u32),
        lines: &[String],
        spec: &Spec,
    ) -> bool {
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
    pub fn write_text_columns(
        &mut self,
        sheet: usize,
        dest: (u32, u32),
        lines: &[String],
        spec: &Spec,
    ) -> Result<Rect, TextColumnsError> {
        let extent = self.text_columns_extent(dest, lines, spec)?;
        let date_format: Arc<str> = Arc::from(self.locale.short_date_format);
        for (i, line) in lines.iter().enumerate() {
            let row = dest.0 + i as u32;
            for (j, (field, format)) in spec.fields(line).into_iter().enumerate() {
                let col = dest.1 + j as u32;
                match format {
                    ColumnFormat::Text => {
                        self.update_style(sheet, Rect::cell(row, col), &|s| {
                            s.num_fmt = Arc::from("@")
                        });
                        let content = if field.is_empty() {
                            Content::Empty
                        } else {
                            Content::Value(Value::text(&field))
                        };
                        self.set_content(sheet, row, col, content);
                    }
                    ColumnFormat::Date(order) => match parse_date(&field, order) {
                        Some(serial) => {
                            if self.style_at(sheet, row, col).is_general() {
                                let fmt = date_format.clone();
                                self.update_style(sheet, Rect::cell(row, col), &move |s| {
                                    s.num_fmt = fmt.clone()
                                });
                            }
                            self.set_content(
                                sheet,
                                row,
                                col,
                                Content::Value(Value::Number(serial)),
                            );
                        }
                        // What is not a date in that order stays as it is.
                        None => self.enter_general(sheet, row, col, &field),
                    },
                    ColumnFormat::General | ColumnFormat::Skip => {
                        self.enter_general(sheet, row, col, &field)
                    }
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
    pub fn text_to_columns(
        &mut self,
        sheet: usize,
        source: Rect,
        dest: (u32, u32),
        spec: &Spec,
    ) -> Result<Rect, TextColumnsError> {
        let lines = self.text_column_lines(sheet, source)?;
        self.text_columns_extent(dest, &lines, spec)?;
        // The column converted elsewhere keeps its text, as in Excel.
        self.write_text_columns(sheet, dest, &lines, spec)
    }
}

/// A text file imported into a sheet (Data > Get Data > From Text/CSV):
/// where it came from, how it was cut and where it landed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextImport {
    /// The name of the query, also that of the range in a file.
    pub name: String,
    /// The path of the text file.
    pub path: String,
    /// The cells the data fills.
    pub range: Rect,
    /// The first line imported (1 is the first of the file).
    pub first_row: u32,
    pub spec: Spec,
    /// Set when the source is a document rather than cut text (a web
    /// page, JSON, XML, a workbook): `path` is then its address, and
    /// `first_row` and `spec` are not used. See `external.rs`.
    pub web: Option<crate::external::WebQuery>,
    pub props: crate::external::QueryProps,
}

/// The lines of a text file. A line break inside a quoted field does not
/// end the line.
pub fn text_lines(text: &str, qualifier: Option<char>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if Some(ch) == qualifier {
            quoted = !quoted;
            line.push(ch);
        } else if !quoted && (ch == '\n' || ch == '\r') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            lines.push(std::mem::take(&mut line));
        } else {
            line.push(ch);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

impl TextImport {
    /// The lines of the file this import takes.
    fn lines(&self, text: &str) -> Vec<String> {
        let qualifier = match &self.spec.layout {
            Layout::Delimited(delimiters) => delimiters.qualifier,
            Layout::FixedWidth(_) => None,
        };
        let skip = self.first_row.saturating_sub(1) as usize;
        text_lines(text, qualifier).into_iter().skip(skip).collect()
    }
}

impl Workbook {
    /// A name for the query of a file that no import of the workbook and
    /// no defined name has: the name of the file made a valid name, with
    /// a number when it is taken.
    pub fn text_import_name(&self, path: &str) -> String {
        // Of an address, the last piece of its path, or else its host.
        let path = match path.split_once("://") {
            Some((_, rest)) => rest
                .split(['?', '#'])
                .next()
                .unwrap_or(rest)
                .trim_end_matches('/'),
            None => path,
        };
        let stem = std::path::Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("data");
        let mut base: String = stem
            .chars()
            .map(|ch| {
                if ch.is_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        // A name starts with a letter and must not read as a cell.
        if !base.chars().next().is_some_and(char::is_alphabetic)
            || crate::refs::parse_cell_name(&base).is_some()
        {
            base.insert(0, '_');
        }
        let taken = |name: &str| {
            let key = name.to_uppercase();
            self.names.contains_key(&key)
                || self.sheets.iter().any(|sh| {
                    sh.names.contains_key(&key)
                        || sh
                            .text_imports
                            .iter()
                            .any(|import| import.name.to_uppercase() == key)
                })
        };
        let mut name = base.clone();
        let mut n = 1;
        while taken(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }

    /// Where the text of a file would land from `dest` with the settings
    /// of `import`.
    pub fn text_import_extent(
        &self,
        dest: (u32, u32),
        text: &str,
        import: &TextImport,
    ) -> Result<Rect, TextColumnsError> {
        self.text_columns_extent(dest, &import.lines(text), &import.spec)
    }

    /// Import the text of a file into `sheet` from `dest`, and remember
    /// the import there. `import.range` and an empty `import.name` are
    /// filled in. Returns the rectangle written. The caller commits.
    pub fn import_text(
        &mut self,
        sheet: usize,
        dest: (u32, u32),
        text: &str,
        mut import: TextImport,
    ) -> Result<Rect, TextColumnsError> {
        if sheet >= self.sheets.len() {
            return Err(TextColumnsError::NoData);
        }
        let lines = import.lines(text);
        if lines.is_empty() {
            return Err(TextColumnsError::NoData);
        }
        let extent = self.write_text_columns(sheet, dest, &lines, &import.spec)?;
        if import.name.is_empty() {
            import.name = self.text_import_name(&import.path);
        }
        import.range = extent;
        self.snapshot();
        self.sheets[sheet].text_imports.push(import);
        Ok(extent)
    }

    /// Refresh an import with the text its file holds now: the old data
    /// is cleared and the new written from the same corner. Returns the
    /// rectangle written. The caller commits.
    pub fn refresh_text_import(
        &mut self,
        sheet: usize,
        index: usize,
        text: &str,
    ) -> Result<Rect, TextColumnsError> {
        let import = self
            .sheets
            .get(sheet)
            .and_then(|sh| sh.text_imports.get(index))
            .cloned()
            .ok_or(TextColumnsError::NoData)?;
        let lines = import.lines(text);
        let dest = (import.range.r0, import.range.c0);
        let extent = self.text_columns_extent(dest, &lines, &import.spec)?;
        self.clear_import(sheet, &import);
        self.write_text_columns(sheet, dest, &lines, &import.spec)?;
        self.snapshot();
        self.sheets[sheet].text_imports[index].range = extent;
        Ok(extent)
    }

    /// The import whose data a cell belongs to: its index on the sheet.
    pub fn text_import_at(&self, sheet: usize, row: u32, col: u32) -> Option<usize> {
        self.sheets
            .get(sheet)?
            .text_imports
            .iter()
            .position(|import| import.range.contains(row, col))
    }
}

/// See `features::on_structural_edit`: the range of an import follows
/// the rows and columns of its sheet, and an import whose first row or
/// whose columns are deleted is forgotten.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.name != edited_sheet {
        return;
    }
    sheet.text_imports.retain_mut(|import| {
        let mut range = AutoFilter::new(import.range);
        let kept = range.adjust(edit);
        import.range = range.range;
        kept
    });
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
        assert_eq!(
            delimited("\"Dupont, Jean\",42", &comma),
            ["Dupont, Jean", "42"]
        );
        assert_eq!(
            delimited("\"say \"\"hi\"\"\",x", &comma),
            ["say \"hi\"", "x"]
        );
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
        let lines = [
            "Jean     12  Paris",
            "Martine   7  Lyon",
            "Zoé     100  Nice",
        ];
        assert_eq!(guess_breaks(&lines), vec![8, 13]);
        assert!(!looks_delimited(&lines));
        assert!(looks_delimited(&["a;b", "c;d"]));
        assert!(looks_delimited(&["Jean Dupont", "Marie Curie"]));
        assert_eq!(guess_breaks(&["abc", "de"]), Vec::<usize>::new());
        assert!(guess_delimiters(&["a;b", "c;d"]).semicolon);
        assert!(guess_delimiters(&["a,b", "c,d"]).comma);
        assert!(guess_delimiters(&["Jean Dupont", "Marie Curie"]).space);
        assert!(guess_delimiters(&["Sales 2024", "a;b", "c;d"]).semicolon);
        assert!(guess_delimiters(&["a;b,5", "c;d,5"]).semicolon);
        let tab = guess_delimiters(&["a\tb c", "d\te"]);
        assert!(tab.tab && !tab.space);
        assert_eq!(
            field_count(&["a;b", "c;d;e"], &Layout::Delimited(Delimiters::only(';'))),
            3
        );
    }

    #[test]
    fn dates_in_every_order() {
        let serial = |y, m, d| date_to_serial(y, m, d);
        assert_eq!(
            parse_date("31/01/2024", DateOrder::DMY),
            serial(2024, 1, 31)
        );
        assert_eq!(
            parse_date("01/31/2024", DateOrder::MDY),
            serial(2024, 1, 31)
        );
        assert_eq!(
            parse_date("2024-01-31", DateOrder::YMD),
            serial(2024, 1, 31)
        );
        assert_eq!(
            parse_date("01.2024.31", DateOrder::MYD),
            serial(2024, 1, 31)
        );
        assert_eq!(
            parse_date("31 2024 01", DateOrder::DYM),
            serial(2024, 1, 31)
        );
        assert_eq!(
            parse_date("2024/31/01", DateOrder::YDM),
            serial(2024, 1, 31)
        );
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
        let mut w = wb(&[
            ("A1", "Jean,Dupont,42"),
            ("A2", "Marie,Curie,1.5"),
            ("A4", "Zoé"),
        ]);
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
        spec.formats = vec![
            ColumnFormat::Text,
            ColumnFormat::Date(DateOrder::DMY),
            ColumnFormat::Skip,
        ];
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
        assert_eq!(
            Some(val(&w, "B2")),
            date_to_serial(2024, 1, 31).map(Value::Number)
        );
        assert_eq!(w.style_at(0, 1, 1).num_fmt.as_ref(), "m/d/yyyy");
        // The skipped column leaves no gap; "1,5" is 15 when typed in English.
        assert_eq!(val(&w, "C2"), Value::Number(15.0));
        assert_eq!(val(&w, "D2"), Value::Empty);
    }

    #[test]
    fn general_follows_the_locale() {
        let mut w = Workbook::new(Locale::fr());
        w.set_cell_text(0, 0, 0, "'1,5 31/01/2024 VRAI =1+1")
            .unwrap();
        w.commit("setup");
        let spec = Spec::new(Layout::Delimited(Delimiters::only(' ')));
        w.text_to_columns(0, Rect::cell(0, 0), (0, 1), &spec)
            .unwrap();
        w.commit("Text to Columns");
        assert_eq!(w.value_at(0, 0, 1), Value::Number(1.5));
        assert_eq!(
            Some(w.value_at(0, 0, 2)),
            date_to_serial(2024, 1, 31).map(Value::Number)
        );
        assert_eq!(w.value_at(0, 0, 3), Value::Bool(true));
        assert_eq!(w.value_at(0, 0, 4), Value::Number(2.0));
    }

    #[test]
    fn refusals() {
        let mut w = wb(&[("A1", "a b"), ("B1", "c")]);
        let spec = Spec::new(Layout::Delimited(Delimiters::only(' ')));
        assert_eq!(
            w.text_column_lines(0, Rect::new(0, 0, 0, 1)),
            Err(TextColumnsError::SeveralColumns)
        );
        assert_eq!(
            w.text_column_lines(0, Rect::cell(5, 5)),
            Err(TextColumnsError::NoData)
        );
        assert_eq!(
            w.text_to_columns(0, Rect::cell(0, 0), (0, MAX_COLS - 1), &spec),
            Err(TextColumnsError::OffSheet)
        );
        // Numbers are cut as they show.
        w.set_cell_text(0, 2, 0, "12.5").unwrap();
        w.commit("setup");
        assert_eq!(w.text_column_lines(0, Rect::cell(2, 0)).unwrap(), ["12.5"]);
    }

    fn csv_import(path: &str) -> TextImport {
        TextImport {
            name: String::new(),
            path: path.to_string(),
            range: Rect::cell(0, 0),
            first_row: 1,
            spec: Spec::new(Layout::Delimited(Delimiters::only(','))),
            web: None,
            props: Default::default(),
        }
    }

    #[test]
    fn lines_of_a_file() {
        assert_eq!(
            text_lines("a\nb\r\nc\rd\n", Some('"')),
            ["a", "b", "c", "d"]
        );
        assert_eq!(
            text_lines("a,\"x\ny\",b\nc", Some('"')),
            ["a,\"x\ny\",b", "c"]
        );
        assert_eq!(text_lines("a,\"x\ny", None), ["a,\"x", "y"]);
        assert_eq!(text_lines("a\n\nb", None), ["a", "", "b"]);
        assert!(text_lines("", None).is_empty());
    }

    #[test]
    fn import_and_refresh() {
        let mut w = wb(&[("E1", "keep")]);
        let mut import = csv_import("/tmp/sales 2024.csv");
        import.first_row = 2;
        let extent = w
            .import_text(0, (1, 1), "skipped\nName,Qty\nAnn,3\nBob,4\n", import)
            .unwrap();
        w.commit("Import Data");
        assert_eq!(extent, Rect::parse("B2:C4").unwrap());
        assert_eq!(val(&w, "B3"), Value::text("Ann"));
        assert_eq!(val(&w, "C4"), Value::Number(4.0));
        let kept = w.sheets[0].text_imports[0].clone();
        assert_eq!((kept.name.as_str(), kept.range), ("sales_2024", extent));
        assert_eq!(w.text_import_at(0, 2, 2), Some(0));
        assert_eq!(w.text_import_at(0, 0, 0), None);
        // The file changed: fewer rows, one more column.
        let extent = w
            .refresh_text_import(0, 0, "skipped\nName,Qty,City\nCid,5,Paris\n")
            .unwrap();
        w.commit("Refresh All");
        assert_eq!(extent, Rect::parse("B2:D3").unwrap());
        assert_eq!(val(&w, "B3"), Value::text("Cid"));
        assert_eq!(val(&w, "D3"), Value::text("Paris"));
        assert_eq!(val(&w, "B4"), Value::Empty);
        assert_eq!(val(&w, "E1"), Value::text("keep"));
        assert_eq!(w.sheets[0].text_imports[0].range, extent);
        // One step back each.
        w.undo();
        assert_eq!(val(&w, "B4"), Value::text("Bob"));
        assert_eq!(
            w.sheets[0].text_imports[0].range,
            Rect::parse("B2:C4").unwrap()
        );
        w.undo();
        assert!(w.sheets[0].text_imports.is_empty());
        assert_eq!(val(&w, "B3"), Value::Empty);
    }

    #[test]
    fn import_names_and_refusals() {
        let mut w = wb(&[]);
        w.import_text(0, (0, 0), "a,b", csv_import("/x/data.csv"))
            .unwrap();
        w.import_text(0, (5, 0), "a,b", csv_import("/y/data.csv"))
            .unwrap();
        w.import_text(0, (9, 0), "a,b", csv_import("/y/2024 q1.txt"))
            .unwrap();
        w.import_text(0, (12, 0), "a,b", csv_import("/y/A1.csv"))
            .unwrap();
        w.commit("Import Data");
        let names: Vec<&str> = w.sheets[0]
            .text_imports
            .iter()
            .map(|import| import.name.as_str())
            .collect();
        assert_eq!(names, ["data", "data_1", "_2024_q1", "_A1"]);
        assert_eq!(
            w.import_text(0, (0, 0), "", csv_import("/x/empty.csv")),
            Err(TextColumnsError::NoData)
        );
        assert_eq!(
            w.import_text(0, (0, MAX_COLS - 1), "a,b", csv_import("/x/wide.csv")),
            Err(TextColumnsError::OffSheet)
        );
        assert_eq!(
            w.refresh_text_import(0, 9, "a"),
            Err(TextColumnsError::NoData)
        );
        // The ranges follow the rows; an import whose first row goes is forgotten.
        w.insert_rows(0, 0, 1);
        w.delete_rows(0, 6, 1);
        w.commit("rows");
        let ranges: Vec<Rect> = w.sheets[0]
            .text_imports
            .iter()
            .map(|import| import.range)
            .collect();
        assert_eq!(
            ranges,
            [
                Rect::parse("A2:B2").unwrap(),
                Rect::parse("A10:B10").unwrap(),
                Rect::parse("A13:B13").unwrap()
            ]
        );
    }
}
