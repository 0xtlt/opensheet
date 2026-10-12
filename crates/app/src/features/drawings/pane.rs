//! The panes docked beside the sheet: Format Shape / Format Picture (fill,
//! line, effects, size, properties, alt text, and for a picture its
//! corrections, colours and transparency), Alt Text and Selection. Each
//! follows the selected drawing and changes it at once, one undo step per
//! change; with several drawings selected the Format pane shows the one
//! selected last and changes them all.

use super::geometry::normalise_angle;
use super::tabs::{self, dashes, in_units, length_field, set_side};
use crate::widgets::text_area::{AreaEvent, TextArea};
use super::adjust;
use super::{Cmd, DrawingUi, edit, edit_drawing, edit_shape, is_picture, reset_picture, selected};
use crate::app::ExcelApp;
use crate::features::{self, Command, Cx, Intent, ObjectId, ZMove};
use crate::objects::{engine_ref, object_of};
use crate::i18n::{lang, t, t_ctx};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, Checkbox, ColorButton, GroupBox, NumberField, RadioGroup, Select, TabStrip, labeled,
};
use gpui::{
    AnyElement, Context, Entity, Focusable, MouseButton, MouseDownEvent, SharedString, Subscription, WeakEntity, Window,
    div, prelude::*, px,
};
use opensheet_engine::drawing::{
    Drawing, Glow, LineEnd, NEUTRAL_TEMPERATURE, ObjectRef, Placement, Recolor, SEPIA, ShadowPreset, TextAnchor,
};
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

/// The pages of the Format pane. A shape has the first three, a picture
/// the last three, a group Effects and Size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    /// "Fill & Line".
    Fill,
    /// "Effects": shadow, glow, soft edges.
    Effects,
    /// "Size & Properties".
    Size,
    /// "Picture": corrections, colour, transparency.
    Picture,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Fill => t("Fill & Line", "Remplissage et contour"),
            Section::Effects => t("Effects", "Effets"),
            Section::Size => t("Size & Properties", "Taille et propriétés"),
            Section::Picture => t("Picture", "Image"),
        }
    }

    /// The pages Excel shows for a kind of drawing.
    fn of(drawing: &Drawing) -> Vec<Section> {
        if is_picture(drawing) {
            vec![Section::Effects, Section::Size, Section::Picture]
        } else if drawing.as_shape().is_some() {
            vec![Section::Fill, Section::Effects, Section::Size]
        } else {
            vec![Section::Effects, Section::Size]
        }
    }
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
    /// One of the numbers of the Effects and Picture pages.
    Look(Look),
}

/// The numbers of the Effects and Picture pages: each is read from the
/// drawing and written back the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Look {
    ShadowTransparency,
    ShadowSize,
    ShadowBlur,
    ShadowAngle,
    ShadowDistance,
    GlowSize,
    GlowTransparency,
    SoftEdge,
    Brightness,
    Contrast,
    Saturation,
    Temperature,
    Transparency,
}

impl Look {
    const ALL: [Look; 13] = [
        Look::ShadowTransparency,
        Look::ShadowSize,
        Look::ShadowBlur,
        Look::ShadowAngle,
        Look::ShadowDistance,
        Look::GlowSize,
        Look::GlowTransparency,
        Look::SoftEdge,
        Look::Brightness,
        Look::Contrast,
        Look::Saturation,
        Look::Temperature,
        Look::Transparency,
    ];

    /// Smallest and largest value, the step of the spinner, and the unit
    /// shown after the field.
    fn range(self) -> (f64, f64, f64, &'static str) {
        match self {
            Look::ShadowTransparency | Look::GlowTransparency | Look::Transparency => (0.0, 100.0, 1.0, "%"),
            Look::ShadowSize => (1.0, 200.0, 1.0, "%"),
            Look::ShadowBlur | Look::SoftEdge => (0.0, 100.0, 1.0, "pt"),
            Look::ShadowAngle => (0.0, 359.9, 1.0, "°"),
            Look::ShadowDistance => (0.0, 200.0, 1.0, "pt"),
            Look::GlowSize => (0.0, 150.0, 1.0, "pt"),
            Look::Brightness | Look::Contrast => (-100.0, 100.0, 1.0, "%"),
            Look::Saturation => (0.0, 400.0, 1.0, "%"),
            Look::Temperature => (1500.0, 11500.0, 100.0, "K"),
        }
    }

