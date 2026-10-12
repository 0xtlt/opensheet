//! AutoFilter: the filter buttons of a range (Data > Filter) and of table
//! headers, the criteria of each column and the rows they hide
//! (`Sheet::filtered_rows`).
//!
//! The model follows the `autoFilter` element of xlsx files: at most one
//! criterion per column (`filterColumn`), which is a list of checked
//! values, one or two custom conditions, a top / bottom N, a dynamic filter
//! (average, date periods) or a colour. A sheet has one filter of its own
//! (`Sheet::auto_filter`) and one per table (`Table::auto_filter`); a row
//! is hidden as soon as one of them rejects it.
//!
//! Criteria are evaluated when they are set and on "Reapply", never behind
//! the user's back when cells change, as in Excel.

mod advanced;

pub use advanced::{AdvancedFilter, AdvancedFilterError};

use crate::ast::StructuralEdit;
use crate::datetime::{date_to_serial, now_serial, serial_to_hms, serial_to_ymd, weekday};
use crate::eval::compare_text;
use crate::features::CellShift;
use crate::format::{self, FormatKind};
use crate::functions::wildcard_match;
use crate::input::parse_numeric_text;
use crate::locale::Locale;
use crate::ops::{SortError, SortLevel, SortOn, SortSpec, shift_set, sort_order};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::Color;
use crate::table::TableOption;
use crate::value::Value;
use crate::workbook::{Content, Sheet, Workbook};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The drop-down list stops at this many distinct values, as in Excel.
pub const MAX_FILTER_ITEMS: usize = 10_000;

/// Which filter of a sheet: its own (Data > Filter) or the one of a table,
/// by table id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterScope {
    Sheet,
    Table(u32),
}

/// The filter of a range: its buttons sit on the first row of `range`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoFilter {
    /// Header row and data rows.
    pub range: Rect,
    /// The criterion of each filtered column, keyed by the column's offset
    /// from the left of `range` (`colId` in files).
    pub columns: BTreeMap<u32, Criteria>,
    /// Columns (same offsets) whose button is not drawn (`hiddenButton`).
    pub hidden_buttons: BTreeSet<u32>,
    /// The last sort made through the filter (`sortState`): it puts the
    /// arrow on the button.
    pub sort: Option<SortState>,
}

/// How a filtered range was last sorted. The keys of the levels are column
/// offsets from the left of the filter's range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SortState {
    pub levels: Vec<SortLevel>,
    pub case_sensitive: bool,
}

/// What a column is filtered on (the forms of `filterColumn`).
#[derive(Clone, Debug, PartialEq)]
pub enum Criteria {
    /// The values checked in the list (`filters`).
    Values(ValueFilter),
    /// One or two comparisons (`customFilters`).
    Custom(CustomFilter),
    /// The N largest or smallest numbers (`top10`).
    Top10(Top10),
    /// Above / below average and the date periods (`dynamicFilter`).
    Dynamic(DynamicFilter),
    /// A fill or font colour (`colorFilter`).
    Color(ColorFilter),
}

/// The checked entries of the drop-down list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValueFilter {
    /// Checked values, as the cells display them (compared without case).
    pub values: Vec<String>,
    /// "(Blanks)" is checked.
    pub blanks: bool,
    /// Checked nodes of the date tree (`dateGroupItem`).
    pub dates: Vec<DateGroup>,
}

/// A node of the date tree: a whole year, a month of a year, a day... The
/// first `None` gives the depth (`dateTimeGrouping` in files).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateGroup {
    pub year: i32,
    pub month: Option<u32>,
    pub day: Option<u32>,
    pub hour: Option<u32>,
    pub minute: Option<u32>,
    pub second: Option<u32>,
}

impl DateGroup {
    pub fn year(year: i32) -> DateGroup {
        DateGroup {
            year,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
        }
    }

    pub fn month(year: i32, month: u32) -> DateGroup {
        DateGroup {
            month: Some(month),
            ..DateGroup::year(year)
        }
    }

    pub fn day(year: i32, month: u32, day: u32) -> DateGroup {
        DateGroup {
            day: Some(day),
            ..DateGroup::month(year, month)
        }
    }

    /// The depth of the node, as `dateTimeGrouping` names it in files.
    pub fn grouping(&self) -> &'static str {
        match (self.month, self.day, self.hour, self.minute, self.second) {
            (None, ..) => "year",
            (_, None, ..) => "month",
            (_, _, None, ..) => "day",
            (_, _, _, None, _) => "hour",
            (_, _, _, _, None) => "minute",
            _ => "second",
        }
    }

    /// True when the date-time `serial` falls in this group.
    pub fn matches(&self, serial: f64) -> bool {
        let (y, m, d) = serial_to_ymd(serial);
        if y != self.year as i64 || self.month.is_some_and(|x| x != m) || self.day.is_some_and(|x| x != d) {
            return false;
        }
        if self.hour.is_none() {
            return true;
        }
        let (h, min, s) = serial_to_hms(serial);
        self.hour == Some(h) && self.minute.is_none_or(|x| x == min) && self.second.is_none_or(|x| x == s)
    }
}

/// The operators files know (`customFilter operator`). "Begins with",
/// "contains"... are `Equal` / `NotEqual` with wildcards, see
/// [`Comparison`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomOp {
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

impl CustomOp {
    /// The `operator` attribute of `customFilter`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            CustomOp::Equal => "equal",
            CustomOp::NotEqual => "notEqual",
            CustomOp::Greater => "greaterThan",
            CustomOp::GreaterOrEqual => "greaterThanOrEqual",
            CustomOp::Less => "lessThan",
            CustomOp::LessOrEqual => "lessThanOrEqual",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<CustomOp> {
        Some(match name {
            "equal" => CustomOp::Equal,
            "notEqual" => CustomOp::NotEqual,
            "greaterThan" => CustomOp::Greater,
            "greaterThanOrEqual" => CustomOp::GreaterOrEqual,
            "lessThan" => CustomOp::Less,
            "lessThanOrEqual" => CustomOp::LessOrEqual,
            _ => return None,
        })
    }
}

/// One comparison of a custom filter. `value` is written as in files: a
/// number in plain notation ("1.5", a date as its serial number) or text,
/// where `?` stands for one character, `*` for any run and `~` escapes
/// them.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub op: CustomOp,
    pub value: String,
}

/// The comparisons the Custom AutoFilter dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Equals,
    NotEquals,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
    BeginsWith,
    NotBeginsWith,
    EndsWith,
    NotEndsWith,
    Contains,
    NotContains,
}

impl Condition {
    /// The condition for a comparison and what the user typed after it
    /// (numbers and dates in the notation of `locale`).
    pub fn new(comparison: Comparison, text: &str, locale: &Locale) -> Condition {
        let plain = || match parse_numeric_text(text, locale) {
            Some(n) => n.to_string(),
            None => text.to_string(),
        };
        let (op, value) = match comparison {
            Comparison::Equals => (CustomOp::Equal, plain()),
            Comparison::NotEquals => (CustomOp::NotEqual, plain()),
            Comparison::Greater => (CustomOp::Greater, plain()),
            Comparison::GreaterOrEqual => (CustomOp::GreaterOrEqual, plain()),
            Comparison::Less => (CustomOp::Less, plain()),
            Comparison::LessOrEqual => (CustomOp::LessOrEqual, plain()),
            Comparison::BeginsWith => (CustomOp::Equal, format!("{text}*")),
            Comparison::NotBeginsWith => (CustomOp::NotEqual, format!("{text}*")),
            Comparison::EndsWith => (CustomOp::Equal, format!("*{text}")),
            Comparison::NotEndsWith => (CustomOp::NotEqual, format!("*{text}")),
            Comparison::Contains => (CustomOp::Equal, format!("*{text}*")),
            Comparison::NotContains => (CustomOp::NotEqual, format!("*{text}*")),
        };
        Condition { op, value }
    }

    /// The comparison and the operand the dialog shows for this condition
    /// (the reverse of [`Condition::new`], the operand still as in files).
    pub fn comparison(&self) -> (Comparison, &str) {
        let v = self.value.as_str();
        let negated = match self.op {
            CustomOp::Equal => false,
            CustomOp::NotEqual => true,
            CustomOp::Greater => return (Comparison::Greater, v),
            CustomOp::GreaterOrEqual => return (Comparison::GreaterOrEqual, v),
            CustomOp::Less => return (Comparison::Less, v),
            CustomOp::LessOrEqual => return (Comparison::LessOrEqual, v),
        };
        let starts = v.len() > 1 && v.starts_with('*');
        let ends = v.len() > 1 && v.ends_with('*') && !v.ends_with("~*");
        let pick = |plain, not| if negated { not } else { plain };
        match (starts, ends) {
            (true, true) if v.len() > 2 => (pick(Comparison::Contains, Comparison::NotContains), &v[1..v.len() - 1]),
            (false, true) => (
                pick(Comparison::BeginsWith, Comparison::NotBeginsWith),
                &v[..v.len() - 1],
            ),
            (true, false) => (pick(Comparison::EndsWith, Comparison::NotEndsWith), &v[1..]),
            _ => (pick(Comparison::Equals, Comparison::NotEquals), v),
        }
    }

    /// The operand as the user would type it: numbers in the notation of
    /// `locale`, as a short date when the column holds dates.
    pub fn operand_text(&self, locale: &Locale, as_date: bool) -> String {
        let (_, operand) = self.comparison();
        match operand.parse::<f64>() {
            Ok(n) if as_date => format::format_number(n, locale.short_date_format, locale).text,
            Ok(n) => locale.number_to_text(n),
            Err(_) => operand.to_string(),
        }
    }

    fn number(&self) -> Option<f64> {
        self.value.trim().parse::<f64>().ok().filter(|n| n.is_finite())
    }

    /// Whether a cell holding `value`, displayed as `text`, passes.
    fn matches(&self, value: &Value, text: &str) -> bool {
        let number = self.number();
        let equal = || {
            if self.value.is_empty() {
                return text.is_empty();
            }
            matches!((number, value), (Some(n), Value::Number(x)) if *x == n) || wildcard_match(&self.value, text)
        };
        // Numbers are compared with numbers and text with text; a cell of
        // the other kind, or a blank one, is neither greater nor less.
        let order = || match (number, value) {
            (Some(n), Value::Number(x)) => x.partial_cmp(&n),
            (None, Value::Text(_)) => Some(compare_text(text, &self.value)),
            _ => None,
        };
        match self.op {
            CustomOp::Equal => equal(),
            CustomOp::NotEqual => !equal(),
            CustomOp::Greater => order() == Some(Ordering::Greater),
            CustomOp::GreaterOrEqual => order().is_some_and(|o| o != Ordering::Less),
            CustomOp::Less => order() == Some(Ordering::Less),
            CustomOp::LessOrEqual => order().is_some_and(|o| o != Ordering::Greater),
        }
    }
}

