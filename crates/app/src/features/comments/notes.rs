//! The boxes of notes: where they sit on the screen, how they are drawn,
//! and what dragging their border or one of their handles makes of them.

use super::Spot;
use crate::features::Painter;
use crate::grid::{Draw, FontResolver, GridLayout, pt_to_px};
use crate::objects::handle_points;
use crate::theme::{self, c};
use gpui::{
    CursorStyle, Font, FontStyle, FontWeight, PathBuilder, SharedString, StrikethroughStyle, TextAlign, TextRun,
    UnderlineStyle, px,
};
use opensheet_engine::comment::Note;
use opensheet_engine::drawing::Frame;
use opensheet_engine::style::Underline;
use opensheet_engine::{Rect, Sheet};

/// A pixel box (x0, y0, x1, y1) in grid-element coordinates.
pub(super) type Px = (f32, f32, f32, f32);

/// Fill of a note box: the "tooltip yellow" notes have always had.
pub(super) const PAPER: u32 = 0xFFFFE1;
/// Room between the border of a box and its text, in pixels at 100 %.
pub(super) const INSET: (f32, f32) = (4.0, 2.0);
/// Width of the band around a box that is selected or being typed in.
const BAND: f32 = 5.0;
/// How far from its border a box still takes a press meant to move it.
const REACH: f32 = 3.0;
/// Smallest box the handles can make, in pixels at 100 %.
const MIN_SIZE: (f64, f64) = (24.0, 16.0);

/// What a press on the edge of a note box takes hold of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grip {
    /// The border: the whole box moves.
    Move,
    /// A handle: 0 = top-left, then clockwise (as `objects::handle_points`).
    Handle(u8),
}

impl Grip {
    fn cursor(self) -> CursorStyle {
        match self {
            Grip::Move => CursorStyle::OpenHand,
            Grip::Handle(0 | 4) => CursorStyle::ResizeUpLeftDownRight,
            Grip::Handle(2 | 6) => CursorStyle::ResizeUpRightDownLeft,
            Grip::Handle(1 | 5) => CursorStyle::ResizeUpDown,
            Grip::Handle(_) => CursorStyle::ResizeLeftRight,
        }
    }
}

/// A box being moved or resized with the mouse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BoxDrag {
    pub cell: (u32, u32),
    pub grip: Grip,
    /// Where the press landed, in grid-element coordinates.
    pub start: (f32, f32),
    /// The frame at that moment, and as dragged so far.
    pub origin: Frame,
    pub current: Frame,
}

/// The frame of a box whose grip was dragged by (`dx`, `dy`) pixels at
/// 100 %. A box never leaves the sheet by its top or its left, and a
/// handle stops where the box would get smaller than a line of text.
pub(super) fn dragged(origin: Frame, grip: Grip, dx: f64, dy: f64) -> Frame {
    let mut frame = origin;
    let Grip::Handle(handle) = grip else {
        frame.x = (origin.x + dx).max(0.0);
        frame.y = (origin.y + dy).max(0.0);
        return frame;
    };
    if matches!(handle, 0 | 6 | 7) {
        let d = dx.min(origin.width - MIN_SIZE.0).max(-origin.x);
        frame.x += d;
        frame.width -= d;
    }
    if matches!(handle, 2 | 3 | 4) {
        frame.width = (origin.width + dx).max(MIN_SIZE.0);
    }
    if matches!(handle, 0 | 1 | 2) {
        let d = dy.min(origin.height - MIN_SIZE.1).max(-origin.y);
        frame.y += d;
        frame.height -= d;
    }
    if matches!(handle, 4 | 5 | 6) {
        frame.height = (origin.height + dy).max(MIN_SIZE.1);
    }
    frame
}

/// Where to show a box the pointer (or Next Note) made appear, so that it
/// is whole inside the cells in view: where it is when it fits, else on
/// the other side of its cell, else pushed back inside.
pub(super) fn kept_in_view(rect: Px, cell: Px, area: Px) -> Px {
    let (w, h) = (rect.2 - rect.0, rect.3 - rect.1);
    let mut x0 = rect.0;
    if x0 + w > area.2 {
        // As far left of the cell as it was right of it.
        let left = cell.0 - (rect.0 - cell.2) - w;
        x0 = if left >= area.0 { left } else { area.2 - w };
    }
    let x0 = x0.max(area.0);
    let y0 = rect.1.min(area.3 - h).max(area.1);
    (x0, y0, x0 + w, y0 + h)
}

/// The line tying a box to its cell: from the corner of the box nearest
/// to the top-right corner of the cell, to that corner. None when the box
/// covers it.
pub(super) fn connector(rect: Px, cell: Px) -> Option<((f32, f32), (f32, f32))> {
    let tip = (cell.2, cell.1);
    if tip.0 >= rect.0 && tip.0 <= rect.2 && tip.1 >= rect.1 && tip.1 <= rect.3 {
        return None;
    }
    let x = if (tip.0 - rect.0).abs() <= (tip.0 - rect.2).abs() { rect.0 } else { rect.2 };
    let y = if (tip.1 - rect.1).abs() <= (tip.1 - rect.3).abs() { rect.1 } else { rect.3 };
    Some(((x, y), tip))
}

