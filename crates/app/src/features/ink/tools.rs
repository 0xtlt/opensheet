//! The pointer on the sheet with a tool of the Draw tab in hand (a pen,
//! the eraser, the lasso), how strokes are painted, and Ink Replay.
//!
//! Strokes are placed like charts: by the cell holding their corner. A
//! point of the grid element and a point of the sheet (pixels at 100 %
//! from the corner of A1) are exchanged through the cell under them
//! ([`sheet_point`], [`grid_point`]), so a stroke stays where it was drawn
//! at any zoom.

use super::{Cmd, InkFeature, InkUi, Pen, Tool, group, pens, selected, stroke_id};
use crate::app::ExcelApp;
use crate::features::{
    Command, Cx, Feature, Intent, PaintLayer, Painter, Zone, ZoneEvent, ZoneResponse,
};
use crate::grid::{Draw, GridLayout};
use crate::i18n::t;
use crate::theme::{c, ca};
use gpui::{CursorStyle, Hsla, MouseButton, PathBuilder, Task, Window, px};
use opensheet_engine::drawing::{Drawing, Frame, Outline, ShapeTool};
use opensheet_engine::ink::{self, Ink, PenKind, Pt, Recognised};
use opensheet_engine::{Color, Sheet};
use std::collections::HashMap;
use std::time::Duration;

/// The zone that covers the cells while a tool is in hand.
struct Canvas;

/// What the mouse is doing with the tool.
pub(super) enum Drag {
    /// A stroke being drawn: the pen, and the points so far (of the sheet).
    Stroke { pen: Pen, points: Vec<(f64, f64)> },
    /// The eraser is down; `erased`: it has removed something, which is
    /// committed when the button is released.
    Erase { erased: bool },
    /// The outline of the lasso so far.
    Lasso { points: Vec<(f64, f64)> },
}

/// How close to a stroke the pointer must be to touch it, in pixels of
/// the screen.
const REACH: f32 = 5.0;

// --------------------------------------------------------------- coordinates

/// The point of the sheet shown at a position of the grid element.
pub(super) fn sheet_point(layout: &GridLayout, sheet: &Sheet, x: f32, y: f32) -> (f64, f64) {
    let z = layout.zoom;
    let (col, row) = (
        layout.col_at(x.max(layout.header_w)),
        layout.row_at(y.max(layout.header_h)),
    );
    let inside = (
        ((x - layout.col_x(col)) / z).max(0.0),
        ((y - layout.row_y(row)) / z).max(0.0),
    );
    sheet.corner_point(((row, col), inside))
}

/// Where a point of the sheet is in the grid element.
pub(super) fn grid_point(layout: &GridLayout, sheet: &Sheet, at: (f64, f64)) -> Pt {
    let ((row, col), (dx, dy)) = sheet.corner_at(at.0, at.1);
    (
        layout.col_x(col) + dx * layout.zoom,
        layout.row_y(row) + dy * layout.zoom,
    )
}

// --------------------------------------------------------------------- tools

/// Take a tool in hand (`Tool::Select` puts it down).
pub(super) fn take(app: &mut ExcelApp, tool: Tool, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    if tool != Tool::Select && !app.may(Intent::EditObject(None), cx) {
        return;
    }
    stop_replay(app);
    let ui = app.ext_mut::<InkUi>();
    ui.tool = tool;
    ui.drag = None;
    if matches!(tool, Tool::Draw(_)) {
        app.deselect_object(cx);
    }
    app.popup = None;
    window.focus(&app.grid_focus);
    cx.notify();
}

/// Put the tool down (Escape). False when none was in hand.
pub(super) fn put_down(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let replaying = stop_replay(app);
    let Some(ui) = app.ext::<InkUi>() else {
        return false;
    };
    if ui.tool == Tool::Select {
        if replaying {
            cx.notify();
        }
        return replaying;
    }
    let ui = app.ext_mut::<InkUi>();
    ui.tool = Tool::Select;
    ui.drag = None;
    cx.notify();
    true
}

/// The pen in hand.
fn pen_in_hand(app: &ExcelApp) -> Option<Pen> {
    match app.ext::<InkUi>()?.tool {
        Tool::Draw(index) => pens().get(index).copied(),
        _ => None,
    }
}

