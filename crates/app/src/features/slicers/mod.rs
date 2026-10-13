//! Slicers and timelines: floating objects whose buttons filter a
//! PivotTable or a table.
//!
//! * `logic.rs`: where the parts of a slicer and of a timeline are, what a
//!   click selects, the colours of the styles — everything that is decided
//!   without a window.
//! * `paint.rs`: how they are drawn.
//! * `dialogs.rs`: Insert Slicers, Insert Timelines, Slicer Settings,
//!   Report Connections and Filter Connections.
//! * `tabs.rs`: the "Slicer" and "Timeline" tabs and their style
//!   galleries.
//!
//! Both kinds are objects of this one feature: a timeline is told from a
//! slicer by the high bit of its object id (see [`TIMELINE`]). Moving,
//! resizing, copying, deleting and stacking come from `objects.rs`; the
//! buttons are zones registered while the object is painted. A selection
//! is applied when the button is released, through the commands of the
//! engine (`slicer_select`, `timeline_select`), as one undo step.

mod dialogs;
mod logic;
mod paint;
mod tabs;

#[cfg(test)]
mod ui_tests;

use super::{
    Command, ContextMenu, ContextTab, Cx, Feature, Intent, MenuSection, MenuTarget, ObjectInfo, ObjectVerb, Painter,
    Placement, ZMove, Zone, ZoneEvent, ZoneResponse, object_id, pivot,
};
use crate::app::{ExcelApp, MenuEntry, Popup, PopupKind};
use crate::i18n::t;
use gpui::{AnyElement, MouseButton, Window};
use logic::{Pick, SlicerLayout, TimelineLayout};
use opensheet_engine::slicer::{Period, Slicer, SlicerItem, SlicerSource, TimeLevel, Timeline};
use std::any::Any;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub struct Slicers;

pub(crate) const SLICER_TAB: &str = "slicer";
pub(crate) const TIMELINE_TAB: &str = "timeline";

/// The bit that tells the object id of a timeline from that of a slicer.
pub(crate) const TIMELINE: u32 = 1 << 31;

