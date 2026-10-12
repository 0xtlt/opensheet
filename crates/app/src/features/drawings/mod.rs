//! Pictures, shapes and text boxes: inserting them, drawing them, working
//! on them with the mouse, and the "Picture Format" and "Shape Format"
//! tabs, the Format pane and the menus that go with them.
//!
//! Selecting (one object or several), moving, nudging, deleting, copying,
//! pasting, restacking and aligning come from `objects.rs` through the
//! `*_object` hooks below; a command of this feature works on every
//! selected drawing (`edit`), and on the members of a selected group. What that
//! file cannot know is done here: a drawing is hit where it is drawn (a
//! line along its length, a turned shape where it was turned to), and it
//! has handles of its own (the eight of its turned frame and the one that
//! turns it, or the two ends of a line).
//!
//! * `adjust.rs`: cropping, transparency, corrections, colours and the
//!   effects that are drawn: their menus and what they change.
//! * `geometry.rs`: flips, rotation, outlines, handles: plain numbers.
//! * `paint.rs`: where a drawing is in each pane, and what draws it.
//! * `picture.rs`: the bitmaps of pictures.
//! * `text.rs`: the text of shapes as it is drawn (it is typed in
//!   `widgets::text_area`, placed over the shape).
//! * `tool.rs`: drawing a shape by dragging, the handles, bringing
//!   pictures in.
//! * `tabs.rs`: the two ribbon tabs, the gallery and the drop-downs.
//! * `pane.rs`: the Format Shape / Format Picture, Alt Text and Selection
//!   panes.
//!
//! Positions are always asked of the engine (`Sheet::drawing_frame`): a
//! drawing sized with its cells follows them without being touched.

mod adjust;
pub(crate) mod geometry;
mod paint;
mod pane;
mod picture;
mod tabs;
mod text;
mod tool;
#[cfg(test)]
mod ui_tests;

