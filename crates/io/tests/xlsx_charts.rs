//! Charts in `.xlsx` files: what the writer produces, that it comes back
//! unchanged, and what is made of charts written by other applications.

mod common;

use common::{assert_well_formed, enter, keep_sample, package, part, part_names};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::chart::{
    Axes, Axis, AxisCrossing, BlanksAs, Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, Dash,
    DataLabels, DataTable, DisplayUnit, ErrorAmount, ErrorBars, ErrorDirection, Family, Fill,
    LabelOffset, LabelPosition, LegendPos, Line, ManualLayout, Marker, MarkerShape, PointFormat,
    Series, SeriesName, ShapeFormat, TextFormat, TickLabels, TickMark, TrendKind, Trendline,
    View3D,
};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

/// A sheet name that has to be quoted in references and escaped in XML.
const QUOTED: &str = "Q3 'final' & co";

fn range(text: &str) -> RefExpr {
    match parse_formula(text, Locale::en()) {
        Ok(Expr::Ref(r)) => r,
        other => panic!("{text} is not a reference: {other:?}"),
    }
}

fn series(name: SeriesName, values: &str, categories: Option<&str>) -> Series {
    Series {
        name,
        categories: categories.map(range),
        ..Series::new(range(values))
    }
}

fn named(cell: &str) -> SeriesName {
    SeriesName::Cell(range(cell))
}

fn chart(kind: ChartKind, series: Vec<Series>) -> Chart {
    Chart {
        series,
        ..Chart::new(kind)
    }
}

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn reload(wb: &Workbook) -> Workbook {
    read_xlsx(Cursor::new(to_bytes(wb)), Locale::en()).unwrap()
}

/// Set `OPENSHEET_CHART_SAMPLES` to a directory to keep the packages these
/// tests build, to open them in Excel or to run a schema validator on
/// their parts.
fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_CHART_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}

/// A workbook with charts of the nine basic kinds, on two sheets.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Sales"));
    assert!(wb.rename_sheet(1, QUOTED));
    assert!(wb.rename_sheet(2, "Empty"));

    let table = [
        ["Product", "Q1", "Q2", "Q3", "", "x", "y"],
        ["Keyboard", "10", "15", "12", "", "1", "2.5"],
        ["Mouse", "20", "5", "8", "", "2", "4"],
        ["Screen", "30.5", "45", "40", "", "3", "-1.5"],
        ["Cable", "-4", "=1+1", "", "", "4", "8"],
    ];
    for (r, row) in table.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if !text.is_empty() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
    }
    wb.update_style(0, Rect::parse("B2:B5").unwrap(), &|s| {
        s.num_fmt = Arc::from("#,##0.00")
    });
    enter(&mut wb, 1, "A1", "Month");
    enter(&mut wb, 1, "B1", "Total");
    for (i, (date, total)) in [
        ("1/31/2025", "100"),
        ("2/28/2025", "200"),
        ("3/31/2025", "300"),
    ]
    .into_iter()
    .enumerate()
    {
        wb.set_cell_text(1, i as u32 + 1, 0, date).unwrap();
        wb.set_cell_text(1, i as u32 + 1, 1, total).unwrap();
    }
    // Geometry the anchors have to cope with.
    wb.set_col_width(0, 8, Some(100.0));
    wb.sheets[0].hidden_cols.insert(10);
    wb.set_row_height(0, 9, Some(30.0));

    let products = Some("Sales!$A$2:$A$5");
    let quarter = |col: char| {
        series(
            named(&format!("Sales!${col}$1")),
            &format!("Sales!${col}$2:${col}$5"),
            products,
        )
    };
    let other = format!("'{}'", QUOTED.replace('\'', "''"));

    // Names in cells, categories, automatic everything.
    let mut column = chart(
        ChartKind::Column,
        vec![quarter('B'), quarter('C'), quarter('D')],
    );
    column.legend = LegendPos::Bottom;
    column.anchor = (7, 0);

    // A typed title, an explicit colour, labels, an offset into the cell.
    let mut stacked = chart(ChartKind::StackedColumn, vec![quarter('B'), quarter('C')]);
    stacked.title = ChartTitle::Text("Sales & <costs>\nper quarter".into());
    stacked.legend = LegendPos::Right;
    stacked.data_labels = true;
    stacked.series[0].color = Some(Color(0xC00000));
    stacked.anchor = (7, 8);
    stacked.offset = (12.0, 7.0);
    stacked.size = (400.0, 250.0);

    // No title, typed and missing names, no categories.
    let mut bar = chart(
        ChartKind::Bar,
        vec![
            series(
                SeriesName::Text("Literal & <name>".into()),
                "Sales!$C$2:$C$5",
                None,
            ),
            series(SeriesName::None, "Sales!$D$2:$D$5", None),
        ],
    );
    bar.title = ChartTitle::None;
    bar.legend = LegendPos::Top;
    bar.anchor = (24, 0);

    let mut stacked_bar = chart(
        ChartKind::StackedBar,
        vec![
            quarter('B'),
            series(SeriesName::Text("Plan".into()), "Sales!$C$2:$C$5", products),
            series(SeriesName::None, "Sales!$D$2:$D$5", products),
        ],
    );
    stacked_bar.title = ChartTitle::Text("Stacked".into());
    stacked_bar.legend = LegendPos::Left;
    stacked_bar.series[2].color = Some(Color(0x0070C0));
    stacked_bar.anchor = (24, 9);

    let mut line = chart(
        ChartKind::Line,
        vec![
            series(SeriesName::None, "Sales!$B$2:$B$5", products),
            quarter('C'),
        ],
    );
    line.series[0].color = Some(Color(0x00B050));
    line.anchor = (40, 0);
    line.size = (320.5, 200.25);

    let mut area = chart(ChartKind::Area, vec![quarter('D')]);
    area.title = ChartTitle::Text("Área – ünïcode ✓".into());
    area.series[0].color = Some(Color(0xFFC000));
    area.anchor = (40, 9);

    let mut pie = chart(ChartKind::Pie, vec![quarter('B')]);
    pie.legend = LegendPos::Right;
    pie.data_labels = true;
    pie.anchor = (56, 0);

    let mut doughnut = chart(ChartKind::Doughnut, vec![quarter('C')]);
    doughnut.title = ChartTitle::None;
    doughnut.legend = LegendPos::Bottom;
    doughnut.series[0].color = Some(Color(0x123456));
    doughnut.anchor = (56, 9);

    // X values where other charts have categories.
    let mut scatter = chart(
        ChartKind::Scatter,
        vec![
            series(
                named("Sales!$G$1"),
                "Sales!$G$2:$G$5",
                Some("Sales!$F$2:$F$5"),
            ),
            series(SeriesName::Text("No X".into()), "Sales!$B$2:$B$5", None),
        ],
    );
    scatter.series[0].color = Some(Color(0x7030A0));
    scatter.legend = LegendPos::Top;
    scatter.anchor = (72, 0);

    // Data on another sheet, whose name needs quotes.
    let mut elsewhere = chart(
        ChartKind::Column,
        vec![series(
            named(&format!("{other}!$B$1")),
            &format!("{other}!$B$2:$B$4"),
            Some(&format!("{other}!$A$2:$A$4")),
        )],
    );
    elsewhere.anchor = (72, 9);

    for c in [
        column,
        stacked,
        bar,
        stacked_bar,
        line,
        area,
        pie,
        doughnut,
        scatter,
        elsewhere,
    ] {
        wb.add_chart(0, c).unwrap();
    }

    // The second sheet: its own data, and that of the first.
    let mut own = chart(
        ChartKind::Line,
        vec![series(
            named(&format!("{other}!$B$1")),
            &format!("{other}!$B$2:$B$4"),
            Some(&format!("{other}!$A$2:$A$4")),
        )],
    );
    own.title = ChartTitle::Text("Totals".into());
    own.anchor = (5, 3);
    own.offset = (3.0, 4.0);
    let mut back = chart(ChartKind::Column, vec![quarter('B')]);
    back.legend = LegendPos::Right;
    back.anchor = (22, 3);
    wb.add_chart(1, own).unwrap();
    wb.add_chart(1, back).unwrap();
    wb.commit("charts");
    wb
}

/// Charts are the same when everything but their ids is: what they show
/// (the whole of [`Chart::content`]) and where they sit.
fn assert_same_charts(before: &Workbook, after: &Workbook) {
    assert_eq!(before.sheets.len(), after.sheets.len());
    let mut ids = HashSet::new();
    for (a, b) in before.sheets.iter().zip(&after.sheets) {
        let name = &a.name;
        assert_eq!(a.charts.len(), b.charts.len(), "{name}: number of charts");
        for (i, (x, y)) in a.charts.iter().zip(&b.charts).enumerate() {
            let which = format!("{name}, chart {} ({:?})", i + 1, x.kind);
            assert_same_content(&which, x, y);
            assert_eq!(x.anchor, y.anchor, "{which}: anchor");
            for (what, p, q) in [
                ("x offset", x.offset.0, y.offset.0),
                ("y offset", x.offset.1, y.offset.1),
                ("width", x.size.0, y.size.0),
                ("height", x.size.1, y.size.1),
            ] {
                assert!((p - q).abs() < 0.01, "{which}: {what} {p} became {q}");
            }
            assert!(y.id > 0 && ids.insert(y.id), "{which}: id {}", y.id);
        }
    }
}

/// Two charts show the same thing. The parts are compared one by one
/// first, for a failure that says where to look.
fn assert_same_content(which: &str, x: &Chart, y: &Chart) {
    assert_eq!(x.kind, y.kind, "{which}: kind");
    assert_eq!(x.title, y.title, "{which}: title");
    assert_eq!(x.legend, y.legend, "{which}: legend");
    assert_eq!(x.series.len(), y.series.len(), "{which}: number of series");
    for (n, (s, t)) in x.series.iter().zip(&y.series).enumerate() {
        assert_eq!(s, t, "{which}: series {}", n + 1);
    }
    assert_eq!(x.data_labels, y.data_labels, "{which}: data labels");
    assert_eq!(x.label_options, y.label_options, "{which}: label options");
    assert_eq!(x.axes.category, y.axes.category, "{which}: category axis");
    assert_eq!(x.axes.value, y.axes.value, "{which}: value axis");
    assert_eq!(x.axes, y.axes, "{which}: secondary axes");
    assert_eq!(x.content(), y.content(), "{which}");
}
#[test]
fn charts_round_trip() {
    let original = sample();
    assert_eq!(original.sheets[0].charts.len(), 10);
    let kinds: HashSet<ChartKind> = original.sheets[0].charts.iter().map(|c| c.kind).collect();
    assert_eq!(kinds.len(), 9, "the basic kinds are all there");

    let bytes = to_bytes(&original);
    keep("opensheet-charts.xlsx", &bytes);
    let mut loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
    assert_same_charts(&original, &loaded);
    assert!(loaded.sheets[2].charts.is_empty());
    assert!(!loaded.modified && !loaded.can_undo());

    // The charts plot the same data after the trip.
    for (si, sheet) in original.sheets.iter().enumerate() {
        for (a, b) in sheet.charts.iter().zip(&loaded.sheets[si].charts) {
            assert_eq!(original.chart_data(si, a), loaded.chart_data(si, b));
        }
    }
    let data = loaded.chart_data(0, &loaded.sheets[0].charts[0]);
    assert_eq!(data.categories, ["Keyboard", "Mouse", "Screen", "Cable"]);
    assert_eq!(data.series[1].name, "Q2");
    assert_eq!(
        data.series[1].values,
        [Some(15.0), Some(5.0), Some(45.0), Some(2.0)]
    );

    // A second trip changes nothing, down to the bytes of every part:
    // charts nobody touched are written back as they were read...
    let again = to_bytes(&loaded);
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
    }
    // ... and written from the model alone, they are the same text.
    for chart in loaded.sheets.iter_mut().flat_map(|s| &mut s.charts) {
        assert!(chart.unchanged_source().is_some());
        chart.foreign = None;
    }
    let generated = to_bytes(&loaded);
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&generated, &name), "{name}");
    }
}

#[test]
fn package_lists_the_chart_parts() {
    let bytes = to_bytes(&sample());
    let names = part_names(&bytes);
    let mut expected = vec![
        "xl/worksheets/_rels/sheet1.xml.rels".to_string(),
        "xl/worksheets/_rels/sheet2.xml.rels".to_string(),
        "xl/drawings/drawing1.xml".to_string(),
        "xl/drawings/drawing2.xml".to_string(),
        "xl/drawings/_rels/drawing1.xml.rels".to_string(),
        "xl/drawings/_rels/drawing2.xml.rels".to_string(),
    ];
    expected.extend((1..=12).map(|n| format!("xl/charts/chart{n}.xml")));
    for name in &expected {
        assert!(names.contains(name), "missing {name}");
    }
    assert!(
        !names.iter().any(|n| n.contains("sheet3.xml.rels")
            || n.contains("drawing3")
            || n.contains("chart13")),
        "a sheet without charts has no drawing: {names:?}"
    );
    for name in &names {
        assert_well_formed(name, &part(&bytes, name));
    }

    // Every part has a content type, of the right kind.
    let types = part(&bytes, "[Content_Types].xml");
    for name in names
        .iter()
        .filter(|n| n.ends_with(".xml") && !n.starts_with('['))
    {
        let kind = if name.starts_with("xl/charts/") {
            "application/vnd.openxmlformats-officedocument.drawingml.chart+xml"
        } else if name.starts_with("xl/drawings/") {
            "application/vnd.openxmlformats-officedocument.drawing+xml"
        } else {
            ""
        };
        assert!(
            types.contains(&format!(
                "<Override PartName=\"/{name}\" ContentType=\"{kind}"
            )),
            "{name} is not declared as {kind:?}"
        );
    }
    assert_eq!(types.matches("drawingml.chart+xml").count(), 12);
    assert_eq!(types.matches("officedocument.drawing+xml").count(), 2);

    // Sheet → drawing → charts, by relationships whose targets exist.
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    let targets = |rels: &str, base: &str| -> Vec<String> {
        part(&bytes, rels)
            .split("Target=\"../")
            .skip(1)
            .map(|s| format!("{base}{}", s.split('"').next().unwrap()))
            .collect()
    };
    for (sheet, drawing, charts) in [(1, 1, 1..=10), (2, 2, 11..=12)] {
        let xml = part(&bytes, &format!("xl/worksheets/sheet{sheet}.xml"));
        assert!(
            xml.ends_with("<drawing r:id=\"rId1\"/></worksheet>")
                && xml.contains("/><drawing r:id=\"rId1\"/>")
                && xml.find("<pageMargins").unwrap() < xml.find("<drawing").unwrap(),
            "sheet {sheet} refers to its drawing after the page margins"
        );
        let rels = format!("xl/worksheets/_rels/sheet{sheet}.xml.rels");
        assert_eq!(
            part(&bytes, &rels),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL}/drawing\" Target=\"../drawings/drawing{drawing}.xml\"/></Relationships>"
            )
        );
        assert_eq!(
            targets(&rels, "xl/"),
            [format!("xl/drawings/drawing{drawing}.xml")]
        );
        let rels = format!("xl/drawings/_rels/drawing{drawing}.xml.rels");
        let chart_parts: Vec<String> = charts.map(|n| format!("xl/charts/chart{n}.xml")).collect();
        assert_eq!(targets(&rels, "xl/"), chart_parts);
        let rels = part(&bytes, &rels);
        let drawing = part(&bytes, &format!("xl/drawings/drawing{drawing}.xml"));
        assert_eq!(
            drawing.matches("<xdr:twoCellAnchor").count(),
            chart_parts.len()
        );
        for k in 1..=chart_parts.len() {
            assert!(rels.contains(&format!("Id=\"rId{k}\" Type=\"{REL}/chart\"")));
            assert!(drawing.contains(&format!(" r:id=\"rId{k}\"/>")));
        }
    }
    let sheet3 = part(&bytes, "xl/worksheets/sheet3.xml");
    assert!(!sheet3.contains("<drawing"));
}

