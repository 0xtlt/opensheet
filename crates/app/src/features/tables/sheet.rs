//! Tables on the grid: the corner handle that resizes a table, the list of
//! a totals cell, the button that takes back what a table did by itself,
//! the column names shown in place of the column letters — and what
//! follows an edit, Tab, a name typed in the Name Box and cells pointed at
//! while a formula is typed.

use super::actions::{self, AutoChange, AutoKind, TabMove};
use super::{Doing, TablesUi, active_table, refuse, resize};
use crate::app::{ExcelApp, MenuEntry, Popup, PopupKind};
use crate::features::{self, CellEdit, ChangeCause, Cx, Intent, PaintLayer, Painter, Zone, ZoneEvent, ZoneResponse};
use crate::grid::{Draw, GridLayout};
use crate::i18n::t;
use crate::theme::{self, c};
use gpui::{CursorStyle, MouseButton, Window, px};
use opensheet_engine::Rect;
use opensheet_engine::ops::Direction;
use opensheet_engine::table::{Table, TableArea, TotalsFunction};
use opensheet_engine::workbook::ClearMode;

/// What a zone of this module stands for.
#[derive(Clone, Copy)]
enum Spot {
    /// The corner handle of a table (its id).
    Corner(u32),
    /// The button of the totals cell (`row`, `col`) of a column.
    Totals {
        table: u32,
        column: usize,
        row: u32,
        col: u32,
    },
    /// The button that takes back what a table did by itself.
    Auto,
}

/// A corner handle being dragged.
pub(super) struct ResizeDrag {
    sheet: usize,
    table: u32,
    /// The range the table would take if the button were released now.
    to: Rect,
}

/// Tab moves through a table in its own way, and `ExcelApp::advance`
/// serves Return and the arrow keys as well: the key is announced
/// (`tab_pressed`) before the move it asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TabKey {
    #[default]
    None,
    /// Tab was just pressed.
    Pressed,
    /// The entry being typed is stored first; the move follows.
    Committing,
}

/// Colour of the handle at the corner of a table.
const HANDLE: u32 = 0x1F4E79;

// ----------------------------------------------------------------- painting

pub(super) fn paint(p: &mut Painter) {
    let app = p.app;
    let Some(sheet) = app.wb.sheets.get(app.sheet) else {
        return;
    };
    let ui = app.ext::<TablesUi>();
    match p.layer {
        PaintLayer::Under => {}
        PaintLayer::Over => {
            // The range typed in the Create Table or Resize Table dialog.
            let shown = ui.and_then(|ui| ui.shown.upgrade()).and_then(|shown| shown.get());
            if let Some((_, range)) = shown.filter(|(on, _)| *on == app.sheet) {
                marquee(p, range);
            }
            if sheet.tables.is_empty() {
                return;
            }
            for table in &sheet.tables {
                corner(p, table.id, table.range);
            }
            if let Some(drag) = ui
                .and_then(|ui| ui.resize.as_ref())
                .filter(|drag| drag.sheet == app.sheet)
            {
                if drag.to.intersects(&p.visible) {
                    let (x0, y0, x1, y1) = p.cell_box(drag.to);
                    p.frame(x0 - 1.0, y0 - 1.0, x1, y1, 2.0, c(HANDLE));
                }
            }
            totals_button(p);
            auto_button(p);
        }
        PaintLayer::Top => {
            if sheet.tables.is_empty() {
                return;
            }
            header_names(p);
            for table in &sheet.tables {
                corner_over_fill_handle(p, table.id, table.range);
            }
        }
    }
}

/// The dashed outline of a range a dialog is asking about, drawn like the
/// one around copied cells.
fn marquee(p: &mut Painter, range: Rect) {
    if !range.intersects(&p.visible) {
        return;
    }
    let (x0, y0, x1, y1) = p.cell_box(range);
    let (x0, y0) = (x0 - 1.0, y0 - 1.0);
    let color = c(theme::SELECTION);
    p.dashed((x0, y0), (x1, y0), 2.0, 4.0, 3.0, color);
    p.dashed((x0, y1 - 1.0), (x1, y1 - 1.0), 2.0, 4.0, 3.0, color);
    p.dashed((x0, y0), (x0, y1), 2.0, 4.0, 3.0, color);
    p.dashed((x1 - 1.0, y0), (x1 - 1.0, y1), 2.0, 4.0, 3.0, color);
}