/// The Custom AutoFilter dialog: one condition, or two joined by And / Or.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomFilter {
    pub first: Condition,
    pub second: Option<Condition>,
    /// Both conditions must hold (And) rather than either (Or).
    pub and: bool,
}

impl CustomFilter {
    pub fn one(condition: Condition) -> CustomFilter {
        CustomFilter {
            first: condition,
            second: None,
            and: false,
        }
    }

    fn matches(&self, value: &Value, text: &str) -> bool {
        let first = self.first.matches(value, text);
        match &self.second {
            None => first,
            Some(second) if self.and => first && second.matches(value, text),
            Some(second) => first || second.matches(value, text),
        }
    }
}

/// The Top 10 dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Top10 {
    /// Top (largest) rather than bottom.
    pub top: bool,
    /// `value` is a percentage of the number of items, not a count.
    pub percent: bool,
    pub value: f64,
    /// The cut-off the filter last worked out (`filterVal` in files):
    /// numbers at least (top) or at most (bottom) this large pass.
    pub filter_value: Option<f64>,
}

impl Top10 {
    pub fn new(top: bool, percent: bool, value: f64) -> Top10 {
        Top10 {
            top,
            percent,
            value,
            filter_value: None,
        }
    }

    /// The cut-off among `numbers`: ties with the Nth item all pass.
    fn cut_off(&self, mut numbers: Vec<f64>) -> Option<f64> {
        if numbers.is_empty() {
            return None;
        }
        numbers.sort_by(f64::total_cmp);
        if self.top {
            numbers.reverse();
        }
        let wanted = if self.percent {
            numbers.len() as f64 * self.value / 100.0
        } else {
            self.value
        };
        let n = (wanted.max(0.0).floor() as usize).clamp(1, numbers.len());
        Some(numbers[n - 1])
    }
}

/// The filters that depend on the other cells of the column or on today's
/// date (`dynamicFilter type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicFilter {
    AboveAverage,
    BelowAverage,
    Today,
    Yesterday,
    Tomorrow,
    ThisWeek,
    LastWeek,
    NextWeek,
    ThisMonth,
    LastMonth,
    NextMonth,
    ThisQuarter,
    LastQuarter,
    NextQuarter,
    ThisYear,
    LastYear,
    NextYear,
    YearToDate,
    /// All dates in a quarter (1-4), whatever the year.
    Quarter(u8),
    /// All dates in a month (1-12), whatever the year.
    Month(u8),
}

impl DynamicFilter {
    /// The `type` attribute of `dynamicFilter`.
    pub fn ooxml_type(self) -> String {
        match self {
            DynamicFilter::AboveAverage => "aboveAverage".into(),
            DynamicFilter::BelowAverage => "belowAverage".into(),
            DynamicFilter::Today => "today".into(),
            DynamicFilter::Yesterday => "yesterday".into(),
            DynamicFilter::Tomorrow => "tomorrow".into(),
            DynamicFilter::ThisWeek => "thisWeek".into(),
            DynamicFilter::LastWeek => "lastWeek".into(),
            DynamicFilter::NextWeek => "nextWeek".into(),
            DynamicFilter::ThisMonth => "thisMonth".into(),
            DynamicFilter::LastMonth => "lastMonth".into(),
            DynamicFilter::NextMonth => "nextMonth".into(),
            DynamicFilter::ThisQuarter => "thisQuarter".into(),
            DynamicFilter::LastQuarter => "lastQuarter".into(),
            DynamicFilter::NextQuarter => "nextQuarter".into(),
            DynamicFilter::ThisYear => "thisYear".into(),
            DynamicFilter::LastYear => "lastYear".into(),
            DynamicFilter::NextYear => "nextYear".into(),
            DynamicFilter::YearToDate => "yearToDate".into(),
            DynamicFilter::Quarter(q) => format!("Q{q}"),
            DynamicFilter::Month(m) => format!("M{m}"),
        }
    }

    pub fn from_ooxml_type(name: &str) -> Option<DynamicFilter> {
        Some(match name {
            "aboveAverage" => DynamicFilter::AboveAverage,
            "belowAverage" => DynamicFilter::BelowAverage,
            "today" => DynamicFilter::Today,
            "yesterday" => DynamicFilter::Yesterday,
            "tomorrow" => DynamicFilter::Tomorrow,
            "thisWeek" => DynamicFilter::ThisWeek,
            "lastWeek" => DynamicFilter::LastWeek,
            "nextWeek" => DynamicFilter::NextWeek,
            "thisMonth" => DynamicFilter::ThisMonth,
            "lastMonth" => DynamicFilter::LastMonth,
            "nextMonth" => DynamicFilter::NextMonth,
            "thisQuarter" => DynamicFilter::ThisQuarter,
            "lastQuarter" => DynamicFilter::LastQuarter,
            "nextQuarter" => DynamicFilter::NextQuarter,
            "thisYear" => DynamicFilter::ThisYear,
            "lastYear" => DynamicFilter::LastYear,
            "nextYear" => DynamicFilter::NextYear,
            "yearToDate" => DynamicFilter::YearToDate,
            _ => {
                let number = |prefix: char, max: u8| {
                    name.strip_prefix(prefix)
                        .and_then(|n| n.parse::<u8>().ok())
                        .filter(|n| (1..=max).contains(n))
                };
                match (number('Q', 4), number('M', 12)) {
                    (Some(q), _) => DynamicFilter::Quarter(q),
                    (_, Some(m)) => DynamicFilter::Month(m),
                    _ => return None,
                }
            }
        })
    }

    /// For the date periods, the days `first <= day < end` (serial numbers)
    /// that pass when today is `today`. Weeks run from Sunday to Saturday.
    pub fn period(self, today: f64) -> Option<(f64, f64)> {
        let today = today.floor();
        let (y, m, _) = serial_to_ymd(today);
        let m = m as i64;
        let day = |year: i64, month: i64| date_to_serial(year, month, 1).unwrap_or(0.0);
        let week = today - weekday(today) as f64;
        let quarter = (m - 1) / 3 * 3 + 1;
        Some(match self {
            DynamicFilter::Today => (today, today + 1.0),
            DynamicFilter::Yesterday => (today - 1.0, today),
            DynamicFilter::Tomorrow => (today + 1.0, today + 2.0),
            DynamicFilter::ThisWeek => (week, week + 7.0),
            DynamicFilter::LastWeek => (week - 7.0, week),
            DynamicFilter::NextWeek => (week + 7.0, week + 14.0),
            DynamicFilter::ThisMonth => (day(y, m), day(y, m + 1)),
            DynamicFilter::LastMonth => (day(y, m - 1), day(y, m)),
            DynamicFilter::NextMonth => (day(y, m + 1), day(y, m + 2)),
            DynamicFilter::ThisQuarter => (day(y, quarter), day(y, quarter + 3)),
            DynamicFilter::LastQuarter => (day(y, quarter - 3), day(y, quarter)),
            DynamicFilter::NextQuarter => (day(y, quarter + 3), day(y, quarter + 6)),
            DynamicFilter::ThisYear => (day(y, 1), day(y + 1, 1)),
            DynamicFilter::LastYear => (day(y - 1, 1), day(y, 1)),
            DynamicFilter::NextYear => (day(y + 1, 1), day(y + 2, 1)),
            DynamicFilter::YearToDate => (day(y, 1), today + 1.0),
            DynamicFilter::AboveAverage
            | DynamicFilter::BelowAverage
            | DynamicFilter::Quarter(_)
            | DynamicFilter::Month(_) => return None,
        })
    }
}

/// Filter by Color: the cells filled with `color`, or written in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorFilter {
    /// `None` is "No Fill" for cells and "Automatic" for fonts.
    pub color: Option<Color>,
    /// The font colour rather than the cell colour (`cellColor="0"`).
    pub font: bool,
}

impl AutoFilter {
    pub fn new(range: Rect) -> AutoFilter {
        AutoFilter {
            range,
            ..Default::default()
        }
    }

    /// The rows under the header; `None` when there are none.
    pub fn data_range(&self) -> Option<Rect> {
        let r = self.range;
        (r.r1 > r.r0).then(|| Rect::new(r.r0 + 1, r.c0, r.r1, r.c1))
    }

    /// True when at least one column has a criterion.
    pub fn is_filtered(&self) -> bool {
        !self.columns.is_empty()
    }

    /// Renumber the columns after `count` of them were inserted at offset
    /// `at`, or deleted from there: criteria, hidden buttons and sort keys
    /// follow their columns, those of deleted columns go.
    pub(crate) fn shift_columns(&mut self, at: u32, count: u32, insert: bool) {
        let moved = |id: u32| -> Option<u32> {
            if insert {
                Some(if id >= at { id + count } else { id })
            } else if id < at {
                Some(id)
            } else {
                (id >= at + count).then(|| id - count)
            }
        };
        self.columns = std::mem::take(&mut self.columns)
            .into_iter()
            .filter_map(|(id, c)| moved(id).map(|id| (id, c)))
            .collect();
        self.hidden_buttons = self.hidden_buttons.iter().filter_map(|id| moved(*id)).collect();
        if let Some(sort) = &mut self.sort {
            sort.levels = std::mem::take(&mut sort.levels)
                .into_iter()
                .filter_map(|mut level| {
                    level.key = moved(level.key)?;
                    Some(level)
                })
                .collect();
            if sort.levels.is_empty() {
                self.sort = None;
            }
        }
    }

