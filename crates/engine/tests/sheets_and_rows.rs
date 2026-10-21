//! Rows a filter hides under formatting, pasting and Find; what follows
//! when a sheet is deleted or moved, or when a table shifts cells; and the
//! questions protection is asked before an edit.

mod common;

use common::{at, enter, num, shown, value};
use opensheet_engine::comment::Person;
use opensheet_engine::condfmt::{CondFormat, HighlightFormat, Rule, parse_operand};
use opensheet_engine::features::Edit;
use opensheet_engine::filter::{Comparison, Condition, Criteria, CustomFilter, FilterScope};
use opensheet_engine::ops::{BorderPreset, FindOptions, PasteMode};
use opensheet_engine::protect::{Denied, Permission, SheetProtection};
use opensheet_engine::style::Border;
use opensheet_engine::table::TableOption;
use opensheet_engine::validation::{Operator, Validation, ValidationKind};
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{Locale, Rect, Value, Workbook};

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn cells(names: &[&str]) -> Vec<(u32, u32)> {
    names.iter().map(|name| at(name)).collect()
}

/// A list in A1:B6.
fn list() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let rows = [
        ("Item", "Qty"),
        ("a", "5"),
        ("b", "1"),
        ("c", "7"),
        ("d", "2"),
        ("e", "9"),
    ];
    for (r, (item, qty)) in rows.iter().enumerate() {
        wb.set_cell_text(0, r as u32, 0, item).unwrap();
        wb.set_cell_text(0, r as u32, 1, qty).unwrap();
    }
    wb.commit("list");
    wb
}

/// Filter the list to the rows where Qty > 2: rows 3 and 5 (as numbered
/// on screen) are hidden.
fn filter(wb: &mut Workbook) {
    wb.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
    let more = Condition::new(Comparison::Greater, "2", Locale::en());
    wb.set_filter_criteria(
        0,
        FilterScope::Sheet,
        1,
        Some(Criteria::Custom(CustomFilter::one(more))),
    );
    wb.commit("filter");
    assert_eq!(
        wb.sheets[0]
            .filtered_rows
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [2, 4]
    );
}

fn filtered() -> Workbook {
    let mut wb = list();
    filter(&mut wb);
    wb
}

fn bold(wb: &Workbook, name: &str) -> bool {
    let (row, col) = at(name);
    wb.style_at(0, row, col).font.bold
}

/// Which sides of a cell have a border: top, bottom.
fn edges(wb: &Workbook, name: &str) -> (bool, bool) {
    let (row, col) = at(name);
    let borders = &wb.style_at(0, row, col).borders;
    (!borders.top.is_none(), !borders.bottom.is_none())
}

#[test]
fn formats_skip_the_rows_a_filter_hides() {
    let mut wb = filtered();
    // A selection over the list: only what is shown is formatted.
    wb.update_style(0, rect("A2:B6"), &|s| s.font.bold = true);
    wb.commit("bold");
    let rows = ["A2", "A3", "A4", "A5", "A6"];
    assert_eq!(
        rows.map(|cell| bold(&wb, cell)),
        [true, false, true, false, true]
    );
    // Rows hidden by hand are formatted like the others.
    wb.set_rows_hidden(0, 5, 5, true);
    wb.update_style(0, rect("A6:B6"), &|s| s.font.italic = true);
    assert!(wb.style_at(0, 5, 0).font.italic);

    // Whole columns: the hidden rows keep their look, cells typed later
    // further down take the column's.
    let mut wb = filtered();
    wb.update_style(0, rect("B:C"), &|s| s.font.bold = true);
    wb.commit("bold");
    assert_eq!(
        ["B2", "B3", "B5", "B6"].map(|cell| bold(&wb, cell)),
        [true, false, false, true]
    );
    assert!(bold(&wb, "B40") && !bold(&wb, "A40"));
    wb.clear_filters(0, FilterScope::Sheet);
    wb.commit("show all");
    assert_eq!(["B3", "B4"].map(|cell| bold(&wb, cell)), [false, true]);
    // Whole rows.
    let mut wb = filtered();
    wb.update_style(0, rect("2:4"), &|s| s.font.bold = true);
    assert_eq!(
        ["A2", "A3", "A4", "K2", "K3"].map(|cell| bold(&wb, cell)),
        [true, false, true, true, false]
    );

    // Borders: each block of shown rows is outlined on its own.
    let mut wb = filtered();
    wb.apply_borders(0, rect("A2:B6"), BorderPreset::Outside, Border::thin());
    wb.commit("borders");
    assert_eq!(
        ["A2", "A4", "A6"].map(|cell| edges(&wb, cell)),
        [(true, true); 3]
    );
    assert_eq!(
        ["A3", "A5"].map(|cell| edges(&wb, cell)),
        [(false, false); 2]
    );
    // Taking them off leaves the hidden neighbours alone.
    let mut wb = list();
    wb.apply_borders(0, rect("A2:B6"), BorderPreset::All, Border::thin());
    filter(&mut wb);
    wb.apply_borders(0, rect("A4:B4"), BorderPreset::None, Border::thin());
    wb.commit("no borders");
    assert_eq!(
        ["A3", "A4", "A5"].map(|cell| edges(&wb, cell)),
        [(true, true), (false, false), (true, true)]
    );

    // Clear Formats and Clear All: the same rows, and what they carry.
    let mut wb = filtered();
    for row in 1..6 {
        wb.set_note(0, row, 0, "Ada", "note");
    }
    wb.clear(0, rect("A2:B6"), ClearMode::All);
    wb.commit("clear");
    let left: Vec<(u32, u32)> = wb.sheets[0].comment_cells(None);
    assert_eq!(left, cells(&["A3", "A5"]));
    assert_eq!(
        [value(&wb, 0, "A3"), value(&wb, 0, "A4")],
        [Value::text("b"), Value::Empty]
    );
}

