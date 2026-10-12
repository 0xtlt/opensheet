//! Tests of the pivot tables. The expected reports were worked out by hand
//! from the small data sets below.

use super::*;
use crate::locale::Locale;
use crate::refs::parse_cell_name;
use crate::style::Color;
use crate::value::ErrorKind;
use crate::workbook::Workbook;

/// Six sales: Region, Product, Year, Sales, Units.
///
/// Sales by region and year:      2023  2024  total
///                         East    250   120    370
///                         West    200   120    320
///                         total   450   240    690
/// Sales by region and product: East Apples 220, East Pears 150,
/// West Apples 200, West Pears 120. Units: East 27, West 29.
const SALES: &[[&str; 5]] = &[
    ["Region", "Product", "Year", "Sales", "Units"],
    ["East", "Apples", "2023", "100", "10"],
    ["East", "Pears", "2023", "150", "5"],
    ["East", "Apples", "2024", "120", "12"],
    ["West", "Apples", "2023", "200", "20"],
    ["West", "Pears", "2024", "50", "2"],
    ["West", "Pears", "2024", "70", "7"],
];

const REGION: usize = 0;
const PRODUCT: usize = 1;
const YEAR: usize = 2;
const SALES_F: usize = 3;
const UNITS: usize = 4;

/// A workbook whose first sheet holds `rows` from A1, and an empty
/// PivotTable on them placed on a new second sheet (at A3, as in Excel).
fn pivot_on<const N: usize>(rows: &[[&str; N]], locale: &'static Locale) -> (Workbook, u32) {
    let mut wb = Workbook::new(locale);
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
        }
    }
    wb.commit("data");
    let source = PivotSource::Range {
        sheet: wb.sheets[0].name.clone(),
        range: Rect::new(0, 0, rows.len() as u32 - 1, N as u32 - 1),
    };
    let id = wb
        .pivot_create(source, PivotDestination::NewSheet { index: 1 }, false)
        .unwrap();
    wb.commit("pivot");
    (wb, id)
}

fn sales(locale: &'static Locale) -> (Workbook, u32) {
    pivot_on(SALES, locale)
}

/// Apply a change to the PivotTable and commit it.
fn update(wb: &mut Workbook, id: u32, edit: impl FnOnce(&mut PivotTable)) {
    wb.pivot_update(id, false, edit).unwrap();
    wb.commit("pivot");
}

/// The cells of a range as the grid shows them: rows separated by line
/// breaks, cells by `|`.
fn cells(wb: &Workbook, sheet: usize, rect: Rect) -> String {
    (rect.r0..=rect.r1)
        .map(|r| {
            (rect.c0..=rect.c1)
                .map(|c| wb.display_text(sheet, r, c).text)
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The report of a PivotTable as the grid shows it.
fn shown(wb: &Workbook, id: u32) -> String {
    let (si, p) = wb.pivot(id).unwrap();
    cells(wb, si, p.location.unwrap())
}

/// Compare a report with the expected one, written one row per line.
#[track_caller]
fn assert_report(wb: &Workbook, id: u32, expected: &str) {
    let expected: Vec<&str> = expected.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    assert_eq!(shown(wb, id), expected.join("\n"));
}

fn at(wb: &Workbook, id: u32, name: &str) -> Value {
    let (si, _) = wb.pivot(id).unwrap();
    let (r, c) = parse_cell_name(name).unwrap();
    wb.value_at(si, r, c)
}

#[track_caller]
fn assert_close(v: Value, expected: f64) {
    match v {
        Value::Number(n) => assert!((n - expected).abs() < 1e-9, "{n} is not {expected}"),
        other => panic!("{other:?} is not {expected}"),
    }
}

// ---- creation ----------------------------------------------------------------

#[test]
fn a_new_pivot_reserves_its_frame() {
    let (wb, id) = sales(Locale::en());
    let (si, p) = wb.pivot(id).unwrap();
    assert_eq!(si, 1);
    assert_eq!(wb.sheets[1].name, "Sheet2");
    assert_eq!(p.name, "PivotTable1");
    // Excel's placeholder: A3:C20.
    assert_eq!(p.location, Some(Rect::new(2, 0, 19, 2)));
    assert_eq!(p.page_location, None);
    assert_eq!(wb.pivot_cell(1, 2, 0).unwrap().part, PivotPart::Placeholder);
    assert!(wb.pivot_cell(1, 20, 0).is_none());
    assert!(wb.sheets[1].cells.is_empty());
    let names: Vec<&str> = p.cache.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Region", "Product", "Year", "Sales", "Units"]);
    assert_eq!(p.cache.rows, 6);
    // Numbers are summed by default, the rest goes to the rows.
    assert_eq!(p.default_area(REGION), Area::Rows);
    assert_eq!(p.default_area(SALES_F), Area::Values);
}

#[test]
fn default_names_follow_the_language() {
    for (locale, name) in [
        (Locale::en(), "PivotTable"),
        (Locale::fr(), "Tableau croisé dynamique"),
        (Locale::es(), "TablaDinámica"),
        (Locale::de(), "PivotTable"),
        (Locale::it(), "Tabella pivot"),
    ] {
        let (mut wb, id) = sales(locale);
        assert_eq!(wb.pivot(id).unwrap().1.name, format!("{name}1"));
        let source = wb.pivot(id).unwrap().1.source.clone();
        let second = wb
            .pivot_create(
                source,
                PivotDestination::Existing {
                    sheet: 1,
                    row: 30,
                    col: 0,
                },
                false,
            )
            .unwrap();
        assert_eq!(wb.pivot(second).unwrap().1.name, format!("{name}2"));
    }
}

#[test]
fn the_source_must_be_a_list() {
    let mut wb = Workbook::new(Locale::en());
    wb.set_cell_text(0, 0, 0, "Region").unwrap();
    wb.set_cell_text(0, 0, 2, "Sales").unwrap();
    wb.set_cell_text(0, 1, 0, "East").unwrap();
    wb.commit("data");
    let range = |r: Rect| PivotSource::Range {
        sheet: "Sheet1".to_string(),
        range: r,
    };
    let dest = PivotDestination::Existing {
        sheet: 0,
        row: 10,
        col: 0,
    };
    // A header row alone, a column without a header, a sheet that is gone.
    assert_eq!(
        wb.pivot_create(range(Rect::new(0, 0, 0, 0)), dest, false),
        Err(PivotError::InvalidSource)
    );
    assert_eq!(
        wb.pivot_create(range(Rect::new(0, 0, 1, 2)), dest, false),
        Err(PivotError::EmptyFieldName)
    );
    let gone = PivotSource::Range {
        sheet: "Nowhere".to_string(),
        range: Rect::new(0, 0, 1, 0),
    };
    assert_eq!(wb.pivot_create(gone, dest, false), Err(PivotError::InvalidSource));
    assert_eq!(
        wb.pivot_create(PivotSource::Name("Table9".into()), dest, false),
        Err(PivotError::InvalidSource)
    );
    assert!(wb.pivot_create(range(Rect::new(0, 0, 1, 0)), dest, false).is_ok());
    // A whole-column source stops at the data.
    let whole = PivotSource::parse("A:A", 0, &wb).unwrap();
    let id = wb
        .pivot_create(
            whole,
            PivotDestination::Existing {
                sheet: 0,
                row: 40,
                col: 0,
            },
            false,
        )
        .unwrap();
    assert_eq!(wb.pivot(id).unwrap().1.cache.rows, 1);
}

// ---- layout ------------------------------------------------------------------

#[test]
fn rows_and_one_value() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         West|320
         Grand Total|690",
    );
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.location, Some(Rect::new(2, 0, 5, 1)));
    assert_eq!(
        (
            p.report.first_header_row(),
            p.report.first_data_row(),
            p.report.first_data_col()
        ),
        (0, 1, 1)
    );
    // The values are plain numbers in the cells.
    assert_eq!(at(&wb, id, "B4"), Value::Number(370.0));
}

#[test]
fn rows_and_one_value_in_french() {
    let (mut wb, id) = sales(Locale::fr());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::fr());
        p.add_field(SALES_F, Area::Values, None, Locale::fr());
    });
    assert_report(
        &wb,
        id,
        "Étiquettes de lignes|Somme de Sales
         East|370
         West|320
         Total général|690",
    );
}

/// Region in the rows, Year in the columns, Sales in the values.
fn region_by_year(locale: &'static Locale) -> (Workbook, u32) {
    let (mut wb, id) = sales(locale);
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, locale);
        p.add_field(YEAR, Area::Columns, None, locale);
        p.add_field(SALES_F, Area::Values, None, locale);
    });
    (wb, id)
}

/// Region then Product in the rows, Sales in the values.
fn region_product(locale: &'static Locale) -> (Workbook, u32) {
    let (mut wb, id) = sales(locale);
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, locale);
        p.add_field(PRODUCT, Area::Rows, None, locale);
        p.add_field(SALES_F, Area::Values, None, locale);
    });
    (wb, id)
}

#[test]
fn rows_columns_and_one_value() {
    let (wb, id) = region_by_year(Locale::en());
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|250|120|370
         West|200|120|320
         Grand Total|450|240|690",
    );
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(
        (
            p.report.first_header_row(),
            p.report.first_data_row(),
            p.report.first_data_col()
        ),
        (1, 2, 1)
    );
    let (wb, id) = region_by_year(Locale::fr());
    assert_report(
        &wb,
        id,
        "Somme de Sales|Étiquettes de colonnes||
         Étiquettes de lignes|2023|2024|Total général
         East|250|120|370
         West|200|120|320
         Total général|450|240|690",
    );
}

#[test]
fn captions_in_every_language() {
    for (locale, expected) in [
        (
            Locale::es(),
            "Suma de Sales|Etiquetas de columna||
             Etiquetas de fila|2023|2024|Total general
             East|250|120|370
             West|200|120|320
             Total general|450|240|690",
        ),
        (
            Locale::de(),
            "Summe von Sales|Spaltenbeschriftungen||
             Zeilenbeschriftungen|2023|2024|Gesamtergebnis
             East|250|120|370
             West|200|120|320
             Gesamtergebnis|450|240|690",
        ),
        (
            Locale::it(),
            "Somma di Sales|Etichette di colonna||
             Etichette di riga|2023|2024|Totale complessivo
             East|250|120|370
             West|200|120|320
             Totale complessivo|450|240|690",
        ),
    ] {
        let (wb, id) = region_by_year(locale);
        assert_report(&wb, id, expected);
    }
}

#[test]
fn deleting_the_source_sheet_leaves_nothing_to_refresh_from() {
    let (mut wb, id) = region_by_year(Locale::en());
    let before = shown(&wb, id);
    assert!(wb.delete_sheet(0));
    wb.commit("delete");
    // The report stays as it was, on what is now the first sheet.
    let (si, p) = wb.pivot(id).unwrap();
    assert_eq!((si, &p.source), (0, &PivotSource::Invalid));
    assert_eq!(shown(&wb, id), before);
    assert_eq!(wb.pivot_refresh(id, false), Err(PivotError::InvalidSource));
    // A new sheet taking the name of the old one is not the source.
    let added = wb.add_sheet(0);
    assert!(wb.rename_sheet(added, "Sheet1"));
    wb.commit("add");
    assert_eq!(wb.pivot_refresh(id, false), Err(PivotError::InvalidSource));
    // Undo brings the sheet and the source back together.
    wb.undo();
    wb.undo();
    assert!(matches!(wb.pivot(id).unwrap().1.source, PivotSource::Range { .. }));
    assert_eq!(wb.pivot_refresh(id, false), Ok(()));
}

#[test]
fn compact_form_nests_the_labels_in_one_column() {
    let (wb, id) = region_product(Locale::en());
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         Apples|220
         Pears|150
         West|320
         Apples|200
         Pears|120
         Grand Total|690",
    );
    // The inner labels are indented, as Excel indents them, and all of
    // them leave room for the expand / collapse buttons of the regions.
    let style = |wb: &Workbook, name: &str| {
        let (r, c) = parse_cell_name(name).unwrap();
        wb.style_at(1, r, c).clone()
    };
    assert_eq!(style(&wb, "A4").indent, 1);
    assert_eq!(style(&wb, "A5").indent, 2);
    assert_eq!(wb.pivot_cell(1, 4, 0).unwrap().indent, 1);
    // "Grand Total" has no button to make room for.
    assert_eq!(style(&wb, "A10").indent, 0);

    // Without the buttons the labels move back to the edge.
    let mut wb = wb;
    update(&mut wb, id, |p| p.options.show_drill = false);
    assert_eq!(style(&wb, "A4").indent, 0);
    assert_eq!(style(&wb, "A5").indent, 1);
}

#[test]
fn labels_with_a_button_leave_room_for_it() {
    // Outline form: the regions carry the buttons, the products do not.
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Outline));
    let indent = |wb: &Workbook, name: &str| {
        let (r, c) = parse_cell_name(name).unwrap();
        wb.style_at(1, r, c).indent
    };
    assert_eq!(wb.pivot_cell(1, 3, 0).unwrap().button, Some(PivotButton::Collapse));
    assert_eq!(indent(&wb, "A4"), 1);
    assert_eq!(wb.pivot_cell(1, 4, 1).unwrap().button, None);
    assert_eq!(indent(&wb, "B5"), 0);

    // A single field has no button at all.
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    });
    assert_eq!(wb.pivot_cell(1, 3, 0).unwrap().button, None);
    assert_eq!(indent(&wb, "A4"), 0);

    // Column labels: the years above the products are numbers, aligned to
    // the right; the regions above the years are text and move over.
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Columns, None, Locale::en());
        p.add_field(YEAR, Area::Columns, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    });
    let cell = wb.pivot_cell(1, 3, 1).unwrap();
    assert_eq!((cell.part, cell.button), (PivotPart::ColumnLabel, Some(PivotButton::Collapse)));
    assert_eq!(indent(&wb, "B4"), 1);
    assert_eq!(wb.pivot_cell(1, 4, 1).unwrap().part, PivotPart::ColumnLabel);
    assert_eq!(indent(&wb, "B5"), 0);
}

#[test]
fn outline_form_gives_each_field_its_column() {
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Outline));
    assert_report(
        &wb,
        id,
        "Region|Product|Sum of Sales
         East||370
         |Apples|220
         |Pears|150
         West||320
         |Apples|200
         |Pears|120
         Grand Total||690",
    );
    update(&mut wb, id, |p| p.set_repeat_labels(true));
    assert_report(
        &wb,
        id,
        "Region|Product|Sum of Sales
         East||370
         East|Apples|220
         East|Pears|150
         West||320
         West|Apples|200
         West|Pears|120
         Grand Total||690",
    );
}

#[test]
fn tabular_form_starts_the_detail_on_the_item_row() {
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Tabular));
    assert_report(
        &wb,
        id,
        "Region|Product|Sum of Sales
         East|Apples|220
         |Pears|150
         East Total||370
         West|Apples|200
         |Pears|120
         West Total||320
         Grand Total||690",
    );
    update(&mut wb, id, |p| p.set_repeat_labels(true));
    assert_report(
        &wb,
        id,
        "Region|Product|Sum of Sales
         East|Apples|220
         East|Pears|150
         East Total||370
         West|Apples|200
         West|Pears|120
         West Total||320
         Grand Total||690",
    );
    let (mut wb, id) = region_product(Locale::fr());
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Tabular));
    assert_report(
        &wb,
        id,
        "Region|Product|Somme de Sales
         East|Apples|220
         |Pears|150
         Total East||370
         West|Apples|200
         |Pears|120
         Total West||320
         Total général||690",
    );
}

#[test]
fn subtotals_off_at_the_bottom_and_per_field() {
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_subtotals(true, false));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|
         Apples|220
         Pears|150
         East Total|370
         West|
         Apples|200
         Pears|120
         West Total|320
         Grand Total|690",
    );
    update(&mut wb, id, |p| p.set_subtotals(false, true));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|
         Apples|220
         Pears|150
         West|
         Apples|200
         Pears|120
         Grand Total|690",
    );
    // Custom subtotals: one row per function, always below the detail.
    update(&mut wb, id, |p| {
        p.fields[REGION].subtotals = Subtotals::Custom(vec![Aggregation::Sum, Aggregation::Max])
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|
         Apples|220
         Pears|150
         East Sum|370
         East Max|150
         West|
         Apples|200
         Pears|120
         West Sum|320
         West Max|200
         Grand Total|690",
    );
}

