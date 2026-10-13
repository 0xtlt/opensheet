//! The classes of the object model, one script per group of members,
//! and the proof that every member `api()` lists answers a call.

mod common;
use common::*;
use opensheet_engine::Rect;
use opensheet_engine::chart::{ChartKind, ChartTitle, LegendPos};
use opensheet_engine::locale::Locale;
use opensheet_engine::style::BorderStyle;
use opensheet_engine::workbook::Workbook;
use opensheet_script::{Autofit, Context, api, enums, members_called};

/// A small list with a header row.
fn stock() -> Workbook {
    book(&[
        ("A1", "Name"),
        ("B1", "Qty"),
        ("C1", "Price"),
        ("A2", "Pen"),
        ("B2", "10"),
        ("C2", "1.5"),
        ("A3", "Ink"),
        ("B3", "2"),
        ("C3", "12.25"),
        ("A4", "Pad"),
        ("B4", "5"),
        ("C4", "3"),
    ])
}

fn worksheets() {
    let mut wb = book(&[("A1", "1")]);
    let context = Context {
        workbook_name: "Budget.xlsx".into(),
        ..Context::default()
    };
    let out = ok_with(&mut wb, include_str!("model/worksheets.ts"), &context);
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Sheet1 (3)", "Sheet1", "Sheet1 (5)", "Data", "Sheet1 (2)", "Sheet1 (4)"]);
    assert_eq!(out.activated, Some(3));
    assert!(!wb.sheets[3].show_gridlines && !wb.sheets[3].show_headers);
    assert_eq!(wb.sheets[3].tab_color.map(|c| c.value()), Some(0x00FF00));
    use opensheet_engine::workbook::SheetVisibility::{Hidden, Visible};
    assert_eq!(wb.sheets.iter().map(|s| s.visibility).collect::<Vec<_>>(), [Visible, Visible, Visible, Visible, Visible, Hidden]);
    // A script that fails after hiding sheets leaves them as they were;
    // the last visible sheet stays; a protected structure refuses.
    let all = "for (const s of workbook.getWorksheets()) { s.setVisibility(ExcelScript.SheetVisibility.hidden); }";
    assert!(fails(&mut wb, &script(all)).message.contains("at least one visible worksheet"));
    wb.protect_workbook(true, false, "");
    let show = "workbook.getLastWorksheet().setVisibility(ExcelScript.SheetVisibility.visible);";
    assert!(fails(&mut wb, &script(show)).message.contains("protected"));
    ok_body(&mut wb, "workbook.getLastWorksheet().setVisibility(ExcelScript.SheetVisibility.hidden);");
}

fn names() {
    let mut wb = book(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "4"), ("B2", "5"), ("B3", "6")]);
    ok(&mut wb, include_str!("model/names.ts"));
    assert_eq!(wb.names.keys().collect::<Vec<_>>(), ["DATA", "LABEL", "TOTAL"]);
    assert_eq!(wb.sheets[0].names.keys().collect::<Vec<_>>(), ["LOCAL", "TWICE"]);
    assert_eq!(number(&wb, "E1"), 101.0);
}

fn values() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/values.ts"));
    assert_eq!(number(&wb, "D3"), 24.5);
    assert_eq!(text(&wb, "C3"), "12.3");
}

fn navigation() {
    let mut wb = stock();
    let context = Context {
        selection: Rect::new(1, 1, 2, 2),
        active_cell: (2, 2),
        ..Context::default()
    };
    let out = ok_with(&mut wb, include_str!("model/navigation.ts"), &context);
    assert_eq!(out.selected, Some((0, Rect::new(1, 0, 3, 0))));
    assert_eq!(out.activated, Some(0));
}

fn editing() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/editing.ts"));
    assert_eq!(wb.sheets[0].merges, [Rect::new(0, 13, 1, 14), Rect::new(3, 13, 3, 14), Rect::new(4, 13, 4, 14)]);
    assert_eq!(wb.sheets.len(), 2);
}

fn data() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/data.ts"));
    assert!(wb.sheets[0].hidden_rows.contains(&2) && !wb.sheets[0].hidden_rows.contains(&3));
    assert!(wb.sheets[0].hidden_cols.contains(&1) && !wb.sheets[0].hidden_cols.contains(&2));
    assert_eq!(wb.sheets[0].hyperlinks.len(), 0);
}

