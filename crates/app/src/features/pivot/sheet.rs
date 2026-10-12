//! What a PivotTable shows and answers on the grid: the drop-down buttons
//! of its headers and filter cells, the expand / collapse buttons of its
//! labels, the frame of an empty report, double clicks and right clicks.

use super::logic::{self, Target};
use super::{actions, dialogs, filter, menus};
use crate::app::ExcelApp;
use crate::features::{Cx, PaintLayer, Painter, Zone, ZoneEvent, ZoneResponse};
use crate::i18n::t;
use crate::theme::{self, c};
use gpui::{MouseButton, Window};
use opensheet_engine::Rect;
use opensheet_engine::pivot::{PivotButton, PivotPart, PivotTable};

/// The cells of a PivotTable: where double clicks and right clicks mean
/// something else than on plain cells.
struct Body;

/// A button drawn in a cell.
struct ButtonZone {
    pivot: u32,
    row: u32,
    col: u32,
    button: PivotButton,
}

pub(super) fn paint(p: &mut Painter) {
    if p.layer != PaintLayer::Over {
        return;
    }
    let app = p.app;
    let Some(sheet) = app.wb.sheets.get(app.sheet) else {
        return;
    };
    for pivot in &sheet.pivots {
        let parts: Vec<Rect> = pivot.footprint().filter_map(|rect| rect.intersection(&p.visible)).collect();
        if parts.is_empty() {
            continue;
        }
        for part in &parts {
            let bounds = p.cell_box(*part);
            p.zone(bounds, Body);
        }
        if is_placeholder(pivot) {
            paint_placeholder(p, pivot);
        }
        paint_buttons(p, pivot);
    }
}

fn is_placeholder(pivot: &PivotTable) -> bool {
    pivot
        .location
        .and_then(|body| pivot.cell(body.r0, body.c0))
        .is_some_and(|cell| cell.part == PivotPart::Placeholder)
}

/// The buttons of the visible cells. Only header rows, label columns and
/// filter cells carry one, so the values are not looked at.
fn paint_buttons(p: &mut Painter, pivot: &PivotTable) {
    let visible = p.visible;
    let mut cells: Vec<(u32, u32)> = Vec::new();
    if let Some(pages) = pivot.page_location.and_then(|r| r.intersection(&visible)) {
        cells.extend((pages.r0..=pages.r1).flat_map(|row| (pages.c0..=pages.c1).map(move |col| (row, col))));
    }
    if let (Some(body), Some(seen)) = (pivot.location, pivot.location.and_then(|r| r.intersection(&visible))) {
        let header_end = body.r0 + pivot.report.first_data_row();
        let label_end = body.c0 + pivot.report.first_data_col();
        for row in seen.r0..=seen.r1 {
            let last = if row < header_end { seen.c1 + 1 } else { label_end.min(seen.c1 + 1) };
            cells.extend((seen.c0..last).map(|col| (row, col)));
        }
    }
    let zoom = p.zoom();
    for (row, col) in cells {
        let Some(cell) = pivot.cell(row, col) else {
            continue;
        };
        let Some(button) = cell.button else {
            continue;
        };
        let cell_box = p.cell_box(Rect::cell(row, col));
        let bounds = match button {
            PivotButton::Filter { filtered } => {
                let Some(bounds) = logic::filter_button_box(cell_box, zoom) else {
                    continue;
                };
                paint_filter_button(p, bounds, filtered);
                bounds
            }
            PivotButton::Expand | PivotButton::Collapse => {
                let Some(bounds) = logic::drill_button_box(cell_box, cell.indent, zoom) else {
                    continue;
                };
                paint_drill_button(p, bounds, button == PivotButton::Expand);
                bounds
            }
        };
        p.zone(
            bounds,
            ButtonZone {
                pivot: pivot.id,
                row,
                col,
                button,
            },
        );
    }
}

/// The drop-down button of a header: an arrow, or a funnel when one of the
/// fields it serves is filtered.
fn paint_filter_button(p: &mut Painter, (x0, y0, x1, y1): (f32, f32, f32, f32), filtered: bool) {
    p.rounded(x0, y0, x1, y1, 2.0, c(0xFDFDFD), Some(c(0xABABAB)));
    let size = x1 - x0;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let u = size / 15.0;
    let ink = c(0x444444);
    if filtered {
        // A funnel, with a small arrow beside it.
        let fx = cx - 1.5 * u;
        p.polygon(
            &[
                (fx - 4.0 * u, cy - 4.0 * u),
                (fx + 4.0 * u, cy - 4.0 * u),
                (fx + 1.0 * u, cy - 0.5 * u),
                (fx + 1.0 * u, cy + 4.0 * u),
                (fx - 1.0 * u, cy + 2.5 * u),
                (fx - 1.0 * u, cy - 0.5 * u),
            ],
            ink,
        );
        let ax = cx + 4.2 * u;
        p.polygon(
            &[
                (ax - 2.0 * u, cy + 1.5 * u),
                (ax + 2.0 * u, cy + 1.5 * u),
                (ax, cy + 4.0 * u),
            ],
            ink,
        );
    } else {
        p.polygon(
            &[
                (cx - 3.5 * u, cy - 1.5 * u),
                (cx + 3.5 * u, cy - 1.5 * u),
                (cx, cy + 2.5 * u),
            ],
            ink,
        );
    }
}

