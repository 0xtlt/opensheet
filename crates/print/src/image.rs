//! Raster images placed on a page: pictures of headers and footers, and
//! whatever the application paints through `Canvas::image`.

use flate2::read::ZlibDecoder;
use std::io::Read;

/// The pixels of an image.
#[derive(Clone, Debug, PartialEq)]
pub enum Pixels {
    /// Three bytes per pixel, rows from the top.
    Rgb(Vec<u8>),
    /// Four bytes per pixel (straight alpha), rows from the top.
    Rgba(Vec<u8>),
    /// A JPEG file, embedded as it is.
    Jpeg(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

impl Image {
    /// `None` when `data` is not `width * height * 3` bytes.
    pub fn from_rgb(width: u32, height: u32, data: Vec<u8>) -> Option<Image> {
        (data.len() == width as usize * height as usize * 3 && !data.is_empty()).then_some(Image {
            width,
            height,
            pixels: Pixels::Rgb(data),
        })
    }

    /// `None` when `data` is not `width * height * 4` bytes.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Option<Image> {
        (data.len() == width as usize * height as usize * 4 && !data.is_empty()).then_some(Image {
            width,
            height,
            pixels: Pixels::Rgba(data),
        })
    }

    /// Read a PNG or JPEG file; `None` for anything else, and for the
    /// rare PNG variants not handled (interlaced images).
    pub fn from_file_bytes(data: &[u8]) -> Option<Image> {
        if data.starts_with(&[0x89, b'P', b'N', b'G']) {
            decode_png(data)
        } else if data.starts_with(&[0xFF, 0xD8]) {
            let (width, height) = jpeg_size(data)?;
            Some(Image {
                width,
                height,
                pixels: Pixels::Jpeg(data.to_vec()),
            })
        } else {
            None
        }
    }
}

/// Width and height of a JPEG, from its frame header.
fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *data.get(at)? != 0xFF {
            return None;
        }
        let marker = *data.get(at + 1)?;
        // Markers without a payload.
        if marker == 0xFF {
            at += 1;
            continue;
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            at += 2;
            continue;
        }
        let length = u16::from_be_bytes([*data.get(at + 2)?, *data.get(at + 3)?]) as usize;
        // Start of frame: baseline, extended, progressive (not the tables
        // that share the range).
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let height = u16::from_be_bytes([*data.get(at + 5)?, *data.get(at + 6)?]) as u32;
            let width = u16::from_be_bytes([*data.get(at + 7)?, *data.get(at + 8)?]) as u32;
            return (width > 0 && height > 0).then_some((width, height));
        }
        at += 2 + length;
    }
}

