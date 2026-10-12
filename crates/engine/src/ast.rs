//! Formula syntax tree, printing, and reference rewriting.

use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, col_to_letters};
use crate::value::{ErrorKind, Value};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Plus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Pow => "^",
            BinaryOp::Concat => "&",
            BinaryOp::Eq => "=",
            BinaryOp::Ne => "<>",
            BinaryOp::Lt => "<",
            BinaryOp::Gt => ">",
            BinaryOp::Le => "<=",
            BinaryOp::Ge => ">=",
        }
    }
}

/// One corner of a reference as written in a formula.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
    pub abs_row: bool,
    pub abs_col: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `A1` or `A1:B2`
    Cells,
    /// `A:C` (rows span the whole sheet)
    Cols,
    /// `1:3` (columns span the whole sheet)
    Rows,
}

/// What `RefExpr::sheet` holds once the sheet was deleted: the reference
/// reads `#REF!A1` and evaluates to `#REF!`. Square brackets cannot appear
/// in a sheet name, so no real sheet is ever called this.
pub const DELETED_SHEET: &str = "[#REF!]";

/// A cell / range reference as written in a formula.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RefExpr {
    /// Sheet name as written (`None` = the formula's own sheet,
    /// `DELETED_SHEET` = a sheet that no longer exists).
    pub sheet: Option<Arc<str>>,
    pub start: CellRef,
    /// `None` for a single cell.
    pub end: Option<CellRef>,
    pub kind: RefKind,
}

impl RefExpr {
    pub fn cell(row: u32, col: u32) -> RefExpr {
        RefExpr {
            sheet: None,
            start: CellRef {
                row,
                col,
                abs_row: false,
                abs_col: false,
            },
            end: None,
            kind: RefKind::Cells,
        }
    }

    pub fn range(rect: Rect) -> RefExpr {
        if rect.is_single() {
            return RefExpr::cell(rect.r0, rect.c0);
        }
        let kind = if rect.is_whole_cols() && !rect.is_whole_rows() {
            RefKind::Cols
        } else if rect.is_whole_rows() && !rect.is_whole_cols() {
            RefKind::Rows
        } else {
            RefKind::Cells
        };
        RefExpr {
            sheet: None,
            start: CellRef {
                row: rect.r0,
                col: rect.c0,
                abs_row: false,
                abs_col: false,
            },
            end: Some(CellRef {
                row: rect.r1,
                col: rect.c1,
                abs_row: false,
                abs_col: false,
            }),
            kind,
        }
    }

    /// The rectangle this reference covers (normalized).
    pub fn rect(&self) -> Rect {
        let end = self.end.unwrap_or(self.start);
        match self.kind {
            RefKind::Cells => Rect::new(self.start.row, self.start.col, end.row, end.col),
            RefKind::Cols => Rect::new(0, self.start.col, MAX_ROWS - 1, end.col),
            RefKind::Rows => Rect::new(self.start.row, 0, end.row, MAX_COLS - 1),
        }
    }

    /// True when the sheet this reference pointed to was deleted
    /// (`#REF!A1`).
    pub fn sheet_deleted(&self) -> bool {
        self.sheet.as_deref() == Some(DELETED_SHEET)
    }

    pub fn write(&self, out: &mut String) {
        if self.sheet_deleted() {
            out.push_str(ErrorKind::Ref.as_str());
        } else if let Some(sheet) = &self.sheet {
            write_sheet_name(sheet, out);
            out.push('!');
        }
        self.write_cells(out);
    }

    /// The cells without the sheet name: `A1`, `$A$1:B2`, `A:C`, `1:3`.
    pub fn write_cells(&self, out: &mut String) {
        let write_col = |c: &CellRef, out: &mut String| {
            if c.abs_col {
                out.push('$');
            }
            out.push_str(&col_to_letters(c.col));
        };
        let write_row = |c: &CellRef, out: &mut String| {
            if c.abs_row {
                out.push('$');
            }
            out.push_str(&(c.row + 1).to_string());
        };
        match self.kind {
            RefKind::Cells => {
                write_col(&self.start, out);
                write_row(&self.start, out);
                if let Some(end) = &self.end {
                    out.push(':');
                    write_col(end, out);
                    write_row(end, out);
                }
            }
            RefKind::Cols => {
                let end = self.end.unwrap_or(self.start);
                write_col(&self.start, out);
                out.push(':');
                write_col(&end, out);
            }
            RefKind::Rows => {
                let end = self.end.unwrap_or(self.start);
                write_row(&self.start, out);
                out.push(':');
                write_row(&end, out);
            }
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }
}

/// True when a sheet name can be written without quotes: only letters,
/// digits, `_`, `.`, not starting with a digit and not looking like a cell
/// reference.
fn is_simple_sheet_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.chars().next().unwrap().is_ascii_digit()
        && crate::refs::parse_cell_name(name).is_none()
}

