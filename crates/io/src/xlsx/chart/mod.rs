//! Charts: the drawing part of a sheet (where each chart sits) and the chart
//! parts (what each one plots).
//!
//! The engine's model is small: a kind, a title, a legend position, data
//! labels on or off and, per series, the ranges it reads and an optional
//! colour. That is what is written and read back; the formatting other
//! applications store on top of it (fonts, gridlines, axis scales, colours
//! of single points, trend lines, ...) is not kept.

pub(crate) mod read;
pub(crate) mod write;

use opensheet_engine::ast::{CellRef, RefExpr, RefKind};
use std::collections::{BTreeMap, BTreeSet};

/// Drawing coordinates are in English Metric Units: 914 400 to the inch,
/// hence 9 525 to the pixel at 96 dpi.
const EMU_PER_PX: f64 = 9525.0;

/// Whether a reference covers several rows and several columns.
fn is_block(r: &RefExpr) -> bool {
    let area = r.rect();
    area.rows() > 1 && area.cols() > 1
}

/// One column (or one row) of the cells a reference covers: the first or
/// the last one.
fn line_of(r: &RefExpr, column: bool, last: bool) -> RefExpr {
    let area = r.rect();
    // Whole columns say nothing of rows, whole rows nothing of columns.
    let (abs_row, abs_col) = match r.kind {
        RefKind::Cells => (r.start.abs_row, r.start.abs_col),
        RefKind::Cols => (r.start.abs_col, r.start.abs_col),
        RefKind::Rows => (r.start.abs_row, r.start.abs_row),
    };
    let corner = |row: u32, col: u32| CellRef {
        row,
        col,
        abs_row,
        abs_col,
    };
    let (start, end) = if column {
        let col = if last { area.c1 } else { area.c0 };
        (corner(area.r0, col), corner(area.r1, col))
    } else {
        let row = if last { area.r1 } else { area.r0 };
        (corner(row, area.c0), corner(row, area.c1))
    };
    // A column of whole columns is a whole column, a row of whole rows a
    // whole row; anything else is plain cells.
    let kind = match (r.kind, column) {
        (RefKind::Cols, true) => RefKind::Cols,
        (RefKind::Rows, false) => RefKind::Rows,
        _ => RefKind::Cells,
    };
    RefExpr {
        sheet: r.sheet.clone(),
        start,
        end: (kind != RefKind::Cells || start != end).then_some(end),
        kind,
    }
}

/// The values of a series, as a file can hold them. Excel takes them from
/// a single row or column and stops with an error message on a file where
/// they come from a block. Of a block, this is the first column, or the
/// first row when it is wider than tall: the way the engine itself lays
/// series out in a table.
pub(crate) fn series_line(values: &RefExpr) -> RefExpr {
    if !is_block(values) {
        return values.clone();
    }
    let area = values.rect();
    line_of(values, area.rows() >= area.cols(), false)
}

/// The cell a series is named after: the engine reads the name in the
/// first cell of the range, and a block of cells is no name to Excel.
pub(crate) fn name_cell(name: &RefExpr) -> RefExpr {
    if !is_block(name) {
        return name.clone();
    }
    let area = name.rect();
    let across = line_of(name, false, false);
    RefExpr {
        start: CellRef {
            row: area.r0,
            col: area.c0,
            ..across.start
        },
        end: None,
        kind: RefKind::Cells,
        ..across
    }
}

/// Category labels in several columns (year, then quarter) are a list of
/// plain labels to the engine: this is the level next to the plot.
/// `values` tells which way the series runs.
pub(crate) fn innermost_level(categories: &RefExpr, values: &RefExpr) -> RefExpr {
    if !is_block(categories) {
        return categories.clone();
    }
    let (area, plotted) = (categories.rect(), values.rect());
    let in_columns = plotted.cols() == 1 || (plotted.rows() > 1 && area.rows() >= area.cols());
    line_of(categories, in_columns, true)
}

/// The widths of the columns (or the heights of the rows) of a sheet, in
/// pixels at 100 %. Anchors in a drawing are a cell plus an offset inside
/// it; this converts between them and distances in pixels.
pub(crate) struct Track<'a> {
    /// Size of the cells that have no size of their own.
    pub default: f32,
    pub sizes: &'a BTreeMap<u32, f32>,
    pub hidden: &'a BTreeSet<u32>,
    /// Largest size a cell can have.
    pub max: f32,
    /// Number of cells: the columns or rows of a sheet.
    pub count: u32,
}

