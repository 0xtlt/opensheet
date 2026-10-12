//! Data > Subtotal: a `SUBTOTAL(…)` row at each change in a column of a
//! list, a grand total, and the outline that folds the groups away.
//!
//! The list is a block of cells whose first row holds the column labels.
//! A row of the list is a subtotal row when one of its cells holds a
//! formula that is a call to `SUBTOTAL`: that is how "Replace current
//! subtotals" and "Remove All" find the rows to take away, as in Excel.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::ast::{Expr, RefExpr};
use crate::locale::Locale;
use crate::ops::sort_order;
use crate::refs::{MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Content, Workbook};

/// The eleven functions of the Subtotal dialog, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtotalFn {
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNumbers,
    StdDev,
    StdDevp,
    Var,
    Varp,
}

impl SubtotalFn {
    pub const ALL: [SubtotalFn; 11] = [
        SubtotalFn::Sum,
        SubtotalFn::Count,
        SubtotalFn::Average,
        SubtotalFn::Max,
        SubtotalFn::Min,
        SubtotalFn::Product,
        SubtotalFn::CountNumbers,
        SubtotalFn::StdDev,
        SubtotalFn::StdDevp,
        SubtotalFn::Var,
        SubtotalFn::Varp,
    ];

    /// The first argument of `SUBTOTAL` for this function.
    pub fn code(self) -> u8 {
        match self {
            SubtotalFn::Average => 1,
            SubtotalFn::CountNumbers => 2,
            SubtotalFn::Count => 3,
            SubtotalFn::Max => 4,
            SubtotalFn::Min => 5,
            SubtotalFn::Product => 6,
            SubtotalFn::StdDev => 7,
            SubtotalFn::StdDevp => 8,
            SubtotalFn::Sum => 9,
            SubtotalFn::Var => 10,
            SubtotalFn::Varp => 11,
        }
    }