/// Quote a sheet name when it contains anything but letters, digits, `_`, `.`
/// or when it could be mistaken for a cell reference.
pub fn write_sheet_name(name: &str, out: &mut String) {
    if is_simple_sheet_name(name) {
        out.push_str(name);
    } else {
        out.push('\'');
        out.push_str(&name.replace('\'', "''"));
        out.push('\'');
    }
}

/// Sheet, table and column names are compared without case.
pub fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

/// `Sheet1:Sheet3!A1:B2`: the same cells on every sheet standing between
/// two sheets in tab order, both included. Inserting a sheet between the
/// two adds it to the reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ref3D {
    pub first: Arc<str>,
    pub last: Arc<str>,
    /// The cells, the same on every sheet (`sheet` is always `None`).
    pub cells: RefExpr,
}

impl Ref3D {
    /// A span is quoted as a whole: `'Jan 2024:Mar 2024'!A1`.
    pub fn write(&self, out: &mut String) {
        if is_simple_sheet_name(&self.first) && is_simple_sheet_name(&self.last) {
            out.push_str(&self.first);
            out.push(':');
            out.push_str(&self.last);
        } else {
            out.push('\'');
            out.push_str(&self.first.replace('\'', "''"));
            out.push(':');
            out.push_str(&self.last.replace('\'', "''"));
            out.push('\'');
        }
        out.push('!');
        self.cells.write_cells(out);
    }
}

/// The `#` item specifiers of a structured reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableItem {
    All,
    Data,
    Headers,
    Totals,
    ThisRow,
}

impl TableItem {
    /// The order of `Locale::table_items`.
    pub const ALL: [TableItem; 5] = [
        TableItem::All,
        TableItem::Data,
        TableItem::Headers,
        TableItem::Totals,
        TableItem::ThisRow,
    ];

    /// Canonical (English) spelling, as stored in files.
    pub fn as_str(self) -> &'static str {
        Locale::en().table_items[self as usize]
    }
}

/// The rows of the table a structured reference covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableRows {
    /// No item specifier (`Table1[Col]`): the data rows.
    Unspecified,
    /// `[#All]`: headers, data and totals.
    All,
    /// `[#Data]`
    Data,
    /// `[#Headers]`
    Headers,
    /// `[#Totals]`
    Totals,
    /// `[[#Headers],[#Data]]`
    HeadersData,
    /// `[[#Data],[#Totals]]`
    DataTotals,
    /// `[#This Row]`, shown as `@`: the row of the formula.
    ThisRow,
}

impl TableRows {
    /// The item specifiers that spell these rows.
    pub fn items(self) -> &'static [TableItem] {
        match self {
            TableRows::Unspecified => &[],
            TableRows::All => &[TableItem::All],
            TableRows::Data => &[TableItem::Data],
            TableRows::Headers => &[TableItem::Headers],
            TableRows::Totals => &[TableItem::Totals],
            TableRows::HeadersData => &[TableItem::Headers, TableItem::Data],
            TableRows::DataTotals => &[TableItem::Data, TableItem::Totals],
            TableRows::ThisRow => &[TableItem::ThisRow],
        }
    }

    /// The rows spelled by a list of item specifiers; `None` when Excel
    /// refuses the combination.
    pub fn from_items(items: &[TableItem]) -> Option<TableRows> {
        use TableItem::*;
        Some(match items {
            [] => TableRows::Unspecified,
            [All] => TableRows::All,
            [Data] => TableRows::Data,
            [Headers] => TableRows::Headers,
            [Totals] => TableRows::Totals,
            [ThisRow] => TableRows::ThisRow,
            [Headers, Data] | [Data, Headers] => TableRows::HeadersData,
            [Data, Totals] | [Totals, Data] => TableRows::DataTotals,
            _ => return None,
        })
    }
}

/// The columns of the table a structured reference covers. Column names
/// are kept as written and compared without case.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TableCols {
    /// No column specifier: every column.
    All,
    /// `[Col]`
    One(Arc<str>),
    /// `[[Col1]:[Col2]]`
    Range(Arc<str>, Arc<str>),
}

/// A structured reference to (part of) a table: `Table1[Col]`,
/// `Table1[[#Headers],[Col1]:[Col2]]`, `[@Col]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TableRef {
    /// Table name. `None` for the forms without one (`[Col]`, `[@Col]`),
    /// which mean the table holding the formula.
    pub table: Option<Arc<str>>,
    pub rows: TableRows,
    pub cols: TableCols,
}

/// Characters that force a column name into brackets of its own
/// (`Table1[[Total $]]`), as listed by Excel's documentation. Leading and
/// trailing spaces are bracketed too so they survive.
fn column_needs_brackets(name: &str) -> bool {
    name.starts_with(' ')
        || name.ends_with(' ')
        || name.chars().any(|c| {
            matches!(
                c,
                '\t' | '\n'
                    | '\r'
                    | ','
                    | ':'
                    | '.'
                    | '['
                    | ']'
                    | '#'
                    | '\''
                    | '"'
                    | '{'
                    | '}'
                    | '$'
                    | '^'
                    | '&'
                    | '*'
                    | '+'
                    | '='
                    | '-'
                    | '>'
                    | '<'
                    | '/'
                    | '@'
                    | '\\'
                    | '!'
                    | '('
                    | ')'
                    | '%'
                    | '?'
                    | '`'
                    | ';'
                    | '~'
                    | '_'
            )
        })
}