pub(super) fn mouse_down(
    app: &mut ExcelApp,
    zone: &Zone,
    ev: &ZoneEvent,
    window: &mut Window,
    cx: &mut Cx,
) -> ZoneResponse {
    if zone.payload::<Canvas>().is_none() {
        return ZoneResponse::Ignored;
    }
    let tool = app.ext::<InkUi>().map(|ui| ui.tool).unwrap_or_default();
    if ev.button != MouseButton::Left {
        // Another button puts the tool down, like Escape.
        put_down(app, cx);
        return ZoneResponse::Handled;
    }
    let Some(layout) = app.current_layout() else {
        return ZoneResponse::Handled;
    };
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return ZoneResponse::Handled;
    }
    let at = sheet_point(&layout, &app.wb.sheets[app.sheet], ev.x, ev.y);
    match tool {
        Tool::Select => ZoneResponse::Ignored,
        Tool::Draw(_) => {
            let Some(pen) = pen_in_hand(app) else {
                // Its pen was deleted.
                put_down(app, cx);
                return ZoneResponse::Handled;
            };
            if !app.may(Intent::EditObject(None), cx) {
                return ZoneResponse::Handled;
            }
            stop_replay(app);
            app.deselect_object(cx);
            app.ext_mut::<InkUi>().drag = Some(Drag::Stroke {
                pen,
                points: vec![at],
            });
            ZoneResponse::Capture
        }
        Tool::Eraser => {
            stop_replay(app);
            app.ext_mut::<InkUi>().drag = Some(Drag::Erase { erased: false });
            erase_at(app, &layout, at, cx);
            ZoneResponse::Capture
        }
        Tool::Lasso => {
            // A press on what the lasso selected moves it.
            if on_selection(app, at) {
                return ZoneResponse::Ignored;
            }
            app.ext_mut::<InkUi>().drag = Some(Drag::Lasso { points: vec![at] });
            ZoneResponse::Capture
        }
    }
}

pub(super) fn mouse_drag(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Cx) {
    if zone.payload::<Canvas>().is_none() {
        return;
    }
    let Some(layout) = app.current_layout() else {
        return;
    };
    let at = sheet_point(&layout, &app.wb.sheets[app.sheet], ev.x, ev.y);
    let erasing = match app.ext_mut::<InkUi>().drag.as_mut() {
        Some(Drag::Stroke { points, .. }) | Some(Drag::Lasso { points }) => {
            if points.last() != Some(&at) {
                points.push(at);
            }
            false
        }
        Some(Drag::Erase { .. }) => true,
        None => false,
    };
    if erasing {
        erase_at(app, &layout, at, cx);
    }
    cx.notify();
}

pub(super) fn mouse_up(app: &mut ExcelApp, zone: &Zone, _ev: &ZoneEvent, cx: &mut Cx) {
    if zone.payload::<Canvas>().is_none() {
        return;
    }
    match app.ext_mut::<InkUi>().drag.take() {
        Some(Drag::Stroke { pen, points }) => finish_stroke(app, pen, &points, cx),
        Some(Drag::Erase { erased: true }) => app.commit(Cmd::Eraser.label(), cx),
        Some(Drag::Lasso { points }) => {
            let enclosed = app.wb.sheets[app.sheet].inks_inside(&points);
            match enclosed.last().copied() {
                Some(top) => {
                    // The topmost one is the selected object, the others
                    // its companions.
                    app.select_object(Some(stroke_id(top)), cx);
                    app.ext_mut::<InkUi>().group = enclosed;
                }
                None => app.deselect_object(cx),
            }
        }
        _ => {}
    }
    cx.notify();
}

/// The stroke that was drawn becomes ink, or the shape it looks like when
/// Ink to Shape is on.
fn finish_stroke(app: &mut ExcelApp, pen: Pen, points: &[(f64, f64)], cx: &mut Cx) {
    let to_shape = app.ext::<InkUi>().is_some_and(|ui| ui.to_shape);
    if let (true, Some(shape)) = (to_shape, ink::recognise(points)) {
        if add_shape(app, pen, shape) {
            app.commit(Cmd::InkToShape.label(), cx);
            return;
        }
    }
    let clean = ink::smooth(points);
    if app
        .wb
        .draw_ink(app.sheet, pen.kind, Color(pen.color), pen.width(), &clean)
        .is_some()
    {
        app.commit(t("Ink", "Entrée manuscrite"), cx);
    }
}

