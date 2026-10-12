//! PivotTables in `.xlsx` files: what the writer produces, that it comes
//! back unchanged, and what is made of the parts Excel writes.

mod common;

use common::{assert_well_formed, at, enter, package, part, part_names, value};
use opensheet_engine::pivot::{
    Aggregation, Area, AxisField, BaseItem, FieldLayout, FilterKind, GroupBy, GroupKind,
    LabelFilter, PivotDestination, PivotSource, PivotTable, ShowAs, SortOrder, Subtotals, Top10By,
    ValueFilter,
};
use opensheet_engine::{Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Set `OPENSHEET_PIVOT_SAMPLES` to a directory to keep the packages these
/// tests build, to open them in Excel.
fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_PIVOT_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

/// Region, Product, Date, Sales, Units, Note.
const SALES: &[[&str; 6]] = &[
    ["Region", "Product", "Date", "Sales", "Units", "Note"],
    ["East", "Apples", "1/15/2023", "100", "10", "ok"],
    ["East", "Pears", "3/2/2023", "150.5", "5", ""],
    ["East", "Apples", "7/20/2024", "120", "12", "late"],
    ["West", "Apples", "11/5/2023", "200", "20", "ok"],
    ["West", "Pears", "2/14/2024", "50", "2", "ok"],
    ["West", "Pears", "8/30/2024", "70", "7", ""],
    ["North", "Plums", "5/5/2024", "35", "3", "late"],
    ["North", "Apples", "12/24/2024", "64.25", "8", "ok"],
    ["South & <co>", "Plums", "6/1/2023", "90", "9", "ok"],
    ["South & <co>", "Pears", "9/9/2024", "110", "11", "7"],
    ["east", "Plums", "10/10/2024", "40", "4", "ok"],
    ["West", "Apples", "4/1/2025", "15", "1", "ok"],
];

const REGION: usize = 0;
const PRODUCT: usize = 1;
const DATE: usize = 2;
const SALES_F: usize = 3;
const UNITS: usize = 4;
const NOTE: usize = 5;

fn update(wb: &mut Workbook, id: u32, edit: impl FnOnce(&mut PivotTable)) {
    wb.pivot_update(id, false, edit)
        .unwrap_or_else(|e| panic!("pivot {id}: {e:?}"));
}

fn pivot(wb: &Workbook, id: u32) -> &PivotTable {
    wb.pivot(id).unwrap_or_else(|| panic!("no pivot {id}")).1
}

/// The PivotTable called `name`.
fn named<'a>(wb: &'a Workbook, name: &str) -> (usize, &'a PivotTable) {
    wb.pivots()
        .find(|(_, p)| p.name == name)
        .unwrap_or_else(|| panic!("no pivot {name}"))
}

/// The item of a field whose label is `label`.
fn item(wb: &Workbook, id: u32, field: usize, label: &str) -> u32 {
    let p = pivot(wb, id);
    (0..p.cache.items(field).len() as u32)
        .find(|x| p.item_text(field, *x, wb.locale) == label)
        .unwrap_or_else(|| panic!("no item {label}"))
}

