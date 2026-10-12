//! The dialogs of a PivotTable and of its fields: Value Field Settings,
//! Field Settings, PivotTable Options, Grouping and Show Report Filter
//! Pages.

use super::logic::{self, ShowKind, Target};
use super::{actions, active_pivot, active_target, perform_in_dialog, update_in_dialog};
use crate::app::ExcelApp;
use crate::features::{Cx, format_cells};
use crate::i18n::{t, t_ctx};
use crate::shell::Modal;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, Checkbox, GroupBox, ListBox, NumberField, RadioGroup, Select, TabStrip, TextField, dialog, label,
    labeled,
};
use gpui::{AnyElement, Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::pivot::{
    Aggregation, BaseItem, FieldLayout, GroupBy, GroupKind, PivotError, PivotTable, Subtotals, data_field_name,
};
use opensheet_engine::{Locale, Value};
use std::sync::Arc;

/// "Field Settings" of the ribbon: the settings of the field under the
/// active cell, or of its value field.
pub(super) fn open_settings(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(id) = active_pivot(app).map(|p| p.id) else {
        return;
    };
    match active_target(app) {
        Some(Target::Data(data)) => open_value_settings(app, id, data, 0, window, cx),
        Some(Target::Field(field)) => open_field_settings(app, id, field, window, cx),
        None => {}
    }
}

/// The row of buttons of a settings dialog: something on the left, OK and
/// Cancel on the right.
fn footer(modal: &Modal, left: impl IntoElement, ok: impl Fn(&(), &mut Window, &mut gpui::App) + 'static) -> impl IntoElement {
    let modal = modal.clone();
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(left)
        .child(div().flex_1())
        .child(Button::new("dialog-ok", t("OK", "OK")).primary().on_click(ok))
        .child(Button::new("dialog-cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx)))
}

/// "Source Name" and "Custom Name", the head of both settings dialogs.
fn names(source: &str, name: &Entity<TextInput>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(label(t("Source Name:", "Nom de la source :")))
                .child(div().truncate().child(source.to_string())),
        )
        .child(labeled(t("Custom Name:", "Nom personnalisé :"), TextField::new(name)))
}

// ------------------------------------------------------ value field settings

/// Open Value Field Settings on a tab (0: Summarize Values By, 1: Show
/// Values As).
pub(super) fn open_value_settings(app: &mut ExcelApp, id: u32, data: usize, tab: usize, window: &mut Window, cx: &mut Cx) {
    if let Some(mut opening) = value_opening(app, id, data) {
        opening.tab = tab;
        app.open_modal(window, cx, move |modal, _, cx| ValueSettings::new(modal, opening, cx));
    }
}

/// Open Value Field Settings on "Show Values As" with a calculation
/// chosen, for the user to pick its base field (and item).
pub(super) fn open_show_values_as(app: &mut ExcelApp, id: u32, data: usize, kind: ShowKind, window: &mut Window, cx: &mut Cx) {
    if let Some(mut opening) = value_opening(app, id, data) {
        opening.tab = 1;
        opening.choose(kind);
        app.open_modal(window, cx, move |modal, _, cx| ValueSettings::new(modal, opening, cx));
    }
}

/// What Value Field Settings shows of a value field.
fn value_opening(app: &ExcelApp, id: u32, data: usize) -> Option<ValueOpening> {
    let locale = app.wb.locale;
    let (sheet, pivot) = app.wb.pivot(id)?;
    let d = pivot.data_fields.get(data)?;
    let caption = pivot.field_caption(d.field).to_string();
    let (kind, base, item) = ShowKind::of(&d.show_as);
    let bases: Vec<Base> = logic::base_fields(pivot)
        .into_iter()
        .map(|field| Base {
            field,
            caption: pivot.field_caption(field).to_string().into(),
            items: pivot
                .items_in_order(field)
                .into_iter()
                .map(|x| (x, pivot.item_text(field, x, locale).into()))
                .collect(),
        })
        .collect();
    // A value of the field, to show what a number format does to it.
    let sample = pivot
        .location
        .map(|body| {
            let (top, left) = (pivot.report.first_data_row(), pivot.report.first_data_col());
            app.wb.value_at(sheet, body.r0 + top, body.c0 + left)
        })
        .filter(Value::is_number)
        .unwrap_or(Value::Number(1234.5));
    Some(ValueOpening {
        pivot: id,
        data,
        locale,
        source: pivot.cache.fields[d.field].name.clone(),
        caption,
        name: d.name.clone(),
        function: d.function,
        tab: 0,
        kind,
        base: base.and_then(|field| bases.iter().position(|b| b.field == field)),
        item,
        bases,
        num_fmt: d.num_fmt.clone(),
        sample,
    })
}

/// A field a calculation can run along, with its items.
struct Base {
    field: usize,
    caption: SharedString,
    items: Vec<(u32, SharedString)>,
}

struct ValueOpening {
    pivot: u32,
    data: usize,
    locale: &'static Locale,
    source: String,
    caption: String,
    name: String,
    function: Aggregation,
    tab: usize,
    kind: ShowKind,
    base: Option<usize>,
    item: Option<BaseItem>,
    bases: Vec<Base>,
    num_fmt: Option<Arc<str>>,
    sample: Value,
}

impl ValueOpening {
    /// Choose a calculation. One that needs a base starts on the first
    /// field and on "(previous)", as in Excel.
    fn choose(&mut self, kind: ShowKind) {
        self.kind = kind;
        if kind.needs_field() && self.base.is_none() && !self.bases.is_empty() {
            self.base = Some(0);
        }
        if kind.needs_item() && self.item.is_none() {
            self.item = Some(BaseItem::Previous);
        }
    }
}

struct ValueSettings {
    modal: Modal,
    open: ValueOpening,
    name: Entity<TextInput>,
    /// The name box still holds a name Excel made up ("Sum of Sales"): it
    /// follows the function.
    generated: bool,
}

impl ValueSettings {
    fn new(modal: Modal, open: ValueOpening, cx: &mut Context<Self>) -> Self {
        let generated = open.name.to_lowercase() == data_field_name(open.function, &open.caption, open.locale).to_lowercase();
        let name = widgets::input(cx, open.name.clone());
        cx.subscribe(&name, |this, _, ev: &InputEvent, _| {
            if *ev == InputEvent::Changed {
                this.generated = false;
            }
        })
        .detach();
        ValueSettings {
            modal,
            open,
            name,
            generated,
        }
    }

    fn pick_function(&mut self, function: Aggregation, cx: &mut Context<Self>) {
        self.open.function = function;
        if self.generated {
            let name = data_field_name(function, &self.open.caption, self.open.locale);
            self.name.update(cx, |input, cx| input.set_text(name, cx));
        }
        cx.notify();
    }

    fn pick_kind(&mut self, kind: ShowKind, cx: &mut Context<Self>) {
        self.open.choose(kind);
        cx.notify();
    }

    fn number_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let code = self.open.num_fmt.clone().unwrap_or_else(|| Arc::from("General"));
        let sample = self.open.sample.clone();
        let this = cx.weak_entity();
        self.modal.update_app(cx, |app, cx| {
            format_cells::open_number_format(app, &code, sample, window, cx, move |code, _, cx| {
                this.update(cx, |this, cx| {
                    this.open.num_fmt = Some(Arc::from(code));
                    cx.notify();
                })
                .ok();
            })
        });
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let open = &self.open;
        let (id, data, function, locale) = (open.pivot, open.data, open.function, open.locale);
        let base = open.base.and_then(|b| open.bases.get(b)).map(|b| b.field);
        let Some(show) = open.kind.build(base, open.item) else {
            // The calculation needs a field the report does not have.
            return;
        };
        let num_fmt = open.num_fmt.clone();
        let custom = (!self.generated).then(|| self.name.read(cx).text().trim().to_string());
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            if let (Some(name), Some((_, pivot))) = (&custom, app.wb.pivot(id)) {
                let mut probe = pivot.clone();
                probe.set_function(data, function, locale);
                if !probe.set_data_name(data, name) {
                    actions::warn(app, logic::name_exists(), cx);
                    return;
                }
            }
            let label = t("Value Field Settings", "Paramètres des champs de valeurs");
            update_in_dialog(app, &modal, id, label, window, cx, move |p| {
                p.set_function(data, function, locale);
                if let Some(name) = &custom {
                    p.set_data_name(data, name);
                }
                if let Some(d) = p.data_fields.get_mut(data) {
                    d.show_as = show;
                    d.num_fmt = num_fmt.clone();
                }
            });
        });
    }

    fn summarize_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.open.locale;
        let functions: Vec<SharedString> = Aggregation::ALL.iter().map(|f| f.label(locale).into()).collect();
        let selected = Aggregation::ALL.iter().position(|f| *f == self.open.function);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(t("Summarize value field by", "Résumer le champ de valeurs par")).font_weight(gpui::FontWeight::SEMIBOLD))
            .child(div().line_height(px(17.)).child(t(
                "Choose the type of calculation that you want to use to summarize data from the selected field",
                "Choisissez le type de calcul à utiliser pour synthétiser les données du champ sélectionné.",
            )))
            .child(
                div().flex().pl(px(12.)).child(
                    ListBox::new("functions", functions, selected)
                        .width(220.0)
                        .height(widgets::ROW_H * 6.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| this.pick_function(Aggregation::ALL[*ix], cx))),
                ),
            )
            .into_any_element()
    }

    fn show_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = &self.open;
        let kinds: Vec<SharedString> = ShowKind::ALL.iter().map(|kind| kind.label().into()).collect();
        let selected = ShowKind::ALL.iter().position(|kind| *kind == open.kind);
        // The two lists are empty while the calculation has no use for them.
        let fields: Vec<SharedString> = if open.kind.needs_field() {
            open.bases.iter().map(|b| b.caption.clone()).collect()
        } else {
            Vec::new()
        };
        let base = open.base.and_then(|b| open.bases.get(b));
        let mut items: Vec<(BaseItem, SharedString)> = Vec::new();
        if let (true, Some(base)) = (open.kind.needs_item(), base) {
            items.push((BaseItem::Previous, t("(previous)", "(précédent)").into()));
            items.push((BaseItem::Next, t("(next)", "(suivant)").into()));
            items.extend(base.items.iter().map(|(x, text)| (BaseItem::Item(*x), text.clone())));
        }
        let item_labels: Vec<SharedString> = items.iter().map(|(_, text)| text.clone()).collect();
        let item_selected = open.item.and_then(|item| items.iter().position(|(i, _)| *i == item));
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(label(t("Show values as", "Afficher les valeurs")).font_weight(gpui::FontWeight::SEMIBOLD))
            .child(
                Select::new("kind", kinds, selected)
                    .width(396.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| this.pick_kind(ShowKind::ALL[*ix], cx))),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Base field:", "Champ de base :")))
                            .child(
                                div().flex().child(
                                    ListBox::new("base-field", fields, open.base.filter(|_| open.kind.needs_field()))
                                        .height(widgets::ROW_H * 5.0)
                                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                            this.open.base = Some(*ix);
                                            if this.open.kind.needs_item() {
                                                this.open.item = Some(BaseItem::Previous);
                                            }
                                            cx.notify();
                                        })),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(label(t("Base item:", "Élément de base :")))
                            .child(
                                div().flex().child(
                                    ListBox::new("base-item", item_labels, item_selected)
                                        .height(widgets::ROW_H * 5.0)
                                        .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                                            this.open.item = items.get(*ix).map(|(item, _)| *item);
                                            cx.notify();
                                        })),
                                ),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ValueSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = vec![
            t("Summarize Values By", "Synthèse des valeurs par").into(),
            t("Show Values As", "Afficher les valeurs").into(),
        ];
        let body = if self.open.tab == 1 { self.show_tab(cx) } else { self.summarize_tab(cx) };
        dialog(&self.modal, t("Value Field Settings", "Paramètres des champs de valeurs"))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(names(&self.open.source, &self.name))
            .child(TabStrip::new("tabs", tabs, self.open.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.open.tab = *ix;
                cx.notify();
            })))
            .child(div().h(px(196.)).child(body))
            .footer(footer(
                &self.modal,
                Button::new("number-format", t("Number Format", "Format de nombre"))
                    .on_click(cx.listener(|this, _, window, cx| this.number_format(window, cx))),
                cx.listener(Self::ok),
            ))
    }
}

