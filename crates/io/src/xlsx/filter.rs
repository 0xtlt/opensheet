//! The `<autoFilter>` element: the filter of a sheet or of a table, with
//! the criterion of each column (`filterColumn`) and the last sort made
//! through it (`sortState`).
//!
//! The element is the same in a worksheet and in a table part; both call
//! [`push_auto_filter`] and [`parse_auto_filter`]. The rows a filter hides
//! are hidden rows in the file; the filter of a sheet also has a hidden
//! defined name there, `_xlnm._FilterDatabase` ([`push_names`]), which is
//! not needed to read it back.
//!
//! Kept: every form of criterion the engine has (checked values and dates,
//! one or two comparisons, top and bottom N, the dynamic filters, a cell
//! or text colour), hidden buttons, the levels of the sort with their
//! custom lists and colours. Not kept: filters on icons, the calendar of
//! a date list, sorts by icon or by columns, the sort method of East Asian
//! text.

use super::color::{Palette, RawColor};
use super::dxf::Dxfs;
use super::write::push_number;
use super::xml::{Event, Reader, Tag, decode_cell_text, push_cell_text};
use opensheet_engine::filter::{
    AdvancedFilter, AutoFilter, ColorFilter, Condition, Criteria, CustomFilter, CustomOp, DateGroup, DynamicFilter,
    FilterScope, SortState, Top10, ValueFilter,
};
use opensheet_engine::ops::{SortLevel, SortOn};
use opensheet_engine::refs::cell_name;
use opensheet_engine::style::StylePatch;
use opensheet_engine::{Color, Rect, Sheet, Workbook};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// "A1:B2", always with both corners spelled out (a filter on whole
/// columns is still a block of cells in a file); "A1" for one cell.
pub(crate) fn area(rect: Rect) -> String {
    let first = cell_name(rect.r0, rect.c0);
    if rect.is_single() {
        first
    } else {
        format!("{first}:{}", cell_name(rect.r1, rect.c1))
    }
}

/// Append an attribute holding text, with the `_xHHHH_` escapes of
/// SpreadsheetML strings.
pub(crate) fn push_text_attr(x: &mut String, name: &str, value: &str) {
    let mut text = String::new();
    push_cell_text(&mut text, value);
    // That was escaped as the content of an element; an attribute also
    // has to escape its quotes, and the white space a parser would turn
    // into spaces, which Excel writes the way it writes control characters.
    let text = text
        .replace('"', "&quot;")
        .replace('\n', "_x000A_")
        .replace('\t', "_x0009_");
    w!(x, " {name}=\"{text}\"");
}

fn push_criteria(
    x: &mut String,
    criteria: &Criteria,
    values: Option<(f64, Option<f64>)>,
    dxfs: &mut Dxfs,
) {
    match criteria {
        Criteria::Values(filter) => {
            x.push_str("<filters");
            if filter.blanks {
                x.push_str(" blank=\"1\"");
            }
            if filter.values.is_empty() && filter.dates.is_empty() {
                x.push_str("/>");
                return;
            }
            x.push('>');
            for value in &filter.values {
                x.push_str("<filter");
                push_text_attr(x, "val", value);
                x.push_str("/>");
            }
            for date in &filter.dates {
                w!(x, "<dateGroupItem year=\"{}\"", date.year);
                for (name, part) in [
                    ("month", date.month),
                    ("day", date.day),
                    ("hour", date.hour),
                    ("minute", date.minute),
                    ("second", date.second),
                ] {
                    // A part below the depth of the group is not said.
                    let Some(part) = part else {
                        break;
                    };
                    w!(x, " {name}=\"{part}\"");
                }
                w!(x, " dateTimeGrouping=\"{}\"/>", date.grouping());
            }
            x.push_str("</filters>");
        }
        Criteria::Custom(custom) => {
            x.push_str("<customFilters");
            if custom.and && custom.second.is_some() {
                x.push_str(" and=\"1\"");
            }
            x.push('>');
            for condition in std::iter::once(&custom.first).chain(&custom.second) {
                x.push_str("<customFilter");
                // "Equal" is what a filter without operator is.
                if condition.op != CustomOp::Equal {
                    w!(x, " operator=\"{}\"", condition.op.ooxml_name());
                }
                push_text_attr(x, "val", &condition.value);
                x.push_str("/>");
            }
            x.push_str("</customFilters>");
        }
        Criteria::Top10(top) => {
            x.push_str("<top10");
            if !top.top {
                x.push_str(" top=\"0\"");
            }
            if top.percent {
                x.push_str(" percent=\"1\"");
            }
            x.push_str(" val=\"");
            push_number(x, finite(top.value));
            x.push('"');
            if let Some(cut_off) = top.filter_value.filter(|v| v.is_finite()) {
                x.push_str(" filterVal=\"");
                push_number(x, cut_off);
                x.push('"');
            }
            x.push_str("/>");
        }
        Criteria::Dynamic(dynamic) => {
            w!(x, "<dynamicFilter type=\"{}\"", dynamic.ooxml_type());
            // What the filter worked out last: the average, or the first
            // day of the period and the day after its last.
            if let Some((value, max)) = values.filter(|(v, _)| v.is_finite()) {
                x.push_str(" val=\"");
                push_number(x, value);
                x.push('"');
                if let Some(max) = max.filter(|v| v.is_finite()) {
                    x.push_str(" maxVal=\"");
                    push_number(x, max);
                    x.push('"');
                }
            }
            x.push_str("/>");
        }
        Criteria::Color(color) => {
            if color.font {
                let id = font_color_id(dxfs, color.color);
                w!(x, "<colorFilter dxfId=\"{id}\" cellColor=\"0\"/>");
            } else {
                let id = dxfs.cell_fill_id(color.color);
                w!(x, "<colorFilter dxfId=\"{id}\"/>");
            }
        }
    }
}

