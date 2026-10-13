//! Charts, pictures and shapes floating over the cells: what they have in
//! common. Selecting (one object or several), moving, resizing with the
//! eight handles, nudging with the arrow keys, deleting, copying, pasting,
//! restacking, aligning and distributing are done here once; a feature
//! only says which objects exist and how to draw, place and remove them
//! (the `*_object` hooks of `features::Feature`).
//!
//! Several objects can be selected: `ExcelApp::object_sel` is the one
//! that was selected last (its feature shows its tab and its panes),
//! `ExcelApp::object_more` the others. What is done to the selection is
//! done to all of them as one undo step.
//!
//! Charts, pictures and shapes share one stacking order, which the engine
//! keeps (`Sheet::object_stack`); the objects of other features lie above
//! them, in the order of their features.

use crate::app::{Drag, ExcelApp, Hit};
use crate::features::{
    self, Intent, ObjectId, ObjectInfo, ObjectVerb, PaintLayer, Painter, Placement, ZMove, Zone, ZoneEvent, ZoneResponse,
};
use crate::grid::GridLayout;
use crate::i18n::t;
use crate::theme::{self, c};
use gpui::{ClipboardItem, Context, CursorStyle, MouseButton};
use opensheet_engine::drawing::{ObjectRef, ZOrder};
use opensheet_engine::ops::Direction;
use std::any::Any;
use std::rc::Rc;

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

/// A frame on the sheet: x, y, width, height in pixels at 100 %.
pub type Box4 = (f64, f64, f64, f64);

/// One of the objects Copy put aside, with where it was from the corner
/// of the box around all of them.
#[derive(Clone)]
struct Clip {
    owner: usize,
    data: Rc<dyn Any>,
    offset: (f64, f64),
}

/// What the object platform remembers between events.
#[derive(Default)]
struct ObjectsUi {
    /// Where every selected object was when a drag began.
    origins: Vec<(ObjectId, Box4)>,
    /// The objects Copy put aside, bottom-most first (`ExcelApp::object_clip`
    /// holds the first of them and says whether they are still there).
    clips: Vec<Clip>,
    /// "Select Objects": the pointer selects objects, never cells.
    pointer: bool,
    /// The rectangle being dragged out in that mode, in grid-element
    /// coordinates: where the press was and where the pointer is.
    band: Option<((f32, f32), (f32, f32))>,
    /// "Snap to Grid": objects are moved and sized from cell edge to cell
    /// edge.
    snap: bool,
    /// "Snap to Shape": the edges of the other objects count as lines of
    /// the grid too.
    snap_shape: bool,
}

/// The zone that covers the cells while "Select Objects" is on.
struct PointerZone;

/// The commands of the Align menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    DistributeHorizontally,
    DistributeVertically,
}

/// Where Align puts each of `frames` (x, y of each, in the same order).
/// Objects line up on the edge of the outermost one, or on the middle of
/// the box around them all; distributing leaves the two outermost objects
/// where they are and puts the same gap between the others.
pub fn aligned(frames: &[Box4], how: Align) -> Vec<(f64, f64)> {
    if frames.is_empty() {
        return Vec::new();
    }
    let left = frames.iter().map(|f| f.0).fold(f64::INFINITY, f64::min);
    let top = frames.iter().map(|f| f.1).fold(f64::INFINITY, f64::min);
    let right = frames.iter().map(|f| f.0 + f.2).fold(f64::NEG_INFINITY, f64::max);
    let bottom = frames.iter().map(|f| f.1 + f.3).fold(f64::NEG_INFINITY, f64::max);
    // The positions along one axis that put equal gaps between objects
    // given as (start, length).
    let distribute = |spans: Vec<(f64, f64)>| -> Vec<f64> {
        let mut order: Vec<usize> = (0..spans.len()).collect();
        order.sort_by(|a, b| spans[*a].0.total_cmp(&spans[*b].0));
        let mut out: Vec<f64> = spans.iter().map(|s| s.0).collect();
        if spans.len() < 3 {
            return out;
        }
        let start = spans[order[0]].0;
        let end = spans.iter().map(|s| s.0 + s.1).fold(f64::NEG_INFINITY, f64::max);
        let taken: f64 = spans.iter().map(|s| s.1).sum();
        let gap = (end - start - taken) / (spans.len() - 1) as f64;
        let mut at = start;
        for i in order {
            out[i] = at;
            at += spans[i].1 + gap;
        }
        out
    };
    match how {
        Align::Left => frames.iter().map(|f| (left, f.1)).collect(),
        Align::Center => frames.iter().map(|f| ((left + right - f.2) / 2.0, f.1)).collect(),
        Align::Right => frames.iter().map(|f| (right - f.2, f.1)).collect(),
        Align::Top => frames.iter().map(|f| (f.0, top)).collect(),
        Align::Middle => frames.iter().map(|f| (f.0, (top + bottom - f.3) / 2.0)).collect(),
        Align::Bottom => frames.iter().map(|f| (f.0, bottom - f.3)).collect(),
        Align::DistributeHorizontally => {
            let xs = distribute(frames.iter().map(|f| (f.0, f.2)).collect());
            frames.iter().zip(xs).map(|(f, x)| (x, f.1)).collect()
        }
        Align::DistributeVertically => {
            let ys = distribute(frames.iter().map(|f| (f.1, f.3)).collect());
            frames.iter().zip(ys).map(|(f, y)| (f.0, y)).collect()
        }
    }
}

