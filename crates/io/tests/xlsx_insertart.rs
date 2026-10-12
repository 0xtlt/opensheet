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

/// The header of a GIF image: enough for a picture to know its size.
fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes
}

#[test]
fn vector_pictures_keep_their_document_beside_an_image() {
    use opensheet_engine::drawing::Picture;
    let star = br##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 24 24"><path d="M12 2l3 7h7l-5.5 4.5 2 7.5-6.5-4.5-6.5 4.5 2-7.5L2 9h7z"/></svg>"##;
    let mut wb = Workbook::new(Locale::en());
    let picture = Picture::vector(star.to_vec(), gif(192, 192)).expect("a vector picture");
    // Inserted at the size the document asks for, not that of its image.
    let mut first = Drawing::picture(picture.clone());
    assert_eq!(first.size, (96.0, 96.0));
    first.offset = (30.0, 20.0);
    let first = wb.add_drawing(0, first).unwrap();
    assert_eq!(wb.drawing(0, first).unwrap().name, "Graphic 1");
    // The same icon again, and a plain picture of the same image.
    let mut second = Drawing::picture(picture);
    second.offset = (200.0, 20.0);
    second.rotation = 90.0;
    wb.add_drawing(0, second).unwrap();
    wb.add_picture(0, gif(192, 192), (10, 1)).unwrap();
    wb.commit("pictures");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-icons.xlsx", &bytes);

    // One image and one document, however many pictures show them.
    let names = common::part_names(&bytes);
    assert_eq!(names.iter().filter(|name| name.starts_with("xl/media/")).count(), 2, "{names:?}");
    assert_eq!(part(&bytes, "xl/media/image2.svg").as_bytes(), star);
    let types = part(&bytes, "[Content_Types].xml");
    assert!(types.contains("<Default Extension=\"svg\" ContentType=\"image/svg+xml\"/>"), "{types}");
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    assert_eq!(xml.matches("<asvg:svgBlip xmlns:asvg=\"http://schemas.microsoft.com/office/drawing/2016/SVG/main\" r:embed=\"rId2\"/>").count(), 2, "{xml}");
    assert_eq!(xml.matches("r:embed=\"rId1\"").count(), 3);
    assert!(xml.contains("<a:ext uri=\"{96DAC541-7B7A-43D3-8B79-37D633B846F1}\">"));
    let rels = part(&bytes, "xl/drawings/_rels/drawing1.xml.rels");
    assert!(rels.contains("Target=\"../media/image1.gif\"") && rels.contains("Target=\"../media/image2.svg\""), "{rels}");
    assert_eq!(rels.matches("relationships/image\"").count(), 2);

    let back = load(&bytes);
    let pictures: Vec<&Picture> = back.sheets[0].drawings.iter().map(|drawing| drawing.as_picture().unwrap()).collect();
    assert_eq!(pictures[0].svg.as_deref(), Some(&star[..]));
    assert_eq!(pictures[1].svg.as_deref(), Some(&star[..]));
    assert!(pictures[2].svg.is_none());
    assert_eq!(back.sheets[0].drawings[1].rotation, 90.0);
    assert_eq!(back.drawing_frame(0, back.sheets[0].drawings[0].id).unwrap().width, 96.0);
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}
