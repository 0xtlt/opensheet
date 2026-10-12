//! Charts, pictures and shapes floating over the cells: what they have in
//! common. Selecting, moving, resizing with the eight handles, nudging with
//! the arrow keys, deleting, copying, pasting and restacking are done here
//! once; a feature only says which objects exist and how to draw, place and
//! remove them (the `*_object` hooks of `features::Feature`).

use crate::app::{Drag, ExcelApp, Hit};
use crate::features::{self, Intent, ObjectId, ObjectInfo, ObjectVerb, Painter, Placement, ZMove};
use crate::grid::GridLayout;
use crate::theme::c;
use gpui::{ClipboardItem, Context};
use opensheet_engine::ops::Direction;

const HANDLE: u32 = 0x8A8A8A;

/// Pixel box (x0, y0, x1, y1) of an object in grid-element coordinates.
pub fn object_rect(layout: &GridLayout, place: &Placement) -> (f32, f32, f32, f32) {
    let z = layout.zoom;
    let x0 = layout.col_x(place.anchor.1) + place.offset.0 * z;
    let y0 = layout.row_y(place.anchor.0) + place.offset.1 * z;
    (x0, y0, x0 + place.size.0 * z, y0 + place.size.1 * z)
}

/// The eight resize handles: corners and edge midpoints, clockwise from the
/// top-left corner.
pub fn handle_points(rect: (f32, f32, f32, f32)) -> [(f32, f32); 8] {
    let (x0, y0, x1, y1) = rect;
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    [(x0, y0), (mx, y0), (x1, y0), (x1, my), (x1, y1), (mx, y1), (x0, y1), (x0, my)]
}

/// The frame and the handles of the selected object.
pub fn paint_selection(p: &mut Painter, rect: (f32, f32, f32, f32)) {
    let (x0, y0, x1, y1) = rect;
    p.frame(x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0, 1.0, c(HANDLE));
    for (hx, hy) in handle_points(rect) {
        p.disc(hx, hy, 4.5, c(HANDLE));
        p.disc(hx, hy, 3.5, c(0xFFFFFF));
    }
}

/// An object's top-left corner in sheet pixels at 100 %.
fn corner(layout: &GridLayout, place: &Placement) -> (f64, f64) {
    let z = layout.zoom as f64;
    (
        layout.cols.start(place.anchor.1) / z + place.offset.0 as f64,
        layout.rows.start(place.anchor.0) / z + place.offset.1 as f64,
    )
}

/// The cell anchor and in-cell offset for a position in sheet pixels.
fn anchor_at(layout: &GridLayout, x: f64, y: f64) -> ((u32, u32), (f32, f32)) {
    let z = layout.zoom as f64;
    let (x, y) = (x.max(0.0), y.max(0.0));
    let col = layout.cols.index_at(x * z);
    let row = layout.rows.index_at(y * z);
    (
        (row, col),
        (
            (x - layout.cols.start(col) / z) as f32,
            (y - layout.rows.start(row) / z) as f32,
        ),
    )
}

impl ExcelApp {
    /// The selected object of the current sheet.
    pub fn selected_object(&self) -> Option<(ObjectId, ObjectInfo)> {
        let sel = self.object_sel?;
        features::objects(self, self.sheet).into_iter().find(|(id, _)| *id == sel)
    }

    /// Select a chart, picture or shape of the current sheet (`None`
    /// deselects).
    pub fn select_object(&mut self, id: Option<ObjectId>, cx: &mut Context<Self>) {
        if self.object_sel == id {
            return;
        }
        if let Some(old) = self.object_sel.take() {
            features::all()[old.owner].object_selected(self, None, cx);
        }
        self.object_sel = id;
        if let Some(new) = id {
            features::all()[new.owner].object_selected(self, Some(new.id), cx);
        }
        cx.notify();
    }

    pub fn deselect_object(&mut self, cx: &mut Context<Self>) {
        self.select_object(None, cx);
    }