    /// Follow rows or columns inserted into or deleted from the sheet.
    /// False when the filter is gone: its header row or all its columns
    /// were deleted.
    pub(crate) fn adjust(&mut self, edit: StructuralEdit) -> bool {
        let r = &mut self.range;
        match edit {
            StructuralEdit::InsertRows { at, count } => {
                if at <= r.r0 {
                    if r.r0 + count >= MAX_ROWS {
                        return false;
                    }
                    r.r0 += count;
                }
                if at <= r.r1 {
                    r.r1 = (r.r1 + count).min(MAX_ROWS - 1);
                }
            }
            StructuralEdit::DeleteRows { at, count } => {
                let end = at + count - 1;
                if at <= r.r0 && r.r0 <= end {
                    return false;
                }
                if end < r.r0 {
                    r.r0 -= count;
                    r.r1 -= count;
                } else if at <= r.r1 {
                    r.r1 -= end.min(r.r1) - at + 1;
                }
            }
            StructuralEdit::InsertCols { at, count } => {
                if at <= r.c0 {
                    if r.c0 + count >= MAX_COLS {
                        return false;
                    }
                    r.c0 += count;
                    r.c1 = (r.c1 + count).min(MAX_COLS - 1);
                } else if at <= r.c1 {
                    r.c1 = (r.c1 + count).min(MAX_COLS - 1);
                    let offset = at - r.c0;
                    self.shift_columns(offset, count, true);
                }
            }
            StructuralEdit::DeleteCols { at, count } => {
                let end = at + count - 1;
                if end < r.c0 {
                    r.c0 -= count;
                    r.c1 -= count;
                } else if at <= r.c1 {
                    let (first, last) = (at.max(r.c0), end.min(r.c1));
                    let gone = last - first + 1;
                    if gone == r.cols() {
                        return false;
                    }
                    let (offset, left) = (first - r.c0, r.cols() - gone);
                    // Columns deleted to the left of the range pull it over.
                    r.c0 = r.c0.min(at);
                    r.c1 = r.c0 + left - 1;
                    self.shift_columns(offset, gone, false);
                }
            }
        }
        true
    }
}

/// What the cells of a column mostly hold: decides between "Text
/// Filters", "Number Filters" and "Date Filters", and between "A to Z",
/// "Smallest to Largest" and "Oldest to Newest".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Text,
    Number,
    Date,
}

/// One entry of the drop-down's list of values.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterItem {
    /// As the cells display it.
    pub text: String,
    pub value: Value,
    pub checked: bool,
}

/// A year of the date tree, most recent first in the list.
#[derive(Clone, Debug, PartialEq)]
pub struct YearItem {
    pub year: i32,
    pub months: Vec<MonthItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonthItem {
    /// 1-12.
    pub month: u32,
    pub days: Vec<DayItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayItem {
    pub day: u32,
    pub checked: bool,
}

/// The state of a check box that stands for several others: `None` when
/// they disagree.
fn summary(mut states: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let Some(first) = states.next() else {
        return Some(true);
    };
    let first = first?;
    states.all(|s| s == Some(first)).then_some(first)
}

impl MonthItem {
    /// Checked, unchecked or (`None`) partly checked.
    pub fn checked(&self) -> Option<bool> {
        summary(self.days.iter().map(|d| Some(d.checked)))
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.days.iter_mut().for_each(|d| d.checked = checked);
    }
}

impl YearItem {
    /// Checked, unchecked or (`None`) partly checked.
    pub fn checked(&self) -> Option<bool> {
        summary(self.months.iter().map(|m| m.checked()))
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.months.iter_mut().for_each(|m| m.set_checked(checked));
    }
}

/// Everything the drop-down of a filter button shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterDropdown {
    pub scope: FilterScope,
    /// Offset of the column in the filter's range.
    pub col_id: u32,
    /// The text of the header cell.
    pub header: String,
    pub kind: ColumnKind,
    /// The column's current criterion.
    pub criteria: Option<Criteria>,
    /// `Some(descending)` when the range was last sorted on this column.
    pub sort: Option<bool>,
    /// The dates of the column as a year / month / day tree.
    pub dates: Vec<YearItem>,
    /// The other distinct values, in Excel's order: numbers, text,
    /// booleans, errors. Only the rows the other columns' criteria let
    /// through are listed.
    pub items: Vec<FilterItem>,
    /// The column has blank cells: the list ends with "(Blanks)".
    pub has_blanks: bool,
    pub blanks_checked: bool,
    /// The fill colours found in the column (`None` = no fill), for
    /// "Filter by Color" and "Sort by Color"; offer them when there are at
    /// least two.
    pub cell_colors: Vec<Option<Color>>,
    /// The font colours found in the column (`None` = automatic).
    pub font_colors: Vec<Option<Color>>,
    /// More than [`MAX_FILTER_ITEMS`] distinct values: the list is cut.
    pub truncated: bool,
}

impl FilterDropdown {
    pub fn is_filtered(&self) -> bool {
        self.criteria.is_some()
    }

    /// The state of "(Select All)": checked, unchecked or (`None`) mixed.
    pub fn all_checked(&self) -> Option<bool> {
        let blanks = self.has_blanks.then_some(Some(self.blanks_checked));
        summary(
            self.items
                .iter()
                .map(|i| Some(i.checked))
                .chain(self.dates.iter().map(|y| y.checked()))
                .chain(blanks),
        )
    }

    /// Check or uncheck everything ("(Select All)").
    pub fn set_all(&mut self, checked: bool) {
        self.items.iter_mut().for_each(|i| i.checked = checked);
        self.dates.iter_mut().for_each(|y| y.set_checked(checked));
        self.blanks_checked = checked;
    }

