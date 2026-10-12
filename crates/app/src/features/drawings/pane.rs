//! The panes docked beside the sheet: Format Shape / Format Picture (fill,
//! line, size, properties, alt text), Alt Text and Selection. Each follows
//! the selected drawing and changes it at once, one undo step per change.

use super::geometry::normalise_angle;
use super::tabs::{self, dashes, in_units, length_field, set_side};
use crate::widgets::text_area::{AreaEvent, TextArea};
use super::{Cmd, Drawings, DrawingUi, edit, edit_drawing, edit_shape, is_picture, reset_picture, selected};
use crate::app::ExcelApp;
use crate::features::{self, Command, Cx, ObjectId, ZMove};
use crate::i18n::{lang, t, t_ctx};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, Checkbox, ColorButton, GroupBox, ListBox, NumberField, RadioGroup, Select, TabStrip, labeled,
};
use gpui::{
    AnyElement, Context, Entity, Focusable, MouseButton, MouseDownEvent, SharedString, Subscription, WeakEntity, Window,
    div, prelude::*, px,
};
use opensheet_engine::drawing::{Drawing, LineEnd, Placement, TextAnchor};
use opensheet_engine::style::Color;

pub(super) const FORMAT: &str = "drawing-format";
pub(super) const ALT_TEXT: &str = "drawing-alt-text";
pub(super) const SELECTION: &str = "drawing-selection";

/// What the panes keep between events.
#[derive(Default)]
pub(super) struct PaneState {
    format: Option<Entity<FormatPane>>,
    alt_text: Option<Entity<AltTextPane>>,
}

/// The selection is leaving a drawing: a description typed in a pane and
/// not confirmed yet (the box still has the keyboard) becomes its
/// description, before the pane turns to what is selected next.
pub(super) fn keep_descriptions(app: &mut ExcelApp, cx: &mut Cx) {
    let panes = &app.ext_mut::<DrawingUi>().panes;
    let typed = [
        panes.format.as_ref().and_then(|pane| pane.read(cx).typed_description(cx)),
        panes.alt_text.as_ref().and_then(|pane| pane.read(cx).typed_description(cx)),
    ];
    for (id, text) in typed.into_iter().flatten() {
        edit_drawing(app, id, Cmd::AltText.label(), cx, |drawing| drawing.alt_text = text);
    }
}

/// The two pages of the Format pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    /// "Fill & Line".
    Fill,
    /// "Size & Properties".
    Size,
}

/// A box of several lines for a pane; it scrolls past `height`.
fn description_area<V: 'static>(height: f32, cx: &mut Context<V>) -> Entity<TextArea> {
    cx.new(|cx| {
        let mut area = TextArea::field(cx);
        area.max_height = Some(height - 12.0);
        area
    })
}

/// The box in which a description is typed.
fn description_box(area: &Entity<TextArea>, height: f32) -> gpui::Div {
    let field = area.clone();
    div()
        .h(px(height))
        .w_full()
        .p(px(5.))
        .bg(c(0xFFFFFF))
        .border_1()
        .border_color(c(0xABABAB))
        .rounded(px(3.))
        .line_height(px(17.))
        .overflow_hidden()
        // A click below the text still lands in the box.
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
            window.focus(&field.read(cx).focus_handle(cx));
            field.update(cx, |field, cx| field.caret_to_point(event.position, cx));
        })
        .child(area.clone())
}

/// A text for a list or a group of radio buttons.
fn item(text: &'static str) -> SharedString {
    text.into()
}

fn title_for(app: &ExcelApp) -> &'static str {
    match selected(app) {
        Some(drawing) if is_picture(drawing) => t("Format Picture", "Format de l'image"),
        _ => t("Format Shape", "Format de la forme"),
    }
}

/// The names of the ends of a line, in the order of `LineEnd::ALL`.
fn line_end_names() -> Vec<SharedString> {
    LineEnd::ALL
        .iter()
        .map(|end| {
            item(match end {
                LineEnd::None => t_ctx("arrow", "None", "Aucune"),
                LineEnd::Triangle => t("Arrow", "Flèche"),
                LineEnd::Stealth => t("Stealth Arrow", "Flèche élancée"),
                LineEnd::Diamond => t("Diamond Arrow", "Flèche losange"),
                LineEnd::Oval => t("Oval Arrow", "Flèche ovale"),
                LineEnd::Open => t("Open Arrow", "Flèche ouverte"),
            })
        })
        .collect()
}

