//! The background picture of a sheet in `.xlsx` files: where the writer
//! puts it, that it comes back, and one written the way Excel does.

mod common;

use common::{assert_well_formed, enter, keep_sample, package, part, part_names};
use opensheet_engine::drawing::{ImageFormat, Picture};
use opensheet_engine::{Locale, Rect, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::io::Cursor;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + *byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// A real PNG image: a tile of `side` pixels, pale with a darker frame,
/// so that the tiling shows.
fn tile(side: u32, pale: [u8; 3], dark: [u8; 3]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..side {
        // Every row starts with its filter: none.
        pixels.push(0);
        for x in 0..side {
            let edge = x == 0 || y == 0 || x == side - 1 || y == side - 1;
            pixels.extend_from_slice(if edge { &dark } else { &pale });
        }
    }
    // A zlib stream whose blocks are stored as they are.
    let mut stream = vec![0x78, 0x01];
    let mut blocks = pixels.chunks(60_000).peekable();
    while let Some(block) = blocks.next() {
        stream.push(u8::from(blocks.peek().is_none()));
        stream.extend_from_slice(&(block.len() as u16).to_le_bytes());
        stream.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        stream.extend_from_slice(block);
    }
    stream.extend_from_slice(&adler32(&pixels).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&side.to_be_bytes());
    header.extend_from_slice(&side.to_be_bytes());
    // 8 bits a channel, RGB.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [(b"IHDR", header.as_slice()), (b"IDAT", stream.as_slice()), (b"IEND", &[])] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    out
}

/// The header of a JPEG image of 30 × 15 pixels: enough to know what it
/// is and how large.
fn jpeg() -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[1, 1, 1, 0, 72, 0, 72, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 15, 0, 30, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new())).unwrap().into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).unwrap()
}

/// Three sheets: a background over cells with and without a fill, next
/// to a table; a JPEG background on a sheet that also shows the first
/// picture as a floating object; a sheet without one.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    wb.add_sheet(2);
    let paper = tile(48, [0xFF, 0xF8, 0xE1], [0xE0, 0xC0, 0x80]);
    for (name, text) in [("B2", "Month"), ("C2", "Units"), ("B3", "Jan"), ("C3", "12"), ("B4", "Feb"), ("C4", "15")] {
        enter(&mut wb, 0, name, text);
    }
    wb.update_style(0, Rect::parse("E2:F4").unwrap(), &|s| s.fill = Some(opensheet_engine::Color(0xC6E0B4)));
    enter(&mut wb, 0, "E2", "Filled cells hide the background");
    wb.add_table(0, Rect::parse("B2:C4").unwrap(), true).expect("a table");
    assert!(wb.set_sheet_background(0, Picture::new(paper.clone())));
    assert!(wb.set_sheet_background(1, Picture::new(jpeg())));
    wb.add_picture(1, paper, (2, 2)).expect("the same image as a picture");
    wb.commit("backgrounds");
    wb
}

