//! Review ▸ Workbook Statistics: what the current sheet and the whole
//! workbook hold, counted.
//!
//! Excel's dialog lists, for the sheet in front, where it ends and how
//! many cells with data, tables, PivotTables, formulas, charts, images,
//! form controls, objects, comments and notes it has; then the number of
//! sheets and the same counts for the workbook. A count of zero has no
//! line.

use super::{Command, Cx};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::shell::Modal;
use crate::theme::{self, c};
use crate::widgets::{Button, dialog};
use gpui::{Context, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::comment::CommentKind;
use opensheet_engine::drawing::DrawingKind;
use opensheet_engine::refs::cell_name;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Sheet, Workbook};

commands! {
    /// The one command of workbook statistics.
    pub enum Cmd {
        Open = "calculator", t("Workbook Statistics", "Statistiques du classeur");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        // What is being typed counts once it is in its cell.
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        app.popup = None;
        let (sheet, workbook) = (sheet_lines(&app.wb.sheets[app.sheet]), workbook_lines(&app.wb));
        app.open_modal(window, cx, |modal, _, _| StatsDialog { modal, sheet, workbook });
    }
}

// ----------------------------------------------------------------- counting

/// What a sheet holds. Counts add up over the sheets of a workbook.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Cells with a constant or a formula.
    pub cells: usize,
    pub tables: usize,
    pub pivots: usize,
    pub formulas: usize,
    pub charts: usize,
    /// Pictures.
    pub images: usize,
    /// Buttons, check boxes and the like: OpenSheet has none, so this is
    /// always zero and its line never shows.
    pub controls: usize,
    /// Everything else that floats over the cells: shapes, text boxes,
    /// WordArt, SmartArt, groups, ink, slicers and timelines.
    pub objects: usize,
    /// Comment threads.
    pub comments: usize,
    pub notes: usize,
}

impl Counts {
    pub fn of(sheet: &Sheet) -> Counts {
        let mut counts = Counts {
            tables: sheet.tables.len(),
            pivots: sheet.pivots.len(),
            charts: sheet.charts.len(),
            comments: sheet.comment_cells(Some(CommentKind::Thread)).len(),
            notes: sheet.comment_cells(Some(CommentKind::Note)).len(),
            objects: sheet.inks.len() + sheet.slicers.len() + sheet.timelines.len(),
            ..Counts::default()
        };
        for cell in sheet.cells.values() {
            match cell.content {
                Content::Empty => {}
                Content::Value(_) => counts.cells += 1,
                Content::Formula(_) => {
                    counts.cells += 1;
                    counts.formulas += 1;
                }
            }
        }
        for drawing in &sheet.drawings {
            match drawing.kind {
                DrawingKind::Picture(_) => counts.images += 1,
                DrawingKind::Shape(_) | DrawingKind::SmartArt(_) | DrawingKind::Group(_) => counts.objects += 1,
            }
        }
        counts
    }

    fn add(&mut self, other: &Counts) {
        self.cells += other.cells;
        self.tables += other.tables;
        self.pivots += other.pivots;
        self.formulas += other.formulas;
        self.charts += other.charts;
        self.images += other.images;
        self.controls += other.controls;
        self.objects += other.objects;
        self.comments += other.comments;
        self.notes += other.notes;
    }

    /// The lines of the dialog, in Excel's order, those at zero left out.
    fn lines(&self) -> Vec<(&'static str, String)> {
        [
            (t("Cells with data", "Cellules contenant des données"), self.cells),
            (t("Tables", "Tableaux"), self.tables),
            (t("PivotTables", "Tableaux croisés dynamiques"), self.pivots),
            (t("Formulas", "Formules"), self.formulas),
            (t("Charts", "Graphiques"), self.charts),
            (t("Images", "Images"), self.images),
            (t("Form controls", "Contrôles de formulaire"), self.controls),
            (t("Objects", "Objets"), self.objects),
            (t("Comments", "Commentaires"), self.comments),
            (t("Notes", "Notes"), self.notes),
        ]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(label, count)| (label, count.to_string()))
        .collect()
    }
}

/// The last cell of a sheet: the bottom-right corner of what holds data
/// or formatting, A1 on an empty sheet.
pub(crate) fn end_of_sheet(sheet: &Sheet) -> String {
    sheet.used_rect().map_or_else(|| cell_name(0, 0), |used| cell_name(used.r1, used.c1))
}

/// The lines under "Current Sheet:".
pub(crate) fn sheet_lines(sheet: &Sheet) -> Vec<(&'static str, String)> {
    let mut lines = vec![(t("End of sheet", "Fin de la feuille"), end_of_sheet(sheet))];
    lines.extend(Counts::of(sheet).lines());
    lines
}

