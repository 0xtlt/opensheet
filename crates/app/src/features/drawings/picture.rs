//! The pixels of pictures: decoding them once, and the bitmaps they are
//! painted from. GPUI stretches a bitmap over an upright box and nothing
//! else, so a picture that is flipped or turned is painted from a bitmap
//! made that way, and a large picture shown small from a smaller copy.

use super::geometry::Placed;
use gpui::{App, RenderImage, Window};
use image::{Frame, RgbaImage};
use opensheet_engine::drawing::{Picture, image_info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

/// The longest side kept of a decoded picture: a sheet never shows more.
const MAX_SIDE: u32 = 3072;

/// The longest side of the bitmap made for a picture turned by an odd
/// angle.
const MAX_TURNED_SIDE: f32 = 2048.0;

/// How many repaints a bitmap that is no longer drawn is kept for.
const KEPT_FOR: u64 = 300;

/// Pixels as GPUI paints them: blue, green, red and alpha for each one,
/// row after row, colours not multiplied by alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Bitmap {
    /// Decode an image file (the first frame of an animation). `None` when
    /// it cannot be read.
    pub fn decode(bytes: &[u8]) -> Option<Bitmap> {
        let mut pixels = image::load_from_memory(bytes).ok()?.into_rgba8();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let (width, height) = pixels.dimensions();
        let mut bitmap = Bitmap {
            width,
            height,
            data: pixels.into_raw(),
        };
        while bitmap.width.max(bitmap.height) > MAX_SIDE {
            bitmap = bitmap.halved();
        }
        (bitmap.width > 0 && bitmap.height > 0).then_some(bitmap)
    }

    fn offset(&self, x: u32, y: u32) -> usize {
        (y as usize * self.width as usize + x as usize) * 4
    }

    /// The same picture at half the size: each pixel is the average of
    /// four, weighted by how opaque they are (a transparent pixel has no
    /// colour to give).
    pub fn halved(&self) -> Bitmap {
        let (width, height) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let at = self.offset((x * 2 + dx).min(self.width - 1), (y * 2 + dy).min(self.height - 1));
                    let alpha = self.data[at + 3] as u32;
                    for channel in 0..3 {
                        sum[channel] += self.data[at + channel] as u32 * alpha;
                    }
                    sum[3] += alpha;
                }
                match sum[3] {
                    0 => data.extend_from_slice(&[0; 4]),
                    alpha => data.extend_from_slice(&[
                        (sum[0] / alpha) as u8,
                        (sum[1] / alpha) as u8,
                        (sum[2] / alpha) as u8,
                        (alpha / 4) as u8,
                    ]),
                }
            }
        }
        Bitmap { width, height, data }
    }

    /// The picture flipped, then turned by a number of quarter turns
    /// clockwise: the pixels are only moved.
    pub fn oriented(&self, flip_h: bool, flip_v: bool, quarters: u8) -> Bitmap {
        let quarters = quarters % 4;
        if !flip_h && !flip_v && quarters == 0 {
            return self.clone();
        }
        let (w, h) = (self.width, self.height);
        let (width, height) = if quarters % 2 == 1 { (h, w) } else { (w, h) };
        let mut data = Vec::with_capacity(self.data.len());
        for y in 0..height {
            for x in 0..width {
                // Where this pixel was before the turn, then before the
                // flips.
                let (sx, sy) = match quarters {
                    0 => (x, y),
                    1 => (y, h - 1 - x),
                    2 => (w - 1 - x, h - 1 - y),
                    _ => (w - 1 - y, x),
                };
                let sx = if flip_h { w - 1 - sx } else { sx };
                let sy = if flip_v { h - 1 - sy } else { sy };
                let at = self.offset(sx, sy);
                data.extend_from_slice(&self.data[at..at + 4]);
            }
        }
        Bitmap { width, height, data }
    }

    /// The colour at a point between pixels (their centres are at whole
    /// coordinates), blended from the four around it; beyond the edges the
    /// picture is transparent, which smooths the edges of a turned picture.
    fn sample(&self, x: f32, y: f32) -> [u8; 4] {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let mut sum = [0.0f32; 4];
        for (dx, dy, weight) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let (px, py) = (x0 as i64 + dx, y0 as i64 + dy);
            if weight <= 0.0 || px < 0 || py < 0 || px >= self.width as i64 || py >= self.height as i64 {
                continue;
            }
            let at = self.offset(px as u32, py as u32);
            let alpha = self.data[at + 3] as f32 * weight;
            for channel in 0..3 {
                sum[channel] += self.data[at + channel] as f32 * alpha;
            }
            sum[3] += alpha;
        }
        if sum[3] <= 0.0 {
            return [0; 4];
        }
        [
            (sum[0] / sum[3]).round() as u8,
            (sum[1] / sum[3]).round() as u8,
            (sum[2] / sum[3]).round() as u8,
            sum[3].round().min(255.0) as u8,
        ]
    }

    /// The picture stretched over a frame of `size` pixels, flipped, and
    /// turned by any angle (clockwise): the bitmap of the upright box
    /// around the turned frame, transparent outside the picture.
    pub fn turned(&self, size: (f32, f32), degrees: f32, flip_h: bool, flip_v: bool) -> Bitmap {
        let (frame_w, frame_h) = (size.0.max(1.0), size.1.max(1.0));
        // A quarter turn must not come out a pixel too wide for a cosine
        // that is almost nought.
        let exact = |v: f32| {
            if v.abs() < 1e-6 {
                0.0
            } else if (v.abs() - 1.0).abs() < 1e-6 {
                v.signum()
            } else {
                v
            }
        };
        let (sin, cos) = degrees.to_radians().sin_cos();
        let (sin, cos) = (exact(sin), exact(cos));
        let width = ((frame_w * cos.abs() + frame_h * sin.abs()).ceil() as u32).max(1);
        let height = ((frame_w * sin.abs() + frame_h * cos.abs()).ceil() as u32).max(1);
        let mut data = vec![0u8; width as usize * height as usize * 4];
        let (source_w, source_h) = (self.width as f32, self.height as f32);
        for y in 0..height {
            for x in 0..width {
                // The centre of the pixel, from the centre of the box,
                // turned back into the frame.
                let (px, py) = (x as f32 + 0.5 - width as f32 / 2.0, y as f32 + 0.5 - height as f32 / 2.0);
                let (lx, ly) = (px * cos + py * sin, -px * sin + py * cos);
                let (mut u, mut v) = (lx / frame_w + 0.5, ly / frame_h + 0.5);
                if flip_h {
                    u = 1.0 - u;
                }
                if flip_v {
                    v = 1.0 - v;
                }
                let (sx, sy) = (u * source_w - 0.5, v * source_h - 0.5);
                if sx <= -1.0 || sy <= -1.0 || sx >= source_w || sy >= source_h {
                    continue;
                }
                let at = (y as usize * width as usize + x as usize) * 4;
                data[at..at + 4].copy_from_slice(&self.sample(sx, sy));
            }
        }
        Bitmap { width, height, data }
    }

    fn into_image(self) -> Option<Arc<RenderImage>> {
        let pixels = RgbaImage::from_raw(self.width, self.height, self.data)?;
        Some(Arc::new(RenderImage::new(vec![Frame::new(pixels)])))
    }
}

