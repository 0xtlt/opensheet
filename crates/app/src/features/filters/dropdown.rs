//! The drop-down of a filter button: the sort entries, the filters by
//! colour and by condition in submenus, the search box, and the list of the
//! column's values with its check boxes.
//!
//! The drop-down is a view shown with `open_popup_view`. Its submenus hang
//! outside that view, so a press on one of their entries first closes the
//! drop-down (a press outside it does) and then runs the entry: entries act
//! on the press itself, through the application, never through this view.

use super::{
    Cmd, State, apply_criteria, blanks_label, clear_filter_label, column_title, custom, dynamic_label, month_name,
    sort, sort_labels, sort_target,
};
use super::{OpenMenu, SortTarget, button_box};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::t;
use crate::text_input::{self, InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{self, Button, ROW_H, ScrollState, TextField, Tri, check_mark, icon, virtual_rows};
use gpui::{
    AnyElement, App, Bounds, Context, Corner, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, Pixels, Point,
    SharedString, WeakEntity, Window, anchored, canvas, deferred, div, point, prelude::*, px,
};
use opensheet_engine::datetime::date_to_serial;
use opensheet_engine::filter::{
    ColorFilter, ColumnKind, Comparison, Criteria, DynamicFilter, FilterButton, FilterDropdown, FilterScope,
};
use opensheet_engine::format;
use opensheet_engine::functions::wildcard_match;
use opensheet_engine::ops::SortOn;
use opensheet_engine::{Color, Locale, Rect};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

// ------------------------------------------------------------------ entries

/// What an entry of the drop-down does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Act {
    Sort { descending: bool },
    /// Put the cells of a fill or font colour on top.
    SortColor(SortOn),
    CustomSort,
    /// "Clear Filter From ...".
    Clear,
    Color(ColorFilter),
    Dynamic(DynamicFilter),
    /// Open the Custom AutoFilter dialog on these comparisons ("Begins
    /// With...", "Between...").
    Custom(Comparison, Option<Comparison>),
    /// "Custom Filter...": the dialog on the column's current conditions.
    CustomDialog,
    Top10,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum EntryKind {
    Separator,
    /// A title inside a submenu ("Sort by Cell Color").
    Heading,
    Act(Act),
    Sub(Vec<Entry>),
}

/// A line of the menu part of the drop-down, or of one of its submenus.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    /// Shown as a bar of this colour instead of its label.
    pub swatch: Option<Color>,
    pub checked: bool,
    pub disabled: bool,
    pub kind: EntryKind,
}

impl Entry {
    fn new(label: impl Into<SharedString>, kind: EntryKind) -> Entry {
        Entry {
            label: label.into(),
            icon: None,
            swatch: None,
            checked: false,
            disabled: false,
            kind,
        }
    }

    fn act(label: impl Into<SharedString>, act: Act) -> Entry {
        Entry::new(label, EntryKind::Act(act))
    }

    fn sub(label: impl Into<SharedString>, entries: Vec<Entry>) -> Entry {
        Entry::new(label, EntryKind::Sub(entries))
    }

    fn separator() -> Entry {
        Entry::new("", EntryKind::Separator)
    }

    fn heading(label: impl Into<SharedString>) -> Entry {
        Entry::new(label, EntryKind::Heading)
    }

    fn icon(mut self, name: &'static str) -> Entry {
        self.icon = Some(name);
        self
    }

    fn checked(mut self, on: bool) -> Entry {
        self.checked = on;
        self
    }

    fn disabled(mut self, on: bool) -> Entry {
        self.disabled = on;
        self
    }

    /// Whether the pointer and the arrow keys stop on it.
    fn selectable(&self) -> bool {
        !self.disabled && matches!(self.kind, EntryKind::Act(_) | EntryKind::Sub(_))
    }
}

/// The colours of a column as entries: a heading, a bar per colour, then
/// "No Fill" / "Automatic". Nothing when the cells all look the same.
fn color_entries(
    heading: &'static str,
    none_label: &'static str,
    colors: &[Option<Color>],
    current: Option<Option<Color>>,
    act: impl Fn(Option<Color>) -> Act,
) -> Vec<Entry> {
    if colors.len() < 2 {
        return Vec::new();
    }
    let mut out = vec![Entry::heading(heading)];
    for color in colors.iter().flatten() {
        let Color(rgb) = *color;
        let name = format!("RGB({}, {}, {})", rgb >> 16 & 0xFF, rgb >> 8 & 0xFF, rgb & 0xFF);
        let mut entry = Entry::act(name, act(Some(*color))).checked(current == Some(Some(*color)));
        entry.swatch = Some(*color);
        out.push(entry);
    }
    if colors.contains(&None) {
        out.push(Entry::act(none_label, act(None)).checked(current == Some(None)));
    }
    out
}

fn sort_color_entries(data: &FilterDropdown) -> Vec<Entry> {
    let mut out = color_entries(
        t("Sort by Cell Color", "Trier par couleur de cellule"),
        t("No Fill", "Aucun remplissage"),
        &data.cell_colors,
        None,
        |color| Act::SortColor(SortOn::CellColor(color)),
    );
    out.extend(color_entries(
        t("Sort by Font Color", "Trier par couleur de police"),
        t("Automatic", "Automatique"),
        &data.font_colors,
        None,
        |color| Act::SortColor(SortOn::FontColor(color)),
    ));
    if !out.is_empty() {
        out.push(Entry::separator());
    }
    out.push(Entry::act(Cmd::CustomSort.label(), Act::CustomSort));
    out
}