#[test]
fn pasting_into_a_filtered_list() {
    // One cell over a selection: the rows that are shown, as in Excel.
    let mut wb = filtered();
    enter(&mut wb, 0, "D1", "x");
    wb.set_note(0, 0, 3, "Ada", "copied");
    wb.commit("setup");
    let one = wb.copy(0, rect("D1"));
    assert_eq!(wb.paste_area(0, rect("A2:A6"), &one), rect("A2:A6"));
    assert_eq!(
        wb.paste_blocks(0, rect("A2:A6"), &one),
        [rect("A2"), rect("A4"), rect("A6")]
    );
    wb.paste(0, rect("A2:A6"), &one, PasteMode::All);
    wb.commit("paste");
    let column = |wb: &Workbook| ["A2", "A3", "A4", "A5", "A6"].map(|cell| value(wb, 0, cell));
    let (x, b, d) = (Value::text("x"), Value::text("b"), Value::text("d"));
    assert_eq!(
        column(&wb),
        [x.clone(), b.clone(), x.clone(), d.clone(), x.clone()]
    );
    assert_eq!(
        wb.sheets[0].comment_cells(None),
        cells(&["D1", "A2", "A4", "A6"])
    );
    assert!(wb.undo());
    // Ctrl+D goes the same way.
    wb.fill_down(0, rect("D1:D6"));
    wb.commit("fill");
    assert_eq!(
        wb.sheets[0].comment_cells(None),
        cells(&["D1", "D2", "D4", "D6"])
    );
    assert_eq!(
        [value(&wb, 0, "D3"), value(&wb, 0, "D4")],
        [Value::Empty, x.clone()]
    );

    // Several rows are laid on consecutive rows, hidden or not.
    let mut wb = filtered();
    enter(&mut wb, 0, "D1", "x");
    enter(&mut wb, 0, "D2", "y");
    let two = wb.copy(0, rect("D1:D2"));
    assert_eq!(wb.paste(0, rect("A2"), &two, PasteMode::All), rect("A2:A3"));
    assert_eq!(
        [value(&wb, 0, "A2"), value(&wb, 0, "A3")],
        [x, Value::text("y")]
    );

    // What was copied from a filtered list comes without its hidden rows,
    // with what the rows that are shown carry.
    let mut wb = filtered();
    wb.set_note(0, 2, 0, "Ada", "hidden");
    wb.set_note(0, 3, 0, "Ada", "shown");
    let mut whole = Validation {
        kind: ValidationKind::Whole,
        operator: Operator::GreaterThan,
        ..Default::default()
    };
    whole.set_operands("0", "", Locale::en()).unwrap();
    wb.set_validation(0, &[rect("B3:B4")], (2, 1), whole);
    wb.commit("setup");
    let list = wb.copy(0, rect("A1:B6"));
    assert_eq!(
        wb.paste(0, rect("E1"), &list, PasteMode::All),
        rect("E1:F4")
    );
    wb.commit("paste");
    assert_eq!(value(&wb, 0, "E3"), Value::text("c"));
    assert_eq!(wb.sheets[0].comment_cells(None), cells(&["A3", "E3", "A4"]));
    let validated: Vec<bool> = (0..4)
        .map(|row| wb.validation_at(0, row, 5).is_some())
        .collect();
    assert_eq!(validated, [false, false, true, false]);

    // Cutting takes the whole block, hidden rows included, as in Excel.
    let mut wb = filtered();
    wb.move_range(0, rect("A2:A6"), 0, 1, 3);
    wb.commit("cut");
    assert_eq!(
        [value(&wb, 0, "D3"), value(&wb, 0, "A3")],
        [b, Value::Empty]
    );
}

