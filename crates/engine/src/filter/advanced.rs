//! The Advanced Filter (Data > Advanced): the records of a list that
//! satisfy a criteria range are kept in place, the others hidden, or
//! copied to another location.
//!
//! A criteria range has field names in its first row and conditions under
//! them: the conditions of a row must all hold (AND), and a record is kept
//! when one row is satisfied (OR). A condition is a value to compare the
//! field with, optionally after `=`, `<>`, `>`, `>=`, `<` or `<=`; text
//! without an operator matches the cells that begin with it, `*` and `?`
//! are wildcards, and `="=text"` asks for exactly that text. Under a
//! heading that is empty or not a field name, the cell holds a formula
//! written for the first record: it is calculated for each record with its
//! relative references following the record.

use super::AutoFilter;
use crate::ast::{Expr, StructuralEdit};
use crate::eval::EvalSession;
use crate::functions::criteria::Criteria as Comparison;
use crate::ops::shift_set;
use crate::refs::{MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::collections::{BTreeSet, HashSet};

/// An Advanced Filter applied in place: the list it filters and the rows
/// it hides (they are also in `Sheet::filtered_rows`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdvancedFilter {
    /// The list: header row and records.
    pub range: Rect,
    /// The criteria range, when it is on the same sheet (Excel remembers
    /// it under the name `Criteria`).
    pub criteria: Option<Rect>,
    pub hidden: BTreeSet<u32>,
}

/// Why an Advanced Filter cannot be applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedFilterError {
    /// The list range has no header row with a record under it.
    NoList,
    /// The criteria range has no row of field names.
    BadCriteria,
    /// A heading of the "Copy to" range is not a field name of the list.
    BadExtract,
    /// The copy does not fit on the sheet, or would land on the list.
    BadDestination,
}

/// One condition of a criteria row.
enum Condition {
    /// Compare the field at this offset in the list.
    Field(u32, Comparison),
    /// Calculate this formula of the criteria range for each record.
    Formula { row: u32, col: u32, expr: Expr },
    /// A constant under a heading that is not a field name.
    Always(bool),
}

/// A criteria range made ready to test records.
struct Test {
    sheet: usize,
    /// The conditions of each criteria row.
    rows: Vec<Vec<Condition>>,
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0,
        _ => false,
    }
}

impl Workbook {
    /// The headings of a list, lower-cased as they show.
    fn list_fields(&self, sheet: usize, list: Rect) -> Vec<String> {
        (list.c0..=list.c1)
            .map(|col| self.display_text(sheet, list.r0, col).text.to_lowercase())
            .collect()
    }

    fn criteria_test(
        &self,
        list: (usize, Rect),
        criteria: (usize, Rect),
    ) -> Result<Test, AdvancedFilterError> {
        let (sheet, range) = criteria;
        let sh = self
            .sheets
            .get(sheet)
            .ok_or(AdvancedFilterError::BadCriteria)?;
        let range = self.clamp_to_used(sheet, range);
        if range.rows() < 2 {
            return Err(AdvancedFilterError::BadCriteria);
        }
        let fields = self.list_fields(list.0, list.1);
        let headings: Vec<Option<u32>> = (range.c0..=range.c1)
            .map(|col| {
                let heading = self.display_text(sheet, range.r0, col).text.to_lowercase();
                fields
                    .iter()
                    .position(|f| !heading.is_empty() && *f == heading)
                    .map(|i| i as u32)
            })
            .collect();
        let mut rows = Vec::new();
        for row in range.r0 + 1..=range.r1 {
            let mut conditions = Vec::new();
            for (i, col) in (range.c0..=range.c1).enumerate() {
                let value = sh.value_at(row, col);
                if value.is_empty() {
                    continue;
                }
                let formula = match sh.cells.get(&(row, col)).map(|cell| &cell.content) {
                    Some(Content::Formula(expr)) => Some(expr.as_ref().clone()),
                    _ => None,
                };
                conditions.push(match (headings[i], formula) {
                    (Some(field), _) => {
                        let wanted = match &value {
                            // Text alone asks for the cells that begin with it.
                            Value::Text(text) if !text.starts_with(['=', '<', '>']) => {
                                Value::text(format!("{text}*"))
                            }
                            other => other.clone(),
                        };
                        Condition::Field(field, Comparison::parse(&wanted, self.locale))
                    }
                    (None, Some(expr)) => Condition::Formula { row, col, expr },
                    (None, None) => Condition::Always(truthy(&value)),
                });
            }
            rows.push(conditions);
        }
        Ok(Test { sheet, rows })
    }