fn format() {
    let mut wb = stock();
    let out = ok(&mut wb, include_str!("model/format.ts"));
    let header = wb.style_at(0, 0, 1).clone();
    assert!(header.font.bold && header.font.italic && !header.font.strike && header.wrap);
    assert_eq!((header.font.name.as_ref(), header.font.size), ("Arial", 28));
    assert_eq!(header.fill, None);
    assert_eq!(wb.style_at(0, 0, 2).fill.map(|c| c.value()), Some(0x4472C4));
    // The lines inside are the bottom of a cell and the top of the next.
    let middle = wb.style_at(0, 2, 1).borders;
    assert_eq!((middle.top.style, middle.bottom.style), (BorderStyle::Dotted, BorderStyle::Dotted));
    assert_eq!(middle.left.color.map(|c| c.value()), Some(0x808080));
    assert_eq!(wb.style_at(0, 1, 0).borders.top.style, BorderStyle::Hair);
    assert_eq!(wb.style_at(0, 1, 0).borders.left.style, BorderStyle::Thin);
    assert_eq!(wb.style_at(0, 3, 2).borders.right.style, BorderStyle::Medium);
    assert_eq!(wb.style_at(0, 3, 0).borders.bottom.style, BorderStyle::None);
    assert!(!wb.style_at(0, 2, 1).locked && wb.style_at(0, 2, 1).hidden);
    assert_eq!(
        out.autofit,
        [
            Autofit { sheet: 0, columns: true, first: 0, last: 4 },
            Autofit { sheet: 0, columns: false, first: 0, last: 3 },
        ]
    );
}

fn filter() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/filter.ts"));
    assert_eq!((wb.sheets[0].frozen_rows, wb.sheets[0].frozen_cols), (1, 1));
    assert!(wb.sheets[0].auto_filter.is_some() && wb.sheets[0].filtered_rows.is_empty());
}

fn tables() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/tables.ts"));
    assert_eq!(wb.sheets[0].tables.len(), 1);
    assert_eq!(wb.sheets[0].tables[0].columns.len(), 2);
}

fn charts() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/charts.ts"));
    let chart = &wb.sheets[0].charts[0];
    assert_eq!((chart.name.as_str(), chart.kind, chart.series.len()), ("Stock", ChartKind::StackedBar, 2));
    assert_eq!((chart.title.clone(), chart.legend), (ChartTitle::Text("Stock".into()), LegendPos::TopRight));
    assert_eq!((chart.anchor, chart.size), ((1, 2), (400.0, 800.0 / 3.0)));
    // Charted from another sheet, by rows: three series that name their sheet.
    let other = &wb.sheets[1].charts[0];
    assert_eq!((other.kind, other.series.len()), (ChartKind::LineMarkers, 3));
    assert_eq!(wb.chart_data(1, other).series[0].values, [Some(10.0), Some(1.5)]);
}

fn condfmt() {
    use opensheet_engine::condfmt::{CellOp, Rule};
    let mut wb = stock();
    ok(&mut wb, include_str!("model/condfmt.ts"));
    let rules = &wb.sheets[0].cond_formats;
    assert_eq!(rules.len(), 4);
    assert!(matches!(rules[0].rule, Rule::CellIs { op: CellOp::Between, .. }) && rules[0].stop_if_true);
    assert_eq!(rules[0].ranges, [Rect::new(2, 1, 3, 1)]);
    assert_eq!(rules[0].format.fill, Some(Some(opensheet_engine::Color(0xFFFF00))));
    // What the rules show is what the same rules show made by hand: B3
    // (2) is between 2 and 6, B4 (5) too.
    assert_eq!(wb.display(0, 2, 1).style.fill.map(|c| c.value()), Some(0xFFFF00));
    // On a protected sheet a rule needs the permission to format cells.
    wb.protect_sheet(0, Default::default(), "");
    let add = "sheet.getRange(\"A1\").addConditionalFormat(ExcelScript.ConditionalFormatType.custom);";
    assert!(fails(&mut wb, &script(add)).message.contains("protected"));
}

fn validation() {
    use opensheet_engine::validation::{AlertStyle, ValidationKind};
    let mut wb = stock();
    ok(&mut wb, include_str!("model/validation.ts"));
    let rules = &wb.sheets[0].validations;
    assert_eq!(rules.len(), 3);
    let qty = wb.validation_at(0, 2, 1).expect("a rule on B3");
    assert_eq!((qty.kind, qty.error_style, qty.allow_blank), (ValidationKind::Whole, AlertStyle::Warning, false));
    assert_eq!(qty.ranges, [Rect::new(1, 1, 3, 1)]);
    // The rule is the one the dialog makes: an entry is checked by it.
    assert!(matches!(wb.check_entry(0, 1, 1, "50"), opensheet_engine::validation::EntryCheck::Invalid(_)));
    assert!(wb.validation_at(0, 2, 2).is_none());
    wb.protect_sheet(0, Default::default(), "");
    assert!(fails(&mut wb, &script("sheet.getRange(\"B2\").getDataValidation().clear();")).message.contains("protected"));
}