#[test]
fn blank_lines_and_grand_totals() {
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_blank_lines(true));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         Apples|220
         Pears|150
         |
         West|320
         Apples|200
         Pears|120
         |
         Grand Total|690",
    );
    assert_eq!(wb.pivot_cell(1, 6, 0).unwrap().row_kind, Some(ReportLineKind::Blank));
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| p.options.row_grand_totals = false);
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels|
         Row Labels|2023|2024
         East|250|120
         West|200|120
         Grand Total|450|240",
    );
    update(&mut wb, id, |p| {
        p.options.row_grand_totals = true;
        p.options.col_grand_totals = false;
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|250|120|370
         West|200|120|320",
    );
}

#[test]
fn several_values_go_to_the_columns_and_can_move_to_the_rows() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
        p.add_field(UNITS, Area::Values, None, Locale::en());
    });
    assert_eq!(wb.pivot(id).unwrap().1.col_fields, [AxisField::Values]);
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales|Sum of Units
         East|370|27
         West|320|29
         Grand Total|690|56",
    );
    // Drag "Σ Values" from the columns to the end of the rows.
    update(&mut wb, id, |p| {
        assert!(p.move_entry(Area::Columns, 0, Area::Rows, 1, Locale::en()))
    });
    assert_report(
        &wb,
        id,
        "Row Labels|
         East|
         Sum of Sales|370
         Sum of Units|27
         West|
         Sum of Sales|320
         Sum of Units|29
         Total Sum of Sales|690
         Total Sum of Units|56",
    );
    // Removing a value field removes "Σ Values" with it.
    update(&mut wb, id, |p| p.remove_data_field(1));
    assert_eq!(wb.pivot(id).unwrap().1.row_fields, [AxisField::Field(REGION)]);
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         West|320
         Grand Total|690",
    );
}

#[test]
fn several_values_under_a_column_field() {
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| p.add_field(UNITS, Area::Values, None, Locale::en()));
    assert_report(
        &wb,
        id,
        "|Column Labels|||||
         |2023||2024||Total Sum of Sales|Total Sum of Units
         Row Labels|Sum of Sales|Sum of Units|Sum of Sales|Sum of Units||
         East|250|15|120|12|370|27
         West|200|20|120|9|320|29
         Grand Total|450|35|240|21|690|56",
    );
    let (mut wb, id) = region_by_year(Locale::fr());
    update(&mut wb, id, |p| p.add_field(UNITS, Area::Values, None, Locale::fr()));
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 5, 3, 6)),
        "Total Somme de Sales|Total Somme de Units"
    );
}

#[test]
fn only_columns_or_only_values() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| p.add_field(SALES_F, Area::Values, None, Locale::en()));
    assert_report(
        &wb,
        id,
        "Sum of Sales
         690",
    );
    update(&mut wb, id, |p| p.add_field(YEAR, Area::Columns, None, Locale::en()));
    assert_report(
        &wb,
        id,
        "|Column Labels||
         |2023|2024|Grand Total
         Sum of Sales|450|240|690",
    );
    update(&mut wb, id, |p| p.remove_data_field(0));
    assert_report(
        &wb,
        id,
        "|Column Labels||
         |2023|2024|Grand Total",
    );
    // Rows without values: the list of the items.
    update(&mut wb, id, |p| {
        p.remove_field(YEAR);
        p.add_field(REGION, Area::Rows, None, Locale::en());
    });
    assert_report(
        &wb,
        id,
        "Row Labels
         East
         West
         Grand Total",
    );
}

// ---- values ------------------------------------------------------------------

/// Group, N: East has 2 and 4, West has 4 and 5; North has a text and an
/// empty cell.
const NUMBERS: &[[&str; 2]] = &[
    ["Group", "N"],
    ["East", "2"],
    ["East", "4"],
    ["West", "4"],
    ["West", "5"],
    ["North", "n/a"],
    ["North", ""],
];

/// One report row per group for a summary function: East, West, North
/// and the grand total.
fn summary(function: Aggregation) -> [Value; 4] {
    let (mut wb, id) = pivot_on(NUMBERS, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
        p.set_function(0, function, Locale::en());
    });
    // Items are in ascending order: East, North, West.
    [
        at(&wb, id, "B4"),
        at(&wb, id, "B6"),
        at(&wb, id, "B5"),
        at(&wb, id, "B7"),
    ]
}

#[test]
fn every_summary_function() {
    let n = Value::Number;
    let div0 = Value::Error(ErrorKind::Div0);
    // North holds no number: it sums to 0 and has no average.
    assert_eq!(summary(Aggregation::Sum), [n(6.0), n(9.0), n(0.0), n(15.0)]);
    assert_eq!(summary(Aggregation::Count), [n(2.0), n(2.0), n(1.0), n(5.0)]);
    assert_eq!(summary(Aggregation::CountNums), [n(2.0), n(2.0), n(0.0), n(4.0)]);
    assert_eq!(summary(Aggregation::Average), [n(3.0), n(4.5), div0.clone(), n(3.75)]);
    assert_eq!(summary(Aggregation::Max), [n(4.0), n(5.0), n(0.0), n(5.0)]);
    assert_eq!(summary(Aggregation::Min), [n(2.0), n(4.0), n(0.0), n(2.0)]);
    assert_eq!(summary(Aggregation::Product), [n(8.0), n(20.0), n(0.0), n(160.0)]);
    // 2, 4, 4, 5: mean 3.75, squared deviations 4.75.
    let [east, west, north, all] = summary(Aggregation::Var);
    assert_close(east, 2.0);
    assert_close(west, 0.5);
    assert_eq!(north, div0);
    assert_close(all, 4.75 / 3.0);
    let [east, west, north, all] = summary(Aggregation::Varp);
    assert_close(east, 1.0);
    assert_close(west, 0.25);
    assert_eq!(north, div0);
    assert_close(all, 4.75 / 4.0);
    let [east, west, _, all] = summary(Aggregation::StdDev);
    assert_close(east, 2f64.sqrt());
    assert_close(west, 0.5f64.sqrt());
    assert_close(all, (4.75f64 / 3.0).sqrt());
    let [east, west, _, all] = summary(Aggregation::StdDevp);
    assert_close(east, 1.0);
    assert_close(west, 0.5);
    assert_close(all, (4.75f64 / 4.0).sqrt());
}

#[test]
fn value_fields_are_named_after_their_function() {
    let en = Locale::en();
    let (mut wb, id) = pivot_on(NUMBERS, en);
    // A column with text or empty cells is counted, not summed.
    update(&mut wb, id, |p| p.add_field(1, Area::Values, None, en));
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(
        (p.data_fields[0].function, p.data_fields[0].name.as_str()),
        (Aggregation::Count, "Count of N")
    );
    // The same field twice: the second name is numbered.
    update(&mut wb, id, |p| p.add_field(1, Area::Values, None, en));
    assert_eq!(wb.pivot(id).unwrap().1.data_fields[1].name, "Count of N2");
    // The name follows the function until it is customised.
    update(&mut wb, id, |p| p.set_function(1, Aggregation::Average, en));
    assert_eq!(wb.pivot(id).unwrap().1.data_fields[1].name, "Average of N");
    update(&mut wb, id, |p| {
        assert!(!p.set_data_name(1, "N"));
        assert!(!p.set_data_name(1, "count of n"));
        assert!(p.set_data_name(1, "Mean"));
        p.set_function(1, Aggregation::Max, en);
    });
    assert_eq!(wb.pivot(id).unwrap().1.data_fields[1].name, "Mean");
    let name = |f: Aggregation, l: &Locale| data_field_name(f, "N", l);
    let all: Vec<String> = Aggregation::ALL.iter().map(|f| name(*f, en)).collect();
    assert_eq!(
        all,
        [
            "Sum of N",
            "Count of N",
            "Average of N",
            "Max of N",
            "Min of N",
            "Product of N",
            "Count of N",
            "StdDev of N",
            "StdDevp of N",
            "Var of N",
            "Varp of N"
        ]
    );
    let fr: Vec<String> = Aggregation::ALL.iter().map(|f| name(*f, Locale::fr())).collect();
    assert_eq!(
        fr,
        [
            "Somme de N",
            "Nombre de N",
            "Moyenne de N",
            "Max. de N",
            "Min. de N",
            "Produit de N",
            "Nombre de N",
            "Écartype de N",
            "Écartypep de N",
            "Var de N",
            "Varp de N"
        ]
    );
    assert_eq!(name(Aggregation::Sum, Locale::es()), "Suma de N");
    assert_eq!(name(Aggregation::Count, Locale::de()), "Anzahl von N");
    assert_eq!(name(Aggregation::Average, Locale::it()), "Media di N");
}

/// The report of Region by Year with Sales shown as `show_as`.
fn shown_as(show_as: ShowAs) -> (Workbook, u32) {
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| p.data_fields[0].show_as = show_as);
    (wb, id)
}

#[test]
fn show_values_as_percentages_of_totals() {
    let (wb, id) = shown_as(ShowAs::PercentOfTotal);
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|36.23%|17.39%|53.62%
         West|28.99%|17.39%|46.38%
         Grand Total|65.22%|34.78%|100.00%",
    );
    assert_close(at(&wb, id, "B5"), 250.0 / 690.0);
    let (wb, id) = shown_as(ShowAs::PercentOfColumn);
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|55.56%|50.00%|53.62%
         West|44.44%|50.00%|46.38%
         Grand Total|100.00%|100.00%|100.00%",
    );
    let (wb, id) = shown_as(ShowAs::PercentOfRow);
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|67.57%|32.43%|100.00%
         West|62.50%|37.50%|100.00%
         Grand Total|65.22%|34.78%|100.00%",
    );
    // 250 x 690 / (370 x 450), and so on.
    let (wb, id) = shown_as(ShowAs::Index);
    assert_close(at(&wb, id, "B5"), 250.0 * 690.0 / (370.0 * 450.0));
    assert_close(at(&wb, id, "C6"), 120.0 * 690.0 / (320.0 * 240.0));
    assert_close(at(&wb, id, "D7"), 1.0);
    // In French the percentages are written the French way.
    let (mut wb, id) = region_by_year(Locale::fr());
    update(&mut wb, id, |p| p.data_fields[0].show_as = ShowAs::PercentOfTotal);
    assert_eq!(cells(&wb, 1, Rect::new(4, 1, 4, 3)), "36,23%|17,39%|53,62%");
}

#[test]
fn show_values_as_compared_with_a_base_item() {
    let east = p_item(REGION, "East");
    let (wb, id) = shown_as(ShowAs::PercentOf {
        field: REGION,
        item: BaseItem::Item(east),
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|100.00%|100.00%|100.00%
         West|80.00%|100.00%|86.49%
         Grand Total|||",
    );
    let (wb, id) = shown_as(ShowAs::Difference {
        field: YEAR,
        item: BaseItem::Previous,
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East||-130|
         West||-80|
         Grand Total||-210|",
    );
    let (wb, id) = shown_as(ShowAs::PercentDifference {
        field: YEAR,
        item: BaseItem::Previous,
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East||-52.00%|
         West||-40.00%|
         Grand Total||-46.67%|",
    );
    let (wb, id) = shown_as(ShowAs::Difference {
        field: REGION,
        item: BaseItem::Next,
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|50|0|50
         West|||
         Grand Total|||",
    );
}

/// Index of an item of the sales data by its label.
fn p_item(field: usize, label: &str) -> u32 {
    let (wb, id) = sales(Locale::en());
    let p = wb.pivot(id).unwrap().1;
    (0..p.cache.items(field).len() as u32)
        .find(|x| p.item_text(field, *x, Locale::en()) == label)
        .unwrap()
}

#[test]
fn show_values_as_running_totals_and_ranks() {
    let (wb, id) = shown_as(ShowAs::RunningTotal { field: YEAR });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|250|370|
         West|200|320|
         Grand Total|450|690|",
    );
    let (wb, id) = shown_as(ShowAs::PercentRunningTotal { field: YEAR });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|67.57%|100.00%|
         West|62.50%|100.00%|
         Grand Total|65.22%|100.00%|",
    );
    // Equal values share their rank.
    let (wb, id) = shown_as(ShowAs::RankDescending { field: REGION });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|1|1|1
         West|2|1|2
         Grand Total|||",
    );
    let (wb, id) = shown_as(ShowAs::RankAscending { field: REGION });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|2|1|2
         West|1|1|1
         Grand Total|||",
    );
}

#[test]
fn show_values_as_percentages_of_parents() {
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.data_fields[0].show_as = ShowAs::PercentOfParentRow);
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|53.62%
         Apples|59.46%
         Pears|40.54%
         West|46.38%
         Apples|62.50%
         Pears|37.50%
         Grand Total|100.00%",
    );
    update(&mut wb, id, |p| {
        p.data_fields[0].show_as = ShowAs::PercentOfParent { field: REGION }
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|100.00%
         Apples|59.46%
         Pears|40.54%
         West|100.00%
         Apples|62.50%
         Pears|37.50%
         Grand Total|",
    );
    let (wb, id) = shown_as(ShowAs::PercentOfParentColumn);
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|67.57%|32.43%|100.00%
         West|62.50%|37.50%|100.00%
         Grand Total|65.22%|34.78%|100.00%",
    );
    // A calculation whose base field leaves the report is switched off.
    let (mut wb, id) = shown_as(ShowAs::RunningTotal { field: YEAR });
    update(&mut wb, id, |p| p.remove_field(YEAR));
    assert_eq!(wb.pivot(id).unwrap().1.data_fields[0].show_as, ShowAs::Normal);
}

#[test]
fn number_formats_of_values_and_labels() {
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| p.data_fields[0].num_fmt = Some(Arc::from("#,##0.00")));
    assert_eq!(cells(&wb, 1, Rect::new(6, 1, 6, 3)), "450.00|240.00|690.00");
    // The labels and the captions stay as they are.
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 3)), "Row Labels|2023|2024|Grand Total");
    let (mut wb, id) = region_by_year(Locale::fr());
    update(&mut wb, id, |p| p.data_fields[0].num_fmt = Some(Arc::from("#,##0.00")));
    assert_eq!(cells(&wb, 1, Rect::new(6, 1, 6, 3)), "450,00|240,00|690,00");
}

#[test]
fn empty_cells_and_errors_can_be_replaced() {
    // West sold nothing in 2022; the East record of 2022 is an error.
    let rows: &[[&str; 3]] = &[
        ["Region", "Year", "Sales"],
        ["East", "2022", "=1/0"],
        ["East", "2023", "5"],
        ["West", "2023", "7"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Columns, None, Locale::en());
        p.add_field(2, Area::Values, None, Locale::en());
    });
    // An error in the source: the field is counted by default.
    assert_eq!(wb.pivot(id).unwrap().1.data_fields[0].function, Aggregation::Count);
    update(&mut wb, id, |p| p.set_function(0, Aggregation::Sum, Locale::en()));
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2022|2023|Grand Total
         East|#DIV/0!|5|#DIV/0!
         West||7|7
         Grand Total|#DIV/0!|12|#DIV/0!",
    );
    update(&mut wb, id, |p| {
        p.options.missing_caption = "-".to_string();
        p.options.show_error = true;
        p.options.error_caption = "n/a".to_string();
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2022|2023|Grand Total
         East|n/a|5|n/a
         West|-|7|7
         Grand Total|n/a|12|n/a",
    );
    // "For empty cells show" unchecked: zeros.
    update(&mut wb, id, |p| p.options.show_missing = false);
    assert_eq!(at(&wb, id, "B6"), Value::Number(0.0));
}

// ---- items -------------------------------------------------------------------

