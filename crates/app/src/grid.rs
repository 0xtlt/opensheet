//! The worksheet grid: geometry, scene building and painting.
//!
//! The grid is painted directly (quads + shaped text) rather than built from
//! elements, so a full screen of cells costs one pass over the visible range.
//! Features add their own drawing through the hooks of `features/mod.rs`.

use crate::app::ExcelApp;
use crate::features::{self, PaintLayer, Painter, Zone};
use crate::theme::{self, c, ca};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Font, FontStyle, FontWeight, Hsla, PaintQuad, Pixels,
    Point, ShapedLine, SharedString, StrikethroughStyle, TextAlign, TextRun, UnderlineStyle, Window,
    WindowTextSystem, WrappedLine, fill, point, px, quad, size,
};
use opensheet_engine::format;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, col_to_letters};
use opensheet_engine::style::{Border, BorderStyle as CellBorderStyle, HAlign, Style, Underline, VAlign};
use opensheet_engine::workbook::{Content, Sheet};
use opensheet_engine::features::CellDisplay;
use opensheet_engine::{Rect, StyleId, Value, Workbook};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

// ---------------------------------------------------------------- selection

#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    /// The active cell (where typing goes).
    pub active: (u32, u32),
    /// The fixed corner while extending with Shift / dragging.
    pub anchor: (u32, u32),
    /// Selected rectangles; the last one is the current range and contains
    /// the active cell.
    pub ranges: Vec<Rect>,
}

impl Selection {
    pub fn single(row: u32, col: u32) -> Selection {
        Selection {
            active: (row, col),
            anchor: (row, col),
            ranges: vec![Rect::cell(row, col)],
        }
    }

    pub fn current(&self) -> Rect {
        *self.ranges.last().unwrap()
    }

    pub fn set_current(&mut self, rect: Rect) {
        *self.ranges.last_mut().unwrap() = rect;
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.ranges.iter().any(|r| r.contains(row, col))
    }

    pub fn is_single_cell(&self) -> bool {
        self.ranges.len() == 1 && self.ranges[0].is_single()
    }
}

// --------------------------------------------------------------------- axis

#[derive(Clone, Debug)]
struct AxisEntry {
    index: u32,
    size: f32,
    start: f64,
}

/// Positions of rows or columns along one axis, in zoomed pixels. Most
/// tracks have the default size; only customised ones are stored.
#[derive(Clone, Debug)]
pub struct Axis {
    default: f32,
    entries: Vec<AxisEntry>,
    count: u32,
    /// Blank space before some tracks (between the pages of the Page
    /// Layout view): (track, space before it and all earlier tracks).
    gaps: Vec<(u32, f64)>,
}

impl Axis {
    /// `sizes` must be sorted by index and hold absolute sizes in pixels.
    pub fn new(default: f32, count: u32, sizes: impl Iterator<Item = (u32, f32)>) -> Axis {
        let mut entries: Vec<AxisEntry> = Vec::new();
        let mut delta = 0.0f64;
        for (index, size) in sizes {
            let start = index as f64 * default as f64 + delta;
            delta += (size - default) as f64;
            entries.push(AxisEntry { index, size, start });
        }
        Axis {
            default,
            entries,
            count,
            gaps: Vec::new(),
        }
    }

    /// Leave blank space before some tracks: (track, pixels), sorted by
    /// track. Positions inside a gap belong to the track that follows it.
    #[allow(dead_code)] // for `Feature::adjust_layout` of the Page Layout view
    pub fn set_gaps(&mut self, gaps: &[(u32, f32)]) {
        let mut total = 0.0f64;
        self.gaps = gaps
            .iter()
            .map(|(index, space)| {
                total += space.max(0.0) as f64;
                (*index, total)
            })
            .collect();
    }

    fn gap_before(&self, index: u32) -> f64 {
        let k = self.gaps.partition_point(|(i, _)| *i <= index);
        if k == 0 { 0.0 } else { self.gaps[k - 1].1 }
    }

    pub fn start(&self, index: u32) -> f64 {
        self.packed_start(index) + self.gap_before(index)
    }

    /// Start of a track when the gaps are left out.
    fn packed_start(&self, index: u32) -> f64 {
        let k = self.entries.partition_point(|e| e.index < index);
        let delta = if k == 0 {
            0.0
        } else {
            let e = &self.entries[k - 1];
            e.start + e.size as f64 - (e.index as f64 + 1.0) * self.default as f64
        };
        index as f64 * self.default as f64 + delta
    }

    pub fn size(&self, index: u32) -> f32 {
        match self.entries.binary_search_by_key(&index, |e| e.index) {
            Ok(i) => self.entries[i].size,
            Err(_) => self.default,
        }
    }

    pub fn end(&self, index: u32) -> f64 {
        self.start(index) + self.size(index) as f64
    }

    pub fn total(&self) -> f64 {
        self.start(self.count)
    }

    /// The track containing position `pos` (clamped to the axis).
    pub fn index_at(&self, pos: f64) -> u32 {
        if pos <= 0.0 {
            return 0;
        }
        // Take out the gaps lying before the position.
        let mut pos = pos;
        let mut before = 0.0f64;
        for (index, total) in &self.gaps {
            let track = self.packed_start(*index);
            if pos >= track + total {
                before = *total;
            } else {
                if pos >= track + before {
                    return (*index).min(self.count - 1);
                }
                break;
            }
        }
        pos -= before;
        let k = self.entries.partition_point(|e| e.start <= pos);
        let idx = if k == 0 {
            (pos / self.default as f64).floor() as u64
        } else {
            let e = &self.entries[k - 1];
            let end = e.start + e.size as f64;
            if pos < end {
                e.index as u64
            } else {
                e.index as u64 + 1 + ((pos - end) / self.default as f64).floor() as u64
            }
        };
        idx.min(self.count as u64 - 1) as u32
    }
}

/// `hidden`: the tracks shown with no size (for rows: hidden by hand or by
/// a filter).
fn build_axis(
    default: f32,
    count: u32,
    zoom: f32,
    custom: &std::collections::BTreeMap<u32, f32>,
    hidden: &std::collections::BTreeSet<u32>,
) -> Axis {
    let d = (default * zoom).round().max(1.0);
    let mut merged: Vec<(u32, f32)> = custom.iter().map(|(i, s)| (*i, (s * zoom).round())).collect();
    if !hidden.is_empty() {
        let mut map: std::collections::BTreeMap<u32, f32> = merged.into_iter().collect();
        for h in hidden {
            map.insert(*h, 0.0);
        }
        merged = map.into_iter().collect();
    }
    Axis::new(d, count, merged.into_iter())
}

// ------------------------------------------------------------------- layout

#[derive(Clone, Debug)]
pub struct GridLayout {
    /// Element bounds in window coordinates.
    pub bounds: Bounds<Pixels>,
    pub cols: Axis,
    pub rows: Axis,
    pub header_w: f32,
    pub header_h: f32,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    /// Pixel size of the frozen columns / rows.
    pub frozen_w: f32,
    pub frozen_h: f32,
    pub zoom: f32,
}

