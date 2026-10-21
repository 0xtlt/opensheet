//! Tests of the logic behind the filter and sort interface: what the
//! commands decide to act on, what the drop-down lists and applies, what
//! the dialogs hand to the engine. They read the interface in English.

use super::custom::{Fields, comparisons, suggestions, top10_criteria};
use super::dropdown::{Act, Entry, EntryKind, Row, ValueList, entries};
use super::duplicates::{self, Plan};
use super::sort::{self, Basis, Level, Problem, Setup};
use super::*;
use crate::widgets::Tri;
use opensheet_engine::filter::{CustomFilter, DateGroup, FilterDropdown, Top10};
use opensheet_engine::ops::DuplicatesRemoved;
use opensheet_engine::style::Color;
use std::sync::Arc;

/// A list in A1:D7 (fruit, quantity, date, note) with a blank fruit, and a
/// lone label in F1.
fn list() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let rows = [
        ["Fruit", "Quantity", "Picked", "Note"],
        ["pear", "3", "2026-01-05", "ripe"],
        ["apple", "10", "2026-02-11", "green"],
        ["fig", "7", "2025-12-24", "ripe"],
        ["apple", "1", "2026-02-12", ""],
        ["", "4", "2026-03-01", "late"],
        ["cherry", "7", "2026-03-02", "ripe"],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    let short: Arc<str> = Arc::from(wb.locale.short_date_format);
    wb.update_style(0, Rect::new(1, 2, 6, 2), &move |s| {
        s.num_fmt = short.clone()
    });
    wb.update_style(0, Rect::new(0, 0, 0, 3), &|s| s.font.bold = true);
    wb.set_cell_text(0, 0, 5, "alone").unwrap();
    wb.commit("fill");
    wb
}

/// The same list with its filter buttons on.
fn filtered_list() -> Workbook {
    let mut wb = list();
    wb.toggle_auto_filter(0, Rect::cell(2, 1)).unwrap();
    wb.commit("filter");
    wb
}

fn dropdown(wb: &Workbook, col_id: u32) -> FilterDropdown {
    wb.filter_dropdown(0, FilterScope::Sheet, col_id).unwrap()
}

fn labels(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry.kind {
            EntryKind::Separator => "-".to_string(),
            _ => entry.label.to_string(),
        })
        .collect()
}

fn find<'a>(entries: &'a [Entry], label: &str) -> &'a Entry {
    entries
        .iter()
        .find(|entry| entry.label.as_ref() == label)
        .unwrap_or_else(|| panic!("no entry {label:?} among {:?}", labels(entries)))
}

fn children(entry: &Entry) -> &[Entry] {
    match &entry.kind {
        EntryKind::Sub(children) => children,
        other => panic!("{:?} is not a submenu: {other:?}", entry.label),
    }
}

// ------------------------------------------------------------------ wording

#[test]
fn records_found_uses_excel_wording() {
    assert_eq!(records_found(3, 8), "3 of 8 records found");
}

#[test]
fn sort_entries_follow_what_the_column_holds() {
    assert_eq!(
        sort_labels(ColumnKind::Text),
        ("Sort A to Z", "Sort Z to A")
    );
    assert_eq!(
        sort_labels(ColumnKind::Number),
        ("Sort Smallest to Largest", "Sort Largest to Smallest")
    );
    assert_eq!(
        sort_labels(ColumnKind::Date),
        ("Sort Oldest to Newest", "Sort Newest to Oldest")
    );
}

#[test]
fn a_column_without_header_is_named_by_its_letter() {
    assert_eq!(column_title("Region", 1), "Region");
    assert_eq!(column_title("  ", 27), "(Column AB)");
    assert_eq!(clear_filter_label("Region"), "Clear Filter From \"Region\"");
}

#[test]
fn column_kinds_are_told_from_the_cells() {
    let wb = list();
    assert_eq!(column_kind(&wb, 0, 0, 1..=6), ColumnKind::Text);
    assert_eq!(column_kind(&wb, 0, 1, 1..=6), ColumnKind::Number);
    assert_eq!(column_kind(&wb, 0, 2, 1..=6), ColumnKind::Date);
    // An empty column counts as text, as the engine has it.
    assert_eq!(column_kind(&wb, 0, 9, 1..=6), ColumnKind::Text);
}

#[test]
fn criteria_are_summarised_for_the_tip_of_a_button() {
    let en = Locale::en();
    assert_eq!(criteria_summary(None, false, en), "(Showing All)");
    let values = Criteria::Values(ValueFilter {
        values: vec!["apple".into(), "fig".into()],
        blanks: true,
        dates: Vec::new(),
    });
    assert_eq!(
        criteria_summary(Some(&values), false, en),
        "Equals \"apple\" or \"fig\" or (Blanks)"
    );
    let many = Criteria::Values(ValueFilter {
        values: (0..5).map(|i| i.to_string()).collect(),
        blanks: false,
        dates: vec![DateGroup::year(2026)],
    });
    assert_eq!(criteria_summary(Some(&many), false, en), "6 items selected");
    let between = Criteria::Custom(CustomFilter {
        first: Condition::new(Comparison::GreaterOrEqual, "3", en),
        second: Some(Condition::new(Comparison::Less, "7.5", en)),
        and: true,
    });
    assert_eq!(
        criteria_summary(Some(&between), false, en),
        "Greater than or equal to \"3\" and Less than \"7.5\""
    );
    let begins = Criteria::Custom(CustomFilter::one(Condition::new(
        Comparison::BeginsWith,
        "ap",
        en,
    )));
    assert_eq!(
        criteria_summary(Some(&begins), false, en),
        "Begins with \"ap\""
    );
    let after = Criteria::Custom(CustomFilter::one(Condition::new(
        Comparison::Greater,
        "1/31/2026",
        en,
    )));
    assert_eq!(
        criteria_summary(Some(&after), true, en),
        "After \"1/31/2026\""
    );
    let top = Criteria::Top10(Top10::new(false, true, 25.0));
    assert_eq!(criteria_summary(Some(&top), false, en), "Bottom 25 percent");
    assert_eq!(
        criteria_summary(Some(&Criteria::Dynamic(DynamicFilter::Month(3))), false, en),
        "March"
    );
    assert_eq!(
        criteria_summary(
            Some(&Criteria::Dynamic(DynamicFilter::Quarter(2))),
            false,
            en
        ),
        "Quarter 2"
    );
    let no_fill = Criteria::Color(ColorFilter {
        color: None,
        font: false,
    });
    assert_eq!(
        criteria_summary(Some(&no_fill), false, en),
        "Equals a cell color (No Fill)"
    );
}

