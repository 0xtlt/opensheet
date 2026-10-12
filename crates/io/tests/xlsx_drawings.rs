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

/// Keep a package these tests build, to open it in Excel (see
/// `common::keep_sample`).
fn keep(name: &str, bytes: &[u8]) {
    common::keep_sample(name, bytes);
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
            // Where it is first: a difference there is easier to read
            // without the bytes of a picture around it.
            let place = |d: &Drawing| {
                (
                    d.name.clone(),
                    d.placement,
                    (d.anchor, d.offset),
                    (d.to, d.to_offset),
                    d.size,
                    d.rotation,
                )
            };
            assert_eq!(place(da), place(&db), "on {}", sa.name);
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

// ---- parts written the way Excel writes them ------------------------------------

fn relationships(list: &[(&str, &str, &str)]) -> String {
    let mut x = format!("{DECL}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in list {
        x.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>"
        ));
    }
    x.push_str("</Relationships>");
    x
}

/// An anchor between two cells, as Excel writes them: (column, row) of
/// each corner, on the lines between cells.
fn between(from: (u32, u32), to: (u32, u32), edit_as: &str, content: &str) -> String {
    let edit_as = if edit_as.is_empty() {
        String::new()
    } else {
        format!(" editAs=\"{edit_as}\"")
    };
    format!(
        "<xdr:twoCellAnchor{edit_as}>{}{}{content}</xdr:twoCellAnchor>",
        corner("from", from.0, 0, from.1, 0),
        corner("to", to.0, 0, to.1, 0)
    )
}

/// The style Excel gives a new shape: everything comes from the theme.
const EXCEL_SHAPE_STYLE: &str = "<xdr:style><a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"15000\"/></a:schemeClr></a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef></xdr:style>";

