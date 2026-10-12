//! Conditional formatting in `.xlsx` files: our own round trip of every
//! kind of rule, rules as Excel writes them (2007 form, x14 extension and
//! the two tied together), and damaged input.

mod common;

use common::{assert_well_formed, assert_worksheet_order, at, enter, package, part, value};
use opensheet_engine::condfmt::{
    BarAxis, BarColor, BarDirection, CellOp, Cfvo, ColorScale, ColorStop, CondFormat, DataBar,
    HighlightFormat, IconRule, IconSet, IconThreshold, Rule, ScalePreset, TextOp, TimePeriod,
    result_at,
};
use opensheet_engine::page::{Margins, Orientation, PaperSize};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::style::{Border, BorderStyle, StylePatch, Underline};
use opensheet_engine::validation::{Validation, ValidationKind};
use opensheet_engine::{Color, Expr, Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;
use std::sync::Arc;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn rects(names: &str) -> Vec<Rect> {
    names
        .split(' ')
        .map(|name| Rect::parse(name).unwrap())
        .collect()
}

fn f(text: &str) -> Expr {
    parse_formula(text, Locale::en()).unwrap_or_else(|e| panic!("{text}: {e:?}"))
}

fn fill(rgb: u32) -> StylePatch {
    StylePatch {
        fill: Some(Some(Color(rgb))),
        ..StylePatch::default()
    }
}

fn rule(ranges: &str, rule: Rule) -> CondFormat {
    CondFormat::new(rects(ranges), rule)
}

/// A workbook with every kind of rule, in an order that mixes the three
/// ways they are stored.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    assert!(wb.rename_sheet(0, "Scores"));
    assert!(wb.rename_sheet(1, "Limits & more"));
    for row in 1..20u32 {
        wb.set_value(0, row, 0, Value::Number((row * 7 % 100) as f64));
        wb.set_value(0, row, 1, Value::Number(row as f64 - 8.0));
        wb.set_value(
            0,
            row,
            2,
            Value::text(["abc", "xyz", "zab"][row as usize % 3]),
        );
    }
    enter(&mut wb, 0, "H1", "5");
    enter(&mut wb, 0, "H2", "15");
    enter(&mut wb, 0, "H3", "ab");
    enter(&mut wb, 1, "A1", "50");
    enter(&mut wb, 1, "B1", "3");

    let red = HighlightFormat::LightRedFillDarkRedText.patch();
    let fancy = StylePatch {
        font_name: Some(Arc::from("Arial")),
        font_size: Some(24),
        bold: Some(true),
        italic: Some(false),
        underline: Some(Underline::Single),
        strike: Some(true),
        font_color: Some(Some(Color(0x1F4E78))),
        fill: Some(None),
        border_left: Some(Border::thin()),
        border_right: None,
        border_top: Some(Border {
            style: BorderStyle::Double,
            color: Some(Color(0xC00000)),
        }),
        border_bottom: Some(Border::NONE),
        num_fmt: Some(Arc::from("0.0\" pts\"")),
    };
    let percent_format = StylePatch {
        num_fmt: Some(Arc::from("0.00%")),
        ..StylePatch::default()
    };

    let mut rules = vec![
        rule("A2:A20", Rule::greater_than(f("80")))
            .with_format(red.clone())
            .with_stop_if_true(true),
        rule("B2:B20", Rule::between(f("10"), f("20"))).with_format(fancy),
        // The same cells: stored under the same element.
        rule(
            "B2:B20",
            Rule::cell_is(CellOp::NotBetween, f("$H$1"), Some(f("$H$2"))),
        )
        .with_format(percent_format),
        rule("A2:F20", Rule::Formula(f("MOD(ROW(),2)=0"))).with_format(fill(0xF2F2F2)),
        rule("C2:C20 E2:E5", Rule::Formula(f("$A2>B2"))).with_format(red.clone()),
        // Another sheet in the formula: only the extension list can say it.
        rule("A2:A20", Rule::Formula(f("'Limits & more'!$A$1<A2"))).with_format(fill(0xDDEBF7)),
        rule("B2:B20", Rule::greater_than(f("'Limits & more'!$B$1")))
            .with_format(red.clone())
            .with_stop_if_true(true),
        rule("A2:A20", Rule::top_items(3)).with_format(fill(0xC6EFCE)),
        rule("A2:A20", Rule::bottom_percent(10)).with_format(fill(0xFFEB9C)),
        rule("B2:B20", Rule::above_average()).with_format(fill(0xC6EFCE)),
        rule(
            "B2:B20",
            Rule::Average {
                above: false,
                or_equal: true,
                std_dev: 2,
            },
        )
        .with_format(fill(0xFFEB9C)),
        rule("C2:C20", Rule::DuplicateValues).with_format(red.clone()),
        rule("C2:C20", Rule::UniqueValues).with_format(fill(0xC6EFCE)),
        rule("C2:C20", Rule::text_contains(Expr::Text(Arc::from("ab")))).with_format(red.clone()),
        rule(
            "C2:C20",
            Rule::Text {
                op: TextOp::NotContains,
                text: Expr::Text(Arc::from("a<b>&\"c\"")),
            },
        )
        .with_format(fill(0xEEEEEE)),
        rule(
            "C2:C20",
            Rule::Text {
                op: TextOp::BeginsWith,
                text: f("$H$3"),
            },
        )
        .with_format(fill(0xDDDDDD)),
        rule(
            "C2:C20",
            Rule::Text {
                op: TextOp::EndsWith,
                text: Expr::Text(Arc::from("z")),
            },
        )
        .with_format(fill(0xCCCCCC)),
        rule("D2:D20", Rule::Blanks).with_format(fill(0xBBBBBB)),
        rule("D2:D20", Rule::NoBlanks).with_format(fill(0xAAAAAA)),
        rule("D2:D20", Rule::Errors).with_format(red.clone()),
        rule("D2:D20", Rule::NoErrors),
        // A rule with nothing but "stop if true".
        rule("F2:F20", Rule::Formula(f("TRUE"))).with_stop_if_true(true),
    ];
    for period in TimePeriod::ALL {
        rules.push(rule("E2:E20", Rule::date_occurring(period)).with_format(red.clone()));
    }
    // Colour scales.
    rules.push(rule(
        "A2:A20",
        Rule::ColorScale(ScalePreset::GreenYellowRed.scale()),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::ColorScale(ColorScale::two(Color(0xFFFFFF), Color(0x63BE7B))),
    ));
    rules.push(rule(
        "A2:B20",
        Rule::ColorScale(ColorScale {
            stops: vec![
                ColorStop {
                    at: Cfvo::number(-5.0),
                    color: Color(0xF8696B),
                },
                ColorStop {
                    at: Cfvo::percent(40.0),
                    color: Color(0xFFEB84),
                },
                ColorStop {
                    at: Cfvo::Formula(f("$H$2*2")),
                    color: Color(0x63BE7B),
                },
            ],
        }),
    ));
    rules.push(rule(
        "A2:A20",
        Rule::ColorScale(ColorScale {
            stops: vec![
                ColorStop {
                    at: Cfvo::Min,
                    color: Color(0xFFFFFF),
                },
                ColorStop {
                    at: Cfvo::Formula(f("'Limits & more'!$A$1")),
                    color: Color(0x5A8AC6),
                },
            ],
        }),
    ));
    // Data bars: as Excel 2007 made them, as the gallery makes them now
    // (gradient and solid), and with every option changed.
    let mut old_bar = DataBar::legacy(Color(0x638EC6));
    old_bar.max = Cfvo::percentile(90.0);
    old_bar.show_value = false;
    old_bar.min_length = 20;
    old_bar.max_length = 80;
    rules.push(rule("A2:A20", Rule::DataBar(old_bar)));
    rules.push(rule(
        "A2:A20",
        Rule::DataBar(DataBar::legacy(Color(0x63C384))),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::DataBar(DataBar::preset(BarColor::Blue, true)),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::DataBar(DataBar::preset(BarColor::Orange, false)),
    ));
    rules.push(rule(
        "B2:B20 A2:A5",
        Rule::DataBar(DataBar {
            min: Cfvo::number(0.0),
            max: Cfvo::percent(90.0),
            fill: Color(0xD6007B),
            gradient: false,
            border: Some(Color(0x7B0046)),
            negative_fill: None,
            negative_border: Some(Color(0x00B050)),
            axis: BarAxis::Midpoint,
            axis_color: Color(0x808080),
            direction: BarDirection::RightToLeft,
            min_length: 5,
            max_length: 75,
            show_value: false,
        }),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::DataBar(DataBar {
            axis: BarAxis::None,
            direction: BarDirection::LeftToRight,
            negative_border: None,
            ..DataBar::preset(BarColor::Green, true)
        }),
    ));
    rules.push(rule(
        "A2:A20",
        Rule::DataBar(DataBar {
            min: Cfvo::Formula(f("'Limits & more'!$B$1")),
            ..DataBar::preset(BarColor::Red, false)
        }),
    ));
    // Icon sets: old ones, the three added in 2010, and icons picked one
    // by one.
    rules.push(rule(
        "A2:A20",
        Rule::IconSet(IconRule::preset(IconSet::ThreeArrows)),
    ));
    rules.push(rule(
        "A2:A20",
        Rule::IconSet(IconRule::preset(IconSet::ThreeTrafficLights1)),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::IconSet(IconRule {
            set: IconSet::FourRating,
            thresholds: vec![
                IconThreshold {
                    value: Cfvo::percent(0.0),
                    gte: true,
                },
                IconThreshold {
                    value: Cfvo::number(-2.5),
                    gte: false,
                },
                IconThreshold {
                    value: Cfvo::percentile(50.0),
                    gte: true,
                },
                IconThreshold {
                    value: Cfvo::Formula(f("$H$2")),
                    gte: false,
                },
            ],
            reverse: true,
            show_value: false,
            custom: None,
        }),
    ));
    rules.push(rule(
        "B2:B20",
        Rule::IconSet(IconRule::preset(IconSet::FiveQuarters)),
    ));
    for set in [
        IconSet::ThreeStars,
        IconSet::ThreeTriangles,
        IconSet::FiveBoxes,
    ] {
        rules.push(rule("A2:A20", Rule::IconSet(IconRule::preset(set))));
    }
    rules.push(rule(
        "A2:A20",
        Rule::IconSet(IconRule {
            custom: Some(vec![
                Some((IconSet::ThreeSymbols, 0)),
                None,
                Some((IconSet::FiveBoxes, 4)),
            ]),
            show_value: false,
            ..IconRule::preset(IconSet::ThreeFlags)
        }),
    ));
    assert!(wb.set_cond_formats(0, rules));

    // A validation that reads the other sheet: its extension comes after
    // that of the conditional formats.
    let mut list = Validation {
        kind: ValidationKind::List,
        ..Validation::default()
    };
    list.set_operands("='Limits & more'!$A$1:$B$1", "", Locale::en())
        .unwrap();
    assert!(wb.set_validation(0, &rects("G2:G5"), (1, 6), list));
    wb.commit("rules");
    wb
}