/// The part of a segment inside a box (Liang–Barsky).
pub(super) fn clip_segment(a: (f32, f32), b: (f32, f32), clip: Px) -> Option<((f32, f32), (f32, f32))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-dx, a.0 - clip.0),
        (dx, clip.2 - a.0),
        (-dy, a.1 - clip.1),
        (dy, clip.3 - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let t = q / p;
        if p < 0.0 {
            t0 = t0.max(t);
        } else {
            t1 = t1.min(t);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some(((a.0 + t0 * dx, a.1 + t0 * dy), (a.0 + t1 * dx, a.1 + t1 * dy)))
}

pub(super) fn intersect(a: Px, b: Px) -> Option<Px> {
    let r = (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3));
    (r.2 > r.0 && r.3 > r.1).then_some(r)
}

fn inside(point: (f32, f32), clip: Px) -> bool {
    point.0 >= clip.0 && point.0 <= clip.2 && point.1 >= clip.1 && point.1 <= clip.3
}

// ------------------------------------------------------------- what shows

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoxState {
    Plain,
    /// Its border was clicked: it shows its handles.
    Selected,
    /// Its text is being typed.
    Editing,
}

/// A note box on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShownBox {
    pub cell: (u32, u32),
    pub rect: Px,
    /// What it may draw on: the cells of its pane.
    pub clip: Px,
    pub state: BoxState,
    /// It reacts to the mouse (a box the pointer only made appear does not).
    pub solid: bool,
    /// How far it was pushed from its place to stay in view, in pixels.
    pub pushed: (f32, f32),
}

/// The cells area of the grid: all of it, or what scrolls right of and
/// below the frozen panes for a point that scrolls.
fn pane_area(layout: &GridLayout, anchor: Option<(u32, u32)>) -> Px {
    let scrolls = |index: u32, frozen: u32| anchor.is_some() && index >= frozen;
    let (row, col) = anchor.unwrap_or((0, 0));
    (
        layout.header_w + if scrolls(col, layout.frozen_cols) { layout.frozen_w } else { 0.0 },
        layout.header_h + if scrolls(row, layout.frozen_rows) { layout.frozen_h } else { 0.0 },
        layout.width(),
        layout.height(),
    )
}

/// The pixel box of the cell a note belongs to (its merged area).
pub(super) fn cell_px(layout: &GridLayout, sheet: &Sheet, cell: (u32, u32)) -> Px {
    layout.rect_px(sheet.merge_at(cell.0, cell.1).unwrap_or(Rect::cell(cell.0, cell.1)))
}

fn shown(layout: &GridLayout, sheet: &Sheet, cell: (u32, u32), frame: Frame, stays: bool, state: BoxState) -> ShownBox {
    let z = layout.zoom;
    let place = sheet.note_box(frame);
    let x0 = (layout.col_x(place.anchor.1) + place.offset.0 * z).round();
    let y0 = (layout.row_y(place.anchor.0) + place.offset.1 * z).round();
    let rect = (x0, y0, x0 + (place.size.0 * z).round(), y0 + (place.size.1 * z).round());
    if stays {
        // A box left showing scrolls with the sheet, under the frozen panes.
        return ShownBox {
            cell,
            rect,
            clip: pane_area(layout, Some(place.anchor)),
            state,
            solid: true,
            pushed: (0.0, 0.0),
        };
    }
    let area = pane_area(layout, None);
    let room = (
        area.0 + 2.0,
        area.1 + 2.0,
        area.2 - theme::SCROLLBAR - BAND,
        area.3 - theme::SCROLLBAR - BAND,
    );
    let kept = kept_in_view(rect, cell_px(layout, sheet, cell), room);
    ShownBox {
        cell,
        rect: kept,
        clip: area,
        state,
        solid: state != BoxState::Plain,
        pushed: (kept.0 - rect.0, kept.1 - rect.1),
    }
}

/// What decides which note boxes show, besides the notes themselves.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct BoxView {
    /// The note whose text is being typed (it may not exist yet).
    pub editing: Option<(u32, u32)>,
    /// The note whose box is selected.
    pub selected: Option<(u32, u32)>,
    /// Hidden notes that show for now: under the pointer, or where Next
    /// Note went.
    pub peeked: Vec<(u32, u32)>,
    pub drag: Option<BoxDrag>,
}

