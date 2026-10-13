//! Working on drawings with the mouse: drawing a new shape by dragging on
//! the sheet, the handles that resize and turn the selected drawing, and
//! bringing pictures in from files, a drop or the clipboard.

use super::geometry::{self, Placed, Pt};
use super::paint::{self, Grip, Pane};
use super::{Drawings, DrawingUi, PICTURE_TAB, SHAPE_TAB, TextStart, begin_text_edit, end_text_edit, picture, selected};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent, Painter, Zone, ZoneEvent, ZoneResponse, object_id};
use crate::grid::Draw;
use crate::i18n::{t, t_ctx};
use crate::shell::Alert;
use gpui::{CursorStyle, ExternalPaths, Modifiers, MouseButton, PathPromptOptions, Window};
use opensheet_engine::drawing::{Drawing, DrawingKind, Frame, Picture, PictureAdjust, ShapeTool};
use std::path::PathBuf;

/// The zone that covers the cells while a shape waits to be drawn.
pub(super) struct Canvas;

/// The zone of a handle of the selected drawing.
pub(super) struct Handle {
    pub id: u32,
    pub grip: Grip,
}

/// What the mouse is dragging.
pub(super) enum Drag {
    /// A new shape, from where the button went down to where the pointer
    /// is (points of the sheet).
    Draw {
        tool: ShapeTool,
        pane: Pane,
        start: (f64, f64),
        end: (f64, f64),
        /// Where the press was on the grid element: a press that does not
        /// move is a click.
        pressed: Pt,
        moved: bool,
        /// Shift at the last move: a square, or a line at 45 degrees.
        constrain: bool,
    },
    /// A handle of a drawing, with what the drawing was when the button
    /// went down.
    Grip {
        id: u32,
        grip: Grip,
        pane: Pane,
        pressed: (f64, f64),
        frame: Frame,
        rotation: f32,
        flips: (bool, bool),
        lock_aspect: bool,
        changed: bool,
        /// The other selected drawings and how they were turned: they
        /// turn with this one.
        others: Vec<(u32, f32)>,
    },
    /// A crop mark of a picture, or its image under the frame, with what
    /// the picture was when the button went down.
    Crop {
        id: u32,
        grip: Grip,
        pane: Pane,
        pressed: (f64, f64),
        frame: Frame,
        rotation: f32,
        flips: (bool, bool),
        adjust: PictureAdjust,
        changed: bool,
    },
}

/// A crop as the screen shows it: the sides of a flipped picture are
/// those of its image the other way round. (Doing it twice undoes it.)
pub(super) fn seen_crop(crop: [f32; 4], flips: (bool, bool)) -> [f32; 4] {
    let [mut l, mut t, mut r, mut b] = crop;
    if flips.0 {
        (l, r) = (r, l);
    }
    if flips.1 {
        (t, b) = (b, t);
    }
    [l, t, r, b]
}

/// What dragging a crop mark (or the image) by `delta` makes of a
/// picture that was in `frame` with the crop `adjust`: its new frame and
/// its new crop. The image stays where it is on the sheet under a mark;
/// the frame stays where it is when the image is moved.
pub(super) fn cropped(
    grip: Grip,
    frame: Frame,
    rotation: f32,
    flips: (bool, bool),
    adjust: &PictureAdjust,
    delta: (f64, f64),
    keep_aspect: bool,
) -> (Frame, [f32; 4]) {
    // The drag along the sides of the picture, and the picture in a box
    // of its own with its corner at the origin.
    let (sin, cos) = (-(rotation as f64)).to_radians().sin_cos();
    let local = (delta.0 * cos - delta.1 * sin, delta.0 * sin + delta.1 * cos);
    let own = Frame::new(0.0, 0.0, frame.width, frame.height);
    let seen = PictureAdjust {
        crop: seen_crop(adjust.crop, flips),
        ..*adjust
    };
    let image = seen.uncropped(own);
    let (placed, crop) = match grip {
        Grip::Crop(handle) => {
            let kept = geometry::resize(own, 0.0, handle, local, keep_aspect, false);
            (
                geometry::resize(frame, rotation, handle, delta, keep_aspect, false),
                PictureAdjust::crop_between(image, kept),
            )
        }
        _ => {
            let moved = Frame {
                x: image.x + local.0,
                y: image.y + local.1,
                ..image
            };
            (frame, PictureAdjust::crop_between(moved, own))
        }
    };
    (placed, seen_crop(crop, flips))
}