    /// The criterion for the boxes as they are now checked, to hand to
    /// `set_filter_criteria` on OK: `None` (no filter) when everything is
    /// checked.
    pub fn values_criteria(&self) -> Option<Criteria> {
        if self.all_checked() == Some(true) {
            return None;
        }
        let mut dates = Vec::new();
        for year in &self.dates {
            match year.checked() {
                Some(true) => dates.push(DateGroup::year(year.year)),
                Some(false) => {}
                None => {
                    for month in &year.months {
                        match month.checked() {
                            Some(true) => dates.push(DateGroup::month(year.year, month.month)),
                            Some(false) => {}
                            None => dates.extend(
                                month
                                    .days
                                    .iter()
                                    .filter(|d| d.checked)
                                    .map(|d| DateGroup::day(year.year, month.month, d.day)),
                            ),
                        }
                    }
                }
            }
        }
        Some(Criteria::Values(ValueFilter {
            values: self
                .items
                .iter()
                .filter(|i| i.checked)
                .map(|i| i.text.clone())
                .collect(),
            blanks: self.has_blanks && self.blanks_checked,
            dates,
        }))
    }
}

/// A filter button, for drawing and hit-testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterButton {
    pub row: u32,
    pub col: u32,
    pub scope: FilterScope,
    /// Offset of the column in the filter's range.
    pub col_id: u32,
    /// The column has a criterion (funnel on the button).
    pub filtered: bool,
    /// `Some(descending)` when the range was last sorted on this column
    /// (arrow on the button).
    pub sort: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterError {
    /// "This can't be applied to the selected range. Select a single cell
    /// in a range and try again."
    NothingToFilter,
    /// The range runs into a table, which has its own filter.
    Table,
}

/// The fill and font colours a cell shows, conditional formats included
/// (table styles are not: Excel neither filters nor sorts on them). They
/// are the colours as they are seen: a colour of the theme and the same
/// colour picked elsewhere are one colour to a filter or a sort.
pub(crate) fn cell_colors(wb: &Workbook, sheet: usize, row: u32, col: u32) -> (Option<Color>, Option<Color>) {
    let style = wb.style_at(sheet, row, col);
    let (mut fill, mut font) = (style.fill, style.font.color);
    if wb.sheets.get(sheet).is_some_and(|s| !s.cond_formats.is_empty()) {
        if let Some(result) = crate::condfmt::result_at(wb, sheet, row, col) {
            fill = result.patch.fill.unwrap_or(fill);
            font = result.patch.font_color.unwrap_or(font);
        }
    }
    (fill.map(Color::plain), font.map(Color::plain))
}

/// The sheet's own filter, then those of its tables.
fn filters(sheet: &Sheet) -> impl Iterator<Item = (FilterScope, &AutoFilter)> {
    let own = sheet.auto_filter.iter().map(|f| (FilterScope::Sheet, f));
    own.chain(
        sheet
            .tables
            .iter()
            .filter_map(|t| Some((FilterScope::Table(t.id), t.auto_filter.as_ref()?))),
    )
}

/// How many columns of the sheet's filters have a criterion.
pub(crate) fn criteria_count(sheet: &Sheet) -> usize {
    filters(sheet).map(|(_, f)| f.columns.len()).sum()
}

/// Data typed right under a filtered list belongs to it, as in Excel: the
/// range of the sheet's filter reaches down over the rows that hold
/// something.
fn grown_range(sheet: &Sheet, range: Rect) -> Rect {
    let mut r1 = range.r1;
    while r1 + 1 < MAX_ROWS {
        let next = Rect::new(r1 + 1, range.c0, r1 + 1, range.c1);
        let has_data = sheet
            .cells
            .range((next.r0, next.c0)..=(next.r0, next.c1))
            .any(|(_, cell)| !matches!(cell.content, Content::Empty));
        if !has_data || sheet.tables.iter().any(|t| t.range.intersects(&next)) {
            break;
        }
        r1 += 1;
    }
    Rect::new(range.r0, range.c0, r1, range.c1)
}

fn today() -> f64 {
    now_serial().floor()
}

/// A criterion made ready to test cells: the cut-offs that depend on the
/// whole column or on the date are worked out once.
enum Matcher<'a> {
    Values(&'a ValueFilter, HashSet<String>),
    Custom(&'a CustomFilter),
    AtLeast(f64),
    AtMost(f64),
    Above(f64),
    Below(f64),
    /// Dates from the first day up to, not including, the second.
    Days(f64, f64),
    Month(u32),
    Quarter(u32),
    Color(ColorFilter),
    Nothing,
}

impl Workbook {
    fn shown_text(&self, sheet: usize, row: u32, col: u32, value: &Value) -> String {
        if value.is_empty() {
            return String::new();
        }
        format::format_value(value, &self.style_at(sheet, row, col).num_fmt, self.locale).text
    }

    /// True when the cell shows its number as a date.
    fn shows_date(&self, sheet: usize, row: u32, col: u32) -> bool {
        matches!(
            format::format_kind(&self.style_at(sheet, row, col).num_fmt),
            FormatKind::Date | FormatKind::DateTime
        )
    }

    fn column_numbers(&self, sheet: usize, range: Rect, col: u32) -> Vec<f64> {
        let sh = &self.sheets[sheet];
        (range.r0 + 1..=range.r1)
            .filter_map(|row| match sh.value_at(row, col) {
                Value::Number(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    /// Top 10 and the averages look at the whole column, whatever the other
    /// criteria hide.
    fn matcher<'a>(&self, sheet: usize, range: Rect, col: u32, criteria: &'a Criteria, today: f64) -> Matcher<'a> {
        match criteria {
            Criteria::Values(filter) => {
                Matcher::Values(filter, filter.values.iter().map(|v| v.to_lowercase()).collect())
            }
            Criteria::Custom(custom) => Matcher::Custom(custom),
            Criteria::Top10(top) => match top.cut_off(self.column_numbers(sheet, range, col)) {
                Some(cut) if top.top => Matcher::AtLeast(cut),
                Some(cut) => Matcher::AtMost(cut),
                None => Matcher::Nothing,
            },
            Criteria::Dynamic(dynamic) => match dynamic {
                DynamicFilter::AboveAverage | DynamicFilter::BelowAverage => {
                    let numbers = self.column_numbers(sheet, range, col);
                    if numbers.is_empty() {
                        return Matcher::Nothing;
                    }
                    let average = numbers.iter().sum::<f64>() / numbers.len() as f64;
                    if *dynamic == DynamicFilter::AboveAverage {
                        Matcher::Above(average)
                    } else {
                        Matcher::Below(average)
                    }
                }
                DynamicFilter::Month(m) => Matcher::Month(*m as u32),
                DynamicFilter::Quarter(q) => Matcher::Quarter(*q as u32),
                period => match period.period(today) {
                    Some((first, end)) => Matcher::Days(first, end),
                    None => Matcher::Nothing,
                },
            },
            Criteria::Color(color) => Matcher::Color(*color),
        }
    }

    fn passes(&self, matcher: &Matcher, sheet: usize, row: u32, col: u32) -> bool {
        let value = self.value_at(sheet, row, col);
        let number = match &value {
            Value::Number(n) => Some(*n),
            _ => None,
        };
        match matcher {
            Matcher::Values(filter, texts) => {
                let text = self.shown_text(sheet, row, col, &value);
                if text.is_empty() {
                    return filter.blanks;
                }
                if let Some(n) = number {
                    if filter.dates.iter().any(|g| g.matches(n)) && self.shows_date(sheet, row, col) {
                        return true;
                    }
                }
                texts.contains(&text.to_lowercase())
            }
            Matcher::Custom(custom) => custom.matches(&value, &self.shown_text(sheet, row, col, &value)),
            Matcher::AtLeast(cut) => number.is_some_and(|n| n >= *cut),
            Matcher::AtMost(cut) => number.is_some_and(|n| n <= *cut),
            Matcher::Above(average) => number.is_some_and(|n| n > *average),
            Matcher::Below(average) => number.is_some_and(|n| n < *average),
            Matcher::Days(first, end) => number.is_some_and(|n| n.floor() >= *first && n.floor() < *end),
            Matcher::Month(month) => number.is_some_and(|n| serial_to_ymd(n).1 == *month),
            Matcher::Quarter(quarter) => number.is_some_and(|n| serial_to_ymd(n).1.div_ceil(3) == *quarter),
            Matcher::Color(filter) => {
                let (fill, font) = cell_colors(self, sheet, row, col);
                filter.color.map(Color::plain) == if filter.font { font } else { fill }
            }
            Matcher::Nothing => false,
        }
    }

    /// The matchers of a filter's criteria, each with the offset of its
    /// column.
    fn matchers<'a>(&self, sheet: usize, filter: &'a AutoFilter, range: Rect, today: f64) -> Vec<(u32, Matcher<'a>)> {
        filter
            .columns
            .iter()
            .filter(|(id, _)| **id < range.cols())
            .map(|(id, criteria)| (*id, self.matcher(sheet, range, range.c0 + id, criteria, today)))
            .collect()
    }

    /// The filter of a sheet (`FilterScope::Sheet`) or of one of its tables.
    pub fn auto_filter(&self, sheet: usize, scope: FilterScope) -> Option<&AutoFilter> {
        let sh = self.sheets.get(sheet)?;
        match scope {
            FilterScope::Sheet => sh.auto_filter.as_ref(),
            FilterScope::Table(id) => sh.tables.iter().find(|t| t.id == id)?.auto_filter.as_ref(),
        }
    }

    fn auto_filter_mut(&mut self, sheet: usize, scope: FilterScope) -> Option<&mut AutoFilter> {
        let sh = self.sheets.get_mut(sheet)?;
        match scope {
            FilterScope::Sheet => sh.auto_filter.as_mut(),
            FilterScope::Table(id) => sh.tables.iter_mut().find(|t| t.id == id)?.auto_filter.as_mut(),
        }
    }

    /// The filter whose range holds a cell: the one "Clear", "Reapply" and
    /// the sort buttons act on.
    pub fn filter_scope_at(&self, sheet: usize, row: u32, col: u32) -> Option<FilterScope> {
        filters(self.sheets.get(sheet)?)
            .find(|(_, f)| f.range.contains(row, col))
            .map(|(scope, _)| scope)
    }

    /// Work out `Sheet::filtered_rows` again from the criteria of every
    /// filter of the sheet. Not recorded for undo: callers take the
    /// snapshot.
    pub(crate) fn refilter(&mut self, sheet: usize) {
        self.refilter_on(sheet, today());
    }

    /// Filter again after cells moved, when the sheet has criteria or rows
    /// they hid.
    pub(crate) fn refilter_if_filtered(&mut self, sheet: usize) {
        if self
            .sheets
            .get(sheet)
            .is_some_and(|s| !s.filtered_rows.is_empty() || criteria_count(s) > 0)
        {
            self.snapshot();
            self.refilter(sheet);
        }
    }

    pub(crate) fn refilter_on(&mut self, sheet: usize, today: f64) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let grown = sh.auto_filter.as_ref().map(|f| grown_range(sh, f.range));
        let mut hidden = BTreeSet::new();
        let mut cut_offs = Vec::new();
        for (scope, filter) in filters(sh) {
            let range = match scope {
                FilterScope::Sheet => grown.unwrap_or(filter.range),
                FilterScope::Table(_) => filter.range,
            };
            for (id, criteria) in &filter.columns {
                if let Criteria::Top10(top) = criteria {
                    cut_offs.push((
                        scope,
                        *id,
                        top.cut_off(self.column_numbers(sheet, range, range.c0 + id)),
                    ));
                }
            }
            let matchers = self.matchers(sheet, filter, range, today);
            if matchers.is_empty() {
                continue;
            }
            for row in range.r0 + 1..=range.r1 {
                if !matchers.iter().all(|(id, m)| self.passes(m, sheet, row, range.c0 + id)) {
                    hidden.insert(row);
                }
            }
        }
        let sh = &mut self.sheets[sheet];
        // The rows an Advanced Filter hides stay hidden until it is cleared.
        if let Some(advanced) = &sh.advanced_filter {
            hidden.extend(advanced.hidden.iter().copied());
        }
        sh.filtered_rows = hidden;
        if let (Some(filter), Some(range)) = (&mut sh.auto_filter, grown) {
            filter.range = range;
        }
        for (scope, id, cut) in cut_offs {
            if let Some(Criteria::Top10(top)) = self.auto_filter_mut(sheet, scope).and_then(|f| f.columns.get_mut(&id))
            {
                top.filter_value = cut;
            }
        }
    }

    /// The Filter button of the Data tab: inside a table it shows or hides
    /// the table's filter buttons; elsewhere it removes the sheet's filter
    /// when there is one, else puts one on the selection (a single cell
    /// stands for the block of data around it). Returns whether buttons
    /// are now shown.
    pub fn toggle_auto_filter(&mut self, sheet: usize, selection: Rect) -> Result<bool, FilterError> {
        let sh = self.sheets.get(sheet).ok_or(FilterError::NothingToFilter)?;
        if let Some(table) = sh.tables.iter().find(|t| t.contains(selection.r0, selection.c0)) {
            let (id, on) = (table.id, table.auto_filter.is_none());
            return match self.set_table_option(sheet, id, TableOption::FilterButton, on) {
                Ok(()) => Ok(on),
                Err(_) => Err(FilterError::Table),
            };
        }
        if sh.auto_filter.is_some() {
            self.remove_auto_filter(sheet);
            return Ok(false);
        }
        let range = if selection.is_single() {
            self.current_region(sheet, selection.r0, selection.c0)
        } else {
            let range = self.clamp_to_used(sheet, selection);
            // A single row is the header of the data under it.
            if range.r0 == range.r1 {
                grown_range(sh, range)
            } else {
                range
            }
        };
        let blank = (range.r0..=range.r1).all(|r| (range.c0..=range.c1).all(|c| sh.is_blank(r, c)));
        if blank {
            return Err(FilterError::NothingToFilter);
        }
        self.set_auto_filter(sheet, range)?;
        Ok(true)
    }

    /// Put the sheet's filter on `range` (header row first), replacing the
    /// one it had.
    pub fn set_auto_filter(&mut self, sheet: usize, range: Rect) -> Result<(), FilterError> {
        let sh = self.sheets.get(sheet).ok_or(FilterError::NothingToFilter)?;
        if sh.tables.iter().any(|t| t.range.intersects(&range)) {
            return Err(FilterError::Table);
        }
        self.snapshot();
        self.sheets[sheet].auto_filter = Some(AutoFilter::new(range));
        self.refilter(sheet);
        Ok(())
    }

    /// Remove the sheet's filter and show the rows it hid.
    pub fn remove_auto_filter(&mut self, sheet: usize) -> bool {
        if !self.sheets.get(sheet).is_some_and(|s| s.auto_filter.is_some()) {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].auto_filter = None;
        self.refilter(sheet);
        true
    }

    /// Set (or with `None` clear) the criterion of one column and filter
    /// again. `col_id` is the column's offset in the filter's range.
    pub fn set_filter_criteria(
        &mut self,
        sheet: usize,
        scope: FilterScope,
        col_id: u32,
        criteria: Option<Criteria>,
    ) -> bool {
        self.set_filter_criteria_on(sheet, scope, col_id, criteria, today())
    }

    pub(crate) fn set_filter_criteria_on(
        &mut self,
        sheet: usize,
        scope: FilterScope,
        col_id: u32,
        criteria: Option<Criteria>,
        today: f64,
    ) -> bool {
        if !self.auto_filter(sheet, scope).is_some_and(|f| col_id < f.range.cols()) {
            return false;
        }
        self.snapshot();
        if let Some(filter) = self.auto_filter_mut(sheet, scope) {
            match criteria {
                Some(criteria) => filter.columns.insert(col_id, criteria),
                None => filter.columns.remove(&col_id),
            };
        }
        self.refilter_on(sheet, today);
        true
    }

    /// Data > Clear: drop every criterion and the sort state of a filter,
    /// keeping its buttons.
    pub fn clear_filters(&mut self, sheet: usize, scope: FilterScope) -> bool {
        if !self
            .auto_filter(sheet, scope)
            .is_some_and(|f| f.is_filtered() || f.sort.is_some())
        {
            return false;
        }
        self.snapshot();
        if let Some(filter) = self.auto_filter_mut(sheet, scope) {
            filter.columns.clear();
            filter.sort = None;
        }
        self.refilter(sheet);
        true
    }

    /// Data > Reapply: filter again with the current cell values.
    pub fn reapply_filters(&mut self, sheet: usize) {
        if sheet < self.sheets.len() {
            self.snapshot();
            self.refilter(sheet);
        }
    }

    /// Sort the rows of a filtered range from its drop-down ("Sort A to Z",
    /// "Sort by Color"...) and remember it for the arrow on the button.
    pub fn sort_by_filter(
        &mut self,
        sheet: usize,
        scope: FilterScope,
        col_id: u32,
        on: SortOn,
        descending: bool,
    ) -> Result<(), SortError> {
        let level = SortLevel {
            key: col_id,
            on,
            descending,
            custom_list: None,
        };
        self.sort_filter(
            sheet,
            scope,
            SortState {
                levels: vec![level],
                case_sensitive: false,
            },
        )
    }

    /// Sort the rows of a filtered range on several levels (the Sort dialog
    /// opened on a filtered list or a table) and remember the sort with
    /// the filter. The keys of the levels are column offsets in its range.
    pub fn sort_filter(&mut self, sheet: usize, scope: FilterScope, sort: SortState) -> Result<(), SortError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let Some(filter) = self.auto_filter(sheet, scope) else {
            return Ok(());
        };
        let range = match scope {
            FilterScope::Sheet => grown_range(sh, filter.range),
            FilterScope::Table(_) => filter.range,
        };
        let spec = SortSpec {
            levels: sort.levels.clone(),
            has_header: true,
            case_sensitive: sort.case_sensitive,
            left_to_right: false,
        };
        self.snapshot();
        self.sort_range(sheet, range, &spec)?;
        if let Some(filter) = self.auto_filter_mut(sheet, scope) {
            filter.range = range;
            filter.sort = Some(sort);
        }
        Ok(())
    }

    /// The filter buttons of a sheet (its own and those of its tables).
    pub fn filter_buttons(&self, sheet: usize) -> Vec<FilterButton> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (scope, filter) in filters(sh) {
            for col_id in (0..filter.range.cols()).filter(|id| !filter.hidden_buttons.contains(id)) {
                out.push(FilterButton {
                    row: filter.range.r0,
                    col: filter.range.c0 + col_id,
                    scope,
                    col_id,
                    filtered: filter.columns.contains_key(&col_id),
                    sort: sorted_on(filter, col_id),
                });
            }
        }
        out
    }