// ------------------------------------------------------------ field settings

pub(super) fn open_field_settings(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let Some(f) = pivot.fields.get(field) else {
        return;
    };
    let source = pivot.cache.fields[field].name.clone();
    let caption = pivot.field_caption(field).to_string();
    let (subtotals, functions) = match &f.subtotals {
        Subtotals::Automatic => (0, Vec::new()),
        Subtotals::None => (1, Vec::new()),
        Subtotals::Custom(list) => (
            2,
            list.iter()
                .filter_map(|function| Aggregation::ALL.iter().position(|a| a == function))
                .collect(),
        ),
    };
    let open = FieldOpening {
        pivot: id,
        field,
        locale,
        source,
        caption,
        subtotals,
        functions,
        tabular: f.layout == FieldLayout::Tabular,
        compact: f.layout == FieldLayout::Compact,
        subtotal_top: f.subtotal_top,
        repeat: f.repeat_labels,
        blank_line: f.blank_line,
        show_all: f.show_all,
    };
    app.open_modal(window, cx, move |modal, _, cx| FieldSettings {
        modal,
        name: widgets::input(cx, open.caption.clone()),
        tab: 0,
        open,
    });
}

/// What the Field Settings dialog shows of a field.
struct FieldOpening {
    pivot: u32,
    field: usize,
    locale: &'static Locale,
    source: String,
    caption: String,
    /// 0 automatic, 1 none, 2 custom.
    subtotals: usize,
    /// The functions ticked for custom subtotals (indices of
    /// `Aggregation::ALL`).
    functions: Vec<usize>,
    tabular: bool,
    /// In outline form: labels of the next field in the same column.
    compact: bool,
    subtotal_top: bool,
    repeat: bool,
    blank_line: bool,
    show_all: bool,
}

