//! Pictures, shapes and text boxes: inserting them, drawing them, and the
//! "Picture Format" and "Shape Format" tabs.
//!
//! Not written yet, except the two commands that restack any floating
//! object: the others are in place and say so. See `features/mod.rs`:
//! implementing `objects`, `paint_object`, `place_object` and the other
//! `*_object` hooks is all it takes for pictures and shapes to be
//! selected, moved, resized, nudged, copied, pasted and deleted like
//! charts (`charts.rs` is the example).

use super::{Command, ContextTab, Cx, Feature, ZMove};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use gpui::{AnyElement, Window, div, prelude::*, px};
use opensheet_engine::drawing::Drawing;

pub struct Drawings;

const PICTURE_TAB: &str = "picture-format";
const SHAPE_TAB: &str = "shape-format";

commands! {
    /// What the user can ask of pictures and shapes.
    pub enum Cmd {
        Pictures = "picture", t("Pictures", "Images");
        PictureFromDevice = "picture", t("This Device…", "Cet appareil…");
        StockImages = "picture", t("Stock Images…", "Images de stock…");
        OnlinePictures = "picture", t("Online Pictures…", "Images en ligne…");
        Shapes = "shapes", t("Shapes", "Formes");
        Icons = "shapes", t("Icons", "Icônes");
        SmartArt = "smartart", t("SmartArt", "SmartArt");
        TextBox = "textbox", t("Text Box", "Zone de texte");
        WordArt = "wordart", t("WordArt", "WordArt");
        // Arrange
        BringForward = "bring-forward", t("Bring Forward", "Avancer");
        SendBackward = "send-backward", t("Send Backward", "Reculer");
        SelectionPane = "columns", t("Selection Pane", "Volet Sélection");
        Align = "align-objects", t("Align", "Aligner");
        Group = "group-objects", t("Group", "Grouper");
        Rotate = "refresh", t("Rotate", "Rotation");
        // Picture Format
        RemoveBackground = "picture", t("Remove Background", "Supprimer l'arrière-plan");
        Corrections = "zoom", t("Corrections", "Corrections");
        Color = "fill", t("Color", "Couleur");
        ArtisticEffects = "brush", t("Artistic Effects", "Effets artistiques");
        Transparency = "eye", t("Transparency", "Transparence");
        Compress = "merge", t("Compress Pictures", "Compresser les images");
        ChangePicture = "picture", t("Change Picture", "Modifier l'image");
        ResetPicture = "refresh", t("Reset Picture", "Rétablir l'image");
        PictureStyles = "cell-styles", t("Quick Styles", "Styles rapides");
        PictureBorder = "borders", t("Picture Border", "Bordure de l'image");
        PictureEffects = "shapes", t("Picture Effects", "Effets de l'image");
        Crop = "crop", t("Crop", "Rogner");
        // Shape Format
        EditShape = "shapes", t("Edit Shape", "Modifier la forme");
        ShapeStyles = "cell-styles", t("Shape Styles", "Styles de formes");
        ShapeFill = "fill", t("Shape Fill", "Remplissage");
        ShapeOutline = "borders", t("Shape Outline", "Contour");
        ShapeEffects = "shapes", t("Shape Effects", "Effets");
        TextFill = "fill", t("Text Fill", "Remplissage du texte");
        TextOutline = "borders", t("Text Outline", "Contour du texte");
        TextEffects = "wordart", t("Text Effects", "Effets du texte");
        // Both
        AltText = "comment", t("Alt Text", "Texte de remplacement");
        Size = "format-cells", t("Size and Properties…", "Taille et propriétés…");
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
        // Restacking needs something selected.
        !matches!(self, Cmd::BringForward | Cmd::SendBackward) || app.object_sel.is_some()
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::BringForward => app.reorder_object(ZMove::Forward, cx),
            Cmd::SendBackward => app.reorder_object(ZMove::Backward, cx),
            Cmd::Pictures => {
                let entries = vec![
                    MenuEntry::command(Cmd::PictureFromDevice, app),
                    MenuEntry::command(Cmd::StockImages, app),
                    MenuEntry::command(Cmd::OnlinePictures, app),
                ];
                app.open_command_menu(entries, 220.0, window, cx);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

/// The selected picture or shape.
fn selected(app: &ExcelApp) -> Option<&Drawing> {
    let id = app.object_sel.filter(|id| id.owner == super::index_of(&Drawings))?;
    app.wb.sheets[app.sheet].drawings.iter().find(|drawing| drawing.id == id.id)
}

/// Whether a drawing is a picture (it gets the "Picture Format" tab) or a
/// shape. The engine does not tell them apart yet.
fn is_picture(_drawing: &Drawing) -> bool {
    false
}

/// The Size group shared by the two tabs.
fn size_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(t("Size", "Taille"), row(vec![app.cmd_large("dw-size", Cmd::Size, false, cx)]))
}

fn accessibility_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(
        t("Accessibility", "Accessibilité"),
        row(vec![app.cmd_large("dw-alt", Cmd::AltText, false, cx)]),
    )
}

/// The groups of the "Picture Format" tab.
fn picture_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Adjust", "Ajuster"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("pf-background", Cmd::RemoveBackground, false, cx))
                .child(app.cmd_large("pf-corrections", Cmd::Corrections, true, cx))
                .child(app.cmd_large("pf-color", Cmd::Color, true, cx))
                .child(app.cmd_large("pf-artistic", Cmd::ArtisticEffects, true, cx))
                .child(app.cmd_large("pf-transparency", Cmd::Transparency, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("pf-compress", Cmd::Compress, false, cx),
                    app.cmd_labeled("pf-change", Cmd::ChangePicture, true, cx),
                    app.cmd_labeled("pf-reset", Cmd::ResetPicture, true, cx),
                ])),
        )
        .into_any_element(),
        group(
            t("Picture Styles", "Styles d'image"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("pf-styles", Cmd::PictureStyles, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("pf-border", Cmd::PictureBorder, true, cx),
                    app.cmd_labeled("pf-effects", Cmd::PictureEffects, true, cx),
                ])),
        )
        .into_any_element(),
        accessibility_group(app, cx).into_any_element(),
        app.arrange_group("ar", cx).into_any_element(),
        group(t("Size", "Taille"), row(vec![
            app.cmd_large("pf-crop", Cmd::Crop, true, cx),
            app.cmd_large("dw-size", Cmd::Size, false, cx),
        ]))
        .border_r_0()
        .into_any_element(),
    ]
}

