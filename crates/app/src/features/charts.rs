//! Charts as floating objects: where they sit, how they are inserted,
//! moved, copied and deleted, selecting the elements of a chart with the
//! mouse and the keyboard, typing titles in place, and the two buttons
//! that float beside a selected chart.
//!
//! The rest of what a user can do to a chart is in the modules below
//! (`elements`: names, deleting, context menus; `design`: the "Chart
//! Design" tab; `insert`: the galleries and the Insert Chart dialog;
//! `data`: Select Data and Move Chart). Drawing a chart is `chart_view.rs`;
//! formatting its parts is `chart_format.rs`.

mod data;
mod design;
pub(crate) mod elements;
mod gallery;
mod insert;

#[cfg(test)]
mod ui_tests;

pub(crate) use data::{open_move_chart, open_select_data};
pub(crate) use design::{label_positions, open_colors, open_quick_layouts, open_styles};
pub use insert::insert_group;

use super::{
    Command, ContextMenu, ContextTab, Cx, Feature, Intent, MenuTarget, ObjectInfo, ObjectVerb,
    PaintLayer, Painter, Placement, ZMove, Zone, ZoneEvent, ZoneResponse, chart_format, object_id,
};
use crate::app::{
    Cancel, ClearContents, ExcelApp, MoveDown, MoveLeft, MoveRight, MoveUp, Popup, PopupKind,
};
use crate::chart_view::{self, ChartStyle};
use crate::grid::GridLayout;
use crate::i18n::t;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use gpui::{
    AnyElement, AnyView, CursorStyle, Entity, FocusHandle, Focusable, SharedString, Subscription,
    Window, div, point, prelude::*, px,
};
use opensheet_engine::chart::{Chart, ChartKind, ChartTitle, ManualLayout};
use opensheet_engine::chart_layout::{ChartElement, ChartLayout};
use opensheet_engine::drawing::{ObjectRef, ZOrder};
use opensheet_engine::style::{Color, Style, Underline};
use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;

pub struct Charts;

pub(crate) const DESIGN_TAB: &str = "chart-design";

commands! {
    /// What the user can ask of a chart as a whole.
    pub enum Cmd {
        Recommended = "chart-bar", t("Recommended Charts", "Graphiques recommandés");
        AddElement = "charts-add-element", t("Add Chart Element", "Ajouter un élément de graphique");
        SwitchRowColumn = "charts-switch", t("Switch Row/Column", "Intervertir les lignes/colonnes");
        ChangeType = "charts-change-type", t("Change Chart Type", "Modifier le type de graphique");
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
            Cmd::Recommended => insert::open_insert_dialog(app, None, window, cx),
            Cmd::AddElement => {
                let entries = design::add_element_menu(app);
                app.open_command_menu(entries, 250.0, window, cx);
            }
            Cmd::SwitchRowColumn => app.switch_chart_rows_columns(cx),
            Cmd::ChangeType => insert::open_change_type_dialog(app, window, cx),
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::Recommended => true,
            _ => app.selected_chart().is_some(),
        }
    }
}

/// What the zone over a chart carries: the chart as it was last painted,
/// to tell which of its elements lies under the pointer.
pub(crate) struct ChartZone {
    pub chart: u32,
    /// Corner of the chart in grid-element coordinates.
    pub origin: (f32, f32),
    pub layout: Rc<ChartLayout>,
}

/// The buttons that float beside a selected chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloatButton {
    Elements,
    Styles,
}

/// A title being typed over a chart.
struct TextEdit {
    chart: u32,
    element: ChartElement,
    input: Entity<TextInput>,
    _subscription: Subscription,
}

/// A handle of the plot area or of the legend of the selected chart: the
/// zone over it.
struct GripZone {
    chart: u32,
    element: ChartElement,
    /// Which of the eight handles, as [`resized`] numbers them.
    grip: u8,
    origin: (f32, f32),
    layout: Rc<ChartLayout>,
}

/// An element of a chart being dragged to another place, or resized by
/// one of its handles.
#[derive(Clone, Copy)]
struct ElementDrag {
    chart: u32,
    element: ChartElement,
    /// The point whose label is dragged, for data labels.
    point: u32,
    /// The handle held, for a resize.
    grip: Option<u8>,
    /// Where the press landed, in grid-element coordinates.
    start: (f32, f32),
    /// The frame of the element when it was pressed, in pixels from the
    /// corner of the chart, and the size of the chart then.
    frame: [f32; 4],
    size: (f32, f32),
    /// How far the label had been dragged before.
    offset: (f64, f64),
    /// The pointer left the spot: a drag, not a click.
    moved: bool,
    /// The sheet is protected: the element stays where it is.
    refused: bool,
    /// Without a drag, the click starts typing the title it landed on.
    types: bool,
}