fn filter_color_entries(data: &FilterDropdown) -> Vec<Entry> {
    let current = |font: bool| match &data.criteria {
        Some(Criteria::Color(filter)) if filter.font == font => Some(filter.color),
        _ => None,
    };
    let mut out = color_entries(
        t("Filter by Cell Color", "Filtrer par couleur de cellule"),
        t("No Fill", "Aucun remplissage"),
        &data.cell_colors,
        current(false),
        |color| Act::Color(ColorFilter { color, font: false }),
    );
    out.extend(color_entries(
        t("Filter by Font Color", "Filtrer par couleur de police"),
        t("Automatic", "Automatique"),
        &data.font_colors,
        current(true),
        |color| Act::Color(ColorFilter { color, font: true }),
    ));
    out
}

/// The comparisons of the column's custom filter when they are those of an
/// entry of the conditions submenu ("Begins With...", "Between...").
fn custom_shape(criteria: Option<&Criteria>) -> Option<(Comparison, Option<Comparison>)> {
    let Some(Criteria::Custom(custom)) = criteria else {
        return None;
    };
    let first = custom.first.comparison().0;
    match &custom.second {
        None => Some((first, None)),
        Some(second) if custom.and => Some((first, Some(second.comparison().0))),
        Some(_) => None,
    }
}

/// "Text Filters", "Number Filters" or "Date Filters" with its entries.
fn condition_entries(data: &FilterDropdown, locale: &Locale) -> Entry {
    let shape = custom_shape(data.criteria.as_ref());
    let mut listed = false;
    let mut custom = |label: &'static str, first: Comparison, second: Option<Comparison>| {
        let on = shape == Some((first, second));
        listed |= on;
        Entry::act(label, Act::Custom(first, second)).checked(on)
    };
    let dynamic = |filter: DynamicFilter| {
        Entry::act(dynamic_label(filter, locale), Act::Dynamic(filter))
            .checked(data.criteria == Some(Criteria::Dynamic(filter)))
    };
    let equals = t("Equals…", "Est égal à…");
    let not_equal = t("Does Not Equal…", "Est différent de…");
    let between = t("Between…", "Entre…");
    let range = (Comparison::GreaterOrEqual, Some(Comparison::LessOrEqual));
    let (title, mut entries) = match data.kind {
        ColumnKind::Text => (
            t("Text Filters", "Filtres textuels"),
            vec![
                custom(equals, Comparison::Equals, None),
                custom(not_equal, Comparison::NotEquals, None),
                Entry::separator(),
                custom(t("Begins With…", "Commence par…"), Comparison::BeginsWith, None),
                custom(t("Ends With…", "Se termine par…"), Comparison::EndsWith, None),
                Entry::separator(),
                custom(t("Contains…", "Contient…"), Comparison::Contains, None),
                custom(t("Does Not Contain…", "Ne contient pas…"), Comparison::NotContains, None),
            ],
        ),
        ColumnKind::Number => (
            t("Number Filters", "Filtres numériques"),
            vec![
                custom(equals, Comparison::Equals, None),
                custom(not_equal, Comparison::NotEquals, None),
                Entry::separator(),
                custom(t("Greater Than…", "Supérieur à…"), Comparison::Greater, None),
                custom(
                    t("Greater Than Or Equal To…", "Supérieur ou égal à…"),
                    Comparison::GreaterOrEqual,
                    None,
                ),
                custom(t("Less Than…", "Inférieur à…"), Comparison::Less, None),
                custom(
                    t("Less Than Or Equal To…", "Inférieur ou égal à…"),
                    Comparison::LessOrEqual,
                    None,
                ),
                custom(between, range.0, range.1),
                Entry::separator(),
                Entry::act(t("Top 10…", "10 premiers…"), Act::Top10)
                    .checked(matches!(data.criteria, Some(Criteria::Top10(_)))),
                dynamic(DynamicFilter::AboveAverage),
                dynamic(DynamicFilter::BelowAverage),
            ],
        ),
        ColumnKind::Date => {
            let mut period: Vec<Entry> = (1..=4).map(|q| dynamic(DynamicFilter::Quarter(q))).collect();
            period.push(Entry::separator());
            period.extend((1..=12).map(|m| dynamic(DynamicFilter::Month(m))));
            let in_period = matches!(
                data.criteria,
                Some(Criteria::Dynamic(DynamicFilter::Quarter(_) | DynamicFilter::Month(_)))
            );
            let mut entries = vec![
                custom(equals, Comparison::Equals, None),
                Entry::separator(),
                custom(t("Before…", "Avant…"), Comparison::Less, None),
                custom(t("After…", "Après…"), Comparison::Greater, None),
                custom(between, range.0, range.1),
            ];
            let groups = [
                [DynamicFilter::Tomorrow, DynamicFilter::Today, DynamicFilter::Yesterday],
                [DynamicFilter::NextWeek, DynamicFilter::ThisWeek, DynamicFilter::LastWeek],
                [DynamicFilter::NextMonth, DynamicFilter::ThisMonth, DynamicFilter::LastMonth],
                [DynamicFilter::NextQuarter, DynamicFilter::ThisQuarter, DynamicFilter::LastQuarter],
                [DynamicFilter::NextYear, DynamicFilter::ThisYear, DynamicFilter::LastYear],
            ];
            for group in groups {
                entries.push(Entry::separator());
                entries.extend(group.into_iter().map(&dynamic));
            }
            entries.push(Entry::separator());
            entries.push(dynamic(DynamicFilter::YearToDate));
            entries.push(
                Entry::sub(t("All Dates in the Period", "Toutes les dates de cette période"), period).checked(in_period),
            );
            (t("Date Filters", "Filtres chronologiques"), entries)
        }
    };
    entries.push(Entry::separator());
    let other = matches!(data.criteria, Some(Criteria::Custom(_))) && !listed;
    entries.push(Entry::act(t("Custom Filter…", "Filtre personnalisé…"), Act::CustomDialog).checked(other));
    let active = matches!(
        data.criteria,
        Some(Criteria::Custom(_) | Criteria::Top10(_) | Criteria::Dynamic(_))
    );
    Entry::sub(title, entries).checked(active)
}

