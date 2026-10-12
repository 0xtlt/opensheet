//! Editing operations on a workbook: copy / paste, autofill, inserting and
//! deleting rows and columns, sorting, removing duplicates, merging,
//! borders, navigation, find.

use crate::ast::{Expr, RefExpr, RefKind, StructuralEdit};
use crate::filter::cell_colors;
use crate::format;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::{Border, Color, StyleId};
use crate::value::{ErrorKind, Value};
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashSet};
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
    /// The sheet rows the cells were taken from, top to bottom: the rows of
    /// `rect` without those a filter hides.
    pub rows: Vec<u32>,
    /// Row-major, `rows.len() * rect.cols()` entries.
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

/// What one level of a sort compares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortOn {
    #[default]
    Values,
    /// The cells filled with this colour (`None` = no fill) come first.
    CellColor(Option<Color>),
    /// The cells written in this colour (`None` = automatic) come first.
    FontColor(Option<Color>),
}

/// One level of a sort ("Sort by" / "Then by" in the Sort dialog).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SortLevel {
    /// The key column, as an offset from the left of the sorted range (the
    /// key row, from its top, when sorting left to right).
    pub key: u32,
    pub on: SortOn,
    /// Z to A, largest to smallest, newest to oldest; for a colour, "on
    /// bottom" instead of "on top".
    pub descending: bool,
    /// Values only: sort in the order of this list ("Mon, Tue, Wed...")
    /// instead of alphabetically; texts not in the list come after it.
    pub custom_list: Option<Vec<String>>,
}

impl SortLevel {
    /// A plain sort on values.
    pub fn values(key: u32, descending: bool) -> SortLevel {
        SortLevel {
            key,
            descending,
            ..Default::default()
        }
    }
}

/// The settings of the Sort dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SortSpec {
    /// Most significant first.
    pub levels: Vec<SortLevel>,
    /// "My data has headers": the first row stays in place. Ignored when
    /// sorting left to right, as in Excel.
    pub has_header: bool,
    pub case_sensitive: bool,
    /// Reorder columns by the values of key rows instead of rows by the
    /// values of key columns.
    pub left_to_right: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortError {
    /// "To do this, all the merged cells need to be the same size."
    MergedCells,
}

/// Outcome of Remove Duplicates, for its message box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuplicatesRemoved {
    pub removed: usize,
    pub remaining: usize,
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

