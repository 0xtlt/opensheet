//! WordArt, vector pictures, SmartArt and equations in `.xlsx` files: what
//! the writer produces, that it comes back unchanged, and what is made of
//! the parts Excel writes.

mod common;

use common::{assert_well_formed, part};
use opensheet_engine::drawing::{Drawing, Frame, ShapeTool};
use opensheet_engine::wordart::{TextOutline, TextShadow, WordArtStyle};
use opensheet_engine::{Color, Locale, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// The drawings of the first sheet without what a file does not keep
/// exactly (ids are given anew, sizes are rounded to EMUs).
fn drawings(wb: &Workbook) -> Vec<Drawing> {
    wb.sheets[0].drawings.clone()
}

#[test]
fn wordart_keeps_the_look_of_its_letters() {
    let mut wb = Workbook::new(Locale::en());
    let mut art = Drawing::shape(WordArtStyle(3).shape("Your text here"), (420.0, 90.0));
    art.offset = (40.0, 30.0);
    let art = wb.add_drawing(0, art).unwrap();
    // A shape whose text changes look half way, with a shadow on one part.
    let plain = wb.add_shape(0, ShapeTool::Rectangle, Frame::new(40.0, 160.0, 220.0, 80.0)).unwrap();
    wb.update_drawing(0, plain, |drawing| {
        let shape = drawing.as_shape_mut().unwrap();
        shape.text = "Total\nsales 2026".to_string();
        shape.format_text(6..11, &|font, effects| {
            font.bold = true;
            font.color = Some(Color(0xC00000));
            effects.shadow = Some(TextShadow::offset(45.0));
        });
        // Letters without fill have no colour to keep.
        shape.format_text(12..16, &|font, effects| {
            font.color = None;
            effects.hollow = true;
            effects.outline = Some(TextOutline {
                color: Color(0x0070C0),
                width: 1.5,
            });
        });
    });
    wb.commit("drawings");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-wordart.xlsx", &bytes);

    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // WordArt as Excel writes it: an unwrapped body that fits its text,
    // and runs with a line, a fill and a shadow, in that order.
    assert!(xml.contains("<a:bodyPr wrap=\"none\""), "{xml}");
    assert!(xml.contains("<a:spAutoFit/>"));
    let run = &xml[xml.find("<a:rPr sz=\"5400\"").expect("a 54 pt run")..];
    let (line, fill, shadow) = (run.find("<a:ln w=\"12700\">").unwrap(), run.find("<a:solidFill><a:srgbClr val=\"FFFFFF\"/>").unwrap(), run.find("<a:effectLst><a:outerShdw").unwrap());
    assert!(line < fill && fill < shadow, "{run}");
    assert!(run.contains("<a:alpha val=\"40000\"/>"));
    // The second shape: three runs in its second paragraph.
    assert!(xml.contains("<a:noFill/><a:latin"), "{xml}");
    assert_eq!(xml.matches("<a:t>").count(), 5);

    let back = load(&bytes);
    let (before, after) = (drawings(&wb), drawings(&back));
    assert_eq!(after.len(), 2);
    for (a, b) in before.iter().zip(&after) {
        let (a, b) = (a.as_shape().unwrap(), b.as_shape().unwrap());
        assert_eq!(a.text, b.text);
        assert_eq!(a.runs, b.runs);
        assert_eq!((&a.font, a.effects, a.wrap), (&b.font, b.effects, b.wrap));
    }
    assert!(!back.drawing(0, after[0].id).unwrap().as_shape().unwrap().wrap);
    assert_eq!(wb.drawing(0, art).unwrap().as_shape().unwrap().runs.len(), 0);
    assert_eq!(after[1].as_shape().unwrap().runs.len(), 4);
    // Written again, nothing moves.
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}
