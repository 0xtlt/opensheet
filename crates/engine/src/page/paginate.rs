//! Pagination: which rows and columns of a sheet print on which page.
//!
//! Sheet geometry is in pixels at 100 % zoom, as everywhere in the engine;
//! paper geometry is in points (1/72 inch) from the top-left corner of the
//! paper. One pixel prints as 0.75 point at a scale of 100 %.

use super::{BreakAxis, PageOrder, PageSetup, floating_objects};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Font, HAlign};
use crate::value::Value;
use crate::workbook::{Content, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, Sheet, Workbook};
use std::collections::{BTreeSet, HashSet};

/// Points per sheet pixel at a print scale of 100 %.
pub const PX_TO_PT: f64 = 0.75;
/// Blank space between a cell's edge and its text, in pixels.
pub const CELL_PADDING: f32 = 3.0;
/// Width of one indent level, in pixels.
pub const INDENT_WIDTH: f32 = 9.0;

/// Fit-to never enlarges and Excel never prints below 10 %.
const MIN_SCALE: u16 = 10;
const MAX_SCALE: u16 = 400;
/// Slack when comparing sizes that were summed from `f32` values.
const EPS: f64 = 1e-6;

/// Measures text, so that text flowing out of its cell into blank
/// neighbours counts as part of what prints. Pass the same measure the
/// printer draws with to get the same pages.
pub trait TextMeasure {
    /// Width of `text` on one line in `font`, in pixels at 100 % zoom.
    fn text_width(&self, text: &str, font: &Font) -> f32;
}

/// What to paginate, when it is not simply the sheet as set up.
#[derive(Clone, Copy, Default)]
pub struct PaginateOptions<'a> {
    /// Print these ranges instead of the print area ("Print Selection").
    pub selection: Option<&'a [Rect]>,
    /// "Ignore Print Area": print the whole sheet.
    pub ignore_print_area: bool,
    /// The number of the first page when the sheet's own first page
    /// number is automatic (its place in a job of several sheets).
    pub first_page_number: Option<i32>,
    /// Without it, text overflowing its cell does not extend what prints.
    pub measure: Option<&'a dyn TextMeasure>,
}

/// A page break between two pages of a printed range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageBreak {
    /// The row or column that starts the next page.
    pub at: u32,
    /// Set by the user (solid line) rather than by the paper running out
    /// (dashed line).
    pub manual: bool,
}

/// One range that prints: a range of the print area or of the selection,
/// or the used part of the sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct PrintRange {
    pub rect: Rect,
    /// The columns of each page across (first, last), left to right.
    /// Hidden columns at the edges of a band are left out.
    pub col_bands: Vec<(u32, u32)>,
    /// The rows of each page down (first, last), top to bottom.
    pub row_bands: Vec<(u32, u32)>,
    /// The breaks between the column bands, one fewer than the bands.
    pub col_breaks: Vec<PageBreak>,
    pub row_breaks: Vec<PageBreak>,
}

/// One printed page.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Position in the sheet's printout, from 0.
    pub index: usize,
    /// The number `&P` prints.
    pub number: i32,
    /// Index into `Pagination::ranges`.
    pub range: usize,
    /// Which band of the range's `col_bands` / `row_bands` this page is.
    pub col_band: usize,
    pub row_band: usize,
    /// The rows (first, last) and columns of the page's own cells.
    pub rows: (u32, u32),
    pub cols: (u32, u32),
    /// The print-title rows repeated above the page's own rows.
    pub title_rows: Option<(u32, u32)>,
    /// The print-title columns repeated left of the page's own columns.
    pub title_cols: Option<(u32, u32)>,
    /// Top-left corner of what prints (headings and titles included), in
    /// points from the top-left corner of the paper, centring applied.
    pub origin: (f64, f64),
    /// Width and height of what prints, in points.
    pub size: (f64, f64),
}

/// The pages of a sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Pagination {
    /// Paper width and height in points, turned for the orientation.
    pub paper: (f64, f64),
    /// The area inside the margins: left, top, width, height, in points.
    pub printable: (f64, f64, f64, f64),
    /// The scale in effect: the "Adjust to" percentage, or the one "Fit
    /// to" works out.
    pub scale_percent: u16,
    /// Width of the row-number band and height of the column-letter band,
    /// in pixels at 100 %; zero when headings do not print.
    pub heading_size: (f64, f64),
    pub ranges: Vec<PrintRange>,
    /// In printing order. Empty when there is nothing to print.
    pub pages: Vec<Page>,
}

