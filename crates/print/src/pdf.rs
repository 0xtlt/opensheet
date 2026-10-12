//! Writing a PDF file from page scenes.
//!
//! The file is written by hand: a flat list of numbered objects (pages,
//! their compressed content streams, fonts, images), a cross-reference
//! table and a trailer. TrueType fonts are embedded as subsets and used
//! as CID fonts (text is written as glyph numbers), each with a
//! `ToUnicode` map so that text can be searched and copied; the standard
//! fonts are only named.

use crate::fonts::{Face, Glyph};
use crate::image::{Image, Pixels};
use crate::scene::{LineCap, LineJoin, Op, Paint, Path, Scene, Segment};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use opensheet_engine::datetime;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Arc;

/// What the file says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub title: String,
    pub author: String,
    /// When the file was made, as a date serial number; `None` leaves the
    /// date out.
    pub created: Option<f64>,
}

/// A number as PDF writes it: no exponent, no trailing zeros.
fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let mut s = format!("{v:.4}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" { "0".to_string() } else { s }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut z = ZlibEncoder::new(Vec::with_capacity(data.len() / 2 + 16), Compression::default());
    // Writing to a `Vec` cannot fail.
    let _ = z.write_all(data);
    z.finish().unwrap_or_default()
}

/// A text string: plain when it is ASCII, UTF-16 otherwise.
fn text_string(s: &str) -> String {
    if s.chars().all(|c| (' '..='~').contains(&c)) {
        let mut out = String::from("(");
        for c in s.chars() {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for unit in s.encode_utf16() {
            let _ = write!(out, "{unit:04X}");
        }
        out.push('>');
        out
    }
}

/// The objects of the file being written.
struct Objects {
    out: Vec<u8>,
    /// Byte offset of every object, by number; 0 until written.
    offsets: Vec<usize>,
}

impl Objects {
    fn alloc(&mut self) -> usize {
        self.offsets.push(0);
        self.offsets.len()
    }

    fn write(&mut self, id: usize, body: &str) {
        self.offsets[id - 1] = self.out.len();
        let _ = write!(self.out, "{id} 0 obj\n{body}\nendobj\n");
    }

    /// A stream object; `dict` holds the entries besides the length and
    /// the filter.
    fn stream(&mut self, id: usize, dict: &str, data: &[u8], compress: bool) {
        self.offsets[id - 1] = self.out.len();
        let packed;
        let (data, filter) = if compress {
            packed = deflate(data);
            (&packed[..], " /Filter /FlateDecode")
        } else {
            (data, "")
        };
        let _ = write!(self.out, "{id} 0 obj\n<< {dict} /Length {}{filter} >>\nstream\n", data.len());
        self.out.extend_from_slice(data);
        self.out.extend_from_slice(b"\nendstream\nendobj\n");
    }
}

/// A font used by the document.
struct FontUse {
    face: Arc<Face>,
    /// The glyphs drawn with an embedded font, and the text each stands
    /// for (what copying it gives).
    glyphs: BTreeMap<u16, char>,
}

/// What the content streams refer to by name.
#[derive(Default)]
struct Resources {
    /// By face id; the resource is named `/F<index>`.
    fonts: Vec<FontUse>,
    font_index: HashMap<usize, usize>,
    /// `/Im<index>`.
    images: Vec<Arc<Image>>,
    image_index: HashMap<*const Image, usize>,
    /// Opacities in 255ths; `/GS<index>`.
    opacities: Vec<u8>,
}

impl Resources {
    fn font(&mut self, face: &Arc<Face>) -> usize {
        *self.font_index.entry(face.id()).or_insert_with(|| {
            self.fonts.push(FontUse {
                face: face.clone(),
                glyphs: BTreeMap::new(),
            });
            self.fonts.len() - 1
        })
    }

    fn image(&mut self, image: &Arc<Image>) -> usize {
        *self.image_index.entry(Arc::as_ptr(image)).or_insert_with(|| {
            self.images.push(image.clone());
            self.images.len() - 1
        })
    }

    fn opacity(&mut self, opacity: f32) -> usize {
        let level = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        match self.opacities.iter().position(|o| *o == level) {
            Some(i) => i,
            None => {
                self.opacities.push(level);
                self.opacities.len() - 1
            }
        }
    }
}

fn rgb(paint: &Paint) -> String {
    let c = paint.color;
    format!("{} {} {}", num(c.r() as f64 / 255.0), num(c.g() as f64 / 255.0), num(c.b() as f64 / 255.0))
}

fn path_ops(path: &Path, out: &mut String) {
    for seg in &path.segments {
        let _ = match seg {
            Segment::MoveTo(x, y) => writeln!(out, "{} {} m", num(*x), num(*y)),
            Segment::LineTo(x, y) => writeln!(out, "{} {} l", num(*x), num(*y)),
            Segment::CurveTo(x1, y1, x2, y2, x, y) => {
                writeln!(out, "{} {} {} {} {} {} c", num(*x1), num(*y1), num(*x2), num(*y2), num(*x), num(*y))
            }
            Segment::Close => writeln!(out, "h"),
        };
    }
}

/// The content stream of a page.
fn content(scene: &Scene, res: &mut Resources) -> String {
    let mut out = String::new();
    // Scenes measure from the top-left corner, PDF from the bottom-left.
    let _ = writeln!(out, "1 0 0 -1 0 {} cm", num(scene.size.1));
    // Translucent painting is wrapped so the opacity does not outlive it.
    let begin = |out: &mut String, res: &mut Resources, paint: &Paint| -> bool {
        if paint.opacity >= 1.0 {
            return false;
        }
        let _ = writeln!(out, "q /GS{} gs", res.opacity(paint.opacity));
        true
    };
    for op in &scene.ops {
        match op {
            Op::Save => out.push_str("q\n"),
            Op::Restore => out.push_str("Q\n"),
            Op::Clip(r) => {
                let _ = writeln!(out, "{} {} {} {} re W n", num(r.x), num(r.y), num(r.w), num(r.h));
            }
            Op::Transform(m) => {
                let _ = writeln!(
                    out,
                    "{} {} {} {} {} {} cm",
                    num(m[0]),
                    num(m[1]),
                    num(m[2]),
                    num(m[3]),
                    num(m[4]),
                    num(m[5])
                );
            }
            Op::Fill { path, paint } => {
                let wrapped = begin(&mut out, res, paint);
                let _ = writeln!(out, "{} rg", rgb(paint));
                path_ops(path, &mut out);
                out.push_str("f\n");
                if wrapped {
                    out.push_str("Q\n");
                }
            }
            Op::Stroke { path, stroke } => {
                let wrapped = begin(&mut out, res, &stroke.paint);
                let cap = match stroke.cap {
                    LineCap::Butt => 0,
                    LineCap::Round => 1,
                    LineCap::Square => 2,
                };
                let join = match stroke.join {
                    LineJoin::Miter => 0,
                    LineJoin::Round => 1,
                    LineJoin::Bevel => 2,
                };
                let dash: Vec<String> = stroke.dash.iter().map(|d| num(*d)).collect();
                let _ = writeln!(
                    out,
                    "{} RG {} w [{}] 0 d {cap} J {join} j",
                    rgb(&stroke.paint),
                    num(stroke.width),
                    dash.join(" ")
                );
                path_ops(path, &mut out);
                out.push_str("S\n");
                if wrapped {
                    out.push_str("Q\n");
                }
            }
            Op::Text { face, size, x, y, text, paint } => {
                let font = res.font(face);
                let mut hex = String::with_capacity(text.len() * 4);
                for ch in text.chars() {
                    match face.glyph(ch).unwrap_or(face.missing_glyph()) {
                        Glyph::Index(g) => {
                            res.fonts[font].glyphs.entry(g).or_insert(ch);
                            let _ = write!(hex, "{g:04X}");
                        }
                        Glyph::Code(c) => {
                            let _ = write!(hex, "{c:02X}");
                        }
                    }
                }
                if hex.is_empty() {
                    continue;
                }
                let wrapped = begin(&mut out, res, paint);
                let (bold, italic) = face.synthetic();
                // The text matrix turns the glyphs upright again (the page
                // is flipped) and slants them for a missing italic.
                let slant = if italic { 0.2 } else { 0.0 };
                let _ = write!(out, "BT /F{font} {} Tf {} rg ", num(*size), rgb(paint));
                if bold {
                    // A missing bold is the regular outline, thickened.
                    let _ = write!(out, "2 Tr {} RG {} w ", rgb(paint), num(size * 0.03));
                }
                let _ = writeln!(out, "1 0 {} -1 {} {} Tm <{hex}> Tj ET", num(slant), num(*x), num(*y));
                if wrapped {
                    out.push_str("Q\n");
                }
            }
            Op::Image { image, rect } => {
                let name = res.image(image);
                let _ = writeln!(
                    out,
                    "q {} 0 0 {} {} {} cm /Im{name} Do Q",
                    num(rect.w),
                    num(-rect.h),
                    num(rect.x),
                    num(rect.y + rect.h)
                );
            }
        }
    }
    out
}

/// Six capital letters marking a font as a subset, different per font.
fn subset_tag(index: usize) -> String {
    let mut n = index;
    (0..6)
        .map(|_| {
            let c = (b'A' + (n % 26) as u8) as char;
            n /= 26;
            c
        })
        .collect()
}

/// The `ToUnicode` map of an embedded font: glyph number to text.
fn to_unicode(glyphs: &BTreeMap<u16, char>) -> String {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u16, &char)> = glyphs.iter().collect();
    for chunk in entries.chunks(100) {
        let _ = writeln!(out, "{} beginbfchar", chunk.len());
        for (glyph, ch) in chunk {
            let mut units = [0u16; 2];
            let text: String = ch.encode_utf16(&mut units).iter().map(|u| format!("{u:04X}")).collect();
            let _ = writeln!(out, "<{glyph:04X}> <{text}>");
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

/// Write the objects of a font; returns the number of the font object.
fn write_font(objects: &mut Objects, index: usize, font: &FontUse) -> usize {
    let id = objects.alloc();
    let face = &font.face;
    let (Some(ttf), None) = (face.ttf(), face.standard()) else {
        let base = face.standard().map_or("Helvetica", |s| s.base_font());
        objects.write(id, &format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Encoding /WinAnsiEncoding >>"));
        return id;
    };
    let (descendant, descriptor, file, unicode) = (objects.alloc(), objects.alloc(), objects.alloc(), objects.alloc());
    let name: String = ttf.names.postscript.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let name = if name.is_empty() { format!("Font{index}") } else { name };
    let base = format!("{}+{name}", subset_tag(index));
    let scale = 1000.0 / ttf.units_per_em as f64;

    objects.write(
        id,
        &format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H \
             /DescendantFonts [{descendant} 0 R] /ToUnicode {unicode} 0 R >>"
        ),
    );
    // Widths of the glyphs in use, runs of consecutive glyphs together.
    let mut widths = String::new();
    let mut previous: Option<u16> = None;
    for glyph in font.glyphs.keys() {
        if previous.is_none_or(|p| p + 1 != *glyph) {
            if previous.is_some() {
                widths.push_str("] ");
            }
            let _ = write!(widths, "{glyph} [");
        } else {
            widths.push(' ');
        }
        widths.push_str(&num(ttf.advance(*glyph) as f64 * scale));
        previous = Some(*glyph);
    }
    if previous.is_some() {
        widths.push(']');
    }
    objects.write(
        descendant,
        &format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} \
             /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
             /FontDescriptor {descriptor} 0 R /DW 1000 /W [{widths}] /CIDToGIDMap /Identity >>"
        ),
    );
    // Symbolic (the glyphs are addressed by number), plus fixed pitch and
    // italic where they apply.
    let flags = 4 | if ttf.fixed_pitch { 1 } else { 0 } | if ttf.style.italic { 64 } else { 0 };
    let bbox: Vec<String> = ttf.bbox.iter().map(|v| num((*v as f64 * scale).round())).collect();
    objects.write(
        descriptor,
        &format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags {flags} /FontBBox [{}] /ItalicAngle {} \
             /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 {file} 0 R >>",
            bbox.join(" "),
            num(ttf.italic_angle as f64),
            num((ttf.ascent as f64 * scale).round()),
            num((ttf.descent as f64 * scale).round()),
            num((ttf.cap_height as f64 * scale).round()),
            if ttf.style.bold { 120 } else { 80 },
        ),
    );
    let glyph_set: BTreeSet<u16> = font.glyphs.keys().copied().collect();
    let subset = ttf.subset(&glyph_set);
    objects.stream(file, &format!("/Length1 {}", subset.len()), &subset, true);
    objects.stream(unicode, "", to_unicode(&font.glyphs).as_bytes(), true);
    id
}