/// The lines under "Workbook:".
pub(crate) fn workbook_lines(wb: &Workbook) -> Vec<(&'static str, String)> {
    let mut total = Counts::default();
    for sheet in &wb.sheets {
        total.add(&Counts::of(sheet));
    }
    let mut lines = vec![(t("Sheets", "Feuilles"), wb.sheets.len().to_string())];
    lines.extend(total.lines());
    lines
}

// ------------------------------------------------------------------- dialog

pub(crate) struct StatsDialog {
    modal: Modal,
    pub sheet: Vec<(&'static str, String)>,
    pub workbook: Vec<(&'static str, String)>,
}

/// A heading and its lines: the labels on the left, the numbers in a
/// column on the right.
fn section(title: &'static str, lines: &[(&'static str, String)]) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .children(lines.iter().map(|(label, value)| {
            div()
                .flex()
                .pl(px(12.))
                .gap(px(12.))
                .child(div().flex_1().child(*label))
                .child(div().min_w(px(70.)).text_color(c(theme::TEXT)).child(SharedString::from(value.clone())))
        }))
}

impl Render for StatsDialog {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let close = self.modal.clone();
        let ok = self.modal.clone();
        dialog(&self.modal, Cmd::Open.label())
            .width(360.0)
            .on_ok(move |_, window, cx| ok.close(window, cx))
            .child(section(t("Current Sheet:", "Feuille active\u{a0}:"), &self.sheet))
            .child(section(t("Workbook:", "Classeur\u{a0}:"), &self.workbook))
            .footer(
                Button::new("dialog-ok", t("OK", "OK"))
                    .primary()
                    .on_click(move |_, window, cx| close.close(window, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;
    use opensheet_engine::comment::Person;

    fn pairs(lines: Vec<(&'static str, String)>) -> Vec<(String, String)> {
        lines.into_iter().map(|(label, value)| (label.to_string(), value)).collect()
    }

    fn pair(label: &str, value: &str) -> (String, String) {
        (label.to_string(), value.to_string())
    }

    #[test]
    fn an_empty_workbook_has_one_sheet_that_ends_at_a1() {
        let wb = Workbook::new(Locale::en());
        assert_eq!(pairs(sheet_lines(&wb.sheets[0])), [pair("End of sheet", "A1")]);
        assert_eq!(pairs(workbook_lines(&wb)), [pair("Sheets", "1")]);
    }

    #[test]
    fn what_is_there_is_counted_and_nothing_else_is_listed() {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        for (cell, text) in [((0, 0), "Item"), ((0, 1), "Price"), ((1, 0), "Tea"), ((1, 1), "3"), ((2, 1), "=B2*2")] {
            wb.set_cell_text(0, cell.0, cell.1, text).unwrap();
        }
        wb.set_cell_text(1, 9, 3, "=SUM(1,2)").unwrap();
        wb.set_note(0, 4, 4, "Ada", "Ada:\nA note");
        wb.add_comment(0, 5, 0, &Person::new("Bob"), 45_000.0, "A thread");
        wb.add_comment(1, 0, 0, &Person::new("Bob"), 45_000.0, "Another");
        wb.commit("setup");
        // The sheet in front: its last cell is under the note's row only
        // if a cell is there; here it is B3.
        assert_eq!(
            pairs(sheet_lines(&wb.sheets[0])),
            [
                pair("End of sheet", "B3"),
                pair("Cells with data", "5"),
                pair("Formulas", "1"),
                pair("Comments", "1"),
                pair("Notes", "1"),
            ]
        );
        assert_eq!(
            pairs(sheet_lines(&wb.sheets[1])),
            [
                pair("End of sheet", "D10"),
                pair("Cells with data", "1"),
                pair("Formulas", "1"),
                pair("Comments", "1"),
            ]
        );
        assert_eq!(
            pairs(workbook_lines(&wb)),
            [
                pair("Sheets", "2"),
                pair("Cells with data", "6"),
                pair("Formulas", "2"),
                pair("Comments", "2"),
                pair("Notes", "1"),
            ]
        );
    }

    #[test]
    fn every_kind_of_thing_has_its_line_in_excels_order() {
        let all = Counts {
            cells: 1,
            tables: 2,
            pivots: 3,
            formulas: 4,
            charts: 5,
            images: 6,
            controls: 7,
            objects: 8,
            comments: 9,
            notes: 10,
        };
        let labels: Vec<&str> = all.lines().into_iter().map(|(label, _)| label).collect();
        assert_eq!(
            labels,
            [
                "Cells with data",
                "Tables",
                "PivotTables",
                "Formulas",
                "Charts",
                "Images",
                "Form controls",
                "Objects",
                "Comments",
                "Notes"
            ]
        );
        let mut twice = all;
        twice.add(&all);
        assert_eq!((twice.cells, twice.notes, twice.controls), (2, 20, 14));
    }
}
