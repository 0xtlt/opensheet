//! The Format pane of charts: "Format Chart Area", "Format Data Series",
//! "Format Axis"... It always shows the selected element of the selected
//! chart, and every change is applied at once, as one undo step.

use super::fields::{
    self, Field, MARKER_SHAPES, fill_of, is_lined, labels_mut, line_of, line_of_mut, marker_mut, set_fill, text_of,
};
use crate::app::ExcelApp;
use crate::chart_view;
use crate::features::charts::elements::{axis_name, is_horizontal};
use crate::i18n::t;
use crate::text_input::{InputEvent, NavKey, TextInput};
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ColorButton, RadioGroup, Select, TabStrip, TextField, icon};
use gpui::{AnyElement, Context, Entity, Focusable, FontWeight, Render, SharedString, Subscription, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::chart::{
    AxisCrossing, Chart, ChartTitle, Dash, DataLabels, DataTable, DisplayUnit, ErrorAmount, ErrorDirection, Family,
    Fill, LabelPosition, LegendPos, Marker, MarkerShape, TickLabels, TickMark, TrendKind, View3D,
};
use opensheet_engine::chart_layout::{AxisId, ChartElement, Scale};
use opensheet_engine::format::{self, FormatKind};
use opensheet_engine::style::Color;
use opensheet_engine::{Locale, Value};
use std::collections::{HashMap, HashSet};

pub struct FormatPane {
    app: WeakEntity<ExcelApp>,
    tab: usize,
    /// Sections the user folded, by title.
    folded: HashSet<&'static str>,
    fields: HashMap<Field, Entity<TextInput>>,
    /// What every field shows for the chart as it is: a field left as it
    /// reads changes nothing (its value may be an automatic one).
    reads: HashMap<Field, String>,
    /// The chart and the element the fields were last filled for.
    shown_for: Option<(u32, ChartElement)>,
    /// The selection just changed: fields show the new element even if
    /// one of them still has the keyboard.
    refill: bool,
    _subscriptions: Vec<Subscription>,
}

/// What the pane shows: a copy of the selected chart, its selected
/// element, the scale of the selected axis.
struct Shown {
    chart: Chart,
    element: ChartElement,
    scale: Option<Scale>,
    locale: &'static Locale,
}

fn names(items: &[&'static str]) -> Vec<SharedString> {
    items.iter().map(|name| (*name).into()).collect()
}

const DASHES: [Dash; 8] = [
    Dash::Solid,
    Dash::RoundDot,
    Dash::SquareDot,
    Dash::Dash,
    Dash::DashDot,
    Dash::LongDash,
    Dash::LongDashDot,
    Dash::LongDashDotDot,
];

fn dash_names() -> Vec<SharedString> {
    names(&[
        t("Solid", "Uni"),
        t("Round Dot", "Point rond"),
        t("Square Dot", "Point carré"),
        t("Dash", "Tiret"),
        t("Dash Dot", "Tiret-point"),
        t("Long Dash", "Tiret cadratin"),
        t("Long Dash Dot", "Tiret cadratin-point"),
        t("Long Dash Dot Dot", "Tiret cadratin-point-point"),
    ])
}

fn marker_names() -> Vec<SharedString> {
    // In the order of `MARKER_SHAPES`.
    names(&[
        t("Square", "Carré"),
        t("Diamond", "Losange"),
        t("Triangle", "Triangle"),
        "X",
        t("Star", "Étoile"),
        t("Dot", "Point"),
        t("Dash", "Tiret"),
        t("Circle", "Cercle"),
        t("Plus", "Plus"),
    ])
}

const UNITS: [DisplayUnit; 10] = [
    DisplayUnit::None,
    DisplayUnit::Hundreds,
    DisplayUnit::Thousands,
    DisplayUnit::TenThousands,
    DisplayUnit::HundredThousands,
    DisplayUnit::Millions,
    DisplayUnit::TenMillions,
    DisplayUnit::HundredMillions,
    DisplayUnit::Billions,
    DisplayUnit::Trillions,
];

/// In the order of `UNITS`; Excel writes the units without a name as
/// numbers.
fn unit_names() -> Vec<SharedString> {
    names(&[
        t("None", "Aucun"),
        t("Hundreds", "Centaines"),
        t("Thousands", "Milliers"),
        "10000",
        "100000",
        t("Millions", "Millions"),
        "10000000",
        "100000000",
        t("Billions", "Milliards"),
        t("Trillions", "Billions"),
    ])
}

/// The number formats of the "Category" list, with the code each applies.
fn number_categories(locale: &Locale) -> Vec<(&'static str, FormatKind, String)> {
    vec![
        (t("General", "Standard"), FormatKind::General, "General".into()),
        (crate::i18n::t_ctx("format", "Number", "Nombre"), FormatKind::Number, "#,##0.00".into()),
        (t("Currency", "Monétaire"), FormatKind::Currency, locale.currency_format.into()),
        (t("Accounting", "Comptabilité"), FormatKind::Accounting, locale.accounting_format.into()),
        (t("Date", "Date"), FormatKind::Date, locale.short_date_format.into()),
        (t("Time", "Heure"), FormatKind::Time, locale.time_format.into()),
        (t("Percentage", "Pourcentage"), FormatKind::Percent, "0.00%".into()),
        (t("Fraction", "Fraction"), FormatKind::Fraction, "# ?/?".into()),
        (t("Scientific", "Scientifique"), FormatKind::Scientific, "0.00E+00".into()),
        (t("Text", "Texte"), FormatKind::Text, "@".into()),
        (t("Custom", "Personnalisé"), FormatKind::Custom, String::new()),
    ]
}

impl FormatPane {
    pub fn new(app: WeakEntity<ExcelApp>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        // The pane follows the selection and whatever changes the chart.
        if let Some(app) = app.upgrade() {
            subscriptions.push(cx.observe(&app, |_, _, cx| cx.notify()));
        }
        FormatPane {
            app,
            tab: 0,
            folded: HashSet::new(),
            fields: HashMap::new(),
            reads: HashMap::new(),
            shown_for: None,
            refill: false,
            _subscriptions: subscriptions,
        }
    }

    /// Change the selected chart; `f` gets its selected element.
    fn apply(&mut self, undo: &'static str, cx: &mut Context<Self>, f: impl FnOnce(&mut Chart, ChartElement)) {
        self.app
            .update(cx, |app, cx| {
                let element = app.chart_element();
                app.edit_chart(undo, cx, |chart| f(chart, element));
            })
            .ok();
    }

    // ---- fields ----------------------------------------------------------

    /// The input of a field, once the pane has shown it.
    #[cfg(test)]
    pub(super) fn field_input(&self, field: Field) -> Option<Entity<TextInput>> {
        self.fields.get(&field).cloned()
    }

    #[cfg(test)]
    pub(super) fn show_tab(&mut self, tab: usize, cx: &mut Context<Self>) {
        self.tab = tab;
        cx.notify();
    }

    /// Store what a field holds, when it says something new.
    pub(super) fn commit_field(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(text) = self.fields.get(&field).map(|input| input.read(cx).text().to_string()) else {
            return;
        };
        if self.reads.get(&field) == Some(&text) {
            return;
        }
        // What was typed belongs to the element the field was showing: a
        // click on another element of the chart takes the keyboard away
        // before the pane has shown that element.
        let Some((shown_chart, element)) = self.shown_for else {
            return;
        };
        self.app
            .update(cx, |app, cx| {
                let Some(chart) = app.selected_chart().filter(|chart| chart.id == shown_chart) else { return };
                let mut changed = chart.clone();
                if fields::set(&mut changed, element, field, &text, app.wb.locale) && changed != *chart {
                    app.edit_chart(t("Format Chart", "Mise en forme du graphique"), cx, |chart| *chart = changed);
                }
            })
            .ok();
        cx.notify();
    }

    fn on_field_event(&mut self, field: Field, ev: &InputEvent, cx: &mut Context<Self>) {
        match ev {
            InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => self.commit_field(field, cx),
            // Up and Down step a number like the arrows beside it.
            InputEvent::Navigate { key: NavKey::Up, .. } => self.step_field(field, 1.0, cx),
            InputEvent::Navigate { key: NavKey::Down, .. } => self.step_field(field, -1.0, cx),
            _ => {}
        }
    }

    /// Step a numeric field up or down and apply the result at once. The
    /// field shows its unit ("219 %"), so the number is read here.
    pub(super) fn step_field(&mut self, field: Field, steps: f64, cx: &mut Context<Self>) {
        let (Some((min, max, step)), Some(input)) = (field.spin(), self.fields.get(&field).cloned()) else {
            return;
        };
        let sep = crate::i18n::lang().locale().decimal_sep;
        let current = fields::parse_number(input.read(cx).text(), sep).unwrap_or(min.max(0.0).min(max));
        let next = (current + steps * step).clamp(min, max);
        input.update(cx, |input, cx| input.set_text(widgets::number_text(next, sep), cx));
        self.commit_field(field, cx);
    }

    /// The input of a field, showing what the chart says unless the user
    /// is typing in it.
    fn input(&mut self, field: Field, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextInput> {
        let input = match self.fields.get(&field) {
            Some(input) => input.clone(),
            None => {
                let input = widgets::input(cx, "");
                self._subscriptions
                    .push(cx.subscribe(&input, move |this, _, ev: &InputEvent, cx| this.on_field_event(field, ev, cx)));
                self.fields.insert(field, input.clone());
                input
            }
        };
        let text = fields::value(&shown.chart, shown.element, field, shown.scale.as_ref(), shown.locale);
        let stale = {
            let input = input.read(cx);
            (self.refill || !input.focus_handle(cx).is_focused(window)) && input.text() != text
        };
        if stale {
            input.update(cx, |input, cx| input.set_text(text.clone(), cx));
        }
        self.reads.insert(field, text);
        input
    }

    /// A label with a numeric field on its right.
    fn number(&mut self, label: &'static str, field: Field, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let input = self.input(field, shown, window, cx);
        let mut control = TextField::new(&input).width(88.0);
        if field.spin().is_some() {
            let arrow = |name: &'static str, icon_name: &'static str, steps: f64, cx: &mut Context<Self>| {
                div()
                    .id((name, field as usize))
                    .w(px(14.))
                    .h(px(10.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .active(|s| s.bg(c(theme::PRESSED)))
                    .child(icon(icon_name, 8.0, theme::TEXT_MUTED))
                    // Stepping does not move the caret into the padding.
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| this.step_field(field, steps, cx)))
            };
            control = control.trailing(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .border_l_1()
                    .border_color(c(theme::BORDER))
                    .child(arrow("field-up", "chevron-up", 1.0, cx))
                    .child(arrow("field-down", "chevron-down", -1.0, cx)),
            );
        }
        row(label, control)
    }

    // ---- controls -----------------------------------------------------------

    fn check(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        undo: &'static str,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Chart, ChartElement, bool) + 'static,
    ) -> AnyElement {
        Checkbox::new(id, label, on)
            .on_change(cx.listener(move |this, on: &bool, _, cx| {
                let on = *on;
                this.apply(undo, cx, |chart, element| set(chart, element, on));
            }))
            .into_any_element()
    }

    fn radio(
        &self,
        id: &'static str,
        options: &[&'static str],
        selected: Option<usize>,
        undo: &'static str,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Chart, ChartElement, usize) + 'static,
    ) -> AnyElement {
        RadioGroup::new(id, names(options), selected)
            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                let ix = *ix;
                this.apply(undo, cx, |chart, element| set(chart, element, ix));
            }))
            .into_any_element()
    }

    fn select(
        &self,
        id: &'static str,
        label: &'static str,
        options: Vec<SharedString>,
        selected: Option<usize>,
        undo: &'static str,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Chart, ChartElement, usize) + 'static,
    ) -> AnyElement {
        row(
            label,
            Select::new(id, options, selected).width(150.0).on_change(cx.listener(move |this, ix: &usize, _, cx| {
                let ix = *ix;
                this.apply(undo, cx, |chart, element| set(chart, element, ix));
            })),
        )
    }

    fn color(
        &self,
        id: &'static str,
        current: Option<Color>,
        undo: &'static str,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Chart, ChartElement, Option<Color>) + 'static,
    ) -> AnyElement {
        row(
            t("Color", "Couleur"),
            ColorButton::new(id, Some(current))
                .none_label(t("Automatic", "Automatique"))
                .more(&self.app)
                .on_pick(cx.listener(move |this, color: &Option<Color>, _, cx| {
                    let color = *color;
                    this.apply(undo, cx, |chart, element| set(chart, element, color));
                })),
        )
    }

    /// A titled group of controls that folds.
    fn section(&mut self, title: &'static str, cx: &mut Context<Self>, children: Vec<AnyElement>) -> AnyElement {
        let open = !self.folded.contains(title);
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .id(title)
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(icon(if open { "chevron-down" } else { "chevron-right" }, 10.0, theme::TEXT_MUTED))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.folded.remove(title) {
                            this.folded.insert(title);
                        }
                        cx.notify();
                    })),
            )
            .when(open, |d| d.child(div().pl(px(26.)).pr(px(10.)).pb(px(10.)).flex().flex_col().gap(px(7.)).children(children)))
            .into_any_element()
    }

    // ---- sections -----------------------------------------------------------

    /// "Fill": none, solid, automatic, and the colour.
    fn fill_section(&mut self, shown: &Shown, with_none: bool, cx: &mut Context<Self>) -> AnyElement {
        let fill = fill_of(&shown.chart, shown.element).unwrap_or_default();
        let undo = t("Fill", "Remplissage");
        let mut body = Vec::new();
        if with_none {
            let selected = match fill {
                Fill::None => 0,
                Fill::Solid(_) => 1,
                Fill::Auto => 2,
            };
            body.push(self.radio(
                "fill-kind",
                &[t("No fill", "Aucun remplissage"), t("Solid fill", "Remplissage uni"), t("Automatic", "Automatique")],
                Some(selected),
                undo,
                cx,
                move |chart, element, ix| {
                    let fill = match (ix, fill) {
                        (0, _) => Fill::None,
                        (1, Fill::Solid(color)) => Fill::Solid(color),
                        (1, _) => Fill::Solid(Color::WHITE),
                        _ => Fill::Auto,
                    };
                    set_fill(chart, element, fill);
                },
            ));
        } else {
            let selected = if matches!(fill, Fill::Solid(_)) { 0 } else { 1 };
            let chart = &shown.chart;
            // A solid fill starts from the colour the series has now.
            let current = match shown.element {
                ChartElement::Series(s) | ChartElement::Point(s, _) => opensheet_engine::chart::palette_color(s as usize),
                _ => Color::WHITE,
            };
            let current = shown.element.series().and_then(|s| chart.series.get(s)).and_then(|s| s.color).unwrap_or(current);
            body.push(self.radio(
                "fill-kind",
                &[t("Solid fill", "Remplissage uni"), t("Automatic", "Automatique")],
                Some(selected),
                undo,
                cx,
                move |chart, element, ix| {
                    let fill = match (ix, fill) {
                        (0, Fill::Solid(color)) => Fill::Solid(color),
                        (0, _) => Fill::Solid(current),
                        _ => Fill::Auto,
                    };
                    set_fill(chart, element, fill);
                },
            ));
        }
        let color = match fill {
            Fill::Solid(color) => Some(color),
            _ => None,
        };
        body.push(self.color("fill-color", color, undo, cx, |chart, element, color| {
            set_fill(chart, element, color.map_or(Fill::Auto, Fill::Solid))
        }));
        if let ChartElement::Series(_) = shown.element {
            let chart = &shown.chart;
            if chart.kind.is_round() || chart.series.len() == 1 {
                let label = if chart.kind.is_round() {
                    t("Vary colors by slice", "Variation des couleurs par secteur")
                } else {
                    t("Vary colors by point", "Variation des couleurs par point")
                };
                body.push(self.check("vary", label, chart.varies_colors(), undo, cx, |chart, _, on| chart.vary_colors = Some(on)));
            }
        }
        self.section(t("Fill", "Remplissage"), cx, body)
    }

    /// "Border" / "Line": none, solid, automatic, colour, width, dash.
    fn line_section(&mut self, title: &'static str, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(line) = line_of(&shown.chart, shown.element).cloned() else {
            return div().into_any_element();
        };
        let undo = title;
        let automatic = line.shown.is_none() && line.color.is_none() && line.width.is_none() && line.dash == Dash::Solid;
        let selected = match line.shown {
            Some(false) => 0,
            _ if automatic => 2,
            _ => 1,
        };
        let mut body = vec![
            self.radio(
                "line-kind",
                &[t("No line", "Aucun trait"), t("Solid line", "Trait plein"), t("Automatic", "Automatique")],
                Some(selected),
                undo,
                cx,
                |chart, element, ix| {
                    if let Some(line) = line_of_mut(chart, element) {
                        match ix {
                            0 => line.shown = Some(false),
                            1 => line.shown = Some(true),
                            _ => *line = Default::default(),
                        }
                    }
                },
            ),
            self.color("line-color", line.color, undo, cx, |chart, element, color| {
                if let Some(line) = line_of_mut(chart, element) {
                    line.color = color;
                    if color.is_some() && line.shown == Some(false) {
                        line.shown = Some(true);
                    }
                }
            }),
            self.number(t("Width", "Largeur"), Field::LineWidth, shown, window, cx),
            self.select(
                "line-dash",
                t("Dash type", "Type de tiret"),
                dash_names(),
                DASHES.iter().position(|d| *d == line.dash),
                undo,
                cx,
                |chart, element, ix| {
                    if let Some(line) = line_of_mut(chart, element) {
                        line.dash = DASHES[ix];
                    }
                },
            ),
        ];
        if let ChartElement::Series(s) = shown.element {
            let chart = &shown.chart;
            if is_lined(chart, s as usize) && chart.kind.family() != Family::Radar {
                let series = &chart.series[s as usize];
                let smooth = series.smooth_for(chart.series_kind(series));
                body.push(self.check("smooth", t("Smoothed line", "Lissage"), smooth, undo, cx, |chart, element, on| {
                    if let Some(series) = element.series().and_then(|s| chart.series.get_mut(s)) {
                        series.smooth = Some(on);
                    }
                }));
            }
        }
        self.section(title, cx, body)
    }

    /// "Text Options": the colour of the text of the selected element and
    /// its font (of every text of the chart, for the chart area).
    fn text_tab(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> (&'static str, Vec<AnyElement>) {
        let text = text_of(&shown.chart, shown.element).unwrap_or_default();
        let fill = t("Text Fill", "Remplissage du texte");
        let color = text.color;
        let fill_body = vec![
            self.radio(
                "text-fill-kind",
                &[t("Solid fill", "Remplissage uni"), t("Automatic", "Automatique")],
                Some(color.is_none() as usize),
                fill,
                cx,
                move |chart, element, ix| {
                    // A solid fill starts from the grey the text has by itself.
                    let color = (ix == 0).then(|| color.unwrap_or(Color(0x595959)));
                    element.format_text(chart, |text| text.color = color);
                },
            ),
            self.color("text-color", color, fill, cx, |chart, element, color| {
                element.format_text(chart, |text| text.color = color);
            }),
        ];
        let font = t("Font", "Police");
        let name = self.input(Field::FontName, shown, window, cx);
        let style = |this: &Self, id: &'static str, label: &'static str, on: Option<bool>, cx: &mut Context<Self>, set: fn(&mut opensheet_engine::chart::TextFormat, Option<bool>)| {
            this.check(id, label, on == Some(true), label, cx, move |chart, element, on| {
                element.format_text(chart, |text| set(text, on.then_some(true)));
            })
        };
        let font_body = vec![
            row(font, TextField::new(&name).width(150.0)),
            self.number(t("Size", "Taille"), Field::FontSize, shown, window, cx),
            style(self, "text-bold", t("Bold", "Gras"), text.bold, cx, |text, on| text.bold = on),
            style(self, "text-italic", t("Italic", "Italique"), text.italic, cx, |text, on| text.italic = on),
            style(self, "text-underline", t("Underline", "Souligné"), text.underline, cx, |text, on| text.underline = on),
        ];
        (t("Text Options", "Options de texte"), vec![self.section(fill, cx, fill_body), self.section(font, cx, font_body)])
    }

    /// "Marker": options, fill and border of the markers of a series.
    fn marker_sections(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ChartElement::Series(s) = shown.element else {
            return Vec::new();
        };
        let chart = &shown.chart;
        let Some(series) = chart.series.get(s as usize) else {
            return Vec::new();
        };
        let kind = chart.series_kind(series);
        let marker = series.marker_for(kind);
        let undo = t("Marker", "Marqueur");
        let selected = match &series.marker {
            None => 0,
            Some(m) if m.shape == MarkerShape::None => 1,
            Some(_) => 2,
        };
        let edit = |chart: &mut Chart, element: ChartElement, f: &dyn Fn(&mut Marker)| {
            if let Some(marker) = element.series().and_then(|s| marker_mut(chart, s)) {
                f(marker);
            }
        };
        let options = vec![
            self.radio(
                "marker-kind",
                &[t("Automatic", "Automatique"), t("None", "Aucun"), t("Built-in", "Prédéfini")],
                Some(selected),
                undo,
                cx,
                move |chart, element, ix| {
                    let Some(series) = element.series().and_then(|s| chart.series.get_mut(s)) else { return };
                    series.marker = match ix {
                        0 => None,
                        1 => Some(Marker::new(MarkerShape::None)),
                        _ => Some(Marker {
                            shape: if marker.shape == MarkerShape::None { MarkerShape::Circle } else { marker.shape },
                            ..marker.clone()
                        }),
                    };
                },
            ),
            self.select(
                "marker-shape",
                t("Type", "Type"),
                marker_names(),
                MARKER_SHAPES.iter().position(|m| *m == series.marker_for(kind).shape),
                undo,
                cx,
                move |chart, element, ix| edit(chart, element, &|m| m.shape = MARKER_SHAPES[ix]),
            ),
            self.number(t("Size", "Taille"), Field::MarkerSize, shown, window, cx),
        ];
        let marker = series.marker_for(kind);
        let fill = vec![self.color("marker-fill", marker.fill, undo, cx, move |chart, element, color| {
            edit(chart, element, &|m| m.fill = color)
        })];
        let border = vec![self.color("marker-line", marker.line, undo, cx, move |chart, element, color| {
            edit(chart, element, &|m| m.line = color)
        })];
        vec![
            self.section(t("Marker Options", "Options de marqueur"), cx, options),
            self.section(t("Marker Fill", "Remplissage des marqueurs"), cx, fill),
            self.section(t("Marker Border", "Bordure des marqueurs"), cx, border),
        ]
    }

    fn series_options(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let chart = &shown.chart;
        let undo = t("Series Options", "Options des séries");
        let mut body = Vec::new();
        let series = shown.element.series().and_then(|s| chart.series.get(s));
        let kind = series.map_or(chart.kind, |s| chart.series_kind(s));
        let two_axes = !chart.kind.is_3d() && !chart.kind.is_round() && chart.kind.family() != Family::Radar;
        if let (Some(series), true, ChartElement::Series(_)) = (series, two_axes, shown.element) {
            body.push(widgets::label(t("Plot Series On", "Tracer la série avec")).into_any_element());
            body.push(self.radio(
                "series-axis",
                &[t("Primary Axis", "Axe principal"), t("Secondary Axis", "Axe secondaire")],
                Some(series.secondary as usize),
                undo,
                cx,
                |chart, element, ix| {
                    if let Some(series) = element.series().and_then(|s| chart.series.get_mut(s)) {
                        series.secondary = ix == 1;
                    }
                },
            ));
        }
        match kind.family() {
            Family::Column | Family::Bar => {
                if !kind.is_3d() {
                    body.push(self.number(t("Series Overlap", "Superposition de séries"), Field::Overlap, shown, window, cx));
                }
                body.push(self.number(t("Gap Width", "Largeur de l'intervalle"), Field::GapWidth, shown, window, cx));
            }
            Family::Pie | Family::Doughnut => {
                body.push(self.number(t("Angle of first slice", "Angle du premier secteur"), Field::FirstSlice, shown, window, cx));
                if kind.family() == Family::Pie {
                    let label = match shown.element {
                        ChartElement::Point(..) => t("Point Explosion", "Éclatement des points"),
                        _ => t("Pie Explosion", "Éclatement des secteurs"),
                    };
                    body.push(self.number(label, Field::Explosion, shown, window, cx));
                } else {
                    body.push(self.number(t("Doughnut Hole Size", "Taille du centre"), Field::HoleSize, shown, window, cx));
                }
            }
            Family::Bubble => {
                body.push(self.number(t("Scale bubble size to", "Échelle de la bulle"), Field::BubbleScale, shown, window, cx));
            }
            _ => {}
        }
        if body.is_empty() {
            body.push(widgets::label(t("No options for this series.", "Aucune option pour cette série.")).text_color(c(theme::TEXT_MUTED)).into_any_element());
        }
        self.section(undo, cx, body)
    }

    fn axis_sections(&mut self, id: AxisId, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let chart = &shown.chart;
        let axis = id.of(&chart.axes).clone();
        let undo = t("Format Axis", "Format de l'axe");
        let numeric = !id.is_category() || chart.kind.is_xy();
        let edit = move |chart: &mut Chart, f: &dyn Fn(&mut opensheet_engine::chart::Axis)| f(id.of_mut(&mut chart.axes));
        let mut options = Vec::new();
        // The other axis of the pair: the one that crosses this one.
        let other = match id {
            AxisId::Category => AxisId::Value,
            AxisId::Value => AxisId::Category,
            AxisId::SecondaryCategory => AxisId::SecondaryValue,
            AxisId::SecondaryValue => AxisId::SecondaryCategory,
        };
        let crosses = if is_horizontal(chart, other) {
            t("Horizontal axis crosses", "L'axe horizontal coupe")
        } else {
            t("Vertical axis crosses", "L'axe vertical coupe")
        };
        let crossing = match axis.crossing {
            AxisCrossing::Auto | AxisCrossing::Min => 0,
            AxisCrossing::At(_) => 1,
            AxisCrossing::Max => 2,
        };
        if numeric {
            // A bound set by hand can be handed back to Excel.
            let bound = |this: &mut Self, label: &'static str, field: Field, set: bool, reset: fn(&mut opensheet_engine::chart::Axis), cx: &mut Context<Self>, window: &mut Window| {
                let input = this.input(field, shown, window, cx);
                let state: AnyElement = if set {
                    Button::new(("axis-reset", field as usize), t("Reset", "Rétablir"))
                        .width(84.0)
                        .on_click(cx.listener(move |this, _, _, cx| this.apply(undo, cx, |chart, _| edit(chart, &reset))))
                        .into_any_element()
                } else {
                    // Wide enough for the French word on one line.
                    div()
                        .w(px(84.))
                        .flex_none()
                        .whitespace_nowrap()
                        .text_color(c(theme::TEXT_MUTED))
                        .child(t("Auto", "Automatique"))
                        .into_any_element()
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_1().child(label))
                    .child(TextField::new(&input).width(68.0))
                    .child(state)
                    .into_any_element()
            };
            options.push(widgets::label(t("Bounds", "Limites")).font_weight(FontWeight::SEMIBOLD).into_any_element());
            options.push(bound(self, t("Minimum", "Minimum"), Field::AxisMin, axis.min.is_some(), |a| a.min = None, cx, window));
            options.push(bound(self, t("Maximum", "Maximum"), Field::AxisMax, axis.max.is_some(), |a| a.max = None, cx, window));
            options.push(widgets::label(t("Units", "Unités")).font_weight(FontWeight::SEMIBOLD).into_any_element());
            options.push(bound(self, t("Major", "Principale"), Field::AxisMajor, axis.major_unit.is_some(), |a| a.major_unit = None, cx, window));
            options.push(bound(self, t("Minor", "Secondaire"), Field::AxisMinor, axis.minor_unit.is_some(), |a| a.minor_unit = None, cx, window));
            options.push(widgets::label(crosses).font_weight(FontWeight::SEMIBOLD).into_any_element());
            options.push(self.radio(
                "axis-cross",
                &[t("Automatic", "Automatique"), t("Axis value", "Valeur de l'axe"), t("Maximum axis value", "Valeur maximale de l'axe")],
                Some(crossing),
                undo,
                cx,
                move |chart, _, ix| {
                    edit(chart, &|a| {
                        a.crossing = match ix {
                            0 => AxisCrossing::Auto,
                            1 => AxisCrossing::At(0.0),
                            _ => AxisCrossing::Max,
                        }
                    })
                },
            ));
            if crossing == 1 {
                options.push(self.number(t("Axis value", "Valeur de l'axe"), Field::AxisCross, shown, window, cx));
            }
            options.push(self.select(
                "axis-units",
                t("Display units", "Unités d'affichage"),
                unit_names(),
                UNITS.iter().position(|u| *u == axis.display_unit),
                undo,
                cx,
                move |chart, _, ix| edit(chart, &|a| a.display_unit = UNITS[ix]),
            ));
            if axis.display_unit != DisplayUnit::None {
                options.push(self.check(
                    "axis-unit-label",
                    t("Show display units label on chart", "Afficher l'étiquette des unités d'affichage sur le graphique"),
                    axis.display_unit_label,
                    undo,
                    cx,
                    move |chart, _, on| edit(chart, &|a| a.display_unit_label = on),
                ));
            }
            options.push(self.check(
                "axis-log",
                t("Logarithmic scale", "Échelle logarithmique"),
                axis.log_base.is_some(),
                undo,
                cx,
                move |chart, _, on| edit(chart, &|a| a.log_base = on.then_some(10.0)),
            ));
            if axis.log_base.is_some() {
                options.push(self.number(t("Base", "Base"), Field::LogBase, shown, window, cx));
            }
            options.push(self.check(
                "axis-reverse",
                t("Values in reverse order", "Valeurs en ordre inverse"),
                axis.reversed,
                undo,
                cx,
                move |chart, _, on| edit(chart, &|a| a.reversed = on),
            ));
        } else {
            options.push(widgets::label(crosses).font_weight(FontWeight::SEMIBOLD).into_any_element());
            options.push(self.radio(
                "axis-cross",
                &[
                    t("Automatic", "Automatique"),
                    t("At category number", "À la catégorie numéro"),
                    t("At maximum category", "À la catégorie maximale"),
                ],
                Some(crossing),
                undo,
                cx,
                move |chart, _, ix| {
                    edit(chart, &|a| {
                        a.crossing = match ix {
                            0 => AxisCrossing::Auto,
                            1 => AxisCrossing::At(1.0),
                            _ => AxisCrossing::Max,
                        }
                    })
                },
            ));
            if crossing == 1 {
                options.push(self.number(t("At category number", "À la catégorie numéro"), Field::AxisCross, shown, window, cx));
            }
            options.push(self.check(
                "axis-reverse",
                t("Categories in reverse order", "Abscisses en ordre inverse"),
                axis.reversed,
                undo,
                cx,
                move |chart, _, on| edit(chart, &|a| a.reversed = on),
            ));
        }

        const TICKS: [TickMark; 4] = [TickMark::None, TickMark::Inside, TickMark::Outside, TickMark::Cross];
        let tick_names = || names(&[t("None", "Aucun"), t("Inside", "À l'intérieur"), t("Outside", "À l'extérieur"), t("Cross", "Sur l'axe")]);
        let ticks = vec![
            self.select(
                "axis-major-ticks",
                t("Major type", "Type principal"),
                tick_names(),
                TICKS.iter().position(|m| *m == axis.major_ticks),
                undo,
                cx,
                move |chart, _, ix| edit(chart, &|a| a.major_ticks = TICKS[ix]),
            ),
            self.select(
                "axis-minor-ticks",
                t("Minor type", "Type secondaire"),
                tick_names(),
                TICKS.iter().position(|m| *m == axis.minor_ticks),
                undo,
                cx,
                move |chart, _, ix| edit(chart, &|a| a.minor_ticks = TICKS[ix]),
            ),
        ];
        const LABELS: [TickLabels; 4] = [TickLabels::NextToAxis, TickLabels::High, TickLabels::Low, TickLabels::None];
        let labels = vec![self.select(
            "axis-labels",
            t("Label Position", "Position de l'étiquette"),
            names(&[t("Next to Axis", "À côté de l'axe"), t("High", "Haut"), t("Low", "Bas"), t("None", "Aucun")]),
            LABELS.iter().position(|l| *l == axis.labels),
            undo,
            cx,
            move |chart, _, ix| edit(chart, &|a| a.labels = LABELS[ix]),
        )];
        let mut out = vec![
            self.section(t("Axis Options", "Options d'axe"), cx, options),
            self.section(t("Tick Marks", "Graduations"), cx, ticks),
            self.section(t("Labels", "Étiquettes"), cx, labels),
        ];
        if numeric {
            out.push(self.number_section(axis.number_format.as_deref(), shown, window, cx));
        }
        out
    }

    /// "Number": the category, the format code, linked to source or not.
    fn number_section(&mut self, code: Option<&str>, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let undo = t("Number Format", "Format de nombre");
        let categories = number_categories(shown.locale);
        let kind = match format::format_kind(code.unwrap_or("General")) {
            FormatKind::DateTime => FormatKind::Date,
            kind => kind,
        };
        let store = store_number_format;
        let codes: Vec<String> = categories.iter().map(|(_, _, code)| code.clone()).collect();
        let input = self.input(Field::NumberFormat, shown, window, cx);
        let body = vec![
            self.select(
                "number-category",
                t("Category", "Catégorie"),
                categories.iter().map(|(name, ..)| (*name).into()).collect(),
                categories.iter().position(|(_, k, _)| *k == kind),
                undo,
                cx,
                move |chart, element, ix| {
                    // "Custom" keeps the code as it is, for the user to change.
                    if !codes[ix].is_empty() {
                        store(chart, element, (ix > 0).then(|| codes[ix].clone()));
                    }
                },
            ),
            widgets::label(t("Format Code", "Code de format")).into_any_element(),
            div()
                .flex()
                .gap(px(6.))
                .child(TextField::new(&input))
                .child(Button::new("number-add", t("Add", "Ajouter")).width(64.0).on_click(cx.listener(|this, _, _, cx| {
                    this.commit_field(Field::NumberFormat, cx)
                })))
                .into_any_element(),
            self.check("number-linked", t("Linked to source", "Lier à la source"), code.is_none(), undo, cx, move |chart, element, on| {
                if on {
                    store(chart, element, None);
                }
            }),
            Button::new("number-more", t("More Number Formats…", "Autres formats numériques…"))
                .on_click(cx.listener(|this, _, window, cx| this.open_number_dialog(window, cx)))
                .into_any_element(),
        ];
        self.section(t("Number", "Nombre"), cx, body)
    }

    /// The Number tab of Format Cells, for the format of the labels of the
    /// selected axis or of the selected data labels.
    fn open_number_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = self.app.clone();
        self.app
            .update(cx, |app, cx| {
                let Some(chart) = app.selected_chart() else { return };
                let element = app.chart_element();
                let code = match element {
                    ChartElement::Axis(id) => id.of(&chart.axes).number_format.clone(),
                    _ => element.series().and_then(|s| chart.series.get(s)).and_then(|s| chart.labels_for(s)).and_then(|l| l.number_format.clone()),
                };
                let code = code.unwrap_or_else(|| "General".to_string());
                crate::features::format_cells::open_number_format(app, &code, Value::Number(1234.5), window, cx, move |code, _, cx| {
                    let code = (!code.eq_ignore_ascii_case("General")).then(|| code.to_string());
                    weak.update(cx, |app, cx| {
                        app.edit_chart(t("Number Format", "Format de nombre"), cx, |chart| store_number_format(chart, element, code));
                    })
                    .ok();
                });
            })
            .ok();
    }

    fn label_sections(&mut self, series: usize, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let chart = &shown.chart;
        let Some(labels) = chart.series.get(series).and_then(|s| chart.labels_for(s)).cloned() else {
            return Vec::new();
        };
        let undo = t("Format Data Labels", "Format des étiquettes de données");
        let (xy, round) = (chart.kind.is_xy(), chart.kind.is_round());
        let part = |this: &Self, id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>, set: fn(&mut DataLabels, bool)| {
            this.check(id, label, on, undo, cx, move |chart, element, on| {
                if let Some(labels) = labels_mut(chart, element) {
                    set(labels, on);
                }
            })
        };
        let mut options = vec![
            widgets::label(t("Label Contains", "Texte de l'étiquette")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
            part(self, "label-series", t("Series Name", "Nom de série"), labels.series_name, cx, |l, on| l.series_name = on),
            part(
                self,
                "label-category",
                if xy { t("X Value", "Valeur X") } else { t("Category Name", "Nom de catégorie") },
                labels.category,
                cx,
                |l, on| l.category = on,
            ),
            part(self, "label-value", if xy { t("Y Value", "Valeur Y") } else { t("Value", "Valeur") }, labels.value, cx, |l, on| l.value = on),
        ];
        if round {
            options.push(part(self, "label-percent", t("Percentage", "Pourcentage"), labels.percent, cx, |l, on| l.percent = on));
            options.push(part(
                self,
                "label-leader",
                t("Show Leader Lines", "Afficher les lignes d'étiquettes"),
                labels.leader_lines,
                cx,
                |l, on| l.leader_lines = on,
            ));
        }
        if chart.kind.family() == Family::Bubble {
            options.push(part(self, "label-size", t("Bubble Size", "Taille de la bulle"), labels.bubble_size, cx, |l, on| l.bubble_size = on));
        }
        options.push(part(self, "label-key", t("Legend key", "Symbole de légende"), labels.legend_key, cx, |l, on| l.legend_key = on));
        // The separators Excel lists; the first is the automatic one.
        let auto = format!("{} ", shown.locale.arg_sep);
        let separators: [Option<&'static str>; 5] = [None, Some("; "), Some(". "), Some("\n"), Some(" ")];
        let separator_names = vec![
            SharedString::from(match shown.locale.arg_sep {
                ',' => t(", (comma)", ", (virgule)"),
                _ => t("; (semicolon)", "; (point-virgule)"),
            }),
            t("; (semicolon)", "; (point-virgule)").into(),
            t(". (period)", ". (point)").into(),
            t("(New Line)", "(Nouvelle ligne)").into(),
            t("  (space)", "  (espace)").into(),
        ];
        let current = match labels.separator.as_deref() {
            None => Some(0),
            Some(text) if text == auto => Some(0),
            Some(text) => separators.iter().position(|s| *s == Some(text)),
        };
        options.push(self.select("label-separator", t("Separator", "Séparateur"), separator_names, current, undo, cx, move |chart, element, ix| {
            if let Some(labels) = labels_mut(chart, element) {
                labels.separator = separators[ix].map(str::to_string);
            }
        }));
        let positions = crate::features::charts::label_positions(chart);
        if !positions.is_empty() {
            options.push(widgets::label(t("Label Position", "Position de l'étiquette")).font_weight(FontWeight::SEMIBOLD).into_any_element());
            let places: Vec<LabelPosition> = positions.iter().map(|(p, _)| *p).collect();
            let titles: Vec<&'static str> = positions.iter().map(|(_, name)| *name).collect();
            // Automatic is one of them: Excel shows it ticked.
            let automatic = match chart.kind.family() {
                Family::Column | Family::Bar if chart.kind.is_stacked() => LabelPosition::Center,
                Family::Column | Family::Bar => LabelPosition::OutsideEnd,
                Family::Pie => LabelPosition::BestFit,
                _ => LabelPosition::Right,
            };
            let at = if labels.position == LabelPosition::Auto { automatic } else { labels.position };
            options.push(self.radio("label-position", &titles, places.iter().position(|p| *p == at), undo, cx, move |chart, element, ix| {
                if let Some(labels) = labels_mut(chart, element) {
                    labels.position = places[ix];
                }
            }));
        }
        vec![
            self.section(t("Label Options", "Options d'étiquette"), cx, options),
            self.number_section(labels.number_format.as_deref(), shown, window, cx),
        ]
    }

    fn trendline_options(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ChartElement::Trendline(s, i) = shown.element else {
            return div().into_any_element();
        };
        let Some(trend) = shown.chart.series.get(s as usize).and_then(|x| x.trendlines.get(i as usize)).cloned() else {
            return div().into_any_element();
        };
        let undo = t("Format Trendline", "Format de courbe de tendance");
        let edit = |chart: &mut Chart, element: ChartElement, f: &dyn Fn(&mut opensheet_engine::chart::Trendline)| {
            if let ChartElement::Trendline(s, i) = element {
                if let Some(trend) = chart.series.get_mut(s as usize).and_then(|x| x.trendlines.get_mut(i as usize)) {
                    f(trend);
                }
            }
        };
        let kind = match trend.kind {
            TrendKind::Exponential => 0,
            TrendKind::Linear => 1,
            TrendKind::Logarithmic => 2,
            TrendKind::Polynomial(_) => 3,
            TrendKind::Power => 4,
            TrendKind::MovingAverage(_) => 5,
        };
        let mut body = vec![self.radio(
            "trend-kind",
            &[
                t("Exponential", "Exponentielle"),
                t("Linear", "Linéaire"),
                t("Logarithmic", "Logarithmique"),
                t("Polynomial", "Polynomiale"),
                t("Power", "Puissance"),
                t("Moving Average", "Moyenne mobile"),
            ],
            Some(kind),
            undo,
            cx,
            move |chart, element, ix| {
                edit(chart, element, &|t| {
                    t.kind = match ix {
                        0 => TrendKind::Exponential,
                        1 => TrendKind::Linear,
                        2 => TrendKind::Logarithmic,
                        3 => TrendKind::Polynomial(2),
                        4 => TrendKind::Power,
                        _ => TrendKind::MovingAverage(2),
                    }
                })
            },
        )];
        match trend.kind {
            TrendKind::Polynomial(_) => body.push(self.number(crate::i18n::t_ctx("trendline", "Order", "Degré"), Field::TrendOrder, shown, window, cx)),
            TrendKind::MovingAverage(_) => body.push(self.number(t("Period", "Période"), Field::TrendPeriod, shown, window, cx)),
            _ => {}
        }
        body.push(widgets::label(t("Trendline Name", "Nom de la courbe de tendance")).font_weight(FontWeight::SEMIBOLD).into_any_element());
        body.push(self.radio(
            "trend-name",
            &[t("Automatic", "Automatique"), t("Custom", "Personnalisé")],
            Some(trend.name.is_some() as usize),
            undo,
            cx,
            move |chart, element, ix| edit(chart, element, &|t| t.name = (ix == 1).then(|| t.name.clone().unwrap_or_default())),
        ));
        if trend.name.is_some() {
            let input = self.input(Field::TrendName, shown, window, cx);
            body.push(div().flex().child(TextField::new(&input)).into_any_element());
        }
        let moving = matches!(trend.kind, TrendKind::MovingAverage(_));
        if !moving {
            body.push(widgets::label(t("Forecast", "Prévision")).font_weight(FontWeight::SEMIBOLD).into_any_element());
            body.push(self.number(t("Forward (periods)", "Suivant (périodes)"), Field::Forward, shown, window, cx));
            body.push(self.number(t("Backward (periods)", "En arrière (périodes)"), Field::Backward, shown, window, cx));
            if matches!(trend.kind, TrendKind::Linear | TrendKind::Polynomial(_) | TrendKind::Exponential) {
                body.push(self.check(
                    "trend-intercept",
                    t("Set Intercept", "Définir l'interception"),
                    trend.intercept.is_some(),
                    undo,
                    cx,
                    move |chart, element, on| edit(chart, element, &|t| t.intercept = on.then_some(0.0)),
                ));
                if trend.intercept.is_some() {
                    body.push(self.number(t("Intercept", "Interception"), Field::Intercept, shown, window, cx));
                }
            }
            body.push(self.check(
                "trend-equation",
                t("Display Equation on chart", "Afficher l'équation sur le graphique"),
                trend.show_equation,
                undo,
                cx,
                move |chart, element, on| edit(chart, element, &|t| t.show_equation = on),
            ));
            body.push(self.check(
                "trend-r2",
                t(
                    "Display R-squared value on chart",
                    "Afficher le coefficient de détermination (R²) sur le graphique",
                ),
                trend.show_r2,
                undo,
                cx,
                move |chart, element, on| edit(chart, element, &|t| t.show_r2 = on),
            ));
        }
        self.section(t("Trendline Options", "Options de courbe de tendance"), cx, body)
    }

    fn error_bar_options(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let ChartElement::ErrorBars(s, i) = shown.element else {
            return div().into_any_element();
        };
        let Some(bars) = shown.chart.series.get(s as usize).and_then(|x| x.error_bars.get(i as usize)).cloned() else {
            return div().into_any_element();
        };
        let undo = t("Format Error Bars", "Format des barres d'erreur");
        let edit = |chart: &mut Chart, element: ChartElement, f: &dyn Fn(&mut opensheet_engine::chart::ErrorBars)| {
            if let ChartElement::ErrorBars(s, i) = element {
                if let Some(bars) = chart.series.get_mut(s as usize).and_then(|x| x.error_bars.get_mut(i as usize)) {
                    f(bars);
                }
            }
        };
        let amount = match bars.amount {
            ErrorAmount::Fixed(_) => 0,
            ErrorAmount::Percent(_) => 1,
            ErrorAmount::StdDev(_) => 2,
            ErrorAmount::StdErr => 3,
        };
        const DIRECTIONS: [ErrorDirection; 3] = [ErrorDirection::Both, ErrorDirection::Minus, ErrorDirection::Plus];
        let title = if bars.horizontal {
            t("Horizontal Error Bar", "Barre d'erreur horizontale")
        } else {
            t("Vertical Error Bar", "Barre d'erreur verticale")
        };
        let mut body = vec![
            widgets::label(t("Direction", "Direction")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
            self.radio(
                "error-direction",
                &[t("Both", "Les deux"), t("Minus", "Moins"), t("Plus", "Plus")],
                DIRECTIONS.iter().position(|d| *d == bars.direction),
                undo,
                cx,
                move |chart, element, ix| edit(chart, element, &|b| b.direction = DIRECTIONS[ix]),
            ),
            widgets::label(t("End Style", "Style d'extrémité")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
            self.radio(
                "error-cap",
                &[t("No Cap", "Sans empattement"), t("Cap", "Avec empattement")],
                Some(bars.end_cap as usize),
                undo,
                cx,
                move |chart, element, ix| edit(chart, element, &|b| b.end_cap = ix == 1),
            ),
            widgets::label(t("Error Amount", "Marge d'erreur")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
            self.radio(
                "error-amount",
                &[
                    t("Fixed value", "Valeur fixe"),
                    t("Percentage", "Pourcentage"),
                    t("Standard deviation(s)", "Écarts types"),
                    t("Standard error", "Erreur standard"),
                ],
                Some(amount),
                undo,
                cx,
                move |chart, element, ix| {
                    edit(chart, element, &|b| {
                        b.amount = match ix {
                            0 => ErrorAmount::Fixed(1.0),
                            1 => ErrorAmount::Percent(5.0),
                            2 => ErrorAmount::StdDev(1.0),
                            _ => ErrorAmount::StdErr,
                        }
                    })
                },
            ),
        ];
        match bars.amount {
            ErrorAmount::Fixed(_) => body.push(self.number(t("Fixed value", "Valeur fixe"), Field::ErrorFixed, shown, window, cx)),
            ErrorAmount::Percent(_) => body.push(self.number(t("Percentage", "Pourcentage"), Field::ErrorPercent, shown, window, cx)),
            ErrorAmount::StdDev(_) => body.push(self.number(t("Standard deviation(s)", "Écarts types"), Field::ErrorStdDev, shown, window, cx)),
            ErrorAmount::StdErr => {}
        }
        self.section(title, cx, body)
    }

    fn rotation_section(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let chart = &shown.chart;
        let view = chart.view3d_or_default();
        let undo = t("3-D Rotation", "Rotation 3D");
        let pie = chart.kind.family() == Family::Pie;
        let mut body = vec![
            self.number(t("X Rotation", "Rotation X"), Field::RotationX, shown, window, cx),
            self.number(t("Y Rotation", "Rotation Y"), Field::RotationY, shown, window, cx),
        ];
        if !pie {
            if !view.right_angle_axes {
                body.push(self.number(t("Perspective", "Perspective"), Field::Perspective, shown, window, cx));
            }
            body.push(self.check(
                "view-right-angle",
                t("Right Angle Axes", "Axes à angle droit"),
                view.right_angle_axes,
                undo,
                cx,
                |chart, _, on| {
                    chart.view3d = Some(View3D {
                        right_angle_axes: on,
                        ..chart.view3d_or_default()
                    })
                },
            ));
        }
        body.push(self.number(t("Depth (% of base)", "Profondeur (% de la base)"), Field::Depth, shown, window, cx));
        body.push(
            Button::new("view-default", t("Default Rotation", "Rotation par défaut"))
                .on_click(cx.listener(move |this, _, _, cx| this.apply(undo, cx, |chart, _| chart.view3d = None)))
                .into_any_element(),
        );
        self.section(undo, cx, body)
    }

    /// The tabs of the pane for an element: (title, sections).
    fn tabs(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Vec<(&'static str, Vec<AnyElement>)> {
        let mut tabs = self.element_tabs(shown, window, cx);
        // Whatever is written has its "Text Options" page.
        if text_of(&shown.chart, shown.element).is_some() {
            tabs.push(self.text_tab(shown, window, cx));
        }
        tabs
    }

    /// The pages of options of the selected element itself.
    fn element_tabs(&mut self, shown: &Shown, window: &mut Window, cx: &mut Context<Self>) -> Vec<(&'static str, Vec<AnyElement>)> {
        let chart = &shown.chart;
        let fill_and_line = t("Fill & Line", "Remplissage et contour");
        let border = t("Border", "Bordure");
        match shown.element {
            ChartElement::ChartArea => {
                let mut tabs = vec![(fill_and_line, vec![self.fill_section(shown, true, cx), self.line_section(border, shown, window, cx)])];
                if chart.kind.is_3d() {
                    tabs.push((t("Effects", "Effets"), vec![self.rotation_section(shown, window, cx)]));
                }
                let size = vec![
                    self.number(t("Height", "Hauteur"), Field::Height, shown, window, cx),
                    self.number(t("Width", "Largeur"), Field::Width, shown, window, cx),
                ];
                tabs.push((t("Size & Properties", "Taille et propriétés"), vec![self.section(t("Size", "Taille"), cx, size)]));
                tabs
            }
            ChartElement::PlotArea => vec![(fill_and_line, vec![self.fill_section(shown, true, cx), self.line_section(border, shown, window, cx)])],
            ChartElement::Series(s) => {
                let mut first = Vec::new();
                if is_lined(chart, s as usize) {
                    first.push(self.line_section(t("Line", "Trait"), shown, window, cx));
                    first.extend(self.marker_sections(shown, window, cx));
                    first.push(self.fill_section(shown, false, cx));
                } else {
                    first.push(self.fill_section(shown, false, cx));
                    first.push(self.line_section(border, shown, window, cx));
                }
                vec![(fill_and_line, first), (t("Series Options", "Options des séries"), vec![self.series_options(shown, window, cx)])]
            }
            ChartElement::Point(..) => vec![
                (fill_and_line, vec![self.fill_section(shown, false, cx)]),
                (t("Series Options", "Options des séries"), vec![self.series_options(shown, window, cx)]),
            ],
            ChartElement::Axis(id) => vec![
                (t("Axis Options", "Options d'axe"), self.axis_sections(id, shown, window, cx)),
                (fill_and_line, vec![self.line_section(t("Line", "Trait"), shown, window, cx)]),
            ],
            ChartElement::MajorGridlines(id) | ChartElement::MinorGridlines(id) => {
                let axis = id.of(&chart.axes);
                let undo = t("Gridlines", "Quadrillage");
                let body = vec![
                    widgets::label(axis_name(chart, id)).text_color(c(theme::TEXT_MUTED)).into_any_element(),
                    self.check("grid-major", t("Major Gridlines", "Quadrillage principal"), axis.major_gridlines, undo, cx, move |chart, _, on| {
                        id.of_mut(&mut chart.axes).major_gridlines = on
                    }),
                    self.check("grid-minor", t("Minor Gridlines", "Quadrillage secondaire"), axis.minor_gridlines, undo, cx, move |chart, _, on| {
                        id.of_mut(&mut chart.axes).minor_gridlines = on
                    }),
                ];
                let title = match shown.element {
                    ChartElement::MajorGridlines(_) => t("Major Gridline Options", "Options de quadrillage principal"),
                    _ => t("Minor Gridline Options", "Options de quadrillage secondaire"),
                };
                // As in Excel, how the lines are drawn comes first.
                vec![(fill_and_line, vec![self.line_section(t("Line", "Trait"), shown, window, cx)]), (title, vec![self.section(title, cx, body)])]
            }
            ChartElement::Title => {
                let undo = t("Chart Title", "Titre du graphique");
                let input = self.input(Field::Text, shown, window, cx);
                let body = vec![
                    widgets::label(t("Title Position", "Position du titre")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
                    self.radio(
                        "title-position",
                        &[t("Above Chart", "Au-dessus du graphique"), t("Centered Overlay", "Superposé centré")],
                        Some(chart.title_overlay as usize),
                        undo,
                        cx,
                        |chart, _, ix| chart.title_overlay = ix == 1,
                    ),
                    widgets::label(t("Text", "Texte")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
                    div().flex().child(TextField::new(&input)).into_any_element(),
                    self.check(
                        "title-auto",
                        t("Automatic (the name of the series)", "Automatique (le nom de la série)"),
                        chart.title == ChartTitle::Auto,
                        undo,
                        cx,
                        |chart, _, on| {
                            if on {
                                chart.title = ChartTitle::Auto;
                            }
                        },
                    ),
                ];
                let title = t("Title Options", "Options de titre");
                vec![(title, vec![self.section(title, cx, body)])]
            }
            ChartElement::AxisTitle(_) => {
                let input = self.input(Field::Text, shown, window, cx);
                let body = vec![
                    widgets::label(t("Text", "Texte")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
                    div().flex().child(TextField::new(&input)).into_any_element(),
                ];
                let title = t("Title Options", "Options de titre");
                vec![(title, vec![self.section(title, cx, body)])]
            }
            ChartElement::Legend | ChartElement::LegendEntry(_) => {
                const PLACES: [LegendPos; 5] = [LegendPos::Top, LegendPos::Bottom, LegendPos::Left, LegendPos::Right, LegendPos::TopRight];
                let undo = t("Legend", "Légende");
                let body = vec![
                    widgets::label(t("Legend Position", "Position de la légende")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
                    self.radio(
                        "legend-position",
                        &[t("Top", "Haut"), t("Bottom", "Bas"), t("Left", "Gauche"), t("Right", "Droite"), t("Top Right", "Haut à droite")],
                        PLACES.iter().position(|p| *p == chart.legend),
                        undo,
                        cx,
                        // A side chosen here is where the legend goes,
                        // wherever it had been dragged.
                        |chart, _, ix| {
                            chart.legend = PLACES[ix];
                            chart.legend_layout = None;
                        },
                    ),
                    self.check(
                        "legend-overlay",
                        t(
                            "Show the legend without overlapping the chart",
                            "Afficher la légende sans recouvrir le graphique",
                        ),
                        !chart.legend_overlay,
                        undo,
                        cx,
                        |chart, _, on| chart.legend_overlay = !on,
                    ),
                ];
                let title = t("Legend Options", "Options de légende");
                vec![(title, vec![self.section(title, cx, body)])]
            }
            ChartElement::DataLabels(s) => vec![(t("Label Options", "Options d'étiquette"), self.label_sections(s as usize, shown, window, cx))],
            ChartElement::Trendline(..) => vec![
                (t("Trendline Options", "Options de courbe de tendance"), vec![self.trendline_options(shown, window, cx)]),
                (fill_and_line, vec![self.line_section(t("Line", "Trait"), shown, window, cx)]),
            ],
            ChartElement::ErrorBars(..) => {
                vec![(t("Error Bar Options", "Options de barre d'erreur"), vec![self.error_bar_options(shown, window, cx)])]
            }
            ChartElement::DataTable => {
                let table = chart.data_table.unwrap_or_default();
                let undo = t("Data Table", "Table de données");
                let part = |this: &Self, id: &'static str, label: &'static str, on: bool, cx: &mut Context<Self>, set: fn(&mut DataTable, bool)| {
                    this.check(id, label, on, undo, cx, move |chart, _, on| {
                        let mut table = chart.data_table.unwrap_or_default();
                        set(&mut table, on);
                        chart.data_table = Some(table);
                    })
                };
                let body = vec![
                    widgets::label(t("Table Borders", "Bordures de tableau")).font_weight(FontWeight::SEMIBOLD).into_any_element(),
                    part(self, "table-horizontal", t("Horizontal", "Horizontal"), table.horizontal_lines, cx, |t, on| t.horizontal_lines = on),
                    part(self, "table-vertical", t("Vertical", "Vertical"), table.vertical_lines, cx, |t, on| t.vertical_lines = on),
                    part(self, "table-outline", t("Outline", "Contour"), table.outline, cx, |t, on| t.outline = on),
                    part(self, "table-keys", t("Show legend keys", "Afficher les symboles de légende"), table.legend_keys, cx, |t, on| {
                        t.legend_keys = on
                    }),
                ];
                let title = t("Data Table Options", "Options de table de données");
                vec![(title, vec![self.section(title, cx, body)])]
            }
        }
    }
}

/// Give the labels of an axis, or the data labels of a series, a number
/// format (`None`: that of the cells).
fn store_number_format(chart: &mut Chart, element: ChartElement, code: Option<String>) {
    match element {
        ChartElement::Axis(id) => id.of_mut(&mut chart.axes).number_format = code,
        _ => {
            if let Some(labels) = labels_mut(chart, element) {
                labels.number_format = code;
            }
        }
    }
}

/// A label with its control on the right.
fn row(label: &'static str, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().min_w(px(0.)).child(label))
        .child(control)
        .into_any_element()
}

impl Render for FormatPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.app.upgrade().and_then(|app| {
            let app = app.read(cx);
            let chart = app.selected_chart()?;
            let element = app.chart_element();
            // The bounds left automatic are those the chart is drawn with:
            // laid out here, as the sheet does, they are never a frame late.
            let scale = match element {
                ChartElement::Axis(id) => {
                    let data = app.wb.chart_data(app.sheet, chart);
                    let zoom = app.zoom;
                    let size = (chart.size.0 * zoom, chart.size.1 * zoom);
                    chart_view::layout(chart, &data, size, zoom, window.text_system(), &app.chart_style()).scale(id).cloned()
                }
                _ => None,
            };
            Some(Shown {
                chart: chart.clone(),
                element,
                scale,
                locale: app.wb.locale,
            })
        });
        let Some(shown) = shown else {
            return div()
                .p(px(14.))
                .text_color(c(theme::TEXT_MUTED))
                .child(t("Select a chart to format it.", "Sélectionnez un graphique pour le mettre en forme."))
                .into_any_element();
        };
        let key = (shown.chart.id, shown.element);
        self.refill = self.shown_for != Some(key);
        self.shown_for = Some(key);
        let tabs = self.tabs(&shown, window, cx);
        self.refill = false;
        let tab = self.tab.min(tabs.len().saturating_sub(1));
        let titles: Vec<SharedString> = tabs.iter().map(|(title, _)| (*title).into()).collect();
        let sections = tabs.into_iter().nth(tab).map(|(_, sections)| sections).unwrap_or_default();
        div()
            .id("chart-format-pane")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .when(titles.len() > 1, |d| {
                d.child(div().px(px(8.)).child(TabStrip::new("chart-format-tabs", titles, tab).on_change(cx.listener(
                    |this, ix: &usize, _, cx| {
                        this.tab = *ix;
                        cx.notify();
                    },
                ))))
            })
            .children(sections)
            .into_any_element()
    }
}