#[test]
fn month_names_take_a_capital_in_every_language() {
    assert_eq!(month_name(Locale::by_id("fr"), 2), "Février");
    assert_eq!(month_name(Locale::by_id("fr"), 8), "Août");
    assert_eq!(month_name(Locale::en(), 12), "December");
}

// ------------------------------------------------------------- the buttons

#[test]
fn the_button_sits_in_the_corner_of_its_cell() {
    assert_eq!(
        button_box((100.0, 20.0, 180.0, 40.0), 1.0),
        Some((162.0, 22.0, 178.0, 38.0))
    );
    // It grows with the zoom...
    assert_eq!(
        button_box((0.0, 0.0, 200.0, 50.0), 2.0),
        Some((166.0, 16.0, 198.0, 48.0))
    );
    // ...shrinks in a low row or a narrow column, and leaves a hidden one.
    assert_eq!(
        button_box((0.0, 0.0, 80.0, 12.0), 1.0),
        Some((69.0, 1.0, 78.0, 10.0))
    );
    assert_eq!(
        button_box((0.0, 0.0, 10.0, 20.0), 1.0),
        Some((1.0, 11.0, 8.0, 18.0))
    );
    assert_eq!(button_box((0.0, 0.0, 0.0, 20.0), 1.0), None);
    assert_eq!(button_box((0.0, 0.0, 80.0, 0.0), 1.0), None);
}

#[test]
fn only_the_buttons_in_view_are_drawn() {
    let wb = filtered_list();
    assert_eq!(buttons_in(&wb, 0, Rect::new(0, 0, 30, 30)).len(), 4);
    let some: Vec<u32> = buttons_in(&wb, 0, Rect::new(0, 2, 30, 30))
        .iter()
        .map(|b| b.col)
        .collect();
    assert_eq!(some, [2, 3]);
    // The header row is scrolled out of view.
    assert!(buttons_in(&wb, 0, Rect::new(1, 0, 30, 30)).is_empty());
}

#[test]
fn the_tip_of_a_button_names_its_column_and_criterion() {
    let mut wb = filtered_list();
    let button = wb.filter_button_at(0, 0, 1).unwrap();
    assert_eq!(button_tip(&wb, 0, &button), "Quantity:\n(Showing All)");
    let above = Criteria::Custom(CustomFilter::one(Condition::new(
        Comparison::Greater,
        "5",
        wb.locale,
    )));
    wb.set_filter_criteria(0, FilterScope::Sheet, 1, Some(above));
    assert_eq!(button_tip(&wb, 0, &button), "Quantity:\nGreater than \"5\"");
}

// ---------------------------------------------------------- what is sorted

#[test]
fn a_single_cell_stands_for_the_block_around_it() {
    let wb = list();
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(3, 1), (3, 1)),
        SortPlan::Sort(SortTarget::Range {
            rect: Rect::new(0, 0, 6, 3),
            has_header: true,
        })
    );
    // A lone cell has nothing to sort with.
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(0, 5), (0, 5)),
        SortPlan::Nothing
    );
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(20, 20), (20, 20)),
        SortPlan::Nothing
    );
}

#[test]
fn one_column_of_a_block_asks_whether_to_expand() {
    let wb = list();
    let column = Rect::new(1, 1, 6, 1);
    assert_eq!(
        sort_plan(&wb, 0, column, (1, 1)),
        SortPlan::Ask {
            selection: column,
            expanded: Rect::new(0, 0, 6, 3),
        }
    );
    // A whole column is first cut down to the used rows.
    let whole = Rect::new(0, 1, opensheet_engine::MAX_ROWS - 1, 1);
    assert_eq!(
        sort_plan(&wb, 0, whole, (0, 1)),
        SortPlan::Ask {
            selection: Rect::new(0, 1, 6, 1),
            expanded: Rect::new(0, 0, 6, 3),
        }
    );
    // Several columns are sorted as selected, without a question.
    let block = Rect::new(1, 0, 6, 1);
    assert_eq!(
        sort_plan(&wb, 0, block, (1, 0)),
        SortPlan::Sort(SortTarget::Range {
            rect: block,
            has_header: false,
        })
    );
}

#[test]
fn cells_of_a_filtered_list_stand_for_the_whole_list() {
    let wb = filtered_list();
    let target = SortTarget::Filter {
        scope: FilterScope::Sheet,
        range: Rect::new(0, 0, 6, 3),
    };
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(4, 2), (4, 2)),
        SortPlan::Sort(target)
    );
    assert_eq!(
        sort_plan(&wb, 0, Rect::new(1, 1, 6, 1), (1, 1)),
        SortPlan::Sort(target)
    );
    // Half in, half out: Excel refuses.
    assert_eq!(
        sort_plan(&wb, 0, Rect::new(4, 2, 9, 2), (4, 2)),
        SortPlan::StraddlesFilter
    );
}

#[test]
fn a_table_is_sorted_whole_with_or_without_its_buttons() {
    let mut wb = list();
    let id = wb.add_table(0, Rect::new(0, 0, 6, 3), true).unwrap();
    wb.commit("table");
    let range = Rect::new(0, 0, 6, 3);
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(2, 0), (2, 0)),
        SortPlan::Sort(SortTarget::Filter {
            scope: FilterScope::Table(id),
            range,
        })
    );
    wb.set_table_option(
        0,
        id,
        opensheet_engine::table::TableOption::FilterButton,
        false,
    )
    .unwrap();
    wb.commit("buttons off");
    assert_eq!(
        sort_plan(&wb, 0, Rect::cell(2, 0), (2, 0)),
        SortPlan::Sort(SortTarget::Range {
            rect: range,
            has_header: true,
        })
    );
}

#[test]
fn expanding_a_selection_takes_the_block_it_touches() {
    let wb = list();
    assert_eq!(
        block_around(&wb, 0, Rect::new(2, 2, 3, 2)),
        Rect::new(0, 0, 6, 3)
    );
    // Nothing but blanks: nothing to expand to.
    let blank = Rect::new(10, 10, 12, 10);
    assert_eq!(block_around(&wb, 0, blank), blank);
}