/// The cells of a range as the grid shows them: rows separated by line
/// breaks, cells by `|`.
fn shown(wb: &Workbook, sheet: usize, rect: Rect) -> String {
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

/// A workbook with PivotTables of every sort on two sheets: three caches
/// on ranges (one of them shared by three PivotTables), one on a table.
fn sample(locale: &'static Locale) -> Workbook {
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Data"));
    assert!(wb.rename_sheet(1, "Pivots"));
    assert!(wb.rename_sheet(2, "More 'pivots'"));
    for (r, row) in SALES.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if !text.is_empty() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
    }
    wb.update_style(0, rect("D2:D13"), &|s| s.num_fmt = Arc::from("#,##0.00"));
    for (r, row) in [
        ["Item", "Bin", "Qty"],
        ["nut", "A", "5"],
        ["bolt", "B", "50"],
        ["nut", "B", "7"],
        ["gear", "A", "8"],
    ]
    .iter()
    .enumerate()
    {
        for (c, text) in row.iter().enumerate() {
            wb.set_cell_text(0, r as u32, 8 + c as u32, text).unwrap();
        }
    }
    let stock = wb.add_table(0, rect("I1:K5"), true).unwrap();
    wb.rename_table(0, stock, "Stock").unwrap();
    wb.commit("data");
    // The workbook speaks `locale` from here on: captions and the names of
    // value fields are in its language.
    wb.locale = locale;

    let source = |range: &str| PivotSource::Range {
        sheet: "Data".to_string(),
        range: rect(range),
    };
    let on = |sheet: usize, cell: &str| {
        let (row, col) = at(cell);
        PivotDestination::Existing { sheet, row, col }
    };

    // 1: one row field, one value field.
    let basic = wb
        .pivot_create(source("A1:F13"), on(1, "A3"), false)
        .unwrap();
    wb.pivot_add_field(basic, REGION, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(basic, SALES_F, Area::Values, None, false)
        .unwrap();

    // 2: on the same cache. Two row fields in compact form, a column
    // field, two value fields, a filter field showing one item.
    let cross = wb
        .pivot_create(source("A1:F13"), on(1, "A16"), false)
        .unwrap();
    wb.pivot_add_field(cross, REGION, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(cross, PRODUCT, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(cross, NOTE, Area::Columns, None, false)
        .unwrap();
    wb.pivot_add_field(cross, SALES_F, Area::Values, None, false)
        .unwrap();
    wb.pivot_add_field(cross, UNITS, Area::Values, None, false)
        .unwrap();
    let apples = item(&wb, cross, PRODUCT, "Apples");
    let west = item(&wb, cross, REGION, "West");
    update(&mut wb, cross, |p| {
        p.name = "Cross & <tab>".to_string();
        p.set_function(1, Aggregation::Count, locale);
        p.data_fields[0].num_fmt = Some(Arc::from("#,##0.00"));
        p.set_collapsed(REGION, west, true);
        p.set_item_hidden(PRODUCT, apples, true);
        p.options.grand_total_caption = Some("All of it".to_string());
        p.options.missing_caption = "-".to_string();
        p.style.name = "PivotStyleMedium9".to_string();
        p.style.row_stripes = true;
    });

    // 3: tabular form with everything turned on: custom subtotals, blank
    // lines, repeated labels, captions, sorts, filters of the three
    // kinds, the calculations of "Show Values As".
    let full = wb
        .pivot_create(source("A1:F13"), on(1, "P3"), false)
        .unwrap();
    wb.pivot_add_field(full, PRODUCT, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(full, REGION, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(full, NOTE, Area::Filters, None, false)
        .unwrap();
    for _ in 0..4 {
        wb.pivot_add_field(full, SALES_F, Area::Values, None, false)
            .unwrap();
    }
    wb.pivot_add_field(full, UNITS, Area::Values, None, false)
        .unwrap();
    let east = item(&wb, full, REGION, "East");
    let late = item(&wb, full, NOTE, "late");
    update(&mut wb, full, |p| {
        p.name = "Everything".to_string();
        p.set_report_layout(FieldLayout::Tabular);
        p.fields[PRODUCT].subtotals =
            Subtotals::Custom(vec![Aggregation::Max, Aggregation::Sum, Aggregation::Varp]);
        p.fields[PRODUCT].blank_line = true;
        p.fields[PRODUCT].repeat_labels = true;
        p.fields[PRODUCT].show_all = true;
        p.fields[PRODUCT].subtotal_top = false;
        p.set_field_caption(PRODUCT, "Fruit");
        p.set_item_caption(REGION, east, "Orient", locale);
        p.sort_field(PRODUCT, SortOrder::Descending, Some(0));
        p.sort_field(REGION, SortOrder::Ascending, None);
        p.fields[REGION].subtotals = Subtotals::None;
        p.fields[REGION].num_fmt = Some(Arc::from("@"));
        p.set_page_item(NOTE, None);
        p.set_item_hidden(NOTE, late, true);
        p.data_fields[1].show_as = ShowAs::PercentOfTotal;
        p.data_fields[2].show_as = ShowAs::RunningTotal { field: REGION };
        p.data_fields[3].show_as = ShowAs::PercentOfParentRow;
        p.data_fields[4].show_as = ShowAs::Difference {
            field: REGION,
            item: BaseItem::Item(east),
        };
        p.data_fields[4].function = Aggregation::Average;
        p.data_fields[4].num_fmt = Some(Arc::from("0.0 \"u\""));
        p.set_data_name(2, "Running");
        p.set_label_filter(REGION, Some(LabelFilter::NotContains("zz".to_string())));
        p.set_value_filter(
            PRODUCT,
            Some(FilterKind::Top10 {
                data_field: 0,
                top: true,
                count: 3.0,
                by: Top10By::Items,
            }),
        );
        p.options.row_grand_totals = false;
        p.options.show_error = true;
        p.options.error_caption = "n/a".to_string();
        p.options.show_missing = false;
        p.options.row_header_caption = Some("Rows".to_string());
        p.options.col_header_caption = Some("Columns".to_string());
        p.options.data_caption = Some("Figures".to_string());
        p.options.show_drill = false;
        p.style.name = String::new();
        p.style.col_headers = false;
        p.style.col_stripes = true;
    });

    // 4: dates grouped by years, quarters and months, on a cache of its
    // own; outline form; values moved to the rows.
    let dates = wb
        .pivot_create(source("A1:E13"), on(2, "A3"), false)
        .unwrap();
    wb.pivot_add_field(dates, DATE, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(dates, SALES_F, Area::Values, None, false)
        .unwrap();
    wb.pivot_add_field(dates, UNITS, Area::Values, None, false)
        .unwrap();
    update(&mut wb, dates, |p| {
        p.name = "Dates".to_string();
        p.set_report_layout(FieldLayout::Outline);
        let values = p
            .col_fields
            .iter()
            .position(|a| *a == AxisField::Values)
            .unwrap();
        assert!(p.move_entry(Area::Columns, values, Area::Rows, 1, locale));
        p.options.col_grand_totals = false;
        p.set_value_filter(
            DATE,
            Some(FilterKind::Value {
                data_field: 0,
                test: ValueFilter::Between(1.0, 100_000.5),
            }),
        );
    });

    // 5: numbers grouped by interval and items grouped by hand, on a
    // third cache; filter fields laid out over, then down.
    let groups = wb
        .pivot_create(source("A1:D13"), on(2, "H5"), false)
        .unwrap();
    wb.pivot_add_field(groups, REGION, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(groups, SALES_F, Area::Columns, None, false)
        .unwrap();
    wb.pivot_add_field(groups, PRODUCT, Area::Filters, None, false)
        .unwrap();
    wb.pivot_add_field(groups, SALES_F, Area::Values, None, false)
        .unwrap();
    wb.pivot_group_numbers(groups, SALES_F, Some(0.0), None, 50.0, false)
        .unwrap();
    let picked = [
        item(&wb, groups, REGION, "North"),
        item(&wb, groups, REGION, "West"),
    ];
    wb.pivot_group_items(groups, REGION, &picked, false)
        .unwrap();
    let pears = item(&wb, groups, PRODUCT, "Pears");
    update(&mut wb, groups, |p| {
        p.name = "Groups".to_string();
        p.set_page_item(PRODUCT, Some(pears));
        p.set_function(0, Aggregation::Count, locale);
        p.options.page_over_then_down = true;
        p.options.page_wrap = 2;
        p.options.show_headers = false;
    });

    // 6 and 7: on a table, and a PivotTable nobody has put a field in,
    // sharing the cache of the first two.
    let on_table = wb
        .pivot_create(PivotSource::Name("Stock".to_string()), on(2, "T3"), false)
        .unwrap();
    wb.pivot_add_field(on_table, 1, Area::Columns, None, false)
        .unwrap();
    wb.pivot_add_field(on_table, 0, Area::Rows, None, false)
        .unwrap();
    wb.pivot_add_field(on_table, 2, Area::Values, None, false)
        .unwrap();
    update(&mut wb, on_table, |p| {
        p.set_function(0, Aggregation::Max, locale);
        p.data_fields[0].show_as = ShowAs::RankDescending { field: 0 };
    });
    wb.pivot_create(source("A1:F13"), on(2, "T25"), false)
        .unwrap();
    wb.commit("pivots");
    wb
}

/// The PivotTables of two workbooks are the same, whatever their ids.
fn assert_same_pivots(a: &Workbook, b: &Workbook) {
    assert_eq!(a.sheets.len(), b.sheets.len());
    for (index, (sa, sb)) in a.sheets.iter().zip(&b.sheets).enumerate() {
        assert_eq!(sa.pivots.len(), sb.pivots.len(), "{}", sa.name);
        for (pa, pb) in sa.pivots.iter().zip(&sb.pivots) {
            let mut pb = pb.clone();
            pb.id = pa.id;
            // Field by field first: a difference is easier to read.
            assert_eq!(pa.name, pb.name);
            assert_eq!(pa.source, pb.source, "{}", pa.name);
            assert_eq!(pa.cache.rows, pb.cache.rows, "{}", pa.name);
            for (fa, fb) in pa.cache.fields.iter().zip(&pb.cache.fields) {
                assert_eq!(fa, fb, "{}: cache field {}", pa.name, fa.name);
            }
            assert_eq!(pa.cache, pb.cache, "{}", pa.name);
            for (k, (fa, fb)) in pa.fields.iter().zip(&pb.fields).enumerate() {
                assert_eq!(fa, fb, "{}: field {k}", pa.name);
            }
            assert_eq!(pa.row_fields, pb.row_fields, "{}", pa.name);
            assert_eq!(pa.col_fields, pb.col_fields, "{}", pa.name);
            assert_eq!(pa.page_fields, pb.page_fields, "{}", pa.name);
            assert_eq!(pa.data_fields, pb.data_fields, "{}", pa.name);
            assert_eq!(pa.filters, pb.filters, "{}", pa.name);
            assert_eq!(pa.options, pb.options, "{}", pa.name);
            assert_eq!(pa.style, pb.style, "{}", pa.name);
            assert_eq!(pa.location, pb.location, "{}", pa.name);
            assert_eq!(pa.page_location, pb.page_location, "{}", pa.name);
            assert_eq!(*pa, pb, "{}", pa.name);
            // And the cells say the same.
            for area in pa.footprint() {
                assert_eq!(shown(a, index, area), shown(b, index, area), "{}", pa.name);
            }
        }
    }
}

#[test]
fn pivots_round_trip() {
    let original = sample(Locale::en());
    let names: Vec<&str> = original.pivots().map(|(_, p)| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "PivotTable1",
            "Cross & <tab>",
            "Everything",
            "Dates",
            "Groups",
            "PivotTable2",
            "PivotTable3"
        ]
    );
    let bytes = to_bytes(&original);
    keep("opensheet-pivots.xlsx", &bytes);
    let loaded = load(&bytes);
    assert_same_pivots(&original, &loaded);
    assert!(!loaded.modified && !loaded.can_undo());

    // PivotTables on the same source still share their cache, and only
    // those.
    let cache = |name: &str| named(&loaded, name).1.cache.clone();
    assert!(Arc::ptr_eq(&cache("PivotTable1"), &cache("Cross & <tab>")));
    assert!(Arc::ptr_eq(&cache("PivotTable1"), &cache("Everything")));
    assert!(Arc::ptr_eq(&cache("PivotTable1"), &cache("PivotTable3")));
    assert!(!Arc::ptr_eq(&cache("PivotTable1"), &cache("Dates")));
    assert!(!Arc::ptr_eq(&cache("Dates"), &cache("Groups")));

    // They can be refreshed, and changed.
    let mut changed = load(&bytes);
    enter(&mut changed, 0, "D2", "1100");
    changed.commit("edit");
    changed.pivot_refresh_all(true).unwrap();
    changed.commit("refresh");
    let (sheet, basic) = named(&changed, "PivotTable1");
    let total = basic.location.unwrap();
    assert_eq!(
        changed.value_at(sheet, total.r1, total.c1),
        Value::Number(2044.75)
    );
    assert_eq!(value(&loaded, 1, "B8"), Value::Number(1044.75));

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
    }
}
