//! What travels with cells: conditional formats, data validation, notes
//! and comments follow copy, cut, paste, fill, clear and sort the way they
//! do in Excel.

mod common;

use common::{at, enter, num, shown, value};
use opensheet_engine::comment::Person;
use opensheet_engine::condfmt::{CondFormat, HighlightFormat, Rule};
use opensheet_engine::drawing::Frame;
use opensheet_engine::names::NameScope;
use opensheet_engine::ops::{BorderPreset, PasteMode, SortLevel, SortSpec};
use opensheet_engine::style::{Border, Color};
use opensheet_engine::validation::{Operator, Validation, ValidationKind};
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{Expr, Locale, Rect, Value, Workbook};

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn whole_numbers(first: &str, second: &str) -> Validation {
    let mut rule = Validation {
        kind: ValidationKind::Whole,
        operator: Operator::Between,
        ..Default::default()
    };
    rule.set_operands(first, second, Locale::en()).unwrap();
    rule
}

/// Whether a conditional format fills the cell.
fn filled(wb: &Workbook, sheet: usize, name: &str) -> bool {
    let (row, col) = at(name);
    wb.display(sheet, row, col).style.fill != wb.style_at(sheet, row, col).fill
}

/// What a cell carries besides its content: (is under a conditional
/// format, has a validation rule, text of its note or first comment).
fn carried(wb: &Workbook, sheet: usize, name: &str) -> (bool, bool, Option<String>) {
    let (row, col) = at(name);
    let formatted = !wb
        .cond_formats_in(sheet, &[Rect::cell(row, col)])
        .is_empty();
    let text = wb.comment_at(sheet, row, col).map(|comment| {
        match (comment.as_note(), comment.as_thread()) {
            (Some(note), _) => note.text(),
            (_, Some(thread)) => thread.first.text.clone(),
            _ => String::new(),
        }
    });
    (formatted, wb.validation_at(sheet, row, col).is_some(), text)
}

fn has(note: &str) -> (bool, bool, Option<String>) {
    (true, true, Some(note.to_string()))
}

const NOTHING: (bool, bool, Option<String>) = (false, false, None);

/// A1:A2 hold 7 and 9 under a rule that fills what is over 5, a rule
/// that only takes whole numbers from 1 to 10, a note and a comment.
fn decorated() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "7");
    enter(&mut wb, 0, "A2", "9");
    let over_five = CondFormat::new(vec![rect("A1:A2")], Rule::greater_than(Expr::Number(5.0)));
    assert!(wb.add_cond_format(
        0,
        over_five.with_format(HighlightFormat::LightRedFill.patch())
    ));
    assert!(wb.set_validation(0, &[rect("A1:A2")], (0, 0), whole_numbers("1", "10")));
    assert!(wb.set_note(0, 0, 0, "Ada", "first"));
    assert!(wb.add_comment(0, 1, 0, &Person::new("Ada"), 45000.5, "second"));
    wb.commit("setup");
    assert_eq!(
        [carried(&wb, 0, "A1"), carried(&wb, 0, "A2")],
        [has("first"), has("second")]
    );
    assert!(filled(&wb, 0, "A1"));
    wb
}