/// The menu part of the drop-down of a column (`col` is its sheet column).
pub(super) fn entries(data: &FilterDropdown, col: u32, locale: &Locale) -> Vec<Entry> {
    let (ascending, descending) = sort_labels(data.kind);
    let by_color = filter_color_entries(data);
    let no_colors = by_color.is_empty();
    vec![
        Entry::act(ascending, Act::Sort { descending: false })
            .icon("sort-asc")
            .checked(data.sort == Some(false)),
        Entry::act(descending, Act::Sort { descending: true })
            .icon("sort-desc")
            .checked(data.sort == Some(true)),
        Entry::sub(t("Sort by Color", "Trier par couleur"), sort_color_entries(data)),
        Entry::separator(),
        Entry::act(clear_filter_label(&column_title(&data.header, col)), Act::Clear)
            .icon("filters-clear")
            .disabled(!data.is_filtered()),
        Entry::sub(t("Filter by Color", "Filtrer par couleur"), by_color)
            .disabled(no_colors)
            .checked(matches!(data.criteria, Some(Criteria::Color(_)))),
        condition_entries(data, locale),
    ]
}

/// Run an entry of the drop-down of a column.
pub(super) fn perform(
    app: &mut ExcelApp,
    sheet: usize,
    scope: FilterScope,
    col_id: u32,
    act: Act,
    window: &mut Window,
    cx: &mut Cx,
) {
    let Some(range) = app.wb.auto_filter(sheet, scope).map(|filter| filter.range) else {
        return;
    };
    if app.sheet != sheet || col_id >= range.cols() {
        return;
    }
    let sort_on = |app: &mut ExcelApp, on: SortOn, descending: bool, cx: &mut Cx| {
        sort_target(app, SortTarget::Filter { scope, range }, range.c0 + col_id, on, descending, cx)
    };
    match act {
        Act::Sort { descending } => sort_on(app, SortOn::Values, descending, cx),
        Act::SortColor(on) => sort_on(app, on, false, cx),
        Act::CustomSort => sort::open_on(app, SortTarget::Filter { scope, range }, window, cx),
        Act::Clear => apply_criteria(app, sheet, scope, col_id, None, cx),
        Act::Color(color) => apply_criteria(app, sheet, scope, col_id, Some(Criteria::Color(color)), cx),
        Act::Dynamic(dynamic) => apply_criteria(app, sheet, scope, col_id, Some(Criteria::Dynamic(dynamic)), cx),
        Act::Custom(first, second) => custom::open(app, scope, col_id, Some((first, second)), window, cx),
        Act::CustomDialog => custom::open(app, scope, col_id, None, window, cx),
        Act::Top10 => custom::open_top10(app, scope, col_id, window, cx),
    }
}

// --------------------------------------------------------- the list of values

/// A line of the list of values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Row {
    /// "(Select All)".
    All,
    /// "Add current selection to filter", while searching.
    Add,
    /// A year of the date tree, by index; then a month of it, a day of it.
    Year(usize),
    Month(usize, usize),
    Day(usize, usize, usize),
    /// A value, by index.
    Item(usize),
    Blanks,
}

/// Which leaves of the list the search box lets through.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    items: Vec<bool>,
    days: Vec<Vec<Vec<bool>>>,
    blanks: bool,
}

/// The check-list of a drop-down: the dates of the column as a year /
/// month / day tree, its other values, "(Blanks)", narrowed by what is
/// typed in the search box.
#[derive(Clone)]
pub(super) struct ValueList {
    /// The column, with the check marks as the user sees them.
    pub data: FilterDropdown,
    /// The check marks before the search box was used.
    base: FilterDropdown,
    locale: &'static Locale,
    /// What is searched, in lower case; empty when nothing is.
    needle: String,
    shown: Shown,
    /// "Add current selection to filter" is ticked.
    pub add_to_selection: bool,
    /// Open branches of the date tree: (year, 0) for a year, (year, month)
    /// for a month.
    expanded: BTreeSet<(i32, u32)>,
}

impl ValueList {
    pub(super) fn new(data: FilterDropdown, locale: &'static Locale) -> ValueList {
        let mut list = ValueList {
            base: data.clone(),
            data,
            locale,
            needle: String::new(),
            shown: Shown {
                items: Vec::new(),
                days: Vec::new(),
                blanks: true,
            },
            add_to_selection: false,
            expanded: BTreeSet::new(),
        };
        list.shown = list.matches();
        list
    }

    pub(super) fn searching(&self) -> bool {
        !self.needle.is_empty()
    }

    /// The date tree is drawn (lines get room for its expanders).
    pub(super) fn has_tree(&self) -> bool {
        !self.data.dates.is_empty()
    }

    fn found(&self, text: &str) -> bool {
        let text = text.to_lowercase();
        if self.needle.contains(['*', '?']) {
            wildcard_match(&format!("*{}*", self.needle), &text)
        } else {
            text.contains(&self.needle)
        }
    }

    /// What the search finds a day by: its parts and the date as a cell
    /// would show it.
    fn day_text(&self, year: i32, month: u32, day: u32) -> String {
        let short = date_to_serial(year as i64, month as i64, day as i64)
            .map(|serial| format::format_number(serial, self.locale.short_date_format, self.locale).text)
            .unwrap_or_default();
        format!("{day:02} {} {year} {short}", self.locale.months[(month.clamp(1, 12) - 1) as usize])
    }