#[test]
fn hidden_items_leave_the_totals() {
    let west = p_item(REGION, "West");
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| p.set_item_hidden(REGION, west, true));
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|250|120|370
         Grand Total|250|120|370",
    );
    assert!(wb.pivot(id).unwrap().1.is_filtered(REGION));
    update(&mut wb, id, |p| p.clear_filters(REGION));
    assert_eq!(at(&wb, id, "D7"), Value::Number(690.0));
    update(&mut wb, id, |p| p.set_visible_items(REGION, &[west]));
    assert_eq!(
        cells(&wb, 1, Rect::new(4, 0, 5, 3)),
        "West|200|120|320\nGrand Total|200|120|320"
    );
}

#[test]
fn filter_fields_sit_above_the_report() {
    let y2023 = p_item(YEAR, "2023");
    let y2024 = p_item(YEAR, "2024");
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
        p.add_field(YEAR, Area::Filters, None, Locale::en());
    });
    // The report stays in A3; the filter takes A1:B1, row 2 is empty.
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.location, Some(Rect::new(2, 0, 5, 1)));
    assert_eq!(p.page_location, Some(Rect::new(0, 0, 0, 1)));
    assert_eq!(
        cells(&wb, 1, Rect::new(0, 0, 2, 1)),
        "Year|(All)\n|\nRow Labels|Sum of Sales"
    );
    update(&mut wb, id, |p| p.set_page_item(YEAR, Some(y2023)));
    assert_eq!(cells(&wb, 1, Rect::new(0, 0, 0, 1)), "Year|2023");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|250
         West|200
         Grand Total|450",
    );
    // Several items: none of them hidden means all.
    update(&mut wb, id, |p| p.set_visible_items(YEAR, &[y2024]));
    assert_eq!(cells(&wb, 1, Rect::new(0, 0, 0, 1)), "Year|(Multiple Items)");
    assert_eq!(at(&wb, id, "B6"), Value::Number(240.0));
    // A second filter needs a row more: the report moves down.
    update(&mut wb, id, |p| {
        p.clear_filters(YEAR);
        p.add_field(PRODUCT, Area::Filters, None, Locale::en());
    });
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.page_location, Some(Rect::new(0, 0, 1, 1)));
    assert_eq!(p.location, Some(Rect::new(3, 0, 6, 1)));
    assert_eq!(
        cells(&wb, 1, Rect::new(0, 0, 3, 1)),
        "Year|(All)\nProduct|(All)\n|\nRow Labels|Sum of Sales"
    );
    // Over, then down, two per row: a column is left empty between them.
    // The report stays where it is; the filters keep one empty row above it.
    update(&mut wb, id, |p| {
        p.options.page_over_then_down = true;
        p.options.page_wrap = 2;
    });
    assert_eq!(cells(&wb, 1, Rect::new(0, 0, 1, 4)), "||||\nYear|(All)||Product|(All)");
    assert!(wb.pivot_cell(1, 0, 0).is_none());
    let cell = wb.pivot_cell(1, 1, 4).unwrap();
    assert_eq!((cell.part, cell.field), (PivotPart::FilterValue, Some(PRODUCT)));
    assert_eq!(cell.button, Some(PivotButton::Filter { filtered: false }));
    assert_eq!(wb.pivot_cell(1, 1, 3).unwrap().part, PivotPart::FilterLabel);
    assert_eq!(wb.pivot_cell(1, 1, 2).unwrap().part, PivotPart::Blank);
    // In French.
    let (mut wb, id) = sales(Locale::fr());
    update(&mut wb, id, |p| {
        p.add_field(YEAR, Area::Filters, None, Locale::fr());
        p.add_field(SALES_F, Area::Values, None, Locale::fr());
    });
    assert_eq!(cells(&wb, 1, Rect::new(0, 0, 0, 1)), "Year|(Tous)");
    update(&mut wb, id, |p| p.set_item_hidden(YEAR, y2023, true));
    assert_eq!(cells(&wb, 1, Rect::new(0, 0, 0, 1)), "Year|(Plusieurs éléments)");
}

#[test]
fn label_filters() {
    let (mut wb, id) = region_product(Locale::en());
    let mut rows = |filter: LabelFilter| {
        update(&mut wb, id, |p| p.set_label_filter(PRODUCT, Some(filter)));
        shown(&wb, id)
            .lines()
            .skip(1)
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let apples = "East|220 Apples|220 West|200 Apples|200 Grand Total|420";
    let pears = "East|150 Pears|150 West|120 Pears|120 Grand Total|270";
    assert_eq!(rows(LabelFilter::Equals("apples".into())), apples);
    assert_eq!(rows(LabelFilter::Equals("p*".into())), pears);
    assert_eq!(rows(LabelFilter::NotEquals("Apples".into())), pears);
    assert_eq!(rows(LabelFilter::BeginsWith("ap".into())), apples);
    assert_eq!(rows(LabelFilter::NotBeginsWith("ap".into())), pears);
    assert_eq!(rows(LabelFilter::EndsWith("RS".into())), pears);
    assert_eq!(rows(LabelFilter::NotEndsWith("rs".into())), apples);
    assert_eq!(rows(LabelFilter::Contains("ppl".into())), apples);
    assert_eq!(rows(LabelFilter::NotContains("ppl".into())), pears);
    assert_eq!(rows(LabelFilter::GreaterThan("B".into())), pears);
    assert_eq!(rows(LabelFilter::LessThan("B".into())), apples);
    assert_eq!(rows(LabelFilter::Between("A".into(), "B".into())), apples);
    assert_eq!(rows(LabelFilter::NotBetween("A".into(), "B".into())), pears);
    // Numbers compare as numbers.
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| {
        p.set_label_filter(YEAR, Some(LabelFilter::GreaterOrEqual("2024".into())))
    });
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 2)), "Row Labels|2024|Grand Total");
    update(&mut wb, id, |p| {
        p.set_label_filter(YEAR, Some(LabelFilter::LessOrEqual("2023".into())))
    });
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 2)), "Row Labels|2023|Grand Total");
    update(&mut wb, id, |p| p.set_label_filter(YEAR, None));
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 3)), "Row Labels|2023|2024|Grand Total");
}

#[test]
fn value_filters_and_top_10() {
    let (mut wb, id) = region_product(Locale::en());
    // Products are judged within their region: East Apples 220, East
    // Pears 150, West Apples 200, West Pears 120.
    let test = |test: ValueFilter| FilterKind::Value { data_field: 0, test };
    update(&mut wb, id, |p| {
        p.set_value_filter(PRODUCT, Some(test(ValueFilter::GreaterThan(150.0))))
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|220
         Apples|220
         West|200
         Apples|200
         Grand Total|420",
    );
    update(&mut wb, id, |p| {
        p.set_value_filter(PRODUCT, Some(test(ValueFilter::Between(120.0, 150.0))))
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|150
         Pears|150
         West|120
         Pears|120
         Grand Total|270",
    );
    // Regions are judged on their totals: East 370, West 320.
    update(&mut wb, id, |p| {
        p.set_value_filter(PRODUCT, None);
        p.set_value_filter(REGION, Some(test(ValueFilter::LessThan(350.0))));
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         West|320
         Apples|200
         Pears|120
         Grand Total|320",
    );
    let top = |top: bool, count: f64, by: Top10By| FilterKind::Top10 {
        data_field: 0,
        top,
        count,
        by,
    };
    update(&mut wb, id, |p| {
        p.set_value_filter(REGION, Some(top(true, 1.0, Top10By::Items)))
    });
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 1)), "East|370");
    assert_eq!(at(&wb, id, "B7"), Value::Number(370.0));
    update(&mut wb, id, |p| {
        p.set_value_filter(REGION, Some(top(false, 1.0, Top10By::Items)))
    });
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 1)), "West|320");
    // The top items making up half of 690: East alone (370).
    update(&mut wb, id, |p| {
        p.set_value_filter(REGION, Some(top(true, 50.0, Top10By::Percent)))
    });
    assert_eq!(at(&wb, id, "B7"), Value::Number(370.0));
    // The top items summing to 400: East and West.
    update(&mut wb, id, |p| {
        p.set_value_filter(REGION, Some(top(true, 400.0, Top10By::Sum)))
    });
    assert_eq!(at(&wb, id, "B10"), Value::Number(690.0));
    // The top product of each region.
    update(&mut wb, id, |p| {
        p.set_value_filter(REGION, None);
        p.set_value_filter(PRODUCT, Some(top(true, 1.0, Top10By::Items)));
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|220
         Apples|220
         West|200
         Apples|200
         Grand Total|420",
    );
    assert!(wb.pivot(id).unwrap().1.is_filtered(PRODUCT));
}

#[test]
fn sorting_by_label_by_value_and_by_hand() {
    let (mut wb, id) = region_product(Locale::en());
    let order = |wb: &Workbook| {
        shown(wb, id)
            .lines()
            .skip(1)
            .map(|l| l.split('|').next().unwrap().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(order(&wb), "East Apples Pears West Apples Pears Grand Total");
    update(&mut wb, id, |p| p.sort_field(REGION, SortOrder::Descending, None));
    assert_eq!(order(&wb), "West Apples Pears East Apples Pears Grand Total");
    // By value, within each region: East has Pears 150 < Apples 220, West
    // has Pears 120 < Apples 200.
    update(&mut wb, id, |p| p.sort_field(PRODUCT, SortOrder::Ascending, Some(0)));
    assert_eq!(order(&wb), "West Pears Apples East Pears Apples Grand Total");
    // Regions by value, smallest first: West 320, East 370.
    update(&mut wb, id, |p| {
        p.sort_field(REGION, SortOrder::Ascending, Some(0));
        p.sort_field(PRODUCT, SortOrder::Descending, Some(0));
    });
    assert_eq!(order(&wb), "West Apples Pears East Apples Pears Grand Total");
    // By hand: Pears dragged before Apples.
    let pears = p_item(PRODUCT, "Pears");
    update(&mut wb, id, |p| p.move_item(PRODUCT, pears, 0));
    assert_eq!(wb.pivot(id).unwrap().1.fields[PRODUCT].sort.order, SortOrder::Manual);
    assert_eq!(order(&wb), "West Pears Apples East Pears Apples Grand Total");
}

#[test]
fn text_sorts_without_case_and_numbers_come_first() {
    let rows: &[[&str; 2]] = &[
        ["Key", "N"],
        ["b", "1"],
        ["10", "1"],
        ["A", "1"],
        ["9", "1"],
        ["a", "1"],
        ["TRUE", "1"],
        ["", "1"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    // "A" and "a" are one item; the empty cell is "(blank)" and comes last.
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         9|1
         10|1
         A|2
         b|1
         TRUE|1
         (blank)|1
         Grand Total|7",
    );
    let (mut wb, id) = pivot_on(rows, Locale::fr());
    update(&mut wb, id, |p| p.add_field(0, Area::Rows, None, Locale::fr()));
    assert_eq!(cells(&wb, 1, Rect::new(7, 0, 8, 0)), "VRAI\n(vide)");
}

#[test]
fn collapsing_and_expanding() {
    let east = p_item(REGION, "East");
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| p.set_collapsed(REGION, east, true));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         West|320
         Apples|200
         Pears|120
         Grand Total|690",
    );
    let cell = |wb: &Workbook, name: &str| {
        let (r, c) = parse_cell_name(name).unwrap();
        wb.pivot_cell(1, r, c).unwrap()
    };
    assert_eq!(cell(&wb, "A4").button, Some(PivotButton::Expand));
    assert_eq!(cell(&wb, "A5").button, Some(PivotButton::Collapse));
    assert_eq!(cell(&wb, "A6").button, None);
    update(&mut wb, id, |p| p.set_field_collapsed(REGION, true));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         West|320
         Grand Total|690",
    );
    // Tabular form: the collapsed item keeps its row, without detail.
    update(&mut wb, id, |p| {
        p.set_field_collapsed(REGION, false);
        p.set_collapsed(REGION, east, true);
        p.set_report_layout(FieldLayout::Tabular);
    });
    assert_report(
        &wb,
        id,
        "Region|Product|Sum of Sales
         East||370
         West|Apples|200
         |Pears|120
         West Total||320
         Grand Total||690",
    );
    // Without the buttons.
    update(&mut wb, id, |p| p.options.show_drill = false);
    assert_eq!(cell(&wb, "A4").button, None);
}

#[test]
fn items_without_data() {
    // Pears were never sold in the North.
    let rows: &[[&str; 3]] = &[
        ["Region", "Product", "Sales"],
        ["East", "Apples", "1"],
        ["East", "Pears", "2"],
        ["North", "Apples", "4"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Rows, None, Locale::en());
        p.add_field(2, Area::Values, None, Locale::en());
        p.fields[1].show_all = true;
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|3
         Apples|1
         Pears|2
         North|4
         Apples|4
         Pears|
         Grand Total|7",
    );
}

// ---- grouping ----------------------------------------------------------------

/// Dates over two years with an amount each:
/// 2023: 15 Jan 10, 20 Feb 20, 5 May 30; 2024: 10 Jan 40, 25 Nov 50.
const DATED: &[[&str; 2]] = &[
    ["Date", "Amount"],
    ["1/15/2023", "10"],
    ["2/20/2023", "20"],
    ["5/5/2023", "30"],
    ["1/10/2024", "40"],
    ["11/25/2024", "50"],
];

#[test]
fn a_date_field_is_grouped_when_it_enters_the_rows() {
    let (mut wb, id) = pivot_on(DATED, Locale::en());
    assert!(wb.pivot(id).unwrap().1.cache.is_date(0));
    // Dates go to the rows when their box is checked.
    assert_eq!(wb.pivot(id).unwrap().1.default_area(0), Area::Rows);
    wb.pivot_add_field(id, 0, Area::Rows, None, false).unwrap();
    wb.pivot_update(id, false, |p| p.add_field(1, Area::Values, None, Locale::en()))
        .unwrap();
    wb.commit("pivot");
    let p = wb.pivot(id).unwrap().1;
    let names: Vec<&str> = p.cache.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["Date", "Amount", "Years (Date)", "Quarters (Date)", "Months (Date)"]
    );
    assert_eq!(
        p.row_fields,
        [
            AxisField::Field(2),
            AxisField::Field(3),
            AxisField::Field(4),
            AxisField::Field(0)
        ]
    );
    assert!(!p.cache.fields[2].is_database());
    // Collapsed to the years, as Excel shows it at first.
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Amount
         2023|60
         2024|90
         Grand Total|150",
    );
    // Expanding the years shows the quarters, then the months, then the days.
    update(&mut wb, id, |p| p.set_field_collapsed(2, false));
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Amount
         2023|60
         Qtr1|30
         Qtr2|30
         2024|90
         Qtr1|40
         Qtr4|50
         Grand Total|150",
    );
    update(&mut wb, id, |p| {
        p.set_field_collapsed(3, false);
        p.set_field_collapsed(4, false);
        p.remove_field(3);
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Amount
         2023|60
         Jan|10
         1/15/2023|10
         Feb|20
         2/20/2023|20
         May|30
         5/5/2023|30
         2024|90
         Jan|40
         1/10/2024|40
         Nov|50
         11/25/2024|50
         Grand Total|150",
    );
    // The date labels are dates in the cells, shown in the source's format.
    assert_eq!(
        at(&wb, id, "A6"),
        Value::Number(crate::datetime::date_to_serial(2023, 1, 15).unwrap())
    );
}

