//! Slicers and timelines in `.xlsx` files: the parts the writer produces,
//! that they come back unchanged, and what is made of parts as Excel
//! writes them.

mod common;

use common::{assert_well_formed, enter, part, part_names};
use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};
use opensheet_engine::refs::col_to_letters;
use opensheet_engine::slicer::{SlicerSource, TimeLevel};
use opensheet_engine::{Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

/// Every XML part of the package parses with an independent parser.
fn all_well_formed(bytes: &[u8]) {
    for name in part_names(bytes) {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_well_formed(&name, &part(bytes, &name));
        }
    }
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

const SALES: &[[&str; 4]] = &[
    ["Region", "Product", "Date", "Sales"],
    ["East", "Apples", "1/15/2023", "100"],
    ["East", "Pears", "3/2/2023", "150"],
    ["East", "Apples", "2/10/2024", "120"],
    ["West", "Apples", "3/20/2023", "200"],
    ["West", "Pears", "6/1/2024", "50"],
    ["North", "Plums", "6/30/2024", "70"],
];

/// The sales on the first sheet, with a PivotTable of sales by region on
/// a second one.
fn book() -> (Workbook, u32) {
    let mut wb = Workbook::new(Locale::en());
    for (r, row) in SALES.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            enter(
                &mut wb,
                0,
                &format!("{}{}", col_to_letters(c as u32), r + 1),
                text,
            );
        }
    }
    wb.commit("data");
    let source = PivotSource::Range {
        sheet: wb.sheets[0].name.clone(),
        range: Rect::new(0, 0, 6, 3),
    };
    let id = wb
        .pivot_create(source, PivotDestination::NewSheet { index: 1 }, false)
        .unwrap();
    wb.pivot_update(id, false, |p| {
        p.add_field(0, Area::Rows, None, Locale::en());
        p.add_field(3, Area::Values, None, Locale::en());
    })
    .unwrap();
    wb.commit("pivot");
    (wb, id)
}

fn column(
    wb: &Workbook,
    sheet: usize,
    col: u32,
    rows: std::ops::RangeInclusive<u32>,
) -> Vec<String> {
    rows.map(|r| wb.display_text(sheet, r, col).text).collect()
}