    /// The filter button drawn in a cell, if any.
    pub fn filter_button_at(&self, sheet: usize, row: u32, col: u32) -> Option<FilterButton> {
        let (scope, filter) = filters(self.sheets.get(sheet)?).find(|(_, f)| {
            f.range.r0 == row
                && f.range.c0 <= col
                && col <= f.range.c1
                && !f.hidden_buttons.contains(&(col - f.range.c0))
        })?;
        let col_id = col - filter.range.c0;
        Some(FilterButton {
            row,
            col,
            scope,
            col_id,
            filtered: filter.columns.contains_key(&col_id),
            sort: sorted_on(filter, col_id),
        })
    }

    /// "n of m records found" for the status bar: the rows a filter shows
    /// and the rows it has; `None` when it has no criteria.
    pub fn filter_record_count(&self, sheet: usize, scope: FilterScope) -> Option<(usize, usize)> {
        let filter = self.auto_filter(sheet, scope).filter(|f| f.is_filtered())?;
        let data = filter.data_range()?;
        let hidden = self.sheets[sheet].filtered_rows.range(data.r0..=data.r1).count();
        Some((data.rows() as usize - hidden, data.rows() as usize))
    }

    /// What the drop-down of a filter button shows for its column.
    pub fn filter_dropdown(&self, sheet: usize, scope: FilterScope, col_id: u32) -> Option<FilterDropdown> {
        self.filter_dropdown_on(sheet, scope, col_id, today())
    }

    pub(crate) fn filter_dropdown_on(
        &self,
        sheet: usize,
        scope: FilterScope,
        col_id: u32,
        today: f64,
    ) -> Option<FilterDropdown> {
        let sh = self.sheets.get(sheet)?;
        let filter = self.auto_filter(sheet, scope)?;
        let range = match scope {
            FilterScope::Sheet => grown_range(sh, filter.range),
            FilterScope::Table(_) => filter.range,
        };
        if col_id >= range.cols() {
            return None;
        }
        let col = range.c0 + col_id;
        let matchers = self.matchers(sheet, filter, range, today);
        let own = matchers.iter().find(|(id, _)| *id == col_id).map(|(_, m)| m);

        let mut items: Vec<FilterItem> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut days: BTreeMap<(i32, u32, u32), bool> = BTreeMap::new();
        let (mut has_blanks, mut blanks_checked, mut truncated) = (false, false, false);
        let (mut cell_colors_found, mut font_colors_found) = (Vec::new(), Vec::new());
        let (mut texts, mut numbers, mut dates) = (0usize, 0usize, 0usize);
        for row in range.r0 + 1..=range.r1 {
            // The list only offers what the other columns let through.
            if !matchers
                .iter()
                .all(|(id, m)| *id == col_id || self.passes(m, sheet, row, range.c0 + id))
            {
                continue;
            }
            let (fill, font) = cell_colors(self, sheet, row, col);
            if !cell_colors_found.contains(&fill) {
                cell_colors_found.push(fill);
            }
            if !font_colors_found.contains(&font) {
                font_colors_found.push(font);
            }
            let checked = own.is_none_or(|m| self.passes(m, sheet, row, col));
            let value = sh.value_at(row, col);
            let text = self.shown_text(sheet, row, col, &value);
            if text.is_empty() {
                has_blanks = true;
                blanks_checked |= checked;
                continue;
            }
            match &value {
                Value::Number(n) if self.shows_date(sheet, row, col) => {
                    dates += 1;
                    let (y, m, d) = serial_to_ymd(*n);
                    *days.entry((y as i32, m, d)).or_default() |= checked;
                    continue;
                }
                Value::Number(_) => numbers += 1,
                _ => texts += 1,
            }
            match index.get(&text.to_lowercase()) {
                Some(i) => items[*i].checked |= checked,
                None if items.len() >= MAX_FILTER_ITEMS => truncated = true,
                None => {
                    index.insert(text.to_lowercase(), items.len());
                    items.push(FilterItem { text, value, checked });
                }
            }
        }
        items.sort_by(|a, b| sort_order(&a.value, &b.value, false));

        // Most recent year first, then months and days in calendar order.
        let mut years: Vec<YearItem> = Vec::new();
        for ((year, month, day), checked) in days.into_iter().rev() {
            if years.last().is_none_or(|y| y.year != year) {
                years.push(YearItem {
                    year,
                    months: Vec::new(),
                });
            }
            let months = &mut years.last_mut()?.months;
            if months.first().is_none_or(|m| m.month != month) {
                months.insert(
                    0,
                    MonthItem {
                        month,
                        days: Vec::new(),
                    },
                );
            }
            months.first_mut()?.days.insert(0, DayItem { day, checked });
        }

        let kind = if dates > 0 && dates >= numbers && dates >= texts {
            ColumnKind::Date
        } else if numbers > texts {
            ColumnKind::Number
        } else {
            ColumnKind::Text
        };
        Some(FilterDropdown {
            scope,
            col_id,
            header: self.display_text(sheet, range.r0, col).text,
            kind,
            criteria: filter.columns.get(&col_id).cloned(),
            sort: sorted_on(filter, col_id),
            dates: years,
            items,
            has_blanks,
            blanks_checked,
            cell_colors: cell_colors_found,
            font_colors: font_colors_found,
            truncated,
        })
    }

    /// For a dynamic filter, the `val` and `maxVal` files store beside its
    /// type: the average, or the first day of the period and the day after
    /// its last.
    pub fn dynamic_filter_values(&self, sheet: usize, scope: FilterScope, col_id: u32) -> Option<(f64, Option<f64>)> {
        let filter = self.auto_filter(sheet, scope)?;
        let Some(Criteria::Dynamic(dynamic)) = filter.columns.get(&col_id) else {
            return None;
        };
        match self.matcher(
            sheet,
            filter.range,
            filter.range.c0 + col_id,
            &Criteria::Dynamic(*dynamic),
            today(),
        ) {
            Matcher::Above(average) | Matcher::Below(average) => Some((average, None)),
            Matcher::Days(first, end) => Some((first, Some(end))),
            _ => None,
        }
    }

    /// After loading a file: the rows marked hidden inside a range whose
    /// filter has criteria were hidden by that filter, not by hand. Moves
    /// them from `Sheet::hidden_rows` to `Sheet::filtered_rows` without
    /// evaluating the criteria (the file is the truth, as for Excel).
    pub fn adopt_filtered_rows(&mut self, sheet: usize) {
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let ranges: Vec<Rect> = filters(sh)
            .filter(|(_, f)| f.is_filtered())
            .filter_map(|(_, f)| f.data_range())
            .collect();
        for range in ranges {
            let rows: Vec<u32> = sh.hidden_rows.range(range.r0..=range.r1).copied().collect();
            for row in rows {
                sh.hidden_rows.remove(&row);
                sh.filtered_rows.insert(row);
            }
        }
    }
}

fn sorted_on(filter: &AutoFilter, col_id: u32) -> Option<bool> {
    filter
        .sort
        .as_ref()?
        .levels
        .iter()
        .find(|l| l.key == col_id)
        .map(|l| l.descending)
}

/// Cache of values derived from the cells (see `features::Caches`).
#[derive(Debug, Default)]
pub struct Cache;

