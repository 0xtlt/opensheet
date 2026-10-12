//! Data validation: the dialog, the in-cell drop-down list, the input
//! message, the error alerts, the circles around invalid data.
//!
//! The rules live in the engine (`Sheet::validations`). This module shows
//! them on the sheet (the arrow beside a cell with a list, the yellow note
//! of an input message, the red circles), checks what is typed against
//! them (`may_commit`). Entries that are not typed are never refused, as
//! in Excel; the engine makes the rules follow the cells that are pasted,
//! filled, moved and cleared.

mod dialog;
mod list;
mod textarea;

use super::{
    Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget, PaintLayer, Painter, PendingEdit, Verdict, Zone,
    ZoneEvent, ZoneResponse,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::GridLayout;
use crate::i18n::t;
use crate::shell::Alert;
use crate::theme::c;
use gpui::{AnyElement, CursorStyle, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::ops::Direction;
use opensheet_engine::validation::{AlertStyle, EntryCheck};
use opensheet_engine::{Rect, Workbook};
use std::cell::RefCell;
use std::collections::HashMap;

pub struct Validation;

commands! {
    /// What the user can ask of data validation.
    pub enum Cmd {
        Menu = "validation", t("Data Validation", "Validation des données");
        Open = "validation", t("Data Validation…", "Validation des données…");
        CircleInvalid = "", t("Circle Invalid Data", "Entourer les données non valides");
        ClearCircles = "", t("Clear Validation Circles", "Effacer les cercles de validation");
        PickFromList = "", t("Pick From Drop-down List…", "Liste déroulante de choix…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        // The rules of a protected sheet cannot be looked at or changed.
        self == Cmd::PickFromList || !app.wb.is_sheet_protected(app.sheet)
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::Menu => {
                let entries = vec![
                    MenuEntry::command(Cmd::Open, app),
                    MenuEntry::command(Cmd::CircleInvalid, app),
                    MenuEntry::command(Cmd::ClearCircles, app),
                ];
                app.open_command_menu(entries, 270.0, window, cx);
            }
            Cmd::Open => dialog::open(app, window, cx),
            Cmd::CircleInvalid => {
                let sheet = app.sheet;
                let found = app.wb.invalid_cells(sheet);
                let warning = found.warning(app.wb.locale);
                let name = app.wb.sheets[sheet].name.clone();
                app.ext_mut::<Circles>().set(name, found.cells);
                if let Some(warning) = warning {
                    app.alert(Alert::new("OpenSheet", warning).warning(), cx);
                }
                cx.notify();
            }
            Cmd::ClearCircles => {
                let name = app.wb.sheets[app.sheet].name.clone();
                app.ext_mut::<Circles>().clear(&name);
                cx.notify();
            }
            Cmd::PickFromList => {
                list::open(app, window, cx);
            }
        }
    }
}

// ------------------------------------------------------------------ circles

/// The cells "Circle Invalid Data" marked, sheet by sheet (by name). A
/// circle goes away when its cell is corrected; new mistakes are only
/// circled when the command is run again.
#[derive(Default)]
struct Circles {
    marked: HashMap<String, Vec<(u32, u32)>>,
    /// The marked cells of one sheet that still break their rule, as of a
    /// recalculation: painting asks at every frame.
    remaining: RefCell<Option<(String, u64, Vec<(u32, u32)>)>>,
}

impl Circles {
    fn set(&mut self, sheet: String, cells: Vec<(u32, u32)>) {
        self.marked.insert(sheet, cells);
        self.remaining.take();
    }

    fn clear(&mut self, sheet: &str) {
        self.marked.remove(sheet);
        self.remaining.take();
    }

    /// The cells of a sheet to draw a circle around.
    fn shown(&self, wb: &Workbook, sheet: usize) -> Vec<(u32, u32)> {
        let name = &wb.sheets[sheet].name;
        let Some(marked) = self.marked.get(name) else {
            return Vec::new();
        };
        let mut remaining = self.remaining.borrow_mut();
        match remaining.as_ref() {
            Some((of, generation, cells)) if of == name && *generation == wb.generation => cells.clone(),
            _ => {
                let cells: Vec<(u32, u32)> = marked
                    .iter()
                    .copied()
                    .filter(|(row, col)| !wb.cell_is_valid(sheet, *row, *col))
                    .collect();
                *remaining = Some((name.clone(), wb.generation, cells.clone()));
                cells
            }
        }
    }
}

/// The points of the oval Excel draws around an invalid cell: wider and
/// taller than the cell, so that it cuts through its neighbours.
fn circle_points(cell: (f32, f32, f32, f32), zoom: f32) -> Vec<(f32, f32)> {
    let (x0, y0, x1, y1) = cell;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = ((x1 - x0) / 2.0 + 6.0 * zoom, (y1 - y0) / 2.0 + 4.0 * zoom);
    let steps = 48;
    let mut points: Vec<(f32, f32)> = (0..steps)
        .map(|i| {
            let angle = i as f32 / steps as f32 * std::f32::consts::TAU;
            (cx + rx * angle.cos(), cy + ry * angle.sin())
        })
        .collect();
    // The line ends where it began.
    points.push(points[0]);
    points
}

// -------------------------------------------------------------- on the sheet

/// The zone of the arrow that opens the list of the active cell.
struct DropArrow;

/// Side of the arrow button, which does not grow with the zoom.
const ARROW_SIZE: f32 = 17.0;

/// Where the arrow of a cell is drawn: against its right edge, at its foot.
fn arrow_box(cell: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (_, y0, x1, y1) = cell;
    let height = ARROW_SIZE.min(y1 - y0);
    (x1, y1 - height, x1 + ARROW_SIZE, y1)
}

/// The cell (or merged cell) the active cell stands for.
fn active_rect(app: &ExcelApp) -> Rect {
    let (row, col) = app.active();
    app.wb.sheets[app.sheet].merge_at(row, col).unwrap_or(Rect::cell(row, col))
}

/// Whether a cell is on screen, in the frozen panes or the scrolled part.
fn cell_shown(layout: &GridLayout, row: u32, col: u32) -> bool {
    (row < layout.frozen_rows || layout.scrolled_rows().contains(&row))
        && (col < layout.frozen_cols || layout.scrolled_cols().contains(&col))
}

/// The message box for an entry its rule refuses: what the user may do
/// next depends on the style of the alert.
fn error_alert(alert: opensheet_engine::validation::Alert, advance: Option<Direction>) -> Alert {
    let title: SharedString = if alert.title.is_empty() {
        "OpenSheet".into()
    } else {
        alert.title.into()
    };
    // Back to the entry, selected so that typing replaces it.
    let retry = |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| {
        app.focus_editor(window, cx);
        app.editor().update(cx, |input, cx| input.select_all_text(cx));
    };
    let keep = move |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| {
        app.commit_edit_unchecked(advance, window, cx);
    };
    let cancel = |app: &mut ExcelApp, window: &mut Window, cx: &mut Cx| app.cancel_edit(window, cx);
    let message = Alert::new(title, alert.message);
    match alert.style {
        AlertStyle::Stop => message
            .stop()
            .button(t("Retry", "Réessayer"), retry)
            .button(t("Cancel", "Annuler"), cancel),
        // Excel proposes "No": Return does not let the entry through.
        AlertStyle::Warning => message
            .warning()
            .button(t("Yes", "Oui"), keep)
            .button(t("No", "Non"), retry)
            .button(t("Cancel", "Annuler"), cancel)
            .default_button(1),
        AlertStyle::Information => message
            .button(t("OK", "OK"), keep)
            .button(t("Cancel", "Annuler"), cancel),
    }
}

impl Feature for Validation {
    fn name(&self) -> &'static str {
        "validation"
    }

    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Over {
            return;
        }
        let app = p.app;
        let sheet = &app.wb.sheets[app.sheet];
        // Red ovals around the cells found invalid.
        if let Some(circles) = app.ext::<Circles>() {
            for (row, col) in circles.shown(&app.wb, app.sheet) {
                if !p.visible.contains(row, col) {
                    continue;
                }
                let rect = sheet.merge_at(row, col).unwrap_or(Rect::cell(row, col));
                let points = circle_points(p.cell_box(rect), p.zoom());
                p.polyline(&points, 1.6, c(0xFF0000));
            }
        }
        // The arrow of the list, beside the active cell.
        let (row, col) = app.active();
        let rect = active_rect(app);
        if app.object_sel.is_some()
            || !p.visible.contains(rect.r1, rect.c1)
            || !app.wb.has_validation_dropdown(app.sheet, row, col)
        {
            return;
        }
        let (x0, y0, x1, y1) = arrow_box(p.cell_box(rect));
        p.rounded(x0, y0, x1, y1, 2.0, c(0xF3F3F3), Some(c(0xA6A6A6)));
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        p.polygon(&[(mx - 4.0, my - 2.0), (mx + 4.0, my - 2.0), (mx, my + 2.5)], c(0x444444));
        p.zone((x0, y0, x1, y1), DropArrow).cursor(CursorStyle::Arrow);
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        if zone.payload::<DropArrow>().is_none() || ev.button != gpui::MouseButton::Left {
            return ZoneResponse::Ignored;
        }
        list::open(app, window, cx);
        ZoneResponse::Handled
    }

    /// The input message of the active cell, in Excel's yellow note under
    /// the cell.
    fn overlay_elements(&self, app: &mut ExcelApp, layout: &GridLayout, _window: &mut Window, _cx: &mut Cx) -> Vec<AnyElement> {
        let (row, col) = app.active();
        let Some((title, text)) = app.wb.input_message(app.sheet, row, col) else {
            return Vec::new();
        };
        let rect = active_rect(app);
        if app.object_sel.is_some() || !cell_shown(layout, rect.r1, rect.c0) {
            return Vec::new();
        }
        let (x0, _, x1, y1) = layout.rect_px(rect);
        let lines: Vec<SharedString> = text.split('\n').map(|line| line.to_string().into()).collect();
        vec![
            div()
                .absolute()
                .left(px(x0 + ((x1 - x0) / 2.0).min(40.0)))
                .top(px(y1 + 5.0))
                .max_w(px(260.))
                .px(px(5.))
                .py(px(3.))
                .bg(c(0xFFFFE1))
                .border_1()
                .border_color(c(0x767676))
                .shadow_sm()
                .text_size(px(11.5))
                .line_height(px(15.))
                .text_color(c(0x000000))
                .when(!title.is_empty(), |d| {
                    d.child(div().font_weight(FontWeight::BOLD).child(SharedString::from(title.to_string())))
                })
                .when(!text.is_empty(), |d| {
                    d.children(lines.into_iter().map(|line| div().min_h(px(15.)).child(line)))
                })
                .into_any_element(),
        ]
    }

    fn may_commit(&self, app: &mut ExcelApp, edit: &PendingEdit, _window: &mut Window, cx: &mut Cx) -> Verdict {
        match app.wb.check_entry(edit.sheet, edit.row, edit.col, edit.text) {
            EntryCheck::Accepted => Verdict::Accept,
            EntryCheck::Invalid(alert) => {
                app.alert(error_alert(alert, edit.advance), cx);
                Verdict::Hold
            }
        }
    }

    /// Alt+Down: the list of the active cell.
    fn cell_drop_down(&self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> bool {
        list::open(app, window, cx)
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::PickFromList, app));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;
    use opensheet_engine::validation::{Operator, Validation as Rule, ValidationKind};

    /// A workbook whose A1:A3 only take whole numbers from 1 to 10.
    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        let mut rule = Rule {
            kind: ValidationKind::Whole,
            operator: Operator::Between,
            ..Default::default()
        };
        rule.set_operands("1", "10", Locale::en()).unwrap();
        let range = Rect::parse("A1:A3").unwrap();
        wb.set_validation(0, &[range], (0, 0), rule);
        for (row, text) in ["5", "50", "abc"].into_iter().enumerate() {
            wb.set_cell_text(0, row as u32, 0, text).unwrap();
        }
        wb.commit("setup");
        wb
    }

    #[test]
    fn circles_go_when_cells_are_corrected() {
        let mut wb = workbook();
        let mut circles = Circles::default();
        assert!(circles.shown(&wb, 0).is_empty());
        let found = wb.invalid_cells(0);
        circles.set("Sheet1".to_string(), found.cells);
        assert_eq!(circles.shown(&wb, 0), [(1, 0), (2, 0)]);
        // Correcting a cell removes its circle...
        wb.set_cell_text(0, 1, 0, "7").unwrap();
        wb.commit("typing");
        assert_eq!(circles.shown(&wb, 0), [(2, 0)]);
        // ... a new mistake is not circled until the command runs again...
        wb.set_cell_text(0, 0, 0, "99").unwrap();
        wb.commit("typing");
        assert_eq!(circles.shown(&wb, 0), [(2, 0)]);
        // ... and undoing the correction brings the circle back.
        wb.undo();
        wb.undo();
        assert_eq!(circles.shown(&wb, 0), [(1, 0), (2, 0)]);
        circles.clear("Sheet1");
        assert!(circles.shown(&wb, 0).is_empty());
    }

    #[test]
    fn circles_belong_to_their_sheet() {
        let mut wb = workbook();
        let mut circles = Circles::default();
        circles.set("Sheet1".to_string(), wb.invalid_cells(0).cells);
        let other = wb.add_sheet(1);
        wb.commit("sheet");
        assert!(circles.shown(&wb, other).is_empty());
        assert_eq!(circles.shown(&wb, 0).len(), 2);
    }

    #[test]
    fn the_oval_reaches_beyond_its_cell() {
        let points = circle_points((100.0, 40.0, 164.0, 60.0), 1.0);
        let left = points.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let right = points.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let top = points.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let bottom = points.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        assert!((left - 94.0).abs() < 0.1 && (right - 170.0).abs() < 0.1);
        assert!((top - 36.0).abs() < 0.1 && (bottom - 64.0).abs() < 0.1);
        // A closed line.
        assert_eq!(points.first(), points.last());
    }

    #[test]
    fn the_arrow_sits_at_the_foot_of_the_cell() {
        assert_eq!(arrow_box((10.0, 20.0, 74.0, 40.0)), (74.0, 23.0, 91.0, 40.0));
        // A tall (merged) cell keeps it at the bottom, a low row squeezes it.
        assert_eq!(arrow_box((10.0, 20.0, 74.0, 120.0)), (74.0, 103.0, 91.0, 120.0));
        assert_eq!(arrow_box((10.0, 20.0, 74.0, 30.0)), (74.0, 20.0, 91.0, 30.0));
    }
}