/// Whether the fill handle of the selection sits on the corner of `range`
/// (the conditions under which the grid draws it).
fn under_fill_handle(app: &ExcelApp, range: Rect) -> bool {
    let sel = app.sel();
    let current = sel.current();
    sel.ranges.len() == 1
        && !app.is_editing()
        && app.object_sel.is_none()
        && (current.r1, current.c1) == (range.r1, range.c1)
}

/// Length of the arms of the corner handle, and their thickness.
fn handle_size(zoom: f32) -> (f32, f32) {
    (
        (8.0 * zoom).round().clamp(6.0, 14.0),
        (2.0 * zoom).round().clamp(1.0, 4.0),
    )
}

/// The handle at the bottom-right corner of a table: an angle inside its
/// last cell, as Excel draws it. Dragging it resizes the table.
fn corner(p: &mut Painter, id: u32, range: Rect) {
    if !p.visible.contains(range.r1, range.c1) {
        return;
    }
    let (_, _, x1, y1) = p.cell_box(Rect::cell(range.r1, range.c1));
    let (arm, thick) = handle_size(p.zoom());
    // Inside the gridlines of the cell.
    let (right, bottom) = (x1 - 1.0, y1 - 1.0);
    // Where the fill handle sits on the same corner, it keeps its square
    // and takes the presses there (see `corner_over_fill_handle`).
    let covered = under_fill_handle(p.app, range);
    let inset = if covered { 4.0 } else { 0.0 };
    p.rect(right - arm, bottom - thick, right - inset, bottom, c(HANDLE));
    p.rect(right - thick, bottom - arm, right, bottom - inset, c(HANDLE));
    if !covered {
        p.zone((right - arm - 1.0, bottom - arm - 1.0, x1, y1), Spot::Corner(id))
            .cursor(CursorStyle::ResizeUpLeftDownRight);
    }
}

/// The part of the grid the cell (`row`, `col`) is drawn in: the frozen or
/// the scrolled pane of its row and of its column (x0, y0, x1, y1).
fn pane_of(layout: &GridLayout, row: u32, col: u32) -> (f32, f32, f32, f32) {
    let fx = layout.header_w + layout.frozen_w;
    let fy = layout.header_h + layout.frozen_h;
    let (x0, x1) = if col < layout.frozen_cols {
        (layout.header_w, fx)
    } else {
        (fx, layout.width())
    };
    let (y0, y1) = if row < layout.frozen_rows {
        (layout.header_h, fy)
    } else {
        (fy, layout.height())
    };
    (x0, y0, x1, y1)
}

/// The fill handle of a selection that ends on the corner of a table is
/// found before the zones of the cells: the inner part of the table's
/// handle is registered above it, so that both can be grabbed, the fill
/// handle from outside the corner and the table's from inside.
fn corner_over_fill_handle(p: &mut Painter, id: u32, range: Rect) {
    if !under_fill_handle(p.app, range) {
        return;
    }
    let (_, _, x1, y1) = p.cell_box(Rect::cell(range.r1, range.c1));
    let (arm, _) = handle_size(p.zoom());
    // This layer is not clipped to a pane: keep to the one of the cell.
    let pane = pane_of(p.layout, range.r1, range.c1);
    let bounds = (
        (x1 - arm - 2.0).max(pane.0),
        (y1 - arm - 2.0).max(pane.1),
        (x1 - 3.0).min(pane.2),
        (y1 - 3.0).min(pane.3),
    );
    if bounds.0 < bounds.2 && bounds.1 < bounds.3 {
        p.zone(bounds, Spot::Corner(id))
            .cursor(CursorStyle::ResizeUpLeftDownRight);
    }
}