#[test]
fn rules_round_trip() {
    let original = sample();
    let bytes = to_bytes(&original);
    common::keep_sample("conditional-formats.xlsx", &bytes);
    let loaded = load(&bytes);
    let (before, after) = (
        &original.sheets[0].cond_formats,
        &loaded.sheets[0].cond_formats,
    );
    assert_eq!(before.len(), 51);
    assert_eq!(before.len(), after.len());
    for (i, (a, b)) in before.iter().zip(after).enumerate() {
        assert_eq!(a, b, "rule {} ({})", i + 1, a.rule.ooxml_type());
    }
    assert!(loaded.sheets[1].cond_formats.is_empty());
    assert!(!loaded.modified && !loaded.can_undo());

    // The rules are alive: both workbooks format the cells the same way.
    for row in 0..22 {
        for col in 0..7 {
            assert_eq!(
                result_at(&original, 0, row, col),
                result_at(&loaded, 0, row, col),
                "cell ({row}, {col})"
            );
        }
    }
    let (row, col) = at("A14");
    assert_eq!(value(&loaded, 0, "A14"), Value::Number(91.0));
    let shown = result_at(&loaded, 0, row, col).expect("A14 is above 80");
    assert_eq!(shown.patch.fill, Some(Some(Color(0xFFC7CE))));
    assert_eq!(shown.patch.font_color, Some(Some(Color(0x9C0006))));

    // A second generation is the same file.
    let again = to_bytes(&loaded);
    for name in ["xl/worksheets/sheet1.xml", "xl/styles.xml"] {
        assert_eq!(part(&bytes, name), part(&again, name), "{name}");
    }
}