#[test]
fn paste_carries_what_excel_carries() {
    let mut wb = decorated();
    let clip = wb.copy(0, rect("A1:A2"));
    // Paste: everything, in every copy laid over the selection.
    assert_eq!(
        wb.paste(0, rect("C1:D4"), &clip, PasteMode::All),
        rect("C1:D4")
    );
    wb.commit("paste");
    for cell in ["C1", "D1", "C3", "D3"] {
        assert_eq!(carried(&wb, 0, cell), has("first"), "{cell}");
    }
    for cell in ["C2", "D4"] {
        assert_eq!(carried(&wb, 0, cell), has("second"), "{cell}");
    }
    assert_eq!(value(&wb, 0, "D4"), num(9.0));
    assert!(filled(&wb, 0, "D4"));
    // The rules gained the cells; nothing was made twice.
    assert_eq!(wb.cond_formats(0).len(), 1);
    assert_eq!(wb.sheets[0].validations.len(), 1);
    assert!(wb.undo());
    assert_eq!(carried(&wb, 0, "C1"), NOTHING);

    // To another sheet as well.
    let other = wb.add_sheet(1);
    wb.paste(other, rect("B2"), &clip, PasteMode::All);
    wb.commit("paste");
    assert_eq!(
        [carried(&wb, other, "B2"), carried(&wb, other, "B3")],
        [has("first"), has("second")]
    );

    // Pasting plain cells over them takes it all away again.
    let plain = wb.copy(0, rect("F1:F2"));
    wb.paste(other, rect("B2"), &plain, PasteMode::All);
    wb.commit("paste");
    assert_eq!(
        [carried(&wb, other, "B2"), carried(&wb, other, "B3")],
        [NOTHING, NOTHING]
    );
    assert!(wb.cond_formats(other).is_empty() && wb.sheets[other].validations.is_empty());
}

#[test]
fn each_choice_of_paste_special() {
    let mut wb = decorated();
    wb.update_style(0, rect("A1"), &|s| {
        s.font.bold = true;
        s.num_fmt = "0.00".into();
        s.fill = Some(Color(0x00FF00));
        s.borders.top = Border::thin();
    });
    enter(&mut wb, 0, "B1", "=A1*2");
    wb.set_col_width(0, 0, Some(120.0));
    for cell in ["C1", "D1", "E1", "F1", "G1", "H1", "I1", "J1"] {
        enter(&mut wb, 0, cell, "old");
    }
    wb.apply_borders(0, rect("C1:J1"), BorderPreset::Bottom, Border::thin());
    wb.commit("setup");
    let clip = wb.copy(0, rect("A1"));
    let style = |wb: &Workbook, cell: &str| {
        let (row, col) = at(cell);
        let s = wb.style_at(0, row, col);
        (
            s.font.bold,
            s.num_fmt.to_string(),
            s.borders.top.is_none(),
            s.borders.bottom.is_none(),
        )
    };
    let pasted = [
        ("C1", PasteMode::Formulas),
        ("D1", PasteMode::Values),
        ("E1", PasteMode::Formats),
        ("F1", PasteMode::Comments),
        ("G1", PasteMode::Validation),
        ("H1", PasteMode::AllExceptBorders),
        ("I1", PasteMode::FormulasAndNumberFormats),
        ("J1", PasteMode::ValuesAndNumberFormats),
    ];
    for (cell, mode) in pasted {
        assert_eq!(wb.paste(0, rect(cell), &clip, mode), rect(cell));
    }
    wb.commit("paste special");
    let note = Some("first".to_string());
    // Formulas and values: the content, nothing of the rest.
    assert_eq!(
        (value(&wb, 0, "C1"), carried(&wb, 0, "C1")),
        (num(7.0), NOTHING)
    );
    assert_eq!(
        style(&wb, "C1"),
        (false, "General".to_string(), true, false)
    );
    assert_eq!(
        (value(&wb, 0, "D1"), carried(&wb, 0, "D1")),
        (num(7.0), NOTHING)
    );
    // Formats: the look, conditional formats included, not the content.
    assert_eq!(value(&wb, 0, "E1"), Value::text("old"));
    assert_eq!(style(&wb, "E1"), (true, "0.00".to_string(), false, true));
    assert_eq!(carried(&wb, 0, "E1"), (true, false, None));
    // Comments and notes only; validation only.
    assert_eq!(carried(&wb, 0, "F1"), (false, false, note.clone()));
    assert_eq!(
        (value(&wb, 0, "F1"), style(&wb, "F1").0),
        (Value::text("old"), false)
    );
    assert_eq!(carried(&wb, 0, "G1"), (false, true, None));
    assert_eq!(value(&wb, 0, "G1"), Value::text("old"));
    // All except borders: the cell keeps the borders it had.
    assert_eq!(
        (value(&wb, 0, "H1"), carried(&wb, 0, "H1")),
        (num(7.0), (true, true, note))
    );
    assert_eq!(style(&wb, "H1"), (true, "0.00".to_string(), true, false));
    assert_eq!(wb.style_at(0, 0, 7).fill, Some(Color(0x00FF00)));
    // Number formats with the formulas or the values, no other format.
    assert_eq!(
        (value(&wb, 0, "I1"), carried(&wb, 0, "I1")),
        (num(7.0), NOTHING)
    );
    assert_eq!(style(&wb, "I1"), (false, "0.00".to_string(), true, false));
    assert_eq!(style(&wb, "J1"), (false, "0.00".to_string(), true, false));

    // A formula goes as a formula or as its value.
    let formula = wb.copy(0, rect("B1"));
    wb.paste(0, rect("C3"), &formula, PasteMode::FormulasAndNumberFormats);
    wb.paste(0, rect("D3"), &formula, PasteMode::ValuesAndNumberFormats);
    wb.commit("paste");
    assert_eq!([shown(&wb, 0, "C3"), shown(&wb, 0, "D3")], ["=B3*2", ""]);
    assert_eq!(value(&wb, 0, "D3"), num(14.0));

    // Column widths: the widths, tiled over the selection, and no cell.
    assert_eq!(
        wb.paste(0, rect("L1:M1"), &clip, PasteMode::ColumnWidths),
        rect("L1:M1")
    );
    wb.commit("widths");
    assert_eq!(
        [wb.sheets[0].col_width(11), wb.sheets[0].col_width(12)],
        [120.0, 120.0]
    );
    assert!(wb.cell(0, 0, 11).is_none());
    assert!(wb.undo());
    assert_ne!(wb.sheets[0].col_width(11), 120.0);
}

