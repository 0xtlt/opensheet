//! Themes in `.xlsx` files: the theme of a workbook is written and read
//! back, colours and fonts that refer to it stay references, and a file
//! with a theme that is none of the built-in ones keeps it.

mod common;

use common::{assert_well_formed, at, enter, keep_sample, package, part};
use opensheet_engine::chart::ChartKind;
use opensheet_engine::condfmt::{Cfvo, ColorScale, ColorStop, CondFormat, Rule};
use opensheet_engine::drawing::{Frame, ShapeTool};
use opensheet_engine::sparkline::SparklineType;
use opensheet_engine::style::{Border, BorderStyle, StylePatch, ThemeFont, ThemeSlot};
use opensheet_engine::theme::{Theme, ThemeColors, ThemeFonts};
use opensheet_engine::{Color, Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn style<'a>(wb: &'a Workbook, name: &str) -> &'a opensheet_engine::Style {
    let (row, col) = at(name);
    wb.style_at(0, row, col)
}

/// A theme as the workbook has it, without the part it was read from.
fn bare(theme: &Theme) -> Theme {
    Theme {
        source: None,
        ..theme.clone()
    }
}

/// A workbook in the theme Facet with a reference to the theme wherever
/// one can be kept.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [
        ("A1", "1"),
        ("A2", "2"),
        ("B1", "3"),
        ("B2", "4"),
        ("D1", "themed"),
        ("D2", "standard"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    wb.set_theme(Theme::builtin("Facet").unwrap());
    let theme = wb.theme().clone();
    wb.update_style(0, rect("D1"), &|s| {
        s.fill = Some(theme.accent(2, 0.4));
        s.font.color = Some(theme.color(ThemeSlot::Light1, 0.0));
        s.font.name = theme.font(ThemeFont::Major).clone();
        s.font.theme = Some(ThemeFont::Major);
        s.borders.bottom = Border {
            style: BorderStyle::Thin,
            color: Some(theme.accent(1, -0.25)),
        };
    });
    wb.update_style(0, rect("D2"), &|s| {
        s.fill = Some(Color(0xFFC000));
        s.font.color = Some(Color(0x90C226));
        s.font.name = "Georgia".into();
    });
    wb.apply_cell_style(0, &[rect("D3")], "Accent3").unwrap();
    wb.set_tab_color(0, Some(theme.accent(6, 0.0)));
    let format = StylePatch {
        font_color: Some(Some(theme.accent(5, -0.5))),
        fill: Some(Some(theme.accent(5, 0.8))),
        ..StylePatch::default()
    };
    let greater =
        Rule::greater_than(opensheet_engine::parser::parse_formula("1", Locale::en()).unwrap());
    wb.add_cond_format(
        0,
        CondFormat::new(vec![rect("A1:A2")], greater).with_format(format),
    );
    let scale = ColorScale {
        stops: vec![
            ColorStop {
                at: Cfvo::Min,
                color: theme.color(ThemeSlot::Light1, 0.0),
            },
            ColorStop {
                at: Cfvo::Max,
                color: theme.accent(4, 0.0),
            },
        ],
    };
    wb.add_cond_format(
        0,
        CondFormat::new(vec![rect("B1:B2")], Rule::ColorScale(scale)),
    );
    wb.add_shape(
        0,
        ShapeTool::Rectangle,
        Frame::new(300.0, 100.0, 90.0, 40.0),
    )
    .unwrap();
    let mut chart = wb.chart_from_range(0, rect("A1:B2"), ChartKind::Column);
    chart.series[0].color = Some(theme.accent(3, 0.6));
    chart.series[1].color = Some(Color(0x123456));
    wb.add_chart(0, chart);
    wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:B2")), rect("F1:F2"))
        .unwrap();
    let series = theme.accent(5, -0.5);
    wb.update_sparkline_groups(0, &[rect("F1")], |group| group.colors.series = series);
    wb.commit("setup");
    wb
}

