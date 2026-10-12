//! The New Formatting Rule / Edit Formatting Rule dialog: the list of the
//! six rule types and, under it, the description editor of the type
//! chosen. The dialog works on a [`Draft`]; OK hands the rule it makes to
//! whoever opened the dialog (the ribbon adds it to the selection, the
//! Rules Manager puts it in its list).

use super::draft::{At, Contains, Draft, Graded, Kind, Level};
use super::{add_rule, draw, format_sample, words};
use crate::app::ExcelApp;
use crate::features::{Cx, format_cells};
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, Checkbox, ColorButton, GroupBox, ListBox, NumberField, RadioGroup, RefEdit, Select, dialog, icon,
    label, labeled, ok_cancel, tooltip,
};
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, SharedString, Window, anchored, deferred, div, prelude::*, px,
    relative,
};
use opensheet_engine::Locale;
use opensheet_engine::condfmt::{
    Bar, BarAxis, BarDirection, CellOp, DataBar, Icon, IconSet, Rule, TextOp, TimePeriod,
};
use opensheet_engine::style::{Color, StylePatch};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// What the dialog opens on when it comes from an entry of the ribbon
/// menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    ColorScale,
    DataBar,
    IconSet,
    Contains,
    Ranked,
}

/// Who receives the rule and its format when the dialog is accepted.
pub type OnOk = Rc<dyn Fn(&(Rule, StylePatch), &mut Window, &mut App)>;

/// What the dialog is opened with.
pub struct Setup {
    pub title: &'static str,
    pub draft: Draft,
    pub locale: &'static Locale,
    /// The cell formulas are typed for.
    pub active: (u32, u32),
    /// The cell formulas are stored for: the top-left cell of the rule.
    pub anchor: (u32, u32),
    pub on_ok: OnOk,
}

pub fn new_title() -> &'static str {
    t("New Formatting Rule", "Nouvelle règle de mise en forme")
}

pub fn edit_title() -> &'static str {
    t("Edit Formatting Rule", "Modifier la règle de mise en forme")
}

/// New Rule from the ribbon: the rule goes on the selected cells.
pub fn open_new(app: &mut ExcelApp, start: Start, window: &mut Window, cx: &mut Cx) {
    let (kind, style) = match start {
        Start::ColorScale => (Kind::Values, Graded::TwoColor),
        Start::DataBar => (Kind::Values, Graded::DataBar),
        Start::IconSet => (Kind::Values, Graded::IconSet),
        Start::Contains => (Kind::Contains, Graded::TwoColor),
        Start::Ranked => (Kind::Ranked, Graded::TwoColor),
    };
    let active = app.active();
    let anchor = app.sel().ranges.first().map(|r| (r.r0, r.c0)).unwrap_or(active);
    let weak = cx.weak_entity();
    let setup = Setup {
        title: new_title(),
        draft: Draft::new(kind, style),
        locale: app.wb.locale,
        active,
        anchor,
        on_ok: Rc::new(move |(rule, format), _, cx| {
            weak.update(cx, |app, cx| add_rule(app, rule.clone(), format.clone(), cx)).ok();
        }),
    };
    open(app, setup, window, cx);
}

pub fn open(app: &mut ExcelApp, setup: Setup, window: &mut Window, cx: &mut Cx) {
    app.open_modal(window, cx, move |modal, _, cx| RuleDialog::new(modal, setup, cx));
}

/// The boxes values are typed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    /// Low end, middle, high end of a colour scale.
    Stop(usize),
    BarLow,
    BarHigh,
    /// The threshold of a band of an icon set (0 = the band above the
    /// lowest).
    Icon(usize),
    First,
    Second,
    Text,
    Rank,
    Formula,
}

impl Field {
    const ALL: [Field; 14] = [
        Field::Stop(0),
        Field::Stop(1),
        Field::Stop(2),
        Field::BarLow,
        Field::BarHigh,
        Field::Icon(0),
        Field::Icon(1),
        Field::Icon(2),
        Field::Icon(3),
        Field::First,
        Field::Second,
        Field::Text,
        Field::Rank,
        Field::Formula,
    ];

    /// A number unique to the field, for element ids.
    fn index(self) -> usize {
        Field::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// Where the text of the field lives in a draft.
    fn slot(self, draft: &mut Draft) -> Option<&mut String> {
        Some(match self {
            Field::Stop(i) => &mut draft.stops[i].value,
            Field::BarLow => &mut draft.bar_low.value,
            Field::BarHigh => &mut draft.bar_high.value,
            Field::Icon(i) => &mut draft.icon_rows.get_mut(i)?.level.value,
            Field::First => &mut draft.first,
            Field::Second => &mut draft.second,
            Field::Text => &mut draft.text,
            Field::Rank => &mut draft.rank,
            Field::Formula => &mut draft.formula,
        })
    }

    /// The threshold the field belongs to, when it belongs to one.
    fn level(self, draft: &mut Draft) -> Option<&mut Level> {
        match self {
            Field::Stop(i) => Some(&mut draft.stops[i]),
            Field::BarLow => Some(&mut draft.bar_low),
            Field::BarHigh => Some(&mut draft.bar_high),
            Field::Icon(i) => draft.icon_rows.get_mut(i).map(|row| &mut row.level),
            _ => None,
        }
    }
}

/// The picture lists of the icon set editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drop {
    /// "Icon Style".
    Set,
    /// The icon of a band (0 = lowest).
    Band(usize),
}