    fn index(self) -> usize {
        SubtotalFn::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// Excel's wording of the labels, per language: the word that goes with
/// the value of a group ("North Total"), the label of the last row
/// ("Grand Total"), and whether the value comes first.
struct Words {
    value_first: bool,
    group: [&'static str; 11],
    grand: [&'static str; 11],
}

fn words(locale: &Locale) -> Words {
    match locale.id {
        "fr" => Words {
            value_first: false,
            group: ["Total", "Nombre", "Moyenne", "Max", "Min", "Produit", "Nombre", "Écartype", "Écartypep", "Var", "Varp"],
            grand: ["Total général", "Nbval", "Moyenne", "Max", "Min", "Produit", "Nbval", "Écartype", "Écartypep", "Var", "Varp"],
        },
        "es" => Words {
            value_first: false,
            group: ["Total", "Cuenta", "Promedio", "Máx", "Mín", "Producto", "Cuenta", "Desvest", "Desvestp", "Var", "Varp"],
            grand: [
                "Total general",
                "Cuenta general",
                "Promedio general",
                "Máximo general",
                "Mínimo general",
                "Producto general",
                "Cuenta general",
                "Desvest general",
                "Desvestp general",
                "Var general",
                "Varp general",
            ],
        },
        "de" => Words {
            value_first: true,
            group: [
                "Ergebnis",
                "Anzahl",
                "Mittelwert",
                "Max",
                "Min",
                "Produkt",
                "Anzahl",
                "STABW",
                "Standardabweichung (Grundgesamtheit)",
                "Var",
                "Varianz (Grundgesamtheit)",
            ],
            grand: [
                "Gesamtergebnis",
                "Gesamtanzahl",
                "Gesamtmittelwert",
                "Gesamtmaximum",
                "Gesamtminimum",
                "Gesamtprodukt",
                "Gesamtanzahl",
                "Gesamtstandardabweichung (Stichprobe)",
                "Gesamtstandardabweichung (Grundgesamtheit)",
                "Gesamtvarianz",
                "Gesamtvarianz",
            ],
        },
        "it" => Words {
            value_first: true,
            group: [
                "Totale",
                "Conteggio",
                "Media",
                "Max",
                "Min",
                "Prodotto",
                "Conteggio",
                "Dev. standard",
                "Dev. standard pop.",
                "Varianza",
                "Varianza pop.",
            ],
            grand: [
                "Totale complessivo",
                "Conta comp.",
                "Media comp.",
                "Max comp.",
                "Min comp.",
                "Prodotto comp.",
                "Conta comp.",
                "Dev.st comp.",
                "Dev.st.pop comp.",
                "Var comp.",
                "Var.pop comp.",
            ],
        },
        _ => Words {
            value_first: true,
            group: ["Total", "Count", "Average", "Max", "Min", "Product", "Count", "StdDev", "StdDevp", "Var", "Varp"],
            grand: [
                "Grand Total",
                "Grand Count",
                "Grand Average",
                "Grand Max",
                "Grand Min",
                "Grand Product",
                "Grand Count",
                "Grand StdDev",
                "Grand StdDevp",
                "Grand Var",
                "Grand Varp",
            ],
        },
    }
}

/// The label of the subtotal row of a group ("North Total", "Total Nord").
pub fn group_label(value: &str, function: SubtotalFn, locale: &Locale) -> String {
    let w = words(locale);
    let word = w.group[function.index()];
    let joined = if w.value_first { format!("{value} {word}") } else { format!("{word} {value}") };
    joined.trim().to_string()
}

/// The label of the last row ("Grand Total").
pub fn grand_label(function: SubtotalFn, locale: &Locale) -> &'static str {
    words(locale).grand[function.index()]
}

/// What the Subtotal dialog asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct SubtotalSpec {
    /// "At each change in": a column of the list.
    pub group_by: u32,
    /// "Use function".
    pub function: SubtotalFn,
    /// "Add subtotal to": columns of the list.
    pub columns: Vec<u32>,
    /// "Replace current subtotals".
    pub replace: bool,
    /// "Page break between groups".
    pub page_breaks: bool,
    /// "Summary below data".
    pub summary_below: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtotalError {
    /// The list has no rows under its labels.
    NoList,
    /// Tables do not take subtotals.
    InTable,
    /// No column was ticked, or one lies outside the list.
    NoColumns,
    /// The rows to add do not fit on the sheet.
    NoRoom,
}

/// `SUBTOTAL(code, cells)`.
fn subtotal_formula(function: SubtotalFn, cells: Rect) -> Expr {
    let range = if cells.is_single() { RefExpr::cell(cells.r0, cells.c0) } else { RefExpr::range(cells) };
    Expr::Func(Arc::from("SUBTOTAL"), vec![Expr::Number(function.code() as f64), Expr::Ref(range)])
}

/// The cells a formula that is a call to `SUBTOTAL` reads, when it is one.
fn subtotal_cells(content: &Content) -> Option<Option<Rect>> {
    let Content::Formula(formula) = content else {
        return None;
    };
    let mut expr: &Expr = formula;
    while let Expr::Paren(inner) = expr {
        expr = inner;
    }
    match expr {
        Expr::Func(name, args) if &**name == "SUBTOTAL" => Some(match args.get(1) {
            Some(Expr::Ref(r)) => Some(r.rect()),
            _ => None,
        }),
        _ => None,
    }
}

impl Workbook {
    /// The list Subtotal works on from a cell: the block of cells around
    /// it, labels in its first row. `InTable` inside a table, `NoList`
    /// when there is nothing under the labels.
    pub fn subtotal_list(&self, sheet: usize, row: u32, col: u32) -> Result<Rect, SubtotalError> {
        if self.table_at(sheet, row, col).is_some() {
            return Err(SubtotalError::InTable);
        }
        let list = self.current_region(sheet, row, col);
        if list.rows() < 2 || self.sheets[sheet].tables.iter().any(|t| t.range.intersects(&list)) {
            return Err(if list.rows() < 2 { SubtotalError::NoList } else { SubtotalError::InTable });
        }
        Ok(list)
    }

    /// The rows of the list that hold subtotals, with the rows each one
    /// reads (when its formula names a range).
    fn subtotal_rows(&self, sheet: usize, list: Rect) -> Vec<(u32, Option<(u32, u32)>)> {
        let sh = &self.sheets[sheet];
        let mut rows: Vec<(u32, Option<(u32, u32)>)> = Vec::new();
        for ((row, col), cell) in sh.cells.range((list.r0 + 1, 0)..=(list.r1, u32::MAX)) {
            if *col < list.c0 || *col > list.c1 || rows.last().is_some_and(|(r, _)| r == row) {
                continue;
            }
            if let Some(read) = subtotal_cells(&cell.content) {
                rows.push((*row, read.map(|r| (r.r0, r.r1))));
            }
        }
        rows
    }

    /// True when the list holds subtotal rows.
    pub fn has_subtotals(&self, sheet: usize, list: Rect) -> bool {
        sheet < self.sheets.len() && !self.subtotal_rows(sheet, list).is_empty()
    }

    /// "Remove All": the subtotal rows of the list go, with the outline
    /// and the page breaks of its rows. Returns the list that is left.
    pub fn remove_subtotals(&mut self, sheet: usize, list: Rect) -> Rect {
        if sheet >= self.sheets.len() {
            return list;
        }
        self.snapshot();
        let rows: Vec<u32> = self.subtotal_rows(sheet, list).into_iter().map(|(row, _)| row).collect();
        let sh = &mut self.sheets[sheet];
        // Past the list too: a grand total leaves its mark on the row
        // after it.
        let last = list.r1.saturating_add(1).min(MAX_ROWS - 1);
        for row in list.r0..=last {
            sh.outline.rows.levels.remove(&row);
            sh.outline.rows.collapsed.remove(&row);
            sh.hidden_rows.remove(&row);
            if row > list.r0 {
                sh.page.row_breaks.remove(&row);
            }
        }
        for row in rows.iter().rev() {
            self.delete_rows(sheet, *row, 1);
        }
        Rect::new(list.r0, list.c0, list.r1 - rows.len() as u32, list.c1)
    }

    /// Data > Subtotal on `list` (labels in its first row). Returns the
    /// list with its new rows. One undo step with the caller's `commit`.
    pub fn subtotal(&mut self, sheet: usize, list: Rect, spec: &SubtotalSpec) -> Result<Rect, SubtotalError> {
        let inside = |col: u32| col >= list.c0 && col <= list.c1;
        if sheet >= self.sheets.len() || list.rows() < 2 {
            return Err(SubtotalError::NoList);
        }
        if self.sheets[sheet].tables.iter().any(|t| t.range.intersects(&list)) {
            return Err(SubtotalError::InTable);
        }
        if spec.columns.is_empty() || !spec.columns.iter().copied().all(inside) || !inside(spec.group_by) {
            return Err(SubtotalError::NoColumns);
        }
        self.snapshot();
        let list = if spec.replace { self.remove_subtotals(sheet, list) } else { list };
        if list.rows() < 2 {
            return Err(SubtotalError::NoList);
        }
        let (first, last) = (list.r0 + 1, list.r1);
        let existing = self.subtotal_rows(sheet, list);
        let is_subtotal = |row: u32| existing.iter().any(|(r, _)| *r == row);

        // The runs of rows with the same value in the grouping column; a
        // subtotal row already there ends a run.
        let mut runs: Vec<(u32, u32, String)> = Vec::new();
        let mut current: Option<(u32, u32, Value)> = None;
        for row in first..=last {
            if is_subtotal(row) {
                if let Some((a, b, _)) = current.take() {
                    runs.push((a, b, self.display_text(sheet, a, spec.group_by).text));
                }
                continue;
            }
            let value = self.value_at(sheet, row, spec.group_by);
            match &mut current {
                Some((_, b, key)) if same_group(key, &value) => *b = row,
                _ => {
                    if let Some((a, b, _)) = current.take() {
                        runs.push((a, b, self.display_text(sheet, a, spec.group_by).text));
                    }
                    current = Some((row, row, value));
                }
            }
        }
        if let Some((a, b, _)) = current {
            runs.push((a, b, self.display_text(sheet, a, spec.group_by).text));
        }
        if runs.is_empty() {
            return Err(SubtotalError::NoList);
        }
        let added = runs.len() as u32 + 1;
        if list.r1 as u64 + added as u64 >= MAX_ROWS as u64 {
            return Err(SubtotalError::NoRoom);
        }

        // Where the grand total goes: at the end of the list (at its top
        // when summaries are above), beside those already there.
        let whole = |read: &Option<(u32, u32)>| read.is_some_and(|(a, b)| a <= first + 1 && b + 1 >= last);
        let below = spec.summary_below;
        let grand_at = if below {
            existing.iter().find(|(_, read)| whole(read)).map_or(last + 1, |(row, _)| *row)
        } else {
            first
        };
        let detail_level = self.sheets[sheet].outline.rows.level(runs[0].0);

        // Insert top to bottom, each row at the place it ends up at.
        let mut shift = 0u32;
        let mut grand_row = None;
        if !below {
            self.insert_rows(sheet, grand_at, 1);
            grand_row = Some(grand_at);
            shift = 1;
        }
        // (subtotal row, first and last row of its group, label)
        let mut placed: Vec<(u32, u32, u32, &str)> = Vec::new();
        for (a, b, label) in &runs {
            let (a, b) = (a + shift, b + shift);
            if below {
                self.insert_rows(sheet, b + 1, 1);
                placed.push((b + 1, a, b, label.as_str()));
            } else {
                self.insert_rows(sheet, a, 1);
                placed.push((a, a + 1, b + 1, label.as_str()));
            }
            shift += 1;
        }
        let grand_row = grand_row.unwrap_or_else(|| {
            self.insert_rows(sheet, grand_at + shift, 1);
            grand_at + shift
        });
        let end = list.r1 + added;

        // Labels and formulas.
        let label_col = (!spec.columns.contains(&spec.group_by)).then_some(spec.group_by);
        let locale = self.locale;
        let write = |wb: &mut Workbook, row: u32, label: String, cells: (u32, u32)| {
            if let Some(col) = label_col {
                wb.set_value(sheet, row, col, Value::text(label));
                wb.update_style(sheet, Rect::cell(row, col), &|s| s.font.bold = true);
            }
            for &col in &spec.columns {
                wb.set_formula(sheet, row, col, subtotal_formula(spec.function, Rect::new(cells.0, col, cells.1, col)));
            }
        };
        for (row, a, b, label) in &placed {
            write(self, *row, group_label(label, spec.function, locale), (*a, *b));
        }
        let all = if below { (first, grand_row - 1) } else { (grand_row + 1, end) };
        write(self, grand_row, grand_label(spec.function, locale).to_string(), all);

        // The outline: details one level under their subtotal, the grand
        // total over everything.
        let (details, totals) = if detail_level == 0 { (2, 1) } else { (detail_level + 1, detail_level) };
        let mut levels: Vec<(u32, u32, u8)> = Vec::new();
        for (row, a, b, _) in &placed {
            levels.push((*a, *b, details));
            levels.push((*row, *row, totals));
        }
        levels.push((grand_row, grand_row, 0));
        self.sheets[sheet].outline.summary_below = below;
        self.set_row_levels(sheet, &levels);

        // A page break before each group but the first.
        if spec.page_breaks {
            let breaks: Vec<u32> = placed.iter().skip(1).map(|(row, a, _, _)| if below { *a } else { *row }).collect();
            self.update_page_setup(sheet, |p| p.row_breaks.extend(breaks));
        }
        Ok(Rect::new(list.r0, list.c0, end, list.c1))
    }
}

/// Two values of the grouping column that belong to one group: equal as
/// a sort sees them, whatever the case.
fn same_group(a: &Value, b: &Value) -> bool {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => true,
        (false, false) => sort_order(a, b, false) == Ordering::Equal && !matches!(a, Value::Error(_)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::outline::Axis;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    /// Region, Item, Qty, Amount over six rows in A1:D7.
    fn list(locale: &'static Locale) -> Workbook {
        let mut wb = Workbook::new(locale);
        let rows = [
            ["Region", "Item", "Qty", "Amount"],
            ["North", "a", "1", "10"],
            ["North", "b", "2", "20"],
            ["north", "c", "3", "30"],
            ["South", "a", "4", "40"],
            ["South", "b", "5", "50"],
            ["West", "a", "6", "60"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("setup");
        wb
    }

    fn spec(columns: &[u32]) -> SubtotalSpec {
        SubtotalSpec {
            group_by: 0,
            function: SubtotalFn::Sum,
            columns: columns.to_vec(),
            replace: true,
            page_breaks: false,
            summary_below: true,
        }
    }

    fn text(wb: &Workbook, cell: &str) -> String {
        let (r, c) = crate::refs::parse_cell_name(cell).unwrap();
        wb.display_text(0, r, c).text
    }

    fn formula(wb: &Workbook, cell: &str) -> String {
        let (r, c) = crate::refs::parse_cell_name(cell).unwrap();
        wb.formula_text(0, r, c).unwrap_or_default()
    }

    fn levels(wb: &Workbook, lo: u32, hi: u32) -> Vec<u8> {
        (lo..=hi).map(|r| wb.sheets[0].outline.rows.level(r)).collect()
    }

    #[test]
    fn subtotals_at_each_change_with_a_grand_total() {
        let mut wb = list(Locale::en());
        let done = wb.subtotal(0, rect("A1:D7"), &spec(&[2, 3])).unwrap();
        wb.commit("Subtotal");
        assert_eq!(done, rect("A1:D11"));
        // Case does not make a new group.
        assert_eq!(text(&wb, "A5"), "North Total");
        assert_eq!(formula(&wb, "D5"), "=SUBTOTAL(9,D2:D4)");
        assert_eq!(formula(&wb, "C5"), "=SUBTOTAL(9,C2:C4)");
        assert_eq!(text(&wb, "D5"), "60");
        assert_eq!((text(&wb, "A8"), formula(&wb, "D8")), ("South Total".into(), "=SUBTOTAL(9,D6:D7)".into()));
        // A group of one row reads one cell.
        assert_eq!((text(&wb, "A10"), formula(&wb, "D10")), ("West Total".into(), "=SUBTOTAL(9,D9)".into()));
        assert_eq!((text(&wb, "A11"), formula(&wb, "D11")), ("Grand Total".into(), "=SUBTOTAL(9,D2:D10)".into()));
        assert_eq!(text(&wb, "D11"), "210");
        assert!(wb.style_at(0, 4, 0).font.bold && !wb.style_at(0, 4, 3).font.bold);
        // The outline of levels 1-2-3.
        assert_eq!(levels(&wb, 0, 11), [0, 2, 2, 2, 1, 2, 2, 1, 2, 1, 0, 0]);
        assert!(wb.sheets[0].outline.summary_below);
        assert_eq!(wb.sheets[0].outline.rows.groups(0, 20).len(), 4);
        // One step back.
        assert!(wb.undo());
        assert_eq!(text(&wb, "A5"), "South");
        assert!(wb.sheets[0].outline.is_empty());
    }

    #[test]
    fn replacing_and_removing() {
        let mut wb = list(Locale::en());
        wb.subtotal(0, rect("A1:D7"), &spec(&[3])).unwrap();
        wb.commit("Subtotal");
        // Again with another function: the first ones go.
        let again = SubtotalSpec {
            function: SubtotalFn::Average,
            ..spec(&[3])
        };
        let list = wb.subtotal_list(0, 2, 1).unwrap();
        assert_eq!(list, rect("A1:D11"));
        assert_eq!(wb.subtotal(0, list, &again).unwrap(), rect("A1:D11"));
        wb.commit("Subtotal");
        assert_eq!((text(&wb, "A5"), formula(&wb, "D5")), ("North Average".into(), "=SUBTOTAL(1,D2:D4)".into()));
        assert_eq!(text(&wb, "A11"), "Grand Average");
        assert_eq!(levels(&wb, 0, 11), [0, 2, 2, 2, 1, 2, 2, 1, 2, 1, 0, 0]);

        assert!(wb.has_subtotals(0, rect("A1:D11")));
        assert_eq!(wb.remove_subtotals(0, rect("A1:D11")), rect("A1:D7"));
        wb.commit("Remove");
        assert_eq!(text(&wb, "A5"), "South");
        assert_eq!(text(&wb, "A8"), "");
        assert!(wb.sheets[0].outline.is_empty());
        assert!(!wb.has_subtotals(0, rect("A1:D7")));
    }

    #[test]
    fn nested_subtotals_add_a_level() {
        let mut wb = list(Locale::en());
        wb.subtotal(0, rect("A1:D7"), &spec(&[3])).unwrap();
        wb.commit("Subtotal");
        let by_item = SubtotalSpec {
            group_by: 1,
            function: SubtotalFn::Count,
            replace: false,
            ..spec(&[3])
        };
        let done = wb.subtotal(0, rect("A1:D11"), &by_item).unwrap();
        wb.commit("Subtotal");
        // Three rows in the North group, each an item of its own.
        assert_eq!(done, rect("A1:D18"));
        assert_eq!((text(&wb, "B3"), formula(&wb, "D3")), ("a Count".into(), "=SUBTOTAL(3,D2)".into()));
        assert_eq!(text(&wb, "A8"), "North Total");
        // Its formula grew with the rows added inside its group.
        assert_eq!(formula(&wb, "D8"), "=SUBTOTAL(9,D2:D6)");
        assert_eq!(levels(&wb, 1, 7), [3, 2, 3, 2, 3, 2, 1]);
        // The new grand row comes before the one that was there.
        assert_eq!((text(&wb, "B17"), text(&wb, "A18")), ("Grand Count".into(), "Grand Total".into()));
        assert_eq!(formula(&wb, "D17"), "=SUBTOTAL(3,D2:D16)");
        assert_eq!(wb.remove_subtotals(0, done), rect("A1:D7"));
    }

    #[test]
    fn summary_above_and_page_breaks() {
        let mut wb = list(Locale::en());
        let above = SubtotalSpec {
            summary_below: false,
            page_breaks: true,
            ..spec(&[3])
        };
        assert_eq!(wb.subtotal(0, rect("A1:D7"), &above).unwrap(), rect("A1:D11"));
        wb.commit("Subtotal");
        assert_eq!((text(&wb, "A2"), formula(&wb, "D2")), ("Grand Total".into(), "=SUBTOTAL(9,D3:D11)".into()));
        assert_eq!((text(&wb, "A3"), formula(&wb, "D3")), ("North Total".into(), "=SUBTOTAL(9,D4:D6)".into()));
        assert_eq!((text(&wb, "A7"), formula(&wb, "D7")), ("South Total".into(), "=SUBTOTAL(9,D8:D9)".into()));
        assert_eq!(levels(&wb, 0, 10), [0, 0, 1, 2, 2, 2, 1, 2, 2, 1, 2]);
        assert!(!wb.sheets[0].outline.summary_below);
        let north = wb.sheets[0].outline.rows.group_at(3, 2).unwrap();
        assert_eq!(wb.sheets[0].outline_summary(Axis::Rows, &north), Some(2));
        // A break above each group after the first.
        assert_eq!(wb.sheets[0].page.row_breaks.iter().copied().collect::<Vec<_>>(), [6, 9]);
        wb.remove_subtotals(0, rect("A1:D11"));
        assert!(wb.sheets[0].page.row_breaks.is_empty());
    }

    #[test]
    fn labels_follow_the_language_and_tables_refuse() {
        let mut wb = list(Locale::fr());
        wb.subtotal(0, rect("A1:D7"), &spec(&[3])).unwrap();
        wb.commit("Sous-total");
        assert_eq!((text(&wb, "A5"), text(&wb, "A11")), ("Total North".into(), "Total général".into()));
        assert_eq!(formula(&wb, "D5"), "=SOUS.TOTAL(9;D2:D4)");
        assert_eq!(group_label("Nord", SubtotalFn::Sum, Locale::de()), "Nord Ergebnis");
        assert_eq!(group_label("Nord", SubtotalFn::Average, Locale::es()), "Promedio Nord");
        assert_eq!(group_label("Nord", SubtotalFn::Count, Locale::it()), "Nord Conteggio");
        assert_eq!(grand_label(SubtotalFn::Max, Locale::de()), "Gesamtmaximum");

        let mut wb = list(Locale::en());
        assert_eq!(wb.subtotal(0, rect("A1:D7"), &spec(&[])), Err(SubtotalError::NoColumns));
        assert_eq!(wb.subtotal(0, rect("A1:D7"), &spec(&[9])), Err(SubtotalError::NoColumns));
        assert_eq!(wb.subtotal_list(0, 20, 20), Err(SubtotalError::NoList));
        wb.add_table(0, rect("A1:D7"), true).unwrap();
        assert_eq!(wb.subtotal_list(0, 2, 1), Err(SubtotalError::InTable));
        assert_eq!(wb.subtotal(0, rect("A1:D7"), &spec(&[3])), Err(SubtotalError::InTable));
    }
}