    /// The rows of the records of `list` (sheet, range with its header
    /// row) that satisfy `criteria`, in order. Without criteria every
    /// record is kept. With `unique`, a record that repeats an earlier one
    /// in the columns `unique_on` (offsets in the list; all of them when
    /// empty) is left out.
    pub fn advanced_filter_rows(
        &self,
        list: (usize, Rect),
        criteria: Option<(usize, Rect)>,
        unique: bool,
        unique_on: &[u32],
    ) -> Result<Vec<u32>, AdvancedFilterError> {
        let (sheet, range) = list;
        let range = self.clamp_to_used(sheet, range);
        let sh = self.sheets.get(sheet).ok_or(AdvancedFilterError::NoList)?;
        if range.rows() < 2 || (range.c0..=range.c1).all(|col| sh.is_blank(range.r0, col)) {
            return Err(AdvancedFilterError::NoList);
        }
        let test = criteria
            .map(|criteria| self.criteria_test((sheet, range), criteria))
            .transpose()?;
        let session = EvalSession::new(self);
        let first = range.r0 + 1;
        let passes = |row: u32| match &test {
            None => true,
            Some(test) => test.rows.iter().any(|conditions| {
                conditions.iter().all(|condition| match condition {
                    Condition::Field(field, comparison) => {
                        comparison.matches(&sh.value_at(row, range.c0 + field))
                    }
                    Condition::Formula { row: at, col, expr } => {
                        let for_record = expr.shifted(i64::from(row) - i64::from(first), 0);
                        truthy(&session.eval(test.sheet, *at, *col, &for_record))
                    }
                    Condition::Always(holds) => *holds,
                })
            }),
        };
        let columns: Vec<u32> = if unique_on.is_empty() {
            (0..range.cols()).collect()
        } else {
            unique_on.to_vec()
        };
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut rows = Vec::new();
        for row in first..=range.r1 {
            if !passes(row) {
                continue;
            }
            if unique {
                let key: Vec<String> = columns
                    .iter()
                    .map(|c| {
                        self.display_text(sheet, row, range.c0 + c)
                            .text
                            .to_lowercase()
                    })
                    .collect();
                if !seen.insert(key) {
                    continue;
                }
            }
            rows.push(row);
        }
        Ok(rows)
    }

    /// Filter a list in place: the records that do not satisfy the
    /// criteria are hidden, and the filter buttons of the sheet go away
    /// (a sheet is filtered one way or the other, as in Excel). Returns
    /// how many records show. The caller commits.
    pub fn advanced_filter_in_place(
        &mut self,
        sheet: usize,
        list: Rect,
        criteria: Option<(usize, Rect)>,
        unique: bool,
    ) -> Result<usize, AdvancedFilterError> {
        let list = self.clamp_to_used(sheet, list);
        let kept = self.advanced_filter_rows((sheet, list), criteria, unique, &[])?;
        let hidden: BTreeSet<u32> = (list.r0 + 1..=list.r1)
            .filter(|row| kept.binary_search(row).is_err())
            .collect();
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        sh.auto_filter = None;
        sh.advanced_filter = Some(AdvancedFilter {
            range: list,
            criteria: criteria
                .filter(|(on, _)| *on == sheet)
                .map(|(_, range)| range),
            hidden,
        });
        self.refilter(sheet);
        Ok(kept.len())
    }

