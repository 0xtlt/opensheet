//! View > Limited Grid: the sheet shows only the rows and columns it uses,
//! with a + button after the last column and one under the last row to add
//! more, instead of Excel's grid of a million rows.
//!
//! It is a way of looking at the sheet, not part of it: the workbook and
//! its file do not change, and the setting lasts as long as the window. The
//! grid always reaches the cells that hold something (a value, a format, a
//! merged area), the frozen panes and the selection, so typing at its edge
//! or moving the active cell past it makes it grow.

#[cfg(test)]
mod ui_tests;

use super::{Command, Cx, Feature, PaintLayer, Painter, ViewMode, Zone, ZoneEvent, ZoneResponse};
use crate::app::ExcelApp;
use crate::grid::GridLayout;
use crate::i18n::t;
use crate::theme::{self, c};
use gpui::{CursorStyle, Window};
use opensheet_engine::{MAX_COLS, MAX_ROWS};
use std::collections::HashMap;

pub struct LimitedGrid;

/// Size of the grid a sheet shows when its limit is turned on, before its
/// content makes it larger.
const FIRST_ROWS: u32 = 10;
const FIRST_COLS: u32 = 5;

commands! {
    /// What the user can ask of the limited grid.
    pub enum Cmd {
        Toggle = "", t("Limited Grid", "Grille limitée");
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
            Cmd::Toggle => {
                let name = app.wb.sheets[app.sheet].name.clone();
                let state = app.ext_mut::<State>();
                if state.sheets.remove(&name).is_none() {
                    state.sheets.insert(name, (FIRST_ROWS, FIRST_COLS));
                }
                cx.notify();
            }
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::Toggle => app
                .ext::<State>()
                .is_some_and(|s| s.sheets.contains_key(&app.wb.sheets[app.sheet].name)),
        }
    }
}

/// The sheets shown with a limited grid, by name, with the number of rows
/// and columns asked for (the grid may show more, see [`extent`]).
#[derive(Default)]
struct State {
    sheets: HashMap<String, (u32, u32)>,
}

/// Rows and columns the current sheet shows, `None` when it shows the
/// whole grid. Page Layout view and Page Break Preview always show it whole.
pub fn extent(app: &ExcelApp) -> Option<(u32, u32)> {
    if app.view_mode() != ViewMode::Normal {
        return None;
    }
    let sheet = &app.wb.sheets[app.sheet];
    let &(mut rows, mut cols) = app.ext::<State>()?.sheets.get(&sheet.name)?;
    let mut reach = |row: u32, col: u32| {
        rows = rows.max(row.saturating_add(1));
        cols = cols.max(col.saturating_add(1));
    };
    if let Some(used) = sheet.used_rect() {
        reach(used.r1, used.c1);
    }
    // Spilled arrays have no cells of their own.
    let (r, c) = sheet.content_extent();
    reach(r, c);
    for m in &sheet.merges {
        reach(m.r1, m.c1);
    }
    reach(sheet.frozen_rows, sheet.frozen_cols);
    let sel = app.sel();
    reach(sel.active.0, sel.active.1);
    for range in &sel.ranges {
        // Whole rows and columns do not stretch the grid to the end.
        if !range.is_whole_cols() {
            rows = rows.max(range.r1.saturating_add(1));
        }
        if !range.is_whole_rows() {
            cols = cols.max(range.c1.saturating_add(1));
        }
    }
    Some((rows.min(MAX_ROWS), cols.min(MAX_COLS)))
}

/// What a zone of this module stands for.
enum Spot {
    /// The + after the last column.
    AddCol,
    /// The + under the last row.
    AddRow,
    /// The blank page around the grid: a click there selects nothing.
    Outside,
}

/// Side of a + button, in pixels.
fn button_size(layout: &GridLayout) -> f32 {
    (18.0 * layout.zoom).round().clamp(14.0, 28.0)
}

/// Room between the end of the grid and its + buttons.
const GAP: f32 = 4.0;

