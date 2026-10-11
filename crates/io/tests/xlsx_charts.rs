//! Charts in `.xlsx` files: what the writer produces, that it comes back
//! unchanged, and what is made of charts written by other applications.

mod common;

use common::{assert_well_formed, enter, package, part, part_names};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::chart::{
    Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
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
        values: range(values),
        categories: categories.map(range),
        color: None,
    }
}

fn named(cell: &str) -> SeriesName {
    SeriesName::Cell(range(cell))
}

fn chart(kind: ChartKind, series: Vec<Series>) -> Chart {
    Chart {
        id: 0,
        kind,
        title: ChartTitle::Auto,
        legend: LegendPos::None,
        series,
        data_labels: false,
        anchor: (0, 0),
        offset: (0.0, 0.0),
        size: DEFAULT_CHART_SIZE,
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

/// A workbook with a chart of every kind, on two sheets.
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

/// Charts are the same when everything but their ids is.
fn assert_same_charts(before: &Workbook, after: &Workbook) {
    assert_eq!(before.sheets.len(), after.sheets.len());
    let mut ids = HashSet::new();
    for (a, b) in before.sheets.iter().zip(&after.sheets) {
        let name = &a.name;
        assert_eq!(a.charts.len(), b.charts.len(), "{name}: number of charts");
        for (i, (x, y)) in a.charts.iter().zip(&b.charts).enumerate() {
            let which = format!("{name}, chart {} ({:?})", i + 1, x.kind);
            assert_eq!(x.kind, y.kind, "{which}: kind");
            assert_eq!(x.title, y.title, "{which}: title");
            assert_eq!(x.legend, y.legend, "{which}: legend");
            assert_eq!(x.data_labels, y.data_labels, "{which}: data labels");
            assert_eq!(x.series, y.series, "{which}: series");
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

#[test]
fn charts_round_trip() {
    let original = sample();
    assert_eq!(original.sheets[0].charts.len(), 10);
    let kinds: HashSet<ChartKind> = original.sheets[0].charts.iter().map(|c| c.kind).collect();
    assert_eq!(kinds.len(), ChartKind::ALL.len(), "every kind is exercised");

    let bytes = to_bytes(&original);
    keep("opensheet-charts.xlsx", &bytes);
    let loaded = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
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

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
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
    assert!(column.contains("<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:tx><c:strRef><c:f>Sales!$B$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>Q1</c:v></c:pt></c:strCache></c:strRef></c:tx><c:invertIfNegative val=\"0\"/><c:cat><c:strRef><c:f>Sales!$A$2:$A$5</c:f><c:strCache><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>Keyboard</c:v></c:pt><c:pt idx=\"1\"><c:v>Mouse</c:v></c:pt><c:pt idx=\"2\"><c:v>Screen</c:v></c:pt><c:pt idx=\"3\"><c:v>Cable</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sales!$B$2:$B$5</c:f><c:numCache><c:formatCode>#,##0.00</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"0\"><c:v>10</c:v></c:pt><c:pt idx=\"1\"><c:v>20</c:v></c:pt><c:pt idx=\"2\"><c:v>30.5</c:v></c:pt><c:pt idx=\"3\"><c:v>-4</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>"), "{column}");
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
    assert!(column.contains("<c:crossAx val=\"50000011\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx></c:plotArea><c:legend><c:legendPos val=\"b\"/><c:overlay val=\"0\"/></c:legend><c:plotVisOnly val=\"1\"/>"));
    // An automatic title: a title element without text, and not deleted.
    assert!(column.contains("<c:chart><c:title><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/><c:plotArea>"));
    assert!(!column.contains("<c:dLbls>") && !column.contains("srgbClr val=\"C00000\""));

    let stacked = chart(2);
    assert!(stacked.contains("<c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:r><a:t>Sales &amp; &lt;costs&gt;</a:t></a:r></a:p><a:p><a:pPr><a:defRPr sz=\"1400\" b=\"0\"/></a:pPr><a:r><a:t>per quarter</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>"), "{stacked}");
    assert!(stacked.contains("<c:barDir val=\"col\"/><c:grouping val=\"stacked\"/>"));
    assert!(stacked.contains("</c:tx><c:spPr><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>"));
    assert_eq!(
        stacked.matches("<c:spPr><a:solidFill>").count(),
        1,
        "one explicit colour"
    );
    assert!(stacked.contains("</c:ser><c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/></c:dLbls><c:gapWidth val=\"150\"/><c:overlap val=\"100\"/>"));
    assert!(stacked.contains("<c:legendPos val=\"r\"/>"));

    // Horizontal bars: the category axis on the left. A typed name; a
    // series with neither name nor categories.
    let bar = chart(3);
    assert!(bar.contains("<c:chart><c:autoTitleDeleted val=\"1\"/><c:plotArea>"));
    assert!(bar.contains("<c:barDir val=\"bar\"/><c:grouping val=\"clustered\"/>"));
    assert!(bar.contains("<c:order val=\"0\"/><c:tx><c:v>Literal &amp; &lt;name&gt;</c:v></c:tx><c:invertIfNegative val=\"0\"/><c:val><c:numRef><c:f>Sales!$C$2:$C$5</c:f>"));
    assert!(bar.contains("<c:order val=\"1\"/><c:invertIfNegative val=\"0\"/><c:val><c:numRef><c:f>Sales!$D$2:$D$5</c:f>"));
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
    assert!(pie.contains("</c:ser><c:dLbls><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/><c:showLeaderLines val=\"1\"/></c:dLbls><c:firstSliceAng val=\"0\"/></c:pieChart></c:plotArea>"));
    let doughnut = chart(8);
    assert!(doughnut.contains("<c:doughnutChart><c:varyColors val=\"1\"/><c:ser>"));
    assert!(doughnut.contains("</c:ser><c:firstSliceAng val=\"0\"/><c:holeSize val=\"75\"/></c:doughnutChart></c:plotArea>"));
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
    assert!(scatter.contains("<c:tx><c:v>No X</c:v></c:tx><c:spPr><a:ln w=\"19050\" cap=\"rnd\"><a:noFill/><a:round/></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/></c:marker><c:yVal>"));
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
    // values, the column next to them for categories.
    assert!(xml.contains("<c:tx><c:strRef><c:f>Sheet1!$A$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>g0</c:v></c:pt></c:strCache></c:strRef></c:tx>"));
    assert!(xml.contains("<c:cat><c:strRef><c:f>Sheet1!$B$1:$B$3</c:f><c:strCache><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>c0</c:v></c:pt><c:pt idx=\"1\"><c:v>c1</c:v></c:pt><c:pt idx=\"2\"><c:v>c2</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$C$1:$C$3</c:f>"));
    // A name in several cells of a column is a name; text among the values
    // has no point.
    assert!(xml.contains("<c:tx><c:strRef><c:f>Sheet1!$A$1:$A$3</c:f><c:strCache><c:ptCount val=\"3\"/><c:pt idx=\"0\"><c:v>g0</c:v></c:pt><c:pt idx=\"1\"><c:v>g1</c:v></c:pt><c:pt idx=\"2\"><c:v>g2</c:v></c:pt></c:strCache></c:strRef></c:tx>"));
    assert!(xml.contains("<c:val><c:numRef><c:f>Sheet1!$A$1:$D$1</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"4\"/><c:pt idx=\"2\"><c:v>30</c:v></c:pt><c:pt idx=\"3\"><c:v>300</c:v></c:pt></c:numCache></c:numRef></c:val>"));
    assert!(xml.contains("<c:val><c:numRef><c:f>Data!$A$1:$A$1048576</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"1048576\"/><c:pt idx=\"0\"><c:v>3</c:v></c:pt>"));
    for block in ["$A$1:$B$3", "$C$1:$D$3", "$A$1:$D$2", "XFD"] {
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
        ("Sheet1!$A$1", "Sheet1!$C$1:$C$3", Some("Sheet1!$B$1:$B$3")),
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
            "c:areaChart|c:barChart|c:doughnutChart|c:lineChart|c:pieChart|c:scatterChart",
            "c:catAx|c:valAx",
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
        "c:ser/c:marker",
        &["c:symbol", "c:size", "c:spPr", "c:extLst"],
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
    ("c:strRef", &["c:f", "c:strCache", "c:extLst"]),
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
        "c:scaling",
        &["c:logBase", "c:orientation", "c:max", "c:min", "c:extLst"],
    ),
    ("c:majorGridlines", &["c:spPr", "c:extLst"]),
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
        ],
    ),
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
    ("a:solidFill", &["a:srgbClr"]),
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
    "c:ser",
    "c:pt",
    "c:axId",
    "c:catAx",
    "c:valAx",
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
    // Theme colours, resolved; not the red of one point or the green of a
    // trend line.
    assert_eq!(columns.series[0].color, Some(Color(0x4472C4)));
    assert_eq!(columns.series[1].color, Some(Color(0xED7D31)));
    let data = wb.chart_data(0, columns);
    assert_eq!(data.categories, ["Q1", "Q2", "Q3"]);
    assert_eq!(data.series[0].name, "North");
    assert_eq!(data.series[1].values, [Some(7.0), Some(9.0), Some(4.0)]);
    // From 20, 10 inside F3 to 40 pixels into L17: columns of 70 pixels
    // (one of them hidden) and rows of 24, as the sheet declares.
    assert_eq!(columns.anchor, (2, 5));
    assert_eq!(columns.offset, (20.0, 10.0));
    assert_eq!(columns.size, (5.0 * 70.0 + 40.0 - 20.0, 14.0 * 24.0 - 10.0));

    // Bars stacked to 100 % plus a line: stacked bars, with all three
    // series in the chart's order.
    let combo = &charts[1];
    assert_eq!(combo.kind, ChartKind::StackedBar);
    assert_eq!(combo.title, ChartTitle::None);
    assert_eq!(combo.legend, LegendPos::Right);
    assert!(combo.data_labels, "one series is labelled");
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
    assert_eq!(
        combo.series[1].color,
        Some(Color(0x7C7C7C)),
        "the stroke of the line: accent 3, a quarter darker"
    );
    assert_eq!(combo.series[2].name, SeriesName::None);
    assert_eq!(refs(&combo.series[2]), ("Data!$D$2:$D$4".into(), None));
    assert_eq!(
        combo.series[2].color,
        Some(Color(0x112233)),
        "a pattern is no colour; the outline is"
    );
    assert_eq!(wb.chart_data(0, combo).series[0].name, "Elsewhere");
    assert_eq!((combo.anchor, combo.offset), ((20, 1), (0.0, 0.0)));
    assert_eq!(combo.size, (480.0, 288.0));

    // The 3-D pie.
    let pie = &charts[2];
    assert_eq!(pie.kind, ChartKind::Pie);
    assert_eq!(pie.title, ChartTitle::Text("North".into()));
    assert_eq!(pie.legend, LegendPos::Right);
    assert!(pie.data_labels, "percentages are labels");
    assert_eq!(pie.series.len(), 1);
    assert_eq!(
        refs(&pie.series[0]),
        ("Data!$C$2:$C$4".into(), Some("Data!$B$2:$B$4".into())),
        "of two levels of categories, the inner one"
    );
    assert_eq!(pie.series[0].color, None);
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
    assert_eq!(scatter.series[0].color, Some(Color(0x7030A0)));
    assert_eq!(refs(&scatter.series[1]), ("Data!$C:$C".into(), None));
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

    // What was read can be written, and read again.
    let again = reload(&wb);
    assert_same_charts(&wb, &again);
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
    for (element, head, kind) in [
        ("barChart", bar("col", "clustered"), ChartKind::Column),
        ("barChart", bar("col", "standard"), ChartKind::Column),
        ("barChart", bar("col", "stacked"), ChartKind::StackedColumn),
        (
            "barChart",
            bar("col", "percentStacked"),
            ChartKind::StackedColumn,
        ),
        ("barChart", bar("bar", "clustered"), ChartKind::Bar),
        ("barChart", bar("bar", "stacked"), ChartKind::StackedBar),
        (
            "barChart",
            bar("bar", "percentStacked"),
            ChartKind::StackedBar,
        ),
        ("barChart", String::new(), ChartKind::Column),
        ("bar3DChart", bar("bar", "stacked"), ChartKind::StackedBar),
        ("bar3DChart", bar("col", "standard"), ChartKind::Column),
        (
            "lineChart",
            "<c:grouping val=\"stacked\"/>".into(),
            ChartKind::Line,
        ),
        ("line3DChart", String::new(), ChartKind::Line),
        ("stockChart", String::new(), ChartKind::Line),
        (
            "radarChart",
            "<c:radarStyle val=\"marker\"/>".into(),
            ChartKind::Line,
        ),
        (
            "surfaceChart",
            "<c:wireframe val=\"0\"/>".into(),
            ChartKind::Line,
        ),
        ("surface3DChart", String::new(), ChartKind::Line),
        (
            "areaChart",
            "<c:grouping val=\"percentStacked\"/>".into(),
            ChartKind::Area,
        ),
        ("area3DChart", String::new(), ChartKind::Area),
        ("pieChart", String::new(), ChartKind::Pie),
        ("pie3DChart", String::new(), ChartKind::Pie),
        (
            "ofPieChart",
            "<c:ofPieType val=\"bar\"/>".into(),
            ChartKind::Pie,
        ),
        ("doughnutChart", String::new(), ChartKind::Doughnut),
        ("scatterChart", String::new(), ChartKind::Scatter),
        ("bubbleChart", String::new(), ChartKind::Scatter),
    ] {
        let xml = plot(element, &head);
        assert_well_formed(element, &xml);
        let wb = load(&foreign_package(&[("xl/charts/chart1.xml", &xml)]));
        let chart = &wb.sheets[0].charts[0];
        assert_eq!(chart.kind, kind, "{element} {head}");
        assert_eq!(chart.legend, LegendPos::Left);
        assert_eq!(
            refs(&chart.series[0]),
            ("Data!$C$2:$C$4".into(), Some("Data!$B$2:$B$4".into())),
            "{element}"
        );
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
    // gives fewer charts, or none, but returns.
    let mut attempts = 0;
    for bytes in [to_bytes(&sample()), foreign_package(&[])] {
        let names = part_names(&bytes);
        let parts: Vec<(String, String)> =
            names.iter().map(|n| (n.clone(), part(&bytes, n))).collect();
        let complete = load(&bytes).sheets[0].charts.len();
        for (victim, text) in parts.iter().filter(|(n, _)| {
            ["chart", "drawing", "sheet1.xml"]
                .iter()
                .any(|k| n.contains(k))
        }) {
            let step = (text.len() / 40).max(1);
            for cut in (0..text.len())
                .step_by(step)
                .filter(|i| text.is_char_boundary(*i))
            {
                let damaged: Vec<(&str, &str)> = parts
                    .iter()
                    .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                    .collect();
                if let Ok(wb) = read_xlsx(Cursor::new(package(&damaged)), Locale::en()) {
                    assert!(
                        wb.sheets[0].charts.len() <= complete,
                        "{victim} cut at {cut}"
                    );
                    // And what is left can be saved.
                    to_bytes(&wb);
                }
                attempts += 1;
            }
        }
    }
    assert!(attempts > 500, "{attempts}");
}
