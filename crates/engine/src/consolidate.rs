//! Data > Consolidate: combining several ranges into one, by position or
//! by the labels of their first row and first column.
//!
//! The settings of the dialog are kept on the sheet consolidated into
//! (`Sheet::consolidation`, the `<dataConsolidate>` of a file), so that
//! the dialog opens on them again. The result is written as values: the
//! "Create links to source data" variant, which writes formulas under an
//! outline, is not built.

use crate::ast::StructuralEdit;
use crate::filter::AutoFilter;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::value::{ErrorKind, Value};
use crate::workbook::{Content, Sheet, Workbook};

/// How the cells that fall together are combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConsolidateFunction {
    #[default]
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNumbers,
    StdDev,
    StdDevP,
    Var,
    VarP,
}

impl ConsolidateFunction {
    /// In the order of the dialog's list.
    pub const ALL: [ConsolidateFunction; 11] = [
        ConsolidateFunction::Sum,
        ConsolidateFunction::Count,
        ConsolidateFunction::Average,
        ConsolidateFunction::Max,
        ConsolidateFunction::Min,
        ConsolidateFunction::Product,
        ConsolidateFunction::CountNumbers,
        ConsolidateFunction::StdDev,
        ConsolidateFunction::StdDevP,
        ConsolidateFunction::Var,
        ConsolidateFunction::VarP,
    ];

    /// The value of the `function` attribute of `<dataConsolidate>`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            ConsolidateFunction::Sum => "sum",
            ConsolidateFunction::Count => "count",
            ConsolidateFunction::Average => "average",
            ConsolidateFunction::Max => "max",
            ConsolidateFunction::Min => "min",
            ConsolidateFunction::Product => "product",
            ConsolidateFunction::CountNumbers => "countNums",
            ConsolidateFunction::StdDev => "stdDev",
            ConsolidateFunction::StdDevP => "stdDevp",
            ConsolidateFunction::Var => "var",
            ConsolidateFunction::VarP => "varp",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<ConsolidateFunction> {
        ConsolidateFunction::ALL.into_iter().find(|f| f.ooxml_name() == name)
    }

    /// The result for the cells that fall together: `cells` counts those
    /// that hold something, `numbers` are the numbers among them. `None`
    /// leaves the result cell empty.
    fn of(self, cells: usize, numbers: &[f64]) -> Option<Value> {
        let n = numbers.len() as f64;
        let sum: f64 = numbers.iter().sum();
        let squares = || {
            let mean = sum / n;
            numbers.iter().map(|x| (x - mean).powi(2)).sum::<f64>()
        };
        let spread = |divisor: f64, root: bool| {
            if divisor <= 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            let variance = squares() / divisor;
            Value::Number(if root { variance.sqrt() } else { variance })
        };
        if self == ConsolidateFunction::Count {
            return (cells > 0).then_some(Value::Number(cells as f64));
        }
        if numbers.is_empty() {
            return None;
        }
        Some(match self {
            ConsolidateFunction::Sum => Value::Number(sum),
            ConsolidateFunction::Count | ConsolidateFunction::CountNumbers => Value::Number(n),
            ConsolidateFunction::Average => Value::Number(sum / n),
            ConsolidateFunction::Max => Value::Number(numbers.iter().copied().fold(f64::MIN, f64::max)),
            ConsolidateFunction::Min => Value::Number(numbers.iter().copied().fold(f64::MAX, f64::min)),
            ConsolidateFunction::Product => Value::Number(numbers.iter().product()),
            ConsolidateFunction::StdDev => spread(n - 1.0, true),
            ConsolidateFunction::StdDevP => spread(n, true),
            ConsolidateFunction::Var => spread(n - 1.0, false),
            ConsolidateFunction::VarP => spread(n, false),
        })
    }
}

/// One of the ranges consolidated ("All references"), with the name of
/// its sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsolidateSource {
    pub sheet: String,
    pub range: Rect,
}

/// What the Consolidate dialog holds.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Consolidation {
    pub function: ConsolidateFunction,
    pub sources: Vec<ConsolidateSource>,
    /// "Use labels in: Top row" / "Left column".
    pub top_row: bool,
    pub left_column: bool,
}

/// Why nothing was consolidated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsolidateError {
    /// No reference, or one that names a sheet that is not there.
    NoSource,
    /// The result does not fit on the sheet from the destination.
    OffSheet,
    /// A source range overlaps the destination.
    Overlap,
}

/// Where a cell of a source falls in the result: a position, or a label.
#[derive(Clone, Debug, PartialEq)]
enum Key {
    At(u32),
    Label(String),
}