struct FieldSettings {
    modal: Modal,
    name: Entity<TextInput>,
    tab: usize,
    open: FieldOpening,
}

impl FieldSettings {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let open = &self.open;
        let (id, field) = (open.pivot, open.field);
        let typed = self.name.read(cx).text().trim().to_string();
        // An empty name gives the field its own name back.
        let renamed = (typed != open.caption).then_some(typed);
        let subtotals = match open.subtotals {
            0 => Subtotals::Automatic,
            1 => Subtotals::None,
            _ => Subtotals::Custom(open.functions.iter().map(|ix| Aggregation::ALL[*ix]).collect()),
        };
        let layout = match (open.tabular, open.compact) {
            (true, _) => FieldLayout::Tabular,
            (false, true) => FieldLayout::Compact,
            (false, false) => FieldLayout::Outline,
        };
        let (subtotal_top, repeat, blank_line, show_all) = (open.subtotal_top, open.repeat, open.blank_line, open.show_all);
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            if let (Some(name), Some((_, pivot))) = (&renamed, app.wb.pivot(id)) {
                if !pivot.clone().set_field_caption(field, name) {
                    actions::warn(app, logic::name_exists(), cx);
                    return;
                }
            }
            update_in_dialog(app, &modal, id, t("Field Settings", "Paramètres de champ"), window, cx, move |p| {
                if let Some(name) = &renamed {
                    p.set_field_caption(field, name);
                }
                if let Some(f) = p.fields.get_mut(field) {
                    f.subtotals = subtotals.clone();
                    f.layout = layout;
                    f.subtotal_top = subtotal_top;
                    f.repeat_labels = repeat;
                    f.blank_line = blank_line;
                    f.show_all = show_all;
                }
            });
        });
    }

    fn subtotals_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.open.locale;
        let choices: Vec<SharedString> = vec![
            t("Automatic", "Automatique").into(),
            t("None", "Aucun").into(),
            t_ctx("subtotals", "Custom", "Personnalisé").into(),
        ];
        // The list of functions only answers while "Custom" is chosen.
        let functions: Vec<SharedString> = Aggregation::ALL.iter().map(|f| f.label(locale).into()).collect();
        let custom = self.open.subtotals == 2;
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                GroupBox::new(t("Subtotals", "Sous-totaux"))
                    .child(RadioGroup::new("subtotals", choices, Some(self.open.subtotals)).on_change(cx.listener(
                        |this, ix: &usize, _, cx| {
                            this.open.subtotals = *ix;
                            cx.notify();
                        },
                    )))
                    .child(
                        label(t("Select one or more functions:", "Sélectionnez une ou plusieurs fonctions :"))
                            .when(!custom, |d| d.text_color(c(theme::TEXT_DISABLED))),
                    )
                    .child(
                        div().flex().child(
                            ListBox::multi("functions", functions, if custom { self.open.functions.clone() } else { Vec::new() })
                                .width(220.0)
                                .height(widgets::ROW_H * 5.0)
                                .on_change_multi(cx.listener(|this, rows: &Vec<usize>, _, cx| {
                                    // Picking a function is choosing custom subtotals.
                                    this.open.subtotals = 2;
                                    this.open.functions = rows.clone();
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(GroupBox::new(t_ctx("pivot-field", "Filter", "Filtre")).child(
                Checkbox::new(
                    "new-items",
                    t("Include new items in manual filter", "Inclure les nouveaux éléments dans le filtre manuel"),
                    false,
                )
                .disabled(true),
            ))
            .into_any_element()
    }

    fn layout_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let check = |id: &'static str, text: &'static str, on: bool, enabled: bool, set: fn(&mut Self, bool), cx: &mut Context<Self>| {
            Checkbox::new(id, text, on).disabled(!enabled).on_change(cx.listener(move |this, on: &bool, _, cx| {
                set(this, *on);
                cx.notify();
            }))
        };
        let form = |id: &'static str, text: &'static str, tabular: bool, cx: &mut Context<Self>| {
            RadioGroup::new(id, vec![text.into()], (self.open.tabular == tabular).then_some(0)).on_change(cx.listener(
                move |this, _: &usize, _, cx| {
                    this.open.tabular = tabular;
                    cx.notify();
                },
            ))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                GroupBox::new(t("Layout", "Disposition"))
                    .child(form(
                        "outline",
                        t(
                            "Show item labels in outline form",
                            "Afficher les étiquettes d’éléments sous forme hiérarchique",
                        ),
                        false,
                        cx,
                    ))
                    .child(
                        div()
                            .pl(px(22.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(check(
                                "compact",
                                t(
                                    "Display labels from the next field in the same column (compact form)",
                                    "Afficher les étiquettes du champ suivant dans la même colonne (forme compactée)",
                                ),
                                self.open.compact,
                                !self.open.tabular,
                                |this, on| this.open.compact = on,
                                cx,
                            ))
                            .child(check(
                                "top",
                                t(
                                    "Display subtotals at the top of each group",
                                    "Afficher les sous-totaux au-dessus de chaque groupe",
                                ),
                                self.open.subtotal_top,
                                !self.open.tabular,
                                |this, on| this.open.subtotal_top = on,
                                cx,
                            )),
                    )
                    .child(form(
                        "tabular",
                        t(
                            "Show item labels in tabular form",
                            "Afficher les étiquettes d’éléments sous forme de tableau",
                        ),
                        true,
                        cx,
                    ))
                    .child(check(
                        "repeat",
                        t("Repeat item labels", "Répéter les étiquettes d’éléments"),
                        self.open.repeat,
                        true,
                        |this, on| this.open.repeat = on,
                        cx,
                    ))
                    .child(check(
                        "blank",
                        t(
                            "Insert blank line after each item label",
                            "Insérer un saut de ligne après chaque étiquette d’élément",
                        ),
                        self.open.blank_line,
                        true,
                        |this, on| this.open.blank_line = on,
                        cx,
                    ))
                    .child(check(
                        "show-all",
                        t("Show items with no data", "Afficher les éléments sans données"),
                        self.open.show_all,
                        true,
                        |this, on| this.open.show_all = on,
                        cx,
                    )),
            )
            .child(GroupBox::new(t("Print", "Imprimer")).child(
                Checkbox::new(
                    "page-break",
                    t("Insert page break after each item", "Insérer un saut de page après chaque élément"),
                    false,
                )
                .disabled(true),
            ))
            .into_any_element()
    }
}

impl Render for FieldSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = vec![
            t("Subtotals & Filters", "Sous-totaux et filtres").into(),
            t("Layout & Print", "Disposition et impression").into(),
        ];
        let body = if self.tab == 1 { self.layout_tab(cx) } else { self.subtotals_tab(cx) };
        dialog(&self.modal, t("Field Settings", "Paramètres de champ"))
            .width(610.0)
            .on_ok(cx.listener(Self::ok))
            .child(names(&self.open.source, &self.name))
            .child(TabStrip::new("tabs", tabs, self.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = *ix;
                cx.notify();
            })))
            .child(div().h(px(262.)).child(body))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------- PivotTable options

pub(super) fn open_options(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app) else {
        return;
    };
    let (id, name, options) = (pivot.id, pivot.name.clone(), pivot.options.clone());
    app.open_modal(window, cx, move |modal, _, cx| OptionsDialog {
        modal,
        pivot: id,
        tab: 0,
        original: name.clone(),
        name: widgets::input(cx, name),
        over_then_down: options.page_over_then_down,
        wrap: widgets::number_input(cx, Some(options.page_wrap as f64), 0.0, 255.0, 1.0),
        show_error: options.show_error,
        error: widgets::input(cx, options.error_caption),
        show_missing: options.show_missing,
        missing: widgets::input(cx, options.missing_caption),
        row_totals: options.row_grand_totals,
        col_totals: options.col_grand_totals,
        drill: options.show_drill,
        headers: options.show_headers,
    });
}

struct OptionsDialog {
    modal: Modal,
    pivot: u32,
    tab: usize,
    original: String,
    name: Entity<TextInput>,
    over_then_down: bool,
    wrap: Entity<TextInput>,
    show_error: bool,
    error: Entity<TextInput>,
    show_missing: bool,
    missing: Entity<TextInput>,
    row_totals: bool,
    col_totals: bool,
    drill: bool,
    headers: bool,
}

impl OptionsDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let id = self.pivot;
        let name = self.name.read(cx).text().trim().to_string();
        let renamed = (name != self.original).then_some(name);
        let over_then_down = self.over_then_down;
        let wrap = self.wrap.read(cx).number().unwrap_or(0.0).clamp(0.0, 255.0) as u32;
        let (show_error, error) = (self.show_error, self.error.read(cx).text().to_string());
        let (show_missing, missing) = (self.show_missing, self.missing.read(cx).text().to_string());
        let (row_totals, col_totals, drill, headers) = (self.row_totals, self.col_totals, self.drill, self.headers);
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            let label = t("PivotTable Options", "Options du tableau croisé dynamique");
            perform_in_dialog(app, &modal, label, window, cx, move |app, overwrite| {
                // The name is checked first: a refused name changes nothing.
                if let Some(name) = &renamed {
                    let taken = app.wb.pivot(id).is_some_and(|(sheet, _)| {
                        app.wb.sheets[sheet]
                            .pivots
                            .iter()
                            .any(|p| p.id != id && p.name.to_lowercase() == name.to_lowercase())
                    });
                    if name.is_empty() || taken {
                        return Err(PivotError::InvalidName);
                    }
                }
                app.wb.pivot_update(id, overwrite, |p| {
                    let o = &mut p.options;
                    o.page_over_then_down = over_then_down;
                    o.page_wrap = wrap;
                    o.show_error = show_error;
                    o.error_caption = error.clone();
                    o.show_missing = show_missing;
                    o.missing_caption = missing.clone();
                    o.row_grand_totals = row_totals;
                    o.col_grand_totals = col_totals;
                    o.show_drill = drill;
                    o.show_headers = headers;
                })?;
                match &renamed {
                    Some(name) => app.wb.pivot_rename(id, name),
                    None => Ok(()),
                }
            });
        });
    }

    fn check(&self, id: &'static str, text: &'static str, on: bool, set: fn(&mut Self, bool), cx: &mut Context<Self>) -> Checkbox {
        Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
            set(this, *on);
            cx.notify();
        }))
    }

    fn layout_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let orders: Vec<SharedString> = vec![
            t("Down, Then Over", "Vers le bas, puis à droite").into(),
            t("Over, Then Down", "À droite, puis vers le bas").into(),
        ];
        let per = if self.over_then_down {
            t("Report filter fields per row:", "Champs de filtre de rapport par ligne :")
        } else {
            t("Report filter fields per column:", "Champs de filtre de rapport par colonne :")
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                GroupBox::new(t("Layout", "Disposition"))
                    .child(labeled(
                        t(
                            "Display fields in report filter area:",
                            "Afficher les champs dans la zone Filtre du rapport :",
                        ),
                        Select::new("order", orders, Some(self.over_then_down as usize))
                            .width(190.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.over_then_down = *ix == 1;
                                cx.notify();
                            })),
                    ))
                    .child(labeled(per, NumberField::new("wrap", &self.wrap).width(60.0))),
            )
            .child(
                GroupBox::new(t("Format", "Format"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(self.check(
                                "show-error",
                                t("For error values show:", "Pour les valeurs d'erreur, afficher :"),
                                self.show_error,
                                |this, on| this.show_error = on,
                                cx,
                            ))
                            .child(TextField::new(&self.error).width(150.0)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(self.check(
                                "show-missing",
                                t("For empty cells show:", "Pour les cellules vides, afficher :"),
                                self.show_missing,
                                |this, on| this.show_missing = on,
                                cx,
                            ))
                            .child(TextField::new(&self.missing).width(150.0)),
                    )
                    // The columns are always fitted and the formats kept:
                    // the engine has no switch for either.
                    .child(
                        Checkbox::new(
                            "autofit",
                            t(
                                "Autofit column widths on update",
                                "Ajuster automatiquement la largeur des colonnes lors de la mise à jour",
                            ),
                            true,
                        )
                        .disabled(true),
                    )
                    .child(
                        Checkbox::new(
                            "preserve",
                            t(
                                "Preserve cell formatting on update",
                                "Conserver la mise en forme des cellules lors de la mise à jour",
                            ),
                            true,
                        )
                        .disabled(true),
                    ),
            )
            .into_any_element()
    }

    fn totals_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        GroupBox::new(t("Grand Totals", "Totaux généraux"))
            .child(self.check(
                "row-totals",
                t("Show grand totals for rows", "Afficher les totaux des lignes"),
                self.row_totals,
                |this, on| this.row_totals = on,
                cx,
            ))
            .child(self.check(
                "col-totals",
                t("Show grand totals for columns", "Afficher les totaux des colonnes"),
                self.col_totals,
                |this, on| this.col_totals = on,
                cx,
            ))
            .into_any_element()
    }

    fn display_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        GroupBox::new(t("Display", "Affichage"))
            .child(self.check(
                "drill",
                t("Show expand/collapse buttons", "Afficher les boutons Développer/Réduire"),
                self.drill,
                |this, on| this.drill = on,
                cx,
            ))
            .child(self.check(
                "headers",
                t(
                    "Display field captions and filter drop downs",
                    "Afficher les légendes des champs et les listes déroulantes de filtrage",
                ),
                self.headers,
                |this, on| this.headers = on,
                cx,
            ))
            .into_any_element()
    }
}