/// True where a click selects a drawing: on a picture anywhere in its
/// (turned) frame, on a shape on its outline, and inside it when it is
/// filled or holds text. `p` is a point of the grid element, `placed`
/// where the drawing is in the pane under it.
pub(super) fn hits(drawing: &Drawing, placed: &Placed, natural: (f32, f32), zoom: f32, p: Pt) -> bool {
    match &drawing.kind {
        DrawingKind::Picture(_) => placed.contains(p),
        DrawingKind::Group(_) => paint::leaves(drawing, placed, zoom)
            .iter()
            .any(|(member, inner, natural)| hits(member, inner, *natural, zoom, p)),
        DrawingKind::Shape(shape) => {
            let lines = paint::outline_of(shape, placed, natural);
            let is_line = shape.preset.is_line();
            let reach = shape.outline.as_ref().map_or(0.0, |outline| paint::stroke_width(outline, zoom) / 2.0)
                + if is_line { 5.0 } else { 3.0 };
            if lines.iter().any(|line| geometry::distance_to(p, line) <= reach) {
                return true;
            }
            let solid = shape.fill.is_some() || !shape.text.is_empty();
            !is_line && solid && lines.iter().any(|line| line.closed && geometry::in_polygon(p, &line.points))
        }
    }
}

/// Take a shape of the gallery in hand: the next press on the sheet draws
/// it.
pub(super) fn arm(app: &mut ExcelApp, tool: ShapeTool, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    end_text_edit(app, cx);
    app.popup = None;
    let ui = app.ext_mut::<DrawingUi>();
    ui.tool = Some(tool);
    ui.drag = None;
    window.focus(&app.grid_focus);
    app.set_status(
        if tool == ShapeTool::TextBox {
            t(
                "Click and drag to insert a text box.",
                "Cliquez et faites glisser pour insérer une zone de texte.",
            )
        } else {
            t(
                "Click and drag to insert an AutoShape.",
                "Cliquez et faites glisser pour insérer une forme automatique.",
            )
        },
        cx,
    );
}

/// Put the tool down (Escape, or a click with another button). False when
/// no shape was waiting.
pub(super) fn disarm(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let Some(ui) = app.ext::<DrawingUi>() else {
        return false;
    };
    if ui.tool.is_none() {
        return false;
    }
    let ui = app.ext_mut::<DrawingUi>();
    ui.tool = None;
    ui.drag = None;
    app.status_message = None;
    cx.notify();
    true
}

/// The frame and the flips of the shape a drag draws.
fn drawn(tool: ShapeTool, start: (f64, f64), end: (f64, f64), constrain: bool) -> (Frame, bool, bool) {
    if tool.preset().is_line() {
        let end = if constrain { geometry::constrain_direction(start, end, 45.0) } else { end };
        geometry::line_frame(start, end)
    } else {
        (geometry::drawn_frame(start, end, constrain), false, false)
    }
}

/// While a shape waits to be drawn: the crosshair over the cells, and the
/// shape as it is dragged out.
pub(super) fn paint_tool(p: &mut Painter) {
    let Some(ui) = p.app.ext::<DrawingUi>() else {
        return;
    };
    if ui.tool.is_none() {
        return;
    }
    let layout = p.layout;
    let cells = (layout.header_w, layout.header_h, layout.width(), layout.height());
    if let Some(Drag::Draw {
        tool,
        pane,
        start,
        end,
        moved: true,
        constrain,
        ..
    }) = &ui.drag
    {
        let sheet = &p.app.wb.sheets[p.app.sheet];
        let (frame, flip_h, flip_v) = drawn(*tool, *start, *end, *constrain);
        let placed = paint::placed_frame(layout, sheet, frame, (0.0, flip_h, flip_v), pane);
        let clip = layout.bounds_px(pane.clip.0, pane.clip.1, pane.clip.2, pane.clip.3);
        let brush = paint::Brush {
            origin: layout.bounds.origin,
            zoom: layout.zoom,
            scale: p.scale,
            ts: p.ts,
            clip,
        };
        let preview = Drawing::shape(tool.shape(), (frame.width as f32, frame.height as f32));
        let mut items = Vec::new();
        paint::draw(&preview, &placed, (frame.width as f32, frame.height as f32), &brush, None, &mut items);
        p.push(Draw::Clipped { clip, items });
    }
    p.zone(cells, Canvas).cursor(CursorStyle::Crosshair);
}

