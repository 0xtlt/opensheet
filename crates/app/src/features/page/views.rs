//! What the views draw on the grid and what the mouse does there.
//!
//! * Normal view: the page breaks, dashed where the paper runs out and
//!   solid where the user set them, once the sheet has been previewed or
//!   its setup changed.
//! * Page Break Preview: what prints on white and the rest on grey, the
//!   print area outlined in blue, the breaks as blue lines that can be
//!   dragged, the number of each page behind its cells.
//! * Page Layout view: the sheet cut into pages lying on a desk, with
//!   their margins, rulers, and headers and footers typed in place. The
//!   cells stay where the grid draws them: the view only leaves room
//!   between the pages (`Axis::set_gaps`) and paints over that room.

use super::{Cmd, HF_TAB, PageUi, change, metric, setup};
use crate::app::{Drag, ExcelApp, MenuEntry};
use crate::features::{Cx, Intent, PaintLayer, Painter, StatusItem, ViewMode, Zone, ZoneEvent, ZoneResponse};
use crate::grid::{Axis, GridLayout};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use crate::shell::{Alert, Modal};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c, ca};
use crate::widgets::{self, Checkbox, NumberField, dialog, labeled, ok_cancel};
use gpui::{
    AnyElement, Context, CursorStyle, Entity, Focusable, Font, FontStyle, FontWeight, Hsla, MouseButton,
    PathPromptOptions, Render, Subscription, Window, div, prelude::*, px,
};
use opensheet_engine::datetime;
use opensheet_engine::drawing::image_info;
use opensheet_engine::page::{
    BreakAxis, HeaderFooter, HfContext, HfPicture, HfRun, HfSection, HfSections, HfSlot, HfVariant, PaginateOptions,
    Pagination, SheetPages, expand, from_dialog_text, lines, paginate, plain_text, presets, sheet_pages,
    to_dialog_text,
};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, col_to_letters};
use opensheet_engine::{Locale, Rect};
use opensheet_print::FontDb;
use std::rc::Rc;
use std::sync::Arc;

/// The blue of Page Break Preview.
const PREVIEW_BLUE: u32 = 0x1F4FD8;
/// What does not print, in Page Break Preview.
const PREVIEW_GREY: u32 = 0xB4B4B4;
/// The desk the pages of the Page Layout view lie on.
const DESK: u32 = 0xD6D6D6;
/// Thickness of the rulers of the Page Layout view.
const RULER: f32 = 18.0;
/// What is left between two pages when the white space is hidden.
const FOLD: f32 = 5.0;
/// Stands for a line break in a header typed on one line.
const LINE_BREAK: char = '¶';

// ------------------------------------------------------------------ pages

/// The pages of the current sheet, kept until the workbook changes.
pub(crate) struct Cached {
    key: (usize, u64, u64),
    pagination: Rc<Pagination>,
    /// The whole sheet in pages, worked out when the Page Layout view
    /// first asks for it.
    pages: Option<Rc<SheetPages>>,
}

/// What changes the pages before the workbook is committed: the column
/// or row being resized with the mouse.
fn live_key(app: &ExcelApp) -> u64 {
    let sheet = &app.wb.sheets[app.sheet];
    match &app.drag {
        Some(Drag::ColResize { col, .. }) => 1 << 62 | (*col as u64) << 32 | sheet.col_width(*col).to_bits() as u64,
        Some(Drag::RowResize { row, .. }) => 2 << 62 | (*row as u64) << 32 | sheet.row_height(*row).to_bits() as u64,
        _ => 0,
    }
}

fn with_cache<R>(app: &ExcelApp, f: impl FnOnce(&mut Cached) -> R) -> Option<R> {
    let ui = app.ext::<PageUi>()?;
    let key = (app.sheet, app.wb.generation, live_key(app));
    let mut cache = ui.cache.borrow_mut();
    if cache.as_ref().is_none_or(|cached| cached.key != key) {
        // Text is measured with the fonts the printer uses, so that the
        // screen shows the pages the paper will.
        let options = PaginateOptions {
            measure: Some(FontDb::system()),
            ..PaginateOptions::default()
        };
        *cache = Some(Cached {
            key,
            pagination: Rc::new(paginate(&app.wb, app.sheet, &options)),
            pages: None,
        });
    }
    cache.as_mut().map(f)
}

/// The printed pages of the current sheet. `None` until the feature has
/// been used (nothing shows page breaks before that).
pub(crate) fn pagination(app: &ExcelApp) -> Option<Rc<Pagination>> {
    with_cache(app, |cached| cached.pagination.clone())
}

/// The whole current sheet cut into pages, for the Page Layout view.
fn all_pages(app: &ExcelApp) -> Option<Rc<SheetPages>> {
    with_cache(app, |cached| {
        cached
            .pages
            .get_or_insert_with(|| Rc::new(sheet_pages(&app.wb, app.sheet, &cached.pagination)))
            .clone()
    })
}

/// The file name and the folder of the workbook, for `&F` and `&Z`.
pub(crate) fn file_names(app: &ExcelApp) -> (String, String) {
    match &app.path {
        Some(path) => (
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| app.title.clone()),
            path.parent()
                .map(|dir| format!("{}{}", dir.to_string_lossy(), std::path::MAIN_SEPARATOR))
                .unwrap_or_default(),
        ),
        None => (app.title.clone(), String::new()),
    }
}

// ------------------------------------------------------------ dispatching

pub(crate) fn paint(p: &mut Painter) {
    match p.app.view_mode() {
        ViewMode::Normal => paint_normal(p),
        ViewMode::PageBreakPreview => paint_preview(p),
        ViewMode::PageLayout => paint_layout(p),
    }
}

pub(crate) fn gridline_areas(app: &ExcelApp) -> Option<Vec<Rect>> {
    if app.view_mode() != ViewMode::PageBreakPreview {
        return None;
    }
    pagination(app).map(|p| p.ranges.iter().map(|range| range.rect).collect())
}

/// A horizontal or vertical line of the given thickness, cut to the pane.
fn rule(p: &mut Painter, from: (f32, f32), to: (f32, f32), width: f32, color: Hsla) {
    let (x0, y0) = (from.0.max(p.clip.0 - 4.0), from.1.max(p.clip.1 - 4.0));
    let (x1, y1) = (to.0.min(p.clip.2 + 4.0), to.1.min(p.clip.3 + 4.0));
    if from.1 == to.1 {
        if x1 > x0 {
            p.rect(x0, y0, x1, y0 + width, color);
        }
    } else if y1 > y0 {
        p.rect(x0, y0, x0 + width, y1, color);
    }
}

// ------------------------------------------------------------ Normal view

fn paint_normal(p: &mut Painter) {
    if p.layer != PaintLayer::Over {
        return;
    }
    let app = p.app;
    let sheet = &app.wb.sheets[app.sheet];
    if !app
        .ext::<PageUi>()
        .is_some_and(|ui| ui.breaks_shown.contains(&sheet.name))
    {
        return;
    }
    let Some(pagination) = pagination(app) else {
        return;
    };
    let color = c(0x6E6E6E);
    let visible = p.visible;
    for range in &pagination.ranges {
        let (x0, y0, x1, y1) = p.cell_box(range.rect);
        for b in &range.row_breaks {
            if b.at < visible.r0 || b.at > visible.r1 + 1 {
                continue;
            }
            let y = p.layout.row_y(b.at) - 1.0;
            if b.manual {
                rule(p, (x0, y), (x1, y), 1.0, color);
            } else {
                p.dashed((x0.max(p.clip.0), y), (x1.min(p.clip.2), y), 1.0, 4.0, 3.0, color);
            }
        }
        for b in &range.col_breaks {
            if b.at < visible.c0 || b.at > visible.c1 + 1 {
                continue;
            }
            let x = p.layout.col_x(b.at) - 1.0;
            if b.manual {
                rule(p, (x, y0), (x, y1), 1.0, color);
            } else {
                p.dashed((x, y0.max(p.clip.1)), (x, y1.min(p.clip.3)), 1.0, 4.0, 3.0, color);
            }
        }
    }
    // A print area set by hand is outlined with a thin line.
    for area in &sheet.page.print_area {
        if !area.intersects(&visible) {
            continue;
        }
        let (x0, y0, x1, y1) = p.cell_box(*area);
        let line = c(0xA0A0A0);
        if area.r0 >= visible.r0 {
            rule(p, (x0 - 1.0, y0 - 1.0), (x1, y0 - 1.0), 1.0, line);
        }
        if area.r1 <= visible.r1 {
            rule(p, (x0 - 1.0, y1 - 1.0), (x1, y1 - 1.0), 1.0, line);
        }
        if area.c0 >= visible.c0 {
            rule(p, (x0 - 1.0, y0 - 1.0), (x0 - 1.0, y1), 1.0, line);
        }
        if area.c1 <= visible.c1 {
            rule(p, (x1 - 1.0, y0 - 1.0), (x1 - 1.0, y1), 1.0, line);
        }
    }
}