/// The keys of one axis of the result, in order of first appearance.
#[derive(Default)]
struct Axis {
    keys: Vec<Key>,
    /// The labels as first written (the key folds the case).
    shown: Vec<Value>,
}

impl Axis {
    fn index(&mut self, key: Key, shown: Value) -> usize {
        match self.keys.iter().position(|k| *k == key) {
            Some(index) => index,
            None => {
                self.keys.push(key);
                self.shown.push(shown);
                self.keys.len() - 1
            }
        }
    }
}

impl Workbook {
    /// The table a consolidation gives: its rows of values, labels
    /// included (`Value::Empty` for the cells left empty).
    pub fn consolidated(&self, spec: &Consolidation) -> Result<Vec<Vec<Value>>, ConsolidateError> {
        if spec.sources.is_empty() {
            return Err(ConsolidateError::NoSource);
        }
        let (mut rows, mut cols) = (Axis::default(), Axis::default());
        // For each result cell: how many source cells hold something, and
        // their numbers.
        let mut cells: Vec<Vec<(usize, Vec<f64>)>> = Vec::new();
        for source in &spec.sources {
            let sheet = self.sheet_index(&source.sheet).ok_or(ConsolidateError::NoSource)?;
            let sh = &self.sheets[sheet];
            let range = self.clamp_to_used(sheet, source.range);
            let label = |row: u32, col: u32| {
                let shown = sh.value_at(row, col);
                (Key::Label(self.display_text(sheet, row, col).text.to_lowercase()), shown)
            };
            let first_row = range.r0 + u32::from(spec.top_row);
            let first_col = range.c0 + u32::from(spec.left_column);
            for row in first_row..=range.r1 {
                let (key, shown) = if spec.left_column { label(row, range.c0) } else { (Key::At(row - first_row), Value::Empty) };
                // A row without a label belongs to no category.
                if key == Key::Label(String::new()) {
                    continue;
                }
                let r = rows.index(key, shown);
                for col in first_col..=range.c1 {
                    let (key, shown) = if spec.top_row { label(range.r0, col) } else { (Key::At(col - first_col), Value::Empty) };
                    if key == Key::Label(String::new()) {
                        continue;
                    }
                    let c = cols.index(key, shown);
                    if cells.len() <= r {
                        cells.resize(r + 1, Vec::new());
                    }
                    if cells[r].len() <= c {
                        cells[r].resize(c + 1, (0, Vec::new()));
                    }
                    match sh.value_at(row, col) {
                        Value::Empty => {}
                        Value::Number(n) => {
                            cells[r][c].0 += 1;
                            cells[r][c].1.push(n);
                        }
                        _ => cells[r][c].0 += 1,
                    }
                }
            }
        }
        let mut table: Vec<Vec<Value>> = Vec::new();
        if spec.top_row {
            let mut header = Vec::new();
            if spec.left_column {
                header.push(Value::Empty);
            }
            header.extend(cols.shown.iter().cloned());
            table.push(header);
        }
        for (r, shown) in rows.shown.iter().enumerate() {
            let mut line = Vec::new();
            if spec.left_column {
                line.push(shown.clone());
            }
            for c in 0..cols.keys.len() {
                let value = cells.get(r).and_then(|row| row.get(c)).and_then(|(count, numbers)| spec.function.of(*count, numbers));
                line.push(value.unwrap_or(Value::Empty));
            }
            table.push(line);
        }
        Ok(table)
    }

    /// Where a consolidation would be written from the top-left cell of
    /// `dest`. A destination of several cells takes no more than it
    /// holds, as in Excel.
    pub fn consolidate_extent(&self, sheet: usize, dest: Rect, spec: &Consolidation) -> Result<Rect, ConsolidateError> {
        let table = self.consolidated(spec)?;
        let mut height = table.len() as u32;
        let mut width = table.iter().map(Vec::len).max().unwrap_or(0) as u32;
        if height == 0 || width == 0 {
            return Err(ConsolidateError::NoSource);
        }
        if !dest.is_single() {
            height = height.min(dest.rows());
            width = width.min(dest.cols());
        }
        if dest.r0 + height > MAX_ROWS || dest.c0 + width > MAX_COLS {
            return Err(ConsolidateError::OffSheet);
        }
        let extent = Rect::new(dest.r0, dest.c0, dest.r0 + height - 1, dest.c0 + width - 1);
        let name = self.sheets.get(sheet).map(|sh| sh.name.to_lowercase());
        let overlaps = spec
            .sources
            .iter()
            .any(|source| Some(source.sheet.to_lowercase()) == name && source.range.intersects(&extent));
        if overlaps { Err(ConsolidateError::Overlap) } else { Ok(extent) }
    }