/// A drawing part with what Excel puts in one: shapes styled by the theme,
/// a text box, a connector, pictures (one turned on its side, one linked,
/// one in a format the engine does not show), a group, a choice between a
/// shape and its fallback, anchors of the three kinds, and a chart frame.
fn excel_drawing() -> String {
    let creation = "<a:extLst><a:ext uri=\"{FF2B5EF4-FFF2-40B4-BE49-F238E27FC236}\"><a16:creationId xmlns:a16=\"http://schemas.microsoft.com/office/drawing/2014/main\" id=\"{00000000-0008-0000-0000-000002000000}\"/></a:ext></a:extLst>";
    let rectangle = format!(
        "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"2\" name=\"Rectangle 1\">{creation}</xdr:cNvPr><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"609600\" y=\"190500\"/><a:ext cx=\"1219200\" cy=\"762000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr>{EXCEL_SHAPE_STYLE}<xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" rtlCol=\"0\" anchor=\"t\"/><a:lstStyle/><a:p><a:pPr algn=\"l\"/><a:r><a:rPr lang=\"fr-FR\" sz=\"1100\"/><a:t>Bonjour </a:t></a:r><a:r><a:rPr lang=\"fr-FR\" sz=\"1100\" b=\"1\"/><a:t>tout le monde</a:t></a:r></a:p><a:p><a:pPr algn=\"l\"/><a:endParaRPr lang=\"fr-FR\" sz=\"1100\"/></a:p></xdr:txBody></xdr:sp><xdr:clientData/>"
    );
    let text_box = "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"3\" name=\"ZoneTexte 2\"/><xdr:cNvSpPr txBox=\"1\"/></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val=\"lt1\"/></a:solidFill><a:ln w=\"9525\" cmpd=\"sng\"><a:solidFill><a:schemeClr val=\"lt1\"><a:shade val=\"50000\"/></a:schemeClr></a:solidFill></a:ln></xdr:spPr><xdr:style><a:lnRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:lnRef><a:fillRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:fillRef><a:effectRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"dk1\"/></a:fontRef></xdr:style><xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"t\"/><a:lstStyle/><a:p><a:r><a:rPr lang=\"fr-FR\" sz=\"1100\"/><a:t>Une note</a:t></a:r><a:br><a:rPr lang=\"fr-FR\" sz=\"1100\"/></a:br><a:r><a:rPr lang=\"fr-FR\" sz=\"1100\"/><a:t>sur deux lignes</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData/>";
    let arrow = "<xdr:cxnSp macro=\"\"><xdr:nvCxnSpPr><xdr:cNvPr id=\"5\" name=\"Connecteur droit avec flèche 4\"/><xdr:cNvCxnSpPr/></xdr:nvCxnSpPr><xdr:spPr><a:xfrm flipV=\"1\"><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"straightConnector1\"><a:avLst/></a:prstGeom><a:ln><a:tailEnd type=\"triangle\"/></a:ln></xdr:spPr><xdr:style><a:lnRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:lnRef><a:fillRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"tx1\"/></a:fontRef></xdr:style></xdr:cxnSp><xdr:clientData/>";
    let picture = |id: u32, name: &str, blip: &str, turn: &str| {
        format!(
            "<xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"{name}\" descr=\"Un logo\"/><xdr:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r=\"{REL}\" {blip}><a:extLst><a:ext uri=\"{{28A0092B-C50C-407E-A947-70E740481C1C}}\"/></a:extLst></a:blip><a:srcRect l=\"10000\" r=\"10000\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:xfrm{turn}><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/>"
        )
    };
    let plain = |id: u32, name: &str, geometry: &str, look: &str| {
        format!(
            "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"{id}\" name=\"{name}\"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm>{geometry}{look}</xdr:spPr>"
        )
    };
    let group = format!(
        "<xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id=\"20\" name=\"Groupe 19\"/><xdr:cNvGrpSpPr/></xdr:nvGrpSpPr><xdr:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"10\" cy=\"10\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"10\" cy=\"10\"/></a:xfrm></xdr:grpSpPr>{}{EXCEL_SHAPE_STYLE}</xdr:sp>{}{EXCEL_SHAPE_STYLE}</xdr:sp></xdr:grpSp><xdr:clientData/>",
        plain(
            21,
            "In a group",
            "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
            ""
        ),
        plain(
            22,
            "In a group too",
            "<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>",
            ""
        )
    );
    let choice = format!(
        "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>",
        between(
            (8, 1),
            (9, 3),
            "",
            &format!(
                "{}{EXCEL_SHAPE_STYLE}</xdr:sp><xdr:clientData/>",
                plain(
                    9,
                    "Ellipse 8",
                    "<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>",
                    ""
                )
            )
        ),
        between(
            (8, 1),
            (9, 3),
            "",
            &picture(9, "Fallback", "r:embed=\"rId1\"", "")
        )
    );
    let flow = format!(
        "<xdr:oneCellAnchor>{}<xdr:ext cx=\"952500\" cy=\"476250\"/>{}</xdr:sp><xdr:clientData fLocksWithSheet=\"0\"/></xdr:oneCellAnchor>",
        corner("from", 1, 0, 8, 0),
        plain(
            10,
            "Organigramme : Processus 9",
            "<a:prstGeom prst=\"flowChartProcess\"><a:avLst/></a:prstGeom>",
            "<a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"FF0000\"/></a:gs><a:gs pos=\"100000\"><a:srgbClr val=\"0000FF\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill><a:ln w=\"25400\"><a:pattFill prst=\"pct50\"><a:fgClr><a:srgbClr val=\"00FF00\"/></a:fgClr><a:bgClr><a:srgbClr val=\"FFFFFF\"/></a:bgClr></a:pattFill><a:round/></a:ln>"
        )
    );
    let free = format!(
        "<xdr:absoluteAnchor><xdr:pos x=\"4762500\" y=\"1905000\"/><xdr:ext cx=\"476250\" cy=\"476250\"/>{}<xdr:txBody><a:bodyPr anchor=\"ctr\"/><a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr sz=\"2000\" i=\"1\" u=\"dbl\" strike=\"sngStrike\"><a:solidFill><a:srgbClr val=\"112233\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:rPr><a:t>libre</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData fPrintsWithSheet=\"0\"/></xdr:absoluteAnchor>",
        plain(
            11,
            "Forme libre 10",
            "<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path w=\"10\" h=\"10\"><a:moveTo><a:pt x=\"0\" y=\"0\"/></a:moveTo><a:lnTo><a:pt x=\"10\" y=\"10\"/></a:lnTo></a:path></a:pathLst></a:custGeom>",
            "<a:noFill/><a:ln w=\"38100\"><a:solidFill><a:srgbClr val=\"00FF00\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln>"
        )
    );
    let chart = format!(
        "<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"12\" name=\"Graphique 11\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"{REL}\" r:id=\"rId7\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/>"
    );
    [
        format!("{DECL}<xdr:wsDr xmlns:xdr=\"{XDR}\" xmlns:a=\"{A}\">"),
        between((1, 1), (3, 5), "", &rectangle),
        between((4, 1), (6, 3), "oneCell", text_box),
        between((1, 12), (3, 14), "", arrow),
        // 100 × 40 pixels turned a quarter: the anchor is 40 × 100.
        format!(
            "<xdr:twoCellAnchor editAs=\"oneCell\">{}{}{}</xdr:twoCellAnchor>",
            corner("from", 5, 0, 5, 0),
            corner("to", 5, 40, 10, 0),
            picture(4, "Image 3", "r:embed=\"rId1\"", " rot=\"5400000\"")
        ),
        between(
            (6, 5),
            (7, 6),
            "",
            &picture(6, "Image 5", "r:link=\"rId9\"", ""),
        ),
        between(
            (6, 6),
            (7, 7),
            "",
            &picture(7, "Image 6", "r:embed=\"rId2\"", ""),
        ),
        between((0, 16), (2, 20), "", &group),
        choice,
        flow,
        free,
        between((10, 1), (14, 10), "", &chart),
        between(
            (1, 6),
            (2, 7),
            "",
            &format!(
                "{}{EXCEL_SHAPE_STYLE}</xdr:sp><xdr:clientData/>",
                plain(
                    13,
                    "Rectangle 1",
                    "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>",
                    ""
                )
            ),
        ),
        "</xdr:wsDr>".to_string(),
    ]
    .concat()
}