#[test]
fn the_slicers_of_a_pivot_table_go_through_a_file() {
    let (mut wb, pivot) = book();
    let region = wb.add_pivot_slicer(1, pivot, 0, (2, 4)).unwrap();
    let product = wb.add_pivot_slicer(1, pivot, 1, (2, 7)).unwrap();
    wb.update_slicer(product, |s| {
        s.caption = "Fruit & more".to_string();
        s.columns = 2;
        s.style = "SlicerStyleDark3".to_string();
        s.descending = true;
        s.hide_no_data = true;
        s.offset = (12.0, 6.0);
        s.size = (240.0, 180.0);
    });
    // A field that is in no area is filtered all the same.
    let plums = wb
        .slicer_items(product)
        .iter()
        .find(|i| i.label == "Plums")
        .unwrap()
        .x;
    let apples = wb
        .slicer_items(product)
        .iter()
        .find(|i| i.label == "Apples")
        .unwrap()
        .x;
    wb.slicer_select(product, &[plums, apples], false).unwrap();
    wb.commit("slicers");
    assert_eq!(
        column(&wb, 1, 0, 3..=6),
        ["East", "North", "West", "Grand Total"]
    );
    assert_eq!(wb.display_text(1, 6, 1).text, "490");

    let bytes = to_bytes(&wb);
    common::keep_sample("slicers-pivot.xlsx", &bytes);
    all_well_formed(&bytes);
    let names = part_names(&bytes);
    for name in [
        "xl/slicers/slicer1.xml",
        "xl/slicerCaches/slicerCache1.xml",
        "xl/slicerCaches/slicerCache2.xml",
        "xl/drawings/drawing1.xml",
    ] {
        assert!(names.iter().any(|n| n == name), "missing {name}: {names:?}");
    }
    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains("<definedName name=\"Slicer_Region\">#N/A</definedName>"));
    assert!(workbook.contains(
        "<extLst><ext uri=\"{BBE1A952-AA13-448e-AADC-164F8A28A991}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:slicerCaches><x14:slicerCache r:id=\""
    ));
    let sheet = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(
        sheet.contains("<x14:slicerList><x14:slicer r:id=\"rId"),
        "{sheet}"
    );
    assert!(sheet.ends_with("</extLst></worksheet>"));
    let cache = part(&bytes, "xl/slicerCaches/slicerCache2.xml");
    assert!(
        cache.contains("name=\"Slicer_Product\" sourceName=\"Product\""),
        "{cache}"
    );
    assert!(
        cache.contains("<pivotTables><pivotTable tabId=\"2\" name=\"PivotTable1\"/></pivotTables>")
    );
    assert!(
        cache.contains("<tabular pivotCacheId=\"1\" sortOrder=\"descending\"><items count=\"3\">"),
        "{cache}"
    );
    assert!(cache.contains("slicerCacheHideItemsWithNoData"));
    assert!(
        part(&bytes, "xl/pivotCache/pivotCacheDefinition1.xml")
            .contains("<x14:pivotCacheDefinition pivotCacheId=\"1\"/>")
    );
    let slicers = part(&bytes, "xl/slicers/slicer1.xml");
    assert!(slicers.contains("<slicer name=\"Region\" cache=\"Slicer_Region\" caption=\"Region\" rowHeight=\"241300\"/>"), "{slicers}");
    assert!(
        slicers
            .contains("caption=\"Fruit &amp; more\" columnCount=\"2\" style=\"SlicerStyleDark3\""),
        "{slicers}"
    );
    let drawing = part(&bytes, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<mc:Choice Requires=\"a14\">"));
    assert!(drawing.contains("<sle:slicer xmlns:sle=\"http://schemas.microsoft.com/office/drawing/2010/slicer\" name=\"Product\"/>"));
    let types = part(&bytes, "[Content_Types].xml");
    assert!(
        types.contains("application/vnd.ms-excel.slicer+xml")
            && types.contains("application/vnd.ms-excel.slicerCache+xml")
    );

    let back = load(&bytes);
    assert_eq!(back.sheets[1].slicers.len(), 2);
    let pivot = back.sheets[1].pivots[0].id;
    for (read, written) in back.sheets[1].slicers.iter().zip(&wb.sheets[1].slicers) {
        let mut expected = written.clone();
        expected.id = read.id;
        expected.source = SlicerSource::Pivot {
            pivots: vec![pivot],
            field: match &written.source {
                SlicerSource::Pivot { field, .. } => field.clone(),
                SlicerSource::Table { .. } => unreachable!(),
            },
        };
        assert_eq!(*read, expected);
    }
    // The filter came back with it, and the names of the caches are not
    // names of the workbook.
    assert_eq!(
        column(&back, 1, 0, 3..=6),
        ["East", "North", "West", "Grand Total"]
    );
    assert_eq!(back.display_text(1, 6, 1).text, "490");
    assert!(back.all_names().is_empty());
    let items: Vec<(String, bool)> = back
        .slicer_items(back.sheets[1].slicers[1].id)
        .into_iter()
        .map(|i| (i.label, i.selected))
        .collect();
    assert_eq!(
        items,
        [
            ("Plums".to_string(), true),
            ("Pears".to_string(), false),
            ("Apples".to_string(), true)
        ]
    );
    let _ = region;
    assert_eq!(to_bytes(&back), bytes);
}

#[test]
fn the_slicers_of_a_table_go_through_a_file() {
    let (mut wb, _) = book();
    let table = wb.add_table(0, Rect::new(0, 0, 6, 3), true).unwrap();
    let region = wb.add_table_slicer(0, table, 0, (1, 6)).unwrap();
    let east = wb
        .slicer_items(region)
        .iter()
        .find(|i| i.label == "East")
        .unwrap()
        .x;
    wb.slicer_select(region, &[east], false).unwrap();
    wb.update_slicer(region, |s| s.indicate_no_data = false);
    wb.commit("table");
    let bytes = to_bytes(&wb);
    common::keep_sample("slicers-table.xlsx", &bytes);
    all_well_formed(&bytes);
    let cache = part(&bytes, "xl/slicerCaches/slicerCache1.xml");
    assert!(
        cache.contains("<extLst><x:ext uri=\"{2F2917AC-EB37-4324-AD4E-5DD8C200BD13}\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\"><x15:tableSlicerCache tableId=\"1\" column=\"1\" crossFilter=\"none\"/></x:ext></extLst>"),
        "{cache}"
    );
    assert!(part(&bytes, "xl/workbook.xml").contains("<x15:slicerCaches xmlns:x14="));
    assert!(
        part(&bytes, "xl/worksheets/sheet1.xml").contains("{3A4CF648-6AED-40f4-86FF-DC5316D8AED3}")
    );
    let back = load(&bytes);
    let read = &back.sheets[0].slicers[0];
    assert_eq!(
        read.source,
        SlicerSource::Table {
            table: back.sheets[0].tables[0].id,
            column: "Region".to_string()
        }
    );
    assert!(!read.indicate_no_data);
    assert!(back.slicer_filtered(read.id));
    assert_eq!(
        back.sheets[0]
            .filtered_rows
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [4, 5, 6]
    );
    assert_eq!(to_bytes(&back), bytes);
}

