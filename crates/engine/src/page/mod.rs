//! Page setup and pagination for printing (`Sheet::page`).
//!
//! `PageSetup` holds everything the Page Setup dialog and the Page Layout
//! tab edit, shaped like the OOXML elements that store it (`pageSetup`,
//! `pageMargins`, `printOptions`, `headerFooter`, `rowBreaks`, `colBreaks`
//! and the `_xlnm.Print_Area` / `_xlnm.Print_Titles` names). `paginate`
//! turns a sheet and its setup into the list of pages that print.

mod header_footer;
mod paginate;
mod paper;

pub use header_footer::{
    HeaderFooter, HfContext, HfFormat, HfPicture, HfRun, HfSection, HfSections, HfSlot, HfVariant, Script, expand,
    from_dialog_text, lines, plain_text, presets, to_dialog_text,
};
pub use paginate::{
    CELL_PADDING, INDENT_WIDTH, PX_TO_PT, Page, PageBreak, PageLayout, PaginateOptions, Pagination, PrintRange, Span,
    TextMeasure, axis_offset, paginate, visible_col_width, visible_row_height,
};
pub use paper::{PaperInfo, PaperSize};

use crate::ast::{StructuralEdit, write_sheet_name};
use crate::chart::Chart;
use crate::drawing::Drawing;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, col_to_letters};
use crate::workbook::{Sheet, Workbook};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl Orientation {
    /// The value of `pageSetup/@orientation`.
    pub fn as_ooxml(self) -> &'static str {
        match self {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
    }

    /// Anything but "landscape" (including "default") is portrait.
    pub fn from_ooxml(s: &str) -> Orientation {
        if s == "landscape" {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }
}

/// The order pages are numbered and printed in when the sheet is both
/// wider and taller than one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PageOrder {
    #[default]
    DownThenOver,
    OverThenDown,
}

impl PageOrder {
    /// The value of `pageSetup/@pageOrder`.
    pub fn as_ooxml(self) -> &'static str {
        match self {
            PageOrder::DownThenOver => "downThenOver",
            PageOrder::OverThenDown => "overThenDown",
        }
    }

    pub fn from_ooxml(s: &str) -> PageOrder {
        if s == "overThenDown" {
            PageOrder::OverThenDown
        } else {
            PageOrder::DownThenOver
        }
    }
}

/// Whether and where the notes of the cells print.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PrintComments {
    #[default]
    None,
    /// On a page of their own after the sheet.
    AtEnd,
    /// Where they are shown on the sheet.
    AsDisplayed,
}

impl PrintComments {
    /// The value of `pageSetup/@cellComments`.
    pub fn as_ooxml(self) -> &'static str {
        match self {
            PrintComments::None => "none",
            PrintComments::AtEnd => "atEnd",
            PrintComments::AsDisplayed => "asDisplayed",
        }
    }

    pub fn from_ooxml(s: &str) -> PrintComments {
        match s {
            "atEnd" => PrintComments::AtEnd,
            "asDisplayed" => PrintComments::AsDisplayed,
            _ => PrintComments::None,
        }
    }
}

/// How cells holding an error value print.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PrintErrors {
    #[default]
    Displayed,
    Blank,
    /// `--`
    Dash,
    /// `#N/A`
    NA,
}

impl PrintErrors {
    /// The value of `pageSetup/@errors`.
    pub fn as_ooxml(self) -> &'static str {
        match self {
            PrintErrors::Displayed => "displayed",
            PrintErrors::Blank => "blank",
            PrintErrors::Dash => "dash",
            PrintErrors::NA => "NA",
        }
    }

    pub fn from_ooxml(s: &str) -> PrintErrors {
        match s {
            "blank" => PrintErrors::Blank,
            "dash" => PrintErrors::Dash,
            "NA" => PrintErrors::NA,
            _ => PrintErrors::Displayed,
        }
    }
}