impl Render for OptionsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = vec![
            t("Layout & Format", "Disposition et mise en forme").into(),
            t("Totals & Filters", "Totaux et filtres").into(),
            t("Display", "Affichage").into(),
        ];
        let body = match self.tab {
            1 => self.totals_tab(cx),
            2 => self.display_tab(cx),
            _ => self.layout_tab(cx),
        };
        dialog(&self.modal, t("PivotTable Options", "Options du tableau croisé dynamique"))
            .width(590.0)
            .on_ok(cx.listener(Self::ok))
            .child(labeled(
                t("PivotTable Name:", "Nom du tableau croisé dynamique :"),
                TextField::new(&self.name).width(220.0),
            ))
            .child(TabStrip::new("tabs", tabs, self.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = *ix;
                cx.notify();
            })))
            .child(div().h(px(232.)).child(body))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------------ grouping

/// The periods of the Grouping dialog, in the order of its list.
const PERIODS: [GroupBy; 7] = [
    GroupBy::Seconds,
    GroupBy::Minutes,
    GroupBy::Hours,
    GroupBy::Days,
    GroupBy::Months,
    GroupBy::Quarters,
    GroupBy::Years,
];

/// What the Grouping dialog opens on for a field: whether it groups dates,
/// the bounds, the periods (or the interval) and the number of days.
struct GroupingOpening {
    dates: bool,
    start: f64,
    end: f64,
    auto_start: bool,
    auto_end: bool,
    periods: Vec<GroupBy>,
    interval: f64,
    days: u32,
}