// ----------------------------------------------------- Page Break Preview

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// A line of Page Break Preview that can be dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    /// A page break: the row or column that starts the next page.
    Break { axis: BreakAxis, at: u32 },
    /// An edge of a printed range (index into `Pagination::ranges`).
    Edge { range: usize, edge: Edge },
}

impl Handle {
    fn vertical(&self) -> bool {
        matches!(
            self,
            Handle::Break {
                axis: BreakAxis::Cols,
                ..
            } | Handle::Edge {
                edge: Edge::Left | Edge::Right,
                ..
            }
        )
    }
}

/// A line being dragged, and the row or column boundary it is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BreakDrag {
    handle: Handle,
    to: u32,
}

/// The print area once an edge of one of its ranges has been dragged to
/// the boundary before row or column `to`. An edge cannot cross the one
/// facing it.
pub(crate) fn moved_edge(ranges: &[Rect], range: usize, edge: Edge, to: u32) -> Vec<Rect> {
    let mut out = ranges.to_vec();
    if let Some(r) = out.get_mut(range) {
        match edge {
            Edge::Left => r.c0 = to.min(r.c1),
            Edge::Right => r.c1 = to.saturating_sub(1).max(r.c0).min(MAX_COLS - 1),
            Edge::Top => r.r0 = to.min(r.r1),
            Edge::Bottom => r.r1 = to.saturating_sub(1).max(r.r0).min(MAX_ROWS - 1),
        }
    }
    out
}

/// The row or column boundary nearest to a point of the grid: the index
/// of the track that starts there.
fn boundary_at(layout: &GridLayout, vertical: bool, x: f32, y: f32) -> u32 {
    if vertical {
        let col = layout.col_at(x.max(layout.header_w + 1.0));
        let middle = layout.col_x(col) + layout.col_w(col) / 2.0;
        if x > middle { (col + 1).min(MAX_COLS) } else { col }
    } else {
        let row = layout.row_at(y.max(layout.header_h + 1.0));
        let middle = layout.row_y(row) + layout.row_h(row) / 2.0;
        if y > middle { (row + 1).min(MAX_ROWS) } else { row }
    }
}

fn paint_preview(p: &mut Painter) {
    let app = p.app;
    let Some(pagination) = pagination(app) else {
        return;
    };
    let visible = p.visible;
    let clip = p.clip;
    match p.layer {
        PaintLayer::Under => {
            p.rect(clip.0, clip.1, clip.2, clip.3, c(PREVIEW_GREY));
            for range in &pagination.ranges {
                if let Some(shown) = range.rect.intersection(&visible) {
                    let (x0, y0, x1, y1) = p.cell_box(shown);
                    p.rect(x0, y0, x1, y1, c(theme::GRID_BG));
                }
            }
            // "Page 1" behind the cells of each page.
            let word = t("Page", "Page");
            for page in &pagination.pages {
                let cells = Rect::new(page.rows.0, page.cols.0, page.rows.1, page.cols.1);
                if !cells.intersects(&visible) {
                    continue;
                }
                let (x0, y0, x1, y1) = p.cell_box(cells);
                let size = ((x1 - x0).min(y1 - y0) * 0.2).clamp(9.0, 64.0);
                let line = p.shape(&format!("{word} {}", page.index + 1), size, c(0xA6A6A6), true);
                let width = f32::from(line.width);
                p.put(
                    line,
                    x0 + (x1 - x0 - width) / 2.0,
                    y0 + (y1 - y0 - size * 1.25) / 2.0,
                    size,
                );
            }
        }
        PaintLayer::Over => {
            let blue = c(PREVIEW_BLUE);
            let grab = 4.0;
            for (index, range) in pagination.ranges.iter().enumerate() {
                let rect = range.rect;
                let (x0, y0, x1, y1) = p.cell_box(rect);
                for b in &range.col_breaks {
                    if b.at < visible.c0 || b.at > visible.c1 + 1 {
                        continue;
                    }
                    let x = p.layout.col_x(b.at) - 1.0;
                    if b.manual {
                        rule(p, (x - 1.0, y0), (x - 1.0, y1), 2.0, blue);
                    } else {
                        p.dashed(
                            (x - 1.0, y0.max(clip.1)),
                            (x - 1.0, y1.min(clip.3)),
                            2.0,
                            7.0,
                            5.0,
                            blue,
                        );
                    }
                    let handle = Handle::Break {
                        axis: BreakAxis::Cols,
                        at: b.at,
                    };
                    p.zone((x - grab, y0, x + grab, y1), handle)
                        .cursor(CursorStyle::ResizeLeftRight);
                }
                for b in &range.row_breaks {
                    if b.at < visible.r0 || b.at > visible.r1 + 1 {
                        continue;
                    }
                    let y = p.layout.row_y(b.at) - 1.0;
                    if b.manual {
                        rule(p, (x0, y - 1.0), (x1, y - 1.0), 2.0, blue);
                    } else {
                        p.dashed(
                            (x0.max(clip.0), y - 1.0),
                            (x1.min(clip.2), y - 1.0),
                            2.0,
                            7.0,
                            5.0,
                            blue,
                        );
                    }
                    let handle = Handle::Break {
                        axis: BreakAxis::Rows,
                        at: b.at,
                    };
                    p.zone((x0, y - grab, x1, y + grab), handle)
                        .cursor(CursorStyle::ResizeUpDown);
                }
                // The outline: each edge is drawn (and can be dragged) by
                // the pane that shows it.
                let edges = [
                    (Edge::Left, rect.c0 >= visible.c0 && rect.c0 <= visible.c1 + 1),
                    (Edge::Right, rect.c1 + 1 >= visible.c0 && rect.c1 <= visible.c1),
                    (Edge::Top, rect.r0 >= visible.r0 && rect.r0 <= visible.r1 + 1),
                    (Edge::Bottom, rect.r1 + 1 >= visible.r0 && rect.r1 <= visible.r1),
                ];
                for (edge, shown) in edges {
                    if !shown {
                        continue;
                    }
                    let handle = Handle::Edge { range: index, edge };
                    match edge {
                        Edge::Left | Edge::Right => {
                            // The left edge of a range starting in column A
                            // stays in view beside the headings.
                            let x = if edge == Edge::Left {
                                (x0 - 2.0).max(clip.0)
                            } else {
                                x1 - 2.0
                            };
                            rule(p, (x, y0 - 2.0), (x, y1 + 1.0), 3.0, blue);
                            p.zone((x - grab, y0, x + 3.0 + grab, y1), handle)
                                .cursor(CursorStyle::ResizeLeftRight);
                        }
                        Edge::Top | Edge::Bottom => {
                            let y = if edge == Edge::Top {
                                (y0 - 2.0).max(clip.1)
                            } else {
                                y1 - 2.0
                            };
                            rule(p, (x0 - 2.0, y), (x1 + 1.0, y), 3.0, blue);
                            p.zone((x0, y - grab, x1, y + 3.0 + grab), handle)
                                .cursor(CursorStyle::ResizeUpDown);
                        }
                    }
                }
            }
            // The line being dragged follows the pointer.
            if let Some(drag) = app.ext::<PageUi>().and_then(|ui| ui.drag) {
                let ghost = c(0x404040);
                if drag.handle.vertical() {
                    if drag.to >= visible.c0 && drag.to <= visible.c1 + 1 {
                        let x = p.layout.col_x(drag.to.min(MAX_COLS - 1)) - 2.0;
                        p.dashed((x, clip.1), (x, clip.3), 2.0, 3.0, 3.0, ghost);
                    }
                } else if drag.to >= visible.r0 && drag.to <= visible.r1 + 1 {
                    let y = p.layout.row_y(drag.to.min(MAX_ROWS - 1)) - 2.0;
                    p.dashed((clip.0, y), (clip.2, y), 2.0, 3.0, 3.0, ghost);
                }
            }
        }
        PaintLayer::Top => {}
    }
}