/// The frame (x0, y0, x1, y1) of an element after one of its handles was
/// dragged by (dx, dy): corners 0 to 3 clockwise from the top left one,
/// then the middles of the top, right, bottom and left edges. It keeps a
/// size of `min` at least and stays inside a chart of `size`.
pub(crate) fn resized(
    frame: [f32; 4],
    grip: u8,
    (dx, dy): (f32, f32),
    size: (f32, f32),
    min: f32,
) -> [f32; 4] {
    let [mut x0, mut y0, mut x1, mut y1] = frame;
    let (left, top, right, bottom) = match grip {
        0 => (true, true, false, false),
        1 => (false, true, true, false),
        2 => (false, false, true, true),
        3 => (true, false, false, true),
        4 => (false, true, false, false),
        5 => (false, false, true, false),
        6 => (false, false, false, true),
        _ => (true, false, false, false),
    };
    if left {
        x0 = (x0 + dx).clamp(0.0, (x1 - min).max(0.0));
    }
    if right {
        x1 = (x1 + dx).clamp((x0 + min).min(size.0), size.0);
    }
    if top {
        y0 = (y0 + dy).clamp(0.0, (y1 - min).max(0.0));
    }
    if bottom {
        y1 = (y1 + dy).clamp((y0 + min).min(size.1), size.1);
    }
    [x0, y0, x1, y1]
}

/// The frame of an element dragged by (dx, dy), kept inside its chart.
pub(crate) fn moved(frame: [f32; 4], (dx, dy): (f32, f32), size: (f32, f32)) -> [f32; 4] {
    let (w, h) = (frame[2] - frame[0], frame[3] - frame[1]);
    let x = (frame[0] + dx).clamp(0.0, (size.0 - w).max(0.0));
    let y = (frame[1] + dy).clamp(0.0, (size.1 - h).max(0.0));
    [x, y, x + w, y + h]
}

/// What a formatting command of the Home tab does to the text of an
/// element of a chart. `change` is what the command does to a cell
/// style: it is tried on two styles that differ in everything, and what
/// comes out the same of both is what the command sets. Bold, italics and
/// underline are switches: they turn on unless the text has them already.
/// Returns false for an element without text.
pub(crate) fn restyle(
    chart: &mut Chart,
    element: ChartElement,
    change: &dyn Fn(&mut Style),
) -> bool {
    let Some(current) = chart_format::text_of(chart, element) else {
        return false;
    };
    let probe = |name: &str, size: u16, on: bool, color: u32| {
        let mut style = Style::default();
        style.font.set_name(name);
        style.font.size = size;
        style.font.bold = on;
        style.font.italic = on;
        style.font.underline = if on {
            Underline::Single
        } else {
            Underline::None
        };
        style.font.strike = on;
        style.font.color = Some(Color(color));
        change(&mut style);
        style.font
    };
    let (a, b) = (
        probe("\u{1}a", 3, false, 0x010203),
        probe("\u{1}b", 5, true, 0x030201),
    );
    let flip = |was: Option<bool>| (was != Some(true)).then_some(true);
    element.format_text(chart, |text| {
        if a.name == b.name {
            text.font = Some(a.name.clone());
        }
        if a.size == b.size {
            text.size = Some(a.size_pt());
        }
        if a.bold == b.bold {
            text.bold = flip(current.bold);
        }
        if a.italic == b.italic {
            text.italic = flip(current.italic);
        }
        if a.underline == b.underline {
            text.underline = flip(current.underline);
        }
        if a.color == b.color {
            text.color = a.color;
        }
    })
}

/// The size, in points, of the text the Home tab would format: that of
/// the selected element of the selected chart.
pub(crate) fn home_text_size(app: &ExcelApp) -> Option<f32> {
    let chart = app.selected_chart()?;
    chart_format::text_size(chart, app.chart_element())
}

/// Whether an element can be dragged to another place of its chart.
fn draggable(chart: &Chart, element: ChartElement) -> bool {
    match element {
        ChartElement::Title
        | ChartElement::Legend
        | ChartElement::AxisTitle(_)
        | ChartElement::DataLabels(_) => true,
        // The plot of a 3-D chart is placed by its camera.
        ChartElement::PlotArea => !chart.kind.is_3d(),
        _ => false,
    }
}