#[test]
fn rules_are_written_the_way_excel_writes_them() {
    let bytes = to_bytes(&sample());
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert_worksheet_order("sheet1.xml", &sheet);
    let styles = part(&bytes, "xl/styles.xml");
    assert_well_formed("styles.xml", &styles);
    let contains = |text: &str| assert!(sheet.contains(text), "missing {text}\nin {sheet}");

    // The plain rules sit between the cells and the validation rules, one
    // element per group of cells, each rule with its priority.
    contains(concat!(
        "</sheetData><conditionalFormatting sqref=\"A2:A20\">",
        "<cfRule type=\"cellIs\" dxfId=\"0\" priority=\"1\" stopIfTrue=\"1\" operator=\"greaterThan\"><formula>80</formula></cfRule>",
        "<cfRule type=\"top10\" dxfId=\"4\" priority=\"8\" rank=\"3\"/>",
        "<cfRule type=\"top10\" dxfId=\"5\" priority=\"9\" percent=\"1\" bottom=\"1\" rank=\"10\"/>"
    ));
    contains(concat!(
        "<conditionalFormatting sqref=\"B2:B20\">",
        "<cfRule type=\"cellIs\" dxfId=\"1\" priority=\"2\" operator=\"between\"><formula>10</formula><formula>20</formula></cfRule>",
        "<cfRule type=\"cellIs\" dxfId=\"2\" priority=\"3\" operator=\"notBetween\"><formula>$H$1</formula><formula>$H$2</formula></cfRule>",
        "<cfRule type=\"aboveAverage\" dxfId=\"4\" priority=\"10\"/>",
        "<cfRule type=\"aboveAverage\" dxfId=\"5\" priority=\"11\" aboveAverage=\"0\" stdDev=\"2\" equalAverage=\"1\"/>"
    ));
    contains(
        "<conditionalFormatting sqref=\"A2:F20\"><cfRule type=\"expression\" dxfId=\"3\" priority=\"4\"><formula>MOD(ROW(),2)=0</formula></cfRule></conditionalFormatting>",
    );
    contains(
        "<conditionalFormatting sqref=\"C2:C20 E2:E5\"><cfRule type=\"expression\" dxfId=\"0\" priority=\"5\"><formula>$A2&gt;B2</formula></cfRule></conditionalFormatting>",
    );
    contains(
        "<cfRule type=\"duplicateValues\" dxfId=\"0\" priority=\"12\"/><cfRule type=\"uniqueValues\" dxfId=\"4\" priority=\"13\"/>",
    );
    // Text, blank, error and date rules spell their test out as a formula
    // for the first cell, next to the attributes.
    contains(
        "<cfRule type=\"containsText\" dxfId=\"0\" priority=\"14\" operator=\"containsText\" text=\"ab\"><formula>NOT(ISERROR(SEARCH(\"ab\",C2)))</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"notContainsText\" dxfId=\"6\" priority=\"15\" operator=\"notContains\" text=\"a&lt;b&gt;&amp;&quot;c&quot;\"><formula>ISERROR(SEARCH(\"a&lt;b&gt;&amp;\"\"c\"\"\",C2))</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"beginsWith\" dxfId=\"7\" priority=\"16\" operator=\"beginsWith\"><formula>LEFT(C2,LEN($H$3))=$H$3</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"endsWith\" dxfId=\"8\" priority=\"17\" operator=\"endsWith\" text=\"z\"><formula>RIGHT(C2,LEN(\"z\"))=\"z\"</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"containsBlanks\" dxfId=\"9\" priority=\"18\"><formula>LEN(TRIM(D2))=0</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"notContainsBlanks\" dxfId=\"10\" priority=\"19\"><formula>LEN(TRIM(D2))&gt;0</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"containsErrors\" dxfId=\"0\" priority=\"20\"><formula>ISERROR(D2)</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"notContainsErrors\" priority=\"21\"><formula>NOT(ISERROR(D2))</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"expression\" priority=\"22\" stopIfTrue=\"1\"><formula>TRUE</formula></cfRule>",
    );
    contains(
        "<cfRule type=\"timePeriod\" dxfId=\"0\" priority=\"23\" timePeriod=\"yesterday\"><formula>FLOOR(E2,1)=TODAY()-1</formula></cfRule>",
    );
    contains(
        " timePeriod=\"last7Days\"><formula>AND(TODAY()-FLOOR(E2,1)&lt;=6,FLOOR(E2,1)&lt;=TODAY())</formula>",
    );
    contains(" priority=\"32\" timePeriod=\"nextMonth\">");

    // Colour scales, old data bars and old icon sets: thresholds, then
    // colours.
    contains(concat!(
        "<cfRule type=\"colorScale\" priority=\"33\"><colorScale><cfvo type=\"min\"/><cfvo type=\"percentile\" val=\"50\"/><cfvo type=\"max\"/>",
        "<color rgb=\"FFF8696B\"/><color rgb=\"FFFFEB84\"/><color rgb=\"FF63BE7B\"/></colorScale></cfRule>"
    ));
    contains(
        "<colorScale><cfvo type=\"num\" val=\"-5\"/><cfvo type=\"percent\" val=\"40\"/><cfvo type=\"formula\" val=\"$H$2*2\"/>",
    );
    contains(
        "<cfRule type=\"dataBar\" priority=\"37\"><dataBar minLength=\"20\" maxLength=\"80\" showValue=\"0\"><cfvo type=\"min\"/><cfvo type=\"percentile\" val=\"90\"/><color rgb=\"FF638EC6\"/></dataBar></cfRule>",
    );
    contains(
        "<cfRule type=\"dataBar\" priority=\"38\"><dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF63C384\"/></dataBar></cfRule>",
    );
    contains(
        "<cfRule type=\"iconSet\" priority=\"44\"><iconSet iconSet=\"3Arrows\"><cfvo type=\"percent\" val=\"0\"/><cfvo type=\"percent\" val=\"33\"/><cfvo type=\"percent\" val=\"67\"/></iconSet></cfRule>",
    );
    contains(
        "<cfRule type=\"iconSet\" priority=\"45\"><iconSet><cfvo type=\"percent\" val=\"0\"/>",
    );
    contains(concat!(
        "<iconSet iconSet=\"4Rating\" showValue=\"0\" reverse=\"1\"><cfvo type=\"percent\" val=\"0\"/><cfvo type=\"num\" val=\"-2.5\" gte=\"0\"/>",
        "<cfvo type=\"percentile\" val=\"50\"/><cfvo type=\"formula\" val=\"$H$2\" gte=\"0\"/></iconSet>"
    ));

    // A data bar of today: the 2007 half names the x14 half by an id.
    contains(concat!(
        "<cfRule type=\"dataBar\" priority=\"39\"><dataBar><cfvo type=\"min\"/><cfvo type=\"max\"/><color rgb=\"FF638EC6\"/></dataBar>",
        "<extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" uri=\"{B025F937-C7B1-47D3-B67F-A62EFF666E3E}\">",
        "<x14:id>{0C0FD1F0-0000-4000-0001-000000000027}</x14:id></ext></extLst></cfRule>"
    ));
    contains(concat!(
        "<x14:cfRule type=\"dataBar\" id=\"{0C0FD1F0-0000-4000-0001-000000000027}\">",
        "<x14:dataBar minLength=\"0\" maxLength=\"100\" border=\"1\" negativeBarBorderColorSameAsPositive=\"0\">",
        "<x14:cfvo type=\"autoMin\"/><x14:cfvo type=\"autoMax\"/><x14:borderColor rgb=\"FF638EC6\"/>",
        "<x14:negativeFillColor rgb=\"FFFF0000\"/><x14:negativeBorderColor rgb=\"FFFF0000\"/><x14:axisColor rgb=\"FF000000\"/>",
        "</x14:dataBar></x14:cfRule>"
    ));
    // The solid one of the gallery.
    contains(concat!(
        "<x14:dataBar minLength=\"0\" maxLength=\"100\" gradient=\"0\"><x14:cfvo type=\"autoMin\"/><x14:cfvo type=\"autoMax\"/>",
        "<x14:negativeFillColor rgb=\"FFFF0000\"/><x14:axisColor rgb=\"FF000000\"/></x14:dataBar>"
    ));
    // Every option at once.
    contains(
        "<dataBar showValue=\"0\"><cfvo type=\"num\" val=\"0\"/><cfvo type=\"percent\" val=\"90\"/><color rgb=\"FFD6007B\"/></dataBar>",
    );
    contains(concat!(
        "<x14:dataBar minLength=\"5\" maxLength=\"75\" showValue=\"0\" border=\"1\" gradient=\"0\" direction=\"rightToLeft\" ",
        "negativeBarColorSameAsPositive=\"1\" negativeBarBorderColorSameAsPositive=\"0\" axisPosition=\"middle\">",
        "<x14:cfvo type=\"num\"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type=\"percent\"><xm:f>90</xm:f></x14:cfvo>",
        "<x14:borderColor rgb=\"FF7B0046\"/><x14:negativeBorderColor rgb=\"FF00B050\"/><x14:axisColor rgb=\"FF808080\"/></x14:dataBar></x14:cfRule>",
        "<xm:sqref>B2:B20 A2:A5</xm:sqref>"
    ));
    contains(
        "<x14:dataBar minLength=\"0\" maxLength=\"100\" border=\"1\" direction=\"leftToRight\" axisPosition=\"none\">",
    );

    // Only in the extension list: the icon sets of 2010, icons picked one
    // by one, and whatever looks at another sheet (format included).
    contains(concat!(
        "<x14:cfRule type=\"iconSet\" priority=\"48\" id=\"{0C0FD1F0-0000-4000-0001-000000000030}\">",
        "<x14:iconSet iconSet=\"3Stars\"><x14:cfvo type=\"percent\"><xm:f>0</xm:f></x14:cfvo>",
        "<x14:cfvo type=\"percent\"><xm:f>33</xm:f></x14:cfvo><x14:cfvo type=\"percent\"><xm:f>67</xm:f></x14:cfvo></x14:iconSet></x14:cfRule>"
    ));
    contains(concat!(
        "<x14:iconSet iconSet=\"3Flags\" showValue=\"0\" custom=\"1\">",
        "<x14:cfvo type=\"percent\"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type=\"percent\"><xm:f>33</xm:f></x14:cfvo><x14:cfvo type=\"percent\"><xm:f>67</xm:f></x14:cfvo>",
        "<x14:cfIcon iconSet=\"3Symbols\" iconId=\"0\"/><x14:cfIcon iconSet=\"NoIcons\" iconId=\"0\"/><x14:cfIcon iconSet=\"5Boxes\" iconId=\"4\"/></x14:iconSet>"
    ));
    contains(concat!(
        "<x14:cfRule type=\"expression\" priority=\"6\" id=\"{0C0FD1F0-0000-4000-0001-000000000006}\">",
        "<xm:f>'Limits &amp; more'!$A$1&lt;A2</xm:f>",
        "<x14:dxf><fill><patternFill><bgColor rgb=\"FFDDEBF7\"/></patternFill></fill></x14:dxf></x14:cfRule>"
    ));
    contains(concat!(
        "<x14:cfRule type=\"cellIs\" priority=\"7\" stopIfTrue=\"1\" operator=\"greaterThan\" id=\"{0C0FD1F0-0000-4000-0001-000000000007}\">",
        "<xm:f>'Limits &amp; more'!$B$1</xm:f><x14:dxf><font><color rgb=\"FF9C0006\"/></font>"
    ));
    contains(
        "<x14:colorScale><x14:cfvo type=\"min\"/><x14:cfvo type=\"formula\"><xm:f>'Limits &amp; more'!$A$1</xm:f></x14:cfvo><x14:color rgb=\"FFFFFFFF\"/><x14:color rgb=\"FF5A8AC6\"/></x14:colorScale>",
    );
    contains(
        "<x14:cfvo type=\"formula\"><xm:f>'Limits &amp; more'!$B$1</xm:f></x14:cfvo><x14:cfvo type=\"autoMax\"/><x14:fillColor rgb=\"FFFF555A\"/>",
    );
    assert!(!sheet.contains("<formula>'Limits") && !sheet.contains("val=\"'Limits"));
    assert!(!sheet.contains("<cfRule type=\"iconSet\" priority=\"48\""));

    // The extension of the conditional formats comes first in the list of
    // the worksheet, after the page setup.
    contains(concat!(
        " footer=\"0.3\"/><extLst><ext xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" ",
        "uri=\"{78C0D931-6437-407d-A8EE-F0AAD7539E65}\"><x14:conditionalFormattings>",
        "<x14:conditionalFormatting xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\">"
    ));
    let validations = sheet
        .find("uri=\"{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}\"")
        .unwrap();
    assert!(sheet.rfind("</x14:conditionalFormattings></ext>").unwrap() < validations);
    assert!(sheet.ends_with("</x14:dataValidations></ext></extLst></worksheet>"));

    // Each format is stored once, however many rules use it.
    assert_eq!(styles.matches("<dxf>").count(), 11);
    assert!(styles.contains("<dxfs count=\"11\"><dxf><font><color rgb=\"FF9C0006\"/></font><fill><patternFill><bgColor rgb=\"FFFFC7CE\"/></patternFill></fill></dxf>"), "{styles}");
    assert!(styles.contains(concat!(
        "<dxf><font><b/><i val=\"0\"/><strike/><u/><sz val=\"12\"/><color rgb=\"FF1F4E78\"/><name val=\"Arial\"/></font>",
        "<numFmt numFmtId=\"164\" formatCode=\"0.0&quot; pts&quot;\"/><fill><patternFill patternType=\"none\"/></fill>",
        "<border><left style=\"thin\"><color auto=\"1\"/></left><right/><top style=\"double\"><color rgb=\"FFC00000\"/></top><bottom style=\"none\"/></border></dxf>"
    )));
    assert!(styles.contains("<dxf><numFmt numFmtId=\"10\" formatCode=\"0.00%\"/></dxf>"));
    let dxfs = styles.find("<dxfs").unwrap();
    assert!(
        styles[..dxfs].ends_with("</cellStyles>") && styles[dxfs..].contains("</dxfs><tableStyles")
    );

    // A sheet without rules says nothing of them.
    let other = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(!other.contains("conditionalFormatting") && !other.contains("extLst"));
    let plain = to_bytes(&Workbook::new(Locale::en()));
    assert!(part(&plain, "xl/styles.xml").contains("<dxfs count=\"0\"/>"));
}

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