/// Page margins in inches, as `pageMargins` stores them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Margins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    /// Distance from the top of the paper to the header.
    pub header: f64,
    /// Distance from the bottom of the paper to the footer.
    pub footer: f64,
}

/// The presets of the ribbon's Margins gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarginPreset {
    Normal,
    Wide,
    Narrow,
}

impl Margins {
    pub const NORMAL: Margins = Margins {
        left: 0.7,
        right: 0.7,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    };
    pub const WIDE: Margins = Margins {
        left: 1.0,
        right: 1.0,
        top: 1.0,
        bottom: 1.0,
        header: 0.5,
        footer: 0.5,
    };
    pub const NARROW: Margins = Margins {
        left: 0.25,
        right: 0.25,
        top: 0.75,
        bottom: 0.75,
        header: 0.3,
        footer: 0.3,
    };

    pub fn from_preset(preset: MarginPreset) -> Margins {
        match preset {
            MarginPreset::Normal => Margins::NORMAL,
            MarginPreset::Wide => Margins::WIDE,
            MarginPreset::Narrow => Margins::NARROW,
        }
    }

    /// The preset these margins are, if any (to tick it in the gallery).
    pub fn preset(&self) -> Option<MarginPreset> {
        [MarginPreset::Normal, MarginPreset::Wide, MarginPreset::Narrow]
            .into_iter()
            .find(|p| {
                let m = Margins::from_preset(*p);
                let close = |a: f64, b: f64| (a - b).abs() < 0.005;
                close(m.left, self.left)
                    && close(m.right, self.right)
                    && close(m.top, self.top)
                    && close(m.bottom, self.bottom)
                    && close(m.header, self.header)
                    && close(m.footer, self.footer)
            })
    }
}

impl Default for Margins {
    fn default() -> Margins {
        Margins::NORMAL
    }
}

/// How a sheet prints: paper, orientation, margins, scaling, print area,
/// titles, headers and footers, manual page breaks.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    /// `None` = the paper of the user's region (see
    /// [`PageSetup::paper_size`]), as for a sheet whose file names none.
    pub paper: Option<PaperSize>,
    pub orientation: Orientation,
    /// "Adjust to": percentage of the normal size, 10 to 400. Used unless
    /// `fit_to_page` is set; both settings are kept, as in the dialog.
    pub scale: u16,
    /// "Fit to" is selected instead of "Adjust to".
    pub fit_to_page: bool,
    /// "Fit to": pages wide; 0 = as many as needed.
    pub fit_width: u16,
    /// "Fit to": pages tall; 0 = as many as needed.
    pub fit_height: u16,
    /// `None` = automatic: 1, or the number after the previous sheet's
    /// last page when several sheets print together.
    pub first_page_number: Option<i32>,
    pub page_order: PageOrder,
    /// "Print quality" in dots per inch, across and down; `None` = the
    /// printer's own. Kept for the file: PDF output has no resolution.
    pub horizontal_dpi: Option<u32>,
    pub vertical_dpi: Option<u32>,
    pub black_and_white: bool,
    pub draft: bool,
    pub comments: PrintComments,
    pub errors: PrintErrors,
    pub center_horizontally: bool,
    pub center_vertically: bool,
    /// Print the gridlines (independent of showing them on screen).
    pub gridlines: bool,
    /// Print the row numbers and column letters.
    pub headings: bool,
    pub margins: Margins,
    /// The ranges to print, each on pages of its own; empty = the whole
    /// used part of the sheet.
    pub print_area: Vec<Rect>,
    /// Rows repeated at the top of every page (first, last).
    pub title_rows: Option<(u32, u32)>,
    /// Columns repeated at the left of every page (first, last).
    pub title_cols: Option<(u32, u32)>,
    /// Manual horizontal page breaks: the rows that start a new page.
    pub row_breaks: BTreeSet<u32>,
    /// Manual vertical page breaks: the columns that start a new page.
    pub col_breaks: BTreeSet<u32>,
    pub header_footer: HeaderFooter,
}

