//! Editing operations on a workbook: copy / paste, autofill, inserting and
//! deleting rows and columns, sorting, merging, borders, navigation, find.

use crate::ast::{Expr, StructuralEdit};
use crate::eval::compare_values;
use crate::format;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Border, StyleId};
use crate::value::Value;
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ClipCell {
    pub content: Content,
    pub value: Value,
    pub style: StyleId,
}

/// A copied block of cells.
#[derive(Clone, Debug)]
pub struct Clip {
    pub sheet: usize,
    /// Source rectangle (clamped to the used area for whole rows / columns).
    pub rect: Rect,
    /// Row-major, `rect.rows() * rect.cols()` entries.
    pub cells: Vec<Option<ClipCell>>,
    /// Merged areas, relative to the top-left of `rect`.
    pub merges: Vec<Rect>,
    /// Tab-separated text of the displayed values (for the system clipboard).
    pub text: String,
}

impl Clip {
    fn get(&self, r: u32, c: u32) -> Option<&ClipCell> {
        self.cells[(r * self.rect.cols() + c) as usize].as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteMode {
    All,
    Values,
    Formulas,
    Formats,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderPreset {
    None,
    All,
    Outside,
    ThickOutside,
    Bottom,
    Top,
    Left,
    Right,
    TopAndBottom,
    BottomDouble,
    ThickBottom,
    Inside,
}

/// Aggregates shown in the status bar for the current selection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SelectionStats {
    /// Non-blank cells.
    pub count: usize,
    pub numeric_count: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

impl SelectionStats {
    pub fn average(&self) -> Option<f64> {
        (self.numeric_count > 0).then(|| self.sum / self.numeric_count as f64)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FindOptions {
    pub match_case: bool,
    pub whole_cell: bool,
    /// Search formula text instead of displayed values.
    pub in_formulas: bool,
}

fn shift_keys<V: Clone>(map: &BTreeMap<u32, V>, insert: bool, at: u32, count: u32, max: u32) -> BTreeMap<u32, V> {
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if insert {
            let nk = if *k >= at { *k + count } else { *k };
            if nk < max {
                out.insert(nk, v.clone());
            }
        } else if *k < at {
            out.insert(*k, v.clone());
        } else if *k >= at + count {
            out.insert(*k - count, v.clone());
        }
    }
    out
}

fn shift_set(set: &BTreeSet<u32>, insert: bool, at: u32, count: u32, max: u32) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for k in set {
        if insert {
            let nk = if *k >= at { *k + count } else { *k };
            if nk < max {
                out.insert(nk);
            }
        } else if *k < at {
            out.insert(*k);
        } else if *k >= at + count {
            out.insert(*k - count);
        }
    }
    out
}

fn shift_interval(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32)> {
    if insert {
        let nlo = if lo >= at { lo + count } else { lo };
        let nhi = if hi >= at { hi + count } else { hi };
        if nlo >= max {
            return None;
        }
        Some((nlo, nhi.min(max - 1)))
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

/// Split trailing digits off text: "Item 9" → ("Item ", 9, 1 digit).
fn split_trailing_number(s: &str) -> Option<(&str, u64, usize)> {
    let digits = s.chars().rev().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 15 {
        return None;
    }
    let split = s.len() - digits;
    Some((&s[..split], s[split..].parse().ok()?, digits))
}

impl Workbook {
    /// Clamp whole-row / whole-column rectangles to the used area.
    pub fn clamp_to_used(&self, sheet: usize, rect: Rect) -> Rect {
        let Some(sh) = self.sheets.get(sheet) else {
            return rect;
        };
        let used = sh.used_rect().unwrap_or(Rect::cell(0, 0));
        let mut out = rect;
        if rect.is_whole_cols() {
            out.r1 = used.r1.max(rect.r0);
        }
        if rect.is_whole_rows() {
            out.c1 = used.c1.max(rect.c0);
        }
        out
    }

    // ---- copy / paste ---------------------------------------------------

    pub fn copy(&self, sheet: usize, rect: Rect) -> Clip {
        let rect = self.clamp_to_used(sheet, rect);
        let sh = &self.sheets[sheet];
        let mut cells = Vec::with_capacity((rect.rows() * rect.cols()) as usize);
        let mut text = String::new();
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                if c > rect.c0 {
                    text.push('\t');
                }
                let value = sh.value_at(r, c);
                let style = sh.style_id_at(r, c);
                if !value.is_empty() {
                    let shown = format::format_value(&value, &self.styles.get(style).num_fmt, self.locale).text;
                    if shown.contains(['\t', '\n', '"']) {
                        text.push('"');
                        text.push_str(&shown.replace('"', "\"\""));
                        text.push('"');
                    } else {
                        text.push_str(&shown);
                    }
                }
                let cell = match sh.cells.get(&(r, c)) {
                    Some(cell) => Some(ClipCell {
                        content: cell.content.clone(),
                        value: cell.value.clone(),
                        style: cell.style,
                    }),
                    None if !value.is_empty() || style != StyleId(0) => Some(ClipCell {
                        content: Content::Empty,
                        value,
                        style,
                    }),
                    None => None,
                };
                cells.push(cell);
            }
            if r < rect.r1 {
                text.push('\n');
            }
        }
        let merges = sh
            .merges
            .iter()
            .filter(|m| rect.contains(m.r0, m.c0) && rect.contains(m.r1, m.c1))
            .map(|m| Rect::new(m.r0 - rect.r0, m.c0 - rect.c0, m.r1 - rect.r0, m.c1 - rect.c0))
            .collect();
        Clip {
            sheet,
            rect,
            cells,
            merges,
            text,
        }
    }

    /// Paste a clip at `dest`. When `dest` is a multiple of the clip size the
    /// clip is tiled over it. Returns the rectangle written.
    pub fn paste(&mut self, sheet: usize, dest: Rect, clip: &Clip, mode: PasteMode) -> Rect {
        let (ch, cw) = (clip.rect.rows(), clip.rect.cols());
        let dest = self.clamp_to_used(sheet, dest);
        let (mut th, mut tw) = (ch, cw);
        if dest.rows() % ch == 0 && dest.cols() % cw == 0 {
            th = dest.rows();
            tw = dest.cols();
        }
        let th = th.min(MAX_ROWS - dest.r0);
        let tw = tw.min(MAX_COLS - dest.c0);
        let out = Rect::new(dest.r0, dest.c0, dest.r0 + th - 1, dest.c0 + tw - 1);

        if mode == PasteMode::All && !clip.merges.is_empty() || self.sheets[sheet].merges.iter().any(|m| m.intersects(&out)) {
            self.snapshot();
            self.sheets[sheet].merges.retain(|m| !m.intersects(&out));
            if mode == PasteMode::All {
                for tr in (0..th).step_by(ch as usize) {
                    for tc in (0..tw).step_by(cw as usize) {
                        for m in &clip.merges {
                            let r = Rect::new(
                                out.r0 + tr + m.r0,
                                out.c0 + tc + m.c0,
                                out.r0 + tr + m.r1,
                                out.c0 + tc + m.c1,
                            );
                            if r.r1 < MAX_ROWS && r.c1 < MAX_COLS {
                                self.sheets[sheet].merges.push(r);
                            }
                        }
                    }
                }
            }
        }

        for dr in 0..th {
            for dc in 0..tw {
                let (row, col) = (out.r0 + dr, out.c0 + dc);
                let src = clip.get(dr % ch, dc % cw);
                let d_row = row as i64 - (clip.rect.r0 + dr % ch) as i64;
                let d_col = col as i64 - (clip.rect.c0 + dc % cw) as i64;
                let existing = self.sheets[sheet].cells.get(&(row, col)).cloned();
                let cur_style = self.sheets[sheet].style_id_at(row, col);
                let new = match (mode, src) {
                    (PasteMode::All, None) => None,
                    (PasteMode::All, Some(s)) => Some(Cell {
                        content: shift_content(&s.content, &s.value, d_row, d_col),
                        value: s.value.clone(),
                        style: s.style,
                    }),
                    (PasteMode::Formulas, src) => Some(Cell {
                        content: src
                            .map(|s| shift_content(&s.content, &s.value, d_row, d_col))
                            .unwrap_or(Content::Empty),
                        value: src.map(|s| s.value.clone()).unwrap_or_default(),
                        style: cur_style,
                    }),
                    (PasteMode::Values, src) => {
                        let v = src.map(|s| s.value.clone()).unwrap_or_default();
                        Some(Cell {
                            content: if v.is_empty() {
                                Content::Empty
                            } else {
                                Content::Value(v.clone())
                            },
                            value: v,
                            style: cur_style,
                        })
                    }
                    (PasteMode::Formats, src) => Some(Cell {
                        content: existing.as_ref().map(|c| c.content.clone()).unwrap_or(Content::Empty),
                        value: existing.as_ref().map(|c| c.value.clone()).unwrap_or_default(),
                        style: src.map(|s| s.style).unwrap_or_default(),
                    }),
                };
                self.put_cell(sheet, row, col, new);
            }
        }
        out
    }

    /// Move a block (cut + paste): cells keep their formulas unchanged and
    /// formulas elsewhere that pointed into the block follow it.
    pub fn move_range(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) -> Rect {
        let src = self.clamp_to_used(src_sheet, src);
        let d_row = dest_row as i64 - src.r0 as i64;
        let d_col = dest_col as i64 - src.c0 as i64;
        let rows = src.rows().min(MAX_ROWS - dest_row);
        let cols = src.cols().min(MAX_COLS - dest_col);
        let out = Rect::new(dest_row, dest_col, dest_row + rows - 1, dest_col + cols - 1);
        if src_sheet == dest_sheet && d_row == 0 && d_col == 0 {
            return out;
        }
        self.snapshot();
        let src_name = self.sheets[src_sheet].name.clone();
        let mut moved: Vec<((u32, u32), Cell)> = Vec::new();
        for r in src.r0..=src.r1 {
            for c in src.c0..=src.c1 {
                if let Some(cell) = self.sheets[src_sheet].cells.remove(&(r, c)) {
                    moved.push(((r, c), cell));
                }
            }
        }
        // Clear the destination, then drop the block there.
        for r in out.r0..=out.r1 {
            for c in out.c0..=out.c1 {
                self.sheets[dest_sheet].cells.remove(&(r, c));
            }
        }
        for ((r, c), cell) in moved {
            let (nr, nc) = (r as i64 + d_row, c as i64 + d_col);
            if nr >= 0 && nc >= 0 && (nr as u32) < MAX_ROWS && (nc as u32) < MAX_COLS {
                self.sheets[dest_sheet].cells.insert((nr as u32, nc as u32), cell);
            }
        }
        // Merges travel with the block.
        let merges: Vec<Rect> = self.sheets[src_sheet]
            .merges
            .iter()
            .filter(|m| src.contains(m.r0, m.c0) && src.contains(m.r1, m.c1))
            .copied()
            .collect();
        self.sheets[src_sheet]
            .merges
            .retain(|m| !(src.contains(m.r0, m.c0) && src.contains(m.r1, m.c1)));
        self.sheets[dest_sheet].merges.retain(|m| !m.intersects(&out));
        for m in merges {
            self.sheets[dest_sheet].merges.push(Rect::new(
                (m.r0 as i64 + d_row) as u32,
                (m.c0 as i64 + d_col) as u32,
                (m.r1 as i64 + d_row) as u32,
                (m.c1 as i64 + d_col) as u32,
            ));
        }
        // Formulas referring to the moved block follow it.
        if src_sheet == dest_sheet {
            for sh in &mut self.sheets {
                let own = sh.name.clone();
                for cell in sh.cells.values_mut() {
                    if let Content::Formula(e) = &cell.content {
                        let moved = e.with_moved_range(&src_name, &own, src, d_row, d_col);
                        if moved != **e {
                            cell.content = Content::Formula(Arc::new(moved));
                        }
                    }
                }
            }
        }
        out
    }

    /// Paste tab-separated text (from another application). Returns the
    /// rectangle written.
    pub fn paste_text(&mut self, sheet: usize, row: u32, col: u32, text: &str) -> Rect {
        let rows = parse_tsv(text);
        let mut max_cols = 1;
        for (dr, line) in rows.iter().enumerate() {
            max_cols = max_cols.max(line.len());
            for (dc, field) in line.iter().enumerate() {
                let (r, c) = (row + dr as u32, col + dc as u32);
                if r >= MAX_ROWS || c >= MAX_COLS {
                    continue;
                }
                if self.set_cell_text(sheet, r, c, field).is_err() {
                    self.set_content(sheet, r, c, Content::Value(Value::text(field)));
                }
            }
        }
        Rect::new(
            row,
            col,
            (row + rows.len().max(1) as u32 - 1).min(MAX_ROWS - 1),
            (col + max_cols as u32 - 1).min(MAX_COLS - 1),
        )
    }

    // ---- autofill -------------------------------------------------------

    /// Extend `src` into `dest` (which must contain `src` and extend it
    /// along one axis), continuing series the way the fill handle does.
    pub fn autofill(&mut self, sheet: usize, src: Rect, dest: Rect) {
        if dest == src || sheet >= self.sheets.len() {
            return;
        }
        let vertical = dest.c0 == src.c0 && dest.c1 == src.c1;
        let forward = if vertical { dest.r1 > src.r1 } else { dest.c1 > src.c1 };
        let lanes = if vertical { src.cols() } else { src.rows() };
        let n = if vertical { src.rows() } else { src.cols() } as i64;
        let targets: Vec<i64> = if vertical {
            if forward {
                (src.r1 as i64 + 1..=dest.r1 as i64).collect()
            } else {
                (dest.r0 as i64..src.r0 as i64).rev().collect()
            }
        } else if forward {
            (src.c1 as i64 + 1..=dest.c1 as i64).collect()
        } else {
            (dest.c0 as i64..src.c0 as i64).rev().collect()
        };

        for lane in 0..lanes {
            let pos = |i: i64| -> (u32, u32) {
                if vertical {
                    ((src.r0 as i64 + i) as u32, src.c0 + lane)
                } else {
                    (src.r0 + lane, (src.c0 as i64 + i) as u32)
                }
            };
            let source: Vec<Option<Cell>> = (0..n)
                .map(|i| {
                    let (r, c) = pos(i);
                    self.sheets[sheet].cells.get(&(r, c)).cloned()
                })
                .collect();

            // Pure numeric constants → linear series.
            let numbers: Option<Vec<f64>> = source
                .iter()
                .map(|c| match c {
                    Some(Cell {
                        content: Content::Value(Value::Number(x)),
                        ..
                    }) => Some(*x),
                    _ => None,
                })
                .collect();
            let is_date = source.first().and_then(|c| c.as_ref()).is_some_and(|c| {
                format::is_date_format(&self.styles.get(c.style).num_fmt)
            });
            let linear = match &numbers {
                Some(v) if v.len() >= 2 => {
                    // Least-squares fit, as Excel does.
                    let len = v.len() as f64;
                    let mean_x = (len - 1.0) / 2.0;
                    let mean_y = v.iter().sum::<f64>() / len;
                    let mut num = 0.0;
                    let mut den = 0.0;
                    for (i, y) in v.iter().enumerate() {
                        num += (i as f64 - mean_x) * (y - mean_y);
                        den += (i as f64 - mean_x).powi(2);
                    }
                    let slope = if den == 0.0 { 0.0 } else { num / den };
                    Some((mean_y - slope * mean_x, slope))
                }
                Some(v) if v.len() == 1 && is_date => Some((v[0], 1.0)),
                _ => None,
            };

            for (k, t) in targets.iter().enumerate() {
                // Index along the lane relative to the source start.
                let idx = if forward { n + k as i64 } else { -(k as i64) - 1 };
                let (row, col) = if vertical {
                    (*t as u32, src.c0 + lane)
                } else {
                    (src.r0 + lane, *t as u32)
                };
                let si = idx.rem_euclid(n) as usize;
                let cycle = idx.div_euclid(n);
                let Some(model) = &source[si] else {
                    self.put_cell(sheet, row, col, None);
                    continue;
                };
                let mut cell = model.clone();
                if let Some((intercept, slope)) = linear {
                    let v = intercept + slope * idx as f64;
                    // Remove float noise from the fit.
                    let v = (v * 1e10).round() / 1e10;
                    cell.content = Content::Value(Value::Number(v));
                    cell.value = Value::Number(v);
                } else {
                    match &model.content {
                        Content::Formula(e) => {
                            let (sr, sc) = pos(si as i64);
                            let shifted = e.shifted(row as i64 - sr as i64, col as i64 - sc as i64);
                            cell.content = Content::Formula(Arc::new(shifted));
                        }
                        Content::Value(Value::Text(s)) => {
                            if let Some(next) = self.next_in_list(s, cycle * n + (si as i64) - si as i64, idx - si as i64, n) {
                                cell.content = Content::Value(Value::text(&next));
                                cell.value = Value::text(&next);
                            } else if let Some((prefix, num, digits)) = split_trailing_number(s) {
                                // Step between consecutive numbered items in
                                // the source (default 1).
                                let step = self.text_series_step(&source, si).unwrap_or(1);
                                let v = num as i64 + step * cycle * self.numbered_count(&source).max(1) as i64;
                                if v >= 0 {
                                    let text = format!("{prefix}{:0width$}", v, width = digits);
                                    cell.content = Content::Value(Value::text(&text));
                                    cell.value = Value::text(&text);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                self.put_cell(sheet, row, col, Some(cell));
            }
        }
    }

    fn numbered_count(&self, source: &[Option<Cell>]) -> usize {
        source
            .iter()
            .filter(|c| match c {
                Some(Cell {
                    content: Content::Value(Value::Text(s)),
                    ..
                }) => split_trailing_number(s).is_some(),
                _ => false,
            })
            .count()
    }

    fn text_series_step(&self, source: &[Option<Cell>], _si: usize) -> Option<i64> {
        let nums: Vec<i64> = source
            .iter()
            .filter_map(|c| match c {
                Some(Cell {
                    content: Content::Value(Value::Text(s)),
                    ..
                }) => split_trailing_number(s).map(|(_, n, _)| n as i64),
                _ => None,
            })
            .collect();
        if nums.len() >= 2 {
            Some(nums[1] - nums[0])
        } else {
            None
        }
    }

    /// Continue a built-in list (weekday / month names): the entry `offset`
    /// places after `s`.
    fn next_in_list(&self, s: &str, _unused: i64, offset: i64, _n: i64) -> Option<String> {
        let lists: [&[&str]; 4] = [
            &self.locale.days,
            &self.locale.days_short,
            &self.locale.months,
            &self.locale.months_short,
        ];
        let lower = s.trim_end_matches('.').to_lowercase();
        for list in lists {
            if let Some(i) = list.iter().position(|x| x.to_lowercase() == lower) {
                let len = list.len() as i64;
                let j = (i as i64 + offset).rem_euclid(len) as usize;
                let name = list[j];
                // Preserve the capitalisation style of the source.
                let out = if s.chars().all(|c| !c.is_lowercase()) {
                    name.to_uppercase()
                } else if s.chars().next().is_some_and(|c| c.is_uppercase()) {
                    let mut ch = name.chars();
                    match ch.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                        None => String::new(),
                    }
                } else {
                    name.to_string()
                };
                return Some(out);
            }
        }
        None
    }

    /// Copy the first row of `rect` into the rows below (Ctrl+D).
    pub fn fill_down(&mut self, sheet: usize, rect: Rect) {
        if rect.rows() < 2 {
            return;
        }
        let clip = self.copy(sheet, Rect::new(rect.r0, rect.c0, rect.r0, rect.c1));
        self.paste(sheet, Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1), &clip, PasteMode::All);
    }

    /// Copy the first column of `rect` into the columns to the right (Ctrl+R).
    pub fn fill_right(&mut self, sheet: usize, rect: Rect) {
        if rect.cols() < 2 {
            return;
        }
        let clip = self.copy(sheet, Rect::new(rect.r0, rect.c0, rect.r1, rect.c0));
        self.paste(sheet, Rect::new(rect.r0, rect.c0 + 1, rect.r1, rect.c1), &clip, PasteMode::All);
    }

    // ---- rows & columns -------------------------------------------------

    fn structural(&mut self, sheet: usize, edit: StructuralEdit) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        let (row_axis, insert, at, count) = match edit {
            StructuralEdit::InsertRows { at, count } => (true, true, at, count),
            StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
            StructuralEdit::InsertCols { at, count } => (false, true, at, count),
            StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
        };
        if count == 0 {
            return;
        }
        let edited_name = self.sheets[sheet].name.clone();
        {
            let sh: &mut Sheet = &mut self.sheets[sheet];
            let old = std::mem::take(&mut sh.cells);
            for ((r, c), cell) in old {
                let key = if row_axis { r } else { c };
                let max = if row_axis { MAX_ROWS } else { MAX_COLS };
                let nk = if insert {
                    if key >= at { key + count } else { key }
                } else if key < at {
                    key
                } else if key >= at + count {
                    key - count
                } else {
                    continue;
                };
                if nk >= max {
                    continue;
                }
                let pos = if row_axis { (nk, c) } else { (r, nk) };
                sh.cells.insert(pos, cell);
            }
            if row_axis {
                sh.row_heights = shift_keys(&sh.row_heights, insert, at, count, MAX_ROWS);
                sh.row_styles = shift_keys(&sh.row_styles, insert, at, count, MAX_ROWS);
                sh.hidden_rows = shift_set(&sh.hidden_rows, insert, at, count, MAX_ROWS);
            } else {
                sh.col_widths = shift_keys(&sh.col_widths, insert, at, count, MAX_COLS);
                sh.col_styles = shift_keys(&sh.col_styles, insert, at, count, MAX_COLS);
                sh.hidden_cols = shift_set(&sh.hidden_cols, insert, at, count, MAX_COLS);
            }
            let merges = std::mem::take(&mut sh.merges);
            for m in merges {
                let adjusted = if row_axis {
                    shift_interval(m.r0, m.r1, insert, at, count, MAX_ROWS).map(|(a, b)| Rect::new(a, m.c0, b, m.c1))
                } else {
                    shift_interval(m.c0, m.c1, insert, at, count, MAX_COLS).map(|(a, b)| Rect::new(m.r0, a, m.r1, b))
                };
                if let Some(r) = adjusted {
                    if !r.is_single() {
                        sh.merges.push(r);
                    }
                }
            }
            // New rows / columns take the formatting of the one before.
            if insert && at > 0 {
                if row_axis {
                    let above: Vec<(u32, StyleId)> = sh
                        .cells
                        .range((at - 1, 0)..=(at - 1, u32::MAX))
                        .filter(|(_, cell)| cell.style != StyleId(0))
                        .map(|((_, c), cell)| (*c, cell.style))
                        .collect();
                    for i in 0..count {
                        for (c, style) in &above {
                            sh.cells.insert(
                                (at + i, *c),
                                Cell {
                                    content: Content::Empty,
                                    value: Value::Empty,
                                    style: *style,
                                },
                            );
                        }
                    }
                    if let Some(h) = sh.row_styles.get(&(at - 1)).copied() {
                        for i in 0..count {
                            sh.row_styles.insert(at + i, h);
                        }
                    }
                } else {
                    let left: Vec<(u32, StyleId)> = sh
                        .cells
                        .iter()
                        .filter(|((_, c), cell)| *c == at - 1 && cell.style != StyleId(0))
                        .map(|((r, _), cell)| (*r, cell.style))
                        .collect();
                    for i in 0..count {
                        for (r, style) in &left {
                            sh.cells.insert(
                                (*r, at + i),
                                Cell {
                                    content: Content::Empty,
                                    value: Value::Empty,
                                    style: *style,
                                },
                            );
                        }
                    }
                    if let Some(w) = sh.col_widths.get(&(at - 1)).copied() {
                        for i in 0..count {
                            sh.col_widths.insert(at + i, w);
                        }
                    }
                    if let Some(s) = sh.col_styles.get(&(at - 1)).copied() {
                        for i in 0..count {
                            sh.col_styles.insert(at + i, s);
                        }
                    }
                }
            }
        }
        // Rewrite formulas everywhere.
        for sh in &mut self.sheets {
            let own = sh.name.clone();
            for cell in sh.cells.values_mut() {
                if let Content::Formula(e) = &cell.content {
                    let adjusted = e.adjusted(edit, &edited_name, &own);
                    if adjusted != **e {
                        cell.content = Content::Formula(Arc::new(adjusted));
                    }
                }
            }
        }
        for def in self.names.values_mut() {
            def.expr = Arc::new(def.expr.adjusted(edit, &edited_name, &edited_name));
        }
        for sh in &mut self.sheets {
            let own = sh.name.clone();
            for def in sh.names.values_mut() {
                def.expr = Arc::new(def.expr.adjusted(edit, &edited_name, &own));
            }
        }
        for sh in &mut self.sheets {
            let own = sh.name.clone();
            for chart in &mut sh.charts {
                chart.adjust(edit, &edited_name, &own);
            }
            crate::features::on_structural_edit(sh, edit, &edited_name);
        }
    }

    pub fn insert_rows(&mut self, sheet: usize, at: u32, count: u32) {
        self.structural(sheet, StructuralEdit::InsertRows { at, count });
    }

    pub fn delete_rows(&mut self, sheet: usize, at: u32, count: u32) {
        self.structural(sheet, StructuralEdit::DeleteRows { at, count });
    }

    pub fn insert_cols(&mut self, sheet: usize, at: u32, count: u32) {
        self.structural(sheet, StructuralEdit::InsertCols { at, count });
    }

    pub fn delete_cols(&mut self, sheet: usize, at: u32, count: u32) {
        self.structural(sheet, StructuralEdit::DeleteCols { at, count });
    }

    pub fn set_rows_hidden(&mut self, sheet: usize, r0: u32, r1: u32, hidden: bool) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        for r in r0..=r1 {
            if hidden {
                self.sheets[sheet].hidden_rows.insert(r);
            } else {
                self.sheets[sheet].hidden_rows.remove(&r);
            }
        }
    }

    pub fn set_cols_hidden(&mut self, sheet: usize, c0: u32, c1: u32, hidden: bool) {
        if sheet >= self.sheets.len() {
            return;
        }
        self.snapshot();
        for c in c0..=c1 {
            if hidden {
                self.sheets[sheet].hidden_cols.insert(c);
            } else {
                self.sheets[sheet].hidden_cols.remove(&c);
            }
        }
    }

    // ---- merge ----------------------------------------------------------

    /// Merge a rectangle into one cell, keeping only the top-left content.
    pub fn merge(&mut self, sheet: usize, rect: Rect) {
        if rect.is_single() || sheet >= self.sheets.len() {
            return;
        }
        let rect = self.clamp_to_used(sheet, rect);
        self.snapshot();
        self.sheets[sheet].merges.retain(|m| !m.intersects(&rect));
        // Keep the first non-empty value (reading order) in the top-left.
        let mut keep: Option<Cell> = None;
        let keys: Vec<(u32, u32)> = self.sheets[sheet]
            .cells
            .range((rect.r0, 0)..=(rect.r1, u32::MAX))
            .filter(|((_, c), _)| *c >= rect.c0 && *c <= rect.c1)
            .map(|(k, _)| *k)
            .collect();
        for k in &keys {
            let cell = &self.sheets[sheet].cells[k];
            if keep.is_none() && !matches!(cell.content, Content::Empty) {
                keep = Some(cell.clone());
            }
        }
        let style = self.sheets[sheet].style_id_at(rect.r0, rect.c0);
        for (r, c) in keys {
            if (r, c) != (rect.r0, rect.c0) {
                let st = self.sheets[sheet].cells[&(r, c)].style;
                self.put_cell(
                    sheet,
                    r,
                    c,
                    Some(Cell {
                        content: Content::Empty,
                        value: Value::Empty,
                        style: st,
                    }),
                );
            }
        }
        if let Some(mut k) = keep {
            k.style = style;
            self.put_cell(sheet, rect.r0, rect.c0, Some(k));
        }
        self.sheets[sheet].merges.push(rect);
    }

    /// Remove every merged area touching `rect`. Returns true if any existed.
    pub fn unmerge(&mut self, sheet: usize, rect: Rect) -> bool {
        if sheet >= self.sheets.len() || !self.sheets[sheet].merges.iter().any(|m| m.intersects(&rect)) {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].merges.retain(|m| !m.intersects(&rect));
        true
    }

    // ---- borders --------------------------------------------------------

    /// Apply one of the ribbon's border presets to a rectangle.
    pub fn apply_borders(&mut self, sheet: usize, rect: Rect, preset: BorderPreset, line: Border) {
        let rect = self.clamp_to_used(sheet, rect);
        let thick = Border {
            style: crate::style::BorderStyle::Thick,
            color: line.color,
        };
        let double = Border {
            style: crate::style::BorderStyle::Double,
            color: line.color,
        };
        for r in rect.r0..=rect.r1 {
            for c in rect.c0..=rect.c1 {
                let top_edge = r == rect.r0;
                let bottom_edge = r == rect.r1;
                let left_edge = c == rect.c0;
                let right_edge = c == rect.c1;
                let id = self.sheets[sheet].style_id_at(r, c);
                let mut s = self.styles.get(id).clone();
                let b = &mut s.borders;
                match preset {
                    BorderPreset::None => {
                        *b = Default::default();
                    }
                    BorderPreset::All => {
                        b.left = line;
                        b.right = line;
                        b.top = line;
                        b.bottom = line;
                    }
                    BorderPreset::Outside | BorderPreset::ThickOutside => {
                        let l = if preset == BorderPreset::ThickOutside { thick } else { line };
                        if top_edge {
                            b.top = l;
                        }
                        if bottom_edge {
                            b.bottom = l;
                        }
                        if left_edge {
                            b.left = l;
                        }
                        if right_edge {
                            b.right = l;
                        }
                    }
                    BorderPreset::Inside => {
                        if !top_edge {
                            b.top = line;
                        }
                        if !bottom_edge {
                            b.bottom = line;
                        }
                        if !left_edge {
                            b.left = line;
                        }
                        if !right_edge {
                            b.right = line;
                        }
                    }
                    BorderPreset::Bottom if bottom_edge => b.bottom = line,
                    BorderPreset::Top if top_edge => b.top = line,
                    BorderPreset::Left if left_edge => b.left = line,
                    BorderPreset::Right if right_edge => b.right = line,
                    BorderPreset::TopAndBottom => {
                        if top_edge {
                            b.top = line;
                        }
                        if bottom_edge {
                            b.bottom = line;
                        }
                    }
                    BorderPreset::BottomDouble if bottom_edge => b.bottom = double,
                    BorderPreset::ThickBottom if bottom_edge => b.bottom = thick,
                    _ => {}
                }
                let new = self.styles.intern(s);
                if new != id {
                    self.set_style_id(sheet, r, c, new);
                }
            }
        }
        // Clearing borders also clears the touching edges of the neighbours.
        if preset == BorderPreset::None {
            let clear_edge = |wb: &mut Workbook, r: u32, c: u32, f: &dyn Fn(&mut crate::style::Borders)| {
                let id = wb.sheets[sheet].style_id_at(r, c);
                let mut s = wb.styles.get(id).clone();
                f(&mut s.borders);
                let new = wb.styles.intern(s);
                if new != id {
                    wb.set_style_id(sheet, r, c, new);
                }
            };
            for c in rect.c0..=rect.c1 {
                if rect.r0 > 0 {
                    clear_edge(self, rect.r0 - 1, c, &|b| b.bottom = Border::NONE);
                }
                if rect.r1 + 1 < MAX_ROWS {
                    clear_edge(self, rect.r1 + 1, c, &|b| b.top = Border::NONE);
                }
            }
            for r in rect.r0..=rect.r1 {
                if rect.c0 > 0 {
                    clear_edge(self, r, rect.c0 - 1, &|b| b.right = Border::NONE);
                }
                if rect.c1 + 1 < MAX_COLS {
                    clear_edge(self, r, rect.c1 + 1, &|b| b.left = Border::NONE);
                }
            }
        }
    }

    // ---- sort -----------------------------------------------------------

    /// Sort the rows of `rect` by the values in column `key_col`.
    pub fn sort_rows(&mut self, sheet: usize, rect: Rect, key_col: u32, ascending: bool, has_header: bool) {
        let rect = self.clamp_to_used(sheet, rect);
        let r0 = if has_header { rect.r0 + 1 } else { rect.r0 };
        if r0 >= rect.r1 || sheet >= self.sheets.len() {
            return;
        }
        let sh = &self.sheets[sheet];
        let mut order: Vec<u32> = (r0..=rect.r1).collect();
        let keys: Vec<Value> = order.iter().map(|r| sh.value_at(*r, key_col)).collect();
        let base = r0;
        order.sort_by(|a, b| {
            let (x, y) = (&keys[(*a - base) as usize], &keys[(*b - base) as usize]);
            // Blanks always sort last.
            match (x.is_empty(), y.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => {
                    let o = compare_values(x, y);
                    if ascending { o } else { o.reverse() }
                }
            }
        });
        let rows: Vec<Vec<Option<Cell>>> = order
            .iter()
            .map(|r| {
                (rect.c0..=rect.c1)
                    .map(|c| self.sheets[sheet].cells.get(&(*r, c)).cloned())
                    .collect()
            })
            .collect();
        for (i, (src_row, cells)) in order.iter().zip(rows).enumerate() {
            let dest_row = r0 + i as u32;
            for (j, cell) in cells.into_iter().enumerate() {
                let col = rect.c0 + j as u32;
                let cell = cell.map(|mut c| {
                    if let Content::Formula(e) = &c.content {
                        c.content = Content::Formula(Arc::new(e.shifted(dest_row as i64 - *src_row as i64, 0)));
                    }
                    c
                });
                self.put_cell(sheet, dest_row, col, cell);
            }
        }
    }

    // ---- navigation -----------------------------------------------------

    /// Where Ctrl+Arrow lands: the edge of the current data block, or the
    /// next non-blank cell, or the edge of the sheet.
    pub fn data_edge(&self, sheet: usize, row: u32, col: u32, dir: Direction) -> (u32, u32) {
        let Some(sh) = self.sheets.get(sheet) else {
            return (row, col);
        };
        let step = |r: u32, c: u32| -> Option<(u32, u32)> {
            match dir {
                Direction::Up => r.checked_sub(1).map(|r| (r, c)),
                Direction::Down => (r + 1 < MAX_ROWS).then_some((r + 1, c)),
                Direction::Left => c.checked_sub(1).map(|c| (r, c)),
                Direction::Right => (c + 1 < MAX_COLS).then_some((r, c + 1)),
            }
        };
        let Some(next) = step(row, col) else {
            return (row, col);
        };
        if !sh.is_blank(row, col) && !sh.is_blank(next.0, next.1) {
            // Walk to the end of the contiguous block.
            let mut cur = next;
            while let Some(n) = step(cur.0, cur.1) {
                if sh.is_blank(n.0, n.1) {
                    break;
                }
                cur = n;
            }
            return cur;
        }
        // Jump to the next non-blank cell in that direction.
        let (er, ec) = sh.content_extent();
        let found = match dir {
            Direction::Right => (col + 1..=ec).find(|c| !sh.is_blank(row, *c)).map(|c| (row, c)),
            Direction::Left => (0..col).rev().find(|c| !sh.is_blank(row, *c)).map(|c| (row, c)),
            Direction::Down => (row + 1..=er).find(|r| !sh.is_blank(*r, col)).map(|r| (r, col)),
            Direction::Up => (0..row).rev().find(|r| !sh.is_blank(*r, col)).map(|r| (r, col)),
        };
        found.unwrap_or(match dir {
            Direction::Up => (0, col),
            Direction::Down => (MAX_ROWS - 1, col),
            Direction::Left => (row, 0),
            Direction::Right => (row, MAX_COLS - 1),
        })
    }

    /// The block of data around a cell (Ctrl+A / Ctrl+Shift+*).
    pub fn current_region(&self, sheet: usize, row: u32, col: u32) -> Rect {
        let Some(sh) = self.sheets.get(sheet) else {
            return Rect::cell(row, col);
        };
        let mut rect = Rect::cell(row, col);
        let (er, ec) = sh.content_extent();
        loop {
            let mut grown = false;
            let probe = Rect::new(
                rect.r0.saturating_sub(1),
                rect.c0.saturating_sub(1),
                (rect.r1 + 1).min(er),
                (rect.c1 + 1).min(ec),
            );
            for r in probe.r0..=probe.r1 {
                for c in probe.c0..=probe.c1 {
                    if !rect.contains(r, c) && !sh.is_blank(r, c) {
                        rect = rect.union(&Rect::cell(r, c));
                        grown = true;
                    }
                }
            }
            if !grown {
                return rect;
            }
        }
    }

    /// The range AutoSum would propose for a cell: the run of numbers
    /// directly above, else directly to the left.
    pub fn autosum_range(&self, sheet: usize, row: u32, col: u32) -> Option<Rect> {
        let sh = self.sheets.get(sheet)?;
        let is_num = |r: u32, c: u32| matches!(sh.value_at(r, c), Value::Number(_));
        if row > 0 && is_num(row - 1, col) {
            let mut r = row - 1;
            while r > 0 && is_num(r - 1, col) {
                r -= 1;
            }
            return Some(Rect::new(r, col, row - 1, col));
        }
        if col > 0 && is_num(row, col - 1) {
            let mut c = col - 1;
            while c > 0 && is_num(row, c - 1) {
                c -= 1;
            }
            return Some(Rect::new(row, c, row, col - 1));
        }
        // Skip one blank cell above (totals often sit under a gap).
        if row > 1 && is_num(row - 2, col) {
            let mut r = row - 2;
            while r > 0 && is_num(r - 1, col) {
                r -= 1;
            }
            return Some(Rect::new(r, col, row - 1, col));
        }
        None
    }

    /// Status-bar aggregates over the selected rectangles.
    pub fn selection_stats(&self, sheet: usize, rects: &[Rect]) -> SelectionStats {
        let mut st = SelectionStats {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            ..Default::default()
        };
        let Some(sh) = self.sheets.get(sheet) else {
            return st;
        };
        let mut visit = |v: &Value| match v {
            Value::Empty => {}
            Value::Number(n) => {
                st.count += 1;
                st.numeric_count += 1;
                st.sum += n;
                st.min = st.min.min(*n);
                st.max = st.max.max(*n);
            }
            _ => st.count += 1,
        };
        for rect in rects {
            for ((r, c), cell) in sh.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
                if *c >= rect.c0 && *c <= rect.c1 && !matches!(cell.content, Content::Empty) {
                    let _ = r;
                    visit(&cell.value);
                }
            }
            for sp in &sh.spills {
                for i in 0..sp.array.rows {
                    for j in 0..sp.array.cols {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        if rect.contains(sp.row + i as u32, sp.col + j as u32) {
                            visit(sp.array.get(i, j));
                        }
                    }
                }
            }
        }
        st
    }

    // ---- find -----------------------------------------------------------

    fn cell_matches(&self, sheet: usize, row: u32, col: u32, needle: &str, opts: &FindOptions) -> bool {
        let hay = if opts.in_formulas {
            self.edit_text(sheet, row, col)
        } else {
            self.display_text(sheet, row, col).text
        };
        let (hay, needle) = if opts.match_case {
            (hay, needle.to_string())
        } else {
            (hay.to_lowercase(), needle.to_lowercase())
        };
        if opts.whole_cell {
            hay == needle
        } else {
            hay.contains(&needle)
        }
    }

    /// Next cell after `from` (reading order, wrapping) whose text matches.
    pub fn find_next(&self, sheet: usize, from: (u32, u32), needle: &str, opts: &FindOptions) -> Option<(u32, u32)> {
        if needle.is_empty() {
            return None;
        }
        let sh = self.sheets.get(sheet)?;
        let after = sh
            .cells
            .range((from.0, from.1 + 1)..)
            .map(|(k, _)| *k)
            .chain(sh.cells.range(..=(from.0, from.1)).map(|(k, _)| *k));
        for (r, c) in after {
            if self.cell_matches(sheet, r, c, needle, opts) {
                return Some((r, c));
            }
        }
        None
    }

    /// Every cell whose text matches, in reading order.
    pub fn find_all(&self, sheet: usize, needle: &str, opts: &FindOptions) -> Vec<(u32, u32)> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        if needle.is_empty() {
            return Vec::new();
        }
        sh.cells
            .keys()
            .filter(|(r, c)| self.cell_matches(sheet, *r, *c, needle, opts))
            .copied()
            .collect()
    }

    /// Replace occurrences in constants and formulas. Returns the number of
    /// cells changed.
    pub fn replace_all(&mut self, sheet: usize, needle: &str, replacement: &str, opts: &FindOptions) -> usize {
        let mut o = opts.clone();
        o.in_formulas = true;
        let hits = self.find_all(sheet, needle, &o);
        let mut changed = 0;
        for (r, c) in hits {
            let text = self.edit_text(sheet, r, c);
            let new = if opts.whole_cell {
                replacement.to_string()
            } else if opts.match_case {
                text.replace(needle, replacement)
            } else {
                replace_ignore_case(&text, needle, replacement)
            };
            if new != text && self.set_cell_text(sheet, r, c, &new).is_ok() {
                changed += 1;
            }
        }
        changed
    }
}

fn replace_ignore_case(text: &str, needle: &str, replacement: &str) -> String {
    let lower = text.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() || lower.len() != text.len() {
        return text.replace(&needle, replacement);
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&needle) {
        out.push_str(&text[i..i + pos]);
        out.push_str(replacement);
        i += pos + needle.len();
    }
    out.push_str(&text[i..]);
    out
}

fn shift_content(content: &Content, _value: &Value, d_row: i64, d_col: i64) -> Content {
    match content {
        Content::Formula(e) => Content::Formula(Arc::new(e.shifted(d_row, d_col))),
        other => other.clone(),
    }
}

/// Parse clipboard text: rows separated by newlines, fields by tabs, with
/// optional double-quoted fields (which may contain tabs and newlines).
pub fn parse_tsv(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut in_quotes = false;
    let mut field_start = true;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field_start => {
                in_quotes = true;
                field_start = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut field));
                field_start = true;
            }
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                field_start = true;
            }
            c => {
                field.push(c);
                field_start = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Convenience for building formulas programmatically (AutoSum).
pub fn sum_formula(rect: Rect) -> Expr {
    Expr::Func(Arc::from("SUM"), vec![Expr::Ref(crate::ast::RefExpr::range(rect))])
}