/// Decode a PNG into RGB or RGBA pixels of 8 bits.
fn decode_png(data: &[u8]) -> Option<Image> {
    let mut at = 8;
    let mut header: Option<(u32, u32, u8, u8)> = None;
    let mut palette: Vec<u8> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();
    let mut compressed: Vec<u8> = Vec::new();
    while at + 8 <= data.len() {
        let length = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as usize;
        let kind = &data[at + 4..at + 8];
        let body = data.get(at + 8..at + 8 + length)?;
        match kind {
            b"IHDR" => {
                let width = u32::from_be_bytes(body.get(0..4)?.try_into().ok()?);
                let height = u32::from_be_bytes(body.get(4..8)?.try_into().ok()?);
                // Interlaced images are not handled.
                if *body.get(12)? != 0 {
                    return None;
                }
                header = Some((width, height, *body.get(8)?, *body.get(9)?));
            }
            b"PLTE" => palette = body.to_vec(),
            b"tRNS" => transparency = body.to_vec(),
            b"IDAT" => compressed.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + length;
    }
    let (width, height, depth, color_type) = header?;
    if width == 0 || height == 0 || width > 1 << 15 || height > 1 << 15 {
        return None;
    }
    let channels: usize = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return None,
    };
    if !matches!(depth, 1 | 2 | 4 | 8 | 16) || (depth < 8 && channels != 1) {
        return None;
    }
    let bits_per_pixel = channels * depth as usize;
    let stride = (width as usize * bits_per_pixel).div_ceil(8);
    // Bytes between a pixel and the one before it, for the filters.
    let step = bits_per_pixel.div_ceil(8);
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    ZlibDecoder::new(&compressed[..]).read_to_end(&mut raw).ok()?;
    if raw.len() < (stride + 1) * height as usize {
        return None;
    }

    // Undo the row filters in place.
    let mut previous = vec![0u8; stride];
    let mut rows: Vec<u8> = Vec::with_capacity(stride * height as usize);
    for y in 0..height as usize {
        let filter = raw[y * (stride + 1)];
        let mut row = raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)].to_vec();
        for i in 0..stride {
            let left = if i >= step { row[i - step] } else { 0 };
            let up = previous[i];
            let up_left = if i >= step { previous[i - step] } else { 0 };
            let predicted = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => {
                    let (a, b, c) = (left as i16, up as i16, up_left as i16);
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        up_left
                    }
                }
                _ => return None,
            };
            row[i] = row[i].wrapping_add(predicted);
        }
        rows.extend_from_slice(&row);
        previous = row;
    }

    // One sample of a pixel as 8 bits.
    let sample = |row: &[u8], index: usize| -> u8 {
        match depth {
            8 => row[index],
            16 => row[index * 2],
            _ => {
                let bit = index * depth as usize;
                let value = (row[bit / 8] >> (8 - depth as usize - bit % 8)) & ((1 << depth) - 1);
                // Palette indices stay as they are; grey levels stretch.
                if color_type == 3 { value } else { (value as u16 * 255 / ((1 << depth) - 1)) as u8 }
            }
        }
    };
    let has_alpha = matches!(color_type, 4 | 6) || !transparency.is_empty();
    let mut out: Vec<u8> = Vec::with_capacity(width as usize * height as usize * if has_alpha { 4 } else { 3 });
    for y in 0..height as usize {
        let row = &rows[y * stride..(y + 1) * stride];
        for x in 0..width as usize {
            let s = |c: usize| sample(row, x * channels + c);
            let (r, g, b, a) = match color_type {
                0 => {
                    let v = s(0);
                    // A grey level named by `tRNS` is transparent.
                    let clear = transparency.len() >= 2 && depth <= 8 && transparency[1] as u16 * 255 / ((1u16 << depth) - 1) == v as u16;
                    (v, v, v, if clear { 0 } else { 255 })
                }
                2 => {
                    let (r, g, b) = (s(0), s(1), s(2));
                    let clear = transparency.len() >= 6
                        && depth == 8
                        && (transparency[1], transparency[3], transparency[5]) == (r, g, b);
                    (r, g, b, if clear { 0 } else { 255 })
                }
                3 => {
                    let i = s(0) as usize;
                    let rgb = palette.get(i * 3..i * 3 + 3)?;
                    (rgb[0], rgb[1], rgb[2], transparency.get(i).copied().unwrap_or(255))
                }
                4 => {
                    let v = s(0);
                    (v, v, v, s(1))
                }
                _ => (s(0), s(1), s(2), s(3)),
            };
            out.extend_from_slice(&[r, g, b]);
            if has_alpha {
                out.push(a);
            }
        }
    }
    Some(Image {
        width,
        height,
        pixels: if has_alpha { Pixels::Rgba(out) } else { Pixels::Rgb(out) },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    /// Build a PNG file from filtered rows.
    pub(crate) fn png(width: u32, height: u32, depth: u8, color_type: u8, rows: &[u8], extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut chunk = |kind: &[u8; 4], body: &[u8]| {
            out.extend((body.len() as u32).to_be_bytes());
            out.extend(kind);
            out.extend(body);
            // The checksum is not verified when reading.
            out.extend([0u8; 4]);
        };
        let mut header = Vec::new();
        header.extend(width.to_be_bytes());
        header.extend(height.to_be_bytes());
        header.extend([depth, color_type, 0, 0, 0]);
        chunk(b"IHDR", &header);
        for (kind, body) in extra {
            chunk(kind, body);
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(rows).unwrap();
        chunk(b"IDAT", &z.finish().unwrap());
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn png_colour_types() {
        // RGB, 2 × 2, the second row filtered "up".
        let file = png(2, 2, 8, 2, &[0, 255, 0, 0, 0, 255, 0, 2, 0, 0, 10, 0, 1, 0], &[]);
        let img = Image::from_file_bytes(&file).unwrap();
        assert_eq!((img.width, img.height), (2, 2));
        assert_eq!(img.pixels, Pixels::Rgb(vec![255, 0, 0, 0, 255, 0, 255, 0, 10, 0, 0, 0]));
        // RGBA with the "sub" filter.
        let file = png(2, 1, 8, 6, &[1, 10, 20, 30, 128, 1, 1, 1, 1], &[]);
        assert_eq!(Image::from_file_bytes(&file).unwrap().pixels, Pixels::Rgba(vec![10, 20, 30, 128, 11, 21, 31, 129]));
        // A palette of two colours at one bit per pixel, the first clear.
        let file = png(3, 1, 1, 3, &[0, 0b0100_0000], &[(b"PLTE", &[1, 2, 3, 4, 5, 6]), (b"tRNS", &[0])]);
        assert_eq!(
            Image::from_file_bytes(&file).unwrap().pixels,
            Pixels::Rgba(vec![1, 2, 3, 0, 4, 5, 6, 255, 1, 2, 3, 0])
        );
        // Grey with alpha, and 16-bit grey.
        let file = png(1, 1, 8, 4, &[0, 200, 50], &[]);
        assert_eq!(Image::from_file_bytes(&file).unwrap().pixels, Pixels::Rgba(vec![200, 200, 200, 50]));
        let file = png(1, 1, 16, 0, &[0, 0x80, 0x01], &[]);
        assert_eq!(Image::from_file_bytes(&file).unwrap().pixels, Pixels::Rgb(vec![0x80, 0x80, 0x80]));
        // Truncated data and other formats are refused.
        assert!(Image::from_file_bytes(&file[..30]).is_none());
        assert!(Image::from_file_bytes(b"GIF89a").is_none());
    }

    #[test]
    fn jpeg_dimensions() {
        // SOI, an APP0 segment, then a baseline frame of 300 × 200.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0];
        jpeg.extend([0xFF, 0xC0, 0, 11, 8, 0, 200, 1, 44, 3]);
        let img = Image::from_file_bytes(&jpeg).unwrap();
        assert_eq!((img.width, img.height), (300, 200));
        assert!(matches!(img.pixels, Pixels::Jpeg(_)));
        assert!(Image::from_file_bytes(&[0xFF, 0xD8, 0xFF]).is_none());
        assert!(Image::from_rgb(2, 2, vec![0; 12]).is_some());
        assert!(Image::from_rgb(2, 2, vec![0; 11]).is_none());
        assert!(Image::from_rgba(1, 1, vec![0; 4]).is_some());
    }
}