#[test]
fn backgrounds_round_trip() {
    let wb = sample();
    let bytes = to_bytes(&wb);
    keep_sample("objects-sheet-background.xlsx", &bytes);
    for name in part_names(&bytes) {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_well_formed(&name, &part(&bytes, &name));
        }
    }

    // The element sits where the schema wants it: after the page setup
    // and the drawing, before the tables.
    let first = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(first.contains("<picture r:id=\"rId2\"/><tableParts count=\"1\"><tablePart r:id=\"rId1\"/></tableParts></worksheet>"), "{first}");
    let second = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(second.contains("<drawing r:id=\"rId1\"/><picture r:id=\"rId2\"/></worksheet>"), "{second}");
    assert!(!part(&bytes, "xl/worksheets/sheet3.xml").contains("<picture"));

    // One file for the image two sheets show, another for the JPEG.
    let image = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(rels.contains(&format!("<Relationship Id=\"rId2\" Type=\"{image}\" Target=\"../media/image1.png\"/>")), "{rels}");
    let rels = part(&bytes, "xl/worksheets/_rels/sheet2.xml.rels");
    assert!(rels.contains(&format!("<Relationship Id=\"rId2\" Type=\"{image}\" Target=\"../media/image2.jpeg\"/>")), "{rels}");
    assert!(part(&bytes, "xl/drawings/_rels/drawing1.xml.rels").contains("Target=\"../media/image1.png\""));
    let media: Vec<String> = part_names(&bytes).into_iter().filter(|name| name.starts_with("xl/media/")).collect();
    assert_eq!(media.len(), 2, "{media:?}");
    let types = part(&bytes, "[Content_Types].xml");
    assert!(types.contains("<Default Extension=\"png\" ContentType=\"image/png\"/>"), "{types}");
    assert!(types.contains("<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>"), "{types}");

    let loaded = load(&bytes);
    for (before, after) in wb.sheets.iter().zip(&loaded.sheets) {
        assert_eq!(before.background, after.background, "{}", before.name);
    }
    assert_eq!(loaded.sheets[1].background.as_ref().map(|p| (p.format, p.pixels)), Some((ImageFormat::Jpeg, (30, 15))));
    assert!(!loaded.modified && !loaded.can_undo());
    // A second trip changes nothing.
    let again = to_bytes(&loaded);
    for name in part_names(&bytes) {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
        }
    }
}

#[test]
fn a_background_written_by_excel_is_read() {
    const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
    let content_types = format!(
        "{DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/worksheets/sheet2.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"
    );
    let rels = |list: &[(&str, &str, &str)]| {
        let mut xml = format!("{DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">");
        for (id, kind, target) in list {
            xml.push_str(&format!("<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"));
        }
        xml.push_str("</Relationships>");
        xml
    };
    let workbook = format!(
        "{DECL}<workbook {MAIN}><sheets><sheet name=\"With\" sheetId=\"1\" r:id=\"rId1\"/><sheet name=\"Broken\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"
    );
    // As Excel writes it: after the page margins and setup.
    let sheet = |id: &str| {
        format!(
            "{DECL}<worksheet {MAIN} xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>a &lt;picture of words</t></is></c></row></sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/><pageSetup orientation=\"portrait\"/><picture r:id=\"{id}\"/></worksheet>"
        )
    };
    let image = tile(8, [255, 255, 255], [0, 0, 0]);
    let text_parts: Vec<(String, String)> = vec![
        ("[Content_Types].xml".into(), content_types),
        ("_rels/.rels".into(), rels(&[("rId1", "officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml".into(), workbook),
        (
            "xl/_rels/workbook.xml.rels".into(),
            rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "worksheets/sheet2.xml")]),
        ),
        ("xl/worksheets/sheet1.xml".into(), sheet("rId1")),
        ("xl/worksheets/_rels/sheet1.xml.rels".into(), rels(&[("rId1", "image", "../media/image1.png")])),
        // A relationship to nothing, and to something that is no image.
        ("xl/worksheets/sheet2.xml".into(), sheet("rId7")),
        ("xl/worksheets/_rels/sheet2.xml.rels".into(), rels(&[("rId7", "image", "../media/gone.png")])),
    ];
    let borrowed: Vec<(&str, &str)> = text_parts.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
    // The image goes into the archive as bytes.
    let mut zip = zip::ZipArchive::new(Cursor::new(package(&borrowed))).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for i in 0..zip.len() {
        let file = zip.by_index(i).unwrap();
        out.raw_copy_file(file).unwrap();
    }
    use std::io::Write as _;
    out.start_file("xl/media/image1.png", options).unwrap();
    out.write_all(&image).unwrap();
    let bytes = out.finish().unwrap().into_inner();

    let wb = load(&bytes);
    let background = wb.sheets[0].background.as_ref().expect("the background");
    assert_eq!((background.format, background.pixels, &*background.data), (ImageFormat::Png, (8, 8), image.as_slice()));
    assert_eq!(wb.sheets[1].background, None);
    assert_eq!(wb.sheets[0].drawings.len(), 0);
}