impl Track<'_> {
    fn clamped(&self, px: f32) -> f64 {
        if px.is_finite() {
            px.clamp(0.0, self.max.max(0.0)) as f64
        } else {
            0.0
        }
    }

    fn size(&self, i: u32) -> f64 {
        if self.hidden.contains(&i) {
            return 0.0;
        }
        self.clamped(self.sizes.get(&i).copied().unwrap_or(self.default))
    }

    /// Pixels from the leading edge of cell `a` to that of cell `b`
    /// (negative when `b` comes first).
    pub fn span(&self, a: u32, b: u32) -> f64 {
        if b < a {
            return -self.span(b, a);
        }
        let default = self.clamped(self.default);
        let mut total = (b - a) as f64 * default;
        for (i, px) in self.sizes.range(a..b) {
            if !self.hidden.contains(i) {
                total += self.clamped(*px) - default;
            }
        }
        total - self.hidden.range(a..b).count() as f64 * default
    }

    /// The cell holding the point `offset` pixels past the leading edge of
    /// cell `start`, and how far into that cell the point lies.
    pub fn locate(&self, start: u32, offset: f64) -> (u32, f64) {
        let last = self.count.saturating_sub(1);
        let default = self.clamped(self.default);
        let mut i = start.min(last);
        let mut rest = if offset.is_finite() {
            offset.max(0.0)
        } else {
            0.0
        };
        while i < last && rest > 0.0 {
            // The cells before the next one with a size of its own all have
            // the default size: step over them at once.
            let special = [
                self.sizes.range(i..).next().map(|(k, _)| *k),
                self.hidden.range(i..).next().copied(),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(last, |k| k.min(last));
            if special > i {
                let run = (special - i) as f64;
                if default > 0.0 && rest < run * default {
                    let whole = (rest / default).floor().min(run - 1.0);
                    return (i + whole as u32, (rest - whole * default).max(0.0));
                }
                rest -= run * default;
                i = special;
            } else {
                let size = self.size(i);
                if rest < size {
                    break;
                }
                rest -= size;
                i += 1;
            }
        }
        if i == last {
            // Nothing lies beyond the last cell.
            rest = rest.min(self.size(last));
        }
        (i, rest.max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_and_positions() {
        let sizes = BTreeMap::from([(2, 100.0f32), (5, 10.0), (6, 30.0)]);
        let hidden = BTreeSet::from([3u32, 6]);
        let t = Track {
            default: 64.0,
            sizes: &sizes,
            hidden: &hidden,
            max: 1000.0,
            count: 100,
        };
        // Widths: 64 64 100 0 64 10 0 64 64 ...
        assert_eq!(t.span(0, 0), 0.0);
        assert_eq!(t.span(0, 2), 128.0);
        assert_eq!(t.span(0, 3), 228.0);
        assert_eq!(t.span(0, 4), 228.0);
        assert_eq!(t.span(2, 8), 100.0 + 64.0 + 10.0 + 64.0);
        assert_eq!(t.span(8, 2), -238.0);
        assert_eq!(t.span(10, 20), 640.0);

        assert_eq!(t.locate(0, 0.0), (0, 0.0));
        assert_eq!(t.locate(0, 63.5), (0, 63.5));
        assert_eq!(t.locate(0, 64.0), (1, 0.0));
        assert_eq!(t.locate(0, 130.0), (2, 2.0));
        // The far edge of a cell is the start of the next one; hidden cells
        // are stepped over as soon as there is something left to cover.
        assert_eq!(t.locate(2, 100.0), (3, 0.0));
        assert_eq!(t.locate(2, 101.0), (4, 1.0));
        assert_eq!(t.locate(0, 228.0 + 64.0 + 10.0 + 5.0), (7, 5.0));
        // A position is never moved off the cell it starts in.
        assert_eq!(t.locate(3, 0.0), (3, 0.0));
        assert_eq!(t.locate(20, 64.0 * 30.0 + 1.0), (50, 1.0));
        // The sheet ends somewhere.
        assert_eq!(t.locate(98, 1e12), (99, 64.0));
        assert_eq!(t.locate(5000, 3.0), (99, 3.0));
        assert_eq!(t.locate(0, f64::NAN), (0, 0.0));
        assert_eq!(t.locate(0, -5.0), (0, 0.0));
        // Every position is where `span` says it is.
        for start in [0u32, 2, 3, 6, 40] {
            for offset in [0.5, 10.0, 99.0, 250.0, 1234.5] {
                let (cell, inside) = t.locate(start, offset);
                assert_eq!(t.span(start, cell) + inside, offset, "{start} + {offset}");
            }
        }
    }

    #[test]
    fn blocks_are_reduced_to_lines() {
        use opensheet_engine::Locale;
        use opensheet_engine::ast::Expr;
        use opensheet_engine::parser::parse_formula;
        let range = |text: &str| match parse_formula(text, Locale::en()) {
            Ok(Expr::Ref(r)) => r,
            other => panic!("{text}: {other:?}"),
        };
        let values = |text: &str| series_line(&range(text)).to_text();
        // Rows, columns and cells are what they are.
        for text in ["S!$B$2:$B$9", "S!$B$2:$F$2", "S!$C$3", "S!$B:$B", "S!$4:$4"] {
            assert_eq!(values(text), text);
            assert_eq!(name_cell(&range(text)).to_text(), text);
            let labels = innermost_level(&range(text), &range("S!$A$1:$A$9"));
            assert_eq!(labels.to_text(), text);
        }
        // Blocks: the first column when tall or square, else the first row.
        assert_eq!(values("S!$B$2:$C$9"), "S!$B$2:$B$9");
        assert_eq!(values("S!$B$2:$C$3"), "S!$B$2:$B$3");
        assert_eq!(values("S!$B$2:$F$3"), "S!$B$2:$F$2");
        assert_eq!(values("S!$C$9:$B$2"), "S!$B$2:$B$9");
        assert_eq!(values("'a b'!B2:C9"), "'a b'!B2:B9");
        assert_eq!(values("S!$B:$D"), "S!$B:$B");
        assert_eq!(values("S!$2:$5"), "S!$2:$2");
        assert_eq!(values("S!$A:$XFD"), "S!$A:$A");
        assert_eq!(values("S!$1:$1048576"), "S!$A$1:$A$1048576");
        assert_eq!(values("S!$A$1:$XFD$1048576"), "S!$A$1:$A$1048576");
        // Names: the first cell.
        assert_eq!(name_cell(&range("S!$B$2:$C$9")).to_text(), "S!$B$2");
        assert_eq!(name_cell(&range("S!$C$9:$B$2")).to_text(), "S!$B$2");
        assert_eq!(name_cell(&range("S!$B:$D")).to_text(), "S!$B$1");
        // Categories: the column next to values in a column, the row next
        // to values in a row.
        let labels =
            |text: &str, values: &str| innermost_level(&range(text), &range(values)).to_text();
        assert_eq!(labels("S!$A$2:$B$9", "S!$C$2:$C$9"), "S!$B$2:$B$9");
        assert_eq!(labels("S!$B$1:$F$2", "S!$B$3:$F$3"), "S!$B$2:$F$2");
        assert_eq!(labels("S!$A$2:$B$3", "S!$C$2:$C$3"), "S!$B$2:$B$3");
        assert_eq!(labels("S!$A$2:$B$3", "S!$C$4:$D$4"), "S!$A$3:$B$3");
        assert_eq!(labels("S!$A$2:$C$3", "S!$D$2"), "S!$C$2:$C$3");
        assert_eq!(labels("S!$A:$B", "S!$C:$C"), "S!$B:$B");
        assert_eq!(labels("S!$1:$2", "S!$3:$3"), "S!$2:$2");
    }

    #[test]
    fn degenerate_tracks() {
        let none = BTreeMap::new();
        let hidden = BTreeSet::new();
        let t = Track {
            default: 0.0,
            sizes: &none,
            hidden: &hidden,
            max: f32::INFINITY,
            count: 1_048_576,
        };
        assert_eq!(t.locate(0, 10.0), (1_048_575, 0.0));
        assert_eq!(t.span(0, 1_000_000), 0.0);
        let t = Track { count: 0, ..t };
        assert_eq!(t.locate(7, 10.0), (0, 0.0));
        let t = Track {
            default: f32::NAN,
            count: 10,
            ..t
        };
        assert_eq!(t.locate(0, 10.0), (9, 0.0));
    }
}