#[test]
fn chart_parts_say_what_the_model_says() {
    let bytes = to_bytes(&sample());
    let chart = |n: usize| part(&bytes, &format!("xl/charts/chart{n}.xml"));

    // Clustered columns: names and categories from cells, every reference
    // with the values last seen there.
    let column = chart(1);
    assert!(column.contains("<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:tx><c:strRef><c:f>Sales!$B$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Q1</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/><c:cat><c:strRef><c:f>Sales!$A$2:$A$5</c:f><c:strCache><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>Keyboard</c:v></c:pt><c:pt idx=\"1\"><c:v>Mouse</c:v></c:pt><c:pt idx=\"2\"><c:v>Screen</c:v></c:pt><c:pt idx=\"3\"><c:v>Cable</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sales!$B$2:$B$5</c:f><c:numCache><c:formatCode>#,##0.00</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>10</c:v></c:pt><c:pt idx=\"1\"><c:v>20</c:v></c:pt><c:pt idx=\"2\"><c:v>30.5</c:v></c:pt><c:pt idx=\"3\"><c:v>-4</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"), "{column}");
    // The result of a formula is cached like any number; an empty cell
    // leaves a gap.
    assert!(column.contains("<c:f>Sales!$C$2:$C$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>15</c:v></c:pt><c:pt idx=\"1\"><c:v>5</c:v></c:pt><c:pt idx=\"2\"><c:v>45</c:v></c:pt><c:pt idx=\"3\"><c:v>2</c:v></c:pt></c:numCache>"));
    assert!(column.contains("<c:f>Sales!$D$2:$D$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>12</c:v></c:pt><c:pt idx=\"1\"><c:v>8</c:v></c:pt><c:pt idx=\"2\"><c:v>40</c:v></c:pt></c:numCache>"));
    assert_eq!(column.matches("<c:ser>").count(), 3);
    assert!(column.contains("<c:idx val=\"2\"/><c:order val=\"2\"/>"));
    assert!(column.contains("<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/><c:axId val=\"50000011\"/><c:axId val=\"50000012\"/></c:barChart><c:catAx><c:axId val=\"50000011\"/>"));
    assert!(column.contains("<c:delete val=\"0\"/><c:axPos val=\"b\"/>"));
    assert!(
        column.contains(
            "<c:crossAx val=\"50000012\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/>"
        )
    );
    assert!(column.contains("<c:valAx><c:axId val=\"50000012\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:majorGridlines>"));
    assert!(column.contains("<c:crossAx val=\"50000011\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr></c:plotArea><c:legend><c:legendPos val=\"b\"/><c:overlay val=\"0\"/></c:legend><c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart>"));
    // What is automatic in the model is spelled as Excel spells it in its
    // own charts: a white chart area in a thin grey frame, an empty plot
    // area, grey gridlines, no line along the value axis.
    assert!(column.ends_with("</c:chart><c:spPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"D9D9D9\"/></a:solidFill></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr><a:solidFill><a:srgbClr val=\"595959\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr></c:chartSpace>"));
    assert!(column.contains("<c:majorGridlines><c:spPr><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"D9D9D9\"/></a:solidFill></a:ln></c:spPr></c:majorGridlines><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:crossAx val=\"50000011\"/>"));
    assert_eq!(
        column.matches("Gridlines>").count(),
        2,
        "on the value axis only"
    );
    // An automatic title: a title element without text, and not deleted.
    assert!(column.contains("<c:chart><c:title><c:overlay val=\"0\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr></c:title><c:autoTitleDeleted val=\"0\"/><c:plotArea>"));
    assert!(!column.contains("<c:dLbls>") && !column.contains("srgbClr val=\"C00000\""));
    // The automatic colours of the series are spelled out, whatever Excel
    // would make of the theme of the package.
    for automatic in ["156082", "E97132", "196B24"] {
        assert!(column.contains(&format!("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"{automatic}\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>")));
    }
    assert!(!column.contains("<c:dPt>"));

    let stacked = chart(2);
    assert!(stacked.contains("<c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:r><a:t>Sales &amp; &lt;costs&gt;</a:t></a:r></a:p><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:r><a:t>per quarter</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>"), "{stacked}");
    assert!(stacked.contains("<c:barDir val=\"col\"/><c:grouping val=\"stacked\"/>"));
    assert!(stacked.contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>"));
    assert!(stacked.contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>"));
    assert_eq!(
        stacked.matches("<c:spPr><a:solidFill>").count(),
        3,
        "two series and the chart area"
    );
    assert!(stacked.contains("</c:ser><c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/></c:dLbls><c:gapWidth val=\"150\"/><c:overlap val=\"100\"/>"));
    assert!(stacked.contains("<c:legendPos val=\"r\"/>"));

    // Horizontal bars: the category axis on the left. A typed name; a
    // series with neither name nor categories.
    let bar = chart(3);
    assert!(bar.contains("<c:chart><c:autoTitleDeleted val=\"1\"/><c:plotArea>"));
    assert!(bar.contains("<c:barDir val=\"bar\"/><c:grouping val=\"clustered\"/>"));
    assert!(bar.contains("<c:order val=\"0\"/><c:tx><c:v>Literal &amp; &lt;name&gt;</c:v></c:tx><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/><c:val><c:numRef><c:f>Sales!$C$2:$C$5</c:f>"));
    assert!(bar.contains("<c:order val=\"1\"/><c:spPr><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/><c:val><c:numRef><c:f>Sales!$D$2:$D$5</c:f>"));
    assert!(!bar.contains("<c:cat>"));
    assert!(bar.contains("<c:catAx><c:axId val=\"50000031\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/>"));
    assert!(bar.contains("<c:valAx><c:axId val=\"50000032\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/>"));
    assert!(bar.contains("<c:legendPos val=\"t\"/>"));
    let stacked_bar = chart(4);
    assert!(stacked_bar.contains("<c:barDir val=\"bar\"/><c:grouping val=\"stacked\"/>"));
    assert!(stacked_bar.contains("<c:legendPos val=\"l\"/>"));

    // A line has its colour on its stroke.
    let line = chart(5);
    assert!(line.contains("<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"00B050\"/></a:solidFill><a:round/></a:ln></c:spPr><c:marker><c:symbol val=\"none\"/></c:marker><c:cat>"), "{line}");
    assert!(line.contains("</c:val><c:smooth val=\"0\"/></c:ser><c:ser>"));
    assert!(line.contains("</c:tx><c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill><a:round/></a:ln></c:spPr><c:marker><c:symbol val=\"none\"/></c:marker><c:cat>"));
    assert!(line.contains("</c:ser><c:marker val=\"1\"/><c:axId val=\"50000051\"/><c:axId val=\"50000052\"/></c:lineChart>"));
    assert!(!line.contains("<c:legend>"));

    let area = chart(6);
    assert!(area.contains("<a:t>Área – ünïcode ✓</a:t>"));
    assert!(
        area.contains(
            "<c:areaChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/><c:ser>"
        )
    );
    assert!(area.contains(
        "</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"FFC000\"/></a:solidFill></c:spPr><c:cat>"
    ));
    assert!(area.contains("<c:crossBetween val=\"midCat\"/>"));

    // Round charts: a colour per slice, no axes.
    let pie = chart(7);
    assert!(pie.contains("<c:pieChart><c:varyColors val=\"1\"/><c:ser>"));
    assert!(pie.contains("</c:ser><c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/><c:showLeaderLines val=\"1\"/></c:dLbls><c:firstSliceAng val=\"0\"/></c:pieChart><c:spPr>"));
    // The slices of a pie are kept apart by white lines, and each has
    // its colour spelled out.
    assert!(pie.contains("</c:tx><c:spPr><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr><c:dPt><c:idx val=\"0\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr></c:dPt><c:dPt><c:idx val=\"1\"/>"), "{pie}");
    assert!(pie.contains("<c:dPt><c:idx val=\"3\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"0F9ED5\"/></a:solidFill><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr></c:dPt><c:cat>"));
    assert_eq!(pie.matches("<c:dPt>").count(), 4);
    let doughnut = chart(8);
    assert!(doughnut.contains("<c:doughnutChart><c:varyColors val=\"1\"/><c:ser>"));
    assert!(doughnut.contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"123456\"/></a:solidFill><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr><c:dPt><c:idx val=\"0\"/>"));
    assert!(doughnut.contains(
        "</c:ser><c:firstSliceAng val=\"0\"/><c:holeSize val=\"75\"/></c:doughnutChart><c:spPr>"
    ));
    for round in [&pie, &doughnut] {
        assert!(!round.contains("Ax>") && !round.contains("<c:axId"));
        assert!(!round.contains("invertIfNegative"));
    }

    // X and Y values, both numeric; points without a line between them.
    let scatter = chart(9);
    assert!(scatter.contains(
        "<c:scatterChart><c:scatterStyle val=\"lineMarker\"/><c:varyColors val=\"0\"/><c:ser>"
    ));
    assert!(scatter.contains("</c:tx><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:noFill/><a:round/></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/><c:spPr><a:solidFill><a:srgbClr val=\"7030A0\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"7030A0\"/></a:solidFill></a:ln></c:spPr></c:marker><c:xVal><c:numRef><c:f>Sales!$F$2:$F$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>1</c:v></c:pt>"), "{scatter}");
    assert!(scatter.contains("</c:xVal><c:yVal><c:numRef><c:f>Sales!$G$2:$G$5</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>2.5</c:v></c:pt><c:pt idx=\"1\"><c:v>4</c:v></c:pt><c:pt idx=\"2\"><c:v>-1.5</c:v></c:pt><c:pt idx=\"3\"><c:v>8</c:v></c:pt></c:numCache></c:numRef></c:yVal><c:smooth val=\"0\"/></c:ser>"));
    assert!(scatter.contains("<c:tx><c:v>No X</c:v></c:tx><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:noFill/><a:round/></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/><c:spPr><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill></a:ln></c:spPr></c:marker><c:yVal>"));
    assert_eq!(scatter.matches("<c:valAx>").count(), 2);
    assert!(!scatter.contains("<c:catAx>") && !scatter.contains("<c:cat>"));

    // Another sheet: quoted in the reference, escaped in the XML. Dates
    // are numbers with a format.
    let elsewhere = chart(10);
    assert!(elsewhere.contains("<c:tx><c:strRef><c:f>'Q3 ''final'' &amp; co'!$B$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Total</c:v></c:pt></c:strCache></c:strRef></c:tx>"), "{elsewhere}");
    assert!(elsewhere.contains("<c:cat><c:numRef><c:f>'Q3 ''final'' &amp; co'!$A$2:$A$4</c:f><c:numCache><c:formatCode>m/d/yyyy</c:formatCode><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>45688</c:v></c:pt>"));
    assert!(elsewhere.contains("<c:val><c:numRef><c:f>'Q3 ''final'' &amp; co'!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>100</c:v></c:pt>"));
    // The same data charted from its own sheet is spelled the same way.
    assert!(chart(11).contains("<c:f>'Q3 ''final'' &amp; co'!$B$2:$B$4</c:f>"));
    assert!(chart(12).contains("<c:f>Sales!$B$2:$B$5</c:f>"));
}

#[test]
fn anchors_follow_the_cells() {
    let bytes = to_bytes(&sample());
    let drawing = part(&bytes, "xl/drawings/drawing1.xml");
    // The default size on default cells: 480 × 288 pixels are 7.5 columns
    // of 64 and 14.4 rows of 20 (one of them, row 10, is 30 high).
    assert!(drawing.contains("<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>7</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>7</xdr:col><xdr:colOff>304800</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>171450</xdr:rowOff></xdr:to><xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"2\" name=\"Chart 1\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"), "{drawing}");
    // 400 × 250 from 12, 7 inside I8: across a column of 100 pixels, a
    // hidden one and a row of 30.
    assert!(drawing.contains("<xdr:from><xdr:col>8</xdr:col><xdr:colOff>114300</xdr:colOff><xdr:row>7</xdr:row><xdr:rowOff>66675</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>533400</xdr:colOff><xdr:row>19</xdr:row><xdr:rowOff>66675</xdr:rowOff></xdr:to><xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"3\" name=\"Chart 2\"/>"), "{drawing}");
    // Fractions of a pixel survive (320.5 × 200.25).
    assert!(drawing.contains("<xdr:to><xdr:col>5</xdr:col><xdr:colOff>4763</xdr:colOff><xdr:row>50</xdr:row><xdr:rowOff>2381</xdr:rowOff></xdr:to>"), "{drawing}");

    // A chart pushed past its anchor cell (say, after its columns were
    // narrowed) is written from the cell it really starts in, and the far
    // corner stops at the edge of the sheet.
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "1");
    wb.set_col_width(0, 1, Some(10.0));
    let mut moved = chart(
        ChartKind::Column,
        vec![series(SeriesName::None, "Sheet1!$A$1", None)],
    );
    moved.offset = (150.0, 45.0);
    moved.size = (100.0, 50.0);
    let mut corner = chart(ChartKind::Pie, vec![]);
    corner.anchor = (1_048_575, 16_383);
    corner.offset = (-3.0, f32::NAN);
    corner.size = (f32::INFINITY, 0.0);
    wb.add_chart(0, moved).unwrap();
    wb.add_chart(0, corner).unwrap();
    let bytes = to_bytes(&wb);
    keep("opensheet-chart-anchors.xlsx", &bytes);
    let drawing = part(&bytes, "xl/drawings/drawing1.xml");
    // 150 = 64 + 10 + 64 + 12; 45 = 20 + 20 + 5.
    assert!(drawing.contains("<xdr:from><xdr:col>3</xdr:col><xdr:colOff>114300</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>47625</xdr:rowOff></xdr:from><xdr:to><xdr:col>4</xdr:col><xdr:colOff>457200</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>142875</xdr:rowOff></xdr:to>"), "{drawing}");
    assert!(drawing.contains("<xdr:from><xdr:col>16383</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1048575</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>16383</xdr:col><xdr:colOff>609600</xdr:colOff><xdr:row>1048575</xdr:row><xdr:rowOff>190500</xdr:rowOff></xdr:to>"), "{drawing}");
    let loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
    let charts = &loaded.sheets[0].charts;
    assert_eq!((charts[0].anchor, charts[0].offset), ((2, 3), (12.0, 5.0)));
    assert_eq!(charts[0].size, (100.0, 50.0));
    assert_eq!(
        (charts[1].anchor, charts[1].offset),
        ((1_048_575, 16_383), (0.0, 0.0))
    );
    assert_eq!(charts[1].size, (64.0, 20.0));
    assert!(charts[1].series.is_empty() && charts[1].kind == ChartKind::Pie);
}

#[test]
fn references_are_made_fit_for_files() {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    assert!(wb.rename_sheet(1, "Data"));
    for (i, n) in [3.0, 1.0, 2.0].into_iter().enumerate() {
        let row = i as u32;
        wb.set_value(1, row, 0, Value::Number(n));
        wb.set_value(0, row, 0, Value::text(format!("g{i}")));
        wb.set_value(0, row, 1, Value::text(format!("c{i}")));
        wb.set_value(0, row, 2, Value::Number(n * 10.0));
        wb.set_value(0, row, 3, Value::Number(n * 100.0));
    }
    // A corner far away: whole columns and rows are cached sparsely.
    wb.set_value(1, 1_048_575, 16_383, Value::Number(9.0));
    let mut c = chart(
        ChartKind::Line,
        vec![
            // No sheet (the chart's own), relative.
            series(named("B1"), "C1:C3", Some("b1:B3")),
            // A whole column, a whole row.
            series(SeriesName::None, "data!$A:$A", None),
            series(SeriesName::None, "Data!$1:$1", None),
            // A sheet that does not exist.
            series(named("Gone!$A$1"), "Gone!$A$1:$A$3", Some("Gone!$B$1:$B$3")),
            // Blocks where Excel wants a row or a column, and stops with an
            // error message when it finds anything else: a tall one, a
            // wide one, a whole sheet.
            series(
                named("Sheet1!$A$1:$B$3"),
                "Sheet1!$C$1:$D$3",
                Some("Sheet1!$A$1:$B$3"),
            ),
            series(named("Sheet1!$A$1:$A$3"), "Sheet1!$A$1:$D$2", None),
            series(SeriesName::None, "Data!$A$1:$XFD$1048576", None),
        ],
    );
    c.title = ChartTitle::Text(String::new());
    wb.add_chart(0, c).unwrap();
    let bytes = to_bytes(&wb);
    keep("opensheet-chart-references.xlsx", &bytes);
    let xml = part(&bytes, "xl/charts/chart1.xml");
    assert_well_formed("chart1.xml", &xml);
    assert_schema_order("chart1.xml", &xml);

    assert!(xml.contains("<c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>c0</c:v></c:pt></c:strCache></c:strRef></c:tx>"), "{xml}");
    assert!(xml.contains(
        "<c:cat><c:strRef><c:f>Sheet1!$B$1:$B$3</c:f><c:strCache><c:ptCount val=\"3\"/>"
    ));
    assert!(xml.contains("<c:val><c:numRef><c:f>Sheet1!$C$1:$C$3</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>30</c:v></c:pt>"));
    assert!(xml.contains("<c:f>data!$A:$A</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"1048576\"/><c:pt idx=\"0\"><c:v>3</c:v></c:pt><c:pt idx=\"1\"><c:v>1</c:v></c:pt><c:pt idx=\"2\"><c:v>2</c:v></c:pt></c:numCache>"), "{xml}");
    assert!(xml.contains("<c:f>Data!$1:$1</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"16384\"/><c:pt idx=\"0\"><c:v>3</c:v></c:pt></c:numCache>"));
    // Excel refuses files that name sheets they do not have.
    assert!(xml.contains("<c:tx><c:strRef><c:f>#REF!</c:f></c:strRef></c:tx>"));
    assert!(xml.contains("<c:cat><c:strRef><c:f>#REF!</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>#REF!</c:f></c:numRef></c:val>"));
    assert!(!xml.contains("Gone"));
    // Of a block: the first cell for a name, the first column (or row) for
    // values; the columns of the categories are levels of labels, the
    // one next to the values first.
    assert!(xml.contains("<c:tx><c:strRef><c:f>Sheet1!$A$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>g0</c:v></c:pt></c:strCache></c:strRef></c:tx>"));
    assert!(
        xml.contains("<c:cat><c:multiLvlStrRef><c:f>Sheet1!$A$1:$B$3</c:f><c:multiLvlStrCache><c:ptCount val=\"3\"/><c:lvl><c:pt idx=\"0\"><c:v>c0</c:v></c:pt><c:pt idx=\"1\"><c:v>c1</c:v></c:pt><c:pt idx=\"2\"><c:v>c2</c:v></c:pt></c:lvl><c:lvl><c:pt idx=\"0\"><c:v>g0</c:v></c:pt><c:pt idx=\"1\"><c:v>g1</c:v></c:pt><c:pt idx=\"2\"><c:v>g2</c:v></c:pt></c:lvl></c:multiLvlStrCache></c:multiLvlStrRef></c:cat><c:val><c:numRef><c:f>Sheet1!$C$1:$C$3</c:f>"),
        "{xml}"
    );
    // A name in several cells of a column is a name; text among the values
    // has no point.
    assert!(xml.contains("<c:tx><c:strRef><c:f>Sheet1!$A$1:$A$3</c:f><c:strCache><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>g0</c:v></c:pt><c:pt idx=\"1\"><c:v>g1</c:v></c:pt><c:pt idx=\"2\"><c:v>g2</c:v></c:pt></c:strCache></c:strRef></c:tx>"));
    assert!(xml.contains("<c:val><c:numRef><c:f>Sheet1!$A$1:$D$1</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"2\"><c:v>30</c:v></c:pt><c:pt idx=\"3\"><c:v>300</c:v></c:pt></c:numCache></c:numRef></c:val>"));
    assert!(xml.contains("<c:val><c:numRef><c:f>Data!$A$1:$A$1048576</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"1048576\"/><c:pt idx=\"0\"><c:v>3</c:v></c:pt>"));
    for block in ["$C$1:$D$3", "$A$1:$D$2", "XFD"] {
        assert!(!xml.contains(block), "{block} is no row or column");
    }
    // An empty title is still a title.
    assert!(xml.contains("<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr></a:p></c:rich></c:tx>"));

    let loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
    let c = &loaded.sheets[0].charts[0];
    assert_eq!(c.title, ChartTitle::Text(String::new()));
    let texts: Vec<_> = c
        .series
        .iter()
        .map(|s| {
            (
                match &s.name {
                    SeriesName::Cell(r) => r.to_text(),
                    other => format!("{other:?}"),
                },
                s.values.to_text(),
                s.categories.as_ref().map(RefExpr::to_text),
            )
        })
        .collect();
    // All but the series reading a sheet that is gone.
    let expected = [
        ("Sheet1!$B$1", "Sheet1!$C$1:$C$3", Some("Sheet1!$B$1:$B$3")),
        ("None", "data!$A:$A", None),
        ("None", "Data!$1:$1", None),
        ("Sheet1!$A$1", "Sheet1!$C$1:$C$3", Some("Sheet1!$A$1:$B$3")),
        ("Sheet1!$A$1:$A$3", "Sheet1!$A$1:$D$1", None),
        ("None", "Data!$A$1:$A$1048576", None),
    ];
    assert_eq!(texts.len(), expected.len());
    for (got, want) in texts.iter().zip(expected) {
        assert_eq!((got.0.as_str(), got.1.as_str(), got.2.as_deref()), want);
    }
    // What is written is what is read: a second trip changes nothing.
    assert_same_charts(&loaded, &reload(&loaded));
}

// ---- structure -------------------------------------------------------------------

/// What each element written by the chart and drawing writers may contain,
/// in the order the schemas of ECMA-376 demand (alternatives that share a
/// position are separated by `|`). Excel treats a part whose elements are
/// out of order as damaged.
const CONTENT: &[(&str, &[&str])] = &[
    (
        "c:chartSpace",
        &[
            "c:date1904",
            "c:lang",
            "c:roundedCorners",
            "c:style",
            "c:clrMapOvr",
            "c:pivotSource",
            "c:protection",
            "c:chart",
            "c:spPr",
            "c:txPr",
            "c:externalData",
            "c:printSettings",
            "c:userShapes",
            "c:extLst",
        ],
    ),
    (
        "c:chart",
        &[
            "c:title",
            "c:autoTitleDeleted",
            "c:pivotFmts",
            "c:view3D",
            "c:floor",
            "c:sideWall",
            "c:backWall",
            "c:plotArea",
            "c:legend",
            "c:plotVisOnly",
            "c:dispBlanksAs",
            "c:extLst",
        ],
    ),
    (
        "c:title",
        &[
            "c:tx",
            "c:layout",
            "c:overlay",
            "c:spPr",
            "c:txPr",
            "c:extLst",
        ],
    ),
    ("c:title/c:tx", &["c:strRef|c:rich"]),
    ("c:ser/c:tx", &["c:strRef|c:v"]),
    (
        "c:plotArea",
        &[
            "c:layout",
            "c:areaChart|c:area3DChart|c:barChart|c:bar3DChart|c:bubbleChart|c:doughnutChart|c:lineChart|c:line3DChart|c:pieChart|c:pie3DChart|c:radarChart|c:scatterChart",
            "c:catAx|c:valAx|c:dateAx|c:serAx",
            "c:dTable",
            "c:spPr",
            "c:extLst",
        ],
    ),
    (
        "c:barChart",
        &[
            "c:barDir",
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:gapWidth",
            "c:overlap",
            "c:serLines",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:bar3DChart",
        &[
            "c:barDir",
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:gapWidth",
            "c:gapDepth",
            "c:shape",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:line3DChart",
        &[
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:dropLines",
            "c:gapDepth",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:area3DChart",
        &[
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:dropLines",
            "c:gapDepth",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:pie3DChart",
        &["c:varyColors", "c:ser", "c:dLbls", "c:extLst"],
    ),
    (
        "c:bubbleChart",
        &[
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:bubble3D",
            "c:bubbleScale",
            "c:showNegBubbles",
            "c:sizeRepresents",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:radarChart",
        &[
            "c:radarStyle",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:lineChart",
        &[
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:dropLines",
            "c:hiLowLines",
            "c:upDownBars",
            "c:marker",
            "c:smooth",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:areaChart",
        &[
            "c:grouping",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:dropLines",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:pieChart",
        &[
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:firstSliceAng",
            "c:extLst",
        ],
    ),
    (
        "c:doughnutChart",
        &[
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:firstSliceAng",
            "c:holeSize",
            "c:extLst",
        ],
    ),
    (
        "c:scatterChart",
        &[
            "c:scatterStyle",
            "c:varyColors",
            "c:ser",
            "c:dLbls",
            "c:axId",
            "c:extLst",
        ],
    ),
    (
        "c:barChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:invertIfNegative",
            "c:pictureOptions",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:shape",
            "c:extLst",
        ],
    ),
    (
        "c:lineChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:marker",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:smooth",
            "c:extLst",
        ],
    ),
    (
        "c:areaChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:pictureOptions",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:extLst",
        ],
    ),
    (
        "c:pieChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:explosion",
            "c:dPt",
            "c:dLbls",
            "c:cat",
            "c:val",
            "c:extLst",
        ],
    ),
    (
        "c:doughnutChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:explosion",
            "c:dPt",
            "c:dLbls",
            "c:cat",
            "c:val",
            "c:extLst",
        ],
    ),
    (
        "c:scatterChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:marker",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:xVal",
            "c:yVal",
            "c:smooth",
            "c:extLst",
        ],
    ),
    (
        "c:bar3DChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:invertIfNegative",
            "c:pictureOptions",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:shape",
            "c:extLst",
        ],
    ),
    (
        "c:line3DChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:marker",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:smooth",
            "c:extLst",
        ],
    ),
    (
        "c:area3DChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:pictureOptions",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:cat",
            "c:val",
            "c:extLst",
        ],
    ),
    (
        "c:pie3DChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:explosion",
            "c:dPt",
            "c:dLbls",
            "c:cat",
            "c:val",
            "c:extLst",
        ],
    ),
    (
        "c:bubbleChart/c:ser",
        &[
            "c:idx",
            "c:order",
            "c:tx",
            "c:spPr",
            "c:invertIfNegative",
            "c:dPt",
            "c:dLbls",
            "c:trendline",
            "c:errBars",
            "c:xVal",
            "c:yVal",
            "c:bubbleSize",
            "c:bubble3D",
            "c:extLst",
        ],
    ),
    (
        "c:radarChart/c:ser",
        &[
            "c:idx", "c:order", "c:tx", "c:spPr", "c:marker", "c:dPt", "c:dLbls", "c:cat", "c:val",
            "c:extLst",
        ],
    ),
    ("c:marker", &["c:symbol", "c:size", "c:spPr", "c:extLst"]),
    (
        "c:dPt",
        &[
            "c:idx",
            "c:invertIfNegative",
            "c:marker",
            "c:bubble3D",
            "c:explosion",
            "c:spPr",
            "c:pictureOptions",
            "c:extLst",
        ],
    ),
    (
        "c:trendline",
        &[
            "c:name",
            "c:spPr",
            "c:trendlineType",
            "c:order",
            "c:period",
            "c:forward",
            "c:backward",
            "c:intercept",
            "c:dispRSqr",
            "c:dispEq",
            "c:trendlineLbl",
            "c:extLst",
        ],
    ),
    (
        "c:errBars",
        &[
            "c:errDir",
            "c:errBarType",
            "c:errValType",
            "c:noEndCap",
            "c:plus",
            "c:minus",
            "c:val",
            "c:spPr",
            "c:extLst",
        ],
    ),
    (
        "c:cat",
        &["c:multiLvlStrRef|c:numRef|c:numLit|c:strRef|c:strLit"],
    ),
    (
        "c:xVal",
        &["c:multiLvlStrRef|c:numRef|c:numLit|c:strRef|c:strLit"],
    ),
    ("c:val", &["c:numRef|c:numLit"]),
    ("c:yVal", &["c:numRef|c:numLit"]),
    ("c:bubbleSize", &["c:numRef|c:numLit"]),
    (
        "c:numLit",
        &["c:formatCode", "c:ptCount", "c:pt", "c:extLst"],
    ),
    ("c:strRef", &["c:f", "c:strCache", "c:extLst"]),
    (
        "c:multiLvlStrRef",
        &["c:f", "c:multiLvlStrCache", "c:extLst"],
    ),
    ("c:multiLvlStrCache", &["c:ptCount", "c:lvl", "c:extLst"]),
    ("c:lvl", &["c:pt"]),
    ("c:numRef", &["c:f", "c:numCache", "c:extLst"]),
    ("c:strCache", &["c:ptCount", "c:pt", "c:extLst"]),
    (
        "c:numCache",
        &["c:formatCode", "c:ptCount", "c:pt", "c:extLst"],
    ),
    ("c:pt", &["c:v"]),
    (
        "c:dLbls",
        &[
            "c:dLbl",
            // Either this alone, or what follows.
            "c:delete",
            "c:numFmt",
            "c:spPr",
            "c:txPr",
            "c:dLblPos",
            "c:showLegendKey",
            "c:showVal",
            "c:showCatName",
            "c:showSerName",
            "c:showPercent",
            "c:showBubbleSize",
            "c:separator",
            "c:showLeaderLines",
            "c:leaderLines",
            "c:extLst",
        ],
    ),
    (
        "c:catAx",
        &[
            "c:axId",
            "c:scaling",
            "c:delete",
            "c:axPos",
            "c:majorGridlines",
            "c:minorGridlines",
            "c:title",
            "c:numFmt",
            "c:majorTickMark",
            "c:minorTickMark",
            "c:tickLblPos",
            "c:spPr",
            "c:txPr",
            "c:crossAx",
            "c:crosses|c:crossesAt",
            "c:auto",
            "c:lblAlgn",
            "c:lblOffset",
            "c:tickLblSkip",
            "c:tickMarkSkip",
            "c:noMultiLvlLbl",
            "c:extLst",
        ],
    ),
    (
        "c:valAx",
        &[
            "c:axId",
            "c:scaling",
            "c:delete",
            "c:axPos",
            "c:majorGridlines",
            "c:minorGridlines",
            "c:title",
            "c:numFmt",
            "c:majorTickMark",
            "c:minorTickMark",
            "c:tickLblPos",
            "c:spPr",
            "c:txPr",
            "c:crossAx",
            "c:crosses|c:crossesAt",
            "c:crossBetween",
            "c:majorUnit",
            "c:minorUnit",
            "c:dispUnits",
            "c:extLst",
        ],
    ),
    (
        "c:serAx",
        &[
            "c:axId",
            "c:scaling",
            "c:delete",
            "c:axPos",
            "c:majorGridlines",
            "c:minorGridlines",
            "c:title",
            "c:numFmt",
            "c:majorTickMark",
            "c:minorTickMark",
            "c:tickLblPos",
            "c:spPr",
            "c:txPr",
            "c:crossAx",
            "c:crosses|c:crossesAt",
            "c:tickLblSkip",
            "c:tickMarkSkip",
            "c:extLst",
        ],
    ),
    (
        "c:scaling",
        &["c:logBase", "c:orientation", "c:max", "c:min", "c:extLst"],
    ),
    ("c:majorGridlines", &["c:spPr", "c:extLst"]),
    ("c:minorGridlines", &["c:spPr", "c:extLst"]),
    (
        "c:dispUnits",
        &["c:custUnit|c:builtInUnit", "c:dispUnitsLbl", "c:extLst"],
    ),
    ("c:dispUnitsLbl", &["c:layout", "c:tx", "c:spPr", "c:txPr"]),
    (
        "c:view3D",
        &[
            "c:rotX",
            "c:hPercent",
            "c:rotY",
            "c:depthPercent",
            "c:rAngAx",
            "c:perspective",
            "c:extLst",
        ],
    ),
    (
        "c:floor",
        &["c:thickness", "c:spPr", "c:pictureOptions", "c:extLst"],
    ),
    (
        "c:sideWall",
        &["c:thickness", "c:spPr", "c:pictureOptions", "c:extLst"],
    ),
    (
        "c:backWall",
        &["c:thickness", "c:spPr", "c:pictureOptions", "c:extLst"],
    ),
    (
        "c:dTable",
        &[
            "c:showHorzBorder",
            "c:showVertBorder",
            "c:showOutline",
            "c:showKeys",
            "c:spPr",
            "c:txPr",
            "c:extLst",
        ],
    ),
    ("c:legendEntry", &["c:idx", "c:delete|c:txPr", "c:extLst"]),
    (
        "c:legend",
        &[
            "c:legendPos",
            "c:legendEntry",
            "c:layout",
            "c:overlay",
            "c:spPr",
            "c:txPr",
            "c:extLst",
        ],
    ),
    (
        "c:spPr",
        &[
            "a:xfrm",
            "a:custGeom|a:prstGeom",
            "a:noFill|a:solidFill|a:gradFill|a:pattFill",
            "a:ln",
            "a:effectLst",
        ],
    ),
    ("a:effectLst", &["a:glow", "a:outerShdw", "a:softEdge"]),
    ("a:glow", &["a:srgbClr|a:schemeClr|a:prstClr"]),
    ("a:outerShdw", &["a:srgbClr|a:schemeClr|a:prstClr"]),
    ("a:prstClr", &["a:alpha"]),
    (
        "a:ln",
        &[
            "a:noFill|a:solidFill|a:gradFill|a:pattFill",
            "a:prstDash|a:custDash",
            "a:round|a:bevel|a:miter",
            "a:headEnd",
            "a:tailEnd",
        ],
    ),
    ("a:solidFill", &["a:srgbClr|a:schemeClr"]),
    ("a:srgbClr", &["a:alpha"]),
    ("a:schemeClr", &["a:lumMod", "a:lumOff", "a:alpha"]),
    ("c:rich", &["a:bodyPr", "a:lstStyle", "a:p"]),
    ("c:txPr", &["a:bodyPr", "a:lstStyle", "a:p"]),
    ("a:p", &["a:pPr", "a:r|a:br|a:fld", "a:endParaRPr"]),
    (
        "a:pPr",
        &[
            "a:lnSpc", "a:spcBef", "a:spcAft", "a:buNone", "a:tabLst", "a:defRPr",
        ],
    ),
    (
        "a:defRPr",
        &[
            "a:ln",
            "a:noFill|a:solidFill|a:gradFill|a:pattFill",
            "a:latin",
        ],
    ),
    ("a:r", &["a:rPr", "a:t"]),
    (
        "a:rPr",
        &[
            "a:ln",
            "a:noFill|a:solidFill|a:gradFill|a:pattFill",
            "a:latin",
        ],
    ),
    ("c:layout", &["c:manualLayout", "c:extLst"]),
    (
        "c:manualLayout",
        &[
            "c:layoutTarget",
            "c:xMode",
            "c:yMode",
            "c:wMode",
            "c:hMode",
            "c:x",
            "c:y",
            "c:w",
            "c:h",
            "c:extLst",
        ],
    ),
    (
        "c:dLbl",
        &[
            "c:idx",
            // Either this alone, or what follows.
            "c:delete",
            "c:layout",
            "c:tx",
            "c:numFmt",
            "c:spPr",
            "c:txPr",
            "c:dLblPos",
            "c:showLegendKey",
            "c:showVal",
            "c:showCatName",
            "c:showSerName",
            "c:showPercent",
            "c:showBubbleSize",
            "c:separator",
            "c:extLst",
        ],
    ),
    (
        "xdr:wsDr",
        &["xdr:twoCellAnchor|xdr:oneCellAnchor|xdr:absoluteAnchor"],
    ),
    (
        "xdr:twoCellAnchor",
        &[
            "xdr:from",
            "xdr:to",
            "xdr:sp|xdr:grpSp|xdr:graphicFrame|xdr:cxnSp|xdr:pic",
            "xdr:clientData",
        ],
    ),
    (
        "xdr:from",
        &["xdr:col", "xdr:colOff", "xdr:row", "xdr:rowOff"],
    ),
    (
        "xdr:to",
        &["xdr:col", "xdr:colOff", "xdr:row", "xdr:rowOff"],
    ),
    (
        "xdr:graphicFrame",
        &["xdr:nvGraphicFramePr", "xdr:xfrm", "a:graphic"],
    ),
    (
        "xdr:nvGraphicFramePr",
        &["xdr:cNvPr", "xdr:cNvGraphicFramePr"],
    ),
    ("xdr:xfrm", &["a:off", "a:ext"]),
    ("a:graphic", &["a:graphicData"]),
    ("a:graphicData", &["c:chart"]),
];

/// Elements that may occur once only where they occur several times in a
/// row would be accepted by [`CONTENT`].
const REPEATABLE: &[&str] = &[
    "c:lvl",
    "c:areaChart",
    "c:barChart",
    "c:lineChart",
    "c:scatterChart",
    "c:radarChart",
    "c:bubbleChart",
    "c:ser",
    "c:dPt",
    "c:dLbl",
    "c:trendline",
    "c:errBars",
    "c:pt",
    "c:axId",
    "c:catAx",
    "c:valAx",
    "c:serAx",
    "c:legendEntry",
    "a:p",
    "a:r",
    "xdr:twoCellAnchor",
];

fn content_of(parent: &str, grandparent: &str) -> &'static [&'static str] {
    let qualified = format!("{grandparent}/{parent}");
    CONTENT
        .iter()
        .find(|(name, _)| *name == qualified)
        .or_else(|| CONTENT.iter().find(|(name, _)| *name == parent))
        .unwrap_or_else(|| {
            panic!("{qualified}: the writer nests elements in an element not known to hold any")
        })
        .1
}