/// The shape a stroke was recognised as, with the colour and the
/// thickness of the pen for its outline and no fill.
fn add_shape(app: &mut ExcelApp, pen: Pen, shape: Recognised) -> bool {
    let (tool, frame, flip_h, flip_v) = match shape {
        Recognised::Line { from, to } | Recognised::Arrow { from, to } => {
            let tool = if matches!(shape, Recognised::Arrow { .. }) {
                ShapeTool::Arrow
            } else {
                ShapeTool::Line
            };
            let frame = Frame::new(
                from.0.min(to.0),
                from.1.min(to.1),
                (to.0 - from.0).abs(),
                (to.1 - from.1).abs(),
            );
            (tool, frame, to.0 < from.0, to.1 < from.1)
        }
        Recognised::Rectangle(frame) => (ShapeTool::Rectangle, frame, false, false),
        Recognised::Ellipse(frame) => (ShapeTool::Ellipse, frame, false, false),
        Recognised::Triangle { frame, flip_v } => (ShapeTool::Triangle, frame, false, flip_v),
    };
    let mut look = tool.shape();
    look.fill = None;
    look.font.color = None;
    // The ends of an arrow are those of the gallery's.
    let ends = look
        .outline
        .unwrap_or_else(|| Outline::solid(Color(pen.color), 1.0));
    look.outline = Some(Outline {
        color: Color(pen.color),
        width: pen.mm * 72.0 / 25.4,
        ..ends
    });
    let mut drawing = Drawing::shape(look, (frame.width as f32, frame.height as f32));
    drawing.offset = (frame.x.max(0.0) as f32, frame.y.max(0.0) as f32);
    drawing.flip_h = flip_h;
    drawing.flip_v = flip_v;
    app.wb.add_drawing(app.sheet, drawing).is_some()
}

/// Remove the strokes the eraser touches at a point of the sheet.
fn erase_at(app: &mut ExcelApp, layout: &GridLayout, at: (f64, f64), cx: &mut Cx) {
    let touched = app.wb.sheets[app.sheet].inks_at(at.0, at.1, (REACH / layout.zoom) as f64);
    if touched.is_empty() {
        return;
    }
    if selected(app).is_some_and(|ink| touched.contains(&ink.id)) {
        app.deselect_object(cx);
    }
    for id in touched {
        app.wb.remove_ink(app.sheet, id);
    }
    if let Some(Drag::Erase { erased }) = app.ext_mut::<InkUi>().drag.as_mut() {
        *erased = true;
    }
}

/// True when a point of the sheet is in the frame of the selected stroke
/// or of one of its companions.
fn on_selection(app: &ExcelApp, at: (f64, f64)) -> bool {
    let Some(first) = selected(app) else {
        return false;
    };
    let sheet = &app.wb.sheets[app.sheet];
    let mut ids = group(app);
    ids.push(first.id);
    sheet
        .inks
        .iter()
        .filter(|ink| ids.contains(&ink.id))
        .any(|ink| {
            let frame = sheet.ink_frame(ink);
            let room = 4.0;
            at.0 >= frame.x - room
                && at.0 <= frame.x + frame.width + room
                && at.1 >= frame.y - room
                && at.1 <= frame.y + frame.height + room
        })
}

/// Whether a click at a point of the grid element is on a stroke: on its
/// line, or anywhere in its box once it is selected (to drag it).
pub(super) fn hits(app: &ExcelApp, id: u32, layout: &GridLayout, x: f32, y: f32) -> bool {
    let sheet = &app.wb.sheets[app.sheet];
    let Some(ink) = app.wb.ink(app.sheet, id) else {
        return false;
    };
    if x < layout.header_w || y < layout.header_h {
        return false;
    }
    let at = sheet_point(layout, sheet, x, y);
    let chosen = selected(app).is_some_and(|first| first.id == id) || group(app).contains(&id);
    if chosen {
        let frame = sheet.ink_frame(ink);
        return at.0 >= frame.x
            && at.0 <= frame.x + frame.width
            && at.1 >= frame.y
            && at.1 <= frame.y + frame.height;
    }
    sheet
        .inks_at(at.0, at.1, (REACH / layout.zoom) as f64)
        .contains(&id)
}

// ------------------------------------------------------------------ painting

/// The colour a stroke is painted with.
fn paint_color(kind: PenKind, color: u32) -> Hsla {
    ca(color & 0xFF_FFFF, kind.opacity())
}

/// A line through points of the grid element.
fn line(layout: &GridLayout, points: &[Pt], width: f32, color: Hsla) -> Option<Draw> {
    if points.len() < 2 {
        return None;
    }
    let mut builder = PathBuilder::stroke(px(width.max(0.5)));
    builder.move_to(layout.to_window(points[0].0, points[0].1));
    for (x, y) in &points[1..] {
        builder.line_to(layout.to_window(*x, *y));
    }
    builder.build().ok().map(|path| Draw::Path(path, color))
}