impl Default for PageSetup {
    fn default() -> PageSetup {
        PageSetup {
            paper: None,
            orientation: Orientation::Portrait,
            scale: 100,
            fit_to_page: false,
            fit_width: 1,
            fit_height: 1,
            first_page_number: None,
            page_order: PageOrder::DownThenOver,
            horizontal_dpi: None,
            vertical_dpi: None,
            black_and_white: false,
            draft: false,
            comments: PrintComments::None,
            errors: PrintErrors::Displayed,
            center_horizontally: false,
            center_vertically: false,
            gridlines: false,
            headings: false,
            margins: Margins::NORMAL,
            print_area: Vec::new(),
            title_rows: None,
            title_cols: None,
            row_breaks: BTreeSet::new(),
            col_breaks: BTreeSet::new(),
            header_footer: HeaderFooter::default(),
        }
    }
}

impl PageSetup {
    /// The paper the sheet prints on.
    pub fn paper_size(&self, locale: &Locale) -> PaperSize {
        self.paper.unwrap_or_else(|| PaperSize::default_for(locale))
    }

    /// Paper width and height in points, turned for the orientation.
    pub fn paper_pt(&self, locale: &Locale) -> (f64, f64) {
        let (w, h) = self.paper_size(locale).size_pt();
        match self.orientation {
            Orientation::Portrait => (w, h),
            Orientation::Landscape => (h, w),
        }
    }

    /// The text of the sheet's `_xlnm.Print_Area` defined name
    /// (`Sheet1!$A$1:$C$9,Sheet1!$E$1:$F$9`); `None` without a print area.
    pub fn print_area_formula(&self, sheet_name: &str) -> Option<String> {
        if self.print_area.is_empty() {
            return None;
        }
        let parts: Vec<String> = self.print_area.iter().map(|r| absolute_ref(sheet_name, *r)).collect();
        Some(parts.join(","))
    }

    /// The text of the sheet's `_xlnm.Print_Titles` defined name, columns
    /// first as Excel writes it (`Sheet1!$A:$B,Sheet1!$1:$2`).
    pub fn print_titles_formula(&self, sheet_name: &str) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some((c0, c1)) = self.title_cols {
            parts.push(absolute_ref(sheet_name, Rect::new(0, c0, MAX_ROWS - 1, c1)));
        }
        if let Some((r0, r1)) = self.title_rows {
            parts.push(absolute_ref(sheet_name, Rect::new(r0, 0, r1, MAX_COLS - 1)));
        }
        (!parts.is_empty()).then(|| parts.join(","))
    }

    /// Read the text of a `_xlnm.Print_Area` name. Returns false, leaving
    /// the print area alone, when a part is not a plain range.
    pub fn set_print_area_formula(&mut self, text: &str) -> bool {
        let Some(rects) = parse_ref_list(text) else {
            return false;
        };
        self.print_area = rects;
        true
    }

    /// Read the text of a `_xlnm.Print_Titles` name: whole rows, whole
    /// columns or both.
    pub fn set_print_titles_formula(&mut self, text: &str) -> bool {
        let Some(rects) = parse_ref_list(text) else {
            return false;
        };
        let mut rows = None;
        let mut cols = None;
        for r in rects {
            if r.is_whole_rows() && !r.is_whole_cols() {
                rows = Some((r.r0, r.r1));
            } else if r.is_whole_cols() && !r.is_whole_rows() {
                cols = Some((r.c0, r.c1));
            } else {
                return false;
            }
        }
        self.title_rows = rows;
        self.title_cols = cols;
        true
    }
}