/// Check the nesting and order of the elements of a part against [`CONTENT`].
fn assert_schema_order(part_name: &str, xml: &str) {
    use quick_xml::events::Event;
    // (element, position of its last child in its content model, last child)
    let mut open: Vec<(String, usize, String)> = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        let (name, empty) = match reader.read_event().unwrap() {
            Event::Start(e) => (
                String::from_utf8(e.name().as_ref().to_vec()).unwrap(),
                false,
            ),
            Event::Empty(e) => (String::from_utf8(e.name().as_ref().to_vec()).unwrap(), true),
            Event::End(_) => {
                open.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let depth = open.len();
        if depth > 0 {
            let grandparent = if depth > 1 {
                open[depth - 2].0.clone()
            } else {
                String::new()
            };
            let (parent, position, last) = &mut open[depth - 1];
            let allowed = content_of(parent, &grandparent);
            let at = allowed
                .iter()
                .position(|slot| slot.split('|').any(|n| n == name))
                .unwrap_or_else(|| panic!("{part_name}: <{name}> is not allowed in <{parent}>"));
            assert!(
                at > *position
                    || (at == *position && REPEATABLE.contains(&name.as_str()))
                    || last.is_empty(),
                "{part_name}: <{name}> after <{last}> in <{parent}>"
            );
            *position = at;
            *last = name.clone();
        }
        if !empty {
            open.push((name, 0, String::new()));
        }
    }
}

#[test]
fn elements_come_in_schema_order() {
    let bytes = to_bytes(&sample());
    let mut checked = 0;
    for name in part_names(&bytes) {
        if (name.starts_with("xl/charts/") || name.starts_with("xl/drawings/"))
            && name.ends_with(".xml")
        {
            assert_schema_order(&name, &part(&bytes, &name));
            checked += 1;
        }
    }
    assert_eq!(checked, 14);

    // The check itself notices what it is there for.
    let chart = part(&bytes, "xl/charts/chart1.xml");
    for (good, bad) in [
        (
            "<c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>",
            "<c:grouping val=\"clustered\"/><c:barDir val=\"col\"/>",
        ),
        (
            "<c:delete val=\"0\"/><c:axPos val=\"b\"/>",
            "<c:axPos val=\"b\"/><c:delete val=\"0\"/>",
        ),
        (
            "<c:plotVisOnly val=\"1\"/>",
            "<c:plotVisOnly val=\"1\"/><c:plotVisOnly val=\"1\"/>",
        ),
        ("<c:invertIfNegative val=\"0\"/>", "<c:smooth val=\"0\"/>"),
    ] {
        assert!(chart.contains(good));
        let broken = chart.replacen(good, bad, 1);
        assert!(
            std::panic::catch_unwind(|| assert_schema_order("broken", &broken)).is_err(),
            "{bad} should be refused"
        );
    }
}

// ---- files from other applications -----------------------------------------------

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_C: &str = "xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn relationships(list: &[(&str, &str, &str)]) -> String {
    let mut xml = format!("{DECL}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in list {
        let kind = if kind.contains("://") {
            kind.to_string()
        } else {
            format!("{REL}/{kind}")
        };
        xml.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

/// The Office 2013 theme, as far as colours go.
const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office">
<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
<a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
<a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4>
<a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6>
<a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
</a:clrScheme></a:themeElements></a:theme>"#;

fn foreign_sheet() -> String {
    format!(
        r#"{DECL}<worksheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision">
<dimension ref="A1:D4"/>
<sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews>
<sheetFormatPr baseColWidth="10" defaultColWidth="10" defaultRowHeight="18" x14ac:dyDescent="0.2"/>
<cols><col min="1" max="1" width="20" customWidth="1"/><col min="8" max="8" width="10" hidden="1"/></cols>
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Year</t></is></c><c r="B1" t="inlineStr"><is><t>Quarter</t></is></c><c r="C1" t="inlineStr"><is><t>North</t></is></c><c r="D1" t="inlineStr"><is><t>South</t></is></c></row>
<row r="2"><c r="A2"><v>2024</v></c><c r="B2" t="inlineStr"><is><t>Q1</t></is></c><c r="C2"><v>10</v></c><c r="D2"><v>7</v></c></row>
<row r="3"><c r="B3" t="inlineStr"><is><t>Q2</t></is></c><c r="C3"><v>20</v></c><c r="D3"><v>9</v></c></row>
<row r="4"><c r="A4"><v>2025</v></c><c r="B4" t="inlineStr"><is><t>Q3</t></is></c><c r="C4"><v>30</v></c><c r="D4"><v>4</v></c></row>
</sheetData>
<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>
<drawing r:id="rId1"/><legacyDrawing r:id="rId2"/>
</worksheet>"#
    )
}

/// A graphic frame as Excel writes it. `kind` is the namespace prefix of
/// the chart element: `c` for charts, `cx` for the newer kinds.
fn graphic_frame(id: u32, kind: &str, rel: &str) -> String {
    let (uri, ns) = if kind == "c" {
        (
            "http://schemas.openxmlformats.org/drawingml/2006/chart",
            "http://schemas.openxmlformats.org/drawingml/2006/chart",
        )
    } else {
        (
            "http://schemas.microsoft.com/office/drawing/2014/chartex",
            "http://schemas.microsoft.com/office/drawing/2014/chartex",
        )
    };
    format!(
        r#"<xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="{id}" name="Chart {id}"><a:extLst><a:ext uri="{{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}}"><a16:creationId xmlns:a16="http://schemas.microsoft.com/office/drawing/2014/main" id="{{00000000-0008-0000-0000-00000{id}000000}}"/></a:ext></a:extLst></xdr:cNvPr><xdr:cNvGraphicFramePr><a:graphicFrameLocks/></xdr:cNvGraphicFramePr></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="{uri}"><{kind}:chart xmlns:{kind}="{ns}" xmlns:r="{REL}" r:id="{rel}"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/>"#
    )
}

/// Every kind of anchor, a picture, and a chart of a kind old versions of
/// Excel are offered a text box for.
fn foreign_drawing() -> String {
    format!(
        r#"{DECL}<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>5</xdr:col><xdr:colOff>190500</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>95250</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>11</xdr:col><xdr:colOff>381000</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  {first}
</xdr:twoCellAnchor>
<xdr:oneCellAnchor>
  <xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:ext cx="4572000" cy="2743200"/>
  {second}
</xdr:oneCellAnchor>
<xdr:absoluteAnchor>
  <xdr:pos x="952500" y="1905000"/><xdr:ext cx="3810000" cy="1905000"/>
  {third}
</xdr:absoluteAnchor>
<xdr:twoCellAnchor editAs="oneCell">
  <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  <xdr:pic><xdr:nvPicPr><xdr:cNvPr id="9" name="Picture 1"/><xdr:cNvPicPr><a:picLocks noChangeAspect="1"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r="{REL}" r:embed="rId9"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="100"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>
  <xdr:clientData/>
</xdr:twoCellAnchor>
<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:cx1="http://schemas.microsoft.com/office/drawing/2015/9/8/chartex" Requires="cx1">
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>18</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>14</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  {newer}
</xdr:twoCellAnchor></mc:Choice><mc:Fallback>
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>0</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>18</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>14</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  <xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="0" name=""/><xdr:cNvSpPr><a:spLocks noTextEdit="1"/></xdr:cNvSpPr></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="4572000" cy="2743200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr><xdr:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>This chart isn't available in your version of Excel.</a:t></a:r></a:p></xdr:txBody></xdr:sp>
  <xdr:clientData/>
</xdr:twoCellAnchor></mc:Fallback></mc:AlternateContent>
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>30</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>40</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  {fourth}
</xdr:twoCellAnchor>
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>50</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>60</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  {unknown}
</xdr:twoCellAnchor>
<xdr:twoCellAnchor>
  <xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>70</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>
  <xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>80</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>
  {missing}
</xdr:twoCellAnchor>
</xdr:wsDr>"#,
        first = graphic_frame(2, "c", "rId1"),
        second = graphic_frame(3, "c", "rId2"),
        third = graphic_frame(4, "c", "rId3"),
        newer = graphic_frame(5, "cx", "rId5"),
        fourth = graphic_frame(6, "c", "rId6"),
        unknown = graphic_frame(7, "c", "rId7"),
        missing = graphic_frame(8, "c", "rId8"),
    )
}

/// The formatting Excel attaches to about everything.
const SHAPE: &str = r#"<c:spPr><a:noFill/><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill><a:round/></a:ln><a:effectLst/></c:spPr>"#;
const TEXT: &str = r#"<c:txPr><a:bodyPr rot="-60000000" spcFirstLastPara="1" vertOverflow="ellipsis" vert="horz" wrap="square" anchor="ctr" anchorCtr="1"/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="900" b="0" i="0" u="none" strike="noStrike" kern="1200" baseline="0"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr>"#;

fn axes(first: u32, second: u32) -> String {
    format!(
        r#"<c:catAx><c:axId val="{first}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>An axis title, not the chart's</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/>{SHAPE}{TEXT}<c:crossAx val="{second}"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/><c:noMultiLvlLbl val="0"/></c:catAx>
<c:valAx><c:axId val="{second}"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines>{SHAPE}</c:majorGridlines><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/>{SHAPE}{TEXT}<c:crossAx val="{first}"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>"#
    )
}

fn unique_id(n: u32) -> String {
    format!(
        r#"<c:extLst><c:ext uri="{{C3380CC4-5D6E-409C-BE32-E72D297353CC}}" xmlns:c16="http://schemas.microsoft.com/office/drawing/2014/chart"><c16:uniqueId val="{{0000000{n}-AF0B-4A4B-9A4B-0D1B7D6F0F41}}"/></c:ext></c:extLst>"#
    )
}

/// Clustered columns with the wrapping Excel 2016 and later give a chart:
/// a style in two dialects, formatting everywhere, extension lists.
fn excel_columns() -> String {
    format!(
        r#"{DECL}<c:chartSpace {NS_C} xmlns:c16r2="http://schemas.microsoft.com/office/drawing/2015/06/chart"><c:date1904 val="0"/><c:lang val="en-US"/><c:roundedCorners val="0"/>
<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="c14" xmlns:c14="http://schemas.microsoft.com/office/drawing/2007/8/2/chart"><c14:style val="102"/></mc:Choice><mc:Fallback><c:style val="2"/></mc:Fallback></mc:AlternateContent>
<c:chart><c:title><c:tx><c:rich><a:bodyPr rot="0" spcFirstLastPara="1" vertOverflow="ellipsis" vert="horz" wrap="square" anchor="ctr" anchorCtr="1"/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1400" b="0" i="0" u="none" strike="noStrike" kern="1200" spc="0" baseline="0"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:pPr><a:r><a:rPr lang="en-US"/><a:t>Sales </a:t></a:r><a:r><a:rPr lang="en-US" b="1"/><a:t>by region</a:t></a:r><a:br><a:rPr lang="en-US"/></a:br><a:r><a:rPr lang="en-US"/><a:t>&lt;units &amp; more&gt;</a:t></a:r><a:endParaRPr lang="en-US"/></a:p></c:rich></c:tx><c:overlay val="0"/>{SHAPE}{TEXT}</c:title>
<c:autoTitleDeleted val="0"/>
<c:plotArea><c:layout><c:manualLayout><c:layoutTarget val="inner"/><c:xMode val="edge"/><c:yMode val="edge"/><c:x val="0.1"/><c:y val="0.2"/><c:w val="0.8"/><c:h val="0.6"/></c:manualLayout></c:layout>
<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Data!$C$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>North</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val="0"/>
<c:dPt><c:idx val="1"/><c:invertIfNegative val="0"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr></c:dPt>
<c:cat><c:strRef><c:f>Data!$B$2:$B$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat>
<c:val><c:numRef><c:f>Data!$C$2:$C$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt><c:pt idx="2"><c:v>30</c:v></c:pt></c:numCache></c:numRef></c:val>{id0}</c:ser>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:strRef><c:f>Data!$D$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>South</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val="0"/>
<c:trendline><c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="00FF00"/></a:solidFill></a:ln></c:spPr><c:trendlineType val="linear"/><c:dispRSqr val="0"/><c:dispEq val="0"/></c:trendline>
<c:cat><c:strRef><c:f>Data!$B$2:$B$4</c:f></c:strRef></c:cat>
<c:val><c:numRef><c:f>Data!$D$2:$D$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>7</c:v></c:pt><c:pt idx="1"><c:v>9</c:v></c:pt><c:pt idx="2"><c:v>4</c:v></c:pt></c:numCache></c:numRef></c:val>
<c:extLst><c:ext uri="{{02D57815-91ED-43cb-92C2-25804820EDAC}}" xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart"><c15:filteredSeriesTitle><c15:tx><c:strRef><c:f>Data!$A$1</c:f></c:strRef></c15:tx></c15:filteredSeriesTitle></c:ext><c:ext uri="{{02D57815-91ED-43cb-92C2-25804820EDAC}}" xmlns:c15="http://schemas.microsoft.com/office/drawing/2012/chart"><c15:datalabelsRange><c15:f>Data!$A$2:$A$4</c15:f></c15:datalabelsRange></c:ext></c:extLst></c:ser>
<c:dLbls><c:showLegendKey val="0"/><c:showVal val="0"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="0"/><c:showBubbleSize val="0"/></c:dLbls>
<c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="1458229759"/><c:axId val="1458231407"/></c:barChart>
{axes}
{SHAPE}</c:plotArea>
<c:legend><c:legendPos val="b"/><c:overlay val="0"/>{SHAPE}{TEXT}</c:legend>
<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/>
<c:extLst><c:ext uri="{{56B9EC1D-385E-4148-901F-78D8002777C0}}" xmlns:c16r3="http://schemas.microsoft.com/office/drawing/2017/03/chart"><c16r3:dataDisplayOptions16><c16r3:dispNaAsBlank val="1"/></c16r3:dataDisplayOptions16></c:ext></c:extLst>
</c:chart>{SHAPE}<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr>
<c:printSettings><c:headerFooter/><c:pageMargins b="0.75" l="0.7" r="0.7" t="0.75" header="0.3" footer="0.3"/><c:pageSetup/></c:printSettings>
</c:chartSpace>"#,
        id0 = unique_id(0),
        axes = axes(1458229759, 1458231407),
    )
}

/// Bars stacked to 100 % with a line over them on a second pair of axes;
/// the line comes second in the chart's order although its plot is last.
fn excel_combo() -> String {
    format!(
        r#"{DECL}<c:chartSpace {NS_C}><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/>
<c:barChart><c:barDir val="bar"/><c:grouping val="percentStacked"/><c:varyColors val="0"/>
<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>'Other Sheet'!$A$1</c:f></c:strRef></c:tx><c:invertIfNegative val="0"/><c:cat><c:numRef><c:f>'Other Sheet'!$B$2:$B$3</c:f></c:numRef></c:cat><c:val><c:numRef><c:f>'Other Sheet'!$A$2:$A$3</c:f></c:numRef></c:val></c:ser>
<c:ser><c:idx val="2"/><c:order val="2"/><c:spPr><a:pattFill prst="pct50"><a:fgClr><a:srgbClr val="FF0000"/></a:fgClr><a:bgClr><a:srgbClr val="FFFFFF"/></a:bgClr></a:pattFill><a:ln w="12700"><a:solidFill><a:sysClr val="windowText" lastClr="112233"/></a:solidFill></a:ln></c:spPr><c:invertIfNegative val="0"/>
<c:dLbls><c:spPr><a:solidFill><a:srgbClr val="ABCDEF"/></a:solidFill></c:spPr><c:showLegendKey val="0"/><c:showVal val="1"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="0"/><c:showBubbleSize val="0"/></c:dLbls>
<c:val><c:numRef><c:f>Data!$D$2:$D$4</c:f></c:numRef></c:val></c:ser>
<c:gapWidth val="150"/><c:overlap val="100"/><c:axId val="1"/><c:axId val="2"/></c:barChart>
<c:lineChart><c:grouping val="standard"/><c:varyColors val="0"/>
<c:ser><c:idx val="1"/><c:order val="1"/><c:tx><c:v>Trend _x0041_ &amp; co</c:v></c:tx><c:spPr><a:ln w="28575" cap="rnd"><a:solidFill><a:schemeClr val="accent3"><a:lumMod val="75000"/></a:schemeClr></a:solidFill><a:round/></a:ln></c:spPr><c:marker><c:symbol val="circle"/><c:size val="5"/><c:spPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></c:spPr></c:marker><c:val><c:numRef><c:f>(Data!$C$2:$C$4)</c:f></c:numRef></c:val><c:smooth val="0"/></c:ser>
<c:marker val="1"/><c:axId val="3"/><c:axId val="4"/></c:lineChart>
{axes}{more}
</c:plotArea><c:legend><c:legendPos val="tr"/></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>"#,
        axes = axes(1, 2),
        more = axes(3, 4),
    )
}

/// A pie in three dimensions: a colour per slice, categories in two
/// levels, a title read from a cell, percentages as labels, and a legend
/// that does not say where it is.
fn excel_pie() -> String {
    format!(
        r#"{DECL}<c:chartSpace {NS_C}><c:chart><c:title><c:tx><c:strRef><c:f>Data!$C$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>North</c:v></c:pt></c:strCache></c:strRef></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/>
<c:view3D><c:rotX val="30"/><c:rotY val="0"/><c:rAngAx val="0"/></c:view3D>
<c:plotArea><c:layout/><c:pie3DChart><c:varyColors val="1"/>
<c:ser><c:idx val="0"/><c:order val="0"/>
<c:dPt><c:idx val="0"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></c:spPr></c:dPt>
<c:dPt><c:idx val="1"/><c:bubble3D val="0"/><c:spPr><a:solidFill><a:schemeClr val="accent2"/></a:solidFill></c:spPr></c:dPt>
<c:dLbls><c:dLbl><c:idx val="0"/><c:delete val="1"/></c:dLbl><c:showLegendKey val="0"/><c:showVal val="0"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="1"/><c:showBubbleSize val="0"/><c:showLeaderLines val="1"/></c:dLbls>
<c:cat><c:multiLvlStrRef><c:f>Data!$A$2:$B$4</c:f><c:multiLvlStrCache><c:ptCount val="3"/><c:lvl><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:lvl><c:lvl><c:pt idx="0"><c:v>2024</c:v></c:pt><c:pt idx="2"><c:v>2025</c:v></c:pt></c:lvl></c:multiLvlStrCache></c:multiLvlStrRef></c:cat>
<c:val><c:numRef><c:f>Data!$C$2:$C$4</c:f></c:numRef></c:val></c:ser>
<c:dLbls><c:delete val="1"/></c:dLbls></c:pie3DChart></c:plotArea>
<c:legend><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>"#
    )
}

/// A scatter chart as a tool writing the bare minimum would: no title
/// elements at all, series whose data are not ranges of this workbook.
fn other_scatter() -> String {
    format!(
        r#"{DECL}<c:chartSpace {NS_C}>
  <c:chart>
    <c:plotArea>
      <c:scatterChart>
        <c:scatterStyle val="lineMarker"/>
        <c:ser>
          <c:idx val="0"/>
          <c:tx><c:strRef><c:f>[1]Sheet1!$A$1</c:f><c:strCache><c:ptCount val="2"/><c:pt idx="0"><c:v>Elsewhere</c:v></c:pt><c:pt idx="1"><c:v>kept</c:v></c:pt></c:strCache></c:strRef></c:tx>
          <c:spPr><a:ln w="19050"><a:noFill/></a:ln></c:spPr>
          <c:marker><c:symbol val="circle"/><c:spPr><a:solidFill><a:srgbClr val="7030A0"><a:alpha val="50000"/></a:srgbClr></a:solidFill></c:spPr></c:marker>
          <c:xVal><c:numRef><c:f>Data!$C$2:$C$4</c:f></c:numRef></c:xVal>
          <c:yVal><c:numRef><c:f>Data!$D$2:$D$4</c:f></c:numRef></c:yVal>
        </c:ser>
        <c:ser>
          <c:idx val="1"/>
          <c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:xVal>
          <c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>4</c:v></c:pt></c:numLit></c:yVal>
        </c:ser>
        <c:ser>
          <c:idx val="2"/>
          <c:yVal><c:numRef><c:f>[1]Sheet1!$B$1:$B$3</c:f></c:numRef></c:yVal>
        </c:ser>
        <c:ser>
          <c:idx val="3"/>
          <c:xVal><c:numRef><c:f>Book1.xlsx!SomeName</c:f></c:numRef></c:xVal>
          <c:yVal><c:numRef><c:f>Data!$C:$C</c:f></c:numRef></c:yVal>
        </c:ser>
        <c:axId val="1"/>
        <c:axId val="2"/>
      </c:scatterChart>
      <c:valAx><c:axId val="1"/><c:scaling/><c:axPos val="b"/><c:crossAx val="2"/></c:valAx>
      <c:valAx><c:axId val="2"/><c:scaling/><c:axPos val="l"/><c:crossAx val="1"/></c:valAx>
    </c:plotArea>
  </c:chart>
</c:chartSpace>"#
    )
}

fn foreign_package(extra: &[(&str, &str)]) -> Vec<u8> {
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/><Override PartName="/xl/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/><Override PartName="/xl/charts/style1.xml" ContentType="application/vnd.ms-office.chartstyle+xml"/><Override PartName="/xl/charts/colors1.xml" ContentType="application/vnd.ms-office.chartcolorstyle+xml"/><Override PartName="/xl/charts/chartEx1.xml" ContentType="application/vnd.ms-office.chartex+xml"/></Types>"#
    );
    let workbook = format!(
        r#"{DECL}<workbook {MAIN}><sheets><sheet name="Data" sheetId="1" r:id="rId1"/><sheet name="Other Sheet" sheetId="2" r:id="rId2"/><sheet name="A chart sheet" sheetId="3" r:id="rId3"/></sheets></workbook>"#
    );
    let other = format!(
        r#"{DECL}<worksheet {MAIN}><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Elsewhere</t></is></c></row><row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>5</v></c></row><row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>6</v></c></row></sheetData></worksheet>"#
    );
    // A sheet that is nothing but a chart: not a sheet the engine has.
    let chartsheet = format!(
        r#"{DECL}<chartsheet {MAIN}><sheetViews><sheetView workbookViewId="0"/></sheetViews><drawing r:id="rId1"/></chartsheet>"#
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
                ("rId3", "chartsheet", "chartsheets/sheet1.xml"),
                ("rId4", "theme", "theme/theme1.xml"),
            ]),
        ),
        ("xl/theme/theme1.xml", THEME.to_string()),
        ("xl/worksheets/sheet1.xml", foreign_sheet()),
        ("xl/worksheets/sheet2.xml", other),
        ("xl/chartsheets/sheet1.xml", chartsheet),
        (
            "xl/chartsheets/_rels/sheet1.xml.rels",
            relationships(&[("rId1", "drawing", "../drawings/drawing2.xml")]),
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            relationships(&[
                ("rId2", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
                ("rId1", "drawing", "/xl/drawings/drawing1.xml"),
            ]),
        ),
        ("xl/drawings/drawing1.xml", foreign_drawing()),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            relationships(&[
                ("rId1", "chart", "../charts/chart1.xml"),
                ("rId2", "chart", "../charts/chart2.xml"),
                ("rId3", "chart", "/xl/charts/chart3.xml"),
                (
                    "rId5",
                    "http://schemas.microsoft.com/office/2014/relationships/chartEx",
                    "../charts/chartEx1.xml",
                ),
                ("rId6", "chart", "../charts/chart4.xml"),
                ("rId7", "chart", "../charts/chart5.xml"),
                ("rId8", "chart", "../charts/chart6.xml"),
                ("rId9", "image", "../media/image1.png"),
            ]),
        ),
        ("xl/charts/chart1.xml", excel_columns()),
        (
            "xl/charts/_rels/chart1.xml.rels",
            relationships(&[
                (
                    "rId1",
                    "http://schemas.microsoft.com/office/2011/relationships/chartStyle",
                    "style1.xml",
                ),
                (
                    "rId2",
                    "http://schemas.microsoft.com/office/2011/relationships/chartColorStyle",
                    "colors1.xml",
                ),
            ]),
        ),
        (
            "xl/charts/style1.xml",
            format!(r#"{DECL}<cs:chartStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" id="201"><cs:title><cs:lnRef idx="0"/><cs:fillRef idx="0"/><cs:effectRef idx="0"/><cs:fontRef idx="minor"><a:schemeClr val="tx1"/></cs:fontRef></cs:title></cs:chartStyle>"#),
        ),
        (
            "xl/charts/colors1.xml",
            format!(r#"{DECL}<cs:colorStyle xmlns:cs="http://schemas.microsoft.com/office/drawing/2012/chartStyle" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" meth="cycle" id="10"><a:schemeClr val="accent1"/><a:schemeClr val="accent2"/><cs:variation/></cs:colorStyle>"#),
        ),
        ("xl/charts/chart2.xml", excel_combo()),
        ("xl/charts/chart3.xml", excel_pie()),
        ("xl/charts/chart4.xml", other_scatter()),
        // A waterfall: a chart of the newer kind, in a part of another type.
        (
            "xl/charts/chartEx1.xml",
            format!(r#"{DECL}<cx:chartSpace xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><cx:chartData><cx:data id="0"><cx:numDim type="val"><cx:f>Data!$C$2:$C$4</cx:f></cx:numDim></cx:data></cx:chartData><cx:chart><cx:plotArea><cx:plotAreaRegion><cx:series layoutId="waterfall"><cx:dataId val="0"/></cx:series></cx:plotAreaRegion></cx:plotArea></cx:chart></cx:chartSpace>"#),
        ),
        // A chart part with no plot the engine knows.
        (
            "xl/charts/chart5.xml",
            format!(r#"{DECL}<c:chartSpace {NS_C}><c:chart><c:plotArea><c:layout/><c:futureChart><c:ser><c:val><c:numRef><c:f>Data!$C$2:$C$4</c:f></c:numRef></c:val></c:ser></c:futureChart></c:plotArea></c:chart></c:chartSpace>"#),
        ),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_string(), text))
    .collect();
    for (name, text) in extra {
        match parts.iter_mut().find(|(n, _)| n == name) {
            Some(existing) => existing.1 = text.to_string(),
            None => parts.push((name.to_string(), text.to_string())),
        }
    }
    let parts: Vec<(&str, &str)> = parts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    package(&parts)
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn refs(s: &Series) -> (String, Option<String>) {
    (
        s.values.to_text(),
        s.categories.as_ref().map(RefExpr::to_text),
    )
}

#[test]
fn charts_from_other_applications() {
    let bytes = foreign_package(&[]);
    keep("foreign-charts.xlsx", &bytes);
    for name in part_names(&bytes) {
        assert_well_formed(&name, &part(&bytes, &name));
    }
    let wb = load(&bytes);
    assert_eq!(wb.sheets.len(), 2, "the chart sheet is not a sheet");
    assert!(wb.sheets[1].charts.is_empty());
    let charts = &wb.sheets[0].charts;
    // Four of the seven frames hold a chart the engine can show: not the
    // waterfall, not the plot of an unknown type, not the missing part.
    assert_eq!(charts.len(), 4);
    assert_eq!(
        charts.iter().map(|c| c.id).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );

    // Excel's clustered columns.
    let columns = &charts[0];
    assert_eq!(columns.kind, ChartKind::Column);
    assert_eq!(
        columns.title,
        ChartTitle::Text("Sales by region\n<units & more>".into())
    );
    assert_eq!(columns.legend, LegendPos::Bottom);
    assert!(!columns.data_labels);
    assert_eq!(columns.series.len(), 2);
    assert_eq!(columns.series[0].name, named("Data!$C$1"));
    assert_eq!(
        refs(&columns.series[0]),
        ("Data!$C$2:$C$4".into(), Some("Data!$B$2:$B$4".into()))
    );
    assert_eq!(columns.series[1].name, named("Data!$D$1"));
    assert_eq!(
        refs(&columns.series[1]),
        ("Data!$D$2:$D$4".into(), Some("Data!$B$2:$B$4".into()))
    );
    // The accents of the theme of the file are the automatic colours of
    // its series; the red of one point; the green of a trend line, which
    // is solid. The outlines Excel says are not there are the automatic
    // ones.
    assert_eq!(columns.series[0].color, None);
    assert_eq!(columns.series[1].color, None);
    let data = wb.chart_data(0, columns);
    assert_eq!(
        (data.series[0].color, data.series[1].color),
        (Color(0x4472C4), Color(0xED7D31))
    );
    assert_eq!(columns.series[0].points, [point(1, Some(0xFF0000), None)]);
    assert_eq!(columns.series[0].line, Line::default());
    assert_eq!(
        columns.series[1].trendlines,
        [Trendline {
            line: Line {
                color: Some(Color(0x00FF00)),
                ..Line::default()
            },
            ..Trendline::new(TrendKind::Linear)
        }]
    );
    // What Excel writes for its defaults is automatic in the model.
    assert_eq!((columns.gap_width, columns.overlap), (None, None));
    assert_eq!(columns.vary_colors, None);
    assert_eq!(
        columns.axes.category,
        Axis {
            title: Some("An axis title, not the chart's".into()),
            ..Axis::new()
        }
    );
    // The scale has the line of the other axis, which Excel's own has not.
    assert_eq!(
        columns.axes.value,
        Axis {
            line: Line {
                shown: Some(true),
                ..Line::default()
            },
            ..Axes::for_kind(ChartKind::Column).value
        }
    );
    // A chart area without a fill in the frame Excel draws; a plot area
    // with the same frame, which it does not have by itself.
    assert_eq!(
        columns.chart_area,
        ShapeFormat {
            fill: Fill::None,
            border: Line::default(),
            ..ShapeFormat::default()
        }
    );
    assert_eq!(
        columns.plot_area,
        ShapeFormat {
            fill: Fill::Auto,
            border: Line {
                shown: Some(true),
                ..Line::default()
            },
            ..ShapeFormat::default()
        }
    );
    let data = wb.chart_data(0, columns);
    assert_eq!(data.categories, ["Q1", "Q2", "Q3"]);
    assert_eq!(data.series[0].name, "North");
    assert_eq!(data.series[1].values, [Some(7.0), Some(9.0), Some(4.0)]);
    // From 20, 10 inside F3 to 40 pixels into L17: columns of 70 pixels
    // (one of them hidden) and rows of 24, as the sheet declares.
    assert_eq!(columns.anchor, (2, 5));
    assert_eq!(columns.offset, (20.0, 10.0));
    assert_eq!(columns.size, (5.0 * 70.0 + 40.0 - 20.0, 14.0 * 24.0 - 10.0));

    // Bars stacked to 100 % plus a line on axes of its own, with all three
    // series in the chart's order. Horizontal bars and lines do not mix in
    // the engine: the line is a series like the others, on the secondary
    // axes.
    let combo = &charts[1];
    assert_eq!(combo.kind, ChartKind::PercentStackedBar);
    assert_eq!(combo.title, ChartTitle::None);
    assert_eq!(combo.legend, LegendPos::TopRight);
    assert_eq!(combo.series.len(), 3);
    assert_eq!(combo.series[0].name, named("'Other Sheet'!$A$1"));
    assert_eq!(
        refs(&combo.series[0]),
        (
            "'Other Sheet'!$A$2:$A$3".into(),
            Some("'Other Sheet'!$B$2:$B$3".into())
        )
    );
    assert_eq!(combo.series[0].color, None);
    assert_eq!(
        combo.series[1].name,
        SeriesName::Text("Trend A & co".into())
    );
    assert_eq!(refs(&combo.series[1]), ("Data!$C$2:$C$4".into(), None));
    // The stroke of the line: accent 3, a quarter darker, which stays a
    // colour of the theme (with the value cells give that variant, a unit
    // away from the 7C7C7C of a chart).
    assert_eq!(combo.series[1].color, Some(wb.theme().accent(3, -0.25)));
    assert_eq!(combo.series[1].color.map(|c| c.value()), Some(0x7B7B7B));
    assert_eq!(
        combo
            .series
            .iter()
            .map(|s| (s.kind, s.secondary))
            .collect::<Vec<_>>(),
        [(None, false), (None, true), (None, false)]
    );
    assert_eq!(combo.series[1].line, Line::default());
    assert_eq!(combo.series[1].marker, None);
    assert_eq!(combo.series[2].name, SeriesName::None);
    assert_eq!(refs(&combo.series[2]), ("Data!$D$2:$D$4".into(), None));
    assert_eq!(
        combo.series[2].color,
        Some(Color(0xFF0000)),
        "of a pattern, the foreground"
    );
    assert_eq!(
        combo.series[2].line,
        Line {
            shown: None,
            color: Some(Color(0x112233)),
            width: Some(1.0),
            dash: Dash::Solid,
        },
        "an outline: having a colour, it is there"
    );
    // One series is labelled: that is not the chart's doing.
    assert!(!combo.data_labels);
    assert_eq!(combo.label_options, DataLabels::default());
    assert_eq!(
        combo
            .series
            .iter()
            .map(|s| s.labels.clone())
            .collect::<Vec<_>>(),
        [None, None, Some(DataLabels::default())]
    );
    // The second pair of axes: a category axis that is shown, crossed by
    // its scale at the near end.
    assert!(combo.has_secondary_axis());
    assert_eq!(
        combo.axes.secondary_category,
        Axis {
            title: Some("An axis title, not the chart's".into()),
            crossing: AxisCrossing::Min,
            ..Axis::new()
        }
    );
    // That category axis crosses its scale where the first one does,
    // which is not where a second category axis is by itself.
    assert_eq!(
        combo.axes.secondary_value,
        Axis {
            major_gridlines: true,
            crossing: AxisCrossing::Min,
            // The fixture gives every axis a line.
            line: Line {
                shown: Some(true),
                ..Line::default()
            },
            ..Axis::new()
        }
    );
    assert_eq!((combo.gap_width, combo.overlap), (None, None));
    assert_eq!(wb.chart_data(0, combo).series[0].name, "Elsewhere");
    assert_eq!((combo.anchor, combo.offset), ((20, 1), (0.0, 0.0)));
    assert_eq!(combo.size, (480.0, 288.0));

    // The 3-D pie.
    let pie = &charts[2];
    assert_eq!(pie.kind, ChartKind::Pie3D);
    assert_eq!(pie.view3d, None, "Excel's own camera");
    assert_eq!(pie.first_slice_angle, 0);
    assert_eq!(pie.title, ChartTitle::Text("North".into()));
    assert_eq!(pie.legend, LegendPos::Right);
    // The labels of its only series are those of the chart.
    assert!(pie.data_labels, "percentages are labels");
    assert_eq!(
        pie.label_options,
        DataLabels {
            value: false,
            percent: true,
            ..DataLabels::default()
        }
    );
    assert_eq!(pie.series.len(), 1);
    assert_eq!(pie.series[0].labels, None);
    assert_eq!(
        refs(&pie.series[0]),
        ("Data!$C$2:$C$4".into(), Some("Data!$A$2:$B$4".into())),
        "the two levels of categories"
    );
    assert_eq!(pie.series[0].color, None);
    assert_eq!(
        pie.series[0].points,
        [],
        "the colours of another theme are colours of their own"
    );
    assert_eq!(wb.chart_data(0, pie).categories, ["Q1", "Q2", "Q3"]);
    // 100, 200 on the sheet: inside the first column (140 wide), in the
    // ninth row.
    assert_eq!((pie.anchor, pie.offset), ((8, 0), (100.0, 8.0)));
    assert_eq!(pie.size, (400.0, 200.0));

    // The minimal scatter chart.
    let scatter = &charts[3];
    assert_eq!(scatter.kind, ChartKind::Scatter);
    // No title element and more than one series: Excel shows no title.
    assert_eq!(scatter.title, ChartTitle::None);
    assert_eq!(scatter.legend, LegendPos::None);
    assert!(!scatter.data_labels);
    // Typed-in numbers and the range of another workbook are no series.
    assert_eq!(scatter.series.len(), 2);
    assert_eq!(
        scatter.series[0].name,
        SeriesName::Text("Elsewhere kept".into()),
        "a name out of reach keeps its last text"
    );
    assert_eq!(
        refs(&scatter.series[0]),
        ("Data!$D$2:$D$4".into(), Some("Data!$C$2:$C$4".into()))
    );
    // Markers filled in a colour (its transparency is not kept), in an
    // outline left to Excel: the first accent of the theme of the file.
    assert_eq!(scatter.series[0].color, Some(Color(0x7030A0)));
    assert_eq!(
        scatter.series[0].marker,
        Some(Marker {
            line: Some(Color(0x4472C4)),
            ..Marker::new(MarkerShape::Circle)
        })
    );
    assert_eq!(scatter.series[0].line, Line::default());
    // A series that says nothing of its looks has what Excel gives it in
    // such a chart: a line, and the marker of the fourth series.
    assert_eq!(refs(&scatter.series[1]), ("Data!$C:$C".into(), None));
    assert_eq!(scatter.series[1].color, None);
    assert_eq!(scatter.series[1].line.shown, Some(true));
    assert_eq!(
        scatter.series[1].marker,
        Some(Marker {
            size: 7,
            ..Marker::new(MarkerShape::X)
        })
    );
    // Axes that say next to nothing: no gridlines, Excel's tick marks.
    let bare = Axis {
        major_ticks: TickMark::Outside,
        ..Axis::new()
    };
    assert_eq!(
        (&scatter.axes.category, &scatter.axes.value),
        (&bare, &bare)
    );
    let data = wb.chart_data(0, scatter);
    assert_eq!(data.x_values[..3], [Some(10.0), Some(20.0), Some(30.0)]);
    assert_eq!(data.series[0].values, [Some(7.0), Some(9.0), Some(4.0)]);
    assert_eq!(
        data.series[1].values.len(),
        4,
        "a whole column, as far as it is used"
    );
    assert_eq!((scatter.anchor, scatter.offset), ((30, 0), (0.0, 0.0)));
    assert_eq!(scatter.size, (140.0 + 3.0 * 70.0, 240.0));

    // What was read can be written, and read again: as it was, and from
    // the model alone.
    let again = reload(&wb);
    assert_same_charts(&wb, &again);
    let mut changed = load(&bytes);
    for chart in &mut changed.sheets[0].charts {
        chart.foreign = None;
    }
    let generated = to_bytes(&changed);
    for n in 1..=4 {
        let name = format!("xl/charts/chart{n}.xml");
        assert_well_formed(&name, &part(&generated, &name));
        assert_schema_order(&name, &part(&generated, &name));
    }
    assert_same_charts(&wb, &load(&generated));
}

#[test]
fn other_chart_types_become_the_closest_kind() {
    let plot = |element: &str, head: &str| {
        format!(
            r#"{DECL}<c:chartSpace {NS_C}><c:chart><c:plotArea><c:{element}>{head}<c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strRef><c:f>Data!$B$2:$B$4</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Data!$C$2:$C$4</c:f></c:numRef></c:val><c:bubbleSize><c:numRef><c:f>Data!$D$2:$D$4</c:f></c:numRef></c:bubbleSize></c:ser><c:bandFmts/><c:axId val="1"/><c:axId val="2"/><c:axId val="3"/></c:{element}></c:plotArea><c:legend><c:legendPos val="l"/></c:legend></c:chart></c:chartSpace>"#
        )
    };
    let bar = |direction: &str, grouping: &str| {
        format!("<c:barDir val=\"{direction}\"/><c:grouping val=\"{grouping}\"/>")
    };
    let grouped = |grouping: &str| format!("<c:grouping val=\"{grouping}\"/>");
    let scatter = |style: &str| format!("<c:scatterStyle val=\"{style}\"/>");
    let radar = |style: &str| format!("<c:radarStyle val=\"{style}\"/>");
    // `Line` stays the stroke.
    use ChartKind::{
        Area, Area3D, Bar, Bar3D, Bubble, Column, Column3D, Doughnut, Line3D, LineMarkers,
        PercentStackedArea, PercentStackedArea3D, PercentStackedBar, PercentStackedBar3D,
        PercentStackedColumn, PercentStackedColumn3D, PercentStackedLine,
        PercentStackedLineMarkers, Pie, Pie3D, Radar, RadarFilled, RadarMarkers, Scatter,
        ScatterLines, ScatterLinesMarkers, ScatterSmooth, ScatterSmoothMarkers, StackedArea,
        StackedArea3D, StackedBar, StackedBar3D, StackedColumn, StackedColumn3D, StackedLine,
        StackedLineMarkers,
    };
    for (element, head, kind) in [
        ("barChart", bar("col", "clustered"), Column),
        ("barChart", bar("col", "standard"), Column),
        ("barChart", bar("col", "stacked"), StackedColumn),
        (
            "barChart",
            bar("col", "percentStacked"),
            PercentStackedColumn,
        ),
        ("barChart", bar("bar", "clustered"), Bar),
        ("barChart", bar("bar", "stacked"), StackedBar),
        ("barChart", bar("bar", "percentStacked"), PercentStackedBar),
        ("barChart", String::new(), Column),
        ("bar3DChart", bar("col", "standard"), Column3D),
        ("bar3DChart", bar("col", "stacked"), StackedColumn3D),
        (
            "bar3DChart",
            bar("col", "percentStacked"),
            PercentStackedColumn3D,
        ),
        ("bar3DChart", bar("bar", "clustered"), Bar3D),
        ("bar3DChart", bar("bar", "stacked"), StackedBar3D),
        (
            "bar3DChart",
            bar("bar", "percentStacked"),
            PercentStackedBar3D,
        ),
        // Lines: the series says nothing of markers, the plot does.
        ("lineChart", grouped("standard"), ChartKind::Line),
        ("lineChart", grouped("stacked"), StackedLine),
        ("lineChart", grouped("percentStacked"), PercentStackedLine),
        (
            "lineChart",
            grouped("standard") + "<c:marker val=\"1\"/>",
            LineMarkers,
        ),
        (
            "lineChart",
            grouped("stacked") + "<c:marker val=\"1\"/>",
            StackedLineMarkers,
        ),
        (
            "lineChart",
            grouped("percentStacked") + "<c:marker val=\"1\"/>",
            PercentStackedLineMarkers,
        ),
        ("lineChart", "<c:marker val=\"0\"/>".into(), ChartKind::Line),
        ("line3DChart", grouped("stacked"), Line3D),
        // Types the engine does not have.
        ("stockChart", String::new(), ChartKind::Line),
        (
            "surfaceChart",
            "<c:wireframe val=\"0\"/>".into(),
            ChartKind::Line,
        ),
        ("surface3DChart", String::new(), ChartKind::Line),
        ("ofPieChart", "<c:ofPieType val=\"bar\"/>".into(), Pie),
        ("areaChart", String::new(), Area),
        ("areaChart", grouped("stacked"), StackedArea),
        ("areaChart", grouped("percentStacked"), PercentStackedArea),
        ("area3DChart", String::new(), Area3D),
        ("area3DChart", grouped("stacked"), StackedArea3D),
        (
            "area3DChart",
            grouped("percentStacked"),
            PercentStackedArea3D,
        ),
        ("pieChart", String::new(), Pie),
        ("pie3DChart", String::new(), Pie3D),
        ("doughnutChart", String::new(), Doughnut),
        // A scatter chart is what its style says where its series say
        // nothing; Excel's own is "lineMarker".
        ("scatterChart", String::new(), ScatterLinesMarkers),
        ("scatterChart", scatter("lineMarker"), ScatterLinesMarkers),
        ("scatterChart", scatter("line"), ScatterLines),
        ("scatterChart", scatter("marker"), Scatter),
        ("scatterChart", scatter("none"), Scatter),
        ("scatterChart", scatter("smooth"), ScatterSmooth),
        (
            "scatterChart",
            scatter("smoothMarker"),
            ScatterSmoothMarkers,
        ),
        ("bubbleChart", String::new(), Bubble),
        ("radarChart", String::new(), Radar),
        ("radarChart", radar("standard"), Radar),
        ("radarChart", radar("marker"), RadarMarkers),
        ("radarChart", radar("filled"), RadarFilled),
    ] {
        let xml = plot(element, &head);
        assert_well_formed(element, &xml);
        let wb = load(&foreign_package(&[("xl/charts/chart1.xml", &xml)]));
        let chart = &wb.sheets[0].charts[0];
        assert_eq!(chart.kind, kind, "{element} {head}");
        assert_eq!(chart.legend, LegendPos::Left);
        let series = &chart.series[0];
        assert_eq!(
            refs(series),
            ("Data!$C$2:$C$4".into(), Some("Data!$B$2:$B$4".into())),
            "{element}"
        );
        // Only bubbles have sizes.
        let sizes = series.bubble_sizes.as_ref().map(RefExpr::to_text);
        assert_eq!(
            sizes.as_deref(),
            (kind == Bubble).then_some("Data!$D$2:$D$4"),
            "{element}"
        );
        // The markers Excel picks by itself are not the engine's: a
        // diamond for the first series. Nothing else is said of the
        // looks of the series, which are those of the kind.
        let marker = if head == scatter("none") {
            // A scatter chart whose style draws nothing at all still is
            // one of points: the markers it lacks are said to be missing.
            Some(Marker::new(MarkerShape::None))
        } else {
            kind.has_markers().then(|| Marker {
                size: 7,
                ..Marker::new(MarkerShape::Diamond)
            })
        };
        assert_eq!(series.marker, marker, "{element} {head}");
        assert_eq!(
            (series.kind, series.smooth),
            (None, None),
            "{element} {head}"
        );
        assert_eq!(series.line, Line::default(), "{element} {head}");
        assert_eq!(series.color, None, "{element} {head}");
        // Written from the model and read again, it is the same chart.
        let mut changed = load(&foreign_package(&[("xl/charts/chart1.xml", &xml)]));
        changed.sheets[0].charts[0].foreign = None;
        assert_same_content(element, chart, &reload(&changed).sheets[0].charts[0]);
    }
}

#[test]
fn a_bad_chart_costs_only_itself() {
    let whole = load(&foreign_package(&[]));
    assert_eq!(whole.sheets[0].charts.len(), 4);
    let kinds =
        |wb: &Workbook| -> Vec<ChartKind> { wb.sheets[0].charts.iter().map(|c| c.kind).collect() };
    let all = kinds(&whole);

    // Chart parts that are not charts at all.
    for junk in [
        "",
        "not xml at all <<<< >>>",
        "<c:chartSpace><c:chart><c:plotArea><c:barChart><c:ser>",
        "<worksheet><sheetData/></worksheet>",
        "<c:chartSpace><c:chart/></c:chartSpace>",
        "<c:chartSpace><c:chart><c:plotArea/><c:legend/></c:chart></c:chartSpace>",
    ] {
        let wb = load(&foreign_package(&[("xl/charts/chart2.xml", junk)]));
        let left = kinds(&wb);
        // A plot cut short still shows what it had: nothing.
        let expected: Vec<ChartKind> = if junk.contains("<c:barChart>") {
            vec![all[0], ChartKind::Column, all[2], all[3]]
        } else {
            vec![all[0], all[2], all[3]]
        };
        assert_eq!(left, expected, "{junk:?}");
        assert_eq!(
            wb.sheets[0].charts.iter().map(|c| c.id).collect::<Vec<_>>(),
            (1..=left.len() as u32).collect::<Vec<_>>()
        );
        assert_eq!(wb.sheets[0].cells.len(), whole.sheets[0].cells.len());
    }

    // Drawing parts and relationships that lead nowhere.
    for (victim, text) in [
        (
            "xl/drawings/drawing1.xml",
            "<xdr:wsDr><xdr:twoCellAnchor><xdr:from><xdr:col>",
        ),
        ("xl/drawings/drawing1.xml", ""),
        ("xl/drawings/_rels/drawing1.xml.rels", "<Relationships>"),
        ("xl/worksheets/_rels/sheet1.xml.rels", "garbage"),
    ] {
        let wb = load(&foreign_package(&[(victim, text)]));
        assert!(wb.sheets[0].charts.is_empty(), "{victim}: {text:?}");
        assert_eq!(wb.sheets.len(), 2);
    }

    // Values no drawing should hold.
    let odd = format!(
        r#"<xdr:wsDr xmlns:xdr="x" xmlns:a="a"><xdr:twoCellAnchor>
<xdr:from><xdr:col>99999999</xdr:col><xdr:colOff>-5000000000000</xdr:colOff><xdr:row>4000000000</xdr:row><xdr:rowOff>abc</xdr:rowOff></xdr:from>
<xdr:to><xdr:col>2</xdr:col><xdr:colOff>1e300</xdr:colOff><xdr:row>-7</xdr:row><xdr:rowOff>NaN</xdr:rowOff></xdr:to>{}</xdr:twoCellAnchor>
<xdr:oneCellAnchor><xdr:ext cx="1e30" cy="-1"/>{}</xdr:oneCellAnchor>
<xdr:absoluteAnchor><xdr:pos x="9e99" y="inf"/>{}</xdr:absoluteAnchor>
<xdr:twoCellAnchor/><xdr:oneCellAnchor><xdr:graphicFrame><c:chart/></xdr:graphicFrame></xdr:oneCellAnchor></xdr:wsDr>"#,
        graphic_frame(2, "c", "rId1"),
        graphic_frame(3, "c", "rId1"),
        graphic_frame(4, "c", "rId1"),
    );
    let wb = load(&foreign_package(&[("xl/drawings/drawing1.xml", &odd)]));
    let charts = &wb.sheets[0].charts;
    assert_eq!(charts.len(), 3);
    for c in charts {
        assert!(
            c.anchor.0 < 1_048_576 && c.anchor.1 < 16_384,
            "{:?}",
            c.anchor
        );
        for px in [c.offset.0, c.offset.1, c.size.0, c.size.1] {
            assert!(px.is_finite() && (0.0..=20_000.0).contains(&px), "{px}");
        }
        assert!(c.size.0 >= 1.0 && c.size.1 >= 1.0);
    }
    assert_eq!(charts[0].anchor, (1_048_575, 16_383));
    assert_eq!(charts[1].size, (20_000.0, DEFAULT_CHART_SIZE.1));
    assert_eq!(charts[2].size, DEFAULT_CHART_SIZE);
    // Whatever was read can be written.
    assert_eq!(reload(&wb).sheets[0].charts.len(), 3);
}

#[test]
fn damaged_chart_parts_never_panic() {
    // Cut the parts that describe charts short at many places: loading
    // gives fewer charts, or none, but returns. The gallery has every
    // element the reader knows; its many parts are cut at fewer places.
    let mut attempts = 0;
    for (bytes, cuts) in [
        (to_bytes(&sample()), 40),
        (foreign_package(&[]), 40),
        (to_bytes(&gallery()), 6),
    ] {
        let names = part_names(&bytes);
        let parts: Vec<(String, String)> =
            names.iter().map(|n| (n.clone(), part(&bytes, n))).collect();
        let complete: Vec<usize> = load(&bytes).sheets.iter().map(|s| s.charts.len()).collect();
        for (victim, text) in parts.iter().filter(|(n, _)| {
            ["chart", "drawing", "sheet1.xml"]
                .iter()
                .any(|k| n.contains(k))
        }) {
            let step = (text.len() / cuts).max(1);
            for cut in (0..text.len())
                .step_by(step)
                .filter(|i| text.is_char_boundary(*i))
            {
                let damaged: Vec<(&str, &str)> = parts
                    .iter()
                    .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                    .collect();
                if let Ok(wb) = read_xlsx(Cursor::new(package(&damaged)), Locale::en()) {
                    for (sheet, whole) in wb.sheets.iter().zip(&complete) {
                        assert!(sheet.charts.len() <= *whole, "{victim} cut at {cut}");
                    }
                    // And what is left can be saved: a chart whose part was
                    // cut short is not written back as it was.
                    let saved = to_bytes(&wb);
                    for name in part_names(&saved) {
                        if name.starts_with("xl/charts/") {
                            assert_well_formed(&name, &part(&saved, &name));
                        }
                    }
                }
                attempts += 1;
            }
        }
    }
    assert!(attempts > 1000, "{attempts}");
}

// ---- every kind, every option ----------------------------------------------------

/// A series of the table of the gallery: one of its columns B to E.
fn monthly(column: char) -> Series {
    series(
        named(&format!("Data!${column}$1")),
        &format!("Data!${column}$2:${column}$7"),
        Some("Data!$A$2:$A$7"),
    )
}

/// A series of its X/Y table: Y values in column H or I.
fn measured(column: char) -> Series {
    series(
        named(&format!("Data!${column}$1")),
        &format!("Data!${column}$2:${column}$7"),
        Some("Data!$G$2:$G$7"),
    )
}

fn point(index: u32, color: Option<u32>, explosion: Option<u32>) -> PointFormat {
    PointFormat {
        index,
        color: color.map(Color),
        explosion,
    }
}

fn labels_at(position: LabelPosition) -> DataLabels {
    DataLabels {
        position,
        ..DataLabels::default()
    }
}

/// A chart of the given kind with series that suit it, and nothing but
/// what Excel would give it.
fn plain(kind: ChartKind) -> Chart {
    let series = match kind.family() {
        Family::Pie => vec![monthly('B')],
        Family::Doughnut => vec![monthly('B'), monthly('C')],
        Family::Scatter => vec![measured('H'), measured('I')],
        Family::Bubble => vec![Series {
            bubble_sizes: Some(range("Data!$J$2:$J$7")),
            ..measured('H')
        }],
        _ => vec![monthly('B'), monthly('C'), monthly('D')],
    };
    let mut c = chart(kind, series);
    c.legend = LegendPos::Bottom;
    c
}

/// Charts with every option of the model set at least once, each under a
/// title that says what to look for.
fn optioned() -> Vec<(&'static str, Chart)> {
    // `Line` stays the stroke.
    use ChartKind::{
        Area, Area3D, Bar, Bubble, Column, Column3D, Doughnut, LineMarkers, PercentStackedBar, Pie,
        Pie3D, RadarFilled, RadarMarkers, Scatter, ScatterLinesMarkers, StackedColumn,
    };
    let mut out: Vec<(&'static str, Chart)> = Vec::new();

    let mut c = chart(Column, vec![monthly('B'), monthly('C')]);
    c.axes.category.title = Some("Month".into());
    c.axes.value = Axis {
        title: Some("Units sold".into()),
        min: Some(0.0),
        max: Some(30.0),
        major_unit: Some(10.0),
        minor_unit: Some(2.5),
        number_format: Some("0.0".into()),
        minor_gridlines: true,
        major_ticks: TickMark::Outside,
        minor_ticks: TickMark::Inside,
        ..c.axes.value.clone()
    };
    c.gap_width = Some(50);
    c.overlap = Some(20);
    c.series[0].points = vec![
        point(1, Some(0xC00000), None),
        point(4, Some(0x00B050), None),
    ];
    c.series[1].color = Some(Color(0xFFC000));
    c.series[1].line = Line {
        shown: None,
        color: Some(Color(0x7F6000)),
        width: Some(1.5),
        dash: Dash::Dash,
    };
    c.legend = LegendPos::TopRight;
    c.legend_overlay = true;
    c.title_overlay = true;
    c.chart_area = ShapeFormat {
        fill: Fill::Solid(Color(0xFFF2CC)),
        border: Line {
            shown: None,
            color: Some(Color(0xC00000)),
            width: Some(2.0),
            dash: Dash::LongDash,
        },
        ..ShapeFormat::default()
    };
    c.plot_area = ShapeFormat {
        fill: Fill::Solid(Color(0xDDEBF7)),
        border: Line {
            shown: None,
            color: Some(Color(0x1F4E79)),
            width: Some(1.0),
            dash: Dash::SquareDot,
        },
        ..ShapeFormat::default()
    };
    out.push((
        "Axis titles, scale 0-30 by 10, minor gridlines,\nred and green bars, title over the plot",
        c,
    ));

    let years = |values: &str| {
        series(
            SeriesName::Text("Revenue".into()),
            values,
            Some("Data!$L$2:$L$7"),
        )
    };
    let mut c = chart(Column, vec![years("Data!$M$2:$M$7")]);
    c.axes.value.log_base = Some(10.0);
    c.axes.value.min = Some(100_000.0);
    c.axes.value.max = Some(100_000_000.0);
    c.axes.value.reversed = true;
    c.axes.category.crossing = AxisCrossing::Max;
    c.chart_area.border.shown = Some(false);
    c.chart_area.fill = Fill::None;
    out.push(("Log scale, upside down, no frame", c));

    let mut c = chart(Column, vec![years("Data!$M$2:$M$7")]);
    c.axes.value.display_unit = DisplayUnit::Millions;
    c.axes.value.number_format = Some("0.0".into());
    // Where the category axis crosses the value axis, and the other way round.
    c.axes.value.crossing = AxisCrossing::At(2_000_000.0);
    c.axes.category.crossing = AxisCrossing::At(3.0);
    c.axes.category.labels = TickLabels::Low;
    c.axes.category.major_ticks = TickMark::Cross;
    c.axes.category.number_format = Some("0000".into());
    out.push(("Millions; axes cross at 2 and at the third year", c));

    let mut c = chart(ChartKind::Line, vec![years("Data!$M$2:$M$7")]);
    c.axes.value.display_unit = DisplayUnit::Custom(500.0);
    c.axes.value.display_unit_label = false;
    c.axes.value.crossing = AxisCrossing::Max;
    c.axes.value.labels = TickLabels::High;
    c.axes.category.reversed = true;
    c.axes.category.major_gridlines = true;
    c.axes.category.minor_ticks = TickMark::Outside;
    c.blanks = BlanksAs::Zero;
    out.push(("Values / 500; years from right to left", c));

    let mut c = chart(Bar, vec![monthly('B')]);
    c.vary_colors = Some(true);
    c.axes.category.visible = false;
    c.axes.value.major_gridlines = false;
    c.axes.value.labels = TickLabels::None;
    c.data_labels = true;
    c.label_options = DataLabels {
        category: true,
        series_name: true,
        legend_key: true,
        separator: Some(" | ".into()),
        position: LabelPosition::InsideBase,
        number_format: Some("0.00".into()),
        ..DataLabels::default()
    };
    out.push(("Bars of six colours, no axes, labels inside base", c));

    let mut c = chart(LineMarkers, vec![monthly('B'), monthly('C'), monthly('D')]);
    c.series[0] = Series {
        color: Some(Color(0x00B0F0)),
        line: Line {
            shown: None,
            color: None,
            width: Some(3.0),
            dash: Dash::DashDot,
        },
        marker: Some(Marker {
            shape: MarkerShape::Diamond,
            size: 9,
            fill: Some(Color(0xFFFF00)),
            line: None,
        }),
        smooth: Some(true),
        labels: Some(labels_at(LabelPosition::Above)),
        ..monthly('B')
    };
    c.series[1].line.dash = Dash::RoundDot;
    c.series[1].marker = Some(Marker {
        line: Some(Color(0x000000)),
        ..Marker::new(MarkerShape::Square)
    });
    c.series[1].labels = Some(labels_at(LabelPosition::Below));
    c.series[2].line.shown = Some(false);
    c.series[2].marker = Some(Marker {
        size: 8,
        ..Marker::new(MarkerShape::Triangle)
    });
    c.series[2].labels = Some(labels_at(LabelPosition::Left));
    c.blanks = BlanksAs::Span;
    c.legend = LegendPos::Right;
    out.push((
        "Smooth blue dash-dot with yellow diamonds;\ndotted with squares; triangles alone",
        c,
    ));

    let shapes = [
        MarkerShape::X,
        MarkerShape::Star,
        MarkerShape::Dash,
        MarkerShape::Dot,
        MarkerShape::Plus,
        MarkerShape::Circle,
    ];
    let dashes = [
        Dash::SquareDot,
        Dash::Dash,
        Dash::LongDash,
        Dash::LongDashDot,
        Dash::LongDashDotDot,
        Dash::Solid,
    ];
    let mut c = chart(LineMarkers, Vec::new());
    for (i, (shape, dash)) in shapes.into_iter().zip(dashes).enumerate() {
        let mut s = monthly(['B', 'C', 'D'][i % 3]);
        s.name = SeriesName::Text(format!("{shape:?}, {dash:?}"));
        s.marker = Some(Marker {
            size: 6 + 2 * i as u8,
            ..Marker::new(shape)
        });
        s.line.dash = dash;
        s.line.width = Some(0.5 + i as f32 * 0.5);
        c.series.push(s);
    }
    c.series[4].labels = Some(labels_at(LabelPosition::Right));
    c.series[5].labels = Some(labels_at(LabelPosition::Center));
    c.legend = LegendPos::Left;
    out.push(("Six markers, six dashes, growing", c));

    let mut c = chart(
        StackedColumn,
        vec![monthly('B'), monthly('C'), monthly('D')],
    );
    c.data_labels = true;
    c.label_options = labels_at(LabelPosition::Center);
    c.series[1].labels = Some(labels_at(LabelPosition::InsideEnd));
    c.series[2].labels = Some(DataLabels::none());
    c.series[2].error_bars = vec![ErrorBars {
        horizontal: false,
        direction: ErrorDirection::Both,
        amount: ErrorAmount::StdErr,
        end_cap: true,
    }];
    c.data_table = Some(DataTable::default());
    out.push(("Stacked: labels, none on top; error bars; table", c));

    let mut c = chart(
        Column,
        vec![monthly('B'), monthly('C'), monthly('D'), monthly('B')],
    );
    c.series[3].name = SeriesName::Text("Again".into());
    for (s, position) in c.series.iter_mut().zip([
        LabelPosition::OutsideEnd,
        LabelPosition::InsideEnd,
        LabelPosition::InsideBase,
        LabelPosition::Center,
    ]) {
        s.labels = Some(labels_at(position));
    }
    c.data_table = Some(DataTable {
        horizontal_lines: false,
        vertical_lines: true,
        outline: false,
        legend_keys: false,
    });
    out.push(("Labels outside, inside end, inside base, centred", c));

    let mut c = chart(PercentStackedBar, vec![monthly('B'), monthly('C')]);
    c.series[0].labels = Some(labels_at(LabelPosition::InsideBase));
    c.series[1].labels = Some(DataLabels {
        series_name: true,
        value: false,
        position: LabelPosition::InsideEnd,
        ..DataLabels::default()
    });
    c.legend = LegendPos::Top;
    out.push(("100 % bars: values at the base, names at the end", c));

    let mut c = chart(Pie, vec![monthly('B')]);
    c.series[0].explosion = 10;
    c.series[0].points = vec![
        point(0, Some(0xC00000), Some(30)),
        point(2, None, Some(0)),
        point(3, Some(0x00B050), None),
    ];
    c.series[0].line = Line {
        shown: None,
        color: Some(Color(0x000000)),
        width: Some(0.5),
        dash: Dash::Solid,
    };
    c.first_slice_angle = 45;
    c.data_labels = true;
    c.label_options = DataLabels {
        value: false,
        percent: true,
        category: true,
        separator: Some("\n".into()),
        position: LabelPosition::OutsideEnd,
        ..DataLabels::default()
    };
    c.legend = LegendPos::Right;
    c.legend_hidden = vec![0, 5];
    out.push((
        "Exploded, turned 45°, red slice far out; legend without Jan and Jun",
        c,
    ));

    let mut c = chart(Pie, vec![monthly('E')]);
    c.vary_colors = Some(false);
    c.series[0].line.shown = Some(false);
    c.data_labels = true;
    c.label_options = DataLabels {
        legend_key: true,
        leader_lines: false,
        position: LabelPosition::BestFit,
        number_format: Some("0%".into()),
        ..DataLabels::default()
    };
    out.push(("One colour, no lines between slices, keys in labels", c));

    let mut c = chart(Pie, vec![monthly('C')]);
    c.data_labels = true;
    c.label_options = labels_at(LabelPosition::InsideEnd);
    c.title = ChartTitle::None;
    out.push(("", c));
    let mut c = chart(Pie3D, vec![monthly('C')]);
    c.data_labels = true;
    c.label_options = labels_at(LabelPosition::Center);
    c.first_slice_angle = 120;
    c.view3d = Some(View3D {
        rot_x: 50,
        ..View3D::for_kind(Pie3D)
    });
    c.series[0].explosion = 8;
    out.push(("Pie in 3-D seen from above, turned 120°", c));

    let mut c = chart(Doughnut, vec![monthly('B'), monthly('C')]);
    c.hole_size = 40;
    c.first_slice_angle = 90;
    c.series[1].explosion = 5;
    c.data_labels = true;
    c.label_options = DataLabels {
        value: false,
        percent: true,
        ..DataLabels::default()
    };
    c.legend = LegendPos::Bottom;
    out.push(("Small hole, turned 90°, percentages", c));

    let mut c = chart(
        Column,
        vec![
            monthly('B'),
            monthly('C'),
            Series {
                kind: Some(LineMarkers),
                secondary: true,
                ..monthly('E')
            },
            Series {
                kind: Some(Area),
                color: Some(Color(0xD9D9D9)),
                ..monthly('D')
            },
        ],
    );
    c.axes.secondary_value.title = Some("Share".into());
    c.axes.secondary_value.number_format = Some("0%".into());
    c.axes.value.title = Some("Units".into());
    c.legend = LegendPos::Bottom;
    out.push((
        "Columns, a grey area, and a line on the right-hand scale",
        c,
    ));

    let mut c = chart(
        ChartKind::Line,
        vec![
            monthly('E'),
            Series {
                kind: Some(StackedColumn),
                secondary: true,
                ..monthly('B')
            },
            Series {
                kind: Some(StackedColumn),
                secondary: true,
                ..monthly('C')
            },
        ],
    );
    // Shown, the second category axis is at the far end of its scale.
    c.axes.secondary_category.visible = true;
    c.axes.secondary_category.crossing = AxisCrossing::Min;
    c.axes.secondary_category.labels = TickLabels::High;
    c.axes.secondary_value.major_gridlines = true;
    c.axes.value.major_gridlines = false;
    c.legend = LegendPos::Bottom;
    out.push((
        "Line; stacked columns on a second scale at the left, months on top too",
        c,
    ));

    let mut c = chart(
        Bar,
        vec![
            monthly('B'),
            Series {
                kind: Some(PercentStackedBar),
                secondary: true,
                ..monthly('C')
            },
            Series {
                kind: Some(PercentStackedBar),
                secondary: true,
                ..monthly('D')
            },
        ],
    );
    c.axes.secondary_value.number_format = Some("0%".into());
    c.overlap = Some(-10);
    out.push(("Bars, and 100 % bars on a scale along the top", c));

    let mut c = chart(
        Scatter,
        vec![
            measured('H'),
            // Of the same plot as the chart: its looks say what it is.
            Series {
                secondary: true,
                line: Line {
                    shown: Some(true),
                    ..Line::default()
                },
                marker: Some(Marker::new(MarkerShape::None)),
                smooth: Some(true),
                ..measured('I')
            },
        ],
    );
    c.axes.category.min = Some(0.0);
    c.axes.category.max = Some(8.0);
    c.axes.category.major_unit = Some(2.0);
    c.axes.category.minor_gridlines = true;
    c.axes.category.title = Some("x".into());
    c.axes.value.title = Some("y".into());
    c.axes.secondary_value.log_base = Some(2.0);
    c.legend = LegendPos::Top;
    out.push(("Points; a curve on a base-2 scale at the right", c));

    let trend = |kind: TrendKind| Trendline::new(kind);
    let mut c = chart(Scatter, vec![measured('H'), measured('I')]);
    c.series[0].trendlines = vec![
        Trendline {
            name: Some("Straight & narrow".into()),
            forward: 2.0,
            backward: 0.5,
            intercept: Some(0.5),
            show_equation: true,
            show_r2: true,
            ..trend(TrendKind::Linear)
        },
        Trendline {
            line: Line {
                shown: None,
                color: Some(Color(0xC00000)),
                width: Some(2.5),
                dash: Dash::LongDashDot,
            },
            ..trend(TrendKind::Logarithmic)
        },
        trend(TrendKind::MovingAverage(3)),
    ];
    c.series[1].trendlines = vec![
        Trendline {
            intercept: Some(2.0),
            forward: 1.0,
            ..trend(TrendKind::Exponential)
        },
        Trendline {
            intercept: Some(1.0),
            show_equation: true,
            line: Line {
                width: Some(1.5),
                ..Line::default()
            },
            ..trend(TrendKind::Polynomial(3))
        },
        Trendline {
            show_r2: true,
            ..trend(TrendKind::Power)
        },
    ];
    c.legend = LegendPos::Right;
    c.legend_hidden = vec![4];
    out.push(("Six trendlines: every type", c));

    for (kind, title) in [
        (Column, "Columns with a trend and a moving average"),
        (Bar, "Bars with a trend"),
        (LineMarkers, "A line with a trend"),
        (Area, "An area with a trend"),
    ] {
        let mut c = chart(kind, vec![monthly('B')]);
        c.series[0].trendlines = vec![
            Trendline {
                forward: 1.0,
                ..trend(TrendKind::Linear)
            },
            trend(TrendKind::MovingAverage(2)),
        ];
        c.series[0].error_bars = vec![ErrorBars {
            horizontal: false,
            direction: ErrorDirection::Plus,
            amount: ErrorAmount::Fixed(2.0),
            end_cap: false,
        }];
        c.legend = LegendPos::Bottom;
        out.push((title, c));
    }

    let mut c = chart(ScatterLinesMarkers, vec![measured('H'), measured('I')]);
    c.series[0].error_bars = vec![
        ErrorBars {
            horizontal: true,
            direction: ErrorDirection::Both,
            amount: ErrorAmount::Fixed(0.25),
            end_cap: true,
        },
        ErrorBars {
            horizontal: false,
            direction: ErrorDirection::Plus,
            amount: ErrorAmount::Percent(20.0),
            end_cap: false,
        },
    ];
    c.series[1].error_bars = vec![ErrorBars {
        horizontal: false,
        direction: ErrorDirection::Minus,
        amount: ErrorAmount::StdDev(1.5),
        end_cap: true,
    }];
    out.push(("Error bars: both ways; up by 20 %; down by 1.5 σ", c));

    let mut c = chart(
        Bubble,
        vec![
            Series {
                bubble_sizes: Some(range("Data!$J$2:$J$7")),
                ..measured('H')
            },
            measured('I'),
        ],
    );
    c.bubble_scale = 50;
    c.series[0].labels = Some(DataLabels {
        value: false,
        bubble_size: true,
        position: LabelPosition::Center,
        ..DataLabels::default()
    });
    c.series[0].error_bars = vec![ErrorBars {
        horizontal: true,
        direction: ErrorDirection::Both,
        amount: ErrorAmount::Fixed(0.5),
        end_cap: true,
    }];
    c.series[1].trendlines = vec![trend(TrendKind::Linear)];
    c.series[1].color = Some(Color(0xED7D31));
    c.series[1].points = vec![point(5, Some(0x000000), None)];
    out.push((
        "Bubbles at half size with their sizes; equal ones, one black",
        c,
    ));

    let mut c = chart(Column3D, vec![monthly('B'), monthly('C')]);
    c.view3d = Some(View3D {
        rot_x: 30,
        rot_y: 40,
        perspective: 60,
        right_angle_axes: false,
        depth_percent: 200,
        height_percent: Some(80),
    });
    c.gap_width = Some(80);
    c.data_labels = true;
    c.axes.value.title = Some("Units".into());
    out.push(("3-D columns in perspective, deep, labelled", c));

    let mut c = chart(Area3D, vec![monthly('B'), monthly('C')]);
    c.view3d = Some(View3D {
        right_angle_axes: true,
        ..View3D::for_kind(Area3D)
    });
    out.push(("3-D areas with right-angle axes", c));

    let mut c = chart(RadarMarkers, vec![monthly('B'), monthly('C')]);
    c.series[0].marker = Some(Marker {
        size: 10,
        ..Marker::new(MarkerShape::Star)
    });
    c.series[1].marker = Some(Marker::new(MarkerShape::None));
    c.series[1].line.dash = Dash::Dash;
    c.data_labels = true;
    c.axes.value.major_ticks = TickMark::Cross;
    c.axes.category.major_gridlines = true;
    out.push(("Radar: stars; a dashed line without markers; labels", c));

    let mut c = chart(RadarFilled, vec![monthly('B'), monthly('C')]);
    c.series[0].color = Some(Color(0x9DC3E6));
    c.series[0].line.color = Some(Color(0x1F4E79));
    c.series[1].line.shown = Some(false);
    out.push(("Filled radar: light blue with a dark outline", c));

    let mut c = chart(Scatter, vec![measured('H'), measured('I')]);
    c.series[0].color = Some(Color(0xFFFFFF));
    c.series[0].marker = Some(Marker {
        shape: MarkerShape::Circle,
        size: 12,
        fill: None,
        line: Some(Color(0xC00000)),
    });
    c.series[0].points = vec![point(2, Some(0x00B050), None)];
    c.series[1] = Series {
        line: Line {
            shown: Some(true),
            color: None,
            width: Some(1.0),
            dash: Dash::Solid,
        },
        marker: Some(Marker::new(MarkerShape::None)),
        smooth: Some(true),
        color: Some(Color(0x7030A0)),
        ..measured('I')
    };
    c.axes.category.log_base = Some(10.0);
    c.axes.category.number_format = Some("0.0".into());
    c.axes.value.crossing = AxisCrossing::At(3.0);
    c.title = ChartTitle::None;
    c.legend = LegendPos::Left;
    c.legend_overlay = true;
    out.push(("", c));

    let mut c = chart(Area, vec![monthly('B'), monthly('C')]);
    c.data_labels = true;
    c.label_options = DataLabels {
        series_name: true,
        value: false,
        ..DataLabels::default()
    };
    c.series[0].color = Some(Color(0xA9D18E));
    c.series[0].line = Line {
        shown: None,
        color: Some(Color(0x385723)),
        width: Some(2.0),
        dash: Dash::Solid,
    };
    c.vary_colors = Some(true);
    out.push(("Areas named in labels, the first outlined", c));

    // Lines that say only part of what they look like: the rest is what
    // such a line is by itself.
    let mut c = chart(Area, vec![monthly('B'), monthly('C')]);
    c.series[0].line = Line {
        color: Some(Color(0x404040)),
        width: Some(1.5),
        ..Line::default()
    };
    c.series[1].line.width = Some(3.0);
    c.plot_area.border.color = Some(Color(0x7F7F7F));
    c.chart_area.border.color = Some(Color(0x7F7F7F));
    out.push(("Areas outlined, one in its own colour; grey frames", c));

    let mut c = chart(Column, vec![monthly('B'), monthly('C'), monthly('D')]);
    c.series[0].line.color = Some(Color(0x000000));
    c.series[1].line.shown = Some(true);
    c.series[2].line = Line {
        width: Some(2.0),
        dash: Dash::Dash,
        ..Line::default()
    };
    c.plot_area.border.width = Some(2.0);
    c.chart_area.border.width = Some(3.0);
    c.chart_area.border.dash = Dash::RoundDot;
    out.push((
        "Columns outlined: in black, thinly, in dashes; wide frames",
        c,
    ));

    let mut c = chart(Pie, vec![monthly('B')]);
    c.series[0].line.width = Some(4.0);
    out.push(("Slices kept well apart", c));

    let mut c = chart(Bubble, vec![measured('H')]);
    c.series[0].bubble_sizes = Some(range("Data!$J$2:$J$7"));
    c.series[0].line.color = Some(Color(0xC00000));
    c.vary_colors = Some(true);
    out.push(("Bubbles of six colours, outlined in red", c));

    let mut c = chart(
        Bubble,
        vec![
            Series {
                bubble_sizes: Some(range("Data!$J$2:$J$7")),
                ..measured('H')
            },
            Series {
                bubble_sizes: Some(range("Data!$J$2:$J$7")),
                secondary: true,
                ..measured('I')
            },
        ],
    );
    c.legend = LegendPos::Bottom;
    out.push(("Bubbles on two scales", c));

    let mut c = plain(ChartKind::StackedColumn3D);
    c.data_labels = true;
    c.data_table = Some(DataTable::default());
    c.legend = LegendPos::None;
    out.push(("Stacked 3-D columns, labelled, over their table", c));

    let mut c = plain(RadarFilled);
    c.data_labels = true;
    c.label_options = DataLabels {
        category: true,
        ..DataLabels::default()
    };
    out.push(("Filled radar with categories and values", c));

    // Charts without series are still charts of their kind.
    out.push((
        "No series: an empty column chart",
        chart(Column, Vec::new()),
    ));
    out.push(("No series: an empty pie", chart(Pie, Vec::new())));
    out
}

/// The numbers the charts of the gallery plot, on a sheet called "Data",
/// and two empty sheets for the charts.
fn data_workbook() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Data"));
    assert!(wb.rename_sheet(1, "Kinds"));
    assert!(wb.rename_sheet(2, "Options"));
    let table = [
        [
            "Month", "North", "South", "West", "Share", "", "x", "y1", "y2", "size", "", "Year",
            "Revenue",
        ],
        [
            "Jan", "12", "8", "5", "0.4", "", "1", "2.5", "3", "10", "", "2019", "1250000",
        ],
        [
            "Feb", "15", "11", "7", "0.25", "", "2", "4.1", "5.5", "25", "", "2020", "1890000",
        ],
        [
            "Mar", "9", "14", "10", "0.2", "", "3", "6.2", "9", "5", "", "2021", "2400000",
        ],
        [
            "Apr", "18", "10", "12", "0.1", "", "4", "7.9", "15", "40", "", "2022", "3100000",
        ],
        [
            "May", "21", "13", "9", "0.03", "", "5", "10.3", "24", "15", "", "2023", "4750000",
        ],
        [
            "Jun", "17", "16", "11", "0.02", "", "6", "12.2", "38", "30", "", "2024", "5200000",
        ],
    ];
    for (r, row) in table.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if !text.is_empty() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
    }
    wb
}

/// A workbook with a chart of every kind on one sheet and the charts of
/// [`optioned`] on another.
fn gallery() -> Workbook {
    let mut wb = data_workbook();
    // A grid of charts, five to a row.
    let place = |c: &mut Chart, i: usize| {
        c.anchor = (1 + 13 * (i as u32 / 5), 6 * (i as u32 % 5));
        c.size = (370.0, 245.0);
    };
    for (i, kind) in ChartKind::ALL.into_iter().enumerate() {
        let mut c = plain(kind);
        c.title = ChartTitle::Text(format!("{kind:?}"));
        place(&mut c, i);
        wb.add_chart(1, c).unwrap();
    }
    for (i, (title, mut c)) in optioned().into_iter().enumerate() {
        if !title.is_empty() {
            c.title = ChartTitle::Text(title.to_string());
        }
        place(&mut c, i);
        wb.add_chart(2, c).unwrap();
    }
    wb.commit("charts");
    wb
}

/// The chart parts written for these charts of the data of the gallery,
/// each checked for its form, and the charts as they are read back.
fn written(charts: Vec<Chart>) -> (Vec<String>, Vec<Chart>) {
    let mut wb = data_workbook();
    let count = charts.len();
    for c in charts {
        wb.add_chart(1, c).unwrap();
    }
    let bytes = to_bytes(&wb);
    let parts = (1..=count)
        .map(|n| {
            let name = format!("xl/charts/chart{n}.xml");
            let xml = part(&bytes, &name);
            assert_well_formed(&name, &xml);
            assert_schema_order(&name, &xml);
            xml
        })
        .collect();
    (parts, load(&bytes).sheets.swap_remove(1).charts)
}

/// Written and read again, a chart shows the same.
fn assert_round_trip(charts: Vec<Chart>) -> Vec<String> {
    let (parts, read) = written(charts.clone());
    for (i, (before, after)) in charts.iter().zip(&read).enumerate() {
        assert_same_content(&format!("chart {}", i + 1), before, after);
    }
    parts
}

#[test]
fn every_kind_and_every_option_round_trip() {
    let original = gallery();
    assert_eq!(original.sheets[1].charts.len(), ChartKind::ALL.len());
    let bytes = to_bytes(&original);
    keep("io-sample.xlsx", &bytes);
    let mut checked = 0;
    for name in part_names(&bytes) {
        if name.starts_with("xl/charts/") && name.ends_with(".xml") {
            let xml = part(&bytes, &name);
            assert_well_formed(&name, &xml);
            assert_schema_order(&name, &xml);
            checked += 1;
        }
    }
    assert_eq!(checked, ChartKind::ALL.len() + optioned().len());

    let mut loaded = load(&bytes);
    assert_same_charts(&original, &loaded);
    // Every kind is a kind of its own in the file too.
    let kinds: HashSet<ChartKind> = loaded.sheets[1].charts.iter().map(|c| c.kind).collect();
    assert_eq!(kinds.len(), ChartKind::ALL.len());
    for (si, sheet) in original.sheets.iter().enumerate() {
        for (a, b) in sheet.charts.iter().zip(&loaded.sheets[si].charts) {
            assert_eq!(original.chart_data(si, a), loaded.chart_data(si, b));
        }
    }
    // What was read says the same when written from the model alone.
    for chart in loaded.sheets.iter_mut().flat_map(|s| &mut s.charts) {
        chart.foreign = None;
    }
    let generated = to_bytes(&loaded);
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&generated, &name), "{name}");
    }
}