#[test]
fn sorting_through_a_filter_puts_the_arrow_on_its_button() {
    let mut wb = filtered_list();
    wb.sort_by_filter(0, FilterScope::Sheet, 1, SortOn::Values, true)
        .unwrap();
    wb.commit("sort");
    let quantities: Vec<String> = (1..=6).map(|row| wb.display_text(0, row, 1).text).collect();
    assert_eq!(quantities, ["10", "7", "7", "4", "3", "1"]);
    assert_eq!(wb.filter_button_at(0, 0, 1).unwrap().sort, Some(true));
    assert_eq!(wb.filter_button_at(0, 0, 0).unwrap().sort, None);
}

// ------------------------------------------------- context menu of a cell

#[test]
fn filtering_by_the_selected_cell_keeps_the_rows_like_it() {
    let mut wb = filtered_list();
    assert_eq!(
        criterion_like(&wb, 0, 2, 0, By::Value),
        Criteria::Values(ValueFilter {
            values: vec!["apple".into()],
            blanks: false,
            dates: Vec::new(),
        })
    );
    // A blank cell keeps the blanks.
    assert_eq!(
        criterion_like(&wb, 0, 5, 0, By::Value),
        Criteria::Values(ValueFilter {
            values: Vec::new(),
            blanks: true,
            dates: Vec::new(),
        })
    );
    wb.update_style(0, Rect::cell(3, 3), &|s| {
        s.fill = Some(Color(0xFFFF00));
        s.font.color = Some(Color(0xC00000));
    });
    wb.commit("colours");
    assert_eq!(
        criterion_like(&wb, 0, 3, 3, By::CellColor),
        Criteria::Color(ColorFilter {
            color: Some(Color(0xFFFF00)),
            font: false,
        })
    );
    assert_eq!(
        criterion_like(&wb, 0, 3, 3, By::FontColor),
        Criteria::Color(ColorFilter {
            color: Some(Color(0xC00000)),
            font: true,
        })
    );
    assert_eq!(
        criterion_like(&wb, 0, 2, 3, By::CellColor),
        Criteria::Color(ColorFilter {
            color: None,
            font: false
        })
    );

    let like = criterion_like(&wb, 0, 2, 0, By::Value);
    wb.set_filter_criteria(0, FilterScope::Sheet, 0, Some(like));
    wb.commit("filter");
    assert_eq!(wb.filter_record_count(0, FilterScope::Sheet), Some((2, 6)));
}

#[test]
fn a_cell_knows_the_filtered_column_it_is_in() {
    let mut wb = filtered_list();
    assert_eq!(
        filter_column_at(&wb, 0, 4, 1),
        Some((FilterScope::Sheet, 1, "Quantity".to_string(), false))
    );
    assert_eq!(filter_column_at(&wb, 0, 4, 5), None);
    let top = Criteria::Top10(Top10::new(true, false, 2.0));
    wb.set_filter_criteria(0, FilterScope::Sheet, 1, Some(top));
    assert_eq!(
        filter_column_at(&wb, 0, 0, 1).map(|found| found.3),
        Some(true)
    );
}

// ------------------------------------------------- the menu of a drop-down

#[test]
fn a_text_column_offers_text_filters() {
    let wb = filtered_list();
    let menu = entries(&dropdown(&wb, 0), 0, wb.locale);
    assert_eq!(
        labels(&menu),
        [
            "Sort A to Z",
            "Sort Z to A",
            "Sort by Color",
            "-",
            "Clear Filter From \"Fruit\"",
            "Filter by Color",
            "Text Filters",
        ]
    );
    // Nothing is filtered and every cell looks the same.
    assert!(find(&menu, "Clear Filter From \"Fruit\"").disabled);
    assert!(find(&menu, "Filter by Color").disabled);
    assert_eq!(
        labels(children(find(&menu, "Sort by Color"))),
        ["Custom Sort…"]
    );
    assert_eq!(
        labels(children(find(&menu, "Text Filters"))),
        [
            "Equals…",
            "Does Not Equal…",
            "-",
            "Begins With…",
            "Ends With…",
            "-",
            "Contains…",
            "Does Not Contain…",
            "-",
            "Custom Filter…",
        ]
    );
    let begins = find(children(find(&menu, "Text Filters")), "Begins With…");
    assert_eq!(
        begins.kind,
        EntryKind::Act(Act::Custom(Comparison::BeginsWith, None))
    );
}

#[test]
fn a_number_column_offers_number_filters() {
    let wb = filtered_list();
    let menu = entries(&dropdown(&wb, 1), 1, wb.locale);
    assert_eq!(
        &labels(&menu)[..2],
        ["Sort Smallest to Largest", "Sort Largest to Smallest"]
    );
    let filters = children(find(&menu, "Number Filters"));
    assert_eq!(
        labels(filters),
        [
            "Equals…",
            "Does Not Equal…",
            "-",
            "Greater Than…",
            "Greater Than Or Equal To…",
            "Less Than…",
            "Less Than Or Equal To…",
            "Between…",
            "-",
            "Top 10…",
            "Above Average",
            "Below Average",
            "-",
            "Custom Filter…",
        ]
    );
    assert_eq!(
        find(filters, "Between…").kind,
        EntryKind::Act(Act::Custom(
            Comparison::GreaterOrEqual,
            Some(Comparison::LessOrEqual)
        ))
    );
    assert_eq!(find(filters, "Top 10…").kind, EntryKind::Act(Act::Top10));
    assert_eq!(
        find(filters, "Above Average").kind,
        EntryKind::Act(Act::Dynamic(DynamicFilter::AboveAverage))
    );
}

#[test]
fn a_date_column_offers_date_filters() {
    let wb = filtered_list();
    let menu = entries(&dropdown(&wb, 2), 2, wb.locale);
    assert_eq!(
        &labels(&menu)[..2],
        ["Sort Oldest to Newest", "Sort Newest to Oldest"]
    );
    let filters = children(find(&menu, "Date Filters"));
    assert_eq!(
        labels(filters),
        [
            "Equals…",
            "-",
            "Before…",
            "After…",
            "Between…",
            "-",
            "Tomorrow",
            "Today",
            "Yesterday",
            "-",
            "Next Week",
            "This Week",
            "Last Week",
            "-",
            "Next Month",
            "This Month",
            "Last Month",
            "-",
            "Next Quarter",
            "This Quarter",
            "Last Quarter",
            "-",
            "Next Year",
            "This Year",
            "Last Year",
            "-",
            "Year to Date",
            "All Dates in the Period",
            "-",
            "Custom Filter…",
        ]
    );
    let period = children(find(filters, "All Dates in the Period"));
    assert_eq!(period.len(), 17);
    assert_eq!(
        &labels(period)[..6],
        [
            "Quarter 1",
            "Quarter 2",
            "Quarter 3",
            "Quarter 4",
            "-",
            "January"
        ]
    );
    assert_eq!(
        period[16].kind,
        EntryKind::Act(Act::Dynamic(DynamicFilter::Month(12)))
    );
    assert_eq!(
        find(filters, "Before…").kind,
        EntryKind::Act(Act::Custom(Comparison::Less, None))
    );
}