/// `Sheet1!$A$1:$C$9`, `'My sheet'!$1:$2`, `Sheet1!$A:$B`.
fn absolute_ref(sheet_name: &str, r: Rect) -> String {
    let mut out = String::new();
    write_sheet_name(sheet_name, &mut out);
    out.push('!');
    if r.is_whole_rows() && !r.is_whole_cols() {
        out.push_str(&format!("${}:${}", r.r0 + 1, r.r1 + 1));
    } else if r.is_whole_cols() && !r.is_whole_rows() {
        out.push_str(&format!("${}:${}", col_to_letters(r.c0), col_to_letters(r.c1)));
    } else {
        out.push_str(&format!("${}${}", col_to_letters(r.c0), r.r0 + 1));
        if !r.is_single() {
            out.push_str(&format!(":${}${}", col_to_letters(r.c1), r.r1 + 1));
        }
    }
    out
}

/// Parse a comma-separated list of ranges, each optionally qualified by a
/// sheet name (which is ignored: these names are local to their sheet).
fn parse_ref_list(text: &str) -> Option<Vec<Rect>> {
    let mut out = Vec::new();
    let mut part = String::new();
    let mut quoted = false;
    let flush = |part: &mut String, out: &mut Vec<Rect>| -> Option<()> {
        let s = part.trim().trim_start_matches('=');
        if !s.is_empty() {
            // The range follows the last `!` outside quotes; quotes only
            // ever surround the sheet name.
            let range = match s.rfind('!') {
                Some(i) => &s[i + 1..],
                None => s,
            };
            out.push(Rect::parse(range)?);
        }
        part.clear();
        Some(())
    };
    for ch in text.chars() {
        match ch {
            '\'' => {
                quoted = !quoted;
                part.push(ch);
            }
            ',' if !quoted => flush(&mut part, &mut out)?,
            _ => part.push(ch),
        }
    }
    flush(&mut part, &mut out)?;
    Some(out)
}

// -------------------------------------------------------- floating objects

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Chart,
    Drawing,
}

/// Where a chart, picture or shape sits on its sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingObject {
    pub kind: ObjectKind,
    /// The id of the chart or of the drawing.
    pub id: u32,
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell, pixels at 100 % (x, y).
    pub offset: (f32, f32),
    /// Width and height, pixels at 100 %.
    pub size: (f32, f32),
}

impl FloatingObject {
    fn of_chart(chart: &Chart) -> FloatingObject {
        FloatingObject {
            kind: ObjectKind::Chart,
            id: chart.id,
            anchor: chart.anchor,
            offset: chart.offset,
            size: chart.size,
        }
    }

    /// The object's box in sheet pixels from the top-left corner of A1:
    /// (x, y, width, height).
    pub fn sheet_box(&self, sheet: &Sheet) -> (f64, f64, f64, f64) {
        (
            axis_offset(sheet, false, self.anchor.1) + self.offset.0 as f64,
            axis_offset(sheet, true, self.anchor.0) + self.offset.1 as f64,
            self.size.0 as f64,
            self.size.1 as f64,
        )
    }

    /// The cells the object covers, at least partly.
    pub fn cells(&self, sheet: &Sheet) -> Rect {
        let end = |rows: bool, first: u32, offset: f32, size: f32, max: u32| -> u32 {
            let mut left = (offset + size) as f64;
            let mut i = first;
            loop {
                let s = if rows {
                    visible_row_height(sheet, i)
                } else {
                    visible_col_width(sheet, i)
                };
                // An object ending exactly on a cell boundary does not
                // reach into the next cell.
                if left <= s + 1e-6 || i + 1 >= max {
                    return i;
                }
                left -= s;
                i += 1;
            }
        };
        Rect::new(
            self.anchor.0,
            self.anchor.1,
            end(true, self.anchor.0, self.offset.1, self.size.1, MAX_ROWS),
            end(false, self.anchor.1, self.offset.0, self.size.0, MAX_COLS),
        )
    }
}

