//! Custom views (`custom_view.rs` of the engine) in a package.
//!
//! As Excel writes them, a view is in three places:
//!
//! * `<customWorkbookViews>` of the workbook part names it and says what
//!   it includes;
//! * each worksheet holds its part in `<customSheetViews>`: the window
//!   (zoom, panes, selection, view mode, gridlines and headings), the
//!   print settings and the filter;
//! * hidden names of the workbook hold what has no element: the hidden
//!   rows and columns (`Z_<guid>_.wvu.Rows`, `.wvu.Cols`), the range of
//!   the filter (`.wvu.FilterData`), the print area and the print titles
//!   (`.wvu.PrintArea`, `.wvu.PrintTitles`).
//!
//! The size and the place of the window, which the engine does not keep,
//! are written as those of the workbook's own window.

use super::dxf::Dxfs;
use super::filter::DxfColors;
use super::page;
use super::xml::{Event, Reader, Tag, push_attr, push_text};
use opensheet_engine::custom_view::{CustomSheetView, HiddenState, ViewKind, WindowState, is_guid};
use opensheet_engine::filter::FilterScope;
use opensheet_engine::page::PageSetup;
use opensheet_engine::refs::{cell_name, parse_cell_name};
use opensheet_engine::{MAX_COLS, MAX_ROWS, Rect, Sheet, Workbook};
use std::collections::BTreeSet;
use std::fmt::Write as _;

// ---- writing -------------------------------------------------------------------

/// `<customWorkbookViews>`, which follows `<calcPr>` in the workbook part.
pub(crate) fn write_workbook(x: &mut String, wb: &Workbook) {
    let views = wb.custom_views();
    if views.is_empty() {
        return;
    }
    x.push_str("<customWorkbookViews>");
    for view in views {
        x.push_str("<customWorkbookView name=\"");
        push_attr(x, &view.name);
        w!(x, "\" guid=\"{}\"", view.guid);
        if !view.print_settings {
            x.push_str(" includePrintSettings=\"0\"");
        }
        if !view.hidden_settings {
            x.push_str(" includeHiddenRowCol=\"0\"");
        }
        // The sheet in front: `sheetId` of the first sheet that says so.
        let front = wb
            .sheets
            .iter()
            .position(|sheet| sheet.custom_view(&view.guid).is_some_and(|part| part.window.selected))
            .unwrap_or(0);
        w!(
            x,
            " xWindow=\"0\" yWindow=\"0\" windowWidth=\"28800\" windowHeight=\"17600\" activeSheetId=\"{}\"/>",
            front + 1
        );
    }
    x.push_str("</customWorkbookViews>");
}

/// `Z_6F2E…_.wvu.`: what the hidden names of a view begin with.
fn name_prefix(guid: &str) -> String {
    format!("Z_{}_.wvu.", guid.trim_matches(['{', '}']).replace('-', "_"))
}

/// Runs of indices as whole rows (or columns) of a sheet.
fn runs(indices: &BTreeSet<u32>, rows: bool) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for index in indices {
        match out.last_mut() {
            Some(last) if rows && last.r1 + 1 == *index => last.r1 = *index,
            Some(last) if !rows && last.c1 + 1 == *index => last.c1 = *index,
            _ if rows => out.push(Rect::new(*index, 0, *index, MAX_COLS - 1)),
            _ => out.push(Rect::new(0, *index, MAX_ROWS - 1, *index)),
        }
    }
    out
}

/// A list of ranges of a sheet as the text of a defined name.
fn ranges_formula(sheet_name: &str, ranges: Vec<Rect>) -> Option<String> {
    // The print area of a page setup is written the same way.
    let scratch = PageSetup {
        print_area: ranges,
        ..PageSetup::default()
    };
    scratch.print_area_formula(sheet_name)
}

/// The hidden names of one part of a view: (suffix, text).
fn names_of(sheet: &Sheet, part: &CustomSheetView) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(hidden) = &part.hidden {
        let lists = [("Rows", runs(&hidden.rows, true)), ("Cols", runs(&hidden.cols, false))];
        for (suffix, ranges) in lists {
            out.extend(ranges_formula(&sheet.name, ranges).map(|text| (suffix, text)));
        }
        let filter = hidden.filter.as_ref().map(|filter| vec![filter.range]).unwrap_or_default();
        out.extend(ranges_formula(&sheet.name, filter).map(|text| ("FilterData", text)));
    }
    if let Some(setup) = &part.page {
        out.extend(setup.print_area_formula(&sheet.name).map(|text| ("PrintArea", text)));
        out.extend(setup.print_titles_formula(&sheet.name).map(|text| ("PrintTitles", text)));
    }
    out
}

/// Whether the views need names (see [`push_names`]).
pub(crate) fn has_names(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sheet| sheet.custom_views.iter().any(|part| !names_of(sheet, part).is_empty()))
}