commands! {
    /// What the user can ask of slicers and timelines.
    pub enum Cmd {
        InsertSlicer = "slicer", t("Slicer", "Segment");
        InsertTimeline = "clock", t("Timeline", "Chronologie");
        FilterConnections = "link", t("Filter Connections", "Connexions de filtre");
        ReportConnections = "link", t("Report Connections", "Connexions de rapport");
        SlicerSettings = "format-cells", t("Slicer Settings", "Paramètres des segments");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
        MultiSelect = "check", t("Multi-Select", "Sélection multiple");
        // The check boxes of the Timeline tab.
        ShowHeader = "", t("Header", "En-tête");
        ShowScrollbar = "", t("Scrollbar", "Barre de défilement");
        ShowSelectionLabel = "", t("Selection Label", "Étiquette de sélection");
        ShowTimeLevel = "", t("Time Level", "Niveau temporel");
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
        let (row, col) = app.active();
        // In a PivotTable, or with one of its PivotCharts selected.
        let in_pivot = pivot::target_pivot(app).is_some();
        match self {
            Cmd::InsertSlicer => in_pivot || app.wb.table_at(app.sheet, row, col).is_some(),
            Cmd::InsertTimeline | Cmd::FilterConnections => in_pivot,
            Cmd::ReportConnections => {
                selected_timeline(app).is_some()
                    || selected_slicer(app).is_some_and(|s| matches!(s.source, SlicerSource::Pivot { .. }))
            }
            Cmd::SlicerSettings | Cmd::MultiSelect => selected_slicer(app).is_some(),
            Cmd::Styles => selected_slicer(app).is_some() || selected_timeline(app).is_some(),
            Cmd::ShowHeader | Cmd::ShowScrollbar | Cmd::ShowSelectionLabel | Cmd::ShowTimeLevel => {
                selected_timeline(app).is_some()
            }
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let timeline = selected_timeline(app);
        match self {
            Cmd::MultiSelect => selected_slicer(app).is_some_and(|s| multi_select(app, s.id)),
            Cmd::ShowHeader => timeline.is_some_and(|t| t.show_header),
            Cmd::ShowScrollbar => timeline.is_some_and(|t| t.show_scrollbar),
            Cmd::ShowSelectionLabel => timeline.is_some_and(|t| t.show_selection_label),
            Cmd::ShowTimeLevel => timeline.is_some_and(|t| t.show_time_level),
            _ => false,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::InsertSlicer => dialogs::open_insert_slicers(app, window, cx),
            Cmd::InsertTimeline => dialogs::open_insert_timelines(app, window, cx),
            Cmd::FilterConnections => dialogs::open_filter_connections(app, window, cx),
            Cmd::ReportConnections => dialogs::open_report_connections(app, window, cx),
            Cmd::SlicerSettings => dialogs::open_settings(app, window, cx),
            Cmd::Styles => tabs::open_gallery(app, window, cx),
            Cmd::MultiSelect => {
                if let Some(id) = selected_slicer(app).map(|s| s.id) {
                    let multi = &mut app.ext_mut::<SlicerUi>().multi;
                    if !multi.remove(&id) {
                        multi.insert(id);
                    }
                    cx.notify();
                }
            }
            Cmd::ShowHeader => edit_timeline(app, self.label(), cx, |t| t.show_header = !t.show_header),
            Cmd::ShowScrollbar => edit_timeline(app, self.label(), cx, |t| t.show_scrollbar = !t.show_scrollbar),
            Cmd::ShowSelectionLabel => {
                edit_timeline(app, self.label(), cx, |t| t.show_selection_label = !t.show_selection_label)
            }
            Cmd::ShowTimeLevel => edit_timeline(app, self.label(), cx, |t| t.show_time_level = !t.show_time_level),
        }
    }
}

// -------------------------------------------------------------------- state

/// A press on a part of a slicer or of a timeline that is still held.
#[derive(Clone, Debug, PartialEq)]
enum Press {
    /// On a button of a slicer: the item pressed, and how.
    Buttons { slicer: u32, from: usize, pick: Pick },
    /// On the thumb of a slicer's scroll bar: where, and the first row
    /// shown then.
    Thumb { slicer: u32, y: f32, row: u32 },
    /// On a period of a timeline: the selection runs from this period to
    /// the one under the pointer. A handle of the selected span works the
    /// same way from its other end.
    Periods { timeline: u32, from: usize },
    /// On the thumb of a timeline's scroll bar.
    BandThumb { timeline: u32, x: f32, scroll: f32 },
}

/// The items of a slicer as last worked out, with the state of the
/// workbook they were worked out from.
type ItemCache = RefCell<HashMap<u32, (u64, Rc<Vec<SlicerItem>>)>>;

/// What the slicers remember between events.
#[derive(Default)]
pub(crate) struct SlicerUi {
    /// The slicers whose "Multi-Select" button is on.
    multi: HashSet<u32>,
    /// The first row of buttons each scrolled slicer shows.
    first_row: HashMap<u32, u32>,
    /// How far the band of each scrolled timeline is scrolled.
    band_scroll: HashMap<u32, f32>,
    press: Option<Press>,
    /// What the press in progress selects: shown at once, applied when the
    /// button is released.
    pending_items: Option<(u32, Vec<u32>)>,
    pending_span: Option<(u32, usize, usize)>,
    /// The item last clicked in each slicer, where Shift-click starts.
    last_item: HashMap<u32, usize>,
    items: ItemCache,
    tabs: tabs::TabState,
}

/// What the zone over a slicer carries: the slicer as it was last painted.
pub(super) struct SlicerZone {
    pub id: u32,
    /// Corner of the slicer in grid-element coordinates, and the zoom.
    pub origin: (f32, f32),
    pub zoom: f32,
    pub layout: SlicerLayout,
    pub items: Rc<Vec<SlicerItem>>,
}

/// The same for a timeline.
pub(super) struct TimelineZone {
    pub id: u32,
    pub origin: (f32, f32),
    pub zoom: f32,
    pub layout: TimelineLayout,
    pub periods: Rc<Vec<Period>>,
    /// The periods the selection covers.
    pub selected: Option<(usize, usize)>,
}

// ------------------------------------------------------------------ helpers

/// The selected slicer of the current sheet.
pub(crate) fn selected_slicer(app: &ExcelApp) -> Option<&Slicer> {
    let id = app.object_sel.filter(|id| id.owner == super::index_of(&Slicers) && id.id & TIMELINE == 0)?;
    app.wb.sheets[app.sheet].slicers.iter().find(|s| s.id == id.id)
}

/// The selected timeline of the current sheet.
pub(crate) fn selected_timeline(app: &ExcelApp) -> Option<&Timeline> {
    let id = app.object_sel.filter(|id| id.owner == super::index_of(&Slicers) && id.id & TIMELINE != 0)?;
    app.wb.sheets[app.sheet].timelines.iter().find(|t| t.id == id.id & !TIMELINE)
}

fn multi_select(app: &ExcelApp, id: u32) -> bool {
    app.ext::<SlicerUi>().is_some_and(|ui| ui.multi.contains(&id))
}

/// The buttons of a slicer, worked out once per state of the workbook.
fn items_of(app: &ExcelApp, id: u32) -> Rc<Vec<SlicerItem>> {
    let generation = app.wb.generation;
    let Some(ui) = app.ext::<SlicerUi>() else {
        return Rc::new(app.wb.slicer_items(id));
    };
    if let Some((at, items)) = ui.items.borrow().get(&id)
        && *at == generation
    {
        return items.clone();
    }
    let items = Rc::new(app.wb.slicer_items(id));
    ui.items.borrow_mut().insert(id, (generation, items.clone()));
    items
}

/// The first row of buttons a slicer shows.
fn first_row(app: &ExcelApp, slicer: &Slicer) -> u32 {
    app.ext::<SlicerUi>()
        .and_then(|ui| ui.first_row.get(&slicer.id).copied())
        .unwrap_or(slicer.start_item / slicer.columns.max(1))
}

/// Change the selected slicer, as one undo step.
pub(crate) fn edit_slicer(app: &mut ExcelApp, label: &str, cx: &mut Cx, f: impl FnOnce(&mut Slicer)) {
    let Some(id) = selected_slicer(app).map(|s| s.id) else {
        return;
    };
    if app.may(Intent::EditObject(Some(object_id(&Slicers, id))), cx) {
        app.wb.update_slicer(id, f);
        app.commit(label, cx);
    }
}

/// Change the selected timeline, as one undo step.
pub(crate) fn edit_timeline(app: &mut ExcelApp, label: &str, cx: &mut Cx, f: impl FnOnce(&mut Timeline)) {
    let Some(id) = selected_timeline(app).map(|t| t.id) else {
        return;
    };
    if app.may(Intent::EditObject(Some(object_id(&Slicers, id | TIMELINE))), cx) {
        app.wb.update_timeline(id, f);
        app.commit(label, cx);
    }
}

/// What a protected sheet is asked before a slicer filters.
fn filter_intent(app: &ExcelApp, id: u32) -> Intent {
    match app.wb.slicer(id).map(|(_, s)| &s.source) {
        Some(SlicerSource::Table { .. }) => Intent::UseAutoFilter,
        _ => Intent::UsePivotTable,
    }
}

/// Show exactly these items in what a slicer filters (none: all of them).
pub(crate) fn select_items(app: &mut ExcelApp, id: u32, selected: Vec<u32>, window: &mut Window, cx: &mut Cx) {
    if !app.may(filter_intent(app, id), cx) {
        return;
    }
    let label = if selected.is_empty() {
        t("Clear Filter", "Effacer le filtre")
    } else {
        t("Slicer Selection", "Sélection de segment")
    };
    pivot::run_op(app, label, window, cx, move |app, overwrite| app.wb.slicer_select(id, &selected, overwrite));
}

/// Limit the field of a timeline to these days (`None`: all periods).
pub(crate) fn select_days(app: &mut ExcelApp, id: u32, days: Option<(f64, f64)>, window: &mut Window, cx: &mut Cx) {
    if !app.may(Intent::UsePivotTable, cx) {
        return;
    }
    let label = if days.is_none() {
        t("Clear Filter", "Effacer le filtre")
    } else {
        t("Timeline Selection", "Sélection de chronologie")
    };
    pivot::run_op(app, label, window, cx, move |app, overwrite| app.wb.timeline_select(id, days, overwrite));
}

/// The level a timeline shows, as one undo step.
fn set_level(app: &mut ExcelApp, id: u32, level: TimeLevel, cx: &mut Cx) {
    if app.may(Intent::EditObject(Some(object_id(&Slicers, id | TIMELINE))), cx) {
        app.wb.update_timeline(id, |t| t.level = level);
        // The band starts over where the selection is.
        app.ext_mut::<SlicerUi>().band_scroll.remove(&id);
        app.commit(t("Time Level", "Niveau temporel"), cx);
    }
}

/// Where the next new slicer or timeline goes: the middle of the window,
/// each one a little below and right of the one before.
fn cascade(app: &ExcelApp, size: (f32, f32), rank: usize) -> ((u32, u32), (f32, f32)) {
    let place = app.place_new_object(size);
    let step = 24.0 * rank as f32;
    (place.anchor, (place.offset.0 + step, place.offset.1 + step))
}

/// Put slicers for these fields of a PivotTable (or columns of a table)
/// on the current sheet, as one undo step.
pub(crate) fn insert_slicers(app: &mut ExcelApp, source: dialogs::Source, fields: &[usize], cx: &mut Cx) {
    if fields.is_empty() || !app.may(Intent::EditObject(None), cx) {
        return;
    }
    let sheet = app.sheet;
    let mut last = None;
    for (rank, field) in fields.iter().enumerate() {
        let (anchor, offset) = cascade(app, opensheet_engine::slicer::SLICER_SIZE, rank);
        let id = match source {
            dialogs::Source::Pivot(pivot) => app.wb.add_pivot_slicer(sheet, pivot, *field, anchor),
            dialogs::Source::Table(table) => app.wb.add_table_slicer(sheet, table, *field, anchor),
        };
        if let Some(id) = id {
            app.wb.update_slicer(id, |s| s.offset = offset);
            last = Some(id);
        }
    }
    app.commit(t("Insert Slicer", "Insérer un segment"), cx);
    if let Some(id) = last {
        app.select_object(Some(object_id(&Slicers, id)), cx);
        app.context_tab = Some(SLICER_TAB);
    }
}

/// Put timelines for these date fields of a PivotTable on the sheet.
pub(crate) fn insert_timelines(app: &mut ExcelApp, pivot: u32, fields: &[usize], cx: &mut Cx) {
    if fields.is_empty() || !app.may(Intent::EditObject(None), cx) {
        return;
    }
    let sheet = app.sheet;
    let mut last = None;
    for (rank, field) in fields.iter().enumerate() {
        let (anchor, offset) = cascade(app, opensheet_engine::slicer::TIMELINE_SIZE, rank);
        if let Ok(id) = app.wb.add_timeline(sheet, pivot, *field, anchor) {
            app.wb.update_timeline(id, |t| t.offset = offset);
            last = Some(id);
        }
    }
    app.commit(t("Insert Timeline", "Insérer une chronologie"), cx);
    if let Some(id) = last {
        app.select_object(Some(object_id(&Slicers, id | TIMELINE)), cx);
        app.context_tab = Some(TIMELINE_TAB);
    }
}

/// The slicer as it was last painted, when it is on screen.
#[cfg(test)]
pub(super) fn painted_slicer(app: &ExcelApp, id: u32) -> Option<&SlicerZone> {
    app.zones.iter().find_map(|zone| zone.payload::<SlicerZone>().filter(|z| z.id == id))
}

#[cfg(test)]
pub(super) fn painted_timeline(app: &ExcelApp, id: u32) -> Option<&TimelineZone> {
    app.zones.iter().find_map(|zone| zone.payload::<TimelineZone>().filter(|z| z.id == id))
}

// ------------------------------------------------------------------- presses

/// A press on a slicer. `None`: on nothing of its own (the slicer is then
/// selected and moved like any object).
fn press_slicer(app: &mut ExcelApp, hit: &SlicerZone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> Option<ZoneResponse> {
    let (x, y) = ((ev.x - hit.origin.0) / hit.zoom, (ev.y - hit.origin.1) / hit.zoom);
    let id = hit.id;
    let object = object_id(&Slicers, id);
    if hit.layout.clear.is_some_and(|b| logic::inside(b, x, y)) {
        app.select_object(Some(object), cx);
        if app.wb.slicer_filtered(id) {
            select_items(app, id, Vec::new(), window, cx);
        }
        return Some(ZoneResponse::Handled);
    }
    if hit.layout.multi.is_some_and(|b| logic::inside(b, x, y)) {
        app.select_object(Some(object), cx);
        Cmd::MultiSelect.run(app, window, cx);
        return Some(ZoneResponse::Handled);
    }
    if let Some((track, thumb)) = hit.layout.scroll
        && logic::inside(track, x, y)
    {
        app.select_object(Some(object), cx);
        let row = hit.layout.first_row;
        if logic::inside(thumb, x, y) {
            app.ext_mut::<SlicerUi>().press = Some(Press::Thumb { slicer: id, y, row });
            return Some(ZoneResponse::Capture);
        }
        // A click beside the thumb turns a page.
        let page = hit.layout.rows_shown.max(1);
        let hidden = hit.layout.rows.saturating_sub(hit.layout.rows_shown);
        let to = if y < thumb.1 { row.saturating_sub(page) } else { (row + page).min(hidden) };
        app.ext_mut::<SlicerUi>().first_row.insert(id, to);
        return Some(ZoneResponse::Handled);
    }
    let index = hit.layout.button_at(x, y)?;
    app.select_object(Some(object), cx);
    let toggling = ev.modifiers.platform || ev.modifiers.control || multi_select(app, id);
    let anchor = app.ext::<SlicerUi>().and_then(|ui| ui.last_item.get(&id).copied());
    let (from, pick) = match (ev.modifiers.shift, anchor) {
        (true, Some(anchor)) => (anchor, Pick::Through(anchor)),
        _ if toggling => (index, Pick::Toggle),
        _ => (index, Pick::Through(index)),
    };
    let ui = app.ext_mut::<SlicerUi>();
    ui.pending_items = Some((id, logic::picked(&hit.items, index, pick)));
    ui.press = Some(Press::Buttons { slicer: id, from, pick });
    ui.last_item.insert(id, if ev.modifiers.shift { from } else { index });
    Some(ZoneResponse::Capture)
}

fn press_timeline(app: &mut ExcelApp, hit: &TimelineZone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> Option<ZoneResponse> {
    let (x, y) = ((ev.x - hit.origin.0) / hit.zoom, (ev.y - hit.origin.1) / hit.zoom);
    let id = hit.id;
    let object = object_id(&Slicers, id | TIMELINE);
    let layout = &hit.layout;
    if layout.clear.is_some_and(|b| logic::inside(b, x, y)) {
        app.select_object(Some(object), cx);
        if app.wb.timeline_selection(id).is_some() {
            select_days(app, id, None, window, cx);
        }
        return Some(ZoneResponse::Handled);
    }
    if layout.level.is_some_and(|b| logic::inside(b, x, y)) {
        app.select_object(Some(object), cx);
        let current = app.wb.timeline(id).map(|(_, t)| t.level);
        let locale = app.wb.locale;
        let entries: Vec<MenuEntry> = TimeLevel::ALL
            .into_iter()
            .map(|level| {
                MenuEntry::new(level.label(locale), move |app, _, cx| set_level(app, id, level, cx))
                    .checked(current == Some(level))
            })
            .collect();
        app.popup = Some(Popup::new(PopupKind::Menu(entries), ev.position, 150.0));
        return Some(ZoneResponse::Handled);
    }
    if let Some((track, thumb)) = layout.scrollbar
        && logic::inside(track, x, y)
    {
        app.select_object(Some(object), cx);
        if logic::inside(thumb, x, y) {
            app.ext_mut::<SlicerUi>().press = Some(Press::BandThumb {
                timeline: id,
                x,
                scroll: layout.scroll,
            });
            return Some(ZoneResponse::Capture);
        }
        let page = (layout.band.2 - layout.band.0) * if x < thumb.0 { -1.0 } else { 1.0 };
        let to = (layout.scroll + page).clamp(0.0, layout.max_scroll);
        app.ext_mut::<SlicerUi>().band_scroll.insert(id, to);
        return Some(ZoneResponse::Handled);
    }
    let on_band = logic::inside((layout.band.0, layout.names_y, layout.band.2, layout.bar.1 + 6.0), x, y);
    if !on_band || hit.periods.is_empty() {
        return None;
    }
    app.select_object(Some(object), cx);
    let index = layout.period_at(x, hit.periods.len());
    // A handle of the selected span moves that end: the selection then
    // runs from its other end.
    let on_bar = y >= layout.bar.0 - 4.0;
    let handle = hit.selected.filter(|_| on_bar).and_then(|(first, last)| {
        let (left, right) = (layout.period_x(first), layout.period_x(last + 1));
        if (x - left).abs() <= 6.0 {
            Some(last)
        } else if (x - right).abs() <= 6.0 {
            Some(first)
        } else {
            None
        }
    });
    let from = match (handle, ev.modifiers.shift, hit.selected) {
        (Some(fixed), ..) => fixed,
        (None, true, Some((first, _))) => first,
        _ => index,
    };
    let ui = app.ext_mut::<SlicerUi>();
    ui.press = Some(Press::Periods { timeline: id, from });
    ui.pending_span = Some((id, from.min(index), from.max(index)));
    Some(ZoneResponse::Capture)
}

// ------------------------------------------------------------------ feature

impl Feature for Slicers {
    fn name(&self) -> &'static str {
        "slicers"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        if selected_slicer(app).is_some() {
            vec![ContextTab {
                id: SLICER_TAB,
                title: t("Slicer", "Segment"),
            }]
        } else if selected_timeline(app).is_some() {
            vec![ContextTab {
                id: TIMELINE_TAB,
                title: t("Timeline", "Chronologie"),
            }]
        } else {
            Vec::new()
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, tab: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        if tab == TIMELINE_TAB { tabs::timeline_tab(app, window, cx) } else { tabs::slicer_tab(app, window, cx) }
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        // The other buttons select and move the object and open its menu.
        if ev.button != MouseButton::Left {
            return ZoneResponse::Ignored;
        }
        let handled = if let Some(hit) = zone.payload::<SlicerZone>() {
            if app.editing.is_some() && !app.commit_edit(None, window, cx) {
                return ZoneResponse::Handled;
            }
            press_slicer(app, hit, ev, window, cx)
        } else if let Some(hit) = zone.payload::<TimelineZone>() {
            if app.editing.is_some() && !app.commit_edit(None, window, cx) {
                return ZoneResponse::Handled;
            }
            press_timeline(app, hit, ev, window, cx)
        } else {
            None
        };
        if handled.is_some() {
            cx.notify();
        }
        handled.unwrap_or(ZoneResponse::Ignored)
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        let Some(press) = app.ext::<SlicerUi>().and_then(|ui| ui.press.clone()) else {
            return;
        };
        if let Some(hit) = zone.payload::<SlicerZone>() {
            let (x, y) = ((ev.x - hit.origin.0) / hit.zoom, (ev.y - hit.origin.1) / hit.zoom);
            match press {
                Press::Buttons { slicer, from, pick: Pick::Through(_) } => {
                    // The drag selects from the button pressed to the one
                    // under the pointer; off the buttons it keeps the last.
                    if let Some(index) = hit.layout.button_at(x, y) {
                        let selected = logic::picked(&hit.items, index, Pick::Through(from));
                        app.ext_mut::<SlicerUi>().pending_items = Some((slicer, selected));
                    }
                }
                Press::Thumb { slicer, y: start, row } => {
                    let to = hit.layout.scrolled(row, y - start);
                    app.ext_mut::<SlicerUi>().first_row.insert(slicer, to);
                }
                _ => {}
            }
        } else if let Some(hit) = zone.payload::<TimelineZone>() {
            let x = (ev.x - hit.origin.0) / hit.zoom;
            match press {
                Press::Periods { timeline, from } => {
                    let index = hit.layout.period_at(x, hit.periods.len());
                    app.ext_mut::<SlicerUi>().pending_span = Some((timeline, from.min(index), from.max(index)));
                }
                Press::BandThumb { timeline, x: start, scroll } => {
                    let to = hit.layout.scrolled(scroll, x - start);
                    app.ext_mut::<SlicerUi>().band_scroll.insert(timeline, to);
                }
                _ => {}
            }
        }
        cx.notify();
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, _ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        let ui = app.ext_mut::<SlicerUi>();
        ui.press = None;
        let items = ui.pending_items.take();
        let span = ui.pending_span.take();
        if let Some((id, selected)) = items {
            // Nothing to do when the buttons already show this.
            let now: Vec<u32> = match zone.payload::<SlicerZone>() {
                Some(hit) if hit.items.iter().any(|i| !i.selected) => {
                    hit.items.iter().filter(|i| i.selected).map(|i| i.x).collect()
                }
                _ => Vec::new(),
            };
            let same = now.len() == selected.len() && now.iter().all(|x| selected.contains(x));
            if !same {
                select_items(app, id, selected, window, cx);
            }
        }
        if let (Some((id, first, last)), Some(hit)) = (span, zone.payload::<TimelineZone>()) {
            let days = logic::span_days(&hit.periods, first, last);
            if days.is_some() && days != app.wb.timeline_selection(id) {
                select_days(app, id, days, window, cx);
            }
        }
        cx.notify();
    }

    fn zone_scroll(&self, app: &mut ExcelApp, zone: &Zone, delta: (f32, f32), _cx: &mut Cx) -> bool {
        if let Some(hit) = zone.payload::<SlicerZone>() {
            if hit.layout.scroll.is_none() {
                return false;
            }
            let hidden = hit.layout.rows.saturating_sub(hit.layout.rows_shown);
            let rows = if delta.1 > 0.0 { -1 } else if delta.1 < 0.0 { 1 } else { 0 };
            let to = (hit.layout.first_row as i64 + rows).clamp(0, hidden as i64) as u32;
            app.ext_mut::<SlicerUi>().first_row.insert(hit.id, to);
            return true;
        }
        if let Some(hit) = zone.payload::<TimelineZone>() {
            if hit.layout.max_scroll <= 0.0 {
                return false;
            }
            // Either wheel moves the band.
            let moved = if delta.0.abs() > delta.1.abs() { delta.0 } else { delta.1 };
            let to = (hit.layout.scroll - moved).clamp(0.0, hit.layout.max_scroll);
            app.ext_mut::<SlicerUi>().band_scroll.insert(hit.id, to);
            return true;
        }
        false
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        let MenuTarget::Object(id) = target else {
            return;
        };
        if id.owner != super::index_of(self) {
            return;
        }
        for entry in object_menu(app, id.id) {
            menu.add(MenuSection::Data, entry);
        }
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        let sh = &app.wb.sheets[sheet];
        let slicers = sh.slicers.iter().map(|s| ObjectInfo {
            id: s.id,
            place: Placement {
                anchor: s.anchor,
                offset: s.offset,
                size: s.size,
            },
            min_size: (60.0, 50.0),
            keep_aspect: false,
        });
        let timelines = sh.timelines.iter().map(|t| ObjectInfo {
            id: t.id | TIMELINE,
            place: Placement {
                anchor: t.anchor,
                offset: t.offset,
                size: t.size,
            },
            min_size: (150.0, 70.0),
            keep_aspect: false,
        });
        slicers.chain(timelines).collect()
    }

    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, rect: (f32, f32, f32, f32)) {
        if object.id & TIMELINE != 0 {
            paint::timeline(p, object.id & !TIMELINE, rect);
        } else {
            paint::slicer(p, object.id, rect);
        }
    }

    fn place_object(&self, app: &mut ExcelApp, id: u32, place: Placement) {
        if id & TIMELINE != 0 {
            app.wb.update_timeline(id & !TIMELINE, |t| {
                t.anchor = place.anchor;
                t.offset = place.offset;
                t.size = place.size;
            });
        } else {
            app.wb.update_slicer(id, |s| {
                s.anchor = place.anchor;
                s.offset = place.offset;
                s.size = place.size;
            });
        }
    }

    fn object_label(&self, id: u32, verb: ObjectVerb) -> &'static str {
        let timeline = id & TIMELINE != 0;
        match (verb, timeline) {
            (ObjectVerb::Move, false) => t("Move Slicer", "Déplacer le segment"),
            (ObjectVerb::Move, true) => t("Move Timeline", "Déplacer la chronologie"),
            (ObjectVerb::Resize, false) => t("Resize Slicer", "Redimensionner le segment"),
            (ObjectVerb::Resize, true) => t("Resize Timeline", "Redimensionner la chronologie"),
            (ObjectVerb::Delete, false) => t("Delete Slicer", "Supprimer le segment"),
            (ObjectVerb::Delete, true) => t("Delete Timeline", "Supprimer la chronologie"),
            (ObjectVerb::Paste, _) => t("Paste", "Coller"),
            (ObjectVerb::Reorder, _) => t("Change Order", "Changer l'ordre"),
        }
    }

    /// The filter stays as the slicer left it, as in Excel.
    fn delete_object(&self, app: &mut ExcelApp, id: u32) {
        if id & TIMELINE != 0 {
            app.wb.remove_timeline(id & !TIMELINE);
        } else {
            app.wb.remove_slicer(id);
        }
    }

    fn copy_object(&self, app: &ExcelApp, id: u32) -> Option<Rc<dyn Any>> {
        if id & TIMELINE != 0 {
            let timeline = app.wb.timeline(id & !TIMELINE)?.1.clone();
            Some(Rc::new(timeline))
        } else {
            let slicer = app.wb.slicer(id)?.1.clone();
            Some(Rc::new(slicer))
        }
    }

    fn paste_object(&self, app: &mut ExcelApp, data: &dyn Any, at: ((u32, u32), (f32, f32))) -> Option<u32> {
        if let Some(slicer) = data.downcast_ref::<Slicer>() {
            let mut slicer = slicer.clone();
            (slicer.anchor, slicer.offset) = at;
            return app.wb.paste_slicer(app.sheet, slicer);
        }
        let mut timeline = data.downcast_ref::<Timeline>()?.clone();
        (timeline.anchor, timeline.offset) = at;
        app.wb.paste_timeline(app.sheet, timeline).map(|id| id | TIMELINE)
    }

    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        fn moved<T>(list: &mut Vec<T>, from: usize, mv: ZMove) {
            let last = list.len() - 1;
            let to = match mv {
                ZMove::ToFront => last,
                ZMove::Forward => (from + 1).min(last),
                ZMove::Backward => from.saturating_sub(1),
                ZMove::ToBack => 0,
            };
            let item = list.remove(from);
            list.insert(to, item);
        }
        let sheet = app.sheet;
        app.wb.snapshot();
        let sh = &mut app.wb.sheets[sheet];
        if id & TIMELINE != 0 {
            if let Some(from) = sh.timelines.iter().position(|t| t.id == id & !TIMELINE) {
                moved(&mut sh.timelines, from, mv);
            }
        } else if let Some(from) = sh.slicers.iter().position(|s| s.id == id) {
            moved(&mut sh.slicers, from, mv);
        }
    }

    /// A double click on the frame or the header of a slicer opens its
    /// settings.
    fn object_double_click(&self, app: &mut ExcelApp, id: u32, _ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        if id & TIMELINE == 0 {
            dialogs::open_settings(app, window, cx);
        }
    }

    fn object_selected(&self, app: &mut ExcelApp, id: Option<u32>, _cx: &mut Cx) {
        if id.is_none() {
            let ui = app.ext_mut::<SlicerUi>();
            ui.press = None;
            ui.pending_items = None;
            ui.pending_span = None;
        }
    }
}

