//! Insert > Checkbox: cells that show TRUE and FALSE as a ticked or an
//! empty box. A click on the box, or Space on the selection, turns it over.
//! The grid draws the boxes (`grid::checkbox_box`); this module registers
//! where they are, so that a click reaches them.

#[cfg(test)]
mod ui_tests;

use super::{
    ChangeCause, Command, Cx, Feature, Intent, PaintLayer, Painter, Zone, ZoneEvent, ZoneResponse,
};
use crate::app::ExcelApp;
use crate::i18n::t;
use gpui::{CursorStyle, MouseButton, Window};
use opensheet_engine::Rect;

pub struct Checkboxes;

commands! {
    /// What the user can ask of checkboxes in cells.
    pub enum Cmd {
        Insert = "checkbox", t("Checkbox", "Case à cocher");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, _: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::Insert => {
                let ranges = app.sel().ranges.clone();
                // A format, and FALSE in the empty cells.
                for rect in &ranges {
                    if !app.may(Intent::FormatCells(*rect), cx)
                        || !app.may(Intent::EditCells(*rect), cx)
                    {
                        return;
                    }
                }
                app.wb.insert_checkboxes(app.sheet, &ranges);
                app.commit_cells(self.label(), &ranges, ChangeCause::Typing, cx);
            }
        }
    }
}

/// Turn over the checkboxes of the selection, from the state of `lead`.
/// False when `lead` shows no checkbox that may change (the caller then
/// does what it would have done).
fn toggle(app: &mut ExcelApp, ranges: Vec<Rect>, lead: (u32, u32), cx: &mut Cx) -> bool {
    if !app.wb.checkbox_toggles(app.sheet, lead.0, lead.1) {
        return false;
    }
    for rect in &ranges {
        if !app.may(Intent::EditCells(*rect), cx) {
            return true;
        }
    }
    if app.wb.toggle_checkboxes(app.sheet, &ranges, lead) {
        app.commit_cells(Cmd::Insert.label(), &ranges, ChangeCause::Typing, cx);
    }
    true
}

/// Space typed on the grid: when the active cell is a checkbox, the
/// checkboxes of the selection turn over instead of an edit starting.
pub fn space(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let ranges = app.sel().ranges.clone();
    let lead = app.active();
    toggle(app, ranges, lead, cx)
}

/// A zone of this module: the box of the checkbox of a cell.
struct Box_(u32, u32);

impl Feature for Checkboxes {
    fn name(&self) -> &'static str {
        "checkboxes"
    }

    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Over || crate::features::misc::shows_formulas(p.app) {
            return;
        }
        let wb = &p.app.wb;
        let sheet = &wb.sheets[p.app.sheet];
        let v = p.visible;
        let cells: Vec<(u32, u32)> = sheet
            .cells
            .range((v.r0, v.c0)..=(v.r1, v.c1))
            .map(|(at, _)| *at)
            .filter(|(_, col)| *col >= v.c0 && *col <= v.c1)
            .collect();
        for (row, col) in cells {
            if !wb.checkbox_toggles(p.app.sheet, row, col) {
                continue;
            }
            let rect = sheet
                .merges
                .iter()
                .find(|m| m.contains(row, col))
                .copied()
                .unwrap_or(Rect::cell(row, col));
            if (rect.r0, rect.c0) != (row, col) {
                continue;
            }
            let style = wb.styles.get(sheet.cells[&(row, col)].style);
            let b = crate::grid::checkbox_box(style, p.cell_box(rect), p.zoom());
            p.zone(b, Box_(row, col)).cursor(CursorStyle::PointingHand);
        }
    }

    fn zone_mouse_down(
        &self,
        app: &mut ExcelApp,
        zone: &Zone,
        ev: &ZoneEvent,
        _window: &mut Window,
        cx: &mut Cx,
    ) -> ZoneResponse {
        let Some(Box_(row, col)) = zone.payload::<Box_>() else {
            return ZoneResponse::Ignored;
        };
        // Shift and Command clicks extend the selection as usual.
        if ev.button != MouseButton::Left
            || ev.click_count > 1
            || ev.modifiers.shift
            || ev.modifiers.platform
            || app.editing.is_some()
        {
            return ZoneResponse::Ignored;
        }
        let (row, col) = (*row, *col);
        app.select_cell(row, col, cx);
        toggle(app, vec![Rect::cell(row, col)], (row, col), cx);
        ZoneResponse::Handled
    }
}