pub(super) fn mouse_down(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) -> ZoneResponse {
    if zone.payload::<Canvas>().is_some() {
        let tool = app.ext::<DrawingUi>().and_then(|ui| ui.tool);
        let (Some(tool), MouseButton::Left) = (tool, ev.button) else {
            disarm(app, cx);
            return ZoneResponse::Handled;
        };
        let Some(layout) = app.current_layout() else {
            return ZoneResponse::Handled;
        };
        let Some(pane) = paint::pane_at(&layout, ev.x, ev.y) else {
            return ZoneResponse::Handled;
        };
        let start = paint::sheet_point(&layout, &app.wb.sheets[app.sheet], &pane, ev.x, ev.y);
        app.ext_mut::<DrawingUi>().drag = Some(Drag::Draw {
            tool,
            pane,
            start,
            end: start,
            pressed: (ev.x, ev.y),
            moved: false,
            constrain: false,
        });
        return ZoneResponse::Capture;
    }
    let Some(handle) = zone.payload::<Handle>() else {
        return ZoneResponse::Ignored;
    };
    if ev.button != MouseButton::Left {
        // A right click on a handle is a right click on its drawing.
        return ZoneResponse::Ignored;
    }
    // The handle of one of several selected objects works on them all.
    if !app.may_edit_selected_objects(cx) {
        return ZoneResponse::Handled;
    }
    end_text_edit(app, cx);
    let Some(layout) = app.current_layout() else {
        return ZoneResponse::Handled;
    };
    app.remember_object_boxes(&layout);
    let others: Vec<(u32, f32)> = super::selected_ids(app)
        .into_iter()
        .filter(|id| *id != handle.id)
        .filter_map(|id| app.wb.drawing(app.sheet, id).map(|drawing| (id, drawing.rotation)))
        .collect();
    let Some(drawing) = app.wb.drawing(app.sheet, handle.id) else {
        return ZoneResponse::Handled;
    };
    let sheet = &app.wb.sheets[app.sheet];
    let pane = paint::pane_at(&layout, ev.x, ev.y).unwrap_or_else(|| paint::panes(&layout)[0]);
    if let (Grip::Crop(_) | Grip::Pan, Some(picture)) = (handle.grip, drawing.as_picture()) {
        let drag = Drag::Crop {
            id: handle.id,
            grip: handle.grip,
            pane,
            pressed: paint::sheet_point(&layout, sheet, &pane, ev.x, ev.y),
            frame: sheet.drawing_frame(drawing),
            rotation: drawing.rotation,
            flips: (drawing.flip_h, drawing.flip_v),
            adjust: picture.adjust,
            changed: false,
        };
        app.ext_mut::<DrawingUi>().drag = Some(drag);
        return ZoneResponse::Capture;
    }
    let drag = Drag::Grip {
        id: handle.id,
        grip: handle.grip,
        pane,
        pressed: paint::sheet_point(&layout, sheet, &pane, ev.x, ev.y),
        frame: sheet.drawing_frame(drawing),
        rotation: drawing.rotation,
        flips: (drawing.flip_h, drawing.flip_v),
        lock_aspect: drawing.lock_aspect,
        changed: false,
        others,
    };
    app.ext_mut::<DrawingUi>().drag = Some(drag);
    ZoneResponse::Capture
}