/// Append the hidden names of the views (children of `<definedNames>`).
pub(crate) fn push_names(x: &mut String, wb: &Workbook) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        for part in &sheet.custom_views {
            let prefix = name_prefix(&part.guid);
            for (suffix, text) in names_of(sheet, part) {
                w!(x, "<definedName name=\"{prefix}{suffix}\" localSheetId=\"{index}\" hidden=\"1\">");
                push_text(x, &text);
                x.push_str("</definedName>");
            }
        }
    }
}

/// The elements `page::write` appends, in the order a custom sheet view
/// wants them: the breaks first.
fn push_page(x: &mut String, setup: &PageSetup) {
    let mut written = String::new();
    page::write(&mut written, setup);
    // Where each element begins; each one runs to the next.
    let wanted = ["<rowBreaks", "<colBreaks", "<pageMargins", "<printOptions", "<pageSetup", "<headerFooter"];
    let mut starts: Vec<usize> = wanted.iter().filter_map(|tag| written.find(tag)).collect();
    starts.sort_unstable();
    for tag in wanted {
        if let Some(start) = written.find(tag) {
            let end = starts.iter().copied().find(|other| *other > start).unwrap_or(written.len());
            x.push_str(&written[start..end]);
        }
    }
}

fn push_sqref(x: &mut String, ranges: &[Rect]) {
    for (i, range) in ranges.iter().enumerate() {
        if i > 0 {
            x.push(' ');
        }
        x.push_str(&cell_name(range.r0, range.c0));
        if !range.is_single() {
            x.push(':');
            x.push_str(&cell_name(range.r1, range.c1));
        }
    }
}

/// `<customSheetViews>` of a worksheet, which follows `<dataConsolidate>`.
/// The colours the criteria of a view's filter go by are added to `dxfs`.
pub(crate) fn write_sheet(x: &mut String, wb: &Workbook, index: usize, dxfs: &mut Dxfs) {
    let sheet = &wb.sheets[index];
    if sheet.custom_views.is_empty() {
        return;
    }
    x.push_str("<customSheetViews>");
    for part in &sheet.custom_views {
        let window = &part.window;
        w!(x, "<customSheetView guid=\"{}\"", part.guid);
        if window.zoom != 100 {
            w!(x, " scale=\"{}\"", window.zoom.clamp(10, 400));
        }
        if !window.show_gridlines {
            x.push_str(" showGridLines=\"0\"");
        }
        if !window.show_headers {
            x.push_str(" showRowCol=\"0\"");
        }
        let filter = part.hidden.as_ref().and_then(|hidden| hidden.filter.as_ref());
        if let Some(hidden) = &part.hidden {
            if filter.is_some_and(|filter| filter.is_filtered()) {
                x.push_str(" filter=\"1\"");
            }
            if filter.is_some() {
                x.push_str(" showAutoFilter=\"1\"");
            }
            if !hidden.rows.is_empty() {
                x.push_str(" hiddenRows=\"1\"");
            }
            if !hidden.cols.is_empty() {
                x.push_str(" hiddenColumns=\"1\"");
            }
        }
        if window.mode != ViewKind::Normal {
            w!(x, " view=\"{}\"", window.mode.ooxml_name());
        }
        let (rows, cols) = (window.frozen_rows.min(MAX_ROWS - 1), window.frozen_cols.min(MAX_COLS - 1));
        let top_left = cell_name(window.top_left.0.max(rows), window.top_left.1.max(cols));
        let frozen = rows > 0 || cols > 0;
        if !frozen && window.top_left != (0, 0) {
            w!(x, " topLeftCell=\"{top_left}\"");
        }
        x.push('>');
        let pane = match (rows > 0, cols > 0) {
            (true, true) => "bottomRight",
            (true, false) => "bottomLeft",
            (false, true) => "topRight",
            (false, false) => "",
        };
        if frozen {
            x.push_str("<pane");
            if cols > 0 {
                w!(x, " xSplit=\"{cols}\"");
            }
            if rows > 0 {
                w!(x, " ySplit=\"{rows}\"");
            }
            w!(x, " topLeftCell=\"{top_left}\" activePane=\"{pane}\" state=\"frozen\"/>");
        }
        x.push_str("<selection");
        if frozen {
            w!(x, " pane=\"{pane}\"");
        }
        w!(x, " activeCell=\"{}\" sqref=\"", cell_name(window.active.0, window.active.1));
        if window.ranges.is_empty() {
            push_sqref(x, &[Rect::cell(window.active.0, window.active.1)]);
        } else {
            push_sqref(x, &window.ranges);
        }
        x.push_str("\"/>");
        if let Some(setup) = &part.page {
            push_page(x, setup);
        }
        if let Some(filter) = filter {
            // The filter is written as a sheet's own is: from a workbook
            // that holds nothing else.
            let mut scratch = Workbook::new(wb.locale);
            scratch.sheets[0].auto_filter = Some(filter.clone());
            super::filter::push_auto_filter(x, &scratch, 0, FilterScope::Sheet, dxfs);
        }
        x.push_str("</customSheetView>");
    }
    x.push_str("</customSheetViews>");
}