// ---- what the writer spells out --------------------------------------------------

/// The `nth` element of that name in a part (which must not nest in
/// itself), tags included.
fn element<'a>(xml: &'a str, name: &str, nth: usize) -> &'a str {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    let start = xml
        .match_indices(&open)
        .nth(nth)
        .unwrap_or_else(|| panic!("no {open} number {nth} in {xml}"))
        .0;
    let end = start + xml[start..].find(&close).expect("the element ends") + close.len();
    &xml[start..end]
}

/// The elements of a part that are axes, as (element, `c:axId`).
fn axes_in(xml: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for name in ["c:catAx", "c:valAx", "c:serAx"] {
        let open = format!("<{name}><c:axId val=\"");
        for (at, _) in xml.match_indices(&open) {
            let id = &xml[at + open.len()..];
            out.push((at, name, &id[..id.find('"').unwrap()]));
        }
    }
    out.sort();
    out.into_iter().map(|(_, name, id)| (name, id)).collect()
}

const HAIRLINE: &str = "<c:spPr><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"D9D9D9\"/></a:solidFill></a:ln></c:spPr>";

#[test]
fn automatic_colours_are_spelled_out() {
    // Excel would pick colours of its own from the theme of the package:
    // every series says the colour the engine gives it, wherever it has ink.
    let mut dotted = plain(ChartKind::LineMarkers);
    dotted.series[1].line.color = Some(Color(0x00FF00));
    dotted.series[2].marker = Some(Marker {
        fill: Some(Color(0xFFFFFF)),
        ..Marker::new(MarkerShape::Square)
    });
    let mut bubbles = plain(ChartKind::Bubble);
    bubbles.series[0].points = vec![point(1, Some(0xC00000), None)];
    bubbles.series[0].trendlines = vec![Trendline::new(TrendKind::Linear)];
    let parts = assert_round_trip(vec![
        dotted,
        plain(ChartKind::Area),
        bubbles,
        plain(ChartKind::RadarFilled),
        plain(ChartKind::Line3D),
    ]);
    let line = |rgb: &str| {
        format!(
            "<c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill><a:round/></a:ln></c:spPr>"
        )
    };
    let marker = |symbol: &str, inside: &str, around: &str| {
        format!(
            "<c:marker><c:symbol val=\"{symbol}\"/><c:size val=\"5\"/><c:spPr><a:solidFill><a:srgbClr val=\"{inside}\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"{around}\"/></a:solidFill></a:ln></c:spPr></c:marker>"
        )
    };
    let series = |n: usize| element(&parts[0], "c:ser", n);
    assert!(series(0).contains(&(line("156082") + &marker("circle", "156082", "156082"))));
    // A line of another colour leaves the markers theirs, and markers
    // filled in another colour keep the outline of the series.
    assert!(series(1).contains(&(line("00FF00") + &marker("circle", "E97132", "E97132"))));
    assert!(series(2).contains(&(line("196B24") + &marker("square", "FFFFFF", "196B24"))));
    assert!(element(&parts[1], "c:ser", 2).contains(
        "</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"196B24\"/></a:solidFill></c:spPr><c:cat>"
    ));
    // Bubbles let a quarter of what is behind them through, like Excel's
    // own, and so does the one of another colour; the trendline is of the
    // colour of its series.
    let bubble = element(&parts[2], "c:ser", 0);
    assert!(bubble.contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"156082\"><a:alpha val=\"75000\"/></a:srgbClr></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/><c:dPt><c:idx val=\"1\"/><c:invertIfNegative val=\"0\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"C00000\"><a:alpha val=\"75000\"/></a:srgbClr></a:solidFill></c:spPr></c:dPt><c:trendline><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill><a:prstDash val=\"sysDot\"/><a:round/></a:ln></c:spPr>"), "{bubble}");
    // A filled radar has the colour inside and around; a ribbon in 3-D is
    // a shape.
    assert!(element(&parts[3], "c:ser", 1).contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill></a:ln></c:spPr><c:cat>"));
    assert!(element(&parts[4], "c:ser", 0).contains(
        "</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill></c:spPr><c:cat>"
    ));

    // A colour that is the automatic one anyway is no colour of its own.
    let mut explicit = plain(ChartKind::Column);
    explicit.series[1].color = Some(Color(0xE97132));
    let (_, read) = written(vec![explicit]);
    assert_eq!(read[0].series[1].color, None);
}