/// A column name with `'` in front of the characters that would otherwise
/// be read as syntax.
fn write_column_name(name: &str, out: &mut String) {
    for c in name.chars() {
        if matches!(c, '[' | ']' | '#' | '\'' | '@') {
            out.push('\'');
        }
        out.push(c);
    }
}

fn write_bracketed_column(name: &str, out: &mut String) {
    out.push('[');
    write_column_name(name, out);
    out.push(']');
}

impl TableRef {
    /// Print the reference. In the formula bar (`file` false) item
    /// specifiers and the separator follow the locale and `#This Row` is
    /// shortened to `@`; files hold the long English form.
    pub fn write(&self, locale: &Locale, file: bool, out: &mut String) {
        if let Some(table) = &self.table {
            out.push_str(table);
        }
        if self.rows == TableRows::ThisRow && !file {
            out.push_str("[@");
            match &self.cols {
                TableCols::All => {}
                TableCols::One(c) if !c.contains(' ') && !column_needs_brackets(c) => {
                    out.push_str(c)
                }
                TableCols::One(c) => write_bracketed_column(c, out),
                TableCols::Range(a, b) => {
                    write_bracketed_column(a, out);
                    out.push(':');
                    write_bracketed_column(b, out);
                }
            }
            out.push(']');
            return;
        }
        let item = |i: TableItem| {
            if file {
                i.as_str()
            } else {
                locale.table_items[i as usize]
            }
        };
        let items = self.rows.items();
        match (items, &self.cols) {
            ([], TableCols::All) => out.push_str("[]"),
            ([], TableCols::One(c)) if !column_needs_brackets(c) => write_bracketed_column(c, out),
            ([one], TableCols::All) => {
                out.push('[');
                out.push_str(item(*one));
                out.push(']');
            }
            _ => {
                let sep = if file { ',' } else { locale.arg_sep };
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(sep);
                    }
                    out.push('[');
                    out.push_str(item(*it));
                    out.push(']');
                }
                if !items.is_empty() && self.cols != TableCols::All {
                    out.push(sep);
                }
                match &self.cols {
                    TableCols::All => {}
                    TableCols::One(c) => write_bracketed_column(c, out),
                    TableCols::Range(a, b) => {
                        write_bracketed_column(a, out);
                        out.push(':');
                        write_bracketed_column(b, out);
                    }
                }
                out.push(']');
            }
        }
    }

    /// True when the reference is to `table`: by name, or, for the forms
    /// without a name, because the formula sits inside it (`inside`).
    pub fn is_to(&self, table: &str, inside: bool) -> bool {
        match &self.table {
            Some(name) => same_name(name, table),
            None => inside,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(Arc<str>),
    Bool(bool),
    Error(ErrorKind),
    /// An omitted function argument, as in `IF(A1,,2)`.
    Missing,
    /// A pre-evaluated value (never produced by the parser; used internally
    /// when lifting scalar functions over arrays).
    Value(Value),
    Ref(RefExpr),
    /// `Sheet1:Sheet3!A1`: the same cells on several sheets.
    Ref3D(Ref3D),
    /// `Sheet2!#REF!`: cells of the named sheet that were deleted.
    RefError(Arc<str>),
    /// A defined name, or the name of a table.
    Name(Arc<str>),
    /// `Sheet1!Name`: a defined name as seen from the named sheet (its
    /// sheet-scoped name first, else the workbook's).
    SheetName(Arc<str>, Arc<str>),
    /// A structured reference to a table: `Table1[Col]`, `[@Col]`.
    Table(TableRef),
    Unary(UnaryOp, Box<Expr>),
    Percent(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// Function call; the name is canonical (English, uppercase).
    Func(Arc<str>, Vec<Expr>),
    /// `{1,2;3,4}` array constant, row-major.
    Array(Vec<Vec<Expr>>),
    Paren(Box<Expr>),
    /// `A1#`: the whole array spilled from a cell.
    Spill(Box<Expr>),
    /// `@A1:A10`: implicit intersection, the one value on the formula's
    /// row or column.
    Implicit(Box<Expr>),
    /// `A1:C3 B2:D4`: the cells common to two references.
    Intersect(Box<Expr>, Box<Expr>),
    /// `A1:A3,C1:C3`: several areas as one reference. Written between
    /// parentheses wherever the separator would otherwise split arguments.
    Union(Vec<Expr>),
}

impl Expr {
    /// Render as formula text (without the leading `=`).
    pub fn to_text(&self, locale: &Locale) -> String {
        let mut s = String::new();
        self.write(locale, &mut s);
        s
    }

    pub fn write(&self, locale: &Locale, out: &mut String) {
        self.write_styled(locale, false, out);
    }

    /// Canonical English text with the spellings files use where they
    /// differ from the formula bar: `A1#` is `_xlfn.ANCHORARRAY(A1)`, `@x`
    /// is `_xlfn.SINGLE(x)` and `[@Col]` is `[[#This Row],[Col]]`.
    pub fn to_file_text(&self) -> String {
        let mut s = String::new();
        self.write_styled(Locale::en(), true, &mut s);
        s
    }

    fn write_styled(&self, locale: &Locale, file: bool, out: &mut String) {
        match self {
            Expr::Number(n) => out.push_str(&locale.number_to_text(*n)),
            Expr::Text(s) => {
                out.push('"');
                out.push_str(&s.replace('"', "\"\""));
                out.push('"');
            }
            Expr::Bool(b) => out.push_str(locale.bool_name(*b)),
            Expr::Error(e) => out.push_str(locale.error_name(*e)),
            Expr::Missing => {}
            Expr::Value(v) => match v {
                Value::Empty => {}
                Value::Number(n) => out.push_str(&locale.number_to_text(*n)),
                Value::Text(s) => {
                    out.push('"');
                    out.push_str(&s.replace('"', "\"\""));
                    out.push('"');
                }
                Value::Bool(b) => out.push_str(locale.bool_name(*b)),
                Value::Error(e) => out.push_str(locale.error_name(*e)),
                Value::Array(_) => out.push_str("{}"),
            },
            Expr::Ref(r) if r.sheet_deleted() => {
                out.push_str(locale.error_name(ErrorKind::Ref));
                r.write_cells(out);
            }
            Expr::Ref(r) => r.write(out),
            Expr::Ref3D(r) => r.write(out),
            Expr::RefError(sheet) => {
                write_sheet_name(sheet, out);
                out.push('!');
                out.push_str(locale.error_name(ErrorKind::Ref));
            }
            Expr::Name(n) => out.push_str(n),
            Expr::SheetName(sheet, n) => {
                write_sheet_name(sheet, out);
                out.push('!');
                out.push_str(n);
            }
            Expr::Table(t) => t.write(locale, file, out),
            Expr::Unary(op, e) => {
                out.push(match op {
                    UnaryOp::Neg => '-',
                    UnaryOp::Plus => '+',
                });
                e.write_styled(locale, file, out);
            }
            Expr::Percent(e) => {
                e.write_styled(locale, file, out);
                out.push('%');
            }
            Expr::Binary(op, a, b) => {
                a.write_styled(locale, file, out);
                out.push_str(op.symbol());
                b.write_styled(locale, file, out);
            }
            Expr::Func(name, args) => {
                out.push_str(locale.fn_to_local(name));
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push(locale.arg_sep);
                    }
                    // A bare union would read as several arguments.
                    let union = matches!(a, Expr::Union(_));
                    if union {
                        out.push('(');
                    }
                    a.write_styled(locale, file, out);
                    if union {
                        out.push(')');
                    }
                }
                out.push(')');
            }
            Expr::Array(rows) => {
                out.push('{');
                for (i, row) in rows.iter().enumerate() {
                    if i > 0 {
                        out.push(locale.array_row_sep);
                    }
                    for (j, e) in row.iter().enumerate() {
                        if j > 0 {
                            out.push(locale.array_col_sep);
                        }
                        e.write_styled(locale, file, out);
                    }
                }
                out.push('}');
            }
            Expr::Paren(e) => {
                out.push('(');
                e.write_styled(locale, file, out);
                out.push(')');
            }
            Expr::Spill(e) if file => {
                out.push_str("_xlfn.ANCHORARRAY(");
                e.write_styled(locale, file, out);
                out.push(')');
            }
            Expr::Spill(e) => {
                e.write_styled(locale, file, out);
                out.push('#');
            }
            Expr::Implicit(e) if file => {
                out.push_str("_xlfn.SINGLE(");
                e.write_styled(locale, file, out);
                out.push(')');
            }
            Expr::Implicit(e) => {
                out.push('@');
                e.write_styled(locale, file, out);
            }
            Expr::Intersect(a, b) => {
                a.write_styled(locale, file, out);
                out.push(' ');
                b.write_styled(locale, file, out);
            }
            Expr::Union(items) => {
                for (i, e) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(locale.arg_sep);
                    }
                    e.write_styled(locale, file, out);
                }
            }
        }
    }

    /// Visit every node, depth first.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Unary(_, e)
            | Expr::Percent(e)
            | Expr::Paren(e)
            | Expr::Spill(e)
            | Expr::Implicit(e) => e.walk(f),
            Expr::Binary(_, a, b) | Expr::Intersect(a, b) => {
                a.walk(f);
                b.walk(f);
            }
            Expr::Func(_, args) | Expr::Union(args) => {
                for a in args {
                    a.walk(f);
                }
            }
            Expr::Array(rows) => {
                for row in rows {
                    for e in row {
                        e.walk(f);
                    }
                }
            }
            _ => {}
        }
    }

    /// Rebuild the tree bottom-up: every node is handed to `f` once its
    /// children have been rebuilt, and replaced with what `f` returns.
    pub fn map(&self, f: &mut dyn FnMut(Expr) -> Expr) -> Expr {
        let node = match self {
            Expr::Unary(op, e) => Expr::Unary(*op, Box::new(e.map(f))),
            Expr::Percent(e) => Expr::Percent(Box::new(e.map(f))),
            Expr::Paren(e) => Expr::Paren(Box::new(e.map(f))),
            Expr::Implicit(e) => Expr::Implicit(Box::new(e.map(f))),
            Expr::Spill(e) => match e.map(f) {
                // Nothing spills from a cell that no longer exists.
                broken @ (Expr::Error(_) | Expr::RefError(_)) => broken,
                anchor => Expr::Spill(Box::new(anchor)),
            },
            Expr::Binary(op, a, b) => {
                let a = a.map(f);
                let b = b.map(f);
                Expr::Binary(*op, Box::new(a), Box::new(b))
            }
            Expr::Intersect(a, b) => {
                let a = a.map(f);
                let b = b.map(f);
                Expr::Intersect(Box::new(a), Box::new(b))
            }
            Expr::Func(name, args) => {
                Expr::Func(name.clone(), args.iter().map(|a| a.map(f)).collect())
            }
            Expr::Union(items) => Expr::Union(items.iter().map(|e| e.map(f)).collect()),
            Expr::Array(rows) => Expr::Array(
                rows.iter()
                    .map(|row| row.iter().map(|e| e.map(f)).collect())
                    .collect(),
            ),
            other => other.clone(),
        };
        f(node)
    }

    /// Rebuild the tree, replacing each plain reference with `f(ref)`.
    pub fn map_refs(&self, f: &mut dyn FnMut(&RefExpr) -> Expr) -> Expr {
        self.map(&mut |e| match e {
            Expr::Ref(r) => f(&r),
            other => other,
        })
    }

    /// All plain references in the formula, in source order.
    pub fn refs(&self) -> Vec<RefExpr> {
        let mut out = Vec::new();
        self.walk(&mut |e| {
            if let Expr::Ref(r) = e {
                out.push(r.clone());
            }
        });
        out
    }

    /// The formula as it reads after being copied by (`d_row`, `d_col`):
    /// relative parts move, absolute parts stay. References pushed off the
    /// sheet become `#REF!`. Structured references do not move.
    pub fn shifted(&self, d_row: i64, d_col: i64) -> Expr {
        if d_row == 0 && d_col == 0 {
            return self.clone();
        }
        self.map(&mut |e| match e {
            Expr::Ref(r) => match shift_ref(&r, d_row, d_col, false) {
                Some(r) => Expr::Ref(r),
                None => Expr::Error(ErrorKind::Ref),
            },
            Expr::Ref3D(r) => match shift_ref(&r.cells, d_row, d_col, false) {
                Some(cells) => Expr::Ref3D(Ref3D { cells, ..r }),
                None => Expr::Error(ErrorKind::Ref),
            },
            other => other,
        })
    }

    /// Like `shifted`, but relative parts pushed off one edge of the sheet
    /// come back in on the other. This is how the relative references of a
    /// defined name follow the cell using the name.
    pub fn rebased(&self, d_row: i64, d_col: i64) -> Expr {
        if d_row == 0 && d_col == 0 {
            return self.clone();
        }
        self.map(&mut |e| match e {
            Expr::Ref(r) => {
                shift_ref(&r, d_row, d_col, true).map_or(Expr::Error(ErrorKind::Ref), Expr::Ref)
            }
            Expr::Ref3D(r) => match shift_ref(&r.cells, d_row, d_col, true) {
                Some(cells) => Expr::Ref3D(Ref3D { cells, ..r }),
                None => Expr::Error(ErrorKind::Ref),
            },
            other => other,
        })
    }

    /// True when the formula holds a reference with a relative row or
    /// column, i.e. when it reads differently from another cell.
    pub fn has_relative_refs(&self) -> bool {
        let relative = |r: &RefExpr| {
            let end = r.end.unwrap_or(r.start);
            (r.kind != RefKind::Cols && !(r.start.abs_row && end.abs_row))
                || (r.kind != RefKind::Rows && !(r.start.abs_col && end.abs_col))
        };
        let mut found = false;
        self.walk(&mut |e| match e {
            Expr::Ref(r) => found |= relative(r),
            Expr::Ref3D(r) => found |= relative(&r.cells),
            _ => {}
        });
        found
    }
}