    /// Show the rows an Advanced Filter hides (Data > Clear). False when
    /// the sheet has none. The caller commits.
    pub fn clear_advanced_filter(&mut self, sheet: usize) -> bool {
        if !self
            .sheets
            .get(sheet)
            .is_some_and(|sh| sh.advanced_filter.is_some())
        {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].advanced_filter = None;
        self.refilter(sheet);
        true
    }

    /// The columns of the list an extract takes, in order, given the
    /// "Copy to" range: those its first row names, or all of them when
    /// that row is blank.
    fn extract_columns(
        &self,
        list: (usize, Rect),
        dest_sheet: usize,
        dest: Rect,
    ) -> Result<Vec<u32>, AdvancedFilterError> {
        let sh = self
            .sheets
            .get(dest_sheet)
            .ok_or(AdvancedFilterError::BadDestination)?;
        if (dest.c0..=dest.c1).all(|col| sh.is_blank(dest.r0, col)) {
            return Ok((0..list.1.cols()).collect());
        }
        let fields = self.list_fields(list.0, list.1);
        // Trailing blank cells of a wide selection are not headings.
        let last = (dest.c0..=dest.c1)
            .rev()
            .find(|col| !sh.is_blank(dest.r0, *col))
            .unwrap_or(dest.c0);
        (dest.c0..=last)
            .map(|col| {
                let heading = self
                    .display_text(dest_sheet, dest.r0, col)
                    .text
                    .to_lowercase();
                fields
                    .iter()
                    .position(|f| *f == heading)
                    .map(|i| i as u32)
                    .ok_or(AdvancedFilterError::BadExtract)
            })
            .collect()
    }

    /// Where an extract would be written: the heading row and one row per
    /// record kept.
    pub fn advanced_filter_extent(
        &self,
        list: (usize, Rect),
        criteria: Option<(usize, Rect)>,
        unique: bool,
        dest_sheet: usize,
        dest: Rect,
    ) -> Result<Rect, AdvancedFilterError> {
        let list = (list.0, self.clamp_to_used(list.0, list.1));
        let columns = self.extract_columns(list, dest_sheet, dest)?;
        let rows = self.advanced_filter_rows(list, criteria, unique, &columns)?;
        // A "Copy to" range several rows tall takes no more than it holds.
        let count = if dest.rows() > 1 {
            rows.len().min(dest.rows() as usize - 1)
        } else {
            rows.len()
        } as u32;
        if dest.r0 + count >= MAX_ROWS
            || dest.c0 as usize + columns.len() > crate::refs::MAX_COLS as usize
        {
            return Err(AdvancedFilterError::BadDestination);
        }
        let extent = Rect::new(
            dest.r0,
            dest.c0,
            dest.r0 + count,
            dest.c0 + columns.len() as u32 - 1,
        );
        if list.0 == dest_sheet && extent.intersects(&list.1) {
            return Err(AdvancedFilterError::BadDestination);
        }
        Ok(extent)
    }

    /// Copy the records that satisfy the criteria to another location:
    /// the headings, then the records, with their formats; formulas are
    /// copied as their results. What was under the headings is cleared
    /// down to the end of the data, as Excel does. Returns the rectangle
    /// written. The caller commits.
    pub fn advanced_filter_copy(
        &mut self,
        list: (usize, Rect),
        criteria: Option<(usize, Rect)>,
        unique: bool,
        dest_sheet: usize,
        dest: Rect,
    ) -> Result<Rect, AdvancedFilterError> {
        let list = (list.0, self.clamp_to_used(list.0, list.1));
        let extent = self.advanced_filter_extent(list, criteria, unique, dest_sheet, dest)?;
        let columns = self.extract_columns(list, dest_sheet, dest)?;
        let mut rows = self.advanced_filter_rows(list, criteria, unique, &columns)?;
        rows.truncate(extent.rows() as usize - 1);
        rows.insert(0, list.1.r0);
        // Read everything before writing: the copy may be read by the
        // criteria.
        let cells: Vec<Vec<Option<Cell>>> = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|c| {
                        self.sheets[list.0]
                            .cells
                            .get(&(*row, list.1.c0 + c))
                            .map(|cell| Cell {
                                content: match &cell.value {
                                    Value::Empty => Content::Empty,
                                    value => Content::Value(value.clone()),
                                },
                                value: cell.value.clone(),
                                style: cell.style,
                            })
                    })
                    .collect()
            })
            .collect();
        let (last_used, _) = self.sheets[dest_sheet].content_extent();
        if last_used > extent.r1 {
            let below = Rect::new(extent.r1 + 1, extent.c0, last_used, extent.c1);
            self.clear(dest_sheet, below, crate::workbook::ClearMode::Contents);
        }
        for (i, row) in cells.into_iter().enumerate() {
            for (j, cell) in row.into_iter().enumerate() {
                self.put_cell(dest_sheet, extent.r0 + i as u32, extent.c0 + j as u32, cell);
            }
        }
        Ok(extent)
    }
}

