//! The statements an action may have been, guessed from where the
//! selection and the clipboard were: rows, columns or cells inserted or
//! deleted, a fill, a paste, a sort, a clear, an autofit, a sheet copied.
//!
//! A guess costs a run of the script engine on a copy of the workbook, so
//! each kind is only proposed when the cells the action touched make it
//! plausible. None needs to be right: the recorder keeps a guess only
//! when it brings the copy closer to the workbook the action gave.

use super::Before;
use super::code::{Scope, Step, member, quote, runs_of};
use super::compare::{Formats, Pair, content_at, side_by_side};
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::{Content, Rect, Sheet, Value, Workbook};
use std::collections::BTreeSet;

/// The most candidates of one kind.
const MAX_SORT_KEYS: u32 = 16;
/// Beyond this many cells touched nothing is counted any more.
const MAX_TOUCHED: usize = 100_000;

/// The cells of a sheet an action touched: content and format.
struct Touched {
    contents: BTreeSet<(u32, u32)>,
    formats: BTreeSet<(u32, u32)>,
}

impl Touched {
    fn of(work: &Workbook, ws: &Sheet, after: &Workbook, ts: &Sheet) -> Touched {
        let mut formats = Formats::new(work, after);
        let mut touched = Touched {
            contents: BTreeSet::new(),
            formats: BTreeSet::new(),
        };
        for Pair { pos, contents, styles } in side_by_side(ws, ts) {
            if touched.contents.len() + touched.formats.len() > MAX_TOUCHED {
                break;
            }
            if contents.0 != contents.1 {
                touched.contents.insert(pos);
            }
            if !formats.same(styles.0, styles.1) {
                touched.formats.insert(pos);
            }
        }
        touched
    }

    fn all(&self) -> impl Iterator<Item = &(u32, u32)> {
        self.contents.iter().chain(&self.formats)
    }

    fn is_empty(&self) -> bool {
        self.contents.is_empty() && self.formats.is_empty()
    }

    fn within(&self, rect: Rect) -> bool {
        self.all().all(|(row, col)| rect.contains(*row, *col))
    }
}

fn whole_rows(rect: Rect) -> Rect {
    Rect::new(rect.r0, 0, rect.r1, MAX_COLS - 1)
}

fn whole_cols(rect: Rect) -> Rect {
    Rect::new(0, rect.c0, MAX_ROWS - 1, rect.c1)
}

