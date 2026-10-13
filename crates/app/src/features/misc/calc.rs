//! The Calculation group: Calculation Options, Calculate Now and
//! Calculate Sheet; what the status bar says of calculation ("Calculate",
//! "Circular References: B2"); the warning when a circular reference
//! appears.
//!
//! The workbook holds the calculation mode and knows what is waiting to be
//! calculated (`opensheet_engine::recalc`); this file is the only place of
//! the application that asks it.

use crate::app::ExcelApp;
use crate::features::{Cx, StatusItem};
use crate::shell::Alert;
use crate::i18n::t;
use opensheet_engine::refs::cell_name;
use std::rc::Rc;

pub use opensheet_engine::CalcMode;

/// What was last seen of the workbook, to tell when something appears.
#[derive(Default)]
struct Seen {
    circular: bool,
}

/// The calculation mode of the workbook.
pub fn calc_mode(app: &ExcelApp) -> CalcMode {
    app.wb.calc_mode()
}

/// Choose the calculation mode. Leaving Manual calculates what waited.
pub fn set_calc_mode(app: &mut ExcelApp, mode: CalcMode) {
    app.wb.set_calc_mode(mode);
}

/// Calculate Now (F9): what changed since the last calculation, what
/// depends on it and the volatile functions, in the whole workbook.
pub fn calculate_now(app: &mut ExcelApp) {
    app.wb.calculate_now();
}

/// Calculate Sheet (Shift+F9): the same on one sheet, the others read as
/// they are.
pub fn calculate_sheet(app: &mut ExcelApp, sheet: usize) {
    app.wb.calculate_sheet(sheet);
}

/// The status bar: "Calculate" while formulas wait to be calculated
/// (Manual), a click calculating as in Excel, and "Circular References"
/// with a cell of the loop when it is on the sheet shown.
pub fn status_items(app: &ExcelApp) -> Vec<StatusItem> {
    let mut items = Vec::new();
    if app.wb.needs_calculation() {
        let mut item = StatusItem::new(t("Calculate", "Calculer"));
        item.on_click = Some(Rc::new(|app, _, cx| {
            calculate_now(app);
            app.refresh(cx);
        }));
        items.push(item);
    }
    if app.wb.circular {
        let label = t("Circular References", "Références circulaires");
        let here = app.wb.circular_cells().iter().find(|cell| cell.0 == app.sheet);
        items.push(StatusItem::new(match here {
            // French puts a space before the colon.
            Some((_, row, col)) => format!("{label}{} {}", t(":", " :"), cell_name(*row, *col)),
            None => label.to_string(),
        }));
    }
    items
}