// ---------------------------------------------------------- the format pane

/// The fields of the pane that are typed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Height,
    Width,
    Rotation,
    LineWidth,
}

/// What the fields of the pane were last filled from.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    id: u32,
    width: f64,
    height: f64,
    rotation: f32,
    line_width: Option<f32>,
    alt_text: String,
}

struct FormatPane {
    app: WeakEntity<ExcelApp>,
    section: Section,
    height: Entity<TextInput>,
    width: Entity<TextInput>,
    rotation: Entity<TextInput>,
    line_width: Entity<TextInput>,
    alt_text: Entity<TextArea>,
    shown: Option<Shown>,
    _subscriptions: Vec<Subscription>,
}

impl FormatPane {
    fn new(app: Entity<ExcelApp>, section: Section, window: &mut Window, cx: &mut Context<Self>) -> FormatPane {
        let limit = 5000.0 / tabs::length_unit().1;
        let height = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let width = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let rotation = widgets::number_input(cx, None, -3600.0, 3600.0, 1.0);
        let line_width = widgets::number_input(cx, None, 0.0, 1584.0, 0.25);
        let alt_text = description_area(84.0, cx);
        let mut subscriptions = vec![
            // The pane shows the selected drawing as it is now.
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe(&alt_text, |this, _, event: &AreaEvent, cx| {
                if *event == AreaEvent::Blurred {
                    this.apply_alt_text(cx);
                }
            }),
        ];
        for (field, input) in [
            (Field::Height, &height),
            (Field::Width, &width),
            (Field::Rotation, &rotation),
            (Field::LineWidth, &line_width),
        ] {
            subscriptions.push(cx.subscribe_in(input, window, move |this, input, event: &InputEvent, _, cx| {
                this.on_field(field, input, event, cx)
            }));
        }
        FormatPane {
            app: app.downgrade(),
            section,
            height,
            width,
            rotation,
            line_width,
            alt_text,
            shown: None,
            _subscriptions: subscriptions,
        }
    }

    /// Change the selected drawing from a control of the pane.
    fn change(&self, label: &'static str, cx: &mut Context<Self>, change: impl FnOnce(&mut Drawing)) {
        self.app.update(cx, |app, cx| edit(app, label, cx, change)).ok();
    }

    /// The drawing the description box was filled from and what was typed
    /// there since, when it differs.
    fn typed_description(&self, cx: &gpui::App) -> Option<(u32, String)> {
        let shown = self.shown.as_ref()?;
        let typed = self.alt_text.read(cx).text();
        (shown.alt_text != typed).then(|| (shown.id, typed.to_string()))
    }

    fn apply_alt_text(&mut self, cx: &mut Context<Self>) {
        if let Some((id, typed)) = self.typed_description(cx) {
            self.app
                .update(cx, |app, cx| edit_drawing(app, id, Cmd::AltText.label(), cx, |drawing| drawing.alt_text = typed))
                .ok();
        }
    }

    /// A number was confirmed in a field (Return, Tab, leaving it) or
    /// stepped with its spinner.
    fn on_field(&mut self, field: Field, input: &Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        let confirmed = match event {
            InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => true,
            // The spinner leaves the whole number selected; typing does
            // not.
            InputEvent::Changed => {
                let input = input.read(cx);
                !input.text().is_empty() && input.selection() == (0..input.text().len())
            }
            _ => false,
        };
        let (Some(typed), Some(shown), true) = (input.read(cx).number(), self.shown.clone(), confirmed) else {
            return;
        };
        match field {
            Field::Height | Field::Width => {
                let height = field == Field::Height;
                let current = if height { shown.height } else { shown.width };
                if (typed - current).abs() > 0.004 && typed > 0.0 {
                    self.app.update(cx, |app, cx| set_side(app, height, typed, cx)).ok();
                }
            }
            Field::Rotation => {
                let angle = normalise_angle(typed as f32);
                if (angle - shown.rotation).abs() > 0.004 {
                    self.change(t("Rotate Object", "Faire pivoter l'objet"), cx, |drawing| drawing.rotation = angle);
                }
            }
            Field::LineWidth => {
                let points = typed as f32;
                if shown.line_width != Some(points) {
                    self.app.update(cx, |app, cx| tabs::set_outline(app, cx, |outline| outline.width = points)).ok();
                }
            }
        }
        // Whatever was typed, the fields show the drawing again.
        self.shown = None;
        cx.notify();
    }

