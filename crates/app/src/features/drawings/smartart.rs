//! SmartArt on the sheet: a graphic is laid out by the engine as a list of
//! shapes (`SmartArt::pieces`), which are drawn here like any shape.
//!
//! An entry of the outline is typed in two places that show the same
//! thing: its line of the Text Pane, and its shape on the sheet (a click
//! on a shape of the selected graphic chooses its entry, a second click
//! or a key types in it). Both write to the graphic as they are typed
//! in, so each follows the other and the graphic is laid out again at
//! every key.
//!
//! The "Format" tab works on the shapes that are chosen in the graphic
//! (Shift or ⌘ and a click choose several; all of them when none is):
//! its commands are those of Shape Format, which are handed a stand-in
//! for each shape and whose changes are kept with the entry of the shape
//! (`format_nodes`).

use super::geometry::{self, Placed};
use super::paint::{self, Brush};
use super::{Cmd, Drawings, SMARTART_TAB, TextStart, edit_shape, selected, tabs};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, Cx, Intent, Painter, Zone, ZoneEvent, ZoneResponse, object_id};
use crate::grid::{Draw, FontResolver, Layer};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, row, rows};
use crate::shell::Modal;
use crate::text_input::{InputEvent, NavKey, TextInput};
use crate::theme::{self, c, ca};
use crate::view::tooltip;
use crate::widgets::text_area::{AreaEvent, OutlineKey, TextArea};
use crate::widgets::{self, ListBox};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EntityInputHandler, Focusable, MouseButton, Pixels, SharedString, Subscription, WeakEntity, Window,
    canvas, div, prelude::*, px,
};
use opensheet_engine::Locale;
use opensheet_engine::drawing::{Drawing, ShapeTool};
use opensheet_engine::wordart::TextEffects;
use opensheet_engine::smartart::{Category, ColorScheme, DEFAULT_SIZE, Layout, Line, Piece, QuickStyle, SmartArt, placeholder};
use opensheet_engine::theme::Theme;
use std::sync::Arc;

/// Where a piece of a graphic is drawn, the graphic being drawn in
/// `placed`: `natural` is the size the pieces were laid out for.
pub(super) fn placed_piece(piece: &Piece, placed: &Placed, natural: (f32, f32)) -> Placed {
    let (sx, sy) = (placed.width() / natural.0.max(1.0), placed.height() / natural.1.max(1.0));
    Placed {
        x0: placed.x0 + piece.x * sx,
        y0: placed.y0 + piece.y * sy,
        x1: placed.x0 + (piece.x + piece.width) * sx,
        y1: placed.y0 + (piece.y + piece.height) * sy,
        rotation: piece.rotation,
        flip_h: piece.flip_h,
        flip_v: false,
    }
}

/// The items that draw a graphic in a frame. A graphic is never turned:
/// its frame stands as its cells do. The shapes come first, then what is
/// written in them, so that shapes that overlap hide no text.
/// The text of the shape that shows entry `typed` is left out: the field
/// it is being typed in lies over it.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(art: &SmartArt, theme: &Theme, placed: &Placed, natural: (f32, f32), brush: &Brush, fonts: Option<&FontResolver>, locale: &Locale, typed: Option<usize>, out: &mut Vec<Draw>) {
    let pieces = art.pieces(theme, natural.0, natural.1);
    for piece in &pieces {
        let at = placed_piece(piece, placed, natural);
        let size = (piece.width, piece.height);
        let mut shape = piece.shape.clone();
        shape.text.clear();
        // The shadow and the glow the Format tab gave the shape lie
        // under it.
        if !piece.effects.is_none() {
            let mut stand_in = Drawing::shape(shape.clone(), size);
            stand_in.effects = piece.effects;
            paint::draw_effects(&stand_in, &shape, &at, size, brush, out);
        }
        // A fill that lets what is under it show is painted here.
        if let (Some(fill), true) = (shape.fill, piece.opacity < 1.0) {
            for line in geometry::flatten(&shape.path(size.0, size.1), &at).iter().filter(|line| line.closed) {
                brush.fill(&line.points, ca(fill.value(), piece.opacity.max(0.0)), out);
            }
            shape.fill = None;
        }
        paint::draw_shape(&shape, &at, size, brush, None, out);
    }
    let Some(fonts) = fonts else {
        return;
    };
    let typed = typed.and_then(|line| piece_of(&pieces, line)).and_then(|piece| piece.node);
    for piece in pieces.iter().filter(|piece| piece.node.is_some() && piece.node != typed) {
        let at = placed_piece(piece, placed, natural);
        let mut shape = piece.shape.clone();
        (shape.fill, shape.outline) = (None, None);
        // An entry nothing was typed in says so, on screen only.
        if shape.text.is_empty() {
            shape.text = placeholder(locale).to_string();
        }
        paint::draw_shape(&shape, &at, (piece.width, piece.height), brush, Some(fonts), out);
    }
}

// ---- what is selected ----------------------------------------------------

/// What the feature remembers of SmartArt between events.
#[derive(Default)]
pub(super) struct SmartUi {
    /// The entry the commands of the tab act on: (graphic, line of its
    /// outline). Set by a click on a shape of the selected graphic and by
    /// the line being typed in the Text Pane.
    node: Option<(u32, usize)>,
    /// The other entries chosen with it (Shift or ⌘ and a click), for
    /// the Format tab.
    also: Vec<usize>,
    /// Text was typed in the Text Pane and is not an undo step yet.
    typing: bool,
    pane: Option<Entity<TextPane>>,
    /// The entry being typed in its shape.
    edit: Option<NodeEdit>,
}

/// An entry of a graphic being typed in the shape that shows it.
struct NodeEdit {
    id: u32,
    line: usize,
    area: Entity<TextArea>,
    _subscription: Subscription,
}

/// The shape that shows a line of the outline: its own, or the one that
/// lists it under the entry it belongs to (the pieces of a graphic each
/// name the first line they show).
pub(super) fn piece_of(pieces: &[Piece], line: usize) -> Option<&Piece> {
    pieces.iter().filter(|piece| piece.node.is_some_and(|first| first <= line)).max_by_key(|piece| piece.node)
}

impl SmartUi {
    /// Choose an entry of a graphic, as a click on its shape does.
    #[cfg(test)]
    pub(super) fn set_node(&mut self, id: u32, line: usize) {
        self.node = Some((id, line));
        self.also.clear();
    }
}

const PANE: &str = "smartart-text";

/// The selected graphic.
pub(super) fn selected_art(app: &ExcelApp) -> Option<(u32, &SmartArt)> {
    let drawing = selected(app)?;
    Some((drawing.id, drawing.as_smartart()?))
}