#[test]
fn date_groups_in_french() {
    let rows: &[[&str; 2]] = &[
        ["Date", "Montant"],
        ["15/01/2023", "10"],
        ["20/02/2023", "20"],
        ["10/08/2024", "40"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::fr());
    wb.pivot_add_field(id, 0, Area::Rows, None, false).unwrap();
    wb.pivot_update(id, false, |p| {
        p.add_field(1, Area::Values, None, Locale::fr());
        for f in 2..5 {
            p.set_field_collapsed(f, false);
        }
        p.remove_field(0);
    })
    .unwrap();
    let p = wb.pivot(id).unwrap().1;
    let names: Vec<&str> = p.cache.fields.iter().skip(2).map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Années (Date)", "Trimestres (Date)", "Mois (Date)"]);
    assert_report(
        &wb,
        id,
        "Étiquettes de lignes|Somme de Montant
         2023|30
         Trimestre1|30
         janv|10
         févr|20
         2024|40
         Trimestre3|40
         août|40
         Total général|70",
    );
}

#[test]
fn grouping_dates_by_hand() {
    let (mut wb, id) = pivot_on(DATED, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Columns, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    // Months only: the months of both years fall together.
    wb.pivot_group_dates(id, 0, &[GroupBy::Months], None, None, 1, false)
        .unwrap();
    wb.commit("group");
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.col_fields, [AxisField::Field(2)]);
    assert_eq!(p.cache.fields[2].name, "Months (Date)");
    assert_report(
        &wb,
        id,
        "|Column Labels||||
         |Jan|Feb|May|Nov|Grand Total
         Sum of Amount|50|20|30|50|150",
    );
    // Regrouping replaces the months by years and quarters, in place.
    wb.pivot_group_dates(id, 2, &[GroupBy::Quarters, GroupBy::Years], None, None, 1, false)
        .unwrap();
    wb.commit("group");
    let p = wb.pivot(id).unwrap().1;
    let names: Vec<&str> = p.cache.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["Date", "Amount", "Years (Date)", "Quarters (Date)"]);
    assert_eq!(p.col_fields, [AxisField::Field(2), AxisField::Field(3)]);
    assert_report(
        &wb,
        id,
        "|Column Labels||||||
         |2023||2023 Total|2024||2024 Total|Grand Total
         |Qtr1|Qtr2||Qtr1|Qtr4||
         Sum of Amount|30|30|60|40|50|90|150",
    );
    // Bounds: what lies outside goes to "<" and ">" groups.
    let start = crate::datetime::date_to_serial(2023, 2, 1).unwrap();
    let end = crate::datetime::date_to_serial(2023, 12, 31).unwrap();
    wb.pivot_group_dates(id, 0, &[GroupBy::Months], Some(start), Some(end), 1, false)
        .unwrap();
    wb.commit("group");
    assert_report(
        &wb,
        id,
        "|Column Labels||||
         |<2/1/2023|Feb|May|>12/31/2023|Grand Total
         Sum of Amount|10|20|30|90|150",
    );
    // Weeks: seven days from the first date.
    wb.pivot_group_dates(
        id,
        0,
        &[GroupBy::Days],
        None,
        Some(crate::datetime::date_to_serial(2023, 1, 28).unwrap()),
        7,
        false,
    )
    .unwrap();
    wb.commit("group");
    assert_report(
        &wb,
        id,
        "|Column Labels||
         |1/15/2023 - 1/21/2023|>1/28/2023|Grand Total
         Sum of Amount|10|140|150",
    );
    // Ungrouping brings the dates back where the periods were.
    wb.pivot_ungroup(id, 2, None, false).unwrap();
    wb.commit("ungroup");
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.cache.fields.len(), 2);
    assert_eq!(p.col_fields, [AxisField::Field(0)]);
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 1, 3, 6)),
        "1/15/2023|2/20/2023|5/5/2023|1/10/2024|11/25/2024|Grand Total"
    );
    // Text cannot be grouped by date.
    assert_eq!(wb.pivot_ungroup(id, 0, None, false), Err(PivotError::CannotUngroup));
    let (mut wb, id) = sales(Locale::en());
    assert_eq!(
        wb.pivot_group_dates(id, REGION, &[GroupBy::Months], None, None, 1, false),
        Err(PivotError::CannotGroup)
    );
}

#[test]
fn days_hours_and_minutes() {
    let rows: &[[&str; 2]] = &[
        ["When", "N"],
        ["1/15/2023 9:30", "1"],
        ["1/15/2023 14:05", "2"],
        ["2/1/2023 9:45", "4"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    wb.pivot_group_dates(
        id,
        0,
        &[GroupBy::Days, GroupBy::Hours, GroupBy::Minutes],
        None,
        None,
        1,
        false,
    )
    .unwrap();
    wb.commit("group");
    let names: Vec<&str> = wb
        .pivot(id)
        .unwrap()
        .1
        .cache
        .fields
        .iter()
        .skip(2)
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, ["Days (When)", "Hours (When)", "Minutes (When)"]);
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         15-Jan|3
         9 AM|1
         :30|1
         2 PM|2
         :05|2
         1-Feb|4
         9 AM|4
         :45|4
         Grand Total|7",
    );
    let rows: &[[&str; 2]] = &[["Quand", "N"], ["15/01/2023 09:30", "1"], ["15/01/2023 14:05", "2"]];
    let (mut wb, id) = pivot_on(rows, Locale::fr());
    update(&mut wb, id, |p| p.add_field(0, Area::Rows, None, Locale::fr()));
    wb.pivot_group_dates(id, 0, &[GroupBy::Days, GroupBy::Hours], None, None, 1, false)
        .unwrap();
    assert_report(
        &wb,
        id,
        "Étiquettes de lignes
         15-janv
         09
         14
         Total général",
    );
}

#[test]
fn grouping_numbers_by_interval() {
    let rows: &[[&str; 2]] = &[
        ["Age", "N"],
        ["3", "1"],
        ["12", "1"],
        ["18", "1"],
        ["25", "1"],
        ["41", "1"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    wb.pivot_group_numbers(id, 0, Some(0.0), Some(39.0), 10.0, false)
        .unwrap();
    wb.commit("group");
    // Whole numbers: "0-9", not "0-10"; 41 lies beyond the last group.
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         0-9|1
         10-19|2
         20-29|1
         >40|1
         Grand Total|5",
    );
    assert_eq!(wb.pivot(id).unwrap().1.cache.fields.len(), 2);
    // Automatic bounds: from the smallest value.
    wb.pivot_group_numbers(id, 0, None, None, 20.0, false).unwrap();
    wb.commit("group");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         3-22|3
         23-42|2
         Grand Total|5",
    );
    wb.pivot_ungroup(id, 0, None, false).unwrap();
    wb.commit("ungroup");
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 8, 0)), "3\n12\n18\n25\n41\nGrand Total");
    assert_eq!(
        wb.pivot_group_numbers(id, 0, None, None, 0.0, false),
        Err(PivotError::CannotGroup)
    );
    // Numbers with decimals: "0-10", "10-20".
    let rows: &[[&str; 2]] = &[["Price", "N"], ["2,5", "1"], ["12", "1"]];
    let (mut wb, id) = pivot_on(rows, Locale::fr());
    update(&mut wb, id, |p| p.add_field(0, Area::Rows, None, Locale::fr()));
    wb.pivot_group_numbers(id, 0, Some(0.0), Some(19.0), 10.0, false)
        .unwrap();
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 4, 0)), "0-10\n10-20");
}

#[test]
fn grouping_selected_items() {
    let rows: &[[&str; 2]] = &[
        ["City", "N"],
        ["Lyon", "1"],
        ["Paris", "2"],
        ["Rome", "4"],
        ["Turin", "8"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    let item = |wb: &Workbook, field: usize, label: &str| {
        let p = wb.pivot(id).unwrap().1;
        (0..p.cache.items(field).len() as u32)
            .find(|x| p.item_text(field, *x, wb.locale) == label)
            .unwrap()
    };
    let (lyon, paris) = (item(&wb, 0, "Lyon"), item(&wb, 0, "Paris"));
    let parent = wb.pivot_group_items(id, 0, &[lyon, paris], false).unwrap();
    wb.commit("group");
    // A new field holds the groups; it sits above the cities.
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.cache.fields[parent].name, "City2");
    assert_eq!(p.row_fields, [AxisField::Field(parent), AxisField::Field(0)]);
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         Group1|3
         Lyon|1
         Paris|2
         Rome|4
         Rome|4
         Turin|8
         Turin|8
         Grand Total|15",
    );
    // A second group joins the same field.
    let (rome, turin) = (item(&wb, 0, "Rome"), item(&wb, 0, "Turin"));
    assert_eq!(wb.pivot_group_items(id, 0, &[rome, turin], false), Ok(parent));
    wb.commit("group");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         Group1|3
         Lyon|1
         Paris|2
         Group2|12
         Rome|4
         Turin|8
         Grand Total|15",
    );
    // One item is not a group.
    assert_eq!(
        wb.pivot_group_items(id, 0, &[rome], false),
        Err(PivotError::CannotGroup)
    );
    // Dissolving the first group, then the other: the field goes.
    let group1 = item(&wb, parent, "Group1");
    wb.pivot_ungroup(id, parent, Some(group1), false).unwrap();
    wb.commit("ungroup");
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 7, 0)), "Lyon\nLyon\nParis\nParis\nGroup2");
    let group2 = item(&wb, parent, "Group2");
    wb.pivot_ungroup(id, parent, Some(group2), false).unwrap();
    wb.commit("ungroup");
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.cache.fields.len(), 2);
    assert_eq!(p.row_fields, [AxisField::Field(0)]);
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 0, 7, 1)),
        "Lyon|1\nParis|2\nRome|4\nTurin|8\nGrand Total|15"
    );
    // In French the groups are called "Groupe1".
    let (mut wb, id) = pivot_on(rows, Locale::fr());
    update(&mut wb, id, |p| p.add_field(0, Area::Rows, None, Locale::fr()));
    wb.pivot_group_items(id, 0, &[lyon, paris], false).unwrap();
    assert_eq!(cells(&wb, 1, Rect::new(3, 0, 3, 0)), "Groupe1");
}

// ---- refresh -----------------------------------------------------------------

#[test]
fn the_report_follows_the_source_only_when_refreshed() {
    let west = p_item(REGION, "West");
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| {
        p.set_item_hidden(REGION, west, true);
        p.sort_field(PRODUCT, SortOrder::Descending, None);
    });
    // East Apples 2023 goes from 100 to 1100; a new region appears.
    wb.set_cell_text(0, 1, 3, "1100").unwrap();
    wb.set_cell_text(0, 7, 0, "North").unwrap();
    wb.set_cell_text(0, 7, 1, "Kiwis").unwrap();
    wb.set_cell_text(0, 7, 3, "5").unwrap();
    wb.commit("edit");
    assert_eq!(at(&wb, id, "B4"), Value::Number(370.0));
    // The new row lies outside the source range: refreshing ignores it.
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("refresh");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|1370
         Pears|150
         Apples|1220
         Grand Total|1370",
    );
    // "Change Data Source" takes it in; hidden items stay hidden, the
    // sort order stays, and the new item joins the others.
    let source = PivotSource::parse("Sheet1!$A$1:$E$8", 1, &wb).unwrap();
    assert_eq!(source.to_text(), "Sheet1!$A$1:$E$8");
    wb.pivot_change_source(id, source, false).unwrap();
    wb.commit("source");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|1370
         Pears|150
         Apples|1220
         North|5
         Kiwis|5
         Grand Total|1375",
    );
    // An item that left the source leaves the report.
    wb.set_cell_text(0, 7, 0, "East").unwrap();
    wb.commit("edit");
    wb.pivot_refresh_all(false).unwrap();
    wb.commit("refresh");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|1375
         Pears|150
         Kiwis|5
         Apples|1220
         Grand Total|1375",
    );
    // A renamed column takes its field out of the report.
    wb.set_cell_text(0, 0, 1, "Article").unwrap();
    wb.commit("edit");
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("refresh");
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|1375
         Grand Total|1375",
    );
    assert_eq!(wb.pivot(id).unwrap().1.cache.fields[1].name, "Article");
}

#[test]
fn pivots_on_one_source_share_their_cache() {
    let (mut wb, first) = region_by_year(Locale::en());
    let source = wb.pivot(first).unwrap().1.source.clone();
    wb.set_cell_text(0, 1, 3, "1100").unwrap();
    wb.commit("edit");
    let dest = PivotDestination::Existing {
        sheet: 1,
        row: 20,
        col: 0,
    };
    let second = wb.pivot_create(source, dest, false).unwrap();
    assert!(Arc::ptr_eq(
        &wb.pivot(first).unwrap().1.cache,
        &wb.pivot(second).unwrap().1.cache
    ));
    wb.pivot_update(second, false, |p| {
        p.add_field(SALES_F, Area::Values, None, Locale::en())
    })
    .unwrap();
    wb.commit("pivot");
    // The new PivotTable shows the cache as it stands: not refreshed yet.
    assert_eq!(cells(&wb, 1, Rect::new(20, 0, 21, 0)), "Sum of Sales\n690");
    // Refreshing one refreshes both.
    wb.pivot_refresh(second, false).unwrap();
    wb.commit("refresh");
    assert_eq!(cells(&wb, 1, Rect::new(20, 0, 21, 0)), "Sum of Sales\n1690");
    assert_eq!(at(&wb, first, "D7"), Value::Number(1690.0));
    assert!(Arc::ptr_eq(
        &wb.pivot(first).unwrap().1.cache,
        &wb.pivot(second).unwrap().1.cache
    ));
    // Grouping is shared too: the second PivotTable gets the field.
    let east = p_item(REGION, "East");
    let west = p_item(REGION, "West");
    let parent = wb.pivot_group_items(first, REGION, &[east, west], false).unwrap();
    assert_eq!(wb.pivot(second).unwrap().1.cache.fields[parent].name, "Region2");
    assert_eq!(wb.pivot(second).unwrap().1.fields.len(), 6);
}

#[test]
fn a_table_or_a_name_can_be_the_source() {
    let (mut wb, _) = sales(Locale::en());
    wb.sheets[0].tables.push(crate::table::Table {
        id: 1,
        name: "Sales".to_string(),
        range: Rect::new(0, 0, 6, 4),
        header_row: true,
        totals_row: false,
        columns: Vec::new(),
        ..Default::default()
    });
    let source = PivotSource::parse("sales", 1, &wb).unwrap();
    assert_eq!(source, PivotSource::Name("sales".to_string()));
    let id = wb
        .pivot_create(
            source,
            PivotDestination::Existing {
                sheet: 1,
                row: 30,
                col: 0,
            },
            false,
        )
        .unwrap();
    assert_eq!(wb.pivot(id).unwrap().1.cache.rows, 6);
    // A totals row is not a record.
    wb.sheets[0].tables[0].totals_row = true;
    wb.pivot_refresh(id, false).unwrap();
    assert_eq!(wb.pivot(id).unwrap().1.cache.rows, 5);
    let expr = crate::parser::parse_formula("Sheet1!$A$1:$B$4", Locale::en()).unwrap();
    wb.define_name("Extract", expr);
    let id = wb
        .pivot_create(
            PivotSource::Name("Extract".into()),
            PivotDestination::Existing {
                sheet: 1,
                row: 60,
                col: 0,
            },
            false,
        )
        .unwrap();
    let p = wb.pivot(id).unwrap().1;
    assert_eq!((p.cache.rows, p.cache.fields.len()), (3, 2));
    assert!(PivotSource::parse("NoSuchName", 0, &wb).is_none());
    assert!(PivotSource::parse("1+1", 0, &wb).is_none());
}

// ---- writing into the sheet --------------------------------------------------

#[test]
fn a_report_does_not_overwrite_without_asking() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| p.add_field(REGION, Area::Rows, None, Locale::en()));
    // A note right below the report, where the products would go.
    wb.set_cell_text(1, 6, 0, "note").unwrap();
    wb.commit("note");
    let before = shown(&wb, id);
    let grow = |p: &mut PivotTable| p.add_field(PRODUCT, Area::Rows, None, Locale::en());
    assert_eq!(
        wb.pivot_update(id, false, grow),
        Err(PivotError::WouldOverwrite(Rect::new(2, 0, 9, 0)))
    );
    // Nothing changed, and nothing is left to undo.
    assert_eq!(shown(&wb, id), before);
    assert_eq!(wb.pivot(id).unwrap().1.row_fields, [AxisField::Field(REGION)]);
    assert!(!wb.commit("nothing"));
    // The user agrees.
    wb.pivot_update(id, true, grow).unwrap();
    wb.commit("pivot");
    assert_eq!(cells(&wb, 1, Rect::new(6, 0, 6, 0)), "West");
    // Shrinking the report clears the cells it leaves.
    update(&mut wb, id, |p| p.remove_field(PRODUCT));
    assert_eq!(cells(&wb, 1, Rect::new(5, 0, 9, 0)), "Grand Total\n\n\n\n");
    assert!(wb.sheets[1].cells.keys().all(|(r, _)| *r <= 5));
}