/// The context menu of a slicer or of a timeline.
fn object_menu(app: &ExcelApp, object: u32) -> Vec<MenuEntry> {
    let mut out = Vec::new();
    let quoted = |pattern: &str, name: &str| pattern.replace("{}", name);
    if object & TIMELINE != 0 {
        let Some((_, timeline)) = app.wb.timeline(object & !TIMELINE) else {
            return out;
        };
        let id = timeline.id;
        out.push(
            MenuEntry::new(
                quoted(t("Clear Filter From \"{}\"", "Effacer le filtre de «\u{a0}{}\u{a0}»"), &timeline.caption),
                move |app, window, cx| select_days(app, id, None, window, cx),
            )
            .disabled(app.wb.timeline_selection(id).is_none()),
        );
        out.push(MenuEntry::command(Cmd::ReportConnections, app));
        out.push(MenuEntry::new(
            quoted(t("Remove \"{}\"", "Supprimer «\u{a0}{}\u{a0}»"), &timeline.caption),
            |app, _, cx| app.delete_selected_object(cx),
        ));
        return out;
    }
    let Some((_, slicer)) = app.wb.slicer(object) else {
        return out;
    };
    let id = slicer.id;
    let sort = |descending: bool| {
        move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
            edit_slicer(app, t("Sort", "Trier"), cx, |s| s.descending = descending);
        }
    };
    out.push(MenuEntry::new(t("Sort Ascending", "Tri croissant"), sort(false)).checked(!slicer.descending));
    out.push(MenuEntry::new(t("Sort Descending", "Tri décroissant"), sort(true)).checked(slicer.descending));
    out.push(
        MenuEntry::new(
            quoted(t("Clear Filter From \"{}\"", "Effacer le filtre de «\u{a0}{}\u{a0}»"), &slicer.caption),
            move |app, window, cx| select_items(app, id, Vec::new(), window, cx),
        )
        .disabled(!app.wb.slicer_filtered(id)),
    );
    out.push(
        MenuEntry::new(
            quoted(t("Multi-Select \"{}\"", "Sélection multiple «\u{a0}{}\u{a0}»"), &slicer.caption),
            |app, window, cx| Cmd::MultiSelect.run(app, window, cx),
        )
        .checked(multi_select(app, id)),
    );
    out.push(MenuEntry::command(Cmd::ReportConnections, app));
    out.push(MenuEntry::new(
        quoted(t("Remove \"{}\"", "Supprimer «\u{a0}{}\u{a0}»"), &slicer.caption),
        |app, _, cx| app.delete_selected_object(cx),
    ));
    out.push(MenuEntry::new(t("Slicer Settings...", "Paramètres des segments..."), |app, window, cx| {
        Cmd::SlicerSettings.run(app, window, cx)
    }));
    out
}