#[test]
fn a_workbook_keeps_its_theme_and_its_references() {
    let wb = sample();
    let facet = Theme::builtin("Facet").unwrap();
    let bytes = to_bytes(&wb);
    keep_sample("themes-facet.xlsx", &bytes);

    // The theme of the workbook is the theme of the package.
    let part_xml = part(&bytes, "xl/theme/theme1.xml");
    assert_well_formed("theme1.xml", &part_xml);
    assert!(
        part_xml.contains("name=\"Facet\"><a:themeElements><a:clrScheme name=\"Facet\">"),
        "{part_xml}"
    );
    assert!(part_xml.contains("<a:accent1><a:srgbClr val=\"90C226\"/></a:accent1>"));
    assert!(part_xml.contains(
        "<a:minorFont><a:latin typeface=\"Trebuchet MS\" panose=\"020B0603020202020204\"/>"
    ));
    assert!(part_xml.contains("<a:fmtScheme name=\"Facet\">"));

    // Cells: theme and tint where the colour is of the theme, the font of
    // the theme by its role.
    let styles = part(&bytes, "xl/styles.xml");
    assert!(
        styles.contains("<fgColor theme=\"5\" tint=\"0.3999755851924192\"/>"),
        "{styles}"
    );
    assert!(styles.contains("<fgColor rgb=\"FFFFC000\"/>"));
    assert!(
        styles.contains(
            "<color theme=\"0\"/><name val=\"Trebuchet MS\"/><scheme val=\"major\"/></font>"
        ),
        "{styles}"
    );
    assert!(
        styles.contains("<color rgb=\"FF90C226\"/><name val=\"Georgia\"/></font>"),
        "{styles}"
    );
    assert!(
        styles
            .contains("<sz val=\"11\"/><name val=\"Trebuchet MS\"/><scheme val=\"minor\"/></font>")
    );
    assert!(styles.contains(
        "<bottom style=\"thin\"><color theme=\"4\" tint=\"-0.249977111117893\"/></bottom>"
    ));
    // The differential format of the rule.
    assert!(styles.contains("<dxf><font><color theme=\"8\" tint=\"-0.499984740745262\"/></font><fill><patternFill><bgColor theme=\"8\" tint=\"0.7999816888943144\"/></patternFill></fill></dxf>"), "{styles}");
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<tabColor theme=\"9\"/>"), "{sheet}");
    assert!(
        sheet.contains("<color theme=\"0\"/><color theme=\"7\"/></colorScale>"),
        "{sheet}"
    );
    assert!(
        sheet.contains("<x14:colorSeries theme=\"8\" tint=\"-0.499984740745262\"/>"),
        "{sheet}"
    );
    // Shapes and charts: the name of the colour and its lightness.
    let drawing = part(&bytes, "xl/drawings/drawing1.xml");
    assert!(
        drawing.contains("<a:solidFill><a:schemeClr val=\"accent1\"/></a:solidFill>"),
        "{drawing}"
    );
    let chart = part(&bytes, "xl/charts/chart1.xml");
    assert!(chart.contains("<a:solidFill><a:schemeClr val=\"accent3\"><a:lumMod val=\"40000\"/><a:lumOff val=\"60000\"/></a:schemeClr></a:solidFill>"), "{chart}");
    assert!(chart.contains("<a:solidFill><a:srgbClr val=\"123456\"/></a:solidFill>"));

    // Read back: the same theme, the same colours, references included.
    let read = load(&bytes);
    assert_eq!(bare(read.theme()), facet);
    for cell in ["D1", "D2", "D3", "A1"] {
        assert_eq!(style(&read, cell), style(&wb, cell), "{cell}");
    }
    assert_eq!(style(&read, "D1").fill, Some(facet.accent(2, 0.4)));
    assert_eq!(style(&read, "D2").font.color, Some(Color(0x90C226)));
    assert_eq!(style(&read, "D1").font.theme, Some(ThemeFont::Major));
    assert_eq!(read.cell_style_at(0, 2, 3), "Accent3");
    assert_eq!(read.sheets[0].tab_color, wb.sheets[0].tab_color);
    assert_eq!(read.sheets[0].cond_formats, wb.sheets[0].cond_formats);
    assert_eq!(
        read.sheets[0].drawings[0].as_shape(),
        wb.sheets[0].drawings[0].as_shape()
    );
    assert_eq!(
        read.sheets[0].charts[0].series[0].color,
        Some(facet.accent(3, 0.6))
    );
    assert_eq!(
        read.sheets[0].charts[0].series[1].color,
        Some(Color(0x123456))
    );
    assert_eq!(
        read.sheets[0].sparklines[0].colors,
        wb.sheets[0].sparklines[0].colors
    );
    assert_eq!(read.font_metrics(), wb.font_metrics());
    assert_eq!(
        read.sheets[0].default_col_width,
        wb.sheets[0].default_col_width
    );

    // What was read follows another theme; what is not of the theme stays.
    let mut read = read;
    read.set_theme(Theme::office());
    read.commit("Theme");
    let office = Theme::office();
    assert_eq!(style(&read, "D1").fill, Some(office.accent(2, 0.4)));
    assert_eq!(&*style(&read, "D1").font.name, "Aptos Display");
    assert_eq!(&*style(&read, "A1").font.name, "Aptos Narrow");
    assert_eq!(style(&read, "D2").fill, Some(Color(0xFFC000)));
    assert_eq!(&*style(&read, "D2").font.name, "Georgia");
    assert_eq!(read.sheets[0].tab_color, Some(office.accent(6, 0.0)));
    assert_eq!(
        read.sheets[0].charts[0].series[0].color,
        Some(office.accent(3, 0.6))
    );
    assert_eq!(
        read.sheets[0].drawings[0].as_shape().unwrap().fill,
        Some(office.accent(1, 0.0))
    );
    assert_eq!(
        read.sheets[0].sparklines[0].colors.series,
        office.accent(5, -0.5)
    );
    // Saved again, it is the Office theme that is written.
    let again = to_bytes(&read);
    assert!(
        part(&again, "xl/theme/theme1.xml")
            .contains("name=\"Office Theme\"><a:themeElements><a:clrScheme name=\"Office\">")
    );
    assert_eq!(bare(load(&again).theme()), office);
}