/// Differential formats as Excel writes them, with theme colours; the
/// custom palette that follows them recolours an indexed colour they use.
fn excel_styles() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac x16r2 xr" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac">
<fonts count="1" x14ac:knownFonts="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts>
<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
<dxfs count="4">
<dxf><font><color rgb="FF9C0006"/></font><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf>
<dxf><font><b/><i/><color theme="0"/></font><fill><patternFill><bgColor theme="4" tint="-0.24994659260841701"/></patternFill></fill><border><left style="thin"><color auto="1"/></left><right style="thin"><color auto="1"/></right><top style="thin"><color auto="1"/></top><bottom style="thin"><color auto="1"/></bottom><vertical/><horizontal/></border></dxf>
<dxf><numFmt numFmtId="165" formatCode="0.0%"/><fill><patternFill patternType="solid"><fgColor indexed="10"/><bgColor indexed="10"/></patternFill></fill></dxf>
<dxf><font><strike val="0"/><u val="double"/><color auto="1"/></font></dxf>
</dxfs>
<tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/>
<colors><indexedColors><rgbColor rgb="00000000"/><rgbColor rgb="00FFFFFF"/><rgbColor rgb="00FF0000"/><rgbColor rgb="0000FF00"/><rgbColor rgb="000000FF"/><rgbColor rgb="00FFFF00"/><rgbColor rgb="00FF00FF"/><rgbColor rgb="0000FFFF"/><rgbColor rgb="00000000"/><rgbColor rgb="00FFFFFF"/><rgbColor rgb="00ABCDEF"/></indexedColors></colors>
</styleSheet>"#
    )
}

