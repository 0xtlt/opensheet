//! `RangeAreas`: several ranges of one worksheet held together, as a
//! selection made with ⌘ held, the blanks of a range or the cells of a
//! conditional format.

use serde_json::Value as J;

use opensheet_engine::refs::Rect;

use super::util::{address, range_from, resolve_areas};
use super::{ApiClass, Call, Res, Ret, class, object, state_of};
use crate::host::Host;

/// A RangeAreas object: the sheet, then four numbers per range.
pub(super) fn areas_object(h: &Host, sheet: usize, rects: &[Rect]) -> J {
    let mut state = vec![J::from(h.sheet_id(sheet))];
    for rect in rects {
        state.extend([rect.r0, rect.c0, rect.r1, rect.c1].map(J::from));
    }
    object("RangeAreas", state)
}

/// The sheet and the ranges of a RangeAreas object, from its state.
pub(super) fn areas_from(h: &Host, call: &Call, state: &[J]) -> Res<(usize, Vec<Rect>)> {
    let id = state.first().and_then(J::as_i64).ok_or_else(|| call.invalid())?;
    let sheet = h.sheet(call, id)?;
    let numbers: Vec<u32> = state[1..].iter().filter_map(|n| n.as_u64().map(|n| n as u32)).collect();
    if numbers.is_empty() || numbers.len() != state.len() - 1 || numbers.len() % 4 != 0 {
        return Err(call.invalid());
    }
    Ok((sheet, numbers.chunks(4).map(|n| Rect::new(n[0], n[1], n[2], n[3])).collect()))
}

pub(super) fn this_areas(h: &Host, call: &Call) -> Res<(usize, Vec<Rect>)> {
    areas_from(h, call, call.this)
}

/// An argument that is a Range, a RangeAreas or addresses separated by
/// commas (`A1:B2, D4`), read from `sheet`.
pub(super) fn areas_arg(h: &mut Host, call: &Call, index: usize, sheet: usize) -> Res<(usize, Vec<Rect>)> {
    let value = call.arg(index);
    if let J::String(text) = value {
        return resolve_areas(h, call, sheet, text);
    }
    if let Some(state) = state_of(value, "RangeAreas") {
        return areas_from(h, call, state);
    }
    match state_of(value, "Range") {
        Some(state) => range_from(h, call, state).map(|(sheet, rect)| (sheet, vec![rect])),
        None => Err(call.invalid()),
    }
}

/// Cells as the fewest ranges a reader would name: runs down a column,
/// joined across columns when they span the same rows.
pub(super) fn rects_of_cells(mut cells: Vec<(u32, u32)>) -> Vec<Rect> {
    cells.sort_unstable_by_key(|(row, col)| (*col, *row));
    cells.dedup();
    let mut runs: Vec<Rect> = Vec::new();
    for (row, col) in cells {
        match runs.last_mut() {
            Some(run) if run.c0 == col && run.r1 + 1 == row => run.r1 = row,
            _ => runs.push(Rect::cell(row, col)),
        }
    }
    let mut out: Vec<Rect> = Vec::new();
    for run in runs {
        match out.iter_mut().find(|rect| rect.r0 == run.r0 && rect.r1 == run.r1 && rect.c1 + 1 == run.c0) {
            Some(rect) => rect.c1 = run.c0,
            None => out.push(run),
        }
    }
    out.sort_unstable_by_key(|rect| (rect.r0, rect.c0));
    out
}

/// `Sheet1!A1:B2,Sheet1!D4`, the ranges parted as lists are in a language.
fn addresses(h: &Host, sheet: usize, rects: &[Rect], separator: char) -> String {
    let each: Vec<String> = rects.iter().map(|rect| address(h, sheet, *rect)).collect();
    each.join(&separator.to_string())
}

pub(super) const RANGE_AREAS: ApiClass = class!("RangeAreas", "Several ranges of one worksheet held together.", {
    getAddress "(): string" "The addresses of the ranges, separated by commas (Sheet1!A1:B2,Sheet1!D4)." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(addresses(h, sheet, &rects, ',').into())
    };
    getAddressLocal "(): string" "The addresses of the ranges, separated as lists are in the language of the workbook." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(addresses(h, sheet, &rects, h.wb.locale.arg_sep).into())
    };
    getAreaCount "(): number" "How many ranges there are." => |h, c| Ok((this_areas(h, c)?.1.len() as u64).into());
    getAreas "(): Range[]" "The ranges, one by one." => |h, c| {
        let (sheet, rects) = this_areas(h, c)?;
        Ok(Ret::Object(J::from(rects.into_iter().map(|rect| h.range(sheet, rect)).collect::<Vec<J>>())))
    };
    getCellCount "(): number" "How many cells the ranges hold together." => |h, c| {
        let cells: u64 = this_areas(h, c)?.1.iter().map(|rect| rect.rows() as u64 * rect.cols() as u64).sum();
        Ok(cells.into())
    };
    getWorksheet "(): Worksheet" "The worksheet of the ranges." => |h, c| {
        let (sheet, _) = this_areas(h, c)?;
        Ok(Ret::Object(h.worksheet(sheet)))
    };
});