/// Write the objects of an image; returns the number of the image object.
fn write_image(objects: &mut Objects, image: &Image) -> usize {
    let id = objects.alloc();
    let size = format!("/Type /XObject /Subtype /Image /Width {} /Height {}", image.width, image.height);
    match &image.pixels {
        Pixels::Jpeg(data) => {
            // The file is the stream; a reader finds out by itself
            // whether it is grey or colour from the frame header, but the
            // dictionary must agree, so say what the components are.
            let components = jpeg_components(data);
            let space = match components {
                1 => "/DeviceGray",
                4 => "/DeviceCMYK /Decode [1 0 1 0 1 0 1 0]",
                _ => "/DeviceRGB",
            };
            objects.offsets[id - 1] = objects.out.len();
            let _ = write!(
                objects.out,
                "{id} 0 obj\n<< {size} /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                data.len()
            );
            objects.out.extend_from_slice(data);
            objects.out.extend_from_slice(b"\nendstream\nendobj\n");
        }
        Pixels::Rgb(data) => {
            objects.stream(id, &format!("{size} /ColorSpace /DeviceRGB /BitsPerComponent 8"), data, true);
        }
        Pixels::Rgba(data) => {
            // Colour and transparency are separate images in PDF.
            let mask = objects.alloc();
            let colour: Vec<u8> = data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let alpha: Vec<u8> = data.chunks_exact(4).map(|p| p[3]).collect();
            objects.stream(
                id,
                &format!("{size} /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask {mask} 0 R"),
                &colour,
                true,
            );
            objects.stream(mask, &format!("{size} /ColorSpace /DeviceGray /BitsPerComponent 8"), &alpha, true);
        }
    }
    id
}

