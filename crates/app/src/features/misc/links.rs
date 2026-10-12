//! Which cells a formula reads, and which formulas read a cell: what the
//! tracer arrows and the Precedents / Dependents of Go To Special are
//! built on.

use opensheet_engine::{Content, Expr, Reference, Workbook};

/// The cells an expression written in the cell (`sheet`, `row`, `col`)
/// refers to: its references, the cells its names and its structured
/// references stand for, every sheet of its 3-D references.
pub(crate) fn expr_refs(wb: &Workbook, sheet: usize, row: u32, col: u32, expr: &Expr) -> Vec<Reference> {
    let mut out: Vec<Reference> = Vec::new();
    let mut add = |reference: Reference| {
        if !out.contains(&reference) {
            out.push(reference);
        }
    };
    let named = |from: usize, name: &str, add: &mut dyn FnMut(Reference)| {
        // One area (or a table) is the common case; names standing for
        // several areas are read through the text parser.
        match wb.name_reference(from, row, col, name) {
            Some(reference) => add(reference),
            None if wb.lookup_name(from, name).is_some() => {
                for reference in wb.reference_areas(from, row, col, name).unwrap_or_default() {
                    add(reference);
                }
            }
            None => {}
        }
    };
    expr.walk(&mut |node| match node {
        Expr::Ref(reference) => {
            let on = match &reference.sheet {
                Some(name) => wb.sheet_index(name),
                None => Some(sheet),
            };
            if let Some(on) = on {
                add(Reference::new(on, reference.rect()));
            }
        }
        Expr::Ref3D(span) => {
            if let (Some(first), Some(last)) = (wb.sheet_index(&span.first), wb.sheet_index(&span.last)) {
                for on in first.min(last)..=first.max(last) {
                    add(Reference::new(on, span.cells.rect()));
                }
            }
        }
        Expr::Name(name) => named(sheet, name, &mut add),
        Expr::SheetName(scope, name) => {
            if let Some(from) = wb.sheet_index(scope) {
                named(from, name, &mut add);
            }
        }
        Expr::Table(table) => {
            if let Ok(reference) = opensheet_engine::structured::resolve(wb, table, sheet, row, col) {
                add(reference);
            }
        }
        _ => {}
    });
    out
}

/// The cells the formula of a cell refers to; nothing for a cell without
/// a formula.
pub(crate) fn precedents(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Vec<Reference> {
    match wb.cell(sheet, row, col).map(|cell| &cell.content) {
        Some(Content::Formula(expr)) => expr_refs(wb, sheet, row, col, expr),
        _ => Vec::new(),
    }
}

/// The cells (sheet, row, col) whose formula refers to at least one cell
/// of `target`, in sheet, row and column order.
pub(crate) fn dependents(wb: &Workbook, target: Reference) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    for (index, sheet) in wb.sheets.iter().enumerate() {
        for ((row, col), cell) in sheet.cells.iter() {
            let Content::Formula(expr) = &cell.content else {
                continue;
            };
            let reads = expr_refs(wb, index, *row, *col, expr)
                .iter()
                .any(|reference| reference.sheet == target.sheet && reference.rect.intersects(&target.rect));
            if reads {
                out.push((index, *row, *col));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::names::NameScope;
    use opensheet_engine::{Locale, Rect};

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        wb.add_sheet(2);
        let cells = [
            (0, "A1", "1"),
            (0, "A2", "2"),
            (0, "B1", "=A1*2"),
            (0, "B2", "=SUM(A1:A2)+Sheet2!C3"),
            (0, "C1", "=B1+Rate"),
            (0, "C2", "=SUM(Both)"),
            (0, "D1", "=SUM(Sheet2:Sheet3!A1)"),
            (1, "A1", "=Sheet1!B1"),
        ];
        for (sheet, cell, text) in cells {
            let (row, col) = opensheet_engine::refs::parse_cell_name(cell).unwrap();
            wb.set_cell_text(sheet, row, col, text).unwrap();
        }
        for (name, formula) in [("Rate", "=Sheet1!$A$2"), ("Both", "=(Sheet1!$A$1,Sheet1!$B$1:$B$2)")] {
            let expr = wb.parse_name_formula(formula, 0, 0, 0).unwrap();
            wb.add_name(NameScope::Workbook, name, expr, "").unwrap();
        }
        wb.commit("setup");
        wb
    }

    fn at(sheet: usize, range: &str) -> Reference {
        Reference::new(sheet, Rect::parse(range).unwrap())
    }

    #[test]
    fn precedents_follow_references_names_and_sheets() {
        let wb = workbook();
        assert_eq!(precedents(&wb, 0, 0, 1), [at(0, "A1")]);
        assert_eq!(precedents(&wb, 0, 1, 1), [at(0, "A1:A2"), at(1, "C3")]);
        // A name stands for its cells; several areas give several.
        assert_eq!(precedents(&wb, 0, 0, 2), [at(0, "B1"), at(0, "A2")]);
        assert_eq!(precedents(&wb, 0, 1, 2), [at(0, "A1"), at(0, "B1:B2")]);
        // A 3-D reference reads every sheet of its span.
        assert_eq!(precedents(&wb, 0, 0, 3), [at(1, "A1"), at(2, "A1")]);
        // Constants and blank cells read nothing.
        assert!(precedents(&wb, 0, 0, 0).is_empty());
        assert!(precedents(&wb, 0, 9, 9).is_empty());
    }

    #[test]
    fn dependents_are_found_on_every_sheet() {
        let wb = workbook();
        assert_eq!(dependents(&wb, at(0, "A1")), [(0, 0, 1), (0, 1, 1), (0, 1, 2)]);
        assert_eq!(dependents(&wb, at(0, "B1")), [(0, 0, 2), (0, 1, 2), (1, 0, 0)]);
        assert_eq!(dependents(&wb, at(0, "A2")), [(0, 0, 2), (0, 1, 1)]);
        assert_eq!(dependents(&wb, at(1, "A1")), [(0, 0, 3)]);
        assert!(dependents(&wb, at(0, "D9")).is_empty());
    }
}
