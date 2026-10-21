//! Warped text (Text Effects ▸ Transform), gradient letters and
//! reflections in `.xlsx` files: what the writer produces, that it comes
//! back unchanged, and what is made of what Excel writes.

mod common;

use common::{assert_well_formed, package, part, part_names};
use opensheet_engine::drawing::{Drawing, Shape};
use opensheet_engine::textwarp::{TextWarp, WarpPreset};
use opensheet_engine::wordart::{
    GradientKind, GradientStop, TextGradient, TextReflection, WordArtStyle,
};
use opensheet_engine::{Color, Locale, Workbook};
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

/// The package with one part replaced.
fn with_part(bytes: &[u8], name: &str, text: &str) -> Vec<u8> {
    let names = part_names(bytes);
    let texts: Vec<String> = names
        .iter()
        .map(|n| {
            if n == name {
                text.to_string()
            } else {
                part(bytes, n)
            }
        })
        .collect();
    let parts: Vec<(&str, &str)> = names
        .iter()
        .map(String::as_str)
        .zip(texts.iter().map(String::as_str))
        .collect();
    package(&parts)
}

fn shapes(wb: &Workbook) -> Vec<Shape> {
    wb.sheets[0]
        .drawings
        .iter()
        .filter_map(|drawing| drawing.as_shape().cloned())
        .collect()
}

/// One WordArt per preset, four to a row, each named by its preset.
fn gallery() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (i, preset) in WarpPreset::ALL.into_iter().enumerate() {
        let mut shape = WordArtStyle(1 + (i % 3) as u8).shape(preset.name());
        shape.font.set_size_pt(28.0);
        shape.warp = Some(TextWarp::from(preset));
        // Presets with several places get a line for each.
        if matches!(
            preset,
            WarpPreset::Button | WarpPreset::ButtonPour | WarpPreset::DeflateInflateDeflate
        ) {
            shape.text = format!("{}\nsecond line\nthird line", preset.name());
        } else if preset == WarpPreset::DeflateInflate {
            shape.text = format!("{}\nsecond line", preset.name());
        }
        let mut drawing = Drawing::shape(shape, (230.0, 110.0));
        drawing.offset = (20.0 + 250.0 * (i % 4) as f32, 20.0 + 130.0 * (i / 4) as f32);
        wb.add_drawing(0, drawing).unwrap();
    }
    wb.commit("warps");
    wb
}

#[test]
fn every_warp_is_written_as_excel_writes_it_and_read_back() {
    let wb = gallery();
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-textwarp.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // The warp is the first child of the body properties; a warp that
    // keeps the values of its preset has an empty list, and the box no
    // longer follows the size of its text.
    assert!(
        xml.contains("anchor=\"t\"><a:prstTxWarp prst=\"textArchUp\"><a:avLst/></a:prstTxWarp><a:noAutofit/></a:bodyPr>"),
        "{xml}"
    );
    for preset in WarpPreset::ALL {
        assert!(
            xml.contains(&format!("<a:prstTxWarp prst=\"{}\">", preset.ooxml_name())),
            "{preset:?}"
        );
    }
    assert!(!xml.contains("<a:spAutoFit/>"));
    let back = load(&bytes);
    assert_eq!(shapes(&back), shapes(&wb));
    // A second trip changes nothing.
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}