/// A sheet as Excel 365 saves it: priorities that do not follow the order
/// of the elements, a data bar in two halves, rules only the extension
/// list holds, and rules this program cannot keep.
fn excel_sheet() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac xr xr2 xr3" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision" xr:uid="{{00000000-0001-0000-0000-000000000000}}">
<dimension ref="A1:D6"/>
<sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews>
<sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/>
<sheetData>
<row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>10</v></c><c r="C1" t="inlineStr"><is><t>alpha</t></is></c></row>
<row r="2"><c r="A2"><v>2</v></c><c r="B2"><v>-20</v></c><c r="C2" t="inlineStr"><is><t>beta</t></is></c></row>
<row r="3"><c r="A3"><v>3</v></c><c r="B3"><v>30</v></c><c r="C3" t="inlineStr"><is><t>gamma</t></is></c></row>
</sheetData>
<conditionalFormatting sqref="A1:A3">
<cfRule type="cellIs" dxfId="0" priority="9" operator="greaterThan"><formula>2</formula></cfRule>
<cfRule type="colorScale" priority="8"><colorScale><cfvo type="min"/><cfvo type="percentile" val="50"/><cfvo type="max"/><color rgb="FFF8696B"/><color theme="0"/><color theme="4" tint="0.39997558519241921"/></colorScale></cfRule>
</conditionalFormatting>
<conditionalFormatting sqref="B1:B3">
<cfRule type="dataBar" priority="7"><dataBar><cfvo type="min"/><cfvo type="max"/><color rgb="FF638EC6"/></dataBar><extLst><ext uri="{{B025F937-C7B1-47D3-B67F-A62EFF666E3E}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:id>{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A00001}}</x14:id></ext></extLst></cfRule>
<cfRule type="iconSet" priority="2"><iconSet iconSet="3Arrows" showValue="0" reverse="1"><cfvo type="percent" val="0"/><cfvo type="percent" val="33"/><cfvo type="num" val="25" gte="0"/></iconSet></cfRule>
<cfRule type="top10" dxfId="1" priority="6" percent="1" bottom="1" rank="20"/>
</conditionalFormatting>
<conditionalFormatting sqref="C1:C3 E1">
<cfRule type="containsText" dxfId="2" priority="5" operator="containsText" text="a"><formula>NOT(ISERROR(SEARCH("a",C1)))</formula></cfRule>
<cfRule type="beginsWith" dxfId="3" priority="4" operator="beginsWith" text="al"><formula>EXACT(LEFT(C1,2),"al")</formula></cfRule>
<cfRule type="endsWith" dxfId="3" priority="23" operator="endsWith" text="ma"><formula>RIGHT(C1,2)="ma"</formula></cfRule>
<cfRule type="timePeriod" dxfId="0" priority="3" timePeriod="last7Days"><formula>AND(TODAY()-FLOOR(C1,1)&lt;=6,FLOOR(C1,1)&lt;=TODAY())</formula></cfRule>
<cfRule type="expression" dxfId="99" priority="10" stopIfTrue="1"><formula>LEN(C1)&gt;4</formula></cfRule>
</conditionalFormatting>
<conditionalFormatting pivot="1" sqref="D1:D3">
<cfRule type="expression" dxfId="0" priority="11"><formula>[1]Other!$A$1&gt;0</formula></cfRule>
<cfRule type="sparkles" dxfId="0" priority="12"/>
<cfRule type="cellIs" dxfId="0" priority="13" operator="about"><formula>1</formula></cfRule>
<cfRule type="cellIs" dxfId="0" priority="14" operator="between"><formula>1</formula></cfRule>
<cfRule type="iconSet" priority="15"><iconSet iconSet="3Arrows"><cfvo type="percent" val="0"/><cfvo type="percent" val="50"/></iconSet></cfRule>
<cfRule type="colorScale" priority="16"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFFF0000"/></colorScale></cfRule>
<cfRule type="dataBar" priority="17"><dataBar><cfvo type="min"/><color rgb="FF638EC6"/></dataBar></cfRule>
<cfRule type="timePeriod" dxfId="0" priority="18" timePeriod="nextYear"/>
<cfRule type="uniqueValues" dxfId="1" priority="19"/>
</conditionalFormatting>
<conditionalFormatting><cfRule type="duplicateValues" dxfId="0" priority="20"/></conditionalFormatting>
<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>
<extLst><ext uri="{{78C0D931-6437-407d-A8EE-F0AAD7539E65}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:conditionalFormattings>
<x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="dataBar" id="{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A00001}}"><x14:dataBar minLength="0" maxLength="100" border="1" negativeBarBorderColorSameAsPositive="0"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/><x14:borderColor rgb="FF638EC6"/><x14:negativeFillColor rgb="FFFF0000"/><x14:negativeBorderColor rgb="FFFF0000"/><x14:axisColor rgb="FF000000"/></x14:dataBar></x14:cfRule><xm:sqref>B1:B3</xm:sqref></x14:conditionalFormatting>
<x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="iconSet" priority="1" id="{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A00002}}"><x14:iconSet iconSet="3Triangles" custom="1"><x14:cfvo type="percent"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type="num"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type="formula" gte="0"><xm:f>Limites!$A$1</xm:f></x14:cfvo><x14:cfIcon iconSet="3Triangles" iconId="0"/><x14:cfIcon iconSet="NoIcons" iconId="0"/><x14:cfIcon iconSet="3Stars" iconId="2"/></x14:iconSet></x14:cfRule><xm:sqref>B1:B3</xm:sqref></x14:conditionalFormatting>
<x14:conditionalFormatting xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main"><x14:cfRule type="expression" priority="21" id="{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A00003}}"><xm:f>A1&gt;Limites!$A$1</xm:f><x14:dxf><font><b/><color theme="5"/></font><fill><patternFill><bgColor rgb="FFFFFF00"/></patternFill></fill></x14:dxf></x14:cfRule><x14:cfRule type="cellIs" priority="22" operator="lessThan" id="{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A00004}}"><xm:f>[2]Far!$A$1</xm:f><x14:dxf/></x14:cfRule><x14:cfRule type="dataBar" id="{{9C1B6C1E-0F0A-4E6E-9C55-1B63F0A0FFFF}}"><x14:dataBar><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><xm:sqref>A1:A3</xm:sqref></x14:conditionalFormatting>
</x14:conditionalFormattings></ext><ext uri="{{05C60535-1F16-4fd2-B633-F4F36F0B64E0}}" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main"><x14:sparklineGroups/></ext></extLst>
</worksheet>"#
    )
}