#[test]
fn points_of_varied_colours_say_which() {
    // Slices, and the points of a lone series that is told to vary: one
    // `c:dPt` each, in the colours of the palette unless they have their own.
    let mut pie = plain(ChartKind::Pie);
    pie.series[0].points = vec![point(1, Some(0xC00000), Some(20)), point(4, None, Some(5))];
    let mut bars = chart(ChartKind::Column, vec![monthly('B')]);
    bars.vary_colors = Some(true);
    let mut line = chart(ChartKind::LineMarkers, vec![monthly('B')]);
    line.vary_colors = Some(true);
    // Rings of a doughnut all vary; a series colour is kept for the day
    // they no longer do.
    let mut rings = plain(ChartKind::Doughnut);
    rings.series[1].color = Some(Color(0x123456));
    // Not varied: a pie told not to, columns of several series.
    let mut plain_pie = plain(ChartKind::Pie);
    plain_pie.vary_colors = Some(false);
    let mut columns = plain(ChartKind::Column);
    columns.vary_colors = Some(true);
    columns.series[0].points = vec![point(3, Some(0x00B050), None)];
    let parts = assert_round_trip(vec![pie, bars, line, rings, plain_pie, columns]);

    let white = "<a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln>";
    let slice = |index: u32, rgb: &str, explosion: &str| {
        format!(
            "<c:dPt><c:idx val=\"{index}\"/><c:bubble3D val=\"0\"/>{explosion}<c:spPr><a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill>{white}</c:spPr></c:dPt>"
        )
    };
    let expected = [
        slice(0, "156082", ""),
        slice(1, "C00000", "<c:explosion val=\"20\"/>"),
        slice(2, "196B24", ""),
        slice(3, "0F9ED5", ""),
        slice(4, "A02B93", "<c:explosion val=\"5\"/>"),
        slice(5, "4EA72E", ""),
    ]
    .concat();
    // No fill for the series as a whole: it would hide what Excel gives
    // points that come later.
    assert!(
        parts[0].contains(&format!("</c:tx><c:spPr>{white}</c:spPr>{expected}<c:cat>")),
        "{}",
        parts[0]
    );
    assert!(
        parts[1].contains(
            "<c:varyColors val=\"1\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:tx>"
        )
    );
    assert!(parts[1].contains("</c:tx><c:invertIfNegative val=\"0\"/><c:dPt><c:idx val=\"0\"/><c:invertIfNegative val=\"0\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill></c:spPr></c:dPt><c:dPt><c:idx val=\"1\"/>"));
    assert_eq!(parts[1].matches("<c:dPt>").count(), 6);
    // A point of a line is its marker and the stretch of line leading to it.
    assert!(parts[2].contains("<c:dPt><c:idx val=\"1\"/><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/><c:spPr><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill></a:ln></c:spPr></c:marker><c:bubble3D val=\"0\"/><c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"E97132\"/></a:solidFill><a:round/></a:ln></c:spPr></c:dPt>"));
    assert_eq!(parts[3].matches("<c:dPt>").count(), 12);
    assert!(element(&parts[3], "c:ser", 1).contains(&format!("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"123456\"/></a:solidFill>{white}</c:spPr><c:dPt><c:idx val=\"0\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill>")));
    assert!(parts[4].contains(&format!("<c:pieChart><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:tx><c:strRef><c:f>Data!$B$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>North</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill>{white}</c:spPr><c:cat>")));
    assert_eq!(parts[5].matches("<c:dPt>").count(), 1, "the green one");
    assert!(parts[5].contains("<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"1\"/>"));
}

#[test]
fn combination_charts_and_secondary_axes() {
    let line = |column: char, secondary: bool| Series {
        kind: Some(ChartKind::Line),
        secondary,
        ..monthly(column)
    };
    // Columns, a line on the secondary axes, more columns, a line on the
    // primary ones: one element per type and pair of axes, in the order
    // their first series come in.
    let mixed = chart(
        ChartKind::Column,
        vec![
            monthly('B'),
            line('E', true),
            monthly('C'),
            line('D', false),
        ],
    );
    // Horizontal bars; an X/Y chart.
    let sideways = chart(
        ChartKind::Bar,
        vec![
            monthly('B'),
            Series {
                kind: Some(ChartKind::StackedBar),
                secondary: true,
                ..monthly('C')
            },
        ],
    );
    let mut points = plain(ChartKind::Scatter);
    points.series[1].secondary = true;
    points.axes.secondary_category.visible = true;
    points.axes.secondary_category.crossing = AxisCrossing::At(2.5);
    let parts = assert_round_trip(vec![mixed, sideways, points]);

    let xml = &parts[0];
    assert_eq!(xml.matches("<c:barChart>").count(), 1);
    assert_eq!(xml.matches("<c:lineChart>").count(), 2);
    assert!(xml.find("</c:barChart>") < xml.find("<c:lineChart>"));
    // The number of a series is its position in the chart.
    let positions = |group: &str| -> Vec<usize> {
        (0..4)
            .filter(|i| group.contains(&format!("<c:idx val=\"{i}\"/><c:order val=\"{i}\"/>")))
            .collect()
    };
    let columns = element(xml, "c:barChart", 0);
    assert_eq!(positions(columns), [0, 2]);
    assert!(columns.ends_with("<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/><c:axId val=\"50000011\"/><c:axId val=\"50000012\"/></c:barChart>"));
    let second = element(xml, "c:lineChart", 0);
    assert_eq!(positions(second), [1]);
    assert!(second.ends_with(
        "<c:marker val=\"1\"/><c:axId val=\"50000014\"/><c:axId val=\"50000013\"/></c:lineChart>"
    ));
    let first = element(xml, "c:lineChart", 1);
    assert_eq!(positions(first), [3]);
    assert!(first.ends_with(
        "<c:marker val=\"1\"/><c:axId val=\"50000011\"/><c:axId val=\"50000012\"/></c:lineChart>"
    ));
    // The second pair as Excel has it: the scale at the far end, the
    // category axis there but not shown.
    assert_eq!(
        axes_in(xml),
        [
            ("c:catAx", "50000011"),
            ("c:valAx", "50000012"),
            ("c:valAx", "50000013"),
            ("c:catAx", "50000014")
        ]
    );
    let scale = element(xml, "c:valAx", 1);
    assert!(scale.starts_with("<c:valAx><c:axId val=\"50000013\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"r\"/><c:numFmt"));
    assert!(scale.ends_with("<c:crossAx val=\"50000014\"/><c:crosses val=\"max\"/><c:crossBetween val=\"between\"/></c:valAx>"));
    let hidden = element(xml, "c:catAx", 1);
    assert!(hidden.starts_with("<c:catAx><c:axId val=\"50000014\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"1\"/><c:axPos val=\"b\"/>"));
    assert!(
        hidden.contains(
            "<c:crossAx val=\"50000013\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/>"
        )
    );

    // Bars: categories up the left side, the second scale along the top,
    // each group with its own grouping and overlap.
    let xml = &parts[1];
    assert!(
        element(xml, "c:barChart", 0)
            .contains("<c:barDir val=\"bar\"/><c:grouping val=\"clustered\"/>")
    );
    assert!(element(xml, "c:barChart", 0).ends_with("<c:gapWidth val=\"182\"/><c:overlap val=\"0\"/><c:axId val=\"50000021\"/><c:axId val=\"50000022\"/></c:barChart>"));
    assert!(
        element(xml, "c:barChart", 1)
            .contains("<c:barDir val=\"bar\"/><c:grouping val=\"stacked\"/>")
    );
    assert!(element(xml, "c:barChart", 1).ends_with("<c:gapWidth val=\"182\"/><c:overlap val=\"100\"/><c:axId val=\"50000024\"/><c:axId val=\"50000023\"/></c:barChart>"));
    let sides: Vec<&str> = xml
        .split("<c:axPos val=\"")
        .skip(1)
        .map(|s| &s[..1])
        .collect();
    assert_eq!(sides, ["l", "b", "t", "l"]);

    // X/Y: four scales; the second X axis is shown, and so across the top
    // (at the bottom, it would lie on the first).
    let xml = &parts[2];
    assert_eq!(xml.matches("<c:scatterChart>").count(), 2);
    assert_eq!(
        axes_in(xml),
        [
            ("c:valAx", "50000031"),
            ("c:valAx", "50000032"),
            ("c:valAx", "50000033"),
            ("c:valAx", "50000034")
        ]
    );
    assert!(element(xml, "c:valAx", 2).ends_with("<c:crossAx val=\"50000034\"/><c:crossesAt val=\"2.5\"/><c:crossBetween val=\"midCat\"/></c:valAx>"));
    let top = element(xml, "c:valAx", 3);
    assert!(top.contains("<c:delete val=\"0\"/><c:axPos val=\"b\"/>"));
    assert!(top.ends_with("<c:crossAx val=\"50000033\"/><c:crosses val=\"max\"/><c:crossBetween val=\"midCat\"/></c:valAx>"));
}

#[test]
fn series_types_a_chart_cannot_show_are_not_written() {
    let of = |kind: ChartKind, column: char| Series {
        kind: Some(kind),
        ..monthly(column)
    };
    // A pie, horizontal bars and 3-D columns do not mix with columns:
    // they are columns. A type that is the chart's own in other looks is
    // the chart's type, in those looks.
    let unmixed = chart(
        ChartKind::Column,
        vec![
            monthly('B'),
            of(ChartKind::Pie, 'C'),
            of(ChartKind::Bar, 'D'),
            of(ChartKind::Column3D, 'E'),
            of(ChartKind::Column, 'B'),
        ],
    );
    let looks = chart(
        ChartKind::Line,
        vec![monthly('B'), of(ChartKind::LineMarkers, 'C')],
    );
    // Secondary axes are a second pair: not without a first, and not in
    // charts that have a single set of axes.
    let mut alone = plain(ChartKind::Column);
    for s in &mut alone.series {
        s.secondary = true;
    }
    let single: Vec<Chart> = [ChartKind::Column3D, ChartKind::Doughnut, ChartKind::Radar]
        .into_iter()
        .map(|kind| {
            let mut c = plain(kind);
            c.series[1].secondary = true;
            c
        })
        .collect();
    let (parts, read) = written([vec![unmixed, looks, alone], single].concat());

    assert_eq!(parts[0].matches("<c:barChart>").count(), 1);
    assert_eq!(parts[0].matches("<c:ser>").count(), 5);
    assert!(!parts[0].contains("pieChart") && !parts[0].contains("bar3DChart"));
    assert!(read[0].series.iter().all(|s| s.kind.is_none()));
    assert_eq!(parts[1].matches("<c:lineChart>").count(), 1);
    assert_eq!(read[1].kind, ChartKind::Line);
    assert_eq!(read[1].series[1].kind, None);
    assert_eq!(
        read[1].series[1].marker,
        Some(Marker::new(MarkerShape::Circle))
    );
    for (xml, chart) in parts[2..].iter().zip(&read[2..]) {
        assert!(axes_in(xml).len() <= 2, "{:?}", chart.kind);
        assert!(!xml.contains("<c:crosses val=\"max\"/>"));
        assert!(
            chart.series.iter().all(|s| !s.secondary),
            "{:?}",
            chart.kind
        );
    }
}

#[test]
fn trendlines() {
    let trend = Trendline::new;
    let mut c = chart(ChartKind::Column, vec![monthly('B')]);
    c.series[0].color = Some(Color(0x112233));
    c.series[0].trendlines = vec![
        trend(TrendKind::Linear),
        Trendline {
            name: Some("Custom <name>".into()),
            forward: 2.0,
            backward: 0.5,
            intercept: Some(1.5),
            show_equation: true,
            show_r2: true,
            line: Line {
                shown: None,
                color: Some(Color(0xAA0000)),
                width: Some(2.0),
                dash: Dash::Dash,
            },
            ..trend(TrendKind::Polynomial(3))
        },
        Trendline {
            intercept: Some(2.0),
            ..trend(TrendKind::Exponential)
        },
        trend(TrendKind::Logarithmic),
        trend(TrendKind::Power),
        trend(TrendKind::MovingAverage(4)),
        Trendline {
            line: Line {
                shown: Some(false),
                ..Line::default()
            },
            ..trend(TrendKind::Linear)
        },
        // A solid line: not the automatic one, which is dotted.
        Trendline {
            line: Line {
                width: Some(1.5),
                ..Line::default()
            },
            ..trend(TrendKind::Linear)
        },
    ];
    let parts = assert_round_trip(vec![c]);
    let xml = &parts[0];
    let lines: Vec<&str> = (0..8).map(|n| element(xml, "c:trendline", n)).collect();
    // The automatic line is Excel's: round dots in the colour of the series.
    let dotted = "<c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"112233\"/></a:solidFill><a:prstDash val=\"sysDot\"/><a:round/></a:ln></c:spPr>";
    let off = "<c:dispRSqr val=\"0\"/><c:dispEq val=\"0\"/></c:trendline>";
    assert_eq!(
        lines[0],
        format!("<c:trendline>{dotted}<c:trendlineType val=\"linear\"/>{off}")
    );
    assert_eq!(
        lines[1],
        "<c:trendline><c:name>Custom &lt;name&gt;</c:name><c:spPr><a:ln w=\"25400\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"AA0000\"/></a:solidFill><a:prstDash val=\"dash\"/><a:round/></a:ln></c:spPr><c:trendlineType val=\"poly\"/><c:order val=\"3\"/><c:forward val=\"2\"/><c:backward val=\"0.5\"/><c:intercept val=\"1.5\"/><c:dispRSqr val=\"1\"/><c:dispEq val=\"1\"/></c:trendline>"
    );
    assert_eq!(
        lines[2],
        format!("<c:trendline>{dotted}<c:trendlineType val=\"exp\"/><c:intercept val=\"2\"/>{off}")
    );
    assert_eq!(
        lines[3],
        format!("<c:trendline>{dotted}<c:trendlineType val=\"log\"/>{off}")
    );
    assert_eq!(
        lines[4],
        format!("<c:trendline>{dotted}<c:trendlineType val=\"power\"/>{off}")
    );
    assert_eq!(
        lines[5],
        format!(
            "<c:trendline>{dotted}<c:trendlineType val=\"movingAvg\"/><c:period val=\"4\"/>{off}"
        )
    );
    assert_eq!(
        lines[6],
        format!(
            "<c:trendline><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:trendlineType val=\"linear\"/>{off}"
        )
    );
    assert_eq!(
        lines[7],
        format!(
            "<c:trendline><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"112233\"/></a:solidFill><a:round/></a:ln></c:spPr><c:trendlineType val=\"linear\"/>{off}"
        )
    );
    // Between the labels of the series and its data.
    assert!(xml.contains("<c:invertIfNegative val=\"0\"/><c:trendline>"));
    assert!(xml.contains("</c:trendline><c:cat>"));

    // Excel takes a file for damaged over an order, a period or an
    // intercept it would not let one type in: they are brought to what
    // it accepts, and read back that way.
    let mut c = chart(ChartKind::Scatter, vec![measured('H')]);
    c.series[0].trendlines = vec![
        Trendline {
            intercept: Some(3.0),
            ..trend(TrendKind::Logarithmic)
        },
        Trendline {
            intercept: Some(3.0),
            ..trend(TrendKind::Power)
        },
        Trendline {
            intercept: Some(0.0),
            ..trend(TrendKind::Exponential)
        },
        trend(TrendKind::Polynomial(9)),
        trend(TrendKind::Polynomial(1)),
        Trendline {
            forward: 2.0,
            backward: 1.0,
            intercept: Some(1.0),
            show_equation: true,
            show_r2: true,
            ..trend(TrendKind::MovingAverage(1))
        },
        Trendline {
            forward: -1.0,
            backward: f64::NAN,
            ..trend(TrendKind::Linear)
        },
    ];
    let (parts, read) = written(vec![c]);
    let xml = &parts[0];
    for refused in [
        "<c:intercept",
        "<c:forward",
        "<c:backward",
        "val=\"1\"/><c:dispEq",
    ] {
        assert!(!xml.contains(refused), "{refused}");
    }
    assert!(xml.contains("<c:order val=\"6\"/>") && xml.contains("<c:order val=\"2\"/>"));
    assert!(xml.contains("<c:trendlineType val=\"movingAvg\"/><c:period val=\"2\"/><c:dispRSqr val=\"0\"/><c:dispEq val=\"0\"/>"));
    assert_eq!(
        read[0].series[0].trendlines,
        [
            trend(TrendKind::Logarithmic),
            trend(TrendKind::Power),
            trend(TrendKind::Exponential),
            trend(TrendKind::Polynomial(6)),
            trend(TrendKind::Polynomial(2)),
            trend(TrendKind::MovingAverage(2)),
            trend(TrendKind::Linear),
        ]
    );
}

