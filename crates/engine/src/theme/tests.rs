use super::*;
use crate::chart::{Chart, ChartKind};
use crate::condfmt::{Cfvo, ColorScale, ColorStop, CondFormat};
use crate::drawing::{Frame, ShapeTool};
use crate::refs::Rect;
use crate::style::StyleId;
use crate::tests::wb;

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap()
}

fn facet() -> Theme {
    Theme::builtin("Facet").unwrap()
}

#[test]
fn the_built_in_themes() {
    assert_eq!(builtin::THEMES.len(), 29);
    assert_eq!(Theme::builtin_colors().len(), 24);
    assert_eq!(Theme::builtin_fonts().len(), 26);
    assert_eq!(builtin::EFFECTS.len(), 15);

    let office = Theme::office();
    assert_eq!(office.name, "Office Theme");
    assert_eq!((office.colors.name.as_str(), office.fonts.name.as_str()), ("Office", "Office"));
    assert_eq!(
        office.colors.part_order(),
        [
            0x000000, 0xFFFFFF, 0x0E2841, 0xE8E8E8, 0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93, 0x4EA72E,
            0x467886, 0x96607D
        ]
    );
    assert_eq!((&*office.fonts.major, &*office.fonts.minor), ("Aptos Display", "Aptos Narrow"));
    assert!(office.is_office() && !facet().is_office());
    // Cell colours count the light colours first.
    assert_eq!(office.rgb(ThemeSlot::Light1), 0xFFFFFF);
    assert_eq!(office.rgb(ThemeSlot::Dark2), 0x0E2841);
    assert_eq!(office.rgb(ThemeSlot::Accent1), 0x156082);
    assert_eq!(office.rgb(ThemeSlot::FollowedHyperlink), 0x96607D);

    let facet = facet();
    assert_eq!(facet.rgb(ThemeSlot::Accent1), 0x90C226);
    assert_eq!(&*facet.fonts.minor, "Trebuchet MS");
    let old = Theme::office_2013();
    assert_eq!((old.rgb(ThemeSlot::Accent1), &*old.fonts.minor), (0x4472C4, "Calibri"));
    assert!(Theme::builtin("No such theme").is_none());

    // Every name is there once.
    let mut names: Vec<&str> = builtin::THEMES.iter().map(|t| t.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 29);
    let colors = Theme::builtin_colors();
    assert_eq!(colors[0].name, "Office");
    assert_eq!(colors[1].name, "Office 2013 - 2022");
    assert_eq!(colors[3].name, "Grayscale");
    assert_eq!(colors[3].swatches(), [0x000000, 0xF8F8F8, 0xDDDDDD, 0xB2B2B2, 0x969696, 0x808080, 0x5F5F5F, 0x4D4D4D]);
    let fonts = Theme::builtin_fonts();
    assert_eq!((fonts[0].name.as_str(), &*fonts[0].minor), ("Office", "Aptos Narrow"));
    assert_eq!((&*fonts[2].major, &*fonts[2].minor), ("Cambria", "Calibri"));
}