/// A row or a column placed on paper.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub index: u32,
    /// Left (or top) edge in points from the paper's left (or top) edge.
    pub pos: f64,
    /// Width (or height) in points.
    pub size: f64,
}

/// Where everything of one page falls on the paper, in points from the
/// top-left corner of the paper.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
    /// Points per sheet pixel on this page.
    pub unit: f64,
    /// Everything that prints: x, y, width, height.
    pub block: (f64, f64, f64, f64),
    /// Width of the row-number band and height of the column-letter band
    /// (zero when headings do not print); they start at the block's
    /// corner.
    pub heading: (f64, f64),
    /// The visible columns, repeated title columns first.
    pub cols: Vec<Span>,
    /// The visible rows, repeated title rows first.
    pub rows: Vec<Span>,
    /// How many of the leading `cols` / `rows` are repeated titles.
    pub title_cols: usize,
    pub title_rows: usize,
    /// The page's own cells, without headings and titles: x, y, width,
    /// height.
    pub body: (f64, f64, f64, f64),
    /// Sheet position of the body's top-left corner, in pixels from A1.
    pub body_px: (f64, f64),
}

impl PageLayout {
    /// Paper position of a point of the sheet given in pixels from the
    /// top-left corner of A1 (for floating objects). Only meaningful for
    /// points on the page's own cells.
    pub fn to_paper(&self, x_px: f64, y_px: f64) -> (f64, f64) {
        (
            self.body.0 + (x_px - self.body_px.0) * self.unit,
            self.body.1 + (y_px - self.body_px.1) * self.unit,
        )
    }

    pub fn col(&self, col: u32) -> Option<&Span> {
        self.cols.iter().find(|s| s.index == col)
    }

    pub fn row(&self, row: u32) -> Option<&Span> {
        self.rows.iter().find(|s| s.index == row)
    }
}

/// Row height in pixels as it prints: zero when hidden by hand or by a
/// filter.
pub fn visible_row_height(sheet: &Sheet, row: u32) -> f64 {
    if sheet.filtered_rows.contains(&row) {
        0.0
    } else {
        sheet.row_height(row) as f64
    }
}

/// Column width in pixels as it prints: zero when hidden.
pub fn visible_col_width(sheet: &Sheet, col: u32) -> f64 {
    sheet.col_width(col) as f64
}

/// Distance in pixels from the sheet's top (`rows`) or left edge to the
/// start of a row or column, skipping what is hidden.
pub fn axis_offset(sheet: &Sheet, rows: bool, index: u32) -> f64 {
    let (default, sizes, hidden) = if rows {
        (DEFAULT_ROW_HEIGHT, &sheet.row_heights, &sheet.hidden_rows)
    } else {
        (DEFAULT_COL_WIDTH, &sheet.col_widths, &sheet.hidden_cols)
    };
    let size = |i: u32| sizes.get(&i).copied().unwrap_or(default) as f64;
    let mut total = index as f64 * default as f64;
    for (_, s) in sizes.range(..index) {
        total += (*s - default) as f64;
    }
    for i in hidden.range(..index) {
        total -= size(*i);
    }
    if rows {
        for i in sheet.filtered_rows.range(..index) {
            if !hidden.contains(i) {
                total -= size(*i);
            }
        }
    }
    total
}

/// Total printed size in pixels of the rows or columns `lo..=hi`.
pub(super) fn span_px(sheet: &Sheet, axis: BreakAxis, lo: u32, hi: u32) -> f64 {
    if hi < lo {
        return 0.0;
    }
    let rows = axis == BreakAxis::Rows;
    axis_offset(sheet, rows, hi + 1) - axis_offset(sheet, rows, lo)
}

/// The printed sizes of a run of rows or columns.
struct Sizes {
    first: u32,
    px: Vec<f32>,
}

impl Sizes {
    fn new(sheet: &Sheet, rows: bool, first: u32, last: u32) -> Sizes {
        let (default, sizes, hidden) = if rows {
            (DEFAULT_ROW_HEIGHT, &sheet.row_heights, &sheet.hidden_rows)
        } else {
            (DEFAULT_COL_WIDTH, &sheet.col_widths, &sheet.hidden_cols)
        };
        let mut px = vec![default; (last - first + 1) as usize];
        for (i, s) in sizes.range(first..=last) {
            px[(i - first) as usize] = *s;
        }
        for i in hidden.range(first..=last) {
            px[(i - first) as usize] = 0.0;
        }
        if rows {
            for i in sheet.filtered_rows.range(first..=last) {
                px[(i - first) as usize] = 0.0;
            }
        }
        Sizes { first, px }
    }