/// The drop-down button Excel shows beside the totals cell that is
/// active, just outside its right edge.
fn totals_button(p: &mut Painter) {
    let app = p.app;
    let (row, col) = app.active();
    if app.is_editing() || !p.visible.contains(row, col) {
        return;
    }
    let Some(cell) = app
        .wb
        .table_cell(app.sheet, row, col)
        .filter(|cell| cell.area == TableArea::Totals)
    else {
        return;
    };
    let spot = Spot::Totals {
        table: cell.table.id,
        column: cell.column,
        row,
        col,
    };
    let (_, y0, x1, y1) = p.cell_box(Rect::cell(row, col));
    let zoom = p.zoom();
    let side = (17.0 * zoom).round().clamp(13.0, 26.0);
    let (bx0, by0, bx1, by1) = (x1, (y1 - 1.0 - side).max(y0), x1 + side, y1 - 1.0);
    p.rounded(bx0, by0, bx1, by1, 2.0, c(0xF3F3F3), Some(c(0x8A8A8A)));
    let (cx, cy) = ((bx0 + bx1) / 2.0, (by0 + by1) / 2.0);
    let half = (3.5 * zoom).clamp(2.5, 6.0);
    p.polygon(
        &[
            (cx - half, cy - half / 2.0),
            (cx + half, cy - half / 2.0),
            (cx, cy + half / 2.0 + 1.0),
        ],
        c(0x444444),
    );
    p.zone((bx0, by0, bx1, by1), spot);
}

/// The button that takes back what a table just did by itself, below the
/// cells that were entered (Excel's AutoCorrect Options button).
fn auto_button(p: &mut Painter) {
    let app = p.app;
    let Some(change) = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.auto.as_ref())
        .filter(|change| change.sheet == app.sheet && change.stands(&app.wb))
    else {
        return;
    };
    let (row, col) = (change.rect.r1, change.rect.c1);
    if !p.visible.contains(row, col) {
        return;
    }
    let (_, _, x1, y1) = p.cell_box(Rect::cell(row, col));
    let side = (18.0 * p.zoom()).round().clamp(14.0, 28.0);
    let (bx0, by0) = (x1 + 2.0, y1 + 2.0);
    p.rounded(
        bx0,
        by0,
        bx0 + side,
        by0 + side,
        3.0,
        c(0xFFFFFF),
        Some(c(theme::BORDER_STRONG)),
    );
    // A lightning bolt, on an 18 × 18 design grid.
    let unit = side / 18.0;
    let at = |x: f32, y: f32| (bx0 + x * unit, by0 + y * unit);
    p.polygon(
        &[
            at(10.5, 2.5),
            at(5.0, 10.0),
            at(8.5, 10.0),
            at(7.0, 15.5),
            at(13.0, 7.5),
            at(9.5, 7.5),
        ],
        c(0xE8A200),
    );
    p.zone((bx0, by0, bx0 + side, by0 + side), Spot::Auto)
        .cursor(CursorStyle::PointingHand)
        .tooltip(t("AutoCorrect Options", "Options de correction automatique"));
}

/// Whether the header row of `table` has scrolled out of view above the
/// grid while rows of the table are still shown. A frozen header row
/// never does.
fn header_scrolled_away(layout: &GridLayout, table: &Table) -> bool {
    let first_shown = layout.scrolled_rows().start;
    let head = table.range.r0;
    table.header_row && head >= layout.frozen_rows && head < first_shown && table.range.r1 >= first_shown
}

