//! SmartArt shapes formatted one by one, and charts held in groups, in
//! `.xlsx` files: what is written, and that it reads back as it was.

mod common;

use common::{assert_well_formed, part};
use opensheet_engine::drawing::{Drawing, Effects, Glow, Outline, ShadowPreset, ShapePreset};
use opensheet_engine::smartart::{Layout, SmartArt};
use opensheet_engine::wordart::{TextEffects, TextOutline};
use opensheet_engine::{Color, Locale, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    write_xlsx(wb, &mut out).expect("written");
    out.into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("read")
}

fn graphic(layout: Layout, lines: &[(usize, &str)]) -> SmartArt {
    let mut art = SmartArt::new(layout);
    art.set_outline(
        &lines
            .iter()
            .map(|(level, text)| (*level, text.to_string()))
            .collect::<Vec<_>>(),
    );
    art
}

// ---- the shapes of a graphic, formatted one by one -----------------------------

/// A process whose three shapes each got something from the Format tab,
/// and an organization chart with one box changed.
fn formatted_graphics() -> Vec<SmartArt> {
    let mut process = graphic(
        Layout::Process,
        &[(0, "Larger, red"), (0, "Ellipse, glow"), (0, "Hollow text")],
    );
    process.set_format(0, |format| {
        format.resize(true);
        format.resize(true);
        format.fill = Some(Some(Color(0xC00000)));
        format.outline = Some(Some(Outline::solid(Color(0x000000), 3.0)));
        format.text_color = Some(Color(0xFFFF00));
    });
    process.set_format(1, |format| {
        format.preset = Some(ShapePreset::Ellipse);
        format.effects = Some(Effects {
            glow: Some(Glow {
                color: Color(0xFFC000),
                transparency: 0.5,
                radius: 8.0,
            }),
            shadow: Some(ShadowPreset::BottomRight.shadow()),
            ..Effects::default()
        });
    });
    process.set_format(2, |format| {
        format.resize(false);
        format.fill = Some(None);
        format.outline = Some(None);
        format.text_effects = Some(TextEffects {
            hollow: true,
            outline: Some(TextOutline {
                color: Color(0x0070C0),
                width: 1.0,
            }),
            ..TextEffects::default()
        });
    });
    let mut chart = graphic(Layout::OrgChart, &[(0, "Head"), (1, "Sales"), (1, "Works")]);
    chart.set_format(2, |format| {
        format.preset = Some(ShapePreset::RoundRect);
        format.fill = Some(Some(Color(0x00B050)));
    });
    vec![process, chart]
}