#[test]
fn a_timeline_goes_through_a_file() {
    let (mut wb, pivot) = book();
    let id = wb.add_timeline(1, pivot, 2, (8, 1)).unwrap();
    let (start, _) = wb.timeline_bounds(id).unwrap();
    // March 2023.
    wb.timeline_select(id, Some((start + 59.0, start + 89.0)), false)
        .unwrap();
    wb.update_timeline(id, |t| {
        t.level = TimeLevel::Quarters;
        t.show_time_level = false;
        t.caption = "When".to_string();
        t.style = "TimeSlicerStyleDark2".to_string();
        t.scroll = Some(start);
    });
    wb.commit("timeline");
    assert_eq!(wb.display_text(1, 5, 1).text, "350");

    let bytes = to_bytes(&wb);
    common::keep_sample("timeline.xlsx", &bytes);
    all_well_formed(&bytes);
    let cache = part(&bytes, "xl/timelineCaches/timelineCache1.xml");
    assert!(
        cache.contains("name=\"NativeTimeline_Date\" sourceName=\"Date\""),
        "{cache}"
    );
    assert!(
        cache.contains("<state minimalRefreshVersion=\"6\" lastRefreshVersion=\"6\" pivotCacheId=\"1\" filterType=\"dateBetween\"><selection startDate=\"2023-03-01T00:00:00\" endDate=\"2023-03-31T00:00:00\"/><bounds startDate=\"2023-01-01T00:00:00\" endDate=\"2025-01-01T00:00:00\"/></state>"),
        "{cache}"
    );
    let timeline = part(&bytes, "xl/timelines/timeline1.xml");
    assert!(
        timeline.contains("<timeline name=\"Date\" cache=\"NativeTimeline_Date\" caption=\"When\" showTimeLevel=\"0\" level=\"1\" selectionLevel=\"1\" scrollPosition=\"2023-01-01T00:00:00\" style=\"TimeSlicerStyleDark2\"/>"),
        "{timeline}"
    );
    let table = part(&bytes, "xl/pivotTables/pivotTable1.xml");
    assert!(table.contains("type=\"dateBetween\""), "{table}");
    assert!(table.contains("<x15:pivotFilter useWholeDay=\"1\"/>"));
    assert!(
        part(&bytes, "xl/workbook.xml")
            .contains("<x15:timelineCacheRefs><x15:timelineCacheRef r:id=\"")
    );
    assert!(
        part(&bytes, "xl/worksheets/sheet2.xml")
            .contains("<x15:timelineRefs><x15:timelineRef r:id=\"")
    );
    assert!(part(&bytes, "xl/drawings/drawing1.xml").contains("<tsle:timeslicer name=\"Date\"/>"));

    let back = load(&bytes);
    let read = &back.sheets[1].timelines[0];
    let mut expected = wb.sheets[1].timelines[0].clone();
    expected.id = read.id;
    expected.pivots = vec![back.sheets[1].pivots[0].id];
    assert_eq!(*read, expected);
    assert_eq!(
        back.timeline_selection(read.id),
        Some((start + 59.0, start + 89.0))
    );
    assert_eq!(back.display_text(1, 5, 1).text, "350");
    assert_eq!(to_bytes(&back), bytes);
}