/// A rule that fills what is over 5, on `range`.
fn over_five(range: &str) -> CondFormat {
    let rule = Rule::greater_than(opensheet_engine::Expr::Number(5.0));
    CondFormat::new(vec![rect(range)], rule).with_format(HighlightFormat::LightRedFill.patch())
}

fn positive() -> Validation {
    let mut rule = Validation {
        kind: ValidationKind::Whole,
        operator: Operator::GreaterThan,
        ..Default::default()
    };
    rule.set_operands("0", "", Locale::en()).unwrap();
    rule
}

/// Which of the rows 1 to 7 of a column are under a conditional format,
/// which under a validation rule, which carry a note.
fn rules_down(wb: &Workbook, col: u32) -> [Vec<u32>; 3] {
    let rows =
        |test: &dyn Fn(u32) -> bool| (0..7).filter(|row| test(*row)).map(|row| row + 1).collect();
    [
        rows(&|row| !wb.cond_formats_in(0, &[Rect::cell(row, col)]).is_empty()),
        rows(&|row| wb.validation_at(0, row, col).is_some()),
        rows(&|row| wb.comment_at(0, row, col).is_some()),
    ]
}

#[test]
fn rules_laid_over_a_filtered_list_stay_off_its_hidden_rows() {
    let mut wb = filtered();
    // C2 alone has a rule, a list and a note; D has a rule all the way.
    for cell in ["C2", "D2", "E2"] {
        enter(&mut wb, 0, cell, "8");
    }
    wb.add_cond_format(0, over_five("D:D"));
    wb.add_cond_format(0, over_five("C2"));
    wb.set_validation(0, &[rect("C2")], (1, 2), positive());
    wb.set_note(0, 1, 2, "Ada", "note");
    wb.commit("setup");

    // Ctrl+D down the list: the rows that are shown take them.
    wb.fill_down(0, rect("C2:D6"));
    wb.commit("fill");
    let shown = vec![2, 4, 6];
    assert_eq!(
        rules_down(&wb, 2),
        [shown.clone(), shown.clone(), shown.clone()]
    );
    assert_eq!(
        wb.cond_formats(0)[0].ranges,
        [rect("C2"), rect("C4"), rect("C6")]
    );
    assert_eq!(
        wb.validation_at(0, 1, 2).unwrap().ranges,
        [rect("C2"), rect("C4"), rect("C6")]
    );
    // The rule of the whole column is still one range.
    assert_eq!(wb.cond_formats(0)[1].ranges, [rect("D:D")]);
    assert_ne!(wb.display(0, 3, 3).style.fill, None);
    assert!(wb.undo());
    assert_eq!(rules_down(&wb, 2), [vec![2], vec![2], vec![2]]);

    // One cell pasted over the column, whole or one choice of Paste
    // Special at a time, and the format painter.
    let cell = wb.copy(0, rect("C2"));
    wb.paste(0, rect("E2:E6"), &cell, PasteMode::Validation);
    wb.paste(0, rect("F2:F6"), &cell, PasteMode::Formats);
    wb.paste(0, rect("G2:G6"), &cell, PasteMode::Comments);
    wb.paste(0, rect("H2:H6"), &cell, PasteMode::All);
    wb.commit("paste");
    assert_eq!(rules_down(&wb, 4), [vec![], shown.clone(), vec![]]);
    assert_eq!(rules_down(&wb, 5), [shown.clone(), vec![], vec![]]);
    assert_eq!(rules_down(&wb, 6), [vec![], vec![], shown.clone()]);
    assert_eq!(
        rules_down(&wb, 7),
        [shown.clone(), shown.clone(), shown.clone()]
    );
    assert_eq!(
        [value(&wb, 0, "H4"), value(&wb, 0, "H5")],
        [num(8.0), Value::Empty]
    );

    // Clearing takes the rules off the rows that are shown only.
    wb.clear(0, rect("D2:H6"), ClearMode::Formats);
    wb.commit("clear");
    assert_eq!(rules_down(&wb, 3)[0], [1, 3, 5, 7]);
    assert_eq!(rules_down(&wb, 7), [vec![], shown.clone(), shown.clone()]);
    wb.clear(0, rect("D2:H6"), ClearMode::All);
    wb.commit("clear");
    for col in 4..8 {
        assert_eq!(rules_down(&wb, col), [vec![], vec![], vec![]], "{col}");
    }
    // The fill handle steps over the hidden rows with them.
    wb.autofill(0, rect("C2"), rect("C2:C6"));
    wb.commit("fill");
    assert_eq!(
        rules_down(&wb, 2),
        [shown.clone(), shown.clone(), shown.clone()]
    );
}

