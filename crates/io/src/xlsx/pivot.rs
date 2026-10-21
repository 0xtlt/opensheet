//! PivotTables (`Sheet::pivots`).
//!
//! A PivotTable is kept in three parts. Its *cache*, the copy of the
//! source records it summarises, is a definition
//! (`xl/pivotCache/pivotCacheDefinitionN.xml`: where the records come
//! from, the fields, the distinct values of each and how they are grouped)
//! and the records (`pivotCacheRecordsN.xml`: a line per source row, each
//! value named by its place among the distinct values of its field). The
//! workbook part lists the caches, and PivotTables on the same source
//! share one. The table itself (`xl/pivotTables/pivotTableN.xml`, related
//! from its worksheet) says which field is in which area and how it is
//! shown. The cells of the report are cells of the sheet like any others.
//!
//! Kept: everything the engine's model holds. A cache is written with
//! every distinct value listed, the way Excel writes the fields used in
//! rows, columns or filters; the layout Excel caches beside the definition
//! (`rowItems`, `colItems`) is written from the report as last computed.
//!
//! Not kept, and lost by a PivotTable read from a file: formats applied
//! to parts of the report (`formats`, conditional formats), sources other than a range or a name of this
//! workbook (the cache is kept, the source is not), caches saved without
//! their records (the report stays in the cells, as plain values), date
//! filters other than the two days a timeline sets, the member properties
//! of OLAP sources. Slicers and timelines are in `slicer.rs` and
//! `timeline.rs`.
//! Times are kept to the second.