const FIELD_BORDER: u32 = 0xABABAB;

struct RuleDialog {
    modal: Modal,
    title: &'static str,
    locale: &'static Locale,
    active: (u32, u32),
    anchor: (u32, u32),
    on_ok: OnOk,
    /// Everything but the texts being typed, which are in `inputs`.
    draft: Draft,
    inputs: HashMap<Field, Entity<TextInput>>,
    /// The open picture list, and when a press elsewhere last closed one.
    drop: Option<Drop>,
    drop_closed: Option<Instant>,
}

impl RuleDialog {
    fn new(modal: Modal, setup: Setup, cx: &mut Context<Self>) -> Self {
        // A cell picked on the sheet is written as `=$A$1`.
        let style = RefStyle {
            absolute: true,
            sheet: false,
            equals: true,
        };
        let mut draft = setup.draft;
        let mut inputs = HashMap::new();
        for field in Field::ALL {
            let text = field.slot(&mut draft).cloned().unwrap_or_default();
            let input = match field {
                Field::Rank => {
                    let input = widgets::number_input(cx, None, 1.0, 1000.0, 1.0);
                    input.update(cx, |input, cx| input.set_text(text, cx));
                    input
                }
                _ => modal.ref_input(style, text, cx),
            };
            inputs.insert(field, input);
        }
        RuleDialog {
            modal,
            title: setup.title,
            locale: setup.locale,
            active: setup.active,
            anchor: setup.anchor,
            on_ok: setup.on_ok,
            draft,
            inputs,
            drop: None,
            drop_closed: None,
        }
    }

    fn input(&self, field: Field) -> &Entity<TextInput> {
        &self.inputs[&field]
    }

    /// The draft with what the boxes hold right now.
    fn current(&self, cx: &App) -> Draft {
        let mut draft = self.draft.clone();
        for field in Field::ALL {
            if let Some(slot) = field.slot(&mut draft) {
                *slot = self.input(field).read(cx).text().to_string();
            }
        }
        draft
    }

    /// Change the draft; the boxes follow what the change did to their
    /// texts and keep what was typed in the others.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Draft)) {
        let mut draft = self.current(cx);
        change(&mut draft);
        for field in Field::ALL {
            let Some(text) = field.slot(&mut draft).cloned() else {
                continue;
            };
            let input = self.input(field).clone();
            if input.read(cx).text() != text {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        self.draft = draft;
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.current(cx);
        match draft.rule(self.locale, self.active, self.anchor) {
            Ok(rule) => {
                let format = if draft.has_format() { draft.format } else { StylePatch::default() };
                let on_ok = self.on_ok.clone();
                self.modal.close(window, cx);
                on_ok(&(rule, format), window, cx);
            }
            Err(message) => {
                self.modal
                    .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
            }
        }
    }

    fn choose_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.draft.format.clone();
        let chosen = cx.listener(|this, format: &StylePatch, _, cx| {
            this.draft.format = format.clone();
            cx.notify();
        });
        self.modal
            .update_app(cx, |app, cx| format_cells::open_partial(app, start, window, cx, chosen));
    }