/// How two values of a sorted column follow one another, as far as it
/// takes to tell which column a range was sorted on.
fn order(a: &Value, b: &Value) -> std::cmp::Ordering {
    let rank = |value: &Value| match value {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        _ => 4,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// The statements to try for the action that led from `work` to `after`,
/// the likeliest first. `before` is what the window showed when the
/// action started, `now` what it shows.
pub(super) fn of(work: &Workbook, after: &Workbook, before: &Before, now: &Before, scope: &Scope) -> Vec<Vec<Step>> {
    let mut out: Vec<Vec<Step>> = Vec::new();
    let names = |wb: &Workbook| wb.sheets.iter().map(|sheet| sheet.name.clone()).collect::<Vec<_>>();
    let Some(ws) = work.sheets.get(before.sheet) else {
        return out;
    };
    let sheet = scope.sheet(&ws.name);
    let on = scope.on(&ws.name);
    if names(work) != names(after) {
        // One sheet more, with something on it: a copy of the sheet that
        // was in front.
        if after.sheets.len() == work.sheets.len() + 1 && !ws.cells.is_empty() {
            out.push(vec![Step::new(format!("Duplicate worksheet {on}"), format!("{sheet}.copy({}, {sheet});", member("WorksheetPositionType", "after")))]);
        }
        return out;
    }
    let ts = &after.sheets[before.sheet];
    let touched = Touched::of(work, ws, after, ts);
    let selection = before.selection;
    let range = |rect: Rect| scope.range(&ws.name, rect);

    // Fewer conditional formats on the selection: Clear Rules.
    if ts.cond_formats.len() <= ws.cond_formats.len() && ts.cond_formats != ws.cond_formats {
        out.push(vec![Step::new(format!("Clear the conditional formats of range {} on {on}", selection.name()), format!("{}.clearAllConditionalFormats();", range(selection)))]);
    }

    if touched.is_empty() {
        // Sizes alone: columns or rows fitted to what they hold.
        let resized = |a: &std::collections::BTreeMap<u32, f32>, b: &std::collections::BTreeMap<u32, f32>| -> Vec<u32> {
            a.keys().chain(b.keys()).copied().collect::<BTreeSet<u32>>().into_iter().filter(|line| a.get(line) != b.get(line)).collect()
        };
        let columns = resized(&ws.col_widths, &ts.col_widths);
        let rows = resized(&ws.row_heights, &ts.row_heights);
        if !columns.is_empty() && rows.is_empty() {
            let steps = runs_of(columns)
                .into_iter()
                .map(|(first, last)| {
                    let rect = Rect::new(0, first, MAX_ROWS - 1, last);
                    Step::new(format!("Auto fit the columns of range {} on {on}", rect.name()), format!("{}.getFormat().autofitColumns();", range(rect)))
                })
                .collect();
            out.push(steps);
        } else if !rows.is_empty() && columns.is_empty() {
            let steps = runs_of(rows)
                .into_iter()
                .map(|(first, last)| {
                    let rect = Rect::new(first, 0, last, MAX_COLS - 1);
                    Step::new(format!("Auto fit the rows of range {} on {on}", rect.name()), format!("{}.getFormat().autofitRows();", range(rect)))
                })
                .collect();
            out.push(steps);
        }
        return out;
    }

    let inside = touched.within(selection);

    // A paste of the cells on the clipboard, where the selection was.
    if let Some((from, source, cut)) = before.clip {
        if let Some(from) = work.sheets.get(from) {
            let source_range = scope.range(&from.name, source);
            let from_words = format!("range {} on {}", source.name(), scope.on(&from.name));
            // Where the cells land: the selection, or as many cells from
            // its corner as were copied, either way round.
            let span = source.rows().max(source.cols()).max(selection.rows()).max(selection.cols());
            let reach = Rect::new(selection.r0, selection.c0, selection.r0.saturating_add(span - 1), selection.c0.saturating_add(span - 1));
            let into_selection = touched.within(reach) || (cut && touched.all().all(|(row, col)| reach.contains(*row, *col) || source.contains(*row, *col)));
            if cut && into_selection {
                out.push(vec![Step::new(
                    format!("Move {from_words} to range {} on {on}", selection.name()),
                    format!("{source_range}.moveTo({});", range(Rect::cell(selection.r0, selection.c0))),
                )]);
            } else if into_selection {
                let mut paste = |skip: bool, transpose: bool| {
                    for kind in ["all", "formulas", "values", "formats"] {
                        out.push(vec![Step::new(
                            format!("Paste to range {} on {on} from {from_words}", selection.name()),
                            format!("{}.copyFrom({source_range}, {}, {skip}, {transpose});", range(selection), member("RangeCopyType", kind)),
                        )]);
                    }
                };
                paste(false, false);
                paste(false, true);
                paste(true, false);
            }
        }
    }

    // The formats, or everything, cleared from the selection: what is
    // left there is formatted as cells never touched.
    let standard = after.styles.default_style();
    let bare = |pos: &(u32, u32)| super::compare::shown(after.styles.get(ts.style_id_at(pos.0, pos.1))) == super::compare::shown(standard);
    let emptied = touched.contents.iter().all(|pos| matches!(content_at(ts, *pos), Content::Empty));
    if inside && emptied && !touched.formats.is_empty() && touched.formats.iter().all(bare) {
        for (kind, words) in [("formats", "formats"), ("all", "contents and formats")] {
            out.push(vec![Step::new(format!("Clear {words} of range {} on {on}", selection.name()), format!("{}.clear({});", range(selection), member("ClearApplyTo", kind)))]);
        }
    }

    // The fill handle dragged from the selection: the selection grew.
    let grown = now.selection;
    if now.sheet == before.sheet && grown != selection && grown.intersection(&selection) == Some(selection) && touched.within(grown) {
        out.push(vec![Step::new(
            format!("Auto fill range {} on {on}", grown.name()),
            format!("{}.autoFill({}, {});", range(selection), quote(&grown.name()), member("AutoFillType", "fillDefault")),
        )]);
    }
    // Fill Down and Fill Right: the first row or column of the selection
    // copied over the others.
    if inside && !touched.contents.is_empty() {
        if selection.rows() > 1 && touched.all().all(|(row, _)| *row > selection.r0) {
            let (first, rest) = (Rect::new(selection.r0, selection.c0, selection.r0, selection.c1), Rect::new(selection.r0 + 1, selection.c0, selection.r1, selection.c1));
            out.push(vec![Step::new(format!("Fill down range {} on {on}", selection.name()), format!("{}.copyFrom({});", range(rest), range(first)))]);
        }
        if selection.cols() > 1 && touched.all().all(|(_, col)| *col > selection.c0) {
            let (first, rest) = (Rect::new(selection.r0, selection.c0, selection.r1, selection.c0), Rect::new(selection.r0, selection.c0 + 1, selection.r1, selection.c1));
            out.push(vec![Step::new(format!("Fill right range {} on {on}", selection.name()), format!("{}.copyFrom({});", range(rest), range(first)))]);
        }
    }

    // A sort of the table, or of the block of data, the active cell is
    // in: on a column the action left in order.
    if touched.contents.len() >= 2 {
        let table = work.table_at(before.sheet, before.active.0, before.active.1);
        let region = match table {
            Some(table) => Rect::new(table.range.r0, table.range.c0, table.range.r1 - table.totals_row as u32, table.range.c1),
            None if selection.is_single() => work.current_region(before.sheet, before.active.0, before.active.1),
            None => selection,
        };
        if touched.within(region) && region.rows() > 1 {
            for key in 0..region.cols().min(MAX_SORT_KEYS) {
                let col = region.c0 + key;
                // Whether the column reads in order from `first` down.
                let ordered = |first: u32, descending: bool| {
                    let mut strict = false;
                    let fine = (first..region.r1).all(|row| {
                        let (a, b) = (ts.value_at(row, col), ts.value_at(row + 1, col));
                        let way = if descending { order(&b, &a) } else { order(&a, &b) };
                        strict |= way == std::cmp::Ordering::Less;
                        way != std::cmp::Ordering::Greater
                    });
                    fine && strict
                };
                for descending in [false, true] {
                    match table {
                        Some(table) => {
                            if ordered(region.r0 + table.header_row as u32, descending) {
                                out.push(vec![Step::new(
                                    format!("Sort table {}", table.name),
                                    format!("workbook.getTable({}).getSort().apply([{{ key: {key}, ascending: {} }}], false);", quote(&table.name), !descending),
                                )]);
                            }
                        }
                        None => {
                            for headers in [true, false] {
                                if ordered(region.r0 + headers as u32, descending) {
                                    out.push(vec![Step::new(
                                        format!("Sort the range {} on {on}", region.name()),
                                        format!("{}.getSort().apply([{{ key: {key}, ascending: {} }}], false, {headers});", range(region), !descending),
                                    )]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let filled = now.sheet == before.sheet && grown != selection && touched.within(grown);
    if !inside && !filled {
        // Cells moved beyond the selection: rows, columns or cells inserted
        // or deleted where it was.
        let (down, right) = (member("InsertShiftDirection", "down"), member("InsertShiftDirection", "right"));
        let (up, left) = (member("DeleteShiftDirection", "up"), member("DeleteShiftDirection", "left"));
        let rows = whole_rows(selection);
        let cols = whole_cols(selection);
        let insert = |rect: Rect, shift: &str, words: &str| {
            vec![Step::new(format!("Insert at range {} on {on}, move existing cells {words}", rect.name()), format!("{}.insert({shift});", range(rect)))]
        };
        let delete = |rect: Rect, shift: &str, words: &str| vec![Step::new(format!("Delete range {} on {on}, move existing cells {words}", rect.name()), format!("{}.delete({shift});", range(rect)))];
        if !selection.is_whole_cols() {
            out.push(insert(rows, &down, "down"));
            out.push(delete(rows, &up, "up"));
        }
        if !selection.is_whole_rows() {
            out.push(insert(cols, &right, "right"));
            out.push(delete(cols, &left, "left"));
        }
        if !selection.is_whole_rows() && !selection.is_whole_cols() {
            out.push(insert(selection, &down, "down"));
            out.push(insert(selection, &right, "right"));
            out.push(delete(selection, &up, "up"));
            out.push(delete(selection, &left, "left"));
        }
    }
    out
}