#[test]
fn a_report_never_overlaps_another_pivot_a_table_or_merged_cells() {
    let (mut wb, first) = sales(Locale::en());
    update(&mut wb, first, |p| p.add_field(REGION, Area::Rows, None, Locale::en()));
    let source = wb.pivot(first).unwrap().1.source.clone();
    // Inside the first one.
    let inside = PivotDestination::Existing {
        sheet: 1,
        row: 4,
        col: 0,
    };
    assert_eq!(
        wb.pivot_create(source.clone(), inside, true),
        Err(PivotError::OverlapsPivot("PivotTable1".into()))
    );
    assert_eq!(wb.pivots().count(), 1);
    // To its right, one column apart; then the first one wants to grow
    // into it.
    let right = PivotDestination::Existing {
        sheet: 1,
        row: 2,
        col: 2,
    };
    let second = wb.pivot_create(source, right, false).unwrap();
    wb.commit("pivot");
    let grow = |p: &mut PivotTable| {
        p.add_field(YEAR, Area::Columns, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    };
    assert_eq!(
        wb.pivot_update(first, true, grow),
        Err(PivotError::OverlapsPivot("PivotTable2".into()))
    );
    wb.pivot_delete(second).unwrap();
    wb.commit("delete");
    wb.sheets[1].tables.push(crate::table::Table {
        range: Rect::new(2, 3, 3, 3),
        ..Default::default()
    });
    assert_eq!(wb.pivot_update(first, true, grow), Err(PivotError::OverlapsTable));
    wb.sheets[1].tables.clear();
    wb.merge(1, Rect::new(4, 2, 4, 5));
    wb.commit("merge");
    assert_eq!(wb.pivot_update(first, true, grow), Err(PivotError::OverlapsMerged));
    wb.unmerge(1, Rect::new(4, 2, 4, 5));
    assert_eq!(wb.pivot_update(first, false, grow), Ok(()));
    // A report cannot run off the sheet.
    let edge = PivotDestination::Existing {
        sheet: 1,
        row: 2,
        col: crate::refs::MAX_COLS - 2,
    };
    assert_eq!(wb.pivot_move(first, edge, false), Err(PivotError::TooLarge));
}

#[test]
fn undo_and_redo_restore_the_report_and_its_definition() {
    let (mut wb, id) = region_by_year(Locale::en());
    let before = shown(&wb, id);
    update(&mut wb, id, |p| p.remove_field(YEAR));
    let after = shown(&wb, id);
    assert_ne!(before, after);
    assert!(wb.undo());
    assert_eq!(shown(&wb, id), before);
    assert_eq!(wb.pivot(id).unwrap().1.col_fields, [AxisField::Field(YEAR)]);
    assert_eq!(cells(&wb, 1, Rect::new(3, 3, 3, 3)), "Grand Total");
    assert!(wb.redo());
    assert_eq!(shown(&wb, id), after);
    assert_eq!(cells(&wb, 1, Rect::new(3, 3, 3, 3)), "");
    // Back to before the PivotTable existed: its sheet goes with it.
    while wb.undo() {}
    assert_eq!(wb.pivots().count(), 0);
    assert_eq!(wb.sheets.len(), 1);
}

#[test]
fn rename_move_clear_and_delete() {
    let (mut wb, id) = region_by_year(Locale::en());
    let source = wb.pivot(id).unwrap().1.source.clone();
    let other = wb
        .pivot_create(
            source,
            PivotDestination::Existing {
                sheet: 1,
                row: 30,
                col: 0,
            },
            false,
        )
        .unwrap();
    assert_eq!(wb.pivot_rename(other, " pivottable1 "), Err(PivotError::InvalidName));
    assert_eq!(wb.pivot_rename(other, ""), Err(PivotError::InvalidName));
    assert_eq!(wb.pivot_rename(other, "By region"), Ok(()));
    assert_eq!(wb.pivot(other).unwrap().1.name, "By region");
    assert_eq!(wb.pivot_default_name(), "PivotTable2");
    wb.commit("rename");
    // Move on the same sheet: the old cells are cleared.
    let report = shown(&wb, id);
    wb.pivot_move(
        id,
        PivotDestination::Existing {
            sheet: 1,
            row: 9,
            col: 5,
        },
        false,
    )
    .unwrap();
    wb.commit("move");
    assert_eq!(wb.pivot(id).unwrap().1.location, Some(Rect::new(9, 5, 13, 8)));
    assert_eq!(shown(&wb, id), report);
    assert_eq!(cells(&wb, 1, Rect::new(2, 0, 6, 3)), "|||\n|||\n|||\n|||\n|||");
    // With a filter, the destination is the corner of the filter.
    update(&mut wb, id, |p| p.add_field(PRODUCT, Area::Filters, None, Locale::en()));
    wb.pivot_move(
        id,
        PivotDestination::Existing {
            sheet: 0,
            row: 10,
            col: 1,
        },
        false,
    )
    .unwrap();
    wb.commit("move");
    let (si, p) = wb.pivot(id).unwrap();
    assert_eq!(si, 0);
    assert_eq!(
        (p.page_location, p.location),
        (Some(Rect::new(10, 1, 10, 2)), Some(Rect::new(12, 1, 16, 4)))
    );
    assert_eq!(shown(&wb, id), report);
    assert_eq!(wb.sheets[1].pivots.len(), 1);
    assert!(wb.sheets[1].cells.is_empty());
    // To a new sheet.
    wb.pivot_move(id, PivotDestination::NewSheet { index: 0 }, false)
        .unwrap();
    wb.commit("move");
    let (si, p) = wb.pivot(id).unwrap();
    assert_eq!(
        (si, p.page_location, p.location),
        (0, Some(Rect::new(2, 0, 2, 1)), Some(Rect::new(4, 0, 8, 3)))
    );
    assert_eq!(wb.sheets.len(), 3);
    // "Clear All" empties the areas; deleting removes the cells.
    update(&mut wb, id, |p| p.clear());
    let p = wb.pivot(id).unwrap().1;
    assert!(p.report.placeholder && p.page_location.is_none());
    assert!(wb.sheets[0].cells.is_empty());
    update(&mut wb, id, |p| p.add_field(REGION, Area::Rows, None, Locale::en()));
    wb.pivot_delete(id).unwrap();
    wb.commit("delete");
    assert!(wb.pivot(id).is_none());
    assert!(wb.sheets[0].cells.is_empty());
    assert_eq!(wb.pivot_delete(id), Err(PivotError::NoSuchPivot));
}

#[test]
fn rows_columns_and_sheets_move_under_a_pivot() {
    let (mut wb, id) = region_by_year(Locale::en());
    let report = shown(&wb, id);
    // Two rows above and a column left of the report.
    wb.insert_rows(1, 0, 2);
    wb.insert_cols(1, 0, 1);
    wb.commit("insert");
    assert_eq!(wb.pivot(id).unwrap().1.location, Some(Rect::new(4, 1, 8, 4)));
    assert_eq!(shown(&wb, id), report);
    assert_eq!(wb.pivot_cell(1, 5, 1).unwrap().part, PivotPart::RowHeaderCaption);
    assert!(wb.pivot_cell(1, 3, 0).is_none());
    // The source follows its rows and columns, and its sheet's name.
    wb.insert_rows(0, 0, 3);
    wb.delete_cols(0, 1, 1);
    wb.rename_sheet(0, "Data");
    wb.commit("edit");
    let source = PivotSource::Range {
        sheet: "Data".to_string(),
        range: Rect::new(3, 0, 9, 3),
    };
    assert_eq!(wb.pivot(id).unwrap().1.source, source);
    assert_eq!(source.to_text(), "Data!$A$4:$D$10");
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("refresh");
    // Product is gone with its column; the rest is as before.
    assert_eq!(shown(&wb, id), report);
    assert_eq!(wb.pivot(id).unwrap().1.cache.fields.len(), 4);
    // Edits through a report are for the application to refuse.
    use crate::ast::StructuralEdit::*;
    assert!(wb.pivot_blocks_edit(1, InsertRows { at: 6, count: 1 }));
    assert!(wb.pivot_blocks_edit(1, DeleteCols { at: 0, count: 2 }));
    assert!(!wb.pivot_blocks_edit(1, InsertRows { at: 4, count: 1 }));
    assert!(!wb.pivot_blocks_edit(1, DeleteRows { at: 0, count: 20 }));
    assert!(!wb.pivot_blocks_edit(0, InsertRows { at: 6, count: 1 }));
    assert_eq!(wb.pivots_in(1, Rect::new(4, 1, 5, 1)), [(id, false)]);
    assert_eq!(wb.pivots_in(1, Rect::new(0, 0, 20, 20)), [(id, true)]);
    assert!(wb.pivots_in(1, Rect::new(0, 0, 3, 20)).is_empty());
    // A copy of the sheet carries a copy of the PivotTable, with its own id.
    let copy = wb.duplicate_sheet(1).unwrap();
    wb.commit("copy");
    let twin = &wb.sheets[copy].pivots[0];
    assert_ne!(twin.id, id);
    assert_eq!((twin.location, &twin.source), (Some(Rect::new(4, 1, 8, 4)), &source));
    assert_eq!(cells(&wb, copy, twin.location.unwrap()), report);
    // Deleting the rows of the report deletes the PivotTable; deleting
    // the source leaves a PivotTable that cannot be refreshed.
    wb.delete_rows(1, 0, 20);
    wb.delete_rows(0, 0, 20);
    wb.commit("delete");
    assert!(wb.pivot(id).is_none());
    let twin = wb.sheets[copy].pivots[0].id;
    assert_eq!(wb.pivot(twin).unwrap().1.source, PivotSource::Invalid);
    assert_eq!(wb.pivot(twin).unwrap().1.source.to_text(), "#REF!");
    assert_eq!(wb.pivot_refresh(twin, false), Err(PivotError::InvalidSource));
    assert_eq!(cells(&wb, copy, Rect::new(4, 1, 8, 4)), report);
}

// ---- what a cell is ----------------------------------------------------------

fn cell_at(wb: &Workbook, name: &str) -> PivotCell {
    let (r, c) = parse_cell_name(name).unwrap();
    wb.pivot_cell(1, r, c)
        .unwrap_or_else(|| panic!("{name} is not in a PivotTable"))
}

#[test]
fn every_cell_knows_its_part() {
    // A3: Sum of Sales | Column Labels |      |
    // A4: Row Labels   | 2023          | 2024 | Grand Total
    // A5: East ... A7: Grand Total
    let (wb, id) = region_by_year(Locale::en());
    let (east, y2024) = (p_item(REGION, "East"), p_item(YEAR, "2024"));
    let part = |name: &str| cell_at(&wb, name).part;
    assert_eq!(part("A3"), PivotPart::DataCaption);
    assert_eq!(part("B3"), PivotPart::ColumnHeaderCaption);
    assert_eq!(part("C3"), PivotPart::Blank);
    assert_eq!(part("A4"), PivotPart::RowHeaderCaption);
    assert_eq!(part("B4"), PivotPart::ColumnLabel);
    assert_eq!(part("D4"), PivotPart::ColumnGrandTotalLabel);
    assert_eq!(part("A5"), PivotPart::RowLabel);
    assert_eq!(part("C5"), PivotPart::Value);
    assert_eq!(part("A7"), PivotPart::RowGrandTotalLabel);
    assert!(wb.pivot_cell(1, 7, 0).is_none() && wb.pivot_cell(1, 2, 4).is_none());
    assert_eq!(wb.pivot_at(1, 4, 2).map(|p| p.id), Some(id));
    // The captions carry the drop-downs of their fields.
    let rows = cell_at(&wb, "A4");
    assert_eq!(
        (rows.fields.as_slice(), rows.button),
        (&[REGION][..], Some(PivotButton::Filter { filtered: false }))
    );
    let cols = cell_at(&wb, "B3");
    assert_eq!(
        (cols.fields.as_slice(), cols.button),
        (&[YEAR][..], Some(PivotButton::Filter { filtered: false }))
    );
    assert_eq!(cell_at(&wb, "A3").data_field, Some(0));
    // A value stands for its row items, its column items and its field.
    let value = cell_at(&wb, "C5");
    assert_eq!(value.row_items, [(REGION, east)]);
    assert_eq!(value.col_items, [(YEAR, y2024)]);
    assert_eq!(value.data_field, Some(0));
    assert_eq!(
        (value.row_kind, value.col_kind),
        (Some(ReportLineKind::Item), Some(ReportLineKind::Item))
    );
    let total = cell_at(&wb, "D7");
    assert_eq!(total.part, PivotPart::Value);
    assert!(total.row_items.is_empty() && total.col_items.is_empty());
    assert_eq!(
        (total.row_kind, total.col_kind),
        (Some(ReportLineKind::GrandTotal), Some(ReportLineKind::GrandTotal))
    );
    // Labels name their field and item; with nothing below, no button.
    let label = cell_at(&wb, "A5");
    assert_eq!(
        (label.field, label.item, label.button),
        (Some(REGION), Some(east), None)
    );
    let label = cell_at(&wb, "C4");
    assert_eq!((label.field, label.item), (Some(YEAR), Some(y2024)));
    assert_eq!(label.col_items, [(YEAR, y2024)]);
}

#[test]
fn headers_of_outline_and_tabular_forms() {
    let (mut wb, id) = region_product(Locale::en());
    let (west, pears) = (p_item(REGION, "West"), p_item(PRODUCT, "Pears"));
    update(&mut wb, id, |p| {
        p.set_report_layout(FieldLayout::Tabular);
        p.set_item_hidden(PRODUCT, pears, true);
        p.set_item_hidden(PRODUCT, pears, false);
        p.set_label_filter(PRODUCT, Some(LabelFilter::Contains("p".into())));
    });
    // A3: Region | Product | Sum of Sales
    // A4: East   | Apples  | 220
    // A5:        | Pears   | 150
    // A6: East Total
    let region = cell_at(&wb, "A3");
    assert_eq!(
        (region.part, region.field, region.button),
        (
            PivotPart::FieldHeader,
            Some(REGION),
            Some(PivotButton::Filter { filtered: false })
        )
    );
    let product = cell_at(&wb, "B3");
    assert_eq!(
        (product.part, product.field, product.button),
        (
            PivotPart::FieldHeader,
            Some(PRODUCT),
            Some(PivotButton::Filter { filtered: true })
        )
    );
    assert_eq!(cell_at(&wb, "C3").part, PivotPart::DataCaption);
    let east = cell_at(&wb, "A4");
    assert_eq!(
        (east.part, east.button),
        (PivotPart::RowLabel, Some(PivotButton::Collapse))
    );
    // The second detail row lies under East too, though its label cell is
    // empty.
    let below = cell_at(&wb, "A5");
    assert_eq!(below.part, PivotPart::Blank);
    assert_eq!(below.row_items.len(), 2);
    let pear = cell_at(&wb, "B5");
    assert_eq!((pear.field, pear.item), (Some(PRODUCT), Some(pears)));
    let subtotal = cell_at(&wb, "A6");
    assert_eq!(
        (subtotal.part, subtotal.field, subtotal.button),
        (PivotPart::RowSubtotalLabel, Some(REGION), None)
    );
    assert_eq!(cell_at(&wb, "C6").row_kind, Some(ReportLineKind::Subtotal));
    assert_eq!(cell_at(&wb, "A7").row_items, [(REGION, west)]);
    // Without captions and drop-downs.
    update(&mut wb, id, |p| p.options.show_headers = false);
    assert_eq!(cell_at(&wb, "A3").button, None);
    // The lines of the report, for the file format.
    let p = wb.pivot(id).unwrap().1;
    let kinds: Vec<ReportLineKind> = p.report.lines(true, p).iter().map(|l| l.kind).collect();
    use ReportLineKind::*;
    assert_eq!(kinds, [Item, Item, Subtotal, Item, Item, Subtotal, GrandTotal]);
    assert_eq!(
        p.report.lines(true, p)[1].items,
        [(REGION, p_item(REGION, "East")), (PRODUCT, pears)]
    );
    assert_eq!(p.report.lines(false, p).len(), 1);
}

#[test]
fn values_in_the_rows_with_columns() {
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(UNITS, Area::Values, None, Locale::en());
        p.move_entry(Area::Columns, 1, Area::Rows, 0, Locale::en());
    });
    assert_eq!(
        wb.pivot(id).unwrap().1.row_fields,
        [AxisField::Values, AxisField::Field(REGION)]
    );
    assert_report(
        &wb,
        id,
        "|Column Labels||
         Row Labels|2023|2024|Grand Total
         Sum of Sales|||
         East|250|120|370
         West|200|120|320
         Sum of Units|||
         East|15|12|27
         West|20|9|29
         Total Sum of Sales|450|240|690
         Total Sum of Units|35|21|56",
    );
    let label = cell_at(&wb, "A5");
    assert_eq!(
        (label.part, label.field, label.data_field),
        (PivotPart::RowLabel, None, Some(0))
    );
    assert_eq!(cell_at(&wb, "B10").data_field, Some(1));
    assert_eq!(cell_at(&wb, "A11").data_field, Some(0));
    // Tabular: "Values" has its column and its header.
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Tabular));
    assert_report(
        &wb,
        id,
        "||Year||
         Values|Region|2023|2024|Grand Total
         Sum of Sales|East|250|120|370
         |West|200|120|320
         Sum of Units|East|15|12|27
         |West|20|9|29
         Total Sum of Sales||450|240|690
         Total Sum of Units||35|21|56",
    );
    assert_eq!(cell_at(&wb, "C3").part, PivotPart::FieldHeader);
    let values = cell_at(&wb, "A4");
    assert_eq!(
        (values.part, values.field, values.button),
        (PivotPart::FieldHeader, None, None)
    );
    // Region outside, the values inside: subtotals per value field.
    update(&mut wb, id, |p| {
        p.set_report_layout(FieldLayout::Compact);
        p.move_entry(Area::Rows, 0, Area::Rows, 1, Locale::en());
        p.add_field(PRODUCT, Area::Rows, Some(1), Locale::en());
        p.remove_field(YEAR);
        p.set_collapsed(REGION, p_item(REGION, "West"), true);
    });
    assert_report(
        &wb,
        id,
        "Row Labels|
         East|
         Apples|
         Sum of Sales|220
         Sum of Units|22
         Pears|
         Sum of Sales|150
         Sum of Units|5
         East Sum of Sales|370
         East Sum of Units|27
         West|
         Sum of Sales|320
         Sum of Units|29
         Total Sum of Sales|690
         Total Sum of Units|56",
    );
}