#[test]
fn a_theme_of_ones_own_and_its_effects() {
    let mut wb = Workbook::new(Locale::en());
    enter(&mut wb, 0, "A1", "x");
    let mut colors = wb.theme().colors.clone();
    colors.name = "Mine & yours".to_string();
    colors.set(ThemeSlot::Accent1, 0xAA0044);
    colors.set(ThemeSlot::Dark1, 0x202020);
    wb.set_theme_colors(colors.clone());
    wb.set_theme_fonts(ThemeFonts::new("My fonts", "Georgia", "Verdana"));
    wb.set_theme_effects("Glossy");
    let accent = wb.theme().accent(1, 0.0);
    wb.update_style(0, rect("A1"), &|s| s.fill = Some(accent));
    wb.commit("setup");

    let bytes = to_bytes(&wb);
    keep_sample("themes-custom.xlsx", &bytes);
    let part_xml = part(&bytes, "xl/theme/theme1.xml");
    assert_well_formed("theme1.xml", &part_xml);
    assert!(
        part_xml.contains(
            "<a:clrScheme name=\"Mine &amp; yours\"><a:dk1><a:srgbClr val=\"202020\"/></a:dk1>"
        ),
        "{part_xml}"
    );
    assert!(
        part_xml
            .contains("<a:fontScheme name=\"My fonts\"><a:majorFont><a:latin typeface=\"Georgia\"")
    );
    assert!(part_xml.contains("<a:fmtScheme name=\"Glossy\">"));
    let read = load(&bytes);
    assert_eq!(bare(read.theme()), *wb.theme());
    assert_eq!(read.theme().colors, colors);
    assert_eq!(style(&read, "A1").fill.map(|c| c.value()), Some(0xAA0044));
    assert_eq!(&*style(&read, "A1").font.name, "Verdana");
    assert_eq!(style(&read, "A1").font.theme, Some(ThemeFont::Minor));
    // Text 1 of this theme is not black: it is a colour like another.
    assert_eq!(read.theme().rgb(ThemeSlot::Dark1), 0x202020);
}