/// Put a dragged element where its frame now is (`frame`, in a chart of
/// `size` pixels); a data label is shifted from where it was instead.
fn place_element(chart: &mut Chart, drag: &ElementDrag, frame: [f32; 4]) {
    let (w, h) = (drag.size.0.max(1.0) as f64, drag.size.1.max(1.0) as f64);
    let (x, y) = (frame[0] as f64 / w, frame[1] as f64 / h);
    let (fw, fh) = (
        (frame[2] - frame[0]) as f64 / w,
        (frame[3] - frame[1]) as f64 / h,
    );
    match drag.element {
        ChartElement::Title => chart.title_layout = Some(ManualLayout::at(x, y)),
        ChartElement::AxisTitle(axis) => {
            axis.of_mut(&mut chart.axes).title_layout = Some(ManualLayout::at(x, y))
        }
        ChartElement::Legend => {
            // A legend that was only moved keeps the size it takes by itself.
            let sized = drag.grip.is_some()
                || chart
                    .legend_layout
                    .is_some_and(|m| m.w.is_some() || m.h.is_some());
            chart.legend_layout = Some(ManualLayout {
                x,
                y,
                w: sized.then_some(fw),
                h: sized.then_some(fh),
            });
        }
        ChartElement::PlotArea => {
            chart.plot_layout = Some(ManualLayout {
                x,
                y,
                w: Some(fw),
                h: Some(fh),
            });
            chart.plot_layout_outer = false;
        }
        ChartElement::DataLabels(s) => {
            let dx = drag.offset.0 + (frame[0] - drag.frame[0]) as f64 / w;
            let dy = drag.offset.1 + (frame[1] - drag.frame[1]) as f64 / h;
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.set_label_offset(drag.point, dx, dy);
            }
        }
        _ => {}
    }
}

/// The pointer moved while an element of a chart is held.
fn drag_element(app: &mut ExcelApp, ev: &ZoneEvent, cx: &mut Cx) {
    let Some(mut drag) = app.ext_mut::<ChartUi>().drag else {
        return;
    };
    let by = (ev.x - drag.start.0, ev.y - drag.start.1);
    if !drag.moved && by.0.abs() <= 3.0 && by.1.abs() <= 3.0 {
        return;
    }
    // A protected sheet keeps its charts as they are.
    if !drag.moved {
        drag.moved = true;
        drag.refused = !app.may(Intent::EditObject(Some(object_id(&Charts, drag.chart))), cx);
        app.ext_mut::<ChartUi>().drag = Some(drag);
    }
    if drag.refused {
        return;
    }
    let frame = match drag.grip {
        Some(grip) => resized(drag.frame, grip, by, drag.size, 24.0),
        None => moved(drag.frame, by, drag.size),
    };
    app.wb
        .update_chart(app.sheet, drag.chart, |c| place_element(c, &drag, frame));
    cx.notify();
}

/// Hold an element of a chart, as it was last painted, for dragging.
fn begin_drag(
    app: &mut ExcelApp,
    chart: u32,
    layout: &ChartLayout,
    element: ChartElement,
    grip: Option<u8>,
    ev: &ZoneEvent,
    origin: (f32, f32),
) -> bool {
    let (x, y) = (ev.x - origin.0, ev.y - origin.1);
    let mut point = 0;
    let frame = match element {
        // One label is dragged: the one under the pointer.
        ChartElement::DataLabels(s) => match layout
            .data_label_at(x, y)
            .filter(|(series, _)| *series == s)
        {
            Some((_, k)) => {
                point = k;
                layout.data_label_box(s, k)
            }
            None => None,
        },
        other => layout.bounds(other),
    };
    let Some(frame) = frame else {
        return false;
    };
    let offset = match (element, app.wb.chart(app.sheet, chart)) {
        (ChartElement::DataLabels(s), Some(c)) => c
            .series
            .get(s as usize)
            .and_then(|series| series.label_offset(point as usize))
            .unwrap_or((0.0, 0.0)),
        _ => (0.0, 0.0),
    };
    app.ext_mut::<ChartUi>().drag = Some(ElementDrag {
        chart,
        element,
        point,
        grip,
        start: (ev.x, ev.y),
        frame,
        size: layout.size(),
        offset,
        moved: false,
        refused: false,
        types: false,
    });
    true
}

#[derive(Default)]
pub(crate) struct ChartUi {
    edit: Option<TextEdit>,
    drag: Option<ElementDrag>,
    /// The selected element of the selected chart.
    element: ChartElement,
    /// Holds the keyboard while an element other than the chart area is
    /// selected: Delete then removes that element, not the chart.
    keys: Option<FocusHandle>,
}

impl ExcelApp {
    /// The selected chart of the current sheet.
    pub fn selected_chart(&self) -> Option<&Chart> {
        let id = self
            .object_sel
            .filter(|id| id.owner == super::index_of(&Charts))?;
        self.wb.chart(self.sheet, id.id)
    }

    /// The selected element of the selected chart (the chart area when
    /// nothing else is, or when what was selected no longer exists).
    pub fn chart_element(&self) -> ChartElement {
        let element = self
            .ext::<ChartUi>()
            .map(|ui| ui.element)
            .unwrap_or_default();
        match self.selected_chart() {
            Some(chart) if elements::exists(chart, element) => element,
            _ => ChartElement::ChartArea,
        }
    }

