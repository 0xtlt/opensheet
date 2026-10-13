//! The pixels of pictures: decoding them once, and the bitmaps they are
//! painted from. GPUI stretches a bitmap over an upright box and nothing
//! else, so a picture that is flipped or turned is painted from a bitmap
//! made that way, and a large picture shown small from a smaller copy.
//! What the Picture Format tab does to a picture (crop, crop to a shape,
//! transparency, corrections, colours, soft edges) is done to its pixels
//! here too, once for each look it is given.

use super::geometry::{self, Placed};
use super::render::{self, Dress};
use gpui::{App, RenderImage, Window};
use image::{Frame, RgbaImage};
use opensheet_engine::drawing::{NEUTRAL_TEMPERATURE, Picture, PictureAdjust, ShapePreset, image_info};
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

    pub(super) fn offset(&self, x: u32, y: u32) -> usize {
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

    /// What a frame shows of the picture once it is adjusted: the part
    /// the crop keeps (with transparent room where the frame reaches
    /// past the image), its colours changed, cut to the outline of a
    /// shape, and fading out towards its edges over `soft` (a fraction
    /// of the shorter side of the result). `natural` is the size of the
    /// frame at 100 %, which the proportions of an outline depend on.
    pub fn adjusted(&self, adjust: &PictureAdjust, soft: f32, natural: (f32, f32)) -> Bitmap {
        let (w, h) = (self.width as f32, self.height as f32);
        let [l, t, r, b] = adjust.crop;
        let left = (l * w).round() as i64;
        let top = (t * h).round() as i64;
        let width = ((w * (1.0 - l - r)).round() as i64).clamp(1, 16_384) as u32;
        let height = ((h * (1.0 - t - b)).round() as i64).clamp(1, 16_384) as u32;
        let recolour = adjust.changes_pixels();
        let mut data = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            let sy = top + y as i64;
            if sy < 0 || sy >= self.height as i64 {
                continue;
            }
            for x in 0..width {
                let sx = left + x as i64;
                if sx < 0 || sx >= self.width as i64 {
                    continue;
                }
                let from = self.offset(sx as u32, sy as u32);
                let to = (y as usize * width as usize + x as usize) * 4;
                let pixel = &self.data[from..from + 4];
                if recolour {
                    // Stored blue first.
                    let level = |v: u8| v as f32 / 255.0;
                    let [red, green, blue, alpha] = adjust.apply([level(pixel[2]), level(pixel[1]), level(pixel[0]), level(pixel[3])]);
                    let byte = |v: f32| (v * 255.0).round() as u8;
                    data[to..to + 4].copy_from_slice(&[byte(blue), byte(green), byte(red), byte(alpha)]);
                } else {
                    data[to..to + 4].copy_from_slice(pixel);
                }
            }
        }
        let mut out = Bitmap { width, height, data };
        if adjust.shape != ShapePreset::Rect {
            out.cut_to(adjust.shape, natural);
        }
        if soft > 0.0 {
            out.soften(soft * width.min(height) as f32);
        }
        out
    }

    /// Make transparent what lies outside the outline of a shape drawn
    /// over the whole bitmap.
    pub(super) fn cut_to(&mut self, shape: ShapePreset, natural: (f32, f32)) {
        let placed = Placed::plain(0.0, 0.0, self.width as f32, self.height as f32);
        let outlines = geometry::flatten(&shape.path(natural.0.max(1.0), natural.1.max(1.0)), &placed);
        let outlines: Vec<_> = outlines.into_iter().filter(|line| line.closed && line.points.len() >= 3).collect();
        let mut crossings: Vec<f32> = Vec::new();
        for y in 0..self.height {
            // Where the outlines cross the middle of this row.
            let level = y as f32 + 0.5;
            crossings.clear();
            for line in &outlines {
                let n = line.points.len();
                for i in 0..n {
                    let (a, b) = (line.points[i], line.points[(i + 1) % n]);
                    if (a.1 <= level) != (b.1 <= level) {
                        crossings.push(a.0 + (level - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
            }
            crossings.sort_by(f32::total_cmp);
            for x in 0..self.width {
                let centre = x as f32 + 0.5;
                let inside = crossings.iter().filter(|c| **c <= centre).count() % 2 == 1;
                if !inside {
                    let at = self.offset(x, y);
                    self.data[at + 3] = 0;
                }
            }
        }
    }

    /// Fade the bitmap out over `radius` pixels from its edges.
    fn soften(&mut self, radius: f32) {
        let radius = radius.min(self.width.min(self.height) as f32 / 2.0);
        if radius < 0.5 {
            return;
        }
        for y in 0..self.height {
            for x in 0..self.width {
                let near = (x.min(self.width - 1 - x)).min(y.min(self.height - 1 - y)) as f32 + 0.5;
                if near < radius {
                    let at = self.offset(x, y);
                    self.data[at + 3] = (self.data[at + 3] as f32 * near / radius).round() as u8;
                }
            }
        }
    }

    /// Which pixels of a frame `width` by `height` lie inside the
    /// outline a picture is cut to, the frame flipped as the picture is.
    /// `None` for a rectangle, which holds them all.
    pub(super) fn outline_mask(shape: ShapePreset, width: u32, height: u32, natural: (f32, f32), flips: (bool, bool)) -> Option<Vec<bool>> {
        if shape == ShapePreset::Rect {
            return None;
        }
        let mut white = Bitmap {
            width,
            height,
            data: vec![255; width as usize * height as usize * 4],
        };
        white.cut_to(shape, natural);
        let white = white.oriented(flips.0, flips.1, 0);
        Some(white.data.chunks_exact(4).map(|pixel| pixel[3] > 0).collect())
    }

    /// The picture dressed (`render::compose`): `self` is the picture
    /// as its frame shows it, not yet flipped. The result is centred on
    /// the frame, which takes the returned share of its width and
    /// height.
    pub(super) fn dressed(&self, shape: ShapePreset, natural: (f32, f32), flips: (bool, bool), dress: &Dress) -> (Bitmap, (f32, f32)) {
        let body = self.oriented(flips.0, flips.1, 0);
        let mask = Bitmap::outline_mask(shape, body.width, body.height, natural, flips);
        let k = body.width as f32 / natural.0.max(1.0);
        render::compose(&body, mask.as_deref(), dress, k)
    }

    pub(super) fn into_image(self) -> Option<Arc<RenderImage>> {
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

/// What was done to the pixels of a picture before it is oriented: the
/// adjustments as bits (they are plain numbers), the soft edge in
/// thousandths of the shorter side, and the proportions an outline was
/// drawn for.
type Treatment = (Vec<u32>, u32, (u32, u32));

fn treatment(adjust: &PictureAdjust, soft: f32, natural: (f32, f32), dress: Option<&Dress>) -> Option<Treatment> {
    if *adjust == PictureAdjust::default() && soft <= 0.0 && dress.is_none() {
        return None;
    }
    let mut bits: Vec<u32> = adjust.crop.iter().map(|v| v.to_bits()).collect();
    bits.extend([adjust.transparency, adjust.brightness, adjust.contrast, adjust.saturation, adjust.temperature].map(f32::to_bits));
    // Told apart by its name: the enum may grow kinds that carry data.
    bits.push(adjust.shape.ooxml_name().bytes().fold(0u32, |sum, byte| sum.wrapping_mul(31).wrapping_add(byte as u32)));
    use opensheet_engine::drawing::Recolor;
    bits.extend(match adjust.recolor {
        Recolor::None => [0, 0, 0],
        Recolor::Grayscale => [1, 0, 0],
        Recolor::BlackAndWhite(threshold) => [2, threshold.to_bits(), 0],
        Recolor::Washout => [3, 0, 0],
        Recolor::Duotone(dark, light) => [4, dark.value(), light.value()],
    });
    // What a picture wears is measured in points: its size matters too.
    if let Some(dress) = dress {
        bits.push(u32::MAX);
        bits.extend(dress.bits());
    }
    let shaped = adjust.shape != ShapePreset::Rect || dress.is_some();
    let natural = if shaped { (natural.0.round() as u32, natural.1.round() as u32) } else { (0, 0) };
    Some((bits, (soft * 1000.0).round() as u32, natural))
}

struct Shown {
    image: Arc<RenderImage>,
    /// How much of the bitmap's width and height the frame of the
    /// picture takes: all of it, unless the picture is dressed.
    share: (f32, f32),
    /// The repaint it was last drawn at.
    used: u64,
}

struct Decoded {
    /// Keeps the bytes (and so their address, which is the key) alive.
    bytes: Arc<[u8]>,
    /// The picture, then each time at half the size. Empty when the bytes
    /// could not be decoded.
    levels: Vec<Bitmap>,
    shown: HashMap<(Look, Option<Treatment>), Shown>,
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
    image_as(picture, &picture.adjust, 0.0, (placed.width().abs(), placed.height().abs()), placed, scale, None)
}

/// The image file a picture is drawn from and what is still to be done
/// to its pixels, given the adjustments to show (the picture's own, or
/// others while it is cropped): `Picture::shown`.
fn source_of<'a>(picture: &'a Picture, adjust: &PictureAdjust) -> (&'a Arc<[u8]>, PictureAdjust) {
    let (bytes, _) = picture.shown();
    if Arc::ptr_eq(bytes, &picture.data) {
        (bytes, *adjust)
    } else {
        (
            bytes,
            PictureAdjust {
                saturation: 1.0,
                temperature: NEUTRAL_TEMPERATURE,
                ..*adjust
            },
        )
    }
}

/// `image_for` with the adjustments given apart from the picture (the
/// whole image is shown while it is being cropped), a soft edge as a
/// fraction of the shorter side of the frame, and the size of the frame
/// at 100 %.
pub fn image_as(
    picture: &Picture,
    adjust: &PictureAdjust,
    soft: f32,
    natural: (f32, f32),
    placed: &Placed,
    scale: f32,
    dress: Option<&Dress>,
) -> Option<(Arc<RenderImage>, (f32, f32, f32, f32))> {
    let (bytes, adjust) = source_of(picture, adjust);
    let adjust = &adjust;
    let dress = dress.filter(|dress| !dress.is_plain());
    let treatment = treatment(adjust, soft, natural, dress);
    CACHE.with_borrow_mut(|cache| {
        let repaint = cache.repaint;
        let key = Arc::as_ptr(bytes) as *const u8 as usize;
        let decoded = cache.pictures.entry(key).or_insert_with(|| Decoded {
            bytes: bytes.clone(),
            levels: Bitmap::decode(bytes).into_iter().collect(),
            shown: HashMap::new(),
        });
        let (full_w, full_h) = decoded.levels.first().map(|full| (full.width, full.height))?;
        let shown = (placed.width().abs() * scale, placed.height().abs() * scale);
        // A cropped picture shows a part of the image only: the whole of
        // it would be that much larger.
        let kept = |a: f32, b: f32| (1.0 - a - b).clamp(0.05, 1.0);
        let whole = (shown.0 / kept(adjust.crop[0], adjust.crop[2]), shown.1 / kept(adjust.crop[1], adjust.crop[3]));
        let level = level_for(full_w, full_h, whole);
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
        let key = (look, treatment);
        if !decoded.shown.contains_key(&key) {
            let treated;
            let source = match &key.1 {
                Some(_) => {
                    treated = decoded.levels[level as usize].adjusted(adjust, soft, natural);
                    &treated
                }
                None => &decoded.levels[level as usize],
            };
            // Dressed: flipped first, so that the shadow falls where it
            // should, then only turned.
            let composed;
            let (source, share, flipped) = match dress {
                Some(dress) => {
                    let share;
                    (composed, share) = source.dressed(adjust.shape, natural, (placed.flip_h, placed.flip_v), dress);
                    (&composed, share, true)
                }
                None => (source, (1.0, 1.0), false),
            };
            let bitmap = match look {
                Look::Upright {
                    flip_h,
                    flip_v,
                    quarters,
                    ..
                } => source.oriented(flip_h && !flipped, flip_v && !flipped, quarters),
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
                        let keep = matches!(other.0, Look::Upright { .. });
                        if !keep {
                            retired.push(shown.image.clone());
                        }
                        keep
                    });
                    source.turned(
                        (width as f32 / share.0, height as f32 / share.1),
                        tenths as f32 / 10.0,
                        flip_h && !flipped,
                        flip_v && !flipped,
                    )
                }
            };
            let image = bitmap.into_image()?;
            decoded.shown.insert(key.clone(), Shown { image, share, used: repaint });
        }
        let entry = decoded.shown.get_mut(&key)?;
        entry.used = repaint;
        // A dressed picture is larger than its frame, about the same
        // middle.
        let (cx, cy) = placed.centre();
        let (half_w, half_h) = (placed.width() / entry.share.0 / 2.0, placed.height() / entry.share.1 / 2.0);
        let grown = Placed {
            x0: cx - half_w,
            y0: cy - half_h,
            x1: cx + half_w,
            y1: cy + half_h,
            ..*placed
        };
        Some((entry.image.clone(), if entry.share == (1.0, 1.0) { placed.extent() } else { grown.extent() }))
    })
}

/// A copy of the image a picture is drawn from (`Picture::shown`), with
/// at least `size` pixels each way when the image has them, and not
/// many more.
pub fn small_copy(picture: &Picture, size: (u32, u32)) -> Option<Bitmap> {
    let (bytes, _) = picture.shown();
    CACHE.with_borrow_mut(|cache| {
        let key = Arc::as_ptr(bytes) as *const u8 as usize;
        let decoded = cache.pictures.entry(key).or_insert_with(|| Decoded {
            bytes: bytes.clone(),
            levels: Bitmap::decode(bytes).into_iter().collect(),
            shown: HashMap::new(),
        });
        let (full_w, full_h) = decoded.levels.first().map(|full| (full.width, full.height))?;
        let level = level_for(full_w, full_h, (size.0 as f32, size.1 as f32)) as usize;
        while decoded.levels.len() <= level {
            let smaller = decoded.levels[decoded.levels.len() - 1].halved();
            decoded.levels.push(smaller);
        }
        decoded.levels.get(level).cloned()
    })
}

/// A bitmap as a PNG file.
pub fn png_of(bitmap: &Bitmap) -> Option<Vec<u8>> {
    let mut rgba = bitmap.data.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let pixels = RgbaImage::from_raw(bitmap.width, bitmap.height, rgba)?;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels).write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// A picture as a page shows it in a frame `size` (pixels at 100 %),
/// turned by `turn` degrees and flipped: its pixels red first, a few to
/// each pixel of the sheet, with everything the picture wears, and the
/// share of the bitmap's width and height the frame takes. The bitmap
/// is flipped but not turned: the page turns it.
pub fn printed(picture: &Picture, soft: f32, size: (f32, f32), flips: (bool, bool), dress: &Dress) -> Option<(Bitmap, (f32, f32))> {
    /// Pixels of the page to a pixel of the sheet: 288 to the inch.
    const DENSITY: f32 = 3.0;
    let (bytes, adjust) = source_of(picture, &picture.adjust);
    let full = CACHE.with_borrow_mut(|cache| {
        let key = Arc::as_ptr(bytes) as *const u8 as usize;
        let decoded = cache.pictures.entry(key).or_insert_with(|| Decoded {
            bytes: bytes.clone(),
            levels: Bitmap::decode(bytes).into_iter().collect(),
            shown: HashMap::new(),
        });
        decoded.levels.first().cloned()
    })?;
    let kept = |a: f32, b: f32| (1.0 - a - b).clamp(0.05, 1.0);
    let whole = (
        size.0 * DENSITY / kept(adjust.crop[0], adjust.crop[2]),
        size.1 * DENSITY / kept(adjust.crop[1], adjust.crop[3]),
    );
    let mut source = full;
    for _ in 0..level_for(source.width, source.height, whole) {
        source = source.halved();
    }
    let body = source.adjusted(&adjust, soft, size);
    let (mut bitmap, share) = if dress.is_plain() {
        (body.oriented(flips.0, flips.1, 0), (1.0, 1.0))
    } else {
        body.dressed(adjust.shape, size, flips, dress)
    };
    for pixel in bitmap.data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some((bitmap, share))
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

    #[test]
    fn adjustments_are_done_to_the_pixels() {
        use opensheet_engine::Color;
        use opensheet_engine::drawing::Recolor;
        let source = numbered(8, 4);
        let plain = PictureAdjust::default();
        assert_eq!(source.adjusted(&plain, 0.0, (8.0, 4.0)), source);
        // A quarter off the left and half off the bottom.
        let cropped = source.adjusted(&PictureAdjust { crop: [0.25, 0.0, 0.0, 0.5], ..plain }, 0.0, (6.0, 2.0));
        assert_eq!((cropped.width, cropped.height), (6, 2));
        assert_eq!((at(&cropped, 0, 0), at(&cropped, 5, 1)), ((2, 0), (7, 1)));
        // A frame wider than the image: empty room at its left.
        let padded = source.adjusted(&PictureAdjust { crop: [-0.25, 0.0, 0.0, 0.0], ..plain }, 0.0, (10.0, 4.0));
        assert_eq!(padded.width, 10);
        assert_eq!(padded.data[padded.offset(1, 0) + 3], 0);
        assert_eq!(at(&padded, 2, 0), (0, 0));
        // Half transparent.
        let faded = source.adjusted(&PictureAdjust { transparency: 0.5, ..plain }, 0.0, (8.0, 4.0));
        assert_eq!(faded.data[3], 128);
        // Two colours: the black of the first pixel becomes the dark one
        // (pixels are stored blue first).
        let duo = PictureAdjust { recolor: Recolor::Duotone(Color(0x102030), Color(0xFFFFFF)), ..plain };
        assert_eq!(&source.adjusted(&duo, 0.0, (8.0, 4.0)).data[..4], &[0x30, 0x20, 0x10, 255]);
        // Cut to an ellipse: the corners go, the middle stays.
        let mut white = Bitmap { width: 20, height: 20, data: vec![255; 1600] };
        white.cut_to(ShapePreset::Ellipse, (20.0, 20.0));
        assert_eq!(white.data[white.offset(0, 0) + 3], 0);
        assert_eq!(white.data[white.offset(19, 19) + 3], 0);
        assert_eq!(white.data[white.offset(10, 10) + 3], 255);
        assert_eq!(white.data[white.offset(10, 0) + 3], 255);
        // Soft edges: clear at the rim, whole in the middle.
        let soft = Bitmap { width: 20, height: 20, data: vec![255; 1600] }.adjusted(&plain, 0.25, (20.0, 20.0));
        assert!(soft.data[soft.offset(0, 10) + 3] < 40);
        assert!(soft.data[soft.offset(2, 10) + 3] < 255);
        assert_eq!(soft.data[soft.offset(10, 10) + 3], 255);
        // Each look is a bitmap of its own.
        assert_ne!(treatment(&duo, 0.0, (8.0, 4.0), None), treatment(&plain, 0.1, (8.0, 4.0), None));
        assert_eq!(treatment(&plain, 0.0, (8.0, 4.0), None), None);
    }
}