/// The `dxfId` of the text colour a filter or a sort looks for (`None`:
/// automatic).
fn font_color_id(dxfs: &mut Dxfs, color: Option<Color>) -> u32 {
    dxfs.id(&StylePatch {
        font_color: Some(color),
        ..StylePatch::default()
    })
}

/// Append the `<autoFilter>` of a sheet's own filter, if it has one.
pub(crate) fn write_sheet(x: &mut String, wb: &Workbook, sheet: usize, dxfs: &mut Dxfs) {
    push_auto_filter(x, wb, sheet, FilterScope::Sheet, dxfs);
}

/// Whether any sheet has a filter of its own or an Advanced Filter, which
/// a file also names (see [`push_names`]).
pub(crate) fn has_names(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sheet| sheet.auto_filter.is_some() || sheet.advanced_filter.is_some())
}

/// `Sheet1!$A$1:$C$7`.
fn name_text(sheet: &str, range: Rect) -> String {
    let mut text = String::new();
    opensheet_engine::ast::write_sheet_name(sheet, &mut text);
    text.push('!');
    for (n, (row, col)) in [(range.r0, range.c0), (range.r1, range.c1)]
        .into_iter()
        .enumerate()
    {
        if n == 1 {
            text.push(':');
        }
        let name = cell_name(row, col);
        let letters = name.trim_end_matches(|c: char| c.is_ascii_digit());
        w!(text, "${letters}${}", row + 1);
    }
    text
}

/// Append the hidden defined names Excel keeps beside the filters of the
/// sheets (children of `<definedNames>`): `_xlnm._FilterDatabase`, the
/// range of the filter with its header row (the list, for an Advanced
/// Filter in place), and `_xlnm.Criteria`, the criteria range of an
/// Advanced Filter.
pub(crate) fn push_names(x: &mut String, wb: &Workbook) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let advanced = sheet.advanced_filter.as_ref();
        if let Some(criteria) = advanced.and_then(|filter| filter.criteria) {
            w!(x, "<definedName name=\"_xlnm.Criteria\" localSheetId=\"{index}\">");
            push_cell_text(x, &name_text(&sheet.name, criteria));
            x.push_str("</definedName>");
        }
        let Some(range) = sheet.auto_filter.as_ref().map(|filter| filter.range).or(advanced.map(|filter| filter.range)) else {
            continue;
        };
        w!(
            x,
            "<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"{index}\" hidden=\"1\">"
        );
        push_cell_text(x, &name_text(&sheet.name, range));
        x.push_str("</definedName>");
    }
}

/// The range a filter name of a file stands for (`Sheet1!$A$1:$C$7`).
fn name_range(text: &str) -> Option<Rect> {
    let cells = text.rsplit('!').next()?;
    Rect::parse(&cells.replace('$', ""))
}

/// `<sheetPr filterMode="1">` was read on a sheet: rows are hidden by a
/// filter. When the sheet turns out to have no `<autoFilter>`, it is an
/// Advanced Filter, whose list the names of the workbook give
/// ([`read_name`]).
pub(crate) fn read_filter_mode(sheet: &mut Sheet) {
    sheet.advanced_filter = Some(AdvancedFilter::default());
}

