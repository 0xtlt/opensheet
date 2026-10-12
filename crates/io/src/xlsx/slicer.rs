//! Slicers (`Sheet::slicers`).
//!
//! A slicer is kept in four places, as Excel keeps it:
//!
//! * its *cache* (`xl/slicerCaches/slicerCacheN.xml`, related from the
//!   workbook and listed in the workbook's `<extLst>`): what it filters —
//!   a field of the PivotTables it is connected to, with the state of
//!   every item, or a column of a table —, how its items are sorted and
//!   how those without data are shown. The workbook also lists every
//!   cache as a defined name without a value;
//! * the *slicer* itself (`xl/slicers/slicerN.xml`, related from its
//!   worksheet and named in the worksheet's `<extLst>`): caption, columns,
//!   height of the buttons, style. A sheet has one such part for the
//!   slicers of PivotTables and another for the slicers of tables;
//! * a frame of the sheet's drawing, which says where it is: a choice
//!   between the slicer, by name, and a rectangle for the applications
//!   that know no slicers;
//! * the definition of the pivot cache, which gives the id the slicer
//!   cache names it by.
//!
//! The selection is the PivotTable's or the table's own filter; the item
//! states of a cache are read back only to filter the fields that are in
//! no area of the report, whose items the PivotTable part may not list.

use super::extlst::ExtList;
use super::filter::push_text_attr;
use super::parts::{CacheList, Extras, SheetLinks};
use super::read::{Package, Rel, relationships};
use super::write::NS_MAIN;
use super::xml::{Event, Reader, Tag, XML_DECL, decode_cell_text};
use opensheet_engine::Workbook;
use opensheet_engine::pivot::PivotCache;
use opensheet_engine::slicer::{BUTTON_HEIGHT, SLICER_SIZE, Slicer, SlicerSource};
use opensheet_engine::workbook::Sheet;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

pub(super) const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
pub(super) const NS_X15: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_A14: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
const NS_SLE: &str = "http://schemas.microsoft.com/office/drawing/2010/slicer";
const NS_TSLE: &str = "http://schemas.microsoft.com/office/drawing/2012/timeslicer";

const CT_SLICER: &str = "application/vnd.ms-excel.slicer+xml";
const CT_SLICER_CACHE: &str = "application/vnd.ms-excel.slicerCache+xml";
const REL_SLICER: &str = "http://schemas.microsoft.com/office/2007/relationships/slicer";
const REL_SLICER_CACHE: &str = "http://schemas.microsoft.com/office/2007/relationships/slicerCache";

/// The extensions of the worksheet naming its slicer parts and its
/// timeline part.
const EXT_SHEET_SLICERS: &str = "{A8765BA9-456A-4dab-B4F3-ACF838C121DE}";
const EXT_SHEET_TABLE_SLICERS: &str = "{3A4CF648-6AED-40f4-86FF-DC5316D8AED3}";
const EXT_SHEET_TIMELINES: &str = "{7E03D99C-DC04-49d9-9315-930204A7B6E9}";
/// The extensions of the workbook listing the caches.
const EXT_BOOK_SLICERS: &str = "{BBE1A952-AA13-448e-AADC-164F8A28A991}";
const EXT_BOOK_TABLE_SLICERS: &str = "{46BE6895-7355-4a93-B00E-2C351335B9C9}";
const EXT_BOOK_TIMELINES: &str = "{D0CA8CA8-9F24-4464-BF8E-62219DCF47F9}";
/// The extensions of a pivot cache definition: its id for slicers, and
/// the mark of a cache a timeline reads.
const EXT_CACHE_ID: &str = "{725AE2AE-9491-48be-B2B4-4EB974FC3084}";
const EXT_CACHE_TIMELINE: &str = "{A2CB5862-8E78-49c6-8D9D-AF26E26ADB89}";
/// The extensions of a slicer cache: the table column it filters, and
/// "Hide items with no data".
const EXT_TABLE_CACHE: &str = "{2F2917AC-EB37-4324-AD4E-5DD8C200BD13}";
const EXT_HIDE_NO_DATA: &str = "{470722E0-AACD-4C17-9CDC-17EF765DBC7E}";

const EMU_PER_PX: f64 = 9525.0;

// ---- what both slicers and timelines need ----------------------------------