use super::dxf::Dxfs;
use super::filter::{area, push_text_attr};
use super::parts::{Extras, SheetLinks};
use super::read::{Package, relationships};
use super::stylerefs::StyleLookup;
use super::write::{NS_DOC_REL, NS_MAIN, NS_PKG_REL, push_number};
use super::xml::{Event, Reader, Tag, XML_DECL, decode_cell_text};
use opensheet_engine::datetime::{date_to_serial, serial_to_ymd};
use opensheet_engine::pivot::{
    Aggregation, AxisField, BaseItem, CacheField, CalcItem, DataField, FieldGroup, FieldLayout,
    FieldSort, FilterKind, GroupBy, GroupKind, LabelFilter, PageField, PivotCache, PivotField,
    PivotFilter, PivotItem, PivotSource, PivotTable, ReportLine, ReportLineKind, ShowAs, SortOrder,
    Subtotals, Top10By, ValueFilter,
};
use opensheet_engine::value::ErrorKind;
use opensheet_engine::{Rect, Value, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

const CT_DEFINITION: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
const CT_RECORDS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";
const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";

/// The namespace of what Excel 2010 added to PivotTables: the newer "Show
/// Values As" calculations and repeated labels live in extensions.
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const EXT_DATA_FIELD: &str = "{E15A36E0-9728-4e99-A89B-3F7291B0FE68}";
const EXT_PIVOT_FIELD: &str = "{2946ED86-A175-432a-8AC1-64E0C546D7DE}";
/// What Excel 2013 added: a date filter says there that it means whole days.
const NS_X15: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main";
const EXT_PIVOT_FILTER: &str = "{0605FD5F-26C8-4aeb-8148-2DB25E43C511}";

/// The base items that are no item: the one before, the one after.
const BASE_PREVIOUS: u32 = 1_048_828;
const BASE_NEXT: u32 = 1_048_829;

/// The `x` of "Σ Values" among the fields of an axis, and the field a
/// sort by value names in its scope.
const VALUES_FIELD: i64 = -2;
const DATA_REFERENCE: u32 = 4_294_967_294;

/// The versions of Excel a part says it was made by and can be refreshed
/// with: 8 is Excel 2021 and 365, 3 is Excel 2007.
const VERSIONS: &str = "createdVersion=\"8\" minRefreshableVersion=\"3\"";

// ---- names of the format ---------------------------------------------------

/// (function, `subtotal` of a data field, `t` of a subtotal item or line,
/// flag of a field with custom subtotals).
const FUNCTIONS: [(Aggregation, &str, &str, &str); 11] = [
    (Aggregation::Sum, "sum", "sum", "sumSubtotal"),
    (Aggregation::Count, "count", "countA", "countASubtotal"),
    (Aggregation::Average, "average", "avg", "avgSubtotal"),
    (Aggregation::Max, "max", "max", "maxSubtotal"),
    (Aggregation::Min, "min", "min", "minSubtotal"),
    (
        Aggregation::Product,
        "product",
        "product",
        "productSubtotal",
    ),
    (
        Aggregation::CountNums,
        "countNums",
        "count",
        "countSubtotal",
    ),
    (Aggregation::StdDev, "stdDev", "stdDev", "stdDevSubtotal"),
    (
        Aggregation::StdDevp,
        "stdDevp",
        "stdDevP",
        "stdDevPSubtotal",
    ),
    (Aggregation::Var, "var", "var", "varSubtotal"),
    (Aggregation::Varp, "varp", "varP", "varPSubtotal"),
];

fn function_names(function: Aggregation) -> (&'static str, &'static str, &'static str) {
    FUNCTIONS
        .iter()
        .find(|(f, ..)| *f == function)
        .map_or(("sum", "sum", "sumSubtotal"), |(_, data, item, flag)| {
            (*data, *item, *flag)
        })
}

const GROUP_BY: [(GroupBy, &str); 8] = [
    (GroupBy::Range, "range"),
    (GroupBy::Seconds, "seconds"),
    (GroupBy::Minutes, "minutes"),
    (GroupBy::Hours, "hours"),
    (GroupBy::Days, "days"),
    (GroupBy::Months, "months"),
    (GroupBy::Quarters, "quarters"),
    (GroupBy::Years, "years"),
];

/// (kind of label filter, `type`, whether it takes two values).
const LABEL_FILTERS: [&str; 14] = [
    "captionEqual",
    "captionNotEqual",
    "captionBeginsWith",
    "captionNotBeginsWith",
    "captionEndsWith",
    "captionNotEndsWith",
    "captionContains",
    "captionNotContains",
    "captionGreaterThan",
    "captionGreaterThanOrEqual",
    "captionLessThan",
    "captionLessThanOrEqual",
    "captionBetween",
    "captionNotBetween",
];

const VALUE_FILTERS: [&str; 8] = [
    "valueEqual",
    "valueNotEqual",
    "valueGreaterThan",
    "valueGreaterThanOrEqual",
    "valueLessThan",
    "valueLessThanOrEqual",
    "valueBetween",
    "valueNotBetween",
];

// ---- dates -----------------------------------------------------------------

/// A date-time serial as caches write it: `2024-03-01T09:30:00`. A time
/// of day alone is on the day before 1900 that files give to serial 0.
pub(super) fn iso_date(serial: f64) -> String {
    let serial = if serial.is_finite() {
        serial.max(0.0)
    } else {
        0.0
    };
    let mut day = serial.floor();
    let mut seconds = ((serial - day) * 86_400.0).round() as u32;
    if seconds >= 86_400 {
        day += 1.0;
        seconds = 0;
    }
    let (year, month, date) = if day < 1.0 {
        (1899, 12, 30)
    } else {
        serial_to_ymd(day)
    };
    format!(
        "{year:04}-{month:02}-{date:02}T{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// The reverse of [`iso_date`]; `None` when the text is no date.
pub(super) fn date_from_iso(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = text.split_once('T').unwrap_or((text, ""));
    let mut parts = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Before 1900 there are no dates, only times of day.
    let days = if year < 1900 {
        0.0
    } else {
        date_to_serial(year, month, day)?
    };
    let mut seconds = 0.0;
    for (part, unit) in time.splitn(3, ':').zip([3600.0, 60.0, 1.0]) {
        if !part.is_empty() {
            seconds += part
                .parse::<f64>()
                .ok()
                .filter(|n| (0.0..61.0).contains(n))?
                * unit;
        }
    }
    Some(days + seconds / 86_400.0)
}

// ---- writing: the cache ----------------------------------------------------

/// Append an item of a cache: a distinct value of a field, or a group.
/// `date` writes numbers as dates.
fn push_cache_item(x: &mut String, value: &Value, date: bool) {
    match value {
        Value::Number(n) if date => {
            w!(x, "<d v=\"{}\"/>", iso_date(*n));
        }
        Value::Number(n) => {
            x.push_str("<n v=\"");
            push_number(x, if n.is_finite() { *n } else { 0.0 });
            x.push_str("\"/>");
        }
        Value::Text(text) => {
            x.push_str("<s");
            push_text_attr(x, "v", text);
            x.push_str("/>");
        }
        Value::Bool(b) => {
            w!(x, "<b v=\"{}\"/>", u8::from(*b));
        }
        Value::Error(e) => {
            w!(x, "<e v=\"{}\"/>", e.as_str());
        }
        Value::Empty | Value::Array(_) => x.push_str("<m/>"),
    }
}

/// Append the `<sharedItems>` of a field: its distinct values, and what
/// kinds of values they are, which Excel wants said the way it says it.
fn push_shared_items(x: &mut String, field: &CacheField, date: bool) {
    let items = &field.shared;
    let numbers: Vec<f64> = items
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    let blank = items
        .iter()
        .any(|v| matches!(v, Value::Empty | Value::Array(_)));
    // Text, and what counts as text here: booleans and errors.
    let text = items
        .iter()
        .any(|v| matches!(v, Value::Text(_) | Value::Bool(_) | Value::Error(_)));
    let long = items
        .iter()
        .any(|v| matches!(v, Value::Text(t) if t.chars().count() > 255));
    x.push_str("<sharedItems");
    if items.is_empty() {
        x.push_str("/>");
        return;
    }
    let number = !numbers.is_empty();
    if !text && !blank {
        x.push_str(" containsSemiMixedTypes=\"0\"");
    }
    if (date && number) || (!text && !number) {
        x.push_str(" containsNonDate=\"0\"");
    }
    if date && number {
        x.push_str(" containsDate=\"1\"");
    }
    if !text {
        x.push_str(" containsString=\"0\"");
    }
    if blank {
        x.push_str(" containsBlank=\"1\"");
    }
    if number {
        let min = numbers.iter().copied().fold(f64::INFINITY, f64::min);
        let max = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if date {
            w!(
                x,
                " minDate=\"{}\" maxDate=\"{}\"",
                iso_date(min),
                iso_date(max)
            );
        } else {
            if text {
                x.push_str(" containsMixedTypes=\"1\"");
            }
            x.push_str(" containsNumber=\"1\"");
            if numbers.iter().all(|n| n.fract() == 0.0) {
                x.push_str(" containsInteger=\"1\"");
            }
            x.push_str(" minValue=\"");
            push_number(x, min);
            x.push_str("\" maxValue=\"");
            push_number(x, max);
            x.push('"');
        }
    }
    w!(x, " count=\"{}\"", items.len());
    if long {
        x.push_str(" longText=\"1\"");
    }
    x.push('>');
    for item in items {
        push_cache_item(x, item, date);
    }
    x.push_str("</sharedItems>");
}

/// The period a field groups the dates of another field by.
fn period(cache: &PivotCache, field: usize) -> Option<GroupBy> {
    match cache.fields[field].group.as_ref()?.kind {
        GroupKind::Range { by, .. } if by != GroupBy::Range => Some(by),
        _ => None,
    }
}

/// The fields that group `base`, outermost first. The periods of a date
/// ("Years", "Quarters", "Months", all derived from the date itself) nest
/// by their length, whatever order the fields come in.
fn levels(cache: &PivotCache, base: usize) -> Vec<usize> {
    let mut levels = cache.group_children(base);
    if levels.iter().all(|f| period(cache, *f).is_some()) {
        levels.sort_by_key(|f| std::cmp::Reverse(period(cache, *f)));
    }
    levels
}

/// The field one level up in a grouping (`par`): a field's groups, then
/// the groups of those groups.
fn parent_field(cache: &PivotCache, field: usize) -> Option<usize> {
    if let Some(base) = cache.fields[field].group.as_ref().and_then(|g| g.base)
        && base != field
    {
        let siblings = levels(cache, base);
        if let Some(at) = siblings.iter().position(|f| *f == field)
            && at > 0
        {
            return Some(siblings[at - 1]);
        }
    }
    levels(cache, field).last().copied()
}

/// Append the `<fieldGroup>` of a field that groups, or is grouped.
fn push_field_group(x: &mut String, cache: &PivotCache, index: usize) {
    let field = &cache.fields[index];
    let parent = parent_field(cache, index);
    let Some(group) = &field.group else {
        if let Some(parent) = parent {
            w!(x, "<fieldGroup par=\"{parent}\"/>");
        }
        return;
    };
    x.push_str("<fieldGroup");
    if let Some(parent) = parent {
        w!(x, " par=\"{parent}\"");
    }
    w!(x, " base=\"{}\">", group.base.unwrap_or(index));
    match &group.kind {
        GroupKind::Range {
            by,
            start,
            end,
            interval,
            auto_start,
            auto_end,
        } => {
            let (start, end) = (*start, *end);
            x.push_str("<rangePr");
            if !auto_start {
                x.push_str(" autoStart=\"0\"");
            }
            if !auto_end {
                x.push_str(" autoEnd=\"0\"");
            }
            if *by == GroupBy::Range {
                x.push_str(" startNum=\"");
                push_number(x, start);
                x.push_str("\" endNum=\"");
                push_number(x, end);
                x.push('"');
            } else {
                let name = GROUP_BY
                    .iter()
                    .find(|(g, _)| g == by)
                    .map_or("days", |(_, name)| name);
                w!(
                    x,
                    " groupBy=\"{name}\" startDate=\"{}\" endDate=\"{}\"",
                    iso_date(start),
                    iso_date(end)
                );
            }
            if *interval != 1.0 {
                x.push_str(" groupInterval=\"");
                push_number(x, *interval);
                x.push('"');
            }
            x.push_str("/>");
        }
        GroupKind::Discrete => {
            w!(x, "<discretePr count=\"{}\">", group.map.len());
            for group in &group.map {
                w!(x, "<x v=\"{group}\"/>");
            }
            x.push_str("</discretePr>");
        }
    }
    w!(x, "<groupItems count=\"{}\">", group.items.len());
    for item in &group.items {
        push_cache_item(x, item, false);
    }
    x.push_str("</groupItems></fieldGroup>");
}

/// The definition part of a cache.
fn definition_xml(
    cache: &PivotCache,
    source: &PivotSource,
    extensions: &str,
    dxfs: &mut Dxfs,
) -> String {
    let mut x = String::from(XML_DECL);
    w!(
        x,
        "<pivotCacheDefinition xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_DOC_REL}\" r:id=\"rId1\" refreshedVersion=\"8\" {VERSIONS} recordCount=\"{}\">",
        cache.rows
    );
    x.push_str("<cacheSource type=\"worksheet\"><worksheetSource");
    match source {
        PivotSource::Range { sheet, range } => {
            w!(x, " ref=\"{}\"", area(*range));
            push_text_attr(&mut x, "sheet", sheet);
        }
        PivotSource::Name(name) => push_text_attr(&mut x, "name", name),
        // A source that is gone: a range on a sheet nobody has.
        PivotSource::Invalid => x.push_str(" ref=\"A1\" sheet=\"#REF!\""),
    }
    w!(
        x,
        "/></cacheSource><cacheFields count=\"{}\">",
        cache.fields.len()
    );
    for (index, field) in cache.fields.iter().enumerate() {
        x.push_str("<cacheField");
        push_text_attr(&mut x, "name", &field.name);
        w!(x, " numFmtId=\"{}\"", dxfs.num_fmt_id(&field.num_fmt));
        if let Some(formula) = &field.formula {
            // A calculated field: a formula and nothing else.
            push_text_attr(&mut x, "formula", formula);
            x.push_str(" databaseField=\"0\"/>");
            continue;
        }
        if field.is_database() {
            x.push('>');
            let from = x.len();
            push_shared_items(&mut x, field, cache.is_date(index));
            // The calculated items among them say that they are.
            for item in cache.calc_items.iter().filter(|item| item.field == index) {
                let mut plain = String::from("<s");
                push_text_attr(&mut plain, "v", &item.name);
                let marked = format!("{plain} f=\"1\"/>");
                plain.push_str("/>");
                if let Some(at) = x[from..].find(&plain) {
                    x.replace_range(from + at..from + at + plain.len(), &marked);
                }
            }
        } else {
            x.push_str(" databaseField=\"0\">");
        }
        push_field_group(&mut x, cache, index);
        x.push_str("</cacheField>");
    }
    x.push_str("</cacheFields>");
    // calculatedItems: each formula, and the item it computes.
    let items: Vec<(&opensheet_engine::pivot::CalcItem, u32)> = cache
        .calc_items
        .iter()
        .filter_map(|item| Some((item, cache.calc_item_index(item)?)))
        .collect();
    if !items.is_empty() {
        w!(x, "<calculatedItems count=\"{}\">", items.len());
        for (item, at) in items {
            x.push_str("<calculatedItem");
            push_text_attr(&mut x, "formula", &item.formula);
            w!(
                x,
                "><pivotArea cacheIndex=\"1\" outline=\"0\" fieldPosition=\"0\"><references count=\"1\"><reference field=\"{}\" count=\"1\"><x v=\"{at}\"/></reference></references></pivotArea></calculatedItem>",
                item.field
            );
        }
        x.push_str("</calculatedItems>");
    }
    // What slicers and timelines know the cache by.
    x.push_str(extensions);
    x.push_str("</pivotCacheDefinition>");
    x
}

/// The records part of a cache: for every record, the place of its value
/// among the distinct values of each field that has a column in the
/// source.
fn records_xml(cache: &PivotCache) -> String {
    let columns: Vec<&[u32]> = cache
        .fields
        .iter()
        .filter(|field| field.is_database())
        .map(|field| &field.records[..])
        .collect();
    let mut x = String::with_capacity(64 + cache.rows * columns.len() * 12);
    x.push_str(XML_DECL);
    w!(
        x,
        "<pivotCacheRecords xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_DOC_REL}\" count=\"{}\">",
        cache.rows
    );
    for record in 0..cache.rows {
        x.push_str("<r>");
        for column in &columns {
            w!(x, "<x v=\"{}\"/>", column.get(record).copied().unwrap_or(0));
        }
        x.push_str("</r>");
    }
    x.push_str("</pivotCacheRecords>");
    x
}

// ---- writing: the table ----------------------------------------------------

/// Where each item of each field is in the field's list: files name an
/// item by that place, the engine by its place in the cache.
struct Positions(Vec<HashMap<u32, u32>>);

impl Positions {
    fn of(p: &PivotTable) -> Positions {
        Positions(
            p.fields
                .iter()
                .map(|field| {
                    field
                        .items
                        .iter()
                        .enumerate()
                        .map(|(at, item)| (item.x, at as u32))
                        .collect()
                })
                .collect(),
        )
    }

    fn get(&self, field: usize, item: u32) -> u32 {
        self.0
            .get(field)
            .and_then(|items| items.get(&item))
            .copied()
            .unwrap_or(0)
    }
}

/// Append `<rowItems>` or `<colItems>`: the rows (or columns) of values of
/// the report as it was last laid out. A line names the items it stands
/// for, outermost first, leaving out those the line before it has too.
fn push_lines(
    x: &mut String,
    element: &str,
    axis: &[AxisField],
    lines: &[ReportLine],
    at: &Positions,
) {
    if lines.is_empty() {
        w!(x, "<{element} count=\"1\"><i/></{element}>");
        return;
    }
    w!(x, "<{element} count=\"{}\">", lines.len());
    let mut previous: Vec<u32> = Vec::new();
    for line in lines {
        let mut members: Vec<u32> = Vec::new();
        if line.kind == ReportLineKind::GrandTotal {
            members.push(0);
        } else {
            let mut items = line.items.iter().peekable();
            for entry in axis {
                match entry {
                    AxisField::Field(field) => match items.next() {
                        Some((own, item)) if own == field => members.push(at.get(*field, *item)),
                        _ => break,
                    },
                    // "Σ Values" counts for a line of values, and for any
                    // line with items further in.
                    AxisField::Values => {
                        let own = matches!(line.kind, ReportLineKind::Item | ReportLineKind::Group);
                        match line.data_field {
                            Some(data) if own || items.peek().is_some() => {
                                members.push(data as u32)
                            }
                            _ => break,
                        }
                    }
                }
            }
        }
        x.push_str("<i");
        match (line.kind, line.function) {
            (ReportLineKind::Subtotal, Some(function)) => {
                w!(x, " t=\"{}\"", function_names(function).1);
            }
            (ReportLineKind::Subtotal, None) => x.push_str(" t=\"default\""),
            (ReportLineKind::GrandTotal, _) => x.push_str(" t=\"grand\""),
            (ReportLineKind::Blank, _) => x.push_str(" t=\"blank\""),
            (ReportLineKind::Item | ReportLineKind::Group, _) => {}
        }
        let repeated = if line.kind == ReportLineKind::GrandTotal {
            0
        } else {
            members
                .iter()
                .zip(&previous)
                .take_while(|(a, b)| a == b)
                .count()
                .min(members.len().saturating_sub(1))
        };
        if repeated > 0 {
            w!(x, " r=\"{repeated}\"");
        }
        if let Some(data) = line.data_field.filter(|data| *data > 0) {
            w!(x, " i=\"{data}\"");
        }
        if members.is_empty() {
            x.push_str("/>");
        } else {
            x.push('>');
            for member in &members[repeated..] {
                match member {
                    0 => x.push_str("<x/>"),
                    member => {
                        w!(x, "<x v=\"{member}\"/>");
                    }
                }
            }
            x.push_str("</i>");
        }
        if line.kind != ReportLineKind::GrandTotal {
            previous = members;
        }
    }
    w!(x, "</{element}>");
}

/// The `<customFilters>` equivalent of a label or value filter: Excel
/// wants the filter said twice.
fn push_custom_filters(x: &mut String, and: bool, tests: &[(&str, String)]) {
    x.push_str("<customFilters");
    if and {
        x.push_str(" and=\"1\"");
    }
    x.push('>');
    for (operator, value) in tests {
        x.push_str("<customFilter");
        if !operator.is_empty() {
            w!(x, " operator=\"{operator}\"");
        }
        push_text_attr(x, "val", value);
        x.push_str("/>");
    }
    x.push_str("</customFilters>");
}

fn number_text(n: f64) -> String {
    let mut text = String::new();
    push_number(&mut text, if n.is_finite() { n } else { 0.0 });
    text
}

/// Append a `<filter>` of a PivotTable; `id` is its number among them.
fn push_filter(x: &mut String, filter: &PivotFilter, id: usize) {
    use LabelFilter as L;
    use ValueFilter as V;
    let (kind, measure, values, top) = match &filter.kind {
        FilterKind::Date { start, end } => {
            // What a timeline sets: both days are whole days.
            w!(
                x,
                "<filter fld=\"{}\" type=\"dateBetween\" evalOrder=\"-1\" id=\"{id}\">",
                filter.field
            );
            x.push_str("<autoFilter ref=\"A1\"><filterColumn colId=\"0\">");
            let tests = [
                ("greaterThanOrEqual", number_text(*start)),
                ("lessThanOrEqual", number_text(*end)),
            ];
            push_custom_filters(x, true, &tests);
            w!(
                x,
                "</filterColumn></autoFilter><extLst><ext uri=\"{EXT_PIVOT_FILTER}\" xmlns:x15=\"{NS_X15}\"><x15:pivotFilter useWholeDay=\"1\"/></ext></extLst></filter>"
            );
            return;
        }
        FilterKind::Label(label) => {
            let (at, values): (usize, Vec<&String>) = match label {
                L::Equals(a) => (0, vec![a]),
                L::NotEquals(a) => (1, vec![a]),
                L::BeginsWith(a) => (2, vec![a]),
                L::NotBeginsWith(a) => (3, vec![a]),
                L::EndsWith(a) => (4, vec![a]),
                L::NotEndsWith(a) => (5, vec![a]),
                L::Contains(a) => (6, vec![a]),
                L::NotContains(a) => (7, vec![a]),
                L::GreaterThan(a) => (8, vec![a]),
                L::GreaterOrEqual(a) => (9, vec![a]),
                L::LessThan(a) => (10, vec![a]),
                L::LessOrEqual(a) => (11, vec![a]),
                L::Between(a, b) => (12, vec![a, b]),
                L::NotBetween(a, b) => (13, vec![a, b]),
            };
            let values: Vec<String> = values.into_iter().cloned().collect();
            (LABEL_FILTERS[at], None, values, None)
        }
        FilterKind::Value { data_field, test } => {
            let (at, values): (usize, Vec<f64>) = match *test {
                V::Equals(a) => (0, vec![a]),
                V::NotEquals(a) => (1, vec![a]),
                V::GreaterThan(a) => (2, vec![a]),
                V::GreaterOrEqual(a) => (3, vec![a]),
                V::LessThan(a) => (4, vec![a]),
                V::LessOrEqual(a) => (5, vec![a]),
                V::Between(a, b) => (6, vec![a, b]),
                V::NotBetween(a, b) => (7, vec![a, b]),
            };
            let values = values.into_iter().map(number_text).collect();
            (VALUE_FILTERS[at], Some(*data_field), values, None)
        }
        FilterKind::Top10 {
            data_field,
            top,
            count,
            by,
        } => {
            let kind = match by {
                Top10By::Items => "count",
                Top10By::Percent => "percent",
                Top10By::Sum => "sum",
            };
            (
                kind,
                Some(*data_field),
                Vec::new(),
                Some((*top, *count, *by)),
            )
        }
    };
    w!(
        x,
        "<filter fld=\"{}\" type=\"{kind}\" evalOrder=\"-1\" id=\"{id}\"",
        filter.field
    );
    if let Some(measure) = measure {
        w!(x, " iMeasureFld=\"{measure}\"");
    }
    for (value, name) in values.iter().zip(["stringValue1", "stringValue2"]) {
        push_text_attr(x, name, value);
    }
    x.push_str("><autoFilter ref=\"A1\"><filterColumn colId=\"0\">");
    if let Some((top, count, by)) = top {
        x.push_str("<top10");
        if !top {
            x.push_str(" top=\"0\"");
        }
        if by == Top10By::Percent {
            x.push_str(" percent=\"1\"");
        }
        let count = number_text(count);
        w!(x, " val=\"{count}\" filterVal=\"{count}\"/>");
    } else {
        // The comparison, with wildcards where the kind has them.
        let value = |at: usize| values.get(at).cloned().unwrap_or_default();
        let (and, tests): (bool, Vec<(&str, String)>) = match kind
            .trim_start_matches("caption")
            .trim_start_matches("value")
        {
            "Equal" => (false, vec![("", value(0))]),
            "NotEqual" => (false, vec![("notEqual", value(0))]),
            "BeginsWith" => (false, vec![("", format!("{}*", value(0)))]),
            "NotBeginsWith" => (false, vec![("notEqual", format!("{}*", value(0)))]),
            "EndsWith" => (false, vec![("", format!("*{}", value(0)))]),
            "NotEndsWith" => (false, vec![("notEqual", format!("*{}", value(0)))]),
            "Contains" => (false, vec![("", format!("*{}*", value(0)))]),
            "NotContains" => (false, vec![("notEqual", format!("*{}*", value(0)))]),
            "GreaterThan" => (false, vec![("greaterThan", value(0))]),
            "GreaterThanOrEqual" => (false, vec![("greaterThanOrEqual", value(0))]),
            "LessThan" => (false, vec![("lessThan", value(0))]),
            "LessThanOrEqual" => (false, vec![("lessThanOrEqual", value(0))]),
            "Between" => (
                true,
                vec![
                    ("greaterThanOrEqual", value(0)),
                    ("lessThanOrEqual", value(1)),
                ],
            ),
            _ => (
                false,
                vec![("lessThan", value(0)), ("greaterThan", value(1))],
            ),
        };
        push_custom_filters(x, and, &tests);
    }
    x.push_str("</filterColumn></autoFilter></filter>");
}

/// Append a `<dataField>`.
fn push_data_field(x: &mut String, data: &DataField, at: &Positions, dxfs: &mut Dxfs) {
    x.push_str("<dataField");
    push_text_attr(x, "name", &data.name);
    w!(x, " fld=\"{}\"", data.field);
    if data.function != Aggregation::Sum {
        w!(x, " subtotal=\"{}\"", function_names(data.function).0);
    }
    // The calculations of Excel 2007 are an attribute; the later ones are
    // "normal" to it and said in an extension.
    let (show, later) = match data.show_as {
        ShowAs::Normal => ("", ""),
        ShowAs::PercentOfTotal => ("percentOfTotal", ""),
        ShowAs::PercentOfColumn => ("percentOfCol", ""),
        ShowAs::PercentOfRow => ("percentOfRow", ""),
        ShowAs::PercentOf { .. } => ("percent", ""),
        ShowAs::Difference { .. } => ("difference", ""),
        ShowAs::PercentDifference { .. } => ("percentDiff", ""),
        ShowAs::RunningTotal { .. } => ("runTotal", ""),
        ShowAs::Index => ("index", ""),
        ShowAs::PercentOfParentRow => ("", "percentOfParentRow"),
        ShowAs::PercentOfParentColumn => ("", "percentOfParentCol"),
        ShowAs::PercentOfParent { .. } => ("", "percentOfParent"),
        ShowAs::PercentRunningTotal { .. } => ("", "percentOfRunningTotal"),
        ShowAs::RankAscending { .. } => ("", "rankAscending"),
        ShowAs::RankDescending { .. } => ("", "rankDescending"),
    };
    if !show.is_empty() {
        w!(x, " showDataAs=\"{show}\"");
    }
    let base_field = data.show_as.base_field();
    let base_item = match data.show_as {
        ShowAs::PercentOf { item, .. }
        | ShowAs::Difference { item, .. }
        | ShowAs::PercentDifference { item, .. } => match item {
            BaseItem::Previous => BASE_PREVIOUS,
            BaseItem::Next => BASE_NEXT,
            BaseItem::Item(item) => at.get(base_field.unwrap_or(0), item),
        },
        _ => 0,
    };
    w!(
        x,
        " baseField=\"{}\" baseItem=\"{base_item}\"",
        base_field.unwrap_or(0)
    );
    if let Some(format) = &data.num_fmt {
        w!(x, " numFmtId=\"{}\"", dxfs.num_fmt_id(format));
    }
    if later.is_empty() {
        x.push_str("/>");
    } else {
        w!(
            x,
            "><extLst><ext uri=\"{EXT_DATA_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:dataField pivotShowAs=\"{later}\"/></ext></extLst></dataField>"
        );
    }
}

/// Append a `<pivotField>`. `default` is the field as a new PivotTable on
/// the same cache has it.
fn push_pivot_field(
    x: &mut String,
    p: &PivotTable,
    index: usize,
    default: &PivotField,
    dxfs: &mut Dxfs,
) {
    let field = &p.fields[index];
    let axis = if p.row_fields.contains(&AxisField::Field(index)) {
        Some("axisRow")
    } else if p.col_fields.contains(&AxisField::Field(index)) {
        Some("axisCol")
    } else if p.page_fields.iter().any(|page| page.field == index) {
        Some("axisPage")
    } else {
        None
    };
    x.push_str("<pivotField");
    if let Some(caption) = &field.caption {
        push_text_attr(x, "name", caption);
    }
    if let Some(axis) = axis {
        w!(x, " axis=\"{axis}\"");
    }
    if p.data_fields.iter().any(|data| data.field == index) {
        x.push_str(" dataField=\"1\"");
    }
    if let Some(format) = &field.num_fmt {
        w!(x, " numFmtId=\"{}\"", dxfs.num_fmt_id(format));
    }
    match field.layout {
        FieldLayout::Compact => {}
        FieldLayout::Outline => x.push_str(" compact=\"0\""),
        FieldLayout::Tabular => x.push_str(" compact=\"0\" outline=\"0\""),
    }
    if !field.subtotal_top {
        x.push_str(" subtotalTop=\"0\"");
    }
    if p.cache.fields[index].formula.is_some() {
        // A calculated field cannot be dragged anywhere but the values.
        x.push_str(" dragToRow=\"0\" dragToCol=\"0\" dragToPage=\"0\"");
    }
    w!(x, " showAll=\"{}\"", u8::from(field.show_all));
    if field.blank_line {
        x.push_str(" insertBlankRow=\"1\"");
    }
    // A filter field showing several items says that it may.
    let page = p.page_fields.iter().find(|page| page.field == index);
    if page.is_some_and(|page| page.item.is_none()) && field.items.iter().any(|item| item.hidden) {
        x.push_str(" multipleItemSelectionAllowed=\"1\"");
    }
    match field.sort.order {
        SortOrder::Manual => {}
        SortOrder::Ascending => x.push_str(" sortType=\"ascending\""),
        SortOrder::Descending => x.push_str(" sortType=\"descending\""),
    }
    let functions: &[Aggregation] = match &field.subtotals {
        Subtotals::Automatic => &[],
        Subtotals::None => {
            x.push_str(" defaultSubtotal=\"0\"");
            &[]
        }
        Subtotals::Custom(functions) => {
            x.push_str(" defaultSubtotal=\"0\"");
            for (function, _, _, flag) in &FUNCTIONS {
                if functions.contains(function) {
                    w!(x, " {flag}=\"1\"");
                }
            }
            functions
        }
    };
    // The items: of the fields in an area, and of the others when they are
    // not as a new PivotTable would have them.
    let listed = axis.is_some() || field.items != default.items || !functions.is_empty();
    let sort_by = field
        .sort
        .by_value
        .filter(|_| field.sort.order != SortOrder::Manual);
    if !listed && sort_by.is_none() && !field.repeat_labels {
        x.push_str("/>");
        return;
    }
    x.push('>');
    if listed {
        let subtotals = match &field.subtotals {
            Subtotals::Automatic => 1,
            Subtotals::None => 0,
            Subtotals::Custom(functions) => functions.len(),
        };
        w!(x, "<items count=\"{}\">", field.items.len() + subtotals);
        for item in &field.items {
            x.push_str("<item");
            if let Some(caption) = &item.caption {
                push_text_attr(x, "n", caption);
            }
            if item.hidden {
                x.push_str(" h=\"1\"");
            }
            if item.collapsed {
                x.push_str(" sd=\"0\"");
            }
            if p.cache.is_calc_item(index, item.x) {
                x.push_str(" f=\"1\"");
            }
            w!(x, " x=\"{}\"/>", item.x);
        }
        match &field.subtotals {
            Subtotals::Automatic => x.push_str("<item t=\"default\"/>"),
            Subtotals::None => {}
            Subtotals::Custom(functions) => {
                for function in functions {
                    w!(x, "<item t=\"{}\"/>", function_names(*function).1);
                }
            }
        }
        x.push_str("</items>");
    }
    if let Some(data) = sort_by {
        w!(
            x,
            "<autoSortScope><pivotArea dataOnly=\"0\" outline=\"0\" fieldPosition=\"0\"><references count=\"1\"><reference field=\"{DATA_REFERENCE}\" count=\"1\" selected=\"0\"><x v=\"{data}\"/></reference></references></pivotArea></autoSortScope>"
        );
    }
    if field.repeat_labels {
        w!(
            x,
            "<extLst><ext uri=\"{EXT_PIVOT_FIELD}\" xmlns:x14=\"{NS_X14}\"><x14:pivotField fillDownLabels=\"1\"/></ext></extLst>"
        );
    }
    x.push_str("</pivotField>");
}

/// Append `<rowFields>` or `<colFields>`.
fn push_axis(x: &mut String, element: &str, axis: &[AxisField]) {
    if axis.is_empty() {
        return;
    }
    w!(x, "<{element} count=\"{}\">", axis.len());
    for entry in axis {
        match entry {
            AxisField::Field(field) => {
                w!(x, "<field x=\"{field}\"/>");
            }
            AxisField::Values => {
                w!(x, "<field x=\"{VALUES_FIELD}\"/>");
            }
        }
    }
    w!(x, "</{element}>");
}

/// The part of one PivotTable, on the cache numbered `cache_id`.
fn table_xml(wb: &Workbook, p: &PivotTable, cache_id: usize, dxfs: &mut Dxfs) -> String {
    let options = &p.options;
    let report = &*p.report;
    let mut x = String::from(XML_DECL);
    w!(x, "<pivotTableDefinition xmlns=\"{NS_MAIN}\"");
    push_text_attr(&mut x, "name", &p.name);
    w!(
        x,
        " cacheId=\"{cache_id}\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\""
    );
    push_text_attr(&mut x, "dataCaption", &p.data_caption(wb.locale));
    if let Some(caption) = &options.grand_total_caption {
        push_text_attr(&mut x, "grandTotalCaption", caption);
    }
    if options.show_error {
        x.push_str(" showError=\"1\"");
    }
    if !options.error_caption.is_empty() {
        push_text_attr(&mut x, "errorCaption", &options.error_caption);
    }
    if !options.show_missing {
        x.push_str(" showMissing=\"0\"");
    }
    if !options.missing_caption.is_empty() {
        push_text_attr(&mut x, "missingCaption", &options.missing_caption);
    }
    w!(
        x,
        " updatedVersion=\"8\" {VERSIONS} useAutoFormatting=\"1\" itemPrintTitles=\"1\" indent=\"0\""
    );
    if !options.show_headers {
        x.push_str(" showHeaders=\"0\"");
    }
    // The form new fields take: that of the fields there are.
    let layouts: Vec<FieldLayout> = p.fields.iter().map(|field| field.layout).collect();
    let all = |layout: FieldLayout| !layouts.is_empty() && layouts.iter().all(|l| *l == layout);
    if all(FieldLayout::Tabular) {
        x.push_str(" compact=\"0\" compactData=\"0\"");
    } else if all(FieldLayout::Outline) {
        x.push_str(" compact=\"0\" compactData=\"0\" outline=\"1\" outlineData=\"1\"");
    } else {
        x.push_str(" outline=\"1\" outlineData=\"1\"");
    }
    if !options.row_grand_totals {
        x.push_str(" rowGrandTotals=\"0\"");
    }
    if !options.col_grand_totals {
        x.push_str(" colGrandTotals=\"0\"");
    }
    if options.page_over_then_down {
        x.push_str(" pageOverThenDown=\"1\"");
    }
    if options.page_wrap > 0 {
        w!(x, " pageWrap=\"{}\"", options.page_wrap);
    }
    if !options.show_drill {
        x.push_str(" showDrill=\"0\"");
    }
    x.push_str(" multipleFieldFilters=\"0\"");
    if let Some(caption) = &options.row_header_caption {
        push_text_attr(&mut x, "rowHeaderCaption", caption);
    }
    if let Some(caption) = &options.col_header_caption {
        push_text_attr(&mut x, "colHeaderCaption", caption);
    }
    x.push('>');

    // Where the report is, and where its values start. Without column
    // fields Excel counts the header row as the first one all the same,
    // unless it holds the names of several value fields.
    let location = p.location.unwrap_or(Rect::cell(p.anchor.0, p.anchor.1));
    let header = if p.col_fields.is_empty() {
        1
    } else {
        report.first_header_row()
    };
    w!(
        x,
        "<location ref=\"{}\" firstHeaderRow=\"{header}\" firstDataRow=\"{}\" firstDataCol=\"{}\"",
        area(location),
        report.first_data_row().max(1),
        report.first_data_col().max(1)
    );
    let (page_rows, page_cols) = report.page_size();
    if page_rows > 0 {
        // A filter takes two cells, and a third one between two filters.
        w!(
            x,
            " rowPageCount=\"{page_rows}\" colPageCount=\"{}\"",
            page_cols.div_ceil(3).max(1)
        );
    }
    x.push_str("/>");

    let at = Positions::of(p);
    let defaults = PivotTable::new(
        0,
        String::new(),
        PivotSource::Invalid,
        p.cache.clone(),
        (0, 0),
    );
    w!(x, "<pivotFields count=\"{}\">", p.fields.len());
    for index in 0..p.fields.len() {
        let default = defaults.fields.get(index).cloned().unwrap_or_default();
        push_pivot_field(&mut x, p, index, &default, dxfs);
    }
    x.push_str("</pivotFields>");
    // A PivotTable nothing was put in yet has no lines either way; any
    // other has both lists, were it a single line without items.
    let empty = p.row_fields.is_empty() && p.col_fields.is_empty() && p.data_fields.is_empty();
    push_axis(&mut x, "rowFields", &p.row_fields);
    if !empty {
        push_lines(
            &mut x,
            "rowItems",
            &p.row_fields,
            &report.lines(true, p),
            &at,
        );
    }
    push_axis(&mut x, "colFields", &p.col_fields);
    if !empty {
        push_lines(
            &mut x,
            "colItems",
            &p.col_fields,
            &report.lines(false, p),
            &at,
        );
    }
    if !p.page_fields.is_empty() {
        w!(x, "<pageFields count=\"{}\">", p.page_fields.len());
        for page in &p.page_fields {
            w!(x, "<pageField fld=\"{}\"", page.field);
            if let Some(item) = page.item {
                w!(x, " item=\"{}\"", at.get(page.field, item));
            }
            x.push_str(" hier=\"-1\"/>");
        }
        x.push_str("</pageFields>");
    }
    if !p.data_fields.is_empty() {
        w!(x, "<dataFields count=\"{}\">", p.data_fields.len());
        for data in &p.data_fields {
            push_data_field(&mut x, data, &at, dxfs);
        }
        x.push_str("</dataFields>");
    }
    w!(x, "<pivotTableStyleInfo");
    if !p.style.name.is_empty() {
        push_text_attr(&mut x, "name", &p.style.name);
    }
    let flag = |on: bool| u8::from(on);
    w!(
        x,
        " showRowHeaders=\"{}\" showColHeaders=\"{}\" showRowStripes=\"{}\" showColStripes=\"{}\" showLastColumn=\"1\"/>",
        flag(p.style.row_headers),
        flag(p.style.col_headers),
        flag(p.style.row_stripes),
        flag(p.style.col_stripes)
    );
    if !p.filters.is_empty() {
        w!(x, "<filters count=\"{}\">", p.filters.len());
        for (index, filter) in p.filters.iter().enumerate() {
            push_filter(&mut x, filter, index + 1);
        }
        x.push_str("</filters>");
    }
    x.push_str("</pivotTableDefinition>");
    x
}

/// One relationship part with a single relationship.
fn single_rel(kind: &str, target: &str) -> String {
    format!(
        "{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{NS_DOC_REL}/{kind}\" Target=\"{target}\"/></Relationships>"
    )
}

/// What the PivotTables of a workbook add to its package. The number
/// formats they name get their ids from `dxfs`.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras, dxfs: &mut Dxfs) {
    // The caches, each once, in the order the PivotTables come in.
    let mut caches: Vec<&Arc<PivotCache>> = Vec::new();
    let mut tables = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        for (p, id) in sheet.pivots.iter().zip(&links.pivots) {
            let known = caches.iter().position(|cache| Arc::ptr_eq(cache, &p.cache));
            let cache = known.unwrap_or_else(|| {
                caches.push(&p.cache);
                let n = caches.len();
                extras.add_part(
                    format!("xl/pivotCache/pivotCacheDefinition{n}.xml"),
                    Some(CT_DEFINITION),
                    definition_xml(
                        &p.cache,
                        &p.source,
                        &super::slicer::cache_extensions(wb, &p.cache, n),
                        dxfs,
                    ),
                );
                extras.add_part(
                    format!("xl/pivotCache/_rels/pivotCacheDefinition{n}.xml.rels"),
                    None,
                    single_rel("pivotCacheRecords", &format!("pivotCacheRecords{n}.xml")),
                );
                extras.add_part(
                    format!("xl/pivotCache/pivotCacheRecords{n}.xml"),
                    Some(CT_RECORDS),
                    records_xml(&p.cache),
                );
                extras.add_pivot_cache(n, &format!("pivotCache/pivotCacheDefinition{n}.xml"));
                n - 1
            }) + 1;
            tables += 1;
            extras.add_part(
                format!("xl/pivotTables/pivotTable{tables}.xml"),
                Some(CT_TABLE),
                table_xml(wb, p, cache, dxfs),
            );
            extras.add_part(
                format!("xl/pivotTables/_rels/pivotTable{tables}.xml.rels"),
                None,
                single_rel(
                    "pivotCacheDefinition",
                    &format!("../pivotCache/pivotCacheDefinition{cache}.xml"),
                ),
            );
            extras.relate_sheet(
                index,
                *id,
                &format!("{NS_DOC_REL}/pivotTable"),
                &format!("../pivotTables/pivotTable{tables}.xml"),
            );
        }
    }
}

// ---- pivot charts -------------------------------------------------------------

const NS_C14: &str = "http://schemas.microsoft.com/office/drawing/2007/8/2/chart";
const EXT_PIVOT_OPTIONS: &str = "{781A3756-C4B2-4CAC-9D66-4F8BD8637D16}";

/// A sheet name as a reference writes it.
fn quoted_sheet(name: &str) -> String {
    let plain = name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        && !name.starts_with(|ch: char| ch.is_ascii_digit());
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// The part of a chart, completed with what makes it a PivotChart when
/// `chart` is bound to a PivotTable: the name of the table before the
/// chart (`c:pivotSource`), a format per series inside it
/// (`c:pivotFmts`), and the field buttons in the extensions.
pub(crate) fn bind_chart(
    wb: &Workbook,
    chart: &opensheet_engine::chart::Chart,
    mut xml: String,
) -> String {
    let Some((sheet, p, link)) = wb.pivot_of_chart(chart.id) else {
        return xml;
    };
    // Excel writes the name of the workbook file in brackets and does not
    // mind when the file was renamed since.
    let mut source = String::from("<c:pivotSource><c:name>");
    super::xml::push_text(
        &mut source,
        &format!(
            "[workbook.xlsx]{}!{}",
            quoted_sheet(&wb.sheets[sheet].name),
            p.name
        ),
    );
    source.push_str("</c:name><c:fmtId val=\"0\"/></c:pivotSource>");
    if let Some(at) = xml.find("<c:chart>") {
        xml.insert_str(at, &source);
    }
    let mut formats = String::from("<c:pivotFmts>");
    for index in 0..chart.series.len() {
        w!(
            formats,
            "<c:pivotFmt><c:idx val=\"{index}\"/><c:marker><c:symbol val=\"none\"/></c:marker></c:pivotFmt>"
        );
    }
    formats.push_str("</c:pivotFmts>");
    // After the title and `autoTitleDeleted`, which open the chart.
    let after = xml
        .find("<c:autoTitleDeleted")
        .and_then(|at| xml[at..].find("/>").map(|end| at + end + 2));
    let after = after.or_else(|| xml.find("<c:chart>").map(|at| at + "<c:chart>".len()));
    if let Some(at) = after {
        xml.insert_str(at, &formats);
    }
    if let Some(at) = xml.rfind("</c:chartSpace>") {
        // In the order Excel writes them; `dropZonesVisible` says that
        // some are shown.
        use opensheet_engine::pivot::ChartButtonKind as Kind;
        let flag = |kind: Kind| u8::from(link.shows(kind));
        xml.insert_str(
            at,
            &format!(
                "<c:extLst><c:ext uri=\"{EXT_PIVOT_OPTIONS}\" xmlns:c14=\"{NS_C14}\"><c14:pivotOptions><c14:dropZoneFilter val=\"{}\"/><c14:dropZoneCategories val=\"{}\"/><c14:dropZoneData val=\"{}\"/><c14:dropZoneSeries val=\"{}\"/><c14:dropZonesVisible val=\"{}\"/></c14:pivotOptions></c:ext></c:extLst>",
                flag(Kind::Filter),
                flag(Kind::Axis),
                flag(Kind::Value),
                flag(Kind::Legend),
                u8::from(link.buttons()),
            ),
        );
    }
    xml
}

/// Bind the charts read from the file to the PivotTables their parts
/// name (`c:pivotSource`).
fn link_charts(wb: &mut Workbook) {
    let mut links: Vec<(String, String, u32, [bool; 4])> = Vec::new();
    for chart in wb.sheets.iter().flat_map(|sheet| &sheet.charts) {
        let Some(xml) = chart.foreign.as_ref().map(|foreign| &foreign.xml) else {
            continue;
        };
        if !xml.contains("pivotSource") {
            continue;
        }
        let mut r = Reader::new(xml);
        let mut name = None;
        // Filter, legend, axis, values; and whether any shows at all.
        let mut zones = [false; 4];
        let mut visible = false;
        loop {
            match r.next() {
                Event::Open(t) if t.name == "pivotSource" && !t.empty => loop {
                    match r.next() {
                        Event::Open(c) if c.name == "name" => name = Some(r.text(&c).into_owned()),
                        Event::Open(c) => r.skip(&c),
                        Event::Close(_) | Event::Eof => break,
                        Event::Text(_) => {}
                    }
                },
                Event::Open(t) if t.name.starts_with("dropZone") => {
                    let on = t.attr_bool("val").unwrap_or(true);
                    match t.name {
                        "dropZoneFilter" => zones[0] = on,
                        "dropZoneSeries" => zones[1] = on,
                        "dropZoneCategories" => zones[2] = on,
                        "dropZoneData" => zones[3] = on,
                        _ => visible = on,
                    }
                    r.skip(&t);
                }
                Event::Eof => break,
                _ => {}
            }
        }
        // "[Book.xlsx]Sheet!PivotTable1": the sheet may be quoted.
        let Some(name) = name else {
            continue;
        };
        let rest = name
            .rsplit_once(']')
            .map_or(name.as_str(), |(_, rest)| rest);
        if let Some((sheet, table)) = rest.rsplit_once('!') {
            let sheet = sheet.trim_matches('\'').replace("''", "'");
            links.push((
                sheet,
                table.to_string(),
                chart.id,
                if visible { zones } else { [false; 4] },
            ));
        }
    }
    for (sheet, table, chart, shown) in links {
        let Some(index) = wb.sheet_index(&sheet) else {
            continue;
        };
        if let Some(p) = wb.sheets[index]
            .pivots
            .iter_mut()
            .find(|p| p.name.to_lowercase() == table.to_lowercase())
        {
            p.charts
                .push(opensheet_engine::pivot::PivotChartLink { chart, shown });
        }
    }
}

// ---- reading: the cache ----------------------------------------------------

/// An item of a cache (`<s>`, `<n>`, `<d>`, `<b>`, `<e>`, `<m>`); `None`
/// for anything else, or an item that does not say what it is.
fn parse_cache_item(t: &Tag) -> Option<Value> {
    let text = || t.attr("v").map(|v| decode_cell_text(&v).into_owned());
    Some(match t.name {
        "m" => Value::Empty,
        "n" => Value::Number(t.attr_f64("v")?),
        "b" => Value::Bool(t.attr_bool("v")?),
        "e" => Value::Error(ErrorKind::from_str(&text()?).unwrap_or(ErrorKind::Value)),
        // Empty text and no value are one item to the engine.
        "s" => Some(text()?)
            .filter(|s| !s.is_empty())
            .map_or(Value::Empty, Value::text),
        "d" => Value::Number(date_from_iso(&t.attr("v")?)?),
        _ => return None,
    })
}

/// The items listed in a `<sharedItems>` or `<groupItems>` element, and
/// whether any is a date.
fn parse_items<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> (Vec<Value>, bool) {
    let mut items = Vec::new();
    let mut dates = false;
    if tag.empty {
        return (items, dates);
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                if let Some(value) = parse_cache_item(&t) {
                    dates |= t.name == "d";
                    items.push(value);
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    (items, dates)
}

/// A `<fieldGroup>` of the field at `index`; `None` when it only says
/// which field groups this one.
fn parse_field_group<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, index: usize) -> Option<FieldGroup> {
    let base = tag
        .attr_u32("base")
        .map(|base| base as usize)
        .filter(|base| *base != index);
    let mut kind = None;
    let mut items = Vec::new();
    let mut map = Vec::new();
    if tag.empty {
        return None;
    }
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "rangePr" => {
                    let by = t.attr("groupBy").and_then(|name| {
                        GROUP_BY.iter().find(|(_, n)| *n == name).map(|(by, _)| *by)
                    });
                    let date = |name: &str| t.attr(name).and_then(|text| date_from_iso(&text));
                    let by = by.unwrap_or(GroupBy::Range);
                    let (start, end) = if by == GroupBy::Range {
                        (t.attr_f64("startNum"), t.attr_f64("endNum"))
                    } else {
                        (date("startDate"), date("endDate"))
                    };
                    kind = Some(GroupKind::Range {
                        by,
                        start: start.unwrap_or(0.0),
                        end: end.unwrap_or(0.0),
                        interval: t
                            .attr_f64("groupInterval")
                            .filter(|n| *n > 0.0)
                            .unwrap_or(1.0),
                        // Bounds the file does not give follow the data.
                        auto_start: t.attr_bool("autoStart").unwrap_or(true) || start.is_none(),
                        auto_end: t.attr_bool("autoEnd").unwrap_or(true) || end.is_none(),
                    });
                    r.skip(&t);
                }
                "discretePr" => {
                    kind = Some(GroupKind::Discrete);
                    if !t.empty {
                        loop {
                            match r.next() {
                                Event::Open(x) => {
                                    map.push(x.attr_u32("v").unwrap_or(0));
                                    r.skip(&x);
                                }
                                Event::Close(_) | Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                    }
                }
                "groupItems" => items = parse_items(r, &t).0,
                _ => r.skip(&t),
            },
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    let kind = kind?;
    // The groups of a field's own values need no other field; groups made
    // by hand always group another one.
    let base = match kind {
        GroupKind::Discrete => Some(base?),
        GroupKind::Range { .. } => base,
    };
    Some(FieldGroup {
        base,
        kind,
        items,
        map,
    })
}

/// A cache definition as read, before its records.
struct RawCache {
    source: PivotSource,
    fields: Vec<CacheField>,
    /// For every field, whether the definition lists its distinct values:
    /// the records then name them by their place, else they hold them.
    listed: Vec<bool>,
    /// The date fields the file groups in place, with the field that
    /// holds their period here (see [`split_date_groups`]).
    moved: Vec<(usize, usize)>,
    calc_items: Vec<CalcItem>,
}

/// A cache that was read: what PivotTables on it share.
#[derive(Clone)]
struct ReadCache {
    cache: Arc<PivotCache>,
    source: PivotSource,
    moved: Vec<(usize, usize)>,
}

/// Excel groups a date field in place by the shortest period chosen and
/// derives the longer ones from it. The engine keeps the dates as they are
/// and derives every period, so the period of the field itself moves to a
/// new field at the end. Returns (date field, new field) for each.
fn split_date_groups(
    fields: &mut Vec<CacheField>,
    locale: &opensheet_engine::Locale,
) -> Vec<(usize, usize)> {
    let mut moved = Vec::new();
    for index in 0..fields.len() {
        let by = match &fields[index].group {
            Some(FieldGroup {
                base: None,
                kind: GroupKind::Range { by, .. },
                ..
            }) if *by != GroupBy::Range => *by,
            _ => continue,
        };
        let Some(mut group) = fields[index].group.take() else {
            continue;
        };
        group.base = Some(index);
        let plain = format!("{} ({})", by.label(locale), fields[index].name);
        let mut name = plain.clone();
        let mut n = 2;
        while fields
            .iter()
            .any(|f| f.name.to_lowercase() == name.to_lowercase())
        {
            name = format!("{plain} {n}");
            n += 1;
        }
        moved.push((index, fields.len()));
        fields.push(CacheField {
            name,
            shared: Vec::new(),
            records: Vec::new(),
            num_fmt: Arc::from("General"),
            group: Some(group),
            formula: None,
        });
    }
    moved
}

fn parse_definition(
    xml: &str,
    lookup: &StyleLookup,
    locale: &opensheet_engine::Locale,
) -> Option<RawCache> {
    let mut r = Reader::new(xml);
    let mut source = PivotSource::Invalid;
    let mut fields: Vec<CacheField> = Vec::new();
    let mut listed = Vec::new();
    let mut found = false;
    // The calculated items: field, item, formula.
    let mut calculated: Vec<(usize, u32, String)> = Vec::new();
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "pivotCacheDefinition" => found = true,
                "cacheSource" | "cacheFields" | "calculatedItems" => {}
                "calculatedItem" if !t.empty => {
                    // The formula, then the field and the item it is for.
                    let formula = t.attr("formula").map(|v| decode_cell_text(&v).into_owned());
                    let (mut field, mut item) = (None, None);
                    let mut depth = 1usize;
                    while depth > 0 {
                        match r.next() {
                            Event::Open(c) => {
                                match c.name {
                                    "reference" => field = c.attr_u32("field").map(|f| f as usize),
                                    "x" => item = c.attr_u32("v"),
                                    _ => {}
                                }
                                depth += usize::from(!c.empty);
                            }
                            Event::Close(_) => depth -= 1,
                            Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    }
                    calculated.extend(
                        formula
                            .zip(field)
                            .zip(item)
                            .map(|((formula, field), item)| (field, item, formula)),
                    );
                }
                "worksheetSource" => {
                    let text = |name: &str| t.attr(name).map(|v| decode_cell_text(&v).into_owned());
                    // A source in another workbook cannot be read again.
                    let here = t.attr_local("id").is_none();
                    let range = t.attr("ref").and_then(|name| Rect::parse(&name));
                    source = match (text("name"), text("sheet"), range) {
                        (Some(name), _, _) if here && !name.is_empty() => PivotSource::Name(name),
                        (None, Some(sheet), Some(range)) if here && sheet != "#REF!" => {
                            PivotSource::Range { sheet, range }
                        }
                        _ => PivotSource::Invalid,
                    };
                    r.skip(&t);
                }
                "cacheField" => {
                    let index = fields.len();
                    let mut field = CacheField {
                        name: t
                            .attr("name")
                            .map(|v| decode_cell_text(&v).into_owned())
                            .unwrap_or_default(),
                        shared: Vec::new(),
                        records: Vec::new(),
                        num_fmt: lookup.num_fmt(t.attr_u32("numFmtId").unwrap_or(0)),
                        group: None,
                        // A calculated field has no column in the source.
                        formula: t
                            .attr("formula")
                            .filter(|_| !t.attr_bool("databaseField").unwrap_or(true))
                            .map(|v| decode_cell_text(&v).into_owned()),
                    };
                    let mut lists = false;
                    let mut dates = false;
                    if !t.empty {
                        loop {
                            match r.next() {
                                Event::Open(c) => match c.name {
                                    "sharedItems" => {
                                        (field.shared, dates) = parse_items(&mut r, &c);
                                        lists = !field.shared.is_empty();
                                        dates |= c.attr_bool("containsDate").unwrap_or(false);
                                    }
                                    "fieldGroup" => {
                                        field.group = parse_field_group(&mut r, &c, index)
                                    }
                                    _ => r.skip(&c),
                                },
                                Event::Close(_) | Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                    }
                    // Dates the file marks as such are dates here too,
                    // whatever the format it names.
                    if dates && !opensheet_engine::format::is_date_format(&field.num_fmt) {
                        field.num_fmt = Arc::from(locale.short_date_format);
                    }
                    fields.push(field);
                    listed.push(lists);
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let calc_items = calculated
        .into_iter()
        .filter_map(|(field, item, formula)| {
            match fields.get(field)?.shared.get(item as usize)? {
                Value::Text(name) => Some(CalcItem {
                    field,
                    name: name.to_string(),
                    formula,
                }),
                _ => None,
            }
        })
        .collect();
    let moved = split_date_groups(&mut fields, locale);
    listed.resize(fields.len(), false);
    (found && !fields.is_empty()).then_some(RawCache {
        source,
        fields,
        listed,
        moved,
        calc_items,
    })
}

/// Read the records of a cache into its fields; returns their number, or
/// `None` when a record does not fit the definition.
fn parse_records(xml: &str, raw: &mut RawCache) -> Option<usize> {
    // The fields with a column in the source, in order: a record has one
    // entry for each.
    let database: Vec<usize> = (0..raw.fields.len())
        .filter(|f| raw.fields[*f].is_database())
        .collect();
    let mut inline: Vec<Vec<Value>> = vec![Vec::new(); database.len()];
    let mut rows = 0;
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "pivotCacheRecords" => {}
            Event::Open(t) if t.name == "r" => {
                let mut column = 0;
                if !t.empty {
                    loop {
                        match r.next() {
                            Event::Open(c) => {
                                let field = *database.get(column)?;
                                if raw.listed[field] {
                                    // Anything but a place in the list
                                    // does not belong here.
                                    let at = c.attr_u32("v").filter(|_| c.name == "x")?;
                                    raw.fields[field].records.push(at);
                                } else {
                                    inline[column].push(parse_cache_item(&c)?);
                                }
                                column += 1;
                                r.skip(&c);
                            }
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    }
                }
                if column != database.len() {
                    return None;
                }
                rows += 1;
            }
            Event::Open(t) => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    // Values the records hold themselves: their distinct values are found
    // the way the engine finds those of a source column.
    for (column, values) in inline.into_iter().enumerate() {
        let field = &mut raw.fields[database[column]];
        if raw.listed[database[column]] {
            continue;
        }
        let one = PivotCache::from_columns(
            vec![(field.name.clone(), field.num_fmt.clone())],
            vec![values],
        );
        if let Some(read) = one.fields.into_iter().next() {
            field.shared = read.shared;
            field.records = read.records;
        }
    }
    Some(rows)
}

/// Read a cache: its definition part and, through the relationships of
/// that part, its records. `None` for a cache the engine cannot use: one
/// that makes no sense, or one saved without its records.
fn read_cache<R: Read + Seek>(
    pkg: &mut Package<R>,
    part: &str,
    lookup: &StyleLookup,
    locale: &opensheet_engine::Locale,
) -> Option<ReadCache> {
    let xml = pkg.read(part).ok()??;
    let mut raw = parse_definition(&xml, lookup, locale)?;
    let records = relationships(pkg, part)
        .into_iter()
        .find(|rel| rel.kind == "pivotCacheRecords")?;
    let xml = pkg.read(&records.target).ok()??;
    let rows = parse_records(&xml, &mut raw)?;
    let mut cache = PivotCache {
        fields: raw.fields,
        rows,
        calc_items: raw.calc_items,
    };
    cache.complete_groups(locale);
    cache.is_valid().then(|| ReadCache {
        cache: Arc::new(cache),
        source: raw.source,
        moved: raw.moved,
    })
}

// ---- reading: the table ----------------------------------------------------

/// A `<pivotField>`: the field, and its items when it lists them.
fn parse_pivot_field<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, lookup: &StyleLookup) -> PivotField {
    let flag = |name: &str, default: bool| tag.attr_bool(name).unwrap_or(default);
    let mut field = PivotField {
        caption: tag.attr("name").map(|v| decode_cell_text(&v).into_owned()),
        items: Vec::new(),
        sort: FieldSort {
            order: match tag.attr("sortType").as_deref() {
                Some("ascending") => SortOrder::Ascending,
                Some("descending") => SortOrder::Descending,
                _ => SortOrder::Manual,
            },
            by_value: None,
        },
        subtotals: Subtotals::Automatic,
        subtotal_top: flag("subtotalTop", true),
        layout: match (flag("compact", true), flag("outline", true)) {
            (true, _) => FieldLayout::Compact,
            (false, true) => FieldLayout::Outline,
            (false, false) => FieldLayout::Tabular,
        },
        blank_line: flag("insertBlankRow", false),
        repeat_labels: false,
        show_all: flag("showAll", true),
        num_fmt: tag.attr_u32("numFmtId").map(|id| lookup.num_fmt(id)),
    };
    // Custom subtotals: the flags say which, the items in which order.
    let flagged: Vec<Aggregation> = FUNCTIONS
        .iter()
        .filter(|(_, _, _, name)| flag(name, false))
        .map(|(function, ..)| *function)
        .collect();
    let mut ordered: Vec<Aggregation> = Vec::new();
    if !tag.empty {
        loop {
            match r.next() {
                Event::Open(t) => match t.name {
                    "items" if !t.empty => loop {
                        match r.next() {
                            Event::Open(item) => {
                                match (item.attr("t").as_deref(), item.attr_u32("x")) {
                                    (None | Some("data"), Some(x)) => field.items.push(PivotItem {
                                        x,
                                        hidden: item.attr_bool("h").unwrap_or(false),
                                        collapsed: !item.attr_bool("sd").unwrap_or(true),
                                        caption: item
                                            .attr("n")
                                            .map(|v| decode_cell_text(&v).into_owned()),
                                    }),
                                    (Some(kind), _) => ordered.extend(
                                        FUNCTIONS
                                            .iter()
                                            .find(|(_, _, name, _)| *name == kind)
                                            .map(|(function, ..)| *function),
                                    ),
                                    _ => {}
                                }
                                r.skip(&item);
                            }
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    // A sort by value names the value field in a scope.
                    "autoSortScope" if !t.empty => {
                        let mut depth = 1usize;
                        let mut data = false;
                        while depth > 0 {
                            match r.next() {
                                Event::Open(c) => {
                                    if c.name == "reference" {
                                        data = c.attr_u32("field") == Some(DATA_REFERENCE);
                                    } else if c.name == "x" && data && field.sort.by_value.is_none()
                                    {
                                        field.sort.by_value =
                                            Some(c.attr_u32("v").unwrap_or(0) as usize);
                                    }
                                    depth += usize::from(!c.empty);
                                }
                                Event::Close(_) => depth -= 1,
                                Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                    }
                    "extLst" if !t.empty => {
                        let mut depth = 1usize;
                        while depth > 0 {
                            match r.next() {
                                Event::Open(c) => {
                                    if c.name == "pivotField" {
                                        field.repeat_labels =
                                            c.attr_bool("fillDownLabels").unwrap_or(false);
                                    }
                                    depth += usize::from(!c.empty);
                                }
                                Event::Close(_) => depth -= 1,
                                Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                    }
                    _ => r.skip(&t),
                },
                Event::Close(_) | Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    ordered.retain(|function| flagged.contains(function));
    ordered.extend(
        flagged
            .iter()
            .filter(|function| !ordered.contains(function))
            .copied()
            .collect::<Vec<_>>(),
    );
    field.subtotals = match (flag("defaultSubtotal", true), ordered.is_empty()) {
        (_, false) => Subtotals::Custom(ordered),
        (true, true) => Subtotals::Automatic,
        (false, true) => Subtotals::None,
    };
    field
}

/// A `<dataField>`, with the base item still named by its place in the
/// list of its field.
fn parse_data_field<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    lookup: &StyleLookup,
) -> Option<(DataField, u32)> {
    let field = tag.attr_u32("fld")? as usize;
    let function = match tag.attr("subtotal") {
        Some(name) => FUNCTIONS
            .iter()
            .find(|(_, data, ..)| *data == name)
            .map_or(Aggregation::Sum, |(function, ..)| *function),
        None => Aggregation::Sum,
    };
    let mut show = tag
        .attr("showDataAs")
        .map(|v| v.into_owned())
        .unwrap_or_default();
    // What Excel 2010 added is said in an extension, and goes first.
    if !tag.empty {
        let mut depth = 1usize;
        while depth > 0 {
            match r.next() {
                Event::Open(c) => {
                    if c.name == "dataField"
                        && let Some(later) = c.attr("pivotShowAs")
                    {
                        show = later.into_owned();
                    }
                    depth += usize::from(!c.empty);
                }
                Event::Close(_) => depth -= 1,
                Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    let base = tag.attr_u32("baseField").unwrap_or(0) as usize;
    let base_item = tag.attr_u32("baseItem").unwrap_or(0);
    let item = match base_item {
        BASE_PREVIOUS => BaseItem::Previous,
        BASE_NEXT => BaseItem::Next,
        other => BaseItem::Item(other),
    };
    let show_as = match show.as_str() {
        "percentOfTotal" => ShowAs::PercentOfTotal,
        "percentOfCol" => ShowAs::PercentOfColumn,
        "percentOfRow" => ShowAs::PercentOfRow,
        "percent" => ShowAs::PercentOf { field: base, item },
        "difference" => ShowAs::Difference { field: base, item },
        "percentDiff" => ShowAs::PercentDifference { field: base, item },
        "runTotal" => ShowAs::RunningTotal { field: base },
        "index" => ShowAs::Index,
        "percentOfParentRow" => ShowAs::PercentOfParentRow,
        "percentOfParentCol" => ShowAs::PercentOfParentColumn,
        "percentOfParent" => ShowAs::PercentOfParent { field: base },
        "percentOfRunningTotal" => ShowAs::PercentRunningTotal { field: base },
        "rankAscending" => ShowAs::RankAscending { field: base },
        "rankDescending" => ShowAs::RankDescending { field: base },
        _ => ShowAs::Normal,
    };
    Some((
        DataField {
            field,
            name: tag
                .attr("name")
                .map(|v| decode_cell_text(&v).into_owned())
                .unwrap_or_default(),
            function,
            show_as,
            num_fmt: tag.attr_u32("numFmtId").map(|id| lookup.num_fmt(id)),
        },
        base_item,
    ))
}

/// A `<filter>` of a PivotTable; `None` for the kinds the engine has not
/// (filters on dates, mostly).
fn parse_filter<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<PivotFilter> {
    let field = tag.attr_u32("fld").map(|field| field as usize);
    let kind = tag.attr("type").map(|v| v.into_owned()).unwrap_or_default();
    let text = |name: &str| {
        tag.attr(name)
            .map(|v| decode_cell_text(&v).into_owned())
            .unwrap_or_default()
    };
    let (first, second) = (text("stringValue1"), text("stringValue2"));
    let data_field = tag.attr_u32("iMeasureFld").unwrap_or(0) as usize;
    // The top N is only said in the filter column inside.
    let mut top10: Option<(bool, f64)> = None;
    // The two days of a date filter are only said there too.
    let mut bounds: Vec<f64> = Vec::new();
    if !tag.empty {
        let mut depth = 1usize;
        while depth > 0 {
            match r.next() {
                Event::Open(c) => {
                    if c.name == "top10" {
                        top10 = Some((
                            c.attr_bool("top").unwrap_or(true),
                            c.attr_f64("val").unwrap_or(10.0),
                        ));
                    }
                    if c.name == "customFilter" {
                        bounds.extend(c.attr_f64("val"));
                    }
                    depth += usize::from(!c.empty);
                }
                Event::Close(_) => depth -= 1,
                Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    let field = field?;
    use LabelFilter as L;
    use ValueFilter as V;
    let kind = if kind == "dateBetween" {
        match bounds[..] {
            [start, end] => FilterKind::Date {
                start: start.floor(),
                end: end.floor(),
            },
            _ => return None,
        }
    } else if let Some(at) = LABEL_FILTERS.iter().position(|name| *name == kind) {
        FilterKind::Label(match at {
            0 => L::Equals(first),
            1 => L::NotEquals(first),
            2 => L::BeginsWith(first),
            3 => L::NotBeginsWith(first),
            4 => L::EndsWith(first),
            5 => L::NotEndsWith(first),
            6 => L::Contains(first),
            7 => L::NotContains(first),
            8 => L::GreaterThan(first),
            9 => L::GreaterOrEqual(first),
            10 => L::LessThan(first),
            11 => L::LessOrEqual(first),
            12 => L::Between(first, second),
            _ => L::NotBetween(first, second),
        })
    } else if let Some(at) = VALUE_FILTERS.iter().position(|name| *name == kind) {
        let number = |text: &str| text.trim().parse::<f64>().ok().filter(|n| n.is_finite());
        let a = number(&first)?;
        let b = number(&second);
        FilterKind::Value {
            data_field,
            test: match at {
                0 => V::Equals(a),
                1 => V::NotEquals(a),
                2 => V::GreaterThan(a),
                3 => V::GreaterOrEqual(a),
                4 => V::LessThan(a),
                5 => V::LessOrEqual(a),
                6 => V::Between(a, b?),
                _ => V::NotBetween(a, b?),
            },
        }
    } else {
        let by = match kind.as_str() {
            "count" => Top10By::Items,
            "percent" => Top10By::Percent,
            "sum" => Top10By::Sum,
            _ => return None,
        };
        let (top, count) = top10?;
        FilterKind::Top10 {
            data_field,
            top,
            count,
            by,
        }
    };
    Some(PivotFilter { field, kind })
}

/// The fields of an axis (`<rowFields>`, `<colFields>`).
fn parse_axis<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Vec<AxisField> {
    let mut axis = Vec::new();
    if tag.empty {
        return axis;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                if let Some(x) = t.attr("x").and_then(|x| x.trim().parse::<i64>().ok()) {
                    match x {
                        VALUES_FIELD => axis.push(AxisField::Values),
                        x if x >= 0 => axis.push(AxisField::Field(x as usize)),
                        _ => {}
                    }
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    axis
}

/// A PivotTable part, on the cache it names. `None` when it does not say
/// where the report is.
fn parse_table(
    xml: &str,
    id: u32,
    ReadCache {
        cache,
        source,
        moved,
    }: ReadCache,
    lookup: &StyleLookup,
    locale: &opensheet_engine::Locale,
) -> Option<PivotTable> {
    let mut r = Reader::new(xml);
    let root = loop {
        match r.next() {
            Event::Open(t) if t.name == "pivotTableDefinition" => break t,
            Event::Open(t) => r.skip(&t),
            Event::Eof => return None,
            _ => {}
        }
    };
    let text = |name: &str| root.attr(name).map(|v| decode_cell_text(&v).into_owned());
    let flag = |name: &str, default: bool| root.attr_bool(name).unwrap_or(default);
    let mut location = None;
    let mut pages = (0, 0);
    let mut fields: Vec<PivotField> = Vec::new();
    let mut row_fields = Vec::new();
    let mut col_fields = Vec::new();
    // (field, place of the item shown in the list of the field)
    let mut page_fields: Vec<(usize, Option<u32>)> = Vec::new();
    let mut data_fields: Vec<(DataField, u32)> = Vec::new();
    let mut filters = Vec::new();
    let mut style = opensheet_engine::pivot::PivotStyle {
        name: String::new(),
        row_headers: false,
        col_headers: false,
        row_stripes: false,
        col_stripes: false,
    };
    if !root.empty {
        loop {
            match r.next() {
                Event::Open(t) => match t.name {
                    "location" => {
                        location = t.attr("ref").and_then(|name| Rect::parse(&name));
                        pages = (
                            t.attr_u32("rowPageCount").unwrap_or(0),
                            t.attr_u32("colPageCount").unwrap_or(0),
                        );
                        r.skip(&t);
                    }
                    "pivotFields" if !t.empty => loop {
                        match r.next() {
                            Event::Open(f) if f.name == "pivotField" => {
                                fields.push(parse_pivot_field(&mut r, &f, lookup));
                            }
                            Event::Open(f) => r.skip(&f),
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    "rowFields" => row_fields = parse_axis(&mut r, &t),
                    "colFields" => col_fields = parse_axis(&mut r, &t),
                    "pageFields" if !t.empty => loop {
                        match r.next() {
                            Event::Open(f) => {
                                if let Some(field) = f.attr_u32("fld") {
                                    page_fields.push((field as usize, f.attr_u32("item")));
                                }
                                r.skip(&f);
                            }
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    "dataFields" if !t.empty => loop {
                        match r.next() {
                            Event::Open(f) if f.name == "dataField" => {
                                data_fields.extend(parse_data_field(&mut r, &f, lookup));
                            }
                            Event::Open(f) => r.skip(&f),
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    "filters" if !t.empty => loop {
                        match r.next() {
                            Event::Open(f) if f.name == "filter" => {
                                filters.extend(parse_filter(&mut r, &f))
                            }
                            Event::Open(f) => r.skip(&f),
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    "pivotTableStyleInfo" => {
                        let flag = |name: &str| t.attr_bool(name).unwrap_or(false);
                        style = opensheet_engine::pivot::PivotStyle {
                            name: t
                                .attr("name")
                                .map(|v| decode_cell_text(&v).into_owned())
                                .unwrap_or_default(),
                            row_headers: flag("showRowHeaders"),
                            col_headers: flag("showColHeaders"),
                            row_stripes: flag("showRowStripes"),
                            col_stripes: flag("showColStripes"),
                        };
                        r.skip(&t);
                    }
                    _ => r.skip(&t),
                },
                Event::Close(_) | Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    let location = location?;
    let mut p = PivotTable::new(
        id,
        text("name").unwrap_or_default(),
        source,
        cache,
        (location.r0, location.c0),
    );
    // What the file says of a date field it groups in place is said of
    // the field that holds the period here; only values are still made of
    // the dates themselves.
    let placed = |field: usize| {
        moved
            .iter()
            .find(|(date, _)| *date == field)
            .map_or(field, |(_, period)| *period)
    };
    for axis in [&mut row_fields, &mut col_fields] {
        for field in axis.iter_mut() {
            if let AxisField::Field(f) = field {
                *f = placed(*f);
            }
        }
    }
    for (field, _) in &mut page_fields {
        *field = placed(*field);
    }
    for filter in &mut filters {
        filter.field = placed(filter.field);
    }
    // A field that lists no items has them as a new PivotTable would.
    for (index, mut field) in fields.into_iter().enumerate() {
        let index = placed(index);
        if index >= p.fields.len() {
            continue;
        }
        if field.items.is_empty() {
            field.items = std::mem::take(&mut p.fields[index].items);
        }
        p.fields[index] = field;
    }
    // Files name an item by its place in the list of its field.
    let item_at = |p: &PivotTable, field: usize, place: u32| {
        p.fields
            .get(field)
            .and_then(|f| f.items.get(place as usize))
            .map(|item| item.x)
    };
    p.row_fields = row_fields;
    p.col_fields = col_fields;
    p.page_fields = page_fields
        .into_iter()
        .map(|(field, place)| PageField {
            field,
            item: place.and_then(|place| item_at(&p, field, place)),
        })
        .collect();
    p.data_fields = data_fields
        .into_iter()
        .map(|(mut data, place)| {
            let named = |field: usize| {
                item_at(&p, field, place).map_or(BaseItem::Item(place), BaseItem::Item)
            };
            match &mut data.show_as {
                ShowAs::PercentOf { field, item }
                | ShowAs::Difference { field, item }
                | ShowAs::PercentDifference { field, item } => {
                    *field = placed(*field);
                    if matches!(item, BaseItem::Item(_)) {
                        *item = named(*field);
                    }
                }
                ShowAs::PercentOfParent { field }
                | ShowAs::RunningTotal { field }
                | ShowAs::PercentRunningTotal { field }
                | ShowAs::RankAscending { field }
                | ShowAs::RankDescending { field } => *field = placed(*field),
                _ => {}
            }
            data
        })
        .collect();
    p.filters = filters;
    p.style = style;
    let options = &mut p.options;
    options.row_grand_totals = flag("rowGrandTotals", true);
    options.col_grand_totals = flag("colGrandTotals", true);
    options.show_missing = flag("showMissing", true);
    options.missing_caption = text("missingCaption").unwrap_or_default();
    options.show_error = flag("showError", false);
    options.error_caption = text("errorCaption").unwrap_or_default();
    options.grand_total_caption = text("grandTotalCaption");
    options.row_header_caption = text("rowHeaderCaption");
    options.col_header_caption = text("colHeaderCaption");
    // The caption of "Σ Values" is always in the file; it is one of the
    // engine's own unless someone typed another.
    let values = opensheet_engine::pivot::caption(opensheet_engine::pivot::Caption::Values, locale);
    options.data_caption = text("dataCaption").filter(|caption| caption != values);
    options.page_over_then_down = flag("pageOverThenDown", false);
    options.page_wrap = root.attr_u32("pageWrap").unwrap_or(0);
    options.show_headers = flag("showHeaders", true);
    options.show_drill = flag("showDrill", true);
    // Where the report was, so that what it leaves behind is cleared: the
    // filters sit above it, two cells each with one between two of them,
    // and a row between them and the report.
    p.location = Some(location);
    let (page_rows, page_cols) = pages;
    if page_rows > 0 && location.r0 > page_rows {
        let width = (page_cols.max(1) * 3 - 1).min(opensheet_engine::MAX_COLS - location.c0);
        p.page_location = Some(Rect::new(
            location.r0 - 1 - page_rows,
            location.c0,
            location.r0 - 2,
            location.c0 + width - 1,
        ));
    }
    Some(p)
}

/// Read the PivotTables of every sheet into the workbook and compute
/// their reports. `sheet_parts` names the worksheet part of each sheet of
/// `wb`, in order. A PivotTable whose parts are missing or make no sense,
/// or whose report cannot be laid out where it was, is left out: its
/// cells stay, as plain values.
pub(crate) fn read_all<R: Read + Seek>(
    pkg: &mut Package<R>,
    wb: &mut Workbook,
    sheet_parts: &[String],
    lookup: &StyleLookup,
) {
    let locale = wb.locale;
    // The caches by the name of their part: PivotTables naming the same
    // part share the cache.
    let mut caches: HashMap<String, Option<ReadCache>> = HashMap::new();
    for (index, part) in sheet_parts.iter().enumerate() {
        let mut rels = relationships(pkg, part);
        rels.retain(|rel| rel.kind == "pivotTable");
        // In the order Excel numbers them in.
        rels.sort_by_key(|rel| {
            let digits = rel.id.trim_start_matches(|c: char| !c.is_ascii_digit());
            digits.parse::<u32>().unwrap_or(u32::MAX)
        });
        for rel in rels {
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            let Some(cache_part) = relationships(pkg, &rel.target)
                .into_iter()
                .find(|rel| rel.kind == "pivotCacheDefinition")
                .map(|rel| rel.target)
            else {
                continue;
            };
            let cache = caches
                .entry(cache_part.clone())
                .or_insert_with(|| read_cache(pkg, &cache_part, lookup, locale));
            let Some(cache) = cache.clone() else {
                continue;
            };
            let id = wb.pivots().map(|(_, p)| p.id).max().unwrap_or(0) + 1;
            let Some(mut table) = parse_table(&xml, id, cache, lookup, locale) else {
                continue;
            };
            // A name of its own on the sheet.
            let taken = |name: &str| {
                name.is_empty()
                    || wb.sheets[index]
                        .pivots
                        .iter()
                        .any(|p| p.name.to_lowercase() == name.to_lowercase())
            };
            if taken(&table.name) {
                table.name = wb.pivot_default_name();
            }
            wb.sheets[index].pivots.push(table);
            if wb.pivot_update(id, true, |_| {}).is_err() {
                wb.sheets[index].pivots.retain(|p| p.id != id);
            }
        }
    }
    link_charts(wb);
}
