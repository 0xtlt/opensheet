//! Pictures and shapes in `.xlsx` files: what the writer produces, that it
//! comes back unchanged, and what is made of the parts Excel writes.

mod common;

use common::{assert_well_formed, enter, part, part_names, value};
use opensheet_engine::ast::Expr;
use opensheet_engine::chart::{
    Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
};
use opensheet_engine::drawing::{
    Dash, Drawing, Frame, ImageFormat, LineEnd, Outline, Placement, ShapePreset, ShapeTool,
    TextAlign, TextAnchor,
};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::style::Underline;
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Set `OPENSHEET_DRAWING_SAMPLES` to a directory to keep the packages
/// these tests build, to open them in Excel.
fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_DRAWING_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}

/// The bytes of one part of a package, text or not.
fn raw_part(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a zip archive");
    let mut file = zip
        .by_name(name)
        .unwrap_or_else(|_| panic!("missing part {name}"));
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    data
}

/// A package from (name, bytes) pairs.
fn raw_package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

// ---- images --------------------------------------------------------------------

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

/// A real PNG image: `width` × `height` pixels of two colours in a
/// checkerboard of 4-pixel squares.
fn png(width: u32, height: u32, colors: [[u8; 3]; 2]) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        // Every row starts with its filter: none.
        pixels.push(0);
        for x in 0..width {
            pixels.extend_from_slice(&colors[((x / 4 + y / 4) % 2) as usize]);
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
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8 bits a channel, RGB.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (b"IHDR", header.as_slice()),
        (b"IDAT", stream.as_slice()),
        (b"IEND", &[]),
    ] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    out
}

/// The header of a JPEG image of 30 × 15 pixels at 72 dots an inch: enough
/// to know what it is and how large.
fn jpeg() -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[1, 1, 1, 0, 72, 0, 72, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x0B, 8, 0, 15, 0, 30, 1, 1, 0x11, 0]);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A GIF image of one white pixel.
fn gif() -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&[
        1, 0, 1, 0, 0x80, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0x21, 0xF9, 4, 1, 0, 0, 0, 0, 0x2C, 0,
        0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 1, 0, 0x3B,
    ]);
    out
}

/// A BMP image of 2 × 2 pixels.
fn bmp() -> Vec<u8> {
    let mut out = b"BM".to_vec();
    out.extend_from_slice(&70u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&2i32.to_le_bytes());
    out.extend_from_slice(&2i32.to_le_bytes());
    out.extend_from_slice(&[1, 0, 24, 0]);
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&[0, 0, 255, 255, 255, 255, 0, 0, 0, 255, 0, 255, 0, 0, 0, 0]);
    out
}

fn logo() -> Vec<u8> {
    png(48, 32, [[0x15, 0x60, 0x82], [0xE9, 0x71, 0x32]])
}

// ---- a workbook with everything --------------------------------------------------

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("bad range {name}"))
}

fn change(wb: &mut Workbook, sheet: usize, id: u32, edit: impl FnOnce(&mut Drawing)) {
    assert!(wb.update_drawing(sheet, id, edit), "no drawing {id}");
}

fn shape(wb: &mut Workbook, sheet: usize, tool: ShapeTool, frame: (f64, f64, f64, f64)) -> u32 {
    let (x, y, width, height) = frame;
    wb.add_shape(sheet, tool, Frame::new(x, y, width, height))
        .expect("a shape")
}

fn named<'a>(wb: &'a Workbook, sheet: usize, name: &str) -> &'a Drawing {
    wb.sheets[sheet]
        .drawings
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("no drawing {name}"))
}

