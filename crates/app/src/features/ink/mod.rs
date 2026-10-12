//! The Draw tab: pens, drawing by hand on the sheet, the eraser, the lasso,
//! Ink to Shape and Ink Replay.
//!
//! A stroke is a floating object of its own kind (`opensheet_engine::ink`):
//! selecting, moving, resizing, nudging, deleting, copying, pasting and
//! restacking come from `objects.rs` through the `*_object` hooks below.
//! What the lasso encloses is one selected stroke and its companions
//! (`InkUi::group`), which are moved, deleted and copied with it.
//!
//! * `tools.rs`: the pointer on the sheet with a pen, the eraser or the
//!   lasso in hand, and how strokes are painted.
//! * `tab.rs`: the Draw tab with its gallery of pens and their menus.
//!
//! The pens belong to the application, not to a workbook: they are kept
//! with the application's settings (`preferences::settings`).

mod tab;
mod tools;
#[cfg(test)]
mod ui_tests;

use super::preferences::settings;
use super::{
    Command, Cx, Feature, ObjectInfo, ObjectVerb, PaintLayer, Painter, Placement, ZMove, Zone, ZoneEvent,
    ZoneResponse, object_id,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::GridLayout;
use crate::i18n::t;
use gpui::Window;
use opensheet_engine::drawing::{Frame, ZOrder};
use opensheet_engine::ink::{Ink, PenKind};
use std::any::Any;
use std::rc::Rc;

pub use tab::draw_tab;

pub struct InkFeature;

commands! {
    /// What the user can ask of the Draw tab.
    pub enum Cmd {
        Select = "ink-select", t("Select", "Sélectionner");
        Lasso = "ink-lasso", t("Lasso Select", "Sélection par lasso");
        Eraser = "clear", t("Eraser", "Gomme");
        AddPen = "ink-add-pen", t("Add Pen", "Ajouter un stylo");
        AddPenPen = "ink-pen", t("Pen", "Stylo");
        AddPencil = "ink-pencil", t("Pencil", "Crayon");
        AddHighlighter = "ink-highlighter", t("Highlighter", "Surligneur");
        InkToShape = "shapes", t("Ink to Shape", "Entrée manuscrite en forme");
        Replay = "ink-replay", t("Ink Replay", "Relecture de l'entrée manuscrite");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let ui = app.ext::<InkUi>();
        let tool = ui.map(|ui| ui.tool).unwrap_or_default();
        match self {
            Cmd::Select => tool == Tool::Select,
            Cmd::Lasso => tool == Tool::Lasso,
            Cmd::Eraser => tool == Tool::Eraser,
            Cmd::InkToShape => ui.is_some_and(|ui| ui.to_shape),
            _ => false,
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            // There must be ink to erase, enclose or draw again.
            Cmd::Lasso | Cmd::Eraser | Cmd::Replay => !app.wb.sheets[app.sheet].inks.is_empty(),
            _ => true,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::Select => tools::take(app, Tool::Select, window, cx),
            Cmd::Lasso => tools::take(app, Tool::Lasso, window, cx),
            Cmd::Eraser => tools::take(app, Tool::Eraser, window, cx),
            Cmd::AddPen => {
                let entries = vec![
                    MenuEntry::command(Cmd::AddPenPen, app),
                    MenuEntry::command(Cmd::AddPencil, app),
                    MenuEntry::command(Cmd::AddHighlighter, app),
                ];
                app.open_command_menu(entries, 180.0, window, cx);
            }
            Cmd::AddPenPen => add_pen(app, PenKind::Pen, window, cx),
            Cmd::AddPencil => add_pen(app, PenKind::Pencil, window, cx),
            Cmd::AddHighlighter => add_pen(app, PenKind::Highlighter, window, cx),
            Cmd::InkToShape => {
                let ui = app.ext_mut::<InkUi>();
                ui.to_shape = !ui.to_shape;
                cx.notify();
            }
            Cmd::Replay => tools::replay(app, window, cx),
        }
    }
}

// --------------------------------------------------------------------- pens

/// A pen of the gallery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pen {
    pub kind: PenKind,
    /// 0xRRGGBB.
    pub color: u32,
    /// Thickness in millimetres, as the menu of the pen gives it.
    pub mm: f32,
}