/// Move the relative parts of one corner by (`d_row`, `d_col`). Off the
/// sheet is `None`, unless `wrap` brings it back in on the other side.
fn shift_cell(c: &CellRef, kind: RefKind, d_row: i64, d_col: i64, wrap: bool) -> Option<CellRef> {
    let moved = |pos: u32, delta: i64, max: u32| -> Option<u32> {
        let n = pos as i64 + delta;
        if wrap {
            Some(n.rem_euclid(max as i64) as u32)
        } else if n < 0 || n >= max as i64 {
            None
        } else {
            Some(n as u32)
        }
    };
    let mut out = *c;
    if !c.abs_row && kind != RefKind::Cols {
        out.row = moved(c.row, d_row, MAX_ROWS)?;
    }
    if !c.abs_col && kind != RefKind::Rows {
        out.col = moved(c.col, d_col, MAX_COLS)?;
    }
    Some(out)
}

fn shift_ref(r: &RefExpr, d_row: i64, d_col: i64, wrap: bool) -> Option<RefExpr> {
    let start = shift_cell(&r.start, r.kind, d_row, d_col, wrap)?;
    let end = match &r.end {
        Some(e) => Some(shift_cell(e, r.kind, d_row, d_col, wrap)?),
        None => None,
    };
    Some(RefExpr {
        sheet: r.sheet.clone(),
        start,
        end,
        kind: r.kind,
    })
}