/// True when the selected drawing is a graphic (it gets the "SmartArt
/// Design" tab).
pub(super) fn is_graphic(drawing: &Drawing) -> bool {
    drawing.as_smartart().is_some()
}

/// The line of the outline the commands act on, if one is chosen.
pub(super) fn current_line(app: &ExcelApp) -> Option<usize> {
    let (id, art) = selected_art(app)?;
    let (of, line) = app.ext::<SmartUi>()?.node?;
    (of == id && line < art.outline().len()).then_some(line)
}

/// The lines of the outline the Format tab works on: the entry that is
/// chosen and those chosen with it.
pub(super) fn chosen_lines(app: &ExcelApp) -> Vec<usize> {
    let Some(line) = current_line(app) else {
        return Vec::new();
    };
    let mut lines = vec![line];
    lines.extend(app.ext::<SmartUi>().map(|ui| ui.also.clone()).unwrap_or_default());
    lines
}

/// What was typed in the Text Pane becomes one undo step.
pub(super) fn flush(app: &mut ExcelApp, cx: &mut Cx) {
    if std::mem::take(&mut app.ext_mut::<SmartUi>().typing) {
        app.commit(t("Typing", "Saisie"), cx);
    }
}

/// Change the selected graphic as one undo step. `change` gets the line
/// the commands act on and may move it; it answers whether anything
/// changed.
pub(super) fn change(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl FnOnce(&mut SmartArt, &mut Option<usize>) -> bool) -> bool {
    let Some(id) = selected_art(app).map(|(id, _)| id) else {
        return false;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    flush(app, cx);
    let mut line = current_line(app);
    let sheet = app.sheet;
    // Tried on a copy: a command that changes nothing is no undo step.
    let Some(mut art) = app.wb.drawing(sheet, id).and_then(Drawing::as_smartart).cloned() else {
        return false;
    };
    let levels = |art: &SmartArt| art.outline().into_iter().map(|(level, _)| level).collect::<Vec<_>>();
    let before = levels(&art);
    let changed = change(&mut art, &mut line);
    // The lines are not what they were: only the entry the command
    // worked on stays chosen.
    if levels(&art) != before {
        app.ext_mut::<SmartUi>().also.clear();
    }
    if changed {
        app.wb.update_drawing(sheet, id, |drawing| {
            if let Some(stored) = drawing.as_smartart_mut() {
                *stored = art;
            }
        });
        app.commit(label, cx);
    }
    app.ext_mut::<SmartUi>().node = line.map(|line| (id, line));
    cx.notify();
    changed
}

/// Text typed on a line of the Text Pane: the graphic follows at once,
/// and the typing is one undo step when it ends (`flush`).
fn type_text(app: &mut ExcelApp, id: u32, line: usize, text: &str, cx: &mut Cx) {
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return;
    }
    let sheet = app.sheet;
    let differs = app
        .wb
        .drawing(sheet, id)
        .and_then(Drawing::as_smartart)
        .is_some_and(|art| art.outline().get(line).is_some_and(|(_, old)| old != text));
    if differs {
        app.wb.update_drawing(sheet, id, |drawing| {
            if let Some(art) = drawing.as_smartart_mut() {
                art.set_text(line, text);
            }
        });
    }
    let ui = app.ext_mut::<SmartUi>();
    ui.typing |= differs;
    ui.node = Some((id, line));
    cx.notify();
}

// ---- typing in a shape ---------------------------------------------------

/// The entry being typed in its shape: (graphic, line).
pub(super) fn editing(app: &ExcelApp) -> Option<(u32, usize)> {
    app.ext::<SmartUi>()?.edit.as_ref().map(|edit| (edit.id, edit.line))
}

/// The line of a graphic whose text the field over its shape shows.
pub(super) fn typed_line(app: &ExcelApp, id: u32) -> Option<usize> {
    editing(app).filter(|(of, _)| *of == id).map(|(_, line)| line)
}