/// The "+" or "-" box of a label that has detail below it.
fn paint_drill_button(p: &mut Painter, (x0, y0, x1, y1): (f32, f32, f32, f32), plus: bool) {
    p.rect(x0, y0, x1, y1, c(0xFFFFFF));
    p.frame(x0, y0, x1, y1, 1.0, c(0x7F7F7F));
    let (cx, cy) = (((x0 + x1) / 2.0).floor(), ((y0 + y1) / 2.0).floor());
    let arm = ((x1 - x0) / 2.0 - 2.0).floor().max(1.0);
    let ink = c(0x404040);
    p.rect(cx - arm, cy, cx + arm + 1.0, cy + 1.0, ink);
    if plus {
        p.rect(cx, cy - arm, cx + 1.0, cy + arm + 1.0, ink);
    }
}

/// Break a text into lines no wider than `width`.
fn wrap(p: &Painter, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && f32::from(p.shape(&candidate, size, c(0), false).width) > width {
            lines.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The frame Excel draws where a PivotTable without fields will go: its
/// name, a hint, and a sketch of a field list.
fn paint_placeholder(p: &mut Painter, pivot: &PivotTable) {
    let Some(body) = pivot.location else {
        return;
    };
    let zoom = p.zoom();
    let (x0, y0, x1, y1) = p.cell_box(body);
    let (width, height) = (x1 - x0, y1 - y0);
    if width < 40.0 || height < 60.0 {
        return;
    }
    let line = c(0x9E9E9E);
    p.frame(x0 - 1.0, y0 - 1.0, x1, y1, 1.0, line);

    // Everything is centred in the frame, below its first row so that the
    // outline of the active cell stays visible after the insertion.
    let centre = (x0 + x1) / 2.0;
    let size = (12.0 * zoom).max(7.0);
    let line_h = (size * 1.4).round();
    let mut y = y0 + height * 0.09 + line_h;

    let name = p.shape(&pivot.name, size, c(theme::TEXT), true);
    let name_w = f32::from(name.width);
    let pad = (8.0 * zoom).round();
    let box_w = (name_w + 2.0 * pad).min(width - 8.0);
    p.rounded(
        centre - box_w / 2.0,
        y - pad / 2.0,
        centre + box_w / 2.0,
        y + line_h + pad / 2.0,
        2.0,
        c(0xFFFFFF),
        Some(line),
    );
    p.put(name, centre - name_w / 2.0, y + (line_h - size * 1.25) / 2.0, size);
    y += line_h + pad * 2.0;

    let hint = t(
        "To build a report, choose fields from the PivotTable Field List",
        "Pour générer un rapport, choisissez des champs dans la liste des champs de tableau croisé dynamique",
    );
    let hint_size = (11.0 * zoom).max(6.5);
    let hint_h = (hint_size * 1.35).round();
    let lines = wrap(p, hint, hint_size, width - 2.0 * pad);
    let hint_w = lines
        .iter()
        .map(|text| f32::from(p.shape(text, hint_size, c(0), false).width))
        .fold(0.0, f32::max);
    p.rect(
        centre - hint_w / 2.0 - 3.0,
        y - 2.0,
        centre + hint_w / 2.0 + 3.0,
        y + hint_h * lines.len() as f32 + 2.0,
        c(0xFFFFFF),
    );
    for text in &lines {
        let shaped = p.shape(text, hint_size, c(theme::TEXT_MUTED), false);
        let w = f32::from(shaped.width);
        p.put(shaped, centre - w / 2.0, y, hint_size);
        y += hint_h;
    }
    y += pad * 1.5;

    // The sketch: a field list with ticked boxes over the four areas.
    let sketch_w = (86.0 * zoom).min(width - 2.0 * pad);
    let sketch_h = (104.0 * zoom).min(y1 - y - pad);
    if sketch_h < 50.0 * zoom {
        return;
    }
    let (sx0, sx1) = (centre - sketch_w / 2.0, centre + sketch_w / 2.0);
    p.rounded(sx0, y, sx1, y + sketch_h, 2.0, c(0xFFFFFF), Some(line));
    let u = sketch_w / 86.0;
    let accent = c(theme::ACCENT);
    for k in 0..4 {
        let ly = y + (10.0 + k as f32 * 11.0) * u;
        p.frame(sx0 + 8.0 * u, ly, sx0 + 14.0 * u, ly + 6.0 * u, 1.0, if k % 2 == 0 { accent } else { line });
        if k % 2 == 0 {
            p.rect(sx0 + 9.5 * u, ly + 1.5 * u, sx0 + 12.5 * u, ly + 4.5 * u, accent);
        }
        p.rect(sx0 + 19.0 * u, ly + 2.0 * u, sx1 - (10.0 + k as f32 * 7.0) * u, ly + 4.0 * u, c(0xC8C8C8));
    }
    let top = y + 58.0 * u;
    let (mid, gap) = (centre, 3.0 * u);
    for (ax0, ax1, ay) in [
        (sx0 + 8.0 * u, mid - gap, top),
        (mid + gap, sx1 - 8.0 * u, top),
        (sx0 + 8.0 * u, mid - gap, top + 20.0 * u),
        (mid + gap, sx1 - 8.0 * u, top + 20.0 * u),
    ] {
        p.frame(ax0, ay, ax1, ay + 16.0 * u, 1.0, line);
    }
    p.rect(mid + gap + 3.0 * u, top + 24.0 * u, sx1 - 12.0 * u, top + 29.0 * u, theme::ca(theme::ACCENT, 0.45));
    p.rect(sx0 + 11.0 * u, top + 24.0 * u, mid - gap - 5.0 * u, top + 29.0 * u, c(0xC8C8C8));
}

pub(super) fn mouse_down(
    app: &mut ExcelApp,
    zone: &Zone,
    ev: &ZoneEvent,
    window: &mut Window,
    cx: &mut Cx,
) -> ZoneResponse {
    let button = zone.payload::<ButtonZone>();
    if button.is_none() && zone.payload::<Body>().is_none() {
        return ZoneResponse::Ignored;
    }
    let (row, col) = match button {
        Some(button) => (button.row, button.col),
        None => ev.cell,
    };
    match ev.button {
        MouseButton::Right => {
            if app.is_editing() && !app.commit_edit(None, window, cx) {
                return ZoneResponse::Handled;
            }
            window.focus(&app.grid_focus);
            if !app.sel().contains(row, col) {
                app.select_cell(row, col, cx);
            }
            let entries = menus::context_entries(app, row, col);
            app.open_context_menu(ev.position, entries, cx);
            ZoneResponse::Handled
        }
        // While a formula is typed, a click points at the cell.
        MouseButton::Left if app.is_editing() => ZoneResponse::Ignored,
        MouseButton::Left => match button {
            Some(button) => {
                press_button(app, button, ev, window, cx);
                ZoneResponse::Handled
            }
            None if ev.click_count >= 2 => double_click(app, row, col, window, cx),
            None => ZoneResponse::Ignored,
        },
        _ => ZoneResponse::Ignored,
    }
}

fn press_button(app: &mut ExcelApp, zone: &ButtonZone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
    app.select_cell(zone.row, zone.col, cx);
    match zone.button {
        PivotButton::Filter { .. } => {
            filter::toggle_drop_down(app, zone.pivot, zone.row, zone.col, Some(ev.position), window, cx)
        }
        PivotButton::Expand | PivotButton::Collapse => {
            let cell = app.wb.pivot_cell(app.sheet, zone.row, zone.col);
            if let Some((field, item)) = cell.and_then(|cell| cell.field.zip(cell.item)) {
                let collapse = zone.button == PivotButton::Collapse;
                actions::collapse_item(app, zone.pivot, field, item, collapse, window, cx);
            }
        }
    }
}

/// A double click: the details behind a value, the detail below a label
/// (or the choice of a field to put below it), the settings of a field
/// name. Elsewhere the click goes on to the cell, which answers that it
/// cannot be edited.
fn double_click(app: &mut ExcelApp, row: u32, col: u32, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
    let Some(cell) = app.wb.pivot_cell(app.sheet, row, col) else {
        return ZoneResponse::Ignored;
    };
    let id = cell.pivot;
    match (cell.part, cell.button) {
        (PivotPart::Value, _) => actions::show_details(app, row, col, window, cx),
        (_, Some(PivotButton::Expand | PivotButton::Collapse)) => {
            if let Some((field, item)) = cell.field.zip(cell.item) {
                let collapse = cell.button == Some(PivotButton::Collapse);
                actions::collapse_item(app, id, field, item, collapse, window, cx);
            }
        }
        // A label with nothing below it: Excel asks which field to show
        // under it.
        (PivotPart::RowLabel | PivotPart::ColumnLabel, None) => match cell.field.zip(cell.item) {
            Some((field, item)) => dialogs::open_show_detail(app, id, field, item, window, cx),
            None => return ZoneResponse::Ignored,
        },
        (PivotPart::DataCaption | PivotPart::FieldHeader, _) => match logic::target_of(&cell) {
            Some(Target::Data(data)) => dialogs::open_value_settings(app, id, data, 0, window, cx),
            Some(Target::Field(field)) => dialogs::open_field_settings(app, id, field, window, cx),
            None => return ZoneResponse::Ignored,
        },
        _ => return ZoneResponse::Ignored,
    }
    ZoneResponse::Handled
}