#[test]
fn data_validation_rules_alerts_and_invalid_cells() {
    validation();
}

fn comments() {
    use opensheet_engine::comment::Comment;
    let mut wb = stock();
    let context = Context {
        user_name: "Ada Script".into(),
        ..Context::default()
    };
    ok_with(&mut wb, include_str!("model/comments.ts"), &context);
    let held = &wb.sheets[0].comments;
    assert_eq!(held.keys().copied().collect::<Vec<_>>(), [(1, 1), (3, 0)]);
    let Some(Comment::Thread(thread)) = held.get(&(1, 1)) else {
        panic!("a thread on B2");
    };
    assert_eq!((thread.first.author.name.as_str(), thread.replies.len(), thread.resolved), ("Ada Script", 1, false));
    assert_eq!(thread.replies[0].mentions.len(), 2);
    assert_eq!(thread.replies[0].mentions[1].person.user_id, "ann@example.org");
    let Some(Comment::Note(note)) = held.get(&(3, 0)) else {
        panic!("a note on A4");
    };
    assert!(note.visible && note.frame.is_some_and(|frame| frame.size == (200.0, 80.0)));
    // A protected sheet lets nobody edit comments unless it allows objects.
    wb.protect_sheet(0, Default::default(), "");
    assert!(fails(&mut wb, &script("sheet.addNote(\"F1\", \"x\");")).message.contains("protected"));
}

#[test]
fn comments_replies_mentions_and_notes() {
    comments();
}

/// A list of sales: region, product, quantity, amount.
fn sales() -> Workbook {
    let rows = [
        ["Region", "Product", "Qty", "Amount"],
        ["North", "Pen", "10", "15"],
        ["North", "Ink", "2", "24.5"],
        ["South", "Pen", "5", "7.5"],
        ["South", "Pad", "5", "15"],
        ["East", "Ink", "1", "12.25"],
        ["East", "Pen", "3", "4.5"],
    ];
    let cells: Vec<(String, &str)> = rows
        .iter()
        .enumerate()
        .flat_map(|(r, row)| row.iter().enumerate().map(move |(c, text)| (format!("{}{}", (b'A' + c as u8) as char, r + 1), *text)))
        .collect();
    book(&cells.iter().map(|(name, text)| (name.as_str(), *text)).collect::<Vec<_>>())
}

fn pivots() {
    use opensheet_engine::pivot::{Aggregation, FieldLayout};
    let mut wb = sales();
    ok(&mut wb, include_str!("model/pivots.ts"));
    let (sheet, pivot) = wb.pivots().next().expect("a PivotTable");
    assert_eq!((sheet, pivot.name.as_str(), pivot.anchor), (0, "Report", (4, 5)));
    assert_eq!((pivot.row_fields.len(), pivot.col_fields.len(), pivot.data_fields.len()), (2, 0, 1));
    assert_eq!((pivot.data_fields[0].function, pivot.data_fields[0].name.as_str()), (Aggregation::Sum, "Revenue"));
    assert!(pivot.fields.iter().all(|f| f.layout == FieldLayout::Outline));
    assert_eq!(wb.pivots().count(), 1);
    // A protected sheet refuses unless it allows using PivotTables.
    wb.protect_sheet(0, Default::default(), "");
    assert!(fails(&mut wb, &script("workbook.getPivotTable(\"Report\")!.refresh();")).message.contains("protected"));
}

#[test]
fn pivot_tables_their_fields_values_filters_and_layout() {
    pivots();
}

fn shapes() {
    use opensheet_engine::drawing::{Dash, ShapePreset};
    let mut wb = stock();
    ok(&mut wb, include_str!("model/shapes.ts"));
    let drawings = &wb.sheets[0].drawings;
    assert_eq!(drawings.len(), 6);
    let banner = drawings.iter().find(|d| d.name == "Banner").expect("the banner");
    let shape = banner.as_shape().expect("a shape");
    assert_eq!((shape.preset, shape.text.as_str(), shape.fill.map(|c| c.value())), (ShapePreset::RoundRect, "Q3", Some(0x336699)));
    assert_eq!(shape.outline.map(|o| (o.color.value(), o.width, o.dash)), Some((0x000000, 1.0, Dash::Solid)));
    // 100 by 45 points, 150 by 60: pixels at 100 % in the engine.
    let frame = wb.drawing_frame(0, banner.id).expect("a frame");
    assert!((frame.x - 400.0 / 3.0).abs() < 0.01 && (frame.y - 60.0).abs() < 0.01 && (frame.width - 200.0).abs() < 0.01 && (frame.height - 80.0).abs() < 0.01);
    assert_eq!(drawings.iter().filter(|d| d.as_picture().is_some()).count(), 2);
    // On a protected sheet objects are edited only when it allows it.
    wb.protect_sheet(0, Default::default(), "");
    assert!(fails(&mut wb, &script("sheet.addTextBox(\"x\");")).message.contains("protected"));
}