/// A package the way Excel saves one, with the theme of Office 2013.
/// Parts named in `replace` are changed, or left out for `None`.
fn excel_package(replace: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let workbook = format!(
        r#"{DECL}<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Dessins" sheetId="1" r:id="rId1"/></sheets></workbook>"#
    );
    let sheet = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><sheetData><row r="1"><c r="A1"><v>42</v></c></row></sheetData><drawing r:id="rId1"/></worksheet>"#
    );
    let color = |name: &str, rgb: &str| format!("<a:{name}><a:srgbClr val=\"{rgb}\"/></a:{name}>");
    let theme = format!(
        r#"{DECL}<a:theme xmlns:a="{A}" name="Office"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>{}{}{}{}{}{}{}{}{}{}</a:clrScheme><a:fmtScheme name="Office"><a:fillStyleLst/><a:lnStyleLst><a:ln w="6350"/><a:ln w="12700"/><a:ln w="19050"/></a:lnStyleLst></a:fmtScheme></a:themeElements></a:theme>"#,
        color("dk2", "44546A"),
        color("lt2", "E7E6E6"),
        color("accent1", "4472C4"),
        color("accent2", "ED7D31"),
        color("accent3", "A5A5A5"),
        color("accent4", "FFC000"),
        color("accent5", "5B9BD5"),
        color("accent6", "70AD47"),
        color("hlink", "0563C1"),
        color("folHlink", "954F72"),
    );
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="emf" ContentType="image/x-emf"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/></Types>"#
    );
    let image = logo();
    let mut parts: Vec<(String, Vec<u8>)> = [
        ("[Content_Types].xml", content_types.into_bytes()),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]).into_bytes(),
        ),
        ("xl/workbook.xml", workbook.into_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            relationships(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "theme", "theme/theme1.xml"),
            ])
            .into_bytes(),
        ),
        ("xl/theme/theme1.xml", theme.into_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.into_bytes()),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            relationships(&[("rId1", "drawing", "../drawings/drawing1.xml")]).into_bytes(),
        ),
        ("xl/drawings/drawing1.xml", excel_drawing().into_bytes()),
        (
            "xl/drawings/_rels/drawing1.xml.rels",
            relationships(&[
                ("rId1", "image", "../media/image1.png"),
                ("rId2", "image", "../media/image2.emf"),
            ])
            .into_bytes(),
        ),
        ("xl/media/image1.png", image),
        (
            "xl/media/image2.emf",
            b"\x01\0\0\0 not really a metafile".to_vec(),
        ),
    ]
    .into_iter()
    .map(|(name, data)| (name.to_string(), data))
    .collect();
    for (name, data) in replace {
        parts.retain(|(n, _)| n != name);
        if let Some(data) = data {
            parts.push((name.to_string(), data.to_vec()));
        }
    }
    let parts: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    raw_package(&parts)
}

#[test]
fn drawings_from_excel() {
    let bytes = excel_package(&[]);
    keep("excel-style-drawings.xlsx", &bytes);
    let wb = load(&bytes);
    assert_eq!(value(&wb, 0, "A1"), Value::Number(42.0));
    let sheet = &wb.sheets[0];
    let names: Vec<&str> = sheet.drawings.iter().map(|d| d.name.as_str()).collect();
    // The linked picture and the metafile are left out; of the choice,
    // the shape is taken and not its fallback. Two drawings of one name:
    // the second gets another.
    assert_eq!(
        names,
        [
            "Rectangle 1",
            "ZoneTexte 2",
            "Connecteur droit avec flèche 4",
            "Image 3",
            "Groupe 19",
            "Ellipse 8",
            "Organigramme : Processus 9",
            "Forme libre 10",
            "Rectangle 1 9",
        ]
    );
    assert!(sheet.charts.is_empty());
    let frame = |name: &str| {
        let f = sheet.drawing_frame(named(&wb, 0, name));
        (f.x, f.y, f.width, f.height)
    };
    let look = |name: &str| named(&wb, 0, name).as_shape().unwrap().clone();

    // A new shape of Excel 2013 to 2021: the theme's first accent, its
    // outline a shade of it one point wide, white text.
    let d = named(&wb, 0, "Rectangle 1");
    assert_eq!(frame("Rectangle 1"), (64.0, 20.0, 128.0, 80.0));
    assert_eq!(d.placement, Placement::MoveAndSize);
    assert!(d.locked && d.print && !d.lock_aspect && d.alt_text.is_empty());
    let s = look("Rectangle 1");
    assert_eq!(s.preset, ShapePreset::Rect);
    assert!(!s.text_box);
    assert_eq!(s.fill, Some(Color(0x4472C4)));
    assert_eq!(s.outline, Some(Outline::solid(Color(0x172C51), 1.0)));
    // One font for the whole text: that of its first run.
    assert_eq!(s.text, "Bonjour tout le monde\n");
    assert_eq!((s.font.size, s.font.bold), (22, false));
    assert_eq!(s.font.color, Some(Color(0xFFFFFF)));
    assert_eq!((s.align, s.anchor), (TextAlign::Left, TextAnchor::Top));

    // A text box: white, a grey hairline, black text; a line break is a
    // paragraph here.
    let s = look("ZoneTexte 2");
    assert!(s.text_box);
    assert_eq!(s.fill, Some(Color(0xFFFFFF)));
    assert_eq!(s.outline, Some(Outline::solid(Color(0xBCBCBC), 0.75)));
    assert_eq!(s.text, "Une note\nsur deux lignes");
    assert_eq!(s.font.color, None);
    assert_eq!(named(&wb, 0, "ZoneTexte 2").placement, Placement::MoveOnly);
    assert_eq!(frame("ZoneTexte 2"), (256.0, 20.0, 128.0, 40.0));

    // An arrow: a connector, half a point wide in this theme.
    let d = named(&wb, 0, "Connecteur droit avec flèche 4");
    let s = look(&d.name);
    assert_eq!(s.preset, ShapePreset::Line);
    assert_eq!(s.fill, None);
    assert_eq!(
        s.outline,
        Some(Outline {
            end: LineEnd::Triangle,
            ..Outline::solid(Color(0x4472C4), 0.5)
        })
    );
    assert!(d.flip_v && !d.flip_h);
    assert_eq!((s.align, s.anchor), (TextAlign::Center, TextAnchor::Middle));

    // The picture stands on its side in the anchor: 100 × 40 before the
    // turn, about the same centre.
    let d = named(&wb, 0, "Image 3");
    assert_eq!(d.rotation, 90.0);
    assert_eq!(frame("Image 3"), (290.0, 130.0, 100.0, 40.0));
    assert_eq!(d.alt_text, "Un logo");
    assert!(d.lock_aspect);
    assert_eq!(d.placement, Placement::MoveOnly);
    let image = d.as_picture().unwrap();
    assert_eq!((image.format, image.pixels), (ImageFormat::Png, (48, 32)));
    assert_eq!(&*image.data, logo().as_slice());

    assert_eq!(look("Ellipse 8").preset, ShapePreset::Ellipse);
    assert_eq!(frame("Ellipse 8"), (512.0, 20.0, 64.0, 40.0));

    // One corner and a size; a geometry the engine does not have is a
    // rectangle, a gradient its first colour, a pattern its foreground.
    let d = named(&wb, 0, "Organigramme : Processus 9");
    assert_eq!(d.placement, Placement::MoveOnly);
    assert!(!d.locked && d.print);
    assert_eq!(frame(&d.name), (64.0, 160.0, 100.0, 50.0));
    let s = look(&d.name);
    assert_eq!(s.preset, ShapePreset::Rect);
    assert_eq!(s.fill, Some(Color(0xFF0000)));
    assert_eq!(s.outline, Some(Outline::solid(Color(0x00FF00), 2.0)));

    // A place on the sheet and a size.
    let d = named(&wb, 0, "Forme libre 10");
    assert_eq!(d.placement, Placement::Fixed);
    assert!(d.locked && !d.print);
    assert_eq!(frame(&d.name), (500.0, 200.0, 50.0, 50.0));
    let s = look(&d.name);
    assert_eq!(s.fill, None);
    assert_eq!(
        s.outline,
        Some(Outline {
            dash: Dash::Dash,
            ..Outline::solid(Color(0x00FF00), 3.0)
        })
    );
    assert_eq!(s.text, "libre");
    assert_eq!(
        (s.font.size, s.font.italic, s.font.strike),
        (40, true, true)
    );
    assert_eq!(s.font.underline, Underline::Double);
    assert_eq!(s.font.color, Some(Color(0x112233)));
    // A font of the theme is the font of the workbook.
    assert_eq!(s.font.name, opensheet_engine::style::Font::default().name);
    assert_eq!((s.align, s.anchor), (TextAlign::Center, TextAnchor::Middle));

    // Saved by us and read again, nothing moves.
    let ours = to_bytes(&wb);
    keep("excel-style-drawings-saved.xlsx", &ours);
    let again = load(&ours);
    assert_same_drawings(&wb, &again);
    assert_eq!(
        part_names(&ours)
            .iter()
            .filter(|n| n.starts_with("xl/media/"))
            .count(),
        1
    );
}