/// Where a picture or shape sits. The drawing model does not carry an
/// anchor or a size yet, so drawings take no room on paper; this is the
/// one place that has to read them from it.
fn drawing_object(_drawing: &Drawing) -> Option<FloatingObject> {
    None
}

/// The charts, pictures and shapes of a sheet, in drawing order.
pub fn floating_objects(sheet: &Sheet) -> Vec<FloatingObject> {
    let mut out: Vec<FloatingObject> = sheet.charts.iter().map(FloatingObject::of_chart).collect();
    out.extend(sheet.drawings.iter().filter_map(drawing_object));
    out
}

// ------------------------------------------------------------------ edits

/// Which page breaks an operation is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BreakAxis {
    /// Horizontal breaks, between rows.
    Rows,
    /// Vertical breaks, between columns.
    Cols,
}

/// The breaks "Insert Page Break" adds for a selection: above its first
/// row and left of its first column, or only one of them when whole rows
/// or whole columns are selected (or the selection touches row 1 or
/// column A). Returns (row, column).
pub fn breaks_for_selection(selection: Rect) -> (Option<u32>, Option<u32>) {
    let row = (selection.r0 > 0 && !selection.is_whole_cols()).then_some(selection.r0);
    let col = (selection.c0 > 0 && !selection.is_whole_rows()).then_some(selection.c0);
    (row, col)
}