pub(super) fn mouse_drag(app: &mut ExcelApp, _zone: &Zone, ev: &ZoneEvent, _cx: &mut Cx) {
    let Some(layout) = app.current_layout() else {
        return;
    };
    let sheet_index = app.sheet;
    let Some(drag) = app.ext_mut::<DrawingUi>().drag.take() else {
        return;
    };
    let drag = match drag {
        Drag::Draw {
            tool,
            pane,
            start,
            pressed,
            moved,
            ..
        } => Drag::Draw {
            tool,
            pane,
            start,
            end: paint::sheet_point(&layout, &app.wb.sheets[sheet_index], &pane, ev.x, ev.y),
            pressed,
            moved: moved || (ev.x - pressed.0).hypot(ev.y - pressed.1) > 3.0,
            constrain: ev.modifiers.shift,
        },
        Drag::Grip {
            id,
            grip,
            pane,
            pressed,
            frame,
            rotation,
            flips,
            lock_aspect,
            others,
            ..
        } => {
            let now = paint::sheet_point(&layout, &app.wb.sheets[sheet_index], &pane, ev.x, ev.y);
            drag_grip(app, id, grip, &layout, &pane, (pressed, now), (frame, rotation, flips, lock_aspect), &others, ev);
            Drag::Grip {
                id,
                grip,
                pane,
                pressed,
                frame,
                rotation,
                flips,
                lock_aspect,
                changed: true,
                others,
            }
        }
        Drag::Crop {
            id,
            grip,
            pane,
            pressed,
            frame,
            rotation,
            flips,
            adjust,
            ..
        } => {
            let now = paint::sheet_point(&layout, &app.wb.sheets[sheet_index], &pane, ev.x, ev.y);
            let delta = (now.0 - pressed.0, now.1 - pressed.1);
            let (placed, crop) = cropped(grip, frame, rotation, flips, &adjust, delta, ev.modifiers.shift);
            app.wb.update_drawing(sheet_index, id, |drawing| {
                drawing.anchor = (0, 0);
                drawing.offset = (placed.x.max(0.0) as f32, placed.y.max(0.0) as f32);
                drawing.size = (placed.width as f32, placed.height as f32);
                if let Some(picture) = drawing.as_picture_mut() {
                    picture.adjust.crop = crop;
                }
            });
            Drag::Crop {
                id,
                grip,
                pane,
                pressed,
                frame,
                rotation,
                flips,
                adjust,
                changed: true,
            }
        }
    };
    app.ext_mut::<DrawingUi>().drag = Some(drag);
}

/// Apply the drag of a handle: `points` are where the press was and where
/// the pointer is on the sheet, `was` the frame, the rotation, the flips
/// and the aspect lock of the drawing when the button went down. The
/// other selected objects take the same proportions, and `others` (the
/// other selected drawings) the same turn.
#[allow(clippy::too_many_arguments)]
fn drag_grip(
    app: &mut ExcelApp,
    id: u32,
    grip: Grip,
    layout: &crate::grid::GridLayout,
    pane: &Pane,
    points: ((f64, f64), (f64, f64)),
    was: (Frame, f32, (bool, bool), bool),
    others: &[(u32, f32)],
    ev: &ZoneEvent,
) {
    let sheet = app.sheet;
    let (pressed, now) = points;
    let (frame, rotation, flips, lock_aspect) = was;
    let Modifiers { shift, alt, control, .. } = ev.modifiers;
    match grip {
        Grip::Resize(handle) => {
            let delta = (now.0 - pressed.0, now.1 - pressed.1);
            // A locked picture keeps its proportions from the corners;
            // Shift asks the same of any drawing.
            let resized = geometry::resize(frame, rotation, handle, delta, lock_aspect || shift, alt || control);
            app.wb.set_drawing_frame(sheet, id, resized);
            if frame.width > 0.0 && frame.height > 0.0 {
                let (sx, sy) = (resized.width / frame.width, resized.height / frame.height);
                app.scale_other_objects(layout, object_id(&Drawings, id), handle, sx, sy);
            }
        }
        Grip::LineEnd(end) => {
            let (start_point, end_point) = geometry::line_ends(frame, flips.0, flips.1, rotation);
            let fixed = if end { start_point } else { end_point };
            let moved = if shift { geometry::constrain_direction(fixed, now, 45.0) } else { now };
            let (frame, flip_h, flip_v) = if end {
                geometry::line_frame(fixed, moved)
            } else {
                geometry::line_frame(moved, fixed)
            };
            app.wb.update_drawing(sheet, id, |drawing| {
                // The two points say everything: a rotation the line had
                // is folded into them.
                drawing.anchor = (0, 0);
                drawing.offset = (frame.x.max(0.0) as f32, frame.y.max(0.0) as f32);
                drawing.size = (frame.width as f32, frame.height as f32);
                drawing.flip_h = flip_h;
                drawing.flip_v = flip_v;
                drawing.rotation = 0.0;
            });
        }
        // The marks of a picture being cropped have a drag of their own.
        Grip::Crop(_) | Grip::Pan => {}
        Grip::Rotate => {
            let placed = paint::placed_frame(layout, &app.wb.sheets[sheet], frame, (rotation, flips.0, flips.1), pane);
            let angle = geometry::drag_angle(placed.centre(), (ev.x, ev.y), shift);
            app.wb.update_drawing(sheet, id, |drawing| drawing.rotation = angle);
            for (other, was) in others {
                let turned = geometry::normalise_angle(was + angle - rotation);
                app.wb.update_drawing(sheet, *other, |drawing| drawing.rotation = turned);
            }
        }
    }
}

