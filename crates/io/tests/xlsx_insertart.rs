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
    let (line, fill, shadow) = (run.find("<a:ln w=\"12700\">").unwrap(), run.find("</a:ln><a:solidFill><a:schemeClr val=\"bg1\"/>").unwrap(), run.find("<a:effectLst><a:outerShdw").unwrap());
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

// ---- SmartArt ------------------------------------------------------------------

use opensheet_engine::smartart::{ColorScheme, Layout, QuickStyle, SmartArt};

fn graphic(layout: Layout, lines: &[(usize, &str)]) -> SmartArt {
    let mut art = SmartArt::new(layout);
    art.set_outline(&lines.iter().map(|(level, text)| (*level, text.to_string())).collect::<Vec<_>>());
    art
}

#[test]
fn smartart_is_written_as_five_parts_and_read_back() {
    let mut wb = Workbook::new(Locale::en());
    let mut expected = Vec::new();
    for (i, layout) in Layout::ALL.into_iter().enumerate() {
        let mut art = graphic(layout, &[(0, "Plan"), (1, "Scope & <budget>"), (1, ""), (0, "Build"), (0, "Ship")]);
        art.colors = ColorScheme::all()[i * 4];
        art.style = QuickStyle::ALL[i % 5];
        let mut drawing = Drawing::smartart(art.clone(), (480.0, 300.0));
        drawing.offset = (20.0 + 500.0 * (i % 2) as f32, 20.0 + 320.0 * (i / 2) as f32);
        let id = wb.add_drawing(0, drawing).unwrap();
        assert_eq!(wb.drawing(0, id).unwrap().name, format!("Diagram {}", i + 1));
        expected.push(art);
    }
    // A graphic that was never given text, beside a picture.
    wb.add_drawing(0, Drawing::smartart(SmartArt::new(Layout::Cycle), (300.0, 300.0))).unwrap();
    expected.push(SmartArt::new(Layout::Cycle));
    wb.add_picture(0, gif(20, 20), (40, 1)).unwrap();
    wb.commit("graphics");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-smartart.xlsx", &bytes);

    let names = common::part_names(&bytes);
    let types = part(&bytes, "[Content_Types].xml");
    for number in 1..=9 {
        for (file, kind) in [("data", "diagramData"), ("layout", "diagramLayout"), ("quickStyle", "diagramStyle"), ("colors", "diagramColors")] {
            let name = format!("xl/diagrams/{file}{number}.xml");
            assert!(names.contains(&name), "{name}");
            assert_well_formed(&name, &part(&bytes, &name));
            assert!(types.contains(&format!("<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.{kind}+xml\"/>")), "{name}");
        }
        let name = format!("xl/diagrams/drawing{number}.xml");
        assert_well_formed(&name, &part(&bytes, &name));
        assert!(types.contains(&format!("<Override PartName=\"/{name}\" ContentType=\"application/vnd.ms-office.drawingml.diagramDrawing+xml\"/>")));
    }
    // The frame names four parts; the data part names the fifth by the
    // relationship the drawing part gives it.
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    assert_eq!(xml.matches("<xdr:graphicFrame macro=\"\">").count(), 9);
    assert!(xml.contains("r:dm=\"rId1\" r:lo=\"rId2\" r:qs=\"rId3\" r:cs=\"rId4\""), "{xml}");
    assert!(xml.contains("r:dm=\"rId6\" r:lo=\"rId7\""));
    let rels = part(&bytes, "xl/drawings/_rels/drawing1.xml.rels");
    assert!(rels.contains("<Relationship Id=\"rId5\" Type=\"http://schemas.microsoft.com/office/2007/relationships/diagramDrawing\" Target=\"../diagrams/drawing1.xml\"/>"), "{rels}");
    assert!(rels.contains("Id=\"rId46\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\""), "{rels}");
    let data = part(&bytes, "xl/diagrams/data1.xml");
    assert!(data.contains("<dsp:dataModelExt xmlns:dsp=\"http://schemas.microsoft.com/office/drawing/2008/diagram\" relId=\"rId5\""), "{data}");
    assert!(data.contains("loTypeId=\"urn:microsoft.com/office/officeart/2005/8/layout/default\" loCatId=\"list\""));
    assert!(data.contains("<a:t>Scope &amp; &lt;budget&gt;</a:t>") && data.contains("phldrT=\"[Text]\" phldr=\"1\""));
    // Five entries: a point and two transitions each, and the root.
    assert_eq!((data.matches("<dgm:pt ").count(), data.matches("<dgm:cxn ").count()), (16, 5));
    assert!(data.contains("srcOrd=\"1\" destOrd=\"0\""));
    // Each definition carries the identifier of Office's own.
    for (number, layout) in Layout::ALL.into_iter().enumerate() {
        let definition = part(&bytes, &format!("xl/diagrams/layout{}.xml", number + 1));
        assert!(definition.contains(&format!("uniqueId=\"{}\"", layout.unique_id())), "{layout:?}");
        assert!(definition.contains("<dgm:layoutNode name=\"diagram\">") && definition.matches("xmlns:r=").count() >= 2);
    }
    assert!(part(&bytes, "xl/diagrams/colors2.xml").contains("uniqueId=\"urn:microsoft.com/office/officeart/2005/8/colors/colorful2\""));
    assert!(part(&bytes, "xl/diagrams/quickStyle2.xml").contains("quickstyle/simple2\""));
    // The shapes as they are laid out: three blocks, colours of the theme.
    let cache = part(&bytes, "xl/diagrams/drawing1.xml");
    assert_eq!(cache.matches("<dsp:sp modelId=").count(), 3);
    assert!(cache.contains("<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>") && cache.contains("<a:t>• Scope &amp; &lt;budget&gt;</a:t>"), "{cache}");
    assert!(part(&bytes, "xl/diagrams/drawing8.xml").contains("<a:prstGeom prst=\"trapezoid\"><a:avLst><a:gd name=\"adj\""));

    let back = load(&bytes);
    let read: Vec<&SmartArt> = back.sheets[0].drawings.iter().filter_map(Drawing::as_smartart).collect();
    assert_eq!(read.len(), 9);
    for (before, after) in expected.iter().zip(&read) {
        assert_eq!(*after, before);
    }
    assert_eq!(back.sheets[0].drawings.len(), 10);
    assert_eq!(back.sheets[0].drawings[3].name, "Diagram 4");
    let frame = back.drawing_frame(0, back.sheets[0].drawings[0].id).unwrap();
    assert_eq!((frame.x, frame.y, frame.width, frame.height), (20.0, 20.0, 480.0, 300.0));
    // Written again, the package is the same.
    let again = to_bytes(&back);
    for name in ["xl/drawings/drawing1.xml", "xl/diagrams/data3.xml", "xl/diagrams/drawing5.xml", "xl/diagrams/layout6.xml"] {
        assert_eq!(part(&again, name), part(&bytes, name), "{name}");
    }
}