impl GridLayout {
    pub fn new(sheet: &Sheet, bounds: Bounds<Pixels>, zoom: f32, scroll_x: f64, scroll_y: f64) -> GridLayout {
        // The size of the tracks nobody resized is asked from the sheet (a
        // sheet may have its own default): from the last track that has no
        // size of its own and is shown.
        let not_shown = sheet.rows_not_shown();
        let plain_col = (0..MAX_COLS)
            .rev()
            .find(|c| !sheet.col_widths.contains_key(c) && !sheet.hidden_cols.contains(c))
            .unwrap_or(MAX_COLS - 1);
        let plain_row = (0..MAX_ROWS)
            .rev()
            .find(|r| !sheet.row_heights.contains_key(r) && !not_shown.contains(r))
            .unwrap_or(MAX_ROWS - 1);
        let cols = build_axis(sheet.col_width(plain_col), MAX_COLS, zoom, &sheet.col_widths, &sheet.hidden_cols);
        // Rows a filter hides take no room either.
        let rows = build_axis(sheet.row_height(plain_row), MAX_ROWS, zoom, &sheet.row_heights, &not_shown);
        let frozen_w = cols.start(sheet.frozen_cols) as f32;
        let frozen_h = rows.start(sheet.frozen_rows) as f32;
        let (header_w, header_h) = if sheet.show_headers {
            let view_h = f32::from(bounds.size.height);
            // The row header grows with the number of digits shown.
            let last_row = rows.index_at(scroll_y + view_h as f64) + 1;
            let digits = last_row.to_string().len().max(2) as f32;
            (
                ((digits * 7.5 + 12.0) * zoom).round().max(26.0),
                (theme::COL_HEADER_H * zoom).round(),
            )
        } else {
            (0.0, 0.0)
        };
        let mut layout = GridLayout {
            bounds,
            cols,
            rows,
            header_w,
            header_h,
            scroll_x,
            scroll_y,
            frozen_rows: sheet.frozen_rows,
            frozen_cols: sheet.frozen_cols,
            frozen_w,
            frozen_h,
            zoom,
        };
        layout.clamp_scroll();
        layout
    }

    /// Bring the scroll offsets back inside the sheet (after the axes
    /// changed).
    pub fn clamp_scroll(&mut self) {
        self.scroll_x = self.scroll_x.clamp(0.0, self.max_scroll_x());
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll_y());
    }

    pub fn width(&self) -> f32 {
        f32::from(self.bounds.size.width)
    }

    pub fn height(&self) -> f32 {
        f32::from(self.bounds.size.height)
    }

    /// Width / height of the cell area (without headers).
    pub fn view_w(&self) -> f32 {
        (self.width() - self.header_w).max(0.0)
    }

    pub fn view_h(&self) -> f32 {
        (self.height() - self.header_h).max(0.0)
    }

    pub fn max_scroll_x(&self) -> f64 {
        (self.cols.total() - self.view_w() as f64).max(0.0)
    }

    pub fn max_scroll_y(&self) -> f64 {
        (self.rows.total() - self.view_h() as f64).max(0.0)
    }

    /// Left edge of a column in element coordinates (may be off-screen).
    pub fn col_x(&self, col: u32) -> f32 {
        let start = self.cols.start(col);
        if col < self.frozen_cols {
            self.header_w + start as f32
        } else {
            self.header_w + (start - self.scroll_x) as f32
        }
    }

    pub fn row_y(&self, row: u32) -> f32 {
        let start = self.rows.start(row);
        if row < self.frozen_rows {
            self.header_h + start as f32
        } else {
            self.header_h + (start - self.scroll_y) as f32
        }
    }

    pub fn col_w(&self, col: u32) -> f32 {
        self.cols.size(col)
    }

    pub fn row_h(&self, row: u32) -> f32 {
        self.rows.size(row)
    }

    /// Column under an x position in element coordinates.
    pub fn col_at(&self, x: f32) -> u32 {
        let xr = (x - self.header_w) as f64;
        if xr < self.frozen_w as f64 {
            self.cols.index_at(xr.max(0.0))
        } else {
            self.cols.index_at(xr + self.scroll_x)
        }
    }

    pub fn row_at(&self, y: f32) -> u32 {
        let yr = (y - self.header_h) as f64;
        if yr < self.frozen_h as f64 {
            self.rows.index_at(yr.max(0.0))
        } else {
            self.rows.index_at(yr + self.scroll_y)
        }
    }

    /// Pixel box (x0, y0, x1, y1) of a cell rectangle, element coordinates.
    /// Clamped to a sane range so huge ranges stay representable.
    pub fn rect_px(&self, rect: Rect) -> (f32, f32, f32, f32) {
        let x0 = self.col_x(rect.c0);
        let y0 = self.row_y(rect.r0);
        let x1 = if rect.c1 + 1 >= MAX_COLS {
            1.0e7
        } else {
            self.col_x(rect.c1) + self.col_w(rect.c1)
        };
        let y1 = if rect.r1 + 1 >= MAX_ROWS {
            1.0e7
        } else {
            self.row_y(rect.r1) + self.row_h(rect.r1)
        };
        // A range straddling the frozen boundary keeps its frozen start.
        (x0.max(-1.0e6), y0.max(-1.0e6), x1.min(1.0e7), y1.min(1.0e7))
    }

    pub fn to_window(&self, x: f32, y: f32) -> Point<Pixels> {
        point(self.bounds.origin.x + px(x), self.bounds.origin.y + px(y))
    }

    pub fn local(&self, p: Point<Pixels>) -> (f32, f32) {
        (
            f32::from(p.x - self.bounds.origin.x),
            f32::from(p.y - self.bounds.origin.y),
        )
    }

    pub fn bounds_px(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds<Pixels> {
        Bounds::new(self.to_window(x0, y0), size(px((x1 - x0).max(0.0)), px((y1 - y0).max(0.0))))
    }

    /// Visible scrolled columns (those after the frozen ones).
    pub fn scrolled_cols(&self) -> Range<u32> {
        let first = self.cols.index_at(self.frozen_w as f64 + self.scroll_x).max(self.frozen_cols);
        let last = self.cols.index_at(self.view_w() as f64 + self.scroll_x);
        first..(last + 1).min(MAX_COLS)
    }

    pub fn scrolled_rows(&self) -> Range<u32> {
        let first = self.rows.index_at(self.frozen_h as f64 + self.scroll_y).max(self.frozen_rows);
        let last = self.rows.index_at(self.view_h() as f64 + self.scroll_y);
        first..(last + 1).min(MAX_ROWS)
    }

    /// Number of fully visible scrolled rows (for Page Up / Down).
    pub fn page_rows(&self) -> u32 {
        let r = self.scrolled_rows();
        (r.end - r.start).saturating_sub(2).max(1)
    }

    /// Scroll offsets that bring a cell fully into view.
    pub fn scroll_to_show(&self, row: u32, col: u32) -> (f64, f64) {
        let mut sx = self.scroll_x;
        let mut sy = self.scroll_y;
        if col >= self.frozen_cols {
            let start = self.cols.start(col);
            let end = start + self.cols.size(col) as f64;
            let view_start = self.frozen_w as f64 + sx;
            let view_end = (self.view_w() - theme::SCROLLBAR) as f64 + sx;
            if start < view_start {
                sx = start - self.frozen_w as f64;
            } else if end > view_end {
                sx = (end - (self.view_w() - theme::SCROLLBAR) as f64).min(start - self.frozen_w as f64);
            }
        }
        if row >= self.frozen_rows {
            let start = self.rows.start(row);
            let end = start + self.rows.size(row) as f64;
            let view_start = self.frozen_h as f64 + sy;
            let view_end = (self.view_h() - theme::SCROLLBAR) as f64 + sy;
            if start < view_start {
                sy = start - self.frozen_h as f64;
            } else if end > view_end {
                sy = (end - (self.view_h() - theme::SCROLLBAR) as f64).min(start - self.frozen_h as f64);
            }
        }
        (sx.clamp(0.0, self.max_scroll_x()), sy.clamp(0.0, self.max_scroll_y()))
    }

    /// Vertical scrollbar: (track_x, track_y, track_len, thumb_pos, thumb_len)
    /// given the scrollable content length in pixels.
    pub fn v_scrollbar(&self, content: f64) -> (f32, f32, f32, f32, f32) {
        let track_x = self.width() - theme::SCROLLBAR;
        let track_y = self.header_h;
        let track_len = (self.view_h() - theme::SCROLLBAR).max(1.0);
        let view = self.view_h() as f64;
        let range = content.max(self.scroll_y + view).max(view + 1.0);
        let thumb_len = ((view / range) * track_len as f64).max(28.0).min(track_len as f64) as f32;
        let max_scroll = (range - view).max(1.0);
        let thumb_pos = ((self.scroll_y / max_scroll) * (track_len - thumb_len) as f64) as f32;
        (track_x, track_y, track_len, thumb_pos, thumb_len)
    }

    pub fn h_scrollbar(&self, content: f64) -> (f32, f32, f32, f32, f32) {
        let track_x = self.header_w;
        let track_y = self.height() - theme::SCROLLBAR;
        let track_len = (self.view_w() - theme::SCROLLBAR).max(1.0);
        let view = self.view_w() as f64;
        let range = content.max(self.scroll_x + view).max(view + 1.0);
        let thumb_len = ((view / range) * track_len as f64).max(28.0).min(track_len as f64) as f32;
        let max_scroll = (range - view).max(1.0);
        let thumb_pos = ((self.scroll_x / max_scroll) * (track_len - thumb_len) as f64) as f32;
        (track_x, track_y, track_len, thumb_pos, thumb_len)
    }
}