#[test]
fn the_format_painter_copies_merged_cells_and_conditional_formats() {
    let mut wb = decorated();
    wb.merge(0, rect("A4:B4"));
    wb.update_style(0, rect("A4"), &|s| s.font.italic = true);
    wb.commit("merge");
    let clip = wb.copy(0, rect("A1:B4"));
    wb.paste(0, rect("D1"), &clip, PasteMode::Formats);
    wb.commit("painter");
    assert_eq!(wb.sheets[0].merges, [rect("A4:B4"), rect("D4:E4")]);
    assert!(wb.style_at(0, 3, 3).font.italic);
    // The rule reaches the painted cells and judges their own values.
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("A1:A2"), rect("D1:D2")]);
    enter(&mut wb, 0, "D1", "8");
    enter(&mut wb, 0, "D2", "2");
    wb.commit("values");
    assert_eq!([filled(&wb, 0, "D1"), filled(&wb, 0, "D2")], [true, false]);
    assert_eq!(carried(&wb, 0, "D1"), (true, false, None));
}

#[test]
fn cut_and_paste_moves_everything_and_what_points_at_it() {
    let mut wb = decorated();
    enter(&mut wb, 0, "E1", "=SUM(A1:A2)");
    let refers_to = wb.parse_name_formula("=Sheet1!$A$1:$A$2", 0, 0, 0).unwrap();
    wb.add_name(NameScope::Workbook, "Pair", refers_to, "")
        .unwrap();
    // A list in G1 that takes its entries from the cells about to move.
    let mut list = Validation {
        kind: ValidationKind::List,
        ..Default::default()
    };
    list.set_operands("=$A$1:$A$2", "", Locale::en()).unwrap();
    wb.set_validation(0, &[rect("G1")], (0, 6), list);
    wb.commit("setup");

    assert_eq!(wb.move_range(0, rect("A1:A2"), 0, 4, 2), rect("C5:C6"));
    wb.commit("move");
    assert_eq!(
        [carried(&wb, 0, "C5"), carried(&wb, 0, "C6")],
        [has("first"), has("second")]
    );
    assert_eq!(
        [carried(&wb, 0, "A1"), carried(&wb, 0, "A2")],
        [NOTHING, NOTHING]
    );
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("C5:C6")]);
    assert_eq!(wb.validation_at(0, 4, 2).unwrap().ranges, [rect("C5:C6")]);
    // The formula, the name and the list follow the cells.
    assert_eq!(
        (shown(&wb, 0, "E1"), value(&wb, 0, "E1")),
        ("=SUM(C5:C6)".to_string(), num(16.0))
    );
    let name = wb.name_in(NameScope::Workbook, "Pair").unwrap();
    assert_eq!(name.refers_to(Locale::en(), 0, 0), "=Sheet1!$C$5:$C$6");
    assert_eq!(
        wb.validation_at(0, 0, 6)
            .unwrap()
            .operand1_text(0, 6, Locale::en()),
        "=$C$5:$C$6"
    );
    assert!(wb.undo());
    assert_eq!(
        [carried(&wb, 0, "A1"), carried(&wb, 0, "C5")],
        [has("first"), NOTHING]
    );

    // A move that lands partly on itself.
    wb.move_range(0, rect("A1:A2"), 0, 1, 0);
    wb.commit("move");
    assert_eq!(
        [
            carried(&wb, 0, "A1"),
            carried(&wb, 0, "A2"),
            carried(&wb, 0, "A3")
        ],
        [NOTHING, has("first"), has("second")]
    );
    assert_eq!(wb.validation_at(0, 1, 0).unwrap().ranges, [rect("A2:A3")]);
    // To another sheet.
    let other = wb.add_sheet(1);
    wb.move_range(0, rect("A2:A3"), other, 0, 0);
    wb.commit("move");
    assert_eq!(
        [carried(&wb, other, "A1"), carried(&wb, other, "A2")],
        [has("first"), has("second")]
    );
    assert!(wb.cond_formats(0).is_empty() && wb.sheets[0].comments.is_empty());
    assert!(wb.validation_at(0, 1, 0).is_none());
}