    /// The object or handle under a point of the grid element; the last
    /// object drawn is on top.
    pub(crate) fn hit_object(&self, layout: &GridLayout, x: f32, y: f32) -> Option<Hit> {
        let objects = features::objects(self, self.sheet);
        for (id, object) in objects.iter().rev() {
            let rect = object_rect(layout, &object.place);
            let feature = features::all()[id.owner];
            if self.object_sel == Some(*id) && !feature.object_own_handles(self, id.id) {
                for (i, (hx, hy)) in handle_points(rect).iter().enumerate() {
                    if (x - hx).abs() <= 6.0 && (y - hy).abs() <= 6.0 {
                        return Some(Hit::ObjectHandle(*id, i as u8));
                    }
                }
            }
            // A feature may know better than the frame (a line, a rotated
            // shape).
            let inside = feature
                .object_hit(self, id.id, layout, x, y)
                .unwrap_or(x >= rect.0 && x <= rect.2 && y >= rect.1 && y <= rect.3);
            if inside {
                return Some(Hit::Object(*id));
            }
        }
        None
    }

    /// Start moving (or resizing, by a handle) the object under the pointer.
    pub(crate) fn begin_object_drag(&mut self, layout: &GridLayout, id: ObjectId, handle: Option<u8>, start: (f32, f32)) {
        let Some((_, object)) = self.selected_object().filter(|(sel, _)| *sel == id) else {
            return;
        };
        let (x, y) = corner(layout, &object.place);
        let size = object.place.size;
        self.drag = Some(match handle {
            Some(handle) => Drag::ObjectResize {
                id,
                handle,
                start,
                origin: (x, y, size.0 as f64, size.1 as f64),
            },
            None => Drag::ObjectMove {
                id,
                start,
                origin: (x, y),
                size,
            },
        });
    }

    pub(crate) fn drag_object_move(&mut self, layout: &GridLayout, id: ObjectId, start: (f32, f32), origin: (f64, f64), size: (f32, f32), at: (f32, f32)) {
        let z = self.zoom as f64;
        let x = origin.0 + (at.0 - start.0) as f64 / z;
        let y = origin.1 + (at.1 - start.1) as f64 / z;
        let (anchor, offset) = anchor_at(layout, x, y);
        features::all()[id.owner].place_object(self, id.id, Placement { anchor, offset, size });
    }

    pub(crate) fn drag_object_resize(
        &mut self,
        layout: &GridLayout,
        id: ObjectId,
        handle: u8,
        start: (f32, f32),
        origin: (f64, f64, f64, f64),
        at: (f32, f32),
    ) {
        let Some((_, object)) = self.selected_object().filter(|(sel, _)| *sel == id) else {
            return;
        };
        let z = self.zoom as f64;
        let mut dx = (at.0 - start.0) as f64 / z;
        let mut dy = (at.1 - start.1) as f64 / z;
        let (mut x, mut y, mut w, mut h) = origin;
        let (min_w, min_h) = (object.min_size.0 as f64, object.min_size.1 as f64);
        // Handles: 0 top-left, 1 top, 2 top-right, 3 right, 4 bottom-right,
        // 5 bottom, 6 bottom-left, 7 left.
        if object.keep_aspect && handle % 2 == 0 && w > 0.0 && h > 0.0 {
            // A corner keeps the proportions: follow the larger movement.
            let sx = if matches!(handle, 0 | 6) { -dx } else { dx };
            let sy = if matches!(handle, 0 | 2) { -dy } else { dy };
            let grow = (sx / w).max(sy / h);
            dx = if matches!(handle, 0 | 6) { -grow * w } else { grow * w };
            dy = if matches!(handle, 0 | 2) { -grow * h } else { grow * h };
        }
        if matches!(handle, 0 | 6 | 7) {
            let d = dx.min(w - min_w);
            x += d;
            w -= d;
        }
        if matches!(handle, 2 | 3 | 4) {
            w = (w + dx).max(min_w);
        }
        if matches!(handle, 0 | 1 | 2) {
            let d = dy.min(h - min_h);
            y += d;
            h -= d;
        }
        if matches!(handle, 4 | 5 | 6) {
            h = (h + dy).max(min_h);
        }
        let (anchor, offset) = anchor_at(layout, x, y);
        features::all()[id.owner].place_object(
            self,
            id.id,
            Placement {
                anchor,
                offset,
                size: (w as f32, h as f32),
            },
        );
    }

    /// Close the undo step of something done to an object.
    pub(crate) fn commit_object(&mut self, id: ObjectId, verb: ObjectVerb, cx: &mut Context<Self>) {
        let label = features::all()[id.owner].object_label(id.id, verb);
        self.commit(label, cx);
    }

    /// Delete the selected object (Delete / Backspace).
    pub fn delete_selected_object(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.object_sel else {
            return;
        };
        if !self.may(Intent::EditObject(Some(id)), cx) {
            return;
        }
        let label = features::all()[id.owner].object_label(id.id, ObjectVerb::Delete);
        features::all()[id.owner].delete_object(self, id.id);
        self.deselect_object(cx);
        self.commit(label, cx);
    }