    fn open_axis_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bar = self.draft.bar.clone();
        let chosen = cx.listener(|this, bar: &DataBar, _, cx| {
            this.draft.bar = bar.clone();
            cx.notify();
        });
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, _| AxisDialog {
                modal,
                bar,
                on_ok: Rc::new(chosen),
            });
        });
    }

    // ---- small builders -----------------------------------------------------

    fn names(items: impl IntoIterator<Item = &'static str>) -> Vec<SharedString> {
        items.into_iter().map(SharedString::from).collect()
    }

    /// The line that says what the editor under it sets.
    fn heading(text: &'static str) -> gpui::Div {
        div().font_weight(FontWeight::SEMIBOLD).child(text)
    }

    /// A list bound to a choice of the draft.
    fn choice(
        &self,
        id: &'static str,
        options: Vec<SharedString>,
        selected: usize,
        width: f32,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Draft, usize) + 'static,
    ) -> Select {
        Select::new(id, options, Some(selected))
            .width(width)
            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                let ix = *ix;
                this.edit(cx, |draft| set(draft, ix));
            }))
    }

    /// The "Type" list of a threshold. `place` tells the low end (0), the
    /// middle (1) and the high end (2) apart, for the value a type starts
    /// with.
    fn type_select(&self, field: Field, options: &'static [At], place: usize, width: f32, cx: &mut Context<Self>) -> Select {
        let mut draft = self.draft.clone();
        let at = field.level(&mut draft).map(|level| level.at);
        let names = Self::names(options.iter().map(|at| at.name()));
        Select::new(("type", field.index()), names, options.iter().position(|option| Some(*option) == at))
            .width(width)
            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                let at = options[*ix];
                this.edit(cx, |draft| {
                    if let Some(level) = field.level(draft).filter(|level| level.at != at) {
                        level.at = at;
                        level.value = at.default_value(place).to_string();
                    }
                });
            }))
    }

    /// The "Value" box of a threshold: a range field, or the name of what
    /// a type without a value stands for.
    fn value_box(&self, field: Field, width: f32) -> AnyElement {
        let mut draft = self.draft.clone();
        let at = field.level(&mut draft).map(|level| level.at).unwrap_or(At::Number);
        if at.has_value() {
            return RefEdit::new(self.input(field), &self.modal).width(width).into_any_element();
        }
        div()
            .w(px(width))
            .h(px(widgets::CONTROL_H))
            .flex_none()
            .flex()
            .items_center()
            .px(px(6.))
            .bg(c(theme::CHROME_BG))
            .border_1()
            .border_color(c(theme::BORDER))
            .rounded(px(3.))
            .text_color(c(theme::TEXT_DISABLED))
            .child(at.placeholder())
            .into_any_element()
    }

    /// A column of controls under a title, as wide as `width`.
    fn column(title: &'static str, width: f32, rows: Vec<AnyElement>) -> gpui::Div {
        div()
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().h(px(18.)).child(title))
            .children(rows)
    }

    /// The names of the rows of such columns.
    fn row_names(names: &[&'static str]) -> gpui::Div {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().h(px(18.)))
            .children(names.iter().map(|name| div().h(px(widgets::CONTROL_H)).flex().items_center().child(*name)))
    }

    fn color_button(
        &self,
        id: &'static str,
        color: Color,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Draft, Color) + 'static,
    ) -> ColorButton {
        ColorButton::new(id, Some(Some(color)))
            .more(&self.modal.app)
            .width(120.0)
            .on_pick(cx.listener(move |this, color: &Option<Color>, _, cx| {
                if let Some(color) = *color {
                    this.edit(cx, |draft| set(draft, color));
                }
            }))
    }

    /// "Preview:" with the format of the rule and the button that changes
    /// it.
    fn format_row(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .pt(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(label(t("Preview:", "Aperçu :")))
            .child(format_sample(&self.draft.format, 220.0, 40.0))
            .child(
                Button::new("format", t("Format...", "Format..."))
                    .on_click(cx.listener(|this, _, window, cx| this.choose_format(window, cx))),
            )
    }

    // ---- picture lists --------------------------------------------------------

    fn toggle_drop(&mut self, drop: Drop, cx: &mut Context<Self>) {
        // The press that closed the list is not the click that opens it.
        if self.drop_closed.take().is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
            return;
        }
        self.drop = if self.drop == Some(drop) { None } else { Some(drop) };
        cx.notify();
    }

    /// A box showing `face` that drops `panel` down when clicked.
    fn drop_button(&self, drop: Drop, face: AnyElement, width: f32, panel: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let id = match drop {
            Drop::Set => 0,
            Drop::Band(band) => band + 1,
        };
        div()
            .relative()
            .flex_none()
            .w(px(width))
            .child(
                div()
                    .id(("drop", id))
                    .h(px(widgets::CONTROL_H))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(4.))
                    .pl(px(6.))
                    .pr(px(4.))
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(FIELD_BORDER))
                    .rounded(px(3.))
                    .child(face)
                    .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_drop(drop, cx))),
            )
            .children(panel.map(|panel| {
                div().absolute().left_0().top(relative(1.)).child(
                    deferred(
                        anchored().snap_to_window_with_margin(px(6.)).child(
                            div()
                                .id("drop-panel")
                                .occlude()
                                .mt(px(1.))
                                .p(px(4.))
                                .bg(c(0xFFFFFF))
                                .border_1()
                                .border_color(c(theme::BORDER_STRONG))
                                .rounded(px(3.))
                                .shadow_md()
                                .text_size(px(12.))
                                .text_color(c(theme::TEXT))
                                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.drop = None;
                                    this.drop_closed = Some(Instant::now());
                                    cx.notify();
                                }))
                                .child(panel),
                        ),
                    )
                    .with_priority(3),
                )
            }))
            .into_any_element()
    }

    /// The list of the "Icon Style" box: the twenty sets, two by two.
    fn set_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.draft.shown_set();
        let cells: Vec<AnyElement> = IconSet::ALL
            .iter()
            .enumerate()
            .map(|(i, set)| {
                let set = *set;
                div()
                    .id(("set", i))
                    .w(px(118.))
                    .h(px(26.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .when(shown == Some(set), |d| d.bg(c(theme::ACCENT_LIGHT)))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(draw::set_picture(set, 16.0, 3.0))
                    .tooltip(tooltip(words::set_name(set)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.drop = None;
                        this.edit(cx, |draft| draft.choose_set(set));
                    }))
                    .into_any_element()
            })
            .collect();
        Self::two_columns(cells)
    }

    /// The list of an icon box: "No Cell Icon", then the icons set by set.
    fn icon_panel(&self, band: usize, cx: &mut Context<Self>) -> AnyElement {
        let current = self.draft.icons.get(band).copied().flatten();
        let pick = |picked: Option<(IconSet, u8)>| {
            cx.listener(move |this: &mut Self, _: &gpui::ClickEvent, _, cx| {
                this.drop = None;
                this.edit(cx, |draft| {
                    if let Some(slot) = draft.icons.get_mut(band) {
                        *slot = picked;
                    }
                });
            })
        };
        let none = div()
            .id("no-icon")
            .h(px(24.))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(3.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(t("No Cell Icon", "Aucune icône de cellule"))
            .on_click(pick(None));
        let sets: Vec<AnyElement> = IconSet::ALL
            .iter()
            .enumerate()
            .map(|(s, set)| {
                // The icon of the highest values first, as in the gallery.
                let icons = (0..set.len() as u8).rev().map(|index| {
                    div()
                        .id(("icon", s * 8 + index as usize))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .when(current == Some((*set, index)), |d| d.bg(c(theme::ACCENT_LIGHT)))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .child(draw::icon_picture(set.icons()[index as usize], 16.0))
                        .on_click(pick(Some((*set, index))))
                });
                div().w(px(118.)).h(px(24.)).flex().items_center().children(icons).into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(none)
            .child(Self::two_columns(sets))
            .into_any_element()
    }

    fn two_columns(cells: Vec<AnyElement>) -> AnyElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut cells = cells.into_iter().peekable();
        while cells.peek().is_some() {
            let pair: Vec<AnyElement> = cells.by_ref().take(2).collect();
            rows.push(div().flex().gap(px(4.)).children(pair).into_any_element());
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    // ---- Format all cells based on their values -------------------------------

    fn values_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let style = self.draft.style;
        let styles = Self::names(Graded::ALL.iter().map(|style| style.name()));
        let style_ix = Graded::ALL.iter().position(|s| *s == style).unwrap_or(0);
        let mut top = div().flex().items_center().gap(px(16.)).child(labeled(
            t("Format Style:", "Style de mise en forme :"),
            self.choice("style", styles, style_ix, 180.0, cx, |draft, ix| draft.style = Graded::ALL[ix]),
        ));
        top = match style {
            Graded::DataBar => top.child(
                Checkbox::new("bar-only", t("Show Bar Only", "Afficher la barre uniquement"), !self.draft.bar.show_value)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(cx, |draft| draft.bar.show_value = !on);
                    })),
            ),
            Graded::IconSet => top.child(
                Button::new("reverse", t("Reverse Icon Order", "Ordre inverse des icônes"))
                    .on_click(cx.listener(|this, _, _, cx| this.edit(cx, |draft| draft.icons.reverse()))),
            ),
            _ => top,
        };
        let mut out = vec![
            Self::heading(t(
                "Format all cells based on their values:",
                "Appliquer une mise en forme à toutes les cellules d'après leur valeur :",
            ))
            .into_any_element(),
            top.into_any_element(),
        ];
        out.extend(match style {
            Graded::TwoColor | Graded::ThreeColor => self.scale_editor(style == Graded::ThreeColor, cx),
            Graded::DataBar => self.bar_editor(cx),
            Graded::IconSet => self.icon_editor(cx),
        });
        out
    }

    fn scale_editor(&self, three: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let width = if three { 170.0 } else { 220.0 };
        let stop = |place: usize, title: &'static str, options: &'static [At], id: &'static str, this: &Self, cx: &mut Context<Self>| {
            Self::column(
                title,
                width,
                vec![
                    this.type_select(Field::Stop(place), options, place, width, cx).into_any_element(),
                    this.value_box(Field::Stop(place), width),
                    this.color_button(id, this.draft.colors[place], cx, move |draft, color| draft.colors[place] = color)
                        .into_any_element(),
                ],
            )
        };
        let mut columns = div()
            .flex()
            .gap(px(12.))
            .child(Self::row_names(&[t("Type:", "Type :"), t("Value:", "Valeur :"), t("Color:", "Couleur :")]))
            .child(stop(0, t("Minimum", "Minimum"), &At::SCALE_LOW, "color-low", self, cx));
        if three {
            columns = columns.child(stop(1, t("Midpoint", "Milieu"), &At::MIDDLE, "color-mid", self, cx));
        }
        columns = columns.child(stop(2, t("Maximum", "Maximum"), &At::SCALE_HIGH, "color-high", self, cx));

        let colors: Vec<Color> = match three {
            true => self.draft.colors.to_vec(),
            false => vec![self.draft.colors[0], self.draft.colors[2]],
        };
        let strip = draw::picture(if three { 534.0 } else { 452.0 }, 22.0, move |bounds, out| {
            draw::scale_strip(&colors, bounds, out)
        });
        vec![
            columns.into_any_element(),
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(label(t("Preview:", "Aperçu :")))
                .child(div().border_1().border_color(c(theme::BORDER_STRONG)).child(strip))
                .into_any_element(),
        ]
    }

    fn bar_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bar = &self.draft.bar;
        let end = |field: Field, place: usize, title: &'static str, options: &'static [At], this: &Self, cx: &mut Context<Self>| {
            Self::column(
                title,
                220.0,
                vec![
                    this.type_select(field, options, place, 220.0, cx).into_any_element(),
                    this.value_box(field, 220.0),
                ],
            )
        };
        let ends = div()
            .flex()
            .gap(px(12.))
            .child(Self::row_names(&[t("Type:", "Type :"), t("Value:", "Valeur :")]))
            .child(end(Field::BarLow, 0, t("Minimum", "Minimum"), &At::BAR_LOW, self, cx))
            .child(end(Field::BarHigh, 2, t("Maximum", "Maximum"), &At::BAR_HIGH, self, cx));

        let fills = Self::names([t("Solid Fill", "Remplissage uni"), t("Gradient Fill", "Dégradé")]);
        let borders = Self::names([t("No Border", "Aucune bordure"), t("Solid Border", "Bordure pleine")]);
        let titled = |title: &'static str, control: AnyElement| {
            div().flex().flex_col().gap(px(3.)).child(label(title)).child(control)
        };
        let mut appearance = div()
            .flex()
            .items_end()
            .gap(px(12.))
            .child(titled(
                t_ctx("bar", "Fill", "Remplissage"),
                self.choice("bar-fill-kind", fills, bar.gradient as usize, 130.0, cx, |draft, ix| draft.bar.gradient = ix == 1)
                    .into_any_element(),
            ))
            .child(titled(
                t("Color", "Couleur"),
                self.color_button("bar-fill", bar.fill, cx, |draft, color| draft.bar.fill = color)
                    .into_any_element(),
            ))
            .child(titled(
                t("Border", "Bordure"),
                self.choice("bar-border-kind", borders, bar.border.is_some() as usize, 130.0, cx, |draft, ix| {
                    // A new border starts black, as Excel's does.
                    draft.bar.border = (ix == 1).then(|| draft.bar.border.unwrap_or(Color::BLACK));
                })
                .into_any_element(),
            ));
        if let Some(border) = bar.border {
            appearance = appearance.child(titled(
                t("Color", "Couleur"),
                self.color_button("bar-border", border, cx, |draft, color| draft.bar.border = Some(color))
                    .into_any_element(),
            ));
        }

        let directions = Self::names([
            t("Context", "Contexte"),
            t("Left-to-Right", "De gauche à droite"),
            t("Right-to-Left", "De droite à gauche"),
        ]);
        let direction_ix = BarDirection::ALL.iter().position(|d| *d == bar.direction).unwrap_or(0);
        let sample = Bar {
            start: if bar.direction == BarDirection::RightToLeft { 1.0 } else { 0.0 },
            end: if bar.direction == BarDirection::RightToLeft { 0.25 } else { 0.75 },
            fill: bar.fill,
            gradient: bar.gradient,
            border: bar.border,
            negative: false,
            axis: None,
            axis_color: bar.axis_color,
        };
        let preview = draw::picture(160.0, 22.0, move |bounds, out| {
            let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            draw::bar(&sample, (x, y, x + 160.0, y + 23.0), 1.0, out);
        });
        vec![
            ends.into_any_element(),
            label(t("Bar Appearance:", "Apparence de la barre :")).into_any_element(),
            appearance.into_any_element(),
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .child(
                    Button::new("axis", t("Negative Value and Axis...", "Valeur négative et axe..."))
                        .on_click(cx.listener(|this, _, window, cx| this.open_axis_settings(window, cx))),
                )
                .child(labeled(
                    t("Bar Direction:", "Orientation de la barre :"),
                    self.choice("bar-direction", directions, direction_ix, 150.0, cx, |draft, ix| {
                        draft.bar.direction = BarDirection::ALL[ix]
                    }),
                ))
                .into_any_element(),
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(label(t("Preview:", "Aperçu :")))
                .child(div().bg(c(0xFFFFFF)).border_1().border_color(c(theme::BORDER_STRONG)).child(preview))
                .into_any_element(),
        ]
    }

    fn icon_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let draft = &self.draft;
        let bands = draft.icons.len();
        let style_face: AnyElement = match draft.shown_set() {
            Some(set) => draw::set_picture(set, 16.0, 3.0).into_any_element(),
            None => div().child(t_ctx("icons", "Custom", "Personnalisé")).into_any_element(),
        };
        let set_panel = (self.drop == Some(Drop::Set)).then(|| self.set_panel(cx));
        let top = div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(labeled(
                t("Icon Style:", "Style d'icône :"),
                self.drop_button(Drop::Set, style_face, 140.0, set_panel, cx),
            ))
            .child(
                Checkbox::new("icon-only", t("Show Icon Only", "Afficher l'icône uniquement"), draft.show_icon_only)
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.edit(cx, |draft| draft.show_icon_only = on);
                    })),
            );
        let header = div()
            .flex()
            .gap(px(8.))
            .text_color(c(theme::TEXT_MUTED))
            .child(div().w(px(64.)).child(t("Icon", "Icône")))
            .child(div().w(px(150.)))
            .child(div().w(px(60.)))
            .child(div().w(px(130.)).child(t("Value", "Valeur")))
            .child(div().w(px(120.)).child(t("Type", "Type")));

        // The band of the highest values comes first; each of the others
        // starts where the one above it stops.
        let mut rows: Vec<AnyElement> = Vec::new();
        for band in (0..bands).rev() {
            let face: AnyElement = match draft.icons[band] {
                Some((set, index)) => draw::icon_picture(icon_of(set, index), 16.0).into_any_element(),
                None => div().size(px(16.)).into_any_element(),
            };
            let panel = (self.drop == Some(Drop::Band(band))).then(|| self.icon_panel(band, cx));
            let when = match draft.icon_rows.get(band) {
                // The band above this one: its threshold bounds this band.
                Some(above) => {
                    let sign = if above.gte { "<" } else { "<=" };
                    let value = self.input(Field::Icon(band)).read(cx).text().to_string();
                    let and = if band > 0 { format!(" {}", t("and", "et")) } else { String::new() };
                    format!("{} {sign} {value}{and}", t("when", "si"))
                }
                None => t("when value is", "si la valeur est").to_string(),
            };
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(self.drop_button(Drop::Band(band), face, 64.0, panel, cx))
                .child(div().w(px(150.)).truncate().child(when));
            if band > 0 {
                let field = Field::Icon(band - 1);
                let gte = draft.icon_rows[band - 1].gte;
                row = row
                    .child(
                        Select::new(("gte", band), Self::names([">=", ">"]), Some(if gte { 0 } else { 1 }))
                            .width(60.0)
                            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                                let gte = *ix == 0;
                                this.edit(cx, |draft| {
                                    if let Some(row) = draft.icon_rows.get_mut(band - 1) {
                                        row.gte = gte;
                                    }
                                });
                            })),
                    )
                    .child(self.value_box(field, 130.0))
                    .child(self.type_select(field, &At::MIDDLE, 1, 120.0, cx));
            }
            rows.push(row.into_any_element());
        }
        let mut out = vec![
            top.into_any_element(),
            label(t(
                "Display each icon according to these rules:",
                "Afficher chaque icône en fonction de ces règles :",
            ))
            .into_any_element(),
            header.into_any_element(),
        ];
        out.extend(rows);
        out
    }

    // ---- the other five rule types ----------------------------------------------

    fn contains_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let draft = &self.draft;
        let kinds = Self::names(Contains::ALL.iter().map(|kind| kind.name()));
        let kind_ix = Contains::ALL.iter().position(|k| *k == draft.contains).unwrap_or(0);
        let mut row = div().flex().items_center().gap(px(8.)).child(self.choice(
            "contains",
            kinds,
            kind_ix,
            150.0,
            cx,
            |draft, ix| draft.contains = Contains::ALL[ix],
        ));
        row = match draft.contains {
            Contains::CellValue => {
                let ops = Self::names(CellOp::ALL.iter().map(|op| words::cell_op_name(*op)));
                let op_ix = CellOp::ALL.iter().position(|op| *op == draft.cell_op).unwrap_or(0);
                let two = draft.cell_op.takes_two();
                let width = if two { 110.0 } else { 240.0 };
                row = row
                    .child(self.choice("cell-op", ops, op_ix, 170.0, cx, |draft, ix| draft.cell_op = CellOp::ALL[ix]))
                    .child(RefEdit::new(self.input(Field::First), &self.modal).width(width));
                match two {
                    true => row
                        .child(label(t("and", "et")))
                        .child(RefEdit::new(self.input(Field::Second), &self.modal).width(width)),
                    false => row,
                }
            }
            Contains::Text => {
                let ops = Self::names(TextOp::ALL.iter().map(|op| words::text_op_name(*op)));
                let op_ix = TextOp::ALL.iter().position(|op| *op == draft.text_op).unwrap_or(0);
                row.child(self.choice("text-op", ops, op_ix, 160.0, cx, |draft, ix| draft.text_op = TextOp::ALL[ix]))
                    .child(RefEdit::new(self.input(Field::Text), &self.modal).width(250.0))
            }
            Contains::Dates => {
                let periods = Self::names(TimePeriod::ALL.iter().map(|period| words::period_name(*period)));
                let period_ix = TimePeriod::ALL.iter().position(|p| *p == draft.period).unwrap_or(0);
                row.child(self.choice("period", periods, period_ix, 210.0, cx, |draft, ix| draft.period = TimePeriod::ALL[ix]))
            }
            _ => row,
        };
        vec![
            Self::heading(t(
                "Format only cells with:",
                "Appliquer une mise en forme uniquement aux cellules contenant :",
            ))
            .into_any_element(),
            row.into_any_element(),
            self.format_row(cx).into_any_element(),
        ]
    }

    fn ranked_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let sides = Self::names([t_ctx("rank", "Top", "premiers"), t_ctx("rank", "Bottom", "derniers")]);
        let row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.choice("side", sides, self.draft.bottom as usize, 110.0, cx, |draft, ix| draft.bottom = ix == 1))
            .child(NumberField::new("rank", self.input(Field::Rank)).width(70.0))
            .child(
                Checkbox::new(
                    "percent",
                    t("% of the selected range", "% de la plage sélectionnée"),
                    self.draft.percent,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    let on = *on;
                    this.edit(cx, |draft| draft.percent = on);
                })),
            );
        vec![
            Self::heading(t(
                "Format values that rank in the:",
                "Appliquer une mise en forme aux valeurs situées dans la plage :",
            ))
            .into_any_element(),
            row.into_any_element(),
            self.format_row(cx).into_any_element(),
        ]
    }

    fn average_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.choice(
                "average",
                Self::names(words::average_names()),
                self.draft.average,
                190.0,
                cx,
                |draft, ix| draft.average = ix,
            ))
            .child(label(t("the average for the selected range", "de la moyenne de la plage sélectionnée")));
        vec![
            Self::heading(t("Format values that are:", "Appliquer une mise en forme aux valeurs qui sont :")).into_any_element(),
            row.into_any_element(),
            self.format_row(cx).into_any_element(),
        ]
    }

    fn unique_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let kinds = Self::names([t("duplicate", "valeurs en double"), t("unique", "valeurs uniques")]);
        let row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.choice("unique", kinds, self.draft.unique as usize, 160.0, cx, |draft, ix| draft.unique = ix == 1))
            .child(label(t("values in the selected range", "dans la plage sélectionnée")));
        vec![
            Self::heading(t("Format all:", "Appliquer une mise en forme à toutes les valeurs :")).into_any_element(),
            row.into_any_element(),
            self.format_row(cx).into_any_element(),
        ]
    }

    fn formula_editor(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            Self::heading(t(
                "Format values where this formula is true:",
                "Appliquer une mise en forme aux valeurs pour lesquelles cette formule est vraie :",
            ))
            .into_any_element(),
            div()
                .flex()
                .child(RefEdit::new(self.input(Field::Formula), &self.modal))
                .into_any_element(),
            self.format_row(cx).into_any_element(),
        ]
    }
}