#[test]
fn fills_copy_them_too() {
    // Ctrl+D.
    let mut wb = decorated();
    wb.fill_down(0, rect("A2:A6"));
    wb.commit("fill");
    for cell in ["A3", "A6"] {
        assert_eq!(
            (carried(&wb, 0, cell), value(&wb, 0, cell)),
            (has("second"), num(9.0)),
            "{cell}"
        );
    }
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("A1:A6")]);
    assert_eq!(wb.sheets[0].validations[0].ranges, [rect("A1:A6")]);
    // Ctrl+R.
    wb.fill_right(0, rect("A1:C2"));
    wb.commit("fill");
    assert_eq!(
        [carried(&wb, 0, "C1"), carried(&wb, 0, "B2")],
        [has("first"), has("second")]
    );

    // The fill handle: the two cells again and again, down...
    let mut wb = decorated();
    wb.autofill(0, rect("A1:A2"), rect("A1:A7"));
    wb.commit("fill");
    assert_eq!(value(&wb, 0, "A7"), num(19.0));
    for (cell, note) in [
        ("A3", "first"),
        ("A4", "second"),
        ("A6", "second"),
        ("A7", "first"),
    ] {
        assert_eq!(carried(&wb, 0, cell), has(note), "{cell}");
    }
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("A1:A7")]);
    // ...up, where the last cells of the pair come first...
    let mut wb = decorated();
    wb.move_range(0, rect("A1:A2"), 0, 5, 0);
    wb.autofill(0, rect("A6:A7"), rect("A3:A7"));
    wb.commit("fill");
    for (cell, note) in [("A3", "second"), ("A4", "first"), ("A5", "second")] {
        assert_eq!(carried(&wb, 0, cell), has(note), "{cell}");
    }
    assert_eq!(carried(&wb, 0, "A2"), NOTHING);
    // ...and sideways.
    let mut wb = decorated();
    wb.autofill(0, rect("A1:A2"), rect("A1:C2"));
    wb.commit("fill");
    assert_eq!(
        [carried(&wb, 0, "B1"), carried(&wb, 0, "C2")],
        [has("first"), has("second")]
    );
    assert_eq!(wb.sheets[0].validations[0].ranges, [rect("A1:C2")]);
    assert!(wb.undo());
    assert_eq!(carried(&wb, 0, "B1"), NOTHING);
}