    fn matches(&self) -> Shown {
        let all = !self.searching();
        Shown {
            items: self.data.items.iter().map(|item| all || self.found(&item.text)).collect(),
            days: self
                .data
                .dates
                .iter()
                .map(|year| {
                    year.months
                        .iter()
                        .map(|month| {
                            month
                                .days
                                .iter()
                                .map(|day| all || self.found(&self.day_text(year.year, month.month, day.day)))
                                .collect()
                        })
                        .collect()
                })
                .collect(),
            blanks: all || self.found(blanks_label()),
        }
    }

    /// The text of the search box changed: the lines it finds are listed,
    /// all of them ticked; emptied, the list is as it was before.
    pub(super) fn set_search(&mut self, text: &str) {
        self.needle = text.trim().to_lowercase();
        self.data = self.base.clone();
        self.shown = self.matches();
        if self.searching() {
            self.add_to_selection = false;
            self.set_shown(true);
        }
    }

    /// Every leaf with whether it is listed, for reading or ticking.
    fn leaves_mut(&mut self) -> impl Iterator<Item = (bool, &mut bool)> {
        let shown = &self.shown;
        let items = self.data.items.iter_mut().zip(&shown.items).map(|(item, on)| (*on, &mut item.checked));
        let days = self.data.dates.iter_mut().zip(&shown.days).flat_map(|(year, months)| {
            year.months.iter_mut().zip(months).flat_map(|(month, days)| {
                month.days.iter_mut().zip(days).map(|(day, on)| (*on, &mut day.checked))
            })
        });
        let blanks = self
            .data
            .has_blanks
            .then_some((shown.blanks, &mut self.data.blanks_checked));
        items.chain(days).chain(blanks)
    }

    fn set_shown(&mut self, checked: bool) {
        for (shown, mark) in self.leaves_mut() {
            if shown {
                *mark = checked;
            }
        }
    }

    /// The state of a box standing for several leaves.
    fn summary(states: impl Iterator<Item = bool>) -> Tri {
        let (mut on, mut off) = (false, false);
        for state in states {
            if state {
                on = true;
            } else {
                off = true;
            }
        }
        match (on, off) {
            (true, true) => Tri::Mixed,
            (true, false) => Tri::On,
            _ => Tri::Off,
        }
    }

    fn day_states(&self, year: usize, month: Option<usize>) -> impl Iterator<Item = bool> + '_ {
        let months = &self.data.dates[year].months;
        months
            .iter()
            .enumerate()
            .filter(move |(m, _)| month.is_none_or(|only| only == *m))
            .flat_map(move |(m, item)| {
                item.days
                    .iter()
                    .enumerate()
                    .filter(move |(d, _)| self.shown.days[year][m][*d])
                    .map(|(_, day)| day.checked)
            })
    }

    fn all_states(&self) -> impl Iterator<Item = bool> + '_ {
        let items = self
            .data
            .items
            .iter()
            .zip(&self.shown.items)
            .filter(|(_, shown)| **shown)
            .map(|(item, _)| item.checked);
        let days = (0..self.data.dates.len()).flat_map(|year| self.day_states(year, None));
        let blanks = (self.data.has_blanks && self.shown.blanks).then_some(self.data.blanks_checked);
        items.chain(days).chain(blanks)
    }

    /// The lines of the list, top to bottom.
    pub(super) fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        for (y, year) in self.data.dates.iter().enumerate() {
            if self.day_states(y, None).next().is_none() {
                continue;
            }
            out.push(Row::Year(y));
            // A search shows what it found, whatever was folded.
            if !self.searching() && !self.expanded.contains(&(year.year, 0)) {
                continue;
            }
            for (m, month) in year.months.iter().enumerate() {
                if self.day_states(y, Some(m)).next().is_none() {
                    continue;
                }
                out.push(Row::Month(y, m));
                if !self.searching() && !self.expanded.contains(&(year.year, month.month)) {
                    continue;
                }
                out.extend((0..month.days.len()).filter(|d| self.shown.days[y][m][*d]).map(|d| Row::Day(y, m, d)));
            }
        }
        out.extend((0..self.data.items.len()).filter(|i| self.shown.items[*i]).map(Row::Item));
        if self.data.has_blanks && self.shown.blanks {
            out.push(Row::Blanks);
        }
        if out.is_empty() {
            return out;
        }
        let mut head = vec![Row::All];
        if self.searching() {
            head.push(Row::Add);
        }
        head.extend(out);
        head
    }

    pub(super) fn label(&self, row: Row) -> String {
        match row {
            Row::All if self.searching() => t(
                "(Select All Search Results)",
                "(Sélectionner tous les résultats de la recherche)",
            )
            .to_string(),
            Row::All => t("(Select All)", "(Sélectionner tout)").to_string(),
            Row::Add => t("Add current selection to filter", "Ajouter la sélection actuelle au filtre").to_string(),
            Row::Year(y) => self.data.dates[y].year.to_string(),
            Row::Month(y, m) => month_name(self.locale, self.data.dates[y].months[m].month),
            Row::Day(y, m, d) => format!("{:02}", self.data.dates[y].months[m].days[d].day),
            Row::Item(i) => self.data.items[i].text.clone(),
            Row::Blanks => blanks_label().to_string(),
        }
    }

    pub(super) fn state(&self, row: Row) -> Tri {
        match row {
            Row::All => ValueList::summary(self.all_states()),
            Row::Add => Tri::from(self.add_to_selection),
            Row::Year(y) => ValueList::summary(self.day_states(y, None)),
            Row::Month(y, m) => ValueList::summary(self.day_states(y, Some(m))),
            Row::Day(y, m, d) => Tri::from(self.data.dates[y].months[m].days[d].checked),
            Row::Item(i) => Tri::from(self.data.items[i].checked),
            Row::Blanks => Tri::from(self.data.blanks_checked),
        }
    }

    /// How far a line is indented: the depth of a node of the date tree.
    pub(super) fn depth(&self, row: Row) -> usize {
        match row {
            Row::Month(..) => 1,
            Row::Day(..) => 2,
            _ => 0,
        }
    }

    /// For a branch of the date tree: whether it is open.
    pub(super) fn expanded(&self, row: Row) -> Option<bool> {
        let key = match row {
            Row::Year(y) => (self.data.dates[y].year, 0),
            Row::Month(y, m) => (self.data.dates[y].year, self.data.dates[y].months[m].month),
            _ => return None,
        };
        Some(self.searching() || self.expanded.contains(&key))
    }

    pub(super) fn set_expanded(&mut self, row: Row, open: bool) {
        let key = match row {
            Row::Year(y) => (self.data.dates[y].year, 0),
            Row::Month(y, m) => (self.data.dates[y].year, self.data.dates[y].months[m].month),
            _ => return,
        };
        if open {
            self.expanded.insert(key);
        } else {
            self.expanded.remove(&key);
        }
    }

    /// A click on the box of a line: a ticked box is cleared, any other
    /// one ticked, with the lines it stands for.
    pub(super) fn toggle(&mut self, row: Row) {
        let on = self.state(row) != Tri::On;
        match row {
            Row::All => self.set_shown(on),
            Row::Add => self.add_to_selection = on,
            Row::Year(y) => self.set_days(y, None, on),
            Row::Month(y, m) => self.set_days(y, Some(m), on),
            Row::Day(y, m, d) => self.data.dates[y].months[m].days[d].checked = on,
            Row::Item(i) => self.data.items[i].checked = on,
            Row::Blanks => self.data.blanks_checked = on,
        }
    }

    fn set_days(&mut self, year: usize, month: Option<usize>, checked: bool) {
        let shown = &self.shown.days[year];
        for (m, item) in self.data.dates[year].months.iter_mut().enumerate() {
            if month.is_some_and(|only| only != m) {
                continue;
            }
            for (d, day) in item.days.iter_mut().enumerate() {
                if shown[m][d] {
                    day.checked = checked;
                }
            }
        }
    }

    /// OK has something to keep: at least one listed line is ticked.
    pub(super) fn can_apply(&self) -> bool {
        self.all_states().any(|checked| checked)
    }

    /// The criterion OK gives the column; `None` when everything shows.
    /// After a search, only the ticked lines it found are kept, unless
    /// they are added to what the column already showed.
    pub(super) fn criteria(&self) -> Option<Criteria> {
        if !self.searching() {
            return self.data.values_criteria();
        }
        let mut result = self.clone();
        let add = self.add_to_selection;
        let was = result.base.clone();
        let mut before = ValueList::new(was, self.locale);
        for ((shown, mark), (_, old)) in result.leaves_mut().zip(before.leaves_mut()) {
            if !shown {
                *mark = add && *old;
            } else if add {
                *mark |= *old;
            }
        }
        result.data.values_criteria()
    }
}