    /// The number as the field shows it. A drawing without shadow or
    /// glow shows what the first change would start from.
    pub(super) fn of(self, drawing: &Drawing) -> Option<f64> {
        let shadow = adjust::shadow_of(drawing);
        let glow = drawing.effects.glow;
        let picture = drawing.as_picture().map(|picture| picture.adjust);
        let value = match self {
            Look::ShadowTransparency => shadow.transparency * 100.0,
            Look::ShadowSize => shadow.size * 100.0,
            Look::ShadowBlur => shadow.blur,
            Look::ShadowAngle => shadow.angle,
            Look::ShadowDistance => shadow.distance,
            Look::GlowSize => glow.map_or(0.0, |glow| glow.radius),
            Look::GlowTransparency => glow.map_or(60.0, |glow| glow.transparency * 100.0),
            Look::SoftEdge => drawing.effects.soft_edge,
            Look::Brightness => picture?.brightness * 100.0,
            Look::Contrast => picture?.contrast * 100.0,
            Look::Saturation => picture?.saturation * 100.0,
            Look::Temperature => picture?.temperature,
            Look::Transparency => picture?.transparency * 100.0,
        };
        // As typed: no more decimals than a field shows.
        Some((value as f64 * 100.0).round() / 100.0)
    }

    /// Write a number typed in the field to the selected drawings.
    pub(super) fn set(self, app: &mut ExcelApp, value: f64, cx: &mut Cx) {
        let (min, max, _, _) = self.range();
        let value = value.clamp(min, max) as f32;
        let glow_label = t("Glow", "Lumière");
        match self {
            Look::ShadowTransparency => adjust::set_shadow(app, cx, |shadow| shadow.transparency = value / 100.0),
            Look::ShadowSize => adjust::set_shadow(app, cx, |shadow| shadow.size = value / 100.0),
            Look::ShadowBlur => adjust::set_shadow(app, cx, |shadow| shadow.blur = value),
            Look::ShadowAngle => adjust::set_shadow(app, cx, |shadow| shadow.angle = value),
            Look::ShadowDistance => adjust::set_shadow(app, cx, |shadow| shadow.distance = value),
            // No size is no glow.
            Look::GlowSize => adjust::set_effects(app, glow_label, cx, |effects| {
                effects.glow = (value > 0.0).then(|| Glow {
                    radius: value,
                    ..effects.glow.unwrap_or(DEFAULT_GLOW)
                });
            }),
            Look::GlowTransparency => adjust::set_effects(app, glow_label, cx, |effects| {
                if let Some(glow) = &mut effects.glow {
                    glow.transparency = value / 100.0;
                }
            }),
            Look::SoftEdge => {
                adjust::set_effects(app, t("Soft Edges", "Bordures adoucies"), cx, |effects| effects.soft_edge = value)
            }
            Look::Brightness => {
                adjust::adjust(app, t("Picture Corrections", "Corrections de l'image"), cx, |a| a.brightness = value / 100.0)
            }
            Look::Contrast => {
                adjust::adjust(app, t("Picture Corrections", "Corrections de l'image"), cx, |a| a.contrast = value / 100.0)
            }
            Look::Saturation => adjust::adjust(app, t("Picture Color", "Couleur de l'image"), cx, |a| a.saturation = value / 100.0),
            Look::Temperature => adjust::adjust(app, t("Picture Color", "Couleur de l'image"), cx, |a| a.temperature = value),
            Look::Transparency => {
                adjust::adjust(app, t("Picture Transparency", "Transparence d'image"), cx, |a| a.transparency = value / 100.0)
            }
        }
    }
}

/// The glow a size typed in the pane starts from.
const DEFAULT_GLOW: Glow = Glow {
    color: Color(0x156082),
    transparency: 0.6,
    radius: 0.0,
};

/// What the fields of the pane were last filled from.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    id: u32,
    width: f64,
    height: f64,
    rotation: f32,
    line_width: Option<f32>,
    alt_text: String,
    /// The numbers of the Effects and Picture pages, in the order of
    /// `Look::ALL`.
    looks: Vec<Option<f64>>,
}

struct FormatPane {
    app: WeakEntity<ExcelApp>,
    section: Section,
    height: Entity<TextInput>,
    width: Entity<TextInput>,
    rotation: Entity<TextInput>,
    line_width: Entity<TextInput>,
    /// A field for each of `Look::ALL`.
    looks: Vec<Entity<TextInput>>,
    alt_text: Entity<TextArea>,
    shown: Option<Shown>,
    _subscriptions: Vec<Subscription>,
}