#[test]
fn calculated_fields_and_items_go_through_a_file() {
    let (mut wb, pivot) = book();
    wb.pivot_set_calculated_field(pivot, "Half & more", "=Sales/2", false)
        .unwrap();
    wb.pivot_set_calculated_item(pivot, 0, "E+W", "=East+West", false)
        .unwrap();
    wb.commit("calculated");
    assert_eq!(
        column(&wb, 1, 0, 3..=7),
        ["East", "North", "West", "E+W", "Grand Total"]
    );
    assert_eq!(wb.display_text(1, 6, 2).text, "310");
    let bytes = to_bytes(&wb);
    common::keep_sample("pivot-calculated.xlsx", &bytes);
    all_well_formed(&bytes);
    let cache = part(&bytes, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(cache.contains("<cacheField name=\"Half &amp; more\" numFmtId=\"0\" formula=\"Sales/2\" databaseField=\"0\"/>"), "{cache}");
    assert!(cache.contains("<s v=\"E+W\" f=\"1\"/>"), "{cache}");
    assert!(
        cache.contains("<calculatedItems count=\"1\"><calculatedItem formula=\"East+West\"><pivotArea cacheIndex=\"1\" outline=\"0\" fieldPosition=\"0\"><references count=\"1\"><reference field=\"0\" count=\"1\"><x v=\"3\"/></reference></references></pivotArea></calculatedItem></calculatedItems>"),
        "{cache}"
    );
    let table = part(&bytes, "xl/pivotTables/pivotTable1.xml");
    assert!(
        table.contains("dragToRow=\"0\" dragToCol=\"0\" dragToPage=\"0\""),
        "{table}"
    );
    assert!(table.contains("<item f=\"1\" x=\"3\"/>"), "{table}");
    let back = load(&bytes);
    let (_, p) = back.pivots().next().unwrap();
    assert_eq!(p.cache.fields[4].formula.as_deref(), Some("Sales/2"));
    assert_eq!(
        p.cache.calc_items,
        wb.pivots().next().unwrap().1.cache.calc_items
    );
    assert_eq!(back.display_text(1, 6, 2).text, "310");
    assert_eq!(to_bytes(&back), bytes);
}

#[test]
fn a_pivot_chart_goes_through_a_file() {
    use opensheet_engine::chart::ChartKind;
    let (mut wb, pivot) = book();
    let chart = wb.add_pivot_chart(1, pivot, ChartKind::Column).unwrap();
    wb.update_chart(1, chart, |c| c.anchor = (10, 1));
    wb.commit("chart");
    let bytes = to_bytes(&wb);
    common::keep_sample("pivot-chart.xlsx", &bytes);
    all_well_formed(&bytes);
    let part_xml = part(&bytes, "xl/charts/chart1.xml");
    assert!(
        part_xml.contains("<c:pivotSource><c:name>[workbook.xlsx]Sheet2!PivotTable1</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart>"),
        "{part_xml}"
    );
    assert!(part_xml.contains("<c:pivotFmts><c:pivotFmt><c:idx val=\"0\"/><c:marker><c:symbol val=\"none\"/></c:marker></c:pivotFmt></c:pivotFmts>"));
    assert!(part_xml.contains(
        "<c14:dropZonesVisible val=\"1\"/></c14:pivotOptions></c:ext></c:extLst></c:chartSpace>"
    ));
    assert!(part_xml.contains("Sheet2!$B$4:$B$6"), "{part_xml}");
    let mut back = load(&bytes);
    let read = back.sheets[1].charts[0].id;
    let id = back.sheets[1].pivots[0].id;
    assert_eq!(
        back.pivot_of_chart(read)
            .map(|(_, p, link)| (p.id, link.shown)),
        Some((id, [true; 4]))
    );
    assert_eq!(to_bytes(&back), bytes);
    // Each kind of field buttons is kept on its own.
    {
        use opensheet_engine::pivot::ChartButtonKind;
        let mut some = load(&bytes);
        some.set_pivot_chart_button_kind(read, ChartButtonKind::Value, false);
        some.set_pivot_chart_button_kind(read, ChartButtonKind::Legend, false);
        some.commit("buttons");
        let bytes = to_bytes(&some);
        let part_xml = part(&bytes, "xl/charts/chart1.xml");
        assert!(
            part_xml.contains("<c14:dropZoneFilter val=\"1\"/><c14:dropZoneCategories val=\"1\"/><c14:dropZoneData val=\"0\"/><c14:dropZoneSeries val=\"0\"/><c14:dropZonesVisible val=\"1\"/>"),
            "{part_xml}"
        );
        let again = load(&bytes);
        assert_eq!(
            again
                .pivot_of_chart(again.sheets[1].charts[0].id)
                .unwrap()
                .2
                .shown,
            [true, false, true, false]
        );
    }
    // Still bound: the chart follows a filter made after loading.
    back.pivot_update(id, false, |p| p.set_item_hidden(0, 0, true))
        .unwrap();
    back.commit("filter");
    let data = back.chart_data(1, &back.sheets[1].charts[0]);
    assert_eq!(data.categories, ["North", "West"]);
}

/// Everything of this file on one sheet, kept for a look in Excel and in
/// the application.
#[test]
fn slicers_a_timeline_a_calculated_field_and_a_chart_together() {
    use opensheet_engine::chart::ChartKind;
    let (mut wb, pivot) = book();
    wb.pivot_set_calculated_field(pivot, "Half", "=Sales/2", false)
        .unwrap();
    let region = wb.add_pivot_slicer(1, pivot, 0, (1, 4)).unwrap();
    let product = wb.add_pivot_slicer(1, pivot, 1, (1, 7)).unwrap();
    wb.update_slicer(product, |s| s.style = "SlicerStyleDark2".to_string());
    let timeline = wb.add_timeline(1, pivot, 2, (16, 4)).unwrap();
    let chart = wb.add_pivot_chart(1, pivot, ChartKind::Column).unwrap();
    wb.update_chart(1, chart, |c| c.anchor = (25, 0));
    let east = wb.slicer_items(region)[0].x;
    let north = wb.slicer_items(region)[1].x;
    wb.slicer_select(region, &[east, north], false).unwrap();
    wb.commit("all");
    let bytes = to_bytes(&wb);
    common::keep_sample("pivotx-all.xlsx", &bytes);
    all_well_formed(&bytes);
    let back = load(&bytes);
    assert_eq!(
        (
            back.sheets[1].slicers.len(),
            back.sheets[1].timelines.len(),
            back.sheets[1].charts.len()
        ),
        (2, 1, 1)
    );
    assert!(back.timeline(back.sheets[1].timelines[0].id).is_some() && timeline > 0);
    assert_eq!(to_bytes(&back), bytes);
}

/// A PivotChart of a report with two row fields: the items on two levels
/// of labels, the lines of the outer items left out of the caches.
#[test]
fn a_pivot_chart_with_two_row_fields_writes_its_levels() {
    use opensheet_engine::chart::ChartKind;
    use opensheet_engine::pivot::Area;
    let (mut wb, pivot) = book();
    let locale = wb.locale;
    wb.pivot_update(pivot, false, |p| p.add_field(1, Area::Rows, None, locale))
        .unwrap();
    let chart = wb.add_pivot_chart(1, pivot, ChartKind::Column).unwrap();
    wb.update_chart(1, chart, |c| c.anchor = (2, 5));
    wb.commit("chart");
    let bytes = to_bytes(&wb);
    common::keep_sample("pivot-chart-multilevel.xlsx", &bytes);
    all_well_formed(&bytes);
    let data = wb.chart_data(1, wb.chart(1, chart).unwrap());
    let points = data.categories.len();
    assert!(points >= 3 && data.outer_levels.len() == 1, "{data:?}");
    let xml = part(&bytes, "xl/charts/chart1.xml");
    assert!(xml.contains(&format!("<c:multiLvlStrCache><c:ptCount val=\"{points}\"/><c:lvl><c:pt idx=\"0\"><c:v>{}</c:v></c:pt>", data.categories[0])), "{xml}");
    assert_eq!(
        xml.matches("<c:lvl>").count(),
        2 * wb.chart(1, chart).unwrap().series.len()
    );
    // The values: one point per item, numbered from 0.
    assert!(xml.contains(&format!("<c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{points}\"/><c:pt idx=\"0\">")), "{xml}");
    let back = load(&bytes);
    let read = back.sheets[1].charts[0].id;
    assert_eq!(
        back.chart_data(1, back.chart(1, read).unwrap())
            .outer_levels,
        data.outer_levels
    );
    assert_eq!(
        back.chart_data(1, back.chart(1, read).unwrap()).series[0].values,
        data.series[0].values
    );
    assert_eq!(to_bytes(&back), bytes);
}