/// The number the writer gives the cache of a PivotTable (`cacheId`, and
/// `n` of its parts): caches are numbered in the order their PivotTables
/// come in.
pub(super) fn pivot_cache_number(wb: &Workbook, cache: &Arc<PivotCache>) -> Option<usize> {
    let mut seen: Vec<&Arc<PivotCache>> = Vec::new();
    for (_, p) in wb.pivots() {
        if !seen.iter().any(|known| Arc::ptr_eq(known, &p.cache)) {
            seen.push(&p.cache);
        }
        if Arc::ptr_eq(&p.cache, cache) {
            return seen.iter().position(|known| Arc::ptr_eq(known, cache)).map(|at| at + 1);
        }
    }
    None
}

/// The `<pivotTables>` of a cache: the PivotTables it filters, by the id
/// of their sheet and their name.
pub(super) fn push_pivot_tables(x: &mut String, wb: &Workbook, pivots: &[u32]) {
    x.push_str("<pivotTables>");
    for (sheet, p) in pivots.iter().filter_map(|id| wb.pivot(*id)) {
        w!(x, "<pivotTable tabId=\"{}\"", sheet + 1);
        push_text_attr(x, "name", &p.name);
        x.push_str("/>");
    }
    x.push_str("</pivotTables>");
}

/// The cell a point lies in, `offset` pixels after the start of line
/// `index`, and what is left of the offset there.
fn locate(sheet: &Sheet, cols: bool, mut index: u32, mut offset: f64) -> (u32, f64) {
    let size = |i: u32| if cols { sheet.col_width(i) as f64 } else { sheet.row_height(i) as f64 };
    let last = if cols { opensheet_engine::MAX_COLS } else { opensheet_engine::MAX_ROWS } - 1;
    while index < last && offset >= size(index) && offset > 0.0 {
        offset -= size(index);
        index += 1;
    }
    (index, offset.max(0.0))
}

/// Pixels from the start of line `from` to a point of line `to`.
fn distance(sheet: &Sheet, cols: bool, from: u32, to: u32, offset: f64) -> f64 {
    let size = |i: u32| if cols { sheet.col_width(i) as f64 } else { sheet.row_height(i) as f64 };
    (from..to).map(size).sum::<f64>() + offset
}

/// One frame of a drawing holding a slicer or a timeline. `id` is the
/// object's own in the part.
fn push_frame(x: &mut String, sheet: &Sheet, id: usize, name: &str, timeline: bool, place: ((u32, u32), (f32, f32), (f32, f32))) {
    let (anchor, offset, size) = place;
    let left = locate(sheet, true, anchor.1, offset.0 as f64);
    let top = locate(sheet, false, anchor.0, offset.1 as f64);
    let right = locate(sheet, true, left.0, left.1 + size.0 as f64);
    let bottom = locate(sheet, false, top.0, top.1 + size.1 as f64);
    let emu = |px: f64| (px * EMU_PER_PX).round() as i64;
    let corner = |x: &mut String, name: &str, col: (u32, f64), row: (u32, f64)| {
        w!(
            x,
            "<xdr:{name}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{name}>",
            col.0,
            emu(col.1),
            row.0,
            emu(row.1)
        );
    };
    x.push_str("<xdr:twoCellAnchor editAs=\"oneCell\">");
    corner(x, "from", left, top);
    corner(x, "to", right, bottom);
    let (prefix, element, ns, what) = if timeline {
        ("tsle", "timeslicer", NS_TSLE, "timeline")
    } else {
        ("sle", "slicer", NS_SLE, "slicer")
    };
    // Applications that know the object take the first choice; the others
    // get a rectangle saying what it stands for.
    if timeline {
        w!(x, "<mc:AlternateContent xmlns:mc=\"{NS_MC}\" xmlns:tsle=\"{NS_TSLE}\"><mc:Choice Requires=\"tsle\">");
    } else {
        w!(x, "<mc:AlternateContent xmlns:mc=\"{NS_MC}\" xmlns:a14=\"{NS_A14}\"><mc:Choice Requires=\"a14\">");
    }
    w!(x, "<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{id}\"");
    push_text_attr(x, "name", name);
    x.push_str("/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>");
    x.push_str("<xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm>");
    w!(x, "<a:graphic><a:graphicData uri=\"{ns}\"><{prefix}:{element}");
    if !timeline {
        w!(x, " xmlns:sle=\"{NS_SLE}\"");
    }
    push_text_attr(x, "name", name);
    x.push_str("/></a:graphicData></a:graphic></xdr:graphicFrame></mc:Choice>");
    let (px, py) = (
        distance(sheet, true, 0, left.0, left.1),
        distance(sheet, false, 0, top.0, top.1),
    );
    w!(
        x,
        "<mc:Fallback xmlns=\"\"><xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"0\" name=\"\"/><xdr:cNvSpPr><a:spLocks noTextEdit=\"1\"/></xdr:cNvSpPr></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:prstClr val=\"white\"/></a:solidFill><a:ln w=\"1\"><a:solidFill><a:prstClr val=\"green\"/></a:solidFill></a:ln></xdr:spPr><xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\"/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"1100\"/><a:t>",
        emu(px),
        emu(py),
        emu(size.0 as f64),
        emu(size.1 as f64)
    );
    let version = if timeline { "Excel 2013" } else { "Excel 2010" };
    w!(
        x,
        "This shape represents a {what}. {}s are supported in {version} or later.\n\nIf the shape was modified in an earlier version of Excel, or if the workbook was saved in Excel 2003 or earlier, the {what} cannot be used.",
        if timeline { "Timeline" } else { "Slicer" }
    );
    x.push_str("</a:t></a:r></a:p></xdr:txBody></xdr:sp></mc:Fallback></mc:AlternateContent><xdr:clientData/></xdr:twoCellAnchor>");
}