#[test]
fn a_long_filtered_list_takes_rules_block_by_block_all_the_same() {
    // Two rows shown out of every three, down three thousand rows: what
    // one fill over all the blocks leaves is what copying onto each block
    // in turn leaves.
    const ROWS: u32 = 3000;
    let build = || {
        let mut wb = Workbook::new(Locale::en());
        enter(&mut wb, 0, "A1", "n");
        for row in 1..ROWS {
            wb.set_value(0, row, 0, Value::Number((row % 3) as f64));
        }
        for cell in ["B2", "C2"] {
            enter(&mut wb, 0, cell, "8");
        }
        wb.add_cond_format(0, over_five("B2:C2"));
        wb.add_cond_format(0, over_five("C:C"));
        wb.set_validation(0, &[rect("B2")], (1, 1), positive());
        wb.commit("setup");
        wb.toggle_auto_filter(0, Rect::cell(0, 0)).unwrap();
        let not_zero = Condition::new(Comparison::NotEquals, "0", Locale::en());
        wb.set_filter_criteria(
            0,
            FilterScope::Sheet,
            0,
            Some(Criteria::Custom(CustomFilter::one(not_zero))),
        );
        wb.commit("filter");
        wb
    };
    let mut at_once = build();
    at_once.fill_down(0, Rect::new(1, 1, ROWS - 1, 2));
    at_once.commit("fill");
    let mut in_turn = build();
    let blocks = in_turn.sheets[0].shown_parts(Rect::new(2, 1, ROWS - 1, 2));
    assert_eq!(blocks.len(), 1000);
    for block in blocks {
        in_turn.copy_cond_formats(0, rect("B2:C2"), 0, block);
        for row in block.r0..=block.r1 {
            in_turn.copy_validation(0, rect("B2:C2"), 0, row, 1);
        }
    }
    in_turn.commit("fill");
    for row in 0..ROWS {
        for col in 1..3 {
            let cell = [Rect::cell(row, col)];
            assert_eq!(
                at_once.cond_formats_in(0, &cell),
                in_turn.cond_formats_in(0, &cell),
                "{row}, {col}"
            );
            assert_eq!(
                at_once.validation_at(0, row, col).is_some(),
                in_turn.validation_at(0, row, col).is_some(),
                "{row}, {col}"
            );
        }
    }
    // Hidden rows have the rule of the whole column only.
    assert_eq!(at_once.cond_formats_in(0, &[Rect::cell(3, 2)]).len(), 1);
    assert_eq!(at_once.cond_formats_in(0, &[Rect::cell(4, 2)]).len(), 2);
    assert!(at_once.validation_at(0, 3, 1).is_none() && at_once.validation_at(0, 4, 1).is_some());
    // The rule of the whole column (the last made, so the first) is as it
    // was; the rule over the two columns has one range for each block.
    assert_eq!(at_once.cond_formats(0)[0].ranges, [rect("C:C")]);
    assert_eq!(at_once.cond_formats(0)[1].ranges.len(), 1000);
    assert_eq!(
        at_once.cond_formats(0)[1].ranges[..2],
        [rect("B2:C3"), rect("B5:C6")]
    );
}