impl FormatPane {
    fn new(app: Entity<ExcelApp>, section: Section, cx: &mut Context<Self>) -> FormatPane {
        let limit = 5000.0 / tabs::length_unit().1;
        let height = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let width = widgets::number_input(cx, None, 0.0, limit, 0.1);
        let rotation = widgets::number_input(cx, None, -3600.0, 3600.0, 1.0);
        let line_width = widgets::number_input(cx, None, 0.0, 1584.0, 0.25);
        let looks: Vec<Entity<TextInput>> = Look::ALL
            .iter()
            .map(|look| {
                let (min, max, step, _) = look.range();
                widgets::number_input(cx, None, min, max, step)
            })
            .collect();
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
        let mut fields = vec![
            (Field::Height, &height),
            (Field::Width, &width),
            (Field::Rotation, &rotation),
            (Field::LineWidth, &line_width),
        ];
        fields.extend(Look::ALL.iter().zip(&looks).map(|(look, input)| (Field::Look(*look), input)));
        for (field, input) in fields {
            subscriptions.push(cx.subscribe(input, move |this, input, event: &InputEvent, cx| {
                this.on_field(field, &input, event, cx)
            }));
        }
        FormatPane {
            app: app.downgrade(),
            section,
            height,
            width,
            rotation,
            line_width,
            looks,
            alt_text,
            shown: None,
            _subscriptions: subscriptions,
        }
    }