    fn last(&self) -> u32 {
        self.first + self.px.len() as u32 - 1
    }

    fn get(&self, index: u32) -> f64 {
        self.px[(index - self.first) as usize] as f64
    }
}

/// Cut a run of rows or columns into pages. `available` is the room on a
/// page in pixels; pages that start after the print titles lose
/// `title_px` of it to the repeated titles.
fn bands(
    sizes: &Sizes,
    available: f64,
    title: Option<(u32, u32)>,
    title_px: f64,
    manual: Option<&BTreeSet<u32>>,
) -> (Vec<(u32, u32)>, Vec<PageBreak>) {
    let mut out = Vec::new();
    let mut breaks = Vec::new();
    let room = |start: u32| -> f64 {
        if title.is_some_and(|(_, t1)| start > t1) {
            available - title_px
        } else {
            available
        }
    };
    // The band being filled: its first index, the last visible index put
    // on it, the pixels used and the pixels it can hold.
    let mut current: Option<(u32, u32, f64, f64)> = None;
    for index in sizes.first..=sizes.last() {
        let size = sizes.get(index);
        if size <= 0.0 {
            continue;
        }
        current = Some(match current {
            None => (index, index, size, room(index)),
            Some((start, last, used, limit)) => {
                // A break set on a hidden row applies at the next visible one.
                let forced = manual.is_some_and(|m| m.range(last + 1..=index).next().is_some());
                if forced || used + size > limit + EPS {
                    out.push((start, last));
                    breaks.push(PageBreak {
                        at: index,
                        manual: forced,
                    });
                    (index, index, size, room(index))
                } else {
                    (start, index, used + size, limit)
                }
            }
        });
    }
    if let Some((start, last, _, _)) = current {
        out.push((start, last));
    }
    (out, breaks)
}

/// The first and last band (inclusive) touching `lo..=hi`, if any.
fn bands_touching(bands: &[(u32, u32)], lo: u32, hi: u32) -> Option<(usize, usize)> {
    let first = bands.partition_point(|b| b.1 < lo);
    let end = bands.partition_point(|b| b.0 <= hi);
    (first < end).then(|| (first, end - 1))
}

/// Everything on the sheet that leaves a mark on paper, as cell
/// rectangles: cells with a value or with a fill or border of their own,
/// merged areas, spilled arrays, text flowing over blank neighbours and
/// floating objects.
fn collect_ink(wb: &Workbook, sheet: usize, measure: Option<&dyn TextMeasure>) -> Vec<Rect> {
    let sh = &wb.sheets[sheet];
    let mut ink: Vec<Rect> = Vec::new();
    let marks = |row: u32, col: u32| -> bool {
        sh.cells.get(&(row, col)).is_some_and(|cell| {
            let style = wb.styles.get(cell.style);
            !matches!(cell.content, Content::Empty) || style.fill.is_some() || !style.borders.is_none()
        })
    };

    // Cells under a merged area print nothing of their own.
    let mut covered: HashSet<(u32, u32)> = HashSet::new();
    for m in &sh.merges {
        for (r, c) in sh.cells.range((m.r0, m.c0)..=(m.r1, m.c1)).map(|(k, _)| *k) {
            if c >= m.c0 && c <= m.c1 {
                covered.insert((r, c));
            }
        }
        if marks(m.r0, m.c0) {
            ink.push(*m);
        }
    }

    // Text flows out of the last cell of a row only: anything before it
    // is stopped by the next cell.
    let overflow = |row: u32, col: u32| -> Option<Rect> {
        let measure = measure?;
        if !matches!(sh.value_at(row, col), Value::Text(_)) {
            return None;
        }
        let display = wb.display(sheet, row, col);
        let style = &display.style;
        if style.wrap || style.shrink_to_fit || style.rotation != 0 {
            return None;
        }
        let text = wb.display_text(sheet, row, col).text.replace(['\n', '\r'], " ");
        let width = measure.text_width(&text, &style.font) as f64 + 2.0 * CELL_PADDING as f64;
        let own = visible_col_width(sh, col);
        let mut missing = match style.h_align {
            HAlign::General | HAlign::Left => width + style.indent as f64 * INDENT_WIDTH as f64 - own,
            HAlign::Center | HAlign::CenterAcross => (width - own) / 2.0,
            HAlign::Right | HAlign::Fill | HAlign::Justify => return None,
        };
        let mut end = col;
        while missing > EPS && end + 1 < MAX_COLS && sh.is_blank(row, end + 1) && sh.merge_at(row, end + 1).is_none() {
            end += 1;
            missing -= visible_col_width(sh, end);
        }
        (end > col).then(|| Rect::new(row, col, row, end))
    };

    let mut last_value: Option<(u32, u32)> = None;
    for ((row, col), cell) in &sh.cells {
        if covered.contains(&(*row, *col)) || !marks(*row, *col) {
            continue;
        }
        ink.push(Rect::cell(*row, *col));
        if !matches!(cell.content, Content::Empty) {
            if let Some((r, c)) = last_value.filter(|(r, _)| r != row) {
                ink.extend(overflow(r, c));
            }
            last_value = Some((*row, *col));
        }
    }
    if let Some((r, c)) = last_value {
        ink.extend(overflow(r, c));
    }

    for sp in &sh.spills {
        ink.push(Rect::new(
            sp.row,
            sp.col,
            sp.row + sp.array.rows as u32 - 1,
            sp.col + sp.array.cols as u32 - 1,
        ));
    }
    for object in floating_objects(sh) {
        ink.push(object.cells(sh));
    }
    ink
}