pub(super) fn mouse_up(app: &mut ExcelApp, _zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
    let Some(drag) = app.ext_mut::<DrawingUi>().drag.take() else {
        return;
    };
    match drag {
        Drag::Draw {
            tool,
            start,
            end,
            moved,
            ..
        } => {
            app.ext_mut::<DrawingUi>().tool = None;
            app.status_message = None;
            // A click inserts the shape at the size Excel gives it.
            let (frame, flip_h, flip_v) = if moved {
                drawn(tool, start, end, ev.modifiers.shift)
            } else {
                let (width, height) = tool.default_size();
                (Frame::new(start.0, start.1, width as f64, height as f64), false, false)
            };
            insert_shape(app, tool, frame, (flip_h, flip_v), window, cx);
        }
        Drag::Grip { grip, changed, .. } => {
            if changed {
                let label = match grip {
                    Grip::Rotate => t("Rotate Object", "Faire pivoter l'objet"),
                    _ => t("Size Object", "Dimensionner l'objet"),
                };
                app.commit(label, cx);
            }
        }
        Drag::Crop { changed, .. } => {
            if changed {
                app.commit(t("Crop Picture", "Rogner l'image"), cx);
            }
        }
    }
}

/// Add a shape of the gallery in a frame, select it and show its tab; a
/// text box is ready to be typed in.
pub(super) fn insert_shape(app: &mut ExcelApp, tool: ShapeTool, frame: Frame, flips: (bool, bool), window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(id) = app.wb.add_shape(sheet, tool, frame) else {
        return;
    };
    if flips != (false, false) {
        app.wb.update_drawing(sheet, id, |drawing| (drawing.flip_h, drawing.flip_v) = flips);
    }
    app.commit(
        if tool == ShapeTool::TextBox {
            t("Insert Text Box", "Insérer une zone de texte")
        } else {
            t("Insert Shape", "Insérer une forme")
        },
        cx,
    );
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(SHAPE_TAB);
    if tool == ShapeTool::TextBox {
        begin_text_edit(app, id, TextStart::End, window, cx);
    }
}