/// Start typing an entry of a graphic in its shape, with the editor
/// text boxes use. False when the entry is not there or the sheet's
/// protection holds the graphic.
pub(super) fn begin_edit(app: &mut ExcelApp, id: u32, line: usize, start: TextStart, window: &mut Window, cx: &mut Cx) -> bool {
    let text = app
        .wb
        .drawing(app.sheet, id)
        .and_then(Drawing::as_smartart)
        .filter(|art| art.foreign.is_none())
        .and_then(|art| art.outline().get(line).map(|(_, text)| text.clone()));
    let Some(text) = text else {
        return false;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    end_edit(app, cx);
    if !app.is_object_selected(object_id(&Drawings, id)) {
        app.select_object(Some(object_id(&Drawings, id)), cx);
    }
    let area = cx.new(|cx| {
        let mut area = TextArea::new(cx);
        area.outline_keys = true;
        area.set_text(text, cx);
        area
    });
    area.update(cx, |area, cx| match start {
        TextStart::Typed(typed) => area.replace_text_in_range(None, &typed, window, cx),
        TextStart::At(position) => area.caret_at_next_paint(position),
        TextStart::All => area.select_all_text(cx),
        TextStart::End => {}
    });
    let subscription = cx.subscribe_in(&area, window, on_area);
    window.focus(&area.read(cx).focus_handle(cx));
    let ui = app.ext_mut::<SmartUi>();
    ui.node = Some((id, line));
    ui.edit = Some(NodeEdit {
        id,
        line,
        area: area.clone(),
        _subscription: subscription,
    });
    // What was typed to start with is in the graphic at once.
    let typed = area.read(cx).text().to_string();
    type_text(app, id, line, &typed, cx);
    cx.notify();
    true
}

/// Stop typing in a shape: what was typed is one undo step. False when
/// nothing was being typed.
pub(super) fn end_edit(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let ended = app.ext_mut::<SmartUi>().edit.take().is_some();
    flush(app, cx);
    if ended {
        cx.notify();
    }
    ended
}

/// The field follows the entry the commands act on after the outline
/// changed under it (a shape was added, the entry went up or down).
fn retarget(app: &mut ExcelApp, cx: &mut Cx) {
    let Some((id, _)) = editing(app) else {
        return;
    };
    let line = app.ext::<SmartUi>().and_then(|ui| ui.node).filter(|(of, _)| *of == id).map(|(_, line)| line);
    let text = line.and_then(|line| {
        let art = app.wb.drawing(app.sheet, id).and_then(Drawing::as_smartart)?;
        art.outline().get(line).map(|(_, text)| text.clone())
    });
    let (Some(line), Some(text)) = (line, text) else {
        app.ext_mut::<SmartUi>().edit = None;
        return;
    };
    if let Some(edit) = app.ext_mut::<SmartUi>().edit.as_mut() {
        edit.line = line;
        let area = edit.area.clone();
        if area.read(cx).text() != text {
            area.update(cx, |area, cx| area.set_text(text, cx));
        }
    }
    cx.notify();
}

/// What the field over a shape reports: its text as it changes, and the
/// keys that work on the outline.
fn on_area(app: &mut ExcelApp, area: &Entity<TextArea>, event: &AreaEvent, window: &mut Window, cx: &mut Cx) {
    let Some((id, line)) = editing(app) else {
        return;
    };
    match event {
        AreaEvent::Changed => {
            // An entry is one line, here as in the Text Pane.
            let mut text = area.read(cx).text().to_string();
            if text.contains(['\n', '\r']) {
                text.retain(|ch| !matches!(ch, '\n' | '\r'));
                area.update(cx, |area, cx| area.set_text(text.clone(), cx));
            }
            type_text(app, id, line, &text, cx);
        }
        // Escape leaves the text; its shape stays chosen.
        AreaEvent::Escape => {
            end_edit(app, cx);
            window.focus(&app.grid_focus);
        }
        AreaEvent::Blurred => {
            end_edit(app, cx);
        }
        // Return: a new shape after this one, which takes the caret.
        AreaEvent::Outline(OutlineKey::Enter) => {
            change(app, t("Add Shape", "Ajouter une forme"), cx, |art, at| {
                *at = art.add_after(line).or(*at);
                true
            });
            retarget(app, cx);
        }
        // Tab and Shift+Tab: one level down or up, where the entry can go.
        AreaEvent::Outline(key) => {
            let cmd = if *key == OutlineKey::ShiftTab { Cmd::SmartPromote } else { Cmd::SmartDemote };
            restructure(app, cmd, cx);
            retarget(app, cx);
        }
        _ => {}
    }
}

/// Typing with a graphic selected goes to the entry that is chosen in
/// it. True when the selected object is a graphic (the keys are its own,
/// whether an entry takes them or not).
pub(super) fn typed(app: &mut ExcelApp, id: u32, text: &str, window: &mut Window, cx: &mut Cx) -> bool {
    if app.wb.drawing(app.sheet, id).and_then(Drawing::as_smartart).is_none() {
        return false;
    }
    if let Some(line) = current_line(app) {
        begin_edit(app, id, line, TextStart::Typed(text.to_string()), window, cx);
    }
    true
}

/// Escape with an entry chosen: the graphic as a whole is selected
/// again. False when no entry was chosen.
pub(super) fn leave_node(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    if current_line(app).is_none() {
        return false;
    }
    let ui = app.ext_mut::<SmartUi>();
    ui.node = None;
    ui.also.clear();
    cx.notify();
    true
}

/// The field in which an entry is typed, over its shape.
pub(super) fn overlay(app: &ExcelApp, layout: &crate::grid::GridLayout, cx: &mut Cx) -> Option<AnyElement> {
    let edit = app.ext::<SmartUi>()?.edit.as_ref()?;
    let sheet = &app.wb.sheets[app.sheet];
    let drawing = app.wb.drawing(app.sheet, edit.id)?;
    let art = drawing.as_smartart()?;
    let frame = sheet.drawing_frame(drawing);
    let natural = (frame.width as f32, frame.height as f32);
    let pieces = art.pieces(app.wb.theme(), natural.0, natural.1);
    let piece = piece_of(&pieces, edit.line)?;
    // In the pane that shows the middle of the shape.
    paint::panes(layout).into_iter().find_map(|pane| {
        let whole = paint::placed(layout, sheet, drawing, &pane);
        let at = placed_piece(piece, &Placed::plain(whole.x0, whole.y0, whole.x1, whole.y1), natural);
        let (x, y) = at.centre();
        pane.contains(x, y).then(|| super::area_overlay(app, &edit.area, &piece.shape, &at, (piece.width, piece.height), &pane, layout.zoom, cx))
    })
}

// ---- the look of one shape -----------------------------------------------

/// Whether the shapes of the selected graphic take a look of their own
/// (those of a graphic shown as its file had it do not).
pub(super) fn can_format(app: &ExcelApp) -> bool {
    selected_art(app).is_some_and(|(_, art)| art.foreign.is_none())
}

/// Run a command of Shape Format on the chosen shapes of the selected
/// graphic (on all of them when none is chosen), as one undo step:
/// `change` is given a shape standing for each, and what it changed is
/// kept with the entry. `None` when the selection is not a graphic; else
/// whether anything changed.
pub(super) fn format_nodes(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: &dyn Fn(&mut Drawing)) -> Option<bool> {
    let (id, art) = selected_art(app)?;
    if art.foreign.is_some() {
        return Some(false);
    }
    let frame = app.wb.drawing_frame(app.sheet, id)?;
    let theme = app.wb.theme_arc();
    let chosen = chosen_lines(app);
    Some(self::change(app, label, cx, |art, _| {
        let pieces = art.pieces(&theme, frame.width as f32, frame.height as f32);
        // An entry listed in the shape of another is formatted with it.
        let mut lines: Vec<usize> = if chosen.is_empty() {
            pieces.iter().filter_map(|piece| piece.node).collect()
        } else {
            chosen.iter().filter_map(|line| piece_of(&pieces, *line)?.node).collect()
        };
        lines.sort_unstable();
        lines.dedup();
        let mut changed = false;
        for line in lines {
            let Some(piece) = pieces.iter().find(|piece| piece.node == Some(line)) else {
                continue;
            };
            let mut stand_in = Drawing::shape(piece.shape.clone(), (piece.width, piece.height));
            stand_in.effects = piece.effects;
            let effects = stand_in.effects;
            change(&mut stand_in);
            let Some(now) = stand_in.as_shape() else {
                continue;
            };
            let was = &piece.shape;
            changed |= art.set_format(line, |format| {
                if now.preset != was.preset {
                    format.preset = Some(now.preset);
                }
                if now.fill != was.fill {
                    format.fill = Some(now.fill);
                }
                if now.outline != was.outline {
                    format.outline = Some(now.outline);
                }
                if stand_in.effects != effects {
                    format.effects = Some(stand_in.effects);
                }
                if let Some(color) = now.font.color.filter(|color| Some(*color) != was.font.color) {
                    format.text_color = Some(color);
                }
                // Of what letters can wear, a graphic keeps their
                // outline and whether they are filled.
                if (now.effects.hollow, now.effects.outline) != (was.effects.hollow, was.effects.outline) {
                    format.text_effects = Some(TextEffects {
                        hollow: now.effects.hollow,
                        outline: now.effects.outline,
                        ..TextEffects::default()
                    });
                }
            });
        }
        changed
    }))
}

/// "Larger" and "Smaller": the chosen shapes, one step.
fn resize(app: &mut ExcelApp, cmd: Cmd, cx: &mut Cx) -> bool {
    let larger = cmd == Cmd::SmartLarger;
    let chosen = chosen_lines(app);
    let Some((id, _)) = selected_art(app).filter(|_| !chosen.is_empty()) else {
        return false;
    };
    let Some(frame) = app.wb.drawing_frame(app.sheet, id) else {
        return false;
    };
    let theme = app.wb.theme_arc();
    change(app, cmd.label(), cx, |art, _| {
        let pieces = art.pieces(&theme, frame.width as f32, frame.height as f32);
        let mut lines: Vec<usize> = chosen.iter().filter_map(|line| piece_of(&pieces, *line)?.node).collect();
        lines.sort_unstable();
        lines.dedup();
        lines.into_iter().fold(false, |changed, line| art.set_format(line, |format| format.resize(larger)) || changed)
    })
}

/// "Change Shape": the shapes of the gallery that have an inside.
fn open_change_shape(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let label = Cmd::SmartChangeShape.label();
    let entries = ShapeTool::ALL
        .into_iter()
        .filter(|tool| !tool.preset().is_line() && *tool != ShapeTool::TextBox)
        .map(|tool| {
            MenuEntry::new(tabs::tool_name(tool, app.wb.locale), move |app, _, cx| {
                edit_shape(app, label, cx, |shape| shape.preset = tool.preset());
            })
        })
        .collect();
    app.open_command_menu(entries, 240.0, window, cx);
}

/// The line a new shape is added next to when none is chosen: the last
/// entry of the top level.
fn anchor_line(art: &SmartArt, line: Option<usize>) -> usize {
    line.unwrap_or_else(|| art.outline().iter().rposition(|(level, _)| *level == 0).unwrap_or(0))
}

/// Where a shape is added, relative to the chosen entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    After,
    Before,
    Above,
    Below,
}