// -------------------------------------------------------------------- fonts

/// Maps the font names stored in cells to families installed on this
/// machine (Office fonts are often missing on macOS).
pub struct FontResolver {
    available: HashSet<String>,
    cache: RefCell<HashMap<Arc<str>, SharedString>>,
    text_system: Arc<gpui::TextSystem>,
    pub names: Vec<String>,
}

impl FontResolver {
    pub fn new(text_system: Arc<gpui::TextSystem>) -> FontResolver {
        let mut names = text_system.all_font_names();
        names.sort();
        names.dedup();
        names.retain(|n| !n.starts_with('.'));
        FontResolver {
            available: names.iter().map(|n| n.to_lowercase()).collect(),
            cache: RefCell::new(HashMap::new()),
            text_system,
            names,
        }
    }

    /// True when the family can actually be loaded. Some system fonts are
    /// not enumerated (macOS activates them on demand), so probe by name.
    fn usable(&self, family: &str) -> bool {
        if self.available.contains(&family.to_lowercase()) {
            return true;
        }
        // An unknown family resolves to the fallback stack's font.
        let fallback = self.text_system.resolve_font(&gpui::font("opensheet-missing-font"));
        self.text_system.resolve_font(&gpui::font(family.to_string())) != fallback
    }

    pub fn resolve(&self, name: &Arc<str>) -> SharedString {
        if let Some(s) = self.cache.borrow().get(name) {
            return s.clone();
        }
        let lower = name.to_lowercase();
        let candidates: &[&str] = match lower.as_str() {
            // Seravek ships with macOS and is the closest match in digit
            // width and overall colour to Aptos Narrow / Calibri. Condensed
            // faces (Arial Narrow) look squashed next to the real thing.
            "aptos narrow" => &["Aptos", "Calibri", "Carlito", "Seravek", "Helvetica Neue", "Arial"],
            "aptos" | "aptos display" => &["Calibri", "Carlito", "Seravek", "Helvetica Neue", "Arial"],
            "calibri" | "calibri light" => &["Carlito", "Seravek", "Helvetica Neue", "Arial"],
            "cambria" => &["Georgia", "Times New Roman"],
            "segoe ui" => &["Helvetica Neue", "Arial"],
            "consolas" => &["Menlo", "Courier New"],
            _ => &[],
        };
        let mut chosen: Option<String> = None;
        if self.usable(name) {
            chosen = Some(name.to_string());
        } else {
            for cand in candidates {
                if self.usable(cand) {
                    chosen = Some(cand.to_string());
                    break;
                }
            }
        }
        let family: SharedString = chosen.unwrap_or_else(|| "Helvetica Neue".to_string()).into();
        if std::env::var("OPENSHEET_DEBUG").is_ok() {
            eprintln!("font {name:?} -> {family:?}");
        }
        self.cache.borrow_mut().insert(name.clone(), family.clone());
        family
    }
}

/// Cell font size in pixels at 100 % (points → CSS pixels).
pub fn pt_to_px(pt: f32) -> f32 {
    pt * 96.0 / 72.0
}

pub fn cell_font(style: &Style, fonts: &FontResolver) -> Font {
    Font {
        family: fonts.resolve(&style.font.name),
        features: Default::default(),
        fallbacks: None,
        weight: if style.font.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if style.font.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

// -------------------------------------------------------------------- scene

pub enum Draw {
    Quad(PaintQuad),
    Path(gpui::Path<Pixels>, Hsla),
    Text {
        line: ShapedLine,
        origin: Point<Pixels>,
        line_height: Pixels,
        clip: Bounds<Pixels>,
    },
    Wrapped {
        lines: Vec<WrappedLine>,
        origin: Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        bounds: Bounds<Pixels>,
    },
    /// A line of text turned by `turn` radians (clockwise) around `pivot`:
    /// the title of a vertical chart axis. `path` names the text as an
    /// asset (see `chart_view::text_asset`), `bounds` is its box before it
    /// is turned.
    Turned {
        path: SharedString,
        bounds: Bounds<Pixels>,
        pivot: Point<Pixels>,
        turn: f32,
        color: Hsla,
        clip: Bounds<Pixels>,
    },
}

pub struct Layer {
    pub clip: Bounds<Pixels>,
    pub items: Vec<Draw>,
}

/// What `build_scene` returns: what to paint, and the areas features made
/// sensitive to the mouse.
pub struct Scene {
    pub layers: Vec<Layer>,
    pub zones: Vec<Zone>,
}

/// Everything the scene builder needs to know besides the workbook.
pub struct SceneInput<'a> {
    /// For the feature hooks.
    pub app: &'a ExcelApp,
    pub wb: &'a Workbook,
    pub sheet: usize,
    pub selection: &'a Selection,
    pub fonts: &'a FontResolver,
    /// Dashed outline around copied cells.
    pub marquee: Option<Rect>,
    /// Outline shown while dragging the fill handle.
    pub fill_preview: Option<Rect>,
    /// Coloured ranges of the formula being edited.
    pub ref_highlights: &'a [(Rect, u32)],
    /// Show the fill handle on the current range.
    pub show_fill_handle: bool,
    /// Scrollable content size in pixels (drives the scrollbars).
    pub content_w: f64,
    pub content_h: f64,
    /// 0 = none, 1 = vertical thumb hot, 2 = horizontal thumb hot.
    pub hot_scrollbar: u8,
    /// Device pixels per logical pixel.
    pub scale: f32,
}

struct PaneCtx<'a> {
    input: &'a SceneInput<'a>,
    layout: &'a GridLayout,
    sheet: &'a Sheet,
    ts: &'a WindowTextSystem,
    hair: f32,
    /// Tables, pivot tables or conditional formats restyle cells of this
    /// sheet: every visible cell goes through `Workbook::display`.
    layered: bool,
    /// The sheet shows formulas instead of their results.
    formulas: bool,
    digit_w: RefCell<HashMap<(SharedString, u32, bool), f32>>,
    zones: RefCell<Vec<Zone>>,
}