/// Every note box to draw on a sheet: the notes left showing, the one
/// under the pointer, the one selected or being typed.
pub(super) fn shown_boxes(sheet: &Sheet, layout: &GridLayout, view: &BoxView) -> Vec<ShownBox> {
    let state_of = |cell: (u32, u32)| {
        if view.editing == Some(cell) {
            BoxState::Editing
        } else if view.selected == Some(cell) {
            BoxState::Selected
        } else {
            BoxState::Plain
        }
    };
    let mut out = Vec::new();
    for (cell, comment) in &sheet.comments {
        let Some(note) = comment.as_note() else {
            continue;
        };
        let state = state_of(*cell);
        if !note.visible && state == BoxState::Plain && !view.peeked.contains(cell) {
            continue;
        }
        let frame = match view.drag.filter(|drag| drag.cell == *cell) {
            Some(drag) => Some(drag.current),
            None => sheet.note_frame(cell.0, cell.1),
        };
        if let Some(frame) = frame {
            out.push(shown(layout, sheet, *cell, frame, note.visible, state));
        }
    }
    // A note being written that the workbook does not hold yet.
    if let Some(cell) = view.editing.filter(|cell| !sheet.comments.contains_key(cell)) {
        let frame = sheet.default_note_frame(cell.0, cell.1);
        out.push(shown(layout, sheet, cell, frame, false, BoxState::Editing));
    }
    out
}

// ------------------------------------------------------------------ drawing

/// The text of a note as it is shaped: its runs in their fonts.
pub(super) struct NoteText {
    pub text: SharedString,
    pub runs: Vec<TextRun>,
    /// Font size and line height in pixels at 100 %.
    pub size: f32,
    pub line_height: f32,
}

/// Line height of text of a given size in a note (Tahoma's own leading).
pub(super) fn line_height(size: f32) -> f32 {
    (size * 1.21).round()
}

pub(super) fn note_font(font: &opensheet_engine::style::Font, fonts: &FontResolver) -> Font {
    Font {
        family: fonts.resolve(&font.name),
        features: Default::default(),
        fallbacks: None,
        weight: if font.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if font.italic { FontStyle::Italic } else { FontStyle::Normal },
    }
}

pub(super) fn note_text(note: &Note, fonts: &FontResolver) -> NoteText {
    let mut text = String::new();
    let mut runs = Vec::new();
    for run in &note.runs {
        let color = run.font.color.map(theme::engine_color).unwrap_or(c(0x000000));
        // Lines end with a line feed, whatever the file used.
        let part = run.text.replace("\r\n", "\n").replace('\r', "\n");
        text.push_str(&part);
        runs.push(TextRun {
            len: part.len(),
            font: note_font(&run.font, fonts),
            color,
            background_color: None,
            underline: (run.font.underline != Underline::None).then_some(UnderlineStyle {
                color: Some(color),
                thickness: px(1.),
                wavy: false,
            }),
            strikethrough: run.font.strike.then_some(StrikethroughStyle {
                color: Some(color),
                thickness: px(1.),
            }),
        });
    }
    // One size for the whole box: that of the body of the note.
    let size = pt_to_px(note.runs.last().map_or(Note::font().size_pt(), |run| run.font.size_pt()));
    NoteText {
        text: text.into(),
        runs,
        size,
        line_height: line_height(size),
    }
}

fn rect(p: &mut Painter, clip: Px, area: Px, color: u32) {
    if let Some((x0, y0, x1, y1)) = intersect(area, clip) {
        p.rect(x0, y0, x1, y1, c(color));
    }
}

/// The band around a selected box: slanted lines while its text is typed,
/// crossed ones while the box itself is selected.
fn band(p: &mut Painter, clip: Px, outer: Px, crossed: bool) {
    let (x0, y0, x1, y1) = outer;
    for strip in [
        (x0, y0, x1, y0 + BAND),
        (x0, y1 - BAND, x1, y1),
        (x0, y0 + BAND, x0 + BAND, y1 - BAND),
        (x1 - BAND, y0 + BAND, x1, y1 - BAND),
    ] {
        rect(p, clip, strip, 0xFFFFFF);
    }
    let layout = p.layout;
    let mut builder = PathBuilder::stroke(px(1.));
    let mut any = false;
    let mut line = |a: (f32, f32), b: (f32, f32)| {
        if let Some((a, b)) = clip_segment(a, b, clip) {
            builder.move_to(layout.to_window(a.0, a.1));
            builder.line_to(layout.to_window(b.0, b.1));
            any = true;
        }
    };
    let step = 4.0;
    let mut x = x0;
    while x < x1 {
        let end = (x + BAND).min(x1);
        for y in [y0, y1 - BAND] {
            line((x, y + BAND), (end, y + BAND - (end - x)));
            if crossed {
                line((x, y), (end, y + (end - x)));
            }
        }
        x += step;
    }
    let mut y = y0 + BAND;
    while y < y1 - BAND {
        let end = (y + BAND).min(y1 - BAND);
        for x in [x0, x1 - BAND] {
            line((x, end), (x + (end - y), y));
            if crossed {
                line((x, y), (x + (end - y), end));
            }
        }
        y += step;
    }
    if any {
        if let Ok(path) = builder.build() {
            p.push(Draw::Path(path, c(0x404040)));
        }
    }
}

