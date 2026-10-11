//! Formula parser (locale aware, scannerless recursive descent).
//!
//! Precedence, lowest to highest:
//! comparison, `&`, `+ -`, `* /`, `^`, unary `- +`, postfix `%`.

use crate::ast::{BinaryOp, CellRef, Expr, RefExpr, RefKind, UnaryOp};
use crate::locale::Locale;
use crate::refs::{MAX_ROWS, letters_to_col};
use crate::value::ErrorKind;
use std::ops::Range;
use std::sync::Arc;

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
    pub reference: RefExpr,
}

/// Parse formula text (without the leading `=`).
pub fn parse_formula(src: &str, locale: &Locale) -> Result<Expr, ParseError> {
    let mut p = Parser::new(src, locale);
    let e = p.parse_expr()?;
    p.skip_ws();
    if p.pos < p.chars.len() {
        return Err(p.error("unexpected character"));
    }
    Ok(e)
}

/// Find the references in (possibly incomplete) formula text. Used to
/// colour ranges while the user is typing.
pub fn scan_refs(src: &str, locale: &Locale) -> Vec<RefSpan> {
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
    spans: Vec<RefSpan>,
    lenient: bool,
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '\\'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '\\' || c == '?'
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

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_cmp()
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
        match self.peek() {
            Some('-') => {
                self.pos += 1;
                Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.parse_unary()?)))
            }
            Some('+') => {
                self.pos += 1;
                Ok(Expr::Unary(UnaryOp::Plus, Box::new(self.parse_unary()?)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_primary()?;
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

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        self.skip_ws();
        let Some(c) = self.peek() else {
            return Err(self.error("unexpected end of formula"));
        };
        match c {
            '(' => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.skip_ws();
                if !self.eat(')') {
                    return Err(self.error("expected )"));
                }
                Ok(Expr::Paren(Box::new(e)))
            }
            '"' => Ok(Expr::Text(Arc::from(self.parse_string()?))),
            '{' => self.parse_array(),
            '#' => self.parse_error_literal().map(Expr::Error),
            '@' => {
                self.pos += 1;
                self.parse_primary()
            }
            '\'' => self.parse_ident_or_ref(),
            '$' => self.parse_ident_or_ref(),
            c if c.is_ascii_digit() => {
                if let Some(r) = self.try_parse_ref(None, self.pos) {
                    return Ok(r);
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
        s.parse::<f64>()
            .map_err(|_| self.error("invalid number"))
    }

    fn parse_error_literal(&mut self) -> Result<ErrorKind, ParseError> {
        let rest: String = self.chars[self.pos..].iter().collect();
        let upper = rest.to_uppercase();
        let mut best: Option<(usize, ErrorKind)> = None;
        for (i, kind) in ErrorKind::ALL.iter().enumerate() {
            for name in [self.locale.errors[i], kind.as_str()] {
                if upper.starts_with(name) {
                    let len = name.chars().count();
                    if best.is_none_or(|(l, _)| len > l) {
                        best = Some((len, *kind));
                    }
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

    /// Try to parse a cell / range / whole-column / whole-row reference at
    /// the current position. `span_start` is where the token (including any
    /// sheet prefix) began.
    fn try_parse_ref(&mut self, sheet: Option<Arc<str>>, span_start: usize) -> Option<Expr> {
        let i = self.pos;
        let mut found: Option<(RefExpr, usize)> = None;

        if let Some((start, j)) = self.scan_cell(i) {
            // A1 or A1:B2
            if self.chars.get(j) == Some(&':') {
                if let Some((end, k)) = self.scan_cell(j + 1) {
                    if self.ends_token(k) {
                        found = Some((
                            RefExpr {
                                sheet: sheet.clone(),
                                start,
                                end: Some(end),
                                kind: RefKind::Cells,
                            },
                            k,
                        ));
                    }
                }
            }
            if found.is_none() && self.ends_token(j) {
                found = Some((
                    RefExpr {
                        sheet: sheet.clone(),
                        start,
                        end: None,
                        kind: RefKind::Cells,
                    },
                    j,
                ));
            }
        }
        if found.is_none() {
            // A:C
            if let Some((c0, abs0, j)) = self.scan_col(i) {
                if self.chars.get(j) == Some(&':') {
                    if let Some((c1, abs1, k)) = self.scan_col(j + 1) {
                        if self.ends_token(k) && !self.chars.get(k).is_some_and(|c| c.is_ascii_digit()) {
                            found = Some((
                                RefExpr {
                                    sheet: sheet.clone(),
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
                    }
                }
            }
        }
        if found.is_none() {
            // 1:3
            if let Some((r0, abs0, j)) = self.scan_row(i) {
                if self.chars.get(j) == Some(&':') {
                    if let Some((r1, abs1, k)) = self.scan_row(j + 1) {
                        if self.ends_token(k) {
                            found = Some((
                                RefExpr {
                                    sheet: sheet.clone(),
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
                    }
                }
            }
        }

        let (r, end) = found?;
        self.pos = end;
        self.spans.push(RefSpan {
            range: self.offsets[span_start]..self.offsets[end],
            reference: r.clone(),
        });
        Some(Expr::Ref(r))
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

    fn parse_ident_or_ref(&mut self) -> Result<Expr, ParseError> {
        let span_start = self.pos;
        let mut sheet: Option<Arc<str>> = None;

        if self.peek() == Some('\'') {
            // 'Quoted sheet name'!
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
                            break;
                        }
                    }
                    Some(c) => {
                        name.push(c);
                        self.pos += 1;
                    }
                }
            }
            if !self.eat('!') {
                return Err(self.error("expected ! after sheet name"));
            }
            sheet = Some(Arc::from(name));
        } else if self.peek() != Some('$') {
            // Unquoted sheet prefix: Name!
            let save = self.pos;
            let name = self.scan_name();
            if !name.is_empty() && self.peek() == Some('!') {
                self.pos += 1;
                sheet = Some(Arc::from(name));
            } else {
                self.pos = save;
            }
        }

        if let Some(r) = self.try_parse_ref(sheet.clone(), span_start) {
            return Ok(r);
        }
        if sheet.is_some() {
            // Sheet-scoped name or a broken reference.
            let name = self.scan_name();
            if name.is_empty() {
                return Err(self.error("expected reference"));
            }
            if name.eq_ignore_ascii_case("#REF") || name.starts_with('#') {
                return Ok(Expr::Error(ErrorKind::Ref));
            }
            return Ok(Expr::Name(Arc::from(name)));
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
            let args = self.parse_args()?;
            return Ok(Expr::Func(Arc::from(canonical), args));
        }
        if let Some(b) = self.locale.parse_bool(&name) {
            return Ok(Expr::Bool(b));
        }
        Ok(Expr::Name(Arc::from(name)))
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
        parse_formula(src, Locale::en()).unwrap().to_text(Locale::en())
    }

    #[test]
    fn roundtrip() {
        assert_eq!(rt("1+2*3"), "1+2*3");
        assert_eq!(rt("sum(A1:B2, 3)"), "SUM(A1:B2,3)");
        assert_eq!(rt("$A$1+B$2+$C3"), "$A$1+B$2+$C3");
        assert_eq!(rt("Sheet2!A1+'My Sheet'!B2:C3"), "Sheet2!A1+'My Sheet'!B2:C3");
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
        assert_eq!(e.shifted(1, 1).to_text(Locale::en()), "B2+$B$2+D$3+SUM($D5:F6)");
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
}