#[test]
fn the_variants_of_a_picker_are_excels() {
    // The palette Excel shows for the Office theme of 2013 to 2022.
    let theme = Theme::office_2013();
    let palette = theme.palette();
    assert_eq!(palette.len(), 6);
    let column = |col: usize| -> Vec<String> { palette.iter().map(|row| row[col].to_hex()).collect() };
    assert_eq!(column(0), ["FFFFFF", "F2F2F2", "D9D9D9", "BFBFBF", "A6A6A6", "808080"]);
    assert_eq!(column(1), ["000000", "808080", "595959", "404040", "262626", "0D0D0D"]);
    assert_eq!(column(2), ["E7E6E6", "D0CECE", "AEAAAA", "757171", "3A3838", "161616"]);
    assert_eq!(column(3), ["44546A", "D6DCE4", "ACB9CA", "8497B0", "333F4F", "222B35"]);
    assert_eq!(column(4), ["4472C4", "D9E1F2", "B4C6E7", "8EA9DB", "305496", "203764"]);
    assert_eq!(column(9), ["70AD47", "E2EFDA", "C6E0B4", "A9D08E", "548235", "375623"]);

    // Which variants there are depends on how light the colour is.
    assert_eq!(picker_tints(0xFFFFFF), [-0.05, -0.15, -0.25, -0.35, -0.5]);
    assert_eq!(picker_tints(0x000000), [0.5, 0.35, 0.25, 0.15, 0.05]);
    assert_eq!(picker_tints(0x0E2841), [0.9, 0.75, 0.5, 0.25, 0.1]);
    assert_eq!(picker_tints(0xE8E8E8), [-0.1, -0.25, -0.5, -0.75, -0.9]);
    assert_eq!(picker_tints(0x156082), [0.8, 0.6, 0.4, -0.25, -0.5]);
    assert_eq!(tint_percent(-0.249977111117893), -25);

    // Every swatch is a reference to its column, with the tint files
    // write for it.
    let lighter = palette[3][4].theme().unwrap();
    assert_eq!(lighter.slot, ThemeSlot::Accent1);
    assert_eq!(lighter.tint, 13106.0 / 32767.0);
    assert_eq!(format!("{}", lighter.tint), "0.3999755851924192");
    let darker = palette[4][0].theme().unwrap();
    assert_eq!((darker.slot, darker.tint), (ThemeSlot::Light1, -11468.0 / 32767.0));
}

#[test]
fn a_theme_colour_is_a_reference_and_a_value() {
    let office = Theme::office();
    let accent = office.accent(1, 0.0);
    assert_eq!(accent.value(), 0x156082);
    assert!(accent.is_themed());
    // The same value picked under "Standard Colors" is another colour.
    assert_ne!(accent, Color(0x156082));
    assert_eq!(accent.plain(), Color(0x156082));
    assert_eq!(Color(0x156082).theme(), None);

    // Worked out again in another theme; a plain colour never moves.
    let facet = facet();
    assert_eq!(facet.resolve(accent).value(), 0x90C226);
    assert_eq!(facet.resolve(accent).theme(), accent.theme());
    assert_eq!(facet.resolve(Color(0x156082)), Color(0x156082));
    let light = office.accent(1, 0.8);
    assert_eq!(facet.resolve(light), facet.accent(1, 0.8));
    assert_eq!(office.resolve(facet.resolve(light)), light);
    // A tint from a file comes back digit for digit.
    let from_file = office.color(ThemeSlot::Accent2, -0.499984740745262);
    assert_eq!(from_file.theme().unwrap().tint, -0.499984740745262);
    assert_eq!(from_file, office.color(ThemeSlot::Accent2, -0.5));
}