    /// Fill the fields from the drawing when it changed, leaving alone the
    /// one being typed in.
    fn sync(&mut self, drawing: Option<(&Drawing, (f64, f64))>, window: &Window, cx: &mut Context<Self>) {
        let now = drawing.map(|(drawing, size)| Shown {
            id: drawing.id,
            width: in_units(size.0),
            height: in_units(size.1),
            rotation: drawing.rotation,
            line_width: drawing.as_shape().and_then(|shape| shape.outline).map(|outline| outline.width),
            alt_text: drawing.alt_text.clone(),
        });
        if now == self.shown {
            return;
        }
        let decimal = lang().locale().decimal_sep;
        let number = |value: Option<f64>| value.map(|v| widgets::number_text(v, decimal)).unwrap_or_default();
        let fields = [
            (&self.height, number(now.as_ref().map(|n| n.height))),
            (&self.width, number(now.as_ref().map(|n| n.width))),
            (&self.rotation, number(now.as_ref().map(|n| n.rotation as f64))),
            (&self.line_width, number(now.as_ref().and_then(|n| n.line_width).map(f64::from))),
        ];
        for (field, text) in fields {
            if !field.read(cx).focus_handle(cx).is_focused(window) {
                field.update(cx, |field, cx| field.set_text(text, cx));
            }
        }
        if !self.alt_text.read(cx).focus_handle(cx).is_focused(window) {
            let text = now.as_ref().map(|n| n.alt_text.clone()).unwrap_or_default();
            self.alt_text.update(cx, |area, cx| area.set_text(text, cx));
        }
        self.shown = now;
    }