#[test]
fn moving_entries_between_the_areas() {
    let en = Locale::en();
    let (mut wb, id) = region_by_year(en);
    update(&mut wb, id, |p| {
        // Year from the columns to the rows, before Region.
        assert!(p.move_entry(Area::Columns, 0, Area::Rows, 0, en));
        assert_eq!(p.row_fields, [AxisField::Field(YEAR), AxisField::Field(REGION)]);
        assert_eq!(p.axis_of(YEAR), Some(Area::Rows));
        // Region to the filters; Year to the values (counted? no: summed,
        // years are numbers).
        assert!(p.move_entry(Area::Rows, 1, Area::Filters, 0, en));
        assert_eq!(p.axis_of(REGION), Some(Area::Filters));
        assert!(p.move_entry(Area::Rows, 0, Area::Values, 0, en));
        assert!(p.row_fields.is_empty());
        let names: Vec<&str> = p.data_fields.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Sum of Year", "Sum of Sales"]);
        assert_eq!(p.col_fields, [AxisField::Values]);
        // Reorder the values; "Σ Values" cannot go to the filters.
        assert!(p.move_entry(Area::Values, 0, Area::Values, 1, en));
        assert_eq!(p.data_fields[0].name, "Sum of Sales");
        assert!(!p.move_entry(Area::Columns, 0, Area::Filters, 0, en));
        // A value dragged to the rows becomes a row field.
        assert!(p.move_entry(Area::Values, 1, Area::Rows, 0, en));
        assert_eq!(p.row_fields, [AxisField::Field(YEAR)]);
        assert!(p.col_fields.is_empty());
        assert!(!p.move_entry(Area::Rows, 5, Area::Columns, 0, en));
        // A field sits in one area only, but can be a value besides.
        p.add_field(YEAR, Area::Columns, None, en);
        assert!(p.row_fields.is_empty());
        p.add_field(YEAR, Area::Values, None, en);
        assert_eq!((p.axis_of(YEAR), p.data_fields.len()), (Some(Area::Columns), 2));
    });
    // Sorting by a value field follows that field through the list.
    let (mut wb, id) = region_by_year(en);
    update(&mut wb, id, |p| {
        p.add_field(UNITS, Area::Values, None, en);
        p.sort_field(REGION, SortOrder::Descending, Some(1));
        p.set_value_filter(
            REGION,
            Some(FilterKind::Value {
                data_field: 1,
                test: ValueFilter::GreaterThan(0.0),
            }),
        );
        p.add_field(SALES_F, Area::Values, Some(0), en);
        assert_eq!(p.fields[REGION].sort.by_value, Some(2));
        p.move_entry(Area::Values, 2, Area::Values, 0, en);
        assert_eq!(p.fields[REGION].sort.by_value, Some(0));
        p.remove_data_field(1);
        assert_eq!(p.fields[REGION].sort.by_value, Some(0));
        assert_eq!(
            p.filters[0].kind,
            FilterKind::Value {
                data_field: 0,
                test: ValueFilter::GreaterThan(0.0)
            }
        );
        p.remove_data_field(0);
        assert_eq!(p.fields[REGION].sort, FieldSort::default());
        assert!(p.filters.is_empty());
    });
}

// ---- styles ------------------------------------------------------------------

#[test]
fn the_default_style_is_light_16() {
    let (mut wb, id) = region_product(Locale::en());
    assert_eq!(wb.pivot(id).unwrap().1.style, PivotStyle::default());
    assert_eq!(wb.pivot(id).unwrap().1.style.name, "PivotStyleLight16");
    // A3: Row Labels | Sum of Sales     header
    // A4: East       | 370              item above its detail
    // A5: Apples     | 220
    // A10: Grand Total
    let (pale, rule) = (Color(0xC1E5F5), Color(0x46B1E1));
    let look = |wb: &Workbook, name: &str| {
        let (r, c) = parse_cell_name(name).unwrap();
        wb.display(1, r, c).style.into_owned()
    };
    for name in ["A3", "B3"] {
        let s = look(&wb, name);
        assert_eq!((s.fill, s.font.bold), (Some(pale), true), "{name}");
        assert_eq!(
            (s.borders.bottom.style, s.borders.bottom.color),
            (crate::style::BorderStyle::Thin, Some(rule))
        );
    }
    let s = look(&wb, "B4");
    assert_eq!((s.fill, s.font.bold), (None, true));
    let s = look(&wb, "B5");
    assert_eq!((s.fill, s.font.bold, s.borders.is_none()), (None, false, true));
    let s = look(&wb, "A10");
    assert_eq!(
        (s.fill, s.font.bold, s.borders.top.color),
        (Some(pale), true, Some(rule))
    );
    // Outside the report: nothing.
    assert!(style_patch_at(&wb, 1, 10, 0).is_none());
    assert!(style_patch_at(&wb, 1, 3, 2).is_none());
    assert!(style_patch_at(&wb, 0, 3, 0).is_none());
    // The options.
    update(&mut wb, id, |p| {
        p.style.col_headers = false;
        p.style.row_headers = false;
        p.style.row_stripes = true;
    });
    assert_eq!(look(&wb, "A3").fill, None);
    assert!(!look(&wb, "B4").font.bold);
    assert_eq!(look(&wb, "B4").borders.top.color, Some(rule));
    assert!(look(&wb, "B5").borders.is_none());
    // Another style, by name; an unknown name is no style.
    update(&mut wb, id, |p| {
        p.style = PivotStyle {
            name: "PivotStyleMedium9".into(),
            ..PivotStyle::default()
        }
    });
    let s = look(&wb, "A3");
    assert_eq!((s.fill, s.font.color), (Some(Color(0x156082)), Some(Color::WHITE)));
    assert_eq!(look(&wb, "B5").fill, Some(pale));
    update(&mut wb, id, |p| p.style.name = "PivotStyleDark3".into());
    assert_eq!(look(&wb, "A3").fill, Some(Color::BLACK));
    update(&mut wb, id, |p| p.style.name = String::new());
    assert!(style_patch_at(&wb, 1, 2, 0).is_none());
}

// ---- GETPIVOTDATA ------------------------------------------------------------

#[test]
fn getpivotdata() {
    let (mut wb, id) = region_by_year(Locale::en());
    let mut get = |formula: &str| {
        wb.set_cell_text(1, 0, 8, &format!("={formula}")).unwrap();
        wb.commit("formula");
        wb.value_at(1, 0, 8)
    };
    let n = Value::Number;
    let missing = Value::Error(ErrorKind::Ref);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3)"), n(690.0));
    assert_eq!(get("GETPIVOTDATA(\"Sum of Sales\",$A$3)"), n(690.0));
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"East\")"), n(370.0));
    assert_eq!(
        get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"east\",\"Year\",2023)"),
        n(250.0)
    );
    assert_eq!(
        get("GETPIVOTDATA(\"Sales\",$A$3,\"Year\",2024,\"Region\",\"West\")"),
        n(120.0)
    );
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Year\",\"2024\")"), n(240.0));
    // Any cell of the PivotTable will do, and so does a range.
    assert_eq!(get("GETPIVOTDATA(\"Sales\",C6:D7,\"Year\",2023)"), n(450.0));
    assert_eq!(get("GETPIVOTDATA(\"Sales\",A3,\"Year\",2023)*2"), n(900.0));
    // What the report does not show is #REF!.
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$1)"), missing);
    assert_eq!(get("GETPIVOTDATA(\"Units\",$A$3)"), missing);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"North\")"), missing);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Product\",\"Apples\")"), missing);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Nothing\",1)"), missing);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\")"), missing);
    assert_eq!(
        get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",#N/A)"),
        Value::Error(ErrorKind::NA)
    );
    assert_eq!(get("GETPIVOTDATA(\"Sales\",5)"), missing);
    // The formula follows the report: hide West and its value is gone.
    let west = p_item(REGION, "West");
    wb.set_cell_text(1, 0, 9, "=GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"West\")")
        .unwrap();
    wb.set_cell_text(1, 0, 10, "=GETPIVOTDATA(\"Sales\",$A$3)").unwrap();
    wb.commit("formulas");
    assert_eq!(cells(&wb, 1, Rect::new(0, 9, 0, 10)), "320|690");
    update(&mut wb, id, |p| p.set_item_hidden(REGION, west, true));
    assert_eq!(cells(&wb, 1, Rect::new(0, 9, 0, 10)), "#REF!|370");
}

#[test]
fn getpivotdata_with_subtotals_filters_and_several_values() {
    let (mut wb, id) = region_product(Locale::en());
    let (y2023, apples) = (p_item(YEAR, "2023"), p_item(PRODUCT, "Apples"));
    update(&mut wb, id, |p| {
        p.add_field(UNITS, Area::Values, None, Locale::en());
        p.add_field(YEAR, Area::Filters, None, Locale::en());
    });
    fn get(wb: &mut Workbook, formula: &str) -> Value {
        wb.set_cell_text(1, 0, 8, &format!("={formula}")).unwrap();
        wb.commit("formula");
        wb.value_at(1, 0, 8)
    }
    let n = Value::Number;
    let missing = Value::Error(ErrorKind::Ref);
    assert_eq!(
        get(
            &mut wb,
            "GETPIVOTDATA(\"Sum of Units\",$A$3,\"Region\",\"West\",\"Product\",\"Pears\")"
        ),
        n(9.0)
    );
    assert_eq!(
        get(&mut wb, "GETPIVOTDATA(\"Units\",$A$3,\"Region\",\"West\")"),
        n(29.0)
    );
    assert_eq!(get(&mut wb, "GETPIVOTDATA(\"Sales\",$A$3)"), n(690.0));
    // The filter shows every year: no single one can be asked for.
    assert_eq!(get(&mut wb, "GETPIVOTDATA(\"Sales\",$A$3,\"Year\",2023)"), missing);
    update(&mut wb, id, |p| {
        p.set_page_item(YEAR, Some(y2023));
        p.set_subtotals(false, true);
        p.set_collapsed(PRODUCT, apples, true);
    });
    let mut get = |formula: &str| get(&mut wb, formula);
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Year\",2023)"), n(450.0));
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Year\",2024)"), missing);
    assert_eq!(
        get("GETPIVOTDATA(\"Sales\",$A$1,\"Region\",\"East\",\"Product\",\"Pears\")"),
        n(150.0)
    );
    // Subtotals are off: the total of a region is not in the report.
    assert_eq!(get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"East\")"), missing);
    // West sold no pears in 2023: the item is not there.
    assert_eq!(
        get("GETPIVOTDATA(\"Sales\",$A$3,\"Region\",\"West\",\"Product\",\"Pears\")"),
        missing
    );
}

#[test]
fn getpivotdata_in_the_other_languages() {
    for (locale, formula) in [
        (
            Locale::fr(),
            "=LIREDONNEESTABCROISDYNAMIQUE(\"Sales\";$A$3;\"Region\";\"East\";\"Year\";2023)",
        ),
        (
            Locale::es(),
            "=IMPORTARDATOSDINAMICOS(\"Sales\";$A$3;\"Region\";\"East\";\"Year\";2023)",
        ),
        (
            Locale::de(),
            "=PIVOTDATENZUORDNEN(\"Sales\";$A$3;\"Region\";\"East\";\"Year\";2023)",
        ),
        (
            Locale::it(),
            "=INFO.DATI.TAB.PIVOT(\"Sales\";$A$3;\"Region\";\"East\";\"Year\";2023)",
        ),
    ] {
        let (mut wb, _) = region_by_year(locale);
        wb.set_cell_text(1, 0, 8, formula).unwrap();
        wb.commit("formula");
        assert_eq!(wb.value_at(1, 0, 8), Value::Number(250.0), "{}", locale.id);
        assert_eq!(wb.formula_text(1, 0, 8).as_deref(), Some(formula), "{}", locale.id);
    }
    // Dates can be given as dates or as the report writes them.
    let (mut wb, id) = pivot_on(DATED, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    wb.set_cell_text(1, 0, 8, "=GETPIVOTDATA(\"Amount\",A3,\"Date\",DATE(2023,5,5))")
        .unwrap();
    wb.set_cell_text(1, 0, 9, "=GETPIVOTDATA(\"Amount\",A3,\"Date\",\"1/10/2024\")")
        .unwrap();
    wb.commit("formulas");
    assert_eq!(cells(&wb, 1, Rect::new(0, 8, 0, 9)), "30|40");
    wb.pivot_group_dates(id, 0, &[GroupBy::Years, GroupBy::Months], None, None, 1, false)
        .unwrap();
    wb.set_cell_text(
        1,
        0,
        8,
        "=GETPIVOTDATA(\"Amount\",A3,\"Years (Date)\",2024,\"Months (Date)\",\"Nov\")",
    )
    .unwrap();
    wb.set_cell_text(1, 0, 9, "=GETPIVOTDATA(\"Amount\",A3,\"Years (Date)\",2023)")
        .unwrap();
    wb.commit("formulas");
    assert_eq!(cells(&wb, 1, Rect::new(0, 8, 0, 9)), "50|60");
}

// ---- drilling down -----------------------------------------------------------

#[test]
fn show_details_lists_the_records_behind_a_value() {
    let (mut wb, id) = region_by_year(Locale::en());
    // C6 is West in 2024: two records of pears.
    let (sheet, range) = wb.pivot_show_details(1, 5, 2).unwrap();
    wb.commit("details");
    assert_eq!((sheet, range), (1, Rect::new(0, 0, 2, 4)));
    assert_eq!(wb.sheets[1].name, "Sheet3");
    assert_eq!(
        cells(&wb, 1, range),
        "Region|Product|Year|Sales|Units\nWest|Pears|2024|50|2\nWest|Pears|2024|70|7"
    );
    // The PivotTable is one sheet further now. D7 is the grand total: all
    // six records; the labels have nothing to show.
    assert_eq!(wb.pivot(id).unwrap().0, 2);
    let (sheet, range) = wb.pivot_show_details(2, 6, 3).unwrap();
    assert_eq!(range, Rect::new(0, 0, 6, 4));
    assert_eq!(cells(&wb, sheet, Rect::new(6, 0, 6, 4)), "West|Pears|2024|70|7");
    assert_eq!(wb.pivot_show_details(3, 4, 0), Err(PivotError::NotAValue));
    assert_eq!(wb.pivot_show_details(3, 40, 0), Err(PivotError::NotAValue));
    // Filtered records stay out, and dates keep their format.
    let (mut wb, id) = pivot_on(DATED, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(1, Area::Values, None, Locale::en());
        p.add_field(0, Area::Filters, None, Locale::en());
        p.set_visible_items(0, &[0, 3]);
    });
    assert_eq!(cells(&wb, 1, Rect::new(2, 0, 3, 0)), "Sum of Amount\n50");
    let (sheet, range) = wb.pivot_show_details(1, 3, 0).unwrap();
    assert_eq!(cells(&wb, sheet, range), "Date|Amount\n1/15/2023|10\n1/10/2024|40");
}

