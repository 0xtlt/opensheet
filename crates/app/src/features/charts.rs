//! Charts as floating objects: where they sit, how they are inserted,
//! moved, copied and deleted, the "Chart Design" tab, the chart context
//! menu and the in-place editing of the title. Drawing a chart is
//! `chart_view.rs`; formatting its parts is `chart_format.rs`.

use super::{
    ContextMenu, ContextTab, Cx, Feature, Intent, MenuSection, MenuTarget, ObjectInfo, ObjectVerb, PaintLayer, Painter,
    Placement, ZMove, ZoneEvent, chart_format, object_id,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::chart_view;
use crate::grid::GridLayout;
use crate::i18n::t;
use crate::ribbon::{chart_kind_name, group, row};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use gpui::{AnyElement, Entity, Focusable, Subscription, Window, div, prelude::*, px};
use opensheet_engine::chart::{Chart, ChartKind, ChartTitle, LegendPos};
use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;

pub struct Charts;

const DESIGN_TAB: &str = "chart-design";

/// The title being typed over a chart.
struct TitleEdit {
    chart: u32,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

#[derive(Default)]
struct ChartUi {
    title: Option<TitleEdit>,
}

impl ExcelApp {
    /// The selected chart of the current sheet.
    pub fn selected_chart(&self) -> Option<&Chart> {
        let id = self.object_sel.filter(|id| id.owner == super::index_of(&Charts))?;
        self.wb.chart(self.sheet, id.id)
    }

    /// Insert a chart of the selected data, centred in the window.
    pub fn insert_chart(&mut self, kind: ChartKind, window: &mut Window, cx: &mut Cx) {
        if self.editing.is_some() && !self.commit_edit(None, window, cx) {
            return;
        }
        if !self.may(Intent::EditObject(None), cx) {
            return;
        }
        let mut chart = self.wb.chart_from_range(self.sheet, self.sel().current(), kind);
        if chart.series.is_empty() {
            self.set_status(
                t(
                    "Select the data to chart first.",
                    "Sélectionnez d'abord les données à représenter.",
                ),
                cx,
            );
            return;
        }
        let place = self.place_new_object(chart.size);
        chart.anchor = place.anchor;
        chart.offset = place.offset;
        let id = self.wb.add_chart(self.sheet, chart);
        self.wb.commit(t("Insert Chart", "Insérer un graphique"));
        self.select_object(id.map(|id| object_id(&Charts, id)), cx);
        self.context_tab = Some(DESIGN_TAB);
        self.popup = None;
        self.refresh(cx);
    }

    /// Apply a change to the selected chart.
    pub fn edit_chart(&mut self, label: &str, cx: &mut Cx, f: impl FnOnce(&mut Chart)) {
        if let Some(id) = self.selected_chart().map(|chart| chart.id) {
            self.wb.update_chart(self.sheet, id, f);
            self.commit(label, cx);
        }
    }

    pub fn switch_chart_rows_columns(&mut self, cx: &mut Cx) {
        let Some(mut chart) = self.selected_chart().cloned() else {
            return;
        };
        if chart.switch_row_column(&self.wb, self.sheet) {
            let series = chart.series;
            self.edit_chart(t("Switch Row/Column", "Intervertir les lignes/colonnes"), cx, move |c| {
                c.series = series
            });
        }
    }
}

/// Edit the title of a chart in place (double-click on the chart).
fn begin_title_edit(app: &mut ExcelApp, id: u32, window: &mut Window, cx: &mut Cx) {
    let Some(chart) = app.wb.chart(app.sheet, id) else {
        return;
    };
    let current = app
        .wb
        .chart_data(app.sheet, chart)
        .title
        .unwrap_or_else(|| t("Chart Title", "Titre du graphique").to_string());
    let input = cx.new(TextInput::new);
    input.update(cx, |input, cx| {
        input.set_text(current, cx);
        input.select_all_text(cx);
    });
    let subscription = cx.subscribe_in(&input, window, |app, _, ev, window, cx| on_title_event(app, ev, window, cx));
    window.focus(&input.read(cx).focus_handle(cx));
    app.ext_mut::<ChartUi>().title = Some(TitleEdit {
        chart: id,
        input,
        _subscription: subscription,
    });
    cx.notify();
}

fn on_title_event(app: &mut ExcelApp, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    match ev {
        InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => {
            let Some(edit) = app.ext_mut::<ChartUi>().title.take() else {
                return;
            };
            let text = edit.input.read(cx).text().trim().to_string();
            let title = if text.is_empty() {
                ChartTitle::None
            } else {
                ChartTitle::Text(text)
            };
            if app.wb.chart(app.sheet, edit.chart).is_some_and(|c| c.title != title) {
                app.wb.update_chart(app.sheet, edit.chart, |c| c.title = title);
                app.wb.commit(t("Chart Title", "Titre du graphique"));
            }
            if !matches!(ev, InputEvent::Blurred) {
                window.focus(&app.grid_focus);
            }
            app.refresh(cx);
        }
        InputEvent::Escape => {
            app.ext_mut::<ChartUi>().title = None;
            window.focus(&app.grid_focus);
            cx.notify();
        }
        _ => {}
    }
}

/// The groups of the "Chart Design" tab.
fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Chart Layouts", "Dispositions du graphique"),
            row(vec![
                app.large(
                    "ch-elements",
                    "chart-bar",
                    t("Add Chart Element", "Ajouter un élément de graphique"),
                    true,
                    cx,
                    |this, window, cx| {
                        let Some(chart) = this.selected_chart() else {
                            return;
                        };
                        let has_title = chart.title != ChartTitle::None;
                        let labels = chart.data_labels;
                        let legend = chart.legend;
                        let mut entries = vec![
                            MenuEntry::new(t("Chart Title", "Titre du graphique"), move |a, _, cx| {
                                a.edit_chart(t("Chart Title", "Titre du graphique"), cx, move |c| {
                                    c.title = if has_title { ChartTitle::None } else { ChartTitle::Auto }
                                })
                            })
                            .checked(has_title),
                            MenuEntry::new(t("Data Labels", "Étiquettes de données"), move |a, _, cx| {
                                a.edit_chart(t("Data Labels", "Étiquettes de données"), cx, move |c| {
                                    c.data_labels = !labels
                                })
                            })
                            .checked(labels),
                            MenuEntry::separator(),
                        ];
                        for (pos, name) in [
                            (LegendPos::None, t("No Legend", "Aucune légende")),
                            (LegendPos::Right, t("Legend at Right", "Légende à droite")),
                            (LegendPos::Top, t("Legend at Top", "Légende en haut")),
                            (LegendPos::Left, t("Legend at Left", "Légende à gauche")),
                            (LegendPos::Bottom, t("Legend at Bottom", "Légende en bas")),
                        ] {
                            entries.push(
                                MenuEntry::new(name, move |a, _, cx| {
                                    a.edit_chart(t("Legend", "Légende"), cx, move |c| c.legend = pos)
                                })
                                .checked(legend == pos),
                            );
                        }
                        this.open_menu("ch-elements", entries, 240.0, window, cx);
                    },
                ),
                app.cmd_large("ch-quick-layout", chart_format::Cmd::QuickLayout, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Chart Styles", "Styles du graphique"),
            row(vec![
                app.cmd_large("ch-colors", chart_format::Cmd::ChangeColors, true, cx),
                app.cmd_large("ch-styles", chart_format::Cmd::Styles, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Data", "Données"),
            row(vec![
                app.large(
                    "ch-switch",
                    "table",
                    t("Switch Row/Column", "Intervertir les lignes/colonnes"),
                    false,
                    cx,
                    |this, _, cx| this.switch_chart_rows_columns(cx),
                ),
                app.cmd_large("ch-select-data", chart_format::Cmd::SelectData, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Type", "Type"),
            row(vec![app.large(
                "ch-type",
                "chart-pie",
                t("Change Chart Type", "Modifier le type de graphique"),
                true,
                cx,
                |this, window, cx| {
                    let current = this.selected_chart().map(|c| c.kind);
                    let entries = ChartKind::ALL
                        .iter()
                        .map(|kind| {
                            let kind = *kind;
                            MenuEntry::new(chart_kind_name(kind), move |a, _, cx| {
                                a.edit_chart(t("Change Chart Type", "Modifier le type de graphique"), cx, move |c| {
                                    c.kind = kind
                                })
                            })
                            .checked(current == Some(kind))
                        })
                        .collect();
                    this.open_menu("ch-type", entries, 220.0, window, cx);
                },
            )]),
        )
        .into_any_element(),
        group(
            t("Location", "Emplacement"),
            row(vec![
                app.cmd_large("ch-move", chart_format::Cmd::MoveChart, false, cx),
                app.large(
                    "ch-delete",
                    "trash",
                    t("Delete Chart", "Supprimer le graphique"),
                    false,
                    cx,
                    |this, _, cx| this.delete_selected_object(cx),
                ),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

impl Feature for Charts {
    fn name(&self) -> &'static str {
        "charts"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match app.selected_chart() {
            Some(_) => vec![ContextTab {
                id: DESIGN_TAB,
                title: t("Chart Design", "Création de graphique"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        design_tab(app, cx)
    }

    /// A selected chart shows which cells it plots, coloured by role like
    /// Excel: values blue, categories purple, series names red.
    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Over || p.app.is_editing() {
            return;
        }
        let Some(chart) = p.app.selected_chart() else {
            return;
        };
        for (rect, role) in p.app.wb.chart_ranges(p.app.sheet, chart) {
            let color = match role {
                0 => 0x4472C4,
                1 => 0x8064A2,
                _ => 0xC0504D,
            };
            p.highlight_range(rect, color);
        }
    }

    /// The field shown over a chart while its title is being typed.
    fn overlay_elements(&self, app: &mut ExcelApp, layout: &GridLayout, _window: &mut Window, _cx: &mut Cx) -> Vec<AnyElement> {
        let Some(edit) = app.ext::<ChartUi>().and_then(|ui| ui.title.as_ref()) else {
            return Vec::new();
        };
        let Some(chart) = app.wb.chart(app.sheet, edit.chart) else {
            return Vec::new();
        };
        let (x0, y0, x1, _) = chart_view::chart_rect(layout, chart);
        let z = app.zoom;
        let size = (18.67 * z).max(8.0);
        let line_h = (size * 1.25).round();
        let pad = (10.0 * z).round();
        let font = app.fonts.resolve(&Arc::from(opensheet_engine::style::DEFAULT_FONT));
        vec![
            div()
                .absolute()
                .left(px(x0 + pad))
                .top(px(y0 + pad - 4.0 * z))
                .w(px((x1 - x0 - 2.0 * pad).max(40.0)))
                .h(px(line_h + 4.0))
                .occlude()
                .bg(c(0xFFFFFF))
                .border_1()
                .border_color(c(theme::ACCENT))
                .px(px(4.0))
                .flex()
                .items_center()
                .font_family(font)
                .text_size(px(size))
                .line_height(px(line_h))
                .text_color(c(0x595959))
                .child(edit.input.clone())
                .into_any_element(),
        ]
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        let MenuTarget::Object(id) = target else {
            return;
        };
        if id.owner != super::index_of(self) {
            return;
        }
        menu.add(
            MenuSection::Data,
            MenuEntry::new(t("Switch Row/Column", "Intervertir les lignes/colonnes"), |a, _, cx| {
                a.switch_chart_rows_columns(cx)
            })
            .icon("table"),
        );
        let current = app.wb.chart(app.sheet, id.id).map(|c| c.kind);
        let kinds = ChartKind::ALL
            .iter()
            .map(|kind| {
                let kind = *kind;
                MenuEntry::new(chart_kind_name(kind), move |a, _, cx| {
                    a.edit_chart(t("Change Chart Type", "Modifier le type de graphique"), cx, move |c| {
                        c.kind = kind
                    })
                })
                .checked(current == Some(kind))
            })
            .collect();
        menu.add(
            MenuSection::Data,
            MenuEntry::submenu(t("Change Chart Type", "Modifier le type de graphique"), kinds),
        );
        menu.add(
            MenuSection::Edit,
            MenuEntry::new(t("Delete Chart", "Supprimer le graphique"), |a, _, cx| {
                a.delete_selected_object(cx)
            })
            .icon("trash"),
        );
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        app.wb.sheets[sheet]
            .charts
            .iter()
            .map(|chart| ObjectInfo {
                id: chart.id,
                place: Placement {
                    anchor: chart.anchor,
                    offset: chart.offset,
                    size: chart.size,
                },
                min_size: (120.0, 80.0),
                keep_aspect: false,
            })
            .collect()
    }

    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, _rect: (f32, f32, f32, f32)) {
        let Some(chart) = p.app.wb.chart(p.app.sheet, object.id) else {
            return;
        };
        let style = chart_view::ChartStyle {
            font: p.app.fonts.resolve(&Arc::from(opensheet_engine::style::DEFAULT_FONT)),
            placeholder_title: t("Chart Title", "Titre du graphique"),
            series_prefix: t("Series", "Série"),
            locale: p.app.wb.locale,
        };
        let data = p.app.wb.chart_data(p.app.sheet, chart);
        p.extend(chart_view::build_chart(chart, &data, p.layout, p.ts, &style, false));
    }

    fn place_object(&self, app: &mut ExcelApp, id: u32, place: Placement) {
        app.wb.update_chart(app.sheet, id, |c| {
            c.anchor = place.anchor;
            c.offset = place.offset;
            c.size = place.size;
        });
    }

    fn object_label(&self, _id: u32, verb: ObjectVerb) -> &'static str {
        match verb {
            ObjectVerb::Move => t("Move Chart", "Déplacer le graphique"),
            ObjectVerb::Resize => t("Resize Chart", "Redimensionner le graphique"),
            ObjectVerb::Delete => t("Delete Chart", "Supprimer le graphique"),
            ObjectVerb::Paste => t("Paste", "Coller"),
            ObjectVerb::Reorder => t("Change Order", "Changer l'ordre"),
        }
    }

    fn delete_object(&self, app: &mut ExcelApp, id: u32) {
        app.wb.remove_chart(app.sheet, id);
    }

    fn copy_object(&self, app: &ExcelApp, id: u32) -> Option<Rc<dyn Any>> {
        let chart = app.wb.chart(app.sheet, id)?.clone();
        Some(Rc::new(chart))
    }

    fn paste_object(&self, app: &mut ExcelApp, data: &dyn Any, at: ((u32, u32), (f32, f32))) -> Option<u32> {
        let mut chart = data.downcast_ref::<Chart>()?.clone();
        chart.anchor = at.0;
        chart.offset = at.1;
        app.wb.add_chart(app.sheet, chart)
    }

    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        let sheet = app.sheet;
        let Some(from) = app.wb.sheets[sheet].charts.iter().position(|c| c.id == id) else {
            return;
        };
        let last = app.wb.sheets[sheet].charts.len() - 1;
        let to = match mv {
            ZMove::ToFront => last,
            ZMove::Forward => (from + 1).min(last),
            ZMove::Backward => from.saturating_sub(1),
            ZMove::ToBack => 0,
        };
        if to != from {
            app.wb.snapshot();
            let charts = &mut app.wb.sheets[sheet].charts;
            let chart = charts.remove(from);
            charts.insert(to, chart);
        }
    }

    fn object_double_click(&self, app: &mut ExcelApp, id: u32, _ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        begin_title_edit(app, id, window, cx);
    }
}
