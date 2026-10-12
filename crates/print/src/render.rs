//! Drawing one page of a sheet: cell fills, gridlines, borders, text,
//! row and column headings, floating objects, header and footer.
//!
//! A page is drawn pane by pane: its own cells, and the print titles
//! repeated above and left of them. Inside a pane everything is laid out
//! from the page layout computed by the engine (`Pagination::layout`), in
//! points from the top-left corner of the paper.

use crate::ObjectPainter;
use crate::fonts::{Face, FontDb};
use crate::image::Image;
use crate::scene::{Canvas, LineCap, Paint, RectF, Scene, Stroke};
use opensheet_engine::features::CellDisplay;
use opensheet_engine::format;
use opensheet_engine::page::{
    self, CELL_PADDING, HfContext, HfRun, HfSection, HfSections, HfSlot, INDENT_WIDTH, ObjectKind, Page, PageLayout,
    PageSetup, Pagination, PrintErrors, Script, Span, axis_offset, visible_col_width,
};
use opensheet_engine::refs::{MAX_COLS, Rect, col_to_letters};
use opensheet_engine::style::{Border, BorderStyle, Color, HAlign, Style, StyleId, Underline, VAlign};
use opensheet_engine::value::Value;
use opensheet_engine::workbook::{Content, Sheet, Workbook};
use std::collections::HashSet;
use std::sync::Arc;

/// What is needed to draw one page.
pub(crate) struct PageJob<'a> {
    pub wb: &'a Workbook,
    pub sheet: usize,
    pub pagination: &'a Pagination,
    pub page: &'a Page,
    pub fonts: &'a FontDb,
    pub painter: Option<&'a dyn ObjectPainter>,
    pub hf: HfContext<'a>,
    /// The first page the sheet prints (it may have a header of its own).
    pub first_of_sheet: bool,
}

/// Widths of printed lines in points at a scale of 100 %.
const GRIDLINE_WIDTH: f64 = 0.25;
const HAIR_WIDTH: f64 = 0.25;
const THIN_WIDTH: f64 = 0.5;
const MEDIUM_WIDTH: f64 = 1.0;
const THICK_WIDTH: f64 = 1.5;
/// Blank space above and below a cell's text, in pixels.
const VERTICAL_PADDING: f64 = 1.0;
/// Where the left and right sections of a header start when they do not
/// align with the margins, in inches from the paper's edge.
const HEADER_INSET: f64 = 0.7;

/// A block of rows and columns that are neighbours on the sheet and on
/// the page: the page's own cells, or repeated print titles.
struct Pane<'a> {
    cols: &'a [Span],
    rows: &'a [Span],
    /// Points per sheet pixel.
    unit: f64,
    /// Sheet position of the pane's first column and row, in pixels.
    origin_px: (f64, f64),
}

impl Pane<'_> {
    fn bounds(&self) -> RectF {
        let (c0, c1) = (self.cols[0], self.cols[self.cols.len() - 1]);
        let (r0, r1) = (self.rows[0], self.rows[self.rows.len() - 1]);
        RectF::new(c0.pos, r0.pos, c1.pos + c1.size - c0.pos, r1.pos + r1.size - r0.pos)
    }

    /// The cells of the pane as a sheet rectangle.
    fn cells(&self) -> Rect {
        Rect::new(
            self.rows[0].index,
            self.cols[0].index,
            self.rows[self.rows.len() - 1].index,
            self.cols[self.cols.len() - 1].index,
        )
    }

    fn col_index(&self, col: u32) -> Option<usize> {
        self.cols.binary_search_by_key(&col, |s| s.index).ok()
    }

    fn row_index(&self, row: u32) -> Option<usize> {
        self.rows.binary_search_by_key(&row, |s| s.index).ok()
    }

    /// Left edge of a column, also for columns beside the pane (where a
    /// merged area or overflowing text starts or ends).
    fn col_x(&self, sheet: &Sheet, col: u32) -> f64 {
        match self.col_index(col) {
            Some(j) => self.cols[j].pos,
            None => self.cols[0].pos + (axis_offset(sheet, false, col) - self.origin_px.0) * self.unit,
        }
    }

    fn row_y(&self, sheet: &Sheet, row: u32) -> f64 {
        match self.row_index(row) {
            Some(i) => self.rows[i].pos,
            None => self.rows[0].pos + (axis_offset(sheet, true, row) - self.origin_px.1) * self.unit,
        }
    }

    /// The paper rectangle of a block of cells.
    fn rect(&self, sheet: &Sheet, cells: Rect) -> RectF {
        let x = self.col_x(sheet, cells.c0);
        let y = self.row_y(sheet, cells.r0);
        RectF::new(x, y, self.col_x(sheet, cells.c1 + 1) - x, self.row_y(sheet, cells.r1 + 1) - y)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
    Right,
}

/// One line of a cell's text, placed.
struct TextLine {
    text: String,
    x: f64,
    baseline: f64,
    width: f64,
    /// Extra space added after every word space (justified text).
    word_gap: f64,
}