/// Once the header row of the table holding the active cell has scrolled
/// out of view, the column headers show the names of its columns instead
/// of the column letters, as in Excel. They stay the headers of the
/// sheet's columns: a click still selects the column.
fn header_names(p: &mut Painter) {
    let app = p.app;
    let layout = p.layout;
    let height = layout.header_h;
    if height <= 0.0 {
        return;
    }
    let Some(table) = active_table(app).filter(|table| header_scrolled_away(layout, table)) else {
        return;
    };
    // Drawn as the grid draws its own column headers.
    let size = (12.0 * layout.zoom).clamp(8.0, 20.0);
    let line_h = (size * 1.3).round();
    let pad = (3.0 * layout.zoom).round().max(1.0);
    let frozen_end = layout.header_w + layout.frozen_w;
    let sel = app.sel();
    for (index, column) in table.columns.iter().enumerate() {
        let col = table.range.c0 + index as u32;
        let (x, width) = (layout.col_x(col), layout.col_w(col));
        if width <= 0.0 {
            continue;
        }
        // Frozen columns and scrolled ones have a header strip each.
        let (lo, hi) = if col < layout.frozen_cols {
            (layout.header_w, frozen_end)
        } else {
            (frozen_end, layout.width())
        };
        let (x0, x1) = (x.max(lo), (x + width).min(hi));
        if x1 <= x0 {
            continue;
        }
        let state = sel
            .ranges
            .iter()
            .filter(|r| col >= r.c0 && col <= r.c1)
            .map(|r| if r.is_whole_cols() { 2 } else { 1 })
            .max()
            .unwrap_or(0);
        let (background, ink) = match state {
            2 => (theme::HEADER_FULL_BG, 0xFFFFFF),
            1 => (theme::HEADER_SEL_BG, theme::HEADER_SEL_TEXT),
            _ => (theme::HEADER_BG, theme::HEADER_TEXT),
        };
        p.rect(x0, 0.0, x1, height, c(background));
        if x + width <= hi {
            p.rect(x + width - 1.0, 0.0, x + width, height, c(theme::HEADER_BORDER));
        }
        let line = p.shape(&column.name, size, c(ink), state > 0);
        let text_w = f32::from(line.width);
        // Centred like the letters; a name too long starts at the left.
        let text_x = if text_w + 2.0 * pad <= width {
            x + ((width - text_w) / 2.0).round()
        } else {
            x + pad
        };
        p.push(Draw::Text {
            line,
            origin: layout.to_window(text_x, ((height - line_h) / 2.0).round()),
            line_height: px(line_h),
            clip: layout.bounds_px(x0, 0.0, (x1 - 1.0).max(x0), height),
        });
        if state == 1 {
            p.rect(x0, height - 2.0, x1, height, c(theme::SELECTION));
        }
        p.rect(x0, height - 1.0, x1, height, c(theme::HEADER_BORDER));
    }
}

// -------------------------------------------------------------------- zones

pub(super) fn zone_mouse_down(
    app: &mut ExcelApp,
    zone: &Zone,
    ev: &ZoneEvent,
    window: &mut Window,
    cx: &mut Cx,
) -> ZoneResponse {
    let Some(spot) = zone.payload::<Spot>().copied() else {
        return ZoneResponse::Ignored;
    };
    // Any other button is for the cell underneath (its context menu).
    if ev.button != MouseButton::Left {
        return ZoneResponse::Ignored;
    }
    match spot {
        Spot::Corner(id) => {
            // A chart lying over the corner takes the press.
            let covered = app
                .current_layout()
                .is_some_and(|layout| app.hit_object(&layout, ev.x, ev.y).is_some());
            let Some(range) = app
                .wb
                .table(app.sheet, id)
                .map(|table| table.range)
                .filter(|_| !covered)
            else {
                return ZoneResponse::Ignored;
            };
            if (app.is_editing() && !app.commit_edit(None, window, cx)) || !app.may(Intent::ChangeTable, cx) {
                return ZoneResponse::Handled;
            }
            app.ext_mut::<TablesUi>().resize = Some(ResizeDrag {
                sheet: app.sheet,
                table: id,
                to: range,
            });
            ZoneResponse::Capture
        }
        Spot::Totals {
            table,
            column,
            row,
            col,
        } => {
            open_totals_list(app, table, column, row, col, cx);
            ZoneResponse::Handled
        }
        Spot::Auto => {
            open_auto_menu(app, zone, cx);
            ZoneResponse::Handled
        }
    }
}

pub(super) fn zone_drag(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Cx) {
    if !matches!(zone.payload::<Spot>(), Some(Spot::Corner(_))) {
        return;
    }
    let Some((sheet, id)) = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.resize.as_ref())
        .map(|drag| (drag.sheet, drag.table))
    else {
        return;
    };
    scroll_at_edge(app, ev);
    let Some(to) = app.wb.table(sheet, id).map(|table| actions::drag_range(table, ev.cell)) else {
        return;
    };
    if let Some(drag) = &mut app.ext_mut::<TablesUi>().resize {
        drag.to = to;
    }
    cx.notify();
}