#[test]
fn the_menu_shows_what_is_in_use() {
    let mut wb = filtered_list();
    let scope = FilterScope::Sheet;
    let between = CustomFilter {
        first: Condition::new(Comparison::GreaterOrEqual, "3", wb.locale),
        second: Some(Condition::new(Comparison::LessOrEqual, "7", wb.locale)),
        and: true,
    };
    wb.set_filter_criteria(0, scope, 1, Some(Criteria::Custom(between.clone())));
    wb.sort_by_filter(0, scope, 1, SortOn::Values, false)
        .unwrap();
    let menu = entries(&dropdown(&wb, 1), 1, wb.locale);
    assert!(find(&menu, "Sort Smallest to Largest").checked);
    assert!(!find(&menu, "Sort Largest to Smallest").checked);
    assert!(!find(&menu, "Clear Filter From \"Quantity\"").disabled);
    let filters = find(&menu, "Number Filters");
    assert!(filters.checked);
    assert!(find(children(filters), "Between…").checked);
    assert!(!find(children(filters), "Custom Filter…").checked);

    // Either of two conditions is no entry of the menu: "Custom Filter".
    let either = CustomFilter {
        and: false,
        ..between
    };
    wb.set_filter_criteria(0, scope, 1, Some(Criteria::Custom(either)));
    let menu = entries(&dropdown(&wb, 1), 1, wb.locale);
    let filters = children(find(&menu, "Number Filters"));
    assert!(!find(filters, "Between…").checked);
    assert!(find(filters, "Custom Filter…").checked);

    wb.set_filter_criteria(
        0,
        scope,
        1,
        Some(Criteria::Top10(Top10::new(true, false, 3.0))),
    );
    let menu = entries(&dropdown(&wb, 1), 1, wb.locale);
    assert!(find(children(find(&menu, "Number Filters")), "Top 10…").checked);

    wb.set_filter_criteria(
        0,
        scope,
        2,
        Some(Criteria::Dynamic(DynamicFilter::Quarter(1))),
    );
    let menu = entries(&dropdown(&wb, 2), 2, wb.locale);
    let period = find(
        children(find(&menu, "Date Filters")),
        "All Dates in the Period",
    );
    assert!(period.checked);
    assert!(find(children(period), "Quarter 1").checked);
}

#[test]
fn colours_are_offered_when_the_cells_differ() {
    let mut wb = filtered_list();
    wb.update_style(0, Rect::cell(2, 3), &|s| s.fill = Some(Color(0xFFFF00)));
    wb.update_style(0, Rect::cell(4, 3), &|s| {
        s.font.color = Some(Color(0xC00000))
    });
    wb.commit("colours");
    let menu = entries(&dropdown(&wb, 3), 3, wb.locale);
    let sort = children(find(&menu, "Sort by Color"));
    assert_eq!(
        labels(sort),
        [
            "Sort by Cell Color",
            "RGB(255, 255, 0)",
            "No Fill",
            "Sort by Font Color",
            "RGB(192, 0, 0)",
            "Automatic",
            "-",
            "Custom Sort…",
        ]
    );
    assert_eq!(sort[0].kind, EntryKind::Heading);
    assert_eq!(sort[1].swatch, Some(Color(0xFFFF00)));
    assert_eq!(
        sort[1].kind,
        EntryKind::Act(Act::SortColor(SortOn::CellColor(Some(Color(0xFFFF00)))))
    );
    assert_eq!(
        sort[5].kind,
        EntryKind::Act(Act::SortColor(SortOn::FontColor(None)))
    );

    let by_color = find(&menu, "Filter by Color");
    assert!(!by_color.disabled && !by_color.checked);
    assert_eq!(
        labels(children(by_color)),
        [
            "Filter by Cell Color",
            "RGB(255, 255, 0)",
            "No Fill",
            "Filter by Font Color",
            "RGB(192, 0, 0)",
            "Automatic",
        ]
    );

    let yellow = ColorFilter {
        color: Some(Color(0xFFFF00)),
        font: false,
    };
    wb.set_filter_criteria(0, FilterScope::Sheet, 3, Some(Criteria::Color(yellow)));
    let menu = entries(&dropdown(&wb, 3), 3, wb.locale);
    let by_color = find(&menu, "Filter by Color");
    assert!(by_color.checked);
    assert!(children(by_color)[1].checked);
    assert!(!children(by_color)[2].checked);
}

// --------------------------------------------------- the list of a drop-down

fn rows_text(list: &ValueList) -> Vec<String> {
    list.rows().into_iter().map(|row| list.label(row)).collect()
}

#[test]
fn the_list_shows_the_values_and_the_blanks() {
    let wb = filtered_list();
    let list = ValueList::new(dropdown(&wb, 0), wb.locale);
    assert_eq!(
        rows_text(&list),
        ["(Select All)", "apple", "cherry", "fig", "pear", "(Blanks)"]
    );
    assert_eq!(list.state(Row::All), Tri::On);
    assert!(list.can_apply());
    assert!(!list.has_tree());
    // Untouched, it filters nothing.
    assert_eq!(list.criteria(), None);
}

#[test]
fn ticking_boxes_makes_a_values_filter() {
    let wb = filtered_list();
    let mut list = ValueList::new(dropdown(&wb, 0), wb.locale);
    list.toggle(Row::Item(0));
    assert_eq!(list.state(Row::All), Tri::Mixed);
    list.toggle(Row::Blanks);
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: vec!["cherry".into(), "fig".into(), "pear".into()],
            blanks: false,
            dates: Vec::new(),
        }))
    );
    // "(Select All)" on a mixed list ticks everything; again, nothing.
    list.toggle(Row::All);
    assert_eq!(list.state(Row::All), Tri::On);
    assert_eq!(list.criteria(), None);
    list.toggle(Row::All);
    assert_eq!(list.state(Row::All), Tri::Off);
    assert!(!list.can_apply());
}