fn grouping_opening(pivot: &PivotTable, field: usize) -> Option<GroupingOpening> {
    let cache = &pivot.cache;
    let source = cache.database_field(field);
    let numbers: Vec<f64> = cache.fields.get(source)?.shared.iter().filter_map(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).collect();
    let min = numbers.iter().copied().fold(f64::INFINITY, f64::min);
    let max = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() {
        return None;
    }
    let dates = cache.is_date(source);
    let mut open = GroupingOpening {
        dates,
        start: min,
        end: max,
        auto_start: true,
        auto_end: true,
        periods: if dates { vec![GroupBy::Months] } else { Vec::new() },
        // A round step that cuts the range in about ten groups.
        interval: 10f64.powf(((max - min).max(1.0)).log10().floor()).max(1.0),
        days: 1,
    };
    // The groups the field already has.
    let mut groups = cache.group_children(source);
    groups.push(source);
    let mut periods = Vec::new();
    for group in groups.into_iter().filter_map(|f| cache.fields[f].group.as_ref()) {
        if let GroupKind::Range {
            by,
            start,
            end,
            interval,
            auto_start,
            auto_end,
        } = group.kind
        {
            (open.auto_start, open.auto_end) = (auto_start, auto_end);
            if !auto_start {
                open.start = start;
            }
            if !auto_end {
                open.end = end;
            }
            match by {
                GroupBy::Range => open.interval = interval,
                GroupBy::Days => {
                    open.days = interval.max(1.0) as u32;
                    periods.push(by);
                }
                _ => periods.push(by),
            }
        }
    }
    if !periods.is_empty() {
        open.periods = periods;
    }
    Some(open)
}