/// Number of colour components of a JPEG (1 grey, 3 colour, 4 CMYK).
fn jpeg_components(data: &[u8]) -> u8 {
    let mut at = 2;
    while at + 9 < data.len() && data[at] == 0xFF {
        let marker = data[at + 1];
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            return data[at + 9];
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 || marker == 0xFF {
            at += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        at += 2 + u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize;
    }
    3
}

/// `D:YYYYMMDDHHmmSS` for a date serial number.
fn pdf_date(serial: f64) -> String {
    let (y, m, d) = datetime::serial_to_ymd(serial);
    let (h, min, s) = datetime::serial_to_hms(serial);
    format!("D:{y:04}{m:02}{d:02}{h:02}{min:02}{s:02}")
}

/// Write a PDF whose pages are `scenes[order[0]]`, `scenes[order[1]]`...
/// A scene may appear several times (copies): its content is stored once.
pub fn write_pdf(scenes: &[Scene], order: &[usize], info: &DocInfo) -> Vec<u8> {
    let mut objects = Objects {
        out: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(),
        offsets: Vec::new(),
    };
    let catalog = objects.alloc();
    let pages = objects.alloc();
    let resources = objects.alloc();
    let info_id = objects.alloc();

    let mut res = Resources::default();
    let contents: Vec<usize> = scenes
        .iter()
        .map(|scene| {
            let id = objects.alloc();
            let stream = content(scene, &mut res);
            objects.stream(id, "", stream.as_bytes(), true);
            id
        })
        .collect();

    let mut kids: Vec<String> = Vec::new();
    for index in order.iter().filter(|i| **i < scenes.len()) {
        let id = objects.alloc();
        let (w, h) = scenes[*index].size;
        objects.write(
            id,
            &format!(
                "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {} {}] /Resources {resources} 0 R /Contents {} 0 R >>",
                num(w),
                num(h),
                contents[*index]
            ),
        );
        kids.push(format!("{id} 0 R"));
    }
    objects.write(pages, &format!("<< /Type /Pages /Count {} /Kids [{}] >>", kids.len(), kids.join(" ")));
    objects.write(catalog, &format!("<< /Type /Catalog /Pages {pages} 0 R >>"));

    let mut dict = String::from("<< /ProcSet [/PDF /Text /ImageB /ImageC]");
    if !res.fonts.is_empty() {
        dict.push_str(" /Font <<");
        for (index, font) in res.fonts.iter().enumerate() {
            let id = write_font(&mut objects, index, font);
            let _ = write!(dict, " /F{index} {id} 0 R");
        }
        dict.push_str(" >>");
    }
    if !res.images.is_empty() {
        dict.push_str(" /XObject <<");
        for (index, image) in res.images.iter().enumerate() {
            let id = write_image(&mut objects, image);
            let _ = write!(dict, " /Im{index} {id} 0 R");
        }
        dict.push_str(" >>");
    }
    if !res.opacities.is_empty() {
        dict.push_str(" /ExtGState <<");
        for (index, level) in res.opacities.iter().enumerate() {
            let a = num(*level as f64 / 255.0);
            let _ = write!(dict, " /GS{index} << /Type /ExtGState /ca {a} /CA {a} >>");
        }
        dict.push_str(" >>");
    }
    dict.push_str(" >>");
    objects.write(resources, &dict);

    let mut info_dict = String::from("<< /Producer (OpenSheet) /Creator (OpenSheet)");
    if !info.title.is_empty() {
        let _ = write!(info_dict, " /Title {}", text_string(&info.title));
    }
    if !info.author.is_empty() {
        let _ = write!(info_dict, " /Author {}", text_string(&info.author));
    }
    if let Some(serial) = info.created {
        let _ = write!(info_dict, " /CreationDate ({})", pdf_date(serial));
    }
    info_dict.push_str(" >>");
    objects.write(info_id, &info_dict);

    // The file identifier: any bytes that differ between different files.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in &objects.out {
        hash = (hash ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    let file_id = format!("{hash:016X}{:016X}", hash.rotate_left(32) ^ objects.out.len() as u64);

    let xref = objects.out.len();
    let count = objects.offsets.len() + 1;
    let mut tail = format!("xref\n0 {count}\n0000000000 65535 f \n");
    for offset in &objects.offsets {
        let _ = writeln!(tail, "{offset:010} 00000 n ");
    }
    let _ = write!(
        tail,
        "trailer\n<< /Size {count} /Root {catalog} 0 R /Info {info_id} 0 R /ID [<{file_id}> <{file_id}>] >>\n\
         startxref\n{xref}\n%%EOF\n"
    );
    objects.out.extend_from_slice(tail.as_bytes());
    objects.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontDb;
    use crate::scene::{Canvas, RectF, Stroke, TextStyle};
    use opensheet_engine::style::Color;

    #[test]
    fn numbers_and_strings() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(12.345678), "12.3457");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(text_string("Budget (2026)"), "(Budget \\(2026\\))");
        assert_eq!(text_string("Été"), "<FEFF00C9007400E9>");
        assert_eq!(pdf_date(46307.65625), "D:20261012154500");
        assert_eq!(subset_tag(0), "AAAAAA");
        assert_eq!(subset_tag(27), "BBAAAA");
    }

    /// The offsets of the cross-reference table point at the objects.
    fn check_xref(pdf: &[u8]) -> usize {
        let text = String::from_utf8_lossy(pdf);
        let start: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[start..].starts_with(b"xref\n"));
        let table = String::from_utf8_lossy(&pdf[start..]);
        let mut lines = table.lines().skip(1);
        let count: usize = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
        for (id, line) in lines.take(count).enumerate().skip(1) {
            assert_eq!(line.len(), 19, "{line:?}");
            let offset: usize = line[..10].parse().unwrap();
            assert!(pdf[offset..].starts_with(format!("{id} 0 obj\n").as_bytes()), "object {id}");
        }
        count - 1
    }

    #[test]
    fn writes_a_well_formed_file() {
        let fonts = FontDb::standard_only();
        let mut canvas = Canvas::new(&fonts);
        canvas.fill_rect(RectF::new(10.0, 10.0, 100.0, 50.0), Color(0xFFCC00));
        canvas.line(10.0, 70.0, 110.0, 70.0, Stroke::solid(Color::BLACK, 0.75).dashed(&[3.0, 1.5]));
        let style = TextStyle {
            family: "Arial",
            bold: true,
            italic: false,
            size: 12.0,
        };
        canvas.text("Café (1)", 10.0, 100.0, &style, Paint::translucent(Color(0x0000FF), 0.5));
        let image = Arc::new(Image::from_rgba(1, 1, vec![255, 0, 0, 128]).unwrap());
        canvas.image(&image, RectF::new(10.0, 120.0, 20.0, 20.0));
        canvas.image(&image, RectF::new(40.0, 120.0, 20.0, 20.0));
        let scene = Scene {
            size: (612.0, 792.0),
            ops: canvas.into_ops(),
        };
        let info = DocInfo {
            title: "Test".into(),
            author: "Zoé".into(),
            created: Some(46307.5),
        };
        // Two copies of the page: one content stream, two page objects.
        let pdf = write_pdf(std::slice::from_ref(&scene), &[0, 0], &info);
        assert!(pdf.starts_with(b"%PDF-1.7\n"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let objects = check_xref(&pdf);
        // Catalog, pages, resources, info, content, 2 pages, font, image
        // and its mask.
        assert_eq!(objects, 10);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/MediaBox [0 0 612 792]"));
        assert!(text.contains("/BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding"));
        assert!(text.contains("/Title (Test)"));
        assert!(text.contains("/Author <FEFF005A006F00E9>"));
        assert!(text.contains("/CreationDate (D:20261012120000)"));
        assert!(text.contains("/GS0 << /Type /ExtGState /ca 0.502 /CA 0.502 >>"));
        assert_eq!(text.matches("/Subtype /Image").count(), 2);
        // The same input gives the same bytes.
        assert_eq!(pdf, write_pdf(&[scene.clone()], &[0, 0], &info));

        let stream = content(&scene, &mut Resources::default());
        assert!(stream.starts_with("1 0 0 -1 0 792 cm\n"));
        assert!(stream.contains("1 0.8 0 rg\n10 10 m\n110 10 l\n110 60 l\n10 60 l\nh\nf\n"));
        assert!(stream.contains("0 0 0 RG 0.75 w [3 1.5] 0 d 0 J 0 j\n10 70 m\n110 70 l\nS\n"));
        // "Café (1)" in Windows Latin 1, wrapped for its opacity.
        assert!(stream.contains("q /GS0 gs\nBT /F0 12 Tf 0 0 1 rg 1 0 0 -1 10 100 Tm <436166E920283129> Tj ET\nQ\n"));
        assert!(stream.contains("q 20 0 0 -20 10 140 cm /Im0 Do Q\n"));
        assert!(stream.contains("q 20 0 0 -20 40 140 cm /Im0 Do Q\n"));
    }

    #[test]
    fn unicode_maps() {
        let glyphs = BTreeMap::from([(3u16, 'A'), (40, 'é'), (500, '😀')]);
        let map = to_unicode(&glyphs);
        assert!(map.contains("3 beginbfchar\n<0003> <0041>\n<0028> <00E9>\n<01F4> <D83DDE00>\nendbfchar"));
    }
}