    /// Change the selected drawings from a control of the pane.
    fn change(&self, label: &'static str, cx: &mut Context<Self>, change: impl Fn(&mut Drawing)) {
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
            Field::Look(look) => {
                let was = Look::ALL.iter().position(|other| *other == look).and_then(|i| shown.looks[i]);
                if was.is_some_and(|was| (typed - was).abs() > 0.004) {
                    self.app.update(cx, |app, cx| look.set(app, typed, cx)).ok();
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
            looks: Look::ALL.iter().map(|look| look.of(drawing)).collect(),
        });
        if now == self.shown {
            return;
        }
        let decimal = lang().locale().decimal_sep;
        let number = |value: Option<f64>| value.map(|v| widgets::number_text(v, decimal)).unwrap_or_default();
        let mut fields = vec![
            (&self.height, number(now.as_ref().map(|n| n.height))),
            (&self.width, number(now.as_ref().map(|n| n.width))),
            (&self.rotation, number(now.as_ref().map(|n| n.rotation as f64))),
            (&self.line_width, number(now.as_ref().and_then(|n| n.line_width).map(f64::from))),
        ];
        for (i, input) in self.looks.iter().enumerate() {
            fields.push((input, number(now.as_ref().and_then(|n| n.looks[i]))));
        }
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

    /// A number of the Effects or Picture page with its label and unit.
    fn look_field(&self, look: Look, label: &'static str) -> gpui::Div {
        let index = Look::ALL.iter().position(|other| *other == look).unwrap_or(0);
        let id: SharedString = format!("dw-look-{index}").into();
        labeled(
            label,
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(NumberField::new(id, &self.looks[index]).width(72.0))
                .child(div().w(px(16.)).child(look.range().3)),
        )
        .justify_between()
    }

    /// "Effects": the shadow with all its numbers, the glow, and for a
    /// picture its soft edges.
    fn effects(&self, drawing: &Drawing, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let shadow = drawing.effects.shadow;
        let mut presets = vec![item(t("No Shadow", "Aucune ombre"))];
        let locale = lang().locale();
        presets.extend(ShadowPreset::ALL.iter().map(|preset| item(preset.name(locale))));
        // The preset the shadow is, the first one for a shadow changed
        // by hand.
        let preset = match shadow {
            None => 0,
            Some(shadow) => ShadowPreset::ALL
                .iter()
                .position(|preset| preset.shadow().angle == shadow.angle && (preset.shadow().distance == 0.0) == (shadow.distance == 0.0))
                .map_or(1, |i| i + 1),
        };
        let shadow_box = GroupBox::new(t("Shadow", "Ombre"))
            .child(
                labeled(
                    t("Presets", "Présélections"),
                    Select::new("dw-shadow-preset", presets, Some(preset))
                        .width(190.0)
                        .disabled(locked)
                        .on_change(cx.listener(|this, choice: &usize, _, cx| {
                            let made = choice.checked_sub(1).and_then(|i| ShadowPreset::ALL.get(i)).map(|preset| preset.shadow());
                            this.app
                                .update(cx, |app, cx| adjust::set_effects(app, t("Shadow", "Ombre"), cx, |effects| effects.shadow = made))
                                .ok();
                        })),
                )
                .justify_between(),
            )
            .child(
                labeled(
                    t("Color", "Couleur"),
                    ColorButton::new("dw-shadow-color", Some(Some(adjust::shadow_of(drawing).color))).more(&self.app).on_pick(
                        cx.listener(|this, color: &Option<Color>, _, cx| {
                            if let Some(color) = *color {
                                this.app.update(cx, |app, cx| adjust::set_shadow(app, cx, |shadow| shadow.color = color)).ok();
                            }
                        }),
                    ),
                )
                .justify_between(),
            )
            .child(self.look_field(Look::ShadowTransparency, t("Transparency", "Transparence")))
            .child(self.look_field(Look::ShadowSize, t("Size", "Taille")))
            .child(self.look_field(Look::ShadowBlur, t("Blur", "Flou")))
            .child(self.look_field(Look::ShadowAngle, t("Angle", "Angle")))
            .child(self.look_field(Look::ShadowDistance, t("Distance", "Distance")));
        let glow = drawing.effects.glow.unwrap_or(DEFAULT_GLOW);
        let glow_box = GroupBox::new(t("Glow", "Lumière"))
            .child(
                labeled(
                    t("Color", "Couleur"),
                    ColorButton::new("dw-glow-color", Some(Some(glow.color))).more(&self.app).on_pick(cx.listener(
                        |this, color: &Option<Color>, _, cx| {
                            let Some(color) = *color else {
                                return;
                            };
                            this.app
                                .update(cx, |app, cx| {
                                    adjust::set_effects(app, t("Glow", "Lumière"), cx, |effects| {
                                        // A colour for a drawing without
                                        // glow gives it Excel's smallest.
                                        let glow = effects.glow.get_or_insert(Glow { radius: 5.0, ..DEFAULT_GLOW });
                                        glow.color = color;
                                    })
                                })
                                .ok();
                        },
                    )),
                )
                .justify_between(),
            )
            .child(self.look_field(Look::GlowSize, t("Size", "Taille")))
            .child(self.look_field(Look::GlowTransparency, t("Transparency", "Transparence")));
        let mut page = div().flex().flex_col().gap(px(14.)).child(shadow_box).child(glow_box);
        if is_picture(drawing) {
            page = page.child(GroupBox::new(t("Soft Edges", "Bordures adoucies")).child(self.look_field(Look::SoftEdge, t("Size", "Taille"))));
        }
        page.into_any_element()
    }

    /// "Picture": corrections, colour and transparency of a picture.
    fn picture(&self, drawing: &Drawing, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(adjust) = drawing.as_picture().map(|picture| picture.adjust) else {
            return div().into_any_element();
        };
        let reset = |id: &'static str, cx: &mut Context<Self>, undo: fn(&mut opensheet_engine::drawing::PictureAdjust), label: &'static str| {
            div().flex().child(Button::new(id, t("Reset", "Rétablir")).disabled(locked).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.app.update(cx, |app, cx| adjust::adjust(app, label, cx, undo)).ok();
                },
            )))
        };
        let corrections_label = t("Picture Corrections", "Corrections de l'image");
        let color_label = t("Picture Color", "Couleur de l'image");
        // The recolourings of the pane: the plain ones of the gallery.
        let recolors = [
            (t("No Recolor", "Aucune recolorisation"), Recolor::None),
            (t("Grayscale", "Nuances de gris"), Recolor::Grayscale),
            (t("Sepia", "Sépia"), Recolor::Duotone(SEPIA.0, SEPIA.1)),
            (t("Washout", "Estompé"), Recolor::Washout),
            (t("Black and White", "Noir et blanc"), Recolor::BlackAndWhite(0.5)),
        ];
        let recolor = recolors.iter().position(|(_, recolor)| *recolor == adjust.recolor);
        let corrections = GroupBox::new(corrections_label)
            .child(self.look_field(Look::Brightness, t("Brightness", "Luminosité")))
            .child(self.look_field(Look::Contrast, t("Contrast", "Contraste")))
            .child(reset(
                "dw-reset-corrections",
                cx,
                |adjust| {
                    adjust.brightness = 0.0;
                    adjust.contrast = 0.0;
                },
                corrections_label,
            ));
        let color = GroupBox::new(color_label)
            .child(self.look_field(Look::Saturation, t("Saturation", "Saturation")))
            .child(self.look_field(Look::Temperature, t("Temperature", "Température")))
            .child(
                labeled(
                    t("Recolor", "Recolorier"),
                    Select::new("dw-recolor", recolors.iter().map(|(name, _)| item(name)).collect(), recolor)
                        .width(170.0)
                        .disabled(locked)
                        .on_change(cx.listener(move |this, choice: &usize, _, cx| {
                            let Some((_, recolor)) = recolors.get(*choice).copied() else {
                                return;
                            };
                            this.app
                                .update(cx, |app, cx| adjust::adjust(app, color_label, cx, |adjust| adjust.recolor = recolor))
                                .ok();
                        })),
                )
                .justify_between(),
            )
            .child(reset(
                "dw-reset-color",
                cx,
                |adjust| {
                    adjust.saturation = 1.0;
                    adjust.temperature = NEUTRAL_TEMPERATURE;
                    adjust.recolor = Recolor::None;
                },
                color_label,
            ));
        let transparency = GroupBox::new(t("Picture Transparency", "Transparence d'image"))
            .child(self.look_field(Look::Transparency, t("Transparency", "Transparence")));
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(corrections)
            .child(color)
            .child(transparency)
            .into_any_element()
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
        // The pages of its kind; the one last shown when it has it.
        let sections = Section::of(&drawing);
        let section = if sections.contains(&self.section) { self.section } else { Section::Size };
        let page = match section {
            Section::Fill => self.fill_and_line(&drawing, locked, cx),
            Section::Effects => self.effects(&drawing, locked, cx),
            Section::Size => self.size_and_properties(&drawing, locked, cx),
            Section::Picture => self.picture(&drawing, locked, cx),
        };
        let titles: Vec<SharedString> = sections.iter().map(|section| item(section.title())).collect();
        let shown = sections.iter().position(|other| *other == section).unwrap_or(0);
        div()
            .id("dw-format-pane")
            .size_full()
            .overflow_y_scroll()
            .px(px(12.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                TabStrip::new("dw-format-tabs", titles, shown).on_change(cx.listener(move |this, choice: &usize, _, cx| {
                    if let Some(section) = sections.get(*choice) {
                        this.section = *section;
                        cx.notify();
                    }
                })),
            )
            .child(page)
            .into_any_element()
    }
}