fn excel_file(sheet: &str, styles: &str) -> Vec<u8> {
    let workbook = format!(
        r#"<workbook {MAIN}><sheets><sheet name="Notes" sheetId="1" r:id="rId1"/><sheet name="Limites" sheetId="2" r:id="rId2"/></sheets></workbook>"#
    );
    package(&[
        ("xl/workbook.xml", &workbook),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet),
        (
            "xl/worksheets/sheet2.xml",
            r#"<worksheet><sheetData><row r="1"><c r="A1"><v>2</v></c></row></sheetData></worksheet>"#,
        ),
    ])
}

#[test]
fn rules_from_excel() {
    let bytes = excel_file(&excel_sheet(), &excel_styles());
    let wb = load(&bytes);
    let rules = &wb.sheets[0].cond_formats;
    let summary: Vec<String> = rules
        .iter()
        .map(|cf| {
            let cells: Vec<String> = cf.ranges.iter().map(Rect::name).collect();
            format!("{} {}", cf.rule.ooxml_type(), cells.join(" "))
        })
        .collect();
    // In priority order, wherever each rule was stored; the rules that
    // cannot be kept are dropped one by one, the others stay.
    assert_eq!(
        summary,
        [
            "iconSet B1:B3",
            "iconSet B1:B3",
            "timePeriod C1:C3 E1",
            "expression C1:C3 E1",
            "containsText C1:C3 E1",
            "top10 B1:B3",
            "dataBar B1:B3",
            "colorScale A1:A3",
            "cellIs A1:A3",
            "expression C1:C3 E1",
            "uniqueValues D1:D3",
            "expression A1:A3",
            "endsWith C1:C3 E1",
        ]
    );

    // Icons picked one by one, with a threshold on another sheet.
    let Rule::IconSet(icons) = &rules[0].rule else {
        panic!("{:?}", rules[0].rule);
    };
    assert_eq!(icons.set, IconSet::ThreeTriangles);
    assert_eq!(
        icons.custom,
        Some(vec![
            Some((IconSet::ThreeTriangles, 0)),
            None,
            Some((IconSet::ThreeStars, 2))
        ])
    );
    assert_eq!(icons.thresholds[1].value, Cfvo::number(0.0));
    assert_eq!(icons.thresholds[2].value, Cfvo::Formula(f("Limites!$A$1")));
    assert!(icons.thresholds[1].gte && !icons.thresholds[2].gte);
    let Rule::IconSet(arrows) = &rules[1].rule else {
        panic!("{:?}", rules[1].rule);
    };
    assert_eq!(
        (arrows.set, arrows.reverse, arrows.show_value),
        (IconSet::ThreeArrows, true, false)
    );
    assert_eq!(arrows.thresholds[2].value, Cfvo::number(25.0));
    assert!(!arrows.thresholds[2].gte);

    assert_eq!(rules[2].rule, Rule::DateOccurring(TimePeriod::Last7Days));
    assert_eq!(
        rules[2].format,
        HighlightFormat::LightRedFillDarkRedText.patch()
    );
    // A "begins with" whose formula is not one Excel writes for it is
    // kept as the formula it is; the older spelling of the test is known.
    assert_eq!(rules[3].rule, Rule::Formula(f("EXACT(LEFT(C1,2),\"al\")")));
    assert_eq!(
        rules[12].rule,
        Rule::Text {
            op: TextOp::EndsWith,
            text: Expr::Text(Arc::from("ma"))
        }
    );
    assert_eq!(
        rules[3].format,
        StylePatch {
            strike: Some(false),
            underline: Some(Underline::Double),
            font_color: Some(None),
            ..StylePatch::default()
        }
    );
    assert_eq!(
        rules[4].rule,
        Rule::Text {
            op: TextOp::Contains,
            text: Expr::Text(Arc::from("a"))
        }
    );
    // The palette of the file is defined after the formats that use it.
    assert_eq!(rules[4].format.fill, Some(Some(Color(0xABCDEF))));
    assert_eq!(rules[4].format.num_fmt.as_deref(), Some("0.0%"));
    assert_eq!(
        rules[5].rule,
        Rule::TopBottom {
            rank: 20,
            percent: true,
            bottom: true
        }
    );
    // Theme colours: white text on accent 1, darker 25 %.
    let top = &rules[5].format;
    assert_eq!((top.bold, top.italic), (Some(true), Some(true)));
    let plain = |color: Option<Option<Color>>| color.flatten().map(Color::plain);
    assert_eq!(plain(top.font_color), Some(Color(0xFFFFFF)));
    assert_eq!(plain(top.fill), Some(Color(0x305496)));
    assert!(top.fill.flatten().is_some_and(Color::is_themed));
    assert_eq!(
        [
            top.border_left,
            top.border_right,
            top.border_top,
            top.border_bottom
        ],
        [Some(Border::thin()); 4]
    );

    // The two halves of the data bar made one rule.
    assert_eq!(
        rules[6].rule,
        Rule::DataBar(DataBar::preset(BarColor::Blue, true))
    );
    let Rule::ColorScale(scale) = &rules[7].rule else {
        panic!("{:?}", rules[7].rule);
    };
    let colors: Vec<Color> = scale.stops.iter().map(|s| s.color.plain()).collect();
    assert_eq!(colors, [Color(0xF8696B), Color(0xFFFFFF), Color(0x8EA9DB)]);
    assert_eq!(scale.stops[1].at, Cfvo::percentile(50.0));
    assert_eq!(rules[8].rule, Rule::greater_than(f("2")));
    // A format the file does not have is no format.
    assert_eq!(rules[9].rule, Rule::Formula(f("LEN(C1)>4")));
    assert!(rules[9].stop_if_true && rules[9].format.is_empty());
    assert_eq!(rules[10].rule, Rule::UniqueValues);
    // A rule of the extension list with its format inline.
    assert_eq!(rules[11].rule, Rule::Formula(f("A1>Limites!$A$1")));
    assert_eq!(
        rules[11].format,
        StylePatch {
            bold: Some(true),
            // The second accent of the theme, as a colour of the theme.
            font_color: Some(Some(wb.theme().accent(2, 0.0))),
            fill: Some(Some(Color(0xFFFF00))),
            ..StylePatch::default()
        }
    );

    // They format the cells: A3 (3) is above both 2 and Limites!A1.
    let shown = result_at(&wb, 0, 2, 0).expect("A3 is formatted");
    assert_eq!(shown.patch.font_color, Some(Some(Color(0x9C0006))));
    assert_eq!(shown.patch.bold, Some(true));
    let bar = result_at(&wb, 0, 1, 1)
        .and_then(|r| r.adornment)
        .and_then(|a| a.bar)
        .expect("B2 has a bar");
    assert!(bar.negative && bar.fill == Color(0xFF0000));
    assert_eq!(
        wb.sheets[0].cells.len(),
        9,
        "nothing else came out of the extension list"
    );

    // Saved again and read again, the rules are the same.
    let saved = to_bytes(&wb);
    common::keep_sample("conditional-formats-from-excel.xlsx", &saved);
    let sheet = part(&saved, "xl/worksheets/sheet1.xml");
    assert_worksheet_order("sheet1.xml", &sheet);
    assert_eq!(&load(&saved).sheets[0].cond_formats, rules);
    assert!(sheet.contains("<cfRule type=\"iconSet\" priority=\"2\">"));
    assert!(sheet.contains("<x14:cfRule type=\"iconSet\" priority=\"1\" id="));
}