#[test]
fn a_graphic_of_another_layout_keeps_its_parts_and_shows_its_shapes() {
    // What Excel writes for a layout that is not drawn here, in short.
    let mut wb = Workbook::new(Locale::en());
    wb.add_drawing(0, Drawing::smartart(graphic(Layout::Process, &[(0, "One"), (0, "Two")]), (400.0, 200.0))).unwrap();
    wb.commit("graphic");
    let ours = to_bytes(&wb);
    let foreign_layout = part(&ours, "xl/diagrams/layout1.xml").replace("layout/process1", "layout/hProcess9");
    let foreign_data = part(&ours, "xl/diagrams/data1.xml").replace("layout/process1", "layout/hProcess9").replace("relId=\"rId5\"", "relId=\"rId9\"");
    let rels = part(&ours, "xl/drawings/_rels/drawing1.xml.rels").replace("Id=\"rId5\"", "Id=\"rId9\"");
    let mut parts: Vec<(String, String)> = common::part_names(&ours).into_iter().map(|name| (name.clone(), part(&ours, &name))).collect();
    for (name, content) in &mut parts {
        match name.as_str() {
            "xl/diagrams/layout1.xml" => *content = foreign_layout.clone(),
            "xl/diagrams/data1.xml" => *content = foreign_data.clone(),
            "xl/drawings/_rels/drawing1.xml.rels" => *content = rels.clone(),
            _ => {}
        }
    }
    let borrowed: Vec<(&str, &str)> = parts.iter().map(|(name, content)| (name.as_str(), content.as_str())).collect();
    let bytes = common::package(&borrowed);

    let back = load(&bytes);
    let art = back.sheets[0].drawings[0].as_smartart().expect("a graphic");
    let foreign = art.foreign.as_ref().expect("kept as its file had it");
    // Two boxes and the arrow between them, in the frame of the graphic.
    assert_eq!((foreign.pieces.len(), foreign.size), (3, (400.0, 200.0)));
    assert_eq!(foreign.pieces[0].shape.text, "One");
    assert!(foreign.pieces[0].width > 100.0 && foreign.pieces[2].x > foreign.pieces[0].x);
    assert_eq!(art.pieces(back.theme(), 800.0, 200.0)[0].width, foreign.pieces[0].width * 2.0);
    // The outline is read all the same: giving it a layout draws it here.
    assert_eq!(art.outline(), vec![(0, "One".to_string()), (0, "Two".to_string())]);
    // Saved again, the parts are the file's own, the relationship apart.
    let again = to_bytes(&back);
    assert_eq!(part(&again, "xl/diagrams/layout1.xml"), foreign_layout);
    assert_eq!(part(&again, "xl/diagrams/data1.xml"), foreign_data.replace("relId=\"rId9\"", "relId=\"rId5\""));
    assert_eq!(part(&again, "xl/diagrams/drawing1.xml"), part(&ours, "xl/diagrams/drawing1.xml"));
    assert!(load(&again).sheets[0].drawings[0].as_smartart().unwrap().foreign.is_some());
}

