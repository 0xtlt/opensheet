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

/// Keep a package these tests build, to open it in Excel (see
/// `common::keep_sample`).
fn keep(name: &str, bytes: &[u8]) {
    common::keep_sample(name, bytes);
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

// ---- parts written the way Excel writes them ------------------------------------

fn relationships(list: &[(&str, &str, &str)]) -> String {
    let mut x = format!("{DECL}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in list {
        x.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"
        ));
    }
    x.push_str("</Relationships>");
    x
}

const EXCEL_ROOT: &str = "xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"xr\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\"";

/// The cache of a PivotTable on Data!A1:D7 whose dates are grouped by
/// months, quarters and years: Excel groups the date field itself by the
/// smallest period and adds a field for each of the others. Numbers that
/// are on no axis are not listed, the records hold them.
fn excel_definition() -> String {
    let range = "startDate=\"2024-01-05T00:00:00\" endDate=\"2025-04-02T00:00:00\"";
    format!(
        r#"{DECL}<pivotCacheDefinition xmlns="{MAIN}" xmlns:r="{REL}" {EXCEL_ROOT} r:id="rId1" refreshedBy="Ada" refreshedDate="45800.5" createdVersion="8" refreshedVersion="8" minRefreshableVersion="3" recordCount="6" xr:uid="{{00000000-000A-0000-FFFF-FFFF00000000}}"><cacheSource type="worksheet"><worksheetSource ref="A1:D7" sheet="Data"/></cacheSource><cacheFields count="6"><cacheField name="Region" numFmtId="0"><sharedItems count="3"><s v="North"/><s v="South"/><s v="East"/></sharedItems></cacheField><cacheField name="Date" numFmtId="14"><sharedItems containsSemiMixedTypes="0" containsNonDate="0" containsDate="1" containsString="0" minDate="2024-01-05T00:00:00" maxDate="2025-04-01T00:00:00" count="6"><d v="2024-01-05T00:00:00"/><d v="2024-01-20T00:00:00"/><d v="2024-02-11T00:00:00"/><d v="2024-03-02T00:00:00"/><d v="2025-03-15T00:00:00"/><d v="2025-04-01T00:00:00"/></sharedItems><fieldGroup par="4" base="1"><rangePr groupBy="months" {range}/><groupItems count="14"><s v="&lt;1/5/2024"/><s v="Jan"/><s v="Feb"/><s v="Mar"/><s v="Apr"/><s v="May"/><s v="Jun"/><s v="Jul"/><s v="Aug"/><s v="Sep"/><s v="Oct"/><s v="Nov"/><s v="Dec"/><s v="&gt;4/2/2025"/></groupItems></fieldGroup></cacheField><cacheField name="Amount" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="10" maxValue="60"/></cacheField><cacheField name="Units" numFmtId="0"><sharedItems containsString="0" containsBlank="1" containsNumber="1" containsInteger="1" minValue="1" maxValue="6"/></cacheField><cacheField name="Quarters" numFmtId="0" databaseField="0"><fieldGroup par="5" base="1"><rangePr groupBy="quarters" {range}/><groupItems count="6"><s v="&lt;1/5/2024"/><s v="Qtr1"/><s v="Qtr2"/><s v="Qtr3"/><s v="Qtr4"/><s v="&gt;4/2/2025"/></groupItems></fieldGroup></cacheField><cacheField name="Years" numFmtId="0" databaseField="0"><fieldGroup base="1"><rangePr groupBy="years" {range}/><groupItems count="4"><s v="&lt;1/5/2024"/><s v="2024"/><s v="2025"/><s v="&gt;4/2/2025"/></groupItems></fieldGroup></cacheField></cacheFields><extLst><ext uri="{{725AE2AE-9491-48be-B2B4-4EB974FC3084}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:pivotCacheDefinition/></ext></extLst></pivotCacheDefinition>"#
    )
}

fn excel_records() -> String {
    format!(
        r#"{DECL}<pivotCacheRecords xmlns="{MAIN}" xmlns:r="{REL}" {EXCEL_ROOT} count="6"><r><x v="0"/><x v="0"/><n v="10"/><n v="1"/></r><r><x v="1"/><x v="1"/><n v="20"/><n v="2"/></r><r><x v="0"/><x v="2"/><n v="30"/><n v="3"/></r><r><x v="1"/><x v="3"/><n v="40"/><m/></r><r><x v="0"/><x v="4"/><n v="50"/><n v="5"/></r><r><x v="2"/><x v="5"/><n v="60"/><n v="6"/></r></pivotCacheRecords>"#
    )
}

/// Years then months down the side, regions across, the sum of the
/// amounts; items in the order Excel sorted them in.
fn excel_table() -> String {
    let months: String = (0..14).map(|x| format!("<item x=\"{x}\"/>")).collect();
    format!(
        r#"{DECL}<pivotTableDefinition xmlns="{MAIN}" {EXCEL_ROOT} xr:uid="{{00000000-0007-0000-0100-000001000000}}" name="Par mois" cacheId="7" applyNumberFormats="0" applyBorderFormats="0" applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0" applyWidthHeightFormats="1" dataCaption="Values" updatedVersion="8" minRefreshableVersion="3" useAutoFormatting="1" itemPrintTitles="1" createdVersion="8" indent="0" outline="1" outlineData="1" multipleFieldFilters="0" rowHeaderCaption="When"><location ref="A3:E12" firstHeaderRow="1" firstDataRow="2" firstDataCol="1"/><pivotFields count="6"><pivotField axis="axisCol" showAll="0"><items count="4"><item x="2"/><item x="0"/><item x="1"/><item t="default"/></items></pivotField><pivotField axis="axisRow" numFmtId="14" showAll="0"><items count="15">{months}<item t="default"/></items></pivotField><pivotField dataField="1" showAll="0"/><pivotField showAll="0"/><pivotField showAll="0" defaultSubtotal="0"><items count="6"><item sd="0" x="0"/><item sd="0" x="1"/><item sd="0" x="2"/><item sd="0" x="3"/><item sd="0" x="4"/><item sd="0" x="5"/></items></pivotField><pivotField axis="axisRow" showAll="0" defaultSubtotal="0"><items count="4"><item x="0"/><item x="1"/><item x="2"/><item x="3"/></items></pivotField></pivotFields><rowFields count="2"><field x="5"/><field x="1"/></rowFields><rowItems count="8"><i><x v="1"/></i><i r="1"><x v="1"/></i><i r="1"><x v="2"/></i><i r="1"><x v="3"/></i><i><x v="2"/></i><i r="1"><x v="3"/></i><i r="1"><x v="4"/></i><i t="grand"><x/></i></rowItems><colFields count="1"><field x="0"/></colFields><colItems count="4"><i><x/></i><i><x v="1"/></i><i><x v="2"/></i><i t="grand"><x/></i></colItems><dataFields count="1"><dataField name="Sum of Amount" fld="2" baseField="0" baseItem="0" numFmtId="3"/></dataFields><pivotTableStyleInfo name="PivotStyleMedium2" showRowHeaders="1" showColHeaders="1" showRowStripes="0" showColStripes="0" showLastColumn="1"/><extLst><ext uri="{{962EF5D1-5CA2-4c93-8EF4-DBF5C05439D2}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:pivotTableDefinition hideValuesRow="1" xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"/></ext></extLst></pivotTableDefinition>"#
    )
}

