//! A scene as an SVG document: the same vector operations a PDF is
//! written from, for whoever draws SVG (the application makes the picture
//! of a range of cells this way).

use crate::image::{Image, Pixels};
use crate::scene::{LineCap, LineJoin, Op, Paint, Path, Scene, Segment};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::fmt::Write as _;
use std::io::Write as _;

/// A number as a document writes it: short, and never in exponent form.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out
}

fn path_data(path: &Path) -> String {
    let mut d = String::new();
    for segment in &path.segments {
        match *segment {
            Segment::MoveTo(x, y) => write!(d, "M{} {}", num(x), num(y)),
            Segment::LineTo(x, y) => write!(d, "L{} {}", num(x), num(y)),
            Segment::CurveTo(x1, y1, x2, y2, x, y) => {
                write!(
                    d,
                    "C{} {} {} {} {} {}",
                    num(x1),
                    num(y1),
                    num(x2),
                    num(y2),
                    num(x),
                    num(y)
                )
            }
            Segment::Close => write!(d, "Z"),
        }
        .ok();
    }
    d
}

/// ` fill="#RRGGBB"` (or `stroke`), with its opacity when it has one.
fn paint(attribute: &str, paint: &Paint) -> String {
    let mut out = format!(" {attribute}=\"#{:06X}\"", paint.color.value() & 0xFF_FFFF);
    if paint.opacity < 1.0 {
        write!(
            out,
            " {attribute}-opacity=\"{}\"",
            num(paint.opacity as f64)
        )
        .ok();
    }
    out
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(BASE64[(group >> shift & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG file of pixels given as red, green, blue and alpha, rows from
/// the top. `per_inch` is recorded as the size the image is meant to be
/// seen at (a picture drawn at twice the size of the screen says 192, and
/// is then inserted at the size of the cells it shows).
pub fn png_file(width: u32, height: u32, rgba: &[u8], per_inch: Option<f64>) -> Option<Vec<u8>> {
    png(width, height, 4, rgba, per_inch)
}

/// A PNG file of raw pixels, `channels` bytes each, rows from the top.
fn png(
    width: u32,
    height: u32,
    channels: usize,
    data: &[u8],
    per_inch: Option<f64>,
) -> Option<Vec<u8>> {
    let stride = width as usize * channels;
    if stride == 0 || data.len() < stride * height as usize {
        return None;
    }
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in data.chunks_exact(stride).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&raw).ok()?;
    let packed = encoder.finish().ok()?;
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut tagged = kind.to_vec();
        tagged.extend_from_slice(body);
        out.extend_from_slice(&tagged);
        out.extend_from_slice(&crc32(&tagged).to_be_bytes());
    };
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(b"IHDR", &header);
    if let Some(per_inch) = per_inch.filter(|density| density.is_finite() && *density > 0.0) {
        // Pixels per metre, both ways.
        let per_metre = (per_inch / 0.0254).round() as u32;
        let mut density = Vec::with_capacity(9);
        density.extend_from_slice(&per_metre.to_be_bytes());
        density.extend_from_slice(&per_metre.to_be_bytes());
        density.push(1);
        chunk(b"pHYs", &density);
    }
    chunk(b"IDAT", &packed);
    chunk(b"IEND", &[]);
    Some(out)
}

/// An image as the address of an `<image>`: the file itself, in base 64.
fn data_uri(image: &Image) -> Option<String> {
    let (kind, bytes) = match &image.pixels {
        Pixels::Jpeg(bytes) => ("jpeg", bytes.clone()),
        Pixels::Rgb(data) => ("png", png(image.width, image.height, 3, data, None)?),
        Pixels::Rgba(data) => ("png", png(image.width, image.height, 4, data, None)?),
    };
    Some(format!("data:image/{kind};base64,{}", base64(&bytes)))
}

/// Write a scene as an SVG document `pixels` wide and high (the scene
/// itself is in points; the document scales it).
pub fn scene_svg(scene: &Scene, pixels: (f64, f64)) -> String {
    let mut out = String::new();
    write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        num(pixels.0),
        num(pixels.1),
        num(scene.size.0),
        num(scene.size.1)
    )
    .ok();
    // The groups each `Save` opened since: a clip and a transformation
    // last until the matching `Restore`, and each is a group.
    let mut open: Vec<usize> = vec![0];
    let mut clips = 0usize;
    for op in &scene.ops {
        match op {
            Op::Save => open.push(0),
            Op::Restore => {
                if open.len() > 1 {
                    for _ in 0..open.pop().unwrap_or(0) {
                        out.push_str("</g>");
                    }
                }
            }
            Op::Clip(rect) => {
                clips += 1;
                write!(
                    out,
                    "<clipPath id=\"c{clips}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath><g clip-path=\"url(#c{clips})\">",
                    num(rect.x),
                    num(rect.y),
                    num(rect.w.max(0.0)),
                    num(rect.h.max(0.0))
                )
                .ok();
                *open.last_mut().expect("the outermost level") += 1;
            }
            Op::Transform(m) => {
                let numbers: Vec<String> = m.iter().map(|value| num_precise(*value)).collect();
                write!(out, "<g transform=\"matrix({})\">", numbers.join(" ")).ok();
                *open.last_mut().expect("the outermost level") += 1;
            }
            Op::Fill {
                path,
                paint: colour,
            } => {
                write!(
                    out,
                    "<path d=\"{}\"{}/>",
                    path_data(path),
                    paint("fill", colour)
                )
                .ok();
            }
            Op::Stroke { path, stroke } => {
                write!(
                    out,
                    "<path d=\"{}\" fill=\"none\"{} stroke-width=\"{}\"",
                    path_data(path),
                    paint("stroke", &stroke.paint),
                    num(stroke.width)
                )
                .ok();
                if !stroke.dash.is_empty() {
                    let dash: Vec<String> = stroke.dash.iter().map(|length| num(*length)).collect();
                    write!(out, " stroke-dasharray=\"{}\"", dash.join(" ")).ok();
                }
                match stroke.cap {
                    LineCap::Butt => {}
                    LineCap::Round => out.push_str(" stroke-linecap=\"round\""),
                    LineCap::Square => out.push_str(" stroke-linecap=\"square\""),
                }
                match stroke.join {
                    LineJoin::Miter => {}
                    LineJoin::Round => out.push_str(" stroke-linejoin=\"round\""),
                    LineJoin::Bevel => out.push_str(" stroke-linejoin=\"bevel\""),
                }
                out.push_str("/>");
            }
            Op::Text {
                face,
                size,
                x,
                y,
                text,
                paint: colour,
            } => {
                write!(
                    out,
                    "<text x=\"{}\" y=\"{}\" font-family=\"{}\" font-size=\"{}\"",
                    num(*x),
                    num(*y),
                    escape(face.family()),
                    num(*size)
                )
                .ok();
                if face.bold() {
                    out.push_str(" font-weight=\"bold\"");
                }
                if face.italic() {
                    out.push_str(" font-style=\"italic\"");
                }
                write!(
                    out,
                    "{} xml:space=\"preserve\">{}</text>",
                    paint("fill", colour),
                    escape(text)
                )
                .ok();
            }
            Op::Image { image, rect } => {
                if let Some(address) = data_uri(image) {
                    write!(
                        out,
                        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"{address}\"/>",
                        num(rect.x),
                        num(rect.y),
                        num(rect.w),
                        num(rect.h)
                    )
                    .ok();
                }
            }
        }
    }
    for count in open {
        for _ in 0..count {
            out.push_str("</g>");
        }
    }
    out.push_str("</svg>");
    out
}

/// The numbers of a transformation keep more digits: a rotation rounded
/// to thousandths would show over a wide picture.
fn num_precise(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontDb;
    use crate::scene::{Canvas, RectF, Stroke, TextStyle};
    use opensheet_engine::style::Color;
    use std::sync::Arc;

    #[test]
    fn numbers_and_base64() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(12.3456), "12.346");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_png_says_how_large_it_is_meant_to_be() {
        let file = png_file(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255], Some(192.0)).unwrap();
        let info = opensheet_engine::drawing::image_info(&file).unwrap();
        assert_eq!(info.pixels, (2, 1));
        let (width, height) = info.natural_size();
        assert!((width - 1.0).abs() < 1e-4 && (height - 0.5).abs() < 1e-4);
        let decoded = Image::from_file_bytes(&file).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert!(png_file(2, 2, &[0; 8], None).is_none(), "too few pixels");
    }

    #[test]
    fn a_scene_becomes_a_document() {
        let fonts = FontDb::standard_only();
        let mut canvas = Canvas::new(&fonts);
        canvas.fill_rect(RectF::new(0.0, 0.0, 30.0, 15.0), Color(0xFFCC00));
        canvas.save();
        canvas.clip_rect(RectF::new(0.0, 0.0, 30.0, 15.0));
        canvas.translate(2.0, 3.0);
        canvas.line(
            0.0,
            0.0,
            10.0,
            0.0,
            Stroke::solid(Paint::translucent(Color::BLACK, 0.5), 0.5).dashed(&[2.0, 1.0]),
        );
        let style = TextStyle {
            family: "Arial",
            bold: true,
            italic: false,
            size: 8.0,
        };
        canvas.text("a < b & c", 1.0, 9.0, &style, Color(0x112233));
        canvas.restore();
        let image = Arc::new(Image::from_rgb(1, 1, vec![255, 0, 0]).unwrap());
        canvas.image(&image, RectF::new(20.0, 5.0, 4.0, 4.0));
        let scene = Scene {
            size: (30.0, 15.0),
            ops: canvas.into_ops(),
        };
        let svg = scene_svg(&scene, (40.0, 20.0));
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\" viewBox=\"0 0 30 15\">"));
        assert!(svg.contains("<path d=\"M0 0L30 0L30 15L0 15Z\" fill=\"#FFCC00\"/>"));
        assert!(svg.contains("<clipPath id=\"c1\"><rect x=\"0\" y=\"0\" width=\"30\" height=\"15\"/></clipPath><g clip-path=\"url(#c1)\">"));
        assert!(svg.contains("<g transform=\"matrix(1 0 0 1 2 3)\">"));
        assert!(svg.contains("stroke=\"#000000\" stroke-opacity=\"0.5\" stroke-width=\"0.5\" stroke-dasharray=\"2 1\""));
        assert!(svg.contains(
            "font-weight=\"bold\" fill=\"#112233\" xml:space=\"preserve\">a &lt; b &amp; c</text>"
        ));
        assert!(svg.contains("href=\"data:image/png;base64,iVBORw0KGgo"));
        // The clip and the transformation end with their `Restore`,
        // before the image.
        let closed = svg.find("</g></g>").unwrap();
        assert!(closed < svg.find("<image").unwrap());
        assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
        assert!(svg.ends_with("</svg>"));
    }
}