impl Pen {
    /// Thickness in pixels at 100 %.
    pub fn width(&self) -> f32 {
        self.mm * 96.0 / 25.4
    }

    /// The thicknesses the menu of such a pen offers, in millimetres.
    pub fn thicknesses(kind: PenKind) -> [f32; 5] {
        match kind {
            PenKind::Highlighter => [2.0, 4.0, 6.0, 8.0, 10.0],
            PenKind::Pen | PenKind::Pencil => [0.25, 0.35, 0.5, 1.0, 2.0],
        }
    }

    /// A pen as "Add Pen" makes it.
    pub fn new(kind: PenKind) -> Pen {
        match kind {
            PenKind::Pen => Pen { kind, color: 0x000000, mm: 0.5 },
            PenKind::Pencil => Pen { kind, color: 0x404040, mm: 1.0 },
            PenKind::Highlighter => Pen { kind, color: 0xFFFC00, mm: 6.0 },
        }
    }

    /// One line of the settings file: "pen 000000 0.5".
    pub fn to_setting(&self) -> String {
        format!("{} {:06X} {}", self.kind.file_name(), self.color & 0xFF_FFFF, self.mm)
    }

    pub fn from_setting(text: &str) -> Option<Pen> {
        let mut words = text.split_whitespace();
        let kind = PenKind::from_file_name(words.next()?)?;
        let color = u32::from_str_radix(words.next()?, 16).ok()? & 0xFF_FFFF;
        let mm = words.next()?.parse::<f32>().ok().filter(|mm| mm.is_finite())?.clamp(0.1, 20.0);
        Some(Pen { kind, color, mm })
    }
}

/// The gallery a new installation starts with, as Excel's.
pub fn default_pens() -> Vec<Pen> {
    vec![
        Pen::new(PenKind::Pen),
        Pen {
            color: 0xE71224,
            ..Pen::new(PenKind::Pen)
        },
        Pen::new(PenKind::Pencil),
        Pen::new(PenKind::Highlighter),
    ]
}

/// The pens of the gallery, in order.
pub fn pens() -> Vec<Pen> {
    settings::get().pens
}

/// The colours a pen can be given, by rows of eight.
pub const PEN_COLORS: [u32; 16] = [
    0x000000, 0xFFFFFF, 0x849398, 0x5B2D90, 0x004F8B, 0x00A0D7, 0x33CCFF, 0x008C3A, 0x66CC00, 0xFFC114, 0xF6630D,
    0xE71224, 0xFF0066, 0xAB008B, 0xAE7C5B, 0x6C3B2A,
];

/// The colours of highlighters.
pub const HIGHLIGHTER_COLORS: [u32; 8] =
    [0xFFFC00, 0x00F900, 0x00FDFF, 0xFF40FF, 0xFF8517, 0xFF2600, 0x0096FF, 0xD2D2D2];

fn add_pen(app: &mut ExcelApp, kind: PenKind, window: &mut Window, cx: &mut Cx) {
    let mut all = pens();
    if all.len() >= tab::MAX_PENS {
        return;
    }
    all.push(Pen::new(kind));
    let index = all.len() - 1;
    store_pens(all);
    // The new pen is in hand, as in Excel.
    tools::take(app, Tool::Draw(index), window, cx);
}

/// Change the gallery and keep it for the next launch.
fn store_pens(all: Vec<Pen>) {
    settings::update(|settings| settings.pens = all);
}

// -------------------------------------------------------------------- state

/// What the pointer does on the sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Cells and objects, as everywhere else.
    #[default]
    Select,
    /// Drawing with the pen of this index in the gallery.
    Draw(usize),
    Eraser,
    Lasso,
}