const MAIN: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

/// The theme part of a workbook Excel saved in a theme of a company:
/// fonts for other scripts, effects of its own, an extension list.
const COMPANY_THEME: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Company\">",
    "<a:themeElements><a:clrScheme name=\"Company\">",
    "<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>",
    "<a:dk2><a:srgbClr val=\"003057\"/></a:dk2><a:lt2><a:srgbClr val=\"F2F2F2\"/></a:lt2>",
    "<a:accent1><a:srgbClr val=\"E4002B\"/></a:accent1><a:accent2><a:srgbClr val=\"00A3E0\"/></a:accent2>",
    "<a:accent3><a:srgbClr val=\"84BD00\"/></a:accent3><a:accent4><a:srgbClr val=\"FFB81C\"/></a:accent4>",
    "<a:accent5><a:srgbClr val=\"753BBD\"/></a:accent5><a:accent6><a:srgbClr val=\"00B2A9\"/></a:accent6>",
    "<a:hlink><a:srgbClr val=\"0000EE\"/></a:hlink><a:folHlink><a:srgbClr val=\"551A8B\"/></a:folHlink>",
    "</a:clrScheme><a:fontScheme name=\"Company\">",
    "<a:majorFont><a:latin typeface=\"Georgia\" panose=\"02040502050405020303\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>",
    "<a:font script=\"Jpan\" typeface=\"游ゴシック Light\"/></a:majorFont>",
    "<a:minorFont><a:latin typeface=\"Verdana\" panose=\"020B0604030504040204\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>",
    "<a:font script=\"Jpan\" typeface=\"游ゴシック\"/></a:minorFont>",
    "</a:fontScheme><a:fmtScheme name=\"Company\"><a:fillStyleLst>",
    "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    "<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"50000\"/></a:schemeClr></a:solidFill>",
    "<a:solidFill><a:srgbClr val=\"ABCDEF\"/></a:solidFill>",
    "</a:fillStyleLst><a:lnStyleLst>",
    "<a:ln w=\"6350\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>",
    "<a:ln w=\"12700\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>",
    "<a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>",
    "</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle>",
    "<a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>",
    "<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    "</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/>",
    "<a:extLst><a:ext uri=\"{05A4C25C-085E-4340-85A3-A5531E510DB2}\">",
    "<thm15:themeFamily xmlns:thm15=\"http://schemas.microsoft.com/office/thememl/2012/main\" name=\"Company\" id=\"{62F939B6-93AF-4DB8-9C6B-D6C7DFDC589F}\" vid=\"{4A3C46E8-61CC-4603-A589-7422A47A8E4A}\"/>",
    "</a:ext></a:extLst></a:theme>",
);

/// The styles of that workbook, the way Excel writes them: the default
/// font is the body font, the cells use colours of the theme.
const COMPANY_STYLES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    "<fonts count=\"3\">",
    "<font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Verdana\"/><family val=\"2\"/><scheme val=\"minor\"/></font>",
    "<font><b/><sz val=\"14\"/><color theme=\"4\" tint=\"-0.249977111117893\"/><name val=\"Georgia\"/><family val=\"1\"/><scheme val=\"major\"/></font>",
    "<font><sz val=\"11\"/><color rgb=\"FF00B050\"/><name val=\"Arial\"/><family val=\"2\"/></font>",
    "</fonts>",
    "<fills count=\"4\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill>",
    "<fill><patternFill patternType=\"solid\"><fgColor theme=\"5\" tint=\"0.79998168889431442\"/><bgColor indexed=\"64\"/></patternFill></fill>",
    "<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFE4002B\"/><bgColor indexed=\"64\"/></patternFill></fill></fills>",
    "<borders count=\"2\"><border><left/><right/><top/><bottom/><diagonal/></border>",
    "<border><left/><right/><top/><bottom style=\"medium\"><color theme=\"3\"/></bottom><diagonal/></border></borders>",
    "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
    "<cellXfs count=\"3\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>",
    "<xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\"/>",
    "<xf numFmtId=\"0\" fontId=\"2\" fillId=\"3\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\"/></cellXfs>",
    "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>",
);