/// What a drag of a resize handle makes of a frame when it has made the
/// frame of the object it belongs to `sx` times as wide and `sy` times as
/// high: the same proportions, from the same side (the opposite edge
/// stays where it is). Handles: 0 top-left, then clockwise.
pub fn scaled(frame: Box4, handle: u8, sx: f64, sy: f64, min: (f64, f64)) -> Box4 {
    let (mut x, mut y, w, h) = frame;
    let nw = (w * sx).max(min.0.min(w));
    let nh = (h * sy).max(min.1.min(h));
    if matches!(handle, 0 | 6 | 7) {
        x += w - nw;
    }
    if matches!(handle, 0 | 1 | 2) {
        y += h - nh;
    }
    (x, y, nw, nh)
}

/// The chart, picture or shape of the engine an object is, when it is one
/// of those.
pub fn engine_ref(id: ObjectId) -> Option<ObjectRef> {
    if id.owner == features::index_of(&features::charts::Charts) {
        Some(ObjectRef::Chart(id.id))
    } else if id.owner == features::index_of(&features::drawings::Drawings) {
        Some(ObjectRef::Drawing(id.id))
    } else {
        None
    }
}

/// The object a chart, picture or shape of the engine is.
pub fn object_of(object: ObjectRef) -> ObjectId {
    match object {
        ObjectRef::Chart(id) => features::object_id(&features::charts::Charts, id),
        ObjectRef::Drawing(id) => features::object_id(&features::drawings::Drawings, id),
    }
}

/// Put the objects of a sheet in drawing order: charts, pictures and
/// shapes in the order the engine keeps for them, the objects of other
/// features above, as they came.
pub(crate) fn stack(app: &ExcelApp, sheet: usize, objects: &mut [(ObjectId, ObjectInfo)]) {
    let Some(sh) = app.wb.sheets.get(sheet) else {
        return;
    };
    let order = sh.object_stack();
    let place = |id: &ObjectId| {
        engine_ref(*id)
            .and_then(|object| order.iter().position(|o| *o == object))
            .unwrap_or(usize::MAX)
    };
    objects.sort_by_key(|(id, _)| place(id));
}

/// While "Select Objects" is on: the zone that keeps presses from the
/// cells (in every pane, under the objects), and the rectangle being
/// dragged out (once, over everything).
pub(crate) fn paint_pointer(p: &mut Painter) {
    let Some(ui) = p.app.ext::<ObjectsUi>().filter(|ui| ui.pointer) else {
        return;
    };
    match p.layer {
        PaintLayer::Over => {
            p.zone(p.clip, PointerZone).cursor(CursorStyle::Arrow);
        }
        PaintLayer::Top => {
            if let Some((from, to)) = ui.band {
                let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
                let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
                p.rect(x0, y0, x1, y1, c(theme::ACCENT).opacity(0.12));
                p.frame(x0, y0, x1, y1, 1.0, c(theme::ACCENT));
            }
        }
        PaintLayer::Under => {}
    }
}

/// A press on the cells while "Select Objects" is on starts a rectangle.
/// `None` when the zone is not the pointer's.
pub(crate) fn pointer_mouse_down(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Context<ExcelApp>) -> Option<ZoneResponse> {
    zone.payload::<PointerZone>()?;
    if ev.button != MouseButton::Left {
        return Some(ZoneResponse::Handled);
    }
    if !(ev.modifiers.shift || ev.modifiers.platform || ev.modifiers.control) {
        app.deselect_object(cx);
    }
    app.ext_mut::<ObjectsUi>().band = Some(((ev.x, ev.y), (ev.x, ev.y)));
    Some(ZoneResponse::Capture)
}