/// The picture of an icon of a set; a blank disc for an index the set
/// does not have (a file may hold one).
fn icon_of(set: IconSet, index: u8) -> Icon {
    set.icons().get(index as usize).copied().unwrap_or(Icon::WhiteCircle)
}

impl Render for RuleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds: Vec<SharedString> = Kind::ALL.iter().map(|kind| format!("► {}", kind.name()).into()).collect();
        let kind_ix = Kind::ALL.iter().position(|kind| *kind == self.draft.kind).unwrap_or(0);
        let editor = match self.draft.kind {
            Kind::Values => self.values_editor(cx),
            Kind::Contains => self.contains_editor(cx),
            Kind::Ranked => self.ranked_editor(cx),
            Kind::Average => self.average_editor(cx),
            Kind::Unique => self.unique_editor(cx),
            Kind::Formula => self.formula_editor(cx),
        };
        dialog(&self.modal, self.title)
            .width(680.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Select a Rule Type:", "Sélectionnez un type de règle :")))
            .child(
                ListBox::new("kind", kinds, Some(kind_ix))
                    .height(widgets::ROW_H * Kind::ALL.len() as f32 + 2.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        let kind = Kind::ALL[*ix];
                        this.drop = None;
                        this.edit(cx, |draft| draft.kind = kind);
                    })),
            )
            .child(label(t("Edit the Rule Description:", "Modifier la description de la règle :")))
            .child(
                div()
                    .min_h(px(270.))
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .rounded(px(3.))
                    .children(editor),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------- Negative Value and Axis Settings

/// The colours of the bars of negative numbers and where the axis goes.
struct AxisDialog {
    modal: Modal,
    bar: DataBar,
    on_ok: Rc<dyn Fn(&DataBar, &mut Window, &mut App)>,
}

/// The colour a "same as positive" choice changes to when it is given a
/// colour of its own: Excel's red.
const NEGATIVE_RED: Color = Color(0xFF0000);

impl AxisDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (bar, on_ok) = (self.bar.clone(), self.on_ok.clone());
        self.modal.close(window, cx);
        on_ok(&bar, window, cx);
    }

    /// Two choices, the first of which comes with a colour.
    fn color_choice(
        &self,
        id: &'static str,
        own: &'static str,
        same: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        get: fn(&DataBar) -> Option<Color>,
        set: fn(&mut DataBar, Option<Color>),
    ) -> gpui::Div {
        let options = vec![own.into(), same.into()];
        let color = get(&self.bar);
        let mut row = div().flex().gap(px(12.)).child(
            RadioGroup::new(id, options, Some(if color.is_some() { 0 } else { 1 }))
                .disabled(!enabled)
                .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                    let own = (*ix == 0).then(|| get(&this.bar).unwrap_or(NEGATIVE_RED));
                    set(&mut this.bar, own);
                    cx.notify();
                })),
        );
        if let (Some(color), true) = (color, enabled) {
            row = row.child(
                ColorButton::new((id, 1usize), Some(Some(color)))
                    .more(&self.modal.app)
                    .width(110.0)
                    .on_pick(cx.listener(move |this, picked: &Option<Color>, _, cx| {
                        if let Some(picked) = *picked {
                            set(&mut this.bar, Some(picked));
                            cx.notify();
                        }
                    })),
            );
        }
        row
    }
}