/// The points of a stroke in the box `rect` of the grid element, rounded
/// into the curve that is drawn, and the thickness of its line there.
fn stroke_in(ink: &Ink, rect: (f32, f32, f32, f32), zoom: f32) -> (Vec<Pt>, f32) {
    let size = ((rect.2 - rect.0) / zoom, (rect.3 - rect.1) / zoom);
    let points: Vec<Pt> = ink
        .points_in(size)
        .into_iter()
        .map(|(x, y)| (rect.0 + x * zoom, rect.1 + y * zoom))
        .collect();
    (ink::curve(&points, 4), ink.width_in(size) * zoom)
}

/// Paint a stroke: its line, with round ends when it is opaque.
fn paint_line(p: &mut Painter, ink: &Ink, points: &[Pt], width: f32, alpha: f32) {
    let color = ca(ink.color.value() & 0xFF_FFFF, alpha);
    match points {
        [] => {}
        [only] => p.disc(only.0, only.1, width / 2.0, color),
        _ => {
            p.polyline(points, width, color);
            // Round ends would show twice through a translucent line.
            if alpha >= 1.0 && width > 1.5 {
                for end in [points[0], points[points.len() - 1]] {
                    p.disc(end.0, end.1, width / 2.0, color);
                }
            }
        }
    }
}

/// The strokes of pens and pencils are objects over the cells; a
/// highlighter is painted with the cells instead (`paint_highlights`), and
/// only its selection shows here.
pub(super) fn paint_stroke(p: &mut Painter, id: u32, rect: (f32, f32, f32, f32)) {
    let app = p.app;
    let Some(ink) = app.wb.ink(app.sheet, id) else {
        return;
    };
    if ink.kind != PenKind::Highlighter {
        let (points, width) = stroke_in(ink, rect, p.zoom());
        let points = replayed(app, id, points);
        paint_line(p, ink, &points, width, ink.kind.opacity());
    }
    // The companions of the selected stroke show a frame of their own.
    if group(app).contains(&id) {
        let room = 3.0;
        p.frame(
            rect.0 - room,
            rect.1 - room,
            rect.2 + room,
            rect.3 + room,
            1.0,
            c(0x8A8A8A),
        );
    }
}

/// Highlighters: under the text of the cells, and once more, lighter,
/// over them, so that a cell with a fill of its own still shows the mark.
pub(super) fn paint_highlights(p: &mut Painter) {
    let app = p.app;
    let sheet = &app.wb.sheets[app.sheet];
    if !sheet
        .inks
        .iter()
        .any(|ink| ink.kind == PenKind::Highlighter)
    {
        return;
    }
    let alpha = match p.layer {
        PaintLayer::Under => PenKind::Highlighter.opacity(),
        _ => 0.2,
    };
    let objects = InkFeature.objects(app, app.sheet);
    for (ink, object) in sheet.inks.iter().zip(&objects) {
        if ink.kind != PenKind::Highlighter {
            continue;
        }
        let rect = crate::objects::object_rect(p.layout, &object.place);
        let half = ink.width * p.zoom();
        if rect.2 + half < p.clip.0
            || rect.0 - half > p.clip.2
            || rect.3 + half < p.clip.1
            || rect.1 - half > p.clip.3
        {
            continue;
        }
        let (points, width) = stroke_in(ink, rect, p.zoom());
        let points = replayed(app, ink.id, points);
        paint_line(p, ink, &points, width, alpha);
    }
}

/// While a tool is in hand the cells belong to it: its cursor, and what
/// it is drawing.
pub(super) fn paint_tool(p: &mut Painter) {
    let Some(ui) = p.app.ext::<InkUi>() else {
        return;
    };
    if ui.tool == Tool::Select {
        return;
    }
    let layout = p.layout;
    let sheet = &p.app.wb.sheets[p.app.sheet];
    let cells = (
        layout.header_w,
        layout.header_h,
        layout.width(),
        layout.height(),
    );
    let on_grid = |points: &[(f64, f64)]| -> Vec<Pt> {
        points
            .iter()
            .map(|at| grid_point(layout, sheet, *at))
            .collect()
    };
    let mut items = Vec::new();
    match &ui.drag {
        Some(Drag::Stroke { pen, points }) => {
            let width = pen.width() * layout.zoom;
            let color = paint_color(pen.kind, pen.color);
            let shown = on_grid(points);
            match shown.as_slice() {
                [only] => {
                    p.disc(only.0, only.1, width / 2.0, color);
                }
                _ => items.extend(line(layout, &shown, width, color)),
            }
        }
        Some(Drag::Lasso { points }) => {
            let mut shown = on_grid(points);
            // The lasso closes on its first point.
            if let Some(first) = shown.first().copied() {
                shown.push(first);
            }
            items.extend(line(layout, &shown, 1.0, c(0x5A5A5A)));
        }
        _ => {}
    }
    if !items.is_empty() {
        let clip = layout.bounds_px(cells.0, cells.1, cells.2, cells.3);
        p.push(Draw::Clipped { clip, items });
    }
    let cursor = match ui.tool {
        Tool::Eraser => CursorStyle::PointingHand,
        _ => CursorStyle::Crosshair,
    };
    p.zone(cells, Canvas).cursor(cursor);
}