/// The Grouping dialog of a date or number field.
pub(super) fn open_grouping(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    let Some(open) = app.wb.pivot(id).and_then(|(_, pivot)| grouping_opening(pivot, field)) else {
        actions::warn(app, t("Cannot group that selection.", "Impossible de grouper cette sélection."), cx);
        return;
    };
    let bound = move |n: f64| {
        if open.dates {
            let code = if n.fract() == 0.0 {
                locale.short_date_format.to_string()
            } else {
                format!("{} hh:mm", locale.short_date_format)
            };
            opensheet_engine::format::format_number(n, &code, locale).text
        } else {
            logic::number_text(n, locale)
        }
    };
    app.open_modal(window, cx, move |modal, _, cx| GroupingDialog {
        modal,
        pivot: id,
        field,
        locale,
        dates: open.dates,
        auto_start: open.auto_start,
        auto_end: open.auto_end,
        start: widgets::input(cx, bound(open.start)),
        end: widgets::input(cx, bound(open.end)),
        periods: PERIODS
            .iter()
            .enumerate()
            .filter(|(_, by)| open.periods.contains(by))
            .map(|(i, _)| i)
            .collect(),
        interval: widgets::input(cx, logic::number_text(open.interval, locale)),
        days: widgets::number_input(cx, Some(open.days as f64), 1.0, 32767.0, 1.0),
    });
}