    fn fill_and_line(&self, drawing: &Drawing, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(shape) = drawing.as_shape() else {
            return div().into_any_element();
        };
        let is_line = shape.preset.is_line();
        let outline = shape.outline;
        let dash_names: Vec<SharedString> = dashes().iter().map(|(_, name)| item(name)).collect();
        let dash = outline.and_then(|outline| dashes().iter().position(|(dash, _)| *dash == outline.dash));
        let end_index = |end: LineEnd| LineEnd::ALL.iter().position(|other| *other == end);
        let mut page = div().flex().flex_col().gap(px(14.));
        if !is_line {
            page = page.child(
                GroupBox::new(t_ctx("shape", "Fill", "Remplissage"))
                    .child(
                        RadioGroup::new(
                            "dw-fill-kind",
                            vec![item(t("No fill", "Aucun remplissage")), item(t("Solid fill", "Remplissage uni"))],
                            Some(shape.fill.is_some() as usize),
                        )
                        .disabled(locked)
                        .on_change(cx.listener(|this, choice: &usize, _, cx| {
                            let solid = *choice == 1;
                            this.app
                                .update(cx, |app, cx| {
                                    let now = selected(app).and_then(Drawing::as_shape).and_then(|shape| shape.fill);
                                    if solid != now.is_some() {
                                        tabs::set_fill(app, solid.then_some(Color(0x156082)), cx);
                                    }
                                })
                                .ok();
                        })),
                    )
                    .children(shape.fill.map(|fill| {
                        labeled(
                            t("Color", "Couleur"),
                            ColorButton::new("dw-fill-color", Some(Some(fill))).more(&self.app).on_pick(cx.listener(
                                |this, color: &Option<Color>, _, cx| {
                                    let color = *color;
                                    this.app.update(cx, |app, cx| tabs::set_fill(app, color, cx)).ok();
                                },
                            )),
                        )
                        .justify_between()
                    })),
            );
        }
        let mut line = GroupBox::new(t_ctx("shape", "Line", "Trait")).child(
            RadioGroup::new(
                "dw-line-kind",
                vec![item(t("No line", "Aucun trait")), item(t("Solid line", "Trait plein"))],
                Some(outline.is_some() as usize),
            )
            .disabled(locked)
            .on_change(cx.listener(|this, choice: &usize, _, cx| {
                let solid = *choice == 1;
                this.app
                    .update(cx, |app, cx| {
                        let now = selected(app).and_then(Drawing::as_shape).and_then(|shape| shape.outline);
                        if solid != now.is_some() {
                            tabs::set_outline_color(app, solid.then_some(Color::BLACK), cx);
                        }
                    })
                    .ok();
            })),
        );
        if let Some(outline) = outline {
            let set_end = |start: bool| {
                cx.listener(move |this: &mut FormatPane, choice: &usize, _: &mut Window, cx: &mut Context<FormatPane>| {
                    let Some(end) = LineEnd::ALL.get(*choice).copied() else {
                        return;
                    };
                    this.app
                        .update(cx, |app, cx| {
                            tabs::set_outline(app, cx, |outline| {
                                if start {
                                    outline.start = end;
                                } else {
                                    outline.end = end;
                                }
                            })
                        })
                        .ok();
                })
            };
            line = line
                .child(
                    labeled(
                        t("Color", "Couleur"),
                        ColorButton::new("dw-line-color", Some(Some(outline.color))).more(&self.app).on_pick(cx.listener(
                            |this, color: &Option<Color>, _, cx| {
                                if let Some(color) = *color {
                                    this.app.update(cx, |app, cx| tabs::set_outline_color(app, Some(color), cx)).ok();
                                }
                            },
                        )),
                    )
                    .justify_between(),
                )
                .child(
                    labeled(
                        t("Width", "Largeur"),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(NumberField::new("dw-line-width", &self.line_width).width(72.0))
                            .child("pt"),
                    )
                    .justify_between(),
                )
                .child(
                    labeled(
                        t("Dash type", "Type de tiret"),
                        Select::new("dw-dash", dash_names, dash)
                            .width(170.0)
                            .disabled(locked)
                            .on_change(cx.listener(|this, choice: &usize, _, cx| {
                                let Some((dash, _)) = dashes().get(*choice).copied() else {
                                    return;
                                };
                                this.app
                                    .update(cx, |app, cx| tabs::set_outline(app, cx, |outline| outline.dash = dash))
                                    .ok();
                            })),
                    )
                    .justify_between(),
                )
                // Only a line has ends.
                .child(
                    labeled(
                        t("Begin Arrow type", "Type de flèche initiale"),
                        Select::new("dw-begin", line_end_names(), end_index(outline.start))
                            .width(130.0)
                            .disabled(locked || !is_line)
                            .on_change(set_end(true)),
                    )
                    .justify_between(),
                )
                .child(
                    labeled(
                        t("End Arrow type", "Type de flèche finale"),
                        Select::new("dw-end", line_end_names(), end_index(outline.end))
                            .width(130.0)
                            .disabled(locked || !is_line)
                            .on_change(set_end(false)),
                    )
                    .justify_between(),
                );
        }
        page.child(line).into_any_element()
    }