/// Boxes (x0, y0, x1, y1) of the column and row + buttons.
fn buttons(layout: &GridLayout) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
    let size = button_size(layout);
    let (end_x, end_y) = layout.cells_end();
    let (mw, mh, hw, hh) = (
        layout.margin_w,
        layout.margin_h,
        layout.header_w,
        layout.header_h,
    );
    // In the heading strip, else at the top / left of the cells.
    let col_y = if hh > mh {
        mh + ((hh - mh - size) / 2.0).round()
    } else {
        hh + GAP
    };
    let row_x = if hw > mw {
        mw + ((hw - mw - size) / 2.0).round()
    } else {
        hw + GAP
    };
    let col_x = end_x + GAP;
    let row_y = end_y + GAP;
    (
        (col_x, col_y, col_x + size, col_y + size),
        (row_x, row_y, row_x + size, row_y + size),
    )
}

impl Feature for LimitedGrid {
    fn name(&self) -> &'static str {
        "limited_grid"
    }

    fn adjust_layout(&self, app: &ExcelApp, layout: &mut GridLayout) {
        let Some((rows, cols)) = extent(app) else {
            return;
        };
        layout.rows.set_count(rows);
        layout.cols.set_count(cols);
        // Scrolling to the end leaves the + buttons in view.
        let room = button_size(layout) + 2.0 * GAP + theme::SCROLLBAR;
        layout.room_w = room;
        layout.room_h = room;
        layout.clamp_scroll();
    }

    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Top || extent(p.app).is_none() {
            return;
        }
        let layout = p.layout;
        let (w, h) = (layout.width(), layout.height());
        let (end_x, end_y) = layout.cells_end();
        // Clicks past the grid select nothing (the borders of the last
        // column and row still resize them).
        if end_x + GAP < w {
            p.zone((end_x + GAP, 0.0, w, h), Spot::Outside);
        }
        if end_y + GAP < h {
            p.zone((0.0, end_y + GAP, w, h), Spot::Outside);
        }
        let (col, row) = buttons(layout);
        for (b, spot, tip) in [
            (
                col,
                Spot::AddCol,
                t(
                    "Add a column (Option-click removes one)",
                    "Ajouter une colonne (Option-clic en retire une)",
                ),
            ),
            (
                row,
                Spot::AddRow,
                t(
                    "Add a row (Option-click removes one)",
                    "Ajouter une ligne (Option-clic en retire une)",
                ),
            ),
        ] {
            if b.0 >= w || b.1 >= h {
                continue;
            }
            p.rounded(
                b.0,
                b.1,
                b.2,
                b.3,
                3.0,
                c(theme::HEADER_BG),
                Some(c(theme::HEADER_BORDER)),
            );
            let (mx, my) = ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0);
            let arm = ((b.2 - b.0) * 0.28).round();
            let ink = c(theme::HEADER_TEXT);
            p.rect(mx - arm, my - 0.5, mx + arm, my + 0.5, ink);
            p.rect(mx - 0.5, my - arm, mx + 0.5, my + arm, ink);
            p.zone(b, spot)
                .cursor(CursorStyle::PointingHand)
                .tooltip(tip);
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
        let Some(spot) = zone.payload::<Spot>() else {
            return ZoneResponse::Ignored;
        };
        if matches!(spot, Spot::Outside) {
            return ZoneResponse::Handled;
        }
        let Some((rows, cols)) = extent(app) else {
            return ZoneResponse::Ignored;
        };
        // Option-click takes one away; the grid never hides what it must
        // show (`extent`).
        let step = |n: u32, max: u32| {
            if ev.modifiers.alt {
                n.saturating_sub(1).max(1)
            } else {
                (n + 1).min(max)
            }
        };
        let (rows, cols) = match spot {
            Spot::AddCol => (rows, step(cols, MAX_COLS)),
            _ => (step(rows, MAX_ROWS), cols),
        };
        let name = app.wb.sheets[app.sheet].name.clone();
        app.ext_mut::<State>().sheets.insert(name, (rows, cols));
        cx.notify();
        ZoneResponse::Handled
    }
}
