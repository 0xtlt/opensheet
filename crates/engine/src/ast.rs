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

/// A cell / range reference as written in a formula.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RefExpr {
    /// Sheet name as written (`None` = the formula's own sheet).
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

    pub fn write(&self, out: &mut String) {
        if let Some(sheet) = &self.sheet {
            write_sheet_name(sheet, out);
            out.push('!');
        }
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

/// Quote a sheet name when it contains anything but letters, digits, `_`, `.`
/// or when it could be mistaken for a cell reference.
pub fn write_sheet_name(name: &str, out: &mut String) {
    let simple = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.chars().next().unwrap().is_ascii_digit()
        && crate::refs::parse_cell_name(name).is_none();
    if simple {
        out.push_str(name);
    } else {
        out.push('\'');
        out.push_str(&name.replace('\'', "''"));
        out.push('\'');
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
    /// A defined name.
    Name(Arc<str>),
    Unary(UnaryOp, Box<Expr>),
    Percent(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// Function call; the name is canonical (English, uppercase).
    Func(Arc<str>, Vec<Expr>),
    /// `{1,2;3,4}` array constant, row-major.
    Array(Vec<Vec<Expr>>),
    Paren(Box<Expr>),
}

impl Expr {
    /// Render as formula text (without the leading `=`).
    pub fn to_text(&self, locale: &Locale) -> String {
        let mut s = String::new();
        self.write(locale, &mut s);
        s
    }

    pub fn write(&self, locale: &Locale, out: &mut String) {
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
            Expr::Ref(r) => r.write(out),
            Expr::Name(n) => out.push_str(n),
            Expr::Unary(op, e) => {
                out.push(match op {
                    UnaryOp::Neg => '-',
                    UnaryOp::Plus => '+',
                });
                e.write(locale, out);
            }
            Expr::Percent(e) => {
                e.write(locale, out);
                out.push('%');
            }
            Expr::Binary(op, a, b) => {
                a.write(locale, out);
                out.push_str(op.symbol());
                b.write(locale, out);
            }
            Expr::Func(name, args) => {
                out.push_str(locale.fn_to_local(name));
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push(locale.arg_sep);
                    }
                    a.write(locale, out);
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
                        e.write(locale, out);
                    }
                }
                out.push('}');
            }
            Expr::Paren(e) => {
                out.push('(');
                e.write(locale, out);
                out.push(')');
            }
        }
    }

    /// Visit every node, depth first.
    pub fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match self {
            Expr::Unary(_, e) | Expr::Percent(e) | Expr::Paren(e) => e.walk(f),
            Expr::Binary(_, a, b) => {
                a.walk(f);
                b.walk(f);
            }
            Expr::Func(_, args) => {
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

    /// Rebuild the tree, replacing each reference with `f(ref)`.
    pub fn map_refs(&self, f: &mut dyn FnMut(&RefExpr) -> Expr) -> Expr {
        match self {
            Expr::Ref(r) => f(r),
            Expr::Unary(op, e) => Expr::Unary(*op, Box::new(e.map_refs(f))),
            Expr::Percent(e) => Expr::Percent(Box::new(e.map_refs(f))),
            Expr::Paren(e) => Expr::Paren(Box::new(e.map_refs(f))),
            Expr::Binary(op, a, b) => {
                let a = a.map_refs(f);
                let b = b.map_refs(f);
                Expr::Binary(*op, Box::new(a), Box::new(b))
            }
            Expr::Func(name, args) => {
                Expr::Func(name.clone(), args.iter().map(|a| a.map_refs(f)).collect())
            }
            Expr::Array(rows) => Expr::Array(
                rows.iter()
                    .map(|row| row.iter().map(|e| e.map_refs(f)).collect())
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// All references in the formula, in source order.
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
    /// sheet become `#REF!`.
    pub fn shifted(&self, d_row: i64, d_col: i64) -> Expr {
        if d_row == 0 && d_col == 0 {
            return self.clone();
        }
        self.map_refs(&mut |r| {
            let shift = |c: &CellRef, kind: RefKind| -> Option<CellRef> {
                let mut out = *c;
                if !c.abs_row && kind != RefKind::Cols {
                    let nr = c.row as i64 + d_row;
                    if nr < 0 || nr >= MAX_ROWS as i64 {
                        return None;
                    }
                    out.row = nr as u32;
                }
                if !c.abs_col && kind != RefKind::Rows {
                    let nc = c.col as i64 + d_col;
                    if nc < 0 || nc >= MAX_COLS as i64 {
                        return None;
                    }
                    out.col = nc as u32;
                }
                Some(out)
            };
            let Some(start) = shift(&r.start, r.kind) else {
                return Expr::Error(ErrorKind::Ref);
            };
            let end = match &r.end {
                Some(e) => match shift(e, r.kind) {
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
fn adjust_interval(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32)> {
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

impl Expr {
    /// Rewrite references after rows/columns were inserted into or deleted
    /// from the sheet named `edited_sheet`. `own_sheet` is the name of the
    /// sheet holding this formula (unqualified references point there).
    pub fn adjusted(&self, edit: StructuralEdit, edited_sheet: &str, own_sheet: &str) -> Expr {
        self.map_refs(&mut |r| {
            let target = r.sheet.as_deref().unwrap_or(own_sheet);
            if !target.eq_ignore_ascii_case(edited_sheet) {
                return Expr::Ref(r.clone());
            }
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
                    return Expr::Ref(out);
                }
                let (lo, hi, flipped) = if r.start.row <= end.row {
                    (r.start.row, end.row, false)
                } else {
                    (end.row, r.start.row, true)
                };
                match adjust_interval(lo, hi, insert, at, count, MAX_ROWS) {
                    None => return Expr::Error(ErrorKind::Ref),
                    Some((nlo, nhi)) => {
                        let (s, e) = if flipped { (nhi, nlo) } else { (nlo, nhi) };
                        out.start.row = s;
                        if let Some(end) = &mut out.end {
                            end.row = e;
                        }
                    }
                }
            } else {
                if r.kind == RefKind::Rows {
                    return Expr::Ref(out);
                }
                let (lo, hi, flipped) = if r.start.col <= end.col {
                    (r.start.col, end.col, false)
                } else {
                    (end.col, r.start.col, true)
                };
                match adjust_interval(lo, hi, insert, at, count, MAX_COLS) {
                    None => return Expr::Error(ErrorKind::Ref),
                    Some((nlo, nhi)) => {
                        let (s, e) = if flipped { (nhi, nlo) } else { (nlo, nhi) };
                        out.start.col = s;
                        if let Some(end) = &mut out.end {
                            end.col = e;
                        }
                    }
                }
            }
            Expr::Ref(out)
        })
    }

    /// Rewrite sheet-qualified references after a sheet was renamed.
    pub fn with_sheet_renamed(&self, old: &str, new: &str) -> Expr {
        self.map_refs(&mut |r| {
            let mut out = r.clone();
            if let Some(s) = &r.sheet {
                if s.eq_ignore_ascii_case(old) {
                    out.sheet = Some(Arc::from(new));
                }
            }
            Expr::Ref(out)
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
            if !target.eq_ignore_ascii_case(sheet) || r.kind != RefKind::Cells {
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
}