pub(crate) fn shift_set(set: &BTreeSet<u32>, insert: bool, at: u32, count: u32, max: u32) -> BTreeSet<u32> {
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

pub(crate) fn shift_interval(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32)> {
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

    /// Copy a rectangle. Rows hidden by a filter are left out, as in Excel
    /// (rows hidden by hand are copied).
    pub fn copy(&self, sheet: usize, rect: Rect) -> Clip {
        let rect = self.clamp_to_used(sheet, rect);
        let sh = &self.sheets[sheet];
        let mut rows: Vec<u32> = (rect.r0..=rect.r1).filter(|r| !sh.filtered_rows.contains(r)).collect();
        if rows.is_empty() {
            rows = (rect.r0..=rect.r1).collect();
        }
        let whole = rows.len() as u32 == rect.rows();
        let mut cells = Vec::with_capacity(rows.len() * rect.cols() as usize);
        let mut text = String::new();
        for (i, &r) in rows.iter().enumerate() {
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
            if i + 1 < rows.len() {
                text.push('\n');
            }
        }
        // Merged areas only keep their shape when no row was left out.
        let merges = sh
            .merges
            .iter()
            .filter(|m| whole && rect.contains(m.r0, m.c0) && rect.contains(m.r1, m.c1))
            .map(|m| Rect::new(m.r0 - rect.r0, m.c0 - rect.c0, m.r1 - rect.r0, m.c1 - rect.c0))
            .collect();
        Clip {
            sheet,
            rect,
            rows,
            cells,
            merges,
            text,
        }
    }

    /// Paste a clip at `dest`. When `dest` is a multiple of the clip size the
    /// clip is tiled over it. Returns the rectangle written.
    pub fn paste(&mut self, sheet: usize, dest: Rect, clip: &Clip, mode: PasteMode) -> Rect {
        let (ch, cw) = (clip.rows.len() as u32, clip.rect.cols());
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
                let d_row = row as i64 - clip.rows[(dr % ch) as usize] as i64;
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
        let mut targets: Vec<i64> = if vertical {
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
        // Rows a filter hides are stepped over: the series continues on the
        // rows that are shown.
        let filtered = self.sheets[sheet].filtered_rows.clone();
        if vertical && !filtered.is_empty() {
            targets.retain(|r| !filtered.contains(&(*r as u32)));
        }

        for lane in 0..lanes {
            if !vertical && filtered.contains(&(src.r0 + lane)) {
                continue;
            }
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

    /// Copy the first row of `rect` into the rows below (Ctrl+D). With a
    /// filter on, only the rows it shows take part, as in Excel.
    pub fn fill_down(&mut self, sheet: usize, rect: Rect) {
        if rect.rows() < 2 || sheet >= self.sheets.len() {
            return;
        }
        let filtered = &self.sheets[sheet].filtered_rows;
        if filtered.range(rect.r0..=rect.r1).next().is_none() {
            let clip = self.copy(sheet, Rect::new(rect.r0, rect.c0, rect.r0, rect.c1));
            self.paste(sheet, Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1), &clip, PasteMode::All);
            return;
        }
        let shown: Vec<u32> = (rect.r0..=rect.r1).filter(|r| !filtered.contains(r)).collect();
        let Some((first, rest)) = shown.split_first() else {
            return;
        };
        let clip = self.copy(sheet, Rect::new(*first, rect.c0, *first, rect.c1));
        for row in rest {
            self.paste(sheet, Rect::new(*row, rect.c0, *row, rect.c1), &clip, PasteMode::All);
        }
    }

    /// Copy the first column of `rect` into the columns to the right
    /// (Ctrl+R). Rows a filter hides are left alone.
    pub fn fill_right(&mut self, sheet: usize, rect: Rect) {
        if rect.cols() < 2 || sheet >= self.sheets.len() {
            return;
        }
        let filtered = &self.sheets[sheet].filtered_rows;
        if filtered.range(rect.r0..=rect.r1).next().is_none() {
            let clip = self.copy(sheet, Rect::new(rect.r0, rect.c0, rect.r1, rect.c0));
            self.paste(sheet, Rect::new(rect.r0, rect.c0 + 1, rect.r1, rect.c1), &clip, PasteMode::All);
            return;
        }
        let shown: Vec<u32> = (rect.r0..=rect.r1).filter(|r| !filtered.contains(r)).collect();
        for row in shown {
            let clip = self.copy(sheet, Rect::cell(row, rect.c0));
            self.paste(sheet, Rect::new(row, rect.c0 + 1, row, rect.c1), &clip, PasteMode::All);
        }
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
        let criteria_before = crate::filter::criteria_count(&self.sheets[sheet]);
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
        crate::table::after_structural_edit(self, sheet);
        // A criterion went with its column, or a filter with its header
        // row: show the rows it hid.
        if crate::filter::criteria_count(&self.sheets[sheet]) < criteria_before {
            self.refilter(sheet);
        }
    }

    pub fn insert_rows(&mut self, sheet: usize, at: u32, count: u32) {
        self.structural(sheet, StructuralEdit::InsertRows { at, count });
    }

    /// Delete rows. With a filter on, the rows it hides inside the span are
    /// kept, as in Excel.
    pub fn delete_rows(&mut self, sheet: usize, at: u32, count: u32) {
        let Some(sh) = self.sheets.get(sheet) else {
            return;
        };
        let end = at.saturating_add(count);
        let kept: Vec<u32> = sh.filtered_rows.range(at..end).copied().collect();
        if kept.is_empty() {
            self.structural(sheet, StructuralEdit::DeleteRows { at, count });
            return;
        }
        // The runs of shown rows between the hidden ones.
        let mut runs: Vec<(u32, u32)> = Vec::new();
        let mut start = at;
        for row in kept {
            if row > start {
                runs.push((start, row - start));
            }
            start = row + 1;
        }
        if end > start {
            runs.push((start, end - start));
        }
        match runs.as_slice() {
            [] => {}
            [(start, count)] => self.structural(sheet, StructuralEdit::DeleteRows { at: *start, count: *count }),
            _ => self.delete_row_runs(sheet, &runs),
        }
    }

    /// Delete several runs of rows in one pass over the cells and the
    /// formulas. Deleting what a filter shows of a long list is thousands
    /// of runs: one `structural` edit per run would take for ever.
    fn delete_row_runs(&mut self, sheet: usize, runs: &[(u32, u32)]) {
        self.snapshot();
        let cut = RowCut::new(runs);
        let edited = self.sheets[sheet].name.clone();
        let criteria_before = crate::filter::criteria_count(&self.sheets[sheet]);
        let sh: &mut Sheet = &mut self.sheets[sheet];
        sh.cells = std::mem::take(&mut sh.cells)
            .into_iter()
            .filter_map(|((r, c), cell)| Some(((cut.moved(r)?, c), cell)))
            .collect();
        sh.row_heights = sh.row_heights.iter().filter_map(|(r, h)| Some((cut.moved(*r)?, *h))).collect();
        sh.row_styles = sh.row_styles.iter().filter_map(|(r, s)| Some((cut.moved(*r)?, *s))).collect();
        sh.hidden_rows = sh.hidden_rows.iter().filter_map(|r| cut.moved(*r)).collect();
        // Set aside, so that the hooks below, called run by run, do not
        // shift it thousands of times.
        let filtered: BTreeSet<u32> =
            std::mem::take(&mut sh.filtered_rows).iter().filter_map(|r| cut.moved(*r)).collect();
        sh.merges = std::mem::take(&mut sh.merges)
            .into_iter()
            .filter_map(|m| {
                let (r0, r1) = cut.interval(m.r0, m.r1)?;
                let m = Rect::new(r0, m.c0, r1, m.c1);
                (!m.is_single()).then_some(m)
            })
            .collect();
        for sh in &mut self.sheets {
            let own = sh.name.clone();
            for cell in sh.cells.values_mut() {
                if let Content::Formula(e) = &cell.content {
                    let adjusted = e.map_refs(&mut |r| cut.fix(r, &edited, &own));
                    if adjusted != **e {
                        cell.content = Content::Formula(Arc::new(adjusted));
                    }
                }
            }
        }
        for def in self.names.values_mut() {
            def.expr = Arc::new(def.expr.map_refs(&mut |r| cut.fix(r, &edited, &edited)));
        }
        // Charts and the features of the other modules follow run by run,
        // bottom first so that the rows above keep their numbers.
        for &(at, count) in runs.iter().rev() {
            let edit = StructuralEdit::DeleteRows { at, count };
            for sh in &mut self.sheets {
                let own = sh.name.clone();
                for chart in &mut sh.charts {
                    chart.adjust(edit, &edited, &own);
                }
                crate::features::on_structural_edit(sh, edit, &edited);
            }
        }
        self.sheets[sheet].filtered_rows = filtered;
        crate::table::after_structural_edit(self, sheet);
        if crate::filter::criteria_count(&self.sheets[sheet]) < criteria_before {
            self.refilter(sheet);
        }
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
        // Cells of a table cannot be merged.
        if self.sheets[sheet].tables.iter().any(|t| t.range.intersects(&rect)) {
            return;
        }
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
        let spec = SortSpec {
            levels: vec![SortLevel::values(key_col.saturating_sub(rect.c0), !ascending)],
            has_header,
            ..Default::default()
        };
        // A range with merged cells is left as it is.
        let _ = self.sort_range(sheet, rect, &spec);
    }

    /// Sort a range as the Sort dialog does: on several levels, on values
    /// or colours, by rows or left to right. Hidden and filtered rows
    /// (hidden columns, left to right) stay where they are; formulas move
    /// with their cells and read as if they had been copied there.
    pub fn sort_range(&mut self, sheet: usize, rect: Rect, spec: &SortSpec) -> Result<(), SortError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(());
        };
        let rect = self.clamp_to_used(sheet, rect);
        let by_rows = !spec.left_to_right;
        let area = if by_rows && spec.has_header {
            if rect.r0 == rect.r1 {
                return Ok(());
            }
            Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1)
        } else {
            rect
        };
        if sh.merges.iter().any(|m| m.intersects(&area)) {
            return Err(SortError::MergedCells);
        }
        // The lines (rows, or columns) that are reordered.
        let lines: Vec<u32> = if by_rows {
            sh.visible_rows(area.r0, area.r1).collect()
        } else {
            (area.c0..=area.c1).filter(|c| !sh.hidden_cols.contains(c)).collect()
        };
        if lines.len() < 2 || spec.levels.is_empty() {
            return Ok(());
        }

        let keys: Vec<Vec<SortKey>> = spec
            .levels
            .iter()
            .map(|level| {
                let list: Option<Vec<String>> =
                    level.custom_list.as_ref().map(|list| list.iter().map(|s| list_key(s)).collect());
                lines
                    .iter()
                    .map(|&line| {
                        let (row, col) = if by_rows {
                            (line, rect.c0 + level.key)
                        } else {
                            (rect.r0 + level.key, line)
                        };
                        match level.on {
                            SortOn::Values => {
                                let value = sh.value_at(row, col);
                                let place = match (&list, &value) {
                                    (Some(list), Value::Text(s)) => {
                                        let key = list_key(s);
                                        list.iter().position(|x| *x == key)
                                    }
                                    _ => None,
                                };
                                SortKey::Value(value, place)
                            }
                            SortOn::CellColor(color) => SortKey::Flag(cell_colors(self, sheet, row, col).0 == color),
                            SortOn::FontColor(color) => SortKey::Flag(cell_colors(self, sheet, row, col).1 == color),
                        }
                    })
                    .collect()
            })
            .collect();

        // A stable sort: lines that compare equal (errors, among others)
        // keep their original order.
        let mut order: Vec<usize> = (0..lines.len()).collect();
        order.sort_by(|&a, &b| {
            for (level, keys) in spec.levels.iter().zip(&keys) {
                let o = compare_keys(&keys[a], &keys[b], level.descending, spec.case_sensitive);
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        });

        // Lift every line that moves, then set them down in their new places.
        let across = if by_rows { rect.c0..=rect.c1 } else { rect.r0..=rect.r1 };
        let mut moves: Vec<(u32, u32, Vec<Option<Cell>>)> = Vec::new();
        for (i, &from) in order.iter().enumerate() {
            let (from, to) = (lines[from], lines[i]);
            if from != to {
                let cells = across
                    .clone()
                    .map(|x| sh.cells.get(&if by_rows { (from, x) } else { (x, from) }).cloned())
                    .collect();
                moves.push((from, to, cells));
            }
        }
        for (from, to, cells) in moves {
            let delta = to as i64 - from as i64;
            for (x, cell) in across.clone().zip(cells) {
                let (row, col, d_row, d_col) = if by_rows { (to, x, delta, 0) } else { (x, to, 0, delta) };
                let cell = cell.map(|mut c| {
                    if let Content::Formula(e) = &c.content {
                        c.content = Content::Formula(Arc::new(e.shifted(d_row, d_col)));
                    }
                    c
                });
                self.put_cell(sheet, row, col, cell);
            }
        }
        Ok(())
    }

    /// The built-in custom lists of the Sort dialog in the workbook's
    /// language: short and long weekday names, short and long month names.
    pub fn custom_lists(&self) -> Vec<Vec<String>> {
        let locale = self.locale;
        // Weeks start on Sunday in the English lists, on Monday elsewhere.
        let first = if locale.id == "en" { 0 } else { 1 };
        let days = |names: &[&str; 7]| (0..7).map(|i| names[(i + first) % 7].to_string()).collect::<Vec<_>>();
        let months = |names: &[&str; 12]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        vec![
            days(&locale.days_short),
            days(&locale.days),
            months(&locale.months_short),
            months(&locale.months),
        ]
    }

    /// Whether the first row of `rect` looks like column headers (what "My
    /// data has headers" is preset to): only labels, above a row that holds
    /// other kinds of values or is formatted differently.
    pub fn guess_header_row(&self, sheet: usize, rect: Rect) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let rect = self.clamp_to_used(sheet, rect);
        if rect.r0 == rect.r1 {
            return false;
        }
        let is_text = |r: u32, c: u32| matches!(sh.value_at(r, c), Value::Text(_));
        let cols = rect.c0..=rect.c1;
        let labels = cols.clone().all(|c| is_text(rect.r0, c) || sh.is_blank(rect.r0, c))
            && cols.clone().any(|c| is_text(rect.r0, c));
        labels
            && cols.clone().any(|c| {
                let other_kind = !is_text(rect.r0 + 1, c) && !sh.is_blank(rect.r0 + 1, c);
                let (head, below) = (self.style_at(sheet, rect.r0, c), self.style_at(sheet, rect.r0 + 1, c));
                other_kind || head.font.bold != below.font.bold || head.fill != below.fill
            })
    }

    /// Remove Duplicates: delete the rows of `rect` that repeat an earlier
    /// row in the compared columns (`columns` are offsets from the left of
    /// `rect`; empty = every column). As in Excel, cells are compared by
    /// what they display, without case, and the rows below move up inside
    /// the range only. A table shrinks by the rows removed.
    pub fn remove_duplicates(
        &mut self,
        sheet: usize,
        rect: Rect,
        columns: &[u32],
        has_header: bool,
    ) -> Result<DuplicatesRemoved, SortError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Ok(DuplicatesRemoved::default());
        };
        let rect = self.clamp_to_used(sheet, rect);
        if has_header && rect.r0 == rect.r1 {
            return Ok(DuplicatesRemoved::default());
        }
        let data = Rect::new(rect.r0 + has_header as u32, rect.c0, rect.r1, rect.c1);
        if sh.merges.iter().any(|m| m.intersects(&data)) {
            return Err(SortError::MergedCells);
        }
        let cols: Vec<u32> = if columns.is_empty() {
            (data.c0..=data.c1).collect()
        } else {
            columns.iter().map(|c| data.c0 + c).filter(|c| *c <= data.c1).collect()
        };
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut kept: Vec<u32> = Vec::new();
        for row in data.r0..=data.r1 {
            let key = cols.iter().map(|c| self.display_text(sheet, row, *c).text.to_lowercase()).collect();
            if seen.insert(key) {
                kept.push(row);
            }
        }
        let dropped = data.rows() as usize - kept.len();
        let outcome = DuplicatesRemoved {
            removed: dropped,
            remaining: kept.len(),
        };
        if dropped == 0 {
            return Ok(outcome);
        }
        let table = sh
            .tables
            .iter()
            .find(|t| {
                t.range.c0 == data.c0
                    && t.range.c1 == data.c1
                    && t.data_range().is_some_and(|d| d.r0 <= data.r0 && data.r1 <= d.r1)
            })
            .map(|t| t.id);
        // The rows that stay close up at the top of the range...
        for (i, &from) in kept.iter().enumerate() {
            let to = data.r0 + i as u32;
            if from == to {
                continue;
            }
            for col in data.c0..=data.c1 {
                let cell = self.sheets[sheet].cells.get(&(from, col)).cloned().map(|mut c| {
                    if let Content::Formula(e) = &c.content {
                        c.content = Content::Formula(Arc::new(e.shifted(to as i64 - from as i64, 0)));
                    }
                    c
                });
                self.put_cell(sheet, to, col, cell);
            }
        }
        // ...and the rows left over at its end are emptied. A table gives
        // them up (unless a table under it keeps its cells from moving).
        let vacated = data.r0 + kept.len() as u32;
        for row in vacated..=data.r1 {
            for col in data.c0..=data.c1 {
                self.put_cell(sheet, row, col, None);
            }
        }
        if let Some(id) = table {
            let _ = self.table_delete_rows(sheet, id, vacated, dropped as u32);
        }
        // The rows a filter hid are not the same rows any more.
        self.refilter_if_filtered(sheet);
        Ok(outcome)
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
            // Past the last row or column holding data there is nothing to
            // reach for (and the probe must not turn inside out).
            let probe = Rect::new(
                rect.r0.saturating_sub(1),
                rect.c0.saturating_sub(1),
                (rect.r1 + 1).min(er).max(rect.r1),
                (rect.c1 + 1).min(ec).max(rect.c1),
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
        // Only the cells that are shown count, as in Excel.
        let any_hidden = !sh.hidden_rows.is_empty() || !sh.filtered_rows.is_empty();
        let shown = |row: u32| !any_hidden || !sh.is_row_hidden(row);
        for rect in rects {
            for ((r, c), cell) in sh.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
                if *c >= rect.c0 && *c <= rect.c1 && !matches!(cell.content, Content::Empty) && shown(*r) {
                    visit(&cell.value);
                }
            }
            for sp in &sh.spills {
                for i in 0..sp.array.rows {
                    for j in 0..sp.array.cols {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        let row = sp.row + i as u32;
                        if rect.contains(row, sp.col + j as u32) && shown(row) {
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

/// Runs of rows being deleted together: where the rows that stay end up.
struct RowCut {
    /// First row of each run, ascending.
    starts: Vec<u32>,
    /// The row after each run.
    ends: Vec<u32>,
    /// How many rows the earlier runs delete.
    before: Vec<u32>,
}

impl RowCut {
    /// `runs` are (first row, count), ascending and apart from each other.
    fn new(runs: &[(u32, u32)]) -> RowCut {
        let mut before = Vec::with_capacity(runs.len());
        let mut total = 0;
        for (_, count) in runs {
            before.push(total);
            total += count;
        }
        RowCut {
            starts: runs.iter().map(|(at, _)| *at).collect(),
            ends: runs.iter().map(|(at, count)| at + count).collect(),
            before,
        }
    }

    /// The run a row is deleted with, if any.
    fn run_of(&self, row: u32) -> Option<usize> {
        let i = self.starts.partition_point(|s| *s <= row).checked_sub(1)?;
        (row < self.ends[i]).then_some(i)
    }

    /// The new number of a row; `None` when it is deleted.
    fn moved(&self, row: u32) -> Option<u32> {
        let runs = self.starts.partition_point(|s| *s <= row);
        let Some(last) = runs.checked_sub(1) else {
            return Some(row);
        };
        (row >= self.ends[last]).then(|| row - (self.before[last] + self.ends[last] - self.starts[last]))
    }

    /// What is left of the rows `lo..=hi`; `None` when they all go.
    fn interval(&self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        let lo = self.run_of(lo).map_or(lo, |i| self.ends[i]);
        let hi = match self.run_of(hi) {
            Some(i) => self.starts[i].checked_sub(1)?,
            None => hi,
        };
        (lo <= hi).then_some(())?;
        Some((self.moved(lo)?, self.moved(hi)?))
    }

    /// A reference after the rows went (`edited` is the sheet they were
    /// deleted from, `own` the sheet holding the formula).
    fn fix(&self, r: &RefExpr, edited: &str, own: &str) -> Expr {
        let target = r.sheet.as_deref().unwrap_or(own);
        if !target.eq_ignore_ascii_case(edited) || r.kind == RefKind::Cols {
            return Expr::Ref(r.clone());
        }
        let end = r.end.unwrap_or(r.start);
        let flipped = r.start.row > end.row;
        let Some((lo, hi)) = self.interval(r.start.row.min(end.row), r.start.row.max(end.row)) else {
            return Expr::Error(ErrorKind::Ref);
        };
        let mut out = r.clone();
        out.start.row = if flipped { hi } else { lo };
        if let Some(e) = &mut out.end {
            e.row = if flipped { lo } else { hi };
        }
        Expr::Ref(out)
    }
}

/// What a sort level reads from one line.
enum SortKey {
    /// The value, and its place in the level's custom list.
    Value(Value, Option<usize>),
    /// Whether the cell has the level's colour.
    Flag(bool),
}

/// How texts are matched against a custom list: without case, and "Jan."
/// is "Jan".
fn list_key(s: &str) -> String {
    s.trim().trim_end_matches('.').to_lowercase()
}

fn compare_keys(a: &SortKey, b: &SortKey, descending: bool, case_sensitive: bool) -> Ordering {
    let o = match (a, b) {
        // Cells of the colour first ("on top").
        (SortKey::Flag(x), SortKey::Flag(y)) => y.cmp(x),
        (SortKey::Value(x, px), SortKey::Value(y, py)) => {
            // Blanks sort last whatever the direction.
            match (x.is_empty(), y.is_empty()) {
                (true, true) => return Ordering::Equal,
                (true, false) => return Ordering::Greater,
                (false, true) => return Ordering::Less,
                _ => {}
            }
            match (px, py) {
                (Some(i), Some(j)) => i.cmp(j),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => sort_order(x, y, case_sensitive),
            }
        }
        _ => Ordering::Equal,
    };
    if descending { o.reverse() } else { o }
}

/// Excel's ascending sort order of two non-blank values: numbers, then
/// text, then FALSE, TRUE, then errors (all equal to each other).
pub fn sort_order(a: &Value, b: &Value, case_sensitive: bool) -> Ordering {
    fn kind(v: &Value) -> u8 {
        match v {
            Value::Number(_) => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            Value::Error(_) => 3,
            Value::Empty | Value::Array(_) => 4,
        }
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => collate(x, y, case_sensitive),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => kind(a).cmp(&kind(b)),
    }
}

/// The letter an accented Latin letter sorts with.
fn unaccent(c: char) -> char {
    match c {
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => c,
    }
}

/// Compare text the way Sort does: without case and without accents first
/// ("école" sorts among the e's); accents, then (when `case_sensitive`)
/// lowercase before uppercase, only break ties.
pub fn collate(a: &str, b: &str, case_sensitive: bool) -> Ordering {
    fn lower(s: &str) -> impl Iterator<Item = char> + '_ {
        s.chars().flat_map(char::to_lowercase)
    }
    lower(a)
        .map(unaccent)
        .cmp(lower(b).map(unaccent))
        .then_with(|| lower(a).cmp(lower(b)))
        .then_with(|| {
            if !case_sensitive {
                return Ordering::Equal;
            }
            match a.chars().zip(b.chars()).find(|(x, y)| x != y) {
                Some((x, _)) if x.is_lowercase() => Ordering::Less,
                Some(_) => Ordering::Greater,
                None => Ordering::Equal,
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::{Comparison, Condition, Criteria, CustomFilter, FilterScope};
    use crate::locale::Locale;
    use crate::tests::wb;
    use crate::workbook::ClearMode;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    /// What column `col` displays in rows `r0..=r1` (1-based, as on screen).
    fn column(w: &Workbook, col: u32, r0: u32, r1: u32) -> Vec<String> {
        (r0 - 1..r1).map(|r| w.display_text(0, r, col).text).collect()
    }

    fn sort(w: &mut Workbook, range: &str, levels: Vec<SortLevel>, has_header: bool) {
        let spec = SortSpec {
            levels,
            has_header,
            ..Default::default()
        };
        w.sort_range(0, rect(range), &spec).unwrap();
        w.commit("sort");
    }

    #[test]
    fn excel_order_of_values() {
        let cells = [
            ("A1", "3"),
            ("A2", "b"),
            ("A3", "TRUE"),
            ("A4", "#N/A"),
            ("A6", "1"),
            ("A7", "A"),
            ("A8", "FALSE"),
            ("A9", "=1/0"),
            ("A10", "a"),
            ("A11", "École"),
            ("A12", "zoo"),
        ];
        let mut w = wb(&cells);
        sort(&mut w, "A1:A12", vec![SortLevel::values(0, false)], false);
        // Numbers, text (accents sort with their letter), FALSE, TRUE,
        // errors in their original order, the blank last.
        assert_eq!(
            column(&w, 0, 1, 12),
            ["1", "3", "A", "a", "b", "École", "zoo", "FALSE", "TRUE", "#N/A", "#DIV/0!", ""]
        );
        let mut w = wb(&cells);
        sort(&mut w, "A1:A12", vec![SortLevel::values(0, true)], false);
        assert_eq!(
            column(&w, 0, 1, 12),
            ["#N/A", "#DIV/0!", "TRUE", "FALSE", "zoo", "École", "b", "A", "a", "3", "1", ""]
        );
        assert!(w.undo());
        assert_eq!(column(&w, 0, 1, 4), ["3", "b", "TRUE", "#N/A"]);
    }

    #[test]
    fn levels_headers_and_case() {
        let cells = [
            ("A1", "Team"),
            ("B1", "Score"),
            ("A2", "red"),
            ("B2", "1"),
            ("A3", "Blue"),
            ("B3", "5"),
            ("A4", "Red"),
            ("B4", "7"),
            ("A5", "blue"),
            ("B5", "2"),
        ];
        let mut w = wb(&cells);
        sort(&mut w, "A1:B5", vec![SortLevel::values(0, false), SortLevel::values(1, true)], true);
        assert_eq!(column(&w, 0, 1, 5), ["Team", "Blue", "blue", "Red", "red"]);
        assert_eq!(column(&w, 1, 1, 5), ["Score", "5", "2", "7", "1"]);

        // Case sensitive: lowercase first among equals.
        let mut w = wb(&cells);
        let spec = SortSpec {
            levels: vec![SortLevel::values(0, false)],
            has_header: true,
            case_sensitive: true,
            ..Default::default()
        };
        w.sort_range(0, rect("A1:B5"), &spec).unwrap();
        assert_eq!(column(&w, 0, 1, 5), ["Team", "blue", "Blue", "red", "Red"]);

        // Without the header option the first row is sorted like the rest.
        let mut w = wb(&cells);
        sort(&mut w, "A1:B5", vec![SortLevel::values(1, false)], false);
        assert_eq!(column(&w, 1, 1, 5), ["1", "2", "5", "7", "Score"]);

        // The older entry point still sorts on a sheet column.
        let mut w = wb(&cells);
        w.sort_rows(0, rect("A1:B5"), 1, false, true);
        assert_eq!(column(&w, 1, 1, 5), ["Score", "7", "5", "2", "1"]);

        assert!(w.guess_header_row(0, rect("A1:B5")));
        assert!(!w.guess_header_row(0, rect("A2:B5")));
        assert!(!w.guess_header_row(0, rect("A1:A5")));
        w.update_style(0, rect("A1"), &|s| s.font.bold = true);
        assert!(w.guess_header_row(0, rect("A1:A5")));
    }

    #[test]
    fn custom_lists() {
        let mut w = wb(&[("A1", "Wed"), ("A2", "mon"), ("A3", "Sun"), ("A4", "other"), ("A5", "Tue."), ("A6", "also")]);
        let lists = w.custom_lists();
        assert_eq!(lists[0], ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]);
        assert_eq!(lists[3][0], "January");
        let level = |descending| SortLevel {
            custom_list: Some(lists[0].clone()),
            ..SortLevel::values(0, descending)
        };
        // In the order of the list; what is not in it comes after.
        sort(&mut w, "A1:A6", vec![level(false)], false);
        assert_eq!(column(&w, 0, 1, 6), ["Sun", "mon", "Tue.", "Wed", "also", "other"]);
        sort(&mut w, "A1:A6", vec![level(true)], false);
        assert_eq!(column(&w, 0, 1, 6), ["other", "also", "Wed", "Tue.", "mon", "Sun"]);

        let mut fr = Workbook::new(Locale::fr());
        fr.set_cell_text(0, 0, 0, "x").unwrap();
        let lists = fr.custom_lists();
        assert_eq!(lists[0], ["lun", "mar", "mer", "jeu", "ven", "sam", "dim"]);
        assert_eq!(lists[1][6], "dimanche");
        assert_eq!(lists[2][1], "févr");
    }

    #[test]
    fn colours() {
        let mut w = wb(&[("A1", "a"), ("A2", "b"), ("A3", "c"), ("A4", "d")]);
        let (yellow, red) = (Color(0xFFFF00), Color(0xFF0000));
        w.update_style(0, rect("A2"), &|s| s.fill = Some(yellow));
        w.update_style(0, rect("A4"), &|s| s.fill = Some(yellow));
        w.update_style(0, rect("A3"), &|s| s.font.color = Some(red));
        let level = |on, descending| SortLevel {
            key: 0,
            on,
            descending,
            custom_list: None,
        };
        // On top: the others keep their order under them.
        sort(&mut w, "A1:A4", vec![level(SortOn::CellColor(Some(yellow)), false)], false);
        assert_eq!(column(&w, 0, 1, 4), ["b", "d", "a", "c"]);
        assert_eq!(w.style_at(0, 0, 0).fill, Some(yellow));
        // On bottom.
        sort(&mut w, "A1:A4", vec![level(SortOn::CellColor(Some(yellow)), true)], false);
        assert_eq!(column(&w, 0, 1, 4), ["a", "c", "b", "d"]);
        sort(&mut w, "A1:A4", vec![level(SortOn::FontColor(Some(red)), false)], false);
        assert_eq!(column(&w, 0, 1, 4), ["c", "a", "b", "d"]);
        // "No fill" first, then by value.
        sort(&mut w, "A1:A4", vec![level(SortOn::CellColor(None), false), SortLevel::values(0, true)], false);
        assert_eq!(column(&w, 0, 1, 4), ["c", "a", "d", "b"]);
    }

    #[test]
    fn left_to_right() {
        let mut w = wb(&[
            ("A1", "c"),
            ("B1", "a"),
            ("C1", "b"),
            ("A2", "3"),
            ("B2", "1"),
            ("C2", "2"),
            ("A3", "=A2*10"),
            ("B3", "=B2*10"),
            ("C3", "=C2*10"),
        ]);
        let spec = SortSpec {
            levels: vec![SortLevel::values(0, false)],
            left_to_right: true,
            // Ignored when sorting columns.
            has_header: true,
            ..Default::default()
        };
        w.sort_range(0, rect("A1:C3"), &spec).unwrap();
        w.commit("sort");
        let row = |r: u32| (0..3).map(|c| w.display_text(0, r, c).text).collect::<Vec<_>>();
        assert_eq!(row(0), ["a", "b", "c"]);
        assert_eq!(row(1), ["1", "2", "3"]);
        assert_eq!(row(2), ["10", "20", "30"]);
        assert_eq!(w.formula_text(0, 2, 0).unwrap(), "=A2*10");
    }

    #[test]
    fn formulas_hidden_rows_and_merges() {
        let mut w = wb(&[
            ("A1", "5"),
            ("B1", "=A1*2"),
            ("A2", "9"),
            ("B2", "=A2*2"),
            ("A3", "1"),
            ("B3", "=A3*2"),
            ("A4", "7"),
            ("B4", "=A4*2"),
        ]);
        // Row 2 is hidden by hand: it stays, the others sort around it.
        w.set_rows_hidden(0, 1, 1, true);
        sort(&mut w, "A1:B4", vec![SortLevel::values(0, false)], false);
        assert_eq!(column(&w, 0, 1, 4), ["1", "9", "5", "7"]);
        // Formulas read as if copied to their new rows.
        assert_eq!(w.formula_text(0, 0, 1).unwrap(), "=A1*2");
        assert_eq!(column(&w, 1, 1, 4), ["2", "18", "10", "14"]);

        w.set_rows_hidden(0, 1, 1, false);
        w.merge(0, rect("A3:B3"));
        w.commit("merge");
        let spec = SortSpec {
            levels: vec![SortLevel::values(0, false)],
            ..Default::default()
        };
        assert_eq!(w.sort_range(0, rect("A1:B4"), &spec), Err(SortError::MergedCells));
        assert_eq!(w.remove_duplicates(0, rect("A1:B4"), &[], false), Err(SortError::MergedCells));
        // A merge in the header row is not in the way.
        assert_eq!(
            w.sort_range(0, rect("A3:B4"), &SortSpec { has_header: true, ..spec }),
            Ok(())
        );
    }

    #[test]
    fn remove_duplicates() {
        let cells = [
            ("A1", "Name"),
            ("B1", "City"),
            ("A2", "Ann"),
            ("B2", "Paris"),
            ("A3", "Bob"),
            ("B3", "Rome"),
            ("A4", "ann"),
            ("B4", "PARIS"),
            ("A5", "Ann"),
            ("B5", "Oslo"),
            ("A6", "Bob"),
            ("B6", "Rome"),
            ("A7", "below"),
            ("C3", "beside"),
        ];
        let mut w = wb(&cells);
        let out = w.remove_duplicates(0, rect("A1:B6"), &[], true).unwrap();
        w.commit("dedupe");
        assert_eq!((out.removed, out.remaining), (2, 3));
        assert_eq!(column(&w, 0, 1, 7), ["Name", "Ann", "Bob", "Ann", "", "", "below"]);
        assert_eq!(column(&w, 1, 1, 6), ["City", "Paris", "Rome", "Oslo", "", ""]);
        // Cells beside the range do not move.
        assert_eq!(w.display_text(0, 2, 2).text, "beside");
        assert!(w.undo());
        assert_eq!(column(&w, 0, 1, 7), ["Name", "Ann", "Bob", "ann", "Ann", "Bob", "below"]);

        // On the first column only.
        let out = w.remove_duplicates(0, rect("A1:B6"), &[0], true).unwrap();
        assert_eq!((out.removed, out.remaining), (3, 2));
        assert_eq!(column(&w, 1, 1, 4), ["City", "Paris", "Rome", ""]);

        // Without header the first row is data like the others.
        let mut w = wb(&[("A1", "x"), ("A2", "X"), ("A3", "y")]);
        let out = w.remove_duplicates(0, rect("A1:A3"), &[], false).unwrap();
        assert_eq!((out.removed, out.remaining), (1, 2));
        assert_eq!(column(&w, 0, 1, 3), ["x", "y", ""]);
        let out = w.remove_duplicates(0, rect("A1:A2"), &[], false).unwrap();
        assert_eq!((out.removed, out.remaining), (0, 2));

        // Cells are compared by what they display.
        let mut w = wb(&[("A1", "1"), ("A2", "1"), ("A3", "=A1+A2")]);
        w.update_style(0, rect("A2"), &|s| s.num_fmt = "0.00".into());
        assert_eq!(w.remove_duplicates(0, rect("A1:A2"), &[], false).unwrap().removed, 0);
        w.update_style(0, rect("A1"), &|s| s.num_fmt = "0.00".into());
        assert_eq!(w.remove_duplicates(0, rect("A1:A2"), &[], false).unwrap().removed, 1);
    }

    /// A list in A1:B6 filtered to the rows where B > 2: rows 3 and 5 (1-based)
    /// are hidden.
    fn filtered() -> Workbook {
        let mut w = wb(&[
            ("A1", "Item"),
            ("B1", "Qty"),
            ("A2", "a"),
            ("B2", "5"),
            ("A3", "b"),
            ("B3", "1"),
            ("A4", "c"),
            ("B4", "7"),
            ("A5", "d"),
            ("B5", "2"),
            ("A6", "e"),
            ("B6", "9"),
        ]);
        w.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
        let more = Condition::new(Comparison::Greater, "2", Locale::en());
        w.set_filter_criteria(0, FilterScope::Sheet, 1, Some(Criteria::Custom(CustomFilter::one(more))));
        w.commit("filter");
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2, 4]);
        w
    }

    #[test]
    fn copy_takes_the_rows_a_filter_shows() {
        let mut w = filtered();
        let clip = w.copy(0, rect("A1:B6"));
        assert_eq!(clip.rect, rect("A1:B6"));
        assert_eq!(clip.rows, [0, 1, 3, 5]);
        assert_eq!(clip.text, "Item\tQty\na\t5\nc\t7\ne\t9");
        let out = w.paste(0, rect("D1"), &clip, PasteMode::All);
        w.commit("paste");
        assert_eq!(out, rect("D1:E4"));
        assert_eq!(column(&w, 3, 1, 4), ["Item", "a", "c", "e"]);
        assert_eq!(w.display_text(0, 4, 3).text, "");
        // Rows hidden by hand are copied.
        let mut w = filtered();
        w.clear_filters(0, FilterScope::Sheet);
        w.set_rows_hidden(0, 2, 2, true);
        assert_eq!(w.copy(0, rect("A1:B6")).rows.len(), 6);
        // Formulas keep pointing at the right distance from their source row.
        let mut w = filtered();
        w.set_cell_text(0, 3, 2, "=B4*2").unwrap();
        w.commit("formula");
        let clip = w.copy(0, rect("C2:C4"));
        assert_eq!(clip.rows, [1, 3]);
        w.paste(0, rect("G10"), &clip, PasteMode::All);
        assert_eq!(w.formula_text(0, 10, 6).unwrap(), "=F11*2");
    }

    #[test]
    fn fill_clear_and_delete_skip_filtered_rows() {
        let mut w = filtered();
        w.set_cell_text(0, 1, 2, "=B2*2").unwrap();
        w.fill_down(0, rect("C2:C6"));
        w.commit("fill");
        assert_eq!(column(&w, 2, 2, 6), ["10", "", "14", "", "18"]);
        assert_eq!(w.formula_text(0, 5, 2).unwrap(), "=B6*2");

        // The fill handle steps over the hidden rows too.
        w.set_cell_text(0, 1, 3, "1").unwrap();
        w.set_cell_text(0, 3, 3, "2").unwrap();
        w.autofill(0, rect("D2"), rect("D2:D6"));
        w.commit("autofill");
        assert_eq!(column(&w, 3, 2, 6), ["1", "", "1", "", "1"]);

        // Filling right, with Ctrl+R or the fill handle, as well.
        w.fill_right(0, rect("D2:E6"));
        w.autofill(0, rect("E2:E6"), rect("E2:F6"));
        w.commit("fill right");
        assert_eq!(column(&w, 4, 2, 6), ["1", "", "1", "", "1"]);
        assert_eq!(column(&w, 5, 2, 6), ["1", "", "1", "", "1"]);

        w.clear(0, rect("A2:A6"), ClearMode::Contents);
        w.commit("clear");
        assert_eq!(column(&w, 0, 1, 6), ["Item", "", "b", "", "d", ""]);

        // Deleting rows 2 to 5 only takes the shown ones (2 and 4).
        w.delete_rows(0, 1, 4);
        w.commit("delete");
        assert_eq!(column(&w, 0, 1, 4), ["Item", "b", "d", ""]);
        assert_eq!(column(&w, 1, 1, 4), ["Qty", "1", "2", "9"]);
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [1, 2]);
        assert_eq!(w.auto_filter(0, FilterScope::Sheet).unwrap().range, rect("A1:B4"));
        assert!(w.undo());
        assert_eq!(column(&w, 1, 1, 6), ["Qty", "5", "1", "7", "2", "9"]);
        assert_eq!(w.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [2, 4]);

        // Without a filter everything goes, as before.
        w.clear_filters(0, FilterScope::Sheet);
        w.delete_rows(0, 1, 4);
        assert_eq!(column(&w, 1, 1, 3), ["Qty", "9", ""]);
    }

    #[test]
    fn deleting_many_runs_at_once_is_deleting_them_one_by_one() {
        // A list where every third row is shown, with formulas, formats,
        // row heights and a merged area around, deleted in one go and run
        // by run.
        let build = || {
            let mut w = wb(&[
                ("A1", "n"),
                ("B1", "f"),
                ("D1", "=SUM(A2:A31)"),
                ("D2", "=A5"),
                ("D3", "=A7+A8"),
                ("D4", "=5:9"),
            ]);
            for r in 1..=30u32 {
                w.set_value(0, r, 0, Value::Number(r as f64));
                w.set_cell_text(0, r, 1, &format!("=A{}*2+A{}", r + 1, (r + 3).min(31))).unwrap();
            }
            w.set_cell_text(0, 40, 0, "=A3+A4").unwrap();
            w.set_row_height(0, 7, Some(33.0));
            w.set_row_height(0, 8, Some(44.0));
            w.update_style(0, rect("9:10"), &|s| s.font.bold = true);
            w.merge(0, rect("F8:G12"));
            w.set_rows_hidden(0, 14, 15, true);
            w.commit("setup");
            w.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
            let shown: Vec<String> = (1..=30).filter(|n| n % 3 == 0).map(|n| n.to_string()).collect();
            let criteria = Criteria::Values(crate::filter::ValueFilter {
                values: shown,
                ..Default::default()
            });
            w.set_filter_criteria(0, FilterScope::Sheet, 0, Some(criteria));
            w.commit("filter");
            w
        };
        let mut batch = build();
        batch.delete_rows(0, 2, 26);
        batch.commit("delete");
        let mut stepwise = build();
        let runs: Vec<u32> = (2..28).filter(|r| !stepwise.sheets[0].filtered_rows.contains(r)).collect();
        assert!(runs.len() > 5);
        for row in runs.into_iter().rev() {
            stepwise.delete_rows(0, row, 1);
        }
        stepwise.commit("delete");
        let (a, b) = (&batch.sheets[0], &stepwise.sheets[0]);
        assert_eq!(a.cells, b.cells);
        assert_eq!(a.row_heights, b.row_heights);
        assert_eq!(a.row_styles, b.row_styles);
        assert_eq!(a.hidden_rows, b.hidden_rows);
        assert_eq!(a.filtered_rows, b.filtered_rows);
        assert_eq!(a.merges, b.merges);
        assert_eq!(a.auto_filter, b.auto_filter);
        // The rows a filter hid are all still there; nine rows went.
        assert_eq!(column(&batch, 0, 1, 6), ["n", "1", "2", "4", "5", "7"]);
        assert_eq!(batch.formula_text(0, 0, 3).unwrap(), "=SUM(A2:A22)");
        assert_eq!(batch.formula_text(0, 1, 3).unwrap(), "=A4");
        assert_eq!(batch.formula_text(0, 2, 3).unwrap(), "=#REF!+A6");
        assert!(batch.undo());
        assert_eq!(batch.sheets[0].cells, build().sheets[0].cells);
    }

    #[test]
    fn status_bar_counts_what_is_shown() {
        let mut w = filtered();
        let st = w.selection_stats(0, &[rect("B2:B6")]);
        assert_eq!((st.count, st.numeric_count, st.sum, st.min, st.max), (3, 3, 21.0, 5.0, 9.0));
        w.set_rows_hidden(0, 5, 5, true);
        let st = w.selection_stats(0, &[rect("A1:B6")]);
        assert_eq!((st.count, st.numeric_count, st.sum), (6, 2, 12.0));
        w.clear_filters(0, FilterScope::Sheet);
        let st = w.selection_stats(0, &[rect("B2:B6")]);
        assert_eq!((st.count, st.sum), (4, 15.0));
    }

    #[test]
    fn region_of_a_cell_outside_the_data() {
        let w = wb(&[("A1", "1"), ("B2", "2")]);
        assert_eq!(w.current_region(0, 0, 0), rect("A1:B2"));
        assert_eq!(w.current_region(0, 2, 2), rect("A1:C3"));
        assert_eq!(w.current_region(0, 9, 9), rect("J10"));
    }
}