impl Workbook {
    /// Change the page setup of a sheet. Like every edit of this module it
    /// takes the undo snapshot itself and leaves `commit` to the caller;
    /// nothing is recorded when `f` changes nothing.
    pub fn update_page_setup(&mut self, sheet: usize, f: impl FnOnce(&mut PageSetup)) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let mut setup = sh.page.clone();
        f(&mut setup);
        if setup == sh.page {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].page = setup;
        true
    }

    /// Replace the page setup of a sheet (the Page Setup dialog's OK).
    pub fn set_page_setup(&mut self, sheet: usize, setup: PageSetup) -> bool {
        self.update_page_setup(sheet, |p| *p = setup)
    }

    /// "Set Print Area": print these ranges only.
    pub fn set_print_area(&mut self, sheet: usize, ranges: &[Rect]) -> bool {
        self.update_page_setup(sheet, |p| p.print_area = ranges.to_vec())
    }

    /// "Add to Print Area": one more range, printed on pages of its own.
    pub fn add_to_print_area(&mut self, sheet: usize, range: Rect) -> bool {
        self.update_page_setup(sheet, |p| {
            if !p.print_area.contains(&range) {
                p.print_area.push(range);
            }
        })
    }

    /// "Clear Print Area".
    pub fn clear_print_area(&mut self, sheet: usize) -> bool {
        self.update_page_setup(sheet, |p| p.print_area.clear())
    }

    /// "Print Titles": the rows repeated at the top and the columns
    /// repeated at the left of every page, as (first, last).
    pub fn set_print_titles(&mut self, sheet: usize, rows: Option<(u32, u32)>, cols: Option<(u32, u32)>) -> bool {
        let order = |v: Option<(u32, u32)>| v.map(|(a, b)| (a.min(b), a.max(b)));
        self.update_page_setup(sheet, |p| {
            p.title_rows = order(rows);
            p.title_cols = order(cols);
        })
    }

    /// "Insert Page Break": a manual break above `row` and / or left of
    /// `col` (see [`breaks_for_selection`]). A break cannot sit before the
    /// first row or column.
    pub fn insert_page_break(&mut self, sheet: usize, row: Option<u32>, col: Option<u32>) -> bool {
        self.update_page_setup(sheet, |p| {
            if let Some(r) = row.filter(|r| *r > 0 && *r < MAX_ROWS) {
                p.row_breaks.insert(r);
            }
            if let Some(c) = col.filter(|c| *c > 0 && *c < MAX_COLS) {
                p.col_breaks.insert(c);
            }
        })
    }

    /// "Remove Page Break": the manual breaks above `row` and left of
    /// `col`.
    pub fn remove_page_break(&mut self, sheet: usize, row: Option<u32>, col: Option<u32>) -> bool {
        self.update_page_setup(sheet, |p| {
            if let Some(r) = row {
                p.row_breaks.remove(&r);
            }
            if let Some(c) = col {
                p.col_breaks.remove(&c);
            }
        })
    }

    /// "Reset All Page Breaks".
    pub fn reset_page_breaks(&mut self, sheet: usize) -> bool {
        self.update_page_setup(sheet, |p| {
            p.row_breaks.clear();
            p.col_breaks.clear();
        })
    }

    /// Drag a page break of Page Break Preview from `from` to `to` (both
    /// the row or column that starts the next page). An automatic break
    /// becomes manual. A manual break dropped outside the printed range
    /// is removed. When the page the break closes grows beyond what fits
    /// (an automatic break dragged to the end of the range makes it hold
    /// all that is left), the print scale shrinks until it does, as in
    /// Excel; a sheet set to "Fit to" switches to the equivalent "Adjust
    /// to" first.
    pub fn move_page_break(
        &mut self,
        sheet: usize,
        axis: BreakAxis,
        from: u32,
        to: u32,
        measure: Option<&dyn TextMeasure>,
    ) -> bool {
        if sheet >= self.sheets.len() {
            return false;
        }
        let opts = PaginateOptions {
            measure,
            ..PaginateOptions::default()
        };
        let before = paginate(self, sheet, &opts);
        // The range the dragged break belongs to, and the start of the
        // page that break closes.
        let found = before.ranges.iter().find_map(|range| {
            let (first, last, breaks) = match axis {
                BreakAxis::Rows => (range.rect.r0, range.rect.r1, &range.row_breaks),
                BreakAxis::Cols => (range.rect.c0, range.rect.c1, &range.col_breaks),
            };
            breaks.iter().any(|b| b.at == from).then(|| {
                let start = breaks
                    .iter()
                    .map(|b| b.at)
                    .filter(|at| *at < from)
                    .max()
                    .unwrap_or(first);
                (first, last, start)
            })
        });
        let Some((first, last, page_start)) = found else {
            return false;
        };
        let scale_percent = before.scale_percent;
        let inside = to > first && to <= last;
        let sh = &self.sheets[sheet];
        let was_manual = match axis {
            BreakAxis::Rows => sh.page.row_breaks.contains(&from),
            BreakAxis::Cols => sh.page.col_breaks.contains(&from),
        };
        // The last row or column of the page the break closes, when the
        // drag made that page longer.
        let grown_to = if inside && to > from {
            Some(to - 1)
        } else if to > last && !was_manual {
            Some(last)
        } else {
            None
        };
        let available_pt = match axis {
            BreakAxis::Rows => before.printable.3,
            BreakAxis::Cols => before.printable.2,
        };
        // What the page closed by the break holds once it has grown:
        // its own rows or columns, the titles repeated on it and the
        // heading band.
        let grown_px = grown_to.map(|end| {
            let (titles, heading_px) = match axis {
                BreakAxis::Rows => (sh.page.title_rows, before.heading_size.1),
                BreakAxis::Cols => (sh.page.title_cols, before.heading_size.0),
            };
            let title_px = titles
                .filter(|(_, t1)| page_start > *t1)
                .map_or(0.0, |(t0, t1)| paginate::span_px(sh, axis, t0, t1));
            paginate::span_px(sh, axis, page_start, end) + title_px + heading_px
        });
        self.update_page_setup(sheet, |p| {
            let breaks = match axis {
                BreakAxis::Rows => &mut p.row_breaks,
                BreakAxis::Cols => &mut p.col_breaks,
            };
            breaks.remove(&from);
            if inside {
                breaks.insert(to);
            }
            if p.fit_to_page {
                p.fit_to_page = false;
                p.scale = scale_percent;
            }
            if let Some(px) = grown_px.filter(|px| *px > 0.0) {
                let fits = (available_pt / (px * PX_TO_PT) * 100.0 + 1e-9).floor() as u16;
                p.scale = p.scale.min(fits).max(10);
            }
        })
    }
}