// ----------------------------------------------------------------- the view

const WIDTH: f32 = 300.0;
const ENTRY_H: f32 = 24.0;
const LIST_ROWS: f32 = 10.0;
const FIELD_BORDER: u32 = 0xABABAB;

pub(super) struct FilterMenu {
    app: WeakEntity<ExcelApp>,
    sheet: usize,
    scope: FilterScope,
    col_id: u32,
    entries: Vec<Entry>,
    /// The open submenus, outermost first: an index into `entries`, then
    /// into the entries of that submenu.
    path: Vec<usize>,
    /// The entry under the pointer or the arrow keys, in the innermost
    /// open menu.
    hot: Option<usize>,
    /// Where the entries that open a submenu were painted, by level and
    /// index: their submenu hangs beside them.
    places: Rc<RefCell<HashMap<(usize, usize), Bounds<Pixels>>>>,
    list: ValueList,
    /// The user ticked a box or searched: OK has something to apply.
    dirty: bool,
    search: Entity<TextInput>,
    scroll: Entity<ScrollState>,
    /// Line of the list the arrow keys are on.
    cursor: usize,
    menu_focus: FocusHandle,
    list_focus: FocusHandle,
    dismissed: Rc<Cell<Option<Point<Pixels>>>>,
}

/// Open the drop-down of a filter button.
pub(super) fn open(app: &mut ExcelApp, button: FilterButton, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    if !app.may(Intent::UseAutoFilter, cx) {
        return;
    }
    let sheet = app.sheet;
    let Some(data) = app.wb.filter_dropdown(sheet, button.scope, button.col_id) else {
        return;
    };
    let locale = app.wb.locale;
    let weak = cx.weak_entity();
    let dismissed = Rc::new(Cell::new(None));
    let view = cx.new(|cx| FilterMenu::new(weak, sheet, button, data, locale, dismissed.clone(), cx));
    // It hangs from its button: under the cell, from the left edge of the
    // button.
    let cell = Rect::cell(button.row, button.col);
    let mut anchor = app.cell_anchor(cell);
    if let Some(layout) = app.current_layout() {
        if let Some((left, ..)) = button_box(layout.rect_px(cell), layout.zoom) {
            anchor.x = layout.to_window(left, 0.0).x;
        }
    }
    super::dbg_log(&format!("open at {:?}", anchor));
    app.open_popup_view(anchor, view.clone(), cx);
    window.focus(&view.read(cx).menu_focus);
    app.ext_mut::<State>().menu = Some(OpenMenu {
        key: (button.scope, button.col_id),
        dismissed,
    });
}