/// Take a defined name of the workbook part if it belongs to the filters
/// (`name` in upper case): the list and the criteria of an Advanced
/// Filter are read, the others only recognised. Returns whether it was
/// one of them.
pub(crate) fn read_name(name: &str, text: &str, sheet: Option<&mut Sheet>) -> bool {
    let list = name == "_XLNM._FILTERDATABASE";
    if !list && name != "_XLNM.CRITERIA" {
        return false;
    }
    let Some(sheet) = sheet else {
        return true;
    };
    let (Some(filter), Some(range)) = (sheet.advanced_filter.as_mut(), name_range(text)) else {
        return true;
    };
    if !list {
        filter.criteria = Some(range);
    } else if sheet.auto_filter.is_none() && range.rows() > 1 {
        // The rows hidden under the header of the list are those the
        // filter hides: the file is the truth, as for an AutoFilter.
        filter.range = range;
        let rows: Vec<u32> = sheet.hidden_rows.range(range.r0 + 1..=range.r1).copied().collect();
        for row in rows {
            sheet.hidden_rows.remove(&row);
            sheet.filtered_rows.insert(row);
            filter.hidden.insert(row);
        }
    }
    true
}

/// Once the names are read: a sheet in filter mode without the list of an
/// Advanced Filter was filtered by its AutoFilter or by a table.
pub(crate) fn finish_names(wb: &mut Workbook) {
    for sheet in &mut wb.sheets {
        if sheet.advanced_filter.as_ref().is_some_and(|filter| filter.range.rows() < 2) {
            sheet.advanced_filter = None;
        }
    }
}

fn finite(n: f64) -> f64 {
    if n.is_finite() { n } else { 0.0 }
}

/// Append the `<sortState>` of a sort of the rows `data` of a filter.
fn push_sort(x: &mut String, sort: &SortState, data: Rect, dxfs: &mut Dxfs) {
    w!(x, "<sortState ref=\"{}\"", area(data));
    if sort.case_sensitive {
        x.push_str(" caseSensitive=\"1\"");
    }
    x.push('>');
    for level in sort.levels.iter().filter(|level| level.key < data.cols()) {
        x.push_str("<sortCondition");
        if level.descending {
            x.push_str(" descending=\"1\"");
        }
        match level.on {
            SortOn::Values => {}
            SortOn::CellColor(_) => x.push_str(" sortBy=\"cellColor\""),
            SortOn::FontColor(_) => x.push_str(" sortBy=\"fontColor\""),
        }
        let col = data.c0 + level.key;
        w!(
            x,
            " ref=\"{}\"",
            area(Rect::new(data.r0, col, data.r1, col))
        );
        match (&level.on, &level.custom_list) {
            (SortOn::CellColor(color), _) => {
                w!(x, " dxfId=\"{}\"", dxfs.cell_fill_id(*color));
            }
            (SortOn::FontColor(color), _) => {
                w!(x, " dxfId=\"{}\"", font_color_id(dxfs, *color));
            }
            (SortOn::Values, Some(list)) => push_text_attr(x, "customList", &list.join(",")),
            (SortOn::Values, None) => {}
        }
        x.push_str("/>");
    }
    x.push_str("</sortState>");
}

/// Append the `<autoFilter>` of a sheet (`FilterScope::Sheet`) or of one
/// of its tables, with the sort made through it; nothing when it has no
/// filter. The colours its criteria and its sort go by are added to
/// `dxfs`.
///
/// Excel puts the `<sortState>` of a sheet's filter inside the
/// `<autoFilter>` and that of a table's right after it; both places are
/// read.
pub(crate) fn push_auto_filter(
    x: &mut String,
    wb: &Workbook,
    sheet: usize,
    scope: FilterScope,
    dxfs: &mut Dxfs,
) {
    let Some(filter) = wb.auto_filter(sheet, scope) else {
        return;
    };
    let range = filter.range;
    w!(x, "<autoFilter ref=\"{}\"", area(range));
    let width = range.cols();
    let columns: BTreeSet<u32> = filter
        .columns
        .keys()
        .chain(&filter.hidden_buttons)
        .copied()
        .filter(|id| *id < width)
        .collect();
    // The data rows: what a sort moves.
    let sort = filter
        .sort
        .as_ref()
        .filter(|sort| !sort.levels.is_empty())
        .zip(filter.data_range());
    let (inside, after) = match scope {
        FilterScope::Sheet => (sort, None),
        FilterScope::Table(_) => (None, sort),
    };
    if columns.is_empty() && inside.is_none() {
        x.push_str("/>");
    } else {
        x.push('>');
        for id in columns {
            w!(x, "<filterColumn colId=\"{id}\"");
            if filter.hidden_buttons.contains(&id) {
                x.push_str(" hiddenButton=\"1\"");
            }
            match filter.columns.get(&id) {
                Some(criteria) => {
                    x.push('>');
                    let values = wb.dynamic_filter_values(sheet, scope, id);
                    push_criteria(x, criteria, values, dxfs);
                    x.push_str("</filterColumn>");
                }
                None => x.push_str("/>"),
            }
        }
        if let Some((sort, data)) = inside {
            push_sort(x, sort, data, dxfs);
        }
        x.push_str("</autoFilter>");
    }
    if let Some((sort, data)) = after {
        push_sort(x, sort, data, dxfs);
    }
}