/// After a change to the cells: Excel's warning when the workbook comes to
/// hold a circular reference (once, until there is none left).
pub fn after_change(app: &mut ExcelApp, cx: &mut Cx) {
    let circular = app.wb.circular;
    let seen = app.ext_mut::<Seen>();
    let appeared = circular && !seen.circular;
    seen.circular = circular;
    if appeared {
        let message = t(
            "There are one or more circular references where a formula refers to its own cell \
             either directly or indirectly. This might cause them to calculate incorrectly.\n\n\
             Try removing or changing these references, or moving the formulas to different cells.",
            "Il existe une ou plusieurs références circulaires dans lesquelles une formule fait \
             référence à sa propre cellule, directement ou indirectement.\n\nEssayez de supprimer \
             ou de modifier ces références, ou changez les formules de cellule.",
        );
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::Cmd;
    use super::*;
    use crate::features::{ChangeCause, Command};
    use gpui::{Entity, TestAppContext, VisualTestContext};
    use opensheet_engine::{Rect, Value};

    fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
        cx.update(crate::bind_keys);
        let (app, cx) = cx.add_window_view(ExcelApp::new);
        cx.run_until_parked();
        (app, cx)
    }

    /// Type into cells of the first sheet, as one entry.
    fn enter(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cells: &[(&str, &str)]) {
        app.update(cx, |app, cx| {
            let mut rects = Vec::new();
            for (cell, text) in cells {
                let rect = Rect::parse(cell).unwrap();
                app.wb.set_cell_text(0, rect.r0, rect.c0, text).unwrap();
                rects.push(rect);
            }
            app.commit_cells("Typing", &rects, ChangeCause::Typing, cx);
        });
        cx.run_until_parked();
    }

    fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
        app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
        cx.run_until_parked();
    }

    fn status(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<String> {
        app.update(cx, |app, _| status_items(app).iter().map(|item| item.text.to_string()).collect())
    }

    fn number(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: &str) -> Value {
        let rect = Rect::parse(cell).unwrap();
        app.update(cx, |app, _| app.wb.value_at(0, rect.r0, rect.c0))
    }

    #[gpui::test]
    fn manual_calculation_waits_to_be_asked(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        enter(&app, cx, &[("A1", "1"), ("B1", "=A1+1")]);
        assert!(status(&app, cx).is_empty());
        run(&app, cx, Cmd::CalcManual);
        app.update(cx, |app, _| {
            assert_eq!(calc_mode(app), CalcMode::Manual);
            assert!(Cmd::CalcManual.checked(app) && !Cmd::CalcAutomatic.checked(app));
        });
        enter(&app, cx, &[("A1", "5")]);
        assert_eq!(number(&app, cx, "B1"), Value::Number(2.0));
        assert_eq!(status(&app, cx), ["Calculate"]);
        // Calculate Sheet and Calculate Now (F9).
        run(&app, cx, Cmd::CalcSheet);
        assert_eq!(number(&app, cx, "B1"), Value::Number(6.0));
        enter(&app, cx, &[("A1", "7")]);
        run(&app, cx, Cmd::CalcNow);
        assert_eq!(number(&app, cx, "B1"), Value::Number(8.0));
        assert!(status(&app, cx).is_empty());
        // The indicator is a button, as in Excel.
        enter(&app, cx, &[("A1", "9")]);
        app.update_in(cx, |app, window, cx| {
            let click = status_items(app)[0].on_click.clone().expect("a button");
            click(app, window, cx);
        });
        assert_eq!(number(&app, cx, "B1"), Value::Number(10.0));
        // Back to automatic: what waits is calculated at once.
        enter(&app, cx, &[("A1", "11")]);
        run(&app, cx, Cmd::CalcAutomatic);
        assert_eq!(number(&app, cx, "B1"), Value::Number(12.0));
        enter(&app, cx, &[("A1", "13")]);
        assert_eq!(number(&app, cx, "B1"), Value::Number(14.0));
    }

    #[gpui::test]
    fn a_circular_reference_warns_once_and_is_named_in_the_status_bar(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        enter(&app, cx, &[("B2", "=C2+1"), ("C2", "=B2*2")]);
        assert!(app.update(cx, |app, _| app.modal_open()), "the warning");
        assert_eq!(status(&app, cx), ["Circular References: B2"]);
        assert_eq!(number(&app, cx, "B2"), Value::Number(0.0));
        app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
        // A second loop does not warn again.
        enter(&app, cx, &[("E5", "=E5")]);
        assert!(!app.update(cx, |app, _| app.modal_open()));
        // On another sheet the status bar names no cell.
        app.update(cx, |app, cx| {
            app.wb.add_sheet(1);
            app.commit("sheet", cx);
            app.sheet = 1;
        });
        assert_eq!(status(&app, cx), ["Circular References"]);
        // Once there is none left, the next one warns.
        app.update(cx, |app, _| app.sheet = 0);
        enter(&app, cx, &[("B2", "1"), ("E5", "2")]);
        assert!(status(&app, cx).is_empty());
        enter(&app, cx, &[("A9", "=A9")]);
        assert!(app.update(cx, |app, _| app.modal_open()));
    }

    #[gpui::test]
    fn error_checking_lists_the_circular_references(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        // None: the entry is greyed out.
        app.update(cx, |app, _| {
            assert!(super::super::circular_cells(app).is_empty());
            assert!(super::super::circular_references(app).disabled);
        });
        enter(&app, cx, &[("B2", "=B2+1")]);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        app.update_in(cx, |app, window, cx| {
            assert_eq!(super::super::circular_cells(app), vec![("$B$2".to_string(), (0, 1, 1))]);
            // Seen from another sheet, the cell is named with its sheet.
            app.add_sheet(cx);
            let other = (0..app.wb.sheets.len()).find(|index| app.wb.circular_cells().iter().all(|cell| cell.0 != *index)).unwrap();
            app.switch_sheet(other, window, cx);
            let listed = super::super::circular_cells(app);
            assert_eq!(listed.len(), 1);
            let (text, (sheet, row, col)) = listed[0].clone();
            assert_eq!(text, format!("{}!$B$2", app.wb.sheets[sheet].name));
            // Choosing it goes there.
            let entry = super::super::circular_references(app);
            assert!(!entry.disabled);
            let go = entry.submenu.unwrap().remove(0).action.unwrap();
            go(app, window, cx);
            assert_eq!((app.sheet, app.active()), (sheet, (row, col)));
        });
    }
}