/// The bytes of an image file as a workbook keeps them: as they are when
/// the format is one a workbook holds (PNG, JPEG, GIF, BMP), converted to
/// PNG for the others the clipboard and files hand over (TIFF, WebP).
/// `None` when they are not a picture.
pub fn importable(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if image_info(&bytes).is_some() {
        return Some(bytes);
    }
    let decoded = image::load_from_memory(&bytes).ok()?;
    let mut png = Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// One way a picture is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Look {
    /// Upright or turned by quarters, from the copy halved `level` times.
    Upright {
        level: u8,
        flip_h: bool,
        flip_v: bool,
        quarters: u8,
    },
    /// Turned by an odd angle (in tenths of a degree), at this size.
    Turned {
        tenths: i32,
        flip_h: bool,
        flip_v: bool,
        width: u32,
        height: u32,
    },
}

struct Shown {
    image: Arc<RenderImage>,
    /// The repaint it was last drawn at.
    used: u64,
}

struct Decoded {
    /// Keeps the bytes (and so their address, which is the key) alive.
    bytes: Arc<[u8]>,
    /// The picture, then each time at half the size. Empty when the bytes
    /// could not be decoded.
    levels: Vec<Bitmap>,
    shown: HashMap<Look, Shown>,
}

#[derive(Default)]
struct Cache {
    pictures: HashMap<usize, Decoded>,
    repaint: u64,
    /// Bitmaps no longer wanted, whose textures `sweep` releases.
    retired: Vec<Arc<RenderImage>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// How many times a picture `width` pixels wide can be halved and still
/// have a pixel for each of the `shown` pixels it is drawn on.
fn level_for(width: u32, height: u32, shown: (f32, f32)) -> u8 {
    let mut level = 0u8;
    while level < 12
        && (width >> (level + 1)) as f32 >= shown.0.max(1.0)
        && (height >> (level + 1)) as f32 >= shown.1.max(1.0)
    {
        level += 1;
    }
    level
}

/// What to paint for a picture in a frame: a bitmap and the box (x0, y0,
/// x1, y1) it is stretched over. `scale` is the number of device pixels
/// per pixel. `None` when the picture cannot be decoded.
pub fn image_for(picture: &Picture, placed: &Placed, scale: f32) -> Option<(Arc<RenderImage>, (f32, f32, f32, f32))> {
    CACHE.with_borrow_mut(|cache| {
        let repaint = cache.repaint;
        let key = Arc::as_ptr(&picture.data) as *const u8 as usize;
        let decoded = cache.pictures.entry(key).or_insert_with(|| Decoded {
            bytes: picture.data.clone(),
            levels: Bitmap::decode(&picture.data).into_iter().collect(),
            shown: HashMap::new(),
        });
        let (full_w, full_h) = decoded.levels.first().map(|full| (full.width, full.height))?;
        let shown = (placed.width().abs() * scale, placed.height().abs() * scale);
        let level = level_for(full_w, full_h, shown);
        while decoded.levels.len() <= level as usize {
            let smaller = decoded.levels[decoded.levels.len() - 1].halved();
            decoded.levels.push(smaller);
        }
        let quarters = (placed.rotation / 90.0).round();
        let look = if (placed.rotation - quarters * 90.0).abs() < 0.05 {
            Look::Upright {
                level,
                flip_h: placed.flip_h,
                flip_v: placed.flip_v,
                quarters: (quarters as i32).rem_euclid(4) as u8,
            }
        } else {
            let fit = (MAX_TURNED_SIDE / shown.0.max(shown.1)).min(1.0);
            Look::Turned {
                tenths: (placed.rotation * 10.0).round() as i32,
                flip_h: placed.flip_h,
                flip_v: placed.flip_v,
                width: (shown.0 * fit).round().max(1.0) as u32,
                height: (shown.1 * fit).round().max(1.0) as u32,
            }
        };
        if !decoded.shown.contains_key(&look) {
            let source = &decoded.levels[level as usize];
            let bitmap = match look {
                Look::Upright {
                    flip_h,
                    flip_v,
                    quarters,
                    ..
                } => source.oriented(flip_h, flip_v, quarters),
                Look::Turned {
                    tenths,
                    flip_h,
                    flip_v,
                    width,
                    height,
                } => {
                    // One turned bitmap at a time: while the picture is
                    // being turned or resized each repaint makes a new one.
                    let retired = &mut cache.retired;
                    decoded.shown.retain(|other, shown| {
                        let keep = matches!(other, Look::Upright { .. });
                        if !keep {
                            retired.push(shown.image.clone());
                        }
                        keep
                    });
                    source.turned((width as f32, height as f32), tenths as f32 / 10.0, flip_h, flip_v)
                }
            };
            let image = bitmap.into_image()?;
            decoded.shown.insert(look, Shown { image, used: repaint });
        }
        let entry = decoded.shown.get_mut(&look)?;
        entry.used = repaint;
        Some((entry.image.clone(), placed.extent()))
    })
}

/// Called once per repaint: forget the bitmaps that have not been drawn
/// for a while and the pictures that left the workbook and its undo
/// history, and release their textures.
pub fn sweep(window: &mut Window, cx: &mut App) {
    let retired = CACHE.with_borrow_mut(|cache| {
        cache.repaint += 1;
        let Cache {
            pictures,
            repaint,
            retired,
        } = cache;
        pictures.retain(|_, decoded| {
            // Nothing but this cache holds the bytes any more.
            let alive = Arc::strong_count(&decoded.bytes) > 1;
            decoded.shown.retain(|_, shown| {
                let keep = alive && shown.used + KEPT_FOR >= *repaint;
                if !keep {
                    retired.push(shown.image.clone());
                }
                keep
            });
            alive
        });
        std::mem::take(retired)
    });
    for image in retired {
        cx.drop_image(image, Some(window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bitmap whose pixels are numbered: blue holds the column, green
    /// the row.
    fn numbered(width: u32, height: u32) -> Bitmap {
        let mut data = Vec::new();
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        Bitmap { width, height, data }
    }

    fn at(bitmap: &Bitmap, x: u32, y: u32) -> (u8, u8) {
        let offset = bitmap.offset(x, y);
        (bitmap.data[offset], bitmap.data[offset + 1])
    }

    #[test]
    fn flips_and_quarter_turns_move_pixels() {
        let source = numbered(3, 2);
        assert_eq!(source.oriented(false, false, 0), source);
        let flipped = source.oriented(true, false, 0);
        assert_eq!((at(&flipped, 0, 0), at(&flipped, 2, 1)), ((2, 0), (0, 1)));
        let upside_down = source.oriented(false, true, 0);
        assert_eq!(at(&upside_down, 0, 0), (0, 1));
        // A quarter turn clockwise: the left column becomes the top row,
        // read from the bottom.
        let quarter = source.oriented(false, false, 1);
        assert_eq!((quarter.width, quarter.height), (2, 3));
        assert_eq!((at(&quarter, 0, 0), at(&quarter, 1, 0), at(&quarter, 0, 2)), ((0, 1), (0, 0), (2, 1)));
        let half = source.oriented(false, false, 2);
        assert_eq!(at(&half, 0, 0), (2, 1));
        let three = source.oriented(false, false, 3);
        assert_eq!((at(&three, 0, 0), at(&three, 1, 2)), ((2, 0), (0, 1)));
        // Four quarter turns, or two flips of each kind, give the picture
        // back.
        assert_eq!(quarter.oriented(false, false, 3), source);
        assert_eq!(flipped.oriented(true, false, 0), source);
        // A flip and a half turn is the other flip.
        assert_eq!(source.oriented(true, false, 2), upside_down);
    }

    #[test]
    fn halving_averages_what_is_opaque() {
        let mut source = numbered(4, 2);
        // Columns 0 and 1 of both rows become one pixel.
        let half = source.halved();
        assert_eq!((half.width, half.height), (2, 1));
        assert_eq!(&half.data[..4], &[0, 0, 0, 255]);
        assert_eq!(&half.data[4..], &[2, 0, 0, 255]);
        // A transparent pixel gives no colour, only less opacity.
        source.data[0..4].copy_from_slice(&[200, 200, 200, 0]);
        let half = source.halved();
        assert_eq!(half.data[3], 191);
        assert!(half.data[0] <= 1);
        // One pixel stays one pixel.
        assert_eq!(numbered(1, 1).halved().width, 1);
        assert_eq!(level_for(4000, 3000, (500.0, 375.0)), 3);
        assert_eq!(level_for(4000, 3000, (501.0, 375.0)), 2);
        assert_eq!(level_for(100, 100, (300.0, 300.0)), 0);
    }

    #[test]
    fn turning_by_any_angle() {
        let source = numbered(4, 4);
        // No turn at the picture's own size gives it back.
        assert_eq!(source.turned((4.0, 4.0), 0.0, false, false), source);
        // A quarter turn agrees with moving the pixels.
        let turned = source.turned((4.0, 4.0), 90.0, false, false);
        assert_eq!(turned, source.oriented(false, false, 1));
        assert_eq!(source.turned((4.0, 4.0), 0.0, true, false), source.oriented(true, false, 0));
        // At 45 degrees the box is wider, and its corners are empty.
        let diagonal = numbered(40, 40).turned((40.0, 40.0), 45.0, false, false);
        assert_eq!((diagonal.width, diagonal.height), (57, 57));
        assert_eq!(diagonal.data[3], 0);
        assert_eq!(diagonal.data[diagonal.offset(28, 28) + 3], 255);
        // Stretching to another size.
        let wide = source.turned((8.0, 4.0), 0.0, false, false);
        assert_eq!((wide.width, wide.height), (8, 4));
    }

    #[test]
    fn a_picture_is_decoded_once_and_each_look_made_once() {
        let pixels = RgbaImage::from_raw(8, 4, vec![200; 8 * 4 * 4]).unwrap();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let picture = Picture::new(png.into_inner()).unwrap();
        let placed = Placed::plain(10.0, 20.0, 18.0, 24.0);
        let (first, bounds) = image_for(&picture, &placed, 1.0).unwrap();
        assert_eq!(bounds, (10.0, 20.0, 18.0, 24.0));
        assert_eq!((first.size(0).width.0, first.size(0).height.0), (8, 4));
        // Asked again, the same bitmap comes back.
        let (again, _) = image_for(&picture, &placed, 1.0).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        // A quarter turn is another bitmap, lying on its side, drawn over
        // the box of the turned frame.
        let turned = Placed { rotation: 90.0, ..placed };
        let (side, bounds) = image_for(&picture, &turned, 1.0).unwrap();
        assert_eq!((side.size(0).width.0, side.size(0).height.0), (4, 8));
        assert_eq!(bounds, (12.0, 18.0, 16.0, 26.0));
        // Shown at a quarter of its size it is painted from a smaller copy.
        let small = Placed::plain(0.0, 0.0, 2.0, 1.0);
        let (reduced, _) = image_for(&picture, &small, 1.0).unwrap();
        assert_eq!((reduced.size(0).width.0, reduced.size(0).height.0), (2, 1));
        // By an odd angle: the bitmap of the upright box around it.
        let tilted = Placed { rotation: 45.0, ..placed };
        let (odd, _) = image_for(&picture, &tilted, 1.0).unwrap();
        assert_eq!((odd.size(0).width.0, odd.size(0).height.0), (9, 9));
        // Bytes that are no picture give nothing to paint.
        let broken = Picture {
            data: Arc::from(&b"GIF89a\x01\x00\x01\x00"[..]),
            ..picture.clone()
        };
        assert!(image_for(&broken, &placed, 1.0).is_none());
    }

    #[test]
    fn files_are_decoded_and_converted() {
        // A 2 x 1 PNG written by the encoder: red, then half-transparent
        // blue.
        let pixels = RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels.clone()).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let png = png.into_inner();
        let bitmap = Bitmap::decode(&png).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (2, 1));
        // Blue comes first in what GPUI paints.
        assert_eq!(bitmap.data, vec![0, 0, 255, 255, 255, 0, 0, 128]);
        assert!(Bitmap::decode(b"not a picture").is_none());
        // What a workbook can hold is kept byte for byte; a TIFF becomes
        // a PNG.
        assert_eq!(importable(png.clone()), Some(png));
        let mut tiff = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels).write_to(&mut tiff, image::ImageFormat::Tiff).unwrap();
        let converted = importable(tiff.into_inner()).unwrap();
        assert!(converted.starts_with(b"\x89PNG"));
        assert_eq!(Bitmap::decode(&converted).unwrap(), bitmap);
        assert_eq!(importable(b"%PDF-1.7".to_vec()), None);
    }
}