/// What the feature remembers between events.
#[derive(Default)]
struct InkUi {
    tool: Tool,
    /// Ink to Shape is on: what is drawn becomes a shape when it looks
    /// like one.
    to_shape: bool,
    /// What the mouse is doing with the tool.
    drag: Option<tools::Drag>,
    /// The strokes the lasso selected along with the selected object.
    group: Vec<u32>,
    /// Ink Replay in progress.
    replay: Option<tools::Replay>,
}

fn owner() -> usize {
    super::index_of(&InkFeature)
}

/// The selected stroke.
fn selected(app: &ExcelApp) -> Option<&Ink> {
    let id = app.object_sel.filter(|id| id.owner == owner())?;
    app.wb.ink(app.sheet, id.id)
}

/// The companions of the selected stroke (what the lasso enclosed with
/// it), without the selected one.
fn group(app: &ExcelApp) -> Vec<u32> {
    let Some(first) = selected(app).map(|ink| ink.id) else {
        return Vec::new();
    };
    app.ext::<InkUi>()
        .map(|ui| ui.group.iter().copied().filter(|id| *id != first).collect())
        .unwrap_or_default()
}

/// What Copy puts aside: the strokes with where each was on the sheet.
struct Copied(Vec<(Ink, Frame)>);

impl Feature for InkFeature {
    fn name(&self) -> &'static str {
        "ink"
    }

    fn paint(&self, p: &mut Painter) {
        match p.layer {
            PaintLayer::Top => tools::paint_tool(p),
            _ => tools::paint_highlights(p),
        }
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        tools::mouse_down(app, zone, ev, window, cx)
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        tools::mouse_drag(app, zone, ev, cx);
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        tools::mouse_up(app, zone, ev, cx);
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        let Some(sh) = app.wb.sheets.get(sheet) else {
            return Vec::new();
        };
        sh.inks
            .iter()
            .map(|ink| {
                let frame = sh.ink_frame(ink);
                let (anchor, offset) = sh.corner_at(frame.x, frame.y);
                ObjectInfo {
                    id: ink.id,
                    place: Placement {
                        anchor,
                        offset,
                        size: (frame.width as f32, frame.height as f32),
                    },
                    min_size: (2.0, 2.0),
                    keep_aspect: false,
                }
            })
            .collect()
    }

    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, rect: (f32, f32, f32, f32)) {
        tools::paint_stroke(p, object.id, rect);
    }

    /// The companions of the stroke move by as much as it does.
    fn place_object(&self, app: &mut ExcelApp, id: u32, place: Placement) {
        let sheet = app.sheet;
        let Some(before) = app.wb.ink_frame(sheet, id) else {
            return;
        };
        let (x, y) = app.wb.sheets[sheet].corner_point((place.anchor, place.offset));
        app.wb.set_ink_frame(sheet, id, Frame::new(x, y, place.size.0 as f64, place.size.1 as f64));
        let (dx, dy) = (x - before.x, y - before.y);
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        for other in group(app) {
            if let Some(frame) = app.wb.ink_frame(sheet, other) {
                app.wb.set_ink_frame(sheet, other, Frame::new(frame.x + dx, frame.y + dy, frame.width, frame.height));
            }
        }
    }

    fn object_label(&self, _id: u32, verb: ObjectVerb) -> &'static str {
        match verb {
            ObjectVerb::Move => t("Move Object", "Déplacer l'objet"),
            ObjectVerb::Resize => t("Size Object", "Dimensionner l'objet"),
            ObjectVerb::Delete => t("Delete", "Supprimer"),
            ObjectVerb::Paste => t("Paste", "Coller"),
            ObjectVerb::Reorder => t("Change Order", "Changer l'ordre"),
        }
    }

    fn delete_object(&self, app: &mut ExcelApp, id: u32) {
        for other in group(app) {
            app.wb.remove_ink(app.sheet, other);
        }
        app.wb.remove_ink(app.sheet, id);
    }

    fn copy_object(&self, app: &ExcelApp, id: u32) -> Option<Rc<dyn Any>> {
        let sheet = &app.wb.sheets[app.sheet];
        let mut ids = vec![id];
        ids.extend(group(app));
        let copied: Vec<(Ink, Frame)> = sheet
            .inks
            .iter()
            .filter(|ink| ids.contains(&ink.id))
            .map(|ink| (ink.clone(), sheet.ink_frame(ink)))
            .collect();
        (!copied.is_empty()).then(|| Rc::new(Copied(copied)) as Rc<dyn Any>)
    }

    /// The copies keep their places among themselves; the corner of their
    /// common box goes to `at`.
    fn paste_object(&self, app: &mut ExcelApp, data: &dyn Any, at: ((u32, u32), (f32, f32))) -> Option<u32> {
        let Copied(strokes) = data.downcast_ref::<Copied>()?;
        let left = strokes.iter().map(|(_, frame)| frame.x).fold(f64::INFINITY, f64::min);
        let top = strokes.iter().map(|(_, frame)| frame.y).fold(f64::INFINITY, f64::min);
        let (x, y) = app.wb.sheets[app.sheet].corner_point(at);
        let mut new = Vec::new();
        for (ink, frame) in strokes {
            let mut copy = ink.clone();
            // The copy gets a name of its own.
            copy.name.clear();
            let place = Frame::new(x + frame.x - left, y + frame.y - top, frame.width, frame.height);
            new.extend(app.wb.add_ink(app.sheet, copy, place));
        }
        let first = *new.first()?;
        app.ext_mut::<InkUi>().group = new;
        Some(first)
    }

    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        let order = match mv {
            ZMove::ToFront => ZOrder::BringToFront,
            ZMove::Forward => ZOrder::BringForward,
            ZMove::Backward => ZOrder::SendBackward,
            ZMove::ToBack => ZOrder::SendToBack,
        };
        app.wb.reorder_ink(app.sheet, id, order);
    }

    fn object_selected(&self, app: &mut ExcelApp, id: Option<u32>, _cx: &mut Cx) {
        // The companions belong to the selection the lasso made: another
        // selection has none.
        if let Some(ui) = app.ext::<InkUi>() {
            let keeps = id.is_some_and(|id| ui.group.contains(&id));
            if !keeps && !ui.group.is_empty() {
                app.ext_mut::<InkUi>().group.clear();
            }
        }
    }

    /// A stroke is hit on its line, not anywhere in its box.
    fn object_hit(&self, app: &ExcelApp, id: u32, layout: &GridLayout, x: f32, y: f32) -> Option<bool> {
        Some(tools::hits(app, id, layout, x, y))
    }

    /// Escape puts the pen, the eraser or the lasso down.
    fn cancel(&self, app: &mut ExcelApp, cx: &mut Cx) -> bool {
        tools::put_down(app, cx)
    }

}

/// The id of a stroke among the floating objects.
fn stroke_id(id: u32) -> super::ObjectId {
    object_id(&InkFeature, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pens_are_kept_as_text() {
        for pen in default_pens() {
            assert_eq!(Pen::from_setting(&pen.to_setting()), Some(pen));
        }
        assert_eq!(Pen::new(PenKind::Highlighter).to_setting(), "highlighter FFFC00 6");
        assert_eq!(
            Pen::from_setting("pencil 00ff00 0.35"),
            Some(Pen {
                kind: PenKind::Pencil,
                color: 0x00FF00,
                mm: 0.35
            })
        );
        // What is not a pen is not read.
        for bad in ["", "brush 000000 1", "pen red 1", "pen 000000", "pen 000000 NaN"] {
            assert_eq!(Pen::from_setting(bad), None, "{bad:?}");
        }
        // Half a millimetre is nearly two pixels.
        assert!((Pen::new(PenKind::Pen).width() - 1.89).abs() < 0.01);
    }

    #[test]
    fn the_gallery_starts_as_excels() {
        assert_eq!(pens(), default_pens());
        store_pens(vec![Pen::new(PenKind::Highlighter)]);
        assert_eq!(pens().len(), 1);
    }
}
