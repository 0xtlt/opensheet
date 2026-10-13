//! Timelines (`Sheet::timelines`).
//!
//! Kept like slicers (see `slicer.rs`), in parts of their own kind: the
//! *cache* (`xl/timelineCaches/timelineCacheN.xml`, related from the
//! workbook and listed in its `<extLst>`) names the date field, the
//! PivotTables it filters, the days selected and the days the band runs
//! over; the *timeline* (`xl/timelines/timelineN.xml`, related from its
//! worksheet and named in the worksheet's `<extLst>`) holds the caption,
//! the level, what is shown and the style; a frame of the drawing says
//! where it is.
//!
//! The selection itself is the date filter of the field in each
//! PivotTable (`<filter type="dateBetween">`, see `pivot.rs`); the one of
//! the cache is read back for the PivotTables that lack it.

use super::filter::push_text_attr;
use super::parts::{CacheList, Extras, SheetLinks};
use super::pivot::{date_from_iso, iso_date};
use super::read::{Package, Rel, relationships};
use super::slicer::{NS_X15, pivot_cache_number, push_pivot_tables, read_frames, read_pivot_tables, sheets_by_id};
use super::write::NS_MAIN;
use super::xml::{Event, Reader, XML_DECL, decode_cell_text};
use opensheet_engine::Workbook;
use opensheet_engine::slicer::{TIMELINE_SIZE, TimeLevel, Timeline};
use std::fmt::Write as _;
use std::io::{Read, Seek};

const CT_TIMELINE: &str = "application/vnd.ms-excel.timeline+xml";
const CT_TIMELINE_CACHE: &str = "application/vnd.ms-excel.timelineCache+xml";
const REL_TIMELINE: &str = "http://schemas.microsoft.com/office/2011/relationships/timeline";
const REL_TIMELINE_CACHE: &str = "http://schemas.microsoft.com/office/2011/relationships/timelineCache";

const DEFAULT_STYLE: &str = "TimeSlicerStyleLight1";

fn level_number(level: TimeLevel) -> u32 {
    TimeLevel::ALL.iter().position(|l| *l == level).unwrap_or(2) as u32
}

/// The part of a timeline cache.
fn cache_xml(wb: &Workbook, id: u32, timeline: &Timeline) -> Option<String> {
    let (_, p) = timeline.pivots.iter().find_map(|id| wb.pivot(*id))?;
    let cache = pivot_cache_number(wb, &p.cache)?;
    let mut x = String::from(XML_DECL);
    w!(x, "<timelineCacheDefinition xmlns=\"{NS_X15}\" xmlns:x=\"{NS_MAIN}\"");
    push_text_attr(&mut x, "name", &timeline.cache_name);
    push_text_attr(&mut x, "sourceName", &timeline.field);
    x.push('>');
    push_pivot_tables(&mut x, wb, &timeline.pivots);
    let selection = wb.timeline_selection(id);
    w!(
        x,
        "<state minimalRefreshVersion=\"6\" lastRefreshVersion=\"6\" pivotCacheId=\"{cache}\" filterType=\"{}\">",
        if selection.is_some() { "dateBetween" } else { "unknown" }
    );
    // Days are said from their first instant to the first instant of the
    // last one.
    if let Some((start, end)) = selection {
        w!(x, "<selection startDate=\"{}\" endDate=\"{}\"/>", iso_date(start), iso_date(end));
    }
    if let Some((start, end)) = wb.timeline_bounds(id) {
        w!(x, "<bounds startDate=\"{}\" endDate=\"{}\"/>", iso_date(start), iso_date(end + 1.0));
    }
    x.push_str("</state></timelineCacheDefinition>");
    Some(x)
}