pub(super) fn zone_mouse_up(app: &mut ExcelApp, zone: &Zone, cx: &mut Cx) {
    if !matches!(zone.payload::<Spot>(), Some(Spot::Corner(_))) {
        return;
    }
    if let Some(drag) = app.ext_mut::<TablesUi>().resize.take() {
        resize(app, drag.sheet, drag.table, drag.to, cx);
    }
    cx.notify();
}

/// Dragging the handle against an edge of the grid scrolls the sheet, as
/// dragging a selection does.
fn scroll_at_edge(app: &mut ExcelApp, ev: &ZoneEvent) {
    let Some(layout) = app.current_layout() else {
        return;
    };
    let step = (20.0 * app.zoom) as f64;
    let view = &mut app.views[app.sheet];
    if ev.x > layout.width() - 8.0 {
        view.scroll_x += step * 3.0;
    } else if ev.x < layout.header_w + layout.frozen_w + 4.0 && layout.frozen_cols == 0 {
        view.scroll_x = (view.scroll_x - step * 3.0).max(0.0);
    }
    if ev.y > layout.height() - 8.0 {
        view.scroll_y += step;
    } else if ev.y < layout.header_h + layout.frozen_h + 4.0 && layout.frozen_rows == 0 {
        view.scroll_y = (view.scroll_y - step).max(0.0);
    }
}

/// The list of a totals cell: what the row computes under the column.
fn open_totals_list(app: &mut ExcelApp, table: u32, column: usize, row: u32, col: u32, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(current) = app
        .wb
        .table(sheet, table)
        .and_then(|table| table.columns.get(column))
        .map(|column| column.totals.clone())
    else {
        return;
    };
    let mut entries: Vec<MenuEntry> = actions::totals_choices()
        .into_iter()
        .map(|(label, function)| {
            let checked = function == current;
            MenuEntry::new(label, move |app, _, cx| {
                set_totals(app, sheet, table, column, function.clone(), cx)
            })
            .checked(checked)
        })
        .collect();
    // Any other function is typed in the cell: the list of functions starts
    // the formula.
    entries.push(MenuEntry::new(
        t("More Functions…", "Autres fonctions…"),
        |app, window, cx| app.open_function_menu("fb-fx-anchor", None, window, cx),
    ));
    app.open_context_menu(app.cell_anchor(Rect::cell(row, col)), entries, cx);
}

fn set_totals(app: &mut ExcelApp, sheet: usize, table: u32, column: usize, function: TotalsFunction, cx: &mut Cx) {
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    if app.wb.set_totals_function(sheet, table, column, function) {
        app.commit(
            t("Change Total Row Formula", "Modifier la formule de la ligne de total"),
            cx,
        );
    }
}

/// The menu of the button shown after a table grew or filled a column by
/// itself.
fn open_auto_menu(app: &mut ExcelApp, zone: &Zone, cx: &mut Cx) {
    let Some(kind) = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.auto.as_ref())
        .map(|change| change.kind)
    else {
        return;
    };
    let Some(layout) = app.current_layout() else {
        return;
    };
    let label = match kind {
        AutoKind::Expansion => t(
            "Undo Table AutoExpansion",
            "Annuler le développement automatique du tableau",
        ),
        AutoKind::CalculatedColumn => t("Undo Calculated Column", "Annuler la colonne calculée"),
    };
    let entries = vec![MenuEntry::new(label, |app, _, cx| undo_auto(app, cx)).icon("undo")];
    let position = layout.to_window(zone.bounds.0, zone.bounds.3 + 2.0);
    // Wider than a context menu: the French label is long.
    app.popup = Some(Popup::new(PopupKind::Menu(entries), position, 350.0));
    cx.notify();
}

/// Take back what the table did, leaving the cells as they were entered.
fn undo_auto(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(change) = app.ext_mut::<TablesUi>().auto.take() else {
        return;
    };
    if change.undo(&mut app.wb) {
        app.clip = None;
        app.refresh(cx);
    }
}

// ------------------------------------------------------------ following edits