#[test]
fn dates_are_listed_as_a_tree() {
    let wb = filtered_list();
    let mut list = ValueList::new(dropdown(&wb, 2), wb.locale);
    assert!(list.has_tree());
    // Years, most recent first, folded.
    assert_eq!(rows_text(&list), ["(Select All)", "2026", "2025"]);
    assert_eq!(list.expanded(Row::Year(0)), Some(false));
    assert_eq!(list.expanded(Row::All), None);
    list.set_expanded(Row::Year(0), true);
    assert_eq!(
        rows_text(&list),
        [
            "(Select All)",
            "2026",
            "January",
            "February",
            "March",
            "2025"
        ]
    );
    list.set_expanded(Row::Month(0, 1), true);
    assert_eq!(
        rows_text(&list),
        [
            "(Select All)",
            "2026",
            "January",
            "February",
            "11",
            "12",
            "March",
            "2025"
        ]
    );
    assert_eq!(list.depth(Row::Year(0)), 0);
    assert_eq!(list.depth(Row::Month(0, 1)), 1);
    assert_eq!(list.depth(Row::Day(0, 1, 0)), 2);

    // Unticking a day leaves its month and its year half ticked.
    list.toggle(Row::Day(0, 1, 0));
    assert_eq!(list.state(Row::Month(0, 1)), Tri::Mixed);
    assert_eq!(list.state(Row::Year(0)), Tri::Mixed);
    assert_eq!(list.state(Row::Month(0, 0)), Tri::On);
    assert_eq!(list.state(Row::Year(1)), Tri::On);
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: Vec::new(),
            blanks: false,
            dates: vec![
                DateGroup::month(2026, 1),
                DateGroup::day(2026, 2, 12),
                DateGroup::month(2026, 3),
                DateGroup::year(2025),
            ],
        }))
    );
    // A click on a half-ticked month ticks it whole; on a year, clears it.
    list.toggle(Row::Month(0, 1));
    assert_eq!(list.state(Row::Year(0)), Tri::On);
    list.toggle(Row::Year(0));
    assert_eq!(list.state(Row::Year(0)), Tri::Off);
    assert_eq!(list.state(Row::Day(0, 1, 1)), Tri::Off);
    assert_eq!(list.state(Row::All), Tri::Mixed);
}

#[test]
fn searching_narrows_the_list_and_ticks_what_it_finds() {
    let mut wb = filtered_list();
    let only_pear = Criteria::Values(ValueFilter {
        values: vec!["pear".into()],
        blanks: false,
        dates: Vec::new(),
    });
    wb.set_filter_criteria(0, FilterScope::Sheet, 0, Some(only_pear));
    let mut list = ValueList::new(dropdown(&wb, 0), wb.locale);
    assert_eq!(list.state(Row::All), Tri::Mixed);

    list.set_search("  E ");
    assert!(list.searching());
    assert_eq!(
        rows_text(&list),
        [
            "(Select All Search Results)",
            "Add current selection to filter",
            "apple",
            "cherry",
            "pear",
        ]
    );
    // Everything found is ticked, whatever it was.
    assert_eq!(list.state(Row::All), Tri::On);
    assert_eq!(list.state(Row::Add), Tri::Off);
    list.toggle(Row::Item(1));
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: vec!["apple".into(), "pear".into()],
            blanks: false,
            dates: Vec::new(),
        }))
    );

    // Nothing found: nothing listed, nothing to apply.
    list.set_search("zzz");
    assert!(list.rows().is_empty());
    assert!(!list.can_apply());

    // Wildcards work as in Excel's box.
    list.set_search("?pp*");
    assert_eq!(rows_text(&list)[2..], ["apple"]);

    // Emptied, the list is back as it opened.
    list.set_search("");
    assert_eq!(
        rows_text(&list),
        ["(Select All)", "apple", "cherry", "fig", "pear", "(Blanks)"]
    );
    assert_eq!(list.state(Row::Item(3)), Tri::On);
    assert_eq!(list.state(Row::Item(0)), Tri::Off);
}

#[test]
fn search_results_can_be_added_to_the_current_filter() {
    let mut wb = filtered_list();
    let only_pear = Criteria::Values(ValueFilter {
        values: vec!["pear".into()],
        blanks: false,
        dates: Vec::new(),
    });
    wb.set_filter_criteria(0, FilterScope::Sheet, 0, Some(only_pear));
    let mut list = ValueList::new(dropdown(&wb, 0), wb.locale);
    list.set_search("fig");
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: vec!["fig".into()],
            blanks: false,
            dates: Vec::new(),
        }))
    );
    list.toggle(Row::Add);
    assert_eq!(list.state(Row::Add), Tri::On);
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: vec!["fig".into(), "pear".into()],
            blanks: false,
            dates: Vec::new(),
        }))
    );
}

#[test]
fn searching_finds_dates_by_year_month_or_day() {
    let wb = filtered_list();
    let mut list = ValueList::new(dropdown(&wb, 2), wb.locale);
    list.set_search("feb");
    // The branches holding what was found are unfolded.
    assert_eq!(
        rows_text(&list),
        [
            "(Select All Search Results)",
            "Add current selection to filter",
            "2026",
            "February",
            "11",
            "12",
        ]
    );
    assert_eq!(
        list.criteria(),
        Some(Criteria::Values(ValueFilter {
            values: Vec::new(),
            blanks: false,
            dates: vec![DateGroup::month(2026, 2)],
        }))
    );
    list.set_search("2025");
    assert_eq!(rows_text(&list)[2..], ["2025", "December", "24"]);
}

// ------------------------------------------------------- custom autofilter

#[test]
fn the_comparisons_are_worded_for_dates() {
    let numbers: Vec<&str> = comparisons(ColumnKind::Number)
        .iter()
        .map(|(_, name)| *name)
        .collect();
    assert_eq!(
        numbers,
        [
            "equals",
            "does not equal",
            "is greater than",
            "is greater than or equal to",
            "is less than",
            "is less than or equal to",
            "begins with",
            "does not begin with",
            "ends with",
            "does not end with",
            "contains",
            "does not contain",
        ]
    );
    let dates = comparisons(ColumnKind::Date);
    assert_eq!(dates[2], (Comparison::Greater, "is after"));
    assert_eq!(dates[5], (Comparison::LessOrEqual, "is before or equal to"));
    assert_eq!(dates.len(), 12);
}