#[test]
fn trendlines_and_error_bars_only_where_excel_has_them() {
    let bars = ErrorBars {
        horizontal: false,
        direction: ErrorDirection::Both,
        amount: ErrorAmount::Fixed(1.0),
        end_cap: true,
    };
    let charts: Vec<Chart> = ChartKind::ALL
        .into_iter()
        .map(|kind| {
            let mut c = plain(kind);
            c.series[0].trendlines = vec![Trendline::new(TrendKind::Linear)];
            c.series[0].error_bars = vec![bars.clone()];
            c
        })
        .collect();
    let (parts, read) = written(charts);
    for ((kind, xml), chart) in ChartKind::ALL.into_iter().zip(&parts).zip(&read) {
        // Not in 3-D, not in pies and doughnuts, not in radar charts; no
        // trendline on what is stacked.
        let flat = !kind.is_3d() && !kind.is_round() && kind.family() != Family::Radar;
        let trends = usize::from(flat && !kind.is_stacked());
        assert_eq!(xml.matches("<c:trendline>").count(), trends, "{kind:?}");
        assert_eq!(
            xml.matches("<c:errBars>").count(),
            usize::from(flat),
            "{kind:?}"
        );
        assert_eq!(chart.series[0].trendlines.len(), trends, "{kind:?}");
        assert_eq!(
            chart.series[0].error_bars.len(),
            usize::from(flat),
            "{kind:?}"
        );
    }
}

#[test]
fn error_bars() {
    let bars = |horizontal: bool, direction: ErrorDirection, amount: ErrorAmount, end_cap: bool| {
        ErrorBars {
            horizontal,
            direction,
            amount,
            end_cap,
        }
    };
    use ErrorDirection::{Both, Minus, Plus};
    let mut columns = plain(ChartKind::Column);
    columns.series[0].error_bars = vec![bars(false, Both, ErrorAmount::StdErr, true)];
    columns.series[1].error_bars = vec![bars(false, Minus, ErrorAmount::StdDev(1.5), true)];
    let mut sideways = plain(ChartKind::StackedBar);
    sideways.series[0].error_bars = vec![bars(false, Plus, ErrorAmount::Percent(20.0), false)];
    let mut points = plain(ChartKind::Scatter);
    points.series[0].error_bars = vec![
        bars(true, Minus, ErrorAmount::Fixed(0.25), false),
        bars(false, Plus, ErrorAmount::Percent(20.0), true),
    ];
    let parts = assert_round_trip(vec![columns, sideways, points]);
    assert!(parts[0].contains("<c:errBars><c:errDir val=\"y\"/><c:errBarType val=\"both\"/><c:errValType val=\"stdErr\"/><c:noEndCap val=\"0\"/></c:errBars><c:cat>"));
    assert!(parts[0].contains("<c:errBars><c:errDir val=\"y\"/><c:errBarType val=\"minus\"/><c:errValType val=\"stdDev\"/><c:noEndCap val=\"0\"/><c:val val=\"1.5\"/></c:errBars><c:cat>"));
    // Along the value axis either way: for horizontal bars Excel does not
    // say which.
    assert!(parts[1].contains("<c:errBars><c:errBarType val=\"plus\"/><c:errValType val=\"percentage\"/><c:noEndCap val=\"1\"/><c:val val=\"20\"/></c:errBars><c:cat>"));
    assert!(parts[2].contains("<c:errBars><c:errDir val=\"x\"/><c:errBarType val=\"minus\"/><c:errValType val=\"fixedVal\"/><c:noEndCap val=\"1\"/><c:val val=\"0.25\"/></c:errBars><c:errBars><c:errDir val=\"y\"/><c:errBarType val=\"plus\"/><c:errValType val=\"percentage\"/><c:noEndCap val=\"0\"/><c:val val=\"20\"/></c:errBars><c:xVal>"));

    // One set along each axis a series is measured on; a chart with
    // categories has one such axis.
    let mut columns = plain(ChartKind::Column);
    columns.series[0].error_bars = vec![
        bars(true, Both, ErrorAmount::Fixed(1.0), true),
        bars(false, Both, ErrorAmount::Fixed(2.0), true),
        bars(false, Both, ErrorAmount::Fixed(-3.0), true),
    ];
    let (parts, read) = written(vec![columns]);
    assert_eq!(parts[0].matches("<c:errBars>").count(), 1);
    assert_eq!(
        read[0].series[0].error_bars,
        [bars(false, Both, ErrorAmount::Fixed(2.0), true)]
    );
}

/// Where Excel accepts the labels of a series of each kind.
fn label_positions(kind: ChartKind) -> &'static [LabelPosition] {
    use LabelPosition::*;
    match kind.family() {
        Family::Pie => &[BestFit, OutsideEnd, InsideEnd, Center],
        _ if kind.is_3d() => &[],
        Family::Column | Family::Bar if !kind.is_stacked() => {
            &[OutsideEnd, InsideEnd, InsideBase, Center]
        }
        Family::Column | Family::Bar => &[InsideEnd, InsideBase, Center],
        Family::Line | Family::Scatter | Family::Bubble => &[Above, Below, Left, Right, Center],
        _ => &[],
    }
}

#[test]
fn label_positions_are_written_where_excel_takes_them() {
    use LabelPosition::*;
    let all = [
        Auto, Center, InsideEnd, InsideBase, OutsideEnd, BestFit, Above, Below, Left, Right,
    ];
    let names = [
        "", "ctr", "inEnd", "inBase", "outEnd", "bestFit", "t", "b", "l", "r",
    ];
    // A chart of each kind with a series for each position.
    let charts: Vec<Chart> = ChartKind::ALL
        .into_iter()
        .map(|kind| {
            let mut c = plain(kind);
            let first = c.series[0].clone();
            c.series = all
                .into_iter()
                .map(|position| Series {
                    labels: Some(labels_at(position)),
                    ..first.clone()
                })
                .collect();
            c
        })
        .collect();
    let (parts, read) = written(charts);
    for ((kind, xml), chart) in ChartKind::ALL.into_iter().zip(&parts).zip(&read) {
        let legal = label_positions(kind);
        for (position, name) in all.into_iter().zip(names) {
            let written = xml.contains(&format!("<c:dLblPos val=\"{name}\"/>"));
            assert_eq!(written, legal.contains(&position), "{kind:?} {position:?}");
        }
        assert_eq!(xml.matches("<c:dLblPos").count(), legal.len(), "{kind:?}");
        assert_eq!(xml.matches("<c:dLbls>").count(), all.len(), "{kind:?}");
        if legal.is_empty() {
            // Ten series labelled the same way: that is the chart's way.
            assert!(chart.data_labels && chart.label_options == DataLabels::default());
            assert!(chart.series.iter().all(|s| s.labels.is_none()), "{kind:?}");
            continue;
        }
        // Anything else is left to Excel.
        assert!(!chart.data_labels, "{kind:?}");
        for (series, position) in chart.series.iter().zip(all) {
            let kept = if legal.contains(&position) {
                position
            } else {
                Auto
            };
            assert_eq!(
                series.labels,
                Some(labels_at(kept)),
                "{kind:?} {position:?}"
            );
        }
    }
}

#[test]
fn data_label_options() {
    let everything = DataLabels {
        value: true,
        category: true,
        series_name: true,
        percent: true,
        legend_key: true,
        bubble_size: true,
        separator: Some("; ".into()),
        position: LabelPosition::BestFit,
        number_format: Some("0.0%;\"<none>\"".into()),
        leader_lines: false,
        text: TextFormat::default(),
    };
    let mut pie = plain(ChartKind::Pie);
    pie.data_labels = true;
    pie.label_options = everything.clone();
    // The chart's labels, a series with its own, a series without any.
    let mut columns = plain(ChartKind::Column);
    columns.data_labels = true;
    columns.series[1].labels = Some(DataLabels {
        value: false,
        series_name: true,
        separator: Some("\n".into()),
        position: LabelPosition::InsideBase,
        ..DataLabels::default()
    });
    columns.series[2].labels = Some(DataLabels::none());
    // Labels of the chart in a combination: each type has them where it
    // can.
    let mut mixed = chart(
        ChartKind::Column,
        vec![
            monthly('B'),
            Series {
                kind: Some(ChartKind::LineMarkers),
                ..monthly('C')
            },
            Series {
                kind: Some(ChartKind::Area),
                labels: Some(DataLabels {
                    separator: Some(" & ".into()),
                    category: true,
                    ..DataLabels::default()
                }),
                ..monthly('D')
            },
        ],
    );
    mixed.data_labels = true;
    mixed.label_options = labels_at(LabelPosition::OutsideEnd);
    let parts = assert_round_trip(vec![pie, columns, mixed]);

    assert!(parts[0].contains("</c:ser><c:dLbls><c:numFmt formatCode=\"0.0%;&quot;&lt;none&gt;&quot;\" sourceLinked=\"0\"/><c:dLblPos val=\"bestFit\"/><c:showLegendKey val=\"1\"/><c:showVal val=\"1\"/><c:showCatName val=\"1\"/><c:showSerName val=\"1\"/><c:showPercent val=\"1\"/><c:showBubbleSize val=\"1\"/><c:separator>; </c:separator><c:showLeaderLines val=\"0\"/></c:dLbls><c:firstSliceAng val=\"0\"/>"), "{}", parts[0]);
    let plain_labels = "<c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/></c:dLbls>";
    let xml = &parts[1];
    assert!(xml.contains(&format!("</c:ser><c:dLbls>{plain_labels}<c:gapWidth")));
    assert!(!element(xml, "c:ser", 0).contains("<c:dLbls>"));
    assert!(element(xml, "c:ser", 1).contains("<c:invertIfNegative val=\"0\"/><c:dLbls><c:dLblPos val=\"inBase\"/><c:showLegendKey val=\"0\"/><c:showVal val=\"0\"/><c:showCatName val=\"0\"/><c:showSerName val=\"1\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/><c:separator>\n</c:separator></c:dLbls><c:cat>"));
    assert!(element(xml, "c:ser", 2).contains(
        "<c:invertIfNegative val=\"0\"/><c:dLbls><c:delete val=\"1\"/></c:dLbls><c:cat>"
    ));
    let xml = &parts[2];
    assert!(element(xml, "c:barChart", 0).contains(&format!(
        "</c:ser><c:dLbls><c:dLblPos val=\"outEnd\"/>{plain_labels}<c:gapWidth"
    )));
    assert!(element(xml, "c:lineChart", 0).contains(&format!(
        "</c:ser><c:dLbls>{plain_labels}<c:marker val=\"1\"/>"
    )));
    assert!(element(xml, "c:areaChart", 0).contains(
        "<c:showBubbleSize val=\"0\"/><c:separator> &amp; </c:separator></c:dLbls><c:cat>"
    ));

    // What a file cannot say comes back as what it does say.
    let mut off = plain(ChartKind::Column);
    off.label_options = everything.clone();
    off.series[0].labels = Some(DataLabels::none());
    let mut nothing = plain(ChartKind::Column);
    nothing.data_labels = true;
    nothing.label_options = DataLabels {
        legend_key: true,
        ..DataLabels::none()
    };
    let mut straight = plain(ChartKind::Column);
    straight.data_labels = true;
    straight.label_options.leader_lines = false;
    // Labels every series has are the chart's; so is their absence.
    let mut each = plain(ChartKind::Column);
    let mut none = plain(ChartKind::Column);
    none.data_labels = true;
    for (a, b) in each.series.iter_mut().zip(&mut none.series) {
        a.labels = Some(labels_at(LabelPosition::Center));
        b.labels = Some(DataLabels::none());
    }
    let (parts, read) = written(vec![off, nothing, straight, each, none]);
    for (xml, chart) in parts.iter().zip(&read).take(2) {
        assert!(!xml.contains("<c:dLbls>"));
        assert!(!chart.data_labels && chart.label_options == DataLabels::default());
        assert!(chart.series.iter().all(|s| s.labels.is_none()));
    }
    assert!(!parts[2].contains("showLeaderLines"));
    assert!(read[2].data_labels && read[2].label_options == DataLabels::default());
    assert!(read[3].data_labels);
    assert_eq!(read[3].label_options, labels_at(LabelPosition::Center));
    assert!(!read[4].data_labels);
    for chart in &read[3..] {
        assert!(chart.series.iter().all(|s| s.labels.is_none()));
    }
}

#[test]
fn the_separator_of_label_parts() {
    // Left to the engine, the parts of a label are separated by the list
    // separator of the locale and a space. Excel left to itself puts them
    // on lines of their own: the separator is spelled out where it shows.
    let several = DataLabels {
        category: true,
        percent: true,
        ..DataLabels::default()
    };
    let with = |separator: Option<&str>| {
        let mut c = plain(ChartKind::Column);
        c.data_labels = true;
        c.label_options = DataLabels {
            separator: separator.map(str::to_string),
            ..several.clone()
        };
        c
    };
    let mut single = plain(ChartKind::Column);
    single.data_labels = true;
    let parts = assert_round_trip(vec![with(None), with(Some("; ")), with(Some("")), single]);
    assert!(
        parts[0].contains("<c:showBubbleSize val=\"0\"/><c:separator>, </c:separator></c:dLbls>")
    );
    assert!(parts[1].contains("<c:separator>; </c:separator>"));
    assert!(parts[2].contains("<c:separator></c:separator>"));
    assert!(!parts[3].contains("<c:separator>"));
    // A separator that is the automatic one is no separator of its own.
    let (_, read) = written(vec![with(Some(", "))]);
    assert_eq!(read[0].label_options, several);

    // In a workbook in French, lists are separated by semicolons.
    let mut wb = data_workbook();
    wb.locale = Locale::fr();
    for c in [with(None), with(Some(", ")), with(Some("; "))] {
        wb.add_chart(1, c).unwrap();
    }
    let bytes = to_bytes(&wb);
    let chart = |n: usize| part(&bytes, &format!("xl/charts/chart{n}.xml"));
    assert!(chart(1).contains("<c:separator>; </c:separator>"));
    assert!(chart(2).contains("<c:separator>, </c:separator>"));
    let read = read_xlsx(Cursor::new(&bytes), Locale::fr()).unwrap();
    let separators: Vec<Option<&str>> = read.sheets[1]
        .charts
        .iter()
        .map(|c| c.label_options.separator.as_deref())
        .collect();
    assert_eq!(separators, [None, Some(", "), None]);
}

#[test]
fn markers_made_of_strokes_have_no_fill() {
    // Excel paints the fill of a cross, a star or a plus as a square
    // behind it: these are written without one, in the colour of their
    // strokes.
    let mut c = chart(ChartKind::LineMarkers, Vec::new());
    for (i, shape) in [
        MarkerShape::X,
        MarkerShape::Star,
        MarkerShape::Plus,
        MarkerShape::Dash,
        MarkerShape::Dot,
    ]
    .into_iter()
    .enumerate()
    {
        let mut s = monthly(['B', 'C', 'D'][i % 3]);
        s.marker = Some(Marker::new(shape));
        c.series.push(s);
    }
    c.series[1].color = Some(Color(0x7030A0));
    c.series[2].marker = Some(Marker {
        line: Some(Color(0xC00000)),
        size: 9,
        ..Marker::new(MarkerShape::Plus)
    });
    let mut points = chart(ChartKind::Scatter, vec![measured('H')]);
    points.series[0].marker = Some(Marker::new(MarkerShape::X));
    points.series[0].points = vec![point(2, Some(0x00B050), None)];
    let parts = assert_round_trip(vec![c, points]);
    let stroked = |symbol: &str, size: u8, rgb: &str| {
        format!(
            "<c:marker><c:symbol val=\"{symbol}\"/><c:size val=\"{size}\"/><c:spPr><a:noFill/><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill></a:ln></c:spPr></c:marker>"
        )
    };
    let series = |n: usize| element(&parts[0], "c:ser", n);
    assert!(series(0).contains(&stroked("x", 5, "156082")));
    assert!(series(1).contains(&stroked("star", 5, "7030A0")));
    assert!(series(2).contains(&stroked("plus", 9, "C00000")));
    // A dash and a dot are small bars: they have an inside.
    for (n, symbol) in [(3, "dash"), (4, "dot")] {
        assert!(series(n).contains(&format!(
            "<c:symbol val=\"{symbol}\"/><c:size val=\"5\"/><c:spPr><a:solidFill>"
        )));
    }
    assert!(parts[1].contains(&format!(
        "<c:dPt><c:idx val=\"2\"/>{}",
        stroked("x", 5, "00B050")
    )));

    // In a file that gives such a marker a fill, the fill is not its
    // colour: the strokes are.
    let filled = bare_part(&format!(
        "<c:lineChart>{}</c:lineChart>",
        bare_series(
            0,
            "<c:spPr><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val=\"star\"/><c:size val=\"5\"/><c:spPr><a:solidFill><a:srgbClr val=\"00FF00\"/></a:solidFill><a:ln><a:solidFill><a:srgbClr val=\"0000FF\"/></a:solidFill></a:ln></c:spPr></c:marker>"
        )
    ));
    let read = excel(&filled);
    assert_eq!(read.series[0].color, Some(Color(0xFF0000)));
    assert_eq!(
        read.series[0].marker,
        Some(Marker {
            line: Some(Color(0x0000FF)),
            ..Marker::new(MarkerShape::Star)
        })
    );
}

#[test]
fn axis_options_and_where_axes_cross() {
    let mut columns = plain(ChartKind::Column);
    columns.axes.category = Axis {
        title: Some("Months & more".into()),
        reversed: true,
        number_format: Some("@".into()),
        major_gridlines: true,
        minor_gridlines: true,
        crossing: AxisCrossing::Max,
        major_ticks: TickMark::Inside,
        minor_ticks: TickMark::Cross,
        labels: TickLabels::Low,
        ..Axis::new()
    };
    columns.axes.value = Axis {
        title: Some("Value".into()),
        min: Some(-5.0),
        max: Some(45.5),
        major_unit: Some(10.0),
        minor_unit: Some(2.0),
        number_format: Some("#,##0.0".into()),
        major_gridlines: true,
        minor_gridlines: true,
        crossing: AxisCrossing::At(12.5),
        display_unit: DisplayUnit::Thousands,
        major_ticks: TickMark::Outside,
        minor_ticks: TickMark::Outside,
        labels: TickLabels::High,
        ..Axis::new()
    };
    let mut points = plain(ChartKind::Scatter);
    points.axes.category = Axis {
        visible: false,
        min: Some(0.1),
        max: Some(1000.0),
        log_base: Some(10.0),
        labels: TickLabels::None,
        crossing: AxisCrossing::Min,
        ..Axis::new()
    };
    points.axes.value = Axis {
        crossing: AxisCrossing::At(-2.5),
        display_unit: DisplayUnit::Custom(250.0),
        display_unit_label: false,
        ..Axis::new()
    };
    let parts = assert_round_trip(vec![columns, points]);

    let minor = HAIRLINE.replace("D9D9D9", "F2F2F2");
    let title = |upright: &str, text: &str| {
        format!(
            "<c:title><c:tx><c:rich><a:bodyPr{upright}/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1000\" b=\"0\"/></a:pPr><a:r><a:t>{text}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title>"
        )
    };
    // Where an axis crosses the other one is said on the axis that does
    // the crossing: the category axis says "at 12.5" of the values, the
    // value axis "at the last" of the categories.
    assert_eq!(
        element(&parts[0], "c:catAx", 0),
        format!(
            "<c:catAx><c:axId val=\"50000011\"/><c:scaling><c:orientation val=\"maxMin\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:majorGridlines>{HAIRLINE}</c:majorGridlines><c:minorGridlines>{minor}</c:minorGridlines>{}<c:numFmt formatCode=\"@\" sourceLinked=\"0\"/><c:majorTickMark val=\"in\"/><c:minorTickMark val=\"cross\"/><c:tickLblPos val=\"low\"/>{HAIRLINE}<c:crossAx val=\"50000012\"/><c:crossesAt val=\"12.5\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/></c:catAx>",
            title("", "Months &amp; more")
        )
    );
    assert_eq!(
        element(&parts[0], "c:valAx", 0),
        format!(
            "<c:valAx><c:axId val=\"50000012\"/><c:scaling><c:orientation val=\"minMax\"/><c:max val=\"45.5\"/><c:min val=\"-5\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:majorGridlines>{HAIRLINE}</c:majorGridlines><c:minorGridlines>{minor}</c:minorGridlines>{}<c:numFmt formatCode=\"#,##0.0\" sourceLinked=\"0\"/><c:majorTickMark val=\"out\"/><c:minorTickMark val=\"out\"/><c:tickLblPos val=\"high\"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:crossAx val=\"50000011\"/><c:crosses val=\"max\"/><c:crossBetween val=\"between\"/><c:majorUnit val=\"10\"/><c:minorUnit val=\"2\"/><c:dispUnits><c:builtInUnit val=\"thousands\"/><c:dispUnitsLbl><c:layout/></c:dispUnitsLbl></c:dispUnits></c:valAx>",
            title(" rot=\"-5400000\" vert=\"horz\"", "Value")
        )
    );
    // Both axes of an X/Y chart are scales.
    assert_eq!(
        element(&parts[1], "c:valAx", 0),
        format!(
            "<c:valAx><c:axId val=\"50000021\"/><c:scaling><c:logBase val=\"10\"/><c:orientation val=\"minMax\"/><c:max val=\"1000\"/><c:min val=\"0.1\"/></c:scaling><c:delete val=\"1\"/><c:axPos val=\"b\"/><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"none\"/>{HAIRLINE}<c:crossAx val=\"50000022\"/><c:crossesAt val=\"-2.5\"/><c:crossBetween val=\"midCat\"/></c:valAx>"
        )
    );
    assert!(element(&parts[1], "c:valAx", 1).ends_with("<c:crossAx val=\"50000021\"/><c:crosses val=\"min\"/><c:crossBetween val=\"midCat\"/><c:dispUnits><c:custUnit val=\"250\"/></c:dispUnits></c:valAx>"));

    // Every unit, tick mark and place for labels.
    let units = [
        (DisplayUnit::Hundreds, "hundreds"),
        (DisplayUnit::Thousands, "thousands"),
        (DisplayUnit::TenThousands, "tenThousands"),
        (DisplayUnit::HundredThousands, "hundredThousands"),
        (DisplayUnit::Millions, "millions"),
        (DisplayUnit::TenMillions, "tenMillions"),
        (DisplayUnit::HundredMillions, "hundredMillions"),
        (DisplayUnit::Billions, "billions"),
        (DisplayUnit::Trillions, "trillions"),
    ];
    let ticks = [
        TickMark::None,
        TickMark::Inside,
        TickMark::Outside,
        TickMark::Cross,
    ];
    let places = [
        TickLabels::NextToAxis,
        TickLabels::High,
        TickLabels::Low,
        TickLabels::None,
    ];
    let charts: Vec<Chart> = units
        .iter()
        .enumerate()
        .map(|(i, (unit, _))| {
            let mut c = plain(ChartKind::Line);
            c.axes.value.display_unit = *unit;
            c.axes.value.display_unit_label = i % 2 == 0;
            c.axes.value.major_ticks = ticks[i % 4];
            c.axes.category.minor_ticks = ticks[(i + 1) % 4];
            c.axes.category.labels = places[i % 4];
            c
        })
        .collect();
    let parts = assert_round_trip(charts);
    for (xml, (_, name)) in parts.iter().zip(units) {
        assert!(xml.contains(&format!("<c:dispUnits><c:builtInUnit val=\"{name}\"/>")));
    }

    // What an axis cannot have is not written: a scale on a list of
    // categories, bounds a logarithmic scale cannot reach, a scale that
    // ends before it starts, units of nothing.
    let mut odd = plain(ChartKind::Column);
    odd.axes.category = Axis {
        min: Some(1.0),
        max: Some(3.0),
        major_unit: Some(2.0),
        log_base: Some(10.0),
        display_unit: DisplayUnit::Millions,
        number_format: Some(String::new()),
        crossing: AxisCrossing::At(f64::NAN),
        ..Axis::new()
    };
    odd.axes.value = Axis {
        min: Some(-10.0),
        max: Some(0.0),
        log_base: Some(10.0),
        major_unit: Some(0.0),
        minor_unit: Some(-1.0),
        display_unit: DisplayUnit::Custom(0.0),
        ..Axis::new()
    };
    let mut backwards = plain(ChartKind::Column);
    backwards.axes.value.min = Some(10.0);
    backwards.axes.value.max = Some(5.0);
    backwards.axes.value.log_base = Some(1.0);
    let (parts, read) = written(vec![odd, backwards]);
    assert_eq!(read[0].axes.category, Axis::new());
    assert_eq!(
        read[0].axes.value,
        Axis {
            log_base: Some(10.0),
            ..Axis::new()
        }
    );
    assert!(
        parts[0].contains(
            "<c:scaling><c:logBase val=\"10\"/><c:orientation val=\"minMax\"/></c:scaling>"
        )
    );
    assert_eq!(
        read[1].axes.value,
        Axis {
            min: Some(10.0),
            major_gridlines: true,
            ..Axis::new()
        }
    );
}

#[test]
fn three_d_charts() {
    let mut columns = plain(ChartKind::Column3D);
    columns.view3d = Some(View3D {
        rot_x: 30,
        rot_y: 40,
        perspective: 60,
        right_angle_axes: false,
        depth_percent: 200,
        height_percent: Some(80),
    });
    let mut pie = plain(ChartKind::Pie3D);
    pie.first_slice_angle = 120;
    let parts = assert_round_trip(vec![
        columns,
        plain(ChartKind::StackedBar3D),
        plain(ChartKind::Line3D),
        plain(ChartKind::Area3D),
        plain(ChartKind::PercentStackedArea3D),
        pie,
    ]);
    let wall = "<c:thickness val=\"0\"/><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr>";
    let walls = format!(
        "<c:floor>{wall}</c:floor><c:sideWall>{wall}</c:sideWall><c:backWall>{wall}</c:backWall><c:plotArea>"
    );
    // The camera, in the order of the schema; floor and walls.
    assert!(parts[0].contains(&format!("<c:autoTitleDeleted val=\"0\"/><c:view3D><c:rotX val=\"30\"/><c:hPercent val=\"80\"/><c:rotY val=\"40\"/><c:depthPercent val=\"200\"/><c:rAngAx val=\"0\"/><c:perspective val=\"60\"/></c:view3D>{walls}")));
    // Bars in 3-D: boxes, without an overlap, and a third axis they say
    // they do not have.
    assert!(parts[0].contains("<c:bar3DChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/><c:ser>"));
    assert!(parts[0].contains("</c:ser><c:gapWidth val=\"150\"/><c:shape val=\"box\"/><c:axId val=\"50000011\"/><c:axId val=\"50000012\"/><c:axId val=\"0\"/></c:bar3DChart><c:catAx>"));
    assert_eq!(axes_in(&parts[0]).len(), 2);
    // Left to itself, the camera is Excel's for the type.
    assert!(parts[1].contains(&format!("<c:view3D><c:rotX val=\"15\"/><c:rotY val=\"20\"/><c:depthPercent val=\"100\"/><c:rAngAx val=\"1\"/><c:perspective val=\"30\"/></c:view3D>{walls}")));
    assert!(
        parts[1].contains("<c:bar3DChart><c:barDir val=\"bar\"/><c:grouping val=\"stacked\"/>")
    );
    // Lines and unstacked areas have a row per series along a third axis.
    for (xml, element_name) in [(&parts[2], "c:line3DChart"), (&parts[3], "c:area3DChart")] {
        let n = if element_name == "c:line3DChart" {
            3
        } else {
            4
        };
        assert!(xml.contains(&format!(
            "<c:{0}><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/><c:ser>",
            &element_name[2..]
        )));
        assert!(xml.contains(&format!("</c:ser><c:axId val=\"500000{n}1\"/><c:axId val=\"500000{n}2\"/><c:axId val=\"500000{n}5\"/></{element_name}>")));
        assert!(xml.contains(&format!("</c:valAx><c:serAx><c:axId val=\"500000{n}5\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/>{HAIRLINE}<c:crossAx val=\"500000{n}2\"/><c:crosses val=\"autoZero\"/></c:serAx><c:spPr>")));
        assert!(xml.contains("<c:rAngAx val=\"0\"/>"));
    }
    // A ribbon is a shape: no marker, never curved.
    assert!(!parts[2].contains("<c:marker") && !parts[2].contains("<c:smooth val=\"1\"/>"));
    assert_eq!(parts[2].matches("<c:smooth val=\"0\"/>").count(), 3);
    assert!(parts[4].contains("<c:area3DChart><c:grouping val=\"percentStacked\"/>"));
    assert!(parts[4].contains(
        "<c:axId val=\"50000051\"/><c:axId val=\"50000052\"/><c:axId val=\"0\"/></c:area3DChart>"
    ));
    assert!(!parts[4].contains("serAx"));
    // A pie in 3-D is turned by its camera.
    assert!(parts[5].contains(&format!("<c:view3D><c:rotX val=\"30\"/><c:rotY val=\"120\"/><c:depthPercent val=\"100\"/><c:rAngAx val=\"0\"/><c:perspective val=\"30\"/></c:view3D>{walls}")));
    assert!(parts[5].contains("</c:ser></c:pie3DChart><c:spPr>"));
    assert!(!parts[5].contains("firstSliceAng") && !parts[5].contains("Ax>"));

    // What is not in 3-D has no camera, and a pie one angle for two:
    // turning it around is moving its first slice.
    let mut flat = plain(ChartKind::Column);
    flat.view3d = Some(View3D::for_kind(ChartKind::Pie3D));
    let mut turned = plain(ChartKind::Pie3D);
    turned.first_slice_angle = 350;
    turned.view3d = Some(View3D {
        rot_y: 40,
        rot_x: 200,
        depth_percent: 5,
        perspective: 999,
        height_percent: Some(1),
        ..View3D::for_kind(ChartKind::Pie3D)
    });
    let (parts, read) = written(vec![flat, turned]);
    assert!(!parts[0].contains("view3D") && !parts[0].contains("<c:floor>"));
    assert_eq!(read[0].view3d, None);
    assert!(parts[1].contains("<c:view3D><c:rotX val=\"90\"/><c:hPercent val=\"5\"/><c:rotY val=\"30\"/><c:depthPercent val=\"20\"/><c:rAngAx val=\"0\"/><c:perspective val=\"240\"/></c:view3D>"));
    assert_eq!(read[1].first_slice_angle, 30);
    assert_eq!(
        read[1].view3d,
        Some(View3D {
            rot_x: 90,
            rot_y: 0,
            perspective: 240,
            right_angle_axes: false,
            depth_percent: 20,
            height_percent: Some(5),
        })
    );
}