#[test]
fn adjustments_gradients_and_reflections_round_trip() {
    let mut wb = Workbook::new(Locale::en());
    // A wave whose second handle was moved: both adjustments are listed.
    let mut wave = WordArtStyle(2).shape("Adjusted wave");
    let mut warp = TextWarp::from(WarpPreset::Wave1);
    warp.adjust[1] = Some(6_000);
    wave.warp = Some(warp);
    // A triangle in a wrapped shape, with its one adjustment.
    let mut triangle = WordArtStyle(0).shape("Triangle");
    triangle.wrap = true;
    triangle.warp = Some(TextWarp {
        preset: WarpPreset::Triangle,
        adjust: [Some(25_000), None],
    });
    // Letters filled with a gradient from top to bottom, with a reflection.
    let mut gradient = WordArtStyle(5).shape("Gradient");
    let stops = [
        GradientStop {
            at: 0.0,
            color: Color(0xFFC000),
        },
        GradientStop {
            at: 0.5,
            color: Color(0xED7D31),
        },
        GradientStop {
            at: 1.0,
            color: Color(0xC00000),
        },
    ];
    gradient.effects.gradient = TextGradient::new(GradientKind::Linear(90.0), &stops);
    gradient.font.color = Some(Color(0xFFC000));
    gradient.effects.reflection = Some(TextReflection::PRESETS[4]);
    // One word of another text shines from its middle.
    let mut part_of = WordArtStyle(1).shape("From the centre");
    part_of.format_text(9..15, &|font, effects| {
        font.color = Some(Color(0xFFFFFF));
        effects.gradient = Some(TextGradient::between(
            Color(0xFFFFFF),
            Color(0x0070C0),
            GradientKind::FromCentre,
        ));
        effects.reflection = Some(TextReflection::PRESETS[0]);
    });
    for (i, shape) in [wave, triangle, gradient, part_of].into_iter().enumerate() {
        let mut drawing = Drawing::shape(shape, (360.0, 90.0));
        drawing.offset = (30.0, 20.0 + 120.0 * i as f32);
        wb.add_drawing(0, drawing).unwrap();
    }
    wb.commit("looks");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-text-gradient-reflection.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    assert!(
        xml.contains("<a:prstTxWarp prst=\"textWave1\"><a:avLst><a:gd name=\"adj1\" fmla=\"val 12500\"/><a:gd name=\"adj2\" fmla=\"val 6000\"/></a:avLst></a:prstTxWarp>"),
        "{xml}"
    );
    assert!(xml.contains("wrap=\"square\" rtlCol=\"0\" anchor=\"t\"><a:prstTxWarp prst=\"textTriangle\"><a:avLst><a:gd name=\"adj\" fmla=\"val 25000\"/></a:avLst></a:prstTxWarp></a:bodyPr>"));
    assert!(xml.contains(
        "<a:gradFill flip=\"none\" rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FFC000\"/></a:gs><a:gs pos=\"50000\"><a:srgbClr val=\"ED7D31\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"C00000\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"1\"/><a:tileRect/></a:gradFill>"
    ));
    assert!(xml.contains("<a:path path=\"circle\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path>"));
    assert!(xml.contains(
        "<a:effectLst><a:reflection blurRad=\"6350\" stA=\"50000\" endA=\"300\" endPos=\"55500\" dist=\"50800\" dir=\"5400000\" sy=\"-100000\" algn=\"bl\" rotWithShape=\"0\"/></a:effectLst>"
    ));
    // A shadow comes before the reflection, as the schema orders them.
    let both = xml.find("<a:outerShdw").zip(
        xml.find("<a:reflection blurRad=\"6350\" stA=\"52000\" endA=\"300\" endPos=\"35000\" dir"),
    );
    assert!(both.is_some_and(|(shadow, reflection)| shadow < reflection));
    let back = load(&bytes);
    assert_eq!(shapes(&back), shapes(&wb));
}

#[test]
fn what_excel_writes_is_read() {
    // The body and a run of a WordArt warped and filled in Excel.
    let wb = gallery();
    let bytes = to_bytes(&wb);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    let excel = xml
        .replacen(
            "<a:prstTxWarp prst=\"textArchUp\"><a:avLst/></a:prstTxWarp>",
            "<a:prstTxWarp prst=\"textArchUp\"><a:avLst><a:gd name=\"adj\" fmla=\"val 9600000\"/></a:avLst></a:prstTxWarp><a:scene3d><a:camera prst=\"orthographicFront\"/><a:lightRig rig=\"threePt\" dir=\"t\"/></a:scene3d>",
            1,
        )
        .replacen("<a:prstTxWarp prst=\"textStop\"><a:avLst/></a:prstTxWarp>", "<a:prstTxWarp prst=\"textNoShape\"><a:avLst/></a:prstTxWarp>", 1)
        .replacen(
            "<a:prstTxWarp prst=\"textPlain\"><a:avLst/></a:prstTxWarp>",
            "<a:prstTxWarp prst=\"textFromTheFuture\"><a:avLst><a:gd name=\"adj\" fmla=\"val 1\"/></a:avLst></a:prstTxWarp>",
            1,
        );
    assert_ne!(excel, xml);
    let bytes = with_part(&bytes, "xl/drawings/drawing1.xml", &excel);
    let read = shapes(&load(&bytes));
    let of = |preset: WarpPreset| {
        read.iter()
            .find(|shape| shape.text.starts_with(preset.name()))
            .and_then(|shape| shape.warp)
    };
    assert_eq!(
        of(WarpPreset::ArchUp),
        Some(TextWarp {
            preset: WarpPreset::ArchUp,
            adjust: [Some(9_600_000), None]
        })
    );
    // "No shape" and a preset we do not know are no warp.
    assert_eq!(of(WarpPreset::Stop), None);
    assert_eq!(of(WarpPreset::Plain), None);
    assert_eq!(
        of(WarpPreset::Triangle),
        Some(TextWarp::from(WarpPreset::Triangle))
    );
}