impl FilterMenu {
    fn new(
        app: WeakEntity<ExcelApp>,
        sheet: usize,
        button: FilterButton,
        data: FilterDropdown,
        locale: &'static Locale,
        dismissed: Rc<Cell<Option<Point<Pixels>>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = widgets::input(cx, "");
        search.update(cx, |field, _| field.placeholder = t("Search", "Rechercher").into());
        cx.subscribe(&search, |this: &mut Self, field, event: &InputEvent, cx| {
            if *event == InputEvent::Changed {
                let text = field.read(cx).text().to_string();
                this.list.set_search(&text);
                this.dirty = true;
                this.cursor = 0;
                this.scroll.update(cx, |scroll, _| scroll.top = 0.0);
                cx.notify();
            }
        })
        .detach();
        FilterMenu {
            app,
            sheet,
            scope: button.scope,
            col_id: button.col_id,
            entries: entries(&data, button.col, locale),
            path: Vec::new(),
            hot: None,
            places: Rc::default(),
            list: ValueList::new(data, locale),
            dirty: false,
            search,
            scroll: cx.new(|_| ScrollState::new()),
            cursor: 0,
            menu_focus: cx.focus_handle().tab_stop(true),
            list_focus: cx.focus_handle().tab_stop(true),
            dismissed,
        }
    }

    /// The entries of the menu at a depth of `path`.
    fn level(&self, level: usize) -> &[Entry] {
        let mut entries = &self.entries[..];
        for index in &self.path[..level.min(self.path.len())] {
            match entries.get(*index).map(|entry| &entry.kind) {
                Some(EntryKind::Sub(children)) => entries = children,
                _ => return &[],
            }
        }
        entries
    }

    /// The pointer is on an entry: it is highlighted, its submenu opens
    /// and those of its siblings close.
    fn hover(&mut self, level: usize, index: usize, cx: &mut Context<Self>) {
        // A panel that was just folded may still report the pointer.
        let Some(entry) = self.level(level).get(index).filter(|_| level <= self.path.len()) else {
            return;
        };
        let opens = matches!(entry.kind, EntryKind::Sub(_)) && !entry.disabled;
        if self.path.get(level) == Some(&index) {
            // Its submenu is open already; leaving a deeper entry for it
            // takes the highlight away from that entry.
            if self.path.len() == level + 1 && self.hot.is_some() {
                self.hot = None;
                cx.notify();
            }
            return;
        }
        if !opens && self.path.len() == level && self.hot == Some(index) {
            // Still on the same entry.
            return;
        }
        self.path.truncate(level);
        if opens {
            self.path.push(index);
            self.hot = None;
        } else {
            self.hot = Some(index);
        }
        cx.notify();
    }

