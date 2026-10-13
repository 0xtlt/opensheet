//! Ink in `.xlsx` files: the strokes of the Draw tab are written as
//! freeform shapes Excel shows, come back as ink, and the ink Excel writes
//! (an InkML part behind a `contentPart`) is read.

mod common;

use common::{assert_well_formed, part, part_names};
use opensheet_engine::drawing::ShapeTool;
use opensheet_engine::ink::PenKind;
use opensheet_engine::{Color, Locale, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

fn wave() -> Vec<(f64, f64)> {
    (0..=40).map(|i| (120.0 + 5.0 * i as f64, 90.0 + 30.0 * (i as f64 / 4.0).sin())).collect()
}

#[test]
fn strokes_come_back_as_they_were_drawn() {
    let mut wb = Workbook::new(Locale::en());
    let pen = wb.draw_ink(0, PenKind::Pen, Color(0xE71224), 2.0, &wave()).unwrap();
    let marker = wb
        .draw_ink(0, PenKind::Highlighter, Color(0xFFFC00), 14.0, &[(60.0, 200.0), (260.0, 204.0)])
        .unwrap();
    let pencil = wb.draw_ink(0, PenKind::Pencil, Color(0x333333), 1.0, &[(30.0, 30.0), (40.0, 60.0), (55.0, 30.0)]).unwrap();
    let dot = wb.draw_ink(0, PenKind::Pen, Color::BLACK, 3.0, &[(300.0, 300.0)]).unwrap();
    // One was stretched: its line grew with it.
    let frame = wb.ink_frame(0, pencil).unwrap();
    wb.set_ink_frame(0, pencil, opensheet_engine::drawing::Frame::new(frame.x, frame.y, frame.width * 2.0, frame.height * 2.0));
    wb.update_ink(0, marker, |ink| ink.locked = false);
    wb.commit("Ink");

    let bytes = to_bytes(&wb);
    common::keep_sample("ink.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // Freeform shapes behind a choice only this application takes.
    assert_eq!(xml.matches("<mc:AlternateContent").count(), 4);
    assert_eq!(xml.matches("<mc:Fallback><xdr:twoCellAnchor editAs=\"oneCell\">").count(), 4);
    assert_eq!(xml.matches("<a:custGeom>").count(), 4);
    assert!(xml.contains("<a:srgbClr val=\"FFFC00\"><a:alpha val=\"50000\"/></a:srgbClr>"), "{xml}");
    assert!(xml.contains("<a:ln w=\"19050\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"E71224\"/>"), "{xml}");
    assert!(xml.contains("<xdr:cNvPr id=\"2\" name=\"Ink 1\"/>"));
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").contains("<drawing r:id=\"rId1\"/>"));

    let back = load(&bytes);
    let sheet = &back.sheets[0];
    assert_eq!(sheet.inks.len(), 4);
    assert!(sheet.drawings.is_empty(), "the strokes are not shapes too");
    for id in [pen, marker, pencil, dot] {
        let (was, is) = (wb.ink(0, id).unwrap(), back.ink(0, id).unwrap());
        assert_eq!((is.kind, is.color, &is.name, is.locked), (was.kind, was.color, &was.name, was.locked));
        assert!((is.width - was.width).abs() < 0.01, "{} against {}", is.width, was.width);
        assert_eq!(is.points.len(), was.points.len());
        for (a, b) in is.points.iter().zip(was.points.iter()) {
            assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01, "{a:?} against {b:?}");
        }
        let (a, b) = (back.ink_frame(0, id).unwrap(), wb.ink_frame(0, id).unwrap());
        for (x, y) in [(a.x, b.x), (a.y, b.y), (a.width, b.width), (a.height, b.height)] {
            assert!((x - y).abs() < 0.01, "{a:?} against {b:?}");
        }
    }
    // What was read is on the grid of the file: from there on, writing
    // and reading change nothing.
    let second = to_bytes(&back);
    let third = to_bytes(&load(&second));
    let same = part(&second, "xl/drawings/drawing1.xml") == part(&third, "xl/drawings/drawing1.xml");
    assert!(same, "a second round trip changes the drawing part");
}

