//! What a chart plots and where it lives: the Select Data Source dialog
//! with Edit Series, Axis Labels and Hidden and Empty Cells, and the Move
//! Chart dialog.

use super::Charts;
use super::elements::series_name;
use crate::app::ExcelApp;
use crate::features::{Cx, Intent, object_id};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ListBox, RadioGroup, RefEdit, Select};
use gpui::{Context, Entity, Render, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::chart::{BlanksAs, Chart, ChartKind, Family, Series, SeriesName};
use opensheet_engine::{Locale, parser};
use std::sync::Arc;

// ----------------------------------------------------------------- references

/// A range as typed or picked in a field ("=Sheet1!$A$1:$B$5", "a1:b5"):
/// absolute, and on a named sheet (`own_sheet` when none is given).
pub fn parse_range(text: &str, locale: &Locale, own_sheet: &str) -> Option<RefExpr> {
    let text = text.trim().trim_start_matches('=').trim();
    if text.is_empty() {
        return None;
    }
    let expr = parser::parse_formula(text, locale).or_else(|_| parser::parse_formula(text, Locale::en())).ok()?;
    let mut range = match expr {
        Expr::Ref(range) => range,
        Expr::Paren(inner) => match *inner {
            Expr::Ref(range) => range,
            _ => return None,
        },
        _ => return None,
    };
    if range.sheet.is_none() {
        range.sheet = Some(Arc::from(own_sheet));
    }
    for cell in std::iter::once(&mut range.start).chain(range.end.as_mut()) {
        cell.abs_row = true;
        cell.abs_col = true;
    }
    Some(range)
}

/// A range as a field shows it.
pub fn range_text(range: &RefExpr) -> String {
    format!("={}", range.to_text())
}

/// The name of a series as typed: a cell, a text between quotes or as it
/// is, nothing.
pub fn parse_series_name(text: &str, locale: &Locale, own_sheet: &str) -> SeriesName {
    let text = text.trim();
    if text.is_empty() {
        return SeriesName::None;
    }
    if let Some(formula) = text.strip_prefix('=') {
        if let Some(range) = parse_range(formula, locale, own_sheet) {
            return SeriesName::Cell(range);
        }
        let formula = formula.trim();
        if formula.len() >= 2 && formula.starts_with('"') && formula.ends_with('"') {
            return SeriesName::Text(formula[1..formula.len() - 1].replace("\"\"", "\""));
        }
    }
    SeriesName::Text(text.to_string())
}

pub fn series_name_text(name: &SeriesName) -> String {
    match name {
        SeriesName::None => String::new(),
        SeriesName::Text(text) => format!("=\"{}\"", text.replace('"', "\"\"")),
        SeriesName::Cell(range) => range_text(range),
    }
}

/// The block of cells the series of a chart come from, when they are
/// exactly what Excel would make of one block; otherwise the range is
/// "too complex to be displayed".
pub fn data_range(app: &ExcelApp, own_sheet: usize, kind: ChartKind, series: &[Series]) -> Option<String> {
    let first = series.first()?;
    let sheet_name = first.values.sheet.clone();
    let mut block = first.values.rect();
    for s in series {
        let mut parts = vec![&s.values];
        parts.extend(s.categories.as_ref());
        parts.extend(s.bubble_sizes.as_ref());
        if let SeriesName::Cell(name) = &s.name {
            parts.push(name);
        }
        for part in parts {
            if part.sheet != sheet_name {
                return None;
            }
            block = block.union(&part.rect());
        }
    }
    let sheet = match &sheet_name {
        Some(name) => app.wb.sheet_index(name)?,
        None => own_sheet,
    };
    // Series may run along the rows or the columns of the block, and in
    // any order: moving one up or down does not change the range.
    let same = |rebuilt: &Chart| {
        let alike = |a: &Series, b: &Series| {
            a.values.rect() == b.values.rect()
                && a.categories.as_ref().map(RefExpr::rect) == b.categories.as_ref().map(RefExpr::rect)
                && a.bubble_sizes.as_ref().map(RefExpr::rect) == b.bubble_sizes.as_ref().map(RefExpr::rect)
        };
        rebuilt.series.len() == series.len() && series.iter().all(|ours| rebuilt.series.iter().any(|theirs| alike(ours, theirs)))
    };
    let fits = [Some(false), Some(true)].into_iter().any(|by_rows| same(&app.wb.chart_from_block(sheet, block, kind, by_rows)));
    fits.then(|| RefStyle::formula().format(block, &app.wb.sheets[sheet].name))
}

// --------------------------------------------------------------- Select Data

struct SelectData {
    modal: Modal,
    kind: ChartKind,
    sheet: usize,
    own_sheet: String,
    locale: &'static Locale,
    series: Vec<Series>,
    blanks: BlanksAs,
    selected: Option<usize>,
    range: Entity<TextInput>,
    /// The text of `range` the series were last made from (or shown for).
    applied: String,
}

pub(crate) fn open_select_data(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    open_dialog(app, window, cx);
}

fn open_dialog(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<SelectData>> {
    let chart = app.selected_chart().cloned()?;
    let sheet = app.sheet;
    let own_sheet = app.wb.sheets[sheet].name.to_string();
    let locale = app.wb.locale;
    let range = data_range(app, sheet, chart.kind, &chart.series).unwrap_or_default();
    Some(app.open_modal(window, cx, move |modal, _, cx| SelectData {
        range: modal.ref_input(RefStyle::formula(), range.clone(), cx),
        modal,
        kind: chart.kind,
        sheet,
        own_sheet,
        locale,
        selected: (!chart.series.is_empty()).then_some(0),
        series: chart.series,
        blanks: chart.blanks,
        applied: range,
    }))
}

impl SelectData {
    /// The chart the dialog describes, for reading its data.
    fn chart(&self) -> Chart {
        let mut chart = Chart::new(self.kind);
        chart.series = self.series.clone();
        chart
    }

    /// A range typed or picked in "Chart data range" replaces every series.
    fn sync_range(&mut self, cx: &mut Context<Self>) {
        let text = self.range.read(cx).text().to_string();
        if text == self.applied {
            return;
        }
        let Some(range) = parse_range(&text, self.locale, &self.own_sheet) else {
            return;
        };
        let (kind, own) = (self.kind, self.sheet);
        let rebuilt = self.modal.read_app(cx, |app| {
            let sheet = range.sheet.as_deref().and_then(|name| app.wb.sheet_index(name)).unwrap_or(own);
            app.wb.chart_from_range(sheet, range.rect(), kind).series
        });
        if let Some(series) = rebuilt.filter(|s| !s.is_empty()) {
            // The formatting of the series stays with their position.
            self.series = series
                .into_iter()
                .enumerate()
                .map(|(i, new)| match self.series.get(i) {
                    Some(old) => Series {
                        name: new.name,
                        values: new.values,
                        categories: new.categories,
                        bubble_sizes: new.bubble_sizes,
                        ..old.clone()
                    },
                    None => new,
                })
                .collect();
            self.selected = Some(self.selected.unwrap_or(0).min(self.series.len() - 1));
            self.applied = text;
        }
    }

    /// Show the range the series now come from.
    fn show_range(&mut self, cx: &mut Context<Self>) {
        let (kind, sheet, series) = (self.kind, self.sheet, self.series.clone());
        let text = self
            .modal
            .read_app(cx, |app| data_range(app, sheet, kind, &series))
            .flatten()
            .unwrap_or_default();
        self.applied = text.clone();
        self.range.update(cx, |input, cx| input.set_text(text, cx));
        cx.notify();
    }

    fn switch(&mut self, cx: &mut Context<Self>) {
        let mut chart = self.chart();
        let sheet = self.sheet;
        if self.modal.read_app(cx, |app| chart.switch_row_column(&app.wb, sheet)) == Some(true) {
            self.series = chart.series;
            self.selected = (!self.series.is_empty()).then_some(0);
            self.show_range(cx);
        }
    }

    fn edit_series(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let series = index.and_then(|i| self.series.get(i).cloned());
        let (kind, own_sheet, locale) = (self.kind, self.own_sheet.clone(), self.locale);
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| EditSeries::new(modal, parent, index, series, kind, own_sheet, locale, cx));
        });
    }

    fn edit_labels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let current = self.series.iter().find_map(|s| s.categories.as_ref()).map(range_text).unwrap_or_default();
        let (own_sheet, locale) = (self.own_sheet.clone(), self.locale);
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| AxisLabels {
                range: modal.ref_input(RefStyle::formula(), current, cx),
                modal,
                parent,
                own_sheet,
                locale,
            });
        });
    }

    fn edit_blanks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let blanks = self.blanks;
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, _| EmptyCells { modal, parent, blanks });
        });
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        if let Some(i) = self.selected.filter(|i| *i < self.series.len()) {
            self.series.remove(i);
            self.selected = if self.series.is_empty() { None } else { Some(i.min(self.series.len() - 1)) };
            self.show_range(cx);
        }
    }

    fn shift(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(i) = self.selected else { return };
        let j = if down { i + 1 } else { i.wrapping_sub(1) };
        if j < self.series.len() {
            self.series.swap(i, j);
            self.selected = Some(j);
            self.show_range(cx);
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (series, blanks) = (self.series.clone(), self.blanks);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            app.edit_chart(t("Select Data", "Sélectionner des données"), cx, move |c| {
                c.series = series;
                c.blanks = blanks;
                c.legend_hidden.clear();
            });
        });
    }
}