/// Follow rows or columns inserted into or deleted from the sheet.
pub(super) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit) {
    let Some(filter) = sheet.advanced_filter.as_mut() else {
        return;
    };
    match edit {
        StructuralEdit::InsertRows { at, count } => {
            filter.hidden = shift_set(&filter.hidden, true, at, count, MAX_ROWS)
        }
        StructuralEdit::DeleteRows { at, count } => {
            filter.hidden = shift_set(&filter.hidden, false, at, count, MAX_ROWS)
        }
        StructuralEdit::InsertCols { .. } | StructuralEdit::DeleteCols { .. } => {}
    }
    // The list moves the way the range of a filter does.
    let mut list = AutoFilter::new(filter.range);
    if list.adjust(edit) {
        filter.range = list.range;
        // The criteria range is only remembered for the file.
        filter.criteria = None;
    } else {
        // The header row went: the rows are shown again.
        let hidden = std::mem::take(&mut filter.hidden);
        sheet.filtered_rows.retain(|row| !hidden.contains(row));
        sheet.advanced_filter = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{val, wb};

    /// A list in A1:C7 (Name, Qty, City) and room for criteria in E1:G3.
    fn list(criteria: &[(&str, &str)]) -> Workbook {
        let mut cells = vec![
            ("A1", "Name"),
            ("B1", "Qty"),
            ("C1", "City"),
            ("A2", "Dupont"),
            ("B2", "5"),
            ("C2", "Paris"),
            ("A3", "Durand"),
            ("B3", "12"),
            ("C3", "Lyon"),
            ("A4", "Martin"),
            ("B4", "7"),
            ("C4", "Paris"),
            ("A5", "dupontel"),
            ("B5", "20"),
            ("C5", "Nice"),
            ("A6", "Dupont"),
            ("B6", "5"),
            ("C6", "Paris"),
            ("A7", "Petit"),
            ("B7", "3"),
            ("C7", "Lyon"),
        ];
        cells.extend_from_slice(criteria);
        wb(&cells)
    }

    const LIST: (usize, Rect) = (
        0,
        Rect {
            r0: 0,
            c0: 0,
            r1: 6,
            c1: 2,
        },
    );

    fn range(text: &str) -> Option<(usize, Rect)> {
        Some((0, Rect::parse(text).unwrap()))
    }

    /// The (1-based) sheet rows kept.
    fn kept(w: &Workbook, criteria: &str) -> Vec<u32> {
        w.advanced_filter_rows(LIST, range(criteria), false, &[])
            .unwrap()
            .iter()
            .map(|row| row + 1)
            .collect()
    }

    #[test]
    fn and_along_a_row_or_between_rows() {
        let w = list(&[
            ("E1", "City"),
            ("F1", "Qty"),
            ("E2", "Paris"),
            ("F2", ">5"),
            ("E3", "Lyon"),
        ]);
        // Paris with more than 5, or anything in Lyon.
        assert_eq!(kept(&w, "E1:F3"), [3, 4, 7]);
        assert_eq!(kept(&w, "E1:F2"), [4]);
        // The same field twice on a row: a band.
        let w = list(&[("E1", "Qty"), ("F1", "Qty"), ("E2", ">=5"), ("F2", "<12")]);
        assert_eq!(kept(&w, "E1:F2"), [2, 4, 6]);
        // A blank criteria row keeps every record.
        let w = list(&[("E1", "City"), ("E2", "Paris")]);
        assert_eq!(kept(&w, "E1:E3"), [2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn text_begins_with_wildcards_and_exact() {
        // Text alone: the cells that begin with it, whatever the case.
        let w = list(&[("E1", "name"), ("E2", "dupont")]);
        assert_eq!(kept(&w, "E1:E2"), [2, 5, 6]);
        // ="=Dupont": exactly that.
        let w = list(&[("E1", "Name"), ("E2", "=\"=Dupont\"")]);
        assert_eq!(kept(&w, "E1:E2"), [2, 6]);
        // "Begins with" after a wildcard is "contains"; exact, "ends with".
        let w = list(&[("E1", "Name"), ("E2", "*t")]);
        assert_eq!(kept(&w, "E1:E2"), [2, 4, 5, 6, 7]);
        let w = list(&[("E1", "Name"), ("E2", "'=*t")]);
        assert_eq!(kept(&w, "E1:E2"), [2, 6, 7]);
        let w = list(&[("E1", "Name"), ("E2", "D?r")]);
        assert_eq!(kept(&w, "E1:E2"), [3]);
        let w = list(&[("E1", "Name"), ("E2", "'<>Dupont")]);
        assert_eq!(kept(&w, "E1:E2"), [3, 4, 5, 7]);
        let w = list(&[("E1", "Name"), ("E2", "'>=M")]);
        assert_eq!(kept(&w, "E1:E2"), [4, 7]);
    }

    #[test]
    fn numbers_and_comparisons() {
        let w = list(&[("E1", "Qty"), ("E2", "5")]);
        assert_eq!(kept(&w, "E1:E2"), [2, 6]);
        let w = list(&[("E1", "Qty"), ("E2", "'<>5")]);
        assert_eq!(kept(&w, "E1:E2"), [3, 4, 5, 7]);
        let w = list(&[("E1", "Qty"), ("E2", "'<=5"), ("E3", "'>12")]);
        assert_eq!(kept(&w, "E1:E3"), [2, 5, 6, 7]);
    }

    #[test]
    fn formula_criteria_follow_the_record() {
        // Under an empty heading: above the average quantity.
        let w = list(&[("E2", "=B2>AVERAGE($B$2:$B$7)")]);
        assert_eq!(kept(&w, "E1:E2"), [3, 5]);
        // Under a heading that is not a field, with a field criterion.
        let w = list(&[
            ("E1", "Calc"),
            ("F1", "City"),
            ("E2", "=LEN(A2)=6"),
            ("F2", "Paris"),
        ]);
        assert_eq!(kept(&w, "E1:F2"), [2, 4, 6]);
        // A constant there is true or false for every record.
        let w = list(&[("E1", "Calc"), ("E2", "FALSE")]);
        assert_eq!(kept(&w, "E1:E2"), Vec::<u32>::new());
    }

    #[test]
    fn unique_records() {
        let w = list(&[]);
        let rows = w.advanced_filter_rows(LIST, None, true, &[]).unwrap();
        assert_eq!(rows, [1, 2, 3, 4, 6]);
        // On chosen columns: one record per city.
        let rows = w.advanced_filter_rows(LIST, None, true, &[2]).unwrap();
        assert_eq!(rows, [1, 2, 4]);
    }

    #[test]
    fn in_place_hides_rows_and_clear_shows_them() {
        let mut w = list(&[("E1", "City"), ("E2", "Paris")]);
        w.toggle_auto_filter(0, Rect::cell(1, 0)).unwrap();
        w.commit("filter");
        assert_eq!(
            w.advanced_filter_in_place(0, LIST.1, range("E1:E2"), false),
            Ok(3)
        );
        w.commit("Advanced Filter");
        assert_eq!(
            w.sheets[0]
                .filtered_rows
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [2, 4, 6]
        );
        assert!(w.sheets[0].auto_filter.is_none());
        // Reapplying the sheet's filters keeps them hidden.
        w.reapply_filters(0);
        w.commit("Reapply");
        assert_eq!(w.sheets[0].filtered_rows.len(), 3);
        // Rows inserted above move the hidden rows with the list.
        w.insert_rows(0, 0, 2);
        w.commit("Insert");
        assert_eq!(
            w.sheets[0]
                .filtered_rows
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            [4, 6, 8]
        );
        assert_eq!(
            w.sheets[0].advanced_filter.as_ref().unwrap().range,
            Rect::new(2, 0, 8, 2)
        );
        assert!(w.clear_advanced_filter(0));
        w.commit("Clear");
        assert!(w.sheets[0].filtered_rows.is_empty());
        assert!(!w.clear_advanced_filter(0));
        // Undo brings the filter back in one step.
        w.undo();
        assert_eq!(w.sheets[0].filtered_rows.len(), 3);
    }

    #[test]
    fn copy_to_another_location() {
        let mut w = list(&[("E1", "City"), ("E2", "Paris"), ("I5", "old")]);
        let dest = Rect::parse("I1").unwrap();
        let extent = w
            .advanced_filter_copy(LIST, range("E1:E2"), false, 0, dest)
            .unwrap();
        w.commit("Advanced Filter");
        assert_eq!(extent, Rect::parse("I1:K4").unwrap());
        assert_eq!(val(&w, "I1"), Value::text("Name"));
        assert_eq!(val(&w, "K1"), Value::text("City"));
        assert_eq!(val(&w, "I3"), Value::text("Martin"));
        assert_eq!(val(&w, "J4"), Value::Number(5.0));
        // What was under the extract is cleared.
        assert_eq!(val(&w, "I5"), Value::Empty);
        w.undo();
        assert_eq!(val(&w, "I1"), Value::Empty);
        assert_eq!(val(&w, "I5"), Value::text("old"));
    }

    #[test]
    fn copy_to_headings_choose_and_order_the_columns() {
        let mut w = list(&[("I1", "city"), ("J1", "Name")]);
        let dest = Rect::parse("I1:J1").unwrap();
        let extent = w.advanced_filter_copy(LIST, None, true, 0, dest).unwrap();
        w.commit("Advanced Filter");
        // Unique on the columns copied: Dupont / Paris once.
        assert_eq!(extent, Rect::parse("I1:J6").unwrap());
        assert_eq!(val(&w, "I1"), Value::text("City"));
        assert_eq!(val(&w, "I2"), Value::text("Paris"));
        assert_eq!(val(&w, "J2"), Value::text("Dupont"));
        assert_eq!(val(&w, "J6"), Value::text("Petit"));
        assert_eq!(val(&w, "K2"), Value::Empty);
        // A range several rows tall takes what it holds.
        let mut w = list(&[]);
        let extent = w
            .advanced_filter_copy(LIST, None, false, 0, Rect::parse("I1:K3").unwrap())
            .unwrap();
        assert_eq!(extent, Rect::parse("I1:K3").unwrap());
    }

    #[test]
    fn refusals() {
        let mut w = list(&[("I1", "Colour"), ("E1", "City")]);
        assert_eq!(
            w.advanced_filter_copy(LIST, None, false, 0, Rect::parse("I1").unwrap()),
            Err(AdvancedFilterError::BadExtract)
        );
        assert_eq!(
            w.advanced_filter_copy(LIST, None, false, 0, Rect::cell(MAX_ROWS - 3, 8)),
            Err(AdvancedFilterError::BadDestination)
        );
        // Criteria without a row of conditions, a list without records.
        assert_eq!(
            w.advanced_filter_rows(LIST, range("E1"), false, &[]),
            Err(AdvancedFilterError::BadCriteria)
        );
        assert_eq!(
            w.advanced_filter_rows((0, Rect::parse("A1:C1").unwrap()), None, false, &[]),
            Err(AdvancedFilterError::NoList)
        );
        assert_eq!(
            w.advanced_filter_rows((0, Rect::parse("M1:N9").unwrap()), None, false, &[]),
            Err(AdvancedFilterError::NoList)
        );
    }
}