pub(super) fn add_shape(app: &mut ExcelApp, side: Side, cx: &mut Cx) -> bool {
    change(app, t("Add Shape", "Ajouter une forme"), cx, |art, line| {
        let at = anchor_line(art, *line);
        let added = match side {
            Side::After => art.add_after(at),
            Side::Before => art.add_before(at),
            Side::Above => art.add_above(at),
            Side::Below => art.add_below(at),
        };
        if added.is_some() {
            *line = added;
        }
        added.is_some()
    })
}

/// Promote, demote, move up or down the chosen entry.
pub(super) fn restructure(app: &mut ExcelApp, cmd: Cmd, cx: &mut Cx) -> bool {
    change(app, cmd.label(), cx, |art, line| {
        let Some(at) = *line else {
            return false;
        };
        match cmd {
            Cmd::SmartPromote => art.promote(at),
            Cmd::SmartDemote => art.demote(at),
            Cmd::SmartMoveUp | Cmd::SmartMoveDown => match art.move_entry(at, cmd == Cmd::SmartMoveUp) {
                Some(moved) => {
                    *line = Some(moved);
                    true
                }
                None => false,
            },
            _ => false,
        }
    })
}

/// Whether a command of the Create Graphic group can run now.
pub(super) fn can(app: &ExcelApp, cmd: Cmd) -> bool {
    let Some((_, art)) = selected_art(app) else {
        return false;
    };
    // A graphic shown through the shapes of its file takes a layout, or
    // becomes shapes; its outline is not edited before.
    if art.foreign.is_some() {
        return matches!(cmd, Cmd::SmartLayouts | Cmd::SmartConvert);
    }
    let lines = art.outline();
    let line = current_line(app);
    match cmd {
        Cmd::SmartPromote => line.is_some_and(|at| lines[at].0 > 0),
        Cmd::SmartDemote => line.is_some_and(|at| at > 0 && lines[at - 1].0 >= lines[at].0),
        Cmd::SmartMoveUp | Cmd::SmartMoveDown => line.is_some_and(|at| art.clone().move_entry(at, cmd == Cmd::SmartMoveUp).is_some()),
        // A shape is made larger or smaller by itself, not all at once.
        Cmd::SmartLarger | Cmd::SmartSmaller => line.is_some(),
        _ => true,
    }
}

// ---- inserting and converting --------------------------------------------

