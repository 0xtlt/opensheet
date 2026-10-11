//! Cell addressing: A1 notation, rectangles, resolved references.

pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLS: u32 = 16_384;

/// 0-based column index to letters: 0 → "A", 26 → "AA".
pub fn col_to_letters(col: u32) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        out.push(b'A' + rem);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Letters to 0-based column index; `None` when invalid or out of range.
pub fn letters_to_col(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut n: u32 = 0;
    for ch in s.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        n = n * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1);
    }
    if n == 0 || n > MAX_COLS {
        None
    } else {
        Some(n - 1)
    }
}

/// "A1" style name for a 0-based (row, col).
pub fn cell_name(row: u32, col: u32) -> String {
    format!("{}{}", col_to_letters(col), row + 1)
}

/// Parse a plain cell address ("B12", "$B$12") into 0-based (row, col).
pub fn parse_cell_name(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && bytes[i] == b'$' {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    let col = letters_to_col(&s[start..i])?;
    if i < bytes.len() && bytes[i] == b'$' {
        i += 1;
    }
    let digits = &s[i..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    if row == 0 || row > MAX_ROWS {
        return None;
    }
    Some((row - 1, col))
}

/// An inclusive rectangle of cells, 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    pub r0: u32,
    pub c0: u32,
    pub r1: u32,
    pub c1: u32,
}

impl Rect {
    pub fn new(r0: u32, c0: u32, r1: u32, c1: u32) -> Rect {
        Rect {
            r0: r0.min(r1),
            c0: c0.min(c1),
            r1: r0.max(r1),
            c1: c0.max(c1),
        }
    }

    pub fn cell(row: u32, col: u32) -> Rect {
        Rect {
            r0: row,
            c0: col,
            r1: row,
            c1: col,
        }
    }

    pub fn rows(&self) -> u32 {
        self.r1 - self.r0 + 1
    }

    pub fn cols(&self) -> u32 {
        self.c1 - self.c0 + 1
    }

    pub fn is_single(&self) -> bool {
        self.r0 == self.r1 && self.c0 == self.c1
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.r0 && row <= self.r1 && col >= self.c0 && col <= self.c1
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.r0 <= o.r1 && o.r0 <= self.r1 && self.c0 <= o.c1 && o.c0 <= self.c1
    }

    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        if self.intersects(o) {
            Some(Rect {
                r0: self.r0.max(o.r0),
                c0: self.c0.max(o.c0),
                r1: self.r1.min(o.r1),
                c1: self.c1.min(o.c1),
            })
        } else {
            None
        }
    }

    pub fn union(&self, o: &Rect) -> Rect {
        Rect {
            r0: self.r0.min(o.r0),
            c0: self.c0.min(o.c0),
            r1: self.r1.max(o.r1),
            c1: self.c1.max(o.c1),
        }
    }

    pub fn is_whole_cols(&self) -> bool {
        self.r0 == 0 && self.r1 == MAX_ROWS - 1
    }

    pub fn is_whole_rows(&self) -> bool {
        self.c0 == 0 && self.c1 == MAX_COLS - 1
    }

    /// "A1", "A1:B2", "A:C" or "1:3".
    pub fn name(&self) -> String {
        if self.is_whole_cols() && !self.is_whole_rows() {
            format!("{}:{}", col_to_letters(self.c0), col_to_letters(self.c1))
        } else if self.is_whole_rows() && !self.is_whole_cols() {
            format!("{}:{}", self.r0 + 1, self.r1 + 1)
        } else if self.is_single() {
            cell_name(self.r0, self.c0)
        } else {
            format!(
                "{}:{}",
                cell_name(self.r0, self.c0),
                cell_name(self.r1, self.c1)
            )
        }
    }

    /// Parse "A1", "A1:B2", "A:C" or "1:3" (with optional `$`).
    pub fn parse(s: &str) -> Option<Rect> {
        let s = s.trim().replace('$', "");
        if let Some((a, b)) = s.split_once(':') {
            if let (Some(p), Some(q)) = (parse_cell_name(a), parse_cell_name(b)) {
                return Some(Rect::new(p.0, p.1, q.0, q.1));
            }
            if let (Some(p), Some(q)) = (letters_to_col(a), letters_to_col(b)) {
                return Some(Rect::new(0, p, MAX_ROWS - 1, q));
            }
            if let (Ok(p), Ok(q)) = (a.parse::<u32>(), b.parse::<u32>()) {
                if p >= 1 && q >= 1 && p <= MAX_ROWS && q <= MAX_ROWS {
                    return Some(Rect::new(p - 1, 0, q - 1, MAX_COLS - 1));
                }
            }
            None
        } else {
            parse_cell_name(&s).map(|(r, c)| Rect::cell(r, c))
        }
    }
}

/// A fully resolved reference: a rectangle on a specific sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reference {
    pub sheet: usize,
    pub rect: Rect,
}

impl Reference {
    pub fn new(sheet: usize, rect: Rect) -> Reference {
        Reference { sheet, rect }
    }

    pub fn cell(sheet: usize, row: u32, col: u32) -> Reference {
        Reference {
            sheet,
            rect: Rect::cell(row, col),
        }
    }

    pub fn rows(&self) -> u32 {
        self.rect.rows()
    }

    pub fn cols(&self) -> u32 {
        self.rect.cols()
    }

    pub fn is_single(&self) -> bool {
        self.rect.is_single()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters() {
        assert_eq!(col_to_letters(0), "A");
        assert_eq!(col_to_letters(25), "Z");
        assert_eq!(col_to_letters(26), "AA");
        assert_eq!(col_to_letters(16383), "XFD");
        assert_eq!(letters_to_col("A"), Some(0));
        assert_eq!(letters_to_col("xfd"), Some(16383));
        assert_eq!(letters_to_col("XFE"), None);
        assert_eq!(parse_cell_name("$B$12"), Some((11, 1)));
        assert_eq!(Rect::parse("A:C").unwrap().name(), "A:C");
        assert_eq!(Rect::parse("b2:A1").unwrap().name(), "A1:B2");
    }
}