/// The part listing the timelines of one sheet.
fn timelines_xml(wb: &Workbook, timelines: &[Timeline]) -> String {
    let mut x = String::from(XML_DECL);
    w!(x, "<timelines xmlns=\"{NS_X15}\" xmlns:x=\"{NS_MAIN}\">");
    for t in timelines {
        x.push_str("<timeline");
        push_text_attr(&mut x, "name", &t.name);
        push_text_attr(&mut x, "cache", &t.cache_name);
        push_text_attr(&mut x, "caption", &t.caption);
        for (shown, name) in [
            (t.show_header, "showHeader"),
            (t.show_selection_label, "showSelectionLabel"),
            (t.show_time_level, "showTimeLevel"),
            (t.show_scrollbar, "showHorizontalScrollbar"),
        ] {
            if !shown {
                w!(x, " {name}=\"0\"");
            }
        }
        let level = level_number(t.level);
        w!(x, " level=\"{level}\" selectionLevel=\"{level}\"");
        let scroll = t.scroll.or_else(|| wb.timeline_selection(t.id).map(|(start, _)| start)).or_else(|| wb.timeline_bounds(t.id).map(|(start, _)| start));
        if let Some(day) = scroll {
            w!(x, " scrollPosition=\"{}\"", iso_date(day));
        }
        if t.style != DEFAULT_STYLE && !t.style.is_empty() {
            push_text_attr(&mut x, "style", &t.style);
        }
        x.push_str("/>");
    }
    x.push_str("</timelines>");
    x
}

/// What the timelines of a workbook add to its package.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    let mut caches: Vec<&str> = Vec::new();
    for (_, timeline) in wb.timelines() {
        if caches.contains(&timeline.cache_name.as_str()) {
            continue;
        }
        let Some(xml) = cache_xml(wb, timeline.id, timeline) else {
            continue;
        };
        caches.push(&timeline.cache_name);
        let n = caches.len();
        extras.add_part(format!("xl/timelineCaches/timelineCache{n}.xml"), Some(CT_TIMELINE_CACHE), xml);
        extras.add_listed_cache(CacheList::Timelines, REL_TIMELINE_CACHE, &format!("timelineCaches/timelineCache{n}.xml"));
    }
    let mut parts = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if let Some(id) = SheetLinks::of(sheet).timelines {
            parts += 1;
            extras.add_part(format!("xl/timelines/timeline{parts}.xml"), Some(CT_TIMELINE), timelines_xml(wb, &sheet.timelines));
            extras.relate_sheet(index, id, REL_TIMELINE, &format!("../timelines/timeline{parts}.xml"));
        }
    }
}

/// What a timeline cache part says.
struct ReadCache {
    name: String,
    field: String,
    pivots: Vec<u32>,
    selection: Option<(f64, f64)>,
}