#[test]
fn show_report_filter_pages() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
        p.add_field(YEAR, Area::Filters, None, Locale::en());
    });
    assert_eq!(wb.pivot_filter_pages(id, REGION), Err(PivotError::NoSuchField));
    let pages = wb.pivot_filter_pages(id, YEAR).unwrap();
    wb.commit("pages");
    // One sheet per year, before the PivotTable's own.
    assert_eq!(pages, [1, 2]);
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Sheet1", "2023", "2024", "Sheet2"]);
    assert_eq!(
        cells(&wb, 1, Rect::new(0, 0, 5, 1)),
        "Year|2023\n|\nRow Labels|Sum of Sales\nEast|250\nWest|200\nGrand Total|450"
    );
    assert_eq!(
        cells(&wb, 2, Rect::new(0, 0, 5, 1)),
        "Year|2024\n|\nRow Labels|Sum of Sales\nEast|120\nWest|120\nGrand Total|240"
    );
    let ids: Vec<u32> = wb.pivots().map(|(_, p)| p.id).collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|i| ids.iter().filter(|j| *j == i).count() == 1));
    // The original still shows every year.
    assert_eq!(cells(&wb, 3, Rect::new(0, 0, 0, 1)), "Year|(All)");
}

#[test]
fn compact_labels_are_aligned_left_and_headers_can_be_hidden() {
    use crate::style::HAlign;
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(YEAR, Area::Rows, None, Locale::en());
        p.add_field(REGION, Area::Columns, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    });
    // A5 is the year 2023: a number, on the left like every compact label.
    assert_eq!(at(&wb, id, "A5"), Value::Number(2023.0));
    assert_eq!(wb.style_at(1, 4, 0).h_align, HAlign::Left);
    assert_eq!(wb.style_at(1, 4, 1).h_align, HAlign::General);
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Tabular));
    assert_eq!(wb.style_at(1, 4, 0).h_align, HAlign::General);
    // The filter list of a field: in the order of its sort.
    let p = wb.pivot(id).unwrap().1;
    let labels = |p: &PivotTable| {
        p.items_in_order(REGION)
            .iter()
            .map(|x| p.item_text(REGION, *x, Locale::en()))
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(p), ["East", "West"]);
    assert!(p.is_used(YEAR) && p.is_used(SALES_F) && !p.is_used(PRODUCT));
    update(&mut wb, id, |p| {
        p.sort_field(REGION, SortOrder::Descending, None);
        p.options.show_headers = false;
    });
    assert_eq!(labels(wb.pivot(id).unwrap().1), ["West", "East"]);
    // "Field Headers" off: the captions are empty, the labels stay.
    assert_report(
        &wb,
        id,
        "Sum of Sales|||
         |West|East|Grand Total
         2023|200|250|450
         2024|120|120|240
         Grand Total|320|370|690",
    );
    assert_eq!(cell_at(&wb, "A4").part, PivotPart::FieldHeader);
}

#[test]
fn custom_captions() {
    let east = p_item(REGION, "East");
    let (mut wb, id) = region_by_year(Locale::en());
    update(&mut wb, id, |p| {
        // Names and labels must stay unique.
        assert!(!p.set_field_caption(REGION, "year"));
        assert!(!p.set_field_caption(REGION, "Sum of Sales"));
        assert!(p.set_field_caption(REGION, "Area"));
        assert!(!p.set_item_caption(REGION, east, "west", Locale::en()));
        assert!(p.set_item_caption(REGION, east, "Orient", Locale::en()));
        p.options.grand_total_caption = Some("All".to_string());
        p.options.row_header_caption = Some("Areas".to_string());
        p.options.col_header_caption = Some("Years".to_string());
        p.set_data_name(0, "Turnover");
        p.set_report_layout(FieldLayout::Outline);
    });
    assert_report(
        &wb,
        id,
        "Turnover|Year||
         Area|2023|2024|All
         Orient|250|120|370
         West|200|120|320
         All|450|240|690",
    );
    assert_eq!(wb.pivot(id).unwrap().1.field_index("area"), Some(REGION));
    assert_eq!(wb.pivot(id).unwrap().1.field_index("region"), Some(REGION));
    update(&mut wb, id, |p| p.set_report_layout(FieldLayout::Compact));
    assert_eq!(cells(&wb, 1, Rect::new(2, 0, 3, 1)), "Turnover|Years\nAreas|2023");
    // GETPIVOTDATA knows the items by their captions too.
    wb.set_cell_text(
        1,
        0,
        8,
        "=GETPIVOTDATA(\"Turnover\",A3,\"Area\",\"Orient\")+GETPIVOTDATA(\"Sales\",A3,\"Region\",\"East\")",
    )
    .unwrap();
    wb.commit("formula");
    assert_eq!(wb.value_at(1, 0, 8), Value::Number(740.0));
}

#[test]
fn a_hundred_thousand_records() {
    // Two row fields (5 x 40 items), one column field (10 items) and two
    // value fields over 100 000 records.
    const ROWS: u32 = 100_000;
    let mut wb = Workbook::new(Locale::en());
    for (c, name) in ["Region", "Product", "Year", "Sales", "Units"].iter().enumerate() {
        wb.set_value(0, 0, c as u32, Value::text(name));
    }
    let mut sales = 0.0;
    for r in 1..=ROWS {
        let k = r - 1;
        wb.set_value(0, r, 0, Value::text(format!("Region {}", k % 5)));
        wb.set_value(0, r, 1, Value::text(format!("Product {}", (k / 5) % 40)));
        wb.set_value(0, r, 2, Value::Number(2015.0 + (k / 200 % 10) as f64));
        wb.set_value(0, r, 3, Value::Number((k % 1000) as f64));
        wb.set_value(0, r, 4, Value::Number(1.0));
        sales += (k % 1000) as f64;
    }
    wb.commit("data");
    wb.clear_history();
    let source = PivotSource::Range {
        sheet: "Sheet1".to_string(),
        range: Rect::new(0, 0, ROWS, 4),
    };
    let id = wb
        .pivot_create(source, PivotDestination::NewSheet { index: 1 }, false)
        .unwrap();
    let en = Locale::en();
    wb.pivot_update(id, false, |p| {
        p.add_field(0, Area::Rows, None, en);
        p.add_field(1, Area::Rows, None, en);
        p.add_field(2, Area::Columns, None, en);
        p.add_field(3, Area::Values, None, en);
        p.add_field(4, Area::Values, None, en);
    })
    .unwrap();
    wb.commit("pivot");
    let started = std::time::Instant::now();
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("refresh");
    let elapsed = started.elapsed();
    let p = wb.pivot(id).unwrap().1;
    // 3 header rows, 5 regions, 5 x 40 products and the grand total; the
    // labels, 10 years and the grand total, two values each.
    assert_eq!(p.report.size(), (3 + 5 + 200 + 1, 1 + 20 + 2));
    let total = p.location.unwrap();
    assert_eq!(wb.value_at(1, total.r1, total.c1 - 1), Value::Number(sales));
    assert_eq!(wb.value_at(1, total.r1, total.c1), Value::Number(ROWS as f64));
    // Each region has a fifth of the records, each product of a region a
    // fortieth of those.
    assert_eq!(wb.value_at(1, total.r0 + 3, total.c1), Value::Number(20_000.0));
    assert_eq!(wb.value_at(1, total.r0 + 4, total.c1), Value::Number(500.0));
    // Refreshing must stay well under a second where it matters: in an
    // optimised build (see also `examples/pivot_bench.rs`).
    if !cfg!(debug_assertions) {
        assert!(elapsed.as_millis() < 500, "refresh took {elapsed:?}");
    }
}

#[test]
fn filters_and_sorts_work_on_the_columns_too() {
    let (mut wb, id) = region_by_year(Locale::en());
    // Years by their totals, smallest first: 2024 (240), 2023 (450).
    update(&mut wb, id, |p| p.sort_field(YEAR, SortOrder::Ascending, Some(0)));
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 1, 4, 3)),
        "2024|2023|Grand Total\n120|250|370"
    );
    // Only the years over 300.
    update(&mut wb, id, |p| {
        let test = ValueFilter::GreaterThan(300.0);
        p.set_value_filter(YEAR, Some(FilterKind::Value { data_field: 0, test }));
    });
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels|
         Row Labels|2023|Grand Total
         East|250|250
         West|200|200
         Grand Total|450|450",
    );
}