/// Width of the row-number band in pixels: it grows with the digits of
/// the largest row number, like the grid's.
fn heading_width(last_row: u32) -> f64 {
    let digits = (last_row + 1).to_string().len().max(2) as f64;
    (digits * 7.5 + 12.0).max(26.0)
}

/// Paginate a sheet with its page setup.
pub fn paginate(wb: &Workbook, sheet: usize, opts: &PaginateOptions) -> Pagination {
    let default_setup = PageSetup::default();
    let sh = wb.sheets.get(sheet);
    let setup = sh.map_or(&default_setup, |s| &s.page);
    let paper = setup.paper_pt(wb.locale);
    let m = &setup.margins;
    let printable = (
        m.left * 72.0,
        m.top * 72.0,
        (paper.0 - (m.left + m.right) * 72.0).max(1.0),
        (paper.1 - (m.top + m.bottom) * 72.0).max(1.0),
    );
    let mut out = Pagination {
        paper,
        printable,
        scale_percent: setup.scale.clamp(MIN_SCALE, MAX_SCALE),
        heading_size: (0.0, 0.0),
        ranges: Vec::new(),
        pages: Vec::new(),
    };
    let Some(sh) = sh else {
        return out;
    };

    let ink = collect_ink(wb, sheet, opts.measure);
    let used = ink.iter().copied().reduce(|a, b| a.union(&b));

    // What to print: the selection, else the print area, else everything
    // from A1 to the last cell that leaves a mark.
    let automatic = opts.selection.is_none() && (setup.print_area.is_empty() || opts.ignore_print_area);
    let mut rects: Vec<Rect> = match opts.selection {
        Some(selection) => selection.to_vec(),
        None if automatic => used.map(|u| Rect::new(0, 0, u.r1, u.c1)).into_iter().collect(),
        None => setup.print_area.clone(),
    };
    // Whole rows and columns print as far as the sheet is used.
    for r in &mut rects {
        if r.is_whole_cols() {
            r.r1 = used.map_or(r.r0, |u| u.r1.max(r.r0));
        }
        if r.is_whole_rows() {
            r.c1 = used.map_or(r.c0, |u| u.c1.max(r.c0));
        }
    }
    if rects.is_empty() {
        return out;
    }

    if setup.headings {
        let last_row = rects.iter().map(|r| r.r1).max().unwrap_or(0);
        out.heading_size = (heading_width(last_row), DEFAULT_ROW_HEIGHT as f64);
    }
    let axes: Vec<(Sizes, Sizes)> = rects
        .iter()
        .map(|r| (Sizes::new(sh, false, r.c0, r.c1), Sizes::new(sh, true, r.r0, r.r1)))
        .collect();
    let title_cols_px = setup
        .title_cols
        .map_or(0.0, |(a, b)| span_px(sh, BreakAxis::Cols, a, b));
    let title_rows_px = setup
        .title_rows
        .map_or(0.0, |(a, b)| span_px(sh, BreakAxis::Rows, a, b));
    // "Fit to" ignores manual page breaks, as Excel does.
    let (manual_cols, manual_rows) = if setup.fit_to_page {
        (None, None)
    } else {
        (Some(&setup.col_breaks), Some(&setup.row_breaks))
    };
    let cut = |percent: u16| -> Vec<PrintRange> {
        let unit = PX_TO_PT * percent as f64 / 100.0;
        let across = printable.2 / unit - out.heading_size.0;
        let down = printable.3 / unit - out.heading_size.1;
        rects
            .iter()
            .zip(&axes)
            .map(|(rect, (cols, rows))| {
                let (col_bands, col_breaks) = bands(cols, across, setup.title_cols, title_cols_px, manual_cols);
                let (row_bands, row_breaks) = bands(rows, down, setup.title_rows, title_rows_px, manual_rows);
                PrintRange {
                    rect: *rect,
                    col_bands,
                    row_bands,
                    col_breaks,
                    row_breaks,
                }
            })
            .collect()
    };

    if setup.fit_to_page {
        // The largest whole percentage, never above 100, at which every
        // range is at most that many pages wide and tall.
        let fits = |percent: u16| -> bool {
            cut(percent).iter().all(|r| {
                (setup.fit_width == 0 || r.col_bands.len() <= setup.fit_width as usize)
                    && (setup.fit_height == 0 || r.row_bands.len() <= setup.fit_height as usize)
            })
        };
        let (mut lo, mut hi) = (MIN_SCALE, 100);
        if fits(hi) {
            lo = hi;
        }
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        out.scale_percent = lo;
    }
    out.ranges = cut(out.scale_percent);

    let unit = PX_TO_PT * out.scale_percent as f64 / 100.0;
    let first_number = setup.first_page_number.or(opts.first_page_number).unwrap_or(1);
    let mut any_ink = false;
    for (ri, range) in out.ranges.iter().enumerate() {
        let (nc, nr) = (range.col_bands.len(), range.row_bands.len());
        if nc == 0 || nr == 0 {
            continue;
        }
        // Which pages of the grid have something on them.
        let (cols, rows) = &axes[ri];
        let mut inked = vec![false; nc * nr];
        for mark in &ink {
            let Some(x) = mark.intersection(&range.rect) else {
                continue;
            };
            if x.is_single() && (cols.get(x.c0) <= 0.0 || rows.get(x.r0) <= 0.0) {
                continue;
            }
            let (Some((c0, c1)), Some((r0, r1))) = (
                bands_touching(&range.col_bands, x.c0, x.c1),
                bands_touching(&range.row_bands, x.r0, x.r1),
            ) else {
                continue;
            };
            for cb in c0..=c1 {
                for rb in r0..=r1 {
                    inked[cb * nr + rb] = true;
                }
            }
        }
        let mut order: Vec<(usize, usize)> = Vec::with_capacity(nc * nr);
        match setup.page_order {
            PageOrder::DownThenOver => {
                for cb in 0..nc {
                    for rb in 0..nr {
                        order.push((cb, rb));
                    }
                }
            }
            PageOrder::OverThenDown => {
                for rb in 0..nr {
                    for cb in 0..nc {
                        order.push((cb, rb));
                    }
                }
            }
        }
        let last_inked = order.iter().rposition(|(cb, rb)| inked[cb * nr + rb]);
        any_ink |= last_inked.is_some();
        // The used part of a sheet ends where its last mark is: blank
        // pages after it are not printed. A print area or a selection
        // prints as the user drew it.
        if automatic {
            order.truncate(last_inked.map_or(0, |i| i + 1));
        }
        for (cb, rb) in order {
            let cols = range.col_bands[cb];
            let rows = range.row_bands[rb];
            let title_cols = setup.title_cols.filter(|(_, t1)| cols.0 > *t1);
            let title_rows = setup.title_rows.filter(|(_, t1)| rows.0 > *t1);
            let width_px = out.heading_size.0
                + title_cols.map_or(0.0, |_| title_cols_px)
                + span_px(sh, BreakAxis::Cols, cols.0, cols.1);
            let height_px = out.heading_size.1
                + title_rows.map_or(0.0, |_| title_rows_px)
                + span_px(sh, BreakAxis::Rows, rows.0, rows.1);
            let size = (width_px * unit, height_px * unit);
            let centre = |on: bool, room: f64, size: f64| if on { ((room - size) / 2.0).max(0.0) } else { 0.0 };
            let index = out.pages.len();
            out.pages.push(Page {
                index,
                number: first_number + index as i32,
                range: ri,
                col_band: cb,
                row_band: rb,
                rows,
                cols,
                title_rows,
                title_cols,
                origin: (
                    printable.0 + centre(setup.center_horizontally, printable.2, size.0),
                    printable.1 + centre(setup.center_vertically, printable.3, size.1),
                ),
                size,
            });
        }
    }
    // A print area or a selection of nothing but blank cells: there is
    // nothing to print.
    if !any_ink {
        out.pages.clear();
    }
    out
}