#[test]
fn damaged_rules_cost_nothing_else() {
    let sheet = excel_sheet();
    let styles = excel_styles();
    // Cut the worksheet short everywhere from the first rule on: the cells
    // are always there, and what was read can always be written.
    let start = sheet.find("<conditionalFormatting").unwrap();
    let mut counts = std::collections::BTreeSet::new();
    for cut in (start..sheet.len())
        .step_by(11)
        .filter(|i| sheet.is_char_boundary(*i))
    {
        let wb = load(&excel_file(&sheet[..cut], &styles));
        assert_eq!(value(&wb, 0, "C3"), Value::text("gamma"));
        counts.insert(wb.sheets[0].cond_formats.len());
        let saved = to_bytes(&wb);
        assert_well_formed("sheet1.xml", &part(&saved, "xl/worksheets/sheet1.xml"));
        assert_eq!(
            load(&saved).sheets[0].cond_formats.len(),
            wb.sheets[0].cond_formats.len()
        );
    }
    assert!(
        counts.len() >= 10,
        "rules are lost a few at a time: {counts:?}"
    );
    // The same for the formats.
    let start = styles.find("<dxfs").unwrap();
    for cut in (start..styles.len())
        .step_by(7)
        .filter(|i| styles.is_char_boundary(*i))
    {
        let wb = load(&excel_file(&sheet, &styles[..cut]));
        assert_eq!(wb.sheets[0].cond_formats.len(), 13);
        assert_well_formed("styles.xml", &part(&to_bytes(&wb), "xl/styles.xml"));
    }

    // Nonsense everywhere.
    let nonsense = format!(
        r#"<worksheet {MAIN}><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
<conditionalFormatting sqref="nowhere"><cfRule type="duplicateValues" priority="1"/></conditionalFormatting>
<conditionalFormatting sqref="A1 A1 ZZZ1 B2:A1">
<cfRule/>
<cfRule type="cellIs" priority="-4" operator="equal" dxfId="-1"><formula></formula></cfRule>
<cfRule type="cellIs" priority="0" operator="equal"><formula>1</formula><formula>((</formula></cfRule>
<cfRule type="expression" priority="x"><formula>{deep}</formula></cfRule>
<cfRule type="top10" priority="3" rank="99999999" percent="yes"/>
<cfRule type="aboveAverage" priority="2" stdDev="12" aboveAverage="2"/>
<cfRule type="colorScale" priority="4"><colorScale><cfvo type="num" val="x y"/><cfvo type="max"/><color rgb="zz"/><color rgb="FF00FF00"/></colorScale></cfRule>
<cfRule type="colorScale" priority="5"><colorScale><cfvo type="autoMin"/><cfvo type="max"/><color rgb="FFFF0000"/><color rgb="FF00FF00"/></colorScale></cfRule>
<cfRule type="colorScale" priority="5"><colorScale><cfvo type="min"/><cfvo type="max"/><color theme="99"/><color rgb="FF00FF00"/></colorScale></cfRule>
<cfRule type="dataBar" priority="6"><dataBar minLength="900" maxLength="-1"><cfvo type="min"/><cfvo type="max"/><color rgb="FF112233"/></dataBar><extLst><ext><x14:id> </x14:id></ext><ext/></extLst></cfRule>
<cfRule type="iconSet" priority="7"><iconSet iconSet="9Lives"><cfvo type="percent" val="0"/></iconSet></cfRule>
<cfRule type="iconSet" priority="8"><iconSet iconSet="3Flags" custom="1"><cfvo type="percent" val="0"/><cfvo type="percent" val="33"/><cfvo type="percent" val="67"/><cfIcon iconSet="3Flags" iconId="7"/><cfIcon iconSet="3Flags" iconId="1"/><cfIcon iconSet="NoIcons" iconId="0"/></iconSet></cfRule>
<cfRule type="iconSet" priority="8"><iconSet iconSet="3Flags"><cfvo type="percent" val="0"/><cfvo type="percent" val="1e400"/><cfvo type="percent" val="67"/></iconSet></cfRule>
<cfRule type="iconSet" priority="9"><colorScale><cfvo type="min"/><cfvo type="max"/><color rgb="FFFF0000"/><color rgb="FF00FF00"/></colorScale></cfRule>
<cfRule type="containsText" priority="10" operator="containsText"/>
<cfRule type="endsWith" priority="11" text="_x000A_z"/>
<conditionalFormatting sqref="C1"><cfRule type="uniqueValues" priority="12"/></conditionalFormatting>
</conditionalFormatting>
<extLst><ext><x14:conditionalFormattings><x14:conditionalFormatting><x14:cfRule type="dataBar" priority="13" id=""><x14:dataBar border="1" axisPosition="sideways" direction="up"><x14:cfvo type="autoMin"/><x14:cfvo type="autoMax"/></x14:dataBar></x14:cfRule><x14:cfRule type="iconSet" id="{{X}}"><x14:iconSet iconSet="3Stars" custom="1"><x14:cfvo type="percent"><xm:f>0</xm:f></x14:cfvo><x14:cfvo type="percent"><xm:f>33</xm:f></x14:cfvo><x14:cfvo type="percent"><xm:f>67</xm:f></x14:cfvo><x14:cfIcon iconSet="3Stars" iconId="0"/></x14:iconSet></x14:cfRule><xm:sqref>D1</xm:sqref><xm:sqref>D2:D3</xm:sqref></x14:conditionalFormatting><x14:conditionalFormatting><x14:cfRule type="expression" priority="14"><xm:f>TRUE</xm:f></x14:cfRule></x14:conditionalFormatting><conditionalFormatting/></x14:conditionalFormattings></ext></extLst>
</worksheet>"#,
        deep = "(".repeat(4000),
    );
    let wb = load(&excel_file(
        &nonsense,
        "<styleSheet><dxfs><dxf><font><sz val=\"-3\"/><color rgb=\"\"/></font></dxf><dxf>",
    ));
    let summary: Vec<String> = wb.sheets[0]
        .cond_formats
        .iter()
        .map(|cf| format!("{} {}", cf.rule.ooxml_type(), cf.ranges[0].name()))
        .collect();
    assert_eq!(
        summary,
        [
            "aboveAverage A1",
            "top10 A1",
            "dataBar A1",
            "iconSet A1",
            "endsWith A1",
            "dataBar D2:D3",
        ]
    );
    let rules = &wb.sheets[0].cond_formats;
    assert_eq!(rules[0].ranges, rects("A1 A1:B2"));
    assert_eq!(
        rules[0].rule,
        Rule::Average {
            above: false,
            or_equal: false,
            std_dev: 3
        }
    );
    assert_eq!(
        rules[1].rule,
        Rule::TopBottom {
            rank: 1000,
            percent: false,
            bottom: false
        }
    );
    let Rule::DataBar(bar) = &rules[2].rule else {
        panic!("{:?}", rules[2].rule);
    };
    assert_eq!((bar.min_length, bar.max_length), (100, 90));
    let Rule::IconSet(icons) = &rules[3].rule else {
        panic!("{:?}", rules[3].rule);
    };
    assert_eq!(icons.custom, None, "an icon that does not exist");
    assert_eq!(
        rules[4].rule,
        Rule::Text {
            op: TextOp::EndsWith,
            text: Expr::Text(Arc::from("\nz"))
        }
    );
    let Rule::DataBar(bar) = &rules[5].rule else {
        panic!("{:?}", rules[5].rule);
    };
    assert_eq!(
        (bar.axis, bar.direction),
        (BarAxis::Automatic, BarDirection::Context)
    );
    assert_eq!(bar.border, Some(Color::BLACK));
    // Whatever was read makes a file that reads the same.
    let saved = to_bytes(&wb);
    assert_worksheet_order("sheet1.xml", &part(&saved, "xl/worksheets/sheet1.xml"));
    assert_eq!(&load(&saved).sheets[0].cond_formats, rules);
}

#[test]
fn rules_that_cannot_be_written_are_left_out() {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    let scale = |count: usize| {
        Rule::ColorScale(ColorScale {
            stops: (0..count)
                .map(|_| ColorStop {
                    at: Cfvo::Min,
                    color: Color(0x00FF00),
                })
                .collect(),
        })
    };
    let mut icons = IconRule::preset(IconSet::ThreeArrows);
    icons.thresholds.pop();
    let mut custom = IconRule::preset(IconSet::ThreeFlags);
    custom.custom = Some(vec![None]);
    wb.sheets[0].cond_formats = vec![
        CondFormat::new(Vec::new(), Rule::DuplicateValues),
        rule("A1", scale(1)),
        rule("A1", scale(4)),
        rule("A1", Rule::IconSet(icons)),
        rule("A1", Rule::IconSet(custom)),
        rule("A1", Rule::top_items(5000)).with_format(fill(0xFF0000)),
        // A sheet that is gone: its cells are `#REF!`, which no longer
        // names a sheet.
        rule("A1", Rule::Formula(f("Sheet2!$A$1>0"))),
        // Stop-if-true and a format mean nothing on a graded rule.
        rule("A1", scale(2))
            .with_format(fill(0x0000FF))
            .with_stop_if_true(true),
    ];
    assert!(wb.delete_sheet(1));
    wb.commit("rules");
    let bytes = to_bytes(&wb);
    common::keep_sample("conditional-formats-limits.xlsx", &bytes);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert_eq!(sheet.matches("<cfRule ").count(), 4, "{sheet}");
    assert!(sheet.contains("<cfRule type=\"iconSet\" priority=\"5\"><iconSet iconSet=\"3Flags\">"));
    assert!(sheet.contains("<cfRule type=\"top10\" dxfId=\"0\" priority=\"6\" rank=\"1000\"/>"));
    assert!(sheet.contains(
        "<cfRule type=\"expression\" priority=\"7\"><formula>#REF!&gt;0</formula></cfRule>"
    ));
    assert!(sheet.contains("<cfRule type=\"colorScale\" priority=\"8\"><colorScale>"));
    assert!(!sheet.contains("extLst") && !sheet.contains("stopIfTrue"));
    assert!(part(&bytes, "xl/styles.xml").contains("<dxfs count=\"1\">"));
    assert_eq!(load(&bytes).sheets[0].cond_formats.len(), 4);
}

/// Everything a sheet keeps after its cells, at once: the rules before
/// the page setup, the extension list of rules and validation after it.
#[test]
fn rules_and_page_setup_share_a_sheet() {
    let mut wb = sample();
    assert!(wb.set_print_area(0, &rects("A1:G20")));
    assert!(wb.set_print_titles(0, Some((0, 0)), Some((0, 0))));
    assert!(wb.insert_page_break(0, Some(10), Some(4)));
    assert!(wb.update_page_setup(0, |page| {
        page.paper = Some(PaperSize::A4);
        page.orientation = Orientation::Landscape;
        page.fit_to_page = true;
        page.fit_height = 0;
        page.gridlines = true;
        page.center_horizontally = true;
        page.margins = Margins::NARROW;
        let hf = &mut page.header_footer;
        hf.odd_header = "&L&A&C&\"Arial,Bold\"&14Scores&R&D".into();
        hf.odd_footer = "&CPage &P of &N".into();
        hf.first_header = "&CFirst page".into();
        hf.different_first = true;
    }));
    assert!(wb.set_print_area(1, &rects("A1:B1")));
    wb.commit("page setup");

    let bytes = to_bytes(&wb);
    common::keep_sample("conditional-formats-and-page-setup.xlsx", &bytes);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert_well_formed("sheet1.xml", &sheet);
    assert_worksheet_order("sheet1.xml", &sheet);
    let order = [
        "<pageSetUpPr fitToPage=\"1\"/></sheetPr>",
        "</sheetData>",
        "<conditionalFormatting ",
        "<printOptions ",
        "<pageMargins ",
        "<pageSetup ",
        "<headerFooter ",
        "<rowBreaks ",
        "<colBreaks ",
        "<x14:conditionalFormattings>",
        "<x14:dataValidations ",
    ];
    let places: Vec<usize> = order
        .iter()
        .map(|tag| sheet.find(tag).unwrap_or_else(|| panic!("no {tag}")))
        .collect();
    assert!(places.is_sorted(), "{places:?}");
    assert!(part(&bytes, "xl/workbook.xml").contains(concat!(
        "<definedName name=\"_xlnm.Print_Area\" localSheetId=\"1\">",
        "'Limits &amp; more'!$A$1:$B$1</definedName>"
    )));

    let loaded = load(&bytes);
    for (a, b) in wb.sheets.iter().zip(&loaded.sheets) {
        assert_eq!(a.cond_formats, b.cond_formats, "{}", a.name);
        assert_eq!(a.validations, b.validations, "{}", a.name);
        assert_eq!(a.page, b.page, "{}", a.name);
    }
}