    pub fn select_chart_element(&mut self, element: ChartElement, cx: &mut Cx) {
        self.ext_mut::<ChartUi>().element = element;
        // The Format pane is named after what it formats.
        chart_format::selection_changed(self, cx);
        cx.notify();
    }

    /// The font charts are drawn in.
    pub fn chart_font(&self) -> SharedString {
        self.fonts
            .resolve(&Arc::from(opensheet_engine::style::DEFAULT_FONT))
    }

    pub fn chart_style(&self) -> ChartStyle {
        ChartStyle {
            font: self.chart_font(),
            locale: self.wb.locale,
            fonts: Some(self.fonts.clone()),
        }
    }

    /// Insert a chart of the selected data, centred in the window.
    pub fn insert_chart(&mut self, kind: ChartKind, window: &mut Window, cx: &mut Cx) {
        let chart = self
            .wb
            .chart_from_range(self.sheet, self.sel().current(), kind);
        self.insert_built_chart(chart, window, cx);
    }

    /// Put a chart built from the selection on the sheet and select it.
    pub(crate) fn insert_built_chart(
        &mut self,
        mut chart: Chart,
        window: &mut Window,
        cx: &mut Cx,
    ) {
        if self.editing.is_some() && !self.commit_edit(None, window, cx) {
            return;
        }
        if !self.may(Intent::EditObject(None), cx) {
            return;
        }
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

    /// Apply a change to the selected chart, as one undo step.
    pub fn edit_chart(&mut self, label: &str, cx: &mut Cx, f: impl FnOnce(&mut Chart)) {
        let Some(id) = self.selected_chart().map(|chart| chart.id) else {
            return;
        };
        // A protected sheet keeps its charts as they are.
        if self.may(Intent::EditObject(Some(object_id(&Charts, id))), cx) {
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
            self.edit_chart(Cmd::SwitchRowColumn.label(), cx, move |c| c.series = series);
        }
    }

    /// The chart as it was last painted, when it is on screen.
    pub(crate) fn painted_chart(&self, id: u32) -> Option<&ChartZone> {
        self.zones
            .iter()
            .find_map(|zone| zone.payload::<ChartZone>().filter(|z| z.chart == id))
    }
}

/// Drop a view down from a ribbon control (`None`: at the pointer).
pub(crate) fn open_ribbon_view(
    app: &mut ExcelApp,
    anchor: Option<&'static str>,
    view: impl Into<AnyView>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let held = anchor.and_then(|a| app.anchors.borrow().get(a).copied());
    let pos = match held {
        Some(b) => point(b.origin.x, b.origin.y + b.size.height + px(2.)),
        None => window.mouse_position(),
    };
    let mut popup = Popup::new(PopupKind::View(view.into()), pos, 0.0).max_height(640.0);
    if let Some(anchor) = anchor {
        popup = popup.anchored(anchor);
    }
    app.popup = Some(popup);
    cx.notify();
}

// ------------------------------------------------------------ titles in place

/// Type the chart title or an axis title where it stands.
pub(crate) fn begin_text_edit(
    app: &mut ExcelApp,
    id: u32,
    element: ChartElement,
    window: &mut Window,
    cx: &mut Cx,
) {
    let Some(chart) = app.wb.chart(app.sheet, id) else {
        return;
    };
    let current = match element {
        ChartElement::Title => app
            .wb
            .chart_data(app.sheet, chart)
            .title
            .unwrap_or_else(|| t("Chart Title", "Titre du graphique").to_string()),
        ChartElement::AxisTitle(axis) => axis.of(&chart.axes).title.clone().unwrap_or_default(),
        _ => return,
    };
    if !app.may(Intent::EditObject(Some(object_id(&Charts, id))), cx) {
        return;
    }
    let input = cx.new(TextInput::new);
    input.update(cx, |input, cx| {
        input.set_text(current, cx);
        input.select_all_text(cx);
    });
    let subscription = cx.subscribe_in(&input, window, |app, _, ev, window, cx| {
        on_text_event(app, ev, window, cx)
    });
    window.focus(&input.read(cx).focus_handle(cx));
    app.ext_mut::<ChartUi>().edit = Some(TextEdit {
        chart: id,
        element,
        input,
        _subscription: subscription,
    });
    cx.notify();
}

/// Store the title being typed, if any.
fn finish_text_edit(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(edit) = app.ext_mut::<ChartUi>().edit.take() else {
        return;
    };
    let text = edit.input.read(cx).text().trim().to_string();
    let Some(chart) = app.wb.chart(app.sheet, edit.chart) else {
        return;
    };
    match edit.element {
        ChartElement::Title => {
            let title = if text.is_empty() {
                ChartTitle::None
            } else {
                ChartTitle::Text(text)
            };
            // Leaving an automatic title as it reads keeps it automatic.
            let shown = app.wb.chart_data(app.sheet, chart).title;
            let unchanged = chart.title == title
                || matches!(&title, ChartTitle::Text(text) if chart.title == ChartTitle::Auto && shown.as_deref().unwrap_or(t("Chart Title", "Titre du graphique")) == text);
            if !unchanged {
                app.wb
                    .update_chart(app.sheet, edit.chart, |c| c.title = title);
                app.wb.commit(t("Chart Title", "Titre du graphique"));
            }
        }
        ChartElement::AxisTitle(axis) => {
            let title = (!text.is_empty()).then_some(text);
            if axis.of(&chart.axes).title != title {
                app.wb.update_chart(app.sheet, edit.chart, |c| {
                    axis.of_mut(&mut c.axes).title = title
                });
                app.wb.commit(t("Axis Title", "Titre de l'axe"));
            }
        }
        _ => {}
    }
    app.refresh(cx);
}

fn on_text_event(app: &mut ExcelApp, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    match ev {
        InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => {
            finish_text_edit(app, cx);
            if !matches!(ev, InputEvent::Blurred) {
                window.focus(&app.grid_focus);
            }
        }
        InputEvent::Escape => {
            app.ext_mut::<ChartUi>().edit = None;
            window.focus(&app.grid_focus);
            cx.notify();
        }
        _ => {}
    }
}

// ----------------------------------------------------------------- keyboard

/// Delete / Backspace while an element of a chart is selected.
fn delete_selected_element(app: &mut ExcelApp, cx: &mut Cx) {
    let element = app.chart_element();
    if element == ChartElement::ChartArea {
        app.delete_selected_object(cx);
        return;
    }
    app.edit_chart(t("Delete", "Supprimer"), cx, move |chart| {
        elements::delete(chart, element);
    });
    app.select_chart_element(ChartElement::ChartArea, cx);
}

/// The arrow keys walk the elements of the chart, as in Excel; Left and
/// Right walk the points of a series once one of them is selected.
fn step_element(app: &mut ExcelApp, forward: bool, sideways: bool, cx: &mut Cx) {
    let Some(chart) = app.selected_chart() else {
        return;
    };
    let current = app.chart_element();
    let data = app.wb.chart_data(app.sheet, chart);
    if let (ChartElement::Point(s, k), true) = (current, sideways) {
        let count = data.series.get(s as usize).map_or(0, |sd| sd.values.len()) as u32;
        if count > 0 {
            let next = if forward {
                (k + 1) % count
            } else {
                (k + count - 1) % count
            };
            app.select_chart_element(ChartElement::Point(s, next), cx);
        }
        return;
    }
    let all = elements::listed(chart, &data);
    let at = all.iter().position(|e| *e == current.parent()).unwrap_or(0);
    let next = if forward {
        (at + 1) % all.len()
    } else {
        (at + all.len() - 1) % all.len()
    };
    app.select_chart_element(all[next], cx);
}

// ------------------------------------------------------------------ feature

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