pub(crate) fn pointer_drag(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Context<ExcelApp>) -> bool {
    if zone.payload::<PointerZone>().is_none() {
        return false;
    }
    if let Some(band) = &mut app.ext_mut::<ObjectsUi>().band {
        band.1 = (ev.x, ev.y);
    }
    cx.notify();
    true
}

/// The release selects the objects that lie wholly in the rectangle,
/// with those that were selected when Shift or ⌘ is held.
pub(crate) fn pointer_up(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Context<ExcelApp>) -> bool {
    if zone.payload::<PointerZone>().is_none() {
        return false;
    }
    let Some((from, to)) = app.ext_mut::<ObjectsUi>().band.take() else {
        return true;
    };
    let Some(layout) = app.current_layout() else {
        return true;
    };
    let (x0, x1) = (from.0.min(to.0), from.0.max(to.0));
    let (y0, y1) = (from.1.min(to.1), from.1.max(to.1));
    let mut ids = if ev.modifiers.shift || ev.modifiers.platform || ev.modifiers.control {
        app.selected_objects()
    } else {
        Vec::new()
    };
    for (id, object) in features::objects(app, app.sheet) {
        let rect = object_rect(&layout, &object.place);
        if rect.0 >= x0 && rect.1 >= y0 && rect.2 <= x1 && rect.3 <= y1 && !ids.contains(&id) {
            // The last one found is the topmost: it leads.
            ids.insert(0, id);
        }
    }
    app.select_objects(&ids, cx);
    cx.notify();
    true
}

impl ExcelApp {
    /// The selected object of the current sheet; of several, the one
    /// selected last.
    pub fn selected_object(&self) -> Option<(ObjectId, ObjectInfo)> {
        let sel = self.object_sel?;
        features::objects(self, self.sheet).into_iter().find(|(id, _)| *id == sel)
    }

    /// Every selected object, the one selected last first.
    pub fn selected_objects(&self) -> Vec<ObjectId> {
        self.object_sel.into_iter().chain(self.object_more.iter().copied()).collect()
    }

    /// Every selected object with where it is, bottom-most first.
    pub fn selected_object_infos(&self) -> Vec<(ObjectId, ObjectInfo)> {
        features::objects(self, self.sheet)
            .into_iter()
            .filter(|(id, _)| self.is_object_selected(*id))
            .collect()
    }

    pub fn is_object_selected(&self, id: ObjectId) -> bool {
        self.object_sel == Some(id) || self.object_more.contains(&id)
    }

    /// Select one chart, picture or shape of the current sheet (`None`
    /// deselects).
    pub fn select_object(&mut self, id: Option<ObjectId>, cx: &mut Context<Self>) {
        self.set_object_selection(id, Vec::new(), cx);
    }

    /// Select several objects of the current sheet; the first one leads
    /// (its feature shows its tab).
    pub fn select_objects(&mut self, ids: &[ObjectId], cx: &mut Context<Self>) {
        let mut rest: Vec<ObjectId> = Vec::new();
        for id in ids.iter().skip(1) {
            if Some(id) != ids.first() && !rest.contains(id) {
                rest.push(*id);
            }
        }
        self.set_object_selection(ids.first().copied(), rest, cx);
    }

    fn set_object_selection(&mut self, lead: Option<ObjectId>, more: Vec<ObjectId>, cx: &mut Context<Self>) {
        if self.object_sel == lead && self.object_more == more {
            return;
        }
        let old = self.object_sel;
        self.object_sel = lead;
        self.object_more = more;
        if let Some(old) = old.filter(|old| Some(*old) != lead) {
            features::all()[old.owner].object_selected(self, None, cx);
        }
        // Also when only the others changed: panes show what is common.
        if let Some(new) = lead {
            features::all()[new.owner].object_selected(self, Some(new.id), cx);
        }
        cx.notify();
    }

