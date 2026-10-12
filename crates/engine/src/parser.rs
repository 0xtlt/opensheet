//! Formula parser (locale aware, scannerless recursive descent).
//!
//! Precedence, lowest to highest:
//! comparison, `&`, `+ -`, `* /`, `^`, unary `- +`, postfix `%`, then the
//! reference operators: intersection (a space), `@`, postfix `#`. The
//! union operator is the argument separator, so it is only read where no
//! argument list is open: between parentheses and at the top level.

use crate::ast::{
    BinaryOp, CellRef, DELETED_SHEET, Expr, Ref3D, RefExpr, RefKind, TableCols, TableItem,
    TableRef, TableRows, UnaryOp, same_name,
};
use crate::locale::Locale;
use crate::refs::{MAX_ROWS, letters_to_col};
use crate::value::ErrorKind;
use crate::workbook::Workbook;
use std::ops::Range;
use std::sync::Arc;

/// Parentheses, function calls and signs can nest this deep. The parser is
/// recursive: the limit keeps hostile input from exhausting the stack.
const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// Byte offset in the source where parsing failed.
    pub pos: usize,
    pub message: String,
}

/// A reference found in formula text, with its byte range in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct RefSpan {
    pub range: Range<usize>,
    /// The cells the text stands for. For `A1#` read without a workbook
    /// this is the anchor cell; for a 3-D reference, the cells on the
    /// first sheet of the span.
    pub reference: RefExpr,
    /// The last sheet of a 3-D reference (`reference.sheet` is the first).
    pub last_sheet: Option<Arc<str>>,
}

/// Parse formula text (without the leading `=`).
pub fn parse_formula(src: &str, locale: &Locale) -> Result<Expr, ParseError> {
    let mut p = Parser::new(src, locale);
    let e = p.parse_union()?;
    p.skip_ws();
    if p.pos < p.chars.len() {
        return Err(p.error("unexpected character"));
    }
    Ok(e)
}

/// Find the references in (possibly incomplete) formula text. Used to
/// colour ranges while the user is typing.
///
/// Only what the text says by itself is reported: tables, defined names
/// and the extent of spilled arrays need the workbook (`scan_refs_in`).
pub fn scan_refs(src: &str, locale: &Locale) -> Vec<RefSpan> {
    scan_tokens(src, locale)
        .into_iter()
        .filter_map(|t| {
            let (reference, last_sheet) = match t.token {
                Token::Ref(r) => (r, None),
                Token::Ref3D(r) => (
                    RefExpr {
                        sheet: Some(r.first),
                        ..r.cells
                    },
                    Some(r.last),
                ),
                Token::Table(_) | Token::Name(..) => return None,
            };
            Some(RefSpan {
                range: t.range,
                reference,
                last_sheet,
            })
        })
        .collect()
}

/// Like `scan_refs`, for a formula being typed into the cell (`sheet`,
/// `row`, `col`) of `wb` (in the workbook's locale): structured references
/// and defined names standing for a range are reported with the cells they
/// cover, and `A1#` with the whole spilled array.
pub fn scan_refs_in(src: &str, wb: &Workbook, sheet: usize, row: u32, col: u32) -> Vec<RefSpan> {
    let on_sheet = |reference: crate::refs::Reference| -> Option<RefExpr> {
        let name = &wb.sheets.get(reference.sheet)?.name;
        Some(RefExpr {
            sheet: Some(Arc::from(name.as_str())),
            ..RefExpr::range(reference.rect)
        })
    };
    scan_tokens(src, wb.locale)
        .into_iter()
        .filter_map(|t| {
            let mut last_sheet = None;
            let mut reference = match t.token {
                Token::Ref(r) => r,
                Token::Ref3D(r) => {
                    last_sheet = Some(r.last);
                    RefExpr {
                        sheet: Some(r.first),
                        ..r.cells
                    }
                }
                Token::Table(table_ref) => {
                    on_sheet(crate::structured::resolve(wb, &table_ref, sheet, row, col).ok()?)?
                }
                Token::Name(scope, name) => {
                    let from = match &scope {
                        Some(s) => wb.sheet_index(s)?,
                        None => sheet,
                    };
                    on_sheet(wb.name_reference(from, row, col, &name)?)?
                }
            };
            if t.spill {
                // The whole array, when the anchor has spilled.
                let on = match &reference.sheet {
                    Some(s) => wb.sheet_index(s),
                    None => Some(sheet),
                };
                let anchor = reference.rect();
                let spill = on.and_then(|si| wb.sheets.get(si)).and_then(|sh| {
                    sh.spills
                        .iter()
                        .find(|sp| (sp.row, sp.col) == (anchor.r0, anchor.c0))
                });
                if let Some(sp) = spill {
                    let rect = crate::refs::Rect::new(
                        sp.row,
                        sp.col,
                        sp.row + sp.array.rows as u32 - 1,
                        sp.col + sp.array.cols as u32 - 1,
                    );
                    reference = RefExpr {
                        sheet: reference.sheet,
                        ..RefExpr::range(rect)
                    };
                }
            }
            Some(RefSpan {
                range: t.range,
                reference,
                last_sheet,
            })
        })
        .collect()
}

/// Something in formula text that stands for cells.
enum Token {
    Ref(RefExpr),
    Ref3D(Ref3D),
    Table(TableRef),
    /// A name (defined name or table), with the sheet it was qualified with.
    Name(Option<Arc<str>>, Arc<str>),
}

struct TokenSpan {
    /// Byte range in the source.
    range: Range<usize>,
    token: Token,
    /// Followed by the spilled range operator `#`.
    spill: bool,
}

fn scan_tokens(src: &str, locale: &Locale) -> Vec<TokenSpan> {
    let mut p = Parser::new(src, locale);
    p.lenient = true;
    while p.pos < p.chars.len() {
        let before = p.pos;
        let _ = p.parse_expr();
        if p.pos == before {
            p.pos += 1;
        }
    }
    p.spans
}

struct Parser<'a> {
    chars: Vec<char>,
    /// Byte offset of each char, plus the total length at the end.
    offsets: Vec<usize>,
    pos: usize,
    locale: &'a Locale,
    spans: Vec<TokenSpan>,
    lenient: bool,
    depth: usize,
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '\\'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '\\' || c == '?'
}

/// True for what the reference operators (space, union, `#`) can apply
/// to: anything that may stand for cells. Errors count, since that is what
/// a deleted reference leaves behind (`A1:C3 #REF!`).
fn is_reference_like(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Ref(_)
            | Expr::Ref3D(_)
            | Expr::RefError(_)
            | Expr::Error(_)
            | Expr::Name(_)
            | Expr::SheetName(..)
            | Expr::Table(_)
            | Expr::Paren(_)
            | Expr::Func(..)
            | Expr::Spill(_)
            | Expr::Implicit(_)
            | Expr::Intersect(..)
    )
}

/// What can anchor a spilled array.
fn can_spill(e: &Expr) -> bool {
    match e {
        Expr::Ref(r) => !r.sheet_deleted(),
        Expr::Name(_) | Expr::SheetName(..) | Expr::Table(_) | Expr::Paren(_) | Expr::Func(..) => {
            true
        }
        _ => false,
    }
}

/// A sheet prefix in front of a reference.
enum Prefix {
    None,
    Sheet(Arc<str>),
    /// `First:Last!`
    Span(Arc<str>, Arc<str>),
}

impl Prefix {
    fn span(first: &str, last: &str) -> Prefix {
        if same_name(first, last) {
            Prefix::Sheet(Arc::from(first))
        } else {
            Prefix::Span(Arc::from(first), Arc::from(last))
        }
    }
}