/// A package the way Excel saves one: the data on the first sheet, the
/// PivotTable on the second, where the cells still hold what it showed.
/// Parts named in `replace` are changed, or left out for `None`.
fn excel_package(replace: &[(&str, Option<&str>)]) -> Vec<u8> {
    let workbook = format!(
        r#"{DECL}<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Report" sheetId="2" r:id="rId2"/></sheets><calcPr calcId="191029"/><pivotCaches><pivotCache cacheId="7" r:id="rId4"/></pivotCaches></workbook>"#
    );
    let styles = format!(
        r#"{DECL}<styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"#
    );
    let mut rows = String::new();
    for (r, row) in [
        ["Region", "Date", "Amount", "Units"],
        ["North", "45296", "10", "1"],
        ["South", "45311", "20", "2"],
        ["North", "45333", "30", "3"],
        ["South", "45353", "40", ""],
        ["North", "45731", "50", "5"],
        ["East", "45748", "60", "6"],
    ]
    .iter()
    .enumerate()
    {
        rows.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, text) in row.iter().enumerate() {
            let name = format!("{}{}", (b'A' + c as u8) as char, r + 1);
            if text.is_empty() {
            } else if r == 0 || c == 0 {
                rows.push_str(&format!(
                    "<c r=\"{name}\" t=\"inlineStr\"><is><t>{text}</t></is></c>"
                ));
            } else if c == 1 {
                rows.push_str(&format!("<c r=\"{name}\" s=\"1\"><v>{text}</v></c>"));
            } else {
                rows.push_str(&format!("<c r=\"{name}\"><v>{text}</v></c>"));
            }
        }
        rows.push_str("</row>");
    }
    let data = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}"><dimension ref="A1:D7"/><sheetData>{rows}</sheetData></worksheet>"#
    );
    // What the PivotTable showed when the file was saved, in part: stale
    // on purpose, to see that the cells are computed again.
    let report = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}"><dimension ref="A1:E12"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>above</t></is></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Sum of Amount</t></is></c><c r="B3" t="inlineStr"><is><t>stale</t></is></c></row><row r="12"><c r="A12" t="inlineStr"><is><t>Grand Total</t></is></c><c r="E12"><v>1</v></c></row><row r="14"><c r="A14" t="inlineStr"><is><t>below</t></is></c></row></sheetData></worksheet>"#
    );
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/pivotTables/pivotTable1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/><Override PartName="/xl/pivotCache/pivotCacheDefinition1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/><Override PartName="/xl/pivotCache/pivotCacheRecords1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/></Types>"#
    );
    let mut parts: Vec<(String, String)> = [
        ("[Content_Types].xml", content_types),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", workbook),
        (
            "xl/_rels/workbook.xml.rels",
            relationships(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "worksheet", "worksheets/sheet2.xml"),
                ("rId3", "styles", "styles.xml"),
                (
                    "rId4",
                    "pivotCacheDefinition",
                    "pivotCache/pivotCacheDefinition1.xml",
                ),
            ]),
        ),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", data),
        ("xl/worksheets/sheet2.xml", report),
        (
            "xl/worksheets/_rels/sheet2.xml.rels",
            relationships(&[("rId1", "pivotTable", "../pivotTables/pivotTable1.xml")]),
        ),
        ("xl/pivotTables/pivotTable1.xml", excel_table()),
        (
            "xl/pivotTables/_rels/pivotTable1.xml.rels",
            relationships(&[(
                "rId1",
                "pivotCacheDefinition",
                "../pivotCache/pivotCacheDefinition1.xml",
            )]),
        ),
        (
            "xl/pivotCache/pivotCacheDefinition1.xml",
            excel_definition(),
        ),
        (
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels",
            relationships(&[("rId1", "pivotCacheRecords", "pivotCacheRecords1.xml")]),
        ),
        ("xl/pivotCache/pivotCacheRecords1.xml", excel_records()),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_string(), text))
    .collect();
    for (name, text) in replace {
        parts.retain(|(n, _)| n != name);
        if let Some(text) = text {
            parts.push((name.to_string(), text.to_string()));
        }
    }
    let parts: Vec<(&str, &str)> = parts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    package(&parts)
}