    /// Take out of the selection the objects that are no longer on the
    /// sheet (after an undo or a redo took them away).
    pub(crate) fn prune_object_selection(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_objects();
        if selected.is_empty() {
            return;
        }
        let there: Vec<ObjectId> = features::objects(self, self.sheet).into_iter().map(|(id, _)| id).collect();
        let kept: Vec<ObjectId> = selected.iter().copied().filter(|id| there.contains(id)).collect();
        if kept.len() != selected.len() {
            self.select_objects(&kept, cx);
        }
    }

    pub fn deselect_object(&mut self, cx: &mut Context<Self>) {
        self.select_object(None, cx);
    }

    /// A click on an object: it becomes the selection, or with Shift or ⌘
    /// (`extend`) joins or leaves it. A click on one of several selected
    /// objects keeps them all, so that they can be dragged together.
    /// Returns false when the object is not selected afterwards.
    pub(crate) fn click_object(&mut self, id: ObjectId, extend: bool, cx: &mut Context<Self>) -> bool {
        let selected = self.selected_objects();
        if extend {
            if selected.contains(&id) {
                let rest: Vec<ObjectId> = selected.into_iter().filter(|other| *other != id).collect();
                self.select_objects(&rest, cx);
                return false;
            }
            let mut all = vec![id];
            all.extend(selected);
            self.select_objects(&all, cx);
        } else if !selected.contains(&id) {
            self.select_object(Some(id), cx);
        } else if self.object_sel != Some(id) {
            // It leads from now on.
            let mut all = vec![id];
            all.extend(selected.into_iter().filter(|other| *other != id));
            self.select_objects(&all, cx);
        }
        true
    }

    /// Select every object of the sheet (⌘A with an object selected, Go
    /// To Special ▸ Objects). False when the sheet has none.
    pub fn select_all_objects(&mut self, cx: &mut Context<Self>) -> bool {
        let mut ids: Vec<ObjectId> = features::objects(self, self.sheet).into_iter().map(|(id, _)| id).collect();
        // The one that leads stays; else the topmost does.
        ids.reverse();
        if let Some(lead) = self.object_sel.filter(|lead| ids.contains(lead)) {
            ids.retain(|id| *id != lead);
            ids.insert(0, lead);
        }
        self.select_objects(&ids, cx);
        !ids.is_empty()
    }

    /// "Select Objects" of Find & Select: the pointer selects objects and
    /// drags out a rectangle around those to select, until the command is
    /// chosen again or Escape is pressed.
    pub fn toggle_object_pointer(&mut self, cx: &mut Context<Self>) {
        let ui = self.ext_mut::<ObjectsUi>();
        ui.pointer = !ui.pointer;
        ui.band = None;
        cx.notify();
    }

    pub fn object_pointer(&self) -> bool {
        self.ext::<ObjectsUi>().is_some_and(|ui| ui.pointer)
    }

    /// Leave "Select Objects" (Escape). False when it was not on.
    pub(crate) fn end_object_pointer(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.object_pointer() {
            return false;
        }
        self.toggle_object_pointer(cx);
        true
    }

    pub fn snap_to_grid(&self) -> bool {
        self.ext::<ObjectsUi>().is_some_and(|ui| ui.snap)
    }

    pub fn toggle_snap_to_grid(&mut self, cx: &mut Context<Self>) {
        let ui = self.ext_mut::<ObjectsUi>();
        ui.snap = !ui.snap;
        // Snap to Shape works on the grid: it goes with it.
        ui.snap_shape &= ui.snap;
        cx.notify();
    }

    pub fn snap_to_shape(&self) -> bool {
        self.ext::<ObjectsUi>().is_some_and(|ui| ui.snap_shape)
    }

    /// "Snap to Shape": the grid objects snap to gains the lines that run
    /// through the edges of the other objects. As in Excel, turning it on
    /// turns Snap to Grid on.
    pub fn toggle_snap_to_shape(&mut self, cx: &mut Context<Self>) {
        let ui = self.ext_mut::<ObjectsUi>();
        ui.snap_shape = !ui.snap_shape;
        ui.snap |= ui.snap_shape;
        cx.notify();
    }

    /// The lines through the edges of the objects that are not selected:
    /// where their left and right sides are, and their tops and bottoms.
    fn shape_lines(&self) -> (Vec<f64>, Vec<f64>) {
        let (mut across, mut down) = (Vec::new(), Vec::new());
        let Some(layout) = self.current_layout() else {
            return (across, down);
        };
        for (id, object) in features::objects(self, self.sheet) {
            if self.is_object_selected(id) {
                continue;
            }
            let (x, y) = corner(&layout, &object.place);
            across.extend([x, x + object.place.size.0 as f64]);
            down.extend([y, y + object.place.size.1 as f64]);
        }
        (across, down)
    }

