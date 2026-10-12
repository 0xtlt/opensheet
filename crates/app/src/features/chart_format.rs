//! Formatting charts: the commands of the "Chart Design" tab that restyle
//! a chart, the "Format" tab (Current Selection, Shape Fill and Outline,
//! Size) and the Format pane (`pane`: Format Chart Area, Format Data
//! Series, Format Axis...; `fields`: what its controls read and write).
//! Selecting the elements of a chart and placing charts is `charts.rs`.

mod fields;
mod pane;
#[cfg(test)]
mod ui_tests;

use super::charts::{self, elements};
use super::{Command, ContextTab, Cx, Feature};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, rows};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{ColorGrid, TextField};
use fields::{Field, fill_of, line_of, line_of_mut, set_fill};
use gpui::{AnyElement, Context, Entity, Focusable, Render, Subscription, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::chart::Fill;
use opensheet_engine::chart_layout::ChartElement;
use opensheet_engine::style::Color;

pub struct ChartFormat;

const FORMAT_TAB: &str = "chart-format";
const PANE: &str = "chart-format";

commands! {
    /// What the user can ask of chart formatting.
    pub enum Cmd {
        // Chart Design
        QuickLayout = "charts-quick-layout", t("Quick Layout", "Disposition rapide");
        ChangeColors = "charts-colors", t("Change Colors", "Modifier les couleurs");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
        SelectData = "charts-select-data", t("Select Data", "Sélectionner des données");
        MoveChart = "charts-move", t("Move Chart", "Déplacer le graphique");
        // Format
        Element = "", t("Chart Elements", "Éléments de graphique");
        FormatSelection = "format-cells", t("Format Selection", "Mise en forme de la sélection");
        ResetStyle = "refresh", t("Reset to Match Style", "Rétablir le style d'origine");
        ShapeFill = "fill", t("Shape Fill", "Remplissage");
        ShapeOutline = "borders", t("Shape Outline", "Contour");
        ShapeEffects = "shapes", t("Shape Effects", "Effets");
        TextFill = "fill", t("Text Fill", "Remplissage du texte");
        TextOutline = "borders", t("Text Outline", "Contour du texte");
        TextEffects = "wordart", t("Text Effects", "Effets du texte");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::QuickLayout => charts::open_quick_layouts(app, window, cx),
            Cmd::ChangeColors => charts::open_colors(app, window, cx),
            Cmd::Styles => charts::open_styles(app, window, cx),
            Cmd::SelectData => charts::open_select_data(app, window, cx),
            Cmd::MoveChart => charts::open_move_chart(app, window, cx),
            Cmd::Element => {
                let entries = element_menu(app);
                app.open_command_menu(entries, 300.0, window, cx);
            }
            Cmd::FormatSelection => open_pane(app, window, cx),
            Cmd::ResetStyle => {
                let element = app.chart_element();
                app.edit_chart(self.label(), cx, move |chart| elements::reset(chart, element));
            }
            Cmd::ShapeFill | Cmd::ShapeOutline => {
                let outline = self == Cmd::ShapeOutline;
                let current = app.selected_chart().and_then(|chart| {
                    let element = app.chart_element();
                    if outline {
                        line_of(chart, element).map(|line| if line.shown == Some(false) { None } else { line.color })
                    } else {
                        fill_of(chart, element).map(|fill| match fill {
                            Fill::Solid(color) => Some(color),
                            _ => None,
                        })
                    }
                });
                let weak = cx.weak_entity();
                let view = cx.new(|_| ColorPopup {
                    app: weak,
                    outline,
                    current,
                });
                let anchor = app.command_anchor;
                charts::open_ribbon_view(app, anchor, view, window, cx);
            }
            // Effects and text formatting are not in the chart model.
            Cmd::ShapeEffects | Cmd::TextFill | Cmd::TextOutline | Cmd::TextEffects => app.not_implemented(self.label(), cx),
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        let Some(chart) = app.selected_chart() else {
            return false;
        };
        match self {
            Cmd::ShapeFill => fill_of(chart, app.chart_element()).is_some(),
            Cmd::ShapeOutline => line_of(chart, app.chart_element()).is_some(),
            Cmd::ShapeEffects | Cmd::TextFill | Cmd::TextOutline | Cmd::TextEffects => false,
            _ => true,
        }
    }
}

/// The elements of the selected chart, for the Chart Elements box.
fn element_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let Some(chart) = app.selected_chart() else {
        return Vec::new();
    };
    let data = app.wb.chart_data(app.sheet, chart);
    let current = app.chart_element().parent();
    elements::listed(chart, &data)
        .into_iter()
        .map(|element| {
            MenuEntry::new(elements::element_name(chart, &data, element), move |a, _, cx| a.select_chart_element(element, cx))
                .checked(element == current)
        })
        .collect()
}