/// The frames of the slicers and timelines of a sheet, for its drawing
/// part; the objects get the ids from `first_id` on.
pub(crate) fn anchors(sheet: &Sheet, first_id: usize) -> String {
    let mut x = String::new();
    let mut id = first_id;
    for s in &sheet.slicers {
        push_frame(&mut x, sheet, id, &s.name, false, (s.anchor, s.offset, s.size));
        id += 1;
    }
    for t in &sheet.timelines {
        push_frame(&mut x, sheet, id, &t.name, true, (t.anchor, t.offset, t.size));
        id += 1;
    }
    x
}

/// Where a drawing puts a slicer or a timeline.
pub(super) struct Frame {
    pub name: String,
    pub timeline: bool,
    pub anchor: (u32, u32),
    pub offset: (f32, f32),
    pub size: Option<(f32, f32)>,
}

/// The frames of slicers and timelines in the drawing of a worksheet.
pub(super) fn read_frames<R: Read + Seek>(pkg: &mut Package<R>, sheet_part: &str, sheet: &Sheet) -> Vec<Frame> {
    let mut out = Vec::new();
    for rel in relationships(pkg, sheet_part).into_iter().filter(|rel| rel.kind == "drawing") {
        let Ok(Some(xml)) = pkg.read(&rel.target) else {
            continue;
        };
        let mut r = Reader::new(&xml);
        loop {
            match r.next() {
                Event::Open(t) if matches!(t.name, "twoCellAnchor" | "oneCellAnchor") && !t.empty => {
                    out.extend(read_frame(&mut r, sheet));
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    out
}

/// One anchor, read to its end; `None` when it holds something else than
/// a slicer or a timeline.
fn read_frame(r: &mut Reader, sheet: &Sheet) -> Option<Frame> {
    // (col, colOff, row, rowOff) of the two corners, offsets in pixels.
    let mut corners: [Option<(u32, f64, u32, f64)>; 2] = [None, None];
    let mut extent: Option<(f64, f64)> = None;
    let mut named: Option<(String, bool)> = None;
    let mut depth = 1usize;
    while depth > 0 {
        match r.next() {
            Event::Open(t) => match t.name {
                "from" | "to" if depth == 1 && !t.empty => {
                    let mut corner = (0u32, 0.0f64, 0u32, 0.0f64);
                    loop {
                        match r.next() {
                            Event::Open(c) => {
                                let name = c.name;
                                let number = r.text(&c).trim().parse::<f64>().unwrap_or(0.0).max(0.0);
                                match name {
                                    "col" => corner.0 = number as u32,
                                    "colOff" => corner.1 = number / EMU_PER_PX,
                                    "row" => corner.2 = number as u32,
                                    "rowOff" => corner.3 = number / EMU_PER_PX,
                                    _ => {}
                                }
                            }
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    }
                    corners[usize::from(t.name == "to")] = Some(corner);
                }
                "ext" if depth == 1 => {
                    let length = |name: &str| t.attr_f64(name).unwrap_or(0.0) / EMU_PER_PX;
                    extent = Some((length("cx"), length("cy")));
                    depth += usize::from(!t.empty);
                }
                "slicer" | "timeslicer" => {
                    if let Some(name) = t.attr("name") {
                        named.get_or_insert((decode_cell_text(&name).into_owned(), t.name == "timeslicer"));
                    }
                    depth += usize::from(!t.empty);
                }
                _ => depth += usize::from(!t.empty),
            },
            Event::Close(_) => depth -= 1,
            Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    let (name, timeline) = named?;
    let from = corners[0]?;
    let size = match (corners[1], extent) {
        (Some(to), _) => Some((
            distance(sheet, true, from.0, to.0, to.1) - from.1,
            distance(sheet, false, from.2, to.2, to.3) - from.3,
        )),
        (None, extent) => extent,
    }
    .filter(|(w, h)| *w >= 1.0 && *h >= 1.0)
    .map(|(w, h)| (w as f32, h as f32));
    Some(Frame {
        name,
        timeline,
        anchor: (from.2, from.0),
        offset: (from.1 as f32, from.3 as f32),
        size,
    })
}

/// The sheets of the file by the id caches name them with (`tabId`), as
/// indices into `wb.sheets`.
pub(super) fn sheets_by_id(wb: &Workbook, workbook_xml: &str) -> HashMap<u32, usize> {
    let mut out = HashMap::new();
    let mut r = Reader::new(workbook_xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "sheet" => {
                let name = t.attr("name").map(|name| decode_cell_text(&name).into_owned());
                if let (Some(id), Some(index)) = (t.attr_u32("sheetId"), name.and_then(|name| wb.sheet_index(&name))) {
                    out.insert(id, index);
                }
                r.skip(&t);
            }
            Event::Open(t) if matches!(t.name, "definedNames" | "extLst") => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// A `<pivotTables>` list as ids of PivotTables of `wb`.
pub(super) fn read_pivot_tables(r: &mut Reader, tag: &Tag, wb: &Workbook, sheets: &HashMap<u32, usize>) -> Vec<u32> {
    let mut out = Vec::new();
    if tag.empty {
        return out;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let name = t.attr("name").map(|name| decode_cell_text(&name).to_lowercase());
                let sheet = t.attr_u32("tabId").and_then(|id| sheets.get(&id));
                if let (Some(name), Some(sheet)) = (name, sheet) {
                    out.extend(wb.sheets[*sheet].pivots.iter().find(|p| p.name.to_lowercase() == name).map(|p| p.id));
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    out
}

// ---- writing ---------------------------------------------------------------

fn cache_names(wb: &Workbook) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let all = wb.slicers().map(|(_, s)| s.cache_name.as_str()).chain(wb.timelines().map(|(_, t)| t.cache_name.as_str()));
    for name in all {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

pub(crate) fn has_names(wb: &Workbook) -> bool {
    !cache_names(wb).is_empty()
}

/// The names of the caches, as Excel lists them among the defined names.
pub(crate) fn push_names(x: &mut String, wb: &Workbook) {
    for name in cache_names(wb) {
        x.push_str("<definedName");
        push_text_attr(x, "name", name);
        x.push_str(">#N/A</definedName>");
    }
}

/// True for a defined name that only stands for a cache.
pub(crate) fn is_cache_name(wb: &Workbook, name: &str) -> bool {
    cache_names(wb).iter().any(|known| known.eq_ignore_ascii_case(name))
}

/// The extensions of a worksheet naming its slicers and timelines.
pub(crate) fn write_ext(list: &mut ExtList, sheet: &Sheet) {
    let links = SheetLinks::of(sheet);
    if let Some(id) = links.slicers {
        list.add(format!(
            "<ext uri=\"{EXT_SHEET_SLICERS}\" xmlns:x14=\"{NS_X14}\"><x14:slicerList><x14:slicer r:id=\"rId{id}\"/></x14:slicerList></ext>"
        ));
    }
    if let Some(id) = links.timelines {
        list.add(format!(
            "<ext uri=\"{EXT_SHEET_TIMELINES}\" xmlns:x15=\"{NS_X15}\"><x15:timelineRefs><x15:timelineRef r:id=\"rId{id}\"/></x15:timelineRefs></ext>"
        ));
    }
    if let Some(id) = links.table_slicers {
        list.add(format!(
            "<ext uri=\"{EXT_SHEET_TABLE_SLICERS}\" xmlns:x15=\"{NS_X15}\"><x14:slicerList xmlns:x14=\"{NS_X14}\"><x14:slicer r:id=\"rId{id}\"/></x14:slicerList></ext>"
        ));
    }
}

/// One of the lists of caches of the workbook part's `<extLst>`; `ids`
/// are the relationships to the caches.
pub(crate) fn push_workbook_ext(x: &mut String, list: CacheList, ids: &[usize]) {
    match list {
        CacheList::PivotSlicers => {
            w!(x, "<ext uri=\"{EXT_BOOK_SLICERS}\" xmlns:x14=\"{NS_X14}\"><x14:slicerCaches>");
            for id in ids {
                w!(x, "<x14:slicerCache r:id=\"rId{id}\"/>");
            }
            x.push_str("</x14:slicerCaches></ext>");
        }
        CacheList::TableSlicers => {
            w!(
                x,
                "<ext uri=\"{EXT_BOOK_TABLE_SLICERS}\" xmlns:x15=\"{NS_X15}\"><x15:slicerCaches xmlns:x14=\"{NS_X14}\">"
            );
            for id in ids {
                w!(x, "<x14:slicerCache r:id=\"rId{id}\"/>");
            }
            x.push_str("</x15:slicerCaches></ext>");
        }
        CacheList::Timelines => {
            w!(x, "<ext uri=\"{EXT_BOOK_TIMELINES}\" xmlns:x15=\"{NS_X15}\"><x15:timelineCacheRefs>");
            for id in ids {
                w!(x, "<x15:timelineCacheRef r:id=\"rId{id}\"/>");
            }
            x.push_str("</x15:timelineCacheRefs></ext>");
        }
    }
}

/// The `<extLst>` of the definition of pivot cache number `n`: the id
/// slicers and timelines name it by, when some do.
pub(crate) fn cache_extensions(wb: &Workbook, cache: &Arc<PivotCache>, n: usize) -> String {
    let on_cache = |pivots: &[u32]| pivots.iter().filter_map(|id| wb.pivot(*id)).any(|(_, p)| Arc::ptr_eq(&p.cache, cache));
    let sliced = wb.slicers().any(|(_, s)| on_cache(s.pivots()));
    let timed = wb.timelines().any(|(_, t)| on_cache(&t.pivots));
    if !sliced && !timed {
        return String::new();
    }
    let mut x = format!(
        "<extLst><ext uri=\"{EXT_CACHE_ID}\" xmlns:x14=\"{NS_X14}\"><x14:pivotCacheDefinition pivotCacheId=\"{n}\"/></ext>"
    );
    if timed {
        w!(
            x,
            "<ext uri=\"{EXT_CACHE_TIMELINE}\" xmlns:x15=\"{NS_X15}\"><x15:timelinePivotCacheDefinition timelineData=\"1\"/></ext>"
        );
    }
    x.push_str("</extLst>");
    x
}

/// `crossFilter` of a cache.
fn cross_filter(slicer: &Slicer) -> Option<&'static str> {
    match (slicer.indicate_no_data, slicer.no_data_last) {
        (true, true) => None,
        (true, false) => Some("showItemsWithNoData"),
        (false, _) => Some("none"),
    }
}

fn push_sorting(x: &mut String, slicer: &Slicer) {
    if slicer.descending {
        x.push_str(" sortOrder=\"descending\"");
    }
    if !slicer.custom_lists {
        x.push_str(" customListSort=\"0\"");
    }
    if let Some(mode) = cross_filter(slicer) {
        w!(x, " crossFilter=\"{mode}\"");
    }
}

/// The part of a slicer cache.
fn cache_xml(wb: &Workbook, slicer: &Slicer) -> Option<String> {
    let mut x = String::from(XML_DECL);
    w!(
        x,
        "<slicerCacheDefinition xmlns=\"{NS_X14}\" xmlns:mc=\"{NS_MC}\" mc:Ignorable=\"x\" xmlns:x=\"{NS_MAIN}\""
    );
    push_text_attr(&mut x, "name", &slicer.cache_name);
    let hide = if slicer.hide_no_data {
        format!("<x:ext uri=\"{EXT_HIDE_NO_DATA}\" xmlns:x15=\"{NS_X15}\"><x15:slicerCacheHideItemsWithNoData/></x:ext>")
    } else {
        String::new()
    };
    match &slicer.source {
        SlicerSource::Pivot { pivots, field } => {
            push_text_attr(&mut x, "sourceName", field);
            x.push('>');
            let (_, p) = pivots.iter().find_map(|id| wb.pivot(*id))?;
            let cache = pivot_cache_number(wb, &p.cache)?;
            push_pivot_tables(&mut x, wb, pivots);
            w!(x, "<data><tabular pivotCacheId=\"{cache}\"");
            push_sorting(&mut x, slicer);
            if !slicer.show_deleted {
                x.push_str(" showMissing=\"0\"");
            }
            // Every item, hidden or not, in the order of the slicer.
            let all = Slicer {
                hide_no_data: false,
                ..slicer.clone()
            };
            let items = wb.slicer_items_of(&all);
            w!(x, "><items count=\"{}\">", items.len());
            for item in items {
                w!(x, "<i x=\"{}\"", item.x);
                if item.selected {
                    x.push_str(" s=\"1\"");
                }
                if !item.has_data {
                    x.push_str(" nd=\"1\"");
                }
                x.push_str("/>");
            }
            x.push_str("</items></tabular></data>");
            if !hide.is_empty() {
                w!(x, "<extLst>{hide}</extLst>");
            }
        }
        SlicerSource::Table { table, column } => {
            push_text_attr(&mut x, "sourceName", column);
            x.push('>');
            let (_, t) = wb.table_by_id(*table)?;
            // Table columns are written with the ids 1, 2, 3...
            let column = t.column_index(column)? + 1;
            w!(
                x,
                "<extLst><x:ext uri=\"{EXT_TABLE_CACHE}\" xmlns:x15=\"{NS_X15}\"><x15:tableSlicerCache tableId=\"{table}\" column=\"{column}\""
            );
            push_sorting(&mut x, slicer);
            w!(x, "/></x:ext>{hide}</extLst>");
        }
    }
    x.push_str("</slicerCacheDefinition>");
    Some(x)
}

/// A part listing slicers of one sheet.
fn slicers_xml(slicers: &[&Slicer]) -> String {
    let mut x = String::from(XML_DECL);
    w!(x, "<slicers xmlns=\"{NS_X14}\" xmlns:mc=\"{NS_MC}\" mc:Ignorable=\"x\" xmlns:x=\"{NS_MAIN}\">");
    for s in slicers {
        x.push_str("<slicer");
        push_text_attr(&mut x, "name", &s.name);
        push_text_attr(&mut x, "cache", &s.cache_name);
        push_text_attr(&mut x, "caption", &s.caption);
        if s.start_item > 0 {
            w!(x, " startItem=\"{}\"", s.start_item);
        }
        if s.columns > 1 {
            w!(x, " columnCount=\"{}\"", s.columns);
        }
        if !s.show_header {
            x.push_str(" showCaption=\"0\"");
        }
        if s.style != "SlicerStyleLight1" && !s.style.is_empty() {
            push_text_attr(&mut x, "style", &s.style);
        }
        w!(x, " rowHeight=\"{}\"/>", (s.button_height.max(1.0) as f64 * EMU_PER_PX).round() as i64);
    }
    x.push_str("</slicers>");
    x
}

/// What the slicers of a workbook add to its package.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    // The caches, each once: copies of a slicer share theirs.
    let mut caches: Vec<&str> = Vec::new();
    for (_, slicer) in wb.slicers() {
        if caches.contains(&slicer.cache_name.as_str()) {
            continue;
        }
        let Some(xml) = cache_xml(wb, slicer) else {
            continue;
        };
        caches.push(&slicer.cache_name);
        let n = caches.len();
        extras.add_part(format!("xl/slicerCaches/slicerCache{n}.xml"), Some(CT_SLICER_CACHE), xml);
        let list = match slicer.source {
            SlicerSource::Pivot { .. } => CacheList::PivotSlicers,
            SlicerSource::Table { .. } => CacheList::TableSlicers,
        };
        extras.add_listed_cache(list, REL_SLICER_CACHE, &format!("slicerCaches/slicerCache{n}.xml"));
    }
    let mut parts = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        for (id, of_table) in [(links.slicers, false), (links.table_slicers, true)] {
            let Some(id) = id else {
                continue;
            };
            let listed: Vec<&Slicer> = sheet
                .slicers
                .iter()
                .filter(|s| matches!(s.source, SlicerSource::Table { .. }) == of_table)
                .collect();
            parts += 1;
            extras.add_part(format!("xl/slicers/slicer{parts}.xml"), Some(CT_SLICER), slicers_xml(&listed));
            extras.relate_sheet(index, id, REL_SLICER, &format!("../slicers/slicer{parts}.xml"));
        }
    }
}

// ---- reading ---------------------------------------------------------------

/// What a slicer cache part says.
struct ReadCache {
    name: String,
    source: SlicerSource,
    descending: bool,
    custom_lists: bool,
    cross_filter: String,
    show_deleted: bool,
    hide_no_data: bool,
    /// (`x`, selected) of the items of a PivotTable field.
    items: Vec<(u32, bool)>,
}

fn parse_cache(xml: &str, wb: &Workbook, sheets: &HashMap<u32, usize>, tables: &HashMap<(u32, u32), (u32, String)>) -> Option<ReadCache> {
    let mut r = Reader::new(xml);
    let root = loop {
        match r.next() {
            Event::Open(t) if t.name == "slicerCacheDefinition" => break t,
            Event::Open(t) => r.skip(&t),
            Event::Eof => return None,
            _ => {}
        }
    };
    let text = |tag: &Tag, name: &str| tag.attr(name).map(|v| decode_cell_text(&v).into_owned());
    let mut cache = ReadCache {
        name: text(&root, "name")?,
        source: SlicerSource::Pivot {
            pivots: Vec::new(),
            field: text(&root, "sourceName").unwrap_or_default(),
        },
        descending: false,
        custom_lists: true,
        cross_filter: String::new(),
        show_deleted: true,
        hide_no_data: false,
        items: Vec::new(),
    };
    let mut pivots = Vec::new();
    let mut table: Option<(u32, String)> = None;
    let sorting = |cache: &mut ReadCache, t: &Tag| {
        cache.descending = t.attr("sortOrder").as_deref() == Some("descending");
        cache.custom_lists = t.attr_bool("customListSort").unwrap_or(true);
        cache.cross_filter = t.attr("crossFilter").map(|v| v.into_owned()).unwrap_or_default();
    };
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "pivotTables" => pivots = read_pivot_tables(&mut r, &t, wb, sheets),
                "tabular" => {
                    sorting(&mut cache, &t);
                    cache.show_deleted = t.attr_bool("showMissing").unwrap_or(true);
                }
                "i" => {
                    if let Some(x) = t.attr_u32("x") {
                        cache.items.push((x, t.attr_bool("s").unwrap_or(false)));
                    }
                    r.skip(&t);
                }
                "tableSlicerCache" => {
                    sorting(&mut cache, &t);
                    let key = (t.attr_u32("tableId").unwrap_or(0), t.attr_u32("column").unwrap_or(0));
                    table = tables.get(&key).cloned();
                    // A column the table no longer has: no slicer.
                    table.as_ref()?;
                    r.skip(&t);
                }
                "slicerCacheHideItemsWithNoData" => {
                    cache.hide_no_data = true;
                    r.skip(&t);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    match (table, &mut cache.source) {
        (Some((table, column)), source) => *source = SlicerSource::Table { table, column },
        (None, SlicerSource::Pivot { pivots: list, .. }) => *list = pivots,
        _ => {}
    }
    Some(cache)
}

/// The table columns of the file by the ids caches name them with:
/// (`tableId`, id of the column) → (id of the table in `wb`, name of the
/// column).
fn table_columns<R: Read + Seek>(pkg: &mut Package<R>, wb: &Workbook, sheet_parts: &[String]) -> HashMap<(u32, u32), (u32, String)> {
    let mut out = HashMap::new();
    for part in sheet_parts {
        for rel in relationships(pkg, part).into_iter().filter(|rel| rel.kind == "table") {
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            let mut r = Reader::new(&xml);
            let mut table: Option<(u32, u32)> = None;
            loop {
                match r.next() {
                    Event::Open(t) if t.name == "table" => {
                        let name = t.attr("displayName").or_else(|| t.attr("name"));
                        let known = name.and_then(|name| wb.table_by_name(&decode_cell_text(&name)).map(|(_, table)| table.id));
                        table = t.attr_u32("id").zip(known);
                    }
                    Event::Open(t) if t.name == "tableColumn" => {
                        if let (Some((file_id, id)), Some(column), Some(name)) = (table, t.attr_u32("id"), t.attr("name")) {
                            out.insert((file_id, column), (id, decode_cell_text(&name).into_owned()));
                        }
                        r.skip(&t);
                    }
                    Event::Open(t) if matches!(t.name, "autoFilter" | "sortState" | "tableStyleInfo") => r.skip(&t),
                    Event::Eof => break,
                    _ => {}
                }
            }
        }
    }
    out
}

/// Read the slicers of every sheet into the workbook. A slicer whose
/// cache, PivotTable, table or frame is missing is left out.
pub(crate) fn read_all<R: Read + Seek>(
    pkg: &mut Package<R>,
    wb: &mut Workbook,
    workbook_xml: &str,
    workbook_rels: &[Rel],
    sheet_parts: &[String],
) {
    let cache_parts: Vec<&Rel> = workbook_rels.iter().filter(|rel| rel.kind == "slicerCache").collect();
    if cache_parts.is_empty() {
        return;
    }
    let sheets = sheets_by_id(wb, workbook_xml);
    let tables = table_columns(pkg, wb, sheet_parts);
    let mut caches: Vec<ReadCache> = Vec::new();
    for rel in cache_parts {
        if let Ok(Some(xml)) = pkg.read(&rel.target) {
            caches.extend(parse_cache(&xml, wb, &sheets, &tables));
        }
    }
    let mut next_id = 1;
    for (index, part) in sheet_parts.iter().enumerate() {
        let frames = read_frames(pkg, part, &wb.sheets[index]);
        for rel in relationships(pkg, part).into_iter().filter(|rel| rel.kind == "slicer") {
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            let mut r = Reader::new(&xml);
            loop {
                let t = match r.next() {
                    Event::Open(t) if t.name == "slicer" => t,
                    Event::Eof => break,
                    _ => continue,
                };
                let text = |name: &str| t.attr(name).map(|v| decode_cell_text(&v).into_owned());
                let (Some(name), Some(cache_name)) = (text("name"), text("cache")) else {
                    continue;
                };
                let frame = frames.iter().find(|f| !f.timeline && f.name == name);
                let cache = caches.iter().find(|c| c.name == cache_name);
                let (Some(frame), Some(cache)) = (frame, cache) else {
                    continue;
                };
                let taken = wb.slicers().any(|(_, s)| s.name.to_lowercase() == name.to_lowercase());
                if taken || matches!(&cache.source, SlicerSource::Pivot { pivots, .. } if pivots.is_empty()) {
                    continue;
                }
                wb.sheets[index].slicers.push(Slicer {
                    id: next_id,
                    caption: text("caption").unwrap_or_else(|| name.clone()),
                    name,
                    cache_name,
                    source: cache.source.clone(),
                    anchor: frame.anchor,
                    offset: frame.offset,
                    size: frame.size.unwrap_or(SLICER_SIZE),
                    show_header: t.attr_bool("showCaption").unwrap_or(true),
                    columns: t.attr_u32("columnCount").unwrap_or(1).max(1),
                    button_height: t
                        .attr_f64("rowHeight")
                        .map(|emu| (emu / EMU_PER_PX) as f32)
                        .filter(|h| *h >= 1.0)
                        .unwrap_or(BUTTON_HEIGHT),
                    style: text("style").unwrap_or_else(|| "SlicerStyleLight1".to_string()),
                    descending: cache.descending,
                    custom_lists: cache.custom_lists,
                    hide_no_data: cache.hide_no_data,
                    indicate_no_data: cache.cross_filter != "none",
                    no_data_last: cache.cross_filter.is_empty() || cache.cross_filter == "showItemsWithDataAtTop",
                    show_deleted: cache.show_deleted,
                    start_item: t.attr_u32("startItem").unwrap_or(0),
                });
                next_id += 1;
            }
        }
    }
    // The fields the slicers filter are filtered wherever they are, and
    // the items a PivotTable part did not list take their state from the
    // cache.
    for cache in &caches {
        let SlicerSource::Pivot { pivots, field } = &cache.source else {
            continue;
        };
        if !wb.slicers().any(|(_, s)| s.cache_name == cache.name) {
            continue;
        }
        let selected: Vec<u32> = cache.items.iter().filter(|(_, s)| *s).map(|(x, _)| *x).collect();
        let partial = !selected.is_empty() && selected.len() < cache.items.len();
        let _ = wb.pivot_update_many(pivots, true, |p| {
            let Some(f) = p.cache.field_index(field) else {
                return;
            };
            let listed = p.axis_of(f).is_some() || p.fields[f].items.iter().any(|it| it.hidden);
            if partial && !listed {
                p.set_visible_items(f, &selected);
            }
            if !p.sliced.contains(&f) {
                p.sliced.push(f);
            }
        });
    }
}