/// The whole sheet cut into pages, used or not: what the Page Layout view
/// shows. The pages follow the same rules as the printed ones (paper,
/// margins, scale, manual breaks, room taken by print titles and
/// headings) but start at A1 and go on to the last row and column.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPages {
    /// Paper width and height in points, turned for the orientation.
    pub paper: (f64, f64),
    /// The area inside the margins: left, top, width, height, in points.
    pub printable: (f64, f64, f64, f64),
    pub scale_percent: u16,
    /// The first column of every page across, starting with column A.
    pub col_starts: Vec<u32>,
    /// The first row of every page down, starting with row 1.
    pub row_starts: Vec<u32>,
}

impl SheetPages {
    /// Points on paper per sheet pixel.
    pub fn unit(&self) -> f64 {
        PX_TO_PT * self.scale_percent as f64 / 100.0
    }

    /// Which page across holds a column.
    pub fn col_page(&self, col: u32) -> usize {
        self.col_starts.partition_point(|s| *s <= col).saturating_sub(1)
    }

    /// Which page down holds a row.
    pub fn row_page(&self, row: u32) -> usize {
        self.row_starts.partition_point(|s| *s <= row).saturating_sub(1)
    }

    /// The first and last column of a page across.
    pub fn cols(&self, page: usize) -> (u32, u32) {
        span_of(&self.col_starts, page, MAX_COLS)
    }