/// The groups of the "Shape Format" tab.
fn shape_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Insert Shapes", "Insérer des formes"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sf-shapes", Cmd::Shapes, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sf-edit", Cmd::EditShape, true, cx),
                    app.cmd_labeled("sf-textbox", Cmd::TextBox, false, cx),
                ])),
        )
        .into_any_element(),
        group(
            t("Shape Styles", "Styles de formes"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sf-styles", Cmd::ShapeStyles, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sf-fill", Cmd::ShapeFill, true, cx),
                    app.cmd_labeled("sf-outline", Cmd::ShapeOutline, true, cx),
                    app.cmd_labeled("sf-effects", Cmd::ShapeEffects, true, cx),
                ])),
        )
        .into_any_element(),
        group(
            t("WordArt Styles", "Styles WordArt"),
            rows(vec![
                app.cmd_labeled("sf-text-fill", Cmd::TextFill, true, cx),
                app.cmd_labeled("sf-text-outline", Cmd::TextOutline, true, cx),
                app.cmd_labeled("sf-text-effects", Cmd::TextEffects, true, cx),
            ]),
        )
        .into_any_element(),
        accessibility_group(app, cx).into_any_element(),
        app.arrange_group("ar", cx).into_any_element(),
        size_group(app, cx).border_r_0().into_any_element(),
    ]
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
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        if tab == PICTURE_TAB {
            picture_tab(app, cx)
        } else {
            shape_tab(app, cx)
        }
    }
}
