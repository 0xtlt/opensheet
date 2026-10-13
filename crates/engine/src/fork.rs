//! A working copy of a workbook, for work done away from the window.
//!
//! A script may run for a long time. It runs on a copy ([`Workbook::fork`])
//! in another thread while the window goes on showing the workbook; when
//! it ends well the workbook takes over what the copy has become
//! ([`Workbook::adopt`]) as one undo step, and when it fails the copy is
//! dropped: nothing changed.

use crate::recalc::CalcState;
use crate::workbook::Workbook;

impl Workbook {
    /// A copy of the workbook as it is now: the same sheets, names,
    /// styles and settings, calculated, with no undo history.
    pub fn fork(&self) -> Workbook {
        let mut copy = Workbook::new(self.locale);
        copy.sheets = self.sheets.clone();
        copy.styles = self.styles.clone();
        copy.names = self.names.clone();
        copy.cell_styles = self.cell_styles.clone();
        copy.protection = self.protection.clone();
        copy.vba = self.vba.clone();
        copy.circular = self.circular;
        // The values came with the sheets: the copy is calculated.
        copy.calc = CalcState::of_calculated(&copy);
        copy.set_iterative_calc(self.iterative_calc());
        copy.set_calc_mode(self.calc_mode());
        copy.clear_history();
        copy.modified = false;
        copy
    }

    /// Take over the sheets, the names and the styles of a copy made with
    /// [`Workbook::fork`] and changed since. Part of the current
    /// transaction: the caller commits, and the whole change is one undo
    /// step. Nothing may have changed here since the copy was made.
    pub fn adopt(&mut self, fork: Workbook) {
        self.snapshot();
        self.sheets = fork.sheets;
        self.names = fork.names;
        self.cell_styles = fork.cell_styles;
        // The copy's table holds every style this one had, under the same
        // numbers, and those the copy added.
        self.styles = fork.styles;
        // A copy whose sheets were protected or unprotected says so: the
        // commit that follows forgets the history, as when a sheet is
        // protected by hand.
        self.protection_pending |= fork.protection_pending;
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use crate::{Locale, Workbook};

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.set_cell_text(0, 0, 0, "2").unwrap();
        wb.set_cell_text(0, 1, 0, "=A1*10").unwrap();
        wb.commit("setup");
        wb
    }

    #[test]
    fn a_copy_is_changed_and_taken_over_as_one_step() {
        let mut wb = workbook();
        let mut copy = wb.fork();
        assert_eq!(copy.value_at(0, 1, 0), Value::Number(20.0));
        assert!(!copy.can_undo() && !copy.modified);

        // The copy changes: a value, a format, a new sheet, a name.
        copy.set_cell_text(0, 0, 0, "5").unwrap();
        copy.update_style(0, crate::Rect::cell(0, 0), &|style| style.font.bold = true);
        let added = copy.add_sheet(1);
        copy.set_cell_text(added, 0, 0, "=Sheet1!A2+1").unwrap();
        copy.commit("script");
        // The workbook has not moved.
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(20.0));
        assert_eq!(wb.sheets.len(), 1);

        wb.adopt(copy);
        assert!(wb.commit("Run Script"));
        assert_eq!(wb.value_at(0, 0, 0), Value::Number(5.0));
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(50.0));
        assert_eq!(wb.value_at(1, 0, 0), Value::Number(51.0));
        assert!(wb.style_at(0, 0, 0).font.bold);
        assert_eq!(wb.undo_label(), Some("Run Script"));

        // One undo takes all of it back, one redo brings it again.
        assert!(wb.undo());
        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.value_at(0, 0, 0), Value::Number(2.0));
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(20.0));
        assert!(!wb.style_at(0, 0, 0).font.bold);
        assert_eq!(wb.undo_label(), Some("setup"));
        assert!(wb.redo());
        assert_eq!(wb.value_at(1, 0, 0), Value::Number(51.0));
        // What is typed afterwards is calculated as ever.
        wb.set_cell_text(0, 0, 0, "7").unwrap();
        wb.commit("edit");
        assert_eq!(wb.value_at(1, 0, 0), Value::Number(71.0));
    }

    #[test]
    fn a_copy_that_is_dropped_changes_nothing() {
        let mut wb = workbook();
        let mut copy = wb.fork();
        copy.set_cell_text(0, 0, 0, "99").unwrap();
        copy.commit("script");
        drop(copy);
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(20.0));
        assert_eq!(wb.undo_label(), Some("setup"));
        wb.set_cell_text(0, 0, 0, "3").unwrap();
        wb.commit("edit");
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(30.0));
    }

    #[test]
    fn a_copy_can_go_to_another_thread() {
        fn sendable<T: Send>(_: &T) {}
        let wb = workbook();
        let mut copy = wb.fork();
        sendable(&copy);
        let copy = std::thread::spawn(move || {
            copy.set_cell_text(0, 0, 0, "4").unwrap();
            copy.commit("script");
            copy
        })
        .join()
        .unwrap();
        assert_eq!(copy.value_at(0, 1, 0), Value::Number(40.0));
    }
}