    /// The first and last row of a page down.
    pub fn rows(&self, page: usize) -> (u32, u32) {
        span_of(&self.row_starts, page, MAX_ROWS)
    }
}

fn span_of(starts: &[u32], page: usize, count: u32) -> (u32, u32) {
    let page = page.min(starts.len().saturating_sub(1));
    let first = starts.get(page).copied().unwrap_or(0);
    let last = starts.get(page + 1).map_or(count - 1, |next| next - 1);
    (first, last)
}

/// The first track of every page along one axis of the whole sheet.
fn axis_starts(
    sheet: &Sheet,
    axis: BreakAxis,
    available: f64,
    title: Option<(u32, u32)>,
    manual: Option<&BTreeSet<u32>>,
) -> Vec<u32> {
    let rows = axis == BreakAxis::Rows;
    let (default, count, sizes, hidden) = if rows {
        (DEFAULT_ROW_HEIGHT, MAX_ROWS, &sheet.row_heights, &sheet.hidden_rows)
    } else {
        (DEFAULT_COL_WIDTH, MAX_COLS, &sheet.col_widths, &sheet.hidden_cols)
    };
    // Past this track every one has the default size, none is hidden and
    // no break is set: the pages repeat and need not be walked.
    let mut limit = 0u32;
    let mut reach = |index: Option<u32>| limit = limit.max(index.unwrap_or(0));
    reach(sizes.keys().next_back().copied());
    reach(hidden.iter().next_back().copied());
    reach(manual.and_then(|m| m.iter().next_back().copied()));
    reach(title.map(|(_, last)| last));
    if rows {
        reach(sheet.filtered_rows.iter().next_back().copied());
    }
    let limit = limit.min(count - 1);
    let title_px = title.map_or(0.0, |(a, b)| span_px(sheet, axis, a, b));
    let (bands, breaks) = bands(&Sizes::new(sheet, rows, 0, limit), available, title, title_px, manual);
    let mut starts = vec![0u32];
    starts.extend(breaks.iter().map(|b| b.at));
    let room = |start: u32| -> f64 {
        if title.is_some_and(|(_, last)| start > last) {
            available - title_px
        } else {
            available
        }
    };
    // The page being filled when the walk stopped takes default tracks
    // until it is full, then every page holds the same number of them.
    let open = bands.last().map_or(0, |b| b.0);
    let used = span_px(sheet, axis, open, limit);
    let default = default as f64;
    let more = (((room(open) - used) + EPS) / default).floor().max(0.0) as u64;
    let per_page = ((room(limit + 1) + EPS) / default).floor().max(1.0) as u64;
    let mut next = limit as u64 + 1 + more;
    while next < count as u64 {
        starts.push(next as u32);
        next += per_page;
    }
    starts
}