#[test]
fn a_bad_drawing_costs_only_itself() {
    let drawing = "xl/drawings/drawing1.xml";
    let load_with = |replace: &[(&str, Option<&[u8]>)], what: &str| {
        let wb = load(&excel_package(replace));
        assert_eq!(wb.sheets.len(), 1, "{what}");
        assert_eq!(value(&wb, 0, "A1"), Value::Number(42.0), "{what}");
        // And it is saved as such.
        let again = load(&to_bytes(&wb));
        assert_same_drawings(&wb, &again);
        wb
    };
    let count = |wb: &Workbook| {
        let pictures = wb.sheets[0]
            .drawings
            .iter()
            .filter(|d| d.as_picture().is_some())
            .count();
        (wb.sheets[0].drawings.len() - pictures, pictures)
    };
    assert_eq!(count(&load_with(&[], "as it is")), (8, 1));
    assert_eq!(
        count(&load_with(&[(drawing, None)], "no drawing part")),
        (0, 0)
    );
    assert_eq!(
        count(&load_with(
            &[("xl/worksheets/_rels/sheet1.xml.rels", None)],
            "a sheet that leads nowhere"
        )),
        (0, 0)
    );
    // Without their image, pictures are left out; the shapes stay.
    assert_eq!(
        count(&load_with(
            &[("xl/drawings/_rels/drawing1.xml.rels", None)],
            "a drawing that leads nowhere"
        )),
        (8, 0)
    );
    assert_eq!(
        count(&load_with(&[("xl/media/image1.png", None)], "no image")),
        (8, 0)
    );
    assert_eq!(
        count(&load_with(
            &[(
                "xl/media/image1.png",
                Some(b"\x89PNG\r\n\x1a\n and no more")
            )],
            "half an image"
        )),
        (8, 0)
    );
    assert_eq!(
        count(&load_with(
            &[("xl/media/image1.png", Some(&jpeg()))],
            "an image that is not what its name says"
        )),
        (8, 1)
    );
    // No theme: the colours of the one Excel assumes.
    let wb = load_with(&[("xl/theme/theme1.xml", None)], "no theme");
    assert_eq!(count(&wb), (8, 1));
    assert!(
        named(&wb, 0, "Rectangle 1")
            .as_shape()
            .unwrap()
            .fill
            .is_some()
    );

    for junk in [
        "not xml at all",
        "<xdr:wsDr><xdr:twoCellAnchor><xdr:sp>",
        "<wsDr><twoCellAnchor editAs=\"bogus\"><from><col>99999999</col><colOff>1e999</colOff><row>-4</row><rowOff>NaN</rowOff></from><to><col>0</col><row>0</row></to><sp><nvSpPr><cNvPr id=\"x\" name=\"&#0;\"/></nvSpPr><spPr><xfrm rot=\"1e999\" flipH=\"2\"><off x=\"-1\" y=\"-1\"/></xfrm><prstGeom/><solidFill><srgbClr val=\"zz\"><shade val=\"1e999\"/><tint/></srgbClr></solidFill><ln w=\"-5\"><solidFill><schemeClr val=\"none\"/></solidFill><prstDash val=\"bogus\"/><headEnd type=\"bogus\"/></ln></spPr><style><lnRef idx=\"4000000000\"><srgbClr val=\"010203\"/></lnRef><fillRef idx=\"-1\"/><fontRef/></style><txBody><bodyPr anchor=\"bogus\"/><p><pPr algn=\"bogus\"/><r><rPr sz=\"1e999\" u=\"bogus\" strike=\"bogus\"><latin typeface=\"\"/></rPr><t>x</t></r><fld><t>y</t></fld><br/></p><p/></txBody></sp></twoCellAnchor>",
        "<wsDr><oneCellAnchor><from/><sp/></oneCellAnchor><oneCellAnchor><ext cx=\"1e999\" cy=\"-1\"/><pic><blipFill><blip embed=\"rId1\"/></blipFill></pic></oneCellAnchor><absoluteAnchor><pos x=\"9e99\" y=\"9e99\"/><ext cx=\"9e99\" cy=\"9e99\"/><pic><blipFill><blip embed=\"rId1\"/></blipFill><spPr><xfrm rot=\"5400000\"/></spPr></pic></absoluteAnchor><absoluteAnchor><pos/><sp/></absoluteAnchor></wsDr>",
    ] {
        let wb = load_with(&[(drawing, Some(junk.as_bytes()))], junk);
        for d in &wb.sheets[0].drawings {
            let frame = wb.sheets[0].drawing_frame(d);
            assert!(
                [frame.x, frame.y, frame.width, frame.height]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0),
                "{junk}: {frame:?}"
            );
            assert!(d.rotation.is_finite());
        }
    }
    // Wrappers nested without end are not followed to the bottom.
    let deep = format!(
        "<wsDr><twoCellAnchor>{}<sp/>{}</twoCellAnchor></wsDr>",
        "<AlternateContent><Choice>".repeat(5_000),
        "</Choice></AlternateContent>".repeat(5_000)
    );
    assert_eq!(
        count(&load_with(&[(drawing, Some(deep.as_bytes()))], "deep")),
        (0, 0)
    );
}