fn solid(layout: &GridLayout, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) -> Draw {
    Draw::Quad(fill(layout.bounds_px(x0, y0, x1, y1), color))
}

fn border_width(style: CellBorderStyle) -> f32 {
    match style {
        CellBorderStyle::None => 0.0,
        CellBorderStyle::Hair | CellBorderStyle::Thin | CellBorderStyle::Dashed | CellBorderStyle::Dotted => 1.0,
        CellBorderStyle::Medium => 2.0,
        CellBorderStyle::Thick | CellBorderStyle::Double => 3.0,
    }
}

/// One border edge as quads. `horizontal` edges run along x from `a0` to
/// `a1` centred on `pos`; vertical edges run along y.
fn border_edge(layout: &GridLayout, out: &mut Vec<Draw>, b: Border, horizontal: bool, pos: f32, a0: f32, a1: f32) {
    if b.is_none() {
        return;
    }
    let color = b.color.map(theme::engine_color).unwrap_or(c(0x000000));
    let w = border_width(b.style);
    let seg = |p0: f32, p1: f32, t0: f32, t1: f32, out: &mut Vec<Draw>| {
        if horizontal {
            out.push(solid(layout, p0, t0, p1, t1, color));
        } else {
            out.push(solid(layout, t0, p0, t1, p1, color));
        }
    };
    let t0 = pos - (w / 2.0).floor();
    match b.style {
        CellBorderStyle::Double => {
            seg(a0 - 1.0, a1 + 1.0, pos - 1.0, pos, out);
            seg(a0 - 1.0, a1 + 1.0, pos + 1.0, pos + 2.0, out);
        }
        CellBorderStyle::Dashed | CellBorderStyle::Dotted => {
            let (on, off) = if b.style == CellBorderStyle::Dashed {
                (4.0, 2.0)
            } else {
                (1.0, 1.0)
            };
            let mut p = a0;
            while p < a1 {
                seg(p, (p + on).min(a1), t0, t0 + w, out);
                p += on + off;
            }
        }
        CellBorderStyle::Hair => {
            let mut p = a0;
            while p < a1 {
                seg(p, (p + 1.0).min(a1), t0, t0 + w, out);
                p += 2.0;
            }
        }
        _ => {
            let ext = (w / 2.0).floor();
            seg(a0 - ext, a1 + (w - ext - 1.0).max(0.0), t0, t0 + w, out);
        }
    }
}

