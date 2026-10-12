//! Slicers and timelines (`Sheet::slicers`, `Sheet::timelines`): floating
//! objects holding the buttons of a filter.
//!
//! Neither keeps a selection of its own. A slicer shows, and changes, the
//! checked items of a field in the PivotTables it is connected to, or the
//! checked values of a table column; a timeline does the same with the
//! date filter of a field. A filter made elsewhere (the drop-down of the
//! field, the button of the table) therefore shows in them at once, and
//! deleting one leaves the filter as it is, as in Excel.
//!
//! What a file calls the slicer cache (the source, the sorting, the way
//! items without data are shown) is kept with the slicer: a copied slicer
//! simply carries the same cache name.

use crate::ast::StructuralEdit;
use crate::datetime::{date_to_serial, days_in_month, serial_to_ymd};
use crate::filter::{Criteria, DateGroup, FilterScope, ValueFilter};
use crate::format::{self, FormatKind};
use crate::locale::Locale;
use crate::ops::{collate, sort_order};
use crate::pivot::{self, Caption, GroupKind, PivotError, PivotTable};
use crate::table::TableOption;
use crate::value::Value;
use crate::workbook::{Sheet, Workbook};
use std::collections::HashMap;

/// Size of a new slicer and of a new timeline, pixels at 100 %: Excel's
/// 2" by 2.63" and 3.47" by 1.43".
pub const SLICER_SIZE: (f32, f32) = (192.0, 252.0);
pub const TIMELINE_SIZE: (f32, f32) = (333.0, 137.0);
/// Height of a slicer button, pixels at 100 % (0.26").
pub const BUTTON_HEIGHT: f32 = 25.0;

/// What a slicer filters.
#[derive(Clone, Debug, PartialEq)]
pub enum SlicerSource {
    /// A field (by its name in the cache) of PivotTables sharing a cache,
    /// by `PivotTable::id`.
    Pivot { pivots: Vec<u32>, field: String },
    /// A column (by name) of a table, by `Table::id`.
    Table { table: u32, column: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slicer {
    /// Unique in the workbook.
    pub id: u32,
    /// Unique in the workbook, compared without case ("Region").
    pub name: String,
    /// The text of the header.
    pub caption: String,
    /// Name of the slicer cache in files ("Slicer_Region").
    pub cache_name: String,
    pub source: SlicerSource,
    /// Cell holding the top-left corner (row, col), offset of the corner
    /// inside that cell and size, pixels at 100 %.
    pub anchor: (u32, u32),
    pub offset: (f32, f32),
    pub size: (f32, f32),
    /// "Display header".
    pub show_header: bool,
    /// Buttons per row.
    pub columns: u32,
    /// Height of a button, pixels at 100 %.
    pub button_height: f32,
    /// "SlicerStyleLight1"...
    pub style: String,
    /// "Descending (Z to A)".
    pub descending: bool,
    /// "Use Custom Lists when sorting".
    pub custom_lists: bool,
    /// "Hide items with no data".
    pub hide_no_data: bool,
    /// "Visually indicate items with no data".
    pub indicate_no_data: bool,
    /// "Show items with no data last".
    pub no_data_last: bool,
    /// "Show items deleted from the data source".
    pub show_deleted: bool,
    /// The first item shown, when the items do not all fit.
    pub start_item: u32,
}

impl Slicer {
    /// The PivotTables the slicer is connected to.
    pub fn pivots(&self) -> &[u32] {
        match &self.source {
            SlicerSource::Pivot { pivots, .. } => pivots,
            SlicerSource::Table { .. } => &[],
        }
    }
}

/// A button of a slicer.
#[derive(Clone, Debug, PartialEq)]
pub struct SlicerItem {
    /// What `slicer_select` takes: the index of the item in the cache of
    /// the PivotTable, or the rank of the value among those of the table
    /// column, in order of first appearance.
    pub x: u32,
    pub label: String,
    pub selected: bool,
    /// False when the other filters leave no record with this item.
    pub has_data: bool,
}

/// The periods a timeline shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimeLevel {
    Years,
    Quarters,
    Months,
    Days,
}

impl TimeLevel {
    pub const ALL: [TimeLevel; 4] = [TimeLevel::Years, TimeLevel::Quarters, TimeLevel::Months, TimeLevel::Days];

    /// As the list of the timeline names it.
    pub fn label(self, locale: &Locale) -> &'static str {
        let names: [&str; 5] = match self {
            TimeLevel::Years => ["YEARS", "ANNÉES", "AÑOS", "JAHRE", "ANNI"],
            TimeLevel::Quarters => ["QUARTERS", "TRIMESTRES", "TRIMESTRES", "QUARTALE", "TRIMESTRI"],
            TimeLevel::Months => ["MONTHS", "MOIS", "MESES", "MONATE", "MESI"],
            TimeLevel::Days => ["DAYS", "JOURS", "DÍAS", "TAGE", "GIORNI"],
        };
        names[lang(locale)]
    }
}

fn lang(locale: &Locale) -> usize {
    ["en", "fr", "es", "de", "it"].iter().position(|id| *id == locale.id).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    /// Unique in the workbook, among timelines.
    pub id: u32,
    /// Unique in the workbook, compared without case ("Date").
    pub name: String,
    pub caption: String,
    /// Name of the timeline cache in files ("NativeTimeline_Date").
    pub cache_name: String,
    /// The PivotTables it filters (`PivotTable::id`), all on one cache.
    pub pivots: Vec<u32>,
    /// The date field, by its name in the cache.
    pub field: String,
    pub anchor: (u32, u32),
    pub offset: (f32, f32),
    pub size: (f32, f32),
    pub show_header: bool,
    pub show_selection_label: bool,
    pub show_time_level: bool,
    pub show_scrollbar: bool,
    pub level: TimeLevel,
    /// "TimeSlicerStyleLight1"...
    pub style: String,
    /// The first day of the first period shown; `None` until the user
    /// scrolls (the band then starts at the selection or at the data).
    pub scroll: Option<f64>,
}

/// One period of the band of a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Period {
    /// First and last day, as serial numbers.
    pub start: f64,
    pub end: f64,
    /// The text of its tile ("JAN", "Q1", "2015", "17").
    pub label: String,
    /// What is written above the tile where a group starts: the year of
    /// months and quarters, the month and year of days.
    pub group: Option<String>,
}

fn first_of_month(year: i64, month: u32) -> f64 {
    date_to_serial(year, month as i64, 1).unwrap_or(0.0)
}

fn month_title(locale: &Locale, month: u32) -> String {
    locale.months_short[(month as usize + 11) % 12].trim_end_matches('.').to_string()
}