#[test]
fn round_charts_and_bars() {
    let mut doughnut = plain(ChartKind::Doughnut);
    doughnut.hole_size = 35;
    doughnut.first_slice_angle = 270;
    doughnut.series[0].explosion = 12;
    let mut pie = plain(ChartKind::Pie);
    pie.first_slice_angle = 90;
    pie.series[0].explosion = 25;
    pie.series[0].line.shown = Some(false);
    let mut bars = plain(ChartKind::StackedBar);
    bars.gap_width = Some(0);
    bars.overlap = Some(-100);
    let mut bubbles = chart(ChartKind::Bubble, vec![measured('H')]);
    bubbles.bubble_scale = 250;
    let parts = assert_round_trip(vec![doughnut, pie, bars, bubbles]);
    assert!(parts[0].contains("</c:spPr><c:explosion val=\"12\"/><c:dPt>"));
    assert!(parts[0].contains(
        "</c:ser><c:firstSliceAng val=\"270\"/><c:holeSize val=\"35\"/></c:doughnutChart>"
    ));
    assert!(parts[1].contains("</c:tx><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:explosion val=\"25\"/><c:dPt><c:idx val=\"0\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill><a:ln><a:noFill/></a:ln></c:spPr></c:dPt>"));
    assert!(parts[1].contains("</c:ser><c:firstSliceAng val=\"90\"/></c:pieChart>"));
    assert!(parts[2].contains("<c:gapWidth val=\"0\"/><c:overlap val=\"-100\"/>"));
    // Bubbles without sizes are all of one size, which has to be said.
    assert!(parts[3].contains("</c:yVal><c:bubbleSize><c:numLit><c:formatCode>General</c:formatCode><c:ptCount val=\"6\"/><c:pt idx=\"0\"><c:v>1</c:v></c:pt><c:pt idx=\"1\"><c:v>1</c:v></c:pt><c:pt idx=\"2\"><c:v>1</c:v></c:pt><c:pt idx=\"3\"><c:v>1</c:v></c:pt><c:pt idx=\"4\"><c:v>1</c:v></c:pt><c:pt idx=\"5\"><c:v>1</c:v></c:pt></c:numLit></c:bubbleSize><c:bubble3D val=\"0\"/></c:ser><c:bubbleScale val=\"250\"/><c:showNegBubbles val=\"0\"/><c:axId val=\"50000041\"/><c:axId val=\"50000042\"/></c:bubbleChart>"), "{}", parts[3]);

    // What a chart has no use for stays what it is by itself.
    let mut columns = plain(ChartKind::Column);
    columns.hole_size = 20;
    columns.first_slice_angle = 45;
    columns.bubble_scale = 30;
    columns.series[0].explosion = 10;
    columns.series[0].points = vec![point(0, None, Some(40))];
    columns.data_table = Some(DataTable::default());
    let mut lines = plain(ChartKind::Line);
    lines.gap_width = Some(10);
    lines.overlap = Some(10);
    let mut pie = plain(ChartKind::Pie);
    pie.data_table = Some(DataTable::default());
    pie.hole_size = 500;
    pie.first_slice_angle = 1000;
    let (parts, read) = written(vec![columns, lines, pie]);
    let unused = Chart::new(ChartKind::Column);
    assert_eq!(
        (
            read[0].hole_size,
            read[0].first_slice_angle,
            read[0].bubble_scale
        ),
        (unused.hole_size, 0, unused.bubble_scale)
    );
    assert_eq!(read[0].series[0].explosion, 0);
    assert!(read[0].series[0].points.is_empty());
    assert_eq!(read[0].data_table, Some(DataTable::default()));
    assert!(parts[0].contains("</c:valAx><c:dTable><c:showHorzBorder val=\"1\"/><c:showVertBorder val=\"1\"/><c:showOutline val=\"1\"/><c:showKeys val=\"1\"/></c:dTable><c:spPr>"));
    assert_eq!((read[1].gap_width, read[1].overlap), (None, None));
    assert!(!parts[2].contains("dTable") && parts[2].contains("<c:firstSliceAng val=\"360\"/>"));
    assert_eq!(read[2].data_table, None);
}

#[test]
fn shadows_and_glows_of_the_areas() {
    use opensheet_engine::drawing::{Effects, Glow, ShadowPreset};
    let mut c = plain(ChartKind::Column);
    c.plot_area.fill = Fill::Solid(Color(0xF2F2F2));
    c.plot_area.effects = Effects {
        shadow: Some(ShadowPreset::BottomRight.shadow()),
        ..Effects::default()
    };
    c.chart_area.effects = Effects {
        glow: Some(Glow {
            color: Color(0x4472C4),
            transparency: 0.6,
            radius: 8.0,
        }),
        ..Effects::default()
    };
    let mut sample = data_workbook();
    sample.add_chart(1, c.clone()).unwrap();
    keep("chart-effects.xlsx", &to_bytes(&sample));
    let (parts, read) = written(vec![c.clone()]);
    assert_same_content("effects", &c, &read[0]);
    assert_eq!(read[0].plot_area.effects, c.plot_area.effects);
    assert_eq!(read[0].chart_area.effects, c.chart_area.effects);
    let xml = &parts[0];
    assert_schema_order("chart1.xml", xml);
    // As in a shape: after the line, the shadow Excel calls "Offset:
    // Bottom Right" and a glow of 8 points.
    assert!(
        xml.contains("</a:ln><a:effectLst><a:outerShdw blurRad=\"50800\" dist=\"38100\" dir=\"2700000\" algn=\"tl\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"40000\"/></a:srgbClr></a:outerShdw></a:effectLst></c:spPr></c:plotArea>"),
        "{xml}"
    );
    assert!(xml.contains("<a:effectLst><a:glow rad=\"101600\"><a:srgbClr val=\"4472C4\"><a:alpha val=\"40000\"/></a:srgbClr></a:glow></a:effectLst></c:spPr><c:txPr>"), "{xml}");
}

#[test]
fn title_legend_and_areas() {
    let mut c = plain(ChartKind::Column);
    c.title = ChartTitle::Auto;
    c.title_overlay = true;
    c.legend = LegendPos::TopRight;
    c.legend_overlay = true;
    c.legend_hidden = vec![2, 0];
    c.blanks = BlanksAs::Span;
    c.chart_area = ShapeFormat {
        fill: Fill::Solid(Color(0x102030)),
        border: Line {
            shown: None,
            color: Some(Color(0x405060)),
            width: Some(2.5),
            dash: Dash::LongDashDotDot,
        },
        ..ShapeFormat::default()
    };
    c.plot_area = ShapeFormat {
        fill: Fill::Solid(Color(0xFFFFFF)),
        border: Line {
            shown: Some(true),
            ..Line::default()
        },
        ..ShapeFormat::default()
    };
    let mut bare = plain(ChartKind::Line);
    bare.title = ChartTitle::None;
    bare.legend = LegendPos::None;
    bare.blanks = BlanksAs::Zero;
    bare.chart_area = ShapeFormat {
        fill: Fill::None,
        border: Line {
            shown: Some(false),
            ..Line::default()
        },
        ..ShapeFormat::default()
    };
    let parts = assert_round_trip(vec![c, bare]);
    let xml = &parts[0];
    assert!(xml.contains(
        "<c:chart><c:title><c:overlay val=\"1\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr></c:title><c:autoTitleDeleted val=\"0\"/>"
    ));
    assert!(xml.contains("</c:valAx><c:spPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"D9D9D9\"/></a:solidFill></a:ln></c:spPr></c:plotArea><c:legend><c:legendPos val=\"tr\"/><c:legendEntry><c:idx val=\"2\"/><c:delete val=\"1\"/></c:legendEntry><c:legendEntry><c:idx val=\"0\"/><c:delete val=\"1\"/></c:legendEntry><c:overlay val=\"1\"/></c:legend><c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"span\"/></c:chart><c:spPr><a:solidFill><a:srgbClr val=\"102030\"/></a:solidFill><a:ln w=\"31750\"><a:solidFill><a:srgbClr val=\"405060\"/></a:solidFill><a:prstDash val=\"lgDashDotDot\"/></a:ln></c:spPr><c:txPr>"), "{xml}");
    let xml = &parts[1];
    assert!(xml.contains("<c:chart><c:autoTitleDeleted val=\"1\"/><c:plotArea>"));
    assert!(xml.contains("</c:plotArea><c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"zero\"/></c:chart><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr><c:txPr>"));

    // Without a title or a legend, there is nothing to draw over the plot
    // or to leave out; what is automatic anyway is not kept as a choice.
    let mut c = plain(ChartKind::Column);
    c.title = ChartTitle::None;
    c.title_overlay = true;
    c.legend = LegendPos::None;
    c.legend_overlay = true;
    c.legend_hidden = vec![1, 1];
    c.chart_area = ShapeFormat {
        fill: Fill::Solid(Color(0xFFFFFF)),
        border: Line {
            shown: Some(true),
            color: Some(Color(0xD9D9D9)),
            width: Some(0.75),
            dash: Dash::Solid,
        },
        ..ShapeFormat::default()
    };
    c.plot_area = ShapeFormat {
        fill: Fill::None,
        border: Line {
            shown: Some(false),
            color: Some(Color(0x123456)),
            width: Some(3.0),
            dash: Dash::Dash,
        },
        ..ShapeFormat::default()
    };
    let (_, read) = written(vec![c]);
    let mut expected = plain(ChartKind::Column);
    expected.title = ChartTitle::None;
    expected.legend = LegendPos::None;
    assert_same_content("automatic", &expected, &read[0]);
}

// ---- charts as Excel 365 writes them ---------------------------------------------

/// The Office theme of 2023, as far as colours go: its accents are the
/// colours the engine gives series by itself.
const THEME_2023: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office">
<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="0E2841"/></a:dk2><a:lt2><a:srgbClr val="E8E8E8"/></a:lt2>
<a:accent1><a:srgbClr val="156082"/></a:accent1><a:accent2><a:srgbClr val="E97132"/></a:accent2>
<a:accent3><a:srgbClr val="196B24"/></a:accent3><a:accent4><a:srgbClr val="0F9ED5"/></a:accent4>
<a:accent5><a:srgbClr val="A02B93"/></a:accent5><a:accent6><a:srgbClr val="4EA72E"/></a:accent6>
<a:hlink><a:srgbClr val="467886"/></a:hlink><a:folHlink><a:srgbClr val="96607D"/></a:folHlink>
</a:clrScheme></a:themeElements></a:theme>"#;

/// "No fill, no line", which Excel says of about everything that has
/// neither.
const BLANK: &str = r#"<c:spPr><a:noFill/><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr>"#;

/// The chart of a part as Excel 365 wraps it: language, a style in two
/// dialects, the chart area, print settings.
fn excel_part(chart: &str) -> String {
    format!(
        r#"{DECL}<c:chartSpace {NS_C} xmlns:c16r2="http://schemas.microsoft.com/office/drawing/2015/06/chart"><c:date1904 val="0"/><c:lang val="fr-FR"/><c:roundedCorners val="0"/><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="c14" xmlns:c14="http://schemas.microsoft.com/office/drawing/2007/8/2/chart"><c14:style val="102"/></mc:Choice><mc:Fallback><c:style val="2"/></mc:Fallback></mc:AlternateContent><c:chart><c:title><c:overlay val="0"/>{BLANK}{title_text}</c:title><c:autoTitleDeleted val="0"/>{chart}<c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/><c:extLst><c:ext uri="{{56B9EC1D-385E-4148-901F-78D8002777C0}}" xmlns:c16r3="http://schemas.microsoft.com/office/drawing/2017/03/chart"><c16r3:dataDisplayOptions16><c16r3:dispNaAsBlank val="1"/></c16r3:dataDisplayOptions16></c:ext></c:extLst></c:chart><c:spPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill><a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill><a:round/></a:ln><a:effectLst/></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="fr-FR"/></a:p></c:txPr><c:printSettings><c:headerFooter/><c:pageMargins b="0.75" l="0.7" r="0.7" t="0.75" header="0.3" footer="0.3"/><c:pageSetup/></c:printSettings></c:chartSpace>"#,
        // The title Excel makes up is written in 14 points.
        title_text = TEXT.replace("sz=\"900\"", "sz=\"1400\""),
    )
}

/// A reference with the values Excel keeps next to it.
fn excel_ref(kind: &str, formula: &str, values: &[&str]) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("<c:pt idx=\"{i}\"><c:v>{v}</c:v></c:pt>"))
        .collect();
    let format = if kind == "num" {
        "<c:formatCode>General</c:formatCode>"
    } else {
        ""
    };
    format!(
        "<c:{kind}Ref><c:f>{formula}</c:f><c:{kind}Cache>{format}<c:ptCount val=\"{}\"/>{points}</c:{kind}Cache></c:{kind}Ref>",
        values.len()
    )
}

/// Name, categories (or X values) and values of a series of the sheet of
/// the foreign package: column C or D.
fn excel_data(column: char, xy: bool) -> (String, String) {
    let (name, values) = if column == 'C' {
        ("North", ["10", "20", "30"])
    } else {
        ("South", ["7", "9", "4"])
    };
    let tx = format!(
        "<c:tx>{}</c:tx>",
        excel_ref("str", &format!("Data!${column}$1"), &[name])
    );
    let data = if xy {
        format!(
            "<c:xVal>{}</c:xVal><c:yVal>{}</c:yVal>",
            excel_ref("num", "Data!$C$2:$C$4", &["10", "20", "30"]),
            excel_ref("num", &format!("Data!${column}$2:${column}$4"), &values)
        )
    } else {
        format!(
            "<c:cat>{}</c:cat><c:val>{}</c:val>",
            excel_ref("str", "Data!$B$2:$B$4", &["Q1", "Q2", "Q3"]),
            excel_ref("num", &format!("Data!${column}$2:${column}$4"), &values)
        )
    };
    (tx, data)
}

/// The labels Excel says a group does not have.
const NO_LABELS: &str = r#"<c:dLbls><c:showLegendKey val="0"/><c:showVal val="0"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="0"/><c:showBubbleSize val="0"/></c:dLbls>"#;

/// An axis with the formatting of Excel's charts: the labels of
/// categories (`cat`), a scale with gridlines (`val`) or without
/// (`second`), a scale of an X/Y chart (`xy`); its side, its id, the axis
/// it crosses and where.
fn excel_axis(kind: &str, side: char, (id, cross, crosses): (u32, u32, &str)) -> String {
    let head = format!(
        "<c:axId val=\"{id}\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{side}\"/>"
    );
    let ticks = "<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/>";
    let tail = format!("<c:crossAx val=\"{cross}\"/><c:crosses val=\"{crosses}\"/>");
    let lines = format!("<c:majorGridlines>{SHAPE}</c:majorGridlines>");
    match kind {
        "cat" => format!(
            "<c:catAx>{head}{ticks}{SHAPE}{TEXT}{tail}<c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/></c:catAx>"
        ),
        "val" => format!(
            "<c:valAx>{head}{lines}{ticks}{BLANK}{TEXT}{tail}<c:crossBetween val=\"between\"/></c:valAx>"
        ),
        "second" => format!(
            "<c:valAx>{head}{ticks}{BLANK}{TEXT}{tail}<c:crossBetween val=\"between\"/></c:valAx>"
        ),
        _ => format!(
            "<c:valAx>{head}{lines}{ticks}{SHAPE}{TEXT}{tail}<c:crossBetween val=\"midCat\"/></c:valAx>"
        ),
    }
}

const LEGEND: &str = r#"<c:legend><c:legendPos val="b"/><c:overlay val="0"/><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr></c:legend>"#;

/// The chart of a part of the foreign package, in a workbook whose theme
/// is the one of the engine's colours.
fn excel(chart_part: &str) -> Chart {
    assert_well_formed("chart1.xml", chart_part);
    let bytes = foreign_package(&[
        ("xl/charts/chart1.xml", chart_part),
        ("xl/theme/theme1.xml", THEME_2023),
    ]);
    load(&bytes).sheets[0].charts[0].clone()
}

fn data_series(column: char, xy: bool) -> Series {
    let categories = if xy {
        "Data!$C$2:$C$4"
    } else {
        "Data!$B$2:$B$4"
    };
    series(
        named(&format!("Data!${column}$1")),
        &format!("Data!${column}$2:${column}$4"),
        Some(categories),
    )
}

#[test]
fn a_line_with_markers_from_excel() {
    let line = |index: u32, column: char| {
        let (tx, data) = excel_data(column, false);
        let accent = format!(
            "<a:solidFill><a:schemeClr val=\"accent{}\"/></a:solidFill>",
            index + 1
        );
        format!(
            "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>{tx}<c:spPr><a:ln w=\"28575\" cap=\"rnd\">{accent}<a:round/></a:ln><a:effectLst/></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/><c:spPr>{accent}<a:ln w=\"9525\">{accent}</a:ln><a:effectLst/></c:spPr></c:marker>{data}<c:smooth val=\"0\"/>{}</c:ser>",
            unique_id(index)
        )
    };
    let xml = excel_part(&format!(
        "<c:plotArea><c:layout/><c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{}{}{NO_LABELS}<c:marker val=\"1\"/><c:smooth val=\"0\"/><c:axId val=\"11\"/><c:axId val=\"12\"/></c:lineChart>{}{}{BLANK}</c:plotArea>{LEGEND}",
        line(0, 'C'),
        line(1, 'D'),
        excel_axis("cat", 'b', (11, 12, "autoZero")),
        excel_axis("val", 'l', (12, 11, "autoZero")),
    ));
    // Everything Excel spells out here is what it does by itself: the
    // chart is one of lines with markers, and nothing else is said.
    let mut expected = chart(
        ChartKind::LineMarkers,
        vec![data_series('C', false), data_series('D', false)],
    );
    expected.legend = LegendPos::Bottom;
    let read = excel(&xml);
    assert_same_content("line with markers", &expected, &read);
    assert_eq!(read.series[0].marker, None);
    assert_eq!(read.series[0].line, Line::default());
    assert_eq!(read.series[1].color, None, "the second accent of the theme");
    assert_eq!(read.axes, Axes::for_kind(ChartKind::LineMarkers));
    assert_eq!(read.chart_area, ShapeFormat::default());
    assert_eq!(read.plot_area, ShapeFormat::default());
}

#[test]
fn columns_and_a_line_on_secondary_axes_from_excel() {
    let column = |index: u32, column: char| {
        let (tx, data) = excel_data(column, false);
        format!(
            "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>{tx}<c:spPr><a:solidFill><a:schemeClr val=\"accent{}\"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val=\"0\"/>{data}{}</c:ser>",
            index + 1,
            unique_id(index)
        )
    };
    let line = format!(
        "<c:ser><c:idx val=\"2\"/><c:order val=\"2\"/><c:tx>{}</c:tx><c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent3\"/></a:solidFill><a:round/></a:ln><a:effectLst/></c:spPr><c:marker><c:symbol val=\"none\"/></c:marker><c:cat>{}</c:cat><c:val>{}</c:val><c:smooth val=\"0\"/>{}</c:ser>",
        excel_ref("str", "'Other Sheet'!$A$1", &["Elsewhere"]),
        excel_ref("str", "Data!$B$2:$B$4", &["Q1", "Q2", "Q3"]),
        excel_ref("num", "'Other Sheet'!$B$2:$B$3", &["5", "6"]),
        unique_id(2)
    );
    // The second pair of axes as Excel has it: the scale at the far end,
    // the category axis deleted (and, as deleted axes are in its files,
    // with tick marks nobody sees).
    let hidden = excel_axis("cat", 'b', (13, 14, "autoZero"))
        .replace("<c:delete val=\"0\"/>", "<c:delete val=\"1\"/>")
        .replace(
            "<c:majorTickMark val=\"none\"/>",
            "<c:majorTickMark val=\"out\"/>",
        );
    let xml = excel_part(&format!(
        "<c:plotArea><c:layout/><c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>{}{}{NO_LABELS}<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/><c:axId val=\"11\"/><c:axId val=\"12\"/></c:barChart><c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{line}{NO_LABELS}<c:marker val=\"1\"/><c:smooth val=\"0\"/><c:axId val=\"13\"/><c:axId val=\"14\"/></c:lineChart>{}{}{}{hidden}{BLANK}</c:plotArea>{LEGEND}",
        column(0, 'C'),
        column(1, 'D'),
        excel_axis("cat", 'b', (11, 12, "autoZero")),
        excel_axis("val", 'l', (12, 11, "autoZero")),
        excel_axis("second", 'r', (14, 13, "max")),
    ));
    let mut expected = chart(
        ChartKind::Column,
        vec![
            data_series('C', false),
            data_series('D', false),
            Series {
                kind: Some(ChartKind::Line),
                secondary: true,
                ..series(
                    named("'Other Sheet'!$A$1"),
                    "'Other Sheet'!$B$2:$B$3",
                    Some("Data!$B$2:$B$4"),
                )
            },
        ],
    );
    expected.legend = LegendPos::Bottom;
    expected.axes.secondary_category.major_ticks = TickMark::Outside;
    let read = excel(&xml);
    assert_same_content("combination", &expected, &read);
    assert!(read.has_secondary_axis());
    assert!(!read.axes.secondary_category.visible && read.axes.secondary_value.visible);
    assert_eq!(read.axes.secondary_category.crossing, AxisCrossing::Auto);
    assert_eq!((read.gap_width, read.overlap), (None, None));
    // Changed, it is written with the same groups and the same axes.
    let mut wb = load(&foreign_package(&[
        ("xl/charts/chart1.xml", &xml),
        ("xl/theme/theme1.xml", THEME_2023),
    ]));
    wb.sheets[0].charts[0].title = ChartTitle::Text("Changed".into());
    let saved = to_bytes(&wb);
    let written = part(&saved, "xl/charts/chart1.xml");
    assert_schema_order("chart1.xml", &written);
    assert_eq!(
        axes_in(&written),
        [
            ("c:catAx", "50000011"),
            ("c:valAx", "50000012"),
            ("c:valAx", "50000013"),
            ("c:catAx", "50000014")
        ]
    );
    assert!(
        written.contains("<c:crosses val=\"max\"/>") && written.contains("<c:delete val=\"1\"/>")
    );
    expected.title = ChartTitle::Text("Changed".into());
    assert_same_content(
        "combination, again",
        &expected,
        &load(&saved).sheets[0].charts[0],
    );
}

#[test]
fn a_scatter_chart_with_smooth_lines_from_excel() {
    let (tx, data) = excel_data('D', true);
    let xml = excel_part(&format!(
        "<c:plotArea><c:layout/><c:scatterChart><c:scatterStyle val=\"smoothMarker\"/><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/>{tx}<c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill><a:round/></a:ln><a:effectLst/></c:spPr><c:marker><c:symbol val=\"none\"/></c:marker>{data}<c:smooth val=\"1\"/>{}</c:ser>{NO_LABELS}<c:axId val=\"11\"/><c:axId val=\"12\"/></c:scatterChart>{}{}{BLANK}</c:plotArea>",
        unique_id(0),
        excel_axis("xy", 'b', (11, 12, "autoZero")),
        excel_axis("xy", 'l', (12, 11, "autoZero")),
    ));
    let expected = chart(ChartKind::ScatterSmooth, vec![data_series('D', true)]);
    let read = excel(&xml);
    assert_same_content("smooth lines", &expected, &read);
    assert_eq!(
        (read.series[0].smooth, &read.series[0].marker),
        (None, &None)
    );
    assert_eq!(read.series[0].line, Line::default());
    assert_eq!(read.legend, LegendPos::None);

    // The same chart with markers, then with straight lines, then with a
    // wider line of dashes in a colour of its own.
    for (from, to, kind) in [
        (
            "<c:symbol val=\"none\"/>",
            "<c:symbol val=\"circle\"/><c:size val=\"5\"/>",
            ChartKind::ScatterSmoothMarkers,
        ),
        (
            "<c:smooth val=\"1\"/>",
            "<c:smooth val=\"0\"/>",
            ChartKind::ScatterLines,
        ),
        (
            "<a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill>",
            "<a:ln w=\"19050\" cap=\"rnd\"><a:noFill/>",
            ChartKind::Scatter,
        ),
    ] {
        assert!(xml.contains(from));
        let read = excel(&xml.replacen(from, to, 1));
        assert_eq!(read.kind, kind);
        // Points without a line and without markers still are points, and
        // the line they do not have still is a curve.
        let points = kind == ChartKind::Scatter;
        let invisible = points.then(|| Marker::new(MarkerShape::None));
        assert_eq!(read.series[0].marker, invisible, "{kind:?}");
        assert_eq!(read.series[0].smooth, points.then_some(true), "{kind:?}");
        assert_eq!(read.series[0].line, Line::default(), "{kind:?}");
    }
    let styled = xml.replacen(
        "<a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill>",
        "<a:ln w=\"38100\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill><a:prstDash val=\"lgDash\"/>",
        1,
    );
    let read = excel(&styled);
    assert_eq!(read.kind, ChartKind::ScatterSmooth);
    assert_eq!(read.series[0].color, Some(Color(0xC00000)));
    assert_eq!(
        read.series[0].line,
        Line {
            width: Some(3.0),
            dash: Dash::LongDash,
            ..Line::default()
        }
    );
}

#[test]
fn a_trendline_from_excel() {
    let (tx, data) = excel_data('C', false);
    let label = format!(
        "<c:trendlineLbl><c:layout/><c:numFmt formatCode=\"General\" sourceLinked=\"0\"/>{BLANK}{TEXT}</c:trendlineLbl>"
    );
    let labels = format!(
        "<c:dLbls>{BLANK}{TEXT}<c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/><c:showLeaderLines val=\"0\"/><c:extLst><c:ext uri=\"{{CE6537A1-D6FC-4f65-9D91-7224C49458BB}}\" xmlns:c15=\"http://schemas.microsoft.com/office/drawing/2012/chart\"><c15:showLeaderLines val=\"1\"/></c:ext></c:extLst></c:dLbls>"
    );
    // Error bars whose amounts are read from cells are not in the model.
    let custom = format!(
        "<c:errBars><c:errBarType val=\"both\"/><c:errValType val=\"cust\"/><c:noEndCap val=\"0\"/><c:plus>{0}</c:plus><c:minus>{0}</c:minus></c:errBars>",
        excel_ref("num", "Data!$D$2:$D$4", &["7", "9", "4"])
    );
    let xml = excel_part(&format!(
        "<c:plotArea><c:layout/><c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/>{tx}<c:spPr><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val=\"0\"/>{labels}<c:trendline><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill><a:prstDash val=\"sysDot\"/></a:ln><a:effectLst/></c:spPr><c:trendlineType val=\"linear\"/><c:forward val=\"2\"/><c:dispRSqr val=\"1\"/><c:dispEq val=\"1\"/>{label}</c:trendline><c:trendline><c:name>Moyenne</c:name><c:spPr><a:ln w=\"25400\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:effectLst/></c:spPr><c:trendlineType val=\"movingAvg\"/><c:period val=\"2\"/><c:dispRSqr val=\"0\"/><c:dispEq val=\"0\"/></c:trendline>{custom}{data}{}</c:ser>{NO_LABELS}<c:gapWidth val=\"100\"/><c:overlap val=\"-27\"/><c:axId val=\"11\"/><c:axId val=\"12\"/></c:barChart>{}{}{BLANK}</c:plotArea>{}",
        unique_id(0),
        excel_axis("cat", 'b', (11, 12, "autoZero")),
        excel_axis("val", 'l', (12, 11, "autoZero")),
        LEGEND.replace(
            "<c:overlay",
            "<c:legendEntry><c:idx val=\"2\"/><c:delete val=\"1\"/></c:legendEntry><c:overlay"
        ),
    ));
    let mut expected = chart(ChartKind::Column, vec![data_series('C', false)]);
    expected.series[0].trendlines = vec![
        Trendline {
            forward: 2.0,
            show_equation: true,
            show_r2: true,
            ..Trendline::new(TrendKind::Linear)
        },
        Trendline {
            name: Some("Moyenne".into()),
            line: Line {
                color: Some(Color(0xFF0000)),
                width: Some(2.0),
                ..Line::default()
            },
            ..Trendline::new(TrendKind::MovingAverage(2))
        },
    ];
    // The labels of the only series are the chart's.
    expected.data_labels = true;
    expected.legend = LegendPos::Bottom;
    expected.legend_hidden = vec![2];
    expected.gap_width = Some(100);
    let read = excel(&xml);
    assert_same_content("trendline", &expected, &read);
    assert!(read.series[0].error_bars.is_empty());
}

// ---- what a file leaves to Excel -------------------------------------------------

/// A chart part with nothing but plots.
fn bare_part(plots: &str) -> String {
    format!(
        "{DECL}<c:chartSpace {NS_C}><c:chart><c:plotArea>{plots}</c:plotArea></c:chart></c:chartSpace>"
    )
}

/// A series of such a part: its number, and what it says of its looks.
fn bare_series(index: u32, looks: &str) -> String {
    format!(
        "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>{looks}<c:val><c:numRef><c:f>Data!$C$2:$C$4</c:f></c:numRef></c:val></c:ser>"
    )
}

#[test]
fn markers_left_to_excel_are_its_old_ones() {
    // Series that say nothing of their markers, in a plot that has them:
    // a shape per series, by its number, larger than those Excel draws now.
    let series: String = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 20]
        .into_iter()
        .map(|index| bare_series(index, ""))
        .collect();
    let xml = bare_part(&format!(
        "<c:lineChart><c:grouping val=\"standard\"/>{series}{}{}<c:marker val=\"1\"/></c:lineChart>",
        bare_series(
            21,
            "<c:marker><c:symbol val=\"auto\"/><c:size val=\"9\"/></c:marker>"
        ),
        bare_series(22, "<c:marker><c:symbol val=\"picture\"/></c:marker>"),
    ));
    let read = excel(&xml);
    assert_eq!(read.kind, ChartKind::LineMarkers);
    use MarkerShape as Shape;
    let expected = [
        (Shape::Diamond, 7),
        (Shape::Square, 7),
        (Shape::Triangle, 7),
        (Shape::X, 7),
        (Shape::Star, 7),
        (Shape::Circle, 7),
        (Shape::Plus, 7),
        (Shape::Dot, 7),
        (Shape::Dash, 7),
        (Shape::Diamond, 7),
        (Shape::Triangle, 7),
        (Shape::X, 9),
    ];
    for (series, (shape, size)) in read.series.iter().zip(expected) {
        let marker = Marker {
            size,
            ..Marker::new(shape)
        };
        assert_eq!(series.marker, Some(marker));
        assert_eq!((series.color, &series.line), (None, &Line::default()));
    }
    // A picture is drawn as the engine's own marker, which is the one of
    // the kind.
    assert_eq!(read.series[12].marker, None);

    // Without markers in the plot, there are none; a radar chart has them
    // by its style.
    let plain = bare_part(&format!(
        "<c:lineChart>{}{}</c:lineChart>",
        bare_series(0, ""),
        bare_series(1, "<c:marker><c:symbol val=\"auto\"/></c:marker>")
    ));
    let read = excel(&plain);
    assert_eq!(read.kind, ChartKind::Line);
    assert!(read.series.iter().all(|s| s.marker.is_none()));
    let radar = bare_part(&format!(
        "<c:radarChart><c:radarStyle val=\"marker\"/>{}{}</c:radarChart>",
        bare_series(0, "<c:marker><c:symbol val=\"none\"/></c:marker>"),
        bare_series(4, "")
    ));
    let read = excel(&radar);
    assert_eq!(read.kind, ChartKind::Radar);
    assert_eq!(read.series[0].marker, None);
    assert_eq!(
        read.series[1].marker,
        Some(Marker {
            size: 7,
            ..Marker::new(Shape::Star)
        })
    );
}