/// Draw one note box, the line to its cell, and (unless its text is being
/// typed: the editor shows it) its text.
pub(super) fn paint_box(p: &mut Painter, shown: &ShownBox, cell: Px, text: Option<NoteText>) {
    let z = p.zoom();
    let clip = shown.clip;
    let (x0, y0, x1, y1) = shown.rect;

    // The line to the cell, ending in an arrow head on the indicator.
    if let Some((from, tip)) = connector(shown.rect, cell).filter(|(_, tip)| inside(*tip, clip)) {
        if let Some((a, b)) = clip_segment(from, tip, clip) {
            p.polyline(&[a, b], 1.0, c(0x000000));
        }
        let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
        let length = (dx * dx + dy * dy).sqrt();
        if length > 6.0 {
            let (ux, uy) = (dx / length, dy / length);
            let size = (5.0 * z).clamp(4.0, 8.0);
            let back = (tip.0 - ux * size, tip.1 - uy * size);
            let half = size * 0.35;
            p.polygon(
                &[tip, (back.0 - uy * half, back.1 + ux * half), (back.0 + uy * half, back.1 - ux * half)],
                c(0x000000),
            );
        }
    }

    // The shadow, the paper and the border.
    let shadow = (2.0 * z).round().max(1.0);
    rect(p, clip, (x1, y0 + shadow, x1 + shadow, y1 + shadow), 0x000000);
    rect(p, clip, (x0 + shadow, y1, x1, y1 + shadow), 0x000000);
    if shown.state != BoxState::Plain {
        band(
            p,
            clip,
            (x0 - BAND, y0 - BAND, x1 + BAND, y1 + BAND),
            shown.state == BoxState::Selected,
        );
    }
    rect(p, clip, shown.rect, PAPER);
    for edge in [
        (x0, y0, x1, y0 + 1.0),
        (x0, y1 - 1.0, x1, y1),
        (x0, y0, x0 + 1.0, y1),
        (x1 - 1.0, y0, x1, y1),
    ] {
        rect(p, clip, edge, 0x000000);
    }

    if let Some(text) = text.filter(|_| shown.state != BoxState::Editing) {
        let (pad_x, pad_y) = ((INSET.0 * z).round(), (INSET.1 * z).round());
        let inner = (x0 + pad_x, y0 + pad_y, x1 - pad_x, y1 - 1.0);
        if let Some(bounds) = intersect(inner, clip) {
            let width = (inner.2 - inner.0).max(8.0);
            if let Ok(lines) = p.ts.shape_text(text.text, px(text.size * z), &text.runs, Some(px(width)), None) {
                p.push(Draw::Wrapped {
                    lines: lines.into_iter().collect(),
                    origin: p.layout.to_window(inner.0, inner.1),
                    line_height: px((text.line_height * z).round()),
                    align: TextAlign::Left,
                    bounds: p.layout.bounds_px(bounds.0, bounds.1, bounds.2, bounds.3),
                });
            }
        }
    }

    if shown.state != BoxState::Plain {
        for (hx, hy) in handle_points(handle_frame(shown.rect)) {
            if inside((hx, hy), clip) {
                p.disc(hx, hy, 4.0, c(0x404040));
                p.disc(hx, hy, 3.0, c(0xFFFFFF));
            }
        }
    }
}

/// What makes a box react to the mouse: its inside starts typing, its
/// border moves it, its handles (when it shows them) resize it. Later
/// zones lie over earlier ones.
pub(super) fn box_zones(shown: &ShownBox) -> Vec<(Px, Spot, CursorStyle)> {
    let (x0, y0, x1, y1) = shown.rect;
    let (row, col) = shown.cell;
    let mut out = Vec::new();
    let mut zone = |area: Px, spot: Spot, cursor: CursorStyle| {
        if let Some(area) = intersect(area, shown.clip) {
            out.push((area, spot, cursor));
        }
    };
    if shown.state != BoxState::Editing {
        zone(
            (x0 + REACH, y0 + REACH, x1 - REACH, y1 - REACH),
            Spot::Body(row, col),
            CursorStyle::IBeam,
        );
    }
    let out_by = if shown.state == BoxState::Plain { REACH } else { BAND };
    for strip in [
        (x0 - out_by, y0 - out_by, x1 + out_by, y0 + REACH),
        (x0 - out_by, y1 - REACH, x1 + out_by, y1 + out_by),
        (x0 - out_by, y0 + REACH, x0 + REACH, y1 - REACH),
        (x1 - REACH, y0 + REACH, x1 + out_by, y1 - REACH),
    ] {
        zone(strip, Spot::Grip(row, col, Grip::Move), Grip::Move.cursor());
    }
    if shown.state != BoxState::Plain {
        for (index, (hx, hy)) in handle_points(handle_frame(shown.rect)).into_iter().enumerate() {
            let grip = Grip::Handle(index as u8);
            zone((hx - 5.0, hy - 5.0, hx + 5.0, hy + 5.0), Spot::Grip(row, col, grip), grip.cursor());
        }
    }
    out
}