use super::{
    Command, ContextMenu, ContextTab, Cx, Feature, Intent, MenuSection, MenuTarget, ObjectInfo, ObjectVerb, PaintLayer,
    Painter, Placement, ZMove, Zone, ZoneEvent, ZoneResponse, object_id,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{Draw, GridLayout};
use crate::i18n::t;
use geometry::normalise_angle;
use gpui::{
    AnyElement, ClipboardEntry, ClipboardItem, CursorStyle, Entity, EntityInputHandler, ExternalPaths, Focusable,
    MouseButton, MouseDownEvent, Pixels, Point, Subscription, Window, div, prelude::*, px,
};
use crate::objects::Align;
use opensheet_engine::drawing::{
    Drawing, DrawingKind, Effects, Frame, ImageFormat, Picture, PictureAdjust, Shape, ShapePreset, ShapeTool, TextAlign,
    TextAnchor, ZOrder,
};
use opensheet_engine::style::{HAlign, Style, VAlign};
use paint::Grip;
use std::any::Any;
use std::rc::Rc;
use crate::widgets::text_area::{AreaEvent, TextArea};

pub struct Drawings;

/// The id of the Format Shape / Format Picture pane.
pub(crate) const FORMAT_PANE: &str = pane::FORMAT;

const PICTURE_TAB: &str = "picture-format";
const SHAPE_TAB: &str = "shape-format";

commands! {
    /// What the user can ask of pictures and shapes.
    pub enum Cmd {
        Pictures = "picture", t("Pictures", "Images");
        PictureFromFile = "picture", t("Picture from File…", "Image à partir d'un fichier…");
        StockImages = "picture", t("Stock Images…", "Images de photothèque…");
        OnlinePictures = "picture", t("Online Pictures…", "Images en ligne…");
        Shapes = "shapes", t("Shapes", "Formes");
        Icons = "shapes", t("Icons", "Icônes");
        SmartArt = "smartart", t("SmartArt", "SmartArt");
        TextBox = "textbox", t("Text Box", "Zone de texte");
        WordArt = "wordart", t("WordArt", "WordArt");
        // Arrange
        BringForward = "bring-forward", t("Bring Forward", "Avancer");
        BringToFront = "bring-forward", t("Bring to Front", "Mettre au premier plan");
        SendBackward = "send-backward", t("Send Backward", "Reculer");
        SendToBack = "send-backward", t("Send to Back", "Mettre à l'arrière-plan");
        SelectionPane = "columns", t("Selection Pane", "Volet Sélection");
        Align = "align-objects", t("Align", "Aligner");
        AlignLeft = "align-left", t("Align Left", "Aligner à gauche");
        AlignCenter = "align-center", t("Align Center", "Aligner au centre");
        AlignRight = "align-right", t("Align Right", "Aligner à droite");
        AlignTop = "align-top", t("Align Top", "Aligner en haut");
        AlignMiddle = "align-middle", t("Align Middle", "Aligner au milieu");
        AlignBottom = "align-bottom", t("Align Bottom", "Aligner en bas");
        DistributeHorizontally = "", t("Distribute Horizontally", "Répartir horizontalement");
        DistributeVertically = "", t("Distribute Vertically", "Répartir verticalement");
        SnapToGrid = "", t("Snap to Grid", "Aligner sur la grille");
        Group = "group-objects", t("Group", "Grouper");
        GroupObjects = "group-objects", t("Group", "Grouper");
        Regroup = "", t("Regroup", "Regrouper");
        Ungroup = "", t("Ungroup", "Dissocier");
        Rotate = "refresh", t("Rotate", "Rotation");
        RotateRight = "drawings-rotate-right", t("Rotate Right 90°", "Faire pivoter de 90° à droite");
        RotateLeft = "drawings-rotate-left", t("Rotate Left 90°", "Faire pivoter à gauche de 90°");
        FlipVertical = "drawings-flip-v", t("Flip Vertical", "Retourner verticalement");
        FlipHorizontal = "drawings-flip-h", t("Flip Horizontal", "Retourner horizontalement");
        MoreRotation = "", t("More Rotation Options…", "Autres options de rotation…");
        // Picture Format
        RemoveBackground = "picture", t("Remove Background", "Supprimer l'arrière-plan");
        Corrections = "zoom", t("Corrections", "Corrections");
        Color = "fill", t("Color", "Couleur");
        ArtisticEffects = "brush", t("Artistic Effects", "Effets artistiques");
        Transparency = "eye", t("Transparency", "Transparence");
        Compress = "merge", t("Compress Pictures", "Compresser les images");
        ChangePicture = "picture", t("Change Picture", "Modifier l'image");
        ChangePictureFromFile = "picture", t("From a File…", "À partir d'un fichier…");
        ResetPicture = "refresh", t("Reset Picture", "Rétablir l'image");
        ResetPictureOnly = "refresh", t("Reset Picture", "Rétablir l'image");
        ResetPictureAndSize = "refresh", t("Reset Picture & Size", "Rétablir l'image et la taille");
        PictureStyles = "cell-styles", t("Quick Styles", "Styles rapides");
        PictureBorder = "borders", t("Picture Border", "Bordure de l'image");
        PictureEffects = "shapes", t("Picture Effects", "Effets de l'image");
        Crop = "crop", t("Crop", "Rogner");
        CropPicture = "crop", t("Crop", "Rogner");
        CropFill = "", t("Fill", "Remplissage");
        CropFit = "", t("Fit", "Ajuster");
        // Shape Format
        EditShape = "shapes", t("Edit Shape", "Modifier la forme");
        EditPoints = "", t("Edit Points", "Modifier les points");
        ShapeStyles = "cell-styles", t("Shape Styles", "Styles de formes");
        ShapeFill = "fill", t("Shape Fill", "Remplissage");
        ShapeOutline = "borders", t("Shape Outline", "Contour");
        ShapeEffects = "shapes", t("Shape Effects", "Effets");
        TextFill = "fill", t("Text Fill", "Remplissage du texte");
        TextOutline = "borders", t("Text Outline", "Contour du texte");
        TextEffects = "wordart", t("Text Effects", "Effets du texte");
        // Both
        AltText = "comment", t("Alt Text", "Texte de remplacement");
        ViewAltText = "comment", t("View Alt Text…", "Afficher le texte de remplacement…");
        Size = "format-cells", t("Size and Properties…", "Taille et propriétés…");
        FormatShape = "format-cells", t("Format Shape…", "Format de la forme…");
        FormatPicture = "format-cells", t("Format Picture…", "Format de l'image…");
        EditText = "textbox", t("Edit Text", "Modifier le texte");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        let drawing = selected(app);
        let shape = drawing.and_then(Drawing::as_shape);
        match self {
            // Restacking works for any floating object.
            Cmd::BringForward | Cmd::BringToFront | Cmd::SendBackward | Cmd::SendToBack | Cmd::Align => {
                app.object_sel.is_some()
            }
            Cmd::AlignLeft => app.can_align(Align::Left),
            Cmd::AlignCenter => app.can_align(Align::Center),
            Cmd::AlignRight => app.can_align(Align::Right),
            Cmd::AlignTop => app.can_align(Align::Top),
            Cmd::AlignMiddle => app.can_align(Align::Middle),
            Cmd::AlignBottom => app.can_align(Align::Bottom),
            Cmd::DistributeHorizontally => app.can_align(Align::DistributeHorizontally),
            Cmd::DistributeVertically => app.can_align(Align::DistributeVertically),
            Cmd::Group => drawing.is_some(),
            // Pictures and shapes only: a chart stays out of groups.
            Cmd::GroupObjects => {
                let all = app.selected_objects();
                all.len() >= 2 && all.iter().all(|id| id.owner == owner())
            }
            Cmd::Ungroup => selected_ids(app)
                .into_iter()
                .any(|id| app.wb.drawing(app.sheet, id).is_some_and(|drawing| drawing.as_group().is_some())),
            Cmd::Regroup => regroupable(app).is_some(),
            Cmd::EditPoints => false,
            Cmd::Crop
            | Cmd::CropPicture
            | Cmd::CropFill
            | Cmd::CropFit
            | Cmd::Corrections
            | Cmd::Color
            | Cmd::Transparency
            | Cmd::PictureEffects
            | Cmd::PictureBorder => drawing.is_some_and(is_picture),
            Cmd::ShapeEffects => drawing.is_some(),
            Cmd::Rotate
            | Cmd::RotateRight
            | Cmd::RotateLeft
            | Cmd::FlipVertical
            | Cmd::FlipHorizontal
            | Cmd::MoreRotation
            | Cmd::AltText
            | Cmd::ViewAltText
            | Cmd::Size => drawing.is_some(),
            Cmd::FormatPicture | Cmd::ResetPicture | Cmd::ResetPictureOnly | Cmd::ResetPictureAndSize | Cmd::ChangePicture => {
                drawing.is_some_and(is_picture)
            }
            Cmd::FormatShape | Cmd::ShapeFill | Cmd::ShapeOutline | Cmd::TextFill => {
                drawing.is_some_and(|drawing| !is_picture(drawing))
            }
            Cmd::EditShape => shape.is_some(),
            Cmd::EditText => shape.is_some_and(|shape| !shape.preset.is_line()),
            _ => true,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::Pictures => {
                let entries = vec![
                    MenuEntry::command(Cmd::PictureFromFile, app),
                    MenuEntry::command(Cmd::StockImages, app),
                    MenuEntry::command(Cmd::OnlinePictures, app),
                ];
                app.open_command_menu(entries, 250.0, window, cx);
            }
            Cmd::PictureFromFile => tool::pick_pictures(app, false, window, cx),
            Cmd::ChangePictureFromFile => tool::pick_pictures(app, true, window, cx),
            Cmd::Shapes => tabs::open_gallery(app, window, cx),
            Cmd::TextBox => tool::arm(app, ShapeTool::TextBox, window, cx),
            Cmd::BringForward => app.reorder_object(ZMove::Forward, cx),
            Cmd::BringToFront => app.reorder_object(ZMove::ToFront, cx),
            Cmd::SendBackward => app.reorder_object(ZMove::Backward, cx),
            Cmd::SendToBack => app.reorder_object(ZMove::ToBack, cx),
            Cmd::Rotate => {
                let entries = vec![
                    MenuEntry::command(Cmd::RotateRight, app),
                    MenuEntry::command(Cmd::RotateLeft, app),
                    MenuEntry::command(Cmd::FlipVertical, app),
                    MenuEntry::command(Cmd::FlipHorizontal, app),
                    MenuEntry::separator(),
                    MenuEntry::command(Cmd::MoreRotation, app),
                ];
                app.open_command_menu(entries, 270.0, window, cx);
            }
            Cmd::RotateRight => turn(app, 90.0, cx),
            Cmd::RotateLeft => turn(app, -90.0, cx),
            Cmd::FlipVertical => flip(app, false, cx),
            Cmd::FlipHorizontal => flip(app, true, cx),
            Cmd::Align => tabs::open_align_menu(app, window, cx),
            Cmd::AlignLeft => app.align_objects(Align::Left, cx),
            Cmd::AlignCenter => app.align_objects(Align::Center, cx),
            Cmd::AlignRight => app.align_objects(Align::Right, cx),
            Cmd::AlignTop => app.align_objects(Align::Top, cx),
            Cmd::AlignMiddle => app.align_objects(Align::Middle, cx),
            Cmd::AlignBottom => app.align_objects(Align::Bottom, cx),
            Cmd::DistributeHorizontally => app.align_objects(Align::DistributeHorizontally, cx),
            Cmd::DistributeVertically => app.align_objects(Align::DistributeVertically, cx),
            Cmd::SnapToGrid => app.toggle_snap_to_grid(cx),
            Cmd::Group => {
                let entries = vec![
                    MenuEntry::command(Cmd::GroupObjects, app),
                    MenuEntry::command(Cmd::Regroup, app),
                    MenuEntry::command(Cmd::Ungroup, app),
                ];
                app.open_command_menu(entries, 180.0, window, cx);
            }
            Cmd::GroupObjects => group(app, cx),
            Cmd::Ungroup => ungroup(app, cx),
            Cmd::Regroup => regroup(app, cx),
            Cmd::Crop => adjust::open_crop_menu(app, window, cx),
            Cmd::CropPicture => adjust::toggle_crop(app, cx),
            Cmd::CropFill => adjust::crop_to_frame(app, false, cx),
            Cmd::CropFit => adjust::crop_to_frame(app, true, cx),
            Cmd::Transparency => adjust::open_transparency(app, window, cx),
            Cmd::Corrections => adjust::open_corrections(app, window, cx),
            Cmd::Color => adjust::open_color(app, window, cx),
            Cmd::ShapeEffects | Cmd::PictureEffects => adjust::open_effects(app, window, cx),
            Cmd::SelectionPane => pane::toggle_selection(app, cx),
            Cmd::AltText | Cmd::ViewAltText => pane::open_alt_text(app, window, cx),
            Cmd::Size | Cmd::MoreRotation => pane::open_format(app, pane::Section::Size, window, cx),
            Cmd::FormatShape => pane::open_format(app, pane::Section::Fill, window, cx),
            Cmd::FormatPicture => pane::open_format(app, pane::Section::Size, window, cx),
            Cmd::EditText => {
                if let Some(id) = selected(app).map(|drawing| drawing.id) {
                    begin_text_edit(app, id, TextStart::End, window, cx);
                }
            }
            Cmd::ShapeFill => tabs::open_fill(app, tabs::Paint::Fill, window, cx),
            Cmd::TextFill => tabs::open_fill(app, tabs::Paint::Text, window, cx),
            Cmd::ShapeOutline => tabs::open_outline(app, window, cx),
            Cmd::EditShape => {
                let current = selected(app).and_then(Drawing::as_shape).map(|shape| shape.preset);
                let closed = current.is_some_and(|preset| !preset.is_line());
                let shapes = ShapeTool::ALL
                    .into_iter()
                    .filter(|tool| !tool.preset().is_line() && *tool != ShapeTool::TextBox)
                    .map(|tool| {
                        MenuEntry::new(tabs::tool_name(tool, app.wb.locale), move |app, _, cx| {
                            edit_shape(app, t("Change Shape", "Modifier la forme"), cx, |shape| shape.preset = tool.preset());
                        })
                        .checked(current == Some(tool.preset()))
                    })
                    .collect();
                let entries = vec![
                    MenuEntry::submenu(t("Change Shape", "Modifier la forme"), shapes).disabled(!closed),
                    MenuEntry::command(Cmd::EditPoints, app),
                ];
                app.open_command_menu(entries, 220.0, window, cx);
            }
            Cmd::ResetPicture => {
                let entries = vec![
                    MenuEntry::command(Cmd::ResetPictureOnly, app),
                    MenuEntry::command(Cmd::ResetPictureAndSize, app),
                ];
                app.open_command_menu(entries, 250.0, window, cx);
            }
            Cmd::ResetPictureOnly => reset_picture(app, false, cx),
            Cmd::ResetPictureAndSize => reset_picture(app, true, cx),
            Cmd::ChangePicture => {
                let entries = vec![MenuEntry::command(Cmd::ChangePictureFromFile, app)];
                app.open_command_menu(entries, 220.0, window, cx);
            }
            // Styles, artistic effects, removing the background,
            // compressing and the online sources are not available: the
            // status bar says so.
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

/// The text of a shape being typed in place.
struct TextEdit {
    id: u32,
    area: Entity<TextArea>,
    _subscription: Subscription,
}

/// What the feature remembers between events.
#[derive(Default)]
struct DrawingUi {
    /// The shape of the gallery waiting to be drawn on the sheet.
    tool: Option<ShapeTool>,
    /// What the mouse is dragging: a new shape, or a handle.
    drag: Option<tool::Drag>,
    text: Option<TextEdit>,
    /// The picture being cropped: it shows whole, with the marks that
    /// move the edges of its frame.
    crop: Option<u32>,
    /// The drawings the last Ungroup let go: Regroup takes them back.
    ungrouped: Vec<u32>,
    /// The fields and the view the ribbon and the panes keep.
    tabs: tabs::TabState,
    panes: pane::PaneState,
}

/// Where the caret goes when the text of a shape starts being edited.
enum TextStart {
    End,
    /// After what was just typed with the shape selected.
    Typed(String),
    /// Under the pointer (a double click).
    At(Point<Pixels>),
}

fn owner() -> usize {
    super::index_of(&Drawings)
}

/// The selected picture or shape.
fn selected(app: &ExcelApp) -> Option<&Drawing> {
    let id = app.object_sel.filter(|id| id.owner == owner())?;
    app.wb.drawing(app.sheet, id.id)
}

/// Whether a drawing is a picture (it gets the "Picture Format" tab) or a
/// shape.
fn is_picture(drawing: &Drawing) -> bool {
    matches!(drawing.kind, DrawingKind::Picture(_))
}

/// Change a drawing of the current sheet, as one undo step. False when
/// it is not there or the sheet's protection forbids it.
fn edit_drawing(app: &mut ExcelApp, id: u32, label: &str, cx: &mut Cx, change: impl FnOnce(&mut Drawing)) -> bool {
    if app.wb.drawing(app.sheet, id).is_none() || !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    app.wb.update_drawing(app.sheet, id, change);
    app.commit(label, cx);
    true
}

/// The selected pictures and shapes, the one selected last first.
fn selected_ids(app: &ExcelApp) -> Vec<u32> {
    app.selected_objects()
        .into_iter()
        .filter(|id| id.owner == owner())
        .map(|id| id.id)
        .collect()
}

/// Change the selected drawings, as one undo step. False when none is
/// selected or the sheet's protection holds one of them.
fn edit(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut Drawing)) -> bool {
    let ids = selected_ids(app);
    if ids.is_empty() || !ids.iter().all(|id| app.may(Intent::EditObject(Some(object_id(&Drawings, *id))), cx)) {
        return false;
    }
    for id in ids {
        app.wb.update_drawing(app.sheet, id, &change);
    }
    app.commit(label, cx);
    true
}

/// Do something to a drawing, or to every member of the group it is.
fn each_leaf(drawing: &mut Drawing, change: &dyn Fn(&mut Drawing)) {
    match &mut drawing.kind {
        DrawingKind::Group(group) => {
            for member in &mut group.members {
                each_leaf(member, change);
            }
        }
        _ => change(drawing),
    }
}

/// True when a drawing is, or as a group holds, something `test` likes.
fn any_leaf(drawing: &Drawing, test: &dyn Fn(&Drawing) -> bool) -> bool {
    match &drawing.kind {
        DrawingKind::Group(group) => group.members.iter().any(|member| any_leaf(member, test)),
        _ => test(drawing),
    }
}

/// Change the selected pictures and shapes, and those in the selected
/// groups, as one undo step.
fn edit_leaves(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut Drawing)) -> bool {
    edit(app, label, cx, |drawing| each_leaf(drawing, &change))
}

/// Change the selected shapes, and those in the selected groups (nothing
/// happens to a picture).
fn edit_shape(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut Shape)) -> bool {
    let is_shape = |drawing: &Drawing| drawing.as_shape().is_some();
    let any = selected_ids(app)
        .into_iter()
        .any(|id| app.wb.drawing(app.sheet, id).is_some_and(|drawing| any_leaf(drawing, &is_shape)));
    if !any {
        return false;
    }
    edit_leaves(app, label, cx, |drawing| {
        if let Some(shape) = drawing.as_shape_mut() {
            change(shape);
        }
    })
}

/// Change the selected pictures, and those in the selected groups.
fn edit_picture(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: impl Fn(&mut Picture)) -> bool {
    let is_picture = |drawing: &Drawing| drawing.as_picture().is_some();
    let any = selected_ids(app)
        .into_iter()
        .any(|id| app.wb.drawing(app.sheet, id).is_some_and(|drawing| any_leaf(drawing, &is_picture)));
    if !any {
        return false;
    }
    edit_leaves(app, label, cx, |drawing| {
        if let Some(picture) = drawing.as_picture_mut() {
            change(picture);
        }
    })
}

/// "Group": the selected pictures and shapes become one object.
fn group(app: &mut ExcelApp, cx: &mut Cx) {
    let ids = selected_ids(app);
    if !Cmd::GroupObjects.enabled(app) || !app.may_edit_selected_objects(cx) {
        return;
    }
    end_text_edit(app, cx);
    let Some(id) = app.wb.group_drawings(app.sheet, &ids) else {
        return;
    };
    app.commit(t("Group Objects", "Grouper les objets"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
}

/// "Ungroup": the members of the selected groups are objects of the
/// sheet again, all selected.
fn ungroup(app: &mut ExcelApp, cx: &mut Cx) {
    if !Cmd::Ungroup.enabled(app) || !app.may_edit_selected_objects(cx) {
        return;
    }
    let mut freed = Vec::new();
    for id in selected_ids(app) {
        let members = app.wb.ungroup_drawing(app.sheet, id);
        if members.is_empty() {
            // Not a group: it stays selected with the others.
            freed.push(id);
        } else {
            app.ext_mut::<DrawingUi>().ungrouped = members.clone();
            freed.extend(members);
        }
    }
    app.commit(t("Ungroup Objects", "Dissocier les objets"), cx);
    // The topmost leads.
    let ids: Vec<_> = freed.into_iter().rev().map(|id| object_id(&Drawings, id)).collect();
    app.select_objects(&ids, cx);
}

/// What Regroup would hold together again: the drawings the last Ungroup
/// let go, while they are all still on the sheet and one of them is
/// selected.
fn regroupable(app: &ExcelApp) -> Option<Vec<u32>> {
    let ids = app.ext::<DrawingUi>().map(|ui| ui.ungrouped.clone())?;
    let selected = selected_ids(app);
    let whole = ids.len() >= 2 && ids.iter().all(|id| app.wb.drawing(app.sheet, *id).is_some());
    (whole && ids.iter().any(|id| selected.contains(id))).then_some(ids)
}

fn regroup(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(ids) = regroupable(app) else {
        return;
    };
    if !ids.iter().all(|id| app.may(Intent::EditObject(Some(object_id(&Drawings, *id))), cx)) {
        return;
    }
    let Some(id) = app.wb.group_drawings(app.sheet, &ids) else {
        return;
    };
    app.ext_mut::<DrawingUi>().ungrouped.clear();
    app.commit(t("Regroup Objects", "Regrouper les objets"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
}

fn turn(app: &mut ExcelApp, degrees: f32, cx: &mut Cx) {
    edit(app, t("Rotate Object", "Faire pivoter l'objet"), cx, |drawing| {
        drawing.rotation = normalise_angle(drawing.rotation + degrees);
    });
}

/// What a flip does to a drawing: the mirror image of a turned frame is
/// the mirrored frame turned the other way.
fn flipped(drawing: &mut Drawing, horizontal: bool) {
    if horizontal {
        drawing.flip_h = !drawing.flip_h;
    } else {
        drawing.flip_v = !drawing.flip_v;
    }
    drawing.rotation = normalise_angle(-drawing.rotation);
}

fn flip(app: &mut ExcelApp, horizontal: bool, cx: &mut Cx) {
    edit(app, t("Flip Object", "Retourner l'objet"), cx, |drawing| flipped(drawing, horizontal));
}

/// The size a picture has when it is inserted, in pixels at 100 %.
fn natural_size(drawing: &Drawing) -> Option<(f32, f32)> {
    let picture = drawing.as_picture()?;
    Some(
        opensheet_engine::drawing::image_info(&picture.data)
            .map_or((picture.pixels.0 as f32, picture.pixels.1 as f32), |info| info.natural_size()),
    )
}

/// "Reset Picture": what was done to how the picture shows is undone
/// (colours, transparency, effects, rotation and flips); with `size`,
/// its crop too, and it is back at the size it was inserted at.
fn reset_picture(app: &mut ExcelApp, size: bool, cx: &mut Cx) {
    let label = if size { Cmd::ResetPictureAndSize.label() } else { Cmd::ResetPictureOnly.label() };
    end_crop(app, cx);
    edit(app, label, cx, |drawing| {
        let Some(natural) = natural_size(drawing) else {
            return;
        };
        drawing.rotation = 0.0;
        drawing.flip_h = false;
        drawing.flip_v = false;
        drawing.effects = Effects::default();
        if let Some(picture) = drawing.as_picture_mut() {
            picture.adjust = PictureAdjust {
                crop: if size { [0.0; 4] } else { picture.adjust.crop },
                shape: if size { ShapePreset::Rect } else { picture.adjust.shape },
                ..PictureAdjust::default()
            };
        }
        if size {
            drawing.size = natural;
        }
    });
}

/// The picture being cropped.
fn cropping(app: &ExcelApp) -> Option<u32> {
    app.ext::<DrawingUi>()?.crop
}

/// Leave the crop marks. False when no picture was being cropped.
fn end_crop(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    if cropping(app).is_none() {
        return false;
    }
    app.ext_mut::<DrawingUi>().crop = None;
    cx.notify();
    true
}

/// What a formatting command of the Home tab does to a shape: `change` is
/// applied to a cell style standing for the shape, and what it changed
/// goes back into the text of the shape (font, colour, alignment) and its
/// fill.
fn restyle(shape: &mut Shape, change: &dyn Fn(&mut Style)) {
    let before = Style {
        font: shape.font.clone(),
        fill: shape.fill,
        h_align: match shape.align {
            TextAlign::Left => HAlign::Left,
            TextAlign::Center => HAlign::Center,
            TextAlign::Right => HAlign::Right,
            TextAlign::Justify => HAlign::Justify,
        },
        v_align: match shape.anchor {
            TextAnchor::Top => VAlign::Top,
            TextAnchor::Middle => VAlign::Center,
            TextAnchor::Bottom => VAlign::Bottom,
        },
        ..Style::default()
    };
    let mut after = before.clone();
    change(&mut after);
    shape.font = after.font;
    shape.fill = after.fill;
    if after.h_align != before.h_align {
        shape.align = match after.h_align {
            HAlign::Center | HAlign::CenterAcross => TextAlign::Center,
            HAlign::Right => TextAlign::Right,
            HAlign::Justify | HAlign::Fill => TextAlign::Justify,
            HAlign::General | HAlign::Left => TextAlign::Left,
        };
    }
    shape.anchor = match after.v_align {
        VAlign::Top => TextAnchor::Top,
        VAlign::Center => TextAnchor::Middle,
        VAlign::Bottom => TextAnchor::Bottom,
    };
}

/// Start typing the text of a shape in place.
fn begin_text_edit(app: &mut ExcelApp, id: u32, start: TextStart, window: &mut Window, cx: &mut Cx) {
    let takes_text = app
        .wb
        .drawing(app.sheet, id)
        .and_then(Drawing::as_shape)
        .is_some_and(|shape| !shape.preset.is_line());
    if !takes_text || !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return;
    }
    end_text_edit(app, cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    let Some(shape) = app.wb.drawing(app.sheet, id).and_then(Drawing::as_shape) else {
        return;
    };
    let content = shape.text.clone();
    let area = cx.new(|cx| {
        let mut area = TextArea::new(cx);
        area.set_text(content, cx);
        area
    });
    area.update(cx, |area, cx| match start {
        TextStart::Typed(typed) => area.replace_text_in_range(None, &typed, window, cx),
        // A double click puts the caret where it was made.
        TextStart::At(position) => area.caret_at_next_paint(position),
        TextStart::End => {}
    });
    let subscription = cx.subscribe_in(&area, window, |app, _, event: &AreaEvent, window, cx| match event {
        // Escape leaves the text and keeps the shape selected.
        AreaEvent::Escape => {
            end_text_edit(app, cx);
            window.focus(&app.grid_focus);
        }
        AreaEvent::Blurred => end_text_edit(app, cx),
        _ => {}
    });
    window.focus(&area.read(cx).focus_handle(cx));
    app.ext_mut::<DrawingUi>().text = Some(TextEdit {
        id,
        area,
        _subscription: subscription,
    });
    cx.notify();
}

/// Stop typing in a shape: what was typed becomes its text, as one undo
/// step.
fn end_text_edit(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(edit) = app.ext_mut::<DrawingUi>().text.take() else {
        return;
    };
    let typed = edit.area.read(cx).text().to_string();
    let sheet = app.sheet;
    let changed = app
        .wb
        .drawing(sheet, edit.id)
        .and_then(Drawing::as_shape)
        .is_some_and(|shape| shape.text != typed);
    if changed {
        app.wb.update_drawing(sheet, edit.id, |drawing| {
            if let Some(shape) = drawing.as_shape_mut() {
                shape.text = typed;
            }
        });
        app.commit(t("Typing", "Saisie"), cx);
    } else {
        cx.notify();
    }
}

/// The id of the shape whose text is being typed.
fn editing_text(app: &ExcelApp) -> Option<u32> {
    app.ext::<DrawingUi>()?.text.as_ref().map(|edit| edit.id)
}

/// The field in which the text of a shape is typed, over the shape.
fn text_overlay(app: &ExcelApp, layout: &GridLayout, cx: &mut Cx) -> Option<AnyElement> {
    let edit = app.ext::<DrawingUi>()?.text.as_ref()?;
    let sheet = &app.wb.sheets[app.sheet];
    let drawing = app.wb.drawing(app.sheet, edit.id)?;
    let shape = drawing.as_shape()?;
    let frame = sheet.drawing_frame(drawing);
    // In the pane that shows the middle of the shape.
    let panes = paint::panes(layout);
    let in_pane = |pane: &paint::Pane| {
        let placed = paint::placed(layout, sheet, drawing, pane);
        let (cx, cy) = placed.centre();
        pane.contains(cx, cy).then_some(placed)
    };
    let (pane, placed) = panes.iter().find_map(|pane| in_pane(pane).map(|placed| (*pane, placed)))?;
    let (x0, y0, x1, y1) = paint::text_box(shape, &placed, (frame.width as f32, frame.height as f32), layout.zoom);
    // The font follows the shape (bold from the Home tab) and the zoom.
    let look = paint::text_look(shape, layout.zoom, &app.fonts);
    edit.area.update(cx, |area, cx| {
        if area.align != look.align() || area.caret_color != Some(look.color) {
            area.align = look.align();
            area.caret_color = Some(look.color);
            cx.notify();
        }
    });
    let area = edit.area.clone();
    let (cx0, cy0, cx1, cy1) = pane.clip;
    Some(
        div()
            .absolute()
            .left(px(cx0))
            .top(px(cy0))
            .w(px(cx1 - cx0))
            .h(px(cy1 - cy0))
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .left(px(x0 - cx0))
                    .top(px(y0 - cy0))
                    .w(px(x1 - x0))
                    .h(px(y1 - y0))
                    .flex()
                    .flex_col()
                    .map(|d| match look.anchor {
                        TextAnchor::Top => d.justify_start(),
                        TextAnchor::Middle => d.justify_center(),
                        TextAnchor::Bottom => d.justify_end(),
                    })
                    .font_family(look.font.family.clone())
                    .font_weight(look.font.weight)
                    .when(look.font.style == gpui::FontStyle::Italic, |d| d.italic())
                    .text_size(px(look.size))
                    .line_height(look.line_height())
                    .text_color(look.color)
                    .occlude()
                    // A click beside the text still lands in it.
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
                        window.focus(&area.read(cx).focus_handle(cx));
                        area.update(cx, |area, cx| area.caret_to_point(event.position, cx));
                    })
                    .child(div().flex_none().w_full().child(edit.area.clone())),
            )
            .into_any_element(),
    )
}

impl Feature for Drawings {
    fn name(&self) -> &'static str {
        "drawings"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match selected(app) {
            Some(drawing) if is_picture(drawing) => vec![ContextTab {
                id: PICTURE_TAB,
                title: t("Picture Format", "Format de l'image"),
            }],
            Some(_) => vec![ContextTab {
                id: SHAPE_TAB,
                title: t("Shape Format", "Format de forme"),
            }],
            // Charts selected together with other objects are arranged
            // from this tab too.
            None if app.selected_objects().len() > 1 => vec![ContextTab {
                id: SHAPE_TAB,
                title: t("Shape Format", "Format de forme"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, tab: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        if tab == PICTURE_TAB {
            tabs::picture_tab(app, window, cx)
        } else {
            tabs::shape_tab(app, window, cx)
        }
    }

    /// While a shape of the gallery waits to be drawn, the sheet belongs
    /// to it: a crosshair, and the shape as it is dragged out.
    fn paint(&self, p: &mut Painter) {
        if p.layer == PaintLayer::Top {
            tool::paint_tool(p);
        }
        // "Select Objects" works on every kind of object; this feature
        // lends it its place among the painters.
        crate::objects::paint_pointer(p);
    }

    fn overlay_elements(&self, app: &mut ExcelApp, layout: &GridLayout, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        picture::sweep(window, cx);
        // Image files dropped on the cells become pictures. The element
        // takes nothing else from the mouse.
        let mut out = vec![
            div()
                .absolute()
                .left(px(layout.header_w))
                .top(px(layout.header_h))
                .right_0()
                .bottom_0()
                .on_drop(cx.listener(|app, paths: &ExternalPaths, window, cx| tool::drop_files(app, paths, window, cx)))
                .into_any_element(),
        ];
        out.extend(text_overlay(app, layout, cx));
        out
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        if let Some(response) = crate::objects::pointer_mouse_down(app, zone, ev, cx) {
            return response;
        }
        tool::mouse_down(app, zone, ev, window, cx)
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        if !crate::objects::pointer_drag(app, zone, ev, cx) {
            tool::mouse_drag(app, zone, ev, cx);
        }
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        if !crate::objects::pointer_up(app, zone, ev, cx) {
            tool::mouse_up(app, zone, ev, window, cx);
        }
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        let MenuTarget::Object(id) = target else {
            return;
        };
        if id.owner != owner() {
            return;
        }
        let Some(drawing) = app.wb.drawing(app.sheet, id.id) else {
            return;
        };
        if drawing.as_shape().is_some_and(|shape| !shape.preset.is_line()) {
            menu.add(MenuSection::Edit, MenuEntry::command(Cmd::EditText, app));
        }
        menu.add(MenuSection::Extra, MenuEntry::command(Cmd::ViewAltText, app));
        menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Size, app));
        let format = if is_picture(drawing) { Cmd::FormatPicture } else { Cmd::FormatShape };
        menu.add(MenuSection::Extra, MenuEntry::command(format, app));
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        let Some(sh) = app.wb.sheets.get(sheet) else {
            return Vec::new();
        };
        sh.drawings
            .iter()
            .filter(|drawing| !drawing.hidden)
            .map(|drawing| {
                let ((anchor, offset), _) = sh.drawing_corners(drawing);
                let frame = sh.drawing_frame(drawing);
                ObjectInfo {
                    id: drawing.id,
                    place: Placement {
                        anchor,
                        offset,
                        size: (frame.width as f32, frame.height as f32),
                    },
                    min_size: (geometry::MIN_SIDE as f32, geometry::MIN_SIDE as f32),
                    keep_aspect: drawing.lock_aspect,
                }
            })
            .collect()
    }

    /// The drawing in every pane that shows it (the frame `objects.rs`
    /// computed stands for one pane only), then its handles when it is
    /// selected.
    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, _rect: (f32, f32, f32, f32)) {
        let app = p.app;
        let sheet = &app.wb.sheets[app.sheet];
        let Some(drawing) = app.wb.drawing(app.sheet, object.id) else {
            return;
        };
        let frame = sheet.drawing_frame(drawing);
        let natural = (frame.width as f32, frame.height as f32);
        let is_selected = app.is_object_selected(object_id(&Drawings, drawing.id));
        let typing = editing_text(app) == Some(drawing.id);
        let crop = drawing.as_picture().filter(|_| cropping(app) == Some(drawing.id));
        let zoom = p.zoom();
        for pane in paint::panes(p.layout) {
            let placed = paint::placed(p.layout, sheet, drawing, &pane);
            if !pane.shows(paint::extent(&placed, zoom)) && crop.is_none() {
                continue;
            }
            let clip = p.layout.bounds_px(pane.clip.0, pane.clip.1, pane.clip.2, pane.clip.3);
            let brush = paint::Brush {
                origin: p.layout.bounds.origin,
                zoom,
                scale: p.scale,
                ts: p.ts,
                clip,
            };
            let mut items = Vec::new();
            if let Some(picture) = crop {
                // The whole image, and the marks in place of the handles.
                paint::draw_cropping(picture, &placed, natural, &brush, &mut items);
                p.push(Draw::Clipped { clip, items });
                let (x0, y0, x1, y1) = placed.extent();
                let pan = tool::Handle { id: drawing.id, grip: Grip::Pan };
                p.zone((x0.max(pane.clip.0), y0.max(pane.clip.1), x1.min(pane.clip.2), y1.min(pane.clip.3)), pan)
                    .cursor(CursorStyle::OpenHand);
                for (i, at) in placed.handles().into_iter().enumerate() {
                    if pane.contains(at.0, at.1) {
                        let mark = tool::Handle { id: drawing.id, grip: Grip::Crop(i as u8) };
                        p.zone((at.0 - 8.0, at.1 - 8.0, at.0 + 8.0, at.1 + 8.0), mark).cursor(CursorStyle::Crosshair);
                    }
                }
                continue;
            }
            paint::draw(drawing, &placed, natural, &brush, (!typing).then_some(&*app.fonts), &mut items);
            if is_selected {
                paint::draw_selection(drawing, &placed, &brush, &mut items);
            }
            p.push(Draw::Clipped { clip, items });
            if !is_selected {
                continue;
            }
            for (grip, at) in paint::grips(drawing, &placed) {
                if !pane.contains(at.0, at.1) {
                    continue;
                }
                let cursor = match grip {
                    Grip::Resize(handle) => match geometry::handle_axis(handle, placed.rotation) {
                        0 => CursorStyle::ResizeLeftRight,
                        1 => CursorStyle::ResizeUpLeftDownRight,
                        2 => CursorStyle::ResizeUpDown,
                        _ => CursorStyle::ResizeUpRightDownLeft,
                    },
                    Grip::LineEnd(_) | Grip::Crop(_) => CursorStyle::Crosshair,
                    Grip::Rotate | Grip::Pan => CursorStyle::PointingHand,
                };
                p.zone((at.0 - 6.0, at.1 - 6.0, at.0 + 6.0, at.1 + 6.0), tool::Handle { id: drawing.id, grip })
                    .cursor(cursor);
            }
        }
    }

    /// Moves come here, and the sizes other selected objects are given
    /// when one of them is resized (a drawing's own handles resize it).
    fn place_object(&self, app: &mut ExcelApp, id: u32, place: Placement) {
        let sheet = app.sheet;
        if app.wb.drawing(sheet, id).is_none() {
            return;
        }
        let (x, y) = app.wb.sheets[sheet].corner_point((place.anchor, place.offset));
        app.wb.set_drawing_frame(sheet, id, Frame::new(x, y, place.size.0 as f64, place.size.1 as f64));
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
        // What was being typed in it goes with it.
        if editing_text(app) == Some(id) {
            app.ext_mut::<DrawingUi>().text = None;
        }
        if cropping(app) == Some(id) {
            app.ext_mut::<DrawingUi>().crop = None;
        }
        app.wb.remove_drawing(app.sheet, id);
    }

    fn copy_object(&self, app: &ExcelApp, id: u32) -> Option<Rc<dyn Any>> {
        let mut drawing = app.wb.drawing(app.sheet, id)?.clone();
        // The size it has now: the stored one may date from before its
        // columns were resized.
        let frame = app.wb.drawing_frame(app.sheet, id)?;
        drawing.size = (frame.width as f32, frame.height as f32);
        Some(Rc::new(drawing))
    }

    fn paste_object(&self, app: &mut ExcelApp, data: &dyn Any, at: ((u32, u32), (f32, f32))) -> Option<u32> {
        let mut drawing = data.downcast_ref::<Drawing>()?.clone();
        // The copy gets a name of its own.
        drawing.name.clear();
        drawing.anchor = at.0;
        drawing.offset = at.1;
        app.wb.add_drawing(app.sheet, drawing)
    }

    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        let order = match mv {
            ZMove::ToFront => ZOrder::BringToFront,
            ZMove::Forward => ZOrder::BringForward,
            ZMove::Backward => ZOrder::SendBackward,
            ZMove::ToBack => ZOrder::SendToBack,
        };
        app.wb.reorder_drawing(app.sheet, id, order);
    }

    /// A double click types in a shape; on a picture (or a line) it shows
    /// the tab that formats it.
    fn object_double_click(&self, app: &mut ExcelApp, id: u32, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        let Some(drawing) = app.wb.drawing(app.sheet, id) else {
            return;
        };
        if is_picture(drawing) {
            app.context_tab = Some(PICTURE_TAB);
            cx.notify();
        } else if drawing.as_shape().is_some_and(|shape| shape.preset.is_line()) {
            app.context_tab = Some(SHAPE_TAB);
            cx.notify();
        } else {
            begin_text_edit(app, id, TextStart::At(ev.position), window, cx);
        }
    }

    fn object_selected(&self, app: &mut ExcelApp, id: Option<u32>, cx: &mut Cx) {
        // Typing in a shape ends when the selection leaves it, and a
        // description typed in a pane is kept.
        if editing_text(app).is_some() && editing_text(app) != id {
            end_text_edit(app, cx);
        }
        // Cropping too, and when other objects join the selection.
        if cropping(app).is_some() && (cropping(app) != id || app.selected_objects().len() > 1) {
            end_crop(app, cx);
        }
        if id.is_none() {
            pane::keep_descriptions(app, cx);
        }
        pane::selection_changed(app, cx);
    }

    fn object_hit(&self, app: &ExcelApp, id: u32, layout: &GridLayout, x: f32, y: f32) -> Option<bool> {
        let sheet = &app.wb.sheets[app.sheet];
        let drawing = app.wb.drawing(app.sheet, id)?;
        let Some(pane) = paint::pane_at(layout, x, y) else {
            return Some(false);
        };
        let frame = sheet.drawing_frame(drawing);
        let placed = paint::placed(layout, sheet, drawing, &pane);
        Some(tool::hits(drawing, &placed, (frame.width as f32, frame.height as f32), layout.zoom, (x, y)))
    }

    fn object_extent(&self, app: &ExcelApp, id: u32, layout: &GridLayout) -> Option<(f32, f32, f32, f32)> {
        let sheet = &app.wb.sheets[app.sheet];
        let drawing = app.wb.drawing(app.sheet, id)?;
        // What shows of it, over all panes; a box off the grid when
        // nothing does.
        let mut shown: Option<(f32, f32, f32, f32)> = None;
        for pane in paint::panes(layout) {
            let extent = paint::extent(&paint::placed(layout, sheet, drawing, &pane), layout.zoom);
            if !pane.shows(extent) {
                continue;
            }
            let part = (
                extent.0.max(pane.clip.0),
                extent.1.max(pane.clip.1),
                extent.2.min(pane.clip.2),
                extent.3.min(pane.clip.3),
            );
            shown = Some(match shown {
                Some(all) => (all.0.min(part.0), all.1.min(part.1), all.2.max(part.2), all.3.max(part.3)),
                None => part,
            });
        }
        Some(shown.unwrap_or((-1.0, -1.0, -1.0, -1.0)))
    }

    fn object_own_handles(&self, _app: &ExcelApp, _id: u32) -> bool {
        true
    }

    /// Typing with a shape selected starts its text, after what it holds.
    fn object_typed(&self, app: &mut ExcelApp, id: u32, text: &str, window: &mut Window, cx: &mut Cx) -> bool {
        begin_text_edit(app, id, TextStart::Typed(text.to_string()), window, cx);
        editing_text(app) == Some(id)
    }

    /// Bold, font, colours and alignment of the Home tab go to the text of
    /// the selected shape; a picture has no use for them.
    fn object_style(&self, app: &mut ExcelApp, id: u32, label: &str, change: &dyn Fn(&mut Style), cx: &mut Cx) -> bool {
        let takes_text = app
            .wb
            .drawing(app.sheet, id)
            .and_then(Drawing::as_shape)
            .is_some_and(|shape| !shape.preset.is_line());
        if takes_text && app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
            app.wb.update_drawing(app.sheet, id, |drawing| {
                if let Some(shape) = drawing.as_shape_mut() {
                    restyle(shape, change);
                }
            });
            app.commit(label, cx);
        }
        true
    }

    /// A copied picture can be pasted in other applications.
    fn object_clipboard(&self, app: &ExcelApp, id: u32) -> Option<ClipboardItem> {
        let picture = app.wb.drawing(app.sheet, id)?.as_picture()?;
        let format = match picture.format {
            ImageFormat::Png => gpui::ImageFormat::Png,
            ImageFormat::Jpeg => gpui::ImageFormat::Jpeg,
            ImageFormat::Gif => gpui::ImageFormat::Gif,
            ImageFormat::Bmp => gpui::ImageFormat::Bmp,
        };
        Some(ClipboardItem::new_image(&gpui::Image::from_bytes(format, picture.data.to_vec())))
    }

    /// A picture copied in another application is pasted as a picture.
    fn paste_clipboard(&self, app: &mut ExcelApp, cx: &mut Cx) -> bool {
        if app.is_editing() {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        if item.text().is_some_and(|text| !text.is_empty()) {
            return false;
        }
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            ClipboardEntry::String(_) => None,
        }) else {
            return false;
        };
        // The picture this application put there itself: the object that
        // was copied is pasted, with its size and its rotation.
        let own = app.object_clip.as_ref().is_some_and(|(_, data)| {
            data.downcast_ref::<Drawing>()
                .and_then(Drawing::as_picture)
                .is_some_and(|picture| picture.data[..] == image.bytes()[..])
        });
        if own {
            return false;
        }
        match picture::importable(image.bytes().to_vec()) {
            Some(bytes) => {
                app.object_clip = None;
                tool::insert_pictures(app, vec![bytes], None, cx);
                true
            }
            None => false,
        }
    }

    /// Escape puts down the shape that was waiting to be drawn, or ends
    /// "Select Objects".
    fn cancel(&self, app: &mut ExcelApp, cx: &mut Cx) -> bool {
        tool::disarm(app, cx) || end_crop(app, cx) || app.end_object_pointer(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::Picture;
    use opensheet_engine::style::Color;

    #[test]
    fn flips_turn_the_other_way() {
        let mut drawing = Drawing::shape(ShapeTool::RightArrow.shape(), (96.0, 48.0));
        drawing.rotation = 30.0;
        flipped(&mut drawing, true);
        assert_eq!((drawing.flip_h, drawing.flip_v, drawing.rotation), (true, false, 330.0));
        flipped(&mut drawing, true);
        assert_eq!((drawing.flip_h, drawing.rotation), (false, 30.0));
        flipped(&mut drawing, false);
        assert_eq!((drawing.flip_v, drawing.rotation), (true, 330.0));
        // An upright drawing stays upright.
        let mut upright = Drawing::shape(ShapeTool::Rectangle.shape(), (10.0, 10.0));
        flipped(&mut upright, false);
        assert_eq!((upright.flip_v, upright.rotation), (true, 0.0));
    }

    #[test]
    fn home_tab_formats_the_text_of_a_shape() {
        let mut shape = ShapeTool::Rectangle.shape();
        let fill = shape.fill;
        restyle(&mut shape, &|style| style.font.bold = true);
        assert!(shape.font.bold);
        // Nothing else moved: the fill, the centred text.
        assert_eq!((shape.fill, shape.align, shape.anchor), (fill, TextAlign::Center, TextAnchor::Middle));
        restyle(&mut shape, &|style| style.h_align = HAlign::Right);
        restyle(&mut shape, &|style| style.v_align = VAlign::Top);
        assert_eq!((shape.align, shape.anchor), (TextAlign::Right, TextAnchor::Top));
        restyle(&mut shape, &|style| style.h_align = HAlign::General);
        assert_eq!(shape.align, TextAlign::Left);
        restyle(&mut shape, &|style| {
            style.font.color = Some(Color(0xFF0000));
            style.font.set_size_pt(18.0);
            style.fill = None;
        });
        assert_eq!((shape.font.color, shape.font.size_pt(), shape.fill), (Some(Color(0xFF0000)), 18.0, None));
        // A change that has nothing to do with shapes leaves it alone.
        let before = shape.clone();
        restyle(&mut shape, &|style| style.wrap = true);
        assert_eq!(shape, before);
    }

    #[test]
    fn a_picture_remembers_its_size() {
        // The header of a 300 x 150 GIF.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x2c, 0x01, 0x96, 0x00, 0, 0, 0]);
        let drawing = Drawing::picture(Picture::new(gif).unwrap());
        assert_eq!(natural_size(&drawing), Some((300.0, 150.0)));
        assert!(is_picture(&drawing));
        let shape = Drawing::shape(ShapeTool::Ellipse.shape(), (10.0, 10.0));
        assert_eq!(natural_size(&shape), None);
        assert!(!is_picture(&shape));
    }
}