    /// Write a consolidation on `sheet` from `dest` and remember its
    /// settings there. Returns the rectangle written. The caller commits.
    pub fn consolidate(&mut self, sheet: usize, dest: Rect, spec: &Consolidation) -> Result<Rect, ConsolidateError> {
        let extent = self.consolidate_extent(sheet, dest, spec)?;
        let table = self.consolidated(spec)?;
        for (i, line) in table.into_iter().take(extent.rows() as usize).enumerate() {
            for (j, value) in line.into_iter().take(extent.cols() as usize).enumerate() {
                let content = if value.is_empty() { Content::Empty } else { Content::Value(value) };
                self.set_content(sheet, extent.r0 + i as u32, extent.c0 + j as u32, content);
            }
        }
        self.snapshot();
        self.sheets[sheet].consolidation = Some(spec.clone());
        Ok(extent)
    }
}

/// See `features::on_structural_edit`: the references of every sheet to
/// the edited one follow its rows and columns.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    let Some(spec) = sheet.consolidation.as_mut() else {
        return;
    };
    spec.sources.retain_mut(|source| {
        if !source.sheet.eq_ignore_ascii_case(edited_sheet) {
            return true;
        }
        // A range moves the way the range of a filter does, and goes
        // with its first row or its columns.
        let mut range = AutoFilter::new(source.range);
        let kept = range.adjust(edit);
        source.range = range.range;
        kept
    });
}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    if let Some(spec) = sheet.consolidation.as_mut() {
        for source in spec.sources.iter_mut().filter(|source| source.sheet.eq_ignore_ascii_case(old)) {
            source.sheet = new.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{val, wb};

    fn source(range: &str) -> ConsolidateSource {
        ConsolidateSource {
            sheet: "Sheet1".to_string(),
            range: Rect::parse(range).unwrap(),
        }
    }

    fn spec(function: ConsolidateFunction, ranges: &[&str], top_row: bool, left_column: bool) -> Consolidation {
        Consolidation {
            function,
            sources: ranges.iter().map(|range| source(range)).collect(),
            top_row,
            left_column,
        }
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    #[test]
    fn by_position() {
        let w = wb(&[("A1", "1"), ("B1", "2"), ("A2", "3"), ("B2", "x"), ("D1", "10"), ("E1", "20"), ("D2", "30"), ("D3", "5")]);
        let table = w.consolidated(&spec(ConsolidateFunction::Sum, &["A1:B2", "D1:E3"], false, false)).unwrap();
        // Text is not a number; a cell no source fills stays empty.
        assert_eq!(table, vec![vec![n(11.0), n(22.0)], vec![n(33.0), Value::Empty], vec![n(5.0), Value::Empty]]);
        let table = w.consolidated(&spec(ConsolidateFunction::Count, &["A1:B2", "D1:E3"], false, false)).unwrap();
        assert_eq!(table[1], vec![n(2.0), n(1.0)]);
        let table = w.consolidated(&spec(ConsolidateFunction::CountNumbers, &["A1:B2", "D1:E3"], false, false)).unwrap();
        assert_eq!(table[1], vec![n(2.0), Value::Empty]);
    }

    #[test]
    fn every_function() {
        let w = wb(&[("A1", "2"), ("B1", "4"), ("C1", "4"), ("D1", "6")]);
        let of = |function| {
            let table = w.consolidated(&spec(function, &["A1", "B1", "C1", "D1"], false, false)).unwrap();
            table[0][0].clone()
        };
        assert_eq!(of(ConsolidateFunction::Sum), n(16.0));
        assert_eq!(of(ConsolidateFunction::Count), n(4.0));
        assert_eq!(of(ConsolidateFunction::Average), n(4.0));
        assert_eq!(of(ConsolidateFunction::Max), n(6.0));
        assert_eq!(of(ConsolidateFunction::Min), n(2.0));
        assert_eq!(of(ConsolidateFunction::Product), n(192.0));
        assert_eq!(of(ConsolidateFunction::CountNumbers), n(4.0));
        assert_eq!(of(ConsolidateFunction::VarP), n(2.0));
        assert_eq!(of(ConsolidateFunction::Var), n(8.0 / 3.0));
        assert_eq!(of(ConsolidateFunction::StdDevP), n(2f64.sqrt()));
        assert_eq!(of(ConsolidateFunction::StdDev), n((8.0f64 / 3.0).sqrt()));
        // One value has no sample deviation.
        let table = w.consolidated(&spec(ConsolidateFunction::StdDev, &["A1"], false, false)).unwrap();
        assert_eq!(table[0][0], Value::Error(ErrorKind::Div0));
        for function in ConsolidateFunction::ALL {
            assert_eq!(ConsolidateFunction::from_ooxml_name(function.ooxml_name()), Some(function));
        }
    }

    /// Two regions with products down and quarters across, not in the
    /// same order and not the same ones.
    fn regions() -> Workbook {
        wb(&[
            ("B1", "Q1"),
            ("C1", "Q2"),
            ("A2", "Apples"),
            ("B2", "10"),
            ("C2", "20"),
            ("A3", "Pears"),
            ("B3", "5"),
            ("C3", "6"),
            ("F1", "Q2"),
            ("G1", "Q3"),
            ("E2", "pears"),
            ("F2", "1"),
            ("G2", "2"),
            ("E3", "Plums"),
            ("F3", "7"),
            ("G3", "8"),
        ])
    }

    #[test]
    fn by_category_with_both_labels() {
        let w = regions();
        let table = w.consolidated(&spec(ConsolidateFunction::Sum, &["A1:C3", "E1:G3"], true, true)).unwrap();
        let t = Value::text;
        assert_eq!(
            table,
            vec![
                vec![Value::Empty, t("Q1"), t("Q2"), t("Q3")],
                vec![t("Apples"), n(10.0), n(20.0), Value::Empty],
                vec![t("Pears"), n(5.0), n(7.0), n(2.0)],
                vec![t("Plums"), Value::Empty, n(7.0), n(8.0)],
            ]
        );
    }

    #[test]
    fn labels_on_one_side_only() {
        let w = regions();
        // Top row: columns by label, rows by position.
        let table = w.consolidated(&spec(ConsolidateFunction::Sum, &["B1:C3", "F1:G3"], true, false)).unwrap();
        let t = Value::text;
        assert_eq!(
            table,
            vec![vec![t("Q1"), t("Q2"), t("Q3")], vec![n(10.0), n(21.0), n(2.0)], vec![n(5.0), n(13.0), n(8.0)]]
        );
        // Left column: rows by label, columns by position.
        let table = w.consolidated(&spec(ConsolidateFunction::Max, &["A2:C3", "E2:G3"], false, true)).unwrap();
        assert_eq!(
            table,
            vec![
                vec![t("Apples"), n(10.0), n(20.0)],
                vec![t("Pears"), n(5.0), n(6.0)],
                vec![t("Plums"), n(7.0), n(8.0)]
            ]
        );
    }

    #[test]
    fn writes_remembers_and_undoes() {
        let mut w = regions();
        let s = spec(ConsolidateFunction::Sum, &["A1:C3", "E1:G3"], true, true);
        let extent = w.consolidate(0, Rect::cell(9, 0), &s).unwrap();
        w.commit("Consolidate");
        assert_eq!(extent, Rect::parse("A10:D13").unwrap());
        assert_eq!(val(&w, "A12"), Value::text("Pears"));
        assert_eq!(val(&w, "C12"), n(7.0));
        assert_eq!(val(&w, "B13"), Value::Empty);
        assert_eq!(w.sheets[0].consolidation.as_ref(), Some(&s));
        w.undo();
        assert_eq!(val(&w, "A12"), Value::Empty);
        assert_eq!(w.sheets[0].consolidation, None);
        // A destination of several cells takes what it holds.
        let extent = w.consolidate(0, Rect::parse("A10:B11").unwrap(), &s).unwrap();
        w.commit("Consolidate");
        assert_eq!(extent, Rect::parse("A10:B11").unwrap());
        assert_eq!(val(&w, "C10"), Value::Empty);
    }

    #[test]
    fn refusals_and_references_that_follow() {
        let mut w = regions();
        let s = spec(ConsolidateFunction::Sum, &["A1:C3"], true, true);
        assert_eq!(w.consolidate(0, Rect::cell(1, 1), &s), Err(ConsolidateError::Overlap));
        assert_eq!(w.consolidate(0, Rect::cell(MAX_ROWS - 1, 8), &s), Err(ConsolidateError::OffSheet));
        assert_eq!(w.consolidate(0, Rect::cell(9, 0), &Consolidation::default()), Err(ConsolidateError::NoSource));
        let mut elsewhere = s.clone();
        elsewhere.sources[0].sheet = "Nowhere".to_string();
        assert_eq!(w.consolidate(0, Rect::cell(9, 0), &elsewhere), Err(ConsolidateError::NoSource));
        // The remembered references follow inserted rows and a new name.
        w.consolidate(0, Rect::cell(9, 0), &s).unwrap();
        w.commit("Consolidate");
        w.insert_rows(0, 0, 2);
        w.commit("Insert");
        assert!(w.rename_sheet(0, "Data"));
        let kept = w.sheets[0].consolidation.as_ref().unwrap();
        assert_eq!(kept.sources[0].range, Rect::parse("A3:C5").unwrap());
        assert_eq!(kept.sources[0].sheet, "Data");
    }
}