impl Render for AxisDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let axes: Vec<SharedString> = vec![
            t(
                "Automatic (display at variable position based on negative values)",
                "Automatique (afficher à une position variable en fonction des valeurs négatives)",
            )
            .into(),
            t("Cell midpoint", "Milieu de cellule").into(),
            t(
                "None (show negative value bars in same direction as positive)",
                "Aucun (afficher les barres de valeurs négatives dans la même direction que les valeurs positives)",
            )
            .into(),
        ];
        let axis_ix = BarAxis::ALL.iter().position(|axis| *axis == self.bar.axis).unwrap_or(0);
        dialog(&self.modal, t("Negative Value and Axis Settings", "Paramètres de valeur négative et d'axe"))
            .width(600.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                GroupBox::new(t("Negative bar fill color", "Couleur de remplissage de barre négative")).child(self.color_choice(
                    "negative-fill",
                    t("Fill color:", "Couleur de remplissage :"),
                    t(
                        "Apply same fill color as positive bar",
                        "Appliquer la même couleur de remplissage que la barre positive",
                    ),
                    true,
                    cx,
                    |bar| bar.negative_fill,
                    |bar, color| bar.negative_fill = color,
                )),
            )
            .child(
                GroupBox::new(t("Negative bar border color", "Couleur de bordure de barre négative")).child(self.color_choice(
                    "negative-border",
                    t("Border color:", "Couleur de bordure :"),
                    t(
                        "Apply same border color as positive bar",
                        "Appliquer la même couleur de bordure que la barre positive",
                    ),
                    // Only a bar with a border has one to colour.
                    self.bar.border.is_some(),
                    cx,
                    |bar| bar.negative_border,
                    |bar, color| bar.negative_border = color,
                )),
            )
            .child(
                GroupBox::new(t("Axis settings", "Paramètres de l'axe"))
                    .child(div().line_height(px(17.)).child(t(
                        "Select axis position in cell to change the appearance of bars for negative values",
                        "Sélectionner la position de l'axe dans la cellule pour changer l'apparence des barres pour les valeurs négatives",
                    )))
                    .child(RadioGroup::new("axis", axes, Some(axis_ix)).on_change(cx.listener(
                        |this, ix: &usize, _, cx| {
                            this.bar.axis = BarAxis::ALL[*ix];
                            cx.notify();
                        },
                    )))
                    .child(labeled(
                        t("Axis color:", "Couleur de l'axe :"),
                        ColorButton::new("axis-color", Some(Some(self.bar.axis_color)))
                            .more(&self.modal.app)
                            .width(110.0)
                            .on_pick(cx.listener(|this, picked: &Option<Color>, _, cx| {
                                if let Some(picked) = *picked {
                                    this.bar.axis_color = picked;
                                    cx.notify();
                                }
                            })),
                    )),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
