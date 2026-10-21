//! Paste Special: the choices of the dialog that go beyond a plain paste
//! mode (`ops::PasteMode`): an arithmetic operation between what is copied
//! and what is there, skipping blank cells, transposing rows and columns,
//! and pasting a link to the copied cells.

use crate::ast::{BinaryOp, CellRef, Expr, RefExpr, RefKind};
use crate::ops::{Clip, ClipCell, PasteMode};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Style, StyleId};
use crate::value::{ErrorKind, Value};
use crate::workbook::{Cell, Content, Workbook};
use std::collections::HashMap;
use std::sync::Arc;

/// The "Paste" choices of the dialog and of the Paste drop-down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasteWhat {
    #[default]
    All,
    Formulas,
    Values,
    Formats,
    Comments,
    Validation,
    /// The workbook has one theme: the same as `All`.
    AllUsingSourceTheme,
    AllExceptBorders,
    ColumnWidths,
    FormulasAndNumberFormats,
    ValuesAndNumberFormats,
    /// Conditional formats of the copied cells join those already there,
    /// which is what a paste does here: the same as `All`.
    AllMergingConditionalFormats,
    /// "Values & Source Formatting" of the Paste drop-down.
    ValuesAndSourceFormats,
    /// "Keep Source Column Widths" of the Paste drop-down.
    KeepSourceColumnWidths,
}

impl PasteWhat {
    /// The plain paste mode that says what kind of change this is, for
    /// those who ask (protection, the features following a paste).
    pub fn mode(self) -> PasteMode {
        match self {
            PasteWhat::All
            | PasteWhat::AllUsingSourceTheme
            | PasteWhat::AllMergingConditionalFormats
            | PasteWhat::KeepSourceColumnWidths => PasteMode::All,
            PasteWhat::Formulas => PasteMode::Formulas,
            PasteWhat::Values | PasteWhat::ValuesAndSourceFormats => PasteMode::Values,
            PasteWhat::Formats => PasteMode::Formats,
            PasteWhat::Comments => PasteMode::Comments,
            PasteWhat::Validation => PasteMode::Validation,
            PasteWhat::AllExceptBorders => PasteMode::AllExceptBorders,
            PasteWhat::ColumnWidths => PasteMode::ColumnWidths,
            PasteWhat::FormulasAndNumberFormats => PasteMode::FormulasAndNumberFormats,
            PasteWhat::ValuesAndNumberFormats => PasteMode::ValuesAndNumberFormats,
        }
    }