/// The text of a cell, laid out.
struct CellText {
    lines: Vec<TextLine>,
    face: Arc<Face>,
    size: f64,
    color: Color,
    underline: Underline,
    strike: bool,
    clip: RectF,
    /// Centre and angle (degrees, counter-clockwise) of rotated text; the
    /// lines are then placed relative to the centre.
    rotation: Option<(f64, f64, f64)>,
    /// The row and the first and last column the text covers, when it
    /// flows out of its own cell.
    covers: Option<(u32, u32, u32)>,
}

/// Break text into lines no wider than `max_width`, at spaces where
/// possible. Each line says whether it ends a paragraph.
fn wrap_text(fonts: &FontDb, face: &Arc<Face>, size: f64, text: &str, max_width: f64) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph = paragraph.trim_end_matches('\r');
        let mut line = String::new();
        let mut width = 0.0;
        // Byte position in `line` of the last space, where it can break.
        let mut last_space: Option<usize> = None;
        for ch in paragraph.chars() {
            let w = fonts.char_width(face, ch, size);
            if width + w > max_width + 1e-6 && !line.is_empty() {
                if ch == ' ' {
                    // The space at the break is not printed.
                    out.push((std::mem::take(&mut line), false));
                    width = 0.0;
                    last_space = None;
                    continue;
                }
                match last_space.take() {
                    Some(at) => {
                        let rest = line[at + 1..].to_string();
                        line.truncate(at);
                        out.push((line.trim_end().to_string(), false));
                        width = fonts.text_width(face, &rest, size);
                        line = rest;
                    }
                    None => {
                        out.push((std::mem::take(&mut line), false));
                        width = 0.0;
                    }
                }
            }
            if ch == ' ' {
                last_space = Some(line.len());
            }
            line.push(ch);
            width += w;
        }
        out.push((line.trim_end().to_string(), true));
    }
    out
}

/// How heavy a border is, to decide which of two borders on the same
/// edge shows.
fn border_weight(style: BorderStyle) -> u8 {
    match style {
        BorderStyle::None => 0,
        BorderStyle::Hair => 1,
        BorderStyle::Dotted => 2,
        BorderStyle::Dashed => 3,
        BorderStyle::Thin => 4,
        BorderStyle::Medium => 5,
        BorderStyle::Double => 6,
        BorderStyle::Thick => 7,
    }
}

fn heavier(a: Border, b: Border) -> Border {
    if border_weight(b.style) > border_weight(a.style) { b } else { a }
}

struct Renderer<'a> {
    job: &'a PageJob<'a>,
    sh: &'a Sheet,
    setup: &'a PageSetup,
    layout: PageLayout,
    /// The print scale: 1.0 = 100 %.
    scale: f64,
}

impl<'a> Renderer<'a> {
    fn ink(&self, color: Color) -> Color {
        if self.setup.black_and_white { Color::BLACK } else { color }
    }