#[test]
fn find_and_replace_pass_over_rows_that_are_not_shown() {
    let mut wb = filtered();
    enter(&mut wb, 0, "C3", "=B3+1");
    enter(&mut wb, 0, "C4", "=B4+1");
    wb.commit("formulas");
    let values = FindOptions::default();
    let formulas = FindOptions {
        look_in: opensheet_engine::find::LookIn::Formulas,
        ..Default::default()
    };
    // Rows a filter hides are never searched.
    assert_eq!(wb.find_all(0, "b", &values), Vec::<(u32, u32)>::new());
    assert_eq!(wb.find_all(0, "+1", &formulas), cells(&["C4"]));
    assert_eq!(wb.find_next(0, at("A2"), "d", &values), None);
    assert_eq!(wb.find_next(0, at("A2"), "e", &values), Some(at("A6")));
    // Rows and columns hidden by hand are, when looking in formulas.
    wb.set_rows_hidden(0, 3, 3, true);
    wb.set_cols_hidden(0, 1, 1, true);
    assert_eq!(wb.find_all(0, "c", &values), Vec::<(u32, u32)>::new());
    assert_eq!(wb.find_all(0, "9", &values), Vec::<(u32, u32)>::new());
    assert_eq!(wb.find_all(0, "c", &formulas), cells(&["A4"]));
    assert_eq!(wb.find_all(0, "9", &formulas), cells(&["B6"]));
    // Replace All leaves the filtered rows as they are.
    wb.set_rows_hidden(0, 3, 3, false);
    assert_eq!(wb.replace_all(0, "+1", "+2", &values), 1);
    wb.commit("replace");
    assert_eq!(
        [shown(&wb, 0, "C3"), shown(&wb, 0, "C4")],
        ["=B3+1", "=B4+2"]
    );
}

/// Sheet1 with a rule and a list that read Sheet2, on a workbook of three.
fn linked() -> Workbook {
    let mut wb = common::workbook(3);
    enter(&mut wb, 1, "A1", "3");
    enter(&mut wb, 0, "A1", "5");
    let limit = parse_operand("=Sheet2!$A$1", Locale::en(), (0, 0), (0, 0)).unwrap();
    let over = CondFormat::new(vec![rect("A1:A3")], Rule::greater_than(limit));
    wb.add_cond_format(0, over.with_format(HighlightFormat::LightRedFill.patch()));
    let mut list = Validation {
        kind: ValidationKind::List,
        ..Default::default()
    };
    list.set_operands("=Sheet2!$A$1:$A$3", "", Locale::en())
        .unwrap();
    wb.set_validation(0, &[rect("B1")], (0, 1), list);
    wb.commit("setup");
    wb
}