/// Drop a dragged line where it is: a page break moves (an automatic one
/// becomes manual), an edge of the outline changes the print area.
fn drop_line(app: &mut ExcelApp, drag: BreakDrag, cx: &mut Cx) {
    // A protected sheet keeps its page breaks and its print area.
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    match drag.handle {
        Handle::Break { axis, at } => {
            if drag.to != at
                && app
                    .wb
                    .move_page_break(app.sheet, axis, at, drag.to, Some(FontDb::system()))
            {
                app.commit(t("Move Page Break", "Déplacer le saut de page"), cx);
            }
        }
        Handle::Edge { range, edge } => {
            let Some(pagination) = pagination(app) else {
                return;
            };
            let ranges: Vec<Rect> = pagination.ranges.iter().map(|r| r.rect).collect();
            let moved = moved_edge(&ranges, range, edge, drag.to);
            if moved != ranges && app.wb.set_print_area(app.sheet, &moved) {
                app.commit(
                    crate::i18n::t_ctx("undo", "Set Print Area", "Définir la zone d'impression"),
                    cx,
                );
            }
        }
    }
}

// ------------------------------------------------------- Page Layout view

/// The Page Layout view at a zoom: paper, margins and the desk between
/// the pages, in screen pixels.
struct Desk {
    pages: Rc<SheetPages>,
    pagination: Rc<Pagination>,
    /// Screen pixels per point of paper.
    px_per_pt: f32,
    paper: (f32, f32),
    /// Left, top, right, bottom.
    margin: (f32, f32, f32, f32),
    /// Distance from the top of the paper to the header, and from its
    /// bottom to the footer.
    header: f32,
    footer: f32,
    /// Desk around and between the pages.
    gap: f32,
    /// Thickness of the rulers; 0 when they are hidden.
    ruler: f32,
    /// The margins above and below the pages are folded away.
    folded: bool,
}

/// Where one page and its cells are in a layout: (x0, y0, x1, y1).
struct PageBox {
    paper: (f32, f32, f32, f32),
    cells: (f32, f32, f32, f32),
    rows: (u32, u32),
    cols: (u32, u32),
}

impl Desk {
    fn of(app: &ExcelApp, zoom: f32) -> Option<Desk> {
        let pages = all_pages(app)?;
        let pagination = pagination(app)?;
        let ui = app.ext::<PageUi>()?;
        let margins = &setup(app).margins;
        let px_per_pt = zoom / pages.unit() as f32;
        let px = |pt: f64| (pt as f32 * px_per_pt).round();
        let (left, top, width, height) = pages.printable;
        Some(Desk {
            px_per_pt,
            paper: (px(pages.paper.0), px(pages.paper.1)),
            margin: (
                px(left),
                px(top),
                px(pages.paper.0 - left - width).max(0.0),
                px(pages.paper.1 - top - height).max(0.0),
            ),
            header: px(margins.header * 72.0),
            footer: px(margins.footer * 72.0),
            gap: (16.0 * zoom).round().max(6.0),
            ruler: if ui.ruler_hidden { 0.0 } else { RULER },
            folded: ui.white_space_hidden,
            pages,
            pagination,
        })
    }

    fn page_box(&self, layout: &GridLayout, row_page: usize, col_page: usize) -> PageBox {
        let rows = self.pages.rows(row_page);
        let cols = self.pages.cols(col_page);
        let x0 = layout.col_x(cols.0);
        let x1 = layout.col_x(cols.1) + layout.col_w(cols.1);
        let y0 = layout.row_y(rows.0);
        let y1 = layout.row_y(rows.1) + layout.row_h(rows.1);
        let left = x0 - self.margin.0;
        let paper = if self.folded {
            (left, y0, left + self.paper.0, y1)
        } else {
            let top = y0 - self.margin.1;
            (left, top, left + self.paper.0, top + self.paper.1)
        };
        PageBox {
            paper,
            cells: (x0, y0, x1, y1),
            rows,
            cols,
        }
    }

    /// The pages a pane shows, with one more on each side (its margin
    /// may be in view): rows of pages, then columns of pages.
    fn shown(&self, visible: Rect) -> (std::ops::RangeInclusive<usize>, std::ops::RangeInclusive<usize>) {
        let around = |first: usize, last: usize, count: usize| first.saturating_sub(1)..=(last + 1).min(count - 1);
        (
            around(
                self.pages.row_page(visible.r0),
                self.pages.row_page(visible.r1),
                self.pages.row_starts.len(),
            ),
            around(
                self.pages.col_page(visible.c0),
                self.pages.col_page(visible.c1),
                self.pages.col_starts.len(),
            ),
        )
    }

    /// The printed page showing these cells, if they print.
    fn printed(&self, page: &PageBox) -> Option<&opensheet_engine::page::Page> {
        self.pagination
            .pages
            .iter()
            .find(|p| p.rows.0 == page.rows.0 && p.cols.0 == page.cols.0)
    }

    /// The box of one section of the header or footer of a page.
    fn section_box(&self, page: &PageBox, hf: &HeaderFooter, footer: bool, section: HfSection) -> (f32, f32, f32, f32) {
        let (left, right) = self.header_span(page, hf);
        let third = (right - left) / 3.0;
        let k = match section {
            HfSection::Left => 0.0,
            HfSection::Center => 1.0,
            HfSection::Right => 2.0,
        };
        let (top, bottom) = if footer {
            (page.cells.3 + 2.0, (page.paper.3 - self.footer).max(page.cells.3 + 6.0))
        } else {
            ((page.paper.1 + self.header).min(page.cells.1 - 6.0), page.cells.1 - 2.0)
        };
        (left + k * third, top, left + (k + 1.0) * third, bottom)
    }

    /// Where the left section starts and the right one ends.
    fn header_span(&self, page: &PageBox, hf: &HeaderFooter) -> (f32, f32) {
        if hf.align_with_margins {
            (page.paper.0 + self.margin.0, page.paper.2 - self.margin.2)
        } else {
            // 0.7 inch from the edges of the paper, as on paper.
            let inset = (0.7 * 72.0 * self.px_per_pt).round();
            (page.paper.0 + inset, page.paper.2 - inset)
        }
    }
}

/// The room to leave before the first track of every page along one axis
/// (and after the last one): what is left of the page before, its far
/// margin, the desk and the near margin of the page itself. `margins` are
/// the near and far margins of the paper; `folded` leaves only a thin
/// strip between the pages.
fn page_gaps(
    axis: &Axis,
    starts: &[u32],
    count: u32,
    paper: f32,
    margins: (f32, f32),
    desk: f32,
    folded: bool,
) -> Vec<(u32, f32)> {
    let (before, after) = margins;
    let room = (paper - before - after).max(0.0);
    let left_over = |first: u32, next: u32| (room - (axis.start(next) - axis.start(first)) as f32).max(0.0);
    let mut out = Vec::with_capacity(starts.len() + 1);
    out.push((0, desk + if folded { 0.0 } else { before }));
    for pair in starts.windows(2) {
        let gap = if folded {
            FOLD
        } else {
            left_over(pair[0], pair[1]) + after + desk + before
        };
        out.push((pair[1], gap));
    }
    let last = starts.last().copied().unwrap_or(0);
    out.push((
        count,
        if folded {
            desk
        } else {
            left_over(last, count) + after + desk
        },
    ));
    out
}

pub(crate) fn adjust_layout(app: &ExcelApp, layout: &mut GridLayout) {
    if app.view_mode() != ViewMode::PageLayout {
        return;
    }
    let Some(desk) = Desk::of(app, layout.zoom) else {
        return;
    };
    let (m, paper, pages) = (desk.margin, desk.paper, &desk.pages);
    let cols = page_gaps(
        &layout.cols,
        &pages.col_starts,
        MAX_COLS,
        paper.0,
        (m.0, m.2),
        desk.gap,
        false,
    );
    let rows = page_gaps(
        &layout.rows,
        &pages.row_starts,
        MAX_ROWS,
        paper.1,
        (m.1, m.3),
        desk.gap,
        desk.folded,
    );
    layout.cols.set_gaps(&cols);
    layout.rows.set_gaps(&rows);
    // The rulers run along the headings.
    layout.header_w += desk.ruler;
    layout.header_h += desk.ruler;
}

/// What a click on the desk or on a margin of the Page Layout view lands
/// on (so that it does not select the cell behind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeskZone {
    /// A margin, the desk beside the pages, a ruler.
    Inert,
    /// The desk between two pages: a click folds the white space away,
    /// or brings it back.
    Between,
}

/// A section of the header or footer of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HfZone {
    footer: bool,
    section: HfSection,
    /// Page down, page across.
    page: (usize, usize),
}

fn hf_context<'a>(app: &'a ExcelApp, names: &'a (String, String), page: i32, pages: u32) -> HfContext<'a> {
    HfContext {
        page,
        pages,
        now: datetime::now_serial(),
        file_name: &names.0,
        file_path: &names.1,
        sheet_name: &app.wb.sheets[app.sheet].name,
        locale: app.wb.locale,
    }
}