#[test]
fn the_custom_dialog_opens_on_what_was_asked() {
    let en = Locale::en();
    // From "Between...": both comparisons, joined by And.
    let between = Fields::opening(
        None,
        Some((Comparison::GreaterOrEqual, Some(Comparison::LessOrEqual))),
        false,
        en,
    );
    assert_eq!(
        between.comparisons,
        [
            Some(Comparison::GreaterOrEqual),
            Some(Comparison::LessOrEqual)
        ]
    );
    assert!(between.and && between.operands == [String::new(), String::new()]);
    // From "Custom Filter..." on an unfiltered column.
    let fresh = Fields::opening(None, None, false, en);
    assert_eq!(fresh.comparisons, [Some(Comparison::Equals), None]);
    // On the column's own conditions.
    let custom = Criteria::Custom(CustomFilter {
        first: Condition::new(Comparison::Contains, "a", en),
        second: Some(Condition::new(Comparison::Less, "2.5", Locale::en())),
        and: false,
    });
    let fields = Fields::opening(Some(&custom), None, false, Locale::by_id("fr"));
    assert_eq!(
        fields.comparisons,
        [Some(Comparison::Contains), Some(Comparison::Less)]
    );
    assert_eq!(fields.operands, ["a".to_string(), "2,5".to_string()]);
    assert!(!fields.and);
}

#[test]
fn the_custom_dialog_makes_one_or_two_conditions() {
    let en = Locale::en();
    let fields = |comparisons: [Option<Comparison>; 2], operands: [&str; 2], and: bool| Fields {
        comparisons,
        operands: operands.map(str::to_string),
        and,
    };
    let one = fields(
        [Some(Comparison::BeginsWith), None],
        ["ap", "ignored"],
        true,
    );
    assert_eq!(
        one.criteria(en),
        Some(Criteria::Custom(CustomFilter::one(Condition::new(
            Comparison::BeginsWith,
            "ap",
            en
        ))))
    );
    let two = fields(
        [Some(Comparison::Greater), Some(Comparison::Less)],
        [" 3 ", "7"],
        false,
    );
    assert_eq!(
        two.criteria(en),
        Some(Criteria::Custom(CustomFilter {
            first: Condition::new(Comparison::Greater, "3", en),
            second: Some(Condition::new(Comparison::Less, "7", en)),
            and: false,
        }))
    );
    // Only the second line is used.
    let second = fields([None, Some(Comparison::Contains)], ["x", "pp"], true);
    assert_eq!(
        second.criteria(en),
        Some(Criteria::Custom(CustomFilter::one(Condition::new(
            Comparison::Contains,
            "pp",
            en
        ))))
    );
    // A comparison without operand is dropped, unless it asks for blanks.
    let half = fields(
        [
            Some(Comparison::GreaterOrEqual),
            Some(Comparison::LessOrEqual),
        ],
        ["3", ""],
        true,
    );
    assert_eq!(
        half.criteria(en),
        Some(Criteria::Custom(CustomFilter::one(Condition::new(
            Comparison::GreaterOrEqual,
            "3",
            en
        ))))
    );
    let blanks = fields([Some(Comparison::Equals), None], ["", ""], true);
    assert_eq!(
        blanks.criteria(en),
        Some(Criteria::Custom(CustomFilter::one(Condition::new(
            Comparison::Equals,
            "",
            en
        ))))
    );
    // Nothing chosen: the filter is cleared.
    assert_eq!(fields([None, None], ["3", "4"], true).criteria(en), None);
}

#[test]
fn a_custom_filter_applies_and_reads_back() {
    let mut wb = filtered_list();
    let fields = Fields {
        comparisons: [
            Some(Comparison::GreaterOrEqual),
            Some(Comparison::LessOrEqual),
        ],
        operands: ["2/1/2026".to_string(), "2/28/2026".to_string()],
        and: true,
    };
    let criteria = fields.criteria(wb.locale);
    wb.set_filter_criteria(0, FilterScope::Sheet, 2, criteria);
    wb.commit("filter");
    assert_eq!(wb.filter_record_count(0, FilterScope::Sheet), Some((2, 6)));
    let data = dropdown(&wb, 2);
    assert_eq!(
        Fields::opening(data.criteria.as_ref(), None, true, wb.locale),
        fields
    );
}

#[test]
fn the_value_boxes_suggest_the_values_of_the_column() {
    let wb = filtered_list();
    assert_eq!(
        suggestions(&dropdown(&wb, 0), wb.locale),
        ["apple", "cherry", "fig", "pear"]
    );
    assert_eq!(
        suggestions(&dropdown(&wb, 2), wb.locale),
        [
            "12/24/2025",
            "1/5/2026",
            "2/11/2026",
            "2/12/2026",
            "3/1/2026",
            "3/2/2026"
        ]
    );
}

#[test]
fn top_10_takes_whole_numbers_from_1_to_500() {
    assert_eq!(
        top10_criteria(true, false, Some(10.0)),
        Some(Criteria::Top10(Top10::new(true, false, 10.0)))
    );
    assert_eq!(
        top10_criteria(false, true, Some(500.0)),
        Some(Criteria::Top10(Top10::new(false, true, 500.0)))
    );
    for refused in [None, Some(0.0), Some(501.0), Some(2.5), Some(-3.0)] {
        assert_eq!(top10_criteria(true, false, refused), None);
    }
}

// ----------------------------------------------------------- the sort dialog

fn range_target() -> SortTarget {
    SortTarget::Range {
        rect: Rect::new(0, 0, 6, 3),
        has_header: true,
    }
}

fn level(key: u32, descending: bool) -> Level {
    Level {
        key: Some(key),
        descending,
        ..Level::blank()
    }
}

#[test]
fn the_sort_dialog_opens_on_one_empty_level() {
    let wb = list();
    let setup = Setup::opening(&wb, 0, range_target(), None);
    assert_eq!(setup.levels, [Level::blank()]);
    assert!(setup.has_header && !setup.case_sensitive && !setup.left_to_right);
    assert!(!setup.header_is_fixed());
    assert_eq!(setup.spec(), Err(Problem::NoKey));
}

#[test]
fn the_sort_dialog_remembers_the_last_sort_of_the_same_cells() {
    let wb = list();
    let mut last = Setup::opening(&wb, 0, range_target(), None);
    last.levels = vec![level(1, true)];
    last.case_sensitive = true;
    assert_eq!(Setup::opening(&wb, 0, range_target(), Some(&last)), last);
    // Other cells start afresh.
    let other = SortTarget::Range {
        rect: Rect::new(1, 0, 6, 1),
        has_header: false,
    };
    let setup = Setup::opening(&wb, 0, other, Some(&last));
    assert_eq!(setup.levels, [Level::blank()]);
    assert!(!setup.has_header);
}