#[test]
fn the_look_of_each_shape_of_a_graphic_is_kept() {
    let mut wb = Workbook::new(Locale::en());
    let arts = formatted_graphics();
    for (index, art) in arts.iter().enumerate() {
        let mut drawing = Drawing::smartart(art.clone(), (480.0, 260.0));
        drawing.offset = (40.0, 30.0 + index as f32 * 300.0);
        wb.add_drawing(0, drawing).unwrap();
    }
    wb.commit("graphics");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-smartart-format.xlsx", &bytes);

    // The outline: the look is on the point of its entry, where Office
    // keeps what was changed by hand.
    let data = part(&bytes, "xl/diagrams/data1.xml");
    assert_well_formed("data1.xml", &data);
    assert!(data.contains("<dgm:prSet custScaleX=\"121000\" custScaleY=\"121000\"/><dgm:spPr><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill><a:ln w=\"38100\">"), "{data}");
    assert!(data.contains("<a:rPr lang=\"en-US\"><a:solidFill><a:srgbClr val=\"FFFF00\"/></a:solidFill></a:rPr><a:t>Larger, red</a:t>"), "{data}");
    assert!(data.contains("<dgm:spPr><a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom><a:effectLst><a:glow rad=\"101600\">"), "{data}");
    assert!(
        data.contains("<dgm:spPr><a:noFill/><a:ln><a:noFill/></a:ln></dgm:spPr>"),
        "{data}"
    );
    assert!(data.contains("<a:rPr lang=\"en-US\"><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"0070C0\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:noFill/></a:rPr>"), "{data}");
    // The shapes the graphic was laid out as say the same.
    let cache = part(&bytes, "xl/diagrams/drawing1.xml");
    assert_well_formed("diagrams/drawing1.xml", &cache);
    assert!(
        cache.contains("<a:prstGeom prst=\"ellipse\">")
            && cache.contains("<a:glow rad=\"101600\">")
            && cache.contains("<a:srgbClr val=\"C00000\"/>"),
        "{cache}"
    );
    // And so does the group other applications show: three shapes and
    // two arrows, the second shape an ellipse with its glow and shadow.
    let sheet = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawings/drawing1.xml", &sheet);
    let fallback = &sheet
        [sheet.find("<mc:Fallback>").expect("a fallback")..sheet.find("</mc:Fallback>").unwrap()];
    assert_eq!(fallback.matches("<xdr:sp ").count(), 5);
    assert!(
        fallback.contains("<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>"),
        "{fallback}"
    );
    assert!(
        fallback.contains("<a:effectLst><a:glow rad=\"101600\">")
            && fallback.contains("<a:outerShdw "),
        "{fallback}"
    );
    assert!(
        fallback
            .contains("<a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill><a:ln w=\"38100\">"),
        "{fallback}"
    );
    assert!(fallback.contains("<a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"0070C0\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln><a:noFill/>"), "{fallback}");

    // Read back: the same graphics, and written again the same file.
    let back = load(&bytes);
    let read: Vec<SmartArt> = back.sheets[0]
        .drawings
        .iter()
        .filter_map(|drawing| drawing.as_smartart().cloned())
        .collect();
    assert_eq!(read, arts);
    assert_eq!(to_bytes(&back), bytes);
    // The larger shape is larger than its neighbour was made.
    let pieces = read[0].pieces(back.theme(), 480.0, 260.0);
    let of = |line: usize| {
        pieces
            .iter()
            .find(|piece| piece.node == Some(line))
            .unwrap()
    };
    assert!((of(0).width / of(1).width - 1.21).abs() < 0.01);
    assert!((of(2).width / of(1).width - 1.0 / 1.1).abs() < 0.01);

    // "Reset Graphic" clears every shape: the file is the plain one.
    let mut plain = load(&bytes);
    for id in plain.sheets[0]
        .drawings
        .iter()
        .map(|drawing| drawing.id)
        .collect::<Vec<_>>()
    {
        plain.update_drawing(0, id, |drawing| drawing.as_smartart_mut().unwrap().reset());
    }
    plain.commit("reset");
    let data = part(&to_bytes(&plain), "xl/diagrams/data1.xml");
    assert!(
        !data.contains("custScaleX") && !data.contains("prstGeom") && !data.contains("C00000"),
        "{data}"
    );
}

// ---- a member of a group, changed inside it --------------------------------------

use opensheet_engine::drawing::{Frame, ShapeTool};

#[test]
fn a_group_whose_member_was_moved_and_formatted_is_kept() {
    let mut wb = Workbook::new(Locale::en());
    let a = wb
        .add_shape(0, ShapeTool::Rectangle, Frame::new(60.0, 40.0, 120.0, 70.0))
        .unwrap();
    let b = wb
        .add_shape(0, ShapeTool::Ellipse, Frame::new(220.0, 40.0, 90.0, 70.0))
        .unwrap();
    let c = wb
        .add_shape(0, ShapeTool::Diamond, Frame::new(130.0, 140.0, 70.0, 70.0))
        .unwrap();
    wb.update_drawing(0, a, |drawing| {
        drawing
            .as_shape_mut()
            .unwrap()
            .set_text("Stays".to_string())
    });
    let group = wb.group_drawings(0, &[a, b, c]).expect("a group");
    // The ellipse alone: red, larger, further right; the box of the
    // group follows it.
    wb.update_drawing(0, b, |drawing| {
        let shape = drawing.as_shape_mut().unwrap();
        shape.fill = Some(Color(0xC00000));
        shape.set_text("Moved".to_string());
    });
    wb.set_drawing_frame(0, b, Frame::new(320.0, 20.0, 140.0, 110.0));
    // And the diamond is gone from the group.
    wb.remove_drawing(0, c);
    wb.commit("group");
    assert_eq!(
        wb.drawing_frame(0, group),
        Some(Frame::new(60.0, 20.0, 400.0, 110.0))
    );
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-group-member.xlsx", &bytes);
    let sheet = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawings/drawing1.xml", &sheet);
    assert_eq!(
        (
            sheet.matches("<xdr:grpSp>").count(),
            sheet.matches("<xdr:sp ").count()
        ),
        (1, 2)
    );

    let back = load(&bytes);
    let held = &back.sheets[0].drawings[0];
    let frames: Vec<(String, Frame)> = back.sheets[0]
        .drawing_leaves(held)
        .iter()
        .map(|leaf| (leaf.drawing.as_shape().unwrap().text.clone(), leaf.frame))
        .collect();
    let near = |frame: Frame, x: f64, y: f64, w: f64, h: f64| {
        (frame.x - x).abs() < 0.1
            && (frame.y - y).abs() < 0.1
            && (frame.width - w).abs() < 0.1
            && (frame.height - h).abs() < 0.1
    };
    assert_eq!(frames.len(), 2);
    assert!(
        frames[0].0 == "Stays" && near(frames[0].1, 60.0, 40.0, 120.0, 70.0),
        "{frames:?}"
    );
    assert!(
        frames[1].0 == "Moved" && near(frames[1].1, 320.0, 20.0, 140.0, 110.0),
        "{frames:?}"
    );
    assert_eq!(to_bytes(&back), bytes);
}