/// Show the Format pane for the selected element of the selected chart.
pub(crate) fn open_pane(app: &mut ExcelApp, _window: &mut Window, cx: &mut Cx) {
    if app.selected_chart().is_none() {
        return;
    }
    let title = elements::pane_title(app.chart_element());
    match app.panes.iter().position(|pane| pane.id == PANE) {
        Some(at) => {
            app.panes[at].title = title.into();
            app.active_pane = at;
            cx.notify();
        }
        None => {
            let weak = cx.weak_entity();
            let view = cx.new(|cx| pane::FormatPane::new(weak, cx));
            app.open_pane(PANE, title, view, cx);
        }
    }
}

/// The selected chart or element changed: the pane is named after it.
pub(crate) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    let title = elements::pane_title(app.chart_element());
    if let Some(pane) = app.panes.iter_mut().find(|pane| pane.id == PANE) {
        if pane.title != title {
            pane.title = title.into();
            cx.notify();
        }
    }
}

// ----------------------------------------------------- Shape Fill and Outline

/// The colours dropped down by Shape Fill and Shape Outline.
struct ColorPopup {
    app: WeakEntity<ExcelApp>,
    outline: bool,
    /// The colour of the selected element; `None` when it has no fill or
    /// outline to speak of.
    current: Option<Option<Color>>,
}

/// Fill the selected element, or draw its outline, in a colour (`None`:
/// no fill, no outline).
fn apply_shape_color(app: &mut ExcelApp, outline: bool, color: Option<Color>, cx: &mut Cx) {
    let element = app.chart_element();
    let label = if outline { Cmd::ShapeOutline.label() } else { Cmd::ShapeFill.label() };
    app.edit_chart(label, cx, move |chart| {
        if outline {
            if let Some(line) = line_of_mut(chart, element) {
                line.shown = Some(color.is_some());
                line.color = color;
            }
        } else {
            set_fill(chart, element, color.map_or(Fill::None, Fill::Solid));
        }
    });
}

impl Render for ColorPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let outline = self.outline;
        let app = self.app.clone();
        let none = if outline { t("No Outline", "Aucun contour") } else { t("No Fill", "Aucun remplissage") };
        div().p(px(8.)).text_size(px(12.)).child(
            ColorGrid::new("shape-color", self.current)
                .none_label(none)
                .more(&self.app)
                .on_pick(cx.listener(move |_, color: &Option<Color>, window, cx| {
                    let color = *color;
                    app.update(cx, |app, cx| {
                        app.close_popup(window, cx);
                        apply_shape_color(app, outline, color, cx);
                    })
                    .ok();
                })),
        )
    }
}

// ------------------------------------------------------------- the Format tab

/// The Height and Width fields of the Size group.
struct SizeFields {
    height: Entity<TextInput>,
    width: Entity<TextInput>,
    _subscriptions: [Subscription; 2],
}

#[derive(Default)]
struct FormatUi {
    size: Option<SizeFields>,
}

fn on_size_event(app: &mut ExcelApp, field: Field, input: &Entity<TextInput>, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    if !matches!(ev, InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred) {
        return;
    }
    let text = input.read(cx).text().to_string();
    if let Some(chart) = app.selected_chart() {
        let mut changed = chart.clone();
        // A field left as it reads is not a new size (it shows a rounded one).
        let reads = fields::value(chart, ChartElement::ChartArea, field, None, app.wb.locale);
        if text != reads && fields::set(&mut changed, ChartElement::ChartArea, field, &text, app.wb.locale) && changed.size != chart.size {
            let size = changed.size;
            app.edit_chart(t("Resize Chart", "Redimensionner le graphique"), cx, move |chart| chart.size = size);
        }
    }
    if !matches!(ev, InputEvent::Blurred) {
        window.focus(&app.grid_focus);
    }
    cx.notify();
}