struct GroupingDialog {
    modal: Modal,
    pivot: u32,
    field: usize,
    locale: &'static Locale,
    dates: bool,
    auto_start: bool,
    auto_end: bool,
    start: Entity<TextInput>,
    end: Entity<TextInput>,
    /// The periods selected (indices of `PERIODS`).
    periods: Vec<usize>,
    interval: Entity<TextInput>,
    days: Entity<TextInput>,
}

impl GroupingDialog {
    /// "Number of days" only counts when days are the only period.
    fn days_only(&self) -> bool {
        self.periods.len() == 1 && PERIODS[self.periods[0]] == GroupBy::Days
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (id, field, locale, dates) = (self.pivot, self.field, self.locale, self.dates);
        let read = |input: &Entity<TextInput>| {
            let text = input.read(cx).text().to_string();
            if dates { logic::parse_date(&text, locale) } else { logic::parse_number(&text, locale) }
        };
        // A bound left on "auto" follows the data.
        let start = if self.auto_start { Ok(None) } else { read(&self.start).map(Some).ok_or(()) };
        let end = if self.auto_end { Ok(None) } else { read(&self.end).map(Some).ok_or(()) };
        let interval = logic::parse_number(self.interval.read(cx).text(), locale).filter(|n| *n > 0.0);
        let periods: Vec<GroupBy> = self.periods.iter().map(|ix| PERIODS[*ix]).collect();
        let days = if self.days_only() { self.days.read(cx).number().unwrap_or(1.0).max(1.0) as u32 } else { 1 };
        let (Ok(start), Ok(end), Some(interval)) = (start, end, interval.or(Some(1.0).filter(|_| dates))) else {
            self.modal.update_app(cx, |app, cx| {
                actions::warn(app, t("Value must be a number.", "La valeur doit être un nombre."), cx)
            });
            return;
        };
        if dates && periods.is_empty() {
            return;
        }
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            perform_in_dialog(app, &modal, t("Group", "Grouper"), window, cx, move |app, overwrite| {
                if dates {
                    app.wb.pivot_group_dates(id, field, &periods, start, end, days, overwrite)
                } else {
                    app.wb.pivot_group_numbers(id, field, start, end, interval, overwrite)
                }
            });
        });
    }

    fn bound(&self, id: &'static str, text: &'static str, auto: bool, input: &Entity<TextInput>, start: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().w(px(110.)).child(Checkbox::new(id, text, auto).on_change(cx.listener(move |this, on: &bool, _, cx| {
                if start {
                    this.auto_start = *on;
                } else {
                    this.auto_end = *on;
                }
                cx.notify();
            }))))
            .child(TextField::new(input).width(130.0))
    }
}

impl Render for GroupingDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = self.locale;
        let auto = GroupBox::new(t_ctx("grouping", "Auto", "Automatique"))
            .child(self.bound("auto-start", t("Starting at:", "Début :"), self.auto_start, &self.start, true, cx))
            .child(self.bound("auto-end", t("Ending at:", "Fin :"), self.auto_end, &self.end, false, cx));
        let by: AnyElement = if self.dates {
            let names: Vec<SharedString> = PERIODS.iter().map(|by| by.label(locale).into()).collect();
            let days_only = self.days_only();
            GroupBox::new(t("By", "Par"))
                .child(
                    div().flex().child(
                        ListBox::multi("periods", names, self.periods.clone())
                            .width(250.0)
                            .height(widgets::ROW_H * 7.0)
                            .on_change_multi(cx.listener(|this, rows: &Vec<usize>, _, cx| {
                                this.periods = rows.clone();
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    labeled(t("Number of days:", "Nombre de jours :"), NumberField::new("days", &self.days).width(70.0))
                        .when(!days_only, |d| d.opacity(0.45)),
                )
                .into_any_element()
        } else {
            div()
                .pl(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(110.)).pl(px(20.)).child(label(t("By:", "Par :"))))
                .child(TextField::new(&self.interval).width(130.0))
                .into_any_element()
        };
        dialog(&self.modal, t("Grouping", "Grouper"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(auto)
            .child(by)
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------------- report filter pages

/// "Show Report Filter Pages": pick the filter field whose items each get a
/// sheet.
pub(super) fn open_filter_pages(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app) else {
        return;
    };
    let id = pivot.id;
    let fields: Vec<(usize, SharedString)> = pivot
        .page_fields
        .iter()
        .map(|pf| (pf.field, pivot.field_caption(pf.field).to_string().into()))
        .collect();
    if fields.is_empty() {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| FilterPages {
        modal,
        pivot: id,
        fields,
        selected: 0,
    });
}

struct FilterPages {
    modal: Modal,
    pivot: u32,
    fields: Vec<(usize, SharedString)>,
    selected: usize,
}

impl FilterPages {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some((field, _)) = self.fields.get(self.selected).cloned() else {
            return;
        };
        let id = self.pivot;
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| actions::filter_pages(app, id, field, window, cx));
    }
}

impl Render for FilterPages {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.fields.iter().map(|(_, name)| name.clone()).collect();
        dialog(
            &self.modal,
            t("Show Report Filter Pages", "Afficher les pages de filtre du rapport"),
        )
        .width(380.0)
        .on_ok(cx.listener(Self::ok))
        .child(label(t(
            "Show all report filter pages of:",
            "Afficher toutes les pages de filtre du rapport de :",
        )))
        .child(
            div().flex().child(
                ListBox::new("fields", names, Some(self.selected))
                    .height(widgets::ROW_H * 6.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.selected = *ix;
                        cx.notify();
                    }))
                    .on_activate(cx.listener(|this, ix: &usize, window, cx| {
                        this.selected = *ix;
                        this.ok(&(), window, cx);
                    })),
            ),
        )
        .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------------- show detail

/// "Show Detail": a label with no field below it was double-clicked; pick
/// the field whose items go under it.
pub(super) fn open_show_detail(app: &mut ExcelApp, id: u32, field: usize, item: u32, window: &mut Window, cx: &mut Cx) {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let fields: Vec<(usize, SharedString)> = (0..pivot.fields.len())
        .filter(|f| pivot.axis_of(*f).is_none())
        .map(|f| (f, pivot.field_caption(f).to_string().into()))
        .collect();
    if fields.is_empty() {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| ShowDetail {
        modal,
        pivot: id,
        field,
        item,
        fields,
        selected: 0,
    });
}

struct ShowDetail {
    modal: Modal,
    pivot: u32,
    /// The label that was double-clicked.
    field: usize,
    item: u32,
    fields: Vec<(usize, SharedString)>,
    selected: usize,
}

impl ShowDetail {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some((detail, _)) = self.fields.get(self.selected).cloned() else {
            return;
        };
        let (id, field, item) = (self.pivot, self.field, self.item);
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            let locale = app.wb.locale;
            update_in_dialog(app, &modal, id, t("Show Detail", "Afficher le détail"), window, cx, move |p| {
                logic::show_detail(p, field, item, detail, locale)
            });
        });
    }
}