impl Render for SelectData {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_range(cx);
        let (chart, sheet) = (self.chart(), self.sheet);
        let data = self.modal.read_app(cx, |app| app.wb.chart_data(sheet, &chart)).unwrap_or_default();
        let names: Vec<SharedString> = (0..self.series.len()).map(|i| series_name(&data, i).into()).collect();
        let xy = self.kind.is_xy();
        let labels: Vec<SharedString> = if xy {
            data.x_values.iter().map(|x| x.map(|x| x.to_string()).unwrap_or_default().into()).collect()
        } else {
            // Labels on several levels read outer level first ("2024 Q1").
            (0..data.categories.len()).map(|k| data.category_name(k).into()).collect()
        };
        let complex = self.applied.is_empty() && !self.series.is_empty();
        let has = self.selected.is_some();
        let small = |id: &'static str, label: &'static str| Button::new(id, label).width(74.0);
        widgets::dialog(&self.modal, t("Select Data Source", "Sélectionner la source de données"))
            .width(660.0)
            .on_ok(cx.listener(Self::ok))
            .child(widgets::labeled(
                t("Chart data range:", "Plage de données du graphique :"),
                RefEdit::new(&self.range, &self.modal),
            ))
            .when(complex, |d| {
                d.child(div().text_color(c(theme::TEXT_MUTED)).child(t(
                    "The data range is too complex to be displayed. If a new range is selected, it will replace all of the series in the Series panel.",
                    "La plage de données est trop complexe pour être affichée. Si vous sélectionnez une nouvelle plage de données, elle remplacera toutes les séries du volet Série.",
                )))
            })
            .child(
                div().flex().justify_center().child(
                    Button::new("switch", t("Switch Row/Column", "Changer de ligne ou de colonne"))
                        .on_click(cx.listener(|this, _, _, cx| this.switch(cx))),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(widgets::label(t("Legend Entries (Series)", "Entrées de légende (Série)")))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(4.))
                                    .child(small("add", t("Add", "Ajouter")).on_click(cx.listener(|this, _, window, cx| this.edit_series(None, window, cx))))
                                    .child(small("edit", t("Edit", "Modifier")).disabled(!has).on_click(cx.listener(|this, _, window, cx| {
                                        this.edit_series(this.selected, window, cx)
                                    })))
                                    .child(small("remove", t("Remove", "Supprimer")).disabled(!has).on_click(cx.listener(|this, _, _, cx| this.remove(cx))))
                                    .child(Button::new("up", "▲").width(28.0).disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(false, cx))))
                                    .child(Button::new("down", "▼").width(28.0).disabled(!has).on_click(cx.listener(|this, _, _, cx| this.shift(true, cx)))),
                            )
                            .child(
                                ListBox::new("series", names, self.selected)
                                    .height(150.0)
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        this.selected = Some(*ix);
                                        cx.notify();
                                    }))
                                    .on_activate(cx.listener(|this, ix: &usize, window, cx| this.edit_series(Some(*ix), window, cx))),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(widgets::label(t(
                                "Horizontal (Category) Axis Labels",
                                "Étiquettes de l'axe horizontal (abscisse)",
                            )))
                            .child(
                                div().flex().child(
                                    small("labels", t("Edit", "Modifier"))
                                        .disabled(xy || self.series.is_empty())
                                        .on_click(cx.listener(|this, _, window, cx| this.edit_labels(window, cx))),
                                ),
                            )
                            .child(ListBox::new("categories", labels, None).height(150.0)),
                    ),
            )
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Button::new("blanks", t("Hidden and Empty Cells", "Cellules masquées et cellules vides"))
                            .on_click(cx.listener(|this, _, window, cx| this.edit_blanks(window, cx))),
                    )
                    .child(widgets::ok_cancel(&self.modal, cx.listener(Self::ok))),
            )
    }
}