    fn size_and_properties(&self, drawing: &Drawing, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let takes_text = drawing.as_shape().is_some_and(|shape| !shape.preset.is_line());
        let mut size = GroupBox::new(t("Size", "Taille"))
            .child(length_field("dw-pane-height", t("Height", "Hauteur"), &self.height))
            .child(length_field("dw-pane-width", t("Width", "Largeur"), &self.width))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(div().w(px(52.)).flex_none().text_size(px(11.5)).child(t("Rotation", "Rotation")))
                    .child(NumberField::new("dw-pane-rotation", &self.rotation).width(72.0))
                    .child("°"),
            )
            .child(
                Checkbox::new(
                    "dw-lock-aspect",
                    t("Lock aspect ratio", "Garder les proportions"),
                    drawing.lock_aspect,
                )
                .disabled(locked)
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    let on = *on;
                    this.change(t("Lock aspect ratio", "Garder les proportions"), cx, |drawing| drawing.lock_aspect = on);
                })),
            );
        if is_picture(drawing) {
            size = size.child(div().flex().child(Button::new("dw-reset", t("Reset", "Rétablir")).disabled(locked).on_click(
                cx.listener(|this, _, _, cx| {
                    this.app.update(cx, |app, cx| reset_picture(app, true, cx)).ok();
                }),
            )));
        }
        let placement = Placement::ALL.iter().position(|p| *p == drawing.placement);
        let properties = GroupBox::new(t("Properties", "Propriétés"))
            .child(
                RadioGroup::new(
                    "dw-placement",
                    vec![
                        item(t("Move and size with cells", "Déplacer et dimensionner avec les cellules")),
                        item(t("Move but don't size with cells", "Déplacer sans dimensionner avec les cellules")),
                        item(t("Don't move or size with cells", "Ne pas déplacer ou dimensionner avec les cellules")),
                    ],
                    placement,
                )
                .disabled(locked)
                .on_change(cx.listener(|this, choice: &usize, _, cx| {
                    if let Some(placement) = Placement::ALL.get(*choice).copied() {
                        this.change(t("Object Positioning", "Position de l'objet"), cx, |drawing| drawing.placement = placement);
                    }
                })),
            )
            .child(
                Checkbox::new("dw-print", t("Print object", "Imprimer l'objet"), drawing.print)
                    .disabled(locked)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.change(t("Print object", "Imprimer l'objet"), cx, |drawing| drawing.print = on);
                    })),
            )
            .child(
                Checkbox::new("dw-locked", t_ctx("object", "Locked", "Verrouillé"), drawing.locked)
                    .disabled(locked)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.change(t_ctx("object", "Locked", "Verrouillé"), cx, |drawing| drawing.locked = on);
                    })),
            );
        let mut page = div().flex().flex_col().gap(px(14.)).child(size).child(properties);
        if let (true, Some(shape)) = (takes_text, drawing.as_shape()) {
            let anchor = TextAnchor::ALL.iter().position(|a| *a == shape.anchor);
            page = page.child(
                GroupBox::new(t("Text Box", "Zone de texte")).child(
                    labeled(
                        t("Vertical alignment", "Alignement vertical"),
                        Select::new(
                            "dw-anchor",
                            vec![item(t("Top", "Haut")), item(t("Middle", "Milieu")), item(t("Bottom", "Bas"))],
                            anchor,
                        )
                        .width(110.0)
                        .disabled(locked)
                        .on_change(cx.listener(|this, choice: &usize, _, cx| {
                            let Some(anchor) = TextAnchor::ALL.get(*choice).copied() else {
                                return;
                            };
                            this.app
                                .update(cx, |app, cx| {
                                    edit_shape(app, t("Vertical alignment", "Alignement vertical"), cx, |shape| shape.anchor = anchor)
                                })
                                .ok();
                        })),
                    )
                    .justify_between(),
                ),
            );
        }
        page.child(
            GroupBox::new(t("Alt Text", "Texte de remplacement"))
                .child(div().text_color(c(theme::TEXT_MUTED)).child(t("Description", "Description")))
                .child(description_box(&self.alt_text, 84.0)),
        )
        .into_any_element()
    }
}

impl Render for FormatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The selected drawing as it is now, its size, and whether the
        // sheet's protection lets it be changed.
        let state = self.app.upgrade().and_then(|app| {
            let app = app.read(cx);
            let drawing = selected(app)?;
            let frame = app.wb.drawing_frame(app.sheet, drawing.id)?;
            Some((drawing.clone(), (frame.width, frame.height), !tabs::editable(app)))
        });
        self.sync(state.as_ref().map(|(drawing, size, _)| (drawing, *size)), window, cx);
        let Some((drawing, _, locked)) = state else {
            return div()
                .p(px(14.))
                .text_color(c(theme::TEXT_MUTED))
                .child(t(
                    "Select a picture or a shape to format it.",
                    "Sélectionnez une image ou une forme pour la mettre en forme.",
                ))
                .into_any_element();
        };
        // A picture has no fill or line of its own: one page only.
        let has_fill = drawing.as_shape().is_some();
        let section = if has_fill { self.section } else { Section::Size };
        let page = match section {
            Section::Fill => self.fill_and_line(&drawing, locked, cx),
            Section::Size => self.size_and_properties(&drawing, locked, cx),
        };
        div()
            .id("dw-format-pane")
            .size_full()
            .overflow_y_scroll()
            .px(px(12.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .when(has_fill, |d| {
                d.child(
                    TabStrip::new(
                        "dw-format-tabs",
                        vec![
                            item(t("Fill & Line", "Remplissage et contour")),
                            item(t("Size & Properties", "Taille et propriétés")),
                        ],
                        (section == Section::Size) as usize,
                    )
                    .on_change(cx.listener(|this, choice: &usize, _, cx| {
                        this.section = if *choice == 0 { Section::Fill } else { Section::Size };
                        cx.notify();
                    })),
                )
            })
            .child(page)
            .into_any_element()
    }
}