#[test]
fn the_colour_of_a_series_is_that_of_its_ink() {
    let fill = |rgb: &str| format!("<a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill>");
    let looks = |line: &str, inside: &str, around: &str| {
        let part = |rgb: &str| match rgb {
            "" => String::new(),
            "none" => "<a:noFill/>".to_string(),
            rgb => fill(rgb),
        };
        format!(
            "<c:spPr><a:ln w=\"28575\">{}</a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/><c:spPr>{}<a:ln>{}</a:ln></c:spPr></c:marker>",
            part(line),
            part(inside),
            part(around)
        )
    };
    let (red, green, blue) = (Color(0xFF0000), Color(0x00FF00), Color(0x0000FF));
    // (line, inside of the markers, their outline) as the file has them;
    // then the colour of the series, of its line, of the inside and of the
    // outline of its markers where they have another one.
    let cases = [
        (
            ("FF0000", "FF0000", "FF0000"),
            (Some(red), None, None, None),
        ),
        // The line alone is of another colour: the markers keep the series'.
        (
            ("FF0000", "0000FF", "0000FF"),
            (Some(blue), Some(red), None, None),
        ),
        // The inside of the markers alone: hollow markers on a line.
        (
            ("FF0000", "00FF00", "FF0000"),
            (Some(red), None, Some(green), None),
        ),
        (
            ("FF0000", "FF0000", "0000FF"),
            (Some(red), None, None, Some(blue)),
        ),
        (
            ("FF0000", "00FF00", "0000FF"),
            (Some(red), None, Some(green), Some(blue)),
        ),
        // What is left to Excel has the automatic colour (the first of the
        // palette here), and so has the series if that is its ink.
        (("", "", ""), (None, None, None, None)),
        (("FF0000", "", ""), (None, Some(red), None, None)),
        (("", "00FF00", ""), (None, None, Some(green), None)),
        (("", "", "0000FF"), (None, None, None, Some(blue))),
        // No line: the markers are the series. Markers that are an outline
        // or a fill only are all of that colour.
        (
            ("none", "00FF00", "0000FF"),
            (Some(green), None, None, Some(blue)),
        ),
        (("none", "none", "0000FF"), (Some(blue), None, None, None)),
        (("none", "00FF00", "none"), (Some(green), None, None, None)),
    ];
    for ((line, inside, around), (color, own_line, own_fill, own_outline)) in cases {
        let xml = bare_part(&format!(
            "<c:lineChart>{}<c:marker val=\"1\"/></c:lineChart>",
            bare_series(0, &looks(line, inside, around))
        ));
        let read = excel(&xml);
        let s = &read.series[0];
        let which = format!("{line:?} {inside:?} {around:?}");
        assert_eq!(read.kind, ChartKind::LineMarkers, "{which}");
        assert_eq!(s.color, color, "{which}");
        assert_eq!(s.line.color, own_line, "{which}");
        assert_eq!(s.line.shown, (line == "none").then_some(false), "{which}");
        let marker = (own_fill.is_some() || own_outline.is_some()).then(|| Marker {
            fill: own_fill,
            line: own_outline,
            ..Marker::new(MarkerShape::Circle)
        });
        assert_eq!(s.marker, marker, "{which}");
        // And the chart says the same when written from the model.
        let mut wb = load(&foreign_package(&[("xl/charts/chart1.xml", &xml)]));
        wb.sheets[0].charts[0].foreign = None;
        assert_same_content(&which, &read, &reload(&wb).sheets[0].charts[0]);
    }
}

#[test]
fn lines_widths_and_what_bars_do_by_themselves() {
    // Every dash of the format; widths are in points.
    let dashes = [
        ("solid", Dash::Solid),
        ("sysDot", Dash::RoundDot),
        ("sysDash", Dash::SquareDot),
        ("dash", Dash::Dash),
        ("dashDot", Dash::DashDot),
        ("lgDash", Dash::LongDash),
        ("lgDashDot", Dash::LongDashDot),
        ("lgDashDotDot", Dash::LongDashDotDot),
        // Those Excel does not offer: the nearest.
        ("dot", Dash::RoundDot),
        ("sysDashDot", Dash::DashDot),
        ("sysDashDotDot", Dash::LongDashDotDot),
    ];
    let series: String = dashes
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let width = if i % 2 == 0 { 28575 } else { 12700 };
            bare_series(
                i as u32,
                &format!(
                    "<c:spPr><a:ln w=\"{width}\"><a:prstDash val=\"{name}\"/></a:ln></c:spPr>"
                ),
            )
        })
        .collect();
    let read = excel(&bare_part(&format!("<c:lineChart>{series}</c:lineChart>")));
    for (i, (series, (name, dash))) in read.series.iter().zip(dashes).enumerate() {
        // The width Excel gives the lines of such a chart is no width of
        // their own.
        let width = (i % 2 == 1).then_some(1.0);
        assert_eq!(
            series.line,
            Line {
                width,
                dash,
                ..Line::default()
            },
            "{name}"
        );
    }

    // Gaps and overlaps: Excel's own for the type are automatic; bars that
    // say nothing have a gap of 150 and no overlap.
    let bars = |head: &str, tail: &str| {
        let xml = bare_part(&format!(
            "<c:barChart>{head}{}{tail}</c:barChart>",
            bare_series(0, "")
        ));
        let read = excel(&xml);
        (read.kind, read.gap_width, read.overlap, read.vary_colors)
    };
    let column = "<c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>";
    let stacked = "<c:barDir val=\"bar\"/><c:grouping val=\"stacked\"/>";
    use ChartKind::{Column, StackedBar};
    assert_eq!(
        bars(column, "<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/>"),
        (Column, None, None, None)
    );
    assert_eq!(bars(column, ""), (Column, Some(150), Some(0), None));
    assert_eq!(
        bars(column, "<c:gapWidth val=\"50%\"/><c:overlap val=\"100\"/>"),
        (Column, Some(50), Some(100), None)
    );
    assert_eq!(
        bars(stacked, "<c:gapWidth val=\"150\"/><c:overlap val=\"100\"/>"),
        (StackedBar, None, None, None)
    );
    assert_eq!(
        bars(
            stacked,
            "<c:gapWidth val=\"9999\"/><c:overlap val=\"-500\"/>"
        ),
        (StackedBar, Some(500), Some(-100), None)
    );
    assert_eq!(
        bars(&format!("{column}<c:varyColors val=\"1\"/>"), "").3,
        Some(true)
    );
    assert_eq!(
        bars(&format!("{column}<c:varyColors val=\"0\"/>"), "").3,
        None
    );
    let pie = |vary: &str| {
        let xml = bare_part(&format!(
            "<c:pieChart>{vary}{}</c:pieChart>",
            bare_series(0, "")
        ));
        excel(&xml).vary_colors
    };
    assert_eq!(pie("<c:varyColors val=\"1\"/>"), None);
    assert_eq!(pie(""), None);
    assert_eq!(pie("<c:varyColors val=\"0\"/>"), Some(false));
}

// ---- charts nobody changed -------------------------------------------------------

#[test]
fn untouched_charts_are_written_back_as_they_were() {
    // In a workbook with the theme packages are written with: theme
    // colours can stay what they are.
    let bytes = foreign_package(&[("xl/theme/theme1.xml", THEME_2023)]);
    let original = |n: usize| part(&bytes, &format!("xl/charts/chart{n}.xml"));
    let saved = |wb: &Workbook| -> Vec<String> {
        let bytes = to_bytes(wb);
        (1..=4)
            .map(|n| part(&bytes, &format!("xl/charts/chart{n}.xml")))
            .collect()
    };
    let wb = load(&bytes);
    let charts = saved(&wb);
    // Excel's columns, the combination and the pie come back to the byte,
    // with all the model has no place for: formatting, extensions, a
    // trendline's colour, the labels of single points.
    for n in 1..=3 {
        assert_eq!(charts[n - 1], original(n), "chart {n}");
    }
    assert!(charts[0].contains("c16r3:dispNaAsBlank") && charts[0].contains("<c:printSettings>"));
    // The parts they came with (styles, colours) are not carried over,
    // and nothing refers to them.
    let names = part_names(&to_bytes(&wb));
    assert!(names.contains(&"xl/charts/chart4.xml".to_string()));
    assert!(
        !names
            .iter()
            .any(|n| n.contains("style") && n.contains("charts")
                || n.contains("colors")
                || n.contains("charts/_rels")
                || n.contains("chartEx"))
    );
    // The scatter chart reads another workbook: it is written from the
    // model, without the series nothing can be read for.
    assert_ne!(charts[3], original(4));
    assert!(original(4).contains("[1]Sheet1") && !charts[3].contains("[1]"));
    assert!(charts[3].starts_with(&format!("{DECL}<c:chartSpace xmlns:c=")));
    assert_eq!(charts[3].matches("<c:ser>").count(), 2);
    assert_schema_order("chart4.xml", &charts[3]);

    // Read again, written again: nothing moves, and the charts are the same.
    let again = load(&to_bytes(&wb));
    assert_same_charts(&wb, &again);
    assert_eq!(saved(&again), charts);

    // Moving and resizing a chart is not changing it.
    let mut moved = load(&bytes);
    for chart in &mut moved.sheets[0].charts {
        chart.anchor = (40, 2);
        chart.offset = (3.0, 4.0);
        chart.size = (640.0, 400.0);
    }
    assert_eq!(saved(&moved), charts);
    assert!(part(&to_bytes(&moved), "xl/drawings/drawing1.xml").contains("<xdr:row>40</xdr:row>"));

    // A changed chart is written from the model; the others stay.
    let mut changed = load(&bytes);
    changed.sheets[0].charts[0].title = ChartTitle::Text("Another title".into());
    let after = saved(&changed);
    assert!(after[0].contains("<a:t>Another title</a:t>") && !after[0].contains("c16r3"));
    assert!(after[0].starts_with(&format!("{DECL}<c:chartSpace xmlns:c=")));
    assert_schema_order("chart1.xml", &after[0]);
    assert_eq!(after[1..], charts[1..]);
    // What the model holds of it is still there: the trendline in its
    // colour, the red point, the theme's colours.
    let reread = load(&to_bytes(&changed));
    let mut expected = wb.sheets[0].charts[0].clone();
    expected.title = ChartTitle::Text("Another title".into());
    assert_same_content("changed", &expected, &reread.sheets[0].charts[0]);
    // Changed back, it is the chart of the file again.
    changed.sheets[0].charts[0].title = wb.sheets[0].charts[0].title.clone();
    assert_eq!(saved(&changed), charts);

    // A chart that reads a sheet that is gone cannot be kept as it was:
    // Excel takes a file naming a sheet it does not have for damaged.
    let mut fewer = load(&bytes);
    assert!(fewer.delete_sheet(1));
    let after = saved(&fewer);
    assert_eq!(after[0], charts[0]);
    assert_eq!(after[2], charts[2]);
    assert_ne!(after[1], charts[1]);
    assert!(after[1].contains("<c:f>#REF!</c:f>") && !after[1].contains("Other Sheet"));
    assert_schema_order("chart2.xml", &after[1]);
}

#[test]
fn charts_that_refer_to_other_parts() {
    let columns = excel_columns();
    let saved = |chart: &str| {
        let wb = load(&foreign_package(&[
            ("xl/charts/chart1.xml", chart),
            ("xl/theme/theme1.xml", THEME_2023),
        ]));
        part(&to_bytes(&wb), "xl/charts/chart1.xml")
    };
    assert_eq!(saved(&columns), columns);
    // The workbook a chart of a document was made from, drawings on top
    // of the chart: left out, the rest is kept as it was.
    let embedded = columns
        .replace(
            "<c:printSettings>",
            "<c:externalData r:id=\"rId3\"><c:autoUpdate val=\"0\"/></c:externalData><c:printSettings>",
        )
        .replace("</c:chartSpace>", "<c:userShapes r:id=\"rId4\"/></c:chartSpace>");
    assert_ne!(embedded, columns);
    assert_eq!(saved(&embedded), columns);
    // A picture as the fill of a series, or of the chart area: the picture
    // is not in the package being written, so the chart is written from
    // the model.
    let blip =
        "<a:blipFill><a:blip r:embed=\"rId5\"/><a:stretch><a:fillRect/></a:stretch></a:blipFill>";
    let pictured = columns.replacen(
        "<c:spPr><a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill>",
        &format!("<c:spPr>{blip}"),
        1,
    );
    assert_ne!(pictured, columns);
    for chart in [
        pictured,
        columns.replace("<c:pageSetup/>", "<c:pageSetup/><c:legacyDrawingHF r:id=\"rId6\"/>"),
        // A chart of a pivot table, which is not in the model either.
        columns.replace(
            "<c:chart><c:title>",
            "<c:pivotSource><c:name>[Book.xlsx]Data!Pivot</c:name><c:fmtId val=\"0\"/></c:pivotSource><c:chart><c:title>",
        ),
    ] {
        assert_well_formed("chart1.xml", &chart);
        let written = saved(&chart);
        assert!(written.starts_with(&format!("{DECL}<c:chartSpace xmlns:c=")), "{written}");
        for lost in ["r:embed", "r:id", "pivotSource", "c16r3"] {
            assert!(!written.contains(lost), "{lost}");
        }
        assert_schema_order("chart1.xml", &written);
        assert_eq!(written.matches("<c:ser>").count(), 2);
    }
}

#[test]
fn untouched_charts_keep_the_colours_of_their_theme() {
    // The package is written with the theme the workbook came with: a
    // chart nobody touched keeps its part to the letter, theme colours
    // included, and they stand for the same colours.
    let columns = excel_columns();
    let saved = |theme: &str, name: &str| {
        let wb = load(&foreign_package(&[("xl/theme/theme1.xml", theme)]));
        let bytes = to_bytes(&wb);
        keep(name, &bytes);
        (part(&bytes, "xl/charts/chart1.xml"), load(&bytes))
    };
    let (kept, _) = saved(THEME_2023, "foreign-charts-saved-2023.xlsx");
    assert_eq!(kept, columns);
    let (kept, reread) = saved(THEME, "foreign-charts-saved.xlsx");
    assert_eq!(kept, columns);
    assert!(columns.contains("schemeClr"));
    assert_eq!(
        reread
            .theme()
            .rgb(opensheet_engine::style::ThemeSlot::Accent1),
        0x4472C4
    );
    let original = load(&foreign_package(&[("xl/theme/theme1.xml", THEME)]));
    assert_same_content(
        "columns",
        &original.sheets[0].charts[0],
        &reread.sheets[0].charts[0],
    );
    // The first accent of that theme is the automatic colour of the
    // first series.
    let chart = &reread.sheets[0].charts[0];
    assert_eq!(chart.series[0].color, None);
    assert_eq!(reread.chart_data(0, chart).series[0].color, Color(0x4472C4));
    assert_eq!(part(&to_bytes(&reread), "xl/charts/chart1.xml"), columns);
}

// ---- manual layouts, fonts, gridline formats -------------------------------------

/// Charts whose elements were dragged, resized and given fonts and lines
/// of their own.
fn laid_out() -> Vec<Chart> {
    let font = |name: &str, size: f32| TextFormat {
        font: Some(name.into()),
        size: Some(size),
        ..TextFormat::default()
    };
    let mut column = chart(ChartKind::Column, vec![monthly('B'), monthly('C')]);
    column.title = ChartTitle::Text("Dragged & formatted".into());
    column.title_layout = Some(ManualLayout::at(0.05, 0.03));
    column.title_text = TextFormat {
        font: Some("Georgia".into()),
        size: Some(20.0),
        bold: Some(true),
        italic: Some(true),
        underline: Some(true),
        color: Some(Color(0xC00000)),
    };
    column.legend = LegendPos::Right;
    column.legend_layout = Some(ManualLayout {
        x: 0.62,
        y: 0.08,
        w: Some(0.33),
        h: Some(0.125),
    });
    column.legend_text = TextFormat {
        italic: Some(true),
        color: Some(Color(0x0070C0)),
        ..TextFormat::default()
    };
    column.plot_layout = Some(ManualLayout {
        x: 0.15,
        y: 0.25,
        w: Some(0.7),
        h: Some(0.5),
    });
    column.axes.category.title = Some("Month".into());
    column.axes.category.title_layout = Some(ManualLayout::at(0.7, 0.9));
    column.axes.category.title_text = font("Courier New", 12.0);
    column.axes.category.text = TextFormat {
        size: Some(8.0),
        bold: Some(true),
        ..TextFormat::default()
    };
    column.axes.category.line = Line {
        color: Some(Color(0x7030A0)),
        width: Some(2.25),
        ..Line::default()
    };
    column.axes.value.title = Some("Units".into());
    column.axes.value.title_text.underline = Some(true);
    column.axes.value.text = font("Arial", 11.0);
    column.axes.value.minor_gridlines = true;
    column.axes.value.major_gridline = Line {
        color: Some(Color(0xFF0000)),
        width: Some(1.5),
        dash: Dash::Dash,
        ..Line::default()
    };
    column.axes.value.minor_gridline = Line {
        dash: Dash::RoundDot,
        ..Line::default()
    };
    // A scale with a line of its own, which it has not by itself.
    column.axes.value.line.color = Some(Color(0x00B050));
    column.data_labels = true;
    column.label_options.text = TextFormat {
        size: Some(7.0),
        color: Some(Color(0x404040)),
        ..TextFormat::default()
    };
    // Labels of its own for the second series, two of them dragged; one
    // dragged label on the series that follows the chart.
    column.series[1].labels = Some(DataLabels {
        position: LabelPosition::InsideEnd,
        text: font("Verdana", 10.0),
        ..DataLabels::default()
    });
    column.series[1].label_offsets = vec![
        LabelOffset {
            index: 0,
            dx: 0.025,
            dy: -0.05,
        },
        LabelOffset {
            index: 3,
            dx: -0.0125,
            dy: 0.0,
        },
    ];
    column.series[0].set_label_offset(2, 0.0, -0.075);

    // The box around the plot rather than its inside, no gridlines drawn
    // although the axis has them, an automatic title with a font.
    let mut pie = plain(ChartKind::Pie);
    pie.title = ChartTitle::Auto;
    pie.title_text.color = Some(Color(0x7030A0));
    pie.title_layout = Some(ManualLayout::at(0.6, 0.0));
    pie.plot_layout = Some(ManualLayout {
        x: 0.1,
        y: 0.2,
        w: Some(0.5),
        h: Some(0.75),
    });
    pie.plot_layout_outer = true;
    pie.data_labels = true;
    pie.series[0].set_label_offset(1, 0.1, 0.1);

    let mut scatter = plain(ChartKind::Scatter);
    scatter.legend_layout = Some(ManualLayout::at(0.4, 0.4));
    scatter.axes.category.major_gridline.shown = Some(false);
    scatter.axes.category.line.shown = Some(false);
    scatter.axes.value.line.dash = Dash::LongDash;
    vec![column, pie, scatter]
}

#[test]
fn layouts_fonts_and_gridline_formats_round_trip() {
    let parts = assert_round_trip(laid_out());
    let column = &parts[0];
    // As Excel writes them: edges measured from those of the chart, the
    // inside of the plot area.
    assert!(column.contains(
        "</c:tx><c:layout><c:manualLayout><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.05\"/><c:y val=\"0.03\"/></c:manualLayout></c:layout><c:overlay val=\"0\"/></c:title>"
    ));
    assert!(column.contains(
        "<c:plotArea><c:layout><c:manualLayout><c:layoutTarget val=\"inner\"/><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.15\"/><c:y val=\"0.25\"/><c:w val=\"0.7\"/><c:h val=\"0.5\"/></c:manualLayout></c:layout>"
    ));
    assert!(column.contains(
        "<c:legend><c:legendPos val=\"r\"/><c:layout><c:manualLayout><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.62\"/><c:y val=\"0.08\"/><c:w val=\"0.33\"/><c:h val=\"0.125\"/></c:manualLayout></c:layout><c:overlay val=\"0\"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"900\" i=\"1\"><a:solidFill><a:srgbClr val=\"0070C0\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr></c:legend>"
    ));
    // The font of a title: on the paragraph and on its runs.
    let props = "sz=\"2000\" b=\"1\" i=\"1\" u=\"sng\"><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill><a:latin typeface=\"Georgia\"/>";
    assert!(column.contains(&format!(
        "<a:p><a:pPr><a:defRPr {props}</a:defRPr></a:pPr><a:r><a:rPr lang=\"en-US\" {props}</a:rPr><a:t>Dragged &amp; formatted</a:t></a:r></a:p>"
    )));
    assert!(column.contains(
        "<c:majorGridlines><c:spPr><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></c:spPr></c:majorGridlines><c:minorGridlines><c:spPr><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"F2F2F2\"/></a:solidFill><a:prstDash val=\"sysDot\"/></a:ln></c:spPr></c:minorGridlines>"
    ));
    // The line and the labels of an axis.
    assert!(column.contains(
        "<c:tickLblPos val=\"nextTo\"/><c:spPr><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"7030A0\"/></a:solidFill></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"800\" b=\"1\"/></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr><c:crossAx"
    ));
    // A dragged label says where it went and, once more, what it shows.
    assert!(column.contains(
        "<c:dLbls><c:dLbl><c:idx val=\"0\"/><c:layout><c:manualLayout><c:x val=\"0.025\"/><c:y val=\"-0.05\"/></c:manualLayout></c:layout><c:txPr>"
    ));
    assert!(column.contains(
        "<c:dLbl><c:idx val=\"2\"/><c:layout><c:manualLayout><c:x val=\"0\"/><c:y val=\"-0.075\"/></c:manualLayout></c:layout><c:txPr>"
    ));
    assert!(parts[1].contains(
        "<c:plotArea><c:layout><c:manualLayout><c:layoutTarget val=\"outer\"/><c:xMode val=\"edge\"/>"
    ));
    assert!(parts[2].contains(&format!(
        "<c:majorGridlines><c:spPr>{}</c:spPr></c:majorGridlines>",
        "<a:ln><a:noFill/></a:ln>"
    )));

    // A file to open in Excel: the three charts on a sheet of their data.
    let mut wb = data_workbook();
    for (i, mut c) in laid_out().into_iter().enumerate() {
        c.anchor = (1, 8 * i as u32);
        wb.add_chart(1, c).unwrap();
    }
    wb.commit("charts");
    let bytes = to_bytes(&wb);
    keep_sample("objects-chart-layout.xlsx", &bytes);
    // Read and written again, the parts are the same text: from what was
    // kept of them, and from the model alone.
    let mut loaded = load(&bytes);
    assert_same_charts(&wb, &loaded);
    for chart in loaded.sheets.iter_mut().flat_map(|s| &mut s.charts) {
        assert!(chart.unchanged_source().is_some());
        chart.foreign = None;
    }
    let generated = to_bytes(&loaded);
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&generated, &name), "{name}");
    }
}

#[test]
fn layouts_and_fonts_of_an_excel_chart_survive_a_change() {
    // What Excel writes after the title, the legend, the plot area and a
    // label were dragged, the legend made bold, the gridlines red.
    let title = "<c:title><c:tx><c:rich><a:bodyPr rot=\"0\" spcFirstLastPara=\"1\" vertOverflow=\"ellipsis\" vert=\"horz\" wrap=\"square\" anchor=\"ctr\" anchorCtr=\"1\"/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\" i=\"0\" u=\"none\" strike=\"noStrike\" kern=\"1200\" spc=\"0\" baseline=\"0\"><a:solidFill><a:schemeClr val=\"tx1\"><a:lumMod val=\"65000\"/><a:lumOff val=\"35000\"/></a:schemeClr></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:pPr><a:r><a:rPr lang=\"fr-FR\" sz=\"1800\" i=\"1\"><a:latin typeface=\"Georgia\" panose=\"02040502050405020303\" pitchFamily=\"18\" charset=\"0\"/></a:rPr><a:t>Ventes</a:t></a:r></a:p></c:rich></c:tx><c:layout><c:manualLayout><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.125\"/><c:y val=\"0.0625\"/></c:manualLayout></c:layout><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>";
    let legend = format!(
        "<c:legend><c:legendPos val=\"b\"/><c:layout><c:manualLayout><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.5\"/><c:y val=\"0.75\"/><c:w val=\"0.375\"/><c:h val=\"0.125\"/></c:manualLayout></c:layout><c:overlay val=\"0\"/>{BLANK}{}</c:legend>",
        TEXT.replace("b=\"0\"", "b=\"1\"")
    );
    let red = SHAPE.replace(
        "<a:schemeClr val=\"tx1\"><a:lumMod val=\"15000\"/><a:lumOff val=\"85000\"/></a:schemeClr>",
        "<a:srgbClr val=\"FF0000\"/>",
    );
    let value_axis = excel_axis("val", 'l', (12, 11, "autoZero")).replacen(SHAPE, &red, 1);
    let column = |index: u32, column: char, labels: &str| {
        let (tx, data) = excel_data(column, false);
        format!(
            "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>{tx}<c:spPr><a:solidFill><a:schemeClr val=\"accent{}\"/></a:solidFill><a:ln><a:noFill/></a:ln><a:effectLst/></c:spPr><c:invertIfNegative val=\"0\"/>{labels}{data}{}</c:ser>",
            index + 1,
            unique_id(index)
        )
    };
    let shown = "<c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/>";
    let labels = format!(
        "<c:dLbls><c:dLbl><c:idx val=\"1\"/><c:layout><c:manualLayout><c:x val=\"2.5000000000000001E-2\"/><c:y val=\"-0.125\"/></c:manualLayout></c:layout>{shown}</c:dLbl>{BLANK}{TEXT}{shown}</c:dLbls>"
    );
    let chart_part = format!(
        "<c:plotArea><c:layout><c:manualLayout><c:layoutTarget val=\"inner\"/><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"0.25\"/><c:y val=\"0.25\"/><c:w val=\"0.5\"/><c:h val=\"0.375\"/></c:manualLayout></c:layout><c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>{}{}<c:dLbls>{shown}</c:dLbls><c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/><c:axId val=\"11\"/><c:axId val=\"12\"/></c:barChart>{}{value_axis}{BLANK}</c:plotArea>{legend}",
        column(0, 'C', &labels),
        column(
            1,
            'D',
            &labels.replace("<c:idx val=\"1\"/>", "<c:idx val=\"9\"/>")
        ),
        excel_axis("cat", 'b', (11, 12, "autoZero")),
    );
    let xml = excel_part(&chart_part).replacen(
        &format!(
            "<c:title><c:overlay val=\"0\"/>{BLANK}{}</c:title><c:autoTitleDeleted val=\"0\"/>",
            TEXT.replace("sz=\"900\"", "sz=\"1400\"")
        ),
        title,
        1,
    );
    assert!(xml.contains("Ventes"));
    let read = excel(&xml);
    assert_eq!(read.title_layout, Some(ManualLayout::at(0.125, 0.0625)));
    assert_eq!(
        read.title_text,
        TextFormat {
            font: Some("Georgia".into()),
            size: Some(18.0),
            italic: Some(true),
            ..TextFormat::default()
        }
    );
    assert_eq!(
        read.legend_layout,
        Some(ManualLayout {
            x: 0.5,
            y: 0.75,
            w: Some(0.375),
            h: Some(0.125),
        })
    );
    // Excel's own nine points of dark grey are nothing of the legend's.
    assert_eq!(
        read.legend_text,
        TextFormat {
            bold: Some(true),
            ..TextFormat::default()
        }
    );
    assert_eq!(
        read.plot_layout,
        Some(ManualLayout {
            x: 0.25,
            y: 0.25,
            w: Some(0.5),
            h: Some(0.375),
        })
    );
    assert!(!read.plot_layout_outer);
    assert_eq!(
        read.axes.value.major_gridline,
        Line {
            color: Some(Color(0xFF0000)),
            ..Line::default()
        }
    );
    assert_eq!(read.axes.category.text, TextFormat::default());
    assert_eq!(read.axes.category.line, Line::default());
    assert_eq!(read.axes.value.line, Line::default());
    assert!(read.data_labels);
    assert_eq!(read.label_options.text, TextFormat::default());
    let moved = |index| {
        vec![LabelOffset {
            index,
            dx: 0.025,
            dy: -0.125,
        }]
    };
    assert_eq!(read.series[0].label_offsets, moved(1));
    assert_eq!(read.series[1].label_offsets, moved(9));
    assert_eq!(read.series[0].labels, None);

    // Changed, the chart is written from the model: nothing of this is lost.
    let mut wb = load(&foreign_package(&[
        ("xl/charts/chart1.xml", &xml),
        ("xl/theme/theme1.xml", THEME_2023),
    ]));
    let id = wb.sheets[0].charts[0].id;
    wb.update_chart(0, id, |c| c.legend_overlay = true);
    assert!(wb.sheets[0].charts[0].unchanged_source().is_none());
    let again = reload(&wb);
    let mut expected = read.content();
    expected.legend_overlay = true;
    assert_same_content("regenerated", &expected, &again.sheets[0].charts[0]);
}

/// Category labels in two columns are two levels of labels: written as
/// Excel caches them, read back as the block, and plotted on two levels.
#[test]
fn categories_on_several_levels_go_through_a_file() {
    let mut wb = Workbook::new(Locale::en());
    let rows = [
        ("Year", "Quarter", "Sales", "Costs"),
        ("2024", "Q1", "10", "7"),
        ("", "Q2", "12", "8"),
        ("", "Q3", "9", "6"),
        ("2025", "Q1", "14", "9"),
        ("", "Q2", "11", "8"),
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, text) in [row.0, row.1, row.2, row.3].iter().enumerate() {
            if !text.is_empty() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
    }
    let mut chart = wb.chart_from_range(0, Rect::new(0, 1, 5, 3), ChartKind::Column);
    let name = wb.sheets[0].name.clone();
    let block = match parse_formula(&format!("{name}!$A$2:$B$6"), Locale::en()) {
        Ok(Expr::Ref(r)) => r,
        other => panic!("{other:?}"),
    };
    for series in &mut chart.series {
        series.categories = Some(block.clone());
    }
    chart.anchor = (7, 0);
    wb.add_chart(0, chart);
    wb.commit("chart");
    let bytes = to_bytes(&wb);
    keep_sample("chart-multilevel.xlsx", &bytes);
    let xml = part(&bytes, "xl/charts/chart1.xml");
    assert_schema_order("chart1.xml", &xml);
    let cat = "<c:cat><c:multiLvlStrRef><c:f>Sheet1!$A$2:$B$6</c:f><c:multiLvlStrCache><c:ptCount val=\"5\"/><c:lvl><c:pt idx=\"0\"><c:v>Q1</c:v></c:pt><c:pt idx=\"1\"><c:v>Q2</c:v></c:pt><c:pt idx=\"2\"><c:v>Q3</c:v></c:pt><c:pt idx=\"3\"><c:v>Q1</c:v></c:pt><c:pt idx=\"4\"><c:v>Q2</c:v></c:pt></c:lvl><c:lvl><c:pt idx=\"0\"><c:v>2024</c:v></c:pt><c:pt idx=\"3\"><c:v>2025</c:v></c:pt></c:lvl></c:multiLvlStrCache></c:multiLvlStrRef></c:cat>";
    assert_eq!(xml.matches(cat).count(), 2, "{xml}");
    assert!(xml.contains("<c:noMultiLvlLbl val=\"0\"/>"));
    let back = load(&bytes);
    let read = &back.sheets[0].charts[0];
    assert_eq!(
        refs(&read.series[0]),
        ("Sheet1!$C$2:$C$6".into(), Some("Sheet1!$A$2:$B$6".into()))
    );
    let data = back.chart_data(0, read);
    assert_eq!(data.categories, ["Q1", "Q2", "Q3", "Q1", "Q2"]);
    assert_eq!(data.outer_levels, [["2024", "", "", "2025", ""]]);
    assert_eq!(to_bytes(&back), bytes);
}