/// Four sheets: every shape of the gallery with every setting, pictures of
/// every format beside a chart, drawings beside a note and a table, and a
/// sheet with nothing.
fn sample() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for n in 1..=3 {
        wb.add_sheet(n);
    }
    assert!(wb.rename_sheet(0, "Shapes"));
    assert!(wb.rename_sheet(1, "Pictures & chart"));
    assert!(wb.rename_sheet(2, "Everything"));
    assert!(wb.rename_sheet(3, "Nothing"));

    // Columns and rows of several sizes under the shapes, some hidden.
    wb.set_col_width(0, 1, Some(100.0));
    wb.set_col_width(0, 4, Some(30.0));
    wb.set_cols_hidden(0, 6, 6, true);
    wb.set_row_height(0, 2, Some(40.0));
    wb.set_row_height(0, 9, Some(12.0));

    // The whole gallery, in rows of six.
    let mut ids = Vec::new();
    for (n, tool) in ShapeTool::ALL.into_iter().enumerate() {
        let (width, height) = tool.default_size();
        let frame = (
            20.0 + (n % 6) as f64 * 160.0,
            10.0 + (n / 6) as f64 * 130.0,
            width as f64,
            if tool.preset().is_line() {
                40.0
            } else {
                height as f64
            },
        );
        ids.push(shape(&mut wb, 0, tool, frame));
    }
    let id = |tool: ShapeTool| ids[ShapeTool::ALL.iter().position(|t| *t == tool).unwrap()];

    change(&mut wb, 0, id(ShapeTool::Rectangle), |d| {
        d.name = "Box <1> & \"co\"".to_string();
        d.alt_text = "A box.\nWith two lines, <tags> & \"quotes\".".to_string();
        let s = d.as_shape_mut().unwrap();
        s.text = "Total: 1 < 2 & 3 > 2\n\n  third line, indented ".to_string();
        s.font.bold = true;
        s.font.italic = true;
        s.font.underline = Underline::Single;
        s.font.strike = true;
        s.font.size = 28;
        s.font.name = Arc::from("Georgia");
        s.font.color = Some(Color(0xFFC000));
        s.align = TextAlign::Right;
        s.anchor = TextAnchor::Bottom;
    });
    change(&mut wb, 0, id(ShapeTool::RoundedRectangle), |d| {
        let s = d.as_shape_mut().unwrap();
        s.fill = None;
        s.outline = Some(Outline {
            dash: Dash::SysDash,
            ..Outline::solid(Color(0xC00000), 2.25)
        });
        s.text = "Justified".to_string();
        s.font.underline = Underline::Double;
        s.font.color = None;
        s.align = TextAlign::Justify;
        s.anchor = TextAnchor::Top;
    });
    change(&mut wb, 0, id(ShapeTool::Ellipse), |d| {
        let s = d.as_shape_mut().unwrap();
        s.outline = None;
        s.fill = Some(Color(0x00B050));
        d.lock_aspect = true;
        d.locked = false;
        d.print = false;
    });
    // Turned a little, then on its side, then past the half turn.
    change(&mut wb, 0, id(ShapeTool::Triangle), |d| d.rotation = 30.0);
    change(&mut wb, 0, id(ShapeTool::RightArrow), |d| {
        d.rotation = 90.0;
        d.as_shape_mut().unwrap().text = "Down".to_string();
    });
    change(&mut wb, 0, id(ShapeTool::LeftArrow), |d| {
        d.rotation = 270.0;
        d.placement = Placement::MoveOnly;
    });
    change(&mut wb, 0, id(ShapeTool::Pentagon), |d| {
        d.rotation = 200.5;
        d.flip_h = true;
    });
    change(&mut wb, 0, id(ShapeTool::Hexagon), |d| {
        d.flip_v = true;
        d.placement = Placement::Fixed;
    });
    change(&mut wb, 0, id(ShapeTool::Diamond), |d| {
        d.placement = Placement::MoveOnly;
        d.as_shape_mut().unwrap().outline = Some(Outline {
            dash: Dash::LongDashDotDot,
            ..Outline::solid(Color(0x7030A0), 0.25)
        });
    });
    // Lines: the other diagonal, every end, a line that carries text.
    change(&mut wb, 0, id(ShapeTool::Line), |d| {
        d.flip_v = true;
        d.as_shape_mut().unwrap().outline = Some(Outline {
            dash: Dash::Dot,
            ..Outline::solid(Color(0x000000), 3.0)
        });
    });
    change(&mut wb, 0, id(ShapeTool::Arrow), |d| {
        d.flip_h = true;
        d.rotation = 15.0;
        d.as_shape_mut().unwrap().outline = Some(Outline {
            color: Color(0xFF0000),
            width: 1.5,
            dash: Dash::DashDot,
            start: LineEnd::Oval,
            end: LineEnd::Stealth,
        });
    });
    change(&mut wb, 0, id(ShapeTool::DoubleArrow), |d| {
        let s = d.as_shape_mut().unwrap();
        s.text = "both ways".to_string();
        s.outline = Some(Outline {
            start: LineEnd::Diamond,
            end: LineEnd::Open,
            ..Outline::solid(Color(0x0070C0), 1.0)
        });
    });
    change(&mut wb, 0, id(ShapeTool::TextBox), |d| {
        d.as_shape_mut().unwrap().text = "Élan & co.\nSecond paragraph".to_string();
    });
    change(&mut wb, 0, id(ShapeTool::Callout), |d| {
        d.as_shape_mut().unwrap().text = "Look".to_string();
        d.alt_text = "A speech bubble".to_string();
    });

    // Pictures of every format beside a chart; the logo twice, once
    // resized and turned on its side.
    for (i, value) in ["3", "1", "2"].into_iter().enumerate() {
        wb.set_cell_text(1, i as u32, 0, value).unwrap();
    }
    let values = match parse_formula("'Pictures & chart'!$A$1:$A$3", Locale::en()) {
        Ok(Expr::Ref(range)) => range,
        other => panic!("not a reference: {other:?}"),
    };
    let chart = Chart {
        id: 0,
        kind: ChartKind::Column,
        title: ChartTitle::Auto,
        legend: LegendPos::None,
        series: vec![Series {
            name: SeriesName::None,
            categories: None,
            color: None,
            ..Series::new(values)
        }],
        data_labels: false,
        anchor: (5, 6),
        offset: (0.0, 0.0),
        size: DEFAULT_CHART_SIZE,
        ..Chart::new(ChartKind::Column)
    };
    assert!(wb.add_chart(1, chart).is_some());
    wb.set_col_width(1, 2, Some(20.0));
    let first = wb.add_picture(1, logo(), (1, 2)).expect("a PNG image");
    let second = wb.add_picture(1, logo(), (8, 1)).expect("the same again");
    change(&mut wb, 1, second, |d| {
        d.size = (96.0, 64.0);
        d.rotation = 90.0;
        d.placement = Placement::MoveAndSize;
        d.lock_aspect = false;
        d.alt_text = "The logo, on its side".to_string();
    });
    change(&mut wb, 1, first, |d| d.name = "Logo".to_string());
    assert!(wb.add_picture(1, jpeg(), (14, 0)).is_some());
    let small = wb.add_picture(1, gif(), (14, 2)).expect("a GIF image");
    change(&mut wb, 1, small, |d| {
        d.size = (24.0, 24.0);
        d.placement = Placement::Fixed;
        d.flip_h = true;
        d.print = false;
    });
    assert!(wb.add_picture(1, bmp(), (14, 4)).is_some());
    shape(&mut wb, 1, ShapeTool::Star5, (300.0, 20.0, 60.0, 60.0));

    // A sheet whose worksheet points at everything it can.
    enter(&mut wb, 2, "A1", "Item");
    enter(&mut wb, 2, "B1", "Qty");
    enter(&mut wb, 2, "A2", "nut");
    enter(&mut wb, 2, "B2", "5");
    wb.add_table(2, rect("A1:B2"), true).unwrap();
    assert!(wb.set_note(2, 4, 0, "Ann", "Ann:\nbeside drawings"));
    assert!(wb.add_picture(2, logo(), (6, 3)).is_some());
    shape(&mut wb, 2, ShapeTool::Arrow, (200.0, 30.0, 80.0, 0.0));
    wb.commit("drawings");
    wb
}