/// Open the Format pane on a page ("Format Shape…", the launcher of the
/// Size group, "More Rotation Options…").
pub(super) fn open_format(app: &mut ExcelApp, section: Section, _window: &mut Window, cx: &mut Cx) {
    show_format(app, section, cx);
}

fn show_format(app: &mut ExcelApp, section: Section, cx: &mut Cx) {
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
            let view = cx.new(|cx| FormatPane::new(entity, section, cx));
            app.ext_mut::<DrawingUi>().panes.format = Some(view.clone());
            view
        }
    };
    // Wide enough for its longest label ("Don't move or size with cells").
    app.pane_width = app.pane_width.max(356.0);
    app.open_pane(FORMAT, title, view, cx);
}

/// The selection changed: the Format pane is named after what is selected
/// now. There is one Format pane: the one of a chart that was open turns
/// into this one when a picture or a shape is selected.
pub(super) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(drawing) = selected(app) else {
        return;
    };
    if app.pane_open(features::chart_format::PANE) {
        let section = Section::of(drawing)[0];
        app.close_pane(features::chart_format::PANE, cx);
        show_format(app, section, cx);
        return;
    }
    if !app.pane_open(FORMAT) {
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

/// A row of the Selection pane.
#[derive(Clone)]
pub(super) struct Row {
    pub(super) id: ObjectId,
    /// The chart, picture or shape of the engine it is; the objects of
    /// other features have no name or place of their own to change.
    object: Option<ObjectRef>,
    pub(super) name: SharedString,
    pub(super) hidden: bool,
    pub(super) selected: bool,
}

/// A row of the Selection pane being dragged to another place.
#[derive(Clone)]
struct DraggedRow {
    object: ObjectRef,
    name: SharedString,
}

/// The name that follows the pointer while a row is dragged.
struct DragName(SharedString);

impl Render for DragName {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(ROW_H))
            .px(px(8.))
            .flex()
            .items_center()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::ACCENT))
            .rounded(px(2.))
            .text_size(px(12.))
            .child(self.0.clone())
    }
}