#[test]
fn changing_the_theme_recolours_and_refonts_the_cells() {
    let mut w = wb(&[("A1", "themed"), ("A2", "standard"), ("A3", "heading"), ("A4", "arial"), ("A5", "plain")]);
    let office = w.theme().clone();
    let accent = office.accent(2, 0.4);
    let line = office.color(ThemeSlot::Dark2, 0.0);
    w.update_style(0, rect("A1"), &|s| {
        s.fill = Some(accent);
        s.font.color = Some(office.accent(1, -0.25));
        s.borders.bottom = Border {
            style: crate::style::BorderStyle::Thin,
            color: Some(line),
        };
    });
    w.update_style(0, rect("A2"), &|s| {
        s.fill = Some(Color(0xFFC000));
        s.font.color = Some(Color(0x156082));
    });
    w.update_style(0, rect("A3"), &|s| {
        s.font.name = office.font(ThemeFont::Major).clone();
        s.font.theme = Some(ThemeFont::Major);
    });
    w.update_style(0, rect("A4"), &|s| s.font.name = Arc::from("Arial"));
    w.commit("format");
    // A font that was given a name of its own is not a theme font.
    assert_eq!(w.style_at(0, 3, 0).font.theme, None);
    assert_eq!(w.style_at(0, 4, 0).font.theme, Some(ThemeFont::Minor));
    let ids: Vec<StyleId> = (0..5).map(|r| w.style_id_at(0, r, 0)).collect();
    let undo_depth = w.undo_label().map(str::to_string);

    let facet = facet();
    w.set_theme(facet.clone());
    assert!(w.commit("Theme"));
    assert_eq!(w.theme().name, "Facet");
    // The cells keep their styles; the styles follow.
    assert_eq!((0..5).map(|r| w.style_id_at(0, r, 0)).collect::<Vec<_>>(), ids);
    let a1 = w.style_at(0, 0, 0).clone();
    assert_eq!(a1.fill, Some(facet.accent(2, 0.4)));
    assert_eq!(a1.fill.unwrap().value(), tint(Color(0x54A021), 13106.0 / 32767.0).value());
    assert_eq!(a1.font.color, Some(facet.accent(1, -0.25)));
    assert_eq!(a1.borders.bottom.color.unwrap().value(), 0x2C3C43);
    let a2 = w.style_at(0, 1, 0);
    assert_eq!((a2.fill, a2.font.color), (Some(Color(0xFFC000)), Some(Color(0x156082))));
    assert_eq!(&*w.style_at(0, 2, 0).font.name, "Trebuchet MS");
    assert_eq!(&*w.style_at(0, 3, 0).font.name, "Arial");
    // The default font is the body font.
    assert_eq!(&*w.style_at(0, 4, 0).font.name, "Trebuchet MS");
    assert_eq!(&*w.styles.default_style().font.name, "Trebuchet MS");
    // A style made now is resolved in the theme of now.
    w.update_style(0, rect("B1"), &|s| s.fill = Some(accent));
    w.commit("fill");
    assert_eq!(w.style_at(0, 0, 1).fill.unwrap().value(), facet.accent(2, 0.4).value());

    // One undo step for the theme.
    assert!(w.undo() && w.undo());
    assert_eq!(w.undo_label().map(str::to_string), undo_depth);
    assert_eq!(*w.theme(), office);
    assert_eq!(w.style_at(0, 0, 0).fill, Some(accent));
    assert_eq!(&*w.style_at(0, 2, 0).font.name, "Aptos Display");
    assert_eq!(&*w.styles.default_style().font.name, "Aptos Narrow");
    assert_eq!(*w.styles.default_style(), Style::default());
    assert!(w.redo());
    assert_eq!(w.theme().name, "Facet");
    assert_eq!(w.style_at(0, 0, 0).fill, Some(facet.accent(2, 0.4)));
    // Interning finds the styles again after the table was worked out anew.
    let again = w.styles.intern(w.style_at(0, 0, 0).clone());
    assert_eq!(again, ids[0]);
}

#[test]
fn colours_and_fonts_change_apart() {
    let mut w = wb(&[("A1", "x")]);
    let office = w.theme().clone();
    w.update_style(0, rect("A1"), &|s| s.fill = Some(office.accent(1, 0.0)));
    w.commit("fill");

    let colors = Theme::builtin_colors().into_iter().find(|c| c.name == "Blue Warm").unwrap();
    w.set_theme_colors(colors.clone());
    w.commit("Theme Colors");
    assert_eq!(w.style_at(0, 0, 0).fill.unwrap().value(), 0x4A66AC);
    assert_eq!((w.theme().name.as_str(), w.theme().colors.name.as_str()), ("Office Theme", "Blue Warm"));
    assert_eq!(w.theme().fonts, office.fonts);

    let fonts = Theme::builtin_fonts().into_iter().find(|f| f.name == "Arial").unwrap();
    w.set_theme_fonts(fonts);
    w.commit("Theme Fonts");
    assert_eq!(&*w.style_at(0, 0, 0).font.name, "Arial");
    assert_eq!(w.theme().colors, colors);

    w.set_theme_effects("Glossy");
    w.commit("Theme Effects");
    assert_eq!(w.theme().effects, "Glossy");
    // The same theme again is no change at all.
    w.set_theme(w.theme().clone());
    assert!(!w.commit("Theme"));
    assert!(w.undo() && w.undo() && w.undo());
    assert_eq!(*w.theme(), office);
    assert_eq!(w.style_at(0, 0, 0).fill.unwrap().value(), 0x156082);
}