fn paint_layout(p: &mut Painter) {
    let Some(desk) = Desk::of(p.app, p.zoom()) else {
        return;
    };
    match p.layer {
        PaintLayer::Under => {}
        PaintLayer::Over => paint_desk(p, &desk),
        PaintLayer::Top => paint_rulers(p, &desk),
    }
}

/// The desk, the margins of the pages, their headers and footers.
fn paint_desk(p: &mut Painter, desk: &Desk) {
    let app = p.app;
    let layout = p.layout;
    let clip = p.clip;
    let sheet = &app.wb.sheets[app.sheet];
    let hf = &sheet.page.header_footer;
    let (row_pages, col_pages) = desk.shown(p.visible);
    let desk_color = c(DESK);
    let (n_rows, n_cols) = (desk.pages.row_starts.len(), desk.pages.col_starts.len());

    // The desk shows wherever there are no cells: in the strips left
    // between the columns of pages, then between the rows of pages.
    for j in *col_pages.start()..=col_pages.end() + 1 {
        let from = if j == 0 {
            clip.0
        } else {
            desk.page_box(layout, 0, j - 1).cells.2
        };
        let to = if j < n_cols {
            desk.page_box(layout, 0, j).cells.0
        } else {
            clip.2
        };
        let (from, to) = (from.max(clip.0), to.min(clip.2));
        if to > from {
            p.rect(from, clip.1, to, clip.3, desk_color);
            p.zone((from, clip.1, to, clip.3), DeskZone::Inert);
        }
    }
    for i in *row_pages.start()..=row_pages.end() + 1 {
        let from = if i == 0 {
            clip.1
        } else {
            desk.page_box(layout, i - 1, 0).cells.3
        };
        let to = if i < n_rows {
            desk.page_box(layout, i, 0).cells.1
        } else {
            clip.3
        };
        let (from, to) = (from.max(clip.1), to.min(clip.3));
        if to > from {
            p.rect(clip.0, from, clip.2, to, desk_color);
            let between = i > 0 && i < n_rows;
            let zone = p.zone(
                (clip.0, from, clip.2, to),
                if between { DeskZone::Between } else { DeskZone::Inert },
            );
            if between {
                zone.tooltip(if desk.folded {
                    t("Show White Space", "Afficher les espaces")
                } else {
                    t("Hide White Space", "Masquer les espaces")
                });
            }
        }
    }

    let names = file_names(app);
    let total = desk.pagination.pages.len() as u32;
    let edit = app.ext::<PageUi>().and_then(|ui| ui.hf.as_ref());
    for i in row_pages.clone() {
        for j in col_pages.clone() {
            let page = desk.page_box(layout, i, j);
            let (px0, py0, px1, py1) = page.paper;
            let (cx0, cy0, cx1, cy1) = page.cells;
            if px1 < clip.0 || px0 > clip.2 || py1 < clip.1 || py0 > clip.3 {
                continue;
            }
            // The paper around the cells.
            let white = c(theme::GRID_BG);
            p.rect(px0, py0, px1, cy0, white);
            p.rect(px0, cy1, px1, py1, white);
            p.rect(px0, cy0, cx0, cy1, white);
            p.rect(cx1, cy0, px1, cy1, white);
            p.frame(px0 - 1.0, py0 - 1.0, px1 + 1.0, py1 + 1.0, 1.0, c(0xB0B0B0));

            let printed = desk.printed(&page);
            if printed.is_none() {
                // A page that will not print is dimmed; an unused one
                // says what to do with it.
                p.rect(cx0, cy0, cx1, cy1, ca(0xF0F0F0, 0.55));
                let cells = Rect::new(page.rows.0, page.cols.0, page.rows.1, page.cols.1);
                if !desk.pagination.ranges.iter().any(|range| range.rect.intersects(&cells))
                    && !sheet
                        .cells
                        .range((cells.r0, 0)..=(cells.r1, u32::MAX))
                        .any(|((_, col), _)| *col >= cells.c0 && *col <= cells.c1)
                {
                    let size = (13.0 * p.zoom()).clamp(8.0, 20.0);
                    let line = p.shape(
                        t("Click to add data", "Cliquez ici pour ajouter des données"),
                        size,
                        c(0xA0A0A0),
                        false,
                    );
                    let width = f32::from(line.width);
                    let (mx0, mx1) = (cx0.max(px0), cx1.min(px1));
                    p.put(
                        line,
                        mx0 + (mx1 - mx0 - width) / 2.0,
                        cy0 + (cy1.min(py1) - cy0) * 0.45,
                        size,
                    );
                }
            }
            if desk.folded {
                continue;
            }
            // Headers and footers: the page's own on a printed page, the
            // sheet's ordinary ones as a preview elsewhere.
            let (number, first) = printed.map_or((1, false), |page| (page.number, page.index == 0));
            let variant = hf.variant(first, number);
            let context = hf_context(app, &names, number, total.max(1));
            for footer in [false, true] {
                let code = if footer { hf.footer(variant) } else { hf.header(variant) };
                let sections = HfSections::parse(code);
                for section in [HfSection::Left, HfSection::Center, HfSection::Right] {
                    let bounds = desk.section_box(&page, hf, footer, section);
                    let zone = HfZone {
                        footer,
                        section,
                        page: (i, j),
                    };
                    p.zone(bounds, zone).cursor(CursorStyle::IBeam);
                    let being_typed =
                        edit.is_some_and(|e| e.footer == footer && e.section == section && e.page == (i, j));
                    if being_typed {
                        continue;
                    }
                    if code.is_empty() {
                        if section == HfSection::Center {
                            let hint = if footer {
                                t("Add footer", "Ajouter un pied de page")
                            } else {
                                t("Add header", "Ajouter un en-tête")
                            };
                            let size = (12.0 * p.zoom()).clamp(7.0, 18.0);
                            let line = p.shape(hint, size, c(0xA6A6A6), false);
                            let width = f32::from(line.width);
                            let y = if footer {
                                bounds.1 + 2.0
                            } else {
                                bounds.3 - size * 1.25 - 2.0
                            };
                            p.put(line, (bounds.0 + bounds.2 - width) / 2.0, y, size);
                        }
                        continue;
                    }
                    paint_section(p, desk, hf, sections.get(section), &context, footer, section, bounds);
                }
            }
        }
    }
}

/// The text of one section, laid out as it prints: the header hangs from
/// the header margin, the footer stands on the footer margin.
#[allow(clippy::too_many_arguments)]
fn paint_section(
    p: &mut Painter,
    desk: &Desk,
    hf: &HeaderFooter,
    source: &str,
    context: &HfContext,
    footer: bool,
    section: HfSection,
    bounds: (f32, f32, f32, f32),
) {
    if source.is_empty() {
        return;
    }
    let app = p.app;
    let default_font = app.wb.styles.get(opensheet_engine::StyleId(0)).font.name.clone();
    // Header text keeps its size on paper unless it scales with the sheet.
    let scale = if hf.scale_with_doc {
        desk.pagination.scale() as f32
    } else {
        1.0
    };
    let mut laid: Vec<(Vec<(gpui::ShapedLine, f32)>, f32)> = Vec::new();
    for line in lines(expand(source, context)) {
        let mut pieces = Vec::new();
        let mut height = 0.0f32;
        for run in line {
            let (text, format) = match run {
                HfRun::Text { text, format } => (text, format),
                HfRun::Picture => (format!("&[{}]", t("Picture", "Image")), Default::default()),
            };
            let size = (format.size * scale * desk.px_per_pt).max(1.0);
            let family = format.font.clone().unwrap_or_else(|| default_font.clone());
            let font = Font {
                family: app.fonts.resolve(&family),
                features: Default::default(),
                fallbacks: None,
                weight: if format.bold {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                },
                style: if format.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
            };
            let color = format.color.map(theme::engine_color).unwrap_or(c(0x000000));
            height = height.max((size * 1.22).round());
            pieces.push((p.shape_with(&text, font, size, color), size));
        }
        if height == 0.0 {
            height = (11.0 * scale * desk.px_per_pt * 1.22).round();
        }
        laid.push((pieces, height));
    }
    let total: f32 = laid.iter().map(|(_, height)| height).sum();
    let mut y = if footer { bounds.3 - total } else { bounds.1 };
    for (pieces, height) in laid {
        let width: f32 = pieces.iter().map(|(line, _)| f32::from(line.width)).sum();
        let mut x = match section {
            HfSection::Left => bounds.0,
            HfSection::Center => (bounds.0 + bounds.2 - width) / 2.0,
            HfSection::Right => bounds.2 - width,
        };
        for (line, size) in pieces {
            let advance = f32::from(line.width);
            p.put(line, x, y + (height - size * 1.25) / 2.0, size);
            x += advance;
        }
        y += height;
    }
}