/// The two fields, created on first use and showing the size of the
/// selected chart unless the user is typing in them.
fn size_fields(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> (Entity<TextInput>, Entity<TextInput>) {
    if app.ext::<FormatUi>().is_none_or(|ui| ui.size.is_none()) {
        let make = |field: Field, window: &mut Window, cx: &mut Cx| {
            let input = cx.new(TextInput::new);
            let subscription = cx.subscribe_in(&input, window, move |app, input, ev, window, cx| {
                on_size_event(app, field, input, ev, window, cx)
            });
            (input, subscription)
        };
        let (height, on_height) = make(Field::Height, window, cx);
        let (width, on_width) = make(Field::Width, window, cx);
        app.ext_mut::<FormatUi>().size = Some(SizeFields {
            height,
            width,
            _subscriptions: [on_height, on_width],
        });
    }
    let (height, width) = match app.ext::<FormatUi>().and_then(|ui| ui.size.as_ref()) {
        Some(size) => (size.height.clone(), size.width.clone()),
        None => unreachable!("the fields were just made"),
    };
    if let Some(chart) = app.selected_chart() {
        for (input, field) in [(&height, Field::Height), (&width, Field::Width)] {
            let text = fields::value(chart, ChartElement::ChartArea, field, None, app.wb.locale);
            let stale = {
                let input = input.read(cx);
                !input.focus_handle(cx).is_focused(window) && input.text() != text
            };
            if stale {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
    }
    (height, width)
}

/// The groups of the "Format" tab of charts.
fn format_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let current = app
        .selected_chart()
        .map(|chart| elements::element_name(chart, &app.wb.chart_data(app.sheet, chart), app.chart_element()))
        .unwrap_or_default();
    let (height, width) = size_fields(app, window, cx);
    let size_row = |label: &'static str, input: &Entity<TextInput>| {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(div().w(px(56.)).text_color(c(theme::TEXT)).child(label))
            .child(TextField::new(input).width(84.0))
            .into_any_element()
    };
    vec![
        group(
            t("Current Selection", "Sélection active"),
            rows(vec![
                app.combo("cf-element", current, 200.0, Cmd::Element.label(), cx, |this, window, cx| {
                    let entries = element_menu(this);
                    this.open_menu("cf-element", entries, 300.0, window, cx);
                }),
                app.cmd_labeled("cf-format", Cmd::FormatSelection, false, cx),
                app.cmd_labeled("cf-reset", Cmd::ResetStyle, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Shape Styles", "Styles de formes"),
            rows(vec![
                app.cmd_labeled("cf-fill", Cmd::ShapeFill, true, cx),
                app.cmd_labeled("cf-outline", Cmd::ShapeOutline, true, cx),
                app.cmd_labeled("cf-effects", Cmd::ShapeEffects, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("WordArt Styles", "Styles WordArt"),
            rows(vec![
                app.cmd_labeled("cf-text-fill", Cmd::TextFill, true, cx),
                app.cmd_labeled("cf-text-outline", Cmd::TextOutline, true, cx),
                app.cmd_labeled("cf-text-effects", Cmd::TextEffects, true, cx),
            ]),
        )
        .into_any_element(),
        div().h_full().child(app.arrange_group("ar", cx)).into_any_element(),
        group(
            t("Size", "Taille"),
            rows(vec![
                size_row(t("Height:", "Hauteur :"), &height),
                size_row(t("Width:", "Largeur :"), &width),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

impl Feature for ChartFormat {
    fn name(&self) -> &'static str {
        "chart_format"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match app.selected_chart() {
            Some(_) => vec![ContextTab {
                id: FORMAT_TAB,
                title: t_ctx("tab", "Format", "Mise en forme"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        format_tab(app, window, cx)
    }
}