    fn render_tab(
        &self,
        app: &mut ExcelApp,
        _tab: &'static str,
        _window: &mut Window,
        cx: &mut Cx,
    ) -> Vec<AnyElement> {
        design::design_tab(app, cx)
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

    /// The field shown over a chart while a title is being typed, and what
    /// holds the keyboard while one of its elements is selected.
    fn overlay_elements(
        &self,
        app: &mut ExcelApp,
        layout: &GridLayout,
        window: &mut Window,
        cx: &mut Cx,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let element = app.chart_element();
        let editing = app.ext::<ChartUi>().is_some_and(|ui| ui.edit.is_some());
        if app.selected_chart().is_some()
            || app.ext::<ChartUi>().is_some_and(|ui| ui.keys.is_some())
        {
            let keys = match &app.ext_mut::<ChartUi>().keys {
                Some(keys) => keys.clone(),
                None => {
                    let keys = cx.focus_handle();
                    app.ext_mut::<ChartUi>().keys = Some(keys.clone());
                    keys
                }
            };
            // The keys of the grid act on the chart as a whole: they come
            // here instead while one of its elements is selected.
            let wanted = element != ChartElement::ChartArea && !editing && !app.is_editing();
            if wanted && app.grid_focus.is_focused(window) {
                window.focus(&keys);
            } else if !wanted && keys.is_focused(window) {
                window.focus(&app.grid_focus);
            }
            out.push(
                div()
                    .absolute()
                    .size_0()
                    .track_focus(&keys)
                    .on_action(
                        cx.listener(|app, _: &ClearContents, _, cx| {
                            delete_selected_element(app, cx)
                        }),
                    )
                    .on_action(cx.listener(|app, _: &Cancel, _, cx| {
                        app.select_chart_element(ChartElement::ChartArea, cx)
                    }))
                    .on_action(
                        cx.listener(|app, _: &MoveRight, _, cx| step_element(app, true, true, cx)),
                    )
                    .on_action(
                        cx.listener(|app, _: &MoveLeft, _, cx| step_element(app, false, true, cx)),
                    )
                    .on_action(
                        cx.listener(|app, _: &MoveDown, _, cx| step_element(app, true, false, cx)),
                    )
                    .on_action(
                        cx.listener(|app, _: &MoveUp, _, cx| step_element(app, false, false, cx)),
                    )
                    .into_any_element(),
            );
        }

        let Some(edit) = app.ext::<ChartUi>().and_then(|ui| ui.edit.as_ref()) else {
            return out;
        };
        let Some(chart) = app.wb.chart(app.sheet, edit.chart) else {
            return out;
        };
        let (x0, y0, x1, _) = chart_view::chart_rect(layout, chart);
        let z = app.zoom;
        let title = edit.element == ChartElement::Title;
        let size = ((if title { 18.67 } else { 13.33 }) * z).max(8.0);
        let line_h = (size * 1.25).round();
        // Where the text stands on the chart, when it is drawn there.
        let bounds = app
            .painted_chart(edit.chart)
            .and_then(|zone| zone.layout.bounds(edit.element));
        let (left, top, width) = match bounds {
            Some(b) if title => (x0 + 10.0 * z, y0 + b[1] - 2.0, x1 - x0 - 20.0 * z),
            Some(b) => {
                let width = (b[2] - b[0]).max(160.0 * z);
                let centre = (b[0] + b[2]) / 2.0;
                (
                    (x0 + centre - width / 2.0).max(x0 + 4.0),
                    y0 + (b[1] + b[3]) / 2.0 - line_h / 2.0 - 2.0,
                    width,
                )
            }
            None => (x0 + 10.0 * z, y0 + 6.0 * z, x1 - x0 - 20.0 * z),
        };
        out.push(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width.max(40.0)))
                .h(px(line_h + 4.0))
                .occlude()
                .bg(c(0xFFFFFF))
                .border_1()
                .border_color(c(theme::ACCENT))
                .px(px(4.0))
                .flex()
                .items_center()
                .font_family(app.chart_font())
                .text_size(px(size))
                .line_height(px(line_h))
                .text_color(c(0x595959))
                .child(edit.input.clone())
                .into_any_element(),
        );
        out
    }

    fn zone_mouse_down(
        &self,
        app: &mut ExcelApp,
        zone: &Zone,
        ev: &ZoneEvent,
        window: &mut Window,
        cx: &mut Cx,
    ) -> ZoneResponse {
        if let Some(button) = zone.payload::<FloatButton>() {
            if ev.button != gpui::MouseButton::Left {
                return ZoneResponse::Handled;
            }
            // The list opens beside its button.
            let pos = match app.current_layout() {
                Some(layout) => layout.to_window(zone.bounds.2 + 6.0, zone.bounds.1),
                None => ev.position,
            };
            match button {
                FloatButton::Elements => {
                    let view = design::elements_flyout(app, cx);
                    app.open_popup_view(pos, view, cx);
                }
                FloatButton::Styles => {
                    let view = design::styles_flyout(app, cx);
                    app.open_popup_view(pos, view, cx);
                }
            }
            return ZoneResponse::Handled;
        }
        if let Some(grip) = zone.payload::<GripZone>() {
            if ev.button == gpui::MouseButton::Left
                && begin_drag(
                    app,
                    grip.chart,
                    &grip.layout,
                    grip.element,
                    Some(grip.grip),
                    ev,
                    grip.origin,
                )
            {
                return ZoneResponse::Capture;
            }
            return ZoneResponse::Handled;
        }
        let Some(hit) = zone.payload::<ChartZone>() else {
            return ZoneResponse::Ignored;
        };
        // What is being typed is stored before anything else happens.
        if app.ext::<ChartUi>().is_some_and(|ui| ui.edit.is_some()) {
            finish_text_edit(app, cx);
        }
        if app.editing.is_some() && !app.commit_edit(None, window, cx) {
            return ZoneResponse::Handled;
        }
        let id = object_id(self, hit.chart);
        let current = if app.object_sel == Some(id) {
            app.chart_element()
        } else {
            ChartElement::ChartArea
        };
        let under = hit.layout.hit(ev.x - hit.origin.0, ev.y - hit.origin.1);
        // A click on a series selects the series; a click on one of its
        // points once the series is selected selects that point.
        let target = match under {
            ChartElement::Point(s, _)
                if current.series() == Some(s as usize)
                    && matches!(current, ChartElement::Series(_) | ChartElement::Point(..)) =>
            {
                under
            }
            ChartElement::LegendEntry(_)
                if matches!(current, ChartElement::Legend | ChartElement::LegendEntry(_)) =>
            {
                under
            }
            other => other.parent(),
        };
        app.select_object(Some(id), cx);
        app.select_chart_element(target, cx);
        if ev.button != gpui::MouseButton::Left || target == ChartElement::ChartArea {
            // The chart itself: selected, moved and given its menu like
            // any other object.
            return ZoneResponse::Ignored;
        }
        app.popup = None;
        if ev.click_count >= 2 {
            chart_format::open_pane(app, window, cx);
            return ZoneResponse::Handled;
        }
        if let Some(keys) = app.ext::<ChartUi>().and_then(|ui| ui.keys.clone()) {
            window.focus(&keys);
        }
        // Titles, the legend and the plot area can be dragged anywhere in
        // the chart; a data label once the labels of its series are
        // selected. A click on a selected title types in it instead: which
        // of the two it is shows when the button is released.
        let held = target.parent();
        let free = app
            .wb
            .chart(app.sheet, hit.chart)
            .is_some_and(|chart| draggable(chart, held));
        let ready = !matches!(held, ChartElement::DataLabels(_)) || current == target;
        if free && ready && begin_drag(app, hit.chart, &hit.layout, held, None, ev, hit.origin) {
            if let Some(drag) = app.ext_mut::<ChartUi>().drag.as_mut() {
                drag.types = current == target
                    && matches!(target, ChartElement::Title | ChartElement::AxisTitle(_));
            }
            return ZoneResponse::Capture;
        }
        ZoneResponse::Handled
    }

    fn zone_drag(
        &self,
        app: &mut ExcelApp,
        _zone: &Zone,
        ev: &ZoneEvent,
        _window: &mut Window,
        cx: &mut Cx,
    ) {
        drag_element(app, ev, cx);
    }

    /// One drag is one step to undo.
    fn zone_mouse_up(
        &self,
        app: &mut ExcelApp,
        _zone: &Zone,
        _ev: &ZoneEvent,
        window: &mut Window,
        cx: &mut Cx,
    ) {
        let Some(drag) = app.ext_mut::<ChartUi>().drag.take() else {
            return;
        };
        if drag.moved && !drag.refused {
            let label = if drag.grip.is_some() {
                t("Size", "Taille")
            } else {
                t("Move", "Déplacer")
            };
            app.commit(label, cx);
        } else if drag.types && !drag.moved {
            begin_text_edit(app, drag.chart, drag.element, window, cx);
        }
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        let MenuTarget::Object(id) = target else {
            return;
        };
        if id.owner != super::index_of(self) {
            return;
        }
        if let Some(chart) = app.wb.chart(app.sheet, id.id) {
            elements::context_menu(app, chart, app.chart_element(), menu);
        }
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        app.wb.sheets[sheet]
            .charts
            .iter()
            .filter(|chart| !chart.hidden)
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

    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, rect: (f32, f32, f32, f32)) {
        let Some(chart) = p.app.wb.chart(p.app.sheet, object.id) else {
            return;
        };
        let style = p.app.chart_style();
        let data = p.app.wb.chart_data(p.app.sheet, chart);
        let (layout, items) = chart_view::build_chart(chart, &data, p.layout, rect, p.ts, &style);
        p.extend(items);
        if p.app.object_sel == Some(object_id(self, object.id)) {
            let element = p.app.chart_element();
            if element != ChartElement::ChartArea {
                let origin = p.layout.to_window(rect.0, rect.1);
                p.extend(chart_view::outline_draws(
                    &layout.outline(element),
                    origin,
                    p.ts,
                ));
            }
            paint_float_buttons(p, rect);
        }
        let selected = p.app.object_sel == Some(object_id(self, object.id));
        let element = p.app.chart_element();
        let layout = Rc::new(layout);
        // The handles on the frame stay those of the object.
        p.zone(
            (rect.0 + 6.0, rect.1 + 6.0, rect.2 - 6.0, rect.3 - 6.0),
            ChartZone {
                chart: object.id,
                origin: (rect.0, rect.1),
                layout: layout.clone(),
            },
        );
        // A PivotChart shows the fields of its PivotTable as buttons,
        // which lie over the chart.
        super::pivot::paint_chart_buttons(p, chart, rect);
        // The plot area and the legend are resized by their handles.
        let sizable = match element {
            ChartElement::Legend => true,
            ChartElement::PlotArea => draggable(chart, element),
            _ => false,
        };
        if let (true, true, Some(b)) = (selected, sizable, layout.bounds(element)) {
            let (mx, my) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let grips = [
                (b[0], b[1]),
                (b[2], b[1]),
                (b[2], b[3]),
                (b[0], b[3]),
                (mx, b[1]),
                (b[2], my),
                (mx, b[3]),
                (b[0], my),
            ];
            for (grip, (x, y)) in grips.into_iter().enumerate() {
                let cursor = match grip {
                    0 | 2 => CursorStyle::ResizeUpLeftDownRight,
                    1 | 3 => CursorStyle::ResizeUpRightDownLeft,
                    4 | 6 => CursorStyle::ResizeUpDown,
                    _ => CursorStyle::ResizeLeftRight,
                };
                let (x, y) = (rect.0 + x, rect.1 + y);
                p.zone(
                    (x - 5.0, y - 5.0, x + 5.0, y + 5.0),
                    GripZone {
                        chart: object.id,
                        element,
                        grip: grip as u8,
                        origin: (rect.0, rect.1),
                        layout: layout.clone(),
                    },
                )
                .cursor(cursor);
            }
        }
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

    fn paste_object(
        &self,
        app: &mut ExcelApp,
        data: &dyn Any,
        at: ((u32, u32), (f32, f32)),
    ) -> Option<u32> {
        let mut chart = data.downcast_ref::<Chart>()?.clone();
        chart.anchor = at.0;
        chart.offset = at.1;
        app.wb.add_chart(app.sheet, chart)
    }

    /// Charts share the stacking order of pictures and shapes.
    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        let order = match mv {
            ZMove::ToFront => ZOrder::BringToFront,
            ZMove::Forward => ZOrder::BringForward,
            ZMove::Backward => ZOrder::SendBackward,
            ZMove::ToBack => ZOrder::SendToBack,
        };
        app.wb
            .reorder_objects(app.sheet, &[ObjectRef::Chart(id)], order);
    }

    /// Font, size, bold, italics, underline and font colour of the Home
    /// tab go to the text of the selected element; with the chart area
    /// selected, to every text of the chart. Nothing of the Home tab
    /// formats cells while a chart is selected.
    fn object_style(
        &self,
        app: &mut ExcelApp,
        id: u32,
        label: &str,
        change: &dyn Fn(&mut Style),
        cx: &mut Cx,
    ) -> bool {
        let element = app.chart_element();
        let Some(mut chart) = app.wb.chart(app.sheet, id).cloned() else {
            return false;
        };
        if restyle(&mut chart, element, change) && app.wb.chart(app.sheet, id) != Some(&chart) {
            app.edit_chart(label, cx, move |c| *c = chart);
        }
        true
    }

    /// A double click on the chart itself opens "Format Chart Area".
    fn object_double_click(
        &self,
        app: &mut ExcelApp,
        _id: u32,
        _ev: &ZoneEvent,
        window: &mut Window,
        cx: &mut Cx,
    ) {
        chart_format::open_pane(app, window, cx);
    }

    fn object_selected(&self, app: &mut ExcelApp, id: Option<u32>, cx: &mut Cx) {
        if id.is_none() {
            if app.ext::<ChartUi>().is_some_and(|ui| ui.edit.is_some()) {
                finish_text_edit(app, cx);
            }
            app.ext_mut::<ChartUi>().element = ChartElement::ChartArea;
        }
        chart_format::selection_changed(app, cx);
    }
}