    fn display(&self, row: u32, col: u32) -> CellDisplay<'a> {
        self.job.wb.display(self.job.sheet, row, col)
    }

    // ------------------------------------------------------------- text

    /// Lay out the text of the cell at (row, col), which occupies `cell`
    /// on paper (a whole merged area when `merged`).
    fn plan_text(&self, pane: &Pane, row: u32, col: u32, cell: RectF, merged: bool, style: &Style) -> Option<CellText> {
        let wb = self.job.wb;
        let sh = self.sh;
        let value = sh.value_at(row, col);
        if value.is_empty() {
            return None;
        }
        let formatted = wb.display_text(self.job.sheet, row, col);
        let mut text = formatted.text;
        if matches!(value, Value::Error(_)) {
            match self.setup.errors {
                PrintErrors::Displayed => {}
                PrintErrors::Blank => return None,
                PrintErrors::Dash => text = "--".to_string(),
                PrintErrors::NA => text = wb.locale.error_name(opensheet_engine::ErrorKind::NA).to_string(),
            }
        }
        if text.is_empty() {
            return None;
        }
        let fonts = self.job.fonts;
        let face = fonts.face_for(&style.font);
        let mut size = style.font.size_pt() as f64 * self.scale;
        let unit = pane.unit;
        let pad = CELL_PADDING as f64 * unit;
        let vpad = VERTICAL_PADDING * unit;
        let indent = style.indent as f64 * INDENT_WIDTH as f64 * unit;
        let is_number = matches!(value, Value::Number(_));
        let is_text = matches!(value, Value::Text(_));
        let mut cell = cell;
        let align = match style.h_align {
            HAlign::Left | HAlign::Fill | HAlign::Justify => Align::Left,
            HAlign::Center | HAlign::CenterAcross => Align::Center,
            HAlign::Right => Align::Right,
            HAlign::General => match value {
                Value::Number(_) => Align::Right,
                Value::Bool(_) | Value::Error(_) => Align::Center,
                _ => Align::Left,
            },
        };
        let mut covers = None;
        // Centre across selection: the cell and the blank cells after it
        // that are formatted the same way act as one.
        if style.h_align == HAlign::CenterAcross && !merged {
            let mut end = col;
            while end + 1 < MAX_COLS
                && sh.is_blank(row, end + 1)
                && sh.merge_at(row, end + 1).is_none()
                && self.display(row, end + 1).style.h_align == HAlign::CenterAcross
                && pane.col_x(sh, end + 1) < pane.bounds().right()
            {
                end += 1;
            }
            if end > col {
                cell.w = pane.col_x(sh, end + 1) - cell.x;
                covers = Some((row, col, end));
            }
        }
        let available = (cell.w - 2.0 * pad - indent).max(0.0);
        let color = self.ink(formatted.color.or(style.font.color).unwrap_or(Color::BLACK));
        let line_height = |size: f64| (face.ascent() + face.descent()) * size;
        let line_x = |align: Align, cell: &RectF, width: f64| match align {
            Align::Left => cell.x + pad + indent,
            Align::Right => cell.right() - pad - indent - width,
            Align::Center => cell.x + (cell.w - width) / 2.0,
        };
        let mut planned = CellText {
            lines: Vec::new(),
            face: face.clone(),
            size,
            color,
            underline: style.font.underline,
            strike: style.font.strike,
            clip: cell,
            rotation: None,
            covers,
        };

        // Wrapped text: several lines inside the cell.
        if (style.wrap || style.h_align == HAlign::Justify) && !is_number && style.rotation == 0 {
            let lines = wrap_text(fonts, &face, size, &text, available.max(size));
            let height = line_height(size);
            let block = height * lines.len() as f64;
            let top = match style.v_align {
                VAlign::Top => cell.y + vpad,
                VAlign::Center => cell.y + (cell.h - block) / 2.0,
                VAlign::Bottom => cell.bottom() - vpad - block,
            };
            for (i, (line, ends_paragraph)) in lines.into_iter().enumerate() {
                let width = fonts.text_width(&face, &line, size);
                let spaces = line.matches(' ').count();
                // Justified lines stretch to both edges, except the last
                // of each paragraph.
                let word_gap = if style.h_align == HAlign::Justify && !ends_paragraph && spaces > 0 && width < available {
                    (available - width) / spaces as f64
                } else {
                    0.0
                };
                planned.lines.push(TextLine {
                    x: line_x(align, &cell, width),
                    baseline: top + height * i as f64 + face.ascent() * size,
                    width: width + word_gap * spaces as f64,
                    text: line,
                    word_gap,
                });
            }
            return Some(planned);
        }

        if text.contains(['\n', '\r']) {
            text = text.replace(['\n', '\r'], " ");
        }
        let mut width = fonts.text_width(&face, &text, size);
        if style.shrink_to_fit && width > available && width > 0.0 && style.rotation == 0 {
            size *= available / width;
            width = available;
            planned.size = size;
        }
        if style.h_align == HAlign::Fill && width > 0.0 && width <= available {
            // Fill: the text repeats as many whole times as fit.
            let times = (available / width).floor().max(1.0) as usize;
            text = text.repeat(times);
            width *= times as f64;
        }
        // Rotated text takes the room of its slanted box.
        let angle = style.rotation.clamp(-90, 90) as f64;
        let (sin, cos) = (angle.to_radians().sin().abs(), angle.to_radians().cos().abs());
        let height = line_height(size);
        let (box_w, box_h) = (width * cos + height * sin, width * sin + height * cos);
        if box_w > available + 1e-6 {
            if is_number && angle == 0.0 {
                // Numbers never spill: they lose digits, then show ###.
                let digit = fonts.char_width(&face, '0', size).max(0.01);
                let mut fitted = None;
                if let (true, Value::Number(n)) = (style.is_general(), &value) {
                    let mut chars = ((available / digit).floor() as usize).min(11);
                    while chars >= 1 && fitted.is_none() {
                        let candidate = format::format_general(*n, wb.locale, chars);
                        if fonts.text_width(&face, &candidate, size) <= available + 1e-6 {
                            fitted = Some(candidate);
                        }
                        chars -= 1;
                    }
                }
                text = fitted.unwrap_or_else(|| {
                    let hash = fonts.char_width(&face, '#', size).max(0.01);
                    "#".repeat((available / hash + 1e-6).floor() as usize)
                });
                width = fonts.text_width(&face, &text, size);
            } else if is_text && !merged && covers.is_none() && style.h_align != HAlign::Fill {
                // Text flows over blank neighbours, as far as it needs.
                let bounds = pane.bounds();
                let blank = |c: u32| sh.is_blank(row, c) && sh.merge_at(row, c).is_none();
                let (mut first, mut last) = (col, col);
                let mut clip = cell;
                if align != Align::Right {
                    let needed = match align {
                        Align::Left => cell.x + pad + indent + box_w + pad,
                        _ => cell.x + (cell.w + box_w) / 2.0 + pad,
                    };
                    let mut edge = cell.right();
                    while edge < needed - 1e-6 && edge < bounds.right() && last + 1 < MAX_COLS && blank(last + 1) {
                        last += 1;
                        edge += visible_col_width(sh, last) * unit;
                    }
                    clip.w = edge - clip.x;
                }
                if align != Align::Left {
                    let needed = match align {
                        Align::Right => cell.right() - pad - indent - box_w - pad,
                        _ => cell.x + (cell.w - box_w) / 2.0 - pad,
                    };
                    let mut edge = cell.x;
                    while edge > needed + 1e-6 && edge > bounds.x && first > 0 && blank(first - 1) {
                        first -= 1;
                        edge -= visible_col_width(sh, first) * unit;
                    }
                    clip.w += clip.x - edge;
                    clip.x = edge;
                }
                planned.clip = clip;
                if first < last {
                    planned.covers = Some((row, first, last));
                }
            }
        }
        if angle == 0.0 {
            let baseline = match style.v_align {
                VAlign::Top => cell.y + vpad + face.ascent() * size,
                VAlign::Center => cell.y + (cell.h - height) / 2.0 + face.ascent() * size,
                VAlign::Bottom => cell.bottom() - vpad - face.descent() * size,
            };
            planned.lines.push(TextLine {
                x: line_x(align, &cell, width),
                baseline,
                width,
                text,
                word_gap: 0.0,
            });
        } else {
            let left = line_x(align, &cell, box_w);
            let top = match style.v_align {
                VAlign::Top => cell.y + vpad,
                VAlign::Center => cell.y + (cell.h - box_h) / 2.0,
                VAlign::Bottom => cell.bottom() - vpad - box_h,
            };
            planned.rotation = Some((left + box_w / 2.0, top + box_h / 2.0, angle));
            planned.lines.push(TextLine {
                x: -width / 2.0,
                baseline: -height / 2.0 + face.ascent() * size,
                width,
                text,
                word_gap: 0.0,
            });
        }
        Some(planned)
    }

    fn draw_text(&self, canvas: &mut Canvas, text: &CellText) {
        canvas.save();
        canvas.clip_rect(text.clip);
        if let Some((cx, cy, angle)) = text.rotation {
            canvas.translate(cx, cy);
            canvas.rotate(angle);
        }
        let paint = Paint::from(text.color);
        for line in &text.lines {
            if line.word_gap > 0.0 {
                let mut x = line.x;
                for (i, word) in line.text.split(' ').enumerate() {
                    if i > 0 {
                        x += self.job.fonts.char_width(&text.face, ' ', text.size) + line.word_gap;
                    }
                    x += canvas.face_text(&text.face, word, x, line.baseline, text.size, paint);
                }
            } else {
                canvas.face_text(&text.face, &line.text, line.x, line.baseline, text.size, paint);
            }
            decorate(canvas, &text.face, text.size, paint, text.underline, text.strike, line.x, line.baseline, line.width);
        }
        canvas.restore();
    }

    // ------------------------------------------------------------- panes

    fn draw_pane(&self, canvas: &mut Canvas, cols: &[Span], rows: &[Span]) {
        if cols.is_empty() || rows.is_empty() {
            return;
        }
        let sh = self.sh;
        let pane = Pane {
            cols,
            rows,
            unit: self.layout.unit,
            origin_px: (axis_offset(sh, false, cols[0].index), axis_offset(sh, true, rows[0].index)),
        };
        let bounds = pane.bounds();
        let area = pane.cells();
        let (nr, nc) = (rows.len(), cols.len());
        let merges: Vec<Rect> = sh.merges.iter().filter(|m| m.intersects(&area)).copied().collect();
        let merge_of = |row: u32, col: u32| -> Option<usize> { merges.iter().position(|m| m.contains(row, col)) };

        // The look of every cell of the pane, and the merged area it is in.
        let mut cells: Vec<(CellDisplay<'a>, Option<usize>)> = Vec::with_capacity(nr * nc);
        for row in rows {
            for col in cols {
                cells.push((self.display(row.index, col.index), merge_of(row.index, col.index)));
            }
        }
        let cell = |i: usize, j: usize| &cells[i * nc + j];
        let merge_styles: Vec<CellDisplay<'a>> = merges.iter().map(|m| self.display(m.r0, m.c0)).collect();
        let cell_rect = |i: usize, j: usize| RectF::new(cols[j].pos, rows[i].pos, cols[j].size, rows[i].size);

        canvas.save();
        canvas.clip_rect(bounds);

        // Fills. Neighbours of one colour are painted as one rectangle,
        // so that no seam shows between them.
        if !self.setup.black_and_white {
            // Runs of the previous row still growing: columns, colour, top.
            let mut open: Vec<(usize, usize, Color, f64)> = Vec::new();
            for i in 0..=nr {
                let mut runs: Vec<(usize, usize, Color)> = Vec::new();
                if i < nr {
                    for j in 0..nc {
                        let (display, merge) = cell(i, j);
                        let Some(color) = display.style.fill.filter(|_| merge.is_none()) else {
                            continue;
                        };
                        match runs.last_mut() {
                            Some((_, last, c)) if *last + 1 == j && *c == color => *last = j,
                            _ => runs.push((j, j, color)),
                        }
                    }
                }
                let y = if i < nr { rows[i].pos } else { bounds.bottom() };
                let mut next: Vec<(usize, usize, Color, f64)> = Vec::new();
                for (j0, j1, color, top) in open.drain(..) {
                    if runs.iter().any(|r| *r == (j0, j1, color)) {
                        next.push((j0, j1, color, top));
                    } else {
                        let x = cols[j0].pos;
                        canvas.fill_rect(RectF::new(x, top, cols[j1].pos + cols[j1].size - x, y - top), color);
                    }
                }
                for (j0, j1, color) in runs {
                    if !next.iter().any(|n| (n.0, n.1, n.2) == (j0, j1, color)) {
                        next.push((j0, j1, color, y));
                    }
                }
                open = next;
            }
            for (m, display) in merges.iter().zip(&merge_styles) {
                if let Some(color) = display.style.fill {
                    canvas.fill_rect(pane.rect(sh, *m), color);
                }
            }
        }

        // What conditional formats draw inside cells (data bars, icons)
        // is the application's to paint.
        if let Some(painter) = self.job.painter {
            for i in 0..nr {
                for j in 0..nc {
                    let (display, merge) = cell(i, j);
                    if let (Some(adornment), None) = (&display.adornment, merge) {
                        let r = cell_rect(i, j);
                        let depth = canvas.depth();
                        canvas.save();
                        canvas.clip_rect(r);
                        canvas.translate(r.x, r.y);
                        canvas.scale(pane.unit, pane.unit);
                        let size = (r.w / pane.unit, r.h / pane.unit);
                        painter.paint_adornment(self.job.wb, self.job.sheet, (rows[i].index, cols[j].index), adornment, size, canvas);
                        canvas.restore_to(depth);
                    }
                }
            }
        }

        // Text is laid out before the gridlines are drawn: where it flows
        // over blank neighbours, the lines it crosses do not print.
        let mut texts: Vec<CellText> = Vec::new();
        for i in 0..nr {
            for j in 0..nc {
                let (display, merge) = cell(i, j);
                if merge.is_none() {
                    texts.extend(self.plan_text(&pane, rows[i].index, cols[j].index, cell_rect(i, j), false, &display.style));
                }
            }
        }
        for (m, display) in merges.iter().zip(&merge_styles) {
            texts.extend(self.plan_text(&pane, m.r0, m.c0, pane.rect(sh, *m), true, &display.style));
        }
        // Text of cells beside the pane may flow into it.
        let holds_text = |cell: &opensheet_engine::Cell| matches!(cell.content, Content::Value(_) | Content::Formula(_));
        for row in rows {
            let before = sh.cells.range((row.index, 0)..(row.index, area.c0)).rev().find(|(_, c)| holds_text(c));
            let after = sh.cells.range((row.index, area.c1 + 1)..=(row.index, u32::MAX)).find(|(_, c)| holds_text(c));
            for ((r, c), _) in before.into_iter().chain(after) {
                if sh.merge_at(*r, *c).is_some() || visible_col_width(sh, *c) <= 0.0 {
                    continue;
                }
                let display = self.display(*r, *c);
                let own = pane.rect(sh, Rect::cell(*r, *c));
                if let Some(text) = self.plan_text(&pane, *r, *c, own, false, &display.style) {
                    if text.covers.is_some() && text.clip.intersects(&bounds) {
                        texts.push(text);
                    }
                }
            }
        }
        // (row index, column index) of cells whose right edge is crossed.
        let mut crossed: HashSet<(usize, usize)> = HashSet::new();
        for text in &texts {
            if let Some((row, first, last)) = text.covers {
                if let Some(i) = pane.row_index(row) {
                    for col in first..last {
                        if let Some(j) = pane.col_index(col) {
                            crossed.insert((i, j));
                        }
                    }
                }
            }
        }
        canvas.restore();

        let same_merge = |a: Option<usize>, b: Option<usize>| a.is_some() && a == b;
        if self.setup.gridlines && !self.setup.draft {
            let stroke = || Stroke::solid(Color::BLACK, GRIDLINE_WIDTH * self.scale);
            // Horizontal lines: above every row and below the last.
            for i in 0..=nr {
                let y = if i < nr { rows[i].pos } else { bounds.bottom() };
                let mut start: Option<usize> = None;
                for j in 0..=nc {
                    let hidden = j == nc || (i > 0 && i < nr && same_merge(cell(i - 1, j).1, cell(i, j).1));
                    match (hidden, start) {
                        (false, None) => start = Some(j),
                        (true, Some(s)) => {
                            canvas.line(cols[s].pos, y, cols[j - 1].pos + cols[j - 1].size, y, stroke());
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
            // Vertical lines: left of every column and right of the last.
            for j in 0..=nc {
                let x = if j < nc { cols[j].pos } else { bounds.right() };
                let mut start: Option<usize> = None;
                for i in 0..=nr {
                    let hidden = i == nr
                        || (j > 0 && j < nc && (same_merge(cell(i, j - 1).1, cell(i, j).1) || crossed.contains(&(i, j - 1))));
                    match (hidden, start) {
                        (false, None) => start = Some(i),
                        (true, Some(s)) => {
                            canvas.line(x, rows[s].pos, x, rows[i - 1].pos + rows[i - 1].size, stroke());
                            start = None;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Borders. An edge shared by two cells shows the heavier of
        // their two borders; edges inside a merged area do not show.
        let outside = |row: Option<u32>, col: Option<u32>| -> Option<CellDisplay<'a>> {
            Some(self.display(row?, col?)).filter(|d| !d.style.borders.is_none())
        };
        let mut edges: Vec<(Border, f64, f64, f64, f64)> = Vec::new();
        for i in 0..=nr {
            let y = if i < nr { rows[i].pos } else { bounds.bottom() };
            // A run of segments with one border: border, first column.
            let mut run: Option<(Border, usize)> = None;
            for j in 0..=nc {
                let mut border = Border::NONE;
                if j < nc {
                    let above = if i > 0 {
                        cell(i - 1, j).0.style.borders.bottom
                    } else {
                        outside(rows[0].index.checked_sub(1), Some(cols[j].index)).map_or(Border::NONE, |d| d.style.borders.bottom)
                    };
                    let below = if i < nr {
                        cell(i, j).0.style.borders.top
                    } else {
                        outside(Some(rows[nr - 1].index + 1), Some(cols[j].index)).map_or(Border::NONE, |d| d.style.borders.top)
                    };
                    if !(i > 0 && i < nr && same_merge(cell(i - 1, j).1, cell(i, j).1)) {
                        border = heavier(above, below);
                    }
                }
                if let Some((b, s)) = run {
                    if b != border {
                        edges.push((b, cols[s].pos, y, cols[j - 1].pos + cols[j - 1].size, y));
                        run = None;
                    }
                }
                if run.is_none() && !border.is_none() {
                    run = Some((border, j));
                }
            }
        }
        for j in 0..=nc {
            let x = if j < nc { cols[j].pos } else { bounds.right() };
            let mut run: Option<(Border, usize)> = None;
            for i in 0..=nr {
                let mut border = Border::NONE;
                if i < nr {
                    let left = if j > 0 {
                        cell(i, j - 1).0.style.borders.right
                    } else {
                        outside(Some(rows[i].index), cols[0].index.checked_sub(1)).map_or(Border::NONE, |d| d.style.borders.right)
                    };
                    let right = if j < nc {
                        cell(i, j).0.style.borders.left
                    } else {
                        outside(Some(rows[i].index), Some(cols[nc - 1].index + 1)).map_or(Border::NONE, |d| d.style.borders.left)
                    };
                    if !(j > 0 && j < nc && same_merge(cell(i, j - 1).1, cell(i, j).1)) {
                        border = heavier(left, right);
                    }
                }
                if let Some((b, s)) = run {
                    if b != border {
                        edges.push((b, x, rows[s].pos, x, rows[i - 1].pos + rows[i - 1].size));
                        run = None;
                    }
                }
                if run.is_none() && !border.is_none() {
                    run = Some((border, i));
                }
            }
        }
        // Light borders first, so that heavy ones stay whole where they
        // cross.
        edges.sort_by_key(|e| border_weight(e.0.style));
        for (border, x0, y0, x1, y1) in edges {
            self.draw_border(canvas, border, x0, y0, x1, y1);
        }

        canvas.save();
        canvas.clip_rect(bounds);
        for text in &texts {
            self.draw_text(canvas, text);
        }
        canvas.restore();
    }

    fn draw_border(&self, canvas: &mut Canvas, border: Border, x0: f64, y0: f64, x1: f64, y1: f64) {
        let s = self.scale;
        let color = self.ink(border.color.unwrap_or(Color::BLACK));
        // Solid lines end square so that corners close.
        let solid = |width: f64| Stroke::solid(color, width * s).cap(LineCap::Square);
        let stroke = match border.style {
            BorderStyle::None => return,
            BorderStyle::Hair => Stroke::solid(color, HAIR_WIDTH * s).dashed(&[0.5 * s, 0.5 * s]),
            BorderStyle::Dotted => Stroke::solid(color, THIN_WIDTH * s).dashed(&[THIN_WIDTH * s, THIN_WIDTH * s]),
            BorderStyle::Dashed => Stroke::solid(color, THIN_WIDTH * s).dashed(&[2.0 * s, s]),
            BorderStyle::Thin => solid(THIN_WIDTH),
            BorderStyle::Medium => solid(MEDIUM_WIDTH),
            BorderStyle::Thick => solid(THICK_WIDTH),
            BorderStyle::Double => {
                // Two thin lines, one on each side of the edge.
                let d = THIN_WIDTH * s;
                let (dx, dy) = if y0 == y1 { (0.0, d) } else { (d, 0.0) };
                canvas.line(x0 - dx, y0 - dy, x1 - dx, y1 - dy, solid(THIN_WIDTH));
                canvas.line(x0 + dx, y0 + dy, x1 + dx, y1 + dy, solid(THIN_WIDTH));
                return;
            }
        };
        canvas.line(x0, y0, x1, y1, stroke);
    }

    // ---------------------------------------------------------- headings

    /// The row numbers and column letters around the cells.
    fn draw_headings(&self, canvas: &mut Canvas) {
        let (hw, hh) = self.layout.heading;
        if hw <= 0.0 || hh <= 0.0 {
            return;
        }
        let (bx, by, bw, bh) = self.layout.block;
        let font = &self.job.wb.styles.get(StyleId(0)).font;
        let face = self.job.fonts.face_for(font);
        let size = font.size_pt() as f64 * self.scale;
        let stroke = || Stroke::solid(Color::BLACK, GRIDLINE_WIDTH * self.scale);
        let label = |canvas: &mut Canvas, text: &str, r: RectF| {
            let width = self.job.fonts.text_width(&face, text, size);
            let baseline = r.y + (r.h - (face.ascent() + face.descent()) * size) / 2.0 + face.ascent() * size;
            canvas.save();
            canvas.clip_rect(r);
            canvas.face_text(&face, text, r.x + (r.w - width) / 2.0, baseline, size, Paint::BLACK);
            canvas.restore();
        };
        canvas.line(bx, by, bx + bw, by, stroke());
        canvas.line(bx, by + hh, bx + bw, by + hh, stroke());
        canvas.line(bx, by, bx, by + bh, stroke());
        canvas.line(bx + hw, by, bx + hw, by + bh, stroke());
        for col in &self.layout.cols {
            label(canvas, &col_to_letters(col.index), RectF::new(col.pos, by, col.size, hh));
            canvas.line(col.pos + col.size, by, col.pos + col.size, by + hh, stroke());
        }
        for row in &self.layout.rows {
            label(canvas, &(row.index + 1).to_string(), RectF::new(bx, row.pos, hw, row.size));
            canvas.line(bx, row.pos + row.size, bx + hw, row.pos + row.size, stroke());
        }
    }

    // ----------------------------------------------------------- objects

    /// Charts, pictures and shapes over the page's own cells.
    fn draw_objects(&self, canvas: &mut Canvas) {
        let Some(painter) = self.job.painter else {
            return;
        };
        let (x, y, w, h) = self.layout.body;
        let body = RectF::new(x, y, w, h);
        let unit = self.layout.unit;
        for object in page::floating_objects(self.sh) {
            let (px, py, pw, ph) = object.sheet_box(self.sh);
            let (ox, oy) = self.layout.to_paper(px, py);
            if !RectF::new(ox, oy, pw * unit, ph * unit).intersects(&body) {
                continue;
            }
            let depth = canvas.depth();
            canvas.save();
            canvas.clip_rect(body);
            canvas.translate(ox, oy);
            canvas.scale(unit, unit);
            match object.kind {
                ObjectKind::Chart => {
                    if let Some(chart) = self.sh.charts.iter().find(|c| c.id == object.id) {
                        painter.paint_chart(self.job.wb, self.job.sheet, chart, (pw, ph), canvas);
                    }
                }
                ObjectKind::Drawing => {
                    if let Some(drawing) = self.sh.drawings.iter().find(|d| d.id == object.id) {
                        painter.paint_drawing(self.job.wb, self.job.sheet, drawing, (pw, ph), canvas);
                    }
                }
            }
            canvas.restore_to(depth);
        }
    }

    // ------------------------------------------------- header and footer

    fn draw_header_footer(&self, canvas: &mut Canvas) {
        let hf = &self.setup.header_footer;
        let variant = hf.variant(self.job.first_of_sheet, self.job.hf.page);
        let (paper_w, paper_h) = self.job.pagination.paper;
        let margins = &self.setup.margins;
        let scale = if hf.scale_with_doc { self.scale } else { 1.0 };
        let (left, right) = if hf.align_with_margins {
            (margins.left * 72.0, paper_w - margins.right * 72.0)
        } else {
            (HEADER_INSET * 72.0, paper_w - HEADER_INSET * 72.0)
        };
        let default_font = self.job.wb.styles.get(StyleId(0)).font.name.clone();
        for footer in [false, true] {
            let code = if footer { hf.footer(variant) } else { hf.header(variant) };
            if code.is_empty() {
                continue;
            }
            let sections = HfSections::parse(code);
            for (section, align) in
                [(HfSection::Left, Align::Left), (HfSection::Center, Align::Center), (HfSection::Right, Align::Right)]
            {
                let source = sections.get(section);
                if source.is_empty() {
                    continue;
                }
                let picture = hf
                    .picture(HfSlot {
                        variant,
                        footer,
                        section,
                    })
                    .and_then(|p| Image::from_file_bytes(&p.data).map(|image| (Arc::new(image), p.size)));
                // Lay the lines out: pieces with their width, and the
                // height of each line.
                let mut lines: Vec<(Vec<Piece>, f64, f64)> = Vec::new();
                for line in page::lines(page::expand(source, &self.job.hf)) {
                    let mut pieces = Vec::new();
                    let (mut ascent, mut descent) = (0.0f64, 0.0f64);
                    for run in line {
                        match run {
                            HfRun::Picture => {
                                if let Some((image, (w, h))) = &picture {
                                    let (w, h) = (*w as f64 * scale, *h as f64 * scale);
                                    pieces.push(Piece::Picture(image.clone(), w, h));
                                }
                            }
                            HfRun::Text { text, format } => {
                                let family = format.font.as_deref().unwrap_or(&default_font);
                                let face = self.job.fonts.face(family, format.bold, format.italic);
                                let full = format.size as f64 * scale;
                                // Superscripts and subscripts are smaller
                                // and off the baseline.
                                let (size, rise) = match format.script {
                                    Script::Normal => (full, 0.0),
                                    Script::Superscript => (full * 0.65, full * 0.35),
                                    Script::Subscript => (full * 0.65, -full * 0.12),
                                };
                                ascent = ascent.max(face.ascent() * full);
                                descent = descent.max(face.descent() * full);
                                let width = self.job.fonts.text_width(&face, &text, size);
                                pieces.push(Piece::Text {
                                    text,
                                    face,
                                    size,
                                    rise,
                                    width,
                                    color: self.ink(format.color.unwrap_or(Color::BLACK)),
                                    underline: format.underline,
                                    strike: format.strike,
                                });
                            }
                        }
                    }
                    if ascent == 0.0 {
                        // A blank line is as tall as default text.
                        let face = self.job.fonts.face(&default_font, false, false);
                        (ascent, descent) = (face.ascent() * 11.0 * scale, face.descent() * 11.0 * scale);
                    }
                    lines.push((pieces, ascent, descent));
                }
                let total: f64 = lines.iter().map(|(_, a, d)| a + d).sum();
                // Headers hang from the header margin; footers stand on
                // the footer margin.
                let mut y = if footer { paper_h - margins.footer * 72.0 - total } else { margins.header * 72.0 };
                for (pieces, ascent, descent) in lines {
                    let width: f64 = pieces.iter().map(Piece::width).sum();
                    let mut x = match align {
                        Align::Left => left,
                        Align::Center => (left + right - width) / 2.0,
                        Align::Right => right - width,
                    };
                    let baseline = y + ascent;
                    for piece in pieces {
                        match piece {
                            Piece::Picture(image, w, h) => {
                                // A picture hangs from the top of its line
                                // (it may reach over the cells, as a
                                // watermark does).
                                canvas.image(&image, RectF::new(x, y, w, h));
                                x += w;
                            }
                            Piece::Text { text, face, size, rise, width, color, underline, strike } => {
                                let paint = Paint::from(color);
                                canvas.face_text(&face, &text, x, baseline - rise, size, paint);
                                decorate(canvas, &face, size, paint, underline, strike, x, baseline - rise, width);
                                x += width;
                            }
                        }
                    }
                    y += ascent + descent;
                }
            }
        }
    }
}

/// A piece of a header or footer line.
enum Piece {
    Text {
        text: String,
        face: Arc<Face>,
        size: f64,
        /// How far above the baseline the text sits.
        rise: f64,
        width: f64,
        color: Color,
        underline: Underline,
        strike: bool,
    },
    Picture(Arc<Image>, f64, f64),
}

impl Piece {
    fn width(&self) -> f64 {
        match self {
            Piece::Text { width, .. } => *width,
            Piece::Picture(_, w, _) => *w,
        }
    }
}

/// Underline and strike a line of text.
#[allow(clippy::too_many_arguments)]
fn decorate(
    canvas: &mut Canvas,
    face: &Face,
    size: f64,
    paint: Paint,
    underline: Underline,
    strike: bool,
    x: f64,
    baseline: f64,
    width: f64,
) {
    if width <= 0.0 {
        return;
    }
    let thickness = (face.underline_thickness() * size).max(0.3);
    let stroke = || Stroke::solid(paint, thickness);
    if underline != Underline::None {
        let y = baseline + face.underline_offset() * size;
        canvas.line(x, y, x + width, y, stroke());
        if underline == Underline::Double {
            canvas.line(x, y + 2.0 * thickness, x + width, y + 2.0 * thickness, stroke());
        }
    }
    if strike {
        let y = baseline - face.strikeout_offset() * size;
        canvas.line(x, y, x + width, y, Stroke::solid(paint, (face.strikeout_thickness() * size).max(0.3)));
    }
}

/// Draw a page.
pub(crate) fn render(job: &PageJob) -> Scene {
    let size = job.pagination.paper;
    let mut canvas = Canvas::new(job.fonts);
    let Some(sh) = job.wb.sheets.get(job.sheet) else {
        return Scene { size, ops: Vec::new() };
    };
    let renderer = Renderer {
        job,
        sh,
        setup: &sh.page,
        layout: job.pagination.layout(sh, job.page),
        scale: job.pagination.scale(),
    };
    canvas.set_black_and_white(sh.page.black_and_white);
    let layout = &renderer.layout;
    let (title_cols, own_cols) = layout.cols.split_at(layout.title_cols);
    let (title_rows, own_rows) = layout.rows.split_at(layout.title_rows);
    for rows in [title_rows, own_rows] {
        for cols in [title_cols, own_cols] {
            renderer.draw_pane(&mut canvas, cols, rows);
        }
    }
    renderer.draw_headings(&mut canvas);
    if !sh.page.draft {
        renderer.draw_objects(&mut canvas);
    }
    renderer.draw_header_footer(&mut canvas);
    Scene {
        size,
        ops: canvas.into_ops(),
    }
}