/// Add pictures (the bytes of image files a workbook can hold) to the
/// current sheet as one undo step: at a point of the sheet, or on the
/// active cell, each a little lower and further right than the one
/// before. The last one is selected.
pub(super) fn insert_pictures(app: &mut ExcelApp, files: Vec<Vec<u8>>, at: Option<(f64, f64)>, cx: &mut Cx) {
    if files.is_empty() || !app.may(Intent::EditObject(None), cx) {
        return;
    }
    let sheet = app.sheet;
    let (x, y) = at.unwrap_or_else(|| app.wb.sheets[sheet].corner_point((app.active(), (0.0, 0.0))));
    let mut last = None;
    for (i, bytes) in files.into_iter().enumerate() {
        let Some(picture) = Picture::new(bytes) else {
            continue;
        };
        let mut drawing = Drawing::picture(picture);
        // From the corner of the sheet: the engine finds the cell.
        let step = 16.0 * i as f64;
        drawing.offset = ((x + step) as f32, (y + step) as f32);
        last = app.wb.add_drawing(sheet, drawing).or(last);
    }
    let Some(id) = last else {
        return;
    };
    app.commit(t("Insert Picture", "Insérer une image"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(PICTURE_TAB);
}

/// Read image files; those that are not pictures are named in a message.
fn read_pictures(app: &mut ExcelApp, paths: &[PathBuf], cx: &mut Cx) -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    let mut refused = Vec::new();
    for path in paths {
        match std::fs::read(path).ok().and_then(picture::importable) {
            Some(bytes) => files.push(bytes),
            None => refused.push(path.display().to_string()),
        }
    }
    if !refused.is_empty() {
        let message = format!(
            "{}\n\n{}",
            t(
                "An error occurred while importing this file.",
                "Une erreur s'est produite lors de l'importation de ce fichier.",
            ),
            refused.join("\n")
        );
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
    }
    files
}

/// "Picture from File…": the system file dialog, then the pictures on the
/// active cell. With `replace`, the file chosen takes the place of the
/// selected picture, in its frame.
pub(super) fn pick_pictures(app: &mut ExcelApp, replace: bool, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    let intent = match (replace, selected(app)) {
        (true, Some(drawing)) => Intent::EditObject(Some(object_id(&Drawings, drawing.id))),
        (true, None) => return,
        (false, _) => Intent::EditObject(None),
    };
    if !app.may(intent, cx) {
        return;
    }
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: !replace,
        prompt: Some(t_ctx("button", "Insert", "Insérer").into()),
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = paths.await {
            this.update(cx, |app, cx| {
                let files = read_pictures(app, &paths, cx);
                if replace {
                    replace_picture(app, files, cx);
                } else {
                    insert_pictures(app, files, None, cx);
                }
            })
            .ok();
        }
    })
    .detach();
}

/// "Change Picture": another image in the frame of the selected picture.
fn replace_picture(app: &mut ExcelApp, files: Vec<Vec<u8>>, cx: &mut Cx) {
    let Some(picture) = files.into_iter().next().and_then(Picture::new) else {
        return;
    };
    super::edit(app, t("Change Picture", "Modifier l'image"), cx, |drawing| {
        if matches!(drawing.kind, DrawingKind::Picture(_)) {
            drawing.kind = DrawingKind::Picture(picture.clone());
        }
    });
}