// ---- reading -------------------------------------------------------------------

/// A `<customWorkbookView>`.
#[derive(Clone, Debug)]
pub(crate) struct BookView {
    name: String,
    guid: String,
    print_settings: bool,
    hidden_settings: bool,
    /// `sheetId` of the sheet in front.
    active_sheet_id: u32,
}

/// Read a `<customWorkbookView>` (an empty element).
pub(crate) fn read_book_view(tag: &Tag) -> Option<BookView> {
    let guid = tag.attr("guid")?.trim().to_uppercase();
    let name: String = tag.attr("name")?.chars().filter(|c| !c.is_control()).collect();
    if !is_guid(&guid) || name.trim().is_empty() {
        return None;
    }
    Some(BookView {
        name: name.trim().to_string(),
        guid,
        print_settings: tag.attr_bool("includePrintSettings").unwrap_or(true),
        hidden_settings: tag.attr_bool("includeHiddenRowCol").unwrap_or(true),
        active_sheet_id: tag.attr_u32("activeSheetId").unwrap_or(0),
    })
}

fn cell(text: &str) -> Option<(u32, u32)> {
    parse_cell_name(text.trim())
}

fn read_view<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, colors: &[DxfColors]) -> Option<CustomSheetView> {
    let guid = tag.attr("guid").map(|guid| guid.trim().to_uppercase()).filter(|guid| is_guid(guid));
    let Some(guid) = guid else {
        r.skip(tag);
        return None;
    };
    let mut window = WindowState {
        zoom: tag.attr_u32("scale").map_or(100, |zoom| zoom.clamp(10, 400)),
        show_gridlines: tag.attr_bool("showGridLines").unwrap_or(true),
        show_headers: tag.attr_bool("showRowCol").unwrap_or(true),
        mode: tag.attr("view").map_or(ViewKind::Normal, |name| ViewKind::from_ooxml_name(&name)),
        top_left: tag.attr("topLeftCell").and_then(|name| cell(&name)).unwrap_or((0, 0)),
        ..WindowState::default()
    };
    let mut hidden = HiddenState::default();
    // The page setup elements are read as a worksheet's are.
    let mut scratch = Sheet::new("");
    if !tag.empty {
        loop {
            match r.next() {
                Event::Open(t) => match t.name {
                    "pane" => {
                        let frozen = matches!(t.attr("state").as_deref(), Some("frozen" | "frozenSplit"));
                        if frozen {
                            let split = |name: &str, max: u32| {
                                let n = t.attr_f64(name).unwrap_or(0.0).round();
                                if n >= 1.0 && n < max as f64 { n as u32 } else { 0 }
                            };
                            window.frozen_cols = split("xSplit", MAX_COLS);
                            window.frozen_rows = split("ySplit", MAX_ROWS);
                            if let Some(top_left) = t.attr("topLeftCell").and_then(|name| cell(&name)) {
                                window.top_left = top_left;
                            }
                        }
                        r.skip(&t);
                    }
                    "selection" => {
                        let ranges = t.attr("sqref").map(|text| super::validation::parse_sqref(&text)).unwrap_or_default();
                        let active = t.attr("activeCell").and_then(|name| cell(&name));
                        // The last selection read is that of the active pane.
                        if let Some(active) = active.or(ranges.first().map(|range| (range.r0, range.c0))) {
                            window.active = active;
                            window.ranges = if ranges.is_empty() { vec![Rect::cell(active.0, active.1)] } else { ranges };
                        }
                        r.skip(&t);
                    }
                    "autoFilter" => hidden.filter = super::filter::parse_auto_filter(r, &t, colors),
                    _ if page::read(r, &t, &mut scratch) => {}
                    _ => r.skip(&t),
                },
                Event::Close(_) | Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    Some(CustomSheetView {
        guid,
        // The workbook part names the view (`finish`).
        name: String::new(),
        window,
        hidden: Some(hidden),
        page: Some(scratch.page),
    })
}

/// Read `<customSheetViews>` of a worksheet. `colors` are the differential
/// formats of the workbook, for filters that go by a colour.
pub(crate) fn read_sheet<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet, colors: &[DxfColors]) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) if t.name == "customSheetView" => {
                if let Some(view) = read_view(r, &t, colors) {
                    if sheet.custom_view(&view.guid).is_none() {
                        sheet.custom_views.push(view);
                    }
                }
            }
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
}