const ROW_H: f32 = 24.0;

/// The Selection pane: every floating object of the sheet, the one on top
/// first, with its name (a double click renames it) and whether it shows.
/// A click selects, with Shift or ⌘ it adds to the selection; a row is
/// dragged to another place in the stacking order.
struct SelectionPane {
    app: WeakEntity<ExcelApp>,
    /// The object being renamed and the field its name is typed in.
    rename: Option<(ObjectRef, Entity<TextInput>)>,
    _subscriptions: Vec<Subscription>,
}

/// The rows of the Selection pane, topmost first: hidden objects too.
pub(super) fn selection_rows(app: &ExcelApp) -> Vec<Row> {
    let sheet = app.sheet;
    let mut rows: Vec<Row> = Vec::new();
    // The objects of other features lie above charts and drawings.
    for (id, _) in features::objects(app, sheet).into_iter().rev() {
        if engine_ref(id).is_none() {
            rows.push(Row {
                id,
                object: None,
                name: format!("{} {}", t("Object", "Objet"), id.id).into(),
                hidden: false,
                selected: app.is_object_selected(id),
            });
        }
    }
    for object in app.wb.object_stack(sheet).into_iter().rev() {
        let id = object_of(object);
        rows.push(Row {
            id,
            object: Some(object),
            name: app.wb.object_name(sheet, object).unwrap_or_default().to_string().into(),
            hidden: app.wb.object_hidden(sheet, object).unwrap_or(false),
            selected: app.is_object_selected(id),
        });
    }
    rows
}

/// Show or hide objects of the current sheet, as one undo step; those
/// that are hidden leave the selection.
pub(super) fn set_hidden(app: &mut ExcelApp, objects: &[ObjectRef], hidden: bool, cx: &mut Cx) {
    if !objects.iter().all(|object| app.may(Intent::EditObject(Some(object_of(*object))), cx)) {
        return;
    }
    if hidden {
        let kept: Vec<ObjectId> = app
            .selected_objects()
            .into_iter()
            .filter(|id| !engine_ref(*id).is_some_and(|object| objects.contains(&object)))
            .collect();
        app.select_objects(&kept, cx);
    }
    if app.wb.set_objects_hidden(app.sheet, objects, hidden) {
        app.commit(if hidden { t("Hide Object", "Masquer l'objet") } else { t("Show Object", "Afficher l'objet") }, cx);
    }
}

/// Put the object `moved` where `target` is in the Selection pane: above
/// it when it comes from below, under it when it comes from above.
pub(super) fn move_in_stack(app: &mut ExcelApp, moved: ObjectRef, target: ObjectRef, cx: &mut Cx) {
    if moved == target || !app.may(Intent::EditObject(Some(object_of(moved))), cx) {
        return;
    }
    let mut order = app.wb.object_stack(app.sheet);
    let (Some(from), Some(to)) = (order.iter().position(|o| *o == moved), order.iter().position(|o| *o == target)) else {
        return;
    };
    order.remove(from);
    order.insert(to, moved);
    if app.wb.restack_objects(app.sheet, &order) {
        app.commit(t("Change Order", "Changer l'ordre"), cx);
    }
}

impl SelectionPane {
    fn begin_rename(&mut self, row: &Row, window: &mut Window, cx: &mut Context<Self>) {
        let Some(object) = row.object else {
            return;
        };
        let field = widgets::input(cx, row.name.to_string());
        field.update(cx, |field, cx| field.select_all_text(cx));
        window.focus(&field.read(cx).focus_handle(cx));
        let subscription = cx.subscribe_in(&field, window, |this, _, event: &InputEvent, _, cx| match event {
            InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => this.end_rename(true, cx),
            InputEvent::Escape => this.end_rename(false, cx),
            _ => {}
        });
        self.rename = Some((object, field));
        self._subscriptions.truncate(1);
        self._subscriptions.push(subscription);
        cx.notify();
    }