// --------------------------------------------------------------- Edit Series

struct EditSeries {
    modal: Modal,
    parent: WeakEntity<SelectData>,
    /// The series being edited; `None` adds one.
    index: Option<usize>,
    base: Option<Series>,
    kind: ChartKind,
    own_sheet: String,
    locale: &'static Locale,
    name: Entity<TextInput>,
    x: Entity<TextInput>,
    values: Entity<TextInput>,
    sizes: Entity<TextInput>,
}

impl EditSeries {
    #[allow(clippy::too_many_arguments)]
    fn new(
        modal: Modal,
        parent: WeakEntity<SelectData>,
        index: Option<usize>,
        base: Option<Series>,
        kind: ChartKind,
        own_sheet: String,
        locale: &'static Locale,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = |range: Option<&RefExpr>| range.map(range_text).unwrap_or_default();
        let field = |text: String, cx: &mut Context<Self>| modal.ref_input(RefStyle::formula(), text, cx);
        EditSeries {
            name: field(base.as_ref().map(|s| series_name_text(&s.name)).unwrap_or_default(), cx),
            x: field(text(base.as_ref().and_then(|s| s.categories.as_ref())), cx),
            values: field(text(base.as_ref().map(|s| &s.values)), cx),
            sizes: field(text(base.as_ref().and_then(|s| s.bubble_sizes.as_ref())), cx),
            modal,
            parent,
            index,
            base,
            kind,
            own_sheet,
            locale,
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let read = |field: &Entity<TextInput>, cx: &Context<Self>| field.read(cx).text().to_string();
        let range = |text: &str| parse_range(text, self.locale, &self.own_sheet);
        let Some(values) = range(&read(&self.values, cx)) else {
            self.modal.update_app(cx, |app, cx| {
                app.alert(
                    Alert::new("OpenSheet", t("Reference is not valid.", "La référence n'est pas valide.")).warning(),
                    cx,
                );
            });
            return;
        };
        let xy = self.kind.is_xy();
        let mut series = self.base.clone().unwrap_or_else(|| Series::new(values.clone()));
        series.values = values;
        series.name = parse_series_name(&read(&self.name, cx), self.locale, &self.own_sheet);
        if xy {
            series.categories = range(&read(&self.x, cx));
        }
        if self.kind.family() == Family::Bubble {
            series.bubble_sizes = range(&read(&self.sizes, cx));
        }
        let index = self.index;
        self.modal.close(window, cx);
        self.parent
            .update(cx, |parent, cx| {
                match index {
                    Some(i) if i < parent.series.len() => parent.series[i] = series,
                    _ => {
                        // A new series shares the categories of the others.
                        if !xy && series.categories.is_none() {
                            series.categories = parent.series.iter().find_map(|s| s.categories.clone());
                        }
                        parent.series.push(series);
                        parent.selected = Some(parent.series.len() - 1);
                    }
                }
                parent.show_range(cx);
            })
            .ok();
    }
}

impl Render for EditSeries {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let xy = self.kind.is_xy();
        let field = |label: &'static str, input: &Entity<TextInput>| {
            div().flex().flex_col().gap(px(3.)).child(widgets::label(label)).child(div().flex().child(RefEdit::new(input, &self.modal)))
        };
        widgets::dialog(&self.modal, t("Edit Series", "Modifier la série"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(field(t("Series name:", "Nom de la série :"), &self.name))
            .when(xy, |d| {
                d.child(field(t("Series X values:", "Valeurs de la série des abscisses X :"), &self.x))
                    .child(field(t("Series Y values:", "Valeurs de la série des ordonnées Y :"), &self.values))
            })
            .when(!xy, |d| d.child(field(t("Series values:", "Valeurs de la série :"), &self.values)))
            .when(self.kind.family() == Family::Bubble, |d| {
                d.child(field(t("Series bubble size:", "Taille des bulles de la série :"), &self.sizes))
            })
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------------- Axis Labels

struct AxisLabels {
    modal: Modal,
    parent: WeakEntity<SelectData>,
    own_sheet: String,
    locale: &'static Locale,
    range: Entity<TextInput>,
}

impl AxisLabels {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let labels = parse_range(self.range.read(cx).text(), self.locale, &self.own_sheet);
        self.modal.close(window, cx);
        self.parent
            .update(cx, |parent, cx| {
                // The categories are those of the chart: of all its series.
                for series in &mut parent.series {
                    series.categories = labels.clone();
                }
                parent.show_range(cx);
            })
            .ok();
    }
}

impl Render for AxisLabels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::dialog(&self.modal, t("Axis Labels", "Étiquettes des axes"))
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(widgets::label(t("Axis label range:", "Plage d'étiquettes des axes :")))
            .child(div().flex().child(RefEdit::new(&self.range, &self.modal)))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------ Hidden and Empty Cells

struct EmptyCells {
    modal: Modal,
    parent: WeakEntity<SelectData>,
    blanks: BlanksAs,
}

const BLANKS: [BlanksAs; 3] = [BlanksAs::Gap, BlanksAs::Zero, BlanksAs::Span];

impl EmptyCells {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let blanks = self.blanks;
        self.modal.close(window, cx);
        self.parent
            .update(cx, |parent, cx| {
                parent.blanks = blanks;
                cx.notify();
            })
            .ok();
    }
}

impl Render for EmptyCells {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options: Vec<SharedString> = vec![
            t("Gaps", "Intervalles").into(),
            t("Zero", "Valeur zéro").into(),
            t("Connect data points with line", "Relier les points de données par une courbe").into(),
        ];
        widgets::dialog(&self.modal, t("Hidden and Empty Cell Settings", "Paramètres des cellules masquées et vides"))
            .width(400.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(widgets::label(t("Show empty cells as:", "Afficher les cellules vides en tant que :")))
                    .child(
                        RadioGroup::new("blanks", options, BLANKS.iter().position(|b| *b == self.blanks)).on_change(cx.listener(
                            |this, ix: &usize, _, cx| {
                                this.blanks = BLANKS[*ix];
                                cx.notify();
                            },
                        )),
                    ),
            )
            // What the engine always does: errors are gaps, hidden cells
            // are plotted.
            .child(Checkbox::new("na", t("Show #N/A as an empty cell", "Afficher #N/A en tant que cellule vide"), true).disabled(true))
            .child(
                Checkbox::new(
                    "hidden",
                    t("Show data in hidden rows and columns", "Afficher les données des lignes et colonnes masquées"),
                    true,
                )
                .disabled(true),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------------------------- Move Chart

struct MoveChart {
    modal: Modal,
    chart: u32,
    sheets: Vec<SharedString>,
    target: usize,
}

pub(crate) fn open_move_chart(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(chart) = app.selected_chart().map(|c| c.id) else {
        return;
    };
    let sheets = app.wb.sheets.iter().map(|s| SharedString::from(s.name.to_string())).collect();
    let target = app.sheet;
    app.open_modal(window, cx, move |modal, _, _| MoveChart {
        modal,
        chart,
        sheets,
        target,
    });
}

/// Move a chart of the current sheet onto another sheet and show it there.
pub(crate) fn move_chart(app: &mut ExcelApp, id: u32, to: usize, window: &mut Window, cx: &mut Cx) {
    let from = app.sheet;
    if to == from || to >= app.wb.sheets.len() {
        return;
    }
    if !app.may(Intent::EditObject(Some(object_id(&Charts, id))), cx) {
        return;
    }
    let Some(at) = app.wb.sheets[from].charts.iter().position(|c| c.id == id) else {
        return;
    };
    let own: Arc<str> = Arc::from(&*app.wb.sheets[from].name);
    app.wb.snapshot();
    let mut chart = app.wb.sheets[from].charts.remove(at);
    // Ranges written without a sheet meant the sheet the chart leaves.
    chart.map_refs(&mut |range| {
        let mut range = range.clone();
        range.sheet.get_or_insert_with(|| own.clone());
        Some(range)
    });
    // On its new sheet it lies on top of what is there.
    chart.z = 0;
    app.wb.sheets[to].charts.push(chart);
    app.deselect_object(cx);
    app.commit(t("Move Chart", "Déplacer le graphique"), cx);
    app.switch_sheet(to, window, cx);
    app.select_object(Some(object_id(&Charts, id)), cx);
}

impl MoveChart {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (chart, target) = (self.chart, self.target);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| move_chart(app, chart, target, window, cx));
    }
}

impl Render for MoveChart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::dialog(&self.modal, t("Move Chart", "Déplacer le graphique"))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(widgets::label(t(
                "Choose where you want the chart to be placed:",
                "Choisissez l'emplacement du graphique :",
            )))
            // Chart sheets do not exist here: the choice is shown, greyed out.
            .child(RadioGroup::new("new-sheet", vec![t("New sheet:", "Nouvelle feuille :").into()], None).disabled(true))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(RadioGroup::new("object-in", vec![t("Object in:", "Objet dans :").into()], Some(0)))
                    .child(
                        Select::new("sheet", self.sheets.clone(), Some(self.target))
                            .width(220.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.target = *ix;
                                cx.notify();
                            })),
                    ),
            )
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Lang, set_lang};
    use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
    use opensheet_engine::Rect;

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
        cx.run_until_parked();
    }

    /// A window with three months of two regions and a chart of them.
    fn open(cx: &mut TestAppContext, kind: ChartKind) -> (Entity<ExcelApp>, &mut VisualTestContext) {
        set_lang(Lang::En);
        cx.update(crate::bind_keys);
        let (app, cx) = cx.add_window_view(ExcelApp::new);
        app.update_in(cx, |app, window, cx| {
            let rows = [["Month", "North", "South"], ["Jan", "12", "8"], ["Feb", "15", "11"], ["Mar", "9", "14"]];
            for (r, row) in rows.iter().enumerate() {
                for (c, text) in row.iter().enumerate() {
                    let _ = app.wb.set_cell_text(0, r as u32, c as u32, text);
                }
            }
            app.commit("setup", cx);
            app.select_rect(Rect::new(0, 0, 3, 2), cx);
            app.insert_chart(kind, window, cx);
        });
        redraw(cx);
        (app, cx)
    }

    fn chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Chart {
        app.update(cx, |app, _| app.wb.sheets.iter().flat_map(|s| s.charts.iter()).next().cloned().expect("a chart"))
    }

    #[gpui::test]
    fn select_data_edits_a_copy_until_ok(cx: &mut TestAppContext) {
        let (app, cx) = open(cx, ChartKind::Column);
        let dialog = app.update_in(cx, |app, window, cx| open_dialog(app, window, cx)).expect("the dialog");
        redraw(cx);
        let range = |cx: &mut VisualTestContext| dialog.read_with(cx, |d, cx| d.range.read(cx).text().to_string());
        assert_eq!(range(cx), "=Sheet1!$A$1:$C$4");
        // Moving a series keeps the range; removing one makes it the rest.
        dialog.update(cx, |d, cx| d.shift(true, cx));
        assert_eq!(dialog.read_with(cx, |d, _| d.selected), Some(1));
        assert_eq!(range(cx), "=Sheet1!$A$1:$C$4");
        // (South is first now; without it the block ends at North.)
        dialog.update(cx, |d, cx| {
            d.selected = Some(0);
            d.remove(cx);
        });
        assert_eq!(dialog.read_with(cx, |d, _| d.series.len()), 1);
        assert_eq!(range(cx), "=Sheet1!$A$1:$B$4");
        // A range typed in the field replaces every series.
        dialog.update(cx, |d, cx| d.range.update(cx, |input, cx| input.set_text("=Sheet1!$A$1:$C$3", cx)));
        redraw(cx);
        let series = dialog.read_with(cx, |d, _| d.series.clone());
        assert_eq!(series.len(), 2);
        assert_eq!(series[1].values.to_text(), "Sheet1!$C$2:$C$3");
        // Switch Row/Column: one series per month.
        dialog.update(cx, |d, cx| d.switch(cx));
        assert_eq!(dialog.read_with(cx, |d, _| d.series.len()), 2);
        assert_eq!(dialog.read_with(cx, |d, _| d.series[0].values.to_text()), "Sheet1!$B$2:$C$2");
        dialog.update(cx, |d, cx| d.switch(cx));
        // Nothing reached the chart yet.
        assert_eq!(chart(&app, cx).series.len(), 2);
        assert_eq!(chart(&app, cx).series[0].values.to_text(), "Sheet1!$B$2:$B$4");
        dialog.update(cx, |d, _| d.blanks = BlanksAs::Span);
        dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
        redraw(cx);
        let after = chart(&app, cx);
        assert_eq!(after.series[0].values.to_text(), "Sheet1!$B$2:$B$3");
        assert_eq!(after.blanks, BlanksAs::Span);
        app.update(cx, |app, _| {
            assert!(!app.modal_open());
            assert_eq!(app.wb.undo_label(), Some("Select Data"));
        });
    }

    #[gpui::test]
    fn edit_series_and_axis_labels_write_into_the_dialog(cx: &mut TestAppContext) {
        let (app, cx) = open(cx, ChartKind::Column);
        let dialog = app.update_in(cx, |app, window, cx| open_dialog(app, window, cx)).expect("the dialog");
        let parent = dialog.downgrade();
        let edit = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, index: Option<usize>| {
            let base = index.and_then(|i| dialog.read_with(cx, |d, _| d.series.get(i).cloned()));
            let parent = parent.clone();
            app.update_in(cx, |app, window, cx| {
                app.open_modal(window, cx, move |modal, _, cx| {
                    EditSeries::new(modal, parent, index, base, ChartKind::Column, "Sheet1".into(), Locale::en(), cx)
                })
            })
        };
        // Edit: the fields show the series; a new name and new values.
        let series = edit(&app, cx, Some(1));
        redraw(cx);
        assert_eq!(series.read_with(cx, |s, cx| s.name.read(cx).text().to_string()), "=Sheet1!$C$1");
        assert_eq!(series.read_with(cx, |s, cx| s.values.read(cx).text().to_string()), "=Sheet1!$C$2:$C$4");
        series.update(cx, |s, cx| {
            s.name.update(cx, |input, cx| input.set_text("South (est.)", cx));
            s.values.update(cx, |input, cx| input.set_text("c2:c3", cx));
        });
        series.update_in(cx, |s, window, cx| s.ok(&(), window, cx));
        let edited = dialog.read_with(cx, |d, _| d.series[1].clone());
        assert_eq!(edited.name, SeriesName::Text("South (est.)".into()));
        assert_eq!(edited.values.to_text(), "Sheet1!$C$2:$C$3");
        // Add: values that cannot be read keep the dialog open.
        let added = edit(&app, cx, None);
        added.update_in(cx, |s, window, cx| s.ok(&(), window, cx));
        assert_eq!(dialog.read_with(cx, |d, _| d.series.len()), 2);
        app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
        added.update(cx, |s, cx| s.values.update(cx, |input, cx| input.set_text("=Sheet1!$B$2:$B$4", cx)));
        added.update_in(cx, |s, window, cx| s.ok(&(), window, cx));
        let series = dialog.read_with(cx, |d, _| d.series.clone());
        assert_eq!(series.len(), 3);
        // The new series shares the categories of the others.
        assert_eq!(series[2].categories, series[0].categories);
        assert_eq!(dialog.read_with(cx, |d, _| d.selected), Some(2));
        // Axis labels are those of every series.
        let labels = app.update_in(cx, |app, window, cx| {
            let parent = parent.clone();
            app.open_modal(window, cx, move |modal, _, cx| AxisLabels {
                range: modal.ref_input(RefStyle::formula(), "=Sheet1!$A$2:$A$3", cx),
                modal,
                parent,
                own_sheet: "Sheet1".into(),
                locale: Locale::en(),
            })
        });
        labels.update_in(cx, |l, window, cx| l.ok(&(), window, cx));
        let series = dialog.read_with(cx, |d, _| d.series.clone());
        assert!(series.iter().all(|s| s.categories.as_ref().map(RefExpr::to_text).as_deref() == Some("Sheet1!$A$2:$A$3")));
    }

    #[gpui::test]
    fn a_chart_moves_to_another_sheet(cx: &mut TestAppContext) {
        let (app, cx) = open(cx, ChartKind::Line);
        let id = chart(&app, cx).id;
        app.update_in(cx, |app, window, cx| {
            app.add_sheet(cx);
            app.switch_sheet(0, window, cx);
            app.select_object(Some(object_id(&Charts, id)), cx);
            move_chart(app, id, 1, window, cx);
        });
        redraw(cx);
        app.update(cx, |app, _| {
            assert!(app.wb.sheets[0].charts.is_empty());
            assert_eq!(app.wb.sheets[1].charts.len(), 1);
            // It shows on its new sheet, selected, plotting the same cells.
            assert_eq!(app.sheet, 1);
            assert_eq!(app.selected_chart().map(|c| c.id), Some(id));
            assert_eq!(app.wb.chart_data(1, &app.wb.sheets[1].charts[0]).series[0].values, vec![Some(12.0), Some(15.0), Some(9.0)]);
            assert_eq!(app.wb.undo_label(), Some("Move Chart"));
        });
        cx.simulate_keystrokes("escape cmd-z");
        assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].charts.len()), 1);
    }

    #[test]
    fn ranges_are_read_as_typed() {
        let en = Locale::en();
        let range = parse_range("=Data!$B$2:$B$9", en, "Sheet1").unwrap();
        assert_eq!(range.to_text(), "Data!$B$2:$B$9");
        // Relative and without a sheet: absolute, on the chart's own sheet.
        let range = parse_range("b2:c4", en, "Sheet1").unwrap();
        assert_eq!(range_text(&range), "=Sheet1!$B$2:$C$4");
        assert_eq!(range.rect(), Rect::new(1, 1, 3, 2));
        assert_eq!(parse_range("='My data'!A1", en, "Sheet1").unwrap().to_text(), "'My data'!$A$1");
        assert!(parse_range("", en, "Sheet1").is_none());
        assert!(parse_range("=SUM(A1:A3)", en, "Sheet1").is_none());
        assert!(parse_range("hello world", en, "Sheet1").is_none());
    }

    #[test]
    fn series_names_are_cells_or_text() {
        let en = Locale::en();
        assert_eq!(parse_series_name("", en, "Sheet1"), SeriesName::None);
        assert_eq!(parse_series_name("Sales", en, "Sheet1"), SeriesName::Text("Sales".into()));
        assert_eq!(parse_series_name("=\"Net \"\"sales\"\"\"", en, "Sheet1"), SeriesName::Text("Net \"sales\"".into()));
        let cell = parse_series_name("=Sheet1!$B$1", en, "Sheet1");
        assert_eq!(series_name_text(&cell), "=Sheet1!$B$1");
        // What a field shows reads back as the same name.
        for name in [SeriesName::None, SeriesName::Text("Q\"1".into()), cell] {
            assert_eq!(parse_series_name(&series_name_text(&name), en, "Sheet1"), name);
        }
    }
}