/// The drawings of two workbooks are the same, whatever their ids.
fn assert_same_drawings(a: &Workbook, b: &Workbook) {
    assert_eq!(a.sheets.len(), b.sheets.len());
    for (sa, sb) in a.sheets.iter().zip(&b.sheets) {
        assert_eq!(sa.drawings.len(), sb.drawings.len(), "{}", sa.name);
        for (da, db) in sa.drawings.iter().zip(&sb.drawings) {
            let mut db = db.clone();
            db.id = da.id;
            assert_eq!(*da, db, "{} on {}", da.name, sa.name);
            assert_eq!(
                sa.drawing_frame(da),
                sb.drawing_frame(&db),
                "{} on {}",
                da.name,
                sa.name
            );
        }
        assert_eq!(sa.charts.len(), sb.charts.len(), "{}", sa.name);
    }
}

#[test]
fn drawings_round_trip() {
    let original = sample();
    assert_eq!(original.sheets[0].drawings.len(), 18);
    assert_eq!(original.sheets[1].drawings.len(), 6);
    assert_eq!(original.sheets[2].drawings.len(), 2);
    let formats: Vec<ImageFormat> = original.sheets[1]
        .drawings
        .iter()
        .filter_map(|d| d.as_picture().map(|p| p.format))
        .collect();
    assert_eq!(
        formats,
        [
            ImageFormat::Png,
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::Bmp
        ]
    );
    let bytes = to_bytes(&original);
    keep("opensheet-drawings.xlsx", &bytes);
    let loaded = load(&bytes);
    assert_same_drawings(&original, &loaded);
    assert!(!loaded.modified && !loaded.can_undo());

    // Ids are unique in the workbook, in the order of the sheets.
    let ids: Vec<u32> = loaded
        .sheets
        .iter()
        .flat_map(|sheet| sheet.drawings.iter().map(|d| d.id))
        .collect();
    assert_eq!(ids, (1..=26).collect::<Vec<u32>>());
    // Pictures of one image still share its bytes, across sheets too.
    let image = |sheet: usize, name: &str| {
        named(&loaded, sheet, name)
            .as_picture()
            .unwrap()
            .data
            .clone()
    };
    assert!(Arc::ptr_eq(&image(1, "Logo"), &image(1, "Picture 3")));
    assert!(Arc::ptr_eq(&image(1, "Logo"), &image(2, "Picture 1")));
    assert_eq!(&*image(1, "Logo"), logo().as_slice());

    // What was read can be changed like what was made here.
    let mut changed = load(&bytes);
    let star = named(&changed, 1, "5-Point Star 7").id;
    assert!(changed.update_drawing(1, star, |d| d.rotation = 45.0));
    assert!(
        changed
            .add_shape(3, ShapeTool::Ellipse, Frame::new(5.0, 5.0, 50.0, 50.0))
            .is_some()
    );
    let again = load(&to_bytes(&changed));
    assert_same_drawings(&changed, &again);

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    for name in part_names(&bytes) {
        assert_eq!(raw_part(&bytes, &name), raw_part(&again, &name), "{name}");
    }
}