    /// With "Snap to Grid" on, the nearest cell corner to a point of the
    /// sheet (each coordinate on its own); with "Snap to Shape" too, an
    /// edge of another object when one is nearer; else the point.
    pub(crate) fn snapped(&self, x: f64, y: f64) -> (f64, f64) {
        if !self.snap_to_grid() {
            return (x, y);
        }
        let sheet = &self.wb.sheets[self.sheet];
        let ((row, col), (dx, dy)) = sheet.corner_at(x.max(0.0), y.max(0.0));
        let (x0, y0) = sheet.corner_point(((row, col), (0.0, 0.0)));
        let (x1, y1) = sheet.corner_point(((row + 1, col + 1), (0.0, 0.0)));
        let on_grid = (
            if (dx as f64) < (x1 - x0) / 2.0 { x0 } else { x1 },
            if (dy as f64) < (y1 - y0) / 2.0 { y0 } else { y1 },
        );
        if !self.snap_to_shape() {
            return on_grid;
        }
        let (across, down) = self.shape_lines();
        let nearest = |to: f64, grid: f64, lines: &[f64]| {
            lines.iter().copied().fold(grid, |best, line| if (line - to).abs() < (best - to).abs() { line } else { best })
        };
        (nearest(x, on_grid.0, &across), nearest(y, on_grid.1, &down))
    }