#[test]
fn columns_and_rows_follow_the_default_font() {
    let mut w = wb(&[("A1", "x")]);
    w.set_col_width(0, 1, Some(110.0));
    w.set_row_height(0, 4, Some(40.0));
    w.commit("sizes");
    assert_eq!((w.sheets[0].default_col_width, w.sheets[0].default_row_height), (64.0, 20.0));

    // Arial 11: digits of 8 pixels where Aptos Narrow has 7.
    let fonts = Theme::builtin_fonts().into_iter().find(|f| f.name == "Arial").unwrap();
    w.set_theme_fonts(fonts);
    w.commit("Theme Fonts");
    let metrics = w.font_metrics();
    assert_eq!(metrics.digit_width, 8.0);
    assert_eq!(w.sheets[0].default_col_width, metrics.col_width);
    assert_eq!(w.sheets[0].default_col_width, 72.0);
    assert_eq!(w.sheets[0].default_row_height, metrics.row_height);
    // The column keeps its 15 characters: 15 x 8 + 5.
    assert_eq!(w.sheets[0].col_width(1), 125.0);
    // A row that was given a height keeps it.
    assert_eq!(w.sheets[0].row_height(4), 40.0);

    assert!(w.undo());
    assert_eq!((w.sheets[0].default_col_width, w.sheets[0].default_row_height), (64.0, 20.0));
    assert_eq!(w.sheets[0].col_width(1), 110.0);

    // A font with the same digits changes nothing.
    w.set_theme(Theme::office_2013());
    w.commit("Theme");
    assert_eq!((w.sheets[0].default_col_width, w.sheets[0].col_width(1)), (64.0, 110.0));
}

#[test]
fn styles_computed_from_the_theme_follow_it() {
    let mut w = wb(&[("A1", "Product"), ("B1", "Sales"), ("A2", "Pen"), ("B2", "10"), ("A3", "Ink"), ("B3", "20")]);
    w.add_table(0, rect("A1:B3"), true).unwrap();
    w.apply_cell_style(0, &[rect("D1")], "Accent3").unwrap();
    w.apply_cell_style(0, &[rect("D2")], "Title").unwrap();
    w.apply_cell_style(0, &[rect("D3")], "Hyperlink").unwrap();
    w.apply_cell_style(0, &[rect("D4")], "Bad").unwrap();
    w.commit("setup");
    let header = |w: &Workbook| crate::table::style_patch_at(w, 0, 0, 0).unwrap().fill.unwrap().unwrap();
    assert_eq!(header(&w).value(), 0x156082);
    assert_eq!(w.style_at(0, 0, 3).fill.unwrap().value(), 0x196B24);
    assert_eq!(&*w.style_at(0, 1, 3).font.name, "Aptos Display");

    let facet = facet();
    w.set_theme(facet.clone());
    w.commit("Theme");
    // The table style, by name, in the colours of the theme.
    assert_eq!(header(&w), facet.accent(1, 0.0));
    assert_eq!(w.display(0, 0, 0).style.fill, Some(facet.accent(1, 0.0)));
    let band = crate::table::style_patch_at(&w, 0, 1, 0).unwrap().fill.unwrap().unwrap();
    assert_eq!(band, facet.accent(1, 0.8));
    assert_eq!(
        crate::table::table_style_in(&facet, "TableStyleMedium2").unwrap().header_row.fill,
        Some(facet.accent(1, 0.0))
    );
    // The built-in ones stay those of the Office theme.
    assert_eq!(crate::table::table_style("TableStyleMedium2").unwrap().header_row.fill.unwrap().value(), 0x156082);
    // Cell styles: what they took from the theme.
    assert_eq!(w.style_at(0, 0, 3).fill, Some(facet.accent(3, 0.0)));
    assert_eq!(&*w.style_at(0, 1, 3).font.name, "Trebuchet MS");
    assert_eq!(w.style_at(0, 1, 3).font.color, Some(facet.color(ThemeSlot::Dark2, 0.0)));
    assert_eq!(w.style_at(0, 2, 3).font.color.unwrap().value(), 0x99CA3C);
    // "Bad" is the same red in every theme.
    assert_eq!(w.style_at(0, 3, 3).fill, Some(Color(0xFFC7CE)));
    // The gallery shows the styles of the theme.
    let accent3 = w.cell_style("Accent3").unwrap();
    assert_eq!(accent3.style.fill, Some(facet.accent(3, 0.0)));
    // A cell is still seen to have the style it was given.
    assert_eq!(w.cell_style_at(0, 0, 3), "Accent3");

    // PivotTable styles are built on the accents of the theme.
    let style = crate::pivot::PivotStyle {
        name: "PivotStyleMedium2".to_string(),
        ..Default::default()
    };
    assert_ne!(style.sample_in(&facet), style.sample());
    assert_eq!(style.sample_in(&Theme::office()), style.sample());
}