#[test]
fn damaged_drawing_parts_never_panic() {
    // Cut the drawing part short at every character, in a package laid
    // out like Excel's: the workbook loads with the drawings that are
    // whole.
    let drawing = excel_drawing();
    let mut seen = 0;
    for cut in (0..drawing.len()).filter(|i| drawing.is_char_boundary(*i)) {
        let wb = load(&excel_package(&[(
            "xl/drawings/drawing1.xml",
            Some(&drawing.as_bytes()[..cut]),
        )]));
        assert_eq!(value(&wb, 0, "A1"), Value::Number(42.0));
        assert!(wb.sheets[0].drawings.len() <= 9, "cut at {cut}");
        seen += wb.sheets[0].drawings.len();
        if cut % 97 == 0 {
            load(&to_bytes(&wb));
        }
    }
    assert!(seen > 20_000, "{seen}");

    // The same at many places in a package of ours, in the parts of the
    // sheet with pictures and a chart, and in an image.
    let ours = to_bytes(&sample());
    let names = part_names(&ours);
    let parts: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|n| (n.clone(), raw_part(&ours, n)))
        .collect();
    let mut attempts = 0;
    for (victim, data) in parts.iter().filter(|(name, _)| {
        name == "xl/drawings/drawing2.xml"
            || name == "xl/drawings/_rels/drawing2.xml.rels"
            || name == "xl/worksheets/_rels/sheet3.xml.rels"
            || name == "xl/media/image1.png"
    }) {
        let text = String::from_utf8(data.clone()).ok();
        let step = if text.is_some() { 7 } else { 3 };
        for cut in (0..data.len()).step_by(step) {
            if text.as_ref().is_some_and(|t| !t.is_char_boundary(cut)) {
                continue;
            }
            let damaged: Vec<(&str, &[u8])> = parts
                .iter()
                .map(|(n, d)| {
                    (
                        n.as_str(),
                        if n == victim { &d[..cut] } else { d.as_slice() },
                    )
                })
                .collect();
            let wb = read_xlsx(Cursor::new(raw_package(&damaged)), Locale::en())
                .expect("the rest of the workbook loads");
            assert_eq!(wb.sheets.len(), 4);
            assert_eq!(wb.sheets[0].drawings.len(), 18, "{victim} cut at {cut}");
            assert!(wb.sheets[1].drawings.len() <= 6, "{victim} cut at {cut}");
            attempts += 1;
        }
    }
    assert!(attempts > 1500, "{attempts}");
}

// ---- one stacking order ----------------------------------------------------------