/// Image files dropped on the cells are inserted where they were dropped;
/// other files are not for this feature.
pub(super) fn drop_files(app: &mut ExcelApp, paths: &ExternalPaths, window: &mut Window, cx: &mut Cx) {
    if app.modal_open() {
        return;
    }
    let pictures: Vec<PathBuf> = paths
        .paths()
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["png", "jpg", "jpeg", "jpe", "gif", "bmp", "tif", "tiff", "webp"].contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .cloned()
        .collect();
    if pictures.is_empty() {
        return;
    }
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    let at = app.current_layout().and_then(|layout| {
        let (x, y) = layout.local(window.mouse_position());
        let pane = paint::pane_at(&layout, x, y)?;
        Some(paint::sheet_point(&layout, &app.wb.sheets[app.sheet], &pane, x, y))
    });
    let files = read_pictures(app, &pictures, cx);
    insert_pictures(app, files, at, cx);
    window.focus(&app.grid_focus);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::ShapePreset;

    fn shape(tool: ShapeTool) -> Drawing {
        Drawing::shape(tool.shape(), (100.0, 100.0))
    }

    #[test]
    fn clicks_select_what_is_drawn() {
        let placed = Placed::plain(0.0, 0.0, 100.0, 100.0);
        let natural = (100.0, 100.0);
        let hit = |drawing: &Drawing, placed: &Placed, p: Pt| hits(drawing, placed, natural, 1.0, p);
        // A filled ellipse: inside, not in the corners of its frame.
        let ellipse = shape(ShapeTool::Ellipse);
        assert!(hit(&ellipse, &placed, (50.0, 50.0)));
        assert!(!hit(&ellipse, &placed, (4.0, 4.0)));
        // Without fill only its outline (and its text) can be clicked.
        let mut hollow = shape(ShapeTool::Ellipse);
        hollow.as_shape_mut().unwrap().fill = None;
        assert!(!hit(&hollow, &placed, (50.0, 50.0)));
        assert!(hit(&hollow, &placed, (50.0, 1.0)));
        hollow.as_shape_mut().unwrap().text = "Text".into();
        assert!(hit(&hollow, &placed, (50.0, 50.0)));
        // A line: along its length, not in the rest of its frame.
        let line = shape(ShapeTool::Line);
        assert_eq!(line.as_shape().unwrap().preset, ShapePreset::Line);
        assert!(hit(&line, &placed, (50.0, 52.0)));
        assert!(!hit(&line, &placed, (80.0, 20.0)));
        let flipped = Placed { flip_h: true, ..placed };
        assert!(hit(&line, &flipped, (80.0, 20.0)));
        // A turned rectangle is hit where it was turned to.
        let wide = Placed {
            rotation: 90.0,
            ..Placed::plain(0.0, 40.0, 100.0, 60.0)
        };
        let rectangle = shape(ShapeTool::Rectangle);
        let hit_wide = |p: Pt| hits(&rectangle, &wide, (100.0, 20.0), 1.0, p);
        assert!(hit_wide((50.0, 5.0)));
        assert!(!hit_wide((5.0, 50.0)));
    }

    #[test]
    fn dragging_draws_frames_and_lines() {
        // A shape: any corner first; Shift gives a square.
        assert_eq!(
            drawn(ShapeTool::Ellipse, (100.0, 100.0), (40.0, 130.0), false),
            (Frame::new(40.0, 100.0, 60.0, 30.0), false, false)
        );
        assert_eq!(
            drawn(ShapeTool::Ellipse, (100.0, 100.0), (40.0, 130.0), true).0,
            Frame::new(40.0, 100.0, 60.0, 60.0)
        );
        // A line remembers which way it was drawn.
        assert_eq!(
            drawn(ShapeTool::Arrow, (100.0, 100.0), (40.0, 130.0), false),
            (Frame::new(40.0, 100.0, 60.0, 30.0), true, false)
        );
        // Shift: level when it is nearly level.
        let (frame, flip_h, flip_v) = drawn(ShapeTool::Line, (0.0, 50.0), (200.0, 58.0), true);
        assert!(frame.height.abs() < 1e-6 && (frame.width - 200.0).abs() < 1e-6);
        assert_eq!((flip_h, flip_v), (false, false));
    }

    #[test]
    fn crop_marks_move_the_frame_over_the_image() {
        let plain = PictureAdjust::default();
        let frame = Frame::new(100.0, 100.0, 200.0, 100.0);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        // The right side pulled in by 50: a quarter of the image goes.
        let (placed, crop) = cropped(Grip::Crop(3), frame, 0.0, (false, false), &plain, (-50.0, 0.0), false);
        assert_eq!(placed, Frame::new(100.0, 100.0, 150.0, 100.0));
        assert!(near(crop, [0.0, 0.0, 0.25, 0.0]), "{crop:?}");
        // Then the top-left corner in by (30, 20) from there.
        let once = PictureAdjust { crop, ..plain };
        let (placed, crop) = cropped(Grip::Crop(0), placed, 0.0, (false, false), &once, (30.0, 20.0), false);
        assert_eq!(placed, Frame::new(130.0, 120.0, 120.0, 80.0));
        assert!(near(crop, [0.15, 0.2, 0.25, 0.0]), "{crop:?}");
        // Moving the image right by 20 shows more of its left.
        let twice = PictureAdjust { crop, ..plain };
        let (still, crop) = cropped(Grip::Pan, placed, 0.0, (false, false), &twice, (20.0, 0.0), false);
        assert_eq!(still, placed);
        assert!(near(crop, [0.05, 0.2, 0.35, 0.0]), "{crop:?}");
        // A flipped picture: the side dragged on screen is the other
        // side of the image.
        let (_, crop) = cropped(Grip::Crop(3), frame, 0.0, (true, false), &plain, (-50.0, 0.0), false);
        assert!(near(crop, [0.25, 0.0, 0.0, 0.0]), "{crop:?}");
        // Turned a quarter, its right side is at the bottom of the
        // screen: dragging up there cuts the right of the image.
        let (_, crop) = cropped(Grip::Crop(3), frame, 90.0, (false, false), &plain, (0.0, -50.0), false);
        assert!(near(crop, [0.0, 0.0, 0.25, 0.0]), "{crop:?}");
        // Pulled out past the image: room around it.
        let (_, crop) = cropped(Grip::Crop(7), frame, 0.0, (false, false), &plain, (-20.0, 0.0), false);
        assert!(near(crop, [-0.1, 0.0, 0.0, 0.0]), "{crop:?}");
    }
}
