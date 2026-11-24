//! Checkboxes in cells (Insert > Checkbox): a cell format that shows TRUE
//! as a ticked box and FALSE, or nothing, as an empty one. The value stays
//! a plain logical value that formulas read; a click turns it over.

use crate::workbook::Content;
use crate::{Rect, Value, Workbook};

/// Largest area whose empty cells Insert > Checkbox sets to FALSE (whole
/// rows and columns only take the format).
const MAX_FILLED: u64 = 1 << 20;

impl Workbook {
    /// The state of the checkbox a cell shows: `Some(ticked)` when its
    /// format is a checkbox and it holds a logical value or nothing;
    /// `None` when it shows its value as usual.
    pub fn checkbox_at(&self, sheet: usize, row: u32, col: u32) -> Option<bool> {
        if sheet >= self.sheets.len() || !self.style_at(sheet, row, col).checkbox {
            return None;
        }
        match self.value_at(sheet, row, col) {
            Value::Bool(on) => Some(on),
            Value::Empty => Some(false),
            _ => None,
        }
    }

    /// Whether a click may turn the checkbox of a cell over: it shows one
    /// and holds no formula.
    pub fn checkbox_toggles(&self, sheet: usize, row: u32, col: u32) -> bool {
        self.checkbox_at(sheet, row, col).is_some()
            && !matches!(
                self.cell(sheet, row, col).map(|c| &c.content),
                Some(Content::Formula(_))
            )
    }

    /// Insert > Checkbox: the cells of the ranges show a checkbox, and
    /// those that hold nothing become FALSE, as in Excel.
    pub fn insert_checkboxes(&mut self, sheet: usize, ranges: &[Rect]) {
        if sheet >= self.sheets.len() || ranges.is_empty() {
            return;
        }
        self.snapshot();
        for rect in ranges {
            self.update_style(sheet, *rect, &|s| s.checkbox = true);
            let area = rect.rows() as u64 * rect.cols() as u64;
            if rect.is_whole_cols() || rect.is_whole_rows() || area > MAX_FILLED {
                continue;
            }
            for row in rect.r0..=rect.r1 {
                for col in rect.c0..=rect.c1 {
                    if self.value_at(sheet, row, col) == Value::Empty
                        && !matches!(
                            self.cell(sheet, row, col).map(|c| &c.content),
                            Some(Content::Formula(_))
                        )
                    {
                        self.set_value(sheet, row, col, Value::Bool(false));
                    }
                }
            }
        }
    }

    /// Turn over the checkboxes of the ranges: they all take the opposite
    /// of the state of `lead` (the active cell, or the one clicked). Cells
    /// that show no checkbox or hold a formula are left alone. False when
    /// nothing changed.
    pub fn toggle_checkboxes(&mut self, sheet: usize, ranges: &[Rect], lead: (u32, u32)) -> bool {
        let Some(on) = self.checkbox_at(sheet, lead.0, lead.1) else {
            return false;
        };
        let mut cells: Vec<(u32, u32)> = Vec::new();
        for rect in ranges {
            let rect = self.clamp_to_used(sheet, *rect);
            if rect.rows() as u64 * rect.cols() as u64 > MAX_FILLED {
                continue;
            }
            for row in rect.r0..=rect.r1 {
                for col in rect.c0..=rect.c1 {
                    if self.checkbox_toggles(sheet, row, col) {
                        cells.push((row, col));
                    }
                }
            }
        }
        if !cells.contains(&lead) && self.checkbox_toggles(sheet, lead.0, lead.1) {
            cells.push(lead);
        }
        if cells.is_empty() {
            return false;
        }
        self.snapshot();
        for (row, col) in cells {
            self.set_value(sheet, row, col, Value::Bool(!on));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;

    fn book() -> Workbook {
        Workbook::new(Locale::en())
    }

    #[test]
    fn inserting_makes_empty_cells_false_and_keeps_values() {
        let mut wb = book();
        wb.set_value(0, 1, 0, Value::Bool(true));
        wb.set_value(0, 2, 0, Value::Number(3.0));
        wb.insert_checkboxes(0, &[Rect::new(0, 0, 2, 0)]);
        assert_eq!(wb.value_at(0, 0, 0), Value::Bool(false));
        assert_eq!(wb.checkbox_at(0, 0, 0), Some(false));
        assert_eq!(wb.checkbox_at(0, 1, 0), Some(true));
        // A number keeps showing as a number.
        assert_eq!(wb.checkbox_at(0, 2, 0), None);
        assert!(wb.style_at(0, 2, 0).checkbox);
    }

    #[test]
    fn toggling_follows_the_lead_cell_and_skips_formulas() {
        let mut wb = book();
        wb.insert_checkboxes(0, &[Rect::new(0, 0, 2, 0)]);
        wb.set_value(0, 1, 0, Value::Bool(true));
        let expr = crate::parser::parse_formula("TRUE()", Locale::en()).expect("a formula");
        wb.set_formula(0, 2, 0, expr);
        assert!(wb.toggle_checkboxes(0, &[Rect::new(0, 0, 2, 0)], (0, 0)));
        assert_eq!(wb.value_at(0, 0, 0), Value::Bool(true));
        assert_eq!(wb.value_at(0, 1, 0), Value::Bool(true));
        assert!(!wb.checkbox_toggles(0, 2, 0));
        // A cell without the format is no checkbox.
        assert!(!wb.toggle_checkboxes(0, &[Rect::cell(5, 5)], (5, 5)));
    }
}