    /// The object or handle under a point of the grid element; the last
    /// object drawn is on top.
    pub(crate) fn hit_object(&self, layout: &GridLayout, x: f32, y: f32) -> Option<Hit> {
        let objects = features::objects(self, self.sheet);
        for (id, object) in objects.iter().rev() {
            let rect = object_rect(layout, &object.place);
            let feature = features::all()[id.owner];
            if self.is_object_selected(*id) && !feature.object_own_handles(self, id.id) {
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

    /// Where an object is on the sheet.
    pub(crate) fn object_box(&self, layout: &GridLayout, object: &ObjectInfo) -> Box4 {
        let (x, y) = corner(layout, &object.place);
        (x, y, object.place.size.0 as f64, object.place.size.1 as f64)
    }

    /// Put an object in a frame of the sheet.
    pub(crate) fn place_object_box(&mut self, layout: &GridLayout, id: ObjectId, frame: Box4) {
        let (anchor, offset) = anchor_at(layout, frame.0, frame.1);
        features::all()[id.owner].place_object(
            self,
            id.id,
            Placement {
                anchor,
                offset,
                size: (frame.2 as f32, frame.3 as f32),
            },
        );
    }

    /// Note where the selected objects are, before a drag changes one of
    /// them: the others follow from there.
    pub(crate) fn remember_object_boxes(&mut self, layout: &GridLayout) {
        let origins = self
            .selected_object_infos()
            .iter()
            .map(|(id, object)| (*id, self.object_box(layout, object)))
            .collect();
        self.ext_mut::<ObjectsUi>().origins = origins;
    }

    /// While a handle of the object `lead` is dragged: give the other
    /// selected objects the same proportions (`sx`, `sy`: how many times
    /// wider and higher `lead` became since the drag began).
    pub(crate) fn scale_other_objects(&mut self, layout: &GridLayout, lead: ObjectId, handle: u8, sx: f64, sy: f64) {
        let origins = self.ext::<ObjectsUi>().map(|ui| ui.origins.clone()).unwrap_or_default();
        let sizes: Vec<(ObjectId, (f32, f32))> = self
            .selected_object_infos()
            .into_iter()
            .map(|(id, object)| (id, object.min_size))
            .collect();
        for (id, origin) in origins {
            if id == lead || !sx.is_finite() || !sy.is_finite() {
                continue;
            }
            let Some((_, min)) = sizes.iter().find(|(other, _)| *other == id) else {
                continue;
            };
            let frame = scaled(origin, handle, sx, sy, (min.0 as f64, min.1 as f64));
            self.place_object_box(layout, id, frame);
        }
    }

    /// Start moving (or resizing, by a handle) the object under the
    /// pointer, and the other selected objects with it.
    pub(crate) fn begin_object_drag(&mut self, layout: &GridLayout, id: ObjectId, handle: Option<u8>, start: (f32, f32)) {
        let Some((_, object)) = features::objects(self, self.sheet)
            .into_iter()
            .find(|(other, _)| *other == id && self.is_object_selected(id))
        else {
            return;
        };
        // An object the sheet's protection holds stays where it is, and
        // so do the others.
        if self.selected_objects().iter().any(|other| features::guard(self, &Intent::EditObject(Some(*other))).is_some()) {
            return;
        }
        self.remember_object_boxes(layout);
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
        let (free_x, free_y) = (x, y);
        let (mut x, mut y) = self.snapped(x, y);
        if self.snap_to_shape() {
            // The far edges of the object snap as well as its corner:
            // whichever has less far to go wins.
            let (w, h) = (size.0 as f64, size.1 as f64);
            let (far_x, far_y) = self.snapped(free_x + w, free_y + h);
            if (far_x - w - free_x).abs() < (x - free_x).abs() {
                x = far_x - w;
            }
            if (far_y - h - free_y).abs() < (y - free_y).abs() {
                y = far_y - h;
            }
        }
        let others: Vec<(ObjectId, Box4)> = self
            .ext::<ObjectsUi>()
            .map(|ui| ui.origins.iter().filter(|(other, _)| *other != id).cloned().collect())
            .unwrap_or_default();
        // No object leaves the sheet: the others would close up on it.
        let (mut dx, mut dy) = (x - origin.0, y - origin.1);
        for (_, frame) in &others {
            dx = dx.max(-frame.0);
            dy = dy.max(-frame.1);
        }
        dx = dx.max(-origin.0);
        dy = dy.max(-origin.1);
        self.place_object_box(layout, id, (origin.0 + dx, origin.1 + dy, size.0 as f64, size.1 as f64));
        for (other, frame) in others {
            self.place_object_box(layout, other, (frame.0 + dx, frame.1 + dy, frame.2, frame.3));
        }
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
        let Some((_, object)) = self.selected_object_infos().into_iter().find(|(sel, _)| *sel == id) else {
            return;
        };
        let z = self.zoom as f64;
        let mut dx = (at.0 - start.0) as f64 / z;
        let mut dy = (at.1 - start.1) as f64 / z;
        let (mut x, mut y, mut w, mut h) = origin;
        if self.snap_to_grid() {
            // The edge being dragged goes from cell edge to cell edge.
            let (ex, ey) = (
                if matches!(handle, 0 | 6 | 7) { x } else { x + w },
                if matches!(handle, 0 | 1 | 2) { y } else { y + h },
            );
            let (sx, sy) = self.snapped(ex + dx, ey + dy);
            (dx, dy) = (sx - ex, sy - ey);
        }
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
        self.place_object_box(layout, id, (x, y, w, h));
        if origin.2 > 0.0 && origin.3 > 0.0 {
            self.scale_other_objects(layout, id, handle, w / origin.2, h / origin.3);
        }
    }

    /// Close the undo step of something done to an object.
    pub(crate) fn commit_object(&mut self, id: ObjectId, verb: ObjectVerb, cx: &mut Context<Self>) {
        let label = features::all()[id.owner].object_label(id.id, verb);
        self.commit(label, cx);
    }

    /// Whether every selected object may be changed; the first refusal
    /// is shown.
    pub(crate) fn may_edit_selected_objects(&mut self, cx: &mut Context<Self>) -> bool {
        self.selected_objects().into_iter().all(|id| self.may(Intent::EditObject(Some(id)), cx))
    }

    /// Delete the selected objects (Delete / Backspace).
    pub fn delete_selected_object(&mut self, cx: &mut Context<Self>) {
        let Some(lead) = self.object_sel else {
            return;
        };
        if !self.may_edit_selected_objects(cx) {
            return;
        }
        let label = features::all()[lead.owner].object_label(lead.id, ObjectVerb::Delete);
        for id in self.selected_objects() {
            features::all()[id.owner].delete_object(self, id.id);
        }
        self.deselect_object(cx);
        self.commit(label, cx);
    }

    /// Move the selected objects one pixel with an arrow key. Returns
    /// false when no object is selected.
    pub fn nudge_object(&mut self, dir: Direction, cx: &mut Context<Self>) -> bool {
        let Some(lead) = self.object_sel else {
            return false;
        };
        let Some(layout) = self.current_layout() else {
            return true;
        };
        if !self.may_edit_selected_objects(cx) {
            return true;
        }
        let (dx, dy) = match dir {
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
        };
        for (id, object) in self.selected_object_infos() {
            let (x, y, w, h) = self.object_box(&layout, &object);
            self.place_object_box(&layout, id, (x + dx, y + dy, w, h));
        }
        self.commit_object(lead, ObjectVerb::Move, cx);
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

    /// Copy (or cut) the selected objects.
    pub(crate) fn copy_object(&mut self, cut: bool, cx: &mut Context<Self>) {
        let Some((lead, _)) = self.selected_object() else {
            return;
        };
        let Some(layout) = self.current_layout() else {
            return;
        };
        let infos = self.selected_object_infos();
        let boxes: Vec<Box4> = infos.iter().map(|(_, object)| self.object_box(&layout, object)).collect();
        let left = boxes.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
        let top = boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
        let mut clips = Vec::new();
        for ((id, _), frame) in infos.iter().zip(&boxes) {
            if let Some(data) = features::all()[id.owner].copy_object(self, id.id) {
                clips.push(Clip {
                    owner: id.owner,
                    data,
                    offset: (frame.0 - left, frame.1 - top),
                });
            }
        }
        let Some(first) = clips.first() else {
            return;
        };
        self.object_clip = Some((first.owner, first.data.clone()));
        self.ext_mut::<ObjectsUi>().clips = clips;
        self.clip = None;
        // Other applications get what the feature offers them (a picture);
        // what the system clipboard held before is no longer what Paste
        // means.
        let item = features::all()[lead.owner].object_clipboard(self, lead.id);
        cx.write_to_clipboard(item.unwrap_or_else(|| ClipboardItem::new_string(String::new())));
        if cut {
            self.delete_selected_object(cx);
        }
        cx.notify();
    }

    /// Paste the objects put aside by Copy: a little below and right of
    /// the selected objects, or at the active cell.
    pub(crate) fn paste_object(&mut self, cx: &mut Context<Self>) {
        let Some((owner, data)) = self.object_clip.clone() else {
            return;
        };
        if !self.may(Intent::EditObject(None), cx) {
            return;
        }
        // What Copy put aside, when it is still what `object_clip` says.
        let mut clips = self.ext::<ObjectsUi>().map(|ui| ui.clips.clone()).unwrap_or_default();
        if !clips.first().is_some_and(|first| Rc::ptr_eq(&first.data, &data)) {
            clips = vec![Clip {
                owner,
                data,
                offset: (0.0, 0.0),
            }];
        }
        let Some(layout) = self.current_layout() else {
            return;
        };
        let selected = self.selected_object_infos();
        let (x, y) = if selected.is_empty() {
            let (anchor, offset) = (self.active(), (0.0, 0.0));
            corner(&layout, &Placement { anchor, offset, size: (0.0, 0.0) })
        } else {
            let boxes: Vec<Box4> = selected.iter().map(|(_, object)| self.object_box(&layout, object)).collect();
            (
                boxes.iter().map(|b| b.0).fold(f64::INFINITY, f64::min) + 16.0,
                boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min) + 16.0,
            )
        };
        let mut pasted = Vec::new();
        for clip in clips {
            let at = anchor_at(&layout, x + clip.offset.0, y + clip.offset.1);
            if let Some(new) = features::all()[clip.owner].paste_object(self, clip.data.as_ref(), at) {
                pasted.push(ObjectId { owner: clip.owner, id: new });
            }
        }
        // The topmost of the copies leads.
        pasted.reverse();
        if let Some(lead) = pasted.first().copied() {
            self.commit_object(lead, ObjectVerb::Paste, cx);
            self.select_objects(&pasted, cx);
        }
    }

    /// Bring the selected objects forward or send them backward.
    pub fn reorder_object(&mut self, mv: ZMove, cx: &mut Context<Self>) {
        let Some(lead) = self.object_sel else {
            return;
        };
        if !self.may_edit_selected_objects(cx) {
            return;
        }
        let order = match mv {
            ZMove::ToFront => ZOrder::BringToFront,
            ZMove::Forward => ZOrder::BringForward,
            ZMove::Backward => ZOrder::SendBackward,
            ZMove::ToBack => ZOrder::SendToBack,
        };
        let selected = self.selected_objects();
        let known: Vec<ObjectRef> = selected.iter().filter_map(|id| engine_ref(*id)).collect();
        self.wb.reorder_objects(self.sheet, &known, order);
        // Objects of other features keep an order of their own.
        for id in selected.into_iter().filter(|id| engine_ref(*id).is_none()) {
            features::all()[id.owner].reorder_object(self, id.id, mv);
        }
        self.commit_object(lead, ObjectVerb::Reorder, cx);
    }

    /// Whether an Align command has something to do: aligning takes two
    /// objects, distributing three.
    pub fn can_align(&self, how: Align) -> bool {
        let needed = match how {
            Align::DistributeHorizontally | Align::DistributeVertically => 3,
            _ => 2,
        };
        self.selected_objects().len() >= needed
    }

    /// Line the selected objects up, or space them out evenly.
    pub fn align_objects(&mut self, how: Align, cx: &mut Context<Self>) {
        if !self.can_align(how) || !self.may_edit_selected_objects(cx) {
            return;
        }
        let Some(layout) = self.current_layout() else {
            return;
        };
        let infos = self.selected_object_infos();
        let boxes: Vec<Box4> = infos.iter().map(|(_, object)| self.object_box(&layout, object)).collect();
        for (((id, _), frame), (x, y)) in infos.iter().zip(&boxes).zip(aligned(&boxes, how)) {
            if (x, y) != (frame.0, frame.1) {
                self.place_object_box(&layout, *id, (x.max(0.0), y.max(0.0), frame.2, frame.3));
            }
        }
        self.commit(t("Align Objects", "Aligner les objets"), cx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligning_and_distributing() {
        let frames = [(10.0, 10.0, 20.0, 10.0), (50.0, 40.0, 40.0, 20.0), (200.0, 100.0, 10.0, 50.0)];
        let xs = |how| aligned(&frames, how).iter().map(|p| p.0).collect::<Vec<_>>();
        let ys = |how| aligned(&frames, how).iter().map(|p| p.1).collect::<Vec<_>>();
        assert_eq!(xs(Align::Left), [10.0, 10.0, 10.0]);
        assert_eq!(xs(Align::Right), [190.0, 170.0, 200.0]);
        // The middle of the box around them all: 10 to 210.
        assert_eq!(xs(Align::Center), [100.0, 90.0, 105.0]);
        assert_eq!(ys(Align::Top), [10.0, 10.0, 10.0]);
        assert_eq!(ys(Align::Bottom), [140.0, 130.0, 100.0]);
        assert_eq!(ys(Align::Middle), [75.0, 70.0, 55.0]);
        // Aligning along one axis leaves the other alone.
        assert_eq!(ys(Align::Left), [10.0, 40.0, 100.0]);
        // 200 wide, 70 taken: two gaps of 65.
        assert_eq!(xs(Align::DistributeHorizontally), [10.0, 95.0, 200.0]);
        // 140 high, 80 taken: two gaps of 30.
        assert_eq!(ys(Align::DistributeVertically), [10.0, 50.0, 100.0]);
        // Two objects have nothing between them to space out.
        assert_eq!(aligned(&frames[..2], Align::DistributeHorizontally), [(10.0, 10.0), (50.0, 40.0)]);
        // Given in another order, each still gets its own place.
        let shuffled = [frames[2], frames[0], frames[1]];
        let got: Vec<f64> = aligned(&shuffled, Align::DistributeHorizontally).iter().map(|p| p.0).collect();
        assert_eq!(got, [200.0, 10.0, 95.0]);
    }

    #[test]
    fn objects_resized_together_keep_their_proportions() {
        let frame = (100.0, 100.0, 80.0, 40.0);
        // From the bottom-right corner the top-left one stays.
        assert_eq!(scaled(frame, 4, 1.5, 2.0, (8.0, 8.0)), (100.0, 100.0, 120.0, 80.0));
        // From the top-left corner the bottom-right one does.
        assert_eq!(scaled(frame, 0, 0.5, 0.5, (8.0, 8.0)), (140.0, 120.0, 40.0, 20.0));
        // A side: one direction only.
        assert_eq!(scaled(frame, 7, 2.0, 1.0, (8.0, 8.0)), (20.0, 100.0, 160.0, 40.0));
        assert_eq!(scaled(frame, 1, 1.0, 0.5, (8.0, 8.0)), (100.0, 120.0, 80.0, 20.0));
        // Never smaller than the object allows.
        assert_eq!(scaled(frame, 4, 0.01, 0.01, (8.0, 8.0)), (100.0, 100.0, 8.0, 8.0));
    }
}