impl<'a> PaneCtx<'a> {
    fn digit_width(&self, font: &Font, size: f32) -> f32 {
        let key = (font.family.clone(), (size * 8.0) as u32, font.weight == FontWeight::BOLD);
        if let Some(w) = self.digit_w.borrow().get(&key) {
            return *w;
        }
        let run = TextRun {
            len: 1,
            font: font.clone(),
            color: c(0),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let w = f32::from(self.ts.shape_line("0".into(), px(size), &[run], None).width).max(1.0);
        self.digit_w.borrow_mut().insert(key, w);
        w
    }

    fn shape(&self, text: &str, font: &Font, size: f32, color: Hsla, style: &Style) -> ShapedLine {
        let text: SharedString = if text.contains(['\n', '\r']) {
            text.replace(['\n', '\r'], " ").into()
        } else {
            SharedString::from(text.to_string())
        };
        let run = TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline: (style.font.underline != Underline::None).then(|| UnderlineStyle {
                color: Some(color),
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: style.font.strike.then(|| StrikethroughStyle {
                color: Some(color),
                thickness: px(1.0),
            }),
        };
        self.ts.shape_line(text, px(size), &[run], None)
    }

    /// What the features draw for one pane and layer.
    fn feature_layer(&self, layer: PaintLayer, visible: Rect, clip: (f32, f32, f32, f32), items: &mut Vec<Draw>) {
        let mut painter = Painter::new(self.input.app, self.layout, self.ts, layer, visible, clip, self.input.scale);
        features::paint(&mut painter);
        items.append(&mut painter.items);
        self.zones.borrow_mut().append(&mut painter.zones);
    }

    /// Build one pane (a block of rows × columns sharing a clip rectangle).
    fn build(&self, rows: Range<u32>, cols: Range<u32>, clip: (f32, f32, f32, f32)) -> Layer {
        let layout = self.layout;
        let sheet = self.sheet;
        let wb = self.input.wb;
        let zoom = layout.zoom;
        let clip_bounds = layout.bounds_px(clip.0, clip.1, clip.2, clip.3);
        let mut items: Vec<Draw> = Vec::new();
        if rows.is_empty() || cols.is_empty() || clip.2 <= clip.0 || clip.3 <= clip.1 {
            return Layer {
                clip: clip_bounds,
                items,
            };
        }

        let xs: Vec<(u32, f32, f32)> = cols
            .clone()
            .map(|col| (col, layout.col_x(col), layout.col_w(col)))
            .filter(|(_, _, w)| *w > 0.0)
            .collect();
        let ys: Vec<(u32, f32, f32)> = rows
            .clone()
            .map(|row| (row, layout.row_y(row), layout.row_h(row)))
            .filter(|(_, _, h)| *h > 0.0)
            .collect();
        let visible = Rect::new(rows.start, cols.start, rows.end - 1, cols.end - 1);

        let mut fills: Vec<Draw> = Vec::new();
        let mut erasers: Vec<Draw> = Vec::new();
        let mut borders: Vec<Draw> = Vec::new();
        let mut texts: Vec<Draw> = Vec::new();

        self.feature_layer(PaintLayer::Under, visible, clip, &mut items);

        // Gridlines.
        if sheet.show_gridlines {
            let line = c(theme::GRIDLINE);
            for (_, y, h) in &ys {
                items.push(solid(layout, clip.0, y + h - self.hair, clip.2, y + h, line));
            }
            for (_, x, w) in &xs {
                items.push(solid(layout, x + w - self.hair, clip.1, x + w, clip.3, line));
            }
        }

        // Merged areas touching this pane.
        let merges: Vec<Rect> = sheet.merges.iter().filter(|m| m.intersects(&visible)).copied().collect();
        let in_merge = |row: u32, col: u32| -> Option<Rect> { merges.iter().find(|m| m.contains(row, col)).copied() };

        let has_inherited = self.layered || !sheet.col_styles.is_empty() || !sheet.row_styles.is_empty();
        let pad = (3.0 * zoom).round().max(1.0);

        // Every cell that needs drawing: (row, col, pixel box, style id).
        let mut cells: Vec<(u32, u32, (f32, f32, f32, f32), StyleId)> = Vec::new();
        for (row, y, h) in &ys {
            if has_inherited {
                for (col, x, w) in &xs {
                    if in_merge(*row, *col).is_some() {
                        continue;
                    }
                    let id = sheet.style_id_at(*row, *col);
                    if self.layered
                        || id != StyleId(0)
                        || sheet.cells.contains_key(&(*row, *col))
                        || !sheet.spills.is_empty()
                    {
                        cells.push((*row, *col, (*x, *y, x + w, y + h), id));
                    }
                }
            } else {
                for ((_, col), cell) in sheet.cells.range((*row, cols.start)..=(*row, cols.end - 1)) {
                    if in_merge(*row, *col).is_some() {
                        continue;
                    }
                    let x = layout.col_x(*col);
                    let w = layout.col_w(*col);
                    if w > 0.0 {
                        cells.push((*row, *col, (x, *y, x + w, y + h), cell.style));
                    }
                }
            }
        }
        // Spilled array values live in cells that do not exist.
        if !has_inherited {
            for sp in &sheet.spills {
                let area = Rect::new(
                    sp.row,
                    sp.col,
                    sp.row + sp.array.rows as u32 - 1,
                    sp.col + sp.array.cols as u32 - 1,
                );
                if let Some(overlap) = area.intersection(&visible) {
                    for row in overlap.r0..=overlap.r1 {
                        for col in overlap.c0..=overlap.c1 {
                            if sheet.cells.contains_key(&(row, col)) || in_merge(row, col).is_some() {
                                continue;
                            }
                            let (x, y) = (layout.col_x(col), layout.row_y(row));
                            let (w, h) = (layout.col_w(col), layout.row_h(row));
                            if w > 0.0 && h > 0.0 {
                                cells.push((row, col, (x, y, x + w, y + h), StyleId(0)));
                            }
                        }
                    }
                }
            }
        }
        for m in &merges {
            let (x0, y0, x1, y1) = layout.rect_px(*m);
            cells.push((m.r0, m.c0, (x0, y0, x1, y1), sheet.style_id_at(m.r0, m.c0)));
            // A merged area hides the gridlines inside it.
            erasers.push(solid(layout, x0, y0, x1 - self.hair, y1 - self.hair, c(theme::GRID_BG)));
        }

        // Text from the nearest non-blank cell left of the pane may overflow
        // into view.
        let mut overflow_sources: Vec<(u32, u32, (f32, f32, f32, f32), StyleId)> = Vec::new();
        if cols.start > 0 {
            for (row, y, h) in &ys {
                if let Some(((_, col), cell)) = sheet.cells.range((*row, 0)..(*row, cols.start)).next_back() {
                    if matches!(cell.value, Value::Text(_)) && in_merge(*row, *col).is_none() {
                        let x = layout.col_x(*col);
                        let w = layout.col_w(*col);
                        // Only when it lies in the same scrolling region.
                        if (*col >= layout.frozen_cols) == (cols.start >= layout.frozen_cols) && w > 0.0 {
                            overflow_sources.push((*row, *col, (x, *y, x + w, y + h), cell.style));
                        }
                    }
                }
            }
        }

        for (pass, list) in [(0, &cells), (1, &overflow_sources)] {
            for (row, col, (x0, y0, x1, y1), style_id) in list.iter() {
                // The cell's own formatting with table styles and
                // conditional formats laid over it.
                let display: Option<CellDisplay> = self.layered.then(|| wb.display(self.input.sheet, *row, *col));
                let style: &Style = match &display {
                    Some(d) => &d.style,
                    None => wb.styles.get(*style_id),
                };
                let merged = in_merge(*row, *col);
                let mut value = sheet.value_at(*row, *col);
                // Show Formulas: formulas as they were typed, values
                // without their number format.
                let mut num_fmt: &str = &style.num_fmt;
                if self.formulas {
                    if let Some(text) = features::misc::shown_formula(wb, self.input.sheet, *row, *col) {
                        value = Value::text(text);
                    }
                    num_fmt = "General";
                }
                let mut hide_text = false;
                let mut inset = (0.0f32, 0.0f32);
                if pass == 0 {
                    if let Some(color) = style.fill {
                        fills.push(solid(
                            layout,
                            x0 - self.hair,
                            y0 - self.hair,
                            *x1,
                            *y1,
                            theme::engine_color(color),
                        ));
                    }
                    if !style.borders.is_none() {
                        let b = &style.borders;
                        border_edge(layout, &mut borders, b.top, true, y0 - 1.0, x0 - 1.0, *x1);
                        border_edge(layout, &mut borders, b.bottom, true, y1 - 1.0, x0 - 1.0, *x1);
                        border_edge(layout, &mut borders, b.left, false, x0 - 1.0, y0 - 1.0, *y1);
                        border_edge(layout, &mut borders, b.right, false, x1 - 1.0, y0 - 1.0, *y1);
                    }
                    if let Some(adornment) = display.as_ref().and_then(|d| d.adornment.as_ref()) {
                        // Data bars and icons: under the text, or beside it.
                        let cell = features::condfmt::CellBox {
                            row: *row,
                            col: *col,
                            rect: (*x0, *y0, *x1, *y1),
                            value: &value,
                            style,
                            layout,
                            ts: self.ts,
                        };
                        let mut paint = features::condfmt::paint_adornment(adornment, &cell);
                        fills.append(&mut paint.under);
                        texts.append(&mut paint.over);
                        // "Show Bar Only" / "Show Icon Only".
                        hide_text = adornment.hide_value;
                        inset = paint.text_inset;
                    }
                }

                if value.is_empty() || hide_text {
                    continue;
                }
                let formatted = format::format_value(&value, num_fmt, wb.locale);
                if formatted.text.is_empty() {
                    continue;
                }
                let font = cell_font(style, self.input.fonts);
                let font_px = (pt_to_px(style.font.size_pt()) * zoom).max(1.0);
                let color = formatted
                    .color
                    .or(style.font.color)
                    .map(theme::engine_color)
                    .unwrap_or(c(0x000000));
                let line_h = (font_px * 1.22).round();
                let (x0, x1) = (&(x0 + inset.0), &(x1 - inset.1));
                let (w, h) = (x1 - x0, y1 - y0);
                let indent = style.indent as f32 * 9.0 * zoom;
                let avail = (w - 2.0 * pad - indent - 1.0).max(0.0);
                let is_number = matches!(value, Value::Number(_));
                let align = match style.h_align {
                    HAlign::Left | HAlign::Fill | HAlign::Justify => TextAlign::Left,
                    HAlign::Center | HAlign::CenterAcross => TextAlign::Center,
                    HAlign::Right => TextAlign::Right,
                    HAlign::General => match value {
                        Value::Number(_) => TextAlign::Right,
                        Value::Bool(_) | Value::Error(_) => TextAlign::Center,
                        _ => TextAlign::Left,
                    },
                };
                let text_y = |lines: f32| match style.v_align {
                    VAlign::Top => y0 + 1.0,
                    VAlign::Center => y0 + ((h - line_h * lines) / 2.0).round(),
                    VAlign::Bottom => y1 - line_h * lines - (1.0 * zoom).round().max(1.0),
                };

                // Wrapped text: several lines inside the cell box.
                if style.wrap && !is_number && pass == 0 {
                    let run = TextRun {
                        len: formatted.text.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    if let Ok(lines) = self.ts.shape_text(
                        formatted.text.clone().into(),
                        px(font_px),
                        &[run],
                        Some(px(avail.max(4.0))),
                        None,
                    ) {
                        let count: usize = lines.iter().map(|l| l.wrap_boundaries().len() + 1).sum();
                        let ty = text_y(count as f32).max(y0 + 1.0);
                        texts.push(Draw::Wrapped {
                            lines: lines.into_iter().collect(),
                            origin: layout.to_window(x0 + pad + indent, ty),
                            line_height: px(line_h),
                            align,
                            bounds: layout.bounds_px(*x0, *y0, x1 - 1.0, y1 - 1.0),
                        });
                    }
                    continue;
                }

                let mut line = self.shape(&formatted.text, &font, font_px, color, style);
                let mut clip_x0 = *x0;
                let mut clip_x1 = x1 - 1.0;
                if f32::from(line.width) > avail {
                    if is_number {
                        // Numbers never spill: lose precision, then show ###.
                        let dw = self.digit_width(&font, font_px);
                        let mut fitted = false;
                        if style.is_general() || self.formulas {
                            if let Value::Number(n) = value {
                                let mut chars = (avail / dw).floor() as usize;
                                while chars >= 1 {
                                    let text = format::format_general(n, wb.locale, chars.min(11));
                                    let candidate = self.shape(&text, &font, font_px, color, style);
                                    if f32::from(candidate.width) <= avail {
                                        line = candidate;
                                        fitted = true;
                                        break;
                                    }
                                    chars -= 1;
                                }
                            }
                        }
                        if !fitted {
                            let hash_w = self
                                .shape("#", &font, font_px, color, style)
                                .width;
                            let n = (avail / f32::from(hash_w).max(1.0)).floor().max(0.0) as usize;
                            line = self.shape(&"#".repeat(n), &font, font_px, color, style);
                        }
                    } else if merged.is_none() && matches!(value, Value::Text(_)) {
                        // Text flows over blank neighbours.
                        let needed = f32::from(line.width) + 2.0 * pad + indent;
                        if matches!(align, TextAlign::Left | TextAlign::Center) {
                            let mut c2 = *col;
                            let mut edge = *x1;
                            while edge - x0 < needed && c2 + 1 < MAX_COLS && c2 < cols.end + 64 {
                                if !sheet.is_blank(*row, c2 + 1) || in_merge(*row, c2 + 1).is_some() {
                                    break;
                                }
                                // Hide the gridline being crossed.
                                let bg = sheet
                                    .cells
                                    .get(&(*row, c2))
                                    .and_then(|cell| wb.styles.get(cell.style).fill)
                                    .map(theme::engine_color)
                                    .unwrap_or(c(theme::GRID_BG));
                                erasers.push(solid(layout, edge - self.hair, *y0, edge, y1 - self.hair, bg));
                                c2 += 1;
                                edge += layout.col_w(c2);
                            }
                            clip_x1 = edge - 1.0;
                        }
                        if matches!(align, TextAlign::Right | TextAlign::Center) && pass == 0 {
                            let mut c2 = *col;
                            let mut edge = *x0;
                            while x1 - edge < needed && c2 > 0 && c2 + 64 > cols.start {
                                if !sheet.is_blank(*row, c2 - 1) || in_merge(*row, c2 - 1).is_some() {
                                    break;
                                }
                                erasers.push(solid(layout, edge - self.hair, *y0, edge, y1 - self.hair, c(theme::GRID_BG)));
                                c2 -= 1;
                                edge -= layout.col_w(c2);
                            }
                            clip_x0 = edge;
                        }
                    }
                } else if pass == 1 {
                    // Fits in its own (off-screen) cell: nothing reaches us.
                    continue;
                }

                let lw = f32::from(line.width);
                let tx = match align {
                    TextAlign::Left => x0 + pad + indent,
                    TextAlign::Right => x1 - pad - indent - lw - 1.0,
                    _ => x0 + ((w - lw) / 2.0).round(),
                };
                texts.push(Draw::Text {
                    line,
                    origin: layout.to_window(tx, text_y(1.0)),
                    line_height: px(line_h),
                    clip: layout.bounds_px(clip_x0, *y0, clip_x1, y1 - 1.0),
                });
            }
        }

        items.append(&mut fills);
        items.append(&mut erasers);
        items.append(&mut borders);
        items.append(&mut texts);

        self.selection_overlay(&mut items, visible);
        self.feature_layer(PaintLayer::Over, visible, clip, &mut items);
        Layer {
            clip: clip_bounds,
            items,
        }
    }

    fn outline(&self, items: &mut Vec<Draw>, rect: Rect, width: f32, color: Hsla, outset: f32) {
        let (x0, y0, x1, y1) = self.layout.rect_px(rect);
        let b = self
            .layout
            .bounds_px(x0 - 1.0 - outset, y0 - 1.0 - outset, x1 + outset, y1 + outset);
        items.push(Draw::Quad(quad(
            b,
            px(0.),
            gpui::transparent_black(),
            px(width),
            color,
            BorderStyle::Solid,
        )));
    }

    fn dashed_outline(&self, items: &mut Vec<Draw>, rect: Rect, color: Hsla) {
        let layout = self.layout;
        let (x0, y0, x1, y1) = layout.rect_px(rect);
        let (x0, y0, x1, y1) = (x0 - 1.0, y0 - 1.0, x1.min(layout.width() + 8.0), y1.min(layout.height() + 8.0));
        let (dash, gap) = (4.0, 3.0);
        let mut x = x0.max(-8.0);
        while x < x1 {
            let e = (x + dash).min(x1);
            items.push(solid(layout, x, y0, e, y0 + 2.0, color));
            items.push(solid(layout, x, y1 - 1.0, e, y1 + 1.0, color));
            x += dash + gap;
        }
        let mut y = y0.max(-8.0);
        while y < y1 {
            let e = (y + dash).min(y1);
            items.push(solid(layout, x0, y, x0 + 2.0, e, color));
            items.push(solid(layout, x1 - 1.0, y, x1 + 1.0, e, color));
            y += dash + gap;
        }
    }

    fn selection_overlay(&self, items: &mut Vec<Draw>, visible: Rect) {
        let layout = self.layout;
        let sel = self.input.selection;
        let accent = c(theme::SELECTION);

        // Reference colours of the formula being edited.
        for (rect, color) in self.input.ref_highlights {
            if !rect.intersects(&visible) {
                continue;
            }
            let (x0, y0, x1, y1) = layout.rect_px(*rect);
            items.push(solid(layout, x0, y0, x1 - 1.0, y1 - 1.0, ca(*color, 0.10)));
            self.outline(items, *rect, 2.0, c(*color), 0.0);
        }

        let active_rect = self
            .sheet
            .merge_at(sel.active.0, sel.active.1)
            .unwrap_or(Rect::cell(sel.active.0, sel.active.1));
        let tint = ca(0x000000, 0.075);
        let multi = sel.ranges.len() > 1;
        for (i, range) in sel.ranges.iter().enumerate() {
            let is_current = i + 1 == sel.ranges.len();
            if let Some(vis) = range.intersection(&visible) {
                let single = *range == active_rect;
                if !single {
                    // Tint everything but the active cell.
                    let hole = if is_current { active_rect.intersection(&vis) } else { None };
                    let mut pieces: Vec<Rect> = Vec::new();
                    match hole {
                        Some(a) => {
                            if a.r0 > vis.r0 {
                                pieces.push(Rect::new(vis.r0, vis.c0, a.r0 - 1, vis.c1));
                            }
                            if a.r1 < vis.r1 {
                                pieces.push(Rect::new(a.r1 + 1, vis.c0, vis.r1, vis.c1));
                            }
                            if a.c0 > vis.c0 {
                                pieces.push(Rect::new(a.r0, vis.c0, a.r1, a.c0 - 1));
                            }
                            if a.c1 < vis.c1 {
                                pieces.push(Rect::new(a.r0, a.c1 + 1, a.r1, vis.c1));
                            }
                        }
                        None => pieces.push(vis),
                    }
                    for p in pieces {
                        let (x0, y0, x1, y1) = layout.rect_px(p);
                        items.push(solid(layout, x0, y0, x1, y1, tint));
                    }
                }
                if is_current && !multi {
                    self.outline(items, *range, 2.0, accent, 0.0);
                } else {
                    self.outline(items, *range, 1.0, accent, 0.0);
                }
            }
        }
        if multi && active_rect.intersects(&visible) {
            self.outline(items, active_rect, 1.0, accent, 0.0);
        }

        if let Some(m) = self.input.marquee {
            if m.intersects(&visible) {
                self.dashed_outline(items, m, accent);
            }
        }
        if let Some(f) = self.input.fill_preview {
            if f.intersects(&visible) {
                self.outline(items, f, 2.0, c(0x808080), 0.0);
            }
        }

        // Fill handle at the bottom-right corner of the current range.
        if self.input.show_fill_handle && !multi {
            let cur = sel.current();
            if visible.contains(cur.r1, cur.c1) {
                let (_, _, x1, y1) = layout.rect_px(cur);
                items.push(solid(layout, x1 - 4.0, y1 - 4.0, x1 + 3.0, y1 + 3.0, c(0xFFFFFF)));
                items.push(solid(layout, x1 - 3.0, y1 - 3.0, x1 + 3.0, y1 + 3.0, accent));
            }
        }
    }
}

fn header_text(ts: &WindowTextSystem, text: String, size: f32, color: Hsla, bold: bool) -> ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: Font {
            family: ".SystemUIFont".into(),
            features: Default::default(),
            fallbacks: None,
            weight: if bold { FontWeight::SEMIBOLD } else { FontWeight::NORMAL },
            style: FontStyle::Normal,
        },
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    ts.shape_line(text.into(), px(size), &[run], None)
}

/// Build the whole grid scene: cell panes, floating objects, headers,
/// scrollbars.
pub fn build_scene(input: &SceneInput, layout: &GridLayout, ts: &WindowTextSystem) -> Scene {
    let sheet = &input.wb.sheets[input.sheet];
    let ctx = PaneCtx {
        input,
        layout,
        sheet,
        ts,
        hair: if input.scale >= 2.0 { 0.5 } else { 1.0 },
        layered: !sheet.tables.is_empty() || !sheet.pivots.is_empty() || !sheet.cond_formats.is_empty(),
        formulas: features::misc::shows_formulas(input.app),
        digit_w: RefCell::new(HashMap::new()),
        zones: RefCell::new(Vec::new()),
    };
    let mut layers: Vec<Layer> = Vec::new();
    let (hw, hh) = (layout.header_w, layout.header_h);
    let (w, h) = (layout.width(), layout.height());
    let fx = hw + layout.frozen_w;
    let fy = hh + layout.frozen_h;

    // Background.
    layers.push(Layer {
        clip: layout.bounds,
        items: vec![solid(layout, 0.0, 0.0, w, h, c(theme::GRID_BG))],
    });

    let s_rows = layout.scrolled_rows();
    let s_cols = layout.scrolled_cols();
    let f_rows = 0..layout.frozen_rows;
    let f_cols = 0..layout.frozen_cols;

    layers.push(ctx.build(s_rows.clone(), s_cols.clone(), (fx, fy, w, h)));
    if layout.frozen_rows > 0 {
        layers.push(ctx.build(f_rows.clone(), s_cols.clone(), (fx, hh, w, fy)));
    }
    if layout.frozen_cols > 0 {
        layers.push(ctx.build(s_rows.clone(), f_cols.clone(), (hw, fy, fx, h)));
    }
    if layout.frozen_rows > 0 && layout.frozen_cols > 0 {
        layers.push(ctx.build(f_rows.clone(), f_cols.clone(), (hw, hh, fx, fy)));
    }

    // Freeze dividers.
    let mut chrome: Vec<Draw> = Vec::new();
    if layout.frozen_cols > 0 {
        chrome.push(solid(layout, fx - 1.0, 0.0, fx, h, c(0x9E9E9E)));
    }
    if layout.frozen_rows > 0 {
        chrome.push(solid(layout, 0.0, fy - 1.0, w, fy, c(0x9E9E9E)));
    }
    layers.push(Layer {
        clip: layout.bounds,
        items: chrome,
    });

    // Charts, pictures and shapes float over the cells and scroll with them.
    let all_visible = Rect::new(
        if layout.frozen_rows > 0 { 0 } else { s_rows.start },
        if layout.frozen_cols > 0 { 0 } else { s_cols.start },
        s_rows.end.max(1) - 1,
        s_cols.end.max(1) - 1,
    );
    let objects = features::objects(input.app, input.sheet);
    if !objects.is_empty() {
        let clip = (hw, hh, w, h);
        let mut painter = Painter::new(input.app, layout, ts, PaintLayer::Top, all_visible, clip, input.scale);
        for (id, object) in &objects {
            let rect = crate::objects::object_rect(layout, &object.place);
            if rect.2 < hw || rect.3 < hh || rect.0 > w || rect.1 > h {
                continue;
            }
            painter.owner = id.owner;
            features::all()[id.owner].paint_object(&mut painter, object, rect);
            if input.app.object_sel == Some(*id) {
                crate::objects::paint_selection(&mut painter, rect);
            }
        }
        ctx.zones.borrow_mut().append(&mut painter.zones);
        layers.push(Layer {
            clip: layout.bounds_px(hw, hh, w, h),
            items: painter.items,
        });
    }

    // Headers.
    if hh > 0.0 {
        let sel = input.selection;
        let font_size = (12.0 * layout.zoom).clamp(8.0, 20.0);
        let line_h = (font_size * 1.3).round();
        let col_state = |col: u32| -> u8 {
            let mut state = 0;
            for r in &sel.ranges {
                if col >= r.c0 && col <= r.c1 {
                    state = state.max(if r.is_whole_cols() { 2 } else { 1 });
                }
            }
            state
        };
        let row_state = |row: u32| -> u8 {
            let mut state = 0;
            for r in &sel.ranges {
                if row >= r.r0 && row <= r.r1 {
                    state = state.max(if r.is_whole_rows() { 2 } else { 1 });
                }
            }
            state
        };
        let colors = |state: u8| -> (Hsla, Hsla) {
            match state {
                2 => (c(theme::HEADER_FULL_BG), c(0xFFFFFF)),
                1 => (c(theme::HEADER_SEL_BG), c(theme::HEADER_SEL_TEXT)),
                _ => (c(theme::HEADER_BG), c(theme::HEADER_TEXT)),
            }
        };

        for (range, clip) in [(s_cols.clone(), (fx, 0.0, w, hh)), (f_cols.clone(), (hw, 0.0, fx, hh))] {
            let mut items = vec![solid(layout, clip.0, 0.0, clip.2, hh, c(theme::HEADER_BG))];
            for col in range {
                let (x, cw) = (layout.col_x(col), layout.col_w(col));
                if cw <= 0.0 {
                    continue;
                }
                let state = col_state(col);
                let (bg, fg) = colors(state);
                if state > 0 {
                    items.push(solid(layout, x, 0.0, x + cw, hh, bg));
                }
                items.push(solid(layout, x + cw - 1.0, 0.0, x + cw, hh, c(theme::HEADER_BORDER)));
                let line = header_text(ts, col_to_letters(col), font_size, fg, state > 0);
                let lw = f32::from(line.width);
                if lw + 2.0 <= cw {
                    items.push(Draw::Text {
                        line,
                        origin: layout.to_window(x + ((cw - lw) / 2.0).round(), ((hh - line_h) / 2.0).round()),
                        line_height: px(line_h),
                        clip: layout.bounds_px(x, 0.0, x + cw, hh),
                    });
                }
                if state == 1 {
                    items.push(solid(layout, x, hh - 2.0, x + cw, hh, c(theme::SELECTION)));
                }
            }
            items.push(solid(layout, clip.0, hh - 1.0, clip.2, hh, c(theme::HEADER_BORDER)));
            layers.push(Layer {
                clip: layout.bounds_px(clip.0, clip.1, clip.2, clip.3),
                items,
            });
        }
        for (range, clip) in [(s_rows.clone(), (0.0, fy, hw, h)), (f_rows.clone(), (0.0, hh, hw, fy))] {
            let mut items = vec![solid(layout, 0.0, clip.1, hw, clip.3, c(theme::HEADER_BG))];
            for row in range {
                let (y, rh) = (layout.row_y(row), layout.row_h(row));
                if rh <= 0.0 {
                    continue;
                }
                let state = row_state(row);
                let (bg, fg) = colors(state);
                if state > 0 {
                    items.push(solid(layout, 0.0, y, hw, y + rh, bg));
                }
                items.push(solid(layout, 0.0, y + rh - 1.0, hw, y + rh, c(theme::HEADER_BORDER)));
                if rh >= line_h * 0.7 {
                    let line = header_text(ts, (row + 1).to_string(), font_size, fg, state > 0);
                    let lw = f32::from(line.width);
                    items.push(Draw::Text {
                        line,
                        origin: layout.to_window(((hw - lw) / 2.0).round(), y + ((rh - line_h) / 2.0).round()),
                        line_height: px(line_h),
                        clip: layout.bounds_px(0.0, y, hw, y + rh),
                    });
                }
                if state == 1 {
                    items.push(solid(layout, hw - 2.0, y, hw, y + rh, c(theme::SELECTION)));
                }
            }
            items.push(solid(layout, hw - 1.0, clip.1, hw, clip.3, c(theme::HEADER_BORDER)));
            layers.push(Layer {
                clip: layout.bounds_px(clip.0, clip.1, clip.2, clip.3),
                items,
            });
        }
        // Select-all corner.
        let mut corner = vec![
            solid(layout, 0.0, 0.0, hw, hh, c(theme::HEADER_BG)),
            solid(layout, hw - 1.0, 0.0, hw, hh, c(theme::HEADER_BORDER)),
            solid(layout, 0.0, hh - 1.0, hw, hh, c(theme::HEADER_BORDER)),
        ];
        // Small triangle, drawn as stacked rows.
        let t = (hh.min(hw) * 0.5).round();
        for i in 0..t as i32 {
            let i = i as f32;
            corner.push(solid(
                layout,
                hw - 4.0 - i,
                hh - 4.0 - (t - i),
                hw - 4.0,
                hh - 3.0 - (t - i),
                c(0xB0B0B0),
            ));
        }
        layers.push(Layer {
            clip: layout.bounds_px(0.0, 0.0, hw, hh),
            items: corner,
        });
    }

    // What features draw over everything.
    let mut top: Vec<Draw> = Vec::new();
    ctx.feature_layer(PaintLayer::Top, all_visible, (0.0, 0.0, w, h), &mut top);
    if !top.is_empty() {
        layers.push(Layer {
            clip: layout.bounds,
            items: top,
        });
    }

    // Scrollbars (overlay style).
    let mut bars: Vec<Draw> = Vec::new();
    let thumb = |hot: bool| ca(0x000000, if hot { 0.45 } else { 0.25 });
    {
        let (tx, ty, _len, pos, tl) = layout.v_scrollbar(input.content_h);
        let b = layout.bounds_px(tx + 3.0, ty + pos + 2.0, tx + theme::SCROLLBAR - 3.0, ty + pos + tl - 2.0);
        bars.push(Draw::Quad(fill(b, thumb(input.hot_scrollbar == 1)).corner_radii(px(3.))));
    }
    {
        let (tx, ty, _len, pos, tl) = layout.h_scrollbar(input.content_w);
        let b = layout.bounds_px(tx + pos + 2.0, ty + 3.0, tx + pos + tl - 2.0, ty + theme::SCROLLBAR - 3.0);
        bars.push(Draw::Quad(fill(b, thumb(input.hot_scrollbar == 2)).corner_radii(px(3.))));
    }
    layers.push(Layer {
        clip: layout.bounds,
        items: bars,
    });

    Scene {
        layers,
        zones: ctx.zones.into_inner(),
    }
}

pub fn paint_layers(layers: Vec<Layer>, window: &mut Window, cx: &mut App) {
    for layer in layers {
        window.with_content_mask(Some(ContentMask { bounds: layer.clip }), |window| {
            for item in layer.items {
                match item {
                    Draw::Quad(q) => window.paint_quad(q),
                    Draw::Path(path, color) => window.paint_path(path, color),
                    Draw::Text {
                        line,
                        origin,
                        line_height,
                        clip,
                    } => {
                        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                            let _ = line.paint(origin, line_height, window, cx);
                        });
                    }
                    Draw::Turned {
                        path,
                        bounds,
                        pivot,
                        turn,
                        color,
                        clip,
                    } => {
                        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                            crate::chart_view::paint_turned(path, bounds, pivot, turn, color, window, cx);
                        });
                    }
                    Draw::Wrapped {
                        lines,
                        origin,
                        line_height,
                        align,
                        bounds,
                    } => {
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            let mut y = origin.y;
                            let text_bounds = Bounds::new(
                                point(origin.x, bounds.origin.y),
                                size(
                                    (bounds.size.width - (origin.x - bounds.origin.x) * 2.0).max(px(1.)),
                                    bounds.size.height,
                                ),
                            );
                            for line in &lines {
                                let rows = line.wrap_boundaries().len() + 1;
                                let _ = line.paint(point(origin.x, y), line_height, align, Some(text_bounds), window, cx);
                                y += line_height * rows as f32;
                            }
                        });
                    }
                }
            }
        });
    }
}