/// The rulers, and the headings of the pages (the grid's own run across
/// the desk as well).
fn paint_rulers(p: &mut Painter, desk: &Desk) {
    let app = p.app;
    let layout = p.layout;
    let (hw, hh) = (layout.header_w, layout.header_h);
    let (w, h) = (layout.width(), layout.height());
    let r = desk.ruler;
    let sheet = &app.wb.sheets[app.sheet];
    let (row_pages, col_pages) = desk.shown(p.visible);
    let sel = app.sel();
    let border = c(theme::HEADER_BORDER);
    let font_size = (12.0 * layout.zoom).clamp(8.0, 20.0);
    let line_h = (font_size * 1.3).round();
    // Ruler units: inches where Excel is in English, centimetres elsewhere.
    let unit = 72.0 * desk.px_per_pt / if metric(app.wb.locale) { 2.54 } else { 1.0 };
    let every = [1u32, 2, 5, 10]
        .into_iter()
        .find(|n| *n as f32 * unit >= 16.0)
        .unwrap_or(20);
    let ink = c(0x8A8A8A);
    let paper = c(0xFFFFFF);
    let margin_shade = c(0xBDBDBD);
    let state_colors = |state: u8| match state {
        2 => (c(theme::HEADER_FULL_BG), c(0xFFFFFF)),
        1 => (c(theme::HEADER_SEL_BG), c(theme::HEADER_SEL_TEXT)),
        _ => (c(theme::HEADER_BG), c(theme::HEADER_TEXT)),
    };

    if hh > 0.0 {
        p.rect(hw, 0.0, w, hh, c(DESK));
        for j in col_pages.clone() {
            let page = desk.page_box(layout, 0, j);
            let (px0, px1) = (page.paper.0.max(hw), page.paper.2);
            if px1 <= hw || page.paper.0 >= w {
                continue;
            }
            if r > 0.0 {
                let (inner0, inner1) = (page.paper.0 + desk.margin.0, page.paper.2 - desk.margin.2);
                p.rect(px0, 2.0, px1, r - 2.0, margin_shade);
                p.rect(inner0.max(hw), 2.0, inner1.max(hw), r - 2.0, paper);
                // Units are counted from the left margin.
                let mut k = 0u32;
                loop {
                    let x = inner0 + k as f32 * unit;
                    if x > inner1 {
                        break;
                    }
                    if x >= hw && k > 0 {
                        if k % every == 0 {
                            let label = p.shape(&k.to_string(), 9.0, ink, false);
                            let width = f32::from(label.width);
                            if x + width / 2.0 < inner1 {
                                p.put(label, x - width / 2.0, 3.0, 9.0);
                            }
                        } else {
                            p.rect(x, r / 2.0 - 2.0, x + 1.0, r / 2.0 + 2.0, ink);
                        }
                    }
                    k += 1;
                }
            }
            if sheet.show_headers {
                for col in page.cols.0.max(p.visible.c0)..=page.cols.1.min(p.visible.c1) {
                    let (x, cw) = (layout.col_x(col), layout.col_w(col));
                    if cw <= 0.0 || x + cw <= hw {
                        continue;
                    }
                    let state = sel
                        .ranges
                        .iter()
                        .filter(|range| col >= range.c0 && col <= range.c1)
                        .map(|range| if range.is_whole_cols() { 2 } else { 1 })
                        .max()
                        .unwrap_or(0);
                    let (bg, fg) = state_colors(state);
                    let left = x.max(hw);
                    p.rect(left, r, x + cw, hh, bg);
                    p.rect(x + cw - 1.0, r, x + cw, hh, border);
                    let label = p.shape(&col_to_letters(col), font_size, fg, state > 0);
                    let width = f32::from(label.width);
                    if width + 2.0 <= cw && x + (cw - width) / 2.0 >= hw {
                        p.put(
                            label,
                            x + ((cw - width) / 2.0).round(),
                            r + ((hh - r - line_h) / 2.0).round(),
                            font_size,
                        );
                    }
                    if state == 1 {
                        p.rect(left, hh - 2.0, x + cw, hh, c(theme::SELECTION));
                    }
                }
            }
        }
        p.rect(hw, hh - 1.0, w, hh, border);
    }
    if hw > 0.0 {
        p.rect(0.0, hh, hw, h, c(DESK));
        for i in row_pages.clone() {
            let page = desk.page_box(layout, i, 0);
            let (py0, py1) = (page.paper.1.max(hh), page.paper.3);
            if py1 <= hh || page.paper.1 >= h {
                continue;
            }
            if r > 0.0 {
                let (inner0, inner1) = if desk.folded {
                    (page.paper.1, page.paper.3)
                } else {
                    (page.paper.1 + desk.margin.1, page.paper.3 - desk.margin.3)
                };
                p.rect(2.0, py0, r - 2.0, py1, margin_shade);
                p.rect(2.0, inner0.max(hh), r - 2.0, inner1.max(hh), paper);
                let mut k = 0u32;
                loop {
                    let y = inner0 + k as f32 * unit;
                    if y > inner1 {
                        break;
                    }
                    if y >= hh && k > 0 {
                        if k % every == 0 {
                            let label = p.shape(&k.to_string(), 9.0, ink, false);
                            let width = f32::from(label.width);
                            if y + 6.0 < inner1 {
                                p.put(label, ((r - width) / 2.0).round(), y - 6.0, 9.0);
                            }
                        } else {
                            p.rect(r / 2.0 - 2.0, y, r / 2.0 + 2.0, y + 1.0, ink);
                        }
                    }
                    k += 1;
                }
            }
            if sheet.show_headers {
                for row in page.rows.0.max(p.visible.r0)..=page.rows.1.min(p.visible.r1) {
                    let (y, rh) = (layout.row_y(row), layout.row_h(row));
                    if rh <= 0.0 || y + rh <= hh {
                        continue;
                    }
                    let state = sel
                        .ranges
                        .iter()
                        .filter(|range| row >= range.r0 && row <= range.r1)
                        .map(|range| if range.is_whole_rows() { 2 } else { 1 })
                        .max()
                        .unwrap_or(0);
                    let (bg, fg) = state_colors(state);
                    let top = y.max(hh);
                    p.rect(r, top, hw, y + rh, bg);
                    p.rect(r, y + rh - 1.0, hw, y + rh, border);
                    if rh >= line_h * 0.7 && y + (rh - line_h) / 2.0 >= hh - 2.0 {
                        let label = p.shape(&(row + 1).to_string(), font_size, fg, state > 0);
                        let width = f32::from(label.width);
                        p.put(
                            label,
                            r + ((hw - r - width) / 2.0).round(),
                            y + ((rh - line_h) / 2.0).round(),
                            font_size,
                        );
                    }
                    if state == 1 {
                        p.rect(hw - 2.0, top, hw, y + rh, c(theme::SELECTION));
                    }
                }
            }
        }
        p.rect(hw - 1.0, hh, hw, h, border);
    }
    // The corner, and the rulers as a place where clicks do nothing.
    p.rect(0.0, 0.0, hw, hh, c(theme::HEADER_BG));
    p.rect(hw - 1.0, 0.0, hw, hh, border);
    p.rect(0.0, hh - 1.0, hw, hh, border);
    if r > 0.0 {
        p.zone((0.0, 0.0, w, r), DeskZone::Inert);
        p.zone((0.0, 0.0, r, h), DeskZone::Inert);
    }
}

// --------------------------------------------- headers and footers, typed

/// A section of a header or footer being typed on its page.
pub(crate) struct HfEdit {
    footer: bool,
    section: HfSection,
    variant: HfVariant,
    /// Page down, page across.
    page: (usize, usize),
    input: Entity<TextInput>,
    _subscription: Subscription,
}

pub(crate) fn editing_header_footer(app: &ExcelApp) -> bool {
    app.ext::<PageUi>().is_some_and(|ui| ui.hf.is_some())
}

/// The slot of the section being typed, when it holds a picture.
pub(crate) fn edited_picture(app: &ExcelApp) -> Option<HfSlot> {
    let edit = app.ext::<PageUi>()?.hf.as_ref()?;
    let slot = HfSlot {
        variant: edit.variant,
        footer: edit.footer,
        section: edit.section,
    };
    setup(app).header_footer.picture(slot).map(|_| slot)
}