/// The periods of a level covering the days `start..=end`.
pub fn periods(level: TimeLevel, start: f64, end: f64, locale: &Locale) -> Vec<Period> {
    let (y0, m0, _) = serial_to_ymd(start);
    let (y1, m1, _) = serial_to_ymd(end);
    let mut out = Vec::new();
    match level {
        TimeLevel::Years => {
            for year in y0..=y1 {
                out.push(Period {
                    start: first_of_month(year, 1),
                    end: first_of_month(year + 1, 1) - 1.0,
                    label: year.to_string(),
                    group: None,
                });
            }
        }
        TimeLevel::Quarters => {
            let (mut year, mut q) = (y0, (m0 - 1) / 3);
            while (year, q) <= (y1, (m1 - 1) / 3) {
                let next = if q == 3 { first_of_month(year + 1, 1) } else { first_of_month(year, q * 3 + 4) };
                out.push(Period {
                    start: first_of_month(year, q * 3 + 1),
                    end: next - 1.0,
                    label: format!("{}{}", quarter_prefix(locale), q + 1),
                    group: (q == 0 || out.is_empty()).then(|| year.to_string()),
                });
                (year, q) = if q == 3 { (year + 1, 0) } else { (year, q + 1) };
            }
        }
        TimeLevel::Months => {
            let (mut year, mut month) = (y0, m0);
            while (year, month) <= (y1, m1) {
                out.push(Period {
                    start: first_of_month(year, month),
                    end: first_of_month(year, month) + days_in_month(year, month) as f64 - 1.0,
                    label: month_title(locale, month).to_uppercase(),
                    group: (month == 1 || out.is_empty()).then(|| year.to_string()),
                });
                (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
            }
        }
        TimeLevel::Days => {
            let mut day = start.floor();
            while day <= end {
                let (year, month, d) = serial_to_ymd(day);
                out.push(Period {
                    start: day,
                    end: day,
                    label: d.to_string(),
                    group: (d == 1 || out.is_empty())
                        .then(|| format!("{} {year}", month_title(locale, month).to_uppercase())),
                });
                day += 1.0;
            }
        }
    }
    out
}

fn quarter_prefix(locale: &Locale) -> &'static str {
    ["Q", "T", "T", "Q", "T"][lang(locale)]
}

/// What the selection label of a timeline reads for the days
/// `start..=end` ("Mar 2015", "Q1 - Q2 2015", "2014 - 2016").
pub fn span_label(level: TimeLevel, start: f64, end: f64, locale: &Locale) -> String {
    let (ys, ms, ds) = serial_to_ymd(start);
    let (ye, me, de) = serial_to_ymd(end);
    let month = |m: u32| month_title(locale, m);
    let quarter = |m: u32| format!("{}{}", quarter_prefix(locale), (m - 1) / 3 + 1);
    match level {
        TimeLevel::Years if ys == ye => ys.to_string(),
        TimeLevel::Years => format!("{ys} - {ye}"),
        TimeLevel::Quarters if (ys, (ms - 1) / 3) == (ye, (me - 1) / 3) => format!("{} {ys}", quarter(ms)),
        TimeLevel::Quarters if ys == ye => format!("{} - {} {ys}", quarter(ms), quarter(me)),
        TimeLevel::Quarters => format!("{} {ys} - {} {ye}", quarter(ms), quarter(me)),
        TimeLevel::Months if (ys, ms) == (ye, me) => format!("{} {ys}", month(ms)),
        TimeLevel::Months if ys == ye => format!("{} - {} {ys}", month(ms), month(me)),
        TimeLevel::Months => format!("{} {ys} - {} {ye}", month(ms), month(me)),
        TimeLevel::Days if (ys, ms, ds) == (ye, me, de) => format!("{ds} {} {ys}", month(ms)),
        TimeLevel::Days if (ys, ms) == (ye, me) => format!("{ds} - {de} {} {ys}", month(ms)),
        TimeLevel::Days => format!("{ds} {} {ys} - {de} {} {ye}", month(ms), month(me)),
    }
}

/// "All Periods", the selection label of a timeline that filters nothing.
pub fn all_periods(locale: &Locale) -> &'static str {
    [
        "All Periods",
        "Toutes les périodes",
        "Todos los períodos",
        "Alle Zeiträume",
        "Tutti i periodi",
    ][lang(locale)]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlicerError {
    NotFound,
    /// The name is empty or another slicer or timeline has it.
    InvalidName,
    /// The field holds something else than dates.
    NotADate,
}

/// A name for a file's slicer cache: what is not a letter or a digit
/// becomes "_".
fn cache_name(prefix: &str, name: &str) -> String {
    let body: String = name.chars().map(|ch| if ch.is_alphanumeric() { ch } else { '_' }).collect();
    format!("{prefix}{body}")
}

fn unique(base: String, taken: &[String], separator: &str) -> String {
    let has = |name: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(name) || t.to_lowercase() == name.to_lowercase());
    if !has(&base) {
        return base;
    }
    (1..).map(|n| format!("{base}{separator}{n}")).find(|name| !has(name)).unwrap_or(base)
}

/// The items of a table column as a slicer lists them.
struct ColumnItems {
    /// Label and value of each distinct value, in order of first
    /// appearance; a blank label stands for the empty cells.
    items: Vec<(String, Value)>,
    /// The day of the values shown as dates.
    days: Vec<Option<(i32, u32, u32)>>,
}

impl Workbook {
    // ---- finding -----------------------------------------------------------

    /// Every slicer of the workbook with the index of its sheet.
    pub fn slicers(&self) -> impl Iterator<Item = (usize, &Slicer)> {
        self.sheets.iter().enumerate().flat_map(|(si, sh)| sh.slicers.iter().map(move |s| (si, s)))
    }

    pub fn slicer(&self, id: u32) -> Option<(usize, &Slicer)> {
        self.slicers().find(|(_, s)| s.id == id)
    }

    pub fn timelines(&self) -> impl Iterator<Item = (usize, &Timeline)> {
        self.sheets.iter().enumerate().flat_map(|(si, sh)| sh.timelines.iter().map(move |t| (si, t)))
    }

    pub fn timeline(&self, id: u32) -> Option<(usize, &Timeline)> {
        self.timelines().find(|(_, t)| t.id == id)
    }

    fn slicer_names(&self) -> Vec<String> {
        self.slicers().map(|(_, s)| s.name.clone()).chain(self.timelines().map(|(_, t)| t.name.clone())).collect()
    }

    fn slicer_cache_names(&self) -> Vec<String> {
        self.slicers()
            .map(|(_, s)| s.cache_name.clone())
            .chain(self.timelines().map(|(_, t)| t.cache_name.clone()))
            .collect()
    }