/// A chart of A1:A3 of the first sheet.
fn small_chart(wb: &mut Workbook) -> u32 {
    for (i, value) in ["3", "1", "2"].into_iter().enumerate() {
        wb.set_cell_text(0, i as u32, 0, value).unwrap();
    }
    let name = wb.sheets[0].name.clone();
    let values = match parse_formula(&format!("'{name}'!$A$1:$A$3"), Locale::en()) {
        Ok(Expr::Ref(range)) => range,
        other => panic!("not a reference: {other:?}"),
    };
    let chart = Chart {
        series: vec![Series::new(values)],
        anchor: (5, 3),
        ..Chart::new(ChartKind::Column)
    };
    wb.add_chart(0, chart).expect("a chart")
}

#[test]
fn anchors_are_written_in_stacking_order() {
    use opensheet_engine::drawing::{ObjectRef, ZOrder};
    let mut wb = Workbook::new(Locale::en());
    let under = shape(&mut wb, 0, ShapeTool::Rectangle, (10.0, 10.0, 80.0, 40.0));
    let first = small_chart(&mut wb);
    let between = wb.add_picture(0, logo(), (2, 2)).expect("a picture");
    let second = small_chart(&mut wb);
    let over = shape(&mut wb, 0, ShapeTool::Ellipse, (40.0, 40.0, 80.0, 40.0));
    wb.update_chart(0, second, |chart| {
        chart.name = "Sales".to_string();
        chart.hidden = true;
    });
    change(&mut wb, 0, between, |d| d.hidden = true);
    // The second chart under everything, the ellipse under the picture.
    wb.reorder_objects(0, &[ObjectRef::Chart(second)], ZOrder::SendToBack);
    wb.reorder_objects(0, &[ObjectRef::Drawing(over)], ZOrder::SendBackward);
    wb.reorder_objects(0, &[ObjectRef::Drawing(over)], ZOrder::SendBackward);
    wb.commit("objects");
    let expected = vec![
        ObjectRef::Chart(second),
        ObjectRef::Drawing(under),
        ObjectRef::Drawing(over),
        ObjectRef::Chart(first),
        ObjectRef::Drawing(between),
    ];
    assert_eq!(wb.object_stack(0), expected);

    let bytes = to_bytes(&wb);
    keep("objects-stacking.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    // One anchor per object, in that order.
    let names: Vec<&str> = xml
        .split("<xdr:cNvPr ")
        .skip(1)
        .map(|rest| {
            let from = rest.find("name=\"").unwrap() + 6;
            &rest[from..from + rest[from..].find('"').unwrap()]
        })
        .collect();
    assert_eq!(names, ["Sales", "Rectangle 1", "Oval 5", "Chart 2", "Picture 3"]);
    assert_eq!(xml.matches("hidden=\"1\"").count(), 2);
    assert_eq!(xml.matches("<xdr:twoCellAnchor").count(), 5);
    assert_eq!(xml.matches("</xdr:wsDr>").count(), 1);

    let back = load(&bytes);
    let kinds = |wb: &Workbook| -> Vec<(bool, String, bool)> {
        wb.object_stack(0)
            .into_iter()
            .map(|object| match object {
                ObjectRef::Chart(id) => {
                    let chart = wb.chart(0, id).unwrap();
                    (true, chart.name.clone(), chart.hidden)
                }
                ObjectRef::Drawing(id) => {
                    let drawing = wb.drawing(0, id).unwrap();
                    (false, drawing.name.clone(), drawing.hidden)
                }
            })
            .collect()
    };
    assert_eq!(kinds(&back), kinds(&wb));
    // And once more: what was read is written the same.
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}

// ---- groups -----------------------------------------------------------------------

/// What a drawing shows: every picture and shape in it with its name,
/// where it is and how it is turned.
fn shown(wb: &Workbook, sheet: usize, name: &str) -> Vec<(String, [i64; 4], f32, bool, bool)> {
    let sh = &wb.sheets[sheet];
    sh.drawing_leaves(named(wb, sheet, name))
        .iter()
        .map(|leaf| {
            let f = leaf.frame;
            // To a hundredth of a pixel: files count in EMU.
            let round = |v: f64| (v * 100.0).round() as i64;
            (
                leaf.drawing.name.clone(),
                [round(f.x), round(f.y), round(f.width), round(f.height)],
                leaf.rotation,
                leaf.flip_h,
                leaf.flip_v,
            )
        })
        .collect()
}

#[test]
fn groups_round_trip() {
    let mut wb = Workbook::new(Locale::en());
    let a = shape(&mut wb, 0, ShapeTool::Rectangle, (100.0, 60.0, 120.0, 60.0));
    let b = shape(&mut wb, 0, ShapeTool::Ellipse, (260.0, 80.0, 60.0, 90.0));
    let logo_id = wb.add_picture(0, logo(), (9, 1)).expect("a picture");
    let c = shape(&mut wb, 0, ShapeTool::Arrow, (60.0, 300.0, 80.0, 40.0));
    change(&mut wb, 0, b, |d| d.rotation = 30.0);
    change(&mut wb, 0, a, |d| d.as_shape_mut().unwrap().text = "In a group".to_string());
    let inner = wb.group_drawings(0, &[a, b]).expect("a group");
    let outer = wb.group_drawings(0, &[inner, logo_id, c]).expect("a group of a group");
    change(&mut wb, 0, outer, |d| {
        d.name = "Everything".to_string();
        d.rotation = 15.0;
        d.alt_text = "Shapes held together".to_string();
    });
    // Stretched after grouping: the members stretch with it.
    let frame = wb.drawing_frame(0, outer).unwrap();
    wb.set_drawing_frame(0, outer, Frame::new(frame.x, frame.y, frame.width * 1.5, frame.height));
    wb.commit("groups");

    let bytes = to_bytes(&wb);
    keep("objects-groups.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    assert_eq!(xml.matches("<xdr:grpSp>").count(), 2);
    assert_eq!(xml.matches("<xdr:twoCellAnchor").count(), 1);
    assert!(xml.contains("<a:chOff "), "{xml}");
    // Every object of the part has an id of its own.
    let ids: Vec<&str> = xml
        .split("<xdr:cNvPr id=\"")
        .skip(1)
        .map(|rest| &rest[..rest.find('"').unwrap()])
        .collect();
    assert_eq!(ids.len(), 6);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 6);
    // The picture in the group has its image.
    let rels = part(&bytes, "xl/drawings/_rels/drawing1.xml.rels");
    assert!(rels.contains("../media/image1.png"), "{rels}");

    let back = load(&bytes);
    assert_eq!(back.sheets[0].drawings.len(), 1);
    let group = named(&back, 0, "Everything");
    assert_eq!((group.rotation, group.alt_text.as_str()), (15.0, "Shapes held together"));
    assert_eq!(shown(&back, 0, "Everything"), shown(&wb, 0, "Everything"));
    assert_eq!(shown(&back, 0, "Everything").len(), 4);
    let text = |wb: &Workbook| {
        wb.sheets[0].drawing_leaves(named(wb, 0, "Everything"))[0]
            .drawing
            .as_shape()
            .map(|s| s.text.clone())
    };
    assert_eq!(text(&back), Some("In a group".to_string()));
    // Written again, the part says the same.
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
    // Ungrouped after the round trip, the members are where they were
    // drawn (the outer group was stretched and turned since).
    let mut again = load(&bytes);
    let id = named(&again, 0, "Everything").id;
    assert_eq!(again.ungroup_drawing(0, id).len(), 3);
}

#[test]
fn a_group_whose_children_have_a_scale_of_their_own() {
    // The group is 200 x 100 pixels on the sheet; its children are placed
    // in a box of 4 000 000 x 2 000 000 EMU starting at (1 000 000, 0).
    let child = |id: u32, name: &str, x: u32, y: u32, cx: u32, cy: u32| {
        format!(
            "<xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"{id}\" name=\"{name}\"/><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill></xdr:spPr></xdr:sp>"
        )
    };
    let group = format!(
        "<xdr:grpSp><xdr:nvGrpSpPr><xdr:cNvPr id=\"2\" name=\"Groupe 1\"/><xdr:cNvGrpSpPr><a:grpSpLocks noChangeAspect=\"1\"/></xdr:cNvGrpSpPr></xdr:nvGrpSpPr><xdr:grpSpPr><a:xfrm><a:off x=\"609600\" y=\"190500\"/><a:ext cx=\"1905000\" cy=\"952500\"/><a:chOff x=\"1000000\" y=\"0\"/><a:chExt cx=\"4000000\" cy=\"2000000\"/></a:xfrm></xdr:grpSpPr>{}{}</xdr:grpSp><xdr:clientData/>",
        child(3, "Left", 1_000_000, 0, 2_000_000, 2_000_000),
        child(4, "Right", 4_000_000, 1_000_000, 1_000_000, 1_000_000),
    );
    let drawing = format!(
        "{DECL}<xdr:wsDr xmlns:xdr=\"{XDR}\" xmlns:a=\"{A}\"><xdr:absoluteAnchor><xdr:pos x=\"952500\" y=\"476250\"/><xdr:ext cx=\"1905000\" cy=\"952500\"/>{group}</xdr:absoluteAnchor></xdr:wsDr>"
    );
    let bytes = excel_package(&[("xl/drawings/drawing1.xml", Some(drawing.as_bytes()))]);
    let wb = load(&bytes);
    let group = named(&wb, 0, "Groupe 1");
    assert!(group.lock_aspect);
    assert_eq!(group.placement, Placement::Fixed);
    assert_eq!(
        shown(&wb, 0, "Groupe 1"),
        vec![
            ("Left".to_string(), [10000, 5000, 10000, 10000], 0.0, false, false),
            ("Right".to_string(), [25000, 10000, 5000, 5000], 0.0, false, false),
        ]
    );
}

// ---- picture adjustments and effects ---------------------------------------------

#[test]
fn picture_adjustments_and_effects_round_trip() {
    use opensheet_engine::drawing::{Effects, Glow, PictureAdjust, Recolor, Shadow, ShadowPreset};
    let mut wb = Workbook::new(Locale::en());
    let adjusted = |wb: &mut Workbook, row: u32, name: &str, adjust: PictureAdjust, effects: Effects| {
        let id = wb.add_picture(0, logo(), (row, 1)).expect("a picture");
        change(wb, 0, id, |d| {
            d.name = name.to_string();
            d.effects = effects;
            d.as_picture_mut().unwrap().adjust = adjust;
        });
    };
    let plain = PictureAdjust::default();
    let none = Effects::default();
    adjusted(&mut wb, 1, "Cropped", PictureAdjust { crop: [0.25, 0.0, 0.1, -0.05], shape: ShapePreset::Ellipse, ..plain }, none);
    adjusted(&mut wb, 6, "Faded", PictureAdjust { transparency: 0.3, brightness: 0.2, contrast: -0.4, ..plain }, none);
    adjusted(&mut wb, 11, "Grey", PictureAdjust { recolor: Recolor::Grayscale, saturation: 2.0, ..plain }, none);
    adjusted(&mut wb, 16, "Sepia", PictureAdjust { recolor: Recolor::Duotone(Color(0), Color(0xD9C3A5)), temperature: 4700.0, ..plain }, none);
    adjusted(&mut wb, 21, "Washed", PictureAdjust { recolor: Recolor::Washout, temperature: 8800.0, ..plain }, none);
    adjusted(&mut wb, 26, "Two tones", PictureAdjust { recolor: Recolor::BlackAndWhite(0.25), ..plain }, none);
    let all = Effects {
        shadow: Some(Shadow { color: Color(0x112233), transparency: 0.5, size: 1.02, blur: 6.0, angle: 135.0, distance: 4.0 }),
        glow: Some(Glow { color: Color(0xE97132), transparency: 0.6, radius: 8.0 }),
        soft_edge: 2.5,
    };
    adjusted(&mut wb, 31, "Effects", plain, all);
    let shadowed = shape(&mut wb, 0, ShapeTool::Rectangle, (400.0, 40.0, 120.0, 60.0));
    change(&mut wb, 0, shadowed, |d| d.effects.shadow = Some(ShadowPreset::BottomRight.shadow()));
    let arrow = shape(&mut wb, 0, ShapeTool::Arrow, (400.0, 160.0, 120.0, 60.0));
    change(&mut wb, 0, arrow, |d| d.effects.shadow = Some(ShadowPreset::Center.shadow()));
    wb.commit("pictures");

    let bytes = to_bytes(&wb);
    keep("objects-picture-format.xlsx", &bytes);
    let xml = part(&bytes, "xl/drawings/drawing1.xml");
    assert_well_formed("drawing1.xml", &xml);
    for expected in [
        "<a:srcRect l=\"25000\" r=\"10000\" b=\"-5000\"/>",
        "<a:prstGeom prst=\"ellipse\"><a:avLst/></a:prstGeom>",
        "<a:alphaModFix amt=\"70000\"/><a:lum bright=\"20000\" contrast=\"-40000\"/></a:blip>",
        "<a:grayscl/><a:hsl hue=\"0\" sat=\"100000\" lum=\"0\"/>",
        "<a:duotone><a:srgbClr val=\"000000\"/><a:srgbClr val=\"D9C3A5\"/></a:duotone><a:tint hue=\"12600000\" amt=\"36000\"/>",
        "<a:tint hue=\"1800000\" amt=\"46000\"/><a:lum bright=\"70000\" contrast=\"-70000\"/>",
        "<a:biLevel thresh=\"25000\"/>",
        "<a:effectLst><a:glow rad=\"101600\"><a:srgbClr val=\"E97132\"><a:alpha val=\"40000\"/></a:srgbClr></a:glow><a:outerShdw blurRad=\"76200\" dist=\"50800\" dir=\"8100000\" sx=\"102000\" sy=\"102000\" algn=\"tr\" rotWithShape=\"0\"><a:srgbClr val=\"112233\"><a:alpha val=\"50000\"/></a:srgbClr></a:outerShdw><a:softEdge rad=\"31750\"/></a:effectLst>",
        // Excel's first preset, as Excel writes it.
        "<a:outerShdw blurRad=\"50800\" dist=\"38100\" dir=\"2700000\" algn=\"tl\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"40000\"/></a:srgbClr></a:outerShdw>",
        "algn=\"ctr\"",
    ] {
        assert!(xml.contains(expected), "{expected} is missing from\n{xml}");
    }
    // An untouched blip stays an empty element.
    assert!(xml.contains("r:embed=\"rId1\"/><a:stretch>"), "{xml}");

    let back = load(&bytes);
    // To the precision of a file: thousandths of a percent, EMU.
    let near = |a: f32, b: f32| (a - b).abs() < 2e-3;
    for (da, db) in wb.sheets[0].drawings.iter().zip(&back.sheets[0].drawings) {
        assert_eq!(da.name, db.name);
        match (da.effects.shadow, db.effects.shadow) {
            (Some(x), Some(y)) => {
                assert_eq!(x.color, y.color, "{}", da.name);
                for (p, q) in [(x.transparency, y.transparency), (x.size, y.size), (x.blur, y.blur), (x.angle, y.angle), (x.distance, y.distance)] {
                    assert!(near(p, q), "{}: {p} became {q}", da.name);
                }
            }
            (x, y) => assert_eq!(x, y, "{}", da.name),
        }
        assert_eq!(da.effects.glow.map(|g| (g.color, g.radius)), db.effects.glow.map(|g| (g.color, g.radius)), "{}", da.name);
        assert!(near(da.effects.soft_edge, db.effects.soft_edge), "{}", da.name);
        let (Some(pa), Some(pb)) = (da.as_picture(), db.as_picture()) else {
            continue;
        };
        let (x, y) = (pa.adjust, pb.adjust);
        assert_eq!((x.shape, x.recolor), (y.shape, y.recolor), "{}", da.name);
        for (p, q) in x.crop.into_iter().zip(y.crop).chain([
            (x.transparency, y.transparency),
            (x.brightness, y.brightness),
            (x.contrast, y.contrast),
            (x.saturation, y.saturation),
            (x.temperature / 1000.0, y.temperature / 1000.0),
        ]) {
            assert!(near(p, q), "{}: {p} became {q}", da.name);
        }
    }
    // Written again, the part says the same.
    assert_eq!(part(&to_bytes(&back), "xl/drawings/drawing1.xml"), xml);
}