/// Open the Format pane on a page ("Format Shape…", the launcher of the
/// Size group, "More Rotation Options…").
pub(super) fn open_format(app: &mut ExcelApp, section: Section, window: &mut Window, cx: &mut Cx) {
    let title = title_for(app);
    let existing = app.ext_mut::<DrawingUi>().panes.format.clone().filter(|_| app.pane_open(FORMAT));
    let view = match existing {
        Some(view) => {
            view.update(cx, |pane, cx| {
                pane.section = section;
                cx.notify();
            });
            view
        }
        None => {
            let entity = cx.entity();
            let view = cx.new(|cx| FormatPane::new(entity, section, window, cx));
            app.ext_mut::<DrawingUi>().panes.format = Some(view.clone());
            view
        }
    };
    app.open_pane(FORMAT, title, view, cx);
}

/// The selection changed: the Format pane is named after what is selected
/// now.
pub(super) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    if !app.pane_open(FORMAT) || selected(app).is_none() {
        return;
    }
    let title = title_for(app);
    if let Some(index) = app.panes.iter().position(|pane| pane.id == FORMAT) {
        if app.panes[index].title != title {
            app.panes[index].title = title.into();
            cx.notify();
        }
    }
}

// ------------------------------------------------------------ the alt text

/// The Alt Text pane: the description read to people who cannot see the
/// picture.
struct AltTextPane {
    app: WeakEntity<ExcelApp>,
    text: Entity<TextArea>,
    /// The drawing and the description the box was last filled from.
    shown: Option<(u32, String)>,
    _subscriptions: Vec<Subscription>,
}

impl AltTextPane {
    /// The drawing the box was filled from and what was typed there
    /// since, when it differs.
    fn typed_description(&self, cx: &gpui::App) -> Option<(u32, String)> {
        let (id, shown) = self.shown.as_ref()?;
        let typed = self.text.read(cx).text();
        (shown != typed).then(|| (*id, typed.to_string()))
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        if let Some((id, typed)) = self.typed_description(cx) {
            self.app
                .update(cx, |app, cx| edit_drawing(app, id, Cmd::AltText.label(), cx, |drawing| drawing.alt_text = typed))
                .ok();
        }
    }
}

impl Render for AltTextPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = self.app.upgrade().and_then(|app| {
            let drawing = selected(app.read(cx))?;
            Some((drawing.id, drawing.alt_text.clone()))
        });
        // The box follows the drawing, except while its own description is
        // being typed.
        let typing = self.text.read(cx).focus_handle(cx).is_focused(window);
        let same_drawing = matches!((&now, &self.shown), (Some((a, _)), Some((b, _))) if a == b);
        if now != self.shown && !(typing && same_drawing) {
            let text = now.as_ref().map(|(_, text)| text.clone()).unwrap_or_default();
            self.text.update(cx, |area, cx| area.set_text(text, cx));
            self.shown = now.clone();
        }
        let mut pane = div().p(px(12.)).flex().flex_col().gap(px(8.)).line_height(px(17.));
        if now.is_none() {
            return pane
                .text_color(c(theme::TEXT_MUTED))
                .child(t(
                    "Select a picture or a shape to describe it.",
                    "Sélectionnez une image ou une forme pour la décrire.",
                ))
                .into_any_element();
        }
        pane = pane
            .child(t(
                "How would you describe this object and its context to someone who is blind or low vision?",
                "Comment décririez-vous cet objet et son contexte à une personne non voyante ?",
            ))
            .child(div().text_color(c(theme::TEXT_MUTED)).child(t(
                "(1-2 detailed sentences recommended)",
                "(1 à 2 phrases détaillées recommandées)",
            )))
            .child(description_box(&self.text, 140.0));
        pane.into_any_element()
    }
}