impl<'a> Parser<'a> {
    fn new(src: &str, locale: &'a Locale) -> Parser<'a> {
        let mut chars = Vec::with_capacity(src.len());
        let mut offsets = Vec::with_capacity(src.len() + 1);
        for (i, c) in src.char_indices() {
            chars.push(c);
            offsets.push(i);
        }
        offsets.push(src.len());
        Parser {
            chars,
            offsets,
            pos: 0,
            locale,
            spans: Vec::new(),
            lenient: false,
            depth: 0,
        }
    }

    fn error(&self, msg: &str) -> ParseError {
        ParseError {
            pos: self.offsets[self.pos.min(self.chars.len())],
            message: msg.to_string(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn push_span(&mut self, start: usize, token: Token) {
        self.spans.push(TokenSpan {
            range: self.offsets[start]..self.offsets[self.pos],
            token,
            spill: false,
        });
    }

    /// One more level of nesting; an error when that is too many.
    fn enter(&mut self) -> Result<(), ParseError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error("formula is nested too deeply"));
        }
        self.depth += 1;
        Ok(())
    }

    /// An expression, or several references joined by the union operator.
    fn parse_union(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_expr()?;
        self.skip_ws();
        if self.peek() != Some(self.locale.arg_sep) || !is_reference_like(&first) {
            return Ok(first);
        }
        let mut items = vec![first];
        loop {
            self.skip_ws();
            if !self.eat(self.locale.arg_sep) {
                break;
            }
            let item = self.parse_expr()?;
            if !is_reference_like(&item) {
                return Err(self.error("expected a reference"));
            }
            items.push(item);
        }
        Ok(Expr::Union(items))
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.enter()?;
        let e = self.parse_cmp();
        self.depth -= 1;
        e
    }

    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_concat()?;
        loop {
            self.skip_ws();
            let op = match (self.peek(), self.peek_at(1)) {
                (Some('<'), Some('=')) => (BinaryOp::Le, 2),
                (Some('>'), Some('=')) => (BinaryOp::Ge, 2),
                (Some('<'), Some('>')) => (BinaryOp::Ne, 2),
                (Some('<'), _) => (BinaryOp::Lt, 1),
                (Some('>'), _) => (BinaryOp::Gt, 1),
                (Some('='), _) => (BinaryOp::Eq, 1),
                _ => break,
            };
            self.pos += op.1;
            let right = self.parse_concat()?;
            left = Expr::Binary(op.0, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_add()?;
        loop {
            self.skip_ws();
            if self.eat('&') {
                let right = self.parse_add()?;
                left = Expr::Binary(BinaryOp::Concat, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_mul()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('+') => BinaryOp::Add,
                Some('-') => BinaryOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_mul()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_pow()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some('*') => BinaryOp::Mul,
                Some('/') => BinaryOp::Div,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_pow()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_pow(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            if self.eat('^') {
                let right = self.parse_unary()?;
                left = Expr::Binary(BinaryOp::Pow, Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let op = match self.peek() {
            Some('-') => UnaryOp::Neg,
            Some('+') => UnaryOp::Plus,
            _ => return self.parse_postfix(),
        };
        self.pos += 1;
        self.enter()?;
        let inner = self.parse_unary();
        self.depth -= 1;
        Ok(Expr::Unary(op, Box::new(inner?)))
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_intersection()?;
        loop {
            self.skip_ws();
            if self.eat('%') {
                e = Expr::Percent(Box::new(e));
            } else {
                break;
            }
        }
        Ok(e)
    }

    /// References separated by spaces: the cells they have in common.
    fn parse_intersection(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_prefixed()?;
        while is_reference_like(&left) && self.at_intersection() {
            let right = self.parse_prefixed()?;
            if !is_reference_like(&right) {
                return Err(self.error("expected a reference"));
            }
            left = Expr::Intersect(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// True when spaces are followed by the start of another operand: the
    /// spaces are then the intersection operator.
    fn at_intersection(&self) -> bool {
        let mut i = self.pos;
        while self.chars.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        if i == self.pos {
            return false;
        }
        match self.chars.get(i) {
            Some(&c) if is_name_start(c) || matches!(c, '$' | '\'' | '(' | '[' | '#' | '@') => true,
            // Among numbers only whole rows (`1:3`) are references.
            Some(c) if c.is_ascii_digit() => self
                .scan_row(i)
                .is_some_and(|(_, _, j)| self.chars.get(j) == Some(&':')),
            _ => false,
        }
    }

    /// An operand with its `@` prefix, if any.
    fn parse_prefixed(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let mut implicit = false;
        while self.eat('@') {
            // `@@x` says no more than `@x`.
            implicit = true;
            self.skip_ws();
        }
        let e = self.parse_spilled()?;
        Ok(if implicit {
            Expr::Implicit(Box::new(e))
        } else {
            e
        })
    }

    /// An operand with its `#` suffix, if any.
    fn parse_spilled(&mut self) -> Result<Expr, ParseError> {
        let e = self.parse_primary()?;
        if self.peek() != Some('#') || !can_spill(&e) {
            return Ok(e);
        }
        let hash = self.pos;
        self.pos += 1;
        if let Some(last) = self.spans.last_mut()
            && last.range.end == self.offsets[hash]
        {
            last.range.end = self.offsets[self.pos];
            last.spill = true;
        }
        Ok(Expr::Spill(Box::new(e)))
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return Err(self.error("unexpected end of formula"));
        };
        match c {
            '(' => {
                self.pos += 1;
                let e = self.parse_union()?;
                self.skip_ws();
                if !self.eat(')') {
                    return Err(self.error("expected )"));
                }
                Ok(Expr::Paren(Box::new(e)))
            }
            '"' => Ok(Expr::Text(Arc::from(self.parse_string()?))),
            '{' => self.parse_array(),
            '#' => {
                let kind = self.parse_error_literal()?;
                if kind == ErrorKind::Ref
                    && let Some((cells, end)) = self.scan_ref(self.pos)
                {
                    // `#REF!A1`: cells of a sheet that was deleted.
                    self.pos = end;
                    return Ok(Expr::Ref(RefExpr {
                        sheet: Some(Arc::from(DELETED_SHEET)),
                        ..cells
                    }));
                }
                Ok(Expr::Error(kind))
            }
            '[' => self.parse_table_ref(None, self.pos),
            '\'' => self.parse_ident_or_ref(),
            '$' => self.parse_ident_or_ref(),
            c if c.is_ascii_digit() => {
                if let Some((r, end)) = self.scan_ref(self.pos) {
                    let start = self.pos;
                    self.pos = end;
                    self.push_span(start, Token::Ref(r.clone()));
                    return Ok(Expr::Ref(r));
                }
                self.parse_number().map(Expr::Number)
            }
            c if c == self.locale.decimal_sep
                && self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) =>
            {
                self.parse_number().map(Expr::Number)
            }
            c if is_name_start(c) => self.parse_ident_or_ref(),
            _ => Err(self.error("unexpected character")),
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.peek(), Some('"'));
        self.pos += 1;
        let mut s = String::new();
        loop {
            match self.peek() {
                None => {
                    if self.lenient {
                        return Ok(s);
                    }
                    return Err(self.error("unterminated string"));
                }
                Some('"') => {
                    if self.peek_at(1) == Some('"') {
                        s.push('"');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        return Ok(s);
                    }
                }
                Some(c) => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_number(&mut self) -> Result<f64, ParseError> {
        let start = self.pos;
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.peek() == Some(self.locale.decimal_sep)
            && (self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) || !s.is_empty())
        // In array constants the column separator may equal '.', which is
        // only a decimal point when it is the locale's decimal separator.
        {
            let save = self.pos;
            self.pos += 1;
            let mut frac = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    frac.push(c);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if frac.is_empty() && s.is_empty() {
                self.pos = save;
            } else {
                s.push('.');
                s.push_str(&frac);
            }
        }
        if s.is_empty() {
            self.pos = start;
            return Err(self.error("expected number"));
        }
        // Exponent
        if matches!(self.peek(), Some('e' | 'E')) {
            let mut i = 1;
            let mut exp = String::from("e");
            if matches!(self.peek_at(i), Some('+' | '-')) {
                exp.push(self.peek_at(i).unwrap());
                i += 1;
            }
            let mut digits = 0;
            while let Some(c) = self.peek_at(i) {
                if c.is_ascii_digit() {
                    exp.push(c);
                    i += 1;
                    digits += 1;
                } else {
                    break;
                }
            }
            if digits > 0 {
                s.push_str(&exp);
                self.pos += i;
            }
        }
        s.parse::<f64>().map_err(|_| self.error("invalid number"))
    }

    /// An error value in the locale's spelling or in English. The longest
    /// spelling wins (`#NULL!` over the French `#NUL!`).
    fn parse_error_literal(&mut self) -> Result<ErrorKind, ParseError> {
        let mut best: Option<(usize, ErrorKind)> = None;
        for (i, kind) in ErrorKind::ALL.iter().enumerate() {
            for name in [self.locale.errors[i], kind.as_str()] {
                let len = name.chars().count();
                let matches = self.chars.len() - self.pos >= len
                    && name
                        .chars()
                        .zip(&self.chars[self.pos..])
                        .all(|(n, c)| c.to_uppercase().eq(std::iter::once(n)));
                if matches && best.is_none_or(|(l, _)| len > l) {
                    best = Some((len, *kind));
                }
            }
        }
        match best {
            Some((len, kind)) => {
                self.pos += len;
                Ok(kind)
            }
            None => Err(self.error("unknown error value")),
        }
    }

    fn parse_array(&mut self) -> Result<Expr, ParseError> {
        debug_assert_eq!(self.peek(), Some('{'));
        self.pos += 1;
        let mut rows: Vec<Vec<Expr>> = vec![Vec::new()];
        loop {
            self.skip_ws();
            let elem = match self.peek() {
                Some('"') => Expr::Text(Arc::from(self.parse_string()?)),
                Some('#') => Expr::Error(self.parse_error_literal()?),
                Some('-') => {
                    self.pos += 1;
                    self.skip_ws();
                    Expr::Number(-self.parse_number()?)
                }
                Some('+') => {
                    self.pos += 1;
                    self.skip_ws();
                    Expr::Number(self.parse_number()?)
                }
                Some(c) if c.is_ascii_digit() || c == self.locale.decimal_sep => {
                    Expr::Number(self.parse_number()?)
                }
                Some(c) if c.is_alphabetic() => {
                    let mut name = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_alphabetic() {
                            name.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    match self.locale.parse_bool(&name) {
                        Some(b) => Expr::Bool(b),
                        None => return Err(self.error("invalid array constant")),
                    }
                }
                _ => return Err(self.error("invalid array constant")),
            };
            rows.last_mut().unwrap().push(elem);
            self.skip_ws();
            match self.peek() {
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                Some(c) if c == self.locale.array_row_sep => {
                    self.pos += 1;
                    rows.push(Vec::new());
                }
                Some(c) if c == self.locale.array_col_sep => {
                    self.pos += 1;
                }
                _ => return Err(self.error("expected } in array constant")),
            }
        }
        let width = rows[0].len();
        if rows.iter().any(|r| r.len() != width) {
            return Err(self.error("array rows must have the same length"));
        }
        Ok(Expr::Array(rows))
    }

    /// Parse `$?LETTERS` at `i`; returns (col, abs, next index).
    fn scan_col(&self, mut i: usize) -> Option<(u32, bool, usize)> {
        let abs = self.chars.get(i) == Some(&'$');
        if abs {
            i += 1;
        }
        let start = i;
        while i < self.chars.len() && self.chars[i].is_ascii_alphabetic() && i - start < 4 {
            i += 1;
        }
        if i == start {
            return None;
        }
        let letters: String = self.chars[start..i].iter().collect();
        letters_to_col(&letters).map(|c| (c, abs, i))
    }

    /// Parse `$?DIGITS` at `i`; returns (row, abs, next index).
    fn scan_row(&self, mut i: usize) -> Option<(u32, bool, usize)> {
        let abs = self.chars.get(i) == Some(&'$');
        if abs {
            i += 1;
        }
        let start = i;
        while i < self.chars.len() && self.chars[i].is_ascii_digit() && i - start < 8 {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits: String = self.chars[start..i].iter().collect();
        let n: u32 = digits.parse().ok()?;
        if n == 0 || n > MAX_ROWS {
            return None;
        }
        Some((n - 1, abs, i))
    }

    fn scan_cell(&self, i: usize) -> Option<(CellRef, usize)> {
        let (col, abs_col, i) = self.scan_col(i)?;
        let (row, abs_row, i) = self.scan_row(i)?;
        Some((
            CellRef {
                row,
                col,
                abs_row,
                abs_col,
            },
            i,
        ))
    }

    fn ends_token(&self, i: usize) -> bool {
        match self.chars.get(i) {
            None => true,
            Some(&c) => !(is_name_char(c) || c == '(' || c == '$' || c == '!'),
        }
    }

    /// Read a cell / range / whole-column / whole-row reference (without
    /// sheet name) starting at `i`; returns it with the index after it.
    fn scan_ref(&self, i: usize) -> Option<(RefExpr, usize)> {
        if let Some((start, j)) = self.scan_cell(i) {
            // A1:B2
            if self.chars.get(j) == Some(&':')
                && let Some((end, k)) = self.scan_cell(j + 1)
                && self.ends_token(k)
            {
                return Some((
                    RefExpr {
                        sheet: None,
                        start,
                        end: Some(end),
                        kind: RefKind::Cells,
                    },
                    k,
                ));
            }
            // A1
            if self.ends_token(j) {
                return Some((
                    RefExpr {
                        sheet: None,
                        start,
                        end: None,
                        kind: RefKind::Cells,
                    },
                    j,
                ));
            }
        }
        // A:C
        if let Some((c0, abs0, j)) = self.scan_col(i)
            && self.chars.get(j) == Some(&':')
            && let Some((c1, abs1, k)) = self.scan_col(j + 1)
            && self.ends_token(k)
            && !self.chars.get(k).is_some_and(|c| c.is_ascii_digit())
        {
            return Some((
                RefExpr {
                    sheet: None,
                    start: CellRef {
                        row: 0,
                        col: c0,
                        abs_row: false,
                        abs_col: abs0,
                    },
                    end: Some(CellRef {
                        row: MAX_ROWS - 1,
                        col: c1,
                        abs_row: false,
                        abs_col: abs1,
                    }),
                    kind: RefKind::Cols,
                },
                k,
            ));
        }
        // 1:3
        if let Some((r0, abs0, j)) = self.scan_row(i)
            && self.chars.get(j) == Some(&':')
            && let Some((r1, abs1, k)) = self.scan_row(j + 1)
            && self.ends_token(k)
        {
            return Some((
                RefExpr {
                    sheet: None,
                    start: CellRef {
                        row: r0,
                        col: 0,
                        abs_row: abs0,
                        abs_col: false,
                    },
                    end: Some(CellRef {
                        row: r1,
                        col: crate::refs::MAX_COLS - 1,
                        abs_row: abs1,
                        abs_col: false,
                    }),
                    kind: RefKind::Rows,
                },
                k,
            ));
        }
        None
    }

    fn scan_name(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_name_char(c) {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    /// `'Quoted name'` with `''` standing for one quote.
    fn parse_quoted_name(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.peek(), Some('\''));
        self.pos += 1;
        let mut name = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("unterminated sheet name")),
                Some('\'') => {
                    if self.peek_at(1) == Some('\'') {
                        name.push('\'');
                        self.pos += 2;
                    } else {
                        self.pos += 1;
                        return Ok(name);
                    }
                }
                Some(c) => {
                    name.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// The sheet name (`Sheet1!`, `'My sheet'!`) or span of sheets
    /// (`Sheet1:Sheet3!`, `'Jan 24:Mar 24'!`) in front of a reference.
    fn parse_sheet_prefix(&mut self) -> Result<Prefix, ParseError> {
        if self.peek() == Some('\'') {
            let name = self.parse_quoted_name()?;
            if !self.eat('!') {
                return Err(self.error("expected ! after sheet name"));
            }
            // A sheet name cannot hold a colon: this is a span of sheets,
            // unless it is the path of another workbook.
            return Ok(match name.split_once(':') {
                Some((first, last))
                    if !first.is_empty()
                        && !last.is_empty()
                        && !name.contains(['[', ']', '\\', '/']) =>
                {
                    Prefix::span(first, last)
                }
                _ => Prefix::Sheet(Arc::from(name)),
            });
        }
        if self.peek() == Some('$') {
            return Ok(Prefix::None);
        }
        let save = self.pos;
        let name = self.scan_name();
        if !name.is_empty() && self.eat('!') {
            return Ok(Prefix::Sheet(Arc::from(name)));
        }
        if !name.is_empty() && self.eat(':') {
            let last = self.scan_name();
            if !last.is_empty() && self.eat('!') {
                return Ok(Prefix::span(&name, &last));
            }
        }
        self.pos = save;
        Ok(Prefix::None)
    }

    fn parse_ident_or_ref(&mut self) -> Result<Expr, ParseError> {
        let span_start = self.pos;
        match self.parse_sheet_prefix()? {
            Prefix::Span(first, last) => {
                let Some((cells, end)) = self.scan_ref(self.pos) else {
                    return Err(self.error("expected reference"));
                };
                self.pos = end;
                let r = Ref3D { first, last, cells };
                self.push_span(span_start, Token::Ref3D(r.clone()));
                return Ok(Expr::Ref3D(r));
            }
            Prefix::Sheet(sheet) => {
                if let Some((cells, end)) = self.scan_ref(self.pos) {
                    self.pos = end;
                    let r = RefExpr {
                        sheet: Some(sheet),
                        ..cells
                    };
                    self.push_span(span_start, Token::Ref(r.clone()));
                    return Ok(Expr::Ref(r));
                }
                if self.peek() == Some('#') {
                    // `Sheet2!#REF!`: what a reference to deleted cells
                    // of another sheet turns into.
                    return Ok(match self.parse_error_literal()? {
                        ErrorKind::Ref => Expr::RefError(sheet),
                        other => Expr::Error(other),
                    });
                }
                // A name as seen from that sheet.
                if !self.peek().is_some_and(is_name_start) {
                    return Err(self.error("expected reference"));
                }
                let name: Arc<str> = Arc::from(self.scan_name());
                self.push_span(span_start, Token::Name(Some(sheet.clone()), name.clone()));
                return Ok(Expr::SheetName(sheet, name));
            }
            Prefix::None => {}
        }
        if let Some((r, end)) = self.scan_ref(self.pos) {
            self.pos = end;
            self.push_span(span_start, Token::Ref(r.clone()));
            return Ok(Expr::Ref(r));
        }
        if self.peek() == Some('$') {
            return Err(self.error("invalid reference"));
        }

        let name = self.scan_name();
        if name.is_empty() {
            return Err(self.error("unexpected character"));
        }
        if self.peek() == Some('(') {
            self.pos += 1;
            let mut upper = name.to_uppercase();
            for prefix in ["_XLFN._XLWS.", "_XLFN.", "_XLWS."] {
                if let Some(rest) = upper.strip_prefix(prefix) {
                    upper = rest.to_string();
                    break;
                }
            }
            let canonical = self.locale.fn_to_canonical(&upper);
            let mut args = self.parse_args()?;
            // The names under which files (and typing) spell the `#` and
            // `@` operators.
            return Ok(match (canonical.as_str(), args.as_slice()) {
                ("ANCHORARRAY", [anchor]) if can_spill(anchor) => {
                    Expr::Spill(Box::new(args.remove(0)))
                }
                ("SINGLE", [_]) => {
                    let inner = args.remove(0);
                    // `@` binds tighter than the operators a function
                    // argument may hold.
                    let operand = matches!(
                        inner,
                        Expr::Unary(..)
                            | Expr::Binary(..)
                            | Expr::Percent(_)
                            | Expr::Intersect(..)
                            | Expr::Union(_)
                            | Expr::Missing
                    );
                    Expr::Implicit(Box::new(if operand {
                        Expr::Paren(Box::new(inner))
                    } else {
                        inner
                    }))
                }
                _ => Expr::Func(Arc::from(canonical), args),
            });
        }
        if self.peek() == Some('[') {
            return self.parse_table_ref(Some(Arc::from(name)), span_start);
        }
        if let Some(b) = self.locale.parse_bool(&name) {
            return Ok(Expr::Bool(b));
        }
        let name: Arc<str> = Arc::from(name);
        self.push_span(span_start, Token::Name(None, name.clone()));
        Ok(Expr::Name(name))
    }

    /// A structured reference from its opening bracket: `[Col]`,
    /// `[#Headers]`, `[@Col]`, `[[#Headers],[Col1]:[Col2]]`. Item
    /// specifiers are read in the locale's spelling or in English, the
    /// parts are separated by the locale's list separator or a comma.
    fn parse_table_ref(
        &mut self,
        table: Option<Arc<str>>,
        span_start: usize,
    ) -> Result<Expr, ParseError> {
        debug_assert_eq!(self.peek(), Some('['));
        self.pos += 1;
        let mut items: Vec<TableItem> = Vec::new();
        let mut cols = TableCols::All;
        let save = self.pos;
        self.skip_ws();
        match self.peek() {
            Some(']') => {}
            Some('@') => {
                self.pos += 1;
                items.push(TableItem::ThisRow);
                self.skip_ws();
                match self.peek() {
                    Some(']') => {}
                    Some('[') => cols = self.parse_column_range()?,
                    _ => cols = TableCols::One(self.parse_column_name()?),
                }
            }
            Some('#') => items.push(self.parse_table_item()?),
            Some('[') => loop {
                self.skip_ws();
                if self.peek() != Some('[') {
                    return Err(self.error("expected ["));
                }
                if self.peek_at(1) == Some('#') {
                    self.pos += 1;
                    items.push(self.parse_table_item()?);
                    if !self.eat(']') {
                        return Err(self.error("expected ]"));
                    }
                } else if cols == TableCols::All {
                    cols = self.parse_column_range()?;
                } else {
                    return Err(self.error("only one column or column range is allowed"));
                }
                self.skip_ws();
                match self.peek() {
                    Some(c) if c == self.locale.arg_sep || c == ',' => self.pos += 1,
                    _ => break,
                }
            },
            _ => {
                // A plain column name keeps its spaces.
                self.pos = save;
                cols = TableCols::One(self.parse_column_name()?);
            }
        }
        self.skip_ws();
        if !self.eat(']') {
            return Err(self.error("expected ]"));
        }
        let Some(rows) = TableRows::from_items(&items) else {
            return Err(self.error("these table items cannot be combined"));
        };
        if rows == TableRows::Unspecified && cols == TableCols::All {
            // `Table1[]` is the table itself.
            let Some(name) = table else {
                return Err(self.error("expected a column name"));
            };
            self.push_span(span_start, Token::Name(None, name.clone()));
            return Ok(Expr::Name(name));
        }
        let reference = TableRef { table, rows, cols };
        self.push_span(span_start, Token::Table(reference.clone()));
        Ok(Expr::Table(reference))
    }

    /// `#Headers` and the like, up to (not including) the closing bracket.
    fn parse_table_item(&mut self) -> Result<TableItem, ParseError> {
        debug_assert_eq!(self.peek(), Some('#'));
        let start = self.pos;
        while self.peek().is_some_and(|c| c != ']' && c != '[') {
            self.pos += 1;
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        match self.locale.parse_table_item(&text) {
            Some(index) => Ok(TableItem::ALL[index]),
            None => {
                self.pos = start;
                Err(self.error("unknown table item"))
            }
        }
    }

    /// A column name up to (not including) the closing bracket. `'` takes
    /// the next character literally.
    fn parse_column_name(&mut self) -> Result<Arc<str>, ParseError> {
        let mut name = String::new();
        loop {
            match self.peek() {
                None => return Err(self.error("expected ]")),
                Some(']') => break,
                Some('[') => return Err(self.error("unexpected [ in a column name")),
                Some('\'') => {
                    let Some(c) = self.peek_at(1) else {
                        return Err(self.error("expected ]"));
                    };
                    name.push(c);
                    self.pos += 2;
                }
                Some(c) => {
                    name.push(c);
                    self.pos += 1;
                }
            }
        }
        if name.is_empty() {
            return Err(self.error("expected a column name"));
        }
        Ok(Arc::from(name))
    }

    /// `[Col]` or `[Col1]:[Col2]`.
    fn parse_column_range(&mut self) -> Result<TableCols, ParseError> {
        let one = |p: &mut Self| -> Result<Arc<str>, ParseError> {
            if !p.eat('[') {
                return Err(p.error("expected ["));
            }
            let name = p.parse_column_name()?;
            if !p.eat(']') {
                return Err(p.error("expected ]"));
            }
            Ok(name)
        };
        let first = one(self)?;
        if !self.eat(':') {
            return Ok(TableCols::One(first));
        }
        Ok(TableCols::Range(first, one(self)?))
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        self.skip_ws();
        if self.eat(')') {
            return Ok(args);
        }
        loop {
            self.skip_ws();
            match self.peek() {
                Some(c) if c == self.locale.arg_sep => {
                    args.push(Expr::Missing);
                    self.pos += 1;
                    continue;
                }
                Some(')') => {
                    args.push(Expr::Missing);
                    self.pos += 1;
                    return Ok(args);
                }
                None => {
                    if self.lenient {
                        return Ok(args);
                    }
                    return Err(self.error("expected )"));
                }
                _ => {}
            }
            args.push(self.parse_expr()?);
            self.skip_ws();
            match self.peek() {
                Some(c) if c == self.locale.arg_sep => {
                    self.pos += 1;
                }
                Some(')') => {
                    self.pos += 1;
                    return Ok(args);
                }
                None if self.lenient => return Ok(args),
                _ => return Err(self.error("expected ) or argument separator")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(src: &str) -> String {
        parse_formula(src, Locale::en())
            .unwrap()
            .to_text(Locale::en())
    }

    #[test]
    fn roundtrip() {
        assert_eq!(rt("1+2*3"), "1+2*3");
        assert_eq!(rt("sum(A1:B2, 3)"), "SUM(A1:B2,3)");
        assert_eq!(rt("$A$1+B$2+$C3"), "$A$1+B$2+$C3");
        assert_eq!(
            rt("Sheet2!A1+'My Sheet'!B2:C3"),
            "Sheet2!A1+'My Sheet'!B2:C3"
        );
        assert_eq!(rt("A:A+1:1"), "A:A+1:1");
        assert_eq!(rt("\"a\"\"b\"&\"c\""), "\"a\"\"b\"&\"c\"");
        assert_eq!(rt("{1,2;3,4}"), "{1,2;3,4}");
        assert_eq!(rt("-A1%"), "-A1%");
        assert_eq!(rt("IF(A1>=2,,\"x\")"), "IF(A1>=2,,\"x\")");
        assert_eq!(rt("LOG10(100)"), "LOG10(100)");
        assert_eq!(rt("1.5e3"), "1500");
        assert_eq!(rt("#N/A"), "#N/A");
        assert_eq!(rt("TRUE"), "TRUE");
        assert_eq!(rt("(1+2)^2"), "(1+2)^2");
    }

    #[test]
    fn french() {
        let fr = Locale::fr();
        let e = parse_formula("SOMME(A1:A3;1,5)", fr).unwrap();
        assert_eq!(e.to_text(Locale::en()), "SUM(A1:A3,1.5)");
        assert_eq!(e.to_text(fr), "SOMME(A1:A3;1,5)");
        let e = parse_formula("SI(A1>0;VRAI;\"non\")", fr).unwrap();
        assert_eq!(e.to_text(Locale::en()), "IF(A1>0,TRUE,\"non\")");
        let e = parse_formula("NB.SI(A:A;\">1\")", fr).unwrap();
        assert_eq!(e.to_text(Locale::en()), "COUNTIF(A:A,\">1\")");
        assert!(parse_formula("SOMME(1;", fr).is_err());
    }

    #[test]
    fn shifting() {
        let e = parse_formula("A1+$B$2+C$3+SUM($D4:E5)", Locale::en()).unwrap();
        assert_eq!(
            e.shifted(1, 1).to_text(Locale::en()),
            "B2+$B$2+D$3+SUM($D5:F6)"
        );
        let e = parse_formula("A1", Locale::en()).unwrap();
        assert_eq!(e.shifted(-1, 0).to_text(Locale::en()), "#REF!");
    }

    #[test]
    fn spans() {
        let spans = scan_refs("SUM(A1:B2)+C3*", Locale::en());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].range, 4..9);
        assert_eq!(spans[1].range, 11..13);
    }

    /// Parse in `locale`, print in English.
    fn canon(src: &str, locale: &Locale) -> String {
        parse_formula(src, locale)
            .unwrap_or_else(|e| panic!("{src}: {e:?}"))
            .to_text(Locale::en())
    }

    /// The formula (English) survives printing and parsing in every
    /// locale, and printing and parsing in the syntax of files.
    fn survives(src: &str) -> Expr {
        let e = parse_formula(src, Locale::en()).unwrap_or_else(|e| panic!("{src}: {e:?}"));
        for locale in Locale::all() {
            let text = e.to_text(locale);
            let back = parse_formula(&text, locale)
                .unwrap_or_else(|err| panic!("{src} as {text} ({}): {err:?}", locale.id));
            assert_eq!(back, e, "{src} as {text} ({})", locale.id);
        }
        let file = e.to_file_text();
        assert_eq!(
            parse_formula(&file, Locale::en()).as_ref(),
            Ok(&e),
            "{src} as {file} (file)"
        );
        e
    }

    #[test]
    fn spilled_ranges() {
        assert_eq!(rt("A1#"), "A1#");
        assert_eq!(
            rt("sum(Sheet2!$B$2#)+'My Sheet'!a1#"),
            "SUM(Sheet2!$B$2#)+'My Sheet'!A1#"
        );
        assert_eq!(rt("Totals#"), "Totals#");
        assert_eq!(
            canon("SOMME(A1#;Feuil2!B2#)", Locale::fr()),
            "SUM(A1#,Feuil2!B2#)"
        );
        assert_eq!(
            survives("SUM(A1#)*Sheet2!A1#").to_text(Locale::fr()),
            "SOMME(A1#)*Sheet2!A1#"
        );
        // Files and older versions spell the operator as a function.
        assert_eq!(rt("_xlfn.ANCHORARRAY(A1)"), "A1#");
        assert_eq!(rt("ANCHORARRAY(Sheet2!A1)+1"), "Sheet2!A1#+1");
        assert_eq!(survives("A1#").to_file_text(), "_xlfn.ANCHORARRAY(A1)");
        // Only something that can anchor an array takes the operator.
        for bad in ["1#", "\"a\"#", "A1##", "A1 #", "#", "A1#B1", "{1,2}#"] {
            assert!(parse_formula(bad, Locale::en()).is_err(), "{bad}");
        }
    }

    #[test]
    fn three_d_references() {
        assert_eq!(rt("sum(Sheet1:Sheet3!a1:b2)"), "SUM(Sheet1:Sheet3!A1:B2)");
        assert_eq!(
            rt("'Jan 24:Mar 24'!$A$1+'It''s:B'!C:C"),
            "'Jan 24:Mar 24'!$A$1+'It''s:B'!C:C"
        );
        assert_eq!(rt("SUM('Sheet1:Sheet 3'!1:2)"), "SUM('Sheet1:Sheet 3'!1:2)");
        // A span of one sheet is that sheet.
        assert_eq!(rt("Sheet1:Sheet1!A1"), "Sheet1!A1");
        assert_eq!(rt("'Sheet1:sheet1'!A1"), "Sheet1!A1");
        assert_eq!(
            canon("MOYENNE(Feuil1:Feuil3!A1:A3;'Été:Hiver'!B2)", Locale::fr()),
            "AVERAGE(Feuil1:Feuil3!A1:A3,Été:Hiver!B2)"
        );
        let e = survives("SUM(Sheet1:Sheet3!A1:B2,'A b:C d'!$A:$B)");
        assert_eq!(
            e.to_text(Locale::fr()),
            "SOMME(Sheet1:Sheet3!A1:B2;'A b:C d'!$A:$B)"
        );
        match parse_formula("Sheet1:Sheet3!B2", Locale::en()).unwrap() {
            Expr::Ref3D(r) => {
                assert_eq!((&*r.first, &*r.last), ("Sheet1", "Sheet3"));
                assert_eq!(r.cells.to_text(), "B2");
            }
            other => panic!("{other:?}"),
        }
        // A plain range is not a span of sheets, a path is not either.
        assert!(matches!(
            parse_formula("A1:B2", Locale::en()).unwrap(),
            Expr::Ref(_)
        ));
        assert!(matches!(
            parse_formula("'C:\\dir\\[Book.xlsx]Sheet1'!A1", Locale::en()).unwrap(),
            Expr::Ref(_)
        ));
        for bad in [
            "Sheet1:Sheet3!",
            "Sheet1:Sheet3!Name",
            "Sheet1:!A1",
            "'Sheet1:Sheet3'A1",
            "Sheet1:Sheet3!#REF!",
        ] {
            assert!(parse_formula(bad, Locale::en()).is_err(), "{bad}");
        }
    }

    #[test]
    fn unions_and_intersections() {
        assert_eq!(rt("SUM((A1:A3,C1:C3))"), "SUM((A1:A3,C1:C3))");
        assert_eq!(
            rt("sum( ( a1 , b2:c3 , Sheet2!d4 ) )"),
            "SUM((A1,B2:C3,Sheet2!D4))"
        );
        assert_eq!(rt("A1:C3 B2:D4"), "A1:C3 B2:D4");
        assert_eq!(rt("SUM(A1:C3   B2:D4 C:C)"), "SUM(A1:C3 B2:D4 C:C)");
        assert_eq!(rt("(A1,B2) (A1:B9)"), "(A1,B2) (A1:B9)");
        assert_eq!(rt("A:A 1:1"), "A:A 1:1");
        assert_eq!(rt("Prices Sheet2!B:B"), "Prices Sheet2!B:B");
        // Spaces around operators are still just spaces.
        assert_eq!(rt("A1 + B1 * ( C1 - 2 ) & \" x\""), "A1+B1*(C1-2)&\" x\"");
        assert_eq!(rt("SUM( A1 , B1 )"), "SUM(A1,B1)");
        // At the top level (the definition of a name) a union needs no
        // parentheses.
        assert_eq!(
            rt("Sheet1!$A$1:$A$3,Sheet1!$C$1:$C$3"),
            "Sheet1!$A$1:$A$3,Sheet1!$C$1:$C$3"
        );
        let fr = Locale::fr();
        assert_eq!(canon("SOMME((A1:A3;C1:C3))", fr), "SUM((A1:A3,C1:C3))");
        assert_eq!(canon("SOMME(A1:C3 B2:D4)", fr), "SUM(A1:C3 B2:D4)");
        assert_eq!(canon("NB((A1;B2) A:A)", fr), "COUNT((A1,B2) A:A)");
        let e = survives(
            "SUM((A1:A3,C1:C3))+COUNT(A1:C3 B2:D4)+AREAS((A1,B2,C3))+INDEX((A1:B2,D1:E2),1,1,2)",
        );
        assert_eq!(
            e.to_text(fr),
            "SOMME((A1:A3;C1:C3))+NB(A1:C3 B2:D4)+ZONES((A1;B2;C3))+INDEX((A1:B2;D1:E2);1;1;2)"
        );
        survives("SUM(Sheet1!A1 Sheet1!A1:B2,(A:A,1:1) B2:C3)");
        // Only references can be joined or intersected.
        for bad in [
            "(1,2)",
            "1,2",
            "SUM((A1,2))",
            "A1 2",
            "A1 \"x\"",
            "A1 TRUE",
            "(A1,)",
            "(,A1)",
            "A1,",
            "2 A1",
        ] {
            assert!(parse_formula(bad, Locale::en()).is_err(), "{bad}");
        }
        assert!(
            parse_formula("SOMME((A1,B2))", fr).is_err(),
            "the French separator is ;"
        );
    }

    #[test]
    fn implicit_intersection() {
        assert_eq!(rt("@A1:A3"), "@A1:A3");
        assert_eq!(rt("-@A1:A3%+@Rate*@SUM(B:B)"), "-@A1:A3%+@Rate*@SUM(B:B)");
        assert_eq!(rt("@ @A1"), "@A1");
        assert_eq!(rt("@Sheet2!A:A & @A1#"), "@Sheet2!A:A&@A1#");
        assert_eq!(rt("_xlfn.SINGLE(A1:A3)"), "@A1:A3");
        // `@` binds tighter than the operators an argument may hold.
        assert_eq!(rt("SINGLE(A1:A3*2)+1"), "@(A1:A3*2)+1");
        assert_eq!(
            canon("@A1:A3+@SOMME(B:B)", Locale::fr()),
            "@A1:A3+@SUM(B:B)"
        );
        let e = survives("@A1:A3*@Table1[Qty]+@(B1:B3&\"x\")");
        assert_eq!(
            e.to_file_text(),
            "_xlfn.SINGLE(A1:A3)*_xlfn.SINGLE(Table1[Qty])+_xlfn.SINGLE((B1:B3&\"x\"))"
        );
        for bad in ["@", "A1@", "@+", "@)"] {
            assert!(parse_formula(bad, Locale::en()).is_err(), "{bad}");
        }
    }

    #[test]
    fn structured_references() {
        let en = Locale::en();
        let fr = Locale::fr();
        // (as typed, as shown in English, as shown in French, as stored in files)
        let cases = [
            ("Table1[Col]", "Table1[Col]", "Table1[Col]", "Table1[Col]"),
            (
                "Table1[[Col1]:[Col2]]",
                "Table1[[Col1]:[Col2]]",
                "Table1[[Col1]:[Col2]]",
                "Table1[[Col1]:[Col2]]",
            ),
            (
                "Table1[#All]",
                "Table1[#All]",
                "Table1[#Tout]",
                "Table1[#All]",
            ),
            (
                "Table1[#Headers]",
                "Table1[#Headers]",
                "Table1[#En-têtes]",
                "Table1[#Headers]",
            ),
            (
                "Table1[#Data]",
                "Table1[#Data]",
                "Table1[#Données]",
                "Table1[#Data]",
            ),
            (
                "Table1[#Totals]",
                "Table1[#Totals]",
                "Table1[#Totaux]",
                "Table1[#Totals]",
            ),
            ("Table1[@]", "Table1[@]", "Table1[@]", "Table1[#This Row]"),
            (
                "Table1[#This Row]",
                "Table1[@]",
                "Table1[@]",
                "Table1[#This Row]",
            ),
            (
                "Table1[@Col]",
                "Table1[@Col]",
                "Table1[@Col]",
                "Table1[[#This Row],[Col]]",
            ),
            (
                "Table1[[#This Row],[Col]]",
                "Table1[@Col]",
                "Table1[@Col]",
                "Table1[[#This Row],[Col]]",
            ),
            (
                "Table1[@[Col 1]:[Col 2]]",
                "Table1[@[Col 1]:[Col 2]]",
                "Table1[@[Col 1]:[Col 2]]",
                "Table1[[#This Row],[Col 1]:[Col 2]]",
            ),
            (
                "Table1[@Col 1]",
                "Table1[@[Col 1]]",
                "Table1[@[Col 1]]",
                "Table1[[#This Row],[Col 1]]",
            ),
            (
                "Table1[[#Headers],[Col]]",
                "Table1[[#Headers],[Col]]",
                "Table1[[#En-têtes];[Col]]",
                "Table1[[#Headers],[Col]]",
            ),
            (
                "Table1[[#Headers],[#Data]]",
                "Table1[[#Headers],[#Data]]",
                "Table1[[#En-têtes];[#Données]]",
                "Table1[[#Headers],[#Data]]",
            ),
            (
                "Table1[ [#Data], [#Totals], [A]:[B] ]",
                "Table1[[#Data],[#Totals],[A]:[B]]",
                "Table1[[#Données];[#Totaux];[A]:[B]]",
                "Table1[[#Data],[#Totals],[A]:[B]]",
            ),
            (
                "Table1[[#All],[Sales Amount]]",
                "Table1[[#All],[Sales Amount]]",
                "Table1[[#Tout];[Sales Amount]]",
                "Table1[[#All],[Sales Amount]]",
            ),
            (
                "Table1[Sales Amount]",
                "Table1[Sales Amount]",
                "Table1[Sales Amount]",
                "Table1[Sales Amount]",
            ),
            // Without a table name: the table holding the formula.
            ("[Col]", "[Col]", "[Col]", "[Col]"),
            ("[@Col]", "[@Col]", "[@Col]", "[[#This Row],[Col]]"),
            ("[@]", "[@]", "[@]", "[#This Row]"),
            ("[#Headers]", "[#Headers]", "[#En-têtes]", "[#Headers]"),
            (
                "[[#Totals],[Col]]",
                "[[#Totals],[Col]]",
                "[[#Totaux];[Col]]",
                "[[#Totals],[Col]]",
            ),
            // Special characters: brackets of their own, `'` as escape.
            (
                "Table1[[Total $]]",
                "Table1[[Total $]]",
                "Table1[[Total $]]",
                "Table1[[Total $]]",
            ),
            (
                "Table1[@[Total $]]",
                "Table1[@[Total $]]",
                "Table1[@[Total $]]",
                "Table1[[#This Row],[Total $]]",
            ),
            (
                "Table1['#Items]",
                "Table1[['#Items]]",
                "Table1[['#Items]]",
                "Table1[['#Items]]",
            ),
            (
                "Table1[[a'[b']]]",
                "Table1[[a'[b']]]",
                "Table1[[a'[b']]]",
                "Table1[[a'[b']]]",
            ),
            (
                "Table1[[It''s]]",
                "Table1[[It''s]]",
                "Table1[[It''s]]",
                "Table1[[It''s]]",
            ),
            (
                "Table1[['@home]]",
                "Table1[['@home]]",
                "Table1[['@home]]",
                "Table1[['@home]]",
            ),
            (
                "Table1[[a,b]:[c;d]]",
                "Table1[[a,b]:[c;d]]",
                "Table1[[a,b]:[c;d]]",
                "Table1[[a,b]:[c;d]]",
            ),
            // The table itself.
            ("Table1[]", "Table1", "Table1", "Table1"),
        ];
        for (typed, shown_en, shown_fr, file) in cases {
            let e = survives(typed);
            assert_eq!(e.to_text(en), shown_en, "{typed}");
            assert_eq!(e.to_text(fr), shown_fr, "{typed}");
            assert_eq!(e.to_file_text(), file, "{typed}");
        }
        // The names are kept exactly as written.
        match parse_formula("Sales['#Items sold']]", en).unwrap() {
            Expr::Table(t) => {
                assert_eq!(t.table.as_deref(), Some("Sales"));
                assert_eq!(t.rows, TableRows::Unspecified);
                assert_eq!(t.cols, TableCols::One(Arc::from("#Items sold]")));
            }
            other => panic!("{other:?}"),
        }
        // French: localized items, `;` between the parts; the English
        // spellings and the comma are understood too.
        assert_eq!(
            canon("SOMME(Ventes[[#Tout];[Qté]])", fr),
            "SUM(Ventes[[#All],[Qté]])"
        );
        assert_eq!(
            canon("Ventes[[#cette ligne];[Qté]]*[@Prix]", fr),
            "Ventes[@Qté]*[@Prix]"
        );
        assert_eq!(
            canon("Ventes[[#En-têtes];[#Données];[Qté]:[Prix]]", fr),
            "Ventes[[#Headers],[#Data],[Qté]:[Prix]]"
        );
        assert_eq!(
            canon("Ventes[#Totaux]&Ventes[#données]&Ventes[#TOUT]", fr),
            "Ventes[#Totals]&Ventes[#Data]&Ventes[#All]"
        );
        assert_eq!(
            canon("Ventes[[#Headers],[Qté]]", fr),
            "Ventes[[#Headers],[Qté]]"
        );
        // The other locales.
        let all = parse_formula(
            "T[[#All],[a]]+T[#Data]+T[[#Headers],[#Data]]+T[#Totals]+T[@a]",
            en,
        )
        .unwrap();
        for (locale, text) in [
            (
                Locale::es(),
                "T[[#Todo];[a]]+T[#Datos]+T[[#Encabezados];[#Datos]]+T[#Totales]+T[@a]",
            ),
            (
                Locale::de(),
                "T[[#Alle];[a]]+T[#Daten]+T[[#Kopfzeilen];[#Daten]]+T[#Ergebnisse]+T[@a]",
            ),
            (
                Locale::it(),
                "T[[#Tutti];[a]]+T[#Dati]+T[[#Intestazioni];[#Dati]]+T[#Totali]+T[@a]",
            ),
        ] {
            assert_eq!(all.to_text(locale), text);
            assert_eq!(parse_formula(text, locale).as_ref(), Ok(&all), "{text}");
        }
        assert_eq!(canon("T[[#Diese Zeile];[a]]", Locale::de()), "T[@a]");
        assert_eq!(
            canon("T[[#Esta fila];[a]]+T[#Todas]", Locale::es()),
            "T[@a]+T[#All]"
        );
        assert_eq!(canon("T[[#Questa riga];[a]]", Locale::it()), "T[@a]");

        for bad in [
            "Table1[",
            "Table1[Col",
            "Table1[[Col]",
            "Table1[[Col]:]",
            "Table1[[A]:[B]:[C]]",
            "Table1[[A],[B]]",
            "Table1[#Nope]",
            "Table1[[#Headers],[#Totals]]",
            "Table1[[#This Row],[#Data]]",
            "Table1[[#All],[#Data]]",
            "Table1[a[b]",
            "Table1[[#Headers]",
            "Table1[[#Headers],]",
            "Table1[@[A]:]",
            "Table1[']",
            "[]",
            "[",
            "[@",
            "[[#Data],",
            "Table1[Col]]",
            "Table1[Col]x",
        ] {
            assert!(parse_formula(bad, en).is_err(), "{bad}");
        }
    }

    #[test]
    fn errors_and_broken_references() {
        // Every error value, alone and inside what is left of a reference,
        // in every locale.
        let e = survives(
            "Sheet2!#REF!+#REF!A1+SUM(#REF!$A$1:B2,'My sheet'!#REF!,#REF!A:A)+#N/A+#NULL!+#DIV/0!+#VALUE!+#NAME?+#NUM!+#SPILL!+#CALC!",
        );
        assert_eq!(
            e.to_text(Locale::fr()),
            "Sheet2!#REF!+#REF!A1+SOMME(#REF!$A$1:B2;'My sheet'!#REF!;#REF!A:A)+#N/A+#NUL!+#DIV/0!+#VALEUR!+#NOM?+#NOMBRE!+#PROPAGATION!+#CALC!"
        );
        assert_eq!(
            e.to_text(Locale::de()),
            "Sheet2!#BEZUG!+#BEZUG!A1+SUMME(#BEZUG!$A$1:B2;'My sheet'!#BEZUG!;#BEZUG!A:A)+#NV+#NULL!+#DIV/0!+#WERT!+#NAME?+#ZAHL!+#ÜBERLAUF!+#KALK!"
        );
        assert_eq!(
            e.to_text(Locale::es()),
            "Sheet2!#¡REF!+#¡REF!A1+SUMA(#¡REF!$A$1:B2;'My sheet'!#¡REF!;#¡REF!A:A)+#N/D+#¡NULO!+#¡DIV/0!+#¡VALOR!+#¿NOMBRE?+#¡NUM!+#¡DESBORDAMIENTO!+#¡CALC!"
        );
        assert_eq!(
            e.to_text(Locale::it()),
            "Sheet2!#RIF!+#RIF!A1+SOMMA(#RIF!$A$1:B2;'My sheet'!#RIF!;#RIF!A:A)+#N/D+#NULLO!+#DIV/0!+#VALORE!+#NOME?+#NUM!+#ESPANSIONE!+#CALC!"
        );
        // The English spellings are read in every locale, in any case.
        assert_eq!(
            canon("#ref!+Feuil2!#REF!+#REF!a1+#valeur!+#VALUE!", Locale::fr()),
            "#REF!+Feuil2!#REF!+#REF!A1+#VALUE!+#VALUE!"
        );
        assert_eq!(
            canon("#bezug!A1+Tabelle2!#Bezug!+#überlauf!", Locale::de()),
            "#REF!A1+Tabelle2!#REF!+#SPILL!"
        );
        // What is left of an intersection, a union and a spilled range.
        survives("SUM(A1:C3 #REF!)+SUM((A1,#REF!))+#REF! #REF!+@#REF!");
        match parse_formula("#REF!$B$2:C3", Locale::en()).unwrap() {
            Expr::Ref(r) => {
                assert!(r.sheet_deleted());
                assert_eq!(r.to_text(), "#REF!$B$2:C3");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            parse_formula("Sheet2!#REF!", Locale::en()),
            Ok(Expr::RefError(Arc::from("Sheet2")))
        );
        assert_eq!(
            parse_formula("Sheet2!#N/A", Locale::en()),
            Ok(Expr::Error(ErrorKind::NA))
        );
        for bad in [
            "#",
            "#REF",
            "#NOPE!",
            "Sheet2!#",
            "Sheet2!#NOPE!",
            "#REF!!",
            "Sheet2!",
            "Sheet2!!A1",
            "'Sheet2!A1",
            "''",
        ] {
            assert!(parse_formula(bad, Locale::en()).is_err(), "{bad}");
        }
    }

    #[test]
    fn sheet_qualified_names() {
        assert_eq!(rt("Sheet1!Rate*2"), "Sheet1!Rate*2");
        assert_eq!(
            rt("'My Sheet'!Tax.2024+sheet2!_x"),
            "'My Sheet'!Tax.2024+sheet2!_x"
        );
        assert_eq!(
            parse_formula("Sheet1!Rate", Locale::en()),
            Ok(Expr::SheetName(Arc::from("Sheet1"), Arc::from("Rate")))
        );
        survives("SUM(Sheet1!Prices)+'A b'!Rate#");
        assert_eq!(
            canon("SOMME(Feuil1!Prix)", Locale::fr()),
            "SUM(Feuil1!Prix)"
        );
    }

    #[test]
    fn spans_of_the_new_references() {
        let fr = Locale::fr();
        let src = "SOMME(A1#;Feuil1:Feuil3!C1;Ventes[Qté];(A1:A2;B5) $C$1:C2;'Feuil 2'!A1#)";
        let spans = scan_refs(src, fr);
        let found: Vec<(&str, String, Option<&str>)> = spans
            .iter()
            .map(|s| {
                (
                    &src[s.range.clone()],
                    s.reference.to_text(),
                    s.last_sheet.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            found,
            [
                ("A1#", "A1".to_string(), None),
                ("Feuil1:Feuil3!C1", "Feuil1!C1".to_string(), Some("Feuil3")),
                ("A1:A2", "A1:A2".to_string(), None),
                ("B5", "B5".to_string(), None),
                ("$C$1:C2", "$C$1:C2".to_string(), None),
                ("'Feuil 2'!A1#", "'Feuil 2'!A1".to_string(), None),
            ]
        );
        // Unfinished input still yields what is there.
        let found: Vec<String> = scan_refs("SUM(A1:B2 C3;Table1[Co", Locale::en())
            .iter()
            .map(|s| s.reference.to_text())
            .collect();
        assert_eq!(found, ["A1:B2", "C3"]);
        assert_eq!(scan_refs("=@A1+B2#+Sheet1:Sheet2!", Locale::en()).len(), 2);
    }

    #[test]
    fn malformed_input_is_an_error_never_a_panic() {
        // Nesting has a limit, whatever nests.
        for n in [255usize, 256, 5_000, 100_000] {
            for (open, close) in [
                ("(", ")"),
                ("SUM(", ")"),
                ("-", ""),
                ("(-(", "))"),
                ("@(", ")"),
            ] {
                let src = format!("{}A1{}", open.repeat(n), close.repeat(n));
                if let Ok(e) = parse_formula(&src, Locale::en()) {
                    assert!(n < 256, "{open} x {n}");
                    assert_eq!(parse_formula(&e.to_text(Locale::en()), Locale::en()), Ok(e));
                }
            }
        }
        assert!(parse_formula(&"@".repeat(100_000), Locale::en()).is_err());
        assert!(parse_formula(&format!("{}A1", "@".repeat(100_000)), Locale::en()).is_ok());
        assert!(parse_formula(&"[".repeat(100_000), Locale::en()).is_err());
        assert!(parse_formula(&"Table1[[#All],".repeat(10_000), Locale::en()).is_err());

        // Every prefix and every single-character deletion of formulas
        // using each syntax, in every locale: parsing, printing what parsed
        // and scanning for references must not panic.
        let sources = [
            "SUM(A1#,Sheet2!$B$2#)+'My Sheet'!A1#",
            "SUM(Sheet1:Sheet3!A1:B2,'Jan 24:Mar 24'!$A:$B)",
            "SUM((A1:A3,C1:C3)) + COUNT(A1:C3 B2:D4) - @A1:A3%",
            "Table1[[#Headers],[#Data],[Col 1]:[Col '[2']]]*[@[Total $]]+Table1[@]&T[#All]",
            "Sheet2!#REF!+#REF!$A$1:B2+#N/A+Sheet1!Name+{1,2;\"a\",#DIV/0!}",
            "Ventes[[#Tout];[Qté]]+SOMME((A1;B2) C:C)+#VALEUR!+1,5",
            "Été!A1+'Ça:Là'!B2+T[[#Diese Zeile];[ä]]+#ÜBERLAUF!+#¡REF!A1",
        ];
        for src in sources {
            let chars: Vec<char> = src.chars().collect();
            let mut variants: Vec<String> = (0..=chars.len())
                .map(|n| chars[..n].iter().collect())
                .collect();
            variants.extend((0..chars.len()).map(|skip| {
                chars
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != skip)
                    .map(|(_, c)| *c)
                    .collect::<String>()
            }));
            for locale in Locale::all() {
                for v in &variants {
                    if let Ok(e) = parse_formula(v, locale) {
                        let text = e.to_text(locale);
                        assert_eq!(
                            parse_formula(&text, locale).as_ref(),
                            Ok(&e),
                            "{v} printed as {text} ({})",
                            locale.id
                        );
                        e.to_file_text();
                    }
                    for span in scan_refs(v, locale) {
                        assert!(v.get(span.range).is_some(), "{v}");
                    }
                }
            }
        }
        for junk in [
            "",
            " ",
            "=",
            "!",
            "'",
            "''!",
            "$",
            "$A",
            "A$",
            ":",
            "A1:",
            ":A1",
            "A1::B2",
            "!A1",
            "A1!B2!C3",
            "\u{0}",
            "é#",
            "#é",
            "]",
            "@]",
            "A1 @",
            "(A1 )B1",
            "1 :3",
            "'a:b:c'!A1",
            "':'!A1",
            "'a:'!A1",
        ] {
            for locale in Locale::all() {
                let _ = parse_formula(junk, locale);
                let _ = scan_refs(junk, locale);
            }
        }
    }
}