#[test]
fn the_sort_dialog_opens_on_the_sort_of_a_filter() {
    let mut wb = filtered_list();
    wb.sort_by_filter(
        0,
        FilterScope::Sheet,
        2,
        SortOn::FontColor(Some(Color(0xC00000))),
        true,
    )
    .unwrap();
    let target = SortTarget::Filter {
        scope: FilterScope::Sheet,
        range: Rect::new(0, 0, 6, 3),
    };
    let setup = Setup::opening(&wb, 0, target, None);
    assert_eq!(
        setup.levels,
        [Level {
            key: Some(2),
            basis: Basis::FontColor,
            descending: true,
            color: Some(Color(0xC00000)),
            custom_list: None,
        }]
    );
    assert!(setup.header_is_fixed());
}

#[test]
fn the_levels_become_the_sort_of_the_engine() {
    let wb = list();
    let mut setup = Setup::opening(&wb, 0, range_target(), None);
    let days = vec!["Mon".to_string(), "Tue".to_string()];
    setup.levels = vec![
        level(0, false),
        Level {
            basis: Basis::CellColor,
            color: Some(Color(0xFFFF00)),
            ..level(0, true)
        },
        Level {
            custom_list: Some(days.clone()),
            ..level(3, true)
        },
    ];
    setup.case_sensitive = true;
    let spec = setup.spec().unwrap();
    assert_eq!(
        spec.levels,
        [
            SortLevel::values(0, false),
            SortLevel {
                key: 0,
                on: SortOn::CellColor(Some(Color(0xFFFF00))),
                descending: true,
                custom_list: None,
            },
            SortLevel {
                key: 3,
                on: SortOn::Values,
                descending: true,
                custom_list: Some(days),
            },
        ]
    );
    assert!(spec.has_header && spec.case_sensitive && !spec.left_to_right);

    // The same values twice, a key outside the range, a key not chosen.
    setup.levels = vec![level(1, false), level(1, true)];
    assert_eq!(setup.spec(), Err(Problem::Repeated(1)));
    setup.levels = vec![level(4, false)];
    assert_eq!(setup.spec(), Err(Problem::NoKey));
    setup.levels = vec![level(1, false), Level::blank()];
    assert_eq!(setup.spec(), Err(Problem::NoKey));
    // No level at all sorts nothing, without complaint.
    setup.levels.clear();
    assert_eq!(setup.spec().map(|spec| spec.levels.len()), Ok(0));
}

#[test]
fn the_keys_are_named_after_headers_letters_or_rows() {
    let mut wb = list();
    wb.set_cell_text(0, 0, 3, "").unwrap();
    wb.commit("blank header");
    let mut setup = Setup::opening(&wb, 0, range_target(), None);
    assert_eq!(
        sort::key_names(&wb, 0, &setup),
        ["Fruit", "Quantity", "Picked", "(Column D)"]
    );
    setup.has_header = false;
    assert_eq!(
        sort::key_names(&wb, 0, &setup),
        ["Column A", "Column B", "Column C", "Column D"]
    );
    setup.left_to_right = true;
    let rows = sort::key_names(&wb, 0, &setup);
    assert_eq!(rows.len(), 7);
    assert_eq!((rows[0].as_str(), rows[6].as_str()), ("Row 1", "Row 7"));
    // Left to right, every row is sorted: the header choice does not count.
    setup.has_header = true;
    setup.levels = vec![level(6, false)];
    let spec = setup.spec().unwrap();
    assert!(spec.left_to_right && !spec.has_header);
}

#[test]
fn a_key_tells_its_kind_and_its_colours() {
    let mut wb = list();
    wb.update_style(0, Rect::cell(2, 1), &|s| s.fill = Some(Color(0xFFFF00)));
    wb.update_style(0, Rect::cell(4, 1), &|s| {
        s.font.color = Some(Color(0xC00000))
    });
    wb.commit("colours");
    let setup = Setup::opening(&wb, 0, range_target(), None);
    let quantity = sort::facts(&wb, 0, &setup, Some(1));
    assert_eq!(quantity.kind, ColumnKind::Number);
    assert_eq!(quantity.cell_colors, [None, Some(Color(0xFFFF00))]);
    assert_eq!(quantity.font_colors, [None, Some(Color(0xC00000))]);
    assert_eq!(sort::facts(&wb, 0, &setup, Some(2)).kind, ColumnKind::Date);
    assert_eq!(sort::facts(&wb, 0, &setup, None).kind, ColumnKind::Text);
    assert_eq!(
        sort::orders(ColumnKind::Number),
        ["Smallest to Largest", "Largest to Smallest"]
    );
    assert_eq!(
        sort::orders(ColumnKind::Date),
        ["Oldest to Newest", "Newest to Oldest"]
    );
    assert_eq!(sort::orders(ColumnKind::Text), ["A to Z", "Z to A"]);
}

#[test]
fn custom_lists_are_named_by_their_items() {
    let list = vec!["Mon".to_string(), "Tue".to_string(), "Wed".to_string()];
    assert_eq!(sort::list_name(&list, false), "Mon, Tue, Wed");
    assert_eq!(sort::list_name(&list, true), "Wed, Tue, Mon");
}

#[test]
fn the_sort_dialog_sorts_a_range_on_several_levels() {
    let mut wb = list();
    let mut setup = Setup::opening(&wb, 0, range_target(), None);
    // By note, then by quantity, largest first; rows that tie keep their
    // order (fig before cherry), a blank note comes last.
    setup.levels = vec![level(3, false), level(1, true)];
    let spec = setup.spec().unwrap();
    setup.apply(&mut wb, 0, spec).unwrap();
    wb.commit("sort");
    let fruits: Vec<String> = (0..=6).map(|row| wb.display_text(0, row, 0).text).collect();
    assert_eq!(
        fruits,
        ["Fruit", "apple", "", "fig", "cherry", "pear", "apple"]
    );
}