    /// Close the drop-down, then run something on the application. Done
    /// once this view is out of the way: it goes with its drop-down.
    fn finish(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut ExcelApp, &mut Window, &mut Cx) + 'static,
    ) {
        let app = self.app.clone();
        window.defer(cx, move |window, cx| {
            app.update(cx, |app, cx| {
                app.close_popup(window, cx);
                then(app, window, cx);
            })
            .ok();
        });
    }

    fn run(&self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let (sheet, scope, col_id) = (self.sheet, self.scope, self.col_id);
        self.finish(window, cx, move |app, window, cx| perform(app, sheet, scope, col_id, act, window, cx));
    }

    /// Return or Space on the highlighted entry.
    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.path.len();
        let Some((index, entry)) = self.hot.and_then(|i| Some((i, self.level(level).get(i)?))) else {
            return;
        };
        if !entry.selectable() {
            return;
        }
        match &entry.kind {
            EntryKind::Act(act) => self.run(*act, window, cx),
            EntryKind::Sub(children) => {
                // The keyboard goes into the submenu it opens.
                self.hot = children.iter().position(Entry::selectable);
                self.path.push(index);
                cx.notify();
            }
            EntryKind::Separator | EntryKind::Heading => {}
        }
    }

    fn menu_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !widgets::plain(event) {
            return;
        }
        let level = self.path.len();
        let stops: Vec<usize> = self
            .level(level)
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.selectable())
            .map(|(i, _)| i)
            .collect();
        let at = self.hot.and_then(|hot| stops.iter().position(|i| *i == hot));
        match event.keystroke.key.as_str() {
            "down" if !stops.is_empty() => self.hot = Some(stops[at.map_or(0, |at| (at + 1) % stops.len())]),
            "up" if !stops.is_empty() => {
                self.hot = Some(stops[at.map_or(stops.len() - 1, |at| (at + stops.len() - 1) % stops.len())])
            }
            "right" => {
                let opens = self
                    .hot
                    .and_then(|hot| self.level(level).get(hot))
                    .is_some_and(|entry| matches!(entry.kind, EntryKind::Sub(_)));
                if opens {
                    self.activate(window, cx);
                }
            }
            "left" => {
                if let Some(parent) = self.path.pop() {
                    self.hot = Some(parent);
                }
            }
            "space" => self.activate(window, cx),
            _ => {
                // Typing goes to the search box.
                let Some(text) = event.keystroke.key_char.clone().filter(|text| !text.trim().is_empty()) else {
                    return;
                };
                window.focus(&self.search.read(cx).focus_handle(cx));
                self.search.update(cx, |field, cx| field.insert(&text, window, cx));
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn list_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !widgets::plain(event) {
            return;
        }
        let rows = self.list.rows();
        let Some(row) = rows.get(self.cursor).copied() else {
            return;
        };
        let last = rows.len() - 1;
        let page = LIST_ROWS as usize - 1;
        let open = self.list.expanded(row);
        let next = match event.keystroke.key.as_str() {
            "up" => self.cursor.saturating_sub(1),
            "down" => (self.cursor + 1).min(last),
            "pageup" => self.cursor.saturating_sub(page),
            "pagedown" => (self.cursor + page).min(last),
            "home" => 0,
            "end" => last,
            "space" => {
                self.list.toggle(row);
                self.dirty = true;
                self.cursor
            }
            "right" if open == Some(false) => {
                self.list.set_expanded(row, true);
                self.cursor
            }
            "left" if open == Some(true) => {
                self.list.set_expanded(row, false);
                self.cursor
            }
            _ => return,
        };
        self.cursor = next;
        self.scroll
            .update(cx, |scroll, _| scroll.reveal(next, next, ROW_H, LIST_ROWS * ROW_H));
        cx.stop_propagation();
        cx.notify();
    }

    /// OK: give the column the criterion of the boxes as they are ticked.
    fn ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.can_apply() {
            return;
        }
        let (sheet, scope, col_id) = (self.sheet, self.scope, self.col_id);
        // Untouched, the list leaves the column as it is filtered.
        let criteria = self.dirty.then(|| self.list.criteria());
        self.finish(window, cx, move |app, _, cx| {
            if let Some(criteria) = criteria {
                apply_criteria(app, sheet, scope, col_id, criteria, cx);
            }
        });
    }

    /// Return: the highlighted entry when the menu has the keyboard, else
    /// OK.
    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu_focus.is_focused(window) && self.hot.is_some() {
            self.activate(window, cx);
        } else {
            self.ok(window, cx);
        }
    }

    /// Escape closes the innermost submenu, then the drop-down.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        match self.path.pop() {
            Some(parent) => {
                self.hot = Some(parent);
                cx.notify();
            }
            None => cx.propagate(),
        }
    }

    fn entry_row(&self, level: usize, index: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        if entry.kind == EntryKind::Separator {
            return div().h(px(1.)).my(px(3.)).mx(px(8.)).bg(c(theme::BORDER)).into_any_element();
        }
        if entry.kind == EntryKind::Heading {
            return div()
                .h(px(ENTRY_H - 4.0))
                .px(px(10.))
                .flex()
                .items_center()
                .text_size(px(11.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(c(theme::TEXT_MUTED))
                .child(entry.label.clone())
                .into_any_element();
        }
        let lit = !entry.disabled
            && (self.path.get(level) == Some(&index) || (self.path.len() == level && self.hot == Some(index)));
        let color = if entry.disabled { theme::TEXT_DISABLED } else { theme::TEXT };
        let mark = div().w(px(18.)).h(px(18.)).flex_none().flex().items_center().justify_center();
        let mark = match (entry.icon, entry.checked) {
            // A sort entry in use shows its icon pressed.
            (Some(name), on) => mark
                .rounded(px(3.))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .child(icon(name, 14.0, if entry.disabled { color } else { theme::TEXT_MUTED })),
            (None, true) => mark.child(icon("check", 13.0, theme::ACCENT)),
            (None, false) => mark,
        };
        let body = match entry.swatch {
            Some(Color(rgb)) => div()
                .flex_1()
                .child(
                    div()
                        .w(px(96.))
                        .h(px(14.))
                        .bg(c(rgb))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG)),
                )
                .into_any_element(),
            None => div().flex_1().truncate().child(entry.label.clone()).into_any_element(),
        };
        let mut row = div()
            .id(("filter-entry", level * 1000 + index))
            .relative()
            .h(px(ENTRY_H))
            .mx(px(4.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .text_color(c(color))
            .when(lit, |d| d.bg(c(theme::HOVER)))
            .child(mark)
            .child(body)
            .on_mouse_move(cx.listener(move |this, _, _, cx| this.hover(level, index, cx)));
        match &entry.kind {
            EntryKind::Sub(_) => {
                let places = self.places.clone();
                row = row.child(icon("chevron-right", 10.0, color)).child(
                    canvas(
                        move |bounds, _, _| {
                            places.borrow_mut().insert((level, index), bounds);
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                );
            }
            EntryKind::Act(act) if !entry.disabled => {
                let (app, sheet, scope, col_id, act) = (self.app.clone(), self.sheet, self.scope, self.col_id, *act);
                row = row.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.close_popup(window, cx);
                        perform(app, sheet, scope, col_id, act, window, cx);
                    })
                    .ok();
                    cx.stop_propagation();
                });
            }
            _ => {}
        }
        row.into_any_element()
    }

    /// The panels of the open submenus, each hung beside its entry.
    fn submenus(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        for level in 1..=self.path.len() {
            let parent = self.path[level - 1];
            let Some(place) = self.places.borrow().get(&(level - 1, parent)).copied() else {
                continue;
            };
            let entries = self.level(level);
            let longest = entries.iter().map(|entry| entry.label.chars().count()).max().unwrap_or(0);
            let swatches = entries.iter().any(|entry| entry.swatch.is_some());
            let width = if swatches { 190.0 } else { (longest as f32 * 7.0 + 64.0).clamp(160.0, 340.0) };
            let rows: Vec<AnyElement> = entries
                .iter()
                .enumerate()
                .map(|(index, entry)| self.entry_row(level, index, entry, cx))
                .collect();
            let panel = div()
                .id(("filter-submenu", level))
                .occlude()
                .w(px(width))
                .max_h(window.viewport_size().height - px(12.))
                .overflow_y_scroll()
                .py(px(4.))
                .bg(c(0xFFFFFF))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .rounded(px(6.))
                .shadow_lg()
                .text_size(px(12.5))
                .text_color(c(theme::TEXT))
                .children(rows);
            // Beside its entry: on the right, or on the left when the
            // window has no room there.
            let fits = place.right() + px(width + 8.0) <= window.viewport_size().width;
            let (corner, x) = if fits {
                (Corner::TopLeft, place.right() + px(2.))
            } else {
                (Corner::TopRight, place.left() - px(2.))
            };
            out.push(
                deferred(
                    anchored()
                        .anchor(corner)
                        .position(point(x, place.top() - px(5.)))
                        .snap_to_window_with_margin(px(6.))
                        .child(panel),
                )
                .with_priority(2)
                .into_any_element(),
            );
        }
        out
    }

    fn value_list(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.list.rows();
        let focused = self.list_focus.is_focused(window);
        let height = LIST_ROWS * ROW_H;
        if rows.is_empty() {
            return div()
                .h(px(height))
                .flex()
                .items_center()
                .justify_center()
                .text_color(c(theme::TEXT_MUTED))
                .child(t("No items match your search.", "Aucun élément ne correspond à votre recherche."))
                .into_any_element();
        }
        self.cursor = self.cursor.min(rows.len() - 1);
        let cursor = self.cursor;
        let tree = self.list.has_tree();
        let list = &self.list;
        let view = cx.weak_entity();
        let lines = virtual_rows(&self.scroll, rows.len(), ROW_H, height, cx, |i| {
            let row = rows[i];
            let depth = list.depth(row) as f32;
            let expander = match list.expanded(row) {
                Some(open) => {
                    let view = view.clone();
                    div()
                        .id(("expand", i))
                        .size(px(14.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(2.))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 9.0, theme::TEXT_MUTED))
                        .on_click(move |_, _, cx: &mut App| {
                            view.update(cx, |this, cx| {
                                this.list.set_expanded(row, !open);
                                this.cursor = i;
                                cx.notify();
                            })
                            .ok();
                            cx.stop_propagation();
                        })
                        .into_any_element()
                }
                None => div().size(px(14.)).flex_none().into_any_element(),
            };
            let view = view.clone();
            div()
                .id(("value", i))
                .size_full()
                .pl(px(6.0 + depth * 16.0))
                .pr(px(6.))
                .flex()
                .items_center()
                .gap(px(5.))
                .when(focused && i == cursor, |d| d.bg(c(widgets::SELECTED_BG)))
                .when(!(focused && i == cursor), |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .when(tree, |d| d.child(expander))
                .child(check_mark(list.state(row), false))
                .child(div().truncate().child(list.label(row)))
                .on_click(move |_, _, cx: &mut App| {
                    view.update(cx, |this, cx| {
                        this.list.toggle(row);
                        this.dirty = true;
                        this.cursor = i;
                        cx.notify();
                    })
                    .ok();
                })
                .into_any_element()
        });
        div()
            .id("filter-values")
            .track_focus(&self.list_focus)
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(if focused { theme::ACCENT } else { FIELD_BORDER }))
            .rounded(px(3.))
            .overflow_hidden()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.list_key(event, cx)))
            .child(lines)
            .into_any_element()
    }
}