/// Cells were just typed, pasted, filled or cleared, and the change is not
/// committed yet: the tables of the sheet follow it in the same undo step
/// (they grow over what was entered beside them, take the names typed in
/// their header cells, fill calculated columns).
pub(super) fn cells_written(app: &mut ExcelApp, edit: &CellEdit) {
    let sheet = edit.sheet;
    if app.wb.sheets.get(sheet).is_none_or(|sh| sh.tables.is_empty()) {
        return;
    }
    // Formats alone change nothing a table follows.
    if matches!(edit.cause, ChangeCause::Clear(ClearMode::Formats)) {
        return;
    }
    // A protected sheet keeps its tables as they are (the engine does not
    // ask by itself).
    if features::guard(app, &Intent::ChangeTable).is_some() {
        return;
    }
    // What the user entered is kept aside, to offer taking back what the
    // table does next. Not when the step holds more than this change:
    // cells moved from elsewhere, or several ranges.
    let moved = matches!(edit.cause, ChangeCause::Paste { cut: true, .. });
    let several = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.auto.as_ref())
        .is_some_and(|change| change.generation.is_none());
    let before = if moved || several || !actions::near_tables(&app.wb, sheet, edit.rect) {
        None
    } else {
        AutoChange::before(&app.wb, sheet, edit.rect)
    };
    let did = app.wb.tables_after_edit(sheet, edit.rect);
    let kind = if did.expanded.is_some() {
        Some(AutoKind::Expansion)
    } else if did.filled.is_some() {
        Some(AutoKind::CalculatedColumn)
    } else {
        None
    };
    app.ext_mut::<TablesUi>().auto = before.zip(kind).map(|(mut change, kind)| {
        change.kind = kind;
        change
    });
}

/// The change is committed: from now on, what a table did in it can be
/// taken back until the workbook changes again.
pub(super) fn cells_changed(app: &mut ExcelApp) {
    let generation = app.wb.generation;
    if let Some(change) = app.ext_mut::<TablesUi>().auto.as_mut() {
        if change.generation.is_none() {
            change.generation = Some(generation);
        }
    }
}

// ---------------------------------------------------------------------- Tab

/// Tab (or Shift+Tab) was pressed: the move that follows goes through a
/// table row after row. Called by the application before it stores the
/// entry being typed and moves the active cell.
pub fn tab_pressed(app: &mut ExcelApp) {
    // With a chart or a picture selected, Tab goes from object to object:
    // no cell moves, and the press must not be mistaken for a later one.
    if app.editing.is_none() && app.object_sel.is_some() {
        return;
    }
    app.ext_mut::<TablesUi>().tab = TabKey::Pressed;
}

/// An entry is about to be stored. Only the move of the Tab that asked for
/// it is a Tab move: a press left over from an entry that was refused is
/// forgotten.
pub(super) fn entry_committing(app: &mut ExcelApp) {
    let Some(key) = app
        .ext::<TablesUi>()
        .map(|ui| ui.tab)
        .filter(|key| *key != TabKey::None)
    else {
        return;
    };
    app.ext_mut::<TablesUi>().tab = match key {
        TabKey::Pressed => TabKey::Committing,
        _ => TabKey::None,
    };
}

/// The active cell is about to move one cell in `dir` after an entry or a
/// key. Returns true when Tab was pressed in a table and the move was
/// made here instead: to the next table row from the last column (and
/// back with Shift+Tab), with a new row from the last cell of the data.
pub fn advance(app: &mut ExcelApp, dir: Direction, cx: &mut Cx) -> bool {
    if app.ext::<TablesUi>().is_none_or(|ui| ui.tab == TabKey::None) {
        return false;
    }
    app.ext_mut::<TablesUi>().tab = TabKey::None;
    let backward = match dir {
        Direction::Right => false,
        Direction::Left => true,
        _ => return false,
    };
    // Inside a selection of several cells, Tab walks the selection.
    let sel = app.sel();
    if sel.ranges.len() != 1 || !sel.current().is_single() {
        return false;
    }
    let (sheet, active) = (app.sheet, sel.active);
    match actions::tab_move(&app.wb, sheet, active, backward) {
        None => false,
        Some(TabMove::To(row, col)) => {
            app.select_cell(row, col, cx);
            true
        }
        Some(TabMove::AddRow { table, at, col }) => {
            // A protected sheet keeps its tables as they are: Tab moves on.
            if features::guard(app, &Intent::ChangeTable).is_some() {
                return false;
            }
            match app.wb.table_insert_rows(sheet, table, at, 1) {
                Ok(()) => {
                    app.commit(t("Add Table Row", "Ajouter une ligne de tableau"), cx);
                    app.select_cell(at, col, cx);
                }
                Err(error) => refuse(app, error, Doing::Other, cx),
            }
            true
        }
    }
}

