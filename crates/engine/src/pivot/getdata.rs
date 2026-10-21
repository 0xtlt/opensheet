//! `GETPIVOTDATA`: reading a value out of a PivotTable by the names of its
//! fields and items rather than by cell address.

use super::cache::item_key;
use super::compute::{Line, LineKind, Tag, Tree};
use super::{PivotTable, same_name};
use crate::locale::Locale;
use crate::value::{ErrorKind, R, Value};
use crate::workbook::Workbook;

/// The item of `field` the formula means by `wanted`: the same value, or
/// the same text as the label the report shows (a date typed as text, a
/// year group asked for as a number, a custom caption).
fn find_item(p: &PivotTable, field: usize, wanted: &Value, locale: &Locale) -> Option<u32> {
    let items = p.cache.items(field);
    let key = item_key(wanted);
    if let Some(x) = items.iter().position(|v| item_key(v) == key) {
        return Some(x as u32);
    }
    let text = match wanted {
        Value::Text(s) => s.to_string(),
        Value::Number(n) => locale.number_to_text(*n),
        _ => return None,
    };
    (0..items.len() as u32).find(|x| same_name(&p.item_text(field, *x, locale), &text))
}

/// The value the PivotTable at the cell `at` (sheet, row, column) shows for
/// `data_field` (the name of a value field, or of its source field) at the
/// given (field, item) pairs. Row and column fields left out are totalled,
/// which only works where the report shows that total. Every failure is
/// `#REF!`, as in Excel.
pub fn get_pivot_data(
    wb: &Workbook,
    at: (usize, u32, u32),
    data_field: &str,
    pairs: &[(String, Value)],
) -> R<Value> {
    let missing = ErrorKind::Ref;
    let p = wb.pivot_at(at.0, at.1, at.2).ok_or(missing)?;
    let report = &*p.report;
    let by_source = || {
        p.field_index(data_field)
            .and_then(|f| p.data_fields.iter().position(|d| d.field == f))
    };
    let d = p
        .data_fields
        .iter()
        .position(|d| same_name(&d.name, data_field))
        .or_else(by_source)
        .ok_or(missing)?;
    let fields =
        |axis: &[super::AxisField]| axis.iter().filter_map(|a| a.field()).collect::<Vec<_>>();
    let (row_fields, col_fields) = (fields(&p.row_fields), fields(&p.col_fields));
    let mut row_items: Vec<Option<u32>> = vec![None; row_fields.len()];
    let mut col_items: Vec<Option<u32>> = vec![None; col_fields.len()];
    for (name, wanted) in pairs {
        let f = p.field_index(name).ok_or(missing)?;
        let x = find_item(p, f, wanted, wb.locale).ok_or(missing)?;
        if let Some(k) = row_fields.iter().position(|rf| *rf == f) {
            row_items[k] = Some(x);
        } else if let Some(k) = col_fields.iter().position(|cf| *cf == f) {
            col_items[k] = Some(x);
        } else {
            // A filter field only answers for the item it shows.
            match p.page_fields.iter().find(|pg| pg.field == f) {
                Some(pg) if pg.item == Some(x) => {}
                _ => return Err(missing),
            }
        }
    }
    // The items given must lead down one branch from the outermost field:
    // the report has no total of an inner field across the outer ones.
    let node = |tree: &Tree, items: &[Option<u32>]| -> R<u32> {
        let mut n = 0;
        let mut ended = false;
        for item in items {
            match item {
                Some(x) if !ended => n = tree.find_child(n, *x).ok_or(missing)?,
                Some(_) => return Err(missing),
                None => ended = true,
            }
        }
        Ok(n)
    };
    let row_node = node(&report.row_tree, &row_items)?;
    let col_node = node(&report.col_tree, &col_items)?;
    let find = |lines: &[Line], n: u32| {
        lines.iter().position(|l| {
            l.values
                && l.kind != LineKind::Header
                && l.node == n
                && !l.is_custom()
                && l.data().is_none_or(|ld| ld == d)
        })
    };
    let r = find(&report.row_lines, row_node).ok_or(missing)? as u32;
    let c = find(&report.col_lines, col_node).ok_or(missing)? as u32;
    if report.tag(r, c) != Tag::Value || report.data_at(r, c, p) != Some(d) {
        return Err(missing);
    }
    match report.value(r, c) {
        Value::Empty => Err(missing),
        v => Ok(v.clone()),
    }
}