/// Take a defined name of the workbook part if it is one of the hidden
/// names of a view (`name` as written). Returns whether it was one, read
/// or not.
pub(crate) fn read_name(name: &str, text: &str, sheet: Option<&mut Sheet>) -> bool {
    let upper = name.to_uppercase();
    let Some((guid, suffix)) = upper.strip_prefix("Z_").and_then(|rest| rest.split_once("_.WVU.")) else {
        return false;
    };
    let guid = format!("{{{}}}", guid.replace('_', "-"));
    if !is_guid(&guid) {
        return false;
    }
    let Some(part) = sheet.and_then(|sheet| sheet.custom_views.iter_mut().find(|part| part.guid == guid)) else {
        return true;
    };
    // The text is a list of ranges, as a print area is.
    let mut scratch = PageSetup::default();
    let ranges = |scratch: &mut PageSetup| {
        scratch.set_print_area_formula(text);
        std::mem::take(&mut scratch.print_area)
    };
    match suffix {
        "ROWS" => {
            if let Some(hidden) = part.hidden.as_mut() {
                for range in ranges(&mut scratch) {
                    // A damaged name must not hide a million rows one by one.
                    hidden.rows.extend(range.r0..=range.r1.min(range.r0.saturating_add(100_000)));
                }
            }
        }
        "COLS" => {
            if let Some(hidden) = part.hidden.as_mut() {
                for range in ranges(&mut scratch) {
                    hidden.cols.extend(range.c0..=range.c1);
                }
            }
        }
        "PRINTAREA" => {
            if let Some(setup) = part.page.as_mut() {
                setup.set_print_area_formula(text);
            }
        }
        "PRINTTITLES" => {
            if let Some(setup) = part.page.as_mut() {
                setup.set_print_titles_formula(text);
            }
        }
        // The range of the filter is in its element.
        _ => {}
    }
    true
}

/// Once the sheets and the names are read: the parts the sheets hold get
/// the name of their view and lose what the view does not include; parts
/// of views the workbook does not list are dropped. `sheet_ids` gives the
/// `sheetId` of each sheet of `wb`.
pub(crate) fn finish(wb: &mut Workbook, views: &[BookView], sheet_ids: &[u32]) {
    for (index, sheet) in wb.sheets.iter_mut().enumerate() {
        let id = sheet_ids.get(index).copied();
        sheet.custom_views.retain_mut(|part| {
            let Some(view) = views.iter().find(|view| view.guid == part.guid) else {
                return false;
            };
            part.name = view.name.clone();
            part.window.selected = id == Some(view.active_sheet_id);
            if !view.hidden_settings {
                part.hidden = None;
            }
            if !view.print_settings {
                part.page = None;
            }
            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_rows_are_named_as_runs() {
        let rows: BTreeSet<u32> = [2, 3, 4, 7].into_iter().collect();
        assert_eq!(runs(&rows, true), vec![Rect::new(2, 0, 4, MAX_COLS - 1), Rect::new(7, 0, 7, MAX_COLS - 1)]);
        let cols: BTreeSet<u32> = [0, 1, 5].into_iter().collect();
        assert_eq!(runs(&cols, false), vec![Rect::new(0, 0, MAX_ROWS - 1, 1), Rect::new(0, 5, MAX_ROWS - 1, 5)]);
        assert_eq!(ranges_formula("Sheet1", runs(&rows, true)).as_deref(), Some("Sheet1!$3:$5,Sheet1!$8:$8"));
        assert_eq!(ranges_formula("My sheet", runs(&cols, false)).as_deref(), Some("'My sheet'!$A:$B,'My sheet'!$F:$F"));
        assert_eq!(ranges_formula("Sheet1", Vec::new()), None);
        assert_eq!(name_prefix("{6F2E3C1A-0B1C-4D2E-8F30-A1B2C3D4E5F6}"), "Z_6F2E3C1A_0B1C_4D2E_8F30_A1B2C3D4E5F6_.wvu.");
    }

    #[test]
    fn page_elements_are_reordered_for_a_view() {
        let mut setup = PageSetup::default();
        setup.row_breaks.insert(10);
        let mut x = String::new();
        push_page(&mut x, &setup);
        let at = |tag: &str| x.find(tag).unwrap_or_else(|| panic!("{tag} in {x}"));
        assert!(at("<rowBreaks") < at("<pageMargins"));
        setup.orientation = opensheet_engine::page::Orientation::Landscape;
        let mut x = String::new();
        push_page(&mut x, &setup);
        let at = |tag: &str| x.find(tag).unwrap_or_else(|| panic!("{tag} in {x}"));
        assert!(at("<rowBreaks") < at("<pageMargins") && at("<pageMargins") < at("<pageSetup"));
        // Nothing is lost on the way.
        let mut plain = String::new();
        page::write(&mut plain, &setup);
        assert_eq!(x.len(), plain.len());
    }
}