#[test]
fn what_sheets_keep_follows_the_theme() {
    let mut w = wb(&[("A1", "1"), ("A2", "2"), ("B1", "3"), ("B2", "4")]);
    let office = w.theme().clone();
    w.set_tab_color(0, Some(office.accent(6, 0.0)));
    let scale = ColorScale {
        stops: vec![
            ColorStop {
                at: Cfvo::Min,
                color: office.accent(1, 0.8),
            },
            ColorStop {
                at: Cfvo::Max,
                color: Color(0xFF0000),
            },
        ],
    };
    w.add_cond_format(0, CondFormat::new(vec![rect("A1:A2")], Rule::ColorScale(scale)));
    let shape = w.add_shape(0, ShapeTool::Rectangle, Frame::new(10.0, 10.0, 80.0, 40.0)).unwrap();
    let mut chart: Chart = w.chart_from_range(0, rect("A1:B2"), ChartKind::Column);
    assert_eq!(chart.series.len(), 2);
    chart.series[1].color = Some(office.accent(4, -0.25));
    chart.chart_area.fill = Fill::Solid(office.color(ThemeSlot::Light2, 0.0));
    w.add_chart(0, chart);
    w.commit("setup");

    let facet = facet();
    w.set_theme(facet.clone());
    w.commit("Theme");
    let sheet = &w.sheets[0];
    assert_eq!(sheet.tab_color, Some(facet.accent(6, 0.0)));
    let Rule::ColorScale(scale) = &sheet.cond_formats[0].rule else {
        panic!("a colour scale");
    };
    assert_eq!((scale.stops[0].color, scale.stops[1].color), (facet.accent(1, 0.8), Color(0xFF0000)));
    // A new shape is filled with the first accent.
    let DrawingKind::Shape(rectangle) = &w.drawing(0, shape).unwrap().kind else {
        panic!("a shape");
    };
    assert_eq!(rectangle.fill, Some(facet.accent(1, 0.0)));
    assert_eq!(&*rectangle.font.name, "Trebuchet MS");
    let chart = &sheet.charts[0];
    assert_eq!(chart.series[1].color, Some(facet.accent(4, -0.25)));
    assert_eq!(chart.chart_area.fill, Fill::Solid(facet.color(ThemeSlot::Light2, 0.0)));
    // The automatic colour of the first series is the first accent.
    let data = w.chart_data(0, chart);
    assert_eq!(data.series[0].color.value(), 0x90C226);
    assert_eq!(data.palette_color(1).value(), 0x54A021);
    assert_eq!(facet.chart_color(0).value(), 0x90C226);

    // A shape drawn in the new theme takes its accent at once.
    let second = w.add_shape(0, ShapeTool::Ellipse, Frame::new(10.0, 80.0, 80.0, 40.0)).unwrap();
    w.commit("shape");
    let DrawingKind::Shape(ellipse) = &w.drawing(0, second).unwrap().kind else {
        panic!("a shape");
    };
    assert_eq!(ellipse.fill.unwrap().value(), 0x90C226);

    assert!(w.undo() && w.undo());
    assert_eq!(w.sheets[0].tab_color, Some(office.accent(6, 0.0)));
    assert_eq!(w.sheets[0].charts[0].series[1].color, Some(office.accent(4, -0.25)));
}