#[test]
fn the_sort_dialog_sorts_a_filtered_list_through_its_filter() {
    let mut wb = filtered_list();
    let target = SortTarget::Filter {
        scope: FilterScope::Sheet,
        range: Rect::new(0, 0, 6, 3),
    };
    let mut setup = Setup::opening(&wb, 0, target, None);
    setup.levels = vec![level(0, true), level(1, false)];
    let spec = setup.spec().unwrap();
    setup.apply(&mut wb, 0, spec).unwrap();
    wb.commit("sort");
    let fruits: Vec<String> = (1..=6).map(|row| wb.display_text(0, row, 0).text).collect();
    assert_eq!(fruits, ["pear", "fig", "cherry", "apple", "apple", ""]);
    // The filter remembers both levels: arrows on both buttons.
    assert_eq!(wb.filter_button_at(0, 0, 0).unwrap().sort, Some(true));
    assert_eq!(wb.filter_button_at(0, 0, 1).unwrap().sort, Some(false));
    assert_eq!(Setup::opening(&wb, 0, target, None).levels, setup.levels);

    // Left to right goes around the filter and leaves its arrows alone.
    setup.left_to_right = true;
    setup.levels = vec![level(0, false)];
    let spec = setup.spec().unwrap();
    setup.apply(&mut wb, 0, spec).unwrap();
    wb.commit("sort columns");
    let headers: Vec<String> = (0..=3).map(|col| wb.display_text(0, 0, col).text).collect();
    assert_eq!(headers, ["Fruit", "Note", "Picked", "Quantity"]);
    assert_eq!(wb.filter_button_at(0, 0, 0).unwrap().sort, Some(true));
}

// -------------------------------------------------------- remove duplicates

#[test]
fn remove_duplicates_acts_on_the_block_or_the_table() {
    let mut wb = list();
    assert_eq!(
        duplicates::plan(&wb, 0, Rect::cell(3, 1), (3, 1)),
        Plan::Rows {
            rect: Rect::new(0, 0, 6, 3),
            has_header: true,
            fixed_header: false,
        }
    );
    assert_eq!(
        duplicates::plan(&wb, 0, Rect::cell(30, 9), (30, 9)),
        Plan::Nothing
    );
    // Part of a block: the rest may follow.
    let part = Rect::new(1, 0, 6, 1);
    assert_eq!(
        duplicates::plan(&wb, 0, part, (1, 0)),
        Plan::Ask {
            selection: part,
            expanded: Rect::new(0, 0, 6, 3),
        }
    );
    // A block selected whole is taken as it is.
    let whole = Rect::new(0, 0, 6, 3);
    assert_eq!(
        duplicates::plan(&wb, 0, whole, (0, 0)),
        Plan::Rows {
            rect: whole,
            has_header: true,
            fixed_header: false,
        }
    );
    wb.add_table(0, whole, true).unwrap();
    wb.commit("table");
    assert_eq!(
        duplicates::plan(&wb, 0, Rect::cell(3, 1), (3, 1)),
        Plan::Rows {
            rect: whole,
            has_header: true,
            fixed_header: true,
        }
    );
}

#[test]
fn remove_duplicates_lists_headers_or_letters() {
    let wb = list();
    let rect = Rect::new(0, 0, 6, 3);
    assert_eq!(
        duplicates::column_names(&wb, 0, rect, true),
        ["Fruit", "Quantity", "Picked", "Note"]
    );
    assert_eq!(
        duplicates::column_names(&wb, 0, rect, false),
        ["Column A", "Column B", "Column C", "Column D"]
    );
}

#[test]
fn remove_duplicates_reports_with_excel_wording() {
    let mut wb = list();
    // Rows repeating an earlier fruit go: one "apple".
    let outcome = wb
        .remove_duplicates(0, Rect::new(0, 0, 6, 3), &[0], true)
        .unwrap();
    wb.commit("duplicates");
    assert_eq!(
        outcome,
        DuplicatesRemoved {
            removed: 1,
            remaining: 5
        }
    );
    assert_eq!(
        duplicates::outcome_message(outcome),
        "1 duplicate values found and removed; 5 unique values remain. Note that counts may include empty cells, spaces, etc."
    );
    let none = DuplicatesRemoved {
        removed: 0,
        remaining: 5,
    };
    assert_eq!(
        duplicates::outcome_message(none),
        "No duplicate values found."
    );
}

// --------------------------------------------- the context menu, as entries

fn entry(label: &'static str) -> MenuEntry {
    MenuEntry::new(label, |_, _, _| {})
}

fn submenu(entry: &MenuEntry) -> &[MenuEntry] {
    entry.submenu.as_deref().expect("a submenu")
}

fn names(entries: &[MenuEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| entry.label.to_string())
        .collect()
}

#[test]
fn the_context_menu_of_a_cell_has_filter_and_sort_submenus() {
    let mut wb = filtered_list();
    let menu = |wb: &Workbook, row: u32, col: u32| {
        cell_menu(wb, 0, row, col, entry("Reapply"), entry("Custom Sort…"))
    };
    let [filter, sort] = menu(&wb, 2, 1);
    assert_eq!(
        (filter.label.as_ref(), sort.label.as_ref()),
        ("Filter", "Sort")
    );
    assert_eq!(
        names(submenu(&filter)),
        [
            "Clear Filter From \"Quantity\"",
            "Reapply",
            "Filter by Selected Cell's Value",
            "Filter by Selected Cell's Color",
            "Filter by Selected Cell's Font Color",
        ]
    );
    // Nothing to clear yet.
    assert!(submenu(&filter)[0].disabled);
    assert_eq!(
        names(submenu(&sort)),
        [
            "Sort Smallest to Largest",
            "Sort Largest to Smallest",
            "Put Selected Cell Color On Top",
            "Put Selected Font Color On Top",
            "Custom Sort…",
        ]
    );
    // The sort entries are worded for the cell: a date, a text.
    assert_eq!(
        names(submenu(&menu(&wb, 2, 2)[1]))[0],
        "Sort Oldest to Newest"
    );
    assert_eq!(names(submenu(&menu(&wb, 2, 0)[1]))[1], "Sort Z to A");

    let top = Criteria::Top10(Top10::new(true, false, 2.0));
    wb.set_filter_criteria(0, FilterScope::Sheet, 1, Some(top));
    assert!(!submenu(&menu(&wb, 2, 1)[0])[0].disabled);
    // Another column of the list has nothing to clear; a cell outside any
    // list has no column to name.
    assert!(submenu(&menu(&wb, 2, 0)[0])[0].disabled);
    let outside = menu(&wb, 0, 5);
    assert_eq!(names(submenu(&outside[0]))[0], "Clear Filter");
    assert!(submenu(&outside[0])[0].disabled);
}