// ---- equations -----------------------------------------------------------------

use opensheet_engine::math::{BUILT_IN, Equation, Structure};

fn equation_box(wb: &mut Workbook, linear: &str, at: (f32, f32)) -> u32 {
    let mut shape = ShapeTool::TextBox.shape();
    shape.fill = None;
    shape.outline = None;
    shape.text = Equation::parse(linear).to_linear();
    shape.math = Some(Equation::parse(linear));
    shape.font.set_size_pt(14.0);
    let mut drawing = Drawing::shape(shape, (360.0, 60.0));
    drawing.offset = at;
    wb.add_drawing(0, drawing).unwrap()
}

#[test]
fn equations_are_written_as_office_math_and_read_back() {
    let mut wb = Workbook::new(Locale::en());
    let mut all: Vec<&str> = BUILT_IN.iter().map(|(_, linear)| *linear).collect();
    for structure in Structure::ALL {
        all.extend(structure.templates());
    }
    for (i, linear) in all.iter().enumerate() {
        equation_box(&mut wb, linear, (20.0 + 380.0 * (i % 3) as f32, 10.0 + 70.0 * (i / 3) as f32));
    }
    // One shown in the linear format, in red.
    let shown_linear = equation_box(&mut wb, "x=(-b±√(b^2-4ac))/2a", (1200.0, 10.0));
    wb.update_drawing(0, shown_linear, |drawing| {
        let shape = drawing.as_shape_mut().unwrap();
        shape.math.as_mut().unwrap().linear = true;
        shape.font.color = Some(Color(0xC00000));
    });
    wb.commit("equations");
    let bytes = to_bytes(&wb);
    common::keep_sample("opensheet-equations.xlsx", &bytes);

    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // Each shape twice: with its equation, and with the text of it.
    let count = all.len() + 1;
    assert_eq!(xml.matches("<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\">").count(), count);
    assert_eq!(xml.matches("<mc:Fallback>").count(), count);
    assert_eq!(xml.matches("<xdr:sp ").count(), 2 * count);
    assert_eq!(xml.matches("<a14:m><m:oMathPara xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">").count(), count);
    // The Pythagorean theorem: three superscripts and the signs between.
    let pythagoras = "<m:sSup><m:e><m:r><a:rPr sz=\"1400\" i=\"1\"><a:latin typeface=\"Cambria Math\" panose=\"02040503050406030204\" pitchFamily=\"18\" charset=\"0\"/></a:rPr><m:t>a</m:t></m:r></m:e><m:sup>";
    assert!(xml.contains(pythagoras), "{xml}");
    assert!(xml.contains("<a:t>a^2+b^2=c^2</a:t>"));
    // The structures, as Office names them.
    for element in [
        "<m:f><m:num>", "<m:f><m:fPr><m:type m:val=\"noBar\"/></m:fPr>", "<m:sSub>", "<m:sSubSup>", "<m:rad><m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>",
        "<m:nary><m:naryPr><m:chr m:val=\"∑\"/><m:limLoc m:val=\"undOvr\"/></m:naryPr>", "<m:nary><m:naryPr><m:limLoc m:val=\"subSup\"/><m:subHide m:val=\"1\"/><m:supHide m:val=\"1\"/></m:naryPr><m:sub/><m:sup/><m:e/></m:nary>",
        "<m:d><m:e>", "<m:d><m:dPr><m:begChr m:val=\"[\"/><m:endChr m:val=\"]\"/></m:dPr>", "<m:func><m:fName><m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr>",
        "<m:acc><m:accPr><m:chr m:val=\"\u{302}\"/></m:accPr><m:e/></m:acc>", "<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr>", "<m:limLow><m:e><m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr>",
        "<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val=\"2\"/><m:mcJc m:val=\"center\"/></m:mcPr></m:mc></m:mcs></m:mPr><m:mr><m:e/><m:e/></m:mr>",
    ] {
        assert!(xml.contains(element), "{element}");
    }

    let back = load(&bytes);
    let read: Vec<_> = back.sheets[0].drawings.iter().map(|drawing| drawing.as_shape().unwrap()).collect();
    assert_eq!(read.len(), count);
    for (linear, shape) in all.iter().zip(&read) {
        let expected = Equation::parse(linear);
        assert_eq!(shape.math.as_ref(), Some(&expected), "{linear}");
        assert_eq!(shape.text, expected.to_linear(), "{linear}");
        assert_eq!((shape.font.size_pt(), shape.font.italic, shape.runs.len()), (14.0, false, 0), "{linear}");
    }
    let last = read[count - 1];
    assert!(last.math.as_ref().unwrap().linear);
    assert_eq!(last.math.as_ref().unwrap().body, Equation::parse("x=(-b±√(b^2-4ac))/2a").body);
    assert_eq!(last.font.color, Some(Color(0xC00000)));
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}

#[test]
fn equations_of_other_writers_are_read() {
    // A shape as Excel writes it, with structures the engine has not (a
    // box, a grouping character) and delimiters without characters.
    let mut wb = Workbook::new(Locale::en());
    equation_box(&mut wb, "a", (10.0, 10.0));
    wb.commit("equation");
    let ours = to_bytes(&wb);
    let xml = part(&ours, "xl/drawings/drawing1.xml");
    let start = xml.find("<m:oMath xmlns").unwrap();
    let end = xml.find("</m:oMath>").unwrap();
    let foreign = "<m:oMath xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\"><m:borderBox><m:e><m:r><m:t>E=m</m:t></m:r><m:sSup><m:e><m:r><m:t>c</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:e></m:borderBox><m:d><m:dPr><m:begChr m:val=\"\"/><m:endChr m:val=\"\"/><m:ctrlPr><a:rPr i=\"1\"/></m:ctrlPr></m:dPr><m:e><m:r><m:t>+1</m:t></m:r></m:e></m:d><m:nary><m:naryPr><m:supHide m:val=\"on\"/></m:naryPr><m:sub><m:r><m:t>0</m:t></m:r></m:sub><m:sup/><m:e><m:func><m:fName><m:r><m:rPr><m:sty m:val=\"p\"/></m:rPr><m:t>cos</m:t></m:r></m:fName><m:e><m:r><m:t>x</m:t></m:r></m:e></m:func></m:e></m:nary>";
    let changed = format!("{}{foreign}{}", &xml[..start], &xml[end..]);
    let mut parts: Vec<(String, String)> = common::part_names(&ours).into_iter().map(|name| (name.clone(), part(&ours, &name))).collect();
    for (name, content) in &mut parts {
        if name == "xl/drawings/drawing1.xml" {
            *content = changed.clone();
        }
    }
    let borrowed: Vec<(&str, &str)> = parts.iter().map(|(name, content)| (name.as_str(), content.as_str())).collect();
    let back = load(&common::package(&borrowed));
    assert_eq!(back.sheets[0].drawings.len(), 1);
    let shape = back.sheets[0].drawings[0].as_shape().unwrap();
    assert_eq!(shape.text, "E=mc^2+1∫_0▒〖cos\u{2061}〖x〗〗");
    assert_eq!(shape.math, Some(Equation::parse(&shape.text)));
}