fn text_attr(tag: &Tag, name: &str) -> Option<String> {
    tag.attr(name).map(|v| decode_cell_text(&v).into_owned())
}

/// The colours of one `<dxf>` of a file that is read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DxfColors {
    /// The colour of the text; `None` when the format has none or says
    /// automatic.
    pub font: Option<Color>,
    /// The fill; `None` when the format has none or says no fill.
    pub fill: Option<Color>,
}

/// The colours of the `<dxf>` elements of a styles part, by `dxfId`.
pub(crate) fn parse_colors(styles: &str, palette: &Palette) -> Vec<DxfColors> {
    let mut out = Vec::new();
    let mut r = Reader::new(styles);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "styleSheet" => {}
            Event::Open(t) if t.name == "dxfs" && !t.empty => loop {
                match r.next() {
                    Event::Open(dxf) if dxf.name == "dxf" => {
                        let mut colors = DxfColors::default();
                        // Foreground and background of the pattern, and
                        // whether it is one that shows.
                        let (mut fg, mut bg, mut solid) = (None, None, true);
                        let mut font = false;
                        let mut depth = usize::from(!dxf.empty);
                        while depth > 0 {
                            match r.next() {
                                Event::Open(t) => {
                                    match t.name {
                                        "font" => font = !t.empty,
                                        "color" if font => {
                                            colors.font = RawColor::from_tag(&t).resolve(palette);
                                        }
                                        "patternFill" => {
                                            solid =
                                                t.attr("patternType").as_deref() != Some("none");
                                        }
                                        "fgColor" => fg = RawColor::from_tag(&t).resolve(palette),
                                        "bgColor" => bg = RawColor::from_tag(&t).resolve(palette),
                                        _ => {}
                                    }
                                    depth += usize::from(!t.empty);
                                }
                                Event::Close(name) => {
                                    font &= name != "font";
                                    depth -= 1;
                                }
                                Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                        // Excel writes the colour a filter goes by as the
                        // foreground of a solid pattern, and the fill of a
                        // conditional format as the background.
                        colors.fill = fg.or(bg).filter(|_| solid);
                        out.push(colors);
                    }
                    Event::Open(t) => r.skip(&t),
                    Event::Close(_) | Event::Eof => break,
                    Event::Text(_) => {}
                }
            },
            Event::Open(t) => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// The children of a `<filterColumn>`: its criterion, if it is one the
/// engine has.
fn parse_criteria<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, colors: &[DxfColors]) -> Option<Criteria> {
    let mut criteria = None;
    if tag.empty {
        return None;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let parsed = match t.name {
                    "filters" => {
                        let mut filter = ValueFilter {
                            blanks: t.attr_bool("blank").unwrap_or(false),
                            ..ValueFilter::default()
                        };
                        if !t.empty {
                            loop {
                                match r.next() {
                                    Event::Open(c) => {
                                        match c.name {
                                            "filter" => filter.values.extend(text_attr(&c, "val")),
                                            "dateGroupItem" => filter.dates.extend(date_group(&c)),
                                            _ => {}
                                        }
                                        r.skip(&c);
                                    }
                                    Event::Close(_) | Event::Eof => break,
                                    Event::Text(_) => {}
                                }
                            }
                        }
                        Some(Criteria::Values(filter))
                    }
                    "customFilters" => {
                        let and = t.attr_bool("and").unwrap_or(false);
                        let mut conditions = Vec::new();
                        if !t.empty {
                            loop {
                                match r.next() {
                                    Event::Open(c) => {
                                        if c.name == "customFilter" {
                                            let op = match c.attr("operator") {
                                                Some(name) => CustomOp::from_ooxml_name(&name),
                                                None => Some(CustomOp::Equal),
                                            };
                                            if let (Some(op), Some(value)) =
                                                (op, text_attr(&c, "val"))
                                            {
                                                conditions.push(Condition { op, value });
                                            }
                                        }
                                        r.skip(&c);
                                    }
                                    Event::Close(_) | Event::Eof => break,
                                    Event::Text(_) => {}
                                }
                            }
                        }
                        let mut conditions = conditions.into_iter();
                        conditions.next().map(|first| {
                            let second = conditions.next();
                            Criteria::Custom(CustomFilter {
                                and: and && second.is_some(),
                                first,
                                second,
                            })
                        })
                    }
                    "top10" => {
                        let top = Top10 {
                            top: t.attr_bool("top").unwrap_or(true),
                            percent: t.attr_bool("percent").unwrap_or(false),
                            value: t.attr_f64("val").unwrap_or(10.0),
                            filter_value: t.attr_f64("filterVal"),
                        };
                        r.skip(&t);
                        Some(Criteria::Top10(top))
                    }
                    "dynamicFilter" => {
                        let dynamic = t
                            .attr("type")
                            .and_then(|name| DynamicFilter::from_ooxml_type(&name));
                        r.skip(&t);
                        dynamic.map(Criteria::Dynamic)
                    }
                    "colorFilter" => {
                        // A text colour when `cellColor` says no.
                        let font = !t.attr_bool("cellColor").unwrap_or(true);
                        let format = t
                            .attr_u32("dxfId")
                            .and_then(|id| colors.get(id as usize))
                            .copied()
                            .unwrap_or_default();
                        r.skip(&t);
                        Some(Criteria::Color(ColorFilter {
                            color: if font { format.font } else { format.fill },
                            font,
                        }))
                    }
                    _ => {
                        r.skip(&t);
                        None
                    }
                };
                // A column has one criterion: the first that reads.
                if criteria.is_none() {
                    criteria = parsed;
                }
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    criteria
}

/// A `<dateGroupItem>`; `None` when it does not say what its grouping
/// promises.
fn date_group(tag: &Tag) -> Option<DateGroup> {
    let year = tag.attr("year")?.trim().parse::<i32>().ok()?;
    let part = |name: &str, max: u32| tag.attr_u32(name).filter(|n| *n <= max);
    let depth = match tag.attr("dateTimeGrouping").as_deref() {
        Some("year") => 0,
        Some("month") => 1,
        Some("day") => 2,
        Some("hour") => 3,
        Some("minute") => 4,
        Some("second") => 5,
        _ => return None,
    };
    let parts = [
        part("month", 12).filter(|n| *n >= 1),
        part("day", 31).filter(|n| *n >= 1),
        part("hour", 23),
        part("minute", 59),
        part("second", 59),
    ];
    // Every part down to the depth is needed, none below it counts.
    if parts[..depth].iter().any(Option::is_none) {
        return None;
    }
    let at = |i: usize| parts[i].filter(|_| i < depth);
    Some(DateGroup {
        year,
        month: at(0),
        day: at(1),
        hour: at(2),
        minute: at(3),
        second: at(4),
    })
}

/// A `<sortState>` (`tag`, just opened) of the filter on `range`.
pub(crate) fn parse_sort<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    range: Rect,
    colors: &[DxfColors],
) -> SortState {
    let mut sort = SortState {
        levels: Vec::new(),
        case_sensitive: tag.attr_bool("caseSensitive").unwrap_or(false),
    };
    if tag.empty {
        return sort;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                // The key is the column the condition names.
                let key = t
                    .attr("ref")
                    .and_then(|name| Rect::parse(&name))
                    .filter(|key| key.c0 >= range.c0 && key.c0 <= range.c1)
                    .map(|key| key.c0 - range.c0);
                let format = t
                    .attr_u32("dxfId")
                    .and_then(|id| colors.get(id as usize))
                    .copied()
                    .unwrap_or_default();
                let on = match t.attr("sortBy").as_deref() {
                    None | Some("value") => Some(SortOn::Values),
                    Some("cellColor") => Some(SortOn::CellColor(format.fill)),
                    Some("fontColor") => Some(SortOn::FontColor(format.font)),
                    Some(_) => None,
                };
                if let (true, Some(key), Some(on)) = (t.name == "sortCondition", key, on) {
                    let custom_list = text_attr(&t, "customList")
                        .filter(|_| on == SortOn::Values)
                        .map(|list| list.split(',').map(str::to_string).collect());
                    sort.levels.push(SortLevel {
                        key,
                        on,
                        descending: t.attr_bool("descending").unwrap_or(false),
                        custom_list,
                    });
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    sort
}

/// Read an `<autoFilter>` element (`tag`, just opened). `colors` are the
/// differential formats of the workbook, for the criteria and sorts that
/// go by a colour. `None` when the element does not say what it covers.
pub(crate) fn parse_auto_filter<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    colors: &[DxfColors],
) -> Option<AutoFilter> {
    let Some(range) = tag.attr("ref").and_then(|name| Rect::parse(&name)) else {
        r.skip(tag);
        return None;
    };
    let mut filter = AutoFilter::new(range);
    if tag.empty {
        return Some(filter);
    }
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "filterColumn" => {
                    let id = t.attr_u32("colId").filter(|id| *id < range.cols());
                    let hidden = t.attr_bool("hiddenButton").unwrap_or(false)
                        || !t.attr_bool("showButton").unwrap_or(true);
                    let criteria = parse_criteria(r, &t, colors);
                    if let Some(id) = id {
                        if hidden {
                            filter.hidden_buttons.insert(id);
                        }
                        if let Some(criteria) = criteria {
                            filter.columns.entry(id).or_insert(criteria);
                        }
                    }
                }
                "sortState" => {
                    let sort = parse_sort(r, &t, range, colors);
                    if !sort.levels.is_empty() {
                        filter.sort = Some(sort);
                    }
                }
                _ => r.skip(&t),
            },
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    Some(filter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;

    fn parse(xml: &str, colors: &[DxfColors]) -> Option<AutoFilter> {
        let mut r = Reader::new(xml);
        loop {
            match r.next() {
                Event::Open(t) if t.name == "autoFilter" => {
                    return parse_auto_filter(&mut r, &t, colors);
                }
                Event::Eof => return None,
                _ => {}
            }
        }
    }

    /// A workbook whose sheet filter is `filter`.
    fn with_filter(filter: AutoFilter) -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        for row in 0..=filter.range.r1.min(20) {
            for col in filter.range.c0..=filter.range.c1.min(10) {
                let text = if row == 0 {
                    format!("h{col}")
                } else {
                    format!("{}", row * (col + 1))
                };
                wb.set_cell_text(0, row, col, &text).unwrap();
            }
        }
        wb.sheets[0].auto_filter = Some(filter);
        wb
    }

    fn written(wb: &Workbook) -> (String, Dxfs) {
        let mut dxfs = Dxfs::new(Vec::new(), Locale::en());
        let mut x = String::new();
        push_auto_filter(&mut x, wb, 0, FilterScope::Sheet, &mut dxfs);
        (x, dxfs)
    }

    #[test]
    fn every_form_of_criterion() {
        let mut filter = AutoFilter::new(Rect::new(0, 0, 9, 7));
        filter.columns.insert(
            0,
            Criteria::Values(ValueFilter {
                values: vec!["a & b".into(), "\"q\" <x>".into(), "line\nbreak".into()],
                blanks: true,
                dates: vec![
                    DateGroup::year(2024),
                    DateGroup::month(2023, 12),
                    DateGroup::day(2023, 1, 31),
                    DateGroup {
                        hour: Some(13),
                        minute: Some(5),
                        second: Some(59),
                        ..DateGroup::day(2022, 2, 28)
                    },
                ],
            }),
        );
        filter.columns.insert(
            1,
            Criteria::Custom(CustomFilter {
                first: Condition {
                    op: CustomOp::GreaterOrEqual,
                    value: "1.5".into(),
                },
                second: Some(Condition {
                    op: CustomOp::Equal,
                    value: "a*~?".into(),
                }),
                and: true,
            }),
        );
        filter.columns.insert(
            2,
            Criteria::Top10(Top10 {
                top: false,
                percent: true,
                value: 25.0,
                filter_value: Some(4.5),
            }),
        );
        filter
            .columns
            .insert(3, Criteria::Dynamic(DynamicFilter::AboveAverage));
        filter
            .columns
            .insert(4, Criteria::Dynamic(DynamicFilter::Quarter(3)));
        filter.columns.insert(
            5,
            Criteria::Color(ColorFilter {
                color: Some(Color(0xFFFF00)),
                font: false,
            }),
        );
        filter.columns.insert(
            6,
            Criteria::Color(ColorFilter {
                color: None,
                font: true,
            }),
        );
        filter.hidden_buttons.extend([6, 7]);
        filter.sort = Some(SortState {
            case_sensitive: true,
            levels: vec![
                SortLevel::values(1, true),
                SortLevel {
                    key: 0,
                    on: SortOn::Values,
                    descending: false,
                    custom_list: Some(vec!["Mon".into(), "Tue".into()]),
                },
                SortLevel {
                    key: 5,
                    on: SortOn::CellColor(Some(Color(0xFFFF00))),
                    descending: true,
                    custom_list: None,
                },
                SortLevel {
                    key: 2,
                    on: SortOn::FontColor(Some(Color(0xC00000))),
                    descending: false,
                    custom_list: None,
                },
            ],
        });
        let wb = with_filter(filter.clone());
        let (xml, mut dxfs) = written(&wb);
        assert_eq!(
            xml,
            concat!(
                "<autoFilter ref=\"A1:H10\">",
                "<filterColumn colId=\"0\"><filters blank=\"1\"><filter val=\"a &amp; b\"/><filter val=\"&quot;q&quot; &lt;x&gt;\"/><filter val=\"line_x000A_break\"/>",
                "<dateGroupItem year=\"2024\" dateTimeGrouping=\"year\"/><dateGroupItem year=\"2023\" month=\"12\" dateTimeGrouping=\"month\"/><dateGroupItem year=\"2023\" month=\"1\" day=\"31\" dateTimeGrouping=\"day\"/>",
                "<dateGroupItem year=\"2022\" month=\"2\" day=\"28\" hour=\"13\" minute=\"5\" second=\"59\" dateTimeGrouping=\"second\"/></filters></filterColumn>",
                "<filterColumn colId=\"1\"><customFilters and=\"1\"><customFilter operator=\"greaterThanOrEqual\" val=\"1.5\"/><customFilter val=\"a*~?\"/></customFilters></filterColumn>",
                "<filterColumn colId=\"2\"><top10 top=\"0\" percent=\"1\" val=\"25\" filterVal=\"4.5\"/></filterColumn>",
                "<filterColumn colId=\"3\"><dynamicFilter type=\"aboveAverage\" val=\"20\"/></filterColumn>",
                "<filterColumn colId=\"4\"><dynamicFilter type=\"Q3\"/></filterColumn>",
                "<filterColumn colId=\"5\"><colorFilter dxfId=\"0\"/></filterColumn>",
                "<filterColumn colId=\"6\" hiddenButton=\"1\"><colorFilter dxfId=\"1\" cellColor=\"0\"/></filterColumn>",
                "<filterColumn colId=\"7\" hiddenButton=\"1\"/>",
                "<sortState ref=\"A2:H10\" caseSensitive=\"1\"><sortCondition descending=\"1\" ref=\"B2:B10\"/><sortCondition ref=\"A2:A10\" customList=\"Mon,Tue\"/>",
                "<sortCondition descending=\"1\" sortBy=\"cellColor\" ref=\"F2:F10\" dxfId=\"0\"/><sortCondition sortBy=\"fontColor\" ref=\"C2:C10\" dxfId=\"2\"/></sortState>",
                "</autoFilter>"
            )
        );
        // And back, through the formats the colours went to.
        let styles = format!("<styleSheet>{}</styleSheet>", dxfs.xml());
        assert_eq!(
            styles,
            concat!(
                "<styleSheet><dxfs count=\"3\">",
                "<dxf><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFFFFF00\"/><bgColor rgb=\"FFFFFF00\"/></patternFill></fill></dxf>",
                "<dxf><font><color auto=\"1\"/></font></dxf>",
                "<dxf><font><color rgb=\"FFC00000\"/></font></dxf>",
                "</dxfs></styleSheet>"
            )
        );
        let colors = parse_colors(&styles, &Palette::default());
        assert_eq!(parse(&xml, &colors), Some(filter));
    }

    #[test]
    fn plain_filters() {
        let wb = with_filter(AutoFilter::new(Rect::new(0, 0, 4, 2)));
        assert_eq!(written(&wb).0, "<autoFilter ref=\"A1:C5\"/>");
        assert_eq!(
            parse("<autoFilter ref=\"A1:C5\"/>", &[]),
            Some(AutoFilter::new(Rect::new(0, 0, 4, 2)))
        );
        // Whole columns are spelled out as cells.
        let wb = with_filter(AutoFilter::new(Rect::new(0, 1, 1_048_575, 2)));
        assert_eq!(written(&wb).0, "<autoFilter ref=\"B1:C1048576\"/>");
        // Nothing checked at all, and a filter without data rows.
        let mut filter = AutoFilter::new(Rect::new(0, 0, 0, 1));
        filter
            .columns
            .insert(1, Criteria::Values(ValueFilter::default()));
        filter.sort = Some(SortState {
            levels: vec![SortLevel::values(0, false)],
            case_sensitive: false,
        });
        let wb = with_filter(filter);
        assert_eq!(
            written(&wb).0,
            "<autoFilter ref=\"A1:B1\"><filterColumn colId=\"1\"><filters/></filterColumn></autoFilter>"
        );
        // No filter, no element.
        assert_eq!(written(&Workbook::new(Locale::en())).0, "");
    }

    #[test]
    fn filters_from_other_applications() {
        let colors = [
            DxfColors {
                font: Some(Color(0xFF0000)),
                fill: Some(Color(0x00FF00)),
            },
            DxfColors::default(),
        ];
        let xml = r#"<x:autoFilter ref="$B$2:$E$20" xr:uid="{0}">
            <x:filterColumn colId="0" showButton="0"><x:filters calendarType="gregorian"><x:filter val="a_x000D_b"/><x:filter/><x:dateGroupItem year="2024" month="2" day="30" hour="1" dateTimeGrouping="month"/><x:dateGroupItem year="2024" dateTimeGrouping="day"/><x:dateGroupItem month="2" dateTimeGrouping="month"/></x:filters></x:filterColumn>
            <x:filterColumn colId="1"><x:customFilters><x:customFilter operator="notEqual" val=""/><x:customFilter operator="bogus" val="1"/><x:customFilter operator="lessThan" val="9"/><x:customFilter val="extra"/></x:customFilters></x:filterColumn>
            <x:filterColumn colId="2"><x:iconFilter iconSet="3Arrows" iconId="0"/></x:filterColumn>
            <x:filterColumn colId="3"><x:colorFilter dxfId="0" cellColor="1"/><x:top10 val="3"/></x:filterColumn>
            <x:filterColumn colId="4"><x:top10 val="3"/></x:filterColumn>
            <x:filterColumn colId="1"><x:top10 val="5"/></x:filterColumn>
            <x:filterColumn><x:top10 val="5"/></x:filterColumn>
            <x:sortState ref="B3:E20" columnSort="0"><x:sortCondition ref="C3:C20" sortBy="icon" iconSet="3Arrows"/><x:sortCondition ref="Z3:Z20"/><x:sortCondition ref="E3:E20" sortBy="fontColor" dxfId="0"/><x:sortCondition ref="D3" sortBy="cellColor" dxfId="7"/><x:other ref="B3:B20"/></x:sortState>
            <x:extLst><x:ext><x:filterColumn colId="2"><x:top10 val="1"/></x:filterColumn></x:ext></x:extLst>
        </x:autoFilter>"#;
        let filter = parse(xml, &colors).unwrap();
        assert_eq!(filter.range, Rect::new(1, 1, 19, 4));
        assert_eq!(
            filter.hidden_buttons.iter().copied().collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            filter.columns.get(&0),
            Some(&Criteria::Values(ValueFilter {
                values: vec!["a\rb".into()],
                blanks: false,
                dates: vec![DateGroup::month(2024, 2)],
            }))
        );
        assert_eq!(
            filter.columns.get(&1),
            Some(&Criteria::Custom(CustomFilter {
                first: Condition {
                    op: CustomOp::NotEqual,
                    value: String::new()
                },
                second: Some(Condition {
                    op: CustomOp::Less,
                    value: "9".into()
                }),
                and: false,
            }))
        );
        assert_eq!(filter.columns.get(&2), None, "icons are not filtered on");
        assert_eq!(
            filter.columns.get(&3),
            Some(&Criteria::Color(ColorFilter {
                color: Some(Color(0x00FF00)),
                font: false
            }))
        );
        assert_eq!(filter.columns.len(), 3, "column 4 is off the range");
        assert_eq!(
            filter.sort,
            Some(SortState {
                case_sensitive: false,
                levels: vec![
                    SortLevel {
                        key: 3,
                        on: SortOn::FontColor(Some(Color(0xFF0000))),
                        descending: false,
                        custom_list: None,
                    },
                    SortLevel {
                        key: 2,
                        on: SortOn::CellColor(None),
                        descending: false,
                        custom_list: None,
                    },
                ],
            })
        );
        // Without a range it is no filter, and the element is passed over.
        let mut r =
            Reader::new("<s><autoFilter><filterColumn colId=\"0\"/></autoFilter><next/></s>");
        let Event::Open(_) = r.next() else { panic!() };
        let Event::Open(t) = r.next() else { panic!() };
        assert_eq!(parse_auto_filter(&mut r, &t, &[]), None);
        assert!(matches!(r.next(), Event::Open(t) if t.name == "next"));
        // Cut short anywhere, it still ends.
        for cut in (0..xml.len()).filter(|i| xml.is_char_boundary(*i)) {
            parse(&xml[..cut], &colors);
        }
    }

    #[test]
    fn colours_of_formats_from_excel() {
        // A filter colour, a conditional format, theme and indexed colours,
        // and elements that are not colours of the format itself.
        let styles = r#"<styleSheet><fonts><font><color rgb="FF123456"/></font></fonts><dxfs count="5">
            <dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor rgb="FF000000"/></patternFill></fill></dxf>
            <dxf><font><b/><color rgb="FF9C0006"/></font><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill><border><left style="thin"><color rgb="FF00FF00"/></left></border></dxf>
            <dxf><font><color theme="4"/></font><fill><patternFill><bgColor indexed="10"/></patternFill></fill></dxf>
            <dxf/>
            <dxf><fill><patternFill patternType="none"><fgColor rgb="FFFF0000"/></patternFill></fill></dxf>
            </dxfs><extLst><ext><dxfs><dxf><font><color rgb="FF000001"/></font></dxf></dxfs></ext></extLst></styleSheet>"#;
        let colors = parse_colors(styles, &Palette::default());
        assert_eq!(colors.len(), 5);
        assert_eq!(
            colors[0],
            DxfColors {
                font: None,
                fill: Some(Color(0xFFFF00))
            }
        );
        assert_eq!(
            colors[1],
            DxfColors {
                font: Some(Color(0x9C0006)),
                fill: Some(Color(0xFFC7CE))
            }
        );
        assert_eq!(colors[2].font.map(Color::plain), Some(Color(0x4472C4)));
        assert_eq!(colors[2].fill, Some(Color(0xFF0000)));
        assert_eq!(colors[3], DxfColors::default());
        assert_eq!(colors[4], DxfColors::default());
        // Cut short anywhere, it still ends.
        for cut in (0..styles.len()).filter(|i| styles.is_char_boundary(*i)) {
            parse_colors(&styles[..cut], &Palette::default());
        }
    }
}