fn rule_text(wb: &Workbook, sheet: usize) -> String {
    match &wb.cond_formats(sheet)[0].rule {
        Rule::CellIs { first, .. } | Rule::Formula(first) => first.to_text(Locale::en()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn deleting_a_sheet_reaches_every_feature() {
    let mut wb = linked();
    assert_ne!(wb.display(0, 0, 0).style.fill, None);
    assert!(wb.delete_sheet(1));
    wb.commit("delete");
    // The rule and the list read cells that are no more.
    assert_eq!(rule_text(&wb, 0), "#REF!$A$1");
    assert_eq!(
        wb.validation_at(0, 0, 1)
            .unwrap()
            .operand1_text(0, 1, Locale::en()),
        "=#REF!$A$1:$A$3"
    );
    assert_eq!(wb.display(0, 0, 0).style.fill, None);
    assert!(wb.undo());
    assert_eq!(rule_text(&wb, 0), "Sheet2!$A$1");
    assert_ne!(wb.display(0, 0, 0).style.fill, None);

    // Deleting the sheet before the one with the rules: they are still
    // found on what is now the first sheet.
    let mut wb = linked();
    wb.move_sheet(0, 2);
    wb.commit("move");
    assert_ne!(wb.display(2, 0, 0).style.fill, None);
    assert!(wb.delete_sheet(1));
    wb.commit("delete");
    assert_ne!(wb.display(1, 0, 0).style.fill, None);
    assert_eq!(wb.display(0, 0, 0).style.fill, None);
}

#[test]
fn moving_a_sheet_reaches_every_feature() {
    let mut wb = common::workbook(4);
    for sheet in 1..4 {
        enter(&mut wb, sheet, "A1", "1");
    }
    let mut custom = Validation {
        kind: ValidationKind::Custom,
        ..Default::default()
    };
    custom
        .set_operands("=SUM(Sheet2:Sheet4!$A$1)>2", "", Locale::en())
        .unwrap();
    wb.set_validation(0, &[rect("A1")], (0, 0), custom);
    let total = parse_operand("=SUM(Sheet2:Sheet4!$A$1)", Locale::en(), (0, 0), (0, 0)).unwrap();
    wb.add_cond_format(0, CondFormat::new(vec![rect("A1")], Rule::Formula(total)));
    wb.commit("setup");
    // The last sheet of the span is carried before the first: the span
    // lets go of it, in rules as in the formulas of cells.
    wb.move_sheet(3, 0);
    wb.commit("move");
    assert_eq!(rule_text(&wb, 1), "SUM(Sheet2:Sheet3!$A$1)");
    let rule = wb.validation_at(1, 0, 0).unwrap();
    assert_eq!(
        rule.operand1_text(0, 0, Locale::en()),
        "=SUM(Sheet2:Sheet3!$A$1)>2"
    );
    assert!(wb.undo());
    assert_eq!(rule_text(&wb, 0), "SUM(Sheet2:Sheet4!$A$1)");
}

#[test]
fn table_rows_and_columns_carry_what_sits_on_their_cells() {
    let mut wb = Workbook::new(Locale::en());
    let rows = [
        ["Item", "Qty", "Price"],
        ["nut", "1", "10"],
        ["bolt", "2", "20"],
        ["gear", "3", "30"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    let id = wb.add_table(0, rect("A1:C4"), true).unwrap();
    enter(&mut wb, 0, "B7", "under");
    enter(&mut wb, 0, "E3", "beside");
    wb.set_note(0, 2, 1, "Ada", "on bolt");
    wb.add_comment(0, 6, 1, &Person::new("Ada"), 45000.0, "under the table");
    wb.set_note(0, 2, 4, "Ada", "beside the table");
    let mut whole = Validation {
        kind: ValidationKind::Whole,
        operator: Operator::GreaterThan,
        ..Default::default()
    };
    whole.set_operands("0", "", Locale::en()).unwrap();
    wb.set_validation(0, &[rect("B2:B4")], (1, 1), whole);
    let dear = CondFormat::new(
        vec![rect("C2:C4")],
        Rule::greater_than(opensheet_engine::Expr::Number(15.0)),
    );
    wb.add_cond_format(0, dear.with_format(HighlightFormat::LightRedFill.patch()));
    // A list elsewhere whose entries are cells of the table.
    let mut list = Validation {
        kind: ValidationKind::List,
        ..Default::default()
    };
    list.set_operands("=$A$3:$A$4", "", Locale::en()).unwrap();
    wb.set_validation(0, &[rect("G1")], (0, 6), list);
    wb.commit("setup");
    let notes = |wb: &Workbook| wb.sheets[0].comment_cells(None);
    let source = |wb: &Workbook| {
        wb.validation_at(0, 0, 6)
            .unwrap()
            .operand1_text(0, 6, Locale::en())
    };

    // A table row above "bolt": only the cells of the table's columns
    // move down, and with them what they carry.
    wb.table_insert_rows(0, id, 2, 1).unwrap();
    wb.commit("insert");
    assert_eq!(notes(&wb), cells(&["E3", "B4", "B8"]));
    assert_eq!(wb.validation_at(0, 4, 1).unwrap().ranges, [rect("B2:B5")]);
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C5")]);
    assert_eq!(source(&wb), "=$A$4:$A$5");
    assert_ne!(wb.display(0, 3, 2).style.fill, None);
    assert_eq!(value(&wb, 0, "B8"), Value::text("under"));
    // Deleting it again, then the row of the note.
    wb.table_delete_rows(0, id, 2, 1).unwrap();
    wb.commit("delete");
    assert_eq!(notes(&wb), cells(&["B3", "E3", "B7"]));
    assert_eq!(wb.validation_at(0, 3, 1).unwrap().ranges, [rect("B2:B4")]);
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C4")]);
    assert_eq!(source(&wb), "=$A$3:$A$4");
    wb.table_delete_rows(0, id, 2, 1).unwrap();
    wb.commit("delete");
    assert_eq!(notes(&wb), cells(&["E3", "B6"]));
    assert_eq!(wb.validation_at(0, 1, 1).unwrap().ranges, [rect("B2:B3")]);
    assert!(wb.undo());
    assert_eq!(notes(&wb), cells(&["B3", "E3", "B7"]));

    // A table column before Qty: the cells of the table's rows move right.
    wb.table_insert_cols(0, id, 1, 1).unwrap();
    wb.commit("insert");
    assert_eq!(notes(&wb), cells(&["C3", "F3", "B7"]));
    assert_eq!(wb.validation_at(0, 2, 2).unwrap().ranges, [rect("C2:C4")]);
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("D2:D4")]);
    assert_eq!(value(&wb, 0, "F3"), Value::text("beside"));
    wb.table_delete_cols(0, id, 1, 1).unwrap();
    wb.commit("delete");
    assert_eq!(notes(&wb), cells(&["B3", "E3", "B7"]));
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C4")]);

    // New table rows look like their neighbours whatever is around: one
    // inserted at the top takes the rules of the row under it, not of the
    // header, and so does a row typed under the table.
    wb.table_insert_rows(0, id, 1, 1).unwrap();
    assert_eq!(wb.validation_at(0, 1, 1).unwrap().ranges, [rect("B2:B5")]);
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C5")]);
    wb.table_delete_rows(0, id, 1, 1).unwrap();
    enter(&mut wb, 0, "C5", "40");
    assert_eq!(wb.tables_after_edit(0, rect("C5")).expanded, Some(id));
    wb.commit("row");
    assert_eq!(wb.validation_at(0, 4, 1).unwrap().ranges, [rect("B2:B5")]);
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C5")]);
    assert_ne!(wb.display(0, 4, 2).style.fill, None);
    assert!(wb.undo());
    // The totals row pushes down what is in its way under the table,
    // notes included, and is no data row: the rules stop above it.
    enter(&mut wb, 0, "A5", "in the way");
    wb.set_note(0, 4, 0, "Ada", "pushed");
    wb.set_table_option(0, id, TableOption::TotalsRow, true)
        .unwrap();
    wb.commit("totals");
    assert_eq!(notes(&wb), cells(&["B3", "E3", "A6", "B8"]));
    assert_eq!(
        [value(&wb, 0, "A6"), value(&wb, 0, "C5")],
        [Value::text("in the way"), num(60.0)]
    );
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C2:C4")]);
    assert_eq!(wb.validation_at(0, 1, 1).unwrap().ranges, [rect("B2:B4")]);
}

#[test]
fn what_protection_is_asked_before_each_edit() {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [
        ("A1", "locked"),
        ("B1", "open"),
        ("B2", "open too"),
        ("D1", "x"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    wb.update_style(0, rect("B1:B5"), &|s| s.locked = false);
    wb.commit("setup");
    let one = wb.copy(0, rect("D1"));
    let two = wb.copy(0, rect("D1:E1"));
    let all = FindOptions::default();
    let asks = |wb: &Workbook| {
        [
            wb.permits_edit(&Edit::Enter {
                sheet: 0,
                ranges: &[rect("B1")],
            }),
            wb.permits_edit(&Edit::Enter {
                sheet: 0,
                ranges: &[rect("A1:B1")],
            }),
            wb.permits_edit(&Edit::Clear {
                sheet: 0,
                ranges: &[rect("B1")],
                mode: ClearMode::Contents,
            }),
            wb.permits_edit(&Edit::Clear {
                sheet: 0,
                ranges: &[rect("B1")],
                mode: ClearMode::Formats,
            }),
            wb.permits_edit(&Edit::Clear {
                sheet: 0,
                ranges: &[rect("B1")],
                mode: ClearMode::All,
            }),
            wb.permits_edit(&Edit::Clear {
                sheet: 0,
                ranges: &[rect("B1")],
                mode: ClearMode::Comments,
            }),
            // One cell tiled over unlocked cells; two cells that run over.
            wb.permits_edit(&Edit::Paste {
                sheet: 0,
                dest: rect("B1:B4"),
                clip: &one,
                mode: PasteMode::All,
            }),
            wb.permits_edit(&Edit::Paste {
                sheet: 0,
                dest: rect("B1"),
                clip: &two,
                mode: PasteMode::Values,
            }),
            wb.permits_edit(&Edit::Paste {
                sheet: 0,
                dest: rect("B1"),
                clip: &one,
                mode: PasteMode::Formats,
            }),
            wb.permits_edit(&Edit::Paste {
                sheet: 0,
                dest: rect("B1"),
                clip: &one,
                mode: PasteMode::Validation,
            }),
            wb.permits_edit(&Edit::Paste {
                sheet: 0,
                dest: rect("B1"),
                clip: &one,
                mode: PasteMode::ColumnWidths,
            }),
            wb.permits_edit(&Edit::Move {
                src_sheet: 0,
                src: rect("B1"),
                dest_sheet: 0,
                dest_row: 3,
                dest_col: 1,
            }),
            wb.permits_edit(&Edit::Move {
                src_sheet: 0,
                src: rect("B1:B2"),
                dest_sheet: 0,
                dest_row: 4,
                dest_col: 1,
            }),
            wb.permits_edit(&Edit::Fill {
                sheet: 0,
                dest: rect("B1:B5"),
            }),
            wb.permits_edit(&Edit::Fill {
                sheet: 0,
                dest: rect("B1:C5"),
            }),
            wb.permits_edit(&Edit::Format { sheet: 0 }),
            wb.permits_edit(&Edit::Merge { sheet: 0 }),
            wb.permits_edit(&Edit::InsertRows { sheet: 0 }),
            wb.permits_edit(&Edit::DeleteRows {
                sheet: 0,
                at: 1,
                count: 2,
            }),
            wb.permits_edit(&Edit::DeleteCols {
                sheet: 0,
                at: 4,
                count: 1,
            }),
            wb.permits_edit(&Edit::SizeRows { sheet: 0 }),
            wb.permits_edit(&Edit::SizeCols { sheet: 0 }),
            wb.permits_edit(&Edit::Sort {
                sheet: 0,
                range: rect("B1:B5"),
            }),
            wb.permits_edit(&Edit::RemoveDuplicates { sheet: 0 }),
            // Only the cells that would change count.
            wb.permits_edit(&Edit::Replace {
                sheet: 0,
                needle: "open",
                options: &all,
            }),
            wb.permits_edit(&Edit::Replace {
                sheet: 0,
                needle: "e",
                options: &all,
            }),
        ]
    };
    // Nothing is refused on a sheet that is not protected.
    assert!(asks(&wb).iter().all(Result::is_ok));

    wb.protect_sheet(0, SheetProtection::default(), "");
    let (yes, no) = (Ok(()), Err(Denied::Sheet));
    assert_eq!(
        asks(&wb),
        [
            yes,
            no,
            yes,
            no,
            no,
            Err(Denied::Comment),
            yes,
            no,
            no,
            no,
            no,
            yes,
            no,
            yes,
            no,
            no,
            no,
            no,
            no,
            no,
            no,
            no,
            no,
            no,
            yes,
            no,
        ]
    );
    // With every permission of the dialog, what the cells' locks decide
    // stays refused: rows with locked cells cannot go, nor be sorted.
    wb.protect_sheet(0, SheetProtection::allowing(&Permission::ALL), "");
    assert_eq!(
        asks(&wb),
        [
            yes,
            no,
            yes,
            yes,
            yes,
            yes,
            yes,
            no,
            yes,
            no,
            yes,
            yes,
            no,
            yes,
            no,
            yes,
            no,
            yes,
            Err(Denied::LockedCells { rows: true }),
            Err(Denied::LockedCells { rows: false }),
            yes,
            yes,
            yes,
            no,
            yes,
            no,
        ]
    );
    // A paste over a filtered list is asked about the rows it writes: the
    // locked cells of the rows the filter hides are not in its way.
    let mut wb = filtered();
    wb.update_style(0, rect("A2"), &|s| s.locked = false);
    wb.update_style(0, rect("A4"), &|s| s.locked = false);
    wb.update_style(0, rect("A6"), &|s| s.locked = false);
    wb.commit("unlock");
    let one = wb.copy(0, rect("B1"));
    wb.protect_sheet(0, SheetProtection::default(), "");
    let paste = |dest: &str| Edit::Paste {
        sheet: 0,
        dest: rect(dest),
        clip: &one,
        mode: PasteMode::Values,
    };
    assert_eq!(wb.permits_edit(&paste("A2:A6")), Ok(()));
    assert_eq!(wb.permits_edit(&paste("A2:A3")), Ok(()));
    assert_eq!(wb.permits_edit(&paste("A3")), Err(Denied::Sheet));
    // The edits themselves never refuse: the app asks first.
    wb.clear(0, rect("A1"), ClearMode::Contents);
    assert!(wb.sheets[0].is_blank(0, 0));
}