#[test]
fn pivots_from_excel() {
    let bytes = excel_package(&[]);
    keep("excel-style-pivot.xlsx", &bytes);
    let wb = load(&bytes);
    assert_eq!(wb.pivots().count(), 1);
    let (sheet, p) = named(&wb, "Par mois");
    assert_eq!(sheet, 1);
    assert_eq!(
        p.source,
        PivotSource::Range {
            sheet: "Data".to_string(),
            range: rect("A1:D7")
        }
    );
    // The months Excel keeps on the date field itself have a field of
    // their own, after those of the file; the dates are dates again.
    let names: Vec<&str> = p.cache.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Region",
            "Date",
            "Amount",
            "Units",
            "Quarters",
            "Years",
            "Months (Date)"
        ]
    );
    assert!(p.cache.fields[1].group.is_none() && p.cache.is_date(1));
    let by = |field: usize| match p.cache.fields[field]
        .group
        .as_ref()
        .map(|g| (g.base, &g.kind))
    {
        Some((Some(1), GroupKind::Range { by, .. })) => *by,
        other => panic!("field {field}: {other:?}"),
    };
    assert_eq!(
        [by(4), by(5), by(6)],
        [GroupBy::Quarters, GroupBy::Years, GroupBy::Months]
    );
    assert_eq!(p.cache.rows, 6);
    assert_eq!(p.cache.items(3).len(), 6, "five numbers and a blank");
    assert_eq!(
        p.row_fields,
        [AxisField::Field(5), AxisField::Field(6)],
        "years, then the months that were on the date field"
    );
    assert_eq!(p.col_fields, [AxisField::Field(0)]);
    assert_eq!(p.data_fields.len(), 1);
    assert_eq!(p.data_fields[0].name, "Sum of Amount");
    assert_eq!(p.data_fields[0].num_fmt.as_deref(), Some("#,##0"));
    assert_eq!(p.style.name, "PivotStyleMedium2");
    assert_eq!(p.options.row_header_caption.as_deref(), Some("When"));
    assert_eq!(p.options.data_caption, None);
    assert_eq!(p.fields[5].subtotals, Subtotals::None);
    assert_eq!(p.location, Some(rect("A3:E12")));

    // The cells are computed again from the records, not taken from the
    // sheet; what is around the report is left alone.
    assert_eq!(
        shown(&wb, 1, rect("A1:E14")),
        [
            "above||||",
            "||||",
            "Sum of Amount|Column Labels|||",
            "When|East|North|South|Grand Total",
            "2024||||",
            "Jan||10|20|30",
            "Feb||30||30",
            "Mar|||40|40",
            "2025||||",
            "Mar||50||50",
            "Apr|60|||60",
            "Grand Total|60|90|60|210",
            "||||",
            "below||||",
        ]
        .join("\n")
    );
    assert!(!wb.modified && !wb.can_undo());

    // Refreshing reads the sheet again.
    let mut changed = load(&bytes);
    enter(&mut changed, 0, "C2", "1010");
    changed.commit("edit");
    changed.pivot_refresh_all(true).unwrap();
    assert_eq!(value(&changed, 1, "E12"), Value::Number(1210.0));

    // Saved by us, the periods nest by their length whatever the order of
    // the fields, and the file reads back the same.
    let ours = to_bytes(&wb);
    keep("excel-style-pivot-saved.xlsx", &ours);
    let definition = part(&ours, "xl/pivotCache/pivotCacheDefinition1.xml");
    for expected in [
        "</sharedItems><fieldGroup par=\"6\"/></cacheField>",
        "<cacheField name=\"Quarters\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup par=\"5\" base=\"1\"><rangePr groupBy=\"quarters\" startDate=\"2024-01-05T00:00:00\" endDate=\"2025-04-02T00:00:00\"/>",
        "<cacheField name=\"Years\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup base=\"1\"><rangePr groupBy=\"years\"",
        "<cacheField name=\"Months (Date)\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup par=\"4\" base=\"1\"><rangePr groupBy=\"months\"",
        "<cacheField name=\"Units\" numFmtId=\"0\"><sharedItems containsString=\"0\" containsBlank=\"1\" containsNumber=\"1\" containsInteger=\"1\" minValue=\"1\" maxValue=\"6\" count=\"6\">",
    ] {
        assert!(definition.contains(expected), "{expected}\n{definition}");
    }
    let again = load(&ours);
    assert_same_pivots(&wb, &again);
    assert_eq!(
        part(&to_bytes(&again), "xl/pivotTables/pivotTable1.xml"),
        part(&ours, "xl/pivotTables/pivotTable1.xml")
    );

    // Values the definition does not list are in the records; so can all
    // of them be, strings and dates included.
    let definition = excel_definition().replace(
        "<sharedItems count=\"3\"><s v=\"North\"/><s v=\"South\"/><s v=\"East\"/></sharedItems>",
        "<sharedItems/>",
    );
    let records = excel_records()
        .replace("<r><x v=\"0\"/>", "<r><s v=\"North\"/>")
        .replace("<r><x v=\"1\"/>", "<r><s v=\"South\"/>")
        .replace("<r><x v=\"2\"/>", "<r><s v=\"East\"/>");
    let inline = load(&excel_package(&[
        ("xl/pivotCache/pivotCacheDefinition1.xml", Some(&definition)),
        ("xl/pivotCache/pivotCacheRecords1.xml", Some(&records)),
    ]));
    assert_eq!(
        shown(&inline, 1, rect("A4:E12")),
        shown(&wb, 1, rect("A4:E12"))
    );
}

/// The parts of the package a relationships part points at.
fn targets(bytes: &[u8], rels: &str) -> Vec<(String, String)> {
    let base = rels.split("_rels/").next().unwrap();
    part(bytes, rels)
        .split("<Relationship ")
        .skip(1)
        .map(|rel| {
            let attr = |name: &str| {
                let start = rel.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                rel[start..].split('"').next().unwrap().to_string()
            };
            let target = attr("Target");
            let target = match target.strip_prefix("../") {
                Some(up) => format!("xl/{up}"),
                None => format!("{base}{target}"),
            };
            (attr("Type").rsplit('/').next().unwrap().to_string(), target)
        })
        .collect()
}