    /// The first PivotTable a slicer or a timeline is connected to that
    /// still exists, and the index of the field in its cache.
    fn sliced_pivot<'a>(&'a self, pivots: &[u32], field: &str) -> Option<(&'a PivotTable, usize)> {
        pivots.iter().find_map(|id| {
            let (_, p) = self.pivot(*id)?;
            Some((p, p.cache.field_index(field)?))
        })
    }

    /// The PivotTables a slicer on `pivot` can be connected to: those
    /// built on the same cache, itself included, as (sheet, id).
    pub fn pivots_sharing_cache(&self, pivot: u32) -> Vec<(usize, u32)> {
        let Some((_, p)) = self.pivot(pivot) else {
            return Vec::new();
        };
        self.pivots().filter(|(_, q)| std::sync::Arc::ptr_eq(&p.cache, &q.cache)).map(|(si, q)| (si, q.id)).collect()
    }

    // ---- slicers -----------------------------------------------------------

    fn new_slicer(&self, caption: &str, source: SlicerSource, anchor: (u32, u32)) -> Slicer {
        let name = unique(caption.to_string(), &self.slicer_names(), " ");
        Slicer {
            id: self.slicers().map(|(_, s)| s.id).max().unwrap_or(0) + 1,
            cache_name: unique(cache_name("Slicer_", caption), &self.slicer_cache_names(), ""),
            name,
            caption: caption.to_string(),
            source,
            anchor,
            offset: (0.0, 0.0),
            size: SLICER_SIZE,
            show_header: true,
            columns: 1,
            button_height: BUTTON_HEIGHT,
            style: "SlicerStyleLight1".to_string(),
            descending: false,
            custom_lists: true,
            hide_no_data: false,
            indicate_no_data: true,
            no_data_last: true,
            show_deleted: true,
            start_item: 0,
        }
    }

    /// Put a slicer for a field of a PivotTable on `sheet`; returns its id.
    pub fn add_pivot_slicer(&mut self, sheet: usize, pivot: u32, field: usize, anchor: (u32, u32)) -> Option<u32> {
        let (_, p) = self.pivot(pivot)?;
        let name = p.cache.fields.get(field)?.name.clone();
        let caption = p.field_caption(field).to_string();
        let source = SlicerSource::Pivot {
            pivots: vec![pivot],
            field: name,
        };
        let slicer = self.new_slicer(&caption, source, anchor);
        self.push_slicer(sheet, slicer)
    }

    /// Put a slicer for a column of a table on `sheet`; returns its id.
    pub fn add_table_slicer(&mut self, sheet: usize, table: u32, column: usize, anchor: (u32, u32)) -> Option<u32> {
        let (_, t) = self.table_by_id(table)?;
        let name = t.columns.get(column)?.name.clone();
        let source = SlicerSource::Table {
            table,
            column: name.clone(),
        };
        let slicer = self.new_slicer(&name, source, anchor);
        self.push_slicer(sheet, slicer)
    }

    fn push_slicer(&mut self, sheet: usize, slicer: Slicer) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        let id = slicer.id;
        self.snapshot();
        self.sheets[sheet].slicers.push(slicer);
        Some(id)
    }

    /// Put a copy of a slicer on `sheet` (Paste): it filters the same
    /// thing under a name of its own.
    pub fn paste_slicer(&mut self, sheet: usize, mut slicer: Slicer) -> Option<u32> {
        slicer.id = self.slicers().map(|(_, s)| s.id).max().unwrap_or(0) + 1;
        slicer.name = unique(slicer.name, &self.slicer_names(), " ");
        self.push_slicer(sheet, slicer)
    }

    /// Change a slicer (recorded for undo). Its name is changed with
    /// `rename_slicer`.
    pub fn update_slicer(&mut self, id: u32, f: impl FnOnce(&mut Slicer)) -> bool {
        let Some((si, _)) = self.slicer(id) else {
            return false;
        };
        self.snapshot();
        if let Some(slicer) = self.sheets[si].slicers.iter_mut().find(|s| s.id == id) {
            let (keep_id, keep_name) = (slicer.id, slicer.name.clone());
            f(slicer);
            slicer.id = keep_id;
            slicer.name = keep_name;
            slicer.columns = slicer.columns.clamp(1, 20_000);
        }
        true
    }

    pub fn rename_slicer(&mut self, id: u32, name: &str) -> Result<(), SlicerError> {
        let name = name.trim();
        let (si, current) = self.slicer(id).ok_or(SlicerError::NotFound)?;
        if current.name == name {
            return Ok(());
        }
        let taken = self
            .slicers()
            .filter(|(_, s)| s.id != id)
            .map(|(_, s)| &s.name)
            .chain(self.timelines().map(|(_, t)| &t.name))
            .any(|other| other.to_lowercase() == name.to_lowercase());
        if name.is_empty() || taken {
            return Err(SlicerError::InvalidName);
        }
        self.snapshot();
        if let Some(slicer) = self.sheets[si].slicers.iter_mut().find(|s| s.id == id) {
            slicer.name = name.to_string();
        }
        Ok(())
    }

    /// Delete a slicer. What it filtered stays filtered.
    pub fn remove_slicer(&mut self, id: u32) -> bool {
        let Some((si, _)) = self.slicer(id) else {
            return false;
        };
        self.snapshot();
        self.sheets[si].slicers.retain(|s| s.id != id);
        true
    }

    /// The table column a slicer filters: sheet, table id, offset of the
    /// column in the table.
    fn slicer_column(&self, slicer: &Slicer) -> Option<(usize, u32, u32)> {
        let SlicerSource::Table { table, column } = &slicer.source else {
            return None;
        };
        let (si, t) = self.table_by_id(*table)?;
        Some((si, *table, t.column_index(column)? as u32))
    }

    fn column_items(&self, sheet: usize, table: u32, col_id: u32) -> ColumnItems {
        let mut out = ColumnItems {
            items: Vec::new(),
            days: Vec::new(),
        };
        let Some(range) = self.table(sheet, table).and_then(|t| t.column_range(col_id as usize)) else {
            return out;
        };
        let mut seen: HashMap<String, ()> = HashMap::new();
        for row in range.r0..=range.r1 {
            let value = self.value_at(sheet, row, range.c0);
            let text = self.display_text(sheet, row, range.c0).text;
            if seen.insert(text.to_lowercase(), ()).is_some() {
                continue;
            }
            let dated = matches!(
                format::format_kind(&self.style_at(sheet, row, range.c0).num_fmt),
                FormatKind::Date | FormatKind::DateTime
            );
            out.days.push(match &value {
                Value::Number(n) if dated && !text.is_empty() => {
                    let (y, m, d) = serial_to_ymd(*n);
                    Some((y as i32, m, d))
                }
                _ => None,
            });
            out.items.push((text, value));
        }
        out
    }

    /// The buttons of a slicer, in the order it shows them.
    pub fn slicer_items(&self, id: u32) -> Vec<SlicerItem> {
        let Some((_, slicer)) = self.slicer(id) else {
            return Vec::new();
        };
        let locale = self.locale;
        // Label, value, natural rank, selected, has data.
        let mut items: Vec<(SlicerItem, Value, bool)> = Vec::new();
        match &slicer.source {
            SlicerSource::Pivot { pivots, field } => {
                let Some((p, f)) = self.sliced_pivot(pivots, field) else {
                    return Vec::new();
                };
                let with_data = p.items_with_data(f);
                let page = p.page_fields.iter().find(|pf| pf.field == f).and_then(|pf| pf.item);
                // Periods and intervals keep the order of the calendar.
                let ranged = p.cache.fields[f].group.as_ref().is_some_and(|g| g.kind != GroupKind::Discrete);
                for it in &p.fields[f].items {
                    let value = p.cache.items(f).get(it.x as usize).cloned().unwrap_or(Value::Empty);
                    items.push((
                        SlicerItem {
                            x: it.x,
                            label: p.item_text(f, it.x, locale),
                            selected: match page {
                                Some(only) => only == it.x,
                                None => !it.hidden,
                            },
                            has_data: with_data.get(it.x as usize).copied().unwrap_or(false),
                        },
                        if ranged { Value::Number(it.x as f64) } else { value },
                        false,
                    ));
                }
            }
            SlicerSource::Table { .. } => {
                let Some((si, table, col_id)) = self.slicer_column(slicer) else {
                    return Vec::new();
                };
                let column = self.column_items(si, table, col_id);
                let scope = FilterScope::Table(table);
                let dropdown = self.filter_dropdown(si, scope, col_id);
                let own = self.auto_filter(si, scope).and_then(|f| f.columns.get(&col_id));
                for (x, ((text, value), day)) in column.items.iter().zip(&column.days).enumerate() {
                    // What the button of the column lists is what the
                    // other columns' filters let through.
                    let listed: Option<bool> = match (&dropdown, day) {
                        (None, _) => Some(true),
                        (Some(d), _) if text.is_empty() => d.has_blanks.then_some(d.blanks_checked),
                        (Some(d), Some((y, m, dd))) => d
                            .dates
                            .iter()
                            .find(|year| year.year == *y)
                            .and_then(|year| year.months.iter().find(|month| month.month == *m))
                            .and_then(|month| month.days.iter().find(|day| day.day == *dd))
                            .map(|day| day.checked),
                        (Some(d), None) => {
                            d.items.iter().find(|i| i.text.to_lowercase() == text.to_lowercase()).map(|i| i.checked)
                        }
                    };
                    let selected = listed.unwrap_or_else(|| match own {
                        None => true,
                        Some(Criteria::Values(v)) => match day {
                            _ if text.is_empty() => v.blanks,
                            Some((y, m, d)) => v.dates.contains(&DateGroup::day(*y, *m, *d)),
                            None => v.values.iter().any(|t| t.to_lowercase() == text.to_lowercase()),
                        },
                        Some(_) => false,
                    });
                    let label = if text.is_empty() {
                        pivot::caption(Caption::Blank, locale).to_string()
                    } else {
                        text.clone()
                    };
                    items.push((
                        SlicerItem {
                            x: x as u32,
                            label,
                            selected,
                            has_data: listed.is_some(),
                        },
                        value.clone(),
                        text.is_empty(),
                    ));
                }
            }
        }
        // A custom list that holds every label gives the order.
        let list = slicer.custom_lists.then(|| self.custom_lists()).and_then(|lists| {
            lists.into_iter().find(|list| {
                items
                    .iter()
                    .all(|(it, _, blank)| *blank || list.iter().any(|entry| entry.to_lowercase() == it.label.to_lowercase()))
            })
        });
        let rank = |label: &str| list.as_ref().and_then(|l| l.iter().position(|e| e.to_lowercase() == label.to_lowercase()));
        items.sort_by(|(a, av, ab), (b, bv, bb)| {
            // Blanks close the list whichever way it is sorted.
            ab.cmp(bb).then_with(|| {
                let order = match (rank(&a.label), rank(&b.label)) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    _ => match (av, bv) {
                        (Value::Empty, Value::Empty) => std::cmp::Ordering::Equal,
                        (Value::Empty, _) => std::cmp::Ordering::Greater,
                        (_, Value::Empty) => std::cmp::Ordering::Less,
                        _ => sort_order(av, bv, false).then_with(|| collate(&a.label, &b.label, false)),
                    },
                };
                if slicer.descending { order.reverse() } else { order }
            })
        });
        let mut out: Vec<SlicerItem> = items.into_iter().map(|(it, _, _)| it).collect();
        if slicer.hide_no_data {
            out.retain(|it| it.has_data);
        } else if slicer.indicate_no_data && slicer.no_data_last {
            out.sort_by_key(|it| !it.has_data);
        }
        out
    }

    /// True when the slicer leaves items out ("Clear Filter" is lit).
    pub fn slicer_filtered(&self, id: u32) -> bool {
        let Some((_, slicer)) = self.slicer(id) else {
            return false;
        };
        match &slicer.source {
            SlicerSource::Pivot { pivots, field } => self.sliced_pivot(pivots, field).is_some_and(|(p, f)| {
                p.fields[f].items.iter().any(|it| it.hidden)
                    || p.page_fields.iter().any(|pf| pf.field == f && pf.item.is_some())
            }),
            SlicerSource::Table { .. } => self.slicer_column(slicer).is_some_and(|(si, table, col_id)| {
                self.auto_filter(si, FilterScope::Table(table)).is_some_and(|f| f.columns.contains_key(&col_id))
            }),
        }
    }

    /// Show exactly the items `selected` (see `SlicerItem::x`) in what the
    /// slicer filters; none, or all of them, clears the filter. With
    /// `overwrite`, the reports may cover cells holding data.
    pub fn slicer_select(&mut self, id: u32, selected: &[u32], overwrite: bool) -> Result<(), PivotError> {
        let (_, slicer) = self.slicer(id).ok_or(PivotError::NoSuchPivot)?;
        match slicer.source.clone() {
            SlicerSource::Pivot { pivots, field } => {
                let live: Vec<u32> = pivots.into_iter().filter(|p| self.pivot(*p).is_some()).collect();
                let all = selected.is_empty();
                self.pivot_update_many(&live, overwrite, |p| {
                    let Some(f) = p.cache.field_index(&field) else {
                        return;
                    };
                    if all {
                        p.set_page_item(f, None);
                    } else {
                        p.set_visible_items(f, selected);
                    }
                    if !p.sliced.contains(&f) {
                        p.sliced.push(f);
                    }
                })
            }
            SlicerSource::Table { .. } => {
                let (si, table, col_id) = self.slicer_column(slicer).ok_or(PivotError::NoSuchField)?;
                let column = self.column_items(si, table, col_id);
                let all = selected.is_empty() || (0..column.items.len() as u32).all(|x| selected.contains(&x));
                let criteria = (!all).then(|| {
                    let mut filter = ValueFilter::default();
                    for x in selected {
                        match (column.items.get(*x as usize), column.days.get(*x as usize)) {
                            (Some((text, _)), _) if text.is_empty() => filter.blanks = true,
                            (Some(_), Some(Some((y, m, d)))) => filter.dates.push(DateGroup::day(*y, *m, *d)),
                            (Some((text, _)), _) => filter.values.push(text.clone()),
                            _ => {}
                        }
                    }
                    Criteria::Values(filter)
                });
                // A table whose buttons are off has no filter to set.
                if criteria.is_some() && self.auto_filter(si, FilterScope::Table(table)).is_none() {
                    self.set_table_option(si, table, TableOption::FilterButton, true)
                        .map_err(|_| PivotError::NoSuchField)?;
                }
                if self.auto_filter(si, FilterScope::Table(table)).is_some() {
                    self.set_filter_criteria(si, FilterScope::Table(table), col_id, criteria);
                }
                Ok(())
            }
        }
    }

    /// Connect a slicer to these PivotTables and to no other ("Report
    /// Connections"). Those that join take the slicer's selection; those
    /// that leave keep theirs.
    pub fn slicer_connect(&mut self, id: u32, pivots: &[u32], overwrite: bool) -> Result<(), PivotError> {
        let (si, slicer) = self.slicer(id).ok_or(PivotError::NoSuchPivot)?;
        let SlicerSource::Pivot { pivots: old, field } = slicer.source.clone() else {
            return Err(PivotError::NoSuchPivot);
        };
        let joining: Vec<u32> = pivots.iter().copied().filter(|p| !old.contains(p)).collect();
        // What the slicer shows now is what the newcomers must show.
        let state = self.sliced_pivot(&old, &field).map(|(p, f)| {
            let page = p.page_fields.iter().find(|pf| pf.field == f).and_then(|pf| pf.item);
            let values = p.cache.items(f);
            p.fields[f]
                .items
                .iter()
                .filter(|it| page.map_or(!it.hidden, |only| only == it.x))
                .filter_map(|it| values.get(it.x as usize).cloned())
                .collect::<Vec<Value>>()
        });
        self.pivot_update_many(&joining, overwrite, |p| {
            let Some(f) = p.cache.field_index(&field) else {
                return;
            };
            if let Some(shown) = &state {
                let visible: Vec<u32> = p
                    .cache
                    .items(f)
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| shown.contains(v))
                    .map(|(x, _)| x as u32)
                    .collect();
                p.set_visible_items(f, &visible);
            }
            if !p.sliced.contains(&f) {
                p.sliced.push(f);
            }
        })?;
        self.snapshot();
        if let Some(slicer) = self.sheets[si].slicers.iter_mut().find(|s| s.id == id) {
            slicer.source = SlicerSource::Pivot {
                pivots: pivots.to_vec(),
                field,
            };
        }
        Ok(())
    }

    // ---- timelines ---------------------------------------------------------

    /// Put a timeline for a date field of a PivotTable on `sheet`.
    pub fn add_timeline(&mut self, sheet: usize, pivot: u32, field: usize, anchor: (u32, u32)) -> Result<u32, SlicerError> {
        let (_, p) = self.pivot(pivot).ok_or(SlicerError::NotFound)?;
        if sheet >= self.sheets.len() {
            return Err(SlicerError::NotFound);
        }
        let cf = p.cache.fields.get(field).ok_or(SlicerError::NotFound)?;
        if !cf.is_database() || !p.cache.is_date(field) {
            return Err(SlicerError::NotADate);
        }
        let caption = p.field_caption(field).to_string();
        let id = self.timelines().map(|(_, t)| t.id).max().unwrap_or(0) + 1;
        let timeline = Timeline {
            id,
            name: unique(caption.clone(), &self.slicer_names(), " "),
            cache_name: unique(cache_name("NativeTimeline_", &caption), &self.slicer_cache_names(), ""),
            caption,
            pivots: vec![pivot],
            field: cf.name.clone(),
            anchor,
            offset: (0.0, 0.0),
            size: TIMELINE_SIZE,
            show_header: true,
            show_selection_label: true,
            show_time_level: true,
            show_scrollbar: true,
            level: TimeLevel::Months,
            style: "TimeSlicerStyleLight1".to_string(),
            scroll: None,
        };
        self.snapshot();
        self.sheets[sheet].timelines.push(timeline);
        Ok(id)
    }

    pub fn paste_timeline(&mut self, sheet: usize, mut timeline: Timeline) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        timeline.id = self.timelines().map(|(_, t)| t.id).max().unwrap_or(0) + 1;
        timeline.name = unique(timeline.name, &self.slicer_names(), " ");
        let id = timeline.id;
        self.snapshot();
        self.sheets[sheet].timelines.push(timeline);
        Some(id)
    }

    /// Change a timeline (recorded for undo); `rename_timeline` changes
    /// its name.
    pub fn update_timeline(&mut self, id: u32, f: impl FnOnce(&mut Timeline)) -> bool {
        let Some((si, _)) = self.timeline(id) else {
            return false;
        };
        self.snapshot();
        if let Some(timeline) = self.sheets[si].timelines.iter_mut().find(|t| t.id == id) {
            let (keep_id, keep_name) = (timeline.id, timeline.name.clone());
            f(timeline);
            timeline.id = keep_id;
            timeline.name = keep_name;
        }
        true
    }

    pub fn rename_timeline(&mut self, id: u32, name: &str) -> Result<(), SlicerError> {
        let name = name.trim();
        let (si, current) = self.timeline(id).ok_or(SlicerError::NotFound)?;
        if current.name == name {
            return Ok(());
        }
        let taken = self
            .timelines()
            .filter(|(_, t)| t.id != id)
            .map(|(_, t)| &t.name)
            .chain(self.slicers().map(|(_, s)| &s.name))
            .any(|other| other.to_lowercase() == name.to_lowercase());
        if name.is_empty() || taken {
            return Err(SlicerError::InvalidName);
        }
        self.snapshot();
        if let Some(timeline) = self.sheets[si].timelines.iter_mut().find(|t| t.id == id) {
            timeline.name = name.to_string();
        }
        Ok(())
    }

    /// Delete a timeline. Its date filter stays.
    pub fn remove_timeline(&mut self, id: u32) -> bool {
        let Some((si, _)) = self.timeline(id) else {
            return false;
        };
        self.snapshot();
        self.sheets[si].timelines.retain(|t| t.id != id);
        true
    }

    /// The days the band of a timeline runs over: the whole years of the
    /// dates of its field.
    pub fn timeline_bounds(&self, id: u32) -> Option<(f64, f64)> {
        let (_, timeline) = self.timeline(id)?;
        let (p, f) = self.sliced_pivot(&timeline.pivots, &timeline.field)?;
        let dates = p.cache.items(f).iter().filter_map(|v| match v {
            Value::Number(n) => Some(n.floor()),
            _ => None,
        });
        let (lo, hi) = dates.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)));
        if lo > hi {
            return None;
        }
        Some((first_of_month(serial_to_ymd(lo).0, 1), first_of_month(serial_to_ymd(hi).0 + 1, 1) - 1.0))
    }

    /// The days the timeline limits its field to, first and last
    /// included; `None` for all periods.
    pub fn timeline_selection(&self, id: u32) -> Option<(f64, f64)> {
        let (_, timeline) = self.timeline(id)?;
        let (p, f) = self.sliced_pivot(&timeline.pivots, &timeline.field)?;
        p.date_filter(f)
    }

    /// Limit the field of a timeline to the days `start..=end` in every
    /// PivotTable it is connected to; `None` clears the filter.
    pub fn timeline_select(&mut self, id: u32, days: Option<(f64, f64)>, overwrite: bool) -> Result<(), PivotError> {
        let (_, timeline) = self.timeline(id).ok_or(PivotError::NoSuchPivot)?;
        let field = timeline.field.clone();
        let live: Vec<u32> = timeline.pivots.iter().copied().filter(|p| self.pivot(*p).is_some()).collect();
        let days = days.map(|(a, b)| (a.min(b).floor(), a.max(b).floor()));
        self.pivot_update_many(&live, overwrite, |p| {
            if let Some(f) = p.cache.field_index(&field) {
                p.set_date_filter(f, days);
            }
        })
    }

    /// "Report Connections" of a timeline: see `slicer_connect`.
    pub fn timeline_connect(&mut self, id: u32, pivots: &[u32], overwrite: bool) -> Result<(), PivotError> {
        let (si, timeline) = self.timeline(id).ok_or(PivotError::NoSuchPivot)?;
        let (old, field) = (timeline.pivots.clone(), timeline.field.clone());
        let days = self.timeline_selection(id);
        let joining: Vec<u32> = pivots.iter().copied().filter(|p| !old.contains(p)).collect();
        self.pivot_update_many(&joining, overwrite, |p| {
            if let Some(f) = p.cache.field_index(&field) {
                p.set_date_filter(f, days);
            }
        })?;
        self.snapshot();
        if let Some(timeline) = self.sheets[si].timelines.iter_mut().find(|t| t.id == id) {
            timeline.pivots = pivots.to_vec();
        }
        Ok(())
    }
}