/// Where the handles of a selected box sit: in the middle of its band.
fn handle_frame(rect: Px) -> Px {
    (rect.0 - BAND / 2.0, rect.1 - BAND / 2.0, rect.2 + BAND / 2.0, rect.3 + BAND / 2.0)
}

/// Register the zones of a box with the grid.
pub(super) fn zones(p: &mut Painter, shown: &ShownBox) {
    for (area, spot, cursor) in box_zones(shown) {
        p.zone(area, spot).cursor(cursor);
    }
}

/// The corner mark of an annotated cell: a triangle for a note, a small
/// flag for a comment, inside the gridlines at the top right of the cell.
pub(super) fn mark(thread: bool, cell: Px, zoom: f32) -> Vec<(f32, f32)> {
    let (_, y0, x1, y1) = cell;
    let right = x1 - 1.0;
    let size = ((if thread { 8.0 } else { 6.0 }) * zoom).round().clamp(4.0, 18.0).min(y1 - y0);
    if !thread {
        return vec![(right - size, y0), (right, y0), (right, y0 + size)];
    }
    let cut = (size * 0.45).round();
    vec![
        (right - size, y0),
        (right, y0),
        (right, y0 + size),
        (right - size + cut, y0 + size),
        (right - size, y0 + size - cut),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, point, size};
    use opensheet_engine::{Locale, Workbook};

    const BOX: Frame = Frame {
        x: 100.0,
        y: 50.0,
        width: 144.0,
        height: 79.0,
    };

    fn layout(wb: &Workbook, zoom: f32, scroll: (f64, f64)) -> GridLayout {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1000.), px(600.)));
        GridLayout::new(&wb.sheets[0], bounds, zoom, scroll.0, scroll.1)
    }

    /// A workbook with a note on C5 (left showing or not).
    fn noted(visible: bool) -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.set_note(0, 4, 2, "Ada", "Ada:\nCheck");
        wb.set_note_visible(0, 4, 2, visible);
        wb.commit("note");
        wb
    }

    #[test]
    fn shown_boxes_follow_zoom_and_scrolling() {
        let wb = noted(true);
        let sheet = &wb.sheets[0];
        // 15 pixels right of C5, a little above its top.
        let frame = sheet.note_frame(4, 2).unwrap();
        assert_eq!(frame, Frame::new(207.0, 70.0, 144.0, 79.0));
        let at = |zoom: f32, scroll: (f64, f64)| {
            let l = layout(&wb, zoom, scroll);
            let shown = shown(&l, sheet, (4, 2), frame, true, BoxState::Plain);
            assert!(shown.solid && shown.pushed == (0.0, 0.0));
            let (x0, y0, x1, y1) = shown.rect;
            (x0 - l.header_w, y0 - l.header_h, x1 - x0, y1 - y0)
        };
        assert_eq!(at(1.0, (0.0, 0.0)), (207.0, 70.0, 144.0, 79.0));
        // Twice as large, twice as far: the box stays 15 zoomed pixels from its cell.
        assert_eq!(at(2.0, (0.0, 0.0)), (414.0, 140.0, 288.0, 158.0));
        // It scrolls with the sheet, out of view if need be.
        assert_eq!(at(1.0, (64.0, 40.0)), (143.0, 30.0, 144.0, 79.0));
        assert_eq!(at(1.0, (640.0, 400.0)), (-433.0, -330.0, 144.0, 79.0));
    }

    #[test]
    fn shown_boxes_slide_under_frozen_panes() {
        let mut wb = noted(true);
        wb.sheets[0].frozen_rows = 2;
        wb.sheets[0].frozen_cols = 1;
        let sheet = &wb.sheets[0];
        let frame = sheet.note_frame(4, 2).unwrap();
        let l = layout(&wb, 1.0, (100.0, 30.0));
        let shown = shown(&l, sheet, (4, 2), frame, true, BoxState::Plain);
        // The box scrolls; what it may draw on starts after the frozen
        // column and the frozen rows.
        assert_eq!(shown.rect.0, l.header_w + 107.0);
        assert_eq!(shown.rect.1, l.header_h + 40.0);
        assert_eq!(shown.clip, (l.header_w + 64.0, l.header_h + 40.0, 1000.0, 600.0));
        // A box anchored in the frozen column stays put and draws over
        // the whole grid.
        let frozen = shown_at(&l, sheet, Frame::new(10.0, 5.0, 144.0, 79.0));
        assert_eq!(frozen.rect.0, l.header_w + 10.0);
        assert_eq!(frozen.clip, (l.header_w, l.header_h, 1000.0, 600.0));
    }

    fn shown_at(l: &GridLayout, sheet: &Sheet, frame: Frame) -> ShownBox {
        shown(l, sheet, (4, 2), frame, true, BoxState::Plain)
    }

    #[test]
    fn peeked_boxes_are_pushed_into_view() {
        let wb = noted(false);
        let sheet = &wb.sheets[0];
        let frame = sheet.note_frame(4, 2).unwrap();
        // In view: where the note is, and deaf to the mouse.
        let l = layout(&wb, 1.0, (0.0, 0.0));
        let peek = shown(&l, sheet, (4, 2), frame, false, BoxState::Plain);
        assert_eq!((peek.rect.0, peek.pushed, peek.solid), (l.header_w + 207.0, (0.0, 0.0), false));
        // Scrolled so that row 5 is the first in view: the box, which
        // starts above its cell, is pushed down under the headers.
        let l = layout(&wb, 1.0, (0.0, 80.0));
        let peek = shown(&l, sheet, (4, 2), frame, false, BoxState::Plain);
        assert_eq!(peek.rect.1, l.header_h + 2.0);
        assert_eq!(peek.pushed, (0.0, 12.0));
        // Once selected (its border was pressed), it takes the mouse.
        assert!(shown(&l, sheet, (4, 2), frame, false, BoxState::Selected).solid);
    }

    /// The spot a press at (x, y) lands on: the last zone that holds it,
    /// as the grid decides.
    fn spot_at(shown: &ShownBox, x: f32, y: f32) -> Option<Spot> {
        box_zones(shown)
            .into_iter()
            .rev()
            .find(|(area, _, _)| x >= area.0 && x < area.2 && y >= area.1 && y < area.3)
            .map(|(_, spot, _)| spot)
    }

    fn at(rect: Px, clip: Px, state: BoxState) -> ShownBox {
        ShownBox {
            cell: (4, 2),
            rect,
            clip,
            state,
            solid: true,
            pushed: (0.0, 0.0),
        }
    }

    #[test]
    fn a_press_on_a_box_finds_its_inside_its_border_or_a_handle() {
        let everywhere = (0.0, 0.0, 1000.0, 600.0);
        let rect = (300.0, 100.0, 444.0, 179.0);
        let plain = at(rect, everywhere, BoxState::Plain);
        // The inside starts typing; the border, a few pixels either side
        // of the line, moves the box; beside it the press is for the cells.
        assert_eq!(spot_at(&plain, 370.0, 140.0), Some(Spot::Body(4, 2)));
        assert_eq!(spot_at(&plain, 301.0, 140.0), Some(Spot::Grip(4, 2, Grip::Move)));
        assert_eq!(spot_at(&plain, 298.0, 140.0), Some(Spot::Grip(4, 2, Grip::Move)));
        assert_eq!(spot_at(&plain, 370.0, 180.0), Some(Spot::Grip(4, 2, Grip::Move)));
        assert_eq!(spot_at(&plain, 290.0, 140.0), None);
        assert_eq!(spot_at(&plain, 370.0, 190.0), None);
        // No handles until the box is selected: a corner moves it.
        assert_eq!(spot_at(&plain, 299.0, 99.0), Some(Spot::Grip(4, 2, Grip::Move)));

        // Selected: the eight handles, clockwise from the top-left corner,
        // win over the border they sit on.
        let selected = at(rect, everywhere, BoxState::Selected);
        assert_eq!(spot_at(&selected, 298.0, 98.0), Some(Spot::Grip(4, 2, Grip::Handle(0))));
        assert_eq!(spot_at(&selected, 372.0, 98.0), Some(Spot::Grip(4, 2, Grip::Handle(1))));
        assert_eq!(spot_at(&selected, 446.0, 98.0), Some(Spot::Grip(4, 2, Grip::Handle(2))));
        assert_eq!(spot_at(&selected, 446.0, 139.5), Some(Spot::Grip(4, 2, Grip::Handle(3))));
        assert_eq!(spot_at(&selected, 446.0, 181.0), Some(Spot::Grip(4, 2, Grip::Handle(4))));
        assert_eq!(spot_at(&selected, 372.0, 181.0), Some(Spot::Grip(4, 2, Grip::Handle(5))));
        assert_eq!(spot_at(&selected, 298.0, 181.0), Some(Spot::Grip(4, 2, Grip::Handle(6))));
        assert_eq!(spot_at(&selected, 298.0, 139.5), Some(Spot::Grip(4, 2, Grip::Handle(7))));
        assert_eq!(spot_at(&selected, 330.0, 98.0), Some(Spot::Grip(4, 2, Grip::Move)));
        assert_eq!(spot_at(&selected, 370.0, 140.0), Some(Spot::Body(4, 2)));

        // While its text is typed the inside belongs to the editor laid
        // over it; the band around still moves and resizes the box.
        let editing = at(rect, everywhere, BoxState::Editing);
        assert_eq!(spot_at(&editing, 370.0, 140.0), None);
        assert_eq!(spot_at(&editing, 330.0, 98.0), Some(Spot::Grip(4, 2, Grip::Move)));
        assert_eq!(spot_at(&editing, 446.0, 181.0), Some(Spot::Grip(4, 2, Grip::Handle(4))));

        // A box half under the headers or a frozen pane takes no press there.
        let clipped = at(rect, (350.0, 120.0, 1000.0, 600.0), BoxState::Selected);
        assert_eq!(spot_at(&clipped, 330.0, 140.0), None);
        assert_eq!(spot_at(&clipped, 370.0, 110.0), None);
        assert_eq!(spot_at(&clipped, 370.0, 140.0), Some(Spot::Body(4, 2)));
        assert_eq!(spot_at(&clipped, 298.0, 98.0), None);
    }

    #[test]
    fn which_boxes_show() {
        let mut wb = Workbook::new(Locale::en());
        wb.set_note(0, 1, 0, "Ada", "left showing");
        wb.set_note_visible(0, 1, 0, true);
        wb.set_note(0, 4, 2, "Ada", "hidden");
        wb.add_comment(0, 6, 1, &opensheet_engine::comment::Person::new("Bob"), 45_000.0, "a thread");
        wb.commit("setup");
        let sheet = &wb.sheets[0];
        let l = layout(&wb, 1.0, (0.0, 0.0));
        let cells = |view: &BoxView| -> Vec<((u32, u32), BoxState, bool)> {
            shown_boxes(sheet, &l, view).iter().map(|shown| (shown.cell, shown.state, shown.solid)).collect()
        };
        // Only the notes left showing; a comment has no box.
        assert_eq!(cells(&BoxView::default()), [((1, 0), BoxState::Plain, true)]);
        // Under the pointer (or where Next Note went) a hidden note shows,
        // deaf to the mouse.
        let mut view = BoxView {
            peeked: vec![(4, 2), (6, 1)],
            ..BoxView::default()
        };
        assert_eq!(
            cells(&view),
            [((1, 0), BoxState::Plain, true), ((4, 2), BoxState::Plain, false)]
        );
        // Selected or typed in, it shows whatever the pointer does.
        view.peeked.clear();
        view.selected = Some((4, 2));
        assert_eq!(cells(&view)[1], ((4, 2), BoxState::Selected, true));
        view.selected = None;
        view.editing = Some((4, 2));
        assert_eq!(cells(&view)[1], ((4, 2), BoxState::Editing, true));
        // A note being written on a cell that has none yet gets a box
        // where Excel would put it.
        view.editing = Some((8, 3));
        let boxes = shown_boxes(sheet, &l, &view);
        assert_eq!(boxes.len(), 2);
        let frame = sheet.default_note_frame(8, 3);
        assert_eq!((boxes[1].cell, boxes[1].state), ((8, 3), BoxState::Editing));
        assert_eq!(boxes[1].rect.0, l.header_w + frame.x as f32);
        assert_eq!(boxes[1].rect.2 - boxes[1].rect.0, 144.0);
        // A box follows the mouse that drags it before the note is changed.
        let origin = sheet.note_frame(1, 0).unwrap();
        let moved = dragged(origin, Grip::Move, 30.0, 12.0);
        let view = BoxView {
            selected: Some((1, 0)),
            drag: Some(BoxDrag {
                cell: (1, 0),
                grip: Grip::Move,
                start: (0.0, 0.0),
                origin,
                current: moved,
            }),
            ..BoxView::default()
        };
        let still = shown_boxes(sheet, &l, &BoxView::default())[0].rect;
        let dragging = shown_boxes(sheet, &l, &view)[0].rect;
        assert_eq!((dragging.0 - still.0, dragging.1 - still.1), (30.0, 12.0));
        assert_eq!(sheet.note_frame(1, 0), Some(origin));
    }

    #[test]
    fn marks_sit_in_the_top_right_corner() {
        let cell = (100.0, 40.0, 164.0, 60.0);
        // A note: a red triangle against the gridline, six pixels a side.
        assert_eq!(mark(false, cell, 1.0), [(157.0, 40.0), (163.0, 40.0), (163.0, 46.0)]);
        // A comment: a larger shape with its lower-left corner cut off.
        let flag = mark(true, cell, 1.0);
        assert_eq!(flag.len(), 5);
        assert_eq!((flag[0], flag[1], flag[2]), ((155.0, 40.0), (163.0, 40.0), (163.0, 48.0)));
        // They grow with the zoom, and never past the height of a low row.
        assert_eq!(mark(false, (200.0, 80.0, 328.0, 120.0), 2.0)[0], (315.0, 80.0));
        assert_eq!(mark(true, (100.0, 40.0, 164.0, 45.0), 1.0)[2], (163.0, 45.0));
    }

    #[test]
    fn moving_and_resizing() {
        assert_eq!(dragged(BOX, Grip::Move, 10.0, -5.0), Frame::new(110.0, 45.0, 144.0, 79.0));
        // A box stops at the top and the left of the sheet.
        assert_eq!(dragged(BOX, Grip::Move, -500.0, -500.0), Frame::new(0.0, 0.0, 144.0, 79.0));
        // The bottom-right handle changes the size only.
        assert_eq!(dragged(BOX, Grip::Handle(4), 20.0, 10.0), Frame::new(100.0, 50.0, 164.0, 89.0));
        // The top-left one moves the corner and keeps the opposite one.
        assert_eq!(dragged(BOX, Grip::Handle(0), 20.0, 10.0), Frame::new(120.0, 60.0, 124.0, 69.0));
        // Edge handles change one dimension.
        assert_eq!(dragged(BOX, Grip::Handle(1), 20.0, -10.0), Frame::new(100.0, 40.0, 144.0, 89.0));
        assert_eq!(dragged(BOX, Grip::Handle(3), -20.0, 30.0), Frame::new(100.0, 50.0, 124.0, 79.0));
        assert_eq!(dragged(BOX, Grip::Handle(7), -20.0, 30.0), Frame::new(80.0, 50.0, 164.0, 79.0));
        // A handle cannot cross the opposite side, nor leave the sheet.
        assert_eq!(dragged(BOX, Grip::Handle(4), -500.0, -500.0), Frame::new(100.0, 50.0, 24.0, 16.0));
        assert_eq!(dragged(BOX, Grip::Handle(0), 500.0, 500.0), Frame::new(220.0, 113.0, 24.0, 16.0));
        assert_eq!(dragged(BOX, Grip::Handle(0), -500.0, -500.0), Frame::new(0.0, 0.0, 244.0, 129.0));
    }

    #[test]
    fn boxes_under_the_pointer_stay_in_view() {
        let area = (30.0, 20.0, 800.0, 600.0);
        let cell = (200.0, 100.0, 264.0, 120.0);
        // Room on the right: where Excel puts it.
        let beside = (279.0, 90.0, 423.0, 169.0);
        assert_eq!(kept_in_view(beside, cell, area), beside);
        // A cell at the right edge: the box goes left of it, as far away.
        let cell = (700.0, 100.0, 764.0, 120.0);
        assert_eq!(
            kept_in_view((779.0, 90.0, 923.0, 169.0), cell, area),
            (541.0, 90.0, 685.0, 169.0)
        );
        // No room on either side: pushed back inside.
        let narrow = (30.0, 20.0, 300.0, 600.0);
        let cell = (120.0, 100.0, 184.0, 120.0);
        assert_eq!(
            kept_in_view((199.0, 90.0, 343.0, 169.0), cell, narrow),
            (156.0, 90.0, 300.0, 169.0)
        );
        // A cell of the first or the last row: the box stays under the
        // headers and above the bottom.
        let cell = (200.0, 20.0, 264.0, 40.0);
        assert_eq!(kept_in_view((279.0, 10.0, 423.0, 89.0), cell, area).1, 20.0);
        let cell = (200.0, 580.0, 264.0, 600.0);
        assert_eq!(kept_in_view((279.0, 570.0, 423.0, 649.0), cell, area).3, 600.0);
    }

    #[test]
    fn the_line_goes_to_the_nearest_corner() {
        let cell = (200.0, 100.0, 264.0, 120.0);
        // Right of the cell and a little above: from the top-left corner.
        assert_eq!(
            connector((279.0, 90.0, 423.0, 169.0), cell),
            Some(((279.0, 90.0), (264.0, 100.0)))
        );
        // Moved left of the cell and above it: from the bottom-right corner.
        assert_eq!(
            connector((20.0, 10.0, 164.0, 89.0), cell),
            Some(((164.0, 89.0), (264.0, 100.0)))
        );
        // Over the corner of the cell: no line.
        assert_eq!(connector((250.0, 90.0, 394.0, 169.0), cell), None);
    }

    #[test]
    fn segments_are_cut_at_the_pane() {
        let clip = (0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            clip_segment((10.0, 10.0), (50.0, 50.0), clip),
            Some(((10.0, 10.0), (50.0, 50.0)))
        );
        assert_eq!(
            clip_segment((-50.0, 50.0), (50.0, 50.0), clip),
            Some(((0.0, 50.0), (50.0, 50.0)))
        );
        assert_eq!(
            clip_segment((50.0, 50.0), (50.0, 150.0), clip),
            Some(((50.0, 50.0), (50.0, 100.0)))
        );
        assert_eq!(clip_segment((-10.0, 50.0), (-5.0, 80.0), clip), None);
        assert_eq!(clip_segment((120.0, -20.0), (150.0, 20.0), clip), None);
    }
}