    /// Move the selected object one pixel with an arrow key. Returns false
    /// when no object is selected.
    pub fn nudge_object(&mut self, dir: Direction, cx: &mut Context<Self>) -> bool {
        let Some((id, object)) = self.selected_object() else {
            return false;
        };
        let Some(layout) = self.current_layout() else {
            return true;
        };
        if !self.may(Intent::EditObject(Some(id)), cx) {
            return true;
        }
        let (x, y) = corner(&layout, &object.place);
        let (dx, dy) = match dir {
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
        };
        let (anchor, offset) = anchor_at(&layout, x + dx, y + dy);
        features::all()[id.owner].place_object(
            self,
            id.id,
            Placement {
                anchor,
                offset,
                size: object.place.size,
            },
        );
        self.commit_object(id, ObjectVerb::Move, cx);
        true
    }

    /// Select the object after (or before) the selected one, as Tab does
    /// while an object is selected.
    pub fn cycle_object(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let Some(sel) = self.object_sel else {
            return false;
        };
        let objects = features::objects(self, self.sheet);
        if let Some(i) = objects.iter().position(|(id, _)| *id == sel) {
            let n = objects.len();
            let next = if forward { (i + 1) % n } else { (i + n - 1) % n };
            self.select_object(Some(objects[next].0), cx);
        }
        true
    }

    /// Copy (or cut) the selected object.
    pub(crate) fn copy_object(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some(id) = self.object_sel else {
            return;
        };
        let Some(data) = features::all()[id.owner].copy_object(self, id.id) else {
            return;
        };
        self.object_clip = Some((id.owner, data));
        self.clip = None;
        // Other applications get what the feature offers them (a picture);
        // what the system clipboard held before is no longer what Paste
        // means.
        let item = features::all()[id.owner].object_clipboard(self, id.id);
        cx.write_to_clipboard(item.unwrap_or_else(|| ClipboardItem::new_string(String::new())));
        if cut {
            self.delete_selected_object(cx);
        }
        cx.notify();
    }

    /// Paste the object put aside by Copy: a little below and right of the
    /// selected object, or at the active cell.
    pub(crate) fn paste_object(&mut self, cx: &mut Context<Self>) {
        let Some((owner, data)) = self.object_clip.clone() else {
            return;
        };
        if !self.may(Intent::EditObject(None), cx) {
            return;
        }
        let at = match (self.selected_object(), self.current_layout()) {
            (Some((_, object)), Some(layout)) => {
                let (x, y) = corner(&layout, &object.place);
                anchor_at(&layout, x + 16.0, y + 16.0)
            }
            _ => (self.active(), (0.0, 0.0)),
        };
        let feature = features::all()[owner];
        if let Some(new) = feature.paste_object(self, data.as_ref(), at) {
            let id = ObjectId { owner, id: new };
            self.commit_object(id, ObjectVerb::Paste, cx);
            self.select_object(Some(id), cx);
        }
    }

    /// Bring the selected object forward or send it backward.
    pub fn reorder_object(&mut self, mv: ZMove, cx: &mut Context<Self>) {
        let Some(id) = self.object_sel else {
            return;
        };
        if !self.may(Intent::EditObject(Some(id)), cx) {
            return;
        }
        features::all()[id.owner].reorder_object(self, id.id, mv);
        self.commit_object(id, ObjectVerb::Reorder, cx);
    }

    /// Where a new object of the given size goes: the middle of the
    /// visible cells.
    pub fn place_new_object(&self, size: (f32, f32)) -> Placement {
        let Some(layout) = self.current_layout() else {
            return Placement {
                anchor: self.active(),
                offset: (0.0, 0.0),
                size,
            };
        };
        let z = layout.zoom;
        let lx = layout.header_w + layout.frozen_w + ((layout.view_w() - layout.frozen_w - size.0 * z) / 2.0).max(8.0);
        let ly = layout.header_h + layout.frozen_h + ((layout.view_h() - layout.frozen_h - size.1 * z) / 2.0).max(8.0);
        let (col, row) = (layout.col_at(lx), layout.row_at(ly));
        Placement {
            anchor: (row, col),
            offset: (((lx - layout.col_x(col)) / z).max(0.0), ((ly - layout.row_y(row)) / z).max(0.0)),
            size,
        }
    }
}