// ------------------------------------------------------------------ hooks

/// Move an index with an insertion or deletion; `None` when it was deleted.
fn shift_index(index: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<u32> {
    if insert {
        let moved = if index >= at { index + count } else { index };
        (moved < max).then_some(moved)
    } else if index < at {
        Some(index)
    } else if index >= at + count {
        Some(index - count)
    } else {
        None
    }
}

/// Move an inclusive interval with an insertion or deletion; it shrinks
/// when part of it is deleted and is `None` when all of it is.
fn shift_span(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32)> {
    if insert {
        let nlo = if lo >= at { lo + count } else { lo };
        let nhi = if hi >= at { hi + count } else { hi };
        (nlo < max).then(|| (nlo, nhi.min(max - 1)))
    } else {
        let end = at + count - 1;
        if lo >= at && hi <= end {
            return None;
        }
        let nlo = if lo > end {
            lo - count
        } else if lo >= at {
            at
        } else {
            lo
        };
        let nhi = if hi > end {
            hi - count
        } else if hi >= at {
            at - 1
        } else {
            hi
        };
        Some((nlo, nhi))
    }
}

/// See `features::on_structural_edit`: manual breaks, the print area and
/// the print titles follow the rows and columns they were set on.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if !sheet.name.eq_ignore_ascii_case(edited_sheet) {
        return;
    }
    let (rows, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    if count == 0 {
        return;
    }
    let p = &mut sheet.page;
    if rows {
        // A break belongs to the row below it: it goes when that row does.
        p.row_breaks = p
            .row_breaks
            .iter()
            .filter_map(|r| shift_index(*r, insert, at, count, MAX_ROWS))
            .collect();
        p.row_breaks.remove(&0);
        p.title_rows = p
            .title_rows
            .and_then(|(a, b)| shift_span(a, b, insert, at, count, MAX_ROWS));
        p.print_area = p
            .print_area
            .iter()
            .filter_map(|r| {
                if r.is_whole_cols() {
                    return Some(*r);
                }
                shift_span(r.r0, r.r1, insert, at, count, MAX_ROWS).map(|(a, b)| Rect::new(a, r.c0, b, r.c1))
            })
            .collect();
    } else {
        p.col_breaks = p
            .col_breaks
            .iter()
            .filter_map(|c| shift_index(*c, insert, at, count, MAX_COLS))
            .collect();
        p.col_breaks.remove(&0);
        p.title_cols = p
            .title_cols
            .and_then(|(a, b)| shift_span(a, b, insert, at, count, MAX_COLS));
        p.print_area = p
            .print_area
            .iter()
            .filter_map(|r| {
                if r.is_whole_rows() {
                    return Some(*r);
                }
                shift_span(r.c0, r.c1, insert, at, count, MAX_COLS).map(|(a, b)| Rect::new(r.r0, a, r.r1, b))
            })
            .collect();
    }
}

/// See `features::on_sheet_renamed`. The print area and titles are kept as
/// ranges of their own sheet, so a new name changes nothing here.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`. The copy keeps the whole page
/// setup of the original (print area, titles, breaks, headers), which the
/// clone of the sheet already carries: nothing names the original sheet.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}

/// See `features::on_sheet_deleted`.
pub(crate) fn on_sheet_deleted(_wb: &mut crate::workbook::Workbook, _deleted: &crate::workbook::Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`.
pub(crate) fn on_sheet_moved(_wb: &mut crate::workbook::Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`.
pub(crate) fn on_cells_shifted(_wb: &mut crate::workbook::Workbook, _sheet: usize, _shift: crate::features::CellShift) {}

#[cfg(test)]
mod tests;