/// A structural edit to a sheet, used to rewrite references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructuralEdit {
    InsertRows { at: u32, count: u32 },
    DeleteRows { at: u32, count: u32 },
    InsertCols { at: u32, count: u32 },
    DeleteCols { at: u32, count: u32 },
}

/// Adjust an inclusive index interval for an insert/delete along one axis.
/// Returns `None` when the whole interval was deleted.
fn adjust_interval(
    lo: u32,
    hi: u32,
    insert: bool,
    at: u32,
    count: u32,
    max: u32,
) -> Option<(u32, u32)> {
    if insert {
        let nlo = if lo >= at { lo + count } else { lo };
        let nhi = if hi >= at { hi + count } else { hi };
        if nlo >= max {
            return None;
        }
        Some((nlo, nhi.min(max - 1)))
    } else {
        let del_end = at + count - 1;
        if lo >= at && hi <= del_end {
            return None;
        }
        let nlo = if lo > del_end {
            lo - count
        } else if lo >= at {
            at
        } else {
            lo
        };
        let nhi = if hi > del_end {
            hi - count
        } else if hi >= at {
            at - 1
        } else {
            hi
        };
        Some((nlo, nhi))
    }
}

/// A reference after rows / columns were inserted into or deleted from its
/// sheet; `None` when all its cells were deleted.
fn adjust_ref(r: &RefExpr, edit: StructuralEdit) -> Option<RefExpr> {
    let end = r.end.unwrap_or(r.start);
    let (row_axis, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    let mut out = r.clone();
    if row_axis {
        if r.kind == RefKind::Cols {
            return Some(out);
        }
        let (lo, hi, flipped) = if r.start.row <= end.row {
            (r.start.row, end.row, false)
        } else {
            (end.row, r.start.row, true)
        };
        let (nlo, nhi) = adjust_interval(lo, hi, insert, at, count, MAX_ROWS)?;
        let (s, e) = if flipped { (nhi, nlo) } else { (nlo, nhi) };
        out.start.row = s;
        if let Some(end) = &mut out.end {
            end.row = e;
        }
    } else {
        if r.kind == RefKind::Rows {
            return Some(out);
        }
        let (lo, hi, flipped) = if r.start.col <= end.col {
            (r.start.col, end.col, false)
        } else {
            (end.col, r.start.col, true)
        };
        let (nlo, nhi) = adjust_interval(lo, hi, insert, at, count, MAX_COLS)?;
        let (s, e) = if flipped { (nhi, nlo) } else { (nlo, nhi) };
        out.start.col = s;
        if let Some(end) = &mut out.end {
            end.col = e;
        }
    }
    Some(out)
}

/// Position of a sheet name in a list of names (tab order).
fn sheet_pos(order: &[&str], name: &str) -> Option<usize> {
    order.iter().position(|s| same_name(s, name))
}

/// A 3-D reference between two sheets given by name; one sheet is a plain
/// reference.
fn span_ref(first: &str, last: &str, cells: RefExpr) -> Expr {
    if same_name(first, last) {
        Expr::Ref(RefExpr {
            sheet: Some(Arc::from(first)),
            ..cells
        })
    } else {
        Expr::Ref3D(Ref3D {
            first: Arc::from(first),
            last: Arc::from(last),
            cells,
        })
    }
}

impl Expr {
    /// Rewrite references after rows/columns were inserted into or deleted
    /// from the sheet named `edited_sheet`. `own_sheet` is the name of the
    /// sheet holding this formula (unqualified references point there).
    ///
    /// A 3-D reference keeps its cells: they are the same on every sheet of
    /// the span, and only one of them changed.
    pub fn adjusted(&self, edit: StructuralEdit, edited_sheet: &str, own_sheet: &str) -> Expr {
        self.map_refs(&mut |r| {
            let target = r.sheet.as_deref().unwrap_or(own_sheet);
            if !same_name(target, edited_sheet) {
                return Expr::Ref(r.clone());
            }
            match (adjust_ref(r, edit), &r.sheet) {
                (Some(r), _) => Expr::Ref(r),
                // The sheet is still there, its cells are not: `Sheet2!#REF!`.
                (None, Some(sheet)) => Expr::RefError(sheet.clone()),
                (None, None) => Expr::Error(ErrorKind::Ref),
            }
        })
    }

    /// Rewrite sheet-qualified references after a sheet was renamed.
    pub fn with_sheet_renamed(&self, old: &str, new: &str) -> Expr {
        let renamed = |name: &Arc<str>| -> Arc<str> {
            if same_name(name, old) {
                Arc::from(new)
            } else {
                name.clone()
            }
        };
        self.map(&mut |e| match e {
            Expr::Ref(mut r) => {
                r.sheet = r.sheet.as_ref().map(&renamed);
                Expr::Ref(r)
            }
            Expr::Ref3D(r) => Expr::Ref3D(Ref3D {
                first: renamed(&r.first),
                last: renamed(&r.last),
                cells: r.cells,
            }),
            Expr::SheetName(sheet, name) => Expr::SheetName(renamed(&sheet), name),
            Expr::RefError(sheet) => Expr::RefError(renamed(&sheet)),
            other => other,
        })
    }

    /// Rewrite references after the sheet named `deleted` was removed.
    /// `order` lists the sheet names in tab order as they were before.
    ///
    /// References to the sheet read `#REF!A1`. A 3-D reference ending on
    /// the sheet shrinks to its neighbour inside the span; one going
    /// through it is unchanged.
    pub fn with_sheet_deleted(&self, deleted: &str, order: &[&str]) -> Expr {
        self.map(&mut |e| match e {
            Expr::Ref(mut r) if r.sheet.as_deref().is_some_and(|s| same_name(s, deleted)) => {
                r.sheet = Some(Arc::from(DELETED_SHEET));
                Expr::Ref(r)
            }
            Expr::Ref3D(r) => {
                let (Some(a), Some(b), Some(d)) = (
                    sheet_pos(order, &r.first),
                    sheet_pos(order, &r.last),
                    sheet_pos(order, deleted),
                ) else {
                    return Expr::Ref3D(r);
                };
                let (lo, hi) = (a.min(b), a.max(b));
                if lo == hi && d == lo {
                    return Expr::Ref(RefExpr {
                        sheet: Some(Arc::from(DELETED_SHEET)),
                        ..r.cells
                    });
                }
                if d == lo {
                    span_ref(order[lo + 1], order[hi], r.cells)
                } else if d == hi {
                    span_ref(order[lo], order[hi - 1], r.cells)
                } else {
                    Expr::Ref3D(r)
                }
            }
            Expr::SheetName(sheet, _) | Expr::RefError(sheet) if same_name(&sheet, deleted) => {
                Expr::Error(ErrorKind::Ref)
            }
            other => other,
        })
    }

    /// Rewrite 3-D references after the sheet named `moved` changed place.
    /// `before` and `after` list the sheet names in tab order around the
    /// move.
    ///
    /// A span follows its two end sheets wherever they go. Only when one end
    /// is carried past the other does the reference change: it lets go of
    /// the moved sheet and ends on its former neighbour inside the span.
    pub fn with_sheet_moved(&self, moved: &str, before: &[&str], after: &[&str]) -> Expr {
        self.map(&mut |e| match e {
            Expr::Ref3D(r) => {
                let (Some(a), Some(b)) = (sheet_pos(before, &r.first), sheet_pos(before, &r.last))
                else {
                    return Expr::Ref3D(r);
                };
                let (lo, hi) = (a.min(b), a.max(b));
                let (Some(new_lo), Some(new_hi)) =
                    (sheet_pos(after, before[lo]), sheet_pos(after, before[hi]))
                else {
                    return Expr::Ref3D(r);
                };
                if lo == hi || new_lo < new_hi {
                    return Expr::Ref3D(r);
                }
                if same_name(moved, before[lo]) {
                    span_ref(before[lo + 1], before[hi], r.cells)
                } else {
                    span_ref(before[lo], before[hi - 1], r.cells)
                }
            }
            other => other,
        })
    }

    /// Rewrite references to cells inside `from` (on `sheet`) so they follow
    /// a cut-and-paste move by (`d_row`, `d_col`).
    pub fn with_moved_range(
        &self,
        sheet: &str,
        own_sheet: &str,
        from: Rect,
        d_row: i64,
        d_col: i64,
    ) -> Expr {
        self.map_refs(&mut |r| {
            let target = r.sheet.as_deref().unwrap_or(own_sheet);
            if !same_name(target, sheet) || r.kind != RefKind::Cells {
                return Expr::Ref(r.clone());
            }
            let rect = r.rect();
            if !(from.contains(rect.r0, rect.c0) && from.contains(rect.r1, rect.c1)) {
                return Expr::Ref(r.clone());
            }
            let mv = |c: &CellRef| -> Option<CellRef> {
                let nr = c.row as i64 + d_row;
                let nc = c.col as i64 + d_col;
                if nr < 0 || nc < 0 || nr >= MAX_ROWS as i64 || nc >= MAX_COLS as i64 {
                    return None;
                }
                Some(CellRef {
                    row: nr as u32,
                    col: nc as u32,
                    ..*c
                })
            };
            let Some(start) = mv(&r.start) else {
                return Expr::Error(ErrorKind::Ref);
            };
            let end = match &r.end {
                Some(e) => match mv(e) {
                    Some(e) => Some(e),
                    None => return Expr::Error(ErrorKind::Ref),
                },
                None => None,
            };
            Expr::Ref(RefExpr {
                sheet: r.sheet.clone(),
                start,
                end,
                kind: r.kind,
            })
        })
    }

    /// Names bound by `LET(name, value, ..., body)` and
    /// `LAMBDA(param, ..., body)` anywhere in the formula: these are
    /// variables of the formula, not defined names.
    pub fn bound_names(&self) -> Vec<Arc<str>> {
        let mut names: Vec<Arc<str>> = Vec::new();
        self.walk(&mut |e| {
            let Expr::Func(name, args) = e else {
                return;
            };
            let Some((_body, params)) = args.split_last() else {
                return;
            };
            let step = match name.as_ref() {
                "LET" => 2,
                "LAMBDA" => 1,
                _ => return,
            };
            for p in params.iter().step_by(step) {
                if let Expr::Name(n) = p
                    && !names.iter().any(|x| x.eq_ignore_ascii_case(n))
                {
                    names.push(n.clone());
                }
            }
        });
        names
    }

    /// Give unqualified references the sheet they are meant for: a defined
    /// name always says which sheet its cells are on.
    pub fn with_default_sheet(&self, sheet: &str) -> Expr {
        self.map_refs(&mut |r| {
            let mut r = r.clone();
            if r.sheet.is_none() {
                r.sheet = Some(Arc::from(sheet));
            }
            Expr::Ref(r)
        })
    }
}

// ---- structured references -------------------------------------------------

impl Expr {
    /// True when the formula holds a structured reference.
    pub fn has_table_refs(&self) -> bool {
        let mut found = false;
        self.walk(&mut |e| found |= matches!(e, Expr::Table(_)));
        found
    }

    /// Follow a table renamed from `old` to `new`. The bare name of the
    /// table (`=ROWS(Table1)`) is a reference to it too.
    pub fn with_table_renamed(&self, old: &str, new: &str) -> Expr {
        self.map(&mut |e| match e {
            Expr::Table(mut t) if t.table.as_deref().is_some_and(|n| same_name(n, old)) => {
                t.table = Some(Arc::from(new));
                Expr::Table(t)
            }
            Expr::Name(n) if same_name(&n, old) => Expr::Name(Arc::from(new)),
            other => other,
        })
    }

    /// Follow a column of `table` renamed from `old` to `new`. `inside`
    /// says whether the formula sits in that table, so that its references
    /// without a table name (`[@Col]`) are meant too.
    pub fn with_table_column_renamed(
        &self,
        table: &str,
        inside: bool,
        old: &str,
        new: &str,
    ) -> Expr {
        let renamed = |c: Arc<str>| -> Arc<str> {
            if same_name(&c, old) {
                Arc::from(new)
            } else {
                c
            }
        };
        self.map(&mut |e| match e {
            Expr::Table(mut t) if t.is_to(table, inside) => {
                t.cols = match t.cols {
                    TableCols::All => TableCols::All,
                    TableCols::One(c) => TableCols::One(renamed(c)),
                    TableCols::Range(a, b) => TableCols::Range(renamed(a), renamed(b)),
                };
                Expr::Table(t)
            }
            other => other,
        })
    }

    /// References to a column of `table` that was deleted become `#REF!`.
    pub fn with_table_column_deleted(&self, table: &str, inside: bool, column: &str) -> Expr {
        self.map(&mut |e| match e {
            Expr::Table(t) if t.is_to(table, inside) => {
                let gone = match &t.cols {
                    TableCols::All => false,
                    TableCols::One(c) => same_name(c, column),
                    TableCols::Range(a, b) => same_name(a, column) || same_name(b, column),
                };
                if gone {
                    Expr::Error(ErrorKind::Ref)
                } else {
                    Expr::Table(t)
                }
            }
            other => other,
        })
    }

    /// Spell out the table name in the references that leave it to the
    /// position of the formula: `[@Col]` becomes `Table1[@Col]`.
    pub fn with_table_qualified(&self, table: &str) -> Expr {
        self.map(&mut |e| match e {
            Expr::Table(mut t) if t.table.is_none() => {
                t.table = Some(Arc::from(table));
                Expr::Table(t)
            }
            other => other,
        })
    }

    /// The reverse of `with_table_qualified`, for showing a formula that
    /// sits inside `table`: `Table1[@Col]` becomes `[@Col]`. The bare
    /// `Table1[]` forms that need the name keep it.
    pub fn with_table_unqualified(&self, table: &str) -> Expr {
        self.map(&mut |e| match e {
            Expr::Table(mut t)
                if t.table.as_deref().is_some_and(|n| same_name(n, table))
                    && !(t.rows == TableRows::Unspecified && t.cols == TableCols::All) =>
            {
                t.table = None;
                Expr::Table(t)
            }
            other => other,
        })
    }
}