/// The variant (first, odd, even) the page at (page down, page across)
/// uses; pages that do not print show the ordinary one.
fn variant_of(app: &ExcelApp, page: (usize, usize)) -> HfVariant {
    let hf = &setup(app).header_footer;
    let printed = all_pages(app).zip(pagination(app)).and_then(|(pages, pagination)| {
        let (row, col) = (pages.rows(page.0).0, pages.cols(page.1).0);
        pagination
            .pages
            .iter()
            .find(|p| p.rows.0 == row && p.cols.0 == col)
            .map(|p| hf.variant(p.index == 0, p.number))
    });
    printed.unwrap_or_else(|| hf.variant(false, 1))
}

/// The text of a section as it is typed: fields spelled out, on one line.
fn typed_text(code: &str, locale: &Locale) -> String {
    to_dialog_text(code, locale).replace('\n', &LINE_BREAK.to_string())
}

fn code_of(typed: &str, locale: &Locale) -> String {
    from_dialog_text(&typed.replace(LINE_BREAK, "\n"), locale)
}

fn section_code(hf: &HeaderFooter, variant: HfVariant, footer: bool, section: HfSection) -> String {
    let code = if footer { hf.footer(variant) } else { hf.header(variant) };
    HfSections::parse(code).get(section).to_string()
}

/// Replace one section of a header or footer.
fn set_section(hf: &mut HeaderFooter, variant: HfVariant, footer: bool, section: HfSection, code: &str) {
    let target = if footer {
        hf.footer_mut(variant)
    } else {
        hf.header_mut(variant)
    };
    let mut sections = HfSections::parse(target);
    *sections.get_mut(section) = code.to_string();
    *target = sections.to_code();
}

/// Start typing a section of the header or footer of a page.
fn begin_edit(
    app: &mut ExcelApp,
    footer: bool,
    section: HfSection,
    page: (usize, usize),
    window: &mut Window,
    cx: &mut Cx,
) {
    // What was being typed elsewhere is kept.
    store_edit(app, cx);
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    let variant = variant_of(app, page);
    let text = typed_text(
        &section_code(&setup(app).header_footer, variant, footer, section),
        app.wb.locale,
    );
    let input = cx.new(TextInput::new);
    input.update(cx, |input, cx| input.set_text(text, cx));
    let subscription = cx.subscribe_in(&input, window, |app, _, ev, window, cx| {
        on_edit_event(app, ev, window, cx)
    });
    window.focus(&input.read(cx).focus_handle(cx));
    app.ext_mut::<PageUi>().hf = Some(HfEdit {
        footer,
        section,
        variant,
        page,
        input,
        _subscription: subscription,
    });
    app.context_tab = Some(HF_TAB);
    cx.notify();
}

/// Write what was typed into the page setup (one undo step); the section
/// stays open.
fn store_edit(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(edit) = app.ext::<PageUi>().and_then(|ui| ui.hf.as_ref()) else {
        return;
    };
    let (variant, footer, section) = (edit.variant, edit.footer, edit.section);
    let code = code_of(edit.input.read(cx).text(), app.wb.locale);
    change(app, t("Header/Footer", "En-tête/Pied de page"), cx, |s| {
        set_section(&mut s.header_footer, variant, footer, section, &code)
    });
}

/// Stop typing without keeping the text (the view is left, or Escape).
pub(crate) fn drop_header_footer_edit(app: &mut ExcelApp, cx: &mut Cx) {
    if app.ext_mut::<PageUi>().hf.take().is_some() {
        if app.context_tab == Some(HF_TAB) {
            app.context_tab = None;
        }
        cx.notify();
    }
}

fn on_edit_event(app: &mut ExcelApp, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    match ev {
        InputEvent::Enter { .. } | InputEvent::Blurred => {
            store_edit(app, cx);
            drop_header_footer_edit(app, cx);
            if !matches!(ev, InputEvent::Blurred) {
                window.focus(&app.grid_focus);
            }
        }
        InputEvent::Tab { shift } => {
            // On to the next section; past the last one, back to the cells.
            let Some((footer, section, page)) = app
                .ext::<PageUi>()
                .and_then(|ui| ui.hf.as_ref())
                .map(|e| (e.footer, e.section, e.page))
            else {
                return;
            };
            let next = match (section, shift) {
                (HfSection::Left, false) | (HfSection::Right, true) => Some(HfSection::Center),
                (HfSection::Center, false) => Some(HfSection::Right),
                (HfSection::Center, true) => Some(HfSection::Left),
                _ => None,
            };
            match next {
                Some(section) => begin_edit(app, footer, section, page, window, cx),
                None => {
                    store_edit(app, cx);
                    drop_header_footer_edit(app, cx);
                    window.focus(&app.grid_focus);
                }
            }
        }
        InputEvent::Escape => {
            drop_header_footer_edit(app, cx);
            window.focus(&app.grid_focus);
        }
        _ => {}
    }
}

/// Insert > Header & Footer: go to the Page Layout view and type the
/// header of the page the active cell is on.
pub(crate) fn insert_header_footer(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    app.set_view_mode(ViewMode::PageLayout, cx);
    // The view may have been refused (frozen panes): the user is asked.
    if app.view_mode() != ViewMode::PageLayout {
        return;
    }
    let (row, col) = app.active();
    let Some(pages) = all_pages(app) else {
        return;
    };
    let page = (pages.row_page(row), pages.col_page(col));
    // The header is at the top of the page: bring it into view.
    let top = pages.rows(page.0).0;
    app.ensure_visible(top, col);
    if let Some(layout) = app.current_layout() {
        let margin = Desk::of(app, layout.zoom).map_or(0.0, |desk| desk.margin.1 + desk.gap);
        let view = &mut app.views[app.sheet];
        view.scroll_y = (layout.rows.start(top) - margin as f64).max(0.0).min(view.scroll_y);
    }
    begin_edit(app, false, HfSection::Center, page, window, cx);
}

/// Insert a field (`&P`, `&D`...) where the caret is.
pub(crate) fn insert_field(app: &mut ExcelApp, code: &str, window: &mut Window, cx: &mut Cx) {
    let Some(input) = app
        .ext::<PageUi>()
        .and_then(|ui| ui.hf.as_ref())
        .map(|e| e.input.clone())
    else {
        return;
    };
    let text = to_dialog_text(code, app.wb.locale);
    window.focus(&input.read(cx).focus_handle(cx));
    input.update(cx, |input, cx| input.insert(&text, window, cx));
    cx.notify();
}

/// "Go to Header" / "Go to Footer": the same section, at the other end
/// of the page.
pub(crate) fn go_to(app: &mut ExcelApp, footer: bool, window: &mut Window, cx: &mut Cx) {
    let Some((section, page)) = app
        .ext::<PageUi>()
        .and_then(|ui| ui.hf.as_ref())
        .map(|e| (e.section, e.page))
    else {
        return;
    };
    if let (Some(pages), Some(layout)) = (all_pages(app), app.current_layout()) {
        // Show the end of the page the section is at.
        let rows = pages.rows(page.0);
        let col = pages.cols(page.1).0;
        app.ensure_visible(if footer { rows.1 } else { rows.0 }, col);
        if let Some(desk) = Desk::of(app, layout.zoom) {
            let view = &mut app.views[app.sheet];
            if footer {
                let bottom = layout.rows.end(rows.1) + desk.margin.3 as f64 + desk.gap as f64;
                view.scroll_y = view
                    .scroll_y
                    .max(bottom - layout.view_h() as f64)
                    .min(layout.max_scroll_y());
            } else {
                let top = layout.rows.start(rows.0) - (desk.margin.1 + desk.gap) as f64;
                view.scroll_y = view.scroll_y.min(top.max(0.0));
            }
        }
    }
    begin_edit(app, footer, section, page, window, cx);
}

/// One of the options of the Header & Footer tab.
pub(crate) fn toggle_option(app: &mut ExcelApp, cx: &mut Cx, f: impl FnOnce(&mut HeaderFooter)) {
    store_edit(app, cx);
    change(app, t("Header/Footer", "En-tête/Pied de page"), cx, |s| {
        f(&mut s.header_footer)
    });
    // The page being typed may now use another header.
    let Some((footer, section, page, input)) = app
        .ext::<PageUi>()
        .and_then(|ui| ui.hf.as_ref())
        .map(|e| (e.footer, e.section, e.page, e.input.clone()))
    else {
        return;
    };
    let variant = variant_of(app, page);
    let text = typed_text(
        &section_code(&setup(app).header_footer, variant, footer, section),
        app.wb.locale,
    );
    input.update(cx, |input, cx| input.set_text(text, cx));
    if let Some(edit) = app.ext_mut::<PageUi>().hf.as_mut() {
        edit.variant = variant;
    }
}