// -------------------------------------------------------------------- replay

/// Ink Replay: the strokes of the sheet are drawn again one after the
/// other, as they were made.
pub(super) struct Replay {
    /// How much line is drawn so far, in pixels at 100 %.
    drawn: f32,
    /// Where each stroke begins along the whole and how long it is.
    spans: HashMap<u32, (f32, f32)>,
    _task: Task<()>,
}

fn length(points: &[Pt]) -> f32 {
    points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// How long a replay takes for `total` pixels of line.
fn replay_time(total: f32) -> f32 {
    (total / 900.0).clamp(1.2, 8.0)
}

const REPLAY_STEP: Duration = Duration::from_millis(16);

pub(super) fn replay(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    stop_replay(app);
    let sheet = &app.wb.sheets[app.sheet];
    let mut spans = HashMap::new();
    let mut total = 0.0;
    for ink in &sheet.inks {
        // A dot takes a moment too.
        let own = length(&ink.points_in(ink.size)).max(4.0);
        spans.insert(ink.id, (total, own));
        total += own;
    }
    if spans.is_empty() {
        return;
    }
    let speed = total / replay_time(total);
    let task = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(REPLAY_STEP).await;
            let running = this.update(cx, |app, cx| {
                let ui = app.ext_mut::<InkUi>();
                let Some(replay) = ui.replay.as_mut() else {
                    return false;
                };
                replay.drawn += speed * REPLAY_STEP.as_secs_f32();
                let done = replay.drawn >= total;
                if done {
                    ui.replay = None;
                }
                cx.notify();
                !done
            });
            if !matches!(running, Ok(true)) {
                break;
            }
        }
    });
    app.deselect_object(cx);
    app.ext_mut::<InkUi>().replay = Some(Replay {
        drawn: 0.0,
        spans,
        _task: task,
    });
    cx.notify();
}

/// Stop a replay; true when one was running.
fn stop_replay(app: &mut ExcelApp) -> bool {
    match app.ext::<InkUi>() {
        Some(ui) if ui.replay.is_some() => {
            app.ext_mut::<InkUi>().replay = None;
            true
        }
        _ => false,
    }
}

/// True while Ink Replay runs.
#[cfg(test)]
pub(super) fn replaying(app: &ExcelApp) -> bool {
    app.ext::<InkUi>().is_some_and(|ui| ui.replay.is_some())
}

/// The part of a stroke a replay has reached (all of it when none runs).
fn replayed(app: &ExcelApp, id: u32, points: Vec<Pt>) -> Vec<Pt> {
    let Some(replay) = app.ext::<InkUi>().and_then(|ui| ui.replay.as_ref()) else {
        return points;
    };
    let Some((start, own)) = replay.spans.get(&id).copied() else {
        // Drawn since the replay began.
        return points;
    };
    cut(points, ((replay.drawn - start) / own).clamp(0.0, 1.0))
}

/// The first `part` (0 to 1) of a line, by length.
fn cut(mut points: Vec<Pt>, part: f32) -> Vec<Pt> {
    if part >= 1.0 {
        return points;
    }
    if part <= 0.0 {
        return Vec::new();
    }
    let mut left = length(&points) * part;
    for i in 1..points.len() {
        let (a, b) = (points[i - 1], points[i]);
        let step = (b.0 - a.0).hypot(b.1 - a.1);
        if step >= left {
            let t = if step > 0.0 { left / step } else { 0.0 };
            points.truncate(i);
            points.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            return points;
        }
        left -= step;
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replay_cuts_lines_by_length() {
        let line = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 30.0)];
        assert_eq!(cut(line.clone(), 1.0), line);
        assert!(cut(line.clone(), 0.0).is_empty());
        assert_eq!(cut(line.clone(), 0.125), vec![(0.0, 0.0), (5.0, 0.0)]);
        assert_eq!(
            cut(line.clone(), 0.5),
            vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]
        );
        // A dot is there as soon as its turn comes.
        assert_eq!(cut(vec![(3.0, 3.0)], 0.5), vec![(3.0, 3.0)]);
        // Short work is not rushed, long work does not drag on.
        assert_eq!(
            (replay_time(10.0), replay_time(1800.0), replay_time(1.0e6)),
            (1.2, 2.0, 8.0)
        );
    }
}