/// Insert a new graphic in the middle of what the window shows of the
/// sheet, and open its Text Pane.
pub(super) fn insert(app: &mut ExcelApp, layout: Layout, window: &mut Window, cx: &mut Cx) -> Option<u32> {
    if !app.may(Intent::EditObject(None), cx) {
        return None;
    }
    let sheet = app.sheet;
    let (x, y) = super::wordart::middle_of_view(app, DEFAULT_SIZE);
    let mut drawing = Drawing::smartart(SmartArt::new(layout), DEFAULT_SIZE);
    drawing.offset = (x as f32, y as f32);
    let id = app.wb.add_drawing(sheet, drawing)?;
    app.commit(t("Insert SmartArt Graphic", "Insérer un graphique SmartArt"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(SMARTART_TAB);
    app.ext_mut::<SmartUi>().node = Some((id, 0));
    open_pane(app, window, cx);
    Some(id)
}

/// "Convert to Shapes": the shapes the graphic is laid out as take its
/// place, each a shape of its own.
pub(super) fn convert(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    let Some(id) = selected_art(app).map(|(id, _)| id) else {
        return false;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    flush(app, cx);
    let sheet = app.sheet;
    let Some(frame) = app.wb.drawing_frame(sheet, id) else {
        return false;
    };
    let Some(art) = app.wb.drawing(sheet, id).and_then(Drawing::as_smartart).cloned() else {
        return false;
    };
    let pieces = art.pieces(app.wb.theme(), frame.width as f32, frame.height as f32);
    app.wb.remove_drawing(sheet, id);
    let mut last = None;
    for piece in pieces {
        let mut drawing = Drawing::shape(piece.shape, (piece.width, piece.height));
        drawing.offset = (frame.x as f32 + piece.x, frame.y as f32 + piece.y);
        drawing.rotation = piece.rotation;
        drawing.flip_h = piece.flip_h;
        drawing.effects = piece.effects;
        // A line keeps its direction: it is level or upright here.
        last = app.wb.add_drawing(sheet, drawing).or(last);
    }
    close_pane(app, cx);
    app.commit(Cmd::SmartConvert.label(), cx);
    app.select_object(last.map(|id| object_id(&Drawings, id)), cx);
    true
}

// ---- on the sheet --------------------------------------------------------

/// A shape of the selected graphic, as a zone of the grid.
pub(super) struct NodeZone {
    id: u32,
    line: usize,
}

/// The zones of the entries of the selected graphic, and the frame around
/// the one that is chosen.
pub(super) fn paint_selection(p: &mut Painter, drawing: &Drawing, art: &SmartArt, placed: &Placed, natural: (f32, f32), brush: &Brush, items: &mut Vec<Draw>) {
    let chosen = if selected(p.app).is_some_and(|lead| lead.id == drawing.id) { chosen_lines(p.app) } else { Vec::new() };
    for piece in art.pieces(p.app.wb.theme(), natural.0, natural.1) {
        let Some(line) = piece.node else {
            continue;
        };
        let at = placed_piece(&piece, placed, natural);
        if chosen.contains(&line) {
            // The frame of a shape chosen inside the graphic: as that of
            // an object, a size smaller.
            let around = Placed::plain(at.x0 - 2.0, at.y0 - 2.0, at.x1 + 2.0, at.y1 + 2.0);
            let frame = geometry::Polyline {
                points: around.corners().to_vec(),
                closed: true,
            };
            brush.stroke(&frame, 1.0, &[], c(0x5F5F5F), items);
            for handle in around.handles() {
                brush.disc(handle, 3.5, c(0x5F5F5F), items);
                brush.disc(handle, 2.5, c(0xFFFFFF), items);
            }
        }
        p.zone((at.x0, at.y0, at.x1, at.y1), NodeZone { id: drawing.id, line });
    }
}

/// A press on a shape of the selected graphic chooses its entry; on the
/// entry that is chosen already, or twice, it types in the shape, the
/// caret where the press was.
pub(super) fn mouse_down(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> Option<ZoneResponse> {
    let node = zone.payload::<NodeZone>()?;
    if ev.button != MouseButton::Left {
        return Some(ZoneResponse::Ignored);
    }
    let chosen = app.ext::<SmartUi>().and_then(|ui| ui.node) == Some((node.id, node.line));
    // Shift or ⌘: the shape joins those that are chosen, or leaves them.
    if ev.modifiers.shift || ev.modifiers.platform {
        end_edit(app, cx);
        let lead = current_line(app);
        let ui = app.ext_mut::<SmartUi>();
        match lead {
            None => ui.node = Some((node.id, node.line)),
            Some(line) if line == node.line => {
                ui.node = (!ui.also.is_empty()).then(|| (node.id, ui.also.remove(0)));
            }
            Some(_) => match ui.also.iter().position(|line| *line == node.line) {
                Some(at) => {
                    ui.also.remove(at);
                }
                None => ui.also.push(node.line),
            },
        }
        cx.notify();
        return Some(ZoneResponse::Handled);
    }
    app.ext_mut::<SmartUi>().also.clear();
    if chosen || ev.click_count >= 2 {
        begin_edit(app, node.id, node.line, TextStart::At(ev.position), window, cx);
        return Some(ZoneResponse::Handled);
    }
    end_edit(app, cx);
    app.ext_mut::<SmartUi>().node = Some((node.id, node.line));
    // The keys are the graphic's from here on, not a line's of the pane.
    window.focus(&app.grid_focus);
    cx.notify();
    Some(ZoneResponse::Handled)
}

/// The selection left the graphic: what was typed is kept, the pane goes.
pub(super) fn selection_changed(app: &mut ExcelApp, id: Option<u32>, cx: &mut Cx) {
    if editing(app).is_some_and(|(of, _)| Some(of) != id) {
        end_edit(app, cx);
    }
    if app.ext::<SmartUi>().and_then(|ui| ui.node).is_some_and(|(of, _)| Some(of) != id) {
        app.ext_mut::<SmartUi>().also.clear();
    }
    let shown = app.ext::<SmartUi>().and_then(|ui| ui.pane.as_ref()).map(|pane| pane.read(cx).id);
    if shown.is_some() && shown != id {
        flush(app, cx);
        close_pane(app, cx);
    }
}

// ---- the text pane -------------------------------------------------------

struct Row {
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// The Text Pane: the outline of a graphic, a line for each entry.
/// Return adds a line, Tab and Shift+Tab demote and promote it, Delete or
/// Backspace on an empty line removes it; the graphic follows as it is
/// typed.
pub(super) struct TextPane {
    app: WeakEntity<ExcelApp>,
    id: u32,
    rows: Vec<Row>,
    /// The line to give the keyboard to at the next repaint.
    focus: Option<usize>,
}

impl TextPane {
    fn outline(&self, cx: &App) -> Vec<Line> {
        self.app
            .upgrade()
            .and_then(|app| app.read(cx).wb.drawing(app.read(cx).sheet, self.id).and_then(Drawing::as_smartart).map(SmartArt::outline))
            .unwrap_or_default()
    }

    /// A field for each line, holding what the outline says.
    fn sync(&mut self, lines: &[Line], window: &mut Window, cx: &mut Context<Self>) {
        while self.rows.len() > lines.len() {
            self.rows.pop();
        }
        while self.rows.len() < lines.len() {
            let input = widgets::input(cx, "");
            let subscription = cx.subscribe_in(&input, window, Self::on_input);
            self.rows.push(Row {
                input,
                _subscription: subscription,
            });
        }
        for (row, (_, text)) in self.rows.iter().zip(lines) {
            if row.input.read(cx).text() != text {
                row.input.update(cx, |input, cx| input.set_text(text.clone(), cx));
            }
        }
    }

    fn line_of(&self, input: &Entity<TextInput>) -> Option<usize> {
        self.rows.iter().position(|row| row.input == *input)
    }

    /// Run a change of the outline on the application.
    fn act(&mut self, cx: &mut Context<Self>, act: impl FnOnce(&mut ExcelApp, &mut Cx)) {
        self.app.update(cx, |app, cx| act(app, cx)).ok();
        cx.notify();
    }

    fn on_input(&mut self, input: &Entity<TextInput>, event: &InputEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(line) = self.line_of(input) else {
            return;
        };
        let id = self.id;
        let choose = move |app: &mut ExcelApp| app.ext_mut::<SmartUi>().node = Some((id, line));
        match event {
            InputEvent::Changed => {
                // A line is one line: a line feed that was typed or
                // pasted does not stay.
                let mut text = input.read(cx).text().to_string();
                if text.contains(['\n', '\r']) {
                    text.retain(|ch| !matches!(ch, '\n' | '\r'));
                    input.update(cx, |input, cx| input.set_text(text.clone(), cx));
                }
                self.act(cx, |app, cx| type_text(app, id, line, &text, cx));
            }
            InputEvent::Enter { .. } => {
                self.act(cx, |app, cx| {
                    choose(app);
                    change(app, t("Add Shape", "Ajouter une forme"), cx, |art, at| {
                        *at = art.add_line_after(line);
                        at.is_some()
                    });
                });
                self.focus = Some(line + 1);
            }
            InputEvent::Tab { shift } => {
                let cmd = if *shift { Cmd::SmartPromote } else { Cmd::SmartDemote };
                self.act(cx, |app, cx| {
                    choose(app);
                    restructure(app, cmd, cx);
                });
                self.focus = Some(line);
            }
            InputEvent::Navigate { key, .. } => match key {
                NavKey::Up if line > 0 => self.focus = Some(line - 1),
                NavKey::Down if line + 1 < self.rows.len() => self.focus = Some(line + 1),
                _ => {}
            },
            InputEvent::Focused => self.act(cx, |app, cx| {
                choose(app);
                cx.notify();
            }),
            InputEvent::Blurred => self.act(cx, flush),
            _ => {}
        }
        cx.notify();
    }

    /// Delete or Backspace on a line: an empty one is removed.
    fn remove_empty(&mut self, line: usize, backward: bool, cx: &mut Context<Self>) -> bool {
        let empty = self.rows.get(line).is_some_and(|row| row.input.read(cx).text().is_empty());
        if !empty || self.rows.len() < 2 {
            return false;
        }
        let id = self.id;
        self.act(cx, |app, cx| {
            app.ext_mut::<SmartUi>().node = Some((id, line));
            change(app, t("Delete", "Supprimer"), cx, |art, at| {
                let removed = art.remove(line);
                *at = Some(if backward { line.saturating_sub(1) } else { line.min(art.outline().len().saturating_sub(1)) });
                removed
            });
        });
        self.focus = Some(if backward { line.saturating_sub(1) } else { line.min(self.rows.len().saturating_sub(2)) });
        true
    }

    #[cfg(test)]
    pub(super) fn texts(&self, cx: &App) -> Vec<String> {
        self.rows.iter().map(|row| row.input.read(cx).text().to_string()).collect()
    }
}

impl Render for TextPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.outline(cx);
        self.sync(&lines, window, cx);
        if let Some(row) = self.focus.take().and_then(|line| self.rows.get(line)) {
            window.focus(&row.input.read(cx).focus_handle(cx));
        }
        let chosen = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).ext::<SmartUi>().and_then(|ui| ui.node))
            .filter(|(id, _)| *id == self.id)
            .map(|(_, line)| line);
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .zip(&lines)
            .enumerate()
            .map(|(line, (row, (level, _)))| {
                div()
                    .id(("smartart-line", line))
                    .h(px(26.))
                    .pl(px(6.0 + 16.0 * *level as f32))
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(chosen == Some(line), |d| d.bg(c(theme::HOVER)))
                    // Before the field deletes a character: an empty line
                    // goes instead.
                    .capture_action(cx.listener(move |this, _: &crate::text_input::Backspace, _, cx| {
                        if this.remove_empty(line, true, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .capture_action(cx.listener(move |this, _: &crate::text_input::Delete, _, cx| {
                        if this.remove_empty(line, false, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .child(div().flex_none().text_color(c(theme::TEXT_MUTED)).child("•"))
                    .child(div().flex_1().child(widgets::TextField::new(&row.input).width(232.0 - 16.0 * *level as f32)))
                    .into_any_element()
            })
            .collect();
        div()
            .w(px(280.))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .text_size(px(12.))
            .child(div().pb(px(4.)).text_color(c(theme::TEXT_MUTED)).child(t("Type your text here", "Tapez votre texte ici")))
            .children(rows)
    }
}

pub(super) fn open_pane(app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
    let Some((id, art)) = selected_art(app) else {
        return;
    };
    if art.foreign.is_some() {
        return;
    }
    let line = current_line(app).unwrap_or(0);
    let weak = cx.weak_entity();
    let pane = cx.new(|_| TextPane {
        app: weak,
        id,
        rows: Vec::new(),
        focus: Some(line),
    });
    app.ext_mut::<SmartUi>().pane = Some(pane.clone());
    app.open_pane(PANE, t("Text Pane", "Volet Texte"), pane, cx);
}

fn close_pane(app: &mut ExcelApp, cx: &mut Cx) {
    if app.ext_mut::<SmartUi>().pane.take().is_some() {
        app.close_pane(PANE, cx);
    }
}

pub(super) fn pane_shown(app: &ExcelApp) -> bool {
    app.pane_open(PANE) && app.ext::<SmartUi>().is_some_and(|ui| ui.pane.is_some())
}

pub(super) fn toggle_pane(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if pane_shown(app) {
        flush(app, cx);
        close_pane(app, cx);
    } else {
        open_pane(app, window, cx);
    }
}

/// The pane of the selected graphic, for the tests.
#[cfg(test)]
pub(super) fn pane(app: &ExcelApp) -> Option<Entity<TextPane>> {
    app.ext::<SmartUi>()?.pane.clone()
}

// ---- names ---------------------------------------------------------------

pub(super) fn layout_name(layout: Layout) -> &'static str {
    match layout {
        Layout::BlockList => t("Basic Block List", "Liste de blocs simple"),
        Layout::BulletList => t("Vertical Bullet List", "Liste à puces verticale"),
        Layout::Process => t("Basic Process", "Processus simple"),
        Layout::Chevron => t("Basic Chevron Process", "Processus simple en chevrons"),
        Layout::Cycle => t("Basic Cycle", "Cycle simple"),
        Layout::OrgChart => t("Organization Chart", "Organigramme"),
        Layout::Venn => t("Basic Venn", "Venn simple"),
        Layout::Pyramid => t("Basic Pyramid", "Pyramide simple"),
    }
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::List => t_ctx("smartart", "List", "Liste"),
        Category::Process => t_ctx("smartart", "Process", "Processus"),
        Category::Cycle => t_ctx("smartart", "Cycle", "Cycle"),
        Category::Hierarchy => t_ctx("smartart", "Hierarchy", "Hiérarchie"),
        Category::Relationship => t_ctx("smartart", "Relationship", "Relation"),
        Category::Pyramid => t_ctx("smartart", "Pyramid", "Pyramide"),
    }
}

// ---- previews and galleries ----------------------------------------------

/// A graphic drawn small, without its text: what the galleries show.
fn preview(art: SmartArt, theme: Arc<Theme>, width: f32, height: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let ts = window.text_system().clone();
            let brush = Brush {
                origin: bounds.origin,
                zoom: 1.0,
                scale: window.scale_factor(),
                ts: &ts,
                clip: bounds,
            };
            let mut items = Vec::new();
            let placed = Placed::plain(1.0, 1.0, width - 1.0, height - 1.0);
            draw(&art, &theme, &placed, (width - 2.0, height - 2.0), &brush, None, opensheet_engine::Locale::en(), None, &mut items);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// A graphic of a layout with the entries a new one has.
fn sample(layout: Layout, colors: ColorScheme, style: QuickStyle) -> SmartArt {
    SmartArt {
        colors,
        style,
        ..SmartArt::new(layout)
    }
}

/// What a drop-down of the tab chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Layouts,
    Colors,
    Styles,
}

struct Gallery {
    app: WeakEntity<ExcelApp>,
    choice: Choice,
}

impl Gallery {
    fn cell(&self, index: usize, on: bool, content: impl IntoElement, name: Option<&'static str>, apply: impl Fn(&mut SmartArt) + 'static, label: &'static str) -> AnyElement {
        let app = self.app.clone();
        div()
            .id(("smartart-choice", index))
            .p(px(3.))
            .rounded(px(3.))
            .border_1()
            .border_color(if on { c(theme::ACCENT) } else { c(theme::BORDER) })
            .bg(c(0xFFFFFF))
            .hover(|s| s.border_color(c(theme::ACCENT)))
            .child(content)
            .when_some(name, |d, name| d.tooltip(tooltip(name)))
            .on_click(move |_, _, cx| {
                app.update(cx, |app, cx| {
                    app.popup = None;
                    change(app, label, cx, |art, _| {
                        let before = art.clone();
                        apply(art);
                        *art != before
                    });
                })
                .ok();
            })
            .into_any_element()
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((art, theme)) = self.app.upgrade().and_then(|app| {
            let app = app.read(cx);
            Some((selected_art(app)?.1.clone(), app.wb.theme_arc()))
        }) else {
            return div();
        };
        let section = |title: &'static str| {
            div()
                .w_full()
                .mt(px(4.))
                .px(px(4.))
                .h(px(20.))
                .flex()
                .items_center()
                .bg(c(theme::CHROME_BG))
                .text_size(px(11.5))
                .text_color(c(theme::TEXT_MUTED))
                .child(title)
                .into_any_element()
        };
        let mut cells: Vec<AnyElement> = Vec::new();
        let mut index = 0;
        match self.choice {
            Choice::Layouts => {
                for layout in Layout::ALL {
                    let shown = sample(layout, art.colors, art.style);
                    let on = art.foreign.is_none() && art.layout == layout;
                    cells.push(self.cell(index, on, preview(shown, theme.clone(), 84.0, 56.0), Some(layout_name(layout)), move |art| art.set_layout(layout), Cmd::SmartLayouts.label()));
                    index += 1;
                }
            }
            Choice::Colors => {
                let mut group = None;
                for colors in ColorScheme::all() {
                    let title = match colors {
                        ColorScheme::Dark(_) => t("Primary Theme Colors", "Principales couleurs de thème"),
                        ColorScheme::Colorful(_) => t("Colorful", "En couleur"),
                        ColorScheme::Accent { accent: 1, .. } => t("Accent 1", "Accentuation 1"),
                        ColorScheme::Accent { accent: 2, .. } => t("Accent 2", "Accentuation 2"),
                        ColorScheme::Accent { accent: 3, .. } => t("Accent 3", "Accentuation 3"),
                        ColorScheme::Accent { accent: 4, .. } => t("Accent 4", "Accentuation 4"),
                        ColorScheme::Accent { accent: 5, .. } => t("Accent 5", "Accentuation 5"),
                        ColorScheme::Accent { .. } => t("Accent 6", "Accentuation 6"),
                    };
                    if group != Some(title) {
                        group = Some(title);
                        cells.push(section(title));
                    }
                    let mut shown = sample(Layout::Process, colors, art.style);
                    shown.nodes.truncate(3);
                    cells.push(self.cell(index, art.colors == colors, preview(shown, theme.clone(), 72.0, 22.0), None, move |art| art.colors = colors, Cmd::SmartColors.label()));
                    index += 1;
                }
            }
            Choice::Styles => {
                for style in QuickStyle::ALL {
                    let mut shown = art.clone();
                    shown.style = style;
                    cells.push(self.cell(index, art.style == style, preview(shown, theme.clone(), 84.0, 56.0), None, move |art| art.style = style, Cmd::SmartStyles.label()));
                    index += 1;
                }
            }
        }
        div().w(px(if self.choice == Choice::Colors { 420.0 } else { 392.0 })).p(px(6.)).flex().flex_wrap().gap(px(4.)).children(cells)
    }
}

pub(super) fn open_gallery(app: &mut ExcelApp, choice: Choice, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |_| Gallery { app: weak, choice });
}

/// Insert ▸ SmartArt: "Choose a SmartArt Graphic". The categories, the
/// layouts of the one chosen, and the layout that is picked drawn large.
pub(super) struct ChooseDialog {
    modal: Modal,
    theme: Arc<Theme>,
    /// `None`: all.
    category: Option<Category>,
    pub(super) layout: Layout,
}

impl ChooseDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let layout = self.layout;
        self.modal.close(window, cx);
        let app = self.modal.app.clone();
        // Once the dialog is gone: the Text Pane takes the keyboard.
        window.defer(cx, move |window, cx| {
            app.update(cx, |app, cx| {
                insert(app, layout, window, cx);
            })
            .ok();
        });
    }
}

impl Render for ChooseDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut categories: Vec<SharedString> = vec![t_ctx("smartart", "All", "Tous").into()];
        categories.extend(Category::ALL.into_iter().map(|category| SharedString::from(category_name(category))));
        let selected_category = self.category.and_then(|category| Category::ALL.iter().position(|other| *other == category)).map_or(0, |at| at + 1);
        let layouts: Vec<AnyElement> = Layout::ALL
            .into_iter()
            .filter(|layout| self.category.is_none_or(|category| layout.category() == category))
            .enumerate()
            .map(|(index, layout)| {
                let on = self.layout == layout;
                div()
                    .id(("smartart-layout", index))
                    .p(px(4.))
                    .rounded(px(3.))
                    .border_2()
                    .border_color(if on { c(theme::ACCENT) } else { gpui::transparent_black() })
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(preview(sample(layout, ColorScheme::default(), QuickStyle::default()), self.theme.clone(), 78.0, 52.0))
                    .tooltip(tooltip(layout_name(layout)))
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        this.layout = layout;
                        cx.notify();
                        if event.click_count() >= 2 {
                            this.ok(&(), window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let chosen = sample(self.layout, ColorScheme::default(), QuickStyle::default());
        widgets::dialog(&self.modal, t("Choose a SmartArt Graphic", "Choisir un graphique SmartArt"))
            .width(760.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ListBox::new("smartart-categories", categories, Some(selected_category))
                            .width(150.0)
                            .height(300.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                this.category = index.checked_sub(1).and_then(|at| Category::ALL.get(at).copied());
                                // The layout picked stays in view.
                                if let Some(category) = this.category.filter(|category| this.layout.category() != *category) {
                                    this.layout = Layout::ALL.into_iter().find(|layout| layout.category() == category).unwrap_or(this.layout);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(290.))
                            .h(px(300.))
                            .border_1()
                            .border_color(c(theme::BORDER))
                            .bg(c(0xFFFFFF))
                            .p(px(6.))
                            .flex()
                            .flex_wrap()
                            .content_start()
                            .gap(px(4.))
                            .children(layouts),
                    )
                    .child(
                        div()
                            .w(px(270.))
                            .h(px(300.))
                            .border_1()
                            .border_color(c(theme::BORDER))
                            .bg(c(theme::CHROME_BG))
                            .p(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(div().bg(c(0xFFFFFF)).child(preview(chosen, self.theme.clone(), 248.0, 166.0)))
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(layout_name(self.layout))),
                    ),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

pub(super) fn open_dialog(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<ChooseDialog>> {
    if !app.may(Intent::EditObject(None), cx) {
        return None;
    }
    let theme = app.wb.theme_arc();
    Some(app.open_modal(window, cx, move |modal, _, _| ChooseDialog {
        modal,
        theme,
        category: None,
        layout: Layout::BlockList,
    }))
}

// ---- the tab -------------------------------------------------------------

/// The groups of the "SmartArt Design" tab.
pub(super) fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Create Graphic", "Créer un graphique"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sa-add", Cmd::SmartAddShape, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sa-bullet", Cmd::SmartAddBullet, false, cx),
                    app.cmd_labeled("sa-pane", Cmd::SmartTextPane, false, cx),
                ]))
                .child(rows(vec![
                    app.cmd_labeled("sa-promote", Cmd::SmartPromote, false, cx),
                    app.cmd_labeled("sa-demote", Cmd::SmartDemote, false, cx),
                    app.cmd_labeled("sa-rtl", Cmd::SmartRightToLeft, false, cx),
                ]))
                .child(rows(vec![
                    app.cmd_labeled("sa-up", Cmd::SmartMoveUp, false, cx),
                    app.cmd_labeled("sa-down", Cmd::SmartMoveDown, false, cx),
                ])),
        )
        .into_any_element(),
        group(t("Layouts", "Dispositions"), row(vec![app.cmd_large("sa-layouts", Cmd::SmartLayouts, true, cx)])).into_any_element(),
        group(
            t("SmartArt Styles", "Styles SmartArt"),
            row(vec![
                app.cmd_large("sa-colors", Cmd::SmartColors, true, cx),
                app.cmd_large("sa-styles", Cmd::SmartStyles, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t_ctx("smartart", "Reset", "Rétablir"),
            row(vec![
                app.cmd_large("sa-reset", Cmd::SmartReset, false, cx),
                app.cmd_large("sa-convert", Cmd::SmartConvert, false, cx),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// The groups of the "Format" tab of a graphic: the shapes that are
/// chosen in it, and their text.
pub(super) fn format_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Shapes", "Formes"),
            rows(vec![
                app.cmd_labeled("saf-shape", Cmd::SmartChangeShape, true, cx),
                app.cmd_labeled("saf-larger", Cmd::SmartLarger, false, cx),
                app.cmd_labeled("saf-smaller", Cmd::SmartSmaller, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Shape Styles", "Styles de formes"),
            rows(vec![
                app.cmd_labeled("saf-fill", Cmd::ShapeFill, true, cx),
                app.cmd_labeled("saf-outline", Cmd::ShapeOutline, true, cx),
                app.cmd_labeled("saf-effects", Cmd::ShapeEffects, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("WordArt Styles", "Styles WordArt"),
            rows(vec![
                app.cmd_labeled("saf-text-fill", Cmd::TextFill, true, cx),
                app.cmd_labeled("saf-text-outline", Cmd::TextOutline, true, cx),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// Run a command of the tab.
pub(super) fn run(app: &mut ExcelApp, cmd: Cmd, window: &mut Window, cx: &mut Cx) {
    match cmd {
        Cmd::SmartArt => {
            open_dialog(app, window, cx);
        }
        Cmd::SmartAddShape => {
            let entries = [Cmd::SmartAddAfter, Cmd::SmartAddBefore, Cmd::SmartAddAbove, Cmd::SmartAddBelow]
                .into_iter()
                .map(|cmd| MenuEntry::command(cmd, app))
                .collect();
            app.open_command_menu(entries, 240.0, window, cx);
        }
        Cmd::SmartAddAfter => {
            add_shape(app, Side::After, cx);
        }
        Cmd::SmartAddBefore => {
            add_shape(app, Side::Before, cx);
        }
        Cmd::SmartAddAbove => {
            add_shape(app, Side::Above, cx);
        }
        Cmd::SmartAddBelow | Cmd::SmartAddBullet => {
            add_shape(app, Side::Below, cx);
        }
        Cmd::SmartTextPane => toggle_pane(app, window, cx),
        Cmd::SmartPromote | Cmd::SmartDemote | Cmd::SmartMoveUp | Cmd::SmartMoveDown => {
            restructure(app, cmd, cx);
        }
        Cmd::SmartRightToLeft => {
            change(app, cmd.label(), cx, |art, _| {
                art.rtl = !art.rtl;
                true
            });
        }
        Cmd::SmartLayouts => open_gallery(app, Choice::Layouts, window, cx),
        Cmd::SmartColors => open_gallery(app, Choice::Colors, window, cx),
        Cmd::SmartStyles => open_gallery(app, Choice::Styles, window, cx),
        Cmd::SmartReset => {
            change(app, cmd.label(), cx, |art, _| {
                let before = art.clone();
                art.reset();
                *art != before
            });
        }
        Cmd::SmartChangeShape => open_change_shape(app, window, cx),
        Cmd::SmartLarger | Cmd::SmartSmaller => {
            resize(app, cmd, cx);
        }
        Cmd::SmartConvert => {
            convert(app, cx);
        }
        _ => {}
    }
}