/// The parts of the package a relationships part points at.
fn targets(bytes: &[u8], rels: &str) -> Vec<(String, String, String)> {
    let base = rels.split("_rels/").next().unwrap();
    part(bytes, rels)
        .split("<Relationship ")
        .skip(1)
        .map(|rel| {
            let attr = |name: &str| {
                let start = rel.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                rel[start..].split('"').next().unwrap().to_string()
            };
            let target = attr("Target");
            let target = match target.strip_prefix("../") {
                Some(up) => format!("xl/{up}"),
                None => format!("{base}{target}"),
            };
            (
                attr("Id"),
                attr("Type").rsplit('/').next().unwrap().to_string(),
                target,
            )
        })
        .collect()
}

#[test]
fn package_lists_the_drawing_parts() {
    let bytes = to_bytes(&sample());
    let names = part_names(&bytes);
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    // One drawing part for each sheet that has something to draw, and
    // each image once.
    let under = |prefix: &str| {
        let mut found: Vec<&str> = names
            .iter()
            .filter(|n| n.starts_with(prefix))
            .map(|n| n.as_str())
            .collect();
        found.sort();
        found
    };
    assert_eq!(
        under("xl/drawings/"),
        [
            "xl/drawings/_rels/drawing1.xml.rels",
            "xl/drawings/_rels/drawing2.xml.rels",
            "xl/drawings/_rels/drawing3.xml.rels",
            "xl/drawings/drawing1.xml",
            "xl/drawings/drawing2.xml",
            "xl/drawings/drawing3.xml",
            "xl/drawings/vmlDrawing1.vml",
        ]
    );
    assert_eq!(
        under("xl/media/"),
        [
            "xl/media/image1.png",
            "xl/media/image2.jpeg",
            "xl/media/image3.gif",
            "xl/media/image4.bmp",
        ]
    );
    assert_eq!(raw_part(&bytes, "xl/media/image1.png"), logo());
    assert_eq!(raw_part(&bytes, "xl/media/image2.jpeg"), jpeg());
    assert_eq!(raw_part(&bytes, "xl/media/image3.gif"), gif());
    assert_eq!(raw_part(&bytes, "xl/media/image4.bmp"), bmp());
    let types = part(&bytes, "[Content_Types].xml");
    assert_well_formed("[Content_Types].xml", &types);
    for (extension, kind) in [
        ("png", "image/png"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("bmp", "image/bmp"),
    ] {
        assert_eq!(
            types
                .matches(&format!(
                    "<Default Extension=\"{extension}\" ContentType=\"{kind}\"/>"
                ))
                .count(),
            1,
            "{extension}"
        );
    }
    for n in 1..=3 {
        assert!(types.contains(&format!(
            "<Override PartName=\"/xl/drawings/drawing{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>"
        )));
        let name = format!("xl/drawings/drawing{n}.xml");
        assert_well_formed(&name, &part(&bytes, &name));
        let rels = format!("xl/drawings/_rels/drawing{n}.xml.rels");
        assert_well_formed(&rels, &part(&bytes, &rels));
    }

    // Sheet → drawing, under the id `<drawing>` names; the drawing comes
    // before the note boxes and the tables.
    let triple =
        |id: &str, kind: &str, target: &str| (id.to_string(), kind.to_string(), target.to_string());
    assert_eq!(
        targets(&bytes, "xl/worksheets/_rels/sheet1.xml.rels"),
        [triple("rId1", "drawing", "xl/drawings/drawing1.xml")]
    );
    assert_eq!(
        targets(&bytes, "xl/worksheets/_rels/sheet2.xml.rels"),
        [triple("rId1", "drawing", "xl/drawings/drawing2.xml")]
    );
    assert_eq!(
        targets(&bytes, "xl/worksheets/_rels/sheet3.xml.rels"),
        [
            triple("rId1", "drawing", "xl/drawings/drawing3.xml"),
            triple("rId2", "vmlDrawing", "xl/drawings/vmlDrawing1.vml"),
            triple("rId3", "comments", "xl/comments1.xml"),
            triple("rId4", "table", "xl/tables/table1.xml"),
        ]
    );
    assert!(!names.contains(&"xl/worksheets/_rels/sheet4.xml.rels".to_string()));
    for n in 1..=4 {
        let name = format!("xl/worksheets/sheet{n}.xml");
        let sheet = part(&bytes, &name);
        common::assert_worksheet_order(&name, &sheet);
        assert_eq!(sheet.contains("<drawing r:id=\"rId1\"/>"), n < 4, "{name}");
    }
    assert!(part(&bytes, "xl/worksheets/sheet3.xml").ends_with(
        "<drawing r:id=\"rId1\"/><legacyDrawing r:id=\"rId2\"/><tableParts count=\"1\"><tablePart r:id=\"rId4\"/></tableParts></worksheet>"
    ));

    // Drawing → what it shows: shapes need nothing, the chart comes
    // first, then each image once.
    assert_eq!(
        part(&bytes, "xl/drawings/_rels/drawing1.xml.rels"),
        format!("{DECL}<Relationships xmlns=\"{PKG_REL}\"></Relationships>")
    );
    assert_eq!(
        targets(&bytes, "xl/drawings/_rels/drawing2.xml.rels"),
        [
            triple("rId1", "chart", "xl/charts/chart1.xml"),
            triple("rId2", "image", "xl/media/image1.png"),
            triple("rId3", "image", "xl/media/image2.jpeg"),
            triple("rId4", "image", "xl/media/image3.gif"),
            triple("rId5", "image", "xl/media/image4.bmp"),
        ]
    );
    assert_eq!(
        targets(&bytes, "xl/drawings/_rels/drawing3.xml.rels"),
        [triple("rId1", "image", "xl/media/image1.png")]
    );
    let second = part(&bytes, "xl/drawings/drawing2.xml");
    let embeds: Vec<&str> = second
        .split("r:embed=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect();
    assert_eq!(embeds, ["rId2", "rId2", "rId3", "rId4", "rId5"]);
    // The objects of a part have ids of their own, the chart first.
    let ids: Vec<&str> = second
        .split("<xdr:cNvPr id=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect();
    assert_eq!(ids, ["2", "3", "4", "5", "6", "7", "8"]);
    assert!(second.find("<xdr:graphicFrame").unwrap() < second.find("<xdr:pic>").unwrap());

    // A workbook without drawings has none of this.
    let plain = to_bytes(&Workbook::new(Locale::en()));
    assert!(
        !part_names(&plain)
            .iter()
            .any(|n| n.contains("drawing") || n.contains("media"))
    );
}

/// One of each: a picture, a shape with text, an arrow, a text box that
/// stays where it is.
fn small() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    let picture = wb.add_picture(0, logo(), (1, 1)).unwrap();
    change(&mut wb, 0, picture, |d| d.alt_text = "Our logo".to_string());
    let hello = shape(&mut wb, 0, ShapeTool::Rectangle, (10.0, 5.0, 100.0, 50.0));
    change(&mut wb, 0, hello, |d| {
        d.as_shape_mut().unwrap().text = "Hi\nthere".to_string()
    });
    shape(&mut wb, 0, ShapeTool::Arrow, (200.0, 40.0, 80.0, 0.0));
    let note = shape(&mut wb, 0, ShapeTool::TextBox, (300.0, 100.0, 96.0, 48.0));
    change(&mut wb, 0, note, |d| {
        d.placement = Placement::Fixed;
        d.rotation = 90.0;
        d.lock_aspect = true;
        d.locked = false;
        d.print = false;
    });
    wb.commit("drawings");
    wb
}

/// `<xdr:from>` or `<xdr:to>`: a cell and a distance into it, in pixels.
fn corner(name: &str, col: u32, x: u32, row: u32, y: u32) -> String {
    format!(
        "<xdr:{name}><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{name}>",
        x * 9525,
        y * 9525
    )
}

#[test]
fn parts_say_what_the_model_says() {
    let bytes = to_bytes(&small());
    // Columns of 64 pixels and rows of 20; a pixel is 9525 EMU, a point
    // 12700.
    let run = |color: &str| {
        format!("{color}<a:latin typeface=\"Aptos Narrow\"/><a:cs typeface=\"Aptos Narrow\"/>")
    };
    let white = run("<a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>");
    let expected = [
        format!("{DECL}<xdr:wsDr xmlns:xdr=\"{XDR}\" xmlns:a=\"{A}\">"),
        // The picture, 48 × 32 pixels on B2: it moves with its cell and
        // keeps its size and its proportions.
        "<xdr:twoCellAnchor editAs=\"oneCell\">".to_string(),
        corner("from", 1, 0, 1, 0),
        corner("to", 1, 48, 2, 12),
        "<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"2\" name=\"Picture 1\" descr=\"Our logo\"/><xdr:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></xdr:cNvPicPr></xdr:nvPicPr>".to_string(),
        format!("<xdr:blipFill><a:blip xmlns:r=\"{REL}\" r:embed=\"rId1\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>"),
        "<xdr:spPr><a:xfrm><a:off x=\"609600\" y=\"190500\"/><a:ext cx=\"457200\" cy=\"304800\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:twoCellAnchor>".to_string(),
        // The rectangle at (10, 5), 100 × 50, with two paragraphs.
        "<xdr:twoCellAnchor>".to_string(),
        corner("from", 0, 10, 0, 5),
        corner("to", 1, 46, 2, 15),
        "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"3\" name=\"Rectangle 2\"/><xdr:cNvSpPr/></xdr:nvSpPr>".to_string(),
        "<xdr:spPr><a:xfrm><a:off x=\"95250\" y=\"47625\"/><a:ext cx=\"952500\" cy=\"476250\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"042433\"/></a:solidFill></a:ln></xdr:spPr>".to_string(),
        "<xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"ctr\"/><a:lstStyle/>".to_string(),
        format!("<a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr sz=\"1100\">{white}</a:rPr><a:t>Hi</a:t></a:r></a:p>"),
        format!("<a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr sz=\"1100\">{white}</a:rPr><a:t>there</a:t></a:r></a:p>"),
        "</xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor>".to_string(),
        // The arrow, a connector as Excel's own, from (200, 40) to
        // (280, 40).
        "<xdr:twoCellAnchor>".to_string(),
        corner("from", 3, 8, 2, 0),
        corner("to", 4, 24, 2, 0),
        "<xdr:cxnSp macro=\"\"><xdr:nvCxnSpPr><xdr:cNvPr id=\"4\" name=\"Straight Arrow Connector 3\"/><xdr:cNvCxnSpPr/></xdr:nvCxnSpPr>".to_string(),
        "<xdr:spPr><a:xfrm><a:off x=\"1905000\" y=\"381000\"/><a:ext cx=\"762000\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"156082\"/></a:solidFill><a:tailEnd type=\"triangle\"/></a:ln></xdr:spPr></xdr:cxnSp><xdr:clientData/></xdr:twoCellAnchor>".to_string(),
        // The text box at (300, 100), 96 × 48, turned on its side: the
        // anchor holds the box as it stands, 48 × 96 about the same
        // centre, `xfrm` the box before the turn.
        "<xdr:twoCellAnchor editAs=\"absolute\">".to_string(),
        corner("from", 5, 4, 3, 16),
        corner("to", 5, 52, 8, 12),
        "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"5\" name=\"TextBox 4\"/><xdr:cNvSpPr txBox=\"1\"><a:spLocks noChangeAspect=\"1\"/></xdr:cNvSpPr></xdr:nvSpPr>".to_string(),
        "<xdr:spPr><a:xfrm rot=\"5400000\"><a:off x=\"2857500\" y=\"952500\"/><a:ext cx=\"914400\" cy=\"457200\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"BCBCBC\"/></a:solidFill></a:ln></xdr:spPr>".to_string(),
        "<xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"t\"/><a:lstStyle/>".to_string(),
        format!("<a:p><a:pPr algn=\"l\"/><a:endParaRPr sz=\"1100\">{}</a:endParaRPr></a:p>", run("")),
        "</xdr:txBody></xdr:sp><xdr:clientData fLocksWithSheet=\"0\" fPrintsWithSheet=\"0\"/></xdr:twoCellAnchor>".to_string(),
        "</xdr:wsDr>".to_string(),
    ]
    .concat();
    assert_eq!(part(&bytes, "xl/drawings/drawing1.xml"), expected);
    assert_eq!(
        part(&bytes, "xl/drawings/_rels/drawing1.xml.rels"),
        format!(
            "{DECL}<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{REL}/image\" Target=\"../media/image1.png\"/></Relationships>"
        )
    );

    // In the workbook with everything: what each setting becomes.
    let bytes = to_bytes(&sample());
    let shapes = part(&bytes, "xl/drawings/drawing1.xml");
    for expected in [
        // Names and descriptions, escaped.
        "<xdr:cNvPr id=\"2\" name=\"Box &lt;1&gt; &amp; &quot;co&quot;\" descr=\"A box.&#10;With two lines, &lt;tags&gt; &amp; &quot;quotes&quot;.\"/>",
        // Text: a font, the paragraphs (an empty one keeps the font),
        // spaces kept.
        "<a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"b\"/><a:lstStyle/><a:p><a:pPr algn=\"r\"/><a:r><a:rPr sz=\"1400\" b=\"1\" i=\"1\" u=\"sng\" strike=\"sngStrike\"><a:solidFill><a:srgbClr val=\"FFC000\"/></a:solidFill><a:latin typeface=\"Georgia\"/><a:cs typeface=\"Georgia\"/></a:rPr><a:t>Total: 1 &lt; 2 &amp; 3 &gt; 2</a:t></a:r></a:p><a:p><a:pPr algn=\"r\"/><a:endParaRPr sz=\"1400\" b=\"1\" i=\"1\" u=\"sng\" strike=\"sngStrike\">",
        "<a:t>  third line, indented </a:t>",
        // No fill, a dashed outline; no outline.
        "<a:prstGeom prst=\"roundRect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"C00000\"/></a:solidFill><a:prstDash val=\"sysDash\"/></a:ln>",
        "<a:pPr algn=\"just\"/><a:r><a:rPr sz=\"1100\" u=\"dbl\"><a:latin ",
        "<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"00B050\"/></a:solidFill><a:ln><a:noFill/></a:ln>",
        "<xdr:cNvSpPr><a:spLocks noChangeAspect=\"1\"/></xdr:cNvSpPr>",
        // Turns and flips.
        "<a:xfrm rot=\"1800000\">",
        "<a:xfrm rot=\"5400000\">",
        "<a:xfrm rot=\"16200000\">",
        "<a:xfrm rot=\"12030000\" flipH=\"1\">",
        "<a:xfrm flipV=\"1\">",
        "<a:ln w=\"3175\"><a:solidFill><a:srgbClr val=\"7030A0\"/></a:solidFill><a:prstDash val=\"lgDashDotDot\"/></a:ln>",
        // A line without ends is a shape; arrows are connectors unless
        // they carry text.
        "<xdr:cNvPr id=\"15\" name=\"Straight Connector 14\"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm flipV=\"1\">",
        "<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"38100\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill><a:prstDash val=\"dot\"/></a:ln>",
        "<xdr:cxnSp macro=\"\"><xdr:nvCxnSpPr><xdr:cNvPr id=\"16\" name=\"Straight Arrow Connector 15\"/><xdr:cNvCxnSpPr/></xdr:nvCxnSpPr><xdr:spPr><a:xfrm rot=\"900000\" flipH=\"1\">",
        "<a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom><a:ln w=\"19050\"><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill><a:prstDash val=\"dashDot\"/><a:headEnd type=\"oval\"/><a:tailEnd type=\"stealth\"/></a:ln></xdr:spPr></xdr:cxnSp>",
        "<a:prstGeom prst=\"line\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"0070C0\"/></a:solidFill><a:headEnd type=\"diamond\"/><a:tailEnd type=\"arrow\"/></a:ln></xdr:spPr><xdr:txBody>",
        "<a:t>both ways</a:t>",
        "<xdr:cNvSpPr txBox=\"1\"/>",
        "<a:t>Élan &amp; co.</a:t>",
        "<a:prstGeom prst=\"wedgeRectCallout\"><a:avLst/></a:prstGeom>",
    ] {
        assert!(shapes.contains(expected), "{expected}\n{shapes}");
    }
    for preset in ShapePreset::ALL {
        assert!(
            shapes.contains(&format!("<a:prstGeom prst=\"{}\">", preset.ooxml_name())),
            "{preset:?}"
        );
    }
    // How each follows the cells, and what is locked or printed.
    assert_eq!(shapes.matches("<xdr:twoCellAnchor>").count(), 15);
    assert_eq!(
        shapes
            .matches("<xdr:twoCellAnchor editAs=\"oneCell\">")
            .count(),
        2
    );
    assert_eq!(
        shapes
            .matches("<xdr:twoCellAnchor editAs=\"absolute\">")
            .count(),
        1
    );
    assert_eq!(
        shapes
            .matches("<xdr:clientData fLocksWithSheet=\"0\" fPrintsWithSheet=\"0\"/>")
            .count(),
        1
    );
    assert_eq!(shapes.matches("<xdr:clientData/>").count(), 17);
    let pictures = part(&bytes, "xl/drawings/drawing2.xml");
    assert!(pictures.contains(
        "<xdr:cNvPr id=\"4\" name=\"Picture 3\" descr=\"The logo, on its side\"/><xdr:cNvPicPr/>"
    ));
    assert!(pictures.contains("<a:xfrm flipH=\"1\"><a:off "));
    assert!(pictures.contains("<xdr:clientData fPrintsWithSheet=\"0\"/>"));
}