#[test]
fn running_totals_below_the_base_field() {
    // Rows: Region, then Product. Running along the regions, each product
    // adds up with the same product of the regions before.
    let (mut wb, id) = region_product(Locale::en());
    update(&mut wb, id, |p| {
        p.data_fields[0].show_as = ShowAs::RunningTotal { field: REGION }
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|370
         Apples|220
         Pears|150
         West|690
         Apples|420
         Pears|270
         Grand Total|",
    );
    // Along the products, within each region.
    update(&mut wb, id, |p| {
        p.data_fields[0].show_as = ShowAs::RunningTotal { field: PRODUCT }
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|
         Apples|220
         Pears|370
         West|
         Apples|200
         Pears|320
         Grand Total|",
    );
    update(&mut wb, id, |p| {
        p.data_fields[0].show_as = ShowAs::Difference {
            field: REGION,
            item: BaseItem::Previous,
        }
    });
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of Sales
         East|
         Apples|
         Pears|
         West|-50
         Apples|-20
         Pears|-30
         Grand Total|",
    );
}

#[test]
fn deleting_sheets() {
    let (mut wb, id) = region_by_year(Locale::en());
    let report = shown(&wb, id);
    // Without its source the PivotTable keeps what it shows.
    wb.delete_sheet(0);
    wb.commit("delete");
    assert_eq!(shown(&wb, id), report);
    assert_eq!(wb.pivot_refresh(id, false), Err(PivotError::InvalidSource));
    assert_eq!(wb.pivot_refresh_all(false), Err(PivotError::InvalidSource));
    assert_eq!(shown(&wb, id), report);
    // The cache still answers: the layout can change.
    update(&mut wb, id, |p| p.remove_field(YEAR));
    assert_eq!(at(&wb, id, "B6"), Value::Number(690.0));
    wb.undo();
    wb.undo();
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(wb.pivot_refresh(id, false), Ok(()));
    // The sheet of the PivotTable takes it along.
    wb.delete_sheet(1);
    assert!(wb.pivot(id).is_none());
    assert_eq!(wb.pivot_update(id, false, |_| {}), Err(PivotError::NoSuchPivot));
}

#[test]
fn a_cache_built_by_hand() {
    // What reading a file amounts to: a cache with a date grouping whose
    // members are not worked out yet, and a definition pushed on the sheet.
    let jan = crate::datetime::date_to_serial(2024, 1, 20).unwrap();
    let mar = crate::datetime::date_to_serial(2024, 3, 2).unwrap();
    let general: Arc<str> = Arc::from("General");
    let mut cache = PivotCache::from_columns(
        vec![
            ("Day".to_string(), Arc::from("m/d/yyyy")),
            ("N".to_string(), general.clone()),
        ],
        vec![
            vec![Value::Number(jan), Value::Number(mar), Value::Number(jan)],
            vec![1.0.into(), 2.0.into(), 4.0.into()],
        ],
    );
    assert_eq!((cache.rows, cache.fields[0].shared.len()), (3, 2));
    cache.fields.push(CacheField {
        name: "Months".to_string(),
        shared: Vec::new(),
        records: Vec::new(),
        num_fmt: general,
        group: Some(FieldGroup {
            base: Some(0),
            kind: GroupKind::Range {
                by: GroupBy::Months,
                start: jan,
                end: mar,
                interval: 1.0,
                auto_start: true,
                auto_end: true,
            },
            // The file's own labels, in another language.
            items: [
                "<", "janv", "févr", "mars", "avr", "mai", "juin", "juil", "août", "sept", "oct", "nov", "déc", ">",
            ]
            .iter()
            .map(Value::text)
            .collect(),
            map: Vec::new(),
        }),
        formula: None,
    });
    assert!(!cache.is_valid());
    cache.complete_groups(Locale::en());
    assert!(cache.is_valid());
    assert_eq!(cache.fields[2].group.as_ref().unwrap().map, [1, 3]);
    let mut wb = Workbook::new(Locale::en());
    let mut p = PivotTable::new(7, "Loaded".to_string(), PivotSource::Invalid, Arc::new(cache), (1, 1));
    p.row_fields = vec![AxisField::Field(2)];
    p.data_fields = vec![DataField {
        field: 1,
        name: "Total".to_string(),
        function: Aggregation::Sum,
        show_as: ShowAs::Normal,
        num_fmt: None,
    }];
    wb.sheets[0].pivots.push(p);
    wb.pivot_update(7, true, |_| {}).unwrap();
    wb.clear_history();
    assert_eq!(
        cells(&wb, 0, Rect::new(1, 1, 4, 2)),
        "Row Labels|Total\njanv|5\nmars|2\nGrand Total|7"
    );
    // A cache that contradicts itself is refused rather than trusted.
    let mut broken = (*wb.pivot(7).unwrap().1.cache).clone();
    broken.fields[1].records[0] = 99;
    assert_eq!(
        wb.pivot_update(7, true, |p| p.cache = Arc::new(broken)),
        Err(PivotError::InvalidSource)
    );
}

/// Thousands of random commands in a row, on text and numbers and on
/// dates: whatever the layout, the report is computed, written and
/// described cell by cell without a hitch, and what it occupies is what it
/// wrote.
#[test]
fn random_commands_keep_the_pivot_sound() {
    random_commands(sales(Locale::en()), 0x9E37_79B9_7F4A_7C15);
    random_commands(pivot_on(DATED, Locale::en()), 0x2545_F491_4F6C_DD1D);
}

fn random_commands((mut wb, id): (Workbook, u32), mut seed: u64) {
    let en = Locale::en();
    wb.set_cell_text(1, 0, 30, "=GETPIVOTDATA(\"Sales\",$A$3)").unwrap();
    wb.commit("formula");
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    let areas = [Area::Filters, Area::Columns, Area::Rows, Area::Values];
    let layouts = [FieldLayout::Compact, FieldLayout::Outline, FieldLayout::Tabular];
    let orders = [SortOrder::Manual, SortOrder::Ascending, SortOrder::Descending];
    for step in 0..2000 {
        let p = wb.pivot(id).unwrap().1;
        let nf = p.fields.len();
        let (f, g) = (next(nf), next(nf));
        let item = next(p.cache.items(f).len()) as u32;
        let nd = p.data_fields.len();
        let (a, b, c, d) = (next(4), next(4), next(4), next(3));
        let flag = next(2) == 0;
        let command = next(24);
        let result = match command {
            0 => wb.pivot_group_items(id, f, &[item, next(4) as u32], true).map(|_| ()),
            1 => wb.pivot_ungroup(id, f, flag.then_some(item), true),
            2 => wb.pivot_group_numbers(id, f, None, None, 1.0 + next(100) as f64, true),
            3 => wb.pivot_add_field(id, f, areas[a], flag.then_some(b), true),
            4 => wb.pivot_refresh(id, true),
            _ => wb.pivot_update(id, true, |p| match command {
                5..=7 => p.add_field(f, areas[a], flag.then_some(b), en),
                8 => {
                    p.move_entry(areas[a], b, areas[c], d, en);
                }
                9 => p.remove_field(f),
                10 => p.remove_data_field(b),
                11 => p.set_report_layout(layouts[d]),
                12 => p.fields[f].layout = layouts[d],
                13 => p.set_subtotals(flag, a < 2),
                14 => p.fields[f].subtotals = Subtotals::Custom(vec![Aggregation::ALL[next(11)], Aggregation::Count]),
                15 => {
                    p.set_repeat_labels(flag);
                    p.fields[g].blank_line = a == 0;
                    p.fields[g].show_all = b == 0;
                }
                16 => {
                    p.options.row_grand_totals = flag;
                    p.options.col_grand_totals = a != 0;
                    p.options.page_wrap = b as u32;
                    p.options.page_over_then_down = c < 2;
                }
                17 => p.set_item_hidden(f, item, flag),
                18 => p.set_collapsed(f, item, flag),
                19 => p.sort_field(f, orders[d], flag.then_some(b)),
                20 if nd > 0 => {
                    let k = next(nd);
                    let base = BaseItem::Item(item);
                    p.data_fields[k].function = Aggregation::ALL[next(11)];
                    p.data_fields[k].show_as = [
                        ShowAs::Normal,
                        ShowAs::PercentOfTotal,
                        ShowAs::PercentOfColumn,
                        ShowAs::PercentOfRow,
                        ShowAs::PercentOf { field: f, item: base },
                        ShowAs::PercentOfParentRow,
                        ShowAs::PercentOfParentColumn,
                        ShowAs::PercentOfParent { field: f },
                        ShowAs::Difference {
                            field: f,
                            item: BaseItem::Previous,
                        },
                        ShowAs::PercentDifference {
                            field: f,
                            item: BaseItem::Next,
                        },
                        ShowAs::RunningTotal { field: f },
                        ShowAs::PercentRunningTotal { field: f },
                        ShowAs::RankAscending { field: f },
                        ShowAs::RankDescending { field: f },
                        ShowAs::Index,
                    ][next(15)];
                }
                21 => p.set_label_filter(f, flag.then(|| LabelFilter::Contains("a".into()))),
                22 => {
                    let test = ValueFilter::GreaterThan(next(300) as f64);
                    let kind = match a {
                        0 => None,
                        1 => Some(FilterKind::Value { data_field: b, test }),
                        _ => Some(FilterKind::Top10 {
                            data_field: b,
                            top: flag,
                            count: 1.0 + d as f64,
                            by: [Top10By::Items, Top10By::Percent, Top10By::Sum][d],
                        }),
                    };
                    p.set_value_filter(f, kind);
                }
                23 => p.set_page_item(f, flag.then_some(item)),
                _ => {}
            }),
        };
        // A refused command (nothing to group, a report too wide for the
        // sheet) leaves everything as it was.
        if result.is_err() {
            assert!(!wb.commit("nothing"), "step {step}: {result:?} left a change behind");
            continue;
        }
        wb.commit("step");
        let (si, p) = wb.pivot(id).unwrap();
        assert!(p.cache.is_valid(), "step {step}");
        let body = p.location.unwrap();
        assert_eq!((body.rows(), body.cols()), p.report.size(), "step {step}");
        for rect in p.footprint() {
            for r in rect.r0..=rect.r1 {
                for c in rect.c0..=rect.c1 {
                    let cell = wb.pivot_cell(si, r, c).unwrap();
                    assert_eq!(cell.pivot, id);
                    let _ = style_patch_at(&wb, si, r, c);
                    let _ = wb.display(si, r, c);
                }
            }
        }
        // Nothing of the PivotTable lies outside what it says it occupies.
        let stray = wb.sheets[si].cells.keys().find(|(r, c)| !p.contains(*r, *c) && *c < 30);
        assert_eq!(stray, None, "step {step}");
        if step % 100 == 0 {
            wb.undo();
            wb.redo();
        }
    }
}

#[test]
fn groups_of_groups() {
    let rows: &[[&str; 2]] = &[
        ["City", "N"],
        ["Lyon", "1"],
        ["Paris", "2"],
        ["Rome", "4"],
        ["Turin", "8"],
    ];
    let (mut wb, id) = pivot_on(rows, Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(1, Area::Values, None, Locale::en());
    });
    let item = |wb: &Workbook, field: usize, label: &str| {
        let p = wb.pivot(id).unwrap().1;
        (0..p.cache.items(field).len() as u32)
            .find(|x| p.item_text(field, *x, wb.locale) == label)
            .unwrap()
    };
    // Lyon and Paris make Group1; Group1 and Rome make Group2, one field up.
    let city2 = wb
        .pivot_group_items(id, 0, &[item(&wb, 0, "Lyon"), item(&wb, 0, "Paris")], false)
        .unwrap();
    let members = [item(&wb, city2, "Group1"), item(&wb, city2, "Rome")];
    let city3 = wb.pivot_group_items(id, city2, &members, false).unwrap();
    wb.commit("group");
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(p.cache.fields[city3].name, "City3");
    assert_eq!(
        p.row_fields,
        [AxisField::Field(city3), AxisField::Field(city2), AxisField::Field(0)]
    );
    assert_report(
        &wb,
        id,
        "Row Labels|Sum of N
         Group2|7
         Group1|3
         Lyon|1
         Paris|2
         Rome|4
         Rome|4
         Turin|8
         Turin|8
         Turin|8
         Grand Total|15",
    );
    // Rome and Turin become a group of the first field: the groups made
    // of its former groups go.
    let group1 = item(&wb, city2, "Group1");
    let more = [item(&wb, 0, "Rome"), item(&wb, 0, "Turin")];
    assert_eq!(wb.pivot_group_items(id, 0, &more, false), Ok(city2));
    wb.commit("group");
    let p = wb.pivot(id).unwrap().1;
    assert!(p.cache.is_valid());
    assert_eq!(p.cache.fields.len(), 3);
    assert_eq!(p.row_fields, [AxisField::Field(city2), AxisField::Field(0)]);
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 0, 8, 1)),
        "Group1|3\nLyon|1\nParis|2\nGroup2|12\nRome|4\nTurin|8"
    );
    // Dissolving both groups leaves the cities alone.
    wb.pivot_ungroup(id, city2, Some(group1), false).unwrap();
    wb.pivot_ungroup(id, city2, None, false).unwrap();
    wb.commit("ungroup");
    let p = wb.pivot(id).unwrap().1;
    assert_eq!(
        (p.cache.fields.len(), p.row_fields.as_slice()),
        (2, &[AxisField::Field(0)][..])
    );
    assert_eq!(
        cells(&wb, 1, Rect::new(3, 0, 7, 1)),
        "Lyon|1\nParis|2\nRome|4\nTurin|8\nGrand Total|15"
    );
}

// ---- calculated fields and items ----------------------------------------------

#[test]
fn a_calculated_field_applies_to_the_sums() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| p.add_field(REGION, Area::Rows, None, Locale::en()));
    // Sales per unit: 370 / 27 in the East, not the sum of the six ratios.
    let field = wb.pivot_set_calculated_field(id, "Price", "=Sales/Units", false).unwrap();
    wb.commit("field");
    assert_eq!(field, 5);
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!(p.cache.fields[5].formula.as_deref(), Some("Sales/Units"));
    assert_eq!(p.data_fields[0].name, "Sum of Price");
    assert_eq!(at(&wb, id, "A4"), Value::text("East"));
    assert_close(at(&wb, id, "B4"), 370.0 / 27.0);
    assert_close(at(&wb, id, "B5"), 320.0 / 29.0);
    assert_close(at(&wb, id, "B6"), 690.0 / 56.0);
    // Modify, with a function and a field computed from another one.
    wb.pivot_set_calculated_field(id, "price", "=ROUND(Sales/Units;0)", false).unwrap_err();
    wb.pivot_set_calculated_field(id, "price", "=ROUND(Sales/Units,0)", false).unwrap();
    wb.pivot_set_calculated_field(id, "Double price", "=Price*2", false).unwrap();
    wb.commit("more");
    assert_close(at(&wb, id, "B4"), 14.0);
    assert_close(at(&wb, id, "C4"), 28.0);
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!(p.cache.fields[6].formula.as_deref(), Some("Price*2"));
    assert_eq!(
        p.calculated_fields(Locale::fr()),
        [(5, "Price".to_string(), "ARRONDI(Sales/Units;0)".to_string()), (6, "Double price".to_string(), "Price*2".to_string())]
    );
    // What Excel refuses.
    assert_eq!(wb.pivot_set_calculated_field(id, "Sales", "=1", false), Err(PivotError::FormulaNameTaken));
    assert_eq!(wb.pivot_set_calculated_field(id, "X", "=A1*2", false), Err(PivotError::FormulaReference));
    assert_eq!(wb.pivot_set_calculated_field(id, "X", "=Nothing+1", false), Err(PivotError::FormulaReference));
    assert_eq!(wb.pivot_set_calculated_field(id, "X", "=RAND()", false), Err(PivotError::FormulaFunction));
    assert_eq!(wb.pivot_set_calculated_field(id, "X", "=Sales+", false), Err(PivotError::BadFormula));
    assert_eq!(wb.pivot_set_calculated_field(id, "Price", "='Double price'/2", false), Err(PivotError::FormulaCircular));
    // A calculated field is a value field only, and survives a refresh.
    update(&mut wb, id, |p| p.add_field(5, Area::Rows, None, Locale::en()));
    assert_eq!(wb.pivot(id).unwrap().1.row_fields.len(), 1);
    wb.set_cell_text(0, 1, 4, "20").unwrap();
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("refresh");
    assert_close(at(&wb, id, "B4"), 10.0);
    // Division by nothing shows as in a cell.
    wb.pivot_set_calculated_field(id, "Price", "=Sales/(Units-Units)", false).unwrap();
    assert_eq!(at(&wb, id, "B4"), Value::Error(ErrorKind::Div0));
    // Deleting takes it out of the values; undo brings it all back.
    wb.pivot_delete_calculated_field(id, 6, false).unwrap();
    wb.pivot_delete_calculated_field(id, 5, false).unwrap();
    wb.commit("delete");
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!((p.cache.fields.len(), p.data_fields.len()), (5, 0));
    assert!(wb.undo());
    assert_eq!(wb.pivot(id).unwrap().1.data_fields.len(), 2);
}

#[test]
fn names_in_formulas_are_quoted_when_they_must() {
    let rows = [["Unit price", "Q1", "Sold"], ["2", "5", "10"], ["3", "7", "20"]];
    let (mut wb, id) = pivot_on(&rows, Locale::en());
    let field = wb.pivot_set_calculated_field(id, "Turnover", "='unit price'*Sold+q1", false).unwrap();
    wb.commit("field");
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!(p.cache.fields[field].formula.as_deref(), Some("'Unit price'*Sold+'Q1'"));
    // (2 + 3) * 30 + 12, on the only row of a report without row fields.
    assert_close(at(&wb, id, "A4"), 162.0);
}

#[test]
fn a_calculated_item_computes_from_the_items_of_its_field() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| {
        p.add_field(REGION, Area::Rows, None, Locale::en());
        p.add_field(YEAR, Area::Columns, None, Locale::en());
        p.add_field(SALES_F, Area::Values, None, Locale::en());
    });
    // A new item goes after the others, as in Excel.
    let item = wb.pivot_set_calculated_item(id, REGION, "Gap", "=East-West", false).unwrap();
    wb.commit("item");
    assert_eq!(item, 2);
    assert!(wb.pivot(id).unwrap().1.cache.is_calc_item(REGION, 2));
    assert_report(
        &wb,
        id,
        "Sum of Sales|Column Labels||
         Row Labels|2023|2024|Grand Total
         East|250|120|370
         West|200|120|320
         Gap|50|0|50
         Grand Total|500|240|740",
    );
    // A second item computed from the first; the later one in solve order
    // gives the value where two formulas meet.
    wb.pivot_set_calculated_item(id, REGION, "Twice", "=Gap*2", false).unwrap();
    wb.pivot_set_calculated_item(id, YEAR, "Both", "='2023'+'2024'", false).unwrap();
    wb.commit("more");
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!(p.solve_order(Locale::en()), ["Gap = East-West", "Twice = Gap*2", "Both = '2023'+'2024'"]);
    assert_eq!(p.calculated_items(REGION, Locale::en()).len(), 2);
    // Row "Twice", column "Both": (2023 + 2024) of Twice = 100.
    assert_eq!(shown(&wb, id).lines().nth(5).unwrap(), "Twice|100|0|100|200");
    assert_eq!(wb.pivot_set_calculated_item(id, REGION, "East", "=1", false), Err(PivotError::FormulaNameTaken));
    assert_eq!(wb.pivot_set_calculated_item(id, REGION, "Gap", "=Gap+1", false), Err(PivotError::FormulaCircular));
    assert_eq!(wb.pivot_set_calculated_item(id, REGION, "N", "=Apples+1", false), Err(PivotError::FormulaReference));
    // Excel's restrictions.
    update(&mut wb, id, |p| p.set_function(0, Aggregation::Average, Locale::en()));
    assert_eq!(wb.pivot_set_calculated_item(id, REGION, "N", "=East", false), Err(PivotError::ItemsWithAverages));
    update(&mut wb, id, |p| p.set_function(0, Aggregation::Sum, Locale::en()));
    // Unticked, a calculated item leaves the report and its totals.
    update(&mut wb, id, |p| {
        p.set_item_hidden(REGION, 3, true);
        p.set_item_hidden(YEAR, 2, true);
    });
    assert_eq!(shown(&wb, id).lines().last().unwrap(), "Grand Total|500|240|740");
    // Deleting an item, and refreshing, keep the others.
    wb.pivot_delete_calculated_item(id, REGION, 3, false).unwrap();
    wb.pivot_refresh(id, false).unwrap();
    wb.commit("delete");
    let (_, p) = wb.pivot(id).unwrap();
    assert_eq!(p.solve_order(Locale::en()), ["Gap = East-West", "Both = '2023'+'2024'"]);
    assert_eq!(p.cache.items(REGION).len(), 3);
    wb.pivot_set_solve_order(id, &[1, 0], false).unwrap();
    assert_eq!(wb.pivot(id).unwrap().1.solve_order(Locale::en())[0], "Both = '2023'+'2024'");
}

#[test]
fn list_formulas_writes_a_sheet() {
    let (mut wb, id) = sales(Locale::en());
    update(&mut wb, id, |p| p.add_field(REGION, Area::Rows, None, Locale::en()));
    wb.pivot_set_calculated_field(id, "Price", "=Sales/Units", false).unwrap();
    wb.pivot_set_calculated_item(id, REGION, "All of it", "=East+West", false).unwrap();
    let labels = ListLabels {
        calculated_field: "Calculated Field".into(),
        calculated_item: "Calculated Item".into(),
        solve_order: "Solve Order".into(),
        field: "Field".into(),
        item: "Item".into(),
        formula: "Formula".into(),
        note: vec!["Note:".into()],
    };
    let sheet = wb.pivot_list_formulas(id, &labels).unwrap();
    wb.commit("list");
    assert_eq!(sheet, 1);
    assert_eq!(
        cells(&wb, sheet, Rect::new(0, 0, 9, 2)),
        "Calculated Field||\nSolve Order|Field|Formula\n1|Price|=Sales/Units\n||\nCalculated Item||\nSolve Order|Item|Formula\n1|'All of it'|=East+West\n||\n||\nNote:||"
    );
}