fn company_workbook() -> Vec<u8> {
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN}><sheetPr><tabColor theme="7"/></sheetPr><sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>Title</t></is></c><c r="B1" s="2"><v>2</v></c><c r="C1"><v>3</v></c></row></sheetData></worksheet>"#
    );
    package(&[
        (
            "xl/workbook.xml",
            &format!(
                r#"<workbook {MAIN}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        ("xl/theme/theme1.xml", COMPANY_THEME),
        ("xl/styles.xml", COMPANY_STYLES),
        ("xl/worksheets/sheet1.xml", &sheet),
    ])
}

#[test]
fn a_file_from_excel_in_another_theme() {
    let wb = load(&company_workbook());
    let theme = wb.theme().clone();
    assert_eq!(
        (
            theme.name.as_str(),
            theme.colors.name.as_str(),
            theme.effects.as_str()
        ),
        ("Company", "Company", "Company")
    );
    assert_eq!(theme.rgb(ThemeSlot::Accent1), 0xE4002B);
    assert_eq!(
        (&*theme.fonts.major, &*theme.fonts.minor),
        ("Georgia", "Verdana")
    );
    assert!(Theme::builtin(&theme.name).is_none());

    // The right colours, and they are colours of the theme.
    let a1 = style(&wb, "A1");
    assert_eq!(a1.fill, Some(theme.accent(2, 0.8)));
    assert_eq!(
        a1.fill.unwrap().value(),
        opensheet_engine::theme::tint(Color(0x00A3E0), 0.79998168889431442).value()
    );
    assert_eq!(a1.font.color, Some(theme.accent(1, -0.25)));
    assert_eq!(
        (&*a1.font.name, a1.font.theme),
        ("Georgia", Some(ThemeFont::Major))
    );
    assert_eq!(a1.borders.bottom.color.map(|c| c.value()), Some(0x003057));
    // A colour and a font given outright are not: the red of B1 is the
    // value of accent 1, and no reference to it.
    let b1 = style(&wb, "B1");
    assert_eq!(b1.fill, Some(Color(0xE4002B)));
    assert_eq!(
        (b1.font.color, &*b1.font.name, b1.font.theme),
        (Some(Color(0x00B050)), "Arial", None)
    );
    // Plain text is in the body font, automatic.
    let c1 = style(&wb, "C1");
    assert_eq!(
        (&*c1.font.name, c1.font.theme, c1.font.color),
        ("Verdana", Some(ThemeFont::Minor), None)
    );
    assert_eq!(wb.sheets[0].tab_color, Some(theme.accent(4, 0.0)));

    // Saved again: the theme part to the letter, references as references.
    let bytes = to_bytes(&wb);
    keep_sample("themes-company-saved.xlsx", &bytes);
    assert_eq!(part(&bytes, "xl/theme/theme1.xml"), COMPANY_THEME);
    let styles = part(&bytes, "xl/styles.xml");
    assert!(
        styles.contains("<fgColor theme=\"5\" tint=\"0.7999816888943144\"/>"),
        "{styles}"
    );
    assert!(styles.contains("<color theme=\"4\" tint=\"-0.249977111117893\"/><name val=\"Georgia\"/><scheme val=\"major\"/>"));
    assert!(styles.contains("<fgColor rgb=\"FFE4002B\"/>"));
    assert!(styles.contains("<name val=\"Arial\"/></font>"));
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").contains("<tabColor theme=\"7\"/>"));
    let again = load(&bytes);
    assert_eq!(*again.theme(), theme);
    for cell in ["A1", "B1", "C1"] {
        assert_eq!(style(&again, cell), style(&wb, cell), "{cell}");
    }

    // Recoloured here, the rest of the theme part stays what it was.
    let mut wb = wb;
    let blue = Theme::builtin_colors()
        .into_iter()
        .find(|c| c.name == "Blue Warm")
        .unwrap();
    wb.set_theme_colors(blue);
    wb.commit("Theme Colors");
    assert_eq!(
        style(&wb, "A1").fill.map(|c| c.value()),
        Some(wb.theme().accent(2, 0.8).value())
    );
    assert_eq!(style(&wb, "B1").fill, Some(Color(0xE4002B)));
    let bytes = to_bytes(&wb);
    let written = part(&bytes, "xl/theme/theme1.xml");
    assert_well_formed("theme1.xml", &written);
    assert!(
        written.contains("<a:clrScheme name=\"Blue Warm\">")
            && written.contains("<a:accent1><a:srgbClr val=\"4A66AC\"/></a:accent1>")
    );
    assert!(written.contains("<a:font script=\"Jpan\" typeface=\"游ゴシック\"/>"));
    assert!(
        written.contains("thm15:themeFamily") && written.contains("<a:srgbClr val=\"ABCDEF\"/>")
    );
    assert_eq!(bare(load(&bytes).theme()), bare(wb.theme()));
}