/// Open the Alt Text pane with the keyboard in its box.
pub(super) fn open_alt_text(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let entity = cx.entity();
    let view = cx.new(|cx| {
        let text = description_area(140.0, cx);
        let subscriptions = vec![
            cx.observe(&entity, |_, _, cx| cx.notify()),
            cx.subscribe(&text, |this: &mut AltTextPane, _, event: &AreaEvent, cx| {
                if *event == AreaEvent::Blurred {
                    this.apply(cx);
                }
            }),
        ];
        AltTextPane {
            app: entity.downgrade(),
            text,
            shown: None,
            _subscriptions: subscriptions,
        }
    });
    window.focus(&view.read(cx).text.read(cx).focus_handle(cx));
    app.ext_mut::<DrawingUi>().panes.alt_text = Some(view.clone());
    app.open_pane(ALT_TEXT, Cmd::AltText.label(), view, cx);
}

// ----------------------------------------------------------- the selection

/// The Selection pane: every floating object of the sheet, the one on top
/// first.
struct SelectionPane {
    app: WeakEntity<ExcelApp>,
}

/// The name of an object in the Selection pane: a drawing has one; the
/// objects of other features are numbered.
fn object_name(app: &ExcelApp, id: ObjectId) -> SharedString {
    if id.owner == features::index_of(&Drawings) {
        if let Some(drawing) = app.wb.drawing(app.sheet, id.id) {
            return drawing.name.clone().into();
        }
    }
    format!("{} {}", t("Chart", "Graphique"), id.id).into()
}

impl Render for SelectionPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (objects, selected_row) = self
            .app
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                let mut objects: Vec<(ObjectId, SharedString)> = features::objects(app, app.sheet)
                    .into_iter()
                    .map(|(id, _)| (id, object_name(app, id)))
                    .collect();
                objects.reverse();
                let row = app.object_sel.and_then(|sel| objects.iter().position(|(id, _)| *id == sel));
                (objects, row)
            })
            .unwrap_or_default();
        let ids: Vec<ObjectId> = objects.iter().map(|(id, _)| *id).collect();
        let names: Vec<SharedString> = objects.into_iter().map(|(_, name)| name).collect();
        let reorder = |id: &'static str, label: &'static str, mv: ZMove, cx: &mut Context<Self>| {
            Button::new(id, label)
                .disabled(selected_row.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.app.update(cx, |app, cx| app.reorder_object(mv, cx)).ok();
                }))
        };
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(reorder("dw-sel-forward", Cmd::BringForward.label(), ZMove::Forward, cx))
                    .child(reorder("dw-sel-backward", Cmd::SendBackward.label(), ZMove::Backward, cx)),
            )
            .child(
                div().flex().child(
                    ListBox::new("dw-selection-list", names, selected_row)
                        .height(260.0)
                        .on_change(cx.listener(move |this, row: &usize, _, cx| {
                            let Some(id) = ids.get(*row).copied() else {
                                return;
                            };
                            this.app.update(cx, |app, cx| app.select_object(Some(id), cx)).ok();
                        })),
                ),
            )
    }
}

/// Show or hide the Selection pane.
pub(super) fn toggle_selection(app: &mut ExcelApp, cx: &mut Cx) {
    if app.pane_open(SELECTION) {
        app.close_pane(SELECTION, cx);
        return;
    }
    let entity = cx.entity();
    let view = cx.new(|cx| {
        cx.observe(&entity, |_, _, cx| cx.notify()).detach();
        SelectionPane {
            app: entity.downgrade(),
        }
    });
    app.open_pane(SELECTION, t("Selection", "Sélection"), view, cx);
}