/// The built-in headers (or footers) of the Header & Footer tab.
pub(crate) fn preset_menu(app: &ExcelApp, footer: bool) -> Vec<MenuEntry> {
    let names = file_names(app);
    let context = hf_context(
        app,
        &names,
        1,
        pagination(app).map_or(1, |p| p.pages.len().max(1) as u32),
    );
    presets(app.wb.locale, &user_name())
        .into_iter()
        .map(|code| {
            let label = if code.is_empty() {
                t("(none)", "(aucun)").to_string()
            } else {
                plain_text(&code, &context)
            };
            MenuEntry::new(label, move |app: &mut ExcelApp, _: &mut Window, cx: &mut Cx| {
                let edit = app.ext::<PageUi>().and_then(|ui| ui.hf.as_ref());
                let variant = edit.map_or(HfVariant::Odd, |e| e.variant);
                let shown = edit.map(|e| (e.footer, e.section, e.input.clone()));
                let code = code.clone();
                change(app, t("Header/Footer", "En-tête/Pied de page"), cx, |s| {
                    let hf = &mut s.header_footer;
                    *(if footer {
                        hf.footer_mut(variant)
                    } else {
                        hf.header_mut(variant)
                    }) = code;
                });
                // The section being typed shows its new text.
                if let Some((edited_footer, section, input)) = shown {
                    if edited_footer == footer {
                        let text = typed_text(
                            &section_code(&setup(app).header_footer, variant, footer, section),
                            app.wb.locale,
                        );
                        input.update(cx, |input, cx| input.set_text(text, cx));
                    }
                }
            })
        })
        .collect()
}

/// The name built-in headers sign with: the login name of the user.
pub(crate) fn user_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

/// Header & Footer > Picture: choose an image file for the section being
/// typed. The section then holds `&[Picture]`.
pub(crate) fn insert_picture(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(edit) = app.ext::<PageUi>().and_then(|ui| ui.hf.as_ref()) else {
        return;
    };
    let slot = HfSlot {
        variant: edit.variant,
        footer: edit.footer,
        section: edit.section,
    };
    // The file panel takes the keyboard: what was typed is stored first.
    store_edit(app, cx);
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = paths.await else {
            return;
        };
        let Some(data) = paths.into_iter().next().and_then(|path| std::fs::read(path).ok()) else {
            return;
        };
        this.update_in(cx, |app, _, cx| place_picture(app, slot, data, cx)).ok();
    })
    .detach();
}

/// Put an image in a section of a header or footer, at its natural size.
pub(crate) fn place_picture(app: &mut ExcelApp, slot: HfSlot, data: Vec<u8>, cx: &mut Cx) {
    let Some(info) = image_info(&data) else {
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "This file is not a picture OpenSheet can read.",
                    "Ce fichier n'est pas une image lisible par OpenSheet.",
                ),
            )
            .warning(),
            cx,
        );
        return;
    };
    let (width, height) = info.natural_size();
    let picture = HfPicture {
        slot,
        data: Arc::from(data),
        // Pixels at 96 per inch, kept in points.
        size: (width * 0.75, height * 0.75),
    };
    change(
        app,
        t("Header/Footer Picture", "Image d'en-tête/de pied de page"),
        cx,
        |s| {
            let hf = &mut s.header_footer;
            hf.pictures.retain(|p| p.slot != slot);
            hf.pictures.push(picture);
            let code = section_code(hf, slot.variant, slot.footer, slot.section);
            if !code.contains("&G") {
                set_section(hf, slot.variant, slot.footer, slot.section, &format!("{code}&G"));
            }
        },
    );
}

/// The size of the picture of a section, in the unit of the locale.
struct PictureDialog {
    modal: Modal,
    slot: HfSlot,
    /// Height over width of the picture when the dialog opened.
    ratio: f64,
    lock: bool,
    width: Entity<TextInput>,
    height: Entity<TextInput>,
}

impl PictureDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (Some(width), Some(height)) = (self.width.read(cx).number(), self.height.read(cx).number()) else {
            return;
        };
        let (slot, lock, ratio) = (self.slot, self.lock, self.ratio);
        self.modal.update_app(cx, |app, cx| {
            let locale = app.wb.locale;
            let to_pt = |v: f64| (super::from_unit(v, locale) * 72.0) as f32;
            let width = to_pt(width);
            // With the proportions locked the width decides.
            let height = if lock { width * ratio as f32 } else { to_pt(height) };
            change(app, t("Format Picture", "Format de l'image"), cx, |s| {
                if let Some(picture) = s.header_footer.pictures.iter_mut().find(|p| p.slot == slot) {
                    picture.size = (width, height);
                }
            });
        });
        self.modal.close(window, cx);
    }
}