#[test]
fn files_without_a_theme_or_with_a_damaged_one() {
    // No theme part: the Office theme of the years such files were made.
    let bytes = company_workbook();
    let parts: Vec<(String, String)> = [
        "xl/workbook.xml",
        "xl/styles.xml",
        "xl/worksheets/sheet1.xml",
    ]
    .iter()
    .map(|name| (name.to_string(), part(&bytes, name)))
    .collect();
    let without: Vec<(&str, &str)> = parts
        .iter()
        .map(|(n, c)| (n.as_str(), c.as_str()))
        .collect();
    let wb = load(&package(&without));
    assert_eq!(wb.theme().name, "Office 2013 - 2022 Theme");
    assert_eq!(
        style(&wb, "A1").fill,
        Some(Theme::office_2013().accent(2, 0.8))
    );
    // The fonts are named as the file names them; they are not those of
    // this theme, and so no fonts of the theme.
    assert_eq!(
        (&*style(&wb, "C1").font.name, style(&wb, "C1").font.theme),
        ("Verdana", None)
    );

    // A theme part cut short anywhere is read as far as it goes.
    for cut in (0..COMPANY_THEME.len())
        .filter(|i| COMPANY_THEME.is_char_boundary(*i))
        .step_by(37)
    {
        let mut damaged = without.clone();
        damaged.push(("xl/theme/theme1.xml", &COMPANY_THEME[..cut]));
        let wb = load(&package(&damaged));
        let bytes = to_bytes(&wb);
        assert_well_formed("theme1.xml", &part(&bytes, "xl/theme/theme1.xml"));
        assert_eq!(bare(load(&bytes).theme()), bare(wb.theme()), "cut at {cut}");
    }
    // Colours that name no colour of the theme are no colours.
    let odd = COMPANY_STYLES.replace("theme=\"5\"", "theme=\"12\"");
    let mut parts = without.clone();
    parts[1] = ("xl/styles.xml", &odd);
    parts.push(("xl/theme/theme1.xml", COMPANY_THEME));
    assert_eq!(style(&load(&package(&parts)), "A1").fill, None);
    // A theme made here with odd names is still a part that reads.
    let mut wb = Workbook::new(Locale::en());
    wb.set_theme(Theme {
        name: "<\"a\" & b>".to_string(),
        colors: ThemeColors::from_part_order("c > d", [0x10; 12]),
        fonts: ThemeFonts::new("e \"f\"", "A&B", "C<D"),
        effects: "g & h".to_string(),
        source: None,
    });
    wb.commit("Theme");
    let bytes = to_bytes(&wb);
    assert_well_formed("theme1.xml", &part(&bytes, "xl/theme/theme1.xml"));
    assert_eq!(bare(load(&bytes).theme()), *wb.theme());
}