// ---- hooks -----------------------------------------------------------------

fn shift(pos: u32, insert: bool, at: u32, count: u32) -> u32 {
    if insert {
        if pos >= at { pos + count } else { pos }
    } else if pos >= at + count {
        pos - count
    } else {
        pos.min(at)
    }
}

/// See `features::on_structural_edit`: slicers and timelines move with the
/// cell their corner is in, like charts.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if !sheet.name.eq_ignore_ascii_case(edited_sheet) {
        return;
    }
    let anchors = sheet
        .slicers
        .iter_mut()
        .map(|s| &mut s.anchor)
        .chain(sheet.timelines.iter_mut().map(|t| &mut t.anchor));
    for anchor in anchors {
        match edit {
            StructuralEdit::InsertRows { at, count } => anchor.0 = shift(anchor.0, true, at, count),
            StructuralEdit::DeleteRows { at, count } => anchor.0 = shift(anchor.0, false, at, count),
            StructuralEdit::InsertCols { at, count } => anchor.1 = shift(anchor.1, true, at, count),
            StructuralEdit::DeleteCols { at, count } => anchor.1 = shift(anchor.1, false, at, count),
        }
    }
}

/// See `features::on_sheet_duplicated`. The PivotTables and the tables of
/// the copy got ids of their own, in the order of the originals: the
/// copied slicers and timelines follow them, under names of their own.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    let Some(from) = wb.sheet_index(original) else {
        return;
    };
    let pair = |old: Vec<u32>, new: Vec<u32>| -> HashMap<u32, u32> { old.into_iter().zip(new).collect() };
    let pivots = pair(
        wb.sheets[from].pivots.iter().map(|p| p.id).collect(),
        wb.sheets[copy].pivots.iter().map(|p| p.id).collect(),
    );
    let tables = pair(
        wb.sheets[from].tables.iter().map(|t| t.id).collect(),
        wb.sheets[copy].tables.iter().map(|t| t.id).collect(),
    );
    let mut names = wb.slicer_names();
    let mut caches = wb.slicer_cache_names();
    let mut next_slicer = wb.slicers().map(|(_, s)| s.id).max().unwrap_or(0);
    let mut next_timeline = wb.timelines().map(|(_, t)| t.id).max().unwrap_or(0);
    let sh = &mut wb.sheets[copy];
    for slicer in &mut sh.slicers {
        next_slicer += 1;
        slicer.id = next_slicer;
        slicer.name = unique(slicer.name.clone(), &names, " ");
        slicer.cache_name = unique(slicer.cache_name.clone(), &caches, "");
        names.push(slicer.name.clone());
        caches.push(slicer.cache_name.clone());
        match &mut slicer.source {
            SlicerSource::Pivot { pivots: ids, .. } => {
                for id in ids {
                    *id = pivots.get(id).copied().unwrap_or(*id);
                }
            }
            SlicerSource::Table { table, .. } => *table = tables.get(table).copied().unwrap_or(*table),
        }
    }
    for timeline in &mut sh.timelines {
        next_timeline += 1;
        timeline.id = next_timeline;
        timeline.name = unique(timeline.name.clone(), &names, " ");
        timeline.cache_name = unique(timeline.cache_name.clone(), &caches, "");
        names.push(timeline.name.clone());
        caches.push(timeline.cache_name.clone());
        for id in &mut timeline.pivots {
            *id = pivots.get(id).copied().unwrap_or(*id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pivot::{Area, PivotDestination, PivotSource};
    use crate::refs::Rect;

    const SALES: &[[&str; 4]] = &[
        ["Region", "Product", "Date", "Sales"],
        ["East", "Apples", "2023-01-15", "100"],
        ["East", "Pears", "2023-03-02", "150"],
        ["East", "Apples", "2024-02-10", "120"],
        ["West", "Apples", "2023-03-20", "200"],
        ["West", "Pears", "2024-06-01", "50"],
        ["North", "Plums", "2024-06-30", "70"],
    ];

    fn data() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        for (r, row) in SALES.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("data");
        wb
    }

    /// The data and a PivotTable of sales by region on a second sheet.
    fn pivot() -> (Workbook, u32) {
        let mut wb = data();
        let source = PivotSource::Range {
            sheet: wb.sheets[0].name.clone(),
            range: Rect::new(0, 0, 6, 3),
        };
        let id = wb.pivot_create(source, PivotDestination::NewSheet { index: 1 }, false).unwrap();
        wb.pivot_update(id, false, |p| {
            p.add_field(0, Area::Rows, None, Locale::en());
            p.add_field(3, Area::Values, None, Locale::en());
        })
        .unwrap();
        wb.commit("pivot");
        (wb, id)
    }

    fn column(wb: &Workbook, sheet: usize, col: u32, rows: std::ops::RangeInclusive<u32>) -> Vec<String> {
        rows.map(|r| wb.display_text(sheet, r, col).text).collect()
    }

    fn labels(wb: &Workbook, id: u32) -> Vec<(String, bool, bool)> {
        wb.slicer_items(id).into_iter().map(|i| (i.label, i.selected, i.has_data)).collect()
    }

    #[test]
    fn a_slicer_filters_its_pivot_and_shows_filters_made_elsewhere() {
        let (mut wb, pivot) = pivot();
        let id = wb.add_pivot_slicer(1, pivot, 0, (2, 5)).unwrap();
        wb.commit("slicer");
        let slicer = wb.slicer(id).unwrap().1;
        assert_eq!((slicer.name.as_str(), slicer.cache_name.as_str()), ("Region", "Slicer_Region"));
        assert_eq!(
            labels(&wb, id),
            [("East".to_string(), true, true), ("North".to_string(), true, true), ("West".to_string(), true, true)]
        );
        assert!(!wb.slicer_filtered(id));

        let west = wb.slicer_items(id)[2].x;
        wb.slicer_select(id, &[west], false).unwrap();
        wb.commit("slice");
        assert_eq!(column(&wb, 1, 0, 2..=4), ["Row Labels", "West", "Grand Total"]);
        assert_eq!(wb.display_text(1, 4, 1).text, "250");
        assert!(wb.slicer_filtered(id));
        assert_eq!(labels(&wb, id)[0], ("East".to_string(), false, true));

        // The drop-down of the field changes what the slicer shows.
        wb.pivot_update(pivot, false, |p| p.set_item_hidden(0, 0, false)).unwrap();
        wb.commit("filter");
        assert_eq!(labels(&wb, id).iter().filter(|(_, selected, _)| *selected).count(), 2);

        // One undo step per selection; clearing shows everything again.
        assert!(wb.undo());
        assert_eq!(labels(&wb, id).iter().filter(|(_, selected, _)| *selected).count(), 1);
        wb.slicer_select(id, &[], false).unwrap();
        wb.commit("clear");
        assert!(!wb.slicer_filtered(id));
        assert_eq!(column(&wb, 1, 0, 3..=5), ["East", "North", "West"]);

        // Deleting the slicer leaves the filter in place.
        wb.slicer_select(id, &[west], false).unwrap();
        assert!(wb.remove_slicer(id));
        wb.commit("delete");
        assert_eq!(column(&wb, 1, 0, 3..=4), ["West", "Grand Total"]);
    }

    #[test]
    fn a_slicer_filters_a_field_that_is_not_in_the_report() {
        let (mut wb, pivot) = pivot();
        let product = wb.add_pivot_slicer(1, pivot, 1, (2, 5)).unwrap();
        let region = wb.add_pivot_slicer(1, pivot, 0, (2, 8)).unwrap();
        assert_eq!(wb.slicer(region).unwrap().1.name, "Region");
        let plums = wb.slicer_items(product).iter().find(|i| i.label == "Plums").unwrap().x;
        wb.slicer_select(product, &[plums], false).unwrap();
        wb.commit("slice");
        assert_eq!(column(&wb, 1, 0, 3..=4), ["North", "Grand Total"]);
        assert_eq!(wb.display_text(1, 4, 1).text, "70");
        // The regions without plums are greyed out and listed last.
        assert_eq!(
            labels(&wb, region),
            [("North".to_string(), true, true), ("East".to_string(), true, false), ("West".to_string(), true, false)]
        );
        wb.update_slicer(region, |s| s.hide_no_data = true);
        assert_eq!(labels(&wb, region).len(), 1);
        wb.update_slicer(region, |s| {
            s.hide_no_data = false;
            s.no_data_last = false;
            s.descending = true;
        });
        let order: Vec<String> = labels(&wb, region).into_iter().map(|(label, ..)| label).collect();
        assert_eq!(order, ["West", "North", "East"]);
    }

    #[test]
    fn one_slicer_drives_the_pivots_sharing_its_cache() {
        let (mut wb, first) = pivot();
        let source = wb.pivot(first).unwrap().1.source.clone();
        let second = wb.pivot_create(source, PivotDestination::Existing { sheet: 1, row: 2, col: 10 }, false).unwrap();
        wb.pivot_update(second, false, |p| {
            p.add_field(1, Area::Rows, None, Locale::en());
            p.add_field(3, Area::Values, None, Locale::en());
        })
        .unwrap();
        wb.commit("second");
        assert_eq!(wb.pivots_sharing_cache(first), [(1, first), (1, second)]);

        let id = wb.add_pivot_slicer(1, first, 0, (10, 0)).unwrap();
        let east = wb.slicer_items(id)[0].x;
        wb.slicer_select(id, &[east], false).unwrap();
        wb.commit("slice");
        // Not connected yet: the second report still shows every product.
        assert_eq!(column(&wb, 1, 10, 3..=6), ["Apples", "Pears", "Plums", "Grand Total"]);
        wb.slicer_connect(id, &[first, second], false).unwrap();
        wb.commit("connect");
        assert_eq!(column(&wb, 1, 10, 3..=5), ["Apples", "Pears", "Grand Total"]);
        assert_eq!(wb.display_text(1, 5, 11).text, "370");
        wb.slicer_select(id, &[], false).unwrap();
        wb.commit("clear");
        assert_eq!(wb.display_text(1, 6, 11).text, "690");
        assert!(wb.undo() && wb.undo());
        assert_eq!(wb.slicer(id).unwrap().1.pivots(), [first]);
    }

    #[test]
    fn a_slicer_filters_a_table_column() {
        let mut wb = data();
        let table = wb.add_table(0, Rect::new(0, 0, 6, 3), true).unwrap();
        wb.commit("table");
        let region = wb.add_table_slicer(0, table, 0, (0, 6)).unwrap();
        let product = wb.add_table_slicer(0, table, 1, (0, 9)).unwrap();
        wb.commit("slicers");
        let east = wb.slicer_items(region).iter().find(|i| i.label == "East").unwrap().x;
        wb.slicer_select(region, &[east], false).unwrap();
        wb.commit("slice");
        let hidden: Vec<u32> = wb.sheets[0].filtered_rows.iter().copied().collect();
        assert_eq!(hidden, [4, 5, 6]);
        assert!(wb.slicer_filtered(region));
        // The other slicer greys what the first one leaves no row for.
        assert_eq!(
            labels(&wb, product),
            [("Apples".to_string(), true, true), ("Pears".to_string(), true, true), ("Plums".to_string(), true, false)]
        );
        // The button of the column and the slicer show the same filter.
        assert!(wb.set_filter_criteria(0, FilterScope::Table(table), 0, None));
        wb.commit("clear");
        assert!(!wb.slicer_filtered(region));
        assert!(labels(&wb, region).iter().all(|(_, selected, data)| *selected && *data));
        // Dates are chosen by day.
        let date = wb.add_table_slicer(0, table, 2, (0, 12)).unwrap();
        let first = wb.slicer_items(date)[0].clone();
        wb.slicer_select(date, &[first.x], false).unwrap();
        wb.commit("date");
        assert_eq!(wb.sheets[0].filtered_rows.len(), 5);
        assert!(wb.slicer_items(date)[0].selected && !wb.slicer_items(date)[1].selected);
    }

    #[test]
    fn slicers_keep_unique_names_and_follow_rows_and_copies() {
        let (mut wb, pivot) = pivot();
        let a = wb.add_pivot_slicer(1, pivot, 0, (4, 5)).unwrap();
        let b = wb.add_pivot_slicer(1, pivot, 0, (4, 8)).unwrap();
        wb.commit("slicers");
        let second = wb.slicer(b).unwrap().1;
        assert_eq!((second.name.as_str(), second.cache_name.as_str()), ("Region 1", "Slicer_Region1"));
        assert_eq!(wb.rename_slicer(b, "region"), Err(SlicerError::InvalidName));
        assert_eq!(wb.rename_slicer(b, "Zones"), Ok(()));
        wb.insert_rows(1, 0, 2);
        wb.commit("rows");
        assert_eq!(wb.slicer(a).unwrap().1.anchor, (6, 5));
        let copy = wb.duplicate_sheet(1).unwrap();
        wb.commit("copy");
        let copied = &wb.sheets[copy].slicers[0];
        assert_ne!(copied.id, a);
        assert_eq!(copied.name, "Region 1");
        assert_eq!(copied.pivots(), [wb.sheets[copy].pivots[0].id]);
    }

    #[test]
    fn a_timeline_limits_the_dates_of_its_pivots() {
        let (mut wb, pivot) = pivot();
        assert_eq!(wb.add_timeline(1, pivot, 0, (8, 0)), Err(SlicerError::NotADate));
        let id = wb.add_timeline(1, pivot, 2, (8, 0)).unwrap();
        wb.commit("timeline");
        let timeline = wb.timeline(id).unwrap().1;
        assert_eq!((timeline.name.as_str(), timeline.cache_name.as_str()), ("Date", "NativeTimeline_Date"));
        let (start, end) = wb.timeline_bounds(id).unwrap();
        assert_eq!((serial_to_ymd(start), serial_to_ymd(end)), ((2023, 1, 1), (2024, 12, 31)));
        assert_eq!(wb.timeline_selection(id), None);

        // March 2023: one sale in the East, one in the West.
        let months = periods(TimeLevel::Months, start, end, Locale::en());
        assert_eq!(months.len(), 24);
        assert_eq!((months[2].label.as_str(), months[0].group.as_deref()), ("MAR", Some("2023")));
        wb.timeline_select(id, Some((months[2].start, months[2].end)), false).unwrap();
        wb.commit("march");
        assert_eq!(column(&wb, 1, 0, 3..=5), ["East", "West", "Grand Total"]);
        assert_eq!(wb.display_text(1, 5, 1).text, "350");
        assert_eq!(wb.timeline_selection(id), Some((months[2].start, months[2].end)));
        assert_eq!(span_label(TimeLevel::Months, months[2].start, months[2].end, Locale::en()), "Mar 2023");
        assert_eq!(span_label(TimeLevel::Quarters, months[0].start, months[5].end, Locale::fr()), "T1 - T2 2023");

        wb.timeline_select(id, None, false).unwrap();
        wb.commit("clear");
        assert_eq!(wb.display_text(1, 6, 1).text, "690");
        assert!(wb.undo());
        assert_eq!(wb.display_text(1, 5, 1).text, "350");
        // The filter outlives its timeline.
        assert!(wb.remove_timeline(id));
        wb.commit("delete");
        assert_eq!(wb.display_text(1, 5, 1).text, "350");
    }

    #[test]
    fn the_levels_of_a_timeline() {
        let en = Locale::en();
        let start = date_to_serial(2023, 11, 20).unwrap();
        let end = date_to_serial(2024, 2, 3).unwrap();
        let years: Vec<String> = periods(TimeLevel::Years, start, end, en).into_iter().map(|p| p.label).collect();
        assert_eq!(years, ["2023", "2024"]);
        let quarters = periods(TimeLevel::Quarters, start, end, en);
        assert_eq!(quarters.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), ["Q4", "Q1"]);
        assert_eq!(quarters[1].group.as_deref(), Some("2024"));
        assert_eq!(serial_to_ymd(quarters[1].end), (2024, 3, 31));
        let days = periods(TimeLevel::Days, start, end, en);
        assert_eq!(days.len(), 76);
        assert_eq!(days[11].group.as_deref(), Some("DEC 2023"));
        assert_eq!(span_label(TimeLevel::Days, start, start + 2.0, en), "20 - 22 Nov 2023");
        assert_eq!(span_label(TimeLevel::Years, start, end, en), "2023 - 2024");
    }
}