    /// The plain pastes that make up this choice, in order.
    fn steps(self) -> &'static [PasteMode] {
        match self {
            PasteWhat::ValuesAndSourceFormats => &[PasteMode::Formats, PasteMode::Values],
            PasteWhat::KeepSourceColumnWidths => &[PasteMode::All, PasteMode::ColumnWidths],
            PasteWhat::All
            | PasteWhat::AllUsingSourceTheme
            | PasteWhat::AllMergingConditionalFormats => &[PasteMode::All],
            PasteWhat::Formulas => &[PasteMode::Formulas],
            PasteWhat::Values => &[PasteMode::Values],
            PasteWhat::Formats => &[PasteMode::Formats],
            PasteWhat::Comments => &[PasteMode::Comments],
            PasteWhat::Validation => &[PasteMode::Validation],
            PasteWhat::AllExceptBorders => &[PasteMode::AllExceptBorders],
            PasteWhat::ColumnWidths => &[PasteMode::ColumnWidths],
            PasteWhat::FormulasAndNumberFormats => &[PasteMode::FormulasAndNumberFormats],
            PasteWhat::ValuesAndNumberFormats => &[PasteMode::ValuesAndNumberFormats],
        }
    }

    /// What of the copied contents is written.
    fn contents(self) -> Contents {
        match self {
            PasteWhat::Formats
            | PasteWhat::Comments
            | PasteWhat::Validation
            | PasteWhat::ColumnWidths => Contents::Kept,
            PasteWhat::Values
            | PasteWhat::ValuesAndNumberFormats
            | PasteWhat::ValuesAndSourceFormats => Contents::Values,
            _ => Contents::Formulas,
        }
    }

    /// What of the copied formats is written.
    fn formats(self) -> Formats {
        match self {
            PasteWhat::All
            | PasteWhat::AllUsingSourceTheme
            | PasteWhat::AllMergingConditionalFormats
            | PasteWhat::KeepSourceColumnWidths
            | PasteWhat::Formats
            | PasteWhat::ValuesAndSourceFormats => Formats::All,
            PasteWhat::AllExceptBorders => Formats::AllButBorders,
            PasteWhat::FormulasAndNumberFormats | PasteWhat::ValuesAndNumberFormats => {
                Formats::Number
            }
            _ => Formats::Kept,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Contents {
    Kept,
    Values,
    Formulas,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Formats {
    Kept,
    All,
    AllButBorders,
    Number,
}

/// The "Operation" choices: what is copied is combined with what the
/// cells hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasteOp {
    #[default]
    None,
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl PasteOp {
    fn binary(self) -> Option<BinaryOp> {
        match self {
            PasteOp::None => None,
            PasteOp::Add => Some(BinaryOp::Add),
            PasteOp::Subtract => Some(BinaryOp::Sub),
            PasteOp::Multiply => Some(BinaryOp::Mul),
            PasteOp::Divide => Some(BinaryOp::Div),
        }
    }
}

/// Everything the Paste Special dialog asks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PasteSpecial {
    pub what: PasteWhat,
    pub operation: PasteOp,
    pub skip_blanks: bool,
    pub transpose: bool,
}

impl PasteSpecial {
    pub fn of(what: PasteWhat) -> PasteSpecial {
        PasteSpecial {
            what,
            ..PasteSpecial::default()
        }
    }

    /// Whether a plain paste (or two) does it all.
    fn is_plain(&self) -> bool {
        (self.operation == PasteOp::None && !self.skip_blanks && !self.transpose)
            // These write no cell: the other choices mean nothing to them.
            || matches!(self.what, PasteWhat::Comments | PasteWhat::Validation | PasteWhat::ColumnWidths)
    }
}

/// One side of an operation.
enum Operand {
    Number(f64),
    Formula(Arc<Expr>),
    /// Text, a logical value, an error: an operation leaves the cell as
    /// it is.
    Other,
}

impl Operand {
    fn of(content: &Content) -> Operand {
        match content {
            Content::Empty => Operand::Number(0.0),
            Content::Value(Value::Number(n)) => Operand::Number(*n),
            Content::Value(Value::Empty) => Operand::Number(0.0),
            Content::Value(_) => Operand::Other,
            Content::Formula(e) => Operand::Formula(e.clone()),
        }
    }

    /// As a term of the formula Excel writes: `=(old)+value`.
    fn term(&self) -> Expr {
        match self {
            Operand::Number(n) => Expr::Number(*n),
            Operand::Formula(e) => Expr::Paren(Box::new((**e).clone())),
            Operand::Other => Expr::Number(0.0),
        }
    }
}

/// What a cell holds after `existing op pasted`; `None` leaves it alone.
fn combine(existing: &Content, pasted: &Content, op: BinaryOp) -> Option<Content> {
    match (Operand::of(existing), Operand::of(pasted)) {
        (Operand::Other, _) | (_, Operand::Other) => None,
        (Operand::Number(a), Operand::Number(b)) => Some(Content::Value(match op {
            BinaryOp::Add => Value::Number(a + b),
            BinaryOp::Sub => Value::Number(a - b),
            BinaryOp::Mul => Value::Number(a * b),
            _ if b == 0.0 => Value::Error(ErrorKind::Div0),
            _ => Value::Number(a / b),
        })),
        (a, b) => Some(Content::Formula(Arc::new(Expr::Binary(
            op,
            Box::new(a.term()),
            Box::new(b.term()),
        )))),
    }
}

/// One corner of a reference of a formula copied from `from` to `to` with
/// rows and columns exchanged: what was so many rows away is now as many
/// columns away. Fixed parts stay; `None` when it falls off the sheet.
fn transposed_corner(corner: &CellRef, from: (u32, u32), to: (u32, u32)) -> Option<CellRef> {
    let d_row = corner.row as i64 - from.0 as i64;
    let d_col = corner.col as i64 - from.1 as i64;
    let (row, col) = match (corner.abs_row, corner.abs_col) {
        (true, true) => (corner.row as i64, corner.col as i64),
        (false, false) => (to.0 as i64 + d_col, to.1 as i64 + d_row),
        // A half-fixed reference keeps its distance along the free axis.
        (true, false) => (corner.row as i64, to.1 as i64 + d_col),
        (false, true) => (to.0 as i64 + d_row, corner.col as i64),
    };
    ((0..MAX_ROWS as i64).contains(&row) && (0..MAX_COLS as i64).contains(&col)).then_some(
        CellRef {
            row: row as u32,
            col: col as u32,
            ..*corner
        },
    )
}

/// Where a block of cells lands when the block `src` is pasted with its
/// rows and columns exchanged, its top-left cell on that of `out`; `None`
/// when it falls off the sheet or outside `out`.
pub(crate) fn turned(piece: &Rect, src: &Rect, out: &Rect) -> Option<Rect> {
    let rect = Rect::new(
        out.r0 + (piece.c0 - src.c0),
        out.c0 + (piece.r0 - src.r0),
        out.r0 + (piece.c1 - src.c0),
        out.c0 + (piece.r1 - src.r0),
    );
    rect.intersection(out)
}

/// A formula copied from `from` to `to` by a transposed paste: Excel
/// transposes its relative references with the cells.
pub(crate) fn transposed(e: &Expr, from: (u32, u32), to: (u32, u32)) -> Expr {
    let (d_row, d_col) = (to.0 as i64 - from.0 as i64, to.1 as i64 - from.1 as i64);
    e.map(&mut |x| match x {
        Expr::Ref(r) if r.kind == RefKind::Cells => {
            let start = transposed_corner(&r.start, from, to);
            let end = r.end.as_ref().map(|end| transposed_corner(end, from, to));
            match (start, end) {
                (Some(start), None) => Expr::Ref(RefExpr {
                    start,
                    end: None,
                    ..r
                }),
                (Some(start), Some(Some(end))) => {
                    // Corners named top-left first, as they are written.
                    let (r0, r1) = (start.row.min(end.row), start.row.max(end.row));
                    let (c0, c1) = (start.col.min(end.col), start.col.max(end.col));
                    Expr::Ref(RefExpr {
                        start: CellRef {
                            row: r0,
                            col: c0,
                            ..start
                        },
                        end: Some(CellRef {
                            row: r1,
                            col: c1,
                            ..end
                        }),
                        ..r
                    })
                }
                _ => Expr::Error(ErrorKind::Ref),
            }
        }
        // Whole rows and columns, and references across sheets, move as
        // in any copy.
        other @ (Expr::Ref(_) | Expr::Ref3D(_)) => other.shifted(d_row, d_col),
        other => other,
    })
}

/// Where a special paste writes and how the clip is laid there.
struct Layout {
    out: Rect,
    /// Rows and columns of one copy of the clip as it lands.
    height: u32,
    width: u32,
}

impl Workbook {
    fn special_layout(&self, sheet: usize, dest: Rect, clip: &Clip, transpose: bool) -> Layout {
        let (ch, cw) = (clip.rows.len() as u32, clip.rect.cols());
        let (height, width) = if transpose { (cw, ch) } else { (ch, cw) };
        let dest = self.clamp_to_used(sheet, dest);
        let (mut th, mut tw) = (height, width);
        if dest.rows() % height == 0 && dest.cols() % width == 0 {
            th = dest.rows();
            tw = dest.cols();
        }
        let th = th.min(MAX_ROWS - dest.r0);
        let tw = tw.min(MAX_COLS - dest.c0);
        Layout {
            out: Rect::new(dest.r0, dest.c0, dest.r0 + th - 1, dest.c0 + tw - 1),
            height,
            width,
        }
    }

    /// The rectangle `paste_special` would write for these arguments.
    pub fn paste_special_area(
        &self,
        sheet: usize,
        dest: Rect,
        clip: &Clip,
        spec: &PasteSpecial,
    ) -> Rect {
        if sheet >= self.sheets.len() || clip.rows.is_empty() {
            return dest;
        }
        if spec.is_plain() {
            return self.paste_area(sheet, dest, clip);
        }
        self.special_layout(sheet, dest, clip, spec.transpose).out
    }

    /// Paste a clip at `dest` as the Paste Special dialog asks. Returns
    /// the rectangle written.
    ///
    /// With an operation, numbers are combined into numbers and formulas
    /// are wrapped (`=(old)+value`), blank cells counting as zero and
    /// cells holding text staying as they are. With `skip_blanks` the
    /// blank cells of the clip leave the cells they land on untouched.
    /// With `transpose` rows become columns, and the relative references
    /// of the formulas are transposed with them, in the cells and in the
    /// conditional formats and validation rules that travel with them;
    /// comments are then left behind.
    pub fn paste_special(
        &mut self,
        sheet: usize,
        dest: Rect,
        clip: &Clip,
        spec: &PasteSpecial,
    ) -> Rect {
        if sheet >= self.sheets.len() || clip.rows.is_empty() {
            return dest;
        }
        if spec.is_plain() {
            let mut out = dest;
            for (i, mode) in spec.what.steps().iter().enumerate() {
                let written = self.paste(sheet, dest, clip, *mode);
                if i == 0 {
                    out = written;
                }
            }
            return out;
        }
        let layout = self.special_layout(sheet, dest, clip, spec.transpose);
        let out = layout.out;
        let (contents, formats) = (spec.what.contents(), spec.what.formats());
        let cw = clip.rect.cols();
        let op = spec
            .operation
            .binary()
            .filter(|_| contents != Contents::Kept);

        // What is kept beside the cells travels when the cells keep their
        // places.
        if !spec.transpose && formats == Formats::All && contents == Contents::Formulas {
            self.paste(sheet, out, clip, PasteMode::Comments);
            self.paste(sheet, out, clip, PasteMode::Validation);
        }
        // Turned, they are turned too: each copy of the clip gets the
        // rules of the copied cells, rows for columns. (Cells taken from
        // a filtered list are not the block the rules are read from.)
        let whole = clip.rows.len() as u32 == clip.rect.rows();
        if spec.transpose && whole && op.is_none() && !spec.skip_blanks {
            for top in (0..out.rows()).step_by(layout.height as usize) {
                for left in (0..out.cols()).step_by(layout.width as usize) {
                    let copy = Rect::new(
                        out.r0 + top,
                        out.c0 + left,
                        (out.r0 + top + layout.height - 1).min(out.r1),
                        (out.c0 + left + layout.width - 1).min(out.c1),
                    );
                    if matches!(formats, Formats::All | Formats::AllButBorders) {
                        self.transpose_cond_formats(clip.sheet, clip.rect, sheet, copy);
                    }
                    if formats == Formats::All && contents == Contents::Formulas {
                        self.transpose_validation(clip.sheet, clip.rect, sheet, copy);
                    }
                }
            }
        }
        if matches!(formats, Formats::All | Formats::AllButBorders) && !spec.skip_blanks {
            let merged = !clip.merges.is_empty();
            if merged || self.sheets[sheet].merges.iter().any(|m| m.intersects(&out)) {
                self.snapshot();
                self.sheets[sheet].merges.retain(|m| !m.intersects(&out));
                for top in (0..out.rows()).step_by(layout.height as usize) {
                    for left in (0..out.cols()).step_by(layout.width as usize) {
                        for m in &clip.merges {
                            let m = if spec.transpose {
                                Rect::new(m.c0, m.r0, m.c1, m.r1)
                            } else {
                                *m
                            };
                            let placed = Rect::new(
                                out.r0 + top + m.r0,
                                out.c0 + left + m.c0,
                                out.r0 + top + m.r1,
                                out.c0 + left + m.c1,
                            );
                            if placed.r1 <= out.r1 && placed.c1 <= out.c1 {
                                self.sheets[sheet].merges.push(placed);
                            }
                        }
                    }
                }
            }
        }

        // Styles made of the copied cell's and of the one it lands on.
        let mut mixed: HashMap<(StyleId, StyleId), StyleId> = HashMap::new();
        for dr in 0..out.rows() {
            for dc in 0..out.cols() {
                let (row, col) = (out.r0 + dr, out.c0 + dc);
                let (i, j) = if spec.transpose {
                    (dc % layout.width, dr % layout.height)
                } else {
                    (dr % layout.height, dc % layout.width)
                };
                let src: Option<&ClipCell> = clip.cells[(i * cw + j) as usize].as_ref();
                let blank =
                    src.is_none_or(|s| matches!(s.content, Content::Empty) && s.value.is_empty());
                if blank && spec.skip_blanks {
                    continue;
                }
                let from = (clip.rows[i as usize], clip.rect.c0 + j);
                let existing = self.sheets[sheet].cells.get(&(row, col)).cloned();
                let cur_style = self.sheets[sheet].style_id_at(row, col);
                let kept = || {
                    existing
                        .as_ref()
                        .map(|c| c.content.clone())
                        .unwrap_or(Content::Empty)
                };
                let value = src.map(|s| s.value.clone()).unwrap_or_default();
                let pasted = match (contents, src.map(|s| &s.content)) {
                    (Contents::Kept, _) => kept(),
                    (Contents::Formulas, Some(Content::Formula(e))) => {
                        Content::Formula(Arc::new(if spec.transpose {
                            transposed(e, from, (row, col))
                        } else {
                            e.shifted(row as i64 - from.0 as i64, col as i64 - from.1 as i64)
                        }))
                    }
                    (Contents::Formulas, Some(content)) => content.clone(),
                    (Contents::Values, _) if !value.is_empty() => Content::Value(value),
                    _ => Content::Empty,
                };
                let content = match op {
                    Some(op) => combine(&kept(), &pasted, op).unwrap_or_else(kept),
                    None => pasted,
                };
                let from_style = src.map(|s| s.style).unwrap_or_default();
                let style = match formats {
                    Formats::Kept => cur_style,
                    Formats::All => from_style,
                    Formats::AllButBorders | Formats::Number => {
                        *mixed.entry((from_style, cur_style)).or_insert_with(|| {
                            let (from, onto) = (
                                self.styles.get(from_style).clone(),
                                self.styles.get(cur_style).clone(),
                            );
                            let style = if formats == Formats::AllButBorders {
                                Style {
                                    borders: onto.borders,
                                    ..from
                                }
                            } else {
                                Style {
                                    num_fmt: from.num_fmt,
                                    ..onto
                                }
                            };
                            self.styles.intern(style)
                        })
                    }
                };
                let value = match &content {
                    Content::Value(v) => v.clone(),
                    _ => Value::Empty,
                };
                self.put_cell(
                    sheet,
                    row,
                    col,
                    Some(Cell {
                        content,
                        value,
                        style,
                    }),
                );
            }
        }
        out
    }

    /// Paste Link: formulas that read the copied cells, laid from the
    /// top-left cell of `dest`. One cell is linked by a fixed reference
    /// (`=$A$1`), a range cell by cell with relative ones, as in Excel.
    /// Returns the rectangle written.
    pub fn paste_link(&mut self, sheet: usize, dest: Rect, clip: &Clip) -> Rect {
        if sheet >= self.sheets.len() || clip.rows.is_empty() || clip.sheet >= self.sheets.len() {
            return dest;
        }
        let source: Option<Arc<str>> =
            (clip.sheet != sheet).then(|| Arc::from(self.sheets[clip.sheet].name.as_str()));
        let single = clip.rows.len() == 1 && clip.rect.cols() == 1;
        let mut out = Rect::cell(dest.r0, dest.c0);
        for (i, &from_row) in clip.rows.iter().enumerate() {
            for j in 0..clip.rect.cols() {
                let (row, col) = (dest.r0 + i as u32, dest.c0 + j);
                if row >= MAX_ROWS || col >= MAX_COLS {
                    continue;
                }
                let mut link = RefExpr::cell(from_row, clip.rect.c0 + j);
                link.sheet = source.clone();
                link.start.abs_row = single;
                link.start.abs_col = single;
                self.set_formula(sheet, row, col, Expr::Ref(link));
                out.r1 = out.r1.max(row);
                out.c1 = out.c1.max(col);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{val, wb};

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn text(w: &Workbook, name: &str) -> String {
        let at = rect(name);
        w.edit_text(0, at.r0, at.c0)
    }

    fn spec(what: PasteWhat, operation: PasteOp) -> PasteSpecial {
        PasteSpecial {
            what,
            operation,
            ..Default::default()
        }
    }

    #[test]
    fn operations_combine_numbers_and_wrap_formulas() {
        let mut w = wb(&[
            ("A1", "10"),
            ("C1", "5"),
            ("C2", "=A1*2"),
            ("C3", "text"),
            ("C5", "7"),
            ("E1", "=A1+1"),
            ("G1", "4"),
        ]);
        let ten = w.copy(0, rect("A1"));
        // A number onto numbers, a formula, text and a blank cell.
        assert_eq!(
            w.paste_special(0, rect("C1:C4"), &ten, &spec(PasteWhat::All, PasteOp::Add)),
            rect("C1:C4")
        );
        w.commit("add");
        assert_eq!(val(&w, "C1"), Value::Number(15.0));
        assert_eq!(text(&w, "C2"), "=(A1*2)+10");
        assert_eq!(val(&w, "C2"), Value::Number(30.0));
        assert_eq!(val(&w, "C3"), Value::text("text"));
        assert_eq!(val(&w, "C4"), Value::Number(10.0));
        // A copied formula is wrapped too, moved as any copy.
        let formula = w.copy(0, rect("E1"));
        w.paste_special(
            0,
            rect("G1"),
            &formula,
            &spec(PasteWhat::Formulas, PasteOp::Multiply),
        );
        w.commit("multiply");
        assert_eq!(text(&w, "G1"), "=4*(C1+1)");
        // Its value alone when values are pasted.
        w.paste_special(
            0,
            rect("C5"),
            &formula,
            &spec(PasteWhat::Values, PasteOp::Subtract),
        );
        w.commit("subtract");
        assert_eq!(val(&w, "C5"), Value::Number(-4.0));
        // Dividing by a blank cell is a division by zero.
        let blank = w.copy(0, rect("Z50"));
        w.paste_special(
            0,
            rect("C5"),
            &blank,
            &spec(PasteWhat::All, PasteOp::Divide),
        );
        w.commit("divide");
        assert_eq!(val(&w, "C5"), Value::Error(ErrorKind::Div0));
        assert!(w.undo());
        assert_eq!(val(&w, "C5"), Value::Number(-4.0));
    }

    #[test]
    fn skip_blanks_leaves_what_blank_cells_land_on() {
        let mut w = wb(&[
            ("A1", "1"),
            ("A3", "3"),
            ("C1", "x"),
            ("C2", "y"),
            ("C3", "z"),
        ]);
        let clip = w.copy(0, rect("A1:A3"));
        let skip = PasteSpecial {
            skip_blanks: true,
            ..Default::default()
        };
        assert_eq!(w.paste_special(0, rect("C1"), &clip, &skip), rect("C1:C3"));
        w.commit("paste");
        assert_eq!(val(&w, "C1"), Value::Number(1.0));
        assert_eq!(val(&w, "C2"), Value::text("y"));
        assert_eq!(val(&w, "C3"), Value::Number(3.0));
    }

    #[test]
    fn transpose_turns_rows_into_columns_with_their_references() {
        let mut w = wb(&[
            ("A1", "1"),
            ("B1", "2"),
            ("C1", "=A1+B1"),
            ("A2", "10"),
            ("B2", "20"),
            ("C2", "=SUM(A2:B2)*$A$1"),
        ]);
        let clip = w.copy(0, rect("A1:C2"));
        let turn = PasteSpecial {
            transpose: true,
            ..Default::default()
        };
        assert_eq!(
            w.paste_special_area(0, rect("E5"), &clip, &turn),
            rect("E5:F7")
        );
        assert_eq!(w.paste_special(0, rect("E5"), &clip, &turn), rect("E5:F7"));
        w.commit("transpose");
        assert_eq!(val(&w, "E6"), Value::Number(2.0));
        assert_eq!(val(&w, "F5"), Value::Number(10.0));
        // What was two cells to the left is now two cells above.
        assert_eq!(text(&w, "E7"), "=E5+E6");
        assert_eq!(val(&w, "E7"), Value::Number(3.0));
        assert_eq!(text(&w, "F7"), "=SUM(F5:F6)*$A$1");
        assert_eq!(val(&w, "F7"), Value::Number(30.0));
        // A reference that falls off the sheet is lost.
        let mut w = wb(&[("B5", "=A5")]);
        let clip = w.copy(0, rect("B5"));
        w.paste_special(0, rect("C1"), &clip, &turn);
        w.commit("transpose");
        assert_eq!(text(&w, "C1"), "=#REF!");
    }

    #[test]
    fn a_transposed_paste_turns_conditional_formats_and_validation() {
        use crate::condfmt::{CellOp, CondFormat, Rule};
        use crate::locale::Locale;
        use crate::parser::parse_formula;
        use crate::validation::{Validation, ValidationKind};
        let formula = |text: &str| parse_formula(text, Locale::en()).unwrap();
        // A1:C2: numbers; a rule on the row A1:C1 comparing each cell
        // with the one under it; a list on A2:B2 reading three cells of
        // the same row, four columns further.
        let mut w = wb(&[
            ("A1", "5"),
            ("B1", "1"),
            ("C1", "9"),
            ("A2", "3"),
            ("B2", "3"),
            ("C2", "3"),
        ]);
        let rule = Rule::CellIs {
            op: CellOp::Greater,
            first: formula("A2"),
            second: None,
        };
        w.add_cond_format(0, CondFormat::new(vec![rect("A1:C1")], rule));
        let list = Validation {
            kind: ValidationKind::List,
            formula1: Some(Arc::new(formula("E2:G2"))),
            ..Validation::default()
        };
        w.set_validation(0, &[rect("A2:B2")], (1, 0), list);
        w.commit("setup");
        let clip = w.copy(0, rect("A1:C2"));
        let spec = PasteSpecial {
            transpose: true,
            ..PasteSpecial::default()
        };
        let out = w.paste_special(0, rect("A5"), &clip, &spec);
        w.commit("transpose");
        assert_eq!(out, rect("A5:B7"));
        // The rule now runs down A5:A7 and compares with the cell on the
        // right; the copied cells keep theirs.
        let rules = w.cond_formats(0);
        let turned_rule = rules
            .iter()
            .find(|cf| cf.ranges == vec![rect("A5:A7")])
            .expect("the rule of the turned cells");
        match &turned_rule.rule {
            Rule::CellIs { first, .. } => assert_eq!(first.to_text(Locale::en()), "B5"),
            other => panic!("{other:?}"),
        }
        assert!(rules.iter().any(|cf| cf.ranges == vec![rect("A1:C1")]));
        // The list is on B5:B6 and reads three cells of the same column,
        // four rows further.
        let listed = w.validation_at(0, 5, 1).expect("a rule on B6");
        assert_eq!(listed.ranges, vec![rect("B5:B6")]);
        assert_eq!(listed.kind, ValidationKind::List);
        assert_eq!(
            listed
                .formula1
                .as_ref()
                .map(|source| source.to_text(Locale::en()))
                .as_deref(),
            Some("B9:B11")
        );
        assert_eq!(
            w.validation_at(0, 1, 0).map(|rule| rule.ranges.clone()),
            Some(vec![rect("A2:B2")])
        );
        assert!(w.validation_at(0, 6, 1).is_none() && w.validation_at(0, 4, 0).is_none());
        // One step of undo takes cells and rules away together.
        w.undo();
        assert_eq!(w.cond_formats(0).len(), 1);
        assert!(w.validation_at(0, 5, 1).is_none());
    }

    #[test]
    fn transposed_values_keep_their_formats_and_merges() {
        let mut w = wb(&[("A1", "0.5"), ("B1", "wide")]);
        w.update_style(0, rect("A1"), &|s| s.num_fmt = "0%".into());
        w.merge(0, rect("B1:C1"));
        w.commit("setup");
        let clip = w.copy(0, rect("A1:C1"));
        let turn = PasteSpecial {
            transpose: true,
            ..Default::default()
        };
        w.paste_special(0, rect("E1"), &clip, &turn);
        w.commit("transpose");
        assert_eq!(w.display_text(0, 0, 4).text, "50%");
        assert_eq!(w.sheets[0].merge_at(1, 4), Some(rect("E2:E3")));
    }

    #[test]
    fn the_choices_of_the_drop_down_are_plain_pastes() {
        let mut w = wb(&[("A1", "=1/4"), ("A2", "x")]);
        w.update_style(0, rect("A1"), &|s| {
            s.num_fmt = "0%".into();
            s.font.bold = true;
        });
        w.set_col_width(0, 0, Some(140.0));
        w.commit("setup");
        let clip = w.copy(0, rect("A1"));
        w.paste_special(
            0,
            rect("C1"),
            &clip,
            &PasteSpecial::of(PasteWhat::ValuesAndSourceFormats),
        );
        w.paste_special(
            0,
            rect("D1"),
            &clip,
            &PasteSpecial::of(PasteWhat::KeepSourceColumnWidths),
        );
        w.paste_special(
            0,
            rect("E1"),
            &clip,
            &PasteSpecial::of(PasteWhat::ValuesAndNumberFormats),
        );
        w.commit("paste");
        assert_eq!(text(&w, "C1"), "25%");
        assert!(w.style_at(0, 0, 2).font.bold);
        assert_eq!(text(&w, "D1"), "=1/4");
        assert_eq!(w.sheets[0].col_width(3), 140.0);
        assert_eq!(w.display_text(0, 0, 4).text, "25%");
        assert!(!w.style_at(0, 0, 4).font.bold);
    }

    #[test]
    fn paste_link_reads_the_copied_cells() {
        let mut w = wb(&[("A1", "1"), ("A2", "2"), ("B1", "3"), ("B2", "4")]);
        let one = w.copy(0, rect("A1"));
        let block = w.copy(0, rect("A1:B2"));
        assert_eq!(w.paste_link(0, rect("D1"), &one), rect("D1"));
        assert_eq!(w.paste_link(0, rect("D3"), &block), rect("D3:E4"));
        let other = w.add_sheet(1);
        w.paste_link(other, rect("A1"), &block);
        w.commit("links");
        assert_eq!(text(&w, "D1"), "=$A$1");
        assert_eq!(text(&w, "E4"), "=B2");
        assert_eq!(val(&w, "E4"), Value::Number(4.0));
        assert_eq!(w.edit_text(other, 1, 0), "=Sheet1!A2");
        assert_eq!(w.value_at(other, 1, 0), Value::Number(2.0));
    }
}