#[test]
fn package_lists_the_pivot_parts() {
    let bytes = to_bytes(&sample(Locale::en()));
    let names = part_names(&bytes);
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    let count = |prefix: &str| names.iter().filter(|n| n.starts_with(prefix)).count();
    // Seven PivotTables on four caches, each with its records.
    assert_eq!(count("xl/pivotTables/pivotTable"), 7);
    assert_eq!(count("xl/pivotTables/_rels/"), 7);
    assert_eq!(count("xl/pivotCache/pivotCacheDefinition"), 4);
    assert_eq!(count("xl/pivotCache/pivotCacheRecords"), 4);
    assert_eq!(count("xl/pivotCache/_rels/"), 4);
    let types = part(&bytes, "[Content_Types].xml");
    for name in &names {
        let xml = part(&bytes, name);
        assert_well_formed(name, &xml);
        let kind = if name.starts_with("xl/pivotTables/pivotTable") {
            "pivotTable"
        } else if name.starts_with("xl/pivotCache/pivotCacheDefinition") {
            "pivotCacheDefinition"
        } else if name.starts_with("xl/pivotCache/pivotCacheRecords") {
            "pivotCacheRecords"
        } else {
            continue;
        };
        assert!(
            types.contains(&format!(
                "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.{kind}+xml\"/>"
            )),
            "{name} is not declared"
        );
    }

    // Workbook → caches: `<pivotCaches>` names relationships of the
    // workbook part, after the calculation properties.
    let workbook = part(&bytes, "xl/workbook.xml");
    common::assert_workbook_order(&workbook);
    let workbook_rels = part(&bytes, "xl/_rels/workbook.xml.rels");
    let list = workbook
        .split("<pivotCaches>")
        .nth(1)
        .and_then(|rest| rest.split("</pivotCaches>").next())
        .expect("a list of caches");
    let mut cache_ids = Vec::new();
    for (n, entry) in list.split("<pivotCache ").skip(1).enumerate() {
        let attr = |name: &str| {
            let start = entry.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
            entry[start..].split('"').next().unwrap().to_string()
        };
        assert!(
            workbook_rels.contains(&format!(
                "<Relationship Id=\"{}\" Type=\"{REL}/pivotCacheDefinition\" Target=\"pivotCache/pivotCacheDefinition{}.xml\"/>",
                attr("r:id"),
                n + 1
            )),
            "{entry}"
        );
        cache_ids.push(attr("cacheId"));
    }
    assert_eq!(cache_ids, ["1", "2", "3", "4"]);

    // Sheet → PivotTables → cache → records, all by relationships whose
    // targets exist.
    let pair = |kind: &str, target: &str| (kind.to_string(), target.to_string());
    assert_eq!(
        targets(&bytes, "xl/worksheets/_rels/sheet1.xml.rels"),
        [pair("table", "xl/tables/table1.xml")]
    );
    assert_eq!(
        targets(&bytes, "xl/worksheets/_rels/sheet2.xml.rels"),
        [
            pair("pivotTable", "xl/pivotTables/pivotTable1.xml"),
            pair("pivotTable", "xl/pivotTables/pivotTable2.xml"),
            pair("pivotTable", "xl/pivotTables/pivotTable3.xml"),
        ]
    );
    let on_third = targets(&bytes, "xl/worksheets/_rels/sheet3.xml.rels");
    assert_eq!(on_third.len(), 4);
    assert_eq!(
        on_third[3],
        pair("pivotTable", "xl/pivotTables/pivotTable7.xml")
    );
    let cache_of = [1, 1, 1, 2, 3, 4, 1];
    for (n, cache) in cache_of.iter().enumerate() {
        let n = n + 1;
        assert_eq!(
            targets(
                &bytes,
                &format!("xl/pivotTables/_rels/pivotTable{n}.xml.rels")
            ),
            [pair(
                "pivotCacheDefinition",
                &format!("xl/pivotCache/pivotCacheDefinition{cache}.xml")
            )]
        );
        let table = part(&bytes, &format!("xl/pivotTables/pivotTable{n}.xml"));
        assert!(table.contains(&format!(" cacheId=\"{cache}\" ")), "{n}");
    }
    for n in 1..=4 {
        assert_eq!(
            targets(
                &bytes,
                &format!("xl/pivotCache/_rels/pivotCacheDefinition{n}.xml.rels")
            ),
            [pair(
                "pivotCacheRecords",
                &format!("xl/pivotCache/pivotCacheRecords{n}.xml")
            )]
        );
        let definition = part(
            &bytes,
            &format!("xl/pivotCache/pivotCacheDefinition{n}.xml"),
        );
        assert!(definition.contains(" r:id=\"rId1\" "));
    }
    // The worksheets say nothing of their PivotTables beyond the cells.
    for n in 1..=3 {
        let sheet = part(&bytes, &format!("xl/worksheets/sheet{n}.xml"));
        common::assert_worksheet_order(&format!("sheet{n}"), &sheet);
        assert!(!sheet.contains("<pivot"));
    }
    // A workbook without PivotTables has none of this.
    let plain = to_bytes(&Workbook::new(Locale::en()));
    assert!(!part_names(&plain).iter().any(|n| n.contains("pivot")));
    assert!(!part(&plain, "xl/workbook.xml").contains("pivotCaches"));
}