impl Render for ShowDetail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.fields.iter().map(|(_, name)| name.clone()).collect();
        dialog(&self.modal, t("Show Detail", "Afficher le détail"))
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(div().line_height(px(17.)).child(t(
                "Choose the field containing the detail you want to show:",
                "Sélectionnez le champ contenant le détail que vous voulez afficher :",
            )))
            .child(
                div().flex().child(
                    ListBox::new("fields", names, Some(self.selected))
                        .height(widgets::ROW_H * 7.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.selected = *ix;
                            cx.notify();
                        }))
                        .on_activate(cx.listener(|this, ix: &usize, window, cx| {
                            this.selected = *ix;
                            this.ok(&(), window, cx);
                        })),
                ),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};
    use opensheet_engine::{Rect, Workbook};

    fn workbook() -> (Workbook, u32) {
        let mut wb = Workbook::new(Locale::en());
        let rows = [
            ["Date", "Amount"],
            ["2023-01-05", "120"],
            ["2023-06-12", "45"],
            ["2024-02-02", "980"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("data");
        let source = PivotSource::Range {
            sheet: wb.sheets[0].name.clone(),
            range: Rect::new(0, 0, 3, 1),
        };
        let id = wb.pivot_create(source, PivotDestination::NewSheet { index: 1 }, false).unwrap();
        wb.commit("pivot");
        (wb, id)
    }

    #[test]
    fn grouping_opens_on_the_data_or_on_the_groups() {
        let (mut wb, id) = workbook();
        // A number field: its range, cut in round steps.
        let open = grouping_opening(wb.pivot(id).unwrap().1, 1).unwrap();
        assert!(!open.dates && open.auto_start && open.auto_end);
        assert_eq!((open.start, open.end, open.interval), (45.0, 980.0, 100.0));
        // A date field: by months until it is grouped.
        let open = grouping_opening(wb.pivot(id).unwrap().1, 0).unwrap();
        assert!(open.dates);
        assert_eq!(open.periods, [GroupBy::Months]);

        // Once grouped, the dialog shows the groups, from the date field
        // and from any of its periods.
        wb.pivot_add_field(id, 0, Area::Rows, None, false).unwrap();
        wb.pivot_group_dates(id, 0, &[GroupBy::Years, GroupBy::Days], Some(44927.0), None, 7, false)
            .unwrap();
        let (_, pivot) = wb.pivot(id).unwrap();
        let years = pivot.field_index("Years (Date)").unwrap();
        for field in [0, years] {
            let open = grouping_opening(pivot, field).unwrap();
            assert!(open.dates && !open.auto_start && open.auto_end);
            assert_eq!(open.start, 44927.0);
            assert_eq!(open.days, 7);
            let mut periods = open.periods.clone();
            periods.sort();
            assert_eq!(periods, [GroupBy::Days, GroupBy::Years]);
        }

        wb.pivot_group_numbers(id, 1, Some(0.0), Some(1000.0), 250.0, false).unwrap();
        let open = grouping_opening(wb.pivot(id).unwrap().1, 1).unwrap();
        assert_eq!((open.start, open.end, open.interval), (0.0, 1000.0, 250.0));
        assert!(!open.auto_start && !open.auto_end);
    }
}