/// Width in pixels (at 100 %) a column needs to show all its content.
pub fn autofit_col_width(wb: &Workbook, sheet: usize, col: u32, fonts: &FontResolver, ts: &WindowTextSystem) -> Option<f32> {
    let sh = wb.sheets.get(sheet)?;
    let mut max_w: f32 = 0.0;
    let mut any = false;
    for ((row, c2), cell) in sh.cells.iter() {
        if *c2 != col || matches!(cell.content, Content::Empty) {
            continue;
        }
        if sh.merge_at(*row, *c2).is_some() {
            continue;
        }
        let style = wb.styles.get(cell.style);
        let text = format::format_value(&cell.value, &style.num_fmt, wb.locale).text;
        if text.is_empty() {
            continue;
        }
        let font = cell_font(style, fonts);
        let size = pt_to_px(style.font.size_pt());
        let run = TextRun {
            len: text.len(),
            font,
            color: c(0),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let text = text.replace(['\n', '\r'], " ");
        let run = TextRun { len: text.len(), ..run };
        let w = f32::from(ts.shape_line(text.into(), px(size), &[run], None).width);
        max_w = max_w.max(w + style.indent as f32 * 9.0);
        any = true;
    }
    any.then(|| (max_w + 9.0).ceil().clamp(12.0, 1800.0))
}