    fn end_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some((object, field)) = self.rename.take() else {
            return;
        };
        let name = field.read(cx).text().to_string();
        if keep {
            self.app
                .update(cx, |app, cx| {
                    if app.may(Intent::EditObject(Some(object_of(object))), cx) && app.wb.rename_object(app.sheet, object, &name) {
                        app.commit(t("Rename Object", "Renommer l'objet"), cx);
                    }
                })
                .ok();
        }
        cx.notify();
    }

    fn row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let renaming = self.rename.as_ref().filter(|(object, _)| Some(*object) == row.object);
        let name: AnyElement = match renaming {
            Some((_, field)) => widgets::TextField::new(field).into_any_element(),
            None => div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .when(row.hidden, |d| d.text_color(c(theme::TEXT_MUTED)))
                .child(row.name.clone())
                .into_any_element(),
        };
        let clicked = row.clone();
        let mut line = div()
            .id(("dw-sel-row", index))
            .h(px(ROW_H))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(3.))
            .when(row.selected, |d| d.bg(c(theme::ACCENT_LIGHT)))
            .when(!row.selected, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .child(div().flex_1().overflow_hidden().child(name))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.begin_rename(&clicked, window, cx);
                    return;
                }
                // What does not show cannot be selected.
                if clicked.hidden {
                    return;
                }
                let extend = event.modifiers().shift || event.modifiers().platform;
                this.app
                    .update(cx, |app, cx| {
                        app.click_object(clicked.id, extend, cx);
                    })
                    .ok();
            }));
        if let Some(object) = row.object {
            let hidden = row.hidden;
            let dragged = DraggedRow {
                object,
                name: row.name.clone(),
            };
            line = line
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| DragName(dragged.name.clone())))
                .drag_over::<DraggedRow>(|style, _, _, _| style.bg(c(theme::HOVER)))
                .on_drop(cx.listener(move |this, dragged: &DraggedRow, _, cx| {
                    let moved = dragged.object;
                    this.app.update(cx, |app, cx| move_in_stack(app, moved, object, cx)).ok();
                }))
                .child(
                    div()
                        .id(("dw-sel-eye", index))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .hover(|s| s.bg(c(theme::PRESSED)))
                        .child(crate::view::icon("eye", 14.0, if hidden { theme::TEXT_DISABLED } else { theme::TEXT }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.app.update(cx, |app, cx| set_hidden(app, &[object], !hidden, cx)).ok();
                        })),
                );
        }
        line.into_any_element()
    }
}

impl Render for SelectionPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.app.upgrade().map(|app| selection_rows(app.read(cx))).unwrap_or_default();
        let any_selected = rows.iter().any(|row| row.selected);
        let all: Vec<ObjectRef> = rows.iter().filter_map(|row| row.object).collect();
        let reorder = |id: &'static str, icon: &'static str, mv: ZMove, cx: &mut Context<Self>| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(c(theme::BORDER))
                .when(any_selected, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(crate::view::icon(icon, 14.0, if any_selected { theme::TEXT } else { theme::TEXT_DISABLED }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.app.update(cx, |app, cx| app.reorder_object(mv, cx)).ok();
                }))
        };
        let show = |id: &'static str, label: &'static str, hidden: bool, cx: &mut Context<Self>| {
            let all = all.clone();
            Button::new(id, label).on_click(cx.listener(move |this, _, _, cx| {
                this.app.update(cx, |app, cx| set_hidden(app, &all, hidden, cx)).ok();
            }))
        };
        let list: Vec<AnyElement> = rows.iter().enumerate().map(|(index, row)| self.row(index, row, cx)).collect();
        div()
            .p(px(12.))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(show("dw-sel-show-all", t("Show All", "Afficher tout"), false, cx))
                    .child(show("dw-sel-hide-all", t("Hide All", "Masquer tout"), true, cx))
                    .child(div().flex_1())
                    .child(reorder("dw-sel-forward", "chevron-up", ZMove::Forward, cx))
                    .child(reorder("dw-sel-backward", "chevron-down", ZMove::Backward, cx)),
            )
            .child(
                div()
                    .id("dw-selection-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .rounded(px(3.))
                    .bg(c(0xFFFFFF))
                    .p(px(2.))
                    .flex()
                    .flex_col()
                    .children(list),
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
    let view = cx.new(|cx| SelectionPane {
        app: entity.downgrade(),
        rename: None,
        _subscriptions: vec![cx.observe(&entity, |_, _, cx| cx.notify())],
    });
    app.open_pane(SELECTION, t("Selection", "Sélection"), view, cx);
}