/// The "+" and the brush beside a selected chart: Chart Elements and Chart
/// Styles. They stand right of the chart, left of it when the window ends
/// there.
fn paint_float_buttons(p: &mut Painter, rect: (f32, f32, f32, f32)) {
    const SIZE: f32 = 24.0;
    let x = if rect.2 + 10.0 + SIZE > p.clip.2 {
        rect.0 - 10.0 - SIZE
    } else {
        rect.2 + 10.0
    };
    let ink = c(theme::ACCENT);
    for (i, (button, tip)) in [
        (
            FloatButton::Elements,
            t("Chart Elements", "Éléments de graphique"),
        ),
        (
            FloatButton::Styles,
            t("Chart Styles", "Styles du graphique"),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let y = rect.1 + i as f32 * (SIZE + 6.0);
        if x < p.clip.0 || y + SIZE > rect.3 {
            continue;
        }
        p.rounded(
            x,
            y,
            x + SIZE,
            y + SIZE,
            4.0,
            c(0xFFFFFF),
            Some(c(theme::BORDER_STRONG)),
        );
        let (mx, my) = (x + SIZE / 2.0, y + SIZE / 2.0);
        match button {
            FloatButton::Elements => {
                p.rect(mx - 6.0, my - 1.0, mx + 6.0, my + 1.0, ink);
                p.rect(mx - 1.0, my - 6.0, mx + 1.0, my + 6.0, ink);
            }
            FloatButton::Styles => {
                // A brush: the handle, then the tuft.
                p.polyline(&[(mx + 6.0, my - 6.5), (mx - 0.5, my + 0.5)], 2.5, ink);
                p.polygon(
                    &[
                        (mx - 0.5, my - 1.5),
                        (mx + 2.0, my + 1.0),
                        (mx - 3.0, my + 5.5),
                        (mx - 6.5, my + 6.5),
                        (mx - 5.0, my + 3.0),
                    ],
                    ink,
                );
            }
        }
        p.zone((x, y, x + SIZE, y + SIZE), button)
            .cursor(CursorStyle::PointingHand)
            .tooltip(tip);
    }
}