#[test]
fn shapes_lines_text_boxes_and_pictures() {
    shapes();
}

fn areas() {
    let mut wb = stock();
    ok(&mut wb, include_str!("model/areas.ts"));
    assert_eq!(wb.style_at(0, 2, 1).fill.map(|c| c.value()), Some(0xFFFF00));
    assert!(wb.style_at(0, 2, 1).font.bold && wb.style_at(0, 0, 3).fill.is_none());
    assert_eq!(wb.sheets[0].merges.len(), 2);
    // A script that only looks for cells leaves no step to undo.
    let mut wb = stock();
    let looking = "sheet.getRange(\"A1:C4\").getSpecialCells(ExcelScript.SpecialCellType.constants); sheet.findAll(\"Pen\", {}); sheet.getRanges(\"A1,B2\").getAreaCount();";
    run_with(&mut wb, &script(looking), &Context::default());
    assert!(!wb.commit("nothing") && !wb.can_undo());
}

#[test]
fn several_ranges_special_cells_find_all_and_visible_views() {
    areas();
}

#[test]
fn conditional_formats_of_every_kind() {
    condfmt();
}

#[test]
fn charts_their_titles_and_their_places() {
    charts();
}

#[test]
fn worksheets_of_a_workbook() {
    worksheets();
}

#[test]
fn names_and_calculation() {
    names();
}

#[test]
fn values_formulas_and_number_formats() {
    values();
}

#[test]
fn from_a_range_to_another() {
    navigation();
}

#[test]
fn cells_that_move_copy_and_clear() {
    editing();
}

#[test]
fn sort_search_fill_link_and_hide() {
    data();
}

#[test]
fn fonts_fills_borders_alignment_and_sizes() {
    format();
}

#[test]
fn frozen_panes_and_the_filter_of_a_sheet() {
    filter();
}

#[test]
fn tables_their_columns_and_their_filters() {
    tables();
}

#[test]
fn english_and_the_language_of_the_workbook() {
    let mut wb = book_in(Locale::fr(), &[("A1", "1,5"), ("A2", "2,5")]);
    ok(&mut wb, include_str!("model/locale.ts"));
    assert_eq!(wb.locale.id, "fr");
    assert_eq!(text(&wb, "B2"), "2");
}

/// Every member the table lists has answered a call of the scripts above:
/// the list a user is shown and what runs cannot differ.
#[test]
fn every_listed_member_runs() {
    for scenario in [worksheets, names, values, navigation, editing, data, format, filter, tables, charts, condfmt, validation, comments, pivots, shapes, areas] {
        scenario();
    }
    let called = members_called();
    let mut missing = Vec::new();
    let mut count = 0;
    for class in api() {
        assert!(!class.description.is_empty(), "{} has no description", class.name);
        for member in class.members {
            count += 1;
            assert!(member.signature.starts_with(&format!("{}(", member.name)), "{}", member.signature);
            assert!(!member.description.is_empty(), "{}.{} has no description", class.name, member.name);
            if !called.contains(&(class.name, member.name)) {
                missing.push(format!("{}.{}", class.name, member.name));
            }
        }
    }
    assert!(missing.is_empty(), "no test calls {missing:?}");
    assert!(count > 250, "{count} members");
    // The first class is the one `main` receives; names are not repeated.
    assert_eq!(api()[0].name, "Workbook");
    let mut names: Vec<&str> = api().iter().map(|c| c.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), api().len());
    for class in api() {
        let mut members: Vec<&str> = class.members.iter().map(|m| m.name).collect();
        members.sort_unstable();
        members.dedup();
        assert_eq!(members.len(), class.members.len(), "{}", class.name);
    }
}

/// The enums are those a script reads, with Excel's values.
#[test]
fn enums_are_what_scripts_read() {
    let mut source = String::from("function main(workbook: ExcelScript.Workbook) {\n");
    for e in enums() {
        for (member, value) in e.members {
            source.push_str(&format!("  eq(ExcelScript.{}.{member}, {value:?});\n", e.name));
        }
    }
    source.push_str("}\n");
    let mut wb = stock();
    ok(&mut wb, &source);
    let of = |name: &str| enums().iter().find(|e| e.name == name).map(|e| e.members).expect(name);
    assert!(of("ClearApplyTo").contains(&("contents", "Contents")));
    assert!(of("BorderIndex").contains(&("edgeBottom", "EdgeBottom")));
    assert!(of("HorizontalAlignment").contains(&("centerAcrossSelection", "CenterAcrossSelection")));
}