impl Render for FilterMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::dbg_log("render menu");
        let menu: Vec<AnyElement> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| self.entry_row(0, index, entry, cx))
            .collect();
        let submenus = self.submenus(window, cx);
        let values = self.value_list(window, cx);
        let list_focus = self.list_focus.clone();
        let dismissed = self.dismissed.clone();
        div()
            .id("filter-menu")
            .w(px(WIDTH))
            .py(px(4.))
            .flex()
            .flex_col()
            .text_size(px(12.5))
            .on_action(cx.listener(|this, _: &widgets::Accept, window, cx| this.accept(window, cx)))
            .on_action(cx.listener(|this, _: &text_input::Enter, window, cx| this.ok(window, cx)))
            .on_action(cx.listener(|this, _: &widgets::Dismiss, _, cx| this.dismiss(cx)))
            .on_action(cx.listener(|this, _: &text_input::Escape, _, cx| this.dismiss(cx)))
            // A press outside closes the drop-down; where it landed tells
            // its button not to open it again.
            .on_mouse_down_out(move |event, _, _| {
                super::dbg_log(&format!("mouse_down_out {:?}", event.position));
                dismissed.set(Some(event.position))
            })
            .child(
                div()
                    .id("filter-entries")
                    .track_focus(&self.menu_focus)
                    .flex()
                    .flex_col()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| this.menu_key(event, window, cx)))
                    .children(menu),
            )
            .child(
                div()
                    .id("filter-list")
                    .px(px(10.))
                    .pt(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    // A press on the list folds the submenus. (Passing over
                    // it does not: the way from an entry to its submenu
                    // often cuts a corner of the list.)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if !this.path.is_empty() || this.hot.is_some() {
                                this.path.clear();
                                this.hot = None;
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            // Down leaves the search box for the list.
                            .on_action(move |_: &text_input::Down, window, _| window.focus(&list_focus))
                            .child(
                                TextField::new(&self.search)
                                    .trailing(div().pr(px(4.)).child(icon("search", 12.0, theme::TEXT_MUTED))),
                            ),
                    )
                    .child(values)
                    .when(self.list.data.truncated, |d| {
                        d.child(
                            div()
                                .text_color(c(theme::TEXT_MUTED))
                                .child(t("Not all items showing", "Les éléments ne s'affichent pas tous")),
                        )
                    })
                    .child(
                        div()
                            .pt(px(3.))
                            .pb(px(6.))
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                Button::new("filter-ok", t("OK", "OK"))
                                    .primary()
                                    .disabled(!self.list.can_apply())
                                    .on_click(cx.listener(|this, _, window, cx| this.ok(window, cx))),
                            )
                            .child(
                                Button::new("filter-cancel", t("Cancel", "Annuler"))
                                    .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx, |_, _, _| {}))),
                            ),
                    ),
            )
            .children(submenus)
    }
}