impl Render for PictureDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Format Picture", "Format de l'image"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(labeled(
                t("Height:", "Hauteur :"),
                NumberField::new("height", &self.height).width(80.0),
            ))
            .child(labeled(
                t("Width:", "Largeur :"),
                NumberField::new("width", &self.width).width(80.0),
            ))
            .child(
                Checkbox::new("lock", t("Lock aspect ratio", "Garder les proportions"), self.lock).on_change(cx.listener(
                    |this, on: &bool, _, cx| {
                        this.lock = *on;
                        cx.notify();
                    },
                )),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// Header & Footer > Format Picture: the size of the section's picture.
pub(crate) fn format_picture(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(slot) = edited_picture(app) else {
        return;
    };
    let Some(size) = setup(app).header_footer.picture(slot).map(|p| p.size) else {
        return;
    };
    let locale = app.wb.locale;
    let shown = |pt: f32| (super::to_unit(pt as f64 / 72.0, locale) * 100.0).round() / 100.0;
    let (width, height) = (shown(size.0), shown(size.1));
    let ratio = if size.0 > 0.0 { (size.1 / size.0) as f64 } else { 1.0 };
    app.open_modal(window, cx, move |modal, _, cx| PictureDialog {
        modal,
        slot,
        ratio,
        lock: true,
        width: widgets::number_input(cx, Some(width), 0.01, 500.0, 0.1),
        height: widgets::number_input(cx, Some(height), 0.01, 500.0, 0.1),
    });
}

/// The groups of the Header & Footer tab.
pub(crate) fn header_footer_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Header & Footer", "En-tête et pied de page"),
            row(vec![
                app.cmd_large("hf-header", Cmd::HfHeader, true, cx),
                app.cmd_large("hf-footer", Cmd::HfFooter, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Header & Footer Elements", "Éléments en-tête et pied de page"),
            row(vec![
                app.cmd_large("hf-page", Cmd::HfPageNumber, false, cx),
                app.cmd_large("hf-pages", Cmd::HfPages, false, cx),
                app.cmd_large("hf-date", Cmd::HfDate, false, cx),
                app.cmd_large("hf-time", Cmd::HfTime, false, cx),
                app.cmd_large("hf-path", Cmd::HfPath, false, cx),
                app.cmd_large("hf-file", Cmd::HfFile, false, cx),
                app.cmd_large("hf-sheet", Cmd::HfSheet, false, cx),
                app.cmd_large("hf-picture", Cmd::HfPicture, false, cx),
                app.cmd_large("hf-format-picture", Cmd::HfFormatPicture, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Navigation", "Navigation"),
            row(vec![
                app.cmd_large("hf-go-header", Cmd::HfGoHeader, false, cx),
                app.cmd_large("hf-go-footer", Cmd::HfGoFooter, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Options", "Options"),
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    app.cmd_check("hf-first", Cmd::HfDifferentFirst, cx),
                    app.cmd_check("hf-odd-even", Cmd::HfDifferentOddEven, cx),
                ]))
                .child(rows(vec![
                    app.cmd_check("hf-scale", Cmd::HfScaleWithDoc, cx),
                    app.cmd_check("hf-align", Cmd::HfAlignMargins, cx),
                ])),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// The field a header or footer is typed in, over its section.
pub(crate) fn overlay_elements(app: &mut ExcelApp, layout: &GridLayout) -> Vec<AnyElement> {
    if app.view_mode() != ViewMode::PageLayout {
        return Vec::new();
    }
    let Some(edit) = app.ext::<PageUi>().and_then(|ui| ui.hf.as_ref()) else {
        return Vec::new();
    };
    let Some(desk) = Desk::of(app, layout.zoom) else {
        return Vec::new();
    };
    let hf = &setup(app).header_footer;
    let page = desk.page_box(layout, edit.page.0, edit.page.1);
    let (x0, y0, x1, y1) = desk.section_box(&page, hf, edit.footer, edit.section);
    let size = (11.0
        * if hf.scale_with_doc {
            desk.pagination.scale() as f32
        } else {
            1.0
        }
        * desk.px_per_pt)
        .clamp(8.0, 28.0);
    let line_h = (size * 1.3).round();
    let height = (y1 - y0).max(line_h + 4.0);
    // The header's field hangs from the header margin, the footer's
    // stands on the footer margin.
    let top = if edit.footer { y1 - height } else { y0 };
    let font = app
        .fonts
        .resolve(&app.wb.styles.get(opensheet_engine::StyleId(0)).font.name);
    vec![
        div()
            .absolute()
            .left(px(x0))
            .top(px(top))
            .w(px((x1 - x0).max(40.0)))
            .h(px(height))
            .occlude()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::ACCENT))
            .px(px(3.0))
            .flex()
            .when(edit.footer, |d| d.items_end())
            .when(!edit.footer, |d| d.items_start())
            .font_family(font)
            .text_size(px(size))
            .line_height(px(line_h))
            .text_color(c(0x000000))
            .child(edit.input.clone())
            .into_any_element(),
    ]
}

// ------------------------------------------------------------------ mouse

pub(crate) fn zone_mouse_down(
    app: &mut ExcelApp,
    zone: &Zone,
    ev: &ZoneEvent,
    window: &mut Window,
    cx: &mut Cx,
) -> ZoneResponse {
    if let Some(handle) = zone.payload::<Handle>() {
        // The right button opens the menu of the cell under the line.
        if ev.button != MouseButton::Left {
            return ZoneResponse::Ignored;
        }
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return ZoneResponse::Handled;
        }
        let Some(layout) = app.current_layout() else {
            return ZoneResponse::Handled;
        };
        let to = boundary_at(&layout, handle.vertical(), ev.x, ev.y);
        app.ext_mut::<PageUi>().drag = Some(BreakDrag { handle: *handle, to });
        return ZoneResponse::Capture;
    }
    if let Some(zone) = zone.payload::<HfZone>() {
        if ev.button != MouseButton::Left {
            return ZoneResponse::Handled;
        }
        let already = app
            .ext::<PageUi>()
            .and_then(|ui| ui.hf.as_ref())
            .is_some_and(|e| e.footer == zone.footer && e.section == zone.section && e.page == zone.page);
        if !already {
            begin_edit(app, zone.footer, zone.section, zone.page, window, cx);
        }
        return ZoneResponse::Handled;
    }
    match zone.payload::<DeskZone>() {
        Some(DeskZone::Between) if ev.button == MouseButton::Left => {
            store_edit(app, cx);
            drop_header_footer_edit(app, cx);
            let ui = app.ext_mut::<PageUi>();
            ui.white_space_hidden = !ui.white_space_hidden;
            let (row, col) = app.active();
            app.ensure_visible(row, col);
            cx.notify();
            ZoneResponse::Handled
        }
        Some(_) => ZoneResponse::Handled,
        None => ZoneResponse::Ignored,
    }
}

pub(crate) fn zone_drag(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, cx: &mut Cx) {
    let Some(handle) = zone.payload::<Handle>() else {
        return;
    };
    let Some(layout) = app.current_layout() else {
        return;
    };
    let to = boundary_at(&layout, handle.vertical(), ev.x, ev.y);
    app.ext_mut::<PageUi>().drag = Some(BreakDrag { handle: *handle, to });
    cx.notify();
}

pub(crate) fn zone_mouse_up(app: &mut ExcelApp, zone: &Zone, _ev: &ZoneEvent, cx: &mut Cx) {
    if zone.payload::<Handle>().is_none() {
        return;
    }
    if let Some(drag) = app.ext_mut::<PageUi>().drag.take() {
        drop_line(app, drag, cx);
    }
    cx.notify();
}

// ------------------------------------------------------------- status bar

/// "Page: 2 of 5" in the Page Layout view: the page of the active cell.
pub(crate) fn status_items(app: &ExcelApp) -> Vec<StatusItem> {
    if app.view_mode() != ViewMode::PageLayout {
        return Vec::new();
    }
    let Some(pagination) = pagination(app) else {
        return Vec::new();
    };
    let (row, col) = app.active();
    let count = pagination.pages.len().max(1);
    let page = pagination.page_of(row, col).map_or(1, |page| page.index + 1);
    vec![StatusItem::new(page_of_text(page, count))]
}

/// "Page: 2 of 5" / "Page : 2 sur 5".
pub(crate) fn page_of_text(page: usize, count: usize) -> String {
    crate::i18n::labelled(t("Page", "Page"), format!("{page} {} {count}", t("of", "sur")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_of_the_print_area() {
        let ranges = vec![Rect::new(0, 0, 9, 4), Rect::new(20, 2, 29, 6)];
        // The right edge dropped before column H: the range ends at G.
        assert_eq!(moved_edge(&ranges, 0, Edge::Right, 7)[0], Rect::new(0, 0, 9, 6));
        // An edge cannot cross the one facing it.
        assert_eq!(moved_edge(&ranges, 0, Edge::Right, 0)[0], Rect::new(0, 0, 9, 0));
        assert_eq!(moved_edge(&ranges, 1, Edge::Top, 40)[1], Rect::new(29, 2, 29, 6));
        assert_eq!(moved_edge(&ranges, 1, Edge::Left, 0)[1], Rect::new(20, 0, 29, 6));
        assert_eq!(moved_edge(&ranges, 1, Edge::Bottom, 25)[1], Rect::new(20, 2, 24, 6));
        // The other ranges are left alone, and so is everything when the
        // range does not exist.
        assert_eq!(moved_edge(&ranges, 1, Edge::Bottom, 25)[0], ranges[0]);
        assert_eq!(moved_edge(&ranges, 5, Edge::Left, 1), ranges);
    }

    #[test]
    fn headers_are_typed_on_one_line() {
        let fr = Locale::fr();
        let code = "Budget\n&P / &N";
        let typed = typed_text(code, fr);
        assert_eq!(typed, "Budget¶&[Page] / &[Pages]");
        assert_eq!(code_of(&typed, fr), code);
        let mut hf = HeaderFooter::default();
        set_section(&mut hf, HfVariant::Odd, true, HfSection::Right, "&D");
        set_section(&mut hf, HfVariant::Odd, true, HfSection::Left, "Brouillon");
        assert_eq!(hf.odd_footer, "&LBrouillon&R&D");
        assert_eq!(section_code(&hf, HfVariant::Odd, true, HfSection::Right), "&D");
        assert_eq!(section_code(&hf, HfVariant::Odd, false, HfSection::Center), "");
    }

    #[test]
    fn status_text() {
        assert_eq!(page_of_text(2, 5), "Page: 2 of 5");
    }
    #[test]
    fn pages_are_spaced_on_the_desk() {
        // Tracks of 20 pixels; the third one is 50 wide.
        let axis = Axis::new(20.0, 1000, [(2u32, 50.0f32)].into_iter());
        // Paper of 300 with margins of 30 and 40 holds 230; pages start at
        // tracks 0, 10 and 20; the desk shows 16 pixels between pages.
        let gaps = page_gaps(&axis, &[0, 10, 20], 1000, 300.0, (30.0, 40.0), 16.0, false);
        assert_eq!(gaps[0], (0, 46.0));
        // The first page holds 9 × 20 + 50 = 230: nothing is left over.
        assert_eq!(gaps[1], (10, 40.0 + 16.0 + 30.0));
        // The second holds 200: 30 are left before its margin.
        assert_eq!(gaps[2], (20, 30.0 + 40.0 + 16.0 + 30.0));
        // After the last page (too long for its paper): margin and desk.
        assert_eq!(gaps[3], (1000, 56.0));
        // Folded: no margins, a thin strip between the pages.
        let folded = page_gaps(&axis, &[0, 10, 20], 1000, 300.0, (30.0, 40.0), 16.0, true);
        assert_eq!(folded, vec![(0, 16.0), (10, FOLD), (20, FOLD), (1000, 16.0)]);
        // The grid finds the cells again through the gaps.
        let mut axis = axis;
        axis.set_gaps(&gaps);
        assert_eq!(axis.start(0), 46.0);
        assert_eq!(axis.start(10), 46.0 + 230.0 + 86.0);
        assert_eq!(axis.index_at(46.0 + 230.0 + 10.0), 10);
        assert_eq!(axis.index_at(45.0), 0);
    }
}