#[test]
fn clearing_takes_what_excel_takes() {
    let state = |wb: &Workbook| [carried(wb, 0, "A1"), carried(wb, 0, "A2")];
    let mut wb = decorated();
    // Contents: the values go, everything else stays.
    wb.clear(0, rect("A1:A2"), ClearMode::Contents);
    wb.commit("clear");
    assert!(wb.sheets[0].is_blank(0, 0));
    assert_eq!(wb.cond_formats(0).len(), 1);
    assert_eq!(
        (carried(&wb, 0, "A1").1, carried(&wb, 0, "A2").2),
        (true, Some("second".to_string()))
    );
    // Formats: conditional formats go with them.
    let mut wb = decorated();
    wb.clear(0, rect("A1"), ClearMode::Formats);
    wb.commit("clear");
    assert_eq!(
        state(&wb),
        [(false, true, Some("first".to_string())), has("second")]
    );
    assert_eq!(wb.cond_formats(0)[0].ranges, [rect("A2")]);
    // Comments and notes.
    wb.clear(0, rect("A1:A2"), ClearMode::Comments);
    wb.commit("clear");
    assert_eq!(state(&wb), [(false, true, None), (true, true, None)]);
    assert_eq!(value(&wb, 0, "A2"), num(9.0));
    assert!(wb.undo());
    assert_eq!(carried(&wb, 0, "A2"), has("second"));
    // All: validation too.
    wb.clear(0, rect("A1:A2"), ClearMode::All);
    wb.commit("clear");
    assert_eq!(state(&wb), [NOTHING, NOTHING]);
    assert!(wb.cond_formats(0).is_empty() && wb.sheets[0].validations.is_empty());
    assert!(wb.sheets[0].cells.is_empty());
    assert!(wb.undo());
    assert_eq!(carried(&wb, 0, "A2"), has("second"));
}