/// See `features::on_structural_edit`. Table filters are moved by
/// `table::on_structural_edit`, with their tables.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.name != edited_sheet {
        return;
    }
    match edit {
        StructuralEdit::InsertRows { at, count } => {
            sheet.filtered_rows = shift_set(&sheet.filtered_rows, true, at, count, MAX_ROWS);
        }
        StructuralEdit::DeleteRows { at, count } => {
            sheet.filtered_rows = shift_set(&sheet.filtered_rows, false, at, count, MAX_ROWS);
        }
        StructuralEdit::InsertCols { .. } | StructuralEdit::DeleteCols { .. } => {}
    }
    if sheet.auto_filter.as_mut().is_some_and(|f| !f.adjust(edit)) {
        sheet.auto_filter = None;
    }
    advanced::on_structural_edit(sheet, edit);
}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`. The copy keeps the filter and the
/// hidden rows of the original; nothing in them names the sheet.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}

/// See `features::on_sheet_deleted`. The filter of the sheet and the rows
/// it hid went with it.
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`. Filters are kept on their sheet and
/// travel with it.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`. Cells under the sheet's filter are
/// never shifted (the move is refused), and the rows hidden for a table
/// are moved by the table as it grows or shrinks.
pub(crate) fn on_cells_shifted(_wb: &mut Workbook, _sheet: usize, _shift: CellShift) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    const S: FilterScope = FilterScope::Sheet;

    /// A list in A1:C7 with headers, a blank name, a text among the
    /// quantities and a blank date; the filter is on.
    fn list() -> Workbook {
        let mut w = wb(&[
            ("A1", "Name"),
            ("B1", "Qty"),
            ("C1", "Date"),
            ("A2", "apple"),
            ("B2", "5"),
            ("C2", "1/15/2024"),
            ("A3", "Banana"),
            ("B3", "12"),
            ("C3", "2/3/2024"),
            ("A4", "cherry"),
            ("B4", "7"),
            ("C4", "3/20/2023"),
            ("A5", "Apple"),
            ("B5", "20"),
            ("C5", "1/20/2024"),
            ("B6", "3"),
            ("A7", "date"),
            ("B7", "x"),
            ("C7", "12/25/2022"),
        ]);
        assert_eq!(w.toggle_auto_filter(0, Rect::cell(2, 1)), Ok(true));
        w.commit("filter");
        w
    }

    /// The (0-based) data rows the filters show.
    fn shown(w: &Workbook) -> Vec<u32> {
        (1..=6).filter(|r| !w.sheets[0].filtered_rows.contains(r)).collect()
    }

    fn values(list: &[&str]) -> Criteria {
        Criteria::Values(ValueFilter {
            values: list.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        })
    }

    fn custom(comparison: Comparison, text: &str) -> Criteria {
        Criteria::Custom(CustomFilter::one(Condition::new(comparison, text, Locale::en())))
    }

    /// 18 January 2024, a Thursday.
    fn today() -> f64 {
        date_to_serial(2024, 1, 18).unwrap()
    }

    #[test]
    fn turning_the_filter_on_and_off() {
        let mut w = list();
        assert_eq!(
            w.sheets[0].auto_filter.as_ref().unwrap().range,
            Rect::parse("A1:C7").unwrap()
        );
        let buttons = w.filter_buttons(0);
        assert_eq!(buttons.len(), 3);
        assert_eq!((buttons[2].row, buttons[2].col, buttons[2].col_id), (0, 2, 2));
        assert!(!buttons[0].filtered && buttons[0].sort.is_none());
        assert_eq!(w.filter_button_at(0, 0, 1).unwrap().col_id, 1);
        assert_eq!(w.filter_button_at(0, 1, 1), None);
        assert_eq!(w.filter_scope_at(0, 4, 2), Some(S));
        assert_eq!(w.filter_scope_at(0, 4, 3), None);

        // Off again: the rows a criterion hid come back.
        w.set_filter_criteria(0, S, 0, Some(values(&["cherry"])));
        assert_eq!(shown(&w), [3]);
        assert_eq!(w.toggle_auto_filter(0, Rect::cell(0, 0)), Ok(false));
        assert!(w.sheets[0].auto_filter.is_none() && w.sheets[0].filtered_rows.is_empty());

        // A multi-cell selection gives the columns and the header row; the
        // data under it belongs to the list. A blank cell cannot be
        // filtered.
        assert_eq!(w.toggle_auto_filter(0, Rect::parse("B1:C3").unwrap()), Ok(true));
        assert_eq!(
            w.sheets[0].auto_filter.as_ref().unwrap().range,
            Rect::parse("B1:C7").unwrap()
        );
        w.remove_auto_filter(0);
        assert_eq!(w.toggle_auto_filter(0, Rect::parse("A1:B1").unwrap()), Ok(true));
        assert_eq!(
            w.sheets[0].auto_filter.as_ref().unwrap().range,
            Rect::parse("A1:B7").unwrap()
        );
        w.remove_auto_filter(0);
        assert_eq!(
            w.toggle_auto_filter(0, Rect::cell(20, 20)),
            Err(FilterError::NothingToFilter)
        );
    }

    #[test]
    fn checked_values_blanks_and_undo() {
        let mut w = list();
        assert!(w.set_filter_criteria(0, S, 0, Some(values(&["APPLE"]))));
        w.commit("filter");
        // Values are compared as displayed, without case.
        assert_eq!(shown(&w), [1, 4]);
        assert_eq!(w.filter_record_count(0, S), Some((2, 6)));
        assert_eq!(w.sheets[0].row_height(2), 0.0);
        assert!(w.sheets[0].hidden_rows.is_empty());
        assert!(w.filter_buttons(0)[0].filtered);

        let with_blanks = Criteria::Values(ValueFilter {
            values: vec!["apple".into()],
            blanks: true,
            dates: Vec::new(),
        });
        w.set_filter_criteria(0, S, 0, Some(with_blanks));
        w.commit("filter");
        assert_eq!(shown(&w), [1, 4, 5]);

        assert!(w.undo());
        assert_eq!(shown(&w), [1, 4]);
        assert!(w.undo());
        assert_eq!(shown(&w), [1, 2, 3, 4, 5, 6]);
        assert!(!w.sheets[0].auto_filter.as_ref().unwrap().is_filtered());
        assert_eq!(w.filter_record_count(0, S), None);
        assert!(w.redo());
        assert_eq!(shown(&w), [1, 4]);

        // Out of the range: nothing happens.
        assert!(!w.set_filter_criteria(0, S, 3, Some(values(&["x"]))));
        assert!(!w.set_filter_criteria(0, FilterScope::Table(9), 0, None));
    }

    #[test]
    fn drop_down_lists() {
        let mut w = list();
        let names = w.filter_dropdown(0, S, 0).unwrap();
        assert_eq!(names.header, "Name");
        assert_eq!(names.kind, ColumnKind::Text);
        let texts: Vec<&str> = names.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["apple", "Banana", "cherry", "date"]);
        assert!(names.has_blanks && names.blanks_checked && !names.is_filtered());
        assert_eq!(names.all_checked(), Some(true));
        assert_eq!(names.values_criteria(), None);
        assert_eq!(names.cell_colors, [None]);

        // Numbers come before text.
        let qty = w.filter_dropdown(0, S, 1).unwrap();
        assert_eq!(qty.kind, ColumnKind::Number);
        let texts: Vec<&str> = qty.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["3", "5", "7", "12", "20", "x"]);
        assert!(!qty.has_blanks);

        // Dates make a tree, most recent year first.
        let dates = w.filter_dropdown(0, S, 2).unwrap();
        assert_eq!(dates.kind, ColumnKind::Date);
        assert!(dates.items.is_empty() && dates.has_blanks);
        let years: Vec<i32> = dates.dates.iter().map(|y| y.year).collect();
        assert_eq!(years, [2024, 2023, 2022]);
        let months: Vec<u32> = dates.dates[0].months.iter().map(|m| m.month).collect();
        assert_eq!(months, [1, 2]);
        let days: Vec<u32> = dates.dates[0].months[0].days.iter().map(|d| d.day).collect();
        assert_eq!(days, [15, 20]);

        // Unchecking boxes gives the criterion to apply.
        let mut pick = names.clone();
        pick.items[1].checked = false;
        assert_eq!(pick.all_checked(), None);
        let criteria = pick.values_criteria().unwrap();
        assert_eq!(
            criteria,
            Criteria::Values(ValueFilter {
                values: vec!["apple".into(), "cherry".into(), "date".into()],
                blanks: true,
                dates: Vec::new(),
            })
        );
        w.set_filter_criteria(0, S, 0, Some(criteria));
        assert_eq!(shown(&w), [1, 3, 4, 5, 6]);
        pick.set_all(false);
        assert_eq!(pick.all_checked(), Some(false));

        let mut pick = dates.clone();
        pick.dates[0].months[1].set_checked(false);
        pick.blanks_checked = false;
        assert_eq!(pick.dates[0].checked(), None);
        let criteria = pick.values_criteria().unwrap();
        assert_eq!(
            criteria,
            Criteria::Values(ValueFilter {
                values: Vec::new(),
                blanks: false,
                dates: vec![DateGroup::month(2024, 1), DateGroup::year(2023), DateGroup::year(2022)],
            })
        );
        // Banana (February 2024) is unchecked in both columns; the blank
        // date goes too.
        w.set_filter_criteria(0, S, 2, Some(criteria));
        assert_eq!(shown(&w), [1, 3, 4, 6]);

        // The list of a column only offers what the other columns let
        // through, and shows its own criterion as check marks.
        let qty = w.filter_dropdown(0, S, 1).unwrap();
        let texts: Vec<&str> = qty.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["5", "7", "20", "x"]);
        let names = w.filter_dropdown(0, S, 0).unwrap();
        let checked: Vec<(&str, bool)> = names.items.iter().map(|i| (i.text.as_str(), i.checked)).collect();
        assert_eq!(checked, [("apple", true), ("cherry", true), ("date", true)]);
        assert!(names.is_filtered() && !names.has_blanks);
        let dates = w.filter_dropdown(0, S, 2).unwrap();
        assert_eq!(dates.dates[0].months[0].checked(), Some(true));
        assert_eq!(dates.dates[0].checked(), Some(true));
        assert!(dates.has_blanks && !dates.blanks_checked);
    }

    #[test]
    fn custom_filters() {
        let mut w = list();
        let set = |w: &mut Workbook, col: u32, criteria: Criteria| {
            w.clear_filters(0, S);
            w.set_filter_criteria(0, S, col, Some(criteria));
            shown(w)
        };
        // Numbers compare with numbers: the text "x" never passes.
        assert_eq!(set(&mut w, 1, custom(Comparison::Greater, "6")), [2, 3, 4]);
        assert_eq!(set(&mut w, 1, custom(Comparison::LessOrEqual, "5")), [1, 5]);
        assert_eq!(set(&mut w, 1, custom(Comparison::Equals, "7")), [3]);
        assert_eq!(set(&mut w, 1, custom(Comparison::NotEquals, "7")), [1, 2, 4, 5, 6]);
        let both = |and: bool| {
            Criteria::Custom(CustomFilter {
                first: Condition::new(Comparison::Greater, "6", Locale::en()),
                second: Some(Condition::new(Comparison::Less, "15", Locale::en())),
                and,
            })
        };
        assert_eq!(set(&mut w, 1, both(true)), [2, 3]);
        assert_eq!(set(&mut w, 1, both(false)), [1, 2, 3, 4, 5]);
        // Text, without case, with wildcards.
        assert_eq!(set(&mut w, 0, custom(Comparison::BeginsWith, "a")), [1, 4]);
        assert_eq!(set(&mut w, 0, custom(Comparison::EndsWith, "E")), [1, 4, 6]);
        assert_eq!(set(&mut w, 0, custom(Comparison::Contains, "an")), [2]);
        assert_eq!(set(&mut w, 0, custom(Comparison::NotContains, "a")), [3, 5]);
        assert_eq!(set(&mut w, 0, custom(Comparison::Equals, "?pple")), [1, 4]);
        assert_eq!(set(&mut w, 0, custom(Comparison::Equals, "*e*r*")), [3]);
        assert_eq!(set(&mut w, 0, custom(Comparison::Equals, "~*")), [] as [u32; 0]);
        assert_eq!(set(&mut w, 0, custom(Comparison::GreaterOrEqual, "c")), [3, 6]);
        // Blanks: "equals nothing" and "does not equal nothing".
        assert_eq!(set(&mut w, 0, custom(Comparison::Equals, "")), [5]);
        assert_eq!(set(&mut w, 0, custom(Comparison::NotEquals, "")), [1, 2, 3, 4, 6]);
        // Dates are typed as dates and stored as serial numbers.
        let after = Condition::new(Comparison::Greater, "1/16/2024", Locale::en());
        assert_eq!(after.value, date_to_serial(2024, 1, 16).unwrap().to_string());
        assert_eq!(after.operand_text(Locale::en(), true), "1/16/2024");
        assert_eq!(set(&mut w, 2, Criteria::Custom(CustomFilter::one(after))), [2, 4]);
    }

    #[test]
    fn conditions_as_typed_and_as_stored() {
        let fr = Locale::fr();
        let c = Condition::new(Comparison::GreaterOrEqual, "6,5", fr);
        assert_eq!((c.op, c.value.as_str()), (CustomOp::GreaterOrEqual, "6.5"));
        assert_eq!(c.comparison(), (Comparison::GreaterOrEqual, "6.5"));
        assert_eq!(c.operand_text(fr, false), "6,5");
        for (comparison, stored, op) in [
            (Comparison::BeginsWith, "ab*", CustomOp::Equal),
            (Comparison::NotBeginsWith, "ab*", CustomOp::NotEqual),
            (Comparison::EndsWith, "*ab", CustomOp::Equal),
            (Comparison::NotEndsWith, "*ab", CustomOp::NotEqual),
            (Comparison::Contains, "*ab*", CustomOp::Equal),
            (Comparison::NotContains, "*ab*", CustomOp::NotEqual),
            (Comparison::Equals, "ab", CustomOp::Equal),
            (Comparison::NotEquals, "ab", CustomOp::NotEqual),
        ] {
            let c = Condition::new(comparison, "ab", fr);
            assert_eq!((c.op, c.value.as_str()), (op, stored));
            assert_eq!(c.comparison(), (comparison, "ab"));
        }
    }

    #[test]
    fn top_10() {
        let mut w = list();
        w.set_filter_criteria(0, S, 1, Some(Criteria::Top10(Top10::new(true, false, 2.0))));
        assert_eq!(shown(&w), [2, 4]);
        let Some(Criteria::Top10(top)) = w.auto_filter(0, S).unwrap().columns.get(&1) else {
            panic!("no top 10");
        };
        assert_eq!(top.filter_value, Some(12.0));
        w.set_filter_criteria(0, S, 1, Some(Criteria::Top10(Top10::new(false, false, 1.0))));
        assert_eq!(shown(&w), [5]);
        // Half of five numbers is two items.
        w.set_filter_criteria(0, S, 1, Some(Criteria::Top10(Top10::new(true, true, 50.0))));
        assert_eq!(shown(&w), [2, 4]);
        // It looks at the whole column, whatever other columns hide.
        w.set_filter_criteria(0, S, 0, Some(values(&["banana", "cherry"])));
        assert_eq!(shown(&w), [2]);
        // Ties with the last item all pass.
        w.clear_filters(0, S);
        w.set_cell_text(0, 5, 1, "12").unwrap();
        w.set_filter_criteria(0, S, 1, Some(Criteria::Top10(Top10::new(true, false, 2.0))));
        assert_eq!(shown(&w), [2, 4, 5]);
    }

    #[test]
    fn averages_and_date_periods() {
        let mut w = list();
        let set = |w: &mut Workbook, col: u32, dynamic: DynamicFilter| {
            w.clear_filters(0, S);
            w.set_filter_criteria_on(0, S, col, Some(Criteria::Dynamic(dynamic)), today());
            shown(w)
        };
        // The average of 5, 12, 7, 20 and 3 is 9.4.
        assert_eq!(set(&mut w, 1, DynamicFilter::AboveAverage), [2, 4]);
        assert_eq!(set(&mut w, 1, DynamicFilter::BelowAverage), [1, 3, 5]);
        assert_eq!(w.dynamic_filter_values(0, S, 1), Some((9.4, None)));
        assert_eq!(set(&mut w, 2, DynamicFilter::ThisMonth), [1, 4]);
        assert_eq!(set(&mut w, 2, DynamicFilter::NextMonth), [2]);
        assert_eq!(set(&mut w, 2, DynamicFilter::ThisWeek), [1, 4]);
        assert_eq!(set(&mut w, 2, DynamicFilter::ThisQuarter), [1, 2, 4]);
        assert_eq!(set(&mut w, 2, DynamicFilter::LastQuarter), [] as [u32; 0]);
        assert_eq!(set(&mut w, 2, DynamicFilter::ThisYear), [1, 2, 4]);
        assert_eq!(set(&mut w, 2, DynamicFilter::LastYear), [3]);
        assert_eq!(set(&mut w, 2, DynamicFilter::YearToDate), [1]);
        assert_eq!(set(&mut w, 2, DynamicFilter::Today), [] as [u32; 0]);
        assert_eq!(set(&mut w, 2, DynamicFilter::Month(12)), [6]);
        assert_eq!(set(&mut w, 2, DynamicFilter::Quarter(1)), [1, 2, 3, 4]);

        let day = |y, m, d| date_to_serial(y, m, d).unwrap();
        let t = today();
        assert_eq!(DynamicFilter::Today.period(t), Some((t, t + 1.0)));
        assert_eq!(DynamicFilter::Yesterday.period(t), Some((t - 1.0, t)));
        assert_eq!(DynamicFilter::Tomorrow.period(t), Some((t + 1.0, t + 2.0)));
        // Weeks run from Sunday (14 January) to Saturday.
        assert_eq!(
            DynamicFilter::ThisWeek.period(t),
            Some((day(2024, 1, 14), day(2024, 1, 21)))
        );
        assert_eq!(
            DynamicFilter::LastWeek.period(t),
            Some((day(2024, 1, 7), day(2024, 1, 14)))
        );
        assert_eq!(
            DynamicFilter::NextWeek.period(t),
            Some((day(2024, 1, 21), day(2024, 1, 28)))
        );
        assert_eq!(
            DynamicFilter::LastMonth.period(t),
            Some((day(2023, 12, 1), day(2024, 1, 1)))
        );
        assert_eq!(
            DynamicFilter::LastQuarter.period(t),
            Some((day(2023, 10, 1), day(2024, 1, 1)))
        );
        assert_eq!(
            DynamicFilter::NextQuarter.period(t),
            Some((day(2024, 4, 1), day(2024, 7, 1)))
        );
        assert_eq!(
            DynamicFilter::NextYear.period(t),
            Some((day(2025, 1, 1), day(2026, 1, 1)))
        );
        assert_eq!(DynamicFilter::YearToDate.period(t), Some((day(2024, 1, 1), t + 1.0)));
        assert_eq!(DynamicFilter::Month(3).period(t), None);

        let all = [
            DynamicFilter::AboveAverage,
            DynamicFilter::BelowAverage,
            DynamicFilter::Today,
            DynamicFilter::Yesterday,
            DynamicFilter::Tomorrow,
            DynamicFilter::ThisWeek,
            DynamicFilter::LastWeek,
            DynamicFilter::NextWeek,
            DynamicFilter::ThisMonth,
            DynamicFilter::LastMonth,
            DynamicFilter::NextMonth,
            DynamicFilter::ThisQuarter,
            DynamicFilter::LastQuarter,
            DynamicFilter::NextQuarter,
            DynamicFilter::ThisYear,
            DynamicFilter::LastYear,
            DynamicFilter::NextYear,
            DynamicFilter::YearToDate,
            DynamicFilter::Quarter(4),
            DynamicFilter::Month(12),
        ];
        for dynamic in all {
            assert_eq!(DynamicFilter::from_ooxml_type(&dynamic.ooxml_type()), Some(dynamic));
        }
        assert_eq!(DynamicFilter::Month(1).ooxml_type(), "M1");
        assert_eq!(DynamicFilter::from_ooxml_type("Q5"), None);
        assert_eq!(DynamicFilter::from_ooxml_type("M13"), None);
    }

    #[test]
    fn date_groups() {
        let noon = date_to_serial(2024, 3, 5).unwrap() + 0.5;
        assert!(DateGroup::year(2024).matches(noon));
        assert!(DateGroup::month(2024, 3).matches(noon));
        assert!(DateGroup::day(2024, 3, 5).matches(noon));
        assert!(!DateGroup::day(2024, 3, 6).matches(noon));
        assert!(!DateGroup::month(2023, 3).matches(noon));
        let hour = DateGroup {
            hour: Some(12),
            ..DateGroup::day(2024, 3, 5)
        };
        assert!(hour.matches(noon) && !hour.matches(noon + 0.25));
        assert_eq!(DateGroup::year(2024).grouping(), "year");
        assert_eq!(DateGroup::month(2024, 3).grouping(), "month");
        assert_eq!(DateGroup::day(2024, 3, 5).grouping(), "day");
        assert_eq!(hour.grouping(), "hour");
        for op in [
            CustomOp::Equal,
            CustomOp::NotEqual,
            CustomOp::Greater,
            CustomOp::GreaterOrEqual,
            CustomOp::Less,
            CustomOp::LessOrEqual,
        ] {
            assert_eq!(CustomOp::from_ooxml_name(op.ooxml_name()), Some(op));
        }
        assert_eq!(CustomOp::from_ooxml_name("between"), None);
    }

    #[test]
    fn colours() {
        let mut w = list();
        let yellow = Color(0xFFFF00);
        let red = Color(0xFF0000);
        w.update_style(0, Rect::parse("A3").unwrap(), &|s| s.fill = Some(yellow));
        w.update_style(0, Rect::parse("A5:A6").unwrap(), &|s| s.font.color = Some(red));
        let drop = w.filter_dropdown(0, S, 0).unwrap();
        assert_eq!(drop.cell_colors, [None, Some(yellow)]);
        assert_eq!(drop.font_colors, [None, Some(red)]);
        let by = |color, font| Some(Criteria::Color(ColorFilter { color, font }));
        w.set_filter_criteria(0, S, 0, by(Some(yellow), false));
        assert_eq!(shown(&w), [2]);
        w.set_filter_criteria(0, S, 0, by(None, false));
        assert_eq!(shown(&w), [1, 3, 4, 5, 6]);
        w.set_filter_criteria(0, S, 0, by(Some(red), true));
        assert_eq!(shown(&w), [4, 5]);
        w.set_filter_criteria(0, S, 0, by(None, true));
        assert_eq!(shown(&w), [1, 2, 3, 6]);
    }

    #[test]
    fn several_columns_clear_and_reapply() {
        let mut w = list();
        w.set_filter_criteria(0, S, 0, Some(custom(Comparison::NotEquals, "")));
        w.set_filter_criteria(0, S, 1, Some(custom(Comparison::Greater, "6")));
        w.commit("filter");
        assert_eq!(shown(&w), [2, 3, 4]);
        // Cells change: nothing moves until the filter is applied again.
        w.set_cell_text(0, 1, 1, "100").unwrap();
        w.commit("edit");
        assert_eq!(shown(&w), [2, 3, 4]);
        w.reapply_filters(0);
        w.commit("reapply");
        assert_eq!(shown(&w), [1, 2, 3, 4]);
        // Clearing one column, then everything.
        w.set_filter_criteria(0, S, 1, None);
        assert_eq!(shown(&w), [1, 2, 3, 4, 6]);
        assert!(w.clear_filters(0, S));
        assert_eq!(shown(&w), [1, 2, 3, 4, 5, 6]);
        assert!(w.sheets[0].auto_filter.is_some());
        assert!(!w.clear_filters(0, S));
    }

    #[test]
    fn rows_typed_under_the_list_join_it() {
        let mut w = list();
        w.set_cell_text(0, 7, 0, "elder").unwrap();
        w.set_cell_text(0, 7, 1, "1").unwrap();
        w.set_cell_text(0, 9, 0, "apart").unwrap();
        w.commit("edit");
        let names = w.filter_dropdown(0, S, 0).unwrap();
        assert!(names.items.iter().any(|i| i.text == "elder"));
        assert!(!names.items.iter().any(|i| i.text == "apart"));
        w.set_filter_criteria(0, S, 1, Some(custom(Comparison::Less, "4")));
        assert_eq!(w.auto_filter(0, S).unwrap().range, Rect::parse("A1:C8").unwrap());
        assert_eq!(
            w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3, 4, 6]
        );
        assert_eq!(w.filter_record_count(0, S), Some((2, 7)));
    }

    #[test]
    fn sorting_from_the_drop_down() {
        let mut w = list();
        w.sort_by_filter(0, S, 1, SortOn::Values, true).unwrap();
        w.commit("sort");
        // Largest to smallest: text before numbers, the header stays.
        let qty: Vec<String> = (0..7).map(|r| w.display_text(0, r, 1).text).collect();
        assert_eq!(qty, ["Qty", "x", "20", "12", "7", "5", "3"]);
        assert_eq!(w.display_text(0, 2, 0).text, "Apple");
        assert_eq!(w.filter_buttons(0)[1].sort, Some(true));
        assert_eq!(w.filter_buttons(0)[0].sort, None);
        assert_eq!(w.filter_dropdown(0, S, 1).unwrap().sort, Some(true));
        // Rows a filter hides stay where they are.
        w.set_filter_criteria(0, S, 1, Some(custom(Comparison::Greater, "6")));
        assert_eq!(shown(&w), [2, 3, 4]);
        w.sort_by_filter(0, S, 1, SortOn::Values, false).unwrap();
        let qty: Vec<String> = (0..7).map(|r| w.display_text(0, r, 1).text).collect();
        assert_eq!(qty, ["Qty", "x", "7", "12", "20", "5", "3"]);
        assert_eq!(shown(&w), [2, 3, 4]);
        // Clear drops the sort arrow too.
        w.clear_filters(0, S);
        assert_eq!(w.filter_buttons(0)[1].sort, None);

        // Several levels, as from the Sort dialog: names without case
        // (apple, Apple, Banana...), then quantities, largest first.
        let sort = SortState {
            levels: vec![SortLevel::values(0, false), SortLevel::values(1, true)],
            case_sensitive: false,
        };
        w.sort_filter(0, S, sort.clone()).unwrap();
        let names: Vec<String> = (1..7).map(|r| w.display_text(0, r, 0).text).collect();
        assert_eq!(names, ["Apple", "apple", "Banana", "cherry", "date", ""]);
        assert_eq!(w.auto_filter(0, S).unwrap().sort, Some(sort));
        assert_eq!(w.filter_buttons(0)[0].sort, Some(false));
        assert_eq!(w.filter_buttons(0)[1].sort, Some(true));
    }

    #[test]
    fn rows_and_columns_inserted_and_deleted() {
        let mut w = list();
        w.set_filter_criteria(0, S, 1, Some(custom(Comparison::Greater, "6")));
        w.set_filter_criteria(0, S, 2, Some(custom(Comparison::NotEquals, "")));
        w.commit("filter");
        assert_eq!(shown(&w), [2, 3, 4]);

        // A row above: the range and the hidden rows move down.
        w.insert_rows(0, 0, 1);
        w.commit("insert");
        assert_eq!(w.auto_filter(0, S).unwrap().range, Rect::parse("A2:C8").unwrap());
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2, 6, 7]);
        // A row inside joins the data, shown.
        w.insert_rows(0, 3, 1);
        w.commit("insert");
        assert_eq!(w.auto_filter(0, S).unwrap().range, Rect::parse("A2:C9").unwrap());
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2, 7, 8]);
        w.delete_rows(0, 3, 1);
        w.delete_rows(0, 0, 1);
        w.commit("delete");
        assert_eq!(w.auto_filter(0, S).unwrap().range, Rect::parse("A1:C7").unwrap());
        assert_eq!(shown(&w), [2, 3, 4]);

        // A column inserted inside: criteria follow their columns.
        w.insert_cols(0, 1, 2);
        w.commit("insert");
        let filter = w.auto_filter(0, S).unwrap();
        assert_eq!(filter.range, Rect::parse("A1:E7").unwrap());
        assert_eq!(filter.columns.keys().copied().collect::<Vec<_>>(), [3, 4]);
        // Deleting a filtered column drops its criterion.
        w.delete_cols(0, 1, 3);
        w.commit("delete");
        let filter = w.auto_filter(0, S).unwrap();
        assert_eq!(filter.range, Rect::parse("A1:B7").unwrap());
        assert_eq!(filter.columns.keys().copied().collect::<Vec<_>>(), [1]);
        // Columns deleted to the left pull the range over.
        w.insert_cols(0, 0, 2);
        w.delete_cols(0, 0, 1);
        w.commit("shift");
        assert_eq!(w.auto_filter(0, S).unwrap().range, Rect::parse("B1:C7").unwrap());

        // Deleting the header row removes the filter and shows the rows.
        assert!(!w.sheets[0].filtered_rows.is_empty());
        w.delete_rows(0, 0, 1);
        w.commit("delete");
        assert!(w.sheets[0].auto_filter.is_none());
        assert!(w.sheets[0].filtered_rows.is_empty());
        assert!(w.undo());
        assert!(w.sheets[0].auto_filter.is_some());
        assert!(!w.sheets[0].filtered_rows.is_empty());
    }

    #[test]
    fn a_copied_sheet_keeps_its_filter() {
        let mut w = list();
        w.set_filter_criteria(0, S, 0, Some(values(&["cherry"])));
        w.commit("filter");
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        assert_eq!(w.sheets[copy].auto_filter, w.sheets[0].auto_filter);
        assert_eq!(w.sheets[copy].filtered_rows, w.sheets[0].filtered_rows);
        // Each sheet is then filtered on its own.
        w.clear_filters(copy, S);
        assert!(w.sheets[copy].filtered_rows.is_empty());
        assert_eq!(shown(&w), [3]);
    }

    #[test]
    fn hidden_rows_of_a_loaded_file() {
        let mut w = list();
        // As a reader leaves things: criteria set, rows marked hidden.
        w.sheets[0]
            .auto_filter
            .as_mut()
            .unwrap()
            .columns
            .insert(0, values(&["cherry"]));
        w.sheets[0].hidden_rows.extend([1, 2, 4, 9]);
        w.adopt_filtered_rows(0);
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!(w.sheets[0].hidden_rows.iter().copied().collect::<Vec<_>>(), [9]);
    }

    #[test]
    fn number_formats_and_many_values() {
        // Values are listed and matched as displayed.
        let mut w = wb(&[("A1", "Price"), ("A2", "1.5"), ("A3", "2"), ("A4", "1.5")]);
        w.update_style(0, Rect::parse("A2:A4").unwrap(), &|s| s.num_fmt = "0.00".into());
        w.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
        let drop = w.filter_dropdown(0, S, 0).unwrap();
        let texts: Vec<&str> = drop.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["1.50", "2.00"]);
        w.set_filter_criteria(0, S, 0, Some(values(&["1.50"])));
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2]);
        // "Equals 1.5" finds the number whatever its format.
        w.set_filter_criteria(0, S, 0, Some(custom(Comparison::Equals, "1.5")));
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2]);

        let mut w = wb(&[("A1", "n")]);
        for r in 1..=(MAX_FILTER_ITEMS as u32 + 5) {
            w.set_value(0, r, 0, Value::Number(r as f64));
        }
        w.commit("fill");
        w.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
        let drop = w.filter_dropdown(0, S, 0).unwrap();
        assert!(drop.truncated);
        assert_eq!(drop.items.len(), MAX_FILTER_ITEMS);
    }
}