#[test]
fn ink_shares_the_drawing_part_with_shapes() {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_shape(1, ShapeTool::Ellipse, opensheet_engine::drawing::Frame::new(10.0, 10.0, 80.0, 40.0)).unwrap();
    wb.draw_ink(1, PenKind::Pen, Color::BLACK, 2.0, &wave()).unwrap();
    wb.commit("Ink");
    let bytes = to_bytes(&wb);
    let names = part_names(&bytes);
    assert!(names.iter().any(|n| n == "xl/drawings/drawing1.xml"));
    assert!(!names.iter().any(|n| n == "xl/drawings/drawing2.xml"));
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // The shape has the first id, the stroke the next.
    assert!(xml.contains("<xdr:cNvPr id=\"2\" name=\"Oval 1\"") || xml.contains("id=\"2\""), "{xml}");
    assert!(xml.contains("<xdr:cNvPr id=\"3\" name=\"Ink 1\"/>"), "{xml}");
    let back = load(&bytes);
    assert_eq!((back.sheets[1].drawings.len(), back.sheets[1].inks.len()), (1, 1));
    assert!(back.sheets[0].inks.is_empty());
}

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// A package as Excel writes one with a single ink object of two strokes.
fn excel_ink() -> Vec<u8> {
    let content_types = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>\
<Override PartName=\"/xl/ink/ink1.xml\" ContentType=\"application/inkml+xml\"/></Types>"
    );
    let rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
    );
    let workbook = format!(
        "{DECL}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Feuil1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>"
    );
    let sheet = format!(
        "{DECL}<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheetData/><drawing r:id=\"rId1\"/></worksheet>"
    );
    let sheet_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing1.xml\"/></Relationships>"
    );
    // 64-pixel columns and 20-pixel rows: from (64, 20) to (256, 100).
    let drawing = format!(
        "{DECL}<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\">\
<mc:Choice xmlns:xdr14=\"http://schemas.microsoft.com/office/excel/2010/spreadsheetDrawing\" Requires=\"xdr14\">\
<xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>\
<xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>\
<xdr14:contentPart xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId1\" bwMode=\"auto\">\
<xdr14:nvContentPartPr><xdr14:cNvPr id=\"3\" name=\"Encre 2\"/><xdr14:cNvContentPartPr/></xdr14:nvContentPartPr>\
<xdr14:xfrm><a:off x=\"609600\" y=\"190500\"/><a:ext cx=\"1828800\" cy=\"762000\"/></xdr14:xfrm></xdr14:contentPart><xdr:clientData/></xdr:twoCellAnchor></mc:Choice>\
<mc:Fallback><xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>\
<xdr:to><xdr:col>4</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>\
<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"3\" name=\"Encre 2\"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill>\
<a:blip xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"rId2\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>\
<xdr:spPr><a:xfrm><a:off x=\"609600\" y=\"190500\"/><a:ext cx=\"1828800\" cy=\"762000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>\
<xdr:clientData/></xdr:twoCellAnchor></mc:Fallback></mc:AlternateContent></xdr:wsDr>"
    );
    let drawing_rels = format!(
        "{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.microsoft.com/office/2007/relationships/customXml\" Target=\"../ink/ink1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image1.png\"/></Relationships>"
    );
    let ink = format!(
        "{DECL}<inkml:ink xmlns:inkml=\"http://www.w3.org/2003/InkML\"><inkml:definitions><inkml:context xml:id=\"ctx0\">\
<inkml:inkSource xml:id=\"inkSrc0\"><inkml:traceFormat><inkml:channel name=\"X\" type=\"integer\" min=\"-2.14748E9\" max=\"2.14748E9\" units=\"cm\"/>\
<inkml:channel name=\"Y\" type=\"integer\" min=\"-2.14748E9\" max=\"2.14748E9\" units=\"cm\"/><inkml:channel name=\"F\" type=\"integer\" max=\"32767\" units=\"dev\"/></inkml:traceFormat>\
<inkml:channelProperties><inkml:channelProperty channel=\"X\" name=\"resolution\" value=\"1000\" units=\"1/cm\"/>\
<inkml:channelProperty channel=\"Y\" name=\"resolution\" value=\"1000\" units=\"1/cm\"/><inkml:channelProperty channel=\"F\" name=\"resolution\" value=\"0\" units=\"1/dev\"/>\
</inkml:channelProperties></inkml:inkSource><inkml:timestamp xml:id=\"ts0\" timeString=\"2026-10-12T10:00:00.000\"/></inkml:context>\
<inkml:brush xml:id=\"br0\"><inkml:brushProperty name=\"width\" value=\"0.05\" units=\"cm\"/><inkml:brushProperty name=\"height\" value=\"0.05\" units=\"cm\"/>\
<inkml:brushProperty name=\"color\" value=\"#E71224\"/></inkml:brush></inkml:definitions>\
<inkml:trace contextRef=\"#ctx0\" brushRef=\"#br0\">25 25 16383,2540 1041 16383,5055 25 16383</inkml:trace>\
<inkml:trace contextRef=\"#ctx0\" brushRef=\"#br0\">25 2091 16383,5055 2091 16383</inkml:trace></inkml:ink>"
    );
    common::package(&[
        ("[Content_Types].xml", &content_types),
        ("_rels/.rels", &rels),
        ("xl/workbook.xml", &workbook),
        ("xl/_rels/workbook.xml.rels", &workbook_rels),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/worksheets/_rels/sheet1.xml.rels", &sheet_rels),
        ("xl/drawings/drawing1.xml", &drawing),
        ("xl/drawings/_rels/drawing1.xml.rels", &drawing_rels),
        ("xl/ink/ink1.xml", &ink),
    ])
}

#[test]
fn the_ink_of_excel_is_shown() {
    let wb = load(&excel_ink());
    let sheet = &wb.sheets[0];
    assert!(sheet.drawings.is_empty(), "the fallback picture is not taken");
    assert_eq!(sheet.inks.len(), 2);
    let (first, second) = (&sheet.inks[0], &sheet.inks[1]);
    assert_eq!((first.kind, first.color), (PenKind::Pen, Color(0xE71224)));
    assert_eq!((first.name.as_str(), second.name.as_str()), ("Encre 2", "Encre 2 2"));
    // The two strokes fill the frame of the anchor, from (64, 20) to
    // (256, 100), less half the thickness of the line.
    let a = sheet.ink_frame(first);
    let b = sheet.ink_frame(second);
    assert!((a.x - 64.0).abs() < 1.5 && (a.y - 20.0).abs() < 1.5, "{a:?}");
    assert!((a.x + a.width - 256.0).abs() < 1.5, "{a:?}");
    assert!((b.y + b.height - 100.0).abs() < 2.0, "{b:?}");
    assert_eq!(first.points.len(), 3);
    // The middle of the first stroke is its lowest point.
    assert!(first.points[1].1 > first.points[0].1 && first.points[1].1 > first.points[2].1);
}