#[test]
fn sorting_and_removing_duplicates_move_notes_with_their_rows() {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [
        ("A1", "c"),
        ("A2", "a"),
        ("A3", "c"),
        ("A4", "b"),
        ("B1", "1"),
        ("B2", "2"),
        ("B3", "3"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    wb.set_note(0, 0, 0, "Ada", "on c");
    wb.set_note(0, 1, 1, "Ada", "on 2");
    wb.add_comment(0, 3, 0, &Person::new("Ada"), 45000.0, "on b");
    wb.set_note(0, 1, 3, "Ada", "beside");
    // A box placed by hand keeps its place next to its cell.
    wb.set_note_frame(0, 0, 0, Some(Frame::new(200.0, 0.0, 80.0, 40.0)));
    let placed = wb
        .comment_at(0, 0, 0)
        .unwrap()
        .as_note()
        .unwrap()
        .frame
        .unwrap();
    wb.commit("setup");
    let note = |wb: &Workbook, cell: &str| carried(wb, 0, cell).2;

    let by_first = SortSpec {
        levels: vec![SortLevel::values(0, false)],
        ..Default::default()
    };
    wb.sort_range(0, rect("A1:B4"), &by_first).unwrap();
    wb.commit("sort");
    assert_eq!(value(&wb, 0, "A1"), Value::text("a"));
    assert_eq!(note(&wb, "B1").as_deref(), Some("on 2"));
    assert_eq!(note(&wb, "A2").as_deref(), Some("on b"));
    assert_eq!(note(&wb, "A3").as_deref(), Some("on c"));
    assert_eq!(
        [note(&wb, "A1"), note(&wb, "A4"), note(&wb, "B2")],
        [None, None, None]
    );
    // What lies beside the sorted range stays.
    assert_eq!(note(&wb, "D2").as_deref(), Some("beside"));
    let moved = wb
        .comment_at(0, 2, 0)
        .unwrap()
        .as_note()
        .unwrap()
        .frame
        .unwrap();
    assert_eq!(
        (moved.anchor, moved.offset),
        ((placed.anchor.0 + 2, placed.anchor.1), placed.offset)
    );
    assert!(wb.undo());
    assert_eq!(note(&wb, "A1").as_deref(), Some("on c"));

    // Left to right, columns move with theirs.
    let columns = SortSpec {
        levels: vec![SortLevel::values(1, false)],
        left_to_right: true,
        ..Default::default()
    };
    wb.sort_range(0, rect("A1:B2"), &columns).unwrap();
    wb.commit("sort");
    assert_eq!(
        [note(&wb, "B1").as_deref(), note(&wb, "A2").as_deref()],
        [Some("on c"), Some("on 2")]
    );
    assert!(wb.undo());

    // Remove Duplicates on the first column: row 3 repeats row 1 and goes
    // with what it had; the row under it moves up with its comment.
    wb.set_note(0, 2, 1, "Ada", "on 3");
    wb.commit("note");
    let out = wb.remove_duplicates(0, rect("A1:B4"), &[0], false).unwrap();
    wb.commit("duplicates");
    assert_eq!((out.removed, out.remaining), (1, 3));
    assert_eq!(note(&wb, "A3").as_deref(), Some("on b"));
    assert_eq!([note(&wb, "B3"), note(&wb, "A4")], [None, None]);
    assert_eq!(note(&wb, "A1").as_deref(), Some("on c"));
    assert_eq!(wb.sheets[0].comments.len(), 4);
}

#[test]
fn renaming_a_name_reaches_rules() {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [("F1", "red"), ("F2", "blue"), ("A1", "5")] {
        enter(&mut wb, 0, cell, text);
    }
    let refers_to = wb.parse_name_formula("=Sheet1!$F$1:$F$2", 0, 0, 0).unwrap();
    wb.add_name(NameScope::Workbook, "Colours", refers_to, "")
        .unwrap();
    let limit = wb.parse_name_formula("=3", 0, 0, 0).unwrap();
    wb.add_name(NameScope::Workbook, "Limit", limit, "")
        .unwrap();
    let mut list = Validation {
        kind: ValidationKind::List,
        ..Default::default()
    };
    list.set_operands("=Colours", "", Locale::en()).unwrap();
    wb.set_validation(0, &[rect("B1:B5")], (0, 1), list);
    let over = CondFormat::new(
        vec![rect("A1")],
        Rule::greater_than(Expr::Name("Limit".into())),
    );
    wb.add_cond_format(0, over.with_format(HighlightFormat::LightRedFill.patch()));
    wb.commit("setup");
    assert!(filled(&wb, 0, "A1"));

    wb.rename_name(NameScope::Workbook, "Colours", "Palette")
        .unwrap();
    wb.rename_name(NameScope::Workbook, "Limit", "Ceiling")
        .unwrap();
    wb.commit("rename");
    // The list and the rule go on working under the new names.
    assert_eq!(
        wb.validation_at(0, 2, 1)
            .unwrap()
            .operand1_text(2, 1, Locale::en()),
        "=Palette"
    );
    let entries: Vec<String> = wb
        .validation_list(0, 2, 1)
        .unwrap()
        .into_iter()
        .map(|item| item.text)
        .collect();
    assert_eq!(entries, ["red", "blue"]);
    assert_eq!(
        wb.cond_formats(0)[0].rule,
        Rule::greater_than(Expr::Name("Ceiling".into()))
    );
    assert!(filled(&wb, 0, "A1"));
    assert!(wb.undo());
    assert_eq!(
        wb.validation_at(0, 2, 1)
            .unwrap()
            .operand1_text(2, 1, Locale::en()),
        "=Colours"
    );
}