// ------------------------------------------------ the Name Box and pointing

/// A table name or a structured reference typed in the Name Box (`Table1`,
/// `Table1[#All]`, `Table1[Sales]`): select the cells it stands for.
/// Returns false when `text` is neither.
pub fn go_to(app: &mut ExcelApp, text: &str, cx: &mut Cx) -> bool {
    let Some(target) = actions::go_to_target(&app.wb, app.sheet, app.active(), text) else {
        return false;
    };
    app.sheet = target.sheet;
    app.select_rect(target.rect, cx);
    true
}

/// What pointing at the cells `rect` of the current sheet writes in the
/// formula being typed, when that is a structured reference (`[@Sales]`,
/// `Table1[Sales]`) and not `rect` itself.
pub fn pointed_reference(app: &ExcelApp, rect: Rect) -> Option<String> {
    let edit = app.editing.as_ref()?;
    let from = (edit.sheet == app.sheet).then_some((edit.row, edit.col));
    actions::pointed_reference(&app.wb, app.sheet, rect, from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, point, size};
    use opensheet_engine::Sheet;

    /// The layout of a sheet in an 800 × 600 grid, scrolled down by
    /// `rows` rows.
    fn layout(sheet: &Sheet, rows: u32) -> GridLayout {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(600.)));
        let unscrolled = GridLayout::new(sheet, bounds, 1.0, 0.0, 0.0);
        GridLayout::new(sheet, bounds, 1.0, 0.0, unscrolled.rows.start(rows))
    }

    fn table(range: Rect) -> Table {
        Table {
            range,
            header_row: true,
            ..Default::default()
        }
    }

    #[test]
    fn column_names_replace_letters_once_the_headers_are_gone() {
        let mut sheet = Sheet::new("Sheet1");
        let long = table(Rect::new(2, 0, 80, 3));
        // The header row is in view, even as the first row shown.
        assert!(!header_scrolled_away(&layout(&sheet, 0), &long));
        assert!(!header_scrolled_away(&layout(&sheet, 2), &long));
        // Scrolled past it, with rows of the table still on screen.
        assert!(header_scrolled_away(&layout(&sheet, 3), &long));
        assert!(header_scrolled_away(&layout(&sheet, 80), &long));
        // The whole table is above the view.
        assert!(!header_scrolled_away(&layout(&sheet, 81), &long));
        // A table without a header row has no names to show.
        let bare = Table {
            header_row: false,
            ..long.clone()
        };
        assert!(!header_scrolled_away(&layout(&sheet, 10), &bare));
        // Frozen with the top rows, the header row stays in view.
        sheet.frozen_rows = 3;
        assert!(!header_scrolled_away(&layout(&sheet, 10), &long));
    }

    #[test]
    fn panes() {
        let mut sheet = Sheet::new("Sheet1");
        sheet.frozen_rows = 2;
        sheet.frozen_cols = 1;
        let layout = layout(&sheet, 0);
        let (hw, hh) = (layout.header_w, layout.header_h);
        let (fx, fy) = (hw + layout.frozen_w, hh + layout.frozen_h);
        assert_eq!(pane_of(&layout, 0, 0), (hw, hh, fx, fy));
        assert_eq!(pane_of(&layout, 5, 0), (hw, fy, fx, 600.0));
        assert_eq!(pane_of(&layout, 0, 5), (fx, hh, 800.0, fy));
        assert_eq!(pane_of(&layout, 5, 5), (fx, fy, 800.0, 600.0));
    }

    #[test]
    fn handle_scales_with_the_zoom() {
        assert_eq!(handle_size(1.0), (8.0, 2.0));
        // Never too small to grab, never huge.
        assert_eq!(handle_size(0.25), (6.0, 1.0));
        assert_eq!(handle_size(4.0), (14.0, 4.0));
    }
}