/// Cut the whole sheet into pages for the Page Layout view. `pagination`
/// is the sheet's own (it gives the paper and the scale, which "Fit to"
/// works out from what prints).
pub fn sheet_pages(wb: &Workbook, sheet: usize, pagination: &Pagination) -> SheetPages {
    let mut out = SheetPages {
        paper: pagination.paper,
        printable: pagination.printable,
        scale_percent: pagination.scale_percent,
        col_starts: vec![0],
        row_starts: vec![0],
    };
    let Some(sh) = wb.sheets.get(sheet) else {
        return out;
    };
    let setup = &sh.page;
    let unit = out.unit();
    let across = pagination.printable.2 / unit - pagination.heading_size.0;
    let down = pagination.printable.3 / unit - pagination.heading_size.1;
    // "Fit to" ignores manual page breaks, as in `paginate`.
    let (manual_cols, manual_rows) = if setup.fit_to_page {
        (None, None)
    } else {
        (Some(&setup.col_breaks), Some(&setup.row_breaks))
    };
    out.col_starts = axis_starts(sh, BreakAxis::Cols, across, setup.title_cols, manual_cols);
    out.row_starts = axis_starts(sh, BreakAxis::Rows, down, setup.title_rows, manual_rows);
    out
}

impl Pagination {
    /// The scale as a factor: 1.0 = 100 %.
    pub fn scale(&self) -> f64 {
        self.scale_percent as f64 / 100.0
    }

    /// Points on paper per sheet pixel.
    pub fn unit(&self) -> f64 {
        PX_TO_PT * self.scale()
    }

    /// The number of the last page: with a first page number of 5, three
    /// pages are numbered 5 to 7.
    pub fn last_number(&self) -> Option<i32> {
        self.pages.last().map(|p| p.number)
    }

    /// The page printing a cell in its own rows and columns (not as a
    /// repeated title).
    pub fn page_of(&self, row: u32, col: u32) -> Option<&Page> {
        self.pages
            .iter()
            .find(|p| row >= p.rows.0 && row <= p.rows.1 && col >= p.cols.0 && col <= p.cols.1)
    }

    /// Where the rows and columns of a page fall on the paper.
    pub fn layout(&self, sheet: &Sheet, page: &Page) -> PageLayout {
        let unit = self.unit();
        let heading = (self.heading_size.0 * unit, self.heading_size.1 * unit);
        // Repeated titles first, then the page's own rows or columns.
        let place = |rows: bool, start: f64, title: Option<(u32, u32)>, own: (u32, u32)| -> (Vec<Span>, usize, f64) {
            let mut spans = Vec::new();
            let mut pos = start;
            let mut add = |(first, last): (u32, u32), spans: &mut Vec<Span>| {
                for index in first..=last {
                    let px = if rows {
                        visible_row_height(sheet, index)
                    } else {
                        visible_col_width(sheet, index)
                    };
                    if px > 0.0 {
                        spans.push(Span {
                            index,
                            pos,
                            size: px * unit,
                        });
                        pos += px * unit;
                    }
                }
                pos
            };
            let body_start = title.map_or(start, |t| add(t, &mut spans));
            let titles = spans.len();
            add(own, &mut spans);
            (spans, titles, body_start)
        };
        let (cols, title_cols, body_x) = place(false, page.origin.0 + heading.0, page.title_cols, page.cols);
        let (rows, title_rows, body_y) = place(true, page.origin.1 + heading.1, page.title_rows, page.rows);
        let right = page.origin.0 + page.size.0;
        let bottom = page.origin.1 + page.size.1;
        PageLayout {
            unit,
            block: (page.origin.0, page.origin.1, page.size.0, page.size.1),
            heading,
            cols,
            rows,
            title_cols,
            title_rows,
            body: (body_x, body_y, right - body_x, bottom - body_y),
            body_px: (
                axis_offset(sheet, false, page.cols.0),
                axis_offset(sheet, true, page.rows.0),
            ),
        }
    }
}