fn parse_cache(xml: &str, wb: &Workbook, sheets: &std::collections::HashMap<u32, usize>) -> Option<ReadCache> {
    let mut r = Reader::new(xml);
    let mut cache: Option<ReadCache> = None;
    let mut filtered = false;
    loop {
        match r.next() {
            Event::Open(t) => match (t.name, &mut cache) {
                ("timelineCacheDefinition", None) => {
                    let text = |name: &str| t.attr(name).map(|v| decode_cell_text(&v).into_owned());
                    cache = Some(ReadCache {
                        name: text("name")?,
                        field: text("sourceName")?,
                        pivots: Vec::new(),
                        selection: None,
                    });
                }
                ("pivotTables", Some(cache)) => cache.pivots = read_pivot_tables(&mut r, &t, wb, sheets),
                ("state", Some(_)) => filtered = t.attr("filterType").as_deref() == Some("dateBetween"),
                ("selection", Some(cache)) => {
                    let day = |name: &str| t.attr(name).and_then(|v| date_from_iso(&v)).map(f64::floor);
                    cache.selection = day("startDate").zip(day("endDate")).filter(|_| filtered);
                    r.skip(&t);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    cache
}

/// Read the timelines of every sheet into the workbook. A timeline whose
/// cache, PivotTable or frame is missing is left out.
pub(crate) fn read_all<R: Read + Seek>(
    pkg: &mut Package<R>,
    wb: &mut Workbook,
    workbook_xml: &str,
    workbook_rels: &[Rel],
    sheet_parts: &[String],
) {
    let cache_parts: Vec<&Rel> = workbook_rels.iter().filter(|rel| rel.kind == "timelineCache").collect();
    if cache_parts.is_empty() {
        return;
    }
    let sheets = sheets_by_id(wb, workbook_xml);
    let mut caches: Vec<ReadCache> = Vec::new();
    for rel in cache_parts {
        if let Ok(Some(xml)) = pkg.read(&rel.target) {
            caches.extend(parse_cache(&xml, wb, &sheets));
        }
    }
    let mut next_id = 1;
    for (index, part) in sheet_parts.iter().enumerate() {
        let frames = read_frames(pkg, part, &wb.sheets[index]);
        for rel in relationships(pkg, part).into_iter().filter(|rel| rel.kind == "timeline") {
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            let mut r = Reader::new(&xml);
            loop {
                let t = match r.next() {
                    Event::Open(t) if t.name == "timeline" => t,
                    Event::Eof => break,
                    _ => continue,
                };
                let text = |name: &str| t.attr(name).map(|v| decode_cell_text(&v).into_owned());
                let (Some(name), Some(cache_name)) = (text("name"), text("cache")) else {
                    continue;
                };
                let frame = frames.iter().find(|f| f.timeline && f.name == name);
                let cache = caches.iter().find(|c| c.name == cache_name);
                let (Some(frame), Some(cache)) = (frame, cache) else {
                    continue;
                };
                let taken = wb.timelines().any(|(_, known)| known.name.to_lowercase() == name.to_lowercase());
                // The field must be one of dates in a PivotTable that is there.
                let dated = cache.pivots.iter().filter_map(|id| wb.pivot(*id)).any(|(_, p)| {
                    p.cache.field_index(&cache.field).is_some_and(|f| p.cache.is_date(f))
                });
                if taken || !dated {
                    continue;
                }
                let shown = |name: &str| t.attr_bool(name).unwrap_or(true);
                wb.sheets[index].timelines.push(Timeline {
                    id: next_id,
                    caption: text("caption").unwrap_or_else(|| name.clone()),
                    name,
                    cache_name,
                    pivots: cache.pivots.clone(),
                    field: cache.field.clone(),
                    anchor: frame.anchor,
                    offset: frame.offset,
                    size: frame.size.unwrap_or(TIMELINE_SIZE),
                    show_header: shown("showHeader"),
                    show_selection_label: shown("showSelectionLabel"),
                    show_time_level: shown("showTimeLevel"),
                    show_scrollbar: shown("showHorizontalScrollbar"),
                    level: t
                        .attr_u32("level")
                        .and_then(|level| TimeLevel::ALL.get(level as usize).copied())
                        .unwrap_or(TimeLevel::Months),
                    style: text("style").unwrap_or_else(|| DEFAULT_STYLE.to_string()),
                    scroll: text("scrollPosition").and_then(|v| date_from_iso(&v)).map(f64::floor),
                });
                next_id += 1;
            }
        }
    }
    // A PivotTable whose part did not carry the date filter takes the
    // selection of the cache.
    for cache in &caches {
        if cache.selection.is_none() || !wb.timelines().any(|(_, t)| t.cache_name == cache.name) {
            continue;
        }
        let missing: Vec<u32> = cache
            .pivots
            .iter()
            .copied()
            .filter(|id| {
                wb.pivot(*id).is_some_and(|(_, p)| {
                    p.cache.field_index(&cache.field).is_some_and(|f| p.date_filter(f).is_none())
                })
            })
            .collect();
        let _ = wb.pivot_update_many(&missing, true, |p| {
            if let Some(f) = p.cache.field_index(&cache.field) {
                p.set_date_filter(f, cache.selection);
            }
        });
    }
}