#[test]
fn parts_say_what_the_model_says() {
    let wb = sample(Locale::en());
    let bytes = to_bytes(&wb);
    // The plainest PivotTable, whole: regions down the side, the sum of
    // the sales.
    assert_eq!(
        part(&bytes, "xl/pivotTables/pivotTable1.xml"),
        format!(
            concat!(
                "{}<pivotTableDefinition xmlns=\"{}\" name=\"PivotTable1\" cacheId=\"1\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"8\" createdVersion=\"8\" minRefreshableVersion=\"3\" useAutoFormatting=\"1\" itemPrintTitles=\"1\" indent=\"0\" outline=\"1\" outlineData=\"1\" multipleFieldFilters=\"0\">",
                "<location ref=\"A3:B8\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/>",
                "<pivotFields count=\"6\"><pivotField axis=\"axisRow\" showAll=\"0\"><items count=\"5\"><item x=\"0\"/><item x=\"2\"/><item x=\"3\"/><item x=\"1\"/><item t=\"default\"/></items></pivotField>",
                "<pivotField showAll=\"0\"/><pivotField showAll=\"0\"/><pivotField dataField=\"1\" showAll=\"0\"/><pivotField showAll=\"0\"/><pivotField showAll=\"0\"/></pivotFields>",
                "<rowFields count=\"1\"><field x=\"0\"/></rowFields>",
                "<rowItems count=\"5\"><i><x/></i><i><x v=\"1\"/></i><i><x v=\"2\"/></i><i><x v=\"3\"/></i><i t=\"grand\"><x/></i></rowItems>",
                "<colItems count=\"1\"><i/></colItems>",
                "<dataFields count=\"1\"><dataField name=\"Sum of Sales\" fld=\"3\" baseField=\"0\" baseItem=\"0\"/></dataFields>",
                "<pivotTableStyleInfo name=\"PivotStyleLight16\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/>",
                "</pivotTableDefinition>"
            ),
            DECL, MAIN
        )
    );
    // Its cache: the source, then every field with the values it takes,
    // typed as Excel types them; the records name the values by place.
    let definition = part(&bytes, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(definition.starts_with(&format!(
        "{DECL}<pivotCacheDefinition xmlns=\"{MAIN}\" xmlns:r=\"{REL}\" r:id=\"rId1\" refreshedVersion=\"8\" createdVersion=\"8\" minRefreshableVersion=\"3\" recordCount=\"12\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"A1:F13\" sheet=\"Data\"/></cacheSource><cacheFields count=\"6\"><cacheField name=\"Region\" numFmtId=\"0\"><sharedItems count=\"4\"><s v=\"East\"/><s v=\"West\"/><s v=\"North\"/><s v=\"South &amp; &lt;co&gt;\"/></sharedItems></cacheField>"
    )));
    for expected in [
        "<cacheField name=\"Date\" numFmtId=\"14\"><sharedItems containsSemiMixedTypes=\"0\" containsNonDate=\"0\" containsDate=\"1\" containsString=\"0\" minDate=\"2023-01-15T00:00:00\" maxDate=\"2025-04-01T00:00:00\" count=\"12\"><d v=\"2023-01-15T00:00:00\"/>",
        "<cacheField name=\"Sales\" numFmtId=\"4\"><sharedItems containsSemiMixedTypes=\"0\" containsString=\"0\" containsNumber=\"1\" minValue=\"15\" maxValue=\"200\" count=\"12\"><n v=\"100\"/><n v=\"150.5\"/>",
        "<cacheField name=\"Units\" numFmtId=\"0\"><sharedItems containsSemiMixedTypes=\"0\" containsString=\"0\" containsNumber=\"1\" containsInteger=\"1\" minValue=\"1\" maxValue=\"20\" count=\"12\">",
        "<cacheField name=\"Note\" numFmtId=\"0\"><sharedItems containsBlank=\"1\" containsMixedTypes=\"1\" containsNumber=\"1\" containsInteger=\"1\" minValue=\"7\" maxValue=\"7\" count=\"4\"><s v=\"ok\"/><s v=\"late\"/><n v=\"7\"/><m/></sharedItems></cacheField></cacheFields></pivotCacheDefinition>",
    ] {
        assert!(definition.contains(expected), "{expected}\n{definition}");
    }
    let records = part(&bytes, "xl/pivotCache/pivotCacheRecords1.xml");
    assert!(records.starts_with(&format!(
        "{DECL}<pivotCacheRecords xmlns=\"{MAIN}\" xmlns:r=\"{REL}\" count=\"12\"><r><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/></r><r><x v=\"0\"/><x v=\"1\"/><x v=\"1\"/><x v=\"1\"/><x v=\"1\"/><x v=\"3\"/></r>"
    )));
    assert_eq!(records.matches("<r>").count(), 12);

    // The other PivotTables, by what is particular to each.
    let table = |name: &str| {
        let n = wb.pivots().position(|(_, p)| p.name == name).unwrap() + 1;
        part(&bytes, &format!("xl/pivotTables/pivotTable{n}.xml"))
    };
    let contains = |name: &str, expected: &[&str]| {
        let xml = table(name);
        for expected in expected {
            assert!(xml.contains(expected), "{name}: {expected}\n{xml}");
        }
    };
    contains(
        "Cross & <tab>",
        &[
            " name=\"Cross &amp; &lt;tab&gt;\" cacheId=\"1\" ",
            " grandTotalCaption=\"All of it\" missingCaption=\"-\" ",
            "<location ref=\"A16:K28\" firstHeaderRow=\"1\" firstDataRow=\"3\" firstDataCol=\"1\"/>",
            // A collapsed item, a hidden one.
            "<item sd=\"0\" x=\"1\"/>",
            "<item h=\"1\" x=\"0\"/>",
            "<colFields count=\"2\"><field x=\"5\"/><field x=\"-2\"/></colFields>",
            "<colItems count=\"10\"><i><x/><x/></i><i r=\"1\" i=\"1\"><x v=\"1\"/></i>",
            "<i t=\"grand\"><x/></i><i t=\"grand\" i=\"1\"><x/></i></colItems>",
            "<dataField name=\"Sum of Sales\" fld=\"3\" baseField=\"0\" baseItem=\"0\" numFmtId=\"4\"/><dataField name=\"Count of Units\" fld=\"4\" subtotal=\"count\" baseField=\"0\" baseItem=\"0\"/>",
            "<pivotTableStyleInfo name=\"PivotStyleMedium9\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"1\" showColStripes=\"0\" showLastColumn=\"1\"/>",
        ],
    );
    contains(
        "Everything",
        &[
            " dataCaption=\"Figures\" showError=\"1\" errorCaption=\"n/a\" showMissing=\"0\" ",
            " compact=\"0\" compactData=\"0\" rowGrandTotals=\"0\" showDrill=\"0\" multipleFieldFilters=\"0\" rowHeaderCaption=\"Rows\" colHeaderCaption=\"Columns\">",
            "<location ref=\"P3:V24\" firstHeaderRow=\"0\" firstDataRow=\"1\" firstDataCol=\"2\" rowPageCount=\"1\" colPageCount=\"1\"/>",
            "<pivotField axis=\"axisRow\" numFmtId=\"49\" compact=\"0\" outline=\"0\" showAll=\"0\" sortType=\"ascending\" defaultSubtotal=\"0\"><items count=\"4\"><item n=\"Orient\" x=\"0\"/>",
            "<pivotField name=\"Fruit\" axis=\"axisRow\" compact=\"0\" outline=\"0\" subtotalTop=\"0\" showAll=\"1\" insertBlankRow=\"1\" sortType=\"descending\" defaultSubtotal=\"0\" sumSubtotal=\"1\" maxSubtotal=\"1\" varPSubtotal=\"1\">",
            "<item t=\"max\"/><item t=\"sum\"/><item t=\"varP\"/></items><autoSortScope><pivotArea dataOnly=\"0\" outline=\"0\" fieldPosition=\"0\"><references count=\"1\"><reference field=\"4294967294\" count=\"1\" selected=\"0\"><x v=\"0\"/></reference></references></pivotArea></autoSortScope>",
            "<x14:pivotField fillDownLabels=\"1\"/>",
            "<pivotField axis=\"axisPage\" compact=\"0\" outline=\"0\" showAll=\"0\" multipleItemSelectionAllowed=\"1\">",
            "<i t=\"max\"><x v=\"1\"/></i><i t=\"sum\"><x v=\"1\"/></i><i t=\"varP\"><x v=\"1\"/></i><i t=\"blank\"><x v=\"1\"/></i>",
            "<colFields count=\"1\"><field x=\"-2\"/></colFields><colItems count=\"5\"><i><x/></i><i i=\"1\"><x v=\"1\"/></i>",
            "<pageFields count=\"1\"><pageField fld=\"5\" hier=\"-1\"/></pageFields>",
            "<dataField name=\"Sum of Sales2\" fld=\"3\" showDataAs=\"percentOfTotal\" baseField=\"0\" baseItem=\"0\"/>",
            "<dataField name=\"Running\" fld=\"3\" showDataAs=\"runTotal\" baseField=\"0\" baseItem=\"0\"/>",
            "<x14:dataField pivotShowAs=\"percentOfParentRow\"/>",
            "<dataField name=\"Sum of Units\" fld=\"4\" subtotal=\"average\" showDataAs=\"difference\" baseField=\"0\" baseItem=\"0\" numFmtId=\"164\"/>",
            "<pivotTableStyleInfo showRowHeaders=\"1\" showColHeaders=\"0\" showRowStripes=\"0\" showColStripes=\"1\" showLastColumn=\"1\"/><filters count=\"2\">",
            "<filter fld=\"0\" type=\"captionNotContains\" evalOrder=\"-1\" id=\"1\" stringValue1=\"zz\"><autoFilter ref=\"A1\"><filterColumn colId=\"0\"><customFilters><customFilter operator=\"notEqual\" val=\"*zz*\"/></customFilters></filterColumn></autoFilter></filter>",
            "<filter fld=\"1\" type=\"count\" evalOrder=\"-1\" id=\"2\" iMeasureFld=\"0\"><autoFilter ref=\"A1\"><filterColumn colId=\"0\"><top10 val=\"3\" filterVal=\"3\"/></filterColumn></autoFilter></filter>",
        ],
    );
    // The format only this PivotTable uses is in the styles part.
    assert!(part(&bytes, "xl/styles.xml").contains("<numFmt numFmtId=\"164\" formatCode=\""));
    contains(
        "Dates",
        &[
            " compact=\"0\" compactData=\"0\" outline=\"1\" outlineData=\"1\" colGrandTotals=\"0\" ",
            "<rowFields count=\"5\"><field x=\"5\"/><field x=\"-2\"/><field x=\"6\"/><field x=\"7\"/><field x=\"2\"/></rowFields>",
            "<filter fld=\"2\" type=\"valueBetween\" evalOrder=\"-1\" id=\"1\" iMeasureFld=\"0\" stringValue1=\"1\" stringValue2=\"100000.5\">",
        ],
    );
    contains(
        "Groups",
        &[
            " showHeaders=\"0\" outline=\"1\" outlineData=\"1\" pageOverThenDown=\"1\" pageWrap=\"2\" ",
            "<pivotField axis=\"axisCol\" dataField=\"1\" showAll=\"0\"><items count=\"8\">",
            "<pageFields count=\"1\"><pageField fld=\"1\" item=\"1\" hier=\"-1\"/></pageFields>",
        ],
    );
    contains(
        "PivotTable2",
        &["<x14:dataField pivotShowAs=\"rankDescending\"/>"],
    );
    // One nothing was put in yet: fields, and no lines.
    let empty = table("PivotTable3");
    assert!(empty.contains("<location ref=\"T25:V42\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/><pivotFields count=\"6\"><pivotField showAll=\"0\"/>"));
    assert!(empty.contains("<pivotField showAll=\"0\"/></pivotFields><pivotTableStyleInfo "));

    // Groups: the periods of a date derive from it and nest, numbers are
    // grouped in place, hand-made groups map the items of their field.
    let dates = part(&bytes, "xl/pivotCache/pivotCacheDefinition2.xml");
    for expected in [
        "</sharedItems><fieldGroup par=\"7\"/></cacheField>",
        "<cacheField name=\"Years (Date)\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup base=\"2\"><rangePr groupBy=\"years\" startDate=\"2023-01-15T00:00:00\" endDate=\"2025-04-01T00:00:00\"/><groupItems count=\"5\"><s v=\"&lt;1/15/2023\"/><s v=\"2023\"/><s v=\"2024\"/><s v=\"2025\"/><s v=\"&gt;4/1/2025\"/></groupItems></fieldGroup></cacheField>",
        "<cacheField name=\"Quarters (Date)\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup par=\"5\" base=\"2\"><rangePr groupBy=\"quarters\" ",
        "<cacheField name=\"Months (Date)\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup par=\"6\" base=\"2\"><rangePr groupBy=\"months\" ",
    ] {
        assert!(dates.contains(expected), "{expected}\n{dates}");
    }
    let groups = part(&bytes, "xl/pivotCache/pivotCacheDefinition3.xml");
    for expected in [
        "<s v=\"South &amp; &lt;co&gt;\"/></sharedItems><fieldGroup par=\"4\"/></cacheField>",
        "<fieldGroup base=\"3\"><rangePr autoStart=\"0\" startNum=\"0\" endNum=\"200\" groupInterval=\"50\"/><groupItems count=\"7\"><s v=\"&lt;0\"/><s v=\"0-50\"/>",
        "<cacheField name=\"Region2\" numFmtId=\"0\" databaseField=\"0\"><fieldGroup base=\"0\"><discretePr count=\"4\"><x v=\"0\"/><x v=\"1\"/><x v=\"1\"/><x v=\"2\"/></discretePr><groupItems count=\"3\"><s v=\"East\"/><s v=\"Group1\"/><s v=\"South &amp; &lt;co&gt;\"/></groupItems></fieldGroup></cacheField>",
    ] {
        assert!(groups.contains(expected), "{expected}\n{groups}");
    }
    // The records of a cache with derived fields hold the source fields
    // only.
    let records = part(&bytes, "xl/pivotCache/pivotCacheRecords3.xml");
    assert!(records.contains("<r><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/><x v=\"0\"/></r>"));
    // A table as the source is named, not located.
    assert!(
        part(&bytes, "xl/pivotCache/pivotCacheDefinition4.xml").contains(
            "<cacheSource type=\"worksheet\"><worksheetSource name=\"Stock\"/></cacheSource>"
        )
    );
}

#[test]
fn elements_come_in_schema_order() {
    let bytes = to_bytes(&sample(Locale::en()));
    let mut checked = 0;
    for name in part_names(&bytes) {
        let xml = part(&bytes, &name);
        if name.starts_with("xl/pivotTables/pivotTable") {
            common::assert_child_order(
                &name,
                &xml,
                &[
                    "location",
                    "pivotFields",
                    "rowFields",
                    "rowItems",
                    "colFields",
                    "colItems",
                    "pageFields",
                    "dataFields",
                    "formats",
                    "conditionalFormats",
                    "chartFormats",
                    "pivotHierarchies",
                    "pivotTableStyleInfo",
                    "filters",
                    "rowHierarchiesUsage",
                    "colHierarchiesUsage",
                    "extLst",
                ],
                &[],
            );
            // Inside a field: items, the sort, extensions.
            for field in xml.split("<pivotField ").skip(1) {
                let field = field.split("</pivotField>").next().unwrap();
                let place = |element: &str| field.find(element).unwrap_or(usize::MAX);
                assert!(place("<items") <= place("<autoSortScope"), "{name}");
                assert!(
                    place("<autoSortScope") == usize::MAX
                        || place("<autoSortScope") < place("<extLst"),
                    "{name}"
                );
            }
            checked += 1;
        } else if name.starts_with("xl/pivotCache/pivotCacheDefinition") {
            common::assert_child_order(
                &name,
                &xml,
                &["cacheSource", "cacheFields", "cacheHierarchies", "extLst"],
                &[],
            );
            for field in xml.split("<cacheField ").skip(1) {
                let field = field.split("</cacheField>").next().unwrap();
                let place = |element: &str| field.find(element).unwrap_or(usize::MAX);
                assert!(
                    place("<sharedItems") < place("<fieldGroup")
                        || place("<sharedItems") == usize::MAX,
                    "{name}"
                );
                for kind in ["<rangePr", "<discretePr"] {
                    assert!(
                        place(kind) == usize::MAX || place(kind) < place("<groupItems"),
                        "{name}"
                    );
                }
            }
            checked += 1;
        } else if name.starts_with("xl/pivotCache/pivotCacheRecords") {
            common::assert_child_order(&name, &xml, &["r"], &["r"]);
            checked += 1;
        }
    }
    assert_eq!(checked, 7 + 4 + 4);
}

#[test]
fn a_bad_pivot_costs_only_itself() {
    let table = "xl/pivotTables/pivotTable1.xml";
    let definition = "xl/pivotCache/pivotCacheDefinition1.xml";
    let records = "xl/pivotCache/pivotCacheRecords1.xml";
    // Without its PivotTable, the sheet is the cells the file holds.
    let lost = |wb: &Workbook, what: &str| {
        assert_eq!(wb.sheets.len(), 2, "{what}");
        assert_eq!(wb.pivots().count(), 0, "{what}");
        assert_eq!(
            shown(wb, 1, rect("A1:B3")),
            "above|\n|\nSum of Amount|stale",
            "{what}"
        );
        assert_eq!(value(wb, 0, "C7"), Value::Number(60.0), "{what}");
        assert_eq!(value(wb, 1, "E12"), Value::Number(1.0), "{what}");
        // And it is saved as such.
        let again = load(&to_bytes(wb));
        assert_eq!(again.pivots().count(), 0, "{what}");
    };
    let with = |replace: &[(&str, Option<&str>)], what: &str| {
        lost(&load(&excel_package(replace)), what);
    };
    with(&[(table, None)], "no table part");
    with(&[(definition, None)], "no cache definition");
    with(&[(records, None)], "a cache saved without its records");
    with(
        &[("xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels", None)],
        "records nothing points at",
    );
    with(
        &[("xl/pivotTables/_rels/pivotTable1.xml.rels", None)],
        "a table that names no cache",
    );
    with(
        &[(table, Some("<pivotTableDefinition name=\"x\""))],
        "half a table",
    );
    with(
        &[(
            table,
            Some(&excel_table().replace("<location ref=\"A3:E12\"", "<location ref=\"nowhere\"")),
        )],
        "a table that is nowhere",
    );
    with(
        &[(definition, Some("<pivotCacheDefinition/>"))],
        "no fields",
    );
    with(
        &[(
            records,
            Some(&excel_records().replace("<r><x v=\"2\"/>", "<r><x v=\"3\"/>")),
        )],
        "a record naming a value the field does not list",
    );
    with(
        &[(
            records,
            Some(&excel_records().replace("<n v=\"60\"/><n v=\"6\"/></r>", "<n v=\"60\"/></r>")),
        )],
        "a record that is too short",
    );
}

#[test]
fn damaged_pivot_parts_never_panic() {
    // Cut each part of a PivotTable short at every character, in a package
    // laid out like Excel's: the workbook loads, with the PivotTable or
    // without it.
    let mut kept = 0;
    for (name, text) in [
        ("xl/pivotTables/pivotTable1.xml", excel_table()),
        (
            "xl/pivotCache/pivotCacheDefinition1.xml",
            excel_definition(),
        ),
        ("xl/pivotCache/pivotCacheRecords1.xml", excel_records()),
    ] {
        for cut in (0..text.len()).filter(|i| text.is_char_boundary(*i)) {
            let wb = load(&excel_package(&[(name, Some(&text[..cut]))]));
            assert_eq!(wb.sheets.len(), 2, "{name} cut at {cut}");
            assert_eq!(value(&wb, 0, "C7"), Value::Number(60.0));
            let pivots = wb.pivots().count();
            assert!(pivots <= 1);
            kept += pivots;
            if cut % 53 == 0 {
                load(&to_bytes(&wb));
            }
        }
    }
    // Most cuts of the table part leave a table that can still be shown.
    assert!(kept > 500, "{kept}");

    // The same at many places in a package of ours, with every feature.
    let ours = to_bytes(&sample(Locale::en()));
    let names = part_names(&ours);
    let parts: Vec<(String, String)> = names.iter().map(|n| (n.clone(), part(&ours, n))).collect();
    let mut attempts = 0;
    for (victim, text) in parts.iter().filter(|(name, _)| {
        name.ends_with("pivotTable3.xml")
            || name.ends_with("pivotTable4.xml")
            || name.ends_with("pivotCacheDefinition2.xml")
            || name.ends_with("pivotCacheDefinition3.xml")
            || name.ends_with("pivotCacheRecords3.xml")
            || name.ends_with("pivotTable5.xml.rels")
            || name == "xl/workbook.xml"
    }) {
        for cut in (0..text.len())
            .step_by(7)
            .filter(|i| text.is_char_boundary(*i))
        {
            let damaged: Vec<(&str, &str)> = parts
                .iter()
                .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                .collect();
            // Half a workbook part may not be a workbook at all.
            if let Ok(wb) = read_xlsx(Cursor::new(package(&damaged)), Locale::en()) {
                assert!(wb.pivots().count() <= 7, "{victim} cut at {cut}");
                if victim != "xl/workbook.xml" {
                    assert_eq!(wb.sheets.len(), 3);
                    assert_eq!(value(&wb, 0, "D2"), Value::Number(100.0));
                }
            }
            attempts += 1;
        }
    }
    assert!(attempts > 2000, "{attempts}");

    for junk in [
        "not xml at all",
        "<pivotTableDefinition name=\"&#0;\" cacheId=\"-1\"><location ref=\"A1:XFD1048576\" firstHeaderRow=\"99\" firstDataRow=\"0\" firstDataCol=\"70000\" rowPageCount=\"4000000000\" colPageCount=\"4000000000\"/><pivotFields count=\"1\"><pivotField axis=\"axisRow\"><items count=\"9\"><item x=\"4000000000\"/><item t=\"bogus\"/><item/></items></pivotField></pivotFields><rowFields><field x=\"-9\"/><field x=\"99\"/><field/></rowFields><pageFields><pageField fld=\"77\" item=\"5\"/><pageField/></pageFields><dataFields><dataField fld=\"99\" subtotal=\"bogus\" showDataAs=\"bogus\" baseField=\"-1\" baseItem=\"1048832\"/><dataField/></dataFields><filters><filter fld=\"9\" type=\"count\"/><filter type=\"captionBetween\"/></filters>",
        "<pivotTableDefinition><location ref=\"A3:B4\"/><pivotFields><pivotField axis=\"axisRow\"/><pivotField axis=\"axisRow\"/><pivotField axis=\"axisCol\" dataField=\"1\"/><pivotField axis=\"axisPage\"/></pivotFields><rowFields><field x=\"0\"/><field x=\"0\"/><field x=\"-2\"/><field x=\"-2\"/></rowFields><colFields><field x=\"0\"/><field x=\"2\"/></colFields><pageFields><pageField fld=\"0\"/></pageFields><dataFields><dataField fld=\"0\"/><dataField fld=\"1\" showDataAs=\"percent\" baseField=\"5\" baseItem=\"9\"/></dataFields></pivotTableDefinition>",
    ] {
        let wb = load(&excel_package(&[(
            "xl/pivotTables/pivotTable1.xml",
            Some(junk),
        )]));
        assert_eq!(wb.sheets.len(), 2, "{junk}");
        load(&to_bytes(&wb));
    }
    for junk in [
        "<pivotCacheDefinition><cacheFields><cacheField/><cacheField name=\"a\"><sharedItems><s/><n v=\"x\"/><d v=\"never\"/><e v=\"#N/A\"/><b v=\"2\"/><x v=\"1\"/></sharedItems><fieldGroup par=\"99\" base=\"99\"><rangePr groupBy=\"bogus\" startNum=\"1e999\" endNum=\"-1e999\" groupInterval=\"0\"/><discretePr><x v=\"4000000000\"/></discretePr><groupItems><s v=\"g\"/></groupItems></fieldGroup></cacheField></cacheFields></pivotCacheDefinition>",
        "<pivotCacheDefinition><cacheSource type=\"external\"/><cacheFields><cacheField name=\"a\"><sharedItems/><fieldGroup base=\"1\"><discretePr/></fieldGroup></cacheField><cacheField name=\"a\" databaseField=\"0\"><fieldGroup base=\"1\"><rangePr groupBy=\"days\" startDate=\"0000-00-00T99:99:99\" endDate=\"9999-99-99\" groupInterval=\"1e9\"/></fieldGroup></cacheField><cacheField name=\"c\" databaseField=\"0\"><fieldGroup base=\"2\"><rangePr groupBy=\"years\"/></fieldGroup></cacheField></cacheFields></pivotCacheDefinition>",
        "<pivotCacheDefinition><cacheFields><cacheField name=\"Region\"><sharedItems><s v=\"a\"/></sharedItems><fieldGroup base=\"0\"><rangePr groupBy=\"months\"/></fieldGroup></cacheField><cacheField name=\"Date\"><fieldGroup base=\"1\"><rangePr groupBy=\"seconds\" startDate=\"1899-12-30T00:00:00\" endDate=\"9999-12-31T23:59:59\"/></fieldGroup></cacheField><cacheField name=\"Amount\"/><cacheField name=\"Units\"/></cacheFields></pivotCacheDefinition>",
    ] {
        let wb = load(&excel_package(&[(
            "xl/pivotCache/pivotCacheDefinition1.xml",
            Some(junk),
        )]));
        assert_eq!(wb.sheets.len(), 2, "{junk}");
        load(&to_bytes(&wb));
    }
    for junk in [
        "<pivotCacheRecords><r><x/><x/><x/><x/><x/><x/><x/></r><r/><r><s v=\"a\"/></r>",
        "<pivotCacheRecords><r><s v=\"North\"/><d v=\"2024-01-05T00:00:00\"/><e v=\"#DIV/0!\"/><b v=\"1\"/></r><r><m/><m/><m/><m/></r><r><x v=\"4000000000\"/><x v=\"1\"/><n v=\"1\"/><n v=\"1\"/></r></pivotCacheRecords>",
    ] {
        let wb = load(&excel_package(&[(
            "xl/pivotCache/pivotCacheRecords1.xml",
            Some(junk),
        )]));
        assert_eq!(wb.sheets.len(), 2, "{junk}");
        load(&to_bytes(&wb));
    }
}
